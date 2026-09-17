# 字幕窗体验改造计划（subtitle-window-overhaul）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> 状态：**已施工（2026-09-08；WP-1~WP-5 全部落地，396 测全绿，D-36）**——WP-2 落地时穿透分区化等行为差异以 D-36 登记；WP-6（托盘字幕勾选项）/WP-7（多语言行、双窗逐像素 alpha）为远期，不在本次
> 域：lt-ui（含 i18n）+ lt-proto 默认值微调；不触碰 lt-pipeline/lt-asr/lt-translate
> 新偏差：本改造引入的行为差异自 **D-36** 起登记（穿透由"全窗"升级为"分区"= D-36 主体，hover 顶条/避让/工作区钳制为 Rust 新增原版无）
> 证据基线：2026-09-08；引用行号为当日基线，施工前若代码漂移以 grep 定位为准

---

## 1. 背景与问题

用户反馈：悬浮窗（主界面）"字幕"按钮打开的额外字幕界面**不好用**，首诉"**没法移动位置**"。

走查补充确认（全部为代码级证据，见 §6 证据附录）：

- **拖动=仅鼠标中键**，且**无任何提示**——i18n 键 `subwin_drag_hint`（"字幕窗口已显示，鼠标中键可拖动字幕位置"，zh.yaml:341 / en.yaml:340）存在但代码中**从未被使用**（grep 全网 crates 零命中）。原版 Python 首次开启会弹一次托盘气泡（main.py:1988-2000），Rust 版缺失。
- **开启穿透后整窗穿透**：`poll_subtitle_click_through`（app.rs:1226-1232）只做全窗 `WS_EX_TRANSPARENT` 位断言；穿透开启时中键也穿透，用户**没有任何办法再碰到窗口**（只能绕去设置页关闭）。
- **半透明是整窗均匀 alpha**：`subtitle_layered_alpha`（app.rs:1569-1572）`LWA_ALPHA = bg_opacity`；默认 bg_opacity=76 → 全窗（**含文字**）仅约 30% 不透明，白字变灰字。原版是逐像素 alpha（背景 76/255、文字 255/255 互不影响）——影响观感的隐性偏差。
- **Z 序无定义**：悬浮窗与字幕窗均 `AlwaysOnTop`（app.rs:171-174），相对顺序靠"创建先后"隐式决定（app.rs:110 悬浮窗先建、113 字幕窗后建 → 字幕窗偏上且无任何维护），show/hide 后可能漂移——若字幕窗盖在悬浮窗上，主界面按钮不可读不可点。
- **钳屏用整显示器尺寸**（subtitle.rs 模块注释明说 winit 无 work-area API 为已知偏差）→ 字幕窗钳到屏底会压住任务栏，恰是最常见使用场景（视频底部）的视觉事故。

## 2. 原版（Python）逻辑闭环（研究结论）

> 全部证据在仓库副本 `LiveTranslate/subtitle_window.py` / `main.py` / `control_panel.py`。

**入口（三处共写 `subtitle_mode.enabled`）**：悬浮窗"字幕"按钮（subtitle_overlay.py:658-660 → `subtitle_toggled` 信号）→ 托盘"Subtitle Window"可勾选项（main.py:1974-2006）→ 面板字幕 tab 启用开关（control_panel.py:949-956 防抖 200ms）。

**形态**：无边框（Frameless + WindowStaysOnTop）、`WA_ShowWithoutActivating`（永不抢焦点）、`WA_TranslucentBackground`（逐像素 alpha）；宽度固定 `window_width`（默认 1000），高度自适应任务（150ms OutCubic，锚定中心 y-Δh/2，subtitle_window.py:646-671）。

**拖动**：全窗**中键**按下→记录偏移→move（subtitle_window.py:789-808）；释放发 `position_changed`。

**位置记忆**：`position_changed` → 500ms 防抖 → 写 `window_x/y + enabled`（main.py:1976-1984）；重启仅位置仍可见（任一屏 ±50px）才恢复否则 (100,100)（subtitle_window.py:563-569），随后 `_clamp_to_screen` 钳屏（538-548）。

**穿透**：全窗 `WS_EX_TRANSPARENT`；因 Qt 在 show/hide/raise 清扩展样式 → **500ms 定时器 + showEvent 双重点断言**（subtitle_window.py:759-788）。与 Rust D-34 发现的 winit `apply_diff` 清位（AGENTS 大坑 15）为同一物理机制。

**自动隐藏**：N 秒无新句淡出（fade/slide_down + 时长，subtitle_window.py:702-757）；新句到达淡入恢复；1500ms 最短显示排队（subtitle_window.py:834-871）；显示最近 N 句，" | " 拼接。

**窗内控件**：**零个**——纯文本 OBS 捕获用（subtitle_window.py:1-9 用途注释）。所有配置收在面板 tab + 托盘。

