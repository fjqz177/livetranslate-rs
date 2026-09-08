# 数据生命周期与足迹盘点（开发 → 分发 → 运行 → 卸载）

> 调研日期 2026-09-08，基线 commit `0edad27`（工作区 clean）。方法：5 个并行只读研究代理对 `crates/` 全量代码取证（结论均带 file:line）+ 本机实测核账（真实配置目录、注册表、开始菜单、PE 导入表解析），落盘点封闭清单经 4 个代理独立 grep 交叉一致。**本文是事实盘点，不是施工计划**；整改候选见 §8，待裁决后按新偏差（D-37 起）立项。

---

## §0 总速览

| 阶段 | 一句话结论 |
|---|---|
| 开发环境 | clone **≠ 零配置**：硬前置 Rustup(MSVC)+VS Build Tools+CMake+LLVM 四件套，`.cargo/config.toml` 的 `LIBCLANG_PATH` 固化为本机绝对路径必须处理；首次构建需联网（657 crates + ~115MB sherpa 预编译库） |
| 日常命令 | `cargo test --workspace` 全绿=收工前提（6 个 ignored 真模型/真网络探针默认不跑）；GUI 冒烟必须设 `LIVETRANSLATE_CONFIG_DIR` + settings 显式 `models_dir`；`cargo test` 不污染真实配置，**裸 `cargo run` 会** |
| 分发产物 | 单文件 `livetranslate.exe`（72.2 MiB，内嵌 ~33.2MB 资产，无旁置文件）；但干净 Win10/11 **需装 VC++ 2015-2022 x64 Redist**（PE 导入表实锤，`distribution.md`「无 VC 运行库依赖」断言是错的）；WD-1~WD-5 未施工，当前分发形态=手工拷裸 exe |
| 运行足迹·文件 | 全部集中一个根目录 `~/.config/livetranslate`（本机实测 2.26GB，其中模型 2.2GB）；日志每次启动新建、**无保留策略无限累积**；`models_dir` 重定向只带走模型树 |
| 运行足迹·系统 | 开始菜单一个 .lnk，**仅此一处**：注册表零写入、TEMP 零足迹、无自启、无崩溃转储、无全局钩子 |
| 运行足迹·网络 | 出口仅两处：模型下载（HF/镜像/MS）+ LLM API（用户自配端点）；**音频零出网**；零遥测、无更新检查 |
| 卸载 | 无安装器故无卸载器：退出应用 → 删配置根（~2.3GB）→ 删 exe → 删 lnk，四步彻底干净；顺序上**先删 exe 再删 lnk**（lnk 只在发通知时复活） |
| 整齐度 | 单根自包含度高（根外足迹仅 lnk+用户导出两处）；`~/.config` 为有意裁决（D-9）但从未写给用户看；主要瑕疵=logs/transcripts 无限累积 + 3 处文档断言与实证冲突 |

**两个受众各拿走什么**：

- **开发者必读**：§1 预装清单与 LIBCLANG_PATH、§2 命令与冒烟纪律、§2.4 开发副作用边界。
- **用户必读（喂给 WD-3 简版 README）**：VC Redist 前置（§3.2）、数据都在 `~/.config/livetranslate` 模型占 GB 级（§4）、联网去向（§5.6）、卸载四步（§6）。

---

## §1 干净机器开发环境搭建（A）

> 【一眼结论】装好 Rustup(MSVC) + VS Build Tools(C++ 工作负载) + CMake + LLVM，处理掉 `LIBCLANG_PATH` 一行，联网即可构建。NASM 不需要；工具链无版本锁定；whisper.cpp 免下载（vendored），sherpa 需从 GitHub 拉 ~115MB 预编译库。

### 1.1 预装软件清单（逐个排查推导）

| 软件 | 硬依赖？ | 被谁需要 | 证据 |
|---|---|---|---|
| Rustup + `x86_64-pc-windows-msvc` 工具链 | 是 | rustc 本体 + 全部链接 | 无 rust-toolchain.toml（test -f 实证，不锁版）；workspace edition 2021（根 Cargo.toml:16）；开发机参考版本 rustc 1.98.1 |
| VS Build Tools「使用 C++ 的桌面开发」（MSVC v14x + Windows SDK） | 是 | cl.exe/link.exe（rustc 链接、whisper.cpp）；SDK 的 rc.exe（lt-app build.rs 经 winresource 嵌图标，**构建失败仅 warning 不阻断** crates/lt-app/build.rs） | whisper-rs-sys build.rs `use cmake::Config` |
| CMake | 是 | whisper-rs-sys 构建 vendored whisper.cpp（cmake crate 需 cmake 在 PATH） | whisper-rs-sys-0.15.0/build.rs:6 |
| LLVM/libclang | 是 | whisper-rs-sys 的 bindgen 0.72 | whisper-rs-sys Cargo.toml.orig:41-46 |
| NASM | **否** | 全仓零引用（whisper.cpp MSVC x64 走 intrinsics） | build.rs grep nasm 零命中 |
| Git | 仅 clone 用 | Cargo.lock git 依赖 = 0 | `grep -c 'source = git'` = 0 |
| Python | 否 | 仅历史巧合：LIBCLANG_PATH 恰指向 pip clang wheel | 见 1.2 |

