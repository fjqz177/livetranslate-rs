# AGENTS.md — LiveTranslate-rs

## 项目定位

Python(PyQt6) 实时音频翻译应用 LiveTranslate 的 **Rust 1:1 重写**（main 分支施工中；M0–M5.3 已完成：278 测全绿，release 59MB 单 exe 分发演练通过）。

- **复刻权威 = 工作区内 `LiveTranslate/` 原版代码副本**（gitignored，扁平结构；2026-09-06 用户明确：与 `D:\biancheng\LiveTranslate`、`LiveTranslate-NG` 等外部仓库无关）。改 GUI 前先回读副本对应 Python 模块：`main.py`、`subtitle_overlay.py`、`subtitle_window.py`、`control_panel.py`、`vad_processor.py` 等（均在副本根目录）。
- **权威文档**：`RESEARCH.md`（选型结论、刻意偏差 D-1~D-16、风险 R-1~R-13）与 `PLAN.md`（施工图：契约全表 §3、算法规格 §5、经验教训 §6、风险预案 §8、验收清单 §9）。GUI 对齐方案见 `docs/ui-realign-plan.md`（五阶段），走查证据在 `docs/ui-audit/`。

## 硬性约束（不可违背）

- **纯 CPU**：不碰 CUDA/DirectML/GPU；whisper GPU feature 禁用；MonitorBar GPU 恒 N/A。
- **远程 ASR 已整体裁剪**（remote-whisper 引擎、相关设置键均已删除）。
- 单 exe 分发；配置目录 = `~/.config/livetranslate`（Windows 下字面 home/.config，**不是** %APPDATA%）；`settings.json` 的 `models_dir` 键可指定模型缓存路径。
- 刻意偏差按 RESEARCH.md §1.5 执行，不擅自"改进"原版行为。
- 当前仅 Windows 实现（wasapi 采集）；AudioBackend trait 已为跨平台抽象，但 macOS/Linux 后端未实装。

## 构建与测试

```bash
cargo test --workspace            # 全量测试（当前 278 测），收工前提
cargo build --release -p lt-app   # 单 exe（约 59MB；onnxruntime.dll + silero_vad.onnx 内嵌，启动解压到配置目录）
cargo run -p lt-app               # GUI 冒烟
```

- **冒烟**：设临时 `LIVETRANSLATE_CONFIG_DIR`，其中 settings.json 必须显式 `models_dir` 指真实模型缓存（如 `C:\Users\fjqz177\.config\livetranslate\models`），否则引擎探测全部失败。
- **原版参照截图**：新版原版仓 `uv run python scripts/grab_reference_ui.py`；参照图在 `assets/reference/`（zh/en 各 4 张）。
- 无 CI（`.github/workflows` 尚未创建）。

## 工作区结构（依赖方向 = 分层规则）

`crates/` 八库，依赖链单向：**lt-proto**（事件/命令/数据契约，已冻结）→ **lt-i18n**（zh/en 489 键）→ **lt-models**（Settings/ModelConfig/模型注册表）→ **lt-pipeline**（wasapi 采集、silero VAD、ORT 内嵌）→ **lt-asr**（ASR worker 子进程 + IPC）→ **lt-translate**（async-openai LLM）→ **lt-ui**（egui 多窗口：悬浮窗/字幕窗/控制面板/日志窗/托盘）→ **lt-app**（装配入口 backend/pipeline/shell；ASR worker 以同 exe `--asr-worker` 自拉起，Job Object 孤儿兜底）。

- `assets/`：i18n yaml、图标、`reference/` 原版参照截图、`ort/`。
- 分层规则：lt-ui 不得依赖 lt-models/lt-translate；新增 UI 能力**不得扩 lt-proto 契约**（日志经 `LogLine{target}` 回流）；Settings 运行时落盘走 `Cmd::PersistSettings` 由 backend 写（300ms debounce 对齐原版）。

## 已知大坑（改码前必读）

1. **egui 多窗口 repaint 回环**（头号坑，见 ad78047）：egui_winit `EventResponse.repaint` 对输入事件为 true **必须**尊重（否则无节拍窗口点击全死）；但对 `RedrawRequested` 自身也返回 true，照单全收 → 1350fps 自旋、CPU 161%、其他窗口饿死白屏。正确姿势：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }`
2. egui 0.36：`set_inner_size` → `request_inner_size`；`ctx.fonts()` 是闭包 API 不能取 owned；Color32 仅 premultiplied 常量是 const。
3. lt-ui 内 `crate::windows` 模块**遮蔽 windows crate**——引用必须写 `::windows::`。
4. 外部 `MoveWindow` 移动 winit 透明窗口会 DXGI 表面失配白屏——兜底 = `Moved` 事件时以当前尺寸强制 `painter.on_window_resized`（幂等）。
5. **双栈 CRT 已解决勿动**：sherpa-onnx（静态 CRT）与 whisper-rs（/MD）冲突由 `.cargo/config.toml` 的 `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` + `CMAKE_POLICY_DEFAULT_CMP0091=NEW` 解决；`LIBCLANG_PATH` 已固化（whisper-rs-sys bindgen 依赖）。
6. `block_on` 外求值 `tokio::time::timeout` 必 panic；async-openai 0.41 需显式 `_api` feature。
7. tray-icon 0.24 无气泡 API（做气泡需 Shell_NotifyIcon 直调或换库）。
8. 字幕窗 30% 黑底叠白窗呈现的 179 灰是正常 alpha 合成，勿误判为渲染 bug。
9. C 盘满会使测试 TEMP 报 StorageFull、GDI+ Save 失败（假死）——临时把 TMPDIR 指 D 盘。
10. 实机走查时 egui 窗口 PrintWindow 会抓到旧帧，必须全屏截图验收。

## 约定

- **自主中文 commit**：每个里程碑/任务卡完成且测试全绿即提交，格式 `feat(scope): 中文主题`（沿用历史风格，如 `feat(m4-panel-gaps): …`）。多代理并行期提交前必须 `git status`/`git diff` 复核——历史教训：stash 手术曾把他人暂存文件混入我方提交（af0ff58）。
- i18n：zh/en 两份 yaml 必须同步修改。
- 子代理分工：general-purpose/Explore 建议设 glm-5.3-flash 做执行/检索；架构承重墙（Win32、下载器、算法移植、CRT/链接问题）由主线程亲自做。

## 当前待办（2026-09-06 截点）

- ErrorBanner（新版悬浮窗错误分类条）、全局热键（hotkeys_group）、M6 interim 装配接线（算法层 `interim.rs` 已完成 37 测，待 VAD Arc 共享拓扑对齐原版 `_vad_lock`）。
- FunASR Nano 实装（目前仅注册表占位）、StartDownload targets 动态化、CI。
- M6 调优三件（启动<2s / 空闲 CPU<1% / 8h 长跑）与内存回收实测；端到端语音复验（需实机非静音时段）。
