# 翻译供应商：连接测试 / 热切换 / 配置体验改造方案（D-85）

> 定稿 2026-09-10。**本文档即施工依据**：所有决策已在 §3 定案、所有接口已在 §4 给出精确形态，
> 施工按 §5 的提交切分逐步落地，验收按 §6 执行。文档先于实现提交（AGENTS「docs 提交时机」约定）。
>
> **定案方式说明**：§3 的定案中，**【用户裁决】标注项为 2026-09-10 用户直接给定**（E 设置页职责 /
> B 预算 10 秒 / G 会话级累计 / H 币种 / J 厂商预设 / K 默认供应商），其余为前两轮「编号 + 推荐 +
> 备选 + 代价」清单的推荐项定案。若用户对某项另有指示，只需改 §3 对应行并同步该行指向的
> §4/§十 小节与 §6 测试项。

---

## 〇、改造后怎么用（大白话，供实现者对齐意图 / 供验收）

### 0.1 两个界面的分工（用户裁决 E，本次最关键的语义变更）

- **设置 → 翻译页的供应商列表 = 只管配置**：新增、编辑、复制、删除，以及每行的「测试」按钮。
  **点一行只是"选中它"**（作为编辑/复制/删除的目标），**不会再切换正在使用的模型**。
- **切换"当前用哪个模型翻译" = 悬浮窗（主界面）第二行的「模型」下拉**（既有功能，位置不变）。
- 设置页列表上方有一行只读的 `当前使用：XXX`（告诉你现在用的是谁）+ 一句提示"要换模型请到悬浮窗的
  「模型」下拉里选"。
- **悬浮窗不在完整形态时怎么切**（用户裁决 ①，2026-09-10）：先把它弄成完整形态，再选：
  - 精简形态（一条窄条）→ 点窄条上的 **「完整」** 按钮展开；
  - 整个隐藏了 → 右键**托盘**图标 → 点 **「显示悬浮窗」**。
  这是**有意接受**的取舍（设置页不新增任何切换入口），不是缺陷；面板里的提示语会把这两步写出来。

### 0.2 测试按钮怎么用

1. 打开 **设置 → 翻译**，找到要验证的那个供应商，点它那一行右边的 **「测试」**。
2. 那一行的按钮立刻变成 **「中断」**，下面开始走秒：`测试中… 1.4 秒`。其他行的测试按钮临时变灰。
3. 不想等了就点 **「中断」**：界面立刻恢复，后台请求也会被真正掐断（不会偷偷跑完、不会占着连接）。
4. **最多 10 秒**必有结论，四种之一：
   - 绿 ✓ `连接成功 · 820 ms · 已降级请求形态 · 返回："…"`
   - 红 ✖ `连接失败` + 一眼看懂的中文原因（地址连不上 / 密钥不对 / 模型名不存在 / 被限流 / 超时）
   - 灰 ⏱ `已中断`
   - 黄 ？ `10 秒内未取得结论`——**不是失败**，可再点一次（本机模型首次加载慢时常见）
5. 测试期间应用照常工作：录音、字幕、翻译都不受影响（测试跑在自己的线程上，不占用识别线程，
   也不会改动你正在用的模型）。
6. 改了某一行的地址/密钥/模型名，那行的旧测试结果自动作废，重新点「测试」即可。

### 0.3 新增供应商怎么加（用户裁决 J/K）

- 编辑对话框顶部有 **「厂商预设」下拉**：OpenAI / DeepSeek / 智谱 GLM / Kimi / 通义千问 / 火山方舟 /
  本机 LM Studio / 本机 Ollama / 自定义。选一个，地址、推荐模型名、关思考姿态、币种自动填好，
  **你只需要贴自己的 API Key**。
- 出厂默认那条配置 = **DeepSeek**（首启后到翻译页把 API Key 贴上即可用）。

### 0.4 费用怎么算（用户裁决 G/H/I）

- 费用按 **本次运行累计**：换模型不清零、退出应用归零；不报用量的调用会让总额偏低，界面标「部分未知」。
- 币种**每个供应商各自一份**（DeepSeek/GLM/Kimi/通义/方舟 = 人民币，OpenAI = 美元；不填就按界面语言：
  中文=人民币、英文=美元）。界面按各笔自己的币种显示，两个币种分别累计、并列显示非零项。

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
| H2 | 零反馈：面板上唯一能看出"当前用哪个"的线索只是 `>>>` 前缀与加粗，没有状态行、没有"已生效"回执；且该线索随"点行即切换"一起被连带（用户裁决 E 后点行不再切换，线索更需显式化） | `translation.rs:64-78`（`model_row_text` 的 `>>>` 前缀） |
| H3 | 换模型时在队未翻译的段收到写死的红色失败回执「队列积压，保留最新（本段已放弃）」——**真实原因是切换了模型** | 回执文案：`pipeline.rs:87-98`；i18n：`assets/i18n/zh.yaml:528` `err_dropped`；字幕按失败色渲染：`crates/lt-ui/src/windows/subtitle.rs:530-537` |
| H4 | 不做语义验证：地址/密钥/模型名错要等后续每段翻译失败才暴露（URL 形态错有红字拦截） | `pipeline.rs:1744-1772`（构建失败发 `TranslatorUnavailable`） |
| H5 | 累计统计被清零：新装置自带全新 `TlStats`，`UpdateStats` 在 UI 侧整体覆盖 | `pipeline.rs:221-266`（per-rig）、`:738`（每装置新建）；`app.rs:1043-1055`（整体覆盖） |

### 1.3 测试覆盖缺口

`route_translator_switch`（`pipeline.rs:1732`）**无直接单测**——只有模拟"替换语义"的 `replaced_rig_workers_shutdown_on_drop`（`pipeline.rs:3241`）与 `retired_pool_emits_receipts_for_queued_jobs`；UI 侧"点行 → 发命令"亦无测试。

---

## 二、目标与非目标

**目标**

1. **两个界面各司其职**（裁决 E）：设置页 = 配置管理（增删改查 + 测试），切换使用中的模型 = 悬浮窗
   「模型」下拉；设置页的行点击不再产生任何切换副作用。
2. 连接测试**点哪测哪**：目标永远是用户点的那一行供应商配置，无静默回落、无副作用。
3. 测试**随时可中断**、**最多 10 秒出结论**（裁决 B），中断后界面 1 秒内恢复且后台请求真正被掐断。
4. 测试**不再卡死**：任何异常路径（命令丢失、线程死亡、回执超时）都必须收敛到终态。
5. 测试不依赖 ASR 线程，识别/翻译进行中也能测，且不干扰在进行的翻译。
6. 热切换可见（当前使用 + 已生效）、可控（下一段边界即生效）、可信（先测再切）。
7. 累计费用按**进程生命周期**累加（裁决 G），**分币种**（裁决 H/I），退出即归零。
8. 新增供应商**有官方参数可依**（裁决 J/K）：预设一键填充，出厂默认可直接用（贴 Key 即可）。

**非目标**（明确不做，避免范围蔓延）

- 不做"切换前自动探针"（换模型不得因网络而变慢，验证权交给用户）。
- 不做并发多探测（同一时刻至多一个在途探测）。
- 设置页不提供任何切换控件（裁决 E 的取舍，见 §八）。
- 不改 ASR 侧任何行为（引擎/模型/下载/零信任闸门）。
- 不改模型编辑对话框的既有字段校验逻辑（`config_warnings` 保持现状；仅新增预设与币种两个控件）。
- 不做跨进程持久的费用累计（重启必须清零）。

---

## 三、定案（A~K）

> 用户裁决的条目以 **【用户裁决】** 标注；其余为前两轮清单的推荐项定案。

