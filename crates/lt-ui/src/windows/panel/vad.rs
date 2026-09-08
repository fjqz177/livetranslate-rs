//! 识别页（对照原版 ui/panel/tabs/vad_tab.py + vad_engine.py + vad_whisper.py）：
//! 引擎/模型/识别语言下拉、扬声器(loopback)/麦克风设备组 + 刷新、VAD 模式与
//! 阈值/时序、Padding（FunASR/Whisper 补零）、模型缓存状态 + 下载。
//!
//! 与原版的差异（Rust 版语义）：
//! - vad_engine.py 的"引擎运行时装/uv sync/一键安装"区块为 Python 打包专属
//!   （frozen 构建 + 引擎 venv），Rust 版无此概念 → 整块省略，仅保留状态提示行；
//! - whisper 引擎 M5.1 已放开：档位下拉 = 原版 _WHISPER_SIZES 五档 + turbo（D-15）；
//!   vad_whisper.py 的本地 GGML 枚举/下载对话框不做——`whisper_model_size` 直接
//!   接受本地路径（settings 手填，`resolve_whisper_model` 文件直通），枚举入口另卡；
//! - vad_tab.py 的下载源折叠组（hub/镜像/代理）本批不做（下载按钮读 settings 默认值）；
//! - D-14：funasr-mlt-nano-2512 无上游 ONNX 转换 → 模型下拉灰显；
//! - "性能基准"按钮在页头（mod.rs），点击仅记日志（窗口随 M4.4 接入）。

use super::{group_card, hint_line, mark_settings_dirty, send_switch_engine, Palette};
use crate::state::{AppState, DeviceCache, DownloadErrKind, DownloadUiState};
use egui::{RichText, Ui};
use lt_proto::{AudioDeviceChoice, MicDeviceChoice};

// ── 引擎 / 模型 / 语言的纯逻辑表（单测覆盖） ──

/// 引擎表（r8 双引擎 + WP-B qwen3；显示名读 i18n `engine_display_*` 键，缺键回退注册表
/// 兜底名——原版 GUI_ENGINE_ORDER × ENGINE_REGISTRY 的 Rust 子集 + 阶段二新增）
pub const ENGINES: [(&str, &str, &str); 3] = [
    // (engine id, i18n 键, 注册表兜底显示名)
    ("funasr", "engine_display_funasr", "FunASR (SenseVoice)"),
    (
        "whisper",
        "engine_display_whisper",
        "Whisper (faster-whisper)",
    ),
    // WP-B：尾部追加，既有两项索引不动
    ("qwen3", "engine_display_qwen3", "Qwen3-ASR"),
];

/// 引擎显示名（原版 t("engine_display_" + id) 语义；t() 缺键回退键名 → 用兜底名）
pub fn engine_display(id: &str) -> String {
    let Some((_, key, fallback)) = ENGINES.iter().find(|(e, _, _)| *e == id) else {
        return id.to_string();
    };
    let text = lt_i18n::t(key);
    if text == *key {
        (*fallback).to_string()
    } else {
        text
    }
}

/// 引擎 id → 下拉索引（未知/legacy 值回退 funasr；settings.sanitize 已裁剪，双保险）
pub fn engine_index_for(engine: &str) -> usize {
    ENGINES
        .iter()
        .position(|(e, _, _)| *e == engine)
        .unwrap_or(0)
}

/// FunASR 模型下拉项（原版 funasr_model_options()：(键, 显示名)；enabled 为 Rust 版
/// 附加语义——mlt 无上游 ONNX（D-14）灰显，nano 标注实验性）
pub struct FunasrModelItem {
    pub key: &'static str,
    pub display: String,
    pub enabled: bool,
}

/// 模型表（顺序 = lt_proto::FUNASR_MODELS；显示名取 lt_models 注册表 display）
pub fn funasr_model_items() -> Vec<FunasrModelItem> {
    let exp = lt_i18n::t("model_experimental");
    vec![
        FunasrModelItem {
            key: "sensevoice-small",
            display: lt_models::registry::funasr_entry("sensevoice-small")
                .map(|e| e.display.to_string())
                .unwrap_or_else(|| "SenseVoice Small".into()),
            enabled: true,
        },
        FunasrModelItem {
            key: "funasr-nano-2512",
            display: lt_models::registry::funasr_entry("funasr-nano-2512")
                .map(|e| format!("{}{exp}", e.display))
                .unwrap_or_else(|| format!("funasr-nano-2512{exp}")),
            enabled: true,
        },
        // D-14：上游未发布 ONNX 转换，运行时回退 sensevoice-small → 灰显不可选
        FunasrModelItem {
            key: "funasr-mlt-nano-2512",
            display: "Fun-ASR-MLT-Nano".into(),
            enabled: false,
        },
    ]
}

/// settings.funasr_model → 下拉索引（非法值回退 sensevoice-small，对齐 sanitize）
pub fn funasr_index_for(model: &str) -> usize {
    funasr_model_items()
        .iter()
        .position(|m| m.key == model)
        .unwrap_or(0)
}

/// Whisper 档位下拉项（原版 _populate_whisper_models 的 builtin 部分：
/// _WHISPER_SIZES 五档 + turbo（D-15）；显示名取注册表 display）
pub struct WhisperTierItem {
    pub key: &'static str,
    pub display: String,
    /// 纯 CPU 实时性弱档（large-v3）→ 下拉下方 hint 行提示
    pub slow: bool,
}

/// whisper 档位表（顺序 = 注册表 WHISPER_ENTRIES，本地路径不进下拉）
pub fn whisper_tier_items() -> Vec<WhisperTierItem> {
    lt_models::registry::WHISPER_ENTRIES
        .iter()
        .map(|e| WhisperTierItem {
            key: e.key,
            display: e.display.to_string(),
            slow: e.key == "large-v3",
        })
        .collect()
}

/// settings.whisper_model_size → builtin 档下拉索引；本地 GGML 路径/未知值 → None
/// （下拉无选中态，当前值展示走 [`whisper_tier_display_for`]）
pub fn whisper_tier_index_for(size: &str) -> Option<usize> {
    lt_models::registry::WHISPER_ENTRIES
        .iter()
        .position(|e| e.key == size)
}

/// 档位当前值显示：builtin → 注册表 display；本地路径 → "本地: 文件名"
/// （原版 whisper_local_prefix 语义；非路径垃圾值原样兜底）
pub fn whisper_tier_display_for(size: &str) -> String {
    if let Some(i) = whisper_tier_index_for(size) {
        return lt_models::registry::WHISPER_ENTRIES[i].display.to_string();
    }
    let name = std::path::Path::new(size)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.is_empty() {
        return size.to_string();
    }
    format!("{}: {name}", lt_i18n::t("whisper_local_prefix"))
}

/// 识别语言下拉项（原版 `f"{code} - {label}"`；auto 显示名走 t("asr_lang_auto")）
pub fn asr_lang_items() -> Vec<(String, String)> {
    lt_i18n::LANGUAGES
        .iter()
        .map(|(code, native)| {
            let label = native
                .map(|n| n.to_string())
                .unwrap_or_else(|| lt_i18n::t("asr_lang_auto"));
            (code.to_string(), format!("{code} - {label}"))
        })
        .collect()
}

/// settings.asr_language → 下拉索引（未知码回退 auto=0）
pub fn asr_lang_index_for(code: &str) -> usize {
    asr_lang_items()
        .iter()
        .position(|(c, _)| c == code)
        .unwrap_or(0)
}

