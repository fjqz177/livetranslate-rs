//! 控制面板（对照原版 control_panel.py 的实测行为，2026-09-06 走查对齐）：
//!
//! - 框架：顶部 QTabWidget 式 Tab 条（7 Tab：VAD/ASR、翻译、样式、字幕、
//!   基准测试、缓存、更新日志）+ 白色内容页 + 按需滚动（make_scroll_area）。
//! - 视觉：Windows 原生浅色（原版未做 QSS 换肤的 PyQt6 默认浅色控件），
//!   调色板 [`Palette::NATIVE`]；整体 visuals 由宿主在 Panel 帧前经
//!   [`panel_visuals`] 注入（悬浮窗等深色窗口不受影响——各窗口独立 egui pass）。
//! - 分组：QGroupBox 式直角细边框组框 + 嵌入标题（[`group_card`]）。
//! - 设置流（panel.py `_auto_save`）：控件直接改 `AppState.settings` 对应字段，
//!   300ms 防抖后经 [`lt_proto::Cmd::ApplySettings`] 整体重放（热应用 + 落盘，
//!   原版 settings_changed 语义）；引擎/模型/语言类即时命令不走防抖。
//!
//! 页实现：VAD/ASR [`vad`] / 翻译 [`translation`] / 样式 [`style`] /
//! 字幕 [`subtitle_page`] / 基准测试 [`benchmark_tab`] / 缓存 [`data`] /
//! 更新日志 [`changelog_tab`]。

pub mod benchmark_tab;
pub mod changelog_tab;
pub mod data;
pub mod font_picker;
pub mod log_tab;
pub mod style;
pub mod subtitle_page;
pub mod translation;
pub mod vad;

use crate::state::TickKind;
use crate::state::{
    BenchUi, LogUi, ModalUi, PanelPage, PanelUi, SessionView, Settings, UiContext, WinId,
};
use egui::{Color32, Frame, RichText, ScrollArea, Stroke, Ui};
use std::time::Instant;

/// Tab 条页签高度（原版 QTabBar 页签视觉高度）
const TAB_H: f32 = 26.0;

/// 面板最小窗宽（逻辑 px；D-120）。原版 480 照搬 Qt 布局器前提在 egui 不成立
/// （egui 横排无收缩换行，最小宽不构成「内容放得下」保证，G-39），故抬到
/// headless 实测「zh/en × 8 页零横向溢出」的下限 + 余量；定值法 = 溢出断言
/// 测试按候选宽迭代。app.rs 的 min_inner_size 与默认创建宽共用本常量。
pub const PANEL_MIN_WIDTH: f32 = 600.0;

/// Windows 原生浅色调色板（PyQt6 Windows 默认控件字面色，2026-09-06 实拍取色）
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 窗口底 / Tab 条区（#F0F0F0）
    pub window_bg: Color32,
    /// Tab 页内容区底（#FFFFFF）
    pub page_bg: Color32,
    /// 组框底（与页底同色，QGroupBox 默认透出页底）
    pub card: Color32,
    /// 组框/页签边框（QGroupBox 默认灰线）
    pub card_stroke: Color32,
    /// 主文本（近黑）
    pub text: Color32,
    /// 标题文本
    pub title: Color32,
    /// 弱化文本（禁用/提示）
    pub weak: Color32,
    /// 悬停底（Windows hover #E5F1FB）
    pub hover: Color32,
    /// 选中底（下拉高亮 #CCE8FF）
    pub selected: Color32,
    /// 强调（Windows 蓝 #0078D4：滑条/勾选/聚焦边框）
    pub accent: Color32,
    /// 状态色
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
}

impl Palette {
    /// 面板恒浅色（原版无主题切换，QTabWidget 原生浅色）
    pub fn resolve(_ui: &Ui) -> Self {
        Palette::NATIVE
    }

