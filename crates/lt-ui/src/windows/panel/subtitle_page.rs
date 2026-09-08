//! 字幕页（对照原版 ui/subtitle_settings.py SubtitleSettingsWidget + LineEditDialog、
//! panel/tabs/subtitle_tab.py 薄壳）：
//! 基本（启用/句数/窗宽/行距/圆角）、背景（颜色/不透明度/图片）、自动隐藏
//! （超时/隐藏动画/时长/鼠标穿透）、文字行列表（摘要行 + 添加/编辑/删除/上移/
//! 下移 + 双击行编辑）与行编辑模态区（SubtitleLine 全部 15 字段）。
//!
//! 全部改动 → `settings.subtitle_mode` + 300ms 防抖 ApplySettings（字幕窗逐帧
//! 读取 cfg 渲染，应用后自动生效）；`enabled` 勾选另发 WinAction::ToggleSubtitle
//! 联动字幕窗显隐（原版 subtitle_settings_changed → app_shell 显隐路径）。
//!
//! 已知偏差：行级字体走系统字体选择器（D-17 内嵌思源置顶 + 注册表扫描，
//! 空串=跟随字幕主字体）；颜色为 "#rrggbb" 文本 + 色块预览（rfd 无颜色对话框）。

use super::{color_field, group_card, mark_settings_dirty, Palette};
use crate::state::{move_line_down, move_line_up, AppState, LineEditState, ANIM_VALUES};
use egui::{RichText, Ui};
use lt_proto::SubtitleLine;

// ── 纯逻辑（单测覆盖） ──

/// 字幕行列表摘要（对照 _refresh_lines_list：`✓/✗ | 类型(语言) | 字体 Npt |
/// 颜色 | 对齐 [| 轮廓] [| 入场动画]`，两空格管道分隔）
pub fn line_row_text(line: &SubtitleLine) -> String {
    let mark = if line.enabled { "\u{2713}" } else { "\u{2717}" };
    let base = if line.line_type == "original" {
        lt_i18n::t("subwin_original")
    } else {
        lt_i18n::t("subwin_translation")
    };
    let label = match line.lang.as_deref() {
        Some(lang) if line.line_type != "original" => format!("{base} ({lang})"),
        _ => base,
    };
    let align_key = match line.align.as_str() {
        "left" => "subwin_align_left",
        "right" => "subwin_align_right",
        _ => "subwin_align_center",
    };
    let mut parts = vec![
        mark.to_string(),
        label,
        // D-17：空串 = 跟随字幕主字体（摘要显示"跟随"而非族名）
        {
            let fam = if line.font_family.is_empty() {
                lt_i18n::t("label_font_follow")
            } else {
                line.font_family.clone()
            };
            format!("{fam} {}pt", line.font_size)
        },
        line.color.clone(),
        lt_i18n::t(align_key),
    ];
    if line.outline_enabled {
        parts.push(lt_i18n::t("subwin_outline"));
    }
    if line.entry_animation != "none" {
        parts.push(lt_i18n::t(&format!("subwin_anim_{}", line.entry_animation)));
    }
    parts.join("  |  ")
}

/// 行编辑语言下拉项（原版 LANGUAGES 跳过 auto：(码, "码 - 原生名")）
pub fn lang_options() -> Vec<(String, String)> {
    lt_i18n::LANGUAGES
        .iter()
        .filter(|(code, _)| *code != "auto")
        .map(|(code, native)| {
            let label = native.map_or_else(|| code.to_string(), |n| n.to_string());
            (code.to_string(), format!("{code} - {label}"))
        })
        .collect()
}

/// 语言码 → 下拉索引（未知/缺省回退 zh=2；原版 findData 失败停首项，
/// 此处取更贴近默认配置的 zh。表序 = lt_i18n::LANGUAGES 跳过 auto：ja/en/zh…）
pub fn lang_index_for(code: &str) -> usize {
    lang_options().iter().position(|(c, _)| c == code).unwrap_or(2)
}