// ── 设备字符串语义（None/__disabled__/__default__/Named ↔ 下拉索引）──

/// settings.audio_device → 扬声器下拉索引。
/// 约定：0=禁用(仅麦克风) 1=系统默认 2+=具名设备。
/// 已存设备名不在当前枚举中（拔出）→ 回退系统默认（原版停在禁用位会静音音频，
/// 已知偏差：取更安全的回退目标）。
pub fn audio_index_for(setting: Option<&str>, outputs: &[String]) -> usize {
    match setting {
        Some("__disabled__") => 0,
        None => 1,
        Some(name) => outputs.iter().position(|d| d == name).map_or(1, |i| i + 2),
    }
}

/// 扬声器下拉索引 → settings.audio_device（原版 collect 的 audio 分支）
pub fn audio_setting_for(index: usize, outputs: &[String]) -> Option<String> {
    match index {
        0 => Some("__disabled__".into()),
        1 => None,
        i => outputs.get(i - 2).cloned(),
    }
}

/// settings.mic_device → 麦克风下拉索引。约定：0=禁用 1=系统默认 2+=具名设备。
/// 注意 settings 语义：None=禁用（与 audio 相反）；"__default__"/legacy"default"=默认。
pub fn mic_index_for(setting: Option<&str>, inputs: &[String]) -> usize {
    match setting {
        None => 0,
        Some("__default__") | Some("default") => 1,
        Some(name) => inputs.iter().position(|d| d == name).map_or(1, |i| i + 2),
    }
}

/// 麦克风下拉索引 → settings.mic_device（原版 collect 的 mic 分支）
pub fn mic_setting_for(index: usize, inputs: &[String]) -> Option<String> {
    match index {
        0 => None,
        1 => Some("__default__".into()),
        i => inputs
            .get(i - 2)
            .cloned()
            .or_else(|| Some("__default__".into())),
    }
}

/// D-29：麦克风输入启用态 = settings.mic_device 有值（UI「启用麦克风输入」勾选框状态）
pub fn mic_enabled_for(setting: &Option<String>) -> bool {
    setting.is_some()
}

// ── 模型缓存探测（原版 _update_whisper_size_label / is_asr_cached 语义）──

/// 缓存状态（Cached/Missing 携带注册表体积估计，用于"已缓存 ✓ 大小"展示）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    /// 已缓存（携带估计字节）
    Cached(u64),
    /// 未缓存（携带估计字节）
    Missing(u64),
    /// 无注册表条目（mlt，D-14）
    Unavailable,
}

/// 当前引擎模型的缓存判定（lt_models::cache 双 hub 或语义）
pub fn model_cache_status(
    models_dir: &std::path::Path,
    engine: &str,
    funasr_model: &str,
    whisper_size: &str,
) -> CacheStatus {
    match engine {
        "funasr" => match lt_models::registry::funasr_entry(funasr_model) {
            Some(entry) => {
                if lt_models::cache::is_funasr_cached(models_dir, &entry) {
                    CacheStatus::Cached(entry.estimated_bytes)
                } else {
                    CacheStatus::Missing(entry.estimated_bytes)
                }
            }
            None => CacheStatus::Unavailable,
        },
        "whisper" => match lt_models::registry::whisper_entry_for(whisper_size) {
            Some(entry) => {
                if lt_models::cache::is_whisper_cached(models_dir, whisper_size) {
                    CacheStatus::Cached(entry.estimated_bytes)
                } else {
                    CacheStatus::Missing(entry.estimated_bytes)
                }
            }
            // 本地 GGML 路径等非 builtin 值：文件存在即缓存（原版同语义）
            None => {
                if std::path::Path::new(whisper_size).is_file() {
                    CacheStatus::Cached(0)
                } else {
                    CacheStatus::Missing(0)
                }
            }
        },
        // WP-B：qwen3 复用 manifest 探测（单一模型，B-α 无模型键）
        "qwen3" => {
            let entry = lt_models::registry::qwen3_entry();
            if lt_models::cache::is_funasr_cached(models_dir, &entry) {
                CacheStatus::Cached(entry.estimated_bytes)
            } else {
                CacheStatus::Missing(entry.estimated_bytes)
            }
        }
        _ => CacheStatus::Unavailable,
    }
}

/// 字节量人性化（对齐原版 format_size / lt_app::backend 同款阈值）
pub fn format_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} B")
    } else if size_bytes < 1024u64.pow(2) {
        format!("{:.1} KB", size_bytes as f64 / 1024.0)
    } else if size_bytes < 1024u64.pow(3) {
        format!("{:.1} MB", size_bytes as f64 / 1024u64.pow(2) as f64)
    } else {
        format!("{:.2} GB", size_bytes as f64 / 1024u64.pow(3) as f64)
    }
}

// ── UI ──

