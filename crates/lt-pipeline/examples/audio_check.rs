//! M1.2 冒烟：设备枚举 + loopback 实采 5 秒。
//!
//! 运行 `cargo run -p lt-pipeline --example audio_check`：
//! - 打印输出/输入设备列表与当前默认输出
//! - 实采 5s：轮询采集下静音期仍持续产出（chunk ≈31/s，静音块 RMS≈0）、
//!   有声期 RMS 明显非零
//! - 如听到本机在放音，RMS 应明显非零

use lt_pipeline::audio::{wasapi_win::WasapiBackend, AudioBackend, BoundedDropQueue, TARGET_RATE};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut be = WasapiBackend::new();
    eprintln!("[1] 枚举输出设备…");
    let outs = be.list_output_devices()?;
    eprintln!("[2] 输出 {} 个；枚举输入…", outs.len());
    let ins = be.list_input_devices()?;
    eprintln!("[3] 输入 {} 个；查默认输出…", ins.len());
    println!("输出设备 ({}):", outs.len());
    for d in &outs {
        println!("  - {d}");
    }
    println!("输入设备 ({}):", ins.len());
    for d in &ins {
        println!("  - {d}");
    }
    println!("当前默认输出: {:?}", be.current_default_output()?);
    eprintln!("[4] 启动采集线程…");

    let q = Arc::new(BoundedDropQueue::new(100, "audio_check"));
    be.start(None, None, q.clone())?;
    eprintln!("[5] 采集 5s（放点音乐看 RMS）…");
    let t0 = Instant::now();
    let mut n = 0usize;
    let mut max_rms = 0.0f32;
    let mut dropped_wait = 0;
    let mut last_beat = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        match q.pop_timeout(Duration::from_millis(200)) {
            Some((chunk, _mic)) => {
                n += 1;
                let r = lt_pipeline::rms(&chunk);
                if r > max_rms {
                    max_rms = r;
                }
                println!(
                    "chunk {:3} len={:4} rate={TARGET_RATE} rms={r:.4}",
                    n,
                    chunk.len()
                );
            }
            None => {
                dropped_wait += 1;
                if last_beat.elapsed() >= Duration::from_secs(1) {
                    last_beat = Instant::now();
                    eprintln!("[beat] elapsed={:?}", t0.elapsed());
                }
            }
        }
    }
    eprintln!("[5b] 采集循环退出，n={n}，stop…");
    be.stop();
    eprintln!("[6] 已停止");
    println!("共 {n} chunks，超时等待 {dropped_wait} 次，max_rms={max_rms:.4}");
    Ok(())
}
