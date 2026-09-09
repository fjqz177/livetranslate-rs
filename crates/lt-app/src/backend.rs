//! 后台命令线程（M2.5）：下载编排 + 下载期日志桥接。
//!
//! 原版对应物：SetupWizardDialog/ModelDownloadDialog 的 `_download_worker`
//! 后台线程 + `_LogCapture`（下载期间捕获 INFO+ 日志进对话框）。
//! Rust 版：UI 只发 [`Cmd::StartDownload`]，本线程起**下载会话线程**
//! （DL-4：下载期间命令线程保持响应，PersistSettings/SwitchEngine 照常消化，
//! 设置镜像不再过期），会话内跑 [`Downloader`]（阻塞式专用线程 + 取消令牌），
//! 把 Downloader 事件类型化转发为 [`UiEvent::Download`]（进度）与
//! [`UiEvent::LogLine`]（target="download" 人读行）；成功发
//! [`UiEvent::DownloadSucceeded`]（向导=13 键默认块，原版 `_check_done`），
//! 取消发 [`UiEvent::DownloadCancelled`]（D-23）。落盘权归 UI 侧单写者
//! （DEC-4）：backend 不再直接写 settings.json。
//!
//! settings 镜像（M5.1）：拦截 PersistSettings/ApplySettings/SwitchEngine 先更新
//! 本地镜像再照旧转发 UI 循环——StartDownload 据此现场重算缺失清单，运行中
//! 切换的引擎/档位即时生效（启动快照会下错模型）。
//!
//! W2（架构 2.0 §3.3）：四条字符串旁路之一在此收口——`format_event` 的
//! `\t` 机器段协议废除：进度改推类型化 [`DownloadEvent`]，下载期 tracing
//! 广播行不再经会话泵二次转发（常驻日志桥已把它送日志窗），对话框人读行
//! 由本层以 `LogLine{target:"download"}` 直接发（保序、与进度事件同源）。

use lt_models::cache::MissingModel;
use lt_models::download::{
    hf_endpoint_for, hub_chain, DlError, DownloadEvent, Downloader, Hub, ProxyMode,
};
use lt_proto::{
    AppCommand, Cmd, DownloadEvent as ProtoDownload, DownloadFailKind, DownloadPhase, Settings,
    UiEvent, UiMsg,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// 在途下载会话（DL-4）：取消令牌 + 会话线程句柄
struct DownloadSession {
    cancel: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<()>,
}

/// 启动后台命令线程（进程生命期常驻；通道关闭即退出）
pub fn spawn(
    cmd_rx: Receiver<Cmd>,
    proxy: EventLoopProxy<UiMsg>,
    artery: Arc<crate::artery::EventArtery>,
    first_launch: bool,
    settings: Settings,
) {
    std::thread::Builder::new()
        .name("lt-backend".into())
        .spawn(move || {
            let mut settings = settings;
            let mut session: Option<DownloadSession> = None;
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Cmd::StartDownload {
                        hub,
                        proxy: proxy_mode,
                    } => {
                        // 回收已结束会话；在途则忽略重复请求（UI 侧亦有按钮守卫）
                        if session.as_ref().is_some_and(|s| s.handle.is_finished()) {
                            session = None;
                        }
                        if session.is_some() {
                            tracing::warn!("已有下载会话在途，忽略重复 StartDownload");
                            line(&artery, "已有下载进行中，请等待完成或取消后重试");
                            continue;
                        }
                        let missing = current_missing(&settings);
                        session = Some(start_session(
                            &artery,
                            first_launch,
                            &settings,
                            missing,
                            &hub,
                            &proxy_mode,
                        ));
                    }
                    Cmd::CancelDownload => match &session {
                        Some(s) if !s.handle.is_finished() => {
                            s.cancel.store(true, Ordering::Relaxed);
                            tracing::info!("已请求取消下载");
                            line(&artery, "正在取消下载（进度已保留）…");
                        }
                        _ => tracing::debug!("无在途下载，忽略 CancelDownload"),
                    },
                    // 镜像更新后照旧转发（设置真值在 UI 循环/AppState）
                    Cmd::PersistSettings(s) => {
                        settings = *s;
                        let _ = proxy.send_event(UiMsg::Cmd(Cmd::PersistSettings(Box::new(
                            settings.clone(),
                        ))));
                    }
                    Cmd::ApplySettings(s) => {
                        settings = *s;
                        let _ = proxy
                            .send_event(UiMsg::Cmd(Cmd::ApplySettings(Box::new(settings.clone()))));
                    }
                    Cmd::SwitchEngine {
                        engine,
                        funasr_model,
                        whisper_model_size,
                        hub,
                        language,
                    } => {
                        settings.asr_engine = engine.clone();
                        settings.funasr_model = funasr_model.clone();
                        settings.whisper_model_size = whisper_model_size.clone();
                        settings.hub = hub.clone();
                        settings.asr_language = language.clone();
                        let _ = proxy.send_event(UiMsg::Cmd(Cmd::SwitchEngine {
                            engine,
                            funasr_model,
                            whisper_model_size,
                            hub,
                            language,
                        }));
                    }
                    // 下载对话框失败后的关闭按钮（原版 reject → sys.exit(0)）
                    Cmd::Stop => quit(&proxy),
                    // 管道类命令原样回流事件循环，由 AppShell 分发（持有 Pipeline）
                    other => {
                        let _ = proxy.send_event(UiMsg::Cmd(other));
                    }
                }
            }
        })
        .ok();
}

