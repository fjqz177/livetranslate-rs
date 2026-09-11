//! 下载管理（架构 2.0 W3 自 lt-app/backend.rs 下载会话域迁入，方案 §3.1：
//! lt-orchestrator 承接下载编排）。单在途会话：目标清单现场重算、磁盘预检、
//! 进度/失败/取消全部经 EventArtery 类型化回流（W2 后的无字符串协议形态，
//! 见 event_artery 注记）。线程模型（W7 收口）：会话线程经
//! [`Supervisor::spawn`]（Policy::Never，`ThreadRole::Download`）出生——死亡
//! 经 `ThreadDied` 可见；下载 worker 为会话子线程，由会话线程 join（完成
//! 结果与 panic 均转终态事件，终止语义仅此一处消费——全局监督器单句柄
//! 单一消费者，子线程不复用同一句柄）。
//!
//! 原版对应物：SetupWizardDialog/ModelDownloadDialog 的 `_download_worker`
//! 后台线程 + `_LogCapture`；Rust 版在 UI 只发 [`Cmd::StartDownload`] 的语义
//! 不变——本管理器在命令线程上下文被调用（start/cancel 非阻塞，
//! 下载全量工作在会话线程内完成）。

use lt_download::{hf_endpoint_for, hub_chain, DlError, DownloadEvent, Downloader, Hub, ProxyMode};
use lt_models::cache::MissingModel;
use lt_proto::{
    DownloadEvent as ProtoDownload, DownloadFailKind, DownloadPhase, Settings, ThreadRole, UiEvent,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::Duration;

use crate::event_artery::EventSink;
use crate::supervisor::{Policy, Supervisor};

/// 会话线程名（生命周期查询键：`Supervisor::is_thread_finished`）
const DL_SESSION_THREAD: &str = "lt-download-session";

/// 下载编排管理器（命令线程上下文独占；会话线程仅持 EventSink + 设置快照）
pub struct DownloadManager {
    artery: EventSink,
    /// 首启目标固定 sensevoice-small（原版向导语义；运行期下载跟随设置镜像现场重算）
    first_launch: bool,
    /// 监督器（下载会话线程出生点；在途判定经 `is_thread_finished`）
    sup: Arc<Supervisor>,
    /// 在途会话取消令牌（start 置位；cancel 只置标志——线程收尾读）
    cancel: Option<Arc<AtomicBool>>,
}

impl DownloadManager {
    pub fn new(artery: EventSink, first_launch: bool, sup: Arc<Supervisor>) -> Self {
        Self {
            artery,
            first_launch,
            sup,
            cancel: None,
        }
    }

    /// 在途判定（W7：会话结束经监督器查询收敛——终态事件丢失时
    /// `is_thread_finished` 仍可自愈，不会永久滞留"下载中"）
    fn in_flight(&self) -> bool {
        !self.sup.is_thread_finished(DL_SESSION_THREAD)
    }

    /// 起下载会话（非阻塞）：目标清单按当前设置现场重算（M5.1——运行中切换
    /// 的引擎/档位即时生效，启动快照会下错模型）；在途时忽略重复请求
    /// （UI 侧亦有按钮守卫）。E2/D-79：hub/proxy 为值域枚举（UI 经透镜
    /// 转换后传入，本层不再解析字符串）。
    pub fn start(&mut self, settings: &Settings, hub: Hub, proxy: ProxyMode) {
        if self.in_flight() {
            tracing::warn!("已有下载会话在途，忽略重复 StartDownload");
            self.line("已有下载进行中，请等待完成或取消后重试");
            return;
        }
        let missing = current_missing(settings);
        let cancel = Arc::new(AtomicBool::new(false));
        let artery = self.artery.clone();
        let first_launch = self.first_launch;
        let session_settings = settings.clone();
        let session_proxy = proxy;
        let cancel_for_run = cancel.clone();
        // INV3：会话线程出生唯一＝监督器；Policy::Never（常量回收语义）。
        // factory 惰性持有（INV5 干净初态），catch_unwind 兜底层 panic →
        // 终态事件（下载卡不再可能停驻"下载中"，R1 同源防线）。
        self.sup.spawn(
            ThreadRole::Download,
            DL_SESSION_THREAD,
            Policy::Never,
            move || {
                let artery = artery.clone();
                let session_settings = session_settings.clone();
                let missing = missing.clone();
                let session_proxy = session_proxy.clone();
                let cancel_for_run = cancel_for_run.clone();
                Box::new(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        run_download(
                            &artery,
                            first_launch,
                            &session_settings,
                            &missing,
                            hub,
                            session_proxy,
                            cancel_for_run,
                        );
                    }));
                    if result.is_err() {
                        fail(
                            &artery,
                            DownloadFailKind::Other,
                            "下载会话异常退出（panic，详情见 crash 文件与日志）",
                        );
                    }
                })
            },
        );
        self.cancel = Some(cancel);
    }

    /// 取消在途下载（进度已保留；无在途则忽略）
    pub fn cancel(&mut self) {
        if self.in_flight() {
            if let Some(c) = &self.cancel {
                c.store(true, Ordering::Relaxed);
                tracing::info!("已请求取消下载");
                self.line("正在取消下载（进度已保留）…");
            }
        } else {
            tracing::debug!("无在途下载，忽略 CancelDownload");
        }
    }

    /// 下载人读行：target="download" 专用日志流（UI 侧分流进下载框/卡片
    /// 日志 + 日志窗）。机器控制流不再骑日志总线（INV9，W2）——经动脉投递
    fn line(&self, s: &str) {
        self.artery.push(UiEvent::LogLine {
            level: 20,
            target: "download".into(),
            msg: s.to_string(),
        });
    }
}

