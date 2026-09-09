//! settings.json 数据契约 —— 与原版 user_settings.json 逐键兼容。
//!
//! 兼容策略（对照 docs/archive/rewrite-plan.md §3.1/3.2 与原版 get_data()/migrate 行为）：
//! - 序列化：默认值不写盘（`skip_serializing_if`），与原版"条件写入"一致；
//! - 反序列化：字段全 `#[serde(default)]`，缺失即默认；
//! - 导入原版文件：经 [`Settings::from_value_compatible`] 做 legacy 键迁移
//!   （`no_think` → `thinking_style` 等）与非法值回退（记日志）。
//! - Rust 版新增键（原版无对应）：`models_dir`、`ui_font_family`、
//!   `subtitle_font_family`（D-17，缺失即默认，导入原版文件不受影响）。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 合法引擎值（r8 双引擎 + WP-B qwen3；anime/remote 已裁剪，导入时回退）
pub const ASR_ENGINES: [&str; 3] = ["funasr", "whisper", "qwen3"];

/// 识别引擎值域的类型化形态（E2/D-79，ADR-9「String 持久层 + 枚举透镜」）：
/// `Settings.asr_engine` 持久层保持 String（与原版 user_settings.json 逐键
/// 兼容），运行时判定一律经 [`Settings::engine_key`] 透镜——未知值单点
/// 回退 FunAsr + warn，替代散落各域的 `== "funasr"` 字面量比较。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineKey {
    FunAsr,
    Whisper,
    Qwen3,
}

impl EngineKey {
    /// settings 字符串 → 值域枚举（唯一转换点；未知值回退 FunAsr 并告警，
    /// 与 sanitize 的既有回退语义一致）
    pub fn from_settings_str(v: &str) -> Self {
        match v {
            "funasr" => Self::FunAsr,
            "whisper" => Self::Whisper,
            "qwen3" => Self::Qwen3,
            other => {
                tracing::warn!("未知 asr_engine 值 {other:?}，回退 funasr（EngineKey 单点转换）");
                Self::FunAsr
            }
        }
    }

    /// → settings 持久层字符串（写回 `Settings.asr_engine` 用）
    pub fn as_settings_str(self) -> &'static str {
        match self {
            Self::FunAsr => "funasr",
            Self::Whisper => "whisper",
            Self::Qwen3 => "qwen3",
        }
    }
}

/// 代理三模式（语义对齐原版 proxy="none" 绕系统代理，E-03；E2/D-79 自
/// lt-download 迁入契约层——`settings.download_proxy` 与
/// `Cmd::StartDownload` 载荷共用的值域类型）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyMode {
    /// 强制直连（trust_env=False 等价）
    #[default]
    None,
    /// 跟随系统环境变量
    System,
    /// 指定 URL
    Url(String),
}

impl ProxyMode {
    /// settings 字符串 → 值域枚举（唯一转换点；空串视同 system——原版语义）
    pub fn from_settings_str(v: &str) -> Self {
        match v {
            "none" => Self::None,
            "" | "system" => Self::System,
            url => Self::Url(url.to_string()),
        }
    }

    /// → settings 持久层字符串（写回 `settings.download_proxy` 用）
    pub fn to_settings_str(&self) -> String {
        match self {
            Self::None => "none".into(),
            Self::System => "system".into(),
            Self::Url(u) => u.clone(),
        }
    }
}
/// 合法 funasr 模型（mlt 置灰但仍是合法存量值）
pub const FUNASR_MODELS: [&str; 3] = [
    "sensevoice-small",
    "funasr-nano-2512",
    "funasr-mlt-nano-2512",
];
/// 合法 whisper 档位（r8：+turbo）
pub const WHISPER_SIZES: [&str; 6] = ["tiny", "base", "small", "medium", "large-v3", "turbo"];
/// 旧版独立引擎名 → funasr 模型（对照原版 FUNASR_LEGACY_ENGINE_ALIASES；
/// 引擎别名命中时必须覆盖 funasr_model，即使其已是合法值）
pub const ASR_ENGINE_LEGACY_ALIASES: [(&str, &str); 3] = [
    ("sensevoice", "sensevoice-small"),
    ("funasr-nano", "funasr-nano-2512"),
    ("funasr-mlt-nano", "funasr-mlt-nano-2512"),
];

