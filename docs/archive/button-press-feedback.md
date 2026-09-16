# 悬浮窗行1按钮按压时文字右移 —— 三态几何不稳定（D-32）

> 用户反馈（2026-09-08）：主界面（悬浮窗）顶部行「完整」「清除」等按钮，
> 按下时按钮内文字向右偏移几个像素；长按时先右移、约一秒后又回到原位，
> 观感诡异。目标：查明根因并给出整体按钮按压动态的改造计划。
>
> **修订注记（2026-09-17，走查 A1 修复）**：BP-4「按压反馈 = 仅底色加深」**自落地起从未生效**——
> `small_btn` 的显式 `Button::fill` 会覆盖全部状态底色（egui 文档原话 "will override any
> on-hover effects"），且三态色写在了 `bg_fill`、按钮实际读 `weak_bg_fill`，两层叠加使
> 悬停/按压与静止完全同色；用户实机走查实测「按钮没有任何变化」证实。本档的**文字零位移**
> 结论有效（回归测试在钉，不受影响）；底色反馈已由 `style::skin_button` 重做落地，
> 见 `docs/archive/live-check-fixes.md`（D-92 批）。

## 1. 现象与目标定位

- 现象窗口：**悬浮窗（WinId::Overlay）顶行 row1**（隐藏/字幕/运行/清除/完整/设置/退出），
  面板、日志窗、字幕窗不受影响。
- 文案佐证：用户口中的「清除」= `t("clear")` = **"清除"**（`assets/i18n/zh.yaml:19`），
  悬浮窗行1按钮文案与用户描述 1:1 吻合，方向定位无歧义。
- 用户诉求：按压时文字**零位移**；按压反馈改由**颜色**承担（按动态整体可感知、不跳字）。

## 2. 程序化实机证据（无头探针 TS-3，逐帧模拟指针按压）

复刻 app.rs `run_frame` 的完整 visuals 注入链（host 全域 visuals +
`stabilize_widget_strokes` + overlay_ui 原位三态覆盖），同帧布局下
**idle → hover → press（连跑 3 帧）**逐帧提取按钮文字 galley 坐标：

```
panel:   idle=(4.0 3.5)  hover=(4.0 3.5)  press1=press2=press3=(4.0 3.5)   ← 零位移
overlay: idle=(6.0 3.5)  hover=(6.0 3.5)  press1=press2=press3=(7.0 3.5)   ← 按压右移 1.0 逻辑px
```

- 位移幅度 **1.0 逻辑像素**；用户机 2560×1600 全屏（200% DPI）→ **2 物理像素**，
  与「偏差几个像素」的主观描述一致。
- 位移方向 **向右**（6.0→7.0），与用户描述一致。
- 位移**随按住持续存在**（press1~3 均为 7.0）——与「长按期间文字在偏移位」一致。
- 对照：面板三态全 `4.0`（`panel_visuals` 三态齐全），引擎下拉组合、日志工具行、
  面板页签条在真实 panel_ui 下逐帧探针（TS-2）三态坐标**全部恒定** ——
  全应用只有悬浮窗行1存在按压位移，边界精确锁定。

探针文件为临时证物（`crates/lt-ui/tests/press_probe.rs`，不随本计划提交，
核心代码与输出见附录 A）。

## 3. 根因链

### 3.1 egui 0.36 的按钮内边距公式（源码事实）

`egui-0.36.1/src/widget_style.rs:146-177` `button_style()`：

```rust
inner_margin: (self.spacing.button_padding + Vec2::splat(visuals.expansion) - Vec2::splat(visuals.bg_stroke.width)).into()
```

即 **按钮文字横向位置 = button_padding − 该状态 bg_stroke.width**（expansion 默认 0）。
`Visuals::dark()` 默认三态 bg_stroke 宽度为 **inactive 0 / hovered 1 / active 1**；
`Visuals::light()` 默认 **0 / 1 / 1**。**同一按钮跨状态描边宽度不一致 → 文字横移。**

另：`egui-0.36.1/src/widgets/button.rs:325-329` `atom_ui()` 通过
`ui.ctx().read_response(id)` 取**上一帧**响应决定状态 → 按压态渲染**滞后一帧**。

### 3.2 本项目样式注入链的三处叠加（缺陷成因）

1. **宿主每帧注入**（`crates/lt-ui/src/app.rs:291-296`）：先取
   `Visuals::dark()`（悬浮窗等）或 `panel_visuals()`，再调
   `style::stabilize_widget_strokes`。
