//! 窗口 UI 分发（egui 0.36：容器一律吃 `&mut Ui`）。
//!
//! - overlay：全量复刻（见 [`overlay`] 模块文档）
//! - setup：启动流对话框（向导/缺模型下载/模型加载）
//! - subtitle/panel/log：M4 后续波次的占位

use crate::state::{AppState, WinId};
use egui::Ui;

pub mod overlay;
pub mod setup;

/// 按窗口分发（根 Ui 由宿主经 ctx.run_ui 提供）
pub fn dispatch(win: WinId, ui: &mut Ui, state: &mut AppState) {
    match win {
        WinId::Overlay => overlay::overlay_ui(ui, state),
        WinId::Subtitle => subtitle_ui(ui, state),
        WinId::Panel => panel_ui(ui, state),
        WinId::Log => log_ui(ui, state),
        // 启动流对话框（首启向导/缺模型下载/模型加载按 state 内部阶段再分派）
        WinId::Setup => setup::setup_ui(ui, state),
    }
}

/// 字幕窗占位：透明 + 底部居中大字（描边两遍绘制法的最小验证；M4.2 全量）。
fn subtitle_ui(ui: &mut Ui, _state: &mut AppState) {
    use egui::{Align2, Color32, FontId};
    let text = "字幕窗口占位 Subtitle";
    let font = FontId::proportional(28.0);
    let rect = ui.available_rect_before_wrap();
    let bottom_center = egui::pos2(rect.center().x, rect.bottom() - 40.0);
    for (dx, dy) in [
        (-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0),
        (-2.0, -2.0), (2.0, -2.0), (-2.0, 2.0), (2.0, 2.0),
    ] {
        ui.painter().text(
            bottom_center + egui::vec2(dx, dy),
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            Color32::BLACK,
        );
    }
    ui.painter().text(bottom_center, Align2::CENTER_CENTER, text, font, Color32::from_rgb(255, 215, 0));
    ui.allocate_space(ui.available_size());
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

/// 日志窗占位（M4.5：2000 行环形缓冲 + 级别过滤 + 内容高亮）
fn log_ui(ui: &mut Ui, _state: &mut AppState) {
    ui.heading("Log");
    ui.label("(M4：2000 行环形缓冲 + 级别过滤 + 内容高亮)");
}