/// 语言代码归一（D-74）：小写 + 主子标签（"-" 前），`"zh-CN"` → `"zh"`、
/// `"ZH-cn"` → `"zh"`；`"auto"`/空串原样（空前缀回空串）。
/// 用途：同语言免翻译比较（目标语言侧归一；ASR 检出侧保持原样——检出
/// 码本就是短码，归一它属于新行为，超出 D-74 登记面）。
pub fn normalize_language(code: &str) -> String {
    let lower = code.trim().to_lowercase();
    let primary = lower.split('-').next().unwrap_or_default().trim();
    primary.to_string()
}

fn normalize_funasr_model(v: &str) -> String {
    match v {
        "sensevoice" => "sensevoice-small".into(),
        "funasr-nano" => "funasr-nano-2512".into(),
        "funasr-mlt-nano" => "funasr-mlt-nano-2512".into(),
        "sensevoice-small" | "funasr-nano-2512" | "funasr-mlt-nano-2512" => v.into(),
        _ => "sensevoice-small".into(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    // ── VAD ──
    pub vad_mode: String,   // silero | energy | disabled
    pub vad_threshold: f32, // 0.0-1.0
    pub energy_threshold: f32,
    pub min_speech_duration: f32, // 秒
    pub max_speech_duration: f32, // 秒
    pub silence_mode: String,     // auto | fixed
    pub silence_duration: f32,    // 秒
    // ── ASR ──
    pub asr_engine: String, // funasr | whisper
    pub funasr_model: String,
    pub asr_language: String,       // "auto" | ISO 码
    pub whisper_model_size: String, // 6 档 | 本地 GGML 路径
    pub sensevoice_pad_seconds: f32,
    pub whisper_pad_seconds: f32,
    pub hub: String,            // ms | hf
    pub download_proxy: String, // none | system | URL
    pub incremental_asr: bool,
    pub interim_interval: f32, // 秒
    // ── 音频设备 ──
    pub audio_device: Option<String>, // None=默认 | 名 | "__disabled__"
    pub mic_device: Option<String>,   // None=禁用 | "__default__" | 名
    // ── 翻译 ──
    pub models: Vec<ModelConfig>,
    pub active_model: usize,
    pub target_language: String,
    pub system_prompt: String,
    pub timeout: u32,    // 秒
    pub ui_lang: String, // en | zh
    // ── 字体（D-17，Rust 版新增：默认内嵌思源，行级键空串=跟随主设置）──
    pub ui_font_family: String, // 界面字体族名（内嵌思源名或系统字体名）
    pub subtitle_font_family: String, // 字幕/悬浮窗显示文本主字体族名
    // ── UI ──
    pub style: Style,
    pub subtitle_mode: SubtitleMode,
    pub overlay_x: Option<i32>,
    pub overlay_y: Option<i32>,
    pub overlay_w: Option<u32>,
    pub overlay_h: Option<u32>,
    pub auto_save_transcript: bool,
    /// Rust 版新增：模型缓存根路径；None → ~/.config/livetranslate/models
    pub models_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vad_mode: "silero".into(),
            vad_threshold: 0.5,
            energy_threshold: 0.02,
            min_speech_duration: 1.0,
            max_speech_duration: 8.0,
            silence_mode: "auto".into(),
            silence_duration: 0.8,
            asr_engine: "funasr".into(),
            funasr_model: "sensevoice-small".into(),
            asr_language: "auto".into(),
            whisper_model_size: "medium".into(),
            sensevoice_pad_seconds: 0.5,
            whisper_pad_seconds: 0.5,
            hub: "ms".into(),
            download_proxy: "system".into(),
            incremental_asr: false,
            interim_interval: 2.0,
            audio_device: None,
            mic_device: None,
            models: vec![ModelConfig::default()],
            active_model: 0,
            target_language: "zh".into(),
            // 为空时由 lt-translate 用 DEFAULT_PROMPT 填充
            system_prompt: String::new(),
            timeout: 10,
            ui_lang: "en".into(),
            ui_font_family: "Noto Sans CJK SC".into(),
            subtitle_font_family: "Noto Sans CJK SC".into(),
            style: Style::default(),
            subtitle_mode: SubtitleMode::default(),
            overlay_x: None,
            overlay_y: None,
            overlay_w: None,
            overlay_h: None,
            auto_save_transcript: true,
            models_dir: None,
        }
    }
}

