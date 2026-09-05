//! 管道装配（M2.6；原版 main.py 启动流等价）：
//!
//! 音频线程（wasapi 后端内部）→ 16k mono chunk 满丢旧队列
//! → capture 线程（CaptureLoop：RMS/VAD 监视 + VAD 状态机 + 超时静音推进）
//!     - Monitor 事件：直接经 EventLoopProxy 推 UI（每 chunk ≈31/s）
//!     - Segment 事件：进满丢旧段队列（监视事件永不挤掉语音段）
//! → ASR 线程：AsrManager.transcribe（独占）→ 段级三层过滤（空/纯标点 → 噪声 → 语言，
//!   对照原版 main.py `_process_segment`）→ AddMessage。
//!
//! 未就绪链路：模型未缓存 → 发 AsrUnavailable（M2.5 向导接管首启下载）。

use lt_asr::{AsrManager, WorkerConfig};
use lt_models::registry;
use lt_pipeline::audio::wasapi_win::WasapiBackend;
use lt_pipeline::{AudioBackend, BoundedDropQueue, CaptureEvent, CaptureLoop, SegmentSource, VadProcessor, VadSettings};
use lt_proto::{UiEvent, UiMsg};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// 段队列容量（对齐原版 _asr_queue 上限量级）
const SEGMENT_QUEUE_CAP: usize = 20;

pub struct Pipeline {
    backend: WasapiBackend,
    stop: Arc<AtomicBool>,
    /// 暂停标志（capture 线程丢弃 chunk 不喂 VAD）；M4 托盘暂停联动
    #[allow(dead_code)]
    paused: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Pipeline {
    /// 按设置启动整条管道；模型未缓存时不阻断 UI（发 AsrUnavailable）
    pub fn start(settings: &lt_proto::Settings, proxy: EventLoopProxy<UiMsg>) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));

        // ── 音频：chunk 满丢旧队列 ──
        let chunk_queue = Arc::new(BoundedDropQueue::new(100));
        let mut backend = WasapiBackend::new();
        backend.start(
            settings.audio_device.clone(),
            settings.mic_device.clone(),
            chunk_queue.clone(),
        )?;

        // ── capture 线程：VAD 状态机 ──
        let vad_settings = vad_settings_from(settings);
        let confidence = lt_pipeline::vad::make_confidence_source(
            &settings.vad_mode,
            settings.energy_threshold as f64,
        );
        let mut vad = VadProcessor::new(
            confidence,
            lt_pipeline::TARGET_RATE as usize,
            settings.vad_threshold as f64,
            settings.min_speech_duration as f64,
            settings.max_speech_duration as f64,
            lt_pipeline::CHUNK_DURATION,
        );
        vad.update_settings(&vad_settings);

        let segment_queue = Arc::new(BoundedDropQueue::new(SEGMENT_QUEUE_CAP));
        // capture 线程事件出口：Monitor → 分流线程推 UI；Segment → 段队列
        let (ev_tx, ev_rx) = crossbeam_channel::unbounded::<CaptureEvent>();
        let mut threads = Vec::new();
        {
            let stop = stop.clone();
            let paused = paused.clone();
            std::thread::Builder::new().name("lt-capture".into()).spawn(move || {
                let loop_ = CaptureLoop { chunk_rx: chunk_queue, events: ev_tx, paused };
                loop_.run(&mut vad, &stop);
            })?;
        }

        // ── 事件分流线程：Monitor→UI；Segment→段队列 ──
        {
            let stop = stop.clone();
            let proxy = proxy.clone();
            let segment_queue = segment_queue.clone();
            threads.push(std::thread::Builder::new().name("lt-events".into()).spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match ev_rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(CaptureEvent::Monitor { rms, vad, mic_rms }) => {
                            let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateMonitor {
                                rms,
                                vad: vad as f32,
                                mic_rms,
                            }));
                        }
                        Ok(CaptureEvent::Segment { source, audio }) => {
                            if matches!(source, SegmentSource::VadFlush) {
                                segment_queue.push((audio, std::time::Instant::now()));
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })?);
        }

        // ── ASR 线程：Manager 独占 + 段处理 ──
        {
            let stop = stop.clone();
            let proxy = proxy.clone();
            let segment_queue = segment_queue.clone();
            let settings = settings.clone();
            threads.push(std::thread::Builder::new().name("lt-asr-main".into()).spawn(move || {
                run_asr_thread(&settings, segment_queue, stop, proxy);
            })?);
        }

        tracing::info!("管道已启动（capture + VAD + ASR）");
        Ok(Self { backend, stop, paused, threads })
    }

    #[allow(dead_code)]
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.backend.stop();
        for h in self.threads.drain(..) {
            let _ = h.join();
        }
        tracing::info!("管道已停止");
    }
}

