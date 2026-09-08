//! AsrManager：worker 生命周期的管理层（原版 main.py _run_asr/_recover_asr_worker/
//! _maybe_recycle_asr_worker 的语义 1:1）。
//!
//! - 成功 → error_count/restart_count 清零
//! - worker 回可恢复错误 → error_count+1；连续 ≥3 次或不可恢复 → 标记不可用
//! - 进程退出/超时 → 自动重启（≤3 次）；耗尽 → 标记不可用
//! - RSS 超出基线 +2048MB → 优雅回收（不占失败配额，E-06；仅在段队列空闲时
//!   调用，对齐原版 _asr_loop queue.Empty 分支）
//! - 引擎配置变更 → 替换 worker（generation 语义：旧实例关闭，计数清零）；
//!   新配置加载失败 → 回滚旧 worker（原版 _switch_asr_engine._load）
//! - 语言/padding 走挂起句柄（原版 _asr_pending_*）：UI 线程只存值，ASR 线程
//!   在每次 transcribe 前应用（_apply_pending_asr_settings，送达即提交）

use crate::client::{AsrClientError, AsrWorkerClient};
use crate::worker::WorkerConfig;
use lt_proto::AsrResult;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// 自动重启上限（原版 _asr_restart_max）
const RESTART_MAX: u32 = 3;
/// RSS 回收阈值：超出加载后基线这么多 MB 就回收（原版 _asr_recycle_delta_mb）
const RECYCLE_DELTA_MB: u64 = 2048;

/// 管理层错误（unavailable = 需要用户干预/换引擎；其余为单次失败）
#[derive(Debug, thiserror::Error)]
pub enum AsrManagerError {
    #[error("ASR 不可用: {0}")]
    Unavailable(String),
    /// worker 死亡/超时且已自动重启（本段丢弃）：**命令未送达**。
    /// pending 语义据此保持挂起（原版异常传播路径：挂起值由重启后的
    /// worker 在下一次 transcribe 前重新应用）
    #[error("{0}")]
    Restarted(String),
    #[error("{0}")]
    Failed(String),
}

impl AsrManagerError {
    pub fn unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

pub type Spawner = Box<dyn Fn(&WorkerConfig) -> Result<AsrWorkerClient, AsrClientError> + Send>;

/// 挂起状态（原版 _asr_pending_language/_asr_pending_padding；
/// UI 线程只写，ASR 线程在每次 transcribe 前应用并清除）
#[derive(Default)]
struct PendingState {
    language: Option<String>,
    /// padding 按引擎类型挂起（"funasr"/"whisper"），互不覆盖
    padding: HashMap<String, f32>,
}

/// UI 线程安全句柄：仅加锁存值，绝不跨进程调用（原版 _set_asr_language/
/// _set_asr_padding 语义——慢/挂死的 worker 不能冻结 UI）。UI 线程与 ASR 线程
/// 各持一份克隆，共享同一挂起状态。
#[derive(Clone, Default)]
pub struct AsrPendingHandle(Arc<Mutex<PendingState>>);

impl AsrPendingHandle {
    /// UI 线程：挂起识别语言（ASR 线程下一次 transcribe 前应用并提交）
    pub fn set_language(&self, lang: &str) {
        self.lock().language = Some(lang.to_string());
    }

    /// UI 线程：按引擎家族（"funasr"/"whisper"）挂起 padding，互不覆盖
    pub fn set_padding(&self, engine_family: &str, secs: f32) {
        self.lock().padding.insert(engine_family.to_string(), secs);
    }

