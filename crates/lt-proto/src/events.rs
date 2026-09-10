//! 事件总线类型 —— 替代 Qt 信号。
//!
//! 三类消息全部汇入 `UiMsg`，经 winit `EventLoopProxy` 唤醒 UI 线程：
//! - [`UiEvent`]：工作线程 → UI（等价原版各 pyqtSignal 跨线程槽调用）
//! - [`Cmd`]：UI → 管道/worker（等价原版主线程直接调 App 方法）
//! - 托盘/菜单：tray-icon 与 muda 的原始事件转发

use crate::settings::{ModelConfig, ProxyMode, Settings};

/// UI ↔ 工作线程的统一外层消息（EventLoopProxy 的 user event）
#[derive(Debug, Clone)]
pub enum UiMsg {
    /// 管道/下载/翻译等业务事件
    Event(UiEvent),
    /// 事件动脉批量变体（W2：桥线程单次 wake 排空 ≤256 条——日志风暴下
    /// winit 唤醒速率与帧率同阶而非逐条 PostMessage）
    Events(Vec<UiEvent>),
    /// 应用级命令（W2 起替代 `Menu(String)`/`Tray(String)` 字符串协议；托盘
    /// 与悬浮窗菜单同源，变体全集见 [`AppCommand`]）
    AppCommand(AppCommand),
    // W4：`Cmd(Cmd)` 变体已删——cmd mpsc 由 AppShell 在 about_to_wait 直排
    //（lt-backend 线程退役，控制面 UI→mpsc→shell 两跳，INV2）。`Cmd` 枚举
    // 本身保留为该 mpsc 的载荷契约。
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
    /// 音频监视（W2 起走快照格 `MonitorSample`：capture 写 ArcSwap，UI 以
    /// ~33ms 节拍读格重绘——替代逐 chunk 事件的 31/s 唤醒，D-67/R23；
    /// update_monitor_signal 语义等价，契约侧从「事件」变为「格」）
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
    /// 监督器已按策略重启。W1 实际覆盖 capture/ASR/翻译池/音频状态转发/日志桥
    /// ——bench 仍在 lt-translate 内裸 spawn（依赖方向所限，监督器暂不可及），
    /// 剩余枚举面随波次扩充——加法豁免）
    ThreadDied(ThreadDied),
    /// 日志行（tracing broadcast → 日志窗/下载框）
    LogLine {
        level: u8,
        target: String,
        msg: String,
    },
    /// 下载进度（W2：替代 `DownloadProgress(String)` —— 由字符串协议淘出
    /// `\t` 机器段进类型化事件；人类可读行改由 `LogLine{target:"download"}` 携带）
    Download(DownloadEvent),
    /// 下载成功（携带应生效的设置：向导=13 键默认块，缺模型=现有设置）
    DownloadSucceeded { settings: Box<Settings> },
    /// 下载失败（可重试；UI 恢复控件并显示 btn_retry）。W2：类型化 kind
    /// 替代字符串前缀还原（checksum/cancel 此前落 Other，分类从此精确）
    DownloadFailed {
        kind: DownloadFailKind,
        message: String,
    },
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
    /// 性能基准流（W2：替代 `LogLine{target:"benchmark"}` + 完成哨兵；
    /// 逐行输出 + 完成语义类型化，基准窗不再借道日志总线）
    Bench(BenchEvent),
    /// 有界队列水位（R15②：翻译池满丢最旧时上报；慢 LLM 积压不再静默）
    QueuePressure {
        queue: QueueId,
        dropped_total: u64,
    },
    /// 音频设备枚举结果（W5/R13：`Cmd::RefreshDevices` 的回执——设备探测下线
    /// 到编排域 Supervisor 一次性线程，UI 帧内不再阻塞 COM 枚举）
    Devices(DeviceList),
    /// 导出文件路径回执（W5/R19：rfd 保存框移出事件循环线程——同步对话框挪
    /// Supervisor 一次性线程，path=None = 用户取消）
    ExportSave {
        mode: ExportFileMode,
        path: Option<String>,
    },
    /// 字幕背景图选择回执（W5/R19：同上——path=None = 用户取消）
    BgImagePicked { path: Option<String> },
    /// 二次启动激活（W6/R11②/WD-5）：第二个实例检测到首实例互斥量并已
    /// 经 message-only 窗投递激活消息——首实例 UI 显示面板并前置。
    /// 纯新增变体（冻结规则加法豁免，PROTO_VERSION 不递增）。
    SecondInstance,
    /// 跳过翻译（W2/方案 §4.4/INV-A：**空译文≠同语言**——「没有译文」的每一条
    /// 出口从此携带机器可读原因，UI 不再靠"字符串是否为空"猜语义）。
    /// 纯新增变体（冻结规则加法豁免，PROTO_VERSION 不递增）。
    TranslationSkipped { id: u64, reason: SkipReason },
    /// 翻译失败/无输出（W2/方案 §4.4）。`detail` = provider 原文或体检摘要，
    /// 只进日志与悬浮提示，主文案由 UI 按 `kind` 选中文 i18n。
    /// 纯新增变体（冻结规则加法豁免，PROTO_VERSION 不递增）。
    TranslationFailed {
        id: u64,
        kind: FailureKind,
        detail: String,
        tl_ms: f64,
    },
}