**内容流**：仅字幕窗可见时喂文本（main.py:1109-1113）；**独立翻译**——按字幕行启用目标语言集合（`get_target_languages`，subtitle_window.py:913-921）额外翻译（main.py:1317-1344/1555-1572），与悬浮窗翻译流并行。

**缺口（原版固有，我们优化的出发点）**：① 穿透后无法交互；② 全窗穿透+中键拖动依赖一次性提示；③ 与主界面重叠无任何处理（两窗都置顶，可叠可乱）；④ 无窗内快捷入口。

## 3. 目标体验闭环（一句话场景）

> 悬浮窗点"字幕"→ 字幕窗在**上次位置**出现 + 一次性通知说明手势 → 顶条/中键拖到视频底部 → 松手自动记忆 → 看视频（正文穿透不挡点击；独占全屏除外，见 §5.7）→ 悬浮窗永远压着字幕窗（重叠自动让位）→ 悬停浮现工具条（穿透/锁定/关闭）→ 面板字幕页深度配置即时生效 → 重启位置/显隐/样式全恢复。

## 4. 设计决策总览

| 项 | 决策 | 备注 |
|---|---|---|
| Z 序 | 悬浮窗 > 字幕窗 > 面板/日志（普通层）> 其他应用；字幕窗恒 topmost | 字幕窗 show 后显式下沉；悬浮窗 show 后抬顶；幂等缓存 |
| 全屏可见性 | 窗口化无边框全屏 ✅；独占全屏 ❌（Windows 系统级限制） | 帮助文案注明 |
| 重叠 | 拖动落点做矩形规避：字幕窗沿最短方向推出 + 钳屏（150ms 平滑）；无解则保留（Z 序保主界面可操作） | 仅悬浮窗；面板重叠记为已知折衷 |
| 穿透 | `click_through` 语义升级为**分区穿透**（正文穿透、顶条豁免）；关闭=全不穿透；穿透中工具条出现由 Win32 轮询驱动 | 不新增 lt-proto 字段（语义升级向后兼容） |
| 拖动 | 顶条左键（新主手势）+ 窗内中键（保留原版）；"锁定"禁用拖动 | 顶条=hover 浮现 |
| 窗内控件 | hover 顶条：☰ 归位 / 穿透开关 / 锁定 / 关闭；覆盖式绘制不参与布局（文字零 reflow） | OBS 捕获不受污染 |
| 半透明 | P0：默认 bg_opacity 76→190 + 文案说明；P2：双窗分层逐像素 alpha（研究项） | wgpu HWND Opaque 限制实锤（§5.5） |
| 设置页 | 穿透文案分区化 + "重置字幕窗位置"按钮 + 拖动提示开关 | 新 WinAction 内部枚举无需契约评审 |

## 5. 分项设计与证据

### 5.1 窗口层级（始终在顶 + 悬浮窗永远可操作）

**现状证据**：`create_window`（app.rs:164-178）对 Overlay/Subtitle 统一 `with_decorations(false) + WindowLevel::AlwaysOnTop + with_active(false) + with_skip_taskbar(true)`；创建顺序 Overlay(110) → Subtitle(113)。

**设计**：

1. **Z 序维护函数**（宿主侧，Win32）：
   - `restack_subtitle_below_overlay()`：字幕窗 `SetWindowPos(hwnd_sub, insertAfter=hwnd_overlay, SWP_NOMOVE|NOSIZE|NOACTIVATE)`——放进 topmost 带内、悬浮窗正下方。
   - `raise_overlay()`：悬浮窗 `SetWindowPos(HWND_TOP, SWP_NOMOVE|NOSIZE|NOACTIVATE)`。
   - 触发点：`setup()` 创建后、`WinAction::ToggleSubtitle` 显示分支（app.rs:960-967）、悬浮窗 set_visible(true) 路径；高频路径缓存比对（记录上次动作，重复调用空转），常态零开销。
2. **效果定义**：字幕窗永远浮于普通应用（含全屏视频的窗口化全屏）之上，但永不为压住主界面按钮/下拉——主界面永远可点可读。
3. **Alt-Tab/任务栏**：现状已 `skip_taskbar`（对应 `WS_EX_TOOLWINDOW`）+ `with_active(false)`，字幕窗不进任务栏、不抢焦点、不在 Alt-Tab；保持。

### 5.2 每显示器工作区钳制（堵住"压任务栏"事故）

**现状证据**：`monitor_rects`（app.rs:1102-1121）用 `MonitorHandle::{position,size}`（整显示器）；subtitle.rs 模块注释（25 行"多屏判定用 winit 全显示器尺寸（原版 availableGeometry 剔除任务栏）"）已记偏差。字幕窗默认场景 = 视频底部 → 钳屏压任务栏正是高频事故。

