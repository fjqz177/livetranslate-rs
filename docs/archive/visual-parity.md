# GUI 视觉质感与手感对齐改造方案（Visual Parity Plan）

> 【已归档】阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档。2026-09-07 起本版不再追求与原版 1:1，Python 原版仅作行为参考；本文仅作决策史，不再作为施工依据（活跃文档见 AGENTS.md 与 docs/README.md）。
> 五工作包已全部施工完成（285 测），见文首状态行。


> **状态：已全部施工完成（2026-09-07）。** 五工作包对应提交：WP-A a883ef7、
> WP-B 448ee7c、WP-C 0f77cbb、WP-D 509150f（尺寸+changelog 修复）、
> 5851e60（面板居中）；285 测全绿，实机走查通过。施工中新增修复：changelog
> 条目 horizontal→horizontal_wrapped（横向溢出）、删除嵌套 ScrollArea（宽度
> 错位）、面板打开主屏偏上居中，详见对应 commit。
>
> 依据：2026-09-07 实机走查 + 双方源码交叉验证 + egui 0.36.1 无头渲染实验。
> 用户反馈三个问题：① 悬浮窗纯黑不透明 + 上半部多了黑紫色矩形框；② 按钮 hover 文字微移；
> ③ 滚动条卡顿不跟手/样式差/设置页多余滚动条。
> 本方案只做研究结论与施工计划，未动任何功能代码。

---

## 0. 结论（TL;DR）

三个问题全部找到**机制级根因**，且多数是"Rust 忠实实现了原版的**意图**，而原版因 Qt 行为实际**从未渲染**"这类双方行为差异，不是简单没对齐：

| # | 用户观感 | 根因 | 性质 |
|---|---|---|---|
| 1 | 头部多了黑紫色矩形框 | Rust 渲染了 `header_color #1a1a2e` 头部色块；**原版从未渲染过它**（Qt QWidget 子类 QSS 背景不生效的坑） | 双方差异，对齐原版实际渲染 = 删除绘制 |
| 1b | 纯黑而非半透明 | 两版 default 预设合成透明度其实**等价**（≈89% 不透明黑，Rust 的 wgpu alpha 链路经像素分析确认生效）；"死黑感"主要来自头部色块破坏了"黑玻璃一体感" | 修复 #1 后观感即恢复 |
| 2 | 按钮 hover 文字微移 | egui 0.36 按钮内边距公式 `padding − bg_stroke.width`：dark 默认三态 stroke 宽度不一致（inactive 0 / hovered 1）→ hover 内容区扩 1px 文字移 0.5px；且 `fg_stroke` 1.0→1.5 hover 变粗。悬浮窗/面板已覆写不受影响，**悬浮窗 3 个下拉、向导/日志/基准窗仍走默认值** | egui 机制，统一覆写可消除 |
| 3 | 滚动条卡顿不跟手 | egui 默认 floating 滚动条：静止 2px 细条难抓 + hover 展开 fade 动画 + **出现时内容区 6px 跳变 reflow**（`floating_allocated_width=6`）+ 滚轮 40 逻辑px/格无平滑 | egui 默认样式问题，定制 ScrollStyle |
| 3b | 滚动条样式丑 | 面板用 egui 默认灰条；原版面板是 Windows 原生 17px 系统滚动条、悬浮窗是 6px 白色半透明圆角条 | 样式定制 |
| 3c | 设置页差一点显示完却出滚动条 | 面板窗口 520×650，但**原版实机窗口是 ≈535×781**（Qt 布局 minimumSize 把 resize(520,650) 顶开，走查实测 802×1171 物理@150%）；Rust 内容按 650 放不下 → 滚动条 | 窗口尺寸没对齐原版实机值 |

---

## 1. 证据与根因分析

### 1.1 黑紫色矩形框（P0，观感主凶）

