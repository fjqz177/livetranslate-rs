# 决策总登记

> 一行一条。号随定稿发（草稿期不发号），禁预留；先登记后引用；下一个号 = 表尾 + 1（不另设指针行）。
> 被推翻的决策不删行、不复用号，原行末尾加「→ 被 D-xx 取代」。
> 维护时机：定稿 = 加行；施工中新裁决 = 当场加行；收口 = 落档路径更新为归档路径。

## D-xx（产品/行为决策）

> 注：D-38~D-59 为 2026-09 预留后弃用的空段（22 号，data-lifecycle 候选段预留但从未实发），保留空洞不回收。

| 号 | 日期 | 一句话 | 落档 |
|---|---|---|---|
| D-1 | 2026-09-05 | 纯 CPU 硬约束：无 CUDA/GPU 路径，asr_device 下拉移除 | docs/archive/rewrite-research.md |
| D-2 | 2026-09-05 | compute_type 设置移除，全部模型固定加载量化档 | docs/archive/rewrite-research.md |
| D-3 | 2026-09-05 | Whisper 模型仓换 ggml-org GGML 档，本地布局改 .bin | docs/archive/rewrite-research.md |
| D-4 | 2026-09-05 | SenseVoice/Nano 仓换 sherpa-onnx 官方 ONNX 转换仓 | docs/archive/rewrite-research.md |
| D-5 | 2026-09-05 | Silero VAD 模型内嵌 exe，首启少下载一项 | docs/archive/rewrite-research.md |
| D-6 | 2026-09-05 | MonitorBar GPU 段恒显 N/A，不引 nvml | docs/archive/rewrite-research.md |
| D-7 | 2026-09-05 | neutralize_funasr_requirements 删除（无 Python 远程代码隐患） | docs/archive/rewrite-research.md |
| D-8 | 2026-09-05 | Nano 不再下载 Qwen3-0.6B → 被 D-13 取代（并入整体裁剪） | docs/archive/rewrite-research.md |
| D-9 | 2026-09-05 | 配置目录统一 ~/.config/livetranslate，新增 models_dir 键 | docs/archive/rewrite-research.md |
| D-10 | 2026-09-05 | Nano 标实验性/Anime 待自转换的旧安排 → 被 D-13 取代（并入整体裁剪） | docs/archive/rewrite-research.md |
| D-11 | 2026-09-05 | 单实例互斥量防双开 + Ctrl+C 优雅退出 | docs/archive/rewrite-research.md |
| D-12 | 2026-09-05 | 远程 ASR 整体裁剪：remote-whisper 引擎与相关设置全删 | docs/archive/rewrite-research.md |
| D-13 | 2026-09-05 | Anime-Whisper 整体裁剪（其连带 nano 裁剪部分被 D-14 恢复） | docs/archive/rewrite-research.md |
| D-14 | 2026-09-05 | FunASR Nano/MLT 恢复实验性（恢复 D-13 裁掉的 nano 部分）→ 被 D-86 取代 | docs/archive/rewrite-research.md |
| D-15 | 2026-09-05 | Whisper 档位加 turbo，共 6 档（超出原版增补） | docs/archive/rewrite-research.md |
| D-16 | 2026-09-05 | whisper 引擎用 whisper.cpp（GGML），弃 sherpa 统一栈 | docs/archive/rewrite-research.md |
| D-17 | 2026-09-07 | 字体默认统一内嵌思源黑体 + 行级级联，系统字体仅锦上添花 | docs/archive/rewrite-research.md |
| D-18 | 2026-09-07 | 暂不公开发布，先本地 zip 分发验证 | docs/distribution.md |
| D-19 | 2026-09-07 | 首启直进主界面，向导代码保留不接线（偏差转正） | docs/distribution.md |
| D-20 | 2026-09-07 | 应用内「检查更新」按钮，随公开发布启用 → 被 D-110 裁不做（2026-09-28） | docs/distribution.md |
| D-21 | 2026-09-07 | 全模型双源：whisper 打破 always-HF + hub 缺失回落 | docs/distribution.md |
| D-22 | 2026-09-08 | 缓存完整性探测由体积阈值改 manifest 逐文件校验 | docs/archive/download-overhaul.md |
| D-23 | 2026-09-08 | 下载可取消，取消保留续传现场 | docs/archive/download-overhaul.md |
| D-24 | 2026-09-08 | 诚实下载源：无 MS 源不伪造 ms 字段，端点随所选 hub | docs/archive/asr-engine-expansion.md |
| D-25 | 2026-09-08 | 新增 Qwen3-ASR-0.6B 引擎（第三值，纯 auto-LID） | docs/archive/asr-engine-expansion.md |
| D-26 | 2026-09-08 | ASR client 非 Worker 类错误一律 recover() 自动重启 | docs/archive/asr-hardening.md |
| D-27 | 2026-09-08 | interim 裁剪加代际校验，VAD 收段后放弃本次 trim | docs/archive/asr-hardening.md |
| D-28 | 2026-09-08 | qwen3 生效段长钳制 ≤15s，用户设置值不动 | docs/archive/asr-hardening.md |
| D-29 | 2026-09-08 | MIC 条显隐改由 UI 启用意图驱动（原版按 mic_rms 有值） | docs/archive/mic-monitor-fix.md |
| D-30 | 2026-09-08 | SenseVoice 语言三优先：显式设置 > 模型标签 > 启发式 | docs/archive/sensevoice-language-fix.md |
| D-31 | 2026-09-08 | 日志视图过滤/贴底跟随与原版分道（显示期过滤可回溯） | docs/archive/log-tab-redesign.md |
| D-32 | 2026-09-08 | 悬浮窗行1按钮三态几何稳定，按压反馈仅色变 | docs/archive/button-press-feedback.md |
| D-33 | 2026-09-08 | 隐藏提示改原生通知、退出确认改 egui 内嵌模态，禁同步模态 | docs/archive/hide-quit-flow-overhaul.md |
| D-34 | 2026-09-08 | 悬浮窗/字幕窗层属性每帧实测缺位即重挂（自愈） | docs/archive/hide-transparency-fix.md |
| D-35 | 2026-09-08 | 托盘图标/菜单整体移专用线程，菜单不再挂死主循环 | docs/archive/tray-menu-blocking-fix.md |
| D-36 | 2026-09-08 | 字幕窗穿透升级为分区穿透 + 顶条工具条/Z 序/工作区钳制 | docs/archive/subtitle-window-overhaul.md |
| D-37 | 2026-09-08 | 字幕窗拖动弃 drag_window，改 SetCapture 手工跟踪 | docs/archive/subtitle-window-overhaul.md |
| D-60 | 2026-09-09 | VAD 模式热切换真实生效（R2 修复） | docs/archive/architecture-v2.md |
| D-61 | 2026-09-09 | models_dir 失败可唤醒（R3，AH-1 哲学推广） | docs/archive/architecture-v2.md |
| D-62 | 2026-09-09 | 线程死亡可见 + 自动重启（R1） | docs/archive/architecture-v2.md |
| D-63 | 2026-09-09 | 字幕窗开关持久化对称，重启不再复活（R7） | docs/archive/architecture-v2.md |
| D-64 | 2026-09-09 | 翻译池有界 keep-latest 64 + 水位告警（R15） | docs/archive/architecture-v2.md |
| D-65 | 2026-09-09 | 启动早期失败弹原生窗（R11①） | docs/archive/architecture-v2.md |
| D-66 | 2026-09-09 | settings 坏档隔离 .corrupt-<ts>，按默认值生效（R17） | docs/archive/architecture-v2.md |
| D-67 | 2026-09-09 | Monitor 条改 ~30ms 节拍重绘（事件驱动改节拍驱动） | docs/archive/architecture-v2.md |
| D-68 | 2026-09-09 | 下载卡片/托盘/悬浮窗菜单命令类型化（用户无感） | docs/archive/architecture-v2.md |
| D-69 | 2026-09-09 | qwen3 钳制改 overlay 归一，不再写穿用户设置（R18） | docs/archive/architecture-v2.md |
| D-70 | 2026-09-09 | 控制面五跳收敛两跳，lt-backend 线程退役 | docs/archive/architecture-v2.md |
| D-71 | 2026-09-09 | 悬浮窗拖动手工化（R22，D-37 同法） | docs/archive/architecture-v2.md |
| D-72 | 2026-09-09 | 设备切换清段队列 + VAD 复位（R31） | docs/archive/architecture-v2.md |
| D-73 | 2026-09-09 | 二次启动激活已有窗口（WD-5 消项） | docs/archive/architecture-v2.md |
| D-74 | 2026-09-09 | 同语言免翻译比较归一化（zh vs zh-CN，仅目标侧） | docs/archive/architecture-v2.md |
| D-75 | 2026-09-09 | settings.json 崩溃中间态由 .bak 自动恢复（R17 补全） | docs/archive/architecture-v2.md |
| D-76 | 2026-09-09 | worker 配置改 stdin 首行传递，命令行不再暴露模型路径（R28） | docs/archive/architecture-v2.md |
| D-77 | 2026-09-09 | 下载会话线程 panic → 下载卡落 DownloadFailed 失败态 | docs/archive/architecture-v2.md |
| D-78 | 2026-09-09 | 向导子系统保留并定位为待开发面（维持并强化 D-19） | docs/archive/architecture-v2-improvements.md |
| D-79 | 2026-09-09 | 值域类型化：String 持久层 + proto 枚举透镜，字段类型不改 | docs/archive/architecture-v2-improvements.md |
| D-80 | 2026-09-09 | 常驻线程 panic 重启改指数退避，8 连败放弃并告警 | docs/archive/architecture-v2-improvements.md |
| D-81 | 2026-09-09 | 死契约清理：删 WorkerExited/SetTimeout，CancelBench 补接线 | docs/archive/architecture-v2-improvements.md |
| D-82 | 2026-09-10 | LLM 接口层改造批起点：关思考默认勾选、UI 后端一比一同步、记忆仅内存 | docs/archive/llm-api-round2.md |
| D-83 | 2026-09-10 | 模型加载前零信任 sha256 校验 + 坏文件隔离 + 自动修复循环 | docs/archive/model-trust-repair.md |
| D-84 | 2026-09-10 | 翻译页新增当前活跃模型「上下文数」直达行（新增入口） | docs/archive/context-turns-ui.md |
| D-85 | 2026-09-10 | 供应商连接测试 10s 封顶、热切换、会话账本、厂商预设（默认 DeepSeek） | docs/archive/translator-probe-hotswap.md |
| D-86 | 2026-09-12 | Fun-ASR-MLT-Nano 幽灵值全量移除，灰显机制废止（取代 D-14） | docs/archive/funasr-mlt-removal.md |
| D-87 | 2026-09-14 | 退出与确认系统重做：专用 WinId::Confirm 确认窗包办六确认（暗色半透明圆角自绘、替换语义、身份键载荷、属主遮罩），借画布杂技链删除 | docs/archive/quit-flow-redesign.md |
| D-88 | 2026-09-15 | CI 工作流全家桶：单 job 整脚本入 CI（A1）+ 工具链钉版 + --locked + cargo-deny 供应链 + tag→Draft 发布链（E2）+ dependabot 收窄 actions 月更（局部号见 docs/archive/ci-workflow-suite.md §二 G 项）；其中 J1（删 pull_request 触发）→ 被 D-89 取代；其 release 链（E2 四道闸版）→ 被 D-90 取代 | docs/archive/ci-workflow-suite.md |
| D-89 | 2026-09-15 | CI 触发面回摆：仓库 09-09 起即 public，J1「私有 + 单人主干直推」前提失效——ci 恢复 pull_request 触发 + 并发组按源仓/源分支去重（治双跑）；security 补自触发路径 + advisories 失败注解化 | docs/archive/ci-workflow-suite.md |
| D-90 | 2026-09-16 | 发布链重做并转正：`scripts/release.ps1` 八动词引擎（check/build/pack/draft/verify/rehearse/release/promote）+ `release.yml` 薄编排，本地与 CI 同一份（取代 D-88 四道闸版）；不变量 = CI 永不可发布（promote 动词不进 workflow，转正人工且必须 -NotesFile）；只发正式版（预发布在 check ① 拒）；不做 attestation 与代码签名；演练通道 = workflow_dispatch（脚本自判）+ 本地 rehearse；其中 promote 必须 -NotesFile 由 D-91 放宽（八动词 → 九动词） | docs/distribution.md |
| D-91 | 2026-09-16 | 更新日志机制：仓根 `CHANGELOG.md` / `CHANGELOG.en.md` 为唯一正典（标准 Markdown + 允许分隔线 `---`；应用内编译期内嵌、Release 正文由引擎自动抽取）；发版三件套闸（版本号 = tag = 同号非空段落，`release.ps1 check ②` + CI 每次 push 早警告的只读动词 `notes`）；promote 的 -NotesFile 降为可选覆盖；应用内渲染改造整体缓办（真粗体/切 tab 滚动/多版本折叠同批，移交清单在档） | docs/archive/changelog-scheme.md |
| D-92 | 2026-09-17 | 面板「删除全部」不退出应用（原版 `_delete_all_and_exit` 退出）：行为差异登记 + 按钮/对话框文案与行为对齐（去"并退出"） | docs/archive/live-check-fixes.md |
| D-93 | 2026-09-17 | 文档网络加固：全仓清淤（README/distribution/归档注记/流程卡链）+ 守护断言 5~8（路径扩面/G 编号全仓/归档对账/水位线）+ 钩子触发面「守护读谁、谁触发」+ 宪法回吐（~0.7KB 复述迁回流程卡） | docs/archive/doc-network-hardening.md |
| D-94 | 2026-09-17 | 退出收尾冲刷用 force_flush 不设 min_speech 门槛（与 Python 非增量路径的带门槛 flush() 有意不同）：capture 侧不知增量是否激活、退出时刚说的话优先保真、下游 reject_segment 三层过滤仍兜底（识别→翻译→显示链路健壮性修复批随行登记） | docs/archive/asr-chain-robustness.md |
| D-95 | 2026-09-17 | 加固收口后评审修复：钩子触发面补根 README.md（「守护读谁谁触发」补漏）+ changelog-scheme/agents-md-overhaul 补权威牌 + AGENTS §2 测试基线去数字（根治滚动漂移，收口卡基线提醒同步撤销） | docs/archive/doc-network-hardening.md |
| D-96 | 2026-09-22 | 阶段定调：进入零代码整理阶段——只做文档治理与流程严谨化（实机走查转 issue 驱动，遗留清账批 12 项分堆处置：Q1/Q4/Q7/Q8/Q11 本批执行、Q2/Q3/Q6/Q9/Q10 缓办归队、Q5/Q12 已裁），改码须用户明示解禁 | AGENTS.md §1 |
| D-97 | 2026-09-22 | 流程纪律载体工具中立：AGENTS.md + docs/prompts/ 纯 Markdown 为唯一载体，不绑单家 agent 软件扩展机制——prompts 技能化（.zcode/skills）验证候选项裁销 → 被 D-102 取代（2026-09-28 流程层换血） | docs/archive/agents-md-overhaul.md §五 |
| D-98 | 2026-09-22 | 仓库治理两表态：tag protection + immutable releases 随下次发版一并开启（现不动手）；build-info 维持现状（本地 dist/ + step summary），不随 Release 永久留档 | docs/distribution.md §4 |
| D-99 | 2026-09-22 | 复刻期 WP-8（ErrorBanner 与全局热键）正式销项：出处未证实，依档案「确认出处前不动工」建议裁销；需求真实出现时再立包 | docs/archive/parity-closure.md |
| D-100 | 2026-09-25 | 窗口闪现整治三裁决：启动全窗隐藏创建 + 首帧后揭示（先画后显，杜绝窗口类刷子白底上屏）；运行期取消模型加载独立弹窗——ModelLoadStart 并入悬浮窗状态行 asr_label（对齐 less-is-more；ModelLoadDone 确认全仓无生产者留空臂）；退出先全窗离屏再 event_loop.exit（surface 持 Arc<Window> 致 drop 序重排无效，隐藏与销毁时序解耦） | docs/archive/window-flash-cleanup.md |
| D-101 | 2026-09-27 | v1.0.1 发布 + D-98 兑现：tag protection ruleset `release-tags-immutable` 建成（id 24072922，target=tag 匹配 `refs/tags/v*`，禁删除/禁改指向、不禁新建，零 bypass——含管理员）；immutable releases 无 REST API 字段，待用户网页设置（Settings → General → Releases）；build-info 维持现状照旧 | docs/distribution.md §4 + scripts/release.ps1 |
| D-102 | 2026-09-28 | 流程层换血：Matt Pocock Skills 为唯一流程载体（推翻 D-97）——docs/prompts/ 七卡删除，孤本知识迁 AGENTS §5（评审八原则 + 拍板机制）与根 README §二（冒烟配方）与 docs/agents/workflow.md（流程纪律）；docs/board.md 退役，状态真源移交 GitHub Issues（ready-for-agent ≠ 开工令）；check_agents_health 断言 4/5/9 改口并扩扫 docs/agents/；ADR-15/22 流程面注记取代。用户四项拍板：删看板 / 删卡 / 标签非开工令 / 授权断言改口 | docs/agents/workflow.md |
| D-103 | 2026-09-28 | LNK4098 链接警告静音拍板：`[lints.rust] linker_messages = "allow"`——实证无害 + 非新发（G-37：dumpbin 三实证 + v1.0.0 发布链/09-18 CI 同警告全绿；`warnings = "deny"` 管不到带 ignore_deny_warnings 的该 lint）；Cargo.toml 属代码面，施工随解禁令 | docs/gotchas.md G-37 |
| D-104 | 2026-09-28 | 滚动测试基线不设真源：真源 = `cargo test --workspace` 全绿本身，数字是滚动值（D-95 已定调数字不作门槛），立真源必漂移；ADR-22 未了项就此全销（断言 9 已随 D-102 重写消解，M11 句式归一转 issue #3 施工） | GitHub issue #3 |
| D-105 | 2026-09-28 | WD-6 首启缺模型轻引导横幅拍板做，P2 低优先——D-78 向导精神的轻量兑现；随代码解禁施工 | docs/distribution.md §3.3 |
| D-106 | 2026-09-28 | 分发渠道裁决：沿用原仓单渠道——v1.0.x 已在原仓发布运转，无迁移理由；WD-9 余下 README 双语完善与导流横幅仍可做（零代码） | docs/distribution.md §4 |
| D-107 | 2026-09-28 | WP-5 托盘气泡通知销项（G-7 对策落裁）：D-33 原生 Toast 已覆盖提示场景，气泡为重复面；与原版行为偏差 +1 随裁登记（D-99 同款销项模式） | docs/gotchas.md G-7 |
| D-108 | 2026-09-28 | path-hygiene §9 全历史脱敏裁不做（`git filter-repo --replace-text` / squash 重建公开仓均不做）：历史仅含个人路径字符串、非凭据，向前已根治（repo_hygiene.rs 为现行权威，ADR-21），仓已 public 且 tag / immutable release 不可变——重写历史代价大于收益 | docs/archive/path-hygiene.md §9 |
| D-109 | 2026-09-28 | llm-api ⑪ 规则 4/5 偏离可见拍板做：重建失败回执「仍在使用 <旧装置名>：<原因>」（规则 4）+ 自愈/降级偏离状态行「当前实际在用：X」（规则 5），界面与日志一致；改码面随解禁施工，排解禁队列 | docs/archive/llm-api-redesign.md §2.5 + issue #21 |
| D-110 | 2026-09-28 | WD-8 检查更新按钮 + WD-10 可选入口（打开配置/日志目录入口 + panic 崩溃尾部落盘）裁不做，推翻 D-20 的启用承诺：产品已发布运转而需求未被用户追讨，公开渠道 + 更新日志页已承接「知道新版本」的职责，维护面优先走查批；WD-10 panic 落盘若日后报障需要再立包 | docs/distribution.md §4 + issue #19 |
| D-111 | 2026-09-28 | SmartScreen 代码签名暂不买：证书年费对当前下载量是纯成本、警告劝退程度无反馈证据；再议触发 = 真实用户反馈「被 SmartScreen 拦住」，届时先评估 OV 级；维持未签名现状 | docs/distribution.md §4 尾注 + issue #18 |
| D-112 | 2026-09-28 | 代码解禁令维持不解，先清实机走查批（#2/#6~#13/#15）：走查结果可能改变解禁队列的优先级与内容；走查批清完后重议解禁（D-103/#23 修法/WD-6/D-109/S3-S4/D-96 缓办原样排队） | docs/agents/workflow.md §2 + issue #20 |
| D-113 | 2026-09-28 | 「清空」不管字幕窗，维持现状：字幕窗定位为 OBS 场景画面，清空只管悬浮窗列表——asr-chain-robustness §10 未修项 1（K2）就此销项，不改码 | docs/archive/asr-chain-robustness.md §10 + issue #6 |

