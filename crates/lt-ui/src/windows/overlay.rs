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
    AppState, OverlayMessage, OverlayMode, WinAction, WinId,
};
use crate::style::{self, parse_color};
use egui::{Align2, Button, Color32, ComboBox, CornerRadius, FontId, RichText, ScrollArea, Sense, Stroke, Ui, Vec2};

/// pt → 逻辑 px（Qt 1pt ≈ 96/72 px）
fn pt(size: u32) -> f32 {
    size as f32 * 4.0 / 3.0
}

/// 两级透明乘算：Qt rgba alpha（0-255）× 整窗不透明度（百分比）。
/// 等价原版 QSS `rgba(r,g,b,alpha)` + `setWindowOpacity(pct/100)` 叠加；
/// egui Color32 为预乘 alpha 空间，全分量同乘系数即正确合成。
fn fade(c: Color32, alpha_0_255: u32, opacity_pct: u32) -> Color32 {
    let k = (alpha_0_255.min(255) as u16 * opacity_pct.min(100) as u16 / 100) as u16;
    let m = |v: u8| (v as u16 * k / 255) as u8;
    Color32::from_rgba_premultiplied(m(c.r()), m(c.g()), m(c.b()), m(c.a()))
}

/// 按整窗不透明度百分比淡出（alpha=255 特例）。
fn opa(c: Color32, opacity_pct: u32) -> Color32 {
    fade(c, 255, opacity_pct)
}

/// 原版字面色
const BTN_TEXT: Color32 = Color32::from_rgb(0xaa, 0xaa, 0xaa);
const BTN_FILL: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
const BTN_STROKE: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
/// 按钮/下拉 hover 底（原版 _BTN_CSS hover：rgba(255,255,255,40)）
const BTN_HOVER_FILL: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
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

pub fn overlay_ui(ui: &mut Ui, state: &mut AppState) {
    let st = &state.settings.style;
    let opa_pct = st.window_opacity;
    let compact = state.overlay.mode == OverlayMode::Compact;
    let bg = fade(
        parse_color(&st.bg_color, Color32::from_rgb(15, 15, 25)),
        st.bg_opacity,
        st.window_opacity,
    );
    let radius = st.border_radius as f32;

    // 动画驱动：进行中则每帧请求窗口高度调整 + 重绘
    if let Some(h) = state.overlay.anim.and_then(|a| a.current(std::time::Instant::now())) {
        state.enqueue_action(WinId::Overlay, WinAction::SetHeight(h));
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
            let vis = &mut ui.style_mut().visuals;
            vis.widgets.inactive.bg_fill = BTN_FILL;
            vis.widgets.inactive.bg_stroke = Stroke::new(1.0, BTN_STROKE);
            vis.widgets.hovered.bg_fill = BTN_HOVER_FILL;
            vis.widgets.hovered.bg_stroke = Stroke::new(1.0, BTN_STROKE);

            drag_handle(ui, state, compact, opa_pct);
            if !compact {
                monitor_bar(ui, state, opa_pct);
            }
            messages_area(ui, state, compact, opa_pct);
            resize_grip(ui, state);
        });
}

// ── DragHandle（原版 DragHandle） ──

fn drag_handle(ui: &mut Ui, state: &mut AppState, compact: bool, opa_pct: u32) {
    // 原版 DragHandle 是 QWidget 子类且未设 WA_StyledBackground/paintEvent，
    // 其 QSS 背景（含 apply_style 的 header_color）从未被渲染——头部区域即
    // 容器黑玻璃贯穿。header_color/header_opacity 字段与样式页控件 1:1 保留，
    // 仅绘制不消费（对齐原版实际行为，D-17）。
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            row1(ui, state, compact, opa_pct);
            if !compact {
                row2_checks(ui, state);
                row2_combos(ui, state);
            }
        });
}

