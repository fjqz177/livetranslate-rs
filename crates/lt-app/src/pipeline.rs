//! 管道装配（M2.6；原版 main.py 启动流等价）：
//!
//! 音频线程（wasapi 后端内部）→ 16k mono chunk 满丢旧队列
//! → capture 线程（CaptureLoop：RMS/VAD 监视 + VAD 状态机 + 超时静音推进），
//!   与原版 _capture_loop 一致、无中间分流线程：
//!     - monitor 数据：capture 线程内经 EventLoopProxy 直接推 UI
//!       （等价原版跨线程调 update_monitor，每 chunk ≈31/s）
//!     - 语音段：capture 线程直接入满丢旧段队列（等价原版 _enqueue_asr）
//! → ASR 线程：段队列空闲时做 RSS 回收（原版 _asr_loop queue.Empty 分支）；
//!   AsrManager.transcribe（独占）→ 段级三层过滤（空/纯标点 → 噪声 → 语言，
//!   对照原版 main.py `_process_segment`）→ AddMessage
//!   → 同语言直接回空译文；否则提交翻译线程池（M3；原版 _tl_executor，
//!   max_workers=8）→ UpdateStreaming/UpdateTranslation/UpdateStats。
//!
//! 未就绪链路：模型未缓存 → 发 AsrUnavailable（M2.5 向导接管首启下载）。

use lt_asr::{AsrManager, WorkerConfig};
use lt_models::registry;
use lt_pipeline::audio::wasapi_win::WasapiBackend;
use lt_pipeline::{AudioBackend, BoundedDropQueue, CaptureLoop, SegmentSource, VadProcessor, VadSettings};
use lt_proto::{UiEvent, UiMsg};
use lt_translate::Translator;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event_loop::EventLoopProxy;

/// 段队列容量（对齐原版 _asr_queue maxsize=16，满丢最旧）
const SEGMENT_QUEUE_CAP: usize = 16;

/// 翻译线程池 worker 数（对齐原版 ThreadPoolExecutor(max_workers=8)）
const TL_POOL_WORKERS: usize = 8;

/// 定宽任务池（等价原版 _tl_executor：任务排队、固定 worker 消费）
struct JobPool {
    tx: crossbeam_channel::Sender<Box<dyn FnOnce() + Send>>,
    stopped: Arc<AtomicBool>,
}

impl JobPool {
    fn new(workers: usize) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Box<dyn FnOnce() + Send>>();
        let rx = Arc::new(rx);
        let stopped = Arc::new(AtomicBool::new(false));
        for i in 0..workers {
            let rx = rx.clone();
            let stopped = stopped.clone();
            let _ = std::thread::Builder::new()
                .name(format!("lt-tl-{i}"))
                .spawn(move || {
                    while let Ok(job) = rx.recv() {
                        // 停止后排队的任务直接丢弃（等价 executor.shutdown）
                        if stopped.load(Ordering::Relaxed) {
                            continue;
                        }
                        job();
                    }
                });
        }
        Self { tx, stopped }
    }

    fn submit(&self, job: impl FnOnce() + Send + 'static) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let _ = self.tx.send(Box::new(job));
    }

    /// 停止接收并丢弃排队任务；在跑的任务自然结束（translate 自带超时兜底）
    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}

/// 翻译/用量统计（跨 ASR 线程与翻译 worker；原子量）
pub(crate) struct TlStats {
    asr_count: AtomicU64,
    tl_count: AtomicU64,
    prompt_tokens: AtomicU64,
    completion_tokens: AtomicU64,
    input_price: f64,
    output_price: f64,
}

impl TlStats {
    fn new(input_price: f64, output_price: f64) -> Self {
        Self {
            asr_count: AtomicU64::new(0),
            tl_count: AtomicU64::new(0),
            prompt_tokens: AtomicU64::new(0),
            completion_tokens: AtomicU64::new(0),
            input_price,
            output_price,
        }
    }

    fn cost(&self) -> f64 {
        lt_translate::compute_cost(
            self.prompt_tokens.load(Ordering::Relaxed),
            self.completion_tokens.load(Ordering::Relaxed),
            self.input_price,
            self.output_price,
        )
    }

    fn snapshot_event(&self) -> UiEvent {
        UiEvent::UpdateStats {
            asr_n: self.asr_count.load(Ordering::Relaxed),
            tl_n: self.tl_count.load(Ordering::Relaxed),
            prompt_tokens: self.prompt_tokens.load(Ordering::Relaxed),
            completion_tokens: self.completion_tokens.load(Ordering::Relaxed),
            cost: self.cost(),
        }
    }
}

