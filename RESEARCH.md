# LiveTranslate → Rust 完整重写研究文档

> 日期：2026-09-05 ｜ 修订：r2 纯 CPU ｜ r3 复核 ｜ r4 裁剪远程 ASR ｜ r5 async-openai ｜ r6 裁剪三个高危引擎变体 ｜ r7（音频跨平台抽象）｜ **r8（2026-09-05：ASR 引擎分析完结——用户决策：Nano 恢复实验性、Anime 确认裁剪、Whisper 加 turbo 档；whisper 引擎经补充调研采纳 whisper.cpp 双栈建议）**｜ 状态：研究完成并已对照源码复核，等待开工指令
> 原版：`LiveTranslate/`（Python 3.10 + PyQt6 + PyTorch，Windows 平台，MIT 协议）
> 目标：**单文件 exe、纯 CPU、零运行时依赖、功能与 GUI 尽可能 1:1**，配置放 `~/.config/livetranslate`，模型缓存路径可在配置中指定。
>
> **纯 CPU 硬性约束（r2 起）**：不引入 CUDA/DirectML/任何 GPU 运行时；全部推理走 CPU。ASR 推理栈历经两阶段：r2 曾统一到 sherpa-onnx 单栈，**r8 起改为双栈**（sherpa-onnx 承载 SenseVoice/Nano + whisper.cpp 承载 Whisper，动因见 §3.4/D-16），两条栈均为纯 CPU。

---

## 0. 结论速览（TL;DR）

| 决策点 | 结论 |
|---|---|
| GUI 框架 | **egui + winit 多窗口**（viewport API / `egui-multiwin`），全部 UI 自绘，真单二进制 |
| 托盘 | `tray-icon` + `muda`（菜单），事件经 `EventLoopProxy` 转入 winit 事件循环 |
| 系统音频采集 | **跨平台抽象层（r7 设计，§4.7）+ 平台后端**：Windows = `wasapi` crate（loopback + 麦克风 + 事件驱动 + 设备变更通知）；未来 macOS = `cpal` 0.17+（CoreAudio loopback，14.6+）/ Linux = `libpulse-binding`（`.monitor`，经 pipewire-pulse 覆盖两大阵营）或 `pipewire-rs`。重采样/混音/静音喂入等管道逻辑平台无关，置于公共层 |
| 重采样 | 自写线性插值重采样器（与原版 `numpy` 逐样本行为一致） |
| VAD | **Silero VAD v5 ONNX + `ort` crate（CPU EP）**（逐 32ms chunk 取置信度，VAD 状态机自写 1:1 复刻） |
| ASR | **双栈（r8）**：SenseVoice（默认）+ FunASR Nano（实验性）走 `sherpa-onnx`；**Whisper（5 档 + turbo）走 `whisper.cpp`（whisper-rs）**——beam=5 与质量对齐原版（sherpa whisper 仅 greedy 且有未解 CER 告警，见 §3.4）。Anime-Whisper 裁剪（D-13） |
| 远程 ASR | **不做（r4 裁剪，用户判断使用场景过小）**：remote-whisper 引擎、asr_server/asr_remote、设置页 Remote 组、`remote_asr_url` 键全部从范围内移除；axum 依赖随之删除（见 D-12） |
| LLM 翻译 | **`async-openai`**（r5；事实标准库，SSE/类型/错误全由库维护），`byot` 特性承载 extra_body；代理三模式经 `Client::build(自定义 reqwest, OpenAIConfig)`；`_build_request_kwargs()` 语义保留在我们自己的请求组装层 |
| 模型下载 | **自写统一下载器（r3）**：HF 直链 `https://{endpoint}/{repo}/resolve/main/{file}` + 树 API；ModelScope `api/v1/models/{id}/repo?FilePath=...`（均已验证）；缓存布局自控且与原版磁盘结构兼容；~~hf-hub~~（r3 移除，理由见 §3.7） |
| 进程模型 | 保留 **ASR worker 子进程**（崩溃隔离 + RSS 回收语义 1:1），stdin/stdout 二进制帧 IPC |
| 异步运行时 | `tokio`（多线程 runtime），翻译/下载/网络全部 async；音频/VAD/ASR 走专用 OS 线程 |
| 配置 | `~/.config/livetranslate/`（settings.json + config 默认值），新增 `models_dir` 键 |
| i18n / 资源 | zh/en YAML、changelog、图标、**Silero VAD 模型**全部 `include_str!/include_bytes!` 内嵌 |
| 发行 | **单一 CPU exe**（约 35–65MB，无 GPU 变体、无 CUDA 运行时要求）；MSVC 工具链 |

**当前主要风险**（r8 后）：① ort 与 sherpa-onnx 各自静态捆绑一份 ONNX Runtime 的符号共存（R-4，M0 首日验证）；② FunASR Nano 实验性质量（R-3，r8 随 D-14 恢复为**用户已知情接受**的风险）；③ egui 多窗口 + 托盘工程细节（R-6）；④ ModelScope files 接口未实测（R-5，低）；⑤ async-openai 流式 byot 入口待确认（R-13，低）。Anime-Whisper（R-2）与 sherpa whisper 质量（R-8）已分别随 D-13 裁剪与 D-16 换栈消除。CPU 性能不列入主风险——SenseVoice int8 单段 <1s（§3.2），弱机 whisper 性能另见 R-11。

### 0.1 r3 复核记录（2026-09-05，对照 Python 源码逐项复核）

**对照源码抽查通过的承重论断**：pad 分桶算法（asr_engine.py:104 / asr_sensevoice.py，量子=round(16000×pad)、尾零填充）✓；SenseVoice 语言标签首命中+`<\|[^|]+\|>` 剔除（asr_sensevoice.py:195-215）✓；IPC 超时 ready 180 / request 60(main 传入) / shutdown 5 / poll 0.2s / set_language·padding min(10,req)（asr_client.py:38-40,106-120,195）✓；远程二进制协议 `[u32 LE][lang][f32 LE PCM]`（asr_remote.py:86-89 / asr_server.py:56-60）✓；向导默认值块 vad_threshold 0.3 / min 1.0 / max 8.0 / silence 0.8（dialogs.py:348-353）✓；STYLE_PRESETS 逐字段（subtitle_overlay.py）✓；LANGUAGES=auto+29 语（i18n.py:42，修正了此前"30 语"笔误）✓；BENCH_SENTENCES 每语 5 句（benchmark.py:8）✓；VAD 状态机全部阈值（vad_processor.py 全文精读）✓；翻译器/音频采集/主管道循环（三文件全文精读）✓。

**复核发现并已修正的问题**：
1. hf-hub 1.0.0 缓存布局未确认与 Python 版兼容、断点续传未文档化 → **移除 hf-hub，双 hub 统一自写下载器**（§3.7），HF 走 `/resolve` 直链 + 树 API，缓存布局自控且与原版磁盘结构兼容；
2. Silero v5 ONNX 需调用方维护并回喂 `state[2,1,128]`（JIT 包装内部有状态，裸 ONNX 没有）→ 补 I/O 契约与版本对齐要求（§3.1）；
3. sherpa-onnx Whisper 的 beam_search 是否生效未获文档确认 → 从"可对齐"改为"实测后定，greedy 兜底"（§3.4 / R-8）；
4. Windows 点击穿透需 `WS_EX_LAYERED|WS_EX_TRANSPARENT` 成对 → 补实现注记（§2.1）；
5. worker 子进程可能比父进程活得久（原版 daemon 在父异常退出时同样残留）→ 加 Job Object KILL_ON_JOB_CLOSE（§4.3）；
6. 语言数量笔误（30→29）修正。

**复核后维持的结论**：egui 方案（窗口能力均经 docs.rs 核实）、wasapi loopback、子进程架构、事件通道模型、纯 CPU 性能判断。（注："sherpa 统一 ASR 栈"在 r8 修订为双栈，见 D-16。）原版 tests/ 四个测试（segmentation/thinking/startup/requirements）中 segmentation 与 thinking 两个直接钉住了必须 1:1 的行为，已列入移植清单（附录 B）。

---

## 1. 原版功能全景（必须保留的清单）

### 1.1 核心管道

```
系统音频 (WASAPI loopback, 32ms chunk, 16kHz mono float32)
   ↑ 可选麦克风混音（等长混入，mic RMS 单独上报）
→ VAD（Silero/能量/禁用 三模式）
   - 预语音环形缓冲（3 chunk ≈ 96ms 防吃字头）
   - 渐进静音：<3s 全量 / 3-6s 半 / 6-10s 1/4 静音上限
   - 自适应静音：P75×1.2，夹在 0.3~2.0s（pause_history 最近 50 条）
   - 最大时长回溯切分：平滑置信度（5-chunk 滑窗）找后 70% 区域最低谷
   - 语音密度过滤：<25% chunk 过阈值 → 丢弃
   - 短段合并不丢弃；trim 后的剩余段 force_flush
→ ASR（worker 子进程）
   - 段级识别 + 增量识别（interim，每 interim_interval 秒对缓冲全量识别 → yasbd 分句 → 提交完整句 → 按比例+0.3s 余量裁剪音频 → 回声去重）
   - 噪声过滤：≥2s 段产出 ≤3 字符丢弃；空/纯标点丢弃
   - 语言过滤：用户指定语言 ≠ 检测语言 → 丢弃
→ 翻译（异步线程池，8+ worker）
   - OpenAI 兼容 API、流式 SSE 逐字显示（50ms 节流）、JSON schema 结构化输出、
     thinking 禁用 6 风格、上下文历史、每模型 overrides/extra_body、代理三模式、
     重复环检测（pattern ≥8 重复）、TTFT/耗时/token/费用统计
→ 展示
   - 主悬浮窗（HTML 富文本消息流）、OBS 字幕窗（描边大字 + 动画）、
     转写文件落盘（transcripts/*.txt 三文件）
```

### 1.2 窗口清单（全部要还原）

