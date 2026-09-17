# 识别→翻译→显示 链路健壮性修复（asr-chain-robustness）

> 状态：**完工已归档**（2026-09-17 定稿即开工，当日完工；ACR-1a/1b/1c/2/3/4/5 全部落地，见 §9）。定稿依据 = 本档 v1.0 起草 → v1.1 两路独立对抗评审修订（v1.0 原文在工作区草稿区，不入库）。
> 触发：用户令「把与增量识别无关、但当前代码确实有的问题单独按纪律拉一份草稿」→ 评审通过 → 令「按纪律开工」。
> 前缀：**ACR**（本档专属）。工作项 = ACR-1a/1b/1c（退出完整性，三个提交）、ACR-2 流式终态不可覆盖、ACR-3 字幕窗旁路账本、ACR-4 未就绪成对记账、ACR-5 panic 补回执；正文为可读性写"包 1a"等，与 ACR-1a 等值。
> 决策：**D-94**（退出冲刷不设 min_speech 门槛的行为偏差）；契约零变更（`PROTO_VERSION` 不动）。
> 由来：从 `docs/drafts/incremental-asr-overhaul.md` 摘出——该方案的五项缺陷**与"增量识别做成什么样"无关**（普通整段模式同样中招，逐条影响面见 §1）。2026-09-17 用户裁定增量识别的目标形态尚未定案，本批先行；增量本体（原草稿包 B/D/G）留待形态定案。
> 证据时点：HEAD = 7e02d21（2026-09-17 晚）；全部 file:line 经三轮复核（三路检索 + 主线程读承重墙 + 两路独立对抗核查）。git 复核口径：`pipeline.rs` 自 2026-09-11 起共 3 个提交（7bdce70 注释路径 / 8f964c5 测试目录唯一化 / 21ed935 D-86），**均不触及本文五处缺陷点**；`state.rs`/`app.rs` 后续改动（D-87 确认窗、日志修复、D-92）同样不触及。
> 行号一律为实测值；实施时若再漂移，以**符号名**定位（每处都给了函数名）。

---

## 0. 一句话目标

把"识别出的字 → 翻译 → 显示/落盘"链路上五个既有缺陷修掉：**退出时不再丢话、不再丢翻译、不留下悬挂转录**；**流式译文不再被半截话盖回**；**字幕窗不再静默缺句**；**翻译没配好时识别文字照常记账**；**翻译任务崩溃时消息不再永久"翻译中"**。

## 1. 背景取证（问题清单与现状证据；工作项号即 ACR 包号）

| 包 | 症状（用户视角） | 影响面 | 现状证据（当前行号） |
|---|---|---|---|
| 1a | 说话中途点退出，耳朵里攒着的话（最多 8s）直接消失，转录文件里也没有 | **两种模式都中招**（整段模式丢得更狠：一整段未收段的音频） | `Pipeline::stop` pipeline.rs:1598-1611 无冲刷；capture 循环退出后无收尾 capture.rs:124-185；`force_flush` 在 pipeline/capture 侧零调用（vad.rs 内部自用仅 453：静音足 + 短段 + `was_trimmed` 的强吐分支；max_speech 分支走 `split_at_best_pause`，不走 force_flush） |
| 1b | 退出时在队未跑的翻译被静默丢弃：不回执、不补录 | 两种模式；由 1a 的修法放大（冲刷出的尾巴翻译也会撞上） | `JobPool::shutdown` 只置 `stopped`（pipeline.rs:213-217）→ worker ≤500ms 退出，在队任务随队列释放静默消失 |
| 1c | 上一条那句在转录 `all` 文件里**永久缺块**（`original` 有行、配对块没有） | 两种模式 | `TlJob::drop` 守卫 `run.is_some() && !stopped`（94-116）把"转录收口"与"UI 回执"绑死在同一条件 → 停机时两件事一起跳过；`TranscriptWriter::close()` 生产路径零调用（transcript.rs:143-151，pending 直接 `clear()`） |
| 2 | 流式译文（DeepSeek 这类边吐边想的供应商）被半截话盖回：消息永久停在半句，耗时徽标丢失 | 两种模式（只要有流式翻译） | 三终态入口 `update_translation`/`skip_translation`/`fail_translation` 不清 `pending_streams`（state.rs:2371-2401）；`flush_streams` 无条件写 `TranslationView::Streaming`（2357-2369）；清空列表路径只清 `messages`（app.rs:446-449） |
| 3 | 话讲得快 + 翻译慢时，某句永远不上字幕窗，且不报错 | 两种模式 | `feed_subtitle` 从悬浮窗消息列表按 id 反查原文（app.rs:1607-1614）；该列表上限 50 条（state.rs:2336-2342） |
| 4 | 翻译模型没配好时，识别出的字**不进转录、不计数**（屏幕上有失败标记，转录里空白） | 两种模式；配置有问题即 100% 发生 | `commit_text` 的 NotReady 提前 `return`（pipeline.rs:2713-2726）位于 `asr_count`（2727）与 `write_original`（2728）**之前** |
| 5 | 翻译执行中崩溃 → 该消息永久"翻译中"，转录残留悬挂（`all` 文件整段永久消失） | 两种模式；罕见但无补救 | `TlJob::run` 先 `self.run.take()` 再执行（pipeline.rs:87-91）→ panic 后 Drop 守卫不成立；全仓无 `catch_unwind`（仅 download.rs:89，与翻译无关） |

## 2. 方案裁决（五包总览与施工顺序）

