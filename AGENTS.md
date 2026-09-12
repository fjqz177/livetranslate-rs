# AGENTS.md — LiveTranslate-rs

## 项目定位

Rust 原生实时音频翻译应用 **LiveTranslate-rs**（Python(PyQt6) 原版 LiveTranslate 仅作行为参考，**不再追求 1:1 复刻**）。阶段一（1:1 复刻期，2026-09-05～09-07）已收尾：M0–M6 主体 + 视觉五工作包 + 字体系统 W-1~W-7 + 分发裁决 D-18~D-21 全部落地，313 测全绿，release 单 exe 分发演练通过；当前进入**阶段二（Rust 自有产品化）**：新增能力以产品体验为准，原版有/没有不再是取舍依据，行为差异落档为决策号 **D-xx**（全局登记 `docs/decisions.md`：先登记后引用、下一个号=表尾+1、禁预留号段；已回填 D-1~D-37、D-60~D-86 与 ADR-1~14，D-38~59 为弃用空段）。

- **参考副本 = 工作区内 `LiveTranslate/` 原版代码副本**（gitignored，扁平结构；2026-09-06 用户明确：与工作区外的外部仓 LiveTranslate、LiveTranslate-NG（本机具体位置不入库）无关）。**参考而非规范**：改 GUI 前仍可回读副本对应 Python 模块（`main.py`、`subtitle_overlay.py`、`subtitle_window.py`、`control_panel.py`、`vad_processor.py` 等，均在副本根目录），但新功能不必拘泥其行为。
- **文档体系（2026-09-13 看板化重组，纪律全文见 `docs/README.md` 顶部）**：四个区（活跃 `*.md` 健康态 ≤3 / `drafts/` 草稿不入库 / `archive/` 完工决策史只读 / 副产物 gitignore）+ 一条路（草稿→定稿→施工→完工即归档，挪卡连带改索引同一提交）。当前依据 = `docs/README.md`（看板+纪律条文）、`docs/distribution.md`（长期：分发路线图）、`docs/decisions.md`（长期：决策总登记）；未拍板草稿在 `docs/drafts/`（看板 = 本文件「待拍板」小节）；全部工作包历史在 `docs/archive/`（一行一档索引在 `docs/README.md` 归档表），**只读**，修改仅允许加"修订"注记与归档波次路径机械替换，不作为施工依据。
- **codegraph 代码索引可用（2026-09-09 起）**：仓库根 `.codegraph/`（本机生成、不入库）。改码前摸清符号/调用链优先用 MCP `codegraph_explore`（一次返回相关符号源码+调用路径），其次 Grep/Read。

## 硬性约束（不可违背）

- **纯 CPU**：不碰 CUDA/DirectML/GPU；whisper GPU feature 禁用；MonitorBar GPU 恒 N/A。
- **远程 ASR 已整体裁剪**（remote-whisper 引擎、相关设置键均已删除）。
- 单 exe 分发；配置目录 = `~/.config/livetranslate`（Windows 下字面 home/.config，**不是** %APPDATA%）；`settings.json` 的 `models_dir` 键可指定模型缓存路径。
- 历史偏差 D-1~D-86 与 ADR-1~14 是决策史不是束缚（全局索引 `docs/decisions.md`；D-38~59 为预留弃用空段）；新行为差异落档新 D-xx/ADR-x——先登记 `decisions.md` 后引用，下一个号 = 表尾 +1，禁预留号段，被推翻加「→ 被 D-xx 取代」不删行。
- 当前仅 Windows 实现（wasapi 采集）；AudioBackend trait 已为跨平台抽象，但 macOS/Linux 后端未实装。

## 构建与测试