/// 识别页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    // N3/N4：识别页偏离默认值提示 + 恢复本页（ui_lang 为语言偏好，不提示不恢复）
    let diffs = crate::panel_diff::diff_paths(&state.settings);
    let page_diffs: Vec<&str> = diffs
        .iter()
        .filter(|p| {
            VAD_PAGE_PATHS
                .iter()
                .any(|pre| p.as_str() == *pre || p.as_str().starts_with(pre))
        })
        .map(|p: &String| p.as_str())
        .collect();
    if !page_diffs.is_empty() {
        super::reset_toolbar(ui, pal, page_diffs.len(), &page_diffs.join("、"), |_| {
            restore_vad_page(state);
        });
    }

    let engine = state.settings.asr_engine.clone();
    let is_funasr = engine == "funasr";

    // 设备枚举缓存：首次进入识别页时构建（UI 线程临时 WasapiBackend，COM 自 init）
    if state.panel.devices.is_none() {
        state.panel.devices = Some(enumerate_devices());
    }

    // ── ASR 引擎（原版 asr_group）──
    group_card(ui, pal, &lt_i18n::t("group_asr_engine"), |ui| {
        // 引擎下拉：funasr/whisper 双引擎可选（M5.1 放开灰显；whisper worker 已实装）
        let eng_idx = engine_index_for(&engine);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_engine"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_engine")
                .selected_text(engine_display(&engine))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, (id, _, _)) in ENGINES.iter().enumerate() {
                        let selected = i == eng_idx;
                        let label = engine_display(id);
                        if ui.selectable_label(selected, label).clicked() && !selected {
                            state.settings.asr_engine = (*id).to_string();
                            send_switch_engine(state);
                        }
                    }
                });
        });
        // 状态提示行（原版 _engine_status_label；安装/变体按钮为 Python 打包专属，省略）
        let (status_text, status_color) = engine_status(state, &engine, pal);
        ui.add_space(2.0);
        ui.label(RichText::new(status_text).size(11.0).color(status_color));

        ui.add_space(4.0);
        // 识别语言（原版 _asr_lang：asr_language_changed → Cmd::SetAsrLanguage）
        let langs = asr_lang_items();
        let lang_idx = asr_lang_index_for(&state.settings.asr_language);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} ", lt_i18n::t("label_language_hint"))).color(pal.text),
            );
            egui::ComboBox::from_id_salt("panel_asr_lang")
                .selected_text(langs[lang_idx].1.clone())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, (code, label)) in langs.iter().enumerate() {
                        if ui.selectable_label(lang_idx == i, label.clone()).clicked()
                            && lang_idx != i
                        {
                            // 即时命令：shell 写 settings.asr_language + 持久化
                            state.send_cmd(lt_proto::Cmd::SetAsrLanguage(code.clone()));
                        }
                    }
                });
        });
        // WP-B：qwen3 无语言参数（纯 auto-LID）——下拉保留（设置对其他引擎仍有效），
        // 但明示对当前引擎无效
        if engine == "qwen3" {
            hint_line(ui, pal, &lt_i18n::t("qwen3_lang_auto_hint"));
        }

        // FunASR 模型（仅 funasr 引擎显示；原版 _on_engine_changed_whisper_vis）
        if is_funasr {
            let items = funasr_model_items();
            let m_idx = funasr_index_for(&state.settings.funasr_model);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} ", lt_i18n::t("label_funasr_model"))).color(pal.text),
                );
                egui::ComboBox::from_id_salt("panel_funasr_model")
                    .selected_text(items[m_idx].display.clone())
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (i, item) in items.iter().enumerate() {
                            if !item.enabled {
                                // D-14：mlt 灰显（无上游 ONNX）
                                ui.add_enabled(
                                    false,
                                    egui::Button::selectable(false, item.display.clone()),
                                );
                            } else if ui
                                .selectable_label(m_idx == i, item.display.clone())
                                .clicked()
                                && m_idx != i
                            {
                                state.settings.funasr_model = item.key.to_string();
                                send_switch_engine(state);
                            }
                        }
                    });
            });
            if m_idx == 2 {
                hint_line(ui, pal, &lt_i18n::t("model_mlt_disabled_hint"));
            }
        }

        // Whisper 档位（原版 _whisper_size_combo；仅 whisper 引擎显示）。
        // 变更语义对齐原版 _on_whisper_size_changed：新档已缓存 → 即时切引擎；
        // 未缓存 → 仅落盘（缓存组下载完成后经 DownloadSucceeded 重发切换）
        if engine == "whisper" {
            let tiers = whisper_tier_items();
            let sel_idx = whisper_tier_index_for(&state.settings.whisper_model_size);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{} ", lt_i18n::t("label_whisper_model")))
                        .color(pal.text),
                );
                egui::ComboBox::from_id_salt("panel_whisper_model")
                    .selected_text(whisper_tier_display_for(&state.settings.whisper_model_size))
                    .width(240.0)
                    .show_ui(ui, |ui| {
                        for (i, item) in tiers.iter().enumerate() {
                            if ui
                                .selectable_label(sel_idx == Some(i), item.display.clone())
                                .clicked()
                                && sel_idx != Some(i)
                            {
                                state.settings.whisper_model_size = item.key.to_string();
                                mark_settings_dirty(state);
                                let cached = lt_models::paths::models_dir(
                                    state.settings.models_dir.as_deref(),
                                )
                                .map(|d| lt_models::cache::is_whisper_cached(&d, item.key))
                                .unwrap_or(false);
                                if cached {
                                    send_switch_engine(state);
                                }
                            }
                        }
                    });
            });
            if tiers
                .iter()
                .any(|t| t.slow && t.key == state.settings.whisper_model_size)
            {
                hint_line(ui, pal, &lt_i18n::t("whisper_slow_hint"));
            }
        }

        // ── Padding（原版 QDoubleSpinBox 0-5/0.1 步进/后缀 s/0=关闭；可见性矩阵
        //     对齐原版 _on_engine_changed_whisper_vis：SenseVoice padding 仅 funasr、
        //     whisper padding 仅 whisper；变更经 Cmd::SetPadding 挂起热应用 +
        //     300ms 防抖 ApplySettings 落盘）──
        ui.add_space(4.0);
        // nano 无 padding 语义（原版 funasr_supports_padding: nano=false，manager 亦
        // 跳过下发）→ 滑杆仅 sensevoice 有意义，选中 nano 时隐藏（显示而无效果属误导）
        if is_funasr && state.settings.funasr_model != "funasr-nano-2512" {
            let mut sv = state.settings.sensevoice_pad_seconds;
            if drag_secs(
                ui,
                "panel_pad_sensevoice",
                &lt_i18n::t("label_sensevoice_padding"),
                &mut sv,
                0.0..=5.0,
                true,
            ) {
                state.settings.sensevoice_pad_seconds = sv;
                state.send_cmd(lt_proto::Cmd::SetPadding {
                    engine: "funasr".into(),
                    secs: sv,
                });
                mark_settings_dirty(state);
            }
            hint_line(ui, pal, &lt_i18n::t("sensevoice_padding_tooltip"));
        }
        if engine == "whisper" {
            let mut wv = state.settings.whisper_pad_seconds;
            if drag_secs(
                ui,
                "panel_pad_whisper",
                &lt_i18n::t("label_whisper_padding"),
                &mut wv,
                0.0..=5.0,
                true,
            ) {
                state.settings.whisper_pad_seconds = wv;
                state.send_cmd(lt_proto::Cmd::SetPadding {
                    engine: "whisper".into(),
                    secs: wv,
                });
                mark_settings_dirty(state);
            }
            hint_line(ui, pal, &lt_i18n::t("whisper_padding_tooltip"));
        }

        // ── 下载源（原版 _hub_combo：ms/hf 二选一，_auto_save 落盘）──
        ui.add_space(4.0);
        let hubs = [lt_i18n::t("hub_modelscope"), lt_i18n::t("hub_huggingface")];
        let hub_idx = if state.settings.hub == "hf" { 1 } else { 0 };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_hub"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_hub")
                .selected_text(hubs[hub_idx].clone())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, label) in hubs.iter().enumerate() {
                        if ui.selectable_label(hub_idx == i, label.clone()).clicked()
                            && hub_idx != i
                        {
                            state.settings.hub = if i == 1 { "hf".into() } else { "ms".into() };
                            mark_settings_dirty(state);
                        }
                    }
                });
        });
        // D-24/AH-6：hub=ms 但所选模型无真实 MS 源 → 诚实提示自动经 HF 镜像下载。
        // 判定从注册表派生（entry.ms.is_none()），不再手写引擎/模型键清单
        // （H10：硬编码清单漏过 whisper 档）；本地 GGML 路径自然不提示
        if state.settings.hub == "ms" {
            let hf_only = match state.settings.asr_engine.as_str() {
                "funasr" => lt_models::registry::funasr_entry(&state.settings.funasr_model)
                    .is_some_and(|e| e.ms.is_none()),
                "whisper" => {
                    lt_models::registry::whisper_entry_for(&state.settings.whisper_model_size)
                        .is_some_and(|e| e.ms.is_none())
                }
                "qwen3" => lt_models::registry::qwen3_entry().ms.is_none(),
                _ => false,
            };
            if hf_only {
                hint_line(ui, pal, &lt_i18n::t("model_hf_only_hint"));
            }
        }

        // ── 界面语言（原版 _ui_lang_combo：["English","中文"]，index 0=en 1=zh；
        //     写 settings.ui_lang 防抖落盘，重启生效）──
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_ui_lang"))).color(pal.text));
            let zh = lt_i18n::t("lang_zh");
            let langs = ["English", zh.as_str()]; // 与原版 addItem(["English","中文"]) 顺序一致
            let lang_idx = if state.settings.ui_lang == "zh" { 1 } else { 0 };
            egui::ComboBox::from_id_salt("panel_ui_lang_vad")
                .selected_text(langs[lang_idx].to_string())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (i, label) in langs.iter().enumerate() {
                        if ui
                            .selectable_label(lang_idx == i, (*label).to_string())
                            .clicked()
                            && lang_idx != i
                        {
                            state.settings.ui_lang = if i == 1 { "zh".into() } else { "en".into() };
                            mark_settings_dirty(state);
                        }
                    }
                });
        });
        hint_line(ui, pal, &lt_i18n::t("ui_lang_restart_hint"));
    });

    // ── 设备组（原版 asr_group 的 audio/mic 行 + 刷新按钮；Rust 版独立成卡）──
    group_card(ui, pal, &lt_i18n::t("group_devices"), |ui| {
        let devices = state.panel.devices.clone().unwrap_or_default();
        let outputs = devices.outputs.clone();
        let inputs = devices.inputs.clone();

        // 扬声器（loopback）：禁用/系统默认/具名（原版 index 0/1/2+ 布局）
        let a_idx = audio_index_for(state.settings.audio_device.as_deref(), &outputs);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_audio"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_audio_device")
                .selected_text(device_label(a_idx, &outputs, lt_i18n::t("audio_disabled")))
                .width(300.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(a_idx == 0, lt_i18n::t("audio_disabled"))
                        .clicked()
                        && a_idx != 0
                    {
                        apply_audio_setting(state, audio_setting_for(0, &outputs));
                    }
                    if ui
                        .selectable_label(a_idx == 1, lt_i18n::t("system_default"))
                        .clicked()
                        && a_idx != 1
                    {
                        apply_audio_setting(state, audio_setting_for(1, &outputs));
                    }
                    for (i, name) in outputs.iter().enumerate() {
                        let idx = i + 2;
                        if ui.selectable_label(a_idx == idx, name.clone()).clicked() && a_idx != idx
                        {
                            apply_audio_setting(state, Some(name.clone()));
                        }
                    }
                });
            if ui
                .add(
                    egui::Button::new(RichText::new(lt_i18n::t("btn_refresh")).size(12.0))
                        .corner_radius(6.0),
                )
                .clicked()
            {
                state.panel.devices = Some(enumerate_devices());
            }
        });

        // 麦克风（D-29）：显式「启用麦克风输入」勾选框 + 设备下拉（未勾选置灰）。
        // 原版为"禁用/系统默认/具名"三值下拉、禁用藏在下拉第一项 → 用户感知不到开关；
        // 勾选=true 写 __default__/具名，取消=false 写 None（后端立即停采）。
        let m_enabled = mic_enabled_for(&state.settings.mic_device);
        // 下拉从"系统默认"起（1）；未勾选时同样显示"系统默认"预览（置灰不可交互）
        let m_idx = mic_index_for(state.settings.mic_device.as_deref(), &inputs).max(1);
        ui.horizontal(|ui| {
            let mut enabled = m_enabled;
            if ui
                .checkbox(&mut enabled, lt_i18n::t("mic_enable"))
                .changed()
            {
                apply_mic_setting(
                    state,
                    if enabled {
                        Some("__default__".into())
                    } else {
                        None
                    },
                );
            }
            let combo = egui::ComboBox::from_id_salt("panel_mic_device")
                .selected_text(device_label(m_idx, &inputs, lt_i18n::t("mic_disabled")))
                .width(300.0);
            ui.add_enabled_ui(m_enabled, |ui| {
                combo.show_ui(ui, |ui| {
                    if ui
                        .selectable_label(m_idx == 1, lt_i18n::t("system_default"))
                        .clicked()
                        && m_idx != 1
                    {
                        apply_mic_setting(state, mic_setting_for(1, &inputs));
                    }
                    for (i, name) in inputs.iter().enumerate() {
                        let idx = i + 2;
                        if ui.selectable_label(m_idx == idx, name.clone()).clicked() && m_idx != idx
                        {
                            apply_mic_setting(state, Some(name.clone()));
                        }
                    }
                });
            });
        });
        if m_enabled {
            hint_line(ui, pal, &lt_i18n::t("mic_enable_hint"));
        }
        if let Some(def) = &devices.default_output {
            hint_line(ui, pal, &format!("{}: {def}", lt_i18n::t("system_default")));
        }
    });

    // ── VAD 模式（原版 mode_group）──
    group_card(ui, pal, &lt_i18n::t("group_vad_mode"), |ui| {
        let modes = ["silero", "energy", "disabled"];
        let labels = [
            lt_i18n::t("vad_silero"),
            lt_i18n::t("vad_energy"),
            lt_i18n::t("vad_disabled"),
        ];
        let mode_idx = modes
            .iter()
            .position(|m| *m == state.settings.vad_mode)
            .unwrap_or(0);
        let mut next = mode_idx;
        egui::ComboBox::from_id_salt("panel_vad_mode")
            .selected_text(labels[mode_idx].clone())
            .width(240.0)
            .show_ui(ui, |ui| {
                for (i, label) in labels.iter().enumerate() {
                    if ui.selectable_label(mode_idx == i, label.clone()).clicked() && mode_idx != i
                    {
                        next = i;
                    }
                }
            });
        if next != mode_idx {
            state.settings.vad_mode = modes[next].to_string();
            mark_settings_dirty(state);
        }
    });

    // Silero 阈值：0-1 滑条（原版 0-100 滑条 + % 标签；此处 step 0.05，
    // 拖动实时改 draft，300ms 无变更后落盘 = 原版松手 auto_save）
    group_card(ui, pal, &lt_i18n::t("group_silero_threshold"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_threshold"))).color(pal.text));
            let mut v = state.settings.vad_threshold.clamp(0.0, 1.0);
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.0..=1.0)
                    .step_by(0.05)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            );
            if resp.changed() {
                state.settings.vad_threshold = v;
                mark_settings_dirty(state);
            }
        });
    });

    // 能量阈值（原版 1-100‰ 滑条）
    group_card(ui, pal, &lt_i18n::t("group_energy_threshold"), |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_threshold"))).color(pal.text));
            let mut v = state.settings.energy_threshold.clamp(0.001, 0.1);
            let resp = ui.add(
                egui::Slider::new(&mut v, 0.001..=0.1)
                    .step_by(0.001)
                    .custom_formatter(|v, _| format!("{:.0}\u{2030}", v * 1000.0)),
            );
            if resp.changed() {
                state.settings.energy_threshold = v;
                mark_settings_dirty(state);
            }
        });
    });

    // ── 时间参数（原版 timing_group：最短/最长语句、静音模式+时长、增量识别）──
    group_card(ui, pal, &lt_i18n::t("group_timing"), |ui| {
        let mut min_v = state.settings.min_speech_duration;
        if drag_secs(
            ui,
            "panel_min_speech",
            &lt_i18n::t("label_min_speech"),
            &mut min_v,
            0.1..=5.0,
            true,
        ) {
            state.settings.min_speech_duration = min_v;
            mark_settings_dirty(state);
        }
        let mut max_v = state.settings.max_speech_duration;
        if drag_secs(
            ui,
            "panel_max_speech",
            &lt_i18n::t("label_max_speech"),
            &mut max_v,
            2.0..=30.0,
            true,
        ) {
            state.settings.max_speech_duration = max_v;
            mark_settings_dirty(state);
        }
        // 静音模式：auto=时长控件禁用 + 原版 tooltip 解释（_on_silence_mode_changed）
        let modes = ["auto", "fixed"];
        let mode_labels = [lt_i18n::t("silence_auto"), lt_i18n::t("silence_fixed")];
        let sm_idx = modes
            .iter()
            .position(|m| *m == state.settings.silence_mode)
            .unwrap_or(0);
        let mut sm_next = sm_idx;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} ", lt_i18n::t("label_silence"))).color(pal.text));
            egui::ComboBox::from_id_salt("panel_silence_mode")
                .selected_text(mode_labels[sm_idx].clone())
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (i, label) in mode_labels.iter().enumerate() {
                        if ui.selectable_label(sm_idx == i, label.clone()).clicked() && sm_idx != i
                        {
                            sm_next = i;
                        }
                    }
                });
        });
        if sm_next != sm_idx {
            state.settings.silence_mode = modes[sm_next].to_string();
            mark_settings_dirty(state);
        }
        let fixed = state.settings.silence_mode == "fixed";
        let mut sil_v = state.settings.silence_duration;
        let sil_changed = drag_secs(
            ui,
            "panel_silence_dur",
            &lt_i18n::t("label_silence_dur"),
            &mut sil_v,
            0.1..=3.0,
            fixed, // auto 模式禁用（原版 setEnabled(saved_smode != "auto")）
        );
        if !fixed {
            hint_line(ui, pal, &lt_i18n::t("silence_dur_disabled_tooltip"));
        }
        if sil_changed {
            state.settings.silence_duration = sil_v;
            mark_settings_dirty(state);
        }

        // 增量识别 + 间隔（原版 _incremental_asr_cb / _interim_interval_spin：
        // 变更即时发 Cmd::IncrementalAsr 热应用 + 300ms 防抖 ApplySettings 落盘）
        let mut inc = state.settings.incremental_asr;
        if ui
            .add(egui::Checkbox::new(
                &mut inc,
                RichText::new(lt_i18n::t("label_incremental_asr")).color(pal.text),
            ))
            .changed()
        {
            state.settings.incremental_asr = inc;
            state.send_cmd(lt_proto::Cmd::IncrementalAsr {
                enabled: inc,
                interval: state.settings.interim_interval,
            });
            mark_settings_dirty(state);
        }
        let mut itv = state.settings.interim_interval;
        let itv_changed = drag_secs(
            ui,
            "panel_interim",
            &lt_i18n::t("label_interim_interval"),
            &mut itv,
            1.0..=10.0,
            inc, // 未勾选时禁用（原版 setEnabled(incremental_asr)）
        );
        if !inc {
            hint_line(ui, pal, &lt_i18n::t("interim_interval_disabled_tooltip"));
        }
        if itv_changed {
            state.settings.interim_interval = itv;
            state.send_cmd(lt_proto::Cmd::IncrementalAsr {
                enabled: inc,
                interval: itv,
            });
            mark_settings_dirty(state);
        }
    });

    // ── 模型缓存（原版 _update_whisper_size_label + 下载按钮的泛化：
    //     显示当前引擎模型缓存状态；未缓存 → 下载按钮走现有 StartDownload 管线。
    //     Rust 版四态：Idle(Missing/Cached) / Downloading(进度+日志) / Failed(分类+重试)）──
    group_card(ui, pal, &lt_i18n::t("group_model_cache"), |ui| {
        let models_dir = lt_models::paths::models_dir(state.settings.models_dir.as_deref())
            .unwrap_or_else(|_| std::env::temp_dir());
        // DL-6/F12：探测结果缓存 2s（探测键变化即失效），下载事件在 app.rs 侧
        // 失效——识别页每帧渲染不再递归扫盘
        let probe_key = format!(
            "{}|{}|{}",
            state.settings.asr_engine,
            state.settings.funasr_model,
            state.settings.whisper_model_size
        );
        let status = match &state.panel.cache_probe {
            Some((at, key, s)) if *key == probe_key && at.elapsed().as_millis() < 2_000 => *s,
            _ => {
                let s = model_cache_status(
                    &models_dir,
                    &state.settings.asr_engine,
                    &state.settings.funasr_model,
                    &state.settings.whisper_model_size,
                );
                state.panel.cache_probe = Some((std::time::Instant::now(), probe_key, s));
                s
            }
        };
        ui.horizontal(|ui| {
            let model_display = if state.settings.asr_engine == "funasr" {
                funasr_model_items()[funasr_index_for(&state.settings.funasr_model)]
                    .display
                    .clone()
            } else if state.settings.asr_engine == "qwen3" {
                // WP-B：qwen3 显示名取注册表（缓存卡片/下载标题行）
                lt_models::registry::qwen3_entry().display.to_string()
            } else {
                whisper_tier_display_for(&state.settings.whisper_model_size)
            };
            ui.label(RichText::new(model_display).color(pal.text));
            match &state.download {
                // ── 下载进行中：按当前文件显示「文件名（k/n）」+ 精确字节进度
                //    （DL-3：机器段驱动；total 未知 → 明确文案的日志模式）──
                DownloadUiState::Downloading {
                    file,
                    k,
                    n,
                    done_bytes,
                    total_bytes,
                    log,
                } => {
                    let known = *total_bytes > 0;
                    let head = if known {
                        let pct = (*done_bytes as f64 / *total_bytes as f64 * 100.0) as u32;
                        format!("{} {file}（{k}/{n}）{pct}%", lt_i18n::t("downloading"))
                    } else {
                        format!("{} {file}（{k}/{n}）", lt_i18n::t("downloading"))
                    };
                    ui.label(RichText::new(head).color(pal.accent));
                    ui.horizontal(|ui| {
                        if known {
                            ui.add(
                                egui::ProgressBar::new(*done_bytes as f32 / *total_bytes as f32)
                                    .desired_width(160.0)
                                    .desired_height(10.0),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} / {}",
                                    format_size(*done_bytes),
                                    format_size(*total_bytes)
                                ))
                                .size(11.0)
                                .color(pal.weak),
                            );
                        } else {
                            ui.label(
                                RichText::new(lt_i18n::t("download_size_unknown"))
                                    .size(11.0)
                                    .color(pal.weak),
                            );
                        }
                    });
                    if log.is_empty() {
                        ui.label(
                            RichText::new(lt_i18n::t("download_starting"))
                                .size(11.0)
                                .color(pal.weak),
                        );
                    } else {
                        let last = log.last().cloned().unwrap_or_default();
                        ui.label(
                            RichText::new(format!("\u{2026} {last}"))
                                .size(11.0)
                                .color(pal.weak),
                        );
                    }
                    // DL-4/D-23：取消 → backend 置会话令牌，Downloader 在检查点停止
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(lt_i18n::t("cancel_download")).size(12.0),
                            )
                            .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        state.send_cmd(lt_proto::Cmd::CancelDownload);
                    }
                }
                // ── 下载已取消（DL-4/D-23）：保留日志 + 继续下载（断点续传）──
                DownloadUiState::Cancelled { log } => {
                    ui.label(
                        RichText::new(lt_i18n::t("download_cancelled_title"))
                            .color(pal.warn)
                            .size(11.5),
                    );
                    ui.label(
                        RichText::new(lt_i18n::t("download_cancelled_hint"))
                            .size(11.0)
                            .color(pal.weak),
                    );
                    let _ = log;
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(lt_i18n::t("resume_download")).size(12.0),
                            )
                            .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        start_download(state);
                    }
                }
                // ── 下载失败：分类建议 + 原始错误 + 重试（P2-6）──
                DownloadUiState::Failed { kind, detail, log } => {
                    ui.label(
                        RichText::new(lt_i18n::t("download_failed_title"))
                            .color(pal.err)
                            .size(11.5),
                    );
                    ui.label(
                        RichText::new(download_err_advice(*kind))
                            .size(11.0)
                            .color(pal.warn),
                    );
                    ui.label(
                        RichText::new(format!("\u{2026} {detail}"))
                            .size(10.5)
                            .color(pal.weak),
                    );
                    let _ = log;
                    if ui
                        .add(
                            egui::Button::new(RichText::new(lt_i18n::t("retry")).size(12.0))
                                .corner_radius(6.0),
                        )
                        .clicked()
                    {
                        start_download(state);
                    }
                }
                // ── 空闲态：磁盘探测三态（原状 + 未缓存下载按钮）──
                DownloadUiState::Idle => match status {
                    CacheStatus::Cached(bytes) => {
                        ui.label(
                            RichText::new(format!(
                                "\u{2713} {} {}",
                                lt_i18n::t("whisper_already_cached"),
                                format_size(bytes)
                            ))
                            .color(pal.ok),
                        );
                    }
                    CacheStatus::Missing(bytes) => {
                        ui.label(
                            RichText::new(format!(
                                "{} \u{2248}{}",
                                lt_i18n::t("model_not_cached"),
                                format_size(bytes)
                            ))
                            .color(pal.warn),
                        );
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(lt_i18n::t("btn_download_whisper")).size(12.0),
                                )
                                .corner_radius(6.0),
                            )
                            .clicked()
                        {
                            start_download(state);
                        }
                    }
                    CacheStatus::Unavailable => {
                        hint_line(ui, pal, &lt_i18n::t("model_mlt_disabled_hint"));
                    }
                },
            }
        });
    });

    ui.add_space(8.0);
}

