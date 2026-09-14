//! 日志基建的 subscriber 组装（W3 起：文件 DEBUG + 控制台 INFO + 广播层三路
//! 在此装配；广播层/日志桥已迁 `lt_orchestrator::logging`——日志生产侧归编排域）。

use std::io::Write;
use std::sync::Mutex;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, Layer};

/// 文件层与广播层共用的噪声闸：默认 debug 全开，仅点名降档**零诊断价值**的
/// 第三方刷屏源（自家 lt_* 不在名单，DEBUG 照常全通路可达）——
/// - `naga=warn`：wgpu 着色器编译器首帧同步刷 ~2500 条 DEBUG，拖慢首绘；
/// - `wasapi=info`：设备热切换轮询（wasapi_win.rs `DEVICE_CHECK_INTERVAL`=2s）
///   每次触发该 crate 内部一条 DEBUG，长跑灌满 UI 2000 行环形缓冲、逐出
///   真实历史并令日志浮钮 +N 空涨（2026-09-14）；轮询本身照跑，设备身份
///   在界面设备列表本就可见，该行零信息量。
///
/// 闸必须文件层与广播层**各挂一次**：tracing 各层过滤相互独立，只挂文件层
/// 时广播层照常收货（naga 旧例即只挂了文件层，UI 环形缓冲每启动仍被灌满）。
const NOISE_FILTER: &str = "debug,naga=warn,wasapi=info";

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
        .with_filter(tracing_subscriber::EnvFilter::new(NOISE_FILTER));
    let stdout_layer = fmt::layer()
        .with_ansi(false)
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stdout_layer)
        .with(
            lt_orchestrator::logging::BroadcastLayer
                .with_filter(tracing_subscriber::EnvFilter::new(NOISE_FILTER)),
        )
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

#[cfg(test)]
mod tests {
    use super::NOISE_FILTER;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer;

    /// 防回归：噪声闸语义——第三方刷屏源（wasapi/naga）的 DEBUG 在总闸丢弃，
    /// 自家 DEBUG 与第三方 INFO+ 放行。闸若被移除或常量被改坏，此测即红。
    #[test]
    fn noise_filter_gates_third_party_debug_only() {
        // 捕获层：记录通过闸的事件 target（顺序即发射顺序）
        let seen = Arc::new(Mutex::new(Vec::new()));
        struct Capture(Arc<Mutex<Vec<String>>>);
        impl<S: tracing::Subscriber> Layer<S> for Capture {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.metadata().target().to_string());
            }
        }
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(NOISE_FILTER))
            .with(Capture(seen.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "wasapi::api", "default device Ok(..)");
            tracing::info!(target: "wasapi::api", "real info kept");
            tracing::debug!(target: "lt_audio::vad", "自家 DEBUG 照常");
            tracing::debug!(target: "naga::back", "naga DEBUG 闸掉");
        });
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["wasapi::api".to_string(), "lt_audio::vad".to_string()],
            "wasapi DEBUG 应被闸掉（INFO 放行）、自家 DEBUG 放行、naga DEBUG 应被闸掉"
        );
    }
}
