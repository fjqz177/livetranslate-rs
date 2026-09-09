//! 事件动脉（架构 2.0 W2，docs/architecture-v2.md §3.2.2）：后台线程 → UI
//! 事件的统一有界通路。
//!
//! 现状病灶（R23）：proxy 每条事件一次 `PostMessageW` 唤醒、winit 无任何
//! 合并——日志逐条直灌 + UpdateMonitor 31/s 泛洪，唤醒次数=事件条数。
//! 目标：
//!
//! ```text
//! 各生产线程(pipeline/backend/logbridge/supervisor) → push（满丢最旧）
//!   → BoundedDropQueue<UiEvent> cap 4096 + 丢弃计数
//!   → lt-artery-bridge 桥线程（Supervisor 出生，Always）
//!       pop_timeout 阻塞读（生产端 push 即 notify，等价 alacritty Wakeup
//!       `wake bounded(1)` 语义：桥在等待 → 立即唤醒；桥在排空 → 事件入队
//!       不丢）→ 单次排空至多 BATCH_MAX=256 条 → UiMsg::Events(Vec) 一次投递
//!   → shell 逐条分发
//! ```
//!
//! 不变量落点：
//! - **INV1 回流通路唯一**：生产侧全部改走 `push`（本文件之外不再直发
//!   proxy 的 UiEvent——`UiMsg::Cmd`/`UiMsg::AppCommand` 是控制面，不经动脉）；
//! - **INV8 事件序**：`BoundedDropQueue` 为 MPMC FIFO——跨生产者无全序，但
//!   同生产者严格 FIFO（实体粘线程：AddMessage/AsrDevice ← ASR 线程、
//!   UpdateTranslation ← tl worker、Download ← 下载会话线程），序即安全；
//! - **批量上限**：单次 wake 至多 256 条入一个 Vec——日志风暴下单帧不产生
//!   巨型 Vec；取不尽留待下次（只影响延迟不影响丢失）。三道闸层级：
//!   tracing broadcast 1024 环 + Lagged 限频（AH-7 既有）→ 动脉 4096 丢最旧
//!   + 丢弃计数 → 批量上限。

use lt_pipeline::audio::BoundedDropQueue;
use lt_proto::{UiEvent, UiMsg};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// 动脉容量（满丢最旧；与 JobPool keep-latest 64 同原语不同参数）
const ARTERY_CAP: usize = 4096;
/// 单次 wake 排空上限（第三道闸）
pub const BATCH_MAX: usize = 256;
/// 桥线程轮询戳脚（停机关：50ms 内响应；有事件时 push 的 notify 即时唤醒）
const BRIDGE_POLL: Duration = Duration::from_millis(50);

/// 生产端签名用类型别名（Arc 克隆廉价；闭包/线程共享）
pub type EventSink = Arc<EventArtery>;

/// 事件动脉句柄（可任意线程持有；push 非阻塞）
pub struct EventArtery {
    q: BoundedDropQueue<UiEvent>,
}

impl EventArtery {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            q: BoundedDropQueue::new(ARTERY_CAP, "artery"),
        })
    }

    /// 线程安全生产（满丢最旧；丢弃计数供水位观测/告警）
    pub fn push(&self, ev: UiEvent) {
        self.q.push(ev);
    }

    /// 桥线程主体（Supervisor factory 用：死亡重生 = 干净循环；INV5）。
    /// 退出条件：`stop` 置位（停机协议在 main 先于 join 置位）。
    pub fn bridge_loop(self: Arc<Self>, proxy: EventLoopProxy<UiMsg>, stop: Arc<AtomicBool>) {
        let mut batch = Vec::with_capacity(BATCH_MAX);
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let Some(first) = self.q.pop_timeout(BRIDGE_POLL) else {
                continue;
            };
            batch.clear();
            batch.push(first);
            while batch.len() < BATCH_MAX {
                match self.q.try_pop() {
                    Some(v) => batch.push(v),
                    None => break,
                }
            }
            if proxy.send_event(UiMsg::Events(std::mem::take(&mut batch))).is_err() {
                // 事件循环已退出（停机期）——静默收尾，不再空转
                return;
            }
        }
    }

    /// 排空至多 `BATCH_MAX` 条（单测面：批量上限 + FIFO 序）
    #[cfg(test)]
    fn drain_one_batch(&self, out: &mut Vec<UiEvent>) {
        out.clear();
        if let Some(first) = self.q.try_pop() {
            out.push(first);
            while out.len() < BATCH_MAX {
                match self.q.try_pop() {
                    Some(v) => out.push(v),
                    None => break,
                }
            }
        }
    }

    /// 经监督器出生桥线程（W2/INV3：`lt-artery-bridge`，Always 重生——
    /// 工厂构造干净循环；停机由 `stop` 置位后 join，见 main 停机序）。
    pub fn spawn_bridge(
        self: &Arc<Self>,
        sup: &crate::supervisor::Supervisor,
        proxy: EventLoopProxy<UiMsg>,
        stop: std::sync::Arc<AtomicBool>,
    ) {
        let arte = self.clone();
        let proxy = proxy.clone();
        let stop = stop.clone();
        sup.spawn(
            lt_proto::ThreadRole::ArteryBridge,
            "lt-artery-bridge",
            crate::supervisor::Policy::Always,
            move || {
                let arte = arte.clone();
                let proxy = proxy.clone();
                let stop = stop.clone();
                Box::new(move || arte.bridge_loop(proxy, stop))
            },
        );
    }
}