// ── 下载状态机辅助 ──

/// 识别页字段路径前缀（N3/N4 页级 diff 判定；ui_lang/target_language/models_dir
/// 等无页内控件命名空间，不纳入）
const VAD_PAGE_PATHS: [&str; 18] = [
    "vad_mode",
    "vad_threshold",
    "energy_threshold",
    "min_speech_duration",
    "max_speech_duration",
    "silence_mode",
    "silence_duration",
    "incremental_asr",
    "interim_interval",
    "asr_engine",
    "funasr_model",
    "whisper_model_size",
    "sensevoice_pad_seconds",
    "whisper_pad_seconds",
    "hub",
    "audio_device",
    "mic_device",
    "asr_language",
];

/// 识别页恢复默认：写回字段 + 逐条即时命令（ApplySettings 无法热应用
/// 引擎/设备/语言/padding/增量，必须重发——N3 生效管道）
fn restore_vad_page(state: &mut AppState) {
    let def = lt_proto::Settings::default();
    let s = &mut state.settings;
    s.vad_mode = def.vad_mode.clone();
    s.vad_threshold = def.vad_threshold;
    s.energy_threshold = def.energy_threshold;
    s.min_speech_duration = def.min_speech_duration;
    s.max_speech_duration = def.max_speech_duration;
    s.silence_mode = def.silence_mode.clone();
    s.silence_duration = def.silence_duration;
    s.incremental_asr = def.incremental_asr;
    s.interim_interval = def.interim_interval;
    s.asr_engine = def.asr_engine.clone();
    s.funasr_model = def.funasr_model.clone();
    s.whisper_model_size = def.whisper_model_size.clone();
    s.sensevoice_pad_seconds = def.sensevoice_pad_seconds;
    s.whisper_pad_seconds = def.whisper_pad_seconds;
    s.hub = def.hub.clone();
    s.audio_device = def.audio_device.clone();
    s.mic_device = def.mic_device.clone();
    s.asr_language = def.asr_language.clone();
    // 逐条下发即时命令（引擎→语言→设备→padding→增量）
    super::send_switch_engine(state);
    state.send_cmd(lt_proto::Cmd::SetAsrLanguage(
        state.settings.asr_language.clone(),
    ));
    state.send_cmd(lt_proto::Cmd::SetAudioDevice(
        lt_proto::AudioDeviceChoice::SystemDefault,
    ));
    state.send_cmd(lt_proto::Cmd::SetMicDevice(lt_proto::MicDeviceChoice::Off));
    state.send_cmd(lt_proto::Cmd::SetPadding {
        engine: "funasr".into(),
        secs: state.settings.sensevoice_pad_seconds,
    });
    state.send_cmd(lt_proto::Cmd::SetPadding {
        engine: "whisper".into(),
        secs: state.settings.whisper_pad_seconds,
    });
    state.send_cmd(lt_proto::Cmd::IncrementalAsr {
        enabled: state.settings.incremental_asr,
        interval: state.settings.interim_interval,
    });
    mark_settings_dirty(state);
}

