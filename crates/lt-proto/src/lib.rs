// lt-proto: 全工程共享的数据契约（settings / 事件 / ASR 结果）
pub mod asr_result;
pub mod events;
pub mod settings;

pub use asr_result::{language_display, AsrResult, EngineError, WordTs};
pub use events::{AudioDeviceChoice, Cmd, MicDeviceChoice, UiEvent, UiMsg};
pub use settings::{ModelConfig, Settings, Style, SubtitleLine, SubtitleMode};
