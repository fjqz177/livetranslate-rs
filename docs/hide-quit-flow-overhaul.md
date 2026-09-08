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
  悬浮窗默认位置主屏右下（app.rs:233-234：x = 宽-640、y = 高-608 → 逻辑 x≈640..1260、
  y≈192..692），居中框（≈465..815 × 325..475）与悬浮窗**几何交叠**，且置顶永远盖住框
  → 框实际弹出但**被悬浮窗压住不可见** = 用户看到"没有任何弹出"。
- 与此同时主线程**卡死在 MessageBoxW 的模态泵里**：winit 事件（对悬浮窗的点击等）被
  泵派发进 winit 内部队列却无人消费（winit 外层 pump_events 被阻塞）→ 界面
  **完全没有响应** = 用户看到"主界面直接没法响应"。

### 3.2 阻塞点 2 + 投递串行：托盘「退出」为何先弹「已隐藏」再弹「退出」

- tray 菜单由 Shell 弹出，右键始终可用；muda 回调（`tray.rs:99-101`）只做
  `EventLoopProxy::send_event` —— **Post，不处理**。
- 此刻循环正卡在 3.1 的模态泵里，`UiMsg::Menu(QUIT)` 静默排队 → 用户点「退出」
  **毫无反应**（"不然就卡住了"）。
- 用户最终遇到/点掉那个被重新暴露的「已隐藏」框（Alt-Tab/任务栏/与前台窗口交互后
  焦点与激活回落到该框——它是本线程最后激活的窗口）→ 模态泵退出 → 外循环恢复 →
  排队的 QUIT 才被处理 → `confirm_quit()` 弹出第二个 rfd 框（退出确认）→
  "点确定后弹出退出的窗口"。
- **三个症状一个根因**：同步模态 + 无 owner/非置顶 + 事件仅投递 = 串行双弹窗、假死、
  提示不可见，全部由 3.1 阻塞派生。

### 3.3 辅助缺陷

- 悬浮窗退出按钮与托盘 QUIT 的确认实现**重复且同样阻塞**（`overlay.rs:220-222` /
  `app.rs:422-428` 均走 `tray::confirm_quit()`）：任意一段处于模态时，另一入口不可达。
- rfd 框无宿主窗口：前台若是其他应用/另一块屏幕，框可能弹出在用户视线之外（与 3.1
  同源）。
- 其余 rfd 同步弹窗同病：清空确认（`overlay.rs:197-203`）、空导出提示
  （`app.rs:1225-1231`）、基准完成提示（`app.rs:852-857`）——都会在事件循环线程内
  阻塞（全面清点见 H-5 表）。

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
  等价物改用 **Windows 原生通知（ToastNotificationManager）**（见 H-2），语义同
  原版气泡：非阻塞、贴任务栏侧、短时驻留、进操作中心可回溯。

## 4. 改造方案（H-1~H-6）

目标状态矩阵：

| 场景 | 现状 | 目标 |
| --- | --- | --- |
| 悬浮窗「隐藏」按钮 | 先阻塞弹框（可能被遮）再隐藏 → 假死 | **立即隐藏** + 首次隐藏发系统原生通知（非阻塞） |
| 首次隐藏提示 | rfd 同步 MessageBoxW（可能被遮、阻塞） | **Windows 原生通知（Toast）**：非阻塞、必然可见（系统通知中心）、短时驻留 |
| 托盘「退出」 | rfd 阻塞确认框，可被其他模态串行/不可达 | egui 内嵌模态确认：不阻塞事件循环、任意时刻可达、单模态源 |
| 悬浮窗「退出」按钮 | 同 rfd | 同收敛（同一套模态） |
| 其他 rfd 弹窗（清空/空导出/基准完成） | rfd 阻塞 | 收敛到同一 egui 模态 / 通知窗（H-5，可另行排期） |

### H-1 隐藏动作：先藏后提示、移除阻塞调用

- `set_overlay_visible_with_hint(false)` 改为**先** `set_visible(WinId::Overlay, false)`
  （`app.rs:513` 提前），**删除 501-511 的 rfd::MessageDialog::show()**；
