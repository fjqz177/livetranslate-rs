# AGENTS.md — LiveTranslate-rs 工作区总纲

> **本文是什么**：常驻约束 + 路由表（指令体系 L0）。细节活在 docs/，按 §4 路由按需读，不通读不背。
> **生效**：2026-09-14 起取代复刻期总纲（ADR-15/16，登记见 docs/decisions.md；旧版与迁移对照见 git 历史）。
> **冷启动三步**：① 通读本文件 → ② 对照 §8 看板：命中「待拍板」的工作未点头禁动工 → ③ 开工先过 `docs/prompts/kickoff.md`。
> **改文纪律**：改本文须在 commit message 说明信息去向（删了什么 → 迁去哪）；预算两档现值唯一真源 = `scripts/check_agents_health.ps1` 断言 1（加字优先以回吐支付）；§7 速查只引用 `docs/gotchas.md` 实存的 G-编号（禁死引用）；§8 三小节标题不得缺——`scripts/check_agents_health.ps1` 机械守护。

## 1. 项目与硬约束

- Rust 原生实时音频翻译应用（Windows 单 exe）。Python 原版 LiveTranslate 仅行为参考，1:1 复刻期已于 2026-09-07 收尾；当前阶段二：以产品体验为准，原版有/没有不再是取舍依据。
- 参考副本 = 工作区 `LiveTranslate/`（gitignored、扁平结构，与任何外部仓无关）。改 GUI 前可回读其 `main.py` / `subtitle_overlay.py` / `subtitle_window.py` / `control_panel.py` / `vad_processor.py`。
- **纯 CPU**：禁 CUDA/DirectML/GPU；whisper GPU feature 禁用；MonitorBar GPU 恒 N/A。
- **单 exe** 分发；配置目录 = `~/.config/livetranslate`（Windows 下字面 home/.config，不是 %APPDATA%）；`settings.json` 的 `models_dir` 键可指定模型缓存路径。
- 远程 ASR 已整体裁剪（remote-whisper 引擎与相关设置键全删）。
- 当前仅 Windows 实装（wasapi 采集）；AudioBackend trait 已留跨平台抽象缝，macOS/Linux 后端未实装。
- 行为/产品差异落 **D-xx**、架构差异落 **ADR-x**：先登记 `docs/decisions.md` 后引用；下一号 = 表尾+1；禁预留号段；被推翻加「→ 被 D-xx 取代」不删行；已登记条目、弃用空段（D-38~59）与维护时机见该表头注。

## 2. 命令与门禁

```bash
winget install Microsoft.PowerShell            # 首次/换机前置：装 PowerShell 7（Windows 不自带 pwsh；CI runner 已
                                               # 预装）——本仓全部脚本 pwsh-only（ADR-16）
uv sync                                        # 首次/换机：钉版 libclang 18.1.1 + cmake 4.4.3 装入仓库内 .venv，
                                               # cargo [env] 相对指向——clone 后零本机路径配置，勿改绝对路径
pwsh -File scripts/fetch_sherpa_libs.ps1       # 首次：预取 sherpa 预编译库到 .cache/sherpa-onnx（缺失构建硬报错；
                                               # GitHub 慢用 -Mirror <前缀>/SHERPA_ONNX_MIRROR 走 Release 镜像）
cargo test --workspace                         # 收工门禁（不在 precommit 内）：全量测试，滚动基线 628+9 绿；
                                               # 9 ignored = 真模型/真网络探针离线纪律，CI 保持跳过
cargo build --release -p lt-app                # 单 exe：target/release/livetranslate.exe ~76MB（滚动值）
cargo run -p lt-app                            # GUI 冒烟
pwsh -File scripts/package_release.ps1         # 打包 dist/LiveTranslate-*.zip（CI 同源；发布路线 = docs/distribution.md）
pwsh -File scripts/release.ps1 <动词>          # 发布链九动词引擎（D-90/D-91；本地/CI 同一份）：rehearse 空跑全链不发布；
                                               # 真发布 = release → promote（正文自动取 CHANGELOG 本版段落；云端等价 = 推 v* tag，转正永远人工）
pwsh -File scripts/precommit.ps1               # 提交前门禁：七项清单真源 = 该脚本头注；ci.yml 与 .githooks
                                               # 同源调用本脚本，门禁增删只改这一处（D-88 废除手抄双边同步）
git config core.hooksPath .githooks            # 每 clone 一次启用提交钩子；触发面 = 守护读谁、谁触发（D-93）：
                                               # 暂存区命中 .rs / Cargo.toml / .cargo / AGENTS.md / docs /
                                               # scripts / .github / rust-toolchain* 任一即跑全套（仅 assets/ 放行）
```