```bash
uv sync                           # 首次/换机器后：仓库内构建工具一次装齐（libclang 18.1.1 供 whisper-rs-sys 的 bindgen；cmake 4.4.3 供编 vendored whisper.cpp）——由 uv 装入仓库内 .venv，cargo [env] LIBCLANG_PATH/CMAKE 均 relative+force 指向，clone 后零本机路径配置、免系统安装 LLVM/CMake
powershell -File scripts/fetch_sherpa_libs.ps1   # 首次：预取 sherpa-onnx 预编译库（120MB GitHub Release，默认联网下载）到 .cache/sherpa-onnx；构建期零联网、cargo clean 不丢；GitHub 慢用 -Mirror <前缀>/SHERPA_ONNX_MIRROR 走 Release 镜像（例 https://gh-proxy.com/，已实测）；build.rs 对 ARCHIVE_DIR 缺失不回落联网（硬报错），首次构建前必须跑
cargo test --workspace            # 全量测试（滚动基线：当前 604+9 测全绿 + 9 个 ignored = 真模型/真网络探针离线纪律；PH-1 后 smoke 探针转 ignored 面，见 docs/archive/path-hygiene.md），收工前提
cargo build --release -p lt-app   # 单 exe（现有 ~76.0MB；onnxruntime.dll + silero_vad.onnx 内嵌；前者启动解压到配置目录，后者内存直载——C5 勘正 2026-09-09）
cargo run -p lt-app               # GUI 冒烟
powershell -File scripts/precommit.ps1   # 提交前门禁（2026-09-11 起）：cargo fmt --all -- --check + cargo clippy --workspace --all-targets -- -D warnings + 四守护脚本，六项与 CI 同源；退出码非 0 禁止提交
```

- **冒烟**：设临时 `LIVETRANSLATE_CONFIG_DIR`，其中 settings.json 必须显式 `models_dir` 指真实模型缓存（如 `~/.config/livetranslate/models`），否则引擎探测全部失败。
- **原版参照截图（2026-09-09 全量重拍）**：`assets/reference/` zh/en 各 10 张——控制面板 7 个标签页（`panel_<lang>.png` = 识别落地页 + `panel_{translation,style,subtitle,benchmark,cache,changelog}_<lang>.png`）+ 悬浮窗/字幕窗/日志窗。**旧一套（2026-09-06）误拍自外部爆改版**（其 panel 侧边栏布局与副本 7 标签页结构完全不符），已作废。截图对象 = 工作区 `LiveTranslate/` 参考副本，出厂缺省状态 + 脚本注入示例内容（api_key 占位，不读 user_settings.json，不写副本）。生成脚本已重建：`scripts/grab_reference_ui.py`（QWidget.grab 程序化截图，无 computer use；解释器用外部原版仓 venv——仅当带 PyQt6 的 Python 解释器用，跑的代码严格限于工作区副本，本机具体路径不入库、见项目记忆）。已知怪癖：Qt 6.11 `grab()` 不渲染 QTextEdit 自身样式表背景（真实显示正常），脚本对 viewport 补同色解决。
- **CI 已建**（`.github/workflows/ci.yml`，W1/R16；2026-09-11 按开发流程重构为两 job：`gate` = 静态面〔`cargo fmt --all -- --check` + 四守护脚本，纯文本扫描不编译，秒级红灯〕、`build` = 编译面〔uv sync + sherpa 缓存 + clippy `-D warnings` + `cargo test --workspace` + `cargo build --release -p lt-app` + `package_release.ps1` 打包 zip 传 Artifact〕；触发面 = 全分支 push〔不限定分支，feature 分支与主线一视同仁〕+ PR + 手动 dispatch）；无 PR 门禁，依赖提交前自查（本地 `precommit.ps1` 六项 = CI `gate` 五项 + `build` 的 clippy，增删须双边同步）。

## 工作区结构（依赖方向 = 分层规则）

`crates/` **十库**（架构 2.0 W0~W6 已合入，拓扑终局见 docs/archive/architecture-v2.md §3.1），依赖链单向：**lt-proto**（事件/命令/数据契约，冻结规则修订案 2026-09-09 随 W2 生效：`PROTO_VERSION` 随结构变更递增；**豁免评审 = 纯新增 Cmd/UiEvent/AppCommand 变体、纯新增 Settings 字段（须 serde default 兼容旧档）、既有枚举增项**；仍须评审（记录入决策史 D-xx）= 删除/改名/改型/改语义任何既有契约项；**新增禁令：任何跨 crate 边界字符串编码协议（前缀/分隔符/哨兵值）一经发现按 P1 立案**——协议无 schema，任何一端改格式都编译通过）→ **lt-i18n**（zh/en 各 579 键）→ **lt-models**（Settings/ModelConfig/模型注册表/cache 探测；**零网络依赖**——W6 下载域分家）→ **lt-download**（W6 新建：下载器 reqwest/sha2/退避/完整性；依赖仅 proto）→ **lt-audio**（wasapi 采集、silero VAD、ORT 内嵌；W3 改名自 lt-pipeline——该 crate 从来只含采集+VAD，原名是拓扑谎言）→ **lt-asr**（ASR worker 子进程 + IPC）→ **lt-translate**（async-openai LLM）→ **lt-orchestrator**（W3 新建：编排域 = 识别/翻译管道 + 线程监督器 + 事件动脉队列侧 + 下载管理 + 日志桥；禁依赖 lt-ui/winit，用户文案经 `Msg` 注入）→ **lt-ui**（egui 多窗口：悬浮窗/字幕窗/控制面板/日志窗/托盘）→ **lt-app**（组合根瘦壳 ~1031 行：boot + 动脉桥 + 命令路由 + 日志订阅 + 单实例消息窗〔W6/R11②：message-only 窗双向激活，`singleton.rs`〕+ worker 入口；ASR worker 以同 exe `--asr-worker` 自拉起〔配置经 stdin 首行，R28/D-76〕，Job Object 孤儿兜底）。