- 首次隐藏（`overlay_hide_notified` 语义保留）改为：调用 `notifications::notify_hidden()`
  ——`SetCurrentProcessExplicitAppUserModelID` + 装配 Toast（标题/正文 = i18n 键 × 当前
  语言，见 H-2）并 `Show()`，**非阻塞**；后续隐藏静默（现状保持）；
- 文本沿用 `hide_tray_hint`/`hide_tray_hint_title`（zh/en 两 yaml 已同步，不改键）；
- 效果：隐藏立即生效，事件循环零阻塞。

### H-2 首次隐藏提示改 Windows 原生通知（Toast，替换气泡语义）

> 用户裁决（2026-09-08）：通知载体用 **Windows 原生通知**（`ToastNotificationManager`），
> 不用自绘窗口，体验更贴近系统。零新依赖——复用已在用的 `windows` crate 0.62.2
> （增 feature 即可），不引第三方 crate。

**微软官方成因与要求（`Windows-classic-samples/Samples/DesktopToasts/CPP`，源码逐行验证）：**

1. 注释原话（DesktopToastsSample.cpp:157-158）："In order to display toasts, a desktop
   application **must have a shortcut on the Start menu**. Also, an AppUserModelID must be
   set on that shortcut." —— 未打包 Win32 桌面程序发 Toast 的**硬前提**：开始菜单
   （每用户 `%APPDATA%\Microsoft\Windows\Start Menu\Programs`）里存在一个**带
   `System.AppUserModel.ID` 属性**的 .lnk，目标指向本程序 exe。
2. 示例序列（同文件：130 RoInitialize / 177-190 检查并安装快捷方式 /
   203-238 InstallShortcut / 397 GetTemplateContent / 461-482 SetTextValues /
   516 CreateToastNotifierWithId / 521-538 ToastNotification + Show）。我们是纯 Rust，
   同一序列映射到 `windows` 0.62.2（符号与 feature 已在本地 crate 逐个验证，见附录 B）：
   - **RoInitialize 模式修正（审查发现，与示例不同）**：winit 0.30.13 在窗口创建线程
     已用 `CoInitializeEx(COINIT_APARTMENTTHREADED)` 初始化 COM（
     winit-0.30.13/src/platform_impl/windows/window.rs:1432-1440 `thread_local`），
     本线程即 STA。示例是无 winit 的裸 WinMain 才用 RO_INIT_MULTITHREADED；
     **本项目必须用 `RoInitialize(RO_INIT_SINGLETHREADED)`**——在 STA 线程调
     MULTITHREADED 会返回 `RPC_E_CHANGED_MODE`，通知静默失败；
   - `EnsureAumidShortcut()`：`CoCreateInstance(ShellLink GUID)` → `IShellLinkW::SetPath(exe)`
     → `IPropertyStore::SetValue(PKEY_AppUserModel_ID, VT_LPWSTR)` + `Commit()`
     → `IPersistFile::Save(lnk, TRUE)`；仅当 .lnk 缺失时创建（幂等，`GetFileAttributes` 探测）；
   - `SetCurrentProcessExplicitAppUserModelID(AUMID)`（一次；对齐任务栏/通知归属；
     官方示例未调用、非显示必要条件，属增强项）；
   - `ToastNotificationManager::CreateToastNotifierWithId(AUMID)`；
   - `GetTemplateContent(ToastText02)` → `XmlDocument`：第 1 个 `text` 节点 = 标题、
     第 2 个 = 正文（`GetElementsByTagName` + `XmlNode::SetInnerText`）；
   - `ToastNotification::CreateToastNotification(xml)` → `ToastNotifier::Show()`.
3. **AUMID**：新约定 `com.livetranslate.app`（`Vendor.AppName` 形态，语义稳定不可轻改；
   桌面示例为 "Microsoft.Samples.DesktopToasts"）。**不注册 ToastActivatorCLSID**（点击
   激活/回呼需 COM 激活器，本期不做——点通知无动作，与原版气泡一致即可）。