**设计**：`monitor_rects` 的输出源改为 `::windows::Win32::Graphics::Gdi::GetMonitorInfoW(MONITORINFOEXW)` 的 `rcWork`（每显示器工作区）。证据：windows-0.62.2 中 `GetMonitorInfoW`/`MONITORINFOEXW` 位于 `Win32::Graphics::Gdi`；lt-ui `Win32_Graphics_Gdi` feature **已启用**（lt-ui/Cargo.toml:41），零新增依赖。枚举仍走 winit `available_monitors`（拿到每个 `MonitorHandle` 的 HMONITOR 需经 winit 内部？——**注意**：winit 的 `MonitorHandle` 未公开 HMONITOR；方案 = 枚举仍用 winit，但 `rcWork` 用 `MonitorFromWindow/MonitorFromRect` 得当前窗所在显示器的工作区，钳屏按"含点显示器为先、主屏兜底"的既有语义（subtitle.rs:192-199），仅将每个矩形的高度/底边替换为 rcWork）。实现细节：`clamp_to_screen` 输入矩形列表由 winit 全屏矩形×rcWork 混合——**简化决策**：把"主屏/含点屏"的矩形直接用 `GetMonitorInfoW(MonitorFromPoint(cursor/窗原点))` 的 rcWork 替换，其余屏退化为整屏矩形（钳制只对首屏语义敏感）。
   - 悬浮窗同函数受益（overlay 默认位已估 48px 任务栏，app.rs:229-236，一并消除估计）。
3. **响应时机**：钳制仍走现有 `ClampSubtitlePos`（app.rs:1016-1029）+ 屏变化（`ScaleFactorChanged`/`Moved`）后追加一次。

### 5.3 重叠避让（视觉处理）

**现状证据**：无任何重叠处理；overlap 即"后排窗被盖"。避让对象仅悬浮窗（面板/日志与字幕重叠记为已知折衷，§5.7）。

**设计**：

1. **纯几何模块**（lt-ui/subtitle.rs 或新 `windows/geo.rs`，可单测）：
   - `rects_intersect(a, b, safety: f32) -> bool`（含 24px 安全边距）；
   - `resolve_overlap(sub_rect, ov_rect, monitors) -> Option<(x, y)>`：沿"上/下/左/右"四个方向求推出成本（推出距离），选最小；推出目标再 `clamp_to_screen`；若推出后仍相交（屏壁夹死子窗尺寸超限），返回 None（保持现状）。
2. **时机**：字幕窗 `Moved` 防抖（app.rs:1405-1408 → `schedule_subtitle_pos_save`）与悬浮窗 `Moved` 防抖（app.rs:1401-1404）回调里各追加一次 `try_resolve_overlap()`；避让发生在拖动落点后，不打扰拖动过程。
3. **移动方式**：`window.set_outer_position` + `request_redraw`；可用 150ms 缓动（宿主 `HeightAnim` 同款机制推广为通用位置缓动，P1 可选；P0 直接落位 + 现有 Moved 防抖保存路径写回）。
4. **设置**：避让开关（默认开）→ 需要持久化时按 lt-proto 评审点（见 §5.8）。

### 5.4 悬停顶条工具条 + 拖动 + 分区穿透（本计划核心 WP）

**现状证据**：拖动入口 `subtitle.rs:423-426`（全窗 `interact` + `drag_started_by(Middle)` → `DragSubtitle`）；穿透 `app.rs:1226-1232`（全窗位断言，500ms）。

**几何常量（单一事实源，subtitle.rs 顶部）**：

```
STRIP_H = 18.0        // 顶条高（逻辑 px）
STRIP_RADIUS = 4.0    // 顶条圆角
STRIP_FADE_MS = 150   // 浮现/隐没淡入淡出
STRIP_MARGIN = 0.0    // 顶条贴窗顶（覆盖字体内容区上缘 8px 内边距不冲突）
CT_POLL_MS = 100      // 穿透/悬停轮询周期（原版 500ms 过慢，对齐悬浮窗 50ms 一档）
```

**顶条布局（覆盖式，不参与布局 → 文字零 reflow）**：

- 位置：窗口顶部内侧，宽=窗口宽、高 18px；左端 ☰ 拖动图标（提示"拖动可移动字幕窗"），右侧按钮流：[穿透开关][锁定][关闭]，各 24px 图标按钮；底色深色 70% + 顶条圆角（跟随窗圆角上半部）。
- 出现/隐没：**由 Win32 轮询驱动**（唯一事实源）——光标在窗矩形内 → 淡入；离开窗矩形 >2s 或点击其他窗口 → 淡出。穿透开启时 egui 收不到鼠标事件（WS_EX_TRANSPARENT），hover 检测**必须**走轮询（GetCursorPos 不受穿透影响），这是闭环关键。
- 重绘纪律（AGENTS 大坑 1）：仅在"进入/离开窗、动画运行中（150ms 内）"请求 redraw；动画期用 `request_repaint` 跟帧；空闲零帧率。