| # | 议题 | **定案** | 备选与否决理由 |
|---|---|---|---|
| **A** | 测试入口 | **每行独立「测试」按钮**（列表行右侧），删除底部四按钮行中的测试按钮与"无选中回落活动模型" | 否决"单按钮 + 必须先选中"：测试目标必须显式（点哪行测哪行），不引入"没选中就不让测"的隐式规则 |
| **B** | 单次预算 | 【用户裁决】**整场测试 10 秒封顶**；单步超时 = `min(用户超时设置, 10 秒, 剩余预算)`；预算耗尽 → `Inconclusive`（**不是失败**，可重试） | 否决 60/30 秒与不封顶：用户要"立刻有结论"；代价是本地模型首次加载可能报"未定论"（重试即可，文案已说明） |
| **C** | 结果内容 | 成功：耗时 + 实际生效请求形态（降级时告知）+ 回执摘要（≤60 字符）；失败：分类中文原因 + 原始详情（次要行/悬停） | 否决"只显示 ✓/✖ + 耗时"（丢失诊断价值） |
| **D** | 并发 | **一次只测一个**：在途期间其他行的测试按钮禁用 | 否决并发（取消语义与结果归属复杂化） |
| **E** | **设置页职责** | 【用户裁决】**设置页的供应商列表只做配置管理**（新增/编辑/复制/删除 + 每行「测试」）；**行点击只选中，不再切换运行中的模型**；**切换当前翻译模型一律在悬浮窗（主界面）既有的「模型」下拉**完成 | 否决"设置页点行即切换"（原版语义）：会让"只想试试备用供应商"变成切走正在用的模型 |
| **F** | 热切换加固 | **做四项**：F1 命令消费点提前到每轮循环开头；F2 切换回执 `TranslatorSwitched` + 「当前使用」状态行（只读）+ 分组使用说明；F3 退休任务回执中性化（新 `FailureKind::Superseded`，字幕中性灰）；F4 管道未装配时切换回 `TranslatorUnavailable`（不再静默）。**另补** `route_translator_switch` 单测 | — |
| **G** | 统计口径 | 【用户裁决】**会话级累计**：`asr_n / tl_n / tokens / cost` 全部按本次运行累加，跨模型切换不清零，重启归零；费用按**每笔发生时的当时单价**累加 | 否决"仅费用累计、句数保持 per-rig"（会出现"费用在涨、句数只有 3"的自相矛盾显示） |
| **H** | 计价币种 | 【用户裁决】**每个供应商配置自带币种**：新增 `ModelConfig.currency: Option<String>`（`"cny"` / `"usd"`）；**不写 = 跟随界面语言**（中文→人民币，英文→美元），写了就按写的来 | 否决"全局单一币种"：国内厂商官方定价是人民币、国外是美元，强制一种会让另一种必须手工换算 |
| **I** | 费用显示 | 符号随**该笔费用所属配置的币种**（不再随界面语言）；价格输入行标注「元 / 1M tok」或「美元 / 1M tok」；会话累计**分币种两个账本**，界面显示非零项（如 `$0.0123 ¥0.0456`） | 否决汇率折算（需要汇率来源，且会引入不可解释的数字） |
| **J** | 厂商预设 | 【用户裁决】**做**：编辑对话框加「厂商预设」下拉，选中即填 `api_base` + 建议模型 id + 关思考姿态 + 币种；参数以**官方文档**为准（见 §十） | — |
| **K** | 出厂默认配置 | 【用户裁决】**默认供应商由"本机 LM Studio"改为 DeepSeek**（`api_base` / `model` / 关思考姿态 / 币种按 §十 官方文档核实值）；`api_key` 留空，用户首启在翻译页粘贴 | 保留 LM Studio 默认的旧行为已不适用：默认那条对本机没装 LM Studio 的用户永远不通，且报错（连接被拒）不如"缺 API Key"可执行 |

---

## 四、详细设计

### 4.1 契约变更（`crates/lt-proto/`）

**`PROTO_VERSION: 5 → 6`**（`lib.rs:21`；结构变更需递增）。

`src/lib.rs` 新增共享常量（跨层单一事实源：`lt-orchestrator` 探测预算与 `lt-ui` 看门狗同源，
且 `lt-ui` **不得**依赖 `lt-orchestrator`——架构 §3.1 白名单）：

```rust
/// 连接探测总预算（秒，用户裁决 B）。编排域据此设 deadline，UI 域据此设看门狗（+10s 裕量）。
pub const PROBE_TOTAL_BUDGET_SECS: u64 = 10;
/// 单步超时上限（秒）：用户超时更短时取用户值，更长时封顶（与总预算同值，故不构成额外限制）。
pub const PROBE_STEP_TIMEOUT_CAP_SECS: u32 = 10;
```

**会话统计的币种（用户裁决 H/I）**：`src/settings.rs` 的 `ModelConfig` 新增

```rust
/// 计价币种（"cny" | "usd"）。None = 跟随界面语言（中文→cny，英文→usd）。
/// serde 默认 + skip_serializing_if = Option::is_none：老档案不写该键 = 走默认。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub currency: Option<String>,
```

> 注：`every_settings_field_is_classified`（`settings_bus.rs:233`）**只走查 Settings 顶层键**，
> `ModelConfig` 内部字段不在其断言面——`currency` 是嵌套新增，**不需要**在该测试的两份清单里登记，
> 该测试也不会因此变红。

配套纯函数（UI 与编排域共用的唯一判据，放 `lt-proto`）：

```rust
/// 配置生效币种：显式值优先，否则按界面语言（"zh" → Cny，其余 → Usd）。
pub fn effective_currency(cfg_currency: Option<&str>, ui_lang: &str) -> Currency;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency { Cny, Usd }
impl Currency {
    /// 显示符号（"¥" / "$"）
    pub fn symbol(self) -> &'static str;
    /// 价格单位文案的 i18n 键（"price_unit_cny" / "price_unit_usd"）
    pub fn unit_key(self) -> &'static str;
}
```

`UpdateStats` 的费用字段改型（单币种 → 双账本）：

```rust
UpdateStats {
    asr_n: u64, tl_n: u64,
    prompt_tokens: u64, completion_tokens: u64,
    /// 本次运行累计（分币种两个账本；未发生的币种为 0.0）
    cost_usd: f64,
    cost_cny: f64,
    usage_known: bool,
},
```

（原 `cost: f64` 删除；消费点仅 `lt-ui/src/windows/overlay.rs` 与 `state.rs::OverlayStats`。）

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
/// 本函数**不写也不读**会话记忆（learned）与降级标记（degraded_notified）：
/// 探测是只读观察，既不改变生产装置的会话状态，也不受"生产已退到某台阶"的记忆
/// 影响——它总是从**配置推导出的起点形态**重跑完整阶梯（复验更彻底）。
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
7. 结论映射（**按表从上到下判定，命中即停**——顺序即优先级）：

   | # | 条件 | outcome | 附带字段 |
   |---|---|---|---|
   | 1 | `out.halted == Some(Halt::Cancelled)` **或** `matches!(out.attempt.error, Some(TranslateError::Cancelled))` | `Cancelled` | — |
   | 2 | `out.halted == Some(Halt::Budget)` | `Inconclusive { attempted: out.attempted }` | — |
   | 3 | `out.attempt.succeeded() && !正文.trim().is_empty()` | `Ok` | `step_note` = 起点形态则 `None`，否则 `Msg` 注入的降级说明；`preview` = 正文去换行后按**字符**截断 60 字符 |
   | 4 | 其余 | `Failed { kind, detail }`，其中 `kind = out.attempt.error.map(failure_kind).unwrap_or(Empty/Truncated)`（`verdict == Some(EmptyTruncated)` → `Truncated`，否则 `Empty`），`detail = ui_text()` 或体检描述 | — |

   > **第 1 行的第二个条件是必须的（易漏）**：取消可能发生在**一次尝试进行中**（流式读取返回
   > `Err(Cancelled)`），此时 `halted` 仍是 `None`（只有"两台阶之间"的取消才置 `halted`）。
   > 漏掉它，用户中断会显示成"连接失败 · 已中断"。测试 `probe_cancel_is_reported` 会钉住这条。

8. `sink.push(UiEvent::TestTranslatorResult { probe_id, name, outcome, ms: t0.elapsed().as_millis() as u64, step_note, preview });`

`step_note` 取值规则（无歧义）：最终成功台阶为 `RequestStep::Minimal` → `msg.t("probe_step_minimal")`；
为 `Plan(p)` 且 `p != 起点形态` → `msg.t("probe_step_degraded")`；等于起点 → `None`。

**日志（3 条，用户报障时的唯一凭据；不得记录 api_key）**：

