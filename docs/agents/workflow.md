# Workflow — Matt Pocock Skills ↔ 本仓映射（流程层唯一真源，D-102）

> **本文是什么**：流程层唯一真源。Matt Pocock Skills 套件为唯一流程载体（推翻 D-97；prompts 七卡已删、本地看板已退役——路径与去向见台账 D-102）。硬约束 / 门禁 / 拓扑真源仍在 AGENTS.md，本文不复述；分工：本文改流程不动约束，AGENTS 改约束不动流程。

## 1. 主流程映射（idea → ship）

| 阶段 | 用什么 | 本仓落点 |
|---|---|---|
| 想法 sharpen | `/grill-with-docs`（仓库内一律用它，不用 `/grill-me`） | 决策落 `docs/decisions.md` 台账（D-xx / ADR-x，先登记后引用），不另设 ADR 目录；`CONTEXT.md` 惰建、缺失静默 |
| 规格与拆单 | `/to-spec` → `/to-tickets`（多会话大活才拆，小活直接进施工） | 拆单 = GitHub issue（`gh` CLI，细则 = `docs/agents/issue-tracker.md`） |
| 施工 | `/implement`：每张 issue 一个**新会话**，内部跑 `/tdd` 一红一绿，收尾 `/code-review` | 收工门禁 = `cargo test --workspace` 全绿；提交门禁 = `scripts/precommit.ps1`（AGENTS §2） |
| 收口 | 收口三问（`docs/README.md` 纪律⑤）+ 归档机械面（守护断言 7） | 实施后逐条回报拍板清单：做了 / 未做 + 为什么 |

- **定稿先行**不变：`docs(scope)` 定稿提交早于第一行实现代码；决策号随定稿同一提交登记。
- **三条汇入**：外来请求堆积 → `/triage`；难缠 bug（第一眼修不掉）→ `/diagnosing-bugs`；大雾级工程（一号会话装不下）→ `/wayfinder`——最终都汇回本表。想不起用哪个 → `/ask-matt`。

## 2. 开工令与标签（2026-09-28 用户拍板）

- **ready-for-agent ≠ 开工令**：标签只代表工单规格合格；agent 只在用户明确指派 / 点头时动工。
- **ready-for-human = 等用户裁决**：须按 AGENTS「拍板机制」节出编号清单，未拍板禁动工。
- 分诊标签映射 = `docs/agents/triage-labels.md`；状态真源 = GitHub Issues（本地看板已废，**禁再造**）。

## 3. 施工纪律

- 分层自查照 AGENTS §3（topology.rs 会拦，别攒到提交才红）；禁令豁免就地 `#[allow]` + 理由。
- 行为改动带测试；离线纪律（禁真模型 / 真网络 / 真设备，探针 `#[ignore]`）+ 临时目录唯一化（G-22）；fmt / clippy 滚动跑，别攒到最后。
- 里程碑 + 全绿即自主中文 commit；多代理并行时提交前 `git status` / `git diff` 逐项复核，显式 pathspec、禁 `add -A`（G-19）。
- 用户文案：orchestrator 域经 `Msg` 注入；只有 lt-ui 碰 i18n yaml（zh/en 必须同步改）。
- 踩新坑当场记 `docs/gotchas.md` 候选；中途新裁决当场登记 decisions.md，别攒到收口。
- 子代理分工：检索 / 执行类下放快模型；承重墙（Win32 / 链接 CRT / 下载器 / 算法移植）主会话亲做。
- CUA 节制使用（2026-09-14 解禁）：只读探针优先；操作须有明确验收点；有更轻替代（headless 探针 / 单测 / 请用户截图）时不为用而用。

## 4. 走查与收口

- GUI 冒烟配方 = 根 `README.md` §二（临时 CONFIG_DIR + `models_dir` 指真实缓存 + `LIVETRANSLATE_SHOW_PANEL=1` 走查旗标；`--version` = 产物能起的最小证明）。
- headless 取证主干：run_ui + RawInput 模拟指针 + 扫 `Shape::Text`（位移 / 状态断言）；视觉证据请用户截图，自验收用全屏截图（PrintWindow 抓旧帧 = G-10）。
- 实机走查 issue 驱动：一包一 issue，正文链档案锚点（`docs/archive/<包>.md` §x）；通过即关 issue。
- 全仓旧路径替换后 grep 复盘（含 .rs 注释）；未跟踪草稿（`docs/drafts/`）禁 `git mv`（不入库目录——复制后 `git add`）。

## 5. 交接与会话卫生

- 跨会话 / 换目录 / 中途分叉用 `/handoff` 产出可移植 md；给下一会话留一句话锚点（进行到哪、下一步做什么）。
- 项目记忆写仓外 ZCode 记忆区（~/.zcode，不入本仓）；相对日期一律换算为绝对日期。
- 会话收尾：git status 干净，或明确说明未提交物是什么、为什么（drafts 除外——本就不入库）。

## 6. 不变量（换血不动的承重墙）

机器门禁（precommit / CI / clippy.toml / topology.rs / repo_hygiene.rs / contract_purity.rs）、`docs/decisions.md` 台账、`docs/gotchas.md` 坑册、`docs/archive/`、发布链——全部原样。
