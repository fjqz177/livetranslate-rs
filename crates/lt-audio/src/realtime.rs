//! 实时识别会话状态机（D-128，D-133 深度优化）：伪流式的「显示 / 定稿 / 裁剪」
//! 纯决策层。
//!
//! 输入 = 当前 tick 的识别假设全文 + 缓冲样本数 + 句内停顿信号；输出 =
//! [`RealtimeOutcome`]（流式行显示文本 / 本轮定稿句 / 裁剪样本数）。
//! 显示层（D-133 A3 弱化 D-128「零门控」为稳定前缀显示）：连续两拍一致的公共
//! 前缀锁定，只有尾部活动区生长/回改，连续 2 拍大改才整段重写（见
//! [`stabilize_display`]）；定稿由句界信号独立触发：
//!
//! ① 句末标点分句（[`crate::interim::split_sentences`] 瘦身复用）：除末句外全部
//!    视为已说完（末句仍在说）；
//! ② 长句无句读走既有逗号回退（分句器内置）；
//! ③ 句内停顿兜底：`trailing_silence_secs ≥ [`PAUSE_FINALIZE_SECS`]` 时**全部**
//!    句子（含末句半句）视为已说完——用户裁决 Q12：强切/收段时已显示的半句
//!    转正，字不蒸发。
//!
//! 退役面（对照旧 `interim` 通道）：整段比例裁剪→短跨度占比（本函数只对当前
//! 缓冲算比例，误差不再随整段线性放大）；回声剥离降为头部保险丝；
//! 「仅一句不提交」的不显示旧语义→零门控显示。
//!
//! 修复面：旧机制「complete 区全是短句时既不提交也不裁剪」会让短句在下一 tick
//! 被重复识别、pending 重复累积——本状态机只要定稿区非空就裁剪（短句出缓冲），
//! pending 天然幂等。

use crate::interim::{
    committed_prefix, is_short_utterance, pending_merge, split_sentences, strip_committed_overlap,
    trim_samples,
};

/// 句内停顿兜底阈值（句界信号③）。auto 档 VAD 收段静音 0.8s 时本兜底先于
/// 收段触发（D-133 A4：0.5→0.7s——解说语流的换气/主播交替常在 0.5s 档，
/// 把半句切成碎片；0.7s 在「半句转正及时性」与「切碎率」间取中，与业界
/// endpoint 经验参数 0.8-1.2s 方向一致更及时）；fixed 档用户把静音调到
/// ≤0.7s 时 VAD flush 先行收段——收段=同效定稿（尾巴照走收尾提交），本兜底
/// 自然失效，语义不受损。
pub const PAUSE_FINALIZE_SECS: f64 = 0.7;

/// 提交尾部回声账本长度（对照原版 `_interim_committed_tail` 的末 50 字符）。
const TAIL_CHARS: usize = 50;

/// 收尾路径短句跨段缓冲的独立成行超时（D-133 A5）：3s 无后续句则不再等
/// 合并、独立提交——内容不蒸发，句尾短叹词（"Yes."）最迟 3s 落屏。
pub const FLUSH_PENDING_TIMEOUT_MS: u64 = 3000;

/// 实时会话状态（原 `interim::InterimState` 的存活子集：裁剪触发时序原子在
/// capture 侧 `InterimControl`，此处只留文本账本）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealtimeState {
    /// 上次定稿区文本的末 ≤50 字符（回声剥离账本）
    pub committed_tail: String,
    /// 短句缓冲（≤8 字母数字；定稿区短句在此等下一长句前置拼接）
    pub pending: String,
    /// 本会话内发生过定稿裁剪（收尾冲刷语义开关：真则尾巴走回声剥离+pending 拼接）
    pub active: bool,
    /// 上一 tick 的原始显示候选（D-133 A3 稳定前缀的「上一拍假设」）
    pub prev_display: String,
    /// 已锁定前缀（连续两拍一致的最长公共前缀；显示层防闪变账本，D-133 A3）
    pub agreed: String,
    /// 连续分歧帧数（假设不再以锁定开头；≥2 接受整段重写，D-133 A3）
    pub divergence: u8,
    /// 收尾路径短句跨段缓冲（D-133 A5）：实时关/收尾段识别出的 ≤8 字符碎片
    /// 暂存于此，并下一句前置或超时独立成行——治「P.」「1.」碎片行
    pub flush_pending: String,
    /// flush_pending 首条入账的 epoch 毫秒（0 = 空；`InterimControl.last_check_ms`
    /// 同口径，SystemTime 粒度足够 3s 超时判定）
    pub flush_pending_at_ms: u64,
    /// flush_pending 识别语言（首条入账时的引擎检出语言；独立成行时供
    /// commit_text 语言过滤——零散碎片不带语言会被误丢）
    pub flush_pending_lang: String,
}

