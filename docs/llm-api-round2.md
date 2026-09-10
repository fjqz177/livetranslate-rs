# LLM 翻译接口层第二轮评审与修复（2026-09-10）

> **状态**：**已实施**（本文 = 评审结论 + 用户裁决 + 施工记录，三者同源）。
> **与 `llm-api-review.md` 的关系**：那份是本轮改造**之前**的评审（触发事件 = LM Studio
> 空译文故障），其 P1/P2 清单驱动了 W1~W5 五个提交。本文是改造**之后**的第二轮
> 深度评审——范围 = 已落地代码的实测复核 + 国内四家厂商官方文档查证 + 用户逐条裁决。
> **偏差登记**：本轮修复自 **D-82** 起（上一条 D-81 为 E6 死契约清理）。

---

## 一、第二轮评审结论（代码侧）

方法：两个只读子代理并行审计（`crates/lt-translate/**` 核心 / 编排+UI+契约集成面），
主线程对全部承重结论逐条回读源码或**实测复现**，另对四家厂商官方文档实地查证。

### 1.1 实证复现的缺陷（主线程亲测）

**R2-1 思维链隔离器溢出泄漏（INV-F 破口）**：`reasoning.rs` 的 `push` 最多跑 8 步
`step()`，而一个配对块要占 2 步——单次喂入 ≥5 个块（非流式整段、或服务端一次下发
整段）时第 5 个块未处理完，`finish()` 却把剩余 pending 当正文交出。实测输出：
`"<think>SECRET5</think>译文"`。9 个游离闭标签同样溢出。

**R2-2 「上下文数」功能整体失效（W3 回归）**：`pipeline.rs` 每次提交都调
`Translator::with_overrides`，其内部走 `share_client()` **新建空白 `MutableState`**
（`context_turns: 0`、history 空）——`set_context_turns` 设在了从不参与请求的实例上。
高级区「上下文数」（0–20）**从未生效**，且无测试发现。

**R2-3 隐藏覆写键反杀可见设置**：`build_request_body` 先插一等 `temperature`/`max_tokens`，
**再插 `OVERRIDE_KEYS`**（含同名键）→ 覆写优先；而 UI 已不再渲染 temperature / max_tokens /
seed 三行，编辑器状态仍原样回写。后果：老档案里的 `max_tokens: 256` 会**原样复活**本次
故障的根因，"不再发送 max_tokens"的修复对该档案完全无效。

**R2-4 自愈链越权**：`heal_translator` 不看用户意愿——取消勾选（`disable_thinking=false`）
后遇到空回复仍会自行注入关闭参数并写入会话记忆；用户显式指定方式也会被链推进推翻。
测试 `heal_disabled_when_switch_off` 的注释（"不得擅自开启关闭参数"）与断言（要求开启成功）
自相矛盾。

### 1.2 集成面与错误面（子代理结论 + 主线程复核）

