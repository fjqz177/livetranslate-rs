# ASR 引擎扩展可行性分析与施工方案（FunASR Nano / Qwen3-ASR / 开源模型扫描）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> 状态：**阶段二活跃文档**。**r2（2026-09-08）WP-A 动工方案实测定稿**：HF 目标仓全量枚举定案、ModelScope 全量核查（无官方源）、hf-mirror 经产品下载器实测 20.4 MB/s、DL-1~6 后代码面逐行重验、**D-24 诚实下载源决策**登记。WP-A 按本文 §3 施工；WP-B（Qwen3-ASR）调研维持，实装新偏差自 **D-25** 起（D-24 已被下载源决策占用，AGENTS.md 原注记「WP-B 自 D-24 起」顺延）。
>
> **r2.1（2026-09-08）D-24 语义修订（用户裁决）**：HF 端点**不设全局默认镜像、不新增 `hf_endpoint` 设置键**——用户选 HF 源 = 官方 `huggingface.co` 直连；用户选 MS 源而 HF 成为实际下载路径时（该模型无 MS 源的直接回退，或 MS 尝试失败后的回落）= **自动经 hf-mirror.com**。端点由所选 hub 决定，实化为 lt-models 常量 + `hf_endpoint_for(selected)`（§3.2 A9/A10）。
>
> **r2.2（2026-09-08）：✗ WP-A 施工完成。** A1~A12 全部落地，341 测全绿（21 套件），新码 clippy 零告警；引擎级验收与 GUI 端到端冒烟数据见 §3.3 末「验收结果」。
>
> **r3（2026-09-08）：WP-B 动工方案实测定稿。** 目标仓 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25` 经 HF API 实测（六文件 987,015,347B + LFS sha256，lastModified 2026-04-07）；binding 1.13.7 `OfflineQwen3ASRModelConfig` 逐字段核对（含 Default 陷阱）；官方文档页参数基线与输出格式取证；v1.13.7（2026-09-01 发布）同 release 自带 `qwen3-asr-c-api.c`——r1 spike 首项「binding 有字段 ≠ 原生支持」风险**基本排除**（spike 保留为验收程序）；设置拓扑 **B-α 经用户拍板生效**（AGENTS.md 2026-09-08）；完整施工清单见 §4（r3 重写）。
>
> r1（2026-09-07）：初版调研。其 §2.4「funasr 缓存 50MB 单文件误判」漏洞与 WP-C 硬化项已被 DL-1（D-22 manifest 探测）消化，r2 正式取消；r1 §1.4 所称「设置里已有 hf-mirror endpoint 覆写」经核仅指 lt-models 机制，**应用层未接线**（本 r2 §3-A10 补齐）。

---

## 0. 结论速览（TL;DR，r3 更新）

| 议题 | 判定 | 一句话依据 |
|---|---|---|
| **FunASR Nano（funasr-nano-2512）实装** | **可行，turnkey，按 §3 立即施工（WP-A）** | 目标仓定案 `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`（HF 全量枚举 55+ 仓后唯一正确解，六文件 2026-01-07 版实测合计 1,009,605,061B）；锁定的 sherpa-onnx 1.13.7 binding 与原生库同 release 均含 `OfflineFunASRNanoModelConfig`；缺口仅注册表修正 + 引擎实现 + 两处分派 |
| **无真实 MS 源模型的下载语义** | **D-24（2026-09-08 用户裁决，r2.1 修订）：诚实回退，不伪造** | 注册表 `ms` 字段只填真实存在的 ModelScope 仓，无则 `None`；`always_hf` 语义 =「仅 HF 源」，所选 hub=ms 时链路**直达 HF 且自动经 hf-mirror 镜像**（选 hub=hf 则官方直连——端点由所选 hub 决定，不新增设置键），禁止把非 MS 链接塞进 ms 字段或做特殊标记（现注册表错误条目即反例）；`hub_chain` 现有实现已满足，需补端点映射常量 + backend 一行接线 + UI 诚实提示 |
| **hf-mirror 可用性** | **实测通过，镜像备选路线全部封存** | 产品下载器全链路（hub_chain→直链→206 续传→manifest 校验）963MB 用时 47.3s、均速 20.4 MB/s，六文件与 HF API 逐字节一致；ModelScope 侧无官方 nano 源，zengshuishui 等社区镜像不采信 |
| **缓存完整性硬化（原 WP-C）** | **取消——已被 DL-1 消化** | `cache.rs:38-42 dir_has_manifest` 逐文件「存在+达下限」对多文件模型天然成立，注册表填对清单即得，无需新代码 |
| **Qwen3-ASR-0.6B 接入** | **✗ 可行且已实装（WP-B，2026-09-08，r3.1）** | 目标仓 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`（HF 实测六文件 987,015,347B + sha256 逐字节校验）；B1~B12 全落地，349 测全绿；S0 实测：加载 2.9s、线程定 3（RTF 0.459/0.312/0.280@1/2/3）、本机 RTF 0.23–0.36（≥3× 实时）、五语样例 LID 全对；GUI 冒烟 worker ready 零告警；**无 language 参数（纯 auto-LID）**；D-25 登记 |
| Qwen3-ASR-1.7B | **不可行，不接（r3 复核维持）** | sherpa-onnx 官方未支持（#3535 2026-09-08 仍 open）；HF 社区已有自转换 1.7B 包（ilmina/thieunv 等）但非官方不采信；纯 CPU AR 解码实时性无保障 |
| funasr-mlt-nano-2512 | **维持置灰（D-14，不变）** | 无官方转换；MS 亦无源（r2 实测）；社区包质量未验证 |

**施工顺序**：✗ WP-A（nano 实装 + 注册表修正 + D-24 接线，已完工）→ WP-B spike（0.5 天，降级为验收程序）→ WP-B 实装（1–1.5 天，**拓扑 B-α 已拍板**，见 §4）。

---

## 1. 现状盘点（r2 代码面重验；行号 = 2026-09-08 现场）

### 1.1 引擎架构（双栈，均已上线，r1 内容维持有效）

```
Settings { asr_engine: "funasr"|"whisper", funasr_model, whisper_model_size, hub, ... }
    │
    ▼  pipeline.rs::build_worker_config (crates/lt-app/src/pipeline.rs:648)
WorkerConfig { engine: "sensevoice"|"whisper", display_name, language, pad_seconds, options{model_dir|model_path} }
    │  spawn: 当前 exe --asr-worker <config-json>
    ▼  main.rs::asr_worker_entry (crates/lt-app/src/main.rs:108) —— match engine 分发
    ├─ "sensevoice" → lt_asr::sensevoice::SenseVoiceEngine   （sherpa-onnx OfflineRecognizer）
    └─ "whisper"    → lt_asr::WhisperEngine                  （whisper-rs / whisper.cpp）
    ▼  stdin/stdout 帧协议（transcribe / set_language / set_input_padding / shutdown）
AsrManager（crates/lt-asr/src/manager.rs）：重启≤3次、错误分类、RSS 回收（基线+2048MB）、
    引擎切换失败回滚旧 worker、padding 按引擎家族挂起（engine_family: "sensevoice"|"nano"→"funasr"）
```

- 推理依赖：`sherpa-onnx = "1.13"`（Cargo.lock 实锁 **1.13.7**）；`whisper-rs = "0.16"`。纯 CPU 硬约束两者均满足。
- **r2 补强结论**：sherpa-onnx-sys 1.13.7 的 FFI 头（含 `OfflineFunASRNanoModelConfig`）与原生静态库出自**同一 release**（build.rs 按 crate 版本拉 `sherpa-onnx-v1.13.7-win-x64-static-MT-Release-lib.tar.bz2`）——r1 遗留的「binding 有字段 ≠ 原生支持」风险**直接排除，nano 无需 spike**。
- AsrEngine trait（`crates/lt-asr/src/engine.rs`）：`transcribe(audio, word_timestamps)` + `set_language` + `set_input_padding`（后两者默认 `Unsupported`）。
- 客户端超时预算（`crates/lt-asr/src/client.rs:125-127`）：ready 180s / 单次 60s / shutdown 5s。对 nano（963MB 包、573MB 级 llm 加载）充裕。

