//! 全局 panic hook（架构 2.0 W1/R1，docs/archive/architecture-v2.md §3.2.1）。
//!
//! 实现约束（INV11 受限，P0 修复）：
//! ① hook 内零 unwrap、分配最小化——allocator 中毒时仍须存活；
//! ② 线程安全——多线程同时 panic 各自独立落盘；
//! ③ 双 panic = 直接 abort：本函数自身绝不 panic（全部 fallible 操作忽略错误）；
//! ④ 禁止 MessageBoxW/任何模态——hook 可能运行在 winit 事件循环线程上
//!   （D-33 禁令适用；alacritty 的 hook 弹窗做法本项目不可效仿）；
//! ⑤ tracing 未初始化时也必须留痕：crash 文件在首次 panic 时按需打开
//!   （避免每次运行都产生空 crash 文件）。

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// crash 文件路径（install 时解析一次；None = logs_dir 不可用，只落 stderr）
static CRASH_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn install() {
    let path = lt_models::paths::logs_dir().ok().map(|dir| {
        // 创建目录失败不阻断——打开文件时还会再试
        let _ = std::fs::create_dir_all(&dir);
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        dir.join(format!("crash_{secs}.log"))
    });
    let _ = CRASH_PATH.set(path);

    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let loc = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".into());
        // payload_as_str：1.91+ 稳定；非字符串 payload 给占位
        let msg = info
            .payload_as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<non-string payload>".into());
        let line = format!("PANIC thread={name} loc={loc} msg={msg}\n");

        // ① tracing：已初始化则入文件/控制台/广播（日志窗可见）
        tracing::error!("PANIC thread={name} loc={loc} msg={msg}");

        // ② crash 文件：按需打开追加（release windows_subsystem 下的最后留痕）
        if let Some(Some(path)) = CRASH_PATH.get() {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f ");
                let _ = f.write_all(ts.to_string().as_bytes());
                let _ = f.write_all(line.as_bytes());
                let _ = f.flush();
            }
        }

        // ③ stderr：debug 构建有控制台可直接看；release 无控制台时写入无害
        let _ = std::io::stderr().write_all(line.as_bytes());
    }));
}
