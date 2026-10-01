# 大坑全书（gotchas）

> **定位**：常青参考（docs/README.md 纪律①「常青参考」类）。只收**坑**——五段式：触发 / 症状 / 根因 / 对策 / 证据；规则与约定不收（那些归 AGENTS.md / prompts 卡）。
> **前缀**：G-（slug 派生自 gotchas，头注声明，全局唯一含 archive）。编号连续不复用；出本档引用带路径（如 `docs/gotchas.md` G-13）。
> **与 AGENTS §7 的关系**：§7 只留 5 条「不读会静默踩」的防呆行（G-1/5/9/12/14），其余全在本册（触发词五段式自索引，AGENTS §4 有专门路由行）；§7 引用的每个 G-n 必须实存（check_agents_health.ps1 断言 3）。
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
- **对策**：WP-5 托盘气泡已裁**不做**（D-107，2026-09-28——D-33 原生 Toast 已覆盖提示场景，气泡为重复面）；若未来翻案，`Shell_NotifyIcon` 直调或换库。
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
- **对策（2026-09-19 政策翻转，ADR-20）**：全仓文本文件一律无 BOM 合法 UTF-8（.ps1 不再豁免，既有 9 个 BOM 已剥）；.ps1 由 pwsh 7 执行（ADR-16），误用 powershell.exe 会乱码+语法炸 = 预期快速失败；入库强制 = lt-app/tests/repo_hygiene.rs 三查（BOM / 严格 UTF-8 / 个人路径，ADR-21 cargo test 承载；index LF 由 .gitattributes 锁）；脚本内文本匹配按 G-20 用 PowerShell 原生正则。
- **证据**：check_agents_health.ps1 草稿 2026-09-13 干跑实锤（加 BOM 前解析失败、加 BOM 后同脚本五断言输出全对）。
- **注（2026-09-13 Q9/ADR-16 → 2026-09-19 翻转 ADR-20）**：pwsh-only 后 BOM 只剩「防误用 powershell.exe」一个价值；text-hygiene 起政策翻转——无 BOM 是常态，带 BOM 反被守护拒收（旧「BOM 保留双读兼容、零 churn 不剥」对策废止，沿革见 git 历史）。

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

## G-31 egui `ScrollArea::horizontal` 给内容无限宽：长文本不换行 + 嵌套滚动区宽度错位

- **触发**：egui 里用 `ScrollArea::horizontal` 装可能很长的内容（日志行、changelog 条目、任意文本）；或在滚动区里再嵌套一层滚动区。
- **症状**：长行不换行、横向溢出被裁；嵌套 ScrollArea 内容宽度错位，内层拿不到正确可用宽。
- **根因**：`horizontal` 方向给内容**无限可用宽**，布局器无从换行，文本按单行 galley 排；嵌套时外层的无限宽约束传导进内层，宽度计算失效。
- **对策**：要「横向区域 + 长文本」用 `ScrollArea::horizontal_wrapped`（按可用宽换行）；滚动区不嵌套。
- **证据**：visual-parity 施工修复（2026-09-07）：changelog 条目 `horizontal`→`horizontal_wrapped`（横向溢出）+ 删除嵌套 ScrollArea（宽度错位），docs/archive/visual-parity.md 头注施工注记。

## G-32 悬浮窗透明度刻度错位：`opa()` 0-100 钳死 Qt 0-255 设置值

- **触发**：把设置层的透明度值（Qt 遗产语义 0-255 alpha，如 `bg_opacity=240`、`header_opacity=230`）直接喂给按 0-100 百分比处理的 `opa()`；`window_opacity`（百分比）只乘文字 stroke 不乘容器/头部填充。
- **症状**：任何非满值透明度被 `.min(100)` 全部钳成 100%——悬浮窗纯黑不透明、把底下窗口遮死；调低 window_opacity 只见文字变淡、底板纹丝不动。
- **根因**：①单位错位：两个刻度（0-255 vs 0-100）直接相接，min 钳满；②作用域错位：原版 `setWindowOpacity(0.95)` 作用于**整窗**，Rust 只乘了文字。字幕窗同一套透明背板机制实现正确（`with_alpha`/`/255`），可作对照。
- **对策**：显式双参数预乘 `k = a/255 × w/100`（预乘空间合成）；容器与头部填充都过 `fade(color, opacity, window_opacity)`；透明度值跨层（设置↔渲染）必须显式换算，禁裸传。
- **证据**：docs/archive/overlay-realign.md 根因表（`overlay.rs:29-33` `opa()` `.min(100)`、`:61`/`:88-91` 填充无 opa_pct；对照 `settings.rs:292-294` 注释 0-255、`panel/style.rs:158-160` 正确写法）；实机对照：原版 RMS 37% 时可透视底下文字，Rust 版为纯黑矩形。

## G-33 扫描型守护「按节截取」的正则被正文节标题字面量抢匹配

