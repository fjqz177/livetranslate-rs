# LLM API 子系统深度评审（2026-09-10）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> 状态：**内部评审报告，未提交**。触发事件 = 用户本机 LM Studio（`http://127.0.0.1:1234`）跑 `qwen3.5-4b-mtp` 时"译文全部显示 `(相同语言)`"的实机故障。
>
> 方法：四维度并行评审（请求构造/wire 协议与安全、流式与并发生命周期、提供商适配矩阵、失败面与用户可见结果），主线程对全部 P1 与关键 P2 逐条回读源码复核。**核实分级**：`[核实·主线程]` = 亲自读过代码或实测；`[核实·子代理]` = 子代理给出行号且与主线程已知事实一致；`[推测]` = 未实机验证的推演。
>
> 范围：`crates/lt-translate/**`、`crates/lt-orchestrator/src/pipeline.rs` 翻译链路、`crates/lt-proto` 的 `ModelConfig/THINKING_STYLES/OVERRIDE_KEYS/prompts`、`crates/lt-ui` 模型编辑与呈现面、`crates/lt-app` 的翻译器构造/切换/停机。

---

## 一、实测基线（本次事故的完整证据链）

用项目自身 `lt-translate` 子系统（`examples/lmstudio_probe.rs`，流式、与生产同参）对本机 LM Studio 实测：

| 请求参数 | 结果 |
|---|---|
| 不带任何参数 | `content` 空，`reasoning_tokens = 256`，`finish_reason = length` |
| `enable_thinking: false`（auto 解析成 qwen 风格发出的） | 同上，**无效** |
| `chat_template_kwargs: {enable_thinking: false}`（vllm 风格） | 同上，**无效** |
| `thinking: {type: "disabled"}`（deepseek 风格） | 同上，**无效** |
| 用户消息追加 `/no_think` | 同上，**无效** |
| `max_tokens` 提到 1024 | 思考更久（3556 字符 reasoning），仍无 content |
| **`reasoning_effort: "none"`（openai 风格）** | **finish=stop，译文正确，reasoning_tokens=0，459ms** |

经应用自身代码路径（`Translator::translate_iter`，`thinking_style` 三档对照）：

```
style=off      deltas=  0 结果.len=   0 usage=(142,256)  3194ms
style=auto     deltas=  0 结果.len=   0 usage=(142,256)  3376ms
style=openai   deltas= 21 结果.len= 102 usage=(144,22)    459ms
```

**结论**：`qwen3.5-4b-mtp` 的 chat template 不认 `enable_thinking`（Qwen3 时代约定），本机 LM Studio 上唯一有效的关闭开关是顶层 `reasoning_effort: "none"`。应用的三处错位（`auto` 盲发 `enable_thinking`、`off` 什么都不发、流式只收 `delta.content`）叠加 256 硬编码 max_tokens，产出「静默空译文」，而 UI 把它显示成 `(相同语言)`——用户因此得到"语言判断错了"的错误结论。

---

## 二、发现清单

### P1 — 用户可见功能失效或强误导

**P1-1 空译文与「同语言免翻译」共用一个信号，UI 统一渲染为 `(相同语言)` `[核实·主线程]`**
证据：`crates/lt-ui/src/windows/overlay.rs:775-782`（`Some(text) if text.is_empty()` → `same_language`）；`crates/lt-orchestrator/src/pipeline.rs:1686`（真·同语言分支同样发 `text: String::new()`）；`crates/lt-translate/src/translator.rs:670-672`（流结束 `acc.trim()` 为空同样产出空串）。
影响：模型空响应 / 被截断 / 服务端拒答，全部显示成"语言相同，无需翻译"。这是本次事故中用户被误导的直接原因，且 `state.rs:2385` 有测试把这个语义钉死。
修复方向：pipeline 显式携带原因（新增 `UiEvent` 侧原因枚举属"纯新增类型化变体"，按契约冻结修订案豁免评审；或最简：空响应改发带原因的占位文案），UI 不再用空串兜底。

**P1-2 thinking 风格适配无反馈闭环：`auto` 盲发 `enable_thinking`、`off` 什么都不发，两者都关不掉思考 `[核实·主线程 + 实测]`**
证据：`crates/lt-translate/src/thinking.rs:31-43`（未知端点默认落 `"qwen"`，与同文件 `:10` 注释「off: LM Studio、Ollama」自相矛盾）；实测见 §一。
影响：任何"默认思考但 template 不认 enable_thinking"的模型（本机 qwen3.5-4b-mtp、以及同族 qwen3.6-*-mtp）在默认配置下 100% 产出空译文；用户唯一的出路是手动把风格改成 `openai`。判断结果在构造时固化为 `&'static str`，运行期无任何来自服务端反馈的校正通道；仓库里唯二能判定失败的两个信号（`finish_reason=length`、`completion_tokens_details.reasoning_tokens`）全仓零引用。
修复方向：**响应驱动兜底**——译后判 `content 空 && (reasoning_tokens > 0 || finish_reason == length)` → 同段用 `reasoning_effort: "none"` 重试一次，成功即按 `(api_base, model)` 记忆；仍失败则给可操作提示。默认端点改为"不发"、白名单才发。

