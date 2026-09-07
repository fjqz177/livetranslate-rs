//! 管道装配（M2.6；原版 main.py 启动流等价）：
//!
//! 音频线程（wasapi 后端内部）→ 16k mono chunk 满丢旧队列
//! → capture 线程（CaptureLoop：RMS/VAD 监视 + VAD 状态机 + 超时静音推进
//!   + 增量触发判定），与原版 _capture_loop 一致、无中间分流线程：
//!     - monitor 数据：capture 线程内经 EventLoopProxy 直接推 UI
//!       （等价原版跨线程调 update_monitor，每 chunk ≈31/s）
//!     - 语音段：capture 线程直接入满丢旧段队列（等价原版 _enqueue_asr）
//!     - 增量触发：VAD 独占改共享（Arc<Mutex>，锁粒度=单次方法调用，对齐
//!       原版 _vad_lock）；条件满足塞 Interim 空标记，不在 capture 线程跑 ASR
//!
//! → ASR 线程：Manager 独占 + 段处理；空闲时 RSS 回收（原版 _asr_loop
//!   queue.Empty 分支）+ 翻译器切换命令。
//!   分流（原版 _asr_loop）：Interim 标记 → 排空重复 → 锁 VAD peek/识别/裁剪
//!   （`_do_interim_asr`）；VadFlush → interim 激活时走回声剥离+pending 拼接
//!   收尾（`_process_interim_final`），否则段级三层过滤（`_process_segment`，
//!   空/纯标点 → 噪声 → 语言）→ AddMessage
//!   → 同语言直接回空译文；否则提交翻译线程池（M3；原版 _tl_executor，
//!   max_workers=8）→ UpdateStreaming/UpdateTranslation/UpdateStats。
//!
//! 未就绪链路：模型未缓存 → 发 AsrUnavailable（M2.5 向导接管首启下载）。

