//! 厂商预设（D-85 决策 J/K，docs/translator-probe-hotswap.md §十）。
//!
//! 数据来源：2026-09-10 逐家核对官方文档（API 参考 / 定价页），URL 见
//! `docs/translator-probe-hotswap.md` §十 表格。三条硬约束：
//! 1. `api_base` 一律取**官方形态**（DeepSeek 官方就不带 `/v1`，照抄；
//!    `config_warnings` 对预设地址豁免"缺 /v1"软提示，见 lt-ui）；
//! 2. `thinking_style` 取**官方能接受且我们已支持**的关闭姿态——DeepSeek/GLM/
//!    Kimi/方舟用嵌套 `thinking:{type:"disabled"}`（= 本仓 `"deepseek"`）、
//!    通义用 `enable_thinking:false`（= `"qwen"`）、本机端点用
//!    `reasoning_effort:"none"`（= `"openai"`）、OpenAI 保守用 `"off"`（不发）；
//! 3. **价格一律留空**（0）：各站价格随时变，且国内站人民币、国际站美元——
//!    写死数字等于替用户记错账，币种由 `currency` 字段声明。

use crate::settings::ModelConfig;

/// 一条厂商预设：选中后填入模型编辑对话框（**只填这 5 个字段**）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    /// 稳定标识，同时是 i18n 键后缀（`preset_<key>`）
    pub key: &'static str,
    /// OpenAI 兼容端点（官方文档形态）
    pub api_base: &'static str,
    /// 推荐模型 id（用户可改；本机端点为空 = 必须用户自填）
    pub model: &'static str,
    /// 关闭思维链姿态（`THINKING_STYLES` 之一）
    pub thinking_style: &'static str,
    /// 是否尝试关闭思维链（预设一律 true；"关不掉"由运行期标记接管）
    pub disable_thinking: bool,
    /// 计价币种（`None` = 跟随界面语言）
    pub currency: Option<&'static str>,
}

impl ProviderPreset {
    /// 特殊项：不填充任何字段（纯手工，与旧行为一致）
    pub fn is_custom(&self) -> bool {
        self.key == "custom"
    }

    /// 是否把"显示名"也填上——**仅当当前为空**（新增场景）；
    /// 用户改过名就保留（预设不得覆盖用户数据）
    pub fn name_for(&self, current_name: &str) -> Option<&'static str> {
        if self.is_custom() || !current_name.trim().is_empty() {
            return None;
        }
        Some(self.display_name())
    }

    /// 预设自带的显示名（与 `key` 同名品牌；i18n 里 `preset_<key>` 是界面标签）
    pub fn display_name(&self) -> &'static str {
        match self.key {
            "deepseek" => "DeepSeek",
            "openai" => "OpenAI",
            "zhipu" => "GLM",
            "moonshot" => "Kimi",
            "qwen" => "Qwen",
            "ark" => "Doubao",
            "lmstudio" => "LM Studio",
            "ollama" => "Ollama",
            other => other,
        }
    }
}

/// 预设全集（界面下拉顺序 = 本数组顺序；末位 `custom` = 不填充）
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        key: "deepseek",
        // 官方形态**不带 /v1**（curl 端点为 https://api.deepseek.com/chat/completions）
        api_base: "https://api.deepseek.com",
        model: "deepseek-flash",
        thinking_style: "deepseek",
        disable_thinking: true,
        currency: Some("cny"),
    },
    ProviderPreset {
        key: "openai",
        api_base: "https://api.openai.com/v1",
        model: "gpt-5.6-luna",
        // `reasoning_effort:"none"` 在 OpenAI 是 model-dependent（部分模型传了 400），
        // 保守用 "off"（不发任何关闭参数）；用户可在「关闭方式」里显式选 openai
        thinking_style: "off",
        disable_thinking: true,
        currency: Some("usd"),
    },
    ProviderPreset {
        key: "zhipu",
        api_base: "https://open.bigmodel.cn/api/paas/v4",
        model: "glm-4.6",
        thinking_style: "deepseek",
        disable_thinking: true,
        currency: Some("cny"),
    },
    ProviderPreset {
        key: "moonshot",
        api_base: "https://api.moonshot.cn/v1",
        model: "kimi-k2.6",
        thinking_style: "deepseek",
        disable_thinking: true,
        currency: Some("cny"),
    },
    ProviderPreset {
        key: "qwen",
        api_base: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        model: "qwen3.5-flash",
        thinking_style: "qwen",
        disable_thinking: true,
        currency: Some("cny"),
    },
    ProviderPreset {
        key: "ark",
        api_base: "https://ark.cn-beijing.volces.com/api/v3",
        model: "doubao-seed-2-0-lite-260215",
        thinking_style: "deepseek",
        disable_thinking: true,
        currency: Some("cny"),
    },
    ProviderPreset {
        key: "lmstudio",
        api_base: "http://localhost:1234/v1",
        // 本机端点不知道用户装了哪个模型 → 留空，由对话框既有的非空守卫拦住
        model: "",
        thinking_style: "openai",
        disable_thinking: true,
        currency: None,
    },
    ProviderPreset {
        key: "ollama",
        api_base: "http://localhost:11434/v1",
        model: "",
        thinking_style: "openai",
        disable_thinking: true,
        currency: None,
    },
    ProviderPreset {
        key: "custom",
        api_base: "",
        model: "",
        thinking_style: "",
        disable_thinking: true,
        currency: None,
    },
];