| 窗口 | 关键特性 |
|---|---|
| 主悬浮窗 SubtitleOverlay | 无边框+置顶+Tool、透明、两行 DragHandle（7 按钮+4 复选框+3 下拉）、MonitorBar（MIC/RMS/VAD 进度条 + CPU/RAM/GPU/ASR/TL/Tok/费用）、消息流（50 条上限）、右键菜单（复制×3/导出×3/清空）、compact 模式 200ms 动画、光标感知点击穿透（header 可点/正文穿透，50ms 轮询）、位置持久化（500ms debounce）、14+1 样式预设 |
| OBS 字幕窗 SubtitleWindow | 无边框+置顶+透明、中键拖动、QPainterPath 描边文字、自动换行（标点优先）、每行独立字体/颜色/描边/对齐/入场出场动画（none/fade/slide×4）、高度 150ms OutCubic 动画且垂直居中、自动隐藏+反向恢复动画、1500ms 最短显示、多句缓冲（" \| " 连接）、背景图/纯色/全透明、点击穿透（500ms 重断言）、额外目标语言并行翻译 |
| 控制面板 ControlPanel | 7 Tab（VAD/ASR、翻译、样式、字幕、基准测试、缓存、更新日志），全部自动保存（300ms debounce），高 DPI 自适应（内容尺寸 + 滚动） |
| 日志窗口 LogWindow | 2000 行环形缓冲、级别过滤（默认 INFO+）、内容高亮（ASR 青绿/Translate 浅蓝/Segment 橙棕）、自动滚动开关 |
| 各类对话框 | 首启向导（单页 + 15s 倒计时自动开始）、模型下载对话框（日志流式显示）、模型加载对话框、模型编辑对话框（Basic 12 字段 + Advanced 6 覆盖行 + extra_body）、字幕行编辑对话框（16 字段）、颜色选择、文件选择 |

### 1.3 托盘菜单（完整结构）

```
暂停/恢复（文字切换）
────────────
隐藏/显示悬浮窗（首次隐藏弹气泡提示）
字幕窗口（checkable，首次显示弹拖动提示）
字幕窗点击穿透（checkable）
────────────
显示控制面板
显示日志
▶ 悬浮窗子菜单：鼠标穿透/始终置顶(默认√)/自动滚动(默认√)/任务栏
▶ 模型子菜单：单选组（动态重建，活动模型打勾）
▶ 目标语言子菜单：常用 9 语 + "更多语言"子菜单（共 29 语）
▶ 源语言(ASR提示)子菜单：auto + 29 语
▶ 导出子菜单：导出原文/导出译文/导出全部
────────────
退出
```
另：内存超限（主+worker RSS ≥4096MB）托盘气泡警告一次。

### 1.4 高层生命周期语义（必须 1:1 的"隐形功能"）

- **启动流程**：无 settings → 向导（hub 按系统语言默认：zh→ModelScope，else→HF；代理三模式；15s 倒计时自动下载 Silero+SenseVoice-small）→ 提示配置翻译 API（预填 LM Studio 本地示例）→ 主 UI。有 settings 但模型缺失 → 下载对话框，取消即退出。全部就绪 → 延迟 100ms 初始化（面板应用设置 → overlay 填充 → 活动模型生效），500ms 后自动启动管道。
- **ASR worker 状态机**：generation 号防新旧竞态；加载失败自动回滚到上一个引擎配置；运行中死亡自动重启（上限 3 次，成功一次即清零）；transcribe 连续 3 次错误或不可恢复错误 → 标记不可用；worker RSS 超基线 +2048MB 时空闲时主动回收重启；语言/padding 运行时修改先挂 pending、在 ASR 线程下一次 transcribe 前下发，并写回 restart config 防重启回退。
- **引擎切换**：签名 `(engine, model, device, hub, compute)` 相同则跳过；切换时先停旧 worker 再加载新的（_ModelLoadDialog 阻塞 + 100ms 轮询）；未缓存模型先弹下载框，取消则保持现状。
- **管道暂停/恢复**：暂停清空 interim 状态、monitor 归零；音频设备切换时 VAD flush+reset；默认输出设备变更（每 2s 检查）自动重连。
- **静音喂入**：音频超时（无数据 1s）且正在说话时，喂入静音 chunk 强制推进 VAD 状态机（保证说完的话能切出来）。

### 1.5 刻意偏差清单（相对原版的**有意**差异，除此之外全部 1:1）

| # | 偏差 | 原因 |
|---|---|---|
| D-1 | **纯 CPU**：无 CUDA/DirectML/任何 GPU 路径；`asr_device`（cuda/cpu 下拉）从设置界面移除；worker 配置无 device 字段 | 用户明确要求（r2），换最大兼容性与开发便利 |
| D-2 | `compute_type`（float16/int8...）设置移除：全部模型固定加载量化档（sherpa=int8 ONNX；whisper.cpp=q5_0/q8_0 GGML，≈原版 CPU 时的自动降级） | 同上，且 UI 少一列无意义选项 |
| D-3 | Whisper 模型仓库替换：`Systran/faster-whisper-*`（CT2）→ **`ggml-org/whisper-*`（GGML，r8 随 D-16 定稿）**；本地模型识别布局同步改为 GGML `.bin` 文件 | Rust 无 CTranslate2 可用绑定 |
| D-4 | SenseVoice / Nano 模型仓库替换为 sherpa-onnx 官方 ONNX 转换仓（见 §3.8） | 推理栈选择 |
| D-5 | Silero VAD 模型内嵌 exe（~2MB），首启少下载一项；向导步骤保留（秒完成） | 单文件分发目标 |
| D-6 | MonitorBar 的 GPU 段恒显示 "N/A"（不引入 nvml-wrapper） | 纯 CPU；与原版无 N 卡机器的显示一致 |
| D-7 | `neutralize_funasr_requirements`（改名 requirements.txt）删除 | 无 Python trust_remote_code 隐患 |
| D-8 | ~~Nano 引擎不再下载 Qwen3-0.6B~~（已并入 D-13 整体裁剪） | 随 D-4 |
| D-9 | 配置目录 `~/.config/livetranslate`（含 settings.json、logs/、transcripts/ 全部从原版程序目录迁入配置目录）、新增 `models_dir` 键 | 用户要求 |
| D-10 | ~~FunASR Nano 标"实验性"、Anime-Whisper 待自转换~~（已并入 D-13 整体裁剪） | 上游模型可用性 |
| D-11 | 单实例互斥量（防双开）、Ctrl+C 优雅退出 | 新增便利性，不改变原行为 |
| D-12 | **远程 ASR 功能整体裁剪**：引擎下拉仅 3 项（whisper/funasr/anime-whisper，前三项索引与原版一致）、设置页 "Remote ASR Server" 组不实现、`remote_asr_url` 键忽略、`asr_engine=="remote-whisper"` 导入时回退 funasr；axum 依赖删除 | 用户判断使用场景过小（r4） |
| D-13 | **Anime-Whisper 裁剪**（r6 决定，r8 用户最终确认）。引擎下拉移除该项（余 2 项：Whisper idx0 / FunASR idx1，索引与原版前两项一致）；`asr_engine == "anime-whisper"` 导入时回退 funasr 并记日志。理由：① 仅 CT2 格式无现成转换、自转换需永久自托管；② 原版特调防重复生成参数（no_repeat_ngram_size=5 等）在可用推理栈中无等价物；③ 模型层质量问题无力修复。若未来社区产出可用转换可经引擎枚举加回。（r6 版本曾连带裁剪 Nano 系，该部分已被 D-14 取代恢复） | 用户确认（r6 方向 + r8 定稿） |
| D-14 | **FunASR Nano / MLT 恢复为实验性**（r8 用户决定，取代 D-13 的 nano 裁剪部分）：`funasr_model` 下拉恢复 3 项 1:1 结构——sensevoice-small（默认）/ funasr-nano-2512（标实验性）/ funasr-mlt-nano-2512（置灰，待上游转换）；UI+文档明示 CPU 慢（2-4s/段）与转换质量告警；不移植 Qwen3 下载链路 | 用户决定（r8） |
| D-15 | **Whisper 档位增加 turbo**（r8 用户决定）：6 档 = tiny/base/small/medium/large-v3/turbo（`ggml-org/whisper-large-v3-turbo`，~809MB，速度≈medium 质量≈large-v3）；large-v3 保留但 UI 标注 CPU 慢 | 用户决定（r8），超出原版的增补 |
| D-16 | **whisper 引擎使用 whisper.cpp（GGML）而非 sherpa 统一栈**（r8，经用户授权的补充调研后采纳）：动因是 sherpa whisper 确认仅 greedy（官方 #671 无计划支持 beam）+ 未解的 3 倍 CER 告警（#2900）；代价 = 第二条原生构建链 + GGML 格式 + 二进制 ~3-4MB | 质量对齐优先（r8） |

---

## 2. Rust 技术选型

### 2.1 GUI 框架：为什么是 egui（+ winit）

候选对比：

| 候选 | 单exe | 透明/穿透/置顶 | 多窗口 | 托盘 | 自绘描边文字 | 富文本列表 | 结论 |
|---|---|---|---|---|---|---|---|
| **egui + winit** | ✅ 静态编译 ~40MB | ✅ `with_transparent` + `with_mouse_passthrough` + `with_always_on_top` + `with_taskbar`（已验证 docs.rs） | ✅ viewport API（0.24+）/ `egui-multiwin` | ✅ tray-icon（Tauri 团队） | ✅ epaint 自绘 | ✅ RichText | **采用** |
| iced | ✅ | 透明勉强/穿透弱 | 弱 | 三方 | 可 | 可 | 排除 |
| Tauri v2 | ~（依赖 WebView2） | ✅ | ✅ | ✅ 内置 | Canvas 可 | HTML | 排除：UI 全要写成 JS，且非严格单文件 |
| Slint | ✅ | 弱 | 一般 | 三方 | 一般 | 一般 | 排除 |
| Qt 绑定（cxx-qt） | ❌ 需带 Qt DLL | ✅ | ✅ | ✅ | ✅ | ✅ | 排除：违背单 exe 目标 |

