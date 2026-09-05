//! 管道装配（M2.6；原版 main.py 启动流等价）：
//!
//! 音频线程（wasapi 后端内部）→ 16k mono chunk 满丢旧队列
//! → capture 线程（CaptureLoop：RMS/VAD 监视 + VAD 状态机 + 超时静音推进）
//!     - Monitor 事件：直接经 EventLoopProxy 推 UI（每 chunk ≈31/s）
//!     - Segment 事件：进满丢旧段队列（监视事件永不挤掉语音段）
//! → ASR 线程：AsrManager.transcribe（独占）→ 段处理（噪声过滤）→ AddMessage。
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
    let entry = match registry::funasr_entry(&settings.funasr_model) {
        Some(e) => e,
        None => {
            let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
            tracing::error!("未知 funasr 模型键: {}", settings.funasr_model);
            return;
        }
    };
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
                if result.text.is_empty() {
                    continue;
                }
                // 噪声过滤（原版 _process_segment：长段却只有 ≤3 个字母数字 → 丢弃）
                let alnum = result.text.chars().filter(|c| c.is_alphanumeric()).count();
                if seg_seconds >= 2.0 && alnum <= 3 {
                    tracing::debug!(
                        "噪声过滤: {seg_seconds:.1}s 段仅产出 {:?}",
                        result.text
                    );
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
