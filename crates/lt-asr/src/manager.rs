//! AsrManager：worker 生命周期的管理层（原版 main.py _run_asr/_recover_asr_worker/
//! _maybe_recycle_asr_worker 的语义 1:1）。
//!
//! - 成功 → error_count/restart_count 清零
//! - worker 回可恢复错误 → error_count+1；连续 ≥3 次或不可恢复 → 标记不可用
//! - 进程退出/超时 → 自动重启（≤3 次）；耗尽 → 标记不可用
//! - RSS 超出基线 +2048MB → 优雅回收（不占失败配额，E-06）
//! - 引擎配置变更 → 替换 worker（generation 语义：旧实例关闭，计数清零）

use crate::client::{AsrClientError, AsrWorkerClient};
use crate::worker::WorkerConfig;
use lt_proto::AsrResult;

/// 自动重启上限（原版 _asr_restart_max）
const RESTART_MAX: u32 = 3;
/// RSS 回收阈值：超出加载后基线这么多 MB 就回收（原版 _asr_recycle_delta_mb）
const RECYCLE_DELTA_MB: u64 = 2048;

/// 管理层错误（unavailable = 需要用户干预/换引擎；其余为单次失败）
#[derive(Debug, thiserror::Error)]
pub enum AsrManagerError {
    #[error("ASR 不可用: {0}")]
    Unavailable(String),
    #[error("{0}")]
    Failed(String),
}

impl AsrManagerError {
    pub fn unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

pub type Spawner = Box<dyn Fn(&WorkerConfig) -> Result<AsrWorkerClient, AsrClientError> + Send>;

pub struct AsrManager {
    client: Option<AsrWorkerClient>,
    config: Option<WorkerConfig>,
    restart_count: u32,
    error_count: u32,
    baseline_mb: Option<u64>,
    unavailable: bool,
    spawn: Spawner,
}

impl AsrManager {
    /// 生产构造：走 `当前exe --asr-worker`（与 AsrWorkerClient::spawn 相同）
    pub fn new() -> Self {
        Self::with_spawner(Box::new(|cfg| AsrWorkerClient::spawn(cfg.clone())))
    }

