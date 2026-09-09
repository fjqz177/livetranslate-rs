//! 样式页（对照原版 panel/tabs/style_tab.py）：
//! 14+1 预设下拉（lt_ui::style::PRESET_NAMES + custom）+ 恢复默认 +
//! 当前配色预览小卡 + 背景/文字/窗口三组微调字段。
//!
//! 联动语义（原版 _on_preset_changed / _on_style_value_changed）：
//! - 选预设（非 custom）→ 整组字段覆写为 preset_style(key) + 防抖落盘；
//! - 手改任一字段 → preset 置 "custom"（原版 blockSignals 静默改下拉）+ 落盘；
//! - 恢复默认 → preset_style("default") 整组覆写（原版 _reset_style）。

use super::{color_field, group_card, mark_settings_dirty, Palette};
use crate::state::{SessionView, Settings, UiContext};
use crate::style::{preset_style, PRESET_NAMES};
use egui::{Color32, RichText, Ui};
use lt_proto::Style;

/// 预设下拉标签（PRESET_NAMES + "custom"；i18n 键 preset_{name}）
pub fn preset_labels() -> Vec<String> {
    PRESET_NAMES
        .iter()
        .map(|n| lt_i18n::t(&format!("preset_{n}")))
        .chain(std::iter::once(lt_i18n::t("preset_custom")))
        .collect()
}

/// settings.style.preset → 下拉索引（未知值回退 custom=末位，原版同语义）
pub fn preset_index_for(preset: &str) -> usize {
    PRESET_NAMES
        .iter()
        .position(|n| *n == preset)
        .unwrap_or(PRESET_NAMES.len())
}

/// 应用预设（原版 _apply_style_to_controls(preset) 的整组覆写）
pub fn apply_preset(style: &mut Style, key: &str) {
    *style = preset_style(key);
}

/// 手改字段后的 custom 联动（原版 _on_style_value_changed：下拉静默切到 custom）
pub fn mark_custom(style: &mut Style) {
    style.preset = "custom".to_string();
}

/// 恢复默认样式（原版 _reset_style：下拉回 index 0 + DEFAULT_STYLE 覆写）
pub fn reset_style(style: &mut Style) {
    *style = preset_style("default");
}

// ── UI ──

