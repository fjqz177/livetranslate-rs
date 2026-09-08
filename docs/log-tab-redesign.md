# 设置面板「日志」Tab 界面改造计划

> 2026-09-08 用户反馈（附实机截图）：① 顶部工具行（显示 DEBUG / 自动滚动 / 清除 /
> 复制全部 / 打开日志文件）应在本 tab 内置顶固定，不随日志滚动；② 最右侧淡蓝色
> 滚动条不应存在，只应保留日志列表自己那根滚动条；③ 总诉求：**简洁、方便、逻辑
> 清晰、无让用户不爽的机制**。本文给出证据链、根因、目标设计与改造方案。
> 施工卡编号 LT-1~LT-6；行为差异落档 **D-31**（自 D-31 起新编号，见
> docs/sensevoice-language-fix.md 收尾注记）。

> **✅ 已完工（2026-09-08，416bbfd）**：LT-1~LT-6 全部落地——378 测全绿（净增 8，
> 无新 clippy 告警）+ release 单 exe 构建过 + 实机对照用户截图（2026-09-08 用户
> 亲自截取）：最右淡蓝外层滚动条消失、工具行恒置顶、按钮改名「打开日志目录」、
> 日志实时流至截图时刻（LT-5 生效）；贴底跟随/DEBUG 回溯/浮钮由单测
> （advance_follow×3、level_visible 矩阵、无尾随内容回归、渲染冒烟）与 egui
> `stick_to_bottom` 官方机制保障。交互项（上翻挂起浮钮/勾选回溯/复制反馈）建议
> 用户顺手再走一遍实机清单 §5.2 的 3~5 步。

## 1. 现象（截图对照）

- 主視覚：设置窗口固定尺寸约 535×781（`lt-ui/src/app.rs:114-128`），Tab 条 8 页，
  「日志」页末位（Rust 版**新增**视图，原版无此 tab——原版日志是独立窗口
  `log_window.py`，面板只有 7 tab，见 `panel/mod.rs` 注释与
  `tab_order_matches_original` 测试）。
- 用户框出的问题（截图中可辨）：
  1. 页面最右缘有一条**全高、极细的淡蓝色竖条**——这不是日志列表的滚动条，
     而是**页面级外层滚动条**；
  2. 工具行在日志列表上方，但**属于外层滚动内容**——一旦内容超高、外层滚动条
     被拖动，工具行会跟着滚走，永远无法"置顶"；
  3. 日志列表自己那根滚动条（日志区右侧）才是"日志打印查看用的滚动条"。
- 用户诉求归纳：**只有一个滚动条（日志列表的）；工具行恒置顶**；顺带排查其他
  "用户不爽机制"（见 §2.4~§2.7，均有代码证据）。

## 2. 现状与证据链（逐环核实）

### 2.1 渲染层级：两层 ScrollArea 嵌套

`lt-ui/src/windows/panel/mod.rs:137-173`（`panel_ui`）对**所有** tab 内容统一套一层
页面级 ScrollArea（对标原版 Qt `make_scroll_area`，注释"内容按需滚动"）：

```
ui（面板 pane 区域，pane ≈ 781 - Tab条 - 边距）
└─ ScrollArea::vertical()            ← 外层（mod.rs:155-158）
   │    .auto_shrink([false,false])   ← 占满视图高度
   │    .scroll_bar_visibility(VisibleWhenNeeded)  ← 内容超高即出现
   └─ Frame::inner_margin(12)        ← mod.rs:159-160
      └─ log_tab::page()             ← mod.rs:169
         ├─ 工具行 horizontal         ← log_tab.rs:35-71（随外层滚动）
         ├─ Frame(NONE).inner_margin(4)
         │  └─ ScrollArea::vertical() ← 内层（log_tab.rs:90，日志列表）
         │      .auto_shrink([false,false])
         └─ 底部"当前会话日志："提示行  ← log_tab.rs:110-118
      └─ ui.add_space(12.0)          ← mod.rs:171
```

- 外层滚动条 = 截图最右淡蓝**细条**（浅色 visuals 默认滚动条样式，贴 pane 右缘、
  竖直占满、几乎无轨道宽度——与截图完全吻合）；
