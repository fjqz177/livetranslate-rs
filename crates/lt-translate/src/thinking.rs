//! 思维链（thinking/reasoning）关闭策略（对照原版 translator.py）。
//!
//! 各提供商关闭 thinking 的请求体形状不同；thinking 若未被关闭，
//! 整个 max_tokens 预算会被推理消耗、completion 返回空（原版 issue #38），
//! UI 会渲染成未翻译的同语言文本。
//! - deepseek: DeepSeek API、火山方舟、智谱 GLM（嵌套 thinking 对象）
//! - qwen:     DashScope/百炼、SiliconFlow（扁平 enable_thinking）
//! - vllm:     自托管 vLLM/SGLang（chat template kwarg）
//! - openai:   OpenAI GPT-5.1+/Grok 4.3+（reasoning_effort=none）
//! - off:      什么都不发（非 thinking 模型、LM Studio、Ollama /v1）

use serde_json::{json, Value};

pub const THINKING_STYLES: [&str; 6] = ["auto", "deepseek", "qwen", "vllm", "openai", "off"];

const NESTED_THINKING_MODELS: [&str; 2] = ["deepseek", "glm"];
const NESTED_THINKING_ENDPOINTS: [&str; 4] = ["deepseek", "volces", "api.z.ai", "bigmodel"];
const PARAMLESS_ENDPOINTS: [&str; 3] = ["api.openai.com", "api.x.ai", "api.anthropic.com"];

/// 将 thinking_style 设置解析为具体提供商风格。
///
/// "auto" 按端点/模型 id 猜测；官方 OpenAI 系端点得 "off"，
/// 因为它们拒绝未知请求参数。
pub fn resolve_thinking_style(style: &str, api_base: &str, model: &str) -> &'static str {
    if THINKING_STYLES.contains(&style) && style != "auto" {
        // 返回静态表内的同一字面量，保证 &'static 生命周期
        return THINKING_STYLES[THINKING_STYLES.iter().position(|s| *s == style).unwrap()];
    }
    let endpoint = api_base.to_lowercase();
    let model_id = model.to_lowercase();
    if NESTED_THINKING_MODELS.iter().any(|m| model_id.contains(m))
        || NESTED_THINKING_ENDPOINTS
            .iter()
            .any(|m| endpoint.contains(m))
    {
        return "deepseek";
    }
    if PARAMLESS_ENDPOINTS.iter().any(|m| endpoint.contains(m)) {
        return "off";
    }
    "qwen"
}

