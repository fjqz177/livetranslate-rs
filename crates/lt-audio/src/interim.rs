//! 识别收尾路径的短句原语与跨段缓冲。
//!
//! 移植基准：`LiveTranslate/main.py`（逐字对照）
//! - [`is_short_utterance`] ← `_is_short_utterance`（L1389）
//! - [`pending_merge`]      ← `_do_interim_asr` L1508 / `_process_interim_final` L1608
//!   的短句 pending 前置拼接（`text = pending + text`，中间无分隔符）
//!
//! 历史注记（D-134）：本模块原为增量识别算法原语层（分句/回声剥离/比例裁剪/
//! 提交前缀 + `InterimState` 会话状态），D-128 起被 `realtime.rs` 状态机取代；
//! 实时通道整体裁撤后，分句/回声/裁剪原语随通道删除，仅存收尾路径仍在用的
//! 短句判定与 pending 合并，并自 `realtime.rs` 迁入收尾短句跨段缓冲
//! [`FlushPending`]（D-133 A5 机制，治「P.」「1.」碎片行）——本模块回归
//! 「收尾路径唯一文本状态容器」的角色。

// ─────────────────────────── 短句判定 ───────────────────────────

/// 短句判定（原版 `_is_short_utterance` 逐字）：字母数字字符 ≤8 视为
/// 噪声/语气词/碎片，缓冲进 [`FlushPending`] 等下句合并。
/// Python `str.isalnum` ↔ Rust `char::is_alphanumeric`（CJK/假名/谚文均计入）。
pub fn is_short_utterance(text: &str) -> bool {
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
    alnum <= 8
}

// ─────────────────────────── pending 合并 ───────────────────────────

/// 短句 pending 前置合并（原版 `_do_interim_asr` L1508-1510 /
/// `_process_interim_final` L1608-1610 逐字）：pending 非空则
/// `text = pending + text`，中间**无分隔符**（短句自带标点）。
/// 调用方传入各自已 strip 的文本；合并后调用方清空 pending。
pub fn pending_merge(pending: &str, text: &str) -> String {
    if pending.is_empty() {
        text.to_string()
    } else {
        format!("{pending}{text}")
    }
}

// ─────────────── 收尾路径短句跨段缓冲（D-133 A5） ───────────────

/// 收尾路径短句跨段缓冲的独立成行超时：3s 无后续句则不再等合并、独立提交
/// ——内容不蒸发，句尾短叹词（"Yes."）最迟 3s 落屏。
pub const FLUSH_PENDING_TIMEOUT_MS: u64 = 3000;

/// 收尾短句跨段缓冲（D-133 A5 机制；D-134 自 `realtime::RealtimeState` 迁入）：
/// 收尾段识别出的 ≤8 字符碎片（[`is_short_utterance`]）暂存于此，并下一句
/// 前置或超时独立成行——治「P.」「1.」碎片行。
///
/// 复位语义：**段级边界保留**（碎片等的是「下一句」而非本段尾巴，段边界
/// 冲掉就永远丢了）；**会话级边界全清**（切引擎/切设备/清空联动/管道重置）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlushPending {
    /// 跨段暂存的碎片文本（按序合并，无分隔符）
    pub text: String,
    /// 首条入账的 epoch 毫秒（0 = 空；SystemTime 粒度足够 3s 超时判定）
    pub at_ms: u64,
    /// 首条入账时的引擎检出语言（独立成行时供 commit_text 语言过滤——
    /// 零散碎片不带语言会被误丢）
    pub lang: String,
}

impl FlushPending {
    /// 短碎片入账（已有暂存则按序合并；空账本记首条时刻与语言）
    pub fn hold_flush_short(&mut self, text: &str, detected_lang: &str, now_ms: u64) {
        if self.text.is_empty() {
            self.at_ms = now_ms;
            self.lang = detected_lang.to_string();
        }
        self.text = pending_merge(&self.text, text.trim());
    }

    /// 取走跨段缓冲并与本句前置合并（碎片在前、本句在后；空则原样）
    pub fn take_merged(&mut self, text: &str) -> String {
        let (held, _) = self.take_flush_pending();
        if held.is_empty() {
            text.to_string()
        } else {
            pending_merge(&held, text)
        }
    }

    /// 取走跨段缓冲与其识别语言 `(文本, 语言)`（超时独立成行/退出冲刷用；
    /// 语言供 commit_text 过滤——零散碎片不带检出语言会被误丢）
    pub fn take_flush_pending(&mut self) -> (String, String) {
        self.at_ms = 0;
        let text = std::mem::take(&mut self.text);
        let lang = std::mem::take(&mut self.lang);
        (text, lang)
    }

