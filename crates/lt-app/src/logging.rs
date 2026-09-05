//! 日志基建：文件（DEBUG，按次滚动）+ 控制台（INFO）+ 广播（供日志窗/下载框订阅）。

use lt_proto::UiEvent;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
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
        .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG);
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
        let mut v = MsgVisitor::default();
        event.record(&mut v);
        let _ = hub().send(UiEvent::LogLine {
            level: level_u8(level),
            target: event.metadata().target().to_string(),
            msg: v.msg,
        });
    }
}

/// 常驻日志桥接线程（交接卡缺口 #4）：启动即订阅广播 hub，全程把 LogLine
/// 事件转发到 UI（原版 LogWindow handler 常驻 root logger 的等价物）。
pub fn spawn_bridge(proxy: winit::event_loop::EventLoopProxy<lt_proto::UiMsg>) {
    let mut rx = subscribe();
    std::thread::Builder::new()
        .name("lt-logbridge".into())
        .spawn(move || loop {
            match rx.blocking_recv() {
                Ok(ev) => {
                    let _ = proxy.send_event(lt_proto::UiMsg::Event(ev));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!("日志桥接丢弃 {n} 行（订阅端积压）");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        })
        .ok();
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
    fn record_debug(
        &mut self,
        field: &tracing::field::Field,
        value: &dyn std::fmt::Debug,
    ) {
        if field.name() == "message" {
            self.msg = format!("{value:?}");
        }
    }
}
