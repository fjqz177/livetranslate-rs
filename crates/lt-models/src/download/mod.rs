//! 双 hub 模型下载器（PLAN §2.10；原版 hf-hub/modelscope SDK 的自控替代，M-01）。
//!
//! - 布局自控（写入与 [`crate::cache`] 探测共用同一约定）：
//!   MS → `modelscope/models/{org}--{name}/snapshots/master/{path}`
//!   HF → `huggingface/hub/models--{org}--{name}/snapshots/main/{path}`
//! - 断点续传：先写 `{file}.incomplete`（带 Range 续传），完成 rename；
//!   服务端忽略 Range 返 200 时从头重写，416 时删除陈旧 .incomplete 重下。
//! - 重试 3 次指数退避（1s/4s/16s）；代理三模式；进度事件走 channel。
//!
//! 用阻塞式 reqwest（专用线程），不用异步管道（D 系偏差：简单且可测）。

pub mod hf;
pub mod ms;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 下载目标 hub
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hub {
    Ms,
    Hf,
}

/// 代理三模式（语义对齐原版 proxy="none" 绕系统代理，E-03）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyMode {
    /// 强制直连（trust_env=False 等价）
    None,
    /// 跟随系统环境变量
    #[default]
    System,
    /// 指定 URL
    Url(String),
}

/// 下载事件（→ 日志窗 / 下载对话框）
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    /// 单文件进度（total 未知时为 None）
    Progress { repo: String, file: String, done: u64, total: Option<u64> },
    /// 单文件完成
    FileDone { repo: String, file: String },
    /// 全部完成（快照目录）
    Done { repo: String, dir: PathBuf },
    /// 非致命日志（重试等）
    Log(String),
}

/// 重试退避序列（原版 1s/4s/16s）
const BACKOFFS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(16),
];

/// 进度事件节流：累计增量超过此值才发一条
const PROGRESS_STEP: u64 = 256 * 1024;

pub struct Downloader {
    models_dir: PathBuf,
    proxy: ProxyMode,
    /// HF endpoint 可覆写为镜像（如 https://hf-mirror.com）
    hf_endpoint: String,
    /// ModelScope API 根（测试可指向本地 mock）
    ms_endpoint: String,
}

impl Downloader {
    pub fn new(models_dir: impl Into<PathBuf>, proxy: ProxyMode) -> Self {
        Self {
            models_dir: models_dir.into(),
            proxy,
            hf_endpoint: "https://huggingface.co".into(),
            ms_endpoint: "https://modelscope.cn".into(),
        }
    }

