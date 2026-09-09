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
    /// UI → 宿主的管道命令（backend 线程转发；AppShell 统一分发）
    Cmd(Cmd),
}

/// 工作线程 → UI 的事件（对齐原版 SubtitleOverlay 的跨线程信号集）
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// 新识别消息（add_message_signal）
    AddMessage {
        id: u64,
        timestamp: String, // "HH:MM:SS"
        original: String,
        lang: String,
        asr_ms: f64,
    },
    /// 译文完成（update_translation_signal）
    UpdateTranslation { id: u64, text: String, tl_ms: f64 },
    /// 流式译文增量（update_streaming_signal；UI 侧 50ms 节流）
    UpdateStreaming { id: u64, partial: String },
    /// 音频监视（update_monitor_signal：rms / vad 置信度 / 可选 mic_rms）
    UpdateMonitor {
        rms: f32,
        vad: f32,
        mic_rms: Option<f32>,
    },
    /// 统计（update_stats_signal）
    UpdateStats {
        asr_n: u64,
        tl_n: u64,
        prompt_tokens: u64,
        completion_tokens: u64,
        cost: f64,
    },
    /// ASR 设备标签（"SenseVoice Small" 等；不可用时 "ASR unavailable"）
    AsrDevice(String),
    /// ASR 完全不可用
    AsrUnavailable,
    /// 音频采集可用性（架构 2.0 W1/R4：三层错误可见性对称性补齐——ASR 有
    /// AsrUnavailable、翻译有 TranslatorUnavailable，音频此前仅 tracing）。
    /// 边沿触发语义对齐 AsrUnavailable：Unavailable 只在转坏瞬间发一次，
    /// 恢复前不重发；Recovered 只在转好瞬间发一次
    Capture(CaptureEvent),
    /// 被监督线程死亡（架构 2.0 W1/R1：线程 panic 不再黑洞；restarted =
    /// 监督器已按策略重启。W1 仅覆盖 capture/ASR/翻译池/bench/日志桥，
    /// 枚举面随波次扩充——加法豁免）
    ThreadDied(ThreadDied),
    /// 日志行（tracing broadcast → 日志窗/下载框）
    LogLine {
        level: u8,
        target: String,
        msg: String,
    },
    /// 下载进度/日志行（向导与缺模型下载对话框共用的日志流形态；
    /// 承载 Downloader 事件与下载期间 INFO 级 tracing 行）
    DownloadProgress(String),
    /// 下载成功（携带应生效的设置：向导=13 键默认块，缺模型=现有设置）
    DownloadSucceeded { settings: Box<Settings> },
    /// 下载失败（可重试；UI 恢复控件并显示 btn_retry）
    DownloadFailed(String),
    /// 下载被用户取消（DL-4/D-23）：UI 卡片回「已取消，进度已保留」态，
    /// 再次下载从 .incomplete 断点续传（仅追加成员，既有成员语义不变）
    DownloadCancelled,
    /// 模型加载开始（_ModelLoadDialog 打开依据；label 如 "SenseVoice Small"）
    ModelLoadStart(String),
    /// 模型加载结束（切换引擎的 _ModelLoadDialog 关闭依据）
    ModelLoadDone { ok: bool, error: Option<String> },
    /// 翻译装置构建失败（配置无效等）：整条翻译静默不可用的唯一用户可见通道
    TranslatorUnavailable { reason: String },
    /// 翻译配置「测试连接」结果（Cmd::TestTranslator 的回执）
    TestTranslatorResult {
        name: String,
        ok: bool,
        error: Option<String>,
        ms: u64,
    },
}

/// 音频采集角色（R4）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioRole {
    /// 系统声输出回环（loopback）
    Loopback,
    /// 麦克风输入（mic）
    Mic,
}

/// 音频采集可用性事件载荷（R4）
#[derive(Debug, Clone)]
pub enum CaptureEvent {
    /// 采集不可用（打开/读取失败；error = 末次错误的人类可读串）
    Unavailable { role: AudioRole, error: String },
    /// 从不可用恢复
    Recovered { role: AudioRole },
}

/// 被监督线程的身份（W1 先覆盖监督器首批接管面；随波次加法扩充）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRole {
    Capture,
    AsrMain,
    TlWorker,
    Bench,
    LogBridge,
    /// wasapi 可用性边沿事件 → UiEvent::Capture 的转发线程（W1/R4）
    AudioBridge,
}

/// 被监督线程死亡事件载荷（R1）
#[derive(Debug, Clone)]
pub struct ThreadDied {
    pub role: ThreadRole,
    /// panic 信息 + 位置（hook 已同时落 crash 文件与 tracing）
    pub detail: String,
    /// 监督器是否已按策略重启
    pub restarted: bool,
}

/// UI → 管道的命令
#[derive(Debug, Clone)]
pub enum Cmd {
    Pause,
    Resume,
    Stop,
    /// 首启向导/缺模型对话框：开始下载（hub: "ms"|"hf"；proxy: "none"|"system"|URL）
    StartDownload {
        hub: String,
        proxy: String,
    },
    /// 取消在途下载（DL-4/D-23）：backend 置会话取消令牌，Downloader 在文件
    /// 边界/重试间隙/读块检查点停止并保留 .incomplete 续传现场（仅追加成员）
    CancelDownload,
    /// 引擎/模型/hub 任一变化触发（签名相同则管道侧自行跳过）
    SwitchEngine {
        engine: String,
        funasr_model: String,
        whisper_model_size: String,
        hub: String,
        language: String,
    },
    SetAsrLanguage(String),
    SetPadding {
        engine: String,
        secs: f32,
    },
    SetAudioDevice(AudioDeviceChoice),
    SetMicDevice(MicDeviceChoice),
    /// 悬浮窗位置/尺寸防抖到期（宿主写盘；500ms 一次，原版 position_changed）
    PersistSettings(Box<Settings>),
    /// 切换翻译模型 / prompt / 超时等设置整体重放
    ApplySettings(Box<Settings>),
    SwitchTranslator(Box<ModelConfig>),
    SetTargetLanguage(String),
    SetTimeout(u32),
    IncrementalAsr {
        enabled: bool,
        interval: f32,
    },
    /// 翻译配置「测试连接」：构建临时装置发一次最简请求，回执 TestTranslatorResult
    TestTranslator(Box<ModelConfig>),
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
        for src in [
            None,
            Some("__disabled__".to_string()),
            Some("扬声器".to_string()),
        ] {
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