- 冒烟（临时配置目录 / models_dir / --version 旗标）= docs/prompts/live-check.md。
- CI = `.github/workflows/` 三 workflow（结构细节真源 = 各文件头注）：**ci**（单 job，门禁 = precommit.ps1 整脚本入 CI〔D-88 A1〕+ 测试 + 分发演练；触发 = push 全分支 + PR + 手动〔D-89 回摆〕）/ **release**（tag `v*` 或手动演练 → 调 `scripts/release.ps1` 全链出草稿 + sha256，转正人工〔D-90〕）/ **security**（cargo-deny 四表，ubuntu）；工具链真源 = 仓库根 `rust-toolchain.toml`；无分支保护——PR 上会跑 CI，但红叉不拦合并，仍靠提交前自查；CI 绿 ≠ 干净机能跑（runner 自带 VC++）。
- 改码前摸底优先 MCP `codegraph_explore`（`.codegraph/` 本机索引，不入库）。

## 3. 十 crate 拓扑与分层硬规则

| crate | 职责一句话 | 允许内部依赖 |
|---|---|---|
| lt-proto | 事件/命令/数据契约 + 翻译域常量（E3/ADR-10） | — |
| lt-i18n | zh/en 双语键集（两份 yaml 键集必须一致） | — |
| lt-models | Settings/ModelConfig/模型注册表/缓存探测（零网络） | proto |
| lt-download | 下载器：reqwest/sha2/退避/完整性 | proto |
| lt-audio | wasapi 采集 + silero VAD + ORT 内嵌 | models |
| lt-asr | ASR worker 子进程 + IPC + 三引擎（SenseVoice/Whisper/Qwen3；配置经 stdin 首行，D-76） | proto（models 仅 dev-dep） |
| lt-translate | async-openai LLM | proto |
| lt-orchestrator | 编排域：识别/翻译管道 + 线程监督器 + 下载管理 + 日志桥；禁依赖 ui/winit/i18n，用户文案经 `Msg` 注入 | proto, models, download, audio, asr, translate |
| lt-ui | egui 多窗口：悬浮/字幕/控制面板/日志窗/托盘（纯投影 crate，禁依赖 lt-app） | proto, i18n, models |
| lt-app | 组合根：boot + 动脉桥 + 命令路由 + 单实例消息窗 + worker 入口（同 exe `--asr-worker` 自拉起）+ Job Object 孤儿兜底 | 十库全部 |

- 依赖白名单真源 = `scripts/check_deps.ps1`（含 dev-dep 特批表）；拓扑终局 = `docs/archive/architecture-v2.md` §3.1 + `docs/archive/architecture-v2-improvements.md`。
- **lt-proto 冻结规则**：`PROTO_VERSION` 随结构变更递增（当前 7）。豁免评审 = 纯新增 Cmd/UiEvent/AppCommand 变体、纯新增 Settings 字段（须 serde default 兼容旧档）、既有枚举增项；仍须评审 = 删除/改名/改型/改语义任何既有契约项（记录入 D-xx）。**跨 crate 字符串编码协议（前缀/分隔符/哨兵值）一经发现按 P1 立案**（check_guards 禁令 5 机械拦截）。
- 新增 UI 能力**不得扩 lt-proto 契约**：日志经 `LogLine{target}` 回流。
- Settings 运行时落盘唯一通道 = `Cmd::PersistSettings`（shell 直排：保存 + 发布总线，300ms debounce 对齐原版）；各命令草稿写入唯一落点 = shell `apply_settings_side_effects` 纯函数（E1-4），UI 只发命令不预写。
- 值域判定一律走类型化透镜（E2/D-79：`Settings::engine_key()/hub()/proxy_mode()` + EngineKey/Hub/ProxyMode；持久层 String 不改型，透镜归一）；域内禁 `== "funasr"` 类字面量比较（lt-asr/lt-models/lt-app worker 分派为单点分派边界豁免）。
- UI → 宿主的窗口动作统一走 **`WinAction` 意图通道**（定义 `crates/lt-ui/src/state.rs`；跨平台抽象缝）：新增窗口行为优先加变体让宿主执行，不在 UI 侧散点直调 Win32。

