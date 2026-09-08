# 主界面隐藏与退出流程 —— 同步模态弹窗假死与双弹窗串行（D-33）

> 用户反馈（2026-09-08）：按下悬浮窗行1「隐藏」按钮后主界面直接没法响应、也没有任何弹出；
> 于是去托盘右键「退出」，此时才弹出一个新窗口告知"已经隐藏"，点确定后弹出退出确认窗口，
> 否则就卡住了。目标：彻底优化整套主界面隐藏的使用逻辑，给出有证据的改造计划。

## 1. 现象与目标定位

- 现象窗口：**悬浮窗（WinId::Overlay）行1「隐藏」按钮**。全仓库唯一 `t("hide")` 按钮在
  `crates/lt-ui/src/windows/overlay.rs:151-154`（文案 "隐藏" = `assets/i18n/zh.yaml:290`）；
  面板（控制面板）、日志窗、字幕窗均无隐藏按钮——定位无歧义。用户称「主界面」指
  悬浮窗（D-19 直进主界面后默认可见的日常操作面，面板按需打开）。
- 三段现象与观察者时间线：
  1. 点「隐藏」→ 主界面假死 + **没有任何弹出**；
  2. 托盘右键「退出」→ **先弹「LiveTranslate 已隐藏」**（`hide_tray_hint_title`）；
  3. 点确定 → 才弹「退出 LiveTranslate」确认框（`quit_confirm_title`）。
- 三段现象串行出现 = 教科书式「阻塞模态串行」；用户诉求：隐藏立即生效且反馈**可见、
  不阻塞**；退出**任意状态可达、一次确认、不与隐藏提示串行**。

## 2. 现状调用链（代码级）

### 2.1 隐藏按钮 → 首次隐藏提示（阻塞点 1）

```
overlay.rs:151-154   隐藏按钮 clicked → state.enqueue_action(WinId::Overlay, WinAction::Hide)
app.rs:916-919       process_actions（run_frame 尾部）→ WinAction::Hide
                     → set_overlay_visible_with_hint(false)
app.rs:491-514       set_overlay_visible_with_hint(false)：
  ├─ 492-498         托盘菜单文字翻转（tray_hide_overlay ↔ tray_show_overlay）
  ├─ 501-511         首次隐藏（overlay_hide_notified==false）：
  │                    rfd::MessageDialog::new()…show()      ← 同步阻塞（505-510）
  └─ 513             self.set_visible(WinId::Overlay, false) ← 藏在弹窗之后
```

### 2.2 托盘退出（阻塞点 2，且事件仅投递）

```
tray.rs:99-101   muda::MenuEvent 回调 → proxy(UiMsg::Menu(id))
                 = EventLoopProxy::send_event —— 纯投递，不重入、不处理
app.rs:1310      user_event → on_menu
app.rs:422-428   m::QUIT → tray::confirm_quit()            （第二个阻塞点）
                 → event_loop.exit()
tray.rs:47-55    confirm_quit() = rfd::MessageDialog::show()（同步阻塞）
```

### 2.3 悬浮窗自身退出按钮（同阻塞点 2）

```
overlay.rs:220-222  退出按钮 → crate::tray::confirm_quit() → state.quit_requested = true
app.rs:1386-1390    about_to_wait：quit_requested → event_loop.exit()
```

### 2.4 rfd 0.15.4 在 Windows 的弹窗实现（源码事实）

`rfd-0.15.4/src/backend/win_cid/message_dialog.rs`：

- `show()`（233-237）：直接 `WinMessageDialog::run()` **同步执行**；
- `run()`（206-218）：`MessageBoxW(self.parent.unwrap_or(null), …)` —— 本项目
  `MessageDialog` 从未 set_parent（全仓库无 `.set_parent(` 调用）→ **hWndOwner=NULL**；
- feature `common-controls-v6` 未启用（Cargo.toml 无该 feature）→ 走 MessageBoxW 分支
  而非 TaskDialog 分支；
- `MessageBoxW(owner=NULL, 无 MB_TOPMOST)`：**模态**——自带模态消息泵、**阻塞调用线程**
  直至被关闭。调用线程 = winit 事件循环线程（隐藏提示在 `RedrawRequested→run_frame→
  process_actions` 内；退出确认在 user_event 内）。

## 3. 根因链

### 3.1 阻塞点 1：隐藏提示 = 无 owner 同步 MessageBoxW

- `set_overlay_visible_with_hint(false)` 在 **set_visible(false) 之前**先 `show()`（
  `app.rs:505-510` vs `app.rs:513`）——弹窗时悬浮窗仍是 **AlwaysOnTop 可见窗**。
