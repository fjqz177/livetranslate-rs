# NOTICES — 第三方组件与许可见证

LiveTranslate-rs 以 MIT 协议发布（见 LICENSE）。本文件列出随软件分发的**运行期**
第三方组件与内嵌资产及其许可（构建期工具链不在此列）。crate 完整列表以
`Cargo.lock` 为准；资产来源与 sha256 见仓库 `assets/SOURCES.md`。

## 运行库（随 exe 分发 / 编译链接）

| 组件 | 版本 | 许可 | 说明 |
|---|---|---|---|
| sherpa-onnx（onnxruntime + 内部依赖） | 1.13 | Apache-2.0 | 本地 ASR 推理运行时（worker 子进程静态链接） |
| onnxruntime.dll（主进程 VAD 用） | 1.29.0 | MIT | PyPI onnxruntime-1.29.0 wheel 内 capi 产物（未改动） |
| whisper-rs / whisper.cpp | 0.16 / 1.x | MIT | Whisper 引擎推理（纯 CPU） |
| ort（Rust onnxruntime 绑定） | 2.0.0-rc.13 | MIT | silero VAD 推理 |
| egui + egui-wgpu + egui-winit | 0.36 | MIT / Apache-2.0 | UI 框架（crate 自选 MIT） |
| winit | 0.30 | MIT / Apache-2.0 | 窗口系统抽象（crate 自选 MIT） |
| tray-icon | 0.24 | MIT | 系统托盘 |
| rfd | 0.15 | MIT / Apache-2.0 | 原生文件对话框 |
| tokio | 1.53 | MIT | 翻译/下载异步运行时 |
| reqwest | 0.13 | MIT / Apache-2.0 | HTTP 客户端（LLM/模型下载） |
| async-openai | 0.41 | MIT | OpenAI 兼容 API 客户端 |
| arc-swap | 1.9 | MIT / Apache-2.0 | 设置快照无锁读取 |
| sysinfo | 0.39 | MIT | 系统信息（Monitor 条） |
| uuid | 1.26 | MIT / Apache-2.0 | 消息/会话 ID |
| serde / serde_json | 1.0 | MIT / Apache-2.0 | 序列化（settings/契约） |
| windows | 0.62 | MIT / Apache-2.0 | Win32 API 绑定 |

## 内嵌资产

| 资产 | 来源 | 许可 |
|---|---|---|
| Noto Sans CJK SC（思源黑体 SC） | notofonts/noto-cjk Sans2.004 | SIL Open Font License 1.1 |
| Noto Sans Mono（可变字体） | google/fonts ofl/notosansmono | SIL Open Font License 1.1 |
| Noto Sans Symbols 2 | google/fonts ofl/notosanssymbols2 | SIL Open Font License 1.1 |
| silero_vad.onnx（Silero VAD v5） | snakers4/silero-vad（ModelScope 镜像 pengzhendong/silero-vad） | MIT |
| 应用/托盘图标 | 用户提供（2026-09-06） | LiveTranslate-rs 项目资产（MIT） |

字体 OFL 允许随软件再分发与嵌入；三字体完整许可原文随仓库
`assets/fonts/OFL.txt` 提供（打包分发时一并携带）。

## 备注

- 本软件**不包含**任何 AGPL 或 GPL 代码投毒：主流依赖均为 MIT/Apache-2.0/OFL 兼容链（sherpa-onnx 的 Apache-2.0 与 onnxruntime 的 MIT 均随分发保留上游许可声明）。
- 模型权重（FunASR SenseVoice / whisper.cpp 量化档 / Qwen3-ASR 等）由用户按需下载，其许可归各自上游（sherpa-onnx 模型仓库与 ggerganov/whisper.cpp 均 Apache-2.0 / MIT 许可），不在本软件分发物内。