fn run_download(
    artery: &Arc<crate::event_artery::EventArtery>,
    first_launch: bool,
    settings: &Settings,
    missing: &[MissingModel],
    hub: Hub,
    proxy: ProxyMode,
    cancel: Arc<AtomicBool>,
) {
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            fail(
                artery,
                DownloadFailKind::Disk,
                &format!("模型目录不可用: {e:#}"),
            );
            return;
        }
    };
    // 首启目标固定 sensevoice-small（原版向导：silero + sensevoice-small；
    // silero 内嵌（D-5）→ 保留步骤展示但秒完成）
    let targets: Vec<MissingModel> = if first_launch {
        line(artery, "Silero VAD 已内嵌，跳过下载");
        lt_models::cache::missing_models(&models_dir, "funasr", "sensevoice-small", "")
    } else {
        missing.to_vec()
    };
    if targets.is_empty() {
        // 已就绪（重试幂等）：直接成功收尾
        succeed(artery, first_launch, settings, hub, &proxy);
        return;
    }

    // AH-5：下载前磁盘剩余空间预检（941MB 级模型默认落 C 盘；不足即明示快速
    // 失败，避免写入中途 Disk 错误且已占用部分空间）。探测失败不阻断。
    let need: u64 = targets.iter().map(|m| m.estimated_bytes).sum::<u64>() + 256 * 1024 * 1024;
    if let Some(free) = free_disk_bytes(&models_dir) {
        if free < need {
            fail(
                artery,
                DownloadFailKind::Disk,
                &format!(
                    "磁盘剩余空间不足：本模型约需 {}，当前仅剩 {}（可改 models_dir 或清理磁盘）",
                    format_size(need),
                    format_size(free)
                ),
            );
            return;
        }
    }

    // 下载线程 + 事件泵（Downloader 阻塞式，独立线程）
    let (tx, rx) = std::sync::mpsc::channel::<DownloadEvent>();
    let dl_dir = models_dir.clone();
    let worker_proxy = proxy.clone();
    let worker = std::thread::Builder::new()
        .name("lt-download".into())
        .spawn(move || {
            // D-24：HF 尝试端点随所选 hub——选 HF=官方直连，选 MS=自动走 hf-mirror
            let dl = Downloader::new(dl_dir, worker_proxy).with_hf_endpoint(hf_endpoint_for(hub));
            for m in &targets {
                // DL-5：所选 hub 优先，404/网络不可达时回落另一 hub（编排下沉
                // Downloader::download_model；hub_chain 见 lt-models）
                let chain = hub_chain(hub, m.hub_hf, m.hub_ms, m.always_hf);
                // 清单 = (文件名, 字节数下限, sha256)：跳过校验/长度/内容三通道
                // 与注册表同源（DL-2 + AH-5）
                let specs: Vec<(&str, u64, &str)> = m
                    .files
                    .iter()
                    .copied()
                    .zip(m.files_min_bytes.iter().copied())
                    .zip(m.files_sha256.iter().copied())
                    .map(|((f, min), sha)| (f, min, sha))
                    .collect();
                if let Err(e) = dl.download_model(&chain, &specs, &cancel, Some(&tx)) {
                    return Err((m.display.clone(), e));
                }
            }
            Ok(())
        })
        .expect("下载线程可启动");

    // 泵：Downloader 事件类型化转发（W2：进度 → Download；人读行 →
    // LogLine[download]）。下载期 tracing 广播行由常驻日志桥直送日志窗，
    // 不再经本会话泵二次转发（单通道，无重复）。
    loop {
        match rx.try_recv() {
            Ok(ev) => emit_download_event(artery, &ev),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // 收尾先排空事件通道再下结论（DL-6/F9：尾部 FileDone/Done 不再丢；
    // worker 已退出 → tx 已 drop，排空必然收敛）
    while let Ok(ev) = rx.try_recv() {
        emit_download_event(artery, &ev);
    }
    // W7：worker panic 不再是"会话线程被 expect 拖死"——join 的 Err 分支
    // 直接落失败终态，下载卡收 DownloadFailed 收敛（卡死解除）。
    match worker.join() {
        Ok(Ok(())) => succeed(artery, first_launch, settings, hub, &proxy),
        Ok(Err((name, e))) => {
            let cancelled = e
                .downcast_ref::<DlError>()
                .is_some_and(|d| d.kind == DownloadFailKind::Cancelled);
            if cancelled {
                tracing::info!("模型下载已被用户取消: {name}");
                artery.push(UiEvent::DownloadCancelled);
            } else {
                let dl = e.downcast_ref::<DlError>();
                match dl {
                    Some(d) => fail(artery, d.kind, &format!("{name}: {}", d.message())),
                    None => fail(artery, DownloadFailKind::Other, &format!("{name}: {e:#}")),
                }
            }
        }
        Err(_) => fail(
            artery,
            DownloadFailKind::Other,
            "下载线程异常退出（panic，详情见 crash 文件与日志）",
        ),
    }
}

/// 目标目录所在盘的剩余字节数（AH-5；Windows GetDiskFreeSpaceExW）。
/// 探测失败返回 None（调用方跳过预检，不阻断下载）。
#[cfg(windows)]
fn free_disk_bytes(dir: &std::path::Path) -> Option<u64> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.to_str().map(String::from))?;
    let mut wide: Vec<u16> = root.encode_utf16().collect();
    wide.push(0);
    let mut free: u64 = 0;
    let ok = unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), None, None, Some(&mut free)) };
    ok.is_ok().then_some(free)
}

