//! 线程监督器（架构 2.0 W1，docs/archive/architecture-v2.md §3.2.1）。
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
use std::time::{Duration, Instant};

use lt_proto::{ThreadDied, ThreadRole, UiEvent};

/// 重启策略（W1：Always=死即重生；Never=一次性线程，死亡仅上报；
/// E4/D-80 增 Backoff=指数退避重生，兑现方案 §3.2.1 原设计）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    Never,
    Always,
    /// 指数退避重生：死亡后延迟 `min(base << (n-1), max)` 再重生；连续
    /// `give_up_after` 次异常退出即放弃并告警（`ThreadDied{restarted:false}`，
    /// 不再静默）；健康存活满 `reset_after_ms` 清零连败计数（偶发 panic
    /// 不积累）。防"确定性 panic 线程以 500ms 周期无限重生"的崩溃风暴
    /// （方案 §3.2.1：Backoff 超限 → 放弃重启 + 告警事件）
    Backoff {
        base_ms: u64,
        max_ms: u64,
        give_up_after: u32,
        reset_after_ms: u64,
    },
}

impl Policy {
    /// 常驻线程默认退避（capture/ASR/翻译池/日志桥，E4/D-80）：0.5s 起步
    /// 翻倍至 30s 封顶，8 连败（累计尝试窗约 91s）后放弃。
    /// **动脉桥保持 Always**——UI 活性本身死透 = 界面全死，宁可无限重生；
    /// 其循环体仅 pop/send/take 三个无 panic 源操作，风暴风险可控。
    pub const fn backoff() -> Self {
        Self::Backoff {
            base_ms: 500,
            max_ms: 30_000,
            give_up_after: 8,
            reset_after_ms: 60_000,
        }
    }
}

/// Backoff 延迟计算（纯函数便于单测）：`min(base << (n-1), max)`；
/// 移位钳 16 位 + saturating_mul 防 u64 溢出
fn backoff_delay_ms(base_ms: u64, max_ms: u64, consecutive: u32) -> u64 {
    let exp = consecutive.saturating_sub(1).min(16);
    base_ms.saturating_mul(1u64 << exp).min(max_ms)
}

