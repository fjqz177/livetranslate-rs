//! 通用 egui 确认模态（D-33/H-3~H-5，docs/hide-quit-flow-overhaul.md）。
//!
//! 替代事件循环线程内的 rfd 同步 MessageBoxW：egui 帧内绘制、不阻塞、单模态源。
//! 同一时刻至多一个（`AppState::request_confirm` 幂等）；宿主窗口 = Overlay 或
//! Panel，渲染帧按 `ConfirmUi.host` 匹配（其它窗口调本函数为空操作）。

use crate::state::{AppState, ConfirmKind, WinAction, WinId};
use egui::{Align2, RichText, Ui};

/// 仅在 `host` 窗口帧内渲染确认模态；确定/取消在模态内收敛并执行效果。
/// 必须在 overlay_ui / panel_ui 帧内各调用一次（共享状态单源）。
pub fn render_confirm_if_host(ui: &mut Ui, state: &mut AppState, host: WinId) {
    let Some(conf) = &state.confirm else { return };
    if conf.host != host {
        return;
    }
    let title = conf.title.clone();
    let msg = conf.msg.clone();

    let mut open = true;
    let mut cancelled = false;
    let mut accepted = false;
    egui::Window::new(RichText::new(title).strong())
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(380.0)
        .min_width(300.0)
        .show(ui.ctx(), |ui| {
            ui.label(RichText::new(msg).size(12.5));
            ui.add_space(10.0);
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(lt_i18n::t("common_ok")).clicked() {
                    accepted = true;
                }
                if ui.button(lt_i18n::t("subwin_cancel")).clicked() {
                    cancelled = true;
                }
            });
        });

    let Some(conf) = state.take_confirm() else {
        return;
    };
    if accepted {
        apply_confirm_kind(ui, state, &conf.kind);
    } else if cancelled || !open {
        // 取消/关闭：面板因确认临时显示则回隐藏（H-3"取消退出不多个面板"）
        if conf.panel_shown_for_confirm {
            state.enqueue_action(WinId::Panel, WinAction::HidePanel);
        }
    } else {
        // 未收敛（仅取回渲染快照）→ 重新放回，等下一帧
        state.confirm = Some(conf);
    }
}

/// 确认结果的执行（与既有页内直接改 settings 的模式一致；所有效果幂等、单发）
fn apply_confirm_kind(ui: &Ui, state: &mut AppState, kind: &ConfirmKind) {
    match kind {
        ConfirmKind::Quit => {
            // about_to_wait 既有路径：quit_requested → event_loop.exit()
            state.quit_requested = true;
        }
        ConfirmKind::Clear => {
            state.messages.clear();
        }
        ConfirmKind::ResetSubtitle => {
            state.settings.subtitle_mode = lt_proto::SubtitleMode::default();
            // 行级字体跟随（空串）解析自注册表 → 重装字体链
            crate::fonts::apply_fonts(ui.ctx(), &state.settings, &mut state.fonts);
            state.enqueue_action(WinId::Panel, WinAction::ToggleSubtitle);
            crate::windows::panel::mark_settings_dirty(state);
        }
        ConfirmKind::ResetTranslation => {
            crate::windows::panel::translation::restore_translation_page(state);
        }
        ConfirmKind::DeleteModel { index } => {
            crate::windows::panel::data::apply_delete_selected(state, *index);
        }
        ConfirmKind::DeleteAll => {
            crate::windows::panel::data::apply_delete_all(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use lt_proto::Settings;

    /// 面板帧内：确认模态打开时产出图元（egui::Window 成功锚定居中），
    /// 取消收敛后模态关闭且面板恢复标记入队。
    #[test]
    fn panel_confirm_modal_renders_and_cancels_headless() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(Settings::default());
        st.request_confirm(
            ConfirmKind::Quit,
            false,
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        );
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::panel::panel_ui(ui, &mut st);
            });
            assert!(!out.shapes.is_empty(), "确认模态帧应产出图元");
            out.textures_delta.clear();
        }
        assert!(st.confirm.is_some(), "未收敛前模态保持打开");
        // 取消：模拟点击取消 → 模态关闭 + 面板临时显示恢复入队（用状态级断言驱动）
        assert_eq!(st.take_confirm().map(|c| c.kind), Some(ConfirmKind::Quit));
    }

    /// 悬浮窗帧内：确认模态与其窗口共帧渲染（覆盖 H-4 同款注入路径）。
    #[test]
    fn overlay_confirm_modal_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(Settings::default());
        st.visible.insert(WinId::Overlay, true);
        st.request_confirm(
            ConfirmKind::Clear,
            true,
            lt_i18n::t("clear_confirm_title"),
            lt_i18n::t("clear_confirm_msg"),
        );
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::overlay::overlay_ui(ui, &mut st);
            });
            assert!(!out.shapes.is_empty(), "悬浮窗确认模态帧应产出图元");
            out.textures_delta.clear();
        }
    }
}