    /// Windows 原生浅色（原版截图实拍取色）
    pub const NATIVE: Palette = Palette {
        window_bg: Color32::from_rgb(0xF0, 0xF0, 0xF0),
        page_bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        card_stroke: Color32::from_rgb(0xC9, 0xC9, 0xC9),
        text: Color32::from_rgb(0x1A, 0x1A, 0x1A),
        title: Color32::from_rgb(0x1A, 0x1A, 0x1A),
        weak: Color32::from_rgb(0x6D, 0x6D, 0x6D),
        hover: Color32::from_rgb(0xE5, 0xF1, 0xFB),
        selected: Color32::from_rgb(0xCC, 0xE8, 0xFF),
        accent: Color32::from_rgb(0x00, 0x78, 0xD4),
        ok: Color32::from_rgb(0x10, 0x7C, 0x10),
        warn: Color32::from_rgb(0x9A, 0x67, 0x00),
        err: Color32::from_rgb(0xC4, 0x1E, 0x3A),
    };
}

/// 控件变更 → 300ms 防抖应用（原版 TabBase.auto_save 的面板侧统一入口）。
/// W5：跨域意图化——本函数可被字幕窗/确认模态/面板页任意调用，宿主在
/// about_to_wait 消费意图后再做面板域防抖登记（窗口边界由借用检查强制）。
pub fn mark_settings_dirty(session: &mut SessionView) {
    session.request_settings_apply();
}

/// 翻译页 prompt 600ms 防抖登记（原版 _prompt_debounce.start()：重启单发定时；
/// 面板域内部——prompt_apply_due 在面板状态、节拍在会话协调面）
pub fn schedule_prompt_apply(panel: &mut PanelUi, session: &mut SessionView, now: Instant) {
    panel.state.prompt_apply_due = Some(now + std::time::Duration::from_millis(600));
    let at = now + std::time::Duration::from_millis(600);
    session.schedule_tick(WinId::Panel, TickKind::PromptApply, at);
}

/// 引擎/模型变化（原版 settings_changed → main 侧 switch_engine 的即时路径）
pub fn send_switch_engine(settings: &Settings, session: &SessionView) {
    let s = settings;
    session.send_cmd(lt_proto::Cmd::SwitchEngine {
        engine: s.asr_engine.clone(),
        funasr_model: s.funasr_model.clone(),
        whisper_model_size: s.whisper_model_size.clone(),
        language: s.asr_language.clone(),
    });
}

/// 当前激活模型配置（active_model 越界时回退首行；sanitize 保证至少一个模型）
pub fn active_model_config(s: &Settings) -> Option<lt_proto::ModelConfig> {
    s.models
        .get(s.active_model)
        .or_else(|| s.models.first())
        .cloned()
}

// ── 打开目录/链接（原版 TabBase.open_path / QDesktopServices.openUrl）──

/// 用系统文件管理器打开目录（原版 open_path：Windows explorer / 其他 xdg-open）。
/// 失败仅记日志（按钮路径已 mkdir 兜底，失败面极窄）。
pub fn open_in_explorer(path: &std::path::Path) {
    #[cfg(windows)]
    let result = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(windows))]
    let result = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = result {
        tracing::warn!("打开目录失败 {}: {e}", path.display());
    }
}

/// 用系统默认浏览器打开 URL（原版 QDesktopServices.openUrl）
pub fn open_url(url: &str) {
    #[cfg(windows)]
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();
    #[cfg(not(windows))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = result {
        tracing::warn!("打开链接失败 {url}: {e}");
    }
}

// ── 框架布局 ──

// WD-6 引导横幅配色（D-105）：自含徽章式暖色，明暗主题下均可读；
// 不取 pal.err——缺模型引导是行动指引不是错误
const BANNER_FILL: Color32 = Color32::from_rgb(0xFF, 0xF4, 0xDE);
const BANNER_FILL_HOVER: Color32 = Color32::from_rgb(0xFF, 0xE9, 0xC6);
const BANNER_STROKE: Color32 = Color32::from_rgb(0xE6, 0xA2, 0x3C);
const BANNER_TEXT: Color32 = Color32::from_rgb(0x6B, 0x43, 0x0A);