impl Settings {
    /// 引擎值域透镜（E2/D-79）：运行时引擎判定的唯一入口——判定处不再写
    /// `asr_engine == "…"` 字面量
    pub fn engine_key(&self) -> EngineKey {
        EngineKey::from_settings_str(&self.asr_engine)
    }

    /// 下载源透镜（E2/D-79）：`settings.hub` 字符串 → 值域枚举
    pub fn hub(&self) -> crate::layout::Hub {
        crate::layout::Hub::from_settings_str(&self.hub)
    }

    /// 代理模式透镜（E2/D-79）：`settings.download_proxy` → 值域枚举
    pub fn proxy_mode(&self) -> ProxyMode {
        ProxyMode::from_settings_str(&self.download_proxy)
    }

    /// 兼容导入：任意 JSON（原版 user_settings.json / 本版 settings.json）→ Settings。
    /// 完成后调用 [`Settings::sanitize`] 做非法值回退。
    pub fn from_value_compatible(v: Value) -> Self {
        let mut root = v;
        // legacy：models[].no_think → thinking_style（原版迁移规则：
        // 无 thinking_style 时 no_think=true→"auto"、false→"off"）
        if let Some(models) = root.get_mut("models").and_then(|m| m.as_array_mut()) {
            for m in models {
                let has_style = m
                    .get("thinking_style")
                    .map(|s| !s.is_null())
                    .unwrap_or(false);
                if !has_style {
                    if let Some(no_think) = m.get("no_think").and_then(|b| b.as_bool()) {
                        m["thinking_style"] =
                            Value::String(if no_think { "auto" } else { "off" }.into());
                    }
                }
                m.as_object_mut().map(|o| o.remove("no_think"));
            }
        }
        // AH-10/H21：类型漂移（旧版/手改键类型不符）此前静默全量回退默认值
        // ——引擎/语言/设备全丢且无提示，必须留痕
        let mut s: Settings = serde_json::from_value(root).unwrap_or_else(|e| {
            tracing::warn!("settings.json 部分键类型不兼容，已回退默认值: {e}");
            Settings::default()
        });
        s.sanitize();
        s
    }