/// 具体风格对应的"关闭 thinking"请求体片段。
/// 每次调用返回新 Value（无共享可变状态）。
pub fn thinking_disable_body(style: &str) -> Value {
    match style {
        "deepseek" => json!({"thinking": {"type": "disabled"}}),
        "qwen" => json!({"enable_thinking": false}),
        "vllm" => json!({"chat_template_kwargs": {"enable_thinking": false}}),
        "openai" => json!({"reasoning_effort": "none"}),
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translator::Translator;

    /// 对照原版 test_translator_thinking.py：经 Translator.merged_extra_body 观察
    /// extra_body（含 Null=不发）。
    fn extra_body(
        api_base: &str,
        model: &str,
        thinking_style: Option<&str>,
        no_think: bool,
        user_extra: Option<Value>,
    ) -> Value {
        let t = Translator::for_test(api_base, model, thinking_style, no_think, user_extra);
        t.merged_extra_body()
    }

    // ── auto 风格检测（对照原版用例逐条） ──

    #[test]
    fn deepseek_model_uses_nested_thinking_toggle() {
        assert_eq!(
            extra_body(
                "https://example.com/v1",
                "deepseek-v4-pro",
                None,
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn official_deepseek_endpoint_supports_model_aliases() {
        assert_eq!(
            extra_body(
                "https://api.deepseek.com/v1",
                "production-alias",
                None,
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn deepseek_proxy_endpoint_uses_nested_thinking_toggle() {
        assert_eq!(
            extra_body(
                "https://deepseek.gateway.example.com/v1",
                "production-alias",
                None,
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn volcano_ark_endpoint_uses_nested_thinking_toggle() {
        assert_eq!(
            extra_body(
                "https://ark.cn-beijing.volces.com/api/v3",
                "ep-2026-alias",
                None,
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn glm_model_uses_nested_thinking_toggle() {
        assert_eq!(
            extra_body("https://example.com/v1", "glm-4.6", None, true, None),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn non_deepseek_models_keep_legacy_toggle() {
        assert_eq!(
            extra_body("https://example.com/v1", "qwen3", None, true, None),
            json!({"enable_thinking": false})
        );
    }

    #[test]
    fn official_openai_endpoint_sends_no_thinking_param() {
        assert!(extra_body("https://api.openai.com/v1", "gpt-5.1", None, true, None).is_null());
    }

    // ── 显式风格 ──

    #[test]
    fn explicit_vllm_style_uses_chat_template_kwargs() {
        assert_eq!(
            extra_body(
                "https://example.com/v1",
                "deepseek-r1-distill",
                Some("vllm"),
                true,
                None
            ),
            json!({"chat_template_kwargs": {"enable_thinking": false}})
        );
    }

    #[test]
    fn explicit_openai_style_uses_reasoning_effort() {
        assert_eq!(
            extra_body(
                "https://example.com/v1",
                "grok-4.3",
                Some("openai"),
                true,
                None
            ),
            json!({"reasoning_effort": "none"})
        );
    }

    #[test]
    fn explicit_off_style_sends_nothing() {
        assert!(extra_body(
            "https://example.com/v1",
            "deepseek-v4",
            Some("off"),
            true,
            None
        )
        .is_null());
    }

    #[test]
    fn explicit_deepseek_style_overrides_detection() {
        assert_eq!(
            extra_body(
                "https://example.com/v1",
                "ep-custom",
                Some("deepseek"),
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    // ── legacy no_think 迁移 ──

    #[test]
    fn legacy_no_think_false_sends_nothing() {
        assert!(extra_body("https://api.deepseek.com", "deepseek-v4", None, false, None).is_null());
    }

    // ── 与显式 extra_body 的交互 ──

    #[test]
    fn explicit_extra_body_overrides_automatic_toggle() {
        assert_eq!(
            extra_body(
                "https://api.deepseek.com",
                "deepseek-v4-pro",
                None,
                true,
                Some(json!({"thinking": {"type": "enabled"}, "user_id": "test"})),
            ),
            json!({"thinking": {"type": "enabled"}, "user_id": "test"})
        );
    }

    #[test]
    fn target_language_clone_preserves_thinking_control() {
        let t = Translator::for_test(
            "https://api.deepseek.com",
            "deepseek-v4-pro",
            None,
            true,
            None,
        );
        let clone = t.with_target_language("ja");
        assert_eq!(
            clone.merged_extra_body(),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    // ── 辅助函数 ──

    #[test]
    fn resolve_thinking_style_passthrough() {
        assert_eq!(
            resolve_thinking_style("vllm", "https://api.deepseek.com", "x"),
            "vllm"
        );
        assert_eq!(
            resolve_thinking_style("off", "https://api.deepseek.com", "x"),
            "off"
        );
    }

    #[test]
    fn thinking_disable_body_returns_fresh_values() {
        // 原版断言"返回新鲜 dict"；Rust 每次构造新 Value，天然满足
        let first = thinking_disable_body("deepseek");
        assert_eq!(first, json!({"thinking": {"type": "disabled"}}));
        assert_eq!(
            thinking_disable_body("deepseek"),
            json!({"thinking": {"type": "disabled"}})
        );
    }

    #[test]
    fn unknown_style_treated_as_auto() {
        // 非法风格不在 THINKING_STYLES 表内 → 走 auto 启发式
        assert_eq!(
            resolve_thinking_style("bogus", "https://example.com/v1", "qwen3"),
            "qwen"
        );
    }
}
