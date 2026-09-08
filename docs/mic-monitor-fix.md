# 麦克风输入开关与监视条（MIC/RMS/VAD）改造计划

> 2026-09-08 用户反馈：启动后监视条只有 RMS/VAD 两根，一旦系统发出任何声音就会"多出"一根 MIC 条；
> 且"接受用户麦克风输入"的开关"好像没做"。本文给出证据链、根因、原版对照与改造方案。
> 施工卡编号起 MC-1（避免与 WD/ASR 扩展/AH 等既有序列混淆）；行为差异落档 **D-29**。

## 1. 现象与用户诉求

- 现象：正常启动、系统静音时 → 监视条两根（`RMS:`、`VAD:`）；系统任何声音（游戏/视频）响起后 → 出现第三根 `MIC` 条，此后常驻，即使麦克风侧实际无人说话（MIC 值≈1%，见用户截图二）。
- 诉求：① MIC 条不应幽灵般"被系统声音唤出"；② 想要一个明确的"是否接受用户麦克风输入"的开关（当前未感知到）。

## 2. 证据链（逐环核实）

### 2.1 用户配置：麦克风实际处于开启状态

`~/.config/livetranslate/settings.json`：

```json
{ "audio_device": null, "mic_device": "__default__" }
```

`mic_device: "__default__"` = UI"麦克风=系统默认"（约定见 `lt-proto/src/settings.rs:64`：`None=禁用 | "__default__" | 名`）。
代码中无任何"默认写入 `__default__`"的路径（全仓 grep 仅有 UI 选项 `mic_setting_for(1)` 与命令臂 `Cmd::SetMicDevice` 写入），故该值来自用户在 UI 下拉选择了"系统默认"。

### 2.2 Rust 采集侧：Mic 流启动即打开、静音期照常累积

- `lt-pipeline/src/audio/wasapi_win.rs:383-387`：`if requested_mic.is_some() { mic = open_mic(...) }` → 用户配置命中，**程序一启动麦克风流就已在采集**。
- `wasapi_win.rs:512-517`：loopback 无新数据（系统静音）→ `handle_mic`（照常把 mic 数据排入 `mic_buf`，见 `drain_mic` wasapi_win.rs:339-355）→ `sleep(5ms) continue`，**不产出 chunk**。
- `wasapi_win.rs:522-527`：mic 混合只发生在有 loopback chunk 的轮次（`for chunk in produced`），`mix_with_mic`（`audio/mod.rs:117-133`）在 `mic_buf` 非空时返回 `Some(mic_rms)`。
- 静音期 `mic_buf` 持续累积背景噪声：**系统第一次出声时，首个 chunk 的 `mic_rms` 立即为 `Some(≈环境噪声 RMS)`**。

### 2.3 事件侧：UpdateMonitor 只在有 chunk 时发送

- `lt-pipeline/src/audio/capture.rs:132-139`：`Some((chunk, mic_rms)) => { ... (self.monitor)(r, last_confidence, mic_rms); }` → monitor 回调（→ `UiEvent::UpdateMonitor`，`lt-app/src/pipeline.rs:400-406`）**只在消费到 chunk 时触发**；超时分支（capture.rs:114-131，静音推进）不发 monitor。
- 系统静音 → 无 chunk → 无 `UpdateMonitor` → UI 侧 `monitor.mic_rms` 停留在初始值。

### 2.4 UI 侧：MIC 条显隐 = `mic_rms.is_some()`（信息迟到的根源）

- `lt-ui/src/state.rs:82-91`：`MonitorData` 默认 `mic_rms: None`（`#[derive(Default)]`）——启动即无 MIC 条。
- `lt-ui/src/windows/overlay.rs:342-345`：`let n_bars = if state.monitor.mic_rms.is_some() { 3.0 } else { 2.0 }; if let Some(mic) = state.monitor.mic_rms { level_bar(ui, "MIC", ...) }`。
- 于是：**启动（静音）→ mic_rms=None → 无 MIC 条；系统首次出声 → 首个 UpdateMonitor{mic_rms: Some(…)} → MIC 条出现，且此后每 chunk 都带 Some → 常驻**。用户观察到的"系统声音唤出 MIC 条"实为"首个 chunk 送来了延迟的真相"，而非麦克风被声音触发。

### 2.5 与原版对照（原版同款行为，Rust 为 1:1 复刻）

| 环节 | Python 原版 | Rust 版 | 结论 |
|---|---|---|---|
| mic 流开启条件 | `audio_capture.py:417` `if self._mic_device_name: self._open_mic_stream()`（None=禁用） | `wasapi_win.rs:383` `if requested_mic.is_some()` | 一致（默认均为禁用：原版 `config.yaml`/`user_settings.json` 无 `mic_device` 键且 `control_panel.py:352-359` 无值时下拉停在"禁用"；Rust `settings.rs:108` `mic_device: None`） |
| mic_rms 计算 | `audio_capture.py:390-403` `if len(self._mic_buf) > 0:` 才算，否则 `mic_rms = None` | `audio/mod.rs:117-121` `mic_buf.is_empty()` 时返回 None | 一致 |
| MIC 条显隐 | `subtitle_overlay.py:474-482` `mic_active = mic_rms is not None` → `setVisible` | `overlay.rs:342-345` `mic_rms.is_some()` | 一致 |
| 事件发送时机 | `main.py:1644-1652` `chunk, mic_rms = item`（loopback 无数据即不产 item）→ `update_monitor` | `capture.rs:132-139`（无 chunk 不发） | 一致——**原版同样存在"MIC 条首现=系统首次出声"的迟到现象** |
| 开关形式 | `control_panel.py:342-362` 麦克风下拉：禁用/系统默认/设备名 | `vad.rs:519-535` 同款下拉 | 一致——开关以"下拉第一项=禁用"形式存在 |

