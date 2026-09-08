//! 性能基准独立工具窗（对照原版 panel/tabs/benchmark_dialog.py + benchmark_tab.py；
//! 由识别页页头"性能基准…"按钮打开，WinId::Benchmark 常规装饰窗口）：
//! 源/目标语言下拉 + 模型多选（settings.models 全部默认勾选）+ 开始按钮 +
//! 只读输出区（Consolas，含 "__DONE__" 停止标记）+ 关闭按钮。
//!
//! 数据流（egui 无跨线程句柄；本版事件通道复用方案）：
//! 后台线程经 `lt_translate::bench::run_benchmark` 测试，on_line 闭包通过
//! `AppState.event_tx`（EventLoopProxy 转发）回流
//! `UiEvent::LogLine { target: "benchmark", msg }`——日志窗照常显示（独立窗 +
//! 日志双显），UI 线程在 app.rs 的 LogLine 分支把 benchmark 行同步追加到
//! `AppState.bench_lines` 渲染，`__DONE__` 到达即复位运行态并弹完成提示。
//! 契约冻结：不新增 UiEvent 变体。

use crate::state::AppState;
use egui::{Color32, RichText, ScrollArea, Ui};
use lt_proto::{ModelConfig, UiEvent, UiMsg};
use lt_translate::bench::{run_benchmark, BenchModel};

/// 源语言下拉项（原版 _bench_lang）
pub const BENCH_SRC_LANGS: [&str; 6] = ["ja", "en", "zh", "ko", "fr", "de"];
/// 目标语言下拉项（原版 _bench_target）
pub const BENCH_TGT_LANGS: [&str; 8] = ["zh", "en", "ja", "ko", "fr", "de", "es", "ru"];

/// 输出区底色/文字色（原版 QTextEdit 样式 background #1e1e2e / color #cdd6f4）
pub const LOG_BG: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x2e);
pub const LOG_FG: Color32 = Color32::from_rgb(0xcd, 0xd6, 0xf4);

// ── 纯逻辑（单测覆盖） ──

/// ModelConfig → BenchModel（基准所需子集：连接四要素 + no_system_role）
pub fn to_bench_model(m: &ModelConfig) -> BenchModel {
    BenchModel {
        name: m.name.clone(),
        api_base: m.api_base.clone(),
        api_key: m.api_key.clone(),
        model: m.model.clone(),
        proxy: m.proxy.clone(),
        no_system_role: m.no_system_role,
    }
}