/// 翻译装置：Translator + 统计 + 线程池（ASR 线程与翻译 worker 共享）
struct TlRig {
    translator: Arc<Translator>,
    stats: Arc<TlStats>,
    pool: JobPool,
    /// 会话转录写盘（原版 self._transcript；ASR 线程写原文，worker 配对译文）
    transcript: Arc<lt_pipeline::transcript::TranscriptWriter>,
}

impl TlRig {
    /// 按设置构建；models 为空/active_model 越界 → None（不翻译，仅 ASR）
    fn from_settings(settings: &lt_proto::Settings) -> Option<Self> {
        let mc = settings.models.get(settings.active_model)?;
        Self::from_model_config(mc, settings)
    }

    /// 按指定模型配置构建（运行时切换用；构建失败仅告警并返回 None）
    fn from_model_config(mc: &lt_proto::ModelConfig, settings: &lt_proto::Settings) -> Option<Self> {
        let params = lt_translate::TranslatorParams {
            api_base: mc.api_base.clone(),
            api_key: mc.api_key.clone(),
            model: mc.model.clone(),
            target_language: settings.target_language.clone(),
            max_tokens: 256,
            temperature: 0.3,
            streaming: mc.streaming,
            system_prompt: (!settings.system_prompt.is_empty())
                .then(|| settings.system_prompt.clone()),
            proxy: mc.proxy.clone(),
            // 原版 no_think 缺省 true（legacy 迁移后仅 thinking_style 生效）
            no_think: true,
            no_system_role: mc.no_system_role,
            thinking_style: mc.thinking_style.clone(),
            json_response: mc.json_response,
            timeout: settings.timeout,
            overrides: mc.overrides.clone(),
            extra_body: mc.extra_body.clone(),
        };
        let translator = match Translator::new(params) {
            Ok(t) => {
                t.set_context_turns(mc.context_turns);
                Arc::new(t)
            }
            Err(e) => {
                tracing::error!("Translator 构建失败（模型 {:?}）: {e}", mc.name);
                return None;
            }
        };
        let stats = Arc::new(TlStats::new(mc.input_price, mc.output_price));
        tracing::info!(
            "Switching translator: {} ({})",
            mc.name,
            mc.model
        );
        Some(Self {
            translator,
            stats,
            pool: JobPool::new(TL_POOL_WORKERS),
            transcript: Pipeline::transcript_handle(),
        })
    }

    /// 提交一段的翻译任务（对照原版 _translate_async 的成功/重复/错误三路）；
    /// 同语言不进此函数（ASR 线程直接回空译文，见 run_asr_thread）
    fn submit_translation(&self, proxy: &EventLoopProxy<UiMsg>, id: u64, text: String, source_lang: String) {
        let translator = self.translator.clone();
        let stats = self.stats.clone();
        let transcript = self.transcript.clone();
        let proxy = proxy.clone();
        self.pool.submit(move || {
            let t0 = Instant::now();
            let mut translated: Option<String> = None;
            for item in translator.translate_iter(&text, &source_lang) {
                match item {
                    Ok(partial) => {
                        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateStreaming {
                            id,
                            partial: partial.clone(),
                        }));
                        translated = Some(partial);
                    }
                    Err(lt_translate::TranslateError::Repetition(_)) => {
                        tracing::warn!(
                            "Repetition loop detected, model may not support structured output well"
                        );
                        transcript.finalize_no_translation(id);
                        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
                            id,
                            text: lt_i18n::t("error_repetition"),
                            tl_ms: 0.0,
                        }));
                        return;
                    }
                    Err(e) => {
                        if e.is_expected() {
                            tracing::warn!("Translate error: {e}");
                        } else {
                            tracing::error!("Translate error: {e}");
                        }
                        transcript.finalize_no_translation(id);
                        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
                            id,
                            text: e.ui_text(),
                            tl_ms: 0.0,
                        }));
                        return;
                    }
                }
            }
            // 成功路径（原版 _translate_async 循环结束后）
            let tl_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let translated = translated.unwrap_or_default();
            stats.tl_count.fetch_add(1, Ordering::Relaxed);
            let (pt, ct) = translator.last_usage();
            stats.prompt_tokens.fetch_add(pt, Ordering::Relaxed);
            stats.completion_tokens.fetch_add(ct, Ordering::Relaxed);
            tracing::info!("Translate ({tl_ms:.0}ms): {translated}");
            let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
                id,
                text: translated.clone(),
                tl_ms,
            }));
            let _ = proxy.send_event(UiMsg::Event(stats.snapshot_event()));
            if translated.is_empty() {
                transcript.finalize_no_translation(id);
            } else {
                transcript.write_translation(id, &translated);
            }
        });
    }

    fn shutdown(&self) {
        self.pool.shutdown();
    }
}

