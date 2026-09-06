# ASR 引擎扩展可行性分析与施工方案（FunASR Nano / Qwen3-ASR / 开源模型扫描）

> 日期：2026-09-07 ｜ 性质：调研文档（未改任何代码）｜ 前置：RESEARCH.md r8 双栈结论、PLAN.md §2.6/§5.2、docs/parity-closure-plan.md WP-2
>
> 调研方法：仓内代码逐点核对（文中 file:line 均为实测）+ sherpa-onnx 官方文档/issue + HuggingFace / ModelScope API 实测（hf-mirror 直连核对仓库存活性与文件清单）+ Qwen3-ASR 官方开源信息（2026-01 技术报告 arXiv:2601.21337）。

---

## 0. 结论速览（TL;DR）

| 议题 | 判定 | 一句话依据 |
|---|---|---|
| **FunASR Nano（funasr-nano-2512）实装** | **可行，接近 turnkey，建议立即做（WP-A）** | 锁定的 sherpa-onnx 1.13.7 Rust binding 已暴露 `OfflineFunASRNanoModelConfig`；官方 int8 转换包实测存在（948MB，CPU RTF 0.144–0.191@2 线程）；manager 层的 nano 无-padding 逻辑**早已写好**（`manager.rs:304-312`）。但注册表条目是错的（仓库名不存在 + 文件清单错误），须先修正，否则就是 parity-closure-plan WP-2 预警的"下载 1.1GB 后必坏"用户陷阱 |
| **Qwen3-ASR-0.6B 接入** | **可行，中低风险，建议做（先 0.5 天 spike 再实装，WP-B）** | Qwen3-ASR 已于 2026-01 以 Apache 2.0 开源（0.6B/1.7B）；sherpa-onnx 官方已支持 0.6B int8 离线识别（2026-03-25 转换包，HF API 实测存在，941MB）；binding 已有 `OfflineQwen3ASRModelConfig`；**CPU RTF 实测 0.077–0.168@2 线程（比 nano 还快）**。属超出原版 1:1 的新能力，需用户拍板设置拓扑（本文推荐方案见 §4.2） |
| Qwen3-ASR-1.7B | **不可行，不接** | sherpa-onnx 官方未支持（issue #3535 open），仅社区非官方转换包；1.7B AR 解码纯 CPU 实时性无保障 |
| funasr-mlt-nano-2512 | **维持置灰，但有条件解锁路径** | 官方（csukuangfj）至今无 MLT 转换；社区包 `lorneluo/sherpa-onnx-funasr-mlt-nano-int8-2512` 实测存在（文件布局与官方 nano 一致），质量未验证。D-14 维持，除非用户愿意吃社区包风险（见 §6.3） |
| 近两年其他开源 ASR | **无一需要现在接；两个观察名单** | 扫描结论：zh 核心场景下现有 SenseVoice（默认）+ whisper 6 档 + nano + qwen3-0.6B 已覆盖速度/质量两端。观察名单：**Cohere Transcribe 2B**（2026-03，Apache 2.0，14 语含 zh，sherpa binding 已有字段）、**FireRedASR2-CTC**（zh 质量备选）。其余（Parakeet/Canary/Voxtral/Moonshine/Kimi-Audio 等）或无中文、或 CPU 不可行，详见 §6 |

**建议施工顺序**：WP-A（nano 实装 + 注册表修正，0.5–1 天）→ WP-B spike（qwen3 加载验证，0.5 天）→ WP-B 实装（1–1.5 天）。WP-C（缓存完整性硬化）并入 WP-A 必做项。

---

## 1. 现状盘点（改造基线）

### 1.1 引擎架构（双栈，均已上线）

```
Settings { asr_engine: "funasr"|"whisper", funasr_model, whisper_model_size, ... }
    │
    ▼  pipeline.rs::build_worker_config (crates/lt-app/src/pipeline.rs:582)
WorkerConfig { engine: "sensevoice"|"whisper", display_name, language, pad_seconds, options{model_dir|model_path} }
    │  spawn: 当前 exe --asr-worker <config-json>
    ▼  main.rs::asr_worker_entry (crates/lt-app/src/main.rs:103) —— match engine 分发
    ├─ "sensevoice" → lt_asr::sensevoice::SenseVoiceEngine   （sherpa-onnx OfflineRecognizer）
    └─ "whisper"    → lt_asr::WhisperEngine                  （whisper-rs / whisper.cpp）
    ▼  stdin/stdout 帧协议（transcribe / set_language / set_input_padding / shutdown）
AsrManager（crates/lt-asr/src/manager.rs）：重启≤3次、错误分类、RSS 回收（基线+2048MB）、
    引擎切换失败回滚旧 worker、padding 按引擎家族挂起（engine_family: "sensevoice"|"nano"→"funasr"）
```

- 推理依赖：`sherpa-onnx = "1.13"`（Cargo.lock 实锁 **1.13.7**，官方 Rust binding，构建时自动下预编译 CPU 原生库）；`whisper-rs = "0.16"`。纯 CPU 硬约束两者均满足。
- AsrEngine trait（`crates/lt-asr/src/engine.rs`）：`transcribe(audio, word_timestamps)` + `set_language` + `set_input_padding`（后两者默认返回 `Unsupported`）。
- 客户端超时预算（`crates/lt-asr/src/client.rs:125-127`）：**ready 180s / 单次请求 60s / shutdown 5s**。对 nano（948MB）与 qwen3（941MB）的加载时长和段级解码时长都充裕。

