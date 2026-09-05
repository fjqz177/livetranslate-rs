//! 启动流三合一对话框（对照原版 LiveTranslate/dialogs.py）：
//! - 首启向导 SetupWizardDialog：hub/代理选择 + 15s 倒计时自动下载 + 日志区 + 失败重试；
//! - 缺模型下载 ModelDownloadDialog：说明行 + 日志区，失败才出现"关闭"（成功自动关）；
//! - 模型加载 _ModelLoadDialog：消息 + 日志区占位，无关闭按钮。
//!
//! 三者共用 WinId::Setup 常规窗口，按 AppState.load_dialog / AppState.startup 分派。
//! 定时不在这里做：倒计时与 500ms 收尾延迟由宿主 about_to_wait 的 Setup 节拍驱动，
//! 本模块只提供 [`needs_setup_tick`] 供宿主判断是否需要节拍。

use crate::state::{AppState, StartupFlow, WizardPhase};
use egui::{Color32, RichText, Ui};

/// 日志区底色（原版 QTextEdit 样式 background: #1e1e2e）
const LOG_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
/// 日志区文字色（原版 color: #cdd6f4）
const LOG_FG: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);
/// 失败提示红（原版仅追加日志文本，这里用易读的红字展示错误行）
const ERROR_RED: Color32 = Color32::from_rgb(0xf3, 0x8b, 0xa8);

/// Setup 窗 UI 总入口（windows::dispatch 按 WinId::Setup 分派到这里）
pub fn setup_ui(ui: &mut Ui, state: &mut AppState) {
    // 模型加载对话框优先（与启动流互斥，加载框显示期间覆盖向导/下载内容）
    if state.load_dialog.is_some() {
        load_dialog_ui(ui, state);
        return;
    }
    let is_wizard = matches!(state.startup, StartupFlow::Wizard(_));
    let is_missing = matches!(state.startup, StartupFlow::DownloadMissing { .. });
    if is_wizard {
        wizard_ui(ui, state);
    } else if is_missing {
        download_missing_ui(ui, state);
    } else {
        // 无启动流且无加载框：窗口此时不应可见，兜底空显示
    }
}

/// Setup 窗是否需要节拍驱动：向导倒计时（Idle）/ 向导成功后的 500ms 收尾（Done）/
/// 缺模型下载成功后的 500ms 收尾（finished）。失败与下载中由事件驱动，无需节拍。
pub fn needs_setup_tick(state: &AppState) -> bool {
    match &state.startup {
        StartupFlow::Wizard(w) => matches!(w.phase, WizardPhase::Idle | WizardPhase::Done),
        StartupFlow::DownloadMissing { finished, .. } => *finished,
        StartupFlow::Ready => false,
    }
}

/// 首启向导（对照 SetupWizardDialog）：分组 + ComboBox + URL 输入 + 倒计时按钮 + 日志区
fn wizard_ui(ui: &mut Ui, state: &mut AppState) {
    ui.heading(lt_i18n::t("window_setup"));
    ui.add_space(4.0);

    let start_clicked = {
        let StartupFlow::Wizard(w) = &mut state.startup else {
            return;
        };
        let busy = matches!(w.phase, WizardPhase::Downloading);

        // ── 下载源分组（原版 QGroupBox + hub QComboBox）──
        egui::Frame::group(ui.style()).show(ui, |ui| {
            if busy {
                ui.disable(); // 原版下载中禁用 hub 下拉
            }
            ui.strong(lt_i18n::t("group_download_source"));
            let hub_items = [lt_i18n::t("hub_modelscope"), lt_i18n::t("hub_huggingface")];
            egui::ComboBox::from_id_salt("wizard_hub")
                .selected_text(hub_items[w.hub_index].clone())
                .show_ui(ui, |ui| {
                    // 任一控件变更重置倒计时（原版 currentIndexChanged → _reset_countdown）
                    for (i, item) in hub_items.iter().enumerate() {
                        if ui.selectable_value(&mut w.hub_index, i, item.clone()).changed() {
                            w.countdown = 15;
                        }
                    }
                });
        });
        ui.add_space(4.0);

        // ── 代理分组（原版 QGroupBox + QFormLayout：模式下拉 + URL 单行输入）──
        egui::Frame::group(ui.style()).show(ui, |ui| {
            if busy {
                ui.disable(); // 原版下载中禁用代理控件
            }
            ui.strong(lt_i18n::t("group_download_proxy"));
            let mode_items = [
                lt_i18n::t("proxy_none"),
                lt_i18n::t("proxy_system"),
                lt_i18n::t("proxy_custom"),
            ];
            egui::Grid::new("wizard_proxy_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label(lt_i18n::t("label_proxy"));
                    egui::ComboBox::from_id_salt("wizard_proxy_mode")
                        .selected_text(mode_items[w.proxy_index].clone())
                        .show_ui(ui, |ui| {
                            for (i, item) in mode_items.iter().enumerate() {
                                if ui
                                    .selectable_value(&mut w.proxy_index, i, item.clone())
                                    .changed()
                                {
                                    // 原版 _on_proxy_mode_changed → _reset_countdown
                                    w.countdown = 15;
                                }
                            }
                        });
                    ui.end_row();
                    ui.label(lt_i18n::t("label_proxy_url"));
                    // 仅"自定义代理"可编辑（原版 setEnabled(index == 2)）
                    let url_edit = ui.add(
                        egui::TextEdit::singleline(&mut w.proxy_url)
                            .hint_text("http://127.0.0.1:7890")
                            .desired_width(f32::INFINITY),
                    );
                    if url_edit.changed() {
                        w.countdown = 15; // 原版 textEdited → _reset_countdown
                    }
                    ui.end_row();
                });
        });
        ui.add_space(6.0);

        // ── 下载按钮：Idle 带 "(Ns)" 倒计时、Downloading 禁用、Failed 变"重试" ──
        let btn_text = match w.phase {
            WizardPhase::Idle => {
                format!("{} ({}s)", lt_i18n::t("btn_start_download"), w.countdown)
            }
            WizardPhase::Downloading | WizardPhase::Done => lt_i18n::t("btn_start_download"),
            WizardPhase::Failed => lt_i18n::t("btn_retry"),
        };
        let btn = egui::Button::new(RichText::new(btn_text));
        let resp = if busy {
            ui.add_enabled(false, btn)
        } else {
            ui.add_sized([ui.available_width(), 28.0], btn)
        };
        let clicked = resp.clicked();

        // ── 日志区：Idle 期隐藏（原版 _log_view.hide()），开始下载后显示 ──
        if !matches!(w.phase, WizardPhase::Idle) {
            ui.add_space(6.0);
            log_view(ui, &w.log);
        }
        clicked
    };
    if start_clicked {
        // 与倒计时归零共用同一条路径：切 Downloading + 发 StartDownload
        state.wizard_auto_start();
    }
}