/// 行1：拖动标题 + 操作按钮（高度 24；按钮顺序=原版 row1：
/// 隐藏/字幕/启停/清除/完整/设置/退出）
fn row1(ui: &mut Ui, state: &mut AppState, compact: bool, opa_pct: u32) {
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
        if ui
            .interact(drag_rect, ui.id().with("ov_drag"), Sense::click_and_drag())
            .drag_started()
        {
            state.enqueue_action(WinId::Overlay, WinAction::Drag);
        }

        // 隐藏（原版 hide_btn：隐藏悬浮窗，托盘"显示悬浮窗"可恢复 + 首次气泡提示）
        if ui.add(small_btn(lt_i18n::t("hide"), BTN_FILL, BTN_STROKE, BTN_TEXT, opa_pct)).clicked() {
            state.enqueue_action(WinId::Overlay, WinAction::Hide);
        }

        // 字幕按钮（紧凑模式隐藏；开启时绿底，原版 set_subtitle_checked）
        if !compact {
            let on = state.settings.subtitle_mode.enabled;
            let sub = ui.add(small_btn(
                lt_i18n::t("subtitle"),
                if on { SUBTITLE_ON_FILL } else { BTN_FILL },
                if on { SUBTITLE_ON_STROKE } else { BTN_STROKE },
                BTN_TEXT,
                opa_pct,
            ));
            if sub.clicked() {
                // 原版 subtitle_toggled → 切换字幕窗可见性
                let vis = !state.settings.subtitle_mode.enabled;
                state.settings.subtitle_mode.enabled = vis;
                state.enqueue_action(WinId::Overlay, WinAction::ToggleSubtitle);
            }
        }

        // 启停按钮（运行=普通样式 t("running")；暂停=琥珀色 t("paused")）
        let running = state.running;
        let (label, fill, stroke, text) = if running {
            (lt_i18n::t("running"), BTN_FILL, BTN_STROKE, BTN_TEXT)
        } else {
            (lt_i18n::t("paused"), PAUSED_FILL, BTN_STROKE, PAUSED_TEXT)
        };
        let start_stop = ui.add(small_btn(label, fill, stroke, text, opa_pct));
        if start_stop.clicked() {
            let cmd = if running { lt_proto::Cmd::Pause } else { lt_proto::Cmd::Resume };
            state.send_cmd(cmd);
            state.running = !running;
        }

        // 清空（紧凑模式隐藏）
        if !compact && ui.add(small_btn(lt_i18n::t("clear"), BTN_FILL, BTN_STROKE, BTN_TEXT, opa_pct)).clicked() {
            state.messages.clear();
        }

        // 模式切换（full↔compact，200ms 高度动画）
        let mode_label = if compact { lt_i18n::t("mode_compact") } else { lt_i18n::t("mode_full") };
        if ui.add(small_btn(mode_label, BTN_FILL, BTN_STROKE, BTN_TEXT, opa_pct)).clicked() {
            toggle_mode(state, compact);
        }

        if ui.add(small_btn(lt_i18n::t("settings"), BTN_FILL, BTN_STROKE, BTN_TEXT, opa_pct)).clicked() {
            state.enqueue_action(WinId::Overlay, WinAction::ShowPanel);
        }

        // 退出（红底）
        if ui.add(small_btn(lt_i18n::t("quit"), QUIT_FILL, QUIT_STROKE, BTN_TEXT, opa_pct)).clicked() {
            state.quit_requested = true;
        }
    });
}

/// 行2a：穿透/置顶/自动滚动/任务栏 复选
fn row2_checks(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let mut ct = state.ov_click_through;
        if ui.add(egui::Checkbox::new(&mut ct, RichText::new(lt_i18n::t("click_through")).size(10.5))).changed() {
            state.ov_click_through = ct;
            if ct {
                state.schedule_click_through_tick();
            }
        }
        let mut tm = state.ov_topmost;
        if ui.add(egui::Checkbox::new(&mut tm, RichText::new(lt_i18n::t("top_most")).size(10.5))).changed() {
            state.ov_topmost = tm;
            state.enqueue_action(WinId::Overlay, WinAction::ApplyOverlayFlags);
        }
        let mut asr = state.ov_auto_scroll;
        if ui.add(egui::Checkbox::new(&mut asr, RichText::new(lt_i18n::t("auto_scroll")).size(10.5))).changed() {
            state.ov_auto_scroll = asr;
        }
        let mut tb = state.ov_taskbar;
        if ui.add(egui::Checkbox::new(&mut tb, RichText::new(lt_i18n::t("taskbar")).size(10.5))).changed() {
            state.ov_taskbar = tb;
            state.enqueue_action(WinId::Overlay, WinAction::ApplyOverlayFlags);
        }
    });
}