```text
info  连接测试开始 #{probe_id}：{name}（{api_base} / {model}，单步超时 {step_timeout}s）
info  连接测试完成 #{probe_id}：{outcome 摘要}（{ms} ms，台阶 {step_name(out.step)}）
warn  连接测试中断 #{probe_id}：{Cancelled | Budget 耗尽，已尝试 N 步}
```
（第 1 条与第 3 条按情况择一；`api_base`/`model` 可记，`api_key` **禁止**入日志）

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

   - 签名：`pub(crate) fn run_ladder(base, start, allow_verdict_advance, text, source_lang, target,
     timeout, ctl: &RunCtl, sink, id, seq, push_partials) -> LadderOutcome`
   - **可见性**：`run_ladder` 现为 `pipeline.rs` 内私有 fn，`probe.rs` 需调用它 →
     `run_ladder` / `RunCtl` / `Halt` / `LadderOutcome` **四个都要改 `pub(crate)`**
     （`LadderOutcome` 现为私有 struct）；`run_attempt` 保持私有（探测只经 `run_ladder`）。
   - `LadderOutcome` 增字段 `halted: Option<Halt>` 与 `attempted: u8`
     （**计数口径**：每调用一次 `run_attempt` 就 +1，含截断补发的那次；首个台阶也算 1）。
   - **三个调用点全部要改**：`pipeline.rs:808`（生产 `submit_translation`）与 `:3051`（单测）
     传 `&RunCtl::none()`；`:1803` 的旧 `TlSwitch::TestTranslator` 路径随该变体一起删除
     （§4.5 已列）。
   - 每次尝试**之前**：`cancel` 置位 → 返回 `halted = Some(Cancelled)`；`deadline` 已过 →
     返回 `halted = Some(Budget)`。
   - 单次尝试超时：`let per = match ctl.deadline { Some(d) =>
     timeout.min(d.saturating_duration_since(Instant::now()).as_secs().max(1) as u32), None => timeout };`
   - 现有生产调用点（`pipeline.rs` worker 闭包内）改传 `&RunCtl::none()`，**行为不变**。
   - 截断补发重试的两次尝试都要走同一套检查（含在"每次尝试之前"内）。

### 4.5 lt-orchestrator：热切换加固（F1/F2/F3）

**F1 命令消费点提前**：把**主循环空闲分支**里的 `while let Ok(sw) = tl_switch.try_recv() { ... }`
整段（含该分支特有的 `ReplaceEngine` 处理，`pipeline.rs:2026-2130`）抽为函数并在两处调用。

```rust
/// 主循环侧的翻译器命令排空（F1 提取；待命循环**不**用本函数——那里 ReplaceEngine
/// 的处理是"装配 worker 退出待命"，语义完全不同，保持原样）
#[allow(clippy::too_many_arguments)]   // 本文件既有先例
fn drain_tl_switch(
    tl: &mut Option<Arc<TlRig>>,
    tl_switch: &crossbeam_channel::Receiver<TlSwitch>,
    manager: &mut AsrManager,
    current_display: &mut String,
    asr_unavailable_notified: &mut bool,
    interim_state: &mut InterimState,
    bus: &Arc<SettingsBus>,
    sink: &EventSink,
    sup: &Arc<Supervisor>,
    transcript: &Arc<lt_audio::transcript::TranscriptWriter>,
    msg: &Msg,
    learned: &Learned,
    degraded_notified: &Arc<Mutex<std::collections::HashSet<(String, String)>>>,
    current_settings: &lt_proto::Settings,
)
```

**签名为什么这么写（易踩）**：`run_asr_thread` 开头就把 `AsrThreadCtx`
**解构**成了局部变量（`let AsrThreadCtx { segment_queue, vad, interim, bus, stop, sink, tl_switch,
sup, transcript, msg, learned, degraded_notified } = ctx;`，`pipeline.rs:1871-1885`），
所以**不能**传 `&AsrThreadCtx`——必须逐个传引用（参数多，加 `#[allow(clippy::too_many_arguments)]`，
本文件已有同款先例）。

调用点：① 主循环 `while` 体**开头**（新增——使切换在下一段边界即生效，不再等 500ms 空窗）；
② 空闲分支（保留，覆盖空闲期的立即消费）。原空闲分支内的内联实现删除，只留调用。

> **别删错东西（重名警告）**：`pipeline.rs` 测试模块里有两个**同名但无关**的辅助函数
> `fn test_translator(disable_thinking: bool) -> Translator`（`:2819`）与
> `fn test_translator_explicit(style: &str)`（`:2828`），它们是阶梯单测的构造器，**必须保留**。
> 本次删除的是 `Pipeline::test_translator(&self, config)` 方法（`:1307`）与
> `TlSwitch::TestTranslator` 变体——按**路径 + 签名**定位，不要按名字搜删。

**F2 切换回执**：`route_translator_switch` 的 `ReplaceRig` 成功分支（`pipeline.rs:1756-1765`）在
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

**F4 管道未装配时切换不再静默**（复核补漏）：`shell.rs:239-244` 的 `Cmd::SwitchTranslator` 在
`self.pipeline` 为 `None` 时什么都不做，但 `publish_settings()` 与 `persist_settings()` 照常执行
（`ui_text` 而言即"设置说切了、实际没切"）。改为：

```rust
Cmd::SwitchTranslator(config) => {
    match self.pipeline.as_mut() {
        Some(p) => {
            p.switch_translator(&config);
            tracing::info!("Switching translator: {} ({})", config.name, config.model);
        }
        // 管道装配失败（pipeline_error 已在识别页红字）——翻译页也必须可见；
        // 复用现有 TranslatorUnavailable{ reason } 形态，不改契约结构
        None => self.artery.push(UiEvent::TranslatorUnavailable {
            reason: lt_i18n::t("translator_switch_no_pipeline"),
        }),
    }
    self.publish_settings();
    self.persist_settings();
}
```

新增 i18n 键 `translator_switch_no_pipeline`：zh「翻译管道未启动，模型切换未生效；请到「识别」页查看错误后重启应用。」
/ en "The translation pipeline is not running, so the model switch did not take effect. Check the errors on the Recognition tab and restart the app."

### 4.6 lt-orchestrator：会话级统计（G）

1. `TlStats`（`pipeline.rs:221-266`）改造：

   ```rust
   pub(crate) struct TlStats {
       asr_count: AtomicU64,
       tl_count: AtomicU64,
       prompt_tokens: AtomicU64,
       completion_tokens: AtomicU64,
       /// 累计金额的两个账本，单位 1e-9（整数累加，避免浮点原子）；币种按各笔调用
       /// 各自配置的 `effective_currency` 归账（裁决 H/I）
       cost_nano_cny: AtomicU64,
       cost_nano_usd: AtomicU64,
       /// 本次运行是否**出现过**用量未知的调用（true = 费用不完整）
       usage_unknown_seen: AtomicBool,
   }
   ```

   方法：
   - `fn new() -> Self`（无参——价格与币种都不属于累计器）
   - `fn record_translation(&self, pt: u64, ct: u64, usage_known: bool,
     prices: (f64, f64), currency: lt_proto::Currency)`：
     累加 tokens、`tl_count`，并按**本次调用的价格与币种**累加金额：
     `(*账本) += (compute_cost(pt, ct, prices.0, prices.1) * 1e9).round() as u64`；
     `usage_known == false` → `usage_unknown_seen.store(true)`。
   - `fn snapshot_event(&self) -> UiEvent`：
     `cost_cny = cost_nano_cny as f64 / 1e9`、`cost_usd = cost_nano_usd as f64 / 1e9`，
     `usage_known = !usage_unknown_seen.load()`。
2. **归属上移**：`Pipeline` 增字段 `session_stats: Arc<TlStats>`，在 `Pipeline::start` 创建一次
   （`pipeline.rs:1130` 附近），传给 `TlRig::from_effective`；`TlRig` 的 `stats` 字段保留但**不再自建**
   （`pipeline.rs:738` 的 `Arc::new(TlStats::new(...))` 删除）。装置持有 `Arc` 克隆，故该字段删掉也不影响
   生命周期；保留它是为了让"会话级"这一事实在类型上可见（若 clippy/编译器报未读，加
   `#[allow(dead_code)]` 并写明"持有即语义"）。