egui 的决定性优势：
1. **即时模式 + 全自绘**：原版大量依赖 QSS/HTML/QPainterPath 自定义视觉，egui 等价物全是"每帧画"，一一对应。
2. **窗口能力齐全**（已验证）：`ViewportBuilder::with_transparent / with_mouse_passthrough / with_always_on_top / with_taskbar(bool) / with_decorations(false) / with_active(false) / with_visible(false)`。
3. **多窗口**：主悬浮窗、字幕窗、控制面板、日志窗 = 4 个常驻原生窗口。egui 官方 viewport API 支持（`ctx.show_viewport_deferred`），或 `uglyoldbob/egui-multiwin`（winit 多窗口 + 每窗口一个 egui 上下文，有现成 example）。**推荐底座：winit 事件循环 + 每窗口独立 egui_winit::State + glow/wgpu 渲染**（egui-multiwin 模式），比 viewport API 控制力强（各窗口可独立设 window flags、独立 DPI、独立透明）。
4. **动画**：原版动画全靠 QPropertyAnimation + 缓动曲线；egui 用 `emath::easing`（含 `out_cubic`）+ 时间插值 + `ctx.request_repaint_after()` 逐帧驱动，语义等价。
5. **无 GC、启动快、内存可控**。

**多窗口 + 托盘事件循环骨架**（一个进程，一个 winit EventLoop）：

```
EventLoop::with_user_event()
  ├─ tray-icon::TrayIconEvent / muda MenuEvent → 全局 channel → winit EventLoopProxy 唤醒
  ├─ Window A: overlay   (transparent, always-on-top, no-decorations, skip-taskbar)
  ├─ Window B: subtitle  (transparent, always-on-top, no-decorations)
  ├─ Window C: panel     (normal, decorations)
  ├─ Window D: log       (normal, decorations, 初始隐藏)
  └─ 模态对话框：在所属窗口内画 egui 层（Modal 层），或临时 viewport
每帧：drain 各 channel（管道事件、托盘事件、日志、设置变更）→ 更新 AppState → 各可见窗口重绘
穿透：50ms 定时器轮询光标位置（windows-rs GetCursorPos + WindowRect），
     header 区 → 清除 WS_EX_TRANSPARENT；正文区 → 设置（SetWindowLongW GWL_EXSTYLE），
     与原版 _ct_timer 逐字节同策略。注意 WS_EX_TRANSPARENT 需与 WS_EX_LAYERED 成对
     （Qt 的 WA_TranslucentBackground 隐含 layered；winit 透明窗口同样满足，设置时一并确保）。
     字幕窗整窗穿透 500ms 重断言。
```

### 2.2 依赖清单总表

| 子系统 | crate | 版本/特性 | 说明 |
|---|---|---|---|
| GUI | `egui`, `egui-winit`, `egui-glow`（或 `egui-wgpu`） | 0.32+ | glow 后端体积更小；透明窗口两者都支持 |
| 窗口/事件 | `winit` | 0.30+ | 多窗口；`WindowExtWindows` 补 skip-taskbar 等 |
| Win32 互操作 | `windows` | 0.6x | GWL_EXSTYLE 穿透、光标查询、DPI、explorer 打开、命名互斥量 |
| 托盘 | `tray-icon`, `muda` | 最新 | Tauri 系；图标运行时生成（复刻 LT 蓝底圆角图标） |
| 音频采集 | `wasapi`（Windows 后端，r7 起藏于抽象层后） | 0.15+ | loopback/麦克风/事件驱动/设备通知（已验证 GitHub README）。**cpal 0.17 已验证不适用于 Windows loopback**（issue #251 仍开放，0.17 的 loopback 只加了 macOS）；未来后端：macOS `cpal` 0.17+（CoreAudio process tap，14.6+）、Linux `libpulse-binding`（`.monitor` source，pipewire-pulse 兼容层同时覆盖 PulseAudio/PipeWire）或 `pipewire-rs` 原生 |
| VAD 推理 | `ort` | 2.0.0-rc.x | ONNX Runtime 绑定，**仅 CPU EP**；SilentKeys 项目同款用法（Silero+ort）已验证 |
| ASR（sherpa 栈） | `sherpa-onnx` | 1.13.7 | 官方 Rust API，默认静态链接，构建时自动下预编译库（CPU 版）；承载 **SenseVoice + FunASR Nano**（已验证 docs.rs） |
| ASR（whisper 栈） | `whisper-rs` | 0.14+ | whisper.cpp 绑定（维护于 codeberg）；r8 恢复采用：beam=5 对齐原版、完整解码策略（temperature fallback）、CPU 性能最强、GGML 量化模型；仅 whisper 引擎使用，藏于 AsrEngine trait 后 |
| LLM 客户端 | `async-openai`（features: `chat-completion-types`, `byot`, rustls）+ `reqwest`（仅用于造注入的 http client）+ `serde_json` | 0.41.x | r5 采纳：SSE 解析/`[DONE]`/chunk 类型/usage 提取/ApiError 全由库维护。已验证：自定义 base_url（OpenAIConfig）、`Client::build(http_client, config)` 注入自定义 reqwest（代理三模式：`no_proxy()`/默认环境/`Proxy::all(url)`）、`byot` 支持 serde flatten 扩展 extra_body、`ChatCompletionStreamOptions.include_usage`、`ResponseFormat::JsonSchema`。~~eventsource-stream~~ 手写 SSE 移除 |
| 模型下载 | **自写**（r3，reqwest） | — | 双 hub 统一下载器：HF `/resolve` 直链（llama.cpp 同款方式）+ ModelScope API；断点续传/进度/代理全自控；~~hf-hub~~ 移除 |
| 异步 | `tokio`（rt-multi-thread, macros, net, time, fs, process, signal） | 1.x | 翻译并发、下载 |
| 序列化 | `serde`, `serde_json`, `serde_yaml` | 1.x | settings.json、i18n yaml、模型配置条件序列化 |
| 系统/路径 | `dirs`（home_dir → `~/.config/livetranslate`）, `anyhow`, `thiserror` | — | 用户明确要求 `~/.config`（非 %APPDATA%），按需求执行 |
| 系统监视 | `sysinfo`（CPU/RAM） | 最新 | MonitorBar + 内存诊断。GPU 段恒显示 "N/A"（与原版在无 N 卡机器上的表现一致，视觉 1:1），不引入 nvml-wrapper |
| 单实例 | windows 命名互斥量（`windows` crate 自写 20 行） | — | 防双开 |
| 日志 | `tracing` + `tracing-subscriber`（fmt + rolling file）+ 自定义 broadcast layer | — | 文件 `logs/livetrans_*.log`（utf-8，DEBUG 级）+ 控制台 INFO + 日志窗口订阅 |
| 时间 | `chrono` | — | 时间戳/文件名 |
| 音频工具 | 自写（无需 crate） | — | 线性重采样、多声道平均、RMS、零填充分桶——共 ~100 行，保证与 numpy 行为逐位一致 |
| 分句 | 自写（~150 行） | — | 复刻 yasbd 核心规则 + main.py 的 CJK/西文逗号回退（25/60 字阈值、before>15/after>3） |
| 随机 ID | `uuid`（v4 简洁 hex） | — | IPC 消息 id |
| 图标编码 | `image`（可选，仅构建期）或直接嵌 PNG | — | 托盘图标 + 窗口图标 |
| 嵌入式资源 | `include_str!`/`include_bytes!` | — | i18n、changelog、图标、默认配置 |

音频备选说明：不选 `cpal`——其 WASAPI loopback 支持不完整；`wasapi` crate 原生 loopback + 事件驱动 + 设备变更回调，是 pyaudiowpatch 的直接等价物。

### 2.3 Qt → egui 概念对照表

| PyQt6 | egui/Rust 等价 | 备注 |
|---|---|---|
| QThread + pyqtSignal(queued) | `std::thread`/tokio task + `std::sync::mpsc`/`crossbeam-channel`，主循环每帧 drain | 所有 UI 更新走事件通道，无锁竞争 |
| QTextBrowser/HTML | `RichText` + `LayoutJob`（彩色 span） | 消息 HTML 拆成 span 序列；escape 语义保留 |
| QPropertyAnimation + OutCubic | 时间戳插值 + `emath::easing::out_cubic` + `request_repaint_after(16ms)` | compact 200ms / 高度 150ms / 文字进出场 |
| QPainterPath 描边文字 | epaint 文字两遍绘制：8 方向偏移描边色 → 中心填充色 | 或 glyph 轮廓扩展；视觉效果等价，实现更简单 |
| QSS 样式表 | 每帧根据 Style 结构取色 | 14 预设 → 同名 struct 数组 |
| QComboBox | `egui::ComboBox` | 保持 itemData 语义：`Vec<(label, value)>` |
| QTimer singleShot | `Instant` + 帧内检查（AppState 里的 pending timers） | debounce 300/500/600ms 同款 |
| QSystemTrayIcon + QMenu | `tray-icon` + `muda`（submenu/checkbox/radio 全支持） | 菜单动态重建同款 |
| WS_EX_TRANSPARENT 光标轮询 | `windows` crate 同 API，50ms 帧内定时 | 完全同策略 |
| 原子写（tmp + os.replace） | `std::fs::rename`（Windows 上即 MOVEFILE_REPLACE_EXISTING 语义） | 同款 |
| multiprocessing spawn + Pipe | `std::process::Command` + stdin/stdout 管道 + 二进制帧 | 见 §4.3 |
| logging.Handler 广播 | tracing 自定义 Layer → `tokio::sync::broadcast` | 日志窗口/下载对话框共用 |

---

## 3. 模型与推理方案映射（模型仓库确定）

### 3.1 Silero VAD

