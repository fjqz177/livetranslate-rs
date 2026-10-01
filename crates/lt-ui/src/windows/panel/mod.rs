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
    BenchUi, LogUi, ModalUi, PanelPage, PanelUi, SessionView, Settings, UiContext, WinAction, WinId,
};
use egui::{Color32, Frame, RichText, ScrollArea, Stroke, Ui};
use std::time::Instant;

/// Tab 条页签高度（原版 QTabBar 页签视觉高度）
const TAB_H: f32 = 26.0;

// ── D-122 tab 条记账常量：绘制（tab_strip）与派生（tab_strip_natural_width）
// 共用同一套——布局实现若与公式脱节，单源直断测试必红 ──
/// Tab 条左 inset（面板根左缘到首页签）
const TAB_STRIP_LEFT_INSET: f32 = 4.0;
/// 页签间距（tab_strip 内 item_spacing 归零，间隙全走本常量，防 egui 二次插空）
const TAB_GAP: f32 = 8.0;
/// 页签文字左右 padding 之和（页签宽 = 文字自然宽 + 本值）
const TAB_LABEL_PAD: f32 = 20.0;
/// 派生最小宽的右缘余量（D-122 裁决 1）
const PANEL_MIN_SLACK: f32 = 8.0;

/// 页套边距 token（D-122 裁决 3「灰框离窗边恒为常数」的唯一值）：
/// 页面 Frame 四边边距的单源，灰框到窗缘距离恒等于它（i8 = Margin::same
/// 原生形态，转型单点发生在派生公式）
pub const PANEL_PAGE_MARGIN: i8 = 12;

/// 面板**首启观感下限**（逻辑 px；D-122）：仅作默认创建宽与创建期 min_inner_size
/// 垫底值，**不参与最小宽正确性**——正确最小宽 = [`derived_panel_min_width`]
/// 运行时派生（语言/字体实测），宿主经 WinAction 首帧即校正。
/// 沿革：D-120 的 PANEL_MIN_WIDTH=600 曾是「溢出安全宽」断言，被实机证伪
/// （egui 默认字体度量 ≠ 实机字体链）；本常数只保观感，不再作任何正确性声明。
pub const PANEL_DEFAULT_WIDTH: f32 = 600.0;

/// 面板最小高（原版 setMinimumSize(480, 420) 的高度项；创建期与
/// WinAction::SetPanelMinWidth 宿主臂共用，防双写漂移）
pub const PANEL_MIN_HEIGHT: f32 = 420.0;

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

/// D-123 C4：**非列表**设置控件变更统一出口——清三处列表选中 + 300ms 防抖落盘。
/// 「列表选中 = 页内临时操作焦点」（docs/panel-list-selection.md §2）：同 tab
/// 改其他设置即焦点离开，选中失效。三列表所在页的非列表控件写回点一律走本
/// 入口；列表自身操作（增删改移/行编辑确认）**豁免**——它们继续走
/// [`mark_settings_dirty`] 并自行维护选中（§3.2），走错入口会吞掉选中跟随。
pub fn mark_settings_changed(panel: &mut PanelUi, session: &mut SessionView) {
    panel.clear_list_selections();
    mark_settings_dirty(session);
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

    // D-123 C2：整窗空白捕获（主案成立）——本 catcher **先于一切内容注册**
    //（egui 后注册者 hit-test 优先，所有真实控件天然压住它；label 等非点击
    // 控件不拦点击），点任意非控件空白——tab 条缝、卡片间衬、行间缝、内容
    // 下方余区——即取消三处列表选中。滚轮/滚动条与 Sense::click 互不抢占
    //（egui 0.36 ScrollArea 无拖拽滚动；headless 验证 =
    // wheel_scroll_still_works_with_blank_catcher）。光标显式回默认，
    // 防 Sense::click 把整片空白染成手型
    if ui
        .interact(
            full,
            egui::Id::new("panel_blank_click_d123"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::Default)
        .clicked()
    {
        panel.clear_list_selections();
    }

    // Tab 条（原版 QTabWidget North 页签行）
    ui.add_space(4.0);
    tab_strip(ui, panel, &pal);

    // D-122：派生最小窗宽差值发送（须先于各页分支——Log 页提前 return）。
    // 语言热切换 / 界面字体更换 → 页签实测宽变 → 派生值变 → 宿主同帧后校正
    let derived_min = derived_panel_min_width(ui.ctx());
    if panel.state.last_emitted_min != Some(derived_min) {
        panel.state.last_emitted_min = Some(derived_min);
        session.enqueue_action(WinId::Panel, WinAction::SetPanelMinWidth(derived_min));
    }

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
            // D-123 C3：横幅点击 = 跳识别页（横幅仅在非识别页可见，必为换页）
            panel.set_page(PanelPage::VadAsr);
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
            .inner_margin(egui::Margin::same(PANEL_PAGE_MARGIN))
            .show(ui, |ui| log_tab::page(ui, log, &pal));
        return;
    }
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            Frame::NONE
                .inner_margin(egui::Margin::same(PANEL_PAGE_MARGIN))
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
            ui.add_space(PANEL_PAGE_MARGIN as f32);
        });
}

/// Tab 条页签字体（绘制与测宽共用同一 FontId——D-122 禁两套标尺）
fn tab_font() -> egui::FontId {
    egui::FontId::proportional(12.5)
}

/// 单个页签的排版宽度（文字自然宽 + 左右 padding）——tab_strip 绘制与
/// [`derived_panel_min_width`] 共用的唯一测宽点（D-122 单源）
fn tab_label_width(ctx: &egui::Context, label: &str) -> f32 {
    // 与 egui Painter::layout_no_wrap 同一代码路径（fonts_mut + layout INFINITY）
    let galley =
        ctx.fonts_mut(|f| f.layout(label.to_owned(), tab_font(), Color32::WHITE, f32::INFINITY));
    galley.rect.width() + TAB_LABEL_PAD
}