3. `TlRig` 增字段 `prices: (f64, f64)` 与 `currency: lt_proto::Currency`（构造期由
   `effective_currency(mc.currency.as_deref(), msg.lang())` 求得——**界面语言经 `Msg` 注入**，
   编排域不依赖 lt-i18n）。`crates/lt-orchestrator/src/lib.rs` 的 `Msg` 增第二个注入点：

   ```rust
   pub struct Msg { t: Arc<dyn Fn(&str) -> String + Send + Sync>,
                    lang: Arc<dyn Fn() -> String + Send + Sync> }
   impl Msg {
       pub fn new(t: impl Fn(&str) -> String + Send + Sync + 'static,
                  lang: impl Fn() -> String + Send + Sync + 'static) -> Self { … }
       pub fn lang(&self) -> String { (self.lang)() }
   }
   ```
   组合根唯一构造点 `shell.rs:152` 改为 `Msg::new(lt_i18n::t, lt_i18n::get_lang)`；
   `finish_ok` / `fail`（`pipeline.rs:547-620`）改为调用
   `stats.record_translation(pt, ct, attempt.usage_known, prices, currency)`，两参数由调用处透传。
4. `asr_count`、同语言路径的 `snapshot_event()` 调用点不变——计数器现在是会话级，天然满足"重启清零"。
5. UI 侧（`app.rs:1043`）仍整体覆盖 `OverlayStats`，但字段改双账本（`cost_usd` / `cost_cny`），
   并按 §4.9 的键补"部分未知"标记。

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

新增字段（`AppShell`）：`probe_cancel: Arc<AtomicBool>`（**在途探测的取消标志**，每启动新探测换成新实例）、
`probe_id: Arc<AtomicU64>`（**在途探测 id，0 = 无在途**）。

`fn start_probe(&mut self, config: ModelConfig, probe_id: u64)`：

1. **取代语义（supersede），不拒绝**：若已有在途（`self.probe_id.load(SeqCst) != 0`）→ 先置位**旧探测**的
   取消标志并记 `warn!("连接测试被新请求取代 #{old} → #{new}")`，然后照常启动新探测。
   **禁止**"在途时拒绝新请求"：旧线程退出可滞后一个轮询周期（流式 ≤150ms、非流式 ≤20s），
   拒绝会把用户"中断后立刻重测"这一正常操作变成最长 70 秒空等（只剩看门狗兜底）。
2. `self.probe_cancel = Arc::new(AtomicBool::new(false)); self.probe_id.store(probe_id, SeqCst);`
3. 经监督器一次性线程出生（镜像 `start_bench` / `spawn_device_probe` 范式，`shell.rs:375-425`）：

   ```rust
   let id_cell = self.probe_id.clone();
   self.sup.spawn(ThreadRole::TranslatorProbe, "lt-probe", Policy::Never, move || {
       let (artery, bus, cancel, msg, id_cell) = (...clone...);
       Box::new(move || {
           let _guard = ProbeExitGuard { id: id_cell, probe_id };  // 任何退出路径（含 panic unwind）复位
           lt_orchestrator::probe::run_probe(probe_id, &config, &bus, cancel, &msg, &artery);
       })
   });
   ```

   `Msg` 句柄：`AppShell` 增字段 `msg: lt_orchestrator::Msg`，在 **`AppShell::new`** 里构造一次
   （`Msg::new(lt_i18n::t)`）存字段；`start_pipeline` 改为复用该字段（不得在 `start_pipeline` 内构造
   ——探测可能发生在管道未启动时）。

4. `shell_helpers.rs` 增 **id 条件复位守卫**（与既有 `BenchActiveGuard` 同款纪律：任何退出路径复位）：

   ```rust
   /// 探测线程退出守卫：仅当在途 id **仍是自己**时归零——被新探测取代时不得
   /// 把新探测的 id 抹掉（否则随后的「中断」会被判为过期而失效）。
   pub struct ProbeExitGuard { pub id: Arc<AtomicU64>, pub probe_id: u64 }
   impl Drop for ProbeExitGuard {
       fn drop(&mut self) {
           let _ = self.id.compare_exchange(self.probe_id, 0, SeqCst, SeqCst);
       }
   }
   ```
5. **停机**：`AppShell::shutdown` 增加 `self.probe_cancel.store(true, SeqCst);`（在 `p.stop()` 之前），
   避免退出被在途探测拖住最多一个流式轮询周期（150ms）或一次尝试超时（非流式，≤10s）。

### 4.8 lt-ui：翻译页交互

**行点击语义变更（用户裁决 E，本节最关键的改动）**：`translation.rs:268-277` 现有的
"行选中 → 写 `settings.active_model` + 发 `Cmd::SwitchTranslator` + `mark_settings_dirty`"
**整段删除**，替换为只写 `panel.state.model_selected = Some(i);`（仅用于编辑/复制/删除的目标行）。
本页**不再产生任何模型切换副作用**。

保留不动（正确性需要，不是"设置页在切换"）：

| 场景 | 行为 | 原因 |
|---|---|---|
| 删除"正在使用"的那一行 | 删后按钳制规则修正 `active_model`，并内部发 `Cmd::SwitchTranslator` 让运行中的翻译器换到存活的那条 | 不换则运行装置指向已不存在的配置 |
| 编辑"正在使用"的那一行并确定 | 内部发 `Cmd::SwitchTranslator`（现有 `was_active` 分支保留） | 让运行中的翻译器用上新地址/密钥 |
| "当前模型的上下文数"直达行（`translation.rs:397-420`） | 保持指向**活动模型**并 600ms 重建 | 原语义（只影响正在跑的那个） |
| 悬浮窗「模型」下拉（`overlay.rs:369-402`） | 保持"选中即切换"（发 `SwitchTranslator` + `mark_settings_dirty`） | 这是**唯一**的用户可见切换入口（裁决 E） |

**切换入口的唯一性与边界（用户裁决 E + ①，2026-09-10 定案，明确写死）**：

- 用户可见的切换入口 = **悬浮窗第二行「模型」下拉**。设置页**不提供**任何切换控件（不加"设为当前使用"
  按钮、不加单选列）——用户明确选择"不加备用入口"。
- 悬浮窗**精简形态**下 `row2_checks` / `row2_combos`（含该下拉）整体不渲染
  （`overlay.rs:134-139`：`if !compact { … }`），悬浮窗整体隐藏时更不可达。
  **回到完整形态的两条路径（文档与产品提示都按此写）**：
  1. 点悬浮窗行 1 的 **「完整」** 按钮（i18n `mode_full`，`overlay.rs:249-261` 的 `toggle_mode`）；
  2. 右键**托盘**图标 → **「显示悬浮窗」**（`AppCommand::OverlayToggle`）。
  这是**有意接受**的边界，不在本方案内新增任何切换入口。
- 设置页列表上方的 `当前使用：XXX` 是**只读**展示，不承担切换职责。
- 该边界必须**在界面上自解释**：`models_group_hint` 的文案包含上面这两步（见 §4.9），
  否则用户遇到"精简形态下无处可切"时会当成 bug。

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

- **行归属判据（统一）**：某行"是本探测的行" ⟺ `probe.running.row == i` **且**
  `probe.running.cfg_key == 该行当前 cfg_key`。只比行号会在"探测期间删行导致行号前移"时把
  「测试中…」显示到别的供应商上；结果行同理（`result.row` + `result.cfg_key` 双校验）。
- 行按钮：文本、`>>>` 前缀、加粗、点击选中、双击编辑——**全部保持现状**。
- 右侧按钮两态（`is_my_row` = 上述判据）：
  - 非在途或 `!is_my_row`：标签 `probe_btn`（「测试」），`add_enabled(running.is_none(), ...)`；
    点击 → `id = probe.next_id; probe.next_id += 1;` → `probe.running = Some(ProbeRun{id, row: i,
    started: Instant::now(), cfg_key: 该行 cfg_key})` → 发 `Cmd::TestTranslator { config: 该行配置克隆,
    probe_id: id }`。
  - `is_my_row`：标签 `probe_cancel`（「中断」）；点击 → 发
    `Cmd::CancelTranslatorTest { probe_id: id }`，**立即**把该行结果落为
    `ProbeResult { outcome: Cancelled, ms: started.elapsed(), .. }` 并清 `running`（不等后台回执）。
