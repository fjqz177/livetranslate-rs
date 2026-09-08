//! 字幕窗全量复刻（对照原版 subtitle_window.py + subtitle_text_widget.py + subtitle_config.py）：
//!
//! - 多行布局：settings.subtitle_mode.lines 启用行序贯渲染"原文行/译文行"，
//!   多句以 " | " 连接（原版 _rebuild_text_widgets / _refresh_display）；
//! - 描边大字：8 方向偏移描边色一遍 + 中心填充一遍（原版 QPainterPath
//!   stroke+fill 两遍绘制的 egui 等价）；文字/宽度/字号/描边不变时按缓存 key
//!   跳过重换行（原版 _text_cache 语义）；
//! - 贪心换行：按可用宽度逐字符累计 + 标点/空格优先断点（原版 split_text）；
//! - 自动隐藏：N 秒无新文本淡出（_auto_hide_timer）+ 1500ms 最小显示排队
//!   （_insert_sentence / _pending_segment_timers）+ 新句恢复
//!   （_restore_from_auto_hide，透明度 0→1 淡入）；
//! - 高度自适应：目标高度计算 + 150ms OutCubic 动画（_calc_target_height /
//!   _fit_height_animated），经 [`WinAction::SetSubtitleHeight`] 由宿主改窗；
//! - 中键拖动（原版 mousePressEvent MiddleButton）→ [`WinAction::DragSubtitle`]；
//! - 背景三态：bg_opacity=0 全透明 / >0 半透明实底 + 圆角（_apply_background）。
//!
//! 已知偏差（详见最终报告）：
//! - bg_image / 行级 bg_image 贴图未渲染（需图片解码与纹理管理；配置键保留）；
//! - 行级 font_family 已按 D-17 级联真实生效（空串=跟随 subtitle_font_family；
//!   未注册族名回落全局链，内嵌思源兜底）；
//! - 描边为 8 方向偏移近似（原版为圆头描边、宽 2×outline_width）；
//! - auto_hide_animation 的 slide_down 以 fade 近似（设置面板仅暴露 none/fade/slide_down）；
//! - 翻译失败文案会进入字幕（原版 pipeline 在 error 路径不调 update_text；
//!   本版事件流 UpdateTranslation 无法区分错误与成功文本）；
//! - 多屏判定用 winit 全显示器尺寸（原版 availableGeometry 剔除任务栏）；
//! - 行级 entry/exit 文本切换动画未做（默认 none，不在 M4.2 清单）。

use crate::state::{
    AppState, Easing, EaseAnim, SubtitleLineKey, SubtitleLineRender, SubtitleUiState, WinAction,
    WinId,
};
use crate::style::parse_color;
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui};
use lt_proto::SubtitleLine;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// 内容区左右边距（原版 _content_layout.setContentsMargins(16, 8, 16, 8)）
const MARGIN_H: f32 = 16.0;
const MARGIN_V: f32 = 8.0;
/// 高度动画时长（原版 _fit_height_animated setDuration(150)）
const HEIGHT_ANIM_MS: u64 = 150;
/// 位置可见性判定边距（原版 _is_pos_visible margin=50）
const POS_MARGIN: i32 = 50;
/// 顶条高度（逻辑 px；D-36 分区穿透的豁免区/拖动柄/悬停工具条高度）
pub const STRIP_H: f32 = 18.0;
/// 顶条绘制圆角
const STRIP_RADIUS: f32 = 4.0;
/// 断点优先字符集（原版 split_text 的 `" ,，。、!！?？;；:：."`）
const BREAK_CHARS: &[char] = &[' ', ',', '，', '。', '、', '!', '！', '?', '？', ';', '；', ':', '：', '.'];
/// 描边偏移方向（原版圆头描边的 8 方向近似）
const OUTLINE_DIRS: [(f32, f32); 8] = [
    (0.0, -1.0),
    (0.0, 1.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, -1.0),
    (1.0, -1.0),
    (-1.0, 1.0),
    (1.0, 1.0),
];

/// 字幕窗光标分区（D-36：分区穿透——顶条豁免、正文穿透的依据）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleZone {
    /// 顶条（悬停工具条/拖动柄/按钮命中区；恒非穿透）
    Strip,
    /// 正文（穿透开启时挂 WS_EX_TRANSPARENT）
    Body,
    /// 窗外（无操作）
    Outside,
}

/// 光标在窗内坐标 (逻辑 px, 窗左上为原点) → 分区（原版无对位：Rust 产品化新增）
pub fn zone_for_cursor(local_x: f32, local_y: f32, win_w: f32, win_h: f32, strip_h: f32) -> SubtitleZone {
    if local_x < 0.0 || local_y < 0.0 || local_x >= win_w || local_y >= win_h {
        return SubtitleZone::Outside;
    }
    if local_y < strip_h { SubtitleZone::Strip } else { SubtitleZone::Body }
}

/// 穿透位目标值：正文穿透 + 顶条豁免 + Ctrl 临时恢复（全部区域解穿透）
pub fn transparent_desired(enabled: bool, zone: SubtitleZone, ctrl: bool) -> bool {
    enabled && zone == SubtitleZone::Body && !ctrl
}

/// pt → 逻辑 px（Qt 1pt ≈ 96/72 px，与 overlay::pt 一致）
fn pt(size: u32) -> f32 {
    size as f32 * 4.0 / 3.0
}

/// #RRGGBB → 指定 alpha 的预乘色（原版 QColor::setAlpha / hex_to_rgba 等价换算）
fn with_alpha(hex: &str, alpha: u32) -> Color32 {
    let c = parse_color(hex, Color32::WHITE);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha.min(255) as u8)
}

/// 文本自然宽度（原版 QFontMetrics.horizontalAdvance 对位；
/// egui 排版带 memoize，贪心换行的重复前缀测量开销可接受）
fn text_width(ui: &Ui, font: &FontId, s: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE)
        .rect
        .width()
}

/// 单行行高（原版 fm.lineSpacing 对位：单行 galley 高，含上下伸展）
fn line_height(ui: &Ui, font: &FontId) -> f32 {
    let g = ui.painter().layout_no_wrap("Ag中文字符，example".to_owned(), font.clone(), Color32::WHITE);
    g.rect.height().max(1.0)
}

