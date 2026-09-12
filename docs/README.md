# docs 目录索引

> 2026-09-07 归档重组：阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档全部归档至 `docs/archive/`（只读决策史）；
> 阶段二（Rust 自有产品化）施工依据为本目录活跃文档 + `AGENTS.md` 待办。阶段一后 Python 原版仅作行为参考，
> 新增能力以产品体验为准，行为差异落档为 D-22 起新编号（决策史目录：`docs/archive/rewrite-research.md` §1.5）。
> 2026-09-09 二次归档：阶段二已完工的 11 份工作包文档移入 `docs/archive/`（全仓引用路径同步更新），
> 活跃文档仅剩 `distribution.md`（WD-1~WD-5 待施工）与 `data-lifecycle.md`（12 项整改候选待裁决）。
> 2026-09-09 三进：路径卫生全面排查立案 `path-hygiene.md`（P0×8/P1×2/P2×15 证据化分级，PH-1~PH-5 待施工）。
> 2026-09-09 四进：全系统架构深度评审 `architecture-review.md`（P0×1/P1×15/P2×16，R1~R32 风险登记册）+ 架构 2.0 方案
> `architecture-v2.md`（五病灶诊断、十 crate 目标拓扑、W0~W7 八波绞杀迁移路线）定稿待施工。
> 2026-09-09 五进：架构收尾与开发体系方案 `architecture-v2-improvements.md`（E1~E6 六波收尾施工卡 + ADR-8~14 +
> 开发流程体系 + 向导现状地图 §5.4 + 后续路线）定稿；同日 W0~W7 全部合入、E1~E6 施工完成。
> 2026-09-10 六进：模型零信任校验与自动修复 `model-trust-repair.md`（用户四条裁决 + 评审勘误；D-83）定稿施工。
> 2026-09-10 七进：「上下文数」设置界面补完 `context-turns-ui.md`（用户反馈：说明错挂 + 控件裸放 + 面板零入口；D-84）。
> 2026-09-10 八进：供应商连接测试改造与热切换加固 `translator-probe-hotswap.md`（用户反馈「测试连接按钮根本不可用」+ 追问多供应商可不可用；
> 全链路取证 → 每行测试按钮 + 独立探测线程 + 可中断 + 四态结果 + 热切换三处加固 + 会话级累计费用；D-85）。

## 活跃文档（阶段二施工依据）

| 文档 | 内容 | 状态 |
|---|---|---|
| `distribution.md` | 分发与用户旅程（D-18~D-21 裁决 + WD-1~WD-10 两阶段） | ✗ WD-1~WD-4 已完成（2026-09-09，本地 zip 打包/版本可见性/LICENSE+NOTICES/whisper 双源）；WD-6/WD-8 按用户裁决推进（横幅留接口、更新源=GitHub Releases）；公开发布（D-18 翻转）待定 |

## 归档文档（`docs/archive/`，只读）

### 阶段二已完工工作包记录（2026-09-13 归档）

| 文档 | 一句话（D-xx） | 完工日期 |
|---|---|---|
| `funasr-mlt-removal.md` | Fun-ASR-MLT-Nano 幽灵值全量移除（D-86，PROTO_VERSION=7） | 2026-09-12 |
| `translator-probe-hotswap.md` | 翻译供应商测试/热切换/配置体验改造（D-85） | 2026-09-10 |
| `context-turns-ui.md` | 「上下文数」设置界面补完（D-84） | 2026-09-10 |
| `model-trust-repair.md` | 模型零信任校验与自动修复（D-83） | 2026-09-10 |
| `llm-api-round2.md` | LLM 二轮评审修复 + 四厂商关闭思考官方查证（D-82 起） | 2026-09-10 |
| `llm-api-redesign.md` | LLM 翻译接口层改造方案与施工（W1~W5） | 2026-09-10 |
| `llm-api-review.md` | LLM API 一轮评审（P1/P2/P3 清单，驱动 W1~W5） | 2026-09-10 |
| `architecture-v2-improvements.md` | 架构 v2.1 收尾 E1~E6 + ADR-8~14 + 开发体系 | 2026-09-09 |
| `architecture-v2.md` | 架构 2.0 方案与 W0~W7 八波迁移（D-60+；拓扑权威已移交 check_deps.ps1） | 2026-09-09 |
| `architecture-review.md` | 全系统架构评审（R1~R32 风险登记册） | 2026-09-09 |
| `path-hygiene.md` | 硬编码路径排查与清理（PH-1~PH-5） | 2026-09-09 |
| `data-lifecycle.md` | 数据生命周期与足迹盘点（12 项候选全裁决） | 2026-09-09 |

### 阶段二已完工工作包记录（2026-09-09 归档）