/// 样式页 UI 总入口
pub fn page(ui: &mut Ui, session: &mut SessionView, settings: &mut Settings, ctx: &mut UiContext, pal: &Palette) {
    // N3/N4：样式页偏离默认提示 + 恢复本页（Style 14 键 + 两把主字体键；
    // 窗口几何（overlay_x 等）不纳入——样式页已有「重置窗口位置」按钮）
    let diffs = crate::panel_diff::diff_paths(settings);
    let page_diffs: Vec<&str> = diffs
        .iter()
        .filter(|p| {
            p.starts_with("style.")
                || p.as_str() == "ui_font_family"
                || p.as_str() == "subtitle_font_family"
        })
        .map(|p: &String| p.as_str())
        .collect();
    if !page_diffs.is_empty() {
        super::reset_toolbar(ui, pal, page_diffs.len(), &page_diffs.join("、"), |ui| {
            let def = lt_proto::Settings::default();
            settings.style = def.style.clone();
            settings.ui_font_family = def.ui_font_family.clone();
            settings.subtitle_font_family = def.subtitle_font_family.clone();
            // 字体键变化必须重装字体链（D-17：行级"跟随"解析自注册表）
            crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
            mark_settings_dirty(session);
        });
    }

    // ── 字体（D-17：界面/字幕两把主旋钮；其余行级在下方「文字」组）──
    group_card(ui, pal, &lt_i18n::t("group_fonts"), |ui| {
        let cur_ui = settings.ui_font_family.clone();
        if let Some(next) = super::font_picker::font_picker_row(
            ui,
            &mut ctx.fonts,
            "font_ui",
            &lt_i18n::t("label_ui_font"),
            cur_ui,
            "",
            false,
        ) {
            settings.ui_font_family = next;
            crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
            mark_settings_dirty(session);
        }
        let cur_sub = settings.subtitle_font_family.clone();
        if let Some(next) = super::font_picker::font_picker_row(
            ui,
            &mut ctx.fonts,
            "font_sub",
            &lt_i18n::t("label_subtitle_font"),
            cur_sub,
            "",
            false,
        ) {
            settings.subtitle_font_family = next;
            crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
            mark_settings_dirty(session);
        }
        ui.label(
            RichText::new(lt_i18n::t("hint_subtitle_font_scope"))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
        super::font_picker::font_group_preview(ui, settings, ctx);
    });

    // ── 预设（原版 preset_group）──
    group_card(ui, pal, &lt_i18n::t("group_preset"), |ui| {
        let labels = preset_labels();
        let cur = preset_index_for(&settings.style.preset);
        ui.horizontal(|ui| {
            let next = super::subtitle_page::combo_index(ui, "style_preset", cur, &labels, 260.0);
            if let Some(next) = next {
                if next < PRESET_NAMES.len() {
                    apply_preset(&mut settings.style, PRESET_NAMES[next]);
                    // 预设可能改写行级字体键（D-17 默认=跟随），字体链需同步
                    crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
                    mark_settings_dirty(session);
                }
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_reset_style")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                reset_style(&mut settings.style);
                crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
                mark_settings_dirty(session);
            }
            // 原版样式 Tab 同行右侧"重置窗口位置"（_on_reset_positions → reset_positions 信号）
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_reset_positions")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                session.enqueue_action(
                    crate::state::WinId::Panel,
                    crate::state::WinAction::ResetPositions,
                );
            }
        });
        ui.add_space(4.0);
        style_preview(ui, settings, pal);
    });

    // ── 背景（原版 bg_group：颜色/不透明度/头部颜色/头部不透明度/圆角）──
    group_card(ui, pal, &lt_i18n::t("group_background"), |ui| {
        style_color_row(
            ui,
            settings,
            session,
            "style_bg_color",
            &lt_i18n::t("label_bg_color"),
            |s| &mut s.bg_color,
        );
        style_pct_row(
            ui,
            settings,
            session,
            "style_bg_op",
            &lt_i18n::t("label_bg_opacity"),
            |s| &mut s.bg_opacity,
        );
        style_color_row(
            ui,
            settings,
            session,
            "style_header_color",
            &lt_i18n::t("label_header_color"),
            |s| &mut s.header_color,
        );
        style_pct_row(
            ui,
            settings,
            session,
            "style_header_op",
            &lt_i18n::t("label_header_opacity"),
            |s| &mut s.header_opacity,
        );
        style_u32_row(
            ui,
            settings,
            session,
            "style_radius",
            &lt_i18n::t("label_border_radius"),
            |s| &mut s.border_radius,
            0..=30,
            " px",
        );
    });

    // ── 文字（原版 text_group：两族字体 + 字号 + 三色）──
    group_card(ui, pal, &lt_i18n::t("group_text"), |ui| {
        font_row(
            ui,
            settings,
            session,
            ctx,
            "style_orig_font",
            &lt_i18n::t("label_original_font"),
            |s| &mut s.original_font_family,
        );
        style_u32_row(
            ui,
            settings,
            session,
            "style_orig_size",
            &lt_i18n::t("label_original_font_size"),
            |s| &mut s.original_font_size,
            6..=24,
            " pt",
        );
        style_color_row(
            ui,
            settings,
            session,
            "style_orig_color",
            &lt_i18n::t("label_original_color"),
            |s| &mut s.original_color,
        );
        font_row(
            ui,
            settings,
            session,
            ctx,
            "style_trans_font",
            &lt_i18n::t("label_translation_font"),
            |s| &mut s.translation_font_family,
        );
        style_u32_row(
            ui,
            settings,
            session,
            "style_trans_size",
            &lt_i18n::t("label_translation_font_size"),
            |s| &mut s.translation_font_size,
            6..=24,
            " pt",
        );
        style_color_row(
            ui,
            settings,
            session,
            "style_trans_color",
            &lt_i18n::t("label_translation_color"),
            |s| &mut s.translation_color,
        );
        style_color_row(
            ui,
            settings,
            session,
            "style_ts_color",
            &lt_i18n::t("label_timestamp_color"),
            |s| &mut s.timestamp_color,
        );
    });

    // ── 窗口（原版 win_group：窗口透明度 30-100%）──
    group_card(ui, pal, &lt_i18n::t("group_window"), |ui| {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} ", lt_i18n::t("label_window_opacity"))).color(pal.text),
            );
            let mut pct = settings.style.window_opacity.clamp(30, 100) as i32;
            let resp = ui.add(
                egui::Slider::new(&mut pct, 30..=100).custom_formatter(|v, _| format!("{v:.0}%")),
            );
            if resp.changed() {
                settings.style.window_opacity = pct.clamp(30, 100) as u32;
                mark_custom(&mut settings.style);
                mark_settings_dirty(session);
            }
        });
    });

    ui.add_space(8.0);
}