struct Entry {
    role: ThreadRole,
    name: String,
    handle: Option<JoinHandle<()>>,
    policy: Policy,
    /// 重启工厂：每次 spawn 重新构造运行闭包（须捕获全部共享态的克隆，
    /// INV5：构造干净初态）
    factory: Box<dyn Fn() -> Box<dyn FnOnce() + Send + 'static> + Send>,
    /// Backoff：当前连败计数（健康窗清零，见 [`Policy::backoff`]）
    consecutive: u32,
    /// Backoff：下次重生到期时刻（死亡后置位；handle=None 等待期间）
    next_eligible: Option<Instant>,
    /// 最近一次出生时刻（健康窗判定基准）
    last_born: Instant,
    /// "预期退役"信号位（`spawn_retirable` 专用）：置位后该线程的正常退出
    /// 视为设计内收尾（静默收割，不报 ThreadDied、不重生）；panic 仍按策略处理
    expected_exit: Option<Arc<AtomicBool>>,
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
        // E1-3/INV4 延伸：stopping 置位后拒绝一切出生——monitor 的 respawn
        // 封堵（begin_shutdown 后复查）之外，本出生口是另一条路，停机期迟到
        // 的任务（如收尾中触发的下载/探测）不得把线程重新拉起。拒绝不 panic：
        // 停机竞态宁可漏一次任务，不可炸穿收尾序。
        if self.stopping.load(Ordering::SeqCst) {
            tracing::error!("Supervisor::spawn 停机后被调用（{role:?}）——已拒绝（INV4）");
            return;
        }
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
            consecutive: 0,
            next_eligible: None,
            last_born: Instant::now(),
            expected_exit: None,
        });
    }

    /// 与 [`Supervisor::spawn`] 同义，但额外接受一个"**预期退役**"信号位：
    /// 线程因自身生命周期结束（如翻译池被替换、池已置停止标志）而正常退出时，
    /// 由该位显式声明"这是设计内收尾"——监督器据此静默收割（不报 ThreadDied、
    /// 不重生），取代旧的"未停机即退出（异常）"判定。
    ///
    /// 背景（第三轮复核实证）：翻译池 8 个 worker 以 Backoff 策略出生，装置被
    /// 替换时它们**正常退出**，却被判为异常 → 每次换模型/测试连接都刷 8 条
    /// 假错误 + 8 条"已放弃重启"。panic（真实异常）仍照常上报与重生。
    pub fn spawn_retirable(
        &self,
        role: ThreadRole,
        name: impl Into<String>,
        policy: Policy,
        expected_exit: Arc<AtomicBool>,
        factory: impl Fn() -> Box<dyn FnOnce() + Send + 'static> + Send + 'static,
    ) {
        if self.stopping.load(Ordering::SeqCst) {
            tracing::error!("Supervisor::spawn_retirable 停机后被调用（{role:?}）——已拒绝（INV4）");
            return;
        }
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
            consecutive: 0,
            next_eligible: None,
            last_born: Instant::now(),
            expected_exit: Some(expected_exit),
        });
    }

    /// INV4 停机序第一步：置 stopping，此后 monitor 不再重生任何线程。
    /// 必须先于一切线程停止信号调用。
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
    }

    /// 查询命名线程是否已结束（W7：DownloadManager 在途会话判定——
    /// JoinHandle 轮询的监督器版语义；Entry 不存在 = 从未出生或已收割 → 真）。
    pub fn is_thread_finished(&self, name: &str) -> bool {
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .find(|e| e.name == name)
            .is_none_or(|e| match &e.handle {
                None => true,
                Some(h) => h.is_finished(),
            })
    }

    /// join 全部被监督线程 + monitor。须在 begin_shutdown 与各线程停止信号
    /// （stop 原子/通道关闭）之后调用。常驻无出口线程（如日志桥，hub 恒
    /// 存活不会 Closed）由调用方注入专用停止标志作为退出条件。
    pub fn join_all(&self) {
        self.begin_shutdown();
        let handles: Vec<JoinHandle<()>> = {
            let mut entries = self.entries.lock().unwrap();
            entries.iter_mut().filter_map(|e| e.handle.take()).collect()
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
            // Backoff 到期重生：handle=None 且 next_eligible 到期的条目补生
            //（死亡处理只置 next_eligible，重生在此统一进行）。
            // INV4（E4 复审补封）：本循环位于 loop-top stopping 检查之后——
            // 停机若恰落在「死亡已判定、等待退避」窗口（handle=None）内，
            // 此处不得补生，否则 join_all 已按 None 跳过的条目会漏出一条
            // 永不 join 的活线程
            if self.stopping.load(Ordering::SeqCst) {
                return;
            }
            for e in entries.iter_mut() {
                if e.handle.is_none() {
                    if let Policy::Backoff { .. } = e.policy {
                        if e.next_eligible.is_some_and(|at| Instant::now() >= at) {
                            match std::thread::Builder::new()
                                .name(e.name.clone())
                                .spawn((e.factory)())
                            {
                                Ok(h) => {
                                    e.handle = Some(h);
                                    e.last_born = Instant::now();
                                }
                                Err(err) => tracing::error!("{} 退避重生失败: {err}", e.name),
                            }
                        }
                    }
                }
            }
            let mut reap = Vec::new();
            for (i, e) in entries.iter_mut().enumerate() {
                let Some(h) = e.handle.as_mut() else { continue };
                if !h.is_finished() {
                    // Backoff 健康窗：存活满 reset_after 即清零连败计数
                    //（偶发 panic 不积累——见 Policy::backoff）
                    if let Policy::Backoff { reset_after_ms, .. } = e.policy {
                        if e.last_born.elapsed() >= Duration::from_millis(reset_after_ms) {
                            e.consecutive = 0;
                        }
                    }
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
                // 预期退役（如翻译池被替换）：正常退出是设计内收尾——静默收割。
                // 只有 panic 才算异常（仍上报并按策略重生）。
                let expected_exit = e
                    .expected_exit
                    .as_ref()
                    .is_some_and(|f| f.load(Ordering::Relaxed));
                if expected_exit && !panicked {
                    tracing::debug!("{} 正常退役（预期退出信号已置位，静默收割）", e.name);
                    reap.push(i);
                    continue;
                }
                // Policy::Always / Backoff：非停机退出 = 上一线程死亡（任何原因）
                let detail = if panicked {
                    format!("{} panic（详情见 crash 文件与日志）", e.name)
                } else {
                    format!("{} 未停机即退出（异常）", e.name)
                };
                match e.policy {
                    Policy::Always => {
                        (self.sink)(ThreadDied {
                            role: e.role,
                            detail,
                            restarted: true,
                        });
                        let run = (e.factory)();
                        match std::thread::Builder::new().name(e.name.clone()).spawn(run) {
                            Ok(h) => {
                                e.handle = Some(h);
                                e.last_born = Instant::now();
                            }
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
                    Policy::Backoff {
                        base_ms,
                        max_ms,
                        give_up_after,
                        ..
                    } => {
                        e.consecutive = e.consecutive.saturating_add(1);
                        if e.consecutive > give_up_after {
                            // 方案 §3.2.1 兑现：超限放弃 + 告警事件（不再静默）
                            (self.sink)(ThreadDied {
                                role: e.role,
                                detail: format!(
                                    "{} 连续 {} 次异常退出，已放弃重启（backoff 超限；详情见 crash 文件与日志）",
                                    e.name, e.consecutive
                                ),
                                restarted: false,
                            });
                            reap.push(i);
                            continue;
                        }
                        let delay_ms = backoff_delay_ms(base_ms, max_ms, e.consecutive);
                        e.next_eligible = Some(Instant::now() + Duration::from_millis(delay_ms));
                        tracing::warn!(
                            "{} 异常退出（第 {} 次），{}ms 后退避重生",
                            e.name,
                            e.consecutive,
                            delay_ms
                        );
                        // handle 保持 None：条目留在表内等待到期重生循环补生
                    }
                    Policy::Never => unreachable!("Never 分支已在上方处理"),
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
pub fn artery_sink(
    sink: Arc<crate::event_artery::EventArtery>,
) -> impl Fn(ThreadDied) + Send + Sync + 'static {
    move |d: ThreadDied| {
        sink.push(UiEvent::ThreadDied(d));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Instant;

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
        let d = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("应有死亡上报");
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
        let d = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("应有死亡上报");
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
        sup.spawn(
            ThreadRole::DeviceProbe,
            "test-oneshot",
            Policy::Never,
            || Box::new(|| {}),
        );
        sup.spawn(
            ThreadRole::FileDialog,
            "test-oneshot-2",
            Policy::Never,
            || Box::new(|| {}),
        );
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

    /// E1-3：begin_shutdown 后 spawn 被拒绝（INV4 延伸——出生口自身的停机
    /// 封堵，非 monitor respawn 面）；拒绝不入表，join_all 立即收敛
    #[test]
    fn spawn_after_shutdown_is_noop() {
        let (sup, _rx) = setup();
        sup.begin_shutdown();
        sup.spawn(
            ThreadRole::Capture,
            "test-late-spawn",
            Policy::Always,
            || Box::new(|| panic!("停机后出生的线程不应存在（E1-3 拒绝失守）")),
        );
        assert!(sup.entries.lock().unwrap().is_empty(), "拒绝出生不得入表");
        sup.join_all();
    }

    /// W7：`is_thread_finished` 生命周期语义——出生待执行 = false；
    /// 正常退出 = true；收割后（Entry 移除）= true（"无在途"收敛）
    #[test]
    fn is_thread_finished_tracks_lifecycle() {
        let (sup, _rx) = setup();
        let enter = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let enter2 = enter.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        sup.spawn(
            ThreadRole::Download,
            "test-dl-session",
            Policy::Never,
            move || {
                let enter = enter2.clone();
                let stop = stop2.clone();
                Box::new(move || {
                    enter.store(true, Ordering::SeqCst);
                    // 退出前让调用方先观察到 in-flight
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = stop.load(Ordering::SeqCst);
                })
            },
        );
        // 出生窗口内必为在途（spawn 返回时线程已启动，enter 很快置位）
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !enter.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(enter.load(Ordering::SeqCst), "线程应已启动");
        assert!(!sup.is_thread_finished("test-dl-session"), "运行中应判在途");
        // 等退出 + monitor 收割（500ms 轮询）→ 判"已结束"
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !sup.is_thread_finished("test-dl-session") && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            sup.is_thread_finished("test-dl-session"),
            "退出（含收割）后应判已结束"
        );
        // 从未出生的名字恒为"已结束"（可开新会话）
        assert!(sup.is_thread_finished("never-existed"));
        sup.join_all();
    }

    /// E4/D-80：Backoff 延迟计算纯函数——指数翻倍 + max 封顶
    #[test]
    fn backoff_delay_caps_at_max() {
        assert_eq!(backoff_delay_ms(500, 30_000, 1), 500);
        assert_eq!(backoff_delay_ms(500, 30_000, 2), 1_000);
        assert_eq!(backoff_delay_ms(500, 30_000, 3), 2_000);
        assert_eq!(
            backoff_delay_ms(500, 30_000, 7),
            30_000,
            "32_000 → 封顶 30_000"
        );
        assert_eq!(backoff_delay_ms(500, 30_000, 8), 30_000);
        assert_eq!(backoff_delay_ms(100, 300, 5), 300, "1_600 → 封顶 300");
        assert_eq!(
            backoff_delay_ms(0, 30_000, 8),
            0,
            "base=0 退化立即重生（saturating 不 panic）"
        );
    }

    /// E4/D-80：Backoff 首次重生前至少等待 base（指数退避生效）
    #[test]
    fn backoff_respects_delay() {
        let (sup, _rx) = setup();
        let births: Arc<Mutex<Vec<Instant>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let b2 = births.clone();
        let s2 = stop.clone();
        sup.spawn(
            ThreadRole::Capture,
            "test-backoff-delay",
            Policy::Backoff {
                base_ms: 500,
                max_ms: 5_000,
                give_up_after: 8,
                reset_after_ms: 60_000,
            },
            move || {
                let births = b2.clone();
                let stop = s2.clone();
                Box::new(move || {
                    births.lock().unwrap().push(Instant::now());
                    if births.lock().unwrap().len() == 1 {
                        panic!("首次出生即炸（backoff 延迟测试注入）");
                    }
                    while !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                })
            },
        );
        std::thread::sleep(Duration::from_millis(1600));
        let births = births.lock().unwrap();
        assert!(births.len() >= 2, "退避后应重生");
        let gap = (births[1] - births[0]).as_millis() as u64;
        assert!(
            gap >= 450,
            "重生间隔 {gap}ms 应 ≥ base 500ms（调度容差 50ms）"
        );
        stop.store(true, Ordering::SeqCst);
        sup.join_all();
    }

    /// 第三轮复核回归：**预期退役**的正常退出必须静默（不报 ThreadDied、不重生）
    /// ——旧实现把它判为"未停机即退出（异常）"，导致每次换模型/测试连接刷一屏
    /// 假错误日志 + "已放弃重启"
    #[test]
    fn retirable_normal_exit_is_silent() {
        let (sup, rx) = setup();
        let exit_flag = Arc::new(AtomicBool::new(false));
        let flag = exit_flag.clone();
        sup.spawn_retirable(
            ThreadRole::TlWorker,
            "retirable-test",
            Policy::backoff(),
            exit_flag.clone(),
            move || {
                let flag = flag.clone();
                Box::new(move || {
                    // 模拟翻译池被替换：置"预期退役"后正常结束
                    flag.store(true, Ordering::Relaxed);
                })
            },
        );
        // 让 monitor 观察到退出与重生窗口
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            match rx.try_recv() {
                Ok(d) => panic!("预期退役不得上报 ThreadDied: {d:?}"),
                Err(_) => std::thread::sleep(Duration::from_millis(30)),
            }
            if sup.is_thread_finished("retirable-test") {
                break;
            }
        }
        assert!(sup.is_thread_finished("retirable-test"));
        assert!(rx.try_recv().is_err(), "预期退役全过程不得有任何死亡事件");
        sup.begin_shutdown();
        sup.join_all();
    }

    /// panic 仍按策略处理（预期退役信号不得掩盖真实异常）
    #[test]
    fn retirable_panic_still_reported() {
        let (sup, rx) = setup();
        let exit_flag = Arc::new(AtomicBool::new(true)); // 即使信号已置位
        sup.spawn_retirable(
            ThreadRole::TlWorker,
            "retirable-panic",
            Policy::Never,
            exit_flag,
            || Box::new(|| panic!("boom")),
        );
        let d = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("panic 必须上报");
        assert!(d.detail.contains("panic"), "actual: {d:?}");
        sup.begin_shutdown();
        sup.join_all();
    }

    /// E4/D-80：连续异常退出超限 → 放弃重生 + 告警事件（restarted=false）
    #[test]
    fn backoff_gives_up_and_reports() {
        let (sup, rx) = setup();
        let born = Arc::new(AtomicUsize::new(0));
        let born2 = born.clone();
        sup.spawn(
            ThreadRole::TlWorker,
            "test-backoff-giveup",
            Policy::Backoff {
                base_ms: 100,
                max_ms: 400,
                give_up_after: 2,
                reset_after_ms: 60_000,
            },
            move || {
                let born = born2.clone();
                Box::new(move || {
                    born.fetch_add(1, Ordering::SeqCst);
                    panic!("出生即炸（give-up 测试注入）");
                })
            },
        );
        // 3 次出生（退避 100/200ms）+ 放弃判定，留 monitor 节拍裕量
        let deadline = std::time::Instant::now() + Duration::from_secs(6);
        while born.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(
            born.load(Ordering::SeqCst),
            3,
            "give_up_after=2 应恰重生 2 次（共 3 次出生）"
        );
        // 放弃事件到达（前两次死亡事件 restarted=true，逐条等目标事件）
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut got = false;
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(d) => {
                    if !d.restarted && d.detail.contains("放弃") {
                        got = true;
                        break;
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert!(got, "应收到放弃重启的 ThreadDied");
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(born.load(Ordering::SeqCst), 3, "放弃后不得再重生");
        sup.join_all();
    }

    /// E4/D-80：健康存活满 reset_after 清零连败计数——偶发 panic 不积累
    ///（give_up_after=1 场景：若清零失效，第 2 次死亡即弃管、第 3 次出生不发生）
    #[test]
    fn backoff_resets_after_healthy_run() {
        let (sup, _rx) = setup();
        let born = Arc::new(AtomicUsize::new(0));
        let born2 = born.clone();
        sup.spawn(
            ThreadRole::AsrMain,
            "test-backoff-reset",
            Policy::Backoff {
                base_ms: 100,
                max_ms: 400,
                give_up_after: 1,
                reset_after_ms: 300,
            },
            move || {
                let born = born2.clone();
                Box::new(move || {
                    let n = born.fetch_add(1, Ordering::SeqCst) + 1;
                    if n == 1 {
                        panic!("首次出生即炸（reset 测试注入）");
                    }
                    // 第 2 次：健康存活 900ms（跨越多个 monitor 节拍，确保
                    // 健康窗清零在退出前生效）后正常退出
                    std::thread::sleep(Duration::from_millis(900));
                })
            },
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while born.load(Ordering::SeqCst) < 3 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            born.load(Ordering::SeqCst) >= 3,
            "健康窗清零应阻止放弃（否则 give_up_after=1 在第 2 次死亡后弃管）"
        );
        sup.join_all();
    }

    /// E4 复审修复（INV4 补封回归）：Backoff 等待重生窗口内停机 → 到期不得
    /// 补生（修复前本测试失败：next_eligible 到期后 monitor 在停机期把线程
    /// 拉起，且该线程在 join_all 的 None 句柄之外永不 join）
    #[test]
    fn backoff_pending_respawn_cancelled_by_shutdown() {
        let (sup, _rx) = setup();
        let born = Arc::new(AtomicUsize::new(0));
        let born2 = born.clone();
        sup.spawn(
            ThreadRole::Capture,
            "test-backoff-shutdown",
            Policy::Backoff {
                base_ms: 300,
                max_ms: 1_000,
                give_up_after: 5,
                reset_after_ms: 60_000,
            },
            move || {
                let born = born2.clone();
                Box::new(move || {
                    born.fetch_add(1, Ordering::SeqCst);
                    panic!("出生即炸（停机窗口测试注入）");
                })
            },
        );
        // 等进入「死亡已判定、等待退避」窗口（handle=None）
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let waiting = sup
                .entries
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.name == "test-backoff-shutdown" && e.handle.is_none());
            if waiting || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            sup.entries
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.handle.is_none()),
            "应进入退避等待窗口"
        );
        sup.begin_shutdown();
        let born_at_shutdown = born.load(Ordering::SeqCst);
        // 越过 next_eligible（base 300ms）+ monitor 节拍裕量
        std::thread::sleep(Duration::from_millis(1200));
        assert_eq!(
            born.load(Ordering::SeqCst),
            born_at_shutdown,
            "停机后不得补生（INV4）"
        );
        sup.join_all();
    }
}
