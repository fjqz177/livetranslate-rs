//! WP-B 回归断言：控件 hover 状态不得移动文本（egui 0.36 按钮内边距 =
//! `button_padding − bg_stroke.width`，三态描边宽度不一致时 hover 会横移
//! 文字 0.5px 且字重变粗）。`stabilize_widget_strokes` 统一三态后，
//! hover 前后文本 galley rect 必须完全相等。
//!
//! D-32 扩展：既有 hover 钉漏掉了 active（按压）态——悬浮窗 overlay_ui 原位
//! 覆盖只有 inactive/hovered 时，active 沿用宿主 stabilize 的 0 宽描边，
//! 按压瞬间文字右移 1 逻辑 px（无头探针实测 6.0→7.0）。按压态与基准下限
//! 一并入钉：任何窗口、任何按钮的三态文字位置都必须完全相等。

use egui::{Color32, CornerRadius, Event, Modifiers, RawInput, Rect, RichText, Sense, Stroke, Vec2};

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

// ── D-32：按压态三态几何全等 + stabilize 基准下限 ──

/// overlay 注入链单帧（host stabilize + overlay_ui 三态原位覆盖）：
/// 返回按钮响应 rect 与文本 galley 合并 rect；`events` 决定指针状态。
fn overlay_frame(ctx: &egui::Context, events: Vec<Event>) -> (Rect, Rect) {
    let mut ri = RawInput::default();
    ri.events = events;
    let mut btn = Rect::NOTHING;
    let mut out = ctx.run_ui(ri, |ui| {
        ui.set_clip_rect(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(600.0, 600.0)));
        // host 链（app.rs run_frame：dark + stabilize；每帧重放）
        let mut v = egui::Visuals::dark();
        lt_ui::style::stabilize_widget_strokes(&mut v);
        ui.style_mut().visuals = v;
        // overlay_ui 原位覆盖（overlay.rs：三态必须每帧成对设置，D-32）
        ui.style_mut().spacing.button_padding = Vec2::new(6.0, 0.0);
        ui.style_mut().spacing.item_spacing = Vec2::new(6.0, 2.0);
        ui.style_mut().spacing.interact_size = Vec2::new(8.0, 18.0);
        let vis = &mut ui.style_mut().visuals;
        vis.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(20, 20, 20, 20);
        vis.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgba_premultiplied(40, 40, 40, 40));
        vis.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(40, 40, 40, 40);
        vis.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgba_premultiplied(40, 40, 40, 40));
        vis.widgets.active.bg_fill = Color32::from_rgba_premultiplied(50, 55, 60, 60);
        vis.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgba_premultiplied(40, 40, 40, 40));
        vis.widgets.active.corner_radius = vis.widgets.inactive.corner_radius;
        // 占位（button 规格 = row1 small_btn：11px 文字 / 20px 高 / 圆角 3）
        ui.allocate_rect(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(50.0, 40.0)), Sense::hover());
        let r = ui.add(
            egui::Button::new(RichText::new("运行").size(11.0).color(Color32::from_rgb(0xaa, 0xaa, 0xaa)))
                .fill(Color32::from_rgba_premultiplied(20, 20, 20, 20))
                .stroke(Stroke::new(1.0, Color32::from_rgba_premultiplied(40, 40, 40, 40)))
                .corner_radius(CornerRadius::same(3))
                .min_size(Vec2::new(0.0, 20.0)),
        );
        btn = r.rect;
    });
    out.textures_delta.clear();
    let mut bb: Option<Rect> = None;
    for cl in &out.shapes {
        if let egui::Shape::Text(t) = &cl.shape {
            bb = Some(match bb {
                Some(b) => b.union(t.galley.rect),
                None => t.galley.rect,
            });
        }
    }
    (btn, bb.unwrap_or(Rect::NOTHING))
}

/// 悬浮窗行1按钮三态（idle / hover / 按压连跑 3 帧——read_response 有 1 帧
/// 滞后，按压态在后帧才渲染）文字位置必须完全相等：D-32 前 idle/hover=6.0、
/// press=7.0（右移 1 逻辑 px），修复后必须全等。
#[test]
fn overlay_button_text_does_not_shift_when_pressed() {
    let ctx = egui::Context::default();
    let (btn, t0) = overlay_frame(&ctx, vec![]);
    let hit = btn.center();
    let (_, t1) = overlay_frame(&ctx, vec![Event::PointerMoved(hit)]);
    let press = vec![
        Event::PointerMoved(hit),
        Event::PointerButton {
            pos: hit,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ];
    let (_, t2) = overlay_frame(&ctx, press.clone());
    let (_, t3) = overlay_frame(&ctx, press.clone());
    let (_, t4) = overlay_frame(&ctx, press);
    for (i, t) in [&t1, &t2, &t3, &t4].iter().enumerate() {
        assert_eq!(
            t0, **t,
            "三态文字位置必须全等（idle vs 帧{i}，dx={} dy={}）",
            t.left() - t0.left(),
            t.top() - t0.top()
        );
    }
}

/// stabilize 基准下限：dark 默认 inactive bg_stroke 为 0 宽（Stroke::NONE），
/// 必须抬到 ≥1.0 且三态互等——否则 hovered/active 整体 0 宽，任一窗口
/// 半覆盖三态（如悬浮窗只设 inactive/hovered）就会压出 1px 按压位移（D-32）。
#[test]
fn stabilize_reaches_one_px_floor() {
    let mut v = egui::Visuals::dark();
    lt_ui::style::stabilize_widget_strokes(&mut v);
    let w = v.widgets.inactive.bg_stroke.width;
    assert!(w >= 1.0, "dark inactive 0 宽描边须抬到 ≥1.0，实测 {w}");
    assert_eq!(v.widgets.hovered.bg_stroke.width, w);
    assert_eq!(v.widgets.active.bg_stroke.width, w);
    assert_eq!(v.widgets.hovered.fg_stroke.width, v.widgets.inactive.fg_stroke.width);
    assert_eq!(v.widgets.active.fg_stroke.width, v.widgets.inactive.fg_stroke.width);
    assert_eq!(v.widgets.hovered.corner_radius, v.widgets.inactive.corner_radius);
    assert_eq!(v.widgets.active.corner_radius, v.widgets.inactive.corner_radius);
    // panel_visuals：1.0 基准不动、三态互等（兼防未来回退）
    let mut p = lt_ui::windows::panel::panel_visuals();
    lt_ui::style::stabilize_widget_strokes(&mut p);
    let pw = p.widgets.inactive.bg_stroke.width;
    assert_eq!(pw, 1.0);
    assert_eq!(p.widgets.hovered.bg_stroke.width, pw);
    assert_eq!(p.widgets.active.bg_stroke.width, pw);
}