    /// 非法/裁剪值回退，返回被修正项的描述（供调用方记日志）
    pub fn sanitize(&mut self) -> Vec<String> {
        let mut fixed = Vec::new();
        // legacy 引擎别名迁移（对照原版 normalize_asr_engine_selection/
        // migrate_funasr_settings）：命中别名表 → funasr + 别名目标，
        // 且覆盖当前 funasr_model（原版语义：引擎别名优先）
        if let Some((_, target)) = ASR_ENGINE_LEGACY_ALIASES
            .iter()
            .find(|(from, _)| *from == self.asr_engine)
        {
            let old = std::mem::replace(&mut self.asr_engine, "funasr".into());
            let target = (*target).into();
            fixed.push(format!("asr_engine: '{old}' → funasr + {target}"));
            self.funasr_model = target;
        }
        if !ASR_ENGINES.contains(&self.asr_engine.as_str()) {
            let old = std::mem::replace(&mut self.asr_engine, "funasr".into());
            fixed.push(format!("asr_engine: '{old}' 已裁剪/非法 → funasr"));
        }
        if !FUNASR_MODELS.contains(&self.funasr_model.as_str()) {
            let old = self.funasr_model.clone();
            let mut new = normalize_funasr_model(&old);
            if !FUNASR_MODELS.contains(&new.as_str()) {
                new = "sensevoice-small".into();
            }
            self.funasr_model = new.clone();
            fixed.push(format!("funasr_model: '{old}' → {new}"));
        }
        if !["silero", "energy", "disabled"].contains(&self.vad_mode.as_str()) {
            fixed.push(format!("vad_mode: '{}' → silero", self.vad_mode));
            self.vad_mode = "silero".into();
        }
        if !["auto", "fixed"].contains(&self.silence_mode.as_str()) {
            fixed.push(format!("silence_mode: '{}' → auto", self.silence_mode));
            self.silence_mode = "auto".into();
        }
        if !["ms", "hf"].contains(&self.hub.as_str()) {
            fixed.push(format!("hub: '{}' → ms", self.hub));
            self.hub = "ms".into();
        }
        if self.asr_language.is_empty() {
            self.asr_language = "auto".into();
        }
        // D-17：字体主键空串（手改/损坏）回退默认内嵌思源；
        // 行级键空串是合法语义（=跟随主设置），不回退
        if self.ui_font_family.is_empty() {
            self.ui_font_family = "Noto Sans CJK SC".into();
            fixed.push("ui_font_family: 空 → 默认思源".into());
        }
        if self.subtitle_font_family.is_empty() {
            self.subtitle_font_family = "Noto Sans CJK SC".into();
            fixed.push("subtitle_font_family: 空 → 默认思源".into());
        }
        if self.models.is_empty() {
            self.models.push(ModelConfig::default());
            fixed.push("models: 空 → 填充默认占位模型".into());
        }
        // legacy 显示名迁移（原版悬浮窗/托盘显示 name；旧占位 name="default"
        // 时代码以 model 字段为真名）→ name=="default" 且 model 有值时以 model 为准
        for m in &mut self.models {
            if m.name == "default" && !m.model.is_empty() && m.model != "default" {
                m.name = m.model.clone();
                fixed.push(format!("model name: 'default' → '{}'", m.name));
            }
        }
        if self.active_model >= self.models.len() {
            fixed.push(format!("active_model: {} 越界 → 0", self.active_model));
            self.active_model = 0;
        }
        fixed
    }
}

/// 翻译模型配置 —— 序列化形状与原版 ModelEditDialog.get_data() 逐键一致。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "snake_case")]
pub struct ModelConfig {
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    /// "none" | "system" | 自定义 URL
    pub proxy: String,
    /// 仅 true 时写盘
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub no_system_role: bool,
    /// 仅非 "auto" 时写盘（None = auto）
    #[serde(skip_serializing_if = "skip_thinking_auto")]
    pub thinking_style: Option<String>,
    /// 默认 true；仅 false 时写盘（键名对齐原版 `streaming: false`）
    #[serde(skip_serializing_if = "is_true")]
    pub streaming: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub json_response: bool,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub context_turns: u32,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub input_price: f64, // 美元 / 1M tokens
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub output_price: f64,
    /// 仅勾选的键（原版 Advanced "checkbox+value" 行）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, Value>>,
    /// 用户自定义 extra_body（JSON object）
    #[serde(skip_serializing_if = "skip_empty_extra_body")]
    pub extra_body: Option<Value>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            // 原版 _ensure_models：无已存模型时 name=model（悬浮窗/托盘显示模型名而非"default"）
            name: "hunyuan-mt-chimera-7b".into(),
            api_base: "http://127.0.0.1:1234/v1".into(),
            api_key: String::new(),
            model: "hunyuan-mt-chimera-7b".into(),
            proxy: "none".into(),
            no_system_role: false,
            thinking_style: None,
            streaming: true,
            json_response: false,
            context_turns: 0,
            input_price: 0.0,
            output_price: 0.0,
            overrides: None,
            extra_body: None,
        }
    }
}

fn skip_thinking_auto(v: &Option<String>) -> bool {
    match v {
        None => true,
        Some(s) => s == "auto",
    }
}
fn is_true(b: &bool) -> bool {
    *b
}
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}
fn skip_empty_extra_body(v: &Option<Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::Object(m)) => m.is_empty(),
        Some(_) => false,
    }
}

