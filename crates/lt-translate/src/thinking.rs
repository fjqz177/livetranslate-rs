//! 思维链（thinking/reasoning）关闭策略（W1 重写：docs/llm-api-redesign.md §2.3）。
//!
//! 语义由两处决定：
//! - `disable_thinking`（模型条目总开关，默认 true）：false → 不发送任何推理相关参数
//!   （即旧 `"off"` 的语义，`Settings::sanitize` 已把旧值归一化到该开关）；
//! - `thinking_style`（**方式**）：auto/None = 自动首选；deepseek/qwen/vllm/openai =
//!   只用该一种。
//!
//! W1 期 auto 的首选 = 顶层 `reasoning_effort: "none"`——事实标准（OpenAI 5.1+ 谱系、
//! Azure、xAI、Ollama、OpenRouter、vLLM 自动映射为 `enable_thinking=false`、llama.cpp
//! 官方 README 明载 "If `none`, reasoning/thinking is disabled"；本机 LM Studio 实测
//! 由 43.7s 降至 0.46s）。W3 补"无效/被拒 → 依次降级"的链式兜底与会话内记忆。
//!
//! 官方 OpenAI/xAI/Anthropic 端点**保守不发**：这些端点对不支持的参数直接 400，
//! 在 W3 的参数被拒降级上线前保持旧行为（见 [`PARAMLESS_ENDPOINTS`]）。

use serde_json::{json, Value};

/// 对"不认识的参数"会直接报错的官方端点（W1 保守不发；W3 由降级链接管）
pub const PARAMLESS_ENDPOINTS: [&str; 3] = ["api.openai.com", "api.x.ai", "api.anthropic.com"];

/// 本机/回环端点：LM Studio / Ollama / llama.cpp / 自建 vLLM 的共同形态，
/// 实测与官方文档均支持顶层 `reasoning_effort:"none"`（W1 最可靠的首选路径）
const LOCAL_HOSTS: [&str; 4] = ["127.0.0.1", "localhost", "[::1]", "0.0.0.0"];

/// 用嵌套 `thinking` 对象关闭的厂商（旧行为保留，避免回归）
const NESTED_THINKING_MODELS: [&str; 2] = ["deepseek", "glm"];
const NESTED_THINKING_ENDPOINTS: [&str; 4] = ["deepseek", "volces", "api.z.ai", "bigmodel"];

/// 用扁平 `enable_thinking` 关闭的厂商端点（旧行为等价物）
const ENABLE_THINKING_ENDPOINTS: [&str; 3] = ["dashscope", "aliyuncs.com", "siliconflow"];

/// 实际要发送的"关闭推理"请求体形态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingPlan {
    /// 不发送任何推理相关参数
    None,
    /// 顶层 `reasoning_effort: "none"`（自动首选）
    ReasoningEffortNone,
    /// 顶层 `enable_thinking: false`（DashScope/Qwen、SiliconFlow）
    EnableThinkingFalse,
    /// `chat_template_kwargs: {"enable_thinking": false}`（vLLM/SGLang 模板）
    ChatTemplateKwargs,
    /// `thinking: {"type": "disabled"}`（DeepSeek、GLM、Moonshot）
    NestedDisabled,
}

impl ThinkingPlan {
    /// 日志/持久化用的短名（与 `THINKING_STYLES` 的显式方式同名）
    pub fn name(self) -> &'static str {
        match self {
            ThinkingPlan::None => "none",
            ThinkingPlan::ReasoningEffortNone => "openai",
            ThinkingPlan::EnableThinkingFalse => "qwen",
            ThinkingPlan::ChatTemplateKwargs => "vllm",
            ThinkingPlan::NestedDisabled => "deepseek",
        }
    }
}