/// 按 key 取预设
pub fn preset_by_key(key: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS.iter().find(|p| p.key == key)
}

/// 出厂默认模型配置（D-85 决策 K：由"本机 LM Studio"改为 DeepSeek）。
///
/// 依据：默认那条对本机没装 LM Studio 的用户永远不通，且报错（连接被拒）不如
/// "缺 API Key"可执行；DeepSeek 国内直连、便宜、OpenAI 兼容，官方
/// `thinking:{type:"disabled"}` 不会 400。`api_key` 留空由用户首启粘贴。
pub fn default_model_config() -> ModelConfig {
    let p = preset_by_key("deepseek").expect("DeepSeek 预设必须在表内");
    ModelConfig {
        name: p.display_name().to_string(),
        api_base: p.api_base.to_string(),
        api_key: String::new(),
        model: p.model.to_string(),
        thinking_style: Some(p.thinking_style.to_string()),
        disable_thinking: p.disable_thinking,
        currency: p.currency.map(str::to_string),
        ..ModelConfig::default()
    }
}

    // ── D-85/J：厂商预设 ──

    /// 预设表自洽：key 唯一、字段非空（custom 除外）、关闭姿态在值域内、
    /// 币种字面量可解析
    #[test]
    fn preset_table_is_well_formed() {
        let mut keys: Vec<&str> = crate::PROVIDER_PRESETS.iter().map(|p| p.key).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "预设 key 不得重复");
        assert_eq!(
            crate::PROVIDER_PRESETS.last().map(|p| p.key),
            Some("custom"),
            "custom 必须是末位（下拉顺序）"
        );
        for p in crate::PROVIDER_PRESETS {
            if p.is_custom() {
                continue;
            }
            assert!(!p.api_base.is_empty(), "{} 缺 api_base", p.key);
            assert!(
                crate::THINKING_STYLES.contains(&p.thinking_style),
                "{} 的关闭姿态 {:?} 不在值域内",
                p.key,
                p.thinking_style
            );
            if let Some(c) = p.currency {
                assert!(
                    crate::Currency::parse(c).is_some(),
                    "{} 的币种字面量 {c:?} 不可解析",
                    p.key
                );
            }
        }
        // 官方形态核对（防手滑改错地址）
        assert_eq!(
            crate::preset_by_key("deepseek").unwrap().api_base,
            "https://api.deepseek.com"
        );
        assert_eq!(
            crate::preset_by_key("openai").unwrap().api_base,
            "https://api.openai.com/v1"
        );
    }

    /// 出厂默认首个模型 = DeepSeek 预设（D-85/K），且零值构造仍是中立空白
    #[test]
    fn default_model_is_deepseek_preset() {
        let d = crate::Settings::default();
        let m = &d.models[0];
        let p = crate::preset_by_key("deepseek").unwrap();
        assert_eq!(m.api_base, p.api_base);
        assert_eq!(m.model, p.model);
        assert_eq!(m.thinking_style.as_deref(), Some(p.thinking_style));
        assert_eq!(m.currency.as_deref(), p.currency);
        assert!(m.api_key.is_empty(), "密钥留空由用户首启粘贴");
        assert_eq!(m.name, "DeepSeek");
        // **零值构造保持中立**：serde 容器默认会用它补缺失字段——一旦这里变成
        // 厂商预设，老档案缺 thinking_style 的模型会被注入 "deepseek"（一发 400）
        let blank = crate::ModelConfig::default();
        assert_eq!(blank.thinking_style, None, "零值构造不得带厂商偏好");
        assert_eq!(blank.currency, None);
        assert_eq!(blank.api_base, "http://127.0.0.1:1234/v1");
    }
