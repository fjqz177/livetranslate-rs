//! 模型拉取工具：按注册表把模型文件下载到真实缓存目录（~/.config/livetranslate/models）。
//!
//! 用法：`cargo run -p lt-models --example model_pull -- [model_key]`
//! 默认 sensevoice-small（MS hub 优先）。完成后打印快照目录。

use lt_models::download::{Downloader, Hub, ProxyMode};
use lt_models::registry;

fn main() -> anyhow::Result<()> {
    let key = std::env::args().nth(1).unwrap_or_else(|| "sensevoice-small".into());
    let entry = registry::funasr_entry(&key).ok_or_else(|| anyhow::anyhow!("未知模型键: {key}"))?;
    let models_dir = lt_models::paths::models_dir(None)?;
    std::fs::create_dir_all(&models_dir)?;

    let dl = Downloader::new(&models_dir, ProxyMode::System);
    let (tx, rx) = std::sync::mpsc::channel();
    let repo = entry.ms.or(entry.hf).expect("至少一个源");
    let hub = if entry.ms.is_some() { Hub::Ms } else { Hub::Hf };
    println!("下载 {key}（{repo}，{:?}，{} 个文件）…", hub, entry.files.len());

    let dir = dl.download_files(hub, repo, entry.files, Some(&tx))?;
    drop(tx);
    for ev in rx.try_iter() {
        match ev {
            lt_models::download::DownloadEvent::Progress { file, done, total, .. } => {
                let t = total.map(|x| x.to_string()).unwrap_or_else(|| "?".into());
                println!("  {file}: {done}/{t}");
            }
            lt_models::download::DownloadEvent::Log(m) => println!("  {m}"),
            lt_models::download::DownloadEvent::FileDone { file, .. } => println!("完成: {file}"),
            lt_models::download::DownloadEvent::Done { .. } => {}
        }
    }
    println!("快照目录: {}", dir.display());
    Ok(())
}