/// 跳过翻译的原因（W2）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 源语言与目标语言相同：按原版语义免翻译，UI 显示 `(相同语言)`
    SameLanguage,
}

/// 翻译失败/无输出的原因（W2；UI 据此选 i18n 文案，禁止 `_ =>` 兜底——
/// 死契约守卫要求每个变体都有消费点）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// 模型无输出（体检 EmptyReasoningBudget / EmptyNoOutput）
    Empty,
    /// 输出被长度截断（体检 EmptyTruncated / OkTruncated）
    Truncated,
    /// 请求超时
    Timeout,
    /// 鉴权失败（401/403）
    Auth,
    /// 模型/端点不存在（404）
    NotFound,
    /// 限流（429）
    RateLimited,
    /// 服务端错误（其余 4xx/5xx）
    ServerError,
    /// 连接失败（服务未启动、网络不可达、代理错误）
    Connection,
    /// 模型输出重复循环
    Repetition,
    /// 其他未知错误
    Unknown,
}

/// 应用级命令（托盘菜单与悬浮窗菜单同源；W2 替代 `Menu(String)` 字符串协议。
/// 变体全集 = 托盘 ids（lt-ui/tray.rs [ids]）+ quit 专用命令）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    /// 暂停/恢复管道
    Pause,
    /// 悬浮窗 显示/隐藏
    OverlayToggle,
    /// 控制面板 显示/隐藏
    ShowPanel,
    /// 退出应用（宿主侧带确认框）
    Quit,
}

impl AppCommand {
    /// muda 菜单 id → 命令。id 清单与 lt-ui/tray.rs `ids` 常量一一对应；
    /// 状态行等只读项无事件故无变体；未知 id → None（不做字符串预言机）
    pub fn from_menu_id(id: &str) -> Option<Self> {
        Some(match id {
            "tray_pause" => Self::Pause,
            "tray_hide_overlay" => Self::OverlayToggle,
            "tray_show_panel" => Self::ShowPanel,
            "quit" => Self::Quit,
            _ => return None,
        })
    }
}

/// 下载进度事件（W2 替代 DownloadProgress(String) 的 `\t` 机器段协议；
/// UI 据字段驱动进度条，人读段由 UI 侧按同格式生成）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadEvent {
    /// 仓库标识（"modelscope/{org}--{name}" 等）
    pub repo: String,
    pub file: String,
    /// 当前第 index/count 个文件（1 起；DL-3 的 k/n）
    pub index: u32,
    pub count: u32,
    pub done: u64,
    /// None = 未知（UI 走日志模式）
    pub total: Option<u64>,
    pub phase: DownloadPhase,
}

