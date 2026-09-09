# 分发与用户旅程（docs/distribution.md）

> 状态：**阶段二活跃文档**（2026-09-07 归档重组；阶段一 docs/archive/ 为决策史）。D-18~D-21 已裁决定案；阶段一（WD-1~WD-5）待施工，阶段二（WD-7 CI / WD-8 检查更新）预案。


> 2026-09-07 用户裁决 D-18~D-21 定案。本文 = 阶段一（本地分发）施工依据 + 阶段二（公开发布）预案。
> 前置调研结论：原版 Python 的分发故事完整（GitHub Releases 便携 zip + CI + 双语 README + 首启强制向导），
> Rust 版功能面已基本就绪（向导/下载器/单实例/日志落盘均有实现与测试），**分发层完全空白**（无打包脚本/LICENSE/README/版本可见性/CI）。

## 0. TL;DR

- **D-18 暂不公开发布**：先本地 zip 分发验证；CI、公开 README、检查更新按钮均属阶段二。
- **D-19 首启直进主界面**：2026-09-01 less-is-more 决策转正；向导代码保留不删、不接线。
- **D-20 检查更新按钮**：设置页按钮查 GitHub Releases，提示 + 打开下载页，不自替换；随公开发布启用。
- **D-21 全模型双源**：whisper 六档增加 ModelScope 镜像源（打破 always-HF）；所选 hub 缺失时回落另一 hub。
- 阶段一工作包：WD-1 打包脚本 / WD-2 版本可见性 / WD-3 随包法律与说明 / WD-4 whisper 双源 / WD-5 二次启动反馈；WD-6（可选）首启轻引导。

## 1. 裁决记录（2026-09-07，不再重开）

| 编号 | 裁决 | 内容 | 与原版关系 |
|---|---|---|---|
| D-18 | **暂不公开发布** | 阶段决策：现阶段本地打 zip 分发给身边用户验证；CI/GitHub Releases/公开 README/原 Python 仓导流全部后置到「公开发布」阶段 | 原版走公开 GitHub Releases；本版暂缓，届时重开渠道议题（新仓 vs 沿用 TheDeathDragon/LiveTranslate） |
| D-19 | **首启直进主界面** | 无 settings 文件 → 全默认值直接进主界面并自动启动管道；模型缺失由悬浮窗 unavailable + 识别页按需下载承接（现状 `lt-app/src/main.rs:3-6`）。向导全套代码（`setup.rs` + `StartupFlow` + backend first_launch 编排，均有测试）**保留不删、不接线**；重启用 = `startup_flow` first_launch 分支 + `main.rs` 传 `first_launch=true`（docs/archive/parity-closure.md 已记录为一行改动） | **偏差转正**：原版为强制向导（取消即退出）+ 缺模型弹下载门；本版 2026-09-01 决策取消，本次用户确认维持 |
| D-20 | **应用内「检查更新」按钮** | 设置页按钮 → GitHub Releases API 比对版本 → 有新版提示并打开下载页；不自动下载/替换。失败（含 api.github.com 不可达）静默提示 | **新增能力**（原版应用内无更新）；依赖公开 Releases，实现排在阶段二 |
| D-21 | **全模型双源** | 所有模型尽量同时具备 HuggingFace + ModelScope 两个源：sensevoice-small、funasr-nano 已双源；**whisper 六档打破 `always_hf`，增加 MS 镜像仓条目**；下载时按用户所选 hub 优先，所选 hub 无该模型则自动回落另一 hub。**不做** hf_endpoint 设置键（原讨论方向已由本裁决取代） | **新增偏差**：原版/现 Rust 均为 whisper 永远走 HF（原版 faster-whisper 仓只在 HF）；MS 候选镜像仓（`cjc1887415157/whisper.cpp`、`timeless/whispercpp`）均为第三方，**档位完整性需实测后登记** |

## 2. 已锁定基线（不因本计划重开）

- 单 exe（release ~75.6MB，打包 zip ~35.6MB）：内嵌 onnxruntime.dll（启动解压 `<config>/ort/` + `ORT_DYLIB_PATH`）、silero_vad.onnx（内存直载，VAD 零下载）、三族字体 brotli（解压 ~100ms，机器无关）。
- Windows 10/11 x64、纯 CPU；**分发前置 = Microsoft VC++ 2015-2022 x64 Redist**（C3 裁决 2026-09-09：exe 导入表实锤依赖 VCRUNTIME140/140_1，解压出的 onnxruntime.dll 另需 MSVCP140/140_1；本行旧断言「静态 CRT 无 VC 依赖」有误已勘正——用户装 Redist 即全覆盖，不随包分发系统组件）；仅 Windows 实装。
- 配置目录 = 字面 `~/.config/livetranslate`（非 %APPDATA%；`LIVETRANSLATE_CONFIG_DIR` 覆写）；子目录 `settings.json` / `models/` / `transcripts/` / `logs/` / `ort/`；`models_dir` 键可重定向模型缓存。
- 单实例命名互斥体（`main.rs:67-95`）；日志落盘 `logs/livetrans_{时间戳}.log`（DEBUG，按次滚动）+ 应用内日志窗。
- settings 原子写（tmp+rename）、300ms 防抖、`from_value_compatible` 兼容导入原版 `user_settings.json` 键名。
- 模型下载：双 hub 统一下载器（断点续传 `.incomplete`+Range、3 次退避重试、代理三模式 none/system/URL）；无 sha256 校验（与原版一致）。

