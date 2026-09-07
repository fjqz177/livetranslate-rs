//! 双 hub 模型下载器（PLAN §2.10；原版 hf-hub/modelscope SDK 的自控替代，M-01；
//! DL-2 改造见 docs/download-overhaul.md）。
//!
//! - 布局自控（写入与 [`crate::cache`] 探测共用同一约定）：
//!   MS → `modelscope/models/{org}--{name}/snapshots/master/{path}`
//!   HF → `huggingface/hub/models--{org}--{name}/snapshots/main/{path}`
//! - 断点续传：先写 `{file}.incomplete`（带 Range 续传），完成 sync + rename；
//!   服务端忽略 Range 返 200 时从头重写，416 时删除陈旧 .incomplete 重下。
//! - 清单带字节数下限：已存在且达下限才跳过，半截残留自动重下（DL-2/F4）。
//! - 快速失败：404/401 等 4xx 立即返回不退避；net/length/5xx/429 才重试
//!   3 次指数退避（1s/4s/16s）（DL-2/F5）；代理三模式；进度事件走 channel。
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

// ── 失败分类（DL-2 类型化；Display 仍带 `[net] `/`[http] `/`[http-404] ` 等前缀，
//    契约不变：DownloadFailed 仍是 String，UI 端按前缀分流提示，未知/无前缀回落
//    第 3 类）──

/// 失败分类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    /// 网络层失败（reqwest：超时/连接拒绝/DNS/TLS/读取中断）
    Net,
    /// HTTP 状态码非 2xx（含 404 仓库缺失、限流、服务端错误）
    Http(u16),
    /// 磁盘/IO 失败（创建目录、写入、rename）
    Disk,
    /// 长度校验失败（下载不完整）
    Length,
    /// 用户取消（保留 .incomplete 续传现场；DL-4）
    Cancelled,
}

impl FailKind {
    pub fn prefix(self) -> &'static str {
        match self {
            FailKind::Net => "net",
            FailKind::Http(404) => "http-404",
            FailKind::Http(_) => "http",
            FailKind::Disk => "disk",
            FailKind::Length => "length",
            FailKind::Cancelled => "cancel",
        }
    }

    /// 是否值得退避重试（DL-2/F5 快速失败）：网络中断与长度不完整可续传重试；
    /// 5xx/429 属服务端暂时性；其余 4xx（404 缺失/401 私有/403 禁止）、磁盘、
    /// 取消均为永久态，立即返回不再白等 1/4/16s。
    pub fn retryable(self) -> bool {
        match self {
            FailKind::Net | FailKind::Length => true,
            FailKind::Http(s) => s >= 500 || s == 429,
            FailKind::Disk | FailKind::Cancelled => false,
        }
    }

    /// 是否值得换另一 hub 回落（DL-5）：仓库缺失或网络不可达才回落；
    /// 磁盘/长度/取消等问题换源无解。
    pub fn fallback_candidate(self) -> bool {
        matches!(self, FailKind::Net | FailKind::Http(404))
    }
}

/// 下载错误（Display = `"[{前缀}] {message}"`，与既有失败前缀契约兼容）
#[derive(Debug)]
pub struct DlError {
    pub kind: FailKind,
    message: String,
}

impl DlError {
    fn new(kind: FailKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind.prefix(), self.message)
    }
}

impl std::error::Error for DlError {}

/// 磁盘/IO 错误 → 带 [disk] 前缀
fn disk_err(e: std::io::Error) -> DlError {
    DlError::new(FailKind::Disk, format!("磁盘 I/O 失败: {e}"))
}