2. **stabilize 基准缺陷**（`crates/lt-ui/src/style.rs:186-196`）：
   `stabilize_widget_strokes` 以 **inactive 的 bg_stroke 宽度为基准**拉齐
   hovered/active——dark 默认 inactive 为 `Stroke::STROKE_NONE`（宽 0）→
   悬浮窗 hovered/active 被压成 **0 宽**（WP-B 的「消 hover 文字微移」只对
   hover 生效且只压不抬，正是在这里埋下 active 侧落差）。
3. **悬浮窗原位覆盖只做了一半**（`crates/lt-ui/src/windows/overlay.rs:85-89`）：
   `overlay_ui` 每帧只重设 `widgets.inactive / widgets.hovered` 的
   bg_fill/bg_stroke（**宽 1.0**），**未触碰 `widgets.active`** →
   active 停留在 stabilize 给出的 **0 宽**。

**合成结果**：hovered 内边距 = 6−1 = 5 → 文字 x=6.0（含 Frame::paint 描边内缩
再 +1）；active 内边距 = 6−0 = 6 → 文字 x=**7.0**。按下瞬间文字右移 1.0 逻辑
像素（对应 200% DPI 2 物理 px）。

### 3.3 为何其他场景免疫（源码对照）

- **面板**：`panel_visuals`（`panel/mod.rs:362-380`）**明确写出**三态
  bg_stroke/fg_stroke/corner_radius（全 1.0）→ stabilize 后仍全 1.0 → 零位移。
- **日志窗/向导/基准窗**：纯 `Visuals::dark()` + stabilize → 三态**全 0 宽齐平**，
  无差值 → 零位移（但代价是深色窗按钮 **hover/active 完全无描边反馈**）。
- **ComboBox**：`combo_box.rs:358` 文字经 `Align2::LEFT_CENTER` 落在
  `allocate_space` 的 rect 内，与状态无关（TS-2 实测恒定）。
- **页签条**：自绘 `painter().text(rect.center(), …)`，无状态相关几何（TS-2 实测恒定）。

### 3.4 「长按 ~1s 回弹」的解释（证据分级标注）

按 3.1 公式，按住期间位移**应持续存在**；「回弹」需要用应用的重绘节奏解释：

- **候选 A（首选，与按钮尺寸并存自洽）**：row1 按钮仅 **20 逻辑 px 高**、文字
  11px，长按 ~1s 时手腕微漂使指针滑出按钮判区 → `response.is_pointer_button_down_on()`
  变 false → 状态回 hovered → 文字回原位（6.0）。**修复后（三态几何全等）即使
  滑出也无位移可回弹**，该候选自然化解。
- **候选 B（并存节奏）**：`read_response` 一帧滞后 + 悬浮窗**事件驱动重绘**
  （静止时仅输入事件/1s 监视 tick 触发，`overlay.rs:70-73` 动画帧除外）→
  用户感知的「按下→跳→(约 1s 内)回」是「按下帧(旧状态) → 重绘帧(新状态) →
  指针/焦点评判变化帧」的节拍，与候选 A 叠加。
- **结论**：回弹的确切成因需实机视频/截图确认（施工前用户一按即验）；
  但**两候选均以「按压位移存在」为前提，根修后同时消失**，不影响施工决策。

## 4. 方案对比与决策

| 方案 | 内容 | 判定 |
| --- | --- | --- |
| BP-1 悬浮窗补 active 覆盖 | overlay.rs 三态覆盖补 `widgets.active`（bg_fill/bg_stroke 宽 1.0/圆角同款） | ✅ 必做：消除位移的直接手段 |
| BP-2 stabilize 基准修正 | 基准宽 = `inactive.width.max(1.0)`（dark 0 → 抬 1.0；panel 1.0 不变） | ✅ 必做：消除同类隐患的全局兜底（深色窗按钮同时获得 hover/active 描边反馈，属预期增强） |
| BP-3 回归测试下沉 | `widget_stability.rs` 扩展「active 态」用例 + overlay 注入链用例 | ✅ 必做：本次漏网的直接原因=既有测试只测 hover |
| 按压动态（产品向） | 悬浮窗行1按压反馈 = **仅底色加深**（新常量 `BTN_PRESSED_FILL`），几何/字重/圆角三态全等；对齐原版 QSS「hover 只变底色字色」（`LiveTranslate/subtitle_overlay.py:366` 语境，阶段二以产品体验为准不追求复刻） | ✅ 一并落地（用户诉求「整体动态优化」的落点） |
| egui 级内容下压（仿原生按钮 1px 内缩） | 手工给内容加 press 偏移 | ❌ 与用户诉求相悖（要求零位移），且需逐按钮侵入式改字；否决 |