/// 通用"下拉 + 索引"（选中项变化时返回新索引；样式页复用）
pub fn combo_index(ui: &mut Ui, id: &str, current: usize, labels: &[String], width: f32) -> Option<usize> {
    let mut next = current;
    egui::ComboBox::from_id_salt(id)
        .selected_text(labels[current].clone())
        .width(width)
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if ui.selectable_label(current == i, label.clone()).clicked() && current != i {
                    next = i;
                }
            }
        });
    (next != current).then_some(next)
}

// ── UI ──

/// 字幕页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // N3/N4：字幕页偏离默认提示 + 恢复本页（SubtitleMode 整体 + 文字行两行默认；
    // 窗口位置 window_x/y 不纳入——样式页「重置窗口位置」单独承担）。
    // 确认文案复用原版孤儿键 subwin_reset_confirm（zh/en 均已有）。
    let diffs = crate::panel_diff::diff_paths(&state.settings);
    let page_diffs: Vec<&str> = diffs
        .iter()
        .filter(|p| p.starts_with("subtitle_mode"))
        .map(|p: &String| p.as_str())
        .collect();
    if !page_diffs.is_empty() {
        super::reset_toolbar(ui, pal, page_diffs.len(), &page_diffs.join("、"), |_| {
            // D-33/H-5：确认改 egui 模态（原位 rfd 同步框阻塞事件循环线程）
            state.request_confirm(
                crate::state::ConfirmKind::ResetSubtitle,
                false,
                lt_i18n::t("reset_confirm_title"),
                lt_i18n::t("subwin_reset_confirm"),
            );
        });
    }

    // ── 基本（原版 subwin_basic 组）──
    group_card(ui, pal, &lt_i18n::t("subwin_basic"), |ui| {
        // 启用字幕窗（settings.subtitle_mode.enabled；联动字幕窗显隐）
        let mut enabled = state.settings.subtitle_mode.enabled;
        if ui
            .add(egui::Checkbox::new(&mut enabled, RichText::new(lt_i18n::t("subwin_show")).color(pal.text)))
            .changed()
        {
            state.settings.subtitle_mode.enabled = enabled;
            // 原版 subtitle_settings_changed → app_shell 按开关显隐字幕窗
            state.enqueue_action(crate::state::WinId::Panel, crate::state::WinAction::ToggleSubtitle);
            mark_settings_dirty(state);
        }
        // 显示句数 1..=10（原版 _sentences_spin）
        let mut v = state.settings.subtitle_mode.sentences.clamp(1, 10) as i32;
        if number_row(ui, "sub_sentences", &lt_i18n::t("subwin_sentences"), &mut v, 1..=10, "") {
            state.settings.subtitle_mode.sentences = v as u32;
            mark_settings_dirty(state);
        }
        // 窗口宽度 200..=3840 px（原版 _width_spin）
        let mut v = state.settings.subtitle_mode.window_width.clamp(200, 3840) as i32;
        if number_row(ui, "sub_width", &lt_i18n::t("subwin_window_width"), &mut v, 200..=3840, " px") {
            state.settings.subtitle_mode.window_width = v as u32;
            mark_settings_dirty(state);
        }
        // 行间距 0..=40 px
        let mut v = state.settings.subtitle_mode.line_spacing.min(40) as i32;
        if number_row(ui, "sub_spacing", &lt_i18n::t("subwin_line_spacing"), &mut v, 0..=40, " px") {
            state.settings.subtitle_mode.line_spacing = v as u32;
            mark_settings_dirty(state);
        }
        // 圆角 0..=30 px
        let mut v = state.settings.subtitle_mode.border_radius.min(30) as i32;
        if number_row(ui, "sub_radius", &lt_i18n::t("subwin_border_radius"), &mut v, 0..=30, " px") {
            state.settings.subtitle_mode.border_radius = v as u32;
            mark_settings_dirty(state);
        }
    });

    // ── 背景（原版背景三要素：颜色/不透明度/背景图片；有图片时颜色仍可调，
    //     渲染层按图片存在性决定底色，原版 setEnabled 联动不保留）──
    group_card(ui, pal, &lt_i18n::t("subwin_background"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("subwin_bg_color"))).color(pal.text));
            let mut color = state.settings.subtitle_mode.bg_color.clone();
            if color_field(ui, "sub_bg_color", &mut color) {
                state.settings.subtitle_mode.bg_color = color;
                mark_settings_dirty(state);
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("subwin_bg_opacity"))).color(pal.text));
            let mut pct = (f64::from(state.settings.subtitle_mode.bg_opacity) / 255.0 * 100.0).round() as i32;
            let resp = ui
                .add(egui::Slider::new(&mut pct, 0..=100).custom_formatter(|v, _| format!("{v:.0}%")))
                .on_hover_text(lt_i18n::t("subwin_bg_opacity_hint"));
            if resp.changed() {
                state.settings.subtitle_mode.bg_opacity = (f64::from(pct.clamp(0, 100)) / 100.0 * 255.0).round() as u32;
                mark_settings_dirty(state);
            }
        });
        bg_image_row(ui, state);
    });

    // ── 自动隐藏与穿透（原版 auto_hide_timeout/hide_animation/hide_duration +
    //     click_through）──
    group_card(ui, pal, &lt_i18n::t("subwin_animation"), |ui| {
        // 超时 0..=120 秒（0=禁用）
        let mut v = state.settings.subtitle_mode.auto_hide_timeout.min(120) as i32;
        if number_row(
            ui,
            "sub_auto_hide",
            &lt_i18n::t("subwin_auto_hide"),
            &mut v,
            0..=120,
            &format!(" {}", lt_i18n::t("subwin_auto_hide_sec")),
        ) {
            state.settings.subtitle_mode.auto_hide_timeout = v.max(0) as u32;
            mark_settings_dirty(state);
        }
        // 隐藏动画（none/fade/slide_down 三项）
        let hide_anims = ["none", "fade", "slide_down"];
        let labels: Vec<String> =
            hide_anims.iter().map(|a| lt_i18n::t(&format!("subwin_anim_{a}"))).collect();
        let idx = hide_anims
            .iter()
            .position(|a| *a == state.settings.subtitle_mode.auto_hide_animation)
            .unwrap_or(1);
        if let Some(next) = combo_index(ui, "sub_hide_anim", idx, &labels, 160.0) {
            state.settings.subtitle_mode.auto_hide_animation = hide_anims[next].to_string();
            mark_settings_dirty(state);
        }
        // 时长 50..=3000 ms
        let mut v = state.settings.subtitle_mode.auto_hide_duration.clamp(50, 3000) as i32;
        if number_row(ui, "sub_hide_dur", &lt_i18n::t("subwin_hide_duration"), &mut v, 50..=3000, " ms") {
            state.settings.subtitle_mode.auto_hide_duration = v as u32;
            mark_settings_dirty(state);
        }
        // 鼠标穿透（宿主 500ms 断言轮询按此开关续拍）
        let mut ct = state.settings.subtitle_mode.click_through;
        if ui
            .add(
                egui::Checkbox::new(&mut ct, RichText::new(lt_i18n::t("subwin_click_through")).color(pal.text)),
            )
            .on_hover_text(lt_i18n::t("subwin_click_through_hint"))
            .changed()
        {
            state.settings.subtitle_mode.click_through = ct;
            if ct && *state.visible.get(&crate::state::WinId::Subtitle).unwrap_or(&false) {
                state.schedule_subtitle_window_poll();
            }
            mark_settings_dirty(state);
        }
    });

    // ── 文字行（原版 lines_group：列表 + 五按钮 + 双击编辑）──
    group_card(ui, pal, &lt_i18n::t("subwin_text_lines"), |ui| {
        let count = state.settings.subtitle_mode.lines.len();
        let mut select: Option<usize> = None;
        let mut edit_row: Option<usize> = None;
        for i in 0..count {
            let text = line_row_text(&state.settings.subtitle_mode.lines[i]);
            let resp = ui
                .push_id(i, |ui| {
                    ui.add(
                        egui::Button::selectable(
                            state.panel.line_selected == Some(i),
                            RichText::new(&text).monospace().size(12.0),
                        )
                        .corner_radius(4.0)
                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                    )
                })
                .inner;
            if resp.clicked() {
                select = Some(i);
            }
            if resp.double_clicked() {
                edit_row = Some(i);
            }
        }
        if select.is_some() {
            state.panel.line_selected = select;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new(lt_i18n::t("btn_add")).size(12.5)).corner_radius(6.0))
                .clicked()
            {
                let idx = state.settings.subtitle_mode.lines.len();
                state.panel.line_editor = Some(LineEditState::new_add(idx));
            }
            let target = edit_row.or(state.panel.line_selected);
            if ui
                .add_enabled(
                    target.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_edit")).size(12.5)).corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = target {
                    if i < state.settings.subtitle_mode.lines.len() {
                        let line = state.settings.subtitle_mode.lines[i].clone();
                        state.panel.line_editor = Some(LineEditState::new_edit(i, &line));
                    }
                }
            }
            let can_remove = count > 1 && state.panel.line_selected.is_some();
            if ui
                .add_enabled(
                    can_remove,
                    egui::Button::new(RichText::new(lt_i18n::t("btn_remove")).size(12.5)).corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = state.panel.line_selected {
                    if i < count && count > 1 {
                        state.settings.subtitle_mode.lines.remove(i);
                        state.panel.line_selected = None;
                        mark_settings_dirty(state);
                    }
                }
            }
            // 上移/下移（原版 subwin_move_up/down；边界在 move_line_* 内判定）
            if ui
                .add_enabled(
                    state.panel.line_selected.is_some_and(|i| i > 0),
                    egui::Button::new(RichText::new(lt_i18n::t("subwin_move_up")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = state.panel.line_selected {
                    if move_line_up(&mut state.settings.subtitle_mode.lines, i) {
                        state.panel.line_selected = Some(i - 1);
                        mark_settings_dirty(state);
                    }
                }
            }
            if ui
                .add_enabled(
                    state.panel.line_selected.is_some_and(|i| i + 1 < count),
                    egui::Button::new(RichText::new(lt_i18n::t("subwin_move_down")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                if let Some(i) = state.panel.line_selected {
                    if move_line_down(&mut state.settings.subtitle_mode.lines, i) {
                        state.panel.line_selected = Some(i + 1);
                        mark_settings_dirty(state);
                    }
                }
            }
        });
    });

    ui.add_space(8.0);

    // ── LineEditDialog（egui::Window 居中模态区）──
    render_line_editor(ui, state);
}