- 原版：PyPI `silero-vad` wheel 内嵌 JIT 模型，`model(torch_tensor_512, 16000) → conf`。
- Rust：**`silero_vad.onnx`（v5）+ `ort`**（CPU EP，单线程足以）。逐 chunk（512 样本 @16k）喂数据取标量置信度，VAD 状态机自写——这是唯一能 1:1 复刻渐进静音/自适应静音/回溯切分的方案（sherpa-onnx 的 VAD 是它自己内置切分逻辑，不可控，不采用）。
- **v5 ONNX I/O 契约（r3 补）**：输入 `input[1,512] f32`、`state[2,1,128] f32`、`sr`；输出 `output[1,1]`（置信度）+ 新 `state`。**调用方必须保存并把 state 回喂下一 chunk**（等价 Python JIT 包装内部维护的 LSTM 状态）；初始 state 全零；`sr` 不匹配 16000 会重置内部缓存。封装成一个带状态的 `SileroVad { conf(&mut self, chunk: &[f32; 512]) -> f32 }` 结构即可。
- 下载源：HF `snakers4/silero-vad`（`src/silero_vad/data/silero_vad.onnx`）或 GitHub release；体积 ~2MB。**Rust 版内嵌进 exe**（`include_bytes!`）→ 用户零下载，替代原版"torch.hub 下载"整条链路（向导里仍保留展示步骤但秒完成）。**版本对齐（r3 补）**：必须取与原版 requirements `silero-vad>=5.0` 同大版本（5.x）的 onnx 权重，保证逐 chunk 置信度数值与 Python 版一致（VAD 阈值语义才有可比性）；实现期内嵌时记录该文件的来源 commit/hash。

### 3.2 SenseVoice Small（默认引擎，最高优先级保证）

| | 原版 | Rust 版 |
|---|---|---|
| 推理 | funasr `AutoModel`（PyTorch，trust_remote_code） | `sherpa-onnx` `OfflineRecognizer` + `OfflineSenseVoiceModelConfig`（已验证官方支持） |
| 模型源 | MS `iic/SenseVoiceSmall` / HF `FunAudioLLM/SenseVoiceSmall` | 同 repo 的 ONNX 化产物：**`csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`**（HF，官方转换，含 `model.int8.onnx` + `tokens.txt`）。ModelScope 侧：`csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`（已镜像，实现期核对） |
| 语言检测 | 输出内嵌 `<|zh|>/<|en|>/<|ja|>/<|ko|>/<|yue|>` 标签 | sherpa-onnx 输出同样带标签（同源模型），**沿用同一套剥离/检测正则**：首个命中标签定语言并删除，其余 `<|...|>`（情感/事件/BGM）全部剔除 |
| 参数 | `use_itn=True`、fp16(cuda) | sherpa `language: auto/zh/en/ja/ko/yue`、`use_itn: true`；纯 CPU int8（r2） |
| pad 桶 | 0.5s 量子尾零填充 | 同款自写函数（`supports_padding=true`） |
| 体积 | ~940MB | int8 onnx ~230MB（fp32 ~918MB），估算值按实际调整 |

### 3.3 Fun-ASR-Nano / Fun-ASR-MLT-Nano【r8：恢复为实验性（用户决定）】

- **r6 曾裁剪，r8 用户决定恢复为实验性**。运行时仍走 `sherpa-onnx`（官方支持 FunASR Nano，模型页 `k2-fsa.github.io/sherpa/onnx/funasr-nano/pretrained.html`，HF 源 `csukuangfj/sherpa-onnx-funasr-nano-*`，实现期按文档页核对具体包与质量）。
- **实验性标注（UI + 文档明示）**，用户需知晓的三条风险：
  1. CPU 性能：0.6B Qwen3 LLM 解码器架构，纯 CPU 约 2–4s/段（默认引擎 SenseVoice <1s）；
  2. 转换质量告警：#3066（int8 重复文本/不准）、#3061（部分 "nano" 包实为 SenseVoice 换皮，实现期须甄别真 nano 转换包）；
  3. **MLT 变体无现成转换** → `funasr_model` 下拉保留 3 项 1:1 结构（sensevoice-small 默认 / funasr-nano-2512 标"实验性" / funasr-mlt-nano-2512 置灰不可选，tooltip 说明待上游转换）。
- 需移植的原版 nano 专属逻辑：语言启发式检测（假名→ja / 谚文>30%→ko / 汉字>30%→zh / else en / 空→auto）、`/sil`→空格等后处理、`supports_padding=false`。**不需要** Qwen3-0.6B 下载链路（ONNX 转换已内联权重；原版 `ensure_qwen_weights` 不移植）。
- `funasr_model` 设置键恢复 3 值枚举；导入非法值回退 sensevoice-small 并记日志。

### 3.4 Whisper（faster-whisper → whisper.cpp）【r8：采纳双栈建议，whisper 引擎回归 whisper.cpp】

- 原版：CTranslate2，`Systran/faster-whisper-{tiny,base,small,medium,large-v3}`，`beam_size=5`，`compute_type`（CPU 自动降 int8），word_timestamps 可选。
- **r8 决策背景**：r2 曾把 Whisper 也并入 sherpa-onnx 统一栈；复核发现两条硬证据后改变结论——① sherpa 官方确认 Whisper 仅支持 greedy 解码且无计划支持 beam（discussion #671），与原版 beam_size=5 构成确定差异；② issue #2900 报告 sherpa whisper（多语 tiny，中文）CER 0.81 vs faster-whisper 0.25 的 3 倍差距，issue 关闭无根因无修复。Whisper 是用户主动选择的"准确档"，质量未经验证不可接受。**whisper.cpp（`whisper-rs` 绑定）是参考级实现**：完整 OpenAI 解码策略（beam/temperature fallback/token suppression）、CPU 性能最强（ggml 内核 + 量化）、生态最成熟——恰是用户偏好的"权威库"模式。
- 模型源：**`ggml-org/whisper-{tiny,base,small,medium,large-v3}`** + **`ggml-org/whisper-large-v3-turbo`**（r8 用户决定增加 turbo 快档，约 809MB、速度接近 medium、质量接近 large-v3）。一律走 HF（与原版"whisper 永远走 HF"语义一致）。仓内含 fp16 与量化（`-q5_0`/`-q8_0`）变体，默认取量化档（对应原版 CPU 自动降 int8）。
- 参数对齐：`beam_size=5` 直接设置（whisper.cpp 原生支持）；language/task 设置；word_timestamps 支持（token 级）。R-8 就此解决。
- 本地自定义模型：扫描 `models/**` 下 **GGML `.bin` 文件**（whisper.cpp 可直接加载验证），UI 语义不变（"Whisper Local: xxx"）。
- 架构影响：`AsrEngine` trait 后的 whisper 实现换为 whisper.cpp，worker/下载/UI 均无感知；sherpa-onnx 保留承载 SenseVoice/Nano。二进制 +3~4MB、多一条原生构建链（whisper-rs 经 cc 编译，MSVC 验证成熟）——为质量对齐支付的明确代价。

### 3.5 Anime-Whisper（ja 动漫/galgame）（~~~~）【r6 裁剪，r8 用户确认】

- **裁剪决定（D-13，用户确认）**。原风险三重叠加：
  1. `litagin/anime-whisper`（kotoba-whisper-v2.0 微调）**仅 CT2 格式**，sherpa-onnx 吃 ONNX、无官方转换——需要自建转换管道 + 自托管模型仓库，我们成为该制品的维护人；
  2. 原版为 galgame 语音特调的生成参数（language=Japanese、no_repeat_ngram_size=5、num_beams=1、禁 initial_prompt）在 sherpa whisper 解码中**无等价物**——其中防重复参数恰是其可用性关键，移植后行为漂移概率高；
  3. 模型层质量问题我们无力修复；受众（ja 动漫/galgame）由 SenseVoice 的日语识别基本覆盖。
- 引擎下拉与 `ASR_DISPLAY_NAMES` 中移除该项；`asr_engine == "anime-whisper"` 导入时回退 `funasr` 并记日志。
- 若未来官方或社区产出可用 ONNX 转换（且解码行为验证无重复问题），可经引擎枚举加回。

### 3.6 Remote Whisper（~~远程 GPU 服务器~~）【r4：整体裁剪】

- **应用户决定不做**（使用场景过小）：remote-whisper 引擎、`asr_server.py`（FastAPI 服务端）、`asr_remote.py`（客户端）、设置页 "Remote ASR Server" 组、`remote_asr_url` 设置键、自定义二进制协议（`[u32 LE][lang][f32 LE PCM]` over HTTP）全部从重写范围移除；`axum` 依赖随之删除。
- 引擎枚举（r4/r6 后收敛为 2 个）：`whisper | funasr`（funasr 下 3 个模型选项，D-14）；导入原版 user_settings.json 时：`asr_engine ∈ {remote-whisper, anime-whisper}` → 回退默认 `funasr`（记日志）；`remote_asr_url` 键忽略。
- 附带简化：原版 `_load_engine_client` 中"remote 引擎主进程内直跑、不建子进程、pid=None 跳过内存回收"的整个特判分支消失，引擎生命周期只剩子进程一条路径。

### 3.7 模型下载子系统【r3 修订：移除 hf-hub，双 hub 统一自写下载器】

**为什么移除 hf-hub**：复核发现 hf-hub 1.0.0 API 重写为 builder 模式，其缓存虽为"content-addressed"但**磁盘布局是否保持 Python 版 `models--org--name/blobs/snapshots` 结构未获文档确认**，断点续传行为也未文档化——而我们恰恰要靠布局兼容来复用原版的缓存探测/双 hub 复用逻辑。与其赌第三方实现，不如两个 hub 都走自写下载器（各 ~150 行），布局、进度、续传、代理完全自控。HF 文件下载本质就是公开直链 `GET https://{endpoint}/{org}/{repo}/resolve/main/{file}`（302 到 CDN，支持 Range），llama.cpp 等项目即用此方式；文件树 `GET /api/models/{repo}/tree/main?recursive=true`。

