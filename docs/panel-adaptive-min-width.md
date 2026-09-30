# 面板最小宽度自适应化（D-122）

> **本文是什么**：D-122 施工定稿——控制面板最小宽度从硬编码常数改**运行时派生**的根因、裁决、设计与施工清单。
> 触发：D-120 修复在实机被证伪——en 界面最小窗 tab 条最右页签仍被右缘截断（用户 2026-09-30 两帧截图：903 物理px 裁 / 939 物理px 完好，ppp≈1.5）。用户定方向：**最小宽度必须是推导值，不是拍出来的常数**。
> 规格与拆单 = GitHub Issues #28（父单规格）与 #29~#33（T1~T5，原生阻塞边）。收口后本文迁 `docs/archive/`。

## 一、根因（事实走查已坐实，三条叠加）

1. **测试标尺漂移（D-120 盲区的真相）**：D-120 文档声称实测标尺 = 「内嵌思源字体 = 跨机确定」（`docs/panel-narrow-layout.md:48`），但提交进去的 L3 溢出测试 ctx 是 `egui::Context::default()`，**从未调用 `fonts::apply_fonts`**（`crates/lt-ui/src/windows/panel/mod.rs:628`）——「en tab 行 ≈591」实为 egui **默认字体**度量；实机装思源链（宿主 `app.rs:83`）度量更宽，en tab 行实机 >600，最小窗必裁。
2. **零反馈回路**：`min_inner_size` 只在创建期设一次（`app.rs:259`），全仓无任何运行时 `set_min_inner_size` 调用——窗口最小宽对语言热切换（D-121）、字体更换、DPI 全部失明。
3. **常数的宿命**：600 无法追「字体 × 语言 × DPI」的乘积。**用户实证**：实机 `ui_font_family` = 系统「Noto Sans CJK SC」（非内嵌链），其 Latin 度量更宽——en tab 条实机 >600；内嵌思源链 headless 实测 en 派生 617.3（600 客户区正好裁 ~9px「Logs」，与截图量级一致）、zh 509.7。600 的全部消费点恰 3 处：默认创建宽（`app.rs:152`）、创建期 `with_min_inner_size`（`app.rs:259`）、测试视口（`panel/mod.rs:625`）——替换面收敛，适合连根拔。

**原料现成**：tab 条自绘逐页签 `layout_no_wrap` 测宽（`panel/mod.rs:281-284`），每帧本来就在算 8 个页签的自然宽——「派生 min」零额外排版成本。灰框（`group_card`）内容 `set_width(available_width)`（`mod.rs:325`），内部不可折行横排溢出会把框 min_rect 撑大越出视口（实机第二症状的机制）。

## 二、裁决（D-122，2026-09-30 用户拍板，问答八项全按推荐）

| # | 项 | 裁决 |
|---|---|---|
| 1 | min width 产生机制 | **运行时实测单源**：UI 用真实字体测「tab 行自然宽＋镶边」→ 经 WinAction 让宿主 `set_min_inner_size`；删 600 硬编码。**推翻 D-120 裁决 #2（常数定值）与 #3（tab 条运行时保底不做）** |
| 2 | tab 折行 | **永不折行**，min width 无上限跟随（不设折行上限） |
| 3 | 灰框宽度法则 | **框宽 = 窗宽 − 2×边距 token**；内容一律折行适应框宽，无内容级 min width |
| 4 | 语言热切换 | 当前窗宽 < 新 min → 宿主**立即抬窗**到新 min |
| 5 | 内容并入 | **否**——min 只由 tab 行推导（正文皆可折行；等宽模型行远窄于 tab 行；超长用户 URL 走折行消化） |
| 6 | 范围 | **仅控制面板**；#27 悬浮窗独立工单（病根同类：min 480 硬编码 `app.rs:239-241`，无溢出测试） |
| 7 | 测试防线 | **四件套全上**（§四.4） |
| 8 | 流程 | 共识确认 → 本定稿 → 点头 → docs 提交＋D-122 登记 → 施工 |

## 三、设计

### 3.1 公式（单源）

```
panel_min_width = tab_strip_natural_width + 左右镶边 + slack(8)
tab_strip_natural_width = Σ( layout_no_wrap(页签文案, 12.5号).width + 20 ) + 页签间距 × 7
```

- **同一套数字**：从 `tab_strip` 抽出「逐页签测宽」纯函数，绘制与推导共用——禁两套标尺再漂移。
- **镶边项**：页签条外层到窗缘的 inset 构成（TopBottomPanel/页套 `Margin::same(12)`），施工首日以 headless 实测逐项校准；页套边距抽常量 `PANEL_PAGE_MARGIN: f32 = 12.0` 单源（灰框离窗边的「常数」即它，裁决 3 的 token）。
- 输入 = ctx 字体 + 当前语言页签键集（8 键：`state.rs:1495-1506`）；按 (lang, 字体版本) 记忆化，稳态零重算。

### 3.2 通道（仿 `WinAction::SetHeight(f32)` 既有模式）