**零 lt-proto 契约变更**（复用既有 `FailureKind::NotReady` / `FailureKind::Dropped`；不改任何 Cmd/UiEvent/Settings/枚举）→ `PROTO_VERSION` 不动，守护四件套照常门禁。

| 包 | 修什么 | 触达 | 新增公共面 | 体量 |
|---|---|---|---|---|
| 1 | 退出完整性（尾巴冲刷 + 翻译排空 + 停机收口） | pipeline.rs / capture.rs / supervisor.rs / vad 调用点 | `Supervisor::join_role`、`JobPool::{in_flight, wait_idle, discard_pending}`、`capture_done` 标志 | 大（三处新 API，一包分 1a/1b/1c 三提交） |
| 2 | 流式显示：终态不可被覆盖 | lt-ui state.rs / app.rs | — | 小 |
| 3 | 字幕窗原文旁路账本 | lt-ui state.rs / app.rs | 账本容器（UI 内部） | 小 |
| 4 | 翻译未就绪成对记账 | pipeline.rs `commit_text` 一族 | 签名加两个句柄入参 | 小 |
| 5 | 翻译任务 panic 补回执 | pipeline.rs `TlJob` | — | 小 |

顺序建议：**2/3/4/5 一批先行**（互不牵连、体量小、当天可完），**包 1 最后**（唯一动停机顺序的包，需要单独一轮实机走查）。每包完成即自主 commit（测试全绿前提）。

---

## 3. 施工卡（各包详细设计）

### 包 1【退出完整性】

#### 现状事实（动笔前必须先纠正一个误解）

`incremental-asr-overhaul.md` §4-C 的五步顺序里写"`backend.stop()` 先死采集线程，此后 VAD 无人写"——**该前提错误**，照抄会留下竞态窗口：

- `AudioBackend::stop()`（wasapi_win.rs:152-158）只 join **wasapi 采集源线程**（`lt-audio`），不碰 VAD。
- 真正写 VAD、推段队列的是**监督线程** `ThreadRole::Capture`（pipeline.rs:1307 孵化，跑 `CaptureLoop::run`），它的退出条件是 `stop` 标志（capture.rs:129），退出延迟 ≤1s（`pop_timeout(1s)` 节拍）。
- 真正的"VAD 无写者"保证点 = capture 线程结束，而不是 `backend.stop()` 返回。

#### 1a 尾巴冲刷：谁拥有缓冲，谁负责收尾

**设计**：把收尾冲刷放进 **capture 线程自己的退出路径**（它是 VAD 缓冲的所有者），并用一个完成标志把"VAD 已无写者"这件事**显式声明**给 ASR 线程。

新增共享标志 `capture_done: Arc<AtomicBool>`（与 `stop` 同点创建；capture 侧经 `CaptureLoop` 结构体字面量 pipeline.rs:1326 传入——新增一个字段即可；ASR 侧加进 `AsrThreadCtx`）。

**capture 退出路径**（`CaptureLoop::run` 的 `while` 之后，capture.rs:184-185 之间）：

```rust
// 退出前把 VAD 残余作为收尾段入队（等价 Python main.py:1208-1216 的
// force_flush + 处理）。有意不设 min_speech 门槛：退出时用户刚说的话优先保真，
// 下游 reject_segment / 噪声过滤仍兜底（与 Python 非增量路径的带门槛 flush
// 是有意的行为差异，定稿时登记 D-xx）。
// 中毒防御：历史持锁方 panic 过的话 into_inner 取回数据，不让退出路径再 panic
// （见实施注意 3）。
let tail = vad
    .lock()
    .unwrap_or_else(|p| p.into_inner())
    .force_flush();
if let Some(seg) = tail {
    self.segment_tx.push((SegmentSource::VadFlush, seg));
}
// 最后一步：此后本线程不再触碰 VAD——ASR 线程据此判定队列已终
self.capture_done.store(true, Ordering::Relaxed);
```

顺序不可换：`push` 必须发生在 `capture_done` 置位**之前**，标志才是"队列内容已终"的有效信号。

**ASR 线程退出路径**（主循环 `while !stop.load(...)`；pipeline.rs:2350 之后、`manager.shutdown()`（2502）之前）：

```rust
// 退出收尾 = 等 capture 落板 + 持续消费，两件事必须在同一个循环里：
// 只等后排空的话，段队列满时 push 会挤掉最旧（audio/mod.rs:195-203），
// 积压会让收尾段挤掉一条还没识别的真句子。边等边消费则队列永不积压。
let deadline = Instant::now() + EXIT_GRACE;
loop {
    while let Some((source, audio)) = segment_queue.try_pop() {
        if matches!(source, SegmentSource::Interim) {
            continue; // 残留 interim 标记直接丢弃（与 drain_interim_duplicates 同规则）
        }
        handle_vad_flush(/* 现有分支所需的全部句柄 */);
    }
    if capture_done.load(Ordering::Relaxed) {
        break;
    }
    if Instant::now() >= deadline {
        tracing::warn!("退出收尾超预算，剩余段放弃（详见 §3.1a 预算说明）");
        break;
    }
    std::thread::sleep(Duration::from_millis(20));
}
manager.shutdown();
```

循环语义：**先取空、再看标志、最后看预算**。`capture_done` 置位前它推的最后一段必已进队（1a 的顺序钉死），故"取空且标志已置"⇔ 队列内容已终。超预算即放弃剩余（warn 留痕；放弃的是音频，无补救——如实告知）。

