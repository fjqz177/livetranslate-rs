//! 诊断页（对照原版 panel/tabs/diagnostics_tab.py + ui/diagnostics.py 的信息面，
//! 按 M4.4 规格取只读信息卡子集）：
//! 平台（OS/音频后端 wasapi）、识别（引擎 + 模型）、翻译/网络（活动模型 +
//! API 地址 + 代理模式）、存储（配置/模型/转录/日志目录）、复制诊断摘要按钮。
//!
//! 已知偏差：原版七卡中的热键/权限/加速器/日志尾行卡与"打包诊断 zip"依赖
//! 宿主组合根与 zip 工具，随对应里程碑接入；本页全部信息仅取自 settings 与
//! lt_models::paths（无外部探测）。

use super::{group_card, hint_line, Palette};
use crate::state::AppState;
use egui::{RichText, Ui};

/// 复制摘要首行（原版 diagnostics.session_id 语义的简化：应用名 + 版本）
pub fn summary_header() -> String {
    format!("LiveTranslate {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS)
}

/// 诊断摘要全文（key: value 行；不含 API Key，API 地址原样保留——
/// 原版 redact_text 仅遮蔽 query/token 类文本，路径不脱敏）
pub fn build_summary(s: &lt_proto::Settings) -> String {
    let model = s.models.get(s.active_model).or_else(|| s.models.first());
    let asr_model =
        if s.asr_engine == "whisper" { s.whisper_model_size.clone() } else { s.funasr_model.clone() };
    let mut lines = vec![summary_header()];
    lines.push(format!("{}: wasapi", lt_i18n::t("diag_audio_backend")));
    lines.push(format!(
        "{}: {} ({asr_model})",
        lt_i18n::t("diag_engine"),
        super::vad::engine_display(&s.asr_engine)
    ));
    match model {
        Some(m) => {
            lines.push(format!(
                "{}: {} ({})",
                lt_i18n::t("diag_model"),
                m.name,
                m.model
            ));
            lines.push(format!("{}: {}", lt_i18n::t("diag_api_base"), m.api_base));
            lines.push(format!(
                "{}: {}",
                lt_i18n::t("diag_proxy_mode"),
                if m.proxy.is_empty() || m.proxy == "none" {
                    lt_i18n::t("proxy_none")
                } else if m.proxy == "system" {
                    lt_i18n::t("proxy_system")
                } else {
                    m.proxy.clone()
                }
            ));
        }
        None => lines.push(format!("{}: --", lt_i18n::t("diag_model"))),
    }
    for (label, path) in storage_rows(s) {
        lines.push(format!("{label}: {}", path.display()));
    }
    lines.join("\n")
}

/// 存储卡四行（配置/模型/转录/日志目录；解析失败回退临时目录占位）
pub fn storage_rows(s: &lt_proto::Settings) -> Vec<(&'static str, std::path::PathBuf)> {
    let fallback = std::env::temp_dir();
    vec![
        (
            "diag_config_dir",
            lt_models::paths::config_dir().unwrap_or_else(|_| fallback.clone()),
        ),
        (
            "diag_models_dir",
            lt_models::paths::models_dir(s.models_dir.as_deref()).unwrap_or_else(|_| fallback.clone()),
        ),
        (
            "diag_transcripts_dir",
            lt_models::paths::transcripts_dir().unwrap_or_else(|_| fallback.clone()),
        ),
        (
            "diag_log_dir",
            lt_models::paths::logs_dir().unwrap_or_else(|_| fallback.clone()),
        ),
    ]
}

/// 诊断页 UI 总入口
pub fn page(ui: &mut Ui, state: &mut AppState, pal: &Palette) {
    let s = state.settings.clone();

    // ── 平台能力（原版 _card_platform 子集 + 应用版本）──
    group_card(ui, pal, &lt_i18n::t("diag_platform"), |ui| {
        kv_row(
            ui,
            pal,
            "LiveTranslate",
            format!("v{} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS),
        );
        kv_row(ui, pal, &lt_i18n::t("diag_os"), std::env::consts::OS.to_string());
        kv_row(ui, pal, &lt_i18n::t("diag_audio_backend"), "wasapi".to_string());
    });

    // ── 识别（ASR 引擎与模型，读 settings）──
    group_card(ui, pal, &lt_i18n::t("group_asr_engine"), |ui| {
        let asr_model = if s.asr_engine == "whisper" {
            s.whisper_model_size.clone()
        } else {
            s.funasr_model.clone()
        };
        kv_row(ui, pal, &lt_i18n::t("diag_engine"), super::vad::engine_display(&s.asr_engine));
        kv_row(ui, pal, &lt_i18n::t("label_funasr_model"), asr_model);
    });

    // ── 翻译 / 网络（原版 _card_network：模型 + API + 代理）──
    group_card(ui, pal, &lt_i18n::t("diag_network"), |ui| {
        match s.models.get(s.active_model).or_else(|| s.models.first()) {
            Some(m) => {
                kv_row(ui, pal, &lt_i18n::t("diag_model"), format!("{} ({})", m.name, m.model));
                kv_row(ui, pal, &lt_i18n::t("diag_api_base"), m.api_base.clone());
                let proxy = if m.proxy.is_empty() || m.proxy == "none" {
                    lt_i18n::t("proxy_none")
                } else if m.proxy == "system" {
                    lt_i18n::t("proxy_system")
                } else {
                    m.proxy.clone()
                };
                kv_row(ui, pal, &lt_i18n::t("diag_proxy_mode"), proxy);
            }
            None => kv_row(ui, pal, &lt_i18n::t("diag_model"), "--".into()),
        }
    });

    // ── 存储（原版 _card_storage 四行）──
    group_card(ui, pal, &lt_i18n::t("diag_storage"), |ui| {
        for (label, path) in storage_rows(&s) {
            kv_row(ui, pal, &lt_i18n::t(label), path.display().to_string());
        }
    });

    // ── 操作行（复制摘要；打包 zip 随后续里程碑）──
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::Button::new(RichText::new(lt_i18n::t("diag_copy_summary")).size(12.5))
                    .corner_radius(6.0),
            )
            .clicked()
        {
            let text = build_summary(&s);
            ui.ctx().copy_text(text.clone());
            tracing::info!("诊断摘要已复制到剪贴板（{} 字符）", text.chars().count());
        }
        hint_line(ui, pal, &lt_i18n::t("diag_lazy_hint"));
    });

    ui.add_space(8.0);
}

