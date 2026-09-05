//! capture 线程（原版 main.py `_capture_loop` 等价）。
//!
//! 从音频队列取 chunk → RMS 监视回调 → VAD 状态机 → 段直接入段队列。
//! 无中间分流线程：monitor 数据由调用方回调直发（等价原版跨线程调
//! `update_monitor`），段由本线程直塞段队列（等价原版 `_enqueue_asr`）。
//! 取数超时且 VAD 在说话时喂静音推进（等价原版超时分支）。

use crate::audio::{rms, BoundedDropQueue, CHUNK_SAMPLES};
use crate::vad::{ConfidenceSource, VadProcessor};
use crate::SegmentSource;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    /// VAD 参数热更新槽（原版面板经 _vad_lock 调 update_settings 的等价物）：
    /// UI 侧塞入 Some(新设置)，capture 线程每 chunk 取走应用（take-and-clear）
    pub vad_update: Arc<std::sync::Mutex<Option<crate::vad::VadSettings>>>,
}

impl<F: Fn(f32, f64, Option<f32>) + Send> CaptureLoop<F> {
    /// 阻塞运行至 `running` 置 false。
    /// `vad` 由调用方构造（含置信度源与设置），跨重启复用。
    pub fn run<C: ConfidenceSource>(&self, vad: &mut VadProcessor<C>, running: &AtomicBool) {
        let silence_chunk = vec![0.0f32; CHUNK_SAMPLES];
        while running.load(Ordering::Relaxed) {
            // 应用挂起的 VAD 参数（原版 vad_processor.update_settings）
            if let Some(s) = self.vad_update.lock().unwrap().take() {
                vad.update_settings(&s);
            }
            match self.chunk_rx.pop_timeout(Duration::from_secs(1)) {
                None => {
                    // 超时：VAD 在说话则喂静音推进（原版 effective_silence_limit()+1 个）
                    if vad.is_speaking() && !self.paused.load(Ordering::Relaxed) {
                        let n = vad.effective_silence_limit() + 1;
                        for _ in 0..n {
                            if let Some(seg) = vad.process_chunk(&silence_chunk) {
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
                    (self.monitor)(r, vad.last_confidence, mic_rms);
                    if let Some(seg) = vad.process_chunk(&chunk) {
                        self.segment_tx.push((SegmentSource::VadFlush, seg));
                    }
                }
            }
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

    fn setup() -> (
        Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
        MonitorLog,
        Arc<AtomicBool>,
    ) {
        let q = Arc::new(BoundedDropQueue::new(100));
        let seg = Arc::new(BoundedDropQueue::new(16));
        (q, seg, Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)))
    }

    fn spawn<C: ConfidenceSource + 'static>(
        q: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        seg_tx: Arc<BoundedDropQueue<(SegmentSource, Vec<f32>)>>,
        monitors: MonitorLog,
        paused: Arc<AtomicBool>,
        vad: VadProcessor<C>,
    ) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
        let running = Arc::new(AtomicBool::new(true));
        let lp = CaptureLoop {
            chunk_rx: q,
            segment_tx: seg_tx,
            monitor: move |rms, vad, mic_rms| {
                monitors.lock().unwrap().push((rms, vad, mic_rms));
            },
            paused,
            vad_update: Arc::new(std::sync::Mutex::new(None)),
        };
        let r = running.clone();
        let h = std::thread::spawn(move || {
            let mut vad = vad;
            lp.run(&mut vad, &r);
        });
        (h, running)
    }

    #[test]
    fn monitor_precedes_segment_and_chunk_rms_used() {
        let (q, seg_tx, monitors, paused) = setup();
        let vad = VadProcessor::new(Burst(40.into()), 16000, 0.5, 1.0, 15.0, 0.032);
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
        assert!(mons.len() >= 1, "段入队前应有监视回调");
        for (rms, _, mic_rms) in mons.iter() {
            assert_eq!(*rms, 0.5);
            assert_eq!(*mic_rms, Some(0.25));
        }
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn pause_drops_chunks() {
        let (q, seg_tx, monitors, paused) = setup();
        paused.store(true, Ordering::Relaxed);
        let vad = VadProcessor::new(Zero, 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(q.clone(), seg_tx.clone(), monitors.clone(), paused, vad);
        for _ in 0..10 {
            q.push((vec![0.1f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert!(seg_tx.is_empty(), "暂停时不应产出任何段");
        assert!(monitors.lock().unwrap().is_empty(), "暂停时不应有任何监视回调");
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn timeout_feeds_silence_to_advance_vad() {
        // 不往队列放数据，VAD 已有缓冲 → 超时路径喂静音收段
        let q = Arc::new(BoundedDropQueue::<(Vec<f32>, Option<f32>)>::new(100));
        let seg_tx = Arc::new(BoundedDropQueue::<(SegmentSource, Vec<f32>)>::new(16));
        let paused = Arc::new(AtomicBool::new(false));
        let mut vad = VadProcessor::new(Burst(40.into()), 16000, 0.5, 1.0, 15.0, 0.032);
        let chunk = vec![0.1f32; 512];
        for _ in 0..40 {
            vad.process_chunk(&chunk);
        }
        assert!(vad.is_speaking());
        let running = Arc::new(AtomicBool::new(true));
        let lp = CaptureLoop {
            chunk_rx: q,
            segment_tx: seg_tx.clone(),
            monitor: |_, _, _| {},
            paused,
            vad_update: Arc::new(std::sync::Mutex::new(None)),
        };
        let r = running.clone();
        let h = std::thread::spawn(move || lp.run(&mut vad, &r));
        let (source, seg) = match seg_tx.pop_timeout(Duration::from_secs(3)) {
            Some(v) => v,
            None => panic!("超时路径未产出段"),
        };
        assert_eq!(source, SegmentSource::VadFlush);
        assert!(seg.len() >= 40 * 512);
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }
}