    /// 锁内仅做存取（无 panic 点）；中毒也取回数据，不放大 panic
    fn lock(&self) -> MutexGuard<'_, PendingState> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// 引擎名 → 引擎家族（padding 挂起键，对应原版 asr_type "funasr"/"whisper"）。
/// 注意：M5 nano 接入时复核（nano 属 funasr 家族但不支持 padding）。
/// WP-B：qwen3 独立家族——若落 whisper 兜底，此前挂起的 whisper padding 会
/// 泄漏到 qwen3 worker（Unsupported 噪音）；UI 不产生 "qwen3" 家族键 → 恒 no-op。
fn engine_family(engine: &str) -> &'static str {
    match engine {
        "sensevoice" | "nano" => "funasr",
        "qwen3" => "qwen3",
        // 其余（whisper 系）一律归 whisper 家族
        _ => "whisper",
    }
}

pub struct AsrManager {
    client: Option<AsrWorkerClient>,
    config: Option<WorkerConfig>,
    restart_count: u32,
    error_count: u32,
    baseline_mb: Option<u64>,
    unavailable: bool,
    spawn: Spawner,
    /// 语言/padding 挂起句柄（与 UI 线程共享；transcribe 前应用）
    pending: AsrPendingHandle,
}

impl AsrManager {
    /// 生产构造：走 `当前exe --asr-worker`（与 AsrWorkerClient::spawn 相同）
    pub fn new() -> Self {
        Self::with_pending(AsrPendingHandle::default())
    }

    /// 生产构造 + 指定挂起句柄（UI 线程持同一句柄即可挂起语言/padding）
    pub fn with_pending(pending: AsrPendingHandle) -> Self {
        Self::with_spawner_and_pending(Box::new(|cfg| AsrWorkerClient::spawn(cfg.clone())), pending)
    }

    /// 注入 spawner（测试用假 worker）
    pub fn with_spawner(spawn: Spawner) -> Self {
        Self::with_spawner_and_pending(spawn, AsrPendingHandle::default())
    }

