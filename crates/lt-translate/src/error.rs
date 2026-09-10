//! 翻译错误分类（对照原版 main.py `_translate_async` 的 except 块）：
//! APIConnectionError / APITimeoutError / AuthenticationError / APIStatusError /
//! TimeoutError / ConnectionError → warning 级；其余 → error 级。
//! UI 渲染统一为 `[error: ...]`，RepetitionError 单独渲染为 i18n 的 error_repetition。

use async_openai::error::{OpenAIError, StreamError};

#[derive(Debug, Clone, thiserror::Error)]
pub enum TranslateError {
    /// 连接失败（APIConnectionError / ConnectionError）
    #[error("connection error: {0}")]
    Connection(String),
    /// 超时（APITimeoutError / TimeoutError）
    #[error("{0}")]
    Timeout(String),
    /// 认证失败（AuthenticationError，HTTP 401/403）
    #[error("authentication error ({code}): {message}")]
    Auth { code: u16, message: String },
    /// HTTP 状态错误（APIStatusError）
    #[error("api error {code}: {message}")]
    Status { code: u16, message: String },
    /// 模型输出重复循环（RepetitionError）
    #[error("repetition loop detected: {0}")]
    Repetition(String),
    /// 其他
    #[error("{0}")]
    Other(String),
}

impl TranslateError {
    /// 是否属于"预期内"错误（原版按 warning 记日志，不吐堆栈）
    pub fn is_expected(&self) -> bool {
        !matches!(
            self,
            TranslateError::Other(_) | TranslateError::Repetition(_)
        )
    }

    /// W2/方案 §4.4：错误 → 用户可见的原因分类（UI 按分类选中文文案）。
    /// 本函数是 `lt_proto::FailureKind` 在错误侧的唯一生产者；UI 侧的穷尽 match
    /// 是唯一消费者（两侧共同满足死契约守卫的 ≥2 引用判定）。
    pub fn failure_kind(&self) -> lt_proto::FailureKind {
        use lt_proto::FailureKind as K;
        match self {
            TranslateError::Auth { .. } => K::Auth,
            TranslateError::Status { code: 404, .. } => K::NotFound,
            TranslateError::Status { code: 429, .. } => K::RateLimited,
            TranslateError::Status { .. } => K::ServerError,
            TranslateError::Connection(_) => K::Connection,
            TranslateError::Timeout(_) => K::Timeout,
            TranslateError::Repetition(_) => K::Repetition,
            TranslateError::Other(_) => K::Unknown,
        }
    }

    /// UI 渲染文本（原版 `f"[error: {e}]"`；W2 起只作为 `detail`——
    /// 主文案由 UI 按 `failure_kind` 选中文，不再直接展示英文原文）
    pub fn ui_text(&self) -> String {
        match self {
            // 原版 TimeoutError 展示为 "Translation exceeded {t}s total timeout"
            TranslateError::Timeout(msg) => format!("[error: {msg}]"),
            other => format!("[error: {other}]"),
        }
    }
}

impl From<OpenAIError> for TranslateError {
    fn from(e: OpenAIError) -> Self {
        match e {
            OpenAIError::Reqwest(err) => {
                if err.is_timeout() {
                    TranslateError::Timeout(format!("request timed out: {err}"))
                } else if err.is_connect() || err.is_request() || err.is_body() || err.is_decode() {
                    TranslateError::Connection(format!("{err}"))
                } else {
                    TranslateError::Other(format!("{err}"))
                }
            }
            OpenAIError::ApiError(resp) => {
                let code = resp.status_code.as_u16();
                let message = resp.api_error.message;
                if code == 401 || code == 403 {
                    TranslateError::Auth { code, message }
                } else {
                    TranslateError::Status { code, message }
                }
            }
            OpenAIError::StreamError(inner) => classify_stream_error(*inner),
            OpenAIError::JSONDeserialize(err, body) => classify_unparsed_error(&err, &body),
            OpenAIError::InvalidArgument(msg) => TranslateError::Other(msg),
            other => TranslateError::Other(other.to_string()),
        }
    }
}

/// 第二轮评审 ⑨：错误体不是 `{"error":{…}}` 形状（平铺 `{code,message}`、网关
/// HTML、纯文本）时，async-openai 连 HTTP 状态码一起丢掉，401/404/429 的中文
/// 提示会全部退化成"未知原因"。这里从原文里**尽力挖出**消息与状态码，
/// 挖不到才退回"未知原因"，且详情保持可读（不再把整块 JSON 倒给用户）。
fn classify_unparsed_error(err: &serde_json::Error, body: &str) -> TranslateError {
    let (code, message) = extract_error_fields(body);
    let message = message.unwrap_or_else(|| truncate_body(body, 300));
    match code {
        Some(c) if c == 401 || c == 403 => TranslateError::Auth { code: c, message },
        Some(c) => TranslateError::Status { code: c, message },
        None => TranslateError::Other(format!("响应无法解析（{err}）：{message}")),
    }
}