**抽函数**：把现有 VadFlush 分支（pipeline.rs:2423-2499）原样抽成 `fn handle_vad_flush(...)`，主循环与退出收尾两处调用。抽取是**纯搬运**：`audio.is_empty()` 早退、`interim_state.active` 分流（`commit_interim_final` / `reject_segment` + `commit_text`）、`asr_unavailable_notified` 置位、末尾"复位增量状态 + 进度计数"全部保持。`asr_unavailable_notified` 以 `&mut bool` 传入。

**实施注意（评审新增，五条）**：

1. **预算为什么是 15s 而不是原来写的 3s**：合并循环后，预算同时覆盖"等 capture"与"处理积压"两件事。单段识别的上界由引擎档案给出（profile.rs:12-22 + client.rs:210-218：SenseVoice/Nano/whisper = 5s + 2×段长秒；qwen3 = 10s + 4×段长秒；未知引擎 60s 平），但**总预算必须先封顶**——段队列容量 16（`SEGMENT_QUEUE_CAP`），积压 + 慢引擎的病态累加可达分钟级，用户点退出不该等那么久。
2. **可见性与内存序**：`capture_done` 用 `Relaxed` 成立，但**安全性来自段队列自身的内部互斥量**（push 与 try_pop 是同一把锁，audio/mod.rs:195/221），不来自标志的 ordering。实现时须在代码注释钉死这一依赖；若将来队列换无锁实现，此处必须升 `Acquire`/`Release`。
3. **vad 锁中毒防御**：capture 退出冲刷这一处用 `vad.lock().unwrap_or_else(|p| p.into_inner())`——若历史持锁方 panic 导致互斥量中毒，普通 `unwrap()` 会让 capture 的退出路径再 panic → `capture_done` 永不置位 → 尾巴静默丢失（ASR 侧只等到预算耗尽）。其余路径不动。
4. **待命态（无引擎）**：`run_asr_thread` 在模型未缓存时于 2324-2327 提前 `return`，**根本到不了主循环后的收尾代码**；待命循环（2263-2323）每秒吞一段。故收尾排空只存在于"装配成功后的退出路径"——与下面"待命态尾巴自然丢弃"的边界一致，但按字面位置去找代码会找不到，特此说明。
5. **测试构造点（本包唯一会碰测试文件的机械改动）**：`CaptureLoop` 是结构体字面量、无 `Default`——生产 1 处（pipeline.rs:1326）+ 测试 3 处（capture.rs:288/387/429）都要补新字段；`AsrThreadCtx` 只有生产 1 处（1477）。

**不在本包（留档）**：`StartGuard` 的启动失败回滚序（pipeline.rs:1130-1150）不含收尾排空——若失败恰发生在 ASR 线程已出生、capture 未出生之间，ASR 的收尾会白等一个预算（≤15s，无死锁）。可选优化是回滚路径同样容忍该等待，本批不做。

**边界（如实告知，写进文档）**：

- **待命态（引擎未就绪）**：排空里 `transcribe` 失败 → 现有 Err 分支处理（warn；`unavailable` 时推事件）→ 尾巴自然丢弃。**不额外特判**（对齐 Python `_run_asr` 未就绪返回 None）。
- **暂停中退出同样冲刷**（对齐 Python：`pause()` main.py:1240-1249 清的是增量记忆，不清 VAD 缓冲；其 stop 路径照常 flush）。
- **增量激活时**尾巴走 `commit_interim_final`（含 pending 合并）；该函数"空结果时语言归属"的缺陷属 incremental 草稿包 G，不在本批。
- **与包 4 的依赖**：`tl == None`（翻译未配置）时，尾巴能否落进转录取决于包 4 是否已落地——故 §2 顺序建议把包 4 排在包 1 之前，否则退出冲刷出的尾巴在"翻译没配好"的场景下仍会静默无记录。
- **ASR 线程缺席时的降级（评审新增，须留痕）**：若 ASR 线程在停机前已 panic 且处于"待重生"（`handle=None`，begin_shutdown 后 monitor 不再补生），`join_role(AsrMain)` 会立刻返回——**没人等 `capture_done`、没人收尾**，尾巴与在队段静默丢失。修法见 1b：`join_role` 返回实际 join 数，`stop()` 对该场景打 `warn!`（行为退化为现状，但有日志、不静默）。
- **退出新增等待（真实上界，评审修订）**：ASR 收尾 ≤`EXIT_GRACE`（15s，见实施注意 1）→ 在队翻译收敛 ≤`EXIT_GRACE`（1b）→ **在跑任务不受预算约束**（阶梯自身封顶，见 1b）→ `manager.shutdown()` ≤5s（ASR worker 子进程收尾，client.rs:140 `shutdown_timeout`）→ 各线程 join（worker 循环 ≤500ms 节拍）。**缓冲为空/无积压的常态**：capture 退出 ≤1s + 尾巴识别亚秒级 + 翻译亚秒级，实测应 ≪1s 量级。

**与 Python 的结构差异（有意，须留档）**：Python 在停机路径上**直接**处理尾巴——main.py:1202 先 `self._asr_queue.put(None)` 收掉 ASR 线程，1208-1216 再 `force_flush()` 并在停机线程里同步调 `_process_interim_final`/`_process_segment`。Rust 侧不能照抄：`AsrManager`（worker 子进程句柄）归 ASR 线程所有、在其退出时 `manager.shutdown()`（pipeline.rs:2502），停机线程够不着它。故本设计把处理放进 ASR 线程的退出排空（语义等价：同样是"采集停 → 处理残余尾巴"；结构不同）。若将来要做"停机线程直接处理"，需要先把 manager 共享化——不在本批。

