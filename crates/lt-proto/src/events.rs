//! 事件总线类型 —— 替代 Qt 信号。
//!
//! 三类消息全部汇入 `UiMsg`，经 winit `EventLoopProxy` 唤醒 UI 线程：
//! - [`UiEvent`]：工作线程 → UI（等价原版各 pyqtSignal 跨线程槽调用）
//! - [`Cmd`]：UI → 管道/worker（等价原版主线程直接调 App 方法）
//! - 托盘/菜单：tray-icon 与 muda 的原始事件转发

use crate::settings::{ModelConfig, Settings};

/// UI ↔ 工作线程的统一外层消息（EventLoopProxy 的 user event）
#[derive(Debug, Clone)]
pub enum UiMsg {
    /// 管道/下载/翻译等业务事件
    Event(UiEvent),
    /// 托盘图标事件（点击/双击等）
    Tray(String),
    /// muda 菜单点击（携带 MenuItemId）
    Menu(String),
}

/// 工作线程 → UI 的事件（对齐原版 SubtitleOverlay 的跨线程信号集）
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// 新识别消息（add_message_signal）
    AddMessage {
        id: u64,
        timestamp: String,   // "HH:MM:SS"
        original: String,
        lang: String,
        asr_ms: f64,
    },
    /// 译文完成（update_translation_signal）
    UpdateTranslation { id: u64, text: String, tl_ms: f64 },
    /// 流式译文增量（update_streaming_signal；UI 侧 50ms 节流）
    UpdateStreaming { id: u64, partial: String },
    /// 音频监视（update_monitor_signal：rms / vad 置信度 / 可选 mic_rms）
    UpdateMonitor { rms: f32, vad: f32, mic_rms: Option<f32> },
    /// 统计（update_stats_signal）
    UpdateStats { asr_n: u64, tl_n: u64, prompt_tokens: u64, completion_tokens: u64, cost: f64 },
    /// ASR 设备标签（"SenseVoice Small" 等；不可用时 "ASR unavailable"）
    AsrDevice(String),
    /// ASR 完全不可用
    AsrUnavailable,
    /// 日志行（tracing broadcast → 日志窗/下载框）
    LogLine { level: u8, target: String, msg: String },
    /// 下载进度文本（走日志流形态）
    DownloadProgress(String),
    /// 模型加载结束（切换引擎的 _ModelLoadDialog 关闭依据）
    ModelLoadDone { ok: bool, error: Option<String> },
}

/// UI → 管道的命令
#[derive(Debug, Clone)]
pub enum Cmd {
    Start,
    Pause,
    Resume,
    Stop,
    /// 引擎/模型/hub 任一变化触发（签名相同则管道侧自行跳过）
    SwitchEngine {
        engine: String,
        funasr_model: String,
        whisper_model_size: String,
        hub: String,
        language: String,
    },
    SetAsrLanguage(String),
    SetPadding { engine: String, secs: f32 },
    SetAudioDevice(AudioDeviceChoice),
    SetMicDevice(MicDeviceChoice),
    /// 切换翻译模型 / prompt / 超时等设置整体重放
    ApplySettings(Box<Settings>),
    SwitchTranslator(Box<ModelConfig>),
    SetTargetLanguage(String),
    SetTimeout(u32),
    IncrementalAsr { enabled: bool, interval: f32 },
}

/// 音频设备选择（对应 settings.audio_device 语义）
#[derive(Debug, Clone, PartialEq)]
pub enum AudioDeviceChoice {
    SystemDefault,
    Named(String),
    /// "__disabled__"：仅麦克风模式
    Disabled,
}

/// 麦克风设备选择（对应 settings.mic_device 语义）
#[derive(Debug, Clone, PartialEq)]
pub enum MicDeviceChoice {
    Off,
    Default,
    Named(String),
}

impl From<Option<String>> for AudioDeviceChoice {
    fn from(v: Option<String>) -> Self {
        match v.as_deref() {
            None => Self::SystemDefault,
            Some("__disabled__") => Self::Disabled,
            Some(n) => Self::Named(n.to_string()),
        }
    }
}

impl From<AudioDeviceChoice> for Option<String> {
    fn from(v: AudioDeviceChoice) -> Self {
        match v {
            AudioDeviceChoice::SystemDefault => None,
            AudioDeviceChoice::Disabled => Some("__disabled__".into()),
            AudioDeviceChoice::Named(n) => Some(n),
        }
    }
}

impl From<Option<String>> for MicDeviceChoice {
    fn from(v: Option<String>) -> Self {
        match v.as_deref() {
            None => Self::Off,
            Some("__default__") | Some("default") => Self::Default,
            Some(n) => Self::Named(n.to_string()),
        }
    }
}

impl From<MicDeviceChoice> for Option<String> {
    fn from(v: MicDeviceChoice) -> Self {
        match v {
            MicDeviceChoice::Off => None,
            MicDeviceChoice::Default => Some("__default__".into()),
            MicDeviceChoice::Named(n) => Some(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_choice_roundtrip() {
        for src in [None, Some("__disabled__".to_string()), Some("扬声器".to_string())] {
            let rt: Option<String> = AudioDeviceChoice::from(src.clone()).into();
            assert_eq!(rt, src);
        }
        for src in [
            None,
            Some("__default__".to_string()),
            Some("default".to_string()), // 原版兼容：两种默认写法等价
            Some("麦克风".to_string()),
        ] {
            let rt: Option<String> = MicDeviceChoice::from(src.clone()).into();
            match src.as_deref() {
                Some("default") => assert_eq!(rt.as_deref(), Some("__default__")),
                other => assert_eq!(rt.as_deref(), other),
            }
        }
    }
}
