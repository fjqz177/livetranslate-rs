# 供应商连接测试与热切换加固方案（D-85）

> 定稿 2026-09-10。**本文档即施工依据**：所有决策已在 §3 定案、所有接口已在 §4 给出精确形态，
> 施工按 §5 的提交切分逐步落地，验收按 §6 执行。文档先于实现提交（AGENTS「docs 提交时机」约定）。
>
> **定案方式说明**：§3 的 A~F 采用前两轮「编号 + 推荐 + 备选 + 代价」清单中的推荐项定案；
> **G 为用户直接裁决**（2026-09-10：「累计价格做成最近一次持续运行以来的费用总和，重启清零」）。
> 若用户对某项另有指示，只需改 §3 对应行并同步该行指向的 §4 小节与 §6 测试项。

---

## 〇、改造后怎么用（大白话，供实现者对齐意图 / 供验收）

1. 打开 **设置 → 翻译**。模型列表里，**每一行右边都有一个「测试」按钮**——想测哪个供应商就点哪个，
   不用先选中、不会把你正在用的模型切走、也不会偷偷去测别的。
2. 点下去后，那一行的按钮变成 **「中断」**，下面开始走秒：`测试中… 3.4 秒`。
   其他行的测试按钮临时变灰（一次只测一个）。
3. 不想等了就点 **「中断」**：界面立刻恢复可用，后台那次请求也会被真正掐断（不会偷偷跑完、不会占着连接）。
4. 结果直接出现在**那一行下面**，四种之一：
   - 绿 ✓ `连接成功 · 820 ms · 已降级请求形态 · 返回："…"`
   - 红 ✖ `连接失败` + 一眼看懂的中文原因（地址连不上 / 密钥不对 / 模型名不存在 / 被限流 / 超时）
   - 灰 ⏱ `已中断`
   - 黄 ？ `60 秒内未取得结论`——**不是失败**，可再点一次（本地模型首次加载常这样）
5. 测试期间应用照常工作：录音、字幕、翻译都不受影响（测试跑在自己的线程上，不再占用识别线程）。
6. 改了某一行的地址/密钥/模型名，那行的旧测试结果自动作废，重新点「测试」即可。
7. 列表上方新增一行 **`当前使用：XXX`**：这就是"当前用哪个 LLM 做翻译"的答案（原来只能靠行首
   `>>>` 猜）。点某一行即把它设为当前使用，切换生效后短暂显示「已切换生效」。
8. 右下角费用统计改为 **本次运行累计**：切换模型不清零、退出应用归零；用不报用量的端点时
   费用旁标「部分未知」。

---

## 一、问题（全部实证，行号 2026-09-10 核对）

### 1.1 「测试连接」按钮在数条常见路径上**点了就永久卡死**

| # | 缺陷 | 证据 |
|---|---|---|
| P1 | UI 先把状态置为 `Running`（按钮变「测试中…」并禁用），命令却可能从未被执行——无人回执，按钮**永久锁死**，只能重启应用 | 先置态：`crates/lt-ui/src/windows/panel/translation.rs:367-368`；静默丢弃：`crates/lt-app/src/shell.rs:246-250`（`if let Some(p) = self.pipeline.as_ref()` 无 else 臂）；发送失败被忽略：`crates/lt-orchestrator/src/pipeline.rs:1307-1312`（`let _ = tx.send(...)`） |
| P2 | 执行位置错：探测命令塞给 ASR 线程，而 ASR 线程**只在空闲分支**才收命令（需段队列连续 500ms 空窗） | `pipeline.rs:2028`（主循环空闲分支）、`:1947`（模型未就绪时的待命循环） |
| P3 | 测试目标含糊：`编辑行 → 选中行 → 活动模型` 三级**静默回落**；结果不回显测的是谁（事件带 `name`，UI 直接丢弃） | `translation.rs:347-350`；`crates/lt-ui/src/app.rs:1138-1146`（`let _ = name;`） |
| P4 | 不能中断；阶梯最多 6 个请求形态 + 1 次截断补发，每级吃满超时（默认 10 秒，可设 1~60 秒）→ **最坏 7 分钟**无反馈 | 阶梯链：`crates/lt-translate/src/thinking.rs:175-183`；补发：`pipeline.rs:474-518`；超时范围：`translation.rs:443`；全仓无取消命令 |

附：每点一次测试会临时搭一个 8 线程翻译池（`TlRig`）只为发一个 HTTP 请求。

### 1.2 多供应商热切换：**链路已实现，但有五处欠缺**

已实现并验证的链路（点行 → 新装置生效 → 落盘 → 重启恢复）见 §附录 A。欠缺：

| # | 欠缺 | 证据 |
|---|---|---|
| H1 | 生效时机不可控：`ReplaceRig` 只在 ASR 线程空闲分支被消费，连续识别时延后到"当前段转录完 + 下一个 500ms 空窗" | `pipeline.rs:2028`、`:1947` |
| H2 | 零反馈：面板唯一变化是 `>>>` 前缀与加粗挪位，无"当前使用"状态行、无"已生效"回执 | `translation.rs:81-84` |
| H3 | 换模型时在队未翻译的段收到写死的红色失败回执「队列积压，保留最新（本段已放弃）」——**真实原因是切换了模型** | 回执文案：`pipeline.rs:87-98`；i18n：`assets/i18n/zh.yaml:528` `err_dropped`；字幕按失败色渲染：`crates/lt-ui/src/windows/subtitle.rs:530-537` |
| H4 | 不做语义验证：地址/密钥/模型名错要等后续每段翻译失败才暴露（URL 形态错有红字拦截） | `pipeline.rs:1744-1772`（构建失败发 `TranslatorUnavailable`） |
| H5 | 累计统计被清零：新装置自带全新 `TlStats`，`UpdateStats` 在 UI 侧整体覆盖 | `pipeline.rs:221-266`（per-rig）、`:738`（每装置新建）；`app.rs:1043-1055`（整体覆盖） |

### 1.3 测试覆盖缺口

