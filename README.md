# LiveTranslate-rs

Rust 原生实时音频翻译应用：实时捕获系统声音（可选叠加麦克风输入）→ 本地纯 CPU 语音识别（ASR）→ 经任意 OpenAI 兼容接口翻译 → 悬浮窗 / OBS 字幕窗 / 控制面板实时展示原文与译文。

- **平台**：Windows 10/11 x64（当前仅 Windows 实装，wasapi 采集）
- **纯 CPU**：不依赖 CUDA / DirectML / GPU
- **单 exe 分发**：onnxruntime.dll、silero VAD 模型、三族字体全部内嵌，启动时解压到配置目录
- **本地 ASR 引擎**：FunASR（SenseVoice Small / Fun-ASR-Nano）、Whisper（tiny→turbo 六档量化）、Qwen3-ASR-0.6B
- **翻译模型**：任意 OpenAI 兼容 API —— 本地 LM Studio / Ollama 等，或云端 DeepSeek、硅基流动等
- **界面**：中/英文（跟随系统或手动切换，重启生效）

> 项目现状：阶段二（Rust 自有产品化）。工作区内 `LiveTranslate/` 是 Python 原版的代码副本（gitignored），仅作行为参考，不再追求 1:1 复刻；行为差异决策史见 `docs/archive/rewrite-research.md`（D-1~D-21 为阶段一，D-22 起为阶段二）。

---

## 一、快速使用（最终用户）

### 运行前提