/// 背景图片行（原版 _make_image_rows：路径 + 选择/清除）
fn bg_image_row(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} ", lt_i18n::t("subwin_bg_image"))).color(ui.visuals().text_color()),
        );
        let mut img = state.settings.subtitle_mode.bg_image.clone();
        if ui
            .add(egui::TextEdit::singleline(&mut img).hint_text("D:/bg.png").desired_width(260.0))
            .changed()
        {
            state.settings.subtitle_mode.bg_image = img.trim().to_string();
            mark_settings_dirty(state);
        }
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("subwin_bg_image_select")).size(12.0))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            if let Some(p) = rfd::FileDialog::new()
                .set_title(lt_i18n::t("subwin_bg_image_select"))
                .add_filter("Images", &["png", "webp", "jpg", "jpeg", "bmp"])
                .pick_file()
            {
                state.settings.subtitle_mode.bg_image = p.display().to_string();
                mark_settings_dirty(state);
            }
        }
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("subwin_bg_image_clear")).size(12.0))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            state.settings.subtitle_mode.bg_image.clear();
            mark_settings_dirty(state);
        }
    });
}

/// "标签 + DragValue(i32)" 行。返回是否变更（写回与防抖由调用方完成）。
fn number_row(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut i32,
    range: std::ops::RangeInclusive<i32>,
    suffix: &str,
) -> bool {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
            let mut v = *value;
            let resp = ui
                .add(
                    egui::DragValue::new(&mut v)
                        .range(*range.start()..=*range.end())
                        .suffix(suffix),
                )
                .changed();
            if resp {
                *value = v;
            }
            resp
        })
        .inner
    })
    .inner
}

