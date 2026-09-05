//! 控制面板（M4.3 第一批：面板框架 + 常规页 + 识别页；对照原版 ui/panel/）：
//!
//! - 框架（panel.py）：左侧 200px 导航列表（品牌行 + 7 页项，选中高亮）+
//!   右侧页栈（每页 = 标题 + 一行提示 + 可滚动内容，高内容页滚动不撑破窗口，
//!   等价原版 page_header / make_scroll_page）；内容列居中限宽（§3.5.4）。
//! - 分组卡片（_chrome.py 深色主题）：圆角描边组框 + 组标题，双主题调色板
//!   取自原版 DARK_QSS/LIGHT_QSS 字面色。
//! - 设置流（panel.py `_auto_save`）：控件直接改 `AppState.settings` 对应字段，
//!   300ms 防抖后经 [`lt_proto::Cmd::ApplySettings`] 整体重放（热应用 + 落盘，
//!   原版 settings_changed 语义）；引擎/模型/语言/设备类即时命令不走防抖。
//!
//! 常规页见 [`general`]，识别页见 [`vad`]；本批未接入的页显示占位提示
//! （翻译/字幕/数据与存储/诊断/关于随 M4.4+ 波次）。

mod autostart;
pub mod general;
pub mod vad;

use crate::state::{AppState, PanelPage, ThemeMode};
use egui::{Color32, Frame, RichText, ScrollArea, Stroke, Ui};
/// 组框描边/内边距等常量（原版 QGroupBox border-radius 10 / padding 12 10 10 10）
use lt_proto::Settings;

/// 组框圆角（原版 QGroupBox border-radius: 10px）
const CARD_RADIUS: f32 = 10.0;
/// 内容列最大宽（原版 _stack.setMaximumWidth(960)，§3.5.4 居中限宽）
const CONTENT_MAX_W: f32 = 960.0;
/// 导航项高度（原版 QListWidgetItem sizeHint 38px）
const NAV_ITEM_H: f32 = 34.0;

/// 主题调色板（原版 _chrome.py DARK_QSS / LIGHT_QSS 的字面色子集）
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// 窗口底（ControlPanel background）
    pub window_bg: Color32,
    /// 卡片底（QGroupBox background）
    pub card: Color32,
    /// 卡片描边（QGroupBox border）
    pub card_stroke: Color32,
    /// 主文本（QLabel color）
    pub text: Color32,
    /// 标题文本（pageTitle color）
    pub title: Color32,
    /// 弱化文本（pageHint/hintLabel color）
    pub weak: Color32,
    /// 导航悬停底（panelNav::item:hover background）
    pub hover: Color32,
    /// 导航选中底（panelNav::item:selected background）
    pub selected: Color32,
    /// 选中/强调（Highlight #4C8DFF）
    pub accent: Color32,
    /// 状态色（QLabel[status=ok/warn/error]）
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
}

impl Palette {
    /// 按 egui 当前主题取调色板（主题由常规页外观组经 ctx.set_theme 切换，
    /// 与原版 apply_app_theme 应用级生效一致）
    pub fn resolve(ui: &Ui) -> Self {
        if ui.ctx().theme() == egui::Theme::Light {
            Palette::LIGHT
        } else {
            Palette::DARK
        }
    }

    /// 原版 DARK_QSS 字面色
    pub const DARK: Palette = Palette {
        window_bg: Color32::from_rgb(0x0E, 0x11, 0x16),
        card: Color32::from_rgb(0x10, 0x14, 0x1B),
        card_stroke: Color32::from_rgb(0x23, 0x2A, 0x35),
        text: Color32::from_rgb(0xE6, 0xED, 0xF3),
        title: Color32::from_rgb(0xF2, 0xF4, 0xF7),
        weak: Color32::from_rgb(0x9A, 0xA3, 0xB2),
        hover: Color32::from_rgb(0x16, 0x1B, 0x22),
        selected: Color32::from_rgb(0x1B, 0x26, 0x35),
        accent: Color32::from_rgb(0x4C, 0x8D, 0xFF),
        ok: Color32::from_rgb(0x3F, 0xB9, 0x50),
        warn: Color32::from_rgb(0xDB, 0xAB, 0x09),
        err: Color32::from_rgb(0xF8, 0x51, 0x49),
    };

