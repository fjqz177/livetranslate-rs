//! 事件动脉的代理桥侧（架构 2.0 W2/W3，docs/archive/architecture-v2.md §3.2.2）。
//!
//! 队列本体（`BoundedDropQueue<UiEvent>` 封装）在
//! `lt_orchestrator::event_artery`——生产端（管道/下载/监督器/日志桥）全部
//! 在该 crate 的线程域；本文件只做唯一需要 winit 的一段：桥线程从动脉取
//! 一批（≤BATCH_MAX 条）→ `UiMsg::Events` 一次投递（INV1 回流唯一通路）。
//!
//! 桥线程出生 = Supervisor（W2/INV3：`lt-artery-bridge`，Always 重生——
//! 工厂构造干净循环，INV5；停机由 `stop` 置位后 join，见 main 停机序）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use winit::event_loop::EventLoopProxy;

/// 桥线程轮询戳脚（停机关：50ms 内响应；有事件时动脉 push 的 notify 即时唤醒）
const BRIDGE_POLL: Duration = Duration::from_millis(50);

/// 经监督器出生桥线程（W2/INV3；工厂构造干净循环——drain_batch 无残留态）
pub fn spawn_bridge(
    artery: std::sync::Arc<lt_orchestrator::EventArtery>,
    sup: &lt_orchestrator::Supervisor,
    proxy: EventLoopProxy<lt_proto::UiMsg>,
    stop: Arc<AtomicBool>,
) {
    let arte = artery.clone();
    let proxy = proxy.clone();
    let stop = stop.clone();
    sup.spawn(
        lt_proto::ThreadRole::ArteryBridge,
        "lt-artery-bridge",
        // E4/D-80：唯一保持 Always 的常驻线程——UI 活性本身死透 = 界面全死，
        // 宁可无限重生；其循环体仅 pop/send/take 三个无 panic 源操作
        //（capture/ASR/翻译池/日志桥已迁 Policy::backoff() 指数退避 + 超限放弃）
        lt_orchestrator::Policy::Always,
        move || {
            let arte = arte.clone();
            let proxy = proxy.clone();
            let stop = stop.clone();
            let mut batch = Vec::with_capacity(lt_orchestrator::event_artery::BATCH_MAX);
            Box::new(move || loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if !arte.drain_batch(&mut batch, BRIDGE_POLL) {
                    continue;
                }
                if proxy
                    .send_event(lt_proto::UiMsg::Events(std::mem::take(&mut batch)))
                    .is_err()
                {
                    // 事件循环已退出（停机期）——静默收尾，不再空转
                    return;
                }
            })
        },
    );
}
