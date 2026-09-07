# 用户体验反馈闭环计划（ux-feedback）

> 【已归档】阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档。2026-09-07 起本版不再追求与原版 1:1，Python 原版仅作行为参考；本文仅作决策史，不再作为施工依据（活跃文档见 AGENTS.md 与 docs/README.md）。
> 一期二期已落地（313 测）；三期可选项（前缀码结构化等）已入 AGENTS.md 待办。


> 2026-09-07 用户要求：从小白用户视角审视打开后使用体验，目标 = 行为语意清晰、逻辑统一、
> 无非必要静默操作；额外四项工程化诉求：① 设置页内置「日志」tab 供用户自查；② 每个重要功能
> 配「及时验证 + 反馈提示」机制；③ 设置项按需提供「恢复默认值」入口；④ 参数偏离默认值时有
> 加粗/变色提示；⑤ 模型下载做一套完备的用户可见闭环。本文 = 现状证据 + 分块方案 + 分期实施 + 验证方法。

> **实施状态（2026-09-07 晚，commit 1206d82）**：一期+二期全部落地，313 测全绿（基线 305+8），
> release 实机截图走查通过（日志页/测试连接失败红字/缓存卡片已缓存态/偏离徽标/8 tab）。
> 与本文差异登记：① 管道启动失败 = 面板识别页红字状态行（弃 rfd 弹窗，降打扰）；② 组级恢复按钮
> 简化为页级「恢复本页默认值」+ 组标题 ● 标识（实施中发现组级按钮与页内借用冲突，页级覆盖
> 用户诉求且更简洁）；③ 契约扩展（TranslatorUnavailable/TestTranslator/TestTranslatorResult）
> 已获用户批准实施；④ 新增 LIVETRANSLATE_SHOW_PANEL 环境变量（开发/截图便利，默认关闭）。
> 待后续：§4.3 前缀码改结构化枚举（三期可选）、设置保存失败 UI 流、P2-4 错误译文样式。

## 0. TL;DR

- **证据先行**：走读代码确认 4 个 P0（下载无反馈、翻译配置错静默关、管道启动失败无提示、
  引擎切换失败状态与行为矛盾）+ 9 个 P1/P2，全部有 `file:line`。
- **核心设计 = 一条不变量**：*任何用户可发起的动作，5 秒内必有一样用户可见的东西发生——要么是
  动作的结果，要么是明确的失败+下一步建议。* 所有方案围绕这条展开，不做多余的花哨。
- **五项方案**：
  - N1 设置页新增「日志」tab（复用现成 2000 行环形缓冲 + 全程 LogLine 广播流，零新数据源）；
  - N2 模型下载闭环（状态机+进度+失败分类+防重复点击，最重点）；
  - N3 恢复默认值（页级 + 组级两层，几何与 API 配置特殊处理）；
  - N4 偏离默认值提示（serde JSON 递归 diff → 路径集合 → 组标题 ● + label 变色，零逐控件侵入）；
  - N5 反馈矩阵：14 个重要功能的「动作 → 反馈」对照表（现状缺口 → 目标行为）。
- **分期**：一期（N2+N1+反馈矩阵中 P0 接线）≈4~5 天；二期（N3+N4+错误分类）≈3 天。每项列验证方法。
- **约束遵守**：lt-proto 契约不扩（错误分类走 DownloadFailed 字符串前缀码，合同仍是 String）；
  偏差一律登记「新增偏差」；i18n zh/en 同步；不把界面做复杂（无搜索、无取消下载按钮、无拖拽）。

## 1. 现状证据：问题清单（走读结论，行号为 2026-09-07 代码）

### 1.1 P0 级：用户动作无响应 或 状态与行为矛盾

