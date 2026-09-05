//! AsrEngine trait：worker 进程内引擎的统一接口（原版各 asr_*.py 的共同形状）。

use lt_proto::{AsrResult, EngineError};

/// 单线程引擎（worker 内独占使用；sherpa/whisper 对象非 Sync）
pub trait AsrEngine: Send {
    /// 16k mono f32 → 识别结果；`pad` 已由引擎内部处理（set_input_padding 生效）
    fn transcribe(
        &mut self,
        audio: &[f32],
        word_timestamps: bool,
    ) -> Result<AsrResult, EngineError>;

    /// 源语言提示（"auto" | ISO 码）；不支持的引擎返回 Unsupported
    fn set_language(&mut self, _language: &str) -> Result<(), EngineError> {
        Err(EngineError::Unsupported)
    }

    /// 输入补零秒数（pad bucket 用）
    fn set_input_padding(&mut self, _pad_seconds: f32) -> Result<(), EngineError> {
        Err(EngineError::Unsupported)
    }
}
