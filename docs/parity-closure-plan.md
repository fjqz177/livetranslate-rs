# 与 Python 原版对齐收口施工计划（parity-closure-plan）

> 制定日期：2026-09-06　基线：HEAD `a3ecaf1`（279 测全绿，release 59MB 单 exe 已验证）
> 制定方式：对 `LiveTranslate/` Python 权威副本与 Rust 八库做全量双面清点后逐项定稿。
> **本文档是后续会话的施工依据。** 按 WP（工作包）推进，每个 WP 独立可交付、可测试、可提交。

---

## 0. 使用说明（执行会话必读）

### 0.1 怎么用本文档

- 按优先级顺序推进 WP；P0 之间无依赖可并行，WP-1 与 WP-6 有软依赖（见 WP-1 §依赖）。
- 每个 WP 内含：**现状证据 → 原版参照 → 改造步骤 → 边界 → 完成标准 → 风险**。步骤写到文件与函数级，但 **行号以基线 `a3ecaf1` 为准，执行时以实际代码为准**（先 Read 再改）。
- 遇到「⚑ 决策点」标记：先看 §附A 的建议默认值；若要偏离建议默认值，停下来问用户，不要自行拍板。
- 收尾规约沿用 AGENTS.md：`cargo test --workspace` 全绿 → 中文 commit（`feat(scope): 主题`）→ 提交前 `git status`/`git diff` 复核。多会话并行时**只改本 WP 涉及的文件**。

### 0.2 参照基准口径（三层）

1. **1:1 权威** = 工作区 `LiveTranslate/` Python 副本（用户 2026-09-06 明确）。本文档所有"原版行为"均引该副本的 `file:line`。
2. **已裁决偏差** = RESEARCH.md §1.5 的 D-1～D-16。新偏差一律从 **D-17** 起编号并回写 RESEARCH.md。
3. **历史反转决策**：`docs/ui-realign-plan.md` Phase C 中"首启向导恢复""托盘全量菜单"两结论已被后续 commit 反转（785eed1 拆向导、7d360b1 托盘精简为 5 项，均引"新版原版 less-is-more"）。本文档将这两处列为 ⚑ 决策点重新裁决，**不默认延续反转**。

### 0.3 硬规约（每个 WP 都适用）

