//! 悬浮窗全量复刻（对照原版 subtitle_overlay.py）：
//!
//! - DragHandle：行1 = 拖动标题 + 7 按钮（隐藏/字幕/暂停/清空/紧凑/设置/退出）；
//!   行2a = 4 复选（穿透/置顶/自动滚动/任务栏）；行2b = 模型/源语言/目标语言下拉。
//!   高度 62（full）/ 24（compact）；行2 与字幕/清空按钮在紧凑模式隐藏。
//! - MonitorBar：MIC/RMS/VAD 三条电平（×500 缩放，原版 update_audio）+ 统计行
//!   （设备 | CPU 进程% RAM 进程MB GPU N/A | ASR n TL n Tok x (p↑c↓) ¥cost）。
//! - 消息流：头部富文本（时间戳/语言/原文/ASR 耗时）+ 译文行（流式节流 50ms、
//!   TL 耗时、同语言占位）；右键菜单（复制×3/导出×3/清空）；50 条上限。
//! - 自动滚动、右下角尺寸手柄、紧凑模式 200ms 高度动画。
//!
//! 已知偏差：窗口整体不透明度（原版 setWindowOpacity）以"绘制期 alpha 乘算"实现
//! （winit+wgpu 无安全窗口级透明度 API，视觉等价）；按钮/下拉视觉按 egui 控件
//! 近似还原 Qt 配色，非逐像素。

use crate::state::{
    ConfirmKind, ModalUi, OverlayMessage, OverlayMode, OverlayUi, SessionView, Settings, UiContext,
    WinAction, WinId,
};
use crate::style::{self, parse_color};
use egui::{
    Align2, Button, Color32, ComboBox, CornerRadius, FontId, RichText, ScrollArea, Sense, Stroke,
    Ui, Vec2,
};

/// pt → 逻辑 px（Qt 1pt ≈ 96/72 px）
fn pt(size: u32) -> f32 {
    size as f32 * 4.0 / 3.0
}

/// 整窗不透明度已由宿主提升为 LWA_ALPHA（见 app.rs apply_layered —— 等价
/// setWindowOpacity 作用到整窗）；控件级不再乘 window_opacity，保留恒等以便
/// 调用点签名不动（历史遗留 opa_pct 参数）。
fn opa(c: Color32, _opacity_pct: u32) -> Color32 {
    c
}

/// 原版字面色
const BTN_TEXT: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);
const BTN_FILL: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
const BTN_STROKE: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
/// 按钮/下拉 hover 底（原版 _BTN_CSS hover：rgba(255,255,255,40)）
const BTN_HOVER_FILL: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
/// 按钮按压底（D-32：按压反馈仅底色加深，几何/描边/字重三态全等）
const BTN_PRESSED_FILL: Color32 = Color32::from_rgba_premultiplied(50, 55, 60, 60);
const QUIT_FILL: Color32 = Color32::from_rgba_premultiplied(31, 9, 9, 40);
const QUIT_STROKE: Color32 = Color32::from_rgba_premultiplied(63, 19, 19, 80);
const PAUSED_FILL: Color32 = Color32::from_rgba_premultiplied(43, 35, 12, 50);
const PAUSED_TEXT: Color32 = Color32::from_rgb(0xdd, 0xdd, 0xbb);
const SUBTITLE_ON_FILL: Color32 = Color32::from_rgba_premultiplied(13, 28, 13, 40);
const SUBTITLE_ON_STROKE: Color32 = Color32::from_rgba_premultiplied(25, 56, 25, 80);
/// 统计行分隔线（原版 #555/#666）
const SEP: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
/// 统计行数值底色（原版 stats_label 基色 #888，数值继承；仅标签着彩色）
const STATS_VAL: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);

/// 电平条配色（原版 _BAR_CSS_TPL）
const BAR_MIC: Color32 = Color32::from_rgb(0xc5, 0x86, 0xc0);
const BAR_RMS: Color32 = Color32::from_rgb(0x4e, 0xc9, 0xb0);
const BAR_VAD: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xaa);
const BAR_SLOT: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);
const BAR_STROKE: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);

// ── 入口 ──

