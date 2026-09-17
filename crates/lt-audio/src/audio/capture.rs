//! capture 线程（原版 main.py `_capture_loop` 等价）。
//!
//! 从音频队列取 chunk → RMS 监视回调 → VAD 状态机 → 段直接入段队列。
//! 无中间分流线程：monitor 数据由调用方回调直发（等价原版跨线程调
//! `update_monitor`），段由本线程直塞段队列（等价原版 `_enqueue_asr`）。
//! 取数超时且 VAD 在说话时喂静音推进（等价原版超时分支）。
//!
//! VAD 为跨线程共享（`Arc<Mutex<VadProcessor>>`）：capture 只写、ASR 线程在
//! 增量识别时读（peek/trim/speech_samples），锁粒度 = 单次方法调用，对齐原版
//! `_vad_lock`。增量触发判定也在本线程（原版 `_capture_loop` 的
//! speech_segment is None 分支）：条件满足塞 `SegmentSource::Interim` 空标记
//! 进段队列，**不在 capture 线程跑 ASR**（原版拓扑）。

use crate::audio::{rms, BoundedDropQueue, CHUNK_SAMPLES, TARGET_RATE};
use crate::vad::VadProcessor;
use crate::SegmentSource;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// 增量 ASR 跨线程控制块（原版 `_incremental_enabled` / `_interim_interval` /
/// `_last_interim_samples` / `_last_interim_check_time` 四个散字段的原子等价）。
/// capture 线程只读 enabled/interval + 写 last_check_ms；ASR 线程写
/// last_interim_samples——各写各的、无复合不变量，故用原子而非锁。
#[derive(Debug, Default)]
pub struct InterimControl {
    /// 增量识别开关（UI 经 `Pipeline::set_interim` 写）
    pub enabled: AtomicBool,
    /// 间隔秒数（f32 bits 存 AtomicU32；0 bits = 0.0s = 永不触发）
    pub interval_bits: AtomicU32,
    /// 上次 interim 消费时的 VAD 缓冲样本数（capture 读、ASR 写；原版
    /// `_last_interim_samples`，vad_flush 复位为 0）
    pub last_interim_samples: AtomicU64,
    /// 上次触发判定的 UNIX 毫秒时间戳（0 = 从未触发；原版
    /// `_last_interim_check_time` 用 perf_counter 秒，此处 epoch 毫秒等价——
    /// 只参与 ≥1s 冷却比较，单调性足够）
    pub last_check_ms: AtomicU64,
}

impl InterimControl {
    /// 热应用开关/间隔（原版 `_incremental_asr_cb`）。关闭时清进度计数，
    /// 重开从零起算（对齐 vad_flush 复位语义）；开启时不清（原版同）。
    pub fn set(&self, enabled: bool, interval: f32) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.interval_bits
            .store(interval.to_bits(), Ordering::Relaxed);
        if !enabled {
            self.last_interim_samples.store(0, Ordering::Relaxed);
            self.last_check_ms.store(0, Ordering::Relaxed);
        }
    }

    /// R31/D-72：会话边界复位——只清进度计数（enabled/interval 保持，
    /// 开关是用户意图，设备切换不应重置；vad_flush 同语义）
    pub fn reset_counter(&self) {
        self.last_interim_samples.store(0, Ordering::Relaxed);
        self.last_check_ms.store(0, Ordering::Relaxed);
    }
}

