# LLM 翻译接口层改造方案（v1.4 定稿）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> **状态**：**定稿**。四项裁决已于 2026-09-10 由用户锁定（见 §0.2）。本文为施工唯一依据；**动工须用户明确指令**。
> **v1.1 方向**：不以"关闭思维链"为中心——应用只做三件事：① 拿 ASR 字符串；② 经 LLM API 翻译到指定语言；③ 取回最终译文。对外解耦（一个接口 + 一组通用参数），内部耦合（差异、兜底、记忆全在里面）。
> **v1.2 修订**：思维链控制收敛为一个复选框；最大输出长度移出界面且内部不发送；新增思维链隔离规范。
> **v1.3 修订**：四项裁决落地（默认关闭思考 / 记忆仅内存 / **UI 与后端一比一同步**（新增 §2.5 专章）/ 高级区全套保留）。
> **v1.4 修订**：废除"复选框直接映射旧字段"的写法——旧字段 `thinking_style` 的 `"off"` 值语义与字面**正好相反**（`off` = 不发送关闭参数 = 让模型照常思考），用户排查本故障时曾因此设错。改为**两个控件 ↔ 两个字段**：新增 `disable_thinking`（默认 true，勾选即发关闭字段）+ 旧字段退为"关闭方式"；旧 `"off"` 在 `sanitize()` 中归一化，此后不再出现。
> 诊断证据见 [llm-api-review.md](llm-api-review.md)。

---

## 0. 摘要

### 0.1 设计要点

| 项 | 内容 |
|---|---|
| 设计北星 | 对外解耦、内部耦合；**UI 与后端一比一同步**（§2.5，评价标准：界面上看到的，就是实际在跑的） |
| 用户可见参数 | 地址、密钥、模型名、**关闭模型思考（复选框）**、温度、超时、流式、系统提示词、代理；高级区：关闭方式、top_p、频率惩罚、存在惩罚、上下文数、无 system 角色、额外参数 |
| 用户不可见 | token 参数名差异、JSON 模式、**最大输出长度**（内部默认不发送）、自愈与学习过程（但**结果可见**，见 §2.5） |
| 默认请求 | `model` / `messages` / `temperature` / `stream` + 勾选时的推理关闭参数；**不发送 `max_tokens`**；不做供应商预判式注入 |
| 思维链 | **绝不进入输出字符串**（三层防线，§4.3） |
| 契约影响 | 纯新增 `UiEvent` 变体 2 个 + 枚举 3 个 + `ModelConfig.disable_thinking` 字段（serde default 兼容旧档，加法豁免，`PROTO_VERSION` 不递增） |
| 波次 | W1 接口与参数面 → W2 体检、结论化与思维链隔离 → W3 兜底与记忆 → W4 诊断面 → W5 取消与账本 |
| 行为偏差 | 自 **D-82** 起登记 |
| 新依赖 | 无 |

### 0.2 用户裁决（2026-09-10，锁定）

| # | 议题 | 裁决 | 落地位置 |
|---|---|---|---|
| 1 | "关闭模型思考"默认状态 | **默认关闭思考**（省 token、更快） | §2.3：新增 `disable_thinking: bool`（默认 `true`，serde default，加法豁免），复选框直接映射它；旧 `thinking_style` 退为"关闭方式" |
| 2 | 学到的"关闭方式"是否落盘 | **只记内存**（不写用户配置文件） | §4.4：会话内存表；每次启动重新学习一次（多数模型一次请求内收敛） |
| 3 | UI 与后端一致性 | **一比一同步，必须处理好** | §2.5 专章（8 条规则 + 对照表 + 4 处现存不一致修复 + 2 个守卫测试） |
| 4 | 高级区参数范围 | **全套保留** | §2.2 |

### 0.3 实测基线（本机 LM Studio + `qwen3.5-4b-mtp`，同一句英文，2026-09-10）