### 1.2 FunASR Nano 的现状：占位已成、实装未做、且有既存陷阱

| 层 | 状态 | 位置 |
|---|---|---|
| proto 契约 | ✅ `FUNASR_MODELS = ["sensevoice-small", "funasr-nano-2512", "funasr-mlt-nano-2512"]`，sanitize 已接受 nano 键 | `crates/lt-proto/src/settings.rs:17` |
| 模型注册表 | ⚠️ **条目存在但内容错误**（见 1.4） | `crates/lt-models/src/registry.rs:36-44` |
| 缓存探测/下载 | ✅ 机制通用，但完整性校验对多文件模型有漏洞（见 1.4/§3.4） | `crates/lt-models/src/cache.rs:116-126`、`crates/lt-app/src/backend.rs:134` |
| 装配分派 | ❌ 所有 funasr 条目一律映射 `engine:"sensevoice"`（nano 必坏点） | `crates/lt-app/src/pipeline.rs:592-605` |
| 引擎实现 | ❌ 无 `engines/nano.rs`（`engines/mod.rs` 仅 `pub mod whisper`） | `crates/lt-asr/src/engines/mod.rs` |
| worker 分发 | ❌ `asr_worker_entry` 只有 "sensevoice"/"whisper" 两臂 | `crates/lt-app/src/main.rs:105` |
| manager 层 | ✅ **已预留**：`engine_family` 认得 `"nano"`；padding 对 nano 跳过下发（原版 `funasr_supports_padding: nano=false` 的语义） | `crates/lt-asr/src/manager.rs:83,304-312` |
| UI 下拉 | ✅ nano 已可选并带"（实验性）"标注；mlt 灰显 | `crates/lt-ui/src/windows/panel/vad.rs:62-74`、i18n `model_experimental` |
| 下载向导/设置镜像 | ✅ 走 `missing_models(engine, funasr_model, ...)` 通用链路 | `crates/lt-app/src/backend.rs:104,176` |

**既存陷阱（parity-closure-plan WP-2，P0）**：当前用户在面板选中 nano → 触发下载（约 1.1GB）→ 下载"成功" → `build_worker_config` 仍映射 `engine:"sensevoice"` → SenseVoice 加载器找不到 `model.int8.onnx` → 加载必然失败。WP-A 实装后该陷阱自然消除（WP-2 的"置灰防坑"步骤即被取代）。

### 1.3 原版 Python 的 nano 行为（1:1 对齐的参照，`LiveTranslate/asr_funasr_nano.py`）

- 运行时：funasr `AutoModel`（PyTorch，trust_remote_code）+ **单独下载 Qwen3-0.6B 权重**（`ensure_qwen_weights`，约 1.5GB）。Rust 版走 ONNX 转换包**权重已内联**，Qwen3 下载链路确认不需要移植（RESEARCH.md D-14 已裁决，本次再度核实成立）。
- 后处理（`asr_funasr_nano.py:99-118`）：取 `text` → 正则清除全部 `<|...|>` 标签 → trim → 空文本或 `"sil"` → 视为无结果。
- 语言判定（`asr_funasr_nano.py:120-135`）：设了 `language` 用设值；否则启发式——假名(3040–30FF/31F0–31FF)>0 → ja；谚文(AC00–D7AF)>30% → ko；汉字(4E00–9FFF)>30% → zh；否则 en；空 → auto。
- `supports_padding=false`、`supports_language=true`（set_language 存值，下次 generate 传参）。
- 模型档案（`LiveTranslate/model_manager.py:75-110`）：原版估计体积 3.5GB（含 PyTorch 全量+Qwen3 权重）；Rust 版 ONNX int8 包远小于此。

### 1.4 注册表 nano 条目错误（本次实测确认，WP-A 必修项）

`registry.rs:36-44` 现状：
```rust
hf: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),   // ← 仓库不存在
ms: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),   // ← MS 上也无
files: &["model.int8.onnx", "tokens.txt"],                                    // ← 真包里没有这两种文件
estimated_bytes: 1_100_000_000,
```
实测（2026-09-07，HF API via hf-mirror）：
- `csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko` → **不存在**（API 返回仓库无效）。
- 真实官方包为 `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`（HF 存在，2026-01-13 更新），文件清单见 §2.2。
- ModelScope 上无对应单仓；`csukuangfj/asr-models` 是树状大仓（子目录套子目录），本项目的平面快照下载器无法使用 → **nano/qwen3 必须 `always_hf: true`**（国内用户经设置里已有的 hf-mirror endpoint 覆写下载，机制已存在：`download/mod.rs` 的 `hf_endpoint`）。

> 注：RESEARCH.md §3.8 当时预估的仓名 `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30` 是对的，注册表里的是后来误写的。另注意甄别：HF 上确有"换皮包"`csukuangfj/sherpa-onnx-sense-voice-funasr-nano-2025-12-17`（名字里带 sense-voice，即 #3061 警告的假 nano），**不要**用。

---

## 2. 关键事实核查（2026-09-07 实测）

### 2.1 Rust binding 能力面：两条路径都不需要升依赖

锁定的 `sherpa-onnx 1.13.7`（docs.rs 核对，`offline_asr.rs`）中 `OfflineModelConfig` 已含 18 个模型族字段，与本调研相关的：