| 文档 | 内容 | 完工状态 |
|---|---|---|
| `asr-engine-expansion.md` | ASR 引擎扩展：✗ FunASR Nano 实装（WP-A，D-24）/ ✗ Qwen3-ASR-0.6B 实装（WP-B r3.1，D-25，目标仓 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`）/ 开源模型扫描 | 全部完工（2026-09-08，349 测；验收数据在 §3.5/§4.8） |
| `asr-hardening.md` | ASR 子系统加固与状态机收口（21 项发现 H1~H21 + DEC-1~7 决策 + AH-1~AH-10 施工卡；D-26~D-28 登记） | 完成（2026-09-08，363 测；GUI 冒烟 A/B/C 待实机） |
| `download-overhaul.md` | 模型下载链路审计与改造（13 项发现证据 + DEC-1~5 决策 + DL-1~6 施工卡；D-22/D-23 登记） | 完成（2026-09-08，334 测；DL-1~6 全落地） |
| `mic-monitor-fix.md` | 麦克风输入开关与监视条（幽灵 MIC 条根因=mic_rms 代理启用状态+无 chunk 不发事件；DEC-1~5 + MC-1~4 施工卡；D-29 登记） | 完成（2026-09-08，368 测，f1f5464；实机双场景冒烟过） |
| `sensevoice-language-fix.md` | SenseVoice 语言恒 auto（三优先语言决策 resolve_language；SV-1~3 施工卡；D-30 登记） | 完成（2026-09-08，369 测，66871a0；实机探针 6 组全绿） |
| `log-tab-redesign.md` | 面板「日志」tab 界面改造（双滚动条根因=外层 ScrollArea 溢出+叠内层日志滚动区；LT-1~6 施工卡；D-31 登记） | 完成（2026-09-08，378 测，416bbfd；实机用户截图对照过） |
| `button-press-feedback.md` | 悬浮窗行1按钮按压文字右移（根因=egui button_style 内边距公式×stabilize 基准×overlay active 未覆盖；BP-1~3 施工卡；D-32 登记） | 完成（2026-09-08，380 测，4054f4b；实机长按确认待用户截图） |
| `hide-quit-flow-overhaul.md` | 主界面隐藏/退出流程改造（rfd 同步模态 MessageBoxW 三症状同源；H-1~H-6 施工卡=先藏后提示/Windows 原生 Toast/egui 内嵌模态；D-33 登记） | 完成（2026-09-08，386 测，adf5a50；toast 视觉确认待用户） |
| `hide-transparency-fix.md` | 悬浮窗隐藏→托盘重显示半透明丢失（winit apply_diff 整体重写 EXSTYLE 清手工 WS_EX_LAYERED；每帧实测缺位重挂自愈） | 完成（2026-09-08，9f27077，D-34） |
| `tray-menu-blocking-fix.md` | 托盘菜单打开主界面卡死（tray-icon TrackPopupMenu 模态循环占死 winit 线程；托盘移专用线程 lt-tray + GetMessage 泵） | 完成（2026-09-08，4fc96f0，D-35） |
| `subtitle-window-overhaul.md` | 字幕窗体验改造（顶条工具条/分区穿透/Z 序定型/重叠避让/工作区钳制/默认 bg_opacity 190；WP-1~WP-5；D-36 登记） | 完成（2026-09-08，396 测；实机走查清单见 §7.1） |

### 阶段一决策史（2026-09-07 归档）

| 文档 | 内容 | 阶段一状态 |
|---|---|---|
| `rewrite-research.md` | 选型研究（r8）+ 偏差决策史 D-1~D-21 + 风险 R-1~R-14 | 研究完成 |
| `rewrite-plan.md` | M0~M6 施工图（契约全表/算法规格/经验教训/验收清单） | 施工主体完成 |
| `parity-closure.md` | 复刻收口九 WP：WP-1/3/4 完成；WP-2 由 asr-engine-expansion WP-A 取代，WP-5/6/7/8 不再默认执行，WP-9 实机调优保留 | 部分完成 |
| `ui-realign.md` | GUI 对齐原版五阶段（"砍掉原版没有的自创功能"原则已于 2026-09-07 反转） | 已被后续决策取代 |
| `overlay-realign.md` | 悬浮窗对齐（不透明/两行排版/紧凑头/Consolas） | 修复已入库 |
| `visual-parity.md` | 视觉五工作包（删色块/三态 stroke/滚动条/面板 535×781 居中） | 完成（285 测） |
| `font-system.md` | 字体系统 W-1~W-7（内嵌思源/等宽/符号 + 级联 + 选择器，D-17/R-14） | 完成 |
| `ux-feedback.md` | 用户体验反馈闭环（下载四态卡片/日志 tab/测试连接/恢复默认/退出统一） | 一期二期完成（313 测），三期可选项入 AGENTS.md 待办 |

## 不入库副产物

- `architecture/`（archify 交互式架构图）、`ui-audit/` 均为 gitignore 副产物目录，禁止入库。
- `assets/reference/` 原版参照截图已入库（zh/en 各 10 张：控制面板 7 个标签页 + 悬浮窗/字幕窗/日志窗；2026-09-09 重拍自工作区 `LiveTranslate/` 参考副本，脚本 `scripts/grab_reference_ui.py`）。