/// 发起下载：写入 Downloading 状态（进度事件随后填充）并发送命令。
/// 进行中忽略重复点击（P2-5 修复：不再触发并发下载线程）。
/// 失败/取消态的日志带入新下载（上下文延续）。
fn start_download(state: &mut AppState) {
    if state.download.downloading() {
        return;
    }
    let mut log = match &state.download {
        DownloadUiState::Failed { log, .. } | DownloadUiState::Cancelled { log } => log.clone(),
        _ => Vec::new(),
    };
    log.push(lt_i18n::t("download_starting").to_string());
    state.download = DownloadUiState::Downloading {
        file: String::new(),
        k: 0,
        n: 0,
        done_bytes: 0,
        total_bytes: 0,
        log,
    };
    state.send_cmd(lt_proto::Cmd::StartDownload {
        hub: state.settings.hub.clone(),
        proxy: state.settings.download_proxy.clone(),
    });
}

/// 失败分类 → 用户建议（i18n；未知类别给通用指引）
fn download_err_advice(kind: DownloadErrKind) -> String {
    let key = match kind {
        DownloadErrKind::Net => "download_err_net",
        DownloadErrKind::Http404 => "download_err_http404",
        DownloadErrKind::Http => "download_err_http",
        DownloadErrKind::Disk => "download_err_disk",
        DownloadErrKind::Length => "download_err_length",
        DownloadErrKind::Other => "download_err_other",
    };
    lt_i18n::t(key)
}