- MessageBoxW 无 owner、无 MB_TOPMOST → Z 序**永远低于置顶窗**；默认居中主屏。
- 几何实证（用户机 200% DPI，2560×1600 物理 → 1280×800 逻辑；D-32 已实锤该机 200%）：
  悬浮窗默认位置主屏右下（620×500 逻辑，x≈640..1260、y≈172..672），居中框
  （≈465..815 × 325..475）与悬浮窗**几何交叠**，且置顶永远盖住框 → 框实际弹出但
  **被悬浮窗压住不可见** = 用户看到"没有任何弹出"。
- 与此同时主线程**卡死在 MessageBoxW 的模态泵里**：winit 事件（对悬浮窗的点击等）被
  泵派发进 winit 内部队列却无人消费（winit 外层 pump_events 被阻塞）→ 界面
  **完全没有响应** = 用户看到"主界面直接没法响应"。

### 3.2 阻塞点 2 + 投递串行：托盘「退出」为何先弹「已隐藏」再弹「退出」

- tray 菜单由 Shell 弹出，右键始终可用；muda 回调（`tray.rs:99-101`）只做
  `EventLoopProxy::send_event` —— **Post，不处理**。
- 此刻循环正卡在 3.1 的模态泵里，`UiMsg::Menu(QUIT)` 静默排队 → 用户点「退出」
  **毫无反应**（"不然就卡住了"）。
- 用户最终遇到/点掉那个被重新暴露的「已隐藏」框（任务栏条目点击、激活回退等重新拉高
  Z 序）→ 模态泵退出 → 外循环恢复 → 排队的 QUIT 才被处理 → `confirm_quit()` 弹出
  第二个 rfd 框（退出确认）→ "点确定后弹出退出的窗口"。
- **三个症状一个根因**：同步模态 + 无 owner/非置顶 + 事件仅投递 = 串行双弹窗、假死、
  提示不可见，全部由 3.1 阻塞派生。

### 3.3 辅助缺陷

- 悬浮窗退出按钮与托盘 QUIT 的确认实现**重复且同样阻塞**（`overlay.rs:220-222` /
  `app.rs:422-428` 均走 `tray::confirm_quit()`）：任意一段处于模态时，另一入口不可达。
- rfd 框无宿主窗口：前台若是其他应用/另一块屏幕，框可能弹出在用户视线之外（与 3.1
  同源）。
- 其余 rfd 同步弹窗同病：清空确认（`overlay.rs:194-201`）、空导出提示
  （`app.rs:1224-1232`）、基准完成提示（`app.rs:851-857`）——都会在事件循环线程内
  阻塞。

### 3.4 对照原版（工作区 `LiveTranslate/` 副本，main.py）

- `on_toggle_overlay`（1933-1946）：**先 `overlay.hide()`**，后
  `tray.showMessage("LiveTranslate", hint, Information, 3000)` —— 非阻塞气泡、3 秒自动消失。
- `on_quit`（2288-2292）：`live_trans.stop(); app.quit()` —— **无确认框**。
  本项目「退出统一确认」（1206d82 提交，P1-1/ux-feedback 决策）是自有 UX，**保留**，
  仅改造其载体与可达性。
- 本项目 tray-icon 0.24.2 无气泡 API（AGENTS 大坑 7）；已核查 0.24.2 源码：
  `TrayIcon::window_handle()`（lib.rs:523）能拿到 hwnd，但 Shell_NotifyIcon 关联的
  **uID = 私有 `internal_id`**（platform_impl/windows/mod.rs:56,75），公开 API 拿不到 →
  NIF_INFO 无法挂到现有托盘图标（乱传 uID 会 NIM_MODIFY 失败或再生一个图标）。故气泡
  等价物改用**自绘置顶轻量通知窗**（见 H-2），语义同原版气泡：非阻塞、贴任务栏侧、
  自动消退。

## 4. 改造方案（H-1~H-6）

目标状态矩阵：

| 场景 | 现状 | 目标 |
| --- | --- | --- |
| 悬浮窗「隐藏」按钮 | 先阻塞弹框（可能被遮）再隐藏 → 假死 | **立即隐藏** + 首次隐藏弹独立置顶通知（非阻塞） |
| 首次隐藏提示 | rfd 同步 MessageBoxW（可能被遮、阻塞） | WinId::Notice 轻量通知窗：非阻塞、必然可见（置顶）、4s 自动消退/点击关闭 |
| 托盘「退出」 | rfd 阻塞确认框，可被其他模态串行/不可达 | egui 内嵌模态确认：不阻塞事件循环、任意时刻可达、单模态源 |
| 悬浮窗「退出」按钮 | 同 rfd | 同收敛（同一套模态） |
| 其他 rfd 弹窗（清空/空导出/基准完成） | rfd 阻塞 | 收敛到同一 egui 模态 / 通知窗（H-5，可另行排期） |

