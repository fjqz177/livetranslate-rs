//! 数据与存储页（对照原版 panel/tabs/cache_tab.py）：转录自动保存复选
//! （经 300ms 防抖 ApplySettings → transcript_shared().set_enabled）与打开转录
//! 目录；模型缓存总量与缓存列表（按注册表扫描，每行"名 — 大小"）、删除所选、
//! 删除全部、打开模型目录、刷新。
//!
//! 与原版的差异：
//! - 原版扫描在后台线程 + _cache_result 信号；本版 UI 线程同步扫描（进入页面
//!   或点刷新触发，注册表条目数固定、耗时可接受），避免给 AppState 加事件源；
//! - `log_transcript` 复选不做（settings 契约无该键，不得私改）；
//! - "删除全部并退出"：确认后删除全部缓存并提示重启生效，**不退出应用**
//!   （Rust 版窗口即应用，退出语义过重；原版 QApplication.quit() 为已知偏差）。

use super::{group_card, hint_line, mark_settings_dirty, Palette};
use crate::state::{AppState, CacheEntry};
use egui::{RichText, Ui};

// ── 缓存扫描（原版 get_cache_entries + dir_size 的 Rust 化） ──

/// 递归目录字节数（原版 dir_size：文件 stat.len() 累加；不可读条目跳过）
pub fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(meta) = e.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// 按注册表扫描已缓存模型目录（原版 get_cache_entries 语义：
/// funasr 双 hub 各记一条 "(ModelScope)"/"(HuggingFace)"；whisper 六档共用
/// 单仓 → 合并为一条；无任何命中 → 空表）。
///
/// AH-6/H10：清单从注册表派生（含 qwen3——此前手写清单漏掉该条目，
/// 941MB 模型在应用内不可见不可删），目录名经 `cache::hf_repo_dir` 单一
/// 拼装源，不再手写 `models--` 格式串。
pub fn scan_cache_entries(models_dir: &std::path::Path) -> Vec<CacheEntry> {
    let mut out: Vec<CacheEntry> = Vec::new();
    let mut push_entry = |display: &str, ms: Option<&'static str>, hf: Option<&'static str>| {
        if let Some(repo) = ms {
            let p = lt_models::cache::ms_model_path(models_dir, repo);
            if p.exists() {
                out.push(CacheEntry {
                    name: format!("{display} (ModelScope)"),
                    path: p,
                    size: 0,
                });
            }
        }
        if let Some(repo) = hf {
            let p = lt_models::cache::hf_repo_dir(models_dir, repo);
            if p.exists() {
                out.push(CacheEntry {
                    name: format!("{display} (HuggingFace)"),
                    path: p,
                    size: 0,
                });
            }
        }
    };
    // funasr 系（注册表键序；mlt 无上游条目自然跳过）
    for key in lt_models::registry::FUNASR_KEYS {
        if let Some(entry) = lt_models::registry::funasr_entry(key) {
            push_entry(entry.display, entry.ms, entry.hf);
        }
    }
    // qwen3（单一模型，B-α）
    let qwen3 = lt_models::registry::qwen3_entry();
    push_entry(qwen3.display, qwen3.ms, qwen3.hf);
    // whisper：六档共用单仓（Rust 版下载布局 always HF）→ 合并为一条，
    // 避免按档多条对同一目录重复计数；仓 id 取自注册表
    if let Some(repo) = lt_models::registry::whisper_repo("tiny") {
        let p = lt_models::cache::hf_repo_dir(models_dir, repo);
        if p.exists() {
            out.push(CacheEntry {
                name: "Whisper (HuggingFace)".into(),
                path: p,
                size: 0,
            });
        }
    }
    for e in &mut out {
        e.size = dir_size(&e.path);
    }
    out
}

// ── UI ──

