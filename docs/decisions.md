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
| D-20 | 2026-09-07 | 应用内「检查更新」按钮，随公开发布启用 | docs/distribution.md |
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
| D-87 | 2026-09-14 | 退出与确认系统重做：专用 WinId::Confirm 确认窗包办六确认（暗色半透明圆角自绘、替换语义、身份键载荷、属主遮罩），借画布杂技链删除 | docs/quit-flow-redesign.md |

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
| ADR-15 | 2026-09-14 | 工程指令体系四层化：AGENTS 重写为宪法+路由表（两档预算入守护）+ 大坑迁 docs/gotchas.md + 流程七卡 docs/prompts/ + 健康守护第七项与钩子触发面扩展；docs 纪律①增常青参考类 | docs/archive/agents-md-overhaul.md |
| ADR-16 | 2026-09-14 | 守护/脚本调用面统一切 PowerShell 7（pwsh-only）：5.1 feature-frozen 随 OS 生命周期，脚本 5.1∩7 公共子集零改动；钩子加 pwsh 探测兜底 | docs/archive/agents-md-overhaul.md |