### 1.2 仓库固化配置：clone ≠ 零配置（关键发现）

`.cargo/config.toml` 全文仅 `[env]` 三条，**无** `[build]`/`[target]` 节、无 vendored LLVM：

| 键 | 值 | 评注 |
|---|---|---|
| `LIBCLANG_PATH`（:4） | `C:/Users/<原开发者>/AppData/Local/Programs/Python/Python313/Lib/site-packages/clang/native` | **本机绝对路径，指向原开发者的 Python site-packages**。干净机上不存在 → bindgen 报 "unable to find libclang"。对策：改本文件，或 shell `export LIBCLANG_PATH=...`（cargo `[env]` 非 force，已有环境变量优先） |
| `CMAKE_POLICY_DEFAULT_CMP0091`（:9） | `NEW` | 双栈 CRT 纪律组成部分，所有机器通用，勿动 |
| `CMAKE_MSVC_RUNTIME_LIBRARY`（:10） | `MultiThreaded` | 同上（AGENTS 大坑 5） |

### 1.3 首次构建的网络拉取

- Cargo.lock 共 **657 个包**，全部 registry/path 来源，git 依赖 0。
- **唯一构建期外网下载**：sherpa-onnx-sys 1.13.7 build.rs 从 `https://github.com/k2-fsa/sherpa-onnx/releases/download` 拉 `sherpa-onnx-v1.13.7-win-x64-static-MT-Release-lib.tar.bz2`（**~115MB**，解压后 984MB）到 `target/sherpa-onnx-prebuilt/`（构建脚本自动下载，非人工放置，gitignored）。
- whisper.cpp **vendored** 在 whisper-rs-sys crate 包内随 crates.io 下载，无二次拉取；ort 用 `load-dynamic` feature，构建期零下载（onnxruntime.dll 已在 `assets/`，运行期从内嵌解压）。
- **断网能否构建**：默认不能（需 crates.io + GitHub 两源）；预置 cargo 缓存 + `SHERPA_ONNX_ARCHIVE_DIR`（指向本地归档）或 `SHERPA_ONNX_LIB_DIR`（指向已解压 lib）后可离线（sherpa-onnx-sys build.rs 原生支持）。
- 磁盘量级：`target/` ≥ 2GB（sherpa 解压 984MB + 中间产物）；`cargo clean` 会删掉 sherpa 缓存 → 下次构建重新下载 115MB。
- 国内建议自配 crates.io 镜像（本机全局 `~/.cargo/config.toml` 配了 rsproxy.cn，**仓库自身不含镜像配置**）。

### 1.4 clone → 首次 `cargo build` 成功的手动清单（可直接当 CONTRIBUTING 骨架）

1. 安装 rustup + `x86_64-pc-windows-msvc` 工具链（无版本锁定，建议较新 stable；参考机 1.98.1）。
2. 安装 VS Build Tools，勾「使用 C++ 的桌面开发」工作负载（含 Windows SDK）。
3. 安装 CMake 并入 PATH。
4. 安装 LLVM，然后**二选一**处理 libclang：改 `.cargo/config.toml:4` 为本机 libclang 目录，或运行前 `export LIBCLANG_PATH=<本机目录>`。
5. 联网（crates.io + GitHub；离线见 1.3 的 sherpa 环境变量兜底），`git clone` 后直接 `cargo build`。
6. （跑真模型才需要）准备模型缓存，见 §2.3 冒烟纪律。

---

## §2 日常开发命令与纪律（B）

> 【一眼结论】`cargo test --workspace` 全绿是收工前提（6 个 ignored 真模型/真网络探针默认不跑）；GUI 冒烟必须设 `LIVETRANSLATE_CONFIG_DIR` 且其中 settings.json 显式 `models_dir` 指真实缓存；`cargo test` 全程临时目录不污染真实配置，**不设环境变量的裸 `cargo run` 会读写真实配置目录**。

### 2.1 命令表

| 命令 | 用途 | 注意 |
|---|---|---|
| `cargo test --workspace` | 收工前提 | 滚动基线（AGENTS 当前 396 测全绿）+ 6 个 `#[ignore]` 默认跳过（见 2.2） |
| `cargo build --release -p lt-app` | 出分发 exe | 产物 `target/release/livetranslate.exe`（72.2 MiB）+ `livetranslate.pdb` 30.7MB（调试符号，**不分发**） |
| `cargo run -p lt-app` | GUI 冒烟 | 必须先设 `LIVETRANSLATE_CONFIG_DIR`（见 2.3），否则写真实配置 |
| `cargo clippy --workspace --all-targets` | lint | 无硬性基线（无 `[workspace.lints]`、无 deny 属性）；软约定「新码零告警」，存量快照 60 warning / 0 error（docs/archive/asr-hardening.md:54；lt-ui 子集基线 20 条） |

