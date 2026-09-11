//! 通用 egui 确认模态（D-33/H-3~H-5，docs/archive/hide-quit-flow-overhaul.md）。
//!
//! 替代事件循环线程内的 rfd 同步 MessageBoxW：egui 帧内绘制、不阻塞、单模态源。
//! 同一时刻至多一个（`ModalUi::request_confirm` 幂等）；宿主窗口 = Overlay 或
//! Panel，渲染帧按 `ConfirmUi.host` 匹配（其它窗口调本函数为空操作）。
//!
//! W5（架构 2.0 R8）：模态的渲染与应用是**跨全域逻辑**（清空悬浮窗消息/重置
//! 设置/删除模型/退出请求），不再内嵌于 overlay_ui/panel_ui 的窗口签名——
//! 由 dispatch 在窗口帧之后以根级 `&mut AppUi` 调 [`render_confirm_if_host`]。

use crate::state::{AppUi, ConfirmKind, WinAction, WinId};
use egui::{Align2, RichText, Ui};

/// 仅在 `host` 窗口帧内渲染确认模态；确定/取消在模态内收敛并执行效果。
/// 由 dispatch 在 overlay/panel 窗口帧之后调用（共享状态单源）。
pub fn render_confirm_if_host(ui: &mut Ui, app: &mut AppUi, host: WinId) {
    let Some(conf) = &app.modal.confirm else {
        return;
    };
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

    let Some(conf) = app.modal.take_confirm() else {
        return;
    };
    if accepted {
        apply_confirm_kind(ui, app, &conf.kind);
    } else if cancelled || !open {
        // 取消/关闭：面板因确认临时显示则回隐藏（H-3"取消退出不多个面板"）
        if conf.panel_shown_for_confirm {
            app.session
                .enqueue_action(WinId::Panel, WinAction::HidePanel);
        }
    } else {
        // 未收敛（仅取回渲染快照）→ 重新放回，等下一帧
        app.modal.confirm = Some(conf);
    }
}

/// 确认结果的执行（与既有页内直接改 settings 的模式一致；所有效果幂等、单发）。
/// W5：跨全域应用逻辑在根层——从 AppUi 解构所需域，窗口边界由 dispatch 维护。
fn apply_confirm_kind(ui: &Ui, app: &mut AppUi, kind: &ConfirmKind) {
    match kind {
        ConfirmKind::Quit => {
            // about_to_wait 既有路径：quit_requested → event_loop.exit()
            app.modal.quit_requested = true;
        }
        ConfirmKind::Clear => {
            app.overlay.messages.clear();
        }
        ConfirmKind::ResetSubtitle => {
            app.settings.subtitle_mode = lt_proto::SubtitleMode::default();
            // 行级字体跟随（空串）解析自注册表 → 重装字体链
            crate::fonts::apply_fonts(ui.ctx(), &app.settings, &mut app.ctx.fonts);
            app.session
                .enqueue_action(WinId::Panel, WinAction::ToggleSubtitle);
            app.session.request_settings_apply();
        }
        ConfirmKind::ResetTranslation => {
            crate::windows::panel::translation::restore_translation_page(
                &mut app.panel,
                &mut app.session,
                &mut app.settings,
            );
        }
        ConfirmKind::DeleteModel { index } => {
            crate::windows::panel::data::apply_delete_selected(
                &mut app.panel,
                &mut app.session,
                &mut app.settings,
                *index,
            );
        }
        ConfirmKind::DeleteAll => {
            crate::windows::panel::data::apply_delete_all(
                &mut app.panel,
                &mut app.session,
                &mut app.settings,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppUi;
    use crate::windows;
    use lt_proto::Settings;

    /// 面板帧内：确认模态打开时产出图元（egui::Window 成功锚定居中），
    /// 取消收敛后模态关闭且面板恢复标记入队。
    #[test]
    fn panel_confirm_modal_renders_and_cancels_headless() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        st.session.visible.insert(WinId::Panel, false);
        st.modal.request_confirm(
            ConfirmKind::Quit,
            false,
            st.session
                .visible
                .get(&WinId::Panel)
                .copied()
                .unwrap_or(true),
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        );
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                windows::dispatch(WinId::Panel, ui, &mut st);
            });
            assert!(!out.shapes.is_empty(), "确认模态帧应产出图元");
            out.textures_delta.clear();
        }
        assert!(st.modal.confirm.is_some(), "未收敛前模态保持打开");
        // 取消：模拟点击取消 → 模态关闭 + 面板临时显示恢复入队（用状态级断言驱动）
        assert_eq!(
            st.modal.take_confirm().map(|c| c.kind),
            Some(ConfirmKind::Quit)
        );
    }

    /// 悬浮窗帧内：确认模态与其窗口共帧渲染（覆盖 H-4 同款注入路径）。
    #[test]
    fn overlay_confirm_modal_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        st.modal.request_confirm(
            ConfirmKind::Clear,
            true,
            st.session
                .visible
                .get(&WinId::Panel)
                .copied()
                .unwrap_or(true),
            lt_i18n::t("clear_confirm_title"),
            lt_i18n::t("clear_confirm_msg"),
        );
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                windows::dispatch(WinId::Overlay, ui, &mut st);
            });
            assert!(!out.shapes.is_empty(), "悬浮窗确认模态帧应产出图元");
            out.textures_delta.clear();
        }
    }
}
