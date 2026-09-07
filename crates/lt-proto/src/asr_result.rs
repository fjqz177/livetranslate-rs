//! ASR 结果与引擎错误类型（worker↔主进程共用）。

use serde::{Deserialize, Serialize};

/// 统一识别结果（对应原版 transcribe() 返回 dict）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AsrResult {
    pub text: String,
    /// ISO 码（"en"/"zh"/...），检测不到时 "auto"/"unknown"
    pub language: String,
    /// 原版 SenseVoice/Nano 直接回传 code；Whisper 查 LANGUAGE_DISPLAY。
    /// 现状注记（AH-10）：lt-ui/lt-app 无任何消费方（UI 语言显示由
    /// `language` 驱动）——契约冻结保留字段，新代码勿依赖其"显示名"语义
    pub language_name: String,
    /// word_timestamps=True 时（仅 Whisper 引擎支持）。
    /// 现状注记（AH-10）：当前管道恒传 false（interim 通道对重复增量太贵），
    /// 四引擎均不产出，恒 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordTs>>,
}

/// 词级时间戳（秒）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordTs {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

/// 引擎错误 —— recoverable 语义与原版 asr_worker 对齐：
/// 加载失败 = 不可恢复；单命令执行错误 = 可恢复（连续 3 次才致命）。
#[derive(Debug, Clone, thiserror::Error)]
pub enum EngineError {
    #[error("不支持的操作")]
    Unsupported,
    #[error("模型加载失败: {0}")]
    Load(String),
    #[error("{message}")]
    Runtime {
        message: String,
        recoverable: bool,
    },
    #[error("请求超时")]
    Timeout,
    #[error("worker 进程退出: {0}")]
    WorkerExited(String),
}

impl EngineError {
    pub fn recoverable(&self) -> bool {
        match self {
            EngineError::Unsupported => true,
            EngineError::Load(_) => false,
            EngineError::Runtime { recoverable, .. } => *recoverable,
            EngineError::Timeout => true,
            EngineError::WorkerExited(_) => false,
        }
    }
}

/// 语言显示名表（translator.py LANGUAGE_DISPLAY，29 项）
pub fn language_display(code: &str) -> &str {
    match code {
        "en" => "English", "ja" => "Japanese", "zh" => "Chinese", "ko" => "Korean",
        "fr" => "French", "de" => "German", "es" => "Spanish", "ru" => "Russian",
        "pt" => "Portuguese", "it" => "Italian", "nl" => "Dutch", "pl" => "Polish",
        "tr" => "Turkish", "ar" => "Arabic", "th" => "Thai", "vi" => "Vietnamese",
        "id" => "Indonesian", "ms" => "Malay", "hi" => "Hindi", "uk" => "Ukrainian",
        "cs" => "Czech", "ro" => "Romanian", "el" => "Greek", "hu" => "Hungarian",
        "sv" => "Swedish", "da" => "Danish", "fi" => "Finnish", "no" => "Norwegian",
        "he" => "Hebrew",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_recoverable_semantics() {
        assert!(!EngineError::Load("x".into()).recoverable());
        assert!(EngineError::Runtime { message: "x".into(), recoverable: true }.recoverable());
        assert!(!EngineError::Runtime { message: "x".into(), recoverable: false }.recoverable());
        assert!(!EngineError::WorkerExited("1".into()).recoverable());
    }

    #[test]
    fn language_display_fallback() {
        assert_eq!(language_display("en"), "English");
        assert_eq!(language_display("zz"), "zz"); // 未知码原样返回
    }
}
