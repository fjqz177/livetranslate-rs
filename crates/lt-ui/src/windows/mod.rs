//! 窗口 UI 分发（egui 0.36：容器一律吃 `&mut Ui`）。
//!
//! - overlay：全量复刻（见 [`overlay`] 模块文档）
//! - subtitle：全量复刻（见 [`subtitle`] 模块文档）
//! - setup：启动流对话框（向导/缺模型下载/模型加载）
//! - panel：控制面板（M4.3 第一批：框架 + 常规页 + 识别页，见 [`panel`]）
//! - log：日志窗（M4.5）

use crate::state::{AppState, WinId};
use egui::Ui;

pub mod logwin;
pub mod overlay;
pub mod panel;
pub mod setup;
pub mod subtitle;

/// 按窗口分发（根 Ui 由宿主经 ctx.run_ui 提供）
pub fn dispatch(win: WinId, ui: &mut Ui, state: &mut AppState) {
    match win {
        WinId::Overlay => overlay::overlay_ui(ui, state),
        WinId::Subtitle => subtitle::subtitle_ui(ui, state),
        WinId::Panel => panel::panel_ui(ui, state),
        WinId::Log => logwin::log_ui(ui, state),
        // 启动流对话框（首启向导/缺模型下载/模型加载按 state 内部阶段再分派）
        WinId::Setup => setup::setup_ui(ui, state),
    }
}

