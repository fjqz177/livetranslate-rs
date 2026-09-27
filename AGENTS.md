# AGENTS.md — LiveTranslate-rs 工作区总纲

> **本文是什么**：指令体系 L0 = 元方法层。只收三类内容——**① 不读就会做错事的硬约束**（必须 / 禁止）、**② 索引**（去哪儿读：路由表、真源指针、看板指针）、**③ 方法**（怎么干活：冷启动、流程卡、纪律、文风）。
> **准入三问**（往本文加一行 / 一节前先过，答错即下沉）：① 不读会犯错吗？（答「否」= 属于「读了才知道」的事实 → 下沉）② 有唯一真源在别处吗？（答「是」→ 只留指针，不留复述）③ 每个工作包都会变吗？（答「是」→ 迁活文档，本文只留机制与指针）。
> **渐进披露链**：L0 本文（冷启动通读一遍，分钟级）→ L1 常青页与流程卡（`docs/`，按 §4 触发词按需读）→ L2 决策史与机器真源（`docs/archive/`、`crates/lt-app/tests/topology.rs`、`clippy.toml`）→ L3 未拍板草稿（`docs/drafts/`，不入库）。**禁把 L1/L2 内容抄回 L0**——准入三问是唯一例外通道。
> **冷启动三步**：① 通读本文 → ② 打开 `docs/board.md`：命中「待拍板」的工作未点头禁动工 → ③ 开工先过 `docs/prompts/kickoff.md`。
> **改文纪律**：改本文须在 commit message 说明信息去向（删了什么 → 迁去哪）；不设机械额度（ADR-18）——防回胀靠准入三问、§4 路由覆盖与评审漂移自查；§7 只引用 `docs/gotchas.md` 实存的 G-编号（禁死引用）；§8 三小节标题不得缺——`scripts/check_agents_health.ps1` 机械守护。
> **生效**：2026-09-14 起取代复刻期总纲（ADR-15/16）；本次元方法化 = ADR-22。登记见 `docs/decisions.md`，旧版与迁移对照见 git 历史。

## 1. 项目与硬约束

- **这是**：Rust 原生实时音频翻译应用，Windows 单 exe。Python 原版 LiveTranslate 仅行为参考（1:1 复刻期 2026-09-07 收尾；阶段二以产品体验为准）；2026-09-19 已公开发布（D-90 发布链；分发现状真源 = `docs/distribution.md`）；2026-09-22 起零代码整理阶段（D-96）——只做文档治理与流程严谨化，**改码须用户明示解禁**。
- **纯 CPU**：禁 CUDA / DirectML / GPU；whisper GPU feature 禁用；MonitorBar GPU 恒 N/A。
- **单 exe 分发**；配置目录 = `~/.config/livetranslate`（Windows 下字面 `home/.config`，不是 `%APPDATA%`）；`settings.json` 的 `models_dir` 键可指定模型缓存路径。
- **远程 ASR 已整体裁剪**（remote-whisper 引擎与相关设置键全删）。
- **仅 Windows 实装**（wasapi 采集）；AudioBackend trait 已留跨平台抽象缝，macOS / Linux 后端未实装。
- **参考副本** = 工作区 `LiveTranslate/`（gitignored，扁平结构，与任何外部仓无关）；改 GUI 前可回读其 `main.py` / `subtitle_overlay.py` / `subtitle_window.py` / `control_panel.py` / `vad_processor.py`。
- **决策登记**：行为 / 产品差异落 **D-xx**、架构差异落 **ADR-x**；先登记 `docs/decisions.md` 后引用；被推翻加「→ 被 D-xx 取代」不删行（编号 / 局部号 / 维护时机细则真源 = `docs/README.md` 纪律⑥ + 该表头注）。

## 2. 命令与门禁

> 首次 / 换机环境（pwsh 7、`uv sync` 钉版 libclang + cmake 入仓库内 `.venv`、`scripts/fetch_sherpa_libs.ps1` 预取 sherpa 库）与日常命令的完整版 = 根 `README.md`「给开发者」§二；脚本参数 / 前置 / 退出码 = `docs/scripts.md` 与各脚本头注（**唯一细节真源**）。

```bash
cargo test --workspace                 # 收工门禁（不在 precommit 内）：全量测试全绿；真模型/真网络探针 #[ignore] 默认跳过
cargo build --release -p lt-app         # 单 exe：target/release/livetranslate.exe（~76MB，滚动值）
cargo run -p lt-app                     # GUI 冒烟；冒烟全套 = docs/prompts/live-check.md
pwsh -File scripts/precommit.ps1         # 提交门禁：三项清单真源 = 该脚本头注（ADR-21）
pwsh -File scripts/package_release.ps1   # 打包 dist/LiveTranslate-*.zip（发布路线 = docs/distribution.md）
pwsh -File scripts/release.ps1 <动词>    # 发布链引擎：rehearse 空跑；真发布 = release → promote（转正永远人工）
git config core.hooksPath .githooks      # 每 clone 一次：钩子触发面 = 「门禁读谁、谁触发」（D-93）
```

