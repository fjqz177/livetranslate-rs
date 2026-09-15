# docs 目录索引

## 本目录怎么运转（纪律条文，2026-09-13 定稿）

1. **四个区**：`README.md`（本看板）｜顶层 `*.md` 分两类——**常青参考**（decisions / distribution / gotchas 与 `prompts/` 目录，无完结条件、不计数）与**工作包文档**（健康态 ≤3 份，完工即归档）｜`drafts/`（未拍板草稿，不入库，看板=AGENTS 待拍板块）｜`archive/`（完工决策史，只读，仅许修订注记与归档波次路径替换）。
2. **什么活立档**：有要留档的裁决（将发 D-xx/ADR-x）或多步施工要验收清单——满足其一立档；两者皆无直接修，不立档不给号。草稿阶段可跳过，定稿阶段不可。
3. **一份文档一条路**：草稿 → 定稿（进 git，`docs(scope)` 提交必须早于第一行实现代码）→ 施工中 → 完工即归档。不许跳站，不许滞留。
4. **完工标准** = 测试全绿 + clippy 0 + 守护过。实机走查、未裁决小遗留都不拦归档——清单留在归档文档里，AGENTS 遗留区留一行指针，清零即删。
5. **挪卡连带改索引**：文件挪动、README 移行、decisions.md 落档路径、AGENTS 收敛，永远在同一个提交里。收口三问：`ls docs/` 健康吗？decisions.md 漏行吗？表和文件对上没？
6. **编号就两层**：全局 `D-xx`（产品/行为决策）与 `ADR-x`（架构决策）——号随定稿发、单体递增、永不复用、禁预留号段、先登记 `decisions.md` 后引用、下一个号 = 表尾 +1、被推翻加「→ 被 D-xx 取代」不删行；局部 = 文档专属前缀-号（头注声明、slug 派生、全局唯一含 archive），WP/W/R/C/S/H/E/M/DEC 禁作前缀，局部号出文档必带路径。
7. **状态词四个**：草稿 / 定稿 / 施工中 / 完工已归档。完工写「完工」二字，不用 ✗。
8. **新工作包文档**：文件名 kebab-case 无日期；头注四行（状态/日期/触发/前缀）；六节骨架（背景取证→方案裁决→施工卡→验收基线→遗留走查）。
9. **本 README 是看板不是日记**：不写流水账，细节只活在文档里；纪律条文超 ~25 行，就是它开始变成第二个 AGENTS 的信号。

## 活跃文档

| 文档 | 状态 | 内容一句话 |
|---|---|---|
| （无） | | |

## 长期文档（有完结条件，满足即归档）

| 文档 | 内容 | 完结条件 |
|---|---|---|
| `distribution.md` | 分发与用户旅程（D-18~D-21 裁决 + WD 工作包） | 公开发布落地 + WD 全清 |
| `decisions.md` | 决策总登记（D-xx / ADR-x，先登记后引用） | 项目终结（实际永久） |
| `gotchas.md` | 大坑全书（触发词五段式，G-编号；AGENTS §7 速查真源） | 项目终结（实际永久） |
| `prompts/` | 开发流程七卡（阶段自查清单；索引 = AGENTS §4 路由表） | 项目终结（实际永久） |

## 归档文档（一行一档、完工日期倒序，细节在文档里）

