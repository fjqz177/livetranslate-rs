//! 更新日志 Tab（对照原版 control_panel.py `_create_changelog_tab` +
//! dialogs.py `_load_latest_changelog`）：
//! 数据源 = 原版 `i18n/CHANGELOG_{lang}.md`（内嵌 assets/i18n/），语言回退
//! 顺序与原版一致（当前语言无文件 → en）。渲染规则对齐 `_changelog_to_html`：
//! `# 文件标题`跳过、`## 日期`大标题、`- 条目`列表（**粗体** / `code` 内联）、
//! 其余段落正文。
//!
//! 已知偏差：egui 无富文本 HTML → 内联 **bold**/`code` 用 painter 逐段
//! 分色绘制（简单词法扫描，不支持嵌套）；条目缩进/间距按原版视觉近似。

use super::{group_card, Palette};
use crate::state::AppState;
use egui::{Color32, RichText, ScrollArea, Ui};

const ZH_MD: &str = include_str!("../../../../../assets/i18n/CHANGELOG_zh.md");
const EN_MD: &str = include_str!("../../../../../assets/i18n/CHANGELOG_en.md");

/// 按当前界面语言取 changelog 原文（原版 _load_latest_changelog 的回退顺序）
pub fn changelog_text(lang: &str) -> &'static str {
    match lang {
        "zh" => ZH_MD,
        _ => EN_MD,
    }
}

/// 更新日志 Tab UI 总入口（panel_ui 按 PanelPage::Changelog 分派）
pub fn page(ui: &mut Ui, _state: &mut AppState, _pal: &Palette) {
    let text = changelog_text(&lt_i18n::get_lang());
    group_card(ui, _pal, &lt_i18n::t("group_changelog"), |ui| {
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let min_h = (ui.available_height() - 8.0).max(300.0);
                ui.set_min_height(min_h);
                render_markdown(ui, text);
            });
    });
}

/// 轻量 markdown 渲染（规则对齐原版 _changelog_to_html）：
/// `# ` 跳过、`## ` 日期大标题、`- ` 列表条目、空行分段、其余正文。
fn render_markdown(ui: &mut Ui, text: &str) {
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("# ") {
            continue; // 文件标题（原版 skip file title）
        }
        if trimmed.is_empty() {
            ui.add_space(6.0);
            continue;
        }
        if let Some(date) = trimmed.strip_prefix("## ") {
            ui.add_space(4.0);
            ui.label(RichText::new(date).strong().size(17.0).color(ui.visuals().text_color()));
            ui.add_space(2.0);
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            ui.horizontal(|ui| {
                ui.label(RichText::new("•").size(12.5));
                inline_rich(ui, item, 12.5);
            });
            ui.add_space(2.0);
            continue;
        }
        inline_rich(ui, trimmed, 12.5);
        ui.add_space(2.0);
    }
}

/// 内联富文本：**粗体** 与 `等宽` 分段绘制（原版 re.sub → b/code 的等价物；
/// 扫描器不支持嵌套，未闭合标记按字面输出——原版正则同样不匹配未闭合对）。
fn inline_rich(ui: &mut Ui, text: &str, size: f32) {
    ui.horizontal_wrapped(|ui| {
        let mut rest = text;
        loop {
            // 找最近的 ** 或 ` 起始标记
            let bold_pos = rest.find("**");
            let code_pos = rest.find('`');
            let next = match (bold_pos, code_pos) {
                (Some(b), Some(c)) => Some(if b < c { (b, true) } else { (c, false) }),
                (Some(b), None) => Some((b, true)),
                (None, Some(c)) => Some((c, false)),
                (None, None) => None,
            };
            let Some((pos, is_bold)) = next else {
                ui.label(RichText::new(rest).size(size));
                break;
            };
            if pos > 0 {
                ui.label(RichText::new(&rest[..pos]).size(size));
            }
            let after = &rest[pos + if is_bold { 2 } else { 1 }..];
            let end_mark = if is_bold { "**" } else { "`" };
            match after.find(end_mark) {
                Some(end) => {
                    let seg = &after[..end];
                    if is_bold {
                        ui.label(RichText::new(seg).strong().size(size));
                    } else {
                        ui.label(
                            RichText::new(seg)
                                .monospace()
                                .size(size - 1.0)
                                .color(Color32::from_rgb(0x8A, 0x5A, 0x00)),
                        );
                    }
                    rest = &after[end + 2..];
                }
                None => {
                    // 未闭合：字面输出剩余部分
                    ui.label(RichText::new(&rest[pos..]).size(size));
                    break;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 语言回退：zh → 中文表；其他 → en 表（原版 path.exists() 回退）
    #[test]
    fn changelog_lang_fallback() {
        assert!(changelog_text("zh").contains("更新日志") || changelog_text("zh").contains("## "));
        assert!(changelog_text("en").contains("## "));
        assert!(changelog_text("fr").starts_with('#'), "非 zh 回退 en");
    }

    /// 渲染规则：`# 文件标题`行不产出版本日期；`## `行是日期标题
    #[test]
    fn changelog_shape_matches_original_rules() {
        let text = changelog_text("zh");
        let mut h2 = 0;
        for line in text.lines() {
            if line.starts_with("## ") {
                h2 += 1;
            }
        }
        assert!(h2 >= 3, "CHANGELOG_zh.md 应含多个日期条目，实际 {h2}");
    }
}
