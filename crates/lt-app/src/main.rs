//! LiveTranslate 主入口。
//!
//! 启动流（对齐新版原版 2026-09-01 less-is-more 决策，first-launch-flow.md）：
//! **没有首启向导、没有缺模型下载门**——打开即进主界面并启动管道；
//! 模型未缓存时管道发 AsrUnavailable（悬浮窗显示 unavailable），用户经
//! 设置 → 识别页按需下载（StartDownload 命令复用 M2.5 下载管线）。
//! `--asr-worker` 为 worker 子进程入口。

// 双击 exe 不再弹控制台窗口（对齐正规 GUI 应用；debug 构建保留 console 便于
// 开发观察 tracing 输出；worker 子进程 stdout/stderr 已管道重定向，见
// lt-asr/src/client.rs E-07，与子系统无关）。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod artery;
mod logging;
mod panic_hook;
mod shell;
mod shell_helpers;
mod singleton;

use lt_proto::{Cmd, UiMsg};

fn main() -> anyhow::Result<()> {
    // worker 子进程入口：--asr-worker（配置经 stdin 首行，R28/D-76——
    // argv 仅旗标，命令行不再明文暴露模型路径）
    if std::env::args().any(|a| a == "--asr-worker") {
        return asr_worker_entry();
    }

    // WD-2：版本旗标（--version/-V）——零初始化直接输出（GUI 外可见性）
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("livetranslate {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // ── R1/D-62：panic hook 最早安装（先于一切可失败步骤；hook 落 crash
    // 文件 + tracing，线程 panic 从此不再黑洞）──
    panic_hook::install();

    // 早期失败呈现（R11①/D-65）：事件循环线程尚未建立，同步 MessageBoxW
    // 合法（D-33 禁令针对事件循环线程）——logging 初始化前的失败不再黑洞
    let early = (|| -> anyhow::Result<lt_proto::Settings> {
        // R-4 预案 A：ort 走 load-dynamic，任何 ort 调用前解压 dll 并指向它
        lt_audio::ensure_ort_dylib()?;
        // 无 settings 文件 → 全默认值（直进主界面；模型缺失走识别页按需下载）
        let initial_settings = lt_models::settings_io::load()?.unwrap_or_default();
        // ui_lang="system" → 系统语言解析（新版 general_tab 的 resolve_ui_lang 语义）
        let ui_lang = initial_settings.ui_lang.clone();
        // R14②：i18n 解析失败 = 内嵌资产损坏（构建期后）→ 硬错进 boot 呈现
        lt_i18n::set_lang(if ui_lang == "system" {
            lt_i18n::detect_system_lang()
        } else {
            &ui_lang
        })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        logging::init()?;
        Ok(initial_settings)
    })();
    let initial_settings = match early {
        Ok(s) => s,
        Err(e) => {
            fatal_early(&format!("{e:#}"));
            return Err(e);
        }
    };

    let event_loop = winit::event_loop::EventLoop::<UiMsg>::with_user_event().build()?;
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    // W2/D-67：音频监视快照格（capture 写、UI 33ms 节拍读；跨管道生命周期）
    let monitor_cell: std::sync::Arc<arc_swap::ArcSwap<lt_proto::MonitorSample>> =
        std::sync::Arc::new(arc_swap::ArcSwap::from_pointee(
            lt_proto::MonitorSample::default(),
        ));
    let mut app = lt_ui::MultiWindowApp::new(
        lt_ui::AppUi::new(initial_settings.clone()),
        &event_loop,
        Some(cmd_tx),
        Some(monitor_cell.clone()),
    )?;
    app.kick_ticks();

    let proxy = event_loop.create_proxy();
    // R11②/D-73（WD-5）：单实例——mutex 判已有实例即激活其窗口并退出；
    // message-only 窗在 run_app 泵启动前入队、启动后 WndProc 投 SecondInstance
    //（须在 run_app 前建窗：PostMessage 的队列缓冲依赖窗已存在）
    if let Err(e) = singleton::ensure(proxy.clone()) {
        fatal_early(&format!("{e:#}"));
        return Ok(());
    }
    // ── 事件动脉（W2/INV1：后台 → UI 事件的唯一通路；满丢最旧 cap 4096）──
    let artery = lt_orchestrator::EventArtery::new();
    let app_sup =
        lt_orchestrator::Supervisor::new(lt_orchestrator::supervisor::artery_sink(artery.clone()));
    // 常驻日志桥接：广播 hub → 动脉（日志窗数据源）。
    // W1 起经监督器出生（死亡可见 + 重生）；stop 标志由 main 在停机序置位
    let bridge_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    lt_orchestrator::logging::spawn_bridge(&app_sup, bridge_stop.clone(), artery.clone());
    // 动脉桥线程（W2：UiMsg::Events 批量投递；Always 重生）——队列侧在
    // lt-orchestrator，本文件只留需要 winit 的投递桥（W3 拓扑勘正）
    artery::spawn_bridge(artery.clone(), &app_sup, proxy.clone(), bridge_stop.clone());
    // W4/INV2：lt-backend 线程退役——cmd mpsc 由 AppShell 在 about_to_wait
    // 直排（UI→mpsc→shell 两跳）；下载编排在 shell 内的 DownloadManager

    let mut shell = shell::AppShell::new(
        app,
        artery,
        monitor_cell,
        cmd_rx,
        proxy.clone(),
        app_sup.clone(),
        Some(initial_settings.clone()),
    );

    let result = event_loop.run_app(&mut shell);
    // 收尾序（INV4）：监督器 stopping 先置位（禁 respawn——必须在一切线程停止
    // 信号之前，否则 monitor 节拍可能把正被关闭的桥复位重拉）→ 桥停止标志 →
    // shell（stop 管道 + save settings）→ 监督器 join
    app_sup.begin_shutdown();
    bridge_stop.store(true, std::sync::atomic::Ordering::SeqCst);
    shell.shutdown();
    app_sup.join_all();
    result?;

    tracing::info!("LiveTranslate 退出");
    Ok(())
}

/// 早期启动失败的呈现（W1/R11①/D-65）：此时尚无事件循环线程，同步
/// MessageBoxW 不违反 D-33；双击后「无事发生」的黑洞从此有形
#[cfg(windows)]
fn fatal_early(msg: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    };
    let text: Vec<u16> = format!(
        "LiveTranslate 启动失败\r\n{msg}\r\n\r\n（详情见日志目录；本窗口关闭后应用退出）\0"
    )
    .encode_utf16()
    .collect();
    let caption: Vec<u16> = "LiveTranslate\0".encode_utf16().collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_ICONERROR | MB_OK | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(not(windows))]