pub fn overlay_ui(
    ui: &mut Ui,
    overlay: &mut OverlayUi,
    session: &mut SessionView,
    settings: &mut Settings,
    modal: &mut ModalUi,
    ctx: &mut UiContext,
) {
    let st = &settings.style;
    let opa_pct = st.window_opacity;
    let compact = overlay.state.mode == OverlayMode::Compact;
    // 背景画成不透明色：整窗 alpha 由宿主 LWA_ALPHA（bg_opacity × window_opacity
    // 的单层等价）承担——半透明填充在键控品红清除区上会混出色偏，必须不透明
    let bg = parse_color(&st.bg_color, Color32::from_rgb(15, 15, 25));
    let radius = st.border_radius as f32;

    // 动画驱动：进行中则每帧请求窗口高度调整 + 重绘
    if let Some(h) = overlay
        .state
        .anim
        .and_then(|a| a.current(std::time::Instant::now()))
    {
        session.enqueue_action(WinId::Overlay, WinAction::SetHeight(h));
        ui.ctx().request_repaint();
    }

    egui::Frame::NONE
        .fill(bg)
        .corner_radius(CornerRadius::same(radius as u8))
        .inner_margin(4.0)
        .show(ui, |ui| {
            // 控件规格对齐原版紧凑 Qt 控件（按钮 padding 0-6 / 行距 2 / 下拉高 18；
            // 下拉、复选底色走 white20 透明 + white40 描边，即原版 _COMBO_CSS）
            ui.spacing_mut().button_padding = Vec2::new(6.0, 0.0);
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 2.0);
            ui.spacing_mut().interact_size = Vec2::new(8.0, 18.0);
            // 三态必须每帧成对覆盖（D-32）：按钮文字位置 = 内边距 − 该状态
            // bg_stroke.width（egui button_style），只设 inactive/hovered 会让
            // active 沿用宿主 stabilize 的 0 宽描边 → 按压时文字右移 1 逻辑 px。
            // 几何/描边宽/字重/圆角三态全等，按压反馈 = 仅底色加深。
            let vis = &mut ui.style_mut().visuals;
            vis.widgets.inactive.bg_fill = BTN_FILL;
            vis.widgets.inactive.bg_stroke = Stroke::new(1.0, BTN_STROKE);
            vis.widgets.hovered.bg_fill = BTN_HOVER_FILL;
            vis.widgets.hovered.bg_stroke = Stroke::new(1.0, BTN_STROKE);
            vis.widgets.active.bg_fill = BTN_PRESSED_FILL;
            vis.widgets.active.bg_stroke = Stroke::new(1.0, BTN_STROKE);
            vis.widgets.active.corner_radius = vis.widgets.inactive.corner_radius;

            drag_handle(ui, overlay, session, settings, modal, compact, opa_pct);
            if !compact {
                monitor_bar(ui, overlay, settings, opa_pct);
            }
            messages_area(ui, overlay, settings, ctx, compact, opa_pct);
            resize_grip(ui, session);
        });
}

// ── DragHandle（原版 DragHandle） ──

fn drag_handle(ui: &mut Ui, overlay: &mut OverlayUi, session: &mut SessionView, settings: &mut Settings, modal: &mut ModalUi, compact: bool, opa_pct: u32) {
    // 原版 DragHandle 是 QWidget 子类且未设 WA_StyledBackground/paintEvent，
    // 其 QSS 背景（含 apply_style 的 header_color）从未被渲染——头部区域即
    // 容器黑玻璃贯穿。header_color/header_opacity 字段与样式页控件 1:1 保留，
    // 仅绘制不消费（对齐原版实际行为，D-17）。
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            row1(ui, overlay, session, settings, modal, compact, opa_pct);
            if !compact {
                row2_checks(ui, overlay, session);
                row2_combos(ui, session, settings);
            }
        });
}