**操作映射（完整表）**：

| 手势 | 区域 | 动作 |
|---|---|---|
| 左键按住拖 | 顶条 | 移动窗口（新主手势）→ `WinAction::DragSubtitle`（复用 app.rs:995-998 drag_window 通路） |
| 中键按住拖 | 窗内任意处 | 移动（保留原版语义） |
| 单击 ☰ | 顶条 | 回到默认/最近视频位？——**P0 定案：☰=按钮，执行"复位到主屏底部居中"**（近似视频字幕惯例位） |
| 单击 穿透 | 顶条 | `settings.subtitle_mode.click_through` 翻转 + 工具条内高亮即时反馈 |
| 单击 锁定 | 顶条 | `subtitle.locked` 翻转（内存态；见 §5.8 持久化评审点）；锁定时拖动动作忽略并按 hover 提示 |
| 单击 ✕ | 顶条 | 隐藏字幕窗（`enabled=false` + `ToggleSubtitle`，与悬浮窗按钮同路径） |
| 右键 | 顶条 | 菜单：打开字幕设置页（定位字幕 tab）/ 复位位置 / 穿透模式 / 关闭（全部走 egui 内嵌模态与既有 action 通路，**禁止 MessageBox**——D-33 纪律） |
| Ctrl+拖 | 窗内 | **穿透开启时**临时恢复交互（轮询判 `GetAsyncKeyState(VK_CONTROL)` 强制解穿透 1s；`with_active(false)` 下键盘状态查询照常，无需焦点） |

**分区穿透**：`poll_subtitle_click_through` 重写为 cursor 判定（复用悬浮窗 `poll_click_through` 的模型，app.rs:1181-1219）：

```
zone = if cursor 在窗矩形内且 y < 顶条带(含 4px 安全带) => Strip
        else if cursor 在窗矩形内 => Body
        else => Outside
TRANSPARENT 位 := (click_through && zone == Body) || (click_through && zone == Strip && Ctrl) ...
```

投递时去重（按位比较）；周期 100ms；关闭/隐藏断链（现状续拍逻辑 app.rs:1455-1465 保留）。

**语义升级说明（D-36）**：`click_through=true` 由"全窗穿透"升级为"正文分区穿透"——原版全窗穿透下"点视频即穿、但窗也不可达"，新语义 99% 覆盖其用途且补回可达性（顶条豁免区）。用户可用穿透开关 + Ctrl 临时恢复兜底；**不新增契约字段**，旧配置 `click_through:true` 读取后自然获得新行为。

### 5.5 半透明（P0 修正 + P2 研究路径）

**技术约束实锤（依赖源码）**：wgpu-hal **30.0.1** `src/dx12/adapter.rs:1364-1365`——`SurfaceTarget::WndHandle(_) => vec![wgt::CompositeAlphaMode::Opaque]`，即 HWND 表面**只报 Opaque**；per-pixel alpha（PreMultiplied）仅 `SwapChainPanel`/`VisualFromWndHandle` 分支（adapter.rs:1367-1375）。项目 Painter = `egui_wgpu::winit::Painter`（app.rs:20），只走 WndHandle 路径 → **单窗逐像素 alpha 在当前栈不可达**。文档（app.rs:164-170）与此吻合。

**P0（本计划范围）**：
- `SubtitleMode::default()` 的 `bg_opacity` 由 **76 → 190**（lt-proto/src/settings.rs:448）。新用户默认"深色胶带"观感（75% 黑底 × 75% 文字——对比足、画面透 25%）。
- 设置页滑杆悬停文案："背景与文字随此项一同透明，建议 ≥70% 以获得清晰阅读"（zh/en 同步）。
- 存量用户保留既有值，不做主动迁移（尊重用户；D-36 记录默认值变更）。

**P1 可选**：设置页新增"整窗不透明度"滑杆（对齐悬浮窗 `window_opacity`，app.rs:1561-1564 模式）；统一 alpha = min/乘（字幕窗无窗级不透明度的原版对应，加此键属 Rust 自有产品化，正值 D-36 域内）。

**P2 远期（仅研究，不入本计划）**：① 双窗分层（背景窗 `LWA_ALPHA=bg_opacity` + 文字窗 `LWA_ALPHA=行目标`，同逻辑实体双 HWND 同步）；② DComp `VisualFromWndHandle` 自建 surface（egui-wgpu Painter 不支持，需 fork/PR 上游）——两者都能达成"背景透、字亮"，但都是"大项"，另立 WP。

### 5.6 设置页与入口补齐