`route_translator_switch`（`pipeline.rs:1732`）**无直接单测**——只有模拟"替换语义"的 `replaced_rig_workers_shutdown_on_drop`（`pipeline.rs:3241`）与 `retired_pool_emits_receipts_for_queued_jobs`；UI 侧"点行 → 发命令"亦无测试。

---

## 二、目标与非目标

**目标**

1. 连接测试**点哪测哪**：目标永远是用户点的那一行供应商配置，无静默回落、无副作用（不切换正在使用的模型）。
2. 测试**随时可中断**，中断后界面 1 秒内恢复且后台请求真正被掐断。
3. 测试**不再卡死**：任何异常路径（命令丢失、线程死亡、回执超时）都必须收敛到终态。
4. 测试不依赖 ASR 线程，识别/翻译进行中也能测，且不干扰在进行的翻译。
5. 热切换可见（当前使用 + 已生效）、可控（下一段边界即生效）、可信（先测再切）。
6. 累计费用按**进程生命周期**累加，切换模型不清零，退出即归零。

**非目标**（明确不做，避免范围蔓延）

- 不做"切换前自动探针"（换模型不得因网络而变慢，验证权交给用户）。
- 不做并发多探测（同一时刻至多一个在途探测）。
- 不改 ASR 侧任何行为（引擎/模型/下载/零信任闸门）。
- 不改模型编辑对话框的字段与校验逻辑（`config_warnings` 保持现状）。
- 不做跨进程持久的费用累计（重启必须清零）。

---

## 三、定案（A~G）

| # | 议题 | **定案** | 备选与否决理由 |
|---|---|---|---|
| **A** | 测试入口 | **每行独立「测试」按钮**（列表行右侧），删除底部四按钮行中的测试按钮与"无选中回落活动模型" | 否决"单按钮 + 必须先选中"：本应用中"选中一行"＝"设为当前使用并立即切换翻译器"（`translation.rs:268-277`），会让"只是想试试备用供应商"变成切走生产模型 |
| **B** | 单次预算 | **总预算 60 秒；单步超时 = min(用户超时, 20 秒)**；预算耗尽 → `Inconclusive`（**不是失败**） | 否决 30 秒（本地模型首次加载常超，会误报未定论）；否决不设上限（最坏 7 分钟，违背即时反馈） |
| **C** | 结果内容 | 成功：耗时 + 实际生效请求形态（降级时告知）+ 回执摘要（≤60 字符）；失败：分类中文原因 + 原始详情（次要行/悬停） | 否决"只显示 ✓/✖ + 耗时"（丢失诊断价值） |
| **D** | 并发 | **一次只测一个**：在途期间其他行的测试按钮禁用 | 否决并发（取消语义与结果归属复杂化） |
| **E** | 选中语义 | **不拆**：行点击保持"设为当前使用"（原版语义）；测试改用每行按钮后与选中互不干扰；另加"当前使用"状态行消除歧义 | 否决本轮拆分"选中/使用"（改动面大，收益可由状态行覆盖） |
| **F** | 热切换加固 | **做三项**：F1 命令消费点提前到每轮循环开头；F2 切换回执 `TranslatorSwitched` + 「当前使用」状态行；F3 退休任务回执中性化（新 `FailureKind::Superseded`，字幕中性灰）。**另补** `route_translator_switch` 单测 | — |
| **G** | 统计口径 | **会话级累计**（用户裁决）：`asr_n / tl_n / tokens / cost` 全部按本次运行累加，跨模型切换不清零，重启归零；费用按**每笔发生时的当时单价**累加 | 否决"仅费用累计、句数保持 per-rig"（会出现"费用在涨、句数只有 3"的自相矛盾显示） |

---

## 四、详细设计

### 4.1 契约变更（`crates/lt-proto/`）

**`PROTO_VERSION: 5 → 6`**（`lib.rs:21`；结构变更需递增）。

`src/lib.rs` 新增共享常量（跨层单一事实源：`lt-orchestrator` 探测预算与 `lt-ui` 看门狗同源，
且 `lt-ui` **不得**依赖 `lt-orchestrator`——架构 §3.1 白名单）：

```rust
/// 连接探测总预算（秒）。编排域据此设 deadline，UI 域据此设看门狗（+10s 裕量）。
pub const PROBE_TOTAL_BUDGET_SECS: u64 = 60;
/// 单步超时上限（秒）：用户超时更短时取用户值，更长时封顶。
pub const PROBE_STEP_TIMEOUT_CAP_SECS: u32 = 20;
```

`src/events.rs` 新增：

```rust
/// 连接探测结果分类（`Cmd::TestTranslator` 回执，D-85）
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeOutcome {
    /// 打通且拿到非空译文
    Ok,
    /// 失败：分类 + 原始详情（详情原文仅供悬停/次要行展示）
    Failed { kind: FailureKind, detail: String },
    /// 用户中断
    Cancelled,
    /// 预算耗尽仍未取得结论——**不是失败**
    Inconclusive { attempted: u8 },
}
```

`Cmd` 变更：

```rust
/// 翻译配置「连接测试」：构建临时装置跑完整回退阶梯，回执 TestTranslatorResult
TestTranslator { config: Box<ModelConfig>, probe_id: u64 },   // 改型（原为元组单参）
/// 中断在途连接测试（probe_id 不匹配则忽略）
CancelTranslatorTest { probe_id: u64 },                       // 新增
```

`UiEvent::TestTranslatorResult` 改型（原 `{name, ok, error, ms}` 删除）：

```rust
TestTranslatorResult {
    /// 回执归属（UI 只采纳与在途探测 id 相同者）
    probe_id: u64,
    name: String,
    outcome: ProbeOutcome,
    /// 从发起探测到得出结论的耗时（毫秒）
    ms: u64,
    /// 成功时实际生效的请求形态说明（起点形态为 None）
    step_note: Option<String>,
    /// 成功时的回执摘要（去换行、截断至 60 字符）
    preview: Option<String>,
},
```

`UiEvent` 新增：

```rust
/// 翻译装置切换完成（ReplaceRig 成功 / 首次装配成功，D-85/F2）
TranslatorSwitched { name: String, model: String },
```