- **触发**：给扫描型守护写「按节标题截到下一个节标题」的正则；同时被扫描的文档在头注/正文里复述了同一个节标题字面量（如引号中的 `## 归档文档`）。
- **症状**：检查恒红但指错方向——报「缺行：<整表全部条目>」（或反向的静默零命中），不报解析错误，排查被误导到内容层。
- **根因**：正则未锚行首，`## 归档文档` 先命中正文里被引号包住的字面量；lazy `.*?` 到下一个 `\n## ` 截出的「节」是一段散文，表零行。
- **对策**：① 节截取正则一律加 `(?m)^` 行首锚定（节标题必须整行起始）；② 扫描面文档避免复述节标题字面量；③ 此类断言报「全表缺行」时，先怀疑「是不是被别处的字面量抢匹配」，再看内容。
- **证据**：2026-09-24 归档索引外迁（ADR-22）——docs/archive-index.md 头注写了 `## 归档文档` 字面量 → 断言 7 假报「归档索引缺行」全 42 档；已加 `(?m)^` 锚定并在 `scripts/check_agents_health.ps1` 断言 7 就地注释成因。

## G-34 窗口透明栈边界：逐像素 alpha 不可达 + COLORKEY 抖动 + 圆角硬裁毛边

- **触发**：想给悬浮窗/字幕窗/确认窗做逐像素透明或外阴影；用 LWA_COLORKEY 抠色键做透明；用 SetWindowRgn 裁圆角。
- **症状**：① 逐像素 alpha 做不出来（要么整窗均匀半透明，连文字一起变灰）；② 抠色键边缘随窗口位置 ±1 抖动闪变；③ 圆角边缘出现马赛克毛边。
- **根因**：① wgpu-hal `SurfaceTarget::WndHandle` 只报 `CompositeAlphaMode::Opaque`（per-pixel 仅 SwapChainPanel / VisualFromWndHandle 分支），而项目 Painter 只走 WndHandle 路径 → **单窗逐像素 alpha 在当前栈不可达**；② LWA_COLORKEY = 抠色键，与 winit 抖动 ±1 不合；③ SetWindowRgn 是 1-bit 硬裁切，会切过 egui 抗锯齿渐变带。
- **对策**：半透明走**整窗** LWA_ALPHA；圆角走 SetWindowRgn 裁区 + 体填充分 `r=6`、裁区曲线内侧 2px rim 抗毛边；逐像素/双窗列为 P2 远期（不做外阴影，靠描边+半透明浮起）。
- **证据**：D-36 裁定 + docs/archive/subtitle-window-overhaul.md §5.5（wgpu-hal 30.0.1 `src/dx12/adapter.rs:1364-1365` 源码实锤、Painter = `egui_wgpu::winit::Painter`）、悬浮窗修复 2e94bf3、docs/archive/quit-flow-redesign.md（圆角抗毛刺法：`FILL_RADIUS`/`RIM_STROKE`）。
- **版本**：wgpu-hal 30.0.1。

## G-35 退出白帧：wgpu surface 先于窗口销毁 + winit 无边框窗 WS_CAPTION 弹回

- **触发**：退出应用（确认退出 → `event_loop.exit()` → 收尾 drop）；或任何「渲染表面先亡、窗口后亡」的窗口期。启动期同机理反向表现：窗口创建即可见、首帧未画 → 白底上屏。
- **症状**：收尾期主界面最后几帧突然变成一扇带原生标题栏（最小化/最大化/关闭）的纯白 Windows 窗，闪 ~0.25s 后消失；启动期表现为纯白空窗 ~0.7s 后才画出内容。
- **根因**：① `MultiWindowApp` 字段序 painter 先于 windows，Rust 按声明序 drop → egui-wgpu surface（`Painter::set_window` 持 `Arc<Window>`）先销毁、窗口本体后销毁，窗口期内失内容的窗口被**窗口类背景刷子**刷白；② winit 0.30 无边框窗的 WS_CAPTION 恒置位（实测 style=0x16CF0000，靠 WM_NCCALCSIZE 抑制绘制），surface 亡后非客户区重画时抑制失效 → 标题栏弹回。
- **对策**：与销毁时序解耦，**不调字段 drop 序**（surface 持 `Arc<Window>`，重排动不了窗口真身销毁时机）——退出分支在 `event_loop.exit()` 前全窗 `set_visible(false)` 先离屏；启动侧反向同理：全窗隐藏创建，`setup()` 尾先同步画首帧再揭示（app.rs WFC-1/WFC-3，D-100）。
- **证据**：docs/archive/window-flash-cleanup.md §一（2026-09-25 gdigrab 30fps 录屏逐帧 + EnumWindows 实测：白窗矩形 = 悬浮窗 GetWindowRect 原位、title="LiveTranslate"、exstyle 含 LAYERED）。
- **版本**：winit 0.30 / egui-wgpu 0.36.1。