### 2.2 ignored 探针清单（实测 6 个；AGENTS 写「5 个」已滞后）

| 位置 | 需要什么 |
|---|---|
| `crates/lt-asr/src/engines/nano.rs:211` | 真实 FunASR nano 模型（~963MB）+ test_wavs |
| `crates/lt-asr/src/engines/qwen3.rs:225` | 真实 qwen3 模型（~941MB）+ test_wavs |
| `crates/lt-asr/src/sensevoice.rs:324` | 真实 SenseVoice 模型 |
| `crates/lt-asr/src/engines/whisper.rs:252` | `LT_WHISPER_MODEL` 指向 ggml .bin |
| `crates/lt-asr/src/engines/whisper.rs:276` | `LT_WHISPER_MODEL` + `LT_WHISPER_SPEECH_WAV` |
| `crates/lt-models/src/download/mod.rs:819` | **真网络 ~1GB（hf-mirror），且写真实模型缓存** |

### 2.3 GUI 冒烟纪律

- `LIVETRANSLATE_CONFIG_DIR` 读取点：`crates/lt-models/src/paths.rs:10-14`（非空则整棵目录树指向它，可为任意盘符路径）。
- `models_dir` 键定义：`crates/lt-proto/src/settings.rs:84`（`Option<PathBuf>`，默认 None → `<config>/models`，paths.rs:25-29）。
- 冒烟临时配置目录里没有模型 → 引擎探测全部失败；必须在临时目录的 settings.json 里显式 `models_dir` 指向真实模型缓存（如 `C:\Users\<u>\.config\livetranslate\models`）。

### 2.4 开发操作副作用边界

- **裸 `cargo run -p lt-app`**（不设环境变量）：读写真实 `~/.config/livetranslate`（settings/logs/ort 解压/transcripts/模型下载）+ 占用单实例互斥 +（首次发通知时）注册开始菜单 lnk。
- **`cargo test`**：lt-models 测试统一 `std::env::temp_dir()` + `LIVETRANSLATE_CONFIG_DIR` 覆盖，进程级 `ENV_LOCK` 串行防污染（`crates/lt-models/src/lib.rs:7-10`）；下载集成测试用 127.0.0.1 本地 mock HTTP，零外网。仅两个只读例外（硬编码本机路径的残留探针，不写）：`cache.rs:528-531`、`cache.rs:540`。
- **会写真实配置的操作清单**：裸 run、手工跑 ignored（尤其 download/mod.rs:819）、GUI 内触发的下载/设置保存。日常 test/clippy 安全。

---

## §3 分发产物（C）

> 【一眼结论】产物 = 单个 `livetranslate.exe`（72.2 MiB，内嵌 ~33.2MB 资产，无旁置文件，worker 同 exe 自拉起）；但干净 Win10/11 **必须装 VC++ 2015-2022 x64 Redist**（exe 导入 VCRUNTIME140/140_1，解压出的 onnxruntime.dll 另需 MSVCP140/140_1）——`distribution.md`「无 VC 运行库依赖」断言被导入表推翻。WD-1~WD-5 全未施工，当前最小分发 = 手工拷裸 exe。

### 3.1 产物与内嵌资产

二进制名 `livetranslate`（`crates/lt-app/Cargo.toml` `[[bin]]`）。实测 exe = **75,754,496 B ≈ 72.2 MiB**（2026-09-08 20:32 构建，对应 0edad27）。

| 内嵌资产 | 大小（实测） | 嵌入点 |
|---|---|---|
| assets/ort/onnxruntime.dll | 18,093,368 B | `lt-pipeline/src/vad.rs:16`（include_bytes!） |
| assets/silero_vad.onnx | 2,327,524 B | `lt-pipeline/src/vad.rs:12`（**内存直载，不落盘**，见 3.3） |
| 三字体 brotli（CJK 11.46MB + Mono 0.76MB + Symbols 0.44MB） | 12,652,532 B | `lt-ui/src/fonts.rs:27/31/33` |
| i18n zh/en yaml | 61,184 B | `lt-i18n/src/lib.rs:12,14` |
| CHANGELOG zh/en | 11,809 B | `lt-ui/src/windows/panel/changelog_tab.rs:15-16` |
| 图标（app/tray 系列 PNG + app.ico） | ~65KB | `lt-ui/src/tray.rs:321-355`、`lt-ui/src/notifications.rs:22` |
| **合计** | **≈ 33.2 MB** | |