## 3. 阶段一：本地分发（现在）

### 3.1 分发物规范

| 项 | 规格 |
|---|---|
| 产物 | `livetranslate.exe`（`cargo build --release -p lt-app`） |
| 包名 | `LivetranslateRS-<版本>-win-x64.zip`（版本号依赖 WD-2 落实） |
| 包内容 | exe + `README_zh.md`（使用说明简版）+ `LICENSE`（MIT）+ `THIRD_PARTY_NOTICES.md` + `sha256.txt` |
| THIRD_PARTY 收录 | onnxruntime（MIT）、silero-vad（MIT）、whisper.cpp/ggml 模型（MIT）、SenseVoice 模型（原仓许可）、sherpa-onnx（Apache-2.0）、字体三族（OFL 1.1，全文/出处对齐 `assets/SOURCES.md`） |
| SmartScreen | 未签名（RESEARCH「签名自便」维持）；README 写明「更多信息 → 仍要运行」，公开发布阶段可再议签名 |

### 3.2 发布手册（人工步骤，脚本化于 WD-1）

1. 版本号进 `Cargo.toml` `[workspace.package]`（WD-2 后由 zip 名自动携带）；
2. `cargo build --release -p lt-app` + `cargo test --workspace` 全绿；
3. `scripts/package_release.ps1`：拷 exe → 汇集 LICENSE/NOTICES/README → zip → 生成 sha256；
4. 干净机（或删 `~/.config/livetranslate`）冒烟一轮首启→下载→出字幕；
5. 分发 zip + sha256。

### 3.3 阶段一工作包

| 包 | 内容 | 估量 | 备注 |
|---|---|---|---|
| **WD-1** | `scripts/package_release.ps1` 打包脚本 + 发布手册落档 | 0.5d | §3.1/§3.2 的自动化 |
| **WD-2** | 版本可见性三件：`build.rs` 写 VERSIONINFO（现只嵌图标无版本字段）、`--version` 参数（现仅 `--asr-worker`）、设置页/关于处版本行（现 UI 无任何版本展示；changelog tab 只有日期） | 1h 级 | zip 命名、用户报障、UA（下载器已带 `livetranslate-rs/{ver}`）都对齐此版本源 |
| **WD-3** | 随包法律与说明：根目录 `LICENSE`（现仅 Cargo.toml 声明 MIT，无文件）、`THIRD_PARTY_NOTICES.md`、简版 `README_zh.md`（下载解压/SmartScreen/模型下载与代理/日志与配置目录位置/常见问题） | 0.5d | |
| **WD-4** | whisper 双源落地（D-21）：实测 MS 候选镜像仓六档 q5_1/q5_0 完整性 → `lt-models/src/registry.rs` 增 MS 条目 → 放宽 `always_hf`（backend.rs:124-129）为「所选 hub 优先，缺失回落另一 hub」→ 回归测试 | 0.5~1d | 网络实测为前置；MS 第三方镜像缺档风险见 §6 |
| **WD-5** | 二次启动反馈：第二实例当前仅 stderr 报错即退（双击场景用户看到"没反应"）；改为激活已有主窗口（FindWindow/SetForegroundWindow）或至少弹 MessageBox | 0.5d | Win32，主线程亲自做；单实例锁是 Rust 新增（原版无），配套体验需补齐 |
| WD-6（可选，待点头） | 首启缺模型轻引导：控制面板顶部一条「识别模型未就绪 → 去识别页下载」高亮横幅（模型就绪即隐） | 0.5d | D-19 的配套；原版无此场景（有向导），登记为新增偏差 |

## 4. 阶段二：公开发布（预案，D-18 届时重开渠道议题）

| 包 | 内容 | 估量 |
|---|---|---|
| WD-7 | CI `release.yml`（PLAN §147 蓝图：windows-latest 单作业 → build → zip+sha256 → tag 触发 attach Release；前置坑：LIBCLANG_PATH、sherpa-onnx sys 拉 GitHub 预编译库 → R-12 `actions/cache` + `SHERPA_ONNX_LIB_DIR`） | 0.5d |
| WD-8 | 检查更新按钮（D-20）：GET `releases/latest` 比对版本 → 提示 + 打开下载页；i18n zh/en 同步；失败静默 | 0.5d |
| WD-9 | 公开 README 双语完善 + 原 Python 仓 README 导流横幅；渠道裁决（新独立仓 vs 沿用原仓双产物） | 0.5d |
| WD-10（可选） | 「打开配置目录/日志目录」入口（现仅缓存页可开 models/transcripts 目录）；panic hook 崩溃尾部落盘，便于反馈 | 0.5d |