1. Windows 10/11 x64；
2. **VC++ 2015–2022 x64 运行库**：[官方下载](https://aka.ms/vs/17/release/vc_redist.x64.exe)（源码编译机已被 VS Build Tools 自带，最终用户干净机必须装——内嵌的 onnxruntime.dll 动态依赖 MSVCP140/140_1）；
3. 能出声的扬声器/耳机（抓系统音频）；可选麦克风；
4. 下载模型需要网络；翻译需要一个能连通的 OpenAI 兼容接口。

### 三步上手

1. **启动**：双击 `livetranslate.exe`（当前无公开发布渠道，先从源码构建或向维护者索取本地 zip；发布路线见 `docs/distribution.md`）。无首启向导，直接进主界面并自动启动管道；模型未就绪时悬浮窗显示 `unavailable`。
2. **配识别**：「设置 → VAD / ASR」页：
   - 「ASR 引擎」选引擎（funasr / whisper / qwen3）与模型档位；
   - 「音频」默认抓系统声音，勾选「麦克风」叠加麦的声音；
   - 「下载源」国内选 **ModelScope**，国外选 **HuggingFace**（没有 MS 源的模型会自动改走 hf-mirror.com 镜像下载）；
   - 模型未缓存时识别页会显示「下载」按钮，点它走下载管线拉取模型（SenseVoice ~250MB、Whisper tiny 32MB ~ large-v3 1.1GB、FunASR-Nano / Qwen3-ASR 各约 1GB；支持断点续传、可取消、失败自动重试）；下载完成引擎自动就绪。
3. **配翻译**：「设置 → 翻译」页填入 OpenAI 兼容接口的 **API 地址**（如 `http://127.0.0.1:1234/v1`，即默认值，对应本地 LM Studio 风格端点）、**密钥**、**模型名**，选目标语言，点「**测试连接**」验证后即可说话出字。

### 界面速览

- **悬浮窗**：实时显示原文/译文/时间戳；行 1 = 隐藏 / 字幕 / 启停（运行↔暂停）/ 清除 / 紧凑切换 / 设置 / 退出（字幕、清除仅非紧凑模式显示），顶部拖条拖动窗口；行 2 = 穿透/置顶/自动滚动/任务栏 复选框 + 模型/源语言/目标语言下拉；**正文右键菜单** = 复制原文/译文/全部、导出（原文/译文/全部）、清除列表。
- **控制面板** 8 个标签页：`VAD/ASR`、`翻译`、`样式`、`字幕`、`基准测试`、`缓存`、`更新日志`、`日志`（均可在悬浮窗点「设置」打开）。样式页有 14 套预设（+自定义）、字体选择（内嵌思源兜底，系统字体锦上添花）、窗口位置复位（含字幕窗）。
- **字幕窗**（OBS 外挂字幕场景，默认关闭，在「字幕」页启用）：顶条悬停出现 ☰ / 穿透 / 锁定 / 关闭按钮；顶条任意键拖动、正文中键拖动（D-37 手工拖动，不再依赖 winit `drag_window`）；穿透时正文鼠标点击直达应用、顶条可交互，按 Ctrl 临时恢复；拖动自动避让悬浮窗并钳制到工作区。
- **托盘**：暂停/继续、显示隐藏悬浮窗、显示隐藏字幕窗、打开设置、退出。托盘菜单运行在专用线程，打开菜单不冻结主界面；隐藏到托盘会弹 Windows 通知提示仍在运行。
- 界面语言在「VAD / ASR」页底部切换，重启生效。

### 数据与配置目录

全部数据收拢在 `~/.config/livetranslate`（Windows 下是**字面 home 下的 `.config`，不是 %APPDATA%**）：

| 内容 | 路径 | 说明 |
|---|---|---|
| 设置 | `settings.json` | 原子写 + 300ms 防抖；`models_dir` 键可把模型缓存重定向到别处（如另一块盘）；兼容导入原版 `user_settings.json` |
| 模型缓存 | `models/`（或 `models_dir`） | 按 hub 分 `modelscope/`、`huggingface/hub/` |
| 转录 | `transcripts/` | 自动保存（可在「缓存」页关闭） |
| 日志 | `logs/livetrans_<时间戳>.log` | DEBUG 级、按次滚动；面板「日志」页同屏可看 |
| 运行时 | `ort/` | 启动时解压的 onnxruntime.dll，勿动 |

其他：**单实例**（重复启动第二个实例会直接退出）；环境变量 `LIVETRANSLATE_CONFIG_DIR` 可整体覆盖配置目录（测试/便携用）；「下载代理」支持 无/系统/自定义 URL 三种模式。

---

## 二、从源码构建（完整开发流程）

> 本节写给开发者：装机 → 克隆 → 首次构建 → 测试 → 冒烟，以及改代码前要知道的分层规矩、提交纪律与常见排错。改码前务必先读一次 [AGENTS.md](AGENTS.md)（项目定位/硬性约束/已知大坑/当前待办）。

### 1. 装环境（一次性，四件套）

| 软件 | 用途 | 安装 |
|---|---|---|
| **Rust stable**（`x86_64-pc-windows-msvc` 工具链） | 主语言 | [rustup.rs](https://rustup.rs) 后执行 `rustup default stable-x86_64-pc-windows-msvc` |
| **VS Build Tools 2019/2022**，工作负载「使用 C++ 的桌面开发」（MSVC + Windows SDK） | whisper.cpp / sherpa-onnx 的 C++ 链接器与 Windows SDK | [下载页](https://visualstudio.microsoft.com/visual-cpp-build-tools/) |
| **CMake** | whisper-rs-sys 构建 whisper.cpp | `winget install Kitware.CMake` 或 [cmake.org/download](https://cmake.org/download/) |
| **uv** | 构建期 libclang：钉版 18.1.1 装进仓库内 `.venv`，供 whisper-rs-sys 的 bindgen 用（约 25MB，免装整套 LLVM） | `winget install astral-sh.uv` 或 [uv 官网](https://docs.astral.sh/uv/) |

装完自检：`rustc -V`、`cmake --version`、`uv --version` 都能出版本号即可开工。首次构建约十几分钟（要编译 whisper.cpp、跑 bindgen、链接 sherpa-onnx 预编译库）；之后日常构建是增量。**clone 后不需要任何本机路径配置**——libclang 怎么来的见第 2 节。

> 备选路线：机器上已装有默认位置（如 `C:\Program Files\LLVM`）的 LLVM 时，删掉 `.cargo/config.toml` 里的 `LIBCLANG_PATH` 行也能编（clang-sys 会按内置目录自动发现）。uv 路线与 LLVM 路线**二选一，不混用**。

### 2. 克隆 + 首次依赖（两条命令）

```bash
git clone <你的仓库地址> livetranslate-rs
cd livetranslate-rs
uv sync                                                             # ① libclang（bindgen 用，约 25MB）
powershell -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1   # ② sherpa-onnx 预编译库（约 120MB）
```

**① 为什么要 uv sync**：whisper-rs-sys 构建时用 bindgen 生成绑定，bindgen 需要一个 `libclang.dll`。仓库用 uv 把 libclang 钉死（`pyproject.toml` dev 组 `libclang==18.1.1`，`uv.lock` 锁 wheel 哈希），装进仓库内 `.venv`，再由 cargo 配置指过去：

```toml
[env]
LIBCLANG_PATH = { value = ".venv/Lib/site-packages/clang/native", relative = true, force = true }
```

**② 为什么要预取 sherpa 库**：sherpa-onnx-sys 构建期默认从 GitHub Releases 下载 120MB 预编译静态库（缓存落在 `target/`，`cargo clean` 即丢、每次重下）。本仓库改为**预取到仓库内 `.cache/sherpa-onnx/`**，构建时从本地复制：全程零联网、`cargo clean` 不再重下。GitHub 慢/失败时脚本支持任意 Release 镜像：

```powershell
scripts\fetch_sherpa_libs.ps1 -Mirror https://gh-proxy.com/           # 已实测：完整回源 + 支持断点续传
$env:SHERPA_ONNX_MIRROR = "https://gh-proxy.com/"; scripts\fetch_sherpa_libs.ps1
# 其他镜像同理（ghfast.top 等，以实际可用为准）；或用代理：设置 HTTP_PROXY / HTTPS_PROXY 后重跑脚本（curl.exe 自动读取）
```

对应 cargo 配置：

```toml
SHERPA_ONNX_ARCHIVE_DIR = { value = ".cache/sherpa-onnx", relative = true, force = true }
```

两条 `[env]` 的公共语义：`relative = true` = 路径相对配置文件目录解析 → **零机器相关绝对路径**，换机器/重 clone 不用改；`force = true` = 覆盖系统残留的同名环境变量。`.venv/` 与 `.cache/` 均已 gitignore；换机器后重跑这两条命令即可（仅首次各需网络）。

> ⚠️ ② 必须在首次构建前跑：sherpa-onnx-sys 的 build.rs 对 `SHERPA_ONNX_ARCHIVE_DIR` 缺失**不回落联网**（硬报错 "does not contain expected archive"）；忘了跑见第 8 节排错表。

### 3. 构建与测试（质量门）

```bash
cargo test --workspace              # 全量：399 测全绿 + 6 个 ignored（真模型/真网络探针的离线纪律，平时不联网不跑）
cargo build --release -p lt-app     # 单 exe：target/release/livetranslate.exe（约 75.6MB；分发请一并附 VC 运行库说明）
cargo run -p lt-app                 # GUI 冒烟
```

**收工前提（两条都满足才算完成）**：

1. `cargo test --workspace` 全绿；
2. `cargo clippy --workspace` **不新增告警**（存量基线：lt-ui 20 条，零净增）。

- 只跑某个 crate：`cargo test -p lt-ui`（其余类似 lt-proto / lt-models / lt-asr …）；
- 手动跑 ignored 探针：`cargo test --workspace -- --ignored`（需真模型/真网络，**平时不要跑**）。

### 4. GUI 冒烟（两种方式）

**方式 A（推荐）：验证真实模型缓存命中。** 设临时配置目录，且其 `settings.json` **必须显式写 `models_dir`** 指向已缓存模型的目录——不写的话默认模型路径是 `~/.config/livetranslate/models`，临时目录下会全部探测失败（悬浮窗显示 `unavailable`）。

PowerShell：

```powershell
$env:LIVETRANSLATE_CONFIG_DIR = "D:\tmp\lt-smoke"
New-Item -ItemType Directory -Force $env:LIVETRANSLATE_CONFIG_DIR | Out-Null
'{ "models_dir": "C:/Users/<你的用户名>/.config/livetranslate/models" }' |
  Set-Content -Encoding Ascii "$env:LIVETRANSLATE_CONFIG_DIR\settings.json"
cargo run -p lt-app
```

> ⚠️ 编码**必须** `Ascii`：本示例内容为纯 ASCII；`Set-Content -Encoding UTF8` 在 Windows PowerShell 5.1 会写入 BOM，而 `settings_io::load` 直接 `serde_json::from_str` 解析——BOM 会让解析失败并按"无配置"处理（`models_dir` 丢失，全探测失败）。

Git Bash：

```bash
export LIVETRANSLATE_CONFIG_DIR=/d/tmp/lt-smoke
mkdir -p /d/tmp/lt-smoke
cat > /d/tmp/lt-smoke/settings.json <<'EOF'
{ "models_dir": "C:/Users/<你的用户名>/.config/livetranslate/models" }
EOF
cargo run -p lt-app
```

**方式 B（开发期灵活）**：直接 `cargo run -p lt-app`，模型未缓存时在「设置 → VAD / ASR」页点「下载」现场下载。

> debug 构建带控制台窗口（tracing 输出）；release 构建无控制台（`windows_subsystem=windows`）。

### 5. 代码结构（分层规则）

workspace 8 crates，依赖链**单向**，改码不得违反：

```
lt-proto → lt-i18n → lt-models → lt-pipeline → lt-asr → lt-translate → lt-ui → lt-app
```

| crate | 职责 |
|---|---|
| `lt-proto` | 事件/命令/数据契约。**已冻结**：值域扩展（如 ASR 引擎增项）允许，结构字段/Cmd/Event 增删需评审 |
| `lt-i18n` | UI 字符串。zh/en 两份 yaml **必须同步修改**（`assets/i18n/zh.yaml` + `en.yaml`，各 579 键） |
| `lt-models` | Settings 读写、模型注册表（仓库映射/文件清单/体积/sha256）、下载器 |
| `lt-pipeline` | wasapi 采集、silero VAD、onnxruntime 内嵌与解压 |
| `lt-asr` | ASR worker 子进程 + IPC（worker 由主进程用**同 exe `--asr-worker`** 自拉起，Job Object 孤儿兜底） |
| `lt-translate` | async-openai LLM 调用 |
| `lt-ui` | egui 多窗口：悬浮窗/字幕窗/控制面板/日志窗/托盘 |
| `lt-app` | 装配入口（`livetranslate` 二进制 + 单实例互斥量 + worker 分派） |

**红线**：

- `lt-ui` 允许**只读**依赖 `lt-models`（注册表/缓存探测）与 `lt-translate`（bench 直调），这是 f266a8b 的有意决策，**不得依赖 lt-app**；
- 新增 UI 能力**不得扩 lt-proto 契约**（日志经 `LogLine { target }` 回流）；Settings 运行时落盘走 `Cmd::PersistSettings`，由 backend 统一写（300ms 防抖对齐原版）；
- 参考副本 `LiveTranslate/`（Python 原版）仅供参考，新功能不必拘泥其行为。

**新功能从哪改**：新增 ASR 引擎 → `crates/lt-models/src/registry.rs` 登记 + `crates/lt-app/src/main.rs` 加 worker 臂（详见 [docs/asr-engine-expansion.md](docs/asr-engine-expansion.md)）；新增 UI 字符串 → 按红线见 `lt-i18n`；引擎/性能实测 → `docs/asr-hardening.md` 等活跃文档。

### 6. 提交与文档纪律（约定俗成，别破坏）

1. **自主中文提交**：里程碑/任务卡完成且测试全绿即提交，格式 `feat(scope): 中文主题`（如 `feat(subtitle-drag): D-37 字幕窗拖动修复`）；文档类 `docs(scope): …`。多代理并行期提交前 `git status` / `git diff` 复核。
2. **docs 先于实现**：计划/调研/决策文档定稿即独立 `docs(scope)` 提交，且先于对应实现；计划完成标记随工作包提交同步，**禁止事后补 docs 提交**。
3. **不用 `git add -A` / `git add .`**：逐项显式 pathspec，提交前核对「提交主题 vs 文件清单」；生成副产物不入库（`docs/architecture/`、`docs/ui-audit/` 已 gitignore）。

### 7. 改码前必读：AGENTS.md「已知大坑」

[AGENTS.md](AGENTS.md) 里总结了 17 条实测血泪坑，动代码前先读。最高频几条：

1. **egui 多窗口 repaint 回环**（头号坑）：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }`——照单全收会 1350fps 自旋、CPU 161%、其他窗口饿死白屏。
2. **窗口拖动弃用 winit `drag_window()`**（D-37）：Windows 上它走系统标题栏模态循环，被自家哑 `WM_MOUSEMOVE(0,0)` 提前取消，且中键根本不启动循环。字幕窗已是宿主 `SetCapture` + `GetCursorPos` 绝对跟踪，悬浮窗 `WinAction::Drag` 同病待修，新窗口拖动照同法写。
3. **`PROPVARIANT` 带 Drop**：`VT_LPWSTR` 值必须 `CoTaskMemAlloc` 分配（所有权交给 `PropVariantClear`），挂 Rust `Vec<u16>` 指针 = 非 COM 内存释放 → 堆损坏 `0xc0000374`（D-33 实锤）。
4. **托盘菜单 = `TrackPopupMenu` 模态循环**（D-35）：tray-icon 0.24 在托盘窗 WndProc 内直调，托盘必须放专用线程；退出用命令 + ack 限时等待，**勿用 `PostThreadMessageW(WM_QUIT)`**。
5. **事件循环线程禁止同步 MessageBox/模态**（D-33）：`rfd::MessageDialog::show()` 阻塞 winit 循环 + 无 owner 被置顶窗压盖。统一用 egui 内嵌模态或原生通知。
6. **winit 会在 flags 翻转时整体重写窗口 EXSTYLE**（D-34）：手工挂的 `WS_EX_LAYERED`/`WS_EX_TRANSPARENT` 位会被清掉，运行期每帧实测缺位即重挂。
7. **字体名未注册会 panic**：渲染一律经 `fonts::font_family_for` 取族，不得直接构造 `FontFamily::Name`。
8. **C 盘满会假死**：测试 TEMP 报 StorageFull、GDI+ Save 失败——临时把 `TMPDIR` 指到 D 盘。

（其余：edition 2021 if-let 临时值自死锁、egui 0.36 `request_inner_size`/fonts 闭包 API、`crate::windows` 遮蔽 windows crate、`block_on` 外 `tokio::time::timeout` 必 panic、双栈 CRT 勿动等，见 AGENTS.md。）

### 8. 常见问题排错

| 症状 | 原因 → 解法 |
|---|---|
| 构建报 `couldn't find any valid shared libraries ... set the LIBCLANG_PATH` | 没跑 `uv sync`，或 `.venv` 被删 → 执行 `uv sync` |
| 构建报 `SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive` | 没跑预取脚本（这条**不回落联网**）→ 执行 `scripts\fetch_sherpa_libs.ps1` |
| 构建器 120MB 下载慢/失败（GitHub 直连） | 换镜像：`-Mirror https://gh-proxy.com/`（已实测可用、支持断点续传），或设 `HTTPS_PROXY` 走代理 |
| `cmake` 命令找不到 / CMake 报错 | 未装 CMake → `winget install Kitware.CMake` |
| 链接报 LNK2005 / LNK1169（CRT 冲突） | `.cargo/config.toml` 里两行 `CMAKE_*` 被改动（双栈 CRT 对齐，**勿动**） |
| 测试报 StorageFull / GDI+ Save 失败（假死） | C 盘满，默认 TEMP 在 C 盘 → 临时把 `TMPDIR` 指到 D 盘 |
| 模型下载失败 / 极慢 | 在「设置 → VAD/ASR」切下载源（ModelScope 国内快）与代理模式；无 MS 源的模型自动走 hf-mirror.com |
| 启动提示「已在运行」 | 单实例设计 → 从托盘退出，或任务管理器结束旧进程 |
| 切换界面语言不生效 | 语言在「设置 → VAD/ASR」页底部切换，**重启生效** |

### 9. 常用命令备忘

```bash
uv sync                                     # 重建 .venv（删除/换机器后）
powershell -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1   # 重取 sherpa 预编译库（.cache 被删/换机器后）
cargo test --workspace                      # 全量测试
cargo test -p lt-ui                         # 单 crate 测试
cargo test --workspace -- --ignored         # 手动跑 ignored 探针（需真模型/真网络）
cargo clippy --workspace                    # lint 检查
cargo run -p lt-app                         # 本地运行
cargo run -p lt-ui --example notify_spike   # 原生通知链路实机验证
cargo build --release -p lt-app             # 产物 target/release/livetranslate.exe
```

---

## 三、文档索引

| 文档 | 内容 |
|---|---|
| [AGENTS.md](AGENTS.md) | 项目定位/硬性约束/分层规则/已知大坑/当前待办与建议分工（施工第一参考） |
| [docs/README.md](docs/README.md) | 文档体系索引：活跃文档（阶段二施工依据）+ 归档决策史 |
| [docs/distribution.md](docs/distribution.md) | 分发路线图：D-18~D-21 裁决、打包规范、发布手册 |
| [docs/asr-engine-expansion.md](docs/asr-engine-expansion.md) | ASR 引擎扩展：FunASR Nano、Qwen3-ASR 实装详情与验收数据 |
| [docs/archive/](docs/archive/) | 阶段一决策史（选型研究 rewrite-research、施工图 rewrite-plan、偏差 D-1~D-21），**只读** |

## 四、许可

代码 MIT。内嵌第三方组件与资产（onnxruntime、silero-vad、whisper.cpp/ggml 模型、sherpa-onnx、思源/等宽/符号三族字体 OFL 1.1 等）的来源与 sha256 见 [assets/SOURCES.md](assets/SOURCES.md)。