fn fatal_early(msg: &str) {
    eprintln!("LiveTranslate 启动失败: {msg}");
}

/// worker 子进程主循环：SenseVoice / Whisper（M5.1）/ Fun-ASR-Nano（WP-A）/ Qwen3-ASR（WP-B）
fn asr_worker_entry() -> anyhow::Result<()> {
    // AH-7/H14：worker 分支在主进程 logging::init 之前 return，引擎里的
    // tracing 此前是无 subscriber 的 no-op（日志黑洞）。初始化写 stderr 的
    // 轻量订阅者——父进程 drain_stderr 捕获后以 debug(target:"asr_worker")
    // 回流日志窗（INFO 级足够：引擎加载/就绪信息，避免 debug 洪泛）
    init_worker_logging();
    // R28/D-76：stdin 首行 = 配置 JSON；BufReader 预读缓冲不丢字节，
    // 帧循环从剩余字节继续
    let mut stdin = std::io::BufReader::new(std::io::stdin().lock());
    let mut cfg_line = String::new();
    std::io::BufRead::read_line(&mut stdin, &mut cfg_line)?;
    let config: lt_asr::WorkerConfig = serde_json::from_str(cfg_line.trim())?;
    match config.engine.as_str() {
        "sensevoice" => lt_asr::worker::run(stdin, std::io::stdout().lock(), config, move |cfg| {
            let dir = match &cfg.options {
                lt_asr::WorkerOptions::ModelDir(d) => d,
                other => anyhow::bail!("sensevoice 应带 ModelDir，实际 {other:?}"),
            };
            lt_asr::sensevoice::SenseVoiceEngine::load(dir, cfg.pad_seconds, &cfg.language)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }),
        "nano" => {
            lt_asr::worker::run(stdin, std::io::stdout().lock(), config, move |cfg| {
                let dir = match &cfg.options {
                    lt_asr::WorkerOptions::ModelDir(d) => d,
                    other => anyhow::bail!("nano 应带 ModelDir，实际 {other:?}"),
                };
                // nano 无 padding 语义（pad_seconds 恒 None），语言为创建期参数
                lt_asr::NanoEngine::load(dir, &cfg.language).map_err(|e| anyhow::anyhow!("{e}"))
            })
        }
        "qwen3" => {
            lt_asr::worker::run(stdin, std::io::stdout().lock(), config, move |cfg| {
                let dir = match &cfg.options {
                    lt_asr::WorkerOptions::ModelDir(d) => d,
                    other => anyhow::bail!("qwen3 应带 ModelDir，实际 {other:?}"),
                };
                // qwen3 无 padding/语言参数（pad_seconds 恒 None，纯 auto-LID）
                lt_asr::Qwen3AsrEngine::load(dir).map_err(|e| anyhow::anyhow!("{e}"))
            })
        }
        "whisper" => lt_asr::worker::run(stdin, std::io::stdout().lock(), config, move |cfg| {
            lt_asr::WhisperEngine::from_config(cfg).map_err(|e| anyhow::anyhow!("{e}"))
        }),
        other => anyhow::bail!("未知 worker 引擎: {other}"),
    }
}

/// worker 进程的轻量日志订阅者（AH-7/H14）：stderr 单路、INFO 级
fn init_worker_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer as _; // with_filter
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
    tracing_subscriber::registry().with(stderr_layer).init();
}