- 内层滚动条 = 日志列表自己的（log_tab.rs:90-107）。

### 2.2 根因一：内容高度恒溢出 → 外层滚动条必然出现

高度账（近似、以实机 781 高面板为基准）：

| 组成 | 高度 |
|---|---|
| pane 区域 | H ≈ 781 − Tab条(4+26) − 面板窗框 ≈ 730 |
| 外层 Frame margin 上下 | 24（预留，不占内容高） |
| 工具行 | ~22 |
| 内层 ScrollArea（`auto_shrink([false,false])` → 占满剩余可用高度） | H − 24 − 22 ≈ H − 46 |
| 底部提示行（`add_space(4)` + 10.5pt label，**路径长时 label 还会换行加高**） | ~19+ |
| 外层收尾 `add_space(12)` | 12 |

外层内容总高 = 24 + 22 + (H−46) + 19 + 12 = **H + 31 > H** → 外层 ScrollArea
`VisibleWhenNeeded` 触发，淡蓝滚动条出现。**触发增量 = 底部提示行 19px**——它
是全页唯一超出的来源（其他 7 页内容不满一屏，无此问题；截图先兆：该提示行在
部分日志行与列表间把内容顶出）。

> 注：日志区为什么"占满剩余"？`auto_shrink([false,false])`（log_tab.rs:91）使内层
> 尺寸 = 可用空间（而非收缩到内容）。这是"日志区天然满高"的正确形态，保留；
> 需要控制的是**日志区之后不允许再有追加高度**。

### 2.3 根因二：工具行不是"置顶元素"，是外层滚动内容

工具行位于 `log_tab::page()` 首部（log_tab.rs:35-71），而 `page()` 整体被外层
ScrollArea 包裹（mod.rs:161-170）。外层滚动的**含义**是把整页（含工具行）滚走。
用户感受到的"工具行不置顶"= 外层滚动条被拖动/滚轮悬停在外层区域时整块内容位移。
日志 tab 的正确语义是：**工具行固定，仅日志列表滚动**——需要把日志 tab 从外层
ScrollArea 中拔出。

### 2.4 根因三：自动滚动 = 每帧强制拽底（用户上翻即被拉回，原版同款）

- `log_tab.rs:104-106`：`if auto_scroll { ui.scroll_to_cursor(BOTTOM) }`——每帧执行，
  用户一旦用滚轮上翻查看历史，下一帧立即被拽回底部；
- 同款于 `logwin.rs:71-73`（独立日志窗）与 Python 原版 `log_window.py:103`
  （`if self._auto_scroll.isChecked(): …setValue(max)`，1:1 复刻至今）。
- 产品判断：这是**经典"用户不爽机制"**——现代日志查看器（VS Code 输出面板、
  系统事件查看器）的通行语义是"**贴底跟随**"：视图贴近底部（小容差）时才跟随；
  用户上翻后立即挂起跟随（rollback 仅当用户滚回底部恢复），并给出"回到最新"
  快捷入口。阶段二以产品体验为准（AGENTS：原版有/没有不再是取舍依据），本条
  改造不背复刻包袱。

### 2.5 根因四：显示 DEBUG 不回溯（勾了也看不到历史，原版同款）

- `state.rs:208-212`（`LogWindowState::push`）：`level < INFO 且未开 debug → 直接
  丢弃`——过滤发生在**追加期**；
- Python 原版同样在追加期过滤（`log_window.py:82`：`if level < logging.INFO and
  not self._show_debug.isChecked(): return`），logwin.rs 注释"原版语义：不回溯历史"
  自证；
- 用户体感：勾选「显示 DEBUG」后，**已产生的 DEBUG 行一条都不会出现**，只能等
  新日志——"这个开关是不是坏了？"。产品判断：过渡到**渲染期过滤**（缓冲全保留
  DEBUG 行，勾选即回溯显示全部保留行），逻辑更清晰：**缓冲=原始数据，显示=过滤
  视图**。原版差异落档 D-31。

### 2.6 根因五：日志不流入面板（LogLine 事件只重绘日志窗，面板不动）

- `app.rs:859-866`：`UiEvent::LogLine` → `logwin.push(...)` → **仅
  `redraw(WinId::Log)`**；
