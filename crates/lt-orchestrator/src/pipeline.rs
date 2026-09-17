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
use arc_swap::ArcSwap;
use lt_asr::{AsrEffectiveSettings, AsrManager, WorkerConfig};
use lt_audio::audio::capture::VadSource;
use lt_audio::audio::wasapi_win::WasapiBackend;
use lt_audio::interim::{
    is_short_utterance, pending_merge, split_sentences, strip_committed_overlap, trim_samples,
    InterimState,
};
use lt_audio::{
    AudioBackend, BoundedDropQueue, CaptureLoop, InterimControl, SegmentSource, VadProcessor,
};
use lt_models::registry;
use lt_proto::{
    AudioRole, CaptureEvent, EngineKey, FailureKind, ModelFault, MonitorSample, QueueId,
    SkipReason, ThreadRole, UiEvent, ASR_ENGINES,
};
use lt_translate::Translator;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
/// 队列任务：被丢弃时**自动补回执**（第二轮评审 ⑬d）——队列满丢最旧、停机清队
/// 都会走 `Drop`，旧实现下被丢任务不产生任何事件，字幕永远停在"翻译中…"。
/// 正常执行时闭包被取走（`run` 变 None），Drop 无副作用。
struct TlJob {
    id: u64,
    sink: EventSink,
    /// 转录句柄（2026-09-11 评审修复）：任务从未执行就被丢弃时，`write_original`
    /// 已把原文放进 `pending`，但无人 finalize → 该段原文在 all 转录文件里永久
    /// 消失。丢弃回执与收尾共用同一条件（非停机），保证"有回执就有落盘"。
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 停机中丢队的任务不再回执（事件无处可去，且不是"积压丢弃"语义）
    stopped: Arc<AtomicBool>,
    /// 装置被替换（D-85/F3）：被替换时出队的在队任务按"换模型让位"回执，
    /// 而不是"队列积压"——后者是假原因，用户只是换了个模型
    superseded: Arc<AtomicBool>,
    run: Option<Box<dyn FnOnce() + Send>>,
}

impl TlJob {
    fn run(mut self) {
        if let Some(f) = self.run.take() {
            // ACR-5：执行期 panic 视同任务丢失——补回执 + 转录收口。worker 不再
            // 因此死亡（`catch_unwind` 就地兜住）：监督器看不到死亡、不重生，
            // 后续任务由同一 worker 继续服务（比"重生"更好的结果）。
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                self.finalize_dropped(FailureKind::Dropped, panic_detail(payload.as_ref()));
            }
        }
    }

    /// 丢弃收口（ACR-5 抽公共体：`Drop` 与 panic 分支共用）：转录 all 文件补
    /// "无译文"块（原文不丢）+ UI 回执。
    /// 收口与回执当前同条件（非停机）——停机中事件无处可去，且不是"积压丢弃"
    /// 语义；ACR-1c 起收口改无条件、只保留回执的条件判断（只改这一处）。
    fn finalize_dropped(&self, kind: FailureKind, detail: String) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        self.transcript.finalize_no_translation(self.id);
        self.sink.push(UiEvent::TranslationFailed {
            id: self.id,
            kind,
            detail,
            tl_ms: 0.0,
        });
    }
}

/// panic 载荷 → 可读文本（ACR-5）：`String` / `&str` 两型，其余兜底文案。
/// 只做 downcast 与克隆，自身不再 panic。
fn panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "panic（载荷非字符串）".to_string()
    }
}

impl Drop for TlJob {
    fn drop(&mut self) {
        if self.run.is_some() {
            // 段从未翻译：转录收口 + 回执（让位路径文案不同，见 superseded）
            let superseded = self.superseded.load(Ordering::Relaxed);
            self.finalize_dropped(
                if superseded {
                    FailureKind::Superseded
                } else {
                    FailureKind::Dropped
                },
                if superseded {
                    "已切换模型，本段未翻译".into()
                } else {
                    "队列积压，保留最新（本段已放弃）".into()
                },
            );
        }
    }
}

struct JobPool {
    queue: Arc<BoundedDropQueue<TlJob>>,
    stopped: Arc<AtomicBool>,
    /// 转录句柄（随任务下发：丢弃路径补 finalize，见 [`TlJob`]）
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 装置被替换标记（D-85/F3：retire 前置位 → 出队任务回执"已切换模型"而非
    /// "队列积压"）。队列溢出路径不受影响（那时本标记仍为 false）
    superseded: Arc<AtomicBool>,
    /// 存活 worker 计数（RAII 增减）：泄漏回归测试的观测面
    #[allow(dead_code)]
    alive_workers: Arc<AtomicUsize>,
    /// R15② 水位事件出口（W2：慢 LLM 积压从"仅日志 warn"升级为类型化事件）
    sink: EventSink,
    /// 已上报丢弃计数（与队列丢弃告警同节奏，≥200 条报一次，防洪泛）
    reported: AtomicU64,
}

impl JobPool {
    fn new(
        workers: usize,
        sup: &Supervisor,
        sink: EventSink,
        transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    ) -> Self {
        let queue = Arc::new(BoundedDropQueue::<TlJob>::new(TL_QUEUE_CAP, "tl-job"));
        let stopped = Arc::new(AtomicBool::new(false));
        let superseded = Arc::new(AtomicBool::new(false));
        let alive_workers = Arc::new(AtomicUsize::new(0));
        for i in 0..workers {
            let queue = queue.clone();
            let stopped = stopped.clone();
            let alive_workers = alive_workers.clone();
            // spawn_retirable：池被替换/停机时 worker 是**正常退出**，交给监督器
            // 的"预期退役"信号静默收割——否则每次换模型都刷 8 条假错误日志
            sup.spawn_retirable(
                ThreadRole::TlWorker,
                format!("lt-tl-{i}"),
                Policy::backoff(),
                stopped.clone(),
                move || {
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
                                Some(job) => job.run(),
                                None => continue,
                            }
                        }
                    })
                },
            );
        }
        Self {
            queue,
            stopped,
            transcript,
            superseded,
            alive_workers,
            sink,
            reported: AtomicU64::new(0),
        }
    }

    /// 提交翻译任务：队列满时丢最旧（R15①/D-64），被丢的那条由 [`TlJob::drop`]
    /// 自动补一条"已放弃"回执（⑬d）。停止后仍可能有在途提交——worker 已退出
    /// 不再消费，任务滞留队列随 Pipeline 释放（其时 stopped 已置位，不补回执）。
    /// R15②：丢弃达上报节拍（≥200 条）即发 [`UiEvent::QueuePressure`]——
    /// 与 BoundedDropQueue 告警节奏同源的类型化水位信号
    fn submit(&self, id: u64, job: impl FnOnce() + Send + 'static) {
        self.queue.push(TlJob {
            id,
            sink: self.sink.clone(),
            transcript: self.transcript.clone(),
            stopped: self.stopped.clone(),
            superseded: self.superseded.clone(),
            run: Some(Box::new(job)),
        });
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

    /// **装置被替换**前的收尾：把在队任务全部取出并**补发"已放弃"回执**
    /// （在 `stopped` 置位前丢弃，[`TlJob::drop`] 才会回执）。停机路径不走这里
    /// ——停机时事件无处可去，也不该刷一堆失败。
    fn retire(&self) {
        // 先声明"这是换模型让位"——出队任务的 Drop 回执据此改文案与分类
        self.superseded.store(true, Ordering::Relaxed);
        while self.queue.try_pop().is_some() {
            // 出队即（在 stopped=false 下）Drop → 回执
        }
        self.shutdown();
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
    /// 累计金额的两个账本，单位 1e-9（整数累加，避免浮点原子）；币种按**每笔
    /// 调用各自配置**的生效币种归账（D-85：跨币种不做汇率折算，各记各的）
    cost_nano_cny: AtomicU64,
    cost_nano_usd: AtomicU64,
    /// 本次运行是否**出现过**用量未知的调用（true = 费用不完整，界面标"部分未知"）
    usage_unknown_seen: AtomicBool,
    /// 是否至少观察到过一次可用用量（区分"未知"与"确为零"——旧实现只有
    /// 唯一失败端点会显示 0 冒充真实值，对抗审计点）
    usage_any_known: AtomicBool,
}

/// 金额累加单位（1 美元 = 1e9 nano）
const NANO_PER_UNIT: f64 = 1e9;

impl TlStats {
    /// 会话级累计器（D-85：由 Pipeline 建一次，跨装置重建共享——**重启归零**）
    fn new() -> Self {
        Self {
            asr_count: AtomicU64::new(0),
            tl_count: AtomicU64::new(0),
            prompt_tokens: AtomicU64::new(0),
            completion_tokens: AtomicU64::new(0),
            cost_nano_cny: AtomicU64::new(0),
            cost_nano_usd: AtomicU64::new(0),
            usage_unknown_seen: AtomicBool::new(false),
            usage_any_known: AtomicBool::new(false),
        }
    }

    /// 记一笔翻译：tokens 与金额都按**本次调用**的价格/币种累加
    /// （换过模型之后，先前那几笔仍按旧单价记着——总额才是真实花销）
    fn record_translation(
        &self,
        pt: u64,
        ct: u64,
        usage_known: bool,
        prices: (f64, f64),
        currency: lt_proto::Currency,
    ) {
        self.prompt_tokens.fetch_add(pt, Ordering::Relaxed);
        self.completion_tokens.fetch_add(ct, Ordering::Relaxed);
        self.tl_count.fetch_add(1, Ordering::Relaxed);
        if usage_known {
            self.usage_any_known.store(true, Ordering::Relaxed);
        } else {
            self.usage_unknown_seen.store(true, Ordering::Relaxed);
        }
        let amount = lt_translate::compute_cost(pt, ct, prices.0, prices.1);
        let nano = (amount * NANO_PER_UNIT).round().max(0.0) as u64;
        match currency {
            lt_proto::Currency::Cny => {
                self.cost_nano_cny.fetch_add(nano, Ordering::Relaxed);
            }
            lt_proto::Currency::Usd => {
                self.cost_nano_usd.fetch_add(nano, Ordering::Relaxed);
            }
        }
    }

    fn snapshot_event(&self) -> UiEvent {
        UiEvent::UpdateStats {
            asr_n: self.asr_count.load(Ordering::Relaxed),
            tl_n: self.tl_count.load(Ordering::Relaxed),
            prompt_tokens: self.prompt_tokens.load(Ordering::Relaxed),
            completion_tokens: self.completion_tokens.load(Ordering::Relaxed),
            cost_cny: self.cost_nano_cny.load(Ordering::Relaxed) as f64 / NANO_PER_UNIT,
            cost_usd: self.cost_nano_usd.load(Ordering::Relaxed) as f64 / NANO_PER_UNIT,
            // 语义（D-85）：true = 本次运行所有调用都给了用量；false = 出现过未知
            // （或**从未**观测到任何用量——此时界面显示 "—"）
            usage_known: !self.usage_unknown_seen.load(Ordering::Relaxed)
                && self.usage_any_known.load(Ordering::Relaxed),
        }
    }
}

/// W3：一次翻译尝试的产出（流式增量已即时推送，此处只汇总结论/用量）
pub(crate) struct Attempt {
    pub(crate) text: Option<String>,
    pub(crate) error: Option<lt_translate::TranslateError>,
    pub(crate) verdict: Option<lt_translate::ResponseVerdict>,
    pub(crate) usage: (u64, u64),
    /// 服务端是否返回了用量统计（⑩：不返回时界面显示"—"）
    pub(crate) usage_known: bool,
}

impl Attempt {
    /// 拿到了可用正文（体检结论为准；无结论时退回"有文本即成功"）
    pub(crate) fn succeeded(&self) -> bool {
        self.error.is_none()
            && match self.verdict {
                Some(v) => v.has_text(),
                None => self.text.as_deref().is_some_and(|t| !t.trim().is_empty()),
            }
    }

    /// 未发起任何请求的占位（D-85：阶梯在"每次尝试之前"被取消/预算耗尽时
    /// 用它填 [`LadderOutcome::attempt`]）——`succeeded() == false` 且无错误，
    /// 调用方按 `halted` 分流，不得把它当"翻译失败"渲染
    fn halted(_halt: Halt) -> Self {
        Self {
            text: None,
            error: None,
            verdict: None,
            usage: (0, 0),
            usage_known: false,
        }
    }
}

/// 部分结果的推送节流间隔（第二轮评审 ⑬c）：每个增量都推"累积全文"是
/// O(n²) 字节，长输出会挤占动脉容量；节流后 UI 仍平滑，队列不再被单一
/// 句子灌满。最终译文由 [`UiEvent::UpdateTranslation`] 单独送达，不失真。
const PARTIAL_THROTTLE: Duration = Duration::from_millis(50);

/// 跑一次翻译：逐增量推送事件（节流），返回结论与用量（失败时保留 provider 错误）
#[allow(clippy::too_many_arguments)] // 局部参数面：装置/文本/语言/超时/出口/标识
fn run_attempt(
    translator: &Translator,
    text: &str,
    source_lang: &str,
    target: &str,
    timeout: u32,
    sink: &EventSink,
    id: u64,
    seq: u64,
    // 是否向 UI 推送流式增量（测试连接用 false——它的探测不该在界面上
    // 留下一条幽灵的"翻译中"消息）
    push_partials: bool,
) -> Attempt {
    let mut text_out: Option<String> = None;
    let mut last_push = Instant::now() - PARTIAL_THROTTLE;
    // W2/方案 §4.4：必须 while let——`for` 会移走迭代器，之后读不到结论/用量
    let mut it = translator.translate_iter(text, source_lang, target, timeout, seq);
    while let Some(item) = it.next() {
        match item {
            Ok(partial) => {
                if push_partials && last_push.elapsed() >= PARTIAL_THROTTLE {
                    sink.push(UiEvent::UpdateStreaming {
                        id,
                        partial: partial.clone(),
                    });
                    last_push = Instant::now();
                }
                text_out = Some(partial);
            }
            Err(e) => {
                if e.is_expected() {
                    tracing::warn!("Translate error: {e}");
                } else {
                    tracing::error!("Translate error: {e}");
                }
                return Attempt {
                    text: text_out,
                    error: Some(e),
                    verdict: it.verdict(),
                    usage: it.usage(),
                    usage_known: it.usage_known(),
                };
            }
        }
    }
    Attempt {
        text: text_out,
        error: None,
        verdict: it.verdict(),
        usage: it.usage(),
        usage_known: it.usage_known(),
    }
}

/// 台阶 → 装置：普通台阶只换关闭形态（会话记忆共享），最小请求另清空全部可选参数
fn translator_for_step(base: &Translator, step: lt_translate::RequestStep) -> Translator {
    match step {
        lt_translate::RequestStep::Plan(p) => base.with_plan(p),
        lt_translate::RequestStep::Minimal => base.minimal(),
    }
}

/// 这一台阶失败后是否应**再退一级**（第二轮评审 ③/④）：
/// - 请求被拒且是"参数类"错误（400/422）→ 退（服务端不认我们注入的参数）；
///   其余错误（网络/鉴权/404）与参数无关，退级只会白试；
/// - 体检显示"预算被思考吃光"且用户没显式指定方式 → 退（换一种关闭形态）。
///   `disable_thinking = false` 时起点已是链尾，`next_step` 自然无下一级。
fn should_advance(step: lt_translate::RequestStep, attempt: &Attempt, allow_verdict: bool) -> bool {
    use lt_translate::TranslateError as E;
    if let Some(e) = &attempt.error {
        return matches!(e, E::Status { code: 400, .. } | E::Status { code: 422, .. });
    }
    allow_verdict
        && matches!(
            attempt.verdict,
            Some(lt_translate::ResponseVerdict::EmptyReasoningBudget)
        )
        && !matches!(step, lt_translate::RequestStep::Minimal)
}

/// 是否应把"该模型无法关闭思维链"回执给界面（item 5）。三个前提缺一不可：
/// - `wants_disable`：用户**确实要求过**关闭——主动取消勾选时退到"不发送"本
///   就是他要的，不构成证据（否则给模型打上错误的持久标记）；
/// - `attempted_disable`：这条链**确实试过**注入关闭形态——官方保守端点
///   （api.openai.com 等）起点就是"不发"，从没试过，谈不上"关不掉"；否则会出现
///   "每翻译一段就被自动取消勾选一次"的自证预言（对抗审计实证）；
/// - `!already_marked`：没标记过（每会话每模型只回执一次）。
fn should_report_cannot_disable(
    wants_disable: bool,
    attempted_disable: bool,
    already_marked: bool,
    step: lt_translate::RequestStep,
) -> bool {
    wants_disable && attempted_disable && !already_marked && lt_translate::gives_up_disabling(step)
}

/// 配置指纹（会话记忆的失效依据）：用户改了与请求形态有关的任何一项，之前学到的
/// 台阶就不再可信——否则会出现"取消了勾选、记忆却仍注入关闭参数"或"改了密钥、
/// 却仍在用退到底的最小请求"这类界面与实际不一致（审计实证）。
fn config_fingerprint(mc: &lt_proto::ModelConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mc.api_base.hash(&mut h);
    mc.api_key.hash(&mut h);
    mc.model.hash(&mut h);
    mc.disable_thinking.hash(&mut h);
    mc.thinking_unavailable.hash(&mut h);
    mc.thinking_style.hash(&mut h);
    mc.streaming.hash(&mut h);
    mc.no_system_role.hash(&mut h);
    mc.json_response.hash(&mut h);
    mc.context_turns.hash(&mut h);
    mc.temperature.map(f64::to_bits).hash(&mut h);
    if let Some(ov) = &mc.overrides {
        for (k, v) in ov {
            k.hash(&mut h);
            v.to_string().hash(&mut h);
        }
    }
    if let Some(eb) = &mc.extra_body {
        eb.to_string().hash(&mut h);
    }
    h.finish()
}

/// 阶梯提前中止的原因（D-85：只有探测会用到——生产翻译传 [`RunCtl::none`]）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Halt {
    /// 调用方置位取消令牌（用户中断）
    Cancelled,
    /// 总预算耗尽（仍未取得结论）
    Budget,
}

