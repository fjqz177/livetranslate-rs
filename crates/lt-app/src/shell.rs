//! AppShell：组合委托 [`lt_ui::MultiWindowApp`] 的 ApplicationHandler。
//!
//! 在用户事件转交 UI 之前做进程级编排（M2.5）：
//! - `DownloadSucceeded` → 启动管道（向导/缺模型流程完成后；原版向导
//!   accept 后经 `_deferred_init` + 500ms `on_start` 的等价时机）；
//! - 退出时统一 `Pipeline::stop` + 设置保存。

use crate::pipeline::Pipeline;
use lt_proto::{Cmd, Settings, UiEvent, UiMsg};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};

pub struct AppShell {
    pub ui: lt_ui::MultiWindowApp,
    proxy: EventLoopProxy<UiMsg>,
    pipeline: Option<Pipeline>,
    started: bool,
}

impl AppShell {
    /// `start_settings` 为 Some（启动即就绪）时立即启动管道。
    pub fn new(
        ui: lt_ui::MultiWindowApp,
        proxy: EventLoopProxy<UiMsg>,
        start_settings: Option<Settings>,
    ) -> Self {
        let mut shell = Self { ui, proxy, pipeline: None, started: false };
        if let Some(s) = start_settings {
            shell.start_pipeline(s);
        }
        shell
    }

    fn start_pipeline(&mut self, settings: Settings) {
        if self.started {
            return;
        }
        self.started = true;
        match Pipeline::start(&settings, self.proxy.clone()) {
            Ok(p) => self.pipeline = Some(p),
            Err(e) => tracing::error!("管道启动失败: {e:#}"),
        }
    }

    /// 事件循环退出后的统一收尾（main 调用）
    pub fn shutdown(&mut self) {
        if let Some(p) = self.pipeline.as_mut() {
            p.stop();
        }
        if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
            tracing::error!("设置保存失败: {e}");
        }
    }

    /// 管道域命令分发（对照原版 App 的 overlay/托盘信号处理段）
    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Pause => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(true);
                }
                self.ui.app_state.running = false;
                tracing::info!("管道暂停");
            }
            Cmd::Resume => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_paused(false);
                }
                self.ui.app_state.running = true;
                tracing::info!("管道恢复");
            }
            // 挂起机制：UI 线程只存值，ASR 线程在下一次 transcribe 前应用
            Cmd::SetAsrLanguage(lang) => {
                if let Some(p) = &self.pipeline {
                    p.set_pending_language(&lang);
                }
                self.ui.app_state.settings.asr_language = lang.clone();
                tracing::info!("源语言: {lang}");
                self.persist_settings();
            }
            Cmd::SetTargetLanguage(lang) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.set_translator_target_language(&lang);
                }
                self.ui.app_state.settings.target_language = lang.clone();
                tracing::info!("目标语言: {lang}");
                self.persist_settings();
            }
            Cmd::SetTimeout(secs) => {
                if let Some(p) = &self.pipeline {
                    p.set_translator_timeout(secs);
                }
                self.ui.app_state.settings.timeout = secs;
                self.persist_settings();
            }
            Cmd::SwitchTranslator(config) => {
                if let Some(p) = self.pipeline.as_mut() {
                    p.switch_translator(&config, &self.ui.app_state.settings);
                }
                // 记为当前激活模型并持久化
                if let Some(idx) = self
                    .ui
                    .app_state
                    .settings
                    .models
                    .iter()
                    .position(|m| m.name == config.name)
                {
                    self.ui.app_state.settings.active_model = idx;
                }
                tracing::info!("Switching translator: {} ({})", config.name, config.model);
                self.persist_settings();
            }
            Cmd::PersistSettings(settings) => {
                self.ui.app_state.settings = *settings;
                if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
                    tracing::error!("设置保存失败: {e:#}");
                }
            }
            // 其余命令 M4 后续波次接线（音频设备/增量 ASR/引擎切换）
            other => tracing::debug!("命令待后续接线: {other:?}"),
        }
    }

    /// 设置落盘（主线程直写；原子写由 settings_io 保证）
    fn persist_settings(&mut self) {
        if let Err(e) = lt_models::settings_io::save(&self.ui.app_state.settings) {
            tracing::error!("设置保存失败: {e:#}");
        }
    }
}


impl ApplicationHandler<UiMsg> for AppShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.ui.resumed(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        self.ui.window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiMsg) {
        // 编排先于 UI：下载成功 → 管道启动（UI 转场由 MultiWindowApp 处理）
        if let UiMsg::Event(UiEvent::DownloadSucceeded { settings }) = &event {
            self.start_pipeline((**settings).clone());
        }
        // UI 回流的管道域命令（backend 不持有 Pipeline，经 proxy 回环至此）
        if let UiMsg::Cmd(cmd) = &event {
            self.handle_cmd(cmd.clone());
        }
        self.ui.user_event(event_loop, event);
    }


    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.ui.about_to_wait(event_loop);
    }
}
