//! 思维链隔离（W2/方案 §4.3，INV-F）：**无论服务端怎么返回，思考内容都不得进入
//! 译文**。
//!
//! 两层职责：
//! - `delta.content` 之外的字段（`reasoning_content` / `reasoning` / `thinking`）在
//!   反序列化层就被丢弃（translator.rs 只取 `delta.content`）；
//! - 但有些服务端不分离思考，直接把推理块写进 `content`（实证：LM Studio issue
//!   #1569 至今把推理标签写进 content；llama.cpp `--reasoning-format none` 官方
//!   说明即 "leaves thoughts unparsed in message.content"；vLLM 不配
//!   `--reasoning-parser` 即不抽取）——本模块把这部分剥掉。
//!
//! 标签表**以实证为准**（不臆造），五组（开 / 闭）：
//!
//! | 形态 | 开 | 闭 | 出处 |
//! |---|---|---|---|
//! | think 标签 | U+3C think U+3E | U+3C /think U+3E | DeepSeek-R1/Qwen3 系无 parser 部署 |
//! | 兼容写法 | U+3C thinking U+3E | U+3C /thinking U+3E | 客户端通行剥离对象（防御性纳入） |
//! | harmony 分析通道 | U+3C\|channel\|\u{3e}analysis U+3C\|message\| U+3E | U+3C\|end\|U+3E | gpt-oss |
//! | Kimi K2 Thinking | U+25C1 think U+25B7 | U+25C1 /think U+25B7 | SGLang parser 表 |
//! | Apertus | U+3C\|inner_prefix\|U+3E | U+3C\|inner_suffix\|U+3E | SGLang parser 表 |
//!
//! 另有**游离闭标签**形态（GLM-5.3 + 老 parser：答案出现在闭标签**之后**）——
//! 无配对开标签时删到该闭标签（含）。
//!
//! **源码书写约定**：下表所有非 ASCII 字符一律用 `\u{..}` 转义书写，源码中不出现
//! 完整标签字面量——实证发现标签序列会被编辑/编码环节**间歇性**改写（闭标签曾被
//! 写成全角变体，导致"块永不闭合 → 译文恒空"的静默故障）。配套运行时守卫见
//! `tag_table_is_intact`。

/// (开标签, 闭标签) 对，顺序即配对关系
const TAGS: [(&str, &str); 5] = [
    ("\u{3c}think\u{3e}", "\u{3c}/think\u{3e}"),
    ("\u{3c}thinking\u{3e}", "\u{3c}/thinking\u{3e}"),
    (
        "\u{3c}|channel|\u{3e}analysis\u{3c}|message|\u{3e}",
        "\u{3c}|end|\u{3e}",
    ),
    ("\u{25c1}think\u{25b7}", "\u{25c1}/think\u{25b7}"),
    ("\u{3c}|inner_prefix|\u{3e}", "\u{3c}|inner_suffix|\u{3e}"),
];

/// ASCII 大小写不敏感查找（标签全是 ASCII 关键字；`◁▷` 等非 ASCII 字节按原样
/// 比较）。实证：`<Think>SECRET</Think>` 在大小写敏感匹配下**整段漏进译文**——
/// 因此这一层必须宽容。返回的下标保证是字符边界。
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    (0..=h.len() - n.len())
        .find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n) && hay.is_char_boundary(i))
}

/// 在 `hay` 中找最早出现的开标签 → (起点, 开标签长, 闭标签)
fn find_open(hay: &str) -> Option<(usize, usize, &'static str)> {
    TAGS.iter()
        .filter_map(|(o, c)| find_ci(hay, o).map(|i| (i, o.len(), *c)))
        .min_by_key(|(i, _, _)| *i)
}

/// 在 `hay` 中找最早出现的闭标签 → (起点, 闭标签)
fn find_close(hay: &str) -> Option<(usize, &'static str)> {
    TAGS.iter()
        .filter_map(|(_, c)| find_ci(hay, c).map(|i| (i, *c)))
        .min_by_key(|(i, _)| *i)
}