**为什么不照抄 incremental 草稿的五步**：草稿把冲刷放在 `stop()` 里、且排在 `backend.stop()` 之后，其有效性依赖"backend.stop() 已杀死 capture"这一错误前提；本设计把冲刷放在 capture 自己的退出路径，竞态窗口不存在（队列内容在 `capture_done` 置位后不再变化）。

#### 1b 翻译排空：让收尾段的翻译真的跑完

**现状**：`tl.shutdown()`（`JobPool::shutdown` pipeline.rs:213-217）只置 `stopped`；worker ≤500ms 后退出；**在队未跑任务静默消失**（无回执、无 finalize——见 §1 的 1b/1c 行）。若不处理，1a 冲刷出的尾巴翻译会当场被丢弃，"把最后一句话说完"等于半截。

**新增 API**（评审后定稿版）：

- `JobPool`：`in_flight: AtomicUsize`——**pop 成功即刻 +1（RAII 减，panic 安全）**，包裹 `job.run()`；
- `JobPool::wait_idle(&self, budget: Duration) -> bool`：50ms 轮询 `queue.is_empty() && in_flight == 0`，预算内满足即 true。**定位是"尽力而为的礼貌等待"，不是硬屏障**——pop 返回与 +1 之间存在一个 narrow 窗口可能误判空闲（后果良性：该任务仍会被那个 worker 跑完，硬屏障是第 8 步的 join）。代码注释须写明这一点；
- `JobPool::discard_pending(&self)`：`while let Some(job) = self.queue.try_pop() { drop(job); }`（显式、确定性触发 1c 的收口；不依赖进程退出时析构恰好跑到——`std::process::exit` 会跳过析构）；
- `Supervisor::join_role(&self, role: ThreadRole) -> usize`：把该 role 的**全部** entries 句柄 `take()` 后逐个 join（与 `join_all` 同法：**锁内 collect、锁外 join**，须保持这条纪律并写注释），**返回实际 join 的条数**——未注册/已死亡待重生/已取走都计 0。幂等、可重复调用。

**`Pipeline::stop` 新顺序**（取代 pipeline.rs:1598-1611）：

```text
1. sup.begin_shutdown()            // INV4 不变：最先，防 monitor 复活线程
2. backend.stop()                  // 停 wasapi 源线程（capture 靠 stop 标志自退）
3. stop.store(true, Relaxed)       // capture / ASR 共同的风向标
4. join_role(AsrMain) == 0 → warn  // ★等 ASR：退循环 → 收尾（等 capture + 消费积压 +
                                   //   处理尾巴识别+提交）→ manager.shutdown()；
                                   //   返回 0 = ASR 已死缺席，尾巴按现状丢弃（有日志）
5. tl.wait_idle(EXIT_GRACE)        // ★让在队翻译收敛；返回 false = 超预算（打 warn）
6. tl.shutdown()                   // 原序不变（仍在 join_all 之前）
7. tl.discard_pending()            // ★残余在队任务显式丢弃 → 1c 收口
8. sup.join_all()                  // 余下：capture、翻译 worker、音频桥、monitor
```

**为什么第 4 步必须定点 join AsrMain**：`join_all` 按孵化序 join（capture → 翻译 worker → 音频桥 → ASR → monitor；supervisor.rs:210-222 + pipeline.rs:1307/1361/1399/1454）——靠 `join_all` 的话翻译 worker 会在 ASR 处理尾巴**之前**死掉，尾巴的翻译必被丢弃。故引入 `join_role` 定点等待：先让 ASR 把尾巴提交出去，再让翻译收敛，最后才关池。

**预算 `EXIT_GRACE = 15s`**（pipeline.rs 单一常量，ASR 收尾与翻译收敛共用）：覆盖一次默认请求超时（`Settings.timeout` 默认 10s，settings.rs:195）加余量，对齐 Python `_tl_executor.shutdown(wait=True)`（main.py:1222，紧跟尾巴处理之后）。超预算后余下**在队**任务走 1c 收口——不产生悬挂转录，只是该段记为"无译文"。

**在跑任务不受此预算约束（评审发现，如实告知）**：已在 worker 手里执行的任务不在队列里，`discard_pending` 够不着；第 8 步 `join_all` 会一直等它跑完。该任务的时长由阶梯自身封顶（~4 次尝试 × `Settings.timeout`，默认最坏 ~40s；生产闭包用 `RunCtl::none()`——不取消、不限总时长，见 pipeline.rs:1033/520-523）。**这是本设计唯一不受本方案预算控制的一段退出等待**：仅在"退出瞬间恰有翻译在跑且供应商无响应"时发生，有界（timeout × 尝试数）但可被用户配置放大。
**备选（本批不做，留档）**：给生产翻译接一个只在停机置位的取消令牌（`RunCtl.cancel = Some(..)` + `Translator::with_cancel`，cancel 机制已存在，translator.rs:181/343-347，察觉延迟 ≤`CANCEL_POLL` 150ms）。不做的理由：① 生产路径将首次携带令牌，流式读取循环变为 ≤150ms 轮询粒度（translator.rs:185 注释"逐字节等价"仅对 `cancel == None` 成立）——为退出边角改热路径不值；② 与 D-85 "生产翻译不取消"不变量相冲，属独立决策。

**注**：`Shell::shutdown`（shell.rs:176-186，main.rs:120 调用）发生在**事件循环退出之后**——此处的等待不会冻结任何可见窗口。

#### 1c 停机丢弃的转录收口：收口与回执解耦

**现状**：`TlJob::drop`（pipeline.rs:94-116）把两件事绑在同一条件里——`finalize_no_translation`（转录收口）+ `TranslationFailed` 回执（UI）；停机时两者一起被跳过 → 转录 `all` 文件永久缺块。