/// 触发判定纯函数（对照原版 main.py `_capture_loop` 的条件组合，真值表单测锚点）：
/// enabled × 在说话 × total/elapsed ≥ 间隔 × 冷却 ≥1s。
/// `interval < 1.0` 拒绝为防御性守卫（原版无此判；UI 下拉限 1..=10s，防
/// interval=0 时每秒空转触发）。
fn interim_due(
    enabled: bool,
    speaking: bool,
    interval: f32,
    total_samples: u64,
    last_samples: u64,
    now_ms: u64,
    last_check_ms: u64,
) -> bool {
    if !enabled || !speaking || interval < 1.0 {
        return false;
    }
    let total_dur = total_samples as f64 / TARGET_RATE as f64;
    let elapsed = total_samples.saturating_sub(last_samples) as f64 / TARGET_RATE as f64;
    total_dur >= interval as f64
        && elapsed >= interval as f64
        && now_ms.saturating_sub(last_check_ms) >= 1_000
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// capture 循环参数
pub struct CaptureLoop<F> {
    /// 音频后端产出的 16k mono chunk 队列（满丢旧）
    pub chunk_rx: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
    /// 段队列：capture 线程直接入队（等价原版 _enqueue_asr；满丢旧）
    pub segment_tx: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
    /// monitor 回调：(rms, vad 置信度, mic_rms)，等价原版 update_monitor 跨线程信号
    pub monitor: F,
    /// 暂停标志（暂停时丢弃 chunk 不喂 VAD，对齐原版 _paused）
    pub paused: Arc<AtomicBool>,
    /// VAD 生效设置发布格（架构 2.0 W4 设置总线读侧）：每循环轮询读
    /// `(版本, 设置)`，版本变化才应用（update_settings / 按模式换置信度源）。
    /// 替代旧"UI 塞槽 take-and-clear"（`vad_update`）——槽即镜像，同步靠人肉
    /// 边（R10）；总线发布为唯一事实源，读者无锁
    pub vad_tick: VadSource,
    /// 增量识别控制块（与 ASR 线程共享；见 [`InterimControl`]）
    pub interim: Arc<InterimControl>,
    /// 当前生效 VAD 模式（架构 2.0 W1/R2：检测模式变化以替换置信度源；
    /// 初值 = 启动装配所用模式，避免首帧把 Silero 重复加载一遍）
    pub current_mode: String,
    /// ACR-1a：本线程收尾完成标志（退出路径最后一步置位）。语义 = "VAD 已无写者
    /// 且残余尾巴已入队"——ASR 线程的退出收尾循环据此判定段队列内容已终。
    /// 可见性由段队列自身的内部互斥量提供（push 与 try_pop 同一把锁），
    /// 标志本身无需 Acquire/Release；**若队列换无锁实现，此处必须升序**。
    pub capture_done: Arc<AtomicBool>,
}

/// VAD 生效设置发布格读数（`(版本, 设置)` 快照；版本单调递增）。
/// 由编排域经设置总线构造——本 crate 不依赖 orchestrator（依赖白名单），
/// 以闭包形态注入（发送方与接收方仅以元组数据耦合）。
pub type VadSource = Arc<dyn Fn() -> (u64, crate::vad::VadSettings) + Send + Sync>;

impl<F: Fn(f32, f64, Option<f32>) + Send> CaptureLoop<F> {
    /// 阻塞运行至 `running`（= stop 标志）置 true。
    /// `vad` 由调用方构造并与 ASR 线程共享（增量识别跨线程读）。
    /// 形参特化为 boxed 默认形态（架构 2.0 W1/R2）：模式热切换需要经
    /// `make_confidence_source` 重建源——只有 boxed trait object 可跨型替换；
    /// 具体类型源（测试 mock）装箱传入即可
    pub fn run(&mut self, vad: &Arc<Mutex<VadProcessor>>, running: &AtomicBool) {
        let silence_chunk = vec![0.0f32; CHUNK_SAMPLES];
        // W4：上次已应用的发布版本（0 = 未应用 → 启动后首 tick 应用一次；
        // 值与启动装配相同则幂等无害）
        let mut applied_version: u64 = 0;
        while !running.load(Ordering::Relaxed) {
            // 应用生效 VAD 参数（W4 总线发布格：版本变了才 update_settings；
            // 替代旧槽 take-and-clear——同一发布内全字段整体重算，无半应用窗口）
            let (ver, s) = (self.vad_tick)();
            if ver != applied_version {
                let mut v = vad.lock().unwrap();
                if self.current_mode != s.mode {
                    // 架构 2.0 W1/R2：模式变化必须替换置信度源——update_settings
                    // 只更新阈值语义，源不换则 silero→energy/disabled 热切换不生效。
                    // 模式不变（仅阈值/时长）走 update_settings，绝不换源
                    let src = crate::vad::make_confidence_source(&s.mode, s.energy_threshold);
                    v.set_confidence_source(src);
                    self.current_mode = s.mode.clone();
                }
                v.update_settings(&s);
                applied_version = ver;
            }
            match self.chunk_rx.pop_timeout(Duration::from_secs(1)) {
                None => {
                    // 超时：VAD 在说话则喂静音推进（原版 effective_silence_limit()+1 个）
                    let (speaking, n) = {
                        let v = vad.lock().unwrap();
                        (v.is_speaking(), v.effective_silence_limit() + 1)
                    };
                    if speaking && !self.paused.load(Ordering::Relaxed) {
                        for _ in 0..n {
                            let seg = vad.lock().unwrap().process_chunk(&silence_chunk);
                            if let Some(seg) = seg {
                                // 原版 _enqueue_asr("vad_flush", seg)：capture 线程直塞段队列
                                self.segment_tx.push((SegmentSource::VadFlush, seg));
                                break;
                            }
                        }
                    }
                }
                Some((chunk, mic_rms)) => {
                    if self.paused.load(Ordering::Relaxed) {
                        continue;
                    }
                    let r = rms(&chunk);
                    // 原版顺序：monitor 用的是上一 chunk 的 last_confidence
                    let last_confidence = vad.lock().unwrap().last_confidence;
                    (self.monitor)(r, last_confidence, mic_rms);
                    // 先绑定结果再分支：MutexGuard 临时值在 if let 的 scrutinee 里
                    // 存活到整个语句结束（edition 2021），否则 else 内再锁 vad 即自死锁
                    let seg = vad.lock().unwrap().process_chunk(&chunk);
                    if let Some(seg) = seg {
                        self.segment_tx.push((SegmentSource::VadFlush, seg));
                    } else {
                        // 仍在积累 —— 增量触发判定（原版 main.py:1657-1668；
                        // 仅数据 chunk 分支判，超时静音分支不判，对齐原版）
                        self.maybe_trigger_interim(vad);
                    }
                }
            }
        }

        // ── 退出收尾（ACR-1a）──
        // 把 VAD 残余作为收尾段入队：退出瞬间耳朵里攒着的话（最多 max_speech）
        // 不再直接消失（等价 Python main.py:1208-1216 的 force_flush + 处理）。
        // 有意不设 min_speech 门槛（D-94）：退出时用户刚说的话优先保真，下游
        // reject_segment 三层过滤 / 噪声过滤仍兜底。
        // 中毒防御：历史持锁方 panic 过的话 into_inner 取回数据，不让退出路径
        // 再 panic（否则 capture_done 永不置位，尾巴静默丢失）。
        let tail = vad
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .force_flush();
        if let Some(seg) = tail {
            self.segment_tx.push((SegmentSource::VadFlush, seg));
        }
        // 最后一步：此后本线程不再触碰 VAD——ASR 线程据此判定队列已终
        self.capture_done.store(true, Ordering::Relaxed);
    }

    /// 增量触发判定：读 VAD 状态 → 纯函数判定 → 塞空音频 interim 标记。
    /// 原版 `_asr_ready` 条件不搬：Rust ASR 线程无引擎时本就吞段待命，无害。
    /// 严守「不在持 vad 锁时碰 segment_tx」。
    fn maybe_trigger_interim(&self, vad: &Arc<Mutex<VadProcessor>>) {
        if !self.interim.enabled.load(Ordering::Relaxed) {
            return;
        }
        let interval = f32::from_bits(self.interim.interval_bits.load(Ordering::Relaxed));
        let (total, speaking) = {
            let v = vad.lock().unwrap();
            (v.speech_samples(), v.is_speaking())
        };
        let now_ms = now_epoch_ms();
        if interim_due(
            true,
            speaking,
            interval,
            total as u64,
            self.interim.last_interim_samples.load(Ordering::Relaxed),
            now_ms,
            self.interim.last_check_ms.load(Ordering::Relaxed),
        ) {
            self.interim.last_check_ms.store(now_ms, Ordering::Relaxed);
            self.segment_tx.push((SegmentSource::Interim, Vec::new()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vad::ConfidenceSource;
    use std::sync::Mutex;

    /// 恒 0 置信度（永不说话）
    struct Zero;
    impl ConfidenceSource for Zero {
        fn confidence(&mut self, _c: &[f32]) -> anyhow::Result<f64> {
            Ok(0.0)
        }
    }

    /// 剩余配额内置信 1.0，其后 0
    struct Burst(std::sync::atomic::AtomicUsize);
    impl ConfidenceSource for Burst {
        fn confidence(&mut self, _c: &[f32]) -> anyhow::Result<f64> {
            // 饱和扣减（fetch_sub 下溢在 debug 构建会 panic）
            let mut prev = self.0.load(Ordering::Relaxed);
            loop {
                match self.0.compare_exchange_weak(
                    prev,
                    prev.saturating_sub(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(p) => prev = p,
                }
            }
            Ok(if prev > 0 { 1.0 } else { 0.0 })
        }
    }

    /// monitor 回调收集日志：(rms, vad, mic_rms)
    type MonitorLog = Arc<Mutex<Vec<(f32, f64, Option<f32>)>>>;

    /// 测试固定装置（队列 ×2 + 日志 + 运行标志）
    type TestRig = (
        Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
        MonitorLog,
        Arc<AtomicBool>,
    );

    /// 测试源装箱（run 特化为 boxed 默认形态后的统一入口）
    fn boxed(src: impl ConfidenceSource + 'static) -> Box<dyn ConfidenceSource + Send> {
        Box::new(src)
    }

    fn setup() -> TestRig {
        let q = Arc::new(BoundedDropQueue::new(100, "test-chunk"));
        let seg = Arc::new(BoundedDropQueue::new(16, "test-seg"));
        (
            q,
            seg,
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(
        q: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        seg_tx: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
        monitors: MonitorLog,
        paused: Arc<AtomicBool>,
        vad: VadProcessor,
        interim: Arc<InterimControl>,
    ) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
        // 正确语义：running=stop 标志，false 运行、true 停止（对齐 Pipeline::stop）
        let running = Arc::new(AtomicBool::new(false));
        let mut lp = CaptureLoop {
            chunk_rx: q,
            segment_tx: seg_tx,
            monitor: move |rms, vad, mic_rms| {
                monitors.lock().unwrap().push((rms, vad, mic_rms));
            },
            paused,
            // 版本 0 恒等于 applied_version：不触发应用/换源（测试源为自定义
            // mock，走 make_confidence_source 会加载真 Silero）
            vad_tick: Arc::new(|| (0, crate::vad::VadSettings::default())),
            interim,
            current_mode: String::new(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let r = running.clone();
        let h = std::thread::spawn(move || {
            lp.run(&vad, &r);
        });
        (h, running)
    }

    #[test]
    fn monitor_precedes_segment_and_chunk_rms_used() {
        let (q, seg_tx, monitors, paused) = setup();
        let vad = VadProcessor::new(boxed(Burst(40.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(
            q.clone(),
            seg_tx.clone(),
            monitors.clone(),
            paused,
            vad,
            Default::default(),
        );
        // chunk 值 0.5 → rms=0.5（monitor 回调应携带）
        for _ in 0..65 {
            q.push((vec![0.5f32; 512], Some(0.25f32)));
        }
        // 等首个段出现；capture 线程内 monitor 回调先于段入队（同线程程序顺序，
        // 段队列互斥锁的获取/释放保证此处读到的 monitor 日志已含此前全部回调）
        let (source, audio) = match seg_tx.pop_timeout(Duration::from_secs(3)) {
            Some(v) => v,
            None => panic!("未收到段 (monitors={})", monitors.lock().unwrap().len()),
        };
        assert_eq!(source, SegmentSource::VadFlush);
        assert!(audio.len() >= 40 * 512);
        let mons = monitors.lock().unwrap();
        assert!(!mons.is_empty(), "段入队前应有监视回调");
        for (rms, _, mic_rms) in mons.iter() {
            assert_eq!(*rms, 0.5);
            assert_eq!(*mic_rms, Some(0.25));
        }
        running.store(true, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn pause_drops_chunks() {
        let (q, seg_tx, monitors, paused) = setup();
        paused.store(true, Ordering::Relaxed);
        let vad = VadProcessor::new(boxed(Zero), 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(
            q.clone(),
            seg_tx.clone(),
            monitors.clone(),
            paused,
            vad,
            Default::default(),
        );
        for _ in 0..10 {
            q.push((vec![0.1f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert!(seg_tx.is_empty(), "暂停时不应产出任何段");
        assert!(
            monitors.lock().unwrap().is_empty(),
            "暂停时不应有任何监视回调"
        );
        running.store(true, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn timeout_feeds_silence_to_advance_vad() {
        // 不往队列放数据，VAD 已有缓冲 → 超时路径喂静音收段
        let q = Arc::new(BoundedDropQueue::<(Vec<f32>, Option<f32>)>::new(
            100,
            "test-chunk",
        ));
        let seg_tx = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(
            16, "test-seg",
        ));
        let paused = Arc::new(AtomicBool::new(false));
        let mut vad = VadProcessor::new(boxed(Burst(40.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let chunk = vec![0.1f32; 512];
        for _ in 0..40 {
            vad.process_chunk(&chunk);
        }
        assert!(vad.is_speaking());
        let running = Arc::new(AtomicBool::new(false));
        let mut lp = CaptureLoop {
            chunk_rx: q,
            segment_tx: seg_tx.clone(),
            monitor: |_, _, _| {},
            paused,
            vad_tick: Arc::new(|| (0, crate::vad::VadSettings::default())),
            interim: Default::default(),
            current_mode: String::new(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let r = running.clone();
        let h = std::thread::spawn(move || lp.run(&vad, &r));
        let (source, seg) = match seg_tx.pop_timeout(Duration::from_secs(3)) {
            Some(v) => v,
            None => panic!("超时路径未产出段"),
        };
        assert_eq!(source, SegmentSource::VadFlush);
        assert!(seg.len() >= 40 * 512);
        running.store(true, Ordering::Relaxed);
        let _ = h.join();
    }

    // ── W4：设置总线发布格（版本变化 → 应用；版本不变 → 零动作） ──

    #[test]
    fn vad_tick_applied_when_version_changes() {
        let q = Arc::new(BoundedDropQueue::<(Vec<f32>, Option<f32>)>::new(
            100,
            "test-chunk",
        ));
        let seg_tx = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(
            16, "test-seg",
        ));
        let paused = Arc::new(AtomicBool::new(false));
        let vad = VadProcessor::new(boxed(Burst(1000.into())), 16000, 0.5, 1.0, 8.0, 0.032);
        let tick_cell = Arc::new(Mutex::new((0u64, crate::vad::VadSettings::default())));
        let reader: VadSource = {
            let c = tick_cell.clone();
            Arc::new(move || c.lock().unwrap().clone())
        };
        let running = Arc::new(AtomicBool::new(false));
        let q_feed = q.clone();
        let mut lp = CaptureLoop {
            chunk_rx: q,
            segment_tx: seg_tx.clone(),
            monitor: |_, _, _| {},
            paused,
            vad_tick: reader,
            interim: Default::default(),
            current_mode: "silero".into(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let vad_obs = vad.clone();
        let r = running.clone();
        let h = std::thread::spawn(move || lp.run(&vad, &r));
        // 喂弱数据让循环跑起来，确认版本 0 不应用
        for _ in 0..4 {
            q_feed.push((vec![0.0f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(
            vad_obs.lock().unwrap().max_speech_samples(),
            (8.0 * 16000.0) as usize
        );
        // 发布版本 1 + 新阈值/时长 → 下一循环轮询应用
        let s = crate::vad::VadSettings {
            max_speech_duration: 15.0,
            ..Default::default()
        };
        *tick_cell.lock().unwrap() = (1, s);
        for _ in 0..4 {
            q_feed.push((vec![0.0f32; 512], None));
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while vad_obs.lock().unwrap().max_speech_samples() != (15.0 * 16000.0) as usize {
            assert!(std::time::Instant::now() < deadline, "版本 1 设置未应用");
            std::thread::sleep(Duration::from_millis(20));
        }
        // 版本不变（再读同版本）不重复应用（换断言：timeout 后会轮询下一轮）
        running.store(true, Ordering::Relaxed);
        let _ = h.join();
    }

    // ── 增量触发判定（对照原版 main.py:1657-1668）──

    /// 真值表：enabled × speaking × interval × total/elapsed × cooldown
    #[test]
    fn interim_due_truth_table() {
        const NOW: u64 = 1_700_000_000_000; // 任意 epoch 毫秒
                                            // 全条件满足（cooldown 从未触发过 → last_check_ms=0 视为远超冷却）
        assert!(interim_due(true, true, 2.0, 16000 * 3, 16000, NOW, 0));
        // 冷却恰好 1s → 通过（原版 >= 1.0）
        assert!(interim_due(
            true,
            true,
            2.0,
            16000 * 3,
            16000,
            NOW,
            NOW - 1_000
        ));
        // 开关关 → false
        assert!(!interim_due(false, true, 2.0, 16000 * 3, 0, NOW, 0));
        // 不在说话 → false
        assert!(!interim_due(true, false, 2.0, 16000 * 3, 0, NOW, 0));
        // interval < 1.0（防御守卫）→ false
        assert!(!interim_due(true, true, 0.5, 16000 * 3, 0, NOW, 0));
        // 累计缓冲不足一个间隔 → false
        assert!(!interim_due(true, true, 2.0, 16000, 0, NOW, 0));
        // 距上次消费不足一个间隔（elapsed < interval）→ false
        assert!(!interim_due(true, true, 2.0, 16000 * 3, 16000 * 2, NOW, 0));
        // 冷却不足 1s → false
        assert!(!interim_due(
            true,
            true,
            2.0,
            16000 * 3,
            16000,
            NOW,
            NOW - 999
        ));
    }

    #[test]
    fn interim_control_set_clears_progress_on_disable() {
        let c = InterimControl::default();
        c.set(true, 2.5);
        assert!(c.enabled.load(Ordering::Relaxed));
        assert_eq!(f32::from_bits(c.interval_bits.load(Ordering::Relaxed)), 2.5);
        c.last_interim_samples.store(32000, Ordering::Relaxed);
        c.last_check_ms.store(12345, Ordering::Relaxed);
        // 关闭 → 进度清零
        c.set(false, 2.5);
        assert_eq!(c.last_interim_samples.load(Ordering::Relaxed), 0);
        assert_eq!(c.last_check_ms.load(Ordering::Relaxed), 0);
        assert!(!c.enabled.load(Ordering::Relaxed));
        // 重开不清（此前清过了）
        c.last_interim_samples.store(100, Ordering::Relaxed);
        c.set(true, 1.0);
        assert_eq!(c.last_interim_samples.load(Ordering::Relaxed), 100);
    }

    /// 集成：连续说话累计 ≥1 间隔后，capture 塞出空音频 Interim 标记
    #[test]
    fn interim_marker_pushed_when_speaking_long_enough() {
        let (q, seg_tx, _monitors, paused) = setup();
        // 200 chunk 配额 ≈ 6.4s 连续语音（间隔 1s 时远超触发线，且 < max 15s 不收段）
        let vad = VadProcessor::new(boxed(Burst(200.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let interim = Arc::new(InterimControl::default());
        interim.set(true, 1.0);
        let (h, running) = spawn(
            q.clone(),
            seg_tx.clone(),
            _monitors,
            paused,
            vad,
            interim.clone(),
        );
        for _ in 0..80 {
            q.push((vec![0.5f32; 512], None)); // 80×512 = 40960 ≈ 2.6s
        }
        // 应收到 Interim 标记（空音频）；说话未断，不应有 VadFlush
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut got_interim = false;
        while std::time::Instant::now() < deadline {
            match seg_tx.pop_timeout(Duration::from_millis(300)) {
                Some((SegmentSource::Interim, audio)) => {
                    assert!(
                        audio.is_empty(),
                        "interim 标记必须为空音频（原版 (\"interim\", None)）"
                    );
                    got_interim = true;
                    break;
                }
                Some((SegmentSource::VadFlush, _)) => panic!("连续语音中不应产出 VadFlush"),
                None => continue,
            }
        }
        assert!(got_interim, "说话 ≥1 间隔后应触发 interim 标记");
        // 触发后 last_check_ms 已记录（冷却计时起点）
        assert!(interim.last_check_ms.load(Ordering::Relaxed) > 0);
        running.store(true, Ordering::Relaxed);
        let _ = h.join();
    }

    /// 集成：开关关闭时零生产（回归锚点——关闭增量不得改变既有行为）
    #[test]
    fn interim_marker_absent_when_disabled_or_silent() {
        let (q, seg_tx, _monitors, paused) = setup();
        // 未启用：即便长语音也不产 interim 标记
        let vad = VadProcessor::new(boxed(Burst(200.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let interim = Arc::new(InterimControl::default());
        let (h, running) = spawn(
            q.clone(),
            seg_tx.clone(),
            _monitors.clone(),
            paused.clone(),
            vad,
            interim,
        );
        for _ in 0..80 {
            q.push((vec![0.5f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(400));
        assert!(seg_tx.is_empty(), "未启用增量时不应有任何标记/段");
        running.store(true, Ordering::Relaxed);
        let _ = h.join();

        // 启用但静音（不说话）：同样零生产
        let (q2, seg_tx2, monitors2, paused2) = setup();
        let vad2 = VadProcessor::new(boxed(Zero), 16000, 0.5, 1.0, 15.0, 0.032);
        let interim2 = Arc::new(InterimControl::default());
        interim2.set(true, 1.0);
        let (h2, running2) = spawn(
            q2.clone(),
            seg_tx2.clone(),
            monitors2,
            paused2,
            vad2,
            interim2,
        );
        for _ in 0..40 {
            q2.push((vec![0.0f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(400));
        assert!(seg_tx2.is_empty(), "静音（不在说话）不应触发 interim 标记");
        running2.store(true, Ordering::Relaxed);
        let _ = h2.join();
    }

    /// R31/D-72：reset_counter 只清进度计数（enabled/interval 保持）——
    /// 设备切换是会话边界，但用户开关意图不应被重置
    #[test]
    fn interim_reset_counter_keeps_toggles() {
        let c = InterimControl::default();
        c.set(true, 2.0);
        c.last_interim_samples.store(1234, Ordering::Relaxed);
        c.last_check_ms.store(999, Ordering::Relaxed);
        c.reset_counter();
        assert_eq!(c.last_interim_samples.load(Ordering::Relaxed), 0);
        assert_eq!(c.last_check_ms.load(Ordering::Relaxed), 0);
        assert!(c.enabled.load(Ordering::Relaxed), "开关保持");
        assert_eq!(
            f32::from_bits(c.interval_bits.load(Ordering::Relaxed)),
            2.0,
            "间隔保持"
        );
    }

    /// ACR-1a：退出路径把 VAD 残余作为收尾段入队，之后才置 capture_done。
    /// 病灶面：旧实现循环退出即返回，残余（说话中途退出最多 max_speech 秒）直接消失。
    /// 本测以 `running=true` 直调 `run()`——循环体不执行，只走退出收尾，全程同步无竞态。
    #[test]
    fn exit_flushes_vad_tail_then_reports_done() {
        let (_q, seg_tx, _monitors, _running) = setup();
        let mut vad = VadProcessor::new(boxed(Burst(40.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let chunk = vec![0.1f32; 512];
        for _ in 0..40 {
            vad.process_chunk(&chunk);
        }
        assert!(vad.is_speaking(), "前置：VAD 处于说话态且有缓冲");
        let vad = Arc::new(Mutex::new(vad));
        let done = Arc::new(AtomicBool::new(false));
        let mut lp = CaptureLoop {
            chunk_rx: Arc::new(BoundedDropQueue::new(100, "test-chunk")),
            segment_tx: seg_tx.clone(),
            monitor: |_, _, _| {},
            paused: Arc::new(AtomicBool::new(false)),
            vad_tick: Arc::new(|| (0, crate::vad::VadSettings::default())),
            interim: Default::default(),
            current_mode: String::new(),
            capture_done: done.clone(),
        };
        lp.run(&vad, &AtomicBool::new(true));
        let (source, seg) = seg_tx.try_pop().expect("退出残余必须作为收尾段入队");
        assert_eq!(source, SegmentSource::VadFlush);
        assert!(!seg.is_empty(), "收尾段应携带残余音频");
        assert!(seg_tx.try_pop().is_none(), "只推一条尾巴（不重复）");
        assert!(
            done.load(Ordering::Relaxed),
            "capture_done 必须置位（语义 = VAD 无写者且尾巴已入队）"
        );
        // 缓冲已随 force_flush 清空：二次退出不重复冲刷
        lp.run(&vad, &AtomicBool::new(true));
        assert!(seg_tx.try_pop().is_none(), "缓冲已空不得重复推尾巴");
    }

    /// ACR-1a：缓冲为空时退出路径不推段，但 capture_done 照常置位
    /// （ASR 侧据此判定队列已终，不能因为无尾巴就悬着）。
    #[test]
    fn exit_with_empty_buffer_still_reports_done() {
        let (_q, seg_tx, _monitors, _running) = setup();
        let vad = Arc::new(Mutex::new(VadProcessor::new(
            boxed(Zero),
            16000,
            0.5,
            1.0,
            15.0,
            0.032,
        )));
        let done = Arc::new(AtomicBool::new(false));
        let mut lp = CaptureLoop {
            chunk_rx: Arc::new(BoundedDropQueue::new(100, "test-chunk")),
            segment_tx: seg_tx.clone(),
            monitor: |_, _, _| {},
            paused: Arc::new(AtomicBool::new(false)),
            vad_tick: Arc::new(|| (0, crate::vad::VadSettings::default())),
            interim: Default::default(),
            current_mode: String::new(),
            capture_done: done.clone(),
        };
        lp.run(&vad, &AtomicBool::new(true));
        assert!(seg_tx.try_pop().is_none(), "无残余不推段");
        assert!(done.load(Ordering::Relaxed), "空缓冲同样置位（成对不变式）");
    }
}