/// 借托盘退出菜单项触发应用退出（MultiWindowApp::on_command 的 Quit 分支）
fn quit(proxy: &EventLoopProxy<UiMsg>) {
    let _ = proxy.send_event(UiMsg::AppCommand(AppCommand::Quit));
}

fn proxy_mode_from(s: &str) -> ProxyMode {
    match s {
        "none" => ProxyMode::None,
        "system" | "" => ProxyMode::System,
        url => ProxyMode::Url(url.to_string()),
    }
}

/// 起下载会话线程（DL-4）：backend 命令线程立即返回继续收命令，
/// 下载全程（worker + 事件泵）在会话线程内完成。
fn start_session(
    artery: &Arc<crate::artery::EventArtery>,
    first_launch: bool,
    settings: &Settings,
    missing: Vec<MissingModel>,
    hub_s: &str,
    proxy_s: &str,
) -> DownloadSession {
    let cancel = Arc::new(AtomicBool::new(false));
    let session_artery = artery.clone();
    let session_settings = settings.clone();
    let session_hub = hub_s.to_string();
    let session_proxy_s = proxy_s.to_string();
    let cancel_for_run = cancel.clone();
    let handle = std::thread::Builder::new()
        .name("lt-download-session".into())
        .spawn(move || {
            run_download(
                &session_artery,
                first_launch,
                &session_settings,
                &missing,
                &session_hub,
                &session_proxy_s,
                cancel_for_run,
            );
        })
        .expect("下载会话线程可启动");
    DownloadSession { cancel, handle }
}

