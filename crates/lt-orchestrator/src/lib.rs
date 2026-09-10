//! lt-orchestrator：编排域（架构 2.0 W3，docs/architecture-v2.md §3.1）。
//!
//! 自 lt-app/pipeline.rs 迁入：整条识别/翻译管道（采集撮合、VAD 状态机、
//! ASR 子进程管理、翻译池、引擎切换）+ 线程监督器 + 事件动脉（队列侧）。
//! 依赖白名单：proto/models/pipeline/asr/translate——**禁依赖 lt-ui/winit**
//! （UI 帧内事务一律下沉到本 crate 的线程域，回流经 EventSink）；用户可见
//! 文案不依赖 lt-i18n（方案白名单无此边），经 [`Msg`] 由组合根注入。
//!
//! W3 边界（纯结构移动，行为不变）：pipeline.rs/supervisor.rs 整文件
//! git mv 入本 crate；事件出口统一 [`EventSink`]（零 winit 依赖——动脉的
//! proxy 桥线程留在 lt-app，见其 artery.rs）。

pub mod download;
pub mod event_artery;
pub mod logging;
pub mod pipeline;
pub mod probe;
pub mod settings_bus;
pub mod supervisor;

pub use download::DownloadManager;
pub use event_artery::{EventArtery, EventSink};
pub use pipeline::Pipeline;
pub use settings_bus::{EffectiveSettings, SettingsBus, TlView};
pub use supervisor::{Policy, Supervisor};

/// 用户可见文案服务（i18n 注入点）。
///
/// 白名单约束（§3.1）：lt-orchestrator 不依赖 lt-i18n；管道线程域内需要
/// 用户可见文案的场景（翻译错误占位/测试连接回执）经本注入取——组合根
/// （lt-app）以 `Msg::new(|k| lt_i18n::t(k))` 提供。均为低频错误呈现场景，
/// 闭包动态分派成本可忽略。
#[derive(Clone)]
pub struct Msg {
    t: std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>,
}

impl Msg {
    pub fn new(t: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        Self { t: std::sync::Arc::new(t) }
    }

    pub fn t(&self, key: &str) -> String {
        (self.t)(key)
    }
}