fn vad_settings_from(s: &lt_proto::Settings) -> VadSettings {
    VadSettings {
        mode: s.vad_mode.clone(),
        threshold: s.vad_threshold as f64,
        energy_threshold: s.energy_threshold as f64,
        min_speech_duration: s.min_speech_duration as f64,
        max_speech_duration: s.max_speech_duration as f64,
        silence_mode: s.silence_mode.clone(),
        silence_duration: s.silence_duration as f64,
    }
}

/// 过滤原因（供 run_asr_thread 按层分级记日志，单测断言用）
const REJECT_EMPTY: &str = "empty/punctuation-only";
const REJECT_NOISE: &str = "noise";
const REJECT_LANGUAGE: &str = "language-mismatch";

/// 段级结果过滤（对照原版 main.py `_process_segment` 的三层过滤，顺序一致）。
/// 返回 `Some(原因)` 表示丢弃该段：
/// 1. 空/纯标点（无任何字母数字字符）；
/// 2. 噪声（段长 ≥2.0s 且字母数字字符数 ≤3）；
/// 3. 语言（设置非 auto 且识别语言与设置不符）。
fn reject_segment(
    text: &str,
    seg_seconds: f64,
    asr_language: &str,
    detected_lang: &str,
) -> Option<&'static str> {
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
    if text.is_empty() || alnum == 0 {
        return Some(REJECT_EMPTY);
    }
    if seg_seconds >= 2.0 && alnum <= 3 {
        return Some(REJECT_NOISE);
    }
    if asr_language != "auto" && detected_lang != asr_language {
        return Some(REJECT_LANGUAGE);
    }
    None
}

/// funasr 模型键 → (条目, 是否回退)。
/// mlt 无上游 ONNX 转换、待上游产出（D-14）；mlt/非法键统一回退 sensevoice-small（bool = true）。
fn resolve_funasr_entry(key: &str) -> (registry::ModelEntry, bool) {
    match registry::funasr_entry(key) {
        Some(e) => (e, false),
        None => (
            registry::funasr_entry("sensevoice-small").expect("sensevoice-small 常量条目必存在"),
            true,
        ),
    }
}

