//! 设置页「日志」tab（Rust 版新增：用户自查/报障入口）。
//!
//! 与日志窗（logwin.rs）共享同一环形缓冲（AppState.logwin），本界面是第二视图：
//! - 级别色 + 时间 + target + 消息，最新在底部；
//! - 布局：工具行恒置顶 + 日志滚动区占满剩余——本页不经过 panel 的页面级
//!   ScrollArea（panel_ui 特判），否则内容溢出时会出现第二根滚动条（LT-1，
//!   见 docs/archive/log-tab-redesign.md）；
//! - 交互（D-31）：显示 DEBUG 渲染期过滤（勾选即回溯显示历史）；自动滚动 =
//!   贴底跟随（上翻挂起 +「回到最新」浮钮）；
//! - 数据零新增：LogLine 事件流（logging.rs BroadcastLayer）本来就全量到达 UI。

use super::Palette;
use crate::state::{LogUi, LogView, LogWindowState};
use crate::windows::log_jump_button;
use egui::{Color32, RichText, ScrollArea, Ui};
use std::time::{Duration, Instant};

/// 「已复制 N 行」按钮反馈留存时长
const COPY_FLASH: Duration = Duration::from_millis(800);

/// 级别色（浅色面板用：INFO 主文字色、DEBUG 弱灰、WARN 琥珀、ERROR 红）
fn level_color(level: u8, pal: &Palette) -> Color32 {
    match level {
        10 => pal.weak,
        30 => pal.warn,
        40..=50 => pal.err,
        _ => pal.text,
    }
}

pub fn page(ui: &mut Ui, log: &mut LogUi, pal: &Palette) {
    toolbar(ui, log);
    ui.add_space(4.0);
    log_region(ui, log, pal);
}

/// ── 工具行：本页不经过 panel 外层滚动区，故天然置顶 ──
fn toolbar(ui: &mut Ui, log: &mut LogUi) {
    ui.horizontal(|ui| {
        ui.add(egui::Checkbox::new(
            &mut log.logwin.show_debug,
            RichText::new(lt_i18n::t("show_debug")).size(12.0),
        ));
        ui.add(egui::Checkbox::new(
            &mut log.logwin.auto_scroll,
            RichText::new(lt_i18n::t("auto_scroll")).size(12.0),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // 打开日志目录（按钮名与行为一致；hover 显示当前会话文件）
            let open = ui.button(RichText::new(lt_i18n::t("log_open_dir")).size(12.0));
            let open = if let Some(path) = latest_log_file() {
                open.on_hover_text(
                    lt_i18n::t("log_current_file").replace("{path}", &path.display().to_string()),
                )
            } else {
                open
            };
            if open.clicked() {
                open_log_dir();
            }
            // 复制全部（当前可见行）；成功后短时切换为「已复制 N 行」
            let flashing = log.logwin
                .copy_at
                .is_some_and(|t| t.elapsed() < COPY_FLASH);
            let n_visible = log.logwin.visible_count();
            let copy_label = if flashing {
                lt_i18n::t("log_copied").replace("{n}", &n_visible.to_string())
            } else {
                lt_i18n::t("log_copy_all").to_string()
            };
            if ui.button(RichText::new(copy_label).size(12.0)).clicked() {
                let mut text = log.logwin.visible_texts().join("\n");
                text.push('\n');
                ui.ctx().copy_text(text);
                log.logwin.copy_at = Some(Instant::now());
            }
            // 清空（仅清空列表，磁盘日志文件不受影响）
            let clear = ui.button(RichText::new(lt_i18n::t("clear")).size(12.0));
            let clear = clear.on_hover_text(lt_i18n::t("clear_list_only"));
            if clear.clicked() {
                log.logwin.clear();
            }
        });
    });
}

/// ── 日志区：占满剩余高度，全局唯一滚动条 ──
fn log_region(ui: &mut Ui, log: &mut LogUi, pal: &Palette) {
    let show_debug = log.logwin.show_debug;
    let auto_scroll = log.logwin.auto_scroll;
    egui::Frame::NONE.inner_margin(4.0).show(ui, |ui| {
        let out = ScrollArea::vertical()
            .auto_shrink([false, false])
            // D-31 贴底跟随：贴底时新行自动跟到底；用户滚轮/拖动离开即挂起，
            // 拖回底部重新跟随（egui stick_to_bottom 语义）
            .stick_to_bottom(auto_scroll)
            .show(ui, |ui| {
                let lines = log.logwin.formatted();
                let mut any_visible = false;
                for (text, level) in lines {
                    if !LogWindowState::level_visible(*level, show_debug) {
                        continue;
                    }
                    any_visible = true;
                    ui.label(
                        RichText::new(text)
                            .monospace()
                            .size(11.5)
                            .color(level_color(*level, pal)),
                    );
                }
                if !any_visible {
                    ui.label(
                        RichText::new(lt_i18n::t("log_empty"))
                            .monospace()
                            .size(11.5)
                            .color(pal.weak),
                    );
                }
            });
        // 用户主动翻离底部且出现未读行：右下角「回到最新（+N）」浮钮
        if log.logwin.advance_follow(
            out.state.offset.y,
            out.inner_rect.height(),
            out.content_size.y,
            LogView::Panel,
        ) && log_jump_button(ui, log.logwin.new_since_bottom())
        {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
            log.logwin.mark_at_bottom();
        }
    });
}