**原版链路**（`LiveTranslate/subtitle_overlay.py`）：
- `DragHandle(QWidget)` 子类：`__init__` 里 `setStyleSheet("background: rgba(60,60,80,200); border-radius:4px;")`（L619）
- `OverlayWindow.apply_style()`（L1186-1203）：`self._handle.setStyleSheet(f"background: {hdr_rgba}; border-radius:4px;")`
- **但**：Qt 规则——QWidget 子类的 QSS `background` 不渲染，除非 `setAttribute(WA_StyledBackground)` 或重写 paintEvent 画 QStyleOption。`grep paintEvent subtitle_overlay.py` = **零结果**。
- 同文件反证：`_container = QWidget()` **直接实例**（非子类）背景生效（半透明黑容器可见）；`DragHandle`/`ChatMessage`/`MonitorBar` 等**子类**全部不渲染背景（它们 setStyleSheet 透明背景"想要"的效果恰好都是透明，作者从未发现）。
- **实机截图证实**：原版头部按钮行/复选行/下拉行背后是容器黑玻璃贯穿，无任何独立色块。

**Rust 链路**（`crates/lt-ui/src/windows/overlay.rs:111-129`）：`drag_handle()` 用 `fade(header_color, header_opacity, window_opacity)` 画了 `Frame.fill` —— `#1a1a2e` × 230/255 × 95% ≈ 亮度 45 的深蓝紫色块，叠在亮度 ≈4 的黑玻璃底上，形成用户所说的"黑紫色矩形框"。

**修复决策**：对齐**原版实际渲染**——悬浮窗不绘制头部色块。`header_color`/`header_opacity` 设置字段与样式页控件**保留**（原版同样保留且无效，这是 1:1），代码注释注明"原版 Qt 行为：QSS 背景对 QWidget 子类不生效，字段存在但从不渲染"。

### 1.2 "纯黑而不是半透明"

- Rust 透明链路完整：`Painter::new(..., support_transparent_backbuffer=true)`（app.rs:63）+ 透明窗 clear `[0,0,0,0]`（app.rs:1274）+ `fade()` 双级预乘 alpha（overlay.rs:30）。
- 实测像素：悬浮窗覆盖区亮度分布 p5=22/p95=169，存在非零透出（纯黑失效应为恒 0），合成行为符合 89% 黑叠加本地桌面内容的预期。
- 原版同参数（`DEFAULT_STYLE` bg #000000×240 + window_opacity 95）经 Qt 合成同样 ≈89% 黑（`WA_TranslucentBackground` + QSS rgba + `setWindowOpacity(0.95)` 叠乘）。
- **两版差距不在透明度数值，而在头部色块把"整体半透明黑玻璃"切成了"死黑窗体 + 一块紫斑"**。执行 1.1 后该观感即消除。
- 若需进一步向"通透"调（超出对齐范畴），属用户偏好决策，见 §4 决策点 D2。

### 1.3 按钮 hover 文字微移（P1，手感）

**egui 0.36.1 机制**（`egui-0.36.1/src/widget_style.rs:146-168`）：
```rust
// button_style():
inner_margin = spacing.button_padding + Vec2::splat(expansion) − Vec2::splat(bg_stroke.width)
```
dark 默认三态（`style.rs:1680-1725`）：
- `inactive.bg_stroke = Stroke::default()`（宽 **0**）→ margin = padding
- `hovered.bg_stroke = Stroke::new(1.0, …)`（宽 **1**）→ margin = padding − 1 → **内容区每边扩 1px，居中文字位移 0.5px**
- `hovered.fg_stroke` 宽 1.0→**1.5**、色 gray180→gray240 → **hover 时按钮文字同时变粗变亮**（150% DPI 下感知强烈，即"文字会动"）

**项目现状**（无头渲染实验实测 hover 前后 galley rect：dx=0, dy=0）：
- 悬浮窗 7 按钮（overlay.rs:94-98 覆写 inactive/hovered bg_stroke=1.0）✓ 不移
- 控制面板按钮（panel_visuals 三态全覆写）✓ 不移
- **未覆写、仍受影响的**：
  - 悬浮窗 3 个下拉（ComboBox selected_text 走 dark 默认 fg_stroke → hover 变粗变亮）
  - Setup 向导窗、Log 日志窗、Benchmark 工具窗、Subtitle 字幕窗（app.rs:241 `_ => Visuals::dark()` 整窗默认 → 真位移 + 变粗）