## 4. 路由表（动手前必读；本表 = 提示词卡唯一索引）

> 卡（docs/prompts/）= 各阶段强制自查清单：出口条件不满足不得进入下一阶段。

| 场景 | 必读 |
|---|---|
| 任何工作包动第一行代码前 | `docs/prompts/kickoff.md` |
| 施工中写码自查（分层/测试/提交） | `docs/prompts/implement.md` |
| 改 GUI / 窗口 / 托盘 / Win32 / 字体渲染 | `docs/gotchas.md`（触发词索引）+ `assets/reference/` 参照截图（zh/en 各 10 张） |
| 动契约（lt-proto / Settings 结构） | 本文 §3 冻结规则 + `docs/decisions.md` |
| 翻译供应商 / LLM 接口改动 | 本文 §6 翻译域 + `docs/archive/llm-api-round2.md`（四厂商对照表） |
| 模型加载 / 校验 / 缓存探测改动 | 本文 §6 模型信任 + `docs/archive/model-trust-repair.md` |
| 立档 / 归档 / 收口 | `docs/README.md` 顶部纪律条文（唯一真源）+ `docs/prompts/closeout.md` |
| 评审 / 代码自查 | `docs/prompts/review.md`（八原则） |
| 需要用户裁决 | `docs/prompts/decision-request.md` |
| 实机走查 / GUI 冒烟取证 | `docs/prompts/live-check.md` |
| 会话收尾交接 | `docs/prompts/handoff.md` |
| 写 / 改守护脚本 | 本文 G-20（grep 方言）+ G-23（.ps1 带 BOM）+ 各守护脚本头注（风格对齐） |
| 分发 / 打包 / 发布 | `docs/distribution.md` + `scripts/release.ps1`（发布链）/ `scripts/package_release.ps1`（打包）头注 |
| 历史工作包依据（下载器 / ASR 加固 / 视觉…） | `docs/archive/` 一行一档索引见 docs/README.md 归档表（只读） |

## 5. 纪律速记（全文真源 = docs/README.md 顶部九条，此处只留最绑定的）

- 主干直推：提交直接落 main，无 PR 流程（CI 全分支 push 兜底）。
- 汇报文风：结论先行、判对项与待拍板项分列、最直白零废话（评审全文风 = `docs/prompts/review.md`）。
- 定稿 `docs(scope)` 提交必须先于第一行实现代码；草稿只进 `docs/drafts/`（不入库）；决策号（D/ADR）随定稿同一提交登记 decisions.md。
- 收口一气呵成 + 收口三问，全文 = docs/README.md 纪律⑤ + `docs/prompts/closeout.md`。
- 收尾不用 `git add -A` / `git add .`，逐项显式 pathspec；生成副产物不入库。
- i18n：zh/en 两份 yaml 必须同步修改（键集一致）。

## 6. 域速记

