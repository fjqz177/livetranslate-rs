//! capture 线程（原版 main.py `_capture_loop` 等价）。
//!
//! 从音频队列取 chunk → RMS 监视回调 → VAD 状态机 → 段直接入段队列。
//! 无中间分流线程：monitor 数据由调用方回调直发（等价原版跨线程调
//! `update_monitor`），段由本线程直塞段队列（等价原版 `_enqueue_asr`）。
//! 取数超时且 VAD 在说话时喂静音推进（等价原版超时分支）。
//!
//! VAD 为跨线程共享（`Arc<Mutex<VadProcessor>>`）：capture 只写，锁粒度 =
//! 单次方法调用，对齐原版 `_vad_lock`。增量识别通道（interim 标记）已随
//! D-134 整体裁撤——capture 只产 VadFlush 段。

use crate::audio::{rms, BoundedDropQueue, CHUNK_SAMPLES};
use crate::vad::{lock_vad, VadProcessor};
use crate::SegmentSource;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// `vad` 由调用方构造；锁跨线程的唯一对手方 = shell 侧设备切换复位
    /// （`Pipeline::reset_session_after_device_switch`），识别线程不再触碰。
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
                let mut v = lock_vad(vad);
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
                        let v = lock_vad(vad);
                        (v.is_speaking(), v.effective_silence_limit() + 1)
                    };
                    if speaking && !self.paused.load(Ordering::Relaxed) {
                        for _ in 0..n {
                            let seg = lock_vad(vad).process_chunk(&silence_chunk);
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
                    let last_confidence = lock_vad(vad).last_confidence;
                    (self.monitor)(r, last_confidence, mic_rms);
                    // 先绑定结果再分支：MutexGuard 临时值在 if let 的 scrutinee 里
                    // 存活到整个语句结束（edition 2021），否则 else 内再锁 vad 即自死锁
                    let seg = lock_vad(vad).process_chunk(&chunk);
                    if let Some(seg) = seg {
                        self.segment_tx.push((SegmentSource::VadFlush, seg));
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

    fn spawn(
        q: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        seg_tx: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
        monitors: MonitorLog,
        paused: Arc<AtomicBool>,
        vad: VadProcessor,
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
            current_mode: String::new(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let r = running.clone();
        #[allow(clippy::disallowed_methods)] // cfg(test) 夹具（生产侧无 spawn；ADR-21 豁免编目）
        let h = std::thread::spawn(move || {
            lp.run(&vad, &r);
        });
        (h, running)
    }

    #[test]
    fn monitor_precedes_segment_and_chunk_rms_used() {
        let (q, seg_tx, monitors, paused) = setup();
        let vad = VadProcessor::new(boxed(Burst(40.into())), 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(q.clone(), seg_tx.clone(), monitors.clone(), paused, vad);
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
        let (h, running) = spawn(q.clone(), seg_tx.clone(), monitors.clone(), paused, vad);
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
            current_mode: String::new(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let r = running.clone();
        #[allow(clippy::disallowed_methods)] // cfg(test) 夹具（生产侧无 spawn；ADR-21 豁免编目）
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
            current_mode: "silero".into(),
            capture_done: Arc::new(AtomicBool::new(false)),
        };
        let vad = Arc::new(Mutex::new(vad));
        let vad_obs = vad.clone();
        let r = running.clone();
        #[allow(clippy::disallowed_methods)] // cfg(test) 夹具（生产侧无 spawn；ADR-21 豁免编目）
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
            current_mode: String::new(),
            capture_done: done.clone(),
        };
        lp.run(&vad, &AtomicBool::new(true));
        assert!(seg_tx.try_pop().is_none(), "无残余不推段");
        assert!(done.load(Ordering::Relaxed), "空缓冲同样置位（成对不变式）");
    }
}