| 规约 | 出处 |
|---|---|
| egui repaint 回环：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }` | AGENTS.md 坑 1（头号坑） |
| `set_inner_size` → `request_inner_size`；`ctx.fonts()` 是闭包 API | AGENTS.md 坑 2 |
| lt-ui 内引用 windows crate 必须写 `::windows::`（`crate::windows` 模块遮蔽） | AGENTS.md 坑 3 |
| 双栈 CRT 配置（`.cargo/config.toml`）与 `LIBCLANG_PATH` 勿动 | AGENTS.md 坑 5 |
| **lt-proto 契约冻结**：新增 UI 能力不得扩 `UiEvent`/`Cmd`。本文档所有 WP 已按"零契约扩展"设计；确需扩契约时先停下来论证 | AGENTS.md 分层规则 |
| i18n：`assets/i18n/zh.yaml` 与 `en.yaml` 必须同步改（当前各 510 键） | AGENTS.md 约定 |
| Settings 运行时落盘走 300ms 防抖 `Cmd::ApplySettings` / `Cmd::PersistSettings` | AGENTS.md 分层规则 |
| 实机走查截图用全屏截图（egui 窗口 PrintWindow 抓旧帧） | AGENTS.md 坑 10 |

### 0.4 差距总表（盘点结论速览）

| # | 差距 | 性质 | WP | 优先级 |
|---|---|---|---|---|
| 1 | Whisper 引擎管线已实装但面板灰显不可选；无档位下拉 | UI 未放开（自相矛盾） | WP-1 | P0 |
| 2 | funasr-nano-2512 下拉可选，选中后必坏（下载成功→加载失败） | 用户陷阱 | WP-2 | P0 |
| 3 | interim 增量识别：算法层 37 测完成，装配零接线，复选框无效 | 真缺口（体感最大） | WP-3 | P0 |
| 4 | `Cmd::SetPadding` 无人发送、无人处理，padding 改动不热生效 | 断线（通道已建好） | WP-4 | P0 |
| 5 | 托盘 5 项 vs 原版 ~12 项；气泡通知缺失 | ⚑ 决策点 + 缺口 | WP-5 | P1 |
| 6 | 缺模型无下载门；DownloadMissing 状态机齐全但无人进入；加载框日志空占位 | 断线（状态机已建） | WP-6 | P1 |
| 7 | 字幕窗 6 项自认偏差（bg_image/行级字体/动画/错误文案/工作区） | 混合（逐项裁决） | WP-7 | P2 |
| 8 | ErrorBanner、全局热键（AGENTS.md 待办） | 超出 Python 副本，出处待确认 | WP-8 | P2 |
| 9 | 启动 <2s / 空闲 CPU <1% / 8h 长跑 / 端到端语音 / 内存回收均无实测 | 未验证 | WP-9 | P1 |

已确认**不构成差距**的项（盘点核实，勿重复排查）：悬浮窗 7 按钮/4 复选/3 下拉/两行制/右键复制导出/50 条上限/紧凑模式/穿透 50ms 轮询/位置记忆多屏校验；MonitorBar 三电平+统计行；设置全字段契约 + 原版 json 迁移导入；翻译流式（先带后撤）/thinking 6 风格/重复循环检测/同语言直显；ASR 三层过滤；worker 崩溃重启 + Job Object 孤儿兜底；转写落盘；缓存页/基准测试页/更新日志页；i18n 510 键双语。Remote ASR 裁剪（D-12）与纯 CPU 为既定硬约束。

---

## WP-1 Whisper 引擎 UI 放开（P0）

### 现状与证据

- 管线侧 Whisper **已全链实装**：`build_worker_config` 有 whisper 分支（`crates/lt-app/src/pipeline.rs:607-621`），worker 入口有 whisper 分支（`crates/lt-app/src/main.rs:133-143`），档位解析支持 builtin 六档 + 本地 GGML 路径（`pipeline.rs:559-579`），单测齐全（`pipeline.rs:963-1029`）。
- UI 侧引擎下拉却按过时注释灰显：`crates/lt-ui/src/windows/panel/vad.rs:227-229`（`if *id == "whisper" && !selected` → `add_enabled(false)`），注释 `vad.rs:8-9` 写"whisper 引擎未实装（M5）……随 M5"——**已被 M5.1（commit 6403cb6）推翻，属未清理的灰显**。
- 面板**没有 whisper 档位下拉**（tiny/base/small/medium/large-v3/turbo）：全面板仅 `vad.rs:554,560,663` 读 `whisper_model_size` 做缓存状态显示，无任何写入控件。原版有型号下拉（`LiveTranslate/control_panel.py:385-408`）。
- 模型缓存组已泛化：whisper 引擎下显示缓存状态 + 下载按钮（`vad.rs:545-598`），`model_cache_status` 已支持 whisper 档位判定（`vad.rs:168-184`）。

### 原版行为参照

- 引擎下拉 4 项中 Rust 保留 2 项（D-12 裁 remote、D-13 裁 anime），whisper 可选（`control_panel.py:213-233`）。
- Whisper 型号下拉 6 档 = 5 原版档 + turbo（D-15），large-v3 标注 CPU 慢（RESEARCH.md §5xx 行 500 区域："Whisper 档位下拉 6 项（5 原版档 + turbo，D-15），large-v3 标注 CPU 慢"）。
- padding 条件可见：whisper padding 仅 whisper 引擎可见，SenseVoice padding 仅支持 padding 的 FunASR 模型可见（`control_panel.py:285-317, 1118-1137`）。**Rust 现状两个 padding 恒显示**（`vad.rs:293-302`），本 WP 一并对齐。

### 改造步骤

1. **放开引擎灰显**：`vad.rs:227-234` 删除 whisper 特判分支，whisper 与 funasr 同走 `selectable_label` + `send_switch_engine`；同步更新 `vad.rs:8-9`、`vad.rs:216` 的过时注释。
2. **新增 whisper 档位下拉**：在 `is_funasr` 块（`vad.rs:262-288`）后加 `if engine == "whisper"` 块。档位表 = `lt_models::registry` 的 whisper 条目（6 档，含 turbo；显示名 + large-v3 慢速标注，i18n 键 `whisper_slow_hint` 若无则新增 zh/en）。选中 → 写 `state.settings.whisper_model_size` + `mark_settings_dirty(state)`（档位切换不触发引擎重启，等价原版"仅缓存状态/下载组联动"；是否即时 `send_switch_engine` ⚑ 见步骤 5）。
3. **padding 条件可见**：`vad.rs:290-303` 改为 `if is_funasr { sensevoice padding }` / `if engine == "whisper" { whisper padding }`，对齐原版可见性矩阵。
4. **模型缓存组显示名**：`vad.rs:557-561` whisper 分支的显示名从 `format!("whisper-{}", size)` 改为 registry display（turbo 档避免显示 "whisper-turbo" 歧义，可接受则不动）。
5. ⚑ **档位变更是否即时切引擎**：原版 `whisper_model_size` 变更仅联动下载组不重启引擎（`control_panel.py:1152-1236`），下次引擎启动生效。**建议默认：仅落盘不切换**（与原版一致），用户重新选引擎或重启后生效。
6. **依赖的下载正确性**：whisper 档位下载按钮（`vad.rs:587` 发 `Cmd::StartDownload`）依赖 backend 的 targets——当前是启动时快照（`crates/lt-app/src/main.rs:54-64` + `backend.rs:81` `missing.to_vec()`），运行中改档位后**会下载错误的模型**。本 WP 必须包含 backend 动态化（原属 WP-6，前置到此处）：
   - `backend.rs::spawn` 的循环内，拦截 `Cmd::PersistSettings`/`Cmd::ApplySettings`/`Cmd::SwitchEngine` 三类命令：先用本地 `settings` 镜像更新（前两者带全量 `Box<Settings>`，后者带 engine/funasr_model/whisper_model_size 三键），再照旧转发事件循环。
   - `run_download` 内 `missing.to_vec()`（`backend.rs:81`）改为用镜像 settings 现场重算 `lt_models::cache::missing_models(...)`。**不扩 lt-proto 契约**。

### 边界（不做）

- 不做 whisper 本地 GGML 目录枚举对话框（原版 vad_whisper.py 的本地模型选择器）——`whisper_model_size` 直接接受本地路径、`resolve_whisper_model` 已支持文件直通（`pipeline.rs:568-577`），UI 输入路径入口后续按需另立卡。
- 不动 whisper worker/引擎层任何代码。

### 完成标准

- [ ] 引擎下拉可选 whisper → 触发 `SwitchEngine` → worker 以 whisper 启动（日志 `引擎已切换: whisper/...`，MonitorBar 设备行出现 `Whisper xxx [cpu]`）。
- [ ] 档位下拉 6 项；切换档位后缓存状态行与下载按钮跟随；未缓存点下载 → 下载的正是当前档位（检查 `models/` 目录产物）。
- [ ] padding 可见性：funasr 引擎只见 SenseVoice padding，whisper 引擎只见 whisper padding。
- [ ] `cargo test --workspace` 全绿；`panel/vad.rs` 既有单测（ENGINES 表等）适配后仍绿。
- [ ] 手测：whisper tiny 识别一段音频 → 悬浮窗两行制消息正常、ASR ms 合理。

### 风险与坑

- `send_switch_engine`（panel/mod.rs 公共助手）走 `Cmd::SwitchEngine` → ASR 线程空闲分支 `ReplaceEngine`（`pipeline.rs:727-756`）→ `build_worker_config` None 时发 `AsrUnavailable`——未缓存档位切换会显示 unavailable，属预期（原版切换前先弹下载框，Rust 版由缓存组下载按钮兜底）。
- whisper 首次加载大档位（large-v3 ~3GB GGML）在纯 CPU 下加载慢，加载框（WinId::Setup）会显示——加载框日志接线在 WP-6d，届时体验闭环。

---

## WP-2 FunASR Nano 处置（P0：先防坑，实装另立卡）

### 现状与证据

- 注册表条目齐全（`crates/lt-models/src/registry.rs:35-44`，HF/MS 双 repo、1.1GB 估计、文件清单）；面板下拉**可选**、标"实验性"（`crates/lt-ui/src/windows/panel/vad.rs:62-68`，`enabled: true`）。
- 但 `build_worker_config` 把**所有** funasr 条目一律映射 `engine: "sensevoice"`（`pipeline.rs:593-606`），而 SenseVoice 加载器只认 `model.int8.onnx + tokens.txt`（`crates/lt-asr/src/engines/sensevoice.rs:30-56`）。**选中 nano → 下载 1.1GB 成功 → 加载必然失败**（或 `local_model_dir` 探测失败 → `AsrUnavailable`）。这是用户陷阱。
- AGENTS.md 待办自认"FunASR Nano 实装（目前仅注册表占位）"。RESEARCH.md D-14 的意图是 nano"标实验性**可选**"——即长期正确解是实装，而不是永久置灰；mlt 才是置灰（无上游 ONNX，D-14）。

### 改造步骤（分两步走）

**第 1 步（本 WP，防坑，1 小时）**：
1. `vad.rs::funasr_model_items()` 中 nano 的 `enabled: true → false`（对齐 mlt 的 D-14 灰显样式 `vad.rs:70,273-275`），display 保留"实验性"标注并追加"实装中"提示（新 i18n 键 `model_nano_disabled_hint`，zh/en 同步）。
2. `vad.rs:285-287` 的 mlt hint 行扩展为按禁用项分别提示（mlt：待上游转换；nano：引擎实装中）。
3. 回写 RESEARCH.md：D-14 修订记录"nano 短期置灰（引擎未实装防误选），实装后恢复可选"。

**第 2 步（独立卡，不在本计划排期，前置调研后立项）**：
- 调研 sherpa-onnx（`sherpa-rs` binding）对 Fun-ASR-Nano 的模型类型支持面：binding 是否暴露 nano 所需的 recognizer 配置（`Cargo.toml` 当前 sherpa 版本、上游 sherpa-onnx 是否已支持该模型 type）。调研结论决定：binding 支持 → 立"Nano 引擎实装"卡（新 `lt-asr::engines::nano`，`build_worker_config` 按 registry 条目分派引擎，`asr_worker_entry` 加分支）；不支持 → nano 维持置灰并记为正式偏差。
- 实装时注意 registry 文件清单中内嵌 Qwen3 权重（原版需单独补下，`LiveTranslate/model_manager.py:416-422`）——Rust 版 D-14 已裁决"不移植 Qwen3 下载链路"，若 nano ONNX 转换不含该权重则无此问题，调研时确认。

### 完成标准（第 1 步）

- [ ] nano 在下拉中灰显不可选，提示文案双语齐。
- [ ] `funasr_model_items` 单测更新（enabled 断言）。
- [ ] settings.json 里遗留 `funasr_model: "funasr-nano-2512"` 的用户启动时回退路径不变（`resolve_funasr_entry` 对 nano 返回 Some 不回退——**核对**：nano 置灰后，已存 nano 设置的启动仍会尝试装配。需在 `resolve_funasr_entry`（`pipeline.rs:546-554`）把 nano 加入回退分支或维持现状，⚑ 建议：加入回退 + warn 日志，与 mlt 同款）。

---

## WP-3 interim 增量识别接线（P0，体感差距最大）

### 现状与证据

- **算法层完成**：`crates/lt-pipeline/src/interim.rs` 纯函数 + `InterimState` 容器，37 测，头注释（`interim.rs:1-40`）明确移植映射与"不做线程/定时/队列装配"的边界。自写分句器对 pysbd 的 6 条偏差已在案（`interim.rs:16-30`），勿重复定界。
- **VAD 预留接口已备**：`peek_buffer`（`crates/lt-pipeline/src/vad.rs:587`）/`trim_front`（`:597`）/`force_flush`（`:625`）。缺 `speech_samples` 公开 getter（字段私有，`vad.rs:230`）。
- **段来源枚举已预留**：`SegmentSource { VadFlush, InterimFinal }`（`crates/lt-pipeline/src/lib.rs:20-25`），但 `InterimFinal` 无人生产（`pipeline.rs:703` 注释"M6 interim 接入后再分流"；`pipeline.rs:761` `let _ = source;`）。
- **capture 线程独占 VAD**：`Pipeline::start` 构造 `let mut vad` 后 move 进 `CaptureLoop::run(&mut vad, &stop)`（`crates/lt-app/src/pipeline.rs:339-373`；`crates/lt-pipeline/src/audio/capture.rs:33`）——无共享拓扑。
- **UI 复选框落盘但不生效**：`vad.rs:519-526`（自认注释 `vad.rs:518`"热应用命令待 M4.4 接线，本批先落盘"）；`Cmd::IncrementalAsr { enabled, interval }` 在契约中（`crates/lt-proto/src/events.rs:89`）但 `shell.rs:184-185` 落入未处理分支，且**无人发送**。

### 原版行为（权威语义，逐条对照）

拓扑（`LiveTranslate/main.py`）——**与直觉不同，务必按此接线**：

1. **capture 线程只做触发判定**（`main.py:1657-1668`）：`process_chunk` 返回 None（未成段）且 `incremental_enabled && _asr_ready && _vad._is_speaking` 且 `total_dur >= interim_interval && elapsed >= interim_interval && cooldown >= 1.0s`（`total_dur = _speech_samples/16000`；`elapsed = (_speech_samples - _last_interim_samples)/16000`）→ `_enqueue_asr("interim", None)`——**往 ASR 队列塞空标记，不在 capture 线程跑 ASR**。
2. **ASR 线程消费标记**（`main.py:1720-1724`）：先 `_drain_interim_duplicates()`（排空队列中连续 interim 项，防积压，`main.py:1726-1734`）→ `_do_interim_asr()` → 更新 `_last_interim_samples = _vad._speech_samples`（经 `_vad_lock`）。
3. **`_do_interim_asr`**（`main.py:1411-1527`，在 ASR 线程）：锁 `_vad_lock` → `peek_buffer`；时长 <1.5s 直接返回；`use_word_ts=False`；ASR 识别 → 回声剥离（`_strip_committed_overlap`）→ 分句 → **只取前 n-1 句**（`sentences[:-1]`，末句仍在说）→ 短句（≤8 字母数字）进 `_interim_pending` 缓存拼接 → 完整句走 `_process_segment_text`（**正常消息流**：语言过滤 → add_message → 翻译，`main.py:1529-1548`）→ 按比例裁剪 `trim_front`（+0.3s 余量、保底 0.5s、防循环下限，`main.py:1485-1495`——interim.rs `trim_samples` 已逐字移植）→ 更新 `_interim_committed_tail`（尾部 50 字符）。
4. **vad_flush 到达时**（`main.py:1710-1719`）：`_interim_active` 为真 → `_process_interim_final`（识别后文本经回声剥离 + pending 前置拼接 + 噪声过滤后照常提交，`main.py:1554-1621`）；否则普通 `_process_segment`。随后**重置全部 interim 状态**（`_interim_active/_interim_pending/_last_interim_samples/_last_interim_check_time/_interim_committed_tail`）。
5. 停止/引擎切换时 flush 残余（`main.py:1208-1221`）——Rust 现状 stop 直接丢队列，见"边界"。

### 改造步骤

**A. VAD 共享拓扑（lt-pipeline + lt-app 装配）**

1. `lib.rs:20-25`：`SegmentSource` 改为 `{ VadFlush, Interim }`（删除无人使用的 `InterimFinal`；先 grep 确认仅 `pipeline.rs:761` 与 capture 测试引用）。语义对齐原版 seg_type：`VadFlush`="vad_flush"（携段音频），`Interim`="interim"（空 `Vec<f32>` 标记）。
2. `vad.rs`：新增 `pub fn speech_samples(&self) -> usize`（读 `speech_samples` 字段）。
3. `capture.rs`：
   - `CaptureLoop` 增字段 `pub interim: Arc<InterimControl>`；
   - 新增（`lib.rs` 导出）：
     ```rust
     pub struct InterimControl {
         pub enabled: AtomicBool,
         /// f32 bits（AtomicU32 存 f32::to_bits）；0 = 未设
         pub interval_bits: AtomicU32,
         /// 原版 _last_interim_samples（capture 读、ASR 写）
         pub last_interim_samples: AtomicU64,
         /// 原版 _last_interim_check_time：毫秒时间戳（UNIX epoch 毫秒，AtomicU64；0=never）
         pub last_check_ms: AtomicU64,
     }
     ```
     （用原子而非 Mutex：原版这四个状态就是散字段，capture/ASR 两线程各写各的，无复合不变量。）
   - `run` 签名改：`pub fn run<C: ConfidenceSource + Send + 'static>(&self, vad: Arc<Mutex<VadProcessor<C>>>, running: &AtomicBool)`；体内三处 `vad.xxx()` 改为拿锁调用（`process_chunk`×2、`is_speaking`/`effective_silence_limit`/`last_confidence`/`update_settings`）。锁粒度 = 单次方法调用，与原版 `_vad_lock` 同粒度（原版在 `main.py:1637,1654` 即如此）。
   - `process_chunk` 返回 None 分支（现 `capture.rs:61-63` 的 else）插入触发判定（对照 `main.py:1657-1668`）：
     ```rust
     } else if self.interim.enabled.load(Ordering::Relaxed) {
         let interval = f32::from_bits(self.interim.interval_bits.load(Ordering::Relaxed));
         let (total, speaking) = { let v = vad.lock().unwrap(); (v.speech_samples(), v.is_speaking()) };
         let elapsed = total.saturating_sub(self.interim.last_interim_samples.load(Ordering::Relaxed));
         let now_ms = now_epoch_ms();
         let cooldown_ok = now_ms - self.interim.last_check_ms.load(Ordering::Relaxed) >= 1_000;
         if speaking && interval >= 1.0
             && total as f32 / 16000.0 >= interval
             && elapsed as f32 / 16000.0 >= interval
             && cooldown_ok {
             self.interim.last_check_ms.store(now_ms, Ordering::Relaxed);
             self.segment_tx.push((SegmentSource::Interim, Vec::new()));
         }
     }
     ```
     （原版 `_asr_ready` 条件不搬：Rust ASR 线程无引擎时本就吞段待命 `pipeline.rs:681-686`，无害。）
