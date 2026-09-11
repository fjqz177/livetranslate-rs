//! 思维链（thinking/reasoning）关闭策略（W1 重写：docs/llm-api-redesign.md §2.3）。
//!
//! 语义由两处决定：
//! - `disable_thinking`（模型条目总开关，默认 true）：false → 不发送任何推理相关参数
//!   （即旧 `"off"` 的语义，`Settings::sanitize` 已把旧值归一化到该开关）；
//! - `thinking_style`（**方式**）：auto/None = 自动首选；deepseek/qwen/vllm/openai =
//!   只用该一种。
//!
//! W1 期 auto 的首选 = 顶层 `reasoning_effort: "none"`——事实标准（OpenAI 5.1+ 谱系、
//! Azure、Ollama 在册、OpenRouter、vLLM 官方记载自动映射为 `enable_thinking=false`、
//! llama.cpp server 文档明载 "If `none`, reasoning/thinking is disabled"）。
//! 2026-09-11 溯源复核的边界（docs/translator-probe-hotswap.md §十"官方来源"表）：
//! - **xAI 不在列**：官方无 `none` 且明载 "Reasoning cannot be disabled"；
//! - **LM Studio 官方未文档化** `reasoning_effort`（native 在册是 `reasoning:"off"`），
//!   本仓实测有效（43.7s→0.46s）——经验证据，保留；
//! - "在册"≠"零推理 token 保证"；静默忽略（Anthropic 兼容层、Ollama gpt-oss）只能靠
//!   体检/阶梯部分发现。
//!
//! W3 补"无效/被拒 → 依次降级"的链式兜底与会话内记忆。
//!
//! 官方 OpenAI / xAI / Anthropic 端点**保守不发**（不是"都会 400"——2026-09-11 勘正）：
//! OpenAI 侧有官方 400 实例（GPT-6 Astra + `none`），xAI 官方"推理无法禁用"、Anthropic
//! 兼容层对 `reasoning_effort` 标注 **Ignored** 且官方明载未知字段静默忽略而非报错——
//! 三家共同点是**本应用可发送的参数无法可靠关闭思维链**，"不发"（吃模型默认）是唯一
//! 不赌兼容性的选择；要真关须各家原生机制（`thinking:{type:"disabled"}` 等），
//! 经 OpenAI 兼容端点不可达；拒不掉的内容由 [`crate::reasoning`] 兜住不外泄。
//!
//! **回退阶梯（2026-09-10 第二轮评审/用户裁决）**：关闭形态按 [`next_step`] 逐级
//! 下退（reasoning_effort → enable_thinking → chat_template_kwargs →
//! thinking.type → 不发送 → 最小请求）；请求被 400/422 拒绝或体检显示"仍在推理"
//! 时下退一级，成功即按 (api_base, model) 记入会话记忆。退到"不含关闭参数"的
//! 台阶即视为**该模型无法关闭思维链**——界面取消勾选并标注，翻译照常进行，
//! 思考内容由 [`crate::reasoning`] 兜住不外泄。

use serde_json::{json, Value};

/// 官方 OpenAI / xAI / Anthropic 端点：**保守不发**（W1；W3 由降级链接管）。
/// 2026-09-11 溯源勘正理由：并非"都会对未知参数 400"——OpenAI 有 400 实例
/// （GPT-6 Astra + `none`，官方明载），xAI 官方"推理无法禁用"，Anthropic 兼容层
/// 对 `reasoning_effort` 标注 Ignored 且官方明载未知字段**静默忽略**；三家共同点是
/// 本应用可发送的参数无法可靠关闭思维链，"不发"是唯一不赌兼容的选择。
pub const PARAMLESS_ENDPOINTS: [&str; 3] = ["api.openai.com", "api.x.ai", "api.anthropic.com"];