/// 阶梯的运行控制（D-85）：`cancel` 每次尝试前与读取循环内均可察觉；
/// `deadline` 限定整场（含补发重试）的总时长。
#[derive(Clone, Default)]
pub(crate) struct RunCtl {
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub deadline: Option<Instant>,
}

impl RunCtl {
    /// 生产翻译路径：不取消、不限总时长（单次尝试仍受 `timeout` 约束）
    pub(crate) fn none() -> Self {
        Self::default()
    }

    /// 取消令牌是否已置位
    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|f| f.load(Ordering::Relaxed))
    }

    /// 本台阶可用的超时（秒）：用户超时与"剩余预算"取小，下限 1 秒。
    ///
    /// 向上取整（ceil）而非截断：否则 10 秒预算会被算成 9 秒，单次尝试在预算
    /// 用尽**之前**先超时退出，探测就再也到不了 `Halt::Budget` 分支——
    /// "未定论"会被误报成"连接失败·超时"。
    fn attempt_timeout(&self, timeout: u32) -> u32 {
        match self.deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
                timeout.min(secs.max(1) as u32)
            }
            None => timeout,
        }
    }

    /// 预算是否已耗尽
    fn budget_exhausted(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
}

/// 本次尝试前的提前中止判定（取消优先于预算：用户意图更值得如实回报）
pub(crate) fn halt_reason(ctl: &RunCtl) -> Option<Halt> {
    if ctl.cancelled() {
        return Some(Halt::Cancelled);
    }
    if ctl.budget_exhausted() {
        return Some(Halt::Budget);
    }
    None
}

/// 一次翻译的完整产出：末次尝试 + 实际打赢的台阶 + 跨尝试累计用量 + 提前中止原因
pub(crate) struct LadderOutcome {
    pub attempt: Attempt,
    pub step: lt_translate::RequestStep,
    pub usage: (u64, u64),
    /// `Some` = 阶梯被提前中止（未跑完）；`None` = 正常收敛（成功或退到端点）
    pub halted: Option<Halt>,
    /// 已尝试的形态数（每次 `run_attempt` 计 1，含截断补发那次）
    pub attempted: u8,
}

/// 回退阶梯（第二轮评审 ③/④/⑤）：按 [`lt_translate::next_step`] 逐级下退，
/// **成功即停**；`EmptyTruncated` 时在同一台阶补发输出上限重试一次（方案 §4.4）。
/// 阶梯由构造保证有限（每级严格前进、端点即 `Minimal`），不会成环。
///
/// D-85：每一次尝试**之前**先查取消与总预算（`ctl`）——提前中止时返回
/// `halted = Some(..)`，调用方据此区分"未定论"与"失败"。
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_ladder(
    base: &Translator,
    start: lt_translate::RequestStep,
    allow_verdict_advance: bool,
    text: &str,
    source_lang: &str,
    target: &str,
    timeout: u32,
    ctl: &RunCtl,
    sink: &EventSink,
    id: u64,
    seq: u64,
    push_partials: bool,
) -> LadderOutcome {
    let mut step = start;
    let mut total = (0u64, 0u64);
    let mut truncation_retried = false;
    let mut attempted: u8 = 0;
    loop {
        // 提前中止检查（每次尝试之前；补发重试在同轮内另有一次检查，见下）
        if let Some(halt) = halt_reason(ctl) {
            return LadderOutcome {
                attempt: Attempt::halted(halt),
                step,
                usage: total,
                halted: Some(halt),
                attempted,
            };
        }
        let device = translator_for_step(base, step);
        let attempt = run_attempt(
            &device,
            text,
            source_lang,
            target,
            ctl.attempt_timeout(timeout),
            sink,
            id,
            seq,
            push_partials,
        );
        attempted = attempted.saturating_add(1);
        total.0 += attempt.usage.0;
        total.1 += attempt.usage.1;
        if attempt.succeeded() {
            return LadderOutcome {
                attempt,
                step,
                usage: total,
                halted: None,
                attempted,
            };
        }
        // 体检判定"被截断且没有正文"：同一台阶补发输出上限重试一次（每段一次）
        if !truncation_retried
            && attempt.error.is_none()
            && matches!(
                attempt.verdict,
                Some(lt_translate::ResponseVerdict::EmptyTruncated)
            )
        {
            // 补发是"同轮内的第二次尝试"：与循环顶同款的提前中止检查必须在这里
            // 再走一遍（2026-09-11 评审修复——旧实现漏掉此检查，取消置位后仍会
            // 多放一次请求、探测总预算也可被超出一次单步超时，违反裁决 B 的
            // 10 秒封顶；方案 §4.4 明令"两次尝试都要走同一套检查"）
            if let Some(halt) = halt_reason(ctl) {
                return LadderOutcome {
                    attempt: Attempt::halted(halt),
                    step,
                    usage: total,
                    halted: Some(halt),
                    attempted,
                };
            }
            tracing::info!("体检：输出被截断，补发输出上限重试一次（{step:?}）");
            let retry = translator_for_step(base, step).with_max_tokens(4096);
            truncation_retried = true;
            let second = run_attempt(
                &retry,
                text,
                source_lang,
                target,
                // 超时同样按剩余预算钳制（与循环顶 `attempt_timeout` 同源）
                ctl.attempt_timeout(timeout),
                sink,
                id,
                seq,
                push_partials,
            );
            // 计数口径与方案一致：每调用一次 run_attempt 就 +1（含补发）
            attempted = attempted.saturating_add(1);
            total.0 += second.usage.0;
            total.1 += second.usage.1;
            if second.succeeded() {
                return LadderOutcome {
                    attempt: second,
                    step,
                    usage: total,
                    halted: None,
                    attempted,
                };
            }
            // 补发上限这次也可能被端点拒绝（400/422）——按同一套判据继续退级
            // （下一级不带 max_tokens，本可成功的情形不该被判死）
            if !should_advance(step, &second, allow_verdict_advance) {
                return LadderOutcome {
                    attempt: second,
                    step,
                    usage: total,
                    halted: None,
                    attempted,
                };
            }
            let Some(next) = lt_translate::next_step(step) else {
                return LadderOutcome {
                    attempt: second,
                    step,
                    usage: total,
                    halted: None,
                    attempted,
                };
            };
            tracing::info!("补发上限被拒，继续降级：{:?} → {:?}", step, next);
            step = next;
            continue;
        }
        if !should_advance(step, &attempt, allow_verdict_advance) {
            return LadderOutcome {
                attempt,
                step,
                usage: total,
                halted: None,
                attempted,
            };
        }
        let Some(next) = lt_translate::next_step(step) else {
            return LadderOutcome {
                attempt,
                step,
                usage: total,
                halted: None,
                attempted,
            };
        };
        tracing::info!("翻译降级：{:?} → {:?}", step, next);
        step = next;
    }
}

/// 成功收尾（原版 _translate_async 成功路径）
#[allow(clippy::too_many_arguments)]
fn finish_ok(
    attempt: &Attempt,
    transcript: &Arc<lt_audio::transcript::TranscriptWriter>,
    stats: &Arc<TlStats>,
    prices: (f64, f64),
    currency: lt_proto::Currency,
    sink: &EventSink,
    id: u64,
    t0: Instant,
    pt: u64,
    ct: u64,
) {
    let tl_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let text = attempt.text.clone().unwrap_or_default();
    // D-85：会话级累计——价格/币种按**本次调用**的配置记账
    stats.record_translation(pt, ct, attempt.usage_known, prices, currency);
    tracing::info!("Translate ({tl_ms:.0}ms): {text}");
    sink.push(UiEvent::UpdateTranslation {
        id,
        text: text.clone(),
        tl_ms,
    });
    sink.push(stats.snapshot_event());
    transcript.write_translation(id, &text);
}

/// 失败收尾（W2/结论化：带原因；用量照记——钱花了就要记账）
#[allow(clippy::too_many_arguments)]
fn fail(
    attempt: &Attempt,
    transcript: &Arc<lt_audio::transcript::TranscriptWriter>,
    stats: &Arc<TlStats>,
    prices: (f64, f64),
    currency: lt_proto::Currency,
    sink: &EventSink,
    id: u64,
    t0: Instant,
    pt: u64,
    ct: u64,
) {
    let tl_ms = t0.elapsed().as_secs_f64() * 1000.0;
    // D-85：失败也记账（钱花了就要记——旧实现只记 tokens 不算金额）
    stats.record_translation(pt, ct, attempt.usage_known, prices, currency);
    transcript.finalize_no_translation(id);
    let (kind, detail) = match &attempt.error {
        Some(e) => (e.failure_kind(), format!("{} ({tl_ms:.0}ms)", e.ui_text())),
        None => {
            let kind = match attempt.verdict {
                Some(lt_translate::ResponseVerdict::EmptyTruncated) => FailureKind::Truncated,
                _ => FailureKind::Empty,
            };
            let detail = match attempt.verdict {
                Some(v) => format!("{v:?} (pt={pt}, ct={ct}, {tl_ms:.0}ms)"),
                None => format!("no verdict (pt={pt}, ct={ct}, {tl_ms:.0}ms)"),
            };
            tracing::warn!("Translation produced no text: {detail}");
            (kind, detail)
        }
    };
    sink.push(UiEvent::TranslationFailed {
        id,
        kind,
        detail,
        tl_ms,
    });
    sink.push(stats.snapshot_event());
}

/// 会话内台阶记忆的共享句柄：`(api_base, model) → (配置指纹, 台阶)`。
/// 记"退到底"的失败台阶同样重要（否则每段重走整条阶梯），但必须与配置指纹绑定：
/// 配置一变即失效（否则会出现"取消了勾选、记忆却仍注入关闭参数"这类界面与实际不一致）。
type LearnedMap = HashMap<(String, String), (u64, lt_translate::RequestStep)>;
/// 记忆句柄（Arc 包装：跨装置重建保留、只记内存）
type Learned = Arc<Mutex<LearnedMap>>;