/// ASR 线程：模型就绪则循环识别；未缓存则发 AsrUnavailable 后待命
fn run_asr_thread(
    settings: &lt_proto::Settings,
    segment_queue: Arc<BoundedDropQueue<(Vec<f32>, std::time::Instant)>>,
    stop: Arc<AtomicBool>,
    proxy: EventLoopProxy<UiMsg>,
) {
    // 构造 worker 配置（当前仅 sensevoice；whisper M5）
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
            tracing::error!("模型目录不可用: {e}");
            return;
        }
    };
    // 模型键 → 条目；mlt/非法键回退 sensevoice-small（不阻断 UI，也不得用 nano 冒充）
    let (entry, fell_back) = resolve_funasr_entry(&settings.funasr_model);
    if fell_back {
        let reason = if settings.funasr_model == "funasr-mlt-nano-2512" {
            "mlt 无上游 ONNX 转换，待上游产出（D-14）"
        } else {
            "非法模型键"
        };
        tracing::warn!(
            "funasr 模型 {:?} 不可用（{reason}），回退 sensevoice-small",
            settings.funasr_model
        );
    }
    let Some(model_dir) = lt_models::cache::local_model_dir(&models_dir, &entry) else {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
        tracing::warn!("ASR 模型未缓存（{entry:?}），等待向导/下载（M2.5）");
        // 待命：模型就绪前吞掉段（无引擎可用）
        while !stop.load(Ordering::Relaxed) {
            if segment_queue.pop_timeout(Duration::from_secs(1)).is_none() {
                continue;
            }
        }
        return;
    };

    let config = WorkerConfig {
        engine: "sensevoice".into(),
        display_name: entry.display.into(),
        language: settings.asr_language.clone(),
        pad_seconds: Some(settings.sensevoice_pad_seconds),
        options: serde_json::json!({ "model_dir": model_dir.to_string_lossy() }),
    };
    let mut manager = AsrManager::new();
    if let Err(e) = manager.ensure_started(&config) {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
        tracing::error!("ASR worker 启动失败: {e}");
    } else {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
            "{} [cpu]",
            entry.display
        ))));
    }

    while !stop.load(Ordering::Relaxed) {
        let Some((audio, _queued_at)) = segment_queue.pop_timeout(Duration::from_millis(500)) else {
            continue;
        };
        if audio.is_empty() {
            continue;
        }
        let seg_seconds = audio.len() as f64 / lt_pipeline::TARGET_RATE as f64;
        let t0 = std::time::Instant::now();
        match manager.transcribe(&audio, false) {
            Ok(result) => {
                // 段级三层过滤（原版 _process_segment：空/纯标点 → 噪声 → 语言）
                if let Some(reason) =
                    reject_segment(&result.text, seg_seconds, &settings.asr_language, &result.language)
                {
                    match reason {
                        REJECT_LANGUAGE => {
                            // 预览按字符截断，避免切坏 UTF-8 边界
                            let preview: String = result.text.chars().take(60).collect();
                            tracing::info!(
                                "语言过滤: 期望 {:?} 但识别为 {:?}，丢弃: {preview}",
                                settings.asr_language,
                                result.language
                            );
                        }
                        REJECT_NOISE => tracing::debug!(
                            "噪声过滤: {seg_seconds:.1}s 段仅产出 {:?}",
                            result.text
                        ),
                        _ => tracing::debug!("ASR 返回空/纯标点结果，跳过: {:?}", result.text),
                    }
                    continue;
                }
                let asr_ms = t0.elapsed().as_secs_f64() * 1000.0;
                let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                let id = uuid::Uuid::new_v4().as_u128() as u64;
                let _ = proxy.send_event(UiMsg::Event(UiEvent::AddMessage {
                    id,
                    timestamp,
                    original: result.text,
                    lang: result.language,
                    asr_ms,
                }));
            }
            Err(e) => {
                tracing::warn!("ASR 段识别失败: {e}");
                if e.unavailable() {
                    let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
                }
            }
        }
    }
    manager.shutdown();
    tracing::info!("ASR 线程退出");
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── reject_segment：三层过滤（对照原版 _process_segment） ──

    #[test]
    fn empty_text_rejected() {
        assert_eq!(reject_segment("", 1.0, "auto", "zh"), Some(REJECT_EMPTY));
    }

    #[test]
    fn punctuation_only_rejected() {
        // 中英文纯标点均无字母数字字符 → 第 1 层拒绝
        assert_eq!(reject_segment("。。。", 1.0, "auto", "zh"), Some(REJECT_EMPTY));
        assert_eq!(reject_segment("!!!", 0.5, "auto", "en"), Some(REJECT_EMPTY));
    }

    #[test]
    fn whitespace_only_rejected() {
        // 空白串同样无字母数字字符，等价纯标点
        assert_eq!(reject_segment("   ", 1.0, "auto", "zh"), Some(REJECT_EMPTY));
    }

    #[test]
    fn noise_long_segment_few_alnum_rejected() {
        // 段长 ≥2.0s 且仅 ≤3 个字母数字 → 第 2 层拒绝
        assert_eq!(reject_segment("abc", 2.0, "auto", "zh"), Some(REJECT_NOISE));
        assert_eq!(reject_segment("嗯。嗯。嗯", 3.5, "auto", "zh"), Some(REJECT_NOISE));
    }

    #[test]
    fn short_segment_few_chars_passes() {
        // 短段（<2.0s）不属于噪声过滤范围
        assert_eq!(reject_segment("ab", 0.5, "auto", "zh"), None);
    }

    #[test]
    fn normal_text_passes() {
        assert_eq!(reject_segment("你好世界", 2.5, "auto", "zh"), None);
    }

    #[test]
    fn language_mismatch_rejected() {
        // 设置 zh 但识别为 en → 第 3 层拒绝
        assert_eq!(reject_segment("hello world", 1.0, "zh", "en"), Some(REJECT_LANGUAGE));
    }

    #[test]
    fn auto_language_skips_lang_filter() {
        // auto 时不做语言过滤：即使识别为 en 也放行
        assert_eq!(reject_segment("hello world", 1.0, "auto", "en"), None);
    }

    #[test]
    fn filter_order_empty_before_language() {
        // 顺序固定：空文本即使语言不匹配也命中第 1 层而非第 3 层
        assert_eq!(reject_segment("", 3.0, "zh", "en"), Some(REJECT_EMPTY));
    }

    // ── resolve_funasr_entry：mlt/非法键回退（D-14） ──

    #[test]
    fn mlt_falls_back_to_sensevoice() {
        let (entry, fell_back) = resolve_funasr_entry("funasr-mlt-nano-2512");
        assert!(fell_back);
        // 绝不能静默用 nano 冒充 mlt（D-14）
        assert_eq!(entry, registry::funasr_entry("sensevoice-small").unwrap());
    }

    #[test]
    fn bogus_key_falls_back_to_sensevoice() {
        // 非法键与原版非法值语义一致：回退 sensevoice-small
        let (entry, fell_back) = resolve_funasr_entry("bogus");
        assert!(fell_back);
        assert_eq!(entry.key, "sensevoice-small");
    }

    #[test]
    fn valid_keys_no_fallback() {
        let (entry, fell_back) = resolve_funasr_entry("sensevoice-small");
        assert!(!fell_back);
        assert_eq!(entry.key, "sensevoice-small");

        let (entry, fell_back) = resolve_funasr_entry("funasr-nano-2512");
        assert!(!fell_back);
        assert_eq!(entry.key, "funasr-nano-2512");
    }
}