/// 面板 UI 总入口（windows::dispatch 按 WinId::Panel 分派到这里；W5 起只拿
/// 面板窗口域面：panel/session/settings/modal/log/bench/ctx——不含兄弟窗口域）
#[allow(clippy::too_many_arguments)]
pub fn panel_ui(
    ui: &mut Ui,
    panel: &mut PanelUi,
    session: &mut SessionView,
    settings: &mut Settings,
    modal: &mut ModalUi,
    log: &mut LogUi,
    bench: &mut BenchUi,
    ctx: &mut UiContext,
) {
    let pal = Palette::resolve(ui);

    // 全窗底色（QTabWidget 外围 #F0F0F0）
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0.0, pal.window_bg);

    // Tab 条（原版 QTabWidget North 页签行）
    ui.add_space(4.0);
    tab_strip(ui, panel, &pal);

    // WD-6 首启缺模型轻引导（D-105）：识别未就绪 → 顶部高亮横幅，点击去识别页；
    // 就绪即隐、加载中不亮、识别页自身不亮（可见性判定见 PanelUi::model_banner_visible）
    if panel.model_banner_visible() {
        let label = lt_i18n::t("model_missing_banner");
        let font = egui::FontId::proportional(12.5);
        // D-120/G-39：按可用宽 wrap 预测行数定高——超宽自动折行，不再双侧裁；
        // 16.0 = 两侧各 8px 呼吸位，40.0 = 极窄兜底（wrap_width 须为正）
        let avail = ui.available_rect_before_wrap().width();
        let galley = ui.painter().layout(
            label.clone(),
            font.clone(),
            BANNER_TEXT,
            (avail - 16.0).max(40.0),
        );
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(avail, galley.size().y + 12.0),
            egui::Sense::click(),
        );
        let fill = if resp.hovered() {
            BANNER_FILL_HOVER
        } else {
            BANNER_FILL
        };
        ui.painter().rect_filled(rect, 3.0, fill);
        ui.painter().rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.0, BANNER_STROKE),
            egui::StrokeKind::Outside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() * 0.5, galley, BANNER_TEXT);
        if resp.clicked() {
            panel.state.page = PanelPage::VadAsr;
        }
        ui.add_space(4.0);
    }

    // Tab 页内容区：白底 + 灰边框（pane），内容按需滚动（make_scroll_area）
    let pane = ui.available_rect_before_wrap();
    ui.painter().rect_filled(pane, 0.0, pal.page_bg);
    ui.painter().rect_stroke(
        pane,
        0.0,
        Stroke::new(1.0, pal.card_stroke),
        egui::StrokeKind::Inside,
    );

    let page = panel.state.page;
    if page == PanelPage::Log {
        // 日志页自带滚动区（工具行置顶 + 单滚动条），不套页面级 ScrollArea：
        // 一旦内容（底部提示行等）超出 pane 高度会出现第二根滚动条（LT-1，
        // 见 docs/archive/log-tab-redesign.md）。
        Frame::NONE
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| log_tab::page(ui, log, &pal));
        return;
    }
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            Frame::NONE
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| match page {
                    PanelPage::VadAsr => vad::page(ui, panel, session, settings, &pal),
                    PanelPage::Translation => {
                        translation::page(ui, panel, session, settings, modal, &pal)
                    }
                    PanelPage::Style => style::page(ui, session, settings, ctx, &pal),
                    PanelPage::Subtitle => {
                        subtitle_page::page(ui, panel, session, settings, modal, ctx, &pal)
                    }
                    PanelPage::Benchmark => benchmark_tab::page(ui, session, settings, bench, &pal),
                    PanelPage::Cache => data::page(ui, panel, session, settings, modal, &pal),
                    PanelPage::Changelog => changelog_tab::page(ui, &pal),
                    PanelPage::Log => unreachable!("日志页已在上方特判，不进入页面级滚动区"),
                });
            ui.add_space(12.0);
        });
}