#[cfg(not(windows))]
fn free_disk_bytes(_dir: &std::path::Path) -> Option<u64> {
    None
}

/// 当前设置的缺失模型清单（StartDownload 现场重算；本地 GGML 路径不触发下载）。
/// models_dir 不可用 → 空（run_download 内同样探测并报"模型目录不可用"）。
fn current_missing(settings: &Settings) -> Vec<MissingModel> {
    lt_models::paths::models_dir(settings.models_dir.as_deref())
        .map(|dir| {
            lt_models::cache::missing_models(
                &dir,
                &settings.asr_engine,
                &settings.funasr_model,
                &settings.whisper_model_size,
            )
        })
        .unwrap_or_default()
}

/// 成功收尾：通知 UI（向导=13 键默认块，原版 _check_done 的 settings dict）。
/// DL-4/DEC-4：本层**不再直接写盘**——settings.json 由 UI 侧单写者
/// （shell.persist_settings）落盘：运行期成功链 = shell 收 DownloadSucceeded
/// → 重发 SwitchEngine → persist；启动流 = app.rs 收成功事件后发
/// Cmd::PersistSettings。旧实现用可能过期的镜像写盘并回踩 UI 状态（F6）。
fn succeed(
    artery: &Arc<crate::event_artery::EventArtery>,
    first_launch: bool,
    settings: &Settings,
    hub: Hub,
    proxy: &ProxyMode,
) {
    let final_settings = if first_launch {
        Settings {
            // E2/D-79：持久层保持字符串——枚举经 as_settings_str/to_settings_str
            // 写回（向导 13 键块的 hub/proxy 语义不变）
            hub: hub.as_settings_str().into(),
            download_proxy: proxy.to_settings_str(),
            asr_engine: "funasr".into(),
            funasr_model: "sensevoice-small".into(),
            vad_mode: "silero".into(),
            vad_threshold: 0.3,
            energy_threshold: 0.02,
            min_speech_duration: 1.0,
            max_speech_duration: 8.0,
            silence_mode: "auto".into(),
            silence_duration: 0.8,
            asr_language: "auto".into(),
            target_language: "zh".into(),
            ..Default::default()
        }
    } else {
        settings.clone()
    };
    artery.push(UiEvent::DownloadSucceeded {
        settings: Box::new(final_settings),
    });
}