/// 预览小卡（当前配色：头部行 + 原文/译文 + 时间戳，色值取 settings.style）
fn style_preview(ui: &mut Ui, settings: &Settings, pal: &Palette) {
    let st = &settings.style;
    let bg = crate::style::parse_color(&st.bg_color, Color32::BLACK)
        .gamma_multiply(st.bg_opacity.clamp(0, 255) as f32 / 255.0);
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(st.border_radius.clamp(0, 30) as u8))
        .stroke(egui::Stroke::new(1.0, pal.card_stroke))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // 头部色与悬浮窗一致不渲染（原版 QSS 对 QWidget 子类不生效，D-17）
            ui.label(
                RichText::new(format!("{} / こんにちは", lt_i18n::t("subwin_original")))
                    .size(st.original_font_size.clamp(6, 24) as f32)
                    .color(crate::style::parse_color(
                        &st.original_color,
                        Color32::LIGHT_GRAY,
                    )),
            );
            ui.label(
                RichText::new(lt_i18n::t("subwin_translation"))
                    .size(st.translation_font_size.clamp(6, 24) as f32)
                    .color(crate::style::parse_color(
                        &st.translation_color,
                        Color32::WHITE,
                    )),
            );
        });
}

/// "标签 + 颜色字段"行（改动 → custom 联动 + 防抖）
fn style_color_row(
    ui: &mut Ui,
    settings: &mut Settings,
    session: &mut SessionView,
    id: &str,
    label: &str,
    field: impl Fn(&mut Style) -> &mut String,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
        let mut color = field(&mut settings.style).clone();
        if color_field(ui, id, &mut color) {
            *field(&mut settings.style) = color;
            mark_custom(&mut settings.style);
            mark_settings_dirty(session);
        }
    });
}

/// "标签 + 百分比滑条"行（存储 0-255 ↔ 显示 %；原版 QSpinBox % 表述）
fn style_pct_row(
    ui: &mut Ui,
    settings: &mut Settings,
    session: &mut SessionView,
    id: &str,
    label: &str,
    field: impl Fn(&mut Style) -> &mut u32,
) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
            let mut pct =
                (f64::from(*field(&mut settings.style)) / 255.0 * 100.0).round() as i32;
            let resp = ui
                .add(
                    egui::Slider::new(&mut pct, 0..=100)
                        .custom_formatter(|v, _| format!("{v:.0}%")),
                )
                .changed();
            if resp {
                *field(&mut settings.style) =
                    (f64::from(pct.clamp(0, 100)) / 100.0 * 255.0).round() as u32;
                mark_custom(&mut settings.style);
                mark_settings_dirty(session);
            }
        });
    });
}