## G-36 fetch_sherpa_libs 版本变量被 tar 探测覆盖：冷缓存路径下载 URL 畸形

- **触发**：CI sherpa 缓存未命中（Cargo.lock 变化 + 上次可命中的缓存过 7 天未访问被 GitHub 回收）→ `scripts/fetch_sherpa_libs.ps1` 走真下载路径；本地/热缓存恒走「已解包秒退」，永远看不到它——2026-09-15（3a0074e）引入即潜伏，首跑冷缓存实锤。
- **症状**：CI「Fetch sherpa-onnx libs」步骤秒红：`curl: (3) URL rejected: Malformed input to a URL function`，URL 被拼成 `https://github.com/k2-fsa/sherpa-onnx/releases/download/vtar (GNU tar) 1.35/sherpa-onnx-v1.13.7-…`（`v$ver` 里整串探测输出混入）。
- **根因**：PowerShell 变量名大小写不敏感（同 G-27 家族陷阱）——脚本第 39 行 `$ver` 存 sherpa-onnx-sys 版本号（Cargo.lock 解析），tar 探测循环内原地复用 `$ver` 接 `tar --version` 首行输出（`tar (GNU tar) 1.35`），其后拼 `$baseUrl` 的 `download/v$ver` 就被污染；秒退路径不用 `$ver` 所以热缓存无恙。
- **对策**：探测循环内改用独立变量名 `$tarVer`（就地注释锁因）；短名禁共用——路径类叫 `*Path`、版本类叫 `*Ver`，同一脚本内一名一义。
- **证据**：2026-09-27 v1.0.1 发布链 ci run 36315366056 失败日志（`gh run view --log-failed`；Cache 步显示 success 但实无缓存可还原 → 冷路径首跑）；修复提交见 git log 本文件。

## G-37 构建期 LNK4098/LIBCMT 警告 = G-5 双栈 CRT 固有产物，无害

- **触发**：`cargo build --release` 链接 lt-app（1.98.1 实测；rustc 1.97 起该 lint 默认 warn）。
- **症状**：`linker stdout: LINK : warning LNK4098: defaultlib 'LIBCMT' conflicts with use of other libs`（中文 link.exe 另多一行「正在创建库 … .lib 和对象 … .exp」进度行；英文链 CI 只显 LNK4098——本地化进度行未被识别）。
- **根因**：sherpa/whisper 静态库带 `/DEFAULTLIB:LIBCMT`（/MT）与 Rust 侧 /MD 冲突（G-5 双栈 CRT 的固有产物，链接器自动择一并警告）；`[workspace.lints.rust] warnings = "deny"` 管不到（lint 带 `ignore_deny_warnings`）。
- **对策**：无害不修（无 LNK2005/2019，动态 CRT 胜出，与 data-lifecycle C3=A 一致）；噪音静音已裁 `[lints.rust] linker_messages = "allow"`（D-103）——**2026-09-28 已落地**（D-114 解禁批①，Cargo.toml `[workspace.lints.rust]`），release 链接输出已净。上游 1.99 起计划归 allow-by-default 的 `linker_info` 会让进度行自动消失（尚未发布，届时复核 Cargo.toml 注记的去留）。
- **证据**：`dumpbin /directives`（静态库带 /DEFAULTLIB:LIBCMT）、`/dependents`（exe 导入 VCRUNTIME140.dll+UCRT）、v1.0.0 发布链 run 35381400025 与 09-18 ci run 35383389311 同警告且全绿。

## G-38 下拉弹层悬停跳变：egui selectable 静止态无帧分支与描边抵消公式不对称