pub struct Pipeline {
    backend: WasapiBackend,
    stop: Arc<AtomicBool>,
    /// 暂停标志（capture 线程丢弃 chunk 不喂 VAD）；M4 托盘暂停联动
    #[allow(dead_code)]
    paused: Arc<AtomicBool>,
    /// UI 线程挂起语言/padding 的句柄：UI 线程调用 set_pending_*，ASR 线程在
    /// 下一次 transcribe 前应用（原版 _set_asr_language/_set_asr_padding +
    /// _apply_pending_asr_settings）；仅加锁存值，绝不跨进程调用
    #[allow(dead_code)]
    pending: lt_asr::AsrPendingHandle,
    /// 翻译装置（models 空/构建失败时 None = 仅 ASR 不翻译）
    tl: Option<Arc<TlRig>>,
    /// 运行时翻译器切换通道（UI 域命令 → ASR 线程空闲分支应用；
    /// 原版对应 _switch_translator / set_target_language / set_timeout）
    tl_switch: Option<crossbeam_channel::Sender<TlSwitch>>,
    /// VAD 参数热更新槽（面板"应用"→ capture 线程）
    vad_update: Arc<std::sync::Mutex<Option<lt_pipeline::VadSettings>>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

/// ASR 线程消费的翻译器/引擎命令
pub(crate) enum TlSwitch {
    /// 整体重建翻译装置（切模型；历史随旧实例丢弃，与原版重建 Translator 一致）
    ReplaceRig { config: Box<lt_proto::ModelConfig>, settings: Box<lt_proto::Settings> },
    /// 原地改目标语言（原版 set_target_language；同语言判定也用新值）
    TargetLanguage(String),
    /// 原地改超时（原版 set_timeout）
    Timeout(u32),
    /// 运行时切换 ASR 引擎/模型（原版 _switch_asr_engine；ensure_started
    /// 内部带替换+失败回滚，此处只补路由与 UI 事件）
    ReplaceEngine {
        engine: String,
        funasr_model: String,
        /// whisper 档位（M5 引擎实装后消费；funasr 路径忽略）
        #[allow(dead_code)]
        whisper_model_size: String,
        language: String,
    },
}

/// 转录写盘共享句柄（面板"应用"热切换 enabled；与 Pipeline 内部同源）
pub fn transcript_shared() -> Arc<lt_pipeline::transcript::TranscriptWriter> {
    Pipeline::transcript_handle()
}

impl Pipeline {
    /// 转录写盘句柄（进程级单例；enabled 跟随 settings.auto_save_transcript）
    fn transcript_handle() -> Arc<lt_pipeline::transcript::TranscriptWriter> {
        static HANDLE: std::sync::OnceLock<Arc<lt_pipeline::transcript::TranscriptWriter>> =
            std::sync::OnceLock::new();
        HANDLE.get_or_init(|| {
            let dir = lt_models::paths::transcripts_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("transcripts"));
            Arc::new(lt_pipeline::transcript::TranscriptWriter::new(dir))
        })
        .clone()
    }

