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
/// 合法 funasr 模型（D-86：mlt 值域成员移除——退役键经 sanitize 回退 sensevoice-small）
pub const FUNASR_MODELS: [&str; 2] = ["sensevoice-small", "funasr-nano-2512"];
/// 合法 whisper 档位（r8：+turbo）
pub const WHISPER_SIZES: [&str; 6] = ["tiny", "base", "small", "medium", "large-v3", "turbo"];
/// 旧版独立引擎名 → funasr 模型（对照原版 FUNASR_LEGACY_ENGINE_ALIASES；
/// 引擎别名命中时必须覆盖 funasr_model，即使其已是合法值）
/// D-86 注记：旧档 funasr-mlt-nano 经"非法引擎→funasr"+"模型归一→sensevoice-small"双重收敛
pub const ASR_ENGINE_LEGACY_ALIASES: [(&str, &str); 2] = [
    ("sensevoice", "sensevoice-small"),
    ("funasr-nano", "funasr-nano-2512"),
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
        "sensevoice-small" | "funasr-nano-2512" => v.into(),
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
            // D-85 决策 K：新装用户的首个模型 = DeepSeek 预设（老档案不受影响——
            // 他们的 models 非空，不信这条）
            models: vec![crate::presets::default_model_config()],
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
        // legacy：models[].no_think 布尔 → 新总开关 `disable_thinking`
        // （原版语义：no_think=true = 要关思考 → 开关开；false = 不发送关闭参数 → 开关关）。
        // W1/方案 §2.3 起不再合成 thinking_style——该字段现仅表示"用哪种方式关闭"。
        if let Some(models) = root.get_mut("models").and_then(|m| m.as_array_mut()) {
            for m in models {
                let has_switch = m
                    .get("disable_thinking")
                    .map(|v| !v.is_null())
                    .unwrap_or(false);
                if !has_switch {
                    if let Some(no_think) = m.get("no_think").and_then(|b| b.as_bool()) {
                        m["disable_thinking"] = Value::Bool(no_think);
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
            self.models.push(crate::presets::default_model_config());
            fixed.push("models: 空 → 填充默认厂商预设".into());
        }
        // legacy 显示名迁移（原版悬浮窗/托盘显示 name；旧占位 name="default"
        // 时代码以 model 字段为真名）→ name=="default" 且 model 有值时以 model 为准
        for m in &mut self.models {
            if m.name == "default" && !m.model.is_empty() && m.model != "default" {
                m.name = m.model.clone();
                fixed.push(format!("model name: 'default' → '{}'", m.name));
            }
        }
        // W1/方案 §2.3：旧 `thinking_style == "off"` 的语义是"不发送关闭参数"
        // （与字面相反，曾致用户设错方向）→ 归一化到新总开关，此后值域内不再出现
        for m in &mut self.models {
            if m.thinking_style.as_deref() == Some("off") {
                m.thinking_style = None;
                m.disable_thinking = false;
                fixed.push("thinking_style: 'off' → disable_thinking=false".into());
            }
        }
        // 2026-09-10 第二轮评审 ②：界面已撤下的覆写键从档案里**清除**——
        // 界面看不见的键绝不允许影响实际请求（温度已升为一等字段、输出上限应用
        // 不再发送——旧值留着会反杀"不再发送 max_tokens"的修复、seed 冷门）
        // json_response 已撤出界面却仍会改写请求（response_format + 提示词追加）
        // ——与上面三个覆写键同理，界面看不见的东西不许影响实发请求
        for m in &mut self.models {
            if m.json_response {
                m.json_response = false;
                fixed.push("json_response: 已撤出界面 → 关闭（不再改写请求）".into());
            }
        }
        for m in &mut self.models {
            if let Some(ov) = m.overrides.as_mut() {
                for k in OVERRIDE_KEYS_HIDDEN {
                    if ov.remove(k).is_some() {
                        fixed.push(format!("overrides: 清除已撤出界面的键 '{k}'"));
                    }
                }
                if ov.is_empty() {
                    m.overrides = None;
                }
            }
        }
        if self.active_model >= self.models.len() {
            fixed.push(format!("active_model: {} 越界 → 0", self.active_model));
            self.active_model = 0;
        }
        fixed
    }
}

/// thinking_style 合法值域（E3/ADR-10：自 lt-translate 上移——本清单是
/// [`ModelConfig.thinking_style`] 字段的值域，与 ASR_ENGINES 同类归 settings；
/// lt-translate 的解析逻辑与 lt-ui 的下拉项两侧同源引用）。
///
/// W1/方案 §2.3：`"off"` 仅为**解析旧档案**保留——[`Settings::sanitize`] 会把它
/// 归一化为 `disable_thinking = false`（语义即旧 `off`：不发送关闭参数），此后
/// 值域内不再出现；界面只呈现前五项（自动 + 四种方式）。
pub const THINKING_STYLES: [&str; 6] = ["auto", "deepseek", "qwen", "vllm", "openai", "off"];

/// 温度控件的预填值（2026-09-10 裁决：高级参数**默认不发送**——温度字段默认
/// `None`；用户界面上勾选启用时的预填就是这个值）
pub const DEFAULT_TEMPERATURE: f64 = 0.3;

/// 可覆写的采样参数键（E3/ADR-10：自 lt-translate 上移——本清单是
/// [`ModelConfig.overrides`] 的**历史键域**，用于解析旧档案）。
///
/// 2026-09-10 第二轮评审 ②：前三个键已撤出界面（温度升为一等字段、输出上限
/// 应用不再发送、seed 冷门）——[`Settings::sanitize`] 会从档案里清除它们，
/// lt-translate 构造期还有第二道闸；界面只渲染后三个。
pub const OVERRIDE_KEYS: [&str; 6] = [
    "temperature",
    "top_p",
    "max_tokens",
    "frequency_penalty",
    "presence_penalty",
    "seed",
];

/// 已撤出界面、**不再参与请求**的覆写键（与 lt-translate 的构造期闸门同源）
pub const OVERRIDE_KEYS_HIDDEN: [&str; 3] = ["temperature", "max_tokens", "seed"];

/// 界面仍在渲染的覆写键（高级区三行）
pub const OVERRIDE_KEYS_VISIBLE: [&str; 3] = ["top_p", "frequency_penalty", "presence_penalty"];

/// 费用计价币种（D-85）：显示符号与价格单位都由它决定——**不再按界面语言切换**
/// （旧实现按 `get_lang()` 选 ¥/$，用户填美元价时中文界面会标成人民币，差 ~7 倍）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// 人民币（元）
    Cny,
    /// 美元
    Usd,
}

impl Currency {
    /// 配置字面量 → 币种（认不出的值按 `None` 处理 = 跟随界面语言）
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cny" | "rmb" | "yuan" => Some(Currency::Cny),
            "usd" | "dollar" => Some(Currency::Usd),
            _ => None,
        }
    }

    /// 写回配置的字面量
    pub fn as_str(self) -> &'static str {
        match self {
            Currency::Cny => "cny",
            Currency::Usd => "usd",
        }
    }

    /// 显示符号
    pub fn symbol(self) -> &'static str {
        match self {
            Currency::Cny => "¥",
            Currency::Usd => "$",
        }
    }

    /// 价格单位文案的 i18n 键（"元 / 1M tok" vs "美元 / 1M tok"）
    pub fn unit_key(self) -> &'static str {
        match self {
            Currency::Cny => "price_unit_cny",
            Currency::Usd => "price_unit_usd",
        }
    }
}