- **触发**：给任何 ComboBox 弹层写非选中项（`ui.selectable_label` / `ui.selectable_value` / `Button::selectable(false, …)`），且 visuals 的 `widgets.inactive.bg_stroke.width > 0`——面板浅色（`panel_visuals` 显式 1.0 灰边）与暗色窗（`stabilize_widget_strokes` 底限 `max(1.0)`，D-32）双双命中；stock egui 两主题该宽均为 0，不触发。
- **症状**：鼠标悬停弹层项的瞬间，该项长高 2×w_inactive px（本仓 = 2px）且变宽同量、框内文字向右下挪 1px、下方所有项与弹层底边被往下顶 2px；移开又缩回。普通按钮 / DragValue / Checkbox / 页签 / 下拉收起态本体均不跳（前者恒全帧、后四者不走该路径）。**盲区注记（评审 D-119 补；2026-09-30 已收编）**：弹层外的内联可选列表行（`egui::Button::selectable` 直调，彼时 4 处）同病灶——D-124 已套同款 scope 归零收编（`style::selectable_button_stable` + `Button::selectable` 入 clippy 禁令），行级细节与 headless 台盲区见 G-41。
- **根因**：egui 0.36 Frame 总尺寸 = content + inner_margin + 2×stroke.width（`containers/frame.rs:13`），`Style::button_style` 以 `inner_margin = button_padding + expansion − bg_stroke.width` 让描边与内边距互相抵消（`src/widget_style.rs:163-165`）——**前提是描边真被画出来**。但 `Button::selectable(false)` = `frame_when_inactive(false)`（`src/widgets/button.rs:77-83`），静止态走 `Button::atom_ui` 的 Retrocompatibility else 分支换 `Frame::new()`（描边 NONE 不画，`button.rs:363-367`），预扣的 w_inactive 无人回补 → 静止项比悬停态矮 2×w_inactive。选中项（SELECTED class）恒全帧，两态对称故恒稳。
- **对策**：用 `style::selectable_stable` / `selectable_value_stable`（lt-ui）替换弹层内 selectable 调用——scope 内归零 `inactive.bg_stroke.width` 再调原生方法（选值版直调上游 `Ui::selectable_value` 单源写回语义），静止/悬停几何像素级相等，悬停蓝框观感不变（D-119）；**禁全局归零** inactive.bg_stroke（会剥掉面板按钮 idle 灰边框、破坏 D-32 底限的半覆盖皮肤语义）；机械防线 = `ui.selectable_label`/`ui.selectable_value` 已入 clippy disallowed-methods（唯一豁免 = 助手本体就地 `#[allow]`），新增弹层项想绕过禁令只能显式豁免留痕。egui 上游若对称化该分支，助手与禁令可退役。内联列表行同族病灶走 `style::selectable_button_stable`（D-124/G-41）。
- **证据**：用户 2026-09-29 控制面板截图两帧（悬停「Fun-ASR-Nano」蓝框坠出弹层底边）；egui 0.36.1 源码行号见根因；本仓 `crates/lt-ui/src/windows/panel/mod.rs panel_visuals()`（inactive.bg_stroke 1.0 #ADADAD）+ `crates/lt-ui/src/style.rs stabilize_widget_strokes`（`max(1.0)` 底限）；headless 等高回归测试 = `crates/lt-ui/src/style.rs` tests（静止/悬停两帧 rect 等高）。
- **版本**：egui 0.36.1。

## G-39 面板窄窗右缘静默裁剪：egui 横排无收缩 + min 窗宽照搬 Qt 布局器前提