    /// 原版 LIGHT_QSS 字面色
    pub const LIGHT: Palette = Palette {
        window_bg: Color32::from_rgb(0xF6, 0xF8, 0xFA),
        card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        card_stroke: Color32::from_rgb(0xD1, 0xD9, 0xE0),
        text: Color32::from_rgb(0x1F, 0x23, 0x28),
        title: Color32::from_rgb(0x1F, 0x23, 0x28),
        weak: Color32::from_rgb(0x57, 0x60, 0x6A),
        hover: Color32::from_rgb(0xEB, 0xEF, 0xF3),
        selected: Color32::from_rgb(0xD9, 0xE6, 0xFF),
        accent: Color32::from_rgb(0x09, 0x69, 0xDA),
        ok: Color32::from_rgb(0x1A, 0x7F, 0x37),
        warn: Color32::from_rgb(0x9A, 0x67, 0x00),
        err: Color32::from_rgb(0xCF, 0x22, 0x2E),
    };
}

/// 控件变更 → 300ms 防抖应用（原版 TabBase.auto_save 的面板侧统一入口）
pub fn mark_settings_dirty(state: &mut AppState) {
    state.schedule_panel_apply();
}

/// 引擎/模型变化（原版 settings_changed → main 侧 switch_engine 的即时路径）
pub fn send_switch_engine(state: &AppState) {
    let s = &state.settings;
    state.send_cmd(lt_proto::Cmd::SwitchEngine {
        engine: s.asr_engine.clone(),
        funasr_model: s.funasr_model.clone(),
        whisper_model_size: s.whisper_model_size.clone(),
        hub: s.hub.clone(),
        language: s.asr_language.clone(),
    });
}

// ── 框架布局 ──