exe 其余 ~39MB = 静态链接代码：sherpa-onnx v1.13.7（static-MT 预编译库，内嵌自己的 ORT）、whisper.cpp/ggml、egui/wgpu/winit、Rust std + tokio/reqwest(rustls) 等。模型文件**不内嵌**（GB 级，运行时下载，见 §4）。

### 3.2 运行库依赖——本盘点最重要实锤

方法说明：grep 二进制不可靠（exe 内嵌了 18MB dll 的字节，会假命中其导入名），故用 Python 解析 PE 导入目录（数据目录第 2 项）。

| 组件 | 动态导入（非系统内置部分） | 结论 |
|---|---|---|
| exe 本体 | **VCRUNTIME140.dll + VCRUNTIME140_1.dll** + UCRT（api-ms-win-crt-*，Win10/11 内置）；**无 MSVCP140**（C++ 标准库已静态化：sherpa /MT + whisper.cpp `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` 生效） | 缺 vcruntime140 → 进程加载即失败 |
| 解压出的 onnxruntime.dll | **VCRUNTIME140/140_1 + MSVCP140/MSVCP140_1** + UCRT（官方预编译 /MD 构建，改不了） | 同上 + msvcp140 |

- UCRT 系统内置；**vcruntime140 / msvcp140 非内置**（System32 里见到的通常是别的软件装的 Redist）。
- **总体结论：干净 Win10/11 需安装 VC++ 2015-2022 Redistributable (x64)**，一次满足两者；替代方案 = 随包在 exe 旁带 4 个 DLL（vcruntime140、vcruntime140_1、msvcp140、msvcp140_1——redist 许可允许再分发），可做成真·解压即用。
- 根因：CMake 级静态化只覆盖 C/C++ 组件；Rust 侧未开 `+crt-static`（默认动态链 vcruntime140）。
- **文档矛盾待勘误**：`docs/distribution.md:30`（「静态 CRT（无 VC 运行库依赖）」）与 `:78`（「无 Python/运行库/字体任何前置要求」）与导入表实证直接矛盾。

### 3.3 启动解压与旁置依赖

- `ensure_ort_dylib`（`lt-pipeline/src/vad.rs:20-44`）：启动早期（`main.rs:28`，任何 ORT Session 之前）把内嵌 dll 解压到 **`<config>/ort/onnxruntime.dll`**；判定 = 目标不存在**或尺寸 ≠ 内嵌字节数**（尺寸即校验，无哈希），`.dll.tmp` + rename 原子替换，进程级 Once。失败 → `main.rs:28` `?` 返回 Err——release 下 `windows_subsystem="windows"` 无控制台且在 `logging::init`（main.rs:42）之前，**用户视角 = 双击静默无反应**。
- 字体 brotli 为**内存解压**（`fonts.rs:46-58`，brotli → `Box::leak` → OnceLock），不落盘。silero_vad.onnx 同为内存直载（`vad.rs:12,93`）——AGENTS.md「onnxruntime.dll + silero_vad.onnx 内嵌，启动解压到配置目录」后半句对 silero 不成立。
- **无 sidecar/便携模式**：`current_exe` 全仓仅两处——worker 自拉起（`lt-asr/src/client.rs:65`）与 lnk 注册（`lt-ui/src/notifications.rs:136`）。单 exe 拷走即用（前提：机器已有 VC Redist）。
- ASR worker：`main.rs:23-25` 分支 → `--asr-worker <config-json>`，父端 `client.rs:64-73`（CREATE_NO_WINDOW + Job Object 孤儿兜底）。四引擎推理库全部静态编入，worker 只需配置指向的模型目录，**零额外文件**。

### 3.4 分发形态：现状 vs 规划（WD-1~WD-5 全未施工）

| 工作包 | 内容 | 现状（逐项核码） |
|---|---|---|
| WD-1 打包脚本 | `scripts/package_release.ps1` | 未做（scripts/ 仅 silero_reference.py） |
| WD-2 版本可见性 | VERSIONINFO 版本字段 / `--version` / UI 版本行 | 未做（build.rs:3-14 仅图标+产品名，无版本号；`--version` 全仓无） |
| WD-3 法律与说明 | LICENSE / THIRD_PARTY_NOTICES / README_zh | 未做（仓库根无 LICENSE*/NOTICE*/README*；Cargo.toml:15 仅声明 MIT） |
| WD-4 whisper 双源 | registry 增 MS 条目 | 未做（registry.rs:149-154 六档 `ms: None, always_hf: true`，有测试钉住） |
| WD-5 二次启动反馈 | 激活已有窗口 | 未做（互斥已有，第二实例仅 bail，release 下无声退出，见 §5.3） |

