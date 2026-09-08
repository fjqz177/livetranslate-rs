# SenseVoice 语言字段恒 "auto" —— 同语言免翻译失灵与显式语言全丢（D-30）

> 2026-09-08 用户反馈：ASR 选 SenseVoice + 识别语言 auto 时，识别结果语言恒为 "auto"，
> 目标语言（如 zh）无法与之相等 →「同语言不翻译」机制不触发，中文语音仍被送进 LLM 翻译。
> 其他引擎无此问题。本文给出根因证据、影响面、方案对比与施工卡。
> 施工卡编号起 **SV-1**（避免与 WD/ASR 扩展/AH/MC 等既有序列混淆）；行为差异落档 **D-30**
> （次新偏差自 D-31 起）。

> **✅ 已完工（2026-09-08，66871a0）**：SV-1~SV-3 全部落地——369 测全绿（净增 1）+ clippy
> 相关文件零告警 + release 单 exe 构建过 + 实机探针 6 组全绿 + GUI 冒烟实测
> `Same language (zh), no translation` 反复出现（修复前此处为白翻译）。
> 探针 `probe_real_sensevoice_lang_decision`（ignored）留档供 AI 引擎级回归复用。

## 1. 现象

实机（GUI 冒烟路径观察 + 引擎级复现，2026-09-08）：

| 引擎 | 识别语言=auto 时 `AsrResult.language` | 徽标（悬浮窗 overlay.rs:517） | 同语言免翻译 |
| --- | --- | --- | --- |
| Whisper | ISO 码（`full_lang_id_from_state()` 模型 LID，whisper.rs:164-170） | `[zh]` | ✅ 触发 |
| FunASR-Nano | 启发式 LID（`guess_language`，nano.rs:140-143） | `[zh]` | ✅ 触发 |
| Qwen3-ASR | 启发式 LID（`guess_language`，qwen3.rs:125） | `[zh]` | ✅ 触发 |
| **SenseVoice** | **恒 "auto"**（sensevoice.rs:172-180） | **`[auto]`** | ❌ **不触发** |

后果（主诉）：目标语言 zh + 中文语音（SenseVoice auto）→ `commit_text` 的
`lang == target_language`（pipeline.rs:1378）恒 false → 照常 `submit_translation` →
LLM 白翻译：token 消耗、延迟与译文行噪声。

## 2. 实机证据（本机缓存模型 + 真实 wav）

探针：`crates/lt-asr/src/sensevoice.rs` 末暂存模块 `probe_real_sensevoice_raw_tags`
（`--ignored`，一次性证据工具；直接读流、打印 sherpa raw 文本与 postprocess 结果）。
模型：`registry::SENSEVOICE_SMALL` → MS 镜像 `pengzhendong/sherpa-onnx-sense-voice-zh-en-ja-ko-yue`
（HF 官方 `csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17`，int8 239MB）；
wav：nano 仓 `test_wavs/`（noise_en=en、dia_yue=粤、rag_physics=普通话，16kHz mono）。
本机路径：`~/.config/livetranslate/models/{modelscope/models/pengzhendong--…/snapshots/master, huggingface/hub/models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30/snapshots/main/test_wavs}`。

| 语言设置 | wav | sherpa **raw** 输出（逐字） | postprocess 语言 |
| --- | --- | --- | --- |
| auto | noise_en | `So what's interesting here is I feel that you know, brands knowing this when people sort of speak to the voice assistants at home and if you want to be the brand,` | auto |
| auto | dia_yue | `啲身体好劲啊，跟住咧，佢哋有一个人咧，就突然可能就有高原反应啦，突然间就啊窒息咗，即系晕晕咗。` | auto |
| auto | rag_physics | `根据碰撞理论，月面样本缺少挥发性物质。` | auto |
| **zh**（显式） | rag_physics | 同上（无任何 `<|...|>`） | auto |
| **en**（显式） | noise_en | 同上（无任何 `<|...|>`） | auto |
| **yue**（显式） | dia_yue | 同上（无任何 `<|...|>`） | auto |

**结论**：sherpa-onnx 版 SenseVoice 的解码输出**不含任何特殊标签**（`<|zh|>`/`<|NEUTRAL|>`/`<|Speech|>`…
全部被 decoder 剥离），且与语言设置无关（auto 与显式 zh/en/yue 均无标签）。
`postprocess`（sensevoice.rs:63-94）的 `LANG_TAGS` 分支是实际上的死代码——`detected` 恒 "auto"。

## 3. 根因链