/// 翻译装置：Translator + 统计 + 线程池（ASR 线程与翻译 worker 共享）
struct TlRig {
    translator: Arc<Translator>,
    stats: Arc<TlStats>,
    pool: JobPool,
    /// 会话转录写盘（原版 self._transcript；ASR 线程写原文，worker 配对译文）
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 设置总线（W4：翻译 worker 提交前读 `tl` 生效视图——目标语言/超时不再
    /// 存实例可变态（旧 MutableState 设置面 + TlSwitch::TargetLanguage/Timeout 镜像）
    bus: Arc<SettingsBus>,
    /// 会话内台阶记忆（见 [`Learned`]）：跨装置重建保留、只记内存
    learned: Learned,
    /// 本装置配置的指纹（记忆命中判据）
    config_fp: u64,
    /// 已就"无法关闭思维链"回过执的模型键（每会话每模型一次，防止事件洪水）
    degraded_notified: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    /// 本装置的 (api_base, model) 记忆键
    model_key: (String, String),
    /// 模型显示名（降级回执带出，供 UI 定位条目）
    model_name: String,
    /// 上下文提交序号（单调递增；id 是 UUID，不能当序号用）
    seq: Arc<AtomicU64>,
    /// 阶梯起点与"体检驱动的降级"许可（按用户配置算一次）
    start_step: lt_translate::RequestStep,
    allow_verdict_advance: bool,
    /// 配置里已声明"本模型关不掉"（UI 已取消勾选并落盘）——回执不再重复发
    thinking_unavailable: bool,
    /// 用户**确实要求关闭思考**（勾选且未标记）：只有这种情形下"退到不发送"
    /// 才算"关不掉"的证据；主动取消勾选不是
    wants_disable: bool,
    /// 本装置的价格（每 1M tokens）与生效币种（D-85：按笔记账用）
    prices: (f64, f64),
    currency: lt_proto::Currency,
    /// 本次构造用的请求参数（D-85：供"探测与生产同源"测试逐字段断言）。
    /// **生产路径不读它**——持有即语义（同源证据），故允许 dead_code
    #[allow(dead_code)]
    params: lt_translate::TranslatorParams,
}

/// 配置 → 请求参数（D-85：**全仓唯一构造点**）。
///
/// 生产装置（[`TlRig::from_effective`]）与连接探测（[`crate::probe`]）都必须经
/// 本函数——"探测判据 == 生产判据"是第二轮评审的硬要求，两处各列一遍字段迟早
/// 漂移（`translator_params_single_source` 测试钉住这条）。
pub(crate) fn translator_params(
    mc: &lt_proto::ModelConfig,
    eff: &EffectiveSettings,
) -> lt_translate::TranslatorParams {
    lt_translate::TranslatorParams {
        api_base: mc.api_base.clone(),
        api_key: mc.api_key.clone(),
        model: mc.model.clone(),
        // W1/方案 §2.1：长度上限不再由应用发送（交给服务端默认——应用强加的
        // 256 会把"先想再答"的模型憋死，实测就是这个原因导致空译文）
        max_tokens: None,
        // 2026-09-10 裁决：高级参数默认一律不发送（None = 不发）；用户手动指定才发
        temperature: mc.temperature,
        streaming: mc.streaming,
        system_prompt: (!eff.raw.system_prompt.is_empty()).then(|| eff.raw.system_prompt.clone()),
        proxy: mc.proxy.clone(),
        no_system_role: mc.no_system_role,
        // W1/方案 §2.3：总开关 + 方式（sanitize 已把旧 "off" 归一化到总开关）
        disable_thinking: mc.disable_thinking,
        thinking_unavailable: mc.thinking_unavailable,
        thinking_style: mc.thinking_style.clone(),
        json_response: mc.json_response,
        overrides: mc.overrides.clone(),
        extra_body: mc.extra_body.clone(),
    }
}

impl TlRig {
    /// 按设置构建；models 为空/active_model 越界 → Ok(None)（不翻译，仅 ASR）；
    /// 配置无效（URL 格式错等）→ Err(原因)（必须让用户可见，见 TranslatorUnavailable）
    #[allow(clippy::too_many_arguments)]
    fn from_settings(
        bus: &Arc<SettingsBus>,
        sup: &Supervisor,
        sink: EventSink,
        transcript: Arc<lt_audio::transcript::TranscriptWriter>,
        learned: Learned,
        degraded_notified: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
        session_stats: &Arc<TlStats>,
        ui_lang: &str,
    ) -> Result<Option<Self>, String> {
        let eff = bus.load();
        let Some(mc) = eff.raw.models.get(eff.raw.active_model) else {
            return Ok(None);
        };
        Self::from_effective(
            mc,
            &eff,
            bus,
            sup,
            sink,
            transcript,
            learned,
            degraded_notified,
            session_stats,
            ui_lang,
        )
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
        learned: Learned,
        degraded_notified: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
        session_stats: &Arc<TlStats>,
        ui_lang: &str,
    ) -> Result<Option<Self>, String> {
        let params = translator_params(mc, eff);
        let translator = match Translator::new(params.clone()) {
            Ok(t) => {
                t.set_context_turns(mc.context_turns);
                Arc::new(t)
            }
            Err(e) => {
                tracing::error!("Translator 构建失败（模型 {:?}）: {e}", mc.name);
                return Err(format!("{}: {e:#}", mc.name));
            }
        };
        // 阶梯起点 = 构造期解析出的关闭形态；用户显式选定方式或取消勾选时，
        // **不允许**体检驱动的降级（尊重用户意愿，方案 §2.3 规则 1/3）——
        // 但参数被服务端拒绝（400/422）时仍会退级，否则该模型整条不可用
        let start_step = lt_translate::first_step(translator.thinking_plan());
        let explicit = mc.disable_thinking
            && !mc.thinking_unavailable
            && matches!(
                mc.thinking_style.as_deref(),
                Some(s) if !s.is_empty() && s != "auto"
            );
        let allow_verdict_advance = mc.disable_thinking && !mc.thinking_unavailable && !explicit;
        tracing::info!("Switching translator: {} ({})", mc.name, mc.model);
        Ok(Some(Self {
            translator,
            stats: session_stats.clone(),
            prices: (mc.input_price, mc.output_price),
            currency: lt_proto::effective_currency(mc.currency.as_deref(), ui_lang),
            pool: JobPool::new(TL_POOL_WORKERS, sup, sink, transcript.clone()),
            transcript,
            bus: bus.clone(),
            learned,
            degraded_notified,
            model_key: (mc.api_base.clone(), mc.model.clone()),
            config_fp: config_fingerprint(mc),
            model_name: mc.name.clone(),
            seq: Arc::new(AtomicU64::new(0)),
            start_step,
            allow_verdict_advance,
            thinking_unavailable: mc.thinking_unavailable,
            wants_disable: mc.disable_thinking && !mc.thinking_unavailable,
            params,
        }))
    }

    /// 探测与生产同源断言面（D-85：只给单测用，不进生产路径）
    #[cfg(test)]
    fn params_for_test(&self) -> &lt_translate::TranslatorParams {
        &self.params
    }

    /// 提交一段的翻译任务（对照原版 _translate_async 的成功/重复/错误三路）；
    /// 同语言不进此函数（ASR 线程直接回空译文，见 run_asr_thread）。
    /// W4：目标语言/超时在 **任务执行时** 读总线（排队期间设置变更与旧
    /// `set_target_language` 改运行时可变面的语义同为"执行时最新"——旧实现
    /// worker 开始执行才取 state 锁；此处等价且无锁）
    fn submit_translation(&self, sink: &EventSink, id: u64, text: String, source_lang: String) {
        let translator = self.translator.clone();
        let stats = self.stats.clone();
        let prices = self.prices;
        let currency = self.currency;
        let transcript = self.transcript.clone();
        let sink = sink.clone();
        // W2：`msg`（用户文案注入）在翻译出口不再需要——失败文案改由 UI 按
        // FailureKind 本地化（编排域禁依赖 lt-i18n 的纪律不变）
        let bus = self.bus.clone();
        let learned = self.learned.clone();
        let degraded_notified = self.degraded_notified.clone();
        let model_key = (self.model_key.0.clone(), self.model_key.1.clone());
        let model_name = self.model_name.clone();
        let start_step = self.start_step;
        let allow_verdict_advance = self.allow_verdict_advance;
        let thinking_unavailable = self.thinking_unavailable;
        let wants_disable = self.wants_disable;
        // "确实尝试过注入关闭形态"：起点不是 Plan(None) 才算试过（官方保守端点
        // 从设计上就不发，永不算"关不掉"）
        let attempted_disable = matches!(
            start_step,
            lt_translate::RequestStep::Plan(p) if p != lt_translate::ThinkingPlan::None
        );
        let config_fp = self.config_fp;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.pool.submit(id, move || {
            let eff = bus.load();
            let target = eff.tl.target_language.clone();
            let timeout = eff.tl.timeout;
            let t0 = Instant::now();
            // 会话内台阶记忆优先（学到的姿势跨段复用；记忆含"退到底"的失败台阶，
            // 避免每段重走整条阶梯）
            let step = learned
                .lock()
                .unwrap()
                .get(&model_key)
                .filter(|(fp, _)| *fp == config_fp)
                .map(|(_, s)| *s)
                .unwrap_or(start_step);
            let outcome = run_ladder(
                &translator,
                step,
                allow_verdict_advance,
                &text,
                &source_lang,
                &target,
                timeout,
                &RunCtl::none(), // 生产翻译：不取消、不限总时长（单次尝试仍受 timeout）
                &sink,
                id,
                seq,
                true,
            );
            let used = outcome.usage;
            if !outcome.attempt.succeeded() {
                // 整条阶梯都没打通过：记住退到底的台阶（下一段一步到位，不再重走）
                learned
                    .lock()
                    .unwrap()
                    .insert(model_key.clone(), (config_fp, outcome.step));
                fail(
                    &outcome.attempt,
                    &transcript,
                    &stats,
                    prices,
                    currency,
                    &sink,
                    id,
                    t0,
                    used.0,
                    used.1,
                );
                return;
            }
            // 记住打赢的台阶
            learned
                .lock()
                .unwrap()
                .insert(model_key.clone(), (config_fp, outcome.step));
            // ── item 5 / 方案 §2.5 规则 5「偏离可见」──────────────────────
            // 用户要求关闭思考、但阶梯最终只能退到"不含关闭参数"的形态
            // （强制思考模型）→ 回执 UI：取消勾选 + 落盘 thinking_unavailable
            // + 提示"该模型无法关闭思维链"。每会话每模型只回执一次。
            if should_report_cannot_disable(
                wants_disable,
                attempted_disable,
                thinking_unavailable,
                outcome.step,
            ) {
                let first_time = degraded_notified.lock().unwrap().insert(model_key.clone());
                if first_time {
                    tracing::warn!(
                        "模型 {model_name} 无法关闭思维链（阶梯退到 {}），已回执界面取消勾选",
                        lt_translate::step_name(outcome.step)
                    );
                    sink.push(UiEvent::TranslatorDegraded {
                        name: model_name.clone(),
                        api_base: model_key.0.clone(),
                        model: model_key.1.clone(),
                        actual: lt_translate::step_name(outcome.step).to_string(),
                        cannot_disable_thinking: true,
                    });
                }
            }
            finish_ok(
                &outcome.attempt,
                &transcript,
                &stats,
                prices,
                currency,
                &sink,
                id,
                t0,
                used.0,
                used.1,
            );
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
    ReplaceRig { config: Box<lt_proto::ModelConfig> },
    /// 运行时切换 ASR 引擎/模型（原版 _switch_asr_engine；ensure_started
    /// 内部带替换+失败回滚，此处只补路由与 UI 事件）
    ReplaceEngine {
        engine: String,
        funasr_model: String,
        /// whisper 档位（builtin 六档 | 本地 GGML 路径）
        whisper_model_size: String,
        language: String,
    },
    // D-85：`TestTranslator` 臂已删除——连接测试改由组合根经监督器起
    // 一次性线程跑 `crate::probe::run_probe`（不再占用 ASR 线程的空闲分支）
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
            sup.spawn(
                ThreadRole::Capture,
                "lt-capture",
                Policy::backoff(),
                move || {
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
                },
            );
        }

        // W3/裁决 2：会话内学习记忆（只记内存；跨装置重建保留）
        let degraded_notified: Arc<Mutex<std::collections::HashSet<(String, String)>>> =
            Arc::new(Mutex::new(std::collections::HashSet::new()));
        let learned: Learned = Arc::new(Mutex::new(HashMap::new()));

        // ── 翻译装置（M3）：models 非空即构建；配置无效必须让用户可见
        //（TranslatorUnavailable → 面板翻译页状态行 + 悬浮窗译文占位）──
        // D-85/G：会话级累计器——**本次进程只建一次**，跨装置重建共享
        //（换模型不清零；进程退出自然归零 = 用户要的"重启重算"）
        let session_stats: Arc<TlStats> = Arc::new(TlStats::new());
        let ui_lang = msg.lang();
        let tl = match TlRig::from_settings(
            bus,
            &sup,
            sink.clone(),
            transcript.clone(),
            learned.clone(),
            degraded_notified.clone(),
            &session_stats,
            &ui_lang,
        ) {
            Ok(t) => {
                // D-85/F2：启动即回执一次"当前使用"——面板状态行首帧就正确，
                // 不必等用户切一次模型才有信息
                if let Some(mc) = eff.raw.models.get(eff.raw.active_model) {
                    sink.push(UiEvent::TranslatorSwitched {
                        name: mc.name.clone(),
                        model: mc.model.clone(),
                    });
                }
                t.map(Arc::new)
            }
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
                                AudioStatus::OutputRecovered => CaptureEvent::Recovered {
                                    role: AudioRole::Loopback,
                                },
                                AudioStatus::InputRecovered => CaptureEvent::Recovered {
                                    role: AudioRole::Mic,
                                },
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
            let session_stats_asr = session_stats.clone();
            // INV3：经监督器出生；panic 重生 = 待命/装配路径干净重启（INV5）
            sup.spawn(
                ThreadRole::AsrMain,
                "lt-asr-main",
                Policy::backoff(),
                move || {
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
                    let learned = learned.clone();
                    let degraded_notified = degraded_notified.clone();
                    let session_stats = session_stats_asr.clone();
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
                                learned,
                                degraded_notified,
                                session_stats,
                            },
                            tl,
                        );
                    })
                },
            );
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
/// 非法/退役键（D-86 后含 mlt 旧档残留）统一回退 sensevoice-small（bool = true）。
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