| 编号 | 缺陷 | 证据 |
|---|---|---|
| R2-5 | **参数被拒无降级**（方案 §2.3.2 标注"必需"却零实现）：默认勾选关闭思考 + 未知端点盲发 `reasoning_effort:"none"` → 服务端回 400 时**每段翻译都失败**，界面只有"翻译服务返回错误" | 全仓 `rg '400'` 仅命中注释 |
| R2-6 | **Moonshot/Kimi 完全不在路由表**：`api.moonshot.cn` / `api.moonshot.ai` 四个常量表都不匹配 → 落到"未知端点"；且 Kimi 官方把 `temperature` 锁死（"传入其他值报错"），而应用默认发 0.3 → **一个请求都发不出去** | `thinking.rs` 常量表；官方文档（见 §三） |
| R2-7 | **严格枚举解码打死整段响应**：async-openai 0.41.3 的 `FinishReason` 只有 5 个变体且无 `serde(other)`——DeepSeek 文档列出的 `insufficient_system_resource` / `aborted` 会让整个响应解码失败，判定"未知原因" | `chat_.rs:1025-1031` |
| R2-8 | **非 `{"error":{…}}` 形状的错误体丢失 HTTP 状态码**：async-openai 只认嵌套 `error` 键，平铺 `{code,message}`（阿里文档形状）或网关 HTML 会退化为"未知原因"，401/404/429 三条中文提示全部失效 | `client.rs:759-765`（`WrappedError`） |
| R2-9 | **无用量端点误导**：服务端不返回 usage 时浮窗 Tok 恒 0，且 `reasoning_tokens` 永远为 None → "预算被思考吃光"不可达 | `translator.rs` / `overlay.rs` |
| R2-10 | **队列丢任务无回执**：`JobPool` 满丢最旧，被丢任务无任何事件 → 字幕永远停在"翻译中" | `pipeline.rs` |
| R2-11 | **partial 事件 O(n²)**：每个增量推一次"累积全文"快照 | `pipeline.rs` |
| R2-12 | **规则 4/5（偏离可见）未落地**：重建失败没有"仍在使用 <旧装置>"，自愈/降级/截断全部只进日志 | 方案 §2.5 规则 4/5 |
| R2-13 | 基准面完全未接入改造：自建请求体仍硬编码 `max_tokens:256`、不过隔离器，思考模型上 TTFT 失真且思考原文进基准页 | `bench.rs:162-222` |
| R2-14 | 死代码与失信文案：`last_usage` 无人读；`err_stream_interrupt` 承诺的"重试 3 次/暂停/横幅"三项功能都不存在 | 全仓引用计数 |

---

## 二、用户裁决（2026-09-10，逐条锁定）

| # | 议题 | 裁决 |
|---|---|---|
| 1 | 上下文数失效 | **完全修复**（前端 UI + 后端逻辑一并） |
| 2 | 隐藏覆写键 | **清理掉** |
| 3&4 | 参数被拒 / 强制思考模型 | 回退阶梯必须能退到**最小请求**（只发模型 + 系统提示词 + 待译文本 + 流式；不带任何用户配置字段），思考内容由隔离器兜底 |
| 5 | 关不掉思考的模型 | 阶梯最终成功关闭 → 不动；退到"关不掉"形态 → **UI 帮用户取消勾选 + 文字提示"该模型无法关闭思维链"**，后端按实际能跑通的配置工作 |
| 6&7 | 思维链隔离 | "一定要处理好，绝不能让思维链漏到译文里" |
| 8&9 | 错误面 | 该报错的必须报错；**用户要在字幕里看得到**，且渲染效果与正常译文**明显不同**（UI 配合） |
| 10 | 无用量端点 | 自行处理好 |
| 11 | 偏离可见（规则 4/5） | **用户表示暂不理解 → 本轮不做，留给下一轮解释后再定** |
| 12 | 基准页 | 自行处理好 |
| 13 | §9 遗留四项（校验/预览/快照/丢任务回执） | 自行处理好 |
| 14 | 清理包 | 自行处理好 |
| 15 | 文档 | **独立一份文档，与上一份 review 区分**（即本文） |
| 附 | 高级参数政策（新） | **默认一律不发送**（现在的模型没必要发；除非用户手动指定）——温度默认 `None` |
| 附 | 关不掉的标记是否落盘 | **写进设置**（按模型持久化） |
| 附 | 最小请求的边界 | **保留系统提示词**（模型必须知道自己在翻译） |

---

## 三、国内四家厂商官方文档查证（2026-09-10 实地抓取）