- **触发**：egui 窗口（面板等可缩窗）拖窄到行自然宽以下；或 en 文案比 zh 更长的行只按 zh 估算放行；`ui.horizontal` 行内「控件 + 控件 + 长 hint」串联最易命中；**第三形态（D-125 悬浮窗）**：`ComboBox` 选中文本比 `width()` 份额宽（长模型名、en 长语言标签）——哪怕行内其余控件全都不超宽也照样撑爆；**第四形态（D-129 下载卡）**：带状态机的卡片只按默认态（Idle）自证宽度——非默认态（下载中/失败/取消）把「模型名+状态头+进度条+字节对+含 repo 全路径日志行+按钮」六件全塞单行 horizontal。
- **症状**：右缘元素（页签 / 按钮 / 标签 / 下拉）整块或半块消失，无滚动条、无报错、无 panic——tab 条最右「日志」裁半（zh 480 窗即触发）；en 界面下横幅文案贴边双侧裁；第三形态 = 悬浮窗 480 窗下目标/源语言下拉右缘裁剪（en 默认即 +37.1），且上游行溢出把下游 ScrollArea 可用宽一并撑歪（en 消息折行标尺 442→500.7，看似消息没折行实为标尺被上游改写）；第四形态 = 下载卡窄窗下字节对/日志行/按钮整块消失（zh 派生最小宽 509.7 下整行自然宽 1164px，越缘 +666.2），**宽窗（1229 实测截图）下长日志行也把取消按钮推出视口不可达**——此形态右缘裁剪不是窄窗专利。
- **根因**：egui `ui.horizontal` 子元素超出 clip rect 即静默裁剪（不换行不收缩不滚动）；tab 条 / 横幅是 painter 定宽直画连收缩机会都没有；`min_inner_size(480,420)` 照搬原版 Qt `setMinimumSize`——Qt 布局器有 stretch / 换行 / minimumSizeHint 联动，480 在 Qt 下成立，egui 无等价物，最小宽不构成「内容放得下」的保证；headless 冒烟一律 `RawInput::default()`（无 screen_rect ≈ 无限宽画布），溢出在测试里结构性不可见。第三形态根因 = `ComboBox::width` 只是**下限**：`actual_width = (选中文本 galley + 图标) .at_least(width)`（egui 0.36 combo_box.rs:360，默认 Extend 排版 = 无穷宽排版），选中文本变宽下拉跟着长——Qt QComboBox 在 stretch 布局里天然 elide，语义差未对齐。第四形态根因 = 状态机各态共用一行 horizontal 且长文案行（日志/错误详情）单行 galley 不折行；测试盲区 = 溢出断言只渲染 `AppUi::new` 默认 Idle 态，非默认态结构性免检（「全绿 ≠ 对」又一实证）。G-31 是 ScrollArea::horizontal 场景的姊妹条目，普通 horizontal 此前无条目。
- **对策**：三层防线（D-120 立法，D-122 修订 L1/L3）——L1 最小宽**运行时派生**（D-122 推翻 D-120 常数定值：常数追不上字体×语言×DPI 乘积，D-120 的 600 实为 egui 默认字体度量）＝ UI 逐帧实测「tab 条自然宽＋镶边」经 `WinAction::SetPanelMinWidth` 差值发送，宿主动态 `set_min_inner_size`，当前宽不足即抬窗；创建期仅 `PANEL_DEFAULT_WIDTH` 垫底（egui Fonts 惰性初始化，`Context::run` 前不可排版，首帧才拿得到派生值）；L2 行纪律不变 = hint_line 禁与控件同行串联（独立行自动 wrap）、painter 直画文本以 `layout(wrap_width)` 预测行数定高、新增固定宽控件须自证行自然宽 ≤ 派生最小宽可用宽（测试红了拆行，禁回灌抬 min）；L3 溢出断言测试标尺随 D-122 升级 = ctx 装**真实内嵌思源链**（旧测试用默认字体度量即 D-120 失守之根）＋断言视口＝派生公式输出＋分组框边距恒定断言。**第三形态对策（D-125 悬浮窗）**：下拉份额帽 = `allocate_ui_with_layout(Vec2::new(份额, interact_size.y))` 子 ui 钳宽 + `ComboBox::truncate()`（Qt QComboBox elide 同款语义，超份额截断加原生省略号；宽窗份额随 available_width 放大自动完整显示）——**desired 高禁传 0**（0 高 + Align::Center = 内容在整段剩余高度里垂直居中，纵向全线塌位、下方行被挤进零高 ScrollArea）；禁手测文本宽自截（两套标尺，D-122 单源精神同款）；下游 ScrollArea 标尺污染不单独修，上游不溢出即自愈。**第四形态对策（D-129）**：状态机卡片按态排版——非 Idle 三态堆叠行（短行 horizontal、长文案行落纵向布局自动折行、按钮独行），Idle 放得下保持横排；溢出断言扩态覆盖（`panel_download_states_no_horizontal_overflow_at_min_width`：zh/en × 下载中/失败/取消 × 派生最小宽，视口高 2000 防整卡滚出视口使断言空转）；成文纪律 = **有状态机的区域，溢出断言必须逐态渲染**。
- **证据**：用户 2026-09-30 两帧截图（535/480 宽，「日志」tab 半裁，多页右缘控件消失）；en 复现（D-122）：最小窗 600 客户区裁「Logs」~9px——定量锚 = 内嵌思源链 en 派生 617.3 > 600、zh 509.7（用户实机 ui_font=系统 Noto Sans CJK SC 更宽，派生值自动跟随）；溢出清单 = docs/panel-narrow-layout.md §二；定值→派生的裁决史 = docs/panel-adaptive-min-width.md；机械防线 = `crates/lt-ui/src/windows/panel/mod.rs` 溢出断言测试 + 边距恒定断言 + 差值发送节奏测试；第三形态定量锚（D-125）= headless 480 矩阵 en 默认 +49.3 / 长模型名 zh +91.5 / en +145.1，机械防线 = `crates/lt-ui/src/windows/overlay.rs` `overlay_no_horizontal_overflow_at_min_width`（zh/en × 精简/完整 × 三场景 12 组），定稿 = docs/overlay-narrow-width.md；第四形态定量锚（D-129）= headless 派生最小宽矩阵 zh 下载中 +666.2（行 bbox 右 1175.9 vs 视口 509.7）/ zh 失败建议行 +118.9 / en 下载中字节对 +61.5（「114.0 MB / 174.0 MB」半裁 = 用户截图症状）/ en 取消态续传按钮 +34.7，机械防线 = `panel_download_states_no_horizontal_overflow_at_min_width`（红→绿留痕同一测试）。

## G-40 headless 点击连发被判双击：RawInput.time=None 时 egui 按 predicted_dt 推进时钟

