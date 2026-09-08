//! 托盘（M0.5）：完整菜单树 + 运行时绘制图标 + 事件转发进 winit 循环。
//! 菜单结构逐项对照 docs/archive/rewrite-research.md §1.3（原版 main.py 托盘段）。
//!
//! 线程模型（D-35，docs/tray-menu-blocking-fix.md）：托盘图标/菜单/文字更新全部
//! 运行在专用线程 `lt-tray`（自带 Win32 消息泵）。根因 = tray-icon 0.24 在托盘窗
//! WndProc 内直调 `TrackPopupMenu`（platform_impl/windows/mod.rs:542-558）跑嵌套
//! 模态循环——托盘建于 winit 事件循环线程时点开菜单即挂死 winit 主循环
//! （egui 不出帧/不收货）。主线程仅持通道句柄（`set_*` 非阻塞、线程安全），
//! 菜单/托盘事件仍经 proxy（EventLoopProxy::send_event，线程安全）回流 winit 循环。

use anyhow::Context;
use lt_proto::UiMsg;
use muda::{accelerator::Accelerator, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// 唤醒消息（hwnd=NULL 线程消息；取值 WM_APP+1=0x8001 属用户自定义区，
/// 与托盘窗 WM_USER 6002-6009 互不冲突）
const TRAY_WAKE: u32 = 0x8001;

/// 托盘状态图标（原版 icons.create_app_icon 的三态：run 绿点/pause 琥珀/error 红）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconStatus {
    /// 应用默认图标（app.png）
    App,
    Run,
    Pause,
    Error,
}

/// 菜单项 id（menu event 依据；保持稳定字符串）。
/// 对齐新版原版 app_shell.build_tray_shell 的最小菜单：
/// 状态行 / 暂停恢复 / 悬浮窗显隐 / 控制面板 / 退出。
/// 模型与语言切换收敛在悬浮窗下拉（新版明确"tray menu stays minimal"）。
pub mod ids {
    pub const STATUS: &str = "tray_status";
    pub const PAUSE: &str = "tray_pause";
    pub const OVERLAY_TOGGLE: &str = "tray_hide_overlay";
    pub const SHOW_PANEL: &str = "tray_show_panel";
    pub const QUIT: &str = "quit";
}

/// 主线程 → 托盘线程命令（全部在托盘线程执行，Win32 对象线程亲和正确）
enum TrayCmd {
    SetStatus(IconStatus),
    SetStatusLine(String),
    SetPauseLabel(String),
    SetOverlayToggleLabel(String),
    Quit,
}

/// 托盘通道句柄（可在任意线程持有；内部 = mpsc send + PostThreadMessageW 唤醒，
/// 均非阻塞——调用方永远不会因为托盘被挂起）。
pub struct Tray {
    tx: std::sync::mpsc::Sender<TrayCmd>,
    /// 托盘线程 Win32 线程 id（唤醒投递目标）
    tid: u32,
    /// 托盘线程（drop 时经 ack 限时等待收尾）
    thread: Option<std::thread::JoinHandle<()>>,
    /// 托盘线程退出 ack（drop 限时等待；超时则分离，进程退出 OS 回收）
    quit_ack: std::sync::mpsc::Receiver<()>,
}

impl Tray {
    fn send(&self, cmd: TrayCmd) {
        let _ = self.tx.send(cmd);
        #[cfg(windows)]
        unsafe {
            let _ = ::windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.tid,
                TRAY_WAKE,
                ::windows::Win32::Foundation::WPARAM(0),
                ::windows::Win32::Foundation::LPARAM(0),
            );
        }
    }

    /// 切换托盘状态图标（原版 tray.setIcon(create_app_icon(status))）
    pub fn set_status(&self, status: IconStatus) {
        self.send(TrayCmd::SetStatus(status));
    }

    /// 刷新托盘状态行（● 状态 · 引擎 · 模型 · 源 → 目标）
    pub fn set_status_line(&self, text: impl Into<String>) {
        self.send(TrayCmd::SetStatusLine(text.into()));
    }

    /// 暂停/恢复菜单文字（随运行态切换）
    pub fn set_pause_label(&self, text: impl Into<String>) {
        self.send(TrayCmd::SetPauseLabel(text.into()));
    }

    /// 悬浮窗 显示/隐藏 菜单文字（随可见性切换）
    pub fn set_overlay_toggle_label(&self, text: impl Into<String>) {
        self.send(TrayCmd::SetOverlayToggleLabel(text.into()));
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let _ = self.tx.send(TrayCmd::Quit);
        #[cfg(windows)]
        unsafe {
            let _ = ::windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                self.tid,
                TRAY_WAKE,
                ::windows::Win32::Foundation::WPARAM(0),
                ::windows::Win32::Foundation::LPARAM(0),
            );
        }
        if let Some(handle) = self.thread.take() {
            // 正常路径：托盘线程在泵的排空点收到 Quit 即刻退出（≤2s）；
            // 异常路径（TrackPopupMenu 模态循环滞留吞掉唤醒）：超时分离，
            // 进程退出时 OS 回收，绝不阻塞主线程收尾。不用
            // PostThreadMessageW(WM_QUIT)——它会先被嵌套模态循环消费、
            // 外层泵随后永挂（Deadlock 风险）。
            if self
                .quit_ack
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok()
            {
                let _ = handle.join();
            }
        }
    }
}

