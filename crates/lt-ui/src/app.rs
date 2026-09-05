//! 多窗口宿主（M0.4）：一个 winit 事件循环 + 共享 egui Context + egui-wgpu Painter
//! （Painter 原生支持多 surface，按 ViewportId 管理）。
//!
//! 设计要点（对照 PLAN.md §2.1/2.2）：
//! - 4 个常驻原生窗口：overlay（透明/无边框/置顶/跳任务栏/不抢焦点）、
//!   subtitle（透明/无边框/置顶）、panel/log（常规装饰窗口）；
//! - 每窗口独立 `egui_winit::State`（独立 viewport id/DPI/输入）；
//! - 空闲低 CPU：`ControlFlow::WaitUntil` + AppState 节拍表 + egui 即时重绘请求；
//! - CloseRequested 一律隐藏窗口（退出仅走托盘 Quit，等价 setQuitOnLastWindowClosed(false)）；
//! - 点击穿透由窗口层按 50ms 轮询处理（M4 接入），本宿主只负责窗口创建与 flags。

use crate::state::{AppState, OverlayMessage, StartupFlow, TickKind, WinAction, WinId, push_log_line};
use crate::tray::{self, Tray};
use crate::windows;
use egui::{Context, ViewportId};
use egui_wgpu::winit::Painter;
use lt_proto::UiMsg;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{ResizeDirection, Window, WindowLevel};

#[cfg(windows)]
use winit::platform::windows::{WindowAttributesExtWindows, WindowExtWindows};

/// 单个窗口的全部伴生状态
struct HostedWindow {
    id: WinId,
    window: Arc<Window>,
    state: egui_winit::State,
}

pub struct MultiWindowApp {
    pub app_state: AppState,
    pub ctx: Context,
    painter: Painter,
    windows: Vec<HostedWindow>,
    pub tray: Option<Tray>,
    proxy: EventLoopProxy<UiMsg>,
}

impl MultiWindowApp {
    /// 创建宿主（需在主线程）。异步的 wgpu 初始化用 pollster 阻塞完成。
    /// cmd_tx 存入 AppState（widget 代码经 send_cmd 直接发送命令）。
    pub fn new(
        mut app_state: AppState,
        event_loop: &EventLoop<UiMsg>,
        cmd_tx: Option<std::sync::mpsc::Sender<lt_proto::Cmd>>,
    ) -> anyhow::Result<Self> {
        app_state.cmd_tx = cmd_tx;
        let ctx = Context::default();
        install_cjk_fonts(&ctx);
        let painter = pollster::block_on(Painter::new(
            ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            true, // support_transparent_backbuffer：悬浮窗/字幕窗必需
            egui_wgpu::RendererOptions::default(),
        ));
        let proxy = event_loop.create_proxy();
        Ok(Self {
            app_state,
            ctx,
            painter,
            windows: Vec::new(),
            tray: None,
            proxy,
        })
    }

    fn viewport_of(id: WinId) -> ViewportId {
        ViewportId::from_hash_of(id.key())
    }

    fn find(&self, id: WinId) -> Option<&HostedWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn find_mut(&mut self, id: WinId) -> Option<&mut HostedWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// 创建全部窗口并建立托盘（resumed 时调用一次）
    fn setup(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        self.create_window(event_loop, WinId::Overlay, (620, 500))?;
        self.create_window(event_loop, WinId::Subtitle, (1000, 160))?;
        self.create_window(event_loop, WinId::Panel, (520, 650))?;
        self.create_window(event_loop, WinId::Log, (900, 500))?;
        // 启动流对话框（原版 QDialog：常规装饰窗口；可见性 = 启动流进行中）
        self.create_window(event_loop, WinId::Setup, (560, 420))?;
        // Setup 标题随启动流阶段动态化（向导/缺模型下载；加载框在 ModelLoadStart 再设）
        let setup_title = match &self.app_state.startup {
            StartupFlow::Wizard(_) => lt_i18n::t("window_setup"),
            StartupFlow::DownloadMissing { .. } => lt_i18n::t("window_download"),
            StartupFlow::Ready => "LiveTranslate".to_string(),
        };
        self.set_setup_title(&setup_title);

        // 托盘（主线程创建；事件经 proxy 回流）
        let proxy = self.proxy.clone();
        let sender = move |msg: UiMsg| {
            let _ = proxy.send_event(msg);
        };
        self.tray = Some(tray::build(Arc::new(sender))?);
        self.sync_tray_checks();
        self.apply_overlay_flags();
        Ok(())
    }