`FailureKind` 新增两变体（`events.rs:217` 段落）：

```rust
/// 用户中断（连接测试取消等，D-85）
Cancelled,
/// 装置被替换：换模型时在队未翻译的段——非错误，字幕中性展示（D-85/F3）
Superseded,
```

`FailureKind::i18n_key()` 增两键：`Cancelled → "err_cancelled"`、`Superseded → "err_superseded"`。

`ThreadRole` 新增变体（`events.rs:405` 段落）：`TranslatorProbe`（一次性探测线程，`Policy::Never`）。

**死契约守卫预期**：`FailureKind::Cancelled` 引用点 = `lt-translate/error.rs`（生产）+ `lt-orchestrator/probe.rs`（消费）≥2；`FailureKind::Superseded` 引用点 = `pipeline.rs`（生产）+ `subtitle.rs`（中性渲染分支）≥2。若 `check_dead_contract.ps1` 仍报 WARN，按脚本既有先例登记进"已定性合法单边形态"注释并说明理由。

### 4.2 lt-translate：可取消的翻译调用

`src/error.rs`：

- 新增 `TranslateError::Cancelled`（`#[error("cancelled")]`）。
- `is_expected()` → `true`。
- `failure_kind()` → `lt_proto::FailureKind::Cancelled`。

`src/translator.rs`：

1. 新增常量 `const CANCEL_POLL: Duration = Duration::from_millis(150);`
2. `Translator` 增字段 `cancel: Option<Arc<AtomicBool>>`；`Translator::new()` 置 `None`。
3. 新增 builder：`pub fn with_cancel(&self, flag: Arc<AtomicBool>) -> Translator`。
4. **字段透传**：`share_client()` 与 `minimal()` 的显式字段清单都要补 `cancel: self.cancel.clone()`
   （`minimal()` 是白名单构造；取消令牌**不是**请求参数，必须保留，否则降级到最小请求的一步将不可中断）。
5. 流式路径 `translate_iter`：`StreamInner::Streaming` 增 `cancel: Option<Arc<AtomicBool>>`；`next()` 的
   `rx.recv_timeout(remaining)` 改为：

   ```text
   loop:
     若 deadline 已过            → return Some(Err(Timeout))        // 语义不变
     若 cancel 已置位            → return Some(Err(Cancelled))      // 新增
     let slice = if cancel.is_some() { CANCEL_POLL.min(remaining) } else { remaining };
     match rx.recv_timeout(slice):
         Err(RecvTimeoutError::Timeout) → continue                  // 重新计算 remaining 后循环
         Err(Disconnected)              → 既有分支不变
         Ok(msg)                        → 既有分支不变
   ```

   **关键约束**：`cancel.is_none()`（生产翻译路径）时 `slice == remaining`，与现状逐字节等价。
6. 非流式路径：`timeout_block`（`translator.rs:552-567`，被 `translate_sync` 使用）在 `cancel.is_some()`
   时改为 `tokio::select!` 竞争：主 future 正常超时语义不变；取消分支每 100ms 轮询标志，命中即
   `Err(TranslateError::Cancelled)`。取消分支必须**丢弃主 future**（`select!` 语义天然满足）。
7. `pump_stream` 与 `TranslateStream::drop` 的 `handle.abort()` 不动——迭代器被丢弃即关闭 HTTP 连接。

### 4.3 lt-orchestrator：探测模块（新文件 `crates/lt-orchestrator/src/probe.rs`）

```rust
//! 供应商连接探测（翻译页「测试」按钮的执行面，D-85）
//! 依赖白名单不变；用户文案经 [`Msg`] 注入。

/// 单次探测的总预算（D-85 决策 B；常量真源在 lt-proto，UI 看门狗同源）
pub const PROBE_TOTAL_BUDGET: Duration =
    Duration::from_secs(lt_proto::PROBE_TOTAL_BUDGET_SECS);
/// 单步超时上限（用户超时可更短，不可更长）
pub const PROBE_STEP_TIMEOUT_CAP: u32 = lt_proto::PROBE_STEP_TIMEOUT_CAP_SECS;
/// 探测文本（与现状一致，避免改变判据语义）
pub const PROBE_TEXT: &str = "Livetranslate test";
/// 回执摘要上限（字符）
pub const PROBE_PREVIEW_MAX_CHARS: usize = 60;

/// 执行一次连接探测；结果经 `sink` 回执 `UiEvent::TestTranslatorResult`。
/// 本函数**不写**任何会话记忆（learned）与降级标记（degraded_notified）——
/// 探测是只读观察，不改变生产装置的会话状态。
pub fn run_probe(
    probe_id: u64,
    config: &lt_proto::ModelConfig,
    bus: &Arc<SettingsBus>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    msg: &Msg,
    sink: &EventSink,
)
```

流程（逐步、无分支遗漏）：

1. `let name = config.name.clone(); let t0 = Instant::now();`
2. `cancel` 已置位 → 直接回执 `Cancelled`（`ms = 0`），结束。
3. `let eff = bus.load();` → `let params = translator_params(config, &eff);`（见 4.4，**单一构造点**）
   → `Translator::new(params)`：
   - `Err(e)` → 回执 `Failed { kind: FailureKind::NotReady, detail: format!("{}: {e:#}", config.name) }`，结束。
4. `let start = lt_translate::first_step(t.thinking_plan());`
   `let explicit = config.disable_thinking && !config.thinking_unavailable
        && matches!(config.thinking_style.as_deref(), Some(s) if !s.is_empty() && s != "auto");`
   `let allow = config.disable_thinking && !config.thinking_unavailable && !explicit;`
   （与 `TlRig::from_effective` 的判据**逐字相同**——判据一致性是第二轮评审的硬要求。）
5. `let t = t.with_cancel(cancel.clone());`
   `let step_timeout = eff.tl.timeout.min(PROBE_STEP_TIMEOUT_CAP).max(1);`
   `let ctl = RunCtl { cancel: Some(cancel.clone()), deadline: Some(t0 + PROBE_TOTAL_BUDGET) };`
