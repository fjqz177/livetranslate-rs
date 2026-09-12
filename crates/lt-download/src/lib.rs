//! 双 hub 模型下载器（PLAN §2.10；原版 hf-hub/modelscope SDK 的自控替代，M-01；
//! DL-2 改造见 docs/archive/download-overhaul.md；架构 2.0 W6 自 lt-models 分家）。
//!
//! - 布局自控（与 lt-models 缓存探测共用 lt_proto::layout 单一事实源）：
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// 下载目标 hub（W6 上移 lt-proto：布局拼装与下载器两 crate 共用）
pub use lt_proto::Hub;

/// 下载源尝试链（DL-5，落地 D-21「所选 hub 优先，缺失回落另一 hub」）：
/// 所选 hub 居首；always_hf 模型恒 HF 居首——D-21 后 always_hf 语义 =
/// 「HF 优先」而非「HF 唯一」。只含实际有仓库的源。
///
/// WD-4 实测登记（2026-09-09）：whisper 仍为单源（hub_ms=None）——
/// ModelScope 无 ggml whisper.cpp 镜像仓库（ggingganov/whisper.cpp 与
/// modelscope 系列候选均经 API 实测 404，搜索面无结果）；其「MS 镜像」
/// 能力由 [`hf_endpoint_for`] 端点机制承接：用户 hub=ms 时 HF 尝试走
/// hf-mirror.com（HEAD 实测可达）→ 下载链路同样可通，单链 + 端点随所选
/// hub = whisper 双源语义的现状完成态。
pub fn hub_chain<'a>(
    hub: Hub,
    hub_hf: Option<&'a str>,
    hub_ms: Option<&'a str>,
    always_hf: bool,
) -> Vec<(Hub, &'a str)> {
    let chosen = if always_hf || hub == Hub::Hf {
        (Hub::Hf, hub_hf)
    } else {
        (Hub::Ms, hub_ms)
    };
    let other = if chosen.0 == Hub::Hf {
        (Hub::Ms, hub_ms)
    } else {
        (Hub::Hf, hub_hf)
    };
    [(chosen.0, chosen.1), (other.0, other.1)]
        .into_iter()
        .filter_map(|(h, r)| r.map(|r| (h, r)))
        .collect()
}

/// HF 官方端点（用户显式选 hub=hf 时的直连源）
pub const HF_OFFICIAL_ENDPOINT: &str = "https://huggingface.co";
/// HF 国内镜像（D-24：用户选 hub=ms 而 HF 成为实际下载路径时自动启用——
/// 涵盖「模型无 MS 源的直接回退」与「MS 尝试失败后的回落」两种情形）
pub const HF_MIRROR_ENDPOINT: &str = "https://hf-mirror.com";

/// 本轮下载所选 hub → 其中 HF 尝试所用的端点（D-24 r2.1：端点随所选 hub，
/// 不设用户设置键；MS 模式选镜像因选 MS 即国内直连场景）。
pub fn hf_endpoint_for(selected: Hub) -> &'static str {
    match selected {
        Hub::Hf => HF_OFFICIAL_ENDPOINT,
        Hub::Ms => HF_MIRROR_ENDPOINT,
    }
}

/// 代理三模式（E2/D-79 迁入 lt-proto 契约层——settings.download_proxy 与
/// Cmd::StartDownload 载荷共用值域；本 crate re-export 保持兼容）
pub use lt_proto::ProxyMode;