1. **原版不踩坑**：原版 Python（`LiveTranslate/asr_sensevoice.py`，FunASR `AutoModel.generate`）
   输出形如 `<|zh|><|NEUTRAL|><|Speech|><|woitn|>…`，`LANG_MAP` 命中得 zh。
2. **移植保留标签提取**：Rust 1:1 移植 postprocess（原版 LANG_MAP 迭代序、`<\|[^|]+\|>` 语义
   均照搬），但运行时换成 **sherpa-onnx 转换包**——输出的标签源不存在。
3. **单测固化了缺陷行为**：现有 `postprocess_strips_tags_and_detects_language` 断言
   `("<|BGM|><|Speech|>music talking") → ("music talking","auto")`（sensevoice.rs:216-219）——
   忠实于原版语义，但在 sherpa 运行时下它描述的正是恒 auto 的缺陷态。
4. **下游三条链路**（`AsrResult.language` 消费点，均因恒 auto 而失效/错乱）：
   - `commit_text` 同语言判定（pipeline.rs:1378）：`lang == target_language` 恒假 → 白翻译（主诉）；
   - **显式语言过滤**（`reject_segment` pipeline.rs:643 与 `commit_text` pipeline.rs:1345）：
     `asr_language != "auto" && detected_lang != asr_language` 恒真 → **识别语言下拉选 zh/en 时
     一切段被丢弃，转写等效全废**（比主诉更严重，同根因）；
   - 悬浮窗语言徽标（overlay.rs:517）显示 `[auto]`（用户所见「只有 auto」）。

## 4. 方案对比与决策

| 方案 | 内容 | 评价 |
| --- | --- | --- |
| A | `postprocess` 无标签兜底 `guess_language(text)` | 修 auto 主诉；但显式语言（尤其 yue）下语言字段仍与设置无关——显式 yue + 汉字转写 → guess=zh ≠ yue → 过滤全丢，未根治 |
| **A′（推荐）** | `transcribe` 层**三优先**：**显式设置 > 模型标签 > 启发式兜底** | 与 nano 同构（`self.language.clone().unwrap_or_else(|| guess_language)`，nano.rs:140-143）并多一层标签；显式语言时字段与设置一致→过滤必然放行，无自相矛盾；保留标签路径（未来模型变体/其它包装若输出标签仍可区分 yue） |
| B | 管线层兜底（`lang=="auto"` 时在 pipeline 内 guess） | 语言信息属引擎职责（`AsrResult.language` 契约）；管线兜底使四引擎行为不一致、职责错位，不取 |
| C | 给 sherpa 加开关恢复标签 | 无此开关/选项，decoder 已剥离；依赖上游版本不可控，不取 |

**决策**：A′。理由：内聚于引擎层（语言产出是引擎契约的一部分）、与 nano/qwen3 共享同一
`guess_language`、显式语言自洽、标签路径保有未来收益。

## 5. 方案设计（A′）

### 5.1 代码形状（`crates/lt-asr/src/sensevoice.rs`）

```rust
/// 语言最终决策：显式设置 > 模型标签 > 启发式兜底（三优先，SV-1/D-30）
/// - setting=Some：用户显式意图优先（过滤层 `detected_lang != asr_language` 必需自洽）
/// - setting=None 且 tagged != "auto"：模型自报（保留 yue/zh 区分能力）
/// - 否则共享启发式（nano/qwen3 同款：假名>0→ja；谚文>30%→ko；汉字>30%→zh；else en）
fn resolve_language(setting: Option<&str>, tagged: &str, text: &str) -> String {
    setting.map(str::to_owned).unwrap_or_else(|| {
        if tagged != "auto" {
            tagged.to_owned()
        } else {
            guess_language(text)
        }
    })
}
```

- `postprocess` **职责不变**（标签检测 + 标签/事件剥离 + trim + 空→None），仅更新语义注释：
  返回的第二个字段是「模型标签检出值（无标签='auto'）」，最终语言由 `transcribe` 的
  `resolve_language` 决策。
- `transcribe` 接线（sensevoice.rs:172-180）：

```rust
match Self::postprocess(&result.text) {
    None => Ok(AsrResult::default()),
    Some((text, tagged)) => {
        let language = resolve_language(self.language.as_deref(), &tagged, &text);
        Ok(AsrResult { text, language: language.clone(), language_name: language, words: None })
    }
}
```

### 5.2 取舍与已知偏差（落档 D-30）