// ── 纯逻辑：贪心换行 / 高度计算 / 多屏钳制 / 行文本选取 ──

/// 贪心换行（原版 subtitle_text_widget.split_text 逐字等价）：
/// 整段放得下直接返回；否则取最大可容前缀，再在后半段自近及远回扫断点字符，
/// 段尾 rstrip、续段 lstrip；best=0（单字超宽）强制 1 字符推进。
pub fn wrap_greedy(text: &str, avail_w: f32, advance: &dyn Fn(&str) -> f32) -> Vec<String> {
    // 原版语义：split_text 不拦空串（_rewrap 拦），空串 advance=0 ≤ avail → [""]
    if advance(text) <= avail_w {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut segments: Vec<String> = Vec::new();
    let mut pos = 0usize; // 剩余段起点（chars 下标）
    loop {
        let rest: String = chars[pos..].iter().collect();
        if advance(&rest) <= avail_w {
            segments.push(rest);
            break;
        }
        // 最大可容前缀长度（原版 for i in range(1, len(text)+1)）
        let mut best = 0usize;
        for i in pos + 1..=n {
            let prefix: String = chars[pos..i].iter().collect();
            if advance(&prefix) > avail_w {
                break;
            }
            best = i - pos;
        }
        if best == 0 {
            best = 1;
        }
        // 标点优先断点：自 best-1 回扫至 best/2（原版 range(best-1, max(best//2, 0), -1)）
        let mut break_at = best;
        let stop = best / 2;
        let mut j = best.saturating_sub(1);
        while j > stop {
            if BREAK_CHARS.contains(&chars[pos + j]) {
                break_at = j + 1;
                break;
            }
            j -= 1;
        }
        // 段尾 rstrip、续段 lstrip（原版 .rstrip()/.lstrip()）
        let seg: String = chars[pos..pos + break_at].iter().collect();
        segments.push(seg.trim_end().to_string());
        pos += break_at;
        while pos < n && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos >= n {
            break;
        }
    }
    segments
}

/// 单行块期望高度（原版 desired_height：lineSpacing×max(行数,1) + 描边×2 + 4）
pub fn desired_height(line_h: f32, n_wrapped: usize, outline_w: f32) -> f32 {
    line_h * n_wrapped.max(1) as f32 + outline_w * 2.0 + 4.0
}

/// 窗口目标高度（原版 _calc_target_height：上下边距 + Σ行高 + 行间距×(行数-1)，下限 20）
pub fn calc_target_height(block_heights: &[f32], line_spacing: f32, margin_v: f32) -> f32 {
    let mut total = margin_v * 2.0;
    for (i, h) in block_heights.iter().enumerate() {
        total += h;
        if i > 0 {
            total += line_spacing;
        }
    }
    total.max(20.0)
}

/// 显示器矩形（逻辑 px；原版 QApplication screen availableGeometry 的最小对位，
/// winit 无工作区概念 → 全显示器尺寸）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonoRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl MonoRect {
    /// Qt QRect.right()/bottom() 语义：left+width-1 / top+height-1
    fn right(&self) -> i32 {
        self.x + self.w - 1
    }
    fn bottom(&self) -> i32 {
        self.y + self.h - 1
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        self.x <= x && x < self.right() && self.y <= y && y < self.bottom()
    }
}

/// 原版 _is_pos_visible：任一显示器满足 (left ≤ x+50) 且 x 在区间内即算可见
pub fn is_pos_visible(x: i32, y: i32, monitors: &[MonoRect]) -> bool {
    monitors
        .iter()
        .any(|g| g.x <= x + POS_MARGIN && x < g.right() && g.y <= y + POS_MARGIN && y < g.bottom())
}

/// 原版 _clamp_to_screen：含点显示器优先（否则首块=主屏），钳入 right-width/bottom-height。
/// 用 max/min 组合（对齐原版写法）：窗比屏宽时 min 先取值、max 兜底到屏左/上沿，
/// 避免 i32::clamp 的 min>max panic（window_width 最大可配 3840）。
pub fn clamp_to_screen(x: i32, y: i32, w: i32, h: i32, monitors: &[MonoRect]) -> (i32, i32) {
    let Some(screen) = monitors.iter().find(|g| g.contains(x, y)).or_else(|| monitors.first()) else {
        return (x, y);
    };
    let nx = screen.x.max(x.min(screen.right() - w));
    let ny = screen.y.max(y.min(screen.bottom() - h));
    (nx, ny)
}

/// 避让安全边距（逻辑 px；D-36：字幕窗让位时与悬浮窗保持的距离）
pub const OVERLAP_SAFETY: i32 = 24;

/// 矩形相交（x,y,w,h 表示）；边缘相切不算相交（< 而非 <=）
pub fn rects_intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32), safety: i32) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw + safety
        && bx < ax + aw + safety
        && ay < by + bh + safety
        && by < ay + ah + safety
}

/// 重叠避让（D-36）：把 sub 沿四个方向推出 ov（加安全边距）再钳屏，选移动成本
/// （曼哈顿距离）最小的合法方位；四向都被屏壁夹死（钳制后仍相交）→ None（保持
/// 现状，Z 序兜底——字幕窗恒压悬浮窗之下，主界面始终可操作）。
pub fn resolve_overlap(
    sub: (i32, i32, i32, i32),
    ov: (i32, i32, i32, i32),
    monitors: &[MonoRect],
    safety: i32,
) -> Option<(i32, i32)> {
    let (sx, sy, sw, sh) = sub;
    let (ox, oy, ow, oh) = ov;
    let candidates = [
        (sx, oy - sh - safety),          // 推出到上方
        (sx, oy + oh + safety),          // 推出到下方
        (ox - sw - safety, sy),          // 推出到左侧
        (ox + ow + safety, sy),          // 推出到右侧
    ];
    let mut best: Option<((i32, i32), i32)> = None;
    for (cx, cy) in candidates {
        let (nx, ny) = clamp_to_screen(cx, cy, sw, sh, monitors);
        if rects_intersect((nx, ny, sw, sh), ov, 0) {
            continue; // 钳制后仍相交（屏壁夹死）→ 该方向不可用
        }
        let cost = (nx - sx).abs() + (ny - sy).abs();
        match &best {
            Some((_, c)) if *c <= cost => {}
            _ => best = Some(((nx, ny), cost)),
        }
    }
    best.map(|(p, _)| p)
}