/// 下载进度阶段（UI 进度条状态语义；当前下载器仅产 Progress，Start/
/// Integrity 为契约预留）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    Start,
    Progress,
    Integrity,
}

/// 下载失败分类（自 lt-models `FailKind` 迁入；谓词与全组合测试随迁。
/// Display 前缀 `[net]` 等不再作为 UI 分流载体——契约字符串协议禁令 INV9）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadFailKind {
    /// 网络层失败（reqwest：超时/连接拒绝/DNS/TLS/读取中断）
    Net,
    /// HTTP 状态码非 2xx（含 404 仓库缺失、限流、服务端错误）
    Http(u16),
    /// 磁盘/IO 失败（创建目录、写入、rename）
    Disk,
    /// 长度校验失败（下载不完整）
    Length,
    /// sha256 内容校验失败（AH-5/H8：内容损坏是确定性的，重试/换源无解）
    Checksum,
    /// 用户取消（保留 .incomplete 续传现场；DL-4）
    Cancelled,
    /// 未知（非下载器 DlError 的防御兜底；UI 示通用提示）
    Other,
}

impl DownloadFailKind {
    /// 前缀串（DlError Display 形态 `[{prefix}] {message}`；人读日志用，
    /// 不再是 UI 分流载体——保留仅为可读性）
    pub fn prefix(self) -> &'static str {
        match self {
            DownloadFailKind::Net => "net",
            DownloadFailKind::Http(404) => "http-404",
            DownloadFailKind::Http(_) => "http",
            DownloadFailKind::Disk => "disk",
            DownloadFailKind::Length => "length",
            DownloadFailKind::Checksum => "checksum",
            DownloadFailKind::Cancelled => "cancel",
            DownloadFailKind::Other => "other",
        }
    }

    /// 是否值得退避重试（DL-2/F5 快速失败）：网络中断与长度不完整可续传重试；
    /// 5xx/429 属服务端暂时性；其余 4xx（404 缺失/401 私有/403 禁止）、磁盘、
    /// 取消均为永久态，立即返回不再白等 1/4/16s。
    pub fn retryable(self) -> bool {
        match self {
            DownloadFailKind::Net | DownloadFailKind::Length => true,
            DownloadFailKind::Http(s) => s >= 500 || s == 429,
            DownloadFailKind::Disk
            | DownloadFailKind::Checksum
            | DownloadFailKind::Cancelled
            | DownloadFailKind::Other => false,
        }
    }

    /// 是否值得换另一 hub 回落（DL-5）：仓库缺失或网络不可达才回落；
    /// 磁盘/长度/取消等问题换源无解。Checksum 不回落——同一注册表哈希对
    /// 两源一致（镜像同步），换源无解（AH-5）
    pub fn fallback_candidate(self) -> bool {
        matches!(self, DownloadFailKind::Net | DownloadFailKind::Http(404))
    }
}

/// 性能基准流事件（W2：替代 `LogLine{target:"benchmark"}` 的 完成哨兵；
/// 基准窗独享通道，不再借日志总线搬运机器控制流——INV9）
#[derive(Debug, Clone)]
pub enum BenchEvent {
    /// 逐行输出（格式稳定，原版 benchmark.py 样式）
    Line(String),
    /// 全部完成（ok = 无失败模型；elapsed_ms = 全程耗时）
    Finished { ok: bool, elapsed_ms: u64 },
}

/// 有界队列身份（QueuePressure 水位告警定位）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueId {
    /// 翻译池（R15：待译段 keep-latest 64，满丢最旧）
    Translation,
}