/// 悬浮窗样式（14 键，对照 DEFAULT_STYLE）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "snake_case")]
pub struct Style {
    pub preset: String,
    pub bg_color: String,
    pub bg_opacity: u32, // 0-255
    pub header_color: String,
    pub header_opacity: u32, // 0-255
    pub border_radius: u32,  // px
    /// D-17（2026-09-07 用户裁决）：空串 = 跟随 settings.subtitle_font_family；
    /// 原版默认 "Microsoft YaHei"（此处保留原值为空语义 1:1 于旧文件导入）
    pub original_font_family: String,
    /// D-17：空串 = 跟随 subtitle_font_family（原版默认 "Microsoft YaHei"）
    pub translation_font_family: String,
    pub original_font_size: u32,    // pt
    pub translation_font_size: u32, // pt
    pub original_color: String,
    pub translation_color: String,
    pub timestamp_color: String,
    pub window_opacity: u32, // 百分比 30-100
}

impl Default for Style {
    fn default() -> Self {
        Self {
            preset: "default".into(),
            bg_color: "#000000".into(),
            bg_opacity: 240,
            header_color: "#1a1a2e".into(),
            header_opacity: 230,
            border_radius: 8,
            // D-17：原版默认 "Microsoft YaHei"，改为空串（跟随字幕主字体）
            original_font_family: "".into(),
            translation_font_family: "".into(),
            original_font_size: 11,
            translation_font_size: 14,
            original_color: "#cccccc".into(),
            translation_color: "#ffffff".into(),
            timestamp_color: "#888899".into(),
            window_opacity: 95,
        }
    }
}

/// 字幕窗单行配置（15 键，对照 DEFAULT_SUBTITLE_WIN_SETTINGS.lines[]）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "snake_case")]
pub struct SubtitleLine {
    /// original | translation
    pub line_type: String,
    /// 仅 translation 行有意义（ISO 码）
    pub lang: Option<String>,
    pub enabled: bool,
    pub font_family: String,
    pub font_size: u32, // pt
    pub color: String,
    pub opacity: u32,  // 0-255
    pub align: String, // left | center | right
    pub outline_enabled: bool,
    pub outline_color: String,
    pub outline_width: u32, // px
    pub bg_image: String,
    pub entry_animation: String, // none|fade|slide_left|slide_right|slide_up|slide_down
    pub exit_animation: String,
    pub animation_duration: u32, // ms
}

impl Default for SubtitleLine {
    fn default() -> Self {
        Self {
            line_type: "translation".into(),
            lang: Some("en".into()),
            enabled: true,
            // D-17：原版默认 "Microsoft YaHei"，改为空串（跟随字幕主字体）
            font_family: "".into(),
            font_size: 24,
            color: "#FFFFFF".into(),
            opacity: 255,
            align: "center".into(),
            outline_enabled: true,
            outline_color: "#000000".into(),
            outline_width: 2,
            bg_image: String::new(),
            entry_animation: "none".into(),
            exit_animation: "none".into(),
            animation_duration: 300,
        }
    }
}

impl SubtitleLine {
    /// 原版默认两行
    pub fn default_pair() -> Vec<SubtitleLine> {
        let orig = Self {
            line_type: "original".into(),
            lang: None,
            font_size: 24,
            ..Default::default()
        };
        let trans = Self {
            line_type: "translation".into(),
            lang: Some("zh".into()),
            font_size: 28,
            color: "#FFD700".into(),
            ..Default::default()
        };
        vec![orig, trans]
    }
}

/// 字幕窗配置（对照 DEFAULT_SUBTITLE_WIN_SETTINGS 顶层 + window_x/y）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "snake_case")]
pub struct SubtitleMode {
    pub enabled: bool,
    pub sentences: u32,
    pub window_width: u32, // px
    pub line_spacing: u32, // px
    pub bg_color: String,
    pub bg_opacity: u32, // 0-255
    pub bg_image: String,
    pub border_radius: u32,
    pub auto_hide_timeout: u32, // 秒，0=禁用
    pub auto_hide_animation: String,
    pub auto_hide_duration: u32, // ms
    pub click_through: bool,
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    pub lines: Vec<SubtitleLine>,
}