| # | 现象（用户视角） | 证据（file:line） | 根因 |
|---|---|---|---|
| P0-1 | 点「下载模型」后界面纹丝不动，期间无进度无失败无完成，N 分钟后引擎自行切换 | app.rs:725-732 `DownloadProgress` 在 `StartupFlow::Ready => {}` 被丢弃；app.rs:734-748 `DownloadFailed` 同；app.rs:750-768 成功行同（仅 762 行「日志行」被吞，766 空转 tick）；main.rs:54 `spawn(..., false, ...)` 使 backend 的 first_launch 分支（backend.rs:102-107）成死代码 | D-19 首启直进主界面后 `AppState::new` 恒 `StartupFlow::Ready`（state.rs:1164），原给向导用的下载对话框流程（setup.rs download_missing_ui）从未接线 |
| P0-2 | 翻译配置填错（api_base 缺协议等）→ 翻译静默关闭，悬浮窗永远「翻译中...」 | pipeline.rs:169-178 `Translator::new` Err → `return None`（仅 error 日志）；pipeline.rs:1095 `let Some(rig) = tl else { return }`——AddMessage 已推，译文行永不更新，悬浮窗停留 t("translating")（overlay.rs:502-509） | 翻译装置构建失败只进日志，无用户路径 |
| P0-3 | 启动时管道失败（音频设备禁用/loopback 打不开等）→ app 打开但无效，面板还显示「引擎可用」 | shell.rs:40-43 `Pipeline::start` Err 仅 `tracing::error!`；面板状态（vad.rs:759 `engine_status_available`）只看模型缓存，不看运行态 | 装配失败无事件流 |
| P0-4 | 运行时切换引擎到未缓存/坏档 → 状态行红字「ASR unavailable」，但旧引擎继续出字幕，消息与状态矛盾 | pipeline.rs:807-814 切换目标不可用 → 只发 `AsrUnavailable`（旧 worker 未停，继续识别）；pipeline.rs:791-792 切换失败内部回滚旧 worker 后也发 `AsrUnavailable`（pipeline.rs:744 启动失败同） | 事件模型缺少「回退/旧引擎仍工作」的表述；`AsrUnavailable` 语义过载 |

### 1.2 P1 级：动作有歧义，或提示被降级到用户看不见的地方

| # | 现象 | 证据 |
|---|---|---|
| P1-1 | 悬浮窗「退出」秒退无确认；托盘「退出」弹确认——同一动作两种语义 | overlay.rs:195-197 `quit_requested=true` → app.rs:1295-1299 立即 `exit()`；托盘 app.rs:418-430 有 rfd 确认 |
| P1-2 | 首次隐藏悬浮窗的「已隐藏，右键托盘可恢复」提示只在日志（原版是托盘气泡） | app.rs:501-506 `hide_tray_hint` 仅 `tracing::info!`（tray-icon 无气泡 API 已知） |
| P1-3 | 「ASR unavailable」硬编码英文，且三种根因（未缓存 pipeline.rs:728 / worker 启动失败 pipeline.rs:744 / 模型目录不可用 pipeline.rs:695）共用同一文案 | app.rs:715 `Some("ASR unavailable".into())` |
| P1-4 | 导出空消息、打开目录/链接失败、设置保存失败：文案齐全但只进日志 | app.rs:1139-1141 `export_empty` 仅 info；panel/mod.rs:117-119、128-130 仅 warn；shell.rs:51-53/140-142/211-213 仅 error |
| P1-5 | 关闭面板窗口 = 静默隐藏到托盘，无任何告知（进程仍在跑） | app.rs:1243-1251 `CloseRequested` → `set_visible(id,false)` |
| P1-6 | 下载失败在运行时完全不可见：backend 的 `fail()`（backend.rs:213-215）连 `tracing::error!` 都没打，日志窗也看不到 | 对比：成功走引擎切换（shell.rs:241-248），失败无处可查 |

### 1.3 P2 级：语意含混（小改）

| # | 现象 | 证据 |
|---|---|---|
| P2-1 | 识别页引擎状态行文案自相矛盾：「引擎已就绪，切换后将自动下载模型」（未缓存时显示） | assets/i18n/zh.yaml:542 `engine_status_needs_model` |
| P2-2 | 手选未缓存 whisper 档位：立即生效语义缺失，仅 300ms 落盘，无「未生效/去下载」提示 | vad.rs:352-359（未缓存 → 仅 `mark_settings_dirty`） |
| P2-3 | 源语言设为非 auto 时语言不符片段全丢，零提示 | pipeline.rs:1074-1083 仅 info 日志 |
| P2-4 | 翻译错误 `[error: ...]` 渲染为正常译文样式，用户误当译文 | error.rs:37 `ui_text()` → pipeline.rs:231-236 → overlay.rs:518-519（颜色与译文相同） |
| P2-5 | 下载按钮无「进行中」态，可重复点击触发多个并发下载 | vad.rs:683-687 无状态检查；backend.rs:29-33 每来一个 StartDownload 起一个 Downloader |
| P2-6 | 下载错误无分类（HTTP 404 / 网络超时 / 磁盘满全压成一条字符串），UI 无法分支提示 | download/mod.rs:161 重试 4 次（1s/4s/16s 退避）；mod.rs:211 `bail!("HTTP {status}")` 只给数字；mod.rs:257-259 仅长度校验无 sha256 |
| P2-7 | 字幕页「恢复所有字幕设置为默认」原版确认文案存在但 Rust 零使用（孤儿 i18n 键） | assets/i18n/zh.yaml:340 `subwin_reset_confirm`；全仓无引用 |
| P2-8 | MS 侧缓存判定仅 `exists()` 无体积阈值，坏/半截文件被判「已缓存」 | cache.rs:116-119 |