/// 数据与存储页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // ── 转录持久化（原版 ts_group）──
    group_card(ui, pal, &lt_i18n::t("group_transcript"), |ui| {
        ui.horizontal(|ui| {
            let mut auto_save = state.settings.auto_save_transcript;
            if ui
                .add(
                    egui::Checkbox::new(
                        &mut auto_save,
                        RichText::new(lt_i18n::t("label_auto_save_transcript")).color(pal.text),
                    ),
                )
                .on_hover_text(lt_i18n::t("auto_save_transcript_tooltip"))
                .changed()
            {
                state.settings.auto_save_transcript = auto_save;
                // 300ms 防抖 ApplySettings → shell transcript_shared().set_enabled
                mark_settings_dirty(state);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_open_transcripts")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                open_transcripts_dir();
            }
        });
    });

    // ── 模型缓存（原版 top_row + cache_list + manage_row）──
    group_card(ui, pal, &lt_i18n::t("group_model_cache"), |ui| {
        // 首次进入页面同步扫描（None=未扫描）
        if state.panel.cache_entries.is_none() {
            refresh_cache(state);
        }
        let entries = state.panel.cache_entries.clone().unwrap_or_default();
        let total: u64 = entries.iter().map(|e| e.size).sum();

        // 总量行 + 打开目录 + 删除全部
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(lt_i18n::t("cache_total").replace("{size}", &super::vad::format_size(total)).replace("{count}", &entries.len().to_string()))
                    .monospace()
                    .strong()
                    .color(pal.text),
            );
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_open_folder")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                open_models_dir(state);
            }
            let delete_all = ui
                .add_enabled(
                    !entries.is_empty(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_delete_all_exit")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked();
            if delete_all {
                delete_all_models(state, &entries);
            }
        });

        ui.add_space(4.0);
        // 缓存列表（Consolas 摘要；点击空行/再次点击取消选中由 select 语义承担）
        let mut select: Option<usize> = None;
        for (i, e) in entries.iter().enumerate() {
            let text = format!("{}  —  {}", e.name, super::vad::format_size(e.size));
            let resp = ui
                .push_id(i, |ui| {
                    ui.add(
                        egui::Button::selectable(
                            state.panel.cache_selected == Some(i),
                            RichText::new(&text).monospace().size(12.0),
                        )
                        .corner_radius(4.0)
                        .min_size(egui::vec2(ui.available_width(), 0.0)),
                    )
                })
                .inner;
            if resp.clicked() {
                select = if state.panel.cache_selected == Some(i) { None } else { Some(i) };
            }
        }
        if entries.is_empty() {
            hint_line(ui, pal, &lt_i18n::t("no_cached_models"));
        }
        if select.is_some() {
            state.panel.cache_selected = select;
        }

        ui.add_space(4.0);
        // 管理行：删除所选 + 刷新 + 提示
        ui.horizontal(|ui| {
            let del = ui
                .add_enabled(
                    state.panel.cache_selected.is_some(),
                    egui::Button::new(RichText::new(lt_i18n::t("btn_delete_selected")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked();
            if del {
                delete_selected(state);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_refresh")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                refresh_cache(state);
            }
            hint_line(ui, pal, &lt_i18n::t("cache_select_hint"));
        });
    });

    ui.add_space(8.0);
}

/// 重新扫描缓存（UI 线程同步；扫描前重置选中）
fn refresh_cache(state: &mut AppState) {
    state.panel.cache_selected = None;
    let dir = lt_models::paths::models_dir(state.settings.models_dir.as_deref())
        .unwrap_or_else(|_| std::env::temp_dir());
    state.panel.cache_entries = Some(scan_cache_entries(&dir));
}

/// 打开转录目录（原版 _open_transcripts_folder：mkdir 兜底 + open_path）
fn open_transcripts_dir() {
    let dir = lt_models::paths::transcripts_dir().unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    super::open_in_explorer(&dir);
    tracing::info!("打开转录目录: {}", dir.display());
}

/// 打开模型目录（原版 _open_models_folder：mkdir 兜底 + open_path）
fn open_models_dir(state: &AppState) {
    let dir = lt_models::paths::models_dir(state.settings.models_dir.as_deref())
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    super::open_in_explorer(&dir);
    tracing::info!("打开模型目录: {}", dir.display());
}

/// 删除所选模型（原版 _delete_selected：warning 确认 → rmtree → 刷新）。
/// D-33/H-5：确认改 egui 模态（rfd 同步框阻塞事件循环线程；确认后执行见
/// [`apply_delete_selected`]）。
fn delete_selected(state: &mut AppState) {
    let entries = state.panel.cache_entries.clone().unwrap_or_default();
    let Some(i) = state.panel.cache_selected else { return };
    let Some(e) = entries.get(i) else { return };
    let msg = lt_i18n::t("delete_selected_confirm_msg")
        .replace("{name}", &e.name)
        .replace("{size}", &super::vad::format_size(e.size));
    state.request_confirm(
        crate::state::ConfirmKind::DeleteModel { index: i },
        false,
        lt_i18n::t("delete_selected_confirm_title"),
        msg,
    );
}

/// 模态确认后的执行：删除索引条目并刷新缓存（索引对位请求时的 cache_entries）
pub(crate) fn apply_delete_selected(state: &mut AppState, index: usize) {
    let entries = state.panel.cache_entries.clone().unwrap_or_default();
    if let Some(e) = entries.get(index) {
        remove_entry(e);
    }
    refresh_cache(state);
}

/// 删除全部缓存（原版 _delete_all_and_exit：warning 确认 → 逐个 rmtree）。
/// Rust 版不退出应用，改为"重启生效"原生通知（见模块注释偏差说明）。
/// D-33/H-5：确认与完成提示均改非阻塞载体（egui 模态 + 原生通知）。
fn delete_all_models(state: &mut AppState, entries: &[CacheEntry]) {
    let total: u64 = entries.iter().map(|e| e.size).sum();
    let msg = lt_i18n::t("dialog_delete_msg")
        .replace("{count}", &entries.len().to_string())
        .replace("{size}", &super::vad::format_size(total));
    state.request_confirm(
        crate::state::ConfirmKind::DeleteAll,
        false,
        lt_i18n::t("dialog_delete_title"),
        msg,
    );
}