/// 解析"这次请求要不要发、发哪种"关闭体。
///
/// 规则（方案 §2.3，唯一一条链）：
/// 1. `disable == false` → 不发（`thinking_style` 被忽略）；
/// 2. `style` 为四种具体方式之一 → 只发该一种（用户显式选择永远优先）；
/// 3. `style == "off"`（仅旧档案可能残留）→ 不发；
/// 4. `style` 为 `None`/`auto`（含非法值兜底）→ 按下表自动路由：
///
/// | 端点/模型 | 形态 | 依据 |
/// |---|---|---|
/// | 本机回环（`127.0.0.1`/`localhost`/…） | `reasoning_effort:"none"` | 本机 LM Studio 实测 0.46s；vLLM/Ollama/llama.cpp 官方文档均认 |
/// | 官方端点 [`PARAMLESS_ENDPOINTS`] | 不发 | 对不支持的参数直接 400（旧行为保留，W3 降级链接管） |
/// | deepseek/glm 家族 | `thinking:{type:disabled}` | 该族不认 `reasoning_effort:"none"`（旧行为保留，避免回归） |
/// | dashscope/siliconflow | `enable_thinking:false` | 阿里/硅基官方关闭参数（旧行为等价物） |
/// | 其余（自建 vLLM/SGLang、网关、未知） | `reasoning_effort:"none"` | vLLM 官方会将 none 映射为 `enable_thinking=false` |
pub fn resolve_thinking_plan(
    disable: bool,
    style: Option<&str>,
    api_base: &str,
    model: &str,
) -> ThinkingPlan {
    if !disable {
        return ThinkingPlan::None;
    }
    match style {
        Some("deepseek") => ThinkingPlan::NestedDisabled,
        Some("qwen") => ThinkingPlan::EnableThinkingFalse,
        Some("vllm") => ThinkingPlan::ChatTemplateKwargs,
        Some("openai") => ThinkingPlan::ReasoningEffortNone,
        Some("off") => ThinkingPlan::None,
        _ => {
            let endpoint = api_base.to_lowercase();
            // 本机/回环优先：这一类端点上 reasoning_effort:"none" 最可靠
            if LOCAL_HOSTS.iter().any(|h| endpoint.contains(h)) {
                return ThinkingPlan::ReasoningEffortNone;
            }
            if PARAMLESS_ENDPOINTS.iter().any(|m| endpoint.contains(m)) {
                return ThinkingPlan::None;
            }
            let model_id = model.to_lowercase();
            if NESTED_THINKING_MODELS.iter().any(|m| model_id.contains(m))
                || NESTED_THINKING_ENDPOINTS
                    .iter()
                    .any(|m| endpoint.contains(m))
            {
                return ThinkingPlan::NestedDisabled;
            }
            if ENABLE_THINKING_ENDPOINTS
                .iter()
                .any(|m| endpoint.contains(m))
            {
                return ThinkingPlan::EnableThinkingFalse;
            }
            ThinkingPlan::ReasoningEffortNone
        }
    }
}

/// W3/方案 §2.3.1：自动链的下一个候选（`None` = 链尾，无法再降级）
pub fn next_plan(plan: ThinkingPlan) -> Option<ThinkingPlan> {
    match plan {
        ThinkingPlan::None => Some(ThinkingPlan::ReasoningEffortNone),
        ThinkingPlan::ReasoningEffortNone => Some(ThinkingPlan::EnableThinkingFalse),
        ThinkingPlan::EnableThinkingFalse => Some(ThinkingPlan::ChatTemplateKwargs),
        ThinkingPlan::ChatTemplateKwargs => Some(ThinkingPlan::NestedDisabled),
        ThinkingPlan::NestedDisabled => None,
    }
}