**修复**：抽公共函数统一"三态 stroke 宽度一致 + fg_stroke 恒宽 1.0"，全部窗口注入；hover 反馈靠底色变化表达（原版 QSS hover 也只变底色/字色，不变粗）。

### 1.4 滚动条（P1，手感+样式+布局）

**egui 0.36 默认**（`style.rs:584-621`，`ScrollStyle::default() = floating()`）：
- 静止时 2px 细条（`floating_width=2`），hover ScrollArea 才展开到 10px（`bar_width=10`），带 fade 过渡 → **hitbox 窄、抓取难、视觉漂移**
- `floating_allocated_width=6` → `VisibleWhenNeeded` 下滚动条出现/消失时**内容宽度 6px 跳变 reflow**（"卡一下"的实锤来源）
- 滚轮：`InputOptions.line_scroll_speed=40`（native 默认）逻辑 px/格，无平滑/惯性（Qt 亦无，但 Qt 步进观感更稳）
- 事件链已核实：`CursorMoved → repaint=true`（egui-winit lib.rs:352-359），拖动期间每帧重绘 + Fifo vsync，渲染层无掉帧机制；感知的"掉帧"主要是 6px reflow + 细条动画的叠加观感

**原版样式基准**：
- 悬浮窗消息区 QSS：6px 宽、handle `rgba(255,255,255,60)` 圆角 3、透明槽（subtitle_overlay.py:989-1002）
- 控制面板：无 QScrollArea（grep 无结果）→ **原版面板页面不滚动**，滚动条是 Windows 原生 17px 系统样式（QTabWidget 页内直接布局）

**面板多余滚动条**：
- 原版 `resize(520, min(650, avail))`（control_panel.py:100）只是请求值；Qt 布局 minimumSizeHint 把窗口顶开 → **实机 ≈535×781 逻辑**（ui-realign.md §2.1 走查实测 802×1171 物理@150%）
- Rust 版 `create_window(Panel, (520,650))`（app.rs:105）按 650 固定，内容密度又略高于原版 → 内容溢出几十像素 → 滚动条出现（"差一点就能显示完"与 781−650=131px 的量级吻合）

---

## 2. 施工计划（五个工作包，顺序执行，每包独立 commit）

### WP-A 悬浮窗"黑玻璃一体"对齐（P0）

文件：`crates/lt-ui/src/windows/overlay.rs`

1. `drag_handle()` 移除 `Frame::fill(header_fill)`，保留布局 margin（头部区域与容器一体透出黑玻璃）。
2. `header_color`/`header_opacity` 字段、样式页控件、预设覆写全部**保留不动**（对齐原版"字段存在但渲染无效"的 Qt 行为）；在字段定义与样式页各加一行注释说明缘由。
3. 复查头部圆角：原版 DragHandle QSS `border-radius:4px` 同样未渲染 → 头部无独立圆角 ✓（删除 fill 后自然一致）。
4. 顺带核对：按钮 rgba 白 8%/16% 底、描边 rgba 白 40/80、消息区空态纯黑等已有项不受影响。

**验收**：① 两版悬浮窗同屏全屏截图，头部区域均无独立色块；② 悬浮窗置于纯白窗口上，合成透出度像素采样与原版差 ≤5%（用 PowerShell CopyFromScreen，注意截图坐标=物理/2）；③ `cargo test --workspace` 全绿。

### WP-B 控件三态样式收敛——消灭文字微移（P1）

文件：`crates/lt-ui/src/style.rs`（新增共享函数）、`app.rs`、`windows/{setup,logwin,bench,subtitle,overlay}.rs`

1. 新增 `pub fn stabilize_widget_strokes(v: &mut egui::Visuals)`：三态（inactive/hovered/active）`bg_stroke` 宽度统一为同一值（建议 1.0）、`fg_stroke` 宽度恒 1.0、`corner_radius` 三态一致；hover 区分度交给 `weak_bg_fill`。
2. 注入点：`app.rs run_frame()` 的 per-viewport `set_visuals` 处，对 `_ => Visuals::dark()` 分支与 overlay 一并应用（面板 `panel_visuals()` 内部同样调用收敛，幂等）。
3. 悬浮窗下拉 `selected_text` 显式 `RichText::color(BTN_TEXT)`，hover 态字色变化交给 visuals（对齐 `_COMBO_CSS`：color #aaa / hover #ddd）。
4. 面板 `panel_visuals()` 复查 `expansion` 恒 0（egui 0.36 默认就是 0，加断言防回归）。