/// 模态确认后的执行：逐条删除并刷新 + 完成提示走原生通知
pub(crate) fn apply_delete_all(state: &mut AppState) {
    let entries = state.panel.cache_entries.clone().unwrap_or_default();
    for e in &entries {
        remove_entry(e);
    }
    refresh_cache(state);
    let done = lt_i18n::t("delete_all_done_msg").replace("{count}", &entries.len().to_string());
    if let Err(e) = crate::notifications::show(&lt_i18n::t("dialog_delete_title"), &done) {
        tracing::warn!("删除完成提示（原生通知）失败: {e}");
    }
}

/// 删除单个缓存目录（原版 shutil.rmtree：失败记日志，不中断其余条目）
fn remove_entry(e: &CacheEntry) {
    match std::fs::remove_dir_all(&e.path) {
        Ok(()) => tracing::info!("已删除: {}", e.path.display()),
        Err(err) => tracing::error!("删除失败 {}: {err}", e.path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lt_models::paths::{hf_cache_root, ms_cache_root};
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("lt_panel_data_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    /// dir_size 递归累加（子目录 + 文件）
    #[test]
    fn dir_size_sums_nested_files() {
        let dir = tmpdir("size");
        fs::write(dir.join("a.bin"), vec![0u8; 100]).unwrap();
        let sub = dir.join("snapshots").join("rev1");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("model.onnx"), vec![0u8; 1000]).unwrap();
        fs::write(sub.join("tokens.txt"), b"abc").unwrap();
        assert_eq!(dir_size(&dir), 1103);
        // 空目录 = 0；不存在 = 0
        assert_eq!(dir_size(&dir.join("empty-missing")), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 缓存扫描：MS/HF 各记一条 + whisper 单仓合并；体积为目录实际字节
    #[test]
    fn scan_cache_entries_finds_registered_models() {
        let dir = tmpdir("scan");
        // MS 侧 SenseVoice（下载器布局 models/{org}--{name}）
        let ms = ms_cache_root(&dir).join("models").join("pengzhendong--sherpa-onnx-sense-voice-zh-en-ja-ko-yue").join("snapshots").join("rev");
        fs::create_dir_all(&ms).unwrap();
        fs::write(ms.join("model.int8.onnx"), vec![0u8; 500]).unwrap();
        // HF 侧 SenseVoice（models--org--name）
        let hf = hf_cache_root(&dir)
            .join("models--csukuangfj--sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
        fs::create_dir_all(&hf).unwrap();
        fs::write(hf.join("x"), vec![0u8; 10]).unwrap();

        let entries = scan_cache_entries(&dir);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"SenseVoice Small (ModelScope)"), "{names:?}");
        assert!(names.contains(&"SenseVoice Small (HuggingFace)"), "{names:?}");
        // whisper 六档共用单仓 → 至多一条
        assert_eq!(names.iter().filter(|n| n.starts_with("Whisper")).count(), 0, "未下载不应出现");
        // 体积为实际字节（MS 目录 500B）
        let ms_entry = entries.iter().find(|e| e.name.ends_with("(ModelScope)")).unwrap();
        assert_eq!(ms_entry.size, 500);

        // whisper 单仓出现时合并为一条
        let wh = hf_cache_root(&dir).join("models--ggerganov--whisper.cpp");
        fs::create_dir_all(&wh).unwrap();
        let entries = scan_cache_entries(&dir);
        assert_eq!(
            entries.iter().filter(|e| e.name.starts_with("Whisper")).count(),
            1,
            "whisper 单仓应合并为一条"
        );
        // 删除条目路径可用（remove_dir_all 目标即扫描路径）
        let wh_entry = entries.iter().find(|e| e.name.starts_with("Whisper")).unwrap();
        assert!(wh_entry.path.exists());

        // AH-6/H9 回归钉：qwen3 必须在扫描清单内——此前手写清单漏掉该条目，
        // 941MB 模型在数据页不可见、不可删（含"删除全部"）
        let q = lt_models::registry::qwen3_entry();
        let (org, name) = q.hf.expect("qwen3 须有 HF 源").split_once('/').unwrap();
        let qh = hf_cache_root(&dir).join(format!("models--{org}--{name}"));
        fs::create_dir_all(&qh).unwrap();
        fs::write(qh.join("conv_frontend.onnx"), vec![0u8; 1234]).unwrap();
        let entries = scan_cache_entries(&dir);
        let q_entry = entries
            .iter()
            .find(|e| e.name.starts_with("Qwen3-ASR-0.6B"))
            .expect("qwen3 缓存必须对数据页可见（AH-6/H9）");
        assert_eq!(q_entry.size, 1234);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 空目录 → 无条目（UI 层显示 no_cached_models 文案）
    #[test]
    fn scan_cache_entries_empty_dir_yields_nothing() {
        let dir = tmpdir("empty");
        assert!(scan_cache_entries(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
