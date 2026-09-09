//! 线程监督器（架构 2.0 W1，docs/architecture-v2.md §3.2.1）。
//!
//! 不变量落地：
//! - INV3 出生唯一：全仓合法线程出生点 = [`Supervisor::spawn`]（白名单特例
//!   lt-ui/tray.rs 与本文件 monitor 由 §6.2 守护禁令圈定）；
//! - INV4 停机序：[`Self::begin_shutdown`] 先于一切线程停止信号置位，此后
//!   monitor 不再重生——封堵「停机时 supervisor 与 stop 竞速重拉线程」的
//!   竞态洞（宁可漏一次 respawn，不可多一次）；
//! - INV5 重启即会话复位：factory 必须构造干净初态，由调用方保证（capture/
//!   ASR 重生入口固定为待命/初始态，不得从主循环中段恢复）。
//!
//! 死亡检测 = monitor 500ms 轮询 `JoinHandle`（ADR-3：catch_unwind 覆盖不了
//! abort/栈溢出；监督 crate 生态不成熟，std 原语零依赖）。panic 细节由全局
//! panic hook 落 crash 文件与 tracing，本层只上报「死亡 + 是否已重启」。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use lt_proto::{ThreadDied, ThreadRole, UiEvent};

/// 重启策略（W1：Always=死即重生；Never=一次性线程，死亡仅上报）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Never,
    Always,
}

struct Entry {
    role: ThreadRole,
    name: String,
    handle: Option<JoinHandle<()>>,
    policy: Policy,
    /// 重启工厂：每次 spawn 重新构造运行闭包（须捕获全部共享态的克隆，
    /// INV5：构造干净初态）
    factory: Box<dyn Fn() -> Box<dyn FnOnce() + Send + 'static> + Send>,
}

/// 死亡上报出口（生产 = proxy 发 UiEvent::ThreadDied；测试 = 通道收集）
type DeathSink = Arc<dyn Fn(ThreadDied) + Send + Sync>;

pub struct Supervisor {
    stopping: AtomicBool,
    entries: Mutex<Vec<Entry>>,
    monitor: Mutex<Option<JoinHandle<()>>>,
    sink: DeathSink,
}

impl Supervisor {
    /// 创建监督器并启动死亡监视线程（monitor 出生在本文件内，白名单特例）。
    /// `sink` 在死亡时被调用（monitor 线程上下文，须非阻塞——proxy.send_event
    /// 满足；禁止在 sink 里做同步模态，INV11）
    pub fn new(sink: impl Fn(ThreadDied) + Send + Sync + 'static) -> Arc<Self> {
        let sup = Arc::new(Self {
            stopping: AtomicBool::new(false),
            entries: Mutex::new(Vec::new()),
            monitor: Mutex::new(None),
            sink: Arc::new(sink),
        });
        let sup2 = sup.clone();
        let h = std::thread::Builder::new()
            .name("lt-supervisor".into())
            .spawn(move || sup2.monitor_loop())
            .expect("监督器 monitor 创建失败");
        *sup.monitor.lock().unwrap() = Some(h);
        sup
    }

    /// 注册并启动一个被监督线程。`factory` 惰性持有：死亡重生时重新调用以
    /// 构造干净初态的运行闭包（INV5）。
    pub fn spawn(
        &self,
        role: ThreadRole,
        name: impl Into<String>,
        policy: Policy,
        factory: impl Fn() -> Box<dyn FnOnce() + Send + 'static> + Send + 'static,
    ) {
        let name = name.into();
        let run = factory();
        let handle = std::thread::Builder::new()
            .name(name.clone())
            .spawn(run)
            .expect("被监督线程创建失败");
        self.entries.lock().unwrap().push(Entry {
            role,
            name,
            handle: Some(handle),
            policy,
            factory: Box::new(factory),
        });
    }