6. `let out = run_ladder(&t, start, allow, PROBE_TEXT, "auto", &eff.tl.target_language,
        step_timeout, &ctl, sink, 0, 0, false);`
7. 结论映射（顺序即优先级）：

   | 条件 | outcome | 附带字段 |
   |---|---|---|
   | `out.halted == Some(Halt::Cancelled)` | `Cancelled` | — |
   | `out.halted == Some(Halt::Budget)` | `Inconclusive { attempted: out.attempted }` | — |
   | `out.attempt.succeeded() && 正文非空` | `Ok` | `step_note` = 起点形态则 `None`，否则 `Msg` 注入的降级说明；`preview` = 正文去换行后截断 60 字符 |
   | 其余 | `Failed { kind, detail }`，其中 `kind = out.attempt.error.map(failure_kind).unwrap_or(Empty/Truncated)`（`verdict == Some(EmptyTruncated)` → `Truncated`，否则 `Empty`），`detail = ui_text()` 或体检描述 | — |

8. `sink.push(UiEvent::TestTranslatorResult { probe_id, name, outcome, ms: t0.elapsed().as_millis() as u64, step_note, preview });`

`step_note` 取值规则（无歧义）：最终成功台阶为 `RequestStep::Minimal` → `msg.t("probe_step_minimal")`；
为 `Plan(p)` 且 `p != 起点形态` → `msg.t("probe_step_degraded")`；等于起点 → `None`。

### 4.4 lt-orchestrator：阶梯与装置构造的单点化

1. **`translator_params` 单点构造**：把 `TlRig::from_effective`（`pipeline.rs:691-727`）中构造
   `lt_translate::TranslatorParams` 的整段抽为自由函数：

   ```rust
   pub(crate) fn translator_params(
       mc: &lt_proto::ModelConfig,
       eff: &EffectiveSettings,
   ) -> lt_translate::TranslatorParams
   ```

   `TlRig::from_effective` 与 `probe::run_probe` **都必须**调用它（禁止第二处字段罗列）。
   为可测性：`TranslatorParams` 增 `#[derive(PartialEq)]`；`TlRig` 增字段 `params: TranslatorParams`
   保存本次构造结果；新增测试 `translator_params_single_source`：断言 `rig.params ==
   translator_params(mc, eff)`（同配置逐字段相等）。

2. **`run_ladder` 增加运行控制参数**（`pipeline.rs:435`）：

   ```rust
   /// 运行控制（探测注入取消与总预算；生产翻译传 `RunCtl::none()`）
   pub(crate) struct RunCtl {
       pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
       pub deadline: Option<Instant>,
   }
   impl RunCtl { pub(crate) fn none() -> Self { Self { cancel: None, deadline: None } } }

   /// 阶梯提前中止原因
   pub(crate) enum Halt { Cancelled, Budget }
   ```

   - 签名：`fn run_ladder(base, start, allow_verdict_advance, text, source_lang, target, timeout,
     ctl: &RunCtl, sink, id, seq, push_partials) -> LadderOutcome`
   - `LadderOutcome` 增字段 `halted: Option<Halt>` 与 `attempted: u8`（已尝试台阶数）。
   - 每次尝试**之前**：`cancel` 置位 → 返回 `halted = Some(Cancelled)`；`deadline` 已过 →
     返回 `halted = Some(Budget)`。
   - 单次尝试超时：`let per = match ctl.deadline { Some(d) =>
     timeout.min(d.saturating_duration_since(Instant::now()).as_secs().max(1) as u32), None => timeout };`
   - 现有生产调用点（`pipeline.rs` worker 闭包内）改传 `&RunCtl::none()`，**行为不变**。
   - 截断补发重试的两次尝试都要走同一套检查（含在"每次尝试之前"内）。

### 4.5 lt-orchestrator：热切换加固（F1/F2/F3）

**F1 命令消费点提前**：把主循环空闲分支里的 `while let Ok(sw) = tl_switch.try_recv() { ... }`
整段（含 `ReplaceEngine` 处理，`pipeline.rs:2026-2130`）抽为函数

```rust
fn drain_tl_switch(
    tl: &mut Option<Arc<TlRig>>,
    ctx: &AsrThreadCtx,          // bus/sink/sup/transcript/msg/learned/degraded_notified
    manager: &mut AsrManager,
    current_display: &mut String,
    asr_unavailable_notified: &mut bool,
    interim_state: &mut InterimState,
    current_settings: &lt_proto::Settings,
)
```

并在**两处**调用：① 主循环 `while` 体**开头**（新增——使切换在下一段边界即生效）；② 空闲分支（保留，
覆盖空闲期的立即消费）。原空闲分支内的内联实现删除，只留调用。

**F2 切换回执**：`route_translator_switch` 的 `ReplaceRig` 成功分支（`pipeline.rs:1755-1761`）在
`*tl = Some(Arc::new(rig));` 之后追加：

```rust
sink.push(UiEvent::TranslatorSwitched { name: config.name.clone(), model: config.model.clone() });
```

`Pipeline::start` 首次装配成功处（`pipeline.rs:1136` 之后，`tl` 为 `Some` 时）同样推送一次，
使启动后状态行立即正确。

**F3 退休回执中性化**：`JobPool` 增 `superseded: Arc<AtomicBool>`（`JobPool::new` 创建，传给每个 `TlJob`）；
`JobPool::retire()` 在出队前 `superseded.store(true, SeqCst)`；`TlJob::drop`（`pipeline.rs:87-98`）据此选择：

```rust
kind: if self.superseded.load(Relaxed) { FailureKind::Superseded } else { FailureKind::Dropped },
detail: if superseded { "已切换模型，本段未翻译" } else { "队列积压，保留最新（本段已放弃）" },
```

队列溢出（`BoundedDropQueue` 满丢最旧）路径语义不变，仍为 `Dropped`。

### 4.6 lt-orchestrator：会话级统计（G）

