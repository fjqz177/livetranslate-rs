# 大坑全书（gotchas）

> **定位**：常青参考（docs/README.md 纪律①「常青参考」类）。只收**坑**——五段式：触发 / 症状 / 根因 / 对策 / 证据；规则与约定不收（那些归 AGENTS.md / prompts 卡）。
> **前缀**：G-（slug 派生自 gotchas，头注声明，全局唯一含 archive）。编号连续不复用；出本档引用带路径（如 `docs/gotchas.md` G-13）。
> **与 AGENTS §7 的关系**：§7 速查可精选不全列，但引用的每个 G-n 必须实存（check_agents_health.ps1 断言 3）。
> **增长机制**：施工卡第 5 条「踩新坑当场记候选」；另设候选清单（文末）承接 archive 各文档「怪癖」节回读——能五段式实证表述的才收编，表述不了的不臆造。
> **版本注记**：涉及依赖版本的条目带「版本」行；依赖升级时 grep「版本」回查相关条目。

## G-1 egui 多窗口 repaint 回环（1350fps 自旋）

- **触发**：egui_winit 多窗口事件循环中，按 `EventResponse.repaint` 转发 `request_redraw`。
- **症状**：1350fps 自旋、CPU 161%、其他窗口饿死白屏。
- **根因**：`EventResponse.repaint` 对输入事件为 true **必须**尊重（否则无节拍窗口点击全死）；但对 `RedrawRequested` 自身也返回 true，照单全收 → 每帧再请求下一帧，回环。
- **对策**：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }`
- **证据**：commit ad78047（头号坑）；旧 AGENTS 大坑 1。
- **版本**：egui 0.36。

## G-2 egui 0.36 三处 API 语义

- **触发**：使用 egui 0.36 的窗口尺寸、字体查询、颜色常量 API。
- **症状**：编译不过或运行语义不符预期。
- **根因**：0.36 改名/改语义：`set_inner_size` 改为 `request_inner_size`；`ctx.fonts()` 是闭包 API 不能取 owned；`Color32` 仅 premultiplied 常量是 const。
- **对策**：按上述新语义书写；构造非 const 颜色走运行时 API。
- **证据**：旧 AGENTS 大坑 2（M4/视觉施工期）。
- **版本**：egui 0.36。

## G-3 lt-ui 内 `crate::windows` 遮蔽 windows crate

- **触发**：在 lt-ui 代码里引用 windows crate 的类型/函数。
- **症状**：解析到 `crate::windows`（窗口模块）而非 windows crate，编译错或静默误用。
- **根因**：lt-ui 内 `windows` 模块名与 crate 名冲突，Rust 路径解析优先本地模块。
- **对策**：引用一律写 `::windows::`。
- **证据**：旧 AGENTS 大坑 3（M4 GUI 施工）。

## G-4 外部 MoveWindow 移动 winit 透明窗 → DXGI 白屏

- **触发**：绕过 winit 用 `MoveWindow` 移动透明窗口（字幕窗避让等场景）。
- **症状**：DXGI 表面失配，窗口白屏。
- **根因**：外部移动不经 winit，渲染表面尺寸未同步。
- **对策**：`Moved` 事件时以当前尺寸强制 `painter.on_window_resized`（幂等，重复调用无害）。
- **证据**：旧 AGENTS 大坑 4（D-36 字幕窗施工，docs/archive/subtitle-window-overhaul.md）。

## G-5 构建环境双保险勿动（双栈 CRT + uv 钉版工具链）

- **触发**：动 `.cargo/config.toml`、改动 libclang/cmake 来源、或新机器首次构建。
- **症状**：sherpa-onnx（静态 CRT）与 whisper-rs（/MD）链接冲突；whisper-rs-sys 无条件 bindgen 找不到 libclang。
- **根因**：两库 CRT 选项不一致；bindgen 硬需求 libclang（clang-sys 搜索语义：`LIBCLANG_PATH` 设置=只搜它，PATH 不搜）。
- **对策**：`.cargo/config.toml` 的 `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` + `CMAKE_POLICY_DEFAULT_CMP0091=NEW` 钉死 CRT；libclang 18.1.1 / cmake 4.4.3 由 uv 装进仓库内 `.venv`，cargo `[env]` relative+force 指向（首次构建前必须 `uv sync`）；**勿改回本机绝对路径、勿改回系统 LLVM/CMake**。
- **证据**：data-lifecycle ③ 根治（docs/archive/data-lifecycle.md）；commit 96ac36a（uv 方案）；旧 AGENTS 大坑 5。

## G-6 `block_on` 外求值 tokio timeout 必 panic；async-openai 需 `_api` feature

- **触发**：在 async 上下文外（如 block_on 闭包外）构造/求值 `tokio::time::timeout`；或编译 async-openai 0.41。
- **症状**：运行期 panic；或 LLM API 类型缺失编译错。
- **根因**：timeout future 在错误 runtime 上下文求值；0.41 的 API 类型藏于 feature 后。
- **对策**：timeout 移入 block_on 内；Cargo.toml 显式开 async-openai `_api` feature。
- **证据**：旧 AGENTS 大坑 6（lt-translate 施工）。
- **版本**：tokio / async-openai 0.41。

## G-7 tray-icon 0.24 无气泡 API

- **触发**：想做托盘气泡通知。
- **症状**：API 不存在。
- **根因**：tray-icon 0.24 未封装。
- **对策**：`Shell_NotifyIcon` 直调或换库（WP-5 遗留，届时按产品价值裁决）。
- **证据**：旧 AGENTS 大坑 7。
- **版本**：tray-icon 0.24。

## G-8 字幕窗 30% 黑底呈 179 灰 = 正常 alpha 合成

- **触发**：实机查看字幕窗（默认 30% 不透明黑底叠白窗）。
- **症状**：文字呈 179 灰而非纯白。
- **根因**：多层 alpha 合成的数学结果，非渲染缺陷。
- **对策**：勿误判为 bug 立案；要更亮直接调不透明度。
- **证据**：旧 AGENTS 大坑 8（D-36）。

## G-9 磁盘满两症（C 盘假死 / D 盘构建失败）

- **触发**：C 盘或 D 盘（构建盘）写满。
- **症状**：C 盘满 → 测试 TEMP 报 StorageFull、GDI+ Save 失败（表现为假死）；D 盘满 → release 构建 `os error 112`（target/ 曾达 57GB）。
- **根因**：TEMP 与构建产物分别在系统盘/工作盘，容量耗尽。
- **对策**：C 盘满临时把 TMPDIR 指 D 盘；D 盘满腾空间 / `cargo clean` / `CARGO_TARGET_DIR` 迁 C 盘。
- **证据**：旧 AGENTS 大坑 9 + 项目记忆两起实例（2026-09-08）。

## G-10 egui 窗 PrintWindow 抓旧帧

- **触发**：实机走查用 PrintWindow 给 egui 窗口取证。
- **症状**：抓到的是旧帧，位移/状态判断失真。
- **根因**：egui/wgpu 呈现与 PrintWindow 时序错位。
- **对策**：实机验收一律全屏截图。
- **证据**：旧 AGENTS 大坑 10。

## G-11 edition 2021 if-let scrutinee 临时值自死锁

- **触发**：`if let Some(x) = shared_lock.lock().unwrap().method() { … } else { 再 lock 同一把 }`。
- **症状**：同线程二次加锁永久死锁（界面/线程挂死）。
- **根因**：MutexGuard 临时值存活到整个 if-let 语句结束（**含 else 分支**）。
- **对策**：共享锁 + 分支派发一律先 `let v = lock.lock().unwrap().method();` 绑定再分支。
- **证据**：旧 AGENTS 大坑 11（WP-3 施工实锤）。

## G-12 `FontFamily::Name` 未注册族名即 panic

- **触发**：渲染侧直接构造 `FontFamily::Name(族名)` 而该族未注册。
- **症状**：panic（epaint `FontsImpl::font`）。
- **根因**：`FontFamily::Name` 只允许出现在 `FontsState::resolved`（已注册者）。
- **对策**：渲染取字体一律经 `fonts::font_family_for`，不得直接构造 Name 族。
- **证据**：旧 AGENTS 大坑 12（字体系统 W-1~W-7，docs/archive/font-system.md）。

## G-13 PROPVARIANT 的 Drop 会释放非 COM 内存（堆损坏 0xc0000374）

- **触发**：经 windows crate 组装 PROPARIANT 存 `VT_LPWSTR`（托盘通知、Shell 集成等）。
- **症状**：进程随机堆损坏崩溃 0xc0000374（D-33 notify_spike 实机反复取证）。
- **根因**：windows crate 对 PROPVARIANT 实现 Drop → PropVariantClear → 对 VT_LPWSTR 的 pwszVal 调 CoTaskMemFree；挂 Rust `Vec<u16>` 指针 = 非 COM 内存被释放（extensions/Win32/System/StructuredStorage.rs）。
- **对策**：VT_LPWSTR 值必须 `CoTaskMemAlloc` 分配，所有权交给析构释放。
- **证据**：D-33；docs/archive/hide-quit-flow-overhaul.md；commit adf5a50；旧 AGENTS 大坑 13。
- **版本**：windows crate。

## G-14 事件循环线程禁同步 MessageBox/模态

- **触发**：在 winit 事件循环线程弹 `rfd::MessageDialog::show()`（= `MessageBoxW(owner=NULL)`）或其他同步模态。
- **症状**：阻塞事件循环（假死）+ 无 owner/非置顶可能被置顶窗压住（「弹了看不见」）+ 托盘事件仅 `EventLoopProxy` 投递导致串行。
- **根因**：同步模态在事件循环线程跑嵌套消息泵。
- **对策**：一律 egui 内嵌模态（crates/lt-ui/src/windows/confirm.rs）或原生通知（crates/lt-ui/src/notifications.rs）。
- **证据**：D-33；旧 AGENTS 大坑 14。

## G-15 winit flags 翻转时整体重写 EXSTYLE，手工位被清

- **触发**：手工经 `SetWindowLongPtrW` 挂 EXSTYLE 位（`WS_EX_LAYERED`/`WS_EX_TRANSPARENT`）后发生 flags 翻转（如 set_visible 显/隐）。
- **症状**：手工位丢失——D-34 实锤 LAYERED true→false，隐藏→托盘重显半透明丢失。
- **根因**：winit `WindowFlags::apply_diff`（window_state.rs:395-410）按自身 flags 表 `SetWindowLongW(GWL_EXSTYLE, …)` 整体重建。
- **对策**：层属性不得只靠创建时设置+缓存比对，运行期**每帧实测缺位即重挂**（`window_has_layered`/`set_window_transparent` 状态在 host 侧维持同理）；任何手工窗口位都要考虑该清理路径。
- **证据**：D-34；docs/archive/hide-transparency-fix.md；旧 AGENTS 大坑 15。
- **版本**：winit 0.30.13。

## G-16 tray TrackPopupMenu 嵌套模态循环：托盘必须专用线程

- **触发**：托盘建于 winit 事件循环线程（旧注释「必须在主线程调用」是**错的**）。
- **症状**：点开托盘菜单即把 winit 主循环挂死（整 UI 冻结、管道正常）。
- **根因**：tray-icon 0.24 在托盘窗 WndProc 内直调 `TrackPopupMenu`（tray-icon-0.24.2 platform_impl/windows/mod.rs:542-558），在调用线程跑嵌套模态循环。
- **对策**：托盘图标/菜单/图标与文字更新全放**专用线程 + 自带 GetMessage 泵**（crates/lt-ui/src/tray.rs——注意无 lt-tray crate，托盘是 lt-ui 内模块）；主线程只经 mpsc 通道 + `PostThreadMessageW(WM_APP+1)` 唤醒发命令；**勿用 `PostThreadMessageW(WM_QUIT)` 退出**——WM_QUIT 会被 TrackPopupMenu 嵌套循环先消费、外层泵永挂，改命令 + ack 限时等待。
- **证据**：D-35；docs/archive/tray-menu-blocking-fix.md；旧 AGENTS 大坑 16。
- **版本**：tray-icon 0.24。

## G-17 winit `drag_window()` 不可靠：字幕窗/悬浮窗手工拖动

- **触发**：想用 winit `drag_window()` 拖动字幕窗/悬浮窗（尤其中键）。
- **症状**：LAYERED+TRANSPARENT 轮询窗上探针实测 0 位移/偶发事件循环挂死；中键拖动根本不启动。
- **根因**：Windows 实现 = `PostMessageW(WM_NCLBUTTONDOWN, HTCAPTION)` 走系统标题栏模态循环——①其 WndProc 收到该消息后**先投递哑 `WM_MOUSEMOVE(0,0)`**（event_loop.rs:1226-1227，注释自陈「cancels the modal loop early」）；②文档明确仅左键（"Moves the window with the left mouse button"）。
- **对策**：`WinAction::SubtitleDragStart/End` + `OverlayDragStart/End`——egui 只报拖动起止，宿主 `SetCapture` + `GetCursorPos` 绝对跟踪 `set_outer_position`（原版 Qt `mouseMoveEvent move(globalPos - drag_pos)` 语义，任意按键可用；`update_subtitle_drag`/`update_overlay_drag` 每帧推进）；拖动期间 `sub.dragging`=true 豁免穿透轮询；**拖动中隐藏窗口必须 `ReleaseCapture` 收尾**（否则输入被吞在隐藏窗）。
- **证据**：D-37（commit 12a9327）/ D-71（commit a289e5e）；docs/archive/subtitle-window-overhaul.md；旧 AGENTS 大坑 17。
- **版本**：winit 0.30.13。

## G-18 Qt 6.11 `grab()` 不渲染 QTextEdit 自身样式表背景

- **触发**：用 QWidget.grab 程序化截带样式表的 QTextEdit（重拍参照图脚本）。
- **症状**：截图中 QTextEdit 背景缺失（真实显示正常）。
- **根因**：Qt 6.11 grab() 的已知怪癖，不渲染 QTextEdit 自身样式表背景。
- **对策**：脚本对 viewport 补同色背景。
- **证据**：scripts/grab_reference_ui.py（已含修复）；旧 AGENTS 参照截图段拆出。
- **版本**：Qt 6.11（外部参考副本环境）。

## G-19 多代理并行期 stash 手术混入他人暂存

- **触发**：多代理并行施工期做 stash 手术或收尾提交。
- **症状**：他人暂存文件混入我方提交（af0ff58 实锤）。
- **根因**：stash/恢复操作作用于整个工作区，与并行代理的暂存交叉。
- **对策**：提交前 `git status` / `git diff` 逐项复核；显式 pathspec 提交，禁 `git add -A`。
- **证据**：commit af0ff58；旧 AGENTS 约定①拆出；项目记忆。

## G-20 会话内 grep 是 ugrep 包装函数，复杂 ERE 与 GNU grep 结果不同

- **触发**：在会话或入库脚本里写复杂 ERE（组内锚点+顶层交替等）并依赖其结果做判断。
- **症状**：同一正则在会话 grep（ugrep 包装）与 GNU grep 下命中不同——曾据此**误判 pre-commit 钩子过滤器有 bug**（2026-09-11 实锤）。
- **根因**：ugrep 与 GNU grep 的 ERE 方言差异。
- **对策**：校验用 `sh -c` / `command grep`；入库脚本避免依赖 grep 方言（PowerShell 脚本用原生 `-match`/`Select-String`）。
- **证据**：项目记忆 harness-grep-is-ugrep-function；docs/gotchas.md G-20（本条即对策范例——check_agents_health.ps1 实现约束）。

## G-21 sherpa binding 的 Default 不可裸用（qwen3 截断 / nano 全零参数）

- **触发**：构造 sherpa-onnx OfflineModelConfig 相关 Config 时直接 `Default::default()`（qwen3/nano 引擎）。
- **症状**：qwen3 输出被截断（max_new_tokens 默认 128，官方 512）；nano 更糟——Default 是 max_new_tokens=0 / temperature=1.0 / top_p=1.0，行为静默异常。
- **根因**：binding 的 Default 实现与官方推荐值不符（源码头注自陈）。
- **对策**：一律用 `OFFICIAL_MAX_NEW_TOKENS` 等显式常量，禁止裸 Default。
- **证据**：crates/lt-asr/src/engines/qwen3.rs:18、nano.rs:21 头注（2026-09-13 实读）；D-25 施工期。
- **版本**：sherpa-onnx 1.13。

## G-22 测试临时目录必须唯一化（Windows code 183 竞态飘测）

- **触发**：测试代码用固定名字的共享临时目录（pipeline/orchestrator 测试曾共用同名 tmp_models_dir）。
- **症状**：Windows 报 code 183（ERROR_ALREADY_EXISTS）竞态飘测。
- **根因**：并行测试同名目录争用。
- **对策**：每个测试用唯一名临时目录。
- **证据**：commit 8f964c5（根治）。

## G-23 无 BOM 的 .ps1 被 Windows PowerShell 按 ANSI 解析（中文全乱码、解析失败）

- **触发**：生成/入库 PowerShell 脚本且文件为无 BOM 的 UTF-8（含中文注释或字符串）。
- **症状**：脚本直接解析失败——中文字符串乱码（如「预算」→「棰勭畻」），报「表达式或语句中包含意外的标记」。
- **根因**：Windows PowerShell 5.1 对无 BOM 的 .ps1 按 ANSI 代码页（中文系统 = GBK）解码；有 BOM 才识别为 UTF-8。
- **对策**：入库 .ps1 一律带 UTF-8 BOM（既有四守护脚本文件头的 `﻿` 即此用途）；脚本内文本匹配按 G-20 用 PowerShell 原生正则。
- **证据**：check_agents_health.ps1 草稿 2026-09-13 干跑实锤（加 BOM 前解析失败、加 BOM 后同脚本五断言输出全对）。
- **注（2026-09-13 Q9/ADR-16）**：本仓调用面已统一切 pwsh-only——pwsh 对无 BOM 脚本默认按 UTF-8 解析，本坑仅在误用 powershell.exe 回落时存在；既有脚本 BOM 保留（pwsh 双读兼容，零 churn 不剥）。

## G-24 `scroll_to_cursor` 在 ScrollArea 外调用不生效（日志「回到最新」浮钮点不动）

- **触发**：在 `ScrollArea::show()` 返回之后的外层 Ui 上调 `ui.scroll_to_cursor(...)`（典型：点击画在滚动区外的浮动按钮想就地滚动视口）。
- **症状**：调用不报错，但视口只挪动约一个 item_spacing（几像素），等同没反应。
- **根因**：`scroll_to_cursor` 只往 `pass_state.scroll_target` 写**全局**滚动目标（值 = 调用点 Ui 的光标位置，即外层坐标），由下一帧收尾（`end()`）的 ScrollArea 消费且不校验目标归属哪个滚动区；外层光标 ≈ 视口底边，代入内容坐标公式算出的偏移增量 ≈ 0。egui 没有「指定某个滚动区滚动」的 API。
- **对策**：滚动指令必须在滚动区**内容闭包内**的 Ui 上执行——点击帧先记请求状态，下一帧在内容闭包末尾消费。「回到最新」浮钮即此实现：`LogWindowState::request_jump` / `take_jump_request`（lt-ui state.rs，D-31 跳底挂一帧）。
- **证据**：egui 0.36.1 `containers/scroll_area.rs` `end()`（scroll_target 消费无 id 校验、按内容坐标求 delta）+ `ui.rs` `scroll_to_cursor`（仅写 pass_state 全局目标）；2026-09-14 修复，headless 防回归 `jump_to_latest_actually_scrolls_to_bottom`（修复前 offset≈0、修复后滚到内容底 >1000px）。
- **版本**：egui 0.36。

## G-25 rust-cache 会清掉 target/ 下所有非 cargo 结构文件（-sys crate 的预编译库由此"消失"）

- **触发**：CI 用 `Swatinem/rust-cache` 缓存 `target/`，且某个 `-sys` crate 的 build.rs 把预编译静态库解到 `target/` 里（本仓 = sherpa-onnx-sys 解到 `<target>/sherpa-onnx-prebuilt/`，1.1GB）。
- **症状**：冷缓存那次（首次用新缓存键）全绿；**缓存命中的下一次**在链接期红：`error: could not find native static library 'sherpa-onnx-c-api'`，而且出错那步只花十几秒——说明 build.rs 根本没重新解包。
- **根因**：rust-cache 的 `src/cleanup.ts` 递归清理 target 目录时，凡不是 cargo profile 结构的目录只继续下钻，**目录里的文件一律 `rm`**（只留目录骨架与 `CACHEDIR.TAG`）。于是恢复出来的 `<target>/sherpa-onnx-prebuilt/<stem>/lib/` 是**空目录**，而 sherpa build.rs 的守卫是 `if lib_dir.is_dir() { 返回"已解包" }`——空目录照样放行，链接器自然找不到 `.lib`。
- **对策**：把预编译产物移出 `target/`：`scripts/fetch_sherpa_libs.ps1` 预解包到 `.cache/sherpa-onnx/extracted/`，`.cargo/config.toml` 设 `SHERPA_ONNX_LIB_DIR` 指向它（build.rs 已声明 `rerun-if-env-changed=SHERPA_ONNX_LIB_DIR`，链/设置变更会自动重跑）；`.cache/` 由 actions/cache 原样缓存（不做清理）。顺带 `cargo clean` 不再逼出一次 ~3 分钟重解包。
- **证据**：run 34965711794（2026-09-15 第二次跑、缓存命中 → Test workspace 19 秒后红）+ rust-cache v2.9.2 `src/cleanup.ts` 源码实证。
- **版本**：Swatinem/rust-cache 2.9.2。

## G-26 pwsh 里调 Git 自带 GNU tar 的两个 Windows 路径坑（反斜杠 / 盘符冒号）

- **触发**：pwsh 脚本里 `& tar ...`，参数是 Windows 绝对路径（含盘符 + 反斜杠）。
- **症状**：`tar: ...: Cannot open: No such file or directory`（反斜杠被吃成转义，路径成了字面反斜杠串，bzip2 子进程一起崩）；或 `tar: Cannot connect to <盘符>: resolve failed`（盘符冒号被当成远程主机名）。
- **根因**：Git for Windows 的 `usr\bin\tar.exe` 是 MSYS 版 GNU tar——反斜杠是它的转义符，而 `<盘符>:/…` 形状命中 GNU tar 的 remote-archive（`host:path`）语法。
- **对策**：喂给 tar 的路径一律转正斜杠 + 加 `--force-local`；tar 选型也要挑 GNU tar（Windows 自带 bsdtar 对 `.tar.bz2` 要外挂 bzip2，实测直接失败 `Child process exited with status 143`）。
- **证据**：2026-09-15 `scripts/fetch_sherpa_libs.ps1` 预解包改造实测（两种报错各复现一次后修复）。

## G-27 pwsh 裸调用 GUI 子系统 exe：不等待、不设退出码（CI「冒烟」恒绿）

- **触发**：在 PowerShell 里裸调用 GUI 子系统程序（本仓 `livetranslate.exe`，PE Subsystem=2）——典型是 CI 的 `--version` 冒烟步骤。
- **症状**：命令十几毫秒就返回（进程才刚起来、输出根本没接住），`$LASTEXITCODE` 不更新（沿用上一个命令的陈旧值）；exe 一启动就崩也发现不了——冒烟步骤恒绿，实际只证明了"文件存在"。
- **根因**：PowerShell 只对控制台子系统程序等待并采集退出码；GUI 子系统程序按其"不占控制台"语义被当作不等待处理。
- **对策**：一律 `Start-Process -Wait -PassThru -RedirectStandardOutput` 取真退出码（`-PassThru` 给进程对象 → `.ExitCode`；重定向还能顺带断言 `--version` 有输出）。本仓两处冒烟（ci.yml / release.yml）已按此修复；发布引擎 `release.ps1` 的 build ② 同款。
- **证据**：2026-09-16 实测（pwsh 7.6.6）：先 `cmd /c exit 7` 立基准 → 裸调用 exe 后 `$LASTEXITCODE` 仍是 7、耗时 14ms 且无输出；`Start-Process` 版得 ExitCode=0 + banner `livetranslate 0.1.0`。文件不存在时两种写法都会报错（裸调用 CommandNotFoundException / Start-Process InvalidOperationException）。
- **版本**：pwsh 7.6.6（行为自 Windows PowerShell 5.1 起一致）。

## G-28 钩子触发面「仅 assets/ 放行」≠ assets 内容免检（个人绝对路径漏网）

- **触发**：assets-only 提交（如改 `assets/SOURCES.md`）——钩子触发面 =「暂存区命中 .rs / Cargo.toml / docs / scripts / .github 等才跑门禁，仅 assets/ 放行」。
- **症状**：assets 提交全程零门禁，带入的个人绝对路径静默入库；直到后续任一触发面提交在本地全套守护（或 CI）才爆红——且爆的是当时 HEAD，不是引入提交，溯源多花一轮。
- **根因**：触发面判定（看暂存区）与守护扫描范围（全仓 `git ls-files`）是两回事：放行的是"这次提交不跑门禁"，不是"assets 内容永远免检"；坏内容只是延迟曝光。
- **对策**：assets-only 提交也自觉 `pwsh -File scripts/precommit.ps1` 跑一遍再交；要收窄/取消放行面属触发面裁决，勿擅自改。
- **证据**：2026-09-19 v1.0.0 发布当天：41ab253（assets-only）带入 SOURCES.md 个人绝对路径，被 cc38ef4（含 Cargo.toml）的本地钩子拦截，a91035d 修复。

## G-29 守护的 Test-Path 判定被本机未入库同名目录掩盖（本地钩子绿 ≠ CI 绿）

- **触发**：凡用文件存在性判定（`Test-Path` / `is_dir()`）的守护——本仓 `check_agents_health.ps1` 断言 5「引用路径存在」；本机存在但未入库 / gitignored 的目录（`docs/architecture/`、`docs/ui-audit/`、`docs/drafts/` 等生成物与草稿区）。
- **症状**：文档里"生成物不入库"之类的**政策性提法**（含路径字面量）本地守护全绿，推上 CI 直接 N 处"死引用"红——本仓 43 个积压提交首推即中。
- **根因**：`Test-Path` 查的是本地磁盘；本机同名目录掩盖了"fresh checkout 上不存在"的事实。本地绿只证明本机状态，不证明 CI 检出状态。
- **对策**：①政策性提法登记断言 5 豁免表（键 = `<相对路径>|<引用>`，必须带理由，禁无理由豁免）；②**推远端前本地 fresh clone 复跑文本守护 ≡ CI 检出**：`git clone . %TEMP%\x` 后在其内跑守护（clone 只有入库文件，无本机噪音）。
- **证据**：2026-09-19 CI run 35379102690 红（4 处死引用）→ 豁免表首批登记 3 处 + 过时注释改写；fresh clone 验证法在推送前又抓到守护自扫自（见 G-30），跳过定义源后 exit 0，第二轮 CI（35380417493）绿。

## G-30 给扫描型守护加豁免表：定义源文件必须跳过自身扫描

- **触发**：扫描型守护自带豁免/例外表，而表键必然是它要匹配的字面量（路径、编号……），且守护的扫描集包含定义源文件自己。
- **症状**：豁免表登记完，fresh clone 验证反而**新增**违规——守护把自己的表键判成命中，自检互搏。
- **根因**：定义源含模式字面量是常态，扫描集含定义源时必然自毙；同构先例 = 断言 6 不扫 `docs/gotchas.md`（G-编号定义源）。
- **对策**：扫描循环对定义源文件直接 `continue`（本仓：断言 5 跳过 `scripts/check_agents_health.ps1` 自身）；新写守护时"定义源不扫自"应进设计而非等撞。
- **证据**：2026-09-19 实测：登记 3 处豁免后 fresh clone 出 3 处 VIOLATION 且全部指向 check_agents_health.ps1 自身；加跳过后 exit 0。

---

## 候选清单（尚未收编——能五段式实证表述才收编，不臆造）

- visual-parity 施工「horizontal 无限宽」egui 布局陷阱（表述不全，待回读 docs/archive/visual-parity.md 取证）。
- visual-parity 施工「hidden 窗口定位失效」（同上）。
- overlay-realign 施工「opa() 刻度错位」（待回读 docs/archive/overlay-realign.md 取证）。