impl RealtimeState {
    /// 全量复位（会话边界：设备切换/引擎切换/清空联动/管道重置——跨段缓冲
    /// 属本会话内容，随会话一并清）
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 段级复位（每收尾段后）：会话文本账本清，跨段 flush_pending **保留**——
    /// 碎片等的是「下一句」而非本段尾巴，段边界冲掉就永远丢了
    pub fn reset_segment(&mut self) {
        let flush = std::mem::take(&mut self.flush_pending);
        let at = self.flush_pending_at_ms;
        let lang = std::mem::take(&mut self.flush_pending_lang);
        *self = Self::default();
        self.flush_pending = flush;
        self.flush_pending_at_ms = at;
        self.flush_pending_lang = lang;
    }

    /// A5：收尾路径短碎片入账（已有暂存则按序合并；空账本记首条时刻与语言）
    pub fn hold_flush_short(&mut self, text: &str, detected_lang: &str, now_ms: u64) {
        if self.flush_pending.is_empty() {
            self.flush_pending_at_ms = now_ms;
            self.flush_pending_lang = detected_lang.to_string();
        }
        self.flush_pending = pending_merge(&self.flush_pending, text.trim());
    }

    /// A5：取走跨段缓冲并与本句前置合并（碎片在前、本句在后；空则原样）
    pub fn take_merged(&mut self, text: &str) -> String {
        let (held, _) = self.take_flush_pending();
        if held.is_empty() {
            text.to_string()
        } else {
            pending_merge(&held, text)
        }
    }

    /// A5：取走跨段缓冲与其识别语言 `(文本, 语言)`（超时独立成行/退出冲刷用；
    /// 语言供 commit_text 过滤——零散碎片不带检出语言会被误丢）
    pub fn take_flush_pending(&mut self) -> (String, String) {
        self.flush_pending_at_ms = 0;
        let text = std::mem::take(&mut self.flush_pending);
        let lang = std::mem::take(&mut self.flush_pending_lang);
        (text, lang)
    }

    /// A5：跨段缓冲是否到独立成行时刻（≥[`FLUSH_PENDING_TIMEOUT_MS`] 无后续；
    /// 0 = 空，永不超时）
    pub fn flush_pending_expired(&self, now_ms: u64) -> bool {
        self.flush_pending_at_ms != 0
            && now_ms.saturating_sub(self.flush_pending_at_ms) >= FLUSH_PENDING_TIMEOUT_MS
    }
}

/// 一次实时 tick 的决策结果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealtimeOutcome {
    /// 流式行当前应显示的完整文本（pending 前缀 + 未定稿假设）；空串 = 清行
    pub display: String,
    /// 本轮新定稿、待提交翻译的句子（已做 pending 前置合并，按序）
    pub finalized: Vec<String>,
    /// 应从缓冲头部裁掉的样本数（0 = 定稿区为空，不裁）
    pub trim_samples: usize,
}

