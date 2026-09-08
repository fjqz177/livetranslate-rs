//! 下载器真网冒烟：从 ModelScope 拉取 SenseVoice 的 tokens.txt（小文件）。
//!
//! 运行：`cargo run -p lt-models --example download_check`
//! 验证 URL 方案、Content-Range 解析与落盘布局在真实 CDN 上成立。

use lt_models::download::{DownloadEvent, Downloader, Hub, ProxyMode};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;

fn main() -> anyhow::Result<()> {
    let models_dir = std::env::temp_dir().join("lt_download_check");
    let _ = std::fs::remove_dir_all(&models_dir);
    let dl = Downloader::new(&models_dir, ProxyMode::System);
    let repo = "pengzhendong/sherpa-onnx-sense-voice-zh-en-ja-ko-yue";

    let (tx, rx) = channel();
    let dir = dl.download_files(
        Hub::Ms,
        repo,
        &[("tokens.txt", 1, "")],
        &AtomicBool::new(false),
        Some(&tx),
    )?;

    for ev in rx.try_iter() {
        match ev {
            DownloadEvent::Progress {
                file, done, total, ..
            } => {
                println!(
                    "progress {file}: {done}/{}",
                    total.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
                )
            }
            DownloadEvent::FileDone { file, .. } => println!("done file: {file}"),
            DownloadEvent::Done { dir, .. } => println!("done dir: {}", dir.display()),
            DownloadEvent::Log(m) => println!("log: {m}"),
        }
    }
    let tokens = std::fs::read_to_string(dir.join("tokens.txt"))?;
    let first = tokens.lines().next().unwrap_or("").to_string();
    println!(
        "tokens.txt 首行: {first:?}（总 {} 行）",
        tokens.lines().count()
    );
    anyhow::ensure!(tokens.lines().count() > 1000, "tokens.txt 行数异常");
    println!("真网冒烟 PASS");
    let _ = std::fs::remove_dir_all(&models_dir);
    Ok(())
}