- 行下附加行（`ui.add_space(2.0)` 后一行）：
  - `is_my_row` → `probe_running`（「测试中…」）+ 已耗时（`{:.1}s`）。**走秒刷新靠宿主节拍，
    不是 `ctx.request_repaint_after`**（见下方"走秒与看门狗的重绘来源"）。
  - `result` 有效（`result.row == i` 且 `result.cfg_key == 该行 cfg_key`）→ 按 outcome 渲染：
    - `Ok`：绿色 `pal.ok`，`✓ {probe_ok} · {ms} ms · {step_note?} · {probe_preview}`；
      `step_note` 与 `preview` 用 ` · ` 拼接，缺省项整段省略
    - `Failed{kind,detail}`：红色 `pal.err`，`✖ {probe_failed} · {ms} ms · {failure_text(kind)}`；
      `detail` 以次要行弱色显示（≤2 行省略）
    - `Cancelled`：弱色 `pal.weak`，`⏱ {probe_cancelled} · {ms} ms`
    - `Inconclusive{attempted}`：警示黄，`？ {probe_inconclusive} · {ms} ms · {probe_attempted}` +
      次要行 `probe_inconclusive_hint`
- **删除**四按钮行中的测试按钮（`translation.rs:345-372`）与旧结果块（`:374-393`）。

**插值写法（硬约束）**：`lt_i18n::t()` **只做查表，不做占位符插值**——全仓既有实践是调用点
`.replace("{x}", …)`（见 `app.rs:533-537`、`:795`、`:831`）。故 `probe_preview` 与 `probe_attempted`
两个带占位符的键，实现必须显式写：

```rust
lt_i18n::t("probe_preview").replace("{text}", &preview)
lt_i18n::t("probe_attempted").replace("{n}", &attempted.to_string())
```

其余结果行文案一律在代码里用 `format!` + ` · ` 拼接**完整句**（不要新增占位符键）。

**回执处理（`app.rs`）**：`UiEvent::TestTranslatorResult { probe_id, name, outcome, ms, step_note,
preview }`：

```text
若 panel.probe.running 为 None 或 running.id != probe_id → 丢弃（debug 日志），不改状态
否则：result = Some(ProbeResult{ id, row: running.row, cfg_key: running.cfg_key, name,
        outcome, ms, step_note, preview }); running = None; redraw(Panel)
```

`UiEvent::TranslatorSwitched { name, .. }` → `panel.state.active_model_note = Some((name, Instant::now()))`
→ `redraw(Panel)`。

> **"已切换生效"的显示/消失时机（不新增节拍）**：回执到达时主动 `redraw(Panel)` 一次 → 面板立刻
> 出帧并显示该提示；**过期清除发生在之后任意一次重绘**（鼠标移动/输入/其他节拍）——面板若一直没人碰，
> 提示会留在画面上直到下次重绘。这是可接受的（提示是锦上添花，`当前使用：X` 才是权威显示）；
> 真要做到"2 秒自动消失"就得为它单开节拍，**本方案不做**。

**走秒与看门狗的重绘来源（易踩，别用 `request_repaint_after`）**：本应用的窗口重绘由宿主的
**节拍机制**驱动（`TickKind` + `SessionView::schedule_tick` / `drain_due_ticks`，分派点
`app.rs:2394-2500` 的 `match tick.kind`），不是 egui 的自动重绘——用 `ctx.request_repaint_after`
在多窗口共享 Context 的布局里**不保证**该窗口按时出帧，于是"测试中… 3.4 秒"会僵在 0.0 秒、
看门狗也不会触发。因此新增一个节拍：

```rust
// state.rs 的 TickKind 增一枚
/// 连接探测的 100ms 走秒/看门狗节拍（仅面板；探测结束即停排班）
ProbeTick,
```

- **排班**：点击「测试」时 `session.schedule_tick(WinId::Panel, TickKind::ProbeTick,
  Instant::now() + Duration::from_millis(100))`。
- **分派**（`app.rs` 的 `match tick.kind` 增一臂 `TickKind::ProbeTick => self.on_probe_tick()`）：

  ```text
  on_probe_tick():
      若 panel.probe.running 为 Some(r):
          r.started.elapsed() > PROBE_TOTAL_BUDGET_SECS + 10 秒（预算 10s → 看门狗 20s）
              → 落 ProbeResult { outcome: Inconclusive { attempted: 0 }, .. }、清 running、
                 tracing::warn!("连接测试回执超时（命令可能未送达）")
          重新排班（now + 100ms）   // 自续拍：探测在途就一直走
          redraw(WinId::Panel)
      否则:
          不排班（自然停止，不留空转节拍）
  ```

  常量取 `lt_proto::PROBE_TOTAL_BUDGET_SECS`（**不得**从 `lt-orchestrator` 取，UI 无此依赖边）。
- 「中断」与回执到达时若 `running` 被清空，下一拍即自然停排班。

**状态行 + 使用说明（F2）**：模型配置 `group_card` 内、错误横幅之后、列表之前增加两行
（**均为只读**，不含任何切换控件——见本节"切换入口的唯一性"）：