**现状证据**：面板字幕页（`windows/panel/subtitle_page.rs:100-345`）：基本组/背景组/自动隐藏与穿透组/文字行组；穿透仅一个 bool 勾选（subtitle_page.rs:221-234）；"重置窗口位置"入口在**样式页**（`windows/panel/style.rs:147` → `WinAction::ResetPositions`，app.rs:1030-1061 同时重置悬浮窗+字幕窗）。

**改动**：
1. 字幕页新增"重置字幕窗位置"按钮 → 新 `WinAction::ResetSubtitlePos`（lt-ui 内部枚举，state.rs:130-148 新增变体，无契约评审）→ 宿主：字幕窗回 (100,100) + 钳屏 + 位置写回（同 app.rs:1034-1057 模式）。
2. 穿透项文案改"正交穿透（正文穿透，顶部拖条可交互）"（D-36 语文案）；勾选行为不变（`click_through` bool）。
3. 新增"拖动提示"开关（控制 WP-1 toast 一次性性质 → 会话内一次，不做持久化；开关仅控制提示是否弹出，P0 不持久）。
4. 悬浮窗"字幕"按钮 hover tooltip 补"中键/顶条拖动可移动字幕窗"（i18n zh/en 同步）。

### 5.7 已知限制与折衷（写入帮助/文档）

| 限制 | 说明 |
|---|---|
| 独占全屏 | 播放器/游戏独占全屏不可见，系统级；提示用户选"窗口化无边框全屏" |
| 面板重叠 | 面板为普通层，字幕窗可能盖住面板；不做自动避让（避免"窗自己动"困惑），可拖开；后续反馈疼则加"面板聚焦时字幕窗避让"（P1 可选） |
| 半透明单窗均匀 alpha | P0 默认 190 缓解；双窗/DComp 为 P2 研究 |
| 内容流单语言 | 字幕行目标语言 ≠ 主目标语言时需要 pipeline 额外翻译（原版 get_target_languages 模式，main.py:1317-1344）；涉及 lt-translate/pipeline，**P2 另立**（本计划不改，D-36 注记） |

### 5.8 lt-proto 契约影响（评审点清单）

按 AGENTS 规约"结构字段/Cmd/Event 增删需评审"：

- **无结构变更**（本计划全部字段增补走 AppState 内存态或现有键值语义）；
- `bg_opacity` 默认值 76→190：值变更非结构变更，随本计划提交；
- 后续评审候选（本计划**不做**）：① `SubtitleMode.locked`（锁定持久化）② `click_through_mode`（如保留"全窗穿透"用户语义）③ `overlap_avoid`（避让开关持久化）④ 字幕窗 window_opacity。

## 6. 工作包拆分与施工顺序

### WP-1 可发现性：一次提示 + 手势文案（P0，最小）

- 证据：i18n 键已存在未接线（zh.yaml:341）；`notifications::show(title, body)`（notifications.rs:55）签名可复用；`WinAction::ToggleSubtitle` 显示分支 app.rs:960-967。
- 改动：AppState 加 `subtitle_hint_shown: bool`（会话内）；ToggleSubtitle(vis=true) 首次时 `notifications::show("LiveTranslate", t("subwin_drag_hint"))`（D-33 通知链路已是 toast 模板，零新依赖）；悬浮窗字幕按钮 tooltip（subtitle 按钮处 i18n）。
- 测试：无单测（外部副作用）；headless 冒烟断言标志位；实机确认 toast 视觉。
- 验收：首启弹提示一次，重启会话再弹（会话内）；文案双语。

### WP-2 悬停顶条工具条 + 分区穿透 + 锁定（P0，核心）

- 证据：拖动入口 subtitle.rs:423-426；穿透 app.rs:1226-1232；悬浮窗 cursor 分区模型 app.rs:1181-1219。
- 改动：
  1. subtitle.rs：常量组（STRIP_H/RADIUS/FADE/CT_POLL）；顶条纯绘制（最后画，覆盖式）；区域纯函数 `strip_zone(cursor_local, win_rect, strip_rect) -> Zone{Strip,Body,Outside}`（可单测）；工具条状态字段（visible/alpha/anim）进 `SubtitleUiState`；按钮点击动作（穿透翻转/locked 翻转/关闭/归位）。
  2. 宿主：`poll_subtitle_click_through` 重写（zone 判定 + `GetAsyncKeyState(VK_CONTROL)` 临时解穿透 + 周期 100ms）；工具条显隐由轮询驱动 + `request_redraw`；`drag_window` 保留（顶条左键与窗内中键均投 `DragSubtitle`，locked 时忽略）。
  3. state.rs：`SubtitleUiState` 增加 `toolbar_{show,fade}`、`locked` 字段；WinAction 不变（复用 DragSubtitle）。