4. **i18n 处理（用户点题）**：
   - 文案复用既有键：标题 = `hide_tray_hint_title`（zh "LiveTranslate 已隐藏" /
     en "LiveTranslate is hidden"，en.yaml:611）、正文 = `hide_tray_hint`
     （zh "悬浮窗已隐藏，右键托盘图标可重新显示" / en "Overlay hidden. Right-click
     the tray icon to show it again."，en.yaml:290）——**zh/en 两 yaml 已同步**，无需新键；
   - 标题/正文在 `Show()` 前经 `lt_i18n::t()` 取**当前语言**拼入 XML → 运行期切语言
     后的通知自动跟随；通知归属显示名 = 快捷方式名（专有名词 "LiveTranslate"，不本地化）；
   - 追加新文案（如分段/副标题）须同时改 assets/i18n/zh.yaml 与 en.yaml（项目 i18n 铁律）。
5. **图标**：`assets/icons/app.ico` 已存在；.lnk 的 `SetIconLocation` 需要一个**磁盘路径**
   ——沿用「内嵌资产运行时解压到配置目录」的既有模式（lt-pipeline/src/vad.rs:14-36 的
   尺寸比对幂等解压），把 app.ico 解出到 `lt_models::paths::config_dir()`（
   lt-models/src/paths.rs:9；lt-ui 只读依赖 lt-models 为分层允许）再 SetIconLocation；
   不设则通知用系统默认图标，不阻断。
6. **失败兜底**：快捷方式创建失败/API 出错（如系统通知被禁用、权限异常）→
   `tracing::warn` + 静默返回；托盘菜单「显示悬浮窗」文字翻转（既有反馈面）仍在。
   旧 rfd 阻塞弹窗**彻底移除**（不再有 H-2 自绘窗方案，WinId::Notice 取消）。
7. **版本边界**：`CreateToastNotifierWithId` 要求 Windows 10 1903+（目标机 19045 ✓；旧系统
   失败走兜底静默，不阻断）。
8. 附带收益：通知进入 Windows 操作中心可回溯（用户错过可见）；H-5 的「空导出提示」
   「基准完成提示」复用同一 `notify(title, body)` 通道。

### H-3 退出确认改 egui 内嵌模态（核心）

- **状态**：`AppState` 增 `quit_confirm: Option<QuitConfirm>`（含宿主窗口选择结果，
  见下）；
- **触发改造**：
  - `on_menu(QUIT)`（app.rs:422-428）：删除 `tray::confirm_quit()`，改
    `request_quit_confirm()`（已开则幂等忽略，不叠加）。
  - 悬浮窗退出按钮（overlay.rs:220-222）：改为置 `quit_confirm`，不再直接
    `confirm_quit()`；
  - `tray::confirm_quit()` 随之删除（两调用点均收敛；rfd 对退出路径闭环）。
- **宿主选择**（`request_quit_confirm` 时定稿）：悬浮窗可见**且非紧凑模式**（逻辑高
  ≥ 280px）→ 宿主=Overlay（弹出即时所见；若悬浮窗正在紧凑态（高 200px，模态会装不下）
  或不可见 → `set_visible(Panel,true)` + focus_window，宿主=Panel——保证**任意状态**
  （含全 UI 隐藏/紧凑）下确认框必有宿主且在前台；
- **渲染**：共享 `render_quit_confirm(ui, state, pal)`（overlay_ui 与 panel_ui 帧内
  各自调用，仅宿主窗口渲染）——仿 `render_model_editor`
  （translation.rs:376-426）：`egui::Window::new(...).anchor(CENTER_CENTER)
  .open(&mut open)`，文案用既有 `quit_confirm_title`/`quit_confirm_msg`，按钮
  确定=置 `quit_requested=true`（既有 about_to_wait 退出路径，app.rs:1386-1390）、
  取消=清 `quit_confirm` + **恢复面板原可见性**（面板是因确认而临时显示的则回隐藏，
  避免"取消退出却多出一个面板"）；
- 效果：托盘退出在任意时刻**立即**响应；确认框在**自己窗口内**绘制（悬浮窗宿主 =
  置顶，恒可见）；无嵌套、无串行、无不可达。

### H-4 模态期间穿透挂起（H-3 安全前置）

- `poll_click_through`（app.rs:1148-1173）与 `poll_subtitle_click_through`
  （app.rs:1180-1186）：`quit_confirm` 打开且宿主=Overlay（悬浮窗穿透开启时）→
  强制 `set_window_transparent(false)` 并跳过轮询设置（否则点击穿透，模态按钮点不到）；
  模态关闭恢复原逻辑。