- 面板（WinId::Panel）的重绘来源 = 输入事件 / 设置防抖节拍（`TickKind::PanelApply`，
  仅设置变更期 300ms）/ 下载进度事件等（app.rs 各处 `redraw(WinId::Panel)` 均非
  日志驱动）；
- egui 即时模式：窗口画面只在 winit `RedrawRequested` 时重新生成；面板停在「日志」
  tab 且无输入事件时 **新日志不会落入画面**（缓冲里有，画面静止）——用户会以为
  "日志卡住了"（与截图"时间停在 17:13"的观感一致）。**需实机确认**（验证步骤见
  §5.2），代码侧论证已足够支撑预案：LogLine 且面板当前显示 Log 页 → 附加
  `redraw(WinId::Panel)`（`state.visible: HashMap<WinId,bool>` 判定可见性可用，
  state.rs:1197）。

### 2.7 其他小不爽（顺带核对）

| # | 现象 | 证据 | 处置 |
|---|---|---|---|
| a | 按钮名「打开日志文件」但实际**打开目录**（explorer） | `log_tab.rs:45-47` → `open_log_dir()`（`log_tab.rs:122-131`，`open_in_explorer(&dir)`）；i18n `log_open_file` | 改名「打开日志目录」，与行为一致；hover 显示"当前会话：\<路径\>"（替代尾部提示行，见 §3） |
| b | 复制全部无任何成功反馈 | `log_tab.rs:48-66` 静默 `ctx().copy_text` | 成功后按钮文字短暂切换「已复制 N 行」（~800ms），新 i18n 键 |
| c | 尾部「当前会话日志：」提示行又矮又长、路径一长就换行，且正是外层溢出触发增量（§2.2） | `log_tab.rs:110-118`；i18n `log_current_file` | 删除该行，信息并入 (a) 的按钮 hover；日志区空态文案 `log_empty` 保留 |
| d | 每帧重建全部行格式化字符串（2000 行 × `format!` × 每帧） | `log_tab.rs:76-86`、`logwin.rs:56-67`；环形上限 `state.rs:204`（MAX_LINES=2000） | P2 可选：push 期格式化缓存（LT-6），本轮可不动 |
| e | 时间戳仅 HH:MM:SS，跨日/跨会话无法定位 | `state.rs:213-215`（chrono %H:%M:%S），原版 datefmt 同款 | 记录，本轮不改（避免与日志文件内时间戳混淆面扩大） |

## 3. 目标设计（改造后形态）

### 3.1 布局（唯一滚动条 + 工具行置顶）

```
┌─ 日志  ─────────────────────────────────────────┐
│ [✓ 显示DEBUG] [✓ 自动滚动]        清除|复制全部|打开日志目录 │ ← 固定，恒置顶
│ ┌──────────────────────────────────────────────┐ │
│ │ HH:MM:SS [LEVEL] target: msg                  │←│ ← 日志列表唯一滚动条
│ │ …（最近 2000 行，按级别着色，filter 生效行）…   │ │
│ │               （回到最新 ⬇ +3 条）   （贴底才显示） │
│ └──────────────────────────────────────────────┘ │
│ （无底部提示行：当前会话路径在「打开日志目录」hover）  │
└──────────────────────────────────────────────────┘
```

层级：**日志 tab 不经过外层 ScrollArea**（`panel_ui` 对 `PanelPage::Log` 特判），
`log_tab::page` 内 = 工具行（固定） + 日志 ScrollArea（占满剩余，唯一滚动条）。
其他 7 页维持外层滚动不变（影响面最小）。

### 3.2 交互规则（每条对应一段"不爽机制"的消除）