**P1-3 悬浮窗模型下拉换模型不生效 `[核实·主线程]`**
证据：`crates/lt-ui/src/windows/overlay.rs:373-388`——`ui.selectable_value(&mut settings.active_model, …)` 只改内存，**既不 `send_cmd(Cmd::SwitchTranslator)` 也不 `mark_settings_dirty`**；同一函数里的源/目标语言下拉都发了命令（`overlay.rs:405-415` 发 `SetTargetLanguage`）。全仓 `SwitchTranslator` 生产者只有翻译页模型列表（`panel/translation.rs:211-214`、`:132`、`:214`、`:279`）与 prompt 600ms 防抖（`app.rs:2280-2287`）；`Cmd::ApplySettings`（`shell.rs:290-303`）只热应用增量/转录开关，不重建翻译器。
影响：从最常用的悬浮窗换模型后，翻译器仍是旧模型，而悬浮窗标题已显示新模型名——显示与实际不一致；用户以为换了、实际没换。设置是否落盘还取决于后续有无其它操作触发防抖登记。
修复方向：该下拉改为"选中即发 `Cmd::SwitchTranslator`（并按需 `mark_settings_dirty`）"，与翻译页语义对齐。

**P1-4「测试连接」把空回复判成功 `[核实·主线程]`**
证据：`crates/lt-orchestrator/src/pipeline.rs:1091-1097`：`match it.next() { Some(Ok(_partial)) => (true, …) }`——流式下第一个增量即判成功；思考型模型没有任何 `content` 增量，首个 item 就是流结束的 `Ok("")`，同样 `ok = true`。`test_translator_no_response`（`zh.yaml:630`）只在迭代器返回 `None` 时触发（`TranslateStream` 该分支自陈"仅防御"）。
影响：**恰好在模型坏掉时显示"测试成功"**。本次事故中用户若点过"测试连接"会得到通过结论，进一步排除了错误方向。
修复方向：跑完整一轮 + 判定内容非空（或至少判 `finish_reason`/`reasoning_tokens`），显示真实耗时与是否截断。

**P1-5 26+ 条中文错误文案零引用，错误实际以英文原文进 UI `[核实·主线程]`**
证据：`assets/i18n/zh.yaml:502-527` 备有 `err_401 / err_timeout / err_conn_refused / err_stream_interrupt …`；`grep -rn '"err_' crates/` = **0 命中**（含 `format!("err_…")` 动态拼接检查）。真实路径是 `crates/lt-translate/src/error.rs:40-48` 的 `[error: {原文}]`。更严重的是 `err_stream_interrupt` 的文案本身承诺「正在重试（最多 3 次）…重试仍失败将暂停翻译，点击此横幅重新启用」——**这三项功能在代码里都不存在**（无重试、无失败计数、无暂停、无横幅）。
影响：用户看到 `[error: api error 401: Incorrect API key provided…]`；同时 i18n 里写着一个并不存在的功能承诺，属于文档级失信。
修复方向：给 `TranslateError` 加文案键映射并在悬浮窗/字幕窗统一呈现；要么实现 `err_stream_interrupt` 承诺的语义，要么改掉该文案。

**P1-6 8 个翻译 worker 共享 `MutableState`：usage 统计串台、上下文历史乱序 `[核实·主线程]`**
证据：`TL_POOL_WORKERS = 8`（`pipeline.rs:51`）共享同一 `Arc<Translator>`；`translator.rs:670-672` 在流结束时把 usage 写进**共享** state，`pipeline.rs:358` 事后 `translator.last_usage()` 读取累加；`append_history_locked` 亦写共享 history。
影响：并发翻译下 A 读到 B 的 usage（同一笔用量重复计），或用例被 `translate_sync` 的置零覆盖（少计）；`warn_if_thinking_burned` 同源误报。`context_turns > 0` 时历史按**完成序**入史，可能出现乱序甚至"未来句"被当作上下文注入。
修复方向：usage 随迭代终点返回（或 `TranslateStream::usage()`），history 按提交序号插入；实例态只保留会话级只读配置。