/// 本机/回环端点：LM Studio / Ollama / llama.cpp / 自建 vLLM 的共同形态，
/// 发顶层 `reasoning_effort:"none"`（W1 最可靠的首选路径）。官方依据分档：
/// Ollama 在册、llama.cpp server 文档明载 none=关闭、vLLM 官方记载自动映射
/// `enable_thinking=false`；**LM Studio 官方未文档化**该字段（native 在册为
/// `reasoning:"off"`），本仓实测有效。
const LOCAL_HOSTS: [&str; 4] = ["127.0.0.1", "localhost", "[::1]", "0.0.0.0"];

/// 用嵌套 `thinking` 对象关闭的厂商（旧行为保留，避免回归）。
/// Moonshot/Kimi 于 2026-09-10 第二轮评审补入：官方文档明载 kimi-k2.6 用
/// `thinking:{"type":"disabled"}`；k3/k2.7-code 强制思考（传该参数会报错），
/// 由回退阶梯兜住并标注（见 [`next_step`]）。2026-09-11 溯源复核：DeepSeek /
/// 智谱 / 火山官方同字段同语义；**智谱 GLM-5.3 / 5.3-FLASH 亦为强制思考**
/// （官方明载传 disabled 报错）——同一兜底路径覆盖。
const NESTED_THINKING_MODELS: [&str; 4] = ["deepseek", "glm", "kimi", "moonshot"];
const NESTED_THINKING_ENDPOINTS: [&str; 6] = [
    "deepseek", "volces", "api.z.ai", "bigmodel", "moonshot", "kimi",
];

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
/// 2. `unavailable == true`（该模型已确认关不掉，持久化在模型配置里）→ 不发；
/// 3. `style` 为四种具体方式之一 → 只发该一种（用户显式选择永远优先）；
/// 4. `style == "off"`（仅旧档案可能残留）→ 不发；
/// 5. `style` 为 `None`/`auto`（含非法值兜底）→ 按下表自动路由：
///
/// | 端点/模型 | 形态 | 依据 |
/// |---|---|---|
/// | 本机回环（`127.0.0.1`/`localhost`/…） | `reasoning_effort:"none"` | 本机 LM Studio 实测 0.46s；vLLM/Ollama/llama.cpp 官方文档均认 |
/// | 官方端点 [`PARAMLESS_ENDPOINTS`] | 不发 | 无可靠关闭手段（OpenAI 有 400 实例 / xAI 不可禁用 / Anthropic 兼容层静默忽略），不发 = 吃模型默认 |
/// | deepseek/glm/kimi(moonshot) 家族 | `thinking:{type:disabled}` | 该族官方关闭参数 |
/// | dashscope/siliconflow | `enable_thinking:false` | 阿里/硅基官方关闭参数 |
/// | 其余（自建 vLLM/SGLang、网关、未知） | `reasoning_effort:"none"` | vLLM 官方会将 none 映射为 `enable_thinking=false` |
pub fn resolve_thinking_plan(
    disable: bool,
    unavailable: bool,
    style: Option<&str>,
    api_base: &str,
    model: &str,
) -> ThinkingPlan {
    if !disable || unavailable {
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

/// W3/方案 §2.3.1：自动链的下一个候选（`None` = 链尾）。
///
/// 链尾形态为 [`ThinkingPlan::None`]——**不发送任何推理相关参数**，即"认输"：
/// 强制思考的模型（GLM-5.3 / kimi-k3 / gpt-oss 等）关不掉，就让它照常思考，
/// 由思维链隔离器（`reasoning.rs`）兜住不外泄。`next_plan(None)` = `None`：已到
/// 链尾，不再有"下一个"（这正是 `disable_thinking = false` 时绝不自动注入的保证）。
pub fn next_plan(plan: ThinkingPlan) -> Option<ThinkingPlan> {
    match plan {
        ThinkingPlan::ReasoningEffortNone => Some(ThinkingPlan::EnableThinkingFalse),
        ThinkingPlan::EnableThinkingFalse => Some(ThinkingPlan::ChatTemplateKwargs),
        ThinkingPlan::ChatTemplateKwargs => Some(ThinkingPlan::NestedDisabled),
        ThinkingPlan::NestedDisabled => Some(ThinkingPlan::None),
        ThinkingPlan::None => None,
    }
}

/// 一次请求的完整形态（回退阶梯的台阶）：先逐个试关闭形态，全部无效则退到
/// **最小请求**——连用户配置的可选参数（温度/覆写/额外参数/输出上限）一起不带，
/// 只剩「模型 + 系统提示词 + 待译文本 + 流式开关」。
///
/// 依据（2026-09-10 用户裁决）：对强制思考的模型，"关不掉"不是失败——退到最小
/// 请求照样能翻译，思考内容由隔离器兜住；对锁定参数的厂商（如 Kimi 的
/// temperature），最小请求也天然规避了参数被拒。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStep {
    /// 带关闭形态的常规请求
    Plan(ThinkingPlan),
    /// 最小请求（不带任何可选参数）
    Minimal,
}

/// 阶梯的起点：从 `start` 形态开始（`Plan(None)` 时起点即"认输形态"，只差
/// 最小请求这一步）
pub fn first_step(start: ThinkingPlan) -> RequestStep {
    RequestStep::Plan(start)
}

/// 阶梯的下一台阶（`None` = 已到端点，无法再退）
pub fn next_step(step: RequestStep) -> Option<RequestStep> {
    match step {
        RequestStep::Plan(p) => Some(match next_plan(p) {
            Some(next) => RequestStep::Plan(next),
            None => RequestStep::Minimal,
        }),
        RequestStep::Minimal => None,
    }
}

/// 该台阶是否"已放弃关闭思维链"（= 最终形态不含任何关闭参数）
pub fn gives_up_disabling(step: RequestStep) -> bool {
    matches!(
        step,
        RequestStep::Plan(ThinkingPlan::None) | RequestStep::Minimal
    )
}

/// 该台阶是否"诚实可展示的关闭形态"（用于界面标注"当前实际在用"）
pub fn step_name(step: RequestStep) -> &'static str {
    match step {
        RequestStep::Plan(p) => p.name(),
        RequestStep::Minimal => "minimal",
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
        // 遍历值域（含旧 "off"）× 两种开关 × 两种"关不掉"标记，确保无 panic 且语义确定
        for style in [None, Some("auto"), Some("off"), Some("qwen"), Some("bogus")] {
            for disable in [true, false] {
                for unavailable in [true, false] {
                    let _ = resolve_thinking_plan(
                        disable,
                        unavailable,
                        style,
                        "http://127.0.0.1:1234/v1",
                        "m",
                    );
                }
            }
        }
    }

    // ── 回退阶梯（第二轮评审 §2.3.1/§2.3.2） ──

    /// 链尾 = 不发送（"认输"形态）；到链尾后不再有下一级
    #[test]
    fn plan_chain_tail_is_silence() {
        assert_eq!(
            next_plan(ThinkingPlan::NestedDisabled),
            Some(ThinkingPlan::None)
        );
        assert_eq!(next_plan(ThinkingPlan::None), None);
    }

    /// 阶梯完整走一遍：四形态 → 不发送 → 最小请求 → 端点
    #[test]
    fn step_ladder_ends_at_minimal() {
        let mut seen = vec![first_step(ThinkingPlan::ReasoningEffortNone)];
        while let Some(next) = next_step(*seen.last().unwrap()) {
            seen.push(next);
            assert!(seen.len() <= 8, "阶梯不得成环");
        }
        assert_eq!(
            seen,
            vec![
                RequestStep::Plan(ThinkingPlan::ReasoningEffortNone),
                RequestStep::Plan(ThinkingPlan::EnableThinkingFalse),
                RequestStep::Plan(ThinkingPlan::ChatTemplateKwargs),
                RequestStep::Plan(ThinkingPlan::NestedDisabled),
                RequestStep::Plan(ThinkingPlan::None),
                RequestStep::Minimal,
            ]
        );
        assert!(gives_up_disabling(RequestStep::Plan(ThinkingPlan::None)));
        assert!(gives_up_disabling(RequestStep::Minimal));
        assert!(!gives_up_disabling(RequestStep::Plan(
            ThinkingPlan::NestedDisabled
        )));
        assert_eq!(step_name(RequestStep::Minimal), "minimal");
    }

    /// 用户取消勾选（disable=false）→ 起点即"不发送"，且**没有任何下一级**
    /// （绝不自动注入关闭参数——方案 §2.3 规则 1）
    #[test]
    fn disabled_switch_has_no_ladder() {
        let start = first_step(resolve_thinking_plan(
            false,
            false,
            None,
            "http://127.0.0.1:1234/v1",
            "qwen3",
        ));
        assert_eq!(start, RequestStep::Plan(ThinkingPlan::None));
        assert_eq!(next_step(start), Some(RequestStep::Minimal));
    }

    /// 已确认关不掉的模型（持久化标记）→ 直接不发，不走关闭链
    #[test]
    fn unavailable_model_never_injects() {
        assert_eq!(
            resolve_thinking_plan(true, true, None, "https://api.deepseek.com/v1", "x"),
            ThinkingPlan::None
        );
        assert_eq!(
            resolve_thinking_plan(true, true, Some("deepseek"), "http://127.0.0.1:1/v1", "x"),
            ThinkingPlan::None
        );
    }

    /// Moonshot/Kimi 命中嵌套体（官方 k2.6 形态）；端点与模型名两条路都要认
    #[test]
    fn kimi_and_moonshot_route_to_nested_thinking() {
        assert_eq!(
            extra_body("https://api.moonshot.cn/v1", "kimi-k2.6", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
        assert_eq!(
            extra_body("https://api.moonshot.ai/v1", "anything", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
        assert_eq!(
            extra_body("https://my-gateway.example.com/v1", "kimi-k2.6", true, None),
            json!({"thinking": {"type": "disabled"}})
        );
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
            extra_body(
                "https://ark.cn-beijing.volces.com/api/v3",
                "ep-x",
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
        // 第三方托管的 deepseek 模型名同样命中（旧行为）
        assert_eq!(
            extra_body(
                "https://my-gateway.example.com/v1",
                "deepseek-r1",
                true,
                None
            ),
            json!({"thinking": {"type": "disabled"}})
        );
        // DashScope / SiliconFlow：扁平 enable_thinking
        assert_eq!(
            extra_body(
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
                "qwen3-max",
                true,
                None
            ),
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
            extra_body(
                "http://127.0.0.1:8080/v1",
                "deepseek-r1-distill-7b",
                true,
                None
            ),
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

    /// D-85/J 预设姿态 → 请求层实际形态（2026-09-11 评审修复补强）：预设表声明
    /// 的"官方能接受且我们已支持"的姿态必须落到预期 plan。OpenAI 行是 `"off"`
    /// 的规范编码（总开关 false = 不发任何关闭参数）——**不是** `reasoning_effort:"none"`。
    #[test]
    fn provider_presets_resolve_to_intended_plans() {
        for p in lt_proto::PROVIDER_PRESETS.iter().filter(|p| !p.is_custom()) {
            let plan = resolve_thinking_plan(
                p.disable_thinking,
                false,
                Some(p.thinking_style),
                p.api_base,
                p.model,
            );
            let want = match p.key {
                "deepseek" | "zhipu" | "moonshot" | "ark" => ThinkingPlan::NestedDisabled,
                "qwen" => ThinkingPlan::EnableThinkingFalse,
                "lmstudio" | "ollama" => ThinkingPlan::ReasoningEffortNone,
                "openai" => ThinkingPlan::None,
                other => panic!("未登记的新预设 {other}：请在此补预期形态"),
            };
            assert_eq!(plan, want, "{} 预设的实际关闭形态不符", p.key);
        }
        // 对话框落盘形态（disable=false）在 OpenAI 端点同样是"不发"
        assert_eq!(
            resolve_thinking_plan(false, false, None, "https://api.openai.com/v1", "gpt-x"),
            ThinkingPlan::None,
            "OpenAI 端点：不勾选关闭 = 不发送任何参数"
        );
    }
}
