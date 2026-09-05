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
        !matches!(self, TranslateError::Other(_) | TranslateError::Repetition(_))
    }

    /// UI 渲染文本（原版 `f"[error: {e}]"`）
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
            OpenAIError::JSONDeserialize(err, body) => {
                TranslateError::Other(format!("failed to decode response: {err}; body: {body}"))
            }
            OpenAIError::InvalidArgument(msg) => TranslateError::Other(msg),
            other => TranslateError::Other(other.to_string()),
        }
    }
}

fn classify_stream_error(e: StreamError) -> TranslateError {
    match e {
        // SSE 流中断/解析失败 → 连接类（reqwest 读超时仍会以 Reqwest 变体冒泡，
        // 不会落到这里）
        StreamError::EventStream(msg) => TranslateError::Connection(format!("stream failed: {msg}")),
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
        let e = TranslateError::Status { code: 500, message: "boom".into() };
        assert_eq!(e.ui_text(), "[error: api error 500: boom]");
    }

    #[test]
    fn expected_classification() {
        assert!(TranslateError::Connection("x".into()).is_expected());
        assert!(TranslateError::Timeout("x".into()).is_expected());
        assert!(TranslateError::Auth { code: 401, message: "x".into() }.is_expected());
        assert!(TranslateError::Status { code: 500, message: "x".into() }.is_expected());
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
}