/// Tab 条自然宽 = 左 inset + Σ页签宽 + 页签间距×(n−1)。
/// tab_strip 的逐项摆放与本公式用同一常量集（D-122）：两者脱节即单源直断测试红。
pub fn tab_strip_natural_width(ctx: &egui::Context) -> f32 {
    let tabs: f32 = PanelPage::ALL
        .iter()
        .map(|p| tab_label_width(ctx, &lt_i18n::t(p.tab_key())))
        .sum();
    TAB_STRIP_LEFT_INSET + tabs + TAB_GAP * (PanelPage::ALL.len() - 1) as f32
}

/// 面板最小窗宽（D-122 运行时派生单源）= tab 条自然宽 + 右缘余量。
/// 取代旧 `PANEL_MIN_WIDTH=600` 常数（egui 默认字体度量，实机思源链更宽即失守）：
/// 语言热切换 / 界面字体更换 / 字体度量变化全部自动跟随。
pub fn derived_panel_min_width(ctx: &egui::Context) -> f32 {
    tab_strip_natural_width(ctx) + PANEL_MIN_SLACK
}

/// 顶部 Tab 条（原版 QTabBar：选中=白底带边框且与下方内容区相连；
/// 未选中=#E9E9E9 灰底；hover 淡蓝）
fn tab_strip(ui: &mut Ui, panel: &mut PanelUi, pal: &Palette) {
    ui.horizontal(|ui| {
        // 间距记账全部走 D-122 公式常量（TAB_STRIP_LEFT_INSET / TAB_GAP）：
        // egui item_spacing 归零防二次插空，保证实摆右缘 = tab_strip_natural_width
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(TAB_STRIP_LEFT_INSET);
        let last = PanelPage::ALL.len() - 1;
        for (i, page) in PanelPage::ALL.iter().enumerate() {
            let selected = panel.state.page == *page;
            let label = lt_i18n::t(page.tab_key());
            // 预测文本宽定页签宽（Qt 页签 = 文字 + 左右 padding）——测宽与
            // derived_panel_min_width 同一函数（D-122 单源）
            let w = tab_label_width(ui.ctx(), &label);
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
                tab_font(),
                pal.text,
            );
            if resp.clicked() {
                // D-123 C3：唯一换页口 set_page——实际换页即清三处列表选中
                //（点击当前页签不算切换，不清）
                panel.set_page(*page);
            }
            if i != last {
                ui.add_space(TAB_GAP);
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
/// 导入结果接线进草稿时（UI 触发点尚未实装，D-121），若 `ui_lang` 与当前
/// 语言不同须调 [`crate::windows::panel::vad::apply_ui_lang`] 同步全局语言表。
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

/// D-123 headless 点击取证共用件（仅测试构建）：显式单调时钟 + galley 文本
/// 精确定位 + 指针单击帧序列（subtitle.rs drag_frames 同源模式）。
/// **时钟为何显式**：egui `RawInput.time=None` 时每帧仅推进 predicted_dt
/// （1/60s），两次连发 click 相距 3 帧 ≈ 50ms < max_double_click_delay 0.3s
/// 会被判成双击——`idle()` 用真实时间步进拉开间隔。
/// **visuals 为何注入**（D-124）：headless 台必须逐项对齐真机注入面
/// （app.rs run_frame）——此前跑 egui 默认 visuals（w_inactive=0），
/// selectable 静止/悬停两态占位恰好相等，悬停跳变类病对本台整体失明。
#[cfg(test)]
pub(crate) mod click_testing {
    /// 面板 headless 测试台：单调时钟 + 真机 visuals/滚动条注入 +
    /// dispatch WinId::Panel 全真路径渲染
    pub(crate) struct PanelHarness {
        ctx: egui::Context,
        screen: egui::Rect,
        t: f64,
    }

    impl PanelHarness {
        pub(crate) fn new(screen: egui::Rect) -> Self {
            Self {
                ctx: egui::Context::default(),
                screen,
                t: 0.0,
            }
        }

        /// 渲染一帧（时间步进 1/60s）
        pub(crate) fn render(
            &mut self,
            st: &mut crate::state::AppUi,
            events: Vec<egui::Event>,
        ) -> egui::FullOutput {
            self.t += 1.0 / 60.0;
            // D-124：注入真机 visuals 三件套（对齐 app.rs run_frame 的面板臂）。
            // 此前跑 egui 默认 visuals（w_inactive=0），selectable 静止/悬停
            // 两态占位恰好相等——悬停跳变类病对本测试台整体失明（与 G-40
            // 时钟盲区并列的测试台教训：headless 台必须逐项对齐真机注入面）。
            let mut visuals = crate::windows::panel::panel_visuals();
            crate::style::stabilize_widget_strokes(&mut visuals);
            self.ctx.set_visuals(visuals);
            let scroll = crate::style::panel_scroll_style();
            self.ctx.all_styles_mut(move |s| s.spacing.scroll = scroll);
            let mut out = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(self.screen),
                    events,
                    time: Some(self.t),
                    ..Default::default()
                },
                |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, st),
            );
            out.textures_delta.clear();
            out
        }

        /// 连续喂多帧事件
        pub(crate) fn feed(
            &mut self,
            st: &mut crate::state::AppUi,
            frames: Vec<Vec<egui::Event>>,
        ) -> egui::FullOutput {
            let mut last = None;
            for events in frames {
                last = Some(self.render(st, events));
            }
            last.expect("至少一帧")
        }

        /// 单击 = 悬停到位 → 按下 → 释放（释放帧登记 click）
        pub(crate) fn click(
            &mut self,
            st: &mut crate::state::AppUi,
            pos: egui::Pos2,
        ) -> egui::FullOutput {
            self.feed(
                st,
                vec![
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    }],
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    }],
                ],
            )
        }

        /// 快进 n 帧无事件（≈n/60s）：把与上次点击的间隔拉出双击窗口
        ///（max_double_click_delay = 0.3s ⇒ n ≥ 19 即安全）
        pub(crate) fn idle(&mut self, st: &mut crate::state::AppUi, n: usize) -> egui::FullOutput {
            let mut last = None;
            for _ in 0..n {
                last = Some(self.render(st, vec![]));
            }
            last.expect("至少一帧")
        }
    }

    /// 在已渲染帧图元中找文本恰为 `needle` 的图元，返回其中心点
    ///（文本必在所在按钮矩形内 → 即可点击位；rect 取 galley 尺寸 + pos，
    /// galley.rect 相对 pos——translation.rs 既有取证同款）
    pub(crate) fn text_center(
        shapes: &[egui::epaint::ClippedShape],
        needle: &str,
    ) -> Option<egui::Pos2> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(t) if t.galley.text() == needle => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-131 回归钉：下拉弹层内悬停/选中项的描边框不得贴弹层裁剪缘。
    /// 根因：弹层 Area 高度在 sizing pass 冻结＝内容高 → 滚动区 clip 底与
    /// 末项框底齐平（1.25 等非整缩放下 scissor 取整越缘 +0.2px）→ Inside
    /// 描边的底边整条被裁没（用户实报：≥3 项下拉末项蓝框缺底边；2 项弹层
    /// 因 sizing 多测 19px 而幸免）。对策 = 弹层滚动区底部 content_margin
    /// 垫 2px（style::combo_popup_style 单源）。断言：弹层内所有描边项框
    /// 的框底到裁剪缘距离 ≥1.0px（1px 描边 + AA 羽化的安全余量），
    /// 3 项引擎下拉 @1.0/@1.25 双缩放 + 2 项静音下拉对照。
    #[test]
    fn combo_popup_last_item_frame_not_clipped() {
        let _lang_guard = crate::lang_test_guard();
        lt_i18n::set_lang("zh").expect("语言表解析");
        let ctx = real_fonts_ctx();
        let min_w = derived_panel_min_width(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(min_w, 1200.0));

        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = PanelPage::VadAsr;

        let render = |ctx: &egui::Context,
                      st: &mut crate::state::AppUi,
                      events: Vec<egui::Event>,
                      t: f64|
         -> egui::FullOutput {
            let mut visuals = panel_visuals();
            crate::style::stabilize_widget_strokes(&mut visuals);
            ctx.set_visuals(visuals);
            let scroll = crate::style::panel_scroll_style();
            ctx.all_styles_mut(move |s| s.spacing.scroll = scroll);
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    time: Some(t),
                    ..Default::default()
                },
                |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, st),
            );
            out.textures_delta.clear();
            out
        };

        // 场景驱动：点开组合框 → 悬停弹层最后一项 → 返回末帧（弹层是否打开
        // 以「末项文本出现」为准——popup Area id 是 Ui 链哈希，外部无法按
        // salt 重建）
        let open_and_hover = |ctx: &egui::Context,
                              st: &mut crate::state::AppUi,
                              sel_text: &str,
                              last_needle: &str,
                              t0: f64|
         -> (egui::FullOutput, egui::Rect, f64) {
            let mut t = t0;
            let mut btn_pos = None;
            for _ in 0..4 {
                t += 1.0 / 60.0;
                let out = render(ctx, st, vec![], t);
                btn_pos = click_testing::text_center(&out.shapes, sel_text);
            }
            let btn_pos = btn_pos.expect("找不到组合框按钮文本");
            let evs = [
                vec![egui::Event::PointerMoved(btn_pos)],
                vec![egui::Event::PointerButton {
                    pos: btn_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }],
                vec![egui::Event::PointerButton {
                    pos: btn_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }],
            ];
            for ev in evs {
                t += 1.0 / 60.0;
                render(ctx, st, ev, t);
            }
            let mut last_rect = None;
            for _ in 0..3 {
                t += 1.0 / 60.0;
                let out = render(ctx, st, vec![], t);
                for clipped in &out.shapes {
                    if let egui::Shape::Text(txt) = &clipped.shape {
                        if txt.galley.text().contains(last_needle) {
                            last_rect = Some(txt.galley.rect.translate(txt.pos.to_vec2()));
                        }
                    }
                }
            }
            let last_rect =
                last_rect.unwrap_or_else(|| panic!("弹层未打开或找不到末项文本: {last_needle:?}"));
            let hover = egui::Pos2::new(last_rect.center().x, last_rect.bottom() - 2.0);
            let mut out = None;
            for _ in 0..3 {
                t += 1.0 / 60.0;
                out = Some(render(ctx, st, vec![egui::Event::PointerMoved(hover)], t));
            }
            (out.expect("悬停帧必有产出"), last_rect, t)
        };

        // 场景间点空白处关掉上一个弹层（CloseOnClick），防下次点击被 toggle 吞掉
        let click_at =
            |ctx: &egui::Context, st: &mut crate::state::AppUi, pos: egui::Pos2, t0: f64| -> f64 {
                let mut t = t0;
                for ev in [
                    vec![egui::Event::PointerMoved(pos)],
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    }],
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    }],
                ] {
                    t += 1.0 / 60.0;
                    render(ctx, st, ev, t);
                }
                for _ in 0..2 {
                    t += 1.0 / 60.0;
                    render(ctx, st, vec![], t);
                }
                t
            };

        // 快进 n 帧无事件（拉开幕间间隔，防双击判定 + ppp 切换后重整一帧）
        let idle = |ctx: &egui::Context, st: &mut crate::state::AppUi, n: usize, t0: f64| -> f64 {
            let mut t = t0;
            for _ in 0..n {
                t += 1.0 / 60.0;
                render(ctx, st, vec![], t);
            }
            t
        };

        let scenarios = [
            ("引擎3项@1.0", "FunASR (SenseVoice)", "Qwen3", 1.0),
            ("引擎3项@1.25", "FunASR (SenseVoice)", "Qwen3", 1.25),
            ("静音2项@1.0", "自动", "固定", 1.0),
        ];
        let mut fails: Vec<String> = Vec::new();
        let mut t = 0.0f64;
        for (tag, sel_text, last_needle, ppp) in scenarios {
            ctx.set_pixels_per_point(ppp);
            t = idle(&ctx, &mut st, 30, t);
            t = click_at(&ctx, &mut st, egui::Pos2::new(5.0, 1150.0), t);
            let (out, last_rect, t_end) = open_and_hover(&ctx, &mut st, sel_text, last_needle, t);
            t = t_end;
            // 弹层项描边框集合：有描边、行高 15..40、宽 >100、且所在 clip
            // 高 <500（页面级控件的 clip = 视口 ~1167，据此只留弹层内行）
            let mut worst: Option<(f32, egui::Rect)> = None;
            for clipped in &out.shapes {
                let cr = clipped.clip_rect;
                if cr.height() >= 500.0 {
                    continue;
                }
                let mut leafs: Vec<&egui::Shape> = Vec::new();
                fn push_leafs<'a>(s: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                    match s {
                        egui::Shape::Vec(v) => v.iter().for_each(|s| push_leafs(s, out)),
                        s => out.push(s),
                    }
                }
                push_leafs(&clipped.shape, &mut leafs);
                for leaf in leafs {
                    if let egui::Shape::Rect(r) = leaf {
                        let h = r.rect.height();
                        if r.stroke.width > 0.0
                            && (15.0..40.0).contains(&h)
                            && r.rect.width() > 100.0
                        {
                            let slack = cr.bottom() - r.rect.bottom();
                            if worst.is_none_or(|(s, _)| slack < s) {
                                worst = Some((slack, r.rect));
                            }
                        }
                    }
                }
            }
            let Some((slack, rect)) = worst else {
                fails.push(format!("{tag}: 弹层内未找到任何描边项框（过滤面失效）"));
                continue;
            };
            if slack < 1.0 {
                fails.push(format!(
                    "{tag}: 弹层项描边框贴/越裁剪缘 slack={slack:+.3}（框 {rect:?}，末项文本矩形 {last_rect:?}）——底边会被 scissor 裁没（D-131）"
                ));
            }
        }
        ctx.set_pixels_per_point(1.0);
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

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

    /// D-122 单源直断：①派生公式随语言变（en > zh）②随字体链变（装真实内嵌
    /// 思源链后值变——实机标尺 ≠ egui 默认字体标尺，正是 D-120 失守的根）③
    /// tab 条实摆右缘 = 公式预测（绘制与公式同一常量集，脱节即红）。
    #[test]
    fn derived_panel_min_width_tracks_lang_and_fonts_and_matches_layout() {
        let _lang_guard = crate::lang_test_guard();
        // egui Fonts 惰性初始化：Context::run 之前无字体可排版（context.rs 硬断言
        // 「No fonts available until first call to Context::run()」）——先垫一帧。
        // 同理宿主创建期也拿不到派生值（用默认宽垫底、首帧动作校正，见 T4）。
        let prime = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label("prime");
            });
            out.textures_delta.clear();
        };

        // ① 默认字体链下 en > zh
        let ctx = egui::Context::default();
        prime(&ctx);
        lt_i18n::set_lang("zh").expect("zh 表解析");
        let zh_default = super::derived_panel_min_width(&ctx);
        lt_i18n::set_lang("en").expect("en 表解析");
        let en_default = super::derived_panel_min_width(&ctx);
        assert!(
            en_default > zh_default,
            "en tab 行须宽于 zh：{en_default} vs {zh_default}"
        );

        // ② 真实内嵌思源链（实机同源标尺）度量 ≠ egui 默认字体度量
        let ctx_real = real_fonts_ctx();
        let en_real = super::derived_panel_min_width(&ctx_real);
        assert!(
            en_real != en_default,
            "真实思源链 en 度量须异于默认字体：{en_real} vs {en_default}"
        );

        // ③ tab 条实摆右缘 = 公式（真实字体链下验证，逐项对账）
        let mut st = crate::state::AppUi::new(Settings::default());
        let mut out = ctx_real.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(en_real + 400.0, 400.0),
                )),
                ..egui::RawInput::default()
            },
            |ui| tab_strip(ui, &mut st.panel, &Palette::NATIVE),
        );
        out.textures_delta.clear();
        let natural = super::tab_strip_natural_width(&ctx_real);
        let mut max_r = f32::MIN;
        for clipped in &out.shapes {
            max_r = max_r.max(clipped.shape.visual_bounding_rect().right());
        }
        assert!(
            (max_r - natural).abs() <= 1.0,
            "tab 条实摆右缘 {max_r:.1} 应等于公式预测 {natural:.1}（±1.0 = 选中页签 \
             Outside 描边余量）——绘制与派生公式脱节（D-122 单源被破坏）"
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

    /// 防回归（G-39 / D-120 / D-122）：最小窗宽下 zh/en × 8 页渲染，任何图元
    /// 左右缘不得越出视口（+2.0 = rect_stroke StrokeKind::Outside 出界 ≤1px 的余量）。
    /// egui 横排溢出是静默右缘裁剪（用户看到的就是「右边没了」），本测试是
    /// 「窄窗不裁」的唯一机械保证：新增长文案 / 固定宽控件转红时，按
    /// docs/panel-narrow-layout.md §四 L2 规则拆行——禁回灌抬最小宽（D-122 裁决 5）。
    ///
    /// D-122 换标尺（四件套①②③）：① ctx 装**真实内嵌思源链**——D-120 盲区
    /// 根修，旧测试用 egui 默认字体度量、600 定值在实机思源链下失守；② 断言
    /// 视口 = [`derived_panel_min_width`] 派生公式输出（公式错即红）；③ en 事实
    /// 锚：内嵌链下派生 min < 旧常数 600（定量证明 D-120 的定值标尺不是思源链）。
    #[test]
    fn panel_no_horizontal_overflow_at_min_width() {
        let _lang_guard = crate::lang_test_guard();
        const TOL: f32 = 2.0;
        for lang in ["zh", "en"] {
            // 先设语再派生——页签宽度是语言的函数（并行残留语言会污染派生值，
            // 首跑曾致 zh/en 同值的假象）
            lt_i18n::set_lang(lang).expect("语言表解析");
            // ① 真实内嵌思源链，与实机同源（helper 内含垫帧）
            let ctx = real_fonts_ctx();
            // ② 断言视口 = 派生公式输出
            let min_w = super::derived_panel_min_width(&ctx);
            if lang == "en" {
                // ③ 事实锚：**内嵌思源链**下 en 派生 > 旧常数 600（实测 strip
                // 609.3，600 客户区正好裁掉 ~9px「Logs」——与用户实机截图量级
                // 一致）。D-120 的「591」是 egui 默认字体度量的假标尺；用户换
                // 更宽 UI 字体时派生值更大（运行时实测自动跟随）。首跑曾得
                // 「en=509.7」的假象，根因 = 派生先于 set_lang（语言污染）。
                assert!(
                    min_w > super::PANEL_DEFAULT_WIDTH,
                    "en 派生 min {min_w} 须 > 旧常数 600（内嵌思源链下旧常数必裁                      tab 条——本 bug 的定量锚）；若不成立须复核字体链/语言时序"
                );
            }
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(min_w, 420.0));
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
                        // 不可见图元不计溢出（透明 = 肉眼无「右边没了」）：egui
                        // ScrollArea 的内容 extent 记账与 clip 缘存在 ~2px 微差
                        // （changelog_tab set_max_width −10 安全量同源），横向
                        // extent 超缘时会发出全透明的滚动条几何残影
                        if shape_paints_nothing(&clipped.shape) {
                            continue;
                        }
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

    /// 防回归（G-39 家族 / 用户 2026-10-01 截图）：下载卡片**非 Idle 三态**在最小
    /// 窗宽下任何图元不得越出视口右缘。既有 [`panel_no_horizontal_overflow_at_min_width`]
    /// 只渲染 `AppUi::new` 默认的 Idle 态（测试盲区逃逸）——Downloading/Failed 的长
    /// 文案（状态头 + 字节对 + 含 repo 路径的日志行/原始错误串 + 取消按钮）全挂在
    /// vad.rs 下载卡单行 horizontal 里，溢出即静默右裁：用户看到「右侧大小/路径
    /// 没了」，宽窗下取消按钮也被长日志行推出视口不可达。技术同上测（真实思源链、
    /// 派生视口、图元 bbox 扫描）；视口高 2000 让整页免滚动完整绘制——下载卡在
    /// VAD/ASR 页底，420 高会整卡滚出视口使断言空转，故断言只看横向。
    #[test]
    fn panel_download_states_no_horizontal_overflow_at_min_width() {
        let _lang_guard = crate::lang_test_guard();
        use crate::state::DownloadUiState;
        const TOL: f32 = 2.0;
        const REPO: &str = "csukuangfj2/sherpa-onnx-qwen3-0.6B-int8-2026-03-25";
        // 真实形态造数（用户截图量级）：Qwen3 第 2/6 个文件、114/174 MB、日志行带全 repo 路径
        let states: [(&str, DownloadUiState); 3] = [
            (
                "downloading",
                DownloadUiState::Downloading {
                    file: "encoder.int8.onnx".into(),
                    k: 2,
                    n: 6,
                    done_bytes: 119_537_664,  // ≈114.0 MiB
                    total_bytes: 182_452_224, // ≈174.0 MiB
                    log: vec![format!("[{REPO}] 开始下载 encoder.int8.onnx")],
                },
            ),
            (
                "failed",
                DownloadUiState::Failed {
                    kind: lt_proto::DownloadFailKind::Net,
                    detail: format!("[{REPO}] encoder.int8.onnx 下载失败，16s 后第 3 次重试"),
                    log: vec![],
                },
            ),
            (
                "cancelled",
                DownloadUiState::Cancelled {
                    log: vec![format!("[{REPO}] 开始下载 encoder.int8.onnx")],
                },
            ),
        ];
        let mut fails: Vec<String> = Vec::new();
        for lang in ["zh", "en"] {
            lt_i18n::set_lang(lang).expect("语言表解析");
            let ctx = real_fonts_ctx();
            let min_w = super::derived_panel_min_width(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(min_w, 2000.0));
            for (name, ds) in &states {
                let mut st = crate::state::AppUi::new(Settings::default());
                st.panel.state.page = PanelPage::VadAsr;
                st.panel.download = ds.clone();
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
                    // epaint debug 断言要求消费纹理增量——先于溢出断言 clear
                    out.textures_delta.clear();
                    frame = Some(out);
                    if lt_i18n::get_lang() == lang && attempt >= 1 {
                        break;
                    }
                }
                let out = frame.expect("重试环内必有产出");
                for clipped in &out.shapes {
                    if shape_paints_nothing(&clipped.shape) {
                        continue;
                    }
                    let bb = clipped.shape.visual_bounding_rect();
                    if bb.right() > screen.right() + TOL {
                        let txt = match &clipped.shape {
                            egui::Shape::Vec(v) => v.iter().find_map(|s| match s {
                                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                                _ => None,
                            }),
                            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                            _ => None,
                        }
                        .unwrap_or_default();
                        fails.push(format!(
                            "{lang} {name} 态图元越右缘 {:+.1}px（视口右 {:.1}）bbox {bb:?} | {txt}",
                            bb.right() - screen.right(),
                            screen.right()
                        ));
                    }
                }
            }
        }
        // 末尾一次性断言：全矩阵失败一次看全
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    /// 防回归（G-42 面板内二级模态形态 / D-126；G-39 同族）：面板拉到最小宽时打开
    /// 编辑模型 / 编辑字幕行两个 egui::Window 内部模态，任何图元不得越出视口
    /// 左右缘（用户 2026-10-01 截图：模态整块超出宿主窗宽、左右双缘被裁）。
    /// 技术同 [`panel_no_horizontal_overflow_at_min_width`]（真实思源链 +
    /// 派生视口 + 图元 bbox 扫描）；模态场景 ≥3 帧再取末帧（egui::Window
    /// 开窗 sizing 收敛，G-40 姊妹坑）。
    #[test]
    fn panel_modals_no_horizontal_overflow_at_min_width() {
        let _lang_guard = crate::lang_test_guard();
        const TOL: f32 = 2.0;
        let mut fails: Vec<String> = Vec::new(); // 全场景失败收集，末尾一次断言
        type Open = fn(&mut crate::state::AppUi);
        let scenarios: [(&str, &str, PanelPage, Open); 2] = [
            (
                "ModelEditDialog",
                "panel_model_edit_dialog",
                PanelPage::Translation,
                |st: &mut crate::state::AppUi| {
                    st.panel.state.model_editor = Some(crate::state::ModelEditState::new_edit(
                        0,
                        &st.settings.models[0],
                    ));
                },
            ),
            (
                "LineEditDialog",
                "panel_line_edit_dialog",
                PanelPage::Subtitle,
                |st: &mut crate::state::AppUi| {
                    st.panel.state.line_editor = Some(crate::state::LineEditState::new_add(0));
                },
            ),
        ];
        for lang in ["zh", "en"] {
            lt_i18n::set_lang(lang).expect("语言表解析");
            let ctx = real_fonts_ctx();
            let min_w = super::derived_panel_min_width(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(min_w, 420.0));
            for (name, win_id_salt, page, open) in scenarios {
                let mut st = crate::state::AppUi::new(Settings::default());
                st.panel.state.page = page;
                open(&mut st);
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
                    // epaint debug 断言要求消费纹理增量——先于溢出断言 clear，
                    // 防 assert 失败时 out 走 Drop 引爆二次 panic 掩盖真因
                    out.textures_delta.clear();
                    frame = Some(out);
                    if lt_i18n::get_lang() == lang && attempt >= 2 {
                        break;
                    }
                }
                let out = frame.expect("重试环内必有产出");
                // ① 模态窗体矩形 ⊆ 视口（横纵双轴）——egui::Window 尺寸随内容
                // 自然宽/高走、Area constrain 只摆位不缩窗，窗体越视口即整块
                // 双缘被裁（用户截图 1）；窗体合围 ⇒ 标题/底部按钮行必然可达。
                // 矩形取自 egui memory（生产侧显式 Id，与语言无关），不做图元
                // 考古。滚动区内容的纵向越缘不在此列——那是有滚动条可达的
                // 正常溢出
                let win_id = egui::Id::new(win_id_salt);
                match ctx.memory(|m| m.area_rect(win_id)) {
                    Some(wr) if wr.is_finite() => {
                        if wr.right() > screen.right() + TOL
                            || wr.left() < screen.left() - TOL
                            || wr.bottom() > screen.bottom() + TOL
                            || wr.top() < screen.top() - TOL
                        {
                            fails.push(format!(
                                "{lang} {name} 窗体矩形越视口：窗 {wr:?} vs 视口 {screen:?}"
                            ));
                        }
                    }
                    _ => fails.push(format!(
                        "{lang} {name} 模态窗体未在 memory 留下矩形（Id 不匹配？）"
                    )),
                }
                // ② 横向逐图元 bbox ⊆ 自身 clip_rect——clip = 窗体∩视口（校准
                // 实证），抓「行越窗内容宽」的窗内裁（用户截图 2 的 元/1M to…
                // 半字消失）；纵向不查（ScrollArea 内容纵向越缘 = 可滚动到达）
                for clipped in &out.shapes {
                    if shape_paints_nothing(&clipped.shape) {
                        continue;
                    }
                    let mut leafs: Vec<&egui::Shape> = Vec::new();
                    fn push_leafs<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
                        match shape {
                            egui::Shape::Vec(v) => v.iter().for_each(|s| push_leafs(s, out)),
                            s => out.push(s),
                        }
                    }
                    push_leafs(&clipped.shape, &mut leafs);
                    for leaf in leafs {
                        if shape_paints_nothing(leaf) {
                            continue;
                        }
                        // blur>0 = 窗阴影（设计上外溢 15px，越缘属正常装饰）
                        if let egui::Shape::Rect(r) = leaf {
                            if r.blur_width > 0.0 {
                                continue;
                            }
                        }
                        let bb = leaf.visual_bounding_rect();
                        let cr = clipped.clip_rect;
                        let over_r = bb.right() - cr.right();
                        let over_l = cr.left() - bb.left();
                        if over_r > TOL || over_l > TOL {
                            let txt = match leaf {
                                egui::Shape::Text(t) => t.galley.text().to_owned(),
                                other => format!("{other:?}"),
                            };
                            fails.push(format!(
                                "{lang} {name} 图元横向越自身裁剪缘：右 {over_r:+.1} / \
                                 左 {over_l:+.1}（clip {cr:?}）bbox {bb:?} | {txt}"
                            ));
                        }
                    }
                }
            }
        }
        // 末尾一次性断言：诊断期全矩阵可见（修复期与未来回归都一次看全）
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }

    /// 测试脚手架：装真实内嵌思源链（实机同源标尺）+ 垫一帧——egui Fonts
    /// 惰性初始化，`Context::run` 之前无字体可排版
    fn real_fonts_ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::apply_fonts(
            &ctx,
            &Settings::default(),
            &mut crate::fonts::FontsState::default(),
        );
        let mut prime = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("prime");
        });
        prime.textures_delta.clear();
        ctx
    }

    /// 图元是否完全不上屏（全透明）——不可见即不构成可见裁剪
    fn shape_paints_nothing(shape: &egui::Shape) -> bool {
        match shape {
            egui::Shape::Noop => true,
            egui::Shape::Vec(v) => v.iter().all(shape_paints_nothing),
            egui::Shape::Mesh(m) => m
                .vertices
                .iter()
                .all(|v| v.color == egui::Color32::TRANSPARENT),
            egui::Shape::Rect(r) => {
                r.fill == egui::Color32::TRANSPARENT
                    && (r.stroke.color == egui::Color32::TRANSPARENT || r.stroke.width <= 0.0)
            }
            _ => false,
        }
    }

    /// D-122：派生最小宽差值发送——首帧发一次后稳态静默；语言热切换派生值
    /// 变化后再发一次（宿主臂据此 set_min_inner_size，桌面端无冗余动作流）
    #[test]
    fn panel_emits_derived_min_once_then_on_change() {
        let _lang_guard = crate::lang_test_guard();
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(Settings::default());
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
        let render = |st: &mut crate::state::AppUi| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..egui::RawInput::default()
                },
                |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, st),
            );
            out.textures_delta.clear();
        };
        let mins = |st: &mut crate::state::AppUi| -> Vec<f32> {
            st.session
                .drain_actions()
                .into_iter()
                .filter_map(|(w, a)| match (w, a) {
                    (WinId::Panel, WinAction::SetPanelMinWidth(v)) => Some(v),
                    _ => None,
                })
                .collect()
        };

        lt_i18n::set_lang("zh").expect("zh 表解析");
        render(&mut st);
        render(&mut st);
        render(&mut st);
        let first = mins(&mut st);
        assert_eq!(first.len(), 1, "首帧应恰好发一次派生 min，实际 {first:?}");
        assert!(
            first[0] > 400.0 && first[0] < 1200.0,
            "派生值离谱：{:?}",
            first
        );

        // 语言热切换 → 页签实测宽变 → 派生值变 → 恰好再发一次且更大
        lt_i18n::set_lang("en").expect("en 表解析");
        render(&mut st);
        let second = mins(&mut st);
        assert_eq!(second.len(), 1, "语言切换后应恰再发一次，实际 {second:?}");
        assert!(
            second[0] > first[0],
            "en 派生值应大于 zh：{:?} vs {:?}",
            second,
            first
        );
        render(&mut st);
        assert!(
            mins(&mut st).is_empty(),
            "稳态（语言/字体不变）不应再发——差值发送失效会刷爆宿主臂"
        );
    }

    /// D-122 四件套④：灰框离窗边恒为常数（裁决 3）——分组框（card_stroke 描边、
    /// 非全宽）左右缘 = 视口缘 ∓ [`PANEL_PAGE_MARGIN`]（±2.0）。视口拉高到无纵向
    /// 滚动条，排除滚动条宽度对水平记账的干扰（溢出测试的 420 高视口已含最严
    /// 滚动态；两组视口互补覆盖）。
    #[test]
    fn panel_group_frames_keep_constant_window_margin() {
        let _lang_guard = crate::lang_test_guard();
        let _ = lt_i18n::set_lang("zh");
        let ctx = real_fonts_ctx();
        let min_w = super::derived_panel_min_width(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(min_w, 2000.0));
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = PanelPage::Translation;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..egui::RawInput::default()
            },
            |ui| crate::windows::dispatch(crate::state::WinId::Panel, ui, &mut st),
        );
        out.textures_delta.clear();
        // 分组框识别：card_stroke 色描边矩形（无填充），排除全宽 pane 描边与
        // tab 条区选中页签描边；横幅描边为橙色非 card_stroke 天然排除
        let mut frame_r = f32::MIN;
        let mut frame_l = f32::MAX;
        for clipped in &out.shapes {
            if let egui::Shape::Rect(rs) = &clipped.shape {
                if rs.fill != egui::Color32::TRANSPARENT
                    || rs.stroke.color != super::Palette::NATIVE.card_stroke
                    || rs.rect.width() > screen.width() - 1.0
                    || rs.rect.top() < 50.0
                {
                    continue;
                }
                frame_r = frame_r.max(rs.rect.right());
                frame_l = frame_l.min(rs.rect.left());
            }
        }
        let m = super::PANEL_PAGE_MARGIN as f32;
        assert!(
            frame_r.is_finite() && (frame_r - (screen.right() - m)).abs() <= 2.0,
            "分组框右缘 {frame_r:.1} 应 = 视口右缘 − 边距 token（{:.1}）——\
             灰框离窗边常数被破坏（D-122 裁决 3）",
            screen.right() - m
        );
        assert!(
            frame_l.is_finite() && (frame_l - (screen.left() + m)).abs() <= 2.0,
            "分组框左缘 {frame_l:.1} 应 = 视口左缘 + 边距 token（{:.1}）",
            screen.left() + m
        );
    }

    /// 设置防抖登记 → 节拍消费闭环（UI 外）
    #[test]
    fn panel_apply_debounce_tick_roundtrip() {
        let _lang_guard = crate::lang_test_guard();
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

    // ── D-123 列表选中态：面板根级行为（C2/C3）──

    /// C3：切 tab 清三处列表选中（切回仍清）；点击当前页签不算切换
    #[test]
    fn tab_switch_clears_all_list_selections() {
        let _lang_guard = crate::lang_test_guard();
        lt_i18n::set_lang("zh").expect("zh 表解析");
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = PanelPage::Translation;
        use click_testing as ct;
        let mut h = ct::PanelHarness::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(700.0, 1400.0),
        ));

        // 预置三处选中（选中路径本身已有专测，这里直接置位隔离验证 C3）
        st.panel.state.model_selected = Some(0);
        st.panel.state.cache_selected = Some(0);
        st.panel.state.line_selected = Some(0);

        // 点「字幕」页签 → 换页 + 三处全清
        let out = h.render(&mut st, vec![]);
        let pos =
            ct::text_center(&out.shapes, &lt_i18n::t("tab_subtitle")).expect("字幕页签应上屏");
        h.idle(&mut st, 25);
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.page, PanelPage::Subtitle, "应换到字幕页");
        assert_eq!(st.panel.state.model_selected, None, "切 tab 应清模型选中");
        assert_eq!(st.panel.state.cache_selected, None, "切 tab 应清缓存选中");
        assert_eq!(st.panel.state.line_selected, None, "切 tab 应清文字行选中");

        // 点「翻译」切回 → 仍清（选中不跨 tab 存活）
        let out = h.render(&mut st, vec![]);
        let pos =
            ct::text_center(&out.shapes, &lt_i18n::t("tab_translation")).expect("翻译页签应上屏");
        h.idle(&mut st, 25);
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.page, PanelPage::Translation);
        assert_eq!(st.panel.state.model_selected, None, "切回应仍清");
        assert_eq!(st.panel.state.line_selected, None, "切回应仍清");

        // 点击当前页签（翻译）→ 不是切换，页不变（无选中可断言，仅验证不 panic）
        let out = h.render(&mut st, vec![]);
        let pos =
            ct::text_center(&out.shapes, &lt_i18n::t("tab_translation")).expect("翻译页签应上屏");
        h.idle(&mut st, 25);
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.page, PanelPage::Translation, "页应不变");
    }

    /// C2：空白区单击 = 取消列表选中（整窗 catcher：内容下方余区 + 页边距衬条
    /// 都算）；空白处拖拽不构成点击（不得误清）
    #[test]
    fn blank_click_deselects_but_drag_does_not() {
        let _lang_guard = crate::lang_test_guard();
        lt_i18n::set_lang("zh").expect("zh 表解析");
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = PanelPage::Subtitle;
        use click_testing as ct;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 1400.0));
        let mut h = ct::PanelHarness::new(screen);

        // 选中文字行 0（点击路径真跑，不预置）
        let out = h.render(&mut st, vec![]);
        let needle = crate::windows::panel::subtitle_page::line_row_text(
            &st.settings.subtitle_mode.lines[0],
        );
        let pos = ct::text_center(&out.shapes, &needle).expect("文字行摘要应上屏");
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.line_selected, Some(0));

        // 空白位 = 最低文本图元下方 150px（钳在视口内）：内容下方余区
        let out = h.render(&mut st, vec![]);
        let bottom = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.pos.y + t.galley.size().y),
                _ => None,
            })
            .fold(0.0_f32, f32::max);
        let blank = egui::pos2(350.0, (bottom + 150.0).min(screen.bottom() - 40.0));

        // 空白处拖拽（按下-移动-释放）不是点击，不得触发取消
        let moved = blank - egui::vec2(0.0, 60.0);
        h.feed(
            &mut st,
            vec![
                vec![egui::Event::PointerMoved(blank)],
                vec![egui::Event::PointerButton {
                    pos: blank,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                }],
                vec![egui::Event::PointerMoved(moved)],
                vec![egui::Event::PointerButton {
                    pos: moved,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                }],
            ],
        );
        assert_eq!(
            st.panel.state.line_selected,
            Some(0),
            "空白处拖拽不是点击，不得误清选中"
        );

        // 快进后同空白区单击 → 取消（C2）
        h.idle(&mut st, 25);
        h.click(&mut st, blank);
        assert_eq!(st.panel.state.line_selected, None, "点空白应取消选中（C2）");

        // 边距衬条（页 Frame 左侧 12px 边距带，卡片间衬同面）也算空白：重选中
        // → 点 x=6 → 再清（锁「整窗捕获」而非仅内容下方余区）
        h.idle(&mut st, 25);
        h.click(&mut st, pos);
        assert_eq!(st.panel.state.line_selected, Some(0));
        h.idle(&mut st, 25);
        h.click(&mut st, egui::pos2(6.0, blank.y));
        assert_eq!(
            st.panel.state.line_selected, None,
            "页边距衬条点击应取消选中（整窗 catcher 覆盖卡片间衬）"
        );
    }

    /// T3 共存验证：滚轮滚动不受空白 catcher 影响（面板滚动主通道。
    /// egui 0.36 ScrollArea 本无拖拽滚动，滚轮+滚动条是唯二通道，catcher 的
    /// Sense::click 与两者互不抢占——本测试锁滚轮，滚动条在 catcher 矩形外）
    #[test]
    fn wheel_scroll_still_works_with_blank_catcher() {
        let _lang_guard = crate::lang_test_guard();
        lt_i18n::set_lang("zh").expect("zh 表解析");
        let mut st = crate::state::AppUi::new(Settings::default());
        st.panel.state.page = PanelPage::Subtitle;
        use click_testing as ct;
        // 420 高视口（溢出断言测试同款）：内容自然高必超视口 → max_offset > 0
        //（800 高视口内容放得下，滚轮无路可滚，catcher 无关）
        let mut h = ct::PanelHarness::new(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(700.0, 420.0),
        ));

        let label = lt_i18n::t("subwin_show");
        // 滚轮需先有指针位置（egui 按 pointer 下所属区域投递滚轮）
        h.render(
            &mut st,
            vec![egui::Event::PointerMoved(egui::pos2(350.0, 400.0))],
        );
        let out = h.render(&mut st, vec![]);
        let before: std::collections::HashSet<String> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(before.contains(&label), "字幕窗口标签应上屏");
        // 滚轮向下（delta 负 = 内容上移）；egui 把离散滚轮量低通分帧（未消费量
        // 逐帧转 smooth_scroll_delta）→ 补 idle 帧走完
        h.feed(
            &mut st,
            vec![vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -120.0),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::default(),
            }]],
        );
        h.idle(&mut st, 30);
        let out = h.render(&mut st, vec![]);
        // 滚动 120px 足以把起始标签完全滚出裁剪区（滚前图元被剔除正是滚动发生
        // 的证据）；改用集合差断言：滚后必须露出滚前不可见的下方内容
        let revealed: Vec<String> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .filter(|txt| !before.contains(txt))
            .collect();
        assert!(
            !revealed.is_empty(),
            "滚轮后应露出滚前不可见的下方内容（catcher 截胡滚轮则空集）"
        );
    }
}