1. `TlStats`（`pipeline.rs:221-266`）改造：

   ```rust
   pub(crate) struct TlStats {
       asr_count: AtomicU64,
       tl_count: AtomicU64,
       prompt_tokens: AtomicU64,
       completion_tokens: AtomicU64,
       /// 累计金额，单位 1e-9 美元（整数累加，避免浮点原子）
       cost_nano: AtomicU64,
       /// 本次运行是否**出现过**用量未知的调用（true = 费用不完整）
       usage_unknown_seen: AtomicBool,
   }
   ```

   方法：
   - `fn new() -> Self`（无参——价格不再属于累计器）
   - `fn record_translation(&self, pt: u64, ct: u64, usage_known: bool, prices: (f64, f64))`：
     累加 tokens、`tl_count`，并按**本次调用的价格**累加金额：
     `cost_nano += (compute_cost(pt, ct, prices.0, prices.1) * 1e9).round() as u64`；
     `usage_known == false` → `usage_unknown_seen.store(true)`。
   - `fn snapshot_event(&self) -> UiEvent`：`cost = cost_nano as f64 / 1e9`，
     `usage_known = !usage_unknown_seen.load()`。
2. **归属上移**：`Pipeline` 增字段 `session_stats: Arc<TlStats>`，在 `Pipeline::start` 创建一次
   （`pipeline.rs:1130` 附近），传给 `TlRig::from_effective`；`TlRig` 的 `stats` 字段保留但**不再自建**
   （`pipeline.rs:738` 的 `Arc::new(TlStats::new(...))` 删除）。装置持有 `Arc` 克隆，故该字段删掉也不影响
   生命周期；保留它是为了让"会话级"这一事实在类型上可见（若 clippy/编译器报未读，加
   `#[allow(dead_code)]` 并写明"持有即语义"）。
3. `TlRig` 增字段 `prices: (f64, f64)`（取自 `mc.input_price/output_price`）；
   `finish_ok` / `fail`（`pipeline.rs:547-620`）改为调用 `stats.record_translation(pt, ct,
   attempt.usage_known, prices)`，`prices` 由调用处透传。
4. `asr_count`、同语言路径的 `snapshot_event()` 调用点不变——计数器现在是会话级，天然满足"重启清零"。
5. UI 侧（`app.rs:1043`）**不改结构**（仍整体覆盖），只按 §4.9 的键补"部分未知"标记。

### 4.7 lt-app：命令路由与一次性线程

`shell.rs`：

```rust
Cmd::TestTranslator { config, probe_id } => self.start_probe(*config, probe_id),
Cmd::CancelTranslatorTest { probe_id } => {
    if self.probe_id.load(Ordering::SeqCst) == probe_id {
        self.probe_cancel.store(true, Ordering::SeqCst);
        tracing::info!("已请求中断连接测试 #{probe_id}");
    } else {
        tracing::debug!("忽略过期取消请求 #{probe_id}（在途 #{})", self.probe_id.load(Ordering::SeqCst));
    }
}
```

新增字段（`AppShell`）：`probe_cancel: Arc<AtomicBool>`、`probe_id: Arc<AtomicU64>`、
`probe_active: Arc<AtomicBool>`。

`fn start_probe(&mut self, config: ModelConfig, probe_id: u64)`：

1. 防重入：`if self.probe_active.swap(true, SeqCst) { tracing::warn!("连接测试已在途，忽略重复请求 #{probe_id}"); return; }`
   （UI 侧已禁用其他按钮，此为纵深防御；**不留回执**由 §4.8 的 UI 看门狗兜底。）
2. `self.probe_cancel = Arc::new(AtomicBool::new(false)); self.probe_id.store(probe_id, SeqCst);`
3. 经监督器一次性线程出生（镜像 `start_bench` / `spawn_device_probe` 范式，`shell.rs:375-425`）：

   ```rust
   self.sup.spawn(ThreadRole::TranslatorProbe, "lt-probe", Policy::Never, move || {
       let (artery, bus, cancel, active, msg) = (...clone...);
       Box::new(move || {
           let _guard = ProbeActiveGuard(active);       // 任何退出路径（含 panic unwind）复位
           lt_orchestrator::probe::run_probe(probe_id, &config, &bus, cancel, &msg, &artery);
       })
   });
   ```

   `Msg` 句柄：`AppShell` 增字段 `msg: lt_orchestrator::Msg`，在 **`AppShell::new`** 里构造一次
   （`Msg::new(lt_i18n::t)`）存字段；`start_pipeline` 改为复用该字段（不得在 `start_pipeline` 内构造
   ——探测可能发生在管道未启动时）。

4. `shell_helpers.rs` 增 `pub struct ProbeActiveGuard(pub Arc<AtomicBool>)`（`Drop` 复位，
   与既有 `BenchActiveGuard` 同款）。
5. **停机**：`AppShell::shutdown` 增加 `self.probe_cancel.store(true, SeqCst);`（在 `p.stop()` 之前），
   避免退出被在途探测拖住最多一个流式轮询周期（150ms）或一次尝试超时（非流式，≤20s）。

### 4.8 lt-ui：翻译页交互

**状态（`crates/lt-ui/src/state.rs`，替换现有 `TestTranslatorState`）**：

```rust
/// 供应商连接测试的 UI 侧状态（D-85；一次至多一个在途）
#[derive(Debug, Clone, Default)]
pub struct ProbeUiState {
    /// 号源（单调递增；迟到回执据此丢弃）
    pub next_id: u64,
    /// 在途探测（None = 无在途）
    pub running: Option<ProbeRun>,
    /// 最近一次结果（渲染期按行号 + 配置指纹校验有效性）
    pub result: Option<ProbeResult>,
}
pub struct ProbeRun { pub id: u64, pub row: usize, pub started: Instant, pub cfg_key: CfgKey }
pub struct ProbeResult {
    pub id: u64, pub row: usize, pub cfg_key: CfgKey, pub name: String,
    pub outcome: lt_proto::ProbeOutcome, pub ms: u64,
    pub step_note: Option<String>, pub preview: Option<String>,
}
/// 配置身份（结果失效判据；行编辑任何一项即失配）
pub type CfgKey = (String, String, String, String);   // name, api_base, model, api_key
```

