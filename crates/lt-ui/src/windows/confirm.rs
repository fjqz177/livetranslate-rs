//! 专用确认窗（D-87，docs/quit-flow-redesign.md）。
//!
//! 六种确认（退出/清空/恢复字幕默认/恢复翻译默认/删模型/删全部）的唯一载体：
//! 独立原生小窗（WinId::Confirm，无边框自绘），替代旧「借画布」宿主模态——
//! 悬浮窗/面板不再被拉来当画布（全隐藏时凭空弹面板的根因），确认永不改变
//! 任何窗口可见性。内容纯 egui 绘制：暗色半透明材质由宿主 LAYERED+LWA_ALPHA
//! 承担（比悬浮窗略实——对话框要读要判），圆角由宿主 SetWindowRgn 裁区
//! （半径 = [`RADIUS`]，与 app.rs 同源）。G-14 合规：帧内绘制、零同步阻塞。

use crate::state::{AppUi, ConfirmKind, WinAction, WinId};
use egui::{Color32, CornerRadius, RichText, Stroke, Ui};

/// 窗口圆角（逻辑 px；宿主 SetWindowRgn 裁区同源此值——app.rs CONFIRM_CORNER_RADIUS）
pub(crate) const RADIUS: f32 = 12.0;
/// 体填充圆角：**必须小于** RADIUS（角更方）——Win10 的 SetWindowRgn 是
/// 1-bit 硬裁剪无抗锯齿，裁刀若与 egui 画角同半径会切过 AA 渐变带产生
/// 马赛克毛边；填充分画小后裁刀切在纯色内部（几何余量 ≈ 0.414×(12-6) ≈
/// 2.5px），毛边彻底消失，可见圆角仍为 12
const FILL_RADIUS: f32 = 6.0;
/// 边缘 rim：沿裁区曲线内侧描 2px，盖住硬裁剪的阶梯锯齿
const RIM_STROKE: f32 = 2.0;

// 材质配色（应用暗色语义：深底 + 灰字；危险操作红系，全部复用既有调色语义）
const BODY_FILL: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x24);
const BODY_STROKE: Color32 = Color32::from_rgb(0x41, 0x46, 0x52);
/// 标题行下分隔线
const SEPARATOR: Color32 = Color32::from_rgb(0x2E, 0x32, 0x3C);
const TEXT_TITLE: Color32 = Color32::from_rgb(0xF2, 0xF2, 0xF2);
const TEXT_MSG: Color32 = Color32::from_rgb(0xC9, 0xCE, 0xD8);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9E, 0xA4, 0xB0);
const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF5);
/// 危险语义（退出/删除系主钮；与悬浮窗红底退出钮同色系）
const DANGER_FILL: Color32 = Color32::from_rgb(0x9A, 0x33, 0x33);
const DANGER_STROKE: Color32 = Color32::from_rgb(0xC0, 0x5A, 0x5A);
const DANGER_HOVER: Color32 = Color32::from_rgb(0xAF, 0x3E, 0x3E);
const DANGER_DOT: Color32 = Color32::from_rgb(0xE0, 0x56, 0x56);
/// 中性主钮（恢复默认/清空）
const PRIMARY_FILL: Color32 = Color32::from_rgb(0x40, 0x45, 0x50);
const PRIMARY_STROKE: Color32 = Color32::from_rgb(0x5A, 0x60, 0x6D);
const PRIMARY_HOVER: Color32 = Color32::from_rgb(0x4E, 0x54, 0x60);
const GHOST_HOVER: Color32 = Color32::from_rgb(0x30, 0x34, 0x3D);