### 1.2 FunASR Nano 现状：占位已成、实装未做（r2 逐行核实；各 ❌ 已随 WP-A 消除，WP-B 基线见 §1.6）

| 层 | 状态 | 位置（r2 行号） |
|---|---|---|
| proto 契约 | ✅ `FUNASR_MODELS` 3 键含 nano，sanitize 放行 | `crates/lt-proto/src/settings.rs:19` |
| 模型注册表 | ❌ **条目内容错误**（见 1.4） | `crates/lt-models/src/registry.rs:44-53` |
| 缓存探测/下载 | ✅ DL-1 manifest 通用机制，**填对清单即得**（原 WP-C 取消） | `crates/lt-models/src/cache.rs:38-42,97-107` |
| hub 回落 | ✅ `hub_chain`：`always_hf+ms=None` → 纯 HF 链（D-24 语义已满足） | `crates/lt-models/src/download/mod.rs:28-48` |
| 下载器子目录 | ✅ `try_download` 对 `target.parent()` create_dir_all，`Qwen3-0.6B/*` 天然支持（r2 实测下载通过） | `crates/lt-models/src/download/mod.rs:342-344` |
| **HF 镜像端点接线** | ❌ **应用层缺口**：`with_hf_endpoint` 仅存在于 lt-models 与测试，lt-app/lt-ui 零调用——真实 app 的 HF 下载直连 `huggingface.co`，仅代理三模式兜底 | `crates/lt-app/src/backend.rs:187` |
| 装配分派 | ❌ funasr 分支一律映射 `engine:"sensevoice"`（nano 必坏点） | `crates/lt-app/src/pipeline.rs:657-671` |
| 引擎实现 | ❌ 无 nano 引擎（`engines/` 仅 `whisper.rs`，sensevoice 在 `src/sensevoice.rs`） | `crates/lt-asr/src/engines/mod.rs` |
| worker 分发 | ❌ `asr_worker_entry` 只有 "sensevoice"/"whisper" 两臂 | `crates/lt-app/src/main.rs:110-139` |
| manager 层 | ✅ **已预留**：`engine_family` 认得 `"nano"`（:81-83）；nano 不下发 padding（:304-312） | `crates/lt-asr/src/manager.rs` |
| UI 下拉 | ✅ nano 可选带「（实验性）」（:62-73）；mlt 灰显（:71）；⚠️ SenseVoice padding 滑杆对 nano 仍显示（:390-396，无效果，A8 处理） | `crates/lt-ui/src/windows/panel/vad.rs` |
| 下载向导/设置镜像 | ✅ 走 `missing_models` 通用链路；`StartDownload` 现场重算 | `crates/lt-app/src/backend.rs:250-261` |

**既存陷阱（WP-2 预警的现状行为，r2 修正严重度评估）**：用户选 nano → 触发下载 → 清单里第一个文件 `model.int8.onnx` 在真实 HF 仓 404 → **DL-2 快速失败立即报错**（r1「白下 1.1GB 后必坏」的描述是 DL-2 之前的行为；现状是「点了就报错」，不浪费流量但仍不可用）。WP-A 实装后自然消除。

### 1.3 原版 Python 的 nano 行为（1:1 参照，`LiveTranslate/asr_funasr_nano.py`，r2 已复读核对）

- 后处理（:99-118）：取 `text` → 正则清除全部 `<|...|>` 标签 → trim → 空文本或 `"sil"` → 视为无结果。
- 语言判定（:16-19, :117-135）：`set_language` 存值，`"auto"` 存 `None`；有值用设值，否则启发式——假名(3040–30FF/31F0–31FF)>0 → ja；谚文(AC00–D7AF)>30% → ko；汉字(4E00–9FFF)>30% → zh；否则 en；空 → auto。
- `supports_padding=false`（nano 无 padding 语义）、`supports_language=true`（创建期参数）。
- ONNX 转换包权重已内联 Qwen3-0.6B，原版单独下载 Qwen 权重的链路**确认不需要**（r1 已核实，D-14 维持）。

### 1.4 注册表 nano 条目错误（WP-A 必修项；亦是 D-24 反例教材）

`registry.rs:44-53` 现状四错：
```rust
hf: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),   // ① 仓库不存在（HF API 401，2026-09-08 两轮实测）
ms: Some("csukuangfj/sherpa-onnx-funasr-nano-2512-zh-cantonese-en-ja-ko"),   // ② MS 上也无——把不存在的仓塞进 ms 字段，
                                                                              //    恰是 D-24 明令禁止的「伪 MS 源」反例
files: &["model.int8.onnx", "tokens.txt"],                                    // ③ 清单是 sensevoice 的，nano 真包六件套（§2.2）
always_hf: false,                                                             // ④ MS 无源却允许 MS 优先
```
> r1 注记补充：当时预估的仓名 `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30` 是对的，注册表是后来误写。另 HF 上确有「换皮包」`csukuangfj/sherpa-onnx-sense-voice-funasr-nano-2025-12-17`（`-int8-` 变体同在，r2 API 实测存在，siblings 为 `model.onnx + tokens.txt` 的 SenseVoice 形状，即 #3061 警告对象），**不得**作为 nano 源。

### 1.5 DL-1~6 后的下载链路面貌（r2 新增盘点）

- 探测：`is_funasr_cached` = 双 hub 各自 manifest 齐全即命中（或语义）；`local_model_dir` MS→HF 优先级；快照取字典序最后且含完整 manifest（E-10）。
- 下载：`backend.rs run_download`（:149-246）会话化线程 + 取消（D-23）+ 泵 drain（F9）；`hub_chain(hub, hub_hf, hub_ms, always_hf)` 生成尝试链，`Downloader::download_model` 逐链尝试（DL-5）；仅 404/网络类错误回落。
- **端点现状**：`Downloader::new(dl_dir, dl_proxy_mode)`（backend.rs:187）**未调 `with_hf_endpoint`** → HF 下载直连官方。国内无代理场景基本不可用（r2 实测 hf-mirror 20.4 MB/s，见 §2.7）——A10 补此缺口。（存档注记，r3：A10 已随 WP-A 接线，现场 backend.rs:188。）

### 1.6 WP-B 相关现状（r3 新增，行号 = 2026-09-08 WP-A 完工后现场）

§1.2 表中各 ❌ 已随 WP-A 消除（注册表已修正、nano 引擎在位、分派/worker 臂已加、hf_endpoint 已接线）；本节为 WP-B 施工基线：

| 层 | 现状（qwen3 视角） | 位置 |
|---|---|---|
| proto 契约 | `ASR_ENGINES: [&str; 2]`（funasr/whisper），sanitize :171 引用该常量——扩三值即自动放行；`Cmd::SwitchEngine` 已含 engine 自由字符串，**无需扩契约** | `lt-proto/settings.rs:17`；`lt-app/backend.rs:77-90` |
| WorkerConfig | 定义在 lt-asr（非冻结的 lt-proto），engine 为自由字符串 | `lt-asr/worker.rs:18` |
| 模型注册表 | 无 qwen3 条目（B1 新增）；`is_funasr_cached`/`local_model_dir` 泛化于 ModelEntry，qwen3 复用**零改动** | `lt-models/registry.rs`；`cache.rs:97,220` |
| 缓存探测 | `is_asr_cached`/`missing_models` 各只有 funasr/whisper 两臂（B3 加 "qwen3"） | `lt-models/cache.rs:146-152,171-215` |
| 装配分派 | `build_worker_config` 已按 entry.key 分派 nano/sensevoice（B4 加 "qwen3" 臂）；启动诊断 :736 对非 funasr 引擎取 SENSEVOICE 条目、TlSwitch 日志 model_key :852/:868 对非 whisper 打 funasr_model——qwen3 会打错名，同改 | `lt-app/pipeline.rs:648-699,736-750,852-856,868-875` |
| 引擎实现 | `engines/` 有 nano.rs/whisper.rs（B5 加 qwen3.rs）；guess_language/标签清理目前是 NanoEngine 关联函数（提升共享，见 B5） | `lt-asr/engines/mod.rs` |
| worker 分发 | `asr_worker_entry` 三臂 sensevoice(:111)/nano(:128)/whisper(:146)（B7 加 "qwen3"） | `lt-app/main.rs:108-159` |
| manager 层 | `engine_family` 映射 sensevoice\|nano→"funasr"、whisper→"whisper"（B8 加 "qwen3"→"qwen3"）；padding 挂起按家族取键且 UI 永不发 `SetPadding{engine:"qwen3"}` → apply_pending 天然 no-op，**零逻辑改动** | `lt-asr/manager.rs:81-83,296-312` |
| UI | `ENGINES` 双值表（B9 扩三值）；`model_cache_status` 双臂（B9 加）；语言下拉恒启用（qwen3 需 auto-only hint）；D-24 nano hint 条件 :429-434 需扩 qwen3；funasr 模型下拉/whisper 档位/双 padding 滑杆显隐已按引擎隔离，qwen3 天然全隐藏 | `lt-ui/windows/panel/vad.rs:24-28,195-231,300-315,429-434` |
| 下载链路 | **零改动**：注册表 `ms=None+always_hf=true` → `hub_chain` 纯 HF 链 + `hf_endpoint_for(hub)` 已接线（backend.rs:188，A10）；`current_missing` :251 已透传 asr_engine | `lt-app/backend.rs` |