- **粤语归一 zh**：启发式无 yue（汉字>30%→zh）→ 粤语段落 auto 时报告 zh、目标 zh 免翻译；
  nano 探针既有预期同为 zh（`dia_yue → zh 属预期`，nano.rs:222）。模型自报标签路径（若未来
  输出 `<|yue|>`）仍优先区分。
- **显式 yue**：修复后语言字段=设置值 → 放行（sherpa 以 yue 提示识别，输出汉字转写，正确）。
- **启发式假阳性**：英文段恰含大量汉字（人名/引文）时判 zh——与 nano/qwen3 现有行为一致，
  不新增不一致面；空文本/纯标签由 postprocess → None 短路，不进 resolve。
- 不涉及 lt-proto 契约变更（`AsrResult` 无增改）。

### 5.3 不改动项（复核结论）

- `split_sentences(text, lang)`（interim.rs）：lang 仅 API 兼容形参，语言无关实现（interim.rs
  已知偏差 #1），不影响。
- transcript 落盘（`write_original(id, timestamp, text)`：不含语言），不影响。
- 统计/日志/引擎表格 UI：不消费 `AsrResult.language` 细节。
- 显式语言过滤本身（pipeline.rs:643/1345）**逻辑正确**，是恒 auto 输入让其误伤——修引擎层
  输入来源后无需改过滤。

## 6. 施工卡

| 卡 | 内容 | 文件 | 验收（✅ 落定 2026-09-08，66871a0） |
| --- | --- | --- | --- |
| **SV-1** ✅ | `resolve_language` 纯函数 + postprocess 语义注释更新 + transcribe 接线；单测：resolve_language 全分支（设置优先/标签优先/兜底 zh-en-ja-ko 边界/空 text 不入 resolve）、postprocess 既有断言复核（无标签 → ("…", "auto") 语义保留、标签路径不变、纯标签 None 不变） | `crates/lt-asr/src/sensevoice.rs` | `resolve_language_three_way_priority` 9 断言全绿 |
| **SV-2** ✅ | 实机探针断言固化（验收工具）：auto→noise_en=en、rag_physics=zh、dia_yue=zh；显式→zh=zh / en=en / yue=yue | 同上 probe 模块（ignored） | `probe_real_sensevoice_lang_decision -- --ignored --nocapture` 6 组全绿（7.4s，真实缓存模型 + 真实语音 wav） |
| **SV-3** ✅ | 全量回归 + GUI 冒烟：① auto+目标 zh：中文语音→徽标 `[zh]`、无译文行（免翻译）、无 UpdateTranslation 事件；② 显式 zh：段正常显示（不再全丢）；③ en 语音目标 zh→仍正常翻译；④ 临时切 nano/Qwen3 对照行为一致 | workspace | `cargo test --workspace` 369 全绿（净增 1）/ `cargo build --release -p lt-app` 过 / GUI 冒烟（临时配置目录 + 真实缓存）：worker ready `SenseVoice Small (sensevoice)` + 实测 `Same language (zh), no translation` 反复出现；显式 zh 路径由引擎探针等价覆盖 |

## 7. 偏差登记（决策史）

> **D-30（SenseVoice 语言字段恒 "auto"，2026-09-08）** 见 docs/sensevoice-language-fix.md。
> 行为差异：原版（FunASR）标签驱动、可区分 yue；sherpa-onnx 转换包输出无标签，本版改为
> 「显式设置 > 模型标签 > 启发式」三优先决策，粤语在 auto 下归 zh（启发式无 yue）。
> 阶段二产品化定位：差异不回追原版，以用户体验为准（避免白翻译/全丢两缺陷）。
> 后续新偏差自 **D-31** 起。

## 8. 测试矩阵

| 输入 | 设置 | 修复前 | 修复后 |
| --- | --- | --- | --- |
| 中文语音 | auto，目标 zh | lang=auto；白翻译；徽标 `[auto]` | lang=zh；免翻译；徽标 `[zh]` |
| 中文语音 | zh（显式） | **全丢**（过滤误伤） | lang=zh；放行 |
| 粤语 | auto，目标 zh | lang=auto；白翻译 | lang=zh（兜底）；免翻译 |
| 英文 | auto，目标 en | lang=auto；白翻译 | lang=en；免翻译 |
| 英文 | auto，目标 zh | lang=auto；翻译 | lang=en；翻译（不变） |
| 空/纯标签/纯标点 | auto | None 短路（不产生消息） | 同左（不变） |
| 假想标签输出模型 | auto | zh（标签路径） | zh（标签优先，不变） |
