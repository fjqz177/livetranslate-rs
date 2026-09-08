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
pub use state::{startup_flow, AppState, StartupFlow, WinId};
