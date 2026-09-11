//! 单实例消息窗（架构 2.0 W6/R11②，落地 WD-5 二次启动激活）。
//!
//! W1 起单实例以 `CreateMutexW` 判 `ERROR_ALREADY_EXISTS` 根除 TOCTOU（R11①）；
//! W6 在此**叠加激活**——第二实例不再只是"退出"，而是定位首实例的
//! message-only 窗并投递激活消息，首实例显示面板并前置（方案 §3.5 步骤 4，
//! 证据 §8.3-4：tauri-plugin-single-instance 同款 + 补 AllowSetForegroundWindow）。
//!
//! 协议（纯 Win32，无 IPC 组件、零新依赖——windows 特性根已含所需 API）：
//! - 首实例：`CreateMutexW` 成功 → 注册类 `LT_SINGLETON_WND` + 建
//!   **message-only 顶层窗**（HWND_MESSAGE，不占任务栏/焦点）；WndProc 收
//!   `WM_APP` 时经 EventLoopProxy 投递 `UiEvent::SecondInstance`（INV1 例外
//!   注记：系统消息→首实例 UI 的唯一契约外通道——消息先天不在动脉生产侧）。
//!   消息在 `run_app` 泵启动前入队，泵启动后由 winit 的 `DispatchMessageW`
//!   按 hwnd 路由到本 WndProc（winit event_loop.rs:418-419 实证）。
//! - 第二实例：`ERROR_ALREADY_EXISTS` → `FindWindowW` → `AllowSetForegroundWindow`
//!   （前台锁防线——无它 SetForegroundWindow 常被系统拒）+ `PostMessageW(WM_APP)`
//!   → Err 退出（窗未找到=首实例 boot 极窄竞窗，放弃激活仅退出，旧语义仍在）。
//!
//! WndProc 零分配零阻塞（proxy 非阻塞投递）；D-33 安全：消息经 winit 泵在
//! 事件循环线程分发，无模态调用。

use winit::event_loop::EventLoopProxy;

use lt_proto::{UiEvent, UiMsg};

/// message-only 窗类名/标题（FindWindowW 匹配键；裸类名不撞系统）
const WND_CLASS: &str = "LiveTranslateSingletonWnd";
/// 互斥量名（与 W1 相同；改名破坏"已在运行检测"既有语义）
const MUTEX_NAME: &str = "LiveTranslateSingleInstance";

const WM_APP: u32 = 0x8000;

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::Graphics::Gdi::HBRUSH;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, FindWindowW,
        GetWindowThreadProcessId, PostMessageW, RegisterClassW, HCURSOR, HICON, HWND_MESSAGE,
        WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW, WNDCLASS_STYLES, WS_OVERLAPPED,
    };

    /// proxy 转发槽（boot 赋值一次；WndProc 仅在泵运行后收到消息，
    /// 彼时 proxy 必已 ready）
    #[cfg(windows)]
    static PROXY: OnceLock<EventLoopProxy<UiMsg>> = OnceLock::new();

    /// WndProc（数据面零分配零阻塞；在事件循环线程经 winit 泵分发）
    #[cfg(windows)]
    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_APP {
            if let Some(proxy) = PROXY.get() {
                let _ = proxy.send_event(UiMsg::Event(UiEvent::SecondInstance));
            }
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// 单实例一体检查（boot 期调用，EventLoop 建立后、run_app 前）：
    /// - 二次实例：mutex 已存在 → 激活首实例窗（尽力而为）→ Err（调用方退出）
    /// - 首实例：建 message-only 窗接激活消息 → Ok（后续由事件循环泵驱动）
    pub(super) fn ensure(proxy: EventLoopProxy<UiMsg>) -> anyhow::Result<()> {
        unsafe {
            let name: Vec<u16> = format!("{MUTEX_NAME}\0").encode_utf16().collect();
            CreateMutexW(None, true, PCWSTR(name.as_ptr()))
                .map_err(|e| anyhow::anyhow!("创建单实例互斥量失败: {e}"))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                activate_first_instance();
                return Err(anyhow::anyhow!(
                    "LiveTranslate 已在运行（单实例；已发起激活）"
                ));
            }
            // 首实例：建消息窗（消息在泵启动前入队，启动后 WndProc 被调）
            let _ = PROXY.set(proxy);
            let module = GetModuleHandleW(None)
                .map_err(|e| anyhow::anyhow!("GetModuleHandleW 失败: {e}"))?;
            let hinstance: windows::Win32::Foundation::HINSTANCE = module.into();
            let cls: Vec<u16> = format!("{WND_CLASS}\0").encode_utf16().collect();
            let wc = WNDCLASSW {
                style: WNDCLASS_STYLES(0),
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance,
                hIcon: HICON::default(),
                hCursor: HCURSOR::default(),
                hbrBackground: HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: PCWSTR(cls.as_ptr()),
            };
            let _ = RegisterClassW(&wc); // 类已注册返回 0（错误 1410）可容忍
            let name: Vec<u16> = format!("{WND_CLASS}\0").encode_utf16().collect();
            let _hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(name.as_ptr()),
                PCWSTR(name.as_ptr()),
                WINDOW_STYLE(WS_OVERLAPPED.0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance),
                None,
            )
            .map_err(|e| anyhow::anyhow!("创建单实例消息窗失败: {e}"))?;
            // 句柄不 Drop（裸 HWND），进程存活期内保持；进程退出 OS 回收
        }
        Ok(())
    }

    /// 第二实例侧：定位首实例窗并投递激活。返回 (是否投递成功)。
    fn activate_first_instance() -> bool {
        unsafe {
            let name: Vec<u16> = format!("{WND_CLASS}\0").encode_utf16().collect();
            let Ok(hwnd) = FindWindowW(PCWSTR::null(), PCWSTR(name.as_ptr())) else {
                return false; // 首实例 boot 极窄竞窗：放弃激活（mutex 守卫仍在）
            };
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let _ = AllowSetForegroundWindow(pid);
            let _ = PostMessageW(Some(hwnd), WM_APP, WPARAM(0), LPARAM(0));
            true
        }
    }
}

#[cfg(not(windows))]
pub(super) fn ensure(_proxy: EventLoopProxy<UiMsg>) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
pub(super) fn ensure(proxy: EventLoopProxy<UiMsg>) -> anyhow::Result<()> {
    win::ensure(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 协议常量稳定性：WD-5 唤醒消息与窗类名是跨进程约定（改名=二次实例
    /// 激活静默失效——FindWindowW 找不到、PostMessage 错窗）
    #[test]
    fn protocol_constants_stable() {
        assert_eq!(WND_CLASS, "LiveTranslateSingletonWnd");
        assert_eq!(MUTEX_NAME, "LiveTranslateSingleInstance");
        assert_eq!(WM_APP, 0x8000);
    }
}
