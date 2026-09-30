# 面板列表行悬停跳变根治（D-124）

> **状态**：定稿（2026-09-30）｜**触发**：用户截图报案——翻译页模型行悬停出蓝框时框内文字微移、下方内容整体被挤下，且「所有类似的蓝框都有这种问题」｜**前缀**：本档局部号 F-x（施工卡）｜**流程**：grilling 于报案当轮以四项拍板表闭合，用户明示授权按推荐项自主走完 Matt 流程全链（裁决留痕 §7）。

## 1. 背景与病根（实证）

**病根 = G-38 同族病灶在面板列表行上的复发**。egui 0.36 `Button::selectable(false)`（非选中态）两个分支占位不等：

- 静止（Inactive）：`frame_when_inactive=false` → 走**无帧分支**（`Frame::new()`，stroke=0 不画），但内边距公式仍按 `button_padding + expansion − inactive.bg_stroke.width` **预扣**；
- 悬停（Hovered）：切**全帧分支**，stroke=hovered.bg_stroke 回补。

`Frame::total_margin() = inner_margin + stroke.width + outer_margin`（egui containers/frame.rs:327，stroke 计入占位）→ 静止总占位 = `padding − w_inactive`，悬停 = `padding − w_inactive + w_hovered`。`panel_visuals` 两态宽均 1.0（panel/mod.rs）→ **悬停每边多 1px：行高 +2px、框内文字下移 1px**。

**headless 探针定量留痕**（报案当轮，自带 harness 复刻 `app.rs run_frame` visuals 注入，翻译页模型行悬停 3 帧后取形）：行内文字 y 116.0→117.0（Δ+1.0）；下方「添加」按钮 y 153.0→155.0（Δ+2.0）；预验修复（scope 内归零 `w_inactive`）后 Δ 全零。两数字与用户截图逐像素吻合。

**既有两道防线为何没拦住**：

1. `stabilize_widget_strokes`（WP-B/D-32）只拉齐三态描边**宽度**——宽度相等时占位差恰 = `w_hovered` 本身，消不掉「静止不画/悬停画」的**分支差**；它护住的是恒全帧的普通按钮。
2. `selectable_stable`（D-119/G-38）修的是同一公式，但修复面只包 ComboBox 弹层项，且明令「禁据此全局归零 `inactive.bg_stroke`」——面板普通按钮 idle 灰边框依赖该值。

**受影响面**（全仓 `Button::selectable` 恰 4 处）：翻译页模型行（translation.rs）/ 字幕页文字行列表（subtitle_page.rs）/ 数据页缓存行（data.rs）三处 D-123 列表行悬停中招；字幕行编辑器灰显占位（disabled → Noninteractive 态无悬停分支，恒全帧）不中招。弹层项已被 D-119 修；普通按钮已被 WP-B 修。**同根隐性症状**：选中行恒全帧 → 点选瞬间行 +2px、取消回缩（与悬停共用同一机制，本包一并消掉——修复后三列表全行等高）。

**测试台盲区**：`click_testing::PanelHarness` 直接调 `windows::dispatch`，未注入真机 visuals，跑在 egui 默认 visuals（`w_inactive=0`）下两态恰好相等——D-123 全套 headless 测试对此类病整体失明（D-119 弹层测试是自带 harness 显式 `set_visuals(panel_visuals())` 才测得出的）。

## 2. 裁决（2026-09-30，四项全按推荐执行，用户授权留痕 §7）