```rust
pub struct OfflineModelConfig {
    pub sense_voice: OfflineSenseVoiceModelConfig,      // 已用
    pub whisper: OfflineWhisperModelConfig,             // 已用（whisper-rs 走另一栈，此字段未用）
    pub funasr_nano: OfflineFunASRNanoModelConfig,      // ★ WP-A
    pub qwen3_asr: OfflineQwen3ASRModelConfig,          // ★ WP-B
    pub fire_red_asr / fire_red_asr_ctc / dolphin / canary / moonshine /
    paraformer / nemo_ctc / zipformer_ctc / omnilingual / cohere_transcribe / ... ,
    pub tokens: Option<String>, pub num_threads: i32, pub model_type: Option<String>, ...
}

pub struct OfflineFunASRNanoModelConfig {   // docs.rs 实测字段
    encoder_adaptor, llm, embedding: Option<String>,   // 三个 onnx 文件
    tokenizer: Option<String>,                          // ★ 目录（Qwen3-0.6B/）
    system_prompt, user_prompt: Option<String>,
    max_new_tokens: i32, temperature: f32, top_p: f32, seed: i32,
    language: Option<String>,                           // ★ 支持指定语言（创建期参数）
    itn: i32, hotwords: Option<String>,
}

pub struct OfflineQwen3ASRModelConfig {
    conv_frontend, encoder, decoder: Option<String>,   // 三个 onnx 文件
    tokenizer: Option<String>,                          // ★ 目录（tokenizer/）
    max_total_len: i32, max_new_tokens: i32,
    temperature: f32, top_p: f32, seed: i32,
    hotwords: Option<String>,                           // ★ 注意：没有 language 字段（仅自动语种检测）
}
```

`Cargo.toml` 的 `sherpa-onnx = "1.13"` 语义不变，**无依赖升级**。唯一保留动作：WP-B spike 须实测原生库确实能加载 qwen3（binding 有字段 ≠ 原生版本足够新，1.13.7 大概率没问题，spike 兜底）。

### 2.2 Fun-ASR-Nano 官方转换包（sherpa-onnx 文档页 + HF API 实测）

仓库：`csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`（HF 存在 ✅；MS 无单仓 ❌）。

| 文件 | 大小 |
|---|---|
| `embedding.int8.onnx` | 149 MB |
| `encoder_adaptor.int8.onnx` | 227 MB |
| `llm.int8.onnx` | 573 MB |
| `Qwen3-0.6B/merges.txt` | 1.6 MB |
| `Qwen3-0.6B/tokenizer.json` | 11 MB |
| `Qwen3-0.6B/vocab.json` | 2.7 MB |
| **合计（下载清单，不含 README/test_wavs）** | **≈964 MB** |

另有 fp16（1.5GB）与 fp32（3.7GB）包，本项目只用 int8。转换脚本源自 `Wasser1462/FunASR-nano-onnx`。

- **CPU 实测速度**（sherpa 官方文档页，int8、2 线程、greedy）：**RTF 0.144–0.191**（≈5–7 倍实时）。RESEARCH.md §3.3 当时"纯 CPU 约 2–4s/段"的悲观估计可以下修：10 秒段约 1.4–1.9s。
- 质量：官方页对照 ground truth 的转写样例显示常规语音/专业词/多数歌词基本一致；闽南语、湖南话等方言误识较多（预期内）。
- 已知上游风险（RESEARCH 已记录，现状更新）：#3066 int8 重复文本问题在 2512 官方 int8 包的样例转写中未见复现；#3061 换皮包已甄别（真包文件名是 embedding/encoder_adaptor/llm 三件套）。
- 架构：SAN-M/DFSMN 编码器 + Qwen3-0.6B LLM 解码器（AR 自回归），31 语种+方言、支持歌词/说唱、热词。

### 2.3 Qwen3-ASR（2026-01 开源，Apache 2.0）