### 1.4 派生事实（方案设计输入）

- **日志流是全量的**：`BroadcastLayer`（logging.rs:82-108）把 DEBUG+ 全部转 `LogLine` 事件（TRACE 旁路、naga=warn 已静音），桥接线程 `spawn_bridge`（logging.rs:127-147）恒常转发 → UI。日志窗已有 2000 行环形缓冲（state.rs:194-217 `LogWinState::MAX_LINES=2000`）。**「日志 tab」复用同一事件流与缓冲即可，零新数据源。**
- **设置默认值齐全**：`Settings::default()`（lt-proto settings.rs:87-128）、`Style::default()`（:326-346）、`SubtitleMode::default()`（:435-455）、`ModelConfig::default()`（:264-284）；`Settings` 已 derive PartialEq（settings.rs:40）——「偏离默认」比对有现成基础。
- **生效方式五类**（恢复默认必须按类重发命令，见 §4.3）：即时命令（SwitchEngine/SetAsrLanguage/SetPadding/IncrementalAsr/SetAudioDevice/SetMicDevice/SetTargetLanguage/SwitchTranslator，shell.rs:80-178）；300ms 防抖 ApplySettings（热应用范围窄，shell.rs:181-203）；600ms PromptApply（翻译页 system_prompt，state.rs:1454-1478）；窗口逐帧读取（style/subtitle 渲染键）；重启生效（ui_lang，main.rs:32-36）。
- **既有反馈先例**（新增一切提示都要对齐这些风格）：缓存 ✓绿/未缓存 ≈琥珀（vad.rs:659-691）；引擎状态行 绿/warn（vad.rs:750-762）；颜色非法红框（panel/mod.rs:264-290）；JSON 非法红字+确定禁用（translation.rs:303-310）。

## 2. 设计原则（用户思维，守住不复杂）

1. **动作必有反馈**：用户点任何按钮/开关/下拉，交互对象自身或相邻位置 5 秒内出现可见变化（状态文字/进度/变色/弹窗），失败时给出「下一步建议」而非甩英文错误。
2. **反馈分三档，不滥用弹窗**：
   - 档 1 = **就地反馈**（label 变色/加粗、按钮变灰、状态行文字）——覆盖 95% 场景，零打扰；
   - 档 2 = **就地横幅/卡片**（下载进度卡、失败卡、引擎状态）——面板组件内自足；
   - 档 3 = **模态确认**（仅三处：退出确认、恢复默认在<对 API 配置/几何有破坏性>时的确认）——误触代价高的才用。
3. **错误必分类**：用户能区分「网络问题」「仓库不存在」「磁盘满」并得到对应建议；无法分类的信息保留原文供日志排查。
4. **默认值可见可恢复**：偏离默认的控件**被动**有标识（不打扰），恢复动作**主动**就地提供（组级按钮），不搞全局「一键恢复」玄学。

## 3. N1 设置页「日志」tab

### 3.1 需求与边界

- 目的：小白用户在出问题时**自己打开设置 → 日志**，看到完整过程，可复制/打开日志文件报障。不与原版分离日志窗冲突：日志窗继续存在（悬浮窗入口不变），日志 tab 是同一数据的第二视图。
- 边界（克制清单）：**不做**搜索框、**不做**按 crate 过滤、**不做**导出按钮（给「复制」+「打开日志文件」）、**不做**自动暂停冻结。参数级过滤只做 4 档级别。

### 3.2 实现