// 退出确认已迁出本模块（D-33/H-3：egui 内嵌模态，见 windows::confirm；
// 注入 rfd 同步框在事件循环线程内阻塞、且可被其他模态串行/不可达）。

/// 构建完整托盘（含子菜单），并启动专用线程承载图标/菜单/消息泵。
/// 托盘构建失败不阻断应用（记日志、线程退出）；事件经 proxy 回流 winit 循环。
pub fn build(proxy: std::sync::Arc<dyn Fn(UiMsg) + Send + Sync>) -> anyhow::Result<Tray> {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TrayCmd>();
    let (tid_tx, tid_rx) = std::sync::mpsc::channel::<u32>();
    let (quit_tx, quit_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::Builder::new()
        .name("lt-tray".to_string())
        .spawn(move || tray_thread(proxy, cmd_rx, tid_tx, quit_tx))
        .context("托盘线程启动失败")?;
    // 线程先建消息队列再报到（见 tray_thread 注释），此处无唤醒竞态
    let tid = tid_rx.recv().context("托盘线程未报到")?;
    Ok(Tray {
        tx: cmd_tx,
        tid,
        thread: Some(handle),
        quit_ack: quit_rx,
    })
}

/// 当前线程 Win32 id（托盘线程注册后供 PostThreadMessageW 唤醒）
#[cfg(windows)]
fn current_thread_id() -> u32 {
    unsafe { ::windows::Win32::System::Threading::GetCurrentThreadId() }
}

#[cfg(not(windows))]
fn current_thread_id() -> u32 {
    0
}

/// 托盘线程主体：建菜单/图标 → 消息泵 → 退出时 drop TrayIcon（NIM_DELETE 在本线程）
fn tray_thread(
    proxy: std::sync::Arc<dyn Fn(UiMsg) + Send + Sync>,
    rx: std::sync::mpsc::Receiver<TrayCmd>,
    tid_tx: std::sync::mpsc::Sender<u32>,
    quit_tx: std::sync::mpsc::Sender<()>,
) {
    let tid = current_thread_id();
    #[cfg(windows)]
    unsafe {
        // PostThreadMessageW 要求目标线程已有消息队列——先 Peek 一次确保队列
        // 存在，再上报 tid，杜绝 build 返回后首个命令唤醒被丢的竞态
        let mut msg = std::mem::zeroed::<::windows::Win32::UI::WindowsAndMessaging::MSG>();
        let _ = ::windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut msg,
            None,
            0,
            0,
            ::windows::Win32::UI::WindowsAndMessaging::PM_NOREMOVE,
        );
    }
    let _ = tid_tx.send(tid);

    let inner = match build_inner(&proxy) {
        Ok(inner) => inner,
        Err(e) => {
            tracing::error!("托盘构建失败: {e:#}");
            let _ = quit_tx.send(());
            return;
        }
    };
    pump(&inner, &rx);
    drop(inner); // TrayIcon drop → Shell_NotifyIcon(NIM_DELETE) 在托盘线程执行
    let _ = quit_tx.send(());
}

/// 执行一条命令；返回是否收到退出
fn apply_cmd(inner: &TrayInner, cmd: TrayCmd) -> bool {
    match cmd {
        TrayCmd::SetStatus(s) => {
            let _ = inner.icon.set_icon(Some(icon_for(s)));
        }
        TrayCmd::SetStatusLine(text) => {
            let _ = inner.status.set_text(text);
        }
        TrayCmd::SetPauseLabel(text) => {
            let _ = inner.pause.set_text(text);
        }
        TrayCmd::SetOverlayToggleLabel(text) => {
            let _ = inner.overlay_toggle.set_text(text);
        }
        TrayCmd::Quit => return true,
    }
    false
}

