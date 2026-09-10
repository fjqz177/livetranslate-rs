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

use crate::event_artery::EventSink;
use crate::settings_bus::{EffectiveSettings, SettingsBus};
use crate::supervisor::{artery_sink, Policy, Supervisor};
use crate::Msg;
use lt_asr::{AsrEffectiveSettings, AsrManager, WorkerConfig};
use lt_models::registry;
use lt_audio::audio::wasapi_win::WasapiBackend;
use lt_audio::audio::capture::VadSource;
use lt_audio::interim::{
    is_short_utterance, pending_merge, split_sentences, strip_committed_overlap, trim_samples,
    InterimState,
};
use lt_audio::{
    AudioBackend, BoundedDropQueue, CaptureLoop, InterimControl, SegmentSource, VadProcessor,
};
use lt_proto::{
    ASR_ENGINES, AudioRole, CaptureEvent, EngineKey, MonitorSample, QueueId, ThreadRole, UiEvent,
};
use lt_translate::Translator;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use arc_swap::ArcSwap;

/// 段队列容量（对齐原版 _asr_queue maxsize=16，满丢最旧）
const SEGMENT_QUEUE_CAP: usize = 16;

/// 翻译线程池 worker 数（对齐原版 ThreadPoolExecutor(max_workers=8)）
const TL_POOL_WORKERS: usize = 8;

/// 翻译池待译段上限（R15①/D-64：慢 LLM + 快语速时保留最新、最旧放弃——
/// 实时翻译语义下陈旧段价值单调衰减；原实现无界，极端时无限积压且不可见。
/// 水位事件的 UI 呈现随 W2 事件动脉补齐）
const TL_QUEUE_CAP: usize = 64;

/// 定宽任务池（等价原版 _tl_executor：任务排队、固定 worker 消费）。
/// 架构 2.0 W1 改造：①队列有界满丢最旧（复用 BoundedDropQueue，ADR-2）；
/// ②worker 经 Supervisor 出生（INV3——修复现状 handle 丢弃、panic 永死）；
/// ③shutdown 置停止标志后由 Supervisor::join_all 真正 join（修复从不 join）；
/// ④Drop 自动停机（W1 泄漏修复：ReplaceRig/TestTranslator 替换或丢弃旧 rig 时
/// 旧池 worker 必须退出——若无人置 stopped，worker 仅凭 500ms 空转循环永不结束
/// （BoundedDropQueue 无信道关闭语义），旧池线程将永久泄漏并阻断
/// Pipeline::stop 的 join_all）
struct JobPool {
    queue: Arc<BoundedDropQueue<Box<dyn FnOnce() + Send>>>,
    stopped: Arc<AtomicBool>,
    /// 存活 worker 计数（RAII 增减）：泄漏回归测试的观测面
    #[allow(dead_code)]
    alive_workers: Arc<AtomicUsize>,
    /// R15② 水位事件出口（W2：慢 LLM 积压从"仅日志 warn"升级为类型化事件）
    sink: EventSink,
    /// 已上报丢弃计数（与队列丢弃告警同节奏，≥200 条报一次，防洪泛）
    reported: AtomicU64,
}

impl JobPool {
    fn new(workers: usize, sup: &Supervisor, sink: EventSink) -> Self {
        let queue = Arc::new(BoundedDropQueue::<Box<dyn FnOnce() + Send>>::new(
            TL_QUEUE_CAP,
            "tl-job",
        ));
        let stopped = Arc::new(AtomicBool::new(false));
        let alive_workers = Arc::new(AtomicUsize::new(0));
        for i in 0..workers {
            let queue = queue.clone();
            let stopped = stopped.clone();
            let alive_workers = alive_workers.clone();
            sup.spawn(ThreadRole::TlWorker, format!("lt-tl-{i}"), Policy::backoff(), move || {
                let queue = queue.clone();
                let stopped = stopped.clone();
                let alive_workers = alive_workers.clone();
                Box::new(move || {
                    alive_workers.fetch_add(1, Ordering::Relaxed);
                    // RAII 减计数：panic 路径同样归零（线程死亡即不存活）
                    let _alive = WorkerAliveGuard(alive_workers);
                    // 停止标志置位后 worker 在 ≤500ms 内退出，由 join_all 回收；
                    // panic 由监督器重生（干净循环状态，INV5）
                    while !stopped.load(Ordering::Relaxed) {
                        match queue.pop_timeout(Duration::from_millis(500)) {
                            Some(job) => job(),
                            None => continue,
                        }
                    }
                })
            });
        }
        Self {
            queue,
            stopped,
            alive_workers,
            sink,
            reported: AtomicU64::new(0),
        }
    }

    /// 提交翻译任务：队列满时丢最旧（R15①/D-64）。停止后仍可能有在途提交
    /// ——worker 已退出不再消费，任务滞留队列随 Pipeline 释放（与原
    /// 「停止后排队的任务直接丢弃」语义一致）。
    /// R15②：丢弃达上报节拍（≥200 条）即发 [`UiEvent::QueuePressure`]——
    /// 与 BoundedDropQueue 告警节奏同源的类型化水位信号
    fn submit(&self, job: impl FnOnce() + Send + 'static) {
        self.queue.push(Box::new(job));
        let dropped = self.queue.dropped_count();
        let reported = self.reported.load(Ordering::Relaxed);
        if dropped >= 200 && dropped - reported >= 200 {
            self.reported.store(dropped, Ordering::Relaxed);
            self.sink.push(UiEvent::QueuePressure {
                queue: QueueId::Translation,
                dropped_total: dropped,
            });
        }
    }

    /// 停止接收并丢弃排队任务；worker 退出后由 Supervisor::join_all 回收
    /// （translate 自带超时兜底，在跑任务自然结束）
    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// 存活 worker 数（泄漏回归测试观测面）
    #[cfg(test)]
    fn alive_worker_count(&self) -> usize {
        self.alive_workers.load(Ordering::Relaxed)
    }
}

/// Drop 自动停机（W1 泄漏修复）：rig 被替换/丢弃时池子一停，worker 在 ≤500ms
/// 节拍内退出——不置标志则 pop_timeout 空转永不结束（BoundedDropQueue 无
/// 信道关闭语义），被替换的池子将永久泄漏并阻断 Pipeline::stop 的 join_all。
impl Drop for JobPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// worker 退出计数句柄（RAII）：闭包入口 +1，任何出口（含 panic）−1
struct WorkerAliveGuard(Arc<AtomicUsize>);
impl Drop for WorkerAliveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
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
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 用户可见文案服务（i18n 注入；错误占位/回执文案经此取，见 [`Msg`]）
    msg: Msg,
    /// 设置总线（W4：翻译 worker 提交前读 `tl` 生效视图——目标语言/超时不再
    /// 存实例可变态（旧 MutableState 设置面 + TlSwitch::TargetLanguage/Timeout 镜像）
    bus: Arc<SettingsBus>,
}

impl TlRig {
    /// 按设置构建；models 为空/active_model 越界 → Ok(None)（不翻译，仅 ASR）；
    /// 配置无效（URL 格式错等）→ Err(原因)（必须让用户可见，见 TranslatorUnavailable）
    fn from_settings(
        bus: &Arc<SettingsBus>,
        sup: &Supervisor,
        sink: EventSink,
        transcript: Arc<lt_audio::transcript::TranscriptWriter>,
        msg: Msg,
    ) -> Result<Option<Self>, String> {
        let eff = bus.load();
        let Some(mc) = eff.raw.models.get(eff.raw.active_model) else {
            return Ok(None);
        };
        Self::from_effective(mc, &eff, bus, sup, sink, transcript, msg)
    }