/// 若 `s` 的某个后缀可能是标签 `tag` 的开头（跨分片到达），返回需要保留的后缀起点；
/// 否则返回 `s.len()`（整段都可安全输出）。按字符边界判定。
fn maybe_tag_start(s: &str, tag: &str) -> usize {
    let n = s.len();
    let lo = n.saturating_sub(tag.len().saturating_sub(1));
    let mut i = lo;
    while i < n {
        if s.is_char_boundary(i) {
            let suffix = &s[i..];
            if suffix.len() < tag.len()
                && tag.as_bytes()[..suffix.len()].eq_ignore_ascii_case(suffix.as_bytes())
            {
                return i;
            }
        }
        i += 1;
    }
    n
}

/// 任意标签的开头都要保留时的切点
fn maybe_any_tag_start(s: &str) -> usize {
    TAGS.iter()
        .map(|(o, c)| maybe_tag_start(s, o).min(maybe_tag_start(s, c)))
        .min()
        .unwrap_or(s.len())
}

/// 批量剥离（非流式/最终结果用）：整段文本里的思考块全部去掉。
/// 剥离后为空是**合法结果**——调用方据此判定"没有拿到译文"，**不得**回退成原文。
pub fn strip_reasoning(text: &str) -> String {
    batch_state(text).visible
}

/// 对**完整文本**做一次剥离，返回处理到稳定的剥离器状态（流式侧也用它做权威
/// 结果与"破坏性规则触发后的状态重建"）
fn batch_state(text: &str) -> ReasoningStripper {
    let mut s = ReasoningStripper::new();
    s.pending.push_str(text);
    s.raw.push_str(text);
    while s.step() {
        s.needs_resync = false; // 批量场景无需重建（本态就是权威）
    }
    s
}

/// 流式剥离器：喂入 `content` 增量，返回当前**可显示**的累积文本。
///
/// 语义要点：
/// - 未闭合的开标签之后一律不输出（不产出 partial），避免思考闪现；
/// - 跨分片标签由尾巴缓冲兜住（`<th` + `ink>`）；
/// - [`finish`] 丢弃未闭合块、flush 合法尾巴。
///
/// [`finish`]: ReasoningStripper::finish
#[derive(Debug, Default)]
pub struct ReasoningStripper {
    /// 累积的原始文本——**最终结果的权威输入**。批量剥离可以回溯（"游离闭标签
    /// 删掉闭标签之前的整段"这条规则要求如此），而 `visible` 只增不减、撤不回
    /// 已经推送给界面的片段。对抗审计实证：流式逐字喂"思考预览 + 游离闭标签 +
    /// 正文"时，批量结果正确而流式结果把思考原样留在 `visible` 里（P1 泄漏）——
    /// 根因就是缺这份原始文本。标签字面量按本模块约定不在源码中出现。
    raw: String,
    /// 已确认可显示的文本
    visible: String,
    /// 尚未定性的尾巴（可能是标签开头，或块内待丢弃内容）
    pending: String,
    /// Some(闭标签) = 正处于未闭合的推理块内
    open_block: Option<&'static str>,
    /// 本轮流式处理触发了破坏性规则（游离闭标签删前缀）——需要依据 `raw` 重建
    /// 全部状态（含已推送的 visible），否则被删掉的思考片段会永久留在界面上
    needs_resync: bool,
}

impl ReasoningStripper {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前累积的可见文本
    pub fn visible(&self) -> &str {
        &self.visible
    }

    /// 喂入增量，返回清洗后的**累积**可见文本。
    ///
    /// **无步数上限**：跑到没有进展为止——`step` 每次返回 true 都严格消费掉
    /// pending 的前缀（find_open / find_close / 块内闭标签三路皆然），因此必然
    /// 终止；旧实现的上限 8 步会在"单次喂入 ≥5 个配对块"时把剩余原文连同思考
    /// 一起当正文交出去（实测复现），违反 INV-F。
    pub fn push(&mut self, delta: &str) -> String {
        self.raw.push_str(delta);
        self.pending.push_str(delta);
        while self.step() {}
        if self.needs_resync {
            // 破坏性规则（游离闭标签）触发：以整段原文为准重建——界面上下一帧
            // 的累积快照就是干净的（partial 是累积值，不是增量）
            self.needs_resync = false;
            let rebuilt = batch_state(&self.raw);
            self.visible = rebuilt.visible;
            self.pending = rebuilt.pending;
            self.open_block = rebuilt.open_block;
        }
        self.visible.clone()
    }