**P1-7 官方 OpenAI 端点（o 系 / GPT-5.x）wire 不兼容且无逃生舱 `[推测·静态]`**
证据：`translator.rs:353-365` 无条件 `insert("max_tokens")` / `insert("temperature")`；`OVERRIDE_KEYS` + `extra_body` 合并只做 `insert`，**无法删除键**；`thinking.rs:20` 却把 `api.openai.com` 列入受支持端点（`PARAMLESS_ENDPOINTS`）。
影响：若用户使用官方端点跑推理模型，`max_tokens` 被拒（需 `max_completion_tokens`）、`temperature` 受限，请求 400 且用户无任何 UI 手段可救。
修复方向：按端点/模型派生 wire profile（token 参数名、temperature 允许值、JSON 模式、thinking 关闭体四件事同源决策），并允许 `extra_body` 显式删除自动键。

### P2 — 资源、状态一致性与可维护性

**P2-1 被放弃的流没有取消机制 `[核实·主线程]`**
`translator.rs:494` `runtime().spawn(async move { pump_stream(...) })` 丢弃 `JoinHandle`；`TranslateStream` 无 `Drop`；pump 只在**成功收到 content 增量**时才发现接收端已关闭（`tx.send(...).is_err()`）。思考型模型整个响应期没有 content 增量 → 消费方超时放弃后，pump 仍会把整段响应拉完，连接与本地 GPU 占用不释放。用户每点一次"测试连接"即触发一次提前放弃。
修复方向：`TranslateStream` 持 `JoinHandle`，`Drop` 时 `abort()`（或 `Notify` + `select!`）。

**P2-2 JobPool 满丢最旧无回执 → 消息永久停在「翻译中」`[核实·子代理]`**
`pipeline.rs:123-134`：`BoundedDropQueue` 丢最旧，被丢任务不产生任何 `UpdateTranslation`，UI 永远渲染 `> 翻译中…`，transcript pending 条目不收敛；`QueuePressure` 每 ≥200 条才上报且只进日志窗。
修复方向：丢任务时按 id 补一条"已放弃"译文并 `finalize_no_translation`。

**P2-3 事件动脉「满丢最旧」与非幂等事件冲突 `[核实·子代理]`**
`pipeline.rs:319-325` 每个 partial 都发一个**累积全文**快照事件（单条翻译 O(n²) 字节），cap 4096 满则丢最旧；被丢的 `AddMessage` 会让该 id 的最终译文无处落地。
修复方向：partial 改走快照格（复用 D-67 Monitor 的 ArcSwap 模式），动脉只承载状态机类事件。

**P2-4 `read_timeout` 一值两用，per-chunk 读超时不可达 `[核实·子代理]`**
`translator.rs:491/650-654`：总 deadline 在请求发起时定死，pump 的 per-chunk 超时自上个 chunk 起算，数学上消费方必然先超时；"停滞"与"总量超时"混为一类错误。
修复方向：拆"总预算 / 静默容忍"两个键。

**P2-5 基准页不应用 thinking 风格，TTFT 把推理算作首字 `[核实·子代理]`**
`bench.rs:162-178` 自建请求体，忽略 `thinking_style`/`overrides`/`extra_body`；TTFT 记在首个任意增量上。思考模型上"首字延迟"是推理起点、结果文本为空——基准数据会系统性误导选型。
修复方向：基准复用 `Translator` 的风格逻辑；TTFT 只认首个 content 增量。

**P2-6 `api_base` 不校验、不归一 `[核实·主线程]`**
`translator.rs:778-780` 仅 `trim()`：空串 → 运行时 relative-URL 错、尾斜杠 → `//chat/completions`、漏 `/v1` → 404。编辑框只校验 `extra_body` JSON，非法值最终以英文 `[error: builder error: …]` 出现在译文行。
修复方向：编辑期规范化（补 scheme、去尾斜杠、校验 `/v1`、拒空）并即时提示。

**P2-7 提示词模板非法时静默回退，且上下文分支判断与渲染背离 `[核实·子代理]`**
`translator.rs:280-288` 渲染失败 → 整段换 `DEFAULT_PROMPT`（仅一条 warn，UI 无提示）；`build_messages` 用**原始模板字符串** `contains("{context}")` 决定是否追加历史消息，与真实渲染结果可背离（`{0}` 等坏占位、`{{context}}` 转义场景下历史被抑制）。
修复方向：编辑期校验模板；分支判断复用渲染结果。

**P2-8 切换失败的"三处状态矛盾" `[核实·子代理]`**
`pipeline.rs:1056-1062` 重建失败保留旧 rig（继续用旧模型翻译），`app.rs:978` 的红字 `panel.translator_error` 只写不清，悬浮窗标题却已显示新模型名。
修复方向：错误有了清零点；失败时 UI 明确标注"仍在使用 <旧模型>"。

**P2-9 失败漏记成本、空响应照记条数 `[核实·主线程]`**
`pipeline.rs:338-351` 错误分支提前 return，`tl_count`/token 均不累加（实测一次 256 token 全丢）；`:357` 空响应成功路径照记 `tl_count`。
修复方向：失败也累加已消耗 usage；空响应不计条数。

