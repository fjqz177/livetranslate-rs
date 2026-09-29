// lt-ui: egui 多窗口宿主 + 各窗口 + 托盘
pub mod app;
pub mod fonts;
pub mod notifications;
pub mod panel_diff;
pub mod state;
pub mod style;
pub mod tray;
pub mod windows;

pub use app::MultiWindowApp;
pub use state::{startup_flow, AppUi, StartupFlow, WinId};

/// 测试专用：i18n 全局语言表互斥守卫（D-121）。语言态是进程级单例，
/// 语言敏感 headless 测试并行跑会互相打飞断言（cargo 默认并行；D-120 的
/// 「渲染后 get_lang 复核重试」在**多个**语言敏感测试并存时结构性不稳）——
/// 持本锁把「切语言 + 渲染 + 断言」临界区串行化，宁等不飘。
/// `t_for_lang` 路径（不触全局）无需本锁。
#[cfg(test)]
pub(crate) fn lang_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LANG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LANG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