/// 每句取译文（原版 _refresh_display 的 translation 分支逐字等价）：
/// 命中 lang → 其值（可为空串）；否则空键 "" 且非空；再否则首个非空值；全无 → None
pub fn pick_translation(lang: &str, tl: &BTreeMap<String, String>) -> Option<String> {
    if !lang.is_empty() {
        if let Some(v) = tl.get(lang) {
            return Some(v.clone());
        }
    }
    if let Some(v) = tl.get("") {
        if !v.is_empty() {
            return Some(v.clone());
        }
    }
    tl.values().find(|v| !v.is_empty()).cloned()
}

/// 多句拼接（原版 `" | ".join(texts) if len(texts) > 1 else (texts[0] if texts else "")`）
fn join_texts(texts: Vec<String>) -> String {
    if texts.len() > 1 {
        texts.join(" | ")
    } else {
        texts.into_iter().next().unwrap_or_default()
    }
}

/// 重排各行显示文本（原版 _refresh_display：启用行逐一取句拼接并失效换行缓存）
fn refresh_display(sub: &mut SubtitleUiState, lines: &[SubtitleLine]) {
    let enabled: Vec<&SubtitleLine> = lines.iter().filter(|l| l.enabled).collect();
    // 行配置数变化 → 重建渲染行（原版 _rebuild_text_widgets / apply_settings 重建）
    if sub.lines.len() != enabled.len() {
        sub.lines = vec![SubtitleLineRender::default(); enabled.len()];
    }
    if sub.sentences.is_empty() {
        for l in &mut sub.lines {
            l.text.clear();
            l.wrapped.clear();
            l.cache_key = None;
        }
        return;
    }
    for (wi, cfg) in enabled.iter().enumerate() {
        let Some(l) = sub.lines.get_mut(wi) else { break };
        let text = if cfg.line_type == "original" {
            let texts: Vec<String> = sub
                .sentences
                .iter()
                .filter(|s| !s.original.is_empty())
                .map(|s| s.original.clone())
                .collect();
            join_texts(texts)
        } else {
            let lang = cfg.lang.as_deref().unwrap_or("");
            let texts: Vec<String> =
                sub.sentences.iter().filter_map(|s| pick_translation(lang, &s.translations)).collect();
            join_texts(texts)
        };
        if text != l.text {
            l.text = text;
            l.cache_key = None; // 文字变化 → 换行缓存失效（原版 _text_cache = None）
        }
    }
}

// ── 入口 ──