4. `pipeline.rs::Pipeline::start`：`let vad = Arc::new(std::sync::Mutex::new(VadProcessor::new(...)))`；capture 线程与 ASR 线程各 move 一个克隆；`Pipeline` 结构体存 `interim: Arc<InterimControl>`；`stop()` 时无需额外处理。
5. `ConfidenceSource` 需 `Send`（SileroVad 的 ORT session 跨线程 move 进 Mutex——编译器会验证；ORT session 是 Send）。

**B. ASR 线程分流（lt-app::pipeline）**

6. `run_asr_thread` 签名加参数 `vad: Arc<Mutex<VadProcessor<...>>>`、`interim: Arc<InterimControl>`；函数内 `let mut interim_state = lt_pipeline::interim::InterimState::default()`。
7. `pipeline.rs:761` `let _ = source;` 改为分流：
   - `SegmentSource::Interim`（audio 必为空）→ `run_interim_pass(...)`（步骤 8）后 `{ let mut v = vad.lock().unwrap(); interim.last_interim_samples.store(v.speech_samples() as u64, ...) }`（对照 `main.py:1723-1724`），`continue`；
   - `SegmentSource::VadFlush` → 若 `interim_state.active`：transcribe 后文本先 `strip_committed_overlap(&text, &interim_state.committed_tail)` 再 `pending_merge(&interim_state.pending, &text)`，然后走既有三层过滤；**无论走哪支，处理完本段后 `interim_state.reset()`** + `interim.last_*` 清零（对照 `main.py:1711-1719`）。
