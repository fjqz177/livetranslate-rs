//! 实时识别节拍探针（D-128，`#[ignore]` 不进 CI）：各引擎对 2/4/8/15s 合成
//! 缓冲的**单次重识别耗时**表——校准 `lt_proto::realtime_tick_scale` 倍率表
//! 与 max_speech 放宽档（规格 issue #39 实现决策 2/7 的数据源）。
//!
//! 运行（需模型缓存）：
//! `LIVETRANSLATE_CONFIG_DIR=<配置目录> cargo test -p lt-asr --test realtime_throttle_probe -- --ignored --nocapture`
//!
//! 判读：倍率 ≈ 各引擎 8s 缓冲耗时与 SenseVoice 8s 耗时之比（向上取 0.5 档）；
//! max_speech 放宽上限受「倍率 × 15s 缓冲耗时」约束（tick 实际节拍 ≤ 2s为宜）。

use lt_asr::AsrEngine;
use std::time::Instant;

const SR: usize = 16_000;

/// 确定性伪噪声（LCG；能量分布近似语音，够测耗时用）
fn synth(secs: usize) -> Vec<f32> {
    let n = secs * SR;
    let mut x = 0x2545F491u32;
    (0..n)
        .map(|i| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let s = ((x >> 16) & 0x7fff) as f32 / 32_768.0 - 0.5;
            if i % SR < SR / 10 {
                0.0
            } else {
                s * 0.3
            }
        })
        .collect()
}

fn sensevoice_dir() -> Option<std::path::PathBuf> {
    let md = lt_models::paths::models_dir(None).ok()?.join(
        "modelscope/models/pengzhendong--sherpa-onnx-sense-voice-zh-en-ja-ko-yue/snapshots/master",
    );
    md.is_dir().then_some(md)
}

fn qwen3_dir() -> Option<std::path::PathBuf> {
    let md = lt_models::paths::models_dir(None).ok()?.join(
        "huggingface/hub/models--csukuangfj2--sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25/snapshots/main",
    );
    md.is_dir().then_some(md)
}

fn bench(name: &str, eng: &mut dyn FnMut(&[f32]) -> Result<lt_proto::AsrResult, String>) {
    println!("── {name} ──");
    for secs in [2usize, 4, 8, 15] {
        let audio = synth(secs);
        // 预热一次（首次调用含惰性初始化，不计入）
        let _ = eng(&audio);
        let t0 = Instant::now();
        let r = eng(&audio);
        let ms = t0.elapsed().as_millis();
        match r {
            Ok(_) => println!(
                "{secs:>2}s 缓冲: {ms:>5}ms（RTF {:.3}，实际节拍须 ≥ 此值）",
                ms as f32 / (secs as u32 * 1000) as f32
            ),
            Err(e) => println!("{secs:>2}s 缓冲: 失败 {e}"),
        }
    }
}

#[test]
#[ignore = "真实模型缓存 + 合成长缓冲；D-128 节拍倍率表校准用（一次性）"]
fn probe_realtime_throttle_sensevoice() {
    let Some(md) = sensevoice_dir() else {
        println!("跳过（SenseVoice 未缓存；设 LIVETRANSLATE_CONFIG_DIR）");
        return;
    };
    let mut eng = lt_asr::sensevoice::SenseVoiceEngine::load(&md, Some(0.5), "auto")
        .expect("sensevoice 加载失败");
    bench("SenseVoice（倍率基准 1.0）", &mut |a| {
        eng.transcribe(a, false).map_err(|e| e.to_string())
    });
}

#[test]
#[ignore = "真实模型缓存（941MB 包）+ 合成长缓冲；D-128 节拍倍率表校准用"]
fn probe_realtime_throttle_qwen3() {
    let Some(md) = qwen3_dir() else {
        println!("跳过（qwen3 未缓存；设 LIVETRANSLATE_CONFIG_DIR）");
        return;
    };
    let mut eng = lt_asr::Qwen3AsrEngine::load(&md).expect("qwen3 加载失败");
    bench("Qwen3（倍率候选 3.0，探针后校准）", &mut |a| {
        eng.transcribe(a, false).map_err(|e| e.to_string())
    });
}