pub fn subtitle_ui(ui: &mut Ui, state: &mut AppState) {
    let now = Instant::now();
    let full = ui.available_rect_before_wrap();

    // 帧内借用拆分（字幕配置只读 / 字幕状态可变），帧后统一投递动作
    let (drag, height_cmd, height_settled, through_toggle, lock_toggle, hide, open_settings, reset_pos) = {
        // 借用拆分：不相交字段
        let sm = &state.settings.subtitle_mode;
        let sub = &mut state.subtitle;
        let fonts = &state.fonts;
        let master = &state.settings.subtitle_font_family;

        // 1) 行配置数量对齐（原版 apply_settings 重建语义）
        let enabled: Vec<&SubtitleLine> = sm.lines.iter().filter(|l| l.enabled).collect();
        if sub.lines.len() != enabled.len() {
            sub.lines = vec![SubtitleLineRender::default(); enabled.len()];
            sub.display_dirty = true;
        }
        // 2) 句子/行文本变化 → _refresh_display
        if sub.display_dirty {
            refresh_display(sub, &sm.lines);
            sub.display_dirty = false;
        }
        // 3) 淡入/淡出推进（进行中持续重绘；结束帧收敛）
        if let Some(f) = &sub.fade {
            if f.current(now).is_none() {
                sub.fade = None;
            } else {
                ui.ctx().request_repaint();
            }
        }

        // 4) 度量与换行（缓存 key 不变则跳过重排，原版 _text_cache 语义）
        let mut block_heights: Vec<f32> = Vec::with_capacity(enabled.len());
        for (cfg, line) in enabled.iter().zip(&mut sub.lines) {
            // D-17 级联：行级空串=跟随字幕主字体；未注册族名回落全局链
            let font = FontId::new(
                pt(cfg.font_size),
                crate::fonts::font_family_for(
                    crate::fonts::resolve_family(&cfg.font_family, master),
                    fonts,
                ),
            );
            let lh = line_height(ui, &font);
            let ow = if cfg.outline_enabled { cfg.outline_width as f32 } else { 0.0 };
            // 原版 split_text：avail_w = widget 宽 - 描边×2（widget 宽 = 窗宽 - 左右边距）
            let avail_w = (full.width() - MARGIN_H * 2.0 - ow * 2.0).max(1.0);
            if !line.text.is_empty() {
                let key = SubtitleLineKey {
                    text: line.text.clone(),
                    avail_w: avail_w as u32,
                    font_size: cfg.font_size,
                    outline_enabled: cfg.outline_enabled,
                    outline_width: cfg.outline_width,
                };
                if line.cache_key.as_ref() != Some(&key) {
                    let font_ref = &font;
                    line.wrapped = wrap_greedy(&line.text, avail_w, &|s: &str| text_width(ui, font_ref, s));
                    line.cache_key = Some(key);
                }
            }
            block_heights.push(desired_height(lh, line.wrapped.len(), ow));
        }

        // 5) 高度自适应（原版 _calc_target_height + _fit_height_animated 150ms OutCubic）
        let target = calc_target_height(&block_heights, sm.line_spacing as f32, MARGIN_V);
        let mut height_cmd: Option<f32> = None;
        let mut height_settled = false;
        let anim_now = sub.height_anim.as_ref().and_then(|a| a.current(now));
        match anim_now {
            Some(v) => {
                sub.applied_height = v;
                height_cmd = Some(v);
                ui.ctx().request_repaint();
            }
            // 动画结束帧：落定终值（原版 on_finished → setFixedHeight + clamp + position_changed）
            None if sub.height_anim.is_some() => {
                let to = sub.height_anim.take().map(|a| a.to).unwrap_or_default();
                sub.applied_height = to;
                height_cmd = Some(to);
                height_settled = true;
            }
            // 首帧 / 精简动效：直接落位（原版 _fit_height_snap）
            None if sub.applied_height <= 0.0 || sub.reduce_motion => {
                if (sub.applied_height - target).abs() >= 1.0 {
                    sub.applied_height = target;
                    height_cmd = Some(target);
                    height_settled = true;
                }
            }
            // 目标变化超 1px：启动新动画
            None if (sub.applied_height - target).abs() >= 1.0 => {
                sub.height_anim = Some(EaseAnim {
                    from: sub.applied_height,
                    to: target,
                    start: now,
                    duration: Duration::from_millis(HEIGHT_ANIM_MS),
                    easing: Easing::OutCubic,
                });
                ui.ctx().request_repaint();
            }
            None => {}
        }

        // 6) 绘制：背景（原版 _apply_background；自动隐藏只淡文本不淡背景，1:1 保留）。
        // 整窗 alpha 由宿主 LWA_ALPHA（=bg_opacity，见 app.rs subtitle_layered_alpha）
        // 承担，这里画不透明色——半透明填充在品红键控清除区上会混出色偏
        if sm.bg_opacity > 0 {
            ui.painter().rect_filled(full, sm.border_radius as f32, parse_color(&sm.bg_color, Color32::BLACK));
        }
        let opacity = sub.current_opacity(now);
        let content_left = full.left() + MARGIN_H;
        let content_right = full.right() - MARGIN_H;
        let mut y = full.top() + MARGIN_V;
        for (i, (cfg, line)) in enabled.iter().zip(&sub.lines).enumerate() {
            let font = FontId::proportional(pt(cfg.font_size));
            let lh = line_height(ui, &font);
            let ow = if cfg.outline_enabled { cfg.outline_width as f32 } else { 0.0 };
            if !line.text.is_empty() {
                // 行不透明度 = 配置 opacity × 窗级淡入淡出系数（原版 setAlpha × painter opacity）
                let k = (cfg.opacity.min(255) as f32 / 255.0 * opacity).clamp(0.0, 1.0);
                let fill = with_alpha(&cfg.color, (k * 255.0).round() as u32);
                let outline_col = with_alpha(&cfg.outline_color, (k * 255.0).round() as u32);
                let mut ry = y + ow;
                for row_text in &line.wrapped {
                    let tw = text_width(ui, &font, row_text);
                    // 水平对齐（原版 _render_text_pixmap：center/right/left，right/left 留描边位）
                    let lx = match cfg.align.as_str() {
                        "right" => content_right - ow - tw,
                        "left" => content_left + ow,
                        _ => (content_left + content_right - tw) * 0.5,
                    };
                    let pos = egui::pos2(lx, ry);
                    // 两遍绘制之一：8 方向偏移描边（原版 QPainterPath 圆头描边宽 2×ow 的近似）
                    if cfg.outline_enabled && cfg.outline_width > 0 {
                        let r = cfg.outline_width as f32;
                        for (dx, dy) in OUTLINE_DIRS {
                            ui.painter().text(
                                pos + egui::vec2(dx * r, dy * r),
                                Align2::LEFT_TOP,
                                row_text.as_str(),
                                font.clone(),
                                outline_col,
                            );
                        }
                    }
                    // 两遍绘制之二：中心填充
                    ui.painter().text(pos, Align2::LEFT_TOP, row_text.as_str(), font.clone(), fill);
                    ry += lh;
                }
            }
            y += desired_height(lh, line.wrapped.len(), ow);
            if i + 1 < enabled.len() {
                y += sm.line_spacing as f32;
            }
        }

        // 8) 顶条淡入/淡出推进（进行中持续重绘；结束帧收敛终态）
        if let Some(a) = &sub.toolbar_anim {
            if a.current(now).is_none() {
                sub.toolbar_anim = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        let strip_alpha = sub.toolbar_opacity(now);

        // 9) 交互区（D-36）：正文区中键拖动（原版全窗中键语义保留给正文）；
        //    顶条 = 任意键拖动 + 右键菜单 + 三按钮（按钮在此后注册 = 同层命中优先）
        let strip_rect = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), STRIP_H));
        let body_rect = egui::Rect::from_min_max(
            egui::pos2(full.left(), full.top() + STRIP_H),
            full.right_bottom(),
        );
        let mut drag = ui
            .interact(body_rect, ui.id().with("sub_body_drag"), Sense::click_and_drag())
            .drag_started_by(egui::PointerButton::Middle);

        let mut through_toggle = false;
        let mut lock_toggle = false;
        let mut hide = false;
        let mut open_settings = false;
        let mut reset_pos = false;
        let strip_resp = ui.interact(strip_rect, ui.id().with("sub_strip"), Sense::click_and_drag());
        if strip_resp.drag_started() && !sub.locked {
            drag = true;
        }
        // 顶条右键菜单（全部走既有 action 通路禁模态；D-33 纪律）
        strip_resp.context_menu(|ui| {
            if ui.button(lt_i18n::t("subwin_menu_open_settings")).clicked() {
                open_settings = true;
                ui.close();
            }
            if ui.button(lt_i18n::t("subwin_menu_reset_pos")).clicked() {
                reset_pos = true;
                ui.close();
            }
            ui.separator();
            if ui.button(lt_i18n::t("subwin_menu_hide")).clicked() {
                hide = true;
                ui.close();
            }
        });

        // 10) 顶条绘制（D-36：覆盖式——浮现/隐没不动文字布局；无布局参与）
        if strip_alpha > 0.02 {
            let bg = Color32::from_rgba_unmultiplied(0x26, 0x26, 0x2e, (210.0 * strip_alpha).round() as u8);
            ui.painter()
                .rect_filled(strip_rect, egui::CornerRadius::same(STRIP_RADIUS as u8), bg);
            // 右侧按钮组（右→左：关闭 / 锁定 / 穿透；宽度按文案测宽）
            let font = FontId::proportional(9.5);
            let mut bx = full.right() - 8.0;
            for (i, (label, tip, on)) in [
                (lt_i18n::t("subwin_tb_close"), lt_i18n::t("subwin_tb_close_hint"), false),
                (lt_i18n::t("subwin_tb_lock"), lt_i18n::t("subwin_tb_lock_hint"), sub.locked),
                (lt_i18n::t("subwin_tb_through"), lt_i18n::t("subwin_tb_through_hint"), sm.click_through),
            ]
            .into_iter()
            .enumerate()
            {
                let w = text_width(ui, &font, &label) + 14.0;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(bx - w, full.top() + 2.0),
                    egui::vec2(w, 14.0),
                );
                bx -= w + 4.0;
                let (fill, stroke, fg) = strip_button_colors(on);
                let resp = ui.put(
                    rect,
                    egui::Button::new(RichText::new(label).size(9.5).color(fg))
                        .fill(fill)
                        .stroke(Stroke::new(1.0, stroke))
                        .corner_radius(3.0),
                );
                if resp.clicked() {
                    match i {
                        0 => hide = true,
                        1 => lock_toggle = true,
                        _ => through_toggle = true,
                    }
                }
                resp.on_hover_text(tip);
            }
            // 左端 ☰ 拖动提示图标（纯提示，不响应；区带交互由 strip_resp 承担）
            ui.painter().text(
                strip_rect.left_center() + egui::vec2(6.0, 0.0),
                Align2::LEFT_CENTER,
                "\u{2630}",
                FontId::proportional(12.0),
                Color32::from_rgba_unmultiplied(0xaa, 0xaa, 0xaa, (255.0 * strip_alpha).round() as u8),
            );
        }
        strip_resp.on_hover_text(lt_i18n::t("subwin_tb_strip_hint"));

        // 撑满布局（窗口即内容尺寸）
        ui.allocate_space(full.size());
        (drag, height_cmd, height_settled, through_toggle, lock_toggle, hide, open_settings, reset_pos)
    };

    // 帧后动作
    if let Some(h) = height_cmd {
        state.enqueue_action(WinId::Subtitle, WinAction::SetSubtitleHeight(h));
    }
    if height_settled {
        state.enqueue_action(WinId::Subtitle, WinAction::ClampSubtitlePos);
    }
    if drag {
        state.enqueue_action(WinId::Subtitle, WinAction::DragSubtitle);
    }
    if through_toggle {
        let ct = !state.settings.subtitle_mode.click_through;
        state.settings.subtitle_mode.click_through = ct;
        crate::windows::panel::mark_settings_dirty(state);
        if ct && *state.visible.get(&WinId::Subtitle).unwrap_or(&false) {
            state.schedule_subtitle_window_poll();
        }
    }
    if lock_toggle {
        state.subtitle.locked = !state.subtitle.locked;
    }
    if hide {
        // 与悬浮窗"字幕"按钮同路径：enabled 翻转 + ToggleSubtitle
        state.settings.subtitle_mode.enabled = false;
        state.enqueue_action(WinId::Subtitle, WinAction::ToggleSubtitle);
    }
    if open_settings {
        state.panel.page = crate::state::PanelPage::Subtitle;
        state.enqueue_action(WinId::Panel, WinAction::ShowPanel);
    }
    if reset_pos {
        state.enqueue_action(WinId::Subtitle, WinAction::ResetSubtitlePos);
    }
}