### H-1 隐藏动作：先藏后提示、移除阻塞调用

- `set_overlay_visible_with_hint(false)` 改为**先** `set_visible(WinId::Overlay, false)`
  （`app.rs:513` 提前），**删除 501-511 的 rfd::MessageDialog::show()**；
- 首次隐藏（`overlay_hide_notified` 语义保留）改为：置
  `state.notice = Some(NoticeState{ text: t("hide_tray_hint"), until: now+4s })`，
  确保 Notice 窗可见并重绘；后续隐藏静默（现状保持）；
- 文本沿用 `hide_tray_hint`/`hide_tray_hint_title`（zh/en 两 yaml 已同步，不改键）；
- 效果：隐藏立即生效，事件循环零阻塞。

### H-2 新增 WinId::Notice 轻量通知窗（替换气泡语义）

- **WinId 是 lt-ui 内部枚举**（`state.rs:8-18`），不涉 lt-proto 契约；
- 窗口属性仿 overlay：无边框、`WindowLevel::AlwaysOnTop`、skip_taskbar、
  `with_active(false)`；尺寸 ≈400×52 逻辑 px；位置 = 主屏工作区右下、任务栏上方
  ~24px、右缘 ~12px（对齐原版气泡"托盘角落"语义）；圆角复用
  `apply_window_region`（app.rs:1556-1569）；
- UI：深色半透明圆角卡片（dark visuals + `stabilize_widget_strokes`），单行
  `hide_tray_hint` 文本；egui 帧内点击 → 立即关闭；
- 消退：新增 `TickKind::Notice`（4s 到点 → `set_visible(false)` + 清状态），接入
  `about_to_wait` 节拍分派（app.rs:1393-1458）；
- 初始化注意：`with_startup`（state.rs:1412-1428）的 `visible` HashMap 需插
  `WinId::Notice → false`；`create_window` 列表（app.rs:109-133）追加；CloseRequested
  → 隐藏不退出（既有语义）。
- 该窗同时是 H-5（空导出/基准完成提示）的公共载体。

### H-3 退出确认改 egui 内嵌模态（核心）

- **状态**：`AppState` 增 `quit_confirm: Option<QuitConfirm>`（含宿主窗口选择结果，
  见下）；
- **触发改造**：
  - `on_menu(QUIT)`（app.rs:422-428）：删除 `tray::confirm_quit()`，改
    `request_quit_confirm()`（已开则幂等忽略）；不接受取消退出。
  - 悬浮窗退出按钮（overlay.rs:220-222）：改为置 `quit_confirm`，不再直接
    `confirm_quit()`；
  - `tray::confirm_quit()` 随之删除（两调用点均收敛；rfd 对退出路径闭环）。
- **宿主选择**（`request_quit_confirm` 时定稿）：悬浮窗可见 → 宿主=Overlay（弹出即时
  所见，置顶窗口内绘制无需切换）；否则 → `set_visible(Panel,true)` + focus_window，
  宿主=Panel（保证**任意状态**（含全 UI 隐藏）下确认框必有宿主且在前台）；
- **渲染**：共享 `render_quit_confirm(ui, state, pal)`（overlay_ui 与 panel_ui 帧内
  各自调用，仅宿主窗口渲染）——仿 `render_model_editor`
  （translation.rs:376-426）：`egui::Window::new(...).anchor(CENTER_CENTER)
  .open(&mut open)`，文案用既有 `quit_confirm_title`/`quit_confirm_msg`，按钮
  确定=置 `quit_requested=true`（既有 about_to_wait 退出路径，app.rs:1386-1390）、
  取消=清 `quit_confirm`；
- 效果：托盘退出在任意时刻**立即**响应；确认框在**自己窗口内**绘制（悬浮窗宿主 =
  置顶，恒可见）；无嵌套、无串行、无不可达。

### H-4 模态期间穿透挂起（H-3 安全前置）

- `poll_click_through`（app.rs:1148-1173）与 `poll_subtitle_click_through`
  （app.rs:1180-1186）：`quit_confirm` 打开且宿主=Overlay（悬浮窗穿透开启时）→
  强制 `set_window_transparent(false)` 并跳过轮询设置（否则点击穿透，模态按钮点不到）；
  模态关闭恢复原逻辑。
- 同类处理顺带覆盖 H-5 的 egui 模态（清空确认）。

### H-5（建议，可另行排期）其余 rfd 同步弹窗收敛