#[cfg(windows)]
fn pump(inner: &TrayInner, rx: &std::sync::mpsc::Receiver<TrayCmd>) {
    use ::windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage,
    };
    unsafe {
        let mut msg = std::mem::zeroed::<::windows::Win32::UI::WindowsAndMessaging::MSG>();
        loop {
            // 先排空命令再等消息：唤醒消息可能被 TrackPopupMenu 嵌套模态循环
            // 吞掉，菜单关闭回到本泵后立即补处理（状态更新不丢失）
            while let Ok(cmd) = rx.try_recv() {
                if apply_cmd(inner, cmd) {
                    return;
                }
            }
            let ret = GetMessageW(&mut msg, None, 0, 0).0;
            if ret <= 0 {
                // 0 = WM_QUIT；-1 = 取消息失败 —— 均收尾
                if ret < 0 {
                    tracing::error!("托盘消息泵 GetMessageW 失败");
                }
                return;
            }
            if msg.message == TRAY_WAKE {
                continue; // 唤醒拍：命令在循环顶排空（可能恰被模态循环吞掉）
            }
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
}

#[cfg(not(windows))]
fn pump(inner: &TrayInner, rx: &std::sync::mpsc::Receiver<TrayCmd>) {
    while let Ok(cmd) = rx.recv() {
        if apply_cmd(inner, cmd) {
            return;
        }
    }
}

/// 托盘线程侧句柄（菜单/图标均为本线程创建、本线程使用）
struct TrayInner {
    icon: TrayIcon,
    /// 状态行（禁用项，● 运行 · 引擎 · 模型 · 源 → 目标）
    status: MenuItem,
    /// 暂停/恢复（文字随状态切换）
    pause: MenuItem,
    /// 悬浮窗 显示/隐藏（文字随可见性切换）
    overlay_toggle: MenuItem,
}

/// 构建菜单 + 图标（在托盘线程执行；失败仅记日志，托盘不阻断应用）
fn build_inner(proxy: &std::sync::Arc<dyn Fn(UiMsg) + Send + Sync>) -> anyhow::Result<TrayInner> {
    let menu = Menu::new();

    // ── 状态行（只读；文字随运行态/引擎/模型/语言刷新）──
    let status = MenuItem::with_id(MenuId::new(ids::STATUS), "● --", false, None::<Accelerator>);
    menu.append(&status)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 暂停/恢复（文字随状态切换）──
    let pause = MenuItem::with_id(
        MenuId::new(ids::PAUSE),
        lt_i18n::t("tray_pause"),
        true,
        None::<Accelerator>,
    );
    menu.append(&pause)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 悬浮窗显隐（文字随可见性切换；首次隐藏弹气泡，原版 _hide_notified）──
    let overlay_toggle = MenuItem::with_id(
        MenuId::new(ids::OVERLAY_TOGGLE),
        lt_i18n::t("tray_hide_overlay"),
        true,
        None::<Accelerator>,
    );
    menu.append(&overlay_toggle)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 控制面板 ──
    let panel = MenuItem::with_id(
        MenuId::new(ids::SHOW_PANEL),
        lt_i18n::t("tray_show_panel"),
        true,
        None::<Accelerator>,
    );
    menu.append(&panel)?;

    // ── 退出（宿主侧带确认框，原版 on_quit(confirm=True)）──
    menu.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::with_id(
        MenuId::new(ids::QUIT),
        lt_i18n::t("quit"),
        true,
        None::<Accelerator>,
    );
    menu.append(&quit)?;

    // 事件转发（Windows：托盘与菜单事件在托盘线程触发，经 EventLoopProxy 桥接
    // 回 winit 循环，与 D-33 同构；EventLoopProxy::send_event 线程安全）
    let proxy_menu = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |ev: muda::MenuEvent| {
        proxy_menu(UiMsg::Menu(ev.id.0.to_string()));
    }));
    let proxy_tray = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |_ev: TrayIconEvent| {
        // 原版无左键行为
        let _ = &proxy_tray;
    }));

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(lt_i18n::t("tray_tooltip"))
        .with_icon(icon_for(IconStatus::App))
        .build()?;

    Ok(TrayInner {
        icon,
        status,
        pause,
        overlay_toggle,
    })
}