| 项 | DeepSeek | GLM（智谱） | Qwen（百炼） | Kimi（月之暗面） |
|---|---|---|---|---|
| 关闭思考 | `thinking:{"type":"disabled"}` | `thinking:{"type":"disabled"}`（GLM-4.5+） | `enable_thinking:false`（**顶层**非标准参数） | `thinking:{"type":"disabled"}`（**仅 kimi-k2.6**） |
| 关不掉的 | — | **GLM-5.3 / 5.3-FLASH / 4.7 / 4.5V 强制**（"returns an error if disabled"） | qwen3-235b-thinking / qwq / qwen-mt-* | **kimi-k3**（只认 `reasoning_effort: low/high/max`，"不应传 thinking"）；k2.7-code 传 disabled 会报错 |
| `temperature` | 0–2，思考模式忽略不报错 | 文档自相矛盾（API 参考 `[0,1]` vs 兼容页 `(0,1)`，0 不适配） | `[0,2)`，官方"不建议 0" | **锁定**：k3 固定 1.0、k2.6 思考 1.0/非思考 0.6，**传入其他值报错**，"建议不要显式传入" |
| 其它锁定参数 | top_p 非思考固定 1.0 | — | — | `top_p` 0.95、两个惩罚项 0，改值报错 |
| 不发 `max_tokens` | 默认 8K（非思考）/64K（思考） → 安全 | 默认 65536 → 安全 | 默认 = 模型最大输出 → 安全 | K3 默认 131072 → 安全 |
| `stream_options.include_usage` | 支持（无独立 usage chunk，挂末块） | **文档未见**（但末块本就带 usage） | 支持 | 支持（另发一个 `choices` 为空的 usage-only chunk） |
| 推理字段 | `reasoning_content` | `reasoning_content` | `reasoning_content` | `reasoning_content` |
| base_url | `https://api.deepseek.com` | 国内 `open.bigmodel.cn/api/paas/v4`；国际 `api.z.ai/api/paas/v4` | 兼容模式已迁 `{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`（旧 dashscope 域名仍可用） | 必须带 `/v1`；**国内/国际两套密钥不通用** |

**结论**：① 应用"不发送输出长度上限"的决定在四家都安全；② 四家推理内容都走独立字段，
`content` 才是正文（应用只读 `content`，安全）；③ 唯一的结构性风险是**参数被拒**
（强制思考模型对关闭参数直接报错、Kimi 对锁定参数直接报错）——这正是本轮"回退阶梯 +
高级参数默认不发送"要解决的；④ "错误文案里带参数名"不可靠（官方都不承诺，GLM 的模板
里是未替换的 `${field}` 占位符）→ 降级判据必须用 HTTP 状态码。

---

## 四、施工内容（本轮）

### 4.1 lt-translate（核心）

- **隔离器加固**（R2-1）：`push` 改为"跑到没有进展为止"（每步严格消费 pending，必然终止）；
  `finish` 丢弃未成形残尾；新增"游离闭标签在开标签之前"的判定（`step` ② 先比位置）。
  新增 4 条实证回归测试（5 配对块 / 9 游离闭标签 / 前置游离 / 残尾不 flush）。
- **回退阶梯**（裁决 3&4/5）：`ThinkingPlan` 链尾补 `None`（= 不发送任何推理参数，即"认输"）；
  新增 `RequestStep { Plan(ThinkingPlan), Minimal }` + `next_step`——四形态 → 不发送 →
  **最小请求**（清空温度/覆写/额外参数/上限，只留模型 + 系统提示词 + 文本 + 流式）。
  `resolve_thinking_plan` 增 `unavailable` 参数（持久化的"关不掉"标记 → 直接不发）。
- **Kimi/Moonshot 入路由表**（R2-6）：模型名含 `kimi`/`moonshot` 或端点含 `moonshot`/`kimi`
  → 嵌套体（官方 k2.6 形态）；k3/k2.7-code 靠阶梯退级并标注。
- **会话记忆共享**（R2-2）：`share_client` 不再新建 `MutableState`——派生副本（阶梯每级一个）
  **共享上下文记忆**；用量账本早已随迭代器返回，共享态只留 `context_turns` + `history`。
- **历史按提交序号**（方案 §4.6）：`history` 改 `BTreeMap<u64,_>`，上下文只取**序号更早**的
  句子（并发 8 worker 下不再引用"未来句"）；`translate_iter` 增 `seq` 形参。
- **高级参数默认不发送**（新裁决）：`TranslatorParams.temperature` 默认 `None`；
  构造期剔除已撤出界面的覆写键（第二道闸，第一道在 `Settings::sanitize`）。