/// 行1：拖动标题 + 操作按钮（高度 24；按钮顺序=原版 row1：
/// 隐藏/字幕/启停/清除/完整/设置/退出）
fn row1(ui: &mut Ui, overlay: &mut OverlayUi, session: &mut SessionView, settings: &mut Settings, modal: &mut ModalUi, compact: bool, opa_pct: u32) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        // 拖动区：标题文本 + 空白拉伸
        let (drag_rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - button_reserved(compact), 20.0),
            Sense::click_and_drag(),
        );
        ui.painter().text(
            drag_rect.left_center() + Vec2::new(2.0, 0.0),
            Align2::LEFT_CENTER,
            "\u{2630} LiveTranslate",
            FontId::monospace(pt(9)),
            BTN_TEXT,
        );
        // W5/D-71：弃 winit drag_window（标题栏模态循环在 LAYERED+TRANSPARENT
        // 轮询窗上 0 位移/挂死）——egui 报起止，宿主 SetCapture 绝对跟踪
        //（任意按键可用；D-37 字幕窗同法）
        let drag_resp = ui.interact(drag_rect, ui.id().with("ov_drag"), Sense::click_and_drag());
        if drag_resp.drag_started() {
            session.enqueue_action(WinId::Overlay, WinAction::OverlayDragStart);
        }
        if drag_resp.drag_stopped() && overlay.state.dragging {
            session.enqueue_action(WinId::Overlay, WinAction::OverlayDragEnd);
        }

        // 隐藏（原版 hide_btn：隐藏悬浮窗，托盘"显示悬浮窗"可恢复 + 首次气泡提示）
        if ui
            .add(small_btn(
                lt_i18n::t("hide"),
                BTN_FILL,
                BTN_STROKE,
                BTN_TEXT,
                opa_pct,
            ))
            .clicked()
        {
            session.enqueue_action(WinId::Overlay, WinAction::Hide);
        }

        // 字幕按钮（紧凑模式隐藏；开启时绿底，原版 set_subtitle_checked）
        if !compact {
            let on = settings.subtitle_mode.enabled;
            // WP-1：手势提示悬停文案（首次开启另有 toast，见 app.rs ToggleSubtitle）
            let sub = ui
                .add(small_btn(
                    lt_i18n::t("subtitle"),
                    if on { SUBTITLE_ON_FILL } else { BTN_FILL },
                    if on { SUBTITLE_ON_STROKE } else { BTN_STROKE },
                    BTN_TEXT,
                    opa_pct,
                ))
                .on_hover_text(lt_i18n::t("subwin_btn_hint"));
            if sub.clicked() {
                // 原版 subtitle_toggled → 切换字幕窗可见性
                let vis = !settings.subtitle_mode.enabled;
                settings.subtitle_mode.enabled = vis;
                session.enqueue_action(WinId::Overlay, WinAction::ToggleSubtitle);
            }
        }

        // 启停按钮（运行=普通样式 t("running")；暂停=琥珀色 t("paused")）
        let running = session.running;
        let (label, fill, stroke, text) = if running {
            (lt_i18n::t("running"), BTN_FILL, BTN_STROKE, BTN_TEXT)
        } else {
            (lt_i18n::t("paused"), PAUSED_FILL, BTN_STROKE, PAUSED_TEXT)
        };
        let start_stop = ui.add(small_btn(label, fill, stroke, text, opa_pct));
        if start_stop.clicked() {
            let cmd = if running {
                lt_proto::Cmd::Pause
            } else {
                lt_proto::Cmd::Resume
            };
            session.send_cmd(cmd);
            session.running = !running;
        }

        // 清空（紧凑模式隐藏；转写自动落盘时免确认，否则确认一次防误触）。
        // D-33/H-5：确认改 egui 模态（原位 rfd 同步框会与置顶悬浮窗叠置不可见+阻塞）
        if !compact
            && ui
                .add(small_btn(
                    lt_i18n::t("clear"),
                    BTN_FILL,
                    BTN_STROKE,
                    BTN_TEXT,
                    opa_pct,
                ))
                .clicked()
        {
            if settings.auto_save_transcript {
                overlay.messages.clear();
            } else {
                modal.request_confirm(
                    ConfirmKind::Clear,
                    true,
                    session.visible.get(&WinId::Panel).copied().unwrap_or(true),
                    lt_i18n::t("clear_confirm_title"),
                    lt_i18n::t("clear_confirm_msg"),
                );
            }
        }

        // 模式切换（full↔compact，200ms 高度动画）
        let mode_label = if compact {
            lt_i18n::t("mode_compact")
        } else {
            lt_i18n::t("mode_full")
        };
        if ui
            .add(small_btn(
                mode_label, BTN_FILL, BTN_STROKE, BTN_TEXT, opa_pct,
            ))
            .clicked()
        {
            toggle_mode(overlay, session, compact);
        }

        if ui
            .add(small_btn(
                lt_i18n::t("settings"),
                BTN_FILL,
                BTN_STROKE,
                BTN_TEXT,
                opa_pct,
            ))
            .clicked()
        {
            session.enqueue_action(WinId::Overlay, WinAction::ShowPanel);
        }

        // 退出（红底；与托盘同一确认语义——P1-1，不再秒退）。
        // D-33/H-3：确认改 egui 模态；紧凑/低矮时模态装不下 → 改由面板宿主。
        if ui
            .add(small_btn(
                lt_i18n::t("quit"),
                QUIT_FILL,
                QUIT_STROKE,
                BTN_TEXT,
                opa_pct,
            ))
            .clicked()
        {
            let overlay_ok = !compact && ui.ctx().content_rect().height() >= 280.0;
            let opened = modal.request_confirm(
                ConfirmKind::Quit,
                overlay_ok,
                session.visible.get(&WinId::Panel).copied().unwrap_or(true),
                lt_i18n::t("quit_confirm_title"),
                lt_i18n::t("quit_confirm_msg"),
            );
            if opened && !overlay_ok {
                session.enqueue_action(WinId::Panel, WinAction::ShowPanel);
            }
        }
    });
}