    /// 按指定模型配置构建（运行时切换用；构建失败返回 Err——UI 收到
    /// TranslatorUnavailable 显示到翻译页状态行，不再静默关闭整条翻译）。
    /// W4：目标语言/超时/系统提示读自设置总线（eff 快照），不再携带
    /// settings 全量（旧 ReplaceRig 载荷）——发布即生效，无镜像同步边
    #[allow(clippy::too_many_arguments)]
    fn from_effective(
        mc: &lt_proto::ModelConfig,
        eff: &EffectiveSettings,
        bus: &Arc<SettingsBus>,
        sup: &Supervisor,
        sink: EventSink,
        transcript: Arc<lt_audio::transcript::TranscriptWriter>,
        msg: Msg,
    ) -> Result<Option<Self>, String> {
        let params = lt_translate::TranslatorParams {
            api_base: mc.api_base.clone(),
            api_key: mc.api_key.clone(),
            model: mc.model.clone(),
            // W1/方案 §2.1：长度上限不再由应用发送（交给服务端默认——应用强加的
            // 256 会把"先想再答"的模型憋死，实测就是这个原因导致空译文）
            max_tokens: None,
            // W1/方案 §2.1：温度取模型条目值（None = 不发送）
            temperature: mc.temperature,
            streaming: mc.streaming,
            system_prompt: (!eff.raw.system_prompt.is_empty())
                .then(|| eff.raw.system_prompt.clone()),
            proxy: mc.proxy.clone(),
            no_system_role: mc.no_system_role,
            // W1/方案 §2.3：总开关 + 方式（sanitize 已把旧 "off" 归一化到总开关）
            disable_thinking: mc.disable_thinking,
            thinking_style: mc.thinking_style.clone(),
            json_response: mc.json_response,
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
        tracing::info!("Switching translator: {} ({})", mc.name, mc.model);
        Ok(Some(Self {
            translator,
            stats,
            pool: JobPool::new(TL_POOL_WORKERS, sup, sink),
            transcript,
            msg,
            bus: bus.clone(),
        }))
    }

    /// 提交一段的翻译任务（对照原版 _translate_async 的成功/重复/错误三路）；
    /// 同语言不进此函数（ASR 线程直接回空译文，见 run_asr_thread）。
    /// W4：目标语言/超时在 **任务执行时** 读总线（排队期间设置变更与旧
    /// `set_target_language` 改运行时可变面的语义同为"执行时最新"——旧实现
    /// worker 开始执行才取 state 锁；此处等价且无锁）
    fn submit_translation(
        &self,
        sink: &EventSink,
        id: u64,
        text: String,
        source_lang: String,
    ) {
        let translator = self.translator.clone();
        let stats = self.stats.clone();
        let transcript = self.transcript.clone();
        let sink = sink.clone();
        let msg = self.msg.clone();
        let bus = self.bus.clone();
        self.pool.submit(move || {
            let eff = bus.load();
            let target = eff.tl.target_language.clone();
            let timeout = eff.tl.timeout;
            let t0 = Instant::now();
            let mut translated: Option<String> = None;
            for item in translator.translate_iter(&text, &source_lang, &target, timeout) {
                match item {
                    Ok(partial) => {
                        sink.push(UiEvent::UpdateStreaming {
                            id,
                            partial: partial.clone(),
                        });
                        translated = Some(partial);
                    }
                    Err(lt_translate::TranslateError::Repetition(_)) => {
                        tracing::warn!(
                            "Repetition loop detected, model may not support structured output well"
                        );
                        transcript.finalize_no_translation(id);
                        sink.push(UiEvent::UpdateTranslation {
                            id,
                            text: msg.t("error_repetition"),
                            tl_ms: 0.0,
                        });
                        return;
                    }
                    Err(e) => {
                        if e.is_expected() {
                            tracing::warn!("Translate error: {e}");
                        } else {
                            tracing::error!("Translate error: {e}");
                        }
                        transcript.finalize_no_translation(id);
                        sink.push(UiEvent::UpdateTranslation {
                            id,
                            text: e.ui_text(),
                            tl_ms: 0.0,
                        });
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
            sink.push(UiEvent::UpdateTranslation {
                id,
                text: translated.clone(),
                tl_ms,
            });
            sink.push(stats.snapshot_event());
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

/// `Pipeline::start` 半初始化守卫（R12/D-62）：armed 期间 Drop = 回滚——
/// 置 stop → 停翻译池 → join 已 spawn 线程 → 停音频后端。disarm 后 Drop 变
/// no-op。注意顺序：必须先 pool.shutdown 再 join_all——翻译 worker 以
/// stopped 为退出条件，不先置标志 join_all 会被永久等待的 worker 卡死
/// （这正是守卫要掩护的场景，遗漏即回滚路径自身死锁）
struct StartGuard {
    armed: bool,
    stop: Arc<AtomicBool>,
    sup: Arc<Supervisor>,
    tl: Option<Arc<TlRig>>,
    backend: Option<WasapiBackend>,
}

impl StartGuard {
    fn disarm(mut self) -> WasapiBackend {
        self.armed = false;
        self.backend.take().expect("守卫移交时后端必须已就位")
    }
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        tracing::warn!("Pipeline::start 半初始化失败，回滚已启动的线程与音频后端");
        // INV4：stopping 先置位（禁 respawn），再发各线程停止信号
        self.sup.begin_shutdown();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(tl) = &self.tl {
            tl.shutdown();
        }
        // 先停音频后端：wasapi 线程退出 → AudioStatus 通道断开 → 音频状态转发
        // 线程（Never 策略，以 rx 迭代结束为退出条件）才有机会收尾；顺序反了
        // join_all 会永久阻塞在该线程上（W1 守卫自身死锁修复）
        if let Some(b) = self.backend.as_mut() {
            b.stop();
        }
        self.sup.join_all();
    }
}

pub struct Pipeline {
    backend: WasapiBackend,
    stop: Arc<AtomicBool>,
    /// 暂停标志（capture 线程丢弃 chunk 不喂 VAD）；M4 托盘暂停联动
    #[allow(dead_code)]
    paused: Arc<AtomicBool>,
    /// 翻译装置（models 空/构建失败时 None = 仅 ASR 不翻译）
    tl: Option<Arc<TlRig>>,
    /// 运行时翻译器切换通道（UI 域命令 → ASR 线程空闲分支应用；
    /// 原版对应 _switch_translator / _switch_asr_engine / 测试连接）
    tl_switch: Option<crossbeam_channel::Sender<TlSwitch>>,
    /// 增量识别控制块（与 capture/ASR 线程共享；set_interim 热应用）
    interim: Arc<InterimControl>,
    /// 线程监督器（架构 2.0 W1/INV3）：capture/ASR/翻译池/音频状态转发全部
    /// 经其出生，stop 时 join_all 统一回收
    sup: Arc<Supervisor>,
    /// 转录写盘（原版 self._transcript；TlRig/面板共用同一句柄）
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// R31/D-72：与 capture/ASR 线程共享的 VAD（集切换复位入口——
    /// 设备切换 = 会话边界：清段队列 + VAD 重置 + interim 复位，
    /// 防新旧设备/会话段拼接错位）
    vad_shared: Arc<Mutex<VadProcessor>>,
    /// R31：段队列（ASR 段消费侧同一实例；切换清空丢弃残留半段）
    segment_queue_shared: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
}

/// ASR 线程消费的翻译器/引擎命令（W4 收敛三臂——目标语言/超时/语言/padding
/// 四臂被设置总线派生视图吸收：真实重建/切换动作才必须走线程命令）
pub(crate) enum TlSwitch {
    /// 整体重建翻译装置（切模型；历史随旧实例丢弃，与原版重建 Translator 一致）
    ReplaceRig {
        config: Box<lt_proto::ModelConfig>,
    },
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
    TestTranslator {
        name: String,
        config: Box<lt_proto::ModelConfig>,
    },
}

/// 转录写盘句柄（W3 起随 Pipeline 生命周期显式持有：进程级 OnceLock 单例
/// 退役——句柄真源从"全局"收敛为"Pipeline 字段"，UI 面板"应用"经
/// [`Pipeline::set_transcript_enabled`] 访问，不再跨域取全局）
fn transcript_handle() -> Arc<lt_audio::transcript::TranscriptWriter> {
    // R26/D-62：回退链=配置目录/transcripts，绝不落 CWD（路径卫生）
    let dir = lt_models::paths::transcripts_dir().unwrap_or_else(|e| {
        tracing::error!("transcripts_dir 不可用（{e}），回退配置目录");
        lt_models::paths::config_dir()
            .map(|d| d.join("transcripts"))
            .unwrap_or_else(|_| std::path::PathBuf::from("transcripts"))
    });
    Arc::new(lt_audio::transcript::TranscriptWriter::new(dir))
}

impl Pipeline {
    /// 按设置总线当前快照启动整条管道；模型未缓存时不阻断 UI（发 AsrUnavailable）。
    /// `bus` 为组合根持有的总线句柄（启动前其当前快照 = 要应用的设置——
    /// shell 先 publish 后调用本函数，INV7）；各线程持克隆自行 `load()`。
    /// `msg` 为 i18n 文案服务（白名单不变量：本 crate 零 lt-i18n 依赖，见 [`Msg`]）。
    pub fn start(
        bus: &Arc<SettingsBus>,
        sink: EventSink,
        monitor_cell: &Arc<ArcSwap<MonitorSample>>,
        msg: Msg,
    ) -> anyhow::Result<Self> {
        let eff = bus.load();
        let settings = eff.raw.clone();

        // ── 转录写盘（原版 self._transcript + auto_save_transcript）──
        let transcript = transcript_handle();
        transcript.set_enabled(settings.auto_save_transcript);

        // ── 线程监督器（架构 2.0 W1/INV3：管道线程唯一出生点）──
        let sup = Supervisor::new(artery_sink(sink.clone()));

        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));

        // R12/D-62：半初始化守卫——任一 ? 提前返回时回滚已 spawn 线程与音频
        // 后端（现状缺陷：音频线程泄漏独占设备直到进程退出）
        let mut guard = StartGuard {
            armed: true,
            stop: stop.clone(),
            sup: sup.clone(),
            tl: None,
            backend: None,
        };

        // ── 音频：chunk 满丢旧队列 + 可用性边沿上报通道（R4/D-62）──
        let chunk_queue = Arc::new(BoundedDropQueue::new(100, "chunk"));
        let mut backend = WasapiBackend::new();
        let (audio_status_tx, audio_status_rx) = std::sync::mpsc::channel();
        backend.start(
            settings.audio_device.clone(),
            settings.mic_device.clone(),
            chunk_queue.clone(),
            Some(audio_status_tx),
        )?;
        guard.backend = Some(backend);

        // ── capture 线程：VAD 状态机 ──
        // W4：VAD 生效值 = 总线派生视图（已含 qwen3 钳制 overlay），capture
        // 每轮读格比对版本应用——旧 vad_update 槽（镜像）退役
        let vad_settings = eff.vad.clone();
        let confidence = lt_audio::vad::make_confidence_source(
            &vad_settings.mode,
            vad_settings.energy_threshold,
        );
        let mut vad = VadProcessor::new(
            confidence,
            lt_audio::TARGET_RATE as usize,
            vad_settings.threshold,
            vad_settings.min_speech_duration,
            vad_settings.max_speech_duration,
            lt_audio::CHUNK_DURATION,
        );
        vad.update_settings(&vad_settings);

        let segment_queue = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(
            SEGMENT_QUEUE_CAP,
            "segment",
        ));
        // VAD 共享拓扑（原版 _vad_lock）：capture 写、ASR 线程增量识别时
        // peek/trim/speech_samples 读，锁粒度 = 单次方法调用
        let vad = Arc::new(Mutex::new(vad));
        let interim = Arc::new(InterimControl::default());
        // 启动即按持久化设置就位（原版 _incremental_enabled/_interim_interval 随启动初始化）
        interim.set(settings.incremental_asr, settings.interim_interval);
        // W2/D-67：音频监视快照格——capture 每 chunk 写；UI 以 ~33ms 节拍读格
        // 重绘（替代 UpdateMonitor 逐事件直发：31/s 唤醒泛洪归零，R23）
        let monitor_seq = Arc::new(AtomicU64::new(0));
        // W4：VAD 生效设置发布格读侧（总线无锁 load——版本比对在 capture 内）
        let vad_tick: VadSource = {
            let bus = bus.clone();
            Arc::new(move || {
                let e = bus.load();
                (e.version, e.vad.clone())
            })
        };
        {
            let stop = stop.clone();
            let paused = paused.clone();
            let segment_queue = segment_queue.clone();
            let monitor_cell = monitor_cell.clone();
            let monitor_seq = monitor_seq.clone();
            let vad_tick = vad_tick.clone();
            let vad = vad.clone();
            let interim = interim.clone();
            let mode = vad_settings.mode.clone();
            // INV3/INV5：经监督器出生；panic 重生 = 工厂重建干净循环状态
            //（消费前清 chunk 陈旧积压——宕机期间音频已满丢旧轮转，续读=句中撕裂）
            sup.spawn(ThreadRole::Capture, "lt-capture", Policy::backoff(), move || {
                let stop = stop.clone();
                let paused = paused.clone();
                let segment_queue = segment_queue.clone();
                let monitor_cell = monitor_cell.clone();
                let monitor_seq = monitor_seq.clone();
                let vad_tick = vad_tick.clone();
                let vad = vad.clone();
                let interim = interim.clone();
                let mode = mode.clone();
                let chunk_queue = chunk_queue.clone();
                Box::new(move || {
                    chunk_queue.clear();
                    // 原版 _capture_loop：monitor 直接跨线程信号（此处写监视快照格，
                    // vad 转换为 UI 侧 f32），段直接塞 _asr_queue 等价队列（满丢旧）
                    let mut loop_ = CaptureLoop {
                        chunk_rx: chunk_queue,
                        segment_tx: segment_queue,
                        monitor: move |rms, vad, mic_rms| {
                            let seq = monitor_seq.fetch_add(1, Ordering::Relaxed) + 1;
                            monitor_cell.store(Arc::new(MonitorSample {
                                rms,
                                vad: vad as f32,
                                mic_rms,
                                seq,
                            }));
                        },
                        paused,
                        vad_tick,
                        interim,
                        // R2/D-60：初值=启动模式；模式热切换时 capture 换置信度源
                        current_mode: mode,
                    };
                    loop_.run(&vad, &stop);
                })
            });
        }

        // ── 翻译装置（M3）：models 非空即构建；配置无效必须让用户可见
        //（TranslatorUnavailable → 面板翻译页状态行 + 悬浮窗译文占位）──
        let tl = match TlRig::from_settings(bus, &sup, sink.clone(), transcript.clone(), msg.clone()) {
            Ok(t) => t.map(Arc::new),
            Err(reason) => {
                sink.push(UiEvent::TranslatorUnavailable {
                    reason: reason.clone(),
                });
                tracing::error!("翻译装置不可用: {reason}");
                None
            }
        };
        // 守卫纳管翻译池（回滚时必须先停机再 join，见 StartGuard 注记）
        guard.tl = tl.clone();
        let (tl_switch_tx, tl_switch_rx) = crossbeam_channel::unbounded::<TlSwitch>();

        // ── 音频可用性转发线程（R4/D-62）：wasapi 边沿事件 → UiEvent::Capture ──
        {
            let sink = sink.clone();
            // rx 不可克隆：装单槽 cell，重生工厂取空即空转退出（Never 策略）
            let rx_cell = Arc::new(Mutex::new(Some(audio_status_rx)));
            sup.spawn(
                ThreadRole::AudioBridge,
                "lt-audio-status",
                Policy::Never,
                move || {
                    let sink = sink.clone();
                    let rx_cell = rx_cell.clone();
                    Box::new(move || {
                        // INV6：先绑定再离开锁
                        let Some(rx) = rx_cell.lock().unwrap().take() else {
                            return;
                        };
                        // tx 在音频线程退出（Pipeline::stop→backend.stop）时 drop
                        // → 本线程自然收尾（停机期监督器静默收割）
                        for st in rx {
                            use lt_audio::audio::wasapi_win::AudioStatus;
                            let ev = match st {
                                AudioStatus::OutputLost(e) => CaptureEvent::Unavailable {
                                    role: AudioRole::Loopback,
                                    error: e,
                                },
                                AudioStatus::InputLost(e) => CaptureEvent::Unavailable {
                                    role: AudioRole::Mic,
                                    error: e,
                                },
                                AudioStatus::OutputRecovered => {
                                    CaptureEvent::Recovered { role: AudioRole::Loopback }
                                }
                                AudioStatus::InputRecovered => {
                                    CaptureEvent::Recovered { role: AudioRole::Mic }
                                }
                            };
                            sink.push(UiEvent::Capture(ev));
                        }
                    })
                },
            );
        }

        // ── ASR 线程：Manager 独占 + 段处理 ──
        {
            let stop = stop.clone();
            let sink = sink.clone();
            let segment_queue = segment_queue.clone();
            let settings = settings.clone();
            let bus_asr = bus.clone();
            let tl = tl.clone();
            let vad = vad.clone();
            let interim = interim.clone();
            let sup_asr = sup.clone();
            let tl_switch_rx = tl_switch_rx.clone();
            let msg_asr = msg.clone();
            let transcript_asr = transcript.clone();
            // INV3：经监督器出生；panic 重生 = 待命/装配路径干净重启（INV5）
            sup.spawn(ThreadRole::AsrMain, "lt-asr-main", Policy::backoff(), move || {
                let stop = stop.clone();
                let sink = sink.clone();
                let segment_queue = segment_queue.clone();
                let settings = settings.clone();
                let bus = bus_asr.clone();
                let tl = tl.clone();
                let vad = vad.clone();
                let interim = interim.clone();
                let sup = sup_asr.clone();
                let tl_switch = tl_switch_rx.clone();
                let msg = msg_asr.clone();
                let transcript = transcript_asr.clone();
                Box::new(move || {
                    run_asr_thread(
                        &settings,
                        AsrThreadCtx {
                            segment_queue,
                            vad,
                            interim,
                            bus,
                            stop,
                            sink,
                            tl_switch,
                            sup,
                            transcript,
                            msg,
                        },
                        tl,
                    );
                })
            });
        }

        tracing::info!("管道已启动（capture + VAD + ASR + 翻译；线程全部受监督）");
        Ok(Self {
            backend: guard.disarm(),
            stop,
            paused,
            tl,
            tl_switch: Some(tl_switch_tx),
            interim,
            sup,
            transcript,
            vad_shared: vad.clone(),
            segment_queue_shared: segment_queue.clone(),
        })
    }

    /// 转录写盘启停（面板"应用"热切换；W3 起句柄真源 = Pipeline 字段，
    /// 不再经进程级单例跨域访问——旧 transcript_shared() 已删）
    pub fn set_transcript_enabled(&self, enabled: bool) {
        self.transcript.set_enabled(enabled);
    }

    /// 运行时切换翻译模型（原版 _switch_translator 的用户可见路径；
    /// ASR 线程在下一次空闲分支应用，慢/挂死的服务端不冻结 UI）。
    /// W4：目标语言/超时/系统提示读自设置总线（不再随命令携带 settings 快照）
    pub fn switch_translator(&self, config: &lt_proto::ModelConfig) {
        if let Some(tx) = &self.tl_switch {
            let _ = tx.send(TlSwitch::ReplaceRig {
                config: Box::new(config.clone()),
            });
        }
    }

    /// 运行时切换 ASR 引擎/模型（原版 _switch_asr_engine 的路由；
    /// ASR 线程空闲分支执行 ensure_started，失败回滚由 Manager 内部保证）
    pub fn switch_engine(
        &self,
        engine: &str,
        funasr_model: &str,
        whisper_model_size: &str,
        language: &str,
    ) {
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
    /// R31/D-72：设备切换 = 会话边界——清段队列 + VAD reset + interim 复位，
    /// 防新旧设备/会话段拼接错位（wasapi 线程内已清 chunk 队列）
    pub fn set_audio_device(&mut self, choice: lt_proto::AudioDeviceChoice) {
        let dev = match choice {
            lt_proto::AudioDeviceChoice::SystemDefault => None,
            lt_proto::AudioDeviceChoice::Named(n) => Some(n),
            lt_proto::AudioDeviceChoice::Disabled => Some("__disabled__".into()),
        };
        self.reset_session_after_device_switch();
        self.backend.set_device(dev);
    }

    /// 运行时切换麦克风（原版 set_mic_device；None=禁用）
    /// R31：与 set_audio_device 同语义（会话边界清根）
    pub fn set_mic_device(&mut self, choice: lt_proto::MicDeviceChoice) {
        let dev = match choice {
            lt_proto::MicDeviceChoice::Off => None,
            lt_proto::MicDeviceChoice::Default => Some("__default__".into()),
            lt_proto::MicDeviceChoice::Named(n) => Some(n),
        };
        self.reset_session_after_device_switch();
        self.backend.set_mic_device(dev);
    }

    /// R31/D-72：会话复位（设备切换共用入口）——清段队列（丢弃残留半段，
    /// ASR 线程下轮 pop 自然空）、VAD 状态重置、interim 复位（旧会话
    /// 增量识别上下文不得跨设备存活）。INV6：vad 锁内不取第二把锁。
    fn reset_session_after_device_switch(&self) {
        self.segment_queue_shared.clear();
        if let Ok(mut v) = self.vad_shared.lock() {
            v.reset();
        }
        self.interim.reset_counter();
    }

    /// 增量识别开关/间隔热应用（原版 _incremental_asr_cb → _incremental_enabled/
    /// _interim_interval）：写共享控制块，capture 线程下一 chunk 生效；
    /// 关闭时清进度计数，重开从零起算
    pub fn set_interim(&self, enabled: bool, interval: f32) {
        self.interim.set(enabled, interval);
        tracing::info!("增量识别: {enabled}（间隔 {interval}s）");
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        // INV4 停机序：监督器 stopping 先置位——必须在一切线程停止信号之前，
        // 否则 monitor 500ms 节拍可能把正被关闭的线程重新拉起（竞态洞封堵）；
        // 随后 stop 标志 → 音频后端 → 翻译池停止 → join 全部受监督线程
        // （capture/ASR/翻译 worker/音频状态转发/monitor）
        self.sup.begin_shutdown();
        self.stop.store(true, Ordering::Relaxed);
        self.backend.stop();
        if let Some(tl) = &self.tl {
            tl.shutdown();
        }
        self.sup.join_all();
        tracing::info!("管道已停止");
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
    // E2/D-79：值域先验（未实装引擎诚实 None——不用 from_settings_str 的
    // FunAsr 回退，那会把"未知引擎"错装配成 funasr worker），再走枚举分派。
    // WorkerConfig.engine 保持 String：worker IPC 是唯一真进程边界
    if !ASR_ENGINES.contains(&engine) {
        tracing::warn!("引擎 {engine:?} 未实装，无法启动 worker");
        return None;
    }
    match EngineKey::from_settings_str(engine) {
        EngineKey::FunAsr => {
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
                    language: language.to_string(),
                    pad_seconds: pad,
                    options: lt_asr::WorkerOptions::ModelDir(model_dir.to_path_buf()),
                },
                entry.display.into(),
            ))
        }
        EngineKey::Whisper => {
            // M5.1：whisper_model_size 为 builtin 档（缓存 snapshot 解析 .bin）
            // 或本地 GGML 路径；未缓存 → None（发 AsrUnavailable，等向导/下载）
            let (model_path, display) = resolve_whisper_model(models_dir, whisper_model)?;
            Some((
                WorkerConfig {
                    engine: "whisper".into(),
                    language: language.to_string(),
                    pad_seconds: Some(whisper_pad),
                    options: lt_asr::WorkerOptions::ModelPath(model_path),
                },
                display,
            ))
        }
        EngineKey::Qwen3 => {
            // WP-B：单一模型（B-α，settings 无独立模型键）；无 padding 语义
            let entry = registry::qwen3_entry();
            let model_dir = lt_models::cache::local_model_dir(models_dir, &entry)?;
            Some((
                WorkerConfig {
                    engine: "qwen3".into(),
                    language: language.to_string(),
                    pad_seconds: None,
                    options: lt_asr::WorkerOptions::ModelDir(model_dir.to_path_buf()),
                },
                entry.display.into(),
            ))
        }
    }
}

/// 引擎切换日志的模型键：whisper 打档位、qwen3 打固定键（settings 无独立键）、
/// funasr 打 funasr_model——打错键会误导诊断。
fn engine_model_key<'a>(engine: &str, funasr_model: &'a str, whisper_model: &'a str) -> &'a str {
    match EngineKey::from_settings_str(engine) {
        EngineKey::Whisper => whisper_model,
        EngineKey::Qwen3 => "qwen3-asr-0.6b",
        EngineKey::FunAsr => funasr_model,
    }
}

/// ASR 线程上下文（Pipeline::start 一次性装配的共享件）
struct AsrThreadCtx {
    segment_queue: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
    /// 与 capture 线程共享的 VAD（增量识别锁内 peek/trim，原版 _vad_lock）
    vad: Arc<Mutex<VadProcessor>>,
    /// 增量识别跨线程控制块（capture 写触发时间、本线程写消费进度）
    interim: Arc<InterimControl>,
    /// 设置总线（W4：语言/padding/目标语言/超时生效值唯一事实源——
    /// transcribe 前 load().asr_lang、同语言判定 load().tl，替代旧
    /// AsrRuntime/挂起句柄/target_language 三重镜像）
    bus: Arc<SettingsBus>,
    stop: Arc<AtomicBool>,
    sink: EventSink,
    tl_switch: crossbeam_channel::Receiver<TlSwitch>,
    /// 线程监督器句柄（ReplaceRig 重建翻译池用）
    sup: Arc<Supervisor>,
    /// 转录写盘（ReplaceRig/TestTranslator 重建翻译装置时共享同一句柄）
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 用户可见文案服务（i18n 注入；错误占位/测试连接回执经此取）
    msg: Msg,
}

/// 翻译器域命令的共享路由（AH-1/H1）：待命循环与主循环空闲分支共用的三臂
/// （ReplaceRig/TestTranslator；W4 收敛——TargetLanguage/Timeout/AsrLanguage/
/// Pad 四臂被设置总线派生视图吸收）。`ReplaceEngine` 的引擎处理两侧不同——
/// 待命态只重试装配，主循环 ensure_started+事件+回滚——原样透传给调用方。
/// W3 起收 transcript/msg 注入参数（ReplaceRig/TestTranslator 重建装置用）；
/// W4 起 `bus` 注入（ReplaceRig/TestTranslator 从总线读目标语言/超时/系统提示）。
#[allow(clippy::too_many_arguments)]
fn route_translator_switch(
    sw: TlSwitch,
    tl: &mut Option<Arc<TlRig>>,
    bus: &Arc<SettingsBus>,
    sink: &EventSink,
    sup: &Supervisor,
    transcript: &Arc<lt_audio::transcript::TranscriptWriter>,
    msg: &Msg,
) -> Option<TlSwitch> {
    match sw {
        TlSwitch::ReplaceRig { config } => {
            let eff = bus.load();
            match TlRig::from_effective(
                &config,
                &eff,
                bus,
                sup,
                sink.clone(),
                transcript.clone(),
                msg.clone(),
            ) {
                Ok(Some(rig)) => {
                    tracing::info!("翻译器已切换: {} ({})", config.name, config.model);
                    *tl = Some(Arc::new(rig));
                }
                Ok(None) => {
                    tracing::warn!("翻译器切换目标为空（models 空/越界），保持当前装置");
                }
                Err(reason) => {
                    sink.push(UiEvent::TranslatorUnavailable { reason });
                }
            }
            None
        }
        TlSwitch::TestTranslator { name, config } => {
            // 构建临时装置（不切换活动翻译器），发一次最简请求回执 UI；
            // 目标语言/超时读总线（与活动翻译器同源，发布即一致）
            let eff = bus.load();
            match TlRig::from_effective(
                &config,
                &eff,
                bus,
                sup,
                sink.clone(),
                transcript.clone(),
                msg.clone(),
            ) {
                Ok(Some(rig)) => {
                    let sink = sink.clone();
                    let name = name.clone();
                    let msg_t = msg.clone();
                    let bus_t = bus.clone();
                    rig.pool.submit(move || {
                        let eff = bus_t.load();
                        let target = eff.tl.target_language.clone();
                        let timeout = eff.tl.timeout;
                        let t0 = Instant::now();
                        let mut it = rig.translator.translate_iter(
                            "Livetranslate test",
                            "auto",
                            &target,
                            timeout,
                        );
                        let (ok, err, ms) = match it.next() {
                            Some(Ok(_partial)) => (true, None, t0.elapsed().as_millis() as u64),
                            Some(Err(e)) => {
                                (false, Some(e.ui_text()), t0.elapsed().as_millis() as u64)
                            }
                            None => (
                                false,
                                Some(msg_t.t("test_translator_no_response")),
                                t0.elapsed().as_millis() as u64,
                            ),
                        };
                        sink.push(UiEvent::TestTranslatorResult {
                            name,
                            ok,
                            error: err,
                            ms,
                        });
                    });
                }
                Ok(None) => {
                    sink.push(UiEvent::TestTranslatorResult {
                        name,
                        ok: false,
                        error: Some(msg.t("test_translator_no_config")),
                        ms: 0,
                    });
                }
                Err(reason) => {
                    sink.push(UiEvent::TestTranslatorResult {
                        name,
                        ok: false,
                        error: Some(reason),
                        ms: 0,
                    });
                }
            }
            None
        }
        engine @ TlSwitch::ReplaceEngine { .. } => Some(engine),
    }
}

/// ASR 线程：模型就绪则循环识别；未缓存则发 AsrUnavailable 后待命
fn run_asr_thread(settings: &lt_proto::Settings, ctx: AsrThreadCtx, mut tl: Option<Arc<TlRig>>) {
    let AsrThreadCtx {
        segment_queue,
        vad,
        interim,
        bus,
        stop,
        sink,
        tl_switch,
        sup,
        transcript,
        msg,
    } = ctx;
    // 增量识别会话状态（原版 _interim_* 字段；跨段存活，vad_flush 复位）
    let mut interim_state = InterimState::default();
    // 构造 worker 配置（当前仅 sensevoice；whisper M5）。
    // R3/D-61：models_dir 失败 → 待命态而非 return 杀线程（AH-1 哲学推广：
    // 线程死亡=切换命令通道消亡，用户从此无法唤醒）。待命循环内每次
    // ReplaceEngine 尝试重新解析目录，用户修正环境后即可唤醒
    let models_dir = match lt_models::paths::models_dir(settings.models_dir.as_deref()) {
        Ok(d) => Some(d),
        Err(e) => {
            sink.push(UiEvent::AsrUnavailable);
            tracing::error!("模型目录不可用，ASR 进入待命（可经切换引擎唤醒重试）: {e}");
            None
        }
    };
    // W4：启动装配时的生效视图（语言/pad 不再走 AsrRuntime 镜像——总线快照；
    // 启动后每次引擎装配/段过滤均重新 load()，无"记得补同步边"）
    let mut worker = None;
    if let Some(models_dir) = &models_dir {
        // 模型键 → 条目；mlt/非法键回退 sensevoice-small（不阻断 UI，也不得用 nano 冒充）。
        // 诊断仅 funasr 引擎相关：whisper/qwen3 启动诊断走 build_worker_config 对应分支
        // （E2/D-79：判定走引擎值域透镜，零字面量比较）
        let (entry, fell_back) = match settings.engine_key() {
            EngineKey::FunAsr => resolve_funasr_entry(&settings.funasr_model),
            // WP-B：诊断用 qwen3 自身条目（否则未缓存日志打错模型名）
            EngineKey::Qwen3 => (registry::qwen3_entry(), false),
            EngineKey::Whisper => (registry::SENSEVOICE_SMALL.clone(), false),
        };
        if fell_back {
            let reason = if registry::funasr_key_is_ghost(&settings.funasr_model) {
                "mlt 无上游 ONNX 转换，待上游产出（D-14）"
            } else {
                "非法模型键"
            };
            tracing::warn!(
                "funasr 模型 {:?} 不可用（{reason}），回退 sensevoice-small",
                settings.funasr_model
            );
        }
        let eff = bus.load();
        worker = build_worker_config(
            models_dir,
            &settings.asr_engine,
            &settings.funasr_model,
            eff.asr_lang.sensevoice_pad,
            &eff.asr_lang.language,
            &settings.whisper_model_size,
            eff.asr_lang.whisper_pad,
        );
        if worker.is_none() {
            sink.push(UiEvent::AsrUnavailable);
            tracing::warn!("ASR 模型未缓存（{entry:?}），进入待命态（AH-1）");
        }
    }
    // 待命 = 可唤醒状态而非死胡同（AH-1/H1，P0）：模型就绪前吞掉段，但必须
    // 消费 tl_switch——运行时下载完成后 shell 重发 SwitchEngine（shell.rs
    // DownloadSucceeded 分支），收到即装配并落回正常装配路径。
    // 原实现只排段不消费命令，唤醒信号无人接，新用户首启旅程必须重启应用。
    while worker.is_none() && !stop.load(Ordering::Relaxed) {
        let _ = segment_queue.pop_timeout(Duration::from_secs(1));
        while let Ok(sw) = tl_switch.try_recv() {
            let Some(TlSwitch::ReplaceEngine {
                engine,
                funasr_model,
                whisper_model_size,
                language,
            }) = route_translator_switch(sw, &mut tl, &bus, &sink, &sup, &transcript, &msg)
            else {
                continue;
            };
            let model_key = engine_model_key(&engine, &funasr_model, &whisper_model_size);
            // R3/D-61：每次唤醒尝试重解析 models_dir（目录恢复后即可唤醒）
            let Ok(models_dir) = lt_models::paths::models_dir(settings.models_dir.as_deref())
            else {
                sink.push(UiEvent::AsrUnavailable);
                tracing::warn!("待命唤醒尝试：模型目录仍不可用，继续待命");
                continue;
            };
            let eff = bus.load();
            match build_worker_config(
                &models_dir,
                &engine,
                &funasr_model,
                eff.asr_lang.sensevoice_pad,
                &language,
                &whisper_model_size,
                eff.asr_lang.whisper_pad,
            ) {
                Some((config, display)) => {
                    tracing::info!("待命中模型已就绪: {engine}/{model_key}，退出待命装配 worker");
                    worker = Some((config, display));
                }
                None => {
                    sink.push(UiEvent::AsrUnavailable);
                    tracing::warn!("待命中切换目标仍未缓存: {engine}/{model_key}，继续待命");
                }
            }
        }
    }
    let Some((config, display)) = worker else {
        tracing::info!("ASR 待命中管道停止");
        return;
    };

    let mut manager = AsrManager::new();
    // 当前生效引擎的显示标签（切换失败回滚后用于恢复 AsrDevice，避免
    // 状态行挂着「ASR unavailable」而旧引擎实际仍在工作——P0-4）
    let mut current_display = display.clone();
    // AH-7/H13：AsrUnavailable 边沿触发——unavailable 是稳态，只在 false→true
    // 沿发一次事件，识别恢复（transcribe Ok / AsrDevice 发出）时复位
    let mut asr_unavailable_notified = false;
    // 模型加载对话框（原版 _ModelLoadDialog：装载期模态；AsrDevice/AsrUnavailable 关闭）
    sink.push(UiEvent::ModelLoadStart(display.clone()));
    if let Err(e) = manager.ensure_started(&config) {
        asr_unavailable_notified = true;
        sink.push(UiEvent::AsrUnavailable);
        tracing::error!("ASR worker 启动失败: {e}");
    } else {
        sink.push(UiEvent::AsrDevice(format!("{display} [cpu]")));
    }

    while !stop.load(Ordering::Relaxed) {
        // 取段：capture 线程直塞的 (source, audio)（source 目前仅 VadFlush，
        // M6 interim 接入后再分流）
        let Some((source, audio)) = segment_queue.pop_timeout(Duration::from_millis(500)) else {
            // 空闲分支：RSS 回收（原版 _asr_loop queue.Empty）+ 翻译器切换命令
            // （AH-1：翻译器三臂经 route_translator_switch 与待命循环共享）
            manager.maybe_recycle_if_idle();
            while let Ok(sw) = tl_switch.try_recv() {
                if let Some(TlSwitch::ReplaceEngine {
                    engine,
                    funasr_model,
                    whisper_model_size,
                    language,
                }) = route_translator_switch(sw, &mut tl, &bus, &sink, &sup, &transcript, &msg)
                {
                    // R3/D-61：每次切换尝试重解析 models_dir（与待命臂一致；
                    // 运行中目录损坏时切换路径同样可恢复）
                    let Ok(models_dir) =
                        lt_models::paths::models_dir(settings.models_dir.as_deref())
                    else {
                        sink.push(UiEvent::AsrUnavailable);
                        tracing::warn!("引擎切换尝试：模型目录不可用，跳过本轮");
                        continue;
                    };
                    // 原版 _switch_asr_engine：装配新配置 → 加载对话框 →
                    // ensure_started（失败内部回滚旧 worker）→ 设备/不可用事件
                    let eff = bus.load();
                    match build_worker_config(
                        &models_dir,
                        &engine,
                        &funasr_model,
                        eff.asr_lang.sensevoice_pad,
                        &language,
                        &whisper_model_size,
                        eff.asr_lang.whisper_pad,
                    ) {
                        Some((config, display)) => {
                            sink.push(UiEvent::ModelLoadStart(display.clone()));
                            if let Err(e) = manager.ensure_started(&config) {
                                // 回滚后旧 worker 仍在工作：恢复旧标签而非
                                // 发 AsrUnavailable（避免状态与行为矛盾，P0-4）
                                sink.push(UiEvent::AsrDevice(
                                    format!("{} [cpu]", current_display),
                                ));
                                tracing::error!("引擎切换失败（已回滚）: {e}");
                            } else {
                                current_display = display.clone();
                                asr_unavailable_notified = false;
                                sink.push(UiEvent::AsrDevice(
                                    format!("{display} [cpu]"),
                                ));
                                // AH-3：切换后增量会话状态复位——旧引擎的
                                // committed_tail/active 对新引擎输出无意义，且
                                // 切换后首个收尾段经 commit_interim_final 绕过
                                // 语言过滤，残留 active 会放行语言不符文本
                                interim_state.reset();
                                interim.last_interim_samples.store(0, Ordering::Relaxed);
                                interim.last_check_ms.store(0, Ordering::Relaxed);
                                // 日志按引擎打实际模型键（whisper 打 funasr_model 会误导诊断）
                                let model_key =
                                    engine_model_key(&engine, &funasr_model, &whisper_model_size);
                                // AH-8/D-28 + R18：段长钳制随总线发布派生生效——
                                // shell 在引擎切换路径 publish 后，capture 下一轮
                                // 读格应用（overlay 视图：raw 永不变、切离即恢复，
                                // 替代旧"直接改共享 VAD + 用户下次应用恢复"写穿）
                                tracing::info!("引擎已切换: {engine}/{model_key}");
                            }
                        }
                        None => {
                            // 未缓存/未知档：旧引擎继续运行——不发
                            // AsrUnavailable（P0-4），恢复标签并落日志；
                            // 面板侧缓存卡片「未缓存 + 下载按钮」给出下一步
                            sink.push(UiEvent::AsrDevice(format!(
                                "{} [cpu]",
                                current_display
                            )));
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
        // W4：每段生效视图一次 load（语言过滤/同语言判定/翻译提交全部据此）
        let eff = bus.load();
        match source {
            SegmentSource::Interim => {
                // 增量通道（原版 main.py:1720-1724）：排空重复标记 → 锁 VAD
                // peek/识别/裁剪 → 更新消费进度（capture 线程的 elapsed 由此起算）
                drain_interim_duplicates(&segment_queue);
                run_interim_pass(
                    &mut manager,
                    &vad,
                    &mut interim_state,
                    &eff.asr_lang,
                    &eff.tl.target_language,
                    tl.as_deref(),
                    &sink,
                    &msg,
                );
                let samples = { vad.lock().unwrap().speech_samples() };
                interim
                    .last_interim_samples
                    .store(samples as u64, Ordering::Relaxed);
            }
            SegmentSource::VadFlush => {
                if audio.is_empty() {
                    continue;
                }
                let seg_seconds = audio.len() as f64 / lt_audio::TARGET_RATE as f64;
                let t0 = std::time::Instant::now();
                match manager.transcribe(&audio, false, &eff.asr_lang) {
                    Ok(result) => {
                        asr_unavailable_notified = false;
                        let asr_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        if interim_state.active {
                            // 收尾段（原版 _process_interim_final 的 Ok(result) 分支）：
                            // 回声剥离 → pending 前置拼接 → 噪声过滤 → 提交
                            commit_interim_final(
                                &mut interim_state,
                                &eff.asr_lang.language,
                                &eff.tl.target_language,
                                tl.as_deref(),
                                &sink,
                                &result.text,
                                &result.language,
                                asr_ms,
                                seg_seconds,
                                &msg,
                            );
                        } else if let Some(reason) = reject_segment(
                            &result.text,
                            seg_seconds,
                            &eff.asr_lang.language,
                            &result.language,
                        ) {
                            // 段级三层过滤（原版 _process_segment：空/纯标点 → 噪声 → 语言）
                            match reason {
                                REJECT_LANGUAGE => {
                                    // 预览按字符截断，避免切坏 UTF-8 边界
                                    let preview: String = result.text.chars().take(60).collect();
                                    tracing::info!(
                                        "语言过滤: 期望 {:?} 但识别为 {:?}，丢弃: {preview}",
                                        eff.asr_lang.language,
                                        result.language
                                    );
                                }
                                REJECT_NOISE => tracing::debug!(
                                    "噪声过滤: {seg_seconds:.1}s 段仅产出 {:?}",
                                    result.text
                                ),
                                _ => tracing::debug!(
                                    "ASR 返回空/纯标点结果，跳过: {:?}",
                                    result.text
                                ),
                            }
                        } else {
                            commit_text(
                                &eff.asr_lang.language,
                                &eff.tl.target_language,
                                tl.as_deref(),
                                &sink,
                                &result.text,
                                &result.language,
                                asr_ms,
                                &msg,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("ASR 段识别失败: {e}");
                        if e.unavailable() && !asr_unavailable_notified {
                            asr_unavailable_notified = true;
                            sink.push(UiEvent::AsrUnavailable);
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
/// `eff` 为设置总线生效视图（W4：语言过滤/transcribe 前应用同源，取代
/// 旧运行时镜像 asr_language）。
#[allow(clippy::too_many_arguments)]
fn run_interim_pass(
    manager: &mut AsrManager,
    vad: &Arc<Mutex<VadProcessor>>,
    st: &mut InterimState,
    eff: &AsrEffectiveSettings,
    target_language: &str,
    tl: Option<&TlRig>,
    sink: &EventSink,
    msg: &Msg,
) -> bool {
    // ① 锁内 peek，立即解锁（原版 with self._vad_lock: peek_buffer）；
    // 代际随行（AH-4/D-27）——识别期间 VAD 可能被 capture 线程收段/切分
    let peek = vad.lock().unwrap().peek_buffer();
    let Some((audio, duration, generation)) = peek else {
        return false;
    };
    if duration < 1.5 {
        return false;
    }
    // ② 识别（原版 use_word_ts=False：词级时间戳对重复增量通道太贵）
    let t0 = Instant::now();
    let Ok(result) = manager.transcribe(&audio, false, eff) else {
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
        lt_audio::TARGET_RATE as usize,
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
        commit_text(
            &eff.language,
            target_language,
            tl,
            sink,
            &text,
            &result.language,
            asr_ms,
            msg,
        );
        committed = true;
    }
    if !committed {
        return false;
    }
    // ⑦ 裁剪已消费音频（带代际校验，AH-4/D-27：识别期间 VAD 已收段/切分则
    // 放弃裁剪，防误裁新段头部；committed_tail 回声剥离仍兜底收尾段重复）
    // + 记录 tail（末 50 字符）+ 置 active
    if trim > 0 {
        vad.lock().unwrap().trim_front_checked(trim, generation);
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
        trim as f64 / lt_audio::TARGET_RATE as f64
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
    asr_language: &str,
    target_language: &str,
    tl: Option<&TlRig>,
    sink: &EventSink,
    raw_text: &str,
    lang: &str,
    asr_ms: f64,
    seg_seconds: f64,
    msg: &Msg,
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
    commit_text(
        asr_language,
        target_language,
        tl,
        sink,
        &text,
        lang,
        asr_ms,
        msg,
    );
}

/// 提交一条已确定文本（原版 `_process_segment_text` 尾部等价）：AddMessage →
/// 统计/转写落盘 → 同语言直显空译文或提交翻译。开头复做空/纯标点与语言过滤
/// （原版 `_process_segment_text` 即如此；vad_flush 整段路径调用前已过三层过滤，
/// 此处检查冗余但无害——1:1 保留原版的双层防线）。
#[allow(clippy::too_many_arguments)]
fn commit_text(
    asr_language: &str,
    target_language: &str,
    tl: Option<&TlRig>,
    sink: &EventSink,
    text: &str,
    lang: &str,
    asr_ms: f64,
    msg: &Msg,
) {
    let original_text = text.trim();
    if original_text.is_empty() || !original_text.chars().any(|c| c.is_alphanumeric()) {
        tracing::debug!("ASR 返回空/纯标点结果，跳过: {original_text:?}");
        return;
    }
    if asr_language != "auto" && lang != asr_language {
        // 预览按字符截断，避免切坏 UTF-8 边界
        let preview: String = original_text.chars().take(60).collect();
        tracing::info!(
            "语言过滤: 期望 {:?} 但识别为 {:?}，丢弃: {preview}",
            asr_language,
            lang
        );
        return;
    }
    let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
    let id = uuid::Uuid::new_v4().as_u128() as u64;
    sink.push(UiEvent::AddMessage {
        id,
        timestamp: timestamp.clone(),
        original: original_text.to_string(),
        lang: lang.to_string(),
        asr_ms,
    });

    // ── 翻译分流（原版 _process_segment_text 尾部；字幕窗 extra_langs 随 M4 接入）──
    let Some(rig) = tl else {
        // 翻译装置未就绪（配置无效已被 TranslatorUnavailable 提醒）：译文行立即
        // 给出明确占位，不停留在永久的「翻译中...」——P0-2
        sink.push(UiEvent::UpdateTranslation {
            id,
            text: msg.t("translator_unavailable_placeholder"),
            tl_ms: 0.0,
        });
        return;
    };
    rig.stats.asr_count.fetch_add(1, Ordering::Relaxed);
    rig.transcript.write_original(id, &timestamp, original_text);
    if lang == target_language {
        tracing::info!("Same language ({lang}), no translation");
        rig.transcript.finalize_no_translation(id);
        sink.push(UiEvent::UpdateTranslation {
            id,
            text: String::new(),
            tl_ms: 0.0,
        });
        sink.push(rig.stats.snapshot_event());
    } else {
        rig.submit_translation(sink, id, original_text.to_string(), lang.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_artery::EventArtery;

    // ── build_worker_config：funasr 按 entry.key 分派（WP-A）──

    /// 造 manifest「完整」的快照：稀疏文件（set_len 即达下限，秒级无磁盘占用）
    fn sparse_manifest(models_dir: &std::path::Path, entry: &registry::ModelEntry) {
        let snap = lt_models::cache::hf_style_snapshot(
            models_dir,
            lt_download::Hub::Hf,
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
            build_worker_config(&base, "funasr", "funasr-nano-2512", 0.5, "auto", "", 2.0)
                .expect("nano 已缓存应可装配");
        assert_eq!(cfg.engine, "nano");
        assert_eq!(cfg.pad_seconds, None, "nano 无 padding 语义");
        assert_eq!(display, "Fun-ASR-Nano");
        assert!(matches!(cfg.options, lt_asr::WorkerOptions::ModelDir(_)));

        let (cfg2, display2) =
            build_worker_config(&base, "funasr", "sensevoice-small", 0.5, "auto", "", 2.0)
                .expect("sensevoice 已缓存应可装配");
        assert_eq!(cfg2.engine, "sensevoice");
        assert_eq!(cfg2.pad_seconds, Some(0.5));
        assert_eq!(display2, "SenseVoice Small");

        // mlt 无注册表条目（D-14）→ 回退 sensevoice-small 条目装配
        let (cfg3, display3) = build_worker_config(
            &base,
            "funasr",
            "funasr-mlt-nano-2512",
            0.5,
            "auto",
            "",
            2.0,
        )
        .expect("mlt 回退 sensevoice 应可装配");
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
        let (cfg, display) = build_worker_config(&base, "qwen3", "", 0.5, "auto", "", 2.0)
            .expect("qwen3 已缓存应可装配");
        assert_eq!(cfg.engine, "qwen3");
        assert_eq!(cfg.pad_seconds, None, "qwen3 无 padding 语义");
        assert_eq!(display, "Qwen3-ASR-0.6B");
        assert!(matches!(cfg.options, lt_asr::WorkerOptions::ModelDir(_)));

        // 日志模型键分派（打错键会误导诊断）
        assert_eq!(
            engine_model_key("qwen3", "funasr-nano-2512", "tiny"),
            "qwen3-asr-0.6b"
        );
        assert_eq!(engine_model_key("whisper", "x", "tiny"), "tiny");
        assert_eq!(
            engine_model_key("funasr", "sensevoice-small", "tiny"),
            "sensevoice-small"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    // ── TlRig::from_settings：翻译装置构建（M3 装配） ──

    /// AH-6/H10 分派臂防线：`ASR_ENGINES` 里每个引擎在缓存齐全时都必须能
    /// 装配——新增引擎若漏改 build_worker_config（`_ => None` 盲兜），此测试
    /// 立即红。qwen3 曾漏改数据页扫描（H9），证明分派臂漂移是现实风险。
    #[test]
    fn every_registered_engine_assembles_when_cached() {
        let dir = tmp_models_dir("all_engines");
        sparse_manifest(&dir, &registry::SENSEVOICE_SMALL);
        sparse_manifest(&dir, &registry::FUNASR_NANO);
        sparse_manifest(&dir, &registry::QWEN3_ASR);
        write_tiny_cache(&dir);
        for engine in lt_proto::settings::ASR_ENGINES {
            let got = match engine {
                "funasr" => {
                    build_worker_config(&dir, engine, "sensevoice-small", 0.5, "auto", "", 0.5)
                }
                "whisper" => build_worker_config(&dir, engine, "", 0.5, "auto", "tiny", 0.5),
                "qwen3" => build_worker_config(&dir, engine, "", 0.5, "auto", "", 0.5),
                other => panic!("引擎 {other:?} 未接入 build_worker_config 分派（AH-6/H10）"),
            };
            assert!(got.is_some(), "引擎 {engine} 缓存齐全时应可装配");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── AH-8/D-28：qwen3 段长钳制（W4 起随设置总线 overlay 派生，测试见
    // settings_bus.rs 的 clamp_is_overlay_never_write_through / clamp_only_* ──

    /// 测试监督器（sink 丢弃；用后 join_all 回收 monitor）
    fn test_sup() -> Arc<Supervisor> {
        Supervisor::new(|_| {})
    }

    /// 测试转录句柄（临时目录；测试内不落盘启用——TranscriptWriter 仅记录语义）
    fn test_transcript() -> Arc<lt_audio::transcript::TranscriptWriter> {
        Arc::new(lt_audio::transcript::TranscriptWriter::new(tmp_models_dir("transcript")))
    }

    /// 测试文案服务（键名原样返回，避免测试组依赖真实 i18n）
    fn test_msg() -> Msg {
        Msg::new(|k| k.to_string())
    }

    /// 测试设置总线（W4：from_settings 改读总线；发布即版本 1）
    fn test_bus(settings: lt_proto::Settings) -> Arc<SettingsBus> {
        Arc::new(SettingsBus::new(settings))
    }

    /// 轮询等待（W1 泄漏回归专用）：worker 出生/退出均异步，条件 3s 内应成立
    fn wait_for(cond: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cond() {
            assert!(Instant::now() < deadline, "等待条件超时（3s）");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn tl_rig_builds_from_default_settings() {
        let settings = lt_proto::Settings::default();
        let sup = test_sup();
        let bus = test_bus(settings);
        let rig = TlRig::from_settings(&bus, &sup, EventArtery::new(), test_transcript(), test_msg())
            .expect("默认设置不应报配置错误")
            .expect("默认 settings 带一个默认模型，应能构建");
        // W4：目标语言不再存实例态（逐调用经总线 tl 视图传入）——验证总线视图
        assert_eq!(bus.load().tl.target_language, "zh");
        // 收尾：先停池再 join——worker 以 stopped 为退出条件，不先置标志
        // join_all 将无限等待（pop_timeout 永不返回）
        rig.pool.shutdown();
        sup.join_all();
    }

    #[test]
    fn tl_rig_none_when_active_model_out_of_bounds() {
        let settings = lt_proto::Settings {
            active_model: 99,
            ..Default::default()
        };
        let sup = test_sup();
        assert!(TlRig::from_settings(&test_bus(settings), &sup, EventArtery::new(), test_transcript(), test_msg()).unwrap().is_none());
        sup.join_all();
    }

    #[test]
    fn tl_rig_none_when_models_empty() {
        let mut settings = lt_proto::Settings::default();
        settings.models.clear();
        let sup = test_sup();
        assert!(TlRig::from_settings(&test_bus(settings), &sup, EventArtery::new(), test_transcript(), test_msg()).unwrap().is_none());
        sup.join_all();
    }

    #[test]
    fn job_pool_drops_jobs_after_shutdown() {
        use std::sync::atomic::AtomicU64;
        let sup = test_sup();
        let pool = JobPool::new(2, &sup, EventArtery::new());
        pool.shutdown();
        sup.join_all();
        // shutdown 之后的提交不执行（submit 侧短路 + worker 侧双重检查）
        let ran = Arc::new(AtomicU64::new(0));
        let r = ran.clone();
        pool.submit(move || {
            r.fetch_add(1, Ordering::Relaxed);
        });
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(ran.load(Ordering::Relaxed), 0);
    }

    /// W1 泄漏回归（ReplaceRig 路径）：旧 rig 被替换 Drop 后，旧池 8 个 worker
    /// 必须在 ≤500ms 节拍内全部退出——否则它们永驻 Supervisor entries，
    /// Pipeline::stop 的 join_all 永久挂起（应用退出即僵尸进程）。
    #[test]
    fn replaced_rig_workers_shutdown_on_drop() {
        let sup = test_sup();
        let bus = test_bus(lt_proto::Settings::default());
        let rig = TlRig::from_settings(&bus, &sup, EventArtery::new(), test_transcript(), test_msg()).unwrap().unwrap();
        let old_alive = rig.pool.alive_workers.clone();
        wait_for(|| old_alive.load(Ordering::Relaxed) == TL_POOL_WORKERS);
        // 模拟 ReplaceRig 的替换语义（route_translator_switch：
        // `*tl = Some(Arc::new(rig))`——旧 rig 被 Drop，无人显式关机）
        let replacement = TlRig::from_settings(&bus, &sup, EventArtery::new(), test_transcript(), test_msg()).unwrap().unwrap();
        wait_for(|| replacement.pool.alive_worker_count() == TL_POOL_WORKERS);
        drop(rig);
        // 旧池经 JobPool::Drop 自动停机：3s 内应归零（500ms pop_timeout 节拍）
        wait_for(|| old_alive.load(Ordering::Relaxed) == 0);
        // 收尾：新池停机 + join 全部（含已退出的旧 worker，即刻返回不挂起）
        replacement.pool.shutdown();
        sup.join_all();
    }

    /// W1 泄漏回归（TestTranslator 路径）：临时 rig 被任务闭包 move 后，任务
    /// 执行完毕闭包 Drop → rig Drop → 池子 Drop，worker 同样必须全部退出
    /// （曾做到每次「测试连接」泄漏 8 个线程并阻断退出）。
    #[test]
    fn test_rig_dropped_with_job_shuts_down_pool() {
        let sup = test_sup();
        let bus = test_bus(lt_proto::Settings::default());
        let rig = TlRig::from_settings(&bus, &sup, EventArtery::new(), test_transcript(), test_msg()).unwrap().unwrap();
        let alive = rig.pool.alive_workers.clone();
        wait_for(|| alive.load(Ordering::Relaxed) == TL_POOL_WORKERS);
        // 等价于任务闭包 Drop 时 rig 的丢弃语义
        drop(rig);
        wait_for(|| alive.load(Ordering::Relaxed) == 0);
        sup.join_all();
    }

    // ── reject_segment：三层过滤（对照原版 _process_segment） ──

    #[test]
    fn empty_text_rejected() {
        assert_eq!(reject_segment("", 1.0, "auto", "zh"), Some(REJECT_EMPTY));
    }

    #[test]
    fn punctuation_only_rejected() {
        // 中英文纯标点均无字母数字字符 → 第 1 层拒绝
        assert_eq!(
            reject_segment("。。。", 1.0, "auto", "zh"),
            Some(REJECT_EMPTY)
        );
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
        assert_eq!(
            reject_segment("嗯。嗯。嗯", 3.5, "auto", "zh"),
            Some(REJECT_NOISE)
        );
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
        assert_eq!(
            reject_segment("hello world", 1.0, "zh", "en"),
            Some(REJECT_LANGUAGE)
        );
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
        assert_eq!(cfg.language, "auto");
        // whisper 分支必须用 whisper_pad 而非 sensevoice pad
        assert_eq!(cfg.pad_seconds, Some(0.7));
        let cfg_mp = cfg.options.clone();
        let model_path = match cfg_mp {
            lt_asr::WorkerOptions::ModelPath(p) => p,
            other => panic!("whisper 应带 ModelPath，实际 {other:?}"),
        };
        assert!(
            model_path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("main/ggml-tiny-q5_1.bin"),
            "{}",
            model_path.display()
        );
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
        let got = build_worker_config(
            dir.parent().unwrap(),
            "whisper",
            "",
            0.5,
            "zh",
            &bin.to_string_lossy(),
            0.6,
        );
        let (cfg, display) = got.expect("存在的本地路径应直通");
        assert_eq!(cfg.engine, "whisper");
        let lp = match &cfg.options {
            lt_asr::WorkerOptions::ModelPath(p) => p.to_path_buf(),
            other => panic!("whisper 应带 ModelPath，实际 {other:?}"),
        };
        assert_eq!(lp, bin);
        assert_eq!(display, "my-whisper");
        assert_eq!(cfg.pad_seconds, Some(0.6));
        // 不存在的本地路径 → None（合成路径 temp 派生，path-hygiene PH-2 补漏）
        let missing = std::env::temp_dir().join("lt_local").join("no-model.bin");
        assert!(build_worker_config(
            &dir,
            "whisper",
            "",
            0.5,
            "zh",
            missing.to_str().unwrap(),
            0.5
        )
        .is_none());
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
        assert!(matches!(cfg.options, lt_asr::WorkerOptions::ModelDir(_)));
        // manifest 不齐（缺 tokens.txt）→ 不得装配（防半截目录进引擎）
        std::fs::remove_file(ms.join("tokens.txt")).unwrap();
        assert!(
            build_worker_config(&dir, "funasr", "sensevoice-small", 0.4, "auto", "tiny", 0.5)
                .is_none()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