    fn create_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WinId,
        size: (u32, u32),
    ) -> anyhow::Result<()> {
        let mut attrs = Window::default_attributes()
            .with_title(id.title())
            .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
            .with_resizable(true);
        if matches!(id, WinId::Overlay | WinId::Subtitle) {
            attrs = attrs
                .with_transparent(true)
                .with_decorations(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_active(false); // WA_ShowWithoutActivating 等价
            #[cfg(windows)]
            {
                attrs = attrs.with_skip_taskbar(true);
            }
        }
        if id == WinId::Overlay {
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(480.0, 200.0));
        }
        let visible = *self.app_state.visible.get(&id).unwrap_or(&true);
        attrs = attrs.with_visible(visible);

        let window = Arc::new(event_loop.create_window(attrs)?);
        // 悬浮窗恢复上次几何（原版 move(x,y)+resize(w,h)；位置需多屏可见校验 E-11）
        if id == WinId::Overlay {
            if let Some(geo) = self.app_state.overlay_geometry() {
                let (x, y, w, h) = geo;
                let visible_on_monitor = event_loop.available_monitors().any(|m| {
                    let (mp, ms) = (m.position(), m.size());
                    let mx2 = mp.x + ms.width as i32;
                    let my2 = mp.y + ms.height as i32;
                    // 至少 80px 主体落在某显示器内
                    x + w as i32 > mp.x + 80 && x < mx2 - 80 && y + h as i32 > mp.y + 80 && y < my2 - 80
                });
                if visible_on_monitor {
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64));
                    window.set_outer_position(winit::dpi::LogicalPosition::new(x as f64, y as f64));
                }
            }
        }
        let viewport = Self::viewport_of(id);
        pollster::block_on(self.painter.set_window(viewport, Some(window.clone())))?;
        let state = egui_winit::State::new(
            self.ctx.clone(),
            viewport,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        self.windows.push(HostedWindow { id, window, state });
        if visible {
            self.window(id).request_redraw();
        }
        Ok(())
    }

    fn window(&self, id: WinId) -> &Arc<Window> {
        &self.find(id).expect("窗口已创建").window
    }

    /// 运行一帧：输入 → run_ui（UI 闭包）→ 平台输出 → 光栅化 → 上屏
    fn run_frame(&mut self, id: WinId) {
        let Some(pos) = self.windows.iter().position(|w| w.id == id) else {
            return;
        };
        let ctx = self.ctx.clone();
        let viewport = Self::viewport_of(id);
        let ppp = self.windows[pos].window.scale_factor() as f32;

        // 取输入（借 windows），随后 UI 闭包只借 app_state，避免借用冲突
        let input = {
            let hw = &mut self.windows[pos];
            hw.state.take_egui_input(&hw.window)
        };
        let app_state = &mut self.app_state;
        let mut full = ctx.run_ui(input, |ui| windows::dispatch(id, ui, app_state));

        {
            let hw = &mut self.windows[pos];
            hw.state
                .handle_platform_output(&hw.window, std::mem::take(&mut full.platform_output));
        }
        let clipped = ctx.tessellate(full.shapes, ppp);
        let mut textures = full.textures_delta;
        let window = self.windows[pos].window.clone();
        self.painter.paint_and_update_textures(
            viewport,
            ppp,
            clear_color_for(id),
            &clipped,
            &mut textures,
            Vec::new(),
            &window,
        );

        // egui 请求了立即重绘（动画/交互进行中）→ 跟一帧
        let want_repaint = full
            .viewport_output
            .get(&viewport.into())
            .map(|vo| vo.repaint_delay == Duration::ZERO)
            .unwrap_or(false);
        if want_repaint {
            window.request_redraw();
        }
        // 帧后：处理窗口动作 / 右键导出 / 清空请求
        self.process_actions();
        if id == WinId::Overlay {
            if let Some(mode) = self.app_state.overlay.export_request.take() {
                self.run_export(&mode);
            }
            if self.app_state.clear_request {
                self.app_state.clear_request = false;
                self.app_state.messages.clear();
            }
        }
    }

    /// 把 AppState 的勾选状态同步到托盘菜单（三向同步的一环）
    fn sync_tray_checks(&mut self) {
        let Some(t) = &self.tray else { return };
        let s = &self.app_state;
        let _ = t
            .handles
            .subwin
            .set_checked(*s.visible.get(&WinId::Subtitle).unwrap_or(&false));
        let _ = t.handles.subwin_ct.set_checked(s.settings.subtitle_mode.click_through);
        let _ = t.handles.ov_ct.set_checked(s.ov_click_through);
        let _ = t.handles.ov_topmost.set_checked(s.ov_topmost);
        let _ = t.handles.ov_autoscroll.set_checked(s.ov_auto_scroll);
        let _ = t.handles.ov_taskbar.set_checked(s.ov_taskbar);
        for (code, item) in &t.handles.langs {
            let _ = item.set_checked(*code == s.settings.target_language);
        }
        for (code, item) in &t.handles.asr_langs {
            let _ = item.set_checked(*code == s.settings.asr_language);
        }
    }

    /// 托盘菜单动作
    fn on_menu(&mut self, event_loop: &ActiveEventLoop, id: &str) {
        use tray::ids as m;
        match id {
            m::PAUSE => {
                self.app_state.running = !self.app_state.running;
                let text = if self.app_state.running {
                    lt_i18n::t("tray_pause")
                } else {
                    lt_i18n::t("tray_resume")
                };
                if let Some(t) = &self.tray {
                    let _ = t.handles.pause.set_text(text);
                }
                tracing::info!("管道 {}", if self.app_state.running { "运行" } else { "暂停" });
            }
            m::OVERLAY_TOGGLE => {
                let vis = !self.window(WinId::Overlay).is_visible().unwrap_or(false);
                self.set_visible(WinId::Overlay, vis);
            }
            m::SUBWIN_TOGGLE => {
                let vis = !self.window(WinId::Subtitle).is_visible().unwrap_or(false);
                self.set_visible(WinId::Subtitle, vis);
                self.app_state.settings.subtitle_mode.enabled = vis;
            }
            m::SUBWIN_CT => {
                let cur = self.app_state.settings.subtitle_mode.click_through;
                self.app_state.settings.subtitle_mode.click_through = !cur;
            }
            m::SHOW_PANEL => {
                let vis = !self.window(WinId::Panel).is_visible().unwrap_or(false);
                self.set_visible(WinId::Panel, vis);
            }
            m::SHOW_LOG => {
                let vis = !self.window(WinId::Log).is_visible().unwrap_or(false);
                self.set_visible(WinId::Log, vis);
            }
            m::OV_CT => self.app_state.ov_click_through = !self.app_state.ov_click_through,
            m::OV_TOPMOST => self.app_state.ov_topmost = !self.app_state.ov_topmost,
            m::OV_AUTOSCROLL => self.app_state.ov_auto_scroll = !self.app_state.ov_auto_scroll,
            m::OV_TASKBAR => self.app_state.ov_taskbar = !self.app_state.ov_taskbar,
            m::QUIT => {
                tracing::info!("收到退出指令");
                event_loop.exit();
            }
            other => {
                if let Some(code) = other.strip_prefix("lang_") {
                    self.app_state.settings.target_language = code.to_string();
                    tracing::info!("目标语言: {code}");
                } else if let Some(code) = other.strip_prefix("asrlang_") {
                    self.app_state.settings.asr_language = code.to_string();
                    tracing::info!("源语言提示: {code}");
                }
            }
        }
        self.sync_tray_checks();
        self.apply_overlay_flags();
    }

    fn set_visible(&mut self, id: WinId, vis: bool) {
        if let Some(hw) = self.find_mut(id) {
            if vis {
                hw.window.set_visible(true);
                hw.window.focus_window();
                hw.window.request_redraw();
            } else {
                hw.window.set_visible(false);
            }
        }
        self.app_state.visible.insert(id, vis);
    }

    /// 应用悬浮窗置顶/任务栏窗口 flags（穿透轮询 M4 接入）
    fn apply_overlay_flags(&mut self) {
        let Some(hw) = self.find(WinId::Overlay) else { return };
        let level = if self.app_state.ov_topmost {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        hw.window.set_window_level(level);
        #[cfg(windows)]
        {
            hw.window.set_skip_taskbar(!self.app_state.ov_taskbar);
        }
    }

    /// Setup 窗标题动态化（向导/缺模型下载/加载框共用一个原生窗口）
    fn set_setup_title(&mut self, title: &str) {
        if let Some(hw) = self.find_mut(WinId::Setup) {
            hw.window.set_title(title);
        }
    }

    /// 重绘 Setup 窗（下载日志追加/失败/成功/加载框开关等状态变化时）
    fn redraw_setup(&mut self) {
        if let Some(hw) = self.find_mut(WinId::Setup) {
            hw.window.request_redraw();
        }
    }

    /// 关闭模型加载对话框（ModelLoadDone / AsrDevice / AsrUnavailable 触发）。
    /// 仅 load_dialog 显示中才动作，且仅 startup==Ready 时隐藏 Setup 窗——
    /// 避免误关首启向导/缺模型下载流程仍在使用的窗口。
    fn close_load_dialog(&mut self) {
        if self.app_state.load_dialog.take().is_some()
            && matches!(self.app_state.startup, StartupFlow::Ready)
        {
            self.set_visible(WinId::Setup, false);
        }
    }

    /// Setup 窗节拍分派：向导倒计时（1s 一拍）与启动流成功后的 500ms 收尾延迟。
    /// 定时仅在向导 Idle / 成功待收尾期间存在（事件到达即重绘，无需节拍）。
    fn on_setup_tick(&mut self) {
        enum Action {
            Countdown,
            Finish,
            None,
        }
        let action = match &self.app_state.startup {
            StartupFlow::Wizard(w) => match w.phase {
                crate::state::WizardPhase::Idle => Action::Countdown,
                crate::state::WizardPhase::Done => Action::Finish,
                _ => Action::None,
            },
            StartupFlow::DownloadMissing { finished, .. } => {
                if *finished { Action::Finish } else { Action::None }
            }
            StartupFlow::Ready => Action::None,
        };
        match action {
            Action::Countdown => {
                if let StartupFlow::Wizard(w) = &mut self.app_state.startup {
                    w.countdown -= 1;
                }
                // 原版 _tick_countdown：归零即自动开始下载，否则按 1s 续拍
                if matches!(&self.app_state.startup, StartupFlow::Wizard(w) if w.countdown <= 0) {
                    self.app_state.wizard_auto_start();
                } else {
                    self.app_state.schedule_setup_tick(Duration::from_secs(1));
                }
            }
            Action::Finish => {
                self.finish_startup();
                return; // 窗已隐藏，无需重绘
            }
            Action::None => {}
        }
        self.redraw_setup(); // 刷新倒计时文本等
    }

    /// 启动流收尾（原版对话框 accept 之后）：startup=Ready、关 Setup 窗、
    /// 揭开主窗口（字幕窗按 settings.subtitle_mode.enabled，日志窗保持隐藏）
    fn finish_startup(&mut self) {
        self.app_state.startup = StartupFlow::Ready;
        // 下载成功即启管道（AppShell），ModelLoadStart 可能落在 500ms 收尾期内——
        // 原版两个对话框先后出现，这里共用一个原生窗口，故加载框已开则保留窗口
        if self.app_state.load_dialog.is_none() {
            self.set_visible(WinId::Setup, false);
        }
        let subtitle = self.app_state.settings.subtitle_mode.enabled;
        self.set_visible(WinId::Overlay, true);
        self.set_visible(WinId::Subtitle, subtitle);
        self.set_visible(WinId::Panel, true);
        self.app_state.visible.insert(WinId::Log, false);
        self.sync_tray_checks();
    }

    /// UiMsg 统一入口（管道事件/托盘事件）
    fn on_msg(&mut self, event_loop: &ActiveEventLoop, msg: UiMsg) {
        match msg {
            UiMsg::Menu(id) => self.on_menu(event_loop, &id),
            UiMsg::Tray(_t) => {}
            // 管道域命令由 lt-app::AppShell.user_event 处理（本层无 Pipeline）
            UiMsg::Cmd(_) => {}
            UiMsg::Event(e) => match e {
                // 监视条数据（capture 线程每 chunk 一条 → 节流重绘）
                lt_proto::UiEvent::UpdateMonitor { rms, vad, mic_rms } => {
                    let m = &mut self.app_state.monitor;
                    m.rms = rms;
                    m.vad = vad;
                    m.mic_rms = mic_rms;
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // 新识别消息 → 追加到悬浮窗消息链并重绘
                lt_proto::UiEvent::AddMessage { id, timestamp, original, lang, asr_ms } => {
                    self.app_state.push_message(OverlayMessage {
                        id,
                        timestamp,
                        original,
                        lang,
                        asr_ms,
                        translation: None,
                        tl_ms: 0.0,
                        streaming: false,
                    });
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // 流式译文增量（原版 update_streaming；50ms 节流渲染随 M4）
                lt_proto::UiEvent::UpdateStreaming { id, partial } => {
                    self.app_state.update_streaming(id, partial);
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // 译文完成（含错误文本/同语言空串；原版 update_translation）
                lt_proto::UiEvent::UpdateTranslation { id, text, tl_ms } => {
                    self.app_state.update_translation(id, text, tl_ms);
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // 翻译/用量统计（原版 update_stats）
                lt_proto::UiEvent::UpdateStats { asr_n, tl_n, prompt_tokens, completion_tokens, cost } => {
                    self.app_state.update_stats(crate::state::OverlayStats {
                        asr_n,
                        tl_n,
                        prompt_tokens,
                        completion_tokens,
                        cost,
                    });
                }
                // ASR 设备标签（悬浮窗 MonitorBar device 段）；同时视作加载框关闭信号
                //（原版 App.model_load_done 在设备就绪/不可用时都会被调用）
                lt_proto::UiEvent::AsrDevice(label) => {
                    self.app_state.asr_label = Some(label);
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                    self.close_load_dialog();
                }
                // ASR 完全不可用（沿用原版字面文案）；同样关闭加载框
                lt_proto::UiEvent::AsrUnavailable => {
                    self.app_state.asr_label = Some("ASR unavailable".into());
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                    self.close_load_dialog();
                }
                // ── 启动流：下载日志流（向导/缺模型对话框共用）──
                lt_proto::UiEvent::DownloadProgress(line) => {
                    match &mut self.app_state.startup {
                        StartupFlow::Wizard(w) => push_log_line(&mut w.log, line),
                        StartupFlow::DownloadMissing { log, .. } => push_log_line(log, line),
                        StartupFlow::Ready => {}
                    }
                    self.redraw_setup();
                }
                // ── 启动流：下载失败（可重试；恢复控件 / 显示"关闭"按钮）──
                lt_proto::UiEvent::DownloadFailed(e) => {
                    let failed_line = lt_i18n::t("download_failed").replace("{error}", &e);
                    match &mut self.app_state.startup {
                        StartupFlow::Wizard(w) => {
                            w.phase = crate::state::WizardPhase::Failed;
                            push_log_line(&mut w.log, failed_line);
                        }
                        StartupFlow::DownloadMissing { failed, log, .. } => {
                            *failed = Some(e);
                            push_log_line(log, failed_line);
                        }
                        StartupFlow::Ready => {}
                    }
                    self.redraw_setup();
                }
                // ── 启动流：下载成功（应用下发设置 + 500ms 后收尾关窗）──
                lt_proto::UiEvent::DownloadSucceeded { settings } => {
                    self.app_state.settings = *settings;
                    let done_line = lt_i18n::t("download_complete");
                    match &mut self.app_state.startup {
                        StartupFlow::Wizard(w) => {
                            push_log_line(&mut w.log, done_line);
                            w.phase = crate::state::WizardPhase::Done;
                        }
                        StartupFlow::DownloadMissing { log, finished, .. } => {
                            push_log_line(log, done_line);
                            *finished = true;
                        }
                        StartupFlow::Ready => {}
                    }
                    // 原版 QTimer.singleShot(500, accept)：安排 500ms 收尾节拍
                    self.app_state.cancel_setup_tick();
                    self.app_state.schedule_setup_tick(Duration::from_millis(500));
                    self.redraw_setup();
                }
                // ── 模型加载对话框打开（标题固定 "LiveTranslate"）──
                lt_proto::UiEvent::ModelLoadStart(label) => {
                    self.app_state.load_dialog = Some(label);
                    self.set_visible(WinId::Setup, true);
                    self.set_setup_title("LiveTranslate");
                    self.redraw_setup();
                }
                // ── 模型加载结束：关闭加载框（仅 load_dialog 显示中才动作）──
                lt_proto::UiEvent::ModelLoadDone { .. } => self.close_load_dialog(),
                // M1：其余管道事件尚未接入（M2 起逐个接线）；先落日志防黑洞
                other => tracing::debug!("UI 事件（待接线）: {other:?}"),
            },
        }
    }

    /// 重绘指定窗口（节拍/动作路径的便捷入口）
    fn redraw(&mut self, id: WinId) {
        if let Some(hw) = self.find(id) {
            hw.window.request_redraw();
        }
    }

    /// 消费 UI 帧入队的窗口动作（run_frame 尾部调用；winit 句柄操作在此）
    fn process_actions(&mut self) {
        for (win, action) in self.app_state.drain_actions() {
            let Some(hw) = self.find(win) else { continue };
            let window = hw.window.clone();
            match action {
                WinAction::Drag => {
                    let _ = window.drag_window();
                }
                WinAction::ResizeSouthEast => {
                    let _ = window.drag_resize_window(ResizeDirection::SouthEast);
                }
                WinAction::Hide => {
                    self.set_visible(win, false);
                }
                WinAction::ShowPanel => {
                    self.set_visible(WinId::Panel, true);
                }
                WinAction::ToggleSubtitle => {
                    let vis = self.app_state.settings.subtitle_mode.enabled;
                    self.set_visible(WinId::Subtitle, vis);
                }
                WinAction::ApplyOverlayFlags => {
                    self.apply_overlay_flags();
                }
                WinAction::ToggleMode => {
                    let compact =
                        self.app_state.overlay.mode == crate::state::OverlayMode::Compact;
                    let cur_h = window.inner_size().to_logical::<f32>(window.scale_factor()).height;
                    let (from, to) = if compact {
                        self.app_state.overlay.height_before_compact = Some(cur_h);
                        (cur_h, 200.0) // 200 = 原版 minimumHeight
                    } else {
                        (cur_h, self.app_state.overlay.height_before_compact.unwrap_or(500.0))
                    };
                    // 差距过小直接落位（原版 abs(actual-target)<10 分支）
                    if (from - to).abs() < 10.0 {
                        self.enqueue_height(to);
                    } else {
                        self.app_state.overlay.anim = Some(crate::state::HeightAnim {
                            from,
                            to,
                            start: Instant::now(),
                        });
                    }
                }
                WinAction::SetHeight(h) => {
                    self.enqueue_height(h);
                }
            }
        }
        // 动画推进：结束帧落定终值并清除（进行中由 UI 帧投递 SetHeight）
        if let Some(anim) = self.app_state.overlay.anim {
            if anim.current(Instant::now()).is_none() {
                let target = anim.to;
                self.app_state.overlay.anim = None;
                self.enqueue_height(target);
            }
        }
    }

    /// 当前悬浮窗逻辑几何 (x, y, w, h)（防抖登记用）
    fn overlay_geo(&self) -> (i32, i32, u32, u32) {
        let Some(hw) = self.find(WinId::Overlay) else {
            return (0, 0, 0, 0);
        };
        let scale = hw.window.scale_factor() as f32;
        let pos = hw.window.outer_position().unwrap_or_default();
        let logical = hw.window.inner_size().to_logical::<f32>(hw.window.scale_factor());
        (
            (pos.x as f32 / scale) as i32,
            (pos.y as f32 / scale) as i32,
            logical.width as u32,
            logical.height as u32,
        )
    }

    /// 保持宽度调整窗口高度（逻辑 px；下限 200 = 原版 min height）
    fn enqueue_height(&mut self, h: f32) {
        let Some(hw) = self.find(WinId::Overlay) else { return };
        let scale = hw.window.scale_factor();
        let w = hw.window.inner_size().to_logical::<f32>(scale).width;
        let _ = hw
            .window
            .request_inner_size(winit::dpi::LogicalSize::new(w, h.max(200.0)));
    }

    /// 位置/尺寸防抖到期：读几何（逻辑 px）写设置并持久化（原版 position_changed）
    fn on_pos_save_tick(&mut self) {
        let Some(hw) = self.find(WinId::Overlay) else { return };
        let window = hw.window.clone();
        let scale = window.scale_factor() as f32;
        let Ok(pos) = window.outer_position() else { return };
        let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
        let x = (pos.x as f32 / scale) as i32;
        let y = (pos.y as f32 / scale) as i32;
        let w = logical.width as u32;
        let h = logical.height as u32;
        let geo = (x, y, w, h);
        self.app_state.overlay.pos_dirty_since = None;
        if self.app_state.overlay.last_saved_geo == Some(geo) {
            return;
        }
        self.app_state.overlay.last_saved_geo = Some(geo);
        self.app_state.settings.overlay_x = Some(x);
        self.app_state.settings.overlay_y = Some(y);
        self.app_state.settings.overlay_w = Some(w);
        self.app_state.settings.overlay_h = Some(h);
        self.app_state.send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
            self.app_state.settings.clone(),
        )));
    }

    /// 穿透轮询（原版 _check_click_through 50ms）：光标在头部区（消息区之上）
    /// 时可交互，否则正文穿透。仅 Windows。
    #[cfg(windows)]
    fn poll_click_through(&mut self) {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(hw) = self.find(WinId::Overlay) else { return };
        let window = hw.window.clone();
        let enabled = self.app_state.ov_click_through;
        if !enabled {
            Self::set_overlay_transparent(&window, false);
            return;
        }
        let Ok(win_pos) = window.outer_position() else { return };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let scale = window.scale_factor() as f64;
        let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
        let local_x = (pt.x as f64 - win_pos.x as f64) / scale;
        let local_y = (pt.y as f64 - win_pos.y as f64) / scale;
        let header_px = self.app_state.overlay.header_px as f64;
        let in_header = local_x >= 0.0
            && local_x <= logical.width as f64
            && local_y >= 0.0
            && local_y < header_px;
        Self::set_overlay_transparent(&window, !in_header);
    }

    /// WS_EX_TRANSPARENT 位切换。注：原版 E-04 要求与 WS_EX_LAYERED 成对
    /// （Qt 语义）；winit+wgpu 透明窗口走 DWM 合成，加 LAYERED 反而破坏
    /// surface 呈现，故仅切 TRANSPARENT 位（已知偏差，实机走查项）。
    #[cfg(windows)]
    fn set_overlay_transparent(window: &Window, enable: bool) {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        use ::windows::Win32::Foundation::HWND;
        use ::windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_TRANSPARENT,
        };
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else { return };
        let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
        unsafe {
            let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let new_style = if enable {
                style | WS_EX_TRANSPARENT.0
            } else {
                style & !WS_EX_TRANSPARENT.0
            };
            if new_style != style {
                SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
            }
        }
    }

    /// 非 Windows 兜底（无穿透能力）
    #[cfg(not(windows))]
    fn poll_click_through(&mut self) {}

    /// 执行导出（原版 export_messages；rfd 保存对话框 + 三种模式行格式）
    fn run_export(&mut self, mode: &str) {
        if self.app_state.messages.is_empty() {
            tracing::info!("{}", lt_i18n::t("export_empty"));
            return;
        }
        let suffix = match mode {
            "original" => "original",
            "translation" => "translation",
            _ => "all",
        };
        let default_name = format!(
            "livetrans_{}_{}.txt",
            chrono::Local::now().format("%Y%m%d_%H%M%S"),
            suffix
        );
        let Some(path) = rfd::FileDialog::new()
            .set_title(lt_i18n::t("export_dialog_title"))
            .set_file_name(&default_name)
            .add_filter("Text", &["txt"])
            .save_file()
        else {
            return;
        };
        let mut lines = Vec::new();
        for msg in &self.app_state.messages {
            let ts = &msg.timestamp;
            let orig = msg.original.trim();
            let trans = msg.translation.as_deref().unwrap_or("").trim();
            match mode {
                "original" => lines.push(format!("[{ts}] {orig}")),
                "translation" => {
                    if !trans.is_empty() {
                        lines.push(format!("[{ts}] {trans}"));
                    }
                }
                _ => {
                    lines.push(format!("[{ts}] {orig}"));
                    if !trans.is_empty() {
                        lines.push(format!("  -> {trans}"));
                    }
                    lines.push(String::new());
                }
            }
        }
        let body = lines.join("\n").trim_end().to_string() + "\n";
        if let Err(e) = std::fs::write(&path, body) {
            tracing::error!("{}: {e}", lt_i18n::t("export_failed"));
        } else {
            tracing::info!("导出完成: {}", path.display());
        }
    }

    /// 启动即安排节拍（由 lt-app 在 run 前调用）：
    /// 悬浮窗监视节拍（overlay 行为不变）+ 启动流节拍（向导倒计时/收尾延迟）
    pub fn kick_ticks(&mut self) {
        self.app_state.schedule_monitor_tick(WinId::Overlay);
        self.app_state.kick_setup_tick();
        if self.app_state.ov_click_through {
            self.app_state.schedule_click_through_tick();
        }
    }
}