- 新页签 `PanelPage::Log`（state.rs:883-891 `PanelPage::ALL` 追加，顺序放最后）；`tab_key` 补 i18n（zh/en 同步）。
- 数据源：**与日志窗共享** `AppState.logwin`（state.rs:194-217）——已是环形 2000 行、push 时按 `show_debug` 过滤、时间戳自动补（state.rs:208-217）。tab 视图照读 `logwin.lines`，额外渲染级别色（DEBUG 灰 / INFO 默认 / WARN 琥珀 / ERROR 红）、target 前缀列、时间列；**不修改缓冲结构**。
- 控件行（页顶水平条）：级别下拉（`logwin.show_debug` 即现有开关，文案「显示 DEBUG」）+「清空」（清 `logwin.lines`）+「复制」（`ctx.copy_text` 全量格式化，2000 行上限可控）+「打开日志文件」（`logs_dir` + explorer，复用 panel/mod.rs:112-120 `open_in_explorer`；先在日志文件按本次会话名 `livetrans_*.log` 找最新一个，找不到则开目录）。
- 渲染性能：日志行 2000 条 × 12px 等宽行内渲染在 egui 滚动区可承受（与日志窗同量级，日志窗已实装验证）；只在 tab 可见且 LogLine 到达时 `request_redraw`（现有 app.rs:793-795 已按 push 返回重绘，收益相同——LogLine 桥接是常驻的，不会漏）。
- 与日志窗差异：日志窗已有 show_debug 开关与自动滚动；tab 提供级别色 + 复制 + 打开文件（这三个是排查刚需）。

### 3.3 验证

- 单测：`push` 环形上限/过滤（现有 LogWinState 测试扩展）；`PanelPage::ALL` 含新页签且 key 可解析（panel/mod.rs:388 `all_tab_titles_resolve` 扩展）。
- 冒烟：`cargo run -p lt-app` 触发一次引擎切换/下载失败，设置页日志 tab 可见对应 WARN/ERROR 行且颜色分级；「打开日志文件」能打开最新 `livetrans_*.log`。

## 4. N2 模型下载闭环（重点）

### 4.1 目标态：用户看到的完整流程

```
识别页缓存组卡片（四态）：
  [模型名] ✓ 已缓存 (410.3 MB)          ← Cached，常态
  [模型名] ≈未缓存 (410.3 MB) [下载]     ← Missing，可点
  [模型名] 正在下载 34% (142/410 MB) [⋯日志展开]   ← Downloading，按钮禁用
  [模型名] 下载失败：[分类建议] [重试] [换下载源]   ← Failed，带动作按钮
下载完成（成功帧）：
  ✓ 已下载，正在加载模型... → 引擎热切换 → 状态行变「SenseVoice Small [cpu]」
```

### 4.2 状态机与数据结构（不动 lt-proto 契约）

`AppState` 新增 `download: DownloadUiState`（lt-ui 内部状态，不违反「UI 能力不扩契约」）：

```rust
enum DownloadUiState {
    Idle,
    Downloading {
        started: Instant,
        done_bytes: u64,
        total_bytes: u64,        // 从 repository 条目 estimated_bytes 预取；未知=0
        log: VecDeque<String>,   // 环形 200 行（复用 push_log_line 上限语义）
    },
    Failed {
        kind: DownloadErrKind,   // 前缀码解析，见 4.4
        detail: String,          // 原始错误串（展示细节/复制用）
        log: VecDeque<String>,
    },
}
```

事件接线：app.rs 现三处 `StartupFlow::Ready => {}` 丢弃分支改为写入 `state.download`：
- `DownloadProgress(line)`（app.rs:725-732）：`Downloading.log.push(line)` + 从行内解析进度数字更新 done/total（backend.rs:222-232 `format_event` 输出格式固定 `[{repo}] {file} {done} / {total}`——直接复用，不加新事件）；
- `DownloadFailed(e)`（app.rs:734-748）：`Failed{ kind: parse_kind(&e), detail: e }` + **补一条 backend 侧 `tracing::error!`**（backend.rs:213-215 先修——失败必须进日志）；
- `DownloadSucceeded{..}`（app.rs:750-768）：`Idle` + 卡片立即显示已缓存（磁盘探测下一帧自然翻转 vad.rs:646-651）+ 状态行随引擎切换（shell.rs:241-248 已有）变可用。
- `StartDownload` 发起时（vad.rs:683 前）：`state.download = Downloading{...}` 并**按钮禁用**（`download.is_downloading()`），防止 P2-5 重复触发；估算体积取 `registry` 条目 `estimated_bytes` 汇总（cache.rs 的 MissingModel 不带体积，从 registry 按 display/files 再取，或直接改 missing_models 补一个 `estimated_bytes: u64` 字段进 MissingModel——**lt-models 内部结构**，lt-proto 不涉及）。

### 4.3 失败分类（关键缺口 P2-6）