fn run_download(
    artery: &Arc<crate::artery::EventArtery>,
    first_launch: bool,
    settings: &Settings,
    missing: &[MissingModel],
    hub_s: &str,
    proxy_s: &str,
    cancel: Arc<AtomicBool>,
) {
    let hub = if hub_s == "hf" { Hub::Hf } else { Hub::Ms };
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            fail(artery, DownloadFailKind::Disk, &format!("模型目录不可用: {e:#}"));
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
        succeed(artery, first_launch, settings, hub_s, proxy_s);
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
    let dl_proxy_mode = proxy_mode_from(proxy_s);
    let worker = std::thread::Builder::new()
        .name("lt-download".into())
        .spawn(move || {
            // D-24：HF 尝试端点随所选 hub——选 HF=官方直连，选 MS=自动走 hf-mirror
            let dl = Downloader::new(dl_dir, dl_proxy_mode).with_hf_endpoint(hf_endpoint_for(hub));
            for m in &targets {
                // DL-5：所选 hub 优先，404/网络不可达时回落另一 hub（编排下沉
                // Downloader::download_model；hub_chain 见 lt-models）
                let chain = hub_chain(hub, m.hub_hf, m.hub_ms, m.always_hf);
                // 清单 = (文件名, 字节数下限)：下载器跳过校验与探测 manifest 同源（DL-2）
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
    match worker.join().expect("下载线程不 panic") {
        Ok(()) => succeed(artery, first_launch, settings, hub_s, proxy_s),
        Err((name, e)) => {
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
/// DL-4/DEC-4：backend **不再直接写盘**——settings.json 由 UI 侧单写者
/// （shell.persist_settings）落盘：运行期成功链 = shell 收 DownloadSucceeded
/// → 重发 SwitchEngine → persist；启动流 = app.rs 收成功事件后发
/// Cmd::PersistSettings。旧实现在这里用可能过期的镜像写盘并回踩 UI 状态（F6）。
fn succeed(
    artery: &Arc<crate::artery::EventArtery>,
    first_launch: bool,
    settings: &Settings,
    hub_s: &str,
    proxy_s: &str,
) {
    let final_settings = if first_launch {
        let mut s = Settings::default();
        s.hub = hub_s.into();
        s.download_proxy = proxy_s.into();
        s.asr_engine = "funasr".into();
        s.funasr_model = "sensevoice-small".into();
        s.vad_mode = "silero".into();
        s.vad_threshold = 0.3;
        s.energy_threshold = 0.02;
        s.min_speech_duration = 1.0;
        s.max_speech_duration = 8.0;
        s.silence_mode = "auto".into();
        s.silence_duration = 0.8;
        s.asr_language = "auto".into();
        s.target_language = "zh".into();
        s
    } else {
        settings.clone()
    };
    artery.push(UiEvent::DownloadSucceeded {
        settings: Box::new(final_settings),
    });
}

fn fail(artery: &Arc<crate::artery::EventArtery>, kind: DownloadFailKind, msg: &str) {
    // 失败必须进日志（日志 tab/日志窗双通道），否则运行时下载失败无处可查
    tracing::error!("模型下载失败: {msg}");
    artery.push(UiEvent::DownloadFailed {
        kind,
        message: msg.to_string(),
    });
}

fn line(artery: &Arc<crate::artery::EventArtery>, s: &str) {
    // 下载人读行：target="download" 专用日志流（UI 侧分流进下载框/卡片
    // 日志 + 日志窗）。机器控制流不再骑日志总线（INV9，W2）——经动脉投递
    artery.push(UiEvent::LogLine {
        level: 20,
        target: "download".into(),
        msg: s.to_string(),
    });
}

/// Downloader 事件 → 类型化 UiEvent（W2，替代 format_event 的 `\t` 机器段）：
/// Progress → `UiEvent::Download`（UI 驱动进度条 + 同格式人读行）；
/// FileDone/Done/Log → 人读行（`LogLine[download]`）。
fn emit_download_event(artery: &Arc<crate::artery::EventArtery>, ev: &DownloadEvent) {
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

    #[test]
    fn size_format_matches_original() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(250_000_000), "238.4 MB");
        assert_eq!(format_size(3_100_000_000), "2.89 GB");
    }

    #[test]
    fn proxy_mode_mapping() {
        assert!(matches!(proxy_mode_from("none"), ProxyMode::None));
        assert!(matches!(proxy_mode_from("system"), ProxyMode::System));
        assert!(matches!(proxy_mode_from(""), ProxyMode::System));
        assert!(matches!(
            proxy_mode_from("http://127.0.0.1:7890"),
            ProxyMode::Url(_)
        ));
    }

    /// 下载目标现场重算：跟随 settings 镜像的引擎/档位（M5.1 快照 bug 回归测试）。
    /// 场景：启动时 funasr/sensevoice-small 未缓存（快照非空），运行中切 whisper/tiny
    /// → StartDownload 必须下载 whisper tiny，而不是启动快照里的 sensevoice。
    #[test]
    fn missing_targets_follow_settings_mirror() {
        let dir = std::env::temp_dir().join(format!("lt_backend_mirror_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Settings::default();
        s.models_dir = Some(dir.clone());

        // funasr/sensevoice-small 未缓存 → 1 条目标
        s.asr_engine = "funasr".into();
        s.funasr_model = "sensevoice-small".into();
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