- 测试：
  - 单测：zone 判定矩阵（Strip/Body/Outside×穿透开关×Ctrl）；淡入/淡出时机收敛；locked 拖动忽略；按钮状态翻转；
  - headless 冒烟：RawInput 模拟指针（复用 dsV4 教训的 headless 探针法，memory「dsv4-not-suited-zcode-cua」：run_ui + RawInput 模拟 + 扫 Shape::Text）断言顶条出现/消失/按钮居中；
  - 实机：穿透开启时鼠标滑至窗顶 → 工具条 1 拍内出现可操作；正文区点击穿到视频；Ctrl+拖可恢复移动。
- 验收：顶条出现/隐没不影响文字布局像素；穿透开启时窗内正文无可交互（视频可点）；顶条按钮全部有效；锁定后拖动不移动。

### WP-3 窗口层级：Z 序 + 重叠避让 + 工作区钳制（P0）

- 证据：创建顺序 app.rs:110/113；Moved 防抖 app.rs:1399-1420；钳制现状 app.rs:1016-1029；subtitle.rs:192-199。
- 改动：
  1. 宿主：`restack_subtitle_below_overlay` / `raise_overlay`（SetWindowPos, SWP_NOACTIVATE；触发点 = setup 后、ToggleSubtitle show、悬浮窗 set_visible）；`try_resolve_overlap` 挂 Moved 防抖回调；
  2. 几何：`rects_intersect` / `resolve_overlap`（纯函数，单测）；
  3. 钳屏：`monitor_rects`/`clamp_to_screen` 输入矩形改 `GetMonitorInfoW rcWork`（`Win32_Graphics_Gdi` feature 已启用，lt-ui/Cargo.toml:41）；悬浮窗默认位（app.rs:229-236）迁走 48px 估算。
- 测试：单测（intersect 边界/安全距/四向推出成本/退出后仍相交返回 None/rcWork 钳制到任务栏上沿）；实机（重叠拖动观察让位；200% DPI；副屏）。
- 验收：重叠拖动后字幕窗自动不覆盖悬浮窗；字幕窗恒置顶且全屏视频可见（窗口化全屏）；任务栏不被压住。

### WP-4 半透明默认与文案（P0）

- 改动：lt-proto `SubtitleMode::default()` bg_opacity 76→190（settings.rs:448）；subtitle_page.rs 滑杆悬停文案 + 行 opacity 提示；i18n zh/en。
- 测试：单测（default 值、subtitle_layered_alpha 边界）；实机截图对比（半透黑底 + 白字可读）。
- 验收：新设置（删除 settings.json 或首次启动）默认观感 = 深色胶带、文字清晰；存量值不动。

### WP-5 字幕页入口补齐（P0）

- 改动：subtitle_page.rs 加"重置字幕窗位置"按钮；state.rs `WinAction::ResetSubtitlePos` 新增；app.rs 处理（回 (100,100) + 钳屏 + 写回，参照 app.rs:1034-1057）；穿透项文案分区化；"拖动提示"开关（若实现 WP-1 持久化则前置评审）。
- 测试：单测（无）；headless 冒烟（按钮存在、动作入队）；实机（重置后位置正确 + 重启恢复）。
- 验收：字幕页四项调整全部即时生效且经 300ms 防抖落盘（既有机制）。

### WP-6（P1 可选）托盘"字幕窗"勾选项

- 原版有（main.py:1974-2006 显隐 + 2026-2044 穿透快捷）；Rust 托盘当前最小菜单（tray.rs:34-40 ids 仅 5 项）。托盘菜单重构 = 托盘线程内重建菜单 + 勾选状态命令（TrayCmd 扩展）+ 事件回流（既有 proxy）。非阻塞可选，另立任务卡。

### WP-7（P2 远期，不入本计划）多语言行翻译流 / 双窗 per-pixel alpha / DComp 探索

- 各自为独立调研 + 施工文档。

## 7. 测试计划汇总

| 层 | 内容 | 预期增量 |
|---|---|---|
| 单测 | zone 判定、避让几何、cT 去重、locked、默认值、rcWork 钳制 | ~20 个 |
| headless 冒烟 | 顶条出现/隐没/按钮、穿透勾选后工具条仍可用、ResetSubtitlePos 入队 | 2-3 个 |
| 回归 | 现有 387 测全绿不回退（字幕换行/自动隐藏/排队/高度动画等既有用例全动不得） | 0 回退 |
| 实机走查 | §7.1 清单 | 手动 |

### 7.1 实机走查清单