- 同类处理覆盖**所有宿主为 Overlay 的 egui 模态**（清空确认等，H-5）；宿主为 Panel
  的模态（页面恢复默认/删模型）本就在普通窗口内，无穿透问题。

### H-5（建议，可另行排期）其余 rfd 同步弹窗收敛

全面清点（审查后修正——初版只列了 4 处，实际 **8 处调用**，其中 2 处已由 H-1/H-3
收敛，剩余 6 处由本卡过渡）：

| 位置 | 语义 | 目标载体 |
| --- | --- | --- |
| `tray.rs:48-54` | 退出确认 | H-3 已收敛（删 confirm_quit） |
| `app.rs:505-510` | 首次隐藏提示 | H-1/H-2 已收敛（删弹窗） |
| `app.rs:852-857` | 基准完成提示 | 原生通知 `notify` |
| `app.rs:1225-1231` | 空导出提示 | 原生通知 `notify` |
| `overlay.rs:197-203` | 清空确认 | egui 模态（H-3 同一基础设施） |
| `subtitle_page.rs:112-118` | 字幕页「恢复默认」确认 | egui 模态（同设施） |
| `translation.rs:138-144` | 翻译页「恢复默认」确认 | egui 模态（同设施） |
| `data.rs:257-261` | 删除模型确认（YesNo） | egui 模态（同设施） |

- 纪律入册：**事件循环线程禁止任何同步 MessageBox/模态**（AGENTS 大坑追加条目）；
  `reset_toolbar` 等确认回调经 `reset_confirm` 状态桥接（页内闭包回调改为 `state` 置
  模态 + 完成后消费），确认结果的执行放在**宿主帧后动作**（与现有
  D-32/清空路径同款：入队 → 结果回写），避免在 UI 闭包内直接改 settings 落盘。

### H-6 回归测试

- **state 级**（既有测试风格）：
  - `quit_confirm`：request → Some；重复 request 幂等；confirm → quit_requested=true
    + None；cancel → None；
- **headless 渲染**（仿 `panel_ui_smoke_renders_all_pages_headless`、
  `model_editor_modal_smoke_renders_headless`：translation.rs:812）：
  - 面板/悬浮窗 smoke 增加 `quit_confirm=Some` 帧：不 panic、产出图元；
- **通知组装单元测试**（纯函数、无 OS 依赖——WinRT 调用不可在无窗口测试线程
  执行（RoInitialize 为线程级、且 STA 与测试线程不匹配），XML 文本装配留给实机冒烟）：
  - `notify_texts()`（i18n 映射纯函数）：两个语言下都能取到标题/正文
    （zh/en 对齐断言：zh 标题含 "已隐藏" / en 标题含 "hidden" 等）；
- **实机冒烟脚本**（运行册新增三场景）：
  - A：悬浮窗隐藏 → 立即无假死 + 首次隐藏弹出系统通知（标题/正文随系统语言），
    通知点击无副作用；再次隐藏不再弹；首次运行时开始菜单出现 LiveTranslate 快捷方式；
  - B：悬浮窗隐藏状态下托盘退出 → 立即弹面板 + 确认框，确定退出/取消恢复正常
    （取消后临时显示的面板回隐藏）；
  - C：悬浮窗穿透开启时退出 → 确认框按钮可点击（H-4 生效）。

## 5. D-33 偏差登记