`PanelUi.test_translator: TestTranslatorState` 替换为 `PanelUi.probe: ProbeUiState`；
`PanelUiState` 增 `active_model_note: Option<(String, Instant)>`（"已切换生效"瞬时提示，2 秒过期）。

**行渲染（`windows/panel/translation.rs`）**：列表行由"整行 selectable 按钮"改为

```text
[ 行按钮（selectable，宽 = available_width - 72 - item_spacing） ][ 测试/中断（宽 72）]
```

- 行按钮：文本、`>>>` 前缀、加粗、点击选中、双击编辑——**全部保持现状**。
- 右侧按钮两态：
  - 非在途（或 `running.row != i`）：标签 `probe_btn`（「测试」），`add_enabled(running.is_none(), ...)`；
    点击 → `id = probe.next_id; probe.next_id += 1;` → `probe.running = Some(ProbeRun{id, row: i,
    started: Instant::now(), cfg_key: 该行 cfg_key})` → 发 `Cmd::TestTranslator { config: 该行配置克隆,
    probe_id: id }`。
  - 在途且 `running.row == i`：标签 `probe_cancel`（「中断」）；点击 → 发
    `Cmd::CancelTranslatorTest { probe_id: id }`，**立即**把该行结果落为
    `ProbeResult { outcome: Cancelled, ms: started.elapsed(), .. }` 并清 `running`（不等后台回执）。
- 行下附加行（`ui.add_space(2.0)` 后一行）：
  - `running.row == i` → `probe_running`（「测试中…」）+ 已耗时（`{:.1}s`），并
    `ui.ctx().request_repaint_after(Duration::from_millis(100))` 保证走秒刷新。
  - `result` 有效（`result.row == i` 且 `result.cfg_key == 该行 cfg_key`）→ 按 outcome 渲染：
    - `Ok`：绿色 `pal.ok`，`✓ probe_ok · {ms} ms · {step_note?} · probe_preview{preview}`
    - `Failed{kind,detail}`：红色 `pal.err`，`✖ probe_failed · {ms} ms · failure_text(kind)`；
      `detail` 以次要行弱色显示（≤2 行省略）
    - `Cancelled`：弱色 `pal.weak`，`⏱ probe_cancelled · {ms} ms`
    - `Inconclusive{attempted}`：警示黄，`？ probe_inconclusive · {ms} ms · probe_attempted{n}` +
      次要行 `probe_inconclusive_hint`
- **删除**四按钮行中的测试按钮（`translation.rs:345-372`）与旧结果块（`:374-393`）。

**回执处理（`app.rs`）**：`UiEvent::TestTranslatorResult { probe_id, name, outcome, ms, step_note,
preview }`：

```text
若 panel.probe.running 为 None 或 running.id != probe_id → 丢弃（debug 日志），不改状态
否则：result = Some(ProbeResult{ id, row: running.row, cfg_key: running.cfg_key, name,
        outcome, ms, step_note, preview }); running = None; redraw(Panel)
```

`UiEvent::TranslatorSwitched { name, .. }` → `panel.state.active_model_note = Some((name, Instant::now()))`
→ `redraw(Panel)`。

**看门狗（永不卡死的最终保证）**：面板帧内（渲染前）检查
`if let Some(r) = &panel.probe.running { if r.started.elapsed().as_secs() > lt_proto::PROBE_TOTAL_BUDGET_SECS + 10 { ... } }`
（常量取 `lt_proto`——**不得**从 `lt-orchestrator` 取，UI 无此依赖边）
→ 落 `ProbeResult { outcome: Inconclusive { attempted: 0 }, .. }`、清 `running`、记
`tracing::warn!("连接测试回执超时（命令可能未送达）")`、`redraw`。

**状态行（F2）**：模型配置 `group_card` 内、错误横幅之后、列表之前增加一行：

```text
当前使用：{name}            // settings.active_model 对应行；越界时回落第 0 行
    + 若 active_model_note 新鲜（<2s）→ 追加 " · 已切换生效"
```

### 4.9 i18n 键表（`assets/i18n/zh.yaml` + `en.yaml` 同步）

**删除**：`test_translator_btn`、`test_translator_testing`、`test_translator_ok`、`test_translator_fail`、
`test_translator_no_response`、`test_translator_no_config`（删除前逐键确认全仓零引用）。

**新增**：

| 键 | zh | en |
|---|---|---|
| `probe_btn` | 测试 | Test |
| `probe_cancel` | 中断 | Stop |
| `probe_running` | 测试中… | Testing… |
| `probe_ok` | 连接成功 | Connection OK |
| `probe_failed` | 连接失败 | Connection failed |
| `probe_cancelled` | 已中断 | Cancelled |
| `probe_inconclusive` | 60 秒内未取得结论 | No verdict within 60 s |
| `probe_inconclusive_hint` | 不是失败：可能是模型较慢或端点无响应。可重试。 | Not a failure: the model may be slow or the endpoint unresponsive. You can retry. |
| `probe_preview` | 返回：{text} | Reply: {text} |
| `probe_attempted` | 已尝试 {n} 种请求形态 | {n} request shapes tried |
| `probe_step_minimal` | 已降级到最小请求 | Fell back to the minimal request |
| `probe_step_degraded` | 已降级请求形态 | Fell back to a degraded request shape |
| `probe_no_receipt` | 未收到测试回执（已超时），请重试 | No test receipt (timed out); please retry |
| `err_cancelled` | 已中断。 | Cancelled. |
| `err_superseded` | 已跳过（切换了模型，本段未翻译）。 | Skipped (model switched; this segment was not translated). |
| `err_subtitle_skipped` | — 已跳过（切换模型） | — skipped (model switched) |
| `status_active_model` | 当前使用：{name} | Active model: {name} |
| `status_switched` | 已切换生效 | Switched |
| `stats_partial_unknown` | 部分未知 | partial |

`usage_known == false` 且 `cost > 0` 时，MonitorBar 费用段追加 `stats_partial_unknown`
（`crates/lt-ui/src/windows/overlay.rs:547` 与 `:639` 两处渲染分支）；
`cost == 0` 且从未有已知用量时维持现状显示「—」。

