// lt-proto: 全工程共享的数据契约（settings / 事件 / ASR 结果）
pub mod asr_result;
pub mod events;
pub mod layout;
pub mod prompts;
pub mod settings;

/// 契约结构版本（架构 2.0 W2，§3.3）：repo 卫生标记——结构性变更
/// （变体删除/改名/改型/改语义）时递增，评审与守护脚本对照用；
/// 纯新增变体/字段（加法豁免）不递增。非 wire 版本：唯一真进程边界是
/// worker IPC（同 exe 同版本），不存在线级兼容面。
///
/// 历史：W0/W1 无结构变更（1）；W2 契约类型化（Menu/Tray→AppCommand、
/// DownloadProgress→Download、DownloadFailed 类型化、完成哨兵→Bench、
/// 新增 Events 批量变体）= 2；W4 设置总线（`UiMsg::Cmd` 删除——控制面
/// mpsc 由 AppShell 直排，INV2；lt-backend 线程退役）= 3；E2 值域枚举化
/// （`Cmd::StartDownload` 载荷改型 String→`Hub`/`ProxyMode`，D-79）= 4。
pub const PROTO_VERSION: u32 = 4;

pub use asr_result::{language_display, AsrResult, EngineError, WordTs};
pub use events::{
    AppCommand, AudioDeviceChoice, AudioRole, BenchEvent, CaptureEvent, Cmd, DeviceList,
    DownloadEvent, DownloadFailKind, DownloadPhase, ExportFileMode, MicDeviceChoice, MonitorSample,
    QueueId, ThreadDied, ThreadRole, UiEvent, UiMsg,
};
pub use layout::Hub;
pub use prompts::{DEFAULT_PROMPT, PROMPT_PRESETS};
pub use settings::{
    normalize_language, ASR_ENGINES, EngineKey, ModelConfig, OVERRIDE_KEYS, ProxyMode, Settings,
    Style, SubtitleLine, SubtitleMode, THINKING_STYLES,
};