- `assets/`：i18n yaml、`fonts/`（三 brotli 字体 + OFL 许可）、图标、`reference/` 原版参照截图、`silero_vad.onnx`、`SOURCES.md`（资产来源/sha256）。
- 分层规则：lt-ui 内部依赖 = **proto/i18n/models 三条**（E3/ADR-10 起为纯投影 crate——翻译域常量〔DEFAULT_PROMPT/PROMPT_PRESETS/THINKING_STYLES/OVERRIDE_KEYS〕上移 lt-proto，lt-translate 边裁除；不得依赖 lt-app）；lt-translate 依赖 lt-proto（取翻译域常量，E3）；lt-orchestrator 依赖白名单 = proto/models/download/audio/asr/translate（禁 ui/winit/i18n——用户文案经 `Msg` 注入，W3）；值域判定一律走类型化透镜（E2/D-79：`Settings::engine_key()/hub()/proxy_mode()` + `EngineKey/Hub/ProxyMode`——域内禁止 `== "funasr"` 类字面量比较，lt-asr/lt-models/lt-app worker 分派为单点分派边界豁免）；新增 UI 能力**不得扩 lt-proto 契约**（日志经 `LogLine{target}` 回流；冻结规则修订案已于 W2 生效——纯新增类型化变体豁免评审，见本文档首段与 docs/archive/architecture-v2.md §3.3）；Settings 运行时落盘走 `Cmd::PersistSettings`（W4 起 shell 直排：保存 + 发布总线；300ms debounce 对齐原版；E1-4 起各命令的草稿写入唯一落点 = shell `apply_settings_side_effects` 纯函数，UI 只发命令不预写）。**拓扑终局以 docs/archive/architecture-v2.md §3.1 白名单 + docs/archive/architecture-v2-improvements.md（E1~E6 收尾）为准。**

## 已知大坑（改码前必读）