**决策**：BP-1 + BP-2 双保险根修（各自独立消除位移，互不依赖）；
BP-3 把「按压态几何稳定」固化为回归测试；按压反馈按 BP-4 仅颜色化。
零契约变更（不涉及 lt-proto）；i18n 无新键。

## 5. 方案设计

### 5.1 overlay.rs（`crates/lt-ui/src/windows/overlay.rs` ≈85-89）

补第三态覆盖并新增按压底色常量：

```rust
// 常量区新增（对齐原版 QSS pressed 只变底色语义）
const BTN_PRESSED_FILL: Color32 = Color32::from_rgba_premultiplied(55, 60, 72, 60);

// overlay_ui 原位覆盖段追加：
vis.widgets.active.bg_fill = BTN_PRESSED_FILL;
vis.widgets.active.bg_stroke = Stroke::new(1.0, BTN_STROKE);
vis.widgets.active.corner_radius = CornerRadius::same(3);
```

注：`.fill()/.stroke()/.corner_radius()` 的按钮级覆盖（`small_btn`）只影响
**帧外观**，不影响 button_style 的 margin 计算——**根修必须落 visuals 三态**，
按钮级不动。文案注释同步更新为「三态几何全等：描边/字重/圆角一致，按压仅底色加深」。

### 5.2 style.rs（`crates/lt-ui/src/style.rs:186-196`）

```rust
pub fn stabilize_widget_strokes(v: &mut egui::Visuals) {
    let stroke_w = v.widgets.inactive.bg_stroke.width.max(1.0); // dark 默认 0 → 抬到 1.0
    v.widgets.hovered.bg_stroke.width = stroke_w;
    v.widgets.active.bg_stroke.width = stroke_w;
    // fg_stroke / corner_radius 逻辑不变（基准取 inactive 原值即可，dark 默认 fg=1.0）
    ...
}
```

函数头注释补充：三位一体的「**三态几何稳定**」原则——按钮内容横向位置 =
`button_padding − bg_stroke.width`，故任何窗口、任何按钮的 hover/active 与
inactive 描边宽度必须全等（WP-B 只覆盖了 hover，本次补齐 active）。

### 5.3 回归测试（`crates/lt-ui/tests/widget_stability.rs` 扩展）

1. `press_state_text_position_is_stable`：dark + stabilize + overlay 原位覆盖
   （inactive/hovered 1.0）→ 模拟 `PointerButton pressed` 事件连跑 3 帧 →
   断言文字 galley rect 三态**完全相等**（含 dx/dy 差值输出）。
2. `stabilize_reaches_one_px_floor`：dark 输入 → 三态 bg_stroke.width 全 ≥1.0
   且互等；panel 输入 → 全 1.0（兼防 `panel_visuals` 未来回退）。
3. 既有 `button_text_does_not_shift_on_hover` 等保持不动（hover 路径回归钉）。

### 5.4 不改动项（复核结论）

- tab_strip / ComboBox / Checkbox / logwin / subtitle / setup / bench：已证明
  无状态相关几何（TS-2），不回归、不动。
- 面板 `panel_visuals` 三态色板、`stabilize` 的 fg_stroke/corner_radius 拉齐逻辑：保留。
- 动画（模式切换 200ms SetHeight 等）：与按压位移无关，不动。

## 6. 施工卡

| 卡 | 内容 | 产出 |
| --- | --- | --- |
| BP-1 | overlay.rs 补 active 三态 + `BTN_PRESSED_FILL` 常量 | 悬浮窗行1按压零位移 + 底色加深反馈 |
| BP-2 | style.rs stabilize 基准 `max(1.0)` + 注释更新 | 深色窗按钮三态统一（hover/active 描边 1.0） |
| BP-3 | widget_stability.rs 新增 2 用例（active 序列 + 基准下限） | 回归钉（~380 测） |
| BP-4 | 实机验收（见 §7） | 用户确认 |
| BP-5 | 完工回写：docs 状态、AGENTS 待办、本档完结标记 | 文档同步 |

提交拆分（docs 先行）：`docs(button-press)`（本档 + README 索引）→
`feat(overlay-press): BP-1~3` →（验收后）`docs(button-press): 完工回写`。

## 7. 验收标准（实机）

1. 悬浮窗行1任意按钮（完整/清除/设置/退出…）：快速单击 + **长按 ≥1s**，
   按钮文字**始终零位移**（按下、保持、松开全阶段）。
