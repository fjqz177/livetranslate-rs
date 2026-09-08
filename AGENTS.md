# AGENTS.md — LiveTranslate-rs

## 项目定位

Rust 原生实时音频翻译应用 **LiveTranslate-rs**（Python(PyQt6) 原版 LiveTranslate 仅作行为参考，**不再追求 1:1 复刻**）。阶段一（1:1 复刻期，2026-09-05～09-07）已收尾：M0–M6 主体 + 视觉五工作包 + 字体系统 W-1~W-7 + 分发裁决 D-18~D-21 全部落地，313 测全绿，release 单 exe 分发演练通过；当前进入**阶段二（Rust 自有产品化）**：新增能力以产品体验为准，原版有/没有不再是取舍依据，行为差异落档为**新偏差 D-22 起**编号的决策史（`docs/archive/rewrite-research.md` §1.5 顺延）。

- **参考副本 = 工作区内 `LiveTranslate/` 原版代码副本**（gitignored，扁平结构；2026-09-06 用户明确：与 `D:\biancheng\LiveTranslate`、`LiveTranslate-NG` 等外部仓库无关）。**参考而非规范**：改 GUI 前仍可回读副本对应 Python 模块（`main.py`、`subtitle_overlay.py`、`subtitle_window.py`、`control_panel.py`、`vad_processor.py` 等，均在副本根目录），但新功能不必拘泥其行为。
- **文档体系（2026-09-07 重组）**：当前依据 = `docs/`（`README.md` 索引、`distribution.md` 分发路线图、`asr-engine-expansion.md` ASR 扩展）与本文档待办；阶段一决策史全部归档 `docs/archive/`（`rewrite-research.md` 选型/偏差 D-1~D-21/风险 R-1~R-14、`rewrite-plan.md` 施工图、`parity-closure.md` 复刻收口九 WP、`ui-realign.md` 等 GUI 对齐方案），**只读**，修改仅允许加"修订"注记，不作为施工依据。

## 硬性约束（不可违背）

- **纯 CPU**：不碰 CUDA/DirectML/GPU；whisper GPU feature 禁用；MonitorBar GPU 恒 N/A。
- **远程 ASR 已整体裁剪**（remote-whisper 引擎、相关设置键均已删除）。
- 单 exe 分发；配置目录 = `~/.config/livetranslate`（Windows 下字面 home/.config，**不是** %APPDATA%）；`settings.json` 的 `models_dir` 键可指定模型缓存路径。
- 阶段一偏差 D-1~D-21 是决策史不是束缚（见 `docs/archive/rewrite-research.md` §1.5）；新阶段行为差异落档为新偏差，编号自 D-22 起（如 D-22 起指 Qwen3-ASR 等阶段二新能力）。
- 当前仅 Windows 实现（wasapi 采集）；AudioBackend trait 已为跨平台抽象，但 macOS/Linux 后端未实装。

## 构建与测试

```bash
cargo test --workspace            # 全量测试（滚动基线：当前 363 测全绿 + 5 个 ignored = 真模型/真网络探针离线纪律），收工前提
cargo build --release -p lt-app   # 单 exe（现有 ~62MB；onnxruntime.dll + silero_vad.onnx 内嵌，启动解压到配置目录）
cargo run -p lt-app               # GUI 冒烟
```

- **冒烟**：设临时 `LIVETRANSLATE_CONFIG_DIR`，其中 settings.json 必须显式 `models_dir` 指真实模型缓存（如 `C:\Users\fjqz177\.config\livetranslate\models`），否则引擎探测全部失败。
- **原版参照截图**：已入库 `assets/reference/`（zh/en 各 4 张）；生成脚本 `scripts/grab_reference_ui.py` 已不在工作区（scripts/ 现仅 silero_reference.py），需要重拍时从外部原版仓获取。
- 无 CI（`.github/workflows` 尚未创建）。

## 工作区结构（依赖方向 = 分层规则）