/// 顶部 Tab 条（原版 QTabBar：选中=白底带边框且与下方内容区相连；
/// 未选中=#E9E9E9 灰底；hover 淡蓝）
fn tab_strip(ui: &mut Ui, panel: &mut PanelUi, pal: &Palette) {
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        for page in PanelPage::ALL {
            let selected = panel.state.page == page;
            let label = lt_i18n::t(page.tab_key());
            let font = egui::FontId::proportional(12.5);
            // 预测文本宽定页签宽（Qt 页签 = 文字 + 左右 padding）
            let probe = ui
                .painter()
                .layout_no_wrap(label.clone(), font.clone(), Color32::WHITE);
            let w = probe.rect.width() + 20.0;
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, TAB_H), egui::Sense::click());
            let fill = if selected {
                pal.page_bg
            } else if resp.hovered() {
                pal.hover
            } else {
                Color32::from_rgb(0xE9, 0xE9, 0xE9)
            };
            ui.painter().rect_filled(rect, 3.0, fill);
            if selected {
                // 选中页签三边描边（底边与内容区相连不描）
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    Stroke::new(1.0, pal.card_stroke),
                    egui::StrokeKind::Outside,
                );
            }
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font,
                pal.text,
            );
            if resp.clicked() {
                panel.state.page = page;
            }
        }
    });
}

/// 分组框：QGroupBox 式直角细边框，标题嵌在框顶线上（白底遮线成缺口）
pub fn group_card(ui: &mut Ui, pal: &Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(12.0);
    let title_font = egui::FontId::proportional(12.5);
    let out = Frame::NONE
        .stroke(Stroke::new(1.0, pal.card_stroke))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    // 标题盖在框顶线上（底色=页底遮断线，等价 QGroupBox 标题缺口）
    let probe =
        ui.painter()
            .layout_no_wrap(format!(" {title} "), title_font.clone(), Color32::WHITE);
    let tw = probe.rect.width() + 6.0;
    let tr = egui::Rect::from_min_size(
        egui::pos2(
            out.response.rect.left() + 10.0,
            out.response.rect.top() - 8.0,
        ),
        egui::vec2(tw, 16.0),
    );
    ui.painter().rect_filled(tr, 0.0, pal.card);
    ui.painter().text(
        egui::pos2(tr.left() + 3.0, tr.center().y),
        egui::Align2::LEFT_CENTER,
        title,
        title_font,
        pal.title,
    );
    ui.add_space(2.0);
}

/// 表单行：标签 + 控件（原版 QGridLayout：col0 标签、col1 控件右列统一宽度）
pub fn form_row(ui: &mut Ui, label: &str, add_control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let label_w = 130.0_f32.min(ui.available_width() * 0.4);
        ui.allocate_ui_with_layout(
            egui::vec2(label_w, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).color(ui.visuals().text_color()));
            },
        );
        add_control(ui);
    });
}

/// 弱化提示行（原版 hintLabel）
pub fn hint_line(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(pal.weak));
}

/// 页顶「偏离默认 + 恢复本页」工具行（N3/N4）：
/// 当前页有参数偏离默认值才显示——蓝色加粗徽标（hover 列出偏离字段）+ 恢复按钮。
/// `reset` 为页内恢复动作（写回默认 + 按字段重发即时命令，见各页 restore_*）；
/// 闭包可拿到 `ui`（字体重载等需要 ctx 的恢复项）。
pub fn reset_toolbar(
    ui: &mut Ui,
    pal: &Palette,
    n: usize,
    fields: &str,
    reset: impl FnOnce(&mut Ui),
) {
    if n == 0 {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(lt_i18n::t("diff_page_badge").replace("{n}", &n.to_string()))
                .size(11.5)
                .color(pal.accent)
                .strong(),
        )
        .on_hover_text(fields);
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("restore_page_defaults")).size(12.0))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            reset(ui);
        }
    });
    ui.add_space(4.0);
}

