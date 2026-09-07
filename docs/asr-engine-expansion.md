# ASR 引擎扩展可行性分析与施工方案（FunASR Nano / Qwen3-ASR / 开源模型扫描）

> 状态：**阶段二活跃文档**。**r2（2026-09-08）WP-A 动工方案实测定稿**：HF 目标仓全量枚举定案、ModelScope 全量核查（无官方源）、hf-mirror 经产品下载器实测 20.4 MB/s、DL-1~6 后代码面逐行重验、**D-24 诚实下载源决策**登记。WP-A 按本文 §3 施工；WP-B（Qwen3-ASR）调研维持，实装新偏差自 **D-25** 起（D-24 已被下载源决策占用，AGENTS.md 原注记「WP-B 自 D-24 起」顺延）。
>
> r1（2026-09-07）：初版调研。其 §2.4「funasr 缓存 50MB 单文件误判」漏洞与 WP-C 硬化项已被 DL-1（D-22 manifest 探测）消化，r2 正式取消；r1 §1.4 所称「设置里已有 hf-mirror endpoint 覆写」经核仅指 lt-models 机制，**应用层未接线**（本 r2 §3-A10 补齐）。

---

## 0. 结论速览（TL;DR，r2 更新）

| 议题 | 判定 | 一句话依据 |
|---|---|---|
| **FunASR Nano（funasr-nano-2512）实装** | **可行，turnkey，按 §3 立即施工（WP-A）** | 目标仓定案 `csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30`（HF 全量枚举 55+ 仓后唯一正确解，六文件 2026-01-07 版实测合计 1,009,605,061B）；锁定的 sherpa-onnx 1.13.7 binding 与原生库同 release 均含 `OfflineFunASRNanoModelConfig`；缺口仅注册表修正 + 引擎实现 + 两处分派 |
| **无真实 MS 源模型的下载语义** | **D-24（2026-09-08 用户裁决）：诚实回退，不伪造** | 注册表 `ms` 字段只填真实存在的 ModelScope 仓，无则 `None`；`always_hf` 语义 =「仅 HF 源」，所选 hub=ms 时链路**直达 HF（经 hf-mirror 镜像端点）**，禁止把非 MS 链接塞进 ms 字段或做特殊标记（现注册表错误条目即反例）；`hub_chain` 现有实现已满足，需补 hf_endpoint 设置键 + 应用层接线 + UI 诚实提示 |
| **hf-mirror 可用性** | **实测通过，镜像备选路线全部封存** | 产品下载器全链路（hub_chain→直链→206 续传→manifest 校验）963MB 用时 47.3s、均速 20.4 MB/s，六文件与 HF API 逐字节一致；ModelScope 侧无官方 nano 源，zengshuishui 等社区镜像不采信 |
| **缓存完整性硬化（原 WP-C）** | **取消——已被 DL-1 消化** | `cache.rs:38-42 dir_has_manifest` 逐文件「存在+达下限」对多文件模型天然成立，注册表填对清单即得，无需新代码 |
| **Qwen3-ASR-0.6B 接入** | **可行，先 0.5 天 spike 再实装（WP-B，维持 r1 结论）** | binding 已有 `OfflineQwen3ASRModelConfig`；CPU RTF 0.077–0.168@2线程；无真实 MS 源，下载同样走 D-24 语义 |
| Qwen3-ASR-1.7B | **不可行，不接** | sherpa-onnx 官方未支持（issue #3535 open）；纯 CPU AR 解码实时性无保障（不变） |
| funasr-mlt-nano-2512 | **维持置灰（D-14，不变）** | 无官方转换；MS 亦无源（r2 实测）；社区包质量未验证 |

**施工顺序**：WP-A（nano 实装 + 注册表修正 + D-24 接线，约 1 天）→ WP-B spike（0.5 天）→ WP-B 实装（1–1.5 天，拓扑 B-α 待用户拍板）。

---

## 1. 现状盘点（r2 代码面重验；行号 = 2026-09-08 现场）

### 1.1 引擎架构（双栈，均已上线，r1 内容维持有效）

