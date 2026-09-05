//! 开机自启（原版 platform 集成的 Windows 路径）：HKCU\Software\Microsoft\
//! Windows\CurrentVersion\Run 写入/删除当前 exe 路径（REG_SZ，带引号防路径空格）。
//!
//! 注意：settings 契约缺 `autostart` 键（lt-proto 不动）→ 注册表即事实源，
//! 面板勾选状态每次进入常规页时回读（[`enabled`]）。

use anyhow::anyhow;

/// Run 键子路径（HKCU）
const RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
/// 值名（进程名语义，原版同款）
const VALUE_NAME: &str = "LiveTranslate";

/// 当前 exe 路径的注册表数据形态：带引号（原版 set_autostart 的 Windows 写法）
fn command_for_exe() -> anyhow::Result<Vec<u16>> {
    let exe = std::env::current_exe()?.to_string_lossy().into_owned();
    let quoted = format!("\"{exe}\"");
    Ok(quoted.encode_utf16().chain(std::iter::once(0)).collect())
}

/// 当前是否已登记自启（读失败按未启用处理，不阻塞 UI）
pub fn enabled() -> bool {
    #[cfg(windows)]
    {
        is_enabled_win()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 写入/删除自启项；失败返回 Err（调用方弹 autostart_failed 提示并回滚勾选，
/// 原版 _on_autostart_toggled 的异常路径）
pub fn set_enabled(on: bool) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        set_enabled_win(on)
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err(anyhow!("autostart unavailable on this platform"))
    }
}

#[cfg(windows)]
fn is_enabled_win() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let subkey: Vec<u16> = RUN_SUBKEY.encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = VALUE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    err == ERROR_SUCCESS && buf.iter().take_while(|&&c| c != 0).count() > 0
}

#[cfg(windows)]
fn set_enabled_win(on: bool) -> anyhow::Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let subkey: Vec<u16> = RUN_SUBKEY.encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = VALUE_NAME.encode_utf16().chain(std::iter::once(0)).collect();
    let err = if on {
        let data = command_for_exe()?;
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(value.as_ptr()),
                REG_SZ.0,
                Some(data.as_ptr().cast()),
                (data.len() * 2) as u32,
            )
        }
    } else {
        // 值不存在（ERROR_FILE_NOT_FOUND）视为删除成功（幂等）
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr()), PCWSTR(value.as_ptr())) }
    };
    if err == ERROR_SUCCESS {
        tracing::info!("开机自启: {on}");
        Ok(())
    } else {
        Err(anyhow!("registry op failed: {err:?}"))
    }
}

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表数据形态：当前 exe 路径带引号 + NUL 结尾（宽字符）
    #[test]
    fn command_data_is_quoted_exe_path() {
        let data = command_for_exe().unwrap();
        let text = String::from_utf16(&data[..data.len() - 1]).unwrap();
        assert!(text.starts_with('"') && text.ends_with('"'), "应带引号: {text}");
        assert!(text.contains(".exe"));
        assert_eq!(*data.last().unwrap(), 0);
    }

    /// Run 键常量与原版/系统约定一致（HKCU\...\CurrentVersion\Run，值名=应用名）
    #[test]
    fn run_key_constants() {
        assert_eq!(RUN_SUBKEY, "Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        assert_eq!(VALUE_NAME, "LiveTranslate");
    }
}

#[cfg(not(windows))]
#[cfg(test)]
mod tests {
    /// 非 Windows：平台不支持（原版 autostart_unavailable 分支）
    #[test]
    fn non_windows_reports_unavailable() {
        assert!(!super::enabled());
        assert!(super::set_enabled(true).is_err());
    }
}