impl Default for SubtitleMode {
    fn default() -> Self {
        Self {
            enabled: false,
            sentences: 1,
            window_width: 1000,
            line_spacing: 8,
            bg_color: "#000000".into(),
            // D-36：76→190（原版 30% 在单窗 uniform alpha 下文字过淡；190=深色胶带
            // 观感：背景透 25%，文字对比足——见 docs/archive/subtitle-window-overhaul.md §5.5）
            bg_opacity: 190,
            bg_image: String::new(),
            border_radius: 8,
            auto_hide_timeout: 5,
            auto_hide_animation: "fade".into(),
            auto_hide_duration: 300,
            click_through: false,
            window_x: None,
            window_y: None,
            lines: SubtitleLine::default_pair(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E2/D-79：EngineKey 单点转换——合法值域恒等、未知值回退 FunAsr
    #[test]
    fn engine_key_mapping_and_fallback() {
        for v in ASR_ENGINES {
            let k = EngineKey::from_settings_str(v);
            assert_eq!(k.as_settings_str(), v, "值域 {v} 往返必须恒等");
        }
        assert_eq!(
            EngineKey::from_settings_str("qwen3"),
            EngineKey::Qwen3
        );
        // 未知值（含旧版残留/手改）回退 FunAsr 而非 panic——与 sanitize 同语义
        assert_eq!(EngineKey::from_settings_str("sensevoice"), EngineKey::FunAsr);
        assert_eq!(EngineKey::from_settings_str(""), EngineKey::FunAsr);
        // Settings 透镜
        let mut s = Settings::default();
        assert_eq!(s.engine_key(), EngineKey::FunAsr);
        s.asr_engine = "qwen3".into();
        assert_eq!(s.engine_key(), EngineKey::Qwen3);
    }

    /// E2/D-79：ProxyMode 往返恒等（空串视同 system 的原版语义钉死）
    #[test]
    fn proxy_mode_roundtrip() {
        for src in ["none", "system", "", "http://p:8080"] {
            let p = ProxyMode::from_settings_str(src);
            assert_eq!(p.to_settings_str(), if src.is_empty() { "system" } else { src });
        }
        assert_eq!(Settings::default().proxy_mode(), ProxyMode::System);
    }

    /// D-36：字幕窗默认背景不透明度 = 190（深色胶带观感：背景透 25%、
    /// 文字对比足；原版 76 在单窗 uniform alpha 下文字过淡）
    #[test]
    fn subtitle_default_bg_opacity_190() {
        assert_eq!(SubtitleMode::default().bg_opacity, 190);
        assert_eq!(SubtitleMode::default().bg_color, "#000000");
    }

    /// 与原版首启向导写入的 13 键默认块对照（dialogs.py:344-355）
    #[test]
    fn wizard_default_block_roundtrip() {
        let js = r#"{
            "hub": "ms", "download_proxy": "system", "asr_engine": "funasr",
            "funasr_model": "sensevoice-small", "vad_mode": "silero",
            "vad_threshold": 0.3, "energy_threshold": 0.02,
            "min_speech_duration": 1.0, "max_speech_duration": 8.0,
            "silence_mode": "auto", "silence_duration": 0.8,
            "asr_language": "auto", "target_language": "zh"
        }"#;
        let s = Settings::from_value_compatible(serde_json::from_str(js).unwrap());
        assert_eq!(s.vad_threshold, 0.3);
        assert_eq!(s.hub, "ms");
        assert_eq!(s.max_speech_duration, 8.0);
        // 未给的键走默认
        assert_eq!(s.timeout, 10);
    }

    #[test]
    fn legacy_import_fixups() {
        let js = r#"{
            "asr_engine": "anime-whisper",
            "funasr_model": "sensevoice",
            "models": [{"name":"a","api_base":"b","api_key":"c","model":"d",
                        "no_think": false}],
            "active_model": 5
        }"#;
        let s = Settings::from_value_compatible(serde_json::from_str(js).unwrap());
        assert_eq!(s.asr_engine, "funasr"); // 裁剪引擎回退
        assert_eq!(s.funasr_model, "sensevoice-small"); // legacy 别名
        assert_eq!(s.active_model, 0); // 越界回退
                                       // no_think=false → thinking_style=off
        assert_eq!(s.models[0].thinking_style.as_deref(), Some("off"));
        // 序列化后不再出现 no_think
        let out = serde_json::to_value(&s).unwrap();
        assert!(out["models"][0].get("no_think").is_none());
    }

    /// legacy 引擎别名：asr_engine 命中别名表 → funasr + 别名目标，
    /// 且覆盖已存的 funasr_model（对照原版 migrate_funasr_settings）
    #[test]
    fn legacy_engine_alias_migration() {
        // funasr-nano → funasr + funasr-nano-2512
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr-nano"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "funasr-nano-2512");

        // 引擎别名优先：即使已存合法 funasr_model 也被覆盖
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "sensevoice",
            "funasr_model": "funasr-nano-2512"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "sensevoice-small");

        // funasr-mlt-nano → funasr + funasr-mlt-nano-2512
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr-mlt-nano"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "funasr-mlt-nano-2512");
    }

