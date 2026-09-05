//! capture 线程（原版 main.py `_capture_loop` 等价）。
//!
//! 从音频队列取 chunk → RMS 监视事件 → VAD 状态机 → 段事件。
//! 取数超时且 VAD 在说话时喂静音推进（等价原版超时分支）。

use crate::audio::{rms, BoundedDropQueue, CHUNK_SAMPLES};
use crate::vad::{ConfidenceSource, VadProcessor};
use crate::{CaptureEvent, SegmentSource};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// capture 循环参数
pub struct CaptureLoop {
    /// 音频后端产出的 16k mono chunk 队列（满丢旧）
    pub chunk_rx: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
    /// 事件出口（monitor / segment）
    pub events: crossbeam_channel::Sender<CaptureEvent>,
    /// 暂停标志（暂停时丢弃 chunk 不喂 VAD，对齐原版 _paused）
    pub paused: Arc<AtomicBool>,
}

impl CaptureLoop {
    /// 阻塞运行至 `running` 置 false。
    /// `vad` 由调用方构造（含置信度源与设置），跨重启复用。
    pub fn run<C: ConfidenceSource>(&self, vad: &mut VadProcessor<C>, running: &AtomicBool) {
        let silence_chunk = vec![0.0f32; CHUNK_SAMPLES];
        while running.load(Ordering::Relaxed) {
            match self.chunk_rx.pop_timeout(Duration::from_secs(1)) {
                None => {
                    // 超时：VAD 在说话则喂静音推进（原版 effective_silence_limit()+1 个）
                    if vad.is_speaking() && !self.paused.load(Ordering::Relaxed) {
                        let n = vad.effective_silence_limit() + 1;
                        for _ in 0..n {
                            if let Some(seg) = vad.process_chunk(&silence_chunk) {
                                let _ = self.events.send(CaptureEvent::Segment {
                                    source: SegmentSource::VadFlush,
                                    audio: seg,
                                });
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
                    let _ = self.events.send(CaptureEvent::Monitor {
                        rms: r,
                        vad: vad.last_confidence,
                        mic_rms,
                    });
                    if let Some(seg) = vad.process_chunk(&chunk) {
                        let _ = self.events.send(CaptureEvent::Segment {
                            source: SegmentSource::VadFlush,
                            audio: seg,
                        });
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

    fn setup() -> (
        Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        crossbeam_channel::Sender<CaptureEvent>,
        crossbeam_channel::Receiver<CaptureEvent>,
        Arc<AtomicBool>,
    ) {
        let q = Arc::new(BoundedDropQueue::new(100));
        let (tx, rx) = crossbeam_channel::unbounded();
        (q, tx, rx, Arc::new(AtomicBool::new(false)))
    }

    fn spawn<C: ConfidenceSource + 'static>(
        q: Arc<BoundedDropQueue<(Vec<f32>, Option<f32>)>>,
        tx: crossbeam_channel::Sender<CaptureEvent>,
        paused: Arc<AtomicBool>,
        vad: VadProcessor<C>,
    ) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
        let running = Arc::new(AtomicBool::new(true));
        let lp = CaptureLoop { chunk_rx: q, events: tx, paused };
        let r = running.clone();
        let h = std::thread::spawn(move || {
            let mut vad = vad;
            lp.run(&mut vad, &r);
        });
        (h, running)
    }

    #[test]
    fn monitor_precedes_segment_and_chunk_rms_used() {
        let (q, tx, rx, paused) = setup();
        let vad = VadProcessor::new(Burst(40.into()), 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(q.clone(), tx, paused, vad);
        // chunk 值 0.5 → rms=0.5（monitor 事件应携带）
        for _ in 0..65 {
            q.push((vec![0.5f32; 512], Some(0.25f32)));
        }
        // 前若干事件必为 Monitor（语音期），首段事件出现前至少有 1 条 monitor
        let mut monitors = 0;
        loop {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(CaptureEvent::Monitor { rms, vad: _, mic_rms }) => {
                    monitors += 1;
                    assert_eq!(rms, 0.5);
                    assert_eq!(mic_rms, Some(0.25));
                }
                Ok(CaptureEvent::Segment { source, audio }) => {
                    assert!(monitors >= 1, "段事件前应有监视事件");
                    assert_eq!(source, SegmentSource::VadFlush);
                    assert!(audio.len() >= 40 * 512);
                    break;
                }
                Err(_) => panic!("未收到段事件 (monitors={monitors})"),
            }
        }
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn pause_drops_chunks() {
        let (q, tx, rx, paused) = setup();
        paused.store(true, Ordering::Relaxed);
        let vad = VadProcessor::new(Zero, 16000, 0.5, 1.0, 15.0, 0.032);
        let (h, running) = spawn(q.clone(), tx, paused, vad);
        for _ in 0..10 {
            q.push((vec![0.1f32; 512], None));
        }
        std::thread::sleep(Duration::from_millis(200));
        assert!(rx.try_recv().is_err(), "暂停时不应有任何事件");
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }

    #[test]
    fn timeout_feeds_silence_to_advance_vad() {
        // 不往队列放数据，VAD 已有缓冲 → 超时路径喂静音收段
        let q = Arc::new(BoundedDropQueue::<(Vec<f32>, Option<f32>)>::new(100));
        let (tx, rx) = crossbeam_channel::unbounded();
        let paused = Arc::new(AtomicBool::new(false));
        let mut vad = VadProcessor::new(Burst(40.into()), 16000, 0.5, 1.0, 15.0, 0.032);
        let chunk = vec![0.1f32; 512];
        for _ in 0..40 {
            vad.process_chunk(&chunk);
        }
        assert!(vad.is_speaking());
        let running = Arc::new(AtomicBool::new(true));
        let lp = CaptureLoop { chunk_rx: q, events: tx, paused };
        let r = running.clone();
        let h = std::thread::spawn(move || lp.run(&mut vad, &r));
        let seg = loop {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(CaptureEvent::Segment { audio, .. }) => break audio,
                Ok(_) => continue,
                Err(_) => panic!("超时路径未产出段"),
            }
        };
        assert!(seg.len() >= 40 * 512);
        running.store(false, Ordering::Relaxed);
        let _ = h.join();
    }
}
