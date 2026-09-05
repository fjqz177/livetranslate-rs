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

use crate::state::{AppState, OverlayMessage, WinId};
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
use winit::window::{Window, WindowLevel};

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
    /// Worker 命令出口（管道就绪后由 lt-app 注入；M0 占位）
    pub cmd_tx: Option<std::sync::mpsc::Sender<lt_proto::Cmd>>,
}

impl MultiWindowApp {
    /// 创建宿主（需在主线程）。异步的 wgpu 初始化用 pollster 阻塞完成。
    pub fn new(
        app_state: AppState,
        event_loop: &EventLoop<UiMsg>,
        cmd_tx: Option<std::sync::mpsc::Sender<lt_proto::Cmd>>,
    ) -> anyhow::Result<Self> {
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
            cmd_tx,
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
        let visible = *self.app_state.visible.get(&id).unwrap_or(&true);
        attrs = attrs.with_visible(visible);

        let window = Arc::new(event_loop.create_window(attrs)?);
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

    /// UiMsg 统一入口（管道事件/托盘事件）
    fn on_msg(&mut self, event_loop: &ActiveEventLoop, msg: UiMsg) {
        match msg {
            UiMsg::Menu(id) => self.on_menu(event_loop, &id),
            UiMsg::Tray(_t) => {}
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
                // 新识别消息 → 追加到悬浮窗消息链并重绘（译文更新 M3 接线）
                lt_proto::UiEvent::AddMessage { id, timestamp, original, lang, asr_ms } => {
                    self.app_state.push_message(OverlayMessage {
                        id,
                        timestamp,
                        original,
                        lang,
                        asr_ms,
                        translation: None, // M3 翻译链路占位
                        tl_ms: 0.0,
                    });
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // ASR 设备标签（悬浮窗 MonitorBar device 段）
                lt_proto::UiEvent::AsrDevice(label) => {
                    self.app_state.asr_label = Some(label);
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // ASR 完全不可用（沿用原版字面文案）
                lt_proto::UiEvent::AsrUnavailable => {
                    self.app_state.asr_label = Some("ASR unavailable".into());
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                }
                // M1：其余管道事件尚未接入（M2 起逐个接线）；先落日志防黑洞
                other => tracing::debug!("UI 事件（待接线）: {other:?}"),
            },
        }
    }

    /// 启动即安排悬浮窗监视节拍（由 lt-app 在 run 前调用）
    pub fn kick_ticks(&mut self) {
        self.app_state.schedule_monitor_tick(WinId::Overlay);
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
        // 1) 到期节拍 → 重绘对应窗口并补下一拍
        for win in self.app_state.drain_due_ticks() {
            // 悬浮窗监视节拍：先做 1s 节流的系统采样（CPU/RAM）
            if win == WinId::Overlay {
                self.app_state.sample_system();
            }
            if let Some(hw) = self.find(win) {
                hw.window.request_redraw();
            }
            // M0：悬浮窗每秒一拍（监视节拍占位）
            if win == WinId::Overlay {
                self.app_state.schedule_monitor_tick(WinId::Overlay);
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