| # | 问题 | 裁决 | 理由与代价 |
|---|---|---|---|
| 1 | 修法 | 三列表行加共享助手 `selectable_button_stable(ui, selected, button)`：**未选中**行在 scope 内归零 `w_inactive` 再 `ui.add(Button::selectable)`，选中行原样直加 | D-119 同款先例；悬停蓝框/选中灰边/底色反馈零视觉变化；备选「三处就地 scope」防不了第 4 处复发、「恒透明描边全帧」覆盖三态反馈，均弃 |
| 2 | 测试台 | `PanelHarness::render` 补真机 visuals 注入（`panel_visuals` + stabilize + 面板滚动条样式，对齐 `app.rs run_frame`） | 除既有一页 e2e 外全 D-123 测试台受益，同类病不再失明；既有用例文本动态查找预期无碍（全量跑验证） |
| 3 | 静止布局 | 接受未选中行静止高度比现状高 2px（一次性静态变化） | 换来所有行等高、悬停/选中/取消零跳动；静态 +2px/行 对肉眼不可辨 |
| 4 | 入册 | 新坑记入坑册（五段式，编号随收口取表尾+1）+ D-124 登记台账 | 同类病第三次排查（G-38→D-119→本案）该有触发词索引了 |

## 3. 行为规格（canonical）

1. **几何不变量**：列表行在 静止/悬停/按下 三态外框占位全等；选中行与其余行等高。悬停前后：行内文字坐标不动、下方一切内容坐标不动。
2. **视觉契约**：悬停反馈 = 底色（hover）+ 描边色（accent 蓝）出现，仅此二事；选中行静止 = selection 底 + 灰边框（`inactive.bg_stroke` 色）+ 选中字色，现状不变。
3. **作用面**：仅非选中行需要 scope 归零（选中行走恒全帧分支，归零反而会抹掉其静止灰边框）；disabled 占位（Noninteractive）无此病，不经助手。
4. **机械防线**：`egui::Button::selectable` 入 clippy `disallowed-methods`，唯一豁免 = 助手本体（与 G-38 禁 `selectable_label` 同款口径）——新增列表行想绕开助手直接编译失败。

## 4. 施工卡

- **F1 助手**：`style.rs` 新增 `selectable_button_stable`（紧邻 `selectable_stable`，G-38 家族聚居；`#[allow(clippy::disallowed_methods)]` + 理由注释 = 禁令唯一豁免出口）。
- **F2 接线**：三列表行调用点改走助手（翻译模型行 / 字幕文字行 / 缓存行）；字幕行编辑器灰显占位就地 `#[allow]` + 理由（Noninteractive 无悬停分支，病不达）。
- **F3 机械防线**：根 `clippy.toml` 增 `egui::Button::selectable` 条目（理由注明 D-124 与豁免位置）。
- **F4 测试台补盲**：`PanelHarness::render` 注入真机 visuals 三件套；头注补记盲区教训（与 G-40 时钟教训并列）。
- **F5 回归测试**（先红后绿，红 = 病灶现行）：
  - 助手缝（style.rs tests，仿 `popup_item_rect` 探针法）：panel visuals 下 助手包出的未选中行 静止/悬停 rect 全等；选中行四态 rect 全等。
  - 页面缝（translation.rs tests，`PanelHarness`）：悬停模型行 3 帧后，行内文字与下方「添加」按钮坐标全等（报案症状 1:1 回归）。

## 5. 验收基线

`cargo test --workspace` 全绿（基线 671+0 滚动上浮）；clippy 0；precommit 三项过；新增测试先红（病灶复现数字留痕 §1）后绿；零契约改动（lt-proto / PROTO_VERSION 不动）；零 i18n 改动。

## 6. 范围外

- 弹层项（D-119 已修）、普通按钮（WP-B 已稳）、悬浮窗任何列表（无 `Button::selectable`）。
- 悬停蓝框/选中态的**颜色与样式**重设计（本包只钉几何，不动美术）。
- egui 上游修复（0.36 分支行为属上游设计取舍——「禁据此全局归零」的边界不变）。
- 近四包定稿文档的归档波次（D-120/122/123 等仍滞留 docs 顶层，另行批量处理）。

## 7. 裁决留痕

2026-09-30 报案当轮：诊断完成后出四项拍板表（修法/测试台/静止布局/入册），用户回复「请你自己按照 Matt pocock skill 的流程严肃做完一整套修复」= 四项按推荐执行 + 开工令 + 全流程自主授权。无推翻、无新政策。