- 清空确认（overlay.rs:194-201）→ 同一 egui 模态基础设施（`confirm` 状态泛化：
  类型 + 文案 + 回调）；
- 空导出提示（app.rs:1224-1232）、基准完成提示（app.rs:851-857）→ Notice 通知窗；
- 纪律入册：**事件循环线程禁止任何同步 MessageBox/模态**（AGENTS 大坑追加条目）。

### H-6 回归测试

- **state 级**（既有测试风格）：
  - `quit_confirm`：request → Some；重复 request 幂等；confirm → quit_requested=true
    + None；cancel → None；
  - `notice`：show → Some + 到期时刻正确；tick 到点 → None + 通知窗可隐藏；
- **headless 渲染**（仿 `panel_ui_smoke_renders_all_pages_headless`、
  `model_editor_modal_smoke_renders_headless`：translation.rs:812）：
  - 面板/悬浮窗 smoke 增加 `quit_confirm=Some` 帧：不 panic、产出图元；
  - 新增 notice 窗 smoke：`ctx.run_ui` 渲染通知卡片、产出图元；
- **实机冒烟脚本**（运行册新增三场景）：
  - A：悬浮窗隐藏 → 立即无假死 + 通知窗 4s 内出现并可点击关闭；
  - B：悬浮窗隐藏状态下托盘退出 → 立即弹面板 + 确认框，确定退出/取消恢复正常；
  - C：悬浮窗穿透开启时退出 → 确认框按钮可点击（H-4 生效）。

## 5. D-33 偏差登记

- **D-33**：隐藏提示由「原生模态 MessageBoxW 兜底」（1206d82 引入）改为置顶轻量通知窗；
  退出确认由 rfd 模态改为 egui 内嵌模态；隐藏/退出全链路从「阻塞事件循环」转为
  「全非阻塞」——与原版（气泡 + 退出无确认）分道，更稳更可见。后续新偏差自 D-34 起。

## 6. 验收清单

- [ ] H-1~H-4 落地且 `cargo test --workspace` 全绿（净增测试 ≥4）、新码 clippy 零告警；
- [ ] 既有 380 测不回退（假死修复不得影响 overlay 三态几何/穿透/位置防抖）；
- [ ] 实机冒烟 A/B/C 三场景通过（A/B 为用户复现路径的原始对，C 覆盖穿透与模态交互）；
- [ ] release `cargo build --release -p lt-app` 零错 + 单 exe 冒烟；
- [ ] 完成回写：AGENTS 待办条目 + D-33 生效标记 + 本计划勾选。

## 附录 A：关键代码索引

| 位置 | 内容 |
| --- | --- |
| `crates/lt-ui/src/windows/overlay.rs:151-154` | 悬浮窗行1「隐藏」按钮 → enqueue(Hide) |
| `crates/lt-ui/src/windows/overlay.rs:220-222` | 悬浮窗「退出」按钮 → confirm_quit() |
| `crates/lt-ui/src/windows/overlay.rs:194-201` | 清空确认 rfd |
| `crates/lt-ui/src/app.rs:491-514` | set_overlay_visible_with_hint（先弹后藏的病灶） |
| `crates/lt-ui/src/app.rs:916-919` | WinAction::Hide 处理 |
| `crates/lt-ui/src/app.rs:422-428` | 托盘 QUIT → confirm_quit |
| `crates/lt-ui/src/app.rs:851-857` | 基准完成 rfd 提示 |
| `crates/lt-ui/src/app.rs:1148-1186` | 穿透轮询（H-4 挂起点） |
| `crates/lt-ui/src/app.rs:1224-1232` | 空导出 rfd 提示 |
| `crates/lt-ui/src/app.rs:1386-1390` | quit_requested → event_loop.exit() |
| `crates/lt-ui/src/tray.rs:47-55` | confirm_quit()（将删除） |
| `crates/lt-ui/src/tray.rs:99-101` | muda 回调 → EventLoopProxy（仅投递） |
| `crates/lt-ui/src/state.rs:8-18` | WinId 枚举（新增 Notice 不涉 lt-proto） |
| `crates/lt-ui/src/windows/panel/translation.rs:376-426` | egui 模态先例（model_editor） |
| `rfd-0.15.4/.../win_cid/message_dialog.rs:206-237` | MessageBoxW(owner=NULL) 同步模态 |
| `tray-icon-0.24.2/.../platform_impl/windows/mod.rs:56,75` | uID=私有 internal_id（气泡直调不可行） |
| `LiveTranslate/main.py:1933-1946 / 2288-2292` | 原版：先藏后气泡；退出无确认 |