- **门禁单点**：`precommit.ps1` 三项（fmt → 总纲健康 → clippy，ADR-21）为提交前唯一闸；`ci.yml` 与 `.githooks` 同源调用它，增删门禁只改这一处（D-88 废双边手抄）。触发面外路径（`assets/`、`Cargo.lock`、`CHANGELOG*` 等白名单）放行 **≠** 内容免检——放行面提交也自觉跑（G-28）。
- **CI** = `.github/workflows/` 三 workflow（结构真源 = 各文件头注）：**ci**（单 job：整脚本入 CI + 测试 + 分发演练；push 全分支 + PR + 手动〔D-89〕）/ **release**（tag `v*` 或手动演练 → 发布链全链出草稿 + sha256，转正人工〔D-90〕）/ **security**（cargo-deny 四表，ubuntu）。工具链真源 = `rust-toolchain.toml`。**无分支保护**——CI 红叉不拦合并，靠提交前自查；CI 绿 ≠ 干净机能跑（runner 自带 VC++）。
- **新增 / 改名 / 删脚本**须同步 `docs/scripts.md`（守护断言 8 双向对账）；改码前摸底优先 MCP `codegraph_explore`（`.codegraph/` 本机索引，不入库）。

## 3. 十 crate 拓扑与分层硬规则

| crate | 职责一句话 | 允许内部依赖 |
|---|---|---|
| lt-proto | 事件/命令/数据契约 + 翻译域常量（E3/ADR-10） | — |
| lt-i18n | zh/en 双语键集（两份 yaml 键集必须一致） | — |
| lt-models | 模型注册表/ModelConfig/缓存探测/Settings 读写 settings_io（零网络；Settings 与值域透镜本体在 lt-proto） | proto |
| lt-download | 下载器：reqwest/sha2/退避/完整性 | proto |
| lt-audio | wasapi 采集 + silero VAD + ORT 内嵌 | models |
| lt-asr | ASR worker 子进程 + IPC + Job Object 孤儿兜底 + 四引擎（funasr 键 = SenseVoice/Nano 两族，另有 Whisper/Qwen3；配置经 stdin 首行，D-76） | proto（models 仅 dev-dep） |
| lt-translate | async-openai LLM | proto |
| lt-orchestrator | 编排域：识别/翻译管道 + 线程监督器 + 下载管理 + 日志桥；禁依赖 ui/winit/i18n，用户文案经 `Msg` 注入 | proto, models, download, audio, asr, translate |
| lt-ui | egui 多窗口：悬浮/字幕/控制面板/日志窗/托盘（纯投影 crate，禁依赖 lt-app） | proto, i18n, models |
| lt-app | 组合根：boot + 动脉桥 + 命令路由 + 单实例消息窗 + worker 入口（同 exe `--asr-worker` 自拉起） | 十库全部 |

- **机器真源**（不读会撞墙，改代码前先看这里）：依赖白名单 = `crates/lt-app/tests/topology.rs`（含 dev-dep 特批表；cargo test 承载，ADR-21）；源码禁令（裸线程 / 直发 proxy / panic hook 位置）= 根级 `clippy.toml`（disallowed-methods，豁免就地 `#[allow]` + 理由）；文本卫生与个人路径 = `crates/lt-app/tests/repo_hygiene.rs`；契约纯度（禁 `serde_json::Value` 裸载荷）= `crates/lt-proto/tests/contract_purity.rs`。拓扑终局 = `docs/archive/architecture-v2.md` §3.1。
- **lt-proto 冻结规则**：`PROTO_VERSION` 随结构变更递增（当前 7）。豁免评审 = 纯新增 Cmd/UiEvent/AppCommand 变体、纯新增 Settings 字段（须 serde default 兼容旧档）、既有枚举增项；仍须评审 = 删除 / 改名 / 改型 / 改语义任何既有契约项（记录入 D-xx）。**跨 crate 字符串编码协议（前缀 / 分隔符 / 哨兵值）一经发现按 P1 立案**。
- **新增 UI 能力不得扩 lt-proto 契约**：日志经 `LogLine{target}` 回流。
- **设置与值域**：Settings 运行时落盘唯一通道 = `Cmd::PersistSettings`（shell 直排 + 发布总线，300ms debounce 对齐原版）；命令草稿写入唯一落点 = shell `apply_settings_side_effects` 纯函数（E1-4），UI 只发命令不预写；值域判定一律走类型化透镜（`Settings::engine_key()/hub()/proxy_mode()` + EngineKey/Hub/ProxyMode，E2/D-79），域内禁 `== "funasr"` 类字面量比较（lt-asr / lt-models / lt-app worker 分派为单点分派边界豁免）。
- **窗口动作**：UI → 宿主统一走 **`WinAction` 意图通道**（`crates/lt-ui/src/state.rs`，跨平台抽象缝）；新增窗口行为优先加变体让宿主执行，不在 UI 侧散点直调 Win32。