// ── 页内小组件 ──

/// "标签 + 秒数 DragValue"行（原版 QDoubleSpinBox：suffix " s"、step 0.1、两位小数）。
/// `enabled=false` 时控件灰显（原版 setEnabled）。返回是否变更（调用方写回
/// settings + mark_settings_dirty）。
fn drag_secs(
    ui: &mut Ui,
    id: &str,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    enabled: bool,
) -> bool {
    ui.push_id(id, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label} ")).color(ui.visuals().text_color()));
            ui.add_enabled(
                enabled,
                egui::DragValue::new(value)
                    .range(range)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(" s"),
            )
            .changed()
        })
        .inner
    })
    .inner
}

/// 设备下拉当前显示文本：0/1 = 禁用/系统默认文案，2+ = 具名（越界回退默认文案）
fn device_label(index: usize, devices: &[String], first_label: String) -> String {
    match index {
        0 => first_label,
        1 => lt_i18n::t("system_default"),
        i => devices
            .get(i - 2)
            .cloned()
            .unwrap_or_else(|| lt_i18n::t("system_default")),
    }
}

/// 扬声器选择 → 设置 + 即时命令（原版 collect 的 audio 分支 → settings_changed 应用）
fn apply_audio_setting(state: &mut AppState, setting: Option<String>) {
    state.settings.audio_device = setting.clone();
    state.send_cmd(lt_proto::Cmd::SetAudioDevice(AudioDeviceChoice::from(
        setting,
    )));
}

/// 麦克风选择 → 设置 + 即时命令
fn apply_mic_setting(state: &mut AppState, setting: Option<String>) {
    state.settings.mic_device = setting.clone();
    state.send_cmd(lt_proto::Cmd::SetMicDevice(MicDeviceChoice::from(setting)));
}