/// 确认窗内容（仅 WinId::Confirm 帧内调用；收敛副作用：确定 → [`ConfirmKind`]
/// 效果 + HideConfirm；取消/X/ESC → HideConfirm；未收敛 → 状态放回等下一帧）
pub fn confirm_ui(ui: &mut Ui, app: &mut AppUi) {
    let Some(conf) = app.modal.confirm.clone() else {
        return;
    };

    // 入场淡入 120ms（reduce_motion 直切；request_repaint 有限跟帧，G-1 安全）
    let t = if app.panel.state.reduce_motion {
        1.0
    } else {
        let t = (conf.opened_at.elapsed().as_secs_f32() / 0.12).min(1.0);
        if t < 1.0 {
            ui.ctx().request_repaint();
        }
        t
    };
    ui.set_opacity(t);

    let danger = matches!(
        conf.kind,
        ConfirmKind::Quit | ConfirmKind::DeleteAll | ConfirmKind::DeleteModel { .. }
    );

    // ── 材质：深底 + 边缘 rim——显式矩形排版，杜绝流式缩进漂移 ──
    // （顶缘高光线经走查反馈删除；圆角抗锯齿方案见 FILL_RADIUS 注）
    let full = ui.max_rect();
    let painter = ui.painter().clone();
    painter.rect_filled(full, CornerRadius::same(FILL_RADIUS as u8), BODY_FILL);
    // rim 沿可见圆角（=裁区曲线）内侧描边，硬裁剪的阶梯被 rim 盖住
    painter.rect_stroke(
        full,
        CornerRadius::same(RADIUS as u8),
        Stroke::new(RIM_STROKE, BODY_STROKE),
        egui::StrokeKind::Inside,
    );

    // 行几何（逻辑 px；左右统一 20px 内容边距）
    let left = full.left() + 20.0;
    let right = full.right() - 20.0;
    let btn_h = 32.0;
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(left, full.top() + 14.0),
        egui::pos2(right, full.top() + 40.0),
    );
    let sep_y = title_rect.bottom() + 8.0;
    let msg_rect = egui::Rect::from_min_max(
        egui::pos2(left, sep_y + 10.0),
        egui::pos2(right, full.bottom() - 14.0 - btn_h),
    );
    let btn_rect = egui::Rect::from_min_max(
        egui::pos2(left, full.bottom() - 14.0 - btn_h),
        egui::pos2(right, full.bottom() - 14.0),
    );

    // ── 标题行：语义色圆点 + 标题 + 关闭钮 ──
    let dot_c = if danger { DANGER_DOT } else { TEXT_DIM };
    let mut title_ui = ui.new_child(egui::UiBuilder::new().max_rect(title_rect));
    title_ui.horizontal(|ui| {
        let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 24.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(dot_rect.left_center() + egui::vec2(3.0, 0.0), 4.0, dot_c);
        ui.label(
            RichText::new(&conf.title)
                .strong()
                .size(15.5)
                .color(TEXT_TITLE),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, "✕") {
                set_closed(app);
            }
        });
    });
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(left, sep_y), egui::pos2(right, sep_y + 1.0)),
        CornerRadius::same(1),
        SEPARATOR,
    );

    // ── 正文（独立纵向 child：宽度即行宽，换行与左对齐恒定）──
    let mut msg_ui = ui.new_child(egui::UiBuilder::new().max_rect(msg_rect));
    msg_ui.label(RichText::new(&conf.msg).size(13.5).color(TEXT_MSG));

    // ── 按钮行（右下：[取消][主钮=后果动词]）──
    let mut accepted = false;
    let mut cancelled = false;
    // 键盘收敛（D-87 补齐：Enter=主钮 / ESC=取消）
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if enter {
        accepted = true;
    }
    if esc {
        cancelled = true;
    }
    let mut btn_ui = ui.new_child(egui::UiBuilder::new().max_rect(btn_rect));
    btn_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let primary = primary_button(ui, &primary_label_of(&conf.kind), danger);
        ui.add_space(10.0);
        let cancel = ghost_button(ui, &lt_i18n::t("subwin_cancel"), 76.0);
        if primary {
            accepted = true;
        }
        if cancel {
            cancelled = true;
        }
    });

    let Some(conf) = app.modal.take_confirm() else {
        return;
    };
    if accepted {
        apply_confirm_kind(ui, app, &conf.kind);
        set_closed(app);
    } else if cancelled {
        set_closed(app);
    } else {
        // 未收敛（仅取回渲染快照）→ 重新放回，等下一帧
        app.modal.confirm = Some(conf);
    }
}

/// 收敛为关闭：经意图通道由宿主隐藏确认窗（UI 不直触窗口）
fn set_closed(app: &mut AppUi) {
    app.session
        .enqueue_action(WinId::Confirm, WinAction::HideConfirm);
}

/// 属主遮罩（D-87 真模态）：确认打开期间在属主窗口（悬浮窗/面板）帧尾盖
/// 微暗吞点击层——对齐原生对话框「打开时禁用属主」语义；确认窗关闭自动
/// 消失。在 Overlay/Panel 分发臂主 UI 之后调用（后画者在上，吞掉点击）。
pub fn owner_shield_if_open(ui: &mut Ui, app: &AppUi) {
    if app.modal.confirm.is_some() {
        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, Color32::from_black_alpha(90));
        ui.allocate_rect(full, egui::Sense::click());
    }
}

/// 主钮文案 = 后果动词（不写「确定」——眼睛不用上移核对；i18n 键集双 yaml 同步）
fn primary_label_of(kind: &ConfirmKind) -> String {
    let key = match kind {
        ConfirmKind::Quit => "quit",
        ConfirmKind::Clear => "clear",
        ConfirmKind::ResetSubtitle | ConfirmKind::ResetTranslation => "btn_restore_default",
        ConfirmKind::DeleteModel { .. } | ConfirmKind::DeleteAll => "confirm_delete",
    };
    lt_i18n::t(key)
}

fn primary_button(ui: &mut Ui, label: &str, danger: bool) -> bool {
    let (fill, hover, stroke) = if danger {
        (DANGER_FILL, DANGER_HOVER, DANGER_STROKE)
    } else {
        (PRIMARY_FILL, PRIMARY_HOVER, PRIMARY_STROKE)
    };
    ui.visuals_mut().widgets.hovered.bg_fill = hover;
    ui.visuals_mut().widgets.active.bg_fill = hover;
    ui.add(
        egui::Button::new(RichText::new(label).size(14.0).color(TEXT_BRIGHT))
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .corner_radius(CornerRadius::same(6))
            .min_size(egui::vec2(96.0, 32.0)),
    )
    .clicked()
}