### 4.10 交互状态机（无歧义清单）

| 当前态 | 触发 | 动作 | 次态 |
|---|---|---|---|
| 无在途 | 点第 i 行「测试」 | 分配 id、置 running、发 `Cmd::TestTranslator{config_i, id}` | 在途(id, i) |
| 在途(id, i) | 收到 `TestTranslatorResult{probe_id != id}` | 丢弃（debug 日志） | 在途(id, i) 不变 |
| 在途(id, i) | 收到 `TestTranslatorResult{probe_id == id}` | 落 result（行 i、cfg_key_i） | 无在途 + Result |
| 在途(id, i) | 点行 i「中断」 | 发 `Cmd::CancelTranslatorTest{id}`；本地立即落 `Cancelled` | 无在途 + Result |
| 在途(id, i) | 已耗时 > 70 秒 | 本地落 `Inconclusive{0}` + warn 日志 | 无在途 + Result |
| 在途(id, i) | 切换面板页/关闭面板 | 无动作（探测继续） | 在途(id, i) |
| 无在途 | 结果行所在配置被编辑 | 渲染期 cfg_key 失配 → 不显示（存储不变） | 无在途 |
| 无在途 | 结果行被删除/行号越界 | 渲染期行号越界 → 不显示 | 无在途 |

**不变量**

1. 同一时刻至多一个在途探测（UI 禁用 + shell 防重入 + 看门狗三重保证）。
2. 任何在途探测必在 ≤70 秒内落到终态（成功/失败/中断/未定论），**不存在永久「测试中…」**。
3. 探测不产生副作用：不切活动模型、不写 learned/degraded、不动 transcript、不进统计。

---

## 五、施工步骤（提交切分；每步提交前必须全绿）

| # | 提交 | 内容 | 收工前提 |
|---|---|---|---|
| C1 | `docs(translator-probe): 方案定稿（D-85）` | 本文件 + `docs/README.md` 索引行 | 四守护脚本过（docs 变更仅路径卫生相关） |
| C2 | `feat(probe-lt): lt-translate 可取消调用 + Cancelled 错误` | §4.2 全部 + 三项取消测试 | `cargo test -p lt-translate` 绿；clippy 零告警 |
| C3 | `feat(probe): 连接测试迁移到独立线程（每行按钮 + 四态结果，D-85）` | §4.1 契约 + §4.3 probe + §4.4-1 params 单点 + §4.7 shell + §4.8 UI + §4.9 键（probe 段） | 全量测试绿 + 四守护 + clippy |
| C4 | `feat(hotswap): 切换回执/消费点提前/退休回执中性化（D-85/F）` | §4.4-2 RunCtl 中 non-probe 部分已随 C3；本提交 = §4.5 三项 + §4.8 字幕中性化 + 状态行 | 全量测试绿 + 四守护 + clippy |
| C5 | `feat(stats): 会话级累计（D-85/G）` | §4.6 + §4.9 的 `stats_partial_unknown` | 全量测试绿 + 四守护 + clippy |

依赖关系：C2 独立可编译；C3 依赖 C2；C4/C5 依赖 C3（共用 `TranslatorSwitched` 与 UI 状态结构）。
C3 是最大提交（契约 + 编排 + shell + UI 需同步改，否则旧调用点编译不过）——允许拆为
"C3a 契约 + 编排 + shell（旧按钮暂删）"与"C3b UI 每行按钮"两提交，但 C3a 必须在删除旧 UI 路径的
同一提交内保证可编译。

---

## 六、验收标准

### 6.1 自动化测试清单

**lt-translate（C2）**

| 测试 | 断言 |
|---|---|
| `cancel_stops_streaming_within_poll_interval` | 假服务端慢流（每 500ms 一 chunk）→ 置取消 → 迭代器 ≤400ms 返回 `Err(Cancelled)` |
| `cancel_before_first_read_returns_immediately` | 置取消后再调用 → 首个 `next()` 即 `Err(Cancelled)` |
| `cancel_works_on_sync_path` | 非流式 + 慢响应 → 置取消 → ≤400ms 返回 |
| `no_cancel_token_keeps_legacy_timeout_semantics` | `cancel = None` 时超时仍返回 `Err(Timeout)`（回归） |
| `minimal_and_with_plan_preserve_cancel` | `minimal()` / `with_plan()` 产物仍持有取消令牌 |

**lt-orchestrator（C3/C4/C5）**

| 测试 | 断言 |
|---|---|
| `probe_ok_reports_preview_and_ms` | 假服务端返回 `"你好"` → `Ok` + `preview = "你好"` + `step_note.is_none()` |
| `probe_ladder_reports_degraded_step` | 首个形态 400 → 降级成功 → `step_note.is_some()` |
| `probe_reports_auth_failure_kind` | 401 → `Failed{ kind: Auth }` |
| `probe_reports_malformed_config_as_not_ready` | URL 非法 → `Failed{ kind: NotReady }` |
| `probe_budget_exhausted_is_inconclusive` | 服务端永不返回 + 预算 1 秒 → `Inconclusive{ attempted >= 1 }` |
| `probe_cancel_is_reported` | 置取消 → `Cancelled` |
| `probe_does_not_touch_learned_or_degraded` | 探测后会话记忆与降级标记为空 |
| `translator_params_single_source` | `rig.params == translator_params(mc, eff)` |
| `session_stats_accumulate_across_rig_replacement` | 换装置（含不同单价）后 `asr_n/tl_n/tokens/cost` 不归零且费用按各自单价累加 |
| `session_stats_marks_partial_unknown` | 一次 `usage_known=false` → 快照 `usage_known=false` 且费用仍累计 |
| `retired_pool_marks_superseded_not_dropped` | `retire()` 双段 → 两条 `TranslationFailed{ kind: Superseded }` |
| `queue_overflow_still_reports_dropped` | 溢出丢最旧仍为 `Dropped`（回归） |
| `replace_rig_pushes_translator_switched` | 成功替换 → `UiEvent::TranslatorSwitched{name, model}` |
| `drain_tl_switch_runs_before_segment_pop` | 段队列非空时仍消费到 `ReplaceRig`（F1 回归） |