| 请求配置 | 结果 | 耗时 |
|---|---|---|
| `max_tokens=256`（现状默认） | 正文空，256 token 全为推理，`finish_reason=length` | 3.2s |
| `max_tokens=1024` | 正文空，1024 token 全为推理，仍在思考 | ~5s |
| **不发送 `max_tokens`** | **正文 33 字符，正确译文**，3437 reasoning tokens，`finish_reason=stop` | **43.7s** |
| 不发送 `max_tokens` + `reasoning_effort:"none"` | **正文正确译文**，0 reasoning tokens，22 tokens | **0.46s** |

三条结论：① 本次故障的直接原因是应用强加的 256 上限（模型"先想再答"，思考把预算吃光，回答永远没机会产出）；② 不设上限时模型会自然想完再答；③ 关不关思考的差别是 **43.7s vs 0.46s（95 倍）**——这是"默认关闭思考"裁决的实证依据。

---

## 1. 设计原则（七条不变量）

- **INV-A 结论完整**：任何"没有译文"的情形都必须携带机器可读原因；UI 禁止用"字符串是否为空"推断语义。
- **INV-B 账本归属**：每段翻译的用量与上下文归属该段调用，不落在跨线程共享状态上。
- **INV-C 可取消**：消费方放弃后，流必须真正中止（含 HTTP 连接与服务端生成）。
- **INV-D 最小通用请求**：默认只发主流通用参数；不做供应商预判式注入；**不发送输出长度上限**。
- **INV-E 接口唯一**：调用方只知道"文本 + 源/目标语言 → 译文"；供应商概念不得出现在接口签名与用户文案中。
- **INV-F 思维链隔离**：思考内容（结构化字段或内联标签）在任何路径下都不得出现在译文、字幕、转写文件中。
- **INV-G UI 与后端一比一**：界面上每一个可调项都能追到唯一后端消费点；后端每一次实际偏离配置，都必须在界面上可见。

---

## 2. 用户可见的参数面

### 2.1 一等字段（主界面）

| 字段 | 默认 | 说明 |
|---|---|---|
| 显示名 / `api_base` / `api_key` / `model` | 现状 | 新增归一化与就地校验（§4.7） |
| `proxy` | none | 不变 |
| **关闭模型思考**（复选框） | **勾选（关）** | 直接映射新增字段 `disable_thinking`（默认 `true`，§2.3） |
| **温度** | 0.3 | **新增一等字段**（现状硬编码于 `pipeline.rs:257`） |
| `streaming` | 开 | 不变 |
| `timeout`（全局） | 10s | 不变 |
| `system_prompt`（全局） | 默认模板 | 不变 |

> **最大输出长度不再出现在界面上**，内部默认**不发送**。仅在体检发现"输出被截断"时，内部补发 `4096` 重试一次并在界面标注（§4.4）。

### 2.2 高级区（折叠，**全套保留**——裁决 4）

| 字段 | 默认 | 说明 |
|---|---|---|
| 关闭方式（手动指定） | 自动 | 五档：自动 / DeepSeek / Qwen / vLLM / OpenAI——自动链不灵时的人工覆盖 |
| `top_p` | 留空=不发 | 主流通用，翻译通常不需要 |
| 频率惩罚 / 存在惩罚 | 留空=不发 | 同上 |
| `context_turns` | 0 | 上下文轮数（2026-09-10 补充：翻译页「模型配置」组另设当前活跃模型的直达行，见 `docs/archive/context-turns-ui.md`，**D-84**） |
| 无 system 角色 | 不勾 | 少数端点兼容开关 |
| `extra_body`（JSON） | 空 | 逃生舱：原样透传 |

### 2.3 「关闭模型思考」：两个控件 ↔ 两个字段，语义字面直白

**为什么不让复选框去承载旧字段**：既有字段 `thinking_style` 的取值 `"off"`，语义是"**关闭这个控制功能**"——即不发送关闭参数、让模型照常思考。字面读起来与真实含义正好相反：旧界面该下拉的标签写作「关闭 (不发送关闭参数)」，本次故障排查中用户本人就把它从 `auto` 改成 `off`，本意是"关掉思考"，实际效果是关掉了"关闭思考"这个功能。裁决 3 要求 UI 与后端一比一，就不该把这个命名陷阱继续留在界面上。