/// 一次实时 tick 的纯决策（无 IO / 无锁 / 无线程——调用方负责 peek、识别、
/// 代际校验与提交）。
///
/// - `raw_text`：本轮识别假设全文（调用方已 trim；空/纯标点由本函数守卫）；
/// - `buffer_samples`：本轮识别所见的缓冲样本数（peek 时点的，供裁剪占比）；
/// - `trailing_silence_secs`：peek 时点的句内静音时长（VAD 同锁读取——识别
///   之后再读会把识别耗时算进去，语义即错）；
/// - `sample_rate`：缓冲采样率（16k）。
pub fn realtime_pass(
    st: &mut RealtimeState,
    raw_text: &str,
    buffer_samples: usize,
    trailing_silence_secs: f64,
    sample_rate: usize,
) -> RealtimeOutcome {
    // ① 头部回声剥离（保险丝）：与上次定稿区尾部重叠的部分剥除
    let hypo = strip_committed_overlap(raw_text.trim(), &st.committed_tail);

    // ② 空/纯标点假设（管道侧已有 alnum 闸，此处纵深防御——回声剥除也可能
    //    把字母数字部分剥干净）：行退到 pending（不清屏不裁剪）。
    //    pending 用 clone 展示、状态保留——它还没被任何定稿区消费，清掉就永远
    //    合并不进翻译（评审抓出的 mem::take 病灶）；惰性构造防提前取用
    let empty_out = |st: &RealtimeState| RealtimeOutcome {
        display: st.pending.clone(),
        finalized: Vec::new(),
        trim_samples: 0,
    };
    if !hypo.chars().any(|c| c.is_alphanumeric()) {
        let mut out = empty_out(st);
        out.display = stabilize_display(st, &out.display);
        return out;
    }

    // ③ 定稿区判定：句界信号③（停顿兜底）= 全部；否则句界信号①（句末标点
    //    + 逗号回退）= 除末句外全部；单句且无兜底 = 无定稿区
    let pause_mode = trailing_silence_secs >= PAUSE_FINALIZE_SECS;
    let sentences = split_sentences(&hypo, "");
    if sentences.is_empty() {
        let mut out = empty_out(st);
        out.display = stabilize_display(st, &out.display);
        return out;
    }
    let complete: &[String] = if pause_mode {
        &sentences[..]
    } else {
        &sentences[..sentences.len() - 1]
    };
    let committed_text: String = if pause_mode {
        sentences.concat()
    } else {
        committed_prefix(&sentences)
    };

    let mut outcome = RealtimeOutcome::default();
    if !committed_text.trim().is_empty() {
        // ④ 定稿句过闸：短句（≤8 字母数字）进 pending 等下一长句前置拼接；
        //    长句携 pending 合并后进定稿列表
        for sent in complete {
            let text = sent.trim();
            if text.is_empty() {
                continue;
            }
            if is_short_utterance(text) {
                st.pending = pending_merge(&st.pending, text);
                continue;
            }
            let merged = pending_merge(&st.pending, text);
            st.pending.clear();
            outcome.finalized.push(merged);
        }
        // ⑤ 裁剪量：短跨度字符占比 + 0.3s 余量（interim::trim_samples 上限/
        //    下限语义不变）；commitd 区非空即裁——含「全是短句」形态（修复旧
        //    机制短句滞留缓冲被重复识别的病灶）
        outcome.trim_samples = trim_samples(
            buffer_samples,
            committed_text.chars().count(),
            hypo.chars().count(),
            sample_rate,
        );
        st.committed_tail = committed_tail_of(&committed_text);
        st.active = true;
    }

    // ⑥ 流式行 = pending 前缀 + 未定稿尾巴（兜底模式全定稿 → 只剩 pending）；
    //    D-133 A3：定稿非空 = 显示基础变更（缓冲被裁）→ 锁定账本重置后重稳
    let tail = if pause_mode {
        ""
    } else {
        sentences[sentences.len() - 1].as_str()
    };
    outcome.display = pending_merge(&st.pending, tail);
    if outcome.display.trim().is_empty() {
        outcome.display.clear();
    }
    if !outcome.finalized.is_empty() {
        st.prev_display.clear();
        st.agreed.clear();
        st.divergence = 0;
    }
    outcome.display = stabilize_display(st, &outcome.display);
    outcome
}

/// 最长公共字符前缀（按 char 比较、byte 切分——不劈 UTF-8）。
fn common_prefix_chars(a: &str, b: &str) -> String {
    let mut end = 0usize;
    for (ac, bc) in a.chars().zip(b.chars()) {
        if ac != bc {
            break;
        }
        end += ac.len_utf8();
    }
    a[..end].to_string()
}

