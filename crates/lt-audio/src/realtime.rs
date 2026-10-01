//! 实时识别会话状态机（D-128）：伪流式的「显示 / 定稿 / 裁剪」纯决策层。
//!
//! 输入 = 当前 tick 的识别假设全文 + 缓冲样本数 + 句内停顿信号；输出 =
//! [`RealtimeOutcome`]（流式行显示文本 / 本轮定稿句 / 裁剪样本数）。**显示零门控**
//! （用户裁决：忠实体现引擎输出、实时性优先）——每次成功 tick 都整段刷新显示，
//! 不等任何「连续一致」确认；定稿由句界信号独立触发：
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

/// 句内停顿兜底阈值（句界信号③）。只在 VAD 收段静音阈值 > 0.5s 时先于收段
/// 触发（auto 档短缓冲 0.8s 起——0.5s 留出兜底窗口）；fixed 档用户把静音调到
/// ≤0.5s 时 VAD flush 先行收段——收段=同效定稿（尾巴照走收尾提交），本兜底
/// 自然失效，语义不受损。
pub const PAUSE_FINALIZE_SECS: f64 = 0.5;

/// 提交尾部回声账本长度（对照原版 `_interim_committed_tail` 的末 50 字符）。
const TAIL_CHARS: usize = 50;

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
}

impl RealtimeState {
    /// 全量复位（会话边界：设备切换/清空联动/管道重置）
    pub fn reset(&mut self) {
        *self = Self::default();
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
        return empty_out(st);
    }

    // ③ 定稿区判定：句界信号③（停顿兜底）= 全部；否则句界信号①（句末标点
    //    + 逗号回退）= 除末句外全部；单句且无兜底 = 无定稿区
    let pause_mode = trailing_silence_secs >= PAUSE_FINALIZE_SECS;
    let sentences = split_sentences(&hypo, "");
    if sentences.is_empty() {
        return empty_out(st);
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

    // ⑥ 流式行 = pending 前缀 + 未定稿尾巴（兜底模式全定稿 → 只剩 pending）
    let tail = if pause_mode {
        ""
    } else {
        sentences[sentences.len() - 1].as_str()
    };
    outcome.display = pending_merge(&st.pending, tail);
    if outcome.display.trim().is_empty() {
        outcome.display.clear();
    }
    outcome
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
        let out = realtime_pass(&mut st, "我想想啊这个事情怎么办呢", buf(4.0), 0.6, SR);
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
        let out_below = realtime_pass(&mut st, "这句话还没完整地说完呢", buf(3.0), 0.49, SR);
        assert!(
            out_below.finalized.is_empty(),
            "0.49s < 兜底阈值 → 仍按句界信号①"
        );

        let mut st2 = RealtimeState::default();
        let out_at = realtime_pass(&mut st2, "这句话还没完整地说完呢", buf(3.0), 0.5, SR);
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

    #[test]
    fn reset_clears_all_state() {
        let mut st = RealtimeState {
            committed_tail: "旧的尾巴".into(),
            pending: "好的。".into(),
            active: true,
        };
        st.reset();
        assert_eq!(st, RealtimeState::default());
    }
}