/// 顶条按钮三态色（开=绿系（对齐悬浮窗字幕钮 SUBTITLE_ON_* 语义），关=中性）
fn strip_button_colors(on: bool) -> (Color32, Color32, Color32) {
    if on {
        (
            Color32::from_rgba_premultiplied(13, 40, 20, 160),
            Color32::from_rgba_premultiplied(40, 100, 50, 200),
            Color32::from_rgb(0x9f, 0xd8, 0x9f),
        )
    } else {
        (
            Color32::from_rgba_premultiplied(34, 34, 40, 120),
            Color32::from_rgba_premultiplied(70, 70, 80, 140),
            Color32::from_rgb(0xaa, 0xaa, 0xaa),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SubtitleSentence;

    /// 假想测宽器：CJK（>0x2E80）=10px、其余=5px，换行用例的确定性基准
    fn fake_advance(s: &str) -> f32 {
        s.chars().map(|c| if c as u32 > 0x2E80 { 10.0 } else { 5.0 }).sum()
    }

    fn sentence(original: &str, pairs: &[(&str, &str)]) -> SubtitleSentence {
        SubtitleSentence {
            original: original.into(),
            translations: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    /// 句子排队用 cfg：句数上限 + 自动隐藏秒数（其余字段走默认）
    fn sub_cfg(sentences: u32, auto_hide_timeout: u32) -> lt_proto::SubtitleMode {
        lt_proto::SubtitleMode { sentences, auto_hide_timeout, ..Default::default() }
    }

    fn insert(sub: &mut SubtitleUiState, s: SubtitleSentence, sm: &lt_proto::SubtitleMode, now: Instant) {
        sub.insert_sentence(s, sm, now);
    }

    // ── 换行（原版 split_text 用例）──

    #[test]
    fn wrap_fits_single_line() {
        // 整段放得下 → 原样单段（原版 horizontalAdvance(text) <= avail_w 早退）
        let segs = wrap_greedy("hello", 30.0, &fake_advance);
        assert_eq!(segs, vec!["hello"]);
    }

    #[test]
    fn wrap_cjk_overflows_greedy() {
        // 8 个 CJK = 80px，可用 35px → 每段至多 3 字
        let segs = wrap_greedy("中中中中中中中中", 35.0, &fake_advance);
        assert_eq!(segs, vec!["中中中", "中中中", "中中"]);
        for s in &segs {
            assert!(fake_advance(s) <= 35.0);
        }
    }

    #[test]
    fn wrap_prefers_punctuation_boundary() {
        // best=10（"abcdef,ghi"），回扫命中 j=6 的逗号 → 断在 "abcdef,"
        let segs = wrap_greedy("abcdef,ghijkl", 50.0, &fake_advance);
        assert_eq!(segs, vec!["abcdef,", "ghijkl"]);
    }

    #[test]
    fn wrap_strips_whitespace_at_break() {
        // 断点命中空格：段尾 rstrip 去掉尾随空格（原版 .rstrip()）
        let segs = wrap_greedy("aaaa bb cccccc", 35.0, &fake_advance);
        assert_eq!(segs, vec!["aaaa", "bb cccc", "cc"]);
        // 无断点命中：续段首空格 lstrip（原版 .lstrip()）
        let segs = wrap_greedy("aaaaaa bbbbbb", 30.0, &fake_advance);
        assert_eq!(segs, vec!["aaaaaa", "bbbbbb"]);
    }

    #[test]
    fn wrap_single_char_overflow_forces_progress() {
        // 单字超宽（best=0 → 强制 1 字符），原版死循环防线
        let segs = wrap_greedy("中文", 5.0, &fake_advance);
        assert_eq!(segs, vec!["中", "文"]);
    }

    #[test]
    fn wrap_mixed_cjk_ascii_accumulates_by_width() {
        // 混排按字符宽度累计：5 ASCII(25) + 1 CJK(10) = 35 ≤ 35，再加 1 CJK 超宽
        let segs = wrap_greedy("abcde中文", 35.0, &fake_advance);
        assert_eq!(segs, vec!["abcde中", "文"]);
    }

    // ── 高度（原版 desired_height / _calc_target_height 用例）──

    #[test]
    fn desired_and_target_height_match_original_formula() {
        // desired = lineSpacing×max(行数,1) + 描边×2 + 4；空文本按 1 行计
        assert_eq!(desired_height(32.0, 0, 2.0), 40.0);
        assert_eq!(desired_height(38.0, 2, 2.0), 84.0);
        // target = 上下边距 16 + Σ行高 + 行间距×(n-1)，两行默认配置
        assert_eq!(calc_target_height(&[40.0, 46.0], 8.0, 8.0), 110.0);
        // 无启用行 → 下限 20（原版 max(total, 20)）
        assert_eq!(calc_target_height(&[], 8.0, 8.0), 20.0);
    }

    // ── 句子排队时序（原版 _on_update_text / _insert_sentence / _pending_segment_timers 用例）──

    #[test]
    fn first_update_inserts_immediately_and_schedules_auto_hide() {
        let mut sub = SubtitleUiState::default();
        let t0 = Instant::now();
        // 首句 last_insert=0 → base_delay=0 → 立即插入
        let pending = sub.update_text("你好".into(), BTreeMap::new(), &sub_cfg(1, 5), t0);
        assert!(pending.is_none());
        assert_eq!(sub.sentences.len(), 1);
        assert!(sub.display_dirty);
        // timeout=5s 且有句子 → 计时（原版 _restart_auto_hide_timer）
        assert_eq!(sub.auto_hide_deadline, Some(t0 + Duration::from_secs(5)));
        assert_eq!(sub.last_insert, Some(t0));
    }

    #[test]
    fn rapid_update_queued_until_min_display_elapsed() {
        let mut sub = SubtitleUiState::default();
        let cfg1 = sub_cfg(1, 0);
        let t0 = Instant::now();
        sub.update_text("第一句".into(), BTreeMap::new(), &cfg1, t0);
        // 500ms 后的更新：距 1500ms 最小显示还差 1000ms → 进 pending 队列
        let t1 = t0 + Duration::from_millis(500);
        let pending = sub.update_text("第二句".into(), BTreeMap::new(), &cfg1, t1);
        assert_eq!(pending, Some(t1 + Duration::from_millis(1000)));
        assert_eq!(sub.sentences.len(), 1, "排队期不插入");
        // 到点消费（原版 timer.timeout → _insert_sentence）；max=1 → 截断只留最新
        let t2 = t1 + Duration::from_millis(1000);
        assert!(sub.flush_pending(&cfg1, t2));
        assert_eq!(sub.sentences.len(), 1);
        assert_eq!(sub.sentences[0].original, "第二句");
        assert_eq!(sub.last_insert, Some(t2));
    }

    #[test]
    fn newer_update_cancels_pending_segment() {
        // 原版 _cancel_pending_segments：新更新永远取代旧待插入（至多 1 个存活）
        let mut sub = SubtitleUiState::default();
        let cfg2 = sub_cfg(2, 0);
        let t0 = Instant::now();
        sub.update_text("第一句".into(), BTreeMap::new(), &cfg2, t0);
        let t1 = t0 + Duration::from_millis(200);
        sub.update_text("第二句".into(), BTreeMap::new(), &cfg2, t1);
        let t2 = t0 + Duration::from_millis(400);
        sub.update_text("第三句".into(), BTreeMap::new(), &cfg2, t2);
        assert_eq!(sub.pending.as_ref().unwrap().1.original, "第三句");
        // 旧第二句的到点时刻（t1+1300 = t0+1500）之前不插入，且待插入仍是第三句
        assert!(!sub.flush_pending(&cfg2, t0 + Duration::from_millis(1000)));
        // 新第三句按自身时刻（t2+1100 = t0+1500）到点插入
        assert!(sub.flush_pending(&cfg2, t2 + Duration::from_millis(1100)));
        let originals: Vec<&str> = sub.sentences.iter().map(|s| s.original.as_str()).collect();
        assert_eq!(originals, vec!["第一句", "第三句"]);
    }

    #[test]
    fn sentences_trim_keeps_newest() {
        // max=2：插入 3 句保留最后 2 句（原版 _sentences[-max_sentences:]）
        let mut sub = SubtitleUiState::default();
        let cfg2 = sub_cfg(2, 0);
        let t0 = Instant::now();
        insert(&mut sub, sentence("a", &[]), &cfg2, t0);
        insert(&mut sub, sentence("b", &[]), &cfg2, t0 + Duration::from_millis(2000));
        insert(&mut sub, sentence("c", &[]), &cfg2, t0 + Duration::from_millis(4000));
        let originals: Vec<&str> = sub.sentences.iter().map(|s| s.original.as_str()).collect();
        assert_eq!(originals, vec!["b", "c"]);
    }

    // ── 自动隐藏（原版 _auto_hide_timer / _restore_from_auto_hide 用例）──

    #[test]
    fn auto_hide_fades_out_and_new_sentence_restores() {
        let mut sub = SubtitleUiState::default();
        let t0 = Instant::now();
        insert(&mut sub, sentence("a", &[]), &sub_cfg(1, 5), t0);
        // timeout=0 → 不计时
        insert(&mut sub, sentence("b", &[]), &sub_cfg(1, 0), t0 + Duration::from_millis(2000));
        assert_eq!(sub.auto_hide_deadline, None);

        // 到点隐藏：置位 + 启动淡出（fade：InCubic → 0，起点为当前不透明度 1.0）
        insert(&mut sub, sentence("c", &[]), &sub_cfg(1, 5), t0 + Duration::from_millis(4000));
        let deadline = sub.auto_hide_deadline.unwrap();
        assert!(sub.on_auto_hide_timeout("fade", 300, deadline));
        assert!(sub.hidden_by_timeout);
        assert!(!sub.on_auto_hide_timeout("fade", 300, deadline), "重复触发无害");
        assert_eq!(sub.current_opacity(deadline), 1.0, "淡出起点不透明");
        // 淡出中点介于 0 与 1 之间
        let mid = deadline + Duration::from_millis(150);
        let o = sub.current_opacity(mid);
        assert!((0.0..1.0).contains(&o));

        // 新句到达 → 恢复（原版 _insert_sentence → _restore_from_auto_hide：0→1 淡入）
        insert(&mut sub, sentence("d", &[]), &sub_cfg(1, 5), deadline + Duration::from_millis(1000));
        assert!(!sub.hidden_by_timeout);
        let o = sub.current_opacity(deadline + Duration::from_millis(1150));
        assert!(o > 0.0 && o < 1.0, "淡入进行中: {o}");
        // none 动画：瞬时切换
        assert!(sub.on_auto_hide_timeout("none", 300, deadline + Duration::from_millis(2000)));
        assert_eq!(sub.current_opacity(deadline + Duration::from_millis(2000)), 0.0);
    }

    #[test]
    fn clear_resets_queue_and_visibility() {
        let mut sub = SubtitleUiState::default();
        let t0 = Instant::now();
        insert(&mut sub, sentence("a", &[]), &sub_cfg(1, 5), t0);
        let deadline = sub.auto_hide_deadline.unwrap();
        sub.on_auto_hide_timeout("fade", 300, deadline);
        sub.clear();
        assert!(sub.sentences.is_empty());
        assert!(!sub.hidden_by_timeout);
        assert_eq!(sub.auto_hide_deadline, None);
        assert!(sub.lines.iter().all(|l| l.text.is_empty()));
    }

    // ── 行文本选取（原版 _refresh_display 用例）──

    #[test]
    fn pick_translation_fallback_chain() {
        let mut tl = BTreeMap::new();
        tl.insert("zh".to_string(), "中文".to_string());
        tl.insert("en".to_string(), String::new());
        tl.insert("".to_string(), "fallback".to_string());
        // 命中 lang → 其值（即使空串也命中，原版 `if lang and lang in tl_dict`）
        assert_eq!(pick_translation("zh", &tl).as_deref(), Some("中文"));
        assert_eq!(pick_translation("en", &tl).as_deref(), Some(""));
        // 未命中 lang → 空键 "" 非空值
        assert_eq!(pick_translation("ja", &tl).as_deref(), Some("fallback"));
        // 空键也为空 → 首个非空值
        let mut tl2 = BTreeMap::new();
        tl2.insert("".to_string(), String::new());
        tl2.insert("ko".to_string(), "한국어".to_string());
        assert_eq!(pick_translation("ja", &tl2).as_deref(), Some("한국어"));
        // 全空 → None（该句不拼入）
        assert_eq!(pick_translation("ja", &BTreeMap::new()), None);
    }

    #[test]
    fn refresh_display_joins_sentences_per_line_type() {
        let mut sub = SubtitleUiState {
            sentences: vec![
                sentence("hello", &[("zh", "你好")]),
                sentence("world", &[("zh", "世界")]),
            ],
            ..Default::default()
        };
        let lines = vec![
            SubtitleLine { line_type: "original".into(), ..Default::default() },
            SubtitleLine { line_type: "translation".into(), lang: Some("zh".into()), ..Default::default() },
            SubtitleLine { line_type: "translation".into(), lang: Some("ja".into()), ..Default::default() },
        ];
        refresh_display(&mut sub, &lines);
        // 原文行：多句 " | " 连接（空原文不计）
        assert_eq!(sub.lines[0].text, "hello | world");
        // 译文行：逐句按 lang 取值后连接
        assert_eq!(sub.lines[1].text, "你好 | 世界");
        // 无该语言 → 逐句回退空键/首非空值（原版兜底分支）
        assert_eq!(sub.lines[2].text, "你好 | 世界");
        // 换行缓存已失效待重排
        assert!(sub.lines.iter().all(|l| l.cache_key.is_none()));
        // 空句子队列 → 全行清空
        sub.sentences.clear();
        refresh_display(&mut sub, &lines);
        assert!(sub.lines.iter().all(|l| l.text.is_empty() && l.wrapped.is_empty()));
    }

    // ── 多屏钳制（原版 _is_pos_visible / _clamp_to_screen 用例）──

    #[test]
    fn pos_visibility_uses_margin_semantics() {
        let monitors = vec![
            MonoRect { x: 0, y: 0, w: 1920, h: 1080 },
            MonoRect { x: 1920, y: 0, w: 1920, h: 1080 },
        ];
        // 正常可见 / margin 内贴边可见（x=-30 → x+50=20 ≥ left）
        assert!(is_pos_visible(500, 500, &monitors));
        assert!(is_pos_visible(-30, 0, &monitors));
        // 双屏之外不可见
        assert!(!is_pos_visible(5000, 500, &monitors));
        assert!(!is_pos_visible(500, 2000, &monitors));
    }

    #[test]
    fn clamp_moves_window_into_containing_monitor() {
        let monitors = vec![
            MonoRect { x: 0, y: 0, w: 1920, h: 1080 },
            MonoRect { x: 1920, y: 0, w: 1920, h: 1080 },
        ];
        // 原位可见 → 不动
        assert_eq!(clamp_to_screen(500, 500, 1000, 160, &monitors), (500, 500));
        // 含点第二屏 → 钳入 right-width（Qt right = left+width-1）
        assert_eq!(clamp_to_screen(3000, 500, 1000, 160, &monitors), (2839, 500));
        // 越下边 → 钳入 bottom-height
        assert_eq!(clamp_to_screen(500, 2000, 1000, 160, &monitors), (500, 919));
        // 无含点屏（screenAt 未命中）→ 落首屏（原版 primaryScreen 兜底）
        assert_eq!(clamp_to_screen(5000, 500, 1000, 160, &monitors), (919, 500));
        assert_eq!(clamp_to_screen(-100, -100, 1000, 160, &monitors), (0, 0));
        // 窗比屏宽：min/max 组合兜底到屏左/上沿（i32::clamp 会 panic 的边界）
        assert_eq!(clamp_to_screen(300, 200, 3840, 2160, &monitors), (0, 0));
    }

    // ── D-36 分区穿透（光标分区 + 穿透位目标矩阵）──

    #[test]
    fn zone_for_cursor_matrix() {
        // 窗内顶条（y < 18；x 覆盖整宽）
        assert_eq!(zone_for_cursor(10.0, 5.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Strip);
        assert_eq!(zone_for_cursor(999.5, 17.9, 1000.0, 160.0, STRIP_H), SubtitleZone::Strip);
        // 窗内正文（y >= 18）
        assert_eq!(zone_for_cursor(10.0, 18.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Body);
        assert_eq!(zone_for_cursor(500.0, 159.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Body);
        // 窗外（四缘 0.1px 越界）
        assert_eq!(zone_for_cursor(-0.1, 50.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Outside);
        assert_eq!(zone_for_cursor(50.0, -0.1, 1000.0, 160.0, STRIP_H), SubtitleZone::Outside);
        assert_eq!(zone_for_cursor(1000.0, 50.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Outside);
        assert_eq!(zone_for_cursor(50.0, 160.0, 1000.0, 160.0, STRIP_H), SubtitleZone::Outside);
    }

    #[test]
    fn transparent_desired_matrix() {
        // 关穿透：任何区域都不穿
        assert!(!transparent_desired(false, SubtitleZone::Strip, false));
        assert!(!transparent_desired(false, SubtitleZone::Body, false));
        // 开穿透：正文穿、顶条豁免、窗外无动作
        assert!(transparent_desired(true, SubtitleZone::Body, false));
        assert!(!transparent_desired(true, SubtitleZone::Strip, false));
        assert!(!transparent_desired(true, SubtitleZone::Outside, false));
        // Ctrl 临时恢复：全部区域解穿透（拖动可达）
        assert!(!transparent_desired(true, SubtitleZone::Body, true));
        assert!(!transparent_desired(true, SubtitleZone::Strip, true));
    }

    // ── D-36 重叠避让（矩形相交 + 四向推出选优 + 屏壁夹死兜底）──

    #[test]
    fn rects_intersect_boundaries() {
        // 相离
        assert!(!rects_intersect((0, 0, 100, 100), (200, 0, 100, 100), 0));
        // 边缘相切不算相交
        assert!(!rects_intersect((0, 0, 100, 100), (100, 100, 100, 100), 0));
        // 两轴间距均 < 安全边距 → 视为相交（避让触发带）
        assert!(rects_intersect((0, 0, 100, 100), (115, 118, 100, 100), OVERLAP_SAFETY));
        // y 轴间距 30 > 24 安全距 → 不触发
        assert!(!rects_intersect((0, 0, 100, 100), (115, 130, 100, 100), OVERLAP_SAFETY));
        // 部分重叠 / 包含
        assert!(rects_intersect((0, 0, 100, 100), (50, 50, 100, 100), 0));
        assert!(rects_intersect((0, 0, 300, 300), (50, 50, 100, 100), 0));
    }

    #[test]
    fn resolve_overlap_moves_shortest_direction() {
        let monitors = vec![MonoRect { x: 0, y: 0, w: 1920, h: 1080 }];
        // 悬浮窗在中央偏上，字幕窗在其右下：上方/下方/左/右四候选，
        // 右候选被钳屏拆回窗内而失效，成本最小 = 下移 → 推出到悬浮窗下方
        let sub = (900, 700, 800, 120);
        let ov = (760, 600, 500, 240);
        assert!(rects_intersect(sub, ov, 0));
        let (nx, ny) = resolve_overlap(sub, ov, &monitors, OVERLAP_SAFETY).unwrap();
        let nsub = (nx, ny, sub.2, sub.3);
        assert!(!rects_intersect(nsub, ov, 0), "推出后不得相交: {nsub:?}");
        assert_eq!(nx, sub.0, "x 不动");
        assert_eq!(ny, ov.1 + ov.3 + OVERLAP_SAFETY, "应推出到悬浮窗下方");
    }

    #[test]
    fn resolve_overlap_clamped_fallback() {
        let monitors = vec![MonoRect { x: 0, y: 0, w: 1920, h: 1080 }];
        // 字幕窗与悬浮窗都贴着主屏四周、窗高接近屏高 → 上下方向被屏壁夹死、左右亦然 → None
        let ov = (0, 0, 1920, 1080);
        let sub = (200, 100, 1600, 1000);
        assert_eq!(resolve_overlap(sub, ov, &monitors, 0), None);
        // 两侧有余量时：推出到上方（clamp 后与悬浮窗不交）
        let ov2 = (0, 300, 1920, 600);
        let sub2 = (100, 500, 800, 160);
        let (nx, ny) = resolve_overlap(sub2, ov2, &monitors, OVERLAP_SAFETY).unwrap();
        let nsub = (nx, ny, sub2.2, sub2.3);
        assert!(!rects_intersect(nsub, ov2, 0));
    }

    /// D-36 顶条 headless 冒烟：悬停态（含按钮/菜单路径）与隐藏+锁定态各跑两帧不 panic
    #[test]
    fn subtitle_ui_headless_smoke_with_toolbar() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppState::new(lt_proto::Settings::default());
        st.settings.subtitle_mode.click_through = true;
        st.settings.subtitle_mode.enabled = true;
        // 悬停中（源于 Win32 轮询注入——穿透开启时 egui 收不到输入，状态由宿主写入）
        st.subtitle.toolbar_hover = true;
        st.subtitle.toolbar_anim = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::windows::subtitle::subtitle_ui(ui, &mut st)
            });
            assert!(!out.shapes.is_empty(), "字幕窗（含顶条）应产出图元");
            out.textures_delta.clear();
        }
        // 隐藏态 + 锁定：不 panic、图元仍在（背景+文字）
        st.subtitle.toolbar_hover = false;
        st.subtitle.locked = true;
        let out = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::windows::subtitle::subtitle_ui(ui, &mut st)
        });
        assert!(!out.shapes.is_empty());
    }
}
