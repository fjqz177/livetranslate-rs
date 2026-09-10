//! 回应体检（W2/方案 §4.4）：一次 LLM 调用结束后，回答"这次到底拿到了什么"。
//!
//! 三个信号来自服务端本身（async-openai 0.41.3 均可读，见
//! `types/chat/chat_.rs`）：
//! - 正文（经思维链隔离后的 `content`）；
//! - `finish_reason`（是否被长度截断）；
//! - `usage.completion_tokens_details.reasoning_tokens`（预算是否被推理吃光）。
//!
//! 体检结论只用于**内部决策与结论化呈现**，不引入用户可见的"推理"概念。

/// 归一化后的结束原因（屏蔽流式/非流式两套 async-openai 枚举的差异）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishKind {
    Stop,
    Length,
    Other,
}

/// 回应体检结论
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseVerdict {
    /// 正文非空且未被截断
    Ok,
    /// 正文非空但被长度截断（照常显示，仅标注）
    OkTruncated,
    /// 正文为空且推理消耗了 token → 预算被思考吃光（方案 §4.4 的头号场景）
    EmptyReasoningBudget,
    /// 正文为空且被长度截断，推理量为 0 或未知
    EmptyTruncated,
    /// 正文为空且正常结束（模型主动空答复 / 未知原因）
    EmptyNoOutput,
}

impl ResponseVerdict {
    /// 是否拿到了可用正文
    pub fn has_text(self) -> bool {
        matches!(self, Self::Ok | Self::OkTruncated)
    }
}

/// 体检判定（顺序固定，不可交换——见方案 §4.4）
pub fn classify_response(
    text: &str,
    finish: Option<FinishKind>,
    reasoning_tokens: Option<u32>,
) -> ResponseVerdict {
    let truncated = matches!(finish, Some(FinishKind::Length));
    if !text.trim().is_empty() {
        return if truncated {
            ResponseVerdict::OkTruncated
        } else {
            ResponseVerdict::Ok
        };
    }
    if reasoning_tokens.unwrap_or(0) > 0 {
        return ResponseVerdict::EmptyReasoningBudget;
    }
    if truncated {
        return ResponseVerdict::EmptyTruncated;
    }
    ResponseVerdict::EmptyNoOutput
}

/// 原始字符串 → [`FinishKind`]（供未来接入其它协议/日志复盘使用）
pub fn finish_kind(raw: Option<&str>) -> Option<FinishKind> {
    raw.map(|s| match s {
        "stop" => FinishKind::Stop,
        "length" => FinishKind::Length,
        _ => FinishKind::Other,
    })
}

/// 非流式结束原因 → [`FinishKind`]
impl From<&async_openai::types::chat::CompletionFinishReason> for FinishKind {
    fn from(v: &async_openai::types::chat::CompletionFinishReason) -> Self {
        use async_openai::types::chat::CompletionFinishReason as R;
        match v {
            R::Stop => FinishKind::Stop,
            R::Length => FinishKind::Length,
            R::ContentFilter => FinishKind::Other,
        }
    }
}

/// 流式结束原因 → [`FinishKind`]
impl From<&async_openai::types::chat::FinishReason> for FinishKind {
    fn from(v: &async_openai::types::chat::FinishReason) -> Self {
        use async_openai::types::chat::FinishReason as R;
        match v {
            R::Stop => FinishKind::Stop,
            R::Length => FinishKind::Length,
            _ => FinishKind::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_text_is_ok() {
        assert_eq!(
            classify_response("译文", Some(FinishKind::Stop), None),
            ResponseVerdict::Ok
        );
        assert!(classify_response("译文", Some(FinishKind::Stop), None).has_text());
        // 仅空白视为空
        assert!(!classify_response("  \n ", Some(FinishKind::Stop), None).has_text());
    }

    #[test]
    fn truncated_text_still_counts_as_text() {
        assert_eq!(
            classify_response("半句译文", Some(FinishKind::Length), None),
            ResponseVerdict::OkTruncated
        );
        assert!(classify_response("半句译文", Some(FinishKind::Length), None).has_text());
    }

    #[test]
    fn empty_with_reasoning_tokens_is_budget_burn() {
        assert_eq!(
            classify_response("", Some(FinishKind::Length), Some(256)),
            ResponseVerdict::EmptyReasoningBudget
        );
        // 推理量优先于"截断"判定
        assert_eq!(
            classify_response("", Some(FinishKind::Stop), Some(1)),
            ResponseVerdict::EmptyReasoningBudget
        );
    }

    #[test]
    fn empty_truncated_without_reasoning() {
        assert_eq!(
            classify_response("", Some(FinishKind::Length), Some(0)),
            ResponseVerdict::EmptyTruncated
        );
        assert_eq!(
            classify_response("", Some(FinishKind::Length), None),
            ResponseVerdict::EmptyTruncated
        );
    }

    #[test]
    fn empty_stop_is_no_output() {
        assert_eq!(
            classify_response("", Some(FinishKind::Stop), Some(0)),
            ResponseVerdict::EmptyNoOutput
        );
        // finish_reason 全程缺失（部分服务端不返回）同样落此分支
        assert_eq!(
            classify_response("", None, None),
            ResponseVerdict::EmptyNoOutput
        );
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(finish_kind(Some("stop")), Some(FinishKind::Stop));
        assert_eq!(finish_kind(Some("length")), Some(FinishKind::Length));
        assert_eq!(finish_kind(Some("content_filter")), Some(FinishKind::Other));
        assert_eq!(finish_kind(None), None);
    }
}
