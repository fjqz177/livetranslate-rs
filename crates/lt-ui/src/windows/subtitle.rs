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
//! - 行级 font_family 走全局 CJK 字体栈（宿主仅装载 msyh/simhei）；
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
use egui::{Align2, Color32, FontId, Sense, Ui};
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
    let (drag, height_cmd, height_settled) = {
        // 借用拆分：不相交字段
        let sm = &state.settings.subtitle_mode;
        let sub = &mut state.subtitle;

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
            let font = FontId::proportional(pt(cfg.font_size));
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

        // 7) 全窗中键拖动（原版 mousePressEvent MiddleButton；全窗穿透开启时输入不会到达）
        let drag = ui
            .interact(full, ui.id().with("sub_drag"), Sense::click_and_drag())
            .drag_started_by(egui::PointerButton::Middle);

        // 撑满布局（窗口即内容尺寸）
        ui.allocate_space(full.size());
        (drag, height_cmd, height_settled)
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
}