```
Settings { asr_engine: "funasr"|"whisper", funasr_model, whisper_model_size, hub, hf_endpoint(r2 新增), ... }
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

### 1.2 FunASR Nano 现状：占位已成、实装未做（r2 逐行核实）

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
- **端点现状**：`Downloader::new(dl_dir, dl_proxy_mode)`（backend.rs:187）**未调 `with_hf_endpoint`** → HF 下载直连官方。国内无代理场景基本不可用（r2 实测 hf-mirror 20.4 MB/s，见 §2.7）——A10 补此缺口。

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

### 2.3 Qwen3-ASR（维持 r1 结论）

0.6B 官方转换包 `csukuangfj2/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`（HF 实测存在；**MS 无官方单仓**——r2 实测 csukuangfj2 404；MS 侧存在 zengshuishui/Qwen3-ASR-onnx 社区散文件镜像，非官方不采信）。1.7B 不接。详情见 r1 §2.3/§4（内容仍有效，不再重复）。

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
3. **HF 下载默认经 hf-mirror**：settings 新增 `hf_endpoint` 键（默认 `https://hf-mirror.com`，可改回官方/自建），backend 接线 `with_hf_endpoint`（补 1.5 缺口）。国内受益者不止 nano——whisper 六档（今日 always_hf）同样受益。
4. **UI 诚实告知**：hub=ms 且选中仅 HF 源模型时，下载区显示「该模型仅提供 HuggingFace 源，将经 HF 镜像下载」（i18n zh/en 同步）。
5. **供应链注记**：第三方镜像 × 无哈希校验 = 信任依赖（distribution.md §6 既有风险）。缓释：端点用户可改；后续在 registry 渐进登记 sha256（既有备忘，非本 WP）。
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
| A9 | `crates/lt-proto/src/settings.rs` | 新增 `pub hf_endpoint: String`：`#[serde(default = "default_hf_endpoint")]`（⚠️ Settings 是容器级 `#[serde(default)]`（:41），字段级 default 必须显式——否则旧 settings.json 导入得空串）；`Default` impl（:97 区）与 `default_hf_endpoint() -> String { "https://hf-mirror.com".into() }`；sanitize（:192 区）：空/非法（非 `http(s)://` 开头）→ 回默认并记 `fixed`。lt-proto 只加数据键不动 Cmd（D-17 字体先例） |
| A10 | `crates/lt-app/src/backend.rs:187` + `assets/i18n/zh.yaml`/`en.yaml` | `Downloader::new(dl_dir, dl_proxy_mode).with_hf_endpoint(settings.hf_endpoint.clone())`（D-24-③ 接线，弥补 1.5 缺口）；i18n 新键两份同步：`label_hf_endpoint`/`hint_hf_endpoint`（设置下载源组输入框，vad.rs :410 hub 下拉同区）/`model_nano_hf_only` |
| A11 | `crates/lt-models/src/download/mod.rs`（探针处置） | `probe_nano_tmp` 探针测试保留转正（`#[ignore]` 注记改为「WP-A 验收演练工具」），随本 WP 提交 |
| A12 | 文档回写 | 本文 §0/§1.2 状态勾销；`docs/archive/rewrite-research.md` §1.5 登记 D-24 + WP-2 勾销（处置=实装）；`AGENTS.md` 待办（WP-A ✗ / D-24 / WP-B 自 D-25 起）；实测数据（下载 47.3s/20.4MB/s、加载时长、RTF 本机复测）回填 §2.2 |

### 3.3 验收清单

1. `cargo test --workspace` 全绿（334+ 新增：A1 不变量/A3 引擎单测/A7 回归/A9 sanitize）。
2. 实机（临时 `LIVETRANSLATE_CONFIG_DIR`，settings 显式 `models_dir` 指真实缓存）：
   - 选 nano → 识别页点下载 → **秒完成**（缓存已预置，验证 skip 幂等 + manifest 命中）；
   - 引擎切换成功、悬浮窗 ready；`test_wavs/` 中/英/粤各一条对照转写，语言显示正确；
   - SenseVoice padding 滑杆对 nano 隐藏、日志无 padding 下发（A7 断言呼应）；
   - hub=ms 时 nano 显示「仅 HF 源」hint；改 `hf_endpoint` 为官方端点后下载设置生效（可用 mock 或观察日志 URL）；
   - 切回 sensevoice 一切如旧；`cargo run -p lt-app` 冒烟无回归。
3. 观察点（不预做，出现再登记）：粤语 `<0xB2>` 类字节 token；本机 RTF 与加载时长（回填 §2.2）；#3066 类重复文本。