/// 生效币种（D-85 唯一判据）：配置显式值优先，否则按界面语言
/// （中文 → 人民币，其余 → 美元）。UI 与编排域共用同一函数，避免两处各判一次。
pub fn effective_currency(cfg_currency: Option<&str>, ui_lang: &str) -> Currency {
    cfg_currency
        .and_then(Currency::parse)
        .unwrap_or(if ui_lang.starts_with("zh") {
            Currency::Cny
        } else {
            Currency::Usd
        })
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
    /// 关闭方式（W1/方案 §2.3）：None/"auto" = 自动首选；deepseek/qwen/vllm/openai
    /// = 只用该一种。仅在 [`ModelConfig::disable_thinking`] 为 true 时参与请求。
    /// 仅非 "auto" 时写盘（None = auto）
    #[serde(skip_serializing_if = "skip_thinking_auto")]
    pub thinking_style: Option<String>,
    /// 关闭模型思考总开关（W1/方案 §2.3，默认 true）：true = 按 `thinking_style`
    /// 发送关闭推理的参数；false = 不发送任何推理相关参数（即旧 `"off"` 的语义）。
    /// 与 `streaming` 同款：默认值不写盘，仅 false 时写。
    #[serde(skip_serializing_if = "is_true")]
    pub disable_thinking: bool,
    /// 该模型已确认**无法关闭思维链**（2026-09-10 第二轮评审 item 5，默认 false，
    /// 仅 true 时写盘）：运行期回退阶梯试完全部关闭形态仍关不掉时由界面写入。
    /// true → 不再尝试注入关闭参数（翻译照常，思考由隔离器兜底不外泄）。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub thinking_unavailable: bool,
    /// 采样温度（2026-09-10 裁决）：**默认 None = 不发送**；高级参数默认一律不
    /// 发送，只有用户在界面上手动指定才发（None 不写盘）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// 默认 true；仅 false 时写盘（键名对齐原版 `streaming: false`）
    #[serde(skip_serializing_if = "is_true")]
    pub streaming: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub json_response: bool,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub context_turns: u32,
    /// 计价币种（"cny" | "usd"）；None = 跟随界面语言（中文→cny，英文→usd）。
    /// D-85：国内外站点官方定价币种不同，由用户按自己看到的定价页填；
    /// 会话费用累计据此分币种两个账本（跨币种不做汇率折算）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    /// 每 1M tokens 的输入价（单位由 [`ModelConfig::currency`] 决定）
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub input_price: f64,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub output_price: f64,
    /// 仅勾选的键（原版 Advanced "checkbox+value" 行；界面只剩 top_p 与两个惩罚项）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, Value>>,
    /// 用户自定义 extra_body（JSON object）
    #[serde(skip_serializing_if = "skip_empty_extra_body")]
    pub extra_body: Option<Value>,
}