use lt_asr::{AsrManager, WorkerConfig};
use lt_models::registry;
use lt_pipeline::audio::wasapi_win::WasapiBackend;
use lt_pipeline::interim::{
    is_short_utterance, pending_merge, split_sentences, strip_committed_overlap, trim_samples,
    InterimState,
};
use lt_pipeline::{
    AudioBackend, BoundedDropQueue, CaptureLoop, InterimControl, SegmentSource, VadProcessor,
    VadSettings,
};
use lt_proto::{UiEvent, UiMsg};
use lt_translate::Translator;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
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
    /// 按设置构建；models 为空/active_model 越界 → Ok(None)（不翻译，仅 ASR）；
    /// 配置无效（URL 格式错等）→ Err(原因)（必须让用户可见，见 TranslatorUnavailable）
    fn from_settings(settings: &lt_proto::Settings) -> Result<Option<Self>, String> {
        let Some(mc) = settings.models.get(settings.active_model) else {
            return Ok(None);
        };
        Self::from_model_config(mc, settings)
    }

    /// 按指定模型配置构建（运行时切换用；构建失败返回 Err——UI 收到
    /// TranslatorUnavailable 显示到翻译页状态行，不再静默关闭整条翻译）
    fn from_model_config(mc: &lt_proto::ModelConfig, settings: &lt_proto::Settings) -> Result<Option<Self>, String> {
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
                return Err(format!("{}: {e:#}", mc.name));
            }
        };
        let stats = Arc::new(TlStats::new(mc.input_price, mc.output_price));
        tracing::info!(
            "Switching translator: {} ({})",
            mc.name,
            mc.model
        );
        Ok(Some(Self {
            translator,
            stats,
            pool: JobPool::new(TL_POOL_WORKERS),
            transcript: Pipeline::transcript_handle(),
        }))
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
    /// 增量识别控制块（与 capture/ASR 线程共享；set_interim 热应用）
    interim: Arc<InterimControl>,
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
        /// whisper 档位（builtin 六档 | 本地 GGML 路径）
        whisper_model_size: String,
        language: String,
    },
    /// 翻译配置「测试连接」：临时装置发一次最简请求后回执 TestTranslatorResult
    TestTranslator { name: String, config: Box<lt_proto::ModelConfig> },
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
        // VAD 共享拓扑（原版 _vad_lock）：capture 写、ASR 线程增量识别时
        // peek/trim/speech_samples 读，锁粒度 = 单次方法调用
        let vad = Arc::new(Mutex::new(vad));
        let interim = Arc::new(InterimControl::default());
        // 启动即按持久化设置就位（原版 _incremental_enabled/_interim_interval 随启动初始化）
        interim.set(settings.incremental_asr, settings.interim_interval);
        let mut threads = Vec::new();
        {
            let stop = stop.clone();
            let paused = paused.clone();
            let segment_queue = segment_queue.clone();
            let proxy = proxy.clone();
            let vad_update_capture = vad_update.clone();
            let vad = vad.clone();
            let interim = interim.clone();
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
                    interim,
                };
                loop_.run(&vad, &stop);
            })?;
        }

        // ── 翻译装置（M3）：models 非空即构建；配置无效必须让用户可见
        //（TranslatorUnavailable → 面板翻译页状态行 + 悬浮窗译文占位）──
        let tl = match TlRig::from_settings(settings) {
            Ok(t) => t.map(Arc::new),
            Err(reason) => {
                let _ = proxy.send_event(UiMsg::Event(UiEvent::TranslatorUnavailable {
                    reason: reason.clone(),
                }));
                tracing::error!("翻译装置不可用: {reason}");
                None
            }
        };
        let (tl_switch_tx, tl_switch_rx) = crossbeam_channel::unbounded::<TlSwitch>();

        // ── ASR 线程：Manager 独占 + 段处理 ──
        {
            let stop = stop.clone();
            let proxy = proxy.clone();
            let segment_queue = segment_queue.clone();
            let settings = settings.clone();
            let pending = pending.clone();
            let tl = tl.clone();
            let vad = vad.clone();
            let interim = interim.clone();
            threads.push(std::thread::Builder::new().name("lt-asr-main".into()).spawn(move || {
                run_asr_thread(
                    &settings,
                    AsrThreadCtx { segment_queue, vad, interim, pending, stop, proxy, tl_switch: tl_switch_rx },
                    tl,
                );
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
            interim,
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

    /// 翻译配置「测试连接」（设置页翻译按钮）：ASR 线程空闲分支执行，
    /// 结果经 TestTranslatorResult 回执 UI；不改变当前活动翻译装置
    pub fn test_translator(&self, config: &lt_proto::ModelConfig) {
        if let Some(tx) = &self.tl_switch {
            let _ = tx.send(TlSwitch::TestTranslator {
                name: config.name.clone(),
                config: Box::new(config.clone()),
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

    /// 增量识别开关/间隔热应用（原版 _incremental_asr_cb → _incremental_enabled/
    /// _interim_interval）：写共享控制块，capture 线程下一 chunk 生效；
    /// 关闭时清进度计数，重开从零起算
    pub fn set_interim(&self, enabled: bool, interval: f32) {
        self.interim.set(enabled, interval);
        tracing::info!("增量识别: {enabled}（间隔 {interval}s）");
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

/// whisper 档位 → (模型 .bin 路径, 显示名)。
/// builtin 档：cache::whisper_model_path（snapshot 字典序最后，E-10）；
/// 非 builtin 值视为本地 GGML 路径，文件存在即用（显示名取文件 stem）。
fn resolve_whisper_model(
    models_dir: &std::path::Path,
    whisper_model: &str,
) -> Option<(std::path::PathBuf, String)> {
    match registry::whisper_entry_for(whisper_model) {
        Some(entry) => {
            let p = lt_models::cache::whisper_model_path(models_dir, whisper_model)?;
            Some((p, format!("Whisper {}", entry.key)))
        }
        None => {
            let p = std::path::PathBuf::from(whisper_model);
            p.is_file().then(|| {
                let display = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| whisper_model.to_string());
                (p, display)
            })
        }
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
    whisper_model: &str,
    whisper_pad: f32,
) -> Option<(WorkerConfig, String)> {
    match engine {
        "funasr" => {
            let (entry, _fell_back) = resolve_funasr_entry(funasr_model);
            let model_dir = lt_models::cache::local_model_dir(models_dir, &entry)?;
            // WP-A：nano 为独立 worker 引擎（LLM 解码、无 padding 语义）；
            // sensevoice 维持原路径
            let (engine, pad) = if entry.key == "funasr-nano-2512" {
                ("nano", None)
            } else {
                ("sensevoice", Some(pad_seconds))
            };
            Some((
                WorkerConfig {
                    engine: engine.into(),
                    display_name: entry.display.into(),
                    language: language.to_string(),
                    pad_seconds: pad,
                    options: serde_json::json!({ "model_dir": model_dir.to_string_lossy() }),
                },
                entry.display.into(),
            ))
        }
        "whisper" => {
            // M5.1：whisper_model_size 为 builtin 档（缓存 snapshot 解析 .bin）
            // 或本地 GGML 路径；未缓存 → None（发 AsrUnavailable，等向导/下载）
            let (model_path, display) = resolve_whisper_model(models_dir, whisper_model)?;
            Some((
                WorkerConfig {
                    engine: "whisper".into(),
                    display_name: display.clone(),
                    language: language.to_string(),
                    pad_seconds: Some(whisper_pad),
                    options: serde_json::json!({ "model_path": model_path.to_string_lossy() }),
                },
                display,
            ))
        }
        "qwen3" => {
            // WP-B：单一模型（B-α，settings 无独立模型键）；无 padding 语义
            let entry = registry::qwen3_entry();
            let model_dir = lt_models::cache::local_model_dir(models_dir, &entry)?;
            Some((
                WorkerConfig {
                    engine: "qwen3".into(),
                    display_name: entry.display.into(),
                    language: language.to_string(),
                    pad_seconds: None,
                    options: serde_json::json!({ "model_dir": model_dir.to_string_lossy() }),
                },
                entry.display.into(),
            ))
        }
        other => {
            tracing::warn!("引擎 {other:?} 未实装，无法启动 worker");
            None
        }
    }
}

/// 引擎切换日志的模型键：whisper 打档位、qwen3 打固定键（settings 无独立键）、
/// funasr 打 funasr_model——打错键会误导诊断。
fn engine_model_key<'a>(engine: &str, funasr_model: &'a str, whisper_model: &'a str) -> &'a str {
    match engine {
        "whisper" => whisper_model,
        "qwen3" => "qwen3-asr-0.6b",
        _ => funasr_model,
    }
}

/// ASR 线程上下文（Pipeline::start 一次性装配的共享件）
struct AsrThreadCtx {
    segment_queue: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
    /// 与 capture 线程共享的 VAD（增量识别锁内 peek/trim，原版 _vad_lock）
    vad: Arc<Mutex<VadProcessor>>,
    /// 增量识别跨线程控制块（capture 写触发时间、本线程写消费进度）
    interim: Arc<InterimControl>,
    pending: lt_asr::AsrPendingHandle,
    stop: Arc<AtomicBool>,
    proxy: EventLoopProxy<UiMsg>,
    tl_switch: crossbeam_channel::Receiver<TlSwitch>,
}

/// 翻译器域命令的共享路由（AH-1/H1）：待命循环与主循环空闲分支共用的四臂
/// （ReplaceRig/TargetLanguage/Timeout/TestTranslator）。`ReplaceEngine` 的引擎
/// 处理两侧不同——待命态只重试装配，主循环 ensure_started+事件+回滚——原样
/// 透传给调用方自行处理。
fn route_translator_switch(
    sw: TlSwitch,
    tl: &mut Option<Arc<TlRig>>,
    target_language: &mut String,
    proxy: &EventLoopProxy<UiMsg>,
    settings: &lt_proto::Settings,
) -> Option<TlSwitch> {
    match sw {
        TlSwitch::ReplaceRig { config, settings } => {
            match TlRig::from_model_config(&config, &settings) {
                Ok(Some(rig)) => {
                    tracing::info!("翻译器已切换: {} ({})", config.name, config.model);
                    *tl = Some(Arc::new(rig));
                }
                Ok(None) => {
                    tracing::warn!("翻译器切换目标为空（models 空/越界），保持当前装置");
                }
                Err(reason) => {
                    let _ = proxy.send_event(UiMsg::Event(UiEvent::TranslatorUnavailable {
                        reason,
                    }));
                }
            }
            None
        }
        TlSwitch::TargetLanguage(lang) => {
            *target_language = lang.clone();
            if let Some(rig) = tl {
                rig.translator.set_target_language(&lang);
            }
            None
        }
        TlSwitch::Timeout(secs) => {
            if let Some(rig) = tl {
                rig.translator.set_timeout(secs);
            }
            None
        }
        TlSwitch::TestTranslator { name, config } => {
            // 构建临时装置（不切换活动翻译器），发一次最简请求回执 UI
            let mut test_settings = settings.clone();
            test_settings.target_language = target_language.clone();
            match TlRig::from_model_config(&config, &test_settings) {
                Ok(Some(rig)) => {
                    let proxy = proxy.clone();
                    let name = name.clone();
                    rig.pool.submit(move || {
                        let t0 = Instant::now();
                        let mut it =
                            rig.translator.translate_iter("Livetranslate test", "auto");
                        let (ok, err, ms) = match it.next() {
                            Some(Ok(_partial)) => (true, None, t0.elapsed().as_millis() as u64),
                            Some(Err(e)) => (false, Some(e.ui_text()), t0.elapsed().as_millis() as u64),
                            None => (false, Some(lt_i18n::t("test_translator_no_response")), t0.elapsed().as_millis() as u64),
                        };
                        let _ = proxy.send_event(UiMsg::Event(
                            UiEvent::TestTranslatorResult { name, ok, error: err, ms },
                        ));
                    });
                }
                Ok(None) => {
                    let _ = proxy.send_event(UiMsg::Event(UiEvent::TestTranslatorResult {
                        name,
                        ok: false,
                        error: Some(lt_i18n::t("test_translator_no_config").into()),
                        ms: 0,
                    }));
                }
                Err(reason) => {
                    let _ = proxy.send_event(UiMsg::Event(UiEvent::TestTranslatorResult {
                        name,
                        ok: false,
                        error: Some(reason),
                        ms: 0,
                    }));
                }
            }
            None
        }
        engine @ TlSwitch::ReplaceEngine { .. } => Some(engine),
    }
}

/// ASR 线程：模型就绪则循环识别；未缓存则发 AsrUnavailable 后待命
fn run_asr_thread(
    settings: &lt_proto::Settings,
    ctx: AsrThreadCtx,
    mut tl: Option<Arc<TlRig>>,
) {
    let AsrThreadCtx { segment_queue, vad, interim, pending, stop, proxy, tl_switch } = ctx;
    // 目标语言的运行时快照（同语言判定用；TlSwitch::TargetLanguage 同步更新）
    let mut target_language = settings.target_language.clone();
    // 增量识别会话状态（原版 _interim_* 字段；跨段存活，vad_flush 复位）
    let mut interim_state = InterimState::default();
    // 构造 worker 配置（当前仅 sensevoice；whisper M5）
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
            tracing::error!("模型目录不可用: {e}");
            return;
        }
    };
    // 模型键 → 条目；mlt/非法键回退 sensevoice-small（不阻断 UI，也不得用 nano 冒充）。
    // 诊断仅 funasr 引擎相关：whisper/qwen3 启动诊断走 build_worker_config 对应分支
    let (entry, fell_back) = if settings.asr_engine == "funasr" {
        resolve_funasr_entry(&settings.funasr_model)
    } else if settings.asr_engine == "qwen3" {
        // WP-B：诊断用 qwen3 自身条目（否则未缓存日志打错模型名）
        (registry::qwen3_entry(), false)
    } else {
        (registry::SENSEVOICE_SMALL.clone(), false)
    };
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
    let mut worker = build_worker_config(
        &models_dir,
        &settings.asr_engine,
        &settings.funasr_model,
        settings.sensevoice_pad_seconds,
        &settings.asr_language,
        &settings.whisper_model_size,
        settings.whisper_pad_seconds,
    );
    if worker.is_none() {
        let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
        tracing::warn!("ASR 模型未缓存（{entry:?}），进入待命态（AH-1）");
    }
    // 待命 = 可唤醒状态而非死胡同（AH-1/H1，P0）：模型就绪前吞掉段，但必须
    // 消费 tl_switch——运行时下载完成后 shell 重发 SwitchEngine（shell.rs
    // DownloadSucceeded 分支），收到即用挂起参数重装配并落回正常装配路径。
    // 原实现只排段不消费命令，唤醒信号无人接，新用户首启旅程必须重启应用。
    while worker.is_none() && !stop.load(Ordering::Relaxed) {
        let _ = segment_queue.pop_timeout(Duration::from_secs(1));
        while let Ok(sw) = tl_switch.try_recv() {
            let Some(TlSwitch::ReplaceEngine { engine, funasr_model, whisper_model_size, language }) =
                route_translator_switch(sw, &mut tl, &mut target_language, &proxy, settings)
            else {
                continue;
            };
            let model_key = engine_model_key(&engine, &funasr_model, &whisper_model_size);
            match build_worker_config(
                &models_dir,
                &engine,
                &funasr_model,
                settings.sensevoice_pad_seconds,
                &language,
                &whisper_model_size,
                settings.whisper_pad_seconds,
            ) {
                Some((config, display)) => {
                    tracing::info!("待命中模型已就绪: {engine}/{model_key}，退出待命装配 worker");
                    worker = Some((config, display));
                }
                None => {
                    let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
                    tracing::warn!("待命中切换目标仍未缓存: {engine}/{model_key}，继续待命");
                }
            }
        }
    }
    let Some((config, display)) = worker else {
        tracing::info!("ASR 待命中管道停止");
        return;
    };

    // Manager 持 UI 线程同款挂起句柄：transcribe 前应用挂起的语言/padding
    let mut manager = AsrManager::with_pending(pending);
    // 当前生效引擎的显示标签（切换失败回滚后用于恢复 AsrDevice，避免
    // 状态行挂着「ASR unavailable」而旧引擎实际仍在工作——P0-4）
    let mut current_display = display.clone();
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
            // （AH-1：翻译器四臂经 route_translator_switch 与待命循环共享）
            manager.maybe_recycle_if_idle();
            while let Ok(sw) = tl_switch.try_recv() {
                if let Some(TlSwitch::ReplaceEngine { engine, funasr_model, whisper_model_size, language }) =
                    route_translator_switch(sw, &mut tl, &mut target_language, &proxy, settings)
                {
                    // 原版 _switch_asr_engine：装配新配置 → 加载对话框 →
                    // ensure_started（失败内部回滚旧 worker）→ 设备/不可用事件
                    match build_worker_config(
                        &models_dir,
                        &engine,
                        &funasr_model,
                        settings.sensevoice_pad_seconds,
                        &language,
                        &whisper_model_size,
                        settings.whisper_pad_seconds,
                    ) {
                        Some((config, display)) => {
                            let _ = proxy.send_event(UiMsg::Event(UiEvent::ModelLoadStart(display.clone())));
                            if let Err(e) = manager.ensure_started(&config) {
                                // 回滚后旧 worker 仍在工作：恢复旧标签而非
                                // 发 AsrUnavailable（避免状态与行为矛盾，P0-4）
                                let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
                                    "{} [cpu]",
                                    current_display
                                ))));
                                tracing::error!("引擎切换失败（已回滚）: {e}");
                            } else {
                                current_display = display.clone();
                                let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
                                    "{display} [cpu]"
                                ))));
                                // 日志按引擎打实际模型键（whisper 打 funasr_model 会误导诊断）
                                let model_key =
                                    engine_model_key(&engine, &funasr_model, &whisper_model_size);
                                tracing::info!("引擎已切换: {engine}/{model_key}");
                            }
                        }
                        None => {
                            // 未缓存/未知档：旧引擎继续运行——不发
                            // AsrUnavailable（P0-4），恢复标签并落日志；
                            // 面板侧缓存卡片「未缓存 + 下载按钮」给出下一步
                            let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrDevice(format!(
                                "{} [cpu]",
                                current_display
                            ))));
                            let model_key =
                                engine_model_key(&engine, &funasr_model, &whisper_model_size);
                            tracing::warn!(
                                "切换目标未缓存/未知，保持当前引擎: {engine}/{model_key}（去识别页下载）"
                            );
                        }
                    }
                }
            }
            continue;
        };
        match source {
            SegmentSource::Interim => {
                // 增量通道（原版 main.py:1720-1724）：排空重复标记 → 锁 VAD
                // peek/识别/裁剪 → 更新消费进度（capture 线程的 elapsed 由此起算）
                drain_interim_duplicates(&segment_queue);
                run_interim_pass(
                    &mut manager,
                    &vad,
                    &mut interim_state,
                    settings,
                    &target_language,
                    tl.as_deref(),
                    &proxy,
                );
                let samples = { vad.lock().unwrap().speech_samples() };
                interim.last_interim_samples.store(samples as u64, Ordering::Relaxed);
            }
            SegmentSource::VadFlush => {
                if audio.is_empty() {
                    continue;
                }
                let seg_seconds = audio.len() as f64 / lt_pipeline::TARGET_RATE as f64;
                let t0 = std::time::Instant::now();
                match manager.transcribe(&audio, false) {
                    Ok(result) => {
                        let asr_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        if interim_state.active {
                            // 收尾段（原版 _process_interim_final 的 Ok(result) 分支）：
                            // 回声剥离 → pending 前置拼接 → 噪声过滤 → 提交
                            commit_interim_final(
                                &mut interim_state,
                                settings,
                                &target_language,
                                tl.as_deref(),
                                &proxy,
                                &result.text,
                                &result.language,
                                asr_ms,
                                seg_seconds,
                            );
                        } else if let Some(reason) = reject_segment(
                            &result.text,
                            seg_seconds,
                            &settings.asr_language,
                            &result.language,
                        ) {
                            // 段级三层过滤（原版 _process_segment：空/纯标点 → 噪声 → 语言）
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
                        } else {
                            commit_text(
                                settings,
                                &target_language,
                                tl.as_deref(),
                                &proxy,
                                &result.text,
                                &result.language,
                                asr_ms,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("ASR 段识别失败: {e}");
                        if e.unavailable() {
                            let _ = proxy.send_event(UiMsg::Event(UiEvent::AsrUnavailable));
                        }
                    }
                }
                // 无论走哪支，处理完收尾段后复位全部增量状态（原版 main.py:1715-1719）
                interim_state.reset();
                interim.last_interim_samples.store(0, Ordering::Relaxed);
                interim.last_check_ms.store(0, Ordering::Relaxed);
            }
        }
    }
    manager.shutdown();
    tracing::info!("ASR 线程退出");
}