- **翻译域（D-82/D-85 起，用户裁决）**：对外只给翻译接口、内部消化供应商差异；**UI 与后端必须一比一同步**；高级参数默认一律不发送（用户显式指定才发）；失败兜底终点 = 最小请求（保留系统提示词）；思考关不掉则 UI 取消勾选 + 提示 + 按模型落盘；报错必须与译文一眼可辨；费用按本次运行累计、重启清零。细节 = `docs/archive/llm-api-round2.md`（四厂商关闭思考/锁定参数对照表）。
- **模型信任（D-83，用户四条裁决）**：凡加载必验 sha256；坏文件隔离 + 自动重下修复，自动重试 3 次后提醒用户；不做版本钉死；只验正在加载的模型。细节 = `docs/archive/model-trust-repair.md`。
- **ASR 域既往裁决**：SenseVoice 输出无语言标签 → 三优先 resolve（显式设置 > 模型标签 > 启发式，D-30）；qwen3 生效段长钳制 ≤15s 且 overlay 归一不写穿用户设置（D-28/D-69）；interim 裁剪带代际校验（D-27）；client 非 Worker 错误自动 recover 重启（D-26）；下载源诚实——无 MS 源不伪造 ms 字段（D-24）。
- **窗口透明**：悬浮窗 = LAYERED + LWA_ALPHA + SetWindowRgn 镂空（LWA_COLORKEY 因 ±1 抖动不可用）；字幕窗单窗逐像素透明不可达（wgpu HWND 仅 Opaque），整窗 alpha 可用（D-36）。
- **字体（D-17）**：内嵌思源（中英韩）+ MonoCJK + NotoSansSymbols2 三字体自洽，系统字体缺失不报错回退内嵌（仅锦上添花，禁依赖）；行级字体键空串 = 级联跟随 `subtitle_font_family`，显式族名 = 独立指定；改字体键 → 立即 `fonts::apply_fonts` + 防抖落盘；渲染侧禁 `FontFamily::Name` 臆造，一律经 `font_family_for`（G-12）。细节 = `docs/archive/font-system.md`（史）。
- **参照截图**：真值 = `assets/reference/`（2026-09-09 拍自工作区副本；旧 2026-09-06 套误拍外部爆改版已作废）。重拍脚本 `scripts/grab_reference_ui.py`，解释器 = 外部带 PyQt6 的 venv（本机路径不入库，见项目记忆）；Qt 6.11 `grab()` 不渲染 QTextEdit 样式表背景，脚本已补 viewport 同色（G-18）。
- **资产**：`assets/` = i18n yaml、`fonts/`（brotli 压缩 ~12.7MB + OFL 许可）、图标、reference、`silero_vad.onnx`、SOURCES.md（来源/sha256）。

## 7. 大坑速查（一行精选；全本 = docs/gotchas.md，禁死引用——本节可精选不全列，但引用的每个 G-编号必须实存）