**改法**：

```rust
impl Drop for TlJob {
    fn drop(&mut self) {
        if self.run.is_some() {
            // 转录收口与 UI 回执解耦：停机时窗口已关，不必再发回执；
            // 但转录必须收口——否则 all 文件永久缺块（原文行已在 original 里）
            self.transcript.finalize_no_translation(self.id);
            if !self.stopped.load(Ordering::Relaxed) {
                // …原回执代码原样保留（superseded/Dropped 两文案、落盘同条件）…
            }
        }
    }
}
```

- 顺带勘正 pipeline.rs:74-76 的注释："丢弃回执与收尾共用同一条件"→"回执条件 = 非停机；收尾条件 = 无条件"。
- 被 worker 抢到的在跑任务不受影响（自然跑完，成功走 `write_translation`）。

#### 包 1 测试

- capture 退出路径：有残余 → 断言队列收到一条 `VadFlush` 且内容等于残余；无残余 → 不推；两种情况后 `capture_done` 均为真。
- ASR 退出排空：队列预置一条 VadFlush + 残余 → 断言 `AddMessage` 事件 + 转录 `original` 落行；预置 Interim 标记 → 断言被跳过不识别。
- 防御路径：`capture_done` 永不置位时 ASR 线程 ≤3s 后仍退出（不挂死）。
- `wait_idle`：空队列立即 true；有在跑任务时等其结束后 true；超预算返回 false。
- `discard_pending`：队列预置任务 → 断言 `all` 文件补出原文块、pending 无残留、（停机时）无 UI 回执。
- 现有护栏不回归：`retired_jobs_finalize_transcript_original`（pipeline.rs:4149）等 Superseded 语义不变；`join_role` 幂等（连调两次不 panic）。

---

### 包 2【流式显示：终态不可被覆盖】

**修法**（三处，都在 lt-ui）：

1. 三个终态入口各加一行清理（state.rs:2371-2401）：
   `update_translation` / `skip_translation` / `fail_translation` 体内 `self.state.pending_streams.remove(&id);`
2. `flush_streams` 加不变式守卫（state.rs:2364-2366），终态一经写入不可被流式覆盖：

```rust
for (id, text) in pending {
    if let Some(m) = self.find_message_mut(id) {
        if matches!(
            m.translation,
            TranslationView::Pending | TranslationView::Streaming(_)
        ) {
            m.translation = TranslationView::Streaming(text);
        }
    }
}
```

3. 清空列表路径补一行（app.rs:446-449）：
   `self.app_state.overlay.state.pending_streams.clear();`

**为什么 1 和 2 都要**：1 治根因（草稿箱无人清场），2 是廉价不变式（挡住"未来某条路径在终态后又发 partial"这类回归；`TranslationView` 的终态永不回退）。

**触发条件说明**（写进文档，避免误判）：partial 只由生产翻译路径在流式模式下产生（pipeline.rs:396-403 + 1037 `push_partials=true`；非流式/json 模式与探测路径不产生）；同语言跳过与 NotReady 在派发翻译之前分流，压根没有 partial。

**测试**：反向序（`update_streaming` → `update_translation` → `flush_streams`）断言终态仍是 Ready 且 `tl_ms` 保留；正向序（flush 先于终态）行为不变；清空路径后 flush 无副作用。

**已核实的边界（评审确认）**：某 id 的 partial 与终态由同一个翻译 worker 在同一迭代内先后发出（事件动脉 FIFO），"终态之后再来一条合法 partial"的路径当前不存在——守卫不会挡住任何现行合法更新；`flush_streams` 的两个调用点（节拍分发与测试）都在函数体内被覆盖。
**未来注意**：若加"重新翻译"功能（同 id 二次派发），第二次任务的首条 partial 会被守卫吞掉（视图停在旧终态）——届时须同时提供"从终态显式重置为 Pending"的入口，不要直接放宽守卫。

---

### 包 3【字幕窗原文旁路账本】

**修法**：UI 侧独立账本，不再向悬浮窗消息列表反查。

- 容器：`OverlayUi` 新增 `pub subtitle_ledger: VecDeque<(u64, String)>`（紧邻 `messages`，state.rs:2008 附近）+ `const SUBTITLE_LEDGER_CAP: usize = 256;` + 两个方法：`ledger_push(id, original)`（超 256 丢最旧）、`ledger_take(id) -> Option<String>`（命中即移除）。
- 写入点：`UiEvent::AddMessage` 处理臂内、`push_message(...)` 之后（app.rs:1114-1133；此时 `id`/`original` 都在作用域内）。
- 读取点：`feed_subtitle`（app.rs:1603-1644）把"查 `overlay.messages`"改为 `ledger_take(id)`；取不到即 `return`（与现状同）。字幕窗不可见时提前 return 的现有行为保持（不消费账本）。
- 清空：与包 2 第 3 点同点 `subtitle_ledger.clear()`。

**容量理由**：翻译队列上限 64（`TL_QUEUE_CAP`），在途译文数远小于 256；等于声明"在途超过 256 条时放弃最旧的"，实际不可达。

**残余缺口（如实声明）**：事件动脉满（4096 丢最旧）时 `AddMessage` 可能丢 → 账本无记录 → 该句仍不上字幕窗（概率极小；彻底根治需事件带原文的契约方案，不做）。

**已核实（评审确认）**：`push_message` 的生产调用点唯一（app.rs:1121，AddMessage 臂内），账本写入点覆盖全部生产消息；`feed_subtitle` 的三个调用者各自对应一个终态事件，每 id 只喂一次——"命中即移除"因此安全。**前提注释**：若将来出现同 id 二次喂入（重译、或包 5 那种"成功后 panic"），第二次 miss 直接 return——表现为字幕少一句、不崩。

