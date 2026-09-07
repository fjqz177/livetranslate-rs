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
use crate::windows::subtitle::{MonoRect, clamp_to_screen, is_pos_visible};
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
    /// 面板首次显示前需定位（主屏偏上居中），定位后置位
    needs_position: bool,
    /// 当前整窗层 alpha（LWA_ALPHA；None=非 layered 普通窗）
    layer_alpha: Option<u8>,
    /// 已应用窗口区域缓存（宽,高,圆角半径物理px；None=未设）
    region_key: Option<(u32, u32, u32)>,
}

pub struct MultiWindowApp {
    pub app_state: AppState,
    pub ctx: Context,
    painter: Painter,
    windows: Vec<HostedWindow>,
    pub tray: Option<Tray>,
    proxy: EventLoopProxy<UiMsg>,
    /// 托盘首次隐藏悬浮窗已弹过气泡（原版 _hide_notified）
    overlay_hide_notified: bool,
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
        // 字体系统（W-3）：内嵌思源默认 + 系统字体扫描；此处按启动 Settings 装配
        crate::fonts::apply_fonts(&ctx, &app_state.settings, &mut app_state.fonts);
        // 主题按窗口注入（run_frame 内：Panel=Windows 原生浅色，其余=深色），
        // 不再全局 set_theme——悬浮窗/字幕窗保持深色（原版面板即原生浅色）。
        let painter = pollster::block_on(Painter::new(
            ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            true, // support_transparent_backbuffer：悬浮窗/字幕窗必需
            egui_wgpu::RendererOptions::default(),
        ));
        let proxy = event_loop.create_proxy();
        // UI → 事件环回出口（后台线程经 AppState.send_event 回流 UiMsg；
        // benchmark 窗的 on_line 日志流即走此通道）
        {
            let proxy = proxy.clone();
            app_state.event_tx = Some(std::sync::Arc::new(move |msg: UiMsg| {
                let _ = proxy.send_event(msg);
            }));
        }
        Ok(Self {
            app_state,
            ctx,
            painter,
            windows: Vec::new(),
            tray: None,
            proxy,
            overlay_hide_notified: false,
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
        // 字幕窗宽度取配置（原版 setFixedWidth(window_width)，高度自适应）
        let sub_w = self.app_state.settings.subtitle_mode.window_width;
        self.create_window(event_loop, WinId::Subtitle, (sub_w, 160))?;
        // 面板尺寸对齐原版实机 ≈535×781（Qt 布局 minimumSizeHint 把
        // resize(520,650) 顶开，2026-09-07 实测 524×775——7 页表单在 781 高
        // 下整页放下、不出滚动条）。小屏按工作区高度钳制（winit 无 work-area
        // API，任务栏按 48 逻辑 px 估），下限 650（minimumSize 480×420 不变）
        let panel_h = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .map(|m| {
                let s = m.scale_factor() as f32;
                (m.size().height as f32 / s - 48.0) as u32
            })
            .map(|avail| 781.min(avail))
            .unwrap_or(781)
            .max(650);
        self.create_window(event_loop, WinId::Panel, (535, panel_h))?;
        self.create_window(event_loop, WinId::Log, (900, 500))?;
        // 启动流对话框（原版 QDialog：常规装饰窗口；可见性 = 启动流进行中）
        self.create_window(event_loop, WinId::Setup, (560, 420))?;
        // 性能基准独立工具窗（原版 BenchmarkDialog resize(680, 480)；默认隐藏）
        self.create_window(event_loop, WinId::Benchmark, (680, 480))?;
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
            .with_resizable(true)
            .with_window_icon(crate::tray::window_icon());
        if matches!(id, WinId::Overlay | WinId::Subtitle) {
            // 不用 winit 的 with_transparent：它开启 NOREDIRECTIONBITMAP + DWM
            // blur-behind，而 wgpu 在 Windows(HWND) 只有 Opaque 合成模式
            // （wgpu-hal dx12 adapter.rs 对 WndHandle 仅报告 Opaque），组合后果 =
            // 清除色 alpha 被忽略、表面恒白底（白角楔 + 整体不透明的根因）。
            // 真半透明 = WS_EX_LAYERED + SetLayeredWindowAttributes（下方
            // apply_layered），等价原版 setWindowOpacity + WA_TranslucentBackground。
            attrs = attrs
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
        if id == WinId::Panel {
            // 原版 setMinimumSize(480, 420)
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(480.0, 420.0));
        }
        if id == WinId::Subtitle {
            // 原版 setFixedWidth：宽度固定（高度自适应）→ 禁用户拖拽缩放
            attrs = attrs.with_resizable(false);
        }
        let visible = *self.app_state.visible.get(&id).unwrap_or(&true);
        attrs = attrs.with_visible(visible);

        let window = Arc::new(event_loop.create_window(attrs)?);
        // 悬浮窗/字幕窗：surface 创建前先挂 layered 层属性（避免呈现抖动）；
        // 初始 alpha 取自当前设置，运行中随设置变更在 run_frame 内刷新
        #[cfg(windows)]
        let layer_alpha = match id {
            WinId::Overlay => Some(overlay_layered_alpha(&self.app_state)),
            WinId::Subtitle => Some(subtitle_layered_alpha(&self.app_state)),
            _ => None,
        };
        #[cfg(not(windows))]
        let layer_alpha = None;
        #[cfg(windows)]
        if let Some(a) = layer_alpha {
            apply_layered(&window, a);
        }
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
                    window.set_outer_position(winit::dpi::PhysicalPosition::new(x as f64, y as f64));
                }
            } else if let Some(mon) = event_loop
                .primary_monitor()
                .or_else(|| event_loop.available_monitors().next())
            {
                // 原版首启默认位：x=avail.right-620-20, y=avail.bottom-500-60（工作区坐标）；
                // winit 无 work-area API，任务栏高度按 48 逻辑 px 估
                let s = mon.scale_factor() as f32;
                let mp = mon.position();
                let ms = mon.size();
                let x = mp.x + (ms.width as f32 - (620.0 + 20.0) * s) as i32;
                let y = mp.y + (ms.height as f32 - (500.0 + 60.0 + 48.0) * s) as i32;
                window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
            }
        }
        // 字幕窗恢复保存位置（原版 _setup_ui：window_x/y 多屏可见才用，否则 (100,100)）
        if id == WinId::Subtitle {
            let sm = &self.app_state.settings.subtitle_mode;
            let monitors =
                Self::monitor_rects(event_loop.available_monitors(), event_loop.primary_monitor());
            let (x, y) = match (sm.window_x, sm.window_y) {
                (Some(x), Some(y)) if is_pos_visible(x, y, &monitors) => (x, y),
                _ => (100, 100),
            };
            window.set_outer_position(winit::dpi::LogicalPosition::new(f64::from(x), f64::from(y)));
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
        self.windows.push(HostedWindow {
            id,
            window,
            state,
            needs_position: id == WinId::Panel,
            layer_alpha,
            region_key: None,
        });
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

        // 按窗口注入 visuals（egui pass 串行执行，帧前 set 对本 pass 生效）：
        // 控制面板 = Windows 原生浅色（原版 PyQt6 默认控件）；其余窗口深色。
        // 三态描边/字重统一（消 hover 文字微移，WP-B）对各主题幂等。
        // 滚动条按窗型定制（WP-C）：面板浅色细条零占位零 reflow，深色窗
        // 白色半透明细条（悬浮窗对齐原版 QSS 6px 白条）。
        let mut visuals = match id {
            WinId::Panel => crate::windows::panel::panel_visuals(),
            _ => egui::Visuals::dark(),
        };
        crate::style::stabilize_widget_strokes(&mut visuals);
        ctx.set_visuals(visuals);
        let scroll = match id {
            WinId::Panel => crate::style::panel_scroll_style(),
            _ => crate::style::dark_scroll_style(),
        };
        ctx.all_styles_mut(move |s| s.spacing.scroll = scroll);

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
        // 整窗半透明（LWA_ALPHA）：样式页改背景不透明度/窗口不透明度后即时刷新
        #[cfg(windows)]
        let target_alpha = match id {
            WinId::Overlay => Some(overlay_layered_alpha(&self.app_state)),
            WinId::Subtitle => Some(subtitle_layered_alpha(&self.app_state)),
            _ => None,
        };
        #[cfg(not(windows))]
        let target_alpha = None;
        if let Some(ta) = target_alpha {
            if self.windows[pos].layer_alpha != Some(ta) {
                #[cfg(windows)]
                apply_layered(&window, ta);
                self.windows[pos].layer_alpha = Some(ta);
            }
        }
        // 圆角窗口区域：窗口尺寸/DPI 缩放/圆角设置变化后重设（缓存比对，常态零开销）；
        // 悬浮窗圆角=样式页 border_radius，字幕窗=字幕页 border_radius（原版各自独立）
        #[cfg(windows)]
        if matches!(id, WinId::Overlay | WinId::Subtitle) {
            let size = window.inner_size();
            let (w, h) = (size.width, size.height);
            let radius = match id {
                WinId::Overlay => self.app_state.settings.style.border_radius,
                _ => self.app_state.settings.subtitle_mode.border_radius,
            };
            let radius_px = (radius as f32 * window.scale_factor() as f32).round() as u32;
            let key = (w, h, radius_px);
            if self.windows[pos].region_key != Some(key) {
                apply_window_region(&window, w, h, radius_px);
                self.windows[pos].region_key = Some(key);
            }
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
    /// 托盘状态同步（新版最小菜单：仅状态行文字；模型/语言切换收敛悬浮窗）
    fn sync_tray_checks(&mut self) {
        self.update_tray_status();
    }

    /// 托盘菜单动作（新版最小菜单：暂停/悬浮窗显隐/面板/退出）
    fn on_menu(&mut self, event_loop: &ActiveEventLoop, id: &str) {
        use tray::ids as m;
        match id {
            m::PAUSE => {
                self.app_state.running = !self.app_state.running;
                if let Some(t) = &self.tray {
                    let text = if self.app_state.running {
                        lt_i18n::t("tray_pause")
                    } else {
                        lt_i18n::t("tray_resume")
                    };
                    let _ = t.handles.pause.set_text(text);
                    let status = if self.app_state.running {
                        tray::IconStatus::Run
                    } else {
                        tray::IconStatus::Pause
                    };
                    t.set_status(status);
                }
                self.update_tray_status();
                tracing::info!("管道 {}", if self.app_state.running { "运行" } else { "暂停" });
            }
            m::OVERLAY_TOGGLE => {
                let vis = !self.window(WinId::Overlay).is_visible().unwrap_or(false);
                self.set_overlay_visible_with_hint(vis);
            }
            m::SHOW_PANEL => {
                let vis = !self.window(WinId::Panel).is_visible().unwrap_or(false);
                self.set_visible(WinId::Panel, vis);
            }
            m::QUIT => {
                // 原版 on_quit(confirm=True)：确认框；取消则不退出
                let confirmed = rfd::MessageDialog::new()
                    .set_title(lt_i18n::t("quit_confirm_title"))
                    .set_description(lt_i18n::t("quit_confirm_msg"))
                    .set_buttons(rfd::MessageButtons::OkCancel)
                    .set_level(rfd::MessageLevel::Info)
                    .show();
                if confirmed == rfd::MessageDialogResult::Ok {
                    tracing::info!("收到退出指令（已确认）");
                    event_loop.exit();
                }
            }
            other => {
                tracing::debug!("未知托盘菜单项: {other}");
            }
        }
        self.apply_overlay_flags();
    }

    /// 刷新托盘状态行（● 状态 · 引擎 · 模型 · 源 → 目标；原版 status_action）
    fn update_tray_status(&mut self) {
        let s = &self.app_state;
        let state = if s.running { "运行" } else { "暂停" };
        let engine = s.asr_label.clone().unwrap_or_else(|| "--".into());
        let model = s
            .settings
            .models
            .get(s.settings.active_model)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "--".into());
        let text = lt_i18n::t("tray_status_format")
            .replace("{state}", state)
            .replace("{engine}", &engine)
            .replace("{model}", &model)
            .replace("{src}", &s.settings.asr_language)
            .replace("{tgt}", &s.settings.target_language);
        if let Some(t) = &self.tray {
            let _ = t.handles.status.set_text(text);
        }
    }

    fn set_visible(&mut self, id: WinId, vis: bool) {
        if let Some(hw) = self.find_mut(id) {
            if vis {
                // 面板首次显示时主屏偏上居中（Windows 对话框惯例；避免固定
                // 左上角与用户日常悬浮窗位置重叠被遮挡）。窗口显示后定位，
                // 规避创建期（隐藏态）set_outer_position 被 Windows 初始化
                // 级联位置覆盖的问题。
                if hw.needs_position {
                    hw.needs_position = false;
                    if let Some(mon) = hw
                        .window
                        .primary_monitor()
                        .or_else(|| hw.window.current_monitor())
                    {
                        let (mp, ms) = (mon.position(), mon.size());
                        let inner = hw.window.inner_size();
                        let x = mp.x + ((ms.width as i32 - inner.width as i32) / 2).max(0);
                        let y = mp.y + ((ms.height as i32 - inner.height as i32) / 4).max(0);
                        hw.window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                    }
                }
                hw.window.set_visible(true);
                hw.window.focus_window();
                hw.window.request_redraw();
            } else {
                hw.window.set_visible(false);
            }
        }
        self.app_state.visible.insert(id, vis);
    }

    /// 悬浮窗显隐统一入口（托盘 OVERLAY_TOGGLE 与主面板"隐藏"按钮共用；
    /// 原版 on_toggle_overlay：托盘菜单文字翻转 + 首次隐藏气泡提示）
    fn set_overlay_visible_with_hint(&mut self, vis: bool) {
        if let Some(t) = &self.tray {
            let text = if vis {
                lt_i18n::t("tray_hide_overlay")
            } else {
                lt_i18n::t("tray_show_overlay")
            };
            let _ = t.handles.overlay_toggle.set_text(text);
            // 首次隐藏提示（原版 tray.showMessage 气泡；tray-icon 0.24
            // 无气泡 API → 降级为日志，已知偏差，待 Shell_NotifyIcon 直调）
            if !vis && !self.overlay_hide_notified {
                self.overlay_hide_notified = true;
                tracing::info!("{}", lt_i18n::t("hide_tray_hint"));
            }
        }
        self.set_visible(WinId::Overlay, vis);
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

    /// 面板设置防抖到期（原版 _do_auto_save → _apply_settings）：整体重放 ApplySettings
    fn on_panel_apply_tick(&mut self) {
        let now = Instant::now();
        if let Some(snapshot) = self.app_state.take_due_panel_apply(now) {
            tracing::info!("面板设置应用（300ms 防抖到期）");
            self.app_state.send_cmd(lt_proto::Cmd::ApplySettings(Box::new(snapshot)));
        }
    }

    /// UiMsg 统一入口（管道事件/托盘事件）
    fn on_msg(&mut self, event_loop: &ActiveEventLoop, msg: UiMsg) {
        match msg {
            UiMsg::Menu(id) => self.on_menu(event_loop, &id),
            UiMsg::Tray(_t) => {}
            // 管道域命令由 lt-app::AppShell.user_event 处理（本层无 Pipeline）
            UiMsg::Cmd(_) => {}
            // catch-all 为未来新增事件防黑洞（当前枚举已全覆盖则不可达）
            #[allow(unreachable_patterns)]
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
                    self.app_state.update_translation(id, text.clone(), tl_ms);
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                    // 字幕窗文本喂入（原版 pipeline 仅 _subwin.isVisible() 时 update_text）：
                    // 译文完成 → {目标语言: 译文}；同语言空串 → {目标语言: 原文}
                    // （原版 pipeline.py:459/629 同语言分支与 868/955 译文完成分支）
                    if *self.app_state.visible.get(&WinId::Subtitle).unwrap_or(&false) {
                        let original = self
                            .app_state
                            .messages
                            .iter()
                            .rev()
                            .find(|m| m.id == id)
                            .map(|m| m.original.clone());
                        if let Some(original) = original {
                            let value = if text.is_empty() { original.clone() } else { text };
                            let mut tl = std::collections::BTreeMap::new();
                            tl.insert(self.app_state.settings.target_language.clone(), value);
                            self.app_state.subtitle_update_text(original, tl);
                        }
                        self.redraw(WinId::Subtitle);
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
                    if let Some(t) = &self.tray {
                        let status = if self.app_state.running {
                            tray::IconStatus::Run
                        } else {
                            tray::IconStatus::Pause
                        };
                        t.set_status(status);
                    }
                    if let Some(hw) = self.find_mut(WinId::Overlay) {
                        hw.window.request_redraw();
                    }
                    self.close_load_dialog();
                }
                // ASR 完全不可用（沿用原版字面文案）；同样关闭加载框 + 托盘错误图标
                lt_proto::UiEvent::AsrUnavailable => {
                    self.app_state.asr_label = Some("ASR unavailable".into());
                    if let Some(t) = &self.tray {
                        t.set_status(tray::IconStatus::Error);
                    }
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
                // 日志行（常驻桥接线程全程转发 → 日志窗；级别过滤在窗口状态内）。
                // target=="benchmark" 的行为基准输出流：同步追加到 bench_lines
                // （独立窗渲染源）并重绘基准窗；__DONE__ 复位运行态 + 完成提示。
                lt_proto::UiEvent::LogLine { level, target, msg } => {
                    if target == "benchmark" {
                        let done = msg == "__DONE__";
                        self.app_state.push_bench_line(msg.clone());
                        self.redraw(WinId::Benchmark);
                        if done && self.app_state.bench_running {
                            self.app_state.bench_running = false;
                            tracing::info!("性能基准完成");
                            rfd::MessageDialog::new()
                                .set_title(lt_i18n::t("bench_done_title"))
                                .set_description(lt_i18n::t("bench_done_msg").as_str())
                                .set_buttons(rfd::MessageButtons::Ok)
                                .set_level(rfd::MessageLevel::Info)
                                .show();
                        }
                    }
                    if self.app_state.logwin.push(crate::state::LogLineEntry {
                        time: chrono::Local::now().format("%H:%M:%S").to_string(),
                        level,
                        target,
                        msg,
                    }) {
                        self.redraw(WinId::Log);
                    }
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
                    // 原版 hide_clicked → on_toggle_overlay(False)：托盘文字翻转 + 首次气泡
                    self.set_overlay_visible_with_hint(false);
                }
                WinAction::ShowPanel => {
                    self.set_visible(WinId::Panel, true);
                }
                WinAction::ShowBenchmark => {
                    // 原版 BenchmarkDialog.exec()：显示独立工具窗
                    self.set_visible(WinId::Benchmark, true);
                }
                WinAction::ToggleSubtitle => {
                    let vis = self.app_state.settings.subtitle_mode.enabled;
                    self.set_visible(WinId::Subtitle, vis);
                    // 开启即恢复 500ms 穿透断言轮询（原版 showEvent 重断言 + _ct_timer）
                    if vis && self.app_state.settings.subtitle_mode.click_through {
                        self.app_state.schedule_subtitle_click_through_tick();
                    }
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
                WinAction::DragSubtitle => {
                    // 原版 mousePressEvent MiddleButton → move；winit 走系统移动循环
                    let _ = window.drag_window();
                }
                WinAction::SetSubtitleHeight(h) => {
                    // 原版 _fit_height_animated：高度变化同时上移 y/2 保持视觉中心
                    let scale = window.scale_factor();
                    let cur = window.inner_size().to_logical::<f32>(scale);
                    let dy = ((h - cur.height) / 2.0).round();
                    if dy.abs() >= 1.0 {
                        let pos = window.outer_position().unwrap_or_default();
                        window.set_outer_position(winit::dpi::LogicalPosition::new(
                            pos.x as f64 / scale,
                            (pos.y as f64 - dy as f64) / scale,
                        ));
                    }
                    let _ = window.request_inner_size(winit::dpi::LogicalSize::new(
                        f64::from(cur.width),
                        f64::from(h.max(20.0)),
                    ));
                }
                WinAction::ClampSubtitlePos => {
                    // 原版 on_finished → _clamp_to_screen + position_changed（500ms 防抖保存）
                    let scale = window.scale_factor();
                    let pos = window.outer_position().unwrap_or_default();
                    let size = window.inner_size().to_logical::<f32>(scale);
                    let cur = ((pos.x as f32 / scale as f32) as i32, (pos.y as f32 / scale as f32) as i32);
                    // current_monitor 优先作兜底屏（对齐原版"未命中 → 主屏"的就近语义）
                    let monitors = Self::monitor_rects(window.available_monitors(), window.current_monitor());
                    let (x, y) = clamp_to_screen(cur.0, cur.1, size.width as i32, size.height as i32, &monitors);
                    if (x, y) != cur {
                        window.set_outer_position(winit::dpi::LogicalPosition::new(f64::from(x), f64::from(y)));
                    }
                    self.app_state.schedule_subtitle_pos_save((x, y));
                }
                WinAction::ResetPositions => {
                    // 原版 app_shell._on_reset_positions：字幕窗回 (100,100)；
                    // 悬浮窗回主屏右下 (right-ow-50, bottom-oh-100)。窗口移动后由
                    // 既有 Moved → 防抖保存路径写回 settings（此处同步直写一次兜底）。
                    if let Some(sub) = self.find_mut(WinId::Subtitle) {
                        sub.window.set_outer_position(winit::dpi::LogicalPosition::new(100.0, 100.0));
                    }
                    if let Some(ov) = self.find(WinId::Overlay) {
                        let scale = ov.window.scale_factor();
                        let size = ov.window.inner_size().to_logical::<f32>(scale);
                        if let Some(m) = ov.window.primary_monitor() {
                            let (mp, ms, msf) = (m.position(), m.size(), m.scale_factor());
                            let right = ((mp.x as f64 + ms.width as f64) / msf) as i32;
                            let bottom = ((mp.y as f64 + ms.height as f64) / msf) as i32;
                            let x = right - size.width as i32 - 50;
                            let y = bottom - size.height as i32 - 100;
                            ov.window
                                .set_outer_position(winit::dpi::LogicalPosition::new(f64::from(x), f64::from(y)));
                        }
                    }
                    // settings 同步（原版 _save_subwin_state/_save_overlay_pos 即时保存）
                    let geo = self.overlay_geo();
                    let s = &mut self.app_state.settings;
                    s.subtitle_mode.window_x = Some(100);
                    s.subtitle_mode.window_y = Some(100);
                    s.overlay_x = Some(geo.0);
                    s.overlay_y = Some(geo.1);
                    self.app_state.subtitle.last_saved_pos = Some((100, 100));
                    self.app_state.send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
                        self.app_state.settings.clone(),
                    )));
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

    /// 当前字幕窗逻辑位置 (x, y)（防抖登记用）
    fn subtitle_pos(&self) -> (i32, i32) {
        let Some(hw) = self.find(WinId::Subtitle) else {
            return (0, 0);
        };
        let scale = hw.window.scale_factor() as f32;
        let pos = hw.window.outer_position().unwrap_or_default();
        ((pos.x as f32 / scale) as i32, (pos.y as f32 / scale) as i32)
    }

    /// 显示器矩形表（逻辑 px，主屏/当前屏排首作钳制兜底；winit 无工作区概念 →
    /// 全显示器尺寸，原版 availableGeometry 剔除任务栏为已知偏差）
    fn monitor_rects(
        monitors: impl Iterator<Item = winit::monitor::MonitorHandle>,
        preferred: Option<winit::monitor::MonitorHandle>,
    ) -> Vec<MonoRect> {
        let mut ms: Vec<(winit::monitor::MonitorHandle, MonoRect)> = monitors
            .map(|m| {
                let scale = m.scale_factor() as f32;
                let (pos, size) = (m.position(), m.size());
                let rect = MonoRect {
                    x: (pos.x as f32 / scale) as i32,
                    y: (pos.y as f32 / scale) as i32,
                    w: (size.width as f32 / scale) as i32,
                    h: (size.height as f32 / scale) as i32,
                };
                (m, rect)
            })
            .collect();
        ms.sort_by_key(|(m, _)| if preferred.as_ref() == Some(m) { 0 } else { 1 });
        ms.into_iter().map(|(_, r)| r).collect()
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

    /// 字幕窗位置防抖到期：读位置（逻辑 px）写 window_x/y 并持久化（原版 position_changed）
    fn on_subtitle_pos_save_tick(&mut self) {
        let Some(hw) = self.find(WinId::Subtitle) else { return };
        let window = hw.window.clone();
        let scale = window.scale_factor() as f32;
        let Ok(pos) = window.outer_position() else { return };
        let p = ((pos.x as f32 / scale) as i32, (pos.y as f32 / scale) as i32);
        self.app_state.subtitle.pos_dirty_since = None;
        if self.app_state.subtitle.last_saved_pos == Some(p) {
            return;
        }
        self.app_state.subtitle.last_saved_pos = Some(p);
        self.app_state.settings.subtitle_mode.window_x = Some(p.0);
        self.app_state.settings.subtitle_mode.window_y = Some(p.1);
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
            Self::set_window_transparent(&window, false);
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
        Self::set_window_transparent(&window, !in_header);
    }

    /// 字幕窗穿透轮询（原版 _ct_timer 500ms → _apply_click_through：
    /// 全窗穿透 `winutil.set_click_through(self, "all")`，无头部例外）。
    /// Qt 需定时重断言（show/raise 会清扩展样式），此处按位去重读写等效。
    /// 仅 Windows。
    #[cfg(windows)]
    fn poll_subtitle_click_through(&mut self) {
        let Some(hw) = self.find(WinId::Subtitle) else { return };
        let window = hw.window.clone();
        let enabled = self.app_state.settings.subtitle_mode.click_through
            && *self.app_state.visible.get(&WinId::Subtitle).unwrap_or(&false);
        Self::set_window_transparent(&window, enabled);
    }

    /// 非 Windows 兜底（无穿透能力）
    #[cfg(not(windows))]
    fn poll_click_through(&mut self) {}

    /// 非 Windows 兜底（无穿透能力）
    #[cfg(not(windows))]
    fn poll_subtitle_click_through(&mut self) {}

    /// WS_EX_TRANSPARENT 位切换（悬浮窗/字幕窗共用的通用窗口层函数）。
    /// 窗口已挂 WS_EX_LAYERED（apply_layered），这里只增删 TRANSPARENT 位，
    /// 两者可共存（原版 QWidget.setWindowOpacity + click-through 亦同）。
    #[cfg(windows)]
    fn set_window_transparent(window: &Window, enable: bool) {
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
    /// + 字幕窗穿透轮询（原版 set_click_through 即启动 500ms 计时）
    pub fn kick_ticks(&mut self) {
        self.app_state.schedule_monitor_tick(WinId::Overlay);
        self.app_state.kick_setup_tick();
        if self.app_state.ov_click_through {
            self.app_state.schedule_click_through_tick();
        }
        if self.app_state.settings.subtitle_mode.click_through
            && *self.app_state.visible.get(&WinId::Subtitle).unwrap_or(&false)
        {
            self.app_state.schedule_subtitle_click_through_tick();
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
        let resp = {
            let hw = &mut self.windows[pos];
            hw.state.on_window_event(&hw.window, &event)
        };
        // 关键：输入事件携带 repaint 标志——忽略它会导致"点击缓冲到下一帧
        // 才被消费"，而该窗口无周期节拍时下一帧永不到来（表现为控件点击
        // 全部无响应）。注意 egui_winit 对 RedrawRequested 本身也返回
        // repaint=true，绝不能回环请求（否则 1350fps 自旋饿死其他窗口）。
        if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) {
            self.window(id).request_redraw();
        }
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
            WindowEvent::Moved(_) => {
                match id {
                    WinId::Overlay => {
                        // 拖动/移动结束防抖保存（原版 moveEvent → _schedule_pos_save）
                        self.app_state.schedule_pos_save(self.overlay_geo());
                    }
                    WinId::Subtitle => {
                        // 字幕窗移动防抖保存（原版 mouseReleaseEvent → position_changed）
                        self.app_state.schedule_subtitle_pos_save(self.subtitle_pos());
                    }
                    _ => {}
                }
                // 外部移动（DPI 上下文变化/挂起恢复）后 DXGI 表面可能失配白屏：
                // 以当前尺寸强制重配置（幂等；尺寸未变时为廉价空转）
                let sz = self.window(id).inner_size();
                if sz.width > 0 && sz.height > 0 {
                    if let (Ok(w), Ok(h)) = (sz.width.try_into(), sz.height.try_into()) {
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
                TickKind::PosSave => match tick.win {
                    WinId::Subtitle => self.on_subtitle_pos_save_tick(),
                    _ => self.on_pos_save_tick(),
                },
                TickKind::ClickThrough => match tick.win {
                    WinId::Subtitle => {
                        // 字幕窗 500ms 全窗穿透断言（原版 _ct_timer 周期重申）
                        self.poll_subtitle_click_through();
                        // 开关开启期间由宿主续拍（关闭/隐藏即断链）
                        if self.app_state.settings.subtitle_mode.click_through
                            && *self.app_state.visible.get(&WinId::Subtitle).unwrap_or(&false)
                        {
                            self.app_state.schedule_subtitle_click_through_tick();
                        }
                    }
                    _ => {
                        self.poll_click_through();
                        if self.app_state.ov_click_through {
                            self.app_state.schedule_click_through_tick();
                        }
                    }
                },
                // 字幕窗自动隐藏到点（原版 _auto_hide_timer.timeout → _on_auto_hide_timeout）
                TickKind::SubtitleAutoHide => {
                    let anim = self.app_state.settings.subtitle_mode.auto_hide_animation.clone();
                    let dur = self.app_state.settings.subtitle_mode.auto_hide_duration;
                    if self.app_state.subtitle.on_auto_hide_timeout(&anim, dur, Instant::now()) {
                        self.redraw(WinId::Subtitle);
                    }
                }
                // 字幕窗排队句子到点（原版 _pending_segment_timers 的 singleShot 到期）
                TickKind::SubtitlePending => {
                    if self.app_state.subtitle_flush_pending() {
                        self.redraw(WinId::Subtitle);
                    }
                }
                TickKind::Setup => self.on_setup_tick(),
                // 面板设置 300ms 防抖到期（原版 _save_timer.timeout → _apply_settings：
                // 整体重放 ApplySettings，AppShell 负责热应用 + 落盘）
                TickKind::PanelApply => self.on_panel_apply_tick(),
                // 翻译页 prompt 600ms 防抖到期（原版 _prompt_debounce → _apply_prompt：
                // system_prompt 已实时写入 settings，此处重建活动模型翻译器）
                TickKind::PromptApply => {
                    if self.app_state.take_due_prompt_apply(Instant::now()) {
                        let s = &self.app_state;
                        if let Some(cfg) =
                            s.settings.models.get(s.settings.active_model).cloned()
                        {
                            tracing::info!("System prompt updated（600ms 防抖到期，重建翻译器）");
                            self.app_state
                                .send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
                        }
                    }
                }
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

/// 窗口清屏色。
/// 悬浮窗/字幕窗：不透明黑——圆角外像素已被 SetWindowRgn 从窗口形状切除
/// （见 apply_window_region），残余清除像素只在圆角 1px 抗锯齿带内与背景色
/// 混合（同为深色，无可察差异）。整窗半透明由 LWA_ALPHA 承担。
/// 其余普通窗：深灰不透明。
fn clear_color_for(id: WinId) -> [f32; 4] {
    match id {
        WinId::Overlay | WinId::Subtitle => [0.0, 0.0, 0.0, 1.0],
        _ => [0.08, 0.08, 0.10, 1.0],
    }
}

/// 悬浮窗整窗不透明度（0-255）。原版是两级叠加：QSS rgba(bg_opacity/255) ×
/// setWindowOpacity(window_opacity%)；LWA_ALPHA 只有一层，取乘积为单值——
/// 背景精确等价，文本/控件随之乘同系数（原版文本仅乘 window_opacity，
/// 默认 95% 下相差 ≤5.6pp，实机不可辨；transparent 预设文本偏淡为已知偏差）。
#[cfg(windows)]
fn overlay_layered_alpha(s: &AppState) -> u8 {
    let st = &s.settings.style;
    ((st.window_opacity.min(100) * st.bg_opacity.min(255)) / 100) as u8
}

/// 字幕窗整窗不透明度：原版无 setWindowOpacity，仅背景 QSS alpha。
/// bg_opacity=0 全透明模式退化为不透明底+文字（键控仍镂空圆角空区，已知偏差）。
#[cfg(windows)]
fn subtitle_layered_alpha(s: &AppState) -> u8 {
    let sm = &s.settings.subtitle_mode;
    if sm.bg_opacity == 0 { 255 } else { sm.bg_opacity.min(255) as u8 }
}

/// WS_EX_LAYERED + SetLayeredWindowAttributes：整窗半透明（等价 Qt 整窗不透明度，
/// 即原版 setWindowOpacity）。圆角镂空不走 LWA_COLORKEY——DWM 对 alpha 合成有
/// ±1 抖动/舍入，精确色键不可靠（实测键控色需 3 通道同时精确命中），改由
/// SetWindowRgn 几何镂空（见 apply_window_region）。surface 创建前调用（呈现稳定），
/// 设置变更时随帧刷新。
#[cfg(windows)]
fn apply_layered(window: &Window, alpha: u8) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use ::windows::Win32::Foundation::{HWND};
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else { return };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        if style & WS_EX_LAYERED.0 == 0 {
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED.0) as isize);
        }
        let _ = SetLayeredWindowAttributes(hwnd, Default::default(), alpha, LWA_ALPHA);
    }
}

/// 圆角窗口区域镂空：圆角外用 CreateRoundRectRgn 从窗口形状上切除（几何级
/// 真透明，任何背景都透出桌面）。半径取逻辑 border_radius × DPI 缩放；窗口
/// 尺寸/缩放/圆角设置变化后需重设（run_frame 内按缓存比对刷新）。
#[cfg(windows)]
fn apply_window_region(window: &Window, w: u32, h: u32, radius_px: u32) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else { return };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    let diam = (radius_px * 2).max(2);
    let rgn = unsafe { CreateRoundRectRgn(0, 0, (w + 1) as i32, (h + 1) as i32, diam as i32, diam as i32) };
    if !rgn.is_invalid() {
        // SetWindowRgn 成功后系统接管区域句柄（不得再 DeleteObject）
        let _ = unsafe { SetWindowRgn(hwnd, Some(rgn), true) };
    }
}