impl Default for ModelConfig {
    /// **零值构造**（不含任何厂商偏好）。
    ///
    /// 为什么不是 DeepSeek 预设：本类型带容器级 `#[serde(default)]`——
    /// 反序列化时**缺失字段**会用本函数补齐。若这里返回厂商预设，老档案里
    /// 没写 `thinking_style` 的模型会被悄悄注入 `"deepseek"`（对本来不接受
    /// 该参数的端点就是一发 400）。厂商默认值放 [`Settings::default`]（见
    /// [`crate::presets::default_model_config`]），只影响"新装用户的首个模型"。
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
            disable_thinking: true,
            thinking_unavailable: false,
            temperature: None,
            streaming: true,
            json_response: false,
            context_turns: 0,
            currency: None,
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
        assert_eq!(EngineKey::from_settings_str("qwen3"), EngineKey::Qwen3);
        // 未知值（含旧版残留/手改）回退 FunAsr 而非 panic——与 sanitize 同语义
        assert_eq!(
            EngineKey::from_settings_str("sensevoice"),
            EngineKey::FunAsr
        );
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
            assert_eq!(
                p.to_settings_str(),
                if src.is_empty() { "system" } else { src }
            );
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
                                       // W1/方案 §2.3：no_think=false → 总开关 disable_thinking=false
                                       //（语义 = 不发送关闭参数）；thinking_style 不再被合成
        assert!(!s.models[0].disable_thinking);
        assert_eq!(s.models[0].thinking_style, None);
        // 序列化后不再出现 no_think；disable_thinking=false 必须写盘（非默认值）
        let out = serde_json::to_value(&s).unwrap();
        assert!(out["models"][0].get("no_think").is_none());
        assert_eq!(
            out["models"][0]["disable_thinking"],
            serde_json::json!(false)
        );
    }

    /// W1/方案 §2.3：旧 `thinking_style == "off"`（语义与字面相反）在 sanitize 中
    /// 归一化为 `disable_thinking = false`，此后值域内不再出现 "off"
    #[test]
    fn legacy_thinking_off_normalizes_to_switch() {
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "models": [{"name": "a", "model": "m", "thinking_style": "off"}]
        }))
        .unwrap();
        // 反序列化后仍是旧值（归一化发生在 sanitize）
        assert_eq!(s.models[0].thinking_style.as_deref(), Some("off"));
        let fixed = s.sanitize();
        assert!(s.models[0].thinking_style.is_none());
        assert!(!s.models[0].disable_thinking);
        assert!(fixed.iter().any(|f| f.contains("disable_thinking")));
        // 写回不再出现 "off"
        let out = serde_json::to_value(&s).unwrap();
        assert!(out["models"][0].get("thinking_style").is_none());
    }

    /// 2026-09-10 第二轮评审 ②：界面已撤下的覆写键（温度/输出上限/seed）在
    /// sanitize 中被**清除**——旧值留着会反杀"不再发送 max_tokens"的修复
    #[test]
    fn hidden_override_keys_are_migrated_out() {
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "models": [{
                "name": "a", "model": "m",
                "overrides": {"temperature": 0.7, "max_tokens": 256, "seed": 42, "top_p": 0.9}
            }]
        }))
        .unwrap();
        let fixed = s.sanitize();
        let ov = s.models[0].overrides.as_ref().expect("top_p 应保留");
        assert!(!ov.contains_key("temperature"), "隐藏温度应被清除");
        assert!(!ov.contains_key("max_tokens"), "隐藏上限应被清除");
        assert!(!ov.contains_key("seed"), "隐藏 seed 应被清除");
        assert_eq!(ov.get("top_p"), Some(&serde_json::json!(0.9)));
        // 清除留有痕迹（供调用方记日志）
        assert!(fixed.iter().any(|f| f.contains("max_tokens")));
        // 全部被清空时整个 overrides 折叠为 None（写回档案不再出现空对象）
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "models": [{"name": "a", "model": "m", "overrides": {"max_tokens": 256}}]
        }))
        .unwrap();
        s.sanitize();
        assert!(s.models[0].overrides.is_none());
    }

    /// W1：缺键的旧档案 → 新字段走默认（开关开 = 关思考；温度 0.3）
    #[test]
    fn missing_new_keys_use_defaults() {
        let mut s: Settings = serde_json::from_value(serde_json::json!({
            "models": [{"name": "a", "model": "m"}]
        }))
        .unwrap();
        assert!(s.models[0].disable_thinking);
        assert_eq!(s.models[0].temperature, None, "高级参数默认不发送");
        assert!(!s.models[0].thinking_unavailable);
        s.sanitize();
        assert!(s.models[0].disable_thinking);
    }

    /// W1/方案 §2.5 规则 8：ModelConfig 每个序列化键必须登记在册并注明消费方
    /// ——新增字段却忘了接线（无任何消费者）会被本测试挡下。
    /// 消费方三类：`请求构造` / `UI` / `本地展示`。
    #[test]
    fn every_model_config_field_is_registered() {
        // 全字段非默认值 → 无键被 skip 掉
        let full = ModelConfig {
            name: "n".into(),
            api_base: "b".into(),
            api_key: "k".into(),
            model: "m".into(),
            proxy: "system".into(),
            no_system_role: true,
            thinking_style: Some("qwen".into()),
            disable_thinking: false,
            thinking_unavailable: true,
            temperature: Some(0.7),
            streaming: false,
            json_response: true,
            context_turns: 3,
            input_price: 1.0,
            currency: None,
            output_price: 2.0,
            overrides: Some(BTreeMap::from([(
                "top_p".to_string(),
                serde_json::json!(0.9),
            )])),
            extra_body: Some(serde_json::json!({"k": 1})),
        };
        let mut keys: Vec<String> = serde_json::to_value(&full)
            .unwrap()
            .as_object()
            .expect("ModelConfig 序列化为对象")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        // (键, 消费方)
        let mut registry: Vec<String> = [
            ("name", "UI（列表/悬浮窗/托盘显示名）"),
            ("api_base", "请求构造（client 端点）"),
            ("api_key", "请求构造（鉴权）"),
            ("model", "请求构造（model 字段）"),
            ("proxy", "请求构造（client 代理）"),
            ("no_system_role", "请求构造（messages 组装）"),
            ("thinking_style", "请求构造（关闭方式，§2.3 规则 3）"),
            ("disable_thinking", "请求构造（关闭总开关，§2.3 规则 1/2）"),
            (
                "thinking_unavailable",
                "请求构造（已确认关不掉：不再注入关闭参数）/ UI 提示",
            ),
            ("temperature", "请求构造（采样温度，None=不发——默认即不发）"),
            ("streaming", "请求构造（stream 开关）"),
            (
                "json_response",
                "请求构造（response_format；W1 起界面不再暴露）",
            ),
            ("context_turns", "请求构造（上下文历史条数）"),
            ("input_price", "本地展示（成本估算）"),
            ("output_price", "本地展示（成本估算）"),
            ("overrides", "请求构造（采样覆写逃生舱）"),
            ("extra_body", "请求构造（逃生舱透传）"),
        ]
        .iter()
        .map(|(k, _)| (*k).to_string())
        .collect();
        registry.sort();
        assert_eq!(
            keys, registry,
            "ModelConfig 字段集与登记清单不一致：新增/删除字段时必须同步本清单并注明消费方"
        );
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

        // D-86：funasr-mlt-nano 别名已删——旧档经"非法引擎→funasr"+"模型归一"收敛
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr-mlt-nano"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "sensevoice-small");
    }

    /// D-86：退役键两条收敛路径的端到端锁（mlt 从值域删除后旧档不炸、不落死值）
    #[test]
    fn retired_mlt_values_converge() {
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr",
            "funasr_model": "funasr-mlt-nano-2512"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "sensevoice-small");
    }

    /// 正常引擎值不触发别名迁移，funasr_model 保持原样
    #[test]
    fn normal_engine_values_untouched() {
        let s = Settings::from_value_compatible(serde_json::json!({
            "asr_engine": "funasr",
            "funasr_model": "funasr-nano-2512"
        }));
        assert_eq!(s.asr_engine, "funasr");
        assert_eq!(s.funasr_model, "funasr-nano-2512");

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
            "disable_thinking", // W1：默认 true（关思考）不写盘
            "temperature",      // 默认不发送（None）不写盘
            "thinking_unavailable",
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
        // W1：非默认值必须写盘——temperature=null（=不发送）/ 0.7 / disable_thinking=false
        let m = ModelConfig {
            temperature: None,
            ..Default::default()
        };
        assert!(serde_json::to_value(m).unwrap()["temperature"].is_null());
        let m = ModelConfig {
            temperature: Some(0.7),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(m).unwrap()["temperature"],
            serde_json::json!(0.7)
        );
        let m = ModelConfig {
            disable_thinking: false,
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(m).unwrap()["disable_thinking"],
            serde_json::json!(false)
        );
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

    // ── D-85/H：币种判据 ──

    /// 生效币种：显式优先，其次按界面语言（中文→人民币，其余→美元）
    #[test]
    fn effective_currency_follows_lang_then_explicit() {
        use super::{effective_currency, Currency};
        assert_eq!(effective_currency(None, "zh"), Currency::Cny);
        assert_eq!(effective_currency(None, "zh-CN"), Currency::Cny);
        assert_eq!(effective_currency(None, "en"), Currency::Usd);
        assert_eq!(effective_currency(Some("usd"), "zh"), Currency::Usd);
        assert_eq!(effective_currency(Some("cny"), "en"), Currency::Cny);
        // 认不出的值 → 回落语言默认（不 panic、不硬失败）
        assert_eq!(effective_currency(Some("bogus"), "zh"), Currency::Cny);
        assert_eq!(effective_currency(Some(""), "en"), Currency::Usd);
        // 符号与单位键
        assert_eq!(Currency::Cny.symbol(), "¥");
        assert_eq!(Currency::Usd.symbol(), "$");
        assert_eq!(Currency::Cny.as_str(), "cny");
        assert_eq!(Currency::Usd.unit_key(), "price_unit_usd");
    }

    /// 老档案（无 currency 键）反序列化后为 None——跟随界面语言，不产生迁移
    #[test]
    fn settings_without_currency_key_loads_as_none() {
        let json =
            r#"{"models":[{"name":"m","api_base":"http://x/v1","api_key":"k","model":"d"}]}"#;
        let s: crate::Settings = serde_json::from_str(json).expect("老档案可解");
        assert_eq!(s.models[0].currency, None);
        // 空 currency 不写盘（保持档案干净）
        let out = serde_json::to_string(&s).expect("可序列化");
        assert!(!out.contains("currency"), "None 不该出现在落盘 JSON：{out}");
        // 显式币种要写盘
        let mut s2 = s.clone();
        s2.models[0].currency = Some("usd".into());
        let out2 = serde_json::to_string(&s2).expect("可序列化");
        assert!(out2.contains("\"currency\":\"usd\""), "{out2}");
    }
}