    /// HF 镜像覆写
    pub fn with_hf_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.hf_endpoint = endpoint.into();
        self
    }

    /// 测试用：覆写 ModelScope API 根
    pub fn with_ms_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.ms_endpoint = endpoint.into();
        self
    }

    /// 下载完成后快照目录（不发起网络请求）
    pub fn snapshot_dir(&self, hub: Hub, repo: &str) -> PathBuf {
        match hub {
            Hub::Ms => crate::cache::hf_style_snapshot(&self.models_dir, hub, repo, "master"),
            Hub::Hf => crate::cache::hf_style_snapshot(&self.models_dir, hub, repo, "main"),
        }
    }

    /// 单文件 URL
    pub fn file_url(&self, hub: Hub, repo: &str, path: &str) -> String {
        match hub {
            Hub::Hf => hf::file_url(&self.hf_endpoint, repo, path),
            Hub::Ms => ms::file_url(&self.ms_endpoint, repo, path),
        }
    }

    /// 构造 HTTP 客户端（代理三模式；连接超时 10s，无总超时——大文件下载）
    pub fn http_client(&self) -> anyhow::Result<reqwest::blocking::Client> {
        let mut b = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("livetranslate-rs/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(10));
        b = match &self.proxy {
            ProxyMode::None => b.no_proxy(),
            ProxyMode::System => b,
            ProxyMode::Url(u) => b.proxy(reqwest::Proxy::all(u)?),
        };
        Ok(b.build()?)
    }

    /// 下载 repo 的文件清单到快照目录，返回快照目录。
    /// 目标文件已存在则跳过（幂等）。
    pub fn download_files(
        &self,
        hub: Hub,
        repo: &str,
        files: &[&str],
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<PathBuf> {
        let client = self.http_client()?;
        let dir = self.snapshot_dir(hub, repo);
        std::fs::create_dir_all(&dir)?;
        for file in files {
            let target = dir.join(file);
            if target.is_file() {
                self.emit(tx, DownloadEvent::Log(format!("[{repo}] 已存在，跳过 {file}")));
                continue;
            }
            self.emit(tx, DownloadEvent::Log(format!("[{repo}] 开始下载 {file}")));
            self.download_one(&client, hub, repo, file, &target, tx)?;
            self.emit(tx, DownloadEvent::FileDone { repo: repo.into(), file: (*file).into() });
        }
        self.emit(tx, DownloadEvent::Done { repo: repo.into(), dir: dir.clone() });
        Ok(dir)
    }

    /// 单文件下载（续传 + 重试）。`target` 为最终路径。
    fn download_one(
        &self,
        client: &reqwest::blocking::Client,
        hub: Hub,
        repo: &str,
        file: &str,
        target: &Path,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<()> {
        let incomplete = incomplete_path(target);
        let mut last_err: Option<anyhow::Error> = None;
        for (attempt, backoff) in std::iter::once(Duration::ZERO).chain(BACKOFFS).enumerate() {
            if backoff > Duration::ZERO {
                let msg = format!("[{repo}] {file} 下载失败，{backoff:?} 后第 {attempt} 次重试");
                tracing::warn!("{msg}");
                self.emit(tx, DownloadEvent::Log(msg));
                std::thread::sleep(backoff);
            }
            match self.try_download(client, hub, repo, file, target, &incomplete, tx) {
                Ok(()) => return Ok(()),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap())
    }

    fn try_download(
        &self,
        client: &reqwest::blocking::Client,
        hub: Hub,
        repo: &str,
        file: &str,
        target: &Path,
        incomplete: &Path,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<()> {
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut done: u64 = incomplete.metadata().map(|m| m.len()).unwrap_or(0);

        // 外层处理"Range 被拒（416）"的一次性回退
        loop {
            let url = self.file_url(hub, repo, file);
            let mut req = client.get(&url);
            if done > 0 {
                req = req.header("Range", format!("bytes={done}-"));
            }
            let resp = req.send()?;
            // 注意：不使用 error_for_status——416 是续传回退信号，非错误
            let status = resp.status();

            let supports_resume = status == reqwest::StatusCode::PARTIAL_CONTENT;
            if !supports_resume && !status.is_success() {
                if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && done > 0 {
                    // .incomplete 比远端文件长（陈旧残留）：删除重下
                    self.emit(tx, DownloadEvent::Log(format!("[{repo}] {file} 续传偏移失效，重新下载")));
                    let _ = std::fs::remove_file(incomplete);
                    done = 0;
                    continue;
                }
                anyhow::bail!("HTTP {status}");
            }
            if !supports_resume && done > 0 {
                // 服务端忽略 Range 返 200：从头重写
                self.emit(
                    tx,
                    DownloadEvent::Log(format!("[{repo}] {file} 服务端不支持续传，从头下载")),
                );
                done = 0;
            }

            // 总长：206 → Content-Range 尾段；200 → Content-Length（此时 done==0）
            let total = parse_total(&resp);

            // 打开续传临时文件：续传走追加；全新/重写走截断
            // （Windows 上 append 句柄 set_len 会报拒绝访问，不能混用）
            let mut f = if supports_resume && done > 0 {
                std::fs::OpenOptions::new().create(true).append(true).open(incomplete)?
            } else {
                done = 0;
                let _ = std::fs::remove_file(incomplete);
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(incomplete)?
            };

            let mut written = done;
            let mut buf = [0u8; 64 * 1024];
            let mut resp = resp;
            loop {
                let n = resp.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n])?;
                written += n as u64;
                if written - done >= PROGRESS_STEP {
                    self.progress(tx, repo, file, written, total);
                    done = written;
                }
            }
            f.flush()?;

            // 完整性：总长已知则校验
            if let Some(total) = total {
                anyhow::ensure!(written == total, "长度不完整 {written}/{total}");
            }
            drop(f);
            std::fs::rename(incomplete, target)?;
            self.progress(tx, repo, file, written, total.or(Some(written)));
            return Ok(());
        }
    }

    fn emit(&self, tx: Option<&Sender<DownloadEvent>>, ev: DownloadEvent) {
        if let Some(tx) = tx {
            let _ = tx.send(ev);
        }
    }

    fn progress(
        &self,
        tx: Option<&Sender<DownloadEvent>>,
        repo: &str,
        file: &str,
        done: u64,
        total: Option<u64>,
    ) {
        self.emit(
            tx,
            DownloadEvent::Progress { repo: repo.into(), file: file.into(), done, total },
        );
    }
}

/// 续传临时文件路径：`model.bin` → `model.bin.incomplete`（无扩展名同样处理）
fn incomplete_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|s| s.to_os_string()).unwrap_or_default();
    name.push(".incomplete");
    target.with_file_name(name)
}

