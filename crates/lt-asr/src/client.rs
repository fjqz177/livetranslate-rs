//! AsrWorkerClient：worker 子进程的父端代理（原版 asr_client.py 1:1）。
//!
//! 状态机 created→starting→loading→ready⇄busy→…→stopped/failed/exited；
//! 超时表：ready 180s / transcribe·set_* 60s（AH-8：nano 语言切换重建识别器
//! 需重载模型，10s 慢机不足）/ shutdown 5s→kill（AH-2 兜底）。
//! 读响应以 0.2s 步进轮询，期间监测子进程退出（对齐原版 conn.poll + exitcode）。

use crate::frame::{FrameReader, FrameWriter, ReadyInfo, ReqKind, Request, Response};
use crate::job::JobHandle;
use crate::worker::WorkerConfig;
use lt_proto::AsrResult;
use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// 客户端可见的失败类型（与原版 ASRWorkerError/Timeout/Exited 对应）
#[derive(Debug, thiserror::Error)]
pub enum AsrClientError {
    /// worker 回了 ok=false；recoverable 语义与原版一致
    #[error("{message}")]
    Worker { message: String, recoverable: bool },
    #[error("ASR worker 响应超时（{0:.1}s）")]
    Timeout(f64),
    #[error("ASR worker 已退出: {0}")]
    Exited(String),
    #[error("ASR worker 状态错误: {0}")]
    Status(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

/// worker 状态（原版 _status 字符串集合）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Starting,
    Loading,
    Ready,
    Busy,
    Failed,
    Exited,
    Stopping,
    Stopped,
}

const POLL_STEP: Duration = Duration::from_millis(200);

pub struct AsrWorkerClient {
    child: Child,
    stdin: FrameWriter<std::process::ChildStdin>,
    resp_rx: Receiver<Result<Response, String>>,
    status: Status,
    pub config: WorkerConfig,
    ready_timeout: Duration,
    request_timeout: Duration,
    shutdown_timeout: Duration,
    /// 持有 Job 句柄：Drop 时 OS 收割子进程（E-05）
    _job: Option<JobHandle>,
}

impl AsrWorkerClient {
    /// 以当前 exe + `--asr-worker <config-json>` 启动 worker（生产路径）
    pub fn spawn(config: WorkerConfig) -> Result<Self, AsrClientError> {
        let exe = std::env::current_exe()
            .map_err(|e| AsrClientError::Status(format!("无法定位当前 exe: {e}")))?;
        Self::spawn_program(&exe, config)
    }

    /// 指定 worker 程序路径（测试注入假 worker）
    pub fn spawn_program(
        program: &std::path::Path,
        config: WorkerConfig,
    ) -> Result<Self, AsrClientError> {
        let cfg_json = serde_json::to_string(&config)
            .map_err(|e| AsrClientError::Status(format!("配置序列化失败: {e}")))?;
        let mut cmd = Command::new(program);
        cmd.arg("--asr-worker")
            .arg(&cfg_json)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()); // E-07：worker 输出重定向到日志，绝不弹控制台
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(AsrClientError::Io)?;

        // Job Object 绑定（失败仅记日志，不阻断——兜底机制）
        let job = match JobHandle::new().and_then(|j| {
            j.assign(&child)?;
            Ok(j)
        }) {
            Ok(j) => Some(j),
            Err(e) => {
                tracing::warn!("Job Object 绑定失败（孤儿兜底不可用）: {e}");
                None
            }
        };

        // stderr → tracing 线程
        if let Some(stderr) = child.stderr.take() {
            std::thread::Builder::new()
                .name("asr-worker-stderr".into())
                .spawn(move || drain_stderr(stderr))
                .ok();
        }

        let stdout: ChildStdout = child.stdout.take().expect("stdout piped");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("asr-worker-reader".into())
            .spawn(move || reader_loop(stdout, tx))
            .ok();

        let stdin = FrameWriter::new(child.stdin.take().expect("stdin piped"));
        tracing::info!("ASR worker 已启动 pid={}", child.id());
        Ok(Self {
            child,
            stdin,
            resp_rx: rx,
            status: Status::Starting,
            config,
            ready_timeout: Duration::from_secs(180),
            request_timeout: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(5),
            _job: job,
        })
    }

    pub fn status(&mut self) -> Status {
        if matches!(
            self.status,
            Status::Starting | Status::Loading | Status::Ready | Status::Busy
        ) && self.child.try_wait().map_or(false, |c| c.is_some())
        {
            self.status = Status::Exited;
        }
        self.status
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// 覆写请求超时（默认 60s；测试/低延迟场景注入）
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.request_timeout = timeout;
    }