**新设计：两个控件、两个字段，各自语义唯一。**

| 界面控件 | 配置字段 | 语义 |
|---|---|---|
| ☑ **关闭模型思考**（默认勾选） | **新增** `disable_thinking: bool`，默认 `true`（serde default 兼容旧档，加法豁免） | **要不要关**——勾选即发送关闭字段 |
| 关闭方式（高级区；未勾选时置灰） | `thinking_style`（保留） | **用哪种方式关**：`auto`=自动链 / `deepseek` / `qwen` / `vllm` / `openai` |

**后端读取规则（定死，唯一一条链）：**

1. `disable_thinking == false` → **不发送任何推理相关参数**（保留旧 `"off"` 的能力），`thinking_style` 的值被忽略；
2. `disable_thinking == true` 且 `thinking_style` 为 `None`/`auto` → 走 §2.3.1 自动链；
3. `disable_thinking == true` 且 `thinking_style` 为四种具体方式之一 → 只发该一种，不自动试；
4. 旧值 `thinking_style == "off"` 在 `sanitize()` 中**归一化**为 `disable_thinking = false` + `thinking_style = None`（见 §6），此后 `"off"` 不再出现在任何档案中。

> `THINKING_STYLES` 常量保留六项（用于解析旧档案），界面只呈现"自动 + 四种方式"五项。

**§2.3.1 自动链（内部，最多对每个新模型试 4 种）：**

```
勾选 → ① reasoning_effort: "none"     ← 事实标准（OpenAI 5.1+谱系/Azure/xAI/Ollama/OpenRouter/vLLM/llama.cpp/SGLang/LM Studio 实测生效）
      → 若体检显示"仍在推理"或"仍无输出"
      → ② enable_thinking: false       ← DashScope/Qwen、SiliconFlow
      → ③ chat_template_kwargs:{enable_thinking:false}  ← vLLM/SGLang 模板
      → ④ thinking:{type:"disabled"}   ← DeepSeek、GLM、Moonshot
      → 任一步成功 → 本次会话记下 (api_base, model) → 该方式，之后只用这一种
      → 全部失败（GLM-5.3 / kimi-k3 / gpt-oss@Ollama 等强制思考模型）
        → 界面标注"该模型无法关闭思考"，转入"不关闭"路径继续工作
```

**§2.3.2 参数被拒的降级（必需，因默认为"关"）：** 若服务端以 400 拒绝我们注入的推理参数（错误文案必须**明确指向该参数名**才触发，不猜），则去掉该参数重试一次、记住"该模型不接受此类参数"，并在界面上把该条目标注为"该模型不支持关闭思考（已忽略）"。代价：对这类模型，每次启动的首次请求会多一次往返（裁决 2 决定不落盘，故不跨会话记忆）。

### 2.4 移出界面的项（字段保留，见 §6）

| 现状项 | 处置 | 理由 |
|---|---|---|
| `json_response`（结构化输出） | 移出界面（字段保留） | 翻译不需要 JSON 模式；现状还把它实现成 `json_schema`，与 i18n 承诺的 `json_object` 不一致 |
| `overrides` 六行覆写表 | 移出界面（字段保留） | 温度成为一等字段；其余项由高级区显式字段承接 |
| 最大输出长度 | 新增概念，不引入界面 | 应用不替用户拧这个旋钮（实测见 §0.3） |

### 2.5 UI 与后端一比一同步规范（裁决 3）

**评价标准：界面上看到的，就是实际在跑的。**

**八条规则：**