/// 音频监视快照（W2 快照格：capture 线程写 ArcSwap，UI 以 ~33ms 节拍读格
/// 重绘——替代每 chunk 一条 监视事件 的逐条唤醒；D-67 视觉等价）
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MonitorSample {
    pub rms: f32,
    pub vad: f32,
    pub mic_rms: Option<f32>,
    /// 单调序号：写侧递增；读者廉价比对「是否变了」（浮点比对不可靠）
    pub seq: u64,
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
    /// W2 预留（当前仍在 lt-translate 内裸 spawn，lt-app 监督器暂不可及；
    /// 转入 orchestrator 后按方案线程表转 Never 接管）
    Bench,
    LogBridge,
    /// wasapi 可用性边沿事件 → UiEvent::Capture 的转发线程（W1/R4）
    AudioBridge,
    /// 事件动脉桥线程（W2：动脉 → `UiMsg::Events` 批量投递；INV1 白名单
    /// "proxy 生产者仅动脉桥"的对位身份）
    ArteryBridge,
    /// 设备探测一次性线程（W5：`Cmd::RefreshDevices` 的 Supervisor 会话；
    /// Policy::Never——死亡仅上报）
    DeviceProbe,
    /// 文件对话框一次性线程（W5/R19：rfd 同步对话框移离事件循环线程；
    /// Policy::Never）
    FileDialog,
    /// 下载会话线程（W7：`DownloadManager` 经 Supervisor 出生；Policy::Never
    /// ——限时 join 语义由会话自管理的子线程承接，父线程死亡上报即可）
    Download,
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

/// 音频设备枚举结果（W5/R13：DeviceCache 的契约形态——UI 侧缓存替换为事件载荷）
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceList {
    /// 输出设备名（默认设备恒在列表内；不含 "-- 默认 --" 合成项）
    pub outputs: Vec<String>,
    /// 输入设备名（不含 "__default__" 合成项）
    pub inputs: Vec<String>,
    /// 系统默认输出设备名（None = 枚举失败/无设备）
    pub default_output: Option<String>,
}

/// 导出文件模式（W5：原 strings "original"/"translation"/"all" 的契约形态；
/// 由 lt-ui 侧映射回原版行格式）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFileMode {
    Original,
    Translation,
    All,
}

impl ExportFileMode {
    /// 文件名后缀（原版 export 的文件名 livetrans_<ts>_<suffix>.txt）
    pub fn suffix(self) -> &'static str {
        match self {
            ExportFileMode::Original => "original",
            ExportFileMode::Translation => "translation",
            ExportFileMode::All => "all",
        }
    }
}

