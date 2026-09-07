//! 托盘（M0.5）：完整菜单树 + 运行时绘制图标 + 事件转发进 winit 循环。
//! 菜单结构逐项对照 RESEARCH.md §1.3（原版 main.py 托盘段）。

use lt_proto::UiMsg;
use muda::{accelerator::Accelerator, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

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

/// 需要在运行期改文字的菜单项句柄（新版最小集）
pub struct TrayHandles {
    /// 状态行（禁用项，● 运行 · 引擎 · 模型 · 源 → 目标）
    pub status: MenuItem,
    /// 暂停/恢复（文字随状态切换）
    pub pause: MenuItem,
    /// 悬浮窗 显示/隐藏（文字随可见性切换）
    pub overlay_toggle: MenuItem,
}

pub struct Tray {
    pub icon: TrayIcon,
    pub handles: TrayHandles,
}

/// 统一退出确认（原版 on_quit(confirm=True)；托盘菜单与悬浮窗退出按钮共用同一
/// 语义——P1-1 修复：同一动作不再双语义）
pub fn confirm_quit() -> bool {
    rfd::MessageDialog::new()
        .set_title(lt_i18n::t("quit_confirm_title"))
        .set_description(lt_i18n::t("quit_confirm_msg"))
        .set_buttons(rfd::MessageButtons::OkCancel)
        .set_level(rfd::MessageLevel::Info)
        .show()
        == rfd::MessageDialogResult::Ok
}

impl Tray {
/// 切换托盘状态图标（原版 tray.setIcon(create_app_icon(status))）
pub fn set_status(&self, status: IconStatus) {
    let _ = self.icon.set_icon(Some(icon_for(status)));
}
}

/// 构建完整托盘（含子菜单），并把事件转发到 EventLoopProxy。
/// 必须在主线程调用（Windows 要求托盘与消息循环同线程）。
pub fn build(proxy: std::sync::Arc<dyn Fn(UiMsg) + Send + Sync>) -> anyhow::Result<Tray> {
    let menu = Menu::new();

    // ── 状态行（只读；文字随运行态/引擎/模型/语言刷新）──
    let status = MenuItem::with_id(MenuId::new(ids::STATUS), "● --", false, None::<Accelerator>);
    menu.append(&status)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 暂停/恢复（文字随状态切换）──
    let pause = MenuItem::with_id(MenuId::new(ids::PAUSE), lt_i18n::t("tray_pause"), true, None::<Accelerator>);
    menu.append(&pause)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 悬浮窗显隐（文字随可见性切换；首次隐藏弹气泡，原版 _hide_notified）──
    let overlay_toggle = MenuItem::with_id(
        MenuId::new(ids::OVERLAY_TOGGLE),
        lt_i18n::t("tray_hide_overlay"),
        true, None::<Accelerator>,
    );
    menu.append(&overlay_toggle)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 控制面板 ──
    let panel = MenuItem::with_id(MenuId::new(ids::SHOW_PANEL), lt_i18n::t("tray_show_panel"), true, None::<Accelerator>);
    menu.append(&panel)?;

    // ── 退出（宿主侧带确认框，原版 on_quit(confirm=True)）──
    menu.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::with_id(MenuId::new(ids::QUIT), lt_i18n::t("quit"), true, None::<Accelerator>);
    menu.append(&quit)?;

    // 事件转发（Windows：托盘与菜单事件不进 winit 循环，必须桥接）
    let proxy_menu = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |ev: muda::MenuEvent| {
        proxy_menu(UiMsg::Menu(ev.id.0.to_string()));
    }));
    let proxy_tray = proxy.clone();
    TrayIconEvent::set_event_handler(Some(move |_ev: TrayIconEvent| {
        // 原版无左键行为
        let _ = &proxy_tray;
    }));

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(lt_i18n::t("tray_tooltip"))
        .with_icon(icon_for(IconStatus::App))
        .build()?;

    Ok(Tray {
        icon: tray,
        handles: TrayHandles { status, pause, overlay_toggle },
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
