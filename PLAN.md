# LiveTranslate-rs 实施计划（施工图）

> 版本：1.0（2026-09-05，基于 RESEARCH.md r8 定稿编制）
> 定位：**写代码时的唯一依据**。选型理由与调研证据见 `RESEARCH.md`；本文档只写"怎么干"。
> 硬约束（全程有效）：纯 CPU、单 exe、`~/.config/livetranslate`、GUI/逻辑与原版 1:1（刻意偏差以 RESEARCH.md §1.5 D-1~D-16 为准，本文不重复论证）。
> 标记约定：**【V】**=已在线核实过的事实；**【M0】【M3】**等=对应里程碑首日必须实测确认的点；其余为设计决策。

---

## 1. 工程骨架

### 1.1 目录结构（文件级）

```
livetranslate-rs/
├── Cargo.toml                      # workspace 根（见 1.2）
├── .cargo/config.toml              # rustflags：静态 CRT（windows）
├── build.rs                        # winres：图标+版本资源（仅 lt-app 需要，放 lt-app/ 下）
├── assets/
│   ├── silero_vad.onnx             # v5，~2MB（内嵌 include_bytes!）
│   ├── icon.png / icon.ico
│   ├── i18n/zh.yaml, en.yaml       # 从原版 i18n/ 原样拷入（~328 键/文件）
│   └── CHANGELOG_zh.md, CHANGELOG_en.md
├── crates/
│   ├── lt-proto/                   # 零依赖共享类型（serde only）
│   │   └── src/
│   │       ├── settings.rs         # Settings/ModelConfig/Style/SubtitleMode（附录 A schema）
│   │       ├── events.rs           # UiEvent / Cmd 枚举
│   │       ├── asr_result.rs       # AsrResult / WordTs / EngineError
│   │       └── lib.rs
│   ├── lt-i18n/
│   │   └── src/lib.rs              # include_str! 双语表 + t(key)/LANGUAGES/COMMON_LANG_CODES
│   ├── lt-models/
│   │   └── src/
│   │       ├── registry.rs         # 模型注册表（§3.4）
│   │       ├── cache.rs            # 缓存探测（移植 model_manager 探测逻辑）
│   │       ├── download/hf.rs      # /resolve 直链 + 树 API
│   │       ├── download/ms.rs      # ModelScope API
│   │       ├── download/mod.rs     # 公共：代理三模式 client、.incomplete 续传、退避
│   │       └── lib.rs
│   ├── lt-pipeline/
│   │   └── src/
│   │       ├── audio/mod.rs        # AudioBackend trait + 公共层（mixer/resampler）
│   │       ├── audio/wasapi_win.rs # #[cfg(windows)] 后端
│   │       ├── vad.rs              # SileroVad(ort) + VAD 状态机（§5.1）
│   │       ├── capture.rs          # capture 线程（_capture_loop 等价）
│   │       ├── interim.rs          # 增量 ASR（§5.3）+ 分句器 segment.rs
│   │       ├── transcript.rs       # TranscriptWriter
│   │       └── lib.rs
│   ├── lt-asr/
│   │   └── src/
│   │       ├── engine.rs           # AsrEngine trait
│   │       ├── engines/sensevoice.rs   # sherpa-onnx
│   │       ├── engines/nano.rs         # sherpa-onnx（实验性）
│   │       ├── engines/whisper.rs      # whisper-rs
│   │       ├── worker.rs           # 子进程入口逻辑（--asr-worker 时调用）
│   │       ├── client.rs           # AsrWorkerClient（IPC、超时、状态机、回收）
│   │       └── lib.rs
│   ├── lt-translate/
│   │   └── src/
│   │       ├── translator.rs       # Translator（§5.4）
│   │       ├── thinking.rs         # resolve_thinking_style/thinking_disable_body
│   │       ├── bench.rs            # run_benchmark
│   │       └── lib.rs
│   ├── lt-ui/
│   │   └── src/
│   │       ├── app.rs              # 多窗口宿主：winit EventLoop + 每窗口 egui State（§2.1）
│   │       ├── windows/overlay.rs / subtitle.rs / panel/ (7 tab) / logwin.rs
│   │       ├── dialogs/            # wizard/download/load/model_edit/line_edit/color
│   │       ├── tray.rs             # tray-icon + muda 菜单树
│   │       ├── style.rs            # 14 预设全表（§3.5）
│   │       └── lib.rs
│   └── lt-app/
│       ├── build.rs                # winres
│       └── src/main.rs             # 启动流程（§4 M0-M2 逐步装配）、单实例、托盘、装配
└── LiveTranslate/                  # 原版参考（只读，不参与构建）
```

### 1.2 根 Cargo.toml（完整可用模板）

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"        # 注：不用 2024，sherpa-onnx-sys 等老代码兼容性优先；如遇依赖要求再升
license = "MIT"