1. **单一字段单一语义**：一个控件 ↔ 一个配置字段 ↔ 一个后端读取点。禁止同一概念在界面出现两次（如复选框 + 下拉各自持有一个"开关"），也禁止后端在多处独立读取同一字段。
2. **控件即字段（无影子状态）**：复选框的勾选状态**就是** `thinking_style != "off"`（§2.3），不设影子变量。
3. **改即生效**：改动**活动模型**的生效字段 → 立即重建翻译装置；改动非活动模型 → 仅落盘，并在界面上标注"切换到该模型时生效"。
4. **回执闭环**：重建成功/失败都必须回执 UI。成功 → 清除错误横幅；失败 → 明确显示"**仍在使用 <旧装置名>**：<原因>"。禁止静默失败。
5. **偏离可见**：后端因自愈/降级而偏离配置时（换关闭方式、补发输出上限、参数被拒降级），界面必须能看到"**当前实际在用：X**"（模型条目状态行），且与日志一致。
6. **预览同源**：请求体预览必须调用与真实请求**同一个构造函数**（`build_request_body`），禁止另写镜像实现。
7. **无孤儿**：界面上不得存在"改了没反应"的控件；反之后端不得存在"生效但界面看不见"的开关（自愈结果除外，按规则 5 展示）。
8. **守卫**：新增两个测试（§5 W1/W4）：
   - `every_model_config_field_has_consumer`——`ModelConfig` 每个序列化键必须落在三类清单之一（"请求构造消费" / "UI 消费" / "本地展示"），仿 `settings_bus.rs:233` 的 `every_settings_field_is_classified` 写法；
   - `preview_matches_actual_body`——预览体与真实请求体逐键比对（同一构造函数的回归护栏）。

**现存四处不一致（本次改造必须一并修掉）：**

| # | 现状 | 修为 | 波次 |
|---|---|---|---|
| a | 悬浮窗模型下拉只改 `active_model`，**不发 `SwitchTranslator`、不登记落盘**（`overlay.rs:373-388`），标题已显示新模型而翻译器仍是旧的 | 与翻译页同语义：选中即发命令 + 登记落盘 | W1 |
| b | `max_tokens`/`temperature` 硬编码于构造点（`pipeline.rs:257-258`），界面无从调整；`overrides` 才是唯一生效路径 | 温度升为一等字段并接进构造点；输出长度不再发送 | W1 |
| c | `json_response` 勾选实际发送 `json_schema`，而 i18n 文案承诺 `json_object`；该键全仓无第二消费点 | 移出界面（字段保留） | W1 |
| d | 重建失败保留旧装置，但标题已切新名、错误横幅只写不清（`pipeline.rs:1056`、`app.rs:979`） | 按规则 4 + 规则 5 补齐回执与清零点 | W4 |

---

## 3. 对外接口（唯一，INV-E）

```rust
/// crates/lt-translate/src/api.rs（新文件）
pub struct TranslateRequest { pub text: String, pub source_lang: String, pub target_lang: String }

pub struct TranslationOutcome {
    pub text: String,        // 最终译文（已 trim、已剥离思维链）
    pub usage: Usage,
    pub truncated: bool,     // finish_reason == length
}
pub struct Usage { pub prompt_tokens: u64, pub completion_tokens: u64, pub reasoning_tokens: u64 }

pub enum TranslateError {
    Connection(String), Timeout(String), Auth { code: u16, message: String },
    NotFound(String), RateLimited(String), ServerError { code: u16, message: String },
    EmptyModelOutput { detail: String }, Repetition(String), Other(String),
}

impl Translator {
    pub fn translate(&self, req: &TranslateRequest, timeout_secs: u32) -> Result<TranslationOutcome, TranslateError>;
    pub fn translate_streaming(&self, req: &TranslateRequest, timeout_secs: u32,
                               on_partial: &mut dyn FnMut(&str)) -> Result<TranslationOutcome, TranslateError>;
}
```

**契约纪律**：接口签名不得出现 `thinking` / `provider` / `vendor` / `reasoning` / `json_mode` 等词（评审项）；现有 `translate_iter()` 降为内部实现。

---

## 4. 内部处理（耦合面）

### 4.1 请求构造

