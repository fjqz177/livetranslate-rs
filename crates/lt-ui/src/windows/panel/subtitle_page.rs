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

use super::{color_field, group_card, mark_settings_changed, mark_settings_dirty, Palette};
use crate::state::{
    move_line_down, move_line_up, LineEditState, ModalUi, PanelUi, SessionView, Settings,
    UiContext, ANIM_VALUES,
};
use crate::style::selectable_stable;
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
    lang_options()
        .iter()
        .position(|(c, _)| c == code)
        .unwrap_or(2)
}

/// 通用"下拉 + 索引"（选中项变化时返回新索引；样式页复用）
pub fn combo_index(
    ui: &mut Ui,
    id: &str,
    current: usize,
    labels: &[String],
    width: f32,
) -> Option<usize> {
    let mut next = current;
    egui::ComboBox::from_id_salt(id)
        .selected_text(labels[current].clone())
        .width(width)
        .show_ui(ui, |ui| {
            for (i, label) in labels.iter().enumerate() {
                if selectable_stable(ui, current == i, label.clone()).clicked() && current != i {
                    next = i;
                }
            }
        });
    (next != current).then_some(next)
}

// ── UI ──

/// 字幕页 UI 总入口
pub fn page(
    ui: &mut Ui,
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    modal: &mut ModalUi,
    ctx: &mut UiContext,
    pal: &Palette,
) {
    // N3/N4：字幕页偏离默认提示 + 恢复本页（SubtitleMode 整体 + 文字行两行默认；
    // 窗口位置 window_x/y 不纳入——样式页「重置窗口位置」单独承担）。
    // 确认文案复用原版孤儿键 subwin_reset_confirm（zh/en 均已有）。
    let diffs = crate::panel_diff::diff_paths(settings);
    let page_diffs: Vec<&str> = diffs
        .iter()
        .filter(|p| p.starts_with("subtitle_mode"))
        .map(|p: &String| p.as_str())
        .collect();
    if !page_diffs.is_empty() {
        super::reset_toolbar(ui, pal, page_diffs.len(), &page_diffs.join("、"), |_| {
            // D-87：确认走专用窗（D-33 旧借画布模态已删）
            modal.request_confirm(
                crate::state::ConfirmKind::ResetSubtitle,
                lt_i18n::t("reset_confirm_title"),
                lt_i18n::t("subwin_reset_confirm"),
            );
            session.enqueue_action(
                crate::state::WinId::Confirm,
                crate::state::WinAction::ShowConfirm,
            );
        });
    }

    // ── 基本（原版 subwin_basic 组）──
    group_card(ui, pal, &lt_i18n::t("subwin_basic"), |ui| {
        // 启用字幕窗（settings.subtitle_mode.enabled；联动字幕窗显隐）
        let mut enabled = settings.subtitle_mode.enabled;
        if ui
            .add(egui::Checkbox::new(
                &mut enabled,
                RichText::new(lt_i18n::t("subwin_show")).color(pal.text),
            ))
            .changed()
        {
            settings.subtitle_mode.enabled = enabled;
            // 原版 subtitle_settings_changed → app_shell 按开关显隐字幕窗
            session.enqueue_action(
                crate::state::WinId::Panel,
                crate::state::WinAction::ToggleSubtitle,
            );
            // D-123 C4：非列表控件变更走 funnel——清三处列表选中 + 防抖落盘
            //（本页各控件写回点同此，列表自身操作除外）
            mark_settings_changed(panel, session);
        }
        // 显示句数 1..=10（原版 _sentences_spin）
        let mut v = settings.subtitle_mode.sentences.clamp(1, 10) as i32;
        if number_row(
            ui,
            "sub_sentences",
            &lt_i18n::t("subwin_sentences"),
            &mut v,
            1..=10,
            "",
        ) {
            settings.subtitle_mode.sentences = v as u32;
            mark_settings_changed(panel, session);
        }
        // 窗口宽度 200..=3840 px（原版 _width_spin）
        let mut v = settings.subtitle_mode.window_width.clamp(200, 3840) as i32;
        if number_row(
            ui,
            "sub_width",
            &lt_i18n::t("subwin_window_width"),
            &mut v,
            200..=3840,
            " px",
        ) {
            settings.subtitle_mode.window_width = v as u32;
            mark_settings_changed(panel, session);
        }
        // 行间距 0..=40 px
        let mut v = settings.subtitle_mode.line_spacing.min(40) as i32;
        if number_row(
            ui,
            "sub_spacing",
            &lt_i18n::t("subwin_line_spacing"),
            &mut v,
            0..=40,
            " px",
        ) {
            settings.subtitle_mode.line_spacing = v as u32;
            mark_settings_changed(panel, session);
        }
        // 圆角 0..=30 px
        let mut v = settings.subtitle_mode.border_radius.min(30) as i32;
        if number_row(
            ui,
            "sub_radius",
            &lt_i18n::t("subwin_border_radius"),
            &mut v,
            0..=30,
            " px",
        ) {
            settings.subtitle_mode.border_radius = v as u32;
            mark_settings_changed(panel, session);
        }
        // 位置复位（D-36：仅复位字幕窗回 (100,100)；样式页"重置窗口位置"仍复位两窗）
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("subwin_reset_pos")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                session.enqueue_action(
                    crate::state::WinId::Subtitle,
                    crate::state::WinAction::ResetSubtitlePos,
                );
            }
        });
    });

    // ── 背景（原版背景三要素：颜色/不透明度/背景图片；有图片时颜色仍可调，
    //     渲染层按图片存在性决定底色，原版 setEnabled 联动不保留）──
    group_card(ui, pal, &lt_i18n::t("subwin_background"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("subwin_bg_color"))).color(pal.text));
            let mut color = settings.subtitle_mode.bg_color.clone();
            if color_field(ui, "sub_bg_color", &mut color) {
                settings.subtitle_mode.bg_color = color;
                mark_settings_changed(panel, session);
            }
        });
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} ", lt_i18n::t("subwin_bg_opacity"))).color(pal.text),
            );
            let mut pct =
                (f64::from(settings.subtitle_mode.bg_opacity) / 255.0 * 100.0).round() as i32;
            let resp = ui
                .add(
                    egui::Slider::new(&mut pct, 0..=100)
                        .custom_formatter(|v, _| format!("{v:.0}%")),
                )
                .on_hover_text(lt_i18n::t("subwin_bg_opacity_hint"));
            if resp.changed() {
                settings.subtitle_mode.bg_opacity =
                    (f64::from(pct.clamp(0, 100)) / 100.0 * 255.0).round() as u32;
                mark_settings_changed(panel, session);
            }
        });
        bg_image_row(ui, panel, settings, session);
    });

    // ── 自动隐藏与穿透（原版 auto_hide_timeout/hide_animation/hide_duration +
    //     click_through）──
    group_card(ui, pal, &lt_i18n::t("subwin_animation"), |ui| {
        // 超时 0..=120 秒（0=禁用）
        let mut v = settings.subtitle_mode.auto_hide_timeout.min(120) as i32;
        if number_row(
            ui,
            "sub_auto_hide",
            &lt_i18n::t("subwin_auto_hide"),
            &mut v,
            0..=120,
            &format!(" {}", lt_i18n::t("subwin_auto_hide_sec")),
        ) {
            settings.subtitle_mode.auto_hide_timeout = v.max(0) as u32;
            mark_settings_changed(panel, session);
        }
        // 隐藏动画（none/fade/slide_down 三项）
        let hide_anims = ["none", "fade", "slide_down"];
        let labels: Vec<String> = hide_anims
            .iter()
            .map(|a| lt_i18n::t(&format!("subwin_anim_{a}")))
            .collect();
        let idx = hide_anims
            .iter()
            .position(|a| *a == settings.subtitle_mode.auto_hide_animation)
            .unwrap_or(1);
        if let Some(next) = combo_index(ui, "sub_hide_anim", idx, &labels, 160.0) {
            settings.subtitle_mode.auto_hide_animation = hide_anims[next].to_string();
            mark_settings_changed(panel, session);
        }
        // 时长 50..=3000 ms
        let mut v = settings.subtitle_mode.auto_hide_duration.clamp(50, 3000) as i32;
        if number_row(
            ui,
            "sub_hide_dur",
            &lt_i18n::t("subwin_hide_duration"),
            &mut v,
            50..=3000,
            " ms",
        ) {
            settings.subtitle_mode.auto_hide_duration = v as u32;
            mark_settings_changed(panel, session);
        }
        // 鼠标穿透（宿主 500ms 断言轮询按此开关续拍）
        let mut ct = settings.subtitle_mode.click_through;
        if ui
            .add(egui::Checkbox::new(
                &mut ct,
                RichText::new(lt_i18n::t("subwin_click_through")).color(pal.text),
            ))
            .on_hover_text(lt_i18n::t("subwin_click_through_hint"))
            .changed()
        {
            settings.subtitle_mode.click_through = ct;
            if ct
                && *session
                    .visible
                    .get(&crate::state::WinId::Subtitle)
                    .unwrap_or(&false)
            {
                // W5：跨域排班直接落会话节拍表（原本墙页不持有 SubtitleUi 域）
                session.schedule_tick(
                    crate::state::WinId::Subtitle,
                    crate::state::TickKind::ClickThrough,
                    std::time::Instant::now()
                        + std::time::Duration::from_millis(crate::state::SUBTITLE_POLL_MS),
                );
            }
            mark_settings_changed(panel, session);
        }
    });

    // ── 文字行（原版 lines_group：列表 + 五按钮 + 双击编辑）──
    group_card(ui, pal, &lt_i18n::t("subwin_text_lines"), |ui| {
        let count = settings.subtitle_mode.lines.len();
        // D-123 C1 三态裁决（state::row_pick_clicked）：再点已选行 = 取消；
        // 双击帧不翻转——净效果恒为「选中 + 开编辑」
        let mut pick = crate::state::RowPick::Idle;
        let mut edit_row: Option<usize> = None;
        for i in 0..count {
            let text = line_row_text(&settings.subtitle_mode.lines[i]);
            let row_selected = panel.state.line_selected == Some(i);
            let resp = ui
                .push_id(i, |ui| {
                    // D-124：列表行走 selectable_button_stable——裸
                    // Button::selectable 悬停跳变 +2px（已入 clippy 禁令）
                    crate::style::selectable_button_stable(
                        ui,
                        row_selected,
                        RichText::new(&text).monospace().size(12.0),
                        egui::vec2(ui.available_width(), 0.0),
                    )
                })
                .inner;
            if resp.clicked() {
                pick = crate::state::row_pick_clicked(
                    panel.state.line_selected,
                    i,
                    resp.double_clicked(),
                );
            }
            if resp.double_clicked() {
                // 双击行 = 直接进入编辑（对齐 translation 页 2026-09-11 修复；
                // 旧实现只武装「编辑」按钮，与模块头「双击行编辑」承诺不符）
                edit_row = Some(i);
                if i < count {
                    let line = settings.subtitle_mode.lines[i].clone();
                    panel.state.line_editor = Some(LineEditState::new_edit(i, &line));
                }
            }
        }
        match pick {
            crate::state::RowPick::Select(i) => panel.state.line_selected = Some(i),
            crate::state::RowPick::Deselect => panel.state.line_selected = None,
            crate::state::RowPick::Idle => {}
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_add")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                let idx = settings.subtitle_mode.lines.len();
                panel.state.line_editor = Some(LineEditState::new_add(idx));
            }
            let target = edit_row.or(panel.state.line_selected);
            if super::panel_btn(ui, &lt_i18n::t("btn_edit"), target.is_some(), 12.5, 6.0).clicked()
            {
                if let Some(i) = target {
                    if i < settings.subtitle_mode.lines.len() {
                        let line = settings.subtitle_mode.lines[i].clone();
                        panel.state.line_editor = Some(LineEditState::new_edit(i, &line));
                    }
                }
            }
            let can_remove = count > 1 && panel.state.line_selected.is_some();
            if super::panel_btn(ui, &lt_i18n::t("btn_remove"), can_remove, 12.5, 6.0).clicked() {
                if let Some(i) = panel.state.line_selected {
                    if i < count && count > 1 {
                        settings.subtitle_mode.lines.remove(i);
                        panel.state.line_selected = None;
                        mark_settings_dirty(session);
                    }
                }
            }
            // 上移/下移（原版 subwin_move_up/down；边界在 move_line_* 内判定）
            let up = panel.state.line_selected.is_some_and(|i| i > 0);
            if super::panel_btn(ui, &lt_i18n::t("subwin_move_up"), up, 12.5, 6.0).clicked() {
                if let Some(i) = panel.state.line_selected {
                    if move_line_up(&mut settings.subtitle_mode.lines, i) {
                        panel.state.line_selected = Some(i - 1);
                        mark_settings_dirty(session);
                    }
                }
            }
            let down = panel.state.line_selected.is_some_and(|i| i + 1 < count);
            if super::panel_btn(ui, &lt_i18n::t("subwin_move_down"), down, 12.5, 6.0).clicked() {
                if let Some(i) = panel.state.line_selected {
                    if move_line_down(&mut settings.subtitle_mode.lines, i) {
                        panel.state.line_selected = Some(i + 1);
                        mark_settings_dirty(session);
                    }
                }
            }
        });
    });

    ui.add_space(8.0);

    // ── LineEditDialog（egui::Window 居中模态区）──
    render_line_editor(ui, panel, session, settings, ctx);
}