`crates/` 八库，依赖链单向：**lt-proto**（事件/命令/数据契约，已冻结：值域扩展如 ASR_ENGINES 增项允许，结构字段/Cmd/Event 增删需评审）→ **lt-i18n**（zh/en 各 521 键）→ **lt-models**（Settings/ModelConfig/模型注册表）→ **lt-pipeline**（wasapi 采集、silero VAD、ORT 内嵌）→ **lt-asr**（ASR worker 子进程 + IPC）→ **lt-translate**（async-openai LLM）→ **lt-ui**（egui 多窗口：悬浮窗/字幕窗/控制面板/日志窗/托盘）→ **lt-app**（装配入口 backend/pipeline/shell；ASR worker 以同 exe `--asr-worker` 自拉起，Job Object 孤儿兜底）。

- `assets/`：i18n yaml、`fonts/`（三 brotli 字体 + OFL 许可）、图标、`reference/` 原版参照截图、`silero_vad.onnx`、`SOURCES.md`（资产来源/sha256）。
- 分层规则：lt-ui **允许**只读依赖 lt-models（注册表/缓存探测）与 lt-translate（bench 直调）——2026-09-06 f266a8b 批次的有意决策（见 lt-ui/Cargo.toml 注释），不得依赖 lt-app；新增 UI 能力**不得扩 lt-proto 契约**（日志经 `LogLine{target}` 回流）；Settings 运行时落盘走 `Cmd::PersistSettings` 由 backend 写（300ms debounce 对齐原版）。

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
11. **edition 2021 的 if-let scrutinee 临时值自死锁**（WP-3 实锤）：`if let Some(x) = shared_lock.lock().unwrap().method() { … } else { 再 lock 同一把 }` 的 MutexGuard 临时值存活到整个 if-let 语句结束（含 else 分支）→ 同线程二次加锁永久死锁。共享锁 + 分支派发一律先 `let v = lock.lock().unwrap().method();` 绑定再分支。
12. **字体 Name 族未注册会 panic**（epaint `FontsImpl::font`）：`FontFamily::Name(族名)` 只允许出现在 `FontsState::resolved`（已注册者）；渲染一律经 `fonts::font_family_for` 取，不得直接构造。

## 约定

- **自主中文 commit**：每个里程碑/任务卡完成且测试全绿即提交，格式 `feat(scope): 中文主题`（沿用历史风格，如 `feat(m4-panel-gaps): …`）。多代理并行期提交前必须 `git status`/`git diff` 复核——历史教训：stash 手术曾把他人暂存文件混入我方提交（af0ff58）。
- **docs 提交时机（2026-09-07 整改归档）**：① 计划/调研/决策文档定稿即独立 `docs(scope)` 提交，且必须先于对应实现提交；② 计划完成标记/提交索引随工作包提交同步，禁止事后补 docs 提交；③ 收尾不用 `git add -A`/`git add .`，逐项显式 pathspec，提交前核对「提交主题 vs 文件清单」；④ 生成副产物不入库——`docs/architecture/`（archify 校验图）与 `docs/ui-audit/` 已 gitignore。
- i18n：zh/en 两份 yaml 必须同步修改。
- **字体策略（D-17，2026-09-07 用户裁决）**：仓库自洽为硬保证——内嵌思源（中英韩）+ 等宽 MonoCJK（chrome）+ 符号 NotoSansSymbols2（✗ 等）三字体覆盖全部 UI 字符（`assets/fonts/`，均 OFL 1.1，SOURCES.md 有来源/sha256），任何系统字体缺失都不方块；系统字体（Consolas/注册表扫描/用户自选）仅锦上添花，**不读取系统符号字体（seguisym 已移出）；系统字体缺失不报错不阻断，回退内嵌**；体积：字体以 brotli 压缩资产入库（~12.7MB），运行时一次性解压（启动 ~100ms 级，字形零损失），解压后 FontData::from_static 零拷贝；行级字体键（style.original/translation_font_family、subtitle.lines[].font_family）空串 = 跟随 `subtitle_font_family`（级联），显式族名 = 独立指定；系统字体列表来自 HKLM+HKCU 注册表扫描（`lt-ui/src/fonts.rs`），选择器在样式页「字体」组与字幕行编辑处（搜索/刷新/预览/缺字提示）；改字体键 → 立即 `fonts::apply_fonts(ui.ctx(), …)`（共享单 Context 全窗口下一帧生效）+ `mark_settings_dirty` 防抖落盘；**渲染侧禁用 `FontFamily::Name` 臆造**——未注册族名经 `font_family_for` 回落 Proportional。
- 子代理分工：general-purpose/Explore 建议设 glm-5.3-flash 做执行/检索；架构承重墙（Win32、下载器、算法移植、CRT/链接问题）由主线程亲自做。