8. 新函数 `run_interim_pass`（对照 `main.py:1411-1527` 逐条）：
   ① 锁 vad → `peek_buffer()` → 解锁；None 或时长 <1.5s → 返回。
   ② `manager.transcribe(&audio, false)`（对齐 `use_word_ts=False`）；失败/空文本/纯标点 → 返回。
   ③ `strip_committed_overlap(&full_text, &interim_state.committed_tail)`；剥后为空 → 返回。
   ④ `split_sentences(&text, &lang)`；`len <= 1` → 返回（末句仍在说，不提交）。
   ⑤ `complete = &sentences[..n-1]`；逐句：`is_short_utterance` → `interim_state.pending = pending_merge(&pending, sent)`；否则先 `pending_merge` 前置拼接并清 pending，再经**语言过滤**（对照 `main.py:1535-1538`，无噪声过滤）→ `commit_text(...)`（见步骤 9）。
   ⑥ `trim_samples(...)` 算裁剪量（interim.rs:257 已移植原版公式）→ `interim_state.committed_tail = committed_prefix(&complete)` 尾 50 字符 → `interim_state.active = true` → 锁 vad `trim_front(n)`。
9. 把 `run_asr_thread` 中"三层过滤 + AddMessage + 同语言判定 + 翻译提交"段（`pipeline.rs:769-820`）抽成 `fn commit_text(&settings, &tl, &proxy, text, lang, asr_ms, transcript)`，vad_flush 路径与 interim 完整句共用。**不新增 UiEvent，interim 句走普通 AddMessage——与原版一致，零契约扩展。**

**C. UI/命令接线（lt-ui + lt-app::shell）**

10. `vad.rs:519-526`：复选框变更处追加 `state.send_cmd(lt_proto::Cmd::IncrementalAsr { enabled: inc, interval: state.settings.interim_interval })`；间隔 DragValue（`vad.rs:527-542`）变更处同样发。更新 `vad.rs:518` 过时注释。
11. `shell.rs`：`other =>` 分支前新增：
    ```rust
    Cmd::IncrementalAsr { enabled, interval } => {
        if let Some(p) = &self.pipeline { p.set_interim(enabled, interval); }
        // settings 已由面板写入，这里只落盘
        self.persist_settings();
    }
    ```
    `Pipeline::set_interim` 写 `InterimControl` 的 enabled/interval_bits。**关闭增量时**同时清 `last_interim_samples/last_check_ms`，并把"当前 VAD 缓冲未消费"的问题留给下一次 vad_flush（原版同语义，无残处理）。

### 边界（不做）

- 不做停止/引擎切换时的残余缓冲 flush（原版 `main.py:1208-1221`）——Rust 现状 stop 丢队列，引擎切换走 `ReplaceEngine` 重载 worker，缓冲语义可接受；若实测出现"切换后首句吞字"，另立小卡补 `force_flush`。
- 不改分句器（pysbd 偏差已定界，`interim.rs:16-30`）。
- 不扩 lt-proto。

### 完成标准

- [ ] 单测：触发条件真值表（enabled×speaking×interval×elapsed×cooldown，`capture.rs` 测模块加用例，用可控 ConfidenceSource 造长语音缓冲）；`run_interim_pass` 抽出可测的纯逻辑（分流判定用假 manager 不现实——以真 worker 的 `#[ignore]` 集成测可选）。
- [ ] 手测主路径：开增量识别 + 间隔 2s，连续说 3 句长句 → 悬浮窗**渐次**出现完整句（不等整段结束）；关闭复选框 → 回到整句模式；暂停/恢复/清空无异常。
- [ ] 手测边界：短于 1.5s 缓冲不触发；auto 语言 + 目标语言不同 → interim 句正常翻译；asr_language 设 zh 且识别为 en → interim 句被语言过滤。
- [ ] 回归：增量关闭时 `SegmentSource::Interim` 零生产（触发条件短路）；`cargo test --workspace` 全绿（capture.rs 既有 3 测适配 Arc 签名后仍绿）。