/// 行2a：穿透/置顶/自动滚动/任务栏 复选
fn row2_checks(ui: &mut Ui, overlay: &mut OverlayUi, session: &mut SessionView) {
    ui.horizontal(|ui| {
        let mut ct = overlay.ov_click_through;
        if ui
            .add(egui::Checkbox::new(
                &mut ct,
                RichText::new(lt_i18n::t("click_through")).size(10.5),
            ))
            .changed()
        {
            overlay.ov_click_through = ct;
            if ct {
                overlay.schedule_click_through_tick(session);
            }
        }
        let mut tm = overlay.ov_topmost;
        if ui
            .add(egui::Checkbox::new(
                &mut tm,
                RichText::new(lt_i18n::t("top_most")).size(10.5),
            ))
            .changed()
        {
            overlay.ov_topmost = tm;
            session.enqueue_action(WinId::Overlay, WinAction::ApplyOverlayFlags);
        }
        let mut asr = overlay.ov_auto_scroll;
        if ui
            .add(egui::Checkbox::new(
                &mut asr,
                RichText::new(lt_i18n::t("auto_scroll")).size(10.5),
            ))
            .changed()
        {
            overlay.ov_auto_scroll = asr;
        }
        let mut tb = overlay.ov_taskbar;
        if ui
            .add(egui::Checkbox::new(
                &mut tb,
                RichText::new(lt_i18n::t("taskbar")).size(10.5),
            ))
            .changed()
        {
            overlay.ov_taskbar = tb;
            session.enqueue_action(WinId::Overlay, WinAction::ApplyOverlayFlags);
        }
    });
}

/// 行2b：模型 / 源语言 / 目标语言 下拉（拉伸宽度 3:2:2）
fn row2_combos(ui: &mut Ui, session: &mut SessionView, settings: &mut Settings) {
    ui.horizontal(|ui| {
        let lbl = |ui: &mut Ui, s: String| {
            ui.label(
                RichText::new(s)
                    .monospace()
                    .size(10.5)
                    .color(Color32::from_rgb(0x88, 0x88, 0x88)),
            );
        };
        let total = ui.available_width();
        let w_model = total * 0.44;
        let w_src = total * 0.24;
        let w_tgt = total * 0.24;

        lbl(ui, lt_i18n::t("model_label"));
        let active = settings
            .active_model
            .min(settings.models.len().saturating_sub(1));
        ComboBox::from_id_salt("ov_model")
            .width(w_model - 60.0)
            .selected_text(
                settings
                    .models
                    .get(active)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "?".into()),
            )
            .show_ui(ui, |ui| {
                for (i, m) in settings.models.iter().enumerate() {
                    ui.selectable_value(&mut settings.active_model, i, m.name.clone());
                }
            });

        lbl(ui, lt_i18n::t("source_label"));
        ComboBox::from_id_salt("ov_src_lang")
            .width(w_src - 50.0)
            .selected_text(lang_label(true, &settings.asr_language))
            .show_ui(ui, |ui| {
                for (code, _native) in lt_i18n::LANGUAGES {
                    let v = settings.asr_language == *code;
                    if ui.selectable_label(v, lang_label(true, code)).clicked() {
                        settings.asr_language = code.to_string();
                        session.send_cmd(lt_proto::Cmd::SetAsrLanguage(code.to_string()));
                    }
                }
            });

        lbl(ui, lt_i18n::t("target_label"));
        ComboBox::from_id_salt("ov_tgt_lang")
            .width(w_tgt - 50.0)
            .selected_text(lang_label(false, &settings.target_language))
            .show_ui(ui, |ui| {
                for (code, _native) in lt_i18n::LANGUAGES {
                    if *code == "auto" {
                        continue;
                    }
                    let v = settings.target_language == *code;
                    if ui.selectable_label(v, lang_label(false, code)).clicked() {
                        settings.target_language = code.to_string();
                        session.send_cmd(lt_proto::Cmd::SetTargetLanguage(code.to_string()));
                    }
                }
            });
    });
}

/// 下拉显示文本：源语言 "auto - 自动检测"；目标 "zh - 简体中文"（原版 `{code} - {label}`）
fn lang_label(with_auto: bool, code: &str) -> String {
    let label: String = if code == "auto" {
        if with_auto {
            lt_i18n::t("asr_lang_auto")
        } else {
            code.to_string()
        }
    } else {
        lt_i18n::LANGUAGES
            .iter()
            .find(|(c, _)| *c == code)
            .and_then(|(_, n)| *n)
            .unwrap_or(code)
            .to_string()
    };
    format!("{code} - {label}")
}