    /// 按设置启动整条管道；模型未缓存时不阻断 UI（发 AsrUnavailable）
    pub fn start(settings: &lt_proto::Settings, proxy: EventLoopProxy<UiMsg>) -> anyhow::Result<Self> {
        // ── 转录写盘（原版 self._transcript + auto_save_transcript）──
        Self::transcript_handle().set_enabled(settings.auto_save_transcript);

        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        // 语言/padding 挂起句柄：Pipeline 存一份供 UI 线程调，ASR 线程持克隆应用
        let pending = lt_asr::AsrPendingHandle::default();

        // ── 音频：chunk 满丢旧队列 ──
        let chunk_queue = Arc::new(BoundedDropQueue::new(100));
        let mut backend = WasapiBackend::new();
        backend.start(
            settings.audio_device.clone(),
            settings.mic_device.clone(),
            chunk_queue.clone(),
        )?;

        // ── capture 线程：VAD 状态机 ──
        let vad_update: Arc<std::sync::Mutex<Option<lt_pipeline::VadSettings>>> =
            Arc::new(std::sync::Mutex::new(None));
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

        let segment_queue = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(SEGMENT_QUEUE_CAP));
        let mut threads = Vec::new();
        {
            let stop = stop.clone();
            let paused = paused.clone();
            let segment_queue = segment_queue.clone();
            let proxy = proxy.clone();
            let vad_update_capture = vad_update.clone();
            std::thread::Builder::new().name("lt-capture".into()).spawn(move || {
                // 原版 _capture_loop：monitor 直接跨线程信号（此处经 proxy 发 UI 事件，
                // vad 转换为 UI 侧 f32），段直接塞 _asr_queue 等价队列（满丢旧）
                let loop_ = CaptureLoop {
                    chunk_rx: chunk_queue,
                    segment_tx: segment_queue,
                    monitor: move |rms, vad, mic_rms| {
                        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateMonitor {
                            rms,
                            vad: vad as f32,
                            mic_rms,
                        }));
                    },
                    paused,
                    vad_update: vad_update_capture,
                };
                loop_.run(&mut vad, &stop);
            })?;
        }

        // ── 翻译装置（M3）：models 非空即构建；失败仅告警不阻断 ASR ──
        let tl = TlRig::from_settings(settings).map(Arc::new);
        let (tl_switch_tx, tl_switch_rx) = crossbeam_channel::unbounded::<TlSwitch>();

        // ── ASR 线程：Manager 独占 + 段处理 ──
        {
            let stop = stop.clone();
            let proxy = proxy.clone();
            let segment_queue = segment_queue.clone();
            let settings = settings.clone();
            let pending = pending.clone();
            let tl = tl.clone();
            threads.push(std::thread::Builder::new().name("lt-asr-main".into()).spawn(move || {
                run_asr_thread(&settings, segment_queue, pending, stop, proxy, tl, tl_switch_rx);
            })?);
        }

        tracing::info!("管道已启动（capture + VAD + ASR + 翻译）");
        Ok(Self {
            backend,
            stop,
            paused,
            pending,
            tl,
            tl_switch: Some(tl_switch_tx),
            vad_update,
            threads,
        })
    }

    /// 运行时切换翻译模型（原版 _switch_translator 的用户可见路径；
    /// ASR 线程在下一次空闲分支应用，慢/挂死的服务端不冻结 UI）
    pub fn switch_translator(&self, config: &lt_proto::ModelConfig, settings: &lt_proto::Settings) {
        let Some(tx) = &self.tl_switch else { return };
        let _ = tx.send(TlSwitch::ReplaceRig {
            config: Box::new(config.clone()),
            settings: Box::new(settings.clone()),
        });
    }

    /// 运行时改目标语言（翻译 + 悬浮窗同语言判定同步生效）
    pub fn set_translator_target_language(&self, lang: &str) {
        if let Some(tx) = &self.tl_switch {
            let _ = tx.send(TlSwitch::TargetLanguage(lang.to_string()));
        }
    }

    /// 运行时改翻译超时（原版 set_timeout）
    pub fn set_translator_timeout(&self, secs: u32) {
        if let Some(tx) = &self.tl_switch {
            let _ = tx.send(TlSwitch::Timeout(secs));
        }
    }

    /// 运行时切换 ASR 引擎/模型（原版 _switch_asr_engine 的路由；
    /// ASR 线程空闲分支执行 ensure_started，失败回滚由 Manager 内部保证）
    pub fn switch_engine(&self, engine: &str, funasr_model: &str, whisper_model_size: &str, language: &str) {
        if let Some(tx) = &self.tl_switch {
            let _ = tx.send(TlSwitch::ReplaceEngine {
                engine: engine.to_string(),
                funasr_model: funasr_model.to_string(),
                whisper_model_size: whisper_model_size.to_string(),
                language: language.to_string(),
            });
        }
    }

    /// 运行时切换采集设备（原版 set_audio_device；后端线程内重启）
    pub fn set_audio_device(&mut self, choice: lt_proto::AudioDeviceChoice) {
        let dev = match choice {
            lt_proto::AudioDeviceChoice::SystemDefault => None,
            lt_proto::AudioDeviceChoice::Named(n) => Some(n),
            lt_proto::AudioDeviceChoice::Disabled => Some("__disabled__".into()),
        };
        self.backend.set_device(dev);
    }

    /// 运行时切换麦克风（原版 set_mic_device；None=禁用）
    pub fn set_mic_device(&mut self, choice: lt_proto::MicDeviceChoice) {
        let dev = match choice {
            lt_proto::MicDeviceChoice::Off => None,
            lt_proto::MicDeviceChoice::Default => Some("__default__".into()),
            lt_proto::MicDeviceChoice::Named(n) => Some(n),
        };
        self.backend.set_mic_device(dev);
    }

    /// VAD 参数热更新（原版面板 apply → vad_processor.update_settings）：
    /// 塞入信号槽，capture 线程下一 chunk 应用
    pub fn update_vad_settings(&self, s: lt_pipeline::VadSettings) {
        *self.vad_update.lock().unwrap() = Some(s);
    }

    #[allow(dead_code)]
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    /// UI 线程调用：挂起 ASR 识别语言；ASR 线程在下一次 transcribe 前应用并提交
    /// （原版 _set_asr_language + _apply_pending_asr_settings；仅加锁存值，
    /// 绝不跨进程调用，慢/挂死的 worker 不会冻结 UI）
    #[allow(dead_code)]
    pub fn set_pending_language(&self, lang: &str) {
        self.pending.set_language(lang);
    }

    /// UI 线程调用：按引擎家族（"funasr"/"whisper"）挂起 padding；ASR 线程在下一次
    /// transcribe 前应用（原版 _set_asr_padding + _apply_pending_asr_settings）
    #[allow(dead_code)]
    pub fn set_pending_padding(&self, engine_family: &str, secs: f32) {
        self.pending.set_padding(engine_family, secs);
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.backend.stop();
        if let Some(tl) = &self.tl {
            tl.shutdown();
        }
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

/// 引擎/模型键 → (worker 配置, 显示名)；未缓存或引擎未实装 → None。
/// 启动路径与运行时切换（TlSwitch::ReplaceEngine）共用。
fn build_worker_config(
    models_dir: &std::path::Path,
    engine: &str,
    funasr_model: &str,
    pad_seconds: f32,
    language: &str,
) -> Option<(WorkerConfig, String)> {
    match engine {
        "funasr" => {
            let (entry, _fell_back) = resolve_funasr_entry(funasr_model);
            let model_dir = lt_models::cache::local_model_dir(models_dir, &entry)?;
            Some((
                WorkerConfig {
                    engine: "sensevoice".into(),
                    display_name: entry.display.into(),
                    language: language.to_string(),
                    pad_seconds: Some(pad_seconds),
                    options: serde_json::json!({ "model_dir": model_dir.to_string_lossy() }),
                },
                entry.display.into(),
            ))
        }
        // whisper 引擎 M5 实装；此前的"无视 engine 恒起 sensevoice"改为如实不可用
        other => {
            tracing::warn!("引擎 {other:?} 尚未实装（M5），无法启动 worker");
            None
        }
    }
}

/// ASR 线程：模型就绪则循环识别；未缓存则发 AsrUnavailable 后待命
fn run_asr_thread(
    settings: &lt_proto::Settings,
    segment_queue: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
    pending: lt_asr::AsrPendingHandle,
    stop: Arc<AtomicBool>,
    proxy: EventLoopProxy<UiMsg>,
    mut tl: Option<Arc<TlRig>>,
    tl_switch: crossbeam_channel::Receiver<TlSwitch>,
) {
    // 目标语言的运行时快照（同语言判定用；TlSwitch::TargetLanguage 同步更新）
    let mut target_language = settings.target_language.clone();
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
    let worker = build_worker_config(
        &models_dir,
        &settings.asr_engine,
        &settings.funasr_model,
        settings.sensevoice_pad_seconds,
        &settings.asr_language,
    );
    let Some((config, display)) = worker else {
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

    // Manager 持 UI 线程同款挂起句柄：transcribe 前应用挂起的语言/padding
    let mut manager = AsrManager::with_pending(pending);
    // 模型加载对话框（原版 _ModelLoadDialog：装载期模态；AsrDevice/AsrUnavailable 关闭）
    let _ = proxy.send_event(UiMsg::Event(UiEvent::ModelLoadStart(display.clone())));
    if let Err(e) = manager.ensure_started(&config) {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
        tracing::error!("ASR worker 启动失败: {e}");
    } else {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
            "{display} [cpu]"
        ))));
    }

    while !stop.load(Ordering::Relaxed) {
        // 取段：capture 线程直塞的 (source, audio)（source 目前仅 VadFlush，
        // M6 interim 接入后再分流）
        let Some((source, audio)) = segment_queue.pop_timeout(Duration::from_millis(500)) else {
            // 空闲分支：RSS 回收（原版 _asr_loop queue.Empty）+ 翻译器切换命令
            manager.maybe_recycle_if_idle();
            while let Ok(sw) = tl_switch.try_recv() {
                match sw {
                    TlSwitch::ReplaceRig { config, settings } => {
                        if let Some(rig) = TlRig::from_model_config(&config, &settings) {
                            tracing::info!("翻译器已切换: {} ({})", config.name, config.model);
                            tl = Some(Arc::new(rig));
                        }
                    }
                    TlSwitch::TargetLanguage(lang) => {
                        target_language = lang.clone();
                        if let Some(rig) = &tl {
                            rig.translator.set_target_language(&lang);
                        }
                    }
                    TlSwitch::Timeout(secs) => {
                        if let Some(rig) = &tl {
                            rig.translator.set_timeout(secs);
                        }
                    }
                    TlSwitch::ReplaceEngine { engine, funasr_model, whisper_model_size: _, language } => {
                        // 原版 _switch_asr_engine：装配新配置 → 加载对话框 →
                        // ensure_started（失败内部回滚旧 worker）→ 设备/不可用事件
                        match build_worker_config(
                            &models_dir,
                            &engine,
                            &funasr_model,
                            settings.sensevoice_pad_seconds,
                            &language,
                        ) {
                            Some((config, display)) => {
                                let _ = proxy.send_event(UiMsg::Event(UiEvent::ModelLoadStart(display.clone())));
                                if let Err(e) = manager.ensure_started(&config) {
                                    let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
                                    tracing::error!("引擎切换失败（已回滚）: {e}");
                                } else {
                                    let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
                                        "{display} [cpu]"
                                    ))));
                                    tracing::info!("引擎已切换: {engine}/{funasr_model}");
                                }
                            }
                            None => {
                                let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
                                tracing::warn!("引擎切换目标不可用（未缓存/未知）: {engine}/{funasr_model}");
                            }
                        }
                    }
                }
            }
            continue;
        };
        let _ = source;
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
                let original_text = result.text;
                let source_lang = result.language;
                let _ = proxy.send_event(UiMsg::Event(UiEvent::AddMessage {
                    id,
                    timestamp: timestamp.clone(),
                    original: original_text.clone(),
                    lang: source_lang.clone(),
                    asr_ms,
                }));

                // ── 翻译分流（原版 _process_segment 尾部；字幕窗 extra_langs 随 M4 接入）──
                if let Some(rig) = &tl {
                    rig.stats.asr_count.fetch_add(1, Ordering::Relaxed);
                    rig.transcript.write_original(id, &timestamp, &original_text);
                    if source_lang == target_language {
                        tracing::info!("Same language ({source_lang}), no translation");
                        rig.transcript.finalize_no_translation(id);
                        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
                            id,
                            text: String::new(),
                            tl_ms: 0.0,
                        }));
                        let _ = proxy.send_event(UiMsg::Event(rig.stats.snapshot_event()));
                    } else {
                        rig.submit_translation(&proxy, id, original_text, source_lang);
                    }
                }
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

    // ── TlRig::from_settings：翻译装置构建（M3 装配） ──

    #[test]
    fn tl_rig_builds_from_default_settings() {
        let settings = lt_proto::Settings::default();
        let rig = TlRig::from_settings(&settings);
        assert!(rig.is_some(), "默认 settings 带一个默认模型，应能构建");
        // 目标语言来自全局设置而非模型配置
        assert_eq!(rig.unwrap().translator.target_language(), settings.target_language);
    }

    #[test]
    fn tl_rig_none_when_active_model_out_of_bounds() {
        let mut settings = lt_proto::Settings::default();
        settings.active_model = 99;
        assert!(TlRig::from_settings(&settings).is_none());
    }

    #[test]
    fn tl_rig_none_when_models_empty() {
        let mut settings = lt_proto::Settings::default();
        settings.models.clear();
        assert!(TlRig::from_settings(&settings).is_none());
    }

    #[test]
    fn job_pool_drops_jobs_after_shutdown() {
        use std::sync::atomic::AtomicU64;
        let pool = JobPool::new(2);
        pool.shutdown();
        // shutdown 之后的提交不执行（submit 侧短路 + worker 侧双重检查）
        let ran = Arc::new(AtomicU64::new(0));
        let r = ran.clone();
        pool.submit(move || {
            r.fetch_add(1, Ordering::Relaxed);
        });
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(ran.load(Ordering::Relaxed), 0);
    }

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
