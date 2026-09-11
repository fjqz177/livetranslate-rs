// lt-proto: 全工程共享的数据契约（settings / 事件 / ASR 结果）
pub mod asr_result;
pub mod events;
pub mod layout;
pub mod presets;
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
/// （`Cmd::StartDownload` 载荷改型 String→`Hub`/`ProxyMode`，D-79）= 4；
/// E6 死契约清理（`EngineError::WorkerExited`/`Cmd::SetTimeout` 删除，
/// 守卫校准发现零引用/零生产者；基准取消按钮接线补齐 `CancelBench`
/// 生产者，D-81）= 5；D-85 连接测试改造（`Cmd::TestTranslator` 载荷改型为
/// 结构体 + `probe_id`、`UiEvent::TestTranslatorResult` 改型为
/// `ProbeOutcome` 判别 + id 归位；`FailureKind`/`ThreadRole` 的新增属加法
/// 豁免不计）= 6。
pub const PROTO_VERSION: u32 = 6;

/// 连接探测总预算（秒，D-85 用户裁决 B）：编排域据此设 deadline，
/// UI 域据此设看门狗（+10s 裕量）。放契约层是为了让两个域同源又不越依赖边
/// （lt-ui 禁依赖 lt-orchestrator，架构 §3.1）。
pub const PROBE_TOTAL_BUDGET_SECS: u64 = 10;
/// 连接探测单步超时上限（秒）：用户超时更短时取用户值，更长时封顶
pub const PROBE_STEP_TIMEOUT_CAP_SECS: u32 = 10;

pub use asr_result::{language_display, AsrResult, EngineError, WordTs};
pub use events::{
    AppCommand, AudioDeviceChoice, AudioRole, BenchEvent, CaptureEvent, Cmd, DeviceList,
    DownloadEvent, DownloadFailKind, DownloadPhase, ExportFileMode, FailureKind, MicDeviceChoice,
    ModelFault, MonitorSample, ProbeOutcome, QueueId, SkipReason, ThreadDied, ThreadRole, UiEvent,
    UiMsg,
};
pub use layout::Hub;
pub use presets::{
    default_model_config, preset_by_key, preset_matching, ProviderPreset, PROVIDER_PRESETS,
};
pub use prompts::{DEFAULT_PROMPT, PROMPT_PRESETS};
pub use settings::{
    effective_currency, normalize_language, Currency, EngineKey, ModelConfig, ProxyMode, Settings,
    Style, SubtitleLine, SubtitleMode, ASR_ENGINES, DEFAULT_TEMPERATURE, OVERRIDE_KEYS,
    THINKING_STYLES,
};