/// 排空队列头部连续的 interim 标记（原版 `_drain_interim_duplicates`）：
/// 标记积压时只处理最早一枚（调用方已弹出），其余直接丢弃——重复识别同一
/// 缓冲只产出空结果，纯烧 CPU。首个非 interim 项回插队首保序。
fn drain_interim_duplicates(queue: &BoundedDropQueue<(SegmentSource, Vec<f32>)>) {
    while let Some((source, audio)) = queue.try_pop() {
        match source {
            SegmentSource::Interim => continue,
            other => {
                queue.push_front((other, audio));
                break;
            }
        }
    }
}

/// 增量通道（对照原版 `_do_interim_asr` 逐条）：锁 VAD peek → 短缓冲跳过 →
/// 识别 → 回声剥离 → 分句 → 提交前 n-1 句（短句 ≤8 字母数字进 pending 拼接）
/// → 比例裁剪已消费音频 → 更新 tail/active。返回是否提交了句子。
fn run_interim_pass(
    manager: &mut AsrManager,
    vad: &Arc<Mutex<VadProcessor>>,
    st: &mut InterimState,
    settings: &lt_proto::Settings,
    target_language: &str,
    tl: Option<&TlRig>,
    proxy: &EventLoopProxy<UiMsg>,
) -> bool {
    // ① 锁内 peek，立即解锁（原版 with self._vad_lock: peek_buffer）
    let peek = vad.lock().unwrap().peek_buffer();
    let Some((audio, duration)) = peek else {
        return false;
    };
    if duration < 1.5 {
        return false;
    }
    // ② 识别（原版 use_word_ts=False：词级时间戳对重复增量通道太贵）
    let t0 = Instant::now();
    let Ok(result) = manager.transcribe(&audio, false) else {
        tracing::warn!("Interim ASR 识别失败，跳过本轮（收尾段不受影响）");
        return false;
    };
    let asr_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let full_raw = result.text.trim();
    if full_raw.is_empty() || !full_raw.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    // ③ 回声剥离（与上次提交的尾部重叠）
    let full_text = strip_committed_overlap(full_raw, &st.committed_tail);
    if full_text.is_empty() {
        return false;
    }
    // ④ 分句；仅一句 → 末句仍在说，不提交
    let sentences = split_sentences(&full_text, &result.language);
    if sentences.len() <= 1 {
        return false;
    }
    let complete = &sentences[..sentences.len() - 1];
    let committed_text: String = complete.concat();
    if committed_text.trim().is_empty() {
        return false;
    }
    // ⑤ 比例裁剪量（原版 use_word_ts=False 分支；公式已在 interim::trim_samples 移植）
    let trim = trim_samples(
        audio.len(),
        committed_text.chars().count(),
        full_text.chars().count(),
        lt_pipeline::TARGET_RATE as usize,
    );
    // ⑥ 提交完整句；短句进 pending 等下句前置拼接（无分隔符，原版同）
    let mut committed = false;
    for sent in complete {
        let text = sent.trim();
        if text.is_empty() {
            continue;
        }
        if is_short_utterance(text) {
            st.pending = pending_merge(&st.pending, text);
            tracing::debug!("Interim 短句缓冲: {text:?}，pending={:?}", st.pending);
            continue;
        }
        let text = pending_merge(&st.pending, text);
        st.pending.clear();
        commit_text(settings, target_language, tl, proxy, &text, &result.language, asr_ms);
        committed = true;
    }
    if !committed {
        return false;
    }
    // ⑦ 裁剪已消费音频 + 记录 tail（末 50 字符）+ 置 active
    if trim > 0 {
        vad.lock().unwrap().trim_front(trim);
    }
    let tail_start = committed_text
        .char_indices()
        .rev()
        .nth(49)
        .map(|(i, _)| i)
        .unwrap_or(0);
    st.committed_tail = committed_text[tail_start..].to_string();
    st.active = true;
    tracing::info!(
        "Interim ASR: committed {} sentence(s), trimmed {:.2}s",
        complete.len(),
        trim as f64 / lt_pipeline::TARGET_RATE as f64
    );
    true
}