**不动 lt-proto**：`DownloadFailed(String)`（lt-proto events.rs:54）保持，错误文本加前缀码（例：`[net] 连接超时: ...`、`[http-404] HTTP 404`、`[disk] ...`、`[io] ...`），UI 端 `parse_kind` 解析前缀→枚举；未知/无前缀 → `Other`（展示原文）。Downloader 侧（lt-models download/mod.rs）在 4 个 bail 点（mod.rs:211 HTTP 状态、:198 网络、:187/228/247/254/261 io/磁盘、:258 长度）加统一 `DownloadError::kind()` 辅助函数产出前缀；重试次数、退避（mod.rs:161）不动。

分类 → 用户建议文案（i18n 新键，zh/en 同步）：

| kind | 显示 | 下一步建议 |
|---|---|---|
| `http-404` | 仓库不存在或已删除 | 切换下载源（Hub 下拉）重试 |
| `http-4xx/5xx` | 服务端拒绝（HTTP xxx） | 稍后重试 / 切换下载源 |
| `net` | 网络失败：连接超时/被拒/DNS | 检查网络；可在识别页代理设置（暂无控件→提示手改 settings.json，登记已知缺口） |
| `disk` | 磁盘写入失败 | 检查磁盘空间与配置目录权限 |
| `length` | 文件长度不完整 | 重试（续传已自动处理 416，mod.rs:204-210） |
| `other` | 原始错误串 | 复制日志报障 |

### 4.4 下载中卡片渲染（vad.rs 缓存组扩展）

- `CacheStatus` 枚举保持（vad.rs:184-192），卡片渲染先查 `state.download` 进行中/失败态再落到缓存态：
  - Downloading：进度条（egui `ProgressBar` 浅色 + 百分比 + 已下/总量 MB）+「展开日志」折叠行（复用 `push_log_line` 风格文本区）+ 下载中禁用按钮；
  - Failed：kind 文案（§4.3 表）+ 原文 detail（`\u{2026}` 截断 + hover 全文）+「重试」（重发 StartDownload）+「改下载源」（切到识别页 hub 下拉；不做新控件，就近提示一句话「可在上方下载源切换后重试」）；
  - 成功：已有 `✓ 已缓存` 自然出现，加一句引擎状态行（vad.rs:750-762）切换后绿色可用即闭环。
- 订阅窗口：下载中面板 tab 无需特殊重绘——LogLine 事件常驻桥接会 `redraw(Log)`（app.rs:793-795），但面板窗口需在 DownloadProgress 时 `request_redraw(Panel)`；在 app.rs:725-732 分支补 `self.redraw(WinId::Panel)`。

### 4.5 验证

- 单测：`parse_kind` 前缀表（含旧格式无前缀 → Other）；`DownloadUiState` 事件驱动转移（Ready 分支三处 → 状态断言）；download 的 4 个 bail 点带前缀（lt-models 测试补）；MissingModel 透传 `estimated_bytes`。
- 实测（需网络）：MockServer 或直接打真实源——① 下载中卡片进度推进、按钮禁用；② 断网时失败 → `[net]` 建议；③ 改错仓库（fake repo 404）→ `[http-404]` 换源建议；④ 重试成功闭环。

## 5. N3 恢复默认值（有分寸：页级 + 组级，二次确认限三处）

### 5.1 铺设面（基于 §1.4 全量盘点）

- **页级「恢复本页默认」按钮**（每页顶部右侧小按钮 `txt 恢复默认值`）：
  - 识别页：VAD 全套 + 引擎/模型/语言/设备/padding/hub（不含 models_dir、download_proxy——无控件项，手改 JSON 的问题不设按钮）；
  - 翻译页：`models = vec![ModelConfig::default()]` + `active_model=0` + `system_prompt=""` + `timeout=10` —— **加确认对话框**（抹掉用户 API 配置是全设置最「贵」的操作，且默认模型指向本地 LM Studio）；
  - 样式页：已有 `btn_reset_style`（style.rs:43-46,105-114，只覆 Style 14 键）——**升级为整页恢复**：补两把主字体键 + 联动 `apply_fonts`（style.rs:112 现有逻辑扩展）；几何**不纳入**（样式页已有独立「重置窗口位置」按钮，P2 建议分开）；
  - 字幕页：`*subtitle_mode = SubtitleMode::default()`（注意用 `default_pair()` 两行，不是单行 `SubtitleLine::default()`）+ 确认框直接用孤儿键 `subwin_reset_confirm`（zh.yaml:340）+ 联动 `apply_fonts`（行级字体跟随键变更）；窗口位置（window_x/y）不纳入。
