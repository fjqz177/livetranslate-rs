//! Windows 原生通知（ToastNotificationManager，D-33/H-2）。
//!
//! 未打包 Win32 桌面应用发 Toast 的硬前提：每用户开始菜单存在**带
//! `System.AppUserModel.ID` 属性**的快捷方式（微软官方 DesktopToasts 示例注释
//! "a desktop application must have a shortcut on the Start menu"）。首次发送时自动
//! 注册：RoInitialize（winit 已用 COINIT_APARTMENTTHREADED 初始化 → 本线程为 STA，
//! 必须 RO_INIT_SINGLETHREADED，见 docs/archive/hide-quit-flow-overhaul.md 附录 B）+
//! SetCurrentProcessExplicitAppUserModelID + 快捷方式创建（图标 = 内嵌 app.ico 经
//! 配置目录解压）。任一步失败：`tracing::warn` + 返回 Err，**绝不回退阻塞弹窗**。

use std::sync::OnceLock;

/// AUMID（`Vendor.AppName` 形态；语义稳定不可轻改——与快捷方式属性、进程
/// AppUserModelID 三方必须一致，改动需同时更新 `.lnk`）
pub const AUMID: &str = "com.livetranslate.app";

/// 快捷方式文件名（开始菜单条目 = 通知归属显示名）
const LNK_NAME: &str = "LiveTranslate.lnk";

/// 内嵌图标（.lnk SetIconLocation 需要磁盘路径 → 解压到配置目录，同
/// onnxruntime.dll/silero_vad.onnx 的幂等解压模式）
const ICON: &[u8] = include_bytes!("../../../assets/icons/app.ico");

/// 首次隐藏提示入口（i18n 取词；结果仅记日志，不可阻断 UI）
pub fn show_hidden_hint() {
    let (title, body) = hidden_hint_texts();
    match show(&title, &body) {
        Ok(()) => tracing::info!("已发送原生通知：{title}"),
        Err(e) => tracing::warn!("原生通知失败: {e}"),
    }
}

/// 隐藏提示文案（纯函数；zh/en 对齐断言用）
pub fn hidden_hint_texts() -> (String, String) {
    (
        lt_i18n::t("hide_tray_hint_title"),
        lt_i18n::t("hide_tray_hint"),
    )
}

/// 字幕窗首启拖动提示入口（WP-1：原版 first-show 托盘气泡的一次性等价；
/// 结果仅记日志，不可阻断 UI）
pub fn show_subtitle_hint() {
    let (title, body) = subtitle_hint_texts();
    match show(&title, &body) {
        Ok(()) => tracing::info!("已发送原生通知：{title}"),
        Err(e) => tracing::warn!("原生通知失败: {e}"),
    }
}

/// 字幕窗拖动提示文案（纯函数；zh/en 对齐断言用）
pub fn subtitle_hint_texts() -> (String, String) {
    (
        lt_i18n::t("subwin_drag_hint_title"),
        lt_i18n::t("subwin_drag_hint"),
    )
}

/// 生成 ToastText02 模板 XML（标题/正文两行；字符串装配避开
/// XmlNode→IXmlNodeSerializer 强转链，LoadXml 一步到位）
pub fn build_toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast>\n  <visual>\n    <binding template=\"ToastText02\">\n      <text id=\"1\">{}</text>\n      <text id=\"2\">{}</text>\n    </binding>\n  </visual>\n</toast>",
        xml_escape(title),
        xml_escape(body),
    )
}