/// 收尾段提交（原版 `_process_interim_final` 的 Ok(result) 分支 1:1）：
/// 回声剥离 → pending 前置拼接 → 空/纯标点检查 → 噪声过滤（与 _process_segment
/// 同阈值，但 interim 完整句不含此层）→ 走 [`commit_text`]（内部再做空/语言过滤，
/// 对齐原版 `_process_segment_text` 的重复防线）。Err 分支在调用方（≈原版异常
/// 路径：不冲刷 pending，随复位丢弃）。
#[allow(clippy::too_many_arguments)]
fn commit_interim_final(
    st: &mut InterimState,
    settings: &lt_proto::Settings,
    target_language: &str,
    tl: Option<&TlRig>,
    proxy: &EventLoopProxy<UiMsg>,
    raw_text: &str,
    lang: &str,
    asr_ms: f64,
    seg_seconds: f64,
) {
    let stripped = strip_committed_overlap(raw_text.trim(), &st.committed_tail);
    let mut text = pending_merge(&st.pending, &stripped);
    st.pending.clear();
    text = text.trim().to_string();
    let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
    if text.is_empty() || alnum == 0 {
        return;
    }
    if seg_seconds >= 2.0 && alnum <= 3 {
        tracing::debug!("噪声过滤: {seg_seconds:.1}s 段仅产出 {text:?}，跳过");
        return;
    }
    commit_text(settings, target_language, tl, proxy, &text, lang, asr_ms);
}