- **宽容响应类型**（R2-7）：新增 `wire` 模块（只声明要用的字段、`finish_reason` 保持字符串），
  用 `create_byot::<Value, wire::ChatChunk>` 替换 async-openai 的严格类型——陌生取值不再
  打死整段响应，同时消除 `service_tier` 等同类风险。
- **错误体宽容解析**（R2-8）：`JSONDeserialize` 分支从任意形状的错误体（嵌套 / 平铺 /
  字符串码 / 非 JSON 文本）挖出 (状态码, 消息)，保住 401/404/429 的中文提示与
  `FailureKind` 分类；详情截断 300 字符，不再把整块 JSON 倒给用户。
- **用量未知可回执**（R2-9）：`TranslateStream::usage_known()`。
- 删除 `Translator::last_usage()`（死 API，INV-B 收尾）与 `verdict.rs` 的 async-openai
  `From` 实现（改走 `finish_kind(Option<&str>)`）。

### 4.2 lt-proto（契约，加法豁免）

- `ModelConfig`：新增 `thinking_unavailable: bool`（默认 false，仅 true 写盘）；`temperature`
  默认改 `None`（`skip_serializing_if = Option::is_none`）。
- `Settings::sanitize`：**一次性迁移**清除 `overrides` 里已撤出界面的三个键
  （temperature / max_tokens / seed），空表折叠为 `None`，清理留痕。
- `UiEvent`：新增 `TranslatorDegraded{name, api_base, model, actual, cannot_disable_thinking}`
  （item 5 回执）；`UpdateStats` 增 `usage_known`。
- `FailureKind`：新增 `Dropped`（⑬d）+ i18n 键 `err_dropped`。
- 新增界面仍在渲染的键清单 `OVERRIDE_KEYS_VISIBLE` 与已撤下的 `OVERRIDE_KEYS_HIDDEN`。
- i18n（zh/en 同步）：`err_dropped` / `err_subtitle_label` / `stats_usage_unknown(_hint)`；
  `thinking_style_deepseek` 标签补 Kimi；`err_stream_interrupt` 改写（删掉不存在的功能承诺）。

### 4.3 lt-orchestrator（编排域）

- **阶梯驱动**：`heal_translator` → `run_ladder`（成功即停、跨尝试累计用量、补发上限只做一次）；
  `should_advance` 判据 = **400/422 参数被拒**（总是退级）或**体检 EmptyReasoningBudget**
  （仅当用户没显式指定方式且没取消勾选时退级）。
- **记忆升级**：`learned` 改存 `RequestStep`，**失败也记**（记"退到底的台阶"，避免每段重走
  整条阶梯）；`degraded_notified` 保证"关不掉"回执每会话每模型一次。
- **丢任务回执**（R2-10）：`JobPool` 队列元素改 `TlJob`，`Drop` 时若未被消费且非停机中
  → 补 `TranslationFailed{kind: Dropped}`。
- **partial 节流**（R2-11）：增量推送 ≥50ms 一次（最终译文仍由 `UpdateTranslation` 送达）。
- **测试连接**吃同一条阶梯（判据与生产一致；`push_partials=false` 不留幽灵消息）。
- 提交序号：`TlRig.seq` 单调递增（id 是 UUID，不能当序号）。

### 4.4 UI

见下节"实施状态"——item 5（取消勾选 + 提示 + 落盘）、item 8/9（字幕报错醒目样式）、
用量未知显示"—"、温度默认不发送、配置校验与请求体预览。

---

## 五、实施状态（2026-09-10 收口）

**测试基线**：`cargo test --workspace` = **516 通过 / 7 ignored / 0 失败**（本轮前 483+7，
净增 33）；`cargo clippy --workspace --all-targets -- -D warnings` **零告警**；
四守护脚本（个人路径 / 依赖白名单 / 源码禁令 / 死契约）**全过**（108 契约变体，0 死）。

**逐条落地**：

