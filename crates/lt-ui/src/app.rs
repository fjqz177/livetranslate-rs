//! 多窗口宿主（M0.4）：一个 winit 事件循环 + 共享 egui Context + egui-wgpu Painter
//! （Painter 原生支持多 surface，按 ViewportId 管理）。
//!
//! 设计要点（对照 docs/archive/rewrite-plan.md §2.1/2.2）：
//! - 4 个常驻原生窗口：overlay（透明/无边框/置顶/跳任务栏/不抢焦点）、
//!   subtitle（透明/无边框/置顶）、panel/log（常规装饰窗口）；
//! - 每窗口独立 `egui_winit::State`（独立 viewport id/DPI/输入）；
//! - 空闲低 CPU：`ControlFlow::WaitUntil` + AppState 节拍表 + egui 即时重绘请求；
//! - CloseRequested 一律隐藏窗口（退出仅走托盘 Quit，等价 setQuitOnLastWindowClosed(false)）；
//! - 点击穿透由窗口层按 50ms 轮询处理（M4 接入），本宿主只负责窗口创建与 flags。

use crate::state::{
    initial_visibility, push_log_line, AppUi, ConfirmKind, DownloadUiState, OverlayMessage,
    PanelPage, StartupFlow, SubtitleFeed, TickKind, WinAction, WinId,
};
use crate::tray::{self, Tray};
use crate::windows;
use crate::windows::subtitle::{
    clamp_to_screen, is_pos_visible, rects_intersect, resolve_overlap, transparent_desired,
    zone_for_cursor, MonoRect, SubtitleZone,
};
use arc_swap::ArcSwap;
use egui::{Context, ViewportId};
use egui_wgpu::winit::Painter;
use lt_proto::{MonitorSample, Settings, UiEvent, UiMsg};
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
    pub app_state: AppUi,
    pub ctx: Context,
    painter: Painter,
    windows: Vec<HostedWindow>,
    pub tray: Option<Tray>,
    proxy: EventLoopProxy<UiMsg>,
    /// 托盘首次隐藏悬浮窗已弹过气泡（原版 _hide_notified）
    overlay_hide_notified: bool,
    /// 音频监视快照格（W2/D-67）：capture 写 ArcSwap，本层 ~33ms 节拍读格
    /// 重绘——替代 UpdateMonitor 逐事件唤醒；Option 为启动前/无管道占位
    monitor_cell: Option<Arc<ArcSwap<MonitorSample>>>,
    /// 已读快照序号（免每拍重绘：格 seq 未变即跳过）
    monitor_seq: u64,
    /// 各窗口可见性真值表（W5b/R20：单一真源——`set_visible` 唯一变更路径，
    /// 窗口帧经 `session.visible` 只读快照访问；winit `is_visible()` 不再被查询）
    visible: std::collections::HashMap<WinId, bool>,
    /// sysinfo 实例与上次采样时刻（1s 节流；宿主侧采样态，W5 自 AppState 迁出）
    sys: Option<sysinfo::System>,
    sys_last: Option<Instant>,
}

impl MultiWindowApp {
    /// 创建宿主（需在主线程）。异步的 wgpu 初始化用 pollster 阻塞完成。
    /// cmd_tx 存入 AppState（widget 代码经 send_cmd 直接发送命令）。
    pub fn new(
        mut app_state: AppUi,
        event_loop: &EventLoop<UiMsg>,
        cmd_tx: Option<std::sync::mpsc::Sender<lt_proto::Cmd>>,
        monitor_cell: Option<Arc<ArcSwap<MonitorSample>>>,
    ) -> anyhow::Result<Self> {
        app_state.session.cmd_tx = cmd_tx;
        let ctx = Context::default();
        // 字体系统（W-3）：内嵌思源默认 + 系统字体扫描；此处按启动 Settings 装配
        crate::fonts::apply_fonts(&ctx, &app_state.settings, &mut app_state.ctx.fonts);
        // 主题按窗口注入（run_frame 内：Panel=Windows 原生浅色，其余=深色），
        // 不再全局 set_theme——悬浮窗/字幕窗保持深色（原版面板即原生浅色）。
        let painter = pollster::block_on(Painter::new(
            ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            true, // support_transparent_backbuffer：悬浮窗/字幕窗必需
            egui_wgpu::RendererOptions::default(),
        ));
        let proxy = event_loop.create_proxy();
        // W5b：可见性真值表初始化（启动流进行中主窗口隐藏；见 initial_visibility）
        let visible = initial_visibility(&app_state.settings, &app_state.startup.flow);
        Ok(Self {
            app_state,
            ctx,
            painter,
            windows: Vec::new(),
            tray: None,
            proxy,
            overlay_hide_notified: false,
            monitor_cell,
            monitor_seq: 0,
            visible,
            sys: None,
            sys_last: None,
        })
    }