```
model / messages / temperature（用户留空则不发）/ stream
+ stream_options.include_usage（首次；被拒则去掉重试——现状机制保留）
+ 高级区显式参数（top_p / 惩罚 / extra_body）
+ `disable_thinking == true` 时的推理关闭参数（按 §2.3 规则 2/3 取"自动链"或"指定方式"）
```

**不发送 `max_tokens`**；不预判供应商（废除"对未知端点盲发 `enable_thinking`"）。

### 4.2 结果提取

只累积 `delta.content`。结构化推理字段一律忽略：`reasoning_content`（LM Studio / DeepSeek / LiteLLM）、`reasoning`（**vLLM 已改名为此**）、`thinking`（Ollama 原生）。提取后必须经 §4.3 隔离器才允许进入译文。

### 4.3 思维链隔离（INV-F，三层防线）

**实证风险**（非理论）：LM Studio 对未识别模板的模型会把 `<think>…</think>` 原样写进 `content`（issue #1569，2026-02 仍开放）；llama.cpp `--reasoning-format none` 官方说明即为"leaves thoughts unparsed in `message.content`"；vLLM 不配 `--reasoning-parser` 即不抽取；SGLang `separate_reasoning: False` 把思考塞回 content。

**防线一**：只读 `content`，其余字段全丢。

**防线二**：正文内联标签剥离，标签表以实证为准：

| 形态 | 开 | 闭 | 出处 |
|---|---|---|---|
| think 标签 | ` thinking` | `<｜end▁of▁thinking｜>` | DeepSeek-R1/Qwen3 无 parser 部署；LM Studio #1569 |
| 兼容写法 | `<thinking>` | `</thinking>` | 客户端通行剥离对象（防御性纳入） |
| harmony 分析通道 | `<\|channel\|>analysis<\|message\|>` | `<\|end\|>` | gpt-oss |
| Kimi K2 Thinking | `◁think▷` | `◁/think▷` | SGLang parser 表 |
| Apertus | `<\|inner_prefix\|>` | `<\|inner_suffix\|>` | SGLang parser 表 |
| **游离闭标签** | — | `</think>`（无配对的半截标签） | GLM-5.3 + 老 parser：`"Simple question.</think>2 + 2 = **4**"`，答案在闭标签之后 |

剥离规则（定死）：
1. 配对开/闭标签 → 删除整段（含标签），保留前后文本；重复至稳定，**最多 4 轮**。
2. 无配对的闭标签 → 删除从头到该标签（含）——答案在其后。
3. 未闭合的开标签（流式进行中）→ 从该标签起**全部不输出**，标记"思考中"，不产出 partial。
4. 剥离后为空 → 按"无输出"处理（进 §4.4 的 `Empty*` 分支），**绝不把思考当译文**。

**防线三**：流式跨块——剥离器维护尾部暂存区（上限 = 最长标签字节数），"可能是标签前缀"的尾巴一律不输出；流结束 flush（构成标签则丢弃）。

**测试（六条，必做）**：配对剥离 / 游离闭标签（GLM 形态）/ harmony / 跨 3 分片 / 未闭合判空 / 两种结构化字段名不达输出。

### 4.4 体检、兜底与记忆

体检 `ResponseVerdict { Ok, OkTruncated, EmptyReasoningBudget, EmptyTruncated, EmptyNoOutput }`（字段来源见附 A）。

| 体检结论 | 兜底动作（同段最多一轮） |
|---|---|
| `EmptyReasoningBudget` | 勾选状态 → 推进 §2.3.1 自动链下一步；未勾选 → 提示用户"模型在思考，建议勾选关闭模型思考" |
| `EmptyTruncated` | 显式补发 `max_tokens = 4096` 重试一次（应对服务端默认上限过小），并在界面标注 |
| `OkTruncated` | 不兜底，正常显示 + 标注"输出被截断" |
| `EmptyNoOutput` | 不兜底；`TranslationFailed{kind: Empty}` + 人话建议 |
| 400 且错误明确指向注入参数 | 去掉该参数重试一次（§2.3.2），界面标注"该模型不支持关闭思考（已忽略）" |
| 其他请求层错误 | 不兜底 |