[workspace.dependencies]
egui = "0.32"
egui-winit = "0.32"
egui-glow = "0.32"
winit = "0.30"
glow = "0.16"
windows = { version = "0.62", features = [
    "Win32_UI_WindowsAndMessaging", "Win32_Graphics_Gdi", "Win32_Foundation",
    "Win32_System_Threading",       # Job Object
    "Win32_System_SystemInformation",
    "Win32_Globalization",          # 字体枚举备选
    "Win32_Storage_FileSystem",     # explorer 打开
] }
tray-icon = "0.24"
muda = "0.19"
wasapi = "0.24"
ort = { version = "2.0.0-rc.13", default-features = false, features = [
    "ndarray", "copy-dylibs",       # 注：M0 验证与 sherpa 的 ORT 共存后调整（R-4）
] }
sherpa-onnx = { version = "1.13", default-features = true }   # static 默认开
whisper-rs = "0.16"                 # 不开 cuda（纯 CPU 硬约束）
async-openai = { version = "0.41", features = ["byot"] }
reqwest = { version = "0.13", default-features = false, features = ["rustls-tls", "stream", "json", "gzip"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "fs", "process", "signal", "sync"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sysinfo = "0.39"
dirs = "6"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "time"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
anyhow = "1"
thiserror = "2"
parking_lot = "0.12"
crossbeam-channel = "0.5"           # 供 select 场景；普通队列用 std mpsc 即可

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
opt-level = 2
panic = "unwind"                    # worker 需要 catch_unwind 分类错误，不用 abort
```

`.cargo/config.toml`（静态 CRT，免 VC 运行库依赖）：
```toml
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]
```

### 1.3 构建与 CI

- 命令：`cargo build --release`；产物 `target/release/livetranslate.exe`（单文件）。
- sherpa-onnx-sys 构建期从 GitHub releases 拉预编译库 → 国内/CI 不稳时设 `SHERPA_ONNX_LIB_DIR` 指向本地缓存目录（R-12）；CI 加 `actions/cache`。
- CI（`.github/workflows/release.yml`）：windows-latest 单作业 → build → 附 sha256 的 zip。winres 嵌图标/版本。
- 启动参数：无参=主程序；`--asr-worker`=worker 子进程入口（对multiprocessing freeze_support 的等价物）；`--extract-resources`（调试用，可选）。

---

## 2. 库信息卡（关键 API / 陷阱 / 验证状态）

### 2.1 egui + egui-winit + egui-glow（GUI 核心）【V】

- **多窗口模式：不用 eframe，直接 winit 事件循环 + 每窗口一个 `egui_winit::State` + 一个共享 `egui::Context`（或多 Context）+ glow 渲染**（egui-multiwin crate 的模式，examples 有现成参考）。eframe 是单窗口假设，viewport API 对"各窗口独立 window flags/DPI/透明"控制力不足。
- **窗口能力**（`winit::window::WindowAttributes` / egui `ViewportBuilder`，均已 docs.rs 核实）：
  - 透明：`with_transparent(true)`（配合 `with_has_shadow(false)` 防 macOS 鬼影，Windows 无碍）
  - 无边框：`with_decorations(false)`
  - 置顶：`with_window_level(WindowLevel::AlwaysAbove)`；运行时切换 `window.set_window_level()`
  - 跳过任务栏：Windows 专有 `WindowExtWindows::with_skip_taskbar(false)` / `set_taskbar_visible()`
  - 不抢焦点显示：`with_active(false)` + `with_visible()`
- **陷阱**：
  - 透明窗口必须保证每帧都画满（未覆盖区域要 `clear(transparent)`），否则残影；
  - egui 是即时模式：所有"控件状态"（combo 展开、滑条拖动）自己存 AppState，窗口重创建后不丢业务状态；
  - 重绘驱动：常驻动画期间 `ctx.request_repaint_after(Duration::from_millis(16))`；空闲时靠事件唤醒（channel → EventLoopProxy `user_event` → 重绘），避免 100% CPU。
- **字体**：启动时 `FontDefinitions` 注入 `C:\Windows\Fonts\msyh.ttc`（微软雅黑）作为 CJK fallback；系统字体列表用 GDI `EnumFontFamiliesExW`（windows crate）供字体下拉。
- **描边文字**（字幕窗）：两遍绘制法——8 方向偏移（dx,dy ∈ {-w,0,w}）以描边色画一遍文字，再以填充色画中心一遍；缓存到 `egui::TextureHandle`，文字/字体/颜色不变则不重绘（对应原版 QPixmap 缓存）。
- **动画**：全部手写时间插值 `t = e.elapsed()/dur`，`emath::easing::out_cubic`/`in_cubic`；紧凑模式 200ms、字幕高度 150ms、进出场每行配置（默认 300ms）。

### 2.2 winit + windows（窗口/Win32）【V】

- winit 0.30 事件循环：`EventLoop::with_user_event()`；工作线程通过 `EventLoopProxy::send_event(UiMsg)` 唤醒 UI（等价 Qt queued signal）。
- **点击穿透**（每周 50ms，对齐原版 `_ct_timer`）：
  ```rust
  // windows crate
  SetWindowLongW(hwnd, GWL_EXSTYLE,
      GetWindowLongW(hwnd, GWL_EXSTYLE) | WS_EX_TRANSPARENT | WS_EX_LAYERED);  // 正文穿透
  // header 区（光标在 handle 矩形内）则去掉 WS_EX_TRANSPARENT 仅留 LAYERED
  ```
  光标：`GetCursorPos` + `MapWindowPoints`/窗口 `outer_rect()`。**WS_EX_LAYERED 必须与 TRANSPARENT 成对**（经验 E-04）。
- 其他 Win32 用途：命名互斥量（单实例）、Job Object（§2.8）、`ShellExecuteW "open"` 打开文件夹、`EnumFontFamiliesExW`。
- 陷阱：winit 的 `outer_position/inner_position` 语义差异——持久化窗口位置统一用 `outer_position()`（对应 Qt frameGeometry）。

### 2.3 tray-icon + muda（托盘）【V】

- Tauri 系标准组合。**Windows 上事件不进 winit 循环**，必须：
  ```rust
  tray_icon::TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| { proxy.send_event(Msg::Tray(e)); }));
  muda::MenuEvent::set_event_handler(Some(move |e: MenuEvent| { proxy.send_event(Msg::Menu(e)); }));
  ```
- 菜单树用 muda `Menu/Submenu/MenuItem/CheckMenuItem/PredefinedMenuItem`（separator/radio 语义用 CheckMenuItem + 同组互斥手控，或 IconMenuItem）；动态重建（模型子菜单）= 原地 set_menu。
- 图标运行时生成：egui 画 64×64 蓝底圆角 "LT" → `image` crate 转 RGBA → `tray_icon::Icon::from_rgba`（复刻原版 create_app_icon）。
- 气泡：`tray.show_message(title, body, MessageIcon, duration)`。

### 2.4 wasapi（音频采集，Windows 后端）【V】

- README 核实能力：loopback、事件驱动/poll、设备枚举、默认设备、设备变更通知、每应用捕获、AEC。
- **线程模型**：每条音频线程先 `wasapi::initialize_mta()?`（COM 初始化，经验 E-09：COM init 必须在用它的线程上）。
- **实现参照 crate 自带 examples**：`devices.rs`（枚举）、loopback 示例（采集系统声）、`playsine_events`（事件驱动模式）。以 docs.rs 实际签名为准，不要按记忆写【M1 首日跑通 examples】。
- 事件驱动读循环骨架：`client.set_event_handle()` → 循环 `event.wait(duration)` → `capture_client.get_next_packet()` 取满 `chunk_duration` 的帧 → 转 f32（mix format 通常是 f32 shared）。
- 设备变更重连：订阅 crate 的设备通知（或兜底 2s 轮询比对默认设备名，对齐原版行为）。
- **只做原始采集**：原生 rate/声道 f32 块推给公共层；重采样/混音/喂零全在 `lt-pipeline::audio::mod`（纯函数、可单测）。

### 2.5 ort + silero_vad.onnx（VAD）【V】

- ort 2.0.0-rc.13；**只开 CPU EP**（默认即 CPU）。
- 模型内嵌：`static MODEL: &[u8] = include_bytes!("../../assets/silero_vad.onnx");` → `Session::builder()?.commit_from_memory(MODEL)?`。
- **v5 ONNX I/O 契约**（经验 E-02）：输入 `input[1,512]f32` + `state[2,1,128]f32` + `sr`（int64=16000）；输出 `output[1,1]` + 新 state。**state 必须自持并逐 chunk 回喂**，初始全零。输入名以模型文件实际为准【M1 首日打印输入输出名核对】。
- 封装：
  ```rust
  struct SileroVad { session: Session, state: Array2<f32> /* 2x128 */ }
  impl SileroVad { fn conf(&mut self, chunk: &[f32; 512]) -> f32 { /* run + 更新 state */ } }
  ```
- 单线程推理，chunk 级 <1ms，不构成瓶颈。
- 版本对齐：资产入库时记录 silero_vad.onnx 的来源（HF `snakers4/silero-vad` 5.x）与 sha256 于 `assets/SOURCES.md`（保证与 Python 版置信度可比）。
- **R-4 注意**：若与 sherpa 的静态 ORT 链接冲突（M0 首日验证），按 §8 预案切 `load-dynamic` 复用同一动态库。

### 2.6 sherpa-onnx（SenseVoice + Nano）【V】

- 1.13.x，官方 Rust API；**默认 static feature**，构建脚本自动下载预编译库。
- SenseVoice 用法（docs.rs 已核实模式）：
  ```rust
  let mut cfg = OfflineRecognizerConfig::default();
  cfg.model_config.sense_voice = OfflineSenseVoiceModelConfig {
      model: Some(path.into()), language: Some("auto".into()), use_itn: true, ..Default::default() };
  cfg.model_config.tokens = Some(tokens_path.into());
  let rec = OfflineRecognizer::create(&cfg)?;
  let mut stream = rec.create_stream();
  stream.accept_waveform(16000, samples);
  rec.decode(&stream);
  let text = stream.result()?.text;
  ```
- 输出文本带 `<|zh|>` 等标签 → 后处理剥离（§5.2）。
- Nano：同 API，模型配置换成 funasr-nano 的文件组【M5 时按官方文档页核对字段名与文件清单】。
- 陷阱：sherpa 所有对象非 Sync——只在 worker 进程内单线程使用（我们本来就是单线程 worker）；首次 `create` 加载大模型 ~秒级，放 worker 启动（180s ready 超时内）。

### 2.7 whisper-rs（Whisper 引擎）【V】

- 0.14+（维护于 codeberg）；**不开任何 GPU feature**（纯 CPU 约束）。
- 用法（已核实）：
  ```rust
  let ctx = WhisperContext::new_with_params(&model_path, WhisperContextParameters::default())?;
  let mut params = FullParams::new(SamplingStrategy::BeamSearch { beam_size: 5 }); // 对齐原版
  params.set_language(if let Some(l) = lang { Some(l) } else { None });           // None=auto
  params.set_task(Task::Transcribe);
  params.set_suppress_non_speech_tokens(true);  // 对齐原版体验，M5 实测定
  let mut state = ctx.create_state()?;
  state.full(params, &audio[..])?;              // audio: &[f32] 16k mono
  let mut out = String::new();
  for i in 0..state.full_n_segments()? { out.push_str(&state.full_get_segment_text(i)?); }
  ```
- 语言检测：auto 时 `state.full_default_speaker?`不对——用 `state.detect_language()?`（返回 (lang_id, prob)，token→ISO 码映射用库内表或自备 99 语言表——**以实际 API 为准**【M5 首日核对】；原版 whisper 的 language 输出是 ISO 码如 "en"）。
- word timestamps：token 级 API（`full_get_token_text/full_get_token_data`），interim 比例裁剪用不上但接口保留。
- 模型文件：单 GGML `.bin`；量化档直接是文件本身（`ggml-medium-q5_0.bin` 等），无需运行时选项。

### 2.8 子进程与 Job Object【V】

- spawn：`Command::new(current_exe()?).arg("--asr-worker")`，`.creation_flags(CREATE_NO_WINDOW)`，`Stdio::piped()` ×2。
- **Job Object（经验 E-05）**：父进程 `CreateJobObjectW` → `SetInformationJobObject(JobObjectExtendedLimitInformation, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)` → `AssignProcessToJobObject(child)`。父进程任何方式退出（含崩溃）OS 收割 worker。
- 帧协议（§3.2）；worker 侧 `std::io::stdin().lock()` 循环读帧。

### 2.9 async-openai（LLM 客户端）【V】

- 0.41.x；features：`byot`（+ rustls 走 reqwest 特性）。
- 客户端构造（等价 make_openai_client，代理三模式）：
  ```rust
  let http = match proxy {
      ProxyMode::None   => reqwest::Client::builder().no_proxy().timeout(t).connect_timeout(5s),
      ProxyMode::System => reqwest::Client::builder().timeout(t).connect_timeout(5s),
      ProxyMode::Url(u) => reqwest::Client::builder().proxy(Proxy::all(u)?).timeout(t),
  }.build()?;
  let cfg = OpenAIConfig::new().with_api_base(base).with_api_key(key);
  let client = async_openai::Client::build(http, cfg);   // 【V】Client::build(http_client, config)
  ```
- **byot 请求结构**（extra_body 唯一正确姿势）：
  ```rust
  #[derive(Serialize)]
  struct ChatReq { #[serde(flatten)] base: CreateChatCompletionRequest,
                   #[serde(flatten)] extra: BTreeMap<String, serde_json::Value> }
  ```
  非流式：`client.chat().create_byot(json!(req))`（README 明示）。**流式 byot 方法名待核**【M3 首日】；若无 → 兜底：手写 POST `/chat/completions`（stream:true）+ reqwest bytes_stream 手解析 SSE `data:` 行（~40 行，仅此路径）。
- `stream_options.include_usage`：字段 `ChatCompletionStreamOptions { include_usage: true }`【V 存在】；先带后撤逻辑保留（原版语义）。
- JSON schema：`ResponseFormat::JsonSchema { json_schema: ResponseFormatJsonSchema { name, strict, schema } }`【V 存在】。
- 错误映射：`openai::error::OpenAIError` → 我们的 TranslateError（连接/超时/认证/状态码分类，对齐原版 except 块）。

### 2.10 自写模型下载器【V】

- **HF**：文件树 `GET {endpoint}/api/models/{repo}/tree/main?recursive=true`；文件 `GET {endpoint}/{repo}/resolve/main/{path}`（302→CDN，需 `redirect(Policy::limited(10))`；Range 续传可用）。endpoint 默认 `https://huggingface.co`，可覆写 `https://hf-mirror.com`。
- **ModelScope**：文件树 `GET https://modelscope.cn/api/v1/models/{id}/repo/files?Revision=master`（**响应结构 M2 首日 curl 实测**，R-5）；文件 `GET /api/v1/models/{id}/repo?Revision=master&FilePath={path}`【V】。
- 落盘布局：`<models_dir>/huggingface/hub/models--{org}--{name}/snapshots/{rev}/...`、`<models_dir>/modelscope/models--{org}--{name}/snapshots/master/...`（与原版探测兼容）。
- 续传：下载到 `{file}.incomplete`（已存在则 `Range: bytes={len}-`），完成 `fs::rename`；重试 3 次指数退避（1s/4s/16s）。
- 进度：事件 `DownloadProgress { repo, file, bytes_done, bytes_total }` → 日志广播 + UI。
- 代理：与 2.9 相同的三模式 client。

### 2.11 其余小库

- **sysinfo**：`System::new_with_specifics(RefreshKind::everything())` 每 1s 采样 CPU%/RSS；worker RSS 用 `sysinfo::System::new()` + `refresh_processes(ProcessesToUpdate::Some([pid]))`。
- **dirs**：`dirs::home_dir()` → `.config/livetranslate`（用户指定语义，非 %APPDATA%，D-9）。
- **tracing**：`tracing_subscriber::registry()` + fmt(file, DEBUG, 滚动) + fmt(stdout, INFO) + 自定义 `broadcast::Layer`（`tokio::sync::broadcast`）→ LogWin/下载框；target 前缀 `livetranslate::*`。
- **serde_yaml**：解析内嵌 i18n（`include_str!` → `HashMap<String,String>`）。
- **chrono**：`%Y%m%d_%H%M%S` 文件名、`%H:%M:%S` 消息时间戳。

---

## 3. 数据契约（结构/schema 全表）

### 3.1 Settings（settings.json，键=原版，见 RESEARCH 附录 A）

```rust
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    pub vad_mode: String,               // "silero"|"energy"|"disabled"
    pub vad_threshold: f32,             // 0.5（向导写 0.3）
    pub energy_threshold: f32,          // 0.02
    pub min_speech_duration: f32,       // 1.0
    pub max_speech_duration: f32,       // 8.0
    pub silence_mode: String,           // "auto"|"fixed"
    pub silence_duration: f32,          // 0.8
    pub asr_engine: String,             // "funasr"|"whisper"（非法→funasr+log）
    pub funasr_model: String,           // 3 值（非法→sensevoice-small+log）
    pub asr_language: String,           // "auto"|29 码
    pub whisper_model_size: String,     // 6 档|本地 GGML 路径
    pub sensevoice_pad_seconds: f32, pub whisper_pad_seconds: f32,   // 0.5
    pub hub: String,                    // "ms"|"hf"
    pub download_proxy: String,         // "none"|"system"|URL
    pub incremental_asr: bool,          // false
    pub interim_interval: f32,          // 2.0
    pub audio_device: Option<String>,   // None|名|"__disabled__"
    pub mic_device: Option<String>,     // None|"__default__"|名
    pub models: Vec<ModelConfig>,
    pub active_model: usize,
    pub target_language: String,        // "zh"
    pub system_prompt: String,
    pub timeout: u32,                   // 10
    pub ui_lang: String,                // "en"|"zh"
    pub style: Style,                   // §3.5
    pub subtitle_mode: SubtitleMode,
    pub overlay_x: Option<i32>, pub overlay_y: Option<i32>,
    pub overlay_w: Option<u32>, pub overlay_h: Option<u32>,
    pub auto_save_transcript: bool,     // true
    pub models_dir: Option<PathBuf>,    // Rust 版新增；None→默认 ~/.config/livetranslate/models
}
```

### 3.2 ModelConfig（条件序列化=原版 get_data 契约）

```rust
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub name: String, pub api_base: String, pub api_key: String, pub model: String,
    pub proxy: String,                                   // "none"|"system"|URL
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub no_system_role: bool,
    #[serde(skip_serializing_if = "Option::is_none")]    pub thinking_style: Option<String>,  // ≠"auto" 才写
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub streaming_off: bool,  // 见注
    #[serde(skip_serializing_if = "std::ops::Not::not")] pub json_response: bool,
    #[serde(skip_serializing_if = "is_zero")]            pub context_turns: u32,
    #[serde(skip_serializing_if = "is_zero")]            pub input_price: f64,
    #[serde(skip_serializing_if = "is_zero")]            pub output_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, Value>>,      // 仅勾选键
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<Value>,                       // JSON object
}
// 注：原版键名 streaming 仅在 false 时写入 → 兼容写法：字段名 streaming，默认 true，
// skip_serializing_if = "is_true"（写 false 时输出 "streaming": false）。
// 读取时兼容旧 no_think 布尔 → thinking_style 迁移（auto/off）。
```

### 3.3 worker IPC 帧格式（小端）

```
帧 = [u32 payload_len][payload]
request  payload = JSON {"id":"hex32","type":"transcribe|set_language|set_input_padding|shutdown",
                         "word_ts":bool?} + (transcribe 时紧跟 f32 LE PCM)
response payload = JSON {"id":回显,"ok":bool,"type":"ready|result|ack|error|shutdown",
                         "payload"?,"error":{"message","recoverable"}?}
首帧 ready（id=null）：{"ok":true,"type":"ready","payload":{"engine","display_name"}}
超时：ready 180s / transcribe 60s（0.2s 步进）/ set_* min(10,60)s / shutdown 5s→kill
```

### 3.4 模型注册表（lt-models::registry）

```rust
pub struct ModelEntry { key, display, hub_hf, hub_ms: Option<String>, always_hf: bool,
                        estimated_bytes: u64, files: &'static [&'static str] }
```
| key | HF repo | MS | 体积 | 说明 |
|---|---|---|---|---|
| sensevoice-small | csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17 | 同左【M2 核对】 | ~250MB | 默认 |
| funasr-nano-2512 | csukuangfj/sherpa-onnx-funasr-nano-*【M5 核对文档页】 | 同左 | ~1.1GB | 实验性 |
| funasr-mlt-nano | — 无转换 — | — | — | UI 置灰 |
| whisper-{tiny,base,small,medium,large-v3} | ggml-org/whisper-{size}（默认 q5_0 量化文件） | — always HF | 78M~3.1G | whisper.cpp |
| whisper-turbo | ggml-org/whisper-large-v3-turbo | — always HF | ~809M | 快档 |
（Silero：内嵌，不占注册表）

ggml 文件名映射【M5 首日核对仓内实际文件名】：fp16=`{size}.bin`、量化=`{size}-q5_0.bin`（默认）/`{size}-q8_0.bin`。

### 3.5 Style 14+1 预设全表（lt-ui::style，字段=原版 15 键）

BASE（default）：`preset=default, bg_color=#000000, bg_opacity=240, header_color=#1a1a2e, header_opacity=230, border_radius=8, original_font_family="Microsoft YaHei", translation_font_family="Microsoft YaHei", original_font_size=11, translation_font_size=14, original_color=#cccccc, translation_color=#ffffff, timestamp_color=#888899, window_opacity=95`

| 预设 | 覆盖字段 |
|---|---|
| transparent | bg_opacity=120, header_opacity=120, window_opacity=70 |
| compact | original_font_size=9, translation_font_size=11 |
| light | bg=#e8e8f0/230, header=#c8c8d8/220, orig=#333333, trans=#111111, ts=#666688 |
| dracula | bg=#282a36/235, header=#44475a/230, orig=#f8f8f2, trans=#f8f8f2, ts=#6272a4 |
| nord | bg=#2e3440/235, header=#3b4252/230, orig=#d8dee9, trans=#eceff4, ts=#4c566a |
| monokai | bg=#272822/235, header=#3e3d32/230, orig=#f8f8f2, trans=#f8f8f2, ts=#75715e |
| solarized | bg=#002b36/235, header=#073642/230, orig=#839496, trans=#eee8d5, ts=#586e75 |
| gruvbox | bg=#282828/235, header=#3c3836/230, orig=#ebdbb2, trans=#fbf1c7, ts=#928374 |
| tokyo_night | bg=#1a1b26/235, header=#24283b/230, orig=#a9b1d6, trans=#c0caf5, ts=#565f89 |
| catppuccin | bg=#1e1e2e/235, header=#313244/230, orig=#cdd6f4, trans=#cdd6f4, ts=#6c7086 |
| one_dark | bg=#282c34/235, header=#3e4452/230, orig=#abb2bf, trans=#e5c07b, ts=#636d83 |
| everforest | bg=#2d353b/235, header=#343f44/230, orig=#d3c6aa, trans=#d3c6aa, ts=#859289 |
| kanagawa | bg=#1f1f28/235, header=#2a2a37/230, orig=#dcd7ba, trans=#dcd7ba, ts=#54546d |
（每预设自动含 preset=自身名；custom=用户改任何字段后自动切换）

### 3.6 事件/命令（lt-proto::events）

继承 RESEARCH §4.2 枚举并补充托盘：`Msg::Tray(TrayIconEvent) / Msg::Menu(MenuEvent) / Msg::Pipe(UiEvent)`。UI 线程每帧 `try_recv` drain 全部。

---

## 4. 里程碑任务分解（任务卡格式：内容→文件→完成标准）

### M0 骨架（预计 2-3 天）

| # | 任务 | 文件 | 完成标准 |
|---|---|---|---|
| 0.1 | workspace+全部 crate 空壳编译 | 全部 Cargo.toml | `cargo build --release` 通过 |
| 0.2 | **R-4 验证**：最小 bin 同时链 ort+sherpa，各跑一次推理 | lt-app/examples/coexist.rs | 两库同进程加载成功；失败则按 §8 预案调整 feature 后重试 |
| 0.3 | R-12 验证：sherpa sys 构建在本地网络可用 | — | 构建成功；记录 SHERPA_ONNX_LIB_DIR 缓存方法 |
| 0.4 | winit 多窗口宿主：4 窗口（overlay 透明无边框置顶 / subtitle 同 / panel 常规 / log 隐藏）+ 各自 egui State + glow | lt-ui::app.rs | 4 窗口显示，透明窗口背景正确，窗口可独立设置 flags |
| 0.5 | 托盘：全菜单树（RESEARCH §1.3）+ 事件转发 + 运行时图标 | lt-ui::tray.rs | 菜单结构与原版逐项一致；点击事件到 UI |
| 0.6 | i18n 内嵌 + t() 缺 key 回退 + sys-locale 检测（zh/en） | lt-i18n | zh/en 切换生效 |
| 0.7 | Settings 读写：原子写、默认值、兼容导入（非法键回退+log） | lt-proto/lt-models | 单测：读原版样例 settings.json 全字段正确 |
| 0.8 | tracing 基建 + broadcast layer + 单实例互斥量 | lt-app | 日志文件/控制台/订阅三路输出；双开被拒 |

### M1 音频 + VAD（3-4 天）

| # | 任务 | 完成标准 |
|---|---|---|
| 1.1 | `AudioBackend` trait + `NativeChunk` + 公共层纯函数（resample_linear / to_mono / mix / rms / pad_bucket） | 公共层单测：与 numpy 参考输出逐样本一致（用 Python 生成对照数据存 tests/fixtures） |
| 1.2 | wasapi 后端：设备枚举、loopback 事件驱动读、mic 混合前缓冲、2s 默认设备轮询重连、`__disabled__` 喂零 | 播放音乐 → 通道持续出 16k mono chunk；切默认设备自动跟随 |
| 1.3 | capture 线程：队列（满丢旧）、RMS/VAD monitor 事件、超时喂静音推进 | 与原版同录音输入下切分时间戳 ±1 chunk |
| 1.4 | SileroVad（ort）：state 回喂封装 + 输入名核对 | 同一 wav 逐 512 样本置信度曲线与 Python 版最大误差 <1e-4 |
| 1.5 | VAD 状态机全量移植（§5.1 参数表） | 单测：渐进静音/自适应/回溯切分/密度过滤/trim 标志各路径 |
| 1.6 | MonitorBar 数据链（1s 节流 sysinfo） | overlay 显示 RMS/VAD 条 + CPU/RAM |

### M2 ASR 主路径（3-4 天）

| # | 任务 | 完成标准 |
|---|---|---|
| 2.1 | worker 子进程框架：帧编解码、超时表、ready/result/ack/error、Job Object | echo 引擎（返回静音文本）全命令往返 + kill 回收单测 |
| 2.2 | AsrWorkerClient 状态机：generation 号、重启 ≤3、错误分类 fatal 规则、RSS 回收（+2048MB） | 注入崩溃/超时的假 worker，行为与原版语义一致 |
| 2.3 | SenseVoice 引擎（sherpa）：加载模型、transcribe、标签剥离后处理、pad 桶、set_language/padding | 用官方样例 wav 出正确文本；标签处理单测 |
| 2.4 | 下载器：HF/MS 双 hub、代理三模式、续传、进度事件、缓存探测（is_asr_cached 双 hub 或逻辑 + 半体积阈值） | 断网续传恢复；探测逻辑对人工布局目录的单测 |
| 2.5 | 向导（单页+15s 倒计时+hub 按语言默认+代理三选）+ 下载对话框（日志流）+ 加载对话框 | 全新目录首启：向导→下载 SenseVoice（内嵌 silero 秒过）→写 13 键默认块 |
| 2.6 | 管道装配：capture→vad→asr 队列→段处理（噪声/语言过滤）→UI 消息 | 对着扬声器说话，悬浮窗出 `[lang] 原文` |

### M3 翻译（3 天）

| # | 任务 | 完成标准 |
|---|---|---|
| 3.1 | **【M3 首日】验证 async-openai 流式 byot 入口**（R-13） | 确定 API；或落地 40 行 SSE 兜底 |
| 3.2 | Translator：thinking.rs（表+测试对照原版 test_translator_thinking.py 用例）+ messages/prompt 组装 + byot 结构 + overrides/extra_body 合并 | thinking 6 风格 × 端点矩阵单测全对 |
| 3.3 | translate_iter：流式聚合、deadline、usage 先带后撤、json {"t"} 提取、repetition 检测、context 历史 | DeepSeek/本地 LM Studio 流式逐字显示；超时/断连错误分类显示 `[error: ...]` |
| 3.4 | 模型列表 UI + ModelEditDialog（Basic+Advanced 条件序列化）+ benchmark tab | 编辑→立即生效；get_data 序列化形状与原版 JSON 逐键一致（快照测试） |
| 3.5 | 费用/统计链路（pt/ct 累计、¥/$） | MonitorBar stats 更新 |

### M4 UI 完备（5-7 天）

| # | 任务 | 完成标准 |
|---|---|---|
| 4.1 | overlay 全量：DragHandle 16 信号行为、消息富文本（span 着色）、流式 50ms 节流、50 条上限、右键菜单/导出、compact 200ms 动画、位置 500ms debounce 持久化、光标感知穿透 50ms | 与原版截图并排走查（zh/en 双语各一遍） |
| 4.2 | 字幕窗：描边渲染+缓存、贪心换行、6 动画、高度 150ms 垂直居中、自动隐藏+反向恢复、1500ms 最小显示、多句 `\|`、背景三态、中键拖动、穿透 500ms | OBS 采集画面无黑底；动画节奏与原版一致 |
| 4.3 | 控制面板 7 tab 全控件 + 联动（引擎可见性/高度自适应）+ 300ms debounce + 滑条拖动语义 + prompt 600ms | 每个控件写读写路径单测（settings 断言） |
| 4.4 | 字幕设置 tab（行列表 CRUD/上下移/行编辑 16 字段对话框） | 序列化 JSON 与原版结构一致 |
| 4.5 | 日志窗（2000 环形/级别过滤/内容高亮三色/自动滚动开关） | 高亮命中 ASR/Translate/Segment |
| 4.6 | 托盘↔UI↔面板三方状态同步（模型/语言/穿透复选） | 切换任一入口其余两处同步 |

### M5 引擎扩展（2-3 天）

| # | 任务 | 完成标准 |
|---|---|---|
| 5.1 | whisper 引擎（whisper.cpp）：6 档下载/加载/beam=5/语言检测/本地 .bin 扫描 | 与 faster-whisper 同段输出人工对比可接受；切换/失败回滚/自动重启链路 |
| 5.2 | Nano 实验性：注册表+下载+引擎+语言启发式+UI 实验性标注；MLT 置灰 | 甄别真 nano 包并实测质量后决定发布默认开/关 |
| 5.3 | 双栈构建验证（whisper.cpp+sherpa+ort 同 exe） | release 构建体积记录（预算 35-65MB） |

### M6 打磨（3-5 天）

interim ASR（§5.3 全逻辑）→ 内存回收实测 → 托盘气泡三处 → 单实例/Ctrl+C → 启动 <2s、空闲 CPU<1% 调优（repaint 策略）→ 长跑 8h 稳定 → CI 产物。

---

## 5. 关键算法规格（伪代码级，参数=原版逐字）

### 5.1 VAD 状态机（vad_processor.py 全参数）

```
常量: pre_speech_chunks=3; tiers=[(3.0s,1.0),(6.0s,0.5),(10.0s,0.25)];
      adaptive: pause_history(50), 目标=P75*1.2 clamp[0.3,2.0]s; 平滑窗=5; 搜索起点=n*3//10;
      dip_ratio<0.8 或 min_val<thr; 密度<0.25 丢; energy: min(1, rms/(thr*2)); disabled: 恒 1.0
      energy 模式下 effective_threshold 固定 0.5（非用户阈值）！
process_chunk(c):
  conf = 置信度(c)
  if conf >= eff_thr:
      若 _is_speaking 且 silence_counter>0: pause=silence_counter*0.032; ≥0.1s 记入 pause_history
        （auto 模式立即 update_adaptive_limit）
      若 not _is_speaking: speech_buffer += pre_buffer(每 chunk 置 conf=eff_thr); pre_buffer.clear()
      _is_speaking=true; silence=0; push(c, conf)
  elif _is_speaking: silence+=1; push(c, conf)
  else: pre_buffer.push(c)
  if samples >= max: return split_at_best_pause()      # 平滑谷搜索，见上常量；无谷→硬 flush
  if _is_speaking and silence >= eff_limit(tiers 乘 silence_limit):
      if samples >= min_speech: flush_segment()        # 密度<0.25 丢
      elif _was_trimmed: force_flush()
      else: _is_speaking=false; silence=0; return None  # 短段保留待合并
peek_buffer()/trim_front(n)（部分 chunk 裁剪、置 _was_trimmed）/force_flush()/flush()
```

### 5.2 SenseVoice 后处理

```
LANG_MAP = {"<|zh|>":zh,"<|en|>":en,"<|ja|>":ja,"<|ko|>":ko,"<|yue|>":yue}   # 首个命中定语言并删除
text = regex_replace(r"<\|[^|]+\|>", "", text).trim()                        # 情感/事件/BGM 全剔除
空→None；language_name = language（不查显示名表）
```

### 5.3 增量 ASR（interim，main.py）

```
触发: incremental_enabled 且 vad._is_speaking 且 total≥interval 且 elapsed≥interval 且 cooldown≥1.0s
流程: peek(≥1.5s 才做) → transcribe(无 word_ts) → strip_committed_overlap(尾 50 字符前缀匹配, ≥3 字符才剥)
     → split_sentences(句末标点+缩写保护+逗号回退: 、@25/其他@60, before>15, after>3)
     → >1 句才提交 complete[:-1]；短句(≤8 字符)入 pending 前置合并
     → 裁剪 = committed_chars/full_chars × total + 0.3s 余量, 上限留 0.5s, 下限 0.3s 且 ≤total/2
     → vad.trim_front; committed_tail=committed[-50:]; _interim_active=true
vad_flush 到达且 _interim_active → interim_final（去回声+pending 前置+同噪声过滤）
interim 请求入队前去重（drain 同类）
```

### 5.4 Translator 关键表

```
thinking styles: auto|deepseek|qwen|vllm|openai|off
  auto 启发式: model∈{deepseek,glm} 或 endpoint∈{deepseek,volces,api.z.ai,bigmodel} → deepseek
              endpoint∈{api.openai.com,api.x.ai,api.anthropic.com} → off; 否则 qwen
  body: deepseek {"thinking":{"type":"disabled"}} / qwen {"enable_thinking":false}
        vllm {"chat_template_kwargs":{"enable_thinking":false}} / openai {"reasoning_effort":"none"} / off {}
  legacy: no_think=true→auto, false→off
repetition: len≥40, plen∈[8,len/2], text[plen:2plen]==text[:plen]
费用: (pt*in+ct*out)/1e6; 货币符号 ui_lang==zh→¥ else $
prompt 模板 format 失败→回退 DEFAULT_PROMPT; json_response 追加 '\nRespond in JSON format: {"t": "translated text"}'
context: history 尾 context_turns 对 (Source:/Translation:)；context_turns=0 清空
```

### 5.5 音频公共层

```
to_mono: interleaved → 各通道平均
resample_linear(src, from, to=16000): n_out=len*to/from; idx=i/ratio; floor/ceil clamp; frac 插值  # 逐位对齐 numpy
mix: loopback_chunk + mic_chunk[:n]（mic 不足补零）；mic_rms 单独
pad_bucket(audio, quantum=round(16000*pad)): remainder==0 原样; 否则尾部补零至整倍
```

---

## 6. 经验教训清单（研究过程沉淀，动工时逐条对照）

**来自原版代码的移植经验（E-xx）**：
- E-01 原版 `torch` 必须先于 PyQt6 导入（DLL 顺序）→ Rust 无此问题，但**原生库初始化顺序仍要固定**：COM/线程池在 main 早期、模型加载只在 worker。
- E-02 **Silero ONNX 必须自管 state 回喂**（JIT 包装有状态、裸 ONNX 没有）——忘了它置信度全错。
- E-03 原版 `proxy="none"` 用 trust_env=False 绕系统代理（localhost 被代理坑过）→ reqwest 必须 `no_proxy()`；**远程功能虽裁剪，此语义在 LLM/下载全链路保留**。
- E-04 **WS_EX_TRANSPARENT 必须与 WS_EX_LAYERED 成对设置**；且每 50ms/500ms 重断言（窗口操作会重置 EXSTYLE）。
- E-05 multiprocessing daemon 在父异常退出时可残留 → Job Object KILL_ON_JOB_CLOSE 兜底。
- E-06 原版 worker RSS 基线+2048MB 回收是防 C++ 侧泄漏——sherpa/whisper.cpp 同为 native，**保留该机制**。
- E-07 FunASR tqdm 在 GUI 进程刷 stderr 会崩 → Rust 版进度全部走结构化事件；worker 的 stdout/stderr 重定向到日志（绝不弹控制台，CREATE_NO_WINDOW）。
- E-08 设置写入 float 必须 round(2)（Python 浮点漂移 0.9999... 的教训）；原子写 tmp+rename。
- E-09 wasapi 的 COM init 必须在使用它的那个线程上做（initialize_mta per audio thread）。
- E-10 原版 `is_asr_cached` 的 HF snapshot 取"字典序最后"而非 mtime；完整性阈值 50MB/半体积——照抄。
- E-11 compact 动画曾因 Windows MINMAXINFO 不一致改用 frameGeometry → egui 下直接管理 size，但**多 DPI 下保存/恢复坐标要用物理像素并做 _is_pos_visible 多屏校验**（原版语义）。

**来自 2026-09-05 中期审计的纠偏记录（P0/P1 清单已执行）**：
- M2.6 曾在"管道侧跑通"时打勾，但悬浮窗未接 AddMessage（验收=用户可见 `[lang] 原文`）——**里程碑打勾以本表"完成标准"列的用户可见行为为准**，不以内部链路为准。
- 装配层文件（pipeline.rs/manager.rs/app.rs）是"无逐行原文可抄"的新发明区，算法层（有 Python 原文对照）零偏离——**每写装配层文件前先回读 main.py 对应段落**（M-05 的强化版）。
- 已知实现级偏差（非 bug，记录在案）：① SenseVoice `set_language` 经重建识别器实现（sherpa 语言是创建期参数），切换语言会重载模型（秒级），原版为瞬时改参——切换罕见，接受；② 段队列/监视事件的线程拓扑已对齐原版（capture 直推段队列=原版 `_enqueue_asr`，容量 16；monitor 直接经 EventLoopProxy=原版跨线程信号）；③ RSS 回收仅在段队列空闲时进行（原版 `_asr_loop` queue.Empty 分支语义）。

**来自本次调研的方法论（M-xx）**：
- M-01 **不赌未文档化的第三方内部行为**（hf-hub 缓存布局教训）→ 关键依赖面要么官方文档确认，要么自控（自写下载器）。
- M-02 **推理栈选型必须核对解码策略与质量证据**（sherpa whisper 仅 greedy + CER 告警教训）→ 已用 whisper.cpp 解决；后续换任何推理库先问"解码参数能否 1:1"。
- M-03 权威封装优先（用户偏好，async-openai/whisper.cpp 均此原则），自写代码集中在"业务语义层"（translator 逻辑/VAD 状态机/下载布局）。
- M-04 每个里程碑首日安排"高危验证"（M0=R-4/R-12、M3=R-13、M5=模型文件名/语言 API），失败当天切预案，不拖。
- M-05 对 Python 源码的行为疑问一律回读源文件，不凭记忆（本次复核纠出 6 处）。
- M-06 **依赖版本耦合审计（2026-09-05 施工期执行）**：注入式 API（async-openai 的 Client::build(http_client)）要求注入方与库方 reqwest 大版本严格一致——async-openai 0.41.3 依赖 reqwest 0.13，故全工程 reqwest 锁 0.13；同理今后凡注入 client/handle 的组合，升级一方必须同查另一方。版本基线已全量对齐最新（egui 0.36.1 / winit 0.30.13 / windows 0.62.2 / tray-icon 0.24.2 / muda 0.19.3 / wasapi 0.24 / whisper-rs 0.16 / ort 2.0.0-rc.13 / sherpa-onnx 1.13.7 / sysinfo 0.39 / pollster 1.0）；serde_yaml 已停止维护但为最终稳定版（0.9.34），内嵌 i18n 用途无升级必要。

---

## 7. 测试计划

| 层 | 内容 | 对照基准 |
|---|---|---|
| 单元 | 音频公共层（重采样/混音/pad） | Python 生成 fixtures 逐样本 |
| 单元 | VAD 状态机全路径 | 构造置信度序列（不跑真模型）断言切分点 |
| 单元 | Silero 置信度 | Python silero-vad 对同一 wav 的输出（<1e-4） |
| 单元 | 分句器 | 原版 tests/test_segmentation.py 用例全量移植 |
| 单元 | thinking styles | 原版 tests/test_translator_thinking.py 用例全量移植 |
| 单元 | settings 兼容导入 | 原版 user_settings.json 样例（含 legacy 键） |
| 单元 | ModelConfig 序列化 | get_data 形状快照（原版 JSON 样例） |
| 单元 | 缓存探测/注册表 | 人工目录布局 fixtures |
| 集成 | worker 协议（echo/崩溃/超时/回收） | 语义对照 asr_client.py 行为 |
| 冒烟 | 全管道（放一段含语音的 wav 进 capture 入口） | 出原文+译文+transcripts 三文件 |
| 走查 | UI 双语截图对比、托盘全菜单、OBS 采集 | 原版 screenshot/zh.png、en.png |

---

## 8. 风险预案速查（动工时遇到直接翻这里）

| 风险 | 症状 | 预案 |
|---|---|---|
| R-4 ort↔sherpa 符号冲突 | M0.2 链接失败/duplicate symbol | A：ort 开 `load-dynamic` 指向 sherpa 捆带的 onruntime 动态库；B：VAD 移入独立小进程；C：检查 sherpa C API 是否暴露逐 chunk 置信度，若有则去 ort |
| R-13 无流式 byot | M3.1 找不到 API | 手写 POST `/chat/completions`（stream）+ reqwest bytes_stream 解析 SSE（~40 行，仅流式路径） |
| R-5 MS files 接口不符 | M2.4 解析失败 | curl 实测响应结构调整解析；必要时改用逐文件试下载 + HTML 目录页兜底 |
| R-6 egui 多窗口坑 | 透明失效/事件丢失/性能 | 参考 egui-multiwin 源码逐窗口隔离 State；DWM 问题退路：每窗口独立 glow context |
| R-12 sherpa 构建下载失败 | build.rs 网络错误 | 手动下预编译包解压设 SHERPA_ONNX_LIB_DIR；CI actions/cache |
| whisper.cpp 模型文件名不符 | M5.1 404 | 以 ggml-org 仓实际文件名为准更新注册表映射表 |
| SenseVoice MS 镜像缺失 | M2.4 下载失败 | always_hf 兜底（原版 anime/whisper 本就 always HF 的先例） |

---

## 9. 最终验收走查清单（对原版 1:1）

1. 首启：向导（倒计时/hub 语言默认/代理）→ 下载日志流 → API 配置弹窗（预填 LM Studio 示例）；
2. 主流程：说话 → 悬浮窗 `[HH:MM:SS] [lang] 原文 ASR xx ms` → `> 译文 TL xx ms` 流式逐字 → 同语言 `(相同语言)`；
3. 悬浮窗：7 按钮/4 复选/3 下拉、compact 动画、穿透（header 可点正文穿透）、样式 14 预设切换、导出 3 模式、位置记忆；
4. 字幕窗：中键拖动、描边大字、动画、自动隐藏、多语并行翻译、OBS 可采集；
5. 面板：7 tab 全控件、自动保存、引擎切换可见性联动、模型 CRUD/复制、benchmark 输出格式、缓存扫描/删除退出、changelog 渲染；
6. 托盘：全菜单树、模型/语言/穿透三向同步、3 处气泡；
7. 设置迁移：原版 user_settings.json 直接放入 `~/.config/livetranslate/settings.json` → 非法键回退+日志、位置/样式/模型全保留；
8. 稳定性：8h 长跑 RSS 平稳、worker 崩溃自动恢复 ≤3 次、主进程崩溃无孤儿 worker；
9. 产物：单 exe 双击即用（干净 Win10 x64 虚拟机验证）、35-65MB。

---

## 附：与 RESEARCH.md 的条款映射

本计划所有决策源自 RESEARCH.md r8：选型依据 §2、模型表 §3、架构 §4、偏差 D-1~D-16、风险 R-1~R-13。冲突时以本文档的任务分解为准、以 RESEARCH.md 的决策记录为解释。