/// "标签 + 整数 DragValue"行（px/pt 后缀）
/// 8 参数 = 行形态配置（label/field/range/suffix 四元配置面，合并为结构
/// 反而遮蔽语义）
#[allow(clippy::too_many_arguments)]
fn style_u32_row(
    ui: &mut Ui,
    settings: &mut Settings,
    session: &mut SessionView,
    id: &str,
    label: &str,
    field: impl Fn(&mut Style) -> &mut u32,
    range: std::ops::RangeInclusive<u32>,
    suffix: &str,
) {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
            let (lo, hi) = (*range.start(), *range.end());
            let mut v = (*field(&mut settings.style)).clamp(lo, hi) as i32;
            let resp = ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(lo as i32..=hi as i32)
                        .suffix(suffix),
                )
                .changed();
            if resp {
                *field(&mut settings.style) = v.clamp(lo as i32, hi as i32) as u32;
                mark_custom(&mut settings.style);
                mark_settings_dirty(session);
            }
        });
    });
}

/// "标签 + 字体选择器"行（D-17：行级空串=跟随；选中即应用字体链并防抖落盘）
fn font_row(
    ui: &mut Ui,
    settings: &mut Settings,
    session: &mut SessionView,
    ctx: &mut UiContext,
    id: &str,
    label: &str,
    field: impl Fn(&mut Style) -> &mut String,
) {
    ui.push_id(id, |ui| {
        let cur = field(&mut settings.style).clone();
        let master = settings.subtitle_font_family.clone();
        if let Some(next) =
            super::font_picker::font_picker_row(ui, &mut ctx.fonts, id, label, cur, &master, true)
        {
            *field(&mut settings.style) = next;
            mark_custom(&mut settings.style);
            crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
            mark_settings_dirty(session);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 14+1 预设下拉与索引映射（未知值回退 custom）
    #[test]
    fn preset_labels_and_index_mapping() {
        assert_eq!(PRESET_NAMES.len(), 14);
        assert_eq!(preset_labels().len(), 15);
        assert_eq!(preset_index_for("default"), 0);
        assert_eq!(preset_index_for("kanagawa"), 13);
        assert_eq!(preset_index_for("custom"), 14, "custom = 末位");
        assert_eq!(preset_index_for("nope"), 14, "未知值回退 custom");
        // t() 键真实存在（label 不回退键名本身）
        for label in preset_labels() {
            assert!(!label.is_empty());
        }
        assert_ne!(
            preset_labels()[0],
            "preset_default",
            "preset_default 键应存在"
        );
    }

    /// 预设套用 → style 字段断言（transparent 只改透明度三键，其余保持 BASE）
    #[test]
    fn apply_preset_overrides_fields() {
        let mut style = preset_style("default");
        apply_preset(&mut style, "transparent");
        assert_eq!(style.preset, "transparent");
        assert_eq!(
            (style.bg_opacity, style.header_opacity, style.window_opacity),
            (120, 120, 70)
        );
        assert_eq!(style.bg_color, "#000000", "非覆写字段保持 BASE");
        assert_eq!(style.original_font_size, 11);

        apply_preset(&mut style, "light");
        assert_eq!(style.bg_color, "#e8e8f0");
        assert_eq!(style.translation_color, "#111111");
        assert_eq!(style.window_opacity, 95);
    }

    /// custom 联动：手改字段只置 preset 标记，不触碰其他字段
    #[test]
    fn manual_edit_marks_custom_only() {
        let mut style = preset_style("nord");
        let before = style.clone();
        mark_custom(&mut style);
        assert_eq!(style.preset, "custom");
        let mut expected = before;
        expected.preset = "custom".into();
        assert_eq!(style, expected);
    }

    /// 恢复默认 = 整组覆写 DEFAULT_STYLE（原版 _reset_style）
    #[test]
    fn reset_restores_default_style() {
        let mut style = preset_style("dracula");
        reset_style(&mut style);
        assert_eq!(style, Style::default());
        assert_eq!(style.preset, "default");
        assert_eq!(style.bg_opacity, 240);
        assert_eq!(style.window_opacity, 95);
    }
}
