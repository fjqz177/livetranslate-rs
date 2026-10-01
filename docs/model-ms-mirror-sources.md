# D-132：funasr-nano / qwen3 补 ModelScope 源（用户自建逐字节镜像仓）

> 状态：已定稿并实施（2026-10-02）。决策登记 = `docs/decisions.md` D-132；本文 = 机理与改动面真源。

## 一、背景与取证（2026-10-02，files API + commits API 实测）

用户诉求：两个 HF 官方包（csukuangfj / csukuangfj2）的 README 声称模型来自 ModelScope，希望为这两族模型补 MS 下载源。逐字节核对结论：

1. **zengshuishui 上游仓不可作源**（HF README 所指的 "downloaded from" 来源）：
   - `zengshuishui/FunASR-nano-onnx`：tokenizer 三件与 HF 包一致，但**三个 int8 onnx 与 HF 包不同源**——MS 仓全部历史版本（2025-12-19 初始至 2026-02 重传共 7 个时点）的 sha256 无一与注册表登记值匹配，且布局为 external-data 成对（`*.onnx` + `*.onnx.data`）而 HF 包单文件内嵌。即 README 所说 "downloaded from" 仅对 tokenizer 成立，onnx 是 csukuangfj 用同源脚本自行再导出的另一轮产物。
   - `zengshuishui/Qwen3-ASR-onnx`：六件 6/6 逐字节命中，但该作者有 5 轮重传覆盖前科（funasr 仓 2026-01~02 实录），跟随 master 有静默漂移风险。
2. **jkman2023 现成 qwen3 镜像**（`jkman2023/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`）：6/6 命中 + 布局与 HF 同构 + Apache-2.0 再分发声明 + 单次上传零改动（备选，最终未选用）。
3. **用户自建镜像两仓**（仓 ID 见 `registry.rs` ms 条目）：funasr-nano / qwen3 各 6/6 sha256 命中，布局与 HF 包同构（根目录 onnx + tokenizer 子目录，无嵌套前缀）。
4. **ModelScope 无 sherpa-onnx 官方模型镜像**：k2-fsa / csukuangfj 在 MS 仅有 studios 演示镜像不发模型包（sherpa-onnx GitHub README + MS 站内搜索实证）；whisper 的 MS 镜像同样实测不存在（WD-4 维持）。

## 二、裁决（2026-10-02，用户拍板）

1. **MS 源 = 用户自建逐字节镜像仓**（funasr-nano / qwen3 两族都加，仓 ID 以 registry 为准）；
2. **always_hf 翻 false** = 所选 hub 优先的真双 hub（sensevoice 同款语义）：hub=ms 直连 modelscope.cn，失败回落 HF 走 hf-mirror；hub=hf 反之；
3. **不钉 Revision** 跟随 master（镜像仓用户自控；sha256 校验 + HF 回落双兜底）；
4. **repo_hygiene Tier1 为「用户名/sherpa-onnx-」仓 ID 形态开豁免**（仓 ID 是刻意公开的下载源非个人路径泄漏；目录路径形态仍由 Users 路径规则拦截，豁免不对其开口）。

## 三、改动清单

| 文件 | 改动 |
|---|---|
| `crates/lt-models/src/registry.rs` | 两条目 `ms=Some` + `always_hf=false`；`ms`/`always_hf` 字段注释重写（现仅 whisper 六档为单源）；两条目注释记录 zengshuishui 不可用机理；`funasr_nano_entry_hf_only`/`qwen3_entry_hf_only` 两测试改名 `*_dual_hub` 并翻转编译期 const 不变量 |
| `crates/lt-models/src/cache.rs` | qwen3 探测测试断言翻双 hub（`hub_ms.is_some()`） |
| `crates/lt-app/tests/repo_hygiene.rs` | Tier1 豁免：`ms_repo_re`（`用户名/sherpa-onnx-` 前缀）+ 判定处豁免；新增 `tier1_waives_ms_repo_id_form_only` 单测（仓 ID 形态豁免、目录路径形态仍拦截）；头注补豁免面注记 |
| `docs/decisions.md` | D-132 登记 + D-24 行「→ 被 D-132 修订」注记 |
| `docs/distribution.md` | D-21 行 as-built 勘正（whisper MS 镜像实测不存在由 hf-mirror 端点承接；两族经自建镜像落地） |
| `README.md` / `README.en.md` | 「下载源」说明更新：直连 ModelScope 的 = SenseVoice / Fun-ASR-Nano / Qwen3-ASR，走 hf-mirror 的仅剩 Whisper |
| i18n / lt-ui | 零改动——`model_hf_only_hint` 显示条件从 `entry.ms.is_none()` 派生（vad.rs H10），两族加源后提示自动消失，文案泛指对 whisper 仍准确 |

## 四、影响面与不变量

- **机制零新增**：`hub_chain` 回落链、`local_model_dir` 双 hub 缓存探测、`hf_endpoint_for` 端点随所选 hub——全部 sensevoice 先例现成；MS 布局与 HF 同构故两 hub 缓存互认。无 lt-proto 契约变更，PROTO_VERSION 不动。
- **行为变化仅一处**：hub=ms 时两模型从「hf-mirror 优先」变为「ModelScope 直连优先」；hub=hf 行为不变。
- **诚实原则保持**（D-24 精神）：ms 字段只登记经 sha256 逐字节核实的仓；坏字节场景 = Checksum 快速失败删现场 + 回落下源，截断/污染文件进不了缓存。
- **归档文档不改**（史档快照只读）：`docs/archive/asr-engine-expansion.md` 的 D-24 时点结论以本文 + decisions.md 注记为准。

## 五、验收

- `cargo test --workspace` 全绿（测试数 = 基线 +1：重命名 2 + 新增 1）。
- 实机走查项挂 GitHub issue：hub=ms 两模型真网络直连下载、hub=hf 行为回归、MS 源故障回落、`model_hf_only_hint` 消失确认。