## 4. 路由表（动手前必读；本表 = 提示词卡唯一索引）

> 卡（`docs/prompts/`）= 各阶段强制自查清单：出口条件不满足不得进入下一阶段。

| 场景 | 必读 |
|---|---|
| 任何工作包动第一行代码前 | `docs/prompts/kickoff.md` |
| 施工中写码自查（分层/测试/提交） | `docs/prompts/implement.md` |
| 改 GUI / 窗口 / 托盘 / Win32 / 字体渲染 | `docs/gotchas.md`（触发词索引）+ `assets/reference/` 参照截图（zh/en 各 10 张） |
| **任何与预期不符的怪现象**（编译 / 链接 / 渲染 / 托盘 / 测试 / 脚本 / CI / 磁盘） | `docs/gotchas.md`（触发词五段式：触发/症状/根因/对策/证据；踩新坑当场记候选） |
| **当前待拍板 / 施工中 / 遗留** | `docs/board.md`（§8 只留指针；未点头禁动工） |
| **首次 / 换机环境、日常命令** | 根 `README.md` §二 + `docs/scripts.md` |
| **资产 / 字体 / 参照图来源** | `assets/SOURCES.md` + `docs/scripts.md` |
| **架构分层细节 / 依赖白名单** | `crates/lt-app/tests/topology.rs` + 根 `README.md` §二5 |
| 动契约（lt-proto / Settings 结构） | 本文 §3 冻结规则 + `docs/decisions.md` |
| 翻译供应商 / LLM 接口改动 | 本文 §6 翻译域 + `docs/archive/llm-api-round2.md`（四厂商对照表） |
| 模型加载 / 校验 / 缓存探测改动 | 本文 §6 模型信任 + `docs/archive/model-trust-repair.md` |
| 立档 / 归档 / 收口 | `docs/README.md` 顶部纪律条文（唯一真源）+ `docs/prompts/closeout.md` |
| 评审 / 代码自查 | `docs/prompts/review.md`（八原则） |
| 需要用户裁决 | `docs/prompts/decision-request.md` |
| 实机走查 / GUI 冒烟取证 | `docs/prompts/live-check.md` |
| 会话收尾交接 | `docs/prompts/handoff.md` |
| 写 / 改守护脚本 | 本文 G-20（grep 方言）+ G-23（文本卫生）+ G-30（定义源跳过自扫）+ G-33（节截取正则须行首锚定）+ 各守护脚本头注（风格对齐） |
| 查脚本用法 / 新增脚本 | `docs/scripts.md` + 各脚本头注 |
| 分发 / 打包 / 发布 | `docs/distribution.md` + `scripts/release.ps1` / `scripts/package_release.ps1` 头注 |
| 历史工作包依据（下载器 / ASR 加固 / 视觉…） | `docs/archive/`（一行一档索引 = `docs/archive-index.md`，只读） |

## 5. 纪律速记（docs 目录治理九条真源 = `docs/README.md` 顶部；git / 文风 / i18n 等真源在各自锚点，此处只留最绑定的）

- **主干直推**：提交直接落 main，无 PR 流程（CI 全分支 push 兜底）。
- **提交**：里程碑 + 全绿即自主中文 commit（`feat(scope): 中文主题`）；收尾逐项显式 pathspec，禁 `git add -A` / `git add .`（G-19）；生成副产物不入库。
- **定稿先行**：`docs(scope)` 定稿提交必须早于第一行实现代码；草稿只进 `docs/drafts/`（不入库）；决策号（D/ADR）随定稿**同一提交**登记 `docs/decisions.md`。
- **收口**：一气呵成 + 收口三问（全文 = `docs/README.md` 纪律⑤ + `docs/prompts/closeout.md`）。
- **文本卫生**：全仓文本无 BOM 合法 UTF-8 + LF（ADR-20，`crates/lt-app/tests/repo_hygiene.rs` 机械拦截）。
- **i18n**：zh/en 两份 yaml 必须同步修改（键集一致）。
- **文风**：结论先行、判对项与待拍板项分列、最直白零废话（评审全文风 = `docs/prompts/review.md`）。