    /// 正常引擎值不触发别名迁移，funasr_model 保持原样
    #[test]
    fn normal_engine_values_untouched() {
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr",
            "funasr_model": "funasr-mlt-nano-2512"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "funasr-mlt-nano-2512");

        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "whisper"
        }));
        assert_eq!(s.asr_engine, "whisper");
        // whisper 引擎不动 funasr_model（默认值）
        assert_eq!(s.funasr_model, "sensevoice-small");
    }

    /// WP-B（B-α）：qwen3 为合法第三引擎值，sanitize 原样放行；
    /// 非法值仍回退 funasr（funasr_model 保持不动）
    #[test]
    fn qwen3_engine_value_passes_sanitize() {
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "qwen3",
            "funasr_model": "funasr-nano-2512"
        }));
        assert_eq!(s.asr_engine, "qwen3");
        // qwen3 不消费 funasr_model，但也不动它（切回 funasr 时语义仍在）
        assert_eq!(s.funasr_model, "funasr-nano-2512");

        let mut s: Settings =
            serde_json::from_value(serde_json::json!({ "asr_engine": "qwen4" })).unwrap();
        let fixed = s.sanitize();
        assert_eq!(s.asr_engine, "funasr");
        assert!(fixed.iter().any(|f| f.starts_with("asr_engine")));
    }

    #[test]
    fn conditional_serialization_shape() {
        // 默认模型：可选键全部不出现（对齐原版"省略默认值"）
        let v = serde_json::to_value(ModelConfig::default()).unwrap();
        for key in [
            "no_system_role",
            "thinking_style",
            "streaming",
            "json_response",
            "context_turns",
            "input_price",
            "output_price",
            "overrides",
            "extra_body",
        ] {
            assert!(v.get(key).is_none(), "默认模型不应写出 {key}");
        }
        // streaming=false 必须写出（原版仅 false 落盘）
        let m = ModelConfig {
            streaming: false,
            ..Default::default()
        };
        let v = serde_json::to_value(m).unwrap();
        assert_eq!(v["streaming"], serde_json::json!(false));
        // thinking_style="auto" 不写；"deepseek" 要写
        let m = ModelConfig {
            thinking_style: Some("auto".into()),
            ..Default::default()
        };
        assert!(serde_json::to_value(m)
            .unwrap()
            .get("thinking_style")
            .is_none());
        let m = ModelConfig {
            thinking_style: Some("deepseek".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(m).unwrap()["thinking_style"],
            serde_json::json!("deepseek")
        );
    }

    #[test]
    fn style_and_subtitle_defaults_match_original() {
        let st = Style::default();
        assert_eq!(st.bg_opacity, 240);
        assert_eq!(st.window_opacity, 95);
        let sm = SubtitleMode::default();
        assert_eq!(sm.window_width, 1000);
        assert_eq!(sm.lines.len(), 2);
        assert_eq!(sm.lines[1].color, "#FFD700");
        assert_eq!(sm.lines[1].lang.as_deref(), Some("zh"));
        assert_eq!(sm.lines[0].line_type, "original");
        // D-17（2026-09-07 用户裁决）：字体键默认从原版 "Microsoft YaHei"
        // 改为空串 = 跟随 subtitle_font_family（主键默认内嵌思源）
        assert_eq!(st.original_font_family, "");
        assert_eq!(st.translation_font_family, "");
        assert!(sm.lines.iter().all(|l| l.font_family.is_empty()));
    }

    /// D-17：两把主字体键默认内嵌思源；行级默认空串（跟随）
    #[test]
    fn default_font_keys_follow_embedded_source_han() {
        let s = Settings::default();
        assert_eq!(s.ui_font_family, "Noto Sans CJK SC");
        assert_eq!(s.subtitle_font_family, "Noto Sans CJK SC");
        assert_eq!(Style::default().original_font_family, "");
        assert_eq!(Style::default().translation_font_family, "");
        assert_eq!(SubtitleLine::default().font_family, "");
        // 默认两行（原文/译文）同样为跟随
        assert!(SubtitleLine::default_pair()
            .iter()
            .all(|l| l.font_family.is_empty()));
        // 序列化携带两把新键
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["ui_font_family"], "Noto Sans CJK SC");
        assert_eq!(v["subtitle_font_family"], "Noto Sans CJK SC");
    }

    /// 旧文件三级语义：缺键 → 默认；显式 "Microsoft YaHei"（原版导入）→ 尊重
    #[test]
    fn legacy_yahei_values_respected() {
        let s = Settings::from_value_compatible(serde_json::json!({
            "style": { "original_font_family": "Microsoft YaHei" },
            "subtitle_mode": { "lines": [ { "font_family": "Microsoft YaHei" } ] }
        }));
        assert_eq!(s.style.original_font_family, "Microsoft YaHei");
        assert_eq!(s.subtitle_mode.lines[0].font_family, "Microsoft YaHei");
        // 缺新键走默认
        assert_eq!(s.ui_font_family, "Noto Sans CJK SC");
        assert_eq!(s.subtitle_font_family, "Noto Sans CJK SC");
    }

    /// 字体主键空串（手改/损坏 JSON）→ sanitize 回默认
    #[test]
    fn font_family_empty_sanitized_to_default() {
        // 绕过 from_value_compatible 的预 sanitize，直接反序列化后手动 sanitize
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "ui_font_family": "", "subtitle_font_family": ""
        }))
        .unwrap();
        let fixed = s.sanitize();
        assert_eq!(s.ui_font_family, "Noto Sans CJK SC");
        assert_eq!(s.subtitle_font_family, "Noto Sans CJK SC");
        assert!(fixed.iter().any(|f| f.starts_with("ui_font_family")));
        assert!(fixed.iter().any(|f| f.starts_with("subtitle_font_family")));
        // 行级空串是合法跟随语义，sanitize 不得改动
        let mut s2: Settings = serde_json::from_value(serde_json::json!({
            "style": { "translation_font_family": "" }
        }))
        .unwrap();
        let fixed2 = s2.sanitize();
        assert_eq!(s2.style.translation_font_family, "");
        assert!(!fixed2.iter().any(|f| f.contains("translation_font_family")));
    }

    /// D-74：语言码归一（同语言免翻译比较的目标语言侧）
    #[test]
    fn normalize_language_codes() {
        assert_eq!(normalize_language("zh-CN"), "zh");
        assert_eq!(normalize_language("ZH-cn"), "zh");
        assert_eq!(normalize_language("zh"), "zh");
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("yue"), "yue");
        assert_eq!(normalize_language(" auto "), "auto");
        assert_eq!(normalize_language(""), "");
        assert_eq!(normalize_language("-"), "");
    }
}