/// 面板按钮统一入口：`enabled=false` 时呈现"看得见但不可点"的禁用观感。
/// 浅灰底来自 [`panel_visuals`] 的 noninteractive（禁用控件与标签同态）；字色在
/// **局部作用域**弱化——全局改 `noninteractive.fg_stroke` 会波及面板里一切未
/// 显式着色的标签（2026-09-17 审计：40+ 处）。
pub fn panel_btn(
    ui: &mut Ui,
    label: &str,
    enabled: bool,
    size: f32,
    radius: f32,
) -> egui::Response {
    ui.scope(|ui| {
        if !enabled {
            ui.style_mut().visuals.widgets.noninteractive.fg_stroke =
                Stroke::new(1.0, Palette::NATIVE.weak);
        }
        ui.add_enabled(
            enabled,
            egui::Button::new(RichText::new(label).size(size)).corner_radius(radius),
        )
    })
    .inner
}

/// 颜色字段行（原版 QColor 色块 + "#rrggbb" 文本输入；rfd 无颜色
/// 对话框 → 文本编辑承载）。非法值：色块灰底红框提示，字符串原样保留
/// （契约自由格式，overlay 侧 parse_color 有回退）。返回是否发生变更。
pub fn color_field(ui: &mut Ui, id: &str, value: &mut String) -> bool {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::hover());
            match crate::state::normalize_hex_color(value) {
                // from_hex 对 8 位 hex 也接受；normalize 已截到 6 位
                Some(hex) => {
                    if let Ok(c) = egui::Color32::from_hex(&hex) {
                        ui.painter().rect_filled(rect, 0.0, c);
                    }
                }
                None => {
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::GRAY);
                    ui.painter().rect_stroke(
                        rect,
                        0.0,
                        Stroke::new(1.5, egui::Color32::RED),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            ui.add(egui::TextEdit::singleline(value).desired_width(96.0))
                .changed()
        })
        .inner
    })
    .inner
}

/// 设置导出 JSON（原版 _export_settings：store.snapshot 的 JSON pretty 形态）
pub fn export_settings_json(s: &Settings) -> String {
    serde_json::to_string_pretty(s).unwrap_or_else(|_| "{}".to_string())
}

/// 设置导入解析（原版 _import_settings：JSON object → 兼容加载 + sanitize）。
/// 非 object / 解析失败 → Err（原版 raise ValueError → import_invalid 文案）。
pub fn import_settings_json(text: &str) -> Result<Settings, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
    if !v.is_object() {
        return Err("not a JSON object".into());
    }
    Ok(Settings::from_value_compatible(v))
}

