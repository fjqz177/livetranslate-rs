//! 日志窗全量复刻（对照原版 log_window.py）：
//! 深色只读文本区（Consolas 9pt，#1e1e1e/#d4d4d4）+ 底部控件行
//! （自动滚动 / 显示 DEBUG / 清空）；级别配色 + ASR/Translate/Segment
//! 内容高亮三色；环形 2000 行。
//! D-31（docs/archive/log-tab-redesign.md）：级别过滤移到渲染期（勾选 show_debug 即回溯
//! 历史，原版为追加期过滤）+ 自动滚动改贴底跟随（上翻挂起 +「回到最新」浮钮）。

use crate::state::{AppState, LogView, LogWindowState};
use crate::windows::log_jump_button;
use egui::{Color32, RichText, ScrollArea, Ui};

/// 原版配色
const BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x1e);
const DEBUG_GRAY: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
const INFO: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);
const WARN: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xaa);
const ERROR: Color32 = Color32::from_rgb(0xf4, 0x47, 0x47);
/// 内容高亮三色（ASR 行青绿 / Translate 行蓝 / Speech segment 行橙棕）
const HL_ASR: Color32 = Color32::from_rgb(0x4e, 0xc9, 0xb0);
const HL_TL: Color32 = Color32::from_rgb(0x9c, 0xdc, 0xfe);
const HL_SEG: Color32 = Color32::from_rgb(0xce, 0x91, 0x78);

/// 行着色（对照原版 _append_log：先按级别取色，再按内容覆盖高亮。
/// 注：原版 "Translate:" 匹配串与实际日志行 "Translate (x ms):" 不符，
/// 该规则在原版即为死分支——1:1 保留原文案）
fn line_color(level: u8) -> Color32 {
    match level {
        10 => DEBUG_GRAY,
        30 => WARN,
        40..=50 => ERROR,
        _ => INFO,
    }
    // 内容高亮在渲染层追加（见下 highlight()）
}

fn highlight(text: &str, base: Color32) -> Color32 {
    if text.contains("ASR [") {
        HL_ASR
    } else if text.contains("Translate:") {
        HL_TL
    } else if text.contains("Speech segment") {
        HL_SEG
    } else {
        base
    }
}

pub fn log_ui(ui: &mut Ui, state: &mut AppState) {
    let show_debug = state.logwin.show_debug;
    let auto_scroll = state.logwin.auto_scroll;
    // 文本区（深色背景）
    egui::Frame::NONE.fill(BG).inner_margin(4.0).show(ui, |ui| {
        let max = ui.available_height() - 28.0;
        let out = ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(max)
            // D-31 贴底跟随（同面板日志 tab）
            .stick_to_bottom(auto_scroll)
            .show(ui, |ui| {
                let lines = state.logwin.formatted();
                for (text, level) in lines {
                    if !LogWindowState::level_visible(*level, show_debug) {
                        continue;
                    }
                    let color = highlight(text, line_color(*level));
                    ui.label(RichText::new(text).monospace().color(color).size(12.0));
                }
            });
        // 用户主动翻离底部且出现未读行：右下角「回到最新（+N）」浮钮
        if state.logwin.advance_follow(
            out.state.offset.y,
            out.inner_rect.height(),
            out.content_size.y,
            LogView::LogWin,
        ) && log_jump_button(ui, state.logwin.new_since_bottom())
        {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            state.logwin.mark_at_bottom();
        }
    });

    // 控件行（原版布局：文本区在下？——原版先 text 后 controls，此处同序）
    ui.horizontal(|ui| {
        ui.add(egui::Checkbox::new(
            &mut state.logwin.auto_scroll,
            RichText::new(lt_i18n::t("auto_scroll")).size(12.0),
        ));
        ui.add(egui::Checkbox::new(
            &mut state.logwin.show_debug,
            RichText::new(lt_i18n::t("show_debug")).size(12.0),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(lt_i18n::t("clear")).clicked() {
                state.logwin.clear();
            }
        });
    });
}