/// 解码内嵌图标 PNG → tray_icon::Icon（素材缺失时回退运行时绘制）
fn icon_for(status: IconStatus) -> Icon {
    let (bytes, label): (&[u8], &str) = match status {
        IconStatus::App => (
            include_bytes!("../../../assets/icons/app.png").as_slice(),
            "app.png",
        ),
        IconStatus::Run => (
            include_bytes!("../../../assets/icons/tray_run_64.png").as_slice(),
            "tray_run_64.png",
        ),
        IconStatus::Pause => (
            include_bytes!("../../../assets/icons/tray_pause_64.png").as_slice(),
            "tray_pause_64.png",
        ),
        IconStatus::Error => (
            include_bytes!("../../../assets/icons/tray_error_64.png").as_slice(),
            "tray_error_64.png",
        ),
    };
    match decode_png(bytes) {
        Some(icon) => icon,
        None => {
            tracing::warn!("图标解码失败（{label}），回退运行时绘制");
            drawn_icon()
        }
    }
}

/// PNG → RGBA → Icon（image crate 仅开 png feature）
fn decode_png(bytes: &[u8]) -> Option<Icon> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// 窗口图标（winit；app_32.png，面板/日志等常规窗口标题栏用）
pub fn window_icon() -> Option<winit::window::Icon> {
    let img = image::load_from_memory(include_bytes!("../../../assets/icons/app_32.png"))
        .ok()?
        .to_rgba8();
    let (w, h) = img.dimensions();
    winit::window::Icon::from_rgba(img.into_raw(), w, h).ok()
}

/// 运行时绘制托盘图标：64×64 蓝底圆角 + 白色 "LT"（矩形拼字，无字体依赖）。
/// 素材解码失败时的兜底（原版 _drawn_icon 的等价物）。
fn drawn_icon() -> Icon {
    const W: usize = 64;
    const H: usize = 64;
    let mut rgba = vec![0u8; W * H * 4];
    let bg = [60, 130, 240, 255];
    let fg = [255, 255, 255, 255];
    let (x0, y0, x1, y1, r): (i32, i32, i32, i32, i32) = (4, 4, 60, 60, 12);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            // 圆角判定：位于四角半径 r 的方角内的像素按圆方程剔除
            let cx = x.clamp(x0 + r, x1 - 1 - r);
            let cy = y.clamp(y0 + r, y1 - 1 - r);
            let dx = (x - cx).abs();
            let dy = (y - cy).abs();
            let rounded_ok = !(dx == r && dy == r) || (dx * dx + dy * dy) <= r * r;
            let inside = x >= x0 && x < x1 && y >= y0 && y < y1 && rounded_ok;
            if inside {
                let i = ((y as usize) * W + x as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&bg);
            }
        }
    }
    // "L"（x16-28）与 "T"（x36-52），字面矩形拼绘
    let mut put = |x0: usize, y0: usize, x1: usize, y1: usize| {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * W + x) * 4;
                rgba[i..i + 4].copy_from_slice(&fg);
            }
        }
    };
    put(16, 14, 23, 52); // L 竖
    put(16, 45, 31, 52); // L 横
    put(33, 14, 52, 21); // T 横
    put(39, 14, 46, 52); // T 竖
    Icon::from_rgba(rgba, W as u32, H as u32).expect("图标像素尺寸合法")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归（D-35）：托盘线程握手/命令非阻塞投递/退出收尾。
    /// 「打开菜单卡死」的根因是 TrackPopupMenu 模态循环占死调用线程——本测试
    /// 证明 set_* 全为通道投递（非阻塞），drop 后托盘线程限时收尾（ack+join）。
    #[test]
    fn tray_thread_lifecycle() {
        let msgs = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cnt = msgs.clone();
        let proxy: std::sync::Arc<dyn Fn(UiMsg) + Send + Sync> = std::sync::Arc::new(move |_| {
            cnt.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let tray = build(proxy).expect("托盘应能构建（专用线程）");
        // 命令投递 + 唤醒必须立即返回；若主线程被模态阻塞会先暴露为超时
        let t0 = std::time::Instant::now();
        tray.set_status(IconStatus::Run);
        tray.set_status_line("● 运行 · sensevoice · tiny · zh → en");
        tray.set_pause_label("暂停");
        tray.set_overlay_toggle_label("隐藏悬浮窗");
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(500),
            "set_* 必须非阻塞（通道投递）"
        );
        drop(tray); // Quit + 唤醒 → 线程排空点退出 → ack + join
    }
}