**记忆（裁决 2：仅内存）**：会话内 `HashMap<(String, String), HealStrategy>`，随进程退出清空；**不写入 `settings.json`**。后果：每次启动对每个模型重新学习一次（多数模型一次请求内收敛）。界面按 §2.5 规则 5 展示当前实际在用的方式。

### 4.5 结论化契约

```rust
// lt-proto/src/events.rs（纯新增，加法豁免）
pub enum SkipReason { SameLanguage }
pub enum FailureKind { Empty, Truncated, Dropped, Timeout, Auth, NotFound, RateLimited, ServerError, Connection, Repetition, Unknown }

UiEvent::TranslationSkipped { id: u64, reason: SkipReason },
UiEvent::TranslationFailed  { id: u64, kind: FailureKind, detail: String, tl_ms: f64 },
```

`UpdateTranslation{id,text,tl_ms}` 语义收窄为"成功译文"（形状不变）。UI 侧 `OverlayMessage.translation: Option<String>` → `TranslationView { Pending, Streaming(String), Ready(String), Skipped(SkipReason), Failed{kind, detail} }`；字幕窗喂养：Skipped → 原文，Ready → 译文，Failed → 失败占位。
**守卫影响**：`FailureKind` 要求 UI 写**穷尽 match**（不得 `_ =>`），以满足 `check_dead_contract.ps1` 的 ≥2 引用判定。

### 4.6 取消与账本

- `TranslateStream` 持 `JoinHandle`，`Drop` 时 `abort()`（现状 `translator.rs:494` 丢弃句柄）→ INV-C。
- 用量随本次调用返回，删除 `Translator::last_usage()` 共享态（`translator.rs:219`；调用点 `pipeline.rs:358`）→ INV-B。
- 上下文历史按提交序号（`BTreeMap<u64, (String,String)>`）插入与裁剪。

### 4.7 配置校验

`ModelConfig::normalize()` + `ModelConfig::issues()`：空地址 / 空模型名 / 地址无 scheme / 缺 `/v1` / 尾斜杠 / 高级参数越界 / `extra_body` 非 object → 就地中文提示，不阻断保存；**不擅自改用户填的地址**（只 trim + 去尾斜杠）。

---

## 5. 施工波次

| 波 | 内容 | 新增测试 | 验收判据 |
|---|---|---|---|
| **W1** | 接口与参数面 + **§2.5 的 a/b/c 三处同步修复**：`api.rs`；移除最大输出长度（内部不发）；关闭思考复选框（映射 `disable_thinking`，默认 true）+ 高级区全套；`thinking_style == "off"` 归一化；温度一等字段；悬浮窗模型下拉接线；`every_model_config_field_has_consumer` | +11 | 请求体不含 `max_tokens` 与预判参数；请求体预览与实发一致；悬浮窗换模型立即生效；旧档 `"off"` 正确落到"未勾选" |
| **W2** | 体检、结论化与**思维链隔离器** | +14 | 空响应不再显示 `(相同语言)`；六条剥离用例全绿 |
| **W3** | 自动链、参数被拒降级、会话内记忆 | +7 | 本机 LM Studio 默认配置下首段即 0.46s 量级；关不掉的模型有明确标注 |
| **W4** | 诊断面 + §2.5 规则 4/5 落地（回执闭环、偏离可见、清零点）+ `preview_matches_actual_body` | +9 | 重建失败显示"仍在使用 X"；空回复的「测试连接」报失败；错误为中文 |
| **W5** | 取消与账本 | +7 | 放弃流后服务端 ≤1s 停止生成；并发 8 段统计不串台 |

---

## 6. 兼容与迁移