// ── D-83 零信任加载闸门（docs/archive/model-trust-repair.md）──────────────────────
//
// 规格：每次装配模型前（启动 / 待命唤醒 / 运行时切换三条路径）逐文件复验
// 注册表 sha256；不通过 → 隔离坏文件 + 发 ModelIntegrityFailed(Hash) → 中止
// 本次装配（隔离后缓存探测自然判"缺"，自动重下的按钮/流程才真正执行）。
// 边界：指纹通过但加载失败 → ModelFault::Unloadable（重下同内容无意义）。

/// 待校验清单：显示名 + 快照根 + (相对文件名, 期望 sha256)。
/// 哈希为 owned：测试可构造合成清单（真实注册表条目内容无法伪造——
/// 那正是零信任的意义，故闸门核心逻辑用可注入清单测）
struct TrustFiles {
    display: String,
    root: std::path::PathBuf,
    files: Vec<(&'static str, String)>,
}

/// 由 worker 配置反查校验清单：**校验的正是即将加载的东西**（比从 settings
/// 重新解析更接近事实——待命唤醒路径的引擎/模型来自切换命令，可能与当前
/// settings 快照不同）。
/// None = 无登记指纹可比（whisper 本地自定义路径 / 已缓存的根不在托管目录下 /
/// Echo 假 worker）——闸门放行（与旧行为一致，仅保留"能加载"判定）。
fn trust_files_for_config(
    models_dir: Option<&std::path::Path>,
    config: &WorkerConfig,
) -> Option<TrustFiles> {
    let pairs = |e: &registry::ModelEntry| -> Vec<(&'static str, String)> {
        e.files
            .iter()
            .copied()
            .zip(e.files_sha256.iter().map(|h| (*h).to_string()))
            .collect()
    };
    match &config.options {
        lt_asr::WorkerOptions::ModelDir(dir) => {
            // worker 引擎名 → 注册表条目（sensevoice/nano 同属 funasr 家族，
            // 但清单不同；qwen3 单一模型）
            let entry = match config.engine.as_str() {
                "sensevoice" => registry::SENSEVOICE_SMALL.clone(),
                "nano" => registry::FUNASR_NANO.clone(),
                "qwen3" => registry::qwen3_entry(),
                _ => return None,
            };
            Some(TrustFiles {
                display: entry.display.into(),
                root: dir.clone(),
                files: pairs(&entry),
            })
        }
        lt_asr::WorkerOptions::ModelPath(path) => {
            // 只认托管缓存里的文件（snapshots 下）——本地自选的 GGML 路径
            // 无指纹语义，校验/隔离会误伤用户文件
            let root = path.parent()?.to_path_buf();
            if !models_dir.is_some_and(|d| root.starts_with(d)) {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            let entry = registry::WHISPER_ENTRIES
                .iter()
                .find(|e| e.files.first() == Some(&name.as_str()))?;
            Some(TrustFiles {
                display: format!("Whisper {}", entry.key),
                root,
                files: pairs(entry),
            })
        }
        lt_asr::WorkerOptions::Echo(_) => None,
    }
}

/// 零信任闸门：逐文件复验；不通过即隔离 + 发事件并返回 false（调用方中止装配）。
/// 无指纹可比 → true（闸门不介入）。
fn trust_gate(
    models_dir: Option<&std::path::Path>,
    config: &WorkerConfig,
    sink: &EventSink,
) -> bool {
    let Some(t) = trust_files_for_config(models_dir, config) else {
        return true;
    };
    verify_files(&t, sink)
}

/// 闸门核心（可注入清单，便于单测）：逐文件复验；不通过即隔离 + 发事件并
/// 返回 false。清单哈希为空串 = 未登记 → 跳过该文件。
fn verify_files(t: &TrustFiles, sink: &EventSink) -> bool {
    for (file, expected) in &t.files {
        if expected.is_empty() {
            continue; // 未登记（渐进登记期语义）→ 不校验
        }
        let path = t.root.join(file);
        match lt_download::verify_file(&path, expected) {
            Ok(true) => {}
            Ok(false) => {
                let actual = lt_download::hash_file_hex(&path).unwrap_or_default();
                let quarantined = match lt_models::cache::quarantine_file(&path) {
                    Ok(dst) => {
                        tracing::error!(
                            "模型文件校验失败：{} / {}（实际 {} ≠ 登记 {}），已隔离 → {}",
                            t.display,
                            file,
                            actual,
                            expected,
                            dst.display()
                        );
                        true
                    }
                    Err(e) => {
                        tracing::error!(
                            "模型文件校验失败且隔离失败：{} / {file}（{e}）——需人工清理缓存目录",
                            t.display
                        );
                        false
                    }
                };
                sink.push(UiEvent::ModelIntegrityFailed {
                    model: t.display.clone(),
                    fault: ModelFault::Hash {
                        file: (*file).to_string(),
                        expected: expected.clone(),
                        actual,
                        quarantined,
                    },
                });
                return false;
            }
            Err(e) => {
                // 读不了（权限/被占用/诡异消失）→ 按不可信中止，但不隔离
                //（内容未知，隔离等于把证据丢掉）
                tracing::error!("模型文件不可读：{} / {file}（{e}）", t.display);
                sink.push(UiEvent::ModelIntegrityFailed {
                    model: t.display.clone(),
                    fault: ModelFault::Hash {
                        file: (*file).to_string(),
                        expected: expected.clone(),
                        actual: format!("不可读: {e}"),
                        quarantined: false,
                    },
                });
                return false;
            }
        }
    }
    tracing::info!(
        "模型完整性校验通过：{}（{} 个文件）",
        t.display,
        t.files.len()
    );
    true
}

/// 装载失败后的复验分流（D-83 §2.4）：内容被改过 → Hash 故障（可修复循环）；
/// 内容与登记一致 → Unloadable（重下同内容无意义，只提醒）。
fn report_load_failure(
    models_dir: Option<&std::path::Path>,
    config: &WorkerConfig,
    detail: &str,
    sink: &EventSink,
) {
    if !trust_gate(models_dir, config, sink) {
        return; // 已发 Hash 故障（隔离 + 可修复）
    }
    // 变量名避开 tracing::field::display（同名会让宏把标识符解析成函数项）
    let model_name = trust_files_for_config(models_dir, config)
        .map(|t| t.display)
        .unwrap_or_else(|| config.engine.clone());
    tracing::error!(
        "模型加载失败（指纹与登记一致，重下无解）: {} / {}",
        model_name,
        detail
    );
    sink.push(UiEvent::ModelIntegrityFailed {
        model: model_name,
        fault: ModelFault::Unloadable {
            detail: detail.to_string(),
        },
    });
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
    /// W3：会话内学习记忆（跨装置重建保留；只记内存）
    learned: Learned,
    degraded_notified: Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    /// 转录写盘（ReplaceRig/TestTranslator 重建翻译装置时共享同一句柄）
    transcript: Arc<lt_audio::transcript::TranscriptWriter>,
    /// 用户可见文案服务（i18n 注入；错误占位/测试连接回执经此取）
    msg: Msg,
    /// 会话级累计器（D-85/G：`Pipeline::start` 建一次，装置替换时传承——
    /// 线程侧持有同一句柄，替代旧的"从旧装置借用/无装置时自建"回退路径）
    session_stats: Arc<TlStats>,
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
    // 文案 + 界面语言注入（D-85：币种默认值按界面语言，编排域不依赖 lt-i18n）
    msg: &Msg,
    learned: &Learned,
    degraded_notified: &Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    // 会话级累计器句柄（D-85/G 评审修复：**唯一创建点在 Pipeline::start**——
    // 旧实现在"无装置"分支自建新账本，会把会话累计静默清零）
    session_stats: &Arc<TlStats>,
) -> Option<TlSwitch> {
    match sw {
        TlSwitch::ReplaceRig { config } => {
            let eff = bus.load();
            // 会话统计与界面语言随装置传承（替换不换账本；语言取当前注入）
            let ui_lang = msg.lang();
            match TlRig::from_effective(
                &config,
                &eff,
                bus,
                sup,
                sink.clone(),
                transcript.clone(),
                learned.clone(),
                degraded_notified.clone(),
                session_stats,
                &ui_lang,
            ) {
                Ok(Some(rig)) => {
                    tracing::info!("翻译器已切换: {} ({})", config.name, config.model);
                    // 旧装置交给 Drop 之前先 retire：在队未跑的任务补发"已放弃"
                    // 回执——否则这些段落在字幕上永远停在「翻译中…」
                    // D-85/F3：retire 前置"让位"标记 → 回执是"已切换模型"而非
                    // "队列积压"（后者是假原因，用户只是换了个模型）
                    if let Some(old) = tl.take() {
                        old.pool.retire();
                    }
                    *tl = Some(Arc::new(rig));
                    // D-85/F2：切换生效回执（面板状态行据此确认"已生效"）
                    sink.push(UiEvent::TranslatorSwitched {
                        name: config.name.clone(),
                        model: config.model.clone(),
                    });
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
        engine @ TlSwitch::ReplaceEngine { .. } => Some(engine),
    }
}

/// 排空翻译器命令（D-85/F1）。
///
/// 提取自 ASR 主循环的空闲分支——**同一份实现**现在挂在主循环每轮开头：
/// 换模型不必再等"段队列连续 500ms 空窗"，下一段边界即生效
/// （旧实现只在空闲分支消费，连续说话时切换要排在转录之后）。
///
/// 待命循环**不用本函数**：那里 `ReplaceEngine` 的语义是"装配 worker 退出待命"，
/// 与运行期的"热切换 + 回滚"完全不同，保持原样。
///
/// 方案偏离留痕（docs §4.5 F1）：文档写"逐个传引用 + allow(too_many_arguments)"，
/// 实现改为把 9 个共享引用收进 [`SwitchDrain`]——参数从 14 降到 6，可读性更好，
/// 语义与调用点不变。
struct SwitchDrain<'a> {
    tl_switch: &'a crossbeam_channel::Receiver<TlSwitch>,
    bus: &'a Arc<SettingsBus>,
    sink: &'a EventSink,
    sup: &'a Arc<Supervisor>,
    transcript: &'a Arc<lt_audio::transcript::TranscriptWriter>,
    learned: &'a Learned,
    degraded_notified: &'a Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    interim: &'a Arc<InterimControl>,
    settings: &'a lt_proto::Settings,
    /// 文案 + 界面语言注入（币种默认值的语言回退在编排域算）
    msg: &'a Msg,
    /// 会话级累计器（替换装置时传承同一账本，见 route_translator_switch）
    session_stats: &'a Arc<TlStats>,
}

fn drain_tl_switch(
    tl: &mut Option<Arc<TlRig>>,
    manager: &mut AsrManager,
    current_display: &mut String,
    asr_unavailable_notified: &mut bool,
    interim_state: &mut InterimState,
    ctx: &SwitchDrain<'_>,
) {
    while let Ok(sw) = ctx.tl_switch.try_recv() {
        if let Some(TlSwitch::ReplaceEngine {
            engine,
            funasr_model,
            whisper_model_size,
            language,
        }) = route_translator_switch(
            sw,
            tl,
            ctx.bus,
            ctx.sink,
            ctx.sup,
            ctx.transcript,
            ctx.msg,
            ctx.learned,
            ctx.degraded_notified,
            ctx.session_stats,
        ) {
            // R3/D-61：每次切换尝试重解析 models_dir（与待命臂一致；
            // 运行中目录损坏时切换路径同样可恢复）
            let Ok(models_dir) = lt_models::paths::models_dir(ctx.settings.models_dir.as_deref())
            else {
                ctx.sink.push(UiEvent::AsrUnavailable);
                tracing::warn!("引擎切换尝试：模型目录不可用，跳过本轮");
                continue;
            };
            // 原版 _switch_asr_engine：装配新配置 → 加载对话框 →
            // ensure_started（失败内部回滚旧 worker）→ 设备/不可用事件
            let eff = ctx.bus.load();
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
                    ctx.sink.push(UiEvent::ModelLoadStart(display.clone()));
                    // D-83 零信任闸门（运行时切换路径）：不通过 → 坏文件
                    // 已隔离 + 事件已发；恢复旧标签（旧引擎继续工作，P0-4）
                    // 并中止本次切换——修复下载完成后 shell 会重发切换
                    if !trust_gate(Some(&models_dir), &config, ctx.sink) {
                        ctx.sink
                            .push(UiEvent::AsrDevice(format!("{} [cpu]", current_display)));
                        tracing::warn!(
                            "引擎切换中止：{} 未通过完整性校验（坏文件已隔离）",
                            engine_model_key(&engine, &funasr_model, &whisper_model_size)
                        );
                        continue;
                    }
                    // D-83/DL-C：切换恒为显式重试（用户意图/修复后重载）
                    if let Err(e) = manager.ensure_started_explicit(&config) {
                        // D-83 §2.4：装载失败复验分流——文件被改过 → Hash
                        // 故障（可修复）；指纹一致仍失败 → Unloadable（重下无解）
                        report_load_failure(Some(&models_dir), &config, &e.to_string(), ctx.sink);
                        // 回滚后旧 worker 仍在工作：恢复旧标签而非
                        // 发 AsrUnavailable（避免状态与行为矛盾，P0-4）
                        ctx.sink
                            .push(UiEvent::AsrDevice(format!("{} [cpu]", current_display)));
                        tracing::error!("引擎切换失败（已回滚）: {e}");
                    } else {
                        *current_display = display.clone();
                        *asr_unavailable_notified = false;
                        ctx.sink
                            .push(UiEvent::AsrDevice(format!("{display} [cpu]")));
                        // AH-3：切换后增量会话状态复位——旧引擎的
                        // committed_tail/active 对新引擎输出无意义，且
                        // 切换后首个收尾段经 commit_interim_final 绕过
                        // 语言过滤，残留 active 会放行语言不符文本
                        interim_state.reset();
                        ctx.interim.last_interim_samples.store(0, Ordering::Relaxed);
                        ctx.interim.last_check_ms.store(0, Ordering::Relaxed);
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
                    ctx.sink
                        .push(UiEvent::AsrDevice(format!("{} [cpu]", current_display)));
                    let model_key = engine_model_key(&engine, &funasr_model, &whisper_model_size);
                    tracing::warn!(
                        "切换目标未缓存/未知，保持当前引擎: {engine}/{model_key}（去识别页下载）"
                    );
                }
            }
        }
    }
}

/// 主循环一轮的取段：**先排空翻译器命令，再取段**（D-85/F1 的位置不变量）。
///
/// 抽成函数让"切换在**下一段边界**生效"这一位置关系有唯一实现点与直接测试面：
/// 旧实现把消费点放在空闲分支，连续识别（队列一直非空）时切换要排到很久之后
/// （等 500ms 空窗）。`drain` 闭包收在这里执行——测试用 `next_segment`
/// 即可钉住"队列非空也先消费切换"这一语义本身。
fn next_segment(
    queue: &Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
    drain: impl FnOnce(),
) -> Option<(SegmentSource, Vec<f32>)> {
    drain();
    queue.pop_timeout(Duration::from_millis(500))
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
        learned,
        degraded_notified,
        session_stats,
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
        // 模型键 → 条目；非法/退役键回退 sensevoice-small（不阻断 UI）。
        // 诊断仅 funasr 引擎相关：whisper/qwen3 启动诊断走 build_worker_config 对应分支
        // （E2/D-79：判定走引擎值域透镜，零字面量比较）
        let (entry, fell_back) = match settings.engine_key() {
            EngineKey::FunAsr => resolve_funasr_entry(&settings.funasr_model),
            // WP-B：诊断用 qwen3 自身条目（否则未缓存日志打错模型名）
            EngineKey::Qwen3 => (registry::qwen3_entry(), false),
            EngineKey::Whisper => (registry::SENSEVOICE_SMALL.clone(), false),
        };
        if fell_back {
            tracing::warn!(
                "funasr 模型 {:?} 非法/退役（D-86），回退 sensevoice-small",
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
        // D-83 零信任闸门（启动路径）：清单齐全 ≠ 内容可信——逐文件复验
        // 注册表 sha256；不通过则坏文件已隔离、事件已发，回待命态等修复下载
        let gate_rejected = match &worker {
            Some((config, _)) => !trust_gate(Some(models_dir), config, &sink),
            None => false,
        };
        if gate_rejected {
            worker = None;
            sink.push(UiEvent::AsrUnavailable);
            tracing::warn!("ASR 模型未通过完整性校验（坏文件已隔离），进入待命态等待修复下载");
        } else if worker.is_none() {
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
            }) = route_translator_switch(
                sw,
                &mut tl,
                &bus,
                &sink,
                &sup,
                &transcript,
                &msg,
                &learned,
                &degraded_notified,
                &session_stats,
            )
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
                    // D-83 零信任闸门（待命唤醒路径）：坏文件已隔离 + 事件已发，
                    // 继续待命等下一轮修复下载后唤醒
                    if !trust_gate(Some(&models_dir), &config, &sink) {
                        sink.push(UiEvent::AsrUnavailable);
                        tracing::warn!(
                            "待命唤醒：{engine}/{model_key} 未通过完整性校验（已隔离），继续待命"
                        );
                        continue;
                    }
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
    // D-83/DL-C：显式重试语义——修复后重载同一模型不得被"重启配额已耗尽"拒绝
    if let Err(e) = manager.ensure_started_explicit(&config) {
        // D-83 §2.4：装载失败复验分流（文件被改过 → Hash 可修复；指纹一致
        // 仍失败 → Unloadable 重下无解）
        report_load_failure(models_dir.as_deref(), &config, &e.to_string(), &sink);
        asr_unavailable_notified = true;
        sink.push(UiEvent::AsrUnavailable);
        tracing::error!("ASR worker 启动失败: {e}");
    } else {
        sink.push(UiEvent::AsrDevice(format!("{display} [cpu]")));
    }

    while !stop.load(Ordering::Relaxed) {
        // D-85/F1：取段**之前**先收翻译器命令（`next_segment` 内建该顺序）——
        // 换模型在**下一段边界**即生效，不再依赖"空闲分支"（旧实现要等段队列
        // 连续 500ms 空窗；位置不变量与回归测试见 `next_segment`）
        let Some((source, audio)) = next_segment(&segment_queue, || {
            drain_tl_switch(
                &mut tl,
                &mut manager,
                &mut current_display,
                &mut asr_unavailable_notified,
                &mut interim_state,
                &SwitchDrain {
                    tl_switch: &tl_switch,
                    bus: &bus,
                    sink: &sink,
                    sup: &sup,
                    transcript: &transcript,
                    learned: &learned,
                    degraded_notified: &degraded_notified,
                    interim: &interim,
                    settings,
                    msg: &msg,
                    session_stats: &session_stats,
                },
            );
        }) else {
            // 空闲分支：RSS 回收（原版 _asr_loop queue.Empty）+ 翻译器切换命令
            // （AH-1：翻译器三臂经 route_translator_switch 与待命循环共享）
            manager.maybe_recycle_if_idle();
            drain_tl_switch(
                &mut tl,
                &mut manager,
                &mut current_display,
                &mut asr_unavailable_notified,
                &mut interim_state,
                &SwitchDrain {
                    tl_switch: &tl_switch,
                    bus: &bus,
                    sink: &sink,
                    sup: &sup,
                    transcript: &transcript,
                    learned: &learned,
                    degraded_notified: &degraded_notified,
                    interim: &interim,
                    settings,
                    msg: &msg,
                    session_stats: &session_stats,
                },
            );
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
                    &transcript,
                    &session_stats,
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
                                &transcript,
                                &session_stats,
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
                                &transcript,
                                &session_stats,
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
    transcript: &lt_audio::transcript::TranscriptWriter,
    stats: &TlStats,
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
            transcript,
            stats,
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
    transcript: &lt_audio::transcript::TranscriptWriter,
    stats: &TlStats,
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
        transcript,
        stats,
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
    transcript: &lt_audio::transcript::TranscriptWriter,
    stats: &TlStats,
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

    // ACR-4：计数与转录落盘先于翻译分流——与"翻译装置是否存在"无关（原版照记，
    // 只是译文栏失败）。write_original 会把 id 挂进转录 pending 表，故下面每条
    // 出口路径都必须成对收口（finalize/write_translation），否则该段原文在
    // 转录 all 文件里永久消失（修漏记变造悬挂）。
    stats.asr_count.fetch_add(1, Ordering::Relaxed);
    transcript.write_original(id, &timestamp, original_text);

    // ── 翻译分流（原版 _process_segment_text 尾部；字幕窗 extra_langs 随 M4 接入）──
    let Some(rig) = tl else {
        // 翻译装置未就绪（配置无效已被 TranslatorUnavailable 提醒）：立即给结论，
        // 不停留在永久的「翻译中...」（P0-2）。**以失败结论下发**——旧实现把它当
        // 正常译文（`UpdateTranslation` + 占位文案），字幕窗用译文样式显示，用户
        // 看不出这是错误（审计 R2：报错必须与译文明显不同）。
        let _ = msg; // 文案由 UI 按 FailureKind 本地化（编排域不依赖 lt-i18n）
        transcript.finalize_no_translation(id);
        sink.push(UiEvent::TranslationFailed {
            id,
            kind: FailureKind::NotReady,
            detail: "translator not ready (invalid model config or no active model)".into(),
            tl_ms: 0.0,
        });
        // ACR-4：计数已自增，统计快照随行（否则 UI 统计行与账本脱节）
        sink.push(stats.snapshot_event());
        return;
    };
    if lang == target_language {
        tracing::info!("Same language ({lang}), no translation");
        transcript.finalize_no_translation(id);
        // W2/INV-A：同语言走显式结论——不再用空串冒充（空串另有"模型没答"的含义）
        sink.push(UiEvent::TranslationSkipped {
            id,
            reason: SkipReason::SameLanguage,
        });
        sink.push(stats.snapshot_event());
    } else {
        rig.submit_translation(sink, id, original_text.to_string(), lang.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_artery::EventArtery;

    // ── D-83 零信任闸门（docs/archive/model-trust-repair.md）──

    /// 合成清单（真实注册表条目内容无法伪造——那正是零信任的意义，
    /// 故闸门核心用可注入清单测）
    fn synthetic_files(
        root: &std::path::Path,
        name: &'static str,
        content: &[u8],
        expected: String,
    ) -> TrustFiles {
        std::fs::write(root.join(name), content).unwrap();
        TrustFiles {
            display: "Synthetic Model".into(),
            root: root.to_path_buf(),
            files: vec![(name, expected)],
        }
    }

    /// 哈希一致 → 放行、不发事件、不动文件
    #[test]
    fn trust_gate_passes_matching_hash() {
        let dir = std::env::temp_dir().join(format!("lt_trust_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let content = b"good-model-bytes";
        let want = lt_download::hash_file_hex(
            &std::fs::write(dir.join("m.bin"), content)
                .map(|_| dir.join("m.bin"))
                .unwrap(),
        )
        .unwrap();
        let t = TrustFiles {
            display: "Synthetic Model".into(),
            root: dir.clone(),
            files: vec![("m.bin", want)],
        };
        let artery = EventArtery::new();
        assert!(verify_files(&t, &artery), "哈希一致应放行");
        assert!(dir.join("m.bin").exists(), "放行不得动文件");
        let mut batch = Vec::new();
        artery.drain_batch(&mut batch, std::time::Duration::from_millis(50));
        assert!(
            !batch
                .iter()
                .any(|e| matches!(e, UiEvent::ModelIntegrityFailed { .. })),
            "放行时不应发故障事件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 哈希不符 → 隔离坏文件（原路径消失 = 探测判缺 = 重下真正执行）+ 发
    /// Hash 故障事件（quarantined=true）
    #[test]
    fn trust_gate_quarantines_mismatch_and_reports() {
        let dir = std::env::temp_dir().join(format!("lt_trust_bad_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.clone();
        let t = synthetic_files(&root, "m.bin", b"corrupted-bytes", "0".repeat(64));
        let artery = EventArtery::new();
        assert!(!verify_files(&t, &artery), "哈希不符应拦截");
        assert!(!root.join("m.bin").exists(), "坏文件原路径必须消失");
        assert!(root.join("m.bin.corrupt").exists(), "隔离产物保留内容");
        let mut batch = Vec::new();
        artery.drain_batch(&mut batch, std::time::Duration::from_millis(50));
        let ev = batch
            .iter()
            .find_map(|e| match e {
                UiEvent::ModelIntegrityFailed { model, fault } => Some((model, fault)),
                _ => None,
            })
            .expect("应发 ModelIntegrityFailed");
        assert_eq!(ev.0, "Synthetic Model");
        match ev.1 {
            ModelFault::Hash {
                file,
                expected,
                actual,
                quarantined,
            } => {
                assert_eq!(file, "m.bin");
                assert_eq!(expected.len(), 64);
                assert!(*quarantined, "隔离应成功");
                // 实测哈希即被隔离内容的哈希（现场可复核）
                let quarantined_hash =
                    lt_download::hash_file_hex(&root.join("m.bin.corrupt")).unwrap();
                assert_eq!(actual, &quarantined_hash);
            }
            other => panic!("应 Hash 故障，实际 {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 读不了（文件缺失）→ 拦截但不隔离（内容未知，隔离等于丢证据）
    #[test]
    fn trust_gate_unreadable_file_blocks_without_quarantine() {
        let dir = std::env::temp_dir().join(format!("lt_trust_missing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let t = TrustFiles {
            display: "Synthetic Model".into(),
            root: dir.clone(),
            files: vec![("gone.bin", "a".repeat(64))],
        };
        let artery = EventArtery::new();
        assert!(!verify_files(&t, &artery));
        assert!(!dir.join("gone.bin.corrupt").exists(), "不该凭空造隔离文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 真缓存零信任探针（ignored）：对本机真实缓存逐引擎跑闸门——校验实现
    /// 与注册表指纹的实机互证（未缓存引擎自动跳过）。
    /// **耗时为 debug 构建**；release 构建 sha2 走硬件加速约快 30 倍
    /// （实测 1.5GB/s：tiny 0.02s / SenseVoice 0.15s / 1GB 级 0.7s）。
    /// 运行：
    /// `cargo test -p lt-orchestrator probe_trust_gate_real_cache -- --ignored --nocapture`
    #[test]
    #[ignore = "真实缓存探针：需本机已缓存模型（LIVETRANSLATE_CONFIG_DIR 可重定向）"]
    fn probe_trust_gate_real_cache() {
        let dir = lt_models::paths::models_dir(None).expect("models_dir 解析失败");
        println!("models_dir = {}", dir.display());
        let artery = EventArtery::new();
        let cases: &[(&str, &str, &str)] = &[
            ("funasr", "sensevoice-small", ""),
            ("funasr", "funasr-nano-2512", ""),
            ("qwen3", "", ""),
            ("whisper", "", "tiny"),
            ("whisper", "", "base"),
        ];
        let mut checked = 0;
        for (engine, funasr_model, whisper_size) in cases {
            let Some((config, display)) =
                build_worker_config(&dir, engine, funasr_model, 0.5, "auto", whisper_size, 0.5)
            else {
                println!("跳过 {engine}/{funasr_model}{whisper_size}（未缓存）");
                continue;
            };
            let t0 = std::time::Instant::now();
            let ok = trust_gate(Some(&dir), &config, &artery);
            println!(
                "{} {display}：校验 {}（{:?}）",
                if ok { "✓" } else { "✗" },
                if ok { "通过" } else { "失败" },
                t0.elapsed()
            );
            assert!(ok, "{display} 真缓存应通过零信任校验");
            checked += 1;
        }
        // 本机至少应有一个缓存模型（否则探针跑了个寂寞）
        assert!(checked > 0, "本机无已缓存模型，探针无意义");
        let mut batch = Vec::new();
        artery.drain_batch(&mut batch, std::time::Duration::from_millis(50));
        assert!(
            batch.is_empty(),
            "真实缓存不应产生完整性故障事件: {batch:?}"
        );
        println!("共校验 {checked} 个模型，全部通过且零故障事件");
    }

    /// 清单解析：whisper 托管快照下的 .bin 命中注册表条目（root=快照目录）；
    /// 本地自定义路径（不在托管 models_dir 下）→ None（无指纹，闸门放行）
    #[test]
    fn trust_files_resolution_scopes_to_managed_whisper_cache() {
        let dir = std::env::temp_dir().join(format!("lt_trust_wh_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // 托管快照：models/huggingface/hub/models--ggerganov--whisper.cpp/snapshots/main
        let snap = lt_models::cache::hf_style_snapshot(
            &dir,
            lt_download::Hub::Hf,
            "ggerganov/whisper.cpp",
            "main",
        );
        std::fs::create_dir_all(&snap).unwrap();
        let bin = snap.join("ggml-tiny-q5_1.bin");
        std::fs::write(&bin, b"x").unwrap();
        let cfg = WorkerConfig {
            engine: "whisper".into(),
            language: "auto".into(),
            pad_seconds: Some(0.5),
            options: lt_asr::WorkerOptions::ModelPath(bin.clone()),
        };
        let t = trust_files_for_config(Some(&dir), &cfg).expect("托管路径应解析出清单");
        assert_eq!(t.root, snap);
        assert_eq!(t.files.len(), 1);
        assert_eq!(t.files[0].0, "ggml-tiny-q5_1.bin");
        assert_eq!(t.files[0].1.len(), 64, "须带注册表指纹");

        // 本地自定义路径（不在托管目录下）→ 不校验
        let custom = std::env::temp_dir().join("lt_custom_ggml_tiny.bin");
        std::fs::write(&custom, b"my own file").unwrap();
        let cfg2 = WorkerConfig {
            options: lt_asr::WorkerOptions::ModelPath(custom.clone()),
            ..cfg
        };
        assert!(
            trust_files_for_config(Some(&dir), &cfg2).is_none(),
            "本地自选模型无指纹语义，闸门不得介入（否则会误隔离用户文件）"
        );
        let _ = std::fs::remove_file(&custom);
        let _ = std::fs::remove_dir_all(&dir);
    }

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

        // D-86：退役键 mlt 与非法键同路 → 回退 sensevoice-small 条目装配（旧档兼容锁）
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
    /// W3：空的会话内学习记忆（测试不依赖记忆）
    fn test_degraded_notified() -> Arc<Mutex<std::collections::HashSet<(String, String)>>> {
        Arc::new(Mutex::new(std::collections::HashSet::new()))
    }

    fn test_learned() -> Learned {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn test_transcript() -> Arc<lt_audio::transcript::TranscriptWriter> {
        Arc::new(lt_audio::transcript::TranscriptWriter::new(tmp_models_dir(
            "transcript",
        )))
    }

    /// 测试设置总线（W4：from_settings 改读总线；发布即版本 1）
    fn test_bus(settings: lt_proto::Settings) -> Arc<SettingsBus> {
        Arc::new(SettingsBus::new(settings))
    }

    // ── 回退阶梯（第二轮评审 ③/④/⑤；方案 §4.4 表） ──

    /// 造一个不发请求的装置（client 构建不触网）
    fn test_translator(disable_thinking: bool) -> Translator {
        Translator::new(lt_translate::TranslatorParams {
            api_base: "http://127.0.0.1:1/v1".into(),
            disable_thinking,
            ..Default::default()
        })
        .expect("测试客户端构建必成功")
    }

    fn test_translator_explicit(style: &str) -> Translator {
        Translator::new(lt_translate::TranslatorParams {
            api_base: "http://127.0.0.1:1/v1".into(),
            disable_thinking: true,
            thinking_style: Some(style.into()),
            ..Default::default()
        })
        .expect("测试客户端构建必成功")
    }

    fn attempt(verdict: Option<lt_translate::ResponseVerdict>, text: Option<&str>) -> Attempt {
        Attempt {
            text: text.map(str::to_string),
            error: None,
            verdict,
            usage: (10, 20),
            usage_known: true,
        }
    }

    fn param_rejected() -> Attempt {
        Attempt {
            text: None,
            error: Some(lt_translate::TranslateError::Status {
                code: 400,
                message: "unknown field: reasoning_effort".into(),
            }),
            verdict: None,
            usage: (0, 0),
            usage_known: false,
        }
    }

    /// 体检显示"预算被思考吃光" → 允许时退一级
    #[test]
    fn ladder_advances_on_reasoning_budget() {
        let base = test_translator(true); // 127.0.0.1 + auto → ReasoningEffortNone
        assert_eq!(
            base.thinking_plan(),
            lt_translate::ThinkingPlan::ReasoningEffortNone
        );
        let step = lt_translate::first_step(base.thinking_plan());
        let verdict = attempt(
            Some(lt_translate::ResponseVerdict::EmptyReasoningBudget),
            None,
        );
        assert!(should_advance(step, &verdict, true));
        assert_eq!(
            lt_translate::next_step(step),
            Some(lt_translate::RequestStep::Plan(
                lt_translate::ThinkingPlan::EnableThinkingFalse
            ))
        );
    }

    /// 用户取消勾选（disable_thinking=false）→ 起点即链尾，**没有任何可退的级**
    /// （绝不擅自注入关闭参数——方案 §2.3 规则 1；旧测试曾把相反行为钉死）
    #[test]
    fn ladder_never_injects_when_switch_off() {
        let base = test_translator(false);
        assert_eq!(base.thinking_plan(), lt_translate::ThinkingPlan::None);
        let step = lt_translate::first_step(base.thinking_plan());
        let verdict = attempt(
            Some(lt_translate::ResponseVerdict::EmptyReasoningBudget),
            None,
        );
        // 起点就是"不发送"形态；下一级只有最小请求，且不涉及任何关闭参数
        assert_eq!(
            lt_translate::next_step(step),
            Some(lt_translate::RequestStep::Minimal)
        );
        let minimal = translator_for_step(&base, lt_translate::RequestStep::Minimal);
        let body = minimal.build_request_body("s", "t", true, false, 0);
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("thinking").is_none());
        // Minimal 起步且体检说"还在推理"时也不再退（已到端点）
        assert!(!should_advance(
            lt_translate::RequestStep::Minimal,
            &verdict,
            true
        ));
    }

    /// 用户**显式选定**关闭方式 → 体检驱动的降级被禁（尊重用户选择）；
    /// 但服务端 400 拒绝参数时仍退级（否则该模型整条不可用）
    #[test]
    fn ladder_respects_explicit_style_but_still_survives_400() {
        let base = test_translator_explicit("qwen");
        assert_eq!(
            base.thinking_plan(),
            lt_translate::ThinkingPlan::EnableThinkingFalse
        );
        let step = lt_translate::first_step(base.thinking_plan());
        let verdict = attempt(
            Some(lt_translate::ResponseVerdict::EmptyReasoningBudget),
            None,
        );
        assert!(
            !should_advance(step, &verdict, false),
            "显式方式不受体检驱动"
        );
        assert!(
            should_advance(step, &param_rejected(), false),
            "400 仍须降级"
        );
    }

    /// 非参数类错误不降级（网络/鉴权/404 与请求内容无关，退级只会白试）
    #[test]
    fn ladder_does_not_advance_on_unrelated_errors() {
        let step = lt_translate::RequestStep::Plan(lt_translate::ThinkingPlan::NestedDisabled);
        for err in [
            lt_translate::TranslateError::Timeout("t".into()),
            lt_translate::TranslateError::Auth {
                code: 401,
                message: "bad key".into(),
            },
            lt_translate::TranslateError::Status {
                code: 404,
                message: "no model".into(),
            },
            lt_translate::TranslateError::Connection("refused".into()),
        ] {
            let mut a = attempt(None, None);
            a.error = Some(err);
            assert!(!should_advance(step, &a, true));
        }
    }

    /// EmptyTruncated 走"同一台阶补发上限"，不换台阶
    #[test]
    fn ladder_raises_budget_on_empty_truncation() {
        let base = test_translator(true);
        let with_budget =
            translator_for_step(&base, lt_translate::RequestStep::Plan(base.thinking_plan()))
                .with_max_tokens(4096);
        let body = with_budget.build_request_body("s", "t", true, false, 0);
        assert_eq!(body["max_tokens"], 4096);
    }

    /// 台阶 → 装置：最小请求清空全部可选参数，普通台阶保留
    #[test]
    fn ladder_step_devices_differ_only_where_intended() {
        let base = Translator::new(lt_translate::TranslatorParams {
            api_base: "http://127.0.0.1:1234/v1".into(),
            model: "m".into(),
            temperature: Some(0.7),
            ..Default::default()
        })
        .unwrap();
        let normal =
            translator_for_step(&base, lt_translate::RequestStep::Plan(base.thinking_plan()));
        assert_eq!(
            normal.build_request_body("s", "t", true, false, 0)["temperature"],
            0.7
        );
        let minimal = translator_for_step(&base, lt_translate::RequestStep::Minimal);
        let body = minimal.build_request_body("s", "t", true, false, 0);
        assert!(body.get("temperature").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    /// 会话记忆与配置指纹绑定：改了请求形态相关的任何一项，记忆即失效
    /// （否则"取消了勾选、记忆却仍注入关闭参数"= 界面与实际不一致）
    #[test]
    fn config_fingerprint_tracks_request_shape() {
        let base = lt_proto::ModelConfig::default();
        assert_eq!(config_fingerprint(&base), config_fingerprint(&base.clone()));
        for changed in [
            lt_proto::ModelConfig {
                disable_thinking: !base.disable_thinking,
                ..base.clone()
            },
            lt_proto::ModelConfig {
                thinking_style: Some("qwen".into()),
                ..base.clone()
            },
            lt_proto::ModelConfig {
                temperature: Some(0.7),
                ..base.clone()
            },
            lt_proto::ModelConfig {
                api_key: "other-key".into(),
                ..base.clone()
            },
            lt_proto::ModelConfig {
                model: "other-model".into(),
                ..base.clone()
            },
            lt_proto::ModelConfig {
                extra_body: Some(serde_json::json!({"a": 1})),
                ..base.clone()
            },
        ] {
            assert_ne!(
                config_fingerprint(&base),
                config_fingerprint(&changed),
                "配置变化必须换指纹: {changed:?}"
            );
        }
    }

    /// item 5 回执判据：只有"用户要求过关闭"才报"关不掉"——主动取消勾选不算
    #[test]
    fn cannot_disable_is_reported_only_when_user_asked() {
        use lt_translate::{RequestStep, ThinkingPlan};
        let gave_up = RequestStep::Plan(ThinkingPlan::None);
        let minimal = RequestStep::Minimal;
        let still_injecting = RequestStep::Plan(ThinkingPlan::NestedDisabled);
        // 用户要求过 + 确实试过注入 + 退到不发送 → 报
        assert!(should_report_cannot_disable(true, true, false, gave_up));
        assert!(should_report_cannot_disable(true, true, false, minimal));
        // 用户主动取消勾选（没要求过）→ 不报（否则给模型打上错误标记）
        assert!(!should_report_cannot_disable(false, true, false, gave_up));
        // **没试过注入**（官方保守端点起点即 Plan(None)）→ 不报，
        // 否则每翻译一段就被自动取消勾选一次（对抗审计实证的自证预言）
        assert!(!should_report_cannot_disable(true, false, false, gave_up));
        // 已经标记过 → 不重复报
        assert!(!should_report_cannot_disable(true, true, true, gave_up));
        // 还在注入关闭参数 → 还没到"关不掉"的结论
        assert!(!should_report_cannot_disable(
            true,
            true,
            false,
            still_injecting
        ));
    }

    /// 阶梯端到端（死端口）：连接类错误**不得**白走阶梯——只在起点试一次，
    /// 立即返回失败，且失败台阶即起点（供"下一段不再重走"的记忆使用）
    #[test]
    fn ladder_stops_on_connection_error_without_walking() {
        let base = test_translator(true);
        let start = lt_translate::first_step(base.thinking_plan());
        let sink = EventArtery::new();
        let t0 = Instant::now();
        let outcome = run_ladder(
            &base,
            start,
            true,
            "hello",
            "en",
            "zh",
            1,
            &RunCtl::none(),
            &sink,
            42,
            1,
            false,
        );
        assert!(!outcome.attempt.succeeded());
        assert_eq!(outcome.step, start, "连接错误不得推进台阶");
        assert_eq!(outcome.usage, (0, 0), "无用量");
        assert_eq!(outcome.halted, None, "生产路径不提前中止");
        assert_eq!(outcome.attempted, 1, "连接类错误只试一次");
        assert!(
            matches!(
                outcome.attempt.error,
                Some(lt_translate::TranslateError::Connection(_))
                    | Some(lt_translate::TranslateError::Timeout(_))
            ),
            "实际: {:?}",
            outcome.attempt.error
        );
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "不得逐级重试耗尽时间"
        );
    }

    /// 台阶名（"当前实际在用"回执用）不得出现厂商词（INV-E）
    #[test]
    fn ladder_step_names_are_vendor_neutral_in_copy() {
        // 实际值本身是内部串（openai/qwen 等形状名），只在日志与内部回执使用；
        // 用户可见文案由 UI 按 i18n 组装——此处只钉住值域稳定
        for step in [
            lt_translate::RequestStep::Plan(lt_translate::ThinkingPlan::None),
            lt_translate::RequestStep::Minimal,
        ] {
            assert!(lt_translate::gives_up_disabling(step));
        }
    }

    #[test]
    fn attempt_success_uses_verdict_first() {
        // 有 url 文本但体检判空（理论矛盾情形）→ 以体检为准
        assert!(!attempt(
            Some(lt_translate::ResponseVerdict::EmptyNoOutput),
            Some("x")
        )
        .succeeded());
        assert!(attempt(Some(lt_translate::ResponseVerdict::Ok), Some("x")).succeeded());
        // 无体检结论时退回"有文本即成功"
        assert!(attempt(None, Some("x")).succeeded());
        assert!(!attempt(None, None).succeeded());
        assert!(!attempt(None, Some("   ")).succeeded());
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
        let rig = TlRig::from_settings(
            &bus,
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh",
        )
        .expect("默认设置不应报配置错误")
        .expect("默认 settings 带一个默认模型，应能构建");
        // W4：目标语言不再存实例态（逐调用经总线 tl 视图传入）——验证总线视图
        assert_eq!(bus.load().tl.target_language, "zh");
        // 收尾：先停池再 join——worker 以 stopped 为退出条件，不先置标志
        // join_all 将无限等待（pop_timeout 永不返回）
        rig.pool.shutdown();
        sup.join_all();
    }

    /// D-85：测试用会话统计（每个用例独立一份——真实实现由 Pipeline 建一次）
    fn test_session_stats() -> Arc<TlStats> {
        Arc::new(TlStats::new())
    }

    /// D-85：探测与生产**同源**——装置参数只有一处构造点
    /// （[`translator_params`]），装置自存一份供逐字段断言。两处各列一遍字段
    /// 的写法迟早漂移，本测试把"漂移"钉成红灯。
    #[test]
    fn translator_params_single_source() {
        let settings = lt_proto::Settings::default();
        let sup = test_sup();
        let bus = test_bus(settings);
        let eff = bus.load();
        let mc = eff.raw.models[eff.raw.active_model].clone();
        let rig = TlRig::from_settings(
            &bus,
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh",
        )
        .expect("默认设置不应报配置错误")
        .expect("默认 settings 带一个默认模型，应能构建");
        assert_eq!(
            rig.params_for_test(),
            &translator_params(&mc, &eff),
            "装置参数必须来自唯一构造点（探测与生产同源）"
        );
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
        assert!(TlRig::from_settings(
            &test_bus(settings),
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh"
        )
        .unwrap()
        .is_none());
        sup.join_all();
    }

    #[test]
    fn tl_rig_none_when_models_empty() {
        let mut settings = lt_proto::Settings::default();
        settings.models.clear();
        let sup = test_sup();
        assert!(TlRig::from_settings(
            &test_bus(settings),
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh"
        )
        .unwrap()
        .is_none());
        sup.join_all();
    }

    #[test]
    fn job_pool_drops_jobs_after_shutdown() {
        use std::sync::atomic::AtomicU64;
        let sup = test_sup();
        let pool = JobPool::new(2, &sup, EventArtery::new(), test_transcript());
        pool.shutdown();
        sup.join_all();
        // shutdown 之后的提交不执行（submit 侧短路 + worker 侧双重检查）
        let ran = Arc::new(AtomicU64::new(0));
        let r = ran.clone();
        pool.submit(7, move || {
            r.fetch_add(1, Ordering::Relaxed);
        });
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(ran.load(Ordering::Relaxed), 0);
    }

    /// 装置被替换（换模型）时要 retire 在队任务：补"已放弃"回执而非静默丢弃
    /// （审计：只覆盖了队列溢出路径，替换路径会让字幕永远停在「翻译中…」）
    #[test]
    fn retired_pool_emits_receipts_for_queued_jobs() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let pool = JobPool::new(0, &sup, sink.clone(), test_transcript());
        for id in 0..3u64 {
            pool.submit(id, || {});
        }
        pool.retire();
        let mut batch = Vec::new();
        let mut dropped_ids = Vec::new();
        loop {
            if !sink.drain_batch(&mut batch, Duration::from_millis(20)) {
                break;
            }
            for ev in batch.iter() {
                if let UiEvent::TranslationFailed { id, kind, .. } = ev {
                    // D-85/F3：retire = 换模型让位（与"队列积压"区分开）
                    assert_eq!(*kind, FailureKind::Superseded);
                    dropped_ids.push(*id);
                }
            }
        }
        assert_eq!(dropped_ids, vec![0, 1, 2], "替换时在队任务必须逐条回执");
        // 停机路径（不 retire）保持静默：事件无处可去，也不该刷失败
        let pool2 = JobPool::new(0, &sup, sink.clone(), test_transcript());
        for id in 10..12u64 {
            pool2.submit(id, || {});
        }
        pool2.shutdown();
        let mut batch2 = Vec::new();
        while sink.drain_batch(&mut batch2, Duration::from_millis(20)) {
            for ev in batch2.iter() {
                if let UiEvent::TranslationFailed { id, .. } = ev {
                    assert!(*id < 10, "停机路径不应补回执");
                }
            }
        }
        sup.join_all();
    }

    /// 第二轮评审 ⑬d：队列满丢最旧时，被丢任务必须补一条"已放弃"回执
    /// （旧实现无声无息，字幕永远停在"翻译中"）
    #[test]
    fn dropped_jobs_emit_receipt() {
        use std::sync::atomic::AtomicU64;
        let sup = test_sup();
        let sink = EventArtery::new();
        // worker 数为 0：任务只进队、不消费，灌满即丢最旧
        let pool = JobPool::new(0, &sup, sink.clone(), test_transcript());
        let ran = Arc::new(AtomicU64::new(0));
        for id in 0..(TL_QUEUE_CAP as u64 + 3) {
            let r = ran.clone();
            pool.submit(id, move || {
                r.fetch_add(1, Ordering::Relaxed);
            });
        }
        // 被丢的 3 条应在动脉里留下 3 条 Dropped 回执
        let mut batch = Vec::new();
        let mut dropped_ids = Vec::new();
        loop {
            if !sink.drain_batch(&mut batch, Duration::from_millis(20)) {
                break;
            }
            for ev in batch.iter() {
                if let UiEvent::TranslationFailed { id, kind, .. } = ev {
                    assert_eq!(*kind, FailureKind::Dropped);
                    dropped_ids.push(*id);
                }
            }
        }
        assert_eq!(dropped_ids, vec![0, 1, 2], "最旧的三条被丢弃并回了执");
        sup.join_all();
    }

    /// ACR-5：翻译任务执行期 panic 视同任务丢失——补 `Dropped` 回执 + 转录收口；
    /// worker 不因该 panic 死亡，后续任务由**同一 worker** 继续服务（不依赖重生）。
    #[test]
    fn panicking_job_emits_receipt_and_keeps_worker_serving() {
        use std::sync::atomic::AtomicU64;
        let sup = test_sup();
        let sink = EventArtery::new();
        let dir = tmp_models_dir("acr5-panic");
        let transcript = Arc::new(lt_audio::transcript::TranscriptWriter::new(&dir));
        let pool = JobPool::new(1, &sup, sink.clone(), transcript.clone());

        // 病灶：闭包 panic——旧实现 `run.take()` 已使 Drop 守卫不成立：
        // 该消息永久"翻译中"、转录 pending 悬挂（all 文件整段消失）
        transcript.write_original(7, "00:00:07", "会崩的段");
        pool.submit(7, || panic!("注入 panic（ACR-5 测试）"));

        let mut events = Vec::new();
        let mut batch = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            while sink.drain_batch(&mut batch, Duration::from_millis(20)) {
                events.append(&mut batch);
            }
            if events
                .iter()
                .any(|e| matches!(e, UiEvent::TranslationFailed { id: 7, .. }))
            {
                break;
            }
        }
        let (kind, detail) = events
            .iter()
            .find_map(|e| match e {
                UiEvent::TranslationFailed {
                    id: 7,
                    kind,
                    detail,
                    ..
                } => Some((*kind, detail.clone())),
                _ => None,
            })
            .expect("panic 任务必须补回执（ACR-5）");
        assert_eq!(kind, FailureKind::Dropped);
        assert!(
            detail.contains("注入 panic"),
            "回执 detail 应含 panic 信息，实际：{detail}"
        );

        // 同一 worker 继续服务（catch_unwind 就地兜住，不依赖监督器重生）
        let ran = Arc::new(AtomicU64::new(0));
        let r = ran.clone();
        pool.submit(8, move || {
            r.fetch_add(1, Ordering::Relaxed);
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while ran.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            ran.load(Ordering::Relaxed),
            1,
            "panic 后 worker 仍能执行后续任务"
        );

        // 转录面：panic 段成对收口（all 出现原文块），无 pending 悬挂
        transcript.close();
        let all = std::fs::read_to_string(transcript.session_paths().get("all").unwrap()).unwrap();
        assert!(all.contains("会崩的段"), "panic 段必须收口：{all}");

        pool.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
        sup.join_all();
    }

    /// W1 泄漏回归（ReplaceRig 路径）：旧 rig 被替换 Drop 后，旧池 8 个 worker
    /// 必须在 ≤500ms 节拍内全部退出——否则它们永驻 Supervisor entries，
    /// Pipeline::stop 的 join_all 永久挂起（应用退出即僵尸进程）。
    #[test]
    fn replaced_rig_workers_shutdown_on_drop() {
        let sup = test_sup();
        let bus = test_bus(lt_proto::Settings::default());
        let rig = TlRig::from_settings(
            &bus,
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh",
        )
        .unwrap()
        .unwrap();
        let old_alive = rig.pool.alive_workers.clone();
        wait_for(|| old_alive.load(Ordering::Relaxed) == TL_POOL_WORKERS);
        // 模拟 ReplaceRig 的替换语义（route_translator_switch：
        // `*tl = Some(Arc::new(rig))`——旧 rig 被 Drop，无人显式关机）
        let replacement = TlRig::from_settings(
            &bus,
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh",
        )
        .unwrap()
        .unwrap();
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
        let rig = TlRig::from_settings(
            &bus,
            &sup,
            EventArtery::new(),
            test_transcript(),
            test_learned(),
            test_degraded_notified(),
            &test_session_stats(),
            "zh",
        )
        .unwrap()
        .unwrap();
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

    // ── resolve_funasr_entry：非法/退役键回退（D-86） ──

    #[test]
    fn retired_mlt_falls_back_to_sensevoice() {
        let (entry, fell_back) = resolve_funasr_entry("funasr-mlt-nano-2512");
        assert!(fell_back);
        // D-86：退役键与 bogus 同路回退，绝不冒充 nano
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
        // 同名目录曾被 17 个测试并发共享（test_transcript 等）：remove+create 非原子，
        // Windows 下 create_dir_all 的 AlreadyExists+is_dir 复查窗口会被并发 remove 打穿
        // （实跑偶发 code 183 panic）——每次调用取唯一序号根治；各测试自带收尾清理
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base =
            std::env::temp_dir().join(format!("lt_bwc_test_{name}_{}_{}", std::process::id(), n));
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

    /// D-85/F3 回归：**队列溢出**路径仍是 `Dropped`（真丢件），
    /// 不得被"换模型让位"的改动串味
    #[test]
    fn queue_overflow_still_reports_dropped() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let pool = JobPool::new(0, &sup, sink.clone(), test_transcript());
        // 队列容量 TL_QUEUE_CAP：塞满再多推一条 → 丢最旧（Drop 补 Dropped 回执）
        for id in 0..(TL_QUEUE_CAP as u64 + 1) {
            pool.submit(id, || {});
        }
        let mut batch = Vec::new();
        let mut kinds = Vec::new();
        loop {
            if !sink.drain_batch(&mut batch, Duration::from_millis(20)) {
                break;
            }
            for ev in batch.iter() {
                if let UiEvent::TranslationFailed { kind, .. } = ev {
                    kinds.push(*kind);
                }
            }
        }
        assert!(
            kinds.iter().all(|k| *k == FailureKind::Dropped),
            "溢出丢弃必须仍是 Dropped，实际 {kinds:?}"
        );
        assert!(!kinds.is_empty(), "至少应有一条被丢弃的回执");
        pool.shutdown();
        sup.join_all();
    }

    /// D-85/F3：装置被替换时的在队任务按"换模型让位"回执——不能报"队列积压"
    ///（后者是假原因：用户只是换了个模型）
    #[test]
    fn retired_pool_marks_superseded_not_dropped() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let pool = JobPool::new(0, &sup, sink.clone(), test_transcript());
        for id in 0..2u64 {
            pool.submit(id, || {});
        }
        pool.retire();
        let mut batch = Vec::new();
        let mut kinds = Vec::new();
        loop {
            if !sink.drain_batch(&mut batch, Duration::from_millis(20)) {
                break;
            }
            for ev in batch.iter() {
                if let UiEvent::TranslationFailed { kind, detail, .. } = ev {
                    kinds.push((*kind, detail.clone()));
                }
            }
        }
        assert_eq!(kinds.len(), 2, "两条在队任务都应回执");
        for (kind, detail) in kinds {
            assert_eq!(kind, FailureKind::Superseded, "换模型让位不是'积压丢弃'");
            assert!(detail.contains("切换模型"), "文案应说明真因：{detail}");
        }
        sup.join_all();
    }

    /// D-85/F2：切换成功后回执 TranslatorSwitched（面板状态行据此确认"已生效"）
    #[test]
    fn replace_rig_pushes_translator_switched() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let bus = test_bus(lt_proto::Settings::default());
        let mut tl = None;
        let sw = TlSwitch::ReplaceRig {
            config: Box::new(lt_proto::Settings::default().models[0].clone()),
        };
        let _ = route_translator_switch(
            sw,
            &mut tl,
            &bus,
            &sink,
            &sup,
            &test_transcript(),
            &Msg::new(|k| k.to_string(), || "zh".into()),
            &test_learned(),
            &test_degraded_notified(),
            &test_session_stats(),
        );
        assert!(tl.is_some(), "替换后应装上新装置");
        let mut batch = Vec::new();
        let mut seen = false;
        while sink.drain_batch(&mut batch, Duration::from_millis(20)) {
            for ev in batch.iter() {
                if let UiEvent::TranslatorSwitched { name, .. } = ev {
                    assert!(!name.is_empty());
                    seen = true;
                }
            }
        }
        assert!(seen, "切换成功必须回执 TranslatorSwitched");
        if let Some(rig) = tl.take() {
            rig.pool.shutdown();
        }
        sup.join_all();
    }

    /// D-85/F1：`drain_tl_switch` 不依赖"空闲分支状态"——提取后可在主循环
    /// 每轮开头直接调用（旧实现只在段队列 500ms 空窗时才消费切换命令）
    #[test]
    fn drain_tl_switch_is_callable_from_anywhere() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let bus = test_bus(lt_proto::Settings::default());
        let (tx, rx) = crossbeam_channel::unbounded::<TlSwitch>();
        let settings = lt_proto::Settings::default();
        let mut tl = None;
        let mut manager = AsrManager::new();
        let mut display = String::from("old");
        let mut notified = false;
        let mut interim_state = InterimState::default();
        let interim = Arc::new(InterimControl::default());
        let learned = test_learned();
        let degraded = test_degraded_notified();
        let transcript = test_transcript();

        // 队列里有 ReplaceRig → 一次调用即装上新装置 + 回执 TranslatorSwitched
        tx.send(TlSwitch::ReplaceRig {
            config: Box::new(settings.models[0].clone()),
        })
        .unwrap();
        drain_tl_switch(
            &mut tl,
            &mut manager,
            &mut display,
            &mut notified,
            &mut interim_state,
            &SwitchDrain {
                tl_switch: &rx,
                bus: &bus,
                sink: &sink,
                sup: &sup,
                transcript: &transcript,
                learned: &learned,
                degraded_notified: &degraded,
                interim: &interim,
                settings: &settings,
                msg: &Msg::new(|k| k.to_string(), || "zh".into()),
                session_stats: &test_session_stats(),
            },
        );
        assert!(tl.is_some(), "主循环开头的一次调用就该完成切换");
        let mut batch = Vec::new();
        let mut switched = false;
        while sink.drain_batch(&mut batch, Duration::from_millis(20)) {
            for ev in batch.iter() {
                if matches!(ev, UiEvent::TranslatorSwitched { .. }) {
                    switched = true;
                }
            }
        }
        assert!(switched, "切换生效回执必须随切换一并到达");
        // 空队列再调一次：无副作用（幂等）
        drain_tl_switch(
            &mut tl,
            &mut manager,
            &mut display,
            &mut notified,
            &mut interim_state,
            &SwitchDrain {
                tl_switch: &rx,
                bus: &bus,
                sink: &sink,
                sup: &sup,
                transcript: &transcript,
                learned: &learned,
                degraded_notified: &degraded,
                interim: &interim,
                settings: &settings,
                msg: &Msg::new(|k| k.to_string(), || "zh".into()),
                session_stats: &test_session_stats(),
            },
        );
        if let Some(rig) = tl.take() {
            rig.pool.shutdown();
        }
        sup.join_all();
    }

    /// D-85/F1 位置不变量（2026-09-11 评审修复）：取段**之前**先排空切换命令——
    /// **队列非空也先消费**。旧实现只在空闲分支消费，连续识别时切换要排到很久
    /// 之后（等 500ms 空窗）；`next_segment` 是生产主循环该环节的唯一实现点
    #[test]
    fn next_segment_drains_switch_before_pop() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let bus = test_bus(lt_proto::Settings::default());
        let settings = lt_proto::Settings::default();
        let (tx, rx) = crossbeam_channel::unbounded::<TlSwitch>();
        let mut tl = None;
        let mut manager = AsrManager::new();
        let mut display = String::from("old");
        let mut notified = false;
        let mut interim_state = InterimState::default();
        let interim = Arc::new(InterimControl::default());
        let learned = test_learned();
        let degraded = test_degraded_notified();
        let transcript = test_transcript();
        // 段队列**非空**（连续识别场景）：切换命令必须仍在本轮被消费
        let queue = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(
            8, "seg-test",
        ));
        queue.push((SegmentSource::VadFlush, vec![0.0f32; 16]));
        tx.send(TlSwitch::ReplaceRig {
            config: Box::new(settings.models[0].clone()),
        })
        .unwrap();

        let seg = next_segment(&queue, || {
            drain_tl_switch(
                &mut tl,
                &mut manager,
                &mut display,
                &mut notified,
                &mut interim_state,
                &SwitchDrain {
                    tl_switch: &rx,
                    bus: &bus,
                    sink: &sink,
                    sup: &sup,
                    transcript: &transcript,
                    learned: &learned,
                    degraded_notified: &degraded,
                    interim: &interim,
                    settings: &settings,
                    msg: &Msg::new(|k| k.to_string(), || "zh".into()),
                    session_stats: &test_session_stats(),
                },
            );
        });
        assert!(seg.is_some(), "队列里的段应被取出");
        assert!(tl.is_some(), "取段之前必须先完成切换（队列非空也不例外）");
        if let Some(rig) = tl.take() {
            rig.pool.shutdown();
        }
        sup.join_all();
    }

    /// 2026-09-11 评审修复：任务从未执行就被丢弃时，转录 all 文件必须补上
    /// "无译文"块——`write_original` 已登记 pending，若无人 finalize，该段原文
    /// 从 all 文件永久消失（F3 让位路径把该缺口从"溢出罕见"变成"换模型常规"）
    #[test]
    fn retired_jobs_finalize_transcript_original() {
        let sup = test_sup();
        let sink = EventArtery::new();
        let dir = tmp_models_dir("transcript-drop");
        let transcript = Arc::new(lt_audio::transcript::TranscriptWriter::new(&dir));
        let pool = JobPool::new(0, &sup, sink.clone(), transcript.clone());
        for id in 0..2u64 {
            transcript.write_original(id, "00:00:01", &format!("原文{id}"));
            pool.submit(id, || {});
        }
        pool.retire();
        let mut batch = Vec::new();
        while sink.drain_batch(&mut batch, Duration::from_millis(20)) {}
        transcript.close();
        let all_path = transcript
            .session_paths()
            .get("all")
            .cloned()
            .expect("all 转录文件应已开");
        let text = std::fs::read_to_string(&all_path).unwrap();
        assert!(
            text.contains("原文0") && text.contains("原文1"),
            "让位/丢弃段的原文必须落 all 转录，实际：{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        sup.join_all();
    }

    /// ACR-4：翻译装置未就绪时，识别文字照常记账落盘（原版照记，只是译文栏失败）。
    /// 病灶 = 旧序把 `asr_count`/`write_original` 放在取翻译器之后，未就绪即提前
    /// return——转录三文件中间缺一片且统计不计数。
    #[test]
    fn not_ready_still_records_transcript_and_stats() {
        let dir = tmp_models_dir("acr4-not-ready");
        let transcript = lt_audio::transcript::TranscriptWriter::new(&dir);
        let stats = test_session_stats();
        let sink = EventArtery::new();

        // tl = None（配置无效/无活动模型）走完整提交路径
        commit_text(
            "auto",
            "zh",
            None,
            &sink,
            &transcript,
            &stats,
            "你好世界",
            "en",
            12.5,
            &Msg::new(|k| k.to_string(), || "zh".into()),
        );

        // 事件面：AddMessage（照常上屏）+ NotReady 失败结论 + 统计快照（计数已变）
        let mut events = Vec::new();
        let mut batch = Vec::new();
        while sink.drain_batch(&mut batch, Duration::from_millis(20)) {
            events.append(&mut batch);
        }
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UiEvent::AddMessage { .. })),
            "识别文字照常上屏"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                UiEvent::TranslationFailed {
                    kind: FailureKind::NotReady,
                    ..
                }
            )),
            "未就绪以失败结论下发"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, UiEvent::UpdateStats { asr_n: 1, .. })),
            "asr 计数自增 1 并随行快照"
        );

        // 转录面：original 有行、all 有成对收口的原文块（无 pending 悬挂）
        transcript.close();
        let paths = transcript.session_paths();
        let original = std::fs::read_to_string(paths.get("original").unwrap()).unwrap();
        let all = std::fs::read_to_string(paths.get("all").unwrap()).unwrap();
        assert!(
            original.contains("你好世界"),
            "original 必须有该行：{original}"
        );
        assert!(
            all.contains("你好世界"),
            "all 必须有成对收口的原文块（ACR-4）：{all}"
        );
        assert!(
            !all.contains("->"),
            "未就绪不得出现译文行（失败而非假译文）：{all}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── D-85/G/H：会话级累计 + 双币种 ──

    /// 会话统计跨装置重建不清零；金额按**每笔当时的单价与币种**归账。
    ///
    /// 2026-09-11 评审修复补强：旧测试只对 `TlStats` 手工记三笔、从不建第二个
    /// 装置——把共享点改回"每个装置自建"测试照样绿。本版**真建三个装置**并
    /// 断言共享同一账本（`Arc::ptr_eq`）+ 端到端金额归账。
    #[test]
    fn session_stats_accumulate_across_rig_replacement() {
        let sup = test_sup();
        let bus = test_bus(lt_proto::Settings::default());
        let sink = EventArtery::new();
        let session_stats = test_session_stats();
        let transcript = test_transcript();
        let learned = test_learned();
        let degraded = test_degraded_notified();
        let eff = bus.load();
        let base = eff.raw.models[eff.raw.active_model].clone();

        let build = |name: &str, inp: f64, outp: f64, cur: &str| {
            let mut mc = base.clone();
            mc.name = name.into();
            mc.input_price = inp;
            mc.output_price = outp;
            mc.currency = Some(cur.into());
            TlRig::from_effective(
                &mc,
                &eff,
                &bus,
                &sup,
                sink.clone(),
                transcript.clone(),
                learned.clone(),
                degraded.clone(),
                &session_stats,
                "zh",
            )
            .expect("配置应可构建")
            .expect("models 非空应产出装置")
        };

        // 第一笔：人民币供应商 ¥1/¥2 每 1M
        let rig1 = build("cny-1", 1.0, 2.0, "cny");
        rig1.stats
            .record_translation(1_000_000, 0, true, rig1.prices, rig1.currency);
        // 换成美元供应商 $0.5/$1.0（新装置）
        let rig2 = build("usd-1", 0.5, 1.0, "usd");
        assert!(
            Arc::ptr_eq(&rig1.stats, &rig2.stats),
            "换装置必须共享同一账本（唯一创建点在 Pipeline::start）"
        );
        rig2.stats
            .record_translation(0, 1_000_000, true, rig2.prices, rig2.currency);
        // 又切回人民币供应商
        let rig3 = build("cny-2", 1.0, 2.0, "cny");
        assert!(Arc::ptr_eq(&rig1.stats, &rig3.stats), "账本跨多次替换保持");
        rig3.stats
            .record_translation(0, 1_000_000, true, rig3.prices, rig3.currency);

        match session_stats.snapshot_event() {
            UiEvent::UpdateStats {
                tl_n,
                cost_cny,
                cost_usd,
                usage_known,
                ..
            } => {
                assert_eq!(tl_n, 3, "句数跨装置累计");
                assert!(
                    (cost_cny - 3.0).abs() < 1e-6,
                    "人民币账本=1+2，实际 {cost_cny}"
                );
                assert!((cost_usd - 1.0).abs() < 1e-6, "美元账本=1，实际 {cost_usd}");
                assert!(usage_known);
            }
            other => panic!("期望 UpdateStats，实际 {other:?}"),
        }
        for rig in [rig1, rig2, rig3] {
            rig.pool.shutdown();
        }
        sup.join_all();
    }

    /// 出现一次"用量未知"→ 快照 usage_known=false（费用可能偏低），
    /// 但已记的金额不丢
    #[test]
    fn session_stats_marks_partial_unknown() {
        let stats = TlStats::new();
        stats.record_translation(100, 50, true, (1.0, 1.0), lt_proto::Currency::Usd);
        stats.record_translation(100, 50, false, (1.0, 1.0), lt_proto::Currency::Usd);
        match stats.snapshot_event() {
            UiEvent::UpdateStats {
                cost_usd,
                usage_known,
                prompt_tokens,
                ..
            } => {
                assert!(!usage_known, "出现过未知 → 费用不完整");
                assert!(cost_usd > 0.0, "已发生的金额仍要记");
                assert_eq!(prompt_tokens, 200);
            }
            other => panic!("期望 UpdateStats，实际 {other:?}"),
        }
    }

    /// 从未观测到用量（只有失败调用）→ 总额为 0 且 unknown（界面显示 "—"）
    #[test]
    fn session_stats_zero_and_unknown_stays_unknown() {
        let stats = TlStats::new();
        stats.record_translation(0, 0, false, (0.0, 0.0), lt_proto::Currency::Cny);
        match stats.snapshot_event() {
            UiEvent::UpdateStats {
                cost_cny,
                cost_usd,
                usage_known,
                ..
            } => {
                assert_eq!((cost_cny, cost_usd), (0.0, 0.0));
                assert!(!usage_known);
            }
            other => panic!("期望 UpdateStats，实际 {other:?}"),
        }
    }
}