| 机制 | 规则 | 消除的不爽 |
|---|---|---|
| 自动滚动（默认勾选） | **贴底跟随**：视图滚动偏移距视口内`内容底` ≤ 4px 容差 → 跟随；用户上翻超容差 → 挂起跟随（复选框仍勾选，无"被取消"感知）；用户滚回底部 → 恢复。挂起期间日志区右下浮动「回到最新 ⬇（+N 条）」，点击回底并恢复跟随。 | 上翻看历史被拽回；状态偷偷开关 |
| 显示 DEBUG（默认不勾） | **渲染期过滤**：缓冲全保留（含 DEBUG），仅渲染时按 `level≥INFO 或 show_debug` 决定可见行。勾选 → 历史 DEBUG 行立即出现，取消 → 立即隐藏。 | 勾选后历史的 DEBUG 不可见，误以为开关坏 |
| 清除 | 清空缓冲（仅列表，磁盘日志文件不受影响）；按钮 hover 注明"仅清空列表"。 | 误以为删除了日志文件 |
| 复制全部 | 复制**当前可见**行（过滤后），成功后按钮短时显示「已复制 N 行」。 | 复制无反馈 |
| 打开日志目录 | 打开 logs 目录；hover 显示「当前会话：\<路径\>」。 | 按钮名与行为不符；路径无处查看 |
| 空态 | 保留 `log_empty` 弱化文案；无底部提示行。 | 高度膨胀引发双层滚动条 |

### 3.3 状态与契约

- **零 `lt-proto` 契约变更**（AGENTS 冻结约束）：不改 Cmd/Event；
- 复用现有设置/状态：`LogWindowState { lines, show_debug, auto_scroll }`（state.rs:194-201）
  不变；新增**渲染依赖字段**：`new_since_bottom: usize`（距上次贴底的新行数，用于
  「回到最新 +N」）；
- 新增共享渲染 helper（建议 `lt-ui/src/windows/log_view.rs` 或 `state.rs` impl）：
  `level_visible(level, show_debug)` / `is_near_bottom(offset_y, viewport_h, content_h)`
  / 行格式化+着色，**logwin 与 log_tab 统一复用**（消除 log_tab.rs 与 logwin.rs 的
  重复格式化/配色逻辑——现两处各一份，是"逻辑不清"的维护层证据）。

## 4. 改造方案（施工卡）

### LT-1 布局：日志 tab 脱离外层滚动 + 工具行置顶（对应 §2.2/§2.3）

- `panel/mod.rs` `panel_ui`：`match page` 分派前特判——`PanelPage::Log` 分支不套
  外层 `ScrollArea`，直接 `Frame::inner_margin(12)` + `log_tab::page`；其余 7 页
  保持现结构（`ScrollArea` 包 `Frame` + `add_space(12)`）。
- `log_tab.rs`：
  - 删除底部提示行（`log_tab.rs:110-118`）→ 外层溢出触发增量消除；
  - 工具行保持 `page()` 首部（脱离外层后自然固定）；
  - 日志区 `ScrollArea::vertical().auto_shrink([false,false])` 占满剩余高度不动；
  - 「打开日志文件」按钮改「打开日志目录」（i18n 键 `log_open_file` →
    `log_open_dir`，zh/en 同步）+ hover「当前会话：\<路径\>」（`log_current_file`
    键改用途或新增 `log_open_dir_tip`，见 LT-4）。
- 验证：headless 固定 781 高渲染 Log 页，断言内容总高 ≤ 视口高（测试方案 §5.1）。

### LT-2 贴底跟随智能自动滚动（对应 §2.4）

- 渲染改用 `ScrollArea::show_viewport(|ui, viewport| { … })`（egui 0.36 提供；
  仓内尚无使用先例，实现时以 0.36 API 为准）；纯函数
  `is_near_bottom(offset_y, viewport_h, content_h, tol=4.0)` 判定；
- `auto_scroll && is_near_bottom` → `scroll_to_cursor(BOTTOM)`（跟随）；
  `!is_near_bottom` → 不跟随（挂起），按 `new_since_bottom` 显示浮动
  「回到最新 ⬇（+N）」按钮（日志区右下 `ui.put` 定位，或
  `Layout::right_to_left + bottom_up` 预留）；点击 → `scroll_to_cursor(BOTTOM)`
  并清零 `new_since_bottom`、恢复跟随；
- `new_since_bottom` 在「贴底渲染」时清零、非贴底时 = `lines.len() - last_bottom_len`
  推导（实现取最小状态记录：`last_bottom_len: usize` + `new_since_bottom` 二者其一；
  建议存 `last_bottom_len`，渲染侧推导计数，避免双字段失同步）；