| 面 | 处理 |
|---|---|
| 旧 `settings.json` | 新增 `disable_thinking`（`serde(default)` = `true`）；`json_response` / `overrides` 字段保留可读，界面不再展示 |
| 旧档案 `thinking_style == "off"` | 在 `sanitize()` 中归一化为 `disable_thinking = false` + `thinking_style = None`：复选框显示**未勾选**，如实保留用户"不干预"的旧意图；用户勾上即切到自动关闭。此后 `"off"` 不再出现在档案中 |
| 默认行为变化 | ① 默认走"自动关闭思考"（对不接受该参数的模型自动降级，§2.3.2）；② 不再发送 `max_tokens`；③ 空响应不再显示"同语言"；④ 测试连接判据收紧 |
| `PROTO_VERSION` | 不递增（纯加法，且无 Settings 字段变更） |
| 测试基线 | 454+7 → 预计 **502+7** |
| 文档 | 批准后登记 D-82~；`docs/README.md` 索引同步 |

---

## 7. 已裁决与遗留

**已裁决（2026-09-10）**：默认关闭思考 / 记忆仅内存 / UI 与后端一比一同步 / 高级区全套保留——见 §0.2。

**遗留待定（不阻塞 W1）**：
1. 高级区"关闭方式"下拉的文案是否需要随复选框改称（如"关闭方式：自动/…"）。
2. `err_stream_interrupt` 文案改写（不实现其承诺的"重试 3 次/暂停/横幅"）——按 §W4 落地时一并处理。
3. 学习记忆若将来要落盘，按"纯新增 Settings 字段 + 结构化条目（禁拼接字符串键）"补一次加法变更。

---

## 8. 最终验收总表（DoD）

| # | 判据 | 验证方式 |
|---|---|---|
| 1 | 默认配置（勾选关闭思考）下，本机 LM Studio + `qwen3.5-4b-mtp` 首段即亚秒级正确译文 | 实机 |
| 2 | 取消勾选后：同一模型仍能出正确译文（44s 量级），且界面如实显示"未关闭思考" | 实机 |
| 3 | 任何情形下思考内容不出现在译文、字幕、转写文件 | 六条剥离用例 + 实机 |
| 4 | 请求体中不含 `max_tokens` 与预判式推理参数；**预览体 == 实发体** | 预览 + 抓包 |
| 5 | 悬浮窗与翻译页改模型/改参数 → 翻译装置立即重建；重建失败显示"仍在使用 X" | 实机 |
| 6 | 空响应不再显示 `(相同语言)`；真·同语言仍显示 | 假服务端 + 实机 |
| 7 | 空回复的「测试连接」报失败；401/404/超时显示中文提示 | 假服务端 |
| 8 | 放弃流后服务端 ≤1s 停止生成；并发 8 段统计不串台 | 实机 + 单元测试 |
| 9 | 五守护脚本 + 全量测试 + clippy 全绿 | CI |

---

## 附 A：体检判定的字段来源（已核实 async-openai 0.41.3）

- `finish_reason`：流式 `CreateChatCompletionStreamResponse.choices[0].finish_reason: Option<FinishReason>`（`chat_.rs:1170`，取最后一个非 None）；非流式 `Choice.finish_reason: Option<CompletionFinishReason>`（`chat_.rs:57`）。
- `reasoning_tokens`：`usage.completion_tokens_details.reasoning_tokens`（`chat_.rs:90-103`）。
- 判定顺序：正文非空 → `Ok`/`OkTruncated`；否则 `reasoning_tokens > 0` → `EmptyReasoningBudget`；否则 `finish_reason == Length` → `EmptyTruncated`；否则 `EmptyNoOutput`。

## 附 B：调研来源（要点）