- **触发**：headless UI 交互测试（`Context::run_ui` + `RawInput.events` 手工喂 `PointerButton` 帧）里连续两次 click 序列——「再点已选行取消选中」「连点开模态」类用例必经此路；`RawInput::default()` 不带 `time` 字段时命中。
- **症状**：第二次 click 失效或行为诡变——本仓 D-123 首跑即「再点已选行断言取消却仍选中」+ 模态测试「确定按钮不在图元里」双红；单跑每个 click 单独看都成功，连发才坏，极像「写回守卫吞取消」复发（实际是另一回事，易误诊回死代码方向）。
- **根因**：`RawInput.time = None` 时 egui 每帧仅推进 `predicted_dt`（1/60s）——两次 click 序列相距 3 帧 ≈ 50ms < `max_double_click_delay` 0.3s（InputOptions 默认），第二次 click 被判成 `double_clicked`；而本仓行点击语义「双击帧不翻转 toggle」（D-123 §4.3）恰好吞掉第二次点击的取消效果。姊妹坑：egui::Window 开窗首两帧 sizing 收敛未含底部按钮行（内嵌 ScrollArea auto_shrink 与窗口尺寸博弈的瞬时态），断言其按钮上屏须有界 settle（至多 3 帧内出现）；滚轮为离散量低通（未消费量逐帧转 smooth_scroll_delta），喂一帧 MouseWheel 后须补 idle 帧才见滚动；ScrollArea 内容放得下时滚轮无路可滚（max_offset=0），滚动类测试视口必须矮于内容自然高。
- **对策**：测试台显式单调时钟——harness 每帧 `time: Some(t)`、t 步进 1/60s；两次独立 click 之间 `idle(≥25 帧)`（≈0.42s > 0.3s 双击窗）；**要测双击就背靠背连发两次 click**（间隔 2 帧 < 窗口，恰好构成 double_clicked——D-123 的 T8 即此法）。参照实现 = `crates/lt-ui/src/windows/panel/mod.rs` `click_testing::PanelHarness`（含 idle/click/text_center 共用件）。
- **证据**：D-123 首跑失败输出（lt-ui 5 败：三列表再点取消 + 空白点击 + 拖拽滚动全红，根因非写回守卫）；egui 0.36.1 `input_state/mod.rs:376`（`time.unwrap_or(self.time + predicted_dt)`）与 `:117`（max_double_click_delay 0.3）；修复后 PanelHarness 15 测全绿（2026-09-30，全仓 671+0）。


## G-41 面板列表行悬停跳变：G-38 同族行级形态 + headless 台 visuals 盲区

- **触发**：鼠标悬停面板三处可选列表行（翻译模型行 / 字幕页文字行 / 缓存页行）——`egui::Button::selectable(false, …)` 直调的非选中行，且 visuals 的 `inactive.bg_stroke.width > 0`（`panel_visuals` 显式 1.0 / `stabilize_widget_strokes` 底限 1.0，同 G-38 命中面）。
- **症状**：悬停出蓝框瞬间该行 +2px 高、框内文字下移 1px，行下所有内容整体下移 2px（用户截图报案：翻译页 DeepSeek 行，「框以下的所有内容都往下挤了一点」）；点选/取消选中瞬间同跳（选中行恒全帧 vs 未选中静止无帧）。同族三形态：弹层项（G-38，D-119 修）/ **内联列表行（本条，D-124 修）** / 普通按钮（恒全帧，WP-B 已稳，无此病）。
- **根因**：与 G-38 同一公式（`inner_margin = button_padding + expansion − bg_stroke.width`、`Frame::total_margin` 含 stroke），行级路径静止态恒走无帧分支（`frame_when_inactive=false` → `Frame::new()`）——非选中行静止占位 `padding − w_inactive`（预扣无人回补），悬停/按下/选中全帧分支 = `padding`；两态描边宽同 1.0 → 差 2×w_inactive。**姊妹坑（测试台）**：`click_testing::PanelHarness` 此前不注入真机 visuals，跑 egui 默认（w_inactive=0）下两态恰好相等——状态相关几何病对 headless 台整体失明，D-123 全套测试无一能红；headless 台必须逐项对齐真机注入面（`app.rs run_frame` 的 visuals 三件套），与 G-40 时钟盲区并列。
- **对策**：三列表行走 `style::selectable_button_stable`（未选中行 scope 归零 `w_inactive`，**恒进 scope 保 Id 链**——分支化 scope 会让按钮 Id 随选中翻转变层、egui 双击判定认 Id 而丢双击，T8 实测拦下）；`egui::Button::selectable` 入 clippy disallowed-methods（唯一豁免 = 助手本体与 Noninteractive 灰显占位）；`PanelHarness::render` 注入 `panel_visuals` + stabilize + 面板滚动条三件套；禁全局归零 `inactive.bg_stroke`（同 G-38 边界）。
- **证据**：headless 探针先红：行文字 y 116.0→117.0（Δ+1.0）、下方「添加」按钮 y 153.0→155.0（Δ+2.0），与用户截图逐像素吻合；egui 0.36.1 `widget_style.rs:158-165`（逐态内边距公式）与 `containers/frame.rs:327`（total_margin 含 stroke）；修复后 `model_row_hover_does_not_shift_geometry` + `selectable_button_stable_row_geometry_stable_across_states` 绿（2026-09-30，全仓 673+0）。定稿 = `docs/panel-selectable-hover-jump.md`。

## G-42 面板内 egui::Window 二级模态窄宿主裁剪：窗随内容走 + constrain 不缩窗 + Grid INFINITY sizing + 窗只涨不缩

