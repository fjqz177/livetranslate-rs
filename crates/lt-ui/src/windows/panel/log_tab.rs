//! 设置页「日志」tab（Rust 版新增：用户自查/报障入口）。
//!
//! 与日志窗（logwin.rs）共享同一环形缓冲（AppState.logwin），本界面是第二视图：
//! - 级别色 + 时间 + target + 消息，最新在底部（auto_scroll 跟随）；
//! - 工具行：显示 DEBUG / 清空 / 复制全部 / 打开日志文件；
//! - 数据零新增：LogLine 事件流（logging.rs BroadcastLayer）本来就全量到达 UI。

use super::Palette;
use crate::state::AppState;
use egui::{Color32, RichText, ScrollArea, Ui};

/// 级别色（浅色面板用：INFO 主文字色、DEBUG 弱灰、WARN 琥珀、ERROR 红）
fn level_color(level: u8, pal: &Palette) -> Color32 {
    match level {
        10 => pal.weak,
        30 => pal.warn,
        40..=50 => pal.err,
        _ => pal.text,
    }
}

fn level_name(level: u8) -> &'static str {
    match level {
        0 => "TRACE",
        10 => "DEBUG",
        20 => "INFO",
        30 => "WARNING",
        40.. => "ERROR",
        _ => "INFO",
    }
}

pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // ── 工具行 ──
    ui.horizontal(|ui| {
        ui.add(egui::Checkbox::new(
            &mut state.logwin.show_debug,
            RichText::new(lt_i18n::t("show_debug")).size(12.0),
        ));
        ui.add(egui::Checkbox::new(
            &mut state.logwin.auto_scroll,
            RichText::new(lt_i18n::t("auto_scroll")).size(12.0),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(lt_i18n::t("log_open_file")).clicked() {
                open_log_dir();
            }
            if ui.button(lt_i18n::t("log_copy_all")).clicked() {
                let text = state
                    .logwin
                    .lines
                    .iter()
                    .map(|e| {
                        format!(
                            "{} [{}] {}: {}",
                            e.time,
                            level_name(e.level),
                            e.target,
                            e.msg
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = text.trim_end().to_string() + "\n";
                ui.ctx().copy_text(text);
            }
            if ui.button(lt_i18n::t("clear")).clicked() {
                state.logwin.clear();
            }
        });
    });
    ui.add_space(4.0);

    // ── 日志区（环形 2000 行；仅在 level ≥ INFO 或 show_debug 时行已由
    //    LogWindowState::push 过滤，此处直接渲染）──
    let lines: Vec<(String, Color32)> = state
        .logwin
        .lines
        .iter()
        .map(|e| {
            (
                format!("{} [{}] {}: {}", e.time, level_name(e.level), e.target, e.msg),
                level_color(e.level, pal),
            )
        })
        .collect();
    egui::Frame::NONE
        .inner_margin(4.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (text, color) in &lines {
                        ui.label(RichText::new(text).monospace().size(11.5).color(*color));
                    }
                    if lines.is_empty() {
                        ui.label(
                            RichText::new(lt_i18n::t("log_empty"))
                                .monospace()
                                .size(11.5)
                                .color(pal.weak),
                        );
                    }
                    if state.logwin.auto_scroll && !lines.is_empty() {
                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    }
                });
        });

    // ── 当前会话日志文件提示 ──
    if let Some(path) = latest_log_file() {
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{} {}", lt_i18n::t("log_current_file"), path.display()))
                .size(10.5)
                .color(pal.weak),
        );
    }
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
        if name.starts_with("livetrans_") && name.ends_with(".log") {
            if best.as_ref().map_or(true, |(bn, _)| *bn < name) {
                best = Some((name, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_log_file_picks_lexicographic_max() {
        let dir = std::env::temp_dir().join(format!("lt_logtab_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("livetrans_20260907_100000.log"), b"").unwrap();
        std::fs::write(dir.join("livetrans_20260907_110000.log"), b"").unwrap();
        std::fs::write(dir.join("other.log"), b"").unwrap();

        let got = latest_log_file_in(&dir).unwrap();
        assert_eq!(got.file_name().unwrap().to_string_lossy(), "livetrans_20260907_110000.log");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
