//! 日志基建：文件（DEBUG，按次滚动）+ 控制台（INFO）+ 广播（供日志窗/下载框订阅）。

use crate::supervisor::Policy;
use lt_proto::{ThreadRole, UiEvent};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, Layer};

/// 日志广播通道（日志窗 M4 订阅；下载进度也走此通道）
static HUB: OnceLock<tokio::sync::broadcast::Sender<UiEvent>> = OnceLock::new();

/// 订阅日志广播（新会话/新窗口各持一份 Receiver；日志窗 M4 / 下载框 M2 接线）
#[allow(dead_code)]
pub fn subscribe() -> tokio::sync::broadcast::Receiver<UiEvent> {
    hub().subscribe()
}

fn hub() -> &'static tokio::sync::broadcast::Sender<UiEvent> {
    HUB.get_or_init(|| tokio::sync::broadcast::channel(1024).0)
}

/// 初始化三路日志。只能调用一次（main 早期）。
pub fn init() -> anyhow::Result<()> {
    let dir = lt_models::paths::logs_dir()?;
    std::fs::create_dir_all(&dir)?;
    let name = format!(
        "livetrans_{}.log",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let file = std::fs::File::create(dir.join(&name))?;

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(FileMaker(Mutex::new(file)))
        // naga（wgpu 着色器编译器）首帧编译会同步刷 ~2500 条 DEBUG，既拖慢首绘
        // 又会灌满广播环，静音到 warn（与原版行为无关的第三方噪音）
        .with_filter(tracing_subscriber::EnvFilter::new("debug,naga=warn"));
    let stdout_layer = fmt::layer()
        .with_ansi(false)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .with(BroadcastLayer)
        .init();
    tracing::info!("日志初始化完成: {}", dir.join(&name).display());
    Ok(())
}

/// 每帧写后即 flush（对齐原版行缓冲语义）
struct FileMaker(Mutex<std::fs::File>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileMaker {
    type Writer = LineWriter;
    fn make_writer(&'a self) -> LineWriter {
        LineWriter(self.0.lock().unwrap().try_clone().unwrap())
    }
}

struct LineWriter(std::fs::File);
impl Write for LineWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// 广播层：把事件转成 UiEvent::LogLine 发给订阅者
struct BroadcastLayer;

/// 桥接线程专用 target。Lagged 通知若再入广播环，会在积压未清时立刻触发
/// 下一轮 Lagged → 通知 → Lagged 确定性死循环（实测 ~27 万行/秒刷爆文件与
/// CPU），故通知用该 target 只落文件/控制台，广播层按它旁路。
const BRIDGE_TARGET: &str = "lt_log_bridge";

impl<S> tracing_subscriber::Layer<S> for BroadcastLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // 广播发 DEBUG 及以上（TRACE 量太大不广播；日志窗默认过滤 DEBUG，
        // show_debug 开关控制显示——过滤在窗口侧做，与原版 handler level=DEBUG 一致）
        let level = *event.metadata().level();
        if matches!(level, tracing::Level::TRACE) {
            return;
        }
        if event.metadata().target() == BRIDGE_TARGET {
            return;
        }
        let mut v = MsgVisitor::default();
        event.record(&mut v);
        let _ = hub().send(UiEvent::LogLine {
            level: level_u8(level),
            target: event.metadata().target().to_string(),
            msg: v.msg,
        });
    }
}

/// 被限频吞掉的累计丢弃行数（仅桥接线程读写）
static LAG_PENDING: AtomicU64 = AtomicU64::new(0);
/// 上次丢弃通知时刻（限频 1 次/秒，防通知洪泛）
static LAST_LAG_REPORT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn lag_report_due() -> bool {
    let cell = LAST_LAG_REPORT.get_or_init(|| Mutex::new(None));
    let mut last = cell.lock().unwrap();
    let due = last.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
    if due {
        *last = Some(Instant::now());
    }
    due
}

/// 常驻日志桥接线程（交接卡缺口 #4）：启动即订阅广播 hub，全程把 LogLine
/// 事件推入事件动脉（W2：不再逐条直发 proxy——日志风暴的唤醒合并由动脉
/// 批量上限承担，R23）。
/// 架构 2.0 W1（INV3）：经监督器出生——死亡可见 + Always 重生（工厂克隆同
/// 一 Receiver，重生从中断处继续）。hub 静态存活不会 Closed，故线程退出
/// 条件 = main 停机序置位的 `stop` 标志（50ms 轮询）
pub fn spawn_bridge(
    sup: &crate::supervisor::Supervisor,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    artery: std::sync::Arc<crate::artery::EventArtery>,
) {
    sup.spawn(ThreadRole::LogBridge, "lt-logbridge", Policy::Always, move || {
        // Receiver 不可克隆：重生时重新订阅（广播 hub 全量重放语义由 Lagged 兜底）
        let mut rx = subscribe();
        let stop = stop.clone();
        let artery = artery.clone();
        Box::new(move || loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            match rx.try_recv() {
                Ok(ev) => artery.push(ev),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    LAG_PENDING.fetch_add(n as u64, Ordering::Relaxed);
                    if lag_report_due() {
                        let total = LAG_PENDING.swap(0, Ordering::Relaxed);
                        tracing::debug!(target: BRIDGE_TARGET, "日志桥接丢弃 {total} 行（订阅端积压）");
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => return,
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        })
    });
}

fn level_u8(l: tracing::Level) -> u8 {
    // 对齐 Python logging 数值（UI 过滤语义：默认 ≥20 显示）
    match l {
        tracing::Level::TRACE => 0,
        tracing::Level::DEBUG => 10,
        tracing::Level::INFO => 20,
        tracing::Level::WARN => 30,
        tracing::Level::ERROR => 40,
    }
}

#[derive(Default)]
struct MsgVisitor {
    msg: String,
}

impl tracing::field::Visit for MsgVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        }
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.msg = format!("{value:?}");
        }
    }
}