/// 显示层稳定前缀（D-133 A3；D-128「零门控显示」的弱化修订——整段重写是直播
/// 字幕可读性的最大伤害，CHI 2023 text stability 结论）：连续两拍一致的公共
/// 前缀锁定为「已稳定区」，只有尾部活动区生长/回改；假设大改（连续 2 拍不再
/// 以锁定开头）才接受整段重写（防错误识别被永久锁死）。定稿/清空帧清账本
/// （显示基础变更，锁定自然失效）。最坏情况 = 现行为（整段重写）。
fn stabilize_display(st: &mut RealtimeState, cur: &str) -> String {
    if cur.is_empty() {
        st.prev_display.clear();
        st.agreed.clear();
        st.divergence = 0;
        return String::new();
    }
    let common = common_prefix_chars(&st.prev_display, cur);
    if st.agreed.is_empty() {
        // 无锁定（首帧/刚重置）：本帧为基准帧，下拍起才有「两拍一致」可言
        st.agreed = common;
        st.divergence = 0;
    } else if cur.starts_with(st.agreed.as_str()) {
        // 兼容帧：锁定只在「两拍一致且单调延伸」时生长
        if common.starts_with(st.agreed.as_str())
            && common.chars().count() > st.agreed.chars().count()
        {
            st.agreed = common;
        }
        st.divergence = 0;
    } else {
        st.divergence += 1;
        if st.divergence >= 2 {
            st.agreed = common;
            st.divergence = 0;
        }
    }
    st.prev_display = cur.to_string();
    if cur.starts_with(st.agreed.as_str()) {
        format!("{}{}", st.agreed, &cur[st.agreed.len()..])
    } else {
        cur.to_string()
    }
}

/// 定稿区文本的回声账本（末 [`TAIL_CHARS`] 字符；对照原版 L1523）。
fn committed_tail_of(committed_text: &str) -> String {
    let start = committed_text
        .char_indices()
        .rev()
        .nth(TAIL_CHARS - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);
    committed_text[start..].to_string()
}

// ─────────────────── 概率谷对齐裁剪（D-133 A2） ───────────────────

/// 谷点采用阈值：谷语音概率高于此值 = 窗内无真实停顿，维持字符比例估计点。
pub const VALLEY_MAX_CONF: f64 = 0.5;
/// 谷点可晚于估计点的最大余量（秒）。**宁早勿晚**：晚切 = 切进未定稿词中间，
/// 下次识别开头出新乱码（回声剥离救不回）；早切 = 已定稿词的尾部音频残留，
/// 下次识别重复出词由回声剥离吃掉——可修复与不可修复之别。
pub const VALLEY_LATE_SLACK_SECS: f64 = 0.3;
/// 谷点下限（秒）：防对刚起话头的缓冲裁到近乎空（与 trim_samples 的最小
/// 裁剪防循环守卫同精神）。
pub const VALLEY_MIN_TRIM_SECS: f64 = 0.15;

