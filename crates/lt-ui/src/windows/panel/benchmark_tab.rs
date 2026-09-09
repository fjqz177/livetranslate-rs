//! 基准测试 Tab（对照原版 control_panel.py `_create_benchmark_tab`）：
//! 源语言/目标语言下拉 + 「测试全部模型」按钮 + 只读等宽输出区（深底浅字，
//! 原版 QTextEdit `background:#1e1e2e; color:#cdd6f4`）。
//!
//! 复用 bench 工具窗的同一数据通路（`AppState.bench_*` 状态 +
//! `start_benchmark` 后台线程 + LogLine 回流），Tab 与工具窗任一入口触发、
//! 状态共享。原版 Tab 版无模型多选（跑全部模型），按钮语义 = 全选。

use super::{group_card, Palette};
use crate::state::{BenchUi, Settings};
use crate::windows::bench::{self, BENCH_SRC_LANGS, BENCH_TGT_LANGS, LOG_BG, LOG_FG};
use egui::{Color32, RichText, ScrollArea, Ui};

/// 基准测试 Tab UI 总入口（panel_ui 按 PanelPage::Benchmark 分派）
pub fn page(ui: &mut Ui, settings: &mut Settings, bench: &mut BenchUi, pal: &Palette) {
    // 模型勾选与 settings.models 对位（Tab 版 = 全部勾选，原版 _run_benchmark 语义）
    bench::align_selection(&mut bench.selected, settings.models.len());

    // ── 控制行（原版 ctrl_row：源语言 / 目标语言 / stretch / 测试全部模型）──
    group_card(ui, pal, &lt_i18n::t("group_benchmark"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_source"))).color(pal.text));
            lang_combo(
                ui,
                "bench_tab_src",
                &mut bench.src,
                &BENCH_SRC_LANGS,
                pal,
            );
            ui.add_space(8.0);
            ui.label(RichText::new(format!("{} ", lt_i18n::t("target_label"))).color(pal.text));
            lang_combo(
                ui,
                "bench_tab_tgt",
                &mut bench.tgt,
                &BENCH_TGT_LANGS,
                pal,
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn_text = if bench.running {
                    lt_i18n::t("testing")
                } else {
                    lt_i18n::t("btn_test_all")
                };
                if ui
                    .add_enabled(
                        !bench.running,
                        egui::Button::new(RichText::new(btn_text).size(12.5)),
                    )
                    .clicked()
                {
                    bench::start_benchmark_public(bench, settings);
                }
            });
        });
        if settings.models.is_empty() {
            ui.label(
                RichText::new(lt_i18n::t("bench_no_models_hint"))
                    .size(11.5)
                    .color(pal.weak),
            );
        }
    });

    // ── 输出区（原版 QTextEdit 深底等宽；W2 起行流不再含 完成哨兵）──
    egui::Frame::NONE
        .fill(LOG_BG)
        .corner_radius(0.0)
        .stroke(egui::Stroke::new(1.0, pal.card_stroke))
        .inner_margin(6.0)
        .show(ui, |ui| {
            let min_h = (ui.available_height() - 8.0).max(200.0);
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(min_h);
                    // 原版空态为空白输出区
                    for line in &bench.lines {
                        let color = if line.contains("FAILED") || line.contains("FAIL ") {
                            Color32::from_rgb(0xf4, 0x47, 0x47)
                        } else if line.contains("OK") || line.contains("✓") {
                            Color32::from_rgb(0x5f, 0xc9, 0x5f)
                        } else {
                            LOG_FG
                        };
                        ui.label(RichText::new(line).monospace().size(12.0).color(color));
                    }
                    // 运行中新行到达时贴底（原版 append 自动滚到底）
                    if bench.running {
                        ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                    }
                });
        });
}

/// 语言下拉（原版 QComboBox 直接列语言码）
fn lang_combo(ui: &mut Ui, id: &str, idx: &mut usize, items: &[&str], pal: &Palette) {
    let i = (*idx).min(items.len() - 1);
    egui::ComboBox::from_id_salt(id)
        .selected_text(items[i].to_string())
        .width(80.0)
        .show_ui(ui, |ui| {
            for (j, code) in items.iter().enumerate() {
                if ui.selectable_label(i == j, (*code).to_string()).clicked() && i != j {
                    *idx = j;
                }
            }
        });
    let _ = pal; // 保持签名一致（预留提示色）
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 语言码表与原版 addItem 一致（_bench_lang 6 项 / _bench_target 8 项）
    #[test]
    fn bench_lang_tables_match_original() {
        assert_eq!(BENCH_SRC_LANGS, ["ja", "en", "zh", "ko", "fr", "de"]);
        assert_eq!(
            BENCH_TGT_LANGS,
            ["zh", "en", "ja", "ko", "fr", "de", "es", "ru"]
        );
    }
}