**现状**：最小可行分发 = 手工拷出 `target/release/livetranslate.exe` 裸文件（未签名，SmartScreen 需「仍要运行」），收件机自备 VC Redist（文档尚未承认此依赖）。**规划**（distribution.md §3.1）：`LivetranslateRS-<版本>-win-x64.zip`（exe + README_zh + LICENSE + NOTICES + sha256.txt）——WD-3 施工时应一并写明 VC Redist 前置或改随包带 DLL。

---

## §4 运行时足迹 I：文件系统与数据分布（D1 + F）

> 【一眼结论】所有运行时数据集中一个根目录 `~/.config/livetranslate`（本机实测 **2.26GB**，模型 2.2GB 占绝对大头）；settings.json 首次改动才落盘（tmp+rename 原子写，300ms 防抖）；日志每次启动新建、**无保留策略**；`models_dir` 重定向只带走模型树，settings/logs/transcripts/ort/app.ico 恒在根目录；**TEMP 零足迹**。

### 4.1 配置根解析（单一事实源）

`crates/lt-models/src/paths.rs:9-17`：`LIVETRANSLATE_CONFIG_DIR` 非空即用（整路径，可任意盘符）；否则 `dirs::home_dir() + ".config" + "livetranslate"` **字面拼接**——Windows 下确为 `C:\Users\<u>\.config\livetranslate`，与 %APPDATA% 无关（文件头注释 paths.rs:1-4 明确这是 D-9 用户裁决）。

### 4.2 完整目录树（每节点：谁写 / 何时 / 多大 / 证据）

```text
C:\Users\<u>\.config\livetranslate\            ← config_dir() paths.rs:9-17；本机实测合计 ≈ 2.26 GB
├── settings.json          2,783 B
│   【写】settings_io.rs:27-42（tmp+remove+rename 原子写）
│   【时】首启不落盘！首次写入在首个持久化命令（面板改动 300ms 防抖 state.rs:707 →
│          Cmd::ApplySettings → shell.rs:229-233；引擎切换/字体变更/下载成功等）
├── ort\onnxruntime.dll    18,093,368 B
│   【写】vad.rs:20-44（缺失或尺寸不符才重解压，tmp+rename）【时】每次启动早期，Once
├── app.ico                 27,917 B
│   【写】notifications.rs:239-250（尺寸比对幂等）【时】首次发 Toast（lnk 创建/重建时）
├── logs\                   33 MB / 43 个文件
│   └── livetrans_%Y%m%d_%H%M%S.log
│       【写】lt-app/src/logging.rs:26-52（文件层 DEBUG、行级 flush；worker 不写文件）
│       【时】每次进程启动新建一个；【保留策略：无，无限累积】
├── transcripts\            244 KB / 126 个
│   └── livetrans_{ts}_{original|translation|all}.txt
│       【写】lt-pipeline/src/transcript.rs:146-170（懒创建，首条识别才开；append）
│       【时】settings.auto_save_transcript 开启且有最终段；【无清理】
└── models\                 2.2 GB（HF hub 兼容布局，cache.rs:15-37）
    ├── huggingface\hub\
    │   ├── models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30\   964 MB
    │   │   └── snapshots\main\（六件套：embedding/encoder_adaptor/llm int8 + tokenizer 三件）
    │   ├── models--csukuangfj2--sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25\  954 MB
    │   │   └── snapshots\main\（conv_frontend/encoder/decoder int8 + tokenizer）
    │   └── models--ggerganov--whisper.cpp\  88 MB（六档共用单仓；tiny/base q5_1 已下）
    ├── modelscope\models\pengzhendong--sherpa-onnx-sense-voice-…\  229 MB（model.int8.onnx 239MB + tokens）
    ├── jfk.wav  344 KB   ← 人工放置的验收样本，非应用写入，UI 缓存删除不覆盖
    └── *.incomplete        （瞬态续传文件：成功 rename 消费 / 取消有意保留现场 D-23）
        【写】download/mod.rs:417-484（.incomplete 续传 + 长度精确校验 + sha256 三通道））

%APPDATA%\Microsoft\Windows\Start Menu\Programs\LiveTranslate.lnk   ← 配置根之外唯一落盘
    【写】notifications.rs:134-189（首次发 Toast 时；缺失或目标≠当前 exe 即重建）
```

- 下载探测 = 注册表 manifest 逐文件「存在 + len ≥ 下限」（cache.rs:41-45）；完整性三通道校验 AH-5（download/mod.rs:417-484：total=None 拒绝收尾 → 长度精确 → sha256 流式）。
- 本机核账：`models_dir: null` 未重定向；无 .tmp/.incomplete 残留；settings.json 全键含默认值落盘（与 `#[serde(default)]` 行为一致）。

### 4.3 models_dir 重定向的跟随边界（用户易踩）