    /// 注入 spawner + 挂起句柄（测试：假 worker + 共享挂起状态）
    pub fn with_spawner_and_pending(spawn: Spawner, pending: AsrPendingHandle) -> Self {
        Self {
            client: None,
            config: None,
            restart_count: 0,
            error_count: 0,
            baseline_mb: None,
            unavailable: false,
            spawn,
            pending,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    pub fn is_ready(&self) -> bool {
        self.client.is_some() && !self.unavailable
    }

    /// 当前 worker 配置（测试与 UI 读取用；worker 未启动时为 None）
    pub fn config(&self) -> Option<&WorkerConfig> {
        self.config.as_ref()
    }

    fn spawn_ready(&mut self, config: &WorkerConfig) -> Result<(), AsrManagerError> {
        let mut client = (self.spawn)(config)
            .map_err(|e| AsrManagerError::Failed(format!("worker 启动失败: {e}")))?;
        client.wait_ready().map_err(|e| {
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
        if self.client.is_some()
            && self
                .config
                .as_ref()
                .map_or(false, |c| sig(c) == sig(config))
        {
            return Ok(()); // 已就绪且配置一致
        }
        // 配置变更：关旧起新（generation 推进）；新配置加载失败回滚旧 worker
        // （原版 _switch_asr_engine._load：先停旧再载新，载入失败恢复旧配置）。
        // 回滚前提：此前确有可用 worker——复活路径 client 已为 None，谈不上回滚。
        let old_config = if self.client.is_some() {
            self.config.clone()
        } else {
            None
        };
        if let Some(old) = self.client.as_mut() {
            tracing::info!("ASR 配置变更，替换 worker");
            old.shutdown();
        }
        self.client = None;
        self.restart_count = 0;
        self.error_count = 0;
        if let Err(e) = self.spawn_ready(config) {
            // 首次启动本就没有旧 worker：无回滚可言，维持原样直接返回失败
            let Some(old_config) = old_config else {
                return Err(e);
            };
            tracing::warn!("新引擎加载失败（{e}），尝试用旧配置恢复 worker");
            match self.spawn_ready(&old_config) {
                Ok(()) => {
                    // 回滚成功：manager 仍可用，但对调用方而言本次切换失败（非 Unavailable）
                    Err(AsrManagerError::Failed(format!(
                        "新引擎加载失败: {e}；已回滚 {}",
                        old_config.engine
                    )))
                }
                Err(re) => {
                    let msg = format!("新引擎加载失败: {e}；恢复旧 worker 也失败: {re}");
                    self.mark_unavailable(&msg);
                    Err(AsrManagerError::Unavailable(msg))
                }
            }
        } else {
            Ok(())
        }
    }

    /// 单次识别：错误分类 + 自动恢复（失败不重试同一段）。
    /// 恢复语义（AH-2/D-26）：`Worker{recoverable}` 走三振计数，**其余一切
    /// 错误（Exited/Timeout/Status/Io）一律进 [`Self::recover`]**——client 层
    /// 任何使用点发现的死亡/协议失序都交还统一重建入口（原版 `except
    /// Exception` 仅按失败上抛，恢复只覆盖"等待期死亡"，留下永久僵尸态）。
    pub fn transcribe(
        &mut self,
        audio: &[f32],
        word_timestamps: bool,
    ) -> Result<AsrResult, AsrManagerError> {
        if self.unavailable {
            return Err(AsrManagerError::Unavailable("worker 未就绪".into()));
        }
        // recover/RSS 回收中 spawn 失败留下的空窗（client=None 且非 unavailable）：
        // 有限重建一次——失败即 mark_unavailable（此后走上面分支，不会每段重试）；
        // 复活路径 = 换配置 ensure_started（引擎切换语义）
        if self.client.is_none() {
            let Some(config) = self.config.clone() else {
                self.mark_unavailable("无配置可重建");
                return Err(AsrManagerError::Unavailable("无配置可重建".into()));
            };
            tracing::warn!("ASR worker 缺位（此前重建失败），尝试恢复");
            return match self.spawn_ready(&config) {
                Ok(()) => Err(AsrManagerError::Restarted("worker 已恢复，本段丢弃".into())),
                Err(e) => {
                    let msg = format!("worker 缺位且重建失败: {e}");
                    self.mark_unavailable(&msg);
                    Err(AsrManagerError::Unavailable(msg))
                }
            };
        }
        // 识别前应用挂起设置（原版 _apply_pending_asr_settings）：
        // worker 死亡/超时 → 保持挂起并直接上抛，重启后的 worker 重新应用
        if let Err(e) = self.apply_pending() {
            return Err(e);
        }
        let result = self
            .client
            .as_mut()
            .unwrap()
            .transcribe(audio, word_timestamps);
        match result {
            Ok(res) => {
                self.error_count = 0;
                self.restart_count = 0;
                Ok(res)
            }
            Err(AsrClientError::Worker {
                message,
                recoverable,
            }) => {
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
            Err(e) => Err(self.recover(&format!("{e}"))),
        }
    }

    pub fn set_language(&mut self, language: &str) -> Result<(), AsrManagerError> {
        self.simple_request(|c| c.set_language(language))
    }

    pub fn set_input_padding(&mut self, pad_seconds: f32) -> Result<(), AsrManagerError> {
        self.simple_request(|c| c.set_input_padding(pad_seconds))
    }

    /// 应用挂起的语言/padding（原版 _apply_pending_asr_settings；transcribe 前调用）。
    /// 提交规则（原版"命令送达即提交"）：
    /// - 命令送达（Ok）或送达后 worker 回可恢复错误（Failed，仅 warn）→ 写回
    ///   restart config（防自动重启/RSS 回收回退到引擎切换时的旧值）+ 清除挂起；
    /// - worker 死亡/超时（Unavailable，原版异常传播）→ 保持挂起并上抛。
    fn apply_pending(&mut self) -> Result<(), AsrManagerError> {
        let snapshot = {
            let st = self.pending.lock();
            (st.language.clone(), st.padding.clone())
        };
        // ── 语言 ──
        if let Some(lang) = snapshot.0 {
            match self.set_language(&lang) {
                Err(e @ AsrManagerError::Unavailable(_)) => return Err(e), // 保持挂起
                // worker 死亡/超时（命令未送达，原版异常传播）：保持挂起，
                // 由重启后的 worker 在下一次 transcribe 前重新应用
                Err(e @ AsrManagerError::Restarted(_)) => return Err(e),
                Err(AsrManagerError::Failed(e)) => {
                    tracing::warn!("ASR 语言更新失败（仍提交挂起值）: {e}");
                }
                Ok(()) => {}
            }
            if let Some(cfg) = self.config.as_mut() {
                cfg.language = lang.clone();
            }
            // 清除挂起（原版 _clear_pending_language：UI 期间又挂了新值则保留新值）
            let mut st = self.pending.lock();
            if st.language.as_deref() == Some(lang.as_str()) {
                st.language = None;
            }
        }
        // ── padding：只取当前 worker 引擎家族对应的挂起条目 ──
        let Some(engine) = self.config.as_ref().map(|c| c.engine.clone()) else {
            return Ok(());
        };
        let family = engine_family(&engine);
        let Some(&secs) = snapshot.1.get(family) else {
            return Ok(());
        };
        // funasr 家族中 nano 不支持 padding（原版 funasr_supports_padding：
        // sensevoice=true、nano=false）：不下发也不写回 config，仅清除挂起
        if family == "funasr" && engine == "nano" {
            let mut st = self.pending.lock();
            if st.padding.get(family) == Some(&secs) {
                st.padding.remove(family);
            }
            return Ok(());
        }
        match self.set_input_padding(secs) {
            Err(e @ AsrManagerError::Unavailable(_)) => return Err(e), // 保持挂起
            // 同语言分支：worker 死亡/超时命令未送达，保持挂起
            Err(e @ AsrManagerError::Restarted(_)) => return Err(e),
            Err(AsrManagerError::Failed(e)) => {
                tracing::warn!("ASR padding 更新失败（仍提交挂起值）: {e}");
            }
            Ok(()) => {}
        }
        if let Some(cfg) = self.config.as_mut() {
            cfg.pad_seconds = Some(secs);
        }
        // 清除挂起（原版 _clear_pending_padding：值已被 UI 更新则保留新值）
        let mut st = self.pending.lock();
        if st.padding.get(family) == Some(&secs) {
            st.padding.remove(family);
        }
        Ok(())
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
            // worker 回错误（含 qwen3 set_language 非 auto 的诚实 Unsupported）：
            // 按失败上抛、带原始消息，不占恢复配额
            Err(AsrClientError::Worker { message, .. }) => Err(AsrManagerError::Failed(message)),
            // AH-2/D-26：与 transcribe 同语义——其余错误一律恢复重建
            // （原版语义只对 Exited/Timeout 走 _recover_asr_worker）
            Err(e) => Err(self.recover(&format!("{e}"))),
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
        tracing::warn!(
            "ASR worker 死亡（{reason}）；自动重启 {}/{}",
            self.restart_count,
            RESTART_MAX
        );
        match self.spawn_ready(&config) {
            Ok(()) => AsrManagerError::Restarted(format!("{reason}；已自动重启，本段丢弃")),
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

    /// RSS 回收（E-06；纯决策函数见 [`should_recycle`]）。
    /// 仅在段队列空闲时调用（原版 _asr_loop queue.Empty 分支；段间回收避免
    /// 打断待处理音频的识别，reload gap 不耗音频）。
    pub fn maybe_recycle_if_idle(&mut self) {
        let Some(client) = self.client.as_ref() else {
            return;
        };
        let pid = client.pid();
        let Some(rss_mb) = sample_rss_mb(pid) else {
            return;
        };
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
    sys.refresh_processes(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]),
        true,
    );
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
    use super::{engine_family, should_recycle};

    #[test]
    fn recycle_only_past_delta() {
        // 原版语义：rss < baseline+delta 跳过 → 恰好等于阈值即回收
        assert!(!should_recycle(500, 2000, 2048));
        assert!(!should_recycle(500, 2547, 2048));
        assert!(should_recycle(500, 2548, 2048));
        assert!(should_recycle(0, 1, 0));
    }

    #[test]
    fn nano_family_and_padding_invariants() {
        // WP-A 回归：nano 归 funasr 家族（padding 挂起键）；但 nano 不支持 padding
        // （原版 funasr_supports_padding: nano=false），apply_pending_asr_settings
        // 对 engine=="nano" 跳过下发——worker 引擎名经 build_worker_config 的
        // "funasr-nano-2512" → "nano" 分派与本表耦合，改名须同步。
        assert_eq!(engine_family("nano"), "funasr");
        assert_eq!(engine_family("sensevoice"), "funasr");
        assert_eq!(engine_family("whisper"), "whisper");
        // WP-B 回归：qwen3 独立家族（不得落 whisper 兜底——防挂起 padding 泄漏）
        assert_eq!(engine_family("qwen3"), "qwen3");
    }
}