## ADR-x（架构决策）

| 号 | 日期 | 一句话 | 落档 |
|---|---|---|---|
| ADR-1 | 2026-09-09 | 设置快照用 arc-swap（单写者不可变快照） | docs/archive/architecture-v2.md |
| ADR-2 | 2026-09-09 | 事件动脉载体复用 BoundedDropQueue（满丢最旧） | docs/archive/architecture-v2.md |
| ADR-3 | 2026-09-09 | 线程死亡检测 = 500ms join 轮询 | docs/archive/architecture-v2.md |
| ADR-4 | 2026-09-09 | lt-backend 线程退役而非保留路由（shell 直排） | docs/archive/architecture-v2.md |
| ADR-5 | 2026-09-09 | lt-pipeline 改名 lt-audio（拓扑诚实） | docs/archive/architecture-v2.md |
| ADR-6 | 2026-09-09 | 托盘线程监督 policy=Never（不重生） | docs/archive/architecture-v2.md |
| ADR-7 | 2026-09-09 | rfd 用同步 API 跑 worker 线程（不引异步） | docs/archive/architecture-v2.md |
| ADR-8 | 2026-09-09 | 监督器维持两实例（app 侧 + 管道侧），文档如实登记不合并 | docs/archive/architecture-v2-improvements.md |
| ADR-9 | 2026-09-09 | 值域类型化 = String 持久层 + proto 枚举透镜（D-79 评审记录） | docs/archive/architecture-v2-improvements.md |
| ADR-10 | 2026-09-09 | 翻译域常量上移 lt-proto，lt-ui 裁掉 lt-translate 边 | docs/archive/architecture-v2-improvements.md |
| ADR-11 | 2026-09-09 | 向导子系统保留为待开发面（D-78，死契约守卫白名单配套） | docs/archive/architecture-v2-improvements.md |
| ADR-12 | 2026-09-09 | 监督器 Backoff 按原设计补齐，参数默认（D-80） | docs/archive/architecture-v2-improvements.md |
| ADR-13 | 2026-09-09 | 死契约治理 = 派生覆盖率测试 + 零引用守卫脚本（A 硬 gate） | docs/archive/architecture-v2-improvements.md |
| ADR-14 | 2026-09-09 | process 纪律三条：验收漂移登记/方案偏离留痕/收口两问 | docs/archive/architecture-v2-improvements.md |
| ADR-15 | 2026-09-14 | 工程指令体系四层化：AGENTS 重写为宪法+路由表（两档预算入守护）+ 大坑迁 docs/gotchas.md + 流程七卡 docs/prompts/ + 健康守护第七项与钩子触发面扩展；docs 纪律①增常青参考类；流程面（七卡 / 看板）→ 被 D-102 取代 | docs/archive/agents-md-overhaul.md |
| ADR-16 | 2026-09-14 | 守护/脚本调用面统一切 PowerShell 7（pwsh-only）：5.1 feature-frozen 随 OS 生命周期，脚本 5.1∩7 公共子集零改动；钩子加 pwsh 探测兜底 | docs/archive/agents-md-overhaul.md |
| ADR-17 | 2026-09-17 | AGENTS 额度重校准（主体 ≤190 行/19KB、全文件 ≤230 行/24KB）+ 额度真源指针化（数字只活在 check_agents_health.ps1，宪法头注只留指针）+ 85% 水位线 WARN 不失败 → 额度与水位线被 ADR-18 取代（用户裁决取消机械额度） | docs/archive/doc-network-hardening.md |
| ADR-18 | 2026-09-17 | AGENTS 取消机械额度与水位线：体量回归自觉治理，防回胀靠 §4 路由纪律 + 守护断言 2~7（引用/看板/归档）+ 评审卡文档漂移自查；断言编号 2~7 保持不变防引用断裂，额度常量留名退役 | docs/archive/doc-network-hardening.md |
| ADR-19 | 2026-09-19 | 脚本管理与文档体系：十一脚本头注「用法卡」标准化（参数/前置/退出码/副作用为唯一细节真源）+ docs 顶层脚本目录页（导览不抄默认值，节标题=精确路径）+ 守护断言 8 脚本登记面（双向对账，无豁免）+ AGENTS §2 存量参数级细节收敛 | docs/archive/scripts-doc-system.md |
| ADR-20 | 2026-09-19 | 文本卫生：全仓文本文件强制无 BOM 合法 UTF-8 + index 全 LF——G-23 BOM 对策翻转（剥 9 个 .ps1 BOM）+ .gitattributes `* text=auto eol=lf` 全仓锁 LF + check_personal_paths 扩三查机械拦截（BOM/严格 UTF-8/index 行尾）+ .editorconfig 去 BOM 钉 LF | docs/archive/text-hygiene.md |
| ADR-21 | 2026-09-23 | 守护体系瘦身：四个 PS 检测守护退役（check_guards/check_deps/check_dead_contract/check_personal_paths），规则择优迁正统载体——方法禁令迁根级 clippy.toml disallowed-methods（豁免就地 #[allow]+理由，含净增强 thread::scope）、依赖白名单迁 lt-app/tests/topology.rs（收口 target/build 节盲区）、文本卫生+个人路径迁 lt-app/tests/repo_hygiene.rs（修静默跳过与转义盲区，LF 查随 .gitattributes 单源化废弃）、契约纯度迁 lt-proto/tests/contract_purity.rs；死契约与字符串协议回归评审纪律；check_agents_health 留任（文档网对账 = shell 舒适区）；precommit 七项并三项（fmt → 总纲健康 → clippy）。评审底稿已归档：docs/archive/scripts-usability-review.md（S3/S4 未了项见其 §二 + 归档核销注记；载体自身注释即档案 = clippy.toml 头注 + 三测试文件头注 + 各豁免点 #[allow] 理由） | docs/archive/scripts-usability-review.md |
| ADR-22 | 2026-09-24 | 指令体系元方法化：AGENTS 定位为「硬约束 + 索引 + 方法」三类内容层（头注写入**准入三问**与四层渐进披露链 L0~L3）；§7 大坑速查 29 条镜像 → 5 条高危防呆（坑册触发词索引承接，§4 增 5 行路由：怪现象/看板/首次换机/资产/架构细节）；§8 看板条目外迁 `docs/board.md`（AGENTS 只留机制 + 三小节指针，守护断言 4 不变）；§2 前置与解释下沉根 README 与脚本头注、§6 压成「D 号 + 一句约束 + 真源指针」；docs/README.md 归档表外迁 `docs/archive-index.md`（该页只留指针，守护断言 7 改读新档）；护法 = 不设机械额度（承 ADR-18），防回胀靠准入三问 + 路由覆盖 + 评审自查；机械面收口 = 断言 9 看板双真源（board.md 三小节 + §8 指针）+ 断言 7 行首锚定加固（G-33 入册）+ 评审卡加「总纲回胀」自查；机械面断言 4/9 看板部分与 §8 看板机制 → 被 D-102 取代 | docs/archive/agents-meta-restructure.md（操作面真源 = AGENTS.md 头注 + docs/board.md〔已退役〕+ docs/archive-index.md） |
| ADR-23 | 2026-09-24 | 内嵌资产退出 git、改由 uv 统一管理：onnxruntime.dll 与 silero_vad.onnx 从 git 追踪退役（未来 ORT 升版不再膨胀 .git；历史 blob 仍在），声明进 pyproject dev 组（`onnxruntime==1.29.0` / `silero-vad==5.1.2`，与 libclang/cmake 同模式），`uv sync` 装进 .venv 后 lt-audio 的 include_bytes! 直引 wheel 内文件（Rust 侧仅改路径，内嵌字节不变）；新增 lt-audio build.rs 哨兵（缺资产报「先跑 uv sync」+ rerun-if-changed 堵 crate 无变化时 cargo 走缓存的静默盲区）；silero-vad 拖 torch 全家仅占 .venv 磁盘、构建链不读（用户拍板：管理单点优先于磁盘）；资产版本身份四方哈希闭环钉 v5 = PyPI 5.1.2 = GitHub f0d880d（上游 master 已漂 v6.2，一切引用钉 v5 禁追浮动）；fresh clone 构建硬前置 = uv sync | 真源 = pyproject.toml + crates/lt-audio/build.rs 头注 + assets/SOURCES.md |