    /// 覆写 ready 超时（默认 180s；测试注入）
    pub fn set_ready_timeout(&mut self, timeout: Duration) {
        self.ready_timeout = timeout;
    }

    /// 等待首帧 ready（装载失败 → Worker{recoverable:false}）
    pub fn wait_ready(&mut self) -> Result<ReadyInfo, AsrClientError> {
        self.status = Status::Loading;
        let resp = self.recv_response(self.ready_timeout, None)?;
        if !resp.ok {
            self.status = Status::Failed;
            return Err(resp_error(&resp));
        }
        match resp.kind {
            crate::frame::RespKind::Ready(info) => {
                self.status = Status::Ready;
                tracing::info!(
                    "ASR worker ready pid={} {}",
                    self.child.id(),
                    info.engine
                );
                Ok(info)
            }
            _ => {
                self.status = Status::Failed;
                Err(AsrClientError::Status(format!(
                    "worker 启动响应类型异常: {:?}",
                    resp.kind
                )))
            }
        }
    }

    pub fn transcribe(
        &mut self,
        audio: &[f32],
        word_timestamps: bool,
    ) -> Result<AsrResult, AsrClientError> {
        let resp = self.request(
            ReqKind::Transcribe { word_timestamps },
            audio,
            self.request_timeout,
        )?;
        match resp.kind {
            crate::frame::RespKind::Result(r) => Ok(r),
            _ => Err(AsrClientError::Status("transcribe 响应类型异常".into())),
        }
    }

    // AH-8/H16：set_* 超时与 transcribe 对齐（request_timeout=60s）——nano
    // set_language 重建识别器需重载 963MB（本机 4.15s），10s 预算慢机不足，
    // 超时即 kill+重启白烧配额；代价=真挂死时等待与识别同级（语义一致）
    pub fn set_language(&mut self, language: &str) -> Result<(), AsrClientError> {
        self.request(
            ReqKind::SetLanguage {
                language: language.into(),
            },
            &[],
            self.request_timeout,
        )
        .map(|_| ())
    }

    pub fn set_input_padding(&mut self, pad_seconds: f32) -> Result<(), AsrClientError> {
        self.request(
            ReqKind::SetInputPadding { pad_seconds },
            &[],
            self.request_timeout,
        )
        .map(|_| ())
    }

    /// 优雅停机：发 shutdown → 等 ack → join 超时 → kill（原版 shutdown 1:1）
    pub fn shutdown(&mut self) {
        if self.status == Status::Stopped || self.status == Status::Stopping {
            return;
        }
        self.status = Status::Stopping;
        let alive = self.child.try_wait().map_or(true, |c| c.is_none());
        if alive {
            let req = Request {
                id: uuid::Uuid::new_v4().to_string(),
                kind: ReqKind::Shutdown,
            };
            if self.stdin.write_request(&req, &[]).is_ok() {
                // 给 ack 一个短暂机会；超时/EOF 由后续 kill 兜底
                for _ in 0..(self.shutdown_timeout.as_millis() / 100).max(1) {
                    match self.resp_rx.recv_timeout(Duration::from_millis(100)) {
                        Ok(Ok(resp)) => {
                            if matches!(resp.kind, crate::frame::RespKind::Shutdown) {
                                break;
                            }
                        }
                        Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                }
            }
        }
        self.finish_stop();
        tracing::info!("ASR worker 已停止");
    }

    /// 立即终止（超时/致命路径）
    pub fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.status = Status::Failed;
    }

    /// 单条请求往返；transcribe 期间置 busy（结束恢复原状态）
    fn request(
        &mut self,
        kind: ReqKind,
        audio: &[f32],
        timeout: Duration,
    ) -> Result<Response, AsrClientError> {
        let st = self.status();
        if st != Status::Ready {
            // 预检刷新后已死（含"请求间隙死亡"：worker 在两次请求之间退出）→
            // 按 Exited 上抛，manager 侧 recover 接管（AH-2/D-26：原实现折成
            // Status 被上层归为用法错误，永不触发自动重启 → 永久僵尸态）
            if matches!(st, Status::Exited | Status::Failed) {
                return Err(AsrClientError::Exited(format!(
                    "预检发现 worker 非 Ready: {st:?}"
                )));
            }
            return Err(AsrClientError::Status(format!("worker 未就绪: {st:?}")));
        }
        let req = Request {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
        };
        self.stdin.write_request(&req, audio)?;
        let prev = self.status;
        if matches!(req.kind, ReqKind::Transcribe { .. }) {
            self.status = Status::Busy;
        }
        let resp = self.recv_response(timeout, Some(&req.id));
        if self.status == Status::Busy {
            self.status = prev;
        }
        let resp = resp?;
        if !resp.ok {
            return Err(resp_error(&resp));
        }
        Ok(resp)
    }