/// 背景图片行（原版 _make_image_rows：路径 + 选择/清除）
fn bg_image_row(
    ui: &mut Ui,
    panel: &mut PanelUi,
    settings: &mut Settings,
    session: &mut SessionView,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} ", lt_i18n::t("subwin_bg_image")))
                .color(ui.visuals().text_color()),
        );
        let mut img = settings.subtitle_mode.bg_image.clone();
        if ui
            .add(
                egui::TextEdit::singleline(&mut img)
                    .hint_text("…/bg.png")
                    .desired_width(260.0),
            )
            .changed()
        {
            settings.subtitle_mode.bg_image = img.trim().to_string();
            mark_settings_changed(panel, session);
        }
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("subwin_bg_image_select")).size(12.0))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            // W5/R19：rfd 选择框移出事件循环线程——编排域 Supervisor 一次性
            // 线程弹框，路径经 UiEvent::BgImagePicked 回执写回（帧内零阻塞）
            session.send_cmd(lt_proto::Cmd::PickBgImage {
                dialog_title: lt_i18n::t("subwin_bg_image_select"),
            });
        }
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("subwin_bg_image_clear")).size(12.0))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            settings.subtitle_mode.bg_image.clear();
            mark_settings_changed(panel, session);
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
fn render_line_editor(
    ui: &mut Ui,
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    ctx: &mut UiContext,
) {
    if panel.state.line_editor.is_none() {
        return;
    }
    let mut open = true;
    let mut cancel = false;
    let mut accepted: Option<SubtitleLine> = None;
    let master = settings.subtitle_font_family.clone();
    {
        // panel.line_editor 与 fonts 是 AppState 不同字段，可并行借用
        let (p, fonts) = (&mut panel.state, &mut ctx.fonts);
        let Some(ed) = p.line_editor.as_mut() else {
            return;
        };
        // G-42 模态形态（D-126）：同 ModelEditDialog——窗体钳视口 + 显式 Id
        // （默认 Id 随标题文本变，语言热切换丢窗状态）；纵向随视口收缩
        let scr = ui.ctx().viewport_rect();
        egui::Window::new(RichText::new(lt_i18n::t("subwin_edit_line")).strong())
            .id(egui::Id::new("panel_line_edit_dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(460.0)
            .max_width((scr.width() - 16.0).max(300.0))
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height((scr.height() - 110.0).clamp(120.0, 440.0))
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
        let ed = panel.state.line_editor.take().expect("行编辑器打开中");
        let lines = &mut settings.subtitle_mode.lines;
        if ed.is_new {
            lines.push(line);
            panel.state.line_selected = Some(lines.len() - 1);
        } else if ed.index < lines.len() {
            lines[ed.index] = line;
            panel.state.line_selected = Some(ed.index);
        }
        // 行级字体键可能变更，字体链（命名族注册）同步重建
        crate::fonts::apply_fonts(ui.ctx(), settings, &mut ctx.fonts);
        mark_settings_dirty(session);
    } else if !open || cancel {
        panel.state.line_editor = None;
    }
}

/// 行编辑字段全集（原版 LineEditDialog QGridLayout 逐行；SubtitleLine 15 字段）
fn line_editor_fields(
    ui: &mut Ui,
    ed: &mut LineEditState,
    fonts: &mut crate::fonts::FontsState,
    master: &str,
) {
    // D-126：字体行不进 Grid——font_picker_row 是整行复合控件（小样 truncate
    // 需真实视口宽），Grid 列宽 sizing pass 以 INFINITY 排版单元格（egui
    // grid.rs max_cell_size 默认无穷），小样/滑条按整行自然宽把列撑到 ~620、
    // 窗体随之超出宿主视口（zh 实测窗宽 ~750 vs 视口 509）；任何「随
    // available_width 派生的列帽」都与「窗随内容走」构成正反馈（窗只涨不缩，
    // 实测 697→985 失控），唯一稳态解 = 让最宽的复合控件不进 Grid。
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
            let types = [
                lt_i18n::t("subwin_original"),
                lt_i18n::t("subwin_translation"),
            ];
            if let Some(next) = combo_index(ui, "line_edit_type", ed.line_type_index, &types, 200.0)
            {
                ed.line_type_index = next;
            }
            ui.end_row();

            // 目标语言（仅翻译行可编辑，原版 _update_lang_visibility）
            ui.add_enabled(
                ed.line_type_index == 1,
                egui::Label::new(lt_i18n::t("subwin_target_lang")),
            );
            let opts = lang_options();
            let idx = opts.iter().position(|(c, _)| *c == ed.lang).unwrap_or(2);
            let labels: Vec<String> = opts.iter().map(|(_, l)| l.clone()).collect();
            if ed.line_type_index == 1 {
                if let Some(next) = combo_index(ui, "line_edit_lang", idx, &labels, 200.0) {
                    ed.lang = opts[next].0.clone();
                }
            } else {
                // 原文行：语言不可编辑（灰显占位）。disabled = Noninteractive
                // 态无悬停分支（恒全帧），D-124 病不达——不经助手、就地豁免。
                #[allow(clippy::disallowed_methods)]
                ui.add_enabled(
                    false,
                    egui::Button::selectable(false, labels[idx].clone()).corner_radius(4.0),
                );
            }
            ui.end_row();

            // 字号 8..=120 pt
            ui.label(lt_i18n::t("subwin_font_size"));
            ui.add(
                egui::DragValue::new(&mut ed.font_size)
                    .range(8..=120)
                    .suffix(" pt"),
            );
            ui.end_row();

            // 颜色
            ui.label(lt_i18n::t("subwin_color"));
            color_field(ui, "line_edit_color", &mut ed.color);
            ui.end_row();

            // 不透明度 0-100%
            ui.label(lt_i18n::t("subwin_opacity"));
            ui.add(
                egui::Slider::new(&mut ed.opacity_pct, 0..=100)
                    .custom_formatter(|v, _| format!("{v:.0}%")),
            );
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
            ui.add(
                egui::DragValue::new(&mut ed.outline_width)
                    .range(0..=10)
                    .suffix(" px"),
            );
            ui.end_row();

            // 背景图片（文本 + 清除；图片文件选择按钮保留在页面级背景卡）
            ui.label(lt_i18n::t("subwin_bg_image"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.bg_image).desired_width(170.0));
                if ui
                    .small_button(lt_i18n::t("subwin_bg_image_clear"))
                    .clicked()
                {
                    ed.bg_image.clear();
                }
            });
            ui.end_row();

            // 入场/退场动画
            let anim_labels: Vec<String> = ANIM_VALUES
                .iter()
                .map(|v| lt_i18n::t(&format!("subwin_anim_{v}")))
                .collect();
            ui.label(lt_i18n::t("subwin_entry_anim"));
            if let Some(next) = combo_index(
                ui,
                "line_edit_entry",
                ed.entry_anim_index,
                &anim_labels,
                200.0,
            ) {
                ed.entry_anim_index = next;
            }
            ui.end_row();
            ui.label(lt_i18n::t("subwin_exit_anim"));
            if let Some(next) = combo_index(
                ui,
                "line_edit_exit",
                ed.exit_anim_index,
                &anim_labels,
                200.0,
            ) {
                ed.exit_anim_index = next;
            }
            ui.end_row();

            // 动画时长 50..=3000 ms
            ui.label(lt_i18n::t("subwin_anim_duration"));
            ui.add(
                egui::DragValue::new(&mut ed.animation_duration)
                    .range(50..=3000)
                    .suffix(" ms"),
            );
            ui.end_row();
        });

    // 字体（D-17：行级空串=跟随主设置；选择器含小样与缺字提示）——
    // 整行复合控件在 Grid 外页级渲染（同样式页用法，见上方 D-126 注）
    let cur_family = ed.font_family.clone();
    if let Some(next) = super::font_picker::font_picker_row(
        ui,
        fonts,
        "line_edit_font",
        &lt_i18n::t("subwin_font"),
        cur_family,
        master,
        true,
    ) {
        ed.font_family = next;
    }
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
        assert!(
            text.contains("跟随（默认） 24pt") || text.contains("Follow (default) 24pt"),
            "{text}"
        );
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
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        st.panel.state.page = crate::state::PanelPage::Subtitle;
        st.panel.state.line_editor = Some(LineEditState::new_edit(
            0,
            &st.settings.subtitle_mode.lines[0],
        ));
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st)
            });
            assert!(!out.shapes.is_empty(), "行编辑器打开态应产出图元");
            out.textures_delta.clear();
        }
        assert!(
            st.panel.state.line_editor.is_some(),
            "编辑器保持打开（状态未被意外消费）"
        );
    }

    // ── D-123 列表选中态（headless 点击取证，共用件 = click_testing）──

    /// 组装已渲染好的字幕页测试环境（zh + Subtitle 页 + 700×1400 视口）
    fn selection_env() -> (
        crate::windows::panel::click_testing::PanelHarness,
        crate::state::AppUi,
    ) {
        let _ = lt_i18n::set_lang("zh");
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = crate::state::PanelPage::Subtitle;
        let h = crate::windows::panel::click_testing::PanelHarness::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(700.0, 1400.0),
        ));
        (h, st)
    }

    /// T1：文字行再点已选行 = 取消选中（C1）
    #[test]
    fn line_list_reclick_toggles_selection_off() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle = line_row_text(&st.settings.subtitle_mode.lines[0]);
        let out = h.render(&mut st, vec![]);
        let pos = ct::text_center(&out.shapes, &needle).expect("文字行摘要应上屏");

        h.click(&mut st, pos);
        assert_eq!(st.panel.state.line_selected, Some(0), "点行应选中");
        // 快进拉出双击窗口（同点连点两次会被 egui 判成双击，双击帧不翻转）
        h.idle(&mut st, 25);
        h.click(&mut st, pos);
        assert_eq!(
            st.panel.state.line_selected, None,
            "再点已选行应取消选中（C1）"
        );
    }

    /// T6a：上移后选中跟随（列表自身操作豁免 C4，不得被 funnel 误清）
    #[test]
    fn line_move_up_keeps_selection_following() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle1 = line_row_text(&st.settings.subtitle_mode.lines[1]);
        let out = h.render(&mut st, vec![]);
        let pos1 = ct::text_center(&out.shapes, &needle1).expect("文字行 1 摘要应上屏");
        h.click(&mut st, pos1);
        assert_eq!(st.panel.state.line_selected, Some(1));

        let out = h.render(&mut st, vec![]);
        let up =
            ct::text_center(&out.shapes, &lt_i18n::t("subwin_move_up")).expect("上移按钮应上屏");
        h.click(&mut st, up);
        assert_eq!(
            st.panel.state.line_selected,
            Some(0),
            "上移后选中应跟随行（豁免 funnel）"
        );
        assert_eq!(st.settings.subtitle_mode.lines[0].line_type, "translation");
    }

    /// T6b：删除所选后选中清除（现语义保持，count>1 守卫内）
    #[test]
    fn line_delete_clears_selection() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle1 = line_row_text(&st.settings.subtitle_mode.lines[1]);
        let out = h.render(&mut st, vec![]);
        let pos1 = ct::text_center(&out.shapes, &needle1).expect("文字行 1 摘要应上屏");
        h.click(&mut st, pos1);

        let out = h.render(&mut st, vec![]);
        let del = ct::text_center(&out.shapes, &lt_i18n::t("btn_remove")).expect("删除按钮应上屏");
        h.click(&mut st, del);
        assert_eq!(st.settings.subtitle_mode.lines.len(), 1, "应删至一行");
        assert_eq!(st.panel.state.line_selected, None, "删除后选中应清除");
    }

    /// T6d：新增 → 确认后选中落在新行（豁免 funnel；§5 T6「新增」分支）
    #[test]
    fn line_add_accept_selects_new_row() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;
        assert_eq!(st.settings.subtitle_mode.lines.len(), 2);

        let out = h.render(&mut st, vec![]);
        let add = ct::text_center(&out.shapes, &lt_i18n::t("btn_add")).expect("添加按钮应上屏");
        h.click(&mut st, add);
        assert!(st.panel.state.line_editor.is_some(), "添加应打开行编辑模态");

        // 窗口 sizing 瞬时态：至多 3 帧内「确定」必须上屏（同 T6c 注）
        let mut ok = None;
        for _ in 0..3 {
            let out = h.render(&mut st, vec![]);
            ok = ct::text_center(&out.shapes, &lt_i18n::t("common_ok"));
            if ok.is_some() {
                break;
            }
        }
        let ok = ok.expect("模态「确定」应上屏（3 帧内）");
        h.idle(&mut st, 25);
        h.click(&mut st, ok);
        assert!(st.panel.state.line_editor.is_none(), "确认后模态应关闭");
        assert_eq!(st.settings.subtitle_mode.lines.len(), 3, "应新增一行");
        assert_eq!(
            st.panel.state.line_selected,
            Some(2),
            "新增确认后选中应落在新行"
        );
    }

    /// T6c：选中 → 编辑按钮开模态 → 确认后选中落回该行（豁免 funnel）
    #[test]
    fn line_edit_accept_reasserts_selection() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle = line_row_text(&st.settings.subtitle_mode.lines[0]);
        let out = h.render(&mut st, vec![]);
        let pos = ct::text_center(&out.shapes, &needle).expect("文字行 0 摘要应上屏");
        h.click(&mut st, pos);

        let out = h.render(&mut st, vec![]);
        let edit = ct::text_center(&out.shapes, &lt_i18n::t("btn_edit")).expect("编辑按钮应上屏");
        h.idle(&mut st, 25);
        h.click(&mut st, edit);
        assert!(st.panel.state.line_editor.is_some(), "编辑应打开行编辑模态");

        // egui Window 开窗首两帧 sizing 未含底部按钮行（内嵌 ScrollArea
        // auto_shrink(false) 与窗口尺寸收敛的瞬时态，真实应用连续帧不可见）
        // ——有界 settle：至多 3 帧内「确定」必须上屏
        let mut ok = None;
        for _ in 0..3 {
            let out = h.render(&mut st, vec![]);
            ok = ct::text_center(&out.shapes, &lt_i18n::t("common_ok"));
            if ok.is_some() {
                break;
            }
        }
        let ok = ok.expect("模态「确定」应上屏（3 帧内）");
        h.idle(&mut st, 25);
        h.click(&mut st, ok);
        assert!(st.panel.state.line_editor.is_none(), "确认后模态应关闭");
        assert_eq!(
            st.panel.state.line_selected,
            Some(0),
            "编辑确认后选中应落回该行"
        );
    }

    /// T8：双击行 = 直接打开行编辑模态且行保持选中（背靠背两次 click 间隔
    /// ≈2 帧 < 双击窗口，恰好构成 double_clicked）
    #[test]
    fn line_double_click_opens_editor_and_keeps_selection() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle = line_row_text(&st.settings.subtitle_mode.lines[0]);
        let out = h.render(&mut st, vec![]);
        let pos = ct::text_center(&out.shapes, &needle).expect("文字行 0 摘要应上屏");

        h.click(&mut st, pos);
        h.click(&mut st, pos);
        assert!(
            st.panel.state.line_editor.is_some(),
            "双击应直接打开行编辑模态"
        );
        assert_eq!(
            st.panel.state.line_selected,
            Some(0),
            "双击净效果 = 选中保持（双击帧不翻转 toggle）"
        );
    }

    /// T5：同 tab 改其他设置（字幕窗口开关）→ 文字行选中被清（C4 funnel）
    #[test]
    fn subtitle_control_change_clears_line_selection() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let (mut h, mut st) = selection_env();
        use crate::windows::panel::click_testing as ct;

        let needle = line_row_text(&st.settings.subtitle_mode.lines[0]);
        let out = h.render(&mut st, vec![]);
        let pos = ct::text_center(&out.shapes, &needle).expect("文字行 0 摘要应上屏");
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.line_selected, Some(0));

        let enabled_before = st.settings.subtitle_mode.enabled;
        let out = h.render(&mut st, vec![]);
        let checkbox =
            ct::text_center(&out.shapes, &lt_i18n::t("subwin_show")).expect("字幕窗口复选框应上屏");
        h.idle(&mut st, 25);
        h.click(&mut st, checkbox);
        assert_ne!(
            st.settings.subtitle_mode.enabled, enabled_before,
            "复选框应已翻转（清选中断言的 sanity 前置）"
        );
        assert_eq!(
            st.panel.state.line_selected, None,
            "改同 tab 其他设置应清文字行选中（C4）"
        );
    }
}
