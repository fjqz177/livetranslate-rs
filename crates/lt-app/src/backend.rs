//! 后台命令线程（M2.5）：下载编排 + 下载期日志桥接。
//!
//! 原版对应物：SetupWizardDialog/ModelDownloadDialog 的 `_download_worker`
//! 后台线程 + `_LogCapture`（下载期间捕获 INFO+ 日志进对话框）。
//! Rust 版：UI 只发 [`Cmd::StartDownload`]，本线程跑 [`Downloader`]（阻塞式
//! 专用线程），把 Downloader 事件与 tracing 广播行统一转发为
//! [`UiEvent::DownloadProgress`] 日志流；成功写设置并发
//! [`UiEvent::DownloadSucceeded`]（向导=13 键默认块，原版 `_check_done`）。
//!
//! settings 镜像（M5.1）：拦截 PersistSettings/ApplySettings/SwitchEngine 先更新
//! 本地镜像再照旧转发 UI 循环——StartDownload 据此现场重算缺失清单，运行中
//! 切换的引擎/档位即时生效（启动快照会下错模型）；下载成功落盘同用镜像值。

use crate::logging;
use lt_models::cache::MissingModel;
use lt_models::download::{DownloadEvent, Downloader, Hub, ProxyMode};
use lt_proto::{Cmd, Settings, UiEvent, UiMsg};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// 启动后台命令线程（进程生命期常驻；通道关闭即退出）
pub fn spawn(cmd_rx: Receiver<Cmd>, proxy: EventLoopProxy<UiMsg>, first_launch: bool, settings: Settings) {
    std::thread::Builder::new()
        .name("lt-backend".into())
        .spawn(move || {
            let mut settings = settings;
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Cmd::StartDownload { hub, proxy: proxy_mode } => {
                        let missing = current_missing(&settings);
                        run_download(&proxy, first_launch, &settings, &missing, &hub, &proxy_mode);
                    }
                    // 镜像更新后照旧转发（设置真值在 UI 循环/AppState）
                    Cmd::PersistSettings(s) => {
                        settings = *s;
                        let _ =
                            proxy.send_event(UiMsg::Cmd(Cmd::PersistSettings(Box::new(settings.clone()))));
                    }
                    Cmd::ApplySettings(s) => {
                        settings = *s;
                        let _ =
                            proxy.send_event(UiMsg::Cmd(Cmd::ApplySettings(Box::new(settings.clone()))));
                    }
                    Cmd::SwitchEngine { engine, funasr_model, whisper_model_size, hub, language } => {
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

/// 借托盘退出菜单项触发应用退出（MultiWindowApp::on_menu 的 QUIT 分支）
fn quit(proxy: &EventLoopProxy<UiMsg>) {
    let _ = proxy.send_event(UiMsg::Menu("quit".into()));
}

fn proxy_mode_from(s: &str) -> ProxyMode {
    match s {
        "none" => ProxyMode::None,
        "system" | "" => ProxyMode::System,
        url => ProxyMode::Url(url.to_string()),
    }
}

fn run_download(
    proxy: &EventLoopProxy<UiMsg>,
    first_launch: bool,
    settings: &Settings,
    missing: &[MissingModel],
    hub_s: &str,
    proxy_s: &str,
) {
    let hub = if hub_s == "hf" { Hub::Hf } else { Hub::Ms };
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            fail(proxy, &format!("模型目录不可用: {e:#}"));
            return;
        }
    };
    // 首启目标固定 sensevoice-small（原版向导：silero + sensevoice-small；
    // silero 内嵌（D-5）→ 保留步骤展示但秒完成）
    let targets: Vec<MissingModel> = if first_launch {
        line(proxy, "Silero VAD 已内嵌，跳过下载");
        lt_models::cache::missing_models(&models_dir, "funasr", "sensevoice-small", "")
    } else {
        missing.to_vec()
    };
    if targets.is_empty() {
        // 已就绪（重试幂等）：直接成功收尾
        succeed(proxy, first_launch, settings, hub_s, proxy_s);
        return;
    }

    // 下载线程 + 事件泵（Downloader 阻塞式，独立线程）
    let (tx, rx) = std::sync::mpsc::channel::<DownloadEvent>();
    let dl_dir = models_dir.clone();
    let dl_proxy_mode = proxy_mode_from(proxy_s);
    let worker = std::thread::Builder::new()
        .name("lt-download".into())
        .spawn(move || {
            let dl = Downloader::new(dl_dir, dl_proxy_mode);
            for m in &targets {
                // hub 选择：always_hf 模型无视用户选择走 HF（原版 whisper/anime 先例）
                let (hub_eff, repo) = if m.always_hf {
                    (Hub::Hf, m.hub_hf)
                } else if hub == Hub::Hf {
                    (Hub::Hf, m.hub_hf)
                } else {
                    (Hub::Ms, m.hub_ms)
                };
                let Some(repo) = repo else {
                    return Err((m.display.clone(), anyhow::anyhow!("该 hub 无仓库")));
                };
                // 清单 = (文件名, 字节数下限)：下载器跳过校验与探测 manifest 同源（DL-2）
                let specs: Vec<(&str, u64)> = m
                    .files
                    .iter()
                    .copied()
                    .zip(m.files_min_bytes.iter().copied())
                    .collect();
                if let Err(e) = dl.download_files(hub_eff, repo, &specs, Some(&tx)) {
                    return Err((m.display.clone(), e));
                }
            }
            Ok(())
        })
        .expect("下载线程可启动");

    // 泵：转发 Downloader 事件 + 下载期 INFO 级 tracing 行（原版 _LogCapture）
    let mut log_rx = logging::subscribe();
    loop {
        loop {
            match log_rx.try_recv() {
                Ok(UiEvent::LogLine { msg, .. }) => line(proxy, &msg),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        match rx.try_recv() {
            Ok(ev) => line(proxy, &format_event(&ev)),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    match worker.join().expect("下载线程不 panic") {
        Ok(()) => succeed(proxy, first_launch, settings, hub_s, proxy_s),
        Err((name, e)) => fail(proxy, &format!("{name}: {e:#}")),
    }
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

/// 成功收尾：写设置（向导=13 键默认块，原版 _check_done 的 settings dict）→ 通知 UI
fn succeed(proxy: &EventLoopProxy<UiMsg>, first_launch: bool, settings: &Settings, hub_s: &str, proxy_s: &str) {
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
    if let Err(e) = lt_models::settings_io::save(&final_settings) {
        fail(proxy, &format!("设置保存失败: {e:#}"));
        return;
    }
    let _ = proxy.send_event(UiMsg::Event(UiEvent::DownloadSucceeded {
        settings: Box::new(final_settings),
    }));
}

fn fail(proxy: &EventLoopProxy<UiMsg>, msg: &str) {
    // 失败必须进日志（日志 tab/日志窗双通道），否则运行时下载失败无处可查
    tracing::error!("模型下载失败: {msg}");
    let _ = proxy.send_event(UiMsg::Event(UiEvent::DownloadFailed(msg.to_string())));
}

fn line(proxy: &EventLoopProxy<UiMsg>, s: &str) {
    let _ = proxy.send_event(UiMsg::Event(UiEvent::DownloadProgress(s.to_string())));
}

/// 下载事件 → 对话框日志行（进度走日志流，原版 UI 形态）
fn format_event(ev: &DownloadEvent) -> String {
    match ev {
        DownloadEvent::Progress { repo, file, done, total } => match total {
            Some(t) => format!("[{repo}] {file} {} / {}", format_size(*done), format_size(*t)),
            None => format!("[{repo}] {file} {}", format_size(*done)),
        },
        DownloadEvent::FileDone { repo, file } => format!("[{repo}] {file} 下载完成"),
        DownloadEvent::Done { repo, dir } => format!("[{repo}] 快照就绪: {}", dir.display()),
        DownloadEvent::Log(s) => s.clone(),
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
        assert!(matches!(proxy_mode_from("http://127.0.0.1:7890"), ProxyMode::Url(_)));
    }

    #[test]
    fn event_lines_readable() {
        assert_eq!(
            format_event(&DownloadEvent::Progress {
                repo: "a/b".into(),
                file: "m.onnx".into(),
                done: 1024,
                total: Some(2048)
            }),
            "[a/b] m.onnx 1.0 KB / 2.0 KB"
        );
        assert_eq!(
            format_event(&DownloadEvent::Log("x".into())),
            "x"
        );
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

        // whisper 档位切到本地 GGML 路径 → 不触发下载
        s.whisper_model_size = "D:/models/ggml-tiny.bin".into();
        assert!(current_missing(&s).is_empty(), "本地路径不触发下载");

        // 已缓存 → 空（伪造半体积以上文件命中阈值）
        s.whisper_model_size = "tiny".into();
        let snap = lt_models::paths::models_dir(s.models_dir.as_deref())
            .unwrap()
            .join("huggingface/hub/models--ggerganov--whisper.cpp/snapshots/main");
        std::fs::create_dir_all(&snap).unwrap();
        let est = lt_models::registry::whisper_entry_for("tiny").unwrap().estimated_bytes;
        std::fs::write(snap.join("ggml-tiny-q5_1.bin"), vec![0u8; (est / 2 + 1) as usize]).unwrap();
        assert!(current_missing(&s).is_empty(), "tiny 已缓存应无目标");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
