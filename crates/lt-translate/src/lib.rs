//! lt-translate：LLM 翻译（原版 translator.py / benchmark.py 1:1 移植）。
//!
//! - [`thinking`]：思维链关闭策略（总开关 + 方式；auto 按端点/模型路由）
//! - [`reasoning`]：思维链隔离（把写进 `content` 的思考块剥掉，INV-F）
//! - [`verdict`]：回应体检（正文空/被截断/预算被推理吃光，方案 §4.4）
//! - [`translator`]：Translator（prompt/messages/请求体组装、流式/同步翻译、
//!   上下文历史、用量统计、重复检测）
//! - [`bench`]：多模型流式基准测试（TTFT/总耗时/排名）
//! - 费用计算：[`compute_cost`] / [`currency_symbol`]（对照原版 _compute_cost）

pub mod bench;
pub mod error;
pub mod reasoning;
pub mod thinking;
pub mod translator;
pub mod verdict;

pub use error::TranslateError;
pub use reasoning::{strip_reasoning, ReasoningStripper};
pub use verdict::{classify_response, FinishKind, ResponseVerdict};
pub(crate) use translator::runtime;
// E3/ADR-10：DEFAULT_PROMPT/PROMPT_PRESETS 已上移 lt-proto（lt-ui 直引 proto，
// 本 crate 的 re-export 随翻译页常量依赖边裁除而撤下）
pub use translator::{check_repetition, make_openai_client, TranslateStream, Translator, TranslatorParams};

/// 累计费用（原版 _compute_cost）：(pt*输入单价 + ct*输出单价) / 1M，
/// 单价为 0 时不计费返回 0。
pub fn compute_cost(
    prompt_tokens: u64,
    completion_tokens: u64,
    input_price: f64,
    output_price: f64,
) -> f64 {
    if input_price > 0.0 || output_price > 0.0 {
        (prompt_tokens as f64 * input_price + completion_tokens as f64 * output_price) / 1_000_000.0
    } else {
        0.0
    }
}

/// 费用货币符号（原版语义：ui_lang == "zh" → ¥，否则 $）
pub fn currency_symbol(ui_lang: &str) -> &'static str {
    if ui_lang == "zh" {
        "¥"
    } else {
        "$"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_accumulates_by_million() {
        let cost = compute_cost(1_000_000, 2_000_000, 1.0, 2.0);
        assert!((cost - 5.0).abs() < 1e-9);
    }

    #[test]
    fn zero_price_no_charge() {
        assert_eq!(compute_cost(123, 456, 0.0, 0.0), 0.0);
        assert_eq!(compute_cost(0, 0, 1.0, 1.0), 0.0);
    }

    #[test]
    fn partial_tokens_proportional() {
        let cost = compute_cost(500_000, 250_000, 2.0, 8.0);
        assert!((cost - (1.0 + 2.0)).abs() < 1e-9);
    }

    #[test]
    fn currency_by_ui_lang() {
        assert_eq!(currency_symbol("zh"), "¥");
        assert_eq!(currency_symbol("en"), "$");
    }
}
