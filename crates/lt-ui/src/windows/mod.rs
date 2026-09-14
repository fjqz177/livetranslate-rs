//! 窗口 UI 分发（egui 0.36：容器一律吃 `&mut Ui`）。
//!
//! - overlay：全量复刻（见 [`overlay`] 模块文档）
//! - subtitle：全量复刻（见 [`subtitle`] 模块文档）
//! - setup：启动流对话框（向导/缺模型下载/模型加载）
//! - panel：控制面板（框架 + 常规/翻译/识别/字幕/样式/数据/诊断/关于七页，见 [`panel`]）
//! - log：日志窗（M4.5）
//! - bench：性能基准独立工具窗（M4.4）

use crate::state::{AppUi, WinId};
use egui::{Color32, RichText, Ui};

pub mod bench;
pub mod confirm;
pub mod logwin;
pub mod overlay;
pub mod panel;
pub mod setup;
pub mod subtitle;

/// 日志区右下角浮动「回到最新（+N）」按钮（`ui.put` 叠加，不占布局；
/// log_tab 与 logwin 两视图共用，D-31 贴底跟随）
pub(crate) fn log_jump_button(ui: &mut Ui, unread: usize) -> bool {
    let text = format!("{} (+{unread})", lt_i18n::t("log_back_to_latest"));
    let font = egui::FontId::proportional(11.5);
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), font, Color32::WHITE);
    let pad = egui::vec2(10.0, 6.0);
    let size = galley.size() + pad + pad;
    let frame = ui.max_rect();
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            frame.right() - size.x - 10.0,
            frame.bottom() - size.y - 10.0,
        ),
        size,
    );
    ui.put(
        rect,
        egui::Button::new(RichText::new(text).size(11.5)).corner_radius(6.0),
    )
    .clicked()
}

/// 按窗口分发（根 Ui 由宿主经 ctx.run_ui 提供）。
///
/// W5/R8：**解构分发**——各窗口模块只拿本域 `&mut` + 共享 `&`，借用检查器
/// 强制窗口边界（越界不再靠命名纪律）；跨域写（打开面板某页/设置防抖意图）
/// 经 WinAction / SessionView 意图由宿主根消费。确认窗（D-87：六确认唯一
/// 载体，跨全域应用逻辑）独占根级 `&mut AppUi` 渲染臂。
pub fn dispatch(win: WinId, ui: &mut Ui, app: &mut AppUi) {
    match win {
        WinId::Overlay => {
            let AppUi {
                settings,
                ctx,
                session,
                overlay,
                modal,
                ..
            } = app;
            overlay::overlay_ui(ui, overlay, session, settings, modal, ctx);
            // D-87 属主遮罩：确认打开期间吞掉悬浮窗点击（真模态）
            confirm::owner_shield_if_open(ui, app);
        }
        WinId::Subtitle => {
            let AppUi {
                settings,
                ctx,
                session,
                subtitle,
                ..
            } = app;
            subtitle::subtitle_ui(ui, subtitle, session, settings, ctx);
        }
        WinId::Panel => {
            let AppUi {
                settings,
                ctx,
                session,
                modal,
                panel,
                log,
                bench,
                ..
            } = app;
            panel::panel_ui(ui, panel, session, settings, modal, log, bench, ctx);
            // D-87 属主遮罩：确认打开期间吞掉面板点击（真模态，防确认期间列表变动）
            confirm::owner_shield_if_open(ui, app);
        }
        WinId::Log => {
            let AppUi { log, .. } = app;
            logwin::log_ui(ui, log);
        }
        // 启动流对话框（首启向导/缺模型下载/模型加载按 state 内部阶段再分派）
        WinId::Setup => {
            let AppUi {
                session,
                startup,
                modal,
                ..
            } = app;
            setup::setup_ui(ui, startup, session, modal);
        }
        // 性能基准独立工具窗（识别页页头按钮打开）
        WinId::Benchmark => {
            let AppUi {
                settings,
                session,
                bench,
                ..
            } = app;
            bench::bench_ui(ui, bench, session, settings);
        }
        // 确认窗（D-87：六确认唯一载体；根级 &mut——效果跨 overlay/panel/settings 域）
        WinId::Confirm => {
            confirm::confirm_ui(ui, app);
        }
    }
}