## 6. 域速记（各处细裁的既往裁决——只留「不知道就会做错」的一句，细节追真源）

- **翻译域**（D-82/D-85）：对外只给翻译接口、内部消化供应商差异；**UI 与后端必须一比一同步**；高级参数默认一律不发送（用户显式指定才发）；失败兜底终点 = 最小请求（保留系统提示词）；报错必须与译文一眼可辨；费用按本次运行累计、重启清零。→ `docs/archive/llm-api-round2.md`
- **模型信任**（D-83）：凡加载必验 sha256；坏文件隔离 + 自动重下（3 次后提醒用户）；不做版本钉死；只验正在加载的模型。→ `docs/archive/model-trust-repair.md`
- **ASR 既往裁决**：SenseVoice 输出无语言标签 → 三优先 resolve（显式设置 > 模型标签 > 启发式，D-30）；qwen3 生效段长钳制 ≤15s 且 overlay 归一不写穿用户设置（D-28/D-69）；interim 裁剪带代际校验（D-27）；client 非 Worker 错误自动 recover 重启（D-26）；下载源诚实（无 MS 源不伪造 ms 字段，D-24）。→ `docs/decisions.md` 按号查 + `docs/archive/` 各档
- **字体与渲染**（D-17）：默认全盘内嵌思源三字体（系统字体仅锦上添花，禁依赖）；行级字体键空串 = 级联跟随 `subtitle_font_family`；改字体键 → 立即 `fonts::apply_fonts` + 防抖落盘；渲染侧禁 `FontFamily::Name` 臆造，一律经 `fonts::font_family_for`（G-12）。→ `docs/archive/font-system.md`
- **窗口透明**：字幕窗/悬浮窗的逐像素透明在当前栈**不可达**（wgpu HWND 仅 Opaque）——半透明走整窗 alpha、圆角走 `SetWindowRgn`，禁改 LWA_COLORKEY 抠色（±1 抖动）。→ 机制与对策 = `docs/gotchas.md` G-34；裁定 = D-36。

## 7. 大坑防呆（只留「不读会静默踩」的 5 条；全本 = `docs/gotchas.md` 触发词五段式，禁死引用——本节引用的每个 G-编号必须实存）

1. G-1 egui 多窗 repaint 回环：`RedrawRequested` 不得再转发 `request_redraw`——否则 1350fps 自旋、其他窗口饿死白屏。
2. G-5 构建环境双保险**勿动**：`.cargo/config.toml`（双栈 CRT）+ 仓库内 `.venv`（libclang / cmake 相对指向）——构建报错先想「是不是动了它」。
3. G-9 磁盘满两症：C 盘满 → 测试 TEMP 报 StorageFull / GDI+ 假死；D 盘（构建盘）满 → os error 112。
4. G-12 `FontFamily::Name(族名)` 未注册即 panic：渲染一律经 `fonts::font_family_for` 取。
5. G-14 事件循环线程禁同步 MessageBox / 模态（含 rfd MessageDialog）：用 egui 内嵌模态或原生通知。

## 8. 看板（机制：一行一包、清零即删；**条目真源 = `docs/board.md`**，本区只留机制与指针）

### 待拍板（等用户裁决；草稿在 `docs/drafts/` 不入库）

- 真源 = `docs/board.md` §待拍板。**命中任一条目 = 未拍板，未点头禁动工**（先走 `docs/prompts/decision-request.md`）。

### 施工中

- 真源 = `docs/board.md` §施工中（有包必列，完工即挪出）。

### 遗留（完工包的实机走查与未了项，一行一包，清零即删）

- 真源 = `docs/board.md` §遗留（每行 = 一包 + 归档档锚点）。

## Agent skills

> Matt Pocock 工程技能套件的仓库级装机配置；三个子节各指向 `docs/agents/` 下唯一真源，细节改那里，本节只留一行指针。

### Issue tracker
工单池 = 本仓 GitHub Issues（`gh` CLI 读写）；`docs/board.md` 仍是当前施工唯一真源，分工见 `docs/agents/issue-tracker.md`。

### Triage labels
五分诊标签沿用默认串（needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix），映射表 = `docs/agents/triage-labels.md`。

### Domain docs
single-context：根 `CONTEXT.md`（按需惰建）+ 决策真源 = `docs/decisions.md` 台账（套件所说 "ADR" ≙ 台账条目，不另设 ADR 目录）；消费规则 = `docs/agents/domain.md`。