1. 首启打开字幕窗 → toast 提示一次；本会话内再次打开不再弹；
2. 顶条左键拖 + 窗内中键拖 → 位置生效；重启恢复位置；
3. 与悬浮窗重叠拖动 → 字幕窗让位；四边夹死场景不崩不跳；
4. 窗口化全屏 YouTube → 字幕恒可见；正文点击穿透到播放器；
5. 穿透开启：鼠标滑窗顶 → 顶条 1 拍出现、可点可拖；Ctrl+拖可移动；
6. 200% DPI 缩放下位置/尺寸/顶条命中正常；双显示器副屏钳制正确；
7. 任务栏区域钳屏不压；面板打开时字幕窗重叠记录折衷（截图留档）；
8. 默认半透明观感（新配置文件）截图；行 opacity/描边联动；
9. OBS 窗口捕获（按窗口名"LiveTranslate Subtitle"）画面无工具条污染；hover 期间捕获可见（记录）；
10. 锁定开关：锁定后拖动无效、解锁恢复；
11. 自动隐藏/1500ms 排队/多句" | "回归（原有行为）；关闭按钮三入口状态一致（悬浮窗/设置页/顶条）。

## 8. 风险与回滚

| 风险 | 级别 | 对策 |
|---|---|---|
| Z 序重排与其他 topmost 应用（如置顶播放器）竞态 | 低 | SetWindowPos 仅作用于我们两窗；字幕窗 show/悬浮窗 show 时重断言 |
| 轮询驱动工具条 = 100ms 内窗口可能"浮现延迟" | 低 | 100ms 对齐悬浮窗 50ms 一档；人可感知阈值内 |
| 避让在旋转屏/屏组变化时误让 | 低 | 几何输入 = 实时 monitor 矩形（app.rs:1023 已有 available_monitors 刷新路径） |
| wgpu Opaque 限制（文字随背景淡） | 已知 | P0 默认 190；P2 双窗；不做"过亮补偿"（c/alpha>255 数学不可行） |
| 顶条点击与 egui 输入竞争（穿透豁免区点击） | 低 | 顶条区永远非穿透；轮询与 egui 事件同线程串行（winit），无竞态 |
| 每次改回滚 | — | 每 WP 独立提交；WP-2/WP-3 可单独 revert；重绘纪律严格（大坑 1） |

## 9. 提交与文档纪律

- 计划文档先于实现提交（本文件）；每 WP 完工 + 测试全绿即 `feat(subtitle-*):` 中文 commit；
- 施工前 `git status`/`git diff` 复核（AGENTS 约定，历史教训 af0ff58）；逐项显式 pathspec；
- i18n zh/en 同步；新偏差 D-36 于 WP-2 落地时在 AGENTS.md 登记（穿透分区化 + hover 顶条 + 避让 + 默认值）。

## 10. 证据附录（研究日 2026-09-08 基线）

### Rust 现状（crates/lt-ui/src/）
| 位置 | 内容 |
|---|---|
| app.rs:110-113 | 悬浮窗/字幕窗创建顺序（Z 序隐式来源） |
| app.rs:164-178 | Overlay/Subtitle 统一 decorations(false)/AlwaysOnTop/active(false)/skip_taskbar |
| app.rs:188-190 | 字幕窗 setFixedWidth 等价（resizable(false)） |
| app.rs:238-248 | 字幕窗启动位置恢复（window_x/y 可见性校验） |
| app.rs:693-717 | UpdateTranslation → 字幕窗喂文本（仅主目标语言） |
| app.rs:960-967 | ToggleSubtitle（显隐 + 穿透断言节拍续起） |
| app.rs:995-998 | DragSubtitle → drag_window |
| app.rs:1016-1029 | ClampSubtitlePos（多屏钳制，整屏尺寸） |
| app.rs:1030-1061 | ResetPositions（样式页入口，双语重置） |
| app.rs:1091-1098 / 1160-1176 | 字幕窗位置防抖保存（500ms → window_x/y 落盘） |
| app.rs:1181-1219 | 悬浮窗分区穿透模型（cursor 判定，50ms） |
| app.rs:1226-1232 | 字幕窗全窗穿透断言（500ms） |
| app.rs:1399-1420 | Moved（防抖位置保存 + 表面重建） |
| app.rs:1432-1486 | 节拍分派（PosSave/ClickThrough/SubtitleAutoHide/SubtitlePending） |
| app.rs:1561-1564 / 1569-1572 | overlay/subtitle 整窗 alpha（乘积 / =bg_opacity） |
| app.rs:1593-1608 | apply_layered（LWA_ALPHA） |
| subtitle.rs:17-26 | 已知偏差清单（含"多屏判定用 winit 全显示器尺寸"） |
| subtitle.rs:192-199 | clamp_to_screen（min/max 组合） |
| subtitle.rs:370-375 | 背景绘制不透明色（整窗 alpha 承担半透明） |
| subtitle.rs:423-426 | 全窗中键拖动 interact（drag_started_by(Middle)） |
| overlay.rs:158-173 | 悬浮窗"字幕"按钮（字幕按钮 → ToggleSubtitle，无提示） |
| overlay.rs:147 | 悬浮窗左键拖动（drag_started() 任意键） |
| panel/subtitle_page.rs:100-345 | 字幕设置页 4 组 |
| panel/subtitle_page.rs:221-234 | 穿透 bool 勾选 |
| panel/style.rs:147 | 样式页"重置窗口位置"入口 |
| state.rs:130-148 | WinAction 全集（内部枚举） |
| state.rs:509-590 | update_text/insert_sentence/on_auto_hide_timeout 逻辑 |
| tray.rs:34-40 | 托盘最小菜单 ids（无字幕项） |
| notifications.rs:25/55/72 | show_hidden_hint / show(title,body)（D-33 原生通知） |
| lt-ui/Cargo.toml:37-56 | windows 0.62 features（Win32_Graphics_Gdi 已启用） |