```text
当前使用：{name}            // settings.active_model 对应行；越界时回落第 0 行
    + 若 active_model_note 新鲜（<2s）→ 追加 " · 已切换生效"（来自 TranslatorSwitched 回执）
<hint_line> models_group_hint    // 见 §4.9：点行=选中；切换去悬浮窗「模型」下拉；每行「测试」可验证
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
| `probe_inconclusive` | 10 秒内未取得结论 | No verdict within 10 s |
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
| `models_group_hint` | 点一行＝选中（用于编辑/复制/删除）；要换当前使用的模型请到悬浮窗的「模型」下拉里选——悬浮窗若是精简形态，先点它的「完整」按钮（隐藏了就右键托盘图标选「显示悬浮窗」）。每行右侧的「测试」可验证该供应商是否可用。 | Clicking a row only selects it (for Edit/Duplicate/Remove). To change the active model, use the model dropdown on the floating window — if it is in compact form click its "Full" button first (if hidden, right-click the tray icon and choose "Show overlay"). The Test button on a row verifies that provider. |
| `stats_partial_unknown` | 部分未知 | partial |
| `stats_partial_unknown_hint` | 本次运行中有调用未返回用量，金额可能偏低。 | Some calls this session returned no usage, so the total may be low. |
| `currency_label` | 币种 | Currency |
| `currency_cny` | 人民币（元） | CNY (¥) |
| `currency_usd` | 美元（$） | USD ($) |
| `currency_follow_lang` | 跟随界面语言 | Follow UI language |
| `price_unit_cny` | 元 / 1M tok | CNY / 1M tok |
| `price_unit_usd` | 美元 / 1M tok | USD / 1M tok |
| `preset_label` | 厂商预设 | Provider preset |
| `preset_custom` | 自定义 | Custom |
| `preset_deepseek` | 深度求索 DeepSeek | DeepSeek |
| `preset_openai` | OpenAI | OpenAI |
| `preset_zhipu` | 智谱 GLM | Zhipu GLM |
| `preset_moonshot` | Kimi（月之暗面） | Kimi (Moonshot) |
| `preset_qwen` | 通义千问（阿里云） | Qwen (Alibaba Cloud) |
| `preset_ark` | 火山方舟（字节跳动） | Volcengine Ark |
| `preset_lmstudio` | 本机 LM Studio | Local LM Studio |
| `preset_ollama` | 本机 Ollama | Local Ollama |
| `translator_switch_no_pipeline` | 翻译管道未启动，模型切换未生效；请到「识别」页查看错误后重启应用。 | The translation pipeline is not running, so the model switch did not take effect. Check the errors on the Recognition tab and restart the app. |

`status_active_model` 的 `{name}` 同样按调用点 `.replace("{name}", …)` 填充（同 §4.8 插值约束）。

**币种相关键的用法（无歧义）**：编辑对话框的价格行单位文本取
`lt_i18n::t(currency.unit_key())`；币种下拉三项 = `currency_follow_lang` / `currency_cny` /
`currency_usd`（选中项写回 `ModelConfig.currency`：`None` / `Some("cny")` / `Some("usd")`）；
费用显示符号取 `currency.symbol()`，**不再**按 `lt_i18n::get_lang()` 判断。
Preset 项的显示名取 `preset_*` 键（品牌名中英同形者两边写同一串）。

**G 在 MonitorBar 的落地（`crates/lt-ui/src/windows/overlay.rs`，两处判据都要改）**：

| 位置 | 现状 | 改为 |
|---|---|---|
| 约 `:547` token 段 | `!usage_known` → 显示 `stats_usage_unknown`（"—"） | `total_tokens == 0 && !usage_known` → "—"；否则显示**累计值**，并在 `!usage_known` 时于悬停保留 `stats_usage_unknown_hint` |
| 约 `:652` 费用段 | 仅 `cost > 0.0` 时渲染 `{¥/$}{:.4}`（符号按界面语言） | 依次渲染**非零账本**：`"{¥}{:.4}"`（若 `cost_cny > 0`）、`"{$}{:.4}"`（若 `cost_usd > 0`），两者以空格并列；符号取自 `lt_proto::Currency::symbol()`（**不再按界面语言**）；`!usage_known` 时在末尾追加弱色 ` · {stats_partial_unknown}` 并 `on_hover_text(stats_partial_unknown_hint)` |

理由（避免自相矛盾显示）：会话累计口径下 `usage_known` 表示"本次运行**出现过**用量未知的调用"，
若沿用旧的"未知即显示 —"，会出现"费用在涨而 token 显示 —"。既有测试需同步：
`crates/lt-ui/src/state.rs:2937-2943`、`crates/lt-ui/src/windows/overlay.rs:1080-1095`（含 `cost` 字段
拆分后的构造点）。

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

1. 同一时刻至多一个**有效**在途探测（UI 禁用其他行 + shell 取代语义 + 看门狗三重保证；
   被取代的旧探测可能仍在退出途中，其回执因 `probe_id` 不符必然被丢弃）。
2. 任何在途探测必在 ≤70 秒内落到终态（成功/失败/中断/未定论），**不存在永久「测试中…」**。
3. 探测不产生副作用：不切活动模型、不写 learned/degraded、不动 transcript、不进统计。

---

## 五、施工步骤（提交切分；每步提交前必须全绿）

| # | 提交 | 内容 | 收工前提 |
|---|---|---|---|
| C1 | `docs(translator-probe): 方案定稿（D-85）` | 本文件 + `docs/README.md` 索引行 | 四守护脚本过（docs 变更仅路径卫生相关） |
| C2 | `feat(probe-lt): lt-translate 可取消调用 + Cancelled 错误` | §4.2 全部 + §6.1 的 lt-translate 五项测试 | `cargo test -p lt-translate` 绿；clippy 零告警 |
| C3 | `feat(probe): 连接测试迁移到独立线程 + 设置页职责收窄（每行按钮/四态结果/E，D-85）` | §4.1 契约（probe 段）+ §4.3 probe + §4.4（params 单点 + `RunCtl` 签名改造，生产调用点传 `RunCtl::none()`）+ §4.7 shell + §4.8 UI（**含行点击语义变更 = 只选中**）+ §4.9 键（probe/status 段） | 全量测试绿 + 四守护 + clippy |
| C4 | `feat(hotswap): 切换回执/消费点提前/退休回执中性化（D-85/F）` | 本提交 = §4.5 四项（F1~F4）+ §4.8 字幕中性化 + 状态行与使用说明 | 全量测试绿 + 四守护 + clippy |
| C5 | `feat(stats): 会话级累计 + 双币种账本（D-85/G/H/I）` | §4.1 契约（`ModelConfig.currency` / `Currency` / `UpdateStats` 改型）+ §4.6 + §4.9 的 `stats_partial_unknown(_hint)` 与 `currency_*` 键（含 MonitorBar 两处判据改造与既有测试同步） | 全量测试绿 + 四守护 + clippy |
| C6 | `feat(providers): 厂商预设 + 默认供应商改 DeepSeek（D-85/J/K）` | §十 预设常量表 + 编辑对话框下拉 + `ModelConfig::default()` 变更 + `config_warnings` 预设地址豁免 + i18n `preset_*` 键 + §6.1 预设测试 | 全量测试绿 + 四守护 + clippy |

依赖关系：C2 独立可编译；C3 依赖 C2；C4 依赖 C3（共用 `TranslatorSwitched` 与 UI 状态结构）；
C5 依赖 C3（`UpdateStats` 改型会同时触及 probe 结果路径无关，但 `OverlayStats` 结构与其测试同名）；
C6 依赖 C5（预设要填 `currency` 字段）。
C3 是最大提交（契约 + 编排 + shell + UI 需同步改，否则旧调用点编译不过）——允许拆为
"C3a 契约 + 编排 + shell（旧按钮暂删）"与"C3b UI 每行按钮 + 行点击语义"两提交，但 C3a 必须在删除
旧 UI 路径的同一提交内保证可编译。

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

**lt-ui（C3/C4/C5/C6）**

| 测试 | 断言 |
|---|---|
| `row_click_only_selects_no_switch_cmd` | **裁决 E 回归**：headless 点击第 1 行 → `model_selected == Some(1)`，且**未发出任何 `Cmd::SwitchTranslator`**、`settings.active_model` 不变 |
| `row_context_menu_and_double_click_unchanged` | 双击行仍进编辑、四按钮（增/编/复制/删）目标 = 选中行（回归） |
| `delete_active_row_still_switches_internally` | 删掉活动行 → 仍发 `Cmd::SwitchTranslator`（正确性保留，与 E 不冲突） |
| `probe_row_button_sends_cmd_with_id` | headless 指针点击第 1 行「测试」→ 发出 `Cmd::TestTranslator{probe_id: 0}` 且 `running.row == 1` |
| `probe_cancel_button_sends_cancel_and_settles_locally` | 在途时点击 → 发 `CancelTranslatorTest` + 本地 `Cancelled` + `running == None` |
| `other_rows_disabled_while_running` | 在途时其他行按钮 `enabled == false` |
| `stale_probe_result_ignored` | `probe_id` 不匹配的回执不改变状态 |
| `probe_result_invalidated_on_config_change` | 改行配置后旧结果不渲染 |
| `probe_tick_self_reschedules_while_running` | 在途时调 `on_probe_tick()` → 能取到下一拍 `TickKind::ProbeTick`（自续拍） |
| `probe_tick_watchdog_settles_inconclusive` | `running.started` 拨回 21 秒前 → 调 `on_probe_tick()` → `running == None` + `Inconclusive{0}`，**且不再续拍** |
| `probe_tick_not_scheduled_when_idle` | 无在途时排班表里不存在 `ProbeTick`（不留空转节拍） |
| `active_model_status_line_renders_name` | 状态行文本含当前模型名；收到 `TranslatorSwitched` 后追加「已切换生效」 |
| `active_model_status_line_has_no_switch_control` | **裁决 E 回归**：状态行区域不含任何按钮/下拉（只读） |
| `superseded_subtitle_line_is_neutral` | `failed = Some(Superseded)` 的行文本为 `err_subtitle_skipped` 且 `fail_kind == Some(Superseded)` |
| `currency_symbol_follows_config_not_lang` | 同一份 `OverlayStats` 在 zh/en 界面下符号一致；`cost_cny>0` 显示 ¥、`cost_usd>0` 显示 $、两者皆非零并列显示 |
| `price_unit_label_follows_currency` | 币种下拉选「人民币」→ 价格行单位键 = `price_unit_cny`；选「跟随界面语言」且界面 zh → 同 cny |
| `preset_fills_all_fields` | 选「DeepSeek」→ `api_base` / `model` / `thinking_style` / `disable_thinking` / `currency` 与 §十 常量表逐字段一致 |
| `preset_never_overwrites_user_data` | 已填 `api_key` / `temperature` / 价格 / 非空 `name` 后再选预设 → 这些字段**一字不变**；`name` 为空时才被填入 |
| `preset_custom_fills_nothing` | 选「自定义」→ 五字段全部保持原值 |
| `preset_api_base_skips_v1_warning` | `api_base = "https://api.deepseek.com"`（与预设一致）→ `config_warnings` 不含 `cfg_warn_api_base_v1`；手填 `"https://foo.example"` 仍含该提示（回归） |

**lt-app（C3）**

| 测试 | 断言 |
|---|---|
| `cancel_test_only_matches_current_id` | 过期 id 不置位取消标志 |
| `second_test_supersedes_in_flight` | 在途时再发一条：旧探测取消标志被置位、`probe_id` 换新、新线程已出生（**不拒绝**） |
| `probe_exit_guard_keeps_newer_id` | 旧探测退出时不抹掉新探测的在途 id（`compare_exchange` 条件复位） |

**lt-proto / lt-orchestrator（C5/C6）**

| 测试 | 断言 |
|---|---|
| `effective_currency_follows_lang_then_explicit` | `None` + "zh" → `Cny`；`None` + "en" → `Usd`；`Some("usd")` + "zh" → `Usd`（显式优先） |
| `default_model_is_deepseek_preset` | `ModelConfig::default()` 的 `api_base/model/thinking_style/currency` == §十 DeepSeek 预设行；`api_key` 为空串 |
| `settings_without_currency_key_loads_as_none` | 老 `settings.json`（无 `currency` 键）反序列化后 `currency == None`（serde 默认兼容） |

### 6.2 实机走查清单（待用户，逐项勾）

1. **设置页点行不再切换模型**：点任意一行，`当前使用：X` 不变、悬浮窗下拉不变、正在跑的翻译不受影响。
2. **切换一律在悬浮窗**：悬浮窗「模型」下拉换一个 → 面板 `当前使用` 跟着更新并短暂显示「已切换生效」。
3. 每行「测试」：新增/复制/删除行后按钮仍在正确行、点击目标正确。
4. 测试中点「中断」：界面 ≤1 秒恢复；日志确认请求已被掐断。
5. 四态观感：绿 `✓ 连接成功`、红 `✖ 连接失败`（含中文原因）、灰 `⏱ 已中断`、黄 `？ 10 秒未定论`。
6. 新增供应商走厂商预设：选「DeepSeek」→ 地址/模型/币种自动填好，只需贴 Key；选「自定义」不覆盖手填内容。
7. 三种典型失败：错地址（连接失败）、错密钥（401）、错模型名（404）的文案是否可执行。
8. 测试期间正在翻译的字幕不中断、不丢句。
9. 换模型时字幕不再出现红色"队列积压"，而是中性"已跳过（切换模型）"。
10. 费用：换模型后继续累加、退出重启归零；用人民币供应商显示 ¥、美元供应商显示 $；两者都用过时并列显示。
11. 未提供用量的端点：费用旁出现「部分未知」。
12. 出厂默认配置：删掉 `settings.json` 后首启 → 翻译页是 DeepSeek 那条，贴 Key 即可用。
13. **裁决 ① 的边界自解释**：悬浮窗点「精简」→ 模型下拉消失；点「完整」能展开并切模型；
    点「隐藏」后再右键托盘「显示悬浮窗」能找回——全程不需要翻文档、不会误判成 bug。

### 6.3 守卫与门禁（每次提交前）

```powershell
powershell -File scripts/check_personal_paths.ps1
powershell -File scripts/check_deps.ps1
powershell -File scripts/check_guards.ps1
powershell -File scripts/check_dead_contract.ps1
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace      # 基线 540+9 → 本方案预计 +32 左右（C2~C6）
```

---

## 七、风险与回滚

| 风险 | 缓解 |
|---|---|
| 改动 lt-translate 流式读取循环影响生产翻译 | 生产路径 `cancel == None`，`slice == remaining` 与原实现逐字节等价；既有全量测试 + `no_cancel_token_keeps_legacy_timeout_semantics` 回归测试 |
| 契约改型破坏其他消费点 | Rust 编译期强制；`PROTO_VERSION` 同步升 6 |
| 探测线程拖住退出 | `Policy::Never` + `shutdown` 前置取消 + `join_all`；最坏等待 = 一次流式轮询（150ms） |
| 探测线程 / 命令链路异常导致 UI 无回执 | UI 看门狗（70 秒）兜底落终态——不依赖任何回执；shell 侧不拒绝新请求（取代语义），避免"取消后立刻重测"被阻塞 |
| 会话统计改造影响成产翻译统计 | `record_translation` 在既有 `finish_ok/fail` 调用点等价替换；`session_stats_accumulate_across_rig_replacement` 与既有统计测试双覆盖 |
| 新 `FailureKind` 触发死契约守卫 | §4.1 已列引用点 ≥2；若 WARN 按脚本先例登记 |
| 出厂默认配置改为 DeepSeek 影响老用户 | 仅影响"无 `settings.json` 或 models 为空"的全新用户；已有档案的 `models` 数组原样保留（`sanitize` 不触碰非空列表）。回滚 = 还原 `ModelConfig::default()` 一处 |
| 币种改动影响老档案 | `currency` 为纯新增可选字段（`serde(default)`）：老档案反序列化得 `None` → 跟随界面语言，与旧行为（按语言显示符号）**等价**，不产生迁移 |
| 设置页行点击语义变更（E）被误读为"功能退化" | 在分组内以 `models_group_hint` 明写"点行=选中；切换去悬浮窗下拉"；§6.1 有 `row_click_only_selects_no_switch_cmd` 回归测试钉住 |

**回滚**：按提交粒度 `git revert`；契约回滚需 `PROTO_VERSION` 回到 5（与 C3 同批回滚）。

---

## 八、不在本次范围（记录备查）

- 「切换前自动探针」：明确不做——换模型不得因网络变慢；验证权经每行「测试」按钮交给用户。
- **设置页不提供任何模型切换控件**（用户裁决 E + ① 的明确取舍，2026-09-10 定案"按①来"）：
  不加"设为当前使用"按钮、不加单选列、不加托盘模型子菜单。已知边界与回到完整形态的两条路径
  见 §4.8（点行 1 的「完整」按钮 / 托盘「显示悬浮窗」），并由 `models_group_hint` 在界面内自解释。
- 并发多探测、探测历史记录、探测结果持久化。
- ASR 侧（引擎/模型/下载/零信任闸门）任何改动。
- 模型编辑对话框的字段校验逻辑（`config_warnings`）改动（仅新增"厂商预设"下拉与"币种"下拉两处控件）。
- 费用跨进程持久化（重启必须清零）。
- api_key 加密落盘（登记备忘：如需提升可接 Windows DPAPI `CryptProtectData`，见 §九 N3）。

---

## 九、复核巡检的其余发现（登记，不在本次做）

> 2026-09-10 文档复核时对翻译子系统做的二次巡检，共 5 项：N1（无厂商预设）与 N2（费用单位口径）
> **已提升为裁决 J/K/I 并纳入本次**（详见 §3、§十）；其余三项登记如下。

| # | 发现 | 证据 | 处置 |
|---|---|---|---|
| **N3** | api_key 明文落盘（`settings.json`） | lt-models 设置落盘路径 | 登记备忘；本次不做 |
| **N4** | 探测"已验证"状态不落盘：重启后无从判断哪个供应商验证过 | — | 不做（与"结果随配置变更即失效"的设计一致性更高） |
| **N5** | 无「测试全部」按钮：多供应商时逐个点 | — | 不做（一次只测一个是定案 D） |

历史证据留档（N1/N2 的原始病灶，供实现者理解动机）：

- N1：`ModelConfig::default()` 原为"本机 LM Studio"（`http://127.0.0.1:1234/v1` +
  `hunyuan-mt-chimera-7b`，空密钥，`lt-proto/settings.rs:452-475`）；编辑对话框无预设入口
  （`translation.rs:596` 起 `editor_fields` 逐字段手填）。