fn ghost_button(ui: &mut Ui, label: &str, w: f32) -> bool {
    ui.visuals_mut().widgets.hovered.bg_fill = GHOST_HOVER;
    ui.visuals_mut().widgets.active.bg_fill = GHOST_HOVER;
    ui.add(
        egui::Button::new(RichText::new(label).size(14.0).color(TEXT_DIM))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, BODY_STROKE))
            .corner_radius(CornerRadius::same(6))
            .min_size(egui::vec2(w, 32.0)),
    )
    .clicked()
}

/// 标题行关闭钮（无描边幽灵款；hover 浮出底色）
fn icon_button(ui: &mut Ui, glyph: &str) -> bool {
    ui.visuals_mut().widgets.hovered.bg_fill = GHOST_HOVER;
    ui.visuals_mut().widgets.active.bg_fill = GHOST_HOVER;
    ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    ui.add(
        egui::Button::new(RichText::new(glyph).size(14.0).color(TEXT_DIM))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(5))
            .min_size(egui::vec2(26.0, 26.0)),
    )
    .clicked()
}

/// 确认结果的执行（与既有页内直接改 settings 的模式一致；所有效果幂等、单发）。
/// 跨全域逻辑在根层——从 AppUi 解构所需域（W5/R8 模式延续）。
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
        ConfirmKind::DeleteModel { path } => {
            // D-87 身份键：按 path 定位；查不到 = 列表已变 → 静默取消（fail-safe）
            crate::windows::panel::data::apply_delete_selected(
                &mut app.panel,
                &mut app.session,
                &mut app.settings,
                path,
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

    fn request_quit(st: &mut AppUi) {
        st.modal.request_confirm(
            ConfirmKind::Quit,
            lt_i18n::t("quit_confirm_title"),
            lt_i18n::t("quit_confirm_msg"),
        );
    }

    fn key_input(key: egui::Key) -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        input
    }

    /// 确认窗帧：确认打开时产出图元（暗色自绘体），未收敛前状态保持打开
    #[test]
    fn confirm_window_renders_headless() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        request_quit(&mut st);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                windows::dispatch(WinId::Confirm, ui, &mut st);
            });
            assert!(!out.shapes.is_empty(), "确认窗帧应产出图元");
            out.textures_delta.clear();
        }
        assert!(st.modal.confirm.is_some(), "未收敛前状态保持打开");
    }

    /// Enter = 主钮：Quit 确认收敛 → quit_requested + HideConfirm 入队
    #[test]
    fn confirm_enter_accepts_quit() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        request_quit(&mut st);
        let mut out = ctx.run_ui(key_input(egui::Key::Enter), |ui| {
            windows::dispatch(WinId::Confirm, ui, &mut st);
        });
        out.textures_delta.clear();
        assert!(st.modal.quit_requested, "Enter 应收敛为退出请求");
        assert!(st.modal.confirm.is_none(), "收敛后状态应关闭");
        assert!(
            st.session
                .drain_actions()
                .iter()
                .any(|(w, a)| *w == WinId::Confirm && matches!(a, WinAction::HideConfirm)),
            "确定后应入队隐藏确认窗"
        );
    }

    /// ESC = 取消：无效果，仅隐藏（Quit 不置 quit_requested）
    #[test]
    fn confirm_esc_cancels_without_effects() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        request_quit(&mut st);
        let mut out = ctx.run_ui(key_input(egui::Key::Escape), |ui| {
            windows::dispatch(WinId::Confirm, ui, &mut st);
        });
        out.textures_delta.clear();
        assert!(!st.modal.quit_requested, "取消不得触发退出");
        assert!(st.modal.confirm.is_none(), "取消后状态应关闭");
        assert!(
            st.session
                .drain_actions()
                .iter()
                .any(|(w, a)| *w == WinId::Confirm && matches!(a, WinAction::HideConfirm)),
            "取消后应入队隐藏确认窗"
        );
    }

    /// 去宿主化回归钉（D-87）：悬浮窗/面板帧不再消费确认状态（旧借画布路径已删）
    #[test]
    fn confirm_state_untouched_by_legacy_host_frames() {
        let ctx = egui::Context::default();
        let mut st = AppUi::new(Settings::default());
        st.panel.state.reduce_motion = true;
        request_quit(&mut st);
        for win in [WinId::Overlay, WinId::Panel] {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                windows::dispatch(win, ui, &mut st);
            });
            out.textures_delta.clear();
        }
        assert!(st.modal.confirm.is_some(), "旧宿主帧不得取走/收敛确认状态");
        assert!(
            !st.session
                .drain_actions()
                .iter()
                .any(|(_, a)| matches!(a, WinAction::HideConfirm)),
            "旧宿主帧不得入队确认窗动作"
        );
    }
}
