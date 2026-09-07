# docs 目录索引

> 2026-09-07 归档重组：阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档全部归档至 `docs/archive/`（只读决策史）；
> 阶段二（Rust 自有产品化）施工依据为本目录活跃文档 + `AGENTS.md` 待办。阶段一后 Python 原版仅作行为参考，
> 新增能力以产品体验为准，行为差异落档为 D-22 起新编号（决策史目录：`docs/archive/rewrite-research.md` §1.5）。

## 活跃文档（阶段二施工依据）

| 文档 | 内容 | 状态 |
|---|---|---|
| `distribution.md` | 分发与用户旅程（D-18~D-21 裁决 + WD-1~WD-10 两阶段） | 阶段一 WD-1~WD-5 待施工，阶段二预案 |
| `asr-engine-expansion.md` | ASR 引擎扩展：✗ FunASR Nano 实装（WP-A，D-24）/ Qwen3-ASR-0.6B 实装方案（WP-B r3 动工定稿，目标仓 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`，B-α 拓扑）/ 开源模型扫描 | WP-A 完成（2026-09-08，341 测）；WP-B r3 动工方案定稿（2026-09-08），待施工 |
| `download-overhaul.md` | 模型下载链路审计与改造（13 项发现证据 + DEC-1~5 决策 + DL-1~6 施工卡；D-22/D-23 登记） | 完成（2026-09-08，334 测；DL-1~6 全落地） |

## 归档文档（`docs/archive/`，只读决策史）

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
- `assets/reference/` 原版参照截图已入库（zh/en 各 4 张）。