- N2：`compute_cost`（`lt-translate/src/lib.rs:34-45`）对价格无币种概念；费用符号按界面语言
  （`overlay.rs:652-657` 原为 `get_lang()=="zh" ? "¥" : "$"`）；价格输入行无单位说明
  （`translation.rs:681-686`）；i18n `price_suffix`（`zh.yaml:253`）**从未被使用**。

---

## 十、厂商预设常量表（裁决 J/K）

**存放位置**：`crates/lt-proto/src/presets.rs`（纯新增文件 + 纯新增常量，冻结规则豁免评审）。

```rust
/// 一条厂商预设：选中后填入模型编辑对话框。
pub struct ProviderPreset {
    pub key: &'static str,        // "deepseek" —— 同时是 i18n 键后缀（preset_<key>）
    pub api_base: &'static str,
    pub model: &'static str,      // 建议模型 id（用户可改）
    pub thinking_style: &'static str, // THINKING_STYLES 之一
    pub disable_thinking: bool,
    pub currency: Option<&'static str>, // Some("cny") / Some("usd") / None 跟随界面语言
}
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[ /* 见下表 */ ];
```

**填充语义（无歧义）**：选中预设 → 覆盖 `api_base` / `model` / `thinking_style` / `disable_thinking` /
`currency` 五个字段。

