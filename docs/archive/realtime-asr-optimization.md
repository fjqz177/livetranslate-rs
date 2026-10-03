# 实时识别深度优化（D-133）

> **状态**：已归档（2026-10-03 被 D-134 取代——实时通道整体裁撤，本文降级留档仅作证据链：①~④⑥ 语义随通道废止，⑤收尾短句跨段 pending 状态机迁 `interim.rs` 幸存，见 `docs/decisions.md` D-134）。原定稿记录：用户开工令「开分支专门优化实时识别，一整套完整功能逻辑深度优化」；施工分支 `feature/realtime-asr-optimization`。方案调研稿 = `docs/drafts/realtime-asr-ux-overhaul.md`（不入库调研底稿，含外部 SOTA 全文与拍板清单 ①-⑦）；**拍板结果全按推荐执行，其中 A5 按施工前复核微调**（见 §6）。
> **范围**：A 包六项（伪流式机制内的全套逻辑与参数重建）。B 路线（真流式引擎级联）按拍板⑥押后至实机走查后再议。

## 0. 背景与证据摘要（为何改）

用户实机取证（2026-10-01~02 日志 + 转录 + settings.json）与本机探针首跑（D-128 欠账）结论：

1. **节拍倍率表错标**：探针实测（`realtime_throttle_probe`，合成缓冲）SenseVoice 8s=435ms（RTF 0.054）、qwen3-0.6B-int8 8s=**587ms（RTF 0.073）**——与 SenseVoice 几乎同级。现役表 `realtime_tick_scale`（lt-proto/settings.rs:60）qwen3=3.0 保守 3 倍：默认基准 1s 下实际节拍 3s，按用户存盘设置（qwen3 × 3.1）重开实时 = 9.3s 一刷，灰显行近乎冻结。
2. **盲裁剪产伤口**：裁剪点 = 定稿字符占比 × 缓冲 + 0.3s（`interim.rs:262`），不看音频内容 → 跨行断词（"the ferra.|stays ahead"）、悬垂碎片（"Can."）、同刻重复畸形行。
3. **partial 整行闪变**：零门控下每 tick 整段重写，LLM 解码型引擎前文也会被改——观感是「闪变」不是「逐字」（与用户愿景「流式逐字实时输出」冲突）。
4. **碎片行无闸**：句内停顿兜底 0.5s 在解说语流下切碎半句；收尾路径（实时关/收尾段）对 ≤8 字符碎片直接提交（"P."、"1."、"W degree." 成行）。
5. **可观测性缺失**：tick 耗时/丢弃率无账，走查只能数「定稿」日志。

目标体验指标（M1-M7 详表见调研稿 §3）：partial 首显 ≤1.5s、刷新粒度 ≤1.5s、已显示前缀基本不动、定稿碎片率 <5%、截词 ≈0、说话期 CPU 增量 ≤0.7 核。

## 1. 改动总览

| 项 | 内容 | 落点 |
|---|---|---|
| A1 | 倍率校准 + 运行时 EMA 自适应退避 | lt-proto / lt-audio(capture) / lt-orchestrator |
| A2 | VAD 概率谷对齐裁剪（宁早勿晚） | lt-audio(vad/realtime) / lt-orchestrator |
| A3 | 显示层稳定前缀（弱化 D-128 零门控） | lt-audio(realtime) |
| A4 | 停顿兜底阈值 0.5→0.7s | lt-audio(realtime) |
| A5 | 收尾路径短句跨段 pending | lt-audio(realtime) / lt-orchestrator |
| A6 | tick 心跳可观测性 | lt-orchestrator / lt-audio(capture) |

**不做面**：B 路线级联（押后）；噪声闸收紧（A5 的 pending 已覆盖且保内容，收紧反丢 "OK" 类真短句）；「清空」打断后端（维持 D-113 既有语义——后端在途/暂存内容本就允许在清空后落板，flush_pending 最迟 3s 冲刷属同容差）；新 Cmd 变体；字幕窗数据源（Q10 拍板不动）；worker 协议 / VAD 主干 / 四引擎 / 多语种（零触碰）。

## 2. A1 节拍校准 + EMA 自适应退避

- **静态表校准**（`lt_proto::realtime_tick_scale`）：funasr **1.0**（不变）/ whisper **1.5→2.0**（medium 档 CPU RTF 社区共识 ≈1 边缘，1.5s 节拍长缓冲下跑不动）/ qwen3 **3.0→1.5**（实测 587ms@8s，1.5s 节拍占核 ~39% 留余量）。
- **运行时 EMA 退避**（机械防线）：`InterimControl` 增 `ema_asr_ms: AtomicU32`（0=未知）；ASR 线程每 tick 识别后回报耗时，α=1/8 指数平滑写入。capture 触发判定取 `effective_interval = max(基准×倍率, 1.2×EMA)`——识别慢于节拍自动拉长间隔，不空转烧 CPU、不积压段队列。治 whisper 类「静态标定追不上机器差异」的结构病。
- **面板诚实显示**：倍率 >1 的提示语补「识别繁忙时自动拉长」语义（zh/en 同步改）。
- 测试：EMA 平滑数学、effective 取 max 语义、interim_due 与退避联动。

## 3. A2 VAD 概率谷对齐裁剪