**P2-10 rustls 只用内置 Mozilla 根，不读系统证书库 `[核实·子代理]`**
`Cargo.toml` 的 `webpki-roots` → 企业 TLS 解密代理、自签/私有 CA 的自建端点全部握手失败——而"system 代理"模式恰好面向这类环境。
修复方向：改 `rustls-native-roots` 或提供开关。

### P3 — 打磨项

- **P3-1** `resolve_thinking_style` 模型名匹配（`contains("deepseek"|"glm")`）优先于端点判定 → DeepSeek-R1 托管在第三方时发错关闭体。`thinking.rs:33-39`
- **P3-2** `overrides` 剔除 null 而 `extra_body` 原样发送 null，用户以为 null=删除其实更糟（发出 `"enable_thinking": null`）。
- **P3-3** `check_repetition`（`translator.rs:743`）只测前缀自重复且周期必须 ≥8；非流式路径完全没有重复检测。
- **P3-4** `TranslatorParams` derive `Debug` 带明文 `api_key`（当前无调用点，属潜在泄漏面）。
- **P3-5** 成本显示符号按 UI 语言硬编码 ¥/$，而价格字段语义是"美元/1M"——中文界面上把美元单价显示成 ¥。
- **P3-6** `from_value_compatible` 的 `sanitize()` 修正（models 空→占位、active_model 越界→0）静默发生，无日志无 UI 痕迹。
- **P3-7** `tl.timeout` 未经钳制直接进总线（UI 钳 1..60，手改档可绕过），叠加停机 `join_all` 无超时 → 退出延迟 = 在途请求剩余超时。
- **P3-8** 每次 `SwitchTranslator` 重建 reqwest Client（连接池/TLS 会话全丢）。`translator.rs:759-782`
- **P3-9** 无请求级留痕：状态码/`finish_reason`/尝试序号都不落日志，失败请求无法复盘；同时 INFO 级会打印用户 `overrides`/`extra_body` 原值（密钥未泄漏——全仓日志 grep `sk-`/`Bearer` 零命中）。

---

## 三、通过审查的部分（不必改，作为基线守护）

- 「先带后撤」流式重试**不会**产生双并发在途请求：async-openai 0.41 先 await 到响应头才构造 SSE 流，首次失败只可能发生在头阶段。`[核实·子代理]`
- `Err` 之后迭代器不再产出 `Ok`；无锁跨 await，`parking_lot` 先绑定后分支纪律完好。`[核实·子代理]`
- API Key 处理合规：全仓无明文密钥进日志/错误文本。`[核实·主线程]`
- 恢复路径不缺：目标语言/超时经设置总线逐调用生效，换活动模型即重建翻译器，**无需重启应用**。`[核实·主线程]`

---

## 四、建议处置顺序

**批次一（与本次事故同源，改动小、收益直接）**
1. 响应驱动闭环：解析 `finish_reason` / `reasoning_tokens`，空响应时以 `reasoning_effort:"none"` 重试一次并按 `(api_base, model)` 记忆；仍空则给可操作提示（P1-2、P1-4 共用）。
2. 空 ≠ 同语言：pipeline 显式携带原因，UI 停止用 `(相同语言)` 兜底（P1-1）。
3. 「测试连接」改完整一轮 + 非空判定（P1-4）。
4. `err_*` 文案接线，或先删掉 `err_stream_interrupt` 里的功能承诺（P1-5）。

**批次二（并发与资源）**
5. 流取消：`TranslateStream` 持 `JoinHandle` + `Drop::abort`（P2-1）。
6. usage 出例、history 按序（P1-6）。
7. 丢任务补回执、partial 改快照格（P2-2、P2-3）。

**批次三（配置与协议边界）**
8. `api_base`/`overrides` 校验 + 请求体预览（P2-6、P3-2）。
9. wire profile（`max_tokens`/`max_completion_tokens`、JSON 模式、temperature 值域）（P1-7）。
10. 悬浮窗模型下拉接线、`translator_error` 清零点（P1-3、P2-8）。

---

## 五、覆盖缺口

- P1-7、P2-5、P2-10 的厂商侧行为未实机验证（仅本机 LM Studio 可测），标注为推测/静态推断。
- 401/404/429/TLS 代理等失败路径无法在本机复现，相关结论来自代码推演。
- 未审计 async-openai 0.41 的 SSE 解码内部与 reqwest 连接池细节。
- 评审期间使用的临时探针 `crates/lt-translate/examples/lmstudio_probe.rs` **已删除**：软件运行期不需要它，不入库（用户裁决 2026-09-10）。等价的调试脚本留在系统临时目录；W3 验收需要复验时按同一形态临时重建即可。