### 风险与坑

- **锁序**：capture 线程锁序恒为 `vad → (segment_tx 内部锁)`；ASR 线程恒为 `段队列 → vad`。两线程无第二把共锁，无死锁面。`BoundedDropQueue` 内部锁不在任何持锁区获取（push/pop 都在 vad 锁外）——实现时严守"不在持 vad 锁时碰 segment_tx"。
- transcribe 在 ASR 线程串行执行，interim 标记与 vad_flush 段天然排队（原版同构，`main.py:1691-1724` 单线程 `_asr_loop`），无需额外 ASR 互斥。
- monitor 回调（31/s）经 proxy 发 UI 事件，与触发判定无交互。

---

## WP-4 SetPadding 热应用接线（P0，半小时级）

### 现状与证据

- 通道已建好但两端断线：`Pipeline::set_pending_padding(engine_family, secs)` 存在且标 `#[allow(dead_code)]`（`crates/lt-app/src/pipeline.rs:483-488`），`AsrPendingHandle` 机制齐（`pipeline.rs:259-264`，ASR 线程在下一次 transcribe 前应用）；`Cmd::SetPadding { engine, secs }` 在契约中（`events.rs:79`）但**无人发送、无人处理**（shell.rs:184 落未处理分支）。
- 面板 padding 滑条只写本地 settings + 落盘（`vad.rs:293-302`），不发热应用——重启/切引擎前不生效。原版 padding 经 `_set_asr_padding` + pending 机制即时生效（`control_panel.py:285-317`）。

### 改造步骤

1. `vad.rs:294-297`（sensevoice 分支）变更回调追加：`state.send_cmd(lt_proto::Cmd::SetPadding { engine: "funasr".into(), secs: sv })`；`vad.rs:299-302`（whisper 分支）同款 `engine: "whisper"`。注意 WP-1 步骤 3 会把两个滑条改成条件可见——接线写在各自块内。
2. `shell.rs` `other =>` 前新增 `Cmd::SetPadding { engine, secs }` 分支：`p.set_pending_padding(&engine, secs)` + `self.persist_settings()`（settings 字段已由面板写入）。删除 `pipeline.rs:483-488` 的 `#[allow(dead_code)]`。
3. `SetAsrLanguage` 已接线（`shell.rs:80-87`），核对引擎家族名约定：`AsrPendingHandle::set_padding` 的 `engine_family` 取值（查 `crates/lt-asr` 中 pending 实现）与发送侧 "funasr"/"whisper" 一致——不一致则统一为引擎 id。

### 完成标准

- [ ] 运行中拖 padding 滑条 → 下一段识别即应用（日志或行为可辨）；重启后值保持。
- [ ] `cargo test --workspace` 全绿。

---

## WP-5 托盘菜单与气泡通知（P1，⚑ 决策点 D-1）

### ⚑ 决策点 D-1：托盘菜单回补 vs 维持最小

- 现状 5 项（状态行/暂停恢复/悬浮窗显隐/控制面板/退出，`crates/lt-ui/src/tray.rs:22-28, 54-106`），commit 7d360b1 引"新版原版 tray menu stays minimal"。
- 工作区 Python 权威副本是**全量菜单**（`LiveTranslate/main.py:1895-2293`）：暂停/继续、悬浮窗显隐、字幕窗显隐（复选）、字幕窗穿透（复选）、悬浮窗子菜单（穿透/置顶/自动滚动/任务栏）、翻译模型子菜单（单选）、目标语言子菜单、识别语言子菜单、导出子菜单×3、控制面板、日志、退出。
- **建议默认：回补至对齐工作区副本**（托盘是"最小化后一切可操作"的核心入口；悬浮窗隐藏时尤其关键）。若用户确认采信"新版最小菜单"，则本 WP 只做 5b（气泡）后关闭。

### 改造步骤（按回补方案写）

**5a. 菜单回补（`crates/lt-ui/src/tray.rs` + `crates/lt-ui/src/app.rs::on_menu`）**

1. 菜单结构（muda；复选用 `CheckMenuItem`，子菜单用 `Submenu`）：
   ```
   ● 状态行（禁用，tray_status_format 已有键）
   ─
   暂停/恢复（动态文字，已有）
   悬浮窗 显示/隐藏（动态文字，已有）
   字幕窗 ▶（CheckMenuItem：显示/隐藏；app.rs 已有字幕窗开关逻辑可复用）
   ─
   悬浮窗 ▶ 鼠标穿透√ / 窗口置顶√ / 自动滚动√ / 任务栏√   ← 与悬浮窗 handle 复选双向同步
   翻译模型 ▶ <动态单选组，分若干项>
   目标语言 ▶ <常用 8 项 + "更多语言"▶（其余）>            ← 原版 main.py:2186-2227
   识别语言提示 ▶ <auto + 常用 + 更多>                     ← 原版 main.py:2230-2268
   导出 ▶ 原文 / 译文 / 全部                               ← 复用悬浮窗右键导出实现
   ─
   显示控制面板（已有） / 显示日志（新增入口）
   ─
   退出（已有，宿主侧确认框）
   ```
2. 动态子菜单策略：muda 无 about-to-show 事件，**在状态变化点重建子菜单**（翻译模型列表变化 = `SwitchTranslator`/设置导入；语言变化 = `SetTargetLanguage`/`SetAsrLanguage`）。`Tray` 提供 `rebuild_dynamic_menus(&mut self, state)`，整体 `TrayIcon::set_menu` 替换（tray-icon 0.24 支持；执行时核对该 API，若不支持整体替换则逐项 `remove/add`）。
3. 双向同步点（原版 `main.py:2026-2133`）：四个悬浮窗复选的三处同步（悬浮窗 handle / 托盘子菜单）——`app.rs` 现有 `ov_click_through/ov_topmost` 等状态字段（`state.rs:1183-1186` 附近）为单一事实源，两处 UI 都改它 + 刷新 `CheckMenuItem::set_checked`。
4. i18n 新键（zh/en 同步）：`tray_show_subtitle`、`tray_subtitle_click_through`、`tray_show_log`、`tray_overlay_menu`、`tray_model_menu`、`tray_target_lang_menu`、`tray_source_lang_menu`、`tray_export_menu`、`tray_export_original`、`tray_export_translation`、`tray_export_all`、`tray_more_languages`。已有可复用键：`tray_show_overlay`/`tray_hide_overlay`/`tray_show_panel`/`tray_status_format` 等（`assets/i18n/zh.yaml:268-280`）。
5. `ids` 模块扩充（`tray.rs:22-28`）：id 用稳定前缀编码参数（如 `tray_model_3`、`tray_tgt_zh`、`tray_src_auto`），`on_menu`（`app.rs:299`）解析分发。