/// 从任意形状的错误体里挖 (状态码, 消息)：
/// - `{"error":{"code":401,"message":"…"}}`（GLM/DeepSeek/Kimi 官方形状）
/// - `{"code":401,"message":"…"}`（平铺——阿里 DashScope 文档形状）
/// - `{"message":"…"}` / `{"msg":"…"}`
/// - 非 JSON（网关 HTML/纯文本）→ 两者皆无，由调用方截断展示
fn extract_error_fields(body: &str) -> (Option<u16>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return (None, None);
    };
    let obj = v
        .get("error")
        .and_then(|e| e.as_object())
        .or_else(|| v.as_object());
    let Some(obj) = obj else {
        return (None, None);
    };
    let code = obj
        .get("code")
        .and_then(code_as_u16)
        .or_else(|| obj.get("status").and_then(code_as_u16));
    let message = obj
        .get("message")
        .and_then(|m| m.as_str())
        .or_else(|| obj.get("msg").and_then(|m| m.as_str()))
        .map(str::to_string);
    (code, message)
}

fn code_as_u16(v: &serde_json::Value) -> Option<u16> {
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    v.as_str().and_then(|s| s.trim().parse::<u16>().ok())
}

/// 按字符边界截断（防多字节切断），超长追加省略号
fn truncate_body(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

fn classify_stream_error(e: StreamError) -> TranslateError {
    match e {
        // SSE 流中断/解析失败 → 连接类（reqwest 读超时仍会以 Reqwest 变体冒泡，
        // 不会落到这里）
        StreamError::EventStream(msg) => {
            TranslateError::Connection(format!("stream failed: {msg}"))
        }
        StreamError::UnknownEvent(ev) => {
            TranslateError::Other(format!("unknown stream event: {}", ev.event))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_text_wraps_in_error_tag() {
        let e = TranslateError::Status {
            code: 500,
            message: "boom".into(),
        };
        assert_eq!(e.ui_text(), "[error: api error 500: boom]");
    }

    #[test]
    fn expected_classification() {
        assert!(TranslateError::Connection("x".into()).is_expected());
        assert!(TranslateError::Timeout("x".into()).is_expected());
        assert!(TranslateError::Auth {
            code: 401,
            message: "x".into()
        }
        .is_expected());
        assert!(TranslateError::Status {
            code: 500,
            message: "x".into()
        }
        .is_expected());
        assert!(!TranslateError::Other("x".into()).is_expected());
        assert!(!TranslateError::Repetition("x".into()).is_expected());
    }

    #[test]
    fn auth_codes_recognized() {
        for code in [401u16, 403] {
            let resp = async_openai::error::ApiErrorResponse {
                status_code: reqwest::StatusCode::from_u16(code).unwrap(),
                api_error: async_openai::error::ApiError {
                    message: "denied".into(),
                    r#type: None,
                    param: None,
                    code: None,
                },
            };
            match TranslateError::from(OpenAIError::ApiError(resp)) {
                TranslateError::Auth { code: c, .. } => assert_eq!(c, code),
                other => panic!("期望 Auth，实际 {other:?}"),
            }
        }
    }

    // ── 任意形状错误体（第二轮评审 ⑨） ──

    /// 平铺 `{code,message}`（阿里系形状）也要能挖出状态码
    #[test]
    fn flat_error_body_yields_status() {
        let (code, msg) = extract_error_fields(r#"{"code":429,"message":"Requests throttling triggered."}"#);
        assert_eq!(code, Some(429));
        assert_eq!(msg.as_deref(), Some("Requests throttling triggered."));
    }

    /// 嵌套 `{"error":{…}}`（GLM/DeepSeek/Kimi 形状）同样支持
    #[test]
    fn nested_error_body_yields_status() {
        let (code, msg) =
            extract_error_fields(r#"{"error":{"code":401,"message":"Invalid API key"}}"#);
        assert_eq!(code, Some(401));
        assert_eq!(msg.as_deref(), Some("Invalid API key"));
    }

    /// 字符串码与 `msg` 键（各家变体）；非 JSON 体（网关 HTML）不 panic
    #[test]
    fn string_code_and_non_json_body() {
        let (code, msg) = extract_error_fields(r#"{"code":"401","msg":"bad key"}"#);
        assert_eq!(code, Some(401));
        assert_eq!(msg.as_deref(), Some("bad key"));
        assert_eq!(extract_error_fields("<html>502 Bad Gateway</html>"), (None, None));
    }

    /// 挖到码 → 归入正确的 FailureKind（而不是"未知原因"）
    #[test]
    fn unparsed_body_keeps_failure_kind() {
        let e = classify_unparsed_error(
            &serde_json::from_str::<serde_json::Value>("[").unwrap_err(),
            r#"{"code":429,"message":"too many"}"#,
        );
        assert_eq!(e.failure_kind(), lt_proto::FailureKind::RateLimited);
        let e = classify_unparsed_error(
            &serde_json::from_str::<serde_json::Value>("[").unwrap_err(),
            "<html>502</html>",
        );
        assert_eq!(e.failure_kind(), lt_proto::FailureKind::Unknown);
        // 详情可读且被截断（不再整块 JSON 倒给用户）
        assert!(e.to_string().contains("502"));
        assert!(truncate_body(&"啊".repeat(500), 300).chars().count() <= 301);
    }
}