1. **egui 多窗口 repaint 回环**（头号坑，见 ad78047）：egui_winit `EventResponse.repaint` 对输入事件为 true **必须**尊重（否则无节拍窗口点击全死）；但对 `RedrawRequested` 自身也返回 true，照单全收 → 1350fps 自旋、CPU 161%、其他窗口饿死白屏。正确姿势：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }`
2. egui 0.36：`set_inner_size` → `request_inner_size`；`ctx.fonts()` 是闭包 API 不能取 owned；Color32 仅 premultiplied 常量是 const。
3. lt-ui 内 `crate::windows` 模块**遮蔽 windows crate**——引用必须写 `::windows::`。
4. 外部 `MoveWindow` 移动 winit 透明窗口会 DXGI 表面失配白屏——兜底 = `Moved` 事件时以当前尺寸强制 `painter.on_window_resized`（幂等）。
5. **双栈 CRT 已解决勿动**：sherpa-onnx（静态 CRT）与 whisper-rs（/MD）冲突由 `.cargo/config.toml` 的 `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` + `CMAKE_POLICY_DEFAULT_CMP0091=NEW` 解决；`LIBCLANG_PATH` 与 `CMAKE` 都由 uv 管理（钉版 `libclang==18.1.1` / `cmake==4.4.3` 装进仓库内 `.venv`，cargo `[env]` `relative=true`+`force=true` 指向它，见根目录 pyproject.toml）——首次构建前必须 `uv sync`，勿改回本机绝对路径、勿改回依赖系统 LLVM/CMake（data-lifecycle ③ 已根治）。
6. `block_on` 外求值 `tokio::time::timeout` 必 panic；async-openai 0.41 需显式 `_api` feature。
7. tray-icon 0.24 无气泡 API（做气泡需 Shell_NotifyIcon 直调或换库）。
8. 字幕窗 30% 黑底叠白窗呈现的 179 灰是正常 alpha 合成，勿误判为渲染 bug。
9. C 盘满会使测试 TEMP 报 StorageFull、GDI+ Save 失败（假死）——临时把 TMPDIR 指 D 盘。
10. 实机走查时 egui 窗口 PrintWindow 会抓到旧帧，必须全屏截图验收。
11. **edition 2021 的 if-let scrutinee 临时值自死锁**（WP-3 实锤）：`if let Some(x) = shared_lock.lock().unwrap().method() { … } else { 再 lock 同一把 }` 的 MutexGuard 临时值存活到整个 if-let 语句结束（含 else 分支）→ 同线程二次加锁永久死锁。共享锁 + 分支派发一律先 `let v = lock.lock().unwrap().method();` 绑定再分支。
12. **字体 Name 族未注册会 panic**（epaint `FontsImpl::font`）：`FontFamily::Name(族名)` 只允许出现在 `FontsState::resolved`（已注册者）；渲染一律经 `fonts::font_family_for` 取，不得直接构造。
13. **windows crate 的 `PROPVARIANT` 带 Drop**（`extensions/Win32/System/StructuredStorage.rs` `impl Drop` → `PropVariantClear` → 对 `VT_LPWSTR` 的 `pwszVal` 调 `CoTaskMemFree`）：挂 Rust `Vec<u16>` 指针 = 非 COM 内存被释放 → 堆损坏 `0xc0000374`（D-33 notify_spike 实机反复取证 + 修复）。`VT_LPWSTR` 值必须 `CoTaskMemAlloc` 分配，所有权交给析构（`PropVariantClear`）释放。
14. **事件循环线程禁止同步 MessageBox/模态**（D-33 纪律）：`rfd::MessageDialog::show()` = `MessageBoxW(owner=NULL)` 同步模态——阻塞 winit 事件循环（假死）+ 无 owner/非置顶可能被置顶窗压住（"弹了看不见"）+ 托盘事件仅 `EventLoopProxy` 投递导致串行。一律用 egui 内嵌模态（`crates/lt-ui/src/windows/confirm.rs`）或原生通知（`crates/lt-ui/src/notifications.rs`）。
15. **winit 会在 flags 翻转时整体重写窗口 EXSTYLE**（`WindowFlags::apply_diff`，window_state.rs:395-410：`SetWindowLongW(GWL_EXSTYLE, …)` 按自身 flags 表重建）——**手工经 `SetWindowLongPtrW` 挂的位（如 `WS_EX_LAYERED`/`WS_EX_TRANSPARENT`）会被清掉**（D-34 实锤：set_visible 翻转后 LAYERED true→false）。对策：层属性不得只靠创建时设置 + 缓存比对，运行期**每帧实测缺位即重挂**（`window_has_layered`/`set_window_transparent` 状态在 host 侧维持时同理）；任何手工窗口位都要考虑该清理路径。
16. **tray-icon 0.24 托盘菜单 = TrackPopupMenu 模态循环：托盘必须放专用线程**（D-35 实锤）：tray-icon 在托盘窗 WndProc 内直调 `TrackPopupMenu`（tray-icon-0.24.2 platform_impl/windows/mod.rs:542-558），在调用线程跑嵌套模态循环——托盘建于 winit 事件循环线程（旧注释「必须在主线程调用」是错的）时**点开菜单即把 winit 主循环挂死**（整 UI 冻结、管道正常）。对策：托盘图标/菜单/图标与文字更新全放**专用线程 + 自带 GetMessage 泵**（`lt-tray`，docs/archive/tray-menu-blocking-fix.md）；主线程只经 mpsc 通道 + `PostThreadMessageW(WM_APP+1)` 唤醒发命令；**勿用 `PostThreadMessageW(WM_QUIT)` 退出**——WM_QUIT 会被 TrackPopupMenu 嵌套循环先消费、外层泵永挂，改命令 + ack 限时等待。
17. **winit `drag_window()` 不可靠（D-37 实锤）：字幕窗拖动弃用，改手工拖动**：winit 0.30.13 的 `drag_window` 在 Windows = `PostMessageW(WM_NCLBUTTONDOWN, HTCAPTION)` 走系统标题栏模态循环——①其 WndProc 在收到该消息后**先投递哑 `WM_MOUSEMOVE(0,0)`**（event_loop.rs:1226-1227，注释自陈「cancels the modal loop early」），LAYERED+TRANSPARENT 轮询窗上探针实测 0 位移/偶发事件循环挂死；②文档明确「Moves the window with the left mouse button」——**中键拖动根本不会启动循环**。对策（`WinAction::SubtitleDragStart/End` + `update_subtitle_drag`）：egui 只报拖动起止，宿主 `SetCapture` + `GetCursorPos` 绝对跟踪 `set_outer_position`（原版 Qt `mouseMoveEvent move(globalPos - drag_pos)` 语义，任意按键可用）；拖动期间 `sub.dragging`=true 令 100ms 穿透轮询豁免恒非穿透；拖动中隐藏窗口必须 `ReleaseCapture` 收尾（否则输入被吞在隐藏窗）。**悬浮窗同病已修（D-71，a289e5e）**：`WinAction::Drag` 删除 → `OverlayDragStart/End` 同法（`update_overlay_drag` 每帧推进）。

## 约定

- **自主中文 commit**：每个里程碑/任务卡完成且测试全绿即提交，格式 `feat(scope): 中文主题`（沿用历史风格，如 `feat(m4-panel-gaps): …`）。多代理并行期提交前必须 `git status`/`git diff` 复核——历史教训：stash 手术曾把他人暂存文件混入我方提交（af0ff58）。
- **docs 纪律（2026-09-13 定稿，唯一真源 = `docs/README.md` 顶部纪律条文）**：① 立档判据：有要留档的裁决（将发 D-xx/ADR-x）或多步施工验收清单才立档；小修小补不立档不给号，commit message 说清。② 定稿独立 `docs(scope)` 提交且先于实现；草稿只在 `docs/drafts/`（gitignore），定稿 = mv + git add（未跟踪不能 git mv）；D 号随定稿发并同提交登记 `docs/decisions.md`。③ 收口提交一气呵成：标完工 + git mv archive/ + README 移行 + decisions.md 落档路径更新 + 全仓旧路径替换（grep 复盘，含 .rs 注释与本文件）+ AGENTS 该包收敛（无遗留整段删，有遗留压一行，清零即删）+ 收口三问（`ls docs/` 健康吗 / decisions.md 漏行吗 / 表和文件对上没）。④ 决策编号：全局 D-xx（产品/行为）与 ADR-x（架构）先登记后引用；局部 = 文档专属前缀-号（slug 派生、头注声明、全局唯一含 archive），WP/W/R/C/S/H/E/M/DEC 禁作前缀，局部号出文档必带路径；用户裁决逐条 D-xx 不用字母。⑤ 工作包指针行格式：`- **xxx**（状态）：一句话。依据 docs/xxx.md；遗留：…`；待办区三小节 = 待拍板/施工中/遗留。⑥ 收尾不用 `git add -A`/`git add .`，逐项显式 pathspec；生成副产物不入库（`docs/architecture/`、`docs/ui-audit/` 已 gitignore）。
- **提交前门禁（2026-09-11 起单一入口）**：提交前跑 `powershell -File scripts/precommit.ps1`——顺序六项、任一非 0 禁止提交：①`cargo fmt --all -- --check`（2026-09-11 新增；起因=61b651d 全量收口后 W6/D-83/D-85 等批次手写格式回漂 57 文件）②`cargo clippy --workspace --all-targets -- -D warnings`（W7 起 CI 已是 gate）③`check_personal_paths.ps1` 路径卫生（入库文件禁个人绝对路径；用户名经 `$env:USERNAME` 动态匹配不写字面；`C:\Windows`/`C:\Program Files` 系统级确定性豁免；docs/archive 仅告警）④`check_deps.ps1` §3.1 依赖白名单 ⑤`check_guards.ps1` §6.2 五组源码禁令（裸 spawn、直发 proxy、字符串协议、panic hook 越位、契约旁路）⑥`check_dead_contract.ps1` 死契约。**git 钩子 `.githooks/pre-commit` 已备**（版本化、`.gitattributes` 钉 LF）：clone 后一次性 `git config core.hooksPath .githooks` 启用，暂存区含 `.rs`/`Cargo.toml`/`.cargo/**` 才触发（纯 docs/资产提交放行），`--no-verify` 仅限应急且 CI 仍拦。**`cargo test --workspace` 不在此列 = 收工门禁**（见上方构建与测试）。
- i18n：zh/en 两份 yaml 必须同步修改。
- **字体策略（D-17，2026-09-07 用户裁决）**：仓库自洽为硬保证——内嵌思源（中英韩）+ 等宽 MonoCJK（chrome）+ 符号 NotoSansSymbols2（✗ 等）三字体覆盖全部 UI 字符（`assets/fonts/`，均 OFL 1.1，SOURCES.md 有来源/sha256），任何系统字体缺失都不方块；系统字体（Consolas/注册表扫描/用户自选）仅锦上添花，**不读取系统符号字体（seguisym 已移出）；系统字体缺失不报错不阻断，回退内嵌**；体积：字体以 brotli 压缩资产入库（~12.7MB），运行时一次性解压（启动 ~100ms 级，字形零损失），解压后 FontData::from_static 零拷贝；行级字体键（style.original/translation_font_family、subtitle.lines[].font_family）空串 = 跟随 `subtitle_font_family`（级联），显式族名 = 独立指定；系统字体列表来自 HKLM+HKCU 注册表扫描（`lt-ui/src/fonts.rs`），选择器在样式页「字体」组与字幕行编辑处（搜索/刷新/预览/缺字提示）；改字体键 → 立即 `fonts::apply_fonts(ui.ctx(), …)`（共享单 Context 全窗口下一帧生效）+ `mark_settings_dirty` 防抖落盘；**渲染侧禁用 `FontFamily::Name` 臆造**——未注册族名经 `font_family_for` 回落 Proportional。
- 子代理分工：general-purpose/Explore 建议设 glm-5.3-flash 做执行/检索；架构承重墙（Win32、下载器、算法移植、CRT/链接问题）由主线程亲自做。

## 当前待办（2026-09-13 重组为三小节：细节只活在文档里，本区只当指针；一行一包、清零即删）

### 待拍板（等用户裁决；草稿在 docs/drafts/ 不入库）

- 待拍板：**ASR 模型选型**——近一年开源模型调研（FireRed2-CTC / Cohere-14lang / Dolphin / 标点闸门候选，A~H 清单）（docs/drafts/asr-model-survey-2026.md）
- 待拍板：**开发配置审计**（docs/drafts/dev-config-audit.md）
- 待拍板：**增量 ASR 改造**（docs/drafts/incremental-asr-overhaul.md）
- 待拍板：**CI 单 job 化方案**——2026-09-12 八原则 review 交付（无草稿，结论在会话记忆；未点头禁动工；附 tag 发布 / dependabot / PR 门禁三小项）
- 待拍板：**跨平台分期 ①~⑤**——2026-09-11 可行性评估（无草稿，结论在会话记忆；P0 = 宿主 trait 化）
- 待拍板：**llm 遗留⑪**——规则 4/5 偏离可见（docs/archive/llm-api-round2.md）
- 待拍板：**UX 三期可选项**——前缀码枚举化 / 设置保存失败 UI 流 / 错误译文样式（docs/archive/ux-feedback.md）
- 待拍板：**PH-6**——参考图 GPU 型号中性化重拍（可选）（docs/archive/path-hygiene.md）

### 施工中

- （无）

### 遗留（完工包的实机走查与未了项，一行一包，清零即删）

- 架构 v2/2.1：实机走查 11 项 + WP-9 性能预算（后续单独方案）（docs/archive/architecture-v2.md §6.4）；W5 走查 6 项——悬浮窗拖动/字幕窗拖动穿透回归/导出保存框/背景图选择框/设备下拉/Monitor 条（同文档 W5 节）
- translator（D-85）：实机走查 13 项（docs/archive/translator-probe-hotswap.md §6.2）
- model-trust（D-83）：实机走查——改坏一个模型文件应自动隔离+重下+装载（docs/archive/model-trust-repair.md）
- context-turns（D-84）：实机走查 4 项（docs/archive/context-turns-ui.md）
- asr-hardening：GUI 冒烟 A/B/C + T1 qwen3 长样例校准（whisper 六档 sha256 已全量登记）（docs/archive/asr-hardening.md）
- download-overhaul：S1/S6 全程真实网络走查（docs/archive/download-overhaul.md）
- 交互细节走查：hide-quit（D-33）通知视觉 / hide-transparency（D-34）隐藏→托盘重显半透明 / tray（D-35）菜单打开期间主界面出帧 / button-press（D-32）长按 ≥1s 零位移 / 字幕窗 D-37 与悬浮窗 D-71 拖动（各自归档文档）
- 复刻期：WP-9 M6 调优（启动<2s / 空闲 CPU<1% / 8h 长跑 / 内存回收 / 端到端）；WP-5 托盘气泡、WP-8 热键届时按产品价值裁决（docs/archive/parity-closure.md）
- distribution：WD-6 首启横幅待点头；WD-7 tag→Release、WD-8 检查更新随公开发布推进（docs/distribution.md）