    /// 收响应：0.2s 步进轮询，间隙查子进程退出（原版 _recv_response 1:1）
    fn recv_response(
        &mut self,
        timeout: Duration,
        expected_id: Option<&str>,
    ) -> Result<Response, AsrClientError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                self.status = Status::Failed;
                let t = timeout.as_secs_f64();
                self.terminate();
                return Err(AsrClientError::Timeout(t));
            }
            let step = POLL_STEP.min(deadline - now);
            match self.resp_rx.recv_timeout(step) {
                Ok(Ok(resp)) => {
                    if let Some(want) = expected_id {
                        if resp.id.as_deref() != Some(want) {
                            // 非本请求的响应（协议错位）——原版直接抛错
                            self.status = Status::Failed;
                            return Err(AsrClientError::Status(format!(
                                "响应 id 不匹配: expected={want}, got={:?}",
                                resp.id
                            )));
                        }
                    }
                    return Ok(resp);
                }
                Ok(Err(e)) => {
                    // reader 线程报告流错误：进程多半已退出
                    self.status = Status::Exited;
                    self.reap();
                    return Err(AsrClientError::Exited(e));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.status = Status::Exited;
                    self.reap();
                    return Err(AsrClientError::Exited("reader 线程已退出".into()));
                }
                Err(RecvTimeoutError::Timeout) => {
                    // 间隙检查退出码（比傻等超时更快感知崩溃）
                    if let Some(code) = self.child.try_wait().ok().flatten() {
                        self.status = Status::Exited;
                        return Err(AsrClientError::Exited(format!("exit code {code}")));
                    }
                }
            }
        }
    }

    fn reap(&mut self) {
        let _ = self.child.try_wait();
    }

    fn finish_stop(&mut self) {
        // ack 窗口过后 kill 兜底（AH-2/H6：兑现 shutdown 文档承诺——worker 忙/
        // 滞留时不无界阻塞 child.wait()；已退出的进程 kill 为无害 no-op）。
        // 否则 UI 线程 Pipeline::stop join ASR 线程会整体挂死。
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.status = Status::Stopped;
    }
}

impl Drop for AsrWorkerClient {
    fn drop(&mut self) {
        // 未走 shutdown 的路径（含 panic 展开）：确保子进程不残留
        let alive = self.child.try_wait().map_or(true, |c| c.is_none());
        if alive {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn resp_error(resp: &Response) -> AsrClientError {
    match resp.error_info() {
        Some(e) => AsrClientError::Worker {
            message: e.message.clone(),
            recoverable: e.recoverable,
        },
        None => AsrClientError::Worker {
            message: "worker 返回失败（无错误详情）".into(),
            recoverable: false,
        },
    }
}

/// reader 线程：流式读响应帧；EOF/错误发 Err 并结束
fn reader_loop(mut stdout: ChildStdout, tx: std::sync::mpsc::Sender<Result<Response, String>>) {
    let mut reader = FrameReader::new(&mut stdout);
    loop {
        match reader.read_response() {
            Ok(Some(resp)) => {
                if tx.send(Ok(resp)).is_err() {
                    break;
                }
            }
            Ok(None) => break, // 干净 EOF
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                break;
            }
        }
    }
}

fn drain_stderr(mut stderr: impl Read) {
    // AH-7：`\r` 视为行界（C 库进度条式输出）；长期无换行时截断 pending，
    // 防无界增长（超长单行/二进制误判）
    const PENDING_CAP: usize = 1 << 20;
    let mut buf = [0u8; 4096];
    let mut pending = Vec::new();
    loop {
        match stderr.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(pos) = pending.iter().position(|&b| b == b'\n' || b == b'\r') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    let s = String::from_utf8_lossy(&line[..line.len() - 1]);
                    if !s.trim().is_empty() {
                        tracing::debug!(target: "asr_worker", "{s}");
                    }
                }
                if pending.len() > PENDING_CAP {
                    let s = String::from_utf8_lossy(&pending);
                    tracing::debug!(target: "asr_worker", "(stderr 超长行截断) {s}");
                    pending.clear();
                }
            }
        }
    }
}
