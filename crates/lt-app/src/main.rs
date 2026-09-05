//! LiveTranslate 主入口（M0.8 装配）。
//!
//! 启动流（对齐原版 main()，向导/下载对话框在 M2 插入）：
//! 单实例检查 → 设置加载（无配置先走默认）→ i18n → 日志三路 → 多窗口 UI + 托盘。
//! `--asr-worker` 为 worker 子进程入口（M2 实装）。

mod logging;
mod pipeline;

use lt_proto::UiMsg;

fn main() -> anyhow::Result<()> {
    // worker 子进程入口：--asr-worker <config-json>（M2.3 起实装 SenseVoice）
    if let Some(cfg) = std::env::args().skip_while(|a| a != "--asr-worker").nth(1) {
        return asr_worker_entry(&cfg);
    }

    // R-4 预案 A：ort 走 load-dynamic，任何 ort 调用前解压 dll 并指向它
    lt_pipeline::ensure_ort_dylib()?;

    ensure_single_instance()?;

    let settings = lt_models::settings_io::load()?.unwrap_or_default();
    lt_i18n::set_lang(&settings.ui_lang);
    logging::init()?;

    let event_loop = winit::event_loop::EventLoop::<UiMsg>::with_user_event().build()?;
    let mut app = lt_ui::MultiWindowApp::new(
        lt_ui::AppState::new(settings.clone()),
        &event_loop,
        None, // cmd_tx：M4 面板控件接入时注入
    )?;
    app.kick_ticks();

    // 管道：capture → VAD → ASR → UI 事件（M2.6）
    let proxy = event_loop.create_proxy();
    let mut pipeline = pipeline::Pipeline::start(&settings, proxy)?;
    tracing::info!("LiveTranslate 启动（4 窗口 + 托盘 + 管道）");

    let result = event_loop.run_app(&mut app);

    pipeline.stop();
    result?;

    // 退出前保存设置（窗口可见性/托盘开关等运行态）
    if let Err(e) = lt_models::settings_io::save(&app.app_state.settings) {
        tracing::error!("设置保存失败: {e}");
    }
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