/// 面板 UI 总入口（windows::dispatch 按 WinId::Panel 分派到这里）
pub fn panel_ui(ui: &mut Ui, state: &mut AppState) {
    // 主题收敛：深色为默认（原版 DEFAULT_THEME = dark，应用级生效）。
    // 主题存 PanelUiState 内存态（settings 契约缺 theme 键，lt-proto 不动）。
    let want = match state.panel.theme {
        ThemeMode::Dark => egui::ThemePreference::Dark,
        ThemeMode::Light => egui::ThemePreference::Light,
    };
    let active = match ui.ctx().theme() {
        egui::Theme::Dark => egui::ThemePreference::Dark,
        egui::Theme::Light => egui::ThemePreference::Light,
    };
    if active != want {
        ui.ctx().set_theme(want);
    }
    let pal = Palette::resolve(ui);

    // 全窗底色（等价 ControlPanel WA_StyledBackground + QSS background）
    ui.painter()
        .rect_filled(ui.available_rect_before_wrap(), 0.0, pal.window_bg);

    // ── 左侧导航（原版 nav_column：brand + caption + 200px QListWidget）──
    egui::Panel::left("panel_nav")
        .resizable(false)
        .exact_size(200.0)
        .frame(Frame::NONE.fill(pal.window_bg))
        .show(ui, |ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("LiveTranslate").strong().size(15.0).color(pal.title));
            ui.label(RichText::new(lt_i18n::t("settings")).size(11.0).color(pal.weak));
            ui.add_space(12.0);
            // 导航项：先分配整行矩形，再绘制底色与文本（原版 QListWidget item
            // ::selected #1B2635 / :hover #161B22，圆角 6、左右缩进 12）
            for page in PanelPage::ALL {
                let selected = state.panel.page == page;
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ITEM_H), egui::Sense::click());
                let fill = if selected {
                    pal.selected
                } else if resp.hovered() {
                    pal.hover
                } else {
                    pal.window_bg
                };
                let item_rect = rect.shrink2(egui::vec2(12.0, 2.0));
                ui.painter().rect_filled(item_rect, 6.0, fill);
                ui.painter().text(
                    egui::pos2(item_rect.left() + 8.0, item_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    lt_i18n::t(page.nav_key()),
                    egui::FontId::proportional(13.0),
                    if selected { pal.title } else { pal.weak },
                );
                if resp.clicked() {
                    // 原版 _on_nav_changed：切栈 + 数据/诊断页刷新（两页本批未接入）
                    state.panel.page = page;
                }
            }
        });

    // ── 右侧内容区（原版 _stack：页头 + 滚动内容，§3.5）──
    egui::CentralPanel::default()
        .frame(Frame::NONE.fill(pal.window_bg).inner_margin(egui::Margin::same(16)))
        .show(ui, |ui| {
            let page = state.panel.page;
            // 识别页页头右侧带"性能基准"按钮（原版 _build_recognition_page 的 header_row）
            if page == PanelPage::Recognition {
                ui.horizontal(|ui| {
                    page_header(ui, &pal, page);
                    if ui
                        .add(
                            egui::Button::new(RichText::new(lt_i18n::t("btn_open_benchmark")).size(12.5))
                                .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        // M4.3：benchmark 独立工具窗随 M4.4 接入（原版 BenchmarkDialog.exec()）
                        tracing::info!("{}", lt_i18n::t("benchmark_pending_m44"));
                    }
                });
            } else {
                page_header(ui, &pal, page);
            }
            ui.add_space(4.0);
            // 高内容页滚动（原版 make_scroll_page：内容滚动、页面 sizeHint 有界）
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
                    // 内容列居中限宽（原版 content_box addStretch×1 + maxWidth 960）
                    let w = ui.available_width().min(CONTENT_MAX_W);
                    let x0 = ui.cursor().left() + ((ui.available_width() - w) * 0.5).max(0.0);
                    let rect =
                        egui::Rect::from_min_size(egui::pos2(x0, ui.cursor().top()), egui::vec2(w, ui.available_height()));
                    let mut col = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    match page {
                        PanelPage::General => general::page(&mut col, state, &pal),
                        PanelPage::Recognition => vad::page(&mut col, state, &pal),
                        other => pending_page(&mut col, &pal, other),
                    }
                    let used = col.min_rect();
                    ui.advance_cursor_after_rect(used);
                });
        });
}

/// 页头：标题 + 一行提示（原版 _tab_base.page_header）
fn page_header(ui: &mut Ui, pal: &Palette, page: PanelPage) {
    ui.add_space(2.0);
    ui.label(RichText::new(lt_i18n::t(page.nav_key())).strong().size(18.0).color(pal.title));
    ui.label(RichText::new(lt_i18n::t(page.hint_key())).size(12.0).color(pal.weak));
    ui.add_space(4.0);
}

/// 本批未接入页的占位（翻译/字幕/数据与存储/诊断/关于随 M4.4+ 波次）
fn pending_page(ui: &mut Ui, pal: &Palette, page: PanelPage) {
    group_card(ui, pal, &lt_i18n::t(page.nav_key()), |ui| {
        ui.label(RichText::new(lt_i18n::t("page_pending")).color(pal.weak));
    });
}