## 当前待办（2026-09-07 截点，已完工项带 ✗ 标记）

- **阶段一收尾（已完工，✗ 标记）**：✗ 对齐收口 WP-1/3/4（2026-09-07 落地，290 测）；✗ 字体系统 W-1~W-7（内嵌思源/等宽/符号 + brotli + 双主旋钮级联 + 系统字体扫描选择器，见 docs/archive/font-system.md）；✗ 视觉五工作包 WP-A~E（285 测）；✗ StartDownload targets 动态化（backend settings 镜像现场重算）；✗ docs 提交规范整改（de0d6d4 已 rebase 拆分为 9c9fd67/5f3be9b/d4c169a，7419115 归档，de0d6d4 不复存在）。
- **分发与用户旅程（docs/distribution.md，D-18~D-21 已裁决）**：暂不公开发布（本地 zip）/ 首启直进主界面（D-19 转正，向导代码保留不接线）/ 检查更新按钮（随公开发布）/ 全模型双源（whisper 打破 always-HF 上 MS 镜像 + hub 缺失回落）。阶段一待施工：**WD-1** 打包脚本、**WD-2** 版本可见性（VERSIONINFO/--version/UI 版本行）、**WD-3** LICENSE+NOTICES+简版 README、**WD-4** whisper 双源实测落地、**WD-5** 二次启动激活已有窗口；**WD-6** 首启轻引导横幅可选待点头；WD-7 CI / WD-8 检查更新归阶段二（D-18）。
- **模型下载链路改造（docs/download-overhaul.md，已完工 ✗）**：✗ DL-1 探测 manifest 化（e1b73ea）/ ✗ DL-2 下载器完整性+快速失败（96aab45）/ ✗ DL-3 进度协议精确化（dff0a5e）/ ✗ DL-4 会话化+取消+落盘归一（92c5d46，F9 同批）/ ✗ DL-5 hub 回落（17f5271）/ ✗ DL-6 杂项（14889a2）/ ✗ clippy 清理（6852da6）；334 测全绿 + release 构建过 + 实机冒烟真实缓存命中。新偏差 **D-22** 探测 manifest 化 / **D-23** 下载可取消（后续新偏差自 **D-26** 起——D-24 下载源、D-25 qwen3 引擎均已占用）。剩余：S1/S6 全程真实网络实机走查（HF 镜像段已随 WP-A 演练覆盖，2026-09-08）。
- **ASR 引擎扩展（docs/asr-engine-expansion.md）**：✗ **WP-A FunASR Nano 实装完工（2026-09-08，r2.2）**——注册表修正（官方 int8 包六件套 + D-24 ms=None）/ `engines/nano.rs` / worker nano 臂 / pipeline 按 key 分派 / `hf_endpoint_for` 端点随所选 hub 接线 / UI（nano 隐藏 pad 滑杆 + hub=ms 镜像 hint）；引擎级验收（中/英/粤转写 RTF 0.10–0.18）+ GUI 冒烟过（详见该文档 §3.5）。✗ **WP-B Qwen3-ASR-0.6B 实装完工（2026-09-08，r3.1）**——目标仓 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`（六件套 941MB，MS 无源走 D-24）/ 注册表 `QWEN3_ASR` 条目（D-25）/ `engines/qwen3.rs`（纯 auto-LID，`set_language` 非 auto 诚实 Unsupported / `NUM_THREADS=3` S0 定案）/ worker 四臂 / pipeline·探测·缺失清单·UI 三值引擎表 / guess_language·标签清理提升 engines/mod.rs 共享；**349 测全绿** + S0 验收（加载 2.9s、线程 1/2/3→RTF 0.459/0.312/0.280、本机 RTF 0.23–0.36、五语 LID 全对）+ GUI 冒烟 worker ready 零告警（详见该文档 §4.8）。**D-25 已登记（新偏差自 D-26 起）**。
- **ASR 子系统加固（docs/asr-hardening.md，已完工 ✗）**：✗ AH-1~AH-10 全部落地（2026-09-08，363 测全绿净增 14）——✗ AH-1 待命态可唤醒（P0，下载完成 SwitchEngine 现可唤醒）/ ✗ AH-2 恢复统一（**D-26**：非 Worker 错误一律 recover，间隙死亡/空窗重建/shutdown kill）/ ✗ AH-3 运行时设置贯通（asr_language/pad 经 TlSwitch 镜像同步）/ ✗ AH-4 interim 代际校验（**D-27**）/ ✗ AH-5 下载完整性（sha256 登记 14/20 文件+Checksum 失败类+total=None 拒绝+no_gzip+磁盘预检）/ ✗ AH-6 分派臂收敛（data.rs qwen3 修复+ASR_ENGINES 全集装配防线测试）/ ✗ AH-7 可观测性五项 / ✗ AH-8 qwen3 段长钳制 15s（**D-28**）+set_* 超时对齐 / ✗ AH-9 fake worker 复用真实 run / ✗ AH-10 文档纠偏（AGENTS 分层规则/基线/SOURCES.md）。剩余：GUI 冒烟脚本 A/B/C 与 T1 qwen3 长样例校准待实机；whisper 其余五档 sha256 待有网络补齐（完成前新偏差自 **D-29** 起）。
- **麦克风监控幽灵条修复（docs/mic-monitor-fix.md，已完工 ✗）**：✗ MC-1~MC-4 全部落地（2026-09-08，f1f5464，368 测全绿）——MIC 条显隐以 UI 启用意图驱动（**D-29**，显式开关/挂起恢复）+ mic_buf 有界化；换引擎/挥动开关不再出幽灵条。
- **SenseVoice 语言恒 auto 修复（docs/sensevoice-language-fix.md，已完工 ✗，D-30）**：✗ SV-1~SV-3 全部落地（2026-09-08，66871a0，369 测全绿净增 1）——引擎层「显式设置 > 模型标签 > 启发式」三优先语言决策 `resolve_language`（sherpa-onnx SenseVoice 输出无语言标签实机取证；auto 时语言恒 "auto"→「同语言免翻译」失灵为主诉，显式语言被过滤整段丢弃为隐性缺陷）；实机探针 6 组全绿 + GUI 冒烟实测 `Same language (zh), no translation`（修复前为白翻译）。新偏差自 **D-31** 起。
- **复刻期遗留按新定位处置（docs/archive/parity-closure.md）**：WP-2 由上面 WP-A 取代；**WP-5/6/7/8 不再默认按复刻执行**——WP-5 托盘菜单与气泡、WP-8 ErrorBanner/全局热键（原版不存在）届时按产品价值裁决；**WP-9 M6 调优与实机实测保留**（启动<2s / 空闲 CPU<1% / 8h 长跑 / 内存回收 / 端到端语音复验，需实机非静音时段）。
- **面板「日志」tab 界面改造（docs/log-tab-redesign.md，已完工 ✗）**：✗ LT-1~LT-6 全部落地（2026-09-08，416bbfd，378 测全绿）——Log 页脱离 panel 页面级 ScrollArea（单滚动条）+ 工具行恒置顶 + 贴底跟随（egui stick_to_bottom + advance_follow 上翻挂起/「回到最新」浮钮）+ 显示 DEBUG 渲染期过滤可回溯 + 复制全部反馈 + 按钮改名「打开日志目录」/ 底部提示行移除（hover 显示当前会话文件）；**D-31 生效**（过滤/滚动交互与原版分道，logwin 同步）；零 lt-proto 契约变更；实机用户截图对照过。
- **UX 三期可选项（docs/archive/ux-feedback.md）**：前缀码改结构化枚举、设置保存失败 UI 流、P2-4 错误译文样式。
