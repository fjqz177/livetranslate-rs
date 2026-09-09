//! M1.4 验证：Silero VAD（ort）输入名核对 + 置信度导出。
//!
//! 运行：
//! 1. `cargo run -p lt-audio --example vad_check`
//!    打印模型输入/输出名（与 v5 契约核对），并写出
//!    `target/vad_input.f32` + `target/vad_conf_rust.json`
//! 2. `python scripts/silero_reference.py`（onnxruntime 逐位对照，容差 1e-4）

use lt_audio::vad::{ConfidenceSource as _, SileroVad};

fn main() -> anyhow::Result<()> {
    // load-dynamic：任何 ort 调用前必须就位（首次加载读 ORT_DYLIB_PATH）
    lt_audio::ensure_ort_dylib()?;
    // 独立构建 session 以打印元数据
    let model = std::fs::read("assets/silero_vad.onnx").or_else(|_| {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/silero_vad.onnx"
        ))
    })?;
    let session = ort::session::Session::builder()?.commit_from_memory(&model)?;
    println!("── 模型 I/O 契约 ──");
    for i in session.inputs() {
        println!("input : {}", i.name());
    }
    for o in session.outputs() {
        println!("output: {}", o.name());
    }

    // 确定性"伪语音"信号：8s，0.4s 正弦爆发 + 0.2s 静音交替
    const SR: usize = 16000;
    const WIN: usize = 512;
    let n = SR * 8;
    let mut signal = Vec::with_capacity(n);
    let mut state: u64 = 0x243F6A8885A308D3;
    let mut lcg = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as f64 / (1u64 << 31) as f64) - 1.0
    };
    for i in 0..n {
        let t = i as f64 / SR as f64;
        let burst = (t % 0.6) < 0.4;
        let v = if burst {
            0.5 * (2.0 * std::f64::consts::PI * 220.0 * t).sin()
                + 0.3 * (2.0 * std::f64::consts::PI * 660.0 * t).sin()
                + 0.05 * lcg()
        } else {
            0.005 * lcg()
        };
        signal.push(v as f32);
    }

    // 逐窗口置信度（与 python 参考同序）
    let mut vad = SileroVad::new()?;
    let mut confs = Vec::new();
    for chunk in signal.chunks(WIN) {
        confs.push(vad.confidence(chunk)?);
    }

    // 落盘：原始输入 + Rust 置信度
    let out_dir = std::path::Path::new("target");
    std::fs::create_dir_all(out_dir)?;
    let mut raw = Vec::with_capacity(n * 4);
    for s in &signal {
        raw.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(out_dir.join("vad_input.f32"), &raw)?;
    let json = serde_json::to_string(&confs)?;
    std::fs::write(out_dir.join("vad_conf_rust.json"), json)?;

    let n_c = confs.len();
    println!(
        "chunks={n_c} 前3={:?} 中3={:?} 末3={:?}",
        &confs[..3],
        &confs[n_c / 2..n_c / 2 + 3],
        &confs[n_c - 3..]
    );
    println!("写出 target/vad_input.f32 与 target/vad_conf_rust.json");
    println!("下一步：python scripts/silero_reference.py 做逐位对照（<1e-4）");
    Ok(())
}
