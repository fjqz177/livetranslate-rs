//! LiveTranslate 主入口。
//!
//! 启动流（对齐新版原版 2026-09-01 less-is-more 决策，first-launch-flow.md）：
//! **没有首启向导、没有缺模型下载门**——打开即进主界面并启动管道；
//! 模型未缓存时管道发 AsrUnavailable（悬浮窗显示 unavailable），用户经
//! 设置 → 识别页按需下载（StartDownload 命令复用 M2.5 下载管线）。
//! `--asr-worker` 为 worker 子进程入口。

mod backend;
mod logging;
mod pipeline;
mod shell;

use lt_proto::{Cmd, UiMsg};

fn main() -> anyhow::Result<()> {
    // worker 子进程入口：--asr-worker <config-json>（sensevoice + whisper）
    if let Some(cfg) = std::env::args().skip_while(|a| a != "--asr-worker").nth(1) {
        return asr_worker_entry(&cfg);
    }

    // R-4 预案 A：ort 走 load-dynamic，任何 ort 调用前解压 dll 并指向它
    lt_pipeline::ensure_ort_dylib()?;

    ensure_single_instance()?;

    // 无 settings 文件 → 全默认值（直进主界面；模型缺失走识别页按需下载）
    let initial_settings = lt_models::settings_io::load()?.unwrap_or_default();

    // ui_lang="system" → 系统语言解析（新版 general_tab 的 resolve_ui_lang 语义）
    let ui_lang = initial_settings.ui_lang.clone();
    lt_i18n::set_lang(if ui_lang == "system" {
        lt_i18n::detect_system_lang()
    } else {
        &ui_lang
    });
    logging::init()?;

    let event_loop = winit::event_loop::EventLoop::<UiMsg>::with_user_event().build()?;
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let mut app = lt_ui::MultiWindowApp::new(
        lt_ui::AppState::new(initial_settings.clone()),
        &event_loop,
        Some(cmd_tx),
    )?;
    app.kick_ticks();

    let proxy = event_loop.create_proxy();
    // 常驻日志桥接：广播 hub → LogLine 事件（日志窗数据源）
    logging::spawn_bridge(proxy.clone());
    // 后台命令线程：下载编排（识别页/面板触发）+ 下载期日志转发。
    // 下载目标不在此快照：backend 维护 settings 镜像，StartDownload 时按当前
    // 引擎/档位现场重算（运行中切换后下载的才是所选模型，M5.1）。
    backend::spawn(cmd_rx, proxy.clone(), false, initial_settings.clone());

    let mut shell = shell::AppShell::new(app, proxy, Some(initial_settings.clone()));

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

/// worker 子进程主循环：SenseVoice / Whisper（M5.1）
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
        "whisper" => {
            lt_asr::worker::run(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                config,
                move |cfg| {
                    lt_asr::WhisperEngine::from_config(cfg)
                        .map_err(|e| anyhow::anyhow!("{e}"))
                },
            )
        }
        other => anyhow::bail!("未知 worker 引擎: {other}"),
    }
}