    /// 流结束：丢弃未闭合块与未成形的标签残尾，返回最终可见文本。
    ///
    /// 收尾时 `pending` 若仍非空，**只可能**是某个标签的前缀（`step` ④ 的尾巴
    /// 缓冲只保留"可能是标签开头"的后缀，其余全部已并入 visible）——一律丢弃：
    /// 宁可少一个孤立的 `<`/`◁`，也绝不把"看起来像标签"的东西当译文输出。
    pub fn finish(self) -> String {
        // 权威结果 = 对整段原文做一次批量剥离：与 `strip_reasoning` 逐字节一致
        // （回归测试 `streaming_matches_batch_on_same_input` 即钉住这条等价）。
        // 批量语义：未闭合块整块丢弃、未成形的标签残尾丢弃——都不并入正文。
        batch_state(&self.raw).visible
    }

    /// 单轮处理；返回是否有进展（可再试一轮）
    fn step(&mut self) -> bool {
        // ① 块内：等闭标签（同样大小写不敏感——`<Think>` 变体在大小写敏感下
        // 永不闭合，最终整段被判"未闭合块"丢弃，连译文一起丢）
        if let Some(close) = self.open_block {
            if let Some(at) = find_ci(&self.pending, close) {
                self.pending = self.pending[at + close.len()..].to_string();
                self.open_block = None;
                return true;
            }
            let keep = maybe_tag_start(&self.pending, close);
            self.pending = self.pending[keep..].to_string();
            return false;
        }
        // ② 开标签与游离闭标签：**先看谁在前**——闭标签在前时按"游离闭标签"
        // 删到该处（答案在其后），否则才进入推理块。旧实现无条件先处理开标签，
        // 导致 `答案</think>正文<think>S</think>尾` 里的孤立闭标签漏进译文。
        let open = find_open(&self.pending);
        let close = find_close(&self.pending);
        match (open, close) {
            (Some((oat, _, _)), Some((cat, cclose))) if cat < oat => {
                self.pending = self.pending[cat + cclose.len()..].to_string();
                // 破坏性：这之前的内容（可能已推送为 visible）整体作废
                self.needs_resync = true;
                return true;
            }
            (Some((at, olen, close_tag)), _) => {
                self.visible.push_str(&self.pending[..at]);
                self.pending = self.pending[at + olen..].to_string();
                self.open_block = Some(close_tag);
                return true;
            }
            (None, Some((at, cclose))) => {
                self.pending = self.pending[at + cclose.len()..].to_string();
                self.needs_resync = true;
                return true;
            }
            (None, None) => {}
        }
        // ④ 普通文本：保留可能构成标签开头的尾巴
        let keep = maybe_any_tag_start(&self.pending);
        let head = self.pending[..keep].to_string();
        self.visible.push_str(&head);
        self.pending = self.pending[keep..].to_string();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THINK: (&str, &str) = (TAGS[0].0, TAGS[0].1);
    /// 闭标签别名（测试里按名引用，避免在源码中书写完整标签字面量）
    const THINK_1: &str = TAGS[0].1;

    /// 标签表完整性守卫：这些字面量是剥离的唯一依据，一旦被编辑工具/编码环节
    /// 改写（曾发生：闭标签被写成全角变体 → 块永不闭合 → 译文恒空），此处立即失败
    #[test]
    fn tag_table_is_intact() {
        let want: [(&str, &str); 5] = [
            ("\u{3c}think\u{3e}", "\u{3c}/think\u{3e}"),
            ("\u{3c}thinking\u{3e}", "\u{3c}/thinking\u{3e}"),
            (
                "\u{3c}|channel|\u{3e}analysis\u{3c}|message|\u{3e}",
                "\u{3c}|end|\u{3e}",
            ),
            ("\u{25c1}think\u{25b7}", "\u{25c1}/think\u{25b7}"),
            ("\u{3c}|inner_prefix|\u{3e}", "\u{3c}|inner_suffix|\u{3e}"),
        ];
        assert_eq!(TAGS, want, "标签表被改动/破坏");
        assert_eq!(TAGS.len(), 5);
    }

    #[test]
    fn strip_paired_think_block() {
        // DeepSeek-R1/Qwen3 无 parser 部署的典型形态
        let raw = format!("{}先分析一下用户的问题{}敏捷的棕色狐狸。", THINK.0, THINK.1);
        assert_eq!(strip_reasoning(&raw), "敏捷的棕色狐狸。");
        let raw = format!("前言 {0}内部推理{1} 正文", THINK.0, THINK.1);
        assert_eq!(strip_reasoning(&raw), "前言  正文");
    }

    #[test]
    fn strip_stray_close_tag_keeps_answer_after() {
        // GLM-5.3 + 老 parser 实证形态：答案在闭标签之后
        let raw = format!("Simple question.{}2 + 2 = **4**", THINK.1);
        assert_eq!(strip_reasoning(&raw), "2 + 2 = **4**");
    }

    #[test]
    fn strip_harmony_analysis_channel() {
        let (o, c) = TAGS[2];
        let raw = format!("{o}用户要翻译…{c}channel-final 译文在这里");
        assert_eq!(strip_reasoning(&raw), "channel-final 译文在这里");
    }

    #[test]
    fn strip_kimi_and_apertus_forms() {
        let (ko, kc) = TAGS[3];
        assert_eq!(strip_reasoning(&format!("{ko}推理{kc}答案")), "答案");
        let (ao, ac) = TAGS[4];
        assert_eq!(strip_reasoning(&format!("{ao}推理{ac}答案")), "答案");
    }

    #[test]
    fn strip_across_chunks_never_leaks_partial_block() {
        // 标签被切成多片到达：中途不得泄露推理内容，最终输出无标签
        let mut s = ReasoningStripper::new();
        let (head, mid) = THINK.0.split_at(3); // "<th" + "ink>"
        let chunks = vec![
            head.to_string(),
            format!("{mid}推理很长的"),
            "内容".to_string(),
            format!("{}译文", THINK.1),
        ];
        let mut seen = Vec::new();
        for c in &chunks {
            seen.push(s.push(c));
        }
        for (i, v) in seen.iter().enumerate() {
            assert!(
                !v.contains("推理") && !v.contains("think"),
                "第 {i} 片泄露了推理内容: {v:?}"
            );
        }
        assert_eq!(s.finish(), "译文");
    }

    #[test]
    fn unclosed_block_stays_empty() {
        // 未闭合（流被截断/模型只输出思考）→ 判定为"没有正文"，不得回退成思考文本
        assert_eq!(
            strip_reasoning(&format!("{}这里全是推理，没有答案", THINK.0)),
            ""
        );
        assert_eq!(
            strip_reasoning(&format!("答案开始  {}接着都是推理", THINK.0)),
            "答案开始  "
        );
        // 流式：喂到一半也不得泄露
        let mut s = ReasoningStripper::new();
        assert_eq!(
            s.push(&format!("正常文本  {}推理中", THINK.0)),
            "正常文本  "
        );
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(
            strip_reasoning("普通译文，含 \u{3c} 与 \u{3e} 符号"),
            "普通译文，含 \u{3c} 与 \u{3e} 符号"
        );
        // 短文本不得被"尾巴缓冲"扣留（回归：曾把任何 N 字节尾巴一律扣下）
        let mut s = ReasoningStripper::new();
        assert_eq!(s.push("普通译"), "普通译");
        assert_eq!(s.push("文"), "普通译文");
        assert_eq!(s.finish(), "普通译文");
    }

    #[test]
    fn streaming_matches_batch_on_same_input() {
        let raw = format!("a {0}r1{1} b {0}r2{1} c", THINK.0, THINK.1);
        let mut s = ReasoningStripper::new();
        let mut streamed = Vec::new();
        for ch in raw.chars() {
            streamed.push(s.push(&ch.to_string()));
        }
        let last = s.finish();
        assert_eq!(last, strip_reasoning(&raw));
        assert_eq!(last, "a  b  c");
        // 最后一个增量即最终结果（尾巴已 flush，无未闭合块）
        assert_eq!(streamed.last().map(String::as_str), Some("a  b  c"));
    }

    // ── 上限/形态回归（2026-09-10 第二轮评审实证复现的三类泄漏） ──

    /// 单次喂入 5 个配对块（非流式路径 / 服务端一次下发整段）——旧实现 8 步上限
    /// 会在第 5 个块中途放弃，把 `…SECRET5</think>译文` 原样交出（已实测复现）
    #[test]
    fn many_paired_blocks_in_one_push_are_all_stripped() {
        let raw = format!(
            "{o}S1{c}{o}S2{c}{o}S3{c}{o}S4{c}{o}S5{c}译文",
            o = THINK.0,
            c = THINK.1
        );
        assert_eq!(strip_reasoning(&raw), "译文");
        // 流式：单片喂入整段同样不得泄漏（finish 是最终结果的来源）
        let mut s = ReasoningStripper::new();
        let _ = s.push(&raw);
        assert_eq!(s.finish(), "译文");
    }

    /// 单次喂入多个游离闭标签——不得因步数预算而残留（剥离规则是"删到该闭
    /// 标签（含）"，其之前的片段按定义属于思考预览，一并删除）
    #[test]
    fn many_stray_closes_in_one_push_are_all_stripped() {
        let raw = format!("AAA{}BBB", THINK.1.repeat(9));
        assert_eq!(strip_reasoning(&raw), "BBB");
    }

    /// 游离闭标签出现在开标签**之前**：旧实现先匹配开标签，把前段（含闭标签）
    /// 原样并入可见文本；正确语义是按"删到该闭标签（含）"处理
    #[test]
    fn stray_close_before_open_is_stripped() {
        let raw = format!("答案{c}正文{o}S{c}尾", o = THINK.0, c = THINK.1);
        assert_eq!(strip_reasoning(&raw), "正文尾");
    }

    /// 对抗审计实证的 P1 泄漏回归：**流式**逐字喂"思考预览 + 游离闭标签 + 正文"
    /// （GLM-5.3 形态）——旧实现把预览写进 visible 后无法撤回，思考永久留在译文里。
    /// 修法 = 以累积原文为权威（破坏性规则触发后整段重建）。
    #[test]
    fn streaming_strips_stray_close_shape() {
        let raw = format!(
            "预览…SECRET

{THINK_1}译文在这里"
        );
        assert_eq!(strip_reasoning(&raw), "译文在这里");
        let mut s = ReasoningStripper::new();
        let mut last = String::new();
        for ch in raw.chars() {
            last = s.push(&ch.to_string());
        }
        assert!(!last.contains("SECRET"), "流式可见文本不得含思考: {last:?}");
        assert_eq!(last, "译文在这里");
        assert_eq!(s.finish(), "译文在这里");
        // 分片（非逐字）喂入同样成立
        let mut s = ReasoningStripper::new();
        let _ = s.push("预览…SEC");
        let _ = s.push(
            "RET

",
        );
        assert_eq!(s.push(&format!("{THINK_1}译文在这里")), "译文在这里");
        assert_eq!(s.finish(), "译文在这里");
    }

    /// 标签大小写变体（`<Think>` / `<THINK>` / `<Thinking>`）——实测在大小写敏感的
    /// 实现下**整段漏进译文**（本轮加固：查找与块内闭合判定统一大小写不敏感）
    #[test]
    fn case_insensitive_tag_variants_are_stripped() {
        let upper_o = THINK.0.replace('t', "T"); // "<Think>"
        let upper_c = THINK.1.replace('t', "T"); // "</Think>"
        assert_eq!(
            strip_reasoning(&format!("{upper_o}SECRET{upper_c}译文")),
            "译文"
        );
        let shout_o = THINK.0.to_uppercase();
        let shout_c = THINK.1.to_uppercase();
        assert_eq!(
            strip_reasoning(&format!("{shout_o}SECRET{shout_c}译文")),
            "译文"
        );
        // 兼容写法（thinking）同样容忍大小写
        let (o2, c2) = TAGS[1];
        assert_eq!(
            strip_reasoning(&format!(
                "{}SECRET{}译文",
                o2.replace('t', "T"),
                c2.replace('t', "T")
            )),
            "译文"
        );
        // 正常小写形态不受影响
        assert_eq!(
            strip_reasoning(&format!("{}SECRET{}译文", THINK.0, THINK.1)),
            "译文"
        );
    }

    /// 收尾残尾（未成形的标签前缀）不得被当成正文——即使正文看起来像标签开头
    #[test]
    fn dangling_tag_prefix_is_not_flushed_as_text() {
        let mut s = ReasoningStripper::new();
        let _ = s.push("译文<thi");
        assert_eq!(s.finish(), "译文");
        // 正常结束的正文不受影响
        let mut s = ReasoningStripper::new();
        let _ = s.push("译文");
        assert_eq!(s.finish(), "译文");
    }
}