**永不覆盖**：`api_key`（用户已贴的密钥）、`proxy` / `temperature` / `overrides` / `extra_body` /
`input_price` / `output_price` / `context_turns` / `streaming` / `json_response` 等其余字段。

**`name` 的特殊规则**：**仅当 `name` 当前为空**（新增配置的初始态）时填入预设的显示名；
已有非空名称**绝不覆盖**（用户改过名就保留）。

**选「自定义」** = 五个字段一个都不填（纯手工，与现状一致）。

**本地预设的 `model` 为空串**（LM Studio / Ollama 不知道用户装了哪个模型）：对话框确定按钮的
既有守卫（`name` 与 `model` 均非空才收，`translation.rs:570-572`）会拦住用户先填模型名——
这是预期行为，不需要额外校验。

**表格数值 = 官方文档核查结果**（2026-09-10 子代理逐家核对官方文档原文，URL 入表留痕）：

| key | 供应商 | `api_base` | `model`（可改） | `thinking_style` | `disable_thinking` | `currency` |
|---|---|---|---|---|---|---|
| `deepseek` | 深度求索 DeepSeek | `https://api.deepseek.com`（官方形态**不带 `/v1`**） | `deepseek-flash` | `"deepseek"` | `true` | `Some("cny")` |
| `openai` | OpenAI | `https://api.openai.com/v1` | `gpt-5.6-luna` | `"off"` | `true` | `Some("usd")` |
| `zhipu` | 智谱 GLM | `https://open.bigmodel.cn/api/paas/v4` | `glm-4.6` | `"deepseek"` | `true` | `Some("cny")` |
| `moonshot` | Kimi（月之暗面） | `https://api.moonshot.cn/v1` | `kimi-k2.6` | `"deepseek"` | `true` | `Some("cny")` |
| `qwen` | 通义千问（阿里云百炼） | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen3.5-flash` | `"qwen"` | `true` | `Some("cny")` |
| `ark` | 火山方舟（字节跳动） | `https://ark.cn-beijing.volces.com/api/v3` | `doubao-seed-2-0-lite-260215` | `"deepseek"` | `true` | `Some("cny")` |
| `lmstudio` | 本机 LM Studio | `http://localhost:1234/v1` | `""`（用户填本机已加载模型） | `"openai"` | `true` | `None` |
| `ollama` | 本机 Ollama | `http://localhost:11434/v1` | `""`（用户填 `ollama list` 的 id） | `"openai"` | `true` | `None` |
| `custom` | 自定义 | —（不填充任何字段） | — | — | — | — |

**填这些值的依据（逐条，实现者不必再查）**：

- **`thinking_style` 列即"官方能接受且我们已支持"的姿态**，全部经官方文档确认不会 400：
  - DeepSeek / GLM-4.6 / Kimi-k2.6 / 方舟：官方参数 `thinking:{"type":"disabled"}` ↔ 我们的
    `NestedDisabled`（`thinking_style = "deepseek"`）。
  - 通义：官方 `enable_thinking: false` ↔ 我们的 `EnableThinkingFalse`（`"qwen"`）。
  - LM Studio / Ollama：官方（Ollama 明确、LM Studio 缺载但本仓库实测有效）
    `reasoning_effort: "none"` ↔ 我们的 `ReasoningEffortNone`（`"openai"`）。
  - **OpenAI 用 `"off"`（= 不发送任何关闭参数）**：`reasoning_effort: "none"` 在 OpenAI 是
    **model-dependent**（部分新模型不支持 `none`，传了直接 400），保守不发是唯一稳的选择；
    用户在「关闭方式」里显式选 `openai` 才会发（该选项仍在）。
- **八家地址全部命中 `crates/lt-translate/src/thinking.rs` 既有路由表**（`deepseek` / `api.openai.com` /
  `bigmodel` / `moonshot` / `dashscope`+`aliyuncs.com` / `volces` / 回环 `LOCAL_HOSTS`），
  **无需为预设新增任何路由关键词**——预设与自动路由不会互相矛盾。
- **价格一律留 0，不写进常量表**：各站价格随时变（且国内站人民币、国际站美元），写死会误导；
  用户在自己供应商的定价页照抄即可，币种由 `currency` 字段决定。
- **`temperature` 保持 `None`（不发送）**：Kimi 全系对 `temperature`/`top_p` 传入非官方值**直接报错**，
  DeepSeek 思考模式忽略该参数——"默认不发送"是唯一通用安全解（与既有裁决一致，预设**不预填温度**）。

**出厂默认配置（裁决 K）**：`ModelConfig::default()` 改为该表 **DeepSeek** 行 + `name: "DeepSeek"` +
`api_key: String::new()`；其余字段沿用现有默认（`streaming: true`、`json_response: false`、
`temperature: None`、`context_turns: 0`、价格 0）。原"本机 LM Studio"退化为预设项之一。
空 `api_key` 首启会得到 401 中文提示「请检查 API Key 是否正确」——可执行，优于旧默认的连接被拒。

**附带改动（1 处，避免预设一选就报警）**：`config_warnings` 的 `/v1` 软提示
（`cfg_warn_api_base_v1`，`translation.rs:has_version_segment`）对**已知供应商的官方地址**放行：

```rust
// 地址与任一预设的 api_base 完全一致 → 该厂商自有路径，跳过"缺 /v1"提示
if lt_proto::PROVIDER_PRESETS.iter().any(|p| p.api_base == base) { /* skip */ }
```

理由：DeepSeek 官方地址就是不带 `/v1` 的 `https://api.deepseek.com`，预设填完立刻弹黄条会被当成 bug；
白名单来自预设表本身，不需要额外维护一份域名清单。

---

## 附录 A：多供应商热切换链路现状（已实现部分，供对照）

| 环节 | 状态 | 位置 |
|---|---|---|
| 面板点行切换 | ✓（现状；**本方案按裁决 E 移除**，改为只选中） | `translation.rs:268-277`（写 `active_model` + 立即发 `Cmd::SwitchTranslator`） |
| 悬浮窗模型下拉切换 | ✓ | `crates/lt-ui/src/windows/overlay.rs:369-402` |
| 命令路由 | ✓（pipeline 为 None 时静默丢弃——**本方案由 §4.5 F4 补上可见回执**） | `shell.rs:239-244` |
| 线程命令 | ✓ | `pipeline.rs:1278-1284` → `TlSwitch::ReplaceRig` |
| 真实重建 + 失败保留旧装置 | ✓ | `pipeline.rs:1744-1772` |
| 旧装置回收（retire + 池自停） | ✓ | `pipeline.rs:1744-1761`、`:206-210` |
| 逐段读当前装置 | ✓ | `pipeline.rs:2204`（`commit_text(..., tl.as_deref(), ...)`） |
| 落盘 / 重启恢复 | ✓ | `mark_settings_dirty` → 300ms → `ApplySettings`；启动 `TlRig::from_settings`（`pipeline.rs:656`） |
| 会话学习记忆跨切换保留 | ✓ | `learned` 按 `(api_base, model)` 存、`Arc` 共享（`pipeline.rs:617-619`） |
| 对话上下文 | 有意重置 | 新 Translator 历史为空（与原版重建语义一致） |