**验收**：① 新增无头测试：Button 与 ComboBox 在指针移入/移出两帧下 galley rect 完全相等（本轮已验证方法可行，扩到全部窗型）；② 实机将光标在悬浮窗按钮行与下拉间来回移动，录屏/连拍确认文字稳定；③ 日志窗/向导窗按钮 hover 不再位移变粗。

### WP-C 滚动条样式与手感（P1）

文件：`crates/lt-ui/src/app.rs`（per-viewport ScrollStyle 注入）、`windows/panel/mod.rs`、`windows/overlay.rs`

1. **面板 = Windows 原生风 solid 滚动条**：`ScrollStyle::solid()` 基底定制——`bar_width≈12`、槽色 #F0F0F0、柄 #C1C1C1（hover #A6A6A6、active #606060）、`corner_radius=2`、`bar_inner_margin=2`；solid 恒占位 → **彻底消除 6px reflow**。
2. **悬浮窗 = 原版 QSS 风 floating 条**：`ScrollStyle::thin()` 基底定制——`bar_width=10`、`floating_width=4`、handle `rgba(255,255,255,~90)` 圆角 3、透明槽；对齐原版"6px 白色半透明圆角"观感（浮动式，出现时 6px reflow 在内容长度跨越阈值才发生一次，可接受）。
3. 注入方式：per-viewport 在 `set_visuals` 同处 `ctx.all_styles_mut(|s| s.scroll = …)`（或 `style_mut`），Panel/Overlay 分别取各自配置；其余深色窗用统一 dark 滚动条（同样收敛宽度）。
4. 滚轮手感：`ctx.options_mut().line_scroll_speed` 40→**50**（略贴近 Qt 一步 3 行 ≈39px 但留余量；不改也能接受，列为微调项）。
5. 已知偏差记录：egui 无滚动平滑动画，Qt 亦无；此项不做"假平滑"自创行为。

**验收**：① 面板滚动条截图像素级对照 Windows 原生样式（宽度/槽/柄/圆角）；② 悬浮窗滚动条对照原版 QSS 效果；③ 面板内拖动滚动条上下滚动连拍，内容无横向跳变；④ 滚轮连续滚动无突兀顿挫。

### WP-D 设置面板尺寸与密度（P0，"差一点出滚动条"）

文件：`crates/lt-ui/src/app.rs`、`windows/panel/*`

1. 面板创建尺寸 `(520,650)` → **`(535,781)`**（原版实机值，来源：走查实测 802×1171 物理@150%；min 480×420 保持）。
2. 小屏兜底：`min(avail_workarea_h, 781)`（winit 无 work-area API，沿用现有 48px 任务栏估高法；最低不低于 650，650 以下才可能出现滚动条——与原版行为一致）。
3. 逐页密度微调（若 781 高下仍有页溢出）：`form_row` 行高 22→20、页边距 12→10、`group_card` 前间距 12→10（从 VAD/ASR 页开始逐页核，改动最小化——先量再动）。
4. 页面滚动策略对齐原版：7 页中仅更新日志页（QTextBrowser 自滚）保留 `ScrollArea`，其余页在 781 高下**不应出现滚动条**。
5. `panel.apply` 高度变化后 geometry 保存链路复查（面板不存几何，无影响）。

**验收**：① zh/en 双语 7 Tab 逐页全屏截图，除更新日志外无滚动条；② 窗口拖到 781 高度上下限外不破版；③ 与原版（或走查实测尺寸记录）并排结构一致。

### WP-E 回归与验收基建

1. `cargo test --workspace` 全绿（当前 281 测）+ WP-B 新增无头断言。
2. 实机走查：zh/en 各一遍，全屏截图（egui 窗口 PrintWindow 抓旧帧的坑照旧规避）。
3. 像素采样脚本沉淀（DPI 坐标换算坑记入：CUA 截图坐标=物理/2，PowerShell 非 DPI-aware 进程 SetCursorPos 用物理/1.5 虚拟坐标，本轮踩过）。
4. 更新 `rewrite-research.md` 已知偏差表（新增 D-17：header 样式字段因原版 Qt 行为不渲染，1:1 保留字段）与记忆。