/// 网络错误 → 带 [net] 前缀
fn net_err(e: reqwest::Error) -> DlError {
    DlError::new(FailKind::Net, format!("网络请求失败: {e}"))
}

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
    /// 清单为 `(文件名, 字节数下限)`：已存在且达下限 → 跳过（幂等）；
    /// 存在但不足（半截/损坏残留）→ 删除重下（DL-2/F4）。
    pub fn download_files(
        &self,
        hub: Hub,
        repo: &str,
        files: &[(&str, u64)],
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<PathBuf> {
        let client = self.http_client()?;
        let dir = self.snapshot_dir(hub, repo);
        std::fs::create_dir_all(&dir).map_err(disk_err)?;
        for (file, min_bytes) in files {
            let target = dir.join(file);
            match skip_decision(&target, *min_bytes) {
                SkipDecision::Skip => {
                    self.emit(tx, DownloadEvent::Log(format!("[{repo}] 已存在，跳过 {file}")));
                    continue;
                }
                SkipDecision::Redo { have } => {
                    self.emit(
                        tx,
                        DownloadEvent::Log(format!(
                            "[{repo}] {file} 已存在但尺寸异常（{have} < {min_bytes}），重新下载"
                        )),
                    );
                    let _ = std::fs::remove_file(&target);
                }
                SkipDecision::Missing => {}
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
    ) -> Result<(), DlError> {
        let incomplete = incomplete_path(target);
        let mut last_err: Option<DlError> = None;
        for (attempt, backoff) in std::iter::once(Duration::ZERO).chain(BACKOFFS).enumerate() {
            if backoff > Duration::ZERO {
                let msg = format!("[{repo}] {file} 下载失败，{backoff:?} 后第 {attempt} 次重试");
                tracing::warn!("{msg}");
                self.emit(tx, DownloadEvent::Log(msg));
                std::thread::sleep(backoff);
            }
            match self.try_download(client, hub, repo, file, target, &incomplete, tx) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // DL-2/F5 快速失败：永久性错误（404 仓库缺失/401 私有/磁盘）不再退避
                    if !e.kind.retryable() {
                        return Err(e);
                    }
                    last_err = Some(e);
                }
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
    ) -> Result<(), DlError> {
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(disk_err)?;
        }
        let mut done: u64 = incomplete.metadata().map(|m| m.len()).unwrap_or(0);

        // 外层处理"Range 被拒（416）"的一次性回退
        loop {
            let url = self.file_url(hub, repo, file);
            let mut req = client.get(&url);
            if done > 0 {
                req = req.header("Range", format!("bytes={done}-"));
            }
            let resp = req.send().map_err(net_err)?;
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
                // 失败分类前缀（契约不变：DownloadFailed 仍是 String；UI 端按前缀分流提示）
                return Err(DlError::new(
                    FailKind::Http(status.as_u16()),
                    format!("HTTP {status}"),
                ));
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
                std::fs::OpenOptions::new().create(true).append(true).open(incomplete).map_err(disk_err)?
            } else {
                done = 0;
                let _ = std::fs::remove_file(incomplete);
                std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(incomplete)
                    .map_err(disk_err)?
            };

            let mut written = done;
            let mut buf = [0u8; 64 * 1024];
            let mut resp = resp;
            loop {
                let n = resp.read(&mut buf).map_err(|e| {
                    DlError::new(FailKind::Net, format!("网络读取中断: {e}"))
                })?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n]).map_err(disk_err)?;
                written += n as u64;
                if written - done >= PROGRESS_STEP {
                    self.progress(tx, repo, file, written, total);
                    done = written;
                }
            }
            f.flush().map_err(disk_err)?;
            // 落盘后再 rename（DL-2/F4：掉电不留半截终版文件）
            f.sync_all().map_err(disk_err)?;

            // 完整性：总长已知则校验
            if let Some(total) = total {
                if written != total {
                    return Err(DlError::new(
                        FailKind::Length,
                        format!("长度不完整 {written}/{total}"),
                    ));
                }
            }
            drop(f);
            finalize_incomplete(incomplete, target)?;
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

/// 跳过判定（DL-2/F4，独立纯函数便于离线测试）：达下限跳过；
/// 存在但不足（半截/损坏残留，如无 Content-Length 时的截断终版）→ 重下；
/// 不存在 → 下载。
#[derive(Debug)]
enum SkipDecision {
    Skip,
    Redo { have: u64 },
    Missing,
}

fn skip_decision(target: &Path, min_bytes: u64) -> SkipDecision {
    match std::fs::metadata(target) {
        Ok(m) if m.is_file() && m.len() >= min_bytes => SkipDecision::Skip,
        Ok(m) => SkipDecision::Redo { have: m.len() },
        Err(_) => SkipDecision::Missing,
    }
}

/// 收尾 rename（Windows 不覆盖已存在目标：双写竞态兜底移除，DL-2/F4）
fn finalize_incomplete(incomplete: &Path, target: &Path) -> Result<(), DlError> {
    if target.exists() {
        let _ = std::fs::remove_file(target);
    }
    std::fs::rename(incomplete, target).map_err(disk_err)
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

    /// DL-2/F5：重试分类表——net/length/5xx/429 可重试；404/401/403/磁盘/取消立即失败
    #[test]
    fn fail_kind_retry_classification() {
        assert!(FailKind::Net.retryable());
        assert!(FailKind::Length.retryable());
        assert!(FailKind::Http(500).retryable());
        assert!(FailKind::Http(503).retryable());
        assert!(FailKind::Http(429).retryable());
        assert!(!FailKind::Http(404).retryable());
        assert!(!FailKind::Http(401).retryable());
        assert!(!FailKind::Http(403).retryable());
        assert!(!FailKind::Http(418).retryable());
        assert!(!FailKind::Disk.retryable());
        assert!(!FailKind::Cancelled.retryable());
    }

    /// DL-5 前置：回落候选 = 仓库缺失或网络不可达；其余换源无解
    #[test]
    fn fail_kind_fallback_candidates() {
        assert!(FailKind::Http(404).fallback_candidate());
        assert!(FailKind::Net.fallback_candidate());
        assert!(!FailKind::Http(401).fallback_candidate());
        assert!(!FailKind::Http(500).fallback_candidate());
        assert!(!FailKind::Length.fallback_candidate());
        assert!(!FailKind::Disk.fallback_candidate());
        assert!(!FailKind::Cancelled.fallback_candidate());
    }

    /// 失败前缀契约不变（UI DownloadErrKind::parse 依赖字符串前缀分流）
    #[test]
    fn dl_error_display_keeps_prefix_contract() {
        let e = DlError::new(FailKind::Http(404), "HTTP 404 Not Found".to_string());
        assert_eq!(e.to_string(), "[http-404] HTTP 404 Not Found");
        let e = DlError::new(FailKind::Http(500), "HTTP 500".to_string());
        assert_eq!(e.to_string(), "[http] HTTP 500");
        let e = DlError::new(FailKind::Net, "网络请求失败: x".to_string());
        assert_eq!(e.to_string(), "[net] 网络请求失败: x");
        let e = DlError::new(FailKind::Disk, "磁盘 I/O 失败: 满".to_string());
        assert_eq!(e.to_string(), "[disk] 磁盘 I/O 失败: 满");
        let e = DlError::new(FailKind::Length, "长度不完整 1/2".to_string());
        assert_eq!(e.to_string(), "[length] 长度不完整 1/2");
        let e = DlError::new(FailKind::Cancelled, "已取消".to_string());
        assert_eq!(e.to_string(), "[cancel] 已取消");
    }

    /// DL-2/F4：跳过判定——足额跳过 / 半截重下 / 缺失下载
    #[test]
    fn skip_decision_validates_size() {
        let dir = std::env::temp_dir().join(format!("lt_dl_skip_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("m.bin");
        assert!(matches!(skip_decision(&target, 1_000), SkipDecision::Missing));
        std::fs::write(&target, vec![0u8; 2_000]).unwrap();
        assert!(matches!(skip_decision(&target, 1_000), SkipDecision::Skip));
        // 半截终版文件（无 Content-Length 下载残留/外部拷贝坏档）→ 重下
        std::fs::write(&target, vec![0u8; 10]).unwrap();
        match skip_decision(&target, 1_000) {
            SkipDecision::Redo { have } => assert_eq!(have, 10),
            other => panic!("应判重下，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DL-2/F4 端到端（零网络）：清单全部足额 → download_files 全跳过即成功
    ///（客户端构造不发请求，端点值无关紧要）
    #[test]
    fn download_files_all_complete_skips_without_network() {
        let dir = std::env::temp_dir().join(format!("lt_dl_skip_e2e_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = Downloader::new(&dir, ProxyMode::None).with_hf_endpoint("http://127.0.0.1:1");
        let files: &[(&str, u64)] = &[("m.bin", 1_000), ("tokens.txt", 10)];
        let snap = d.snapshot_dir(Hub::Hf, "a/b");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("m.bin"), vec![0u8; 2_000]).unwrap();
        std::fs::write(snap.join("tokens.txt"), vec![0u8; 16]).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let got = d.download_files(Hub::Hf, "a/b", files, Some(&tx)).expect("足额应全跳过并成功");
        assert_eq!(got, snap);
        let mut logs = Vec::new();
        let mut done = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                DownloadEvent::Log(s) => logs.push(s),
                DownloadEvent::Done { .. } => done = true,
                _ => {}
            }
        }
        assert!(done, "应发 Done 事件");
        assert!(logs.iter().any(|s| s.contains("跳过 m.bin")), "{logs:?}");
        assert!(logs.iter().any(|s| s.contains("跳过 tokens.txt")), "{logs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// DL-2/F4：收尾 rename 在目标已存在（双写竞态）时覆盖而非报错
    #[test]
    fn finalize_replaces_existing_target() {
        let dir = std::env::temp_dir().join(format!("lt_dl_fin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let incomplete = dir.join("m.bin.incomplete");
        let target = dir.join("m.bin");
        std::fs::write(&incomplete, b"new-content").unwrap();
        std::fs::write(&target, b"stale").unwrap();
        finalize_incomplete(&incomplete, &target).expect("竞态兜底应覆盖旧目标");
        assert_eq!(std::fs::read(&target).unwrap(), b"new-content");
        assert!(!incomplete.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

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
