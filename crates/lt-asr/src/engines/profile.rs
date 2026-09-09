//! 引擎超时档案（R6，架构 2.0 W6）：`base + per_audio * 段长（秒）` 动态超时。
//!
//! 60s 恒定超时（AH-8 前）的病灶：qwen3 本机 RTF≈0.28（≥3× 实时），30s
//! 真实语音段合法耗时 ≈8s 余量被 60s 表格撑住？不撑——烧穿的是**慢机/长段**
//! 组合：RTF 0.5 时 30s 段耗时 60s 整，恰在超时线上，抖动即三振。非超时应
//! 以**段长线性**建模，与模型规格无关（纯算力近似）。
//!
//! 断言锚（docs/archive/asr-engine-expansion.md §4.8）：qwen3 本机 RTF
//! 0.23–0.36；sensevoice/nano 快于实时。基准取保守上界。

/// 引擎键 → 超时档案。未知引擎（fuzz/测试）回落 60s 恒定（旧语义）。
pub fn transcribe_timeout_profile(engine: &str) -> (f64, f64) {
    match engine {
        // qwen3：base 10s + 实时段 ×4 余量（RTF 1.0 时 30s 段 ≈120s 预算）；
        // 慢机 RTF 2.0 仍 30s 段 240s——三振语义只该拦真挂死
        "qwen3" => (10.0, 4.0),
        // sensevoice/nano/whisper 快于实时：base 5s + 段长 ×2
        "sensevoice" | "nano" | "whisper" => (5.0, 2.0),
        // 测试 echo 等：短视窗（不拦正常往返，只防真挂死）
        _ => (60.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen3_long_segment_gets_proportional_budget() {
        // 30s 段（RTF 0.28 本机 ≈8.4s 计算）→ 130s 预算，不烧穿三振
        let (base, per) = transcribe_timeout_profile("qwen3");
        assert_eq!(base + per * 30.0, 10.0 + 4.0 * 30.0);
    }

    #[test]
    fn unknown_engine_falls_back_to_constant() {
        assert_eq!(transcribe_timeout_profile("echo"), (60.0, 0.0));
    }
}