- **组级「恢复本组」**：`group_card`（panel/mod.rs:212-239）标题行右侧，当本组存在偏离字段时才渲染小按钮。组 → 字段映射表在实现时按页定义（识别页 6 组、翻译页 3 组、样式页 N 组、字幕页 N 组），每组恢复仅写回组内键 + 按需重发对应即时命令。
- **不加**：Changelog/基准页（无设置）； models_dir/download_proxy（无控件）；几何（独立按钮已在）。

### 5.2 生效管道（关键正确性，来自 §1.4 生效方式盘点）

「恢复默认」**不能**只 `settings = default` + mark_settings_dirty（ApplySettings 热应用面窄，shell.rs:181-203）。必须按字段类别重发：

| 类别 | 恢复后动作 |
|---|---|
| 引擎/模型/语言 | `Cmd::SwitchEngine` + `SetAsrLanguage`（或经替换后下发） |
| 翻译模型/prompt/超时 | `Cmd::SwitchTranslator`（models 恢复后）+ PromptApply 路径 |
| 音频/麦克风 | `Cmd::SetAudioDevice`(SystemDefault) + `SetMicDevice`(Off) |
| padding/增量 | `Cmd::SetPadding` + `IncrementalAsr` |
| VAD 全套 | 依赖 ApplySettings 热应用（已有），确认下发 |
| 字体/样式/字幕 | `fonts::apply_fonts(ui.ctx(), ...)` + 面板逐帧渲染自然生效 |
| 常规落盘 | `mark_settings_dirty` 统一收尾 |

实现建议：`state.rs` 加 `fn reset_page_defaults(page: PanelPage, state: &mut AppState)` + `fn reset_group_defaults(group: &str, ...)`，两函数内部按上表发命令——**与面板现有 `mark_settings_dirty` 单入口同构**，便于测试。

### 5.3 验证

- 单测：`reset_page_defaults` 后 `settings == Settings::default()` 对四页各自成立（仅断言被覆盖键域）；`Settings` PartialEq 已具备；组级恢复幂等。
- 冒烟：恢复识别页 → VAD 滑条回位、引擎回 funasr/sensevoice-small、状态行变「引擎可用」；恢复样式页 → 字体回思源且预览字体重载；恢复字幕页 → 两行（原文 24/译文 28 金）回默认且确认框文案正确；恢复翻译页 → 确认框出现，确认后模型列表剩一行 LM Studio 默认。

## 6. N4 偏离默认值提示（被动标识，不打扰）

### 6.1 选型（为什么不用逐控件手改）

逐控件传「默认值/当前值」要动全 panel 每个控件签名（200+ 调用点，侵入大、容易漏）。选 **集中 diff 方案**：一次递归比对，产出「偏离路径集合」，页面渲染时按路径给 label / 组标题着色。未来新增字段自动覆盖（diff 算法基于 serde 序列化，不感知字段），单测集中一处。

### 6.2 实现

- `crates/lt-ui/src/panel_diff.rs`（新模块）：
  ```rust
  /// serde_json::Value 递归 diff：返回偏离字段路径（"style.bg_color" / "subtitle_mode.sentences" / "models[0].api_base"）
  pub fn diff_settings(a: &Value, b: &Value) -> Vec<String>;
  pub fn diff_paths(settings: &Settings) -> Vec<String> { diff_settings(&default_value(), &to_value(settings)) }
  ```
  规则：对象键对称差 → 路径；数组：长度不同记 `<arr>[i]`、元素逐位递归；`None` 与 `Some(default)` 等价的对象键（geometry 类）视为相等（默认值比对按 serde 自然序列化等价处理——`Option::None` 序列化为 `null`，`overlay_x=None` 就是默认，直接天然一致）。
- `panel/mod.rs`：每帧 `panel_ui` 开头算一次 `diff_paths`（2000 行级 Settings JSON 递归，微秒级，无性能顾虑）；按页过滤出 `own_diffs` 存 `AppState.panel.diff_cache`（帧间复用，消息：
  - `group_card` 标题行右侧，若路径前缀命中本组 → 显示 `●`（accent 色）小圆点 + tooltip「有参数偏离默认值，可点「恢复本组」」+ 组级恢复按钮（§5.1）；
  - `form_row` 的 label 若命中路径 → 颜色 `pal.weak` → `pal.accent`（蓝）并加粗（`RichText::strong`）；组合控件（combo/slider）不单独处理——label 反馈已足够，**不做**控件描边（防视觉噪音）。
  - 隐藏型偏离（models_dir/download_proxy 无控件）标出但无对应控件——**列出到识别页底部「高级提示行」**（弱文字一行，仅供知道有此设置，不渲染标记）。
