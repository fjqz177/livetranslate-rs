//! M0 阶段的窗口占位 UI（egui 0.36 API：容器一律吃 `&mut Ui`）。
//! M4 将替换为 1:1 实现（悬浮窗消息流 / 字幕窗描边大字 / 面板 7 tab / 日志窗）。

use crate::state::{AppState, WinId};
use egui::{Align2, Color32, Layout, RichText, Ui};

/// 悬浮窗占位：透明背景 + 圆角深色容器 + DragHandle 雏形 + 消息区示意。
/// 已验证关键能力：透明、无边框、置顶、跳过任务栏、CJK 字体渲染。
pub fn overlay_ui(ui: &mut Ui, state: &mut AppState) {
    let outer = ui.available_rect_before_wrap();
    // 容器：rgba(15,15,25,200) 圆角 8（原版初始容器样式）
    egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(15, 15, 25, 200))
        .corner_radius(8.0)
        .inner_margin(8.0)
        .outer_margin(8.0)
        .show(ui, |ui| {
            // ── DragHandle 第一行雏形 ──
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("☰ LiveTranslate")
                        .monospace()
                        .color(Color32::from_rgb(170, 170, 170))
                        .strong(),
                );
            });
            ui.add_space(6.0);
            ui.set_min_height(120.0);
            ui.label(
                RichText::new("waiting for audio…")
                    .color(Color32::from_rgb(153, 153, 153))
                    .italics(),
            );
            ui.label(
                RichText::new(format!(
                    "style={:?} lang={}",
                    state.settings.style.preset,
                    lt_i18n::get_lang()
                ))
                .color(Color32::from_rgb(102, 102, 102))
                .monospace(),
            );
            ui.allocate_rect(outer, egui::Sense::hover());
        });
}

/// 字幕窗占位：透明 + 底部居中大字（描边两遍绘制法的最小验证）。
pub fn subtitle_ui(ui: &mut Ui, _state: &mut AppState) {
    let text = "字幕窗口占位 Subtitle";
    let font = egui::FontId::proportional(28.0);
    // 布局：先把光标推到底部中央（占满可用区，取底部 60px 行）
    let avail = ui.available_size();
    let rect = ui.available_rect_before_wrap();
    let bottom_center = egui::pos2(rect.center().x, rect.bottom() - 40.0);
    // 描边：8 方向偏移黑字（M4 抽公共函数并缓存纹理）
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
    // 中心金字
    ui.painter().text(bottom_center, Align2::CENTER_CENTER, text, font, Color32::from_rgb(255, 215, 0));
    ui.allocate_space(avail);
}

/// 控制面板占位：7 个 tab 的标题壳。
pub fn panel_ui(ui: &mut Ui, _state: &mut AppState) {
    egui::Panel::top("panel_tabs").show(ui, |ui| {
        ui.add_space(4.0);
        ui.heading(lt_i18n::t("window_control_panel"));
    });
    let tabs = [
        ("tab_vad_asr", lt_i18n::t("tab_vad_asr")),
        ("tab_translation", lt_i18n::t("tab_translation")),
        ("tab_style", lt_i18n::t("tab_style")),
        ("tab_subtitle", lt_i18n::t("tab_subtitle")),
        ("tab_benchmark", lt_i18n::t("tab_benchmark")),
        ("tab_cache", lt_i18n::t("tab_cache")),
        ("tab_changelog", "Changelog".to_string()),
    ];
    egui::Grid::new("m0_tabs").num_columns(2).show(ui, |ui| {
        for (i, (_id, name)) in tabs.iter().enumerate() {
            ui.label(format!("{}. {name}", i + 1));
            ui.label("· 待 M4");
            ui.end_row();
        }
    });
}

/// 日志窗占位
pub fn log_ui(ui: &mut Ui, _state: &mut AppState) {
    ui.heading("Log");
    ui.label("(M4：2000 行环形缓冲 + 级别过滤 + 内容高亮)");
}

/// 按窗口分发（根 Ui 由宿主经 ctx.run_ui 提供）
pub fn dispatch(win: WinId, ui: &mut Ui, state: &mut AppState) {
    match win {
        WinId::Overlay => overlay_ui(ui, state),
        WinId::Subtitle => subtitle_ui(ui, state),
        WinId::Panel => panel_ui(ui, state),
        WinId::Log => log_ui(ui, state),
    }
}

// Layout 用于后续 M4（保留导入语义）
#[allow(unused)]
fn _layout_guard(_l: Layout) {}