| # | 裁决 | 落地 |
|---|---|---|
| 1 | 上下文数完全修复 | ✗ 派生副本共享会话记忆 + 历史按提交序号（并发不取"未来句"）+ 3 条回归测试 |
| 2 | 隐藏覆写键清理 | ✗ 档案一次性迁移（`sanitize`）+ 构造期第二道闸 + 2 条测试 |
| 3&4 | 回退阶梯退到最小请求 | ✗ 四形态 → 不发送 → 最小请求；400/422 驱动 + 体检驱动；Kimi 入路由表 |
| 5 | 关不掉 → UI 取消勾选 + 提示 | ✗ `UiEvent::TranslatorDegraded` + `thinking_unavailable` 落盘 + 编辑器提示（重勾即清除） |
| 6&7 | 思维链绝不漏 | ✗ 隔离器去步数上限 + 收尾守卫 + 前置游离闭标签；4 条实证回归测试 |
| 8&9 | 报错可见且与译文可辨 | ✗ 字幕窗失败态类型化（`failed: Option<FailureKind>`）+ 警示红 `#FF6B5C` + `⚠ 翻译标签`；悬浮窗同源；401/404/429 分类经宽容解析保住 |
| 10 | 无用量端点 | ✗ `usage_known` 贯通（迭代器 → 统计事件 → MonitorBar 显"—"） |
| 12 | 基准页 | ✗ 复用同一请求构造 + 隔离器；TTFT 只认首个**可见**增量 |
| 13 | 遗留四项 | ✗ ⑬b 就地校验 / ⑬d 丢任务回执；⑬a 请求体预览与 ⑬c 快照格见"有意偏离" |
| 14 | 清理包 | ✗ `last_usage` 死 API 删除 / `err_stream_interrupt` 假承诺改写 / 供应商名进日志降 debug / docs 索引补两份 LLM 文档 |
| 11 | 规则 4/5 偏离可见 | **未做**（用户要求先解释再定，见下） |
| 附 | 高级参数默认不发送 | ✗ 温度默认 `None`；编辑器默认未勾选 |
| 附 | 关不掉标记落盘 | ✗ `ModelConfig.thinking_unavailable`（按模型持久化） |
| 附 | 最小请求保留系统提示词 | ✗ |

**有意偏离 / 未做**（诚实登记）：

1. **⑪（规则 4/5「偏离可见」）未做**——用户明确表示需先解释清楚再决定；本轮不含半成品。
   现状：降级/补上限仍只进日志；"仍在使用 <旧装置>" 的失败提示未做。
2. **⑬a（请求体预览）未做**——需新增 `Cmd::PreviewRequest` + `UiEvent::RequestPreview` 并经
   编排域构造（分层规则所限 lt-ui 不能直连 lt-translate），本轮未纳入；"预览==实发"由
   **基准页复用同一构造函数**部分兑现（基准与生产同源，已加回归测试）。
3. **⑬c 降级实现**——未引入 ArcSwap 快照格（跨 crate 共享成本高），改在生产者侧**节流**
   （≥50ms 一次）。队列容量压力显著缓解，但单条翻译的字节量仍随文本增长；若后续仍见
   动脉丢事件，再按 D-67 快照格模式升级。
4. **Kimi k3 / GLM-5.3 等强制思考模型**：官方无法关闭 → 阶梯退到"不含关闭参数"并在界面
   取消勾选 + 提示；这是**设计内的最终态**，不是缺陷。
5. **`err_subtitle_label` 只给标签不给原因**：字幕窗空间有限，具体原因在悬浮窗/日志；
   若要字幕带原因需补一套短文案键（未做）。

**新增测试（节选）**：隔离器 4 条实证回归 / 阶梯 6 条（含"取消勾选绝不注入""显式方式不被
体检驱动"）/ 宽容解码 3 条 / 错误体 4 条 / 上下文共享 2 条 / 丢任务回执 1 条 / 降级回执与
编辑器往返 3 条 / 字幕失败态 2 条 / 基准同源 1 条。