**5b. 气泡通知（Shell_NotifyIcon 直调）**

6. tray-icon 0.24 无气泡 API（AGENTS.md 坑 7）。方案：**自建 message-only 隐藏窗口 + 独立 NOTIFYICONDATAW**（复用 app 图标），封装 `crates/lt-ui/src/win32/balloon.rs`（或 `tray.rs` 内 `#[cfg(windows)]` 模块）：`show_balloon(title, msg, flags NIIF_*）`。挂 `Shell_NotifyIconW(NIM_ADD/NIM_MODIFY, &nid)`，`NIF_INFO`。执行前核对 tray-icon 是否已暴露内部 HWND（若暴露可直接对其 NIM_MODIFY，省一个图标位）。
7. 三个场景（原版 `main.py:1943-1948` 首次隐藏悬浮窗、`main.py:1995-2000` 首次开字幕窗拖动提示、`main.py:2320-2328` RSS>4GB 警告）：
   - 前两个：`app.rs:394-399` 现降级日志处改调 `show_balloon`；文案键已有（`hide_tray_hint` zh.yaml:276；字幕窗提示键核对 zh.yaml，无则补）。
   - 4GB 警告：RSS 数据源在 ASR 线程空闲分支（`pipeline.rs:707` 附近已有 RSS 读取），超阈值时经 proxy 发事件 → 宿主调气泡 + 10s 节流（原版 Warning 级 10 秒）。

### 完成标准

- [ ] 托盘全菜单树可操作；悬浮窗隐藏后：暂停恢复/字幕窗/导出/模型切换/退出全部可用。
- [ ] 三向同步：悬浮窗改穿透 → 托盘勾选态即时变；反向亦然。
- [ ] 三气泡实机可见；4GB 警告可用人为加大阈值触发验证。
- [ ] i18n zh/en 同步；`cargo test --workspace` 全绿。

### 风险与坑

- muda 菜单动态变更与 `MenuEvent` handler 的 id 稳定性：重建后旧 id 失效属预期；`on_menu` 对未知 id 静默忽略（勿 panic）。
- 气泡与 tray-icon 共存：独立 NOTIFYICONDATA 时任务栏可能出现第二个图标位（不可见图标 `NIF_STATE` 隐藏或在回调里 `NIM_DELETE`）——实现后实机确认只显示一个托盘图标。

---

## WP-6 缺模型下载门恢复 + 加载框日志（P1，⚑ 决策点 D-2）

### ⚑ 决策点 D-2：下载门恢复范围

- commit 785eed1 以 less-is-more 拆除"首启向导 + 缺模型下载门"（`main.rs:3-7`），当前新用户首启 = 直进主界面 + AsrUnavailable + 自行摸索识别页下载按钮。
- 状态机其实**完整保留**：`AppState::with_startup`（`state.rs:1164-1185`，启动流中主窗全隐仅 Setup 可见）、`StartupFlow::DownloadMissing`（`state.rs:1058-1069`）、装配助手 `lt_ui::state::startup_flow(first_launch, missing_names)`（`state.rs:1078-1092`，**main.rs 未调用**）、下载成功收尾链路全通（`app.rs:643-660` 置 finished + 500ms 收尾；`shell.rs:214` 启动管道）。
- **建议默认：恢复"缺模型下载门"（DownloadMissing），首启（无 settings 文件）维持直进**——既保住 less-is-more 的开箱直进，又补掉"拿到 exe 跑不起来"的断层。若用户要完整向导（Wizard 流代码全在 `setup.rs:49-151`），改 `startup_flow` 的 first_launch 分支即可，属一行决策。

### 现状与证据（断点清单）

- `main.rs:64` 恒传 `first_launch=false`，`missing` 快照只给 backend 不给 UI；`AppState::new`（=Ready）在 `main.rs:42`。
- `DownloadMissing` 流的 UI 消费齐全（标题 `app.rs:114`、日志 `:621`、失败 `:634`、完成 `:651`），但**无人触发自动开始下载**：`wizard_auto_start` 只匹配 Wizard 流（`state.rs:1509-1525`）。
- 加载框日志空占位：`setup.rs:197` `log_view(ui, &[])`（自认注释"M4 接 LogLine 日志流"未兑现）。

### 改造步骤

**a. 恢复下载门**
1. `main.rs`：`missing` 计算后（`main.rs:54-63`），若非空 → `let flow = lt_ui::state::startup_flow(false, missing.iter().map(|m| m.display.clone()).collect())`；`AppState::with_startup(initial_settings.clone(), flow)` 替换 `AppState::new`。同时 `backend::spawn` 的 missing 传参改为**空 Vec**（targets 动态化已在 WP-1 落地，快照不再需要；若 WP-1 未做则保持快照并同步两处）。
2. 自动开始下载：`state.rs` 新增 `download_missing_auto_start()`（幂等：`StartupFlow::DownloadMissing` 且未发过——在 flow 里加 `started: bool` 字段或复用 `finished` 语义外加标记），照 `wizard_auto_start` 模式发 `Cmd::StartDownload { hub, proxy }`（hub/proxy 取 settings）。触发点：`app.rs` Setup 窗首帧绘制处（或 `with_startup` 后由宿主 `kick_ticks` 时发）。对照原版 `ModelDownloadDialog` 打开即下载（`dialogs.py:424-427` 200ms 轮询启动）。
3. 失败路径已通：`DownloadFailed` → `app.rs:634` 置 failed → `setup.rs:168-183` 显示关闭按钮 → `Cmd::Stop` + quit（对齐原版取消即退出 `main.py:1818-1838`）。

**b. 加载框日志接线**
4. `state.rs`：`load_dialog: Option<String>` 旁加 `load_dialog_log: Vec<String>`（复用 `push_log_line` 500 行限幅，`state.rs:1095-1099`）。
5. `app.rs` 的 `LogLine` 分支（约 `:620`）：`if self.app_state.load_dialog.is_some() { push_log_line(&mut self.app_state.load_dialog_log, msg) }`——加载期间 INFO+ 全收（对齐原版 `_LogCapture` 捕 INFO+，`dialogs.py:96-119`）。`ModelLoadDone`/`AsrUnavailable` 关框时清空。
6. `setup.rs:197`：`log_view(ui, &state.load_dialog_log)`，删除"M2.5 占位"注释。

### 完成标准

- [ ] 临时配置目录（无模型）冒烟：启动 → 主窗不出现 → DownloadMissing 对话框自动开始下载 → 日志滚动 → 成功后 500ms 自动关窗 → 主窗显现 → 管道自动启动（`shell.rs:214` 链路）。
- [ ] 下载中途断网 → 失败行 + 关闭按钮 → 退出干净。
- [ ] 运行中切未缓存引擎 → 加载框内可见加载日志（不再是空白暗底）。
- [ ] settings 齐备的常规启动不受影响（无 Setup 窗、直进主界面）。

---