/// 具体形态对应的"关闭 thinking"请求体片段；[`ThinkingPlan::None`] → Null（= 不发）。
/// 每次调用返回新 Value（无共享可变状态）。
pub fn thinking_disable_body(plan: ThinkingPlan) -> Value {
    match plan {
        ThinkingPlan::None => Value::Null,
        ThinkingPlan::ReasoningEffortNone => json!({"reasoning_effort": "none"}),
        ThinkingPlan::EnableThinkingFalse => json!({"enable_thinking": false}),
        ThinkingPlan::ChatTemplateKwargs => {
            json!({"chat_template_kwargs": {"enable_thinking": false}})
        }
        ThinkingPlan::NestedDisabled => json!({"thinking": {"type": "disabled"}}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translator::Translator;

    /// 经 Translator.merged_extra_body 观察实际发送体（无则 Null）
    fn extra_body(
        api_base: &str,
        model: &str,
        disable_thinking: bool,
        thinking_style: Option<&str>,
    ) -> Value {
        let t = Translator::for_test(api_base, model, disable_thinking, thinking_style, None);
        t.merged_extra_body()
    }

    // ── 总开关（方案 §2.3 规则 1） ──

    #[test]
    fn switch_off_sends_nothing() {
        // 关闭总开关后，无论方式如何都不发送
        for style in [None, Some("qwen"), Some("openai"), Some("off")] {
            assert_eq!(
                extra_body("http://127.0.0.1:1234/v1", "qwen3", false, style),
                Value::Null,
                "disable_thinking=false 时不应发送任何参数（style={style:?}）"
            );
        }
    }

    #[test]
    fn legacy_off_style_sends_nothing() {
        // 旧档案值 "off"（sanitize 已归一化，此处兜底）→ 不发
        assert_eq!(
            extra_body("http://example.com/v1", "m", true, Some("off")),
            Value::Null
        );
    }

    // ── 显式方式（方案 §2.3 规则 2） ──

    #[test]
    fn explicit_styles_send_their_own_shape() {
        let b = |style| extra_body("http://example.com/v1", "m", true, Some(style));
        assert_eq!(b("openai"), json!({"reasoning_effort": "none"}));
        assert_eq!(b("qwen"), json!({"enable_thinking": false}));
        assert_eq!(
            b("vllm"),
            json!({"chat_template_kwargs": {"enable_thinking": false}})
        );
        assert_eq!(b("deepseek"), json!({"thinking": {"type": "disabled"}}));
    }

    #[test]
    fn explicit_style_wins_over_endpoint_conservatism() {
        // 用户显式指定时不走端点保守规则（即便官方端点，用户要求就发）
        assert_eq!(
            extra_body("https://api.openai.com/v1", "gpt-5.1", true, Some("openai")),
            json!({"reasoning_effort": "none"})
        );
    }

    // ── 自动首选（方案 §2.3 规则 3；W1 核心行为变更） ──

    #[test]
    fn auto_prefers_reasoning_effort_none_on_local_endpoints() {
        // W1 核心：本机 LM Studio（旧行为误发 enable_thinking，实测无效）
        assert_eq!(
            extra_body("http://127.0.0.1:1234/v1", "qwen3.5-4b-mtp", true, None),
            json!({"reasoning_effort": "none"})
        );
        assert_eq!(
            extra_body("http://127.0.0.1:1234/v1", "m", true, Some("auto")),
            json!({"reasoning_effort": "none"})
        );
        // 非官方端点的自建服务同样走首选
        assert_eq!(
            extra_body("https://my-vllm.internal/v1", "qwen3", true, None),
            json!({"reasoning_effort": "none"})
        );
    }

    #[test]
    fn auto_stays_silent_on_official_endpoints() {
        // W1 保守：官方端点对不支持的参数会 400（W3 由降级链接管）
        for base in [
            "https://api.openai.com/v1",
            "https://api.x.ai/v1",
            "https://api.anthropic.com/v1",
        ] {
            assert_eq!(
                extra_body(base, "some-model", true, None),
                Value::Null,
                "{base} 应保持不发"
            );
        }
    }

    #[test]
    fn unknown_style_falls_back_to_auto() {
        // 非法值（不在 THINKING_STYLES 内）→ 走自动首选
        assert_eq!(
            extra_body("http://127.0.0.1:9999/v1", "m", true, Some("bogus")),
            json!({"reasoning_effort": "none"})
        );
    }

    // ── 形态与解析的纯函数面 ──

    #[test]
    fn plan_body_shapes_are_fresh_values() {
        let first = thinking_disable_body(ThinkingPlan::NestedDisabled);
        assert_eq!(first, json!({"thinking": {"type": "disabled"}}));
        assert_eq!(
            thinking_disable_body(ThinkingPlan::NestedDisabled),
            json!({"thinking": {"type": "disabled"}})
        );
        assert!(thinking_disable_body(ThinkingPlan::None).is_null());
    }

    #[test]
    fn plan_names_match_value_domain() {
        assert_eq!(ThinkingPlan::ReasoningEffortNone.name(), "openai");
        assert_eq!(ThinkingPlan::EnableThinkingFalse.name(), "qwen");
        assert_eq!(ThinkingPlan::ChatTemplateKwargs.name(), "vllm");
        assert_eq!(ThinkingPlan::NestedDisabled.name(), "deepseek");
        assert_eq!(ThinkingPlan::None.name(), "none");
    }

    #[test]
    fn resolve_plan_is_total() {
        // 遍历值域（含旧 "off"）与两种开关，确保无 panic 且语义确定
        for style in [None, Some("auto"), Some("off"), Some("qwen"), Some("bogus")] {
            let _ = resolve_thinking_plan(true, style, "http://127.0.0.1:1234/v1", "m");
            let _ = resolve_thinking_plan(false, style, "http://127.0.0.1:1234/v1", "m");
        }
    }

    // ── 自动路由表（W1：保留旧厂商映射，只改"未知/本机"这一条路径） ──

    #[test]
    fn auto_keeps_known_vendor_mappings() {
        // deepseek/glm 家族：该族不认 reasoning_effort:"none"，保留嵌套体
        assert_eq!(
            extra_body("https://api.deepseek.com/v1", "anything", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
        assert_eq!(
            extra_body("https://ark.cn-beijing.volces.com/api/v3", "ep-x", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
        // 第三方托管的 deepseek 模型名同样命中（旧行为）
        assert_eq!(
            extra_body("https://my-gateway.example.com/v1", "deepseek-r1", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
        // DashScope / SiliconFlow：扁平 enable_thinking
        assert_eq!(
            extra_body("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen3-max", true, None),
            json!({"enable_thinking": false})
        );
        assert_eq!(
            extra_body("https://api.siliconflow.cn/v1", "Qwen/Qwen3-8B", true, None),
            json!({"enable_thinking": false})
        );
    }

    #[test]
    fn local_endpoint_wins_over_model_name() {
        // 本机跑 deepseek 蒸馏模型：回环端点优先 → 发 reasoning_effort:"none"
        //（本机 LM Studio/llama.cpp 只认这个）
        assert_eq!(
            extra_body("http://127.0.0.1:8080/v1", "deepseek-r1-distill-7b", true, None),
            json!({"reasoning_effort": "none"})
        );
        assert_eq!(
            extra_body("http://localhost:11434/v1", "glm-4-flash", true, None),
            json!({"reasoning_effort": "none"})
        );
    }

    #[test]
    fn unknown_endpoint_uses_reasoning_effort_none() {
        // W1 修复点：自建 vLLM/SGLang、聚合网关、Ollama → reasoning_effort:none
        assert_eq!(
            extra_body("https://my-vllm.internal/v1", "my-model", true, None),
            json!({"reasoning_effort": "none"})
        );
    }

    // ── 逃生舱与派生（既有行为，W1 保留） ──

    /// 用户 extra_body 覆盖自动体（同键以用户为准）
    #[test]
    fn user_extra_body_overrides_automatic_toggle() {
        let t = Translator::for_test(
            "http://127.0.0.1:1234/v1",
            "m",
            true,
            None,
            Some(json!({"reasoning_effort": "high", "user_id": "t"})),
        );
        assert_eq!(
            t.merged_extra_body(),
            json!({"reasoning_effort": "high", "user_id": "t"})
        );
    }

    /// share_client 保留关闭形态（派生装置一路传承）
    #[test]
    fn shared_client_preserves_thinking_control() {
        let t = Translator::for_test("http://127.0.0.1:1234/v1", "m", true, None, None);
        assert_eq!(
            t.share_client().merged_extra_body(),
            json!({"reasoning_effort": "none"})
        );
    }
}
