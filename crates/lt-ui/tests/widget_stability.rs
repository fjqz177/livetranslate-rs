//! WP-B 回归断言：控件 hover 状态不得移动文本（egui 0.36 按钮内边距 =
//! `button_padding − bg_stroke.width`，三态描边宽度不一致时 hover 会横移
//! 文字 0.5px 且字重变粗）。`stabilize_widget_strokes` 统一三态后，
//! hover 前后文本 galley rect 必须完全相等。

use egui::{Event, RawInput, RichText, Vec2};

/// 渲染一帧并返回全部文本 galley 的合并 rect（hover_pos 决定指针位置）
fn text_bbox(ctx: &egui::Context, hover_pos: egui::Pos2, add: &dyn Fn(&mut egui::Ui)) -> egui::Rect {
    let mut ri = RawInput::default();
    ri.events.push(Event::PointerMoved(hover_pos));
    let mut out = ctx.run_ui(ri, |ui| {
        ui.set_clip_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 600.0)));
        // 占位保证按钮不在指针初始位置上
        ui.allocate_rect(
            egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(50.0, 40.0)),
            egui::Sense::hover(),
        );
        add(ui);
    });
    // 无渲染器 → 显式消费纹理增量（epaint debug 断言要求）
    out.textures_delta.clear();
    let mut bb: Option<egui::Rect> = None;
    for cl in &out.shapes {
        if let egui::Shape::Text(t) = &cl.shape {
            bb = Some(match bb {
                Some(b) => b.union(t.galley.rect),
                None => t.galley.rect,
            });
        }
    }
    bb.unwrap_or(egui::Rect::NOTHING)
}

/// dark 主题 + stabilize（悬浮窗/日志窗/向导窗/基准窗的注入路径）
fn dark_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    let mut v = egui::Visuals::dark();
    lt_ui::style::stabilize_widget_strokes(&mut v);
    ctx.set_visuals(v);
    ctx
}

#[test]
fn button_text_does_not_shift_on_hover() {
    let ctx = dark_ctx();
    let draw = |ui: &mut egui::Ui| {
        ui.add(egui::Button::new(RichText::new("运行").size(11.0)).min_size(Vec2::new(0.0, 20.0)));
    };
    let t0 = text_bbox(&ctx, egui::pos2(10.0, 10.0), &draw);
    let t1 = text_bbox(&ctx, egui::pos2(60.0, 30.0), &draw);
    assert_eq!(t0, t1, "Button hover 前后文本 rect 必须一致（dx={} dy={}）", t1.left() - t0.left(), t1.top() - t0.top());
}

#[test]
fn combo_box_text_does_not_shift_on_hover() {
    let ctx = dark_ctx();
    let draw = |ui: &mut egui::Ui| {
        egui::ComboBox::from_id_salt("probe")
            .width(120.0)
            .selected_text("hunyuan-mt-chimera-7b")
            .show_ui(ui, |ui| {
                let _ = ui.selectable_label(false, "hunyuan-mt-chimera-7b");
            });
    };
    let t0 = text_bbox(&ctx, egui::pos2(10.0, 10.0), &draw);
    let t1 = text_bbox(&ctx, egui::pos2(60.0, 30.0), &draw);
    assert_eq!(t0, t1, "ComboBox hover 前后文本 rect 必须一致");
}

#[test]
fn panel_visuals_after_stabilize_have_uniform_strokes() {
    let mut v = lt_ui::windows::panel::panel_visuals();
    lt_ui::style::stabilize_widget_strokes(&mut v);
    let w = v.widgets.inactive.bg_stroke.width;
    assert_eq!(v.widgets.hovered.bg_stroke.width, w);
    assert_eq!(v.widgets.active.bg_stroke.width, w);
    let fg = v.widgets.inactive.fg_stroke.width;
    assert_eq!(v.widgets.hovered.fg_stroke.width, fg);
    assert_eq!(v.widgets.active.fg_stroke.width, fg);
}

#[test]
fn plain_dark_theme_is_stabilized() {
    // 未 stabilize 的 dark 默认应当三态不一致（此断言防 egui 升级后误判前提）
    let plain = egui::Visuals::dark();
    let mut v = egui::Visuals::dark();
    lt_ui::style::stabilize_widget_strokes(&mut v);
    assert_eq!(v.widgets.hovered.bg_stroke.width, v.widgets.inactive.bg_stroke.width);
    assert_eq!(v.widgets.active.fg_stroke.width, v.widgets.inactive.fg_stroke.width);
    // corner_radius 同步拉齐
    assert_eq!(v.widgets.hovered.corner_radius, v.widgets.inactive.corner_radius);
    let _ = plain;
}