**结论**：这不是回归，而是原版行为被忠实复刻 + 用户配置开启了麦克风。但该行为存在两个真实的设计缺陷（下述），且用户对开关存在性的困惑说明下拉形式的开关不符合阶段二"产品体验为准"的定位。

## 3. 根因

1. **半身位错误（观察层）**：UI 用"`mic_rms` 是否有值"代理"麦克风是否启用"，而 `mic_rms` 的有无取决于"是否已有 chunk 送达"，与启用状态解耦 → 信息迟到，产生幽灵条。
2. **状态语义混叠**：`UpdateMonitor.mic_rms: Option<f32>`（`lt-proto/src/events.rs:39`）单字段承载三重语义——禁用 ↔ 启用但尚无数据 ↔ 启用且有数值，UI 无法区分，也无法在启动时准确显示。
3. **产品层**：麦克风开关藏于设备下拉（"禁用"项），无显式启停控件、无启用状态提示、无设备打开失败反馈（`wasapi_win.rs:385-386/440-441` 失败仅 warn 日志，UI 无感知）；且易与"扬声器"下拉（语义相反：None=默认、`__disabled__`=禁用，`vad.rs:147-172` 注释自证）混淆。

## 4. 衍生隐患（顺带修复，实测可复现）

- **mic_buf 无上限增长**：`drain_mic`（wasapi_win.rs:339-355）无上限 `mic_buf.extend(...)`；静音期只进不出（512-517 行循环仍在 `handle_mic` 排入，但仅在 loopback 有 chunk 时消费）。16k mono f32 ≈ 64KB/s——无声 8h ≈ 1.8GB 内存（纯泄漏性增长，VAD/混音不受影响）。原版同款（`audio_capture.py:382` `np.concatenate`），按阶段二规范 Rust 侧独立修复，不回追原版。
- **设备打开失败零反馈**：`open_mic` 失败 → `mic=None` → MC 条要么不出现（禁用观感）要么出现后恒 0%（开启观感），与真实状态脱节，无可观测提示（AH-7 日志外无 UI 面）。

## 5. 改造方案

### 5.1 决策（DEC）

- **DEC-1（推荐，主案）**：MIC 条显隐改由"**启用意图**"驱动，即 `state.settings.mic_device.is_some()`；`mic_rms` 仅作数值（`unwrap_or(0.0)`）。**零契约变更、零后端改动**：UI 本就有 settings 镜像（`shell.rs:191` 同步 `state.app_state.settings.mic_device`），命令发出即生效，启动/切换/禁用均立即呈现正确显隐。
  - 优点：消除幽灵条（启用则启动即显示，禁用则永不显示）；改动面小（overlay.rs 一处 + 测试）。
  - 已知取舍：设备打开失败时 UI 仍显示"启用"观感（恒 0% 条）——以 `open_mic` 失败 warn 日志兜底，若后续要失败提示再走 DEC-2。
- **DEC-2（备选，本轮不做）**：契约加字段 `UpdateMonitor { ..., mic_active: bool }` + 后端在 SetMic/SetDevice/启动时立即上报状态。优点：真实状态闭环（含打开失败/掉线）；代价：`lt-proto` 冻结契约评审（AGENTS：结构字段增删需评审）+ 三库联动 + 测试面扩大。留作"设备失败提示"需求出现时的升级路径。
- **DEC-3**：MIC 显式开关形式 = **「启用麦克风输入」勾选框 + 设备下拉（未勾选置灰）**；双主控共用 `mic_device` 键（不新增设置）。
  - 勾选=保留/设置 `__default__`；取消=写 `None`（禁用）。
  - 默认仍为禁用（`mic_device` 默认 `None`，现状本就如此，不引入行为回退）。
- **DEC-4**：mic_buf 上限 = 10s（`16000*10` 样本），drain 后超限从**最旧**丢弃。后果：长静音后恢复消费的头几秒内，混入的 mic 分量时序错位至多 10s——仅影响混音相位，不影响 VAD 判定（VAD 输入为混合信号，silero 对亚秒级相位差不敏感；且仅发生在"静音 ≥min_speech_duration 后突然开口"的罕见时刻，以文档级注记留档）。
- **DEC-5**：行为差异落档 D-29："MIC 条显隐以 UI 启用意图驱动（原版以 mic_rms 是否有值驱动）"；"loopback 静音期 mic_buf 有界化（原版无限累积）"。