- **D-33**：隐藏提示由「原生模态 MessageBoxW 兜底」（1206d82 引入）改为 **Windows
  原生通知（Toast）**——首次隐藏自动注册 AUMID 开始菜单快捷方式（含 app.ico 图标），
  通知走操作中心（成功/失败均不阻塞）；退出确认由 rfd 模态改为 egui 内嵌模态；
  隐藏/退出全链路从「阻塞事件循环」转为「全非阻塞」——与原版（气泡 + 退出无确认）
  分道，更稳更可见。后续新偏差自 D-34 起。

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
| `crates/lt-ui/src/windows/overlay.rs:197-203` | 清空确认 rfd |
| `crates/lt-ui/src/app.rs:491-514` | set_overlay_visible_with_hint（先弹后藏的病灶） |
| `crates/lt-ui/src/app.rs:916-919` | WinAction::Hide 处理 |
| `crates/lt-ui/src/app.rs:422-428` | 托盘 QUIT → confirm_quit |
| `crates/lt-ui/src/app.rs:851-857` | 基准完成 rfd 提示 |
| `crates/lt-ui/src/app.rs:1148-1186` | 穿透轮询（H-4 挂起点） |
| `crates/lt-ui/src/app.rs:1225-1231` | 空导出 rfd 提示 |
| `crates/lt-ui/src/app.rs:1386-1390` | quit_requested → event_loop.exit() |
| `crates/lt-ui/src/tray.rs:47-55` | confirm_quit()（将删除） |
| `crates/lt-ui/src/tray.rs:99-101` | muda 回调 → EventLoopProxy（仅投递） |
| `crates/lt-ui/src/state.rs:8-18` | WinId 枚举（lt-ui 内部，不涉 lt-proto；本方案不加新窗） |
| `crates/lt-ui/src/windows/panel/translation.rs:376-426` | egui 模态先例（model_editor） |
| `Windows-classic-samples/.../DesktopToasts.cpp:130,157-158,177-238,397-482,516-538` | 官方 Win32 Toast 三件套序列（RoInitialize/快捷方式/模板/Show） |
| `rfd-0.15.4/.../win_cid/message_dialog.rs:206-237` | MessageBoxW(owner=NULL) 同步模态 |
| `tray-icon-0.24.2/.../platform_impl/windows/mod.rs:56,75` | uID=私有 internal_id（气泡直调不可行） |
| `LiveTranslate/main.py:1933-1946 / 2288-2292` | 原版：先藏后气泡；退出无确认 |

## 附录 B：Win32 Toast 实现要点（windows crate 0.62.2 逐符号验证）

版本核对：`Cargo.lock` 中 `windows = 0.62.2`（lt-ui 的 `[target."cfg(windows)"]` 依赖，
加 feature 即可，**零新 crate**）。以下符号路径/名均已在本地
`~/.cargo/registry/src/.../windows-0.62.2` 源码逐个 grep 验证（含意外位置）：

| 用途 | 符号 | 路径 | 需增 feature |
| --- | --- | --- | --- |
| 通知管理/发送 | `ToastNotificationManager::CreateToastNotifierWithId(&HSTRING)` / `GetTemplateContent(ToastTemplateType)` | `windows::UI::Notifications` | `UI_Notifications` |
| 通知构造 | `ToastNotification::CreateToastNotification(xml)`（`impl ToastNotification` 内，mod.rs:3044，经由 IToastNotificationFactory 激活）、`ToastNotifier::Show(toast)`（mod.rs:3383） | 同上 | 同上 |
| 模板枚举 | `ToastTemplateType::ToastText02`（5 = 标题+正文两行） | 同上 | 同上 |
| XML 组装 | `XmlDocument::new()` + `LoadXml(&HSTRING)` + `GetElementsByTagName(&HSTRING)`；文本写 = **`XmlNode::SetInnerText(&HSTRING)`**（已验证存在，Dom/mod.rs:248/505/835） | `windows::Data::Xml::Dom` | `Data_Xml_Dom` |
| 快捷方式 | `IShellLinkW`（`SetPath`）、`ShellLink`（CLSID 常量，注意 0.62 里不叫 CLSID_ShellLink） | `windows::Win32::UI::Shell` | `Win32_UI_Shell` |
| AUMID 属性 | `PKEY_AppUserModel_ID`（fmtid 0x9f4c2855-… pid 5，注意位于 **EnhancedStorage**） | `windows::Win32::Storage::EnhancedStorage` | `Win32_Storage_EnhancedStorage` |
| 属性写 | `IPropertyStore::SetValue(*const PROPERTYKEY, *const PROPVARIANT)` / `Commit()` | `windows::Win32::UI::Shell::PropertiesSystem` | `Win32_UI_Shell_PropertiesSystem` |
| 快捷方式落盘 | `IPersistFile::Save(&PCWSTR, true)` | `windows::Win32::System::Com` | `Win32_System_Com` |
| 属性值 | `PROPVARIANT`（`VT_LPWSTR` 置 `pwszVal`） | `windows::Win32::System::Com::StructuredStorage` | `Win32_System_Com_StructuredStorage` |
| COM/WinRT 初始化 | `CoCreateInstance` / `RoInitialize(RO_INIT_TYPE)` | `windows::Win32::System::Com` / `…::WinRT` | `Win32_System_Com` + `Win32_System_WinRT` |
| 进程 AUMID | `SetCurrentProcessExplicitAppUserModelID(&HSTRING)` | `windows::Win32::UI::Shell` | `Win32_UI_Shell` |

