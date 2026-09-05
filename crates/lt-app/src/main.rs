//! LiveTranslate 主入口（M0.8 装配 + M2.5 启动流）。
//!
//! 启动流（对齐原版 main()）：
//! - 无 settings → 首启向导（hub 按语言默认/代理三选/15s 倒计时自动下载）；
//! - 有 settings 但模型缺失 → 下载对话框（取消即退出）；
//! - 就绪 → 立即启动管道；向导/下载流程在 DownloadSucceeded 时启动（AppShell）。
//! `--asr-worker` 为 worker 子进程入口。

mod backend;
mod logging;
mod pipeline;
mod shell;

use lt_proto::{Cmd, UiMsg};

fn main() -> anyhow::Result<()> {
    // worker 子进程入口：--asr-worker <config-json>（M2.3 起实装 SenseVoice）
    if let Some(cfg) = std::env::args().skip_while(|a| a != "--asr-worker").nth(1) {
        return asr_worker_entry(&cfg);
    }

    // R-4 预案 A：ort 走 load-dynamic，任何 ort 调用前解压 dll 并指向它
    lt_pipeline::ensure_ort_dylib()?;

    ensure_single_instance()?;

    let settings_opt = lt_models::settings_io::load()?;
    let initial_settings = settings_opt.clone().unwrap_or_default();

    // 启动流判定（原版：SETTINGS_FILE 不存在 → 向导；否则 get_missing_models
    // 非空 → 下载对话框；silero 内嵌恒不缺 D-5）
    let (first_launch, missing) = match &settings_opt {
        None => (true, Vec::new()),
        Some(s) => {
            let models_dir = lt_models::paths::models_dir(s.models_dir.as_deref())?;
            (
                false,
                lt_models::cache::missing_models(
                    &models_dir,
                    &s.asr_engine,
                    &s.funasr_model,
                    &s.whisper_model_size,
                ),
            )
        }
    };
    let ready_now = !first_launch && missing.is_empty();
    let flow = lt_ui::startup_flow(
        first_launch,
        missing.iter().map(|m| m.display.clone()).collect(),
    );

    lt_i18n::set_lang(&initial_settings.ui_lang);
    logging::init()?;

    let event_loop = winit::event_loop::EventLoop::<UiMsg>::with_user_event().build()?;
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let mut app = lt_ui::MultiWindowApp::new(
        lt_ui::AppState::with_startup(initial_settings.clone(), flow),
        &event_loop,
        Some(cmd_tx),
    )?;
    app.kick_ticks();

    let proxy = event_loop.create_proxy();
    // 常驻日志桥接（缺口 #4）：广播 hub → LogLine 事件（日志窗数据源）
    logging::spawn_bridge(proxy.clone());
    // 后台命令线程：下载编排 + 下载期日志转发（M2.5）
    backend::spawn(cmd_rx, proxy.clone(), first_launch, initial_settings.clone(), missing);

    let mut shell = shell::AppShell::new(
        app,
        proxy,
        ready_now.then(|| initial_settings.clone()),
    );

    let result = event_loop.run_app(&mut shell);

    shell.shutdown();
    result?;

    tracing::info!("LiveTranslate 退出");
    Ok(())
}

/// 单实例：命名互斥量（先 Open 探测已存在的，再 Create 持有至进程退出）
#[cfg(windows)]
fn ensure_single_instance() -> anyhow::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Threading::{
        CreateMutexW, SYNCHRONIZATION_SYNCHRONIZE,
    };

    // 裸名（无反斜杠）：落在会话 BaseNamedObjects 根，
    // 带斜杠的名字会被对象命名空间当子目录解析而报 0x80070003
    let name: Vec<u16> = "LiveTranslateSingleInstance\0".encode_utf16().collect();
    let pcw = PCWSTR(name.as_ptr());
    unsafe {
        // 已有实例持有同名互斥量 → Open 成功即拒绝启动
        if windows::Win32::System::Threading::OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE,
            false,
            pcw,
        )
        .is_ok()
        {
            anyhow::bail!("LiveTranslate 已在运行（单实例互斥量已存在）");
        }
        // HANDLE 是裸包装无 Drop：不关即存活到进程退出，由 OS 回收
        let _handle = CreateMutexW(None, true, pcw)
            .map_err(|e| anyhow::anyhow!("创建单实例互斥量失败: {e}"))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_single_instance() -> anyhow::Result<()> {
    Ok(())
}

/// worker 子进程主循环：SenseVoice（whisper 引擎 M5 扩展）
fn asr_worker_entry(cfg_json: &str) -> anyhow::Result<()> {
    let config: lt_asr::WorkerConfig = serde_json::from_str(cfg_json)?;
    match config.engine.as_str() {
        "sensevoice" => {
            lt_asr::worker::run(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                config,
                move |cfg| {
                    let dir = cfg
                        .options
                        .get("model_dir")
                        .and_then(|v| v.as_str())
                        .map(std::path::PathBuf::from)
                        .ok_or_else(|| anyhow::anyhow!("缺少 model_dir"))?;
                    lt_asr::sensevoice::SenseVoiceEngine::load(&dir, cfg.pad_seconds, &cfg.language)
                        .map_err(|e| anyhow::anyhow!("{e}"))
                },
            )
        }
        other => anyhow::bail!("未知 worker 引擎: {other}"),
    }
}