- 官方开源：`QwenLM/Qwen3-ASR`（GitHub），权重 0.6B / 1.7B + Qwen3-ForcedAligner，Apache 2.0，技术报告 arXiv:2601.21337。0.6B 官方口径"开源 ASR 中 SOTA、可比最强闭源"。
- sherpa-onnx 官方支持（**仅 0.6B**）：转换包 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`（HF API 实测存在 ✅；注意作者是 **csukuangfj2** 不是 csukuangfj）。导出脚本 `Wasser1462/Qwen3-ASR-onnx`。1.7B 无官方支持（issue #3535 open；社区有非官方包，不采信）。

| 文件 | 大小 |
|---|---|
| `conv_frontend.onnx` | 42 MB |
| `encoder.int8.onnx` | 174 MB |
| `decoder.int8.onnx` | 721 MB |
| `tokenizer/merges.txt` + `tokenizer/vocab.json` + `tokenizer/tokenizer_config.json` | ≈4.2 MB |
| **合计（下载清单）** | **≈941 MB** |

- **CPU 实测速度**（sherpa 官方文档页，int8、2 线程、greedy）：**RTF 0.077–0.168**（雷军 272s 长音频 VAD 批解码 0.077；短段 0.096–0.168）。**比 nano 略快**，10 秒段约 0.8–1.7s。
- 语言：30 语（zh/en/yue/ja/ko/de/fr/es/ru/…）+ 22 种中国方言变体 + 歌词/说唱。与本项目 `asr_language` 30 项语言表高度重叠。
- 配置默认值（C++ 打印核对）：`temperature=1e-06, top_p=0.8, seed=42, max_new_tokens=512, max_total_len=1024`（说唱/歌词场景官方建议 max_new_tokens=1024）。
- 实时/流式：官方即"VAD + 模拟流式"用法——与本项目现有 silero-VAD 分段 + 离线识别拓扑**完全同构**。无真流式（有 issue 在求）。

### 2.4 下载器与缓存层的两个硬约束

1. **下载是"按注册表 `files` 清单逐文件"**（`crates/lt-app/src/backend.rs:134` → `downloader.download_files(hub, repo, m.files, ...)`，HF 走 `/resolve/main/{path}` 直链）。所以注册表 `files` 必须**逐字精确**（含子目录前缀 `Qwen3-0.6B/…`、`tokenizer/…`），列错即 404。快照目录按 repo 相对路径落盘，子目录天然保留——**tokenizer 目录参数直接 `model_dir.join("Qwen3-0.6B")` / `model_dir.join("tokenizer")` 即可**。
2. **funasr 缓存完整性判定目前只看"snapshot 总字节 ≥ 50MB"**（`cache.rs:11,116-126`，原版 min_bytes 语义）。对 nano 的多文件包这是**漏洞**：只下完 `llm.int8.onnx`（573MB）单个文件就满足 50MB → 判"已缓存" → 下载向导不再补缺 → 引擎加载必败。**WP-A 必须一并加"清单全文件存在"条件**（对 sensevoice 语义兼容且更严，不破坏原版对齐——原版只有单包模型，此为新增模型族的必要防御）。

---

## 3. WP-A：FunASR Nano 实装（可行性：高；估时 0.5–1 天）

### 3.1 判定理由

- API 面：binding 1.13.7 已有 `OfflineFunASRNanoModelConfig`，模型族官方支持（文档页 `k2-fsa.github.io/sherpa/onnx/funasr-nano/pretrained.html`）。
- 模型面：官方 int8 包实测存在、964MB、RTF 0.15 左右，笔记本 CPU 可实时。
- 工程面：proto/下载/缓存/UI/manager 五层全部就绪（含 nano 无-padding 逻辑），缺口只有注册表内容、装配分派、引擎实现三处，且每处都有 sensevoice 先例可抄。
- 风险面：D-14 已裁决"实验性可选"；UI 已带"（实验性）"标注；默认引擎仍是 SenseVoice，nano 出问题不阻塞主路径。

### 3.2 改造清单（逐文件）

| # | 文件 | 改动 |
|---|---|---|
| A1 | `crates/lt-models/src/registry.rs:36-44` | `FUNASR_NANO` 修正：`hf = Some("csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30")`；`ms = None`；`always_hf = true`；`estimated_bytes = 1_010_000_000`；`files = &["embedding.int8.onnx", "encoder_adaptor.int8.onnx", "llm.int8.onnx", "Qwen3-0.6B/merges.txt", "Qwen3-0.6B/tokenizer.json", "Qwen3-0.6B/vocab.json"]`。同步改 `sensevoice_dual_hub`/`funasr_entry_keys` 等既有测试断言 |
| A2 | `crates/lt-models/src/cache.rs:116-126` | `is_funasr_cached` 增加清单完整性：HF 分支除总量 ≥50MB 外，还要求 `entry.files` 全部存在于 snapshot（含 MS 命中分支同样校验，因 MS 侧 sensevoice 镜像也可能半截）。补单测：只放 llm.int8.onnx（>50MB）→ 必须判未缓存 |
| A3 | `crates/lt-asr/src/engines/nano.rs`（新建） | 仿 `sensevoice.rs`：`NanoEngine::load(model_dir, language)`——配置 `funasr_nano { encoder_adaptor, llm, embedding, tokenizer: Some(model_dir.join("Qwen3-0.6B")), language, itn: 1 }`，`num_threads = 1`（与 sensevoice 一致；官方基准 2 线程，可后续调）；`transcribe` 不做 pad；后处理 1:1 移植原版：清 `<|...|>` → trim → 空或 `"sil"` → None → 启发式 LID（假名→ja / 谚文>30%→ko / 汉字>30%→zh / else en）填 `AsrResult.language/language_name`；`set_language` 为创建期参数 → 变更重建识别器（同 sensevoice.rs:173-183 策略）；`set_input_padding` 保持 trait 默认 `Unsupported`（manager 已按 nano 跳过下发）。单测：postprocess 全分支 + 启发式 LID 边界（30% 阈值恰好）+ load 目录缺失报错 |
| A4 | `crates/lt-asr/src/engines/mod.rs` | `pub mod nano;` |
| A5 | `crates/lt-app/src/main.rs:105` | `asr_worker_entry` 加 `"nano"` 分支（读 `options.model_dir` → `NanoEngine::load(&dir, &cfg.language)`；不传 pad_seconds） |
| A6 | `crates/lt-app/src/pipeline.rs:592-605` | funasr 分支按 `entry.key` 分派：`"sensevoice-small" → engine:"sensevoice"`（pad 照旧）；`"funasr-nano-2512" → engine:"nano"`，`pad_seconds: None`。`resolve_funasr_entry`（pipeline.rs:545）不改（nano 本就返回 Some） |
| A7 | `crates/lt-asr/src/manager.rs` | 无需改码：`engine_family("nano")→"funasr"`（:83）与 nano padding 跳过（:304-312）均已就位。补一条集成断言防回归 |
| A8 | `crates/lt-ui/src/windows/panel/vad.rs:62-74` | 不改结构；可选：display 追加实测速度提示（新 i18n 键）。WP-2 计划中的"置灰"步骤**作废**（由实装取代） |
| A9 | `assets/i18n/zh.yaml` + `en.yaml` | 新增 `model_nano_hint`（zh："Fun-ASR-Nano：LLM 解码架构，实验性；纯 CPU 实测约 6 倍实时（2 线程），首次加载较慢" / en 对应）。两份 yaml 必须同步 |
| A10 | 文档回写 | `RESEARCH.md` §3.3/R-3：实装完成记录 + RTF 实测数据 + 甄别结论（官方 2512 int8 包非换皮）；`docs/parity-closure-plan.md` WP-2 勾销（处置方式 = 实装而非置灰，含 D-6 决议"resolve_funasr_entry 维持不回退"的落实说明）；`AGENTS.md` 待办勾销 |
| A11 | 冒烟验收 | 临时 `LIVETRANSLATE_CONFIG_DIR`（settings.json 显式 `models_dir` 指真实缓存）：选 nano → 下载 964MB → 引擎切换成功 → 中/英/粤各一段实音识别 → 语言显示正确 → padding 滑杆对 nano 无效（日志无 padding 下发）→ 切回 sensevoice 正常 |

### 3.3 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| int8 AR 解码偶发重复/幻觉（上游 #3066 类） | 中 | 保持"实验性"标注；manager 现有错误分类+自动重启兜底；实测若高频复现，可在 postprocess 加相邻重复句折叠（**先不做**，避免擅自"改进"） |
| 首次加载慢（573MB llm + ORT 初始化） | 低 | ready_timeout 180s 充裕；加载期 UI 已有 AsrUnavailable→ready 事件链路 |
| worker 内存（int8 约 1–1.5GB 峰值） | 低 | RSS 回收阈值 = 基线+2048MB，天然覆盖 |
| 下载中断半包被判完整 | 已修 | A2 清单完整性校验 |

---

## 4. WP-B：Qwen3-ASR-0.6B 接入（可行性：中高；spike 0.5 天 + 实装 1–1.5 天）

### 4.1 判定理由

- 2026-01 官方开源（Apache 2.0），0.6B 为官方口径开源 SOTA；30 语 + 22 方言 + 歌词/说唱，与本项目使用场景（实时同传字幕、多语种会议室）高度匹配；粤语/方言覆盖比 SenseVoice 的五语更广。
- sherpa-onnx 官方转换 + binding 字段现成；**CPU RTF 0.077–0.168 实测**，实时性无虞；官方推荐用法（VAD 分段 + 离线识别）与本项目管线同构。
- 代价/风险点：超出原版 1:1 范围（原版无此引擎，属新增能力，需明确记录为新偏差）；无 `language` 参数（不能手动指定源语言，纯自动 LID）；上游 #3509 报告过 hotwords/language 相关参数影响输出的 bug（对策：不暴露 hotwords UI、采样参数固定官方默认）。

### 4.2 决策点：设置拓扑（需要用户/主线程拍板）

| 方案 | 说明 | 评价 |
|---|---|---|
| **B-α（推荐）：`asr_engine` 新增第三值 `"qwen3"`** | 引擎下拉追加第三项（funasr / whisper / Qwen3-ASR）；`ASR_ENGINES`（settings.rs:15）扩为 3 值并放行 sanitize；worker engine key `"qwen3"` | 语义干净：它不是 FunASR 模型，不进 `funasr_model` 的 3 项 1:1 结构（D-14 明确保持 3 项）；padding 家族独立（`engine_family("qwen3") → "qwen3"`，UI 无对应 pad 控件 → 挂起表天然为空，manager 无需改）；原版 settings 导入不受影响（导入只可能产生 funasr/whisper/回退值）；下拉**追加在尾部**，既有两项索引不动 |
| B-β：`funasr_model` 加第 4 项 `"qwen3-asr-0.6b"` | 复用 funasr 引擎族 | 不推荐：污染 D-14 的 3 项 1:1 结构；"FunASR 模型"语义错误；padding/诊断按 funasr 家族走会打架 |

### 4.3 前置 spike（0.5 天，结论回填本文档后再实装）

写一个 `examples/` 临时程序（不合入）：用 binding 对下载好的包跑 `test_wavs/cantonese.wav`、`raokouling.wav`、`transcript.txt` 对照。验证四件事：
1. 锁定版原生库能加载 qwen3（字段存在 ≠ 原生支持，1.13.7 预计通过，实测兜底）；
2. 输出文本格式：有无 `<|...|>` 标签、标点风格、是否需要后处理（决定 `AsrResult` 的 language 字段是否启用启发式 LID——建议启用，与 nano 同款）；
3. 实机 RTF/内存复核（官方 0.077–0.168 是他们的机器，本机弱 CPU 下限确认）；
4. `num_threads=1` 与 2 的速度差（决定跟随 sensevoice 的 1 线程还是官方基准 2 线程）。

### 4.4 改造清单（按 B-α）

| # | 文件 | 改动 |
|---|---|---|
| B1 | `crates/lt-models/src/registry.rs` | 新增 `QWEN3_ASR: ModelEntry { key: "qwen3-asr-0.6b", display: "Qwen3-ASR-0.6B", hf: Some("csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25"), ms: None, always_hf: true, estimated_bytes: 985_000_000, files: &["conv_frontend.onnx", "encoder.int8.onnx", "decoder.int8.onnx", "tokenizer/merges.txt", "tokenizer/tokenizer_config.json", "tokenizer/vocab.json"] }` + `qwen3_entry(key)`；单测同款 |
| B2 | `crates/lt-proto/src/settings.rs:15` | `ASR_ENGINES: [&str; 3] = ["funasr", "whisper", "qwen3"]`；sanitize 自动放行（:164 用的是该常量）；无需新 legacy 别名 |
| B3 | `crates/lt-models/src/cache.rs:166-172,187-227` | `is_asr_cached` / `missing_models` 加 `"qwen3"` 分支（清单全在 + 总量过阈，复用 A2 的完整性函数；显示名 "Qwen3-ASR-0.6B"） |
| B4 | `crates/lt-app/src/backend.rs:104,176` | `missing_models(...)` 调用处与引擎切换处透传 `"qwen3"`（现有 match 加一臂；`SwitchEngine` 命令已含 engine 字符串，proto 无需扩——确认 `Cmd::SwitchEngine` 载荷即可） |
| B5 | `crates/lt-app/src/pipeline.rs:582-626` | `build_worker_config` 加 `"qwen3"` 臂：`engine:"qwen3"`、`options{model_dir}`、`pad_seconds: None`；启动诊断分支同步 |
| B6 | `crates/lt-asr/src/engines/qwen3.rs`（新建） | `Qwen3AsrEngine::load(model_dir, )`：`qwen3_asr { conv_frontend, encoder, decoder, tokenizer: Some(model_dir.join("tokenizer")), max_new_tokens: 512, temperature: 1e-6, top_p: 0.8, seed: 42 }`；`set_language` → `Unsupported`（config 无 language 字段，纯 auto-LID）；`transcribe` 后处理按 spike 结论（预期：trim + 空→None + 启发式 LID 填 language）；`set_input_padding` → 默认 Unsupported |
| B7 | `crates/lt-app/src/main.rs:105` | `asr_worker_entry` 加 `"qwen3"` 分支 |
| B8 | `crates/lt-asr/src/manager.rs:81-87` | `engine_family` 加 `"qwen3" → "qwen3"`（padding 挂起键无 UI 来源 → 天然 no-op，语义显式化） |
| B9 | `crates/lt-ui/src/windows/panel/vad.rs:26-30` | `ENGINES` 尾部追加 `("qwen3", "engine_display_qwen3", "Qwen3-ASR")`；pad 组可见性逻辑确认对 qwen3 隐藏（现逻辑按引擎家族显隐，需过一遍） |
| B10 | `assets/i18n/zh.yaml` + `en.yaml` | `engine_display_qwen3`（"Qwen3-ASR（多语种，实验性）"）+ 模型说明 hint 键；两 yaml 同步 |
| B11 | 文档回写 | RESEARCH.md 新偏差记录（建议编号顺延，如 D-17：新增 Qwen3-ASR-0.6B 引擎，动机/范围/不支持项：无语言指定、无热词 UI、不接 1.7B）；AGENTS.md 待办 |
| B12 | 验收 | spike 四项结论确认 → 下载 941MB → 引擎切换/回滚链路 → 中/粤/英/日实音对照 test_wavs 转写 → 实机 5 分钟会议音频 → CPU/内存采样 → `cargo test --workspace` 全绿 |

### 4.5 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| 原生库版本与 qwen3 支持的匹配 | 低（spike 兜底） | spike 首项验证；不过则升 `sherpa-onnx` crate 补丁版本（`"1.13"` 语义内） |
| 无 language 指定（原版 asr_language 下拉对其无效） | 中 | auto-LID 即设计行为；UI 语言下拉对 qwen3 置灰或提示"自动检测"（i18n hint）；`AsrResult.language` 用启发式兜底 |
| 上游 #3509（hotwords/采样参数扰动输出） | 低 | 不暴露 hotwords；采样参数固定官方默认；seed 固定 42（确定性） |
| decoder 721MB → 引擎切换体感变慢（数秒加载） | 低 | 与 whisper large 同级体验；文档/UI hint 提示 |
| 1.7B 诱惑 | — | 明确不接：官方无支持（#3535 open）+ CPU AR 解码成本翻三倍 |

---

## 5. WP-C：缓存完整性硬化（并入 WP-A 必做，非独立排期）

统一规则（新增模型族防御，不动原版单包模型的行为对齐）：**`entry.files` 全部存在 && 总字节 ≥ 阈值** 才算缓存完整。

- `is_funasr_cached`（funasr 族三模型全走此函数）：加"清单全在"。sensevoice files = 2 件，语义兼容；mlt 恒 None 不涉及。
- qwen3 走新分支但复用同一完整性函数。
- whisper 已有"半体积"规则，不动。
- `estimated_bytes` 校准：nano 1_010_000_000（实测 964MB 清单 + 余量）；qwen3 985_000_000（实测 941MB 清单 + 余量）。进度条与半体积阈值（whisper 用）依赖此值，funasr 族仅用于进度估计。
- 单测：清单缺一件（即使总量过阈）→ 未缓存；全在且过阈 → 缓存。

---

## 6. 近两年开源 ASR 模型扫描（2024-09 ～ 2026-09）

筛选口径：对本项目（**实时音频翻译、Windows、纯 CPU、zh 为核心、单 exe 本地分发**）有实际意义。运行时路径一栏是决定能否低成本接入的关键。

### 6.1 全景表

| 模型 | 发布 | 规模/架构 | 中文 | 许可 | 本项目运行时路径 | CPU 实时性 | 判定 |
|---|---|---|---|---|---|---|---|
| SenseVoice-Small | 2024-07 | 234M 非流式 CTC+AED | ✅ 5 语 | 模型卡自定 | **已接（默认）** | RTF<0.1 级 | 已在役 |
| Whisper large-v3-turbo | 2024-10 | 809M decoder 剪枝 | ✅ | MIT | **已接（D-15 turbo 档）** | 中 | 已在役 |
| Paraformer（大/ сеveral 变体） | 2023 | CIF 非自回归 | ✅ | 模型卡 | sherpa `paraformer` 字段现成 | 快 | **不接**：能力被 SenseVoice 覆盖，无增量 |
| Moonshine tiny/base（v2） | 2024-10 / 2025 迭代 | 27M/61M CTC+解码器 | ❌ 仅英 | MIT | sherpa `moonshine` 现成 | 极快 | **不接**：无中文，场景不符 |
| Kyutai STT（kyutai-labs） | 2025 | 1B/2.6B streaming | ❌ en/fr | CC-BY-4.0 | 自带 Rust(candle) runtime，无 sherpa/ONNX | 快（流式） | **不接**：语言不符 + 引入第二条重型推理栈 |
| NVIDIA Canary-Qwen-2.5B / Canary-1B-v2 | 2025-02 / 2025 | 2.5B/1B AR 翻译型 | ❌（v2 为 25 种欧语） | CC-BY/非商用混合 | sherpa `canary` 现成 | 慢-中 | **不接**：无中文 |
| NVIDIA Parakeet-tdt-0.6b-v2/v3 | 2025 | 0.6B TDT(RNNT+CTC) | ❌（v3 25 欧语） | CC-BY-4.0 | sherpa nemo 系现成 | 极快（英文榜速度王） | **不接**：无中文 |
| Voxtral Mini 3B / Small 24B（Mistral） | 2025-07 | 3B/24B 音频+LLM | ❌（8 语无 zh） | Apache 2.0 | 需 vLLM/mistral.rs 系，无 ONNX/CPU 路径 | — | **不接**：无中文 + CPU 不可行（虽 AA-WER 榜首） |
| FireRedASR-AED-L | 2025-01 | 1.1B AED | ✅ zh 顶级 | Apache-2.0（代码）/模型卡 | sherpa 官方包（2025-02 起），另 v2 有 CTC 版（快） | AED 慢（RTF≈1），v2-CTC 中 | **观察名单①**：zh 质量备选，接法≈WP-A 复用，SenseVoice/qwen3 实测不满意时启用 |
| Dolphin（Dataocean） | 2025-02 | CTC 多语 40 语 | ✅（含 yue） | Apache-2.0 | sherpa 官方包（2025-04） | 快 | 观察：zh 精度预期逊于 qwen3，暂无接的必要 |
| Fun-ASR-Nano-2512 | 2025-12 | 0.8B LLM 解码 | ✅ 31 语+方言 | MIT 代码 | **WP-A** | RTF 0.15 | 本文档主体 |
| Fun-ASR-MLT-Nano-2512 | 2025-12 | 同上（多语变体） | ✅ | MIT 代码 | 官方无转换；社区包 `lorneluo/sherpa-onnx-funasr-mlt-nano-int8-2512` 实测存在（文件布局同官方 nano） | 同 nano | **维持 D-14 置灰**；解锁路径见 6.3 |
| Kimi-Audio 7B | 2025-04 | 7B 音基座 | ✅ | MIT | 无 CPU 路径 | — | **不接**：CPU 不可行 |
| Step-Audio 2 等 LLM 音频大模型 | 2025 | ≥10B | ✅ | 各异 | 无 CPU 路径 | — | **不接** |
| Phi-4-multimodal | 2025-02 | 5.6B | ✅ | MIT | 理论可 ORT 自拼管线，无 sherpa；工程量大 CPU 慢 | 慢 | **不接**：投入产出比差 |
| Meta Omnilingual ASR（CTC 300M/700M/7B） | 2025-11 | CTC wav2vec 系 | ✅（100 训练语含 zh） | 各尺寸开放 | sherpa `omnilingual` 字段已现（1.13.7） | CTC 快 | **不接（暂）**：1600+ 语的"长尾多语"卖点本项目用不上；若未来接小语种需求再评估 |
| **Qwen3-ASR-0.6B / 1.7B** | **2026-01** | 0.6B/1.7B LLM 解码 | ✅ 30 语+22 方言 | **Apache 2.0** | sherpa 官方（仅 0.6B）+ binding 现成 | **RTF 0.08–0.17** | **WP-B** |
| **Cohere Transcribe** | **2026-03** | 2B 音频→文本 | ✅（14 语含 zh/ja/ko/vi） | **Apache 2.0** | sherpa 1.13.7 已有 `cohere_transcribe` 字段（转换包成熟度待查） | 待实测（2B AR，预计介于 nano 与 1.7B 之间） | **观察名单②**：官方 sherpa 转换包与 RTF 数据出现后，作为 qwen3 的质量对照组评估 |
| Granite Speech（IBM） | 2024-2025 | 2B/8B | ❌ en+5 | Apache 2.0 | 无 sherpa | — | **不接** |

### 6.2 扫描总结论

1. 近两年"效果最好"的一档（Voxtral Small、Canary-Qwen、Kimi-Audio 等）几乎全是 **LLM 架构 + 多 B 参数**，其精度优势以 GPU/vLLM 运行时为前提，与本项目"纯 CPU + 单 exe"硬约束正面冲突——**CPU 约束下，0.6B 级 LLM 解码模型（nano / qwen3-0.6B）就是质量上限，且两者恰好都有官方 sherpa-onnx 转换**。这个结论反过来印证了 WP-A/WP-B 就是当前技术约束下的最优集合。
2. zh 场景的现有组合（SenseVoice 秒级 + whisper turbo/medium 质量档 + nano/qwen3 新档）在速度/质量/语言覆盖三维上已无空洞。
3. 值得持续盯的只有两个：**Cohere Transcribe**（sherpa 字段已预留，等官方转换包 + CPU RTF 数据）与 **FireRedASR2-CTC**（zh 质量备选，接法零新增模式）。触发条件写入 RESEARCH.md 即可，不需要现在排期。

### 6.3 MLT 的有条件解锁路径（供决策，默认不动）

社区包 `lorneluo/sherpa-onnx-funasr-mlt-nano-int8-2512`（HF 实测存在，六文件布局与官方 nano 完全一致）使 MLT 在技术上可通过"换一个 hf repo id"直接复用 WP-A 的引擎。但：非官方转换、无 ground-truth 对照、维护者不可考。**建议**：维持 D-14 置灰；若用户明确想要，验收标准 = 用该包跑 sherpa 官方 nano 页同款 25 条 test_wavs 对照转写，质量接近官方 nano 包再解禁（届时 `funasr_entry("funasr-mlt-nano-2512")` 返回指向该 repo 的条目即可，UI 取消灰显一行）。

---

## 7. 排期与边界

```
WP-A  nano 实装 + 注册表修正 + WP-C 缓存硬化      0.5–1 天   ← 消除既存用户陷阱，优先
WP-B  spike（qwen3 加载/格式/RTF 验证）            0.5 天    ← 结论回填本文档
WP-B  qwen3 实装（按 B-α 拓扑）                    1–1.5 天
─── 合计 ≈ 2.5–3.5 天；每步收尾 cargo test --workspace 全绿 + 自主中文 commit
```

**明确不做**（记录理由，防返工）：Qwen3-ASR-1.7B（无官方支持）、hotwords UI（上游 #3509）、Parakeet/Canary/Voxtral/Moonshine/Kimi-Audio/Phi-4-MM（语言或运行时不符）、MLT 默认解禁（社区包质量未验）。

**不改的事**：默认引擎仍是 SenseVoice；`funasr_model` 3 项 1:1 结构不动；whisper 双栈架构不动；mlt 置灰不动。

---

## 8. 参考资料

- sherpa-onnx FunASR-Nano 模型页：https://k2-fsa.github.io/sherpa/onnx/funasr-nano/pretrained.html
- sherpa-onnx Qwen3-ASR 文档：https://k2-fsa.github.io/sherpa/onnx/qwen3-asr/index.html （pretrained 子页含 RTF 表）
- Qwen3-ASR 官方开源：https://github.com/QwenLM/Qwen3-ASR ｜ https://qwen.ai/blog?id=qwen3asr ｜ 技术报告 arXiv:2601.21337 ｜ HF https://huggingface.co/Qwen/Qwen3-ASR-1.7B
- 转换包（本文实测）：https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30 ｜ https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25
- ONNX 导出脚本：https://github.com/Wasser1462/FunASR-nano-onnx ｜ https://github.com/Wasser1462/Qwen3-ASR-onnx
- FunASR/Fun-ASR 上游：https://github.com/modelscope/FunASR ｜ https://github.com/QwenAudio/Fun-ASR ｜ https://modelscope.cn/models/FunAudioLLM/fun-asr-nano-2512
- 上游 issue：#3535（1.7B 支持请求）｜ #3509（qwen3 hotwords/语言参数扰动）｜ #3061（换皮包甄别）｜ #3066（int8 质量）
- Rust binding API：https://docs.rs/sherpa-onnx/1.13.7 （`OfflineFunASRNanoModelConfig` / `OfflineQwen3ASRModelConfig` / `OfflineModelConfig`）
- 纯 Rust qwen3 推理（备选路线，未采用）：https://lib.rs/crates/qwen-asr （BF16 mmap，无 ONNX；sherpa 路线已够用，留档）
- 横向评测：HF Open ASR Leaderboard https://huggingface.co/spaces/hf-audio/open_asr_leaderboard ｜ MarkTechPost 2026-07 横评 ｜ Cohere Transcribe 发布 https://cohere.com/blog/transcribe ｜ Meta Omnilingual https://github.com/facebookresearch/omnilingual-asr
