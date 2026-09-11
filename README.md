# LiveTranslate-rs

Windows 上的实时音频翻译工具：**把系统声音抓下来 → 在本机做语音识别 → 用你自己的 AI 接口翻译 → 实时显示在悬浮窗或 OBS 字幕窗里。**

- Windows 10/11 x64（其他系统暂不支持）
- **纯 CPU**：不需要显卡；语音识别在本机完成，**音频不出网**
- **单个 exe**：双击就能跑，onnxruntime、语音检测模型、字体都打包在里面
- 识别引擎三选一：FunASR（SenseVoice Small / Fun-ASR-Nano）、Whisper（tiny～turbo 六档）、Qwen3-ASR
- 翻译接口随意换：本地 LM Studio / Ollama，或云端 DeepSeek、硅基流动等**任意 OpenAI 兼容接口**
- 界面中英双语

> 本仓库是 Python 原版 LiveTranslate 的 Rust 重写版。工作区里的 `LiveTranslate/` 是原版代码副本，仅供参考，功能不再一一对应。

---

## 一、给用户

### 用之前先装一样东西

**VC++ 2015–2022 x64 运行库**（[下载](https://aka.ms/vs/17/release/vc_redist.x64.exe)，双击安装即可）。Windows 上缺了它 exe 起不来。

### 三步用起来

**1. 双击 `livetranslate.exe`**

没有向导，直接进主界面并自动开始工作。还没下载模型时，悬浮窗显示 `unavailable` 是正常的。

（目前没有公开发布渠道：自己构建，或找维护者要本地 zip。）

**2. 下载识别模型**：「设置 → VAD / ASR」页

- 「ASR 引擎」选引擎和档位——新手推荐 **SenseVoice**（中文好、体积小）；
- 「音频」默认抓系统声音；想同时收自己说话，勾上「麦克风」；
- 「下载源」国内选 **ModelScope**（快），国外选 HuggingFace；
- 点「**下载**」，进度条走完引擎自动就绪。

模型大小参考：SenseVoice 约 230MB；Whisper 从 tiny 约 30MB 到 large-v3 约 1.1GB；Nano / Qwen3 各约 1GB。下载支持断点续传和取消。

**3. 配翻译**：「设置 → 翻译」页

填三样东西——**API 地址**（默认 `http://127.0.0.1:1234/v1`，是给本地 LM Studio 用的）、**密钥**、**模型名**，再选好目标语言，点「**测试连接**」。通了就能对着电脑说话出字。

### 界面在哪

- **悬浮窗就是主界面**：上面一排是 隐藏 / 字幕 / 启停 / 清除 / 紧凑 / 设置 / 退出；下面一排有 穿透、置顶、自动滚动、任务栏，以及模型、源语言、目标语言三个下拉框。正文上点右键可以复制、导出。
- **控制面板**：点悬浮窗上的「设置」打开，共 8 页——VAD/ASR、翻译、样式、字幕、基准测试、缓存、更新日志、日志。
- **字幕窗**（给 OBS 用，默认关闭，在「字幕」页打开）：鼠标移到顶部会出现按钮；顶条或正文中键可以拖动窗口；开穿透后鼠标能直接点到后面的窗口。
- **托盘图标**：暂停/继续、显示隐藏窗口、打开设置、退出。注意：**隐藏到托盘只是收起窗口，程序还在跑**（会弹系统通知提醒）。

### 数据放在哪

全部在 `~/.config/livetranslate`（注意：是**用户主目录下的 `.config`**，不是 `%APPDATA%`）：

| 内容 | 位置 | 备注 |
|---|---|---|
| 设置 | `settings.json` | 里面 `models_dir` 可以把模型存到别的盘 |
| 模型 | `models/` | 按下载源分 `modelscope/`、`huggingface/` |
| 转录记录 | `transcripts/` | 可在「缓存」页关掉自动保存 |
| 日志 | `logs/` | 面板「日志」页同屏也能看 |

程序是**单实例**：重复启动会自己退出。想用别的配置目录（测试或便携），设环境变量 `LIVETRANSLATE_CONFIG_DIR` 指过去即可。

---

## 二、给开发者

> 动代码前务必先读一遍 [AGENTS.md](AGENTS.md)——硬性约束、分层规则、17 条实测踩过的坑都在那。

### 1. 装三样工具

装一次永久有效，除此之外**什么都不用装**：

| 工具 | 干什么用 | 怎么装 |
|---|---|---|
| Rust（MSVC 工具链） | 编译 Rust | [rustup.rs](https://rustup.rs) 装完后执行 `rustup default stable-x86_64-pc-windows-msvc` |
| VS Build Tools | 提供 C/C++ 编译器和链接器（编 whisper.cpp 用） | 安装时勾上「**使用 C++ 的桌面开发**」 |
| uv | 把 libclang 和 cmake 装进项目里——**所以不用另外装 LLVM / CMake** | `winget install astral-sh.uv` |

### 2. 三条命令到能编译

```bash
git clone <仓库地址> && cd livetranslate-rs
uv sync                                                                 # 约 10 秒：把 libclang + cmake 装进项目内 .venv
powershell -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1  # 约 120MB 预编译库，只需一次
```

三条都跑完就能编译了——**首次约十几分钟**（要现场编译 whisper.cpp），之后都是增量：

```bash
cargo run -p lt-app     # 跑起来看看
```

下不动那 120MB（国内直连 GitHub 常见）就加镜像参数重跑：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/fetch_sherpa_libs.ps1 -Mirror https://gh-proxy.com/
```

> 换机器、或不小心删了 `.venv` / `.cache` 时，把上面两条命令重跑一遍就好。项目需要的工具都在仓库内的 `.venv` 里，缓存都在 `.cache` 和 `target` 里，**系统里只有那三样工具**。

### 3. 日常命令

```bash
cargo test --workspace             # 全量测试（收工必跑）。基线：603 通过 + 9 个默认跳过的真模型/真网络探针
cargo test -p lt-ui                # 只测某个 crate（换成 lt-asr / lt-proto 等）
cargo clippy --workspace --all-targets   # 手动跑 lint（提交钩子也会自动跑一遍）
cargo run -p lt-app                # 本机跑 GUI
cargo build --release -p lt-app    # 出单 exe：target/release/livetranslate.exe（约 76MB）
```

想跑 GUI 又不想污染真实配置，用临时配置目录（注意 `models_dir` 必须指向真实模型缓存，否则全部探测失败）：

```powershell
$env:LIVETRANSLATE_CONFIG_DIR = "$env:TEMP\lt-smoke"
New-Item -ItemType Directory -Force $env:LIVETRANSLATE_CONFIG_DIR | Out-Null
'{ "models_dir": "C:/Users/<你的用户名>/.config/livetranslate/models" }' |
  Set-Content -Encoding Ascii "$env:LIVETRANSLATE_CONFIG_DIR\settings.json"   # 必须 Ascii，UTF8 会写 BOM 导致配置解析失败
cargo run -p lt-app
```

### 4. 提交代码

```bash
git config core.hooksPath .githooks    # 每台机器设一次：启用提交前自动检查
git add <具体文件>                      # 不要用 git add -A
git commit -m "fix(overlay): 中文说明"  # 提交信息用中文
```

设过钩子之后，`git commit` 会**自动跑六项检查**（代码格式 / clippy 零告警 / 依赖白名单 / 源码禁令 / 路径卫生 / 死契约），约 10 秒，全过才生成提交；只改文档的提交自动跳过。应急可以用 `--no-verify` 绕过，但 CI 会拦。

另外两条规矩：计划/调研类文档要先于实现单独提交；生成物（`docs/architecture/`、`docs/ui-audit/`）不入库。

### 5. 代码怎么分（10 个 crate，依赖单向）

```
lt-proto → lt-i18n → lt-models → lt-download → lt-audio → lt-asr → lt-translate → lt-orchestrator → lt-ui → lt-app
```

| crate | 职责 |
|---|---|
| `lt-proto` | 事件/命令/数据契约（**已冻结**：加字段要评审，UI 能力不得扩它） |
| `lt-i18n` | 界面文案，`assets/i18n/` 下 zh/en 两份 yaml **必须同步改** |
| `lt-models` | 设置读写、模型注册表、缓存探测（不联网） |
| `lt-download` | 下载器 |
| `lt-audio` | 系统声音采集、VAD |
| `lt-asr` | 语音识别子进程 + 进程间协议 |
| `lt-translate` | 调 LLM 翻译 |
| `lt-orchestrator` | 主流程编排、线程监督、下载管理、日志桥 |
| `lt-ui` | egui 多窗口界面（只依赖 proto/i18n/models） |
| `lt-app` | 组合根：启动、命令路由、worker 分派 |

依赖规则是**机器强制**的（`scripts/check_deps.ps1` + CI），改依赖前先看 `docs/architecture-v2.md` §3.1。

### 6. 卡住了看这里

| 症状 | 怎么办 |
|---|---|
| 构建报 `Unable to find libclang` 或 `cmake` 找不到 | 没跑 `uv sync`，或 `.venv` 被删了 → 跑 `uv sync` |
| 构建报 `SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive` | 没跑预取脚本（这点它不回落联网）→ 跑 `scripts\fetch_sherpa_libs.ps1` |
| 下载 sherpa 库太慢/失败 | 加 `-Mirror https://gh-proxy.com/` 重跑，或设 `HTTPS_PROXY` 走代理 |
| 链接报 `LNK2005` / `LNK1169`（CRT 冲突） | `.cargo/config.toml` 里两行 `CMAKE_*` 被改了 → 改回去（**勿动**） |
| 测试报 StorageFull / 假死 | C 盘满了 → 把 `TMPDIR` 指到别的盘再跑 |
| 模型下载极慢 | 「设置 → VAD/ASR」页把下载源换成 ModelScope |
| 提示「已在运行」 | 单实例设计 → 从托盘退出，或任务管理器结束旧进程 |
| 切换界面语言没反应 | 语言在「VAD/ASR」页底部切换，**重启才生效** |

### 7. 文档去哪看

| 文档 | 内容 |
|---|---|
| [AGENTS.md](AGENTS.md) | 项目定位、硬性约束、分层规则、已知大坑、当前待办（**施工第一参考**） |
| [docs/README.md](docs/README.md) | 文档总索引（活跃文档 + 归档决策史） |
| [docs/distribution.md](docs/distribution.md) | 分发路线：打包规范、发布手册、待办 |
| [docs/architecture-v2.md](docs/architecture-v2.md) | 架构 2.0：十 crate 拓扑与依赖白名单 |
| [docs/archive/](docs/archive/) | 历史决策与已完工工作包（**只读**） |

---

## 三、许可

代码 MIT。内嵌的第三方组件与资产（onnxruntime、silero-vad、whisper.cpp、sherpa-onnx、三族字体等）的来源与校验值见 [assets/SOURCES.md](assets/SOURCES.md)。