1. G-1 egui 多窗 repaint 回环：`RedrawRequested` 不得再转发 request_redraw，否则 1350fps 自旋、其他窗口饿死白屏。
2. G-2 egui 0.36 API：`set_inner_size`→`request_inner_size`；`ctx.fonts()` 是闭包 API；Color32 仅 premultiplied 常量是 const。
3. G-3 lt-ui 内 `crate::windows` 模块遮蔽 windows crate——引用必须写 `::windows::`。
4. G-4 外部 `MoveWindow` 移动 winit 透明窗 → DXGI 表面失配白屏；`Moved` 事件强制 `painter.on_window_resized`（幂等）。
5. G-5 构建环境双保险勿动：双栈 CRT 由 `.cargo/config.toml`（MultiThreaded + CMP0091）钉死；libclang/cmake 由 uv 装仓库内 .venv 相对指向。
6. G-6 `block_on` 外求值 tokio timeout 必 panic；async-openai 0.41 需显式 `_api` feature。
7. G-7 tray-icon 0.24 无气泡 API（做气泡需 Shell_NotifyIcon 直调或换库）。
8. G-8 字幕窗 30% 黑底叠白窗呈 179 灰 = 正常 alpha 合成，非渲染 bug。
9. G-9 磁盘满两症：C 盘满 → 测试 TEMP 报 StorageFull、GDI+ Save 假死；D 盘（构建盘）满 → os error 112（target/ 可达数十 GB）。
10. G-10 egui 窗 PrintWindow 抓旧帧；实机验收必须全屏截图。
11. G-11 edition 2021 if-let scrutinee 临时值自死锁；共享锁 + 分支派发一律先绑定再分支。
12. G-12 `FontFamily::Name(族名)` 未注册即 panic；渲染一律经 `fonts::font_family_for` 取。
13. G-13 PROPVARIANT Drop 会 PropVariantClear→CoTaskMemFree；VT_LPWSTR 必须 CoTaskMemAlloc 分配、所有权交析构。
14. G-14 事件循环线程禁同步 MessageBox/模态（含 rfd MessageDialog）；用 egui 内嵌模态或原生 Toast。
15. G-15 winit apply_diff 整体重写 EXSTYLE，手工位（LAYERED/TRANSPARENT）被清；层属性每帧实测缺位即重挂。
16. G-16 tray TrackPopupMenu = 嵌套模态循环：托盘专用线程（crates/lt-ui/src/tray.rs）+ GetMessage 泵 + PostThreadMessageW 唤醒；禁 WM_QUIT 退出。
17. G-17 winit drag_window 不可靠（哑 WM_MOUSEMOVE + 仅左键）；字幕窗/悬浮窗手工拖动 SetCapture+GetCursorPos；拖动中隐藏必须 ReleaseCapture。
18. G-18 Qt 6.11 `grab()` 不渲染 QTextEdit 自身样式表背景（重拍参照图脚本坑，脚本已补）。
19. G-19 多代理并行期 stash 手术曾混入他人暂存（af0ff58）：提交前逐项 pathspec 复核。
20. G-20 会话内 grep 是 ugrep 包装函数，复杂 ERE 与 GNU grep 结果不同；入库脚本一律 `command grep` / `sh -c`，勿依赖 grep 方言。
21. G-21 sherpa binding `Default` 不可裸用：qwen3 默认 max_new_tokens=128（官方 512）、nano Default 是 max_new_tokens=0/temperature=1.0/top_p=1.0——一律用 OFFICIAL_* 常量（engines/qwen3.rs、nano.rs 头注）。
22. G-22 测试临时目录必须唯一化：共享同名目录在 Windows 报 code 183 竞态飘测（8f964c5 根治）。
23. G-23 无 BOM 的 .ps1 被 Windows PowerShell 按 ANSI 解析（中文全乱码、解析失败）——入库 .ps1 一律带 UTF-8 BOM（本仓已 pwsh-only〔ADR-16〕，pwsh 默认 UTF-8 无此坑，BOM 保留双读兼容）。
24. G-24 `scroll_to_cursor` 在 ScrollArea 外调用不生效：全局滚动目标被下一帧收尾的滚动区按外层坐标消费、偏移增量≈0——跳底须在内容闭包内执行（「回到最新」浮钮经 request_jump 记一帧）。
25. G-25 rust-cache 清 target/ 下非 cargo 结构文件（只留目录骨架）——`-sys` crate 解到 target/ 的预编译库缓存恢复后成空目录、build.rs 的 `is_dir()` 守卫照样放行 → 链接期找不到 `.lib`；对策 = 预解包到 `.cache` 并设其 `*_LIB_DIR` 环境变量（sherpa 已改，见 G-25）。
26. G-27 pwsh 裸调用 GUI 子系统 exe 不等待、不设退出码——CI `--version` 冒烟恒绿只证明文件存在；一律 `Start-Process -Wait -PassThru` 取真退出码（ci.yml / release.ps1 已按此修复）。

## 8. 看板（机制：drafts 看板 = 本区，一行一包、清零即删；细节只活在文档里）

### 待拍板（等用户裁决；草稿在 docs/drafts/ 不入库）

- 待拍板：**ASR 模型选型**——近一年开源模型调研（FireRed2-CTC / Cohere-14lang / Dolphin / 标点闸门候选，A~H 清单）（docs/drafts/asr-model-survey-2026.md）
- 待拍板：**增量识别的目标形态**——"做成什么样"未定案（设计无关批已拆出开工，见施工中；余包 B/D/G 与 J7 待定案）（docs/drafts/incremental-asr-overhaul.md）
- 待拍板：**跨平台分期 ①~⑤**——2026-09-11 可行性评估（无草稿，结论在会话记忆；P0 = 宿主 trait 化）
- 待拍板：**llm 遗留⑪**——规则 4/5 偏离可见（docs/archive/llm-api-round2.md）
- 待拍板：**UX 三期可选项**——前缀码枚举化 / 设置保存失败 UI 流 / 错误译文样式（docs/archive/ux-feedback.md）
- 待拍板：**PH-6**——参考图 GPU 型号中性化重拍（可选）（docs/archive/path-hygiene.md）