- i18n：新增键 `diff_default_tooltip`「此参数偏离默认值」、`diff_group_tooltip`「本组有参数偏离默认值」、`restore_group`「恢复本组」，zh/en 同步。

### 6.3 验证

- 单测：`diff_settings`（合法路径样例：改 bg_color/加 models/删 subtitle 行/重置回默认 → 空集）；`Settings::default()` 自比 = 空集（回归防线）。
- 冒烟：改 vad_threshold → 组标题 ● 出现 + label 变蓝；按「恢复本组」→ 标记消失；`ui_lang=zh` 亦正确标记（重启生效项也视为偏离——提示上 hover 文案建议联动「重启生效」提示，两个 hint 合并显示）。

## 7. N5 重要功能「验证+反馈」矩阵（现状 → 目标）

| 功能 | 用户动作 | 现状反馈 | 目标反馈（实现位置） |
|---|---|---|---|
| 启动 | 双击 exe | 无 wizard，模型缺 → 悬浮窗英文 unavailable（P1-3） | 面板识别页状态行「引擎可用/需要模型」+ 缺模型时缓存组常亮琥珀（**已有** vad.rs:750-762，仅需 P1-3 文案修复 + 启动失败 P0-3 接线） |
| 启动失败 | 同上 | 全静默（P0-3） | 弹一次消息框（rfd，复用托盘退出同库）：「音频设备启动失败：<原因>」+ 面板状态行红字（shell.rs:40-43 → 事件 on_err → app.rs 悬挂提示，项目已有 MessBody 惯例：bench_done_title 等 rfd 窗） |
| 模型下载 | 点下载 | **无任何反馈**（P0-1/P2-5/P2-6） | §4.2-4.4 整条闭环 |
| 引擎切换 | 识别页切引擎 | 成功→加载框+状态行；失败→矛盾红字（P0-4） | 失败改发「切换失败，已回退到 <旧名>」提示（事件加旧名或状态行文字化），关掉矛盾；未缓存切换 → 缓存组琥珀+下载按钮（已有） |
| ASR 识别 | 说话 | 悬浮窗消息流（已有） | 不变（已良好） |
| 翻译失败 | 配置错/API 500 | **静默或假译文**（P0-2/P2-4） | P0-2 构建失败 → 首个错误发生即状态行红字「翻译组件未就绪：<原因>，去翻译页检查」；运行期错误 `[error: ...]` 改样式（error 红色 + 斜体，overlay.rs:518-519 按前缀分段渲染）；翻译页加「测试连接」按钮（复用 SwitchTranslator，完成即 AsrDevice 类回调——**归二期预留**，本期只做错误可见） |
| 目标/源语言 | 悬浮窗下拉 | 即时生效无提示（settings 行变化） | 下拉选择后状态行文案已随 tray_status_format 更新（app.rs:439-458 已有），保持 |
| 音频设备 | 识别页下拉 | 即时切换，失败静默 | 切换失败 → 状态行红字（backend.set_device 返回值接事件，pipeline.rs:483-492 补） |
| 暂停/恢复 | 悬浮窗启停/托盘 | 按钮变「暂停」+ 图标变（已有） | 不变 |
| 字幕开关 | 悬浮窗字幕按钮 | 开/关即时（已有） | 不变 |
| 清空列表 | 悬浮窗清空按钮 | 立即清空无确认（可恢复性低——transcripts 落盘点） | 加一次 rfd 确认（若 auto_save on 则免确认——按 transcripts 存在与否），登记小偏差 |
| 导出 | 右键导出 | 空列表仅日志（P1-4） | 空 → rfd 提示「没有可导出的内容」（复用 export_empty 文案，改弹窗）；成功 → 弹窗显示路径（可选，本期只做空提示） |
| 设置保存 | 改任意项 | 300ms 静默落盘（原版同） | 保存失败 → 状态行红字「设置保存失败」（shell.rs 三处 error 加 UI 事件流）；成功保持静默（档 1 原则下保存成功=正常，不打扰） |
| 退出 | 托盘/悬浮窗 | 托盘确认、悬浮窗秒退（P1-1） | 统一：悬浮窗退出也弹确认（与托盘同款 rfd） |
| 隐藏悬浮窗 | ✕/托盘 | 首次仅日志（P1-2） | 首次隐藏 → rfd 一次性提示「悬浮窗已隐藏，右键托盘图标可恢复」+ 不再复弹（overlay_hide_notified 沿用） |
| 字幕窗自动隐藏 | 设置项 | 有（原版同） | 不变 |