fn fail(artery: &Arc<crate::event_artery::EventArtery>, kind: DownloadFailKind, msg: &str) {
    // 失败必须进日志（日志 tab/日志窗双通道），否则运行时下载失败无处可查
    tracing::error!("模型下载失败: {msg}");
    artery.push(UiEvent::DownloadFailed {
        kind,
        message: msg.to_string(),
    });
}

fn line(artery: &Arc<crate::event_artery::EventArtery>, s: &str) {
    artery.push(UiEvent::LogLine {
        level: 20,
        target: "download".into(),
        msg: s.to_string(),
    });
}

/// Downloader 事件 → 类型化 UiEvent（W2，替代 format_event 的 `\t` 机器段）：
/// Progress → `UiEvent::Download`（UI 驱动进度条 + 同格式人读行）；
/// FileDone/Done/Log → 人读行（`LogLine[download]`）。
fn emit_download_event(artery: &Arc<crate::event_artery::EventArtery>, ev: &DownloadEvent) {
    match ev {
        DownloadEvent::Progress {
            repo,
            file,
            k,
            n,
            done,
            total,
        } => {
            artery.push(UiEvent::Download(ProtoDownload {
                repo: repo.clone(),
                file: file.clone(),
                index: *k as u32,
                count: *n as u32,
                done: *done,
                total: *total,
                phase: DownloadPhase::Progress,
            }));
        }
        DownloadEvent::FileDone { repo, file } => {
            line(artery, &format!("[{repo}] {file} 下载完成"))
        }
        DownloadEvent::Done { repo, dir } => {
            line(artery, &format!("[{repo}] 快照就绪: {}", dir.display()))
        }
        DownloadEvent::Log(s) => line(artery, s),
    }
}