impl ApplicationHandler<UiMsg> for MultiWindowApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty() {
            if let Err(e) = self.setup(event_loop) {
                tracing::error!("窗口初始化失败: {e}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UiMsg) {
        self.on_msg(event_loop, event);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Some(pos) = self.windows.iter().position(|w| w.window.id() == window_id) else {
            return;
        };
        let id = self.windows[pos].id;
        // 先喂给 egui（累积输入），再处理窗口语义
        let _resp = {
            let hw = &mut self.windows[pos];
            hw.state.on_window_event(&hw.window, &event)
        };
        match event {
            WindowEvent::CloseRequested => {
                // 原版语义：关窗=隐藏（退出只走托盘）
                if id == WinId::Subtitle {
                    self.app_state.settings.subtitle_mode.enabled = false;
                }
                self.set_visible(id, false);
                self.sync_tray_checks();
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    if let (Ok(w), Ok(h)) = (size.width.try_into(), size.height.try_into()) {
                        self.painter.on_window_resized(Self::viewport_of(id), w, h);
                    }
                    self.window(id).request_redraw();
                }
            }
            WindowEvent::Moved(_) if id == WinId::Overlay => {
                // 拖动/移动结束防抖保存（原版 moveEvent → _schedule_pos_save）
                self.app_state.schedule_pos_save(self.overlay_geo());
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // surface 重配置由 Painter 在下一帧处理；请求重绘即可
                self.window(id).request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.run_frame(id);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // 0) 缺模型下载失败后点"关闭"：退出应用（原版 reject → main 返回）
        if self.app_state.quit_requested {
            tracing::info!("退出请求（启动流对话框关闭）");
            event_loop.exit();
            return;
        }
        // 1) 到期节拍按 kind 分派（同窗口可并存多种节拍）
        for tick in self.app_state.drain_due_ticks() {
            match tick.kind {
                TickKind::Monitor => {
                    self.app_state.sample_system();
                    self.app_state.schedule_monitor_tick(WinId::Overlay);
                    self.redraw(WinId::Overlay);
                }
                TickKind::StreamFlush => {
                    self.app_state.flush_streams();
                    self.redraw(WinId::Overlay);
                }
                TickKind::PosSave => self.on_pos_save_tick(),
                TickKind::ClickThrough => {
                    self.poll_click_through();
                    if self.app_state.ov_click_through {
                        self.app_state.schedule_click_through_tick();
                    }
                }
                TickKind::Setup => self.on_setup_tick(),
            }
        }
        // 2) 空闲策略：等待最近节拍或事件
        let deadline = self
            .app_state
            .next_tick()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

/// 窗口清屏色：透明窗口必须清成全透明，否则出现残留底色
fn clear_color_for(id: WinId) -> [f32; 4] {
    match id {
        WinId::Overlay | WinId::Subtitle => [0.0; 4],
        _ => [0.08, 0.08, 0.10, 1.0],
    }
}

/// 安装中文字体（微软雅黑）到 egui 字体族首位。
/// 未找到字体文件时保持默认（英文界面仍可用）。
fn install_cjk_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑（原版默认字体）
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf", // 黑体兜底
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".to_string(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, "cjk".to_string());
                }
            }
            tracing::info!("CJK 字体已加载: {path}");
            ctx.set_fonts(fonts);
            return;
        }
    }
    tracing::warn!("未找到中文字体文件，界面将回退默认字体");
}