/// 紧凑模式切换：UI 侧仅翻转模式并投递 ToggleMode；
/// 高度动画的 from/to 由宿主按当前窗口高度计算（原版 _on_mode_changed 的
/// _height_before_compact/minimumHeight 逻辑在窗口层）。
fn toggle_mode(overlay: &mut OverlayUi, session: &mut SessionView, to_compact: bool) {
    overlay.state.mode = if to_compact {
        OverlayMode::Compact
    } else {
        OverlayMode::Full
    };
    session.enqueue_action(WinId::Overlay, WinAction::ToggleMode);
}

// ── MonitorBar（原版 MonitorBar） ──

fn monitor_bar(ui: &mut Ui, overlay: &OverlayUi, settings: &Settings, opa_pct: u32) {
    ui.horizontal(|ui| {
        // D-29：MIC 条显隐按"启用意图"（settings.mic_device 有值）驱动，mic_rms 仅作为
        // 条值填充。原版以 mic_rms 有值驱动，而事件只在有 loopback 数据时到达 →
        // 系统首次出声才"冒出" MIC 条（用户实测幽灵条根因）。
        let mic_active = mic_bar_active(settings);
        let n_bars = if mic_active { 3.0 } else { 2.0 };
        let bar_w = ((ui.available_width() - 26.0 * n_bars - 12.0) / n_bars).max(60.0);
        if mic_active {
            let mic = overlay.monitor.mic_rms.unwrap_or(0.0);
            level_bar(ui, "MIC", mic, BAR_MIC, opa_pct, bar_w);
        }
        level_bar(ui, "RMS:", overlay.monitor.rms, BAR_RMS, opa_pct, bar_w);
        level_bar(ui, "VAD:", overlay.monitor.vad, BAR_VAD, opa_pct, bar_w);
    });
    ui.add_space(2.0);
    stats_line(ui, overlay, opa_pct);
    ui.add_space(2.0);
}

/// MIC 条显隐：以麦克风启用意图为准（D-29，见 monitor_bar 注）。启用但尚无音频数据
/// 时 mic_rms 为 None → 显示 0%（而非隐藏，避免"幽灵出现"观感）。
fn mic_bar_active(settings: &Settings) -> bool {
    settings.mic_device.is_some()
}

/// 原版 update_audio：value = min(100, int(v * 500))
fn level_bar(ui: &mut Ui, label: &str, v: f32, color: Color32, opa_pct: u32, w: f32) {
    const H: f32 = 14.0;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(label)
                .monospace()
                .size(10.5)
                .color(Color32::from_rgb(0x88, 0x88, 0x88)),
        );
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, H), Sense::hover());
        let pct = (v * 500.0).clamp(0.0, 100.0);
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, opa(BAR_SLOT, opa_pct));
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0, opa(BAR_STROKE, opa_pct)),
            egui::StrokeKind::Inside,
        );
        if pct > 1.0 {
            let fill =
                egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * pct / 100.0, H));
            painter.rect_filled(fill, 2.0, opa(color, opa_pct));
        }
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            format!("{:.0}%", pct),
            FontId::monospace(10.0),
            opa(BTN_TEXT, opa_pct),
        );
    });
}

