//! 引擎超时档案（R6，架构 2.0 W6）：`base + per_audio * 段长（秒）` 动态超时。
//!
//! 60s 恒定超时（AH-8 前）的病灶：qwen3 本机 RTF≈0.28（≥3× 实时），30s
//! 真实语音段合法耗时 ≈8s 余量被 60s 表格撑住？不撑——烧穿的是**慢机/长段**
//! 组合：RTF 0.5 时 30s 段耗时 60s 整，恰在超时线上，抖动即三振。非超时应
//! 以**段长线性**建模，与模型规格无关（纯算力近似）。
//!
//! 断言锚（docs/archive/asr-engine-expansion.md §4.8）：qwen3 本机 RTF
//! 0.23–0.36；sensevoice/nano 快于实时。基准取保守上界。

use lt_proto::EngineKey;

/// settings 级引擎身份 → 超时档案。
///
/// D-116（C1）：入参从裸 &str 改 `EngineKey`——旧表键是 worker 级
/// （"sensevoice"/"nano"），唯一调用方却喂 settings 级 `raw.asr_engine`
/// （"funasr"），词表错位致 funasr 族恒落 60s 兜底、fast 档自 R6 起从未生效。
/// 未知键的防护在透镜单点（`from_settings_str` warn + FunAsr 回退），档案只认三变体。
pub fn transcribe_timeout_profile(engine: EngineKey) -> (f64, f64) {
    match engine {
        // qwen3：base 10s + 实时段 ×4 余量（RTF 1.0 时 30s 段 ≈120s 预算）；
        // 慢机 RTF 2.0 仍 30s 段 240s——三振语义只该拦真挂死
        EngineKey::Qwen3 => (10.0, 4.0),
        // funasr 族（sensevoice/nano）与 whisper 快于实时：base 5s + 段长 ×2
        EngineKey::FunAsr | EngineKey::Whisper => (5.0, 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lt_proto::EngineKey;

    #[test]
    fn funasr_family_gets_fast_budget_from_settings_key() {
        // C1 回归（D-116）：settings 键 "funasr" 经透镜必须拿快档——
        // 旧病灶：表键 worker 级、唯一调用方喂 settings 级 "funasr" 恒落 60s
        // 兜底，fast 档自 R6 起从未生效。
        assert_eq!(
            transcribe_timeout_profile(EngineKey::from_settings_str("funasr")),
            (5.0, 2.0)
        );
    }

    #[test]
    fn whisper_gets_fast_budget() {
        assert_eq!(transcribe_timeout_profile(EngineKey::Whisper), (5.0, 2.0));
    }

    #[test]
    fn qwen3_long_segment_gets_proportional_budget() {
        // 30s 段（RTF 0.28 本机 ≈8.4s 计算）→ 130s 预算，不烧穿三振
        let (base, per) = transcribe_timeout_profile(EngineKey::Qwen3);
        assert_eq!(base + per * 30.0, 10.0 + 4.0 * 30.0);
    }

    #[test]
    fn unknown_settings_key_falls_back_via_lens() {
        // 未知键的防护在透镜单点（warn + FunAsr 回退，D-79），档案只认三变体；
        // 旧 `_ => 60s` 臂随枚举化消失，此测试钉住替代路径。
        assert_eq!(EngineKey::from_settings_str("echo"), EngineKey::FunAsr);
        assert_eq!(
            transcribe_timeout_profile(EngineKey::from_settings_str("echo")),
            (5.0, 2.0)
        );
    }
}
