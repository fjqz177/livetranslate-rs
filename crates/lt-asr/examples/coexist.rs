//! R-4 高危验证：ort 与 sherpa-onnx 同进程共存（M0 首日验证）。
//!
//! 两库各自捆绑一份 ONNX Runtime——本例各自执行一次最小操作强制符号解析，
//! 链接期/加载期的冲突会直接暴露。运行 `cargo run -p lt-asr --example coexist`。

fn main() -> anyhow::Result<()> {
    // ort 侧：构造 SessionBuilder（触及 ort-sys 符号）
    let _builder = ort::session::Session::builder()?;
    println!("ort: SessionBuilder 构造成功");

    // sherpa 侧：默认配置构造（触及 sherpa-onnx-sys 链接的 C 库）
    let cfg = sherpa_onnx::OfflineRecognizerConfig::default();
    println!(
        "sherpa: OfflineRecognizerConfig 默认构造成功 (num_threads={})",
        cfg.model_config.num_threads
    );

    println!("R-4 共存验证：链接与最小调用均通过");
    Ok(())
}