| 跟随 models_dir（换盘带走） | 不跟随（恒在配置根） |
|---|---|
| 模型下载落盘（backend.rs:159）、缓存探测、worker 装配（pipeline.rs:909）、磁盘预检、数据页删除/打开 | settings.json、**logs/**、**transcripts/**、**ort/ 解压物**、app.ico |

含义：用户把模型指到 D 盘后，「删配置目录」不会删到 2.2GB 模型；卸载文档必须提示（§8 C6）。

### 4.4 TEMP 足迹

全仓 16 处 `env::temp_dir()` 命中**全部位于 `#[cfg(test)]`/examples/只读回退**；`tempfile` crate 零引用。运行期瞬态临时物（settings.json.tmp、onnxruntime.dll.tmp、*.incomplete）都放在最终目标旁、成功后 rename 消费。**生产运行期不在系统 TEMP 留任何文件。**

---

## §5 运行时足迹 II：系统级与网络副作用（D2 + D3）

> 【一眼结论】系统改动面 = 开始菜单一个 .lnk，**仅此一处**（注册表零写入、无自启、无崩溃转储、无全局钩子/热键）；网络出口仅模型下载与 LLM API 两处，**音频零出网**；二次启动被单实例互斥拒绝且 release 下静默（WD-5 未施工）。

### 5.1 开始菜单 .lnk（唯一系统级落盘）

- 路径：`%APPDATA%\Microsoft\Windows\Start Menu\Programs\LiveTranslate.lnk`（用户级，免管理员；notifications.rs:192-200——全仓唯一 %APPDATA% 使用）。
- 触发：**进程生命周期内第一次真正发 Toast 时**（`OnceLock` 进程级一次，notifications.rs:109-131），启动流程不碰它。缺失**或**目标 ≠ 当前 exe（大小写不敏感）即重建（:138-140）——覆盖便携迁移。
- AUMID `com.livetranslate.app` 的归属 = .lnk 上的 `System.AppUserModel.ID` 属性 + 进程级 `SetCurrentProcessExplicitAppUserModelID`（:124），**不进注册表**。
- 实测：lnk 存在，目标指向 `target\release\livetranslate.exe`，图标指向配置根 `app.ico`。

### 5.2 注册表：零写入

全仓唯一注册表访问 = `lt-ui/src/fonts.rs:155-193` 系统字体扫描（HKCU+HKLM，只开 `KEY_READ`）。实测 `HKCU\Software\Classes\AppUserModelId` 下无本应用键、`HKCU\Software` 一级无 livetranslate、`HKCU\...\CurrentVersion\Run` 无自启条目——与代码证据互证。

### 5.3 单实例与二次启动

- 命名互斥 `LiveTranslateSingleInstance`（`lt-app/src/main.rs:74-100`，M0.8 即有）；第二实例 `bail!` 一行 stderr 即退——release 无控制台且在日志初始化前 → **用户视角无声退出**。WD-5（激活已有窗口）未施工。
- 微竞态注记：`CreateMutexW` 未查 `ERROR_ALREADY_EXISTS`，完全依赖先行 `OpenMutexW` 探测，极小时间窗内理论可双开（常规先后启动无影响）。

### 5.4 崩溃足迹：零

无 `panic::set_hook`、无 minidump、无 crash log（全仓 grep 空）。panic 走 std 默认；worker 子进程 stderr 被父端捕获回流日志窗。

### 5.5 运行期 OS 级效果（不落盘，退出即消失）

| 效果 | 位置 | 说明 |
|---|---|---|
| 托盘图标+菜单 | `lt-ui/src/tray.rs`（D-35 专用线程 + GetMessage 泵） | 图标内存绘制，不写盘 |
| Job Object | `lt-asr/src/job.rs:19-37` | KILL_ON_JOB_CLOSE 收割 worker 孤儿 |
| Toast → 通知中心 | `notifications.rs:78-83` | 系统侧存储管理，应用不跟踪不撤回 |
| 单实例互斥量 | main.rs:74-100 | 内核对象，进程退出 OS 回收 |
| WASAPI 采集 / wgpu 渲染 | lt-pipeline / lt-ui | 设备/显存占用，无持久状态 |

确认不存在：全局钩子（SetWindowsHookEx 零命中）、全局热键（WP-8 未做）、剪贴板写入、开机自启。

### 5.6 网络出口（共两处，无第三处）

| 出口 | 端点 | 说明 |
|---|---|---|
| 模型下载 | HF 官方 `https://huggingface.co` / 镜像 `https://hf-mirror.com`（选 hub=ms 走 HF 时自动用镜像）/ ModelScope `https://modelscope.cn`（download/mod.rs:51,54,224） | hub 回落链：所选 hub 居首，仅 Net/404 失败回落另一 hub（mod.rs:17-40）；重试退避 1/4/16s |
| LLM 翻译 | 用户配置 `api_base`（默认 `http://127.0.0.1:1234/v1` 本地端点，settings.rs:274） | 出网内容 = 译文请求文本 + 用户配置的 key，仅发往该端点 |

- **音频绝不出网（代码事实）**：lt-pipeline 与 lt-asr 的 Cargo.toml 零网络依赖；音频路径 = wasapi → VAD → worker 子进程 stdin/stdout IPC → 本地 ONNX/ggml 推理，不存在出网路径。
- 无更新检查（WD-8 未做）、无遥测/上报、无字体下载（三字体全内嵌）。

---

## §6 完全干净卸载（E）

> 【一眼结论】无安装器即无卸载器。完全卸载 = ① 退出应用 → ② 删 `~/.config/livetranslate`（~2.3GB；若改过 models_dir 另删重定向目录）→ ③ 删 exe → ④ 删开始菜单 lnk。四步后注册表/TEMP/自启/互斥零残留。顺序：**先删 exe 再删 lnk** 最稳——lnk 只在应用发 Toast 时复活，exe 没了就永不复活。

### 6.1 分步清单

| 步 | 操作 | 大小 | 不删的残留 | 依据 |
|---|---|---|---|---|
| 0 | 应用退出（托盘 → 退出） | — | 运行中文件句柄/互斥存活，可能锁文件 | main.rs:74-100、logging.rs:33 |
| 1 | 删 `~/.config/livetranslate` 整目录 | **2.3GB**（models 2.2G + logs 33M + ort 17.3M + transcripts 244K + app.ico 28K + settings.json 4K） | 全部设置/模型/日志/转录/图标 | §4.2 封闭清单 |
| 1b | **仅当改过 `models_dir`**：另删重定向目录 | 最多再 2.2GB | 模型缓存全部残留 | paths.rs:26-33；§4.3 |
| 2 | 删 exe | 72.2MB | — | — |
| 3 | 删 `%APPDATA%\...\Start Menu\Programs\LiveTranslate.lnk` | ~1KB | 开始菜单死快捷方式（无功能后果） | notifications.rs:192-197 |
| 4 | （可选）用户自选路径的导出转录 | 不定 | 应用不知情，无法代删 | app.rs:1507-1540 |

### 6.2 顺序陷阱（代码证据结论）

- **lnk 复活条件（精确）**：`ensure_aumid_shortcut` 只在 `ensure_initialized`（OnceLock 进程级一次）内执行，唯一触发链是 Toast 发送；全部 6 个 Toast 触发点均为用户动作（隐藏悬浮窗 app.rs:645 / 基准完成 / 字幕首开 / 空导出 / 删缓存完成），无启动期通知。
- **先删 exe 再删 lnk 安全**：exe 不存在 → 永远不会发 Toast → lnk 永不复活。
- **反序有复活窗口**：应用运行中删 lnk，用户下一次隐藏悬浮窗即重建。
- **冷启动自愈成立**：配置目录整个删掉后，7 类落盘全部 `create_dir_all` 懒重建（settings_io.rs:30 / vad.rs:27 / logging.rs:28 / transcript.rs:147 / notifications.rs:241 / download/mod.rs:288,373），无一依赖目录已存在；settings 全默认值，模型按缺失清单提示重下。
- 无任何自删/卸载辅助逻辑（确认）；应用内唯一删除能力 = 数据页删模型缓存（data.rs:307，不覆盖 logs/transcripts 与 models 根游离文件如 jfk.wav）。

### 6.3 零残留项（无需处理）

注册表（零写入）、TEMP（生产零足迹）、单实例互斥与 Job Object（内核对象 OS 回收）、通知中心历史 Toast（系统侧数据，可在通知设置手清，非应用残留）。

---

## §7 数据分布整齐度评估（G）

> 【一眼结论】按「单根自包含」尺子：**优**（根外足迹仅 lnk + 用户导出两处，封闭）；按「结构一致性」：**良**（settings 全 snake_case、models 统一 HF hub 命名；models 根有一个游离 jfk.wav）；按「Windows 惯例」：`~/.config` 是**有据可查的有意偏离**（D-9）而非事故，但该裁决只写在开发者文档里，从未写给用户看——且 3 处现有文档断言与实证冲突。

### 7.1 三把尺子逐项

1. **Windows 惯例**：有意偏离——paths.rs:1-4 头注释「D-9：用户明确要求字面 ~/.config 而非 %APPDATA%」；决策史 docs/archive/rewrite-research.md:137；落地 commit fb9789c。讽刺的是全仓唯一遵循 Windows 惯例的落点恰是 lnk（用 %APPDATA%）。**评价：偏离本身合法，缺口在「用户不知情」**——装在哪、多大、为何不在 %APPDATA%，应写进 WD-3 用户文档。
2. **单根自包含**：根外足迹恰好 2 处（lnk——AUMID 身份必需，技术上不可移入配置根；用户导出——用户自选路径）。备份=拷一个目录（+重定向的模型盘）、迁移=拷目录+重开应用，语义清晰。
3. **结构一致性**：settings.json 全 snake_case 扁平键（`#[serde(rename_all="snake_case")]` settings.rs:35-36）；models/ 统一 HF hub 命名（`models--{org}--{name}/snapshots/{main|master}` + modelscope 子根）。不一致点：models 根游离 jfk.wav（人工样本，manifest 探测天然忽略，但 UI 缓存删除不覆盖）。

### 7.2 文档缺口清单（直接喂 WD-3 / WD-9）

1. 仓库根 **README.md 不存在**（面向任何人都零说明）。
2. distribution.md:91 卸载说明「删 exe + 删配置目录即彻底清除」——**漏 lnk**、**漏 models_dir 重定向情形**。
3. distribution.md:30/:78「静态 CRT（无 VC 运行库依赖）/无运行库前置」——**被导入表推翻**（§3.2）。
4. distribution.md:27「~59MB」过期（实测 75.8MB）；:31「无 sha256 校验」过期（AH-5 已加）；:32 行号漂移。
5. 面向用户的「数据位置说明」（各子目录是什么/多大/可否单独删）完全缺失。
6. AGENTS.md「onnxruntime.dll + silero_vad.onnx 内嵌，启动解压到配置目录」——后半句对 silero 不成立（内存直载）。

---

## §8 整改候选（待裁决；立项则新偏差自 D-37 起）

| # | 候选 | 现状证据 | 整改方向 | 影响面 |
|---|---|---|---|---|
| C1 | **logs 无限累积** | 每启动新建一个文件、全仓零清理；实测 4 天 43 文件 33MB | 启动时保留最近 N 个 / 清理 N 天前 | lt-app/src/logging.rs 小改动 |
| C2 | **transcripts 存量永不清扫** | 开关只管新增不管存量 | 数据页加清理入口或同 C1 保留策略 | lt-ui 数据页 |
| C3 | **VC Redist 前置** | 导入表实锤（§3.2）；文档断言相反 | 三选一：文档承认前置 / Rust 侧 `+crt-static`（exe 侧消 vcruntime140，onnxruntime.dll 的 msvcp140 仍在）/ 随包带 4 个 redist DLL（真·解压即用） | 分发前置要求，建议 WD-3 一并裁决 |
| C4 | 卸载说明缺口 | distribution.md:91 漏 lnk + models_dir | 补两句话 | 纯文档（WD-3） |
| C5 | AGENTS silero 措辞 | 「启动解压」对 silero 不成立 | 改为「onnxruntime.dll 启动解压；silero 内存直载」 | 纯文档 |
| C6 | models_dir 不跟随 logs/ort 等 | paths.rs:43-52、vad.rs:26 | 用户文档提示；远期可评估统一跟随 | 文档优先 |
| C7 | 数据页清理不覆盖游离文件 | data.rs:44-85 只扫注册 repo（jfk.wav 类） | 提示或忽略 | 低优先 |
| C8 | **早期失败静默面** | ort 解压失败 main.rs:28 `?` 静默退（在 logging::init 前）；二次启动 bail 静默 | 最小可见反馈（MessageBox/落日志）；后者属 WD-5 范围 | 分发前值得补 |
| C9 | settings.json.tmp 崩溃窗口 | remove+rename 非真原子，极窄窗口无配置（按默认重启，不损坏） | ReplaceFileW 或忽略 | 极低优先 |
| C10 | distribution.md 数字漂移 | 59MB/无 sha256/行号 | 随 WD-2/WD-3 刷新 | 纯文档 |
| C11 | LIBCLANG_PATH 可移植性 | 固化本机绝对路径（§1.2）= clone 非零配置 | 文档化（CONTRIBUTING），可选改探测式/相对路径；顺带评估 rust-toolchain.toml 锁版 | 开发体验 |
| C12 | AGENTS「5 个 ignored」滞后 | 实测 6 个（§2.2） | 更新计数 | 纯文档 |

---

## §9 证据方法与局限

- **方法**：5 个并行只读研究代理（代码取证全部 file:line）+ 本机实测（`ls`/`du`/`reg query`/PE 导入表 Python 解析/开始菜单与注册表核对）。落盘点封闭清单由 4 个代理独立 grep 交叉，结果一致。
- **局限（诚实边界）**：① sherpa-onnx/whisper.cpp/onnxruntime 等 C++ 原生库运行期写盘行为未逐行读 C++ 源证明（按惯例纯内存运算，Procmon 实机走查可闭环——可挂 WP-9）；② lnk 的 AUMID 属性未做机器侧读取验证（基于代码 :155-183）；③ HKCU\Software\Classes 深树未逐级扫（代码零写注册表 API 互证）；④ clippy 存量计数为文档快照（本次纪律禁 build 未实测）。
- **关联待办**：WD-1~WD-5（分发链路）、WD-3（用户文档，§7.2 缺口直接可喂）、WP-9（实机调优，Procmon 走查）、§8 整改候选待用户裁决。