| 项 | 方案 |
|---|---|
| HF | 自写：树 API 列文件 → 逐文件 `/resolve/main/{file}` 直链下载（Range 续传）。endpoint 可覆写（`https://hf-mirror.com` 透传 path 即可） |
| ModelScope | 自写：`GET /api/v1/models/{id}/repo/files?Revision=master` 列文件 → 逐文件 `GET /api/v1/models/{id}/repo?Revision=master&FilePath=...`。实现期实测 files 接口响应结构（分页字段，R-5） |
| 磁盘布局 | **自控且与原版兼容**：`<models_dir>/huggingface/hub/models--{org}--{name}/snapshots/{rev}/...`（HF，rev 取实际 commit 或 `main`）与 `<models_dir>/modelscope/models--{org}--{name}/snapshots/master/...`（MS）。文件直落 snapshot 目录（无 blob/符号链接层，Windows 友好；原版探测逻辑只看 snapshot 树，兼容）。**原版 `_hf_repo_complete`（50MB/半体积阈值）、双 hub 或逻辑、字典序取最后 snapshot 等探测代码可 1:1 移植** |
| 代理 | 三模式 none/system/url：none → `Client::builder().no_proxy()`；url → `Proxy::all()`；system → 默认（读环境变量）。统一套在下载器 client 上 |
| 断点/重试 | `.incomplete` 临时文件 + `Range: bytes=N-` 续传 + 3 次指数退避；完成后原子改名。与原版探测"忽略 .incomplete"语义闭环 |
| 进度 | 每文件进度 + 总进度事件发日志广播（对齐原版"进度走日志流"的 UI 形态），速度/剩余量可显示（超越原版） |
| `neutralize_funasr_requirements` | 删除（无 Python 子进程隐患，D-7） |
| 缓存页/删除退出 | `get_cache_entries()` + `dir_size()` + `format_size()` 1:1 移植；"删除全部并退出" rmtree 保留 |

### 3.8 模型注册表（Rust 版汇总，含原版 repo 对照）

```yaml
silero-vad:        { embedded: true }                                   # 内嵌，~2MB，CPU EP
sensevoice-small:  { display: "SenseVoice Small",                       # 默认 funasr 模型
                     hf: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
                     ms: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",  # 实现期核对
                     bytes: ~250MB, padding: true, lang_tags: true, default_engine: true }
funasr-nano-2512:  { display: "Fun-ASR-Nano", experimental: true,          # r8 恢复（D-14）
                     hf: "csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30",   # 实现期核对文档页并甄别真 nano 包
                     ms: 同左, bytes: ~1.1GB, padding: false, heuristics_lang: true }
funasr-mlt-nano:   { display: "Fun-ASR-MLT-Nano", ui: 置灰, status: 待上游转换 }
whisper-tiny..large-v3 + turbo: { hf: "ggml-org/whisper-{tiny,base,small,medium,large-v3,large-v3-turbo}",
                     always_hf: true, files: "GGML *.bin（默认量化 q5_0/q8_0 档）",
                     bytes: {tiny ~78M, base ~148M, small ~488M, medium ~1.53G, large-v3 ~3.1G, turbo ~809M} }
# anime-whisper 移除（D-13，r8 确认）
```
全部引擎 CPU 推理；无 compute_type/device 概念（见 §6.3 的 UI 简化）。引擎枚举：`whisper | funasr`（funasr 下 3 个模型选项，见 D-14）。whisper 栈 = whisper.cpp，sherpa 栈 = SenseVoice/Nano（D-16）。

---

## 4. Rust 版软件架构设计

### 4.1 Workspace 布局（单 crate 也行，建议 workspace 便于测试）

```
livetranslate-rs/
├── Cargo.toml                 # workspace
├── crates/
│   ├── lt-app/                # 主二进制：main.rs（启动流程、事件循环、托盘、装配）
│   ├── lt-ui/                 # egui 界面：overlay / subtitle_win / panel(7 tabs) / log_win / dialogs
│   ├── lt-pipeline/           # audio_capture / vad(状态机) / segment 逻辑 / incremental asr / transcript
│   ├── lt-asr/                # trait AsrEngine + sherpa 各引擎实现 + worker 子进程协议
│   ├── lt-translate/          # Translator（1:1 移植 translator.py）+ benchmark
│   ├── lt-models/             # model_manager 1:1 移植 + 双 hub 自写下载器 + 缓存探测
│   ├── lt-i18n/               # 内嵌 zh/en + t(key) + LANGUAGES 表
│   └── lt-proto/              # settings.json 结构、IPC 帧、事件类型（共享类型）
└── assets/                    # silero.onnx、icons、i18n yaml、changelog（include 进二进制）
```

### 4.2 线程/任务模型（对应原版逐线程）

| 原版 | Rust 等价 | 职责 |
|---|---|---|
| Qt 主线程事件循环 | **winit 事件循环线程**（唯一 UI 线程） | 所有窗口绘制、定时器（帧内 Instant 检查）、托盘事件 dispatch |
| `_capture_loop` 线程 | `std::thread`（capture） | 从 AudioCapture 队列取 chunk → RMS/monitor 事件 → VAD.process_chunk → 段/interim 请求入 asr 队列；超时喂静音逻辑同款 |
| WASAPI 读线程（AudioCapture 内部） | `std::thread` ×2 | loopback 事件驱动读 + mic 读，混音、重采样、(audio, mic_rms) 入有界队列（满则丢旧） |
| `_asr_loop` 线程 | `std::thread`（asr supervisor） | 取 asr 队列（vad_flush/interim），先应用 pending language/padding → 调 worker → 结果事件发 UI；空转时做 RSS 回收检查 |
| ASR worker 子进程 | `std::process::Child`（spawn 自身 exe `--asr-worker`） | 持有模型；崩溃隔离 + RSS 可控（见 4.3） |
| `_tl_executor` 线程池(8+) | `tokio` 任务（semaphore 8+） | `_translate_async` / extra langs 并行；流式 partial 经 channel 发 UI（等价 update_streaming_signal） |
| 下载线程 | `tokio` 任务 | 向导/下载对话框，进度走日志广播 |
| MonitorBar sys QTimer(1s) | UI 帧内 1s 节流 + 后台 sysinfo 采样线程 | CPU/RAM（GPU 段恒 N/A，与原版无 N 卡表现一致） |
| 内存快照 QTimer(30s) | 后台线程 30s | MEM[tick] 日志 + 阈值警告 |

**事件总线**（替代 Qt 信号，全部无锁 MPSC + 帧内 drain）：

```rust
enum UiEvent {                       // 各工作线程 → UI
    AddMessage { id, ts, original, lang, asr_ms },
    UpdateTranslation { id, text, tl_ms },
    UpdateStreaming { id, partial },
    UpdateMonitor { rms, vad_conf, mic_rms: Option<f32> },
    UpdateStats { asr_n, tl_n, pt, ct, cost },
    AsrDevice { label }, AsrUnavailable,
    LogLine { level, target, msg },          // tracing broadcast → LogWin/下载框
    DownloadProgress(String),
    ModelLoadDone { result }, SettingsApplied(Settings), ...
}
enum Cmd {                           // UI → 管道
    Start, Pause, Resume, Stop,
    SwitchEngine { engine, model, hub, lang, pad },
    SetAsrLanguage(String), SetPadding { engine, secs },
    SetAudioDevice(Option<String>|Disabled), SetMicDevice(Option<String>),
    SwitchTranslator(ModelConfig), SetTargetLang(String), SetTimeout(u64),
    IncrementalAsr { enabled, interval }, ...
}
```

### 4.3 ASR worker 子进程协议（复刻 asr_client/asr_worker）

- spawn：`current_exe().arg("--asr-worker")`（`std::os::windows::process::CreationFlags` 隐藏控制台），stdin/stdout 二进制管道；**子进程挂入 Job Object（JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE，r3 补）**——主进程任何原因退出（含崩溃）时 OS 自动回收 worker，避免孤儿进程占住模型内存（原版 multiprocessing daemon 在父进程异常退出时同样可能残留，此处略优于原版）。
- 帧格式（替代 pickle，简洁二进制）：
  ```
  request:  [u32 len][json header {"id","type","word_ts":bool}][f32 LE audio payload]
  response: [u32 len][json {"id","ok","type"} + payload 或 {"error":{msg,"recoverable":bool}}]
  ready:    {"type":"ready","engine","display_name"}            // 启动后第一帧（无 device 字段，D-1）
  ```
- 超时语义 1:1：ready 180s；transcribe 60s（0.2s 步进轮询 + deadline）；set_language/padding min(10, 60)s；shutdown 5s → kill。
- 错误分类 1:1：加载失败 recoverable=false；命令级 true；连续 3 次 fatal；重启上限 3；generation 号竞态防护；RSS 基线 +2048MB 回收（`sysinfo` 取 child pid 内存）。（r4：无 remote 特判分支，所有引擎一律子进程。）
- 引擎 trait：

```rust
trait AsrEngine: Send {
    fn transcribe(&mut self, audio: &[f32]) -> Result<Option<AsrResult>, EngineError>;
    fn set_language(&mut self, lang: Option<&str>) -> Result<(), EngineError>;
    fn set_input_padding(&mut self, secs: f32) -> Result<(), EngineError> { Err(Unsupported) }
    fn supports_word_timestamps(&self) -> bool { false }
    fn unload(&mut self);
}
struct AsrResult { text: String, language: String, language_name: String, words: Option<Vec<WordTs>> }
```

### 4.4 Translator（translator.py 逐函数移植，r5 起基于 async-openai）

**库层（async-openai 承担，我们不维护协议）**：HTTP/SSE 解析、`[DONE]` 终止、流式 chunk 类型、usage 字段提取、认证头、ApiError（含状态码分类）、multipart 等。
**我们只保留 translator.py 的业务逻辑层**（全部有明确 Python 源，对照翻译）：