- **新增 VAD 观测口**：`VadProcessor::lowest_confidence_boundary_in(from_samples, to_samples) -> Option<(usize, f64)>`——按 `speech_buffer` 实际 chunk 累计边界走（与 `confidence_history` 严格同索引，trim_front 的部分裁剪分支保持一 chunk 一置信不变量），返回窗内语音概率最低 chunk 的**结束样本边界**与概率。
- **对齐纯函数**（realtime.rs）：`align_trim_to_valley(estimate, total, valley, sample_rate)`——谷点满足「≤ 估计点+0.3s」且「谷置信 ≤ 0.5（`VALLEY_MAX_CONF`）」且「≥0.15s」才采用，否则维持现估计值。
- **宁早勿晚原则**：早切 → 已定稿文本的尾部音频残留缓冲，下次识别重复出词由既有回声剥离吃掉；晚切 → 切进未定稿词中间产生新乱码（不可修复）。故谷点晚于估计+0.3s 一律拒绝。
- **管道接线**：`realtime_tick` 在裁剪前同锁搜索窗口 `[估计−1.5s, 估计+0.3s]`，将谷点交给对齐函数后 `trim_front_checked`（代际校验不变）。
- 测试：真值表（有谷采用/无谷回退/谷太晚拒绝/谷置信太高拒绝/下限保护）。

## 4. A3 显示层稳定前缀（D-128「零门控」弱化，已登记修订注记）

- `RealtimeState` 增 `prev_display`（上一 tick 原始显示候选）/ `agreed`（锁定前缀）/ `divergence`（连续分歧计数）。
- 算法（realtime.rs 内纯函数）：每 tick 取「新I上一假设的最长公共字符前缀」；前缀以旧锁定为前导则**延伸锁定**；假设不再以锁定开头则分歧计数 +1，**连续 2 拍**分歧才接受重写（防单帧抖动拆锁）；显示 = 锁定前缀 + 当前假设的余段（不兼容帧回退整段显示 = 现行为）。
- 定稿非空的 tick（缓冲被裁、显示基础变更）直接重置锁定，下拍重新积累。
- 语义边界：最坏情况 = 现行为（整段重写）；最好情况 = 前文静止、尾部逐字生长。定稿语义（句界三信号）完全不变。
- 测试：锁定延伸 / 分歧保持 / 连续分歧重写 / 定稿重置 / 清空复位真值表。

## 5. A4 停顿兜底阈值 0.5→0.7s

- `PAUSE_FINALIZE_SECS` 0.5→0.7：0.5s 在解说语流（换气/主播交替）下频繁把半句转正成碎片；0.7s 在「半句转正及时性」与「切碎率」间取中。auto 档 VAD 收段静音 0.8s 仍留 0.1s 兜底窗；fixed 档静音 ≤0.7s 时收段先行、语义不受损（同现行注释逻辑）。
- 相应真值表测试数值同步更新（0.6s 用例改 0.75s、0.49/0.5 边界改 0.69/0.7）。
- 走查项 5 的预期（停顿 ~0.6s 转正）按新值 0.7s 执行。

## 6. A5 收尾路径短句跨段 pending（施工前复核微调）

- **微调说明**：调研稿 A5 原案 =「收紧噪声闸 + 跨段 pending」；施工前复核确认 pending 已完整覆盖碎片面且**保留内容**（"OK" 类真短句不丢），噪声闸收紧成为冗余且引入误杀面——只做 pending，不动 `reject_segment`。
- `RealtimeState` 增 `flush_pending: String` + `flush_pending_at_ms: u64`（0=空；epoch ms 口径对齐 `InterimControl.last_check_ms` 先例，保 `Eq` 派生）。
- 语义：收尾路径（`handle_vad_flush` 非 interim 臂）识别文本 ≤8 字母数字（`is_short_utterance` 同闸）→ 并入 `flush_pending` 暂不下发；下一句定稿（实时 tick 定稿 / 收尾提交 / interim final）**前置合并**随行；**3s 无后续独立成行**（ASR 线程空闲臂检查超时冲刷；退出收尾 drain 后必冲刷——内容不蒸发）。
- 两级复位：`reset_segment()`（每收尾段后调用）保留 flush_pending（跨段语义）；`reset()`（会话边界：设备切换/引擎切换）全清。
- 测试：暂存不丢 / 前置合并顺序 / 超时独立成行 / 两级复位差异。

## 7. A6 tick 心跳可观测性

- `realtime_tick` 返回 `TickReport { ran, asr_ms, discarded, failed, finalized }`；ASR 线程聚合计数（ticks / 代际丢弃 / 识别失败 / 定稿句数）+ EMA 回报 `InterimControl`。
- 每 30s 有活动则 INFO 一条心跳（计数 + EMA 耗时），零活动不刷屏。
- `SetRealtimeAsr` 应用日志补当前引擎生效倍率。

## 8. 验收与走查

- 门禁：`cargo test --workspace` 全绿（695 基线 + 新增）+ `precommit.ps1` 三项 + clippy 0。
- 探针复跑：`realtime_throttle_probe` 数字回填本档 §0（已完成，2026-10-02）。
- 实机走查清单挂规格 issue（M1-M5 指标逐条 + 原 #39 清单按新参数重走：停顿转正预期 0.7s）。

## 9. 证据链（复现命令）

```bash
# 节拍探针（本机有模型缓存即可复跑）
LIVETRANSLATE_CONFIG_DIR=<配置目录> cargo test -p lt-asr --test realtime_throttle_probe -- --ignored --nocapture
# 用户实机证据面：~/.config/livetranslate/{logs,transcripts} 2026-10-01~02 会话
```
