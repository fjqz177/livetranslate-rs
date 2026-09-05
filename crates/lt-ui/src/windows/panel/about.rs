//! 关于页（对照原版 panel/tabs/about_tab.py 的本批子集）：
//! 应用名 + 版本 + 简介、项目链接（仓库/Issues，系统浏览器打开）、
//! 许可与隐私文本。
//!
//! 已知偏差：原版"检查更新"（GitHub releases API）与"更新日志"
//! （CHANGELOG_{lang}.md 渲染）随更新器里程碑接入，本批不做。

use super::{group_card, open_url, Palette};
use crate::state::AppState;
use egui::{RichText, Ui};

/// 项目主页（对照原版 about_tab.PROJECT_URL）
pub const PROJECT_URL: &str = "https://github.com/fjqz177/LiveTranslate";
/// Issues 页（原版 ISSUES_URL）
pub const ISSUES_URL: &str = "https://github.com/fjqz177/LiveTranslate/issues";

/// 应用版本（workspace 版本，与托盘/诊断页一致）
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 关于页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    let _ = state;

    // ── 应用名 + 版本 + 简介（原版页首两行）──
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!(
            "LiveTranslate  ·  {}",
            lt_i18n::t("about_version").replace("{version}", app_version())
        ))
        .strong()
        .size(16.0)
        .color(pal.title),
    );
    ui.label(RichText::new(lt_i18n::t("about_desc")).size(12.0).color(pal.weak));

    // ── 项目链接（原版 links_group：仓库 / Issues）──
    group_card(ui, pal, &lt_i18n::t("group_links"), |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_open_repo")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                open_url(PROJECT_URL);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_open_issues")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                open_url(ISSUES_URL);
            }
        });
    });

    // ── 许可与隐私（原版 license_group；文本可选中由 egui 文本选择承担）──
    group_card(ui, pal, &lt_i18n::t("group_license"), |ui| {
        ui.label(RichText::new(lt_i18n::t("about_license_text")).color(pal.text));
    });

    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 版本常量与链接形状（对照原版 PROJECT_URL / ISSUES_URL）
    #[test]
    fn version_and_links_match_original() {
        assert!(!app_version().is_empty());
        assert_eq!(PROJECT_URL, "https://github.com/fjqz177/LiveTranslate");
        assert_eq!(ISSUES_URL, format!("{PROJECT_URL}/issues"));
        // 版本号形状 x.y.z（workspace 语义版本）
        assert_eq!(app_version().split('.').count(), 3);
    }
}