### 3.4 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| int8 AR 解码偶发重复/幻觉（#3066 类） | 中 | 「实验性」标注维持；manager 错误分类+自动重启兜底；高频复现再评估相邻重复折叠（先不做） |
| hf-mirror 第三方信任 × 无哈希校验 | 中 | 端点可改（A9/A10）；sha256 渐进登记（既有备忘）；UI 诚实提示 |
| 首次加载慢（600MB llm ORT 初始化） | 低 | ready 180s 充裕；ModelLoadStart 模态已有链路 |
| worker 内存峰值（int8 约 1–1.5GB） | 低 | RSS 回收 = 基线+2048MB 天然覆盖 |
| `hf_endpoint` 旧 settings 导入为空串 | 已防 | A9 字段级 serde default + sanitize 兜底 |
| 大陆直连 HF（用户改回官方端点又无代理） | 低 | 属用户显式选择；代理三模式仍兜底；README 提示 |

---

## 4. WP-B：Qwen3-ASR-0.6B（维持 r1 §4，增补两点）

r1 §4.1–4.5 全部有效（spike 四项验证、B-α 拓扑推荐 `asr_engine` 第三值、改造清单 B1~B12、风险表）。r2 增补：

1. **下载语义**：qwen3 无真实 MS 源（§2.5），注册表 `ms=None + always_hf=true`，走 D-24 链路——B1 条目的 ms 字段按此填写。
2. **编号**：实装产生的新偏差自 **D-25** 起（D-24 已占用）；B11 文档回写相应更新。
3. 设置拓扑（B-α vs B-β）仍需用户拍板后方可开工。

## 5. WP-C：缓存完整性硬化 → **取消**（r2）

原 WP-C（r1 §5「清单全文件存在 && 总字节≥阈值」）已被 DL-1 manifest 探测整体覆盖（§2.4）：`dir_has_manifest` 就是「清单全在 + 逐文件下限」语义，且经六文件 nano 包实战验证。无需任何新代码；A1 填对清单即获得全部防御。

## 6. 近两年开源 ASR 模型扫描（维持 r1 §6）

扫描结论与观察名单（Cohere Transcribe 2B / FireRedASR2-CTC）不变；MLT 有条件解锁路径（r1 §6.3）不变，另补：**MS 亦无 MLT 源**（r2 实测），若未来解禁只能走 HF。

## 7. 排期与边界（r2 更新）

```
WP-A  nano 实装 + 注册表修正 + D-24 hf_endpoint 接线 + A8 UI    ≈ 1 天   ← 本文 §3，可立即开工
WP-B  spike（qwen3 加载/格式/RTF）                                0.5 天   ← 结论回填 §4
WP-B  qwen3 实装（B-α，待拍板）                                  1–1.5 天
─── 每步收尾 cargo test --workspace 全绿 + 自主中文 commit
```

**明确不做**：Qwen3-1.7B、hotwords UI、MLT 默认解禁、MS 社区镜像采信（zengshuishui 系）、tar.bz2 整包下载特性（hf-mirror 实测后封存）、Parakeet/Canary/Voxtral/Moonshine/Kimi-Audio/Phi-4-MM（r1 §6 理由不变）。

**不改的事**：默认引擎 SenseVoice；`funasr_model` 3 项 1:1 结构；whisper 双栈；mlt 置灰；`hub_chain` 回落语义（D-24 仅补接线与不变量，不改链路算法）。

## 8. 参考资料（r2 增补）

- r1 §8 全部保留。增补：
- 目标仓：https://huggingface.co/csukuangfj/sherpa-onnx-funasr-nano-int8-2025-12-30
- hf-mirror：https://hf-mirror.com （`HF_ENDPOINT` 语义的 Rust 等价 = settings `hf_endpoint`）
- MS 核查对象：fuyuantech/fuyuan-sherpa-onnx-funasr-nano-int8-2025-12-30 ｜ zhaochaoqun/sherpa-onnx-asr-models ｜ zengshuishui/FunASR-Nano-onnx ｜ manyeyes/Fun-ASR-Nano-2512-LLM-onnx ｜ FunAudioLLM/Fun-ASR-Nano-2512
- 官方 C API 示例：https://github.com/k2-fsa/sherpa-onnx/blob/master/c-api-examples/funasr-nano-c-api.c