- **触发**：面板拉到最小宽（zh 509.7 / en 617.3）时打开面板内 `egui::Window` 二级模态（编辑模型 ModelEditDialog / 编辑字幕行 LineEditDialog）；模态内有「横排串联多控件」的行或复合控件进 `egui::Grid` 单元格。
- **症状**：模态整块越出宿主左右双缘被裁（截图 1：标题栏横贯全窗、两侧标签/按钮半字消失）；或窗体合围后单行右缘半字消失（截图 2：「元/1M to…」）；最小高 420 下窗体纵向也越缘（按钮行不可达）。无报错无滚动条。
- **根因**（四层叠加，headless 实证）：① `egui::Window` 尺寸随**内容自然宽/高**走（default_width 只是初值，实测 zh 模态窗 531/750 > 视口 509.7），`Area::constrain`（默认 true）**只摆位不缩窗**——窗宽超视口即居中越界双缘裁；② 横排串联行（价格行 7 控件：币种 label+combo+输入/输出 label+DragValue×2+单位文本）自然宽 zh ~518 / en ~592，超窗内容宽即在窗内被裁（clip = 窗体∩视口）；③ `egui::Grid` 列宽 sizing pass 以 **INFINITY 排版单元格**（grid.rs `max_cell_size` 默认无穷）——`font_picker_row`（含 SAMPLE_TEXT 小样 label）进单元格把列撑到 ~620、窗随之 ~750；④ 治③时若用「随 available_width 派生的列帽」会与「窗随内容走」构成**正反馈**：`Resize::begin` 的 `desired = max(desired, last_content_size)` 语义是**窗只涨不缩**（resize.rs 注释自陈防 auto-shrink），实测 697→985 失控，任何 max_width 都钳不住（窗可被内容最小宽顶破）。
- **对策**：模态自适应视口（D-126，承 D-122「拆行适配、禁回灌抬 min」同律）——① 窗体 `max_width(视口−16)` 钳制 + `ScrollArea::max_height` 由硬顶常数改 `min(常数, 视口高−110)`（110 = 标题+按钮行 chrome 预算，测试锁）；② 超宽行拆分（价格行拆两行：币种+单位同行、输入/输出独立行）；③ **整行复合控件不进 Grid**（font_picker_row 移出 Grid 页级渲染，同样式页先例）；④ **禁 available_width 派生列帽/宽度**（正反馈失控，唯一稳态解 = 宽度需求与当前窗宽解耦）；⑤ 显式窗 Id（`Window::id`）——默认 `Id::new(标题文本)`（window.rs:104，实为 `Id::new(Option<Cow>)` 测试侧不可重建）随语言热切换变化会丢窗状态。
- **证据**：用户 2026-10-01 两帧截图；headless 全矩阵（真实思源链 + 派生视口 420 高）：zh ModelEdit 窗 531（右超 +28/左超 +11）、zh LineEdit 窗 750（±120）、en LineEdit 771（±77）、纵向 -20..574；clip_rect 归属实证 = 模态内容图元 clip = 窗体∩视口（`元/1M tok` bbox 右 513.8 > clip 右 509.7）；正反馈失控 = 逐帧 area_rect 697→985。机械防线 = `crates/lt-ui/src/windows/panel/mod.rs` `panel_modals_no_horizontal_overflow_at_min_width`（zh/en × 两模态；断言①窗矩形（`memory.area_rect(显式 Id)`）⊆ 视口横纵双轴 + ②横向逐图元 `bbox ⊆ clip_rect`（**纵向不查**：ScrollArea 内容纵向越缘 = 滚动可达的正常溢出，校准实证逐图元纵向断言全假阳）；跳 blur>0 窗阴影装饰）。
- **版本**：egui 0.36.1。

## G-44 下拉弹层末项描边底边被裁没：弹层 Area 高度 sizing 冻结＝内容高 + ScissorRect 非整缩放取整