### l**t-proto + i18n**
| 位置 | 内容 |
|---|---|
| crates/lt-proto/src/settings.rs:422-460 | SubtitleMode 结构 + 默认值（bg_opacity:76） |
| assets/i18n/zh.yaml:306-358 / en.yaml:305-357 | 字幕相关键（含未接线 subwin_drag_hint: zh:341/en:340） |

### 原版 Python（工作区副本 LiveTranslate/）
| 位置 | 内容 |
|---|---|
| subtitle_window.py:550-589 | _setup_ui（flags/宽固定/位置恢复） |
| subtitle_window.py:538-548 | _clamp_to_screen |
| subtitle_window.py:646-671 | _fit_height_animated（150ms + 中心锚定） |
| subtitle_window.py:702-757 | 自动隐藏/恢复动画 |
| subtitle_window.py:759-788 | 全窗穿透 + 500ms 定时器 + showEvent 重断言 |
| subtitle_window.py:789-808 | 中键拖动 |
| subtitle_window.py:834-871 | 1500ms 最短显示排队 |
| subtitle_window.py:913-921 | get_target_languages |
| subtitle_window.py:1-9 | 窗口用途注释（OBS 捕获） |
| main.py:1974-2006 | 托盘显隐勾选 + 首次气泡提示（_subwin_notified） |
| main.py:2010-2017 | Alt+F4 关闭同步 |
| main.py:1317-1344 / 1555-1572 | 字幕窗独立目标语言翻译流 |
| control_panel.py:943-956 / 1060+ | 面板字幕 tab（SubtitleSettingsWidget + debounce） |
| subtitle_settings.py:311-633 | 设置 widget 全组（窗/背景/自动隐藏/穿透/文字行列表） |

### 依赖源码（win32/wgpu 约束实锤）
| 位置 | 内容 |
|---|---|
| ~/.cargo/registry/src/rsproxy.cn-*/wgpu-hal-30.0.1/src/dx12/adapter.rs:1364-1365 | WndHandle → 仅 Opaque 合成模式 |
| ~/.cargo/registry/src/rsproxy.cn-*/wgpu-hal-30.0.1/src/dx12/adapter.rs:1367-1375 | SwapChainPanel/VisualFromWndHandle → 全套 alpha 模式 |
| ~/.cargo/registry/src/rsproxy.cn-*/windows-0.62.2/src/Windows/Win32/Graphics/Gdi/mod.rs | GetMonitorInfoW/MONITORINFOEXW 所在模块 |

## 修订注记（D-37，2026-09-08）

**WP-2「拖动=顶条任意键+正文中键」落地后实机仍无法移动**：egui 判定链路正常（headless 指针探针实证产出起止），卡在宿主 `window.drag_window()`（winit 0.30.13）——①Windows 实现 = `PostMessageW(WM_NCLBUTTONDOWN, HTCAPTION)` 走系统标题栏模态循环，而 winit WndProc 收到该消息后**先投递哑 `WM_MOUSEMOVE(0,0)`**（platform_impl/windows/event_loop.rs:1219-1232，注释自陈「cancels the modal loop early」）；LAYERED+TRANSPARENT 轮询窗上探针实测 0 位移、偶发事件循环挂死（Loop 挂起收不到释放）；②文档明确「Moves the window with the left mouse button」——**中键路径下循环不启动**。

修复（D-37）：弃 drag_window。egui 只报拖动起止（`WinAction::SubtitleDragStart/End`），宿主 `SetCapture` + `GetCursorPos` 绝对跟踪 `set_outer_position`（`update_subtitle_drag` 每帧推进，= 原版 Qt `mouseMoveEvent move(globalPos - _drag_pos)` 语义，任意按键可用）；拖动期间 `sub.dragging` 令分区穿透轮询豁免恒非穿透；落点沿既有 `ClampSubtitlePos`（钳屏+防抖保存+重叠避让）；拖动中隐藏窗口补 `ReleaseCapture` 收尾；锁定语义顺带收紧（正文中键一并禁用）。新增回归 3（顶条左键/正文中键起止序列、锁定双通道禁用）。

已知遗留：悬浮窗 `WinAction::Drag` 的 drag_window 路径同病（改期按同法修）；拖动实机确认待用户。