/// 提交一条已确定文本（原版 `_process_segment_text` 尾部等价）：AddMessage →
/// 统计/转写落盘 → 同语言直显空译文或提交翻译。开头复做空/纯标点与语言过滤
/// （原版 `_process_segment_text` 即如此；vad_flush 整段路径调用前已过三层过滤，
/// 此处检查冗余但无害——1:1 保留原版的双层防线）。
#[allow(clippy::too_many_arguments)]
fn commit_text(
    settings: &lt_proto::Settings,
    target_language: &str,
    tl: Option<&TlRig>,
    proxy: &EventLoopProxy<UiMsg>,
    text: &str,
    lang: &str,
    asr_ms: f64,
) {
    let original_text = text.trim();
    if original_text.is_empty() || !original_text.chars().any(|c| c.is_alphanumeric()) {
        tracing::debug!("ASR 返回空/纯标点结果，跳过: {original_text:?}");
        return;
    }
    if settings.asr_language != "auto" && lang != settings.asr_language {
        // 预览按字符截断，避免切坏 UTF-8 边界
        let preview: String = original_text.chars().take(60).collect();
        tracing::info!(
            "语言过滤: 期望 {:?} 但识别为 {:?}，丢弃: {preview}",
            settings.asr_language,
            lang
        );
        return;
    }
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    let id = uuid::Uuid::new_v4().as_u128() as u64;
    let _ = proxy.send_event(UiMsg::Event(UiEvent::AddMessage {
        id,
        timestamp: timestamp.clone(),
        original: original_text.to_string(),
        lang: lang.to_string(),
        asr_ms,
    }));

    // ── 翻译分流（原版 _process_segment_text 尾部；字幕窗 extra_langs 随 M4 接入）──
    let Some(rig) = tl else {
        // 翻译装置未就绪（配置无效已被 TranslatorUnavailable 提醒）：译文行立即
        // 给出明确占位，不停留在永久的「翻译中...」——P0-2
        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
            id,
            text: lt_i18n::t("translator_unavailable_placeholder"),
            tl_ms: 0.0,
        }));
        return;
    };
    rig.stats.asr_count.fetch_add(1, Ordering::Relaxed);
    rig.transcript.write_original(id, &timestamp, original_text);
    if lang == target_language {
        tracing::info!("Same language ({lang}), no translation");
        rig.transcript.finalize_no_translation(id);
        let _ = proxy.send_event(UiMsg::Event(UiEvent::UpdateTranslation {
            id,
            text: String::new(),
            tl_ms: 0.0,
        }));
        let _ = proxy.send_event(UiMsg::Event(rig.stats.snapshot_event()));
    } else {
        rig.submit_translation(proxy, id, original_text.to_string(), lang.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_worker_config：funasr 按 entry.key 分派（WP-A）──

    /// 造 manifest「完整」的快照：稀疏文件（set_len 即达下限，秒级无磁盘占用）
    fn sparse_manifest(models_dir: &std::path::Path, entry: &registry::ModelEntry) {
        let snap = lt_models::cache::hf_style_snapshot(
            models_dir,
            lt_models::download::Hub::Hf,
            entry.hf.expect("条目须有 HF 源"),
            "main",
        );
        for (f, min) in entry.files.iter().zip(entry.files_min_bytes.iter()) {
            let p = snap.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::File::create(&p).unwrap().set_len(*min).unwrap();
        }
    }

    #[test]
    fn funasr_dispatch_nano_vs_sensevoice() {
        // WP-A：funasr-nano-2512 → 独立 worker 引擎 "nano" + 恒不传 pad；
        // sensevoice-small → 原路径 "sensevoice" + pad 照旧
        let base = std::env::temp_dir().join(format!("lt_nano_dispatch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        sparse_manifest(&base, &registry::FUNASR_NANO);
        sparse_manifest(&base, &registry::SENSEVOICE_SMALL);

        let (cfg, display) =
            build_worker_config(&base, "funasr", "funasr-nano-2512", 0.5, "auto", "", 2.0).expect("nano 已缓存应可装配");
        assert_eq!(cfg.engine, "nano");
        assert_eq!(cfg.pad_seconds, None, "nano 无 padding 语义");
        assert_eq!(display, "Fun-ASR-Nano");
        assert!(cfg.options["model_dir"].as_str().is_some());

        let (cfg2, display2) =
            build_worker_config(&base, "funasr", "sensevoice-small", 0.5, "auto", "", 2.0).expect("sensevoice 已缓存应可装配");
        assert_eq!(cfg2.engine, "sensevoice");
        assert_eq!(cfg2.pad_seconds, Some(0.5));
        assert_eq!(display2, "SenseVoice Small");

        // mlt 无注册表条目（D-14）→ 回退 sensevoice-small 条目装配
        let (cfg3, display3) =
            build_worker_config(&base, "funasr", "funasr-mlt-nano-2512", 0.5, "auto", "", 2.0).expect("mlt 回退 sensevoice 应可装配");
        assert_eq!(cfg3.engine, "sensevoice");
        assert_eq!(display3, "SenseVoice Small");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn qwen3_dispatch_and_uncached() {
        // WP-B：qwen3 → 独立 worker 引擎 + 恒不传 pad（B-α 单一模型，无模型键）
        let base = std::env::temp_dir().join(format!("lt_qwen3_dispatch_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // 未缓存 → None（发 AsrUnavailable，等待下载）
        assert!(build_worker_config(&base, "qwen3", "", 0.5, "auto", "", 2.0).is_none());

        sparse_manifest(&base, &registry::QWEN3_ASR);
        let (cfg, display) =
            build_worker_config(&base, "qwen3", "", 0.5, "auto", "", 2.0).expect("qwen3 已缓存应可装配");
        assert_eq!(cfg.engine, "qwen3");
        assert_eq!(cfg.pad_seconds, None, "qwen3 无 padding 语义");
        assert_eq!(display, "Qwen3-ASR-0.6B");
        assert!(cfg.options["model_dir"].as_str().is_some());

        // 日志模型键分派（打错键会误导诊断）
        assert_eq!(engine_model_key("qwen3", "funasr-nano-2512", "tiny"), "qwen3-asr-0.6b");
        assert_eq!(engine_model_key("whisper", "x", "tiny"), "tiny");
        assert_eq!(engine_model_key("funasr", "sensevoice-small", "tiny"), "sensevoice-small");

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── TlRig::from_settings：翻译装置构建（M3 装配） ──

    #[test]
    fn tl_rig_builds_from_default_settings() {
        let settings = lt_proto::Settings::default();
        let rig = TlRig::from_settings(&settings).expect("默认设置不应报配置错误");
        assert!(rig.is_some(), "默认 settings 带一个默认模型，应能构建");
        // 目标语言来自全局设置而非模型配置
        assert_eq!(rig.unwrap().translator.target_language(), settings.target_language);
    }

    #[test]
    fn tl_rig_none_when_active_model_out_of_bounds() {
        let mut settings = lt_proto::Settings::default();
        settings.active_model = 99;
        assert!(TlRig::from_settings(&settings).unwrap().is_none());
    }

    #[test]
    fn tl_rig_none_when_models_empty() {
        let mut settings = lt_proto::Settings::default();
        settings.models.clear();
        assert!(TlRig::from_settings(&settings).unwrap().is_none());
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

    // ── build_worker_config：whisper 分支（M5.1） ──

    fn tmp_models_dir(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("lt_bwc_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    /// 造 tiny q5_1 缓存（20MB > 半体积阈值 ≈16MB）
    fn write_tiny_cache(dir: &std::path::Path) {
        let snap = lt_models::paths::hf_cache_root(dir)
            .join("models--ggerganov--whisper.cpp")
            .join("snapshots")
            .join("main");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(snap.join("ggml-tiny-q5_1.bin"), vec![0u8; 20_000_000]).unwrap();
    }

    #[test]
    fn whisper_builtin_cached_builds_worker_config() {
        let dir = tmp_models_dir("cached");
        write_tiny_cache(&dir);
        let got = build_worker_config(&dir, "whisper", "", 0.5, "auto", "tiny", 0.7);
        let (cfg, display) = got.expect("tiny 已缓存应可装配");
        assert_eq!(cfg.engine, "whisper");
        assert_eq!(display, "Whisper tiny");
        assert_eq!(cfg.display_name, "Whisper tiny");
        assert_eq!(cfg.language, "auto");
        // whisper 分支必须用 whisper_pad 而非 sensevoice pad
        assert_eq!(cfg.pad_seconds, Some(0.7));
        let model_path = cfg
            .options
            .get("model_path")
            .and_then(|v| v.as_str())
            .expect("options 应带 model_path");
        assert!(model_path.replace('\\', "/").ends_with("main/ggml-tiny-q5_1.bin"), "{model_path}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whisper_uncached_is_none() {
        let dir = tmp_models_dir("uncached");
        assert!(build_worker_config(&dir, "whisper", "", 0.5, "auto", "tiny", 0.5).is_none());
        // 非法档位（既非 builtin 也非存在的本地路径）同样 None
        assert!(build_worker_config(&dir, "whisper", "", 0.5, "auto", "bogus-size", 0.5).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whisper_local_path_passthrough() {
        let dir = tmp_models_dir("local");
        let bin = dir.join("my-whisper.bin");
        std::fs::write(&bin, vec![0u8; 1024]).unwrap();
        let got = build_worker_config(dir.parent().unwrap(), "whisper", "", 0.5, "zh", &bin.to_string_lossy(), 0.6);
        let (cfg, display) = got.expect("存在的本地路径应直通");
        assert_eq!(cfg.engine, "whisper");
        assert_eq!(
            cfg.options.get("model_path").and_then(|v| v.as_str()),
            Some(bin.to_string_lossy().as_ref())
        );
        assert_eq!(display, "my-whisper");
        assert_eq!(cfg.pad_seconds, Some(0.6));
        // 不存在的本地路径 → None
        assert!(build_worker_config(&dir, "whisper", "", 0.5, "zh", "Z:/no/model.bin", 0.5).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn funasr_branch_unchanged_via_ms_cache() {
        // 守护既有 funasr 路径：MS 侧 manifest 齐全（pengzhendong 镜像布局）→ 正常装配
        // sensevoice。（DL-1 起 local_model_dir 需 manifest 完整——旧版空目录即命中，
        // 装配到半截目录只会在引擎加载时爆，正是 download-overhaul F13。）
        let dir = tmp_models_dir("funasr");
        let ms = lt_models::paths::ms_cache_root(&dir)
            .join("pengzhendong")
            .join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue");
        std::fs::create_dir_all(&ms).unwrap();
        std::fs::write(ms.join("model.int8.onnx"), vec![0u8; 60_000_000]).unwrap();
        std::fs::write(ms.join("tokens.txt"), vec![0u8; 4_096]).unwrap();
        let got = build_worker_config(&dir, "funasr", "sensevoice-small", 0.4, "auto", "tiny", 0.5);
        let (cfg, display) = got.expect("MS 缓存命中应可装配");
        assert_eq!(cfg.engine, "sensevoice");
        assert_eq!(display, "SenseVoice Small");
        assert_eq!(cfg.pad_seconds, Some(0.4)); // funasr 用 sensevoice pad
        assert!(cfg.options.get("model_dir").is_some());
        // manifest 不齐（缺 tokens.txt）→ 不得装配（防半截目录进引擎）
        std::fs::remove_file(ms.join("tokens.txt")).unwrap();
        assert!(build_worker_config(&dir, "funasr", "sensevoice-small", 0.4, "auto", "tiny", 0.5).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