/// 从响应解析文件总长：206 → Content-Range 尾段；200 → Content-Length
fn parse_total(resp: &reqwest::blocking::Response) -> Option<u64> {
    if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let cr = resp.headers().get("Content-Range")?.to_str().ok()?;
        return cr.rsplit('/').next()?.parse().ok();
    }
    resp.content_length()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_path_suffixes_file_name() {
        assert_eq!(
            incomplete_path(Path::new("/x/model.bin")),
            Path::new("/x/model.bin.incomplete")
        );
        assert_eq!(
            incomplete_path(Path::new("/x/tokens.txt")),
            Path::new("/x/tokens.txt.incomplete")
        );
        assert_eq!(incomplete_path(Path::new("/x/model")), Path::new("/x/model.incomplete"));
    }

    #[test]
    fn snapshot_layouts_match_cache_probe() {
        let dir = std::env::temp_dir().join(format!("lt_dl_layout_{}", std::process::id()));
        let d = Downloader::new(&dir, ProxyMode::None);
        // MS 布局
        let ms = d.snapshot_dir(Hub::Ms, "iic/SenseVoiceSmall");
        assert!(ms.ends_with(r"modelscope\models\iic--SenseVoiceSmall\snapshots\master")
            || ms.ends_with("modelscope/models/iic--SenseVoiceSmall/snapshots/master"));
        // HF 布局
        let hf = d.snapshot_dir(Hub::Hf, "ggml-org/whisper-tiny");
        let s = hf.to_string_lossy().replace('\\', "/");
        assert!(s.ends_with("huggingface/hub/models--ggml-org--whisper-tiny/snapshots/main"), "{s}");
        // 与 cache 探测互通：目录创建后 ms_model_path 应命中同一处
        std::fs::create_dir_all(&ms).unwrap();
        assert_eq!(crate::cache::ms_model_path(&dir, "iic/SenseVoiceSmall"), ms);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_builders() {
        let d = Downloader::new("m", ProxyMode::None).with_hf_endpoint("https://hf-mirror.com");
        assert_eq!(
            d.file_url(Hub::Hf, "a/b", "model.onnx"),
            "https://hf-mirror.com/a/b/resolve/main/model.onnx"
        );
        assert_eq!(
            d.file_url(Hub::Ms, "iic/SenseVoiceSmall", "tokens.txt"),
            "https://modelscope.cn/api/v1/models/iic/SenseVoiceSmall/repo?Revision=master&FilePath=tokens.txt"
        );
    }
}