/// "标签: 值"行（值可选中文本，原版 TextSelectableByMouse）
fn kv_row(ui: &mut Ui, pal: &Palette, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{label}: ")).color(pal.weak));
        ui.label(RichText::new(value).monospace().color(pal.text));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 摘要含版本/引擎/模型/目录行，且绝不包含 API Key 值
    #[test]
    fn summary_contains_versions_and_paths_but_never_api_key() {
        let model = lt_proto::ModelConfig {
            api_key: "sk-SUPER-SECRET".into(),
            name: "deepseek".into(),
            model: "deepseek-chat".into(),
            proxy: "system".into(),
            ..lt_proto::ModelConfig::default()
        };
        let s = lt_proto::Settings { models: vec![model], ..lt_proto::Settings::default() };
        let text = build_summary(&s);
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("deepseek"));
        assert!(text.contains("wasapi"));
        // 存储四行：默认配置根 ~/.config/livetranslate 的基名 + transcripts
        assert!(text.contains("livetranslate"), "{text}");
        assert!(text.contains("transcripts"), "{text}");
        assert!(!text.contains("sk-SUPER-SECRET"), "API Key 不得进入摘要");
        // proxy=system → 代理行走 i18n 文案（原始字符串不出现；语言并行测试下
        // 不确定 → 双语任一命中即可）
        assert!(
            !text.lines().any(|l| l.ends_with(": system")),
            "代理行应显示为 i18n 文案: {text}"
        );
    }

    /// 存储四行键序与可解析性
    #[test]
    fn storage_rows_cover_four_dirs() {
        let rows = storage_rows(&lt_proto::Settings::default());
        assert_eq!(
            rows.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            ["diag_config_dir", "diag_models_dir", "diag_transcripts_dir", "diag_log_dir"]
        );
        // models_dir 尊重自定义值
        let s = lt_proto::Settings {
            models_dir: Some(std::path::PathBuf::from("D:/custom-models")),
            ..lt_proto::Settings::default()
        };
        let rows = storage_rows(&s);
        assert_eq!(rows[1].1, std::path::PathBuf::from("D:/custom-models"));
    }

    /// 摘要首行 = 应用名 + 版本 + 平台
    #[test]
    fn summary_header_format() {
        let h = summary_header();
        assert!(h.starts_with("LiveTranslate "));
        assert!(h.contains(env!("CARGO_PKG_VERSION")));
        assert!(h.contains(std::env::consts::OS));
    }
}