    /// 跨段缓冲是否到独立成行时刻（≥[`FLUSH_PENDING_TIMEOUT_MS`] 无后续；
    /// 0 = 空，永不超时）
    pub fn flush_pending_expired(&self, now_ms: u64) -> bool {
        self.at_ms != 0 && now_ms.saturating_sub(self.at_ms) >= FLUSH_PENDING_TIMEOUT_MS
    }

    /// 是否无暂存内容
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 会话级全清（切引擎/切设备/清空联动/管道重置——跨段缓冲属本会话内容，
    /// 随会话一并清）
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

// ─────────────────────────── 测试 ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── 短句判定 ──

    #[test]
    fn short_utterance_english() {
        assert!(is_short_utterance("okay")); // 4 字母
        assert!(is_short_utterance("abcdefgh")); // 边界：恰 8
        assert!(!is_short_utterance("abcdefghi")); // 9 → 非短句
        assert!(!is_short_utterance("hello world again"));
    }

    #[test]
    fn short_utterance_cjk_counts_alnum() {
        assert!(is_short_utterance("好的")); // 2 汉字
        assert!(is_short_utterance("好，谢谢。")); // 标点不计，4 字
        assert!(is_short_utterance("こんにちは")); // 假名计入 isalnum
        assert!(!is_short_utterance("今天天气真的非常好了")); // 10 字 → 非短句
    }

    #[test]
    fn short_utterance_boundary_at_eight() {
        assert!(is_short_utterance("今天天气非常好啊")); // 8 → true（≤8）
        assert!(!is_short_utterance("今天天气非常好了呀")); // 9 → false
        assert!(!is_short_utterance("123456789")); // 9 数字 → false
    }

    // ── pending 合并 ──

    #[test]
    fn pending_merge_prepends_without_separator() {
        // 原版 L1508-1510：text = pending + text，中间无分隔符
        assert_eq!(
            pending_merge("好的。", "Hello world."),
            "好的。Hello world."
        );
    }

    #[test]
    fn pending_merge_empty_pending_is_identity() {
        assert_eq!(pending_merge("", "Hello world."), "Hello world.");
    }

    // ── 跨段短句缓冲（D-133 A5 真值表；自 realtime.rs 迁入改写） ──

    #[test]
    fn flush_short_holds_merges_and_expires() {
        let mut fp = FlushPending::default();
        assert!(!fp.flush_pending_expired(1_000));
        fp.hold_flush_short("P.", "en", 1_000);
        assert_eq!(fp.text, "P.");
        assert_eq!(fp.lang, "en");
        fp.hold_flush_short("1.", "en", 2_000);
        assert_eq!(fp.text, "P.1.", "同批碎片按序合并");
        assert_eq!(fp.at_ms, 1_000, "超时起点 = 首条入账时刻");
        assert!(!fp.flush_pending_expired(1_000 + FLUSH_PENDING_TIMEOUT_MS - 1));
        assert!(fp.flush_pending_expired(1_000 + FLUSH_PENDING_TIMEOUT_MS));
    }

    #[test]
    fn take_merged_prepends_held_before_text() {
        let mut fp = FlushPending::default();
        fp.hold_flush_short("好的。", "zh", 1);
        assert_eq!(fp.take_merged("Hello world."), "好的。Hello world.");
        assert!(fp.is_empty(), "取走即清（含语言）");
        assert_eq!(fp.take_merged("Second."), "Second.", "空账本原样放行");
    }

    #[test]
    fn take_flush_pending_returns_lang_and_resets_timer() {
        let mut fp = FlushPending::default();
        fp.hold_flush_short("Yes.", "en", 5);
        let (text, lang) = fp.take_flush_pending();
        assert_eq!(text, "Yes.");
        assert_eq!(lang, "en");
        assert_eq!((fp.text.as_str(), fp.at_ms, fp.lang.as_str()), ("", 0, ""));
    }

    #[test]
    fn reset_clears_all_and_segment_boundary_keeps_by_default() {
        let mut fp = FlushPending::default();
        fp.hold_flush_short("P.", "en", 1);
        // 段级边界：收尾路径不再调用任何复位（保留是默认行为——碎片等的是下一句）
        // 会话级边界：reset 全清
        fp.reset();
        assert!(fp.is_empty());
        assert_eq!(fp.at_ms, 0);
        assert!(!fp.flush_pending_expired(1_000 + FLUSH_PENDING_TIMEOUT_MS));
    }

    #[test]
    fn hold_trims_and_merges_without_separator() {
        let mut fp = FlushPending::default();
        fp.hold_flush_short("  P.  ", "en", 1);
        assert_eq!(fp.text, "P.", "入账 trim");
        fp.hold_flush_short(" 1.", "en", 2);
        assert_eq!(fp.text, "P.1.", "合并无分隔符");
    }
}