- 新变体 `WinAction::SetPanelMinWidth(f32)`（`state.rs`——lt-ui 内部意图通道，不触 lt-proto 契约，PROTO_VERSION 不动）。
- 面板 UI 每帧比对派生值，**变化才发**；宿主臂（`app.rs` process_actions）执行 `set_min_inner_size(derived, 420)`；**若当前宽 < derived → `request_inner_size` 抬到 derived**（裁决 4）。
- 语言热切换无需在 `ApplyUiLang` 臂特判尺寸：语言变 → 下一帧页签文案变 → 派生值变 → 同一回路自动到达。
- 创建期（`app.rs:152/259`）：~~创建前先算一次派生 min~~ **施工偏离留痕（ADR-14）**：egui Fonts 惰性初始化，`Context::run` 之前无字体可排版（`context.rs` 硬断言），创建期派生不可行——改为 `with_min_inner_size` 用 `PANEL_DEFAULT_WIDTH` **垫底**，首帧派生真值经 `WinAction::SetPanelMinWidth` 到宿主臂校正（含当前宽不足即抬窗）。默认创建宽 = `PANEL_DEFAULT_WIDTH`（600 观感不变）。

### 3.3 常数处置

- `PANEL_MIN_WIDTH = 600.0`（`panel/mod.rs:41`）**删除**——它是错误断言（「600 内放得下」），不是参数。
- 新 `PANEL_DEFAULT_WIDTH: f32 = 600.0` 仅作**首启观感下限**（默认创建宽用），不参与 min 正确性——zh 派生 min（≈520-540）小于 600 时，用户可拖到派生 min 为止，内容在派生 min 下必须放得下（由四件套保证，红了拆行不抬 min）。

### 3.4 内容纪律（裁决 3 的执行机制）

内容行在派生 min 下溢出 → 按 D-120 成文规则 R1/R2 **拆行或折行**，**禁回灌常数抬 min**。预期 zh 派生 min 处 1-3 行需小改造，施工时如实回报观感变化。

## 四、施工清单（文件级）

1. `crates/lt-ui/src/windows/panel/mod.rs`：抽「逐页签测宽」纯函数 + `derived_panel_min_width` 派生函数；`tab_strip` 绘制改用同一函数；删 `PANEL_MIN_WIDTH`，增 `PANEL_DEFAULT_WIDTH` / `PANEL_PAGE_MARGIN` token。
2. `crates/lt-ui/src/state.rs`：`WinAction::SetPanelMinWidth(f32)` 变体。
3. `crates/lt-ui/src/app.rs`：宿主臂（`set_min_inner_size` + 条件 `request_inner_size`）；创建期改派生值（`app.rs:152/259`）。
4. **测试四件套**（`panel/mod.rs` tests）：
   - ① 既有溢出测试 ctx 装 `fonts::apply_fonts`（真实内嵌思源链）——根修标尺漂移（`mod.rs:628`）；
   - ② 断言视口 = `derived_panel_min_width(ctx, lang)`（调用真公式），弃常数视口；
   - ③ 新回归用例：**en＋真实字体＋视口=派生 min** → 最右页签（Logs）完整可见——复现并锁死本次 bug；
   - ④ 边距恒定断言：渲染后 group 框右缘 = 视口右缘 − `PANEL_PAGE_MARGIN`（±2.0）。
5. 文档同步（与 docs 提交同批）：`docs/decisions.md` 登记 D-122（取代 D-120 裁决 #2/#3，不删行）；`docs/panel-narrow-layout.md` 加「→ 被 D-122 取代」注记；gotchas G-39 对策节补「动态派生 min」；本文转正 `docs/panel-adaptive-min-width.md`。

## 五、验收

1. `cargo test --workspace` 全绿（含四件套）＋ `precommit.ps1` 三项过。
2. 用户实机走查：zh/en 各自拖到最窄逐 tab 右缘完整；en↔zh 热切换窗口宽度即时跟随（变窄、变宽双向）；灰框左右边距恒定。

## 六、不自信点 / 风险（如实列）

- ~~镶边构成未逐项核实~~ 已兑现：tab 条记账常量（左 inset 4 / 页签间距 8 / 页签 pad 20）与绘制同一套，单源直断测试钉「实摆右缘＝公式」；实测派生 zh 509.7 / en 617.3。
- ~~zh 派生 min 可能明显 < 600~~ 实测 509.7：zh/en × 8 页在派生视口全部无可见溢出，零拆行（唯一越界者为 egui ScrollArea 全透明滚动条几何残影 ~2.3px，肉眼不可见，溢出扫描已加透明图元过滤并留注）。
- **en 事实锚方向更正留痕**：定稿阶段曾推断「内嵌链 en 亦 >600」成立（617.3，实测确认）；期间因测试语言时序污染（派生先于 set_lang）一度误得 509.7，已修——教训 = 派生是语言的函数，取值前必须显式设语。
- **瞬跳无动画**：热切换抬窗是 winit 瞬跳（裁决 4 已知并接受）。
- **#27 悬浮窗不在本批**：其 min 480 同病但无 tab 条，公式不同，后续另案复用「派生 min」思路。