    /// 注入 spawner（测试用假 worker）
    pub fn with_spawner(spawn: Spawner) -> Self {
        Self {
            client: None,
            config: None,
            restart_count: 0,
            error_count: 0,
            baseline_mb: None,
            unavailable: false,
            spawn,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    pub fn is_ready(&self) -> bool {
        self.client.is_some() && !self.unavailable
    }

    fn spawn_ready(&mut self, config: &WorkerConfig) -> Result<(), AsrManagerError> {
        let mut client = (self.spawn)(config)
            .map_err(|e| AsrManagerError::Failed(format!("worker 启动失败: {e}")))?;
        client
            .wait_ready()
            .map_err(|e| {
                let _ = client.shutdown();
                AsrManagerError::Failed(format!("worker 未就绪: {e}"))
            })?;
        self.client = Some(client);
        self.config = Some(config.clone());
        self.baseline_mb = None; // 新 worker 重取基线
        Ok(())
    }

    /// 确保就绪：配置签名一致则复用；不可用后换新配置可复活（原版引擎切换语义）
    pub fn ensure_started(&mut self, config: &WorkerConfig) -> Result<(), AsrManagerError> {
        if self.unavailable {
            let same = self
                .config
                .as_ref()
                .map_or(false, |c| sig(c) == sig(config));
            if same {
                return Err(AsrManagerError::Unavailable("重启配额已耗尽".into()));
            }
            // 换引擎：复活
            tracing::info!("ASR 引擎已切换，清除不可用标记并重建 worker");
            self.unavailable = false;
            self.restart_count = 0;
            self.error_count = 0;
        }
        if self.client.is_some() && self.config.as_ref().map_or(false, |c| sig(c) == sig(config)) {
            return Ok(()); // 已就绪且配置一致
        }
        // 配置变更：关旧起新（generation 推进）
        if let Some(old) = self.client.as_mut() {
            tracing::info!("ASR 配置变更，替换 worker");
            old.shutdown();
        }
        self.client = None;
        self.restart_count = 0;
        self.error_count = 0;
        self.spawn_ready(config)
    }

    /// 单次识别：错误分类 + 自动恢复（与原版 _run_asr 一致；失败不重试同一段）
    pub fn transcribe(
        &mut self,
        audio: &[f32],
        word_timestamps: bool,
    ) -> Result<AsrResult, AsrManagerError> {
        if self.unavailable || self.client.is_none() {
            return Err(AsrManagerError::Unavailable("worker 未就绪".into()));
        }
        let result = self.client.as_mut().unwrap().transcribe(audio, word_timestamps);
        match result {
            Ok(res) => {
                self.error_count = 0;
                self.restart_count = 0;
                self.maybe_recycle();
                Ok(res)
            }
            Err(AsrClientError::Worker { message, recoverable }) => {
                self.error_count += 1;
                let fatal = !recoverable || self.error_count >= 3;
                if fatal {
                    self.mark_unavailable(&message);
                }
                Err(AsrManagerError::Failed(format!(
                    "ASR 错误（{}/3）: {message}",
                    self.error_count
                )))
            }
            Err(AsrClientError::Exited(reason)) => {
                Err(self.recover(&format!("进程退出: {reason}")))
            }
            Err(AsrClientError::Timeout(t)) => Err(self.recover(&format!("响应超时 {t:.1}s"))),
            Err(e) => {
                // Status/Io：客户端层用法错误，按失败上抛（原版 except Exception 分支）
                Err(AsrManagerError::Failed(format!("{e}")))
            }
        }
    }

    pub fn set_language(&mut self, language: &str) -> Result<(), AsrManagerError> {
        self.simple_request(|c| c.set_language(language))
    }

    pub fn set_input_padding(&mut self, pad_seconds: f32) -> Result<(), AsrManagerError> {
        self.simple_request(|c| c.set_input_padding(pad_seconds))
    }

    fn simple_request(
        &mut self,
        f: impl FnOnce(&mut AsrWorkerClient) -> Result<(), AsrClientError>,
    ) -> Result<(), AsrManagerError> {
        let Some(c) = self.client.as_mut() else {
            return Err(AsrManagerError::Unavailable("worker 未就绪".into()));
        };
        match f(c) {
            Ok(()) => Ok(()),
            Err(AsrClientError::Exited(r)) => Err(self.recover(&format!("进程退出: {r}"))),
            Err(e) => Err(AsrManagerError::Failed(format!("{e}"))),
        }
    }

    /// 崩溃/超时恢复：重启计数 +1，超限标记不可用（原版 _recover_asr_worker）。
    /// 返回调用方应上抛的错误。
    fn recover(&mut self, reason: &str) -> AsrManagerError {
        if let Some(old) = self.client.as_mut() {
            old.shutdown();
        }
        self.client = None;
        let Some(config) = self.config.clone() else {
            self.mark_unavailable("无配置可重启");
            return AsrManagerError::Unavailable("无配置可重启".into());
        };
        self.restart_count += 1;
        if self.restart_count > RESTART_MAX {
            let msg = format!("{reason}；自动重启已达 {RESTART_MAX} 次上限");
            self.mark_unavailable(&msg);
            return AsrManagerError::Unavailable(msg);
        }
        tracing::warn!("ASR worker 死亡（{reason}）；自动重启 {}/{}", self.restart_count, RESTART_MAX);
        match self.spawn_ready(&config) {
            Ok(()) => AsrManagerError::Failed(format!("{reason}；已自动重启，本段丢弃")),
            Err(e) => {
                let msg = format!("{reason}；重启失败: {e}");
                if self.restart_count >= RESTART_MAX {
                    self.mark_unavailable(&msg);
                    return AsrManagerError::Unavailable(msg);
                }
                AsrManagerError::Failed(msg)
            }
        }
    }

    fn mark_unavailable(&mut self, reason: &str) {
        tracing::error!("ASR 标记不可用: {reason}");
        if let Some(old) = self.client.as_mut() {
            old.shutdown();
        }
        self.client = None;
        self.unavailable = true;
    }

    /// RSS 回收（E-06；纯决策函数见 [`should_recycle`]）
    fn maybe_recycle(&mut self) {
        let Some(client) = self.client.as_ref() else { return };
        let pid = client.pid();
        let Some(rss_mb) = sample_rss_mb(pid) else { return };
        match self.baseline_mb {
            None => {
                self.baseline_mb = Some(rss_mb);
            }
            Some(base) => {
                if should_recycle(base, rss_mb, RECYCLE_DELTA_MB) {
                    tracing::warn!(
                        "ASR worker RSS={rss_mb}MB 超出基线 {base}MB 超过 {RECYCLE_DELTA_MB}MB，回收重建"
                    );
                    if let Some(old) = self.client.as_mut() {
                        old.shutdown();
                    }
                    self.client = None;
                    let config = self.config.clone();
                    if let Some(config) = config {
                        if let Err(e) = self.spawn_ready(&config) {
                            tracing::error!("RSS 回收重建失败: {e}");
                        }
                    }
                }
            }
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(old) = self.client.as_mut() {
            old.shutdown();
        }
        self.client = None;
    }
}

/// 回收决策（纯函数便于单测）
fn should_recycle(baseline_mb: u64, current_mb: u64, delta_mb: u64) -> bool {
    current_mb >= baseline_mb.saturating_add(delta_mb)
}

/// 子进程 RSS 采样（MB）
fn sample_rss_mb(pid: u32) -> Option<u64> {
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
    sys.process(sysinfo::Pid::from_u32(pid))
        .map(|p| p.memory() / (1024 * 1024))
}

fn sig(config: &WorkerConfig) -> String {
    serde_json::to_string(config).unwrap_or_default()
}

impl Default for AsrManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod should_recycle_tests {
    use super::should_recycle;

    #[test]
    fn recycle_only_past_delta() {
        // 原版语义：rss < baseline+delta 跳过 → 恰好等于阈值即回收
        assert!(!should_recycle(500, 2000, 2048));
        assert!(!should_recycle(500, 2547, 2048));
        assert!(should_recycle(500, 2548, 2048));
        assert!(should_recycle(0, 1, 0));
    }
}