/// XML 文本节点转义（当前文案均为 i18n 固定串，转义仅防回归）
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 发送原生通知（非阻塞：Show 交付 Shell 即返回；内部一次性注册 AUMID）
#[cfg(windows)]
pub fn show(title: &str, body: &str) -> anyhow::Result<()> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    imp::ensure_initialized()?;
    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(build_toast_xml(title, body)))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
    notifier.Show(&toast)?;
    Ok(())
}
/// 非 Windows 目标：空实现（本项目仅 Windows 有运行时路径；保持跨平台编译健壮）
#[cfg(not(windows))]
pub fn show(_title: &str, _body: &str) -> anyhow::Result<()> {
    tracing::debug!("非 Windows 平台：原生通知为空实现");
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{Context, Result};
    use std::path::{Path, PathBuf};
    use windows::core::{Interface as _, HSTRING, PWSTR};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoTaskMemAlloc, IPersistFile, CLSCTX_INPROC_SERVER,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_SINGLETHREADED};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{
        IShellLinkW, SetCurrentProcessExplicitAppUserModelID, ShellLink,
    };

    /// 进程级一次性注册结果（SetCurrentProcessExplicitAppUserModelID + 快捷方式）；
    /// 错误以字符串缓存（anyhow::Error 不可 Clone，OnceLock 需 Clone 值）
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();

    /// 每调用线程初始化 + 进程级一次性注册。
    /// RoInitialize 为线程级调用（当前仅 winit 主线程发送）：winit 0.30.13 已以
    /// COINIT_APARTMENTTHREADED 初始化本线程 → 只能用 RO_INIT_SINGLETHREADED，
    /// 误用 MULTITHREADED 会返回 RPC_E_CHANGED_MODE（通知静默失败）。
    pub(super) fn ensure_initialized() -> Result<()> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }
            .context("RoInitialize 失败（winit 主线程应为 STA）")?;
        INIT.get_or_init(|| init_process_appid().map_err(|e| format!("{e:#}")))
            .clone()
            .map_err(anyhow::Error::msg)
    }

    fn init_process_appid() -> Result<()> {
        unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) }
            .context("SetCurrentProcessExplicitAppUserModelID 失败")?;
        ensure_aumid_shortcut()?;
        Ok(())
    }

    /// 快捷方式缺失**或目标路径与当前 exe 不一致**则重建（每用户开始菜单；写
    /// AUMID 属性 + 图标 + 指向当前 exe）。目标不一致重建覆盖：便携 exe 迁移、
    /// 测试示例/旧版安装遗留的指向旧路径 .lnk（避免"存在即跳过"把错误目标
    /// 固化到开始菜单）。
    fn ensure_aumid_shortcut() -> Result<()> {
        let lnk = lnk_path()?;
        let exe = std::env::current_exe().context("定位当前 exe 失败")?;
        let exe_path = exe.to_string_lossy().into_owned();
        if file_exists(&lnk) && lnk_target_matches(&lnk, &exe_path) {
            return Ok(());
        }
        let shell: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
                .context("CoCreateInstance(ShellLink) 失败")?;
        unsafe { shell.SetPath(&HSTRING::from(exe_path.as_str())) }
            .context("IShellLinkW::SetPath 失败")?;
        match ensure_icon_path() {
            Ok(Some(ico)) => {
                if let Err(e) = unsafe { shell.SetIconLocation(&HSTRING::from(ico), 0) } {
                    tracing::warn!("SetIconLocation 失败（通知将用默认图标）: {e}");
                }
            }
            Ok(None) => tracing::debug!("app.ico 解压跳过"),
            Err(e) => tracing::warn!("app.ico 解压失败（通知将用默认图标）: {e}"),
        }
        let store: IPropertyStore = shell.cast().context("IPropertyStore 转换失败")?;
        // VT_LPWSTR 值必须来自 CoTaskMemAlloc——windows crate 为 PROPVARIANT 生成
        // Drop：析构调 PropVariantClear → 对 pwszVal CoTaskMemFree。传 Rust Vec 指针
        // 会释放非 COM 内存（实机堆损坏 0xc0000374，notify_spike 取证）。
        // 内存装进 pv 后所有权移交 PropVariantClear，中途失败同样由它释放。
        let aumid_wide: Vec<u16> = AUMID.encode_utf16().chain(std::iter::once(0)).collect();
        let buf = unsafe { CoTaskMemAlloc(aumid_wide.len() * 2) } as *mut u16;
        if buf.is_null() {
            anyhow::bail!("CoTaskMemAlloc(AUMID) 失败");
        }
        unsafe { std::ptr::copy_nonoverlapping(aumid_wide.as_ptr(), buf, aumid_wide.len()) };
        let pv = PROPVARIANT {
            Anonymous: windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0 {
                Anonymous: core::mem::ManuallyDrop::new(
                    windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0 {
                        vt: VT_LPWSTR,
                        wReserved1: 0,
                        wReserved2: 0,
                        wReserved3: 0,
                        Anonymous:
                            windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0_0 {
                                pwszVal: PWSTR(buf),
                            },
                    },
                ),
            },
        };
        unsafe { store.SetValue(&PKEY_AppUserModel_ID, &pv) }.context("SetValue(AUMID) 失败")?;
        unsafe { store.Commit() }.context("IPropertyStore::Commit 失败")?;
        let persist: IPersistFile = shell.cast().context("IPersistFile 转换失败")?;
        unsafe { persist.Save(&HSTRING::from(lnk.to_string_lossy().as_ref()), true) }
            .context("IPersistFile::Save 失败")?;
        tracing::info!("已注册通知快捷方式: {}", lnk.display());
        Ok(())
    }

    /// 开始菜单路径（每用户；免管理员）
    fn lnk_path() -> Result<PathBuf> {
        let appdata = std::env::var("APPDATA").context("APPDATA 未定义")?;
        Ok(Path::new(&appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(LNK_NAME))
    }

    fn file_exists(p: &Path) -> bool {
        let attrs = unsafe { GetFileAttributesW(&HSTRING::from(p.to_string_lossy().as_ref())) };
        attrs != INVALID_FILE_ATTRIBUTES
    }

    /// 读取既有快捷方式目标并与 exe 比对（失败按"不一致"处理 → 重建）。
    /// 必须先经 IPersistFile::Load 载入文件——新建 IShellLinkW 实例目标为空，
    /// 直接 GetPath 会恒判不一致导致每次重建。
    fn lnk_target_matches(lnk: &Path, exe_path: &str) -> bool {
        let Ok(shell) =
            (unsafe { CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) })
        else {
            return false;
        };
        let Ok(persist) = shell.cast::<IPersistFile>() else {
            return false;
        };
        if unsafe {
            persist.Load(
                &HSTRING::from(lnk.to_string_lossy().as_ref()),
                windows::Win32::System::Com::STGM_READ,
            )
        }
        .is_err()
        {
            return false;
        }
        let mut buf = [0u16; 512];
        let fflags = windows::Win32::UI::Shell::SLGP_UNCPRIORITY.0 as u32;
        let Ok(()) = (unsafe { shell.GetPath(&mut buf, std::ptr::null_mut(), fflags) }) else {
            return false;
        };
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let target = String::from_utf16_lossy(&buf[..len]);
        let eq = target.eq_ignore_ascii_case(exe_path);
        if !eq {
            tracing::info!("通知快捷方式目标已变更（{target} → {exe_path}），重建");
        }
        eq
    }

    /// app.ico 解压到配置目录（尺寸比对幂等；失败仅告警，图标缺失不阻断）
    fn ensure_icon_path() -> Result<Option<String>> {
        let dir = lt_models::paths::config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let target = dir.join("app.ico");
        let ok = std::fs::metadata(&target)
            .map(|m| m.len() == ICON.len() as u64)
            .unwrap_or(false);
        if !ok {
            std::fs::write(&target, ICON)?;
        }
        Ok(Some(target.to_string_lossy().into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ToastText02 结构 + 转义（文本装配是通知内容正确性的唯一纯函数环节）
    #[test]
    fn toast_xml_two_text_nodes_and_escapes() {
        let xml = build_toast_xml("a<b>C", "A & B");
        assert!(xml.contains("template=\"ToastText02\""));
        assert!(xml.contains("<text id=\"1\">a&lt;b&gt;C</text>"));
        assert!(xml.contains("<text id=\"2\">A &amp; B</text>"));
        assert!(!xml.contains("a<b"));
    }

    /// i18n：zh/en 两套文案均可取词且语义对齐（D-33 用户点题）。
    /// 用 t_for_lang 断言（全局 set_lang 在并行测试间竞争，不可依赖）
    #[test]
    fn hidden_hint_texts_both_languages() {
        let zh_t = lt_i18n::t_for_lang("zh", "hide_tray_hint_title");
        let zh_b = lt_i18n::t_for_lang("zh", "hide_tray_hint");
        assert!(zh_t.contains("已隐藏"), "zh 标题: {zh_t}");
        assert!(zh_b.contains("悬浮窗已隐藏"), "zh 正文: {zh_b}");
        let en_t = lt_i18n::t_for_lang("en", "hide_tray_hint_title");
        let en_b = lt_i18n::t_for_lang("en", "hide_tray_hint");
        assert!(en_t.to_lowercase().contains("hidden"), "en 标题: {en_t}");
        assert!(
            en_b.to_lowercase().contains("overlay hidden"),
            "en 正文: {en_b}"
        );
        // 运行时取词路径（全局状态）仅做非空冒烟，不作语言断言
        let (t, b) = hidden_hint_texts();
        assert!(!t.is_empty() && !b.is_empty());
    }
}
