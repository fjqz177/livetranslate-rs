//! 性能基准独立工具窗（对照原版 panel/tabs/benchmark_dialog.py + benchmark_tab.py；
//! 由识别页页头"性能基准…"按钮打开，WinId::Benchmark 常规装饰窗口）：
//! 源/目标语言下拉 + 模型多选（settings.models 全部默认勾选）+ 开始按钮 +
//! 只读输出区（Consolas）+ 关闭按钮。
//!
//! 数据流（W5f/R22）：本窗只做**编排请求**——点开始 → `Cmd::RunBench`
//! （模型快照/语言/超时/prompt 类型化载荷）→ 编排域 Supervisor 一次性线程
//! 跑 lt-translate crate 的 run_benchmark → 输出经事件动脉 `UiEvent::Bench`
//! 回流，UI 在 app.rs 的 Bench 分支追加行/复位运行态并弹完成提示（W2 起
//! 不借道日志总线，无完成哨兵；event_tx 后台旁路随本波删除）。

use crate::state::{BenchUi, SessionView, Settings};
use egui::{Color32, RichText, ScrollArea, Ui};
use lt_proto::{Cmd, ModelConfig};

/// 源语言下拉项（原版 _bench_lang）
pub const BENCH_SRC_LANGS: [&str; 6] = ["ja", "en", "zh", "ko", "fr", "de"];
/// 目标语言下拉项（原版 _bench_target）
pub const BENCH_TGT_LANGS: [&str; 8] = ["zh", "en", "ja", "ko", "fr", "de", "es", "ru"];

/// 输出区底色/文字色（原版 QTextEdit 样式 background #1e1e2e / color #cdd6f4）
pub const LOG_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
pub const LOG_FG: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);

// ── 纯逻辑（单测覆盖） ──

/// 基准 prompt（原版 _run_benchmark：system_prompt 缺省回退 DEFAULT_PROMPT，
/// 再按显示名填充 {source_lang}/{target_lang} 占位）
pub fn bench_prompt(settings_prompt: &str, src: &str, tgt: &str) -> String {
    let template = if settings_prompt.trim().is_empty() {
        lt_proto::DEFAULT_PROMPT
    } else {
        settings_prompt
    };
    template
        .replace("{source_lang}", lt_proto::language_display(src))
        .replace("{target_lang}", lt_proto::language_display(tgt))
}

/// 勾选表与模型表对位（新增模型默认勾选；多余勾选截断）
pub fn align_selection(selected: &mut Vec<bool>, models_len: usize) {
    if selected.len() < models_len {
        selected.resize(models_len, true);
    } else if selected.len() > models_len {
        selected.truncate(models_len);
    }
}

// ── UI ──

/// Benchmark 窗 UI 总入口（windows::dispatch 按 WinId::Benchmark 分派到这里）
pub fn bench_ui(ui: &mut Ui, bench: &mut BenchUi, session: &mut SessionView, settings: &mut Settings) {
    align_selection(&mut bench.selected, settings.models.len());

    // ── 控制行（原版 ctrl_row：源语言/目标语言/开始）──
    ui.horizontal(|ui| {
        ui.label(RichText::new(lt_i18n::t("label_source")).size(12.5));
        lang_combo(ui, "bench_src", &mut bench.src, &BENCH_SRC_LANGS);
        ui.label(RichText::new(lt_i18n::t("target_label")).size(12.5));
        lang_combo(ui, "bench_tgt", &mut bench.tgt, &BENCH_TGT_LANGS);
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
            start_benchmark(bench, session, settings);
        }
    });

    // ── 模型多选（原版跑全部模型；本版按勾选，缺省全选）──
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong(lt_i18n::t("group_bench_models"));
        for (i, m) in settings.models.iter().enumerate() {
            let mut checked = bench.selected.get(i).copied().unwrap_or(true);
            let resp = ui.add_enabled(
                !bench.running,
                egui::Checkbox::new(&mut checked, RichText::new(&m.name).monospace().size(12.0)),
            );
            if resp.changed() {
                bench.selected[i] = checked;
            }
        }
    });

    // ── 输出区（只读 Consolas）──
    egui::Frame::NONE
        .fill(LOG_BG)
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(ui.available_height().max(120.0));
                    for line in &bench.lines {
                        let text = RichText::new(line).monospace().size(12.0).color(LOG_FG);
                        if line.starts_with("  FAILED") || line.starts_with("  FAIL ") {
                            ui.label(
                                RichText::new(line)
                                    .monospace()
                                    .size(12.0)
                                    .color(Color32::from_rgb(0xf4, 0x47, 0x47)),
                            );
                        } else {
                            ui.label(text);
                        }
                    }
                    // 追加期自动滚底（原版 QTextEdit append + 滚动条推底）
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                });
        });

    // ── 关闭行（原版 row + close_btn）。E6：取消按钮接线——W5f 取消链
    //    （shell bench_cancel → translate 模型边界截停）早已闭环，唯缺 UI
    //    生产者（死契约守卫校准发现）；运行中显示取消按钮 ──
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(lt_i18n::t("btn_close")).clicked() {
            session.enqueue_action(
                crate::state::WinId::Benchmark,
                crate::state::WinAction::Hide,
            );
        }
        if bench.running && ui.button(lt_i18n::t("btn_cancel_bench")).clicked() {
            session.send_cmd(Cmd::CancelBench);
            tracing::info!("请求取消基准（模型边界截停）");
        }
    });
}