/// UI → 管道的命令
#[derive(Debug, Clone)]
pub enum Cmd {
    Pause,
    Resume,
    Stop,
    /// 首启向导/缺模型对话框：开始下载（E2/D-79：载荷改型为值域枚举——
    /// UI 经 `Settings::hub()`/`proxy_mode()` 透镜转换，消灭未知字符串
    /// 静默暗默认）
    StartDownload {
        hub: crate::layout::Hub,
        proxy: ProxyMode,
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
    // E6/D-81：`SetTimeout(u32)` 删除——死契约守卫校准发现零生产者
    //（翻译页超时改动走 ApplySettings 整体重放），shell 处理臂随删
    IncrementalAsr {
        enabled: bool,
        interval: f32,
    },
    /// 翻译配置「测试连接」：构建临时装置发一次最简请求，回执 TestTranslatorResult
    TestTranslator(Box<ModelConfig>),
    /// 重新枚举音频设备（W5/R13：面板识别页首次进入/刷新按钮；编排域起
    /// Supervisor 一次性探测线程 → `UiEvent::Devices` 回执——UI 帧内不再
    /// 阻塞 COM 枚举）
    RefreshDevices,
    /// 启动性能基准（W5/R13：编排域起 Supervisor 一次性线程跑 `run_benchmark`，
    /// 输出经动脉 `UiEvent::Bench` 回流——UI 侧不再直持线程与事件旁路）
    RunBench {
        models: Vec<ModelConfig>,
        src: String,
        tgt: String,
        timeout: u32,
        prompt: String,
    },
    /// 取消在途基准（supervisor 线程在模型边界轮询取消标志）
    CancelBench,
    /// 导出保存框（W5/R19：rfd 同步对话框移出错线程——编排域 Supervisor
    /// 一次性线程弹框，选中路径经 `UiEvent::ExportSave` 回执）
    PickExportFile {
        mode: ExportFileMode,
        default_name: String,
        dialog_title: String,
    },
    /// 字幕背景图选择框（W5/R19：同上 → `UiEvent::BgImagePicked` 回执）
    PickBgImage { dialog_title: String },
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

    /// W2：托盘菜单 id 全集 → AppCommand（防 tray.rs ids 与映射漂移；
    /// STATUS 为只读状态行无事件，不产生命令）
    #[test]
    fn app_command_menu_id_mapping() {
        assert_eq!(AppCommand::from_menu_id("tray_pause"), Some(AppCommand::Pause));
        assert_eq!(
            AppCommand::from_menu_id("tray_hide_overlay"),
            Some(AppCommand::OverlayToggle)
        );
        assert_eq!(
            AppCommand::from_menu_id("tray_show_panel"),
            Some(AppCommand::ShowPanel)
        );
        assert_eq!(AppCommand::from_menu_id("quit"), Some(AppCommand::Quit));
        // 未知/只读项 → None（不许静默当作某命令）
        assert_eq!(AppCommand::from_menu_id("tray_status"), None);
        assert_eq!(AppCommand::from_menu_id(""), None);
    }

    /// DL-2/F5：重试分类表——net/length/5xx/429 可重试；404/401/403/磁盘/
    /// 取消立即失败（自 lt-models 随迁）
    #[test]
    fn fail_kind_retry_classification() {
        assert!(DownloadFailKind::Net.retryable());
        assert!(DownloadFailKind::Length.retryable());
        assert!(DownloadFailKind::Http(500).retryable());
        assert!(DownloadFailKind::Http(503).retryable());
        assert!(DownloadFailKind::Http(429).retryable());
        assert!(!DownloadFailKind::Http(404).retryable());
        assert!(!DownloadFailKind::Http(401).retryable());
        assert!(!DownloadFailKind::Http(403).retryable());
        assert!(!DownloadFailKind::Http(418).retryable());
        assert!(!DownloadFailKind::Disk.retryable());
        assert!(!DownloadFailKind::Cancelled.retryable());
        assert!(!DownloadFailKind::Checksum.retryable());
        assert!(!DownloadFailKind::Other.retryable());
    }

    /// DL-5 前置：回落候选 = 仓库缺失或网络不可达；其余换源无解
    #[test]
    fn fail_kind_fallback_candidates() {
        assert!(DownloadFailKind::Http(404).fallback_candidate());
        assert!(DownloadFailKind::Net.fallback_candidate());
        assert!(!DownloadFailKind::Http(401).fallback_candidate());
        assert!(!DownloadFailKind::Http(500).fallback_candidate());
        assert!(!DownloadFailKind::Length.fallback_candidate());
        assert!(!DownloadFailKind::Disk.fallback_candidate());
        assert!(!DownloadFailKind::Checksum.fallback_candidate());
        assert!(!DownloadFailKind::Cancelled.fallback_candidate());
    }

    /// 前缀串（DlError Display 形态；字符串不再承载契约语义，仅人读日志）
    #[test]
    fn fail_kind_prefixes() {
        assert_eq!(DownloadFailKind::Net.prefix(), "net");
        assert_eq!(DownloadFailKind::Http(404).prefix(), "http-404");
        assert_eq!(DownloadFailKind::Http(500).prefix(), "http");
        assert_eq!(DownloadFailKind::Disk.prefix(), "disk");
        assert_eq!(DownloadFailKind::Length.prefix(), "length");
        assert_eq!(DownloadFailKind::Checksum.prefix(), "checksum");
        assert_eq!(DownloadFailKind::Cancelled.prefix(), "cancel");
    }
}
