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
   - 点「开始下载」拉取模型（SenseVoice ~250MB、Whisper tiny 32MB ~ large-v3 1.1GB、FunASR-Nano / Qwen3-ASR 各约 1GB；支持断点续传、可取消、失败自动重试）；下载完成引擎自动就绪。
3. **配翻译**：「设置 → 翻译」页填入 OpenAI 兼容接口的 **API 地址**（如 `http://127.0.0.1:1234/v1`，即默认值，对应本地 LM Studio 风格端点）、**密钥**、**模型名**，选目标语言，点「**测试连接**」验证后即可说话出字。

### 界面速览

- **悬浮窗**：实时显示原文/译文/时间戳；行 1 按钮 = 暂停/清除/复制/导出/穿透/置顶/设置/字幕/退出（悬停有文案）；顶部拖条拖动窗口；右键菜单含设置、隐藏。
- **控制面板** 8 个标签页：`VAD/ASR`、`翻译`、`样式`、`字幕`、`基准测试`、`缓存`、`更新日志`、`日志`（均可在悬浮窗点「设置」打开）。样式页有 12 套预设、字体选择（内嵌思源兜底，系统字体锦上添花）、窗口位置复位（含字幕窗）。
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

### 1. 编译机环境清单

| 软件 | 用途 | 安装方式 / 链接 |
|---|---|---|
| **Rust stable**（`x86_64-pc-windows-msvc` 工具链） | 主语言 | [rustup.rs](https://rustup.rs) → `rustup default stable-x86_64-pc-windows-msvc` |
| **VS Build Tools 2019/2022**，工作负载「使用 C++ 的桌面开发」（MSVC + Windows SDK） | whisper.cpp / sherpa-onnx 的 C++ 链接器、Windows SDK | [下载页](https://visualstudio.microsoft.com/visual-cpp-build-tools/) |
| **CMake** | whisper-rs-sys（whisper.cpp）构建 | `winget install Kitware.CMake` 或 [cmake.org/download](https://cmake.org/download/) |
| **libclang（LLVM）** | whisper-rs-sys 的 bindgen（生成 Rust 绑定）需要 | `winget install LLVM.LLVM`（[llvm.org](https://llvm.org)），或 `pip install libclang`（[PyPI](https://pypi.org/project/libclang/)） |

装齐后首次构建约十几分钟（依赖多、要编译 whisper.cpp 与 onnxruntime 绑定）。**注意**：`cargo test` 构建时会需要 `LIBCLANG_PATH` 指向 libclang 所在目录（见下一步），LLVM 也可以只装 VS Build Tools 里的「C++ 桌面开发」自带 LLVM 组件。

### 2. 克隆与 LIBCLANG_PATH 修正

```bash
git clone <你的仓库地址> livetranslate-rs
cd livetranslate-rs
```

`.cargo/config.toml` 中固化着一行本机绝对路径：

```toml
[env]
LIBCLANG_PATH = "C:/Users/fjqz177/AppData/Local/Programs/Python/Python313/Lib/site-packages/clang/native"
```

clone 后**必须**按你的机器调整（Edit 该文件，或设置系统/用户环境变量 `LIBCLANG_PATH`——cargo 的 `[env]` 默认**不覆盖**已存在的环境变量，所以设环境变量即可不改仓库文件；`pip install libclang` 后路径为 `<site-packages>/clang/native`，winget 装 LLVM 后为 `C:\Program Files\LLVM\bin`）。

同文件其余两行（`CMAKE_POLICY_DEFAULT_CMP0091=NEW` + `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`）是**双栈 CRT 对齐的既定解**（sherpa-onnx 预编译静态库为静态 CRT，whisper.cpp 默认 /MD，直接链接会 LNK2005/LNK1169 冲突，M5.1 解决），**不要动**。

### 3. 构建、测试、运行

```bash
cargo test --workspace             # 全量测试：当前基线 399 测全绿 + 5 个 ignored
                                   #（ignored = 真模型/真网络探针的离线纪律，不联网不跑）
cargo build --release -p lt-app    # 单 exe：target/release/livetranslate.exe（约 62MB）
cargo run -p lt-app                # GUI 冒烟
```

收工前提 = `cargo test --workspace` 全绿 + clippy 零告警（lt-ui 的 clippy 基线 20 条、零净增）。

### 4. GUI 冒烟（关键细节）

**方式 A（推荐，验证真实模型缓存命中）**——设临时配置目录，且其 `settings.json` **必须显式写 `models_dir`** 指向已有模型的目录（Git Bash 示例）：

```bash
export LIVETRANSLATE_CONFIG_DIR=/d/tmp/lt-smoke
mkdir -p /d/tmp/lt-smoke
cat > /d/tmp/lt-smoke/settings.json <<'EOF'
{ "models_dir": "C:/Users/<你的用户名>/.config/livetranslate/models" }
EOF
cargo run -p lt-app
```

不写 `models_dir` 时默认模型路径是 `~/.config/livetranslate/models`，临时目录下会全部探测失败（`unavailable`）。

**方式 B**：直接 `cargo run -p lt-app`，模型未缓存时在「设置 → VAD / ASR」页点「开始下载」现场下载。

> debug 构建带控制台窗口（tracing 输出）；release 构建 `windows_subsystem=windows` 无控制台。工作区测试可 `cargo test -p lt-<crate>`；忽略项手动跑：`cargo test --workspace -- --ignored`（需真模型/真网络）。

### 5. 代码结构（分层规则）

workspace 8 crates，依赖链**单向**，改码不得违反：

```
lt-proto → lt-i18n → lt-models → lt-pipeline → lt-asr → lt-translate → lt-ui → lt-app
```

| crate | 职责 |
|---|---|
| `lt-proto` | 事件/命令/数据契约。**已冻结**：值域扩展（如 ASR 引擎增项）允许，结构字段/Cmd/Event 增删需评审 |
| `lt-i18n` | UI 字符串。zh/en 两份 yaml **必须同步修改**（`assets/i18n/zh.yaml` + `en.yaml`，各 521 键） |
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

### 6. 提交与文档纪律

- 里程碑/任务卡完成且测试全绿即**自主中文提交**：`feat(scope): 中文主题`（如 `feat(subtitle-drag): D-37 字幕窗拖动修复`）；文档类 `docs(scope): …`。多代理并行期提交前 `git status` / `git diff` 复核。
- docs 时机：计划/调研/决策文档定稿即独立 `docs(scope)` 提交，且**先于**对应实现提交；计划完成标记随工作包提交同步，**禁止事后补 docs 提交**。
- **禁用 `git add -A` / `git add .`**，逐项显式 pathspec；生成副产物不入库（`docs/architecture/`、`docs/ui-audit/` 已 gitignore）。

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

### 8. 常用命令备忘

```bash
cargo test -p lt-ui                        # 只跑某 crate 测试
cargo test --workspace -- --ignored        # 手动跑 ignored 探针（需真模型/真网络）
cargo clippy --workspace                   # lint
cargo run -p lt-ui --example notify_spike  # 实机验证 Windows 原生通知链路
cargo build --release -p lt-app            # 产出 target/release/livetranslate.exe
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