/// 统计行（原版 _refresh_stats 的富文本 span 序列：数值继承 #888，cost 在行尾）
fn stats_line(ui: &mut Ui, overlay: &OverlayUi, opa_pct: u32) {
    let dev_color = |t: &str| {
        if t.to_lowercase().contains("cuda") {
            Color32::from_rgb(0x4e, 0xc9, 0xb0)
        } else {
            Color32::from_rgb(0xdc, 0xdc, 0xaa)
        }
    };
    let m = overlay.monitor;
    let stats = overlay.stats;
    let total_tokens = stats.prompt_tokens + stats.completion_tokens;
    let tokens_str = if total_tokens >= 1000 {
        format!("{:.1}k", total_tokens as f64 / 1000.0)
    } else {
        format!("{total_tokens}")
    };
    let o = |c: Color32| opa(c, opa_pct);

    ui.horizontal_wrapped(|ui| {
        if let Some(dev) = &overlay.asr_label {
            ui.label(
                RichText::new(dev.clone())
                    .monospace()
                    .size(10.5)
                    .color(o(dev_color(dev))),
            );
            ui.label(RichText::new("|").monospace().size(10.5).color(o(SEP)));
        }
        ui.label(
            RichText::new("CPU")
                .monospace()
                .size(10.5)
                .color(o(style::LANG_BLUE)),
        );
        ui.label(
            RichText::new(format!("{:.0}%", m.cpu))
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(
            RichText::new("RAM")
                .monospace()
                .size(10.5)
                .color(o(style::LANG_BLUE)),
        );
        ui.label(
            RichText::new(format!("{}MB", m.ram_mb as i64))
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(
            RichText::new("GPU")
                .monospace()
                .size(10.5)
                .color(o(style::LANG_BLUE)),
        );
        ui.label(
            RichText::new("N/A")
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(RichText::new("|").monospace().size(10.5).color(o(SEP)));
        ui.label(
            RichText::new("ASR")
                .monospace()
                .size(10.5)
                .color(o(style::ASR_MS)),
        );
        ui.label(
            RichText::new(format!("{}", stats.asr_n))
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(
            RichText::new("TL")
                .monospace()
                .size(10.5)
                .color(o(style::TL_MS)),
        );
        ui.label(
            RichText::new(format!("{}", stats.tl_n))
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(
            RichText::new("Tok")
                .monospace()
                .size(10.5)
                .color(Color32::from_rgb(0xcc, 0x99, 0xcc)),
        );
        ui.label(
            RichText::new(tokens_str)
                .monospace()
                .size(10.5)
                .color(o(STATS_VAL)),
        );
        ui.label(
            RichText::new(format!(
                "({}\u{2191}{}\u{2193})",
                stats.prompt_tokens, stats.completion_tokens
            ))
            .monospace()
            .size(10.5)
            .color(o(Color32::from_rgb(0x66, 0x66, 0x66))),
        );
        // cost 在统计行末尾（原版 _refresh_stats 的 cost_str）
        if stats.cost > 0.0 {
            let symbol = if lt_i18n::get_lang() == "zh" {
                "¥"
            } else {
                "$"
            };
            ui.label(
                RichText::new(format!("{symbol}{:.4}", stats.cost))
                    .monospace()
                    .size(10.5)
                    .color(o(Color32::from_rgb(0xff, 0xaa, 0x55))),
            );
        }
    });
}

// ── 消息流 ──

fn messages_area(
    ui: &mut Ui,
    overlay: &mut OverlayUi,
    settings: &Settings,
    ctx: &UiContext,
    compact: bool,
    opa_pct: u32,
) {
    // 记录消息区顶部 y（穿透轮询的可交互分界）
    overlay.state.header_px = ui.cursor().top();

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            // 原版空态为纯黑空白（无占位文本）
            for msg in &overlay.messages {
                let mut export: Option<lt_proto::ExportFileMode> = None;
                let mut clear = false;
                message_block(ui, settings, ctx, msg, compact, opa_pct, &mut export, &mut clear);
                if let Some(mode) = export {
                    overlay.state.export_request = Some(mode);
                }
                if clear {
                    overlay.clear_request = true;
                }
                ui.add_space(2.0);
            }
            if overlay.state.scroll_pending && overlay.ov_auto_scroll {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                overlay.state.scroll_pending = false;
            }
        });
}

/// 单条消息：头部行 + 译文行 + 右键菜单。原版 ChatMessage = QVBoxLayout 两个
/// 独立换行的富文本 label（译文行必另起一行），外层 margins (8,4,8,4)；
/// 头部行字号 = original_font_size（YaHei 11pt），译文行 = translation_font_size（14pt）。
/// 菜单动作经局部标志回传（消息链借用期间不可变写 AppState）。
fn message_block(
    ui: &mut Ui,
    settings: &Settings,
    ctx: &UiContext,
    msg: &OverlayMessage,
    compact: bool,
    opa_pct: u32,
    export: &mut Option<lt_proto::ExportFileMode>,
    clear: &mut bool,
) {
    let s = &settings.style;
    let o = |c: Color32| opa(c, opa_pct);
    let orig_c = o(parse_color(
        &s.original_color,
        Color32::from_rgb(0xcc, 0xcc, 0xcc),
    ));
    let ts_c = o(parse_color(
        &s.timestamp_color,
        Color32::from_rgb(0x88, 0x88, 0x99),
    ));
    let trans_c = o(parse_color(
        &s.translation_color,
        Color32::from_rgb(0xff, 0xff, 0xff),
    ));
    // D-17 级联：样式键空串=跟随字幕主字体；未注册族名回落全局链
    let head_font = FontId::new(
        pt(s.original_font_size),
        crate::fonts::font_family_for(
            crate::fonts::resolve_family(
                &s.original_font_family,
                &settings.subtitle_font_family,
            ),
            &ctx.fonts,
        ),
    );
    let trans_font = FontId::new(
        pt(s.translation_font_size),
        crate::fonts::font_family_for(
            crate::fonts::resolve_family(
                &s.translation_font_family,
                &settings.subtitle_font_family,
            ),
            &ctx.fonts,
        ),
    );
    let ms_font = FontId::proportional(pt(9));

    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            // ── 头部行：[ts] [lang] 原文 ASR x ms（紧凑模式隐藏 ts 与 ASR 耗时） ──
            ui.horizontal_wrapped(|ui| {
                if !compact {
                    ui.label(
                        RichText::new(format!("[{}]", msg.timestamp))
                            .font(head_font.clone())
                            .color(ts_c),
                    );
                }
                ui.label(
                    RichText::new(format!("[{}]", msg.lang))
                        .font(head_font.clone())
                        .color(o(style::LANG_BLUE)),
                );
                ui.label(
                    RichText::new(&msg.original)
                        .font(head_font.clone())
                        .color(orig_c),
                );
                if !compact {
                    ui.label(
                        RichText::new(format!("ASR {:.0}ms", msg.asr_ms))
                            .font(ms_font.clone())
                            .color(o(style::ASR_MS)),
                    );
                }
            });
            // ── 译文行（独立换行：`> 译文` + TL 耗时 / 占位） ──
            ui.horizontal_wrapped(|ui| match &msg.translation {
                None => {
                    ui.label(
                        RichText::new(format!("> {}", lt_i18n::t("translating")))
                            .font(trans_font.clone())
                            .italics()
                            .color(o(style::PLACEHOLDER)),
                    );
                }
                Some(text) if text.is_empty() => {
                    ui.label(
                        RichText::new(format!("> {}", lt_i18n::t("same_language")))
                            .font(trans_font.clone())
                            .italics()
                            .color(o(style::SAME_LANG)),
                    );
                }
                Some(text) => {
                    ui.label(
                        RichText::new(format!("> {text}"))
                            .font(trans_font.clone())
                            .color(trans_c),
                    );
                    if !compact && !msg.streaming {
                        ui.label(
                            RichText::new(format!("TL {:.0}ms", msg.tl_ms))
                                .font(ms_font.clone())
                                .color(o(style::TL_MS)),
                        );
                    }
                }
            })
        });
    let block_rect = inner.response.rect;

    // 右键菜单（原版 contextMenuEvent）
    let resp = ui.interact(block_rect, inner.response.id, Sense::hover());
    resp.context_menu(|ui| {
        if ui.button(lt_i18n::t("copy_original")).clicked() {
            ui.ctx().copy_text(msg.original.clone());
            ui.close();
        }
        if ui.button(lt_i18n::t("copy_translation")).clicked() {
            ui.ctx()
                .copy_text(msg.translation.clone().unwrap_or_default());
            ui.close();
        }
        if ui.button(lt_i18n::t("copy_all")).clicked() {
            ui.ctx().copy_text(format!(
                "{}\n{}",
                msg.original,
                msg.translation.clone().unwrap_or_default()
            ));
            ui.close();
        }
        ui.separator();
        ui.menu_button(lt_i18n::t("export_menu"), |ui| {
            // 三种模式（原版 export_original/export_translation/export_all）；
            // W5 起类型化（字符串旁路禁令 INV9 的对位）
            for (key, mode) in [
                ("export_original", lt_proto::ExportFileMode::Original),
                ("export_translation", lt_proto::ExportFileMode::Translation),
                ("export_all", lt_proto::ExportFileMode::All),
            ] {
                if ui.button(lt_i18n::t(key)).clicked() {
                    *export = Some(mode);
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button(lt_i18n::t("clear_list")).clicked() {
            *clear = true;
            ui.close();
        }
    });
}

/// 按钮统一外观（原版 _BTN_CSS：11px 字号、20px 高、圆角 3、padding 0-6；
/// 整窗不透明度由宿主 LWA_ALPHA 统一乘，控件不再自乘——opa 恒等）
fn small_btn(
    label: String,
    fill: Color32,
    stroke_col: Color32,
    text_col: Color32,
    opa_pct: u32,
) -> Button<'static> {
    Button::new(
        RichText::new(label)
            .size(11.0)
            .color(opa(text_col, opa_pct)),
    )
    .fill(opa(fill, opa_pct))
    .stroke(Stroke::new(1.0, opa(stroke_col, opa_pct)))
    .corner_radius(CornerRadius::same(3))
    .min_size(Vec2::new(0.0, 20.0))
}

/// 行1 按钮组预留宽度（拖动区让位；完整=7 按钮含隐藏、紧凑=5 按钮
/// （隐藏/启停/完整/设置/退出；字幕与清空隐藏），每按钮≈46 逻辑 px + 余量）
fn button_reserved(compact: bool) -> f32 {
    if compact {
        240.0
    } else {
        380.0
    }
}

/// 右下角尺寸手柄（原版 QSizeGrip 16×16 的小点串斜纹；拖动生效）
fn resize_grip(ui: &mut Ui, session: &mut SessionView) {
    let rect = egui::Rect::from_min_size(
        ui.clip_rect().right_bottom() - Vec2::new(16.0, 16.0),
        Vec2::new(16.0, 16.0),
    );
    // 原版 Windows 暗色 QSizeGrip：沿对角线 3 组小点（每组 2 点），微灰不抢眼
    let col = Color32::from_rgba_premultiplied(0x78, 0x78, 0x80, 0x64);
    let painter = ui.painter();
    for i in 0..3 {
        let off = 3.5 + i as f32 * 3.5;
        for dx in [0.0, 2.5] {
            painter.circle_filled(
                egui::pos2(rect.right() - off + dx, rect.bottom() - off),
                1.2,
                col,
            );
        }
    }
    if ui
        .interact(rect, ui.id().with("ov_grip"), Sense::click_and_drag())
        .drag_started()
    {
        session.enqueue_action(WinId::Overlay, WinAction::ResizeSouthEast);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-29：MIC 条显隐跟随"启用意图"（mic_device 有值），与是否有监控数据无关。
    /// 默认禁用 → 恒不显示；启用（默认/具名）→ 立即显示。
    #[test]
    fn mic_bar_follows_enable_intent() {
        let base = lt_proto::Settings::default();
        assert!(
            !mic_bar_active(&base),
            "默认 mic_device=None = 禁用"
        );

        let mut s = base.clone();
        s.mic_device = Some("__default__".into());
        assert!(mic_bar_active(&s), "系统默认 = 启用");

        let mut s = base;
        s.mic_device = Some("Mic X".into());
        assert!(mic_bar_active(&s), "具名设备 = 启用");
    }

    /// W5/D-71：头部拖动起止入队（宿主 SetCapture 绝对跟踪的唯一输入面——
    /// egui 报 drag_started/drag_stopped，动作队列见 WinAction::OverlayDragStart/End）
    #[test]
    fn overlay_drag_start_stop_enqueued() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        let mut acts: Vec<(crate::state::WinId, crate::state::WinAction)> = Vec::new();
        let run_frame = |ctx: &egui::Context, st: &mut crate::state::AppUi, evs: Vec<egui::Event>| {
            let mut ri = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(620.0, 500.0),
                )),
                ..Default::default()
            };
            ri.events = evs;
            let mut out = ctx.run_ui(ri, |ui| {
                crate::windows::overlay::overlay_ui(
                    ui,
                    &mut st.overlay,
                    &mut st.session,
                    &mut st.settings,
                    &mut st.modal,
                    &mut st.ctx,
                )
            });
            out.textures_delta.clear();
        };
        // 注册帧（布局收敛）→ 光标到位 → 按住 → 移动（拖拽判定）→ 释放
        let start = egui::pos2(60.0, 12.0); // 头部行内（拖动区）
        for evs in [
            vec![],
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
            vec![egui::Event::PointerMoved(start + egui::vec2(24.0, 8.0))],
            vec![egui::Event::PointerButton {
                pos: start + egui::vec2(24.0, 8.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        ] {
            run_frame(&ctx, &mut st, evs);
            acts.extend(st.session.drain_actions());
            // 宿主语义：收到 OverlayDragStart 后置拖动态（process_actions 置位时机）
            if acts.iter().any(|(w, a)| {
                *w == crate::state::WinId::Overlay
                    && *a == crate::state::WinAction::OverlayDragStart
            }) {
                st.overlay.state.dragging = true;
            }
            if acts.iter().any(|(w, a)| {
                *w == crate::state::WinId::Overlay
                    && *a == crate::state::WinAction::OverlayDragEnd
            }) {
                st.overlay.state.dragging = false;
            }
        }
        let has = |a: crate::state::WinAction| {
            acts.iter().any(|(w, x)| *w == crate::state::WinId::Overlay && *x == a)
        };
        assert!(
            has(crate::state::WinAction::OverlayDragStart),
            "头部按住并移动应入队 OverlayDragStart"
        );
        assert!(
            has(crate::state::WinAction::OverlayDragEnd),
            "释放应入队 OverlayDragEnd"
        );
    }
}
