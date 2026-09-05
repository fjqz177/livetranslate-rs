//! AppShell：组合委托 [`lt_ui::MultiWindowApp`] 的 ApplicationHandler。
//!
//! 在用户事件转交 UI 之前做进程级编排（M2.5）：
//! - `DownloadSucceeded` → 启动管道（向导/缺模型流程完成后；原版向导
//!   accept 后经 `_deferred_init` + 500ms `on_start` 的等价时机）；
//! - 退出时统一 `Pipeline::stop` + 设置保存。

use crate::pipeline::Pipeline;
use lt_proto::{Settings, UiEvent, UiMsg};
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
        self.ui.user_event(event_loop, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.ui.about_to_wait(event_loop);
    }
}