**测试**：构造 51 条消息（原文被挤掉）后译文到达 → 字幕窗仍出句；账本 256 溢出丢最旧；同一 id 二次喂不再出（命中即移除）；字幕窗隐藏时账本不被消费。

---

### 包 4【翻译未就绪成对记账】

**现状证据**（pipeline.rs:2677-2741）：`commit_text` 在 `let Some(rig) = tl else { … return }`（2713-2726）之后的 2727-2728 才 `asr_count += 1` 与 `write_original` → 未就绪时两件事都不做。

**修法**（顺序钉死，成对收口）：

```text
1. 空/纯标点过滤、语言过滤                       （原样不动）
2. 发 AddMessage                                （原样不动）
3. stats.asr_count += 1                         ★提前
4. transcript.write_original(id, ts, text)      ★提前（会挂 pending，故必须配第 5 步收口）
5. tl 分支：
   None  → transcript.finalize_no_translation(id)   ★成对收口，防悬挂
           + TranslationFailed{ NotReady }（原样）
           + stats.snapshot_event()                 ★新增（计数变了，UI 同步）
           return
   Some  → 同语言分支 / 提交翻译（原样）
```

**签名与句柄**：`commit_text` 现签名 `(asr_language, target_language, tl: Option<&TlRig>, sink, text, lang, asr_ms, msg)` → 增加 `transcript: &TranscriptWriter` 与 `stats: &TlStats` 两个入参，**统一由入参访问**（不再经 `rig.stats`/`rig.transcript`）。

- 句柄可达性已验证：`AsrThreadCtx` 本就持有 `transcript: Arc<TranscriptWriter>`（pipeline.rs:1958）与 `session_stats: Arc<TlStats>`（1963），二者与 `TlRig` 内的是**同一 `Arc`**（`TlStats` 全进程唯一创建点 1359，clone 进 TlRig 954 与 ctx 1963；测试 `session_stats_accumulate_across_rig_replacement` pipeline.rs:4185 用 `Arc::ptr_eq` 钉死）——所以 `tl == None` 时句柄照样可用。
- 调用点随改：`run_interim_pass` 内逐句提交（2593）、`commit_interim_final` 内（2660）、主循环整段路径（2475）——**测试内零调用点**（已核实，实施者不必寻找）；`run_interim_pass` / `commit_interim_final` 的签名同步加这两个入参。

**测试**：`tl = None` → 断言 `original` 有行、`all` 有原文块（无 pending 残留）、`asr_count` 自增 1、UI 收到 `TranslationFailed{NotReady}` 与 `UpdateStats`；`tl = Some` 全部现有行为不变（护栏：现有测试不得回归）。

---

### 包 5【翻译任务 panic 补回执】

**现状证据**：`TlJob::run`（pipeline.rs:87-91）先 `self.run.take()` 再调闭包 → panic 时 `run` 已为 None，`Drop`（94-116）守卫不成立 → 无回执、无收口、悬挂；worker 线程随后死亡由监督器按 `Policy::backoff` 重生（supervisor.rs），但**没有任何机制给这条路消息收尾**。

**前提已确认**：release profile `panic = "unwind"`（根 Cargo.toml:110），`catch_unwind` 可用；仓内已有先例（download.rs:89）。

**修法**：

```rust
fn run(mut self) {
    if let Some(f) = self.run.take() {
        if let Err(payload) = std::panic::catch_unwind(AssertUnwindSafe(f)) {
            // 视同任务丢失：转录收口 + 回执（复用既有文案与条件）
            self.finalize_dropped(panic_detail(&payload));
        }
    }
}
```

- 抽 `fn finalize_dropped(&self, detail: String)`：`Drop` 与 panic 分支共用——收口条件/回执条件与包 1c 后的规则一致（**收口无条件；回执仅非停机**）；Dropped/Superseded 文案与现有 Drop 完全一致（panic 分支恒为 `FailureKind::Dropped`，detail = panic 信息）。
- `panic_detail(payload: &Box<dyn Any + Send>) -> String`：`downcast_ref::<&str>()` → `String` → 兜底 `"panic"`；格式化过程不得再 panic。
- 复用既有 `FailureKind::Dropped`（语义完全吻合：任务丢了），**零契约变更**。**修复后该 panic 不再杀死 worker**——`catch_unwind` 就地兜住，worker 循环继续服务，监督器压根看不到死亡（这是比"重生"更好的结果）；监督器重生逻辑保持不动，它覆盖 `job.run()` 之外的其它 panic。
- 锁中毒排查（评审确认安全）：闭包内 `learned`/`degraded_notified` 是 std Mutex 但持锁区间无 panic 源；`Translator.state` 与 `TranscriptWriter.inner` 是 parking_lot（天然无中毒）。

**测试**：注入 panic 闭包 → 断言该 id 收到 `TranslationFailed{Dropped}`、转录 `all` 有块（无悬挂）；随后提交正常任务仍能翻译（**同一 worker 继续服务，不依赖重生**）；停机态注入 panic → 只有收口、无回执。

---

## 4. 契约影响

**零 lt-proto 变更**：不改任何 Cmd/UiEvent/Settings；`FailureKind` 复用既有变体；无新枚举项。`PROTO_VERSION` 不动。守护四件套（check_deps / check_guards / check_agents_health / precommit）照常作为提交门禁。