    /// 可见性查询（W5b：唯一权威读点——托盘/穿透轮询/避让全走本表；
    /// winit `window.is_visible()` 不再作为事实源）
    pub fn is_visible(&self, id: WinId) -> bool {
        self.visible.get(&id).copied().unwrap_or(false)
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
        // 确认窗（D-87：六确认唯一载体；常驻隐藏，确认请求时定位显示。
        // 高 160 = 标题行+两行正文+按钮行紧凑排布 + 三行文案余量——永别旧 420 空心）
        self.create_window(event_loop, WinId::Confirm, (380, 160))?;
        // Setup 标题随启动流阶段动态化（向导/缺模型下载；加载框在 ModelLoadStart 再设）
        let setup_title = match &self.app_state.startup.flow {
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
        // W2/D-67：悬浮窗可见即起音频监视节拍（快照格轮询起点；此后
        // 每条 33ms 拍读格、不可见即停）
        if self.visible.get(&WinId::Overlay).copied().unwrap_or(false) {
            self.schedule_audio_monitor_tick();
        }
        // D-36：窗口创建后的 Z 序定型（创建序 = 悬浮窗先 → 字幕窗后，后建偏上——
        // 此处把字幕窗压回悬浮窗之下；运行期显示路径由 set_visible 再维护）
        #[cfg(windows)]
        {
            self.restack_subtitle_below_overlay();
            self.raise_overlay();
        }
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
        if id == WinId::Confirm {
            // D-87 确认窗：无边框自绘 + 置顶 + 跳任务栏；需键盘焦点（不做
            // with_active(false)）。半透明走 LAYERED+LWA_ALPHA（同悬浮窗路径，
            // 不用 with_transparent——wgpu HWND 仅 Opaque，见 Overlay 分支注）
            attrs = attrs
                .with_decorations(false)
                .with_resizable(false)
                .with_window_level(WindowLevel::AlwaysOnTop);
            #[cfg(windows)]
            {
                attrs = attrs.with_skip_taskbar(true);
            }
        }
        if id == WinId::Panel {
            // 原版 setMinimumSize(480, 420)
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(480.0, 420.0));
        }
        if id == WinId::Subtitle {
            // 原版 setFixedWidth：宽度固定（高度自适应）→ 禁用户拖拽缩放
            attrs = attrs.with_resizable(false);
        }
        let visible = *self.visible.get(&id).unwrap_or(&true);
        attrs = attrs.with_visible(visible);

        let window = Arc::new(event_loop.create_window(attrs)?);
        // 悬浮窗/字幕窗：surface 创建前先挂 layered 层属性（避免呈现抖动）；
        // 初始 alpha 取自当前设置，运行中随设置变更在 run_frame 内刷新
        #[cfg(windows)]
        let layer_alpha = match id {
            WinId::Overlay => Some(overlay_layered_alpha(&self.app_state.settings)),
            WinId::Subtitle => Some(subtitle_layered_alpha(&self.app_state.settings)),
            WinId::Confirm => Some(CONFIRM_LAYER_ALPHA),
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
                    x + w as i32 > mp.x + 80
                        && x < mx2 - 80
                        && y + h as i32 > mp.y + 80
                        && y < my2 - 80
                });
                if visible_on_monitor {
                    let _ =
                        window.request_inner_size(winit::dpi::LogicalSize::new(w as f64, h as f64));
                    window
                        .set_outer_position(winit::dpi::PhysicalPosition::new(x as f64, y as f64));
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
            let monitors = Self::monitor_rects(
                event_loop.available_monitors(),
                event_loop.primary_monitor(),
            );
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
        // 三态描边/字重/圆角全等（消 hover 微移 WP-B + 消按压微移 D-32，
        // 基准下限 1.0）对各主题幂等。
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
        // W5b：窗口帧只读快照（真值表宿主独享；窗口代码读 session.visible）
        app_state.session.visible.clone_from(&self.visible);
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
            .get(&viewport)
            .map(|vo| vo.repaint_delay == Duration::ZERO)
            .unwrap_or(false);
        if want_repaint {
            window.request_redraw();
        }
        // 整窗半透明（LWA_ALPHA）：样式页改背景不透明度/窗口不透明度后即时刷新。
        // D-34 修复：winit `apply_diff`（window_state.rs:395-410）在 VISIBLE/层
        // flags 变化时用自身 flags 表**整体重写 GWL_EXSTYLE**——我们手工挂的
        // WS_EX_LAYERED 不在其状态里，隐藏→托盘重显示后被清（悬浮窗"纯黑"，
        // win_layer_spike 实锤：hide 后 LAYERED=true→false）。缓存比对盲于外部
        // 清位 → 每帧实测 EXSTYLE，缺位即重挂（幂等自愈，覆盖一切清位路径）。
        #[cfg(windows)]
        let target_alpha = match id {
            WinId::Overlay => Some(overlay_layered_alpha(&self.app_state.settings)),
            WinId::Subtitle => Some(subtitle_layered_alpha(&self.app_state.settings)),
            WinId::Confirm => Some(CONFIRM_LAYER_ALPHA),
            _ => None,
        };
        #[cfg(not(windows))]
        let target_alpha = None;
        if let Some(ta) = target_alpha {
            if self.windows[pos].layer_alpha != Some(ta) || !window_has_layered(&window) {
                #[cfg(windows)]
                apply_layered(&window, ta);
                self.windows[pos].layer_alpha = Some(ta);
            }
        }
        // 圆角窗口区域：窗口尺寸/DPI 缩放/圆角设置变化后重设（缓存比对，常态零开销）；
        // 悬浮窗圆角=样式页 border_radius，字幕窗=字幕页 border_radius（原版各自独立）；
        // 确认窗=固定 12 逻辑 px（D-87，与 confirm.rs 画角同源）
        #[cfg(windows)]
        if matches!(id, WinId::Overlay | WinId::Subtitle | WinId::Confirm) {
            let size = window.inner_size();
            let (w, h) = (size.width, size.height);
            let radius = match id {
                WinId::Overlay => self.app_state.settings.style.border_radius,
                WinId::Subtitle => self.app_state.settings.subtitle_mode.border_radius,
                WinId::Confirm => CONFIRM_CORNER_RADIUS,
                _ => unreachable!("matches! 已收敛窗型"),
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
            if let Some(mode) = self.app_state.overlay.state.export_request.take() {
                self.run_export(mode);
            }
            if self.app_state.overlay.clear_request {
                self.app_state.overlay.clear_request = false;
                self.app_state.overlay.messages.clear();
            }
        }
    }

    /// 确认窗恒在顶（D-87）：topmost 带内点击激活会把其他置顶窗翻到确认窗
    /// 之上；SWP_NOACTIVATE 抬回，不抢用户刚点给悬浮窗的焦点（原生模态
    /// 「属主可点、模态恒顶」语义）
    #[cfg(windows)]
    fn ensure_confirm_on_top(&mut self) {
        if self.app_state.modal.confirm.is_none() || !self.is_visible(WinId::Confirm) {
            return;
        }
        let Some(hw) = self.find(WinId::Confirm) else {
            return;
        };
        let Some(hwnd) = Self::hwnd_of(&hw.window) else {
            return;
        };
        use ::windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        };
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    #[cfg(not(windows))]
    fn ensure_confirm_on_top(&mut self) {}

    /// 确认窗定位后显示（D-87）：锚定矩形 = 面板可见 → 面板外框；否则悬浮窗
    /// 可见 → 悬浮窗外框；全隐藏（托盘退出）→ 光标所在屏工作区居中——在哪点
    /// 退出就在哪弹。一律钳制在锚定矩形内不出生效区。
    fn position_and_show_confirm(&mut self) {
        let Some(hw) = self.find(WinId::Confirm) else {
            return;
        };
        let window = hw.window.clone();
        let size = window.inner_size();
        let (sw, sh) = (size.width as i32, size.height as i32);
        let target = match self.confirm_anchor_rect() {
            Some((ax, ay, aw, ah)) => (ax + ((aw - sw) / 2).max(0), ay + ((ah - sh) / 2).max(0)),
            None => {
                let area = Self::cursor_monitor_work_area().or_else(|| {
                    // 非 Windows / 光标屏获取失败：任一窗口当前屏兜底
                    self.find(WinId::Panel)
                        .and_then(|w| w.window.current_monitor())
                        .map(|m| {
                            let (p, s) = (m.position(), m.size());
                            (p.x, p.y, s.width as i32, s.height as i32)
                        })
                });
                let Some((wx, wy, ww, wh)) = area else {
                    return;
                };
                (wx + ((ww - sw) / 2).max(0), wy + ((wh - sh) / 2).max(0))
            }
        };
        window.set_outer_position(winit::dpi::PhysicalPosition::new(target.0, target.1));
        self.set_visible(WinId::Confirm, true);
    }

    /// 确认窗锚定矩形（物理 px）：面板可见优先（确认多由面板触发），
    /// 其次悬浮窗；全隐藏 → None（走光标屏）
    fn confirm_anchor_rect(&self) -> Option<(i32, i32, i32, i32)> {
        fn rect_of(hw: &HostedWindow) -> Option<(i32, i32, i32, i32)> {
            let p = hw.window.outer_position().ok()?;
            let s = hw.window.inner_size();
            Some((p.x, p.y, s.width as i32, s.height as i32))
        }
        for id in [WinId::Panel, WinId::Overlay] {
            if self.is_visible(id) {
                if let Some(r) = self.find(id).and_then(rect_of) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// 光标所在显示器工作区（物理 px；托盘触发的确认窗定位源）
    #[cfg(windows)]
    fn cursor_monitor_work_area() -> Option<(i32, i32, i32, i32)> {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        unsafe {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_err() {
                return None;
            }
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            if hmon.is_invalid() {
                return None;
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(hmon, &mut info).as_bool() {
                return None;
            }
            let wa = info.rcWork;
            Some((wa.left, wa.top, wa.right - wa.left, wa.bottom - wa.top))
        }
    }

    #[cfg(not(windows))]
    fn cursor_monitor_work_area() -> Option<(i32, i32, i32, i32)> {
        None
    }

    /// 把 AppState 的勾选状态同步到托盘菜单（三向同步的一环）
    /// 托盘状态同步（新版最小菜单：仅状态行文字；模型/语言切换收敛悬浮窗）
    fn sync_tray_checks(&mut self) {
        self.update_tray_status();
    }

    /// 应用级命令（W2：Menu(String) → AppCommand；托盘菜单与命令路由同源）
    fn on_command(&mut self, _event_loop: &ActiveEventLoop, cmd: lt_proto::AppCommand) {
        match cmd {
            lt_proto::AppCommand::Pause => {
                self.app_state.session.running = !self.app_state.session.running;
                if let Some(t) = &self.tray {
                    let text = if self.app_state.session.running {
                        lt_i18n::t("tray_pause")
                    } else {
                        lt_i18n::t("tray_resume")
                    };
                    t.set_pause_label(text);
                    let status = if self.app_state.session.running {
                        tray::IconStatus::Run
                    } else {
                        tray::IconStatus::Pause
                    };
                    t.set_status(status);
                }
                self.update_tray_status();
                tracing::info!(
                    "管道 {}",
                    if self.app_state.session.running {
                        "运行"
                    } else {
                        "暂停"
                    }
                );
            }
            lt_proto::AppCommand::OverlayToggle => {
                let vis = !self.is_visible(WinId::Overlay);
                self.set_overlay_visible_with_hint(vis);
            }
            lt_proto::AppCommand::ShowPanel => {
                let vis = !self.is_visible(WinId::Panel);
                self.set_visible(WinId::Panel, vis);
            }
            lt_proto::AppCommand::Quit => {
                // D-87：退出确认走专用窗（旧借画布方案在全隐藏时凭空拉面板，
                // docs/archive/quit-flow-redesign.md §一.1）。替换语义——其他确认开着
                // 时退出请求仍必达（未收敛确认无副作用，替换安全）
                self.app_state.modal.request_confirm_replacing(
                    ConfirmKind::Quit,
                    lt_i18n::t("quit_confirm_title"),
                    lt_i18n::t("quit_confirm_msg"),
                );
                self.position_and_show_confirm();
            }
        }
        self.apply_overlay_flags();
    }

    /// 刷新托盘状态行（● 状态 · 引擎 · 模型 · 源 → 目标；原版 status_action）
    fn update_tray_status(&mut self) {
        let s = &self.app_state;
        let state = if s.session.running {
            "运行"
        } else {
            "暂停"
        };
        let engine = s.overlay.asr_label.clone().unwrap_or_else(|| "--".into());
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
            t.set_status_line(text);
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
                        hw.window
                            .set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                    }
                }
                hw.window.set_visible(true);
                hw.window.focus_window();
                hw.window.request_redraw();
            } else {
                hw.window.set_visible(false);
                // 拖动中隐藏：立即收尾手工捕获（否则鼠标输入被吞在隐藏窗，
                // 主界面/其他窗失灵；D-37/W5 拖动与显隐并发兜底——字幕窗与
                // 悬浮窗同款）
                #[cfg(windows)]
                let dragged = (id == WinId::Subtitle && self.app_state.subtitle.state.dragging)
                    || (id == WinId::Overlay && self.app_state.overlay.state.dragging);
                if dragged {
                    #[cfg(windows)]
                    unsafe {
                        let _ = ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                    };
                    if id == WinId::Subtitle {
                        self.app_state.subtitle.state.dragging = false;
                        self.app_state.subtitle.state.drag_grab = None;
                    }
                    if id == WinId::Overlay {
                        self.app_state.overlay.state.dragging = false;
                        self.app_state.overlay.state.drag_grab = None;
                    }
                }
            }
        }
        self.visible.insert(id, vis);
        // W2/D-67：悬浮窗可见即起（或沿用既有）音频监视节拍
        if id == WinId::Overlay && vis {
            self.schedule_audio_monitor_tick();
        }
        // D-36 Z 序维护：字幕窗恒压悬浮窗之下、悬浮窗恒置顶（topmost 带内相对序），
        // 主界面永远可点可读；仅显示路径触发（隐藏无需维护）
        if vis {
            match id {
                WinId::Subtitle => self.restack_subtitle_below_overlay(),
                WinId::Overlay => self.raise_overlay(),
                _ => {}
            }
        }
    }

    /// Win32 HWND 提取（raw-window-handle 桥；失败 = 窗口句柄不可得）
    #[cfg(windows)]
    fn hwnd_of(window: &Window) -> Option<::windows::Win32::Foundation::HWND> {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let Ok(handle) = window.window_handle() else {
            return None;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return None;
        };
        Some(::windows::Win32::Foundation::HWND(
            win32.hwnd.get() as *mut core::ffi::c_void
        ))
    }

    /// Z 序维护：字幕窗压到悬浮窗正下方（同 topmost 带内下沉；NOACTIVATE 不抢焦点；
    /// D-36，原版两窗均置顶但相对序随意）
    #[cfg(windows)]
    fn restack_subtitle_below_overlay(&mut self) {
        use ::windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        };
        let (Some(ov), Some(sub)) = (self.find(WinId::Overlay), self.find(WinId::Subtitle)) else {
            return;
        };
        let (Some(ov_h), Some(sub_h)) = (Self::hwnd_of(&ov.window), Self::hwnd_of(&sub.window))
        else {
            return;
        };
        let _ = unsafe {
            SetWindowPos(
                sub_h,
                Some(ov_h),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
    }

    /// Z 序维护：悬浮窗抬顶（topmost 带顶；主界面按钮/下拉恒可读可点 — D-36）
    #[cfg(windows)]
    fn raise_overlay(&mut self) {
        use ::windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
        };
        let Some(ov) = self.find(WinId::Overlay) else {
            return;
        };
        let Some(h) = Self::hwnd_of(&ov.window) else {
            return;
        };
        let _ = unsafe {
            SetWindowPos(
                h,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
        // D-87：悬浮窗重申 Z 序后，确认窗开着则抬回（模态恒顶）
        self.ensure_confirm_on_top();
    }

    /// 当前窗所在显示器的工作区矩形（rcWork；物理 px → 逻辑 px）。
    /// winit 无 work-area API（既有已知偏差"任务栏按 48 逻辑 px 估"），D-36 起
    /// 钳屏改用 rcWork：字幕窗钳到屏底不再压任务栏（默认场景 = 视频底部）。
    /// 多显示器混合 DPI 为近似（统一用当前窗 scale 换算），已知近似。
    #[cfg(windows)]
    fn work_area_rect(window: &Window) -> Option<MonoRect> {
        use ::windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
        };
        let h = Self::hwnd_of(window)?;
        unsafe {
            let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
            if mon.is_invalid() {
                return None;
            }
            let mut info: MONITORINFOEXW = std::mem::zeroed();
            info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(mon, &mut info.monitorInfo).as_bool() {
                return None;
            }
            let s = window.scale_factor() as f32;
            let r = info.monitorInfo.rcWork;
            Some(MonoRect {
                x: (r.left as f32 / s).round() as i32,
                y: (r.top as f32 / s).round() as i32,
                w: ((r.right - r.left) as f32 / s).round() as i32,
                h: ((r.bottom - r.top) as f32 / s).round() as i32,
            })
        }
    }

    /// 悬浮窗显隐统一入口（托盘 OVERLAY_TOGGLE 与主面板"隐藏"按钮共用；
    /// 原版 on_toggle_overlay：托盘菜单文字翻转 + 首次隐藏提示）。
    /// D-33/H-1：**先藏后提示**——原位 rfd 同步弹窗（先弹后藏 + 无 owner 非置顶
    /// 被裸置顶悬浮窗压住）已移除，首次隐藏改原生通知（非阻塞）。
    fn set_overlay_visible_with_hint(&mut self, vis: bool) {
        if let Some(t) = &self.tray {
            let text = if vis {
                lt_i18n::t("tray_hide_overlay")
            } else {
                lt_i18n::t("tray_show_overlay")
            };
            t.set_overlay_toggle_label(text);
        }
        self.set_visible(WinId::Overlay, vis);
        if !vis && !self.overlay_hide_notified {
            self.overlay_hide_notified = true;
            crate::notifications::show_hidden_hint();
        }
    }

    /// 应用悬浮窗置顶/任务栏窗口 flags（穿透轮询 M4 接入）
    fn apply_overlay_flags(&mut self) {
        let Some(hw) = self.find(WinId::Overlay) else {
            return;
        };
        let level = if self.app_state.overlay.ov_topmost {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        hw.window.set_window_level(level);
        #[cfg(windows)]
        {
            hw.window
                .set_skip_taskbar(!self.app_state.overlay.ov_taskbar);
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
        if self.app_state.startup.load_dialog.take().is_some()
            && matches!(self.app_state.startup.flow, StartupFlow::Ready)
        {
            self.set_visible(WinId::Setup, false);
        }
    }

    // ── D-83 模型零信任修复闭环（docs/archive/model-trust-repair.md §2.2/§2.3）──

    /// 追加一行到下载卡片日志（失败/取消/进行中态通用；上限 200 与卡片一致）
    fn push_download_log(&mut self, line: &str) {
        use crate::state::DownloadUiState;
        let log = match &mut self.app_state.panel.download {
            DownloadUiState::Downloading { log, .. }
            | DownloadUiState::Cancelled { log }
            | DownloadUiState::Failed { log, .. } => log,
            DownloadUiState::Idle => return, // 无卡片上下文（提示只进日志窗）
        };
        log.push(line.to_string());
        let n = log.len();
        if n > 200 {
            log.drain(0..n - 200);
        }
    }

    /// 自动重试一轮（D-83 §2.3）：计数 +1、写卡片/日志窗、重发下载命令。
    /// 返回 false = 不自动重试（用尽或该失败类别需人工介入），调用方负责提醒。
    fn schedule_auto_retry(&mut self, retryable: bool) -> bool {
        let attempts = self.app_state.panel.auto_retry.attempts;
        let Some(next) = crate::state::auto_retry_decision(retryable, attempts) else {
            return false;
        };
        self.app_state.panel.auto_retry.attempts = next;
        let line = lt_i18n::t("model_repair_retrying")
            .replace("{n}", &next.to_string())
            .replace("{max}", &crate::state::DOWNLOAD_AUTO_RETRY_MAX.to_string());
        self.push_download_log(&line);
        self.push_log_line(30, "download", &line);
        let st = &mut self.app_state;
        // 自动路径不清零计数（与手动 start_download 的区别）
        crate::windows::panel::vad::start_download_auto(&mut st.panel, &st.session, &st.settings);
        true
    }

    /// 自动重试用尽/不可重试 → 提醒用户（原生通知 + 卡片日志 + 日志窗）
    fn notify_repair_giveup(&mut self, detail: &str) {
        let line = lt_i18n::t("model_repair_exhausted")
            .replace("{max}", &crate::state::DOWNLOAD_AUTO_RETRY_MAX.to_string());
        self.push_download_log(&line);
        self.push_log_line(40, "download", &format!("{line} — {detail}"));
        let title = lt_i18n::t("model_repair_exhausted_title");
        let body = format!("{line}\n{detail}");
        if let Err(e) = crate::notifications::show(&title, &body) {
            tracing::warn!("模型修复失败通知发送失败: {e}");
        }
        self.redraw(WinId::Panel);
    }

    /// 模型完整性/可加载性故障（D-83）：Hash → 坏文件已隔离、计数内自动重下；
    /// Unloadable → 指纹一致仍加载失败（重下无解），只提醒。
    fn on_model_integrity_failed(&mut self, model: String, fault: lt_proto::ModelFault) {
        match fault {
            lt_proto::ModelFault::Hash {
                file, quarantined, ..
            } => {
                let key = if quarantined {
                    "model_repair_quarantined"
                } else {
                    "model_repair_quarantine_failed"
                };
                let base = lt_i18n::t(key).replace("{file}", &file);
                self.push_download_log(&base);
                self.push_log_line(40, "download", &format!("{model}: {base}"));
                self.app_state.panel.auto_retry.model = model.clone();
                if quarantined {
                    // 隔离成功 → 缓存探测判缺 → 重下真正执行
                    let (n, max) = (
                        self.app_state.panel.auto_retry.attempts + 1,
                        crate::state::DOWNLOAD_AUTO_RETRY_MAX,
                    );
                    let line = base
                        .replace("{n}", &n.to_string())
                        .replace("{max}", &max.to_string());
                    if self.schedule_auto_retry(true) {
                        // schedule_auto_retry 已写通用行；此处补模型上下文行
                        self.push_download_log(&line);
                    } else {
                        self.notify_repair_giveup(&format!("{model} / {file}"));
                    }
                } else {
                    // 隔离失败（文件被占用等）：重下会被探测判"已缓存"而空转，
                    // 直接提醒用户人工清理，不做无效自动重试
                    self.notify_repair_giveup(&base);
                }
            }
            lt_proto::ModelFault::Unloadable { detail } => {
                let line = lt_i18n::t("model_unloadable").replace("{detail}", &detail);
                self.push_download_log(&line);
                self.push_log_line(40, "download", &format!("{model}: {line}"));
                if let Err(e) =
                    crate::notifications::show(&lt_i18n::t("model_unloadable_title"), &line)
                {
                    tracing::warn!("模型不可加载通知发送失败: {e}");
                }
            }
        }
        // 磁盘内容已变（隔离/重下），探测缓存失效
        self.app_state.panel.state.cache_probe = None;
        self.redraw(WinId::Panel);
    }

    /// Setup 窗节拍分派：向导倒计时（1s 一拍）与启动流成功后的 500ms 收尾延迟。
    /// 定时仅在向导 Idle / 成功待收尾期间存在（事件到达即重绘，无需节拍）。
    fn on_setup_tick(&mut self) {
        enum Action {
            Countdown,
            Finish,
            None,
        }
        let action = match &self.app_state.startup.flow {
            StartupFlow::Wizard(w) => match w.phase {
                crate::state::WizardPhase::Idle => Action::Countdown,
                crate::state::WizardPhase::Done => Action::Finish,
                _ => Action::None,
            },
            StartupFlow::DownloadMissing { finished, .. } => {
                if *finished {
                    Action::Finish
                } else {
                    Action::None
                }
            }
            StartupFlow::Ready => Action::None,
        };
        match action {
            Action::Countdown => {
                if let StartupFlow::Wizard(w) = &mut self.app_state.startup.flow {
                    w.countdown -= 1;
                }
                // 原版 _tick_countdown：归零即自动开始下载，否则按 1s 续拍
                if matches!(&self.app_state.startup.flow, StartupFlow::Wizard(w) if w.countdown <= 0)
                {
                    self.app_state.wizard_auto_start();
                } else {
                    self.app_state
                        .startup
                        .schedule_tick(&mut self.app_state.session, Duration::from_secs(1));
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
        self.app_state.startup.flow = StartupFlow::Ready;
        // 下载成功即启管道（AppShell），ModelLoadStart 可能落在 500ms 收尾期内——
        // 原版两个对话框先后出现，这里共用一个原生窗口，故加载框已开则保留窗口
        if self.app_state.startup.load_dialog.is_none() {
            self.set_visible(WinId::Setup, false);
        }
        let subtitle = self.app_state.settings.subtitle_mode.enabled;
        self.set_visible(WinId::Overlay, true);
        self.set_visible(WinId::Subtitle, subtitle);
        self.set_visible(WinId::Panel, true);
        self.visible.insert(WinId::Log, false);
        self.sync_tray_checks();
    }

    /// 面板设置防抖到期（原版 _do_auto_save → _apply_settings）：整体重放 ApplySettings
    fn on_panel_apply_tick(&mut self) {
        let now = Instant::now();
        if let Some(snapshot) = self.app_state.take_due_panel_apply(now) {
            tracing::info!("面板设置应用（300ms 防抖到期）");
            self.app_state
                .session
                .send_cmd(lt_proto::Cmd::ApplySettings(Box::new(snapshot)));
        }
    }

    /// 连接测试节拍（D-85）：走秒刷新 + 看门狗。
    ///
    /// 用宿主节拍而不是 `ctx.request_repaint_after`：本应用的多窗口重绘由
    /// `next_tick()` → `ControlFlow::WaitUntil` 驱动，面板若无人操作不会自己出帧——
    /// 靠 egui 的自动重绘会让"测试中… 0.0 秒"僵住且看门狗永不触发。
    /// 在途即续拍、收敛即停（不留空转节拍）。
    fn on_probe_tick(&mut self) {
        let now = Instant::now();
        let keep = self.app_state.panel.probe.tick(now);
        if keep {
            self.app_state.session.schedule_tick(
                WinId::Panel,
                crate::state::TickKind::ProbeTick,
                now + Duration::from_millis(100),
            );
        }
        self.redraw(WinId::Panel);
    }

    /// UiMsg 统一入口（管道事件/托盘事件）
    fn on_msg(&mut self, event_loop: &ActiveEventLoop, msg: UiMsg) {
        match msg {
            // 应用级命令（W2：Menu(String) 字符串协议 → AppCommand 类型化）
            UiMsg::AppCommand(cmd) => self.on_command(event_loop, cmd),
            // 事件动脉批量变体（W2：桥线程单次 wake 排空 ≤256 条一次投递）
            UiMsg::Events(events) => {
                for ev in events {
                    self.on_event(event_loop, ev);
                }
            }
            // W4：UiMsg::Cmd 变体已删（控制面 = mpsc 由 AppShell 直排）
            UiMsg::Event(e) => self.on_event(event_loop, e),
        }
    }

    /// 单条业务事件分发（on_msg 的 Event 臂；Events 批量逐一复用）
    fn on_event(&mut self, _event_loop: &ActiveEventLoop, e: UiEvent) {
        match e {
            // R11②/D-73（WD-5）：二次启动激活——显示面板并前置
            //（用户再次双击入口 → 既有界面浮到前台，而非"无响应"）
            lt_proto::UiEvent::SecondInstance => {
                self.set_visible(WinId::Panel, true);
                #[cfg(windows)]
                if let Some(hw) = self.find_mut(WinId::Panel) {
                    unsafe {
                        let _ = ::windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(
                            Self::hwnd_of(&hw.window).expect("Panel HWND 可得"),
                        );
                    }
                }
                self.redraw(WinId::Panel);
            }
            // 新识别消息 → 追加到悬浮窗消息链并重绘
            lt_proto::UiEvent::AddMessage {
                id,
                timestamp,
                original,
                lang,
                asr_ms,
            } => {
                self.app_state.overlay.push_message(OverlayMessage {
                    id,
                    timestamp,
                    original,
                    lang,
                    asr_ms,
                    translation: crate::state::TranslationView::Pending,
                    tl_ms: 0.0,
                });
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
            }
            // 流式译文增量（原版 update_streaming；50ms 节流渲染随 M4）
            lt_proto::UiEvent::UpdateStreaming { id, partial } => {
                self.app_state
                    .overlay
                    .update_streaming(&mut self.app_state.session, id, partial);
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
            }
            // 译文完成（W2 起**只表示成功译文**——空串不再流经此处）
            lt_proto::UiEvent::UpdateTranslation { id, text, tl_ms } => {
                self.app_state
                    .overlay
                    .update_translation(id, text.clone(), tl_ms);
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
                // W4/方案 §2.5 规则 4（回执闭环）：真的翻出来了 = 装置已恢复，
                // 清掉翻译页的红字（此前只写不清，修好后横幅常驻误导用户）
                if self.app_state.panel.translator_error.take().is_some() {
                    self.redraw(WinId::Panel);
                }
                self.feed_subtitle(id, SubtitleFeed::Translation(&text));
            }
            // W2：同语言免翻译（显式结论；字幕窗喂原文，原版语义）
            lt_proto::UiEvent::TranslationSkipped { id, reason } => {
                self.app_state.overlay.skip_translation(id, reason);
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
                self.feed_subtitle(id, SubtitleFeed::Skipped);
            }
            // W2：失败/无输出（带原因；字幕窗按失败态渲染 ⚠ + 警示色而
            // **不喂原文**——不给"没翻出来"伪装成翻译成功的机会）
            lt_proto::UiEvent::TranslationFailed {
                id,
                kind,
                detail,
                tl_ms,
            } => {
                self.app_state
                    .overlay
                    .fail_translation(id, kind, detail, tl_ms);
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
                self.feed_subtitle(id, SubtitleFeed::Failed(kind));
            }
            // 翻译/用量统计（原版 update_stats）
            lt_proto::UiEvent::UpdateStats {
                asr_n,
                tl_n,
                prompt_tokens,
                completion_tokens,
                cost_cny,
                cost_usd,
                usage_known,
            } => {
                self.app_state
                    .overlay
                    .update_stats(crate::state::OverlayStats {
                        asr_n,
                        tl_n,
                        prompt_tokens,
                        completion_tokens,
                        cost_cny,
                        cost_usd,
                        usage_known,
                    });
            }
            // 翻译装置切换生效（D-85/F2）：面板「当前使用」状态行给一次确认
            lt_proto::UiEvent::TranslatorSwitched { name, .. } => {
                self.app_state.panel.state.active_model_note = Some((name, Instant::now()));
                self.redraw(WinId::Panel);
            }
            // ASR 设备标签（悬浮窗 MonitorBar device 段）；同时视作加载框关闭信号
            //（原版 App.model_load_done 在设备就绪/不可用时都会被调用）
            lt_proto::UiEvent::AsrDevice(label) => {
                // D-83：装载成功 = 修复闭环的成功判据 → 自动重试计数清零
                self.app_state.panel.auto_retry = crate::state::DownloadAutoRetry::default();
                self.app_state.overlay.asr_label = Some(label);
                if let Some(t) = &self.tray {
                    let status = if self.app_state.session.running {
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
                self.app_state.overlay.asr_label = Some(lt_i18n::t("asr_unavailable"));
                if let Some(t) = &self.tray {
                    t.set_status(tray::IconStatus::Error);
                }
                if let Some(hw) = self.find_mut(WinId::Overlay) {
                    hw.window.request_redraw();
                }
                self.close_load_dialog();
            }
            // 翻译装置配置无效：状态行红字（翻译页）+ 日志已由 pipeline 落
            lt_proto::UiEvent::TranslatorUnavailable { reason } => {
                self.app_state.panel.translator_error = Some(reason);
                self.redraw(WinId::Panel);
            }
            // 翻译装置运行期降级回执（2026-09-10 第二轮评审 item 5 / 用户裁决）：
            // 回退阶梯试完全部关闭形态仍关不掉思维链 → 界面按"一比一"取消该模型
            // 的「关闭模型思考」勾选、写入持久化标记 `thinking_unavailable`，
            // 并落盘走既有设置链路（mark_settings_dirty → 宿主 about_to_wait
            // register_panel_apply 300ms 防抖 → Cmd::ApplySettings → shell 保存）。
            // 阶梯最终成功关闭（cannot_disable_thinking=false）→ 不动（裁决 5）。
            lt_proto::UiEvent::TranslatorDegraded {
                name,
                api_base,
                model,
                actual,
                cannot_disable_thinking,
            } => {
                tracing::warn!(
                    "翻译装置降级: {name}（{api_base} / {model}）实际在用 {actual}，\
                         关不掉思考={cannot_disable_thinking}"
                );
                if cannot_disable_thinking {
                    // (api_base, model) 双键定位（同名模型不误伤）——纯函数在
                    // state.rs，带回执落点单测
                    let matched = crate::state::apply_translator_degraded(
                        &mut self.app_state.settings,
                        &api_base,
                        &model,
                    );
                    // 编辑器打开中且指向同一条目 → 草稿同步（否则"确定"一次会把
                    // 标记与取消勾选一并抹掉，回执闭环失效）
                    if let Some(ed) = self.app_state.panel.state.model_editor.as_mut() {
                        if ed.api_base.trim() == api_base.trim() && ed.model.trim() == model.trim()
                        {
                            ed.apply_thinking_unavailable();
                        }
                    }
                    if matched {
                        crate::windows::panel::mark_settings_dirty(&mut self.app_state.session);
                    }
                    self.redraw(WinId::Panel);
                }
            }
            // 连接测试回执（D-85：按 probe_id 归位——迟到/被取代者一律丢弃，
            // 否则旧探测的结果会顶替新探测的状态）
            lt_proto::UiEvent::TestTranslatorResult {
                probe_id,
                name,
                outcome,
                ms,
                step_note,
                preview,
            } => {
                let adopted = self
                    .app_state
                    .panel
                    .probe
                    .settle_from_receipt(probe_id, name, outcome, ms, step_note, preview);
                if !adopted {
                    tracing::debug!("丢弃过期连接测试回执 #{probe_id}");
                }
                self.redraw(WinId::Panel);
            }
            // ── 启动流：下载进度（向导/缺模型对话框共用；W2 类型化，
            // 人读行由本臂按原格式生成；进度条直取事件字段）──
            lt_proto::UiEvent::Download(ev) => {
                let human = format_download_line(&ev);
                match &mut self.app_state.startup.flow {
                    StartupFlow::Wizard(w) => push_log_line(&mut w.log, human),
                    StartupFlow::DownloadMissing { log, .. } => push_log_line(log, human),
                    StartupFlow::Ready => {
                        // D-19 直进主界面 → 运行期下载：进度写识别页缓存卡片
                        self.app_state.panel.download.apply_progress(
                            ev.file.clone(),
                            ev.index,
                            ev.count,
                            ev.done,
                            ev.total.unwrap_or(0),
                        );
                        self.app_state.panel.download.push_log(human);
                        self.redraw(WinId::Panel);
                    }
                }
                self.redraw_setup();
            }
            // ── 启动流：下载失败（可重试；恢复控件 / 显示"关闭"按钮）──
            lt_proto::UiEvent::DownloadFailed { kind, message } => {
                let failed_line = lt_i18n::t("download_failed").replace("{error}", &message);
                match &mut self.app_state.startup.flow {
                    StartupFlow::Wizard(w) => {
                        w.phase = crate::state::WizardPhase::Failed;
                        push_log_line(&mut w.log, failed_line);
                    }
                    StartupFlow::DownloadMissing { failed, log, .. } => {
                        *failed = Some(message);
                        push_log_line(log, failed_line);
                    }
                    StartupFlow::Ready => {
                        // 运行期下载失败：分类提示 + 历史日志收进卡片（P0-1/P1-6）
                        let mut log = match &self.app_state.panel.download {
                            DownloadUiState::Downloading { log, .. }
                            | DownloadUiState::Cancelled { log }
                            | DownloadUiState::Failed { log, .. } => log.clone(),
                            _ => Vec::new(),
                        };
                        log.push(failed_line.clone());
                        self.app_state.panel.download = DownloadUiState::Failed {
                            kind,
                            detail: message.clone(),
                            log,
                        };
                        // DL-6/F12：磁盘内容已变，探测缓存失效
                        self.app_state.panel.state.cache_probe = None;
                        // D-83 §2.3：自动重试（上限 3 次；磁盘/取消不自动）。
                        // 用尽仍失败 → 提醒用户（卡片保持失败态 + 原生通知）
                        if !self.schedule_auto_retry(crate::state::download_failure_retryable(kind))
                        {
                            self.notify_repair_giveup(&message);
                        }
                        self.redraw(WinId::Panel);
                    }
                }
                self.redraw_setup();
            }
            // ── 下载取消（DL-4/D-23：卡片进「已取消，进度已保留」态）──
            lt_proto::UiEvent::DownloadCancelled => {
                // 启动流（向导/缺模型）无取消入口，仅运行期卡片处理
                if let StartupFlow::Ready = self.app_state.startup.flow {
                    let mut log = match &self.app_state.panel.download {
                        DownloadUiState::Downloading { log, .. }
                        | DownloadUiState::Cancelled { log }
                        | DownloadUiState::Failed { log, .. } => log.clone(),
                        _ => Vec::new(),
                    };
                    log.push(lt_i18n::t("download_cancelled_title").to_string());
                    self.app_state.panel.download = DownloadUiState::Cancelled { log };
                    // DL-6/F12：磁盘内容已变，探测缓存失效
                    self.app_state.panel.state.cache_probe = None;
                    self.redraw(WinId::Panel);
                }
            }
            // ── D-83 模型完整性故障（零信任加载闸门）──
            // Hash：坏文件已隔离 → 探测判缺 → 计数内自动重下；
            // Unloadable：指纹一致仍加载失败（重下无解）→ 只提醒
            lt_proto::UiEvent::ModelIntegrityFailed { model, fault } => {
                self.on_model_integrity_failed(model, fault);
            }
            // ── 启动流：下载成功（应用下发设置 + 500ms 后收尾关窗）──
            lt_proto::UiEvent::DownloadSucceeded { .. } => {
                // DL-4/DEC-4：UI 是 settings 事实源，不再用事件载荷覆盖本状态
                //（F6 回踩）；落盘走单写者 shell.persist_settings——启动流在此
                // 发 PersistSettings，运行期由 AppShell 重发 SwitchEngine 顺带落盘
                if !matches!(self.app_state.startup.flow, StartupFlow::Ready) {
                    let s = self.app_state.settings.clone();
                    self.app_state
                        .session
                        .send_cmd(lt_proto::Cmd::PersistSettings(Box::new(s)));
                }
                let done_line = lt_i18n::t("download_complete");
                match &mut self.app_state.startup.flow {
                    StartupFlow::Wizard(w) => {
                        push_log_line(&mut w.log, done_line);
                        w.phase = crate::state::WizardPhase::Done;
                    }
                    StartupFlow::DownloadMissing { log, finished, .. } => {
                        push_log_line(log, done_line);
                        *finished = true;
                    }
                    StartupFlow::Ready => {
                        // 运行期下载成功：卡片回到已缓存（探测缓存已失效，
                        // 下一次渲染重扫磁盘翻转）+ 引擎热切换由 AppShell 处理
                        self.app_state.panel.download = DownloadUiState::Idle;
                        self.app_state.panel.state.cache_probe = None;
                        self.redraw(WinId::Panel);
                    }
                }
                // 原版 QTimer.singleShot(500, accept)：安排 500ms 收尾节拍
                self.app_state
                    .startup
                    .cancel_tick(&mut self.app_state.session);
                self.app_state
                    .startup
                    .schedule_tick(&mut self.app_state.session, Duration::from_millis(500));
                self.redraw_setup();
            }
            // 日志行（常驻桥接线程全程转发 → 日志窗；级别过滤在窗口状态内）。
            // target=="download"：下载人读行（W2）——除日志窗外同步进
            // 向导日志 / 识别页下载卡片（原 DownloadProgress 的日志面）
            lt_proto::UiEvent::LogLine { level, target, msg } => {
                if target == "download" {
                    match &mut self.app_state.startup.flow {
                        StartupFlow::Wizard(w) => push_log_line(&mut w.log, msg.clone()),
                        StartupFlow::DownloadMissing { log, .. } => push_log_line(log, msg.clone()),
                        StartupFlow::Ready => {
                            self.app_state.panel.download.push_log(msg.clone());
                            self.redraw(WinId::Panel);
                        }
                    }
                    self.redraw_setup();
                }
                if self.app_state.log.logwin.push(crate::state::LogLineEntry {
                    time: chrono::Local::now().format("%H:%M:%S").to_string(),
                    level,
                    target,
                    msg,
                }) {
                    self.redraw(WinId::Log);
                    // LT-5：面板「日志」tab 与日志窗共用同一缓冲——该 tab
                    // 正在展示时一并重绘，否则面板无输入事件不刷新，新日志
                    // 落不到画面（观感为"日志卡住"）
                    if self.app_state.panel.state.page == PanelPage::Log
                        && self.visible.get(&WinId::Panel).copied().unwrap_or(false)
                    {
                        self.redraw(WinId::Panel);
                    }
                }
            }
            // ── 性能基准流（W2：Line 逐行 → 基准窗渲染；Finished 复位
            // 运行态 + 完成提示——替代 LogLine[benchmark] + 完成哨兵）──
            lt_proto::UiEvent::Bench(ev) => match ev {
                lt_proto::BenchEvent::Line(l) => {
                    self.app_state.bench.push_line(l);
                    self.redraw(WinId::Benchmark);
                }
                lt_proto::BenchEvent::Finished { ok, elapsed_ms } => {
                    self.app_state.bench.push_line(format!(
                        "=== {} ({elapsed_ms}ms) ===",
                        lt_i18n::t("bench_done_title")
                    ));
                    self.redraw(WinId::Benchmark);
                    if self.app_state.bench.running {
                        self.app_state.bench.running = false;
                        tracing::info!("性能基准完成（ok={ok}，{elapsed_ms}ms）");
                        // D-33/H-5：完成提示改原生通知（原位 rfd 同步框
                        // 在事件循环线程内阻塞）
                        if let Err(e) = crate::notifications::show(
                            &lt_i18n::t("bench_done_title"),
                            &lt_i18n::t("bench_done_msg"),
                        ) {
                            tracing::warn!("基准完成提示（原生通知）失败: {e}");
                        }
                    }
                }
            },
            // ── 有界队列水位（R15②：翻译池丢最旧段——日志窗可见告警；
            // 队列原语内部已按同节奏落 tracing warn，此事件保证类型化
            // 信号存在，W5 起的呈现深化钩子）──
            lt_proto::UiEvent::QueuePressure {
                queue,
                dropped_total,
            } => {
                let text = format!(
                    "{}（{queue:?}，累计丢弃 {dropped_total}）",
                    lt_i18n::t("queue_pressure").replace("{dropped}", &dropped_total.to_string())
                );
                self.push_log_line(30, "queue", &text);
            }
            // ── 音频采集可用性（架构 2.0 W1/R4/D-62）：日志窗可见告警。
            // 事件本身边沿触发（Unavailable 转坏一次/Recovered 转好一次），
            // 语义对齐 AsrUnavailable；面板设备卡片深化呈现随 W5 ──
            lt_proto::UiEvent::Capture(ev) => {
                use lt_proto::{AudioRole, CaptureEvent};
                let (key, role, detail, level) = match ev {
                    CaptureEvent::Unavailable { role, error } => {
                        tracing::warn!("音频采集不可用: {role:?} {error}");
                        ("capture_unavailable", role, error, 30u8)
                    }
                    CaptureEvent::Recovered { role } => {
                        tracing::info!("音频采集已恢复: {role:?}");
                        ("capture_recovered", role, String::new(), 20u8)
                    }
                };
                let role_text = match role {
                    AudioRole::Loopback => lt_i18n::t("capture_role_loopback"),
                    AudioRole::Mic => lt_i18n::t("capture_role_mic"),
                };
                let text = if detail.is_empty() {
                    format!("{}（{role_text}）", lt_i18n::t(key))
                } else {
                    format!("{}（{role_text}）：{detail}", lt_i18n::t(key))
                };
                self.push_log_line(level, "capture", &text);
            }
            // ── 被监督线程死亡（架构 2.0 W1/R1/D-62）：线程 panic 不再黑洞
            // ——panic 详情由 hook 落 crash 文件与 tracing，此处日志窗可见 ──
            lt_proto::UiEvent::ThreadDied(d) => {
                tracing::error!(
                    "线程死亡: {:?} restarted={} detail={}",
                    d.role,
                    d.restarted,
                    d.detail
                );
                let state = if d.restarted {
                    lt_i18n::t("thread_restarted")
                } else {
                    lt_i18n::t("thread_not_restarted")
                };
                let text = format!("{}（{state}）：{}", lt_i18n::t("thread_died"), d.detail);
                self.push_log_line(40, "supervisor", &text);
            }
            // ── 模型加载对话框打开（标题固定 "LiveTranslate"）──
            lt_proto::UiEvent::ModelLoadStart(label) => {
                self.app_state.startup.load_dialog = Some(label);
                self.set_visible(WinId::Setup, true);
                self.set_setup_title("LiveTranslate");
                self.redraw_setup();
            }
            // ── 模型加载结束：关闭加载框（仅 load_dialog 显示中才动作）──
            lt_proto::UiEvent::ModelLoadDone { .. } => self.close_load_dialog(),
            // ── 音频设备枚举回执（W5/R13）：`Cmd::RefreshDevices` 的响应——
            // 面板识别页设备缓存替换为事件载荷（帧内 COM 枚举已下线）；
            // 空表也落 Ready（收敛在途态，避免 Probe 重发）──
            lt_proto::UiEvent::Devices(list) => {
                self.app_state.panel.state.devices = crate::state::DevicesState::Ready(list);
                self.redraw(WinId::Panel);
            }
            // ── 导出保存路径回执（W5/R19）：rfd 对话框在编排域一次性线程
            // 弹出（事件循环零阻塞）；选中后按模式组行写文件 ──
            lt_proto::UiEvent::ExportSave { mode, path } => {
                if let Some(path) = path {
                    self.write_export_file(&path, mode);
                }
            }
            // ── 字幕背景图路径回执（W5/R19）：同上；选中即写设置 + 防抖 ──
            lt_proto::UiEvent::BgImagePicked { path } => {
                if let Some(path) = path {
                    self.app_state.settings.subtitle_mode.bg_image = path;
                    crate::windows::panel::mark_settings_dirty(&mut self.app_state.session);
                    self.redraw(WinId::Panel);
                }
            }
        }
    }

    /// 重绘指定窗口（节拍/动作路径的便捷入口）
    /// 日志窗 + 面板日志 tab 的统一入缓冲/重绘入口（与 LogLine 臂同款刷新
    /// 语义：任一视图在展示时都要同帧重绘，否则无输入事件不刷新——观感"日志卡住"）
    fn push_log_line(&mut self, level: u8, target: &str, msg: &str) {
        if self.app_state.log.logwin.push(crate::state::LogLineEntry {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            target: target.to_string(),
            msg: msg.to_string(),
        }) {
            self.redraw(WinId::Log);
            if self.app_state.panel.state.page == PanelPage::Log
                && self.visible.get(&WinId::Panel).copied().unwrap_or(false)
            {
                self.redraw(WinId::Panel);
            }
        }
    }

    fn redraw(&mut self, id: WinId) {
        if let Some(hw) = self.find(id) {
            hw.window.request_redraw();
        }
    }

    /// 字幕窗文本喂入（原版 pipeline 仅 `_subwin.isVisible()` 时 update_text）。
    /// W2/方案 §4.4 + 2026-09-10 item 8/9：按**呈现态**取类型化三态值
    /// （[`SubtitleFeed`]）——成功喂译文；同语言免翻译喂原文（原版语义）；
    /// 失败喂失败标记（译文行渲染 ⚠ + 警示色，见 subtitle::refresh_display）。
    fn feed_subtitle(&mut self, id: u64, feed: SubtitleFeed<'_>) {
        if !*self.visible.get(&WinId::Subtitle).unwrap_or(&false) {
            return;
        }
        let Some(original) = self
            .app_state
            .overlay
            .messages
            .iter()
            .rev()
            .find(|m| m.id == id)
            .map(|m| m.original.clone())
        else {
            return;
        };
        let mut tl = std::collections::BTreeMap::new();
        let failed = match feed {
            SubtitleFeed::Translation(text) => {
                tl.insert(
                    self.app_state.settings.target_language.clone(),
                    text.to_string(),
                );
                None
            }
            SubtitleFeed::Skipped => {
                tl.insert(
                    self.app_state.settings.target_language.clone(),
                    original.clone(),
                );
                None
            }
            SubtitleFeed::Failed(kind) => Some(kind),
        };
        self.app_state.subtitle.update_text(
            &mut self.app_state.session,
            &self.app_state.settings,
            original,
            tl,
            failed,
        );
        self.redraw(WinId::Subtitle);
    }

    /// 消费 UI 帧入队的窗口动作（run_frame 尾部调用；winit 句柄操作在此）
    fn process_actions(&mut self) {
        for (win, action) in self.app_state.session.drain_actions() {
            let Some(hw) = self.find(win) else { continue };
            let window = hw.window.clone();
            match action {
                // W5/D-71：弃 winit drag_window（标题栏模态循环 + 哑 WM_MOUSEMOVE
                // 取消；LAYERED/TRANSPARENT 轮询窗实测 0 位移/挂死，见 AGENTS 大坑
                // 17；且中键下循环不启动）。改原版 Qt 语义：SetCapture + 光标绝对
                // 跟踪 set_outer_position（process_actions 尾 update_overlay_drag
                // 每帧推进，任意按键可用）——与字幕窗 D-37 同法。
                WinAction::OverlayDragStart => {
                    #[cfg(windows)]
                    {
                        let grab = Self::hwnd_of(&window).and_then(|hwnd| {
                            let mut pt = ::windows::Win32::Foundation::POINT::default();
                            if unsafe {
                                ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt)
                            }
                            .is_err()
                            {
                                return None;
                            }
                            let pos = window.outer_position().ok()?;
                            unsafe {
                                let _ =
                                    ::windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                            };
                            Some((pt.x - pos.x, pt.y - pos.y))
                        });
                        self.app_state.overlay.state.drag_grab = grab;
                    }
                    self.app_state.overlay.state.dragging = true;
                    // 拖动期间恒非穿透（穿透轮询见 poll_click_through 的 dragging 豁免）
                    Self::set_window_transparent(&window, false);
                    if self.app_state.overlay.state.drag_grab.is_none() {
                        // 抓握记录失败（GetCursorPos/窗位异常）：立即收尾，不悬空拖动态
                        self.app_state.overlay.state.dragging = false;
                    }
                }
                WinAction::OverlayDragEnd => {
                    #[cfg(windows)]
                    unsafe {
                        let _ = ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                    }
                    self.app_state.overlay.state.dragging = false;
                    self.app_state.overlay.state.drag_grab = None;
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
                // W5：跨域"打开面板某页"意图（字幕窗"打开设置"）——根消费
                WinAction::OpenPanelPage(page) => {
                    self.app_state.panel.state.page = page;
                    self.set_visible(WinId::Panel, true);
                }
                WinAction::ShowConfirm => {
                    // D-87：确认窗定位（属主可见→属主居中；全隐藏→光标屏）后显示
                    self.position_and_show_confirm();
                }
                WinAction::HideConfirm => {
                    // D-87：确认收敛（确定/取消/X/ESC）后隐藏
                    self.set_visible(WinId::Confirm, false);
                }
                WinAction::ShowBenchmark => {
                    // 原版 BenchmarkDialog.exec()：显示独立工具窗
                    self.set_visible(WinId::Benchmark, true);
                }
                WinAction::ToggleSubtitle => {
                    let vis = self.app_state.settings.subtitle_mode.enabled;
                    self.set_visible(WinId::Subtitle, vis);
                    if vis {
                        // WP-1：首次开启弹拖动提示（原版 _subwin_notified 会话内一次性）
                        if self.app_state.subtitle.take_subtitle_hint() {
                            crate::notifications::show_subtitle_hint();
                        }
                        // D-36：可见即续 100ms 分区穿透/悬停轮询（不再仅穿透开启时）
                        self.app_state
                            .subtitle
                            .schedule_window_poll(&mut self.app_state.session);
                    }
                }
                WinAction::ApplyOverlayFlags => {
                    self.apply_overlay_flags();
                }
                WinAction::ToggleMode => {
                    let compact =
                        self.app_state.overlay.state.mode == crate::state::OverlayMode::Compact;
                    let cur_h = window
                        .inner_size()
                        .to_logical::<f32>(window.scale_factor())
                        .height;
                    let (from, to) = if compact {
                        self.app_state.overlay.state.height_before_compact = Some(cur_h);
                        (cur_h, 200.0) // 200 = 原版 minimumHeight
                    } else {
                        (
                            cur_h,
                            self.app_state
                                .overlay
                                .state
                                .height_before_compact
                                .unwrap_or(500.0),
                        )
                    };
                    // 差距过小直接落位（原版 abs(actual-target)<10 分支）
                    if (from - to).abs() < 10.0 {
                        self.enqueue_height(to);
                    } else {
                        self.app_state.overlay.state.anim = Some(crate::state::HeightAnim {
                            from,
                            to,
                            start: Instant::now(),
                        });
                    }
                }
                WinAction::SetHeight(h) => {
                    self.enqueue_height(h);
                }
                WinAction::SubtitleDragStart => {
                    // D-37：弃 winit drag_window（标题栏模态循环 + 哑 WM_MOUSEMOVE
                    // 取消；LAYERED/TRANSPARENT 轮询窗实测 0 位移/挂死，见 AGENTS
                    // 大坑 17；且中键下模态循环根本不会启动）。改原版 Qt 语义：
                    // SetCapture + 光标绝对跟踪 set_outer_position（process_actions
                    // 尾部 update_subtitle_drag 每帧推进，任意按键可用）。
                    #[cfg(windows)]
                    {
                        let grab = Self::hwnd_of(&window).and_then(|hwnd| {
                            let mut pt = ::windows::Win32::Foundation::POINT::default();
                            if unsafe {
                                ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt)
                            }
                            .is_err()
                            {
                                return None;
                            }
                            let pos = window.outer_position().ok()?;
                            // SetCapture：光标出窗后鼠标输入仍投递本窗（同原版 Qt
                            // 按压隐式抓取）；释放路径见 SubtitleDragEnd
                            unsafe {
                                let _ =
                                    ::windows::Win32::UI::Input::KeyboardAndMouse::SetCapture(hwnd);
                            };
                            Some((pt.x - pos.x, pt.y - pos.y))
                        });
                        self.app_state.subtitle.state.drag_grab = grab;
                    }
                    self.app_state.subtitle.state.dragging = true;
                    // 拖动期间恒非穿透（穿透轮询见 poll_subtitle_window 的 dragging 豁免）
                    Self::set_window_transparent(&window, false);
                    if self.app_state.subtitle.state.drag_grab.is_none() {
                        // 抓握记录失败（GetCursorPos/窗位异常）：立即收尾，不悬空拖动态
                        self.app_state.subtitle.state.dragging = false;
                    }
                }
                WinAction::SubtitleDragEnd => {
                    #[cfg(windows)]
                    {
                        unsafe {
                            let _ = ::windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture();
                        };
                    }
                    self.app_state.subtitle.state.dragging = false;
                    self.app_state.subtitle.state.drag_grab = None;
                    // 落点钳制（多屏钳回 + 防抖保存；经既有路径触发重叠避让）
                    self.app_state
                        .session
                        .enqueue_action(WinId::Subtitle, WinAction::ClampSubtitlePos);
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
                    let cur = (
                        (pos.x as f32 / scale as f32) as i32,
                        (pos.y as f32 / scale as f32) as i32,
                    );
                    // current_monitor 优先作兜底屏（对齐原版"未命中 → 主屏"的就近语义）
                    let mut monitors =
                        Self::monitor_rects(window.available_monitors(), window.current_monitor());
                    // D-36：首选（窗口当前所在）屏矩形替换为工作区 rcWork（钳屏不压任务栏）
                    #[cfg(windows)]
                    if let Some(wa) = Self::work_area_rect(&window) {
                        if let Some(first) = monitors.first_mut() {
                            *first = wa;
                        }
                    }
                    let (x, y) = clamp_to_screen(
                        cur.0,
                        cur.1,
                        size.width as i32,
                        size.height as i32,
                        &monitors,
                    );
                    if (x, y) != cur {
                        window.set_outer_position(winit::dpi::LogicalPosition::new(
                            f64::from(x),
                            f64::from(y),
                        ));
                    }
                    self.app_state
                        .subtitle
                        .schedule_pos_save(&mut self.app_state.session, (x, y));
                }
                WinAction::ResetSubtitlePos => {
                    // D-36 字幕窗复位（顶条右键菜单/字幕页按钮）：回 (100,100)，
                    // 位置直写 settings + 防抖基准同步（同 ResetPositions 的字幕分支）
                    if let Some(sub) = self.find_mut(WinId::Subtitle) {
                        sub.window
                            .set_outer_position(winit::dpi::LogicalPosition::new(100.0, 100.0));
                    }
                    self.app_state.settings.subtitle_mode.window_x = Some(100);
                    self.app_state.settings.subtitle_mode.window_y = Some(100);
                    self.app_state.subtitle.state.last_saved_pos = Some((100, 100));
                    self.app_state
                        .session
                        .send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
                            self.app_state.settings.clone(),
                        )));
                }
                WinAction::ResetPositions => {
                    // 原版 app_shell._on_reset_positions：字幕窗回 (100,100)；
                    // 悬浮窗回主屏右下 (right-ow-50, bottom-oh-100)。窗口移动后由
                    // 既有 Moved → 防抖保存路径写回 settings（此处同步直写一次兜底）。
                    if let Some(sub) = self.find_mut(WinId::Subtitle) {
                        sub.window
                            .set_outer_position(winit::dpi::LogicalPosition::new(100.0, 100.0));
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
                                .set_outer_position(winit::dpi::LogicalPosition::new(
                                    f64::from(x),
                                    f64::from(y),
                                ));
                        }
                    }
                    // settings 同步（原版 _save_subwin_state/_save_overlay_pos 即时保存）
                    let geo = self.overlay_geo();
                    let s = &mut self.app_state.settings;
                    s.subtitle_mode.window_x = Some(100);
                    s.subtitle_mode.window_y = Some(100);
                    s.overlay_x = Some(geo.0);
                    s.overlay_y = Some(geo.1);
                    self.app_state.subtitle.state.last_saved_pos = Some((100, 100));
                    self.app_state
                        .session
                        .send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
                            self.app_state.settings.clone(),
                        )));
                }
            }
        }
        // 动画推进：结束帧落定终值并清除（进行中由 UI 帧投递 SetHeight）
        if let Some(anim) = self.app_state.overlay.state.anim {
            if anim.current(Instant::now()).is_none() {
                let target = anim.to;
                self.app_state.overlay.state.anim = None;
                self.enqueue_height(target);
            }
        }
        // D-37/W5：字幕窗/悬浮窗拖动进行中每帧推进（光标绝对跟踪；帧由捕获
        // 的鼠标移动事件持续供给——request_redraw → run_frame 回环）
        if self.app_state.subtitle.state.dragging {
            self.update_subtitle_drag();
        }
        if self.app_state.overlay.state.dragging {
            self.update_overlay_drag();
        }
    }

    /// 当前悬浮窗逻辑几何 (x, y, w, h)（防抖登记用）
    fn overlay_geo(&self) -> (i32, i32, u32, u32) {
        let Some(hw) = self.find(WinId::Overlay) else {
            return (0, 0, 0, 0);
        };
        let scale = hw.window.scale_factor() as f32;
        let pos = hw.window.outer_position().unwrap_or_default();
        let logical = hw
            .window
            .inner_size()
            .to_logical::<f32>(hw.window.scale_factor());
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

    /// 当前字幕窗逻辑几何 (x, y, w, h)（避让判定用）
    fn subtitle_geo(&self) -> (i32, i32, i32, i32) {
        let Some(hw) = self.find(WinId::Subtitle) else {
            return (0, 0, 0, 0);
        };
        let scale = hw.window.scale_factor() as f32;
        let pos = hw.window.outer_position().unwrap_or_default();
        let size = hw
            .window
            .inner_size()
            .to_logical::<f32>(hw.window.scale_factor());
        (
            (pos.x as f32 / scale) as i32,
            (pos.y as f32 / scale) as i32,
            size.width as i32,
            size.height as i32,
        )
    }

    /// 重叠避让（D-36）：拖动落点（字幕窗/悬浮窗 Moved 防抖回调）后执行——
    /// 相交则把字幕窗沿最短方向推出悬浮窗并钳屏；无处可退保持现状
    /// （Z 序兜底：字幕窗恒压悬浮窗之下，主界面始终可操作）。
    fn try_resolve_overlap(&mut self) {
        let Some(sub) = self.find(WinId::Subtitle) else {
            return;
        };
        if self.find(WinId::Overlay).is_none() {
            return;
        }
        let visible = |id: WinId| self.visible.get(&id).copied().unwrap_or(false);
        if !visible(WinId::Subtitle) || !visible(WinId::Overlay) {
            return;
        }
        let sub_geo = self.subtitle_geo();
        let ov_geo = self.overlay_geo();
        let ov = (ov_geo.0, ov_geo.1, ov_geo.2 as i32, ov_geo.3 as i32);
        if !rects_intersect(sub_geo, ov, crate::windows::subtitle::OVERLAP_SAFETY) {
            return;
        }
        let monitors = Self::monitor_rects(
            sub.window.available_monitors(),
            sub.window.current_monitor(),
        );
        if let Some((nx, ny)) = resolve_overlap(
            sub_geo,
            ov,
            &monitors,
            crate::windows::subtitle::OVERLAP_SAFETY,
        ) {
            // 让位位置直写（防抖保存由 Moved 事件承继）
            sub.window
                .set_outer_position(winit::dpi::LogicalPosition::new(
                    f64::from(nx),
                    f64::from(ny),
                ));
            self.app_state
                .subtitle
                .schedule_pos_save(&mut self.app_state.session, (nx, ny));
        }
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
        let Some(hw) = self.find(WinId::Overlay) else {
            return;
        };
        let scale = hw.window.scale_factor();
        let w = hw.window.inner_size().to_logical::<f32>(scale).width;
        let _ = hw
            .window
            .request_inner_size(winit::dpi::LogicalSize::new(w, h.max(200.0)));
    }

    /// 位置/尺寸防抖到期：读几何（逻辑 px）写设置并持久化（原版 position_changed）
    fn on_pos_save_tick(&mut self) {
        let Some(hw) = self.find(WinId::Overlay) else {
            return;
        };
        let window = hw.window.clone();
        let scale = window.scale_factor() as f32;
        let Ok(pos) = window.outer_position() else {
            return;
        };
        let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
        let x = (pos.x as f32 / scale) as i32;
        let y = (pos.y as f32 / scale) as i32;
        let w = logical.width as u32;
        // 精简形态下窗口高度是收起值（200），不该落盘——否则重启按「完整形态 +
        // 矮高度」开场，控件挤压。落盘高度一律取完整态高度（收起前记录值；
        // 兜底 500 = 宿主展开分支同款缺省）。
        let h = if self.app_state.overlay.state.mode == crate::state::OverlayMode::Compact {
            self.app_state
                .overlay
                .state
                .height_before_compact
                .map(|v| v as u32)
                .unwrap_or(500)
        } else {
            logical.height as u32
        };
        let geo = (x, y, w, h);
        self.app_state.overlay.state.pos_dirty_since = None;
        if self.app_state.overlay.state.last_saved_geo == Some(geo) {
            return;
        }
        self.app_state.overlay.state.last_saved_geo = Some(geo);
        self.app_state.settings.overlay_x = Some(x);
        self.app_state.settings.overlay_y = Some(y);
        self.app_state.settings.overlay_w = Some(w);
        self.app_state.settings.overlay_h = Some(h);
        self.app_state
            .session
            .send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
                self.app_state.settings.clone(),
            )));
        // D-36：悬浮窗落点避让（与字幕窗相交则推出字幕窗；让位经 Moved 防抖再保存）
        self.try_resolve_overlap();
    }

    /// 字幕窗位置防抖到期：读位置（逻辑 px）写 window_x/y 并持久化（原版 position_changed）
    fn on_subtitle_pos_save_tick(&mut self) {
        let Some(hw) = self.find(WinId::Subtitle) else {
            return;
        };
        let window = hw.window.clone();
        let scale = window.scale_factor() as f32;
        let Ok(pos) = window.outer_position() else {
            return;
        };
        let p = ((pos.x as f32 / scale) as i32, (pos.y as f32 / scale) as i32);
        self.app_state.subtitle.state.pos_dirty_since = None;
        if self.app_state.subtitle.state.last_saved_pos == Some(p) {
            return;
        }
        self.app_state.subtitle.state.last_saved_pos = Some(p);
        self.app_state.settings.subtitle_mode.window_x = Some(p.0);
        self.app_state.settings.subtitle_mode.window_y = Some(p.1);
        self.app_state
            .session
            .send_cmd(lt_proto::Cmd::PersistSettings(Box::new(
                self.app_state.settings.clone(),
            )));
        // D-36：字幕窗落点避让（与悬浮窗相交则推出；让位位置直写后 Moved 防抖兜底）
        self.try_resolve_overlap();
    }

    /// 穿透轮询（原版 _check_click_through 50ms）：光标在头部区（消息区之上）
    /// 时可交互，否则正文穿透。仅 Windows。
    #[cfg(windows)]
    fn poll_click_through(&mut self) {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(hw) = self.find(WinId::Overlay) else {
            return;
        };
        let window = hw.window.clone();
        let enabled = self.app_state.overlay.ov_click_through;
        if !enabled {
            Self::set_window_transparent(&window, false);
            return;
        }
        // D-33/H-4 的「确认模态打开期间强制非穿透」已随 D-87 删除：确认改
        // 专用独立窗，悬浮窗穿透不再影响其可操作性。
        // W5/D-71：拖动进行中恒非穿透（光标随时出窗——光标判定会把
        // TRANSPARENT 重新挂上，打断 SetCapture 供给的输入链；豁免直到
        // OverlayDragEnd 收尾；与字幕窗 D-37 同规则）
        if self.app_state.overlay.state.dragging {
            Self::set_window_transparent(&window, false);
            return;
        }
        let Ok(win_pos) = window.outer_position() else {
            return;
        };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let scale = window.scale_factor();
        let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
        let local_x = (pt.x as f64 - win_pos.x as f64) / scale;
        let local_y = (pt.y as f64 - win_pos.y as f64) / scale;
        let header_px = self.app_state.overlay.state.header_px as f64;
        let in_header = local_x >= 0.0
            && local_x <= logical.width as f64
            && local_y >= 0.0
            && local_y < header_px;
        Self::set_window_transparent(&window, !in_header);
    }

    /// 字幕窗 100ms 光标感知轮询（D-36）：分区穿透（正文穿透 + 顶条豁免 +
    /// Ctrl 临时恢复）与顶条悬停工具条显隐的**单一事实源**——穿透开启时 egui
    /// 收不到鼠标事件，悬停判定只能走 Win32 GetCursorPos（不受 WS_EX_TRANSPARENT
    /// 影响）。原版 _ct_timer 500ms 全窗穿透 -> 100ms 分区判定。仅 Windows。
    #[cfg(windows)]
    fn poll_subtitle_window(&mut self) {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(hw) = self.find(WinId::Subtitle) else {
            return;
        };
        let window = hw.window.clone();
        let visible = *self.visible.get(&WinId::Subtitle).unwrap_or(&false);
        if !visible {
            return;
        }
        // D-37：拖动进行中恒非穿透（光标会随时进入正文区甚至窗外——分区判定
        // 会把 TRANSPARENT 重新挂上，打断 SetCapture 供给的输入链；豁免直到
        // SubtitleDragEnd 收尾）
        if self.app_state.subtitle.state.dragging {
            Self::set_window_transparent(&window, false);
            return;
        }
        let enabled = self.app_state.settings.subtitle_mode.click_through;
        let mut zone = SubtitleZone::Outside;
        let mut ctrl = false;
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_ok() {
            if let Ok(pos) = window.outer_position() {
                let scale = window.scale_factor();
                let logical = window.inner_size().to_logical::<f32>(window.scale_factor());
                zone = zone_for_cursor(
                    ((pt.x as f64 - pos.x as f64) / scale) as f32,
                    ((pt.y as f64 - pos.y as f64) / scale) as f32,
                    logical.width,
                    logical.height,
                    crate::windows::subtitle::STRIP_H,
                );
            }
            // VK_CONTROL=17：Ctrl 临时恢复穿透（全区域可交互，support drag during CT）
            ctrl = unsafe { (GetAsyncKeyState(i32::from(VK_CONTROL.0)) as u16) & 0x8000 != 0 };
        }
        Self::set_window_transparent(&window, transparent_desired(enabled, zone, ctrl));
        // 顶条悬停工具条推进（状态变化才重绘；动画期由渲染帧 request_repaint 跟帧）
        if self
            .app_state
            .subtitle
            .state
            .set_toolbar_hover(zone != SubtitleZone::Outside, std::time::Instant::now())
        {
            self.redraw(WinId::Subtitle);
        }
    }

    /// 非 Windows 兜底（无穿透能力）
    #[cfg(not(windows))]
    fn poll_click_through(&mut self) {}

    /// 非 Windows 兜底（无穿透能力）
    #[cfg(not(windows))]
    fn poll_subtitle_window(&mut self) {}

    /// WS_EX_TRANSPARENT 位切换（悬浮窗/字幕窗共用的通用窗口层函数）。
    /// 窗口已挂 WS_EX_LAYERED（apply_layered），这里只增删 TRANSPARENT 位，
    /// 两者可共存（原版 QWidget.setWindowOpacity + click-through 亦同）。
    #[cfg(windows)]
    fn set_window_transparent(window: &Window, enable: bool) {
        use ::windows::Win32::Foundation::HWND;
        use ::windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_TRANSPARENT,
        };
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
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

    /// 字幕窗拖动每帧推进（D-37）：光标绝对跟踪 set_outer_position——
    /// 原版 mouseMoveEvent `move(globalPos - _drag_pos)` 语义；SetCapture 下
    /// 光标出窗后输入仍达，拖动不中断。
    #[cfg(windows)]
    fn update_subtitle_drag(&mut self) {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(hw) = self.find(WinId::Subtitle) else {
            return;
        };
        let Some((gx, gy)) = self.app_state.subtitle.state.drag_grab else {
            return;
        };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        hw.window
            .set_outer_position(winit::dpi::PhysicalPosition::new(
                (pt.x - gx) as f64,
                (pt.y - gy) as f64,
            ));
    }

    /// 悬浮窗拖动每帧推进（W5/D-71）：光标绝对跟踪 set_outer_position——
    /// 原版 mouseMoveEvent `move(globalPos - _drag_pos)` 语义；SetCapture 下
    /// 光标出窗后输入仍达，拖动不中断（与字幕窗 D-37 同法）。
    #[cfg(windows)]
    fn update_overlay_drag(&mut self) {
        use ::windows::Win32::Foundation::POINT;
        use ::windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let Some(hw) = self.find(WinId::Overlay) else {
            return;
        };
        let Some((gx, gy)) = self.app_state.overlay.state.drag_grab else {
            return;
        };
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        hw.window
            .set_outer_position(winit::dpi::PhysicalPosition::new(
                (pt.x - gx) as f64,
                (pt.y - gy) as f64,
            ));
    }

    /// 非 Windows 兜底（无 Win32 捕获；拖动不移动，保持既有无拖动行为）
    #[cfg(not(windows))]
    fn update_subtitle_drag(&mut self) {}

    /// 非 Windows 兜底（无 Win32 捕获）
    #[cfg(not(windows))]
    fn update_overlay_drag(&mut self) {}

    /// 导出请求（W5/R19）：空检查后发 `Cmd::PickExportFile` ——rfd 保存框
    /// 移出事件循环线程（编排域 Supervisor 一次性线程弹框；原版 export_messages
    /// 的写文件段随 `UiEvent::ExportSave` 回执执行）
    fn run_export(&mut self, mode: lt_proto::ExportFileMode) {
        if self.app_state.overlay.messages.is_empty() {
            // P1-4：空导出就地提示（原版仅日志；用户点了按钮必须看到反馈）。
            // D-33/H-5：改原生通知（原位 rfd 同步框阻塞事件循环线程）。
            if let Err(e) = crate::notifications::show(
                &lt_i18n::t("export_dialog_title"),
                &lt_i18n::t("export_empty"),
            ) {
                tracing::warn!("空导出提示（原生通知）失败: {e}");
            }
            tracing::info!("{}", lt_i18n::t("export_empty"));
            return;
        }
        let default_name = format!(
            "livetrans_{}_{}.txt",
            chrono::Local::now().format("%Y%m%d_%H%M%S"),
            mode.suffix()
        );
        self.app_state
            .session
            .send_cmd(lt_proto::Cmd::PickExportFile {
                mode,
                default_name,
                dialog_title: lt_i18n::t("export_dialog_title"),
            });
    }

    /// 导出写文件（`UiEvent::ExportSave` 回执后执行；三种模式行格式不变）
    fn write_export_file(&self, path: &str, mode: lt_proto::ExportFileMode) {
        let mut lines = Vec::new();
        for msg in &self.app_state.overlay.messages {
            let ts = &msg.timestamp;
            let orig = msg.original.trim();
            let trans = msg.translation.text().trim();
            match mode {
                lt_proto::ExportFileMode::Original => lines.push(format!("[{ts}] {orig}")),
                lt_proto::ExportFileMode::Translation => {
                    if !trans.is_empty() {
                        lines.push(format!("[{ts}] {trans}"));
                    }
                }
                lt_proto::ExportFileMode::All => {
                    lines.push(format!("[{ts}] {orig}"));
                    if !trans.is_empty() {
                        lines.push(format!("  -> {trans}"));
                    }
                    lines.push(String::new());
                }
            }
        }
        let body = lines
            .join(
                "
",
            )
            .trim_end()
            .to_string()
            + "
";
        if let Err(e) = std::fs::write(path, body) {
            tracing::error!("{}: {e}", lt_i18n::t("export_failed"));
        } else {
            tracing::info!("导出完成: {path}");
        }
    }

    /// 启动即安排节拍（由 lt-app 在 run 前调用）：
    /// 悬浮窗监视节拍（overlay 行为不变）+ 启动流节拍（向导倒计时/收尾延迟）
    /// + 字幕窗 100ms 光标感知轮询（D-36：可见即续，穿透开关不再决定链的存续）
    pub fn kick_ticks(&mut self) {
        self.app_state.session.schedule_tick(
            WinId::Overlay,
            TickKind::Monitor,
            Instant::now() + Duration::from_secs(1),
        );
        self.app_state
            .startup
            .kick_tick(&mut self.app_state.session);
        if self.app_state.overlay.ov_click_through {
            self.app_state
                .overlay
                .schedule_click_through_tick(&mut self.app_state.session);
        }
        if *self.visible.get(&WinId::Subtitle).unwrap_or(&false) {
            self.app_state
                .subtitle
                .schedule_window_poll(&mut self.app_state.session);
        }
    }

    /// 音频监视 33ms 节拍排班（W2/D-67：快照格轮询——悬浮窗可见时持续续拍）
    fn schedule_audio_monitor_tick(&mut self) {
        let at = Instant::now() + Duration::from_millis(33);
        self.app_state
            .session
            .schedule_tick(WinId::Overlay, TickKind::AudioMonitor, at);
    }

    /// 1s 节流的系统采样（进程 CPU/RSS，对照原版 psutil.Process）；
    /// 在监视节拍触发时调用。CPU 占用需两次采样才有意义，首次为 0 属预期。
    fn sample_system(&mut self) {
        let now = Instant::now();
        if !self
            .sys_last
            .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_secs(1))
        {
            return;
        }
        self.sys_last = Some(now);
        let sys = self.sys.get_or_insert_with(sysinfo::System::new);
        let pid = sysinfo::Pid::from_u32(std::process::id());
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        let m = &mut self.app_state.overlay.monitor;
        if let Some(proc) = sys.process(pid) {
            m.cpu = proc.cpu_usage();
            m.ram_mb = proc.memory() as f32 / 1024.0 / 1024.0;
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
                    // R7（架构 2.0 W1）：与字幕窗顶条隐藏/穿透路径同款登记防抖
                    // 脏标记，enabled=false 才会落盘（否则重启后字幕窗复活）
                    crate::windows::panel::mark_settings_dirty(&mut self.app_state.session);
                }
                // D-87：确认窗 Alt+F4/系统关闭 = 取消确认（清状态，可再请求）
                if id == WinId::Confirm {
                    self.app_state.modal.take_confirm();
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
            // D-87 确认窗恒顶：topmost 带内点击激活语义会把被点的置顶窗
            // （悬浮窗/字幕窗）翻到确认窗之上，模态被盖住不可点——这里在
            // 不抢焦点的前提下把确认窗抬回带内最顶（原生「属主可点、模态恒顶」
            // 语义；重复点击已激活窗不再触发 Focused，Z 序不动，无抖动战）
            WindowEvent::Focused(true) if matches!(id, WinId::Overlay | WinId::Subtitle) => {
                self.ensure_confirm_on_top();
            }
            WindowEvent::Moved(_) => {
                match id {
                    WinId::Overlay => {
                        // 拖动/移动结束防抖保存（原版 moveEvent → _schedule_pos_save）
                        let geo = self.overlay_geo();
                        self.app_state
                            .overlay
                            .schedule_pos_save(&mut self.app_state.session, geo);
                    }
                    WinId::Subtitle => {
                        // 字幕窗移动防抖保存（原版 mouseReleaseEvent → position_changed）
                        let pos = self.subtitle_pos();
                        self.app_state
                            .subtitle
                            .schedule_pos_save(&mut self.app_state.session, pos);
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
        if self.app_state.modal.quit_requested {
            tracing::info!("退出请求（启动流对话框关闭）");
            event_loop.exit();
            return;
        }
        // 0.5) W5：设置变更意图消费（跨域请求 → 面板 300ms 防抖登记）——
        // 字幕窗/确认模态/面板页的 mark_settings_dirty 均收敛于此（原版
        // _auto_save 的宿主侧落点；登记后 PanelApply 节拍到期整体重放）
        if self.app_state.session.take_settings_apply_pending() {
            crate::state::register_panel_apply(
                &mut self.app_state.panel,
                &mut self.app_state.session,
                Instant::now(),
            );
        }
        // 1) 到期节拍按 kind 分派（同窗口可并存多种节拍）
        for tick in self.app_state.session.drain_due_ticks() {
            match tick.kind {
                TickKind::Monitor => {
                    self.sample_system();
                    self.app_state.session.schedule_tick(
                        WinId::Overlay,
                        TickKind::Monitor,
                        Instant::now() + Duration::from_secs(1),
                    );
                    self.redraw(WinId::Overlay);
                }
                // W2/D-67：音频监视快照格（~33ms 读格，seq 变了才重绘；
                // 悬浮窗不可见即停排班——无事件驱动时的唯一数据通路）
                TickKind::AudioMonitor => {
                    if let Some(cell) = &self.monitor_cell {
                        let snap = cell.load_full();
                        if snap.seq != self.monitor_seq {
                            self.monitor_seq = snap.seq;
                            let m = &mut self.app_state.overlay.monitor;
                            m.rms = snap.rms;
                            m.vad = snap.vad;
                            m.mic_rms = snap.mic_rms;
                            self.redraw(WinId::Overlay);
                        }
                    }
                    if self.visible.get(&WinId::Overlay).copied().unwrap_or(false) {
                        self.schedule_audio_monitor_tick();
                    }
                }
                TickKind::StreamFlush => {
                    self.app_state.overlay.flush_streams();
                    self.redraw(WinId::Overlay);
                }
                TickKind::PosSave => match tick.win {
                    WinId::Subtitle => self.on_subtitle_pos_save_tick(),
                    _ => self.on_pos_save_tick(),
                },
                TickKind::ClickThrough => match tick.win {
                    WinId::Subtitle => {
                        // 字幕窗 100ms 分区穿透 + 顶条悬停轮询（D-36：可见即续拍，
                        // 穿透开关不再决定链的存续——悬停工具条在非穿透态也需要）
                        self.poll_subtitle_window();
                        if *self.visible.get(&WinId::Subtitle).unwrap_or(&false) {
                            self.app_state
                                .subtitle
                                .schedule_window_poll(&mut self.app_state.session);
                        }
                    }
                    _ => {
                        self.poll_click_through();
                        if self.app_state.overlay.ov_click_through {
                            self.app_state
                                .overlay
                                .schedule_click_through_tick(&mut self.app_state.session);
                        }
                    }
                },
                // 字幕窗自动隐藏到点（原版 _auto_hide_timer.timeout → _on_auto_hide_timeout）
                TickKind::SubtitleAutoHide => {
                    let anim = self
                        .app_state
                        .settings
                        .subtitle_mode
                        .auto_hide_animation
                        .clone();
                    let dur = self.app_state.settings.subtitle_mode.auto_hide_duration;
                    if self.app_state.subtitle.state.on_auto_hide_timeout(
                        &anim,
                        dur,
                        Instant::now(),
                    ) {
                        self.redraw(WinId::Subtitle);
                    }
                }
                // 字幕窗排队句子到点（原版 _pending_segment_timers 的 singleShot 到期）
                TickKind::SubtitlePending => {
                    if self
                        .app_state
                        .subtitle
                        .flush_pending(&mut self.app_state.session, &self.app_state.settings)
                    {
                        self.redraw(WinId::Subtitle);
                    }
                }
                TickKind::Setup => self.on_setup_tick(),
                // 面板设置 300ms 防抖到期（原版 _save_timer.timeout → _apply_settings：
                // 整体重放 ApplySettings，AppShell 负责热应用 + 落盘）
                TickKind::PanelApply => self.on_panel_apply_tick(),
                // 连接测试 100ms 走秒/看门狗（D-85）：在途则续拍 + 重绘，
                // 收敛即停（不留空转节拍）
                TickKind::ProbeTick => self.on_probe_tick(),
                // 翻译页 prompt 600ms 防抖到期（原版 _prompt_debounce → _apply_prompt：
                // system_prompt 已实时写入 settings，此处重建活动模型翻译器）
                TickKind::PromptApply => {
                    if self
                        .app_state
                        .panel
                        .state
                        .take_prompt_apply_due(Instant::now())
                    {
                        let s = &self.app_state;
                        if let Some(cfg) = s.settings.models.get(s.settings.active_model).cloned() {
                            tracing::info!("System prompt updated（600ms 防抖到期，重建翻译器）");
                            self.app_state
                                .session
                                .send_cmd(lt_proto::Cmd::SwitchTranslator(Box::new(cfg)));
                        }
                    }
                }
            }
        }
        // 2) 空闲策略：等待最近节拍或事件
        let deadline = self
            .app_state
            .session
            .next_tick()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

/// 下载进度的人读行（W2：与旧 backend format_event 人读段同格式——
/// `[{repo}] {file} {done} / {total}`，人性化单位含 B/KB/MB/GB）
fn format_download_line(ev: &lt_proto::DownloadEvent) -> String {
    match ev.total {
        Some(t) => format!(
            "[{}] {} {} / {}",
            ev.repo,
            ev.file,
            format_size(ev.done),
            format_size(t)
        ),
        None => format!("[{}] {} {}", ev.repo, ev.file, format_size(ev.done)),
    }
}

/// 字节量人性化（阈值与精度对齐原版 model_manager.format_size；
/// backend 侧同实现——UI 只读事件字段不再依赖生产端拼串）
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

/// 窗口清屏色。
/// 悬浮窗/字幕窗/确认窗：不透明黑——圆角外像素已被 SetWindowRgn 从窗口形状
/// 切除（见 apply_window_region），残余清除像素只在圆角 1px 抗锯齿带内与背景色
/// 混合（同为深色，无可察差异）。整窗半透明由 LWA_ALPHA 承担。
/// 其余普通窗：深灰不透明。
fn clear_color_for(id: WinId) -> [f32; 4] {
    match id {
        WinId::Overlay | WinId::Subtitle | WinId::Confirm => [0.0, 0.0, 0.0, 1.0],
        _ => [0.08, 0.08, 0.10, 1.0],
    }
}

/// 确认窗整窗不透明度（LWA_ALPHA，250/255 ≈ 98%——走查反馈壁纸透出过强，
/// 收到近实：玻璃感由顶缘高光与描边承担，不给文字添噪）
#[cfg(windows)]
const CONFIRM_LAYER_ALPHA: u8 = 250;
/// 确认窗圆角（逻辑 px，u32 对齐 style.border_radius；egui 侧画角与
/// SetWindowRgn 裁区同源此值——confirm.rs RADIUS = 12.0）
const CONFIRM_CORNER_RADIUS: u32 = 12;

/// 悬浮窗整窗不透明度（0-255）。原版是两级叠加：QSS rgba(bg_opacity/255) ×
/// setWindowOpacity(window_opacity%)；LWA_ALPHA 只有一层，取乘积为单值——
/// 背景精确等价，文本/控件随之乘同系数（原版文本仅乘 window_opacity，
/// 默认 95% 下相差 ≤5.6pp，实机不可辨；transparent 预设文本偏淡为已知偏差）。
#[cfg(windows)]
fn overlay_layered_alpha(s: &Settings) -> u8 {
    let st = &s.style;
    ((st.window_opacity.min(100) * st.bg_opacity.min(255)) / 100) as u8
}

/// 字幕窗整窗不透明度：原版无 setWindowOpacity，仅背景 QSS alpha。
/// bg_opacity=0 全透明模式退化为不透明底+文字（键控仍镂空圆角空区，已知偏差）。
#[cfg(windows)]
fn subtitle_layered_alpha(s: &Settings) -> u8 {
    let sm = &s.subtitle_mode;
    if sm.bg_opacity == 0 {
        255
    } else {
        sm.bg_opacity.min(255) as u8
    }
}

/// WS_EX_LAYERED 实测（D-34：winit apply_diff 可能整体重写 EXSTYLE 清掉该位；
/// 读回实测而非信任缓存，缺位即由 run_frame 重挂）
#[cfg(windows)]
fn window_has_layered(window: &Window) -> bool {
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::UI::WindowsAndMessaging::{GetWindowLongW, GWL_EXSTYLE, WS_EX_LAYERED};
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return false;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    unsafe { (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_LAYERED.0 != 0 }
}

/// WS_EX_LAYERED + SetLayeredWindowAttributes：整窗半透明（等价 Qt 整窗不透明度，
/// 即原版 setWindowOpacity）。圆角镂空不走 LWA_COLORKEY——DWM 对 alpha 合成有
/// ±1 抖动/舍入，精确色键不可靠（实测键控色需 3 通道同时精确命中），改由
/// SetWindowRgn 几何镂空（见 apply_window_region）。surface 创建前调用（呈现稳定），
/// 设置变更时随帧刷新。
#[cfg(windows)]
fn apply_layered(window: &Window, alpha: u8) {
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA,
        WS_EX_LAYERED,
    };
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
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
    use ::windows::Win32::Foundation::HWND;
    use ::windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = HWND(win32.hwnd.get() as *mut core::ffi::c_void);
    let diam = (radius_px * 2).max(2);
    let rgn = unsafe {
        CreateRoundRectRgn(
            0,
            0,
            (w + 1) as i32,
            (h + 1) as i32,
            diam as i32,
            diam as i32,
        )
    };
    if !rgn.is_invalid() {
        // SetWindowRgn 成功后系统接管区域句柄（不得再 DeleteObject）
        let _ = unsafe { SetWindowRgn(hwnd, Some(rgn), true) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// W2：Download 事件的人读行与旧 format_event 人读段同格式
    ///（`[{repo}] {file} {done} / {total}`，人性化单位）；total=None 走单值
    #[test]
    fn download_event_line_readable() {
        let ev = lt_proto::DownloadEvent {
            repo: "a/b".into(),
            file: "m.onnx".into(),
            index: 1,
            count: 2,
            done: 1024,
            total: Some(2048),
            phase: lt_proto::DownloadPhase::Progress,
        };
        assert_eq!(format_download_line(&ev), "[a/b] m.onnx 1.0 KB / 2.0 KB");
        let ev_none = lt_proto::DownloadEvent { total: None, ..ev };
        assert_eq!(format_download_line(&ev_none), "[a/b] m.onnx 1.0 KB");
    }

    /// 字节人性化（阈值与精度对齐原版 model_manager.format_size）
    #[test]
    fn size_format_matches_original() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(250_000_000), "238.4 MB");
        assert_eq!(format_size(3_100_000_000), "2.89 GB");
    }
}
