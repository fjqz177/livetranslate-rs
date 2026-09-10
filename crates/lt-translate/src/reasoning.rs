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
    (
        "\u{3c}|inner_prefix|\u{3e}",
        "\u{3c}|inner_suffix|\u{3e}",
    ),
];

/// 在 `hay` 中找最早出现的开标签 → (起点, 开标签长, 闭标签)
fn find_open(hay: &str) -> Option<(usize, usize, &'static str)> {
    TAGS.iter()
        .filter_map(|(o, c)| hay.find(o).map(|i| (i, o.len(), *c)))
        .min_by_key(|(i, _, _)| *i)
}

/// 在 `hay` 中找最早出现的闭标签 → (起点, 闭标签)
fn find_close(hay: &str) -> Option<(usize, &'static str)> {
    TAGS.iter()
        .filter_map(|(_, c)| hay.find(c).map(|i| (i, *c)))
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
            if suffix.len() < tag.len() && tag.starts_with(suffix) {
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
    let mut stripper = ReasoningStripper::new();
    stripper.push(text);
    stripper.finish()
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
    /// 已确认可显示的文本
    visible: String,
    /// 尚未定性的尾巴（可能是标签开头，或块内待丢弃内容）
    pending: String,
    /// Some(闭标签) = 正处于未闭合的推理块内
    open_block: Option<&'static str>,
}

impl ReasoningStripper {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前累积的可见文本
    pub fn visible(&self) -> &str {
        &self.visible
    }

    /// 喂入增量，返回清洗后的**累积**可见文本
    pub fn push(&mut self, delta: &str) -> String {
        self.pending.push_str(delta);
        for _ in 0..8 {
            // 有界重试：正常文本一轮即稳定（上限防构造嵌套）
            if !self.step() {
                break;
            }
        }
        self.visible.clone()
    }

    /// 流结束：丢弃未闭合块，flush 合法尾巴，返回最终可见文本
    pub fn finish(mut self) -> String {
        if self.open_block.is_none() {
            let tail = std::mem::take(&mut self.pending);
            self.visible.push_str(&tail);
        }
        self.visible
    }

    /// 单轮处理；返回是否有进展（可再试一轮）
    fn step(&mut self) -> bool {
        // ① 块内：等闭标签
        if let Some(close) = self.open_block {
            if let Some(at) = self.pending.find(close) {
                self.pending = self.pending[at + close.len()..].to_string();
                self.open_block = None;
                return true;
            }
            let keep = maybe_tag_start(&self.pending, close);
            self.pending = self.pending[keep..].to_string();
            return false;
        }
        // ② 见开标签：其前文本可见，进入块内
        if let Some((at, olen, close)) = find_open(&self.pending) {
            self.visible.push_str(&self.pending[..at]);
            self.pending = self.pending[at + olen..].to_string();
            self.open_block = Some(close);
            return true;
        }
        // ③ 游离闭标签：删到该标签（含）——答案在其后
        if let Some((at, close)) = find_close(&self.pending) {
            self.pending = self.pending[at + close.len()..].to_string();
            return true;
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
            (
                "\u{3c}|inner_prefix|\u{3e}",
                "\u{3c}|inner_suffix|\u{3e}",
            ),
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
        assert_eq!(s.push(&format!("正常文本  {}推理中", THINK.0)), "正常文本  ");
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
}