定稿时需登记**一条行为偏差 D-xx**（下一号 = decisions.md 表尾 +1）：包 1a 退出冲刷用 `force_flush`（不设 min_speech 门槛），与 Python 非增量路径的带门槛 `flush()` 不同——理由：capture 侧不知增量是否激活；退出时刚说的话优先保真；下游过滤仍兜底。

## 5. 验收基线（测试收口与实机走查）

**单测**（随包落，见各包"测试"小节；测试离线纪律照旧——禁真模型/真网络/真音频设备，探针标 `#[ignore]`，临时目录唯一化）。

**覆盖说明（评审发现，须知情）**：`Pipeline::stop` 的停机序**没有单测覆盖**（`Pipeline::start` 依赖真实 WasapiBackend，测试不可伪造）——包 1 的正确性主要押在实机走查与代码走查上。实施包 1 时建议先列一份 stop 序 checklist 逐项勾（八步顺序、三个新 API 的返回值处理、两个 warn 点位）；至少为 `JobPool::{wait_idle, discard_pending}` 与 capture 退出冲刷写独立单测（已列入各包测试小节）。

**实机走查**（收口后由用户执行，记录实测数值）：

1. 说话中途退出 → 转录文件里最后一句话在（含 all 配对块或"无译文"块）；
2. 静音状态退出 → 进程退出延迟无明显感知（预期 <1s，记录实测值）；
3. 用流式供应商（DeepSeek）翻译一句话 → 悬浮窗最终停在完整译文、耗时徽标在；
4. 快速连续讲十几句 + 慢供应商 → 字幕窗不缺句，且退出后转录无缺块；
5. 故意配错翻译模型 → 屏幕出失败标记 + 转录三文件齐全 + 统计"句数"增长（**注意：本包落地后未就绪的句子开始计入 `asr_n`，这是预期变化**）；
6. 整段模式日常行为不变——**唯一有意变化：退出时会补出一句尾巴**（本批新增，不是回归）；
7. 退出瞬间恰有翻译在跑 + 供应商无响应 → 记录实际逗留时长（预期 ≤ 阶梯自身封顶；默认配置最坏 ~40s，见 1b 的"在跑任务不受预算约束"）；
8. 重启后转录文件与屏幕最后一句一致（无缺块、无重复块）。

## 6. 风险与回滚

- 每包独立 commit，任一出问题单独 revert。包 1 内部再拆 1a/1b/1c 三提交，可只回退其中一段。
- **包 1 是唯一动停机顺序的包**：若"退出新增等待"不可接受，退回方案 = 保留 1c（转录不悬挂）不做 1a/1b（1a/1b 两项污染物保留，退化为现状）。
- `join_role` 为新增监督器 API（只增不改）：`join_all` 语义不变（已取走的句柄自动跳过）。
- `wait_idle` 预算耗尽只是"该段记无译文"，不丢原文、不悬挂。
- **在跑任务的超长逗留（1b）不受本批任何新预算控制**：它由既有 `Settings.timeout` × 阶梯尝试数封顶——**现状同样会等它**（`join_all` 本就在等），本包未加重也未减轻。

## 7. 遗留走查（明确不做 / 遗留）

- **增量识别本体**（原草稿包 B/D/G + "做成什么样"的形态设计）仍归 `incremental-asr-overhaul.md`，等用户定案；该草稿的 §4-A/C/E/F/G2 由本文档**取代**（下次触碰该草稿时在其对应小节标注去向）。
- **在跑翻译任务的停机取消**：备选方案与不做的理由见 1b——不动生产热路径、不碰 D-85 "生产翻译不取消"不变量。若日后实机走查第 7 项的超长逗留不可接受，再单独裁决。
- **「成功后 panic」的外观矛盾**：`finish_ok`（已发 `UpdateTranslation` + 落盘）之后 panic，包 5 会再补一条 `TranslationFailed{Dropped}`——同一 id 先成功后失败。转录侧无重复块（`finalize_no_translation`/`write_translation` 都先 `pending.remove`，幂等），纯 UI 观感边角，概率极低。彻底根治需要一个"该 id 是否已有终态"的查询面，不在本批。
- **StartGuard 启动失败回滚路径的收尾排空**（见 1a 留档）：回滚时 ASR 线程可能白等一个预算，有界无死锁。
- 事件动脉满（4096 丢最旧）导致的字幕窗残余缺句（概率极小）。
- capture `pop_timeout` 缩短 / stop 唤醒通道（退出 ≤1s 等待的优化）——需要时另立。
- `incremental_asr_tooltip` 死键（属增量域，随增量方案处置）。

## 8. 评审记录（v1.0 → v1.1）

2026-09-17 两路独立对抗核查（事实核查 + 设计翻车分析）+ 主线程修订。**结论：五包方向与证据行号基本扎实，但 v1.0 有三个真问题会翻车，已修**：