/// 缺模型下载对话框（对照 ModelDownloadDialog）：说明行 + 日志区；
/// 无失败时没有任何按钮（原版：不能取消），失败后才出现"关闭"。
fn download_missing_ui(ui: &mut Ui, state: &mut AppState) {
    let mut close_clicked = false;
    {
        let StartupFlow::DownloadMissing { names, log, failed, .. } = &mut state.startup else {
            return;
        };
        ui.heading(lt_i18n::t("window_download"));
        ui.add_space(4.0);
        // 说明行（zh.yaml 模板 "正在下载所需模型: {names}"，按实际占位替换）
        ui.label(lt_i18n::t("downloading_models").replace("{names}", names));
        ui.add_space(6.0);
        log_view(ui, log);

        if let Some(err) = failed {
            ui.add_space(6.0);
            // 失败行（zh.yaml 模板 "下载失败: {error}"）
            ui.label(
                RichText::new(lt_i18n::t("download_failed").replace("{error}", err))
                    .color(ERROR_RED),
            );
            // 原版 reject：无模型不可用 → 退出应用（由宿主消费 quit_requested）
            close_clicked = ui.button(lt_i18n::t("btn_close")).clicked();
        }
        // 成功路径无按钮：DownloadSucceeded 置 finished 后由 500ms 节拍自动关窗
    }
    if close_clicked {
        state.send_cmd(lt_proto::Cmd::Stop);
        state.quit_requested = true;
    }
}

/// 模型加载对话框（对照 _ModelLoadDialog，简化版）。
/// 已知偏差：原版加载框内嵌 INFO 日志流（_LogCapture 捕获 tracing 输出），
/// 本版日志窗 M4 才有，这里日志区暂为占位不接日志流，先显示消息本体（M4 接线）。
fn load_dialog_ui(ui: &mut Ui, state: &mut AppState) {
    let Some(label) = state.load_dialog.clone() else {
        return;
    };
    ui.add_space(4.0);
    // zh.yaml 模板 "正在加载 {name}...\n请稍候。"（YAML 双引号 \n 已解析为真实换行）
    ui.label(lt_i18n::t("loading_model").replace("{name}", &label));
    ui.add_space(6.0);
    log_view(ui, &[]); // M2.5 占位；M4 接 LogLine 日志流
    // 无关闭按钮：加载结束由 ModelLoadDone / AsrDevice / AsrUnavailable 事件关窗
}

/// 日志区：暗底圆角 + 等宽小字 + ScrollArea 自动滚底。
/// 对照原版 QTextEdit（Consolas 8 / background #1e1e2e / color #cdd6f4）；
/// 滚底写法与 windows/mod.rs 悬浮窗消息流一致（scroll_to_cursor BOTTOM）。
fn log_view(ui: &mut Ui, log: &[String]) {
    egui::Frame::NONE
        .fill(LOG_BG)
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // 空日志也保持一块可见的暗底区域（原版 setMinimumHeight 语义）
                    ui.set_min_height(140.0);
                    for line in log {
                        ui.label(RichText::new(line).monospace().small().color(LOG_FG));
                    }
                    // 自动滚动到底部（原版 _append_log 把滚动条推到最大值）
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                });
        });
}