**RoInitialize 模式（审查修正）**：桌面示例用 `RO_INIT_MULTITHREADED`（裸 WinMain 无其他
COM 初始化）；本项目由 winit 0.30.13 先行：`winit-0.30.13/src/platform_impl/windows/
window.rs:1432-1440` 的 `thread_local COM_INITIALIZED` 在窗口创建线程以
`CoInitializeEx(COINIT_APARTMENTTHREADED)`（**STA**）初始化过 COM → 本线程必须调用
`RoInitialize(RO_INIT_SINGLETHREADED)`；若误用 MULTITHREADED 得 `RPC_E_CHANGED_MODE`，
通知静默失败。RoInitialize 为**线程级**调用：仅 winit 主线程（所有 Show 调用侧）初始化
一次即可，H-6 的单元测试不触碰 WinRT。

备选（**不取**）：在 winit 之前于 main() 最先 `RoInitialize(RO_INIT_MULTITHREADED)`——
winit 文档（src/platform/windows.rs:492-497）明示此路径可行，但会改变 winit 自身
ITaskbarList 等的 COM 上下文，且得在企业级线程里协调，风险≥收益；保持与 winit 一致
的 STA 是零副作用选择。

**feature 落点**：新增 feature 加到 `crates/lt-ui/Cargo.toml` 的
`[target."cfg(windows)".dependencies] windows = { workspace = true, features=[...] }`
（feature 在工作区级 union 生效，lt-app 无需改动）。

实现形态建议：

- 新模块 `crates/lt-ui/src/notifications.rs`（**整体 `#[cfg(windows)]` 门控**；非 Windows
  目标编译时对应函数为日志空实现，避免跨平台编译破裂——本项目当前仅 Windows 实现，
  该门控纯粹为编译健壮性）；沿用项目「unsafe Win32 胶水+独立小模块」风格，如
  `app.rs` 里 apply_layered/apply_window_region；对外仅两个入口：
  `pub fn show(title: &str, body: &str) -> anyhow::Result<()>`（内部 `Once` 幂等完成
  RoInitialize + SetCurrentProcessExplicitAppUserModelID + EnsureAumidShortcut；
  每次发送后不缓存失败）、`pub fn show_hidden_hint()`（i18n 取词的薄封装）；
- `EnsureAumidShortcut()`：`.lnk` 路径 = `%APPDATA%\Microsoft\Windows\Start
  Menu\Programs\LiveTranslate.lnk`（env `APPDATA` 拼装）；`GetFileAttributes` 存在即
  跳过（便携 exe 迁移导致目标失效仅影响点击，不阻断通知——见下）；不存在则
  CoCreateInstance(ShellLink) + SetPath(exe) + SetValue(PKEY_AppUserModel_ID,
  "com.livetranslate.app") + Commit + IPersistFile::Save；
- 图标（可选增强）：`assets/icons/app.ico` 已入库；沿用「内嵌资产解压到配置目录」
  既有模式（onnxruntime/silero 同款）解出后 `IShellLinkW::SetIconLocation`；不做也
  可用系统默认图标；
- 失败兜底：任一步 Err → `tracing::warn!("原生通知失败: {e}")` + 返回；托盘菜单文字
  翻转/日志（既有反馈面）兜底，**绝不回退到阻塞弹窗**。
- 已知边界：① 系统通知被用户关闭/专注助手开启 → Show 成功但不显示（产品外行为，
  在日志中记录已发送）；② 首次运行会在用户开始菜单新增一个快捷方式条目（产品行为
  变化，随 D-33 落档）；③ 通知点击默认无动作（不注册 ToastActivatorCLSID）——与原版
  气泡一致；若后续要「点击通知恢复悬浮窗」再引入 COM 激活器（另立项）。