---

## 2. 关键事实核查（r1 2026-09-07 / r2 2026-09-08 实测）

### 2.1 Rust binding 能力面：不需要升依赖

锁定的 `sherpa-onnx 1.13.7`（本地 crate 源码逐字段核对）：

```rust
pub struct OfflineFunASRNanoModelConfig {   // sherpa-onnx-1.13.7/src/offline_asr.rs:393-407
    encoder_adaptor, llm, embedding: Option<String>,   // 三个 onnx 文件
    tokenizer: Option<String>,                          // ★ 目录（Qwen3-0.6B/）
    system_prompt, user_prompt: Option<String>,
    max_new_tokens: i32, temperature: f32, top_p: f32, seed: i32,
    language: Option<String>,                           // ★ 创建期语言参数
    itn: i32, hotwords: Option<String>,
}
```
⚠️ binding 的 `Default` 不能裸用（`max_new_tokens=0, temperature=1.0, top_p=1.0, seed=0`）——**必须显式设官方推荐值**（§3-A3）。`OfflineRecognizerConfig.decoding_method` 维持框架默认 greedy_search（sensevoice 同）。

### 2.2 Fun-ASR-Nano 官方 int8 转换包（r2 实测定案）

**目标仓 = `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`**（作者 = k2-fsa/sherpa-onnx 维护者本人；官方文档页下载指引与 C API 示例 `funasr-nano-c-api.c` 均指向此包；2026-01-07 更新，r2 实测文件比官方文档页略大——以本表为准）：

| 文件 | 实测字节（2026-09-08 HF API） | 注册表下限拟值 |
|---|---|---|
| `embedding.int8.onnx` | 155,584,380 | 100,000,000 |
| `encoder_adaptor.int8.onnx` | 237,792,748 | 150,000,000 |
| `llm.int8.onnx` | 600,356,593 | 300,000,000 |
| `Qwen3-0.6B/merges.txt` | 1,671,853 | 1,000,000 |
| `Qwen3-0.6B/tokenizer.json` | 11,422,654 | 5,000,000 |
| `Qwen3-0.6B/vocab.json` | 2,776,833 | 1,000,000 |
| **合计** | **1,009,605,061 ≈ 963 MB** | `estimated_bytes = 1_050_000_000` |

- 下限刻意取远低于实测（假阴性=多下一次可自愈；假阳性=判已缓存却加载失败）；满足既有不变量测试 `files_min_bytes[0] * 2 < estimated_bytes`（registry.rs:190-192）。
- 仓内 `test_wavs/` 24 条样例（含粤语/方言）→ 验收可免麦克风。
- 官方推荐运行参数（文档页 CLI dump）：`system_prompt="You are a helpful assistant."`、`user_prompt="语音转写："`、`max_new_tokens=512`、`temperature=1e-06`、`top_p=0.8`、`seed=42`、`num_threads=2`、greedy_search。
- CPU RTF（官方页 20 条样例，int8/2线程/greedy）：**0.144–0.191**（≈5–7 倍实时）。
- fp16（1.5GB）/ fp32（3.7GB）官方档存在（`…-fp16-2025-12-30` / `…-2025-12-30`）但体积/内存不满足「纯 CPU + 单 exe + 按需下载」约束，**不用**。
- 已知上游风险：#3066（int8 重复文本）在该包官方样例未复现；方言误识较多（预期内）；粤语样例输出含 `<0xB2>` 类字节级 fallback token 疑似（文档页样例流）→ 列入 A12 验收观察点，若复现再补 postprocess 清理（不预做）。

### 2.3 Qwen3-ASR（r3 起以 §4 为准）

