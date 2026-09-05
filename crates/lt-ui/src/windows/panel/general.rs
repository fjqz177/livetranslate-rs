//! 常规页（对照原版 ui/panel/tabs/general_tab.py 全部控件）：
//! 界面语言 / 外观（深浅色）/ 启动（自启 + 隐藏启动）/ 动效（减少动效）/
//! 窗口布局（重置位置）/ 设置导入导出。
//!
//! 契约缺口（settings 无对应键，lt-proto 不动 → 存 [`crate::state::PanelUiState`] 内存态）：
//! theme（外观）/ autostart（注册表即事实源）/ start_hidden / reduce_motion。
//! 界面语言：写 `settings.ui_lang` 并落盘；与原版一致不做热切换
//! （提示 ui_lang_restart_hint），新语言下次启动生效。

use super::{group_card, hint_line, import_settings_json, mark_settings_dirty, Palette};
use crate::state::{AppState, ThemeMode, WinAction, WinId};
use egui::{RichText, Ui};

/// 界面语言三选项：data 载荷与显示键（原版 addItem(t("lang_*"), code)）
pub const UI_LANGS: [(&str, &str); 3] =
    [("system", "lang_system"), ("zh", "lang_zh"), ("en", "lang_en")];

/// settings.ui_lang → 下拉索引；非法值回退 "system"（原版 stored not in (...) 分支）
pub fn ui_lang_index_for(stored: &str) -> usize {
    UI_LANGS
        .iter()
        .position(|(code, _)| *code == stored)
        .unwrap_or(0)
}

/// 常规页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // 进入页面时回读自启事实源（注册表），此后勾选变化即时写入
    if state.panel.autostart.is_none() {
        state.panel.autostart = Some(super::autostart::enabled());
    }

    // ── 界面语言（原版 lang_group）──
    group_card(ui, pal, &lt_i18n::t("group_ui_language"), |ui| {
        let mut idx = ui_lang_index_for(&state.settings.ui_lang);
        let items: Vec<String> = UI_LANGS.iter().map(|(_, key)| lt_i18n::t(key)).collect();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_ui_lang"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_ui_lang")
                .selected_text(items[idx].clone())
                .width(220.0)
                .show_ui(ui, |ui| {
                    for (i, label) in items.iter().enumerate() {
                        if ui.selectable_label(idx == i, label.clone()).clicked() {
                            idx = i;
                        }
                    }
                });
        });
        // 变更：写 ui_lang + 防抖落盘（不做热切换，原版 _on_ui_lang_changed 的
        // set_lang 只影响后续打开的对话框——egui 全帧重绘会整体换文案，故不调）
        let stored = state.settings.ui_lang.clone();
        let next = UI_LANGS[idx].0.to_string();
        if stored != next {
            state.settings.ui_lang = next;
            mark_settings_dirty(state);
        }
        ui.add_space(2.0);
        hint_line(ui, pal, &lt_i18n::t("ui_lang_restart_hint"));
    });

    // ── 外观（原版 appearance_group：深色/浅色 radio）──
    group_card(ui, pal, &lt_i18n::t("group_appearance"), |ui| {
        ui.horizontal(|ui| {
            let mut theme = state.panel.theme;
            if ui
                .radio(theme == ThemeMode::Dark, RichText::new(lt_i18n::t("theme_dark")).color(pal.text))
                .clicked()
            {
                theme = ThemeMode::Dark;
            }
            if ui
                .radio(theme == ThemeMode::Light, RichText::new(lt_i18n::t("theme_light")).color(pal.text))
                .clicked()
            {
                theme = ThemeMode::Light;
            }
            if theme != state.panel.theme {
                state.panel.theme = theme;
                // 立即切换 egui 主题（原版 set_theme_mode → apply_app_theme 应用级 repolish）
                ui.ctx().set_theme(match theme {
                    ThemeMode::Dark => egui::ThemePreference::Dark,
                    ThemeMode::Light => egui::ThemePreference::Light,
                });
                // 内存态：settings 契约缺 theme 键 → 不落盘，重启回深色默认
            }
        });
    });

    // ── 启动（原版 startup_group）──
    group_card(ui, pal, &lt_i18n::t("group_startup"), |ui| {
        // 开机自启：注册表即时生效；失败提示并回滚（原版 _on_autostart_toggled）
        let mut auto = state.panel.autostart.unwrap_or(false);
        if ui
            .add(egui::Checkbox::new(&mut auto, RichText::new(lt_i18n::t("label_autostart")).color(pal.text)))
            .changed()
        {
            match super::autostart::set_enabled(auto) {
                Ok(()) => state.panel.autostart = Some(auto),
                Err(e) => {
                    tracing::warn!("设置开机自启失败: {e}");
                    state.panel.autostart = Some(!auto); // 回滚勾选
                    rfd::MessageDialog::new()
                        .set_title(lt_i18n::t("autostart_failed_title"))
                        .set_description(lt_i18n::t("autostart_failed_msg"))
                        .set_buttons(rfd::MessageButtons::Ok)
                        .set_level(rfd::MessageLevel::Warning)
                        .show();
                }
            }
            // 内存态：settings 契约缺 autostart 键（注册表即事实源）→ 不落盘
        }
        let mut hidden = state.panel.start_hidden;
        if ui
            .add(egui::Checkbox::new(&mut hidden, RichText::new(lt_i18n::t("label_start_hidden")).color(pal.text)))
            .on_hover_text(lt_i18n::t("hide_tray_hint"))
            .changed()
        {
            state.panel.start_hidden = hidden;
            // 内存态：settings 契约缺 start_hidden 键 → 不落盘（生效随 M4.4 启动流）
        }
    });

    // ── 动效（原版 motion_group）──
    group_card(ui, pal, &lt_i18n::t("group_motion"), |ui| {
        let mut reduce = state.panel.reduce_motion;
        if ui
            .add(egui::Checkbox::new(&mut reduce, RichText::new(lt_i18n::t("label_reduce_motion")).color(pal.text)))
            .changed()
        {
            state.panel.reduce_motion = reduce;
            // 字幕窗高度过渡即时生效（SubtitleUiState.reduce_motion 的 M4.3 接线点）
            state.subtitle.reduce_motion = reduce;
            // 内存态：settings 契约缺 reduce_motion 键 → 不落盘
        }
    });

    // ── 窗口布局（原版 layout_group：重置窗口位置）──
    group_card(ui, pal, &lt_i18n::t("group_window_layout"), |ui| {
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("btn_reset_positions")).size(12.5))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            // 原版 reset_positions 信号 → app_shell._on_reset_positions（宿主层执行）
            state.enqueue_action(WinId::Panel, WinAction::ResetPositions);
        }
    });

    // ── 设置导入导出（原版 io_group）──
    group_card(ui, pal, &lt_i18n::t("group_settings_io"), |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_export_settings")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                export_settings(state);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_import_settings")).size(12.5))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                import_settings(state);
            }
        });
    });

    ui.add_space(8.0);
}

