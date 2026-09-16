//! 更新日志 Tab：数据源 = 仓根 `CHANGELOG.md` / `CHANGELOG.en.md`（编译期 `include_str!` 内嵌；
//! 与发布链同一份正典——发版三件套闸见 `scripts/release.ps1` 的「更新日志」段）。
//! 语言回退：zh → 中文表，其余 → 英文表（沿用原版语义）。
//!
//! 渲染规则：`# 文件标题`跳过、`## [x.y.z] - 日期`大标题、`- 条目`（**粗体** / `code` 内联）、
//! 水平线 `---`/`***`/`___` → 分隔线、其余段落正文。
//!
//! 已知偏差（用户 2026-09-16 裁决：渲染改造缓办，移交清单见 docs/archive/changelog-scheme.md §5.6）：
//! 版本标题的方括号照原样显示；`###` 无层级（按正文渲染）；`*`/`+`/`1.` 列表标记无圆点；
//! 链接/表格/图片按字面文本。

use super::{group_card, Palette};
use egui::{Color32, RichText, Ui};

// 正典在仓库根（与发布链、GitHub Release 正文同源；发版三件套闸见 scripts/release.ps1）
const ZH_MD: &str = include_str!("../../../../../CHANGELOG.md");
const EN_MD: &str = include_str!("../../../../../CHANGELOG.en.md");

/// 按当前界面语言取 changelog 原文（原版 _load_latest_changelog 的回退顺序）
pub fn changelog_text(lang: &str) -> &'static str {
    match lang {
        "zh" => ZH_MD,
        _ => EN_MD,
    }
}

/// 更新日志 Tab UI 总入口（panel_ui 按 PanelPage::Changelog 分派）。
/// 已知偏差：原版是 group 框内 QTextBrowser 自滚；此处直接流式排布、由
/// 面板整页 ScrollArea 滚动——嵌套 ScrollArea 会叠加双层滚动条占位宽
/// （6px×2）造成文本 wrap 宽度与可见宽度错位、行尾被裁（实机走查修复）。
pub fn page(ui: &mut Ui, _pal: &Palette) {
    // WD-2：版本行（弱化显示；与 --version 同源 = CARGO_PKG_VERSION）
    ui.label(
        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
            .weak()
            .small(),
    );
    let text = changelog_text(&lt_i18n::get_lang());
    group_card(ui, _pal, &lt_i18n::t("group_changelog"), |ui| {
        // wrapped 行宽收 10px 安全量：ScrollArea 的 wrap 分配宽与 clip 缘存在
        // 滚动条占位微差，行尾字符会被裁（实机走查 2026-09-07）
        ui.set_max_width(ui.available_width() - 10.0);
        render_markdown(ui, text);
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
        // 水平线（`---` / `***` / `___`）：标准 Markdown 的分段标记 → 画成分隔线，
        // 别按正文打印裸横线（用户 2026-09-16 要求保留分隔线；其余渲染改造已缓办）
        if is_thematic_break(trimmed) {
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);
            continue;
        }
        if let Some(date) = trimmed.strip_prefix("## ") {
            ui.add_space(4.0);
            ui.label(
                RichText::new(date)
                    .strong()
                    .size(17.0)
                    .color(ui.visuals().text_color()),
            );
            ui.add_space(2.0);
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            // 必须用 wrapped 容器：horizontal 给 child 无限可用宽，内层
            // 永不换行 → 文本横向溢出贴边裁切（实机走查 2026-09-07）
            ui.horizontal_wrapped(|ui| {
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
                    rest = &after[end + mark_len(is_bold)..];
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

/// 标准 Markdown 的水平线：≥3 个同类字符（`-` / `*` / `_`，两侧空白容忍）
fn is_thematic_break(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && (t.bytes().all(|b| b == b'-')
            || t.bytes().all(|b| b == b'*')
            || t.bytes().all(|b| b == b'_'))
}

/// 标记闭合后的跳过长度（** 2 字节 / ` 1 字节；错切多字节 UTF-8 会 panic）
fn mark_len(is_bold: bool) -> usize {
    if is_bold {
        2
    } else {
        1
    }
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

    /// 全量无头渲染 zh/en changelog 不 panic（回归：inline_rich 对
    /// `code` 标记按 1 字节跳过，此前统一 +2 切进中文 UTF-8 边界崩溃）
    #[test]
    fn changelog_full_render_headless_no_panic() {
        for lang in ["zh", "en"] {
            let ctx = egui::Context::default();
            let text = changelog_text(lang);
            for _ in 0..2 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| render_markdown(ui, text));
                });
                // epaint debug 断言要求消费纹理增量（无渲染器 → 显式丢弃）
                out.textures_delta.clear();
            }
        }
    }

    /// 结构：至少一个版本段，且首段就是当前版本（抬版本号与写段落必须同提交）
    #[test]
    fn changelog_top_section_is_current_version() {
        let v = env!("CARGO_PKG_VERSION");
        let first = changelog_text("zh")
            .lines()
            .find(|l| l.starts_with("## "))
            .expect("CHANGELOG.md 应至少有一个版本段落（## [x.y.z] - YYYY-MM-DD）");
        assert!(
            first.contains(&format!("[{v}]")),
            "首段应为当前版本 [{v}]，实际：{first}"
        );
    }

    /// 双语守恒：zh / en 的版本标题列表逐字相等（含顺序）
    #[test]
    fn changelog_zh_en_versions_match() {
        fn heads(t: &str) -> Vec<&str> {
            t.lines().filter(|l| l.starts_with("## ")).collect()
        }
        let (zh, en) = (heads(changelog_text("zh")), heads(changelog_text("en")));
        assert!(!zh.is_empty(), "CHANGELOG.md 应至少有一个版本段落");
        assert_eq!(
            zh, en,
            "CHANGELOG.en.md 的版本段必须与中文逐字一致（号 + 日期 + 顺序）"
        );
    }

    /// 水平线识别（纯函数直测，不走 UI）
    #[test]
    fn thematic_break_detection() {
        for s in ["---", "----", "***", "___", "  ---  "] {
            assert!(is_thematic_break(s), "{s} 应视为分隔线");
        }
        for s in ["- 条目", "--", "**粗体**", "-*-", ""] {
            assert!(!is_thematic_break(s), "{s} 不应视为分隔线");
        }
    }
}