## 5. 用户到手全旅程（裁决后目标态）

**① 获取**：拿到 zip → 解压到任意目录（免安装，不放系统保护目录即可）。无 Python/运行库/字体任何前置要求——exe 全内嵌。

**② 首启**：双击 exe → 解压 ORT dll 与字体（首帧 ~100ms 级）→ 单实例检查 → 无 settings 全默认直进主界面（控制面板 + 悬浮窗 + 托盘）→ **管道立即自动启动**（`AppShell::new(Some(settings))`）→ 识别模型缺失 → 悬浮窗显示 unavailable → 用户到识别页「模型缓存」组点下载（`Cmd::StartDownload`，backend 现场重算缺失清单）→ 下载完成 `DownloadSucceeded` 自动重启管道（shell.rs:234-249）。D-19：无向导、无强制下载门；WD-6 可选横幅补引导。

**③ 首次出字幕的三要素**（fresh 默认值即为此设计）：
- **ASR 模型**：默认 funasr/sensevoice-small（int8 onnx ≈230MB），hub 默认 ModelScope（zh 环境大陆直连快）；whisper 六档经 D-21 后也可走 MS。
- **翻译 API**：默认预填本地 LM Studio（`http://127.0.0.1:1234/v1` + `hunyuan-mt-chimera-7b`，**key 留空**——不照搬原版硬编码 dev key）→ 用户在翻译页填自己的 key/端点（DeepSeek 等 OpenAI 兼容均可）。源语言=目标语言时直接回空译文不调 API（已移植，pipeline.rs:194）；API 失败非致命，overlay 显示 error 继续。
- **音频源**：默认系统默认 loopback（`audio_device=None`），麦克风默认关（`mic_device=None`）。

**④ 日常使用**：托盘四态图标（暂停/恢复、悬浮窗开关、面板、退出带确认）；字幕窗/悬浮窗独立；transcripts 自动落盘；日志窗 + `logs/` 文件双通道；字体内嵌保证任何机器渲染一致；改设置 300ms 防抖落盘。

**⑤ 升级**：换新 exe 即完成（配置/模型/日志/转写全在 `~/.config/livetranslate`，与 exe 位置解耦）；`from_value_compatible` 保证旧 settings 兼容。阶段二后由检查更新按钮承接告知（D-20）。

**⑥ 卸载/重置**：删 exe + 删 `~/.config/livetranslate` + 删开始菜单快捷方式（`%APPDATA%\Microsoft\Windows\Start Menu\Programs\LiveTranslate.lnk`——Toast 通知 AUMID 注册载体，C4 补记）即彻底清除；若改过 `models_dir`，模型缓存在自定路径需一并删除。重置 = 仅删该目录下 `settings.json`。

## 6. 风险与预案

| 风险 | 等级 | 预案 |
|---|---|---|
| 未签名 exe 触发 SmartScreen/杀软误报 | 中（分发即遇） | README 图文说明「仍要运行」；阶段二再议签名（RESEARCH 维持"自便"） |
| MS 第三方镜像仓缺档/停更（D-21「尽量」语义的边界） | 中 | WD-4 实测后只登记确认存在的档位；缺失档回落 HF+提示需代理；README 写明 |
| 无 CI 阶段人工发布步骤出错（忘测/忘 sha256/带脏产物） | 低 | WD-1 脚本一键化 + §3.2 手册 checklist |
| 检查更新依赖 api.github.com 大陆可达性（阶段二） | 低 | 失败静默降级，不打扰 |
| 模型文件无哈希校验（与原版一致） | 低 | 后续可在 registry 渐进登记 sha256，非本阶段 |
| 第二实例"静默无反应"引发用户困惑 | 中 | WD-5 激活已有窗口 |

## 7. 偏差登记汇总（本计划产生）

- **D-18** 暂不公开发布（阶段决策，公开时重开渠道）
- **D-19** 首启无向导直进主界面（既有 2026-09-01 决策转正；原版强制向导）
- **D-20** 应用内检查更新（新增能力，阶段二）
- **D-21** whisper 打破 always-HF 双源 + hub 缺失回落（新增偏差；原版 whisper 仅 HF）
- 关联既有偏差：配置目录 `~/.config` 非 exe 旁 portable（RESEARCH 既有）；翻译 API 默认 key 留空不硬编码；单实例锁为 Rust 新增。