- **触发**：任何 ComboBox 弹层在非整数缩放（Windows 125%/150% DPI）实机查看，且弹层高度恰为内容高（sizing pass 精确测量时——典型如选项 ≥3；本仓全部 22 处下拉同病）。用户报案 = 识别页引擎下拉 3 项时末项悬停/选中蓝框缺底边，「3 个或以上才出现」。
- **症状**：悬停/选中项的描边框左/上/右三边可见、底边整条消失；无报错。选项 ≤2 的弹层可能因 sizing 多测出富余而侥幸完好——「3 项以上才犯」是幸存者偏差，不是阈值。
- **根因**：三层叠加——① ComboBox 弹层 Area 高度在 sizing pass 一次性冻结＝内容高，此后每帧滚动区 clip 底与末项框底齐平（headless 实测 3 项 @1.0 slack=+0.000）；② 项描边 = Frame Inside 1px，恰好压在裁剪缘上；③ egui-wgpu `ScissorRect` 对 clip min/max 各自 round，非整缩放下框底越缘 0.2px（@1.25 实测 slack=−0.219）→ 底边整条落在 scissor 外。末项之后弹层还有 menu_margin 白边，肉眼易误判为「框底被弹层边吃掉」。
- **对策**（D-131）：弹层滚动区底部 content_margin 垫 2px——`crate::style::combo_popup_style()` 单源，**一切 ComboBox 弹层一律挂 `.popup_style(combo_popup_style())`**（红线入 style.rs 助手头注，防新调用点绕过）；垫底后 clip 恒比末项框底多 ≥2px（实测 +2.000/+1.781）。回归钉 = `combo_popup_last_item_frame_not_clipped`（3 项 @1.0/@1.25 双缩放 + 2 项静音下拉对照，断言弹层内描边项框底到 clip 缘 ≥1px）。
- **证据**：headless 数值红面：@1.0 slack=+0.000、@1.25 slack=−0.219、2 项对照 −19.000（与用户「3 项以上才犯」逐项吻合）；用户截图放大取证 = 末项蓝框三边俱在、底边整条无；软光栅化（epaint 真实 tessellate + ScissorRect 同款 round）出图证实底边修复后绘出。修复后回归钉全绿（2026-10-01）。

## G-43 内嵌字体链只保 CJK/拉丁/希腊/西里尔：新增含其他脚本的 UI 文案必豆腐（egui 无系统回退）

- **触发**：UI 文案（i18n yaml 值或硬编码）含内嵌三字体覆盖之外的文字脚本——现存案例 = 语言下拉 30 项原生名中的阿拉伯/泰/天城文/希伯来四脚本（`lt_i18n::LANGUAGES`，转录自 Python 原版 i18n.py）；这些语言的转写文本、字幕窗译文、日志行同病。
- **症状**：egui 渲染面（面板/悬浮窗/字幕窗/日志窗）该文字整串显示豆腐块 □□□，无报错无 panic；托盘菜单是 Win32 原生渲染不受影响（同理也不吃内嵌链保护）。
- **根因**：egui `FontDefinitions` 是封闭世界——链上没有的字形直接 .notdef，**无系统字体自动回退**；内嵌三字体（思源 CJK SC / 等宽 Mono VF / Symbols 2）联合覆盖 = CJK/假名/谚文/拉丁/希腊/西里尔，四脚本零字符。旧覆盖红线 `embedded_fonts_cover_all_ui_glyphs` 的字符集是**手抄常量**，语言表原生名从未入册——「UI 字符集清单手工维护」即漏网之缝。
- **对策**（D-127）：链级补字——Noto 四脚本 VF（Arabic/Thai/Devanagari/Hebrew）内嵌进字体链尾，**三条链全挂**（Proportional / Monospace / 命名族链尾——命名族兜底用户系统字体的缺字）；红线改**机械枚举** = 覆盖测试直接遍历 `lt_i18n::LANGUAGES` 全表原生名（`embedded_chain_covers_language_native_names`，新语言进表自动受检）；**整形/RTL 不自修**——egui 0.36 harfrust（HarfBuzz）已就绪，上游 text_layout.rs:1366「once RTL/bidi support is added」注释已过时（实测单方向 run RTL 视觉序正确），只欠字形。
- **证据**：红循环 = LANGUAGES 逐字符 skrifa cmap 过三字体，恰好 ar/th/hi/he 全量缺字、26 项全过（0.71s 确定性红，与用户截图逐项吻合）；harfrust 探针（系统字体预演补字后渲染）= `he - עברית` 输出 ע 最右（RTL 视觉序正确）、`हिन्दी` 6 码点→4 字形（i-matra 重排 + न्द 合体）、`ar - العربية` 混合串前缀 LTR+阿段 RTL 均正确；harfrust 0.12.0 = epaint 0.36.1 `font.rs:361` shaper_data。修复后红线绿（2026-10-01，全仓 676 过 0 败；D-127 提交链 101a920 定稿 → da02410 实现 → ac4cb88 评审收编）。跨方向混排（同一行中文+阿拉伯语交替）未探针，留实机走查。

---


## 候选清单（尚未收编——能五段式实证表述才收编，不臆造）

- ~~visual-parity 施工「horizontal 无限宽」egui 布局陷阱~~ **已收编 G-31（2026-09-22）**。
- ~~overlay-realign 施工「opa() 刻度错位」~~ **已收编 G-32（2026-09-22）**。
- ~~visual-parity 施工「hidden 窗口定位失效」~~ **撤出候选（2026-09-22 回读取证未命中）**：visual-parity.md 全文无此记载、git 史无对应提交（该档施工修复仅 horizontal_wrapped / 删嵌套 ScrollArea / 面板居中三件）——出处仅存于施工会话记忆，无法实证不收编；实机再遇时重新立案。
- ~~构建期 `linker_messages` 警告（1.98.1 实测，2026-09-24 立案）~~ **已收编 G-37（2026-09-28），静音已裁 D-103（施工随代码解禁）**。