| 文档 | 一句话（D-xx） | 完工日期 |
|---|---|---|
| `ci-workflow-suite.md` | CI 工作流全家桶：单 job 整脚本入 CI + 工具链钉版 + cargo-deny + tag→Draft 发布链（D-88） | 2026-09-15 |
| `quit-flow-redesign.md` | 退出与确认系统重做：专用确认窗包办六确认（D-87） | 2026-09-14 |
| `agents-md-overhaul.md` | 指令体系四层化：AGENTS 总纲+坑册+七卡+守护第七项+pwsh-only（ADR-15/16） | 2026-09-14 |
| `funasr-mlt-removal.md` | Fun-ASR-MLT-Nano 幽灵值全量移除（D-86，PROTO_VERSION=7） | 2026-09-12 |
| `translator-probe-hotswap.md` | 翻译供应商测试/热切换/配置体验改造（D-85） | 2026-09-10 |
| `context-turns-ui.md` | 「上下文数」设置界面补完（D-84） | 2026-09-10 |
| `model-trust-repair.md` | 模型零信任校验与自动修复（D-83） | 2026-09-10 |
| `llm-api-round2.md` | LLM 二轮评审修复 + 四厂商关闭思考官方查证（D-82 起） | 2026-09-10 |
| `llm-api-redesign.md` | LLM 翻译接口层改造方案与施工（W1~W5） | 2026-09-10 |
| `llm-api-review.md` | LLM API 一轮评审（P1/P2/P3 清单，驱动 W1~W5） | 2026-09-10 |
| `architecture-v2-improvements.md` | 架构 v2.1 收尾 E1~E6 + ADR-8~14 + 开发体系 | 2026-09-09 |
| `architecture-v2.md` | 架构 2.0 方案与 W0~W7 八波迁移（D-60+；拓扑权威=check_deps.ps1） | 2026-09-09 |
| `architecture-review.md` | 全系统架构评审（R1~R32 风险登记册） | 2026-09-09 |
| `path-hygiene.md` | 硬编码路径排查与清理（PH-1~PH-5） | 2026-09-09 |
| `data-lifecycle.md` | 数据生命周期与足迹盘点（12 项候选全裁决） | 2026-09-09 |
| `asr-engine-expansion.md` | FunASR Nano + Qwen3-ASR 引擎实装（D-24/D-25） | 2026-09-08 |
| `asr-hardening.md` | ASR 子系统加固 AH-1~10（D-26~D-28） | 2026-09-08 |
| `download-overhaul.md` | 模型下载链路改造 DL-1~6（D-22/D-23） | 2026-09-08 |
| `mic-monitor-fix.md` | 麦克风监控幽灵条修复（D-29） | 2026-09-08 |
| `sensevoice-language-fix.md` | SenseVoice 语言恒 auto 修复（D-30） | 2026-09-08 |
| `log-tab-redesign.md` | 面板「日志」tab 界面改造（D-31） | 2026-09-08 |
| `button-press-feedback.md` | 悬浮窗按钮按压反馈稳定（D-32） | 2026-09-08 |
| `hide-quit-flow-overhaul.md` | 隐藏/退出流程改造 + 原生通知（D-33） | 2026-09-08 |
| `hide-transparency-fix.md` | 隐藏重显半透明丢失自愈修复（D-34） | 2026-09-08 |
| `tray-menu-blocking-fix.md` | 托盘菜单卡死修复——专用线程（D-35） | 2026-09-08 |
| `subtitle-window-overhaul.md` | 字幕窗体验改造（D-36，含 D-37 拖动修复） | 2026-09-08 |
| `rewrite-research.md` | 阶段一选型研究 + 偏差决策史 D-1~D-21 + 风险 R-1~R-14 | 2026-09-07 |
| `rewrite-plan.md` | 阶段一 M0~M6 施工图（契约全表/算法规格/验收清单） | 2026-09-07 |
| `parity-closure.md` | 复刻收口九 WP 处置（WP-2 由引擎扩展取代，WP-9 保留） | 2026-09-07 |
| `ui-realign.md` | GUI 对齐原版方案（「砍自创功能」原则已被 2026-09-07 定调取代） | 2026-09-07 |
| `visual-parity.md` | 视觉五工作包（删色块/三态 stroke/滚动条/面板 535×781） | 2026-09-07 |
| `font-system.md` | 字体系统 W-1~W-7（内嵌思源 + 双主旋钮级联，D-17） | 2026-09-07 |
| `ux-feedback.md` | 用户体验反馈闭环（下载四态卡片/日志 tab/退出统一） | 2026-09-07 |
| `overlay-realign.md` | 悬浮窗对齐方案（不透明/两行排版/紧凑头） | 2026-09-06 |

## 不入库副产物

- `architecture/`（archify 交互式架构图）、`ui-audit/`（实机走查截图）、`drafts/`（未拍板草稿）均 gitignore，禁止入库。
- `assets/reference/` 原版参照截图已入库（zh/en 各 10 张，2026-09-09 拍自工作区 `LiveTranslate/` 参考副本，脚本 `scripts/grab_reference_ui.py`）。