- **logwin（独立日志窗）同步**：共享 helper 一致化——日志窗同样受"拽底"困扰，
  此改动同时是对复刻物的一处产品化升级（**D-31**）。
- 复选框语义约定（hover 说明可选）：勾选=期望跟随，用户翻页即挂起、回底即恢复，
  无需取消勾选即可翻看。

### LT-3 渲染期过滤：显示 DEBUG 可回溯（对应 §2.5）

- `state.rs` `LogWindowState::push`：去掉"`level < INFO && !show_debug` → 丢弃"
  分支（`state.rs:208-212`），缓冲全保留（环形 2000 上限不变；空时戳盖 HH:MM:SS
  逻辑保留）；`push` 返回值语义不变（追加成功 → 触发重绘，`app.rs:865` 依赖）。
- 渲染层统一经 `level_visible(level, show_debug)` 过滤；logwin 与 log_tab 都改。
- 测试更新：`state.rs:1771-1806` `logwin_filters_debug_by_default_and_caps_at_2000`
  语义反转（改为"缓冲全保留 + 渲染过滤"断言）；新增 `level_visible` 单测。
- **D-31 登记**："日志视图过滤与自动滚动交互与原版分道——原版追加期过滤不回溯 +
  勾选强制滚底；新为渲染期过滤可回溯 + 贴底跟随"。

### LT-4 按钮文案/反馈对齐（对应 §2.7a/b/c）

- i18n zh/en 同步：`log_open_file`→`log_open_dir`（"打开日志目录"/"Open log
  folder"）；`log_current_file` 改作 hover 模板（"当前会话：{path}"/"Current
  session: {path}"）或新增 `log_open_dir_tip`；新增 `log_copied`（"已复制 {n} 行"/
  "Copied {n} lines"）；`log_empty`/`show_debug`/`auto_scroll`/`clear` 不动。
- 复制反馈：`LogWindowState`（或 log tab 局部状态）记 `copy_at: Option<Instant>`，
  按钮渲染时若 `now - copy_at < 800ms` 显示「已复制 N 行」样式（accent 色短时），
  过期复原；复制行数 = 当前可见行数。
- 清除按钮 hover 提示"仅清空列表，磁盘日志文件不受影响"（可复用 `hint_line`
  风格或直接 `on_hover_text`，新键 `clear_list_only`）。

### LT-5 日志实时流入面板（对应 §2.6）

- `app.rs:859-866` LogLine 分支追加：`if panel 可见 && state.panel.page ==
  PanelPage::Log { self.redraw(WinId::Panel); }`（`state.visible` 判可见）；
- 若实机验证确认"面板日志 tab 静止不刷新"属实，本卡为必备；否则降级为
  "防御性重绘"（成本一次 request_redraw/行，可忽略）；
- 注意与日志窗共用 `logwin.push` 返回值：`push` 返回 false（如被环形丢弃？实际
  恒 true）时 logwin 不重绘——保持现有语义。

### LT-6（P2，可选）格式化性能

- 若走查发现日志高频（如 pad 挂起 debug 每 2s 数十行）时面板日志 tab 渲染卡顿：
  push 期把行格式化为 `VecDeque<String>`（着色渲染期派生），渲染只遍历显示
  行Label；现实测再定，**不阻塞本轮**。

## 5. 测试与验收

### 5.1 headless 自动化

- 新增/更新的单测（`state.rs`/`log_tab.rs` 内 `#[cfg(test)]`，风格随现有）：
  1. `level_visible` 矩阵（level ∈ {10,20,30,40} × show_debug ∈ {t,f}）；
  2. `is_near_bottom` 边界（容差内/外、空内容、超滚偏移）；
  3. `push` 全保留 + 环形 2000 上限（改写现有 1771 测试）；
  4. **布局不回归**：固定视图高（模拟 781 面板 pane）渲染 Log 页两帧，断言
     `crate::windows::panel::log_tab` 导出 helper（如 `content_height(avail)` 或
     直接把新布局拆成纯函数 `layout_parts`）总高 ≤ 可用高——保证外层滚动条
     永不复现（配套 panel 冒烟 `panel_ui_smoke_renders_all_pages_headless` 保持）；
  5. `new_since_bottom` 推导逻辑（贴底清零/上翻计数）。