0.6B 官方转换包 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`（r3 复测在位，六文件 987,015,347B；**MS 无官方单仓**——r2/r3 两轮实测 csukuangfj2 404；MS 侧存在 zengshuishui/Qwen3-ASR-onnx 社区散文件镜像，非官方不采信）。1.7B 不接（#3535 open）。完整目标仓定案、文件字节数、参数基线与施工清单见 **§4（r3 重写）**；r1 §2.3/§4 与 r2 增补作为沿革保留。

### 2.4 下载器与缓存层约束（r2 重写：r1 的两个硬约束一个已消失、一个已验证）

1. ~~「funasr 缓存完整性只看总量≥50MB，多文件包有漏洞」~~ → **已被 DL-1（D-22）消灭**：`dir_has_manifest` 逐文件「存在 + len ≥ 下限」，对六件套天然严格。**原 WP-C 取消**，注册表填对清单即得防御。
2. 「注册表 files 必须逐字精确（含子目录前缀），列错即 404」→ **维持，且已实测通过**：`/resolve/main/{path}` 直链 + `try_download` 对子目录 `create_dir_all`（download/mod.rs:342-344）；r2 真实下载器六文件全部落盘正确（§2.7）。

### 2.5 ModelScope 全量核查（r2 新增，2026-09-08）：**无官方 nano 源**

| MS 仓库 | 格式 | 判定 |
|---|---|---|
| `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30` | — | **404**，官方未发布 MS 单仓 |
| `fuyuantech/fuyuan-sherpa-onnx-funasr-nano-int8-2025-12-30` | tar.bz2 整包（842MB）+ silero_vad | ❌ 逐文件下载器无解压特性 |
| `zhaochaoqun/sherpa-onnx-asr-models` | tar.bz2 整包集（官方 GitHub Releases 资产转存，含**官方** nano/Qwen3-0.6B 包） | ⚠️ 质量可靠但整包格式不可用；**同仓含换皮假包 `sherpa-onnx-sense-voice-funasr-nano-int8-2025-12-17`，将来引用必须逐字核对条目** |
| `zengshuishui/FunASR-Nano-onnx` | 散文件（六件套齐全，llm 在 `llm_int8/` 子目录） | ⚠️ 唯一逐文件可下的，但系 Wasser1462 社区转换镜像（非官方质检；llm 相对路径与官方包不同）；hf-mirror 实测通过后**不采信**（D-24 备注封存） |
| `manyeyes/Fun-ASR-Nano-2512-LLM-onnx` | 自创布局（decoder.onnx+外部 .data） | ❌ 与 sherpa nano 三件套配置不匹配 |
| `FunAudioLLM/Fun-ASR-Nano-2512` | PyTorch 原版 | ❌ funasr AutoModel 权重，sherpa 引擎不识别 |
| MLT（csukuangfj / lorneluo） | — | ❌ MS 无源（D-14 维持） |
| `zengshuishui/Qwen3-ASR-onnx`（WP-B 情报） | 散文件 | 0.6B int8 三件套在，非官方不采信；qwen3 同走 D-24 |

**结论**：nano/qwen3 注册表 `ms: None` + `always_hf: true`，经 D-24 语义走 HF 镜像，诚实且与实测一致。

### 2.6 HuggingFace 全量枚举定仓（r2 新增）：**目标仓唯一**

HF 搜索 API 枚举 nano 相关 55+ 仓（funasr-nano / fun-asr-nano 双关键词），分类排除：

- ✅ **`csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`**：唯一「官方维护者 + sherpa 官方包布局 + int8 档」三全仓。四层适配全通过：文件↔binding 字段 1:1、下载器直链/子目录实测、manifest 探测口径吻合、test_wavs 可验收。
- ❌ 换皮陷阱：`sherpa-onnx-sense-voice-funasr-nano-2025-12-17`（+int8 变体 + darrenxyli/aoiandroid fork）——SenseVoice 布局，#3061 警告对象。
- ❌ 运行时不兼容：GGUF 系（llama.cpp）、mlx-community 全家（Apple）、vllm 系、`FunAudioLLM/Fun-ASR-Nano-2512(-hf)` PyTorch 原版。
- ❌ 非标准实验品：`funasr-nano-with-ctc`、`yuekai/*-Encoder-ONNX-FP32`、`foryoung365/*-int4-onnx` 等。
- ❌ 第三方转存（aoiandroid/sherpa-onnx-funasr-nano-int8-2025-12-30 等）：与官方同内容但无采信必要。
- 备注：原始转换仓 `Wasser1462/FunASR-nano-onnx` 在 HF 已 401（r2 实测）——反衬官方 csukuangfj 重打包的存续价值。

### 2.7 hf-mirror 实测与产品下载器演练（r2 新增，2026-09-08）

- 机制：`Downloader::with_hf_endpoint`（download/mod.rs:203）+ `hf::file_url = {endpoint}/{repo}/resolve/main/{path}`（hf.rs:5）。
- curl 探速：60MB Range 段 3.79s ≈ **16.6 MB/s**（HTTP 206，Range/断点续传可用）。
- **产品下载器全量演练**（`lt-models` 探针测试 `probe_nano_tmp::probe_nano_download_via_hf_mirror`，`#[ignore]` 常规不跑）：六文件 47.3s 全下完，**均速 20.4 MB/s**，字节与 HF API 逐一致，`dir_has_manifest` 按拟登记清单+下限校验通过，落盘 `~/.config/livetranslate/models/huggingface/hub/models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30/snapshots/main/`。
- **推论**：① 963MB 半分钟级，用户下载体验无压力；② **nano 模型已预置真实缓存**，WP-A 冒烟免下载；③ MS 镜像备选路线（zengshuishui 散文件 / tar.bz2 解压特性）全部封存，除非 hf-mirror 未来不可用再重启评估；④ 顺带覆盖 DL S1「真实网络走查」的 HF 直链+镜像+manifest 段。

---

## 3. WP-A：FunASR Nano 实装（r2 动工定稿；估时约 1 天）

### 3.0 D-24 决策：诚实下载源语义（2026-09-08 用户裁决，登记为阶段二偏差 D-24）

1. **ms 字段只填真实存在的 ModelScope 仓**；模型无 MS 源则 `None`。**禁止**把 HF 链接塞进 ms 字段、禁止占位符/特殊标记等「自欺欺人」手段（现注册表错误条目 1.4-② 即反例）。
2. **`always_hf: true` 语义 =「仅 HF 源」**：所选 hub=ms 时 `hub_chain` 直达 HF（现有实现 download/mod.rs:34 `if always_hf || hub == Hub::Hf` 已满足）；以注册表不变量测试钉死 nano：`always_hf && ms.is_none()`。
3. **HF 端点随所选 hub（r2.1 修订）**：选 HF 源 = 官方 `huggingface.co` 直连（用户显式选 HF 即默认其可直连或已配代理）；选 MS 源则 HF 成为实际下载路径时——含「模型无 MS 源的直接回退」与「MS 尝试失败后的回落」两种情形——**自动经 `hf-mirror.com`**。端点不设用户设置键，实化为 lt-models 常量 + `hf_endpoint_for(selected_hub)`，backend 一行接线（补 1.5 缺口）。国内受益者不止 nano——whisper 六档（今日 always_hf）在默认 hub=ms 下同样受益。
4. **UI 诚实告知**：hub=ms 且选中仅 HF 源模型时，下载区显示「该模型仅提供 HuggingFace 源，将经 HF 镜像下载」（i18n zh/en 同步）。
5. **供应链注记**：第三方镜像 × 无哈希校验 = 信任依赖（distribution.md §6 既有风险）。缓释：镜像仅用于 MS 模式下的 HF 尝试（选 HF 源即官方直连，用户可自主选择）；后续在 registry 渐进登记 sha256（既有备忘，非本 WP）。
6. 编号影响：D-24 被本决策占用，**WP-B 新偏差自 D-25 起**（AGENTS.md 原注记「WP-B 自 D-24 起」顺延）。

### 3.1 判定理由

API 面（binding+原生同版本）、模型面（官方包实测 963MB/RTF 0.15）、工程面（proto/下载/缓存/manager/UI 五层就绪，缺口仅 §1.2 四处）、风险面（默认引擎仍 SenseVoice，nano 出问题不阻塞主路径）——四面全绿。

### 3.2 改造清单（逐文件，行号 = 2026-09-08 现场）

| # | 文件 | 改动 |
|---|---|---|
| A1 | `crates/lt-models/src/registry.rs:44-53` | FUNASR_NANO 修正：`hf=Some("csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30")`、`ms=None`、`always_hf=true`、`estimated_bytes=1_050_000_000`、六文件清单+下限（§2.2 表）。补 `const { assert!(FUNASR_NANO.always_hf) }` 与 `assert!(FUNASR_NANO.ms.is_none())`（D-24 不变量）；同步改 `funasr_entry_keys`/`manifest_min_bytes_parallel_to_files` 断言 |
| A2 | ~~cache.rs 硬化~~ | **取消**（DL-1 已消化，§2.4）；仅确认清单修正后既有 F1 系测试语义兼容（files 变长不破坏 dir_has_manifest 通用逻辑） |
| A3 | `crates/lt-asr/src/engines/nano.rs`（新建） | 仿 sensevoice.rs：`NanoEngine::load(model_dir, language)`——`funasr_nano { encoder_adaptor: join("encoder_adaptor.int8.onnx"), llm: join("llm.int8.onnx"), embedding: join("embedding.int8.onnx"), tokenizer: Some(join("Qwen3-0.6B")), system_prompt: Some("You are a helpful assistant."), user_prompt: Some("语音转写："), max_new_tokens: 512, temperature: 1e-6, top_p: 0.8, seed: 42, language: normalize_language(language), itn: 0, hotwords: None }`，`num_threads=2`（官方基准；AR 解码为瓶颈，与 sensevoice 的 1 不同，冒烟可调）；`transcribe` 无 pad 桶；后处理 1:1 移植原版（§1.3：清 `<|...|>` → trim → 空或 "sil" → None；启发式 LID 假名>0→ja / 谚文>30%→ko / 汉字>30%→zh / else en / 空→auto）填 `AsrResult.language/language_name`；`set_language` 创建期参数 → 变更重建识别器（sensevoice.rs:173-183 策略，"auto"→None）；`set_input_padding` 保持 trait 默认 Unsupported。单测：postprocess 全分支 + LID 边界（30% 阈值恰好/假名优先级）+ load 缺目录报错 |
| A4 | `crates/lt-asr/src/engines/mod.rs` + `src/lib.rs` | `pub mod nano;` + `pub use engines::nano::NanoEngine;` |
| A5 | `crates/lt-app/src/main.rs:110-139` | `asr_worker_entry` 加 `"nano"` 臂：读 `options.model_dir` → `NanoEngine::load(&dir, &cfg.language)`（不消费 pad_seconds——build_worker_config 对 nano 恒 None） |
| A6 | `crates/lt-app/src/pipeline.rs:657-671` | funasr 分支按 `entry.key` 分派：`"sensevoice-small"` → `engine:"sensevoice"` + `pad_seconds: Some(pad_seconds)`（照旧）；`"funasr-nano-2512"` → `engine:"nano"` + `pad_seconds: None`。`resolve_funasr_entry`（:611）不改（mlt/非法键回退语义维持） |
| A7 | `crates/lt-asr/src/manager.rs` | 不改码（:81-83/:304-312 已就位）；补一条集成断言防回归：`engine_family("nano")=="funasr"` 且 nano 引擎名触发 padding 跳过路径 |
| A8 | `crates/lt-ui/src/windows/panel/vad.rs` | ① :390-396 SenseVoice padding 滑杆在 `funasr_model=="funasr-nano-2512"` 时隐藏（原版 nano 无 padding 的 UI 面；manager 已跳过，显示而无效果属误导）；② D-24-④：hub=ms 且选中 nano 时模型/下载区 hint 行 `model_nano_hf_only` |
| A9 | `crates/lt-models/src/download/mod.rs` | 新增常量 `HF_OFFICIAL_ENDPOINT = "https://huggingface.co"`、`HF_MIRROR_ENDPOINT = "https://hf-mirror.com"` 与 `pub fn hf_endpoint_for(selected: Hub) -> &'static str`（r2.1 语义：Hf→官方 / Ms→镜像）；单测两分支映射（url_builders 测试旁） |
| A10 | `crates/lt-app/src/backend.rs:187` + `assets/i18n/zh.yaml`/`en.yaml` | `Downloader::new(dl_dir, dl_proxy_mode).with_hf_endpoint(hf_endpoint_for(hub))`（D-24-③ 接线，弥补 1.5 缺口；`hub` = 本轮下载所选 hub，run_download :158 已有）；i18n 新键两份同步：`model_nano_hf_only`（zh「该模型暂无 ModelScope 源，将自动经 HF 镜像（hf-mirror.com）下载」），挂 vad.rs :410 hub 下拉同区 |
| A11 | `crates/lt-models/src/download/mod.rs`（探针处置） | `probe_nano_tmp` 探针测试保留转正（`#[ignore]` 注记改为「WP-A 验收演练工具」），随本 WP 提交 |
| A12 | 文档回写 | 本文 §0/§1.2 状态勾销；`docs/archive/rewrite-research.md` §1.5 登记 D-24 + WP-2 勾销（处置=实装）；`AGENTS.md` 待办（WP-A ✗ / D-24 / WP-B 自 D-25 起）；实测数据（下载 47.3s/20.4MB/s、加载时长、RTF 本机复测）回填 §2.2 |

### 3.3 验收清单

1. `cargo test --workspace` 全绿（334+ 新增：A1 不变量/A3 引擎单测/A7 回归/A9 sanitize）。
2. 实机（临时 `LIVETRANSLATE_CONFIG_DIR`，settings 显式 `models_dir` 指真实缓存）：
   - 选 nano → 识别页点下载 → **秒完成**（缓存已预置，验证 skip 幂等 + manifest 命中）；
   - 引擎切换成功、悬浮窗 ready；`test_wavs/` 中/英/粤各一条对照转写，语言显示正确；
   - SenseVoice padding 滑杆对 nano 隐藏、日志无 padding 下发（A7 断言呼应）；
   - hub=ms 时 nano 显示「暂无 ModelScope 源，将经 HF 镜像下载」hint；日志核对 URL：hub=hf → `huggingface.co` 直连，hub=ms + nano → `hf-mirror.com`（A9 单测 + 日志观察）；
   - 切回 sensevoice 一切如旧；`cargo run -p lt-app` 冒烟无回归。
3. 观察点（不预做，出现再登记）：粤语 `<0xB2>` 类字节 token；本机 RTF 与加载时长（回填 §2.2）；#3066 类重复文本。

### 3.5 验收结果（r2.2 实测回填，2026-09-08）

- **单测**：`cargo test --workspace` 21 套件全绿（337 测，含新增：registry D-24 不变量 / nano postprocess+LID / manager nano 家族回归 / pipeline nano 分派（稀疏文件造 manifest）/ hf_endpoint_for 映射）；新增代码 clippy 零告警。
- **引擎级**（`probe_real_nano`，真实缓存 963MB 包）：加载 **4.15s**；`rag_physics.wav`→「根据碰撞理论月面样本缺少挥发性物质」（zh，RTF 0.097）；`noise_en.wav`→完整英文句（en，RTF 0.160）；`dia_yue.wav`→粤语口语逐字转写含「啲/佢哋/咗/咧」（zh，RTF 0.181）。**无标签残留 / 无字节 token / 无重复幻觉**——三个观察点均未触发。
- **GUI 端到端冒烟**（临时 config，settings 指真实缓存 + `funasr_model=funasr-nano-2512`）：管道自启 → worker 拉起 → 日志 `ASR worker ready … Fun-ASR-Nano (nano)`，全程零 WARN/ERROR；worker 常驻内存 ≈1.09GB（基线+2048MB 回收阈值天然覆盖）。
- **下载链路**（§2.7 演练）：hf-mirror 全量 47.3s / 20.4 MB/s，manifest 校验通过；本机 RTF 0.097–0.181 优于官方文档页基准（0.144–0.191）口径。

### 3.4 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| int8 AR 解码偶发重复/幻觉（#3066 类） | 中 | 「实验性」标注维持；manager 错误分类+自动重启兜底；高频复现再评估相邻重复折叠（先不做） |
| hf-mirror 第三方信任 × 无哈希校验 | 中 | 仅 MS 模式的 HF 尝试走镜像（选 HF 即官方直连，用户可自主选择）；sha256 渐进登记（既有备忘）；UI 诚实提示 |
| 首次加载慢（600MB llm ORT 初始化） | 低 | ready 180s 充裕；ModelLoadStart 模态已有链路 |
| worker 内存峰值（int8 约 1–1.5GB） | 低 | RSS 回收 = 基线+2048MB 天然覆盖 |
| 大陆直连 HF（用户改回官方端点又无代理） | 低 | 属用户显式选择；代理三模式仍兜底；README 提示 |

---

## 4. WP-B：Qwen3-ASR-0.6B 实装（✗ r3.1 施工完成 2026-09-08；spike 0.5 天 + 实装 1–1.5 天）

> **r3.1（2026-09-08）：✗ WP-B 施工完成。** B1~B12 + S0 全部落地，349 测全绿（21 套件），新码 clippy 零告警；S0 结论与引擎级验收、GUI 冒烟数据见 §4.8。施工顺序：缓存预置（hf-mirror 54s，sha256 校验）→ S0 spike（线程/格式定案）→ B1~B12 代码 → 验收。施工期补漏一处：模型缓存卡片 `model_display` 对 qwen3 误显 whisper 档位名（vad.rs，随 B9 修）。

### 4.8 验收结果（r3.1 实测回填，2026-09-08）

- **单测**：`cargo test --workspace` 21 套件全绿（**349 测**，较 WP-A 净增 8：registry 不变量 / sanitize 放行 / 缓存探测语义（含 tokenizer 子目录缺失） / pipeline qwen3 分派与 model_key / qwen3 引擎 postprocess+set_language+load / manager qwen3 家族 / UI 三值表与共享后处理）；新码 clippy 零告警（存量告警均在旧码）。
- **S0-① 加载**：v1.13.7 原生库加载成功，**2.9s**（release；真实缓存 941MB 包）——「binding 有字段 ≠ 原生支持」风险正式关闭。
- **S0-② 输出格式**：官方样例全部**纯文本、无标签残留**、自动标点；cantonese.wav 与 transcript.txt 参照高度一致；绕口令（raokouling）局部错字属预期难度。
- **S0-③ 本机 RTF/内存**：**0.23–0.36 @3线程**（官方口径 0.077–0.168@2线程为其机型；本机 RTX4060-Laptop+i7 场景仍 ≥3× 实时，满足直播字幕）；debug/release RTF 几乎一致（ORT 主导，Rust opt level 不影响解码路径）；GUI 冒烟 worker 常驻 **1060MB**（回收阈值 基线+2048MB 天然覆盖）。
- **S0-④ 线程定案**：1/2/3 线程 → RTF **0.459/0.312/0.280**，单调改善 → `NUM_THREADS=3`（官方 mic 示例同为 3）。
- **LID**：zh/zh/zh/ja/en 五语判定全对（启发式与 sherpa 输出无关，纯文本侧）。
- **GUI 端到端冒烟**（临时 config，settings.json `asr_engine=qwen3` + models_dir 指真实缓存）：管道自启 → worker 拉起 → 日志 `ASR worker ready … Qwen3-ASR-0.6B (qwen3)`，**全程 0 WARN / 0 ERROR**。
- **观察点登记（不预做）**：codeswitch.wav 的法/意/西段被模型转写为英文（Qwen3-ASR 上游语码切换行为，非引擎缺陷）；长段 max_total_len 边界未触发（VAD 8s 上限保护）。
- **施工注记**：test_wavs 混有 44.1kHz 样例 → 生产管线采集侧恒 16k 不受影响，探针内置线性重采样；smoke 用的 settings.json 若含非法 JSON，`load()` 在日志初始化前静默回退默认（旧行为，非本 WP 缺陷，冒烟时已用正斜杠路径规避）。

### 4.0 拍板与编号

- 设置拓扑 **B-α：`asr_engine` 新增第三值 `"qwen3"`**，经用户拍板生效（AGENTS.md 2026-09-08）。B-β（`funasr_model` 加第 4 项）否决：污染 D-14 的 3 项 1:1 结构、"FunASR 模型"语义错误、padding/诊断按 funasr 家族走会打架。qwen3 不是 FunASR 模型，不进 `funasr_model`。
- 新偏差自 **D-25** 起编号（D-24 已被下载源决策占用）。
- 下载语义：qwen3 无可用 MS 源（§4.2 核查）→ 注册表 `ms=None + always_hf=true`，完全复用 D-24 链路（`hub_chain` 纯 HF 链 + `hf_endpoint_for` 端点随所选 hub）与 UI 诚实提示。

### 4.1 判定理由（四面全绿）

- **模型面**：2026-01 官方开源（Apache 2.0）；30 语 + 22 汉语方言 + 歌词/说唱识别，方言/粤语覆盖强于 SenseVoice 五语，与实时同传字幕/多语种会议室场景高度匹配；0.6B 为官方口径开源 SOTA 级小型。
- **引擎面**：binding 1.13.7 `OfflineQwen3ASRModelConfig`（offline_asr.rs:325-352）逐字段核对在位（`conv_frontend/encoder/decoder/tokenizer` 四路径 + `max_total_len/max_new_tokens/temperature/top_p/seed/hotwords`）；**v1.13.7 原生库（2026-09-01 发布）与 binding 同 release，tag 内自带官方 `qwen3-asr-c-api.c` 示例**——r1 的「binding 有字段 ≠ 原生支持」风险基本排除（spike 保留为验收程序）。
- **工程面**：§1.6 盘点——下载/缓存/hub 回落/manager 五层就绪或仅常数表扩展，缺口集中在注册表条目 + 引擎实现 + 三处分派/表臂；UI 模型下拉/padding 滑杆显隐已按引擎隔离，qwen3 天然不显示无关控件。
- **风险面**：默认引擎仍 SenseVoice，qwen3 仅显式选择才启用；「实验性」标注 + manager 错误分类/自动重启兜底，出问题不阻塞主路径。

### 4.2 模型与仓库定案（r3 实测，2026-09-08）

**目标仓（全网唯一正确解）= `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`**（作者 csukuangfj**2** = k2-fsa/sherpa-onnx 维护者小号；lastModified 2026-04-07；官方文档页下载指引同源）。**注意作者多一个 "2"**——`csukuangfj/`（无 2）同名仓 HF/MS 双实测 404。

manifest 六文件（HF API `?blobs=true` 实测；即注册表下载清单）：

| 文件 | 实测字节 | LFS sha256（前 16 位） | 注册表下限拟值 |
|---|---|---|---|
| `conv_frontend.onnx` | 44,148,281 | `d22dc4423e0940e4` | 20,000,000 |
| `encoder.int8.onnx` | 182,491,662 | `60748d3e6744a57c` | 90,000,000 |
| `decoder.int8.onnx` | 755,914,231 | `4f6885be5959ae26` | 350,000,000 |
| `tokenizer/merges.txt` | 1,671,853 | —（非 LFS） | 1,000,000 |
| `tokenizer/tokenizer_config.json` | 12,487 | —（非 LFS） | 5,000 |
| `tokenizer/vocab.json` | 2,776,833 | —（非 LFS） | 1,000,000 |
| **合计** | **987,015,347 ≈ 941 MB** | | `estimated_bytes = 1_020_000_000` |

- 三个 tokenizer 文件与 nano 包 `Qwen3-0.6B/` 目录对应文件字节数完全一致（1,671,853 / 2,776,833）——同源 Qwen3-0.6B tokenizer，互为旁证。
- 仓内 `test_wavs/` 17 条样例**含 `transcript.txt` 参照转写**（中/英/粤/日/德/法/西/俄/阿拉伯/rap/绕口令/噪音/语码切换）→ 验收免麦克风且有 ground truth。
- 不变量：`files_min_bytes[0] × 2 (40MB) < estimated_bytes` ✓；下限刻意远低于实测（假阴性自愈、假阳性死局，同 nano 口径）。

**HF 全量枚举排除清单**（search=`qwen3-asr` / `sherpa-onnx-qwen3`，90+ 仓；r3 实测）：

- ✅ 仅目标仓满足「官方维护者 + sherpa 官方包布局（三 onnx + tokenizer 目录）+ int8 档」三全。
- ❌ 官方**原版**权重：`Qwen/Qwen3-ASR-0.6B(-hf)`、`Qwen/Qwen3-ASR-1.7B(-hf)`——PyTorch/HF 格式，sherpa 引擎不可用（仅溯源注记）。
- ❌ 同名转存：`wt5719001` / `jkman2023` / `cattle12`（与官方包同名，无维护者身份，无采信必要）。
- ❌ 自转换/非官方变体：`pantinor`、`ilmina`（0.6B+1.7B）、`thieunv`（0.6B+1.7B）、`tritueviet`、`HatiSkoll28` 等——布局与质检未验证（1.7B 变体另见「明确不接」）。
- ❌ 运行时不兼容：GGUF 系（unslothai/ggml-org/FlippyDora…）、MLX 系（Apple）、OpenVINO/CoreML/neuron、非 sherpa 布局 ONNX 散件（andrewleech/louis030195/rhasspy 等）。

**ModelScope 核查（r3 复测）：无可用 MS 源**：

| MS 仓库 | 实测 | 判定 |
|---|---|---|
| `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25` | **404** | 官方未发布 MS 单仓 |
| `csukuangfj/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25` | **404** | namespace 无 "2" 亦无 |
| `Qwen/Qwen3-ASR-0.6B` | 200 存在 | 官方**原版**权重（`Qwen3ASRForConditionalGeneration`，PyTorch）——格式不符，引擎不可用，仅溯源 |
| `zengshuishui/Qwen3-ASR-onnx` | 200 存在 | 社区镜像（散文件，含 fp32/1.7B 档；sherpa 官方文档页确实引用其 fp32/1.7B，但本 WP 只用 int8 档，仍不采信——非官方质检，D-24 备注封存） |
| `zhaochaoqun/sherpa-onnx-asr-models` | tar.bz2 整包 | 官方 GitHub Releases 转存（r2 已核），整包格式不可用 |

**官方运行基线**（sherpa 文档页 `qwen3-asr/pretrained.html` CLI dump 实证）：

- 路径参数：`--qwen3-asr-conv-frontend` / `--qwen3-asr-encoder` / `--qwen3-asr-decoder` / `--qwen3-asr-tokenizer=<tokenizer 目录>`——与 binding 四字段 1:1（tokenizer 传**目录**，同 nano 形状）。
- 采样/解码：`max_new_tokens=512`、`num_threads=2~3`（官方示例混用：mic=3、VAD 文件=2）；config dump 实证 `temperature=1e-06, top_p=0.8, seed=42, max_total_len=512, hotwords=""`，`decoding_method=greedy_search`（框架默认，同 sensevoice/nano）。
- **binding Default 陷阱（与 nano 同款，必显式）**：`OfflineQwen3ASRModelConfig::default()` 的 temperature/top_p/seed 恰为官方值，但 **`max_new_tokens=128`（官方 512）**——`max_new_tokens` 与 `max_total_len` 一律显式写死，不裸用 Default。
- 输出格式：官方样例（Obama.wav）转写为**纯文本**，无 `<|...|>` 标签残留——qwen3 预期不需要 nano 式标签清理（保留防御性 strip，spike 复核定案）。
- RTF：r1 口径 **0.077–0.168@2线程**（优于 nano 的 0.144–0.191）；本机弱 CPU 下限与内存峰值由 spike 复核。
- fp32 档 / 1.7B 档：官方文档页指向 MS zengshuishui 社区镜像——**均不用**（体积/CPU 实时性 + 非官方质检）。

### 4.3 与 nano 的实现差异（决定 B5 形状）

| 维度 | nano（WP-A 已实装） | qwen3（本 WP） |
|---|---|---|
| 语言参数 | 有 `language` 创建期字段；set_language 重建识别器 | **无该字段**（纯 auto-LID）：`set_language("auto")`→Ok（no-op）；非 auto→`Err(Unsupported)`（诚实暴露；manager 对 Failed 仅 warn 一次并照常提交挂起，无噪音面） |
| 提示词 | system_prompt/user_prompt 必填官方值 | **无 prompt 字段** |
| tokenizer | 目录 `Qwen3-0.6B/` | 目录 `tokenizer/` |
| 特有字段 | itn/hotwords | `max_total_len`（默认 512 显式设置；本管线 VAD `max_speech_duration` 默认 8s，天然远低于边界） |
| 输出后处理 | `<|...|>` 清理 + "sil" 语义（原版 1:1） | 预期纯文本：防御性标签清理 + trim + 空→None；`AsrResult.language/language_name` 用启发式 LID（与 nano 同款，提升为共享函数） |
| 线程 | num_threads=2 | num_threads=2（官方 VAD 示例口径；spike 对 1/2/3 择优定案） |

### 4.4 S0 spike（0.5 天，降级为验收程序）

探针测试 `probe_real_qwen3`（仿 `probe_real_nano`，`#[ignore]`，不入常规测试面）：模型包经 hf-mirror 预置真实缓存（复用 WP-A 的 lt-models 下载演练模式，或手动 curl 直链）。四项验证，结论回填 §4.2/§4.6 后再进入实装提交：

1. **加载冒烟**：v1.13.7 原生库 create recognizer 成功（预期通过——tag 已含 c-api 示例）；
2. **输出格式**：`test_wavs/cantonese.wav`、`raokouling.wav`、`ja1.wav` 对照 `transcript.txt`——确认无标签残留、标点风格、是否需要额外后处理；
3. **本机 RTF/内存**：官方 0.077–0.168 为官方机器口径，本机复测 + worker RSS 采样（对照回收阈值 基线+2048MB）；
4. **num_threads 1/2/3** 择优（AR 解码受益多线程，预期 2 或 3）。

### 4.5 改造清单（B-α；逐文件，行号 = 2026-09-08 现场）

| # | 文件 | 改动 |
|---|---|---|
| B1 | `crates/lt-models/src/registry.rs` | 新增 `pub const QWEN3_ASR: ModelEntry { key:"qwen3-asr-0.6b", display:"Qwen3-ASR-0.6B", hf:Some("csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25"), ms:None, always_hf:true, estimated_bytes:1_020_000_000, files:六件套, files_min_bytes:§4.2 表 }` + `pub fn qwen3_entry() -> ModelEntry`（单一模型，无键参）；`const { assert!(QWEN3_ASR.always_hf && QWEN3_ASR.ms.is_none()) }`（D-24 不变量，同 FUNASR_NANO）；`manifest_min_bytes_parallel_to_files` 测试纳入 QWEN3_ASR；新增 `qwen3_entry_hf_only` 测试 |
| B2 | `crates/lt-proto/src/settings.rs:17` | `ASR_ENGINES: [&str; 3] = ["funasr", "whisper", "qwen3"]`（sanitize :171 引用常量自动放行，无 legacy 别名）；注释「r8 定稿：双引擎」更新；补测试：`asr_engine="qwen3"` 过 sanitize 原样保留、非法值仍回退 funasr |
| B3 | `crates/lt-models/src/cache.rs:146-152,171-215` | `is_asr_cached` 加 `"qwen3"` 臂：`is_funasr_cached(models_dir, &registry::qwen3_entry())`（manifest 机制泛化，qwen3 的 model 参数无意义，忽略）；`missing_models` 加 `"qwen3"` 臂（固定条目，display "Qwen3-ASR-0.6B"）。测试：HF 布局稀疏 manifest 造缓存 → `is_asr_cached(dir,"qwen3",…)` 命中、`missing_models` 清空；未缓存报缺失且 `always_hf=true` 带出 |
| B4 | `crates/lt-app/src/pipeline.rs:648-699,736-750,852-856,868-875` | ① `build_worker_config` 加 `"qwen3"` 臂：`entry=qwen3_entry()` → `local_model_dir` 未缓存 None；`engine:"qwen3"`、`pad_seconds:None`、`options{model_dir}`；② 启动诊断 :736 分支加 qwen3 臂取 QWEN3 条目（否则未缓存日志打错模型名）；③ TlSwitch 日志 model_key :852/:868 加 `engine=="qwen3" → "qwen3-asr-0.6b"` 臂。测试：稀疏 manifest 下 `build_worker_config(&base,"qwen3",…)` 装配成功且 pad None；未缓存 None |
| B5 | `crates/lt-asr/src/engines/qwen3.rs`（新建）+ `engines/mod.rs` 共享化 | `Qwen3AsrEngine::load(model_dir)`：校验三 onnx + `tokenizer/` 目录（缺一即 EngineError::Load，清单与注册表同源）；`build_recognizer`：`cfg.model_config.qwen3_asr = OfflineQwen3ASRModelConfig{ conv_frontend/encoder/decoder: Some(join(…)), tokenizer: Some(join("tokenizer")), max_total_len:512, max_new_tokens:512, temperature:1e-6, top_p:0.8, seed:42, hotwords:None }`，`num_threads=2`（S0 定案可调）；`transcribe` 同 nano 流形状：防御性 `<\|…\|>` 清理 + trim + 空→None；`guess_language` 填 `AsrResult.language/language_name`；`set_language`：""\|"auto"\|"Auto"→Ok(no-op)，其余→`Err(EngineError::Unsupported)`；`set_input_padding` 保持 trait 默认。**共享化**：`guess_language` 与标签 strip 从 `NanoEngine` 关联函数提升为 `engines/mod.rs` `pub(crate)` 自由函数，nano 委托调用（既有 nano 单测保持通过），qwen3 复用。单测：postprocess 分支 / set_language auto 放行与非 auto 拒绝 / load 缺文件报错 |
| B6 | `crates/lt-asr/src/engines/mod.rs` + `src/lib.rs` | `pub mod qwen3;` + `pub use engines::qwen3::Qwen3AsrEngine;` |
| B7 | `crates/lt-app/src/main.rs:108-159` | `asr_worker_entry` 加 `"qwen3"` 臂（仿 nano 臂：`options.model_dir` → `Qwen3AsrEngine::load(&dir)`，不消费 pad_seconds/language） |
| B8 | `crates/lt-asr/src/manager.rs:79-83` | `engine_family` 加 `"qwen3" => "qwen3"`（显式独立家族；apply_pending :301 `get("qwen3")` 恒 None 天然 no-op，UI 不产生该家族 SetPadding，零逻辑改动）；测试补 `engine_family("qwen3")=="qwen3"` |
| B9 | `crates/lt-ui/src/windows/panel/vad.rs` | ① `ENGINES:24-28` 扩三项 `("qwen3","engine_display_qwen3","Qwen3-ASR")`（尾部追加，既有索引不动）；测试 :1070 三值数组；② `model_cache_status:195` 加 `"qwen3"` 臂（qwen3_entry → is_funasr_cached → Cached/Missing(estimated_bytes)）；③ 语言下拉 :300-315 后：engine=="qwen3" 时 `hint_line(qwen3_lang_auto_hint)`；④ D-24 hint :429-434 条件扩 qwen3（hub==ms && engine=="qwen3" → `model_qwen3_hf_only`）；⑤ 过一遍确认：funasr 模型下拉 :318 / whisper 档位 :349 / 双 padding :391,:400 / nano hint 对 qwen3 均天然隐藏 |
| B10 | `assets/i18n/zh.yaml` + `en.yaml` | 三键两份同步（Rust additions 段）：`engine_display_qwen3`（zh「Qwen3-ASR（多语种，实验性）」/ en "Qwen3-ASR (multilingual, experimental)"）、`model_qwen3_hf_only`（同 nano 措辞）、`qwen3_lang_auto_hint`（zh「Qwen3-ASR 恒自动检测源语言，识别语言选择对其无效」/ en 同义） |
| B11 | 探针处置 | `probe_real_qwen3`（lt-asr）与可选下载演练探针保留为 `#[ignore]` 验收工具（同 A11 模式），随本 WP 提交 |
| B12 | 文档回写 | 本文 §0/§4 状态勾销 + 实测数据回填 §4.2/§4.6；`docs/archive/rewrite-research.md` §1.5 登记 **D-25**（新增 Qwen3-ASR-0.6B 引擎；不支持项：无源语言指定 / 无热词 UI / 不接 1.7B 与 fp32）；`AGENTS.md` 待办更新 |

**明确无需改动**（验证项，非改造项）：`lt-app/backend.rs`（SwitchEngine 载荷引擎无关 :77-90；`current_missing` :251 已透传 asr_engine；hf_endpoint 接线 :188 在位）、`lt-models/download/mod.rs`（hub_chain/hf_endpoint_for 泛化）、manager apply_pending（:296-312 家族取键天然 no-op）、面板 diff/恢复页（恢复默认回 funasr/sensevoice 语义成立）。

### 4.6 验收清单

1. `cargo test --workspace` 全绿（341 + 新增：B1 不变量/B2 sanitize/B3 探测/B4 分派/B5 引擎单测/B8 家族回归/B9 三值表）；新码 clippy 零告警。
2. S0 spike 四项结论确认（§4.4）并回填本文档。
3. 实机（临时 `LIVETRANSLATE_CONFIG_DIR`，settings 显式 `models_dir` 指真实缓存）：
   - 选 qwen3 → 识别页下载 941MB（缓存预置则秒完成，验证 manifest 幂等命中）→ 引擎切换成功、悬浮窗 ready `Qwen3-ASR-0.6B [cpu]`；
   - `test_wavs/` 中/英/粤/日/语码切换各一条对照 `transcript.txt`；`language` 字段启发式判定正确；
   - 语言下拉选非 auto → 日志恰一条 warn（Unsupported）且识别照常（auto-LID）；选回 auto 无 warn；
   - padding 滑杆 / funasr 模型下拉 / whisper 档位均隐藏；hub=ms 显示镜像 hint；日志核对 URL：hub=hf → `huggingface.co` 直连，hub=ms → `hf-mirror.com`；
   - 切回 sensevoice/whisper 一切如旧；`cargo run -p lt-app` 冒烟无回归。
4. 观察点（不预做，出现再登记）：int8 AR 重复/幻觉（#3066 类）；长段行为（max_total_len 边界）；本机 RTF 与加载时长（回填 §4.2）；粤语输出字节级 fallback token（nano 同款观察点）。

### 4.7 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| 原生库支持（binding 字段 ≠ 原生实现） | 低（已基本排除） | v1.13.7 同 release 含官方 c-api 示例（§4.1）；S0 首项兜底；不过则升 `sherpa-onnx` crate 补丁版本（`"1.13"` 语义内） |
| 无 language 指定（asr_language 下拉对其无效） | 中 | auto-LID 即设计行为；UI hint 明示（B10）；set_language 非 auto → Unsupported 诚实暴露；`AsrResult.language` 启发式兜底 |
| decoder 756MB → worker RSS 峰值（估 1.1–1.5GB） | 低 | RSS 回收 = 基线+2048MB 天然覆盖（nano 实测 1.09GB 同级）；S0 复核 |
| max_total_len=512 对长段截断 | 低 | 管线 VAD max_speech_duration 默认 8s，远低于边界；观察点登记 |
| 上游采样参数扰动（r1 #3509 情报） | 低 | 不暴露 hotwords UI；采样参数固定官方基线；seed 固定 42 确定性 |
| int8 AR 幻觉/重复 | 中 | 「实验性」标注维持；manager 错误分类+自动重启兜底；高频复现再评估相邻重复折叠（先不做） |
| hf-mirror 第三方信任 × 无哈希校验 | 中 | D-24 既有语义：选 HF 即官方直连、选 MS 才走镜像；UI 诚实提示；sha256 渐进登记（既有备忘——三个 onnx 的 LFS sha256 已实测入档 §4.2） |
| 1.7B 诱惑（社区已有自转换包） | — | 明确不接：#3535 open（2026-09-08 复核）+ 非官方转换 + 纯 CPU AR 解码成本翻三倍 |

## 5. WP-C：缓存完整性硬化 → **取消**（r2）

原 WP-C（r1 §5「清单全文件存在 && 总字节≥阈值」）已被 DL-1 manifest 探测整体覆盖（§2.4）：`dir_has_manifest` 就是「清单全在 + 逐文件下限」语义，且经六文件 nano 包实战验证。无需任何新代码；A1 填对清单即获得全部防御。

## 6. 近两年开源 ASR 模型扫描（维持 r1 §6）

扫描结论与观察名单（Cohere Transcribe 2B / FireRedASR2-CTC）不变；MLT 有条件解锁路径（r1 §6.3）不变，另补：**MS 亦无 MLT 源**（r2 实测），若未来解禁只能走 HF。

## 7. 排期与边界（r3 更新）

```
✗ WP-A  nano 实装 + 注册表修正 + D-24 端点映射接线 + A8 UI    ≈ 1 天   ← 已完工（r2.2，§3.5）
✗ WP-B  S0 spike（加载/格式/RTF/线程：3 线程定案）             0.5 天   ← 已完工（§4.8）
✗ WP-B  qwen3 实装（B-α，B1~B12）                              1 天     ← 已完工（r3.1，§4.8）
─── 每步收尾 cargo test --workspace 全绿 + 自主中文 commit
```

**明确不做**：Qwen3-1.7B、hotwords UI、MLT 默认解禁、MS 社区镜像采信（zengshuishui 系）、tar.bz2 整包下载特性（hf-mirror 实测后封存）、`hf_endpoint` 设置键（r2.1 语义下端点全自动，无用户设置面；未来镜像不可用或有自定义需求再议）、Parakeet/Canary/Voxtral/Moonshine/Kimi-Audio/Phi-4-MM（r1 §6 理由不变）。

**不改的事**：默认引擎 SenseVoice；`funasr_model` 3 项 1:1 结构；whisper 双栈；mlt 置灰；`hub_chain` 回落语义（D-24 仅补接线与不变量，不改链路算法）。

## 8. 参考资料（r2 增补）

- r1 §8 全部保留。增补：
- 目标仓（nano）：https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30
- 目标仓（qwen3）：https://huggingface.co/csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25 ｜ 官方文档页：https://k2-fsa.github.io/sherpa/onnx/qwen3-asr/pretrained.html ｜ C API 示例：https://github.com/k2-fsa/sherpa-onnx/blob/master/c-api-examples/qwen3-asr-c-api.c ｜ 1.7B 支持追踪：https://github.com/k2-fsa/sherpa-onnx/issues/3535
- Qwen3-ASR 官方原版权重（溯源，非引擎源）：https://huggingface.co/Qwen/Qwen3-ASR-0.6B ｜ https://modelscope.cn/models/Qwen/Qwen3-ASR-0.6B ｜ 发布博客：https://qwen.ai/blog?id=qwen3asr
- hf-mirror：https://hf-mirror.com （`HF_ENDPOINT` 语义的 Rust 等价 = D-24 r2.1 端点映射 `hf_endpoint_for(selected_hub)`：选 MS 时自动取镜像）
- MS 核查对象：fuyuantech/fuyuan-sherpa-onnx-funasr-nano-int8-2025-12-30 ｜ zhaochaoqun/sherpa-onnx-asr-models ｜ zengshuishui/FunASR-Nano-onnx ｜ manyeyes/Fun-ASR-Nano-2512-LLM-onnx ｜ FunAudioLLM/Fun-ASR-Nano-2512
- 官方 C API 示例：https://github.com/k2-fsa/sherpa-onnx/blob/master/c-api-examples/funasr-nano-c-api.c