    /// INV4 停机序第一步：置 stopping，此后 monitor 不再重生任何线程。
    /// 必须先于一切线程停止信号调用。
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// join 全部被监督线程 + monitor。须在 begin_shutdown 与各线程停止信号
    /// （stop 原子/通道关闭）之后调用。常驻无出口线程（如日志桥，hub 恒
    /// 存活不会 Closed）由调用方注入专用停止标志作为退出条件。
    pub fn join_all(&self) {
        self.begin_shutdown();
        let handles: Vec<JoinHandle<()>> = {
            let mut entries = self.entries.lock().unwrap();
            entries
                .iter_mut()
                .filter_map(|e| e.handle.take())
                .collect()
        };
        for h in handles {
            let _ = h.join();
        }
        if let Some(h) = self.monitor.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    fn monitor_loop(self: Arc<Self>) {
        loop {
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
            let mut entries = self.entries.lock().unwrap();
            let mut reap = Vec::new();
            for (i, e) in entries.iter_mut().enumerate() {
                let Some(h) = e.handle.as_mut() else { continue };
                if !h.is_finished() {
                    continue;
                }
                let h = e.handle.take().expect("is_finished 已判定存在句柄");
                let panicked = h.join().is_err();
                // INV4：停机期静默收割（不重生不上报——应用正在退出）
                if self.stopping.load(Ordering::SeqCst) {
                    continue;
                }
                if e.policy == Policy::Never {
                    // W5 收口（P1）：一次性线程（设备探测/文件对话框/基准）的
                    // 正常退出是设计内收尾——旧语义把每次操作都误报为
                    // ThreadDied 错误行（"未停机即退出（异常）"）。panic 仍
                    // 上报，正常退出静默收割（tracing::debug 保留可观测性）。
                    if panicked {
                        (self.sink)(ThreadDied {
                            role: e.role,
                            detail: format!("{} panic（详情见 crash 文件与日志）", e.name),
                            restarted: false,
                        });
                    } else {
                        tracing::debug!("{} 正常退出（Never 策略静默收割）", e.name);
                    }
                    reap.push(i);
                    continue;
                }
                // Policy::Always：非停机退出 = 上一线程死亡（任何原因）→ 上报 + 重生
                let detail = if panicked {
                    format!("{} panic（详情见 crash 文件与日志）", e.name)
                } else {
                    format!("{} 未停机即退出（异常）", e.name)
                };
                (self.sink)(ThreadDied {
                    role: e.role,
                    detail,
                    restarted: true,
                });
                let run = (e.factory)();
                match std::thread::Builder::new().name(e.name.clone()).spawn(run) {
                    Ok(h) => e.handle = Some(h),
                    Err(err) => {
                        tracing::error!("{} 重生失败: {err}", e.name);
                        (self.sink)(ThreadDied {
                            role: e.role,
                            detail: format!("{} 重生失败: {err}", e.name),
                            restarted: false,
                        });
                    }
                }
            }
            // 死条目收割（重生的 handle 已复位；已死未重生的移除——长期运行
            // 不积累空条目；一次性线程每次探测/弹框都有进出）
            for i in reap.into_iter().rev() {
                entries.remove(i);
            }
        }
    }
}

/// 生产死亡出口：UiEvent::ThreadDied 经事件动脉回流 UI（W2：与全部后台
/// 事件同路——INV1 回流通路唯一；sink.push 非阻塞，monitor 线程安全）
pub fn artery_sink(sink: Arc<crate::event_artery::EventArtery>) -> impl Fn(ThreadDied) + Send + Sync + 'static {
    move |d: ThreadDied| {
        sink.push(UiEvent::ThreadDied(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn setup() -> (Arc<Supervisor>, mpsc::Receiver<ThreadDied>) {
        let (tx, rx) = mpsc::channel();
        let sup = Supervisor::new(move |d| {
            let _ = tx.send(d);
        });
        (sup, rx)
    }

    /// Always 策略：panic 死亡 → 上报 restarted=true → 工厂重生
    #[test]
    fn always_policy_restarts_after_panic() {
        let (sup, rx) = setup();
        let stop = Arc::new(AtomicBool::new(false));
        let born = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop2 = stop.clone();
        let born2 = born.clone();
        sup.spawn(
            ThreadRole::TlWorker,
            "test-always",
            Policy::Always,
            move || {
                let born = born2.clone();
                let stop = stop2.clone();
                Box::new(move || {
                    born.fetch_add(1, Ordering::SeqCst);
                    if born.load(Ordering::SeqCst) == 1 {
                        panic!("首次出生即炸（监督测试注入）");
                    }
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                })
            },
        );
        // 等重生（首次死亡 + respawn）
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while born.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(born.load(Ordering::SeqCst) >= 2, "Always 策略应重生");
        let d = rx.recv_timeout(Duration::from_secs(2)).expect("应有死亡上报");
        assert_eq!(d.role, ThreadRole::TlWorker);
        assert!(d.restarted);
        assert!(d.detail.contains("panic"));
        stop.store(true, Ordering::SeqCst);
        sup.join_all();
    }

    /// Never 策略：panic 死亡只上报不重生
    #[test]
    fn never_policy_reports_without_restart() {
        let (sup, rx) = setup();
        sup.spawn(ThreadRole::Bench, "test-never", Policy::Never, || {
            Box::new(|| panic!("一次性线程炸（监督测试注入）"))
        });
        let d = rx.recv_timeout(Duration::from_secs(5)).expect("应有死亡上报");
        assert_eq!(d.role, ThreadRole::Bench);
        assert!(!d.restarted);
        std::thread::sleep(Duration::from_millis(800));
        assert!(rx.try_recv().is_err(), "Never 不得重生再上报");
        sup.join_all();
    }

    /// W5 收口（P1）：Never 一次性线程**正常退出**——设计内收尾，不报
    /// ThreadDied（旧语义把每次设备探测/文件对话框/基准收尾误报为日志
    /// 错误行），死条目收割不积累
    #[test]
    fn never_policy_normal_exit_silent_and_reaped() {
        let (sup, rx) = setup();
        sup.spawn(ThreadRole::DeviceProbe, "test-oneshot", Policy::Never, || {
            Box::new(|| {})
        });
        sup.spawn(ThreadRole::FileDialog, "test-oneshot-2", Policy::Never, || {
            Box::new(|| {})
        });
        // 等 monitor 两轮心跳（500ms/轮）+ 收割
        std::thread::sleep(Duration::from_millis(1200));
        assert!(rx.try_recv().is_err(), "正常退出不得上报 ThreadDied");
        assert_eq!(
            sup.entries.lock().unwrap().len(),
            0,
            "死条目应收割（正常退出无一重生）"
        );
        sup.join_all();
    }

    /// INV4：begin_shutdown 后 monitor 不得重生（停机竞态封堵）
    #[test]
    fn stopping_prevents_respawn() {
        let (sup, rx) = setup();
        let born = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let born2 = born.clone();
        sup.spawn(
            ThreadRole::Capture,
            "test-stopping",
            Policy::Always,
            move || {
                let born = born2.clone();
                Box::new(move || {
                    born.fetch_add(1, Ordering::SeqCst);
                    panic!("出生即炸（监督测试注入）");
                })
            },
        );
        // 立即停机：monitor 下一 tick 只静默收割
        sup.begin_shutdown();
        std::thread::sleep(Duration::from_millis(1200));
        assert!(
            born.load(Ordering::SeqCst) <= 1,
            "stopping 后不得重生（INV4）"
        );
        assert!(rx.try_recv().is_err(), "停机期不上报死亡事件");
        sup.join_all();
    }
}