---

## 3. 依赖与顺序

WP-A、WP-B 互不依赖可并行（不同文件域）；WP-C 依赖 WP-B 的注入点骨架；WP-D 独立但建议在 WP-C 后（密度微调需在滚动条样式落定后测量）；WP-E 收尾。

## 4. 决策点（需用户确认或默认按推荐执行）

| # | 决策 | 推荐 |
|---|---|---|
| D1 | 头部色块：删除绘制且字段保留无效（对齐原版 Qt 行为）vs 字段生效但默认透明（对齐原版"意图"） | **推荐前者**：用户观感以原版实机为准；字段保留保持样式页 1:1 |
| D2 | default 透明度：维持两版等价的 89% 黑 vs 额外调透（如 bg_opacity 240→200） | **推荐维持**：1:1 原则；用户若要更透可自行切换 transparent 预设 |
| D3 | 面板滚动条风格：Windows 原生粗条（≈17px 系统感）vs 现代细圆角条（10-12px） | **推荐 12px 圆角柄**：对齐"Windows 原生观感"的同时不显笨重；若用户偏好系统感可调 14px |
| D4 | 滚轮步进：保持 40 vs 调 50 | **推荐保持 40**，先修 reflow 与样式，滚轮微调留待实测反馈 |

## 5. 风险

| 风险 | 缓解 |
|---|---|
| egui 0.36 `ScrollStyle::solid` 恒占位改变面板可用宽度，表单换行点变化 | WP-C 后逐页截图复查；form_row 标签宽 130px 上限有自适应兜底 |
| fg_stroke 恒宽后 hover 反馈弱化 | hover 底色差保持（面板 #E1E1E1→#E5F1FB 明显；悬浮窗 8%→16% 白叠加）|
| 781 高度在 1200p 以下屏超出工作区 | WP-D.2 小屏兜底 clamp |
| 删 header fill 影响穿透分界计算（header_px 以 cursor.top() 记） | 仅删 fill 不动布局，header_px 逻辑不变；穿透实测覆盖 |
| egui 升级后 margin 公式变化使 WP-B 失效 | 无头断言测试常驻，回归即红 |

## 6. 附：本轮证据索引

| 项 | 内容 |
|---|---|
| 实机截图 | Rust 悬浮窗（头部黑紫框+纯黑）、原版悬浮窗（无头部色块、紧凑 200 高）、Rust 面板 VAD/ASR 页（底部溢出滚动条）|
| 像素分析 | Rust 悬浮窗覆盖区 p5=22/p95=169 → 透明链路生效；原版 user_settings 几何 48,339,620×200 |
| 无头实验 | egui 0.36.1 hover 前后 Button galley rect dx=dy=0（覆写样式下）；margin 公式源码定位 widget_style.rs:161-166 |
| 原版代码 | DragHandle QSS 未渲染机制（无 paintEvent/无 WA_StyledBackground）；apply_style 调用链 main.py:287/1884；面板 resize(520,650) vs 实机 781 |
| egui 源码 | ScrollStyle::floating 默认（style.rs:584）；floating_allocated_width=6；line_scroll_speed=40（input_state/mod.rs:106-112）；CursorMoved→repaint（egui-winit lib.rs:352）|

## 7. D-17 字体基线复核项（2026-09-07 追加）

字体系统改造（docs/archive/font-system.md，D-17）后全默认渲染改内嵌思源黑体（Noto Sans CJK SC），
与原版微软雅黑**文字度量不同**（字号/换行断点/行高观感），以下既有验收量需按新基线重走：

- 面板 535×781 与行控件换行位置（固定尺寸不变，仅文字宽度微差）；
- 悬浮窗/字幕窗行宽与换行阈值（subtitle.rs wrap_greedy 以实际 FontId 度量，自动一致）；
- 样式页预览卡与悬浮窗实测色值不受影响（仅字形变化）；
- 若用户显式选回"微软雅黑"（系统扫描列表），该机器渲染回到原版度量（parity 可局部复现）。