## WP-7 字幕窗已知偏差收口（P2，逐项裁决）

### 现状与证据

自认偏差清单在 `crates/lt-ui/src/windows/subtitle.rs:15-25` 头注释。逐项处置建议：

| # | 偏差 | 处置建议 | 说明 |
|---|---|---|---|
| 1 | bg_image 背景图不渲染（配置键保留） | **做** | OBS 场景可见差距。egui：加载图片 → `ctx.load_texture` → Frame 画布贴图 + 圆角裁剪；路径按原版转相对（`subtitle_settings.py:84-99` 语义）。设置变更热生效走现有 SubtitleMode 应用路径。 |
| 2 | 行级 font_family 走全局字体栈 | **部分做** | 启动时注册常见族（Consolas、Segoe UI、微软雅黑、SimHei 等 3-5 个）入 egui 字体表，行级设置命中已注册族则生效，否则回退全局并记 debug。任意字体枚举不做（egui 动态加载系统字体成本高，记偏差）。 |
| 3 | slide_down 以 fade 近似 | **做**（小） | 绘制时 y 偏移 + alpha 渐变，与现有 150ms OutCubic 高度动画同框架。 |
| 4 | 行级 entry/exit 动画未做 | **做**（中） | 六方向（none/fade/slide_left/right/up/down，`subtitle_settings.py:233-240`）。每行动画状态机：入场（自动隐藏恢复时方向取反，原版 `subtitle_window.py:719-756`）+ 退场。帧驱动挂字幕窗 repaint 节拍（注意坑 1：仅动画期间 request_redraw，动画结束停拍）。 |
| 5 | 翻译错误文案进字幕 | **做**（小，注意契约） | 现状 `UpdateTranslation.text` 携带 `e.ui_text()`/`error_repetition` 文案（`pipeline.rs:204-222`），字幕窗无从分辨。**零契约方案**：字幕窗渲染前比对 `text` 是否等于已知错误文案集合（`t("error_repetition")` + lt-translate `ui_text()` 的 i18n 键全集，枚举处加注释锚定）；命中则字幕行跳过该译文。 Scr 引言注释写明"若未来扩契约应改为显式 is_error 标志"。 |
| 6 | 多屏判定用全屏尺寸不剔任务栏 | **做**（小） | lt-ui 已有 `::windows::` Win32 调用先例（穿透轮询 `app.rs:956-1029`）。加 `work_area()`：`MonitorFromWindow` + `GetMonitorInfoW` 取 `rcWork`；`clamp_to_screen`/`is_pos_visible`（`app.rs:791-804`）改用工作区。悬浮窗校验（`app.rs:168-194`）同步受益。 |

### 完成标准

- [ ] 每项完成后更新 `subtitle.rs:15-25` 清单（做掉的删掉，不做的写明偏差编号 D-17+）。
- [ ] bg_image：设背景图 → 字幕窗可见贴图 + 圆角；清除后回纯色。
- [ ] 行动画：六方向各试一次；快速连发句排队 + 1500ms 最小显示行为不回归（`subtitle.rs` 既有 17 测保持绿）。
- [ ] 错误文案：拔掉翻译 API → 字幕窗不出现错误占位文本，悬浮窗正常显示。
- [ ] 多屏 + 任务栏遮挡场景实机走查（全屏截图，坑 10）。

---

## WP-8 ErrorBanner 与全局热键（P2，⚑ 决策点 D-3/D-4，先确认后动工）

### ⚑ 决策点 D-3：ErrorBanner 出处

- AGENTS.md 待办列"ErrorBanner（新版悬浮窗错误分类条）"，但**工作区 Python 权威副本无此部件**（错误走弹窗/消息内联 `[error: ...]`/托盘气泡，`main.py:750-776, 1130-1131`），RESEARCH.md 亦无条目。
- **必答前置问题（问用户）**：ErrorBanner 的参照物是什么？（新版原版截图/文字描述/错误分类清单？）
- 兜底设计（若确认要做而无详细参照）：悬浮窗头部行下方错误分类条，聚合现有错误面——`AsrUnavailable`（ASR 不可用/模型缺失）、`DownloadFailed`（下载失败，可点重试）、翻译错误计数（连续失败提示，点击跳设置）。展示语义：有错常驻、可手动关、错误恢复自动消失。
- **建议默认：拿到参照前不动工。**

### ⚑ 决策点 D-4：全局热键出处

- AGENTS.md 待办列"全局热键（hotkeys_group）"，但 Python 副本**零热键代码**、Settings 无 `hotkeys_group` 键、RESEARCH.md 无条目——疑似"新版原版"特性或新需求，出处未证实。
- **必答前置问题**：① 出处（新版原版还是新需求）？② 期望热键集（建议最小集：暂停/恢复、显示/隐藏悬浮窗、清空消息）？③ 冲突策略（注册失败降级提示）？
- 技术预研结论（供立项）：Win32 `RegisterHotKey` + message-only 窗口 `WM_HOTKEY`，lt-ui 内 `#[cfg(windows)]` 模块；`WM_HOTKEY` 经 proxy 进事件循环。无新依赖。
- **建议默认：确认出处前不动工。**

---

## WP-9 M6 调优与实测（P1，需实机，建议在 P0 三项落地后做）

### 现状与证据

- 全部无实测记录（AGENTS.md 待办）。机制层已备：worker RSS 回收（ASR 线程空闲分支 `pipeline.rs:707` `maybe_recycle_if_idle`，阈值逻辑在 `crates/lt-asr`）、消息 50 条上限、日志 500 行限幅、2000 行日志环。
- 端到端语音复验缺位：RMS 恒 0 管线故障已修（commit 9a032e0）但**没有真人对麦验证过全链**。

### 改造/实测步骤

1. **启动 <2s**：release 构建冷/热启动各 5 次。计时 = 进程创建 → 悬浮窗首帧可见（实机秒表 + tracing 首帧日志双重）。热点排查顺序：ort dll 解压（`ensure_ort_dylib`，`main.rs:23`）、模型缓存探测（`main.rs:54-63`）、字体栈注册、wasapi 首次初始化。必要时把探测挪后台线程（不得阻塞 UI 首帧）。
2. **空闲 CPU <1%**：暂停态 + 无音频态各采样 60s（Process Explorer / `typeperf`）。已知可疑点：穿透 50ms 轮询（`app.rs:956-1029`）、egui 节拍窗（kick_ticks）、托盘。超标的思路：轮询改事件驱动（WinEventHook/caret 事件）或降频——先测量再动。
3. **8h 长跑**：实机挂 8h（有真实音频源，可用会议/视频回环）。观测：RSS 曲线（ASR worker + 主进程）、`maybe_recycle_if_idle` 触发次数与回收前后 RSS、消息/日志限幅生效、无句柄泄漏（Task 句柄数）、暂停恢复 100 次往返。
4. **内存回收实测**：确认 `maybe_recycle_if_idle` 阈值语义与原版一致（原版 worker RSS 超基线 +2048MB 优雅回收，`main.py:977-1042`；合计超 4096MB 托盘警告，`main.py:1044-1057`——警告依赖 WP-5b 气泡）。
5. **端到端语音复验**（清单化）：真人麦克风 + 系统回环双源；ja→zh 与 en→zh 两组；interim 开/关对照（依赖 WP-3）；whisper 与 sensevoice 双引擎（依赖 WP-1）；导出×3 与转写落盘文件核对；字幕窗 OBS 实采一屏。