/// 打开日志目录（explorer；与面板 open_in_explorer 同语义）
fn open_log_dir() {
    match lt_models::paths::logs_dir() {
        Ok(dir) => {
            super::open_in_explorer(&dir);
        }
        Err(e) => {
            tracing::warn!("日志目录不可用: {e}");
        }
    }
}

/// 最新会话日志文件（按名称时间戳字典序取最后；目录不存在 → None）
fn latest_log_file() -> Option<std::path::PathBuf> {
    let dir = lt_models::paths::logs_dir().ok()?;
    latest_log_file_in(&dir)
}

/// 在指定目录内找最新 livetrans_*.log（测试可直接注入目录；
/// 按文件名时间戳字典序取最大者）
fn latest_log_file_in(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut best: Option<(String, std::path::PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().into_owned();
        if name.starts_with("livetrans_") && name.ends_with(".log")
            && best.as_ref().is_none_or(|(bn, _)| *bn < name) {
                best = Some((name, p));
            }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LogLineEntry;

    #[test]
    fn latest_log_file_picks_lexicographic_max() {
        let dir = std::env::temp_dir().join(format!("lt_logtab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("livetrans_20260907_100000.log"), b"").unwrap();
        std::fs::write(dir.join("livetrans_20260907_110000.log"), b"").unwrap();
        std::fs::write(dir.join("other.log"), b"").unwrap();

        let got = latest_log_file_in(&dir).unwrap();
        assert_eq!(
            got.file_name().unwrap().to_string_lossy(),
            "livetrans_20260907_110000.log"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 防回归（LT-1）：固定高度下渲染本页后不得有尾随内容——有尾随内容即
    /// 重新引入 panel 页面级滚动条（双滚动条根因，见 docs/archive/log-tab-redesign.md §2.2）。
    #[test]
    fn log_tab_leaves_no_tailing_content() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        let render_once = |ctx: &egui::Context, st: &mut crate::state::AppUi| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_size(egui::vec2(500.0, 600.0));
                page(ui, &mut st.log, &Palette::NATIVE);
                let remaining = ui.available_height();
                assert!(
                    remaining > -1.0,
                    "日志页出现尾随内容（可用高 {remaining} < 0，会重新引入外层滚动条）"
                );
            });
            // epaint debug 断言要求消费纹理增量（无渲染器 → 显式丢弃，同 panel smoke）
            out.textures_delta.clear();
        };
        // 满载（其中混有 DEBUG 行，覆盖渲染期过滤路径）
        for i in 0..80 {
            st.log.logwin.push(LogLineEntry {
                time: "10:00:00".into(),
                level: if i % 5 == 0 { 10 } else { 20 },
                target: "t".into(),
                msg: format!("line {i}"),
            });
        }
        render_once(&ctx, &mut st);
        render_once(&ctx, &mut st);
        // 空态
        st.log.logwin.clear();
        render_once(&ctx, &mut st);
    }

    /// 状态冒烟：show_debug 回溯显示 + auto_scroll 开关 + 复制反馈计时——两帧渲染不 panic。
    #[test]
    fn log_tab_state_switches_render_stable() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        for i in 0..60 {
            st.log.logwin.push(LogLineEntry {
                time: "10:00:00".into(),
                level: 10,
                target: "t".into(),
                msg: format!("debug {i}"),
            });
        }
        for frame in 0..4 {
            st.log.logwin.show_debug = frame % 2 == 0;
            st.log.logwin.auto_scroll = frame % 2 == 0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_max_size(egui::vec2(500.0, 600.0));
                page(ui, &mut st.log, &Palette::NATIVE);
            });
            out.textures_delta.clear();
        }
        // 复制反馈：copy_at 失效期外仍渲染稳定
        st.log.logwin.copy_at = Some(std::time::Instant::now());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_max_size(egui::vec2(500.0, 600.0));
            page(ui, &mut st.log, &Palette::NATIVE);
        });
        out.textures_delta.clear();
    }
}
