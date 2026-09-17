# 托盘菜单打开时主界面卡死——TrackPopupMenu 模态循环阻塞 winit 事件循环（D-35）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> 用户反馈（2026-09-08）：主界面显示时点开托盘图标菜单，主界面立刻卡住不动
> （渲染/输入全停），但背后管道（ASR/翻译）运行正常。
> 结论：**tray-icon 0.24 在托盘窗 WndProc 内直调 `TrackPopupMenu`**——该调用在
> 调用线程跑**模态消息循环**；托盘窗建于 winit 事件循环线程，菜单一开就把
> winit 主循环挂起 → egui 不再出帧/收输入 → 整 UI 冻结；管道在后台线程不受影响。

## 1. 根因链（源码实证）

1. 托盘构建于主线程：`MultiWindowApp::setup` → `tray::build`
   （`crates/lt-ui/src/app.rs:147`），TrayIcon 的**隐藏托盘窗伴随创建在同一线程**
   （`tray-icon-0.24.2/src/platform_impl/windows/mod.rs` `TrayIconImpl::new`）。
2. 托盘窗 WndProc（`tray_proc`）对 `WM_RBUTTONUP/WM_LBUTTONUP` 分支直接调
   `show_tray_menu`（同文件 492-498 行）：
   `SetForegroundWindow(hwnd)` + **`TrackPopupMenu`**（542-558 行）。
3. `TrackPopupMenu` **模态**：在调用线程内自行 GetMessage/DispatchMessage 直至
   菜单消失。调用线程 = winit 事件循环线程 → winit `run_app` 主循环被整体挂起：
   - egui 无 `about_to_wait`/`RedrawRequested` 出帧 → 画面冻结；
   - 输入事件只积压不消费 → 点击全部无响应；
   - 菜单点击（muda WM_COMMAND 经 subclass 分发）能触发 `EventLoopProxy` 投递，
     但同样要等 winit 主循环恢复才被处理——**菜单开着期间 UI 无任何反应**。
4. 为什么原版（QSystemTrayIcon + QMenu）不冻结：Qt 把弹出菜单集成进自己的
   event loop（`QEventLoop::exec` 在 Qt 事件分发器内嵌套），主循环仍持续
   paint/处理其他窗口事件；winit 没有等价的「嵌套泵进 winit 循环」机制。
5. 与 D-33 大坑 14 的关系：同是「同步模态在事件循环线程内阻塞」，但性质不同——
   /14 是应用层同步 MessageBox，窗口期短且已知；/16 是库内隐式 TrackPopupMenu，
   每个托盘点击都会触发，属**必然命中**路径。

## 2. 修复（D-35）

- **托盘整体移入专用线程 `lt-tray`**（`TrayIcon`/`Menu`/`MenuItem` 全部建于该
  线程），线程内自带 Win32 消息泵（`GetMessageW`/`DispatchMessageW`）。
  `TrackPopupMenu` 的模态循环改在托盘线程跑，winit 主循环不再被挂起。
- **主线程 ↔ 托盘线程零阻塞**：`Tray` 对外降为通道句柄——`set_status` /
  `set_status_line` / `set_pause_label` / `set_overlay_toggle_label` 均为
  `mpsc::Sender::send` + `PostThreadMessageW(tid, TRAY_WAKE, …)` 唤醒（非阻塞，
  线程安全）；菜单文字/图标更新在托盘线程执行（Win32 对象线程亲和正确）。
- **事件回流保持不变**：muda `MenuEvent`/`TrayIconEvent` 事件处理器在托盘线程
  触发 → 现有 proxy 闭包 → `EventLoopProxy::send_event`（线程安全）→ winit 主循环
  处理 → `UiMsg::Menu` 逻辑零改动。
- **泵细节**：
  - 唤醒消息 `WM_APP+1`（hwnd=NULL 线程消息，不与托盘窗 WM_USER 6002-6009 冲突）；
  - `PostThreadMessageW` 要求目标线程已有消息队列——托盘线程先
    `PeekMessageW(PM_NOREMOVE)` 建队列**再**上报线程 id，杜绝
    「build 返回后首个命令的唤醒被丢」竞态；
  - 托盘线程**每拍先排空命令队列再 GetMessage**——TrackPopupMenu 的嵌套模态
    循环可能吞掉唤醒消息，菜单关闭后回到泵即补处理，不丢失状态更新；
  - **退出**：`Tray::drop` 发 `Quit` + 唤醒，经 ack 通道限时 2s 等待线程收尾
    （托盘线程在泵点退出并 drop `TrayIcon`→NIM_DELETE 在托盘线程执行）；超时
    则分离 join（进程退出 OS 回收），不再用 `PostThreadMessageW(WM_QUIT)`——
    该写法的 WM_QUIT 会被嵌套模态循环首先消费，外层泵随后阻塞、join 永久挂死。
- 移除已失效注释「必须在主线程调用」与 `TrayHandles` 跨线程句柄暴露。

## 3. 验证

- 新增回归测试：`tray::build` 线程握手/命令投递/`drop` 收尾（ack + join ≤2s）。
- 全量 `cargo test --workspace` 不回退（基线 386 测）+ clippy 零告警。
- 实机（待用户确认）：菜单打开期间主界面持续出帧、可操作；菜单选择后动作
  （暂停/显隐/面板/退出）正常触发。

## 4. 影响面

- `lt-ui` 内 API：`Tray.handles` 移除，改为 4 个 setter；`lt-app/shell.rs`
  仅用 `set_status`，签名不变。
- 不再修改 lt-proto 契约；不新增依赖（复用 `::windows::` 既有 feature
  `Win32_UI_WindowsAndMessaging`）。
- 既有 D-33 通知/退出确认流程不受影响（在 winit 主循环内，与托盘线程无耦合）。