### 完成标准

- [ ] 三项指标实测数字回填本文档与 AGENTS.md；不达标项出后续卡。
- [ ] 端到端清单全绿；发现问题按域拆新卡（不塞本 WP）。

---

## 附A 决策点汇总

| # | 问题 | 选项 | 建议 |
|---|---|---|---|
| D-1 | 托盘菜单：回补全量 vs 维持最小 5 项 | 工作区副本全量（~12 项）/ 新版最小 5 项 | **回补**（WP-5a）；若确认新版口径则只做 5b 气泡 |
| D-2 | 下载门恢复范围 | A. 缺模型 DownloadMissing 门 + 首启直进（建议）；B. 连首启向导一起恢复（`startup_flow` first_launch=true）；C. 维持现状无门 | **A** |
| D-3 | ErrorBanner 参照物 | 新版截图/描述/无参照缓做 | **拿到参照前不动工** |
| D-4 | 全局热键出处与键位 | 新版特性/新需求/缓做 | **确认前不动工**；技术方案已备（WP-8） |
| D-5 | WP-1 步骤 5：whisper 档位变更是否即时切引擎 | 仅落盘（原版语义）/ 即时切换 | **仅落盘** |
| D-6 | WP-2 nano 已存设置的回退 | `resolve_funasr_entry` 加回退分支 + warn / 维持直接装配失败 | **加回退**（与 mlt 同款） |

## 附B 与 AGENTS.md「当前待办」的映射

| AGENTS.md 待办 | 本计划 |
|---|---|
| ErrorBanner | WP-8（D-3） |
| 全局热键 hotkeys_group | WP-8（D-4） |
| M6 interim 装配接线 | WP-3 |
| FunASR Nano 实装 | WP-2（先防坑；实装前置调研另立卡） |
| StartDownload targets 动态化 | WP-1 步骤 6 |
| CI | 本计划未含（建议 P0 落地后单独立卡：`cargo test --workspace` + `cargo clippy` 双矩阵即可起步） |
| M6 调优三件 + 内存回收实测 | WP-9 |
| 端到端语音复验 | WP-9 步骤 5 |

## 附C 证据索引（关键 file:line 速查，基线 a3ecaf1）

**Rust 侧**
- 引擎灰显：`crates/lt-ui/src/windows/panel/vad.rs:227-229`（灰显）、`:8-9`（过时注释）、`:216`
- whisper 档位无下拉：`vad.rs:554,560,663`（只读）；缓存组/下载按钮：`vad.rs:545-598`
- nano 可选：`vad.rs:62-68`；mlt 灰显先例：`vad.rs:70,273-275`；funasr→sensevoice 映射：`pipeline.rs:593-606`；SenseVoice loader：`crates/lt-asr/src/engines/sensevoice.rs:30-56`
- interim 断点：`pipeline.rs:703-704,761`、`shell.rs:184-185`、`vad.rs:518`；算法层：`crates/lt-pipeline/src/interim.rs`（37 测）；`SegmentSource`：`crates/lt-pipeline/src/lib.rs:20-25`；CaptureLoop：`crates/lt-pipeline/src/audio/capture.rs:16-67`；VAD 预留接口：`crates/lt-pipeline/src/vad.rs:587,597,625`；VAD 独占：`pipeline.rs:339-373`
- padding 断线：`pipeline.rs:483-488`（`#[allow(dead_code)]`）、`events.rs:79`、`vad.rs:293-302`
- 托盘 5 项：`crates/lt-ui/src/tray.rs:22-28,54-106`；气泡降级：`app.rs:394-399`；on_menu：`app.rs:299`
- 下载门断点：`main.rs:3-7,54-64`（恒 first_launch=false）、`state.rs:1058-1092,1164-1185,1509-1525`（状态机齐全）、`backend.rs:77-105`（targets 快照）、`shell.rs:214`（DownloadSucceeded→启动管道已通）、`app.rs:643-660`
- 加载框日志占位：`crates/lt-ui/src/windows/setup.rs:197`
- 字幕窗偏差自认：`crates/lt-ui/src/windows/subtitle.rs:15-25`
- 穿透轮询：`app.rs:956-1029`；多屏判定：`app.rs:168-206,791-804,875-877`

**Python 原版（LiveTranslate/ 副本）**
- interim 全链：`main.py:1354-1527`（算法）、`:1529-1548`（`_process_segment_text`）、`:1554-1621`（`_process_interim_final`）、`:1657-1668`（capture 触发）、`:1691-1734`（ASR 线程分流+drain）、`_vad_lock`：`main.py:188,1637,1654-1655,1519-1520,1723-1724`
- 托盘全菜单：`main.py:1895-2293`（字幕窗 `:1974-2045`、悬浮窗子菜单 `:2100-2133`、模型 `:2136-2183`、语言 `:2186-2268`、导出 `:2272-2282`）；气泡三场景：`main.py:1943-1948,1995-2000,2320-2328`
- 引擎/档位/padding 下拉：`control_panel.py:213-233,269-317,385-408`；下载组联动：`control_panel.py:1152-1236`
- worker 回收/警告：`main.py:977-1042,1044-1057`；模型完整性校验：`model_manager.py:343-422`
- 字幕窗行为：`subtitle_window.py:51-99`（行配置）、`:522-524,834-851`（排队+1500ms）、`:646-671`（高度动画）、`:703-756`（自动隐藏+反向动画）

## 附D 推进顺序建议（默认路径）

```
第 1 批（P0，互不依赖可并行）：
  WP-1 Whisper 放开 + targets 动态化   ── 半天～1 天
  WP-2 nano 置灰防坑                   ── 1 小时
  WP-4 SetPadding 接线                 ── 半小时
第 2 批（P0 大件）：
  WP-3 interim 接线                    ── 2～3 天（VAD 共享拓扑是承重墙，建议主线程亲自做）
第 3 批（P1，依赖 D-1/D-2 裁决）：
  WP-5 托盘 + 气泡                     ── 1～2 天
  WP-6 下载门 + 加载框日志             ── 1 天
  WP-9 调优实测（需实机时段）          ── 1～2 天
第 4 批（P2，逐项裁决）：
  WP-7 字幕窗六项                      ── 1～2 天
  WP-8 ErrorBanner/热键（D-3/D-4 确认后）
```

每批收尾：`cargo test --workspace` 全绿 + clippy 无告警 + 中文 commit（`feat(wp1-whisper-ui): …` 风格）+ 必要时回写 RESEARCH.md 偏差编号与 AGENTS.md 待办勾销。