2. 按压可感知反馈=**底色加深**（悬停浅灰→按压更深一档），描边/圆角不变。
3. 面板/日志窗/组合框/页签条点击行为与位移前后无差异（回归）。
4. 深色日志窗/向导/基准窗按钮 hover 出现 1.0px 描边（预期增强，文字不动）。
5. 下压-松开-点击回弹节奏正常；`cargo test --workspace` 全绿。

## 8. 偏差登记（决策史）

- **D-32（新增）**：悬浮窗行1按钮按压时文字右移 1.0 逻辑 px（200% DPI ≈ 2 物理 px）。
  根因 = egui 0.36 `button_style` 内边距公式 `button_padding − bg_stroke.width` ×
  `stabilize_widget_strokes` 以 inactive（dark 默认 0 宽）为基准 × overlay_ui
  原位覆盖未含 active 态。修复 = 三态描边宽度全等（BP-1+BP-2）+ 按压反馈颜色化。
  行为差异：深色窗按钮 hover/active 由「无描边」变「1.0px 描边」；按压反馈
  只变色不变形（与原版「仅底色变化」的 QSS 语境一致，阶段二以产品体验为准）。

## 9. 测试矩阵

| 用例 | 归属 | 现状 |
| --- | --- | --- |
| 三态文字坐标全等（overlay 注入链, 含 press 序列） | widget_stability.rs | 新增 BP-3 |
| stabilize 基准 ≥1.0 且三态互等 | widget_stability.rs | 新增 BP-3 |
| hover 三态钉（既有 2 用例 + panel uniform 2 用例） | widget_stability.rs | 保持 |
| 面板 8 页渲染冒烟 | panel/mod.rs | 保持 |
| 全量 | -- | 约 380 测（现 378 + 2） |

## 10. 完工注记（2026-09-08，4054f4b）

BP-1~BP-3 全部落地，**380 测全绿**（净增 2，ignored 5 不变）+ 新码 clippy
零告警 + release 冒烟零错：

- BP-1：`overlay.rs` 三态覆盖补齐 active（`BTN_PRESSED_FILL` 按压底色、
  描边 1.0 同款、圆角继承），注释落 D-32 成因；按压反馈=仅色变。
- BP-2：`style.rs` 基准下限修正为三态**一并**抬到 `max(1.0)`——施工中发现
  「只抬 hovered/active」会让 dark 窗 inactive→hover 回归 1px 位移
  （旧 WP-B 钉的故障重演），inactive 描边色透明故外观不变、几何才真正全等；
  深色窗按钮 hover/active 1.0px 描边为预期增强。
- BP-3：`widget_stability.rs` 新增
  `overlay_button_text_does_not_shift_when_pressed`（idle/hover/按压连跑 3 帧
  文字 rect 全等）与 `stabilize_reaches_one_px_floor`（dark/panel 双输入）；
  既有 `plain_dark_theme_is_stabilized` 断言经新语义验证仍通过。
- 程序化验收（§7 第 1~4 条）：按压零位移已由回归钉常量断言；
  §7 第 5 条（实机长按 ≥1s 无位移 + 按压底色加深观感）**待用户实机截图确认**。

## 附录 A：探针证物（TS-3，`press_probe.rs` 临时文件）

关键点：`ctx.run_ui` + `RawInput{events: PointerMoved + PointerButton(pressed)}`；
视觉复刻链：

```rust
let mut v = if panel { panel 同款 } else { egui::Visuals::dark() };
// host 链：以 inactive 为基准拉齐（BP-2 前行为）
let w = v.widgets.inactive.bg_stroke.width;          // dark → 0.0
v.widgets.hovered.bg_stroke.width = w; v.widgets.active.bg_stroke.width = w;
...
// overlay_ui 链：仅 inactive/hovered 覆盖为 1.0（active 保持 0.0）
vis.widgets.inactive.bg_stroke = Stroke::new(1.0, …);
vis.widgets.hovered.bg_stroke   = Stroke::new(1.0, …);
ui.spacing_mut().button_padding = Vec2::new(6.0, 0.0);
…
// 探针按钮 = small_btn 同款：size(11) + fill/stroke/corner 3 + min_size(0,20)
```

输出（grep `#[test]` 两用例）：

```
panel:   idle=[4.0 3.5] hover=[4.0 3.5] press1=[4.0 3.5] press2=[4.0 3.5] press3=[4.0 3.5]
overlay: idle=[6.0 3.5] hover=[6.0 3.5] press1=[7.0 3.5] press2=[7.0 3.5] press3=[7.0 3.5]
```

（TS-2 真实 panel_ui 探针：日志工具行 [445.0 47.0] / 引擎下拉 [67.9 69.5] /
页签条 [98.6 10.0] 三态全恒定。）