**lt-ui（C3/C4）**

| 测试 | 断言 |
|---|---|
| `probe_row_button_sends_cmd_with_id` | headless 指针点击第 1 行「测试」→ 发出 `Cmd::TestTranslator{probe_id: 0}` 且 `running.row == 1` |
| `probe_cancel_button_sends_cancel_and_settles_locally` | 在途时点击 → 发 `CancelTranslatorTest` + 本地 `Cancelled` + `running == None` |
| `other_rows_disabled_while_running` | 在途时其他行按钮 `enabled == false` |
| `stale_probe_result_ignored` | `probe_id` 不匹配的回执不改变状态 |
| `probe_result_invalidated_on_config_change` | 改行配置后旧结果不渲染 |
| `probe_watchdog_settles_inconclusive` | `started` 拨回 71 秒前 → 渲染一帧 → `running == None` + `Inconclusive` |
| `active_model_status_line_renders_name` | 状态行文本含当前模型名；收到 `TranslatorSwitched` 后追加「已切换生效」 |
| `superseded_subtitle_line_is_neutral` | `failed = Some(Superseded)` 的行文本为 `err_subtitle_skipped` 且 `fail_kind == Some(Superseded)` |

**lt-app（C3）**

| 测试 | 断言 |
|---|---|
| `cancel_test_only_matches_current_id` | 过期 id 不置位取消标志 |
| `second_test_rejected_while_in_flight` | `probe_active == true` 时重复命令被拒且不发线程 |

### 6.2 实机走查清单（待用户，逐项勾）

1. 每行「测试」：新增/复制/切换行后按钮仍在正确行、点击目标正确。
2. 测试中点「中断」：界面 ≤1 秒恢复；日志确认请求已被掐断。
3. 四态观感：绿 `✓ 连接成功`、红 `✖ 连接失败`（含中文原因）、灰 `⏱ 已中断`、黄 `？ 60 秒未定论`。
4. 本地 LM Studio（模型未加载时首次请求慢）的实际表现：慢但最终成功，或落"未定论"且可重试。
5. 三种典型失败：错地址（连接失败）、错密钥（401）、错模型名（404）的文案是否可执行。
6. 测试期间正在翻译的字幕不中断、不丢句。
7. 换模型时字幕不再出现红色"队列积压"，而是中性"已跳过（切换模型）"。
8. 「当前使用：X」随切换更新；切换后出现"已切换生效"。
9. 费用跨模型切换持续累加；退出重启后归零。
10. 未提供用量的端点：费用旁出现「部分未知」。

### 6.3 守卫与门禁（每次提交前）

```powershell
powershell -File scripts/check_personal_paths.ps1
powershell -File scripts/check_deps.ps1
powershell -File scripts/check_guards.ps1
powershell -File scripts/check_dead_contract.ps1
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace      # 基线 540+9 → 本方案预计 +20 左右
```

---

## 七、风险与回滚

| 风险 | 缓解 |
|---|---|
| 改动 lt-translate 流式读取循环影响生产翻译 | 生产路径 `cancel == None`，`slice == remaining` 与原实现逐字节等价；既有全量测试 + `no_cancel_token_keeps_legacy_timeout_semantics` 回归测试 |
| 契约改型破坏其他消费点 | Rust 编译期强制；`PROTO_VERSION` 同步升 6 |
| 探测线程拖住退出 | `Policy::Never` + `shutdown` 前置取消 + `join_all`；最坏等待 = 一次流式轮询（150ms） |
| shell 防重入拒绝命令后 UI 卡死 | UI 看门狗（70 秒）兜底落终态——不依赖任何回执 |
| 会话统计改造影响成产翻译统计 | `record_translation` 在既有 `finish_ok/fail` 调用点等价替换；`session_stats_accumulate_across_rig_replacement` 与既有统计测试双覆盖 |
| 新 `FailureKind` 触发死契约守卫 | §4.1 已列引用点 ≥2；若 WARN 按脚本先例登记 |

**回滚**：按提交粒度 `git revert`；契约回滚需 `PROTO_VERSION` 回到 5（与 C3 同批回滚）。

---

## 八、不在本次范围（记录备查）

- 「切换前自动探针」：明确不做——换模型不得因网络变慢；验证权经每行「测试」按钮交给用户。
- 并发多探测、探测历史记录、探测结果持久化。
- ASR 侧（引擎/模型/下载/零信任闸门）任何改动。
- 模型编辑对话框字段与校验（`config_warnings`）改动。
- 费用跨进程持久化（重启必须清零）。

---

## 附录 A：多供应商热切换链路现状（已实现部分，供对照）

| 环节 | 状态 | 位置 |
|---|---|---|
| 面板点行切换 | ✓ | `translation.rs:268-277`（写 `active_model` + 立即发 `Cmd::SwitchTranslator`） |
| 悬浮窗模型下拉切换 | ✓ | `crates/lt-ui/src/windows/overlay.rs:369-402` |
| 命令路由 | ✓（pipeline 为 None 时静默丢弃——本方案不改，UI 侧由状态行与红字覆盖） | `shell.rs:239-244` |
| 线程命令 | ✓ | `pipeline.rs:1278-1284` → `TlSwitch::ReplaceRig` |
| 真实重建 + 失败保留旧装置 | ✓ | `pipeline.rs:1744-1772` |
| 旧装置回收（retire + 池自停） | ✓ | `pipeline.rs:1744-1761`、`:206-210` |
| 逐段读当前装置 | ✓ | `pipeline.rs:2204`（`commit_text(..., tl.as_deref(), ...)`） |
| 落盘 / 重启恢复 | ✓ | `mark_settings_dirty` → 300ms → `ApplySettings`；启动 `TlRig::from_settings`（`pipeline.rs:656`） |
| 会话学习记忆跨切换保留 | ✓ | `learned` 按 `(api_base, model)` 存、`Arc` 共享（`pipeline.rs:617-619`） |
| 对话上下文 | 有意重置 | 新 Translator 历史为空（与原版重建语义一致） |