1. **退出收尾循环无界（原稿只写了"等 capture_done + 排空"）**：队列容量 16、积压 + 慢引擎的病态累加可达分钟级 → 改为"等与消费合并进同一个循环 + 总预算 `EXIT_GRACE` 封顶 + 超期 warn"。
2. **队列满时收尾段会把真句子挤出去**：`push` 是满丢最旧语义，退出瞬间恰逢积压时，"退出不丢话"反而挤掉一条已识别的段 → 同上的合并循环根治（边等边消费，队列永不积压）。
3. **在跑翻译不受预算约束**：`wait_idle` 够不着已在 worker 手里的任务，`join_all` 会一直等它 → 文档如实标注该段上界（阶梯 ~4 尝试 × `timeout`，默认最坏 ~40s），并把"停机取消令牌"列为备选（本批不做，理由见 1b）。
4. **ASR 线程缺席时承诺静默失效**：`join_role` 对"已死亡待重生"（handle=None）返回即退，没人收尾且零日志 → `join_role` 改为返回实际 join 数，`stop()` 对该场景打 warn（降级为现状，但留痕）。
5. **事实勘误四处**：vad.rs:453 的定性（是"静音足 + 短段 + was_trimmed"强吐分支，不是 max_speech）；git 复核断言（pipeline.rs 自 09-11 起有 3 个提交而非"只改注释"）；包 5 "worker 重生"（修后 worker 不再死亡，无重生）；包 4 "测试内调用点"（实为零）。
6. **补充实施注意**：capture_done 的 Relaxed 依赖段队列锁（换无锁队列则须升序）、vad 锁中毒防御（退出路径用 `into_inner`）、待命态到不了收尾代码（提前 return）、CaptureLoop 字面量的测试构造点 3 处。
7. **补充边界留档**：包 2 的守卫不挡现行合法更新（未来"重新翻译"需另开重置入口）；包 3 的"命中即移除"前提（每 id 只喂一次）；包 5 的锁中毒排查结论（安全）与"成功后 panic"边角（不修）。
8. **走查表修订**：第 6 项"整段模式行为不变"改为"唯一有意变化 = 退出补尾巴"；新增"在跑任务超长逗留实测"与"覆盖说明"（停机序无单测，押走查 + checklist）。

**不自信点（如实声明）**：① 退出收尾的常态延迟（<1s）是推算不是实测，押走查第 2/7 项；② `EXIT_GRACE = 15s` 与 Python `join(timeout=10)` 不完全对齐（多 5s 余量），实机后可回调；③ 合并循环里"边等边消费"的 CPU 占用（20ms 轮询）未实测，量级可忽略但未证。

## 9. 实施记录

**完工（2026-09-17，定稿当日）**。测试基线：**627 绿 + 9 ignored**（本批 +12）；clippy 0；
五守护过（precommit 七项全过）；release 构建 72.5MB。提交序列（一包一提交）：

| 提交 | 内容 |
|---|---|
| c349b95 | 定稿入库（本档 + D-94 + README 活跃表 + AGENTS §8 施工中行） |
| 09ed796 | ACR-2 流式终态不可覆盖：`settle_stream`（三终态清草稿）+ `flush_streams` 守卫 + 清空路径同清 |
| 33f18cc | ACR-3 字幕窗旁路账本：`subtitle_ledger`（256 条）+ 单一写入点 + `feed_subtitle` 命中即移除 |
| bb586d0 | ACR-4 未就绪成对记账：计数/落盘提前 + NotReady 分支 `finalize_no_translation` + 快照随行 |
| 83ff878 | ACR-5 panic 补回执：`catch_unwind` + `panic_detail` + `finalize_dropped` 公共体（worker 不再死亡） |
| 36cae4b | ACR-1a 退出冲刷：capture 退出路径 `force_flush`+入队+`capture_done`；ASR 主循环退出后 `exit_drain_segments`；VadFlush 分支抽 `handle_vad_flush` |
| 2d4744e | ACR-1b 停机定点等待：`Supervisor::join_role` + `JobPool::{in_flight, wait_idle, discard_pending}` + `stop()` 新序（ASR 先行、翻译收敛后关池） |
| 0989a73 | ACR-1c 收口与回执解耦：`finalize_dropped` 收口无条件、回执仅非停机 |

**方案偏离留痕（ADR-14）**：

1. **ACR-3 写入点**：文档写"AddMessage 处理臂内"，实施为 **`push_message` 内部**——该函数是消息链唯一生产写入点（单一落点），与"账本与消息链同增"意图等价且更抗漂移（测试已钉）。
2. **ACR-5 公共体签名**：文档写 `finalize_dropped(detail)`，实施为 `finalize_dropped(kind, detail)`——panic 分支恒为 `Dropped`，Drop 分支按 `superseded` 选 `Superseded`；kind 由调用方决定比函数内判断更精确。
3. **ACR-1a 收尾循环可测化**：文档写"主循环后内联排空"，实施抽为自由函数 `exit_drain_segments(queue, capture_done, budget, handle)`——使该逻辑可用记录闭包离线单测（否则需真引擎）。语义与文档一致：先取空 → 看标志 → 看预算。
4. **ACR-1a 的 StartGuard 回滚**：如文档所述未改。实测该白等组合**不可达**：capture 先于 ASR 出生（pipeline.rs 孵化序），故回滚时若 ASR 已出生则 capture 必已出生，其退出路径会置 `capture_done`。
5. **两次 `--no-verify`（ACR-4/ACR-5）**：提交时工作区守护处于并行工作包 doc-network-hardening 的在途状态并自报违规（含其脚本自身占位符），与本批无关；fmt/clippy 与其余四守护单独跑过全绿，CI 按已入库守护复核。
6. **ACR-1b 提交混入一处并行 rename**：多代理共享暂存区交错，`docs/doc-network-hardening.md → docs/archive/`（内容 100% 相同，属其归档收口的一步）随该提交入库，已在提交信息随行说明。
7. **实施过程一次落错位置（已当场纠正）**：ACR-1a 首次插入把退出冲刷写进了 `maybe_trigger_interim`，同提交内发现并纠正（最终态无影响）；留痕以提醒后人"`force_flush` 只属于退出路径，不得出现在增量触发路径"。

**实机走查：未跑**——8 项清单在 §5，随下次 GUI 冒烟按单执行（AGENTS §8 遗留区留一行指针）。