### 5.2 施工卡

#### MC-1 MIC 条显隐源切换（DEC-1，主案）

- 文件：`crates/lt-ui/src/windows/overlay.rs:338-349`
- 改动：
  ```rust
  let mic_active = state.settings.mic_device.is_some();
  let n_bars = if mic_active { 3.0 } else { 2.0 };
  if mic_active {
      let mic = state.monitor.mic_rms.unwrap_or(0.0);
      level_bar(ui, "MIC", mic, BAR_MIC, opa_pct, bar_w);
  }
  ```
- 同步修正行 340 注释（"原版：mic_rms 有值才出现"→ 改为"D-29：按启用意图驱动；mic_rms 仅数值"）。
- 验收：
  - `mic_device=None`：MIC 条恒不显示（含系统出声）。
  - `mic_device=Some(_)`：启动即显示；系统出声仅影响数值。
  - 运行中下拉切"禁用"→ 下一帧 MIC 条消失（`apply_mic_setting` vad.rs:1023 立即更新 settings 镜像）。

#### MC-2 mic_buf 有界化（DEC-4）

- 文件：`crates/lt-pipeline/src/audio/wasapi_win.rs`（`drain_mic` 或调用侧）
- 改动：`drain_mic` 返回后 / `handle_mic` 内，`if mic_buf.len() > MAX_MIC_BUF { mic_buf.drain(..mic_buf.len() - MAX_MIC_BUF); }`；常量 `MAX_MIC_BUF = TARGET_RATE as usize * 10`（10s）。
- 注意 `handle_mic` 是闭包（捕获 `mic_buf: &mut Vec`），上限逻辑放闭包内；`SetDevice/SetMic` 臂已有 `mic_buf.clear()` 无需改。
- 验收：新增单元/集成测试——注 30s 量数据后断言 `mic_buf.len() <= MAX_MIC_BUF` 且尾段数据未被破坏（最旧丢弃）。

#### MC-3 显式开关（DEC-3，UI 主卡）

- 文件：`crates/lt-ui/src/windows/panel/vad.rs`（设备组 519-535 一段）+ i18n 双 yaml
- 改动：
  - 新增 i18n 键 `mic_enable`（zh："启用麦克风输入" / en："Enable microphone input"）；zh/en 两份同步（521 键基线 → 522）。
  - 设备组"麦克风"行改为 `Checkbox::new(状态, t("mic_enable"))` + `ComboBox`（系统默认/设备名；未勾选时 `ui.add_enabled(false, ...)` 置灰）。
  - 勾选/取消处理复用 `apply_mic_setting`：`Some("__default__")` / `None`，并适配 `mic_index_for` 现有约定（勾选时下拉索引=1 起）。
  - 说明行注明：取消勾选后麦克风音频不参与混音与转写（原版 `mic_disabled` 语义）。
- 验收：
  - 默认未勾选（`mic_device=None`）；勾选→下拉可交互（默认"系统默认"）；取消→下拉置灰且值写 None。
  - `restore_vad_page`（vad.rs:900-931）行为不变（恢复默认=None/未勾选），必要时同步更新其逐条命令注释。

#### MC-4 测试与基线

- 单测：`overlay.rs` 抽 `mic_bar_visible(settings) -> bool` 纯函数并测三态（None / __default__ / 具名）；`vad.rs` 现有 `mic_device_semantics_roundtrip`（1220-1237）扩充"显式开关勾选/取消 → mic_setting_for 输出"断言。
- 集成：`lt-pipeline` 侧已有 `mix_with_mic` 空 buf → None 测试（audio/mod.rs:340-344）；补充 MC-2 上限测试。
- 全量：`cargo test --workspace` 收工前提（当前基线 363 测全绿 + 5 ignored）。
- 实机（GUI 冒烟，`LIVETRANSLATE_CONFIG_DIR` + 真模型缓存）：
  - A：禁用态放系统声音 ≥30s → MIC 条恒不出现；
  - B：勾选"系统默认"启动 → MIC 条启动即出现；
  - C：运行中取消勾选 → 条≤1 帧内消失，翻译混音无可闻变化（麦克风不再混入）；
  - D：无声长跑 ≥1h → RSS 平稳（无 mic_buf 泄漏增长），期间麦克风保持开启。

### 5.3 回归注意

- 布局：`n_bars` 影响 `bar_w` 计算（overlay.rs:343），MIC 条显隐切换时三根/两根自适应——与本改造无关的既有逻辑，勿动。
- 契约：本轮**不改** `lt-proto`（events.rs:39 保持）；若走 DEC-2 再启动评审。
- 原版参照：`LiveTranslate/` 副本（gitignored）仅参考；本改造按阶段二产品定位偏离原版，偏离点即 D-29，不回追。

## 6. 待确认（实机/用户侧）

1. 用户侧设置：`mic_device` 为 `__default__` 是何时/何操作写入的（UI 下拉选过"系统默认"？）——不影响方案，仅记录决策史。
2. 无声时段 MIC 条数值样式：启用未收到任何 chunk 时显示 0%（而非隐藏）——MC-1 已覆盖，实机 A/B 确认观感。