/// Panel 窗专用 visuals：Windows 原生浅色控件（宿主在该窗口帧前注入，
/// 与悬浮窗深色互不影响——两窗各自独立 egui pass）。
pub fn panel_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.panel_fill = Palette::NATIVE.window_bg;
    v.window_fill = Palette::NATIVE.window_bg;
    v.extreme_bg_color = Palette::NATIVE.page_bg; // 输入框/文本区底
    v.faint_bg_color = Color32::from_rgb(0xF7, 0xF7, 0xF7);
    v.selection.bg_fill = Palette::NATIVE.selected;
    v.selection.stroke = Stroke::new(1.0, Palette::NATIVE.accent);

    // 普通控件（按钮）：#E1E1E1 底 + #ADADAD 边框（Windows 按钮）
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xE1, 0xE1, 0xE1);
    v.widgets.inactive.bg_fill = Color32::from_rgb(0xE1, 0xE1, 0xE1);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, Palette::NATIVE.text);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xAD, 0xAD, 0xAD));
    v.widgets.inactive.corner_radius = egui::CornerRadius::same(2);

    // 悬停：#E5F1FB 底 + Windows 蓝边框
    v.widgets.hovered.weak_bg_fill = Palette::NATIVE.hover;
    v.widgets.hovered.bg_fill = Palette::NATIVE.hover;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Palette::NATIVE.text);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Palette::NATIVE.accent);
    v.widgets.hovered.corner_radius = egui::CornerRadius::same(2);

    // 按下：#CCE4F7 底 + 深蓝边框
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0xCC, 0xE4, 0xF7);
    v.widgets.active.bg_fill = Color32::from_rgb(0xCC, 0xE4, 0xF7);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Palette::NATIVE.text);
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x00, 0x54, 0x99));
    v.widgets.active.corner_radius = egui::CornerRadius::same(2);

    // 非交互：egui 只有这一个"非交互"态——禁用控件（add_enabled(false)）与标签同态。
    // 底给浅灰实底：禁用按钮要有"按钮形状"（原为全透明 → 用户把灰掉的「删除全部」
    // 读成"没有这个按钮"，2026-09-17 走查 B2a）；标签底 bg_fill 保持透明不受影响。
    // 字色**不能在此全局改灰**（未显式着色的标签会一起变），禁用态的字色由
    // [`panel_btn`] 在局部作用域收窄。
    v.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0xEC, 0xEC, 0xEC);
    v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Palette::NATIVE.text);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Palette::NATIVE.card_stroke);

    v.window_stroke = Stroke::new(1.0, Palette::NATIVE.card_stroke);
    v.window_corner_radius = egui::CornerRadius::same(0);
    v.menu_corner_radius = egui::CornerRadius::same(0);
    v.text_cursor.stroke = Stroke::new(2.0, Palette::NATIVE.accent);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 设置导出 → 导入往返一致；legacy 键经 from_value_compatible 迁移
    #[test]
    fn settings_export_import_roundtrip() {
        let s = Settings {
            vad_threshold: 0.42,
            target_language: "ja".into(),
            ..Settings::default()
        };
        let json = export_settings_json(&s);
        let back = import_settings_json(&json).expect("自身导出必可导入");
        assert_eq!(back, s);

        // 原版 settings.json 含 legacy no_think → W1 起迁移到总开关（导入容错）
        let legacy = r#"{"asr_engine":"sensevoice","models":[{"name":"m","api_base":"b","api_key":"k","model":"d","no_think":true}]}"#;
        let imported = import_settings_json(legacy).expect("legacy 导入应成功");
        assert_eq!(imported.asr_engine, "funasr");
        assert_eq!(imported.funasr_model, "sensevoice-small");
        assert!(
            imported.models[0].disable_thinking,
            "no_think=true → 总开关开"
        );
        assert_eq!(imported.models[0].thinking_style, None, "不再合成方式");
    }

    /// 导入失败路径：非 JSON / 非 object → Err（原版 ValueError → import_invalid）
    #[test]
    fn settings_import_rejects_invalid() {
        assert!(import_settings_json("not json").is_err());
        assert!(import_settings_json("[1,2,3]").is_err());
    }

    /// Tab 键齐全：7 页标题 t() 均非键名本身（yaml 同步检查）
    #[test]
    fn all_tab_titles_resolve() {
        for page in PanelPage::ALL {
            let label = lt_i18n::t(page.tab_key());
            assert_ne!(
                label,
                page.tab_key().to_string(),
                "{} 键应存在于 yaml",
                page.tab_key()
            );
        }
    }

    /// Tab 顺序 = 原版 addTab 顺序（control_panel.py:170-180）+ Rust 版「日志」页（末位）
    #[test]
    fn tab_order_matches_original() {
        assert_eq!(
            PanelPage::ALL.map(|p| p.tab_key()),
            [
                "tab_vad_asr",
                "tab_translation",
                "tab_style",
                "tab_subtitle",
                "tab_benchmark",
                "tab_cache",
                "tab_changelog",
                "tab_log",
            ]
        );
    }

    /// 无头渲染冒烟：8 个 Tab 各跑两帧（egui 即时模式布局收敛需两帧），
    /// 不 panic 且每帧产出图元；设备枚举（VAD/ASR 页）在测试线程真实执行。
    #[test]
    fn panel_ui_smoke_renders_all_pages_headless() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        // VAD/ASR 页强制走设备枚举 + 缓存探测双分支（退役键 → Unavailable 防御分支）
        st.settings.funasr_model = "funasr-mlt-nano-2512".into();
        for page in PanelPage::ALL {
            st.panel.state.page = page;
            for _ in 0..2 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st)
                });
                assert!(!out.shapes.is_empty(), "{page:?} 页应产出图元");
                // epaint debug 断言要求消费纹理增量（无渲染器 → 显式丢弃）
                out.textures_delta.clear();
            }
        }
    }

    /// 防回归（G-39 / D-120）：最小窗宽下 zh/en × 8 页渲染，任何图元左右缘
    /// 不得越出视口（+2.0 = rect_stroke StrokeKind::Outside 出界 ≤1px 的余量）。
    /// egui 横排溢出是静默右缘裁剪（用户看到的就是「右边没了」），本测试是
    /// 「窄窗不裁」的唯一机械保证：新增长文案 / 固定宽控件转红时，按
    /// docs/panel-narrow-layout.md §四 L2 规则拆行，或实测后抬 PANEL_MIN_WIDTH。
    #[test]
    fn panel_no_horizontal_overflow_at_min_width() {
        let screen =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(super::PANEL_MIN_WIDTH, 420.0));
        const TOL: f32 = 2.0;
        for lang in ["zh", "en"] {
            let ctx = egui::Context::default();
            let mut st = crate::state::AppUi::new(Settings::default());
            for page in PanelPage::ALL {
                st.panel.state.page = page;
                for _ in 0..2 {
                    // 全局语言表与同二进制其他测试共享（translation.rs 两个 i18n
                    // 测试与 debounce 冒烟也 set_lang，cargo 默认并行）——渲染后
                    // get_lang 复核，被并行翻走即整帧重渲染；8 次仍被翻走 = 环境
                    // 异常，按现状扫描断言（宁可红不可静默按错语言绿，评审 D-120）
                    let mut frame = None;
                    for attempt in 0..8 {
                        lt_i18n::set_lang(lang).expect("语言表解析");
                        let mut out = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(screen),
                                ..egui::RawInput::default()
                            },
                            |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st),
                        );
                        // epaint debug 断言要求消费纹理增量——先于溢出断言
                        // clear，防 assert 失败时 out 走 Drop 引爆二次 panic 掩盖真因
                        out.textures_delta.clear();
                        frame = Some(out);
                        if lt_i18n::get_lang() == lang || attempt == 7 {
                            break;
                        }
                    }
                    let out = frame.expect("重试环内必有产出");
                    let mut worst_r = screen.right() + TOL;
                    let mut worst_l = screen.left() - TOL;
                    for clipped in &out.shapes {
                        // 看图元自身 bbox 不看 clip rect——被裁掉的正是要抓的
                        let bb = clipped.shape.visual_bounding_rect();
                        worst_r = worst_r.max(bb.right());
                        worst_l = worst_l.min(bb.left());
                    }
                    assert!(
                        worst_r <= screen.right() + TOL && worst_l >= screen.left() - TOL,
                        "{lang} {page:?} 页横向溢出：右缘超 {:+.1}px / 左缘超 {:+.1}px（视口 {}..{}）",
                        worst_r - screen.right(),
                        screen.left() - worst_l,
                        screen.left(),
                        screen.right(),
                    );
                }
            }
        }
    }

    /// 设置防抖登记 → 节拍消费闭环（UI 外）
    #[test]
    fn panel_apply_debounce_tick_roundtrip() {
        let mut st = crate::state::AppUi::new(Settings::default());
        lt_i18n::set_lang("zh").expect("zh 表解析");
        crate::state::register_panel_apply(
            &mut st.panel,
            &mut st.session,
            std::time::Instant::now(),
        );
        let due = st.panel.state.apply_due_at.expect("登记后应有到期时刻");
        assert!(st.take_due_panel_apply(due).is_some());
    }
}