/// 分组卡片：组标题 + 圆角描边组框（原版 QGroupBox + DARK/LIGHT_QSS 字面色）
pub fn group_card(ui: &mut Ui, pal: &Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong().size(12.5).color(pal.title));
    ui.add_space(4.0);
    Frame::NONE
        .fill(pal.card)
        .stroke(Stroke::new(1.0, pal.card_stroke))
        .corner_radius(egui::CornerRadius::same(CARD_RADIUS as u8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(4.0);
}

/// 表单行：标签 + 控件（原版 QGridLayout：col0 标签、col1 控件 ≥180px）
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

/// 设置导出 JSON（原版 _export_settings：store.snapshot 的 JSON pretty 形态）
pub fn export_settings_json(s: &Settings) -> String {
    serde_json::to_string_pretty(s).unwrap_or_else(|_| "{}".to_string())
}

/// 设置导入解析（原版 _import_settings：JSON object → 兼容加载 + sanitize）。
/// 非 object / 解析失败 → Err（原版 raise ValueError → import_invalid 文案）。
pub fn import_settings_json(text: &str) -> Result<Settings, String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?;
    if !v.is_object() {
        return Err("not a JSON object".into());
    }
    Ok(Settings::from_value_compatible(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常规页界面语言三选项（原版 general_tab：system/zh/en + data 载荷）
    #[test]
    fn ui_lang_table_and_index_mapping() {
        assert_eq!(
            general::UI_LANGS.map(|(code, _)| code),
            ["system", "zh", "en"],
            "原版 addItem 顺序：跟随系统/中文/英文"
        );
        // 非法/缺省值回退"跟随系统"（原版 stored not in (...) → "system"）
        assert_eq!(general::ui_lang_index_for("zh"), 1);
        assert_eq!(general::ui_lang_index_for("en"), 2);
        assert_eq!(general::ui_lang_index_for("system"), 0);
        assert_eq!(general::ui_lang_index_for("bogus"), 0);
        // 索引 → 设置值往返
        for (code, _) in general::UI_LANGS {
            assert_eq!(general::UI_LANGS[general::ui_lang_index_for(code)].0, code);
        }
    }

    /// 导出 → 导入往返一致；legacy 键经 from_value_compatible 迁移
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

        // 原版 settings.json 含 legacy no_think → thinking_style 迁移（导入容错）
        let legacy = r#"{"asr_engine":"sensevoice","models":[{"name":"m","api_base":"b","api_key":"k","model":"d","no_think":true}]}"#;
        let imported = import_settings_json(legacy).expect("legacy 导入应成功");
        assert_eq!(imported.asr_engine, "funasr");
        assert_eq!(imported.funasr_model, "sensevoice-small");
        assert_eq!(imported.models[0].thinking_style.as_deref(), Some("auto"));
    }

    /// 导入失败路径：非 JSON / 非 object → Err（原版 ValueError → import_invalid）
    #[test]
    fn settings_import_rejects_invalid() {
        assert!(import_settings_json("not json").is_err());
        assert!(import_settings_json("[1,2,3]").is_err());
    }

    /// 无头渲染冒烟：7 个页各跑两帧（egui 即时模式布局收敛需两帧），
    /// 不 panic 且每帧产出图元；设备枚举（识别页）在测试线程真实执行。
    #[test]
    fn panel_ui_smoke_renders_all_pages_headless() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(Settings::default());
        // 识别页强制走设备枚举 + 缓存探测双分支（mlt 保存值 → Unavailable 提示）
        st.settings.funasr_model = "funasr-mlt-nano-2512".into();
        for page in PanelPage::ALL {
            st.panel.page = page;
            for _ in 0..2 {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| panel_ui(ui, &mut st));
                assert!(!out.shapes.is_empty(), "{page:?} 页应产出图元");
                // epaint debug 断言要求消费纹理增量（无渲染器 → 显式丢弃）
                out.textures_delta.clear();
            }
        }
        // 常规页控件值 ↔ 设置字段联动（语言下拉映射在 UI 外已单测；
        // 此处锁页栈切换 + 防抖登记 → 节拍 → ApplySettings 快照的 UI 外闭环）
        st.panel.page = PanelPage::General;
        lt_i18n::set_lang("zh");
        st.schedule_panel_apply();
        let due = st.panel.apply_due_at.expect("登记后应有到期时刻");
        assert!(st.take_due_panel_apply(due).is_some());
    }
}
