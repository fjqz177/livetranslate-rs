# 归档文档索引（一行一档、完工日期倒序）

> **定位**：常青参考（docs/README.md 纪律①）——`docs/archive/` 的**唯一索引真源**；docs/README.md 只留指针（2026-09-24 自该页提取，表格整体搬来，内容未改）。
> **一行一档**：文档 / 一句话（D-xx）/ 完工日期；细节只活在档案里，本表不转述。
> **维护**：收口卡第 3 步——`git mv` 归档后本表加行（一行一档），与 docs/README.md 活跃表移行同一提交。

## 归档文档

| 文档 | 一句话（D-xx） | 完工日期 |
|---|---|---|
| `scripts-usability-review.md` | 脚本评审与守护瘦身底稿（ADR-21）：已删四守护死因存档 + check_agents_health 留任五点理由；S1/S2 已落地，S3/S4 未了项细单 + 归档核销注记（随用随做、须代码解禁） | 2026-09-27 |
| `window-flash-cleanup.md` | 窗口闪现整治：启动先画后显（全窗隐藏创建+首帧揭示）/ 加载弹窗退役并入悬浮窗状态行（ModelLoadStart 内联、load_dialog 全删）/ 退出先全窗离屏再收尾；坑册 G-35 入册（D-100） | 2026-09-25 |
| `agents-meta-restructure.md` | 指令体系元方法化（ADR-22）：AGENTS 三类内容层 + 准入三问 + 四层披露链；§7 镜像→5 条防呆、§8 条目外迁 `docs/board.md`、README 归档表外迁 `docs/archive-index.md`；守护断言 7 改读 + 断言 9 看板双真源；坑册 G-33/G-34 入册；全文件 -32% | 2026-09-24 |
| `legacy-cleanup.md` | 遗留清账批：总纲 §8 瘦身（15→10 行）+ 拍板批 12 项分堆处置（D-96~99）+ gotchas 收编 G-31/32 候选清零；走查 51 条转 issue 驱动验收附录（D-96） | 2026-09-22 |
| `text-hygiene.md` | 文本卫生：全仓文本强制无 BOM 合法 UTF-8 + LF——G-23 政策翻转（剥 9 BOM）+ gitattributes 全仓锁 LF + check_personal_paths 三查 + editorconfig 改造（ADR-20） | 2026-09-19 |
| `scripts-doc-system.md` | 脚本管理与文档体系：十一脚本头注用法卡（唯一细节真源）+ docs 顶层目录页（导览不抄细节）+ 断言 8 脚本登记面双向对账 + AGENTS §2 存量参数级细节收敛（ADR-19） | 2026-09-19 |
| `asr-chain-robustness.md` | 识别→翻译→显示链路健壮性修复：退出完整性（尾巴冲刷 / 翻译排空 / 停机收口）+ 流式终态不可覆盖 + 字幕窗旁路账本 + 未就绪成对记账 + panic 补回执（D-94） | 2026-09-17 |
| `doc-network-hardening.md` | 文档网络加固：全仓清淤 + 守护断言 5~8 + 钩子触发面「守护读谁谁触发」+ 流程卡流水线（D-93，ADR-17 额度重校准） | 2026-09-17 |
| `dev-config-audit.md` | 开发配置审计收口：主体经 D-88~D-91 取代性落地 + 逐项对账（A2/B1/B2 反向选择留证）；小项补做 C2/C3/C6/C8/E1 | 2026-09-17 |
| `live-check-fixes.md` | 实机走查三修：精简模式切换可达（A2）/ 按钮三态色反馈（A1，三窗共用）/ 面板禁用态外观 + 删除全部文案（B2、D-92） | 2026-09-17 |
| `changelog-scheme.md` | 更新日志机制：仓根正典（`CHANGELOG.md`/`.en.md`）+ 发版三件套闸（check ② / CI `notes` 早警告）+ 应用内编译期内嵌（D-91） | 2026-09-16 |
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
| `architecture-v2.md` | 架构 2.0 方案与 W0~W7 八波迁移（D-60+；拓扑权威=topology.rs） | 2026-09-09 |
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

## 维护与对账（机械面）

- **加行时机** = 收口卡第 3 步：`git mv docs/<包>.md docs/archive/` 后本表加行（一行一档），与 docs/README.md「活跃文档」表移行同一提交（docs/README.md 纪律⑤「挪卡连带改索引」）。
- **断言 7 解析口径**（`scripts/check_agents_health.ps1`）：本档「## 归档文档」节内的表行须形如 ``| `文件名.md` | … |``（小写 kebab 文件名），并与 `docs/archive/` 目录双向对账（缺行 / 多行 / 孤儿都红）；每份档案前 8 行须含「已归档 / 归档注记」。
- **节序约束**：断言按「`## 归档文档` 到下一个 `## ` 标题」截取本节——本节的表不得置于文末（现由本「维护与对账」节兜底）；重建索引结构时勿删本节标题。
