//! 通用字体行控件（docs/font-system-plan.md §4.3.4）：
//! ComboBox（内嵌置顶 + 系统扫描 + 搜索过滤）+ 行内小样 + 「跟随」首项（行级用）
//! + 刷新 + 缺字提示。
//!
//! 交互语义：选择 = 立即返回新值（调用方写回设置并 apply_fonts 即时生效）；
//! 行级选择器首项「跟随（默认）」→ 空串（D-17 级联）。

use crate::fonts::{self, FontsState, SystemFont};
use egui::{ComboBox, FontId, RichText, ScrollArea, TextEdit, Ui};
use lt_i18n::t;

/// 选择器项（纯数据；构造函数供单测）。
#[derive(Debug, Clone, PartialEq)]
pub enum PickerEntry {
    Follow,
    Embedded,
    System(String),
}

/// 选择器项构造（纯函数）：follow 首项 → 内嵌 → 系统族名（按显示名字母序，
/// filter 子串过滤忽略大小写）。
pub fn picker_entries(follow: bool, system: &[SystemFont], filter: &str) -> Vec<PickerEntry> {
    let mut out = Vec::new();
    if follow {
        out.push(PickerEntry::Follow);
    }
    out.push(PickerEntry::Embedded);
    let f = filter.trim().to_lowercase();
    let mut names: Vec<String> = system
        .iter()
        .filter(|s| f.is_empty() || s.display.to_lowercase().contains(&f))
        .map(|s| s.display.clone())
        .collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    out.extend(names.into_iter().map(PickerEntry::System));
    out
}

/// 通用字体行：返回 Some(新值) = 用户选择了字体（「跟随」→ 空串）。
///
/// - `current`：当前存储值（行级空串 = 跟随）
/// - `master`：跟随目标族名（行级传 subtitle_font_family；主设置行传自身）
/// - `is_line_level`：true = 首项「跟随（默认）」
pub fn font_picker_row(
    ui: &mut Ui,
    fonts: &mut FontsState,
    id: &str,
    label: &str,
    current: String,
    master: &str,
    is_line_level: bool,
) -> Option<String> {
    let mut result = None;
    ui.horizontal(|ui| {
        if !label.is_empty() {
            ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
        }
        let shown = if current.trim().is_empty() {
            t("label_font_follow")
        } else {
            current.clone()
        };
        ComboBox::from_id_salt(id)
            .width(240.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let mut filter = String::new();
                ui.add(
                    TextEdit::singleline(&mut filter)
                        .desired_width(220.0)
                        .hint_text(t("label_font_filter")),
                );
                let entries = picker_entries(is_line_level, &fonts.system, &filter);
                ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for entry in entries {
                        let (label, value) = match entry {
                            PickerEntry::Follow => (t("label_font_follow"), String::new()),
                            PickerEntry::Embedded => (
                                format!("{}（{}）", fonts::EMBEDDED_FAMILY, t("label_font_embedded")),
                                fonts::EMBEDDED_FAMILY.to_string(),
                            ),
                            PickerEntry::System(name) => (name.clone(), name),
                        };
                        if ui.selectable_label(current.trim() == value, label).clicked() {
                            result = Some(value);
                        }
                    }
                });
            });

        if ui
            .add(egui::Button::new(RichText::new(t("btn_font_rescan")).size(11.5)))
            .clicked()
        {
            fonts.rescan();
        }

        let resolved = fonts::resolve_family(&current, master);
        ui.label(
            RichText::new(t("label_resolved_font").replace("{family}", resolved))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
    });

    // 行内小样（截断）+ 缺字提示：即时反映当前解析结果（选中即所见）
    let resolved = fonts::resolve_family(&current, master).to_string();
    let fam = fonts::font_family_for(&resolved, fonts);
    ui.add(
        egui::Label::new(
            RichText::new(fonts::SAMPLE_TEXT)
                .size(12.0)
                .font(FontId::new(12.0, fam.clone())),
        )
        .truncate(),
    );
    if !current.trim().is_empty() && !fonts::family_covers_cjk(ui.ctx(), &fam) {
        ui.label(
            RichText::new(t("font_cjk_fallback_hint"))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        );
    }
    result
}

/// 字体组预览卡：界面字体与字幕字体各一行中英韩样例（选中即所见）。
pub fn font_group_preview(ui: &mut Ui, state: &crate::state::AppState) {
    let ui_fam = fonts::font_family_for(&state.settings.ui_font_family, &state.fonts);
    let sub_fam = fonts::font_family_for(
        fonts::resolve_family("", &state.settings.subtitle_font_family),
        &state.fonts,
    );
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "{}：{}",
            lt_i18n::t("label_ui_font").trim_end_matches(':'),
            fonts::SAMPLE_TEXT
        ))
        .size(13.0)
        .font(FontId::new(13.0, ui_fam)),
    );
    ui.label(
        RichText::new(format!(
            "{}：{}",
            lt_i18n::t("label_subtitle_font").trim_end_matches(':'),
            fonts::SAMPLE_TEXT
        ))
        .size(13.0)
        .font(FontId::new(13.0, sub_fam)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sys(names: &[&str]) -> Vec<SystemFont> {
        names
            .iter()
            .map(|n| SystemFont {
                display: n.to_string(),
                path: PathBuf::from(format!("C:\\Windows\\Fonts\\{n}.ttf")),
            })
            .collect()
    }

    #[test]
    fn entries_follow_embedded_then_sorted_system() {
        let entries = picker_entries(true, &sys(&["Segoe UI", "微软雅黑", "Arial"]), "");
        assert_eq!(entries[0], PickerEntry::Follow);
        assert_eq!(entries[1], PickerEntry::Embedded);
        assert_eq!(
            entries[2..],
            vec![
                PickerEntry::System("Arial".into()),
                PickerEntry::System("Segoe UI".into()),
                PickerEntry::System("微软雅黑".into()),
            ]
        );
    }

    #[test]
    fn entries_without_follow_for_master_rows() {
        let entries = picker_entries(false, &sys(&["Arial"]), "");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], PickerEntry::Embedded);
    }

    #[test]
    fn entries_filter_case_insensitive() {
        let entries = picker_entries(false, &sys(&["Segoe UI", "Arial", "meiryo"]), "ar");
        assert_eq!(entries.len(), 2, "过滤后只剩匹配项 + 内嵌置顶");
        assert_eq!(entries[1], PickerEntry::System("Arial".into()));
    }
}