/// 源/目标语言下拉（索引写回 state；紧凑宽度）
fn lang_combo(ui: &mut Ui, id: &str, index: &mut usize, langs: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(langs[*index].to_string())
        .width(76.0)
        .show_ui(ui, |ui| {
            for (i, code) in langs.iter().enumerate() {
                if ui.selectable_label(*index == i, code.to_string()).clicked() && *index != i {
                    *index = i;
                }
            }
        });
}

/// 开始基准（W5f/R22）：编排请求——快照模型/语言/超时/prompt 进类型化命令
/// 发给 shell（编排域 Supervisor 一次性线程执行；输出经动脉回流）。
/// Tab 版/工具窗版共用入口（原版 Tab 版跑全部模型，勾选表已对位）
pub fn start_benchmark_public(
    bench: &mut BenchUi,
    session: &mut SessionView,
    settings: &mut Settings,
) {
    start_benchmark(bench, session, settings);
}

fn start_benchmark(bench: &mut BenchUi, session: &mut SessionView, settings: &mut Settings) {
    if bench.running {
        return;
    }
    let models: Vec<ModelConfig> = settings
        .models
        .iter()
        .enumerate()
        .filter(|(i, _)| bench.selected.get(*i).copied().unwrap_or(true))
        .map(|(_, m)| m.clone())
        .collect();
    if models.is_empty() {
        return;
    }
    let src = BENCH_SRC_LANGS[bench.src.min(BENCH_SRC_LANGS.len() - 1)];
    let tgt = BENCH_TGT_LANGS[bench.tgt.min(BENCH_TGT_LANGS.len() - 1)];
    let timeout = settings.timeout.max(1);
    let prompt = bench_prompt(&settings.system_prompt, src, tgt);

    bench.lines.clear();
    bench.running = true;
    session.send_cmd(Cmd::RunBench {
        models,
        src: src.to_string(),
        tgt: tgt.to_string(),
        timeout,
        prompt,
    });
    tracing::info!("性能基准已启动（{src} → {tgt}，timeout={timeout}s）");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 基准 prompt：缺省回退 DEFAULT_PROMPT + 显示名占位填充
    #[test]
    fn bench_prompt_fills_display_names() {
        assert_eq!(
            bench_prompt("", "ja", "zh"),
            lt_proto::DEFAULT_PROMPT
                .replace("{source_lang}", "Japanese")
                .replace("{target_lang}", "Chinese")
        );
        let custom = "Translate {source_lang} to {target_lang} now.";
        assert_eq!(
            bench_prompt(custom, "en", "ru"),
            "Translate English to Russian now."
        );
        // 未知码原样返回（language_display 兜底）
        assert_eq!(bench_prompt("x {source_lang}", "zz", "zh"), "x zz");
    }

    /// 勾选表与模型表对位：新增默认勾选、删除截断
    #[test]
    fn align_selection_resizes_with_default_true() {
        let mut sel = Vec::new();
        align_selection(&mut sel, 2);
        assert_eq!(sel, vec![true, true]);
        sel[0] = false;
        align_selection(&mut sel, 3);
        assert_eq!(sel, vec![false, true, true], "新增模型默认勾选");
        align_selection(&mut sel, 1);
        assert_eq!(sel, vec![false], "删除模型后截断");
        align_selection(&mut sel, 0);
        assert!(sel.is_empty());
    }

    /// 语言表形状（对照原版两个下拉的 addItems）
    #[test]
    fn bench_lang_tables_match_original() {
        assert_eq!(BENCH_SRC_LANGS, ["ja", "en", "zh", "ko", "fr", "de"]);
        assert_eq!(
            BENCH_TGT_LANGS,
            ["zh", "en", "ja", "ko", "fr", "de", "es", "ru"]
        );
    }

    /// 基准窗无头渲染冒烟：带模型勾选与输出行跑两帧不 panic；
    /// 行流（原 LogLine[benchmark] 模拟）→ bench_lines 追加 + Finished 复位运行态
    #[test]
    fn bench_ui_smoke_and_bench_line_flow() {
        let ctx = egui::Context::default();
        let mut st = crate::state::AppUi::new(lt_proto::Settings::default());
        align_selection(&mut st.bench.selected, st.settings.models.len());
        // 模拟 app.rs 的 Bench 消费路径
        st.bench.running = true;
        st.bench.push_line("Testing 1 model(s)".into());
        st.bench.push_line("  FAILED: timeout".into());
        // Finished（ok=false）复位运行态
        st.bench.running = false;
        assert!(!st.bench.running);
        assert!(st.bench.lines.len() == 2);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                bench_ui(ui, &mut st.bench, &mut st.session, &mut st.settings)
            });
            assert!(!out.shapes.is_empty(), "基准窗应产出图元");
            out.textures_delta.clear();
        }
        assert_eq!(st.bench.lines.len(), 2);
        assert!(!st.bench.running);
    }
}