/// 字节量人性化（阈值与精度对齐原版 model_manager.format_size）
fn format_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} B")
    } else if size_bytes < 1024u64.pow(2) {
        format!("{:.1} KB", size_bytes as f64 / 1024.0)
    } else if size_bytes < 1024u64.pow(3) {
        format!("{:.1} MB", size_bytes as f64 / 1024u64.pow(2) as f64)
    } else {
        format!("{:.2} GB", size_bytes as f64 / 1024u64.pow(3) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_artery::EventArtery;

    #[test]
    fn size_format_matches_original() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(250_000_000), "238.4 MB");
        assert_eq!(format_size(3_100_000_000), "2.89 GB");
    }

    /// E2/D-79：ProxyMode 单点转换随迁 proto——原 proxy_mode_from 语义等价
    ///（none 直连 / 空串与 system 走系统 / 其余视作 URL）
    #[test]
    fn proxy_mode_mapping() {
        assert!(matches!(
            ProxyMode::from_settings_str("none"),
            ProxyMode::None
        ));
        assert!(matches!(
            ProxyMode::from_settings_str("system"),
            ProxyMode::System
        ));
        assert!(matches!(
            ProxyMode::from_settings_str(""),
            ProxyMode::System
        ));
        assert!(matches!(
            ProxyMode::from_settings_str("http://127.0.0.1:7890"),
            ProxyMode::Url(_)
        ));
        // 往返：枚举 → 持久层字符串 → 枚举 恒等
        for p in [
            ProxyMode::None,
            ProxyMode::System,
            ProxyMode::Url("http://p:8080".into()),
        ] {
            assert_eq!(ProxyMode::from_settings_str(&p.to_settings_str()), p);
        }
    }

    /// W7：下载会话经监督器出生——快速成功会话（缺失清单为空，零网络）
    /// 发终态事件并收敛，随后可重入（在途守卫不误拦）。
    #[test]
    fn session_terminates_and_reallows_restart() {
        let artery = EventArtery::new();
        let sup = Supervisor::new(|_| {});
        let mut dl = DownloadManager::new(artery.clone(), false, sup.clone());

        let temp = std::env::temp_dir().join(format!("lt_dl_mgr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        // 本地 GGML 路径 → 无缺失 → run_download 快速成功（合成路径，PH-2）
        let s = Settings {
            models_dir: Some(temp.clone()),
            asr_engine: "whisper".into(),
            whisper_model_size: temp
                .join("lt_local")
                .join("ggml-tiny.bin")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };

        dl.start(&s, Hub::Hf, ProxyMode::None);
        // 终态事件到达（DownloadSucceeded；本地路径零网络）
        let mut batch = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if artery.drain_batch(&mut batch, Duration::from_millis(200)) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "下载会话应快速成功收尾"
            );
        }
        assert!(
            batch
                .iter()
                .any(|ev| matches!(ev, UiEvent::DownloadSucceeded { .. })),
            "快速失败路径应发成功后遗事件"
        );
        // 会话经监督器收敛 → 重入不被在途守卫拦截
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while dl.in_flight() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!dl.in_flight(), "会话结束后在途判假");
        dl.start(&s, Hub::Hf, ProxyMode::None);
        let mut batch = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if artery.drain_batch(&mut batch, Duration::from_millis(200)) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "重入会话应同样快速收尾"
            );
        }
        assert!(
            batch
                .iter()
                .any(|ev| matches!(ev, UiEvent::DownloadSucceeded { .. })),
            "重入成功路径应发成功后遗事件"
        );
        let _ = std::fs::remove_dir_all(&temp);
        sup.join_all();
    }

    /// 下载目标现场重算：跟随 settings 镜像的引擎/档位（M5.1 快照 bug 回归测试）。
    /// 场景：启动时 funasr/sensevoice-small 未缓存（快照非空），运行中切 whisper/tiny
    /// → StartDownload 必须下载 whisper tiny，而不是启动快照里的 sensevoice。
    #[test]
    fn missing_targets_follow_settings_mirror() {
        let dir = std::env::temp_dir().join(format!("lt_backend_mirror_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Settings {
            models_dir: Some(dir.clone()),
            asr_engine: "funasr".into(),
            funasr_model: "sensevoice-small".into(),
            ..Default::default()
        };

        // funasr/sensevoice-small 未缓存 → 1 条目标
        let miss = current_missing(&s);
        assert_eq!(miss.len(), 1, "sensevoice-small 未缓存应有 1 条目标");
        assert!(!miss[0].always_hf, "sensevoice 双 hub 可选");
        assert!(miss[0].hub_ms.is_some() && miss[0].hub_hf.is_some());

        // 运行中切 whisper/tiny（快照方案会仍返回 sensevoice）→ 目标变为 whisper tiny
        s.asr_engine = "whisper".into();
        s.whisper_model_size = "tiny".into();
        let miss = current_missing(&s);
        assert_eq!(miss.len(), 1);
        assert!(miss[0].always_hf, "whisper 走 always_hf 单仓");
        assert_eq!(miss[0].hub_hf, Some("ggerganov/whisper.cpp"));
        assert_eq!(miss[0].hub_ms, None);
        assert_eq!(miss[0].files, &["ggml-tiny-q5_1.bin"]);

        // whisper 档位切到本地 GGML 路径 → 不触发下载（合成路径 temp 派生，PH-2）
        s.whisper_model_size = std::env::temp_dir()
            .join("lt_local")
            .join("ggml-tiny.bin")
            .to_string_lossy()
            .into_owned();
        assert!(current_missing(&s).is_empty(), "本地路径不触发下载");

        // 已缓存 → 空（伪造半体积以上文件命中阈值）
        s.whisper_model_size = "tiny".into();
        let snap = lt_models::paths::models_dir(s.models_dir.as_deref())
            .unwrap()
            .join("huggingface/hub/models--ggerganov--whisper.cpp/snapshots/main");
        std::fs::create_dir_all(&snap).unwrap();
        let est = lt_models::registry::whisper_entry_for("tiny")
            .unwrap()
            .estimated_bytes;
        std::fs::write(
            snap.join("ggml-tiny-q5_1.bin"),
            vec![0u8; (est / 2 + 1) as usize],
        )
        .unwrap();
        assert!(current_missing(&s).is_empty(), "tiny 已缓存应无目标");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