- 回归：`cargo test --workspace`（基线 369 测全绿 + 新增）+ `cargo clippy --workspace`。

### 5.2 实机走查清单（全部在真实配置下 GUI 冒烟）

1. 打开控制面板 →「日志」tab，**对照截图**：最右淡蓝外层滚动条消失；工具行固定；
   日志列表滚动条唯一且可用；
2. **静止观察 10s 不动鼠标**：新日志行应持续落帧（LT-5 结论——若不落帧即实锤
   §2.6，按 LT-5 施工）；
3. 滚轮上翻到历史中部停留：不被拽回底部；右下出现「回到最新（+N）」；点击回底
   且恢复跟随；滚动到底部区域再次自动跟随；
4. 勾选「显示 DEBUG」：历史 DEBUG 行瞬间出现（如 pad muted/log device 行）；
   取消勾选：瞬间隐藏；
5. 复制全部 → 按钮短暂显示「已复制 N 行」；粘贴到记事本核对内容与时间戳；
6. 打开日志目录 → explorer 打开 `~/.config/livetranslate/logs`；hover 按钮显示
   「当前会话：\<路径\>」；目录内存在 `livetrans_*.log` 且内容与列表一致；
7. 清除 → 列表空 + `log_empty` 文案；磁盘日志文件仍在；
8. 独立日志窗回归：同样具备贴底跟随 + DEBUG 回溯（D-31 行为面）；不破坏原有
   深色高亮三色（ASR/Translate/Segment 着色在 `logwin.rs:16-19/33-43`，共享
   helper 时保留）。

## 6. 偏差登记（D-31）

> 文档编号顺延规则（docs/archive/rewrite-research.md §1.5 + AGENTS）：新阶段行为
> 差异自 D-22 起，当前已占用至 D-30（见 docs/sensevoice-language-fix.md），**本次
> 登记 D-31**。

- **D-31 日志视图显示/滚动交互重设计（2026-09-08，已生效 416bbfd）**：`log_window`（复刻）
  与面板「日志」tab 的过滤与滚动交互与原版分道——① 显示 DEBUG 由"追加期过滤、
  不回溯"改为"渲染期过滤、勾选即回溯"（原版 `log_window.py:82`）；② 自动滚动由
  "勾选即每帧强制滚底"改为"贴底跟随、上翻挂起、回底恢复"（原版
  `log_window.py:103`，实现为 egui `stick_to_bottom` + `advance_follow` 主动滚动
  检测）；③ 面板日志 tab 布局含滚动条/工具行置顶为 Rust 版自有页
  （原版无此 tab），无原版对应行为。登记即生效，施工以本文为准。

## 7. 影响面清单与提交计划

| 文件 | 改动 | 卡 |
|---|---|---|
| `lt-ui/src/windows/panel/mod.rs` | `panel_ui` Log 分支脱离外层 ScrollArea | LT-1 |
| `lt-ui/src/windows/panel/log_tab.rs` | 布局/按钮/hover/复制反馈/贴底跟随/过滤 | LT-1~4 |
| `lt-ui/src/windows/logwin.rs` | 复用共享渲染 helper（着色保留） | LT-2/3 |
| `lt-ui/src/state.rs` | push 全保留 + `last_bottom_len` + helper | LT-2/3 |
| `lt-ui/src/app.rs` | LogLine 附加面板重绘 | LT-5 |
| `assets/i18n/zh.yaml`、`en.yaml` | log_open_dir / log_copied / hover 键 | LT-4 |
| 测试 | §5.1 五组单测 + 冒烟回归 | 各卡 |
| `docs/log-tab-redesign.md` | 本文 | — |
| `AGENTS.md` | 待办区登记 | — |

提交顺序（docs 先行惯例，AGENTS「docs 提交时机」）：① 本文 + 索引 + AGENTS 待办
`docs(log-tab): …` 独立提交；② LT-1 完成+全绿 `feat(log-tab): …`；③ LT-2/3
（同批可合并）④ LT-4 ⑤ LT-5；每卡 `cargo test --workspace` 全绿再提交，不上
`git add -A`，逐项显式 pathspec。