/// 下载事件（→ 日志窗 / 下载对话框）
#[derive(Debug, Clone)]
pub enum DownloadEvent {
    /// 单文件进度（total 未知时为 None；k/n = 当前第 k 个文件/清单共 n 个，DL-3）
    Progress {
        repo: String,
        file: String,
        k: usize,
        n: usize,
        done: u64,
        total: Option<u64>,
    },
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

/// R24 退避轮询粒度：cancel 检查间隔（sleep 型退避拦截取消的最坏延迟）
const BACKOFF_TICK: Duration = Duration::from_millis(200);

// ── 失败分类（DL-2 类型化；W2 迁入 lt-proto 成为契约——Display 前缀
//    `[net]` 等保留仅为人读日志形态，UI 分流不再依赖字符串还原）──

/// 失败分类（契约权威在 lt-proto；`DlError.kind` 原样透传给
/// `UiEvent::DownloadFailed{kind,..}`，跨 crate 不再经字符串前缀往返）
pub use lt_proto::DownloadFailKind as FailKind;

/// 下载错误（Display = `"[{前缀}] {message}"`，与既有失败前缀契约兼容）
#[derive(Debug)]
pub struct DlError {
    pub kind: FailKind,
    message: String,
}

impl DlError {
    fn new(kind: FailKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// 原始错误串（无 `[前缀]`；UiEvent::DownloadFailed.message 的直供源）
    pub fn message(&self) -> &str {
        &self.message
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

/// 单文件下载任务参数（DL-3/DL-4 增补文件序号与取消令牌后聚合，避免参数列车）
struct FileJob<'a> {
    repo: &'a str,
    file: &'a str,
    target: &'a Path,
    incomplete: &'a Path,
    /// 第 k/n 个文件（1 起）
    k: usize,
    n: usize,
    /// sha256 十六进制（空串 = 注册表未登记，跳过内容校验；AH-5/H8）
    sha256: &'a str,
}

/// 下载清单项：`(文件路径, 字节数下限, sha256 十六进制)`。
/// sha256 空串 = 渐进登记未完成，仅做长度/下限校验（AH-5/DEC-4）
pub type FileSpec<'a> = (&'a str, u64, &'a str);

/// 单次读/连接的默认预算（**逐次 read 语义**，非总传输期限）。
///
/// 2026-09-10 显式化（D-83 评审勘误）：reqwest 阻塞客户端的 `timeout` 是
/// **逐次 read** 预算——`blocking/response.rs` 对每次 `body.read()` 包一层
/// `wait::timeout(.., timeout)`，默认 30s；大文件的总耗时不受其约束，但
/// "响应头到达后对端沉默"会在 ≤30s 内判死。实测：mock 沉默服务器下
/// 4 次尝试 ×（30s + 1/4/16s 退避）= 141s 收敛返回 `[net]`（见
/// tests/download_integration.rs 的 stalled_stream_times_out_and_retries）。
/// 显式写死防将来被"顺手去掉"（去掉即回退成永久挂起 + 取消失效）。
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Downloader {
    models_dir: PathBuf,
    proxy: ProxyMode,
    /// HF endpoint 可覆写为镜像（如 https://hf-mirror.com）
    hf_endpoint: String,
    /// ModelScope API 根（测试可指向本地 mock）
    ms_endpoint: String,
    /// 逐次读/连接预算（D-83；默认 30s，测试可注入短值）
    io_timeout: Duration,
}

impl Downloader {
    pub fn new(models_dir: impl Into<PathBuf>, proxy: ProxyMode) -> Self {
        Self {
            models_dir: models_dir.into(),
            proxy,
            hf_endpoint: "https://huggingface.co".into(),
            ms_endpoint: "https://modelscope.cn".into(),
            io_timeout: DEFAULT_IO_TIMEOUT,
        }
    }

    /// 覆写逐次读/连接预算（D-83 回归测试用：卡流场景注入短超时，
    /// 把 141s 的实测收敛压到秒级；生产恒为 [`DEFAULT_IO_TIMEOUT`]）
    pub fn with_io_timeout(mut self, timeout: Duration) -> Self {
        self.io_timeout = timeout;
        self
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
            Hub::Ms => lt_proto::layout::hf_style_snapshot(&self.models_dir, hub, repo, "master"),
            Hub::Hf => lt_proto::layout::hf_style_snapshot(&self.models_dir, hub, repo, "main"),
        }
    }

    /// 单文件 URL
    pub fn file_url(&self, hub: Hub, repo: &str, path: &str) -> String {
        match hub {
            Hub::Hf => hf::file_url(&self.hf_endpoint, repo, path),
            Hub::Ms => ms::file_url(&self.ms_endpoint, repo, path),
        }
    }

    /// 构造 HTTP 客户端（代理三模式；连接超时 10s，逐次读预算见
    /// [`DEFAULT_IO_TIMEOUT`]——显式设置，勿删，见该常量文档）
    pub fn http_client(&self) -> anyhow::Result<reqwest::blocking::Client> {
        let mut b = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // D-83：逐次 read 预算（静默对端 ≤30s 判死 → 走退避重试；
            // 取消也因此最多延迟一个预算生效，不会永久挂起）
            .timeout(self.io_timeout)
            .user_agent(concat!("livetranslate-rs/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(10))
            // AH-5/H7：模型为二进制文件无需自动解压——gzip 解码会让
            // Content-Length 与实际写入字节数脱节（total=None/长度误判来源）
            .no_gzip();
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
    /// `cancel` 置位即取消（DL-4/D-23）：在文件边界与读块检查点停止，
    /// 保留 `.incomplete` 续传现场，返回 `FailKind::Cancelled`。
    pub fn download_files(
        &self,
        hub: Hub,
        repo: &str,
        files: &[FileSpec<'_>],
        cancel: &AtomicBool,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<PathBuf> {
        let client = self.http_client()?;
        let dir = self.snapshot_dir(hub, repo);
        std::fs::create_dir_all(&dir).map_err(disk_err)?;
        let n = files.len();
        for (idx, (file, min_bytes, sha256)) in files.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::Error::new(DlError::new(
                    FailKind::Cancelled,
                    "已取消",
                )));
            }
            let target = dir.join(file);
            match skip_decision(&target, *min_bytes) {
                SkipDecision::Skip => {
                    self.emit(
                        tx,
                        DownloadEvent::Log(format!("[{repo}] 已存在，跳过 {file}")),
                    );
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
            let job = FileJob {
                repo,
                file,
                target: &target,
                incomplete: &incomplete_path(&target),
                k: idx + 1, // 从 1 起（UI 显示「第 k/n 个文件」）
                n,
                sha256,
            };
            self.download_one(&client, hub, &job, cancel, tx)?;
            self.emit(
                tx,
                DownloadEvent::FileDone {
                    repo: repo.into(),
                    file: (*file).into(),
                },
            );
        }
        self.emit(
            tx,
            DownloadEvent::Done {
                repo: repo.into(),
                dir: dir.clone(),
            },
        );
        Ok(dir)
    }

    /// 单文件下载（续传 + 重试）。
    fn download_one(
        &self,
        client: &reqwest::blocking::Client,
        hub: Hub,
        job: &FileJob,
        cancel: &AtomicBool,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> Result<(), DlError> {
        let (repo, file) = (job.repo, job.file);
        let mut last_err: Option<DlError> = None;
        for (attempt, backoff) in std::iter::once(Duration::ZERO).chain(BACKOFFS).enumerate() {
            if backoff > Duration::ZERO {
                let msg = format!("[{repo}] {file} 下载失败，{backoff:?} 后第 {attempt} 次重试");
                tracing::warn!("{msg}");
                self.emit(tx, DownloadEvent::Log(msg));
                // R24：退避 tick 化——连续 sleep 期间取消不响应（最长 16s）；
                // 改 200ms 轮询 cancel 检查，取消延迟 ≤ 1tick（方案验收 ≤1s）
                let deadline = Instant::now() + backoff;
                while Instant::now() < deadline {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(DlError::new(FailKind::Cancelled, "已取消"));
                    }
                    std::thread::sleep(BACKOFF_TICK);
                }
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(DlError::new(FailKind::Cancelled, "已取消"));
            }
            match self.try_download(client, hub, job, cancel, tx) {
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
        job: &FileJob,
        cancel: &AtomicBool,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> Result<(), DlError> {
        let (repo, file, target, incomplete) = (job.repo, job.file, job.target, job.incomplete);
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
                    self.emit(
                        tx,
                        DownloadEvent::Log(format!("[{repo}] {file} 续传偏移失效，重新下载")),
                    );
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
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(incomplete)
                    .map_err(disk_err)?
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
                if cancel.load(Ordering::Relaxed) {
                    // 保留 .incomplete 续传现场直接返回
                    return Err(DlError::new(FailKind::Cancelled, "已取消"));
                }
                let n = resp
                    .read(&mut buf)
                    .map_err(|e| DlError::new(FailKind::Net, format!("网络读取中断: {e}")))?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n]).map_err(disk_err)?;
                written += n as u64;
                if written - done >= PROGRESS_STEP {
                    self.progress(tx, job, written, total);
                    done = written;
                }
            }
            f.flush().map_err(disk_err)?;
            // 落盘后再 rename（DL-2/F4：掉电不留半截终版文件）
            f.sync_all().map_err(disk_err)?;

            // AH-5/H7：总长未知（无 Content-Length/Content-Range）→ 拒绝下载。
            // 字节长度是完整性主通道——收尾放行会造成「截断文件永久判已缓存」
            // 死局（total=None 时长度校验被整体跳过）
            let Some(total) = total else {
                return Err(DlError::new(
                    FailKind::Length,
                    "服务端未提供 Content-Length，拒绝下载收尾",
                ));
            };
            // 完整性：长度校验
            if written != total {
                return Err(DlError::new(
                    FailKind::Length,
                    format!("长度不完整 {written}/{total}"),
                ));
            }
            drop(f);
            // AH-5/H8：sha256 内容校验（finalize 前对 .incomplete 流式计算；
            // 不匹配删除现场快速失败——截断/镜像污染不再可能落为终版文件）
            if !job.sha256.is_empty() {
                let actual = hash_file_hex(incomplete).map_err(disk_err)?;
                if !actual.eq_ignore_ascii_case(job.sha256) {
                    let _ = std::fs::remove_file(incomplete);
                    return Err(DlError::new(
                        FailKind::Checksum,
                        format!("sha256 不匹配 actual={actual}"),
                    ));
                }
            }
            finalize_incomplete(incomplete, target)?;
            self.progress(tx, job, written, Some(total));
            return Ok(());
        }
    }

    fn emit(&self, tx: Option<&Sender<DownloadEvent>>, ev: DownloadEvent) {
        if let Some(tx) = tx {
            let _ = tx.send(ev);
        }
    }

    /// 按「所选 hub 优先，缺失/不可达回落另一 hub」（DL-5，D-21 机制半）下载
    /// 整个模型。`chain` 由 [`hub_chain`] 生成；回落仅仓库级一次，且仅对
    /// 404/网络类错误（[`FailKind::fallback_candidate`]）生效。
    pub fn download_model(
        &self,
        chain: &[(Hub, &str)],
        files: &[FileSpec<'_>],
        cancel: &AtomicBool,
        tx: Option<&Sender<DownloadEvent>>,
    ) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(!chain.is_empty(), "该模型无可用下载源");
        let mut last_err: Option<anyhow::Error> = None;
        for (i, (hub, repo)) in chain.iter().enumerate() {
            match self.download_files(*hub, repo, files, cancel, tx) {
                Ok(dir) => return Ok(dir),
                Err(e) => {
                    let kind = e.downcast_ref::<DlError>().map(|d| d.kind);
                    let can_fallback =
                        i + 1 < chain.len() && kind.is_some_and(|k| k.fallback_candidate());
                    if can_fallback {
                        let msg = format!("[{repo}] 下载源不可用（{e:#}），回落下一下载源…");
                        tracing::warn!("{msg}");
                        self.emit(tx, DownloadEvent::Log(msg));
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }
        Err(last_err.unwrap())
    }

    fn progress(
        &self,
        tx: Option<&Sender<DownloadEvent>>,
        job: &FileJob,
        done: u64,
        total: Option<u64>,
    ) {
        self.emit(
            tx,
            DownloadEvent::Progress {
                repo: job.repo.into(),
                file: job.file.into(),
                k: job.k,
                n: job.n,
                done,
                total,
            },
        );
    }
}

/// 续传临时文件路径：`model.bin` → `model.bin.incomplete`（无扩展名同样处理）
fn incomplete_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
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

/// 文件 sha256（十六进制小写；AH-5/H8 内容校验用，流式读避免大文件驻留）。
/// D-83 起公开：零信任加载闸门（lt-orchestrator `trust_gate`）复用同一实现，
/// 保证"下载期校验"与"加载前校验"是同一套哈希语义。
pub fn hash_file_hex(path: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 内容是否与登记指纹一致（十六进制、大小写不敏感；D-83 零信任闸门用）。
/// `expected_hex` 为空串 = 未登记 → 恒真空通过（与下载器的跳过语义一致：
/// 渐进登记期的条目不做内容校验）。
pub fn verify_file(path: &Path, expected_hex: &str) -> std::io::Result<bool> {
    if expected_hex.is_empty() {
        return Ok(true);
    }
    Ok(hash_file_hex(path)?.eq_ignore_ascii_case(expected_hex))
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

    /// DL-5：下载源尝试链——所选 hub 优先；always_hf 恒 HF 居首；
    /// 无仓源被剔除（whisper ms=None → 单源不回落）
    #[test]
    fn hub_chain_selected_first_with_fallback() {
        let hf = Some("a/b");
        let ms = Some("c/d");
        assert_eq!(
            hub_chain(Hub::Ms, hf, ms, false),
            vec![(Hub::Ms, "c/d"), (Hub::Hf, "a/b")]
        );
        assert_eq!(
            hub_chain(Hub::Hf, hf, ms, false),
            vec![(Hub::Hf, "a/b"), (Hub::Ms, "c/d")]
        );
        // whisper 现状（always_hf 且无 MS 镜像）→ 单源
        assert_eq!(
            hub_chain(Hub::Ms, Some("g/whisper.cpp"), None, true),
            vec![(Hub::Hf, "g/whisper.cpp")]
        );
        // WD-4 登记镜像后：HF 居首 + MS 回落（链序即未来行为）
        assert_eq!(
            hub_chain(Hub::Hf, Some("g/whisper.cpp"), Some("mirror/whisper"), true),
            vec![(Hub::Hf, "g/whisper.cpp"), (Hub::Ms, "mirror/whisper")]
        );
    }

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
        assert!(matches!(
            skip_decision(&target, 1_000),
            SkipDecision::Missing
        ));
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
        let files: &[FileSpec] = &[("m.bin", 1_000, ""), ("tokens.txt", 10, "")];
        let snap = d.snapshot_dir(Hub::Hf, "a/b");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("m.bin"), vec![0u8; 2_000]).unwrap();
        std::fs::write(snap.join("tokens.txt"), vec![0u8; 16]).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let got = d
            .download_files(Hub::Hf, "a/b", files, &AtomicBool::new(false), Some(&tx))
            .expect("足额应全跳过并成功");
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
        assert!(
            logs.iter().any(|s| s.contains("跳过 tokens.txt")),
            "{logs:?}"
        );
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
        assert_eq!(
            incomplete_path(Path::new("/x/model")),
            Path::new("/x/model.incomplete")
        );
    }

    #[test]
    fn snapshot_layouts_match_cache_probe() {
        let dir = std::env::temp_dir().join(format!("lt_dl_layout_{}", std::process::id()));
        let d = Downloader::new(&dir, ProxyMode::None);
        // MS 布局
        let ms = d.snapshot_dir(Hub::Ms, "iic/SenseVoiceSmall");
        assert!(
            ms.ends_with(r"modelscope\models\iic--SenseVoiceSmall\snapshots\master")
                || ms.ends_with("modelscope/models/iic--SenseVoiceSmall/snapshots/master")
        );
        // HF 布局
        let hf = d.snapshot_dir(Hub::Hf, "ggml-org/whisper-tiny");
        let s = hf.to_string_lossy().replace('\\', "/");
        assert!(
            s.ends_with("huggingface/hub/models--ggml-org--whisper-tiny/snapshots/main"),
            "{s}"
        );
        // 与缓存探测互通：目录创建后 ms_model_path 应命中同一处
        std::fs::create_dir_all(&ms).unwrap();
        assert_eq!(
            lt_models::cache::ms_model_path(&dir, "iic/SenseVoiceSmall"),
            ms
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hf_endpoint_follows_selected_hub() {
        // D-24 r2.1：选 HF=官方直连；选 MS=HF 尝试自动走镜像
        assert_eq!(hf_endpoint_for(Hub::Hf), HF_OFFICIAL_ENDPOINT);
        assert_eq!(hf_endpoint_for(Hub::Ms), HF_MIRROR_ENDPOINT);
        assert_eq!(hf_endpoint_for(Hub::Ms), "https://hf-mirror.com");
    }

    /// D-83：哈希/校验公开接口——命中/不命中/大小写不敏感/空串恒真（未登记）
    #[test]
    fn hash_and_verify_file_semantics() {
        let dir = std::env::temp_dir().join(format!("lt_dl_hash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("m.bin");
        std::fs::write(&f, b"0123456789abcdef").unwrap();
        // 已知向量（sha256("0123456789abcdef")，独立工具实测：
        // `printf 0123456789abcdef | sha256sum` → 9f9f5111…929f）
        let want = "9f9f5111f7b27a781f1f1ddde5ebc2dd2b796bfc7365c9c28b548e564176929f";
        let got = hash_file_hex(&f).unwrap();
        assert_eq!(got, want, "哈希实现必须与独立实现逐位一致");
        assert!(verify_file(&f, want).unwrap(), "同值应通过");
        assert!(
            verify_file(&f, &want.to_uppercase()).unwrap(),
            "十六进制大小写不敏感"
        );
        assert!(!verify_file(&f, "0".repeat(64).as_str()).unwrap());
        // 空串 = 未登记 → 通过（渐进登记期语义，与下载器跳过一致）
        assert!(verify_file(&f, "").unwrap());
        // 缺失文件 → Err（调用方按"不可信"处理）
        assert!(verify_file(&dir.join("nope.bin"), want).is_err());
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

/// WP-A 演练探针（临时，不入常规测试面）：经 hf-mirror 用真实下载器把 nano
/// 官方包（~1GB）拉到真实缓存，验证直链/续传/manifest 全链路并测速。
/// 运行：cargo test -p lt-download probe_nano_download -- --ignored --nocapture
#[cfg(test)]
mod probe_nano_tmp {
    use super::{DownloadEvent, Downloader, FileSpec, Hub, ProxyMode};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    const NANO_REPO: &str = "csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30";
    /// 探针清单从注册表派生（AH-5：sha256 全量登记，下载时顺带内容校验）
    fn nano_specs() -> Vec<FileSpec<'static>> {
        let e = lt_models::registry::FUNASR_NANO.clone();
        e.files
            .iter()
            .copied()
            .zip(e.files_min_bytes.iter().copied())
            .zip(e.files_sha256.iter().copied())
            .map(|((f, min), sha)| (f, min, sha))
            .collect()
    }

    #[test]
    #[ignore = "真实网络下载 ~1GB（hf-mirror，写真实缓存）；WP-A 演练/测速用"]
    fn probe_nano_download_via_hf_mirror() {
        // docs/archive/path-hygiene.md PH-1：路径走 paths 派生（env 可重定向，默认真实缓存）
        let md = lt_models::paths::models_dir(None).expect("models_dir 解析失败");
        let dl = Downloader::new(&md, ProxyMode::None).with_hf_endpoint("https://hf-mirror.com");
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let t0 = Instant::now();
        let worker = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                dl.download_model(&[(Hub::Hf, NANO_REPO)], &nano_specs(), &cancel, Some(&tx))
            })
        };
        let mut last_pct = [0usize; 6];
        while let Ok(ev) = rx.recv() {
            match ev {
                DownloadEvent::Progress {
                    file,
                    k,
                    n,
                    done,
                    total,
                    ..
                } => {
                    let (tot, pct) = (total.unwrap_or(0), |d: u64, t: u64| {
                        // checked 链：t==0 或 d*100 溢出 → 0（进度不推进）
                        d.checked_mul(100)
                            .and_then(|x| x.checked_div(t))
                            .map_or(0, |v| v as usize)
                    });
                    let p = pct(done, tot);
                    if p >= last_pct[k - 1] + 25 || done == tot {
                        last_pct[k - 1] = p;
                        println!(
                            "[{k}/{n}] {file}: {done}/{tot} ({p}%)  t={:?}",
                            t0.elapsed()
                        );
                    }
                }
                DownloadEvent::FileDone { file, .. } => {
                    println!("✓ FileDone {file}  t={:?}", t0.elapsed())
                }
                DownloadEvent::Log(m) => println!("… {m}"),
                DownloadEvent::Done { dir, .. } => {
                    println!("★ Done → {dir:?}  t={:?}", t0.elapsed())
                }
            }
        }
        let snapshot = worker.join().expect("下载线程 panic").expect("下载失败");
        let files: Vec<&str> = nano_specs().iter().map(|(f, _, _)| *f).collect();
        let mins: Vec<u64> = nano_specs().iter().map(|(_, m, _)| *m).collect();
        assert!(
            lt_models::cache::dir_has_manifest(&snapshot, &files, &mins),
            "manifest 完整性复核失败"
        );
        let total: u64 = files
            .iter()
            .map(|f| std::fs::metadata(snapshot.join(f)).unwrap().len())
            .sum();
        let el = t0.elapsed();
        println!(
            "snapshot = {snapshot:?}\n六件实测合计 = {total} bytes，总耗时 {el:?}，均速 {:.1} MB/s",
            total as f64 / 1_048_576.0 / el.as_secs_f64()
        );
        assert!(!cancel.load(Ordering::Relaxed));
    }
}

/// whisper 档位真实下载探针：经 hf-mirror 用真实下载器 + 注册表清单（含
/// sha256）下载一档 ggml 量化文件，验证「resolve 直链 → 流式写入 → 长度校验
/// → sha256 内容校验 → rename 收尾 → manifest 探测」全链，并对产物**独立
/// 复算**内容哈希与注册表登记值互证（不用被测代码自证）。
/// 档位经 `LT_WHISPER_SIZE`（默认 base）；落盘位置 = `paths::models_dir`，
/// 建议经 `LIVETRANSLATE_CONFIG_DIR` 指向临时目录，避免写真实缓存。
/// 运行：`LT_WHISPER_SIZE=base cargo test -p lt-download probe_whisper -- --ignored --nocapture`
#[cfg(test)]
mod probe_whisper_tmp {
    use super::{hub_chain, DownloadEvent, Downloader, FileSpec, Hub, ProxyMode};
    use std::io::Read;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// 独立复算（不复用下载器内部 hash_file_hex——探针的意义是与实现互证）
    fn sha256_hex(path: &std::path::Path) -> String {
        use sha2::Digest;
        let mut f = std::fs::File::open(path).expect("打开下载产物");
        let mut h = sha2::Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).expect("读取下载产物");
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        format!("{:x}", h.finalize())
    }

    #[test]
    #[ignore = "真实网络下载（hf-mirror，写 models_dir）；whisper 档位全链路 + sha256 实证用"]
    fn probe_whisper_download_via_hf_mirror() {
        let size = std::env::var("LT_WHISPER_SIZE").unwrap_or_else(|_| "base".into());
        let entry = lt_models::registry::whisper_entry_for(&size).expect("合法 whisper 档位");
        let md = lt_models::paths::models_dir(None).expect("models_dir 解析失败");
        // 端点与生产 hub=ms 路径同源（hf_endpoint_for(Hub::Ms) = hf-mirror）
        let dl = Downloader::new(&md, ProxyMode::None).with_hf_endpoint(super::HF_MIRROR_ENDPOINT);
        let specs: Vec<FileSpec> = entry
            .files
            .iter()
            .copied()
            .zip(entry.files_min_bytes.iter().copied())
            .zip(entry.files_sha256.iter().copied())
            .map(|((f, min), sha)| (f, min, sha))
            .collect();
        let chain = hub_chain(Hub::Ms, entry.hf, entry.ms, entry.always_hf);
        println!(
            "models_dir = {}\n下载 {size}：chain={chain:?}\nspecs={specs:?}",
            md.display()
        );

        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let t0 = Instant::now();
        let worker = {
            let cancel = cancel.clone();
            std::thread::spawn(move || dl.download_model(&chain, &specs, &cancel, Some(&tx)))
        };
        while let Ok(ev) = rx.recv() {
            match ev {
                DownloadEvent::Progress {
                    file, done, total, ..
                } => println!("… {file}: {done}/{:?}  t={:?}", total, t0.elapsed()),
                DownloadEvent::Log(m) => println!("… {m}"),
                DownloadEvent::FileDone { file, .. } => {
                    println!("✓ FileDone {file}  t={:?}", t0.elapsed())
                }
                DownloadEvent::Done { dir, .. } => {
                    println!("★ Done → {dir:?}  t={:?}", t0.elapsed())
                }
            }
        }
        let snapshot = worker
            .join()
            .expect("下载线程 panic")
            .expect("下载失败（含校验）");
        let el = t0.elapsed();

        let path = snapshot.join(entry.files[0]);
        let len = std::fs::metadata(&path).expect("产物存在").len();
        let actual = sha256_hex(&path);
        println!(
            "产物 = {path:?}\nlen = {len}（注册表 estimated_bytes = {}）\nsha256 = {actual}\n注册表登记 = {}\n耗时 {el:?}，均速 {:.1} MB/s",
            entry.estimated_bytes,
            entry.files_sha256[0],
            len as f64 / 1_048_576.0 / el.as_secs_f64()
        );
        // 独立复算 == 注册表登记值：whisper 档位 sha256 的实机凭据
        assert_eq!(
            actual, entry.files_sha256[0],
            "产物内容与注册表 sha256 不一致（登记值有误或镜像内容被改写）"
        );
        assert_eq!(
            len, entry.estimated_bytes,
            "实测字节数与注册表 estimated_bytes 不一致（上游换档或登记值漂移）"
        );
        assert!(
            lt_models::cache::dir_has_manifest(&snapshot, entry.files, entry.files_min_bytes),
            "manifest 完整性复核失败"
        );
    }
}