/// 行2b：模型 / 源语言 / 目标语言 下拉（拉伸宽度 3:2:2）
fn row2_combos(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let lbl = |ui: &mut Ui, s: String| {
            ui.label(RichText::new(s).monospace().size(10.5).color(Color32::from_rgb(0x88, 0x88, 0x88)));
        };
        let total = ui.available_width();
        let w_model = total * 0.44;
        let w_src = total * 0.24;
        let w_tgt = total * 0.24;

        lbl(ui, lt_i18n::t("model_label"));
        let active = state.settings.active_model.min(state.settings.models.len().saturating_sub(1));
        ComboBox::from_id_salt("ov_model")
            .width(w_model - 60.0)
            .selected_text(
                state
                    .settings
                    .models
                    .get(active)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| "?".into()),
            )
            .show_ui(ui, |ui| {
                for (i, m) in state.settings.models.iter().enumerate() {
                    ui.selectable_value(&mut state.settings.active_model, i, m.name.clone());
                }
            });

        lbl(ui, lt_i18n::t("source_label"));
        ComboBox::from_id_salt("ov_src_lang")
            .width(w_src - 50.0)
            .selected_text(lang_label(true, &state.settings.asr_language))
            .show_ui(ui, |ui| {
                for (code, _native) in lt_i18n::LANGUAGES {
                    let v = state.settings.asr_language == *code;
                    if ui.selectable_label(v, lang_label(true, code)).clicked() {
                        state.settings.asr_language = code.to_string();
                        state.send_cmd(lt_proto::Cmd::SetAsrLanguage(code.to_string()));
                    }
                }
            });

        lbl(ui, lt_i18n::t("target_label"));
        ComboBox::from_id_salt("ov_tgt_lang")
            .width(w_tgt - 50.0)
            .selected_text(lang_label(false, &state.settings.target_language))
            .show_ui(ui, |ui| {
                for (code, _native) in lt_i18n::LANGUAGES {
                    if *code == "auto" {
                        continue;
                    }
                    let v = state.settings.target_language == *code;
                    if ui.selectable_label(v, lang_label(false, code)).clicked() {
                        state.settings.target_language = code.to_string();
                        state.send_cmd(lt_proto::Cmd::SetTargetLanguage(code.to_string()));
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
fn toggle_mode(state: &mut AppState, to_compact: bool) {
    state.overlay.mode = if to_compact { OverlayMode::Compact } else { OverlayMode::Full };
    state.enqueue_action(WinId::Overlay, WinAction::ToggleMode);
}

// ── MonitorBar（原版 MonitorBar） ──

fn monitor_bar(ui: &mut Ui, state: &AppState, opa_pct: u32) {
    ui.horizontal(|ui| {
        // 原版：MIC 条仅在麦克风启用（mic_rms 有值）时出现；各条 QProgressBar
        // stretch 平分整行（26=原版标签定宽，12=行内间距余量）
        let n_bars = if state.monitor.mic_rms.is_some() { 3.0 } else { 2.0 };
        let bar_w = ((ui.available_width() - 26.0 * n_bars - 12.0) / n_bars).max(60.0);
        if let Some(mic) = state.monitor.mic_rms {
            level_bar(ui, "MIC", mic, BAR_MIC, opa_pct, bar_w);
        }
        level_bar(ui, "RMS:", state.monitor.rms, BAR_RMS, opa_pct, bar_w);
        level_bar(ui, "VAD:", state.monitor.vad, BAR_VAD, opa_pct, bar_w);
    });
    ui.add_space(2.0);
    stats_line(ui, state, opa_pct);
    ui.add_space(2.0);
}

/// 原版 update_audio：value = min(100, int(v * 500))
fn level_bar(ui: &mut Ui, label: &str, v: f32, color: Color32, opa_pct: u32, w: f32) {
    const H: f32 = 14.0;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).monospace().size(10.5).color(Color32::from_rgb(0x88, 0x88, 0x88)));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, H), Sense::hover());
        let pct = (v * 500.0).clamp(0.0, 100.0);
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, opa(BAR_SLOT, opa_pct));
        painter.rect_stroke(rect, 3.0, Stroke::new(1.0, opa(BAR_STROKE, opa_pct)), egui::StrokeKind::Inside);
        if pct > 1.0 {
            let fill = egui::Rect::from_min_size(rect.min, Vec2::new(rect.width() * pct / 100.0, H));
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
fn stats_line(ui: &mut Ui, state: &AppState, opa_pct: u32) {
    let dev_color = |t: &str| {
        if t.to_lowercase().contains("cuda") {
            Color32::from_rgb(0x4e, 0xc9, 0xb0)
        } else {
            Color32::from_rgb(0xdc, 0xdc, 0xaa)
        }
    };
    let m = state.monitor;
    let stats = state.stats;
    let total_tokens = stats.prompt_tokens + stats.completion_tokens;
    let tokens_str = if total_tokens >= 1000 {
        format!("{:.1}k", total_tokens as f64 / 1000.0)
    } else {
        format!("{total_tokens}")
    };
    let o = |c: Color32| opa(c, opa_pct);

    ui.horizontal_wrapped(|ui| {
        if let Some(dev) = &state.asr_label {
            ui.label(RichText::new(dev.clone()).monospace().size(10.5).color(o(dev_color(dev))));
            ui.label(RichText::new("|").monospace().size(10.5).color(o(SEP)));
        }
        ui.label(RichText::new("CPU").monospace().size(10.5).color(o(style::LANG_BLUE)));
        ui.label(RichText::new(format!("{:.0}%", m.cpu)).monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(RichText::new("RAM").monospace().size(10.5).color(o(style::LANG_BLUE)));
        ui.label(RichText::new(format!("{}MB", m.ram_mb as i64)).monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(RichText::new("GPU").monospace().size(10.5).color(o(style::LANG_BLUE)));
        ui.label(RichText::new("N/A").monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(RichText::new("|").monospace().size(10.5).color(o(SEP)));
        ui.label(RichText::new("ASR").monospace().size(10.5).color(o(style::ASR_MS)));
        ui.label(RichText::new(format!("{}", stats.asr_n)).monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(RichText::new("TL").monospace().size(10.5).color(o(style::TL_MS)));
        ui.label(RichText::new(format!("{}", stats.tl_n)).monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(RichText::new("Tok").monospace().size(10.5).color(Color32::from_rgb(0xcc, 0x99, 0xcc)));
        ui.label(RichText::new(tokens_str).monospace().size(10.5).color(o(STATS_VAL)));
        ui.label(
            RichText::new(format!("({}\u{2191}{}\u{2193})", stats.prompt_tokens, stats.completion_tokens))
                .monospace()
                .size(10.5)
                .color(o(Color32::from_rgb(0x66, 0x66, 0x66))),
        );
        // cost 在统计行末尾（原版 _refresh_stats 的 cost_str）
        if stats.cost > 0.0 {
            let symbol = if lt_i18n::get_lang() == "zh" { "¥" } else { "$" };
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

fn messages_area(ui: &mut Ui, state: &mut AppState, compact: bool, opa_pct: u32) {
    // 记录消息区顶部 y（穿透轮询的可交互分界）
    state.overlay.header_px = ui.cursor().top();

    ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            // 原版空态为纯黑空白（无占位文本）
            for msg in &state.messages {
                let mut export: Option<String> = None;
                let mut clear = false;
                message_block(ui, state, msg, compact, opa_pct, &mut export, &mut clear);
                if let Some(mode) = export {
                    state.overlay.export_request = Some(mode);
                }
                if clear {
                    state.clear_request = true;
                }
                ui.add_space(2.0);
            }
            if state.overlay.scroll_pending && state.ov_auto_scroll {
                ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                state.overlay.scroll_pending = false;
            }
        });
}

/// 单条消息：头部行 + 译文行 + 右键菜单。原版 ChatMessage = QVBoxLayout 两个
/// 独立换行的富文本 label（译文行必另起一行），外层 margins (8,4,8,4)；
/// 头部行字号 = original_font_size（YaHei 11pt），译文行 = translation_font_size（14pt）。
/// 菜单动作经局部标志回传（消息链借用期间不可变写 AppState）。
fn message_block(
    ui: &mut Ui,
    state: &AppState,
    msg: &OverlayMessage,
    compact: bool,
    opa_pct: u32,
    export: &mut Option<String>,
    clear: &mut bool,
) {
    let s = &state.settings.style;
    let o = |c: Color32| opa(c, opa_pct);
    let orig_c = o(parse_color(&s.original_color, Color32::from_rgb(0xcc, 0xcc, 0xcc)));
    let ts_c = o(parse_color(&s.timestamp_color, Color32::from_rgb(0x88, 0x88, 0x99)));
    let trans_c = o(parse_color(&s.translation_color, Color32::from_rgb(0xff, 0xff, 0xff)));
    let head_font = FontId::proportional(pt(s.original_font_size));
    let trans_font = FontId::proportional(pt(s.translation_font_size));
    let ms_font = FontId::proportional(pt(9));

    let inner = egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            // ── 头部行：[ts] [lang] 原文 ASR x ms（紧凑模式隐藏 ts 与 ASR 耗时） ──
            ui.horizontal_wrapped(|ui| {
                if !compact {
                    ui.label(RichText::new(format!("[{}]", msg.timestamp)).font(head_font.clone()).color(ts_c));
                }
                ui.label(RichText::new(format!("[{}]", msg.lang)).font(head_font.clone()).color(o(style::LANG_BLUE)));
                ui.label(RichText::new(&msg.original).font(head_font.clone()).color(orig_c));
                if !compact {
                    ui.label(
                        RichText::new(format!("ASR {:.0}ms", msg.asr_ms))
                            .font(ms_font.clone())
                            .color(o(style::ASR_MS)),
                    );
                }
            });
            // ── 译文行（独立换行：`> 译文` + TL 耗时 / 占位） ──
            ui.horizontal_wrapped(|ui| {
                match &msg.translation {
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
                        ui.label(RichText::new(format!("> {text}")).font(trans_font.clone()).color(trans_c));
                        if !compact && !msg.streaming {
                            ui.label(
                                RichText::new(format!("TL {:.0}ms", msg.tl_ms))
                                    .font(ms_font.clone())
                                    .color(o(style::TL_MS)),
                            );
                        }
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
            ui.ctx().copy_text(msg.translation.clone().unwrap_or_default());
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
            for (key, mode) in [
                ("export_original", "original"),
                ("export_translation", "translation"),
                ("export_all", "both"),
            ] {
                if ui.button(lt_i18n::t(key)).clicked() {
                    *export = Some(mode.to_string());
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
/// 填充/描边/文字统一乘整窗不透明度，等价 setWindowOpacity 作用到按钮）
fn small_btn(
    label: String,
    fill: Color32,
    stroke_col: Color32,
    text_col: Color32,
    opa_pct: u32,
) -> Button<'static> {
    Button::new(RichText::new(label).size(11.0).color(opa(text_col, opa_pct)))
        .fill(opa(fill, opa_pct))
        .stroke(Stroke::new(1.0, opa(stroke_col, opa_pct)))
        .corner_radius(CornerRadius::same(3))
        .min_size(Vec2::new(0.0, 20.0))
}

/// 行1 按钮组预留宽度（拖动区让位；完整=7 按钮含隐藏、紧凑=5 按钮
/// （隐藏/启停/完整/设置/退出；字幕与清空隐藏），每按钮≈46 逻辑 px + 余量）
fn button_reserved(compact: bool) -> f32 {
    if compact { 240.0 } else { 380.0 }
}

/// 右下角尺寸手柄（原版 QSizeGrip 16×16：三条斜线 + 拖动）
fn resize_grip(ui: &mut Ui, state: &mut AppState) {
    let rect = egui::Rect::from_min_size(
        ui.clip_rect().right_bottom() - Vec2::new(16.0, 16.0),
        Vec2::new(16.0, 16.0),
    );
    let col = ui.visuals().widgets.inactive.fg_stroke.color;
    let painter = ui.painter();
    for i in 0..3 {
        let off = 3.0 + i as f32 * 4.0;
        painter.line_segment(
            [
                egui::pos2(rect.right() - off, rect.bottom()),
                egui::pos2(rect.right(), rect.bottom() - off),
            ],
            Stroke::new(1.2, col),
        );
    }
    if ui
        .interact(rect, ui.id().with("ov_grip"), Sense::click_and_drag())
        .drag_started()
    {
        state.enqueue_action(WinId::Overlay, WinAction::ResizeSouthEast);
    }
}