/// 导出设置（原版 _export_settings：明文密钥警告 → 保存框 → JSON 全量写出）
fn export_settings(state: &AppState) {
    let confirmed = rfd::MessageDialog::new()
        .set_title(lt_i18n::t("export_warn_title"))
        .set_description(lt_i18n::t("export_warn_msg"))
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show();
    if confirmed != rfd::MessageDialogResult::Yes {
        return;
    }
    let Some(path) = rfd::FileDialog::new()
        .set_title(lt_i18n::t("btn_export_settings"))
        .set_file_name("livetranslate-settings.json")
        .add_filter("JSON", &["json"])
        .save_file()
    else {
        return;
    };
    let body = super::export_settings_json(&state.settings);
    match std::fs::write(&path, body) {
        Ok(()) => {
            let msg = lt_i18n::t("export_done_msg").replace("{path}", &path.display().to_string());
            rfd::MessageDialog::new()
                .set_title(lt_i18n::t("export_done_title"))
                .set_description(&msg)
                .set_buttons(rfd::MessageButtons::Ok)
                .set_level(rfd::MessageLevel::Info)
                .show();
        }
        Err(e) => {
            tracing::error!("导出设置失败: {e}");
            rfd::MessageDialog::new()
                .set_title(lt_i18n::t("export_failed_title"))
                .set_description(e.to_string())
                .set_buttons(rfd::MessageButtons::Ok)
                .set_level(rfd::MessageLevel::Warning)
                .show();
        }
    }
}

/// 导入设置（原版 _import_settings：读取 → 确认 → 覆盖草稿 + 落盘）。
/// Rust 版：覆盖 AppState.settings 后即发 ApplySettings（egui 全帧重绘 +
/// 命令热应用，等效原版"重启生效"且即刻可见）。
fn import_settings(state: &mut AppState) {
    let Some(path) = rfd::FileDialog::new()
        .set_title(lt_i18n::t("btn_import_settings"))
        .add_filter("JSON", &["json"])
        .pick_file()
    else {
        return;
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            show_import_failed(&lt_i18n::t("import_invalid").replace("{error}", &e.to_string()));
            return;
        }
    };
    let imported = match import_settings_json(&text) {
        Ok(s) => s,
        Err(e) => {
            show_import_failed(&lt_i18n::t("import_invalid").replace("{error}", &e));
            return;
        }
    };
    let confirmed = rfd::MessageDialog::new()
        .set_title(lt_i18n::t("import_confirm_title"))
        .set_description(lt_i18n::t("import_confirm_msg"))
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_level(rfd::MessageLevel::Warning)
        .show();
    if confirmed != rfd::MessageDialogResult::Yes {
        return;
    }
    state.settings = imported;
    // 整体重放：VAD/目标语言/超时/转录开关热应用 + 落盘（原版 store.save 的等价路径）
    state.send_cmd(lt_proto::Cmd::ApplySettings(Box::new(state.settings.clone())));
    rfd::MessageDialog::new()
        .set_title(lt_i18n::t("import_confirm_title"))
        .set_description(lt_i18n::t("import_done_msg"))
        .set_buttons(rfd::MessageButtons::Ok)
        .set_level(rfd::MessageLevel::Info)
        .show();
}

fn show_import_failed(detail: &str) {
    tracing::warn!("导入设置失败: {detail}");
    rfd::MessageDialog::new()
        .set_title(lt_i18n::t("import_failed_title"))
        .set_description(detail)
        .set_buttons(rfd::MessageButtons::Ok)
        .set_level(rfd::MessageLevel::Warning)
        .show();
}
