//! 窗口 UI 分发（egui 0.36：容器一律吃 `&mut Ui`）。
//!
//! - overlay：全量复刻（见 [`overlay`] 模块文档）
//! - subtitle：全量复刻（见 [`subtitle`] 模块文档）
//! - setup：启动流对话框（向导/缺模型下载/模型加载）
//! - panel/log：M4 后续波次的占位（log 已全量，见 [`logwin`]）

use crate::state::{AppState, WinId};
use egui::Ui;

pub mod logwin;
pub mod overlay;
pub mod setup;
pub mod subtitle;

/// 按窗口分发（根 Ui 由宿主经 ctx.run_ui 提供）
pub fn dispatch(win: WinId, ui: &mut Ui, state: &mut AppState) {
    match win {
        WinId::Overlay => overlay::overlay_ui(ui, state),
        WinId::Subtitle => subtitle::subtitle_ui(ui, state),
        WinId::Panel => panel_ui(ui, state),
        WinId::Log => logwin::log_ui(ui, state),
        // 启动流对话框（首启向导/缺模型下载/模型加载按 state 内部阶段再分派）
        WinId::Setup => setup::setup_ui(ui, state),
    }
}

/// 控制面板占位：7 个 tab 的标题壳（M4.3 全量）。
fn panel_ui(ui: &mut Ui, _state: &mut AppState) {
    egui::Panel::top("panel_tabs").show(ui, |ui| {
        ui.add_space(4.0);
        ui.heading(lt_i18n::t("window_control_panel"));
    });
    let tabs = [
        lt_i18n::t("tab_vad_asr"),
        lt_i18n::t("tab_translation"),
        lt_i18n::t("tab_style"),
        lt_i18n::t("tab_subtitle"),
        lt_i18n::t("tab_benchmark"),
        lt_i18n::t("tab_cache"),
        "Changelog".to_string(),
    ];
    egui::Grid::new("m0_tabs").num_columns(2).show(ui, |ui| {
        for (i, name) in tabs.iter().enumerate() {
            ui.label(format!("{}. {name}", i + 1));
            ui.label("· 待 M4");
            ui.end_row();
        }
    });
}