/// 把字符比例估计的裁剪点对齐到 VAD 概率谷（纯函数；A2 真值表锚点）。
///
/// - `estimate_samples`：`trim_samples` 的字符比例估计值（已含 0.3s 余量与
///   上/下限守卫）；`total_samples`：本轮识别所见缓冲总样本；
/// - `valley`：`VadProcessor::lowest_confidence_boundary_in` 的搜索结果
///   `(谷点边界样本数, 谷置信)`——谷点边界 = 低概率 chunk 的结束沿（切在
///   停顿之后，下次识别从新语音起点开始）；
/// - 采用条件：谷置信 ≤ [`VALLEY_MAX_CONF`] 且谷点 ≤ 估计点 +
///   [`VALLEY_LATE_SLACK_SECS`]（宁早勿晚）且 ≥ [`VALLEY_MIN_TRIM_SECS`]；
///   任一不满足回退估计点（现行行为）。
pub fn align_trim_to_valley(
    estimate_samples: usize,
    total_samples: usize,
    valley: Option<(usize, f64)>,
    sample_rate: usize,
) -> usize {
    let sr = sample_rate as f64;
    let late_cap = estimate_samples + (VALLEY_LATE_SLACK_SECS * sr) as usize;
    let min_trim = ((VALLEY_MIN_TRIM_SECS * sr) as usize).min(total_samples);
    match valley {
        Some((boundary, conf))
            if conf <= VALLEY_MAX_CONF && boundary <= late_cap && boundary >= min_trim =>
        {
            boundary
        }
        _ => estimate_samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: usize = 16_000;

    /// 4 秒缓冲的样本数（真值表基准）
    fn buf(secs: f64) -> usize {
        (secs * SR as f64) as usize
    }

    // ── 零门控显示 ──

    #[test]
    fn single_incomplete_sentence_still_updates_display() {
        // 旧语义「仅一句不提交=什么都不发生」→ 新语义零门控：照常刷新显示
        let mut st = RealtimeState::default();
        let out = realtime_pass(&mut st, "今天天气很好我们去公园", buf(4.0), 0.0, SR);
        assert_eq!(out.display, "今天天气很好我们去公园");
        assert!(out.finalized.is_empty(), "单句无句界不定稿");
        assert_eq!(out.trim_samples, 0, "无定稿区不裁剪");
        assert!(!st.active);
    }

    #[test]
    fn empty_hypothesis_keeps_pending_visible() {
        // 识别空/纯标点：行不清空（保留 pending 与上次假设之外的内容——此处
        // 只剩 pending），不裁剪不定稿
        let mut st = RealtimeState {
            pending: "好的。".into(),
            ..Default::default()
        };
        let out = realtime_pass(&mut st, "。！？", buf(3.0), 0.0, SR);
        assert_eq!(out.display, "好的。");
        assert!(out.finalized.is_empty());
        assert_eq!(out.trim_samples, 0);
        assert_eq!(
            st.pending, "好的。",
            "空假设只展示 pending、状态必须保留——清掉就永远合并不进翻译"
        );
    }

    // ── 句界信号①：句末标点 ──

    #[test]
    fn two_sentences_finalize_all_but_last() {
        let mut st = RealtimeState::default();
        let out = realtime_pass(
            &mut st,
            "今天天气真的非常不错。我们现在就去公园散步",
            buf(6.0),
            0.0,
            SR,
        );
        assert_eq!(
            out.finalized,
            vec!["今天天气真的非常不错。"],
            "除末句外定稿"
        );
        assert_eq!(out.display, "我们现在就去公园散步", "流式行只剩未定稿末句");
        assert!(out.trim_samples > 0);
        assert!(st.active, "发生过定稿裁剪即 active");
        assert_eq!(st.committed_tail, "今天天气真的非常不错。");
    }

    #[test]
    fn three_sentences_finalize_in_order() {
        let mut st = RealtimeState::default();
        let out = realtime_pass(
            &mut st,
            "第一句话已经完整地说完了。第二句话也完整地说完了！第三句还在说",
            buf(7.0),
            0.0,
            SR,
        );
        assert_eq!(
            out.finalized,
            vec!["第一句话已经完整地说完了。", "第二句话也完整地说完了！"]
        );
        assert_eq!(out.display, "第三句还在说");
    }

    // ── 短句 pending ──

    #[test]
    fn short_sentence_in_complete_region_buffers_to_pending() {
        let mut st = RealtimeState::default();
        let out = realtime_pass(
            &mut st,
            "好的。今天我们聊聊天气这个话题吧",
            buf(6.0),
            0.0,
            SR,
        );
        assert!(out.finalized.is_empty(), "「好的。」≤8 字符进 pending");
        assert_eq!(st.pending, "好的。");
        assert_eq!(
            out.display, "好的。今天我们聊聊天气这个话题吧",
            "pending 前置显示"
        );
        assert!(
            out.trim_samples > 0,
            "修复面：短句定稿区也必须裁剪，否则下一 tick 重复识别致 pending 翻倍"
        );
        assert!(st.active);
    }

    #[test]
    fn pending_prepends_to_next_finalized_sentence() {
        let mut st = RealtimeState {
            pending: "好的。".into(),
            ..Default::default()
        };
        let out = realtime_pass(
            &mut st,
            "今天天气真的非常不错。我们现在出门",
            buf(5.0),
            0.0,
            SR,
        );
        assert_eq!(
            out.finalized,
            vec!["好的。今天天气真的非常不错。"],
            "pending 无分隔符前置"
        );
        assert_eq!(st.pending, "", "合并后清空");
        assert_eq!(out.display, "我们现在出门");
    }

    // ── 句界信号③：句内停顿兜底 ──

    #[test]
    fn pause_fallback_finalizes_everything_including_half_sentence() {
        let mut st = RealtimeState::default();
        let out = realtime_pass(&mut st, "我想想啊这个事情怎么办呢", buf(4.0), 0.75, SR);
        assert_eq!(
            out.finalized,
            vec!["我想想啊这个事情怎么办呢"],
            "半句转正（Q12）"
        );
        assert_eq!(out.display, "", "全部定稿 → 流式行清空");
        assert!(st.active);
    }

    #[test]
    fn pause_threshold_is_inclusive_boundary() {
        let mut st = RealtimeState::default();
        let out_below = realtime_pass(&mut st, "这句话还没完整地说完呢", buf(3.0), 0.69, SR);
        assert!(
            out_below.finalized.is_empty(),
            "0.69s < 兜底阈值 → 仍按句界信号①"
        );

        let mut st2 = RealtimeState::default();
        let out_at = realtime_pass(&mut st2, "这句话还没完整地说完呢", buf(3.0), 0.7, SR);
        assert_eq!(
            out_at.finalized,
            vec!["这句话还没完整地说完呢"],
            "恰达阈值 → 兜底生效"
        );
    }

    #[test]
    fn pause_fallback_with_multiple_sentences_uses_full_ratio_trim() {
        let mut st = RealtimeState::default();
        let out = realtime_pass(
            &mut st,
            "第一句话已经完整地说完了。第二句话也完整地说完了。第三句还在继续说着",
            buf(6.0),
            0.8,
            SR,
        );
        assert_eq!(out.finalized.len(), 3, "兜底模式全句定稿");
        // committed==full → ratio=1 → 裁到 total-0.5s（trim_samples 上限语义）
        assert_eq!(out.trim_samples, buf(6.0) - buf(0.5));
    }

    // ── 回声剥离（头部保险丝）──

    #[test]
    fn echo_overlap_is_stripped_before_all_decisions() {
        let mut st = RealtimeState {
            committed_tail: "hello world".into(),
            ..Default::default()
        };
        let out = realtime_pass(
            &mut st,
            "world and more words. Next part",
            buf(5.0),
            0.0,
            SR,
        );
        assert_eq!(
            out.finalized,
            vec!["and more words."],
            "回声剥除在分句之前——剥后「and more words. Next part」按句界正常处理"
        );
        assert_eq!(out.display, "Next part");
    }

    #[test]
    fn full_echo_leaves_display_at_pending() {
        let mut st = RealtimeState {
            committed_tail: "hello world".into(),
            ..Default::default()
        };
        let out = realtime_pass(&mut st, "world", buf(2.0), 0.0, SR);
        assert!(out.finalized.is_empty());
        assert_eq!(out.display, "", "全回声剥空 → 行清空");
        assert_eq!(out.trim_samples, 0);
    }

    // ── 裁剪与账本 ──

    #[test]
    fn trim_uses_short_span_ratio_with_margin() {
        // committed 7 字 / full 14 字 → ratio 0.5 × 4s + 0.3s margin = 2.3s
        let mut st = RealtimeState::default();
        let out = realtime_pass(
            &mut st,
            "前半句已经说完。后半句还在继续说",
            buf(4.0),
            0.0,
            SR,
        );
        assert_eq!(
            out.trim_samples,
            (buf(4.0) as f64 * 0.5) as usize + (0.3 * SR as f64) as usize
        );
    }

    #[test]
    fn committed_tail_capped_at_fifty_chars() {
        let long: String = "字".repeat(80);
        assert_eq!(committed_tail_of(&long).chars().count(), TAIL_CHARS);
        assert_eq!(committed_tail_of("短"), "短");
    }

    // ── 概率谷对齐裁剪（A2）──

    #[test]
    fn valley_adopted_when_confident_pause_within_caps() {
        // 估计 2s，谷点 1.9s（更早、置信 0.1）→ 采用谷点（宁早勿晚）
        assert_eq!(
            align_trim_to_valley(buf(2.0), buf(6.0), Some((buf(1.9), 0.1)), SR),
            buf(1.9)
        );
        // 谷点略晚于估计但在 +0.3s 余量内 → 仍采用（切在真实停顿之后）
        assert_eq!(
            align_trim_to_valley(buf(2.0), buf(6.0), Some((buf(2.2), 0.3)), SR),
            buf(2.2)
        );
    }

    #[test]
    fn valley_rejected_when_too_late_too_speechy_or_too_shallow() {
        // 谷点晚于估计+0.3s：晚切会切进未定稿词中间 → 拒绝
        assert_eq!(
            align_trim_to_valley(buf(2.0), buf(6.0), Some((buf(2.4), 0.1)), SR),
            buf(2.0)
        );
        // 谷置信 0.8 = 窗内无真实停顿（高概率仍是语音）→ 拒绝
        assert_eq!(
            align_trim_to_valley(buf(2.0), buf(6.0), Some((buf(1.9), 0.8)), SR),
            buf(2.0)
        );
        // 谷点 < 0.15s：防裁空刚起话头的缓冲 → 拒绝
        assert_eq!(
            align_trim_to_valley(
                buf(2.0),
                buf(6.0),
                Some(((0.1 * SR as f64) as usize, 0.1)),
                SR
            ),
            buf(2.0)
        );
        // 无谷（None）→ 回退估计点
        assert_eq!(align_trim_to_valley(buf(2.0), buf(6.0), None, SR), buf(2.0));
    }

    #[test]
    fn reset_clears_all_state() {
        let mut st = RealtimeState {
            committed_tail: "旧的尾巴".into(),
            pending: "好的。".into(),
            active: true,
            ..Default::default()
        };
        st.reset();
        assert_eq!(st, RealtimeState::default());
    }

    // ── 显示层稳定前缀（A3）──

    #[test]
    fn stable_prefix_locks_and_only_tail_churns() {
        let mut st = RealtimeState::default();
        // 帧1：基准帧（无锁定，照常显示）
        let f1 = realtime_pass(&mut st, "今天我们聊聊天气", buf(3.0), 0.0, SR);
        assert_eq!(f1.display, "今天我们聊聊天气");
        assert!(st.agreed.is_empty(), "单帧无「两拍一致」可言");
        // 帧2：假设稳定生长 → 公共前缀「今天我们聊聊天气」锁定
        let f2 = realtime_pass(&mut st, "今天我们聊聊天气吧", buf(4.0), 0.0, SR);
        assert_eq!(f2.display, "今天我们聊聊天气吧");
        assert_eq!(st.agreed, "今天我们聊聊天气", "两拍一致的前缀入锁");
        // 帧3：继续生长 → 锁定延伸、显示=锁定+尾段（文本面不变，账本单调）
        let f3 = realtime_pass(&mut st, "今天我们聊聊天气吧好", buf(5.0), 0.0, SR);
        assert_eq!(f3.display, "今天我们聊聊天气吧好");
        assert_eq!(st.agreed, "今天我们聊聊天气吧", "锁定随两拍一致延伸");
    }

    #[test]
    fn engine_rewrite_holds_lock_one_frame_then_rewrites() {
        let mut st = RealtimeState::default();
        let _ = realtime_pass(&mut st, "今天天气真的非常不错", buf(3.0), 0.0, SR);
        let _ = realtime_pass(&mut st, "今天天气真的非常不错啊", buf(4.0), 0.0, SR);
        assert_eq!(st.agreed, "今天天气真的非常不错");
        // 帧3：引擎整句改写（前缀不兼容）→ 单帧分歧：账本保持、显示回退整段
        let f3 = realtime_pass(&mut st, "其实昨天晚上下了一场大雨", buf(5.0), 0.0, SR);
        assert_eq!(
            f3.display, "其实昨天晚上下了一场大雨",
            "不兼容帧诚实整段显示"
        );
        assert_eq!(st.agreed, "今天天气真的非常不错", "单帧分歧不拆锁");
        // 帧4：改写延续 → 连续 2 拍分歧 → 接受重写
        let f4 = realtime_pass(&mut st, "其实昨天晚上下了一场大雨还在下", buf(6.0), 0.0, SR);
        assert_eq!(f4.display, "其实昨天晚上下了一场大雨还在下");
        assert_eq!(st.agreed, "其实昨天晚上下了一场大雨", "两拍分歧接受重写");
    }

    #[test]
    fn finalize_resets_prefix_lock() {
        let mut st = RealtimeState::default();
        // 帧1-2：无定稿的两拍 → 锁定积累
        let _ = realtime_pass(&mut st, "今天天气真的非常不错", buf(3.0), 0.0, SR);
        let _ = realtime_pass(&mut st, "今天天气真的非常不错啊", buf(4.0), 0.0, SR);
        assert!(!st.agreed.is_empty());
        // 帧3：句末标点定稿前句 → 显示基础变更（缓冲被裁）→ 锁定重置
        let f3 = realtime_pass(
            &mut st,
            "今天天气真的非常不错啊。新的句子",
            buf(5.0),
            0.0,
            SR,
        );
        assert_eq!(f3.display, "新的句子");
        assert!(st.agreed.is_empty(), "定稿帧重置锁定，下拍重新积累");
    }

    #[test]
    fn clear_frame_resets_prefix_books() {
        let mut st = RealtimeState::default();
        let _ = realtime_pass(&mut st, "今天天气真的非常不错", buf(3.0), 0.0, SR);
        let _ = realtime_pass(&mut st, "今天天气真的非常不错啊", buf(4.0), 0.0, SR);
        assert!(!st.agreed.is_empty());
        // 兜底全定稿 → 显示空 → 清行帧账本随行清
        let out = realtime_pass(&mut st, "今天天气真的非常不错啊", buf(4.0), 0.8, SR);
        assert_eq!(out.display, "", "全部定稿 → 清行");
        assert!(st.agreed.is_empty() && st.prev_display.is_empty() && st.divergence == 0);
    }

    // ── 收尾路径短句跨段缓冲（A5）──

    #[test]
    fn flush_short_holds_merges_and_expires() {
        let mut st = RealtimeState::default();
        st.hold_flush_short("P.", "en", 1000);
        assert_eq!(st.flush_pending, "P.");
        assert_eq!(st.flush_pending_at_ms, 1000, "首条入账记时刻");
        assert_eq!(st.flush_pending_lang, "en", "首条入账记检出语言");
        st.hold_flush_short("1.", "en", 1500);
        assert_eq!(st.flush_pending, "P.1.", "按序合并");
        assert_eq!(st.flush_pending_at_ms, 1000, "时刻不随后续合并漂移");
        assert!(!st.flush_pending_expired(1000 + 2999), "2.999s 未超时");
        assert!(st.flush_pending_expired(1000 + 3000), "恰 3s 超时");
        let (held, lang) = st.take_flush_pending();
        assert_eq!(held, "P.1.");
        assert_eq!(lang, "en");
        assert_eq!(st.flush_pending_at_ms, 0, "取走后计时复位");
        assert!(!st.flush_pending_expired(9_999_999), "空账本永不超时");
    }

    #[test]
    fn take_merged_prepends_held_before_text() {
        let mut st = RealtimeState::default();
        assert_eq!(st.take_merged("Hello there"), "Hello there", "空缓冲原样");
        st.hold_flush_short("Yes.", "en", 0);
        assert_eq!(st.take_merged("Hello there"), "Yes.Hello there");
        assert!(st.flush_pending.is_empty(), "合并即取走");
        assert!(st.flush_pending_lang.is_empty());
    }

    #[test]
    fn reset_segment_keeps_flush_pending_but_reset_clears_it() {
        let mut st = RealtimeState {
            committed_tail: "尾巴".into(),
            pending: "好的。".into(),
            active: true,
            flush_pending: "P.".into(),
            flush_pending_at_ms: 42,
            flush_pending_lang: "en".into(),
            ..Default::default()
        };
        st.reset_segment();
        assert_eq!(
            st,
            RealtimeState {
                flush_pending: "P.".into(),
                flush_pending_at_ms: 42,
                flush_pending_lang: "en".into(),
                ..Default::default()
            },
            "段级复位只留跨段缓冲（碎片等的是下一句不是本段尾巴）"
        );
        st.reset();
        assert_eq!(st, RealtimeState::default(), "会话级复位全清");
    }
}