- OpenAI/Azure：`reasoning_effort` 官方取值含 `none`；o 系与 gpt-5 初代**不支持 none**（会 400），gpt-5.1+ 起支持——§2.3.2 的降级路径即为此准备。
- vLLM：`reasoning_effort:"none"` → 自动注入 `enable_thinking=false`；模板不认该键时静默过滤。
- llama.cpp：README 原文 "If `none`, reasoning/thinking is disabled"。
- Ollama：兼容口接受 `reasoning_effort`（含 `none`）；gpt-oss 的思考"cannot be fully disabled"。
- OpenRouter：`reasoning.effort:"none"` 关闭；模型元数据带 `supported_efforts`。
- LM Studio：OpenAI 兼容文档未列 `reasoning_effort`，但本机实测**生效**；原生 `/api/v1/models` 暴露 `capabilities.reasoning.allowed_options`（本机该模型为 `["off","on"]`）。
- 思维链漏出实证：LM Studio #1569、#851；vLLM #54744；SGLang `separate_reasoning: False`。

---

## 9. 实施状态（2026-09-10 更新）

**已落地**（每波均以"全量测试 + clippy -D warnings + 四守护脚本"为门禁）：

| 波 | 提交 | 内容 |
|---|---|---|
| W1 | `364dd0e` | 接口与参数面：`ModelConfig` 新增 `disable_thinking`（默认 true）/`temperature`（None=不发）；旧 `thinking_style=="off"` 归一化；**不再发送 `max_tokens`**；关闭思考复选框 + 温度一等字段；方式下拉降级高级区；`json_response`/覆写表移出界面；悬浮窗模型下拉补 `SwitchTranslator`；字段登记测试 |
| W2a | `b36fdda` | 思维链隔离器（五行标签表 + 跨分片尾巴缓冲 + 游离闭标签）+ 回应体检 `ResponseVerdict`（finish_reason/reasoning_tokens） |
| W2b | `79b0959` | 结论化：`TranslationSkipped`/`TranslationFailed{kind}` 契约；UI `TranslationView` 五态；字幕窗喂养规则；失败中文文案（10 类） |
| W3 | `a020db8` | 内部兜底链（诊断驱动的一次重试）+ 会话内记忆（只记内存）；用量跨尝试累加 |
| W5/W4a | `f4e62c2` | 流可取消（`Drop` → `abort()`）；测试连接改完整一轮 + 非空判定；`FailureKind::i18n_key()` 单一事实源 |
| W4b | `634eda4` | 翻译恢复后清除 `panel.translator_error`（回执闭环） |

**实机验收**（本机 LM Studio + `qwen3.5-4b-mtp`，默认参数）：`cargo run -p lt-translate --example smoke`
→ 译文正确、**186ms / 10 completion tokens**（改造前 3.2s 空译文、UI 显示 `(相同语言)`）。

测试基线：454+7 → **483+7**；契约变体 93 → 106（0 死）。

**尚未落地**（按优先级）：

1. **请求体预览**（方案 §3.10b）：受分层规则所限，lt-ui 不能依赖 lt-translate 构造请求——
   需要新增 `Cmd::PreviewRequest(config)` + `UiEvent::RequestPreview{json}`（纯加法）经
   编排域构造并把脱敏后的 JSON 回执给 UI；配套 `preview_matches_actual_body` 测试。
2. **配置校验**（方案 §4.7）：`ModelConfig::normalize()/issues()` + 编辑对话框就地提示
   （api_base 归一化、overrides 类型、模板占位符）。
3. **partial 快照格 / 丢任务回执**（评审 P2-2/P2-3）：事件动脉的 partial 改 ArcSwap 快照，
   JobPool 丢任务时补回执（`FailureKind` 需新增 `Dropped` 变体）。
4. **实机走查**：字幕窗喂养规则、失败态视觉、取消行为（放弃后服务端 ≤1s 停止生成）。

**实施中发现并修复的额外缺陷**：
- 源文件中完整标签字面量会被编辑/编码环节**间歇性改写**（闭标签曾被写成全角变体，
  会导致"块永不闭合 → 译文恒空"的静默故障）→ 标签表全部改用 `\u{..}` 转义书写 +
  `tag_table_is_intact` 守卫（`reasoning.rs`）。
- `TlRig.msg` 在翻译出口成为死字段（失败文案改由 UI 本地化）→ 删除。
