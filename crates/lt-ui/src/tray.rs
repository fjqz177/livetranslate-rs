//! 托盘（M0.5）：完整菜单树 + 运行时绘制图标 + 事件转发进 winit 循环。
//! 菜单结构逐项对照 RESEARCH.md §1.3（原版 main.py 托盘段）。

use lt_proto::UiMsg;
use muda::{
    accelerator::Accelerator, CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// 菜单项 id（menu event 依据；保持稳定字符串）
pub mod ids {
    pub const PAUSE: &str = "tray_pause";
    pub const OVERLAY_TOGGLE: &str = "tray_hide_overlay";
    pub const SUBWIN_TOGGLE: &str = "subwin_show";
    pub const SUBWIN_CT: &str = "subwin_click_through_tray";
    pub const SHOW_PANEL: &str = "tray_show_panel";
    pub const SHOW_LOG: &str = "tray_show_log";
    pub const OV_CT: &str = "ov_ct";
    pub const OV_TOPMOST: &str = "ov_topmost";
    pub const OV_AUTOSCROLL: &str = "ov_autoscroll";
    pub const OV_TASKBAR: &str = "ov_taskbar";
    pub const EXPORT_ORIG: &str = "export_original";
    pub const EXPORT_TRANS: &str = "export_translation";
    pub const EXPORT_ALL: &str = "export_all";
    pub const QUIT: &str = "quit";
    pub fn lang(code: &str) -> muda::MenuId { muda::MenuId::new(format!("lang_{code}")) }
    pub fn asr_lang(code: &str) -> muda::MenuId { muda::MenuId::new(format!("asrlang_{code}")) }
}

/// 需要在运行期改状态（勾选/文字）的菜单项句柄
pub struct TrayHandles {
    pub pause: MenuItem,
    pub subwin: CheckMenuItem,
    pub subwin_ct: CheckMenuItem,
    pub ov_ct: CheckMenuItem,
    pub ov_topmost: CheckMenuItem,
    pub ov_autoscroll: CheckMenuItem,
    pub ov_taskbar: CheckMenuItem,
    /// 目标语言单选组（code → 项）
    pub langs: Vec<(String, CheckMenuItem)>,
    /// 源语言单选组（code → 项）
    pub asr_langs: Vec<(String, CheckMenuItem)>,
}

pub struct Tray {
    pub icon: TrayIcon,
    pub handles: TrayHandles,
}

/// 语言条目文本："code - 原生名"（auto 用 i18n 文案）
fn lang_label(code: &str, native: Option<&str>) -> String {
    match native {
        Some(n) => format!("{code} - {n}"),
        None => format!("{code} - {}", lt_i18n::t("asr_lang_auto")),
    }
}

/// 构建完整托盘（含子菜单），并把事件转发到 EventLoopProxy。
/// 必须在主线程调用（Windows 要求托盘与消息循环同线程）。
pub fn build(proxy: std::sync::Arc<dyn Fn(UiMsg) + Send + Sync>) -> anyhow::Result<Tray> {
    let menu = Menu::new();

    // ── 暂停/恢复（文字随状态切换）──
    let pause = MenuItem::with_id(MenuId::new(ids::PAUSE), lt_i18n::t("tray_pause"), true, None::<Accelerator>);
    menu.append(&pause)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 悬浮窗 / 字幕窗 ──
    let overlay_toggle = MenuItem::with_id(
        MenuId::new(ids::OVERLAY_TOGGLE),
        lt_i18n::t("tray_hide_overlay"),
        true, None::<Accelerator>,
    );
    menu.append(&overlay_toggle)?;
    let subwin = CheckMenuItem::with_id(MenuId::new(ids::SUBWIN_TOGGLE), lt_i18n::t("subwin_show"), true, false, None::<Accelerator>);
    menu.append(&subwin)?;
    let subwin_ct = CheckMenuItem::with_id(MenuId::new(ids::SUBWIN_CT), lt_i18n::t("subwin_click_through_tray"), true, false, None::<Accelerator>);
    menu.append(&subwin_ct)?;
    menu.append(&PredefinedMenuItem::separator())?;

    // ── 面板 / 日志 ──
    let panel = MenuItem::with_id(MenuId::new(ids::SHOW_PANEL), lt_i18n::t("tray_show_panel"), true, None::<Accelerator>);
    let log = MenuItem::with_id(MenuId::new(ids::SHOW_LOG), lt_i18n::t("tray_show_log"), true, None::<Accelerator>);
    menu.append(&panel)?;
    menu.append(&log)?;

    // ── 悬浮窗子菜单（4 复选，默认：置顶√ 自动滚动√）──
    let ov_menu = Submenu::with_id(MenuId::new("m_overlay"), lt_i18n::t("tray_menu_overlay"), true);
    let ov_ct = CheckMenuItem::with_id(MenuId::new(ids::OV_CT), lt_i18n::t("click_through"), true, false, None::<Accelerator>);
    let ov_topmost = CheckMenuItem::with_id(MenuId::new(ids::OV_TOPMOST), lt_i18n::t("top_most"), true, true, None::<Accelerator>);
    let ov_autoscroll = CheckMenuItem::with_id(MenuId::new(ids::OV_AUTOSCROLL), lt_i18n::t("auto_scroll"), true, true, None::<Accelerator>);
    let ov_taskbar = CheckMenuItem::with_id(MenuId::new(ids::OV_TASKBAR), lt_i18n::t("taskbar"), true, false, None::<Accelerator>);
    ov_menu.append(&ov_ct)?;
    ov_menu.append(&ov_topmost)?;
    ov_menu.append(&ov_autoscroll)?;
    ov_menu.append(&ov_taskbar)?;
    menu.append(&ov_menu)?;

    // ── 模型子菜单（M0 占位；M2 起按 settings.models 动态重建）──
    let model_menu = Submenu::with_id(MenuId::new("m_model"), lt_i18n::t("tray_menu_model"), true);
    let placeholder = MenuItem::with_id(
        MenuId::new("model_placeholder"),
        lt_i18n::t("model_label"),
        false, None::<Accelerator>,
    );
    model_menu.append(&placeholder)?;
    menu.append(&model_menu)?;

    // ── 目标语言子菜单：常用直接列出 + "更多语言"二级子菜单（共 29 语）──
    let common: Vec<&str> = lt_i18n::COMMON_LANG_CODES.iter().copied().filter(|c| *c != "auto").collect();
    let lang_menu = Submenu::with_id(MenuId::new("m_lang"), lt_i18n::t("tray_menu_target_lang"), true);
    let mut langs = Vec::new();
    let more = Submenu::with_id(MenuId::new("m_lang_more"), lt_i18n::t("tray_more_langs"), true);
    for (code, native) in lt_i18n::LANGUAGES.iter().skip(1) {
        let item = CheckMenuItem::with_id(ids::lang(code), lang_label(code, *native), true, false, None::<Accelerator>);
        if common.contains(code) {
            lang_menu.append(&item)?;
        } else {
            more.append(&item)?;
        }
        langs.push((code.to_string(), item));
    }
    lang_menu.append(&more)?;
    menu.append(&lang_menu)?;

    // ── 源语言(ASR 提示)子菜单：auto + 全部语言 ──
    let asr_menu = Submenu::with_id(MenuId::new("m_asrlang"), lt_i18n::t("tray_menu_asr_lang"), true);
    let mut asr_langs = Vec::new();
    let asr_more = Submenu::with_id(MenuId::new("m_asrlang_more"), lt_i18n::t("tray_more_langs"), true);
    for (code, native) in lt_i18n::LANGUAGES.iter() {
        let item = CheckMenuItem::with_id(ids::asr_lang(code), lang_label(code, *native), true, false, None::<Accelerator>);
        if *code == "auto" || common.contains(code) {
            asr_menu.append(&item)?;
        } else {
            asr_more.append(&item)?;
        }
        asr_langs.push((code.to_string(), item));
    }
    asr_menu.append(&asr_more)?;
    menu.append(&asr_menu)?;

    // ── 导出子菜单 ──
    let export_menu = Submenu::with_id(MenuId::new("m_export"), lt_i18n::t("export_menu"), true);
    let e1 = MenuItem::with_id(MenuId::new(ids::EXPORT_ORIG), lt_i18n::t("export_original"), true, None::<Accelerator>);
    let e2 = MenuItem::with_id(MenuId::new(ids::EXPORT_TRANS), lt_i18n::t("export_translation"), true, None::<Accelerator>);
    let e3 = MenuItem::with_id(MenuId::new(ids::EXPORT_ALL), lt_i18n::t("export_all"), true, None::<Accelerator>);
    export_menu.append(&e1)?;
    export_menu.append(&e2)?;
    export_menu.append(&e3)?;
    menu.append(&export_menu)?;

    // ── 退出 ──
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
        // M0：托盘本体点击暂不处理（原版无左键行为）
        let _ = &proxy_tray;
    }));

    let icon = tray_icon();
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(lt_i18n::t("tray_tooltip"))
        .with_icon(icon)
        .build()?;

    Ok(Tray {
        icon: tray,
        handles: TrayHandles {
            pause,
            subwin,
            subwin_ct,
            ov_ct,
            ov_topmost,
            ov_autoscroll,
            ov_taskbar,
            langs,
            asr_langs,
        },
    })
}

/// 运行时绘制托盘图标：64×64 蓝底圆角 + 白色 "LT"（矩形拼字，无字体依赖）。
/// 对应原版 create_app_icon() 的 Qt 绘制。
fn tray_icon() -> Icon {
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