/// 基准 prompt（原版 _run_benchmark：system_prompt 缺省回退 DEFAULT_PROMPT，
/// 再按显示名填充 {source_lang}/{target_lang} 占位）
pub fn bench_prompt(settings_prompt: &str, src: &str, tgt: &str) -> String {
    let template = if settings_prompt.trim().is_empty() {
        lt_translate::DEFAULT_PROMPT
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
pub fn bench_ui(ui: &mut Ui, state: &mut AppState) {
    align_selection(&mut state.bench_selected, state.settings.models.len());

    // ── 控制行（原版 ctrl_row：源语言/目标语言/开始）──
    ui.horizontal(|ui| {
        ui.label(RichText::new(lt_i18n::t("label_source")).size(12.5));
        lang_combo(ui, "bench_src", &mut state.bench_src, &BENCH_SRC_LANGS);
        ui.label(RichText::new(lt_i18n::t("target_label")).size(12.5));
        lang_combo(ui, "bench_tgt", &mut state.bench_tgt, &BENCH_TGT_LANGS);
        let btn_text = if state.bench_running {
            lt_i18n::t("testing")
        } else {
            lt_i18n::t("btn_test_all")
        };
        if ui
            .add_enabled(
                !state.bench_running,
                egui::Button::new(RichText::new(btn_text).size(12.5)),
            )
            .clicked()
        {
            start_benchmark(state);
        }
    });

    // ── 模型多选（原版跑全部模型；本版按勾选，缺省全选）──
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.strong(lt_i18n::t("group_bench_models"));
        for (i, m) in state.settings.models.iter().enumerate() {
            let mut checked = state.bench_selected.get(i).copied().unwrap_or(true);
            let resp = ui.add_enabled(
                !state.bench_running,
                egui::Checkbox::new(&mut checked, RichText::new(&m.name).monospace().size(12.0)),
            );
            if resp.changed() {
                state.bench_selected[i] = checked;
            }
        }
    });

    // ── 输出区（只读 Consolas；含 __DONE__ 停止提示）──
    egui::Frame::NONE
        .fill(LOG_BG)
        .corner_radius(4.0)
        .inner_margin(6.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(ui.available_height().max(120.0));
                    for line in &state.bench_lines {
                        let text = RichText::new(line).monospace().size(12.0).color(LOG_FG);
                        if line == "__DONE__" {
                            ui.label(
                                RichText::new(line)
                                    .monospace()
                                    .size(12.0)
                                    .color(Color32::GRAY),
                            );
                        } else if line.starts_with("  FAILED") || line.starts_with("  FAIL ") {
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

    // ── 关闭行（原版 row + close_btn）──
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.button(lt_i18n::t("btn_close")).clicked() {
            state.enqueue_action(
                crate::state::WinId::Benchmark,
                crate::state::WinAction::Hide,
            );
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

/// 开始基准（原版 _run_benchmark）：清空输出 → 后台线程测试，
/// on_line 经 event_tx 回流 LogLine{target:"benchmark"}。
/// Tab 版/工具窗版共用入口（原版 Tab 版跑全部模型，勾选表已对位）
pub fn start_benchmark_public(state: &mut AppState) {
    start_benchmark(state);
}

fn start_benchmark(state: &mut AppState) {
    if state.bench_running {
        return;
    }
    let models: Vec<BenchModel> = state
        .settings
        .models
        .iter()
        .enumerate()
        .filter(|(i, _)| state.bench_selected.get(*i).copied().unwrap_or(true))
        .map(|(_, m)| to_bench_model(m))
        .collect();
    if models.is_empty() {
        return;
    }
    let Some(event_tx) = state.event_tx.clone() else {
        tracing::warn!("event_tx 未注入，无法启动基准");
        return;
    };
    let src = BENCH_SRC_LANGS[state.bench_src.min(BENCH_SRC_LANGS.len() - 1)];
    let tgt = BENCH_TGT_LANGS[state.bench_tgt.min(BENCH_TGT_LANGS.len() - 1)];
    let timeout = state.settings.timeout.max(1);
    let prompt = bench_prompt(&state.settings.system_prompt, src, tgt);

    state.bench_lines.clear();
    state.bench_running = true;
    // 原版 run_benchmark：后台线程 + result_callback 逐行回传，末行 "__DONE__"
    run_benchmark(models, src, tgt, timeout, &prompt, move |line: &str| {
        event_tx(UiMsg::Event(UiEvent::LogLine {
            level: 20,
            target: "benchmark".into(),
            msg: line.to_string(),
        }));
    });
    tracing::info!("性能基准已启动（{src} → {tgt}，timeout={timeout}s）");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ModelConfig → BenchModel 字段子集转换
    #[test]
    fn to_bench_model_copies_connection_fields() {
        let cfg = ModelConfig {
            name: "glm".into(),
            api_base: "https://open.bigmodel.cn/api/paas/v4".into(),
            api_key: "k".into(),
            model: "glm-4".into(),
            proxy: "system".into(),
            no_system_role: true,
            ..Default::default()
        };
        let b = to_bench_model(&cfg);
        assert_eq!(b.name, "glm");
        assert_eq!(b.api_base, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(b.api_key, "k");
        assert_eq!(b.model, "glm-4");
        assert_eq!(b.proxy, "system");
        assert!(b.no_system_role);
        // 基准所需之外的字段（价格/overrides/prompt）不参与转换
        let cfg2 = ModelConfig {
            context_turns: 9,
            input_price: 3.0,
            ..cfg
        };
        let b2 = to_bench_model(&cfg2);
        assert_eq!(b2.name, "glm");
        assert_eq!(b2.model, "glm-4");
    }

    /// 基准 prompt：缺省回退 DEFAULT_PROMPT + 显示名占位填充
    #[test]
    fn bench_prompt_fills_display_names() {
        assert_eq!(
            bench_prompt("", "ja", "zh"),
            lt_translate::DEFAULT_PROMPT
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
    /// LogLine(target="benchmark") 回流 → bench_lines 追加 + __DONE__ 复位运行态
    #[test]
    fn bench_ui_smoke_and_bench_line_flow() {
        let ctx = egui::Context::default();
        let mut st = AppState::new(lt_proto::Settings::default());
        align_selection(&mut st.bench_selected, st.settings.models.len());
        // 模拟 app.rs 的 LogLine 消费路径
        st.bench_running = true;
        st.push_bench_line("Testing 1 model(s)".into());
        st.push_bench_line("  FAILED: timeout".into());
        st.push_bench_line("__DONE__".into());
        st.bench_running = false;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| bench_ui(ui, &mut st));
            assert!(!out.shapes.is_empty(), "基准窗应产出图元");
            out.textures_delta.clear();
        }
        assert_eq!(st.bench_lines.len(), 3);
        assert!(!st.bench_running);
    }
}