- `resolve_thinking_style` / `thinking_disable_body`（六种 thinking body）；
- `_build_request_kwargs` 的等价物：定义 **byot 自定义请求结构体**——复用库的 `CreateChatCompletionRequest` 字段 + `#[serde(flatten)] extra: BTreeMap<String, Value>` 承载 extra_body（thinking 禁用参数、用户 extra_body 合并后放这里），overrides（temperature/top_p/max_tokens/frequency_penalty/presence_penalty/seed）只发存在的键，`response_format` 用库的 `ResponseFormat::JsonSchema`（strict、`{"t": string}` schema）；
- `_build_messages`（no_system_role 合并 system 进 user、context_turns 多轮历史）与 `_build_system_prompt`（模板 format 异常回退 DEFAULT_PROMPT；json_response 追加 JSON 指令）；
- 客户端构造（等价 `make_openai_client`）：`OpenAIConfig::new().with_api_base().with_api_key()` + **`Client::build(自定义 reqwest, config)`**——reqwest client 按代理模式预构造（none→`no_proxy()`、system→默认、url→`Proxy::all`），超时 `Timeout(t, connect=5s)` 等价；
- `translate_iter`：`chat().create_stream(byot 请求)` 返回 `ChatCompletionResponseStream`，循环聚合 delta，**自维护 deadline 总超时**（原版语义）、`stream_options.include_usage` 先带后撤（失败时重发不带，与原版同）、json `{"t"}` 提取、thinking-burned-budget 警告、`_check_repetition` 重复检测；
- `with_target_language`（共享 client 的浅克隆）。
- 非流式 `translate`：同一 byot 结构 `stream=false` 单请求。benchmark 用同一套客户端构造（等价 `make_openai_client` 复用）。

### 4.5 VAD 状态机（vad_processor.py 逐行移植）

所有阈值/窗口/队列照抄：pre_buffer 3 chunk；progressive_tiers [(3.0,1.0),(6.0,0.5),(10.0,0.25)]；pause_history 50 条、P75×1.2、[0.3,2.0]s；平滑窗 5；搜索起点 n*3//10；dip_ratio<0.8；密度 <0.25 丢弃；energy 模式 `min(1, rms/(thr*2))`；disabled 恒 1.0；`trim_front` 置 `_was_trimmed`；`force_flush`。置信度来源换 ort+silero.onnx（512 样本窗，不足零填充）。

### 4.6 增量 ASR（interim）逻辑

`_do_interim_asr`（缓冲 ≥1.5s 才识别；word_ts 关闭；回声去除 `_strip_committed_overlap` 尾部 50 字符前缀匹配；分句 `yasbd → 自写规则`；>1 句才提交；比例裁剪 = committed/full × total + 0.3s 余量，上限保留 0.5s，下限 0.3s 且 ≤ total/2；短句 ≤8 字符缓冲 `_interim_pending`；裁剪后 `_interim_active=true`）、`_process_interim_final`、interim 请求去重（drain）、interim_interval/cooldown 1.0s——全部照抄。分句器自写：句末标点 `.。!！?？` 切分 + 缩写保护（Mr./Dr. 等）+ 逗号回退（`、` 25 字 / `,，;；` 60 字，before>15、after>3）。

### 4.7 音频采集抽象层与跨平台路线【r7 新增】

**背景**：系统音频 loopback 三平台机制完全不同（Windows=WASAPI loopback / macOS=CoreAudio process tap（cpal 0.17+，要求 14.6）/ Linux=PulseAudio-PipeWire `.monitor` source）。调研结论（2026-09）：cpal 0.17 的 loopback 仅覆盖 macOS，**Windows loopback 至今不是 cpal 一等公民**（RustAudio/cpal#251 开放中），因此 Windows 后端维持 `wasapi` crate（原生 loopback + 事件驱动 + 设备变更通知，严格优于 cpal）；跨平台靠**自建抽象层**而非"换一个跨平台库"。

**分层原则**：平台后端只做"原始采集"（原生采样率/声道 f32 块 + 设备枚举 + 重连），**所有 1:1 行为逻辑放平台无关的公共层**（32ms 节拍整形、线性插值重采样到 16k 单声道、loopback+mic 等长混音、mic_rms 计算、loopback 禁用时喂零、有界队列丢旧——即 audio_capture.py 的 `_read_loop`/`_resample_to_mono` 全部语义）。

```rust
// lt-pipeline::audio —— 后端契约（v1 即落地，Windows 实现填充）
pub struct DeviceEntry { pub name: String }        // 设置持久化用设备名（与原版一致）

pub struct BackendConfig {
    pub loopback: LoopbackTarget,                   // Default | Named(String) | Disabled(仅麦克风)
    pub mic: MicTarget,                             // Off | Default | Named(String)
}

pub enum NativeSource { Loopback, Mic }
pub struct NativeChunk { pub data: Vec<f32>, pub channels: u16, pub rate: u32, pub source: NativeSource }

pub trait AudioBackend: Send {
    fn id(&self) -> &'static str;                                  // "wasapi" | "coreaudio" | "pulse"
    fn list_output_devices(&mut self) -> anyhow::Result<Vec<DeviceEntry>>;
    fn list_input_devices(&mut self) -> anyhow::Result<Vec<DeviceEntry>>;
    /// 打开 loopback + mic 并开始推送；内部负责设备变更/读错误的重连（通知或轮询，行为对齐原版 2s 检查 + 自动重启）
    fn run(&mut self, cfg: BackendConfig, sink: mpsc::Sender<NativeChunk>) -> anyhow::Result<()>;
    fn stop(&mut self);
}

// 公共层（平台无关）：Mixer/Resampler 消费 NativeChunk → 产出 (16k mono f32 32ms, mic_rms)
// 经有界通道进入 capture 线程（原版 audio_queue 语义，满则丢最旧）
```

**平台后端矩阵（当前 + 未来）**：

| 平台 | loopback（系统声） | 麦克风 | crate | 状态 |
|---|---|---|---|---|
| Windows | WASAPI loopback（事件驱动） | WASAPI capture | `wasapi` | **v1 实现**（`#[cfg(windows)]`） |
| macOS | CoreAudio process tap（macOS 14.6+，cpal 0.17 聚合设备方式）；旧系统引导安装 BlackHole 虚拟设备后按 Named 设备采集 | CoreAudio | `cpal` 0.17+ | 未来（`#[cfg(target_os = "macos")]`） |
| Linux | PulseAudio `.monitor` source（`libpulse-binding`；经 pipewire-pulse 兼容层同时覆盖 PulseAudio 与 PipeWire，运行时只需 libpulse.so）或 `pipewire-rs` 原生 | pulse source / ALSA | `libpulse-binding`（首选，最省事）/ `pipewire` | 未来（`#[cfg(target_os = "linux")]`） |

**现在就做、以后零返工的三件事**：
1. `AudioBackend` trait + `NativeChunk` 通道契约进入 lt-pipeline，Windows 实现为唯一后端，公共层（重采样/混音/喂零/队列）独立成可单测的纯函数模块；
2. 设置/UI 只面向 trait：设备下拉来自 `list_*_devices()`，`audio_device`/`mic_device` 键语义（null=默认、名称、`__disabled__`）平台无关；
3. 重连策略抽象为后端职责（原版语义：默认设备 2s 轮询变更 + 读错误退避重连），接口上以"后端自行恢复、公共层只见连续 chunk 流"为契约——未来平台后端各自用通知（macOS kAudioDevicePropertyDeviceIsAliveSomewhatChanged / Linux pipe context subscribe）或轮询实现。

**注意**：非 Windows 平台当前不在 v1 范围（托盘/穿透/字体等亦需平台适配，另见 §8 的 winres 图标等）；本节只保证音频这层换平台时不用动管道与 UI。

---

## 5. 配置与数据目录设计（按用户要求调整）

```
~/.config/livetranslate/               # dirs::home_dir() / ".config" / "livetranslate"（Windows 上即 C:\Users\<u>\.config\livetranslate，遵用户指定）
├── settings.json                      # user_settings.json 完全等价（键、默认值、原子写、条件序列化全同原版）
├── config.yaml                        # 可选：高级用户覆盖默认值（等价原版 config.yaml；不存在时用内嵌默认）
├── models/                            # 默认模型缓存根（可在 settings.json "models_dir" 键改路径 —— 用户新需求）
│   ├── huggingface/hub/models--org--name/...    # 原版兼容布局（自写下载器落盘）
│   ├── modelscope/models--org--name/snapshots/master/...   # 自写 MS 下载器布局
│   └── (whisper 本地模型用户自放)
├── transcripts/livetrans_{ts}_{original|translation|all}.txt
└── logs/livetrans_{ts}.log
```

- `models_dir` 键：默认 `~/.config/livetranslate/models`；首启向导中可修改（原向导已有"缓存路径"概念位）；运行时修改 → 提示需重启生效（或触发引擎重载）。原版 `apply_cache_env()` 三环境变量的作用消失（无 torch/hub SDK），保留一个 `MODELS_DIR` 全局解析函数。
- settings.json 键集与附录 A 完全一致，便于从原版迁移（可选提供从 `LiveTranslate/user_settings.json` 的导入按钮——一期不做，键兼容已足够手工复制）。
- UI 语言 / hub / download_proxy / overlay_x/y/w/h / subtitle_mode.window_x/y 等全部沿用。

---

## 6. UI 1:1 还原方案（逐窗口要点）