/// LineEditDialog 模态区（打开中每帧渲染）
fn render_line_editor(ui: &mut Ui, state: &mut AppState) {
    if state.panel.line_editor.is_none() {
        return;
    }
    let mut open = true;
    let mut cancel = false;
    let mut accepted: Option<SubtitleLine> = None;
    let master = state.settings.subtitle_font_family.clone();
    {
        // panel.line_editor 与 fonts 是 AppState 不同字段，可并行借用
        let (panel, fonts) = (&mut state.panel, &mut state.fonts);
        let Some(ed) = panel.line_editor.as_mut() else { return };
        egui::Window::new(RichText::new(lt_i18n::t("subwin_edit_line")).strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(460.0)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(440.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| line_editor_fields(ui, ed, fonts, &master));
                ui.add_space(6.0);
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(lt_i18n::t("common_ok")).clicked() {
                        accepted = Some(ed.build());
                    }
                    if ui.button(lt_i18n::t("subwin_cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
    }
    if let Some(line) = accepted {
        let ed = state.panel.line_editor.take().expect("行编辑器打开中");
        let lines = &mut state.settings.subtitle_mode.lines;
        if ed.is_new {
            lines.push(line);
            state.panel.line_selected = Some(lines.len() - 1);
        } else if ed.index < lines.len() {
            lines[ed.index] = line;
            state.panel.line_selected = Some(ed.index);
        }
        // 行级字体键可能变更，字体链（命名族注册）同步重建
        crate::fonts::apply_fonts(ui.ctx(), &state.settings, &mut state.fonts);
        mark_settings_dirty(state);
    } else if !open || cancel {
        state.panel.line_editor = None;
    }
}

/// 行编辑字段全集（原版 LineEditDialog QGridLayout 逐行；SubtitleLine 15 字段）
fn line_editor_fields(
    ui: &mut Ui,
    ed: &mut LineEditState,
    fonts: &mut crate::fonts::FontsState,
    master: &str,
) {
    egui::Grid::new("line_edit_grid")
        .num_columns(2)
        .spacing([8.0, 5.0])
        .min_col_width(110.0)
        .show(ui, |ui| {
            // 启用
            ui.label(lt_i18n::t("subwin_enabled"));
            ui.checkbox(&mut ed.enabled, "");
            ui.end_row();

            // 类型（原文/翻译）
            ui.label(lt_i18n::t("subwin_line_type"));
            let types = [lt_i18n::t("subwin_original"), lt_i18n::t("subwin_translation")];
            if let Some(next) = combo_index(ui, "line_edit_type", ed.line_type_index, &types, 200.0) {
                ed.line_type_index = next;
            }
            ui.end_row();

            // 目标语言（仅翻译行可编辑，原版 _update_lang_visibility）
            ui.add_enabled(ed.line_type_index == 1, egui::Label::new(lt_i18n::t("subwin_target_lang")));
            let opts = lang_options();
            let idx = opts.iter().position(|(c, _)| *c == ed.lang).unwrap_or(2);
            let labels: Vec<String> = opts.iter().map(|(_, l)| l.clone()).collect();
            if ed.line_type_index == 1 {
                if let Some(next) = combo_index(ui, "line_edit_lang", idx, &labels, 200.0) {
                    ed.lang = opts[next].0.clone();
                }
            } else {
                // 原文行：语言不可编辑（灰显占位）
                ui.add_enabled(
                    false,
                    egui::Button::selectable(false, labels[idx].clone()).corner_radius(4.0),
                );
            }
            ui.end_row();

            // 字体（D-17：行级空串=跟随主设置；选择器含小样与缺字提示）
            ui.label(lt_i18n::t("subwin_font"));
            let cur_family = ed.font_family.clone();
            if let Some(next) = super::font_picker::font_picker_row(
                ui,
                fonts,
                "line_edit_font",
                "",
                cur_family,
                master,
                true,
            ) {
                ed.font_family = next;
            }
            ui.end_row();

            // 字号 8..=120 pt
            ui.label(lt_i18n::t("subwin_font_size"));
            ui.add(egui::DragValue::new(&mut ed.font_size).range(8..=120).suffix(" pt"));
            ui.end_row();

            // 颜色
            ui.label(lt_i18n::t("subwin_color"));
            color_field(ui, "line_edit_color", &mut ed.color);
            ui.end_row();

            // 不透明度 0-100%
            ui.label(lt_i18n::t("subwin_opacity"));
            ui.add(egui::Slider::new(&mut ed.opacity_pct, 0..=100).custom_formatter(|v, _| format!("{v:.0}%")));
            ui.end_row();

            // 对齐
            ui.label(lt_i18n::t("subwin_align"));
            let aligns = [
                lt_i18n::t("subwin_align_left"),
                lt_i18n::t("subwin_align_center"),
                lt_i18n::t("subwin_align_right"),
            ];
            if let Some(next) = combo_index(ui, "line_edit_align", ed.align_index, &aligns, 200.0) {
                ed.align_index = next;
            }
            ui.end_row();

            // 轮廓 + 颜色 + 宽度
            ui.label(lt_i18n::t("subwin_outline"));
            ui.checkbox(&mut ed.outline_enabled, "");
            ui.end_row();
            ui.label(lt_i18n::t("subwin_outline_color"));
            color_field(ui, "line_edit_outline_color", &mut ed.outline_color);
            ui.end_row();
            ui.label(lt_i18n::t("subwin_outline_width"));
            ui.add(egui::DragValue::new(&mut ed.outline_width).range(0..=10).suffix(" px"));
            ui.end_row();

            // 背景图片（文本 + 清除；图片文件选择按钮保留在页面级背景卡）
            ui.label(lt_i18n::t("subwin_bg_image"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.bg_image).desired_width(170.0));
                if ui.small_button(lt_i18n::t("subwin_bg_image_clear")).clicked() {
                    ed.bg_image.clear();
                }
            });
            ui.end_row();

            // 入场/退场动画
            let anim_labels: Vec<String> =
                ANIM_VALUES.iter().map(|v| lt_i18n::t(&format!("subwin_anim_{v}"))).collect();
            ui.label(lt_i18n::t("subwin_entry_anim"));
            if let Some(next) = combo_index(ui, "line_edit_entry", ed.entry_anim_index, &anim_labels, 200.0) {
                ed.entry_anim_index = next;
            }
            ui.end_row();
            ui.label(lt_i18n::t("subwin_exit_anim"));
            if let Some(next) = combo_index(ui, "line_edit_exit", ed.exit_anim_index, &anim_labels, 200.0) {
                ed.exit_anim_index = next;
            }
            ui.end_row();

            // 动画时长 50..=3000 ms
            ui.label(lt_i18n::t("subwin_anim_duration"));
            ui.add(egui::DragValue::new(&mut ed.animation_duration).range(50..=3000).suffix(" ms"));
            ui.end_row();
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{align_value, anim_index};

    /// 摘要行格式对照 _refresh_lines_list（分隔符/标记/语言后缀/轮廓/动画段）。
    /// i18n 全局语言在并行测试下不确定 → 断言键已解析（不含 subwin_ 原键）而非具体文案。
    #[test]
    fn line_row_text_matches_original_parts() {
        let mut line = SubtitleLine::default(); // translation en 24pt #FFFFFF
        let text = line_row_text(&line);
        assert!(text.starts_with("\u{2713}  |  "), "{text}");
        assert!(text.contains("(en)"), "翻译行带语言后缀: {text}");
        assert!(text.contains("跟随（默认） 24pt") || text.contains("Follow (default) 24pt"), "{text}");
        assert!(text.contains("#FFFFFF"), "{text}");
        assert!(!text.contains("subwin_"), "i18n 键应已解析: {text}");
        // 5 段：✓ | 类型(语言) | 字体 字号 | 颜色 | 对齐，+ 轮廓 = 6 段
        assert_eq!(text.split("  |  ").count(), 6, "{text}");

        // 原文行：无语言后缀、段数少一（无语言括号但类型标签仍占一段）
        line.line_type = "original".into();
        line.lang = None;
        let orig_text = line_row_text(&line);
        assert!(!orig_text.contains("(en)"), "{orig_text}");
        assert!(orig_text.contains("24pt"));
        // 禁用行 ✗ 标记；入场动画段追加
        line.enabled = false;
        line.entry_animation = "slide_up".into();
        let text2 = line_row_text(&line);
        assert!(text2.starts_with("\u{2717}"), "{text2}");
        assert!(!text2.contains("subwin_"), "{text2}");
        assert_eq!(
            text2.split("  |  ").count(),
            orig_text.split("  |  ").count() + 1,
            "入场动画段应追加"
        );
    }

    /// 语言下拉：30 项去掉 auto = 29 项；索引映射未知回退 zh
    #[test]
    fn lang_options_exclude_auto_and_index() {
        let opts = lang_options();
        assert_eq!(opts.len(), 29);
        assert!(opts.iter().all(|(c, _)| c != "auto"));
        assert_eq!(opts[0].0, "ja", "原版 LANGUAGES 顺序跳过 auto");
        assert_eq!(opts[2].0, "zh");
        assert_eq!(lang_index_for("ja"), 0);
        assert_eq!(lang_index_for("en"), 1);
        assert_eq!(lang_index_for("zh"), 2, "ja/en/zh 顺序 → zh=2");
        assert_eq!(lang_index_for("bogus"), 2, "未知回退 zh");
        assert_eq!(lang_index_for(""), 2);
        // 行编辑默认（new_add lang=en）命中索引 1
        let add = LineEditState::new_add(0);
        assert_eq!(lang_index_for(&add.lang), 1);
    }

    /// 行编辑索引 → 契约字符串（未知值回退语义）
    #[test]
    fn line_editor_index_to_contract_values() {
        let mut ed = LineEditState::new_add(0);
        ed.align_index = crate::state::align_index("right");
        assert_eq!(ed.align_index, 2);
        ed.entry_anim_index = anim_index("fade");
        ed.exit_anim_index = anim_index("nope");
        let line = ed.build();
        assert_eq!(line.align, "right");
        assert_eq!(line.entry_animation, "fade");
        assert_eq!(line.exit_animation, "none", "未知动画回退 none");
        assert_eq!(align_value(1), "center");
        // 默认对齐 center
        assert_eq!(align_value(9), "center");
    }

    /// 行编辑模态区无头渲染冒烟：LineEditDialog 打开态整页面跑两帧不 panic
    #[test]
    fn line_editor_modal_smoke_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(lt_proto::Settings::default());
        st.panel.page = crate::state::PanelPage::Subtitle;
        st.panel.line_editor =
            Some(LineEditState::new_edit(0, &st.settings.subtitle_mode.lines[0]));
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::windows::panel::panel_ui(ui, &mut st));
            assert!(!out.shapes.is_empty(), "行编辑器打开态应产出图元");
            out.textures_delta.clear();
        }
        assert!(st.panel.line_editor.is_some(), "编辑器保持打开（状态未被意外消费）");
    }
}