## 8. 实施分期（每期收工 = 测试全绿 + 冒烟）

### 一期（反馈闭环骨架 + 最高优先 P0；≈4~5 天）

1. **N2 下载闭环**（§4）：backend `fail()` 补 error 日志 + 前缀码 + UI 状态机 + 卡片四态 + 防重复点击 + MissingModel.estimated_bytes。含修复 P0-1、P1-6、P2-5、P2-6、P2-8（MS 判定加阈值，顺手）。
2. **N1 日志 tab**（§3）：新页签 + 共享 logwin + 复制/打开文件/级别过滤 + i18n。
3. **P0 快速接线**（各自 0.5 天内）：
   - P0-2：`TlRig::from_settings` None → 新增 `UiEvent::TranslatorUnavailable(reason)`（**契约扩展，需用户表态**——否则用现有路径：状态行借用 `AsrUnavailable` 语义 + 把原因写进 `DownloadProgress` 类日志行并由状态行显示；推荐前者，一个枚举变体的成本可控）；
   - P0-3：`Pipeline::start` Err → rfd 启动失败框 + 面板状态行红字；
   - P0-4：切换失败/未缓存 → 状态行文字区分「已回退」vs「待下载」；P1-1/P1-3/P1-4/P2-1/P2-4 文案与确认统一；
   - P2-3 语言过滤丢弃 → 悬浮窗「🔇 语言不符已跳过 N 条」小计（状态行右侧计数，可选，**本期可缓**）。

### 二期（默认值可见性；≈3 天）

4. **N4 偏离提示**（§6）：panel_diff.rs + 组标题 ● + label 变色 + i18n。
5. **N3 恢复默认**（§5）：页级 4 按钮 + 组级按钮 + 确认框（翻译页/字幕页）+ 生效管道（§5.2 命令重发表）。
6. 二期收尾：字幕窗/悬浮窗「测试连接」按钮（翻译页，可选）。

### 三期（可选，待用户点头，风险高）

- DownloadErrorKind 结构化（扩 lt-proto）取代前缀码；代理设置面板控件（download_proxy 目前无入口）；Sha256 校验登记（registry 渐进）。

## 9. 风险与约束

| 项 | 处理 |
|---|---|
| lt-proto 契约冻结（AGENTS 硬约束） | §8.3 中 P0-2 的**唯一**契约扩展（TranslatorUnavailable）需用户批准；其余全部用现有事件 + 字符串前缀码实现。批准前提 = 事件枚举加一个变体、UI 分支 10 行级改动。 |
| 与原版 1:1 复刻的张力 | 本计划全部为**新增反馈**（原版无声处有声），不改变既有行为语义（音量/阈值/缓存判定值不动）；偏差登记：首启无向导已 D-19；本计划新增「日志 tab/恢复默认/偏离提示/下载闭环」四项登记为「新增能力偏差」，归 distribution 的 D-20 同类。 |
| 性能 | 日志 tab 2000 行渲染与日志窗同量级（已实装）；diff 递归为微秒级；下载进度仅面板窗口重绘（事件驱动，无节拍）。 |
| i18n | 新增键全部 zh/en 同步（预计 +18 键）；「ASR unavailable」改 i18n 键 + 三种根因拆文案。 |
| 复杂度控制 | 严格按「三档反馈」执行：新增提示里 1 个（启动失败）、2 个（退出/清空/初次隐藏）、1 个（模型恢复）为弹窗/确认，其余全部就地；不做搜索/取消下载/自动冻结。 |

## 10. 验收清单（收工口径）

- [ ] `cargo test --workspace` 全绿（含新增 diff/download kind/状态机/页签解析测试；305 测基线 +30~50 测）；
- [ ] 冒烟全项：见 §3.3/§4.5/§5.3/§6.3；
- [ ] 设置页 8 个 tab 渲染无滚动异常（panel_ui_smoke 扩展）；日志 tab 打开时 app 空闲 CPU 不变（≤1% 基线）；
- [ ] 中文环境全流程（无英文硬编码残留，grep "ASR unavailable" 归零）；
- [ ] 下载三场景实机（断网/404/成功）走查通过；
- [ ] 文档：本计划随实现同步更新；偏差登记入 docs/distribution.md §7 风格列表。