/// 引擎状态提示（原版 _refresh_engine_status 的 Rust 化简版）：
/// 按当前引擎模型缓存判定 → 已缓存 = 可用；未缓存 = needs-model（识别页缓存组
/// 可下载；mlt 等无条目模型同样落此文案，与既有 funasr 行为一致）
fn engine_status(state: &AppState, engine: &str, pal: &Palette) -> (String, egui::Color32) {
    let models_dir = lt_models::paths::models_dir(state.settings.models_dir.as_deref())
        .unwrap_or_else(|_| std::env::temp_dir());
    match model_cache_status(
        &models_dir,
        engine,
        &state.settings.funasr_model,
        &state.settings.whisper_model_size,
    ) {
        CacheStatus::Cached(_) => (lt_i18n::t("engine_status_available"), pal.ok),
        _ => (lt_i18n::t("engine_status_needs_model"), pal.warn),
    }
}

/// UI 线程临时枚举（WasapiBackend::new 仅作枚举面，不 start 线程；
/// 函数自带 COM init，见 lt-pipeline audio 模块注释）
fn enumerate_devices() -> DeviceCache {
    #[cfg(windows)]
    {
        use lt_pipeline::audio::AudioBackend as _;
        let be = lt_pipeline::audio::wasapi_win::WasapiBackend::new();
        let outputs = be.list_output_devices().unwrap_or_default();
        let inputs = be.list_input_devices().unwrap_or_default();
        let default_output = be.current_default_output().unwrap_or(None);
        DeviceCache {
            outputs,
            inputs,
            default_output,
        }
    }
    #[cfg(not(windows))]
    {
        DeviceCache::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DL-4/D-23：取消态 → start_download 恢复为 Downloading 且日志延续；
    /// Downloading 态重复点击被守卫忽略
    #[test]
    fn download_card_cancelled_then_restart_preserves_log() {
        let mut state = crate::state::AppState::new(lt_proto::Settings::default());
        // 取消态（带取消前日志）→ 继续下载
        state.download = DownloadUiState::Cancelled {
            log: vec!["[a/b] model.bin 1.0 KB / 2.0 KB".into()],
        };
        start_download(&mut state);
        match &state.download {
            DownloadUiState::Downloading { log, .. } => {
                assert_eq!(log.len(), 2, "取消前日志 + download_starting");
            }
            other => panic!("{other:?}"),
        }
        // Downloading 中重复触发被忽略（不再新建状态）
        let before = state.download.clone();
        start_download(&mut state);
        assert_eq!(state.download, before);
    }

    fn dev(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ── 引擎 / 模型 / 语言表 ──

    /// 引擎表：r8 双引擎 + WP-B qwen3、显示名可解析（whisper M5.1 起可选，无灰显特判）
    #[test]
    fn engine_table_and_display_fallback() {
        assert_eq!(ENGINES.map(|(id, _, _)| id), ["funasr", "whisper", "qwen3"]);
        assert_eq!(engine_index_for("funasr"), 0);
        assert_eq!(engine_index_for("whisper"), 1);
        assert_eq!(engine_index_for("qwen3"), 2);
        // legacy/未知值回退 funasr（settings.sanitize 之外的 UI 侧双保险）
        assert_eq!(engine_index_for("sensevoice"), 0);
        assert_eq!(engine_index_for("bogus"), 0);
        // 显示名：i18n 键已补 → 命中译文；未知 id 原样返回
        assert!(!engine_display("funasr").is_empty());
        assert_ne!(engine_display("funasr"), "engine_display_funasr");
        assert_ne!(engine_display("qwen3"), "engine_display_qwen3");
        assert_eq!(engine_display("nope"), "nope");
    }

    /// FunASR 模型表：顺序同 FUNASR_MODELS，mlt 灰显（D-14），nano 带实验性标注
    #[test]
    fn funasr_model_items_order_and_gating() {
        let items = funasr_model_items();
        assert_eq!(
            items.iter().map(|m| m.key).collect::<Vec<_>>(),
            [
                "sensevoice-small",
                "funasr-nano-2512",
                "funasr-mlt-nano-2512"
            ]
        );
        assert!(items[0].enabled);
        assert!(items[1].enabled);
        // 实验性标注随界面语言（i18n 全局态在并行测试下不确定）→ 双语等价断言
        assert!(
            items[1].display.contains("experimental") || items[1].display.contains("实验性"),
            "nano 应带实验性标注: {}",
            items[1].display
        );
        assert!(!items[2].enabled, "mlt 应灰显（D-14）");
        // 索引映射：合法值命中、非法值回退 0（sanitize 语义）
        assert_eq!(funasr_index_for("funasr-nano-2512"), 1);
        assert_eq!(funasr_index_for("bogus"), 0);
        // 已存 mlt 时索引仍指到灰显项（展示但不可改选其它后再选回）
        assert_eq!(funasr_index_for("funasr-mlt-nano-2512"), 2);
    }

    /// Whisper 档位表：顺序同注册表（5 原版档 + turbo，D-15）、large-v3 唯一慢速提示档
    #[test]
    fn whisper_tier_items_order_and_hints() {
        let items = whisper_tier_items();
        assert_eq!(
            items.iter().map(|t| t.key).collect::<Vec<_>>(),
            ["tiny", "base", "small", "medium", "large-v3", "turbo"]
        );
        assert!(
            items.iter().all(|t| !t.display.is_empty()),
            "显示名不允许空串"
        );
        let slow: Vec<_> = items.iter().filter(|t| t.slow).map(|t| t.key).collect();
        assert_eq!(slow, ["large-v3"], "large-v3 应是唯一慢速提示档");
    }

    /// 档位索引/显示名：builtin 命中、本地路径 → None + "本地: 文件名"、垃圾值兜底
    #[test]
    fn whisper_tier_selection_semantics() {
        assert_eq!(whisper_tier_index_for("tiny"), Some(0));
        assert_eq!(whisper_tier_index_for("turbo"), Some(5));
        assert_eq!(
            whisper_tier_index_for("D:/models/ggml-tiny.bin"),
            None,
            "本地路径不在下拉"
        );
        assert_eq!(whisper_tier_index_for(""), None);
        // builtin 显示名 = 注册表 display（turbo 已去中文注记，双语中性）
        assert_eq!(whisper_tier_display_for("base"), "base");
        assert_eq!(whisper_tier_display_for("turbo"), "turbo");
        // 本地路径：本地前缀 + 文件 stem（不含目录与扩展名）
        let d = whisper_tier_display_for("C:/x/my-model.bin");
        assert!(
            d.starts_with("本地: ") || d.starts_with("Local: "),
            "应带本地前缀: {d}"
        );
        assert!(d.contains("my-model"), "应含文件 stem: {d}");
        // 无文件名的空值原样兜底（不 panic、非本地分支）；任意垃圾值至少包含原值
        assert_eq!(whisper_tier_display_for(""), "");
        assert!(whisper_tier_display_for("??").contains("??"));
    }

    /// 识别语言下拉：30 项、"code - 名"格式、auto 显示名走 i18n、未知码回退 0
    #[test]
    fn asr_lang_items_format_and_index() {
        let items = asr_lang_items();
        assert_eq!(items.len(), 30);
        // auto 项显示名语言相关（i18n 全局态并行测试下不确定）→ 只锁格式与取值来源
        assert_eq!(items[0].0, "auto");
        assert!(
            items[0].1.starts_with("auto - "),
            "格式应为 \"code - 名\": {}",
            items[0].1
        );
        assert_ne!(
            items[0].1, "auto - auto",
            "auto 显示名应取 t(\"asr_lang_auto\") 而非语言码本身"
        );
        // 原生名与界面语言无关
        assert_eq!(items[3], ("zh".into(), "zh - 中文".into()));
        assert_eq!(items[2], ("en".into(), "en - English".into()));
        assert_eq!(asr_lang_index_for("ja"), 1);
        assert_eq!(asr_lang_index_for("auto"), 0);
        assert_eq!(asr_lang_index_for("xx"), 0, "未知码回退 auto");
    }

    // ── 设备字符串语义（None/__disabled__/__default__/Named 往返）──

    /// 扬声器：None=默认(1)、__disabled__=禁用(0)、Named=2+、未知名回退默认
    #[test]
    fn audio_device_semantics_roundtrip() {
        let outputs = dev(&["Speakers A", "Speakers B"]);
        assert_eq!(audio_index_for(None, &outputs), 1);
        assert_eq!(audio_index_for(Some("__disabled__"), &outputs), 0);
        assert_eq!(audio_index_for(Some("Speakers A"), &outputs), 2);
        assert_eq!(audio_index_for(Some("Speakers B"), &outputs), 3);
        assert_eq!(
            audio_index_for(Some("拔出的设备"), &outputs),
            1,
            "未知名回退系统默认"
        );

        // 索引 → settings 字符串（原版 collect 语义）
        assert_eq!(
            audio_setting_for(0, &outputs).as_deref(),
            Some("__disabled__")
        );
        assert_eq!(audio_setting_for(1, &outputs), None);
        assert_eq!(
            audio_setting_for(2, &outputs).as_deref(),
            Some("Speakers A")
        );
        assert_eq!(audio_setting_for(9, &outputs), None, "越界 → 系统默认");

        // 全往返：任一设置字符串 → 索引 → 字符串保持语义一致
        for setting in [
            None,
            Some("__disabled__".to_string()),
            Some("Speakers A".to_string()),
        ] {
            let idx = audio_index_for(setting.as_deref(), &outputs);
            let back = audio_setting_for(idx, &outputs);
            match setting.as_deref() {
                Some("Speakers A") | None | Some("__disabled__") => assert_eq!(back, setting),
                _ => unreachable!(),
            }
        }
    }

    /// 麦克风：None=禁用(0)（与 audio 相反）、__default__/legacy"default"=默认(1)、Named=2+
    #[test]
    fn mic_device_semantics_roundtrip() {
        let inputs = dev(&["Mic X"]);
        assert_eq!(mic_index_for(None, &inputs), 0);
        assert_eq!(mic_index_for(Some("__default__"), &inputs), 1);
        assert_eq!(
            mic_index_for(Some("default"), &inputs),
            1,
            "legacy 写法等价"
        );
        assert_eq!(mic_index_for(Some("Mic X"), &inputs), 2);
        assert_eq!(
            mic_index_for(Some("消失的麦"), &inputs),
            1,
            "未知名回退默认"
        );

        assert_eq!(mic_setting_for(0, &inputs), None);
        assert_eq!(mic_setting_for(1, &inputs).as_deref(), Some("__default__"));
        assert_eq!(mic_setting_for(2, &inputs).as_deref(), Some("Mic X"));
        assert_eq!(
            mic_setting_for(9, &inputs).as_deref(),
            Some("__default__"),
            "越界 → 默认"
        );

        // 与 lt_proto 命令契约的转换一致性（AudioDeviceChoice/MicDeviceChoice）
        let a: AudioDeviceChoice = audio_setting_for(0, &inputs).into();
        assert!(matches!(a, AudioDeviceChoice::Disabled));
        let m: MicDeviceChoice = mic_setting_for(1, &inputs).into();
        assert!(matches!(m, MicDeviceChoice::Default));
    }

    /// D-29：显式开关语义 —— 勾选态 = settings.mic_device 有值；勾选/取消动作
    /// 分别写入 __default__ / None；UI 下拉以 max(1) 从"系统默认"起（禁用也显示预览）
    #[test]
    fn mic_toggle_semantics() {
        assert!(!mic_enabled_for(&None), "默认禁用");
        assert!(mic_enabled_for(&Some("__default__".into())));
        assert!(mic_enabled_for(&Some("Mic X".into())));
        // 勾选 → 写入系统默认；取消 → None
        assert_eq!(mic_setting_for(1, &[]).as_deref(), Some("__default__"));
        assert_eq!(mic_setting_for(0, &[]), None);
        // 下拉索引：禁用态显示"系统默认"预览（max(1)），具名设备正常
        let inputs = dev(&["Mic X"]);
        assert_eq!(mic_index_for(None, &inputs).max(1), 1);
        assert_eq!(mic_index_for(Some("Mic X"), &inputs).max(1), 2);
    }

    // ── 模型缓存判定（临时目录 fixture）──

    fn tmpdir(name: &str) -> std::path::PathBuf {
        let base =
            std::env::temp_dir().join(format!("lt_panel_cache_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    /// 空目录 → Missing（含注册表体积估计）；MS 侧命中 → Cached；
    /// mlt（无注册表条目）→ Unavailable
    #[test]
    fn cache_status_probes_models_dir() {
        let dir = tmpdir("probe");
        // 体积估计随注册表当前值（whisper 字节数曾按上游实测修订）
        let sensevoice_bytes = lt_models::registry::funasr_entry("sensevoice-small")
            .unwrap()
            .estimated_bytes;
        let tiny_bytes = lt_models::registry::whisper_entry_for("tiny")
            .unwrap()
            .estimated_bytes;
        // 未缓存：funasr sensevoice-small
        assert_eq!(
            model_cache_status(&dir, "funasr", "sensevoice-small", ""),
            CacheStatus::Missing(sensevoice_bytes)
        );
        // MS 镜像仓写入完整 manifest（下限过阈）→ 命中（DL-1 起 manifest 逐文件校验：
        // 旧版单文件过体积阈值即判已缓存，半截仓会误判）
        let ms_dir = lt_models::paths::ms_cache_root(&dir)
            .join("pengzhendong")
            .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue");
        std::fs::create_dir_all(&ms_dir).unwrap();
        let half_min = (sensevoice_bytes / 2).max(50_000_000);
        std::fs::write(
            ms_dir.join("model.int8.onnx"),
            vec![0u8; half_min as usize + 1],
        )
        .unwrap();
        std::fs::write(ms_dir.join("tokens.txt"), vec![0u8; 4_096]).unwrap();
        assert_eq!(
            model_cache_status(&dir, "funasr", "sensevoice-small", ""),
            CacheStatus::Cached(sensevoice_bytes)
        );
        // mlt 无条目 → Unavailable（D-14）
        assert_eq!(
            model_cache_status(&dir, "funasr", "funasr-mlt-nano-2512", ""),
            CacheStatus::Unavailable
        );
        // whisper builtin 未缓存 / 本地路径存在即缓存
        assert_eq!(
            model_cache_status(&dir, "whisper", "", "tiny"),
            CacheStatus::Missing(tiny_bytes)
        );
        let local = dir.join("my-model.bin");
        std::fs::write(&local, b"ggml").unwrap();
        assert_eq!(
            model_cache_status(&dir, "whisper", "", local.to_str().unwrap()),
            CacheStatus::Cached(0)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 字节量格式与原版 format_size 阈值一致
    #[test]
    fn size_format_matches_original() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(250_000_000), "238.4 MB");
        assert_eq!(format_size(3_100_000_000), "2.89 GB");
    }
}
