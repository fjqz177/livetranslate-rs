//! 日志基建的 subscriber 组装（W3 起：文件 DEBUG + 控制台 INFO + 广播层三路
//! 在此装配；广播层/日志桥已迁 `lt_orchestrator::logging`——日志生产侧归编排域）。

use std::io::Write;
use std::sync::Mutex;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, Layer};

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
        .with(lt_orchestrator::logging::BroadcastLayer)
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