### 6.1 主悬浮窗
- 两行 header（62px / compact 24px）：`☰ LiveTranslate` 拖动区 + 7 个按钮（隐藏/字幕/暂停⇄运行(黄⇄蓝)/清空/模式/设置/退出(红)）+ 4 复选框 + 模型/来源/目标 3 下拉（来源含 auto，目标无）。
- 消息行：`[HH:MM:SS]`（timestamp 色）`[lang]`(#6cf) 原文 ASR xx ms(#8b8) / 译文 `> ...` + `TL xx ms`(#db8)；流式 50ms 节流；"翻译中..."斜体灰占位；同语言 `(相同语言)`。
- MonitorBar：MIC(仅混音时)/RMS/VAD 三条 + `device | CPU x% RAM xxMB GPU N/A | ASR n TL n Tok k (p↑c↓) ¥0.0000`（GPU 恒 N/A，见 D-6）。
- 14 样式预设全表移植（default/transparent/compact/light/dracula/nord/monokai/solarized/gruvbox/tokyo_night/catppuccin/one_dark/everforest/kanagawa + custom），15 字段：bg_color/bg_opacity(0-255)/header_color/header_opacity/border_radius/orig+trans font_family/font_size/color/timestamp_color/window_opacity(%)。
- compact 动画 200ms OutCubic（<10px 跳过）；位置 500ms debounce 持久化；消息上限 50；导出 txt 三模式。

### 6.2 OBS 字幕窗
- `_SubtitleTextWidget`：epaint 描边文字（8 向偏移描边 + 填充）、贪心二分换行（标点集合 ` ,，。、!！?？;；:：.`）、desired_height = lineSpacing×n + outline×2 + 4、pixmap 缓存 → egui 的 texture 缓存同思路。
- 进出场动画 6 种 ×150/300ms、高度动画 150ms OutCubic 保持垂直中心、自动隐藏超时 + 反向恢复、1500ms 最小显示、多句 ` | `、行级配置 15 字段。
- 背景三态：图片拉伸 / rgba+圆角 / 全透明；中键拖动；click_through 500ms 重断言。

### 6.3 控制面板 7 Tab
全部控件/键/默认值/联动已在我方分析中逐项登记（见附录 A 摘要 + 源文件）。重点联动：
- 引擎切换 → Whisper 下载组/padding 可见性 + 窗口高度自适应（引擎下拉 2 项：Whisper / FunASR，索引与原版前两项一致）；`funasr_model` 下拉保留 3 项 1:1 结构（sensevoice-small 默认 / funasr-nano-2512 标实验性 / funasr-mlt-nano-2512 置灰，见 D-14）；Whisper 档位下拉 6 项（5 原版档 + turbo，D-15），large-v3 标注 CPU 慢；
- **设备下拉（cuda/cpu）整行移除（D-1）**，compute_type 相关一并消失；加载原版 user_settings.json 时忽略 `asr_device` 键；
- 模型编辑对话框的**条件序列化**（默认值不写盘）必须复刻；
- 300ms debounce + 滑条 isSliderDown 语义 + prompt 600ms + 字幕 tab 200ms；
- Cache tab 惰性扫描（切入时）、删除全部并退出；
- Benchmark：6 语×5 句、并行每模型一线程、TTFT 优先排名、样本标准差(n-1)。

### 6.4 首启向导/下载/加载对话框
单页 + 15s 倒计时自动开始；下载日志流（tracing 广播订阅 INFO+）；无取消、失败重试；成功写 13 键默认设置块（vad_threshold 0.3 等）。`_ModelLoadDialog` 无关闭按钮模态。

### 6.5 托盘
muda 菜单树（含 4 个动态子菜单 + 单选组 + checkbox 同步），图标运行时绘制（蓝底圆角 "LT"）；气泡消息（隐藏提示/字幕窗拖动提示/内存警告）。

---

## 7. 行为兼容细节清单（算法级，重写时逐条核对）

1. 音频 chunk 32ms=512 样本；队列满丢最旧；loopback 禁用时喂零。
2. 重采样：多声道 → 通道平均；线性插值（floor/ceil/frac 逐样本）——不引 FFT 重采样库。
3. 麦克风混音：从 mic 缓冲取等长段拼接（不足补零），mic_rms 单独算。
4. pad 桶：`quantum = round(16000×pad)`，尾部补零到整数倍，≤0 禁用。
5. ASR 结果过滤：空/纯标点丢；≥2s 且 ≤3 字符丢；语言不匹配丢（`asr_language != "auto"` 时）。
6. 同语言不翻译：译文显示空 + `(相同语言)`；字幕窗仍翻 extra langs。
7. `_check_repetition`：len≥40，plen∈[8, len/2]，`text[plen:2plen]==text[:plen]`。
8. thinking styles：auto 启发式（model 含 deepseek/glm 或 endpoint 含 deepseek/volces/api.z.ai/bigmodel → deepseek 风格；endpoint 含 api.openai.com/api.x.ai/api.anthropic.com → off；否则 qwen）；六种 body 形态见 translator.py L167-177。
9. 费用：`(pt×in_price + ct×out_price)/1e6`；语言 zh → ¥ 否则 $。
10. 转写文件：追加、UTF-8、行缓冲（每行 flush）；`all` 配对失败时译文独立成块；`finalize_no_translation` 只写原文。
11. 导出文件名 `livetrans_{%Y%m%d_%H%M%S}_{kind}.txt`。
12. 日志：文件 DEBUG、控制台 INFO、LiveTranslate 命名空间分级、未捕获异常钩子。
13. 设置原子写 + `.tmp`；float 一律 round(2)。
14. Benchmark：流式优先（TTFT=首 chunk），失败回退非流式；`avg ± stdev(n-1)`；排名按 avg TTFT 升序；`__DONE__` 哨兵。
15. UI 语言切换需重启（弹双语提示）；i18n 缺 key 回退 key 本身。
16. 单实例（新增，防双开模型锁冲突）；Ctrl+C 退出（tokio signal → 优雅 stop）。

---

## 8. 构建与分发【r2 修订：纯 CPU 单产物】

- 工具链：`x86_64-pc-windows-msvc`，`cargo build --release`。release profile：`lto = "thin"`, `codegen-units = 1`, `strip = true`, `opt-level = 2`（体积/速度平衡）。目标基线：Windows 10 x64（无 VC 运行库依赖：静态 CRT `/MT` 或 rust 自带）。
- 体积预算：egui+glow ~5MB + ort(onnxruntime CPU 静态) ~12MB + sherpa-onnx(静态, CPU) ~12MB + reqwest/rustls ~4MB + tokio ~2MB + 资源(含 silero.onnx) ~3MB ⇒ **35–65MB 单 exe**（r8：+whisper.cpp ~3-4MB）。
- ** sherpa-onnx 与 ort 都捆绑 ONNX Runtime（CPU）** —— 实现期（M0）第一件事验证二者静态共存；CPU-only 下解法明确：方案 A（首选）：ort 用 `load-dynamic` 指向 sherpa 捆带的同一 onnxruntime 动态库，或让 ort 关闭 download-binaries 复用 sherpa 的；方案 B：VAD 独立小进程；方案 C（保底）：VAD 用 sherpa 的裸 SileroVadModel（需验证其 C API 是否暴露逐 chunk 置信度，若暴露则连 ort 都可去掉，栈更简）。
- **无 GPU 产物矩阵**：仅 `livetranslate.exe` 一个产物；用户机器无需 CUDA、无需 N 卡，任意 x64 Windows 可跑。
- 图标/版本信息：`winres` 嵌入 .ico + 版本资源；代码签名自便。
- CI：GitHub Actions windows-latest 单作业，产物 zip + sha256。
- 自动更新：一期不做（原版 update.bat 语义不适用单 exe，可选后续加自更新）。

---

## 9. 风险登记册【r8 状态】

| # | 风险 | 等级 | 缓解 |
|---|---|---|---|
| ~~R-1~~ | ~~sherpa-onnx CUDA 支持未文档化~~ | — | **已随纯 CPU 决策消除（r2）** |
| ~~R-2~~ | ~~anime-whisper 无 ONNX 官方转换~~ | — | **已随功能裁剪消除（r6，D-13）** |
| R-3 | FunASR Nano sherpa-onnx 转换质量告警（#3061/#3066） | 中（用户已知情接受） | 实验性标注（D-14）；实现期甄别"真 nano"转换包并实测；出问题不阻塞主路径（默认引擎 SenseVoice） |
| R-4 | ort 与 sherpa-onnx 的 ONNX Runtime 符号共存 | 中 | CPU 下解法明确（§8 方案 A/B/C），M0 首日验证；最理想结局是方案 C 让 ort 整个消失 |
| R-5 | ModelScope files API 响应结构未实测（分页/字段） | 低 | 实现期先 curl 实测；接口模式已被 transformers.js 等广泛使用 |
| R-6 | egui 多窗口 + 托盘 + 常驻重绘的性能/坑（透明窗口 DWM、拖动卡顿） | 中 | egui-multiwin 先例 + 事件驱动重绘（repaint 仅在有待处理事件/动画时）；每帧预算 <2ms 级 UI |
| R-7 | 中文字体渲染（微软雅黑加载、字形光栅） | 低 | egui FontDefinitions 加载 `C:\Windows\Fonts\msyh.ttc`；字体下拉用 GDI EnumFontFamiliesExW |
| ~~R-8~~ | ~~sherpa Whisper 与 faster-whisper 识别差异~~ | — | **已解决（r8，D-16）**：whisper 引擎改用 whisper.cpp，beam=5 与解码策略对齐原版；剩余差异仅为同权重不同推理后端的浮点级噪声 |
| R-9 | 单 exe 内嵌全部资源后 i18n/changelog 无法热改 | 低 | 符合"单文件分发"目标；提供 `--extract-resources` 调试开关（可选） |
| R-11 | **纯 CPU 下弱机性能**：whisper medium/large int8 在低主频 CPU 可能跟不上实时 | 中 | 默认引擎 SenseVoice int8（CPU 单段 <1s，覆盖绝大多数场景）；whisper 档位体积/速度提示引导选 tiny/small/turbo（turbo 档是弱机要质量的最优解，D-15）（r4：远程 ASR 兜底已随功能裁剪移除） |
| R-12 | sherpa-onnx sys crate 构建脚本从 GitHub releases 下载预编译库（国内网络/CI 不稳） | 低 | 缓存vendor 目录 + `SHERPA_ONNX_LIB_DIR` 手动指定；CI 加缓存 |
| R-13 | async-openai `byot` 的**流式**入口（create_stream 的 byot 变体）具体方法名/签名待实现期确认（README 只明示了 `create_byot`） | 低 | M3 首日验证；若流式无 byot 变体，回退方案：流式路径用 `serde_json::Value` 直接 POST `/chat/completions`（约 40 行，仅此一条窄路径手写）；非流式路径无影响 |

---

## 10. 实施路线图（建议里程碑）

| 阶段 | 内容 | 验收 |
|---|---|---|
| M0 骨架 | workspace、winit 多窗口 + 托盘 + i18n + settings.json 读写 + 日志/单实例；**验证 ort↔sherpa 共存（R-4）与 sherpa sys 构建** | 4 窗口出现可切换，托盘全菜单可点，两个推理库同时链接成功 |
| M1 音频+VAD | **AudioBackend 抽象层 + wasapi 后端**（loopback+mic+重连；公共层重采样/混音纯函数化并单测）→ silero-onnx 置信度 → VAD 状态机 1:1 | 与原版同输入下切段时间戳对齐（±1 chunk）；抽象层单测通过 |
| M2 ASR | worker 子进程 + SenseVoice(sherpa) + 模型下载器(HF/MS) + 向导/下载/加载对话框 | 首启向导 → 实时出原文 |
| M3 翻译 | Translator 1:1（流式/JSON/thinking/overrides/proxy）+ benchmark tab | DeepSeek/本地 LM Studio 流式逐字显示 |
| M4 UI 完备 | 悬浮窗全部交互/样式预设/monitor/导出；字幕窗动画描边；控制面板 7 tab 全控件；日志窗 | 与原版截图并排走查 |
| M5 引擎扩展 | whisper(whisper.cpp 6 档含 turbo) + FunASR Nano（实验性） + 本地模型扫描 | 引擎切换/失败回滚/自动重启全链路；whisper 双栈并存的构建验证 |
| M6 打磨 | interim ASR、内存回收、托盘同步细节、性能（启动<2s、空闲 CPU<1%、长跑 RSS 稳定）、CI 单产物 | 长跑 8h 稳定 |
| M7 可选 | 自更新机制；若上游产出成熟转换可评估加回 anime-whisper（D-13）；MLT 待上游转换出现后解除置灰（D-14） | — |
| M8 未来 | 跨平台：macOS（cpal 0.17+ loopback 后端 + 托盘/穿透/字体适配）、Linux（libpulse `.monitor` 后端 + Wayland 穿透适配）；音频层按 §4.7 直接换后端 | — |

---

## 附录 A：settings.json 键清单（与原版 user_settings.json 逐键一致 + 新增）

**VAD**：`vad_mode`(silero|energy|disabled), `vad_threshold`(0-1, 默认0.5/向导0.3), `energy_threshold`(0.02), `min_speech_duration`(1.0), `max_speech_duration`(8.0), `silence_mode`(auto|fixed), `silence_duration`(0.8)
**ASR**：`asr_engine`(funasr|whisper；~~anime-whisper、remote-whisper~~ **r4/r6 裁剪**，导入时回退 funasr), `funasr_model`(sensevoice-small|funasr-nano-2512|funasr-mlt-nano-2512，3 项 1:1；mlt 置灰；非法值导入回退 sensevoice-small), `asr_language`(auto+29语共30项，i18n.py:42 已核对), ~~`asr_device`~~（**r2 移除**；导入原版 settings 时忽略）, `whisper_model_size`(tiny|base|small|medium|large-v3|turbo|本地GGML路径), `sensevoice_pad_seconds`(0.5), `whisper_pad_seconds`(0.5), `hub`(ms|hf), `download_proxy`(none|system|url), ~~`remote_asr_url`~~（**r4 移除**；导入时忽略）, `incremental_asr`(false), `interim_interval`(2.0)
**音频**：`audio_device`(null|名称|__disabled__), `mic_device`(null|__default__|名称)
**翻译**：`models[]`(name/api_base/api_key/model/proxy + 条件键 no_system_role/thinking_style/streaming/json_response/context_turns/input_price/output_price/overrides{temperature,top_p,max_tokens,frequency_penalty,presence_penalty,seed}/extra_body), `active_model`(0), `target_language`(zh), `system_prompt`, `timeout`(10), `ui_lang`(en|zh)
**UI**：`style{}`(15键), `subtitle_mode{}`(窗口12键+lines[]每行15键+window_x/y/sentences/enabled), `overlay_x/y/w/h`, `auto_save_transcript`(true)
**新增（Rust 版）**：`models_dir`（模型缓存根路径）

## 附录 B：源码地图

| 原文件 | → Rust 模块 | 备注 |
|---|---|---|
| main.py | lt-app（装配/托盘/启动流）+ lt-pipeline（管道循环） | 2344 行拆两处 |
| audio_capture.py | lt-pipeline::audio | wasapi 替代 pyaudiowpatch |
| vad_processor.py | lt-pipeline::vad | ort+silero.onnx |
| asr_funasr/asr_sensevoice/asr_funasr_nano | lt-asr::engines::{sensevoice, nano} | sherpa-onnx（r8：nano 恢复实验性） |
| asr_engine(whisper) | lt-asr::engines::whisper | whisper.cpp（whisper-rs，r8 D-16） |
| asr_anime_whisper | 不移植（D-13，r8 确认） | — |
| asr_client/asr_worker | lt-asr::worker | 子进程+二进制帧 |
| asr_server/asr_remote | 不移植（r4 裁剪） | — |
| funasr_nano/（本地 PyTorch 实现） | 不移植（nano 引擎经 sherpa-onnx ONNX 运行，D-14） | — |
| translator.py | lt-translate | 1:1 |
| benchmark.py | lt-translate::bench | |
| model_manager.py | lt-models | 双 hub 自写下载器（r3） |
| subtitle_overlay.py | lt-ui::overlay | |
| subtitle_window.py | lt-ui::subtitle | |
| subtitle_settings.py | lt-ui::panel::subtitle_tab | |
| control_panel.py | lt-ui::panel | |
| dialogs.py | lt-ui::dialogs | |
| log_window.py | lt-ui::logwin | tracing broadcast |
| i18n.py + i18n/* | lt-i18n（include_str!） | |
| transcript_writer.py | lt-pipeline::transcript | |
| config.yaml | 内嵌默认 + ~/.config 覆盖 | |
| tests/*（segmentation/thinking/translator 思维用例等） | crates 内单元测试对照移植 | 保行为回归 |

## 附录 C：外部链接（调研来源）

- sherpa-onnx Rust（官方）: https://docs.rs/sherpa-onnx （v1.13.7，静态链接默认）
- sherpa-onnx FunASR Nano 文档: https://k2-fsa.github.io/sherpa/onnx/funasr-nano/pretrained.html
- sherpa whisper 仅 greedy（官方确认无 beam 计划）: https://github.com/k2-fsa/sherpa-onnx/discussions/671 、CER 告警 https://github.com/k2-fsa/sherpa-onnx/issues/2900 、beam 范围说明 https://github.com/k2-fsa/sherpa-onnx/issues/465
- whisper.cpp（r8 whisper 引擎）: https://github.com/ggml-org/whisper.cpp 、whisper-rs https://github.com/tazz4843/whisper-rs 、GGML 模型 https://huggingface.co/ggml-org 、turbo https://huggingface.co/ggml-org/whisper-large-v3-turbo
- sherpa-rs（已归档，指向官方 crate）: https://github.com/thewh1teagle/sherpa-rs
- wasapi: https://github.com/HEnquist/wasapi-rs
- cpal 0.17 CHANGELOG（macOS loopback/CoreAudio process tap 14.6+、稳定设备 ID、自定义 Host）: https://github.com/RustAudio/cpal/blob/master/CHANGELOG.md 、Windows loopback 现状 https://github.com/RustAudio/cpal/issues/251
- Linux 音频：libpulse-binding https://docs.rs/libpulse-binding （.monitor source，经 pipewire-pulse 兼容层双覆盖）、pipewire-rs 官方绑定 https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/
- sherpa-onnx Whisper 预训练模型（ONNX 下载/导出）: https://k2-fsa.github.io/sherpa/onnx/pretrained_models/whisper/index.html 、导出脚本 https://github.com/k2-fsa/sherpa-onnx/blob/master/scripts/whisper/export-onnx.py
- ort: https://github.com/pykeio/ort （2.0.0-rc.13）
- async-openai: https://github.com/64bit/async-openai （0.41.x；byot 特性、Client::build 注入自定义 reqwest、ChatCompletionStreamOptions、ResponseFormat::JsonSchema 均已核实 docs.rs/README）
- ~~hf-hub~~（r3 移除，复核发现 1.0.0 布局未确认兼容；备用参考 https://github.com/huggingface/hf-hub ）
- egui ViewportBuilder（transparent/passthrough/always_on_top/taskbar）: https://docs.rs/egui/latest/egui/viewport/struct.ViewportBuilder.html
- egui 多窗口: https://github.com/emilk/egui/issues/1044 、https://crates.io/crates/egui-multiwin
- egui+托盘示例: https://github.com/emilk/egui/discussions/737 、tray-icon https://lib.rs/crates/tray-icon
- 点击穿透（WS_EX_TRANSPARENT）: https://stackoverflow.com/questions/75630785
- ModelScope 直链 API: `https://modelscope.cn/api/v1/models/{id}/repo?Revision=master&FilePath={file}`（https://discuss.huggingface.co/t/169364 佐证）
- FunASR Nano ONNX 转换: https://github.com/Wasser1462/FunASR-nano-onnx
- 质量告警: https://github.com/k2-fsa/sherpa-onnx/issues/3061 、/issues/3066