// 注：方案图上 `wake bounded(1)` 信号位由 BoundedDropQueue 内 Condvar
// 承担——`push` 的 `notify_one` 等价"唤醒一次"，桥不在等待时事件已在队中
// 不会被跳过（排空循环连续 try_pop），无需独立信号通道（ADR-2 同源）。

#[cfg(test)]
mod tests {
    use super::*;
    use lt_proto::AppCommand;

    /// INV8：单生产者 FIFO——批量排空不重排、不丢条
    #[test]
    fn drain_preserves_fifo_order() {
        let a = EventArtery::new();
        for i in 0..100u64 {
            a.push(UiEvent::ThreadDied(lt_proto::ThreadDied {
                role: lt_proto::ThreadRole::TlWorker,
                detail: format!("{i}"),
                restarted: false,
            }));
        }
        let mut batch = Vec::new();
        a.drain_one_batch(&mut batch);
        assert_eq!(batch.len(), 100);
        for (i, ev) in batch.iter().enumerate() {
            let UiEvent::ThreadDied(d) = ev else {
                panic!("序破坏：第 {i} 条类型不符");
            };
            assert_eq!(d.detail, format!("{i}"), "FIFO 序破坏于 {i}");
        }
    }

    /// 批量上限：BATCH_MAX+50 条 → 第一批量 = BATCH_MAX，剩余 != 0
    #[test]
    fn batch_capped_at_max() {
        let a = EventArtery::new();
        for i in 0..(BATCH_MAX + 50) {
            a.push(UiEvent::ThreadDied(lt_proto::ThreadDied {
                role: lt_proto::ThreadRole::TlWorker,
                detail: format!("{i}"),
                restarted: false,
            }));
        }
        let mut batch = Vec::new();
        a.drain_one_batch(&mut batch);
        assert_eq!(batch.len(), BATCH_MAX, "单批不得超过上限");
        let mut rest = Vec::new();
        a.drain_one_batch(&mut rest);
        assert_eq!(rest.len(), 50, "取不尽留待下次");
    }

    /// 满丢最旧：cap 4096 灌 5000 → 保留最新 4096；丢弃计数 904
    #[test]
    fn drop_oldest_on_overflow() {
        let a = EventArtery::new();
        for i in 0..5000u64 {
            a.push(UiEvent::ThreadDied(lt_proto::ThreadDied {
                role: lt_proto::ThreadRole::TlWorker,
                detail: format!("{i}"),
                restarted: false,
            }));
        }
        assert_eq!(a.q.dropped_count(), 904);
        // 排空验证：剩余 = cap；队首 = 5000-4096
        let mut batch = Vec::new();
        let mut drained = Vec::new();
        loop {
            a.drain_one_batch(&mut batch);
            if batch.is_empty() {
                break;
            }
            drained.extend(batch.drain(..));
        }
        assert_eq!(drained.len(), ARTERY_CAP);
        let UiEvent::ThreadDied(d) = &drained[0] else {
            panic!();
        };
        assert_eq!(d.detail, (5000 - ARTERY_CAP as u64).to_string());
    }

    /// UiMsg::Events 批量变体可携带（契约形态钉死；桥线程 send 的载荷形状）
    #[test]
    fn batch_wraps_events_variant() {
        let evs = vec![
            UiEvent::Download(lt_proto::DownloadEvent {
                repo: "r".into(),
                file: "f".into(),
                index: 1,
                count: 1,
                done: 1,
                total: Some(2),
                phase: lt_proto::DownloadPhase::Progress,
            }),
            UiEvent::Bench(lt_proto::BenchEvent::Finished {
                ok: true,
                elapsed_ms: 42,
            }),
        ];
        let msg = UiMsg::Events(evs);
        let UiMsg::Events(inner) = msg else {
            panic!("Events 变体必须在");
        };
        assert_eq!(inner.len(), 2);
        // AppCommand 独立变体（不再骑事件）
        assert!(matches!(UiMsg::AppCommand(AppCommand::Quit), UiMsg::AppCommand(_)));
    }
}