### 施工中

- （无）

### 遗留（完工包的实机走查与未了项，一行一包，清零即删）

- asr-chain-robustness（D-94）：实机走查 8 项——退出尾巴入转录 / 退出延迟 / 流式终态不被半截话盖回 / 字幕窗缺句 / 未就绪记账 / 整段模式回归 / 在跑任务逗留实测 / 重启一致性；评审遗留：清空是否该管字幕窗句子待裁 + 三项测试覆盖缺口（docs/archive/asr-chain-robustness.md §5/§10）
- doc-network-hardening（D-93）：断言 5 豁免表随误报维护；可选杠杆未取——§7 精选 25→10~12 条（docs/archive/doc-network-hardening.md §五）
- agents-md-overhaul（ADR-15/16）：gotchas 候选回读（visual-parity/overlay-realign 怪癖收编为新条目）/ prompts 技能化验证（ZCode 工作区级 .zcode/skills）/ 两档预算两周后按实测回调 / i18n 键集 parity 守护候选（docs/archive/agents-md-overhaul.md §五）
- quit-flow-redesign（D-87）：§五走查矩阵剩余行为项随用随验（docs/archive/quit-flow-redesign.md §九）
- ci-workflow-suite（D-88/D-89）：deny.toml 首跑校准 + CI 首跑墙钟观察 + nextest/typos 候选 + 仓库设置候选（immutable releases / tag protection；attest 已由 D-90 否决）+ Dependabot alerts/security updates 未开（docs/archive/ci-workflow-suite.md §六）
- dev-config-audit（收口 2026-09-17）：A3 冒烟配置免手抄——settings.smoke.json 模板或 --smoke 参数，随下次冒烟改造裁决（docs/archive/dev-config-audit.md §4）
- release-engine（D-90）：首次真 tag 演练未跑（runner 的 gh 用法 / publish job 写入 / 摘要实貌待验）+ 首发须抬版本（旧 v0.1.0 tag 不删，见 docs/distribution.md §4）+ zip 五件套断言与 build-info 是否随 Release 永久留档待表态
- 架构 v2/2.1：实机走查 11 项 + WP-9 性能预算（后续单独方案）（docs/archive/architecture-v2.md §6.4）；W5 走查 6 项——悬浮窗拖动/字幕窗拖动穿透回归/导出保存框/背景图选择框/设备下拉/Monitor 条（同文档 W5 节）
- translator（D-85）：实机走查 13 项（docs/archive/translator-probe-hotswap.md §6.2）
- model-trust（D-83）：实机走查——改坏一个模型文件应自动隔离+重下+装载（docs/archive/model-trust-repair.md）
- context-turns（D-84）：实机走查 4 项（docs/archive/context-turns-ui.md）
- asr-hardening：GUI 冒烟 A/B/C + T1 qwen3 长样例校准（whisper 六档 sha256 已全量登记）（docs/archive/asr-hardening.md）
- download-overhaul：S1/S6 全程真实网络走查（docs/archive/download-overhaul.md）
- 复刻期：WP-9 M6 调优（启动<2s / 空闲 CPU<1% / 8h 长跑 / 内存回收 / 端到端）；WP-5 托盘气泡、WP-8 热键届时按产品价值裁决（docs/archive/parity-closure.md）
- distribution：WD-6 首启横幅待点头；WD-7 tag→Release、WD-8 检查更新随公开发布推进（docs/distribution.md）；1.0.0 前置：应用内更新日志真实内容待 Phase B 置换（changelog-scheme）、干净机端到端未跑（就绪评估 2026-09-11）；更新日志机制已建（D-91，2026-09-16）——应用内渲染改造（真粗体/切 tab 滚动/多版本折叠）待立包；8 个死 i18n 键（theme_*/btn_check_update/btn_open_repo·issues/hotkey_*/changelog_title）随 WD-8 一并定生死（docs/archive/live-check-fixes.md §五）
