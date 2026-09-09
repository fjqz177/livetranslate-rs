# LiveTranslate-rs 架构收尾与开发体系方案（v2.1 改进计划）

- **定稿日期**：2026-09-09（基线 commit `1d608d2`，439+7 测全绿实测复核）
- **证据基线**：2026-09-09 全仓 review（八原则对抗审查）——十 crate 拓扑、契约形态、总线单写者、监督器、动脉、UI 投影化逐项实测核对；守护脚本与全量测试复跑通过
- **效力**：架构 2.0（W0~W7）之后的收尾期施工依据（本目录，非归档）。各波次为独立工作包，可按裁决调整取舍；**文档先行独立 `docs(scope)` 提交，先于一切实现提交**。
- **状态**：v1.1 定稿候选（2026-09-09 二轮裁决完成：原待裁决 3~7 全部按推荐案通过——D-79 改型执行 / E3 白名单变更通过 / D-80 Backoff 接受默认参数 / 死契约守卫 A 硬 gate / 写入点统一 A 案；whisper sha256=有网时补齐；WD-8 更新源=GitHub Releases）。**全部裁决项闭合（§6），待用户审核本文件后批准开工；未批准前不动任何实现代码。**

---

## 0. 总判断

架构 2.0 的骨架（十 crate 白名单、单写者总线、事件动脉、线程监督、UI 投影化）经实测验证全部为真，是继续开发的合格地基。当前问题不是「架构错了」，而是三类尾巴：

1. **实现偏离方案未留痕**（Backoff 缺失、双监督器未记录）——流程问题，不是设计问题；
2. **迁移期残留**（lt-ui→lt-translate 常量边、THINKING_STYLES 镜像、serde_json 死依赖）——一次性清扫；
3. **类型化只做了事件层**（engine/hub/proxy 等 37 处字符串值域比较）——最后一层「字符串走私」。

全部清完后，本仓库达到：**每一层依赖、每一个契约、每一个值域、每一篇文档都有机器或测试锚定**。向导子系统按用户裁决**保留并定位为待开发面**（D-78），其现状地图与接线点见 §5.4。

---

## 1. 证据基线（全部论据的实测出处，2026-09-09）

| # | 事实 | 证据 |
|---|---|---|
| F1 | 439 测通过 + 7 ignored，0 失败 | 本次实测 `cargo test --workspace` |
| F2 | 依赖白名单 10 crate 21 边全符合；五组禁令闭合 | `check_deps.ps1` / `check_guards.ps1` 实测 OK |
| F3 | `Policy::Backoff` 仅存在于方案文档（architecture-v2.md §3.2.1 代码骨架），代码与全仓零命中 | grep 实测 |
| F4 | `lt-ui` 对 `lt_translate` 的引用只剩常量：`DEFAULT_PROMPT`（translator.rs:78 定义，:278/:355 内部消费）、`PROMPT_PRESETS`（translator.rs:89）、`THINKING_STYLES`（thinking.rs:14） | grep 实测 14 处引用全部为常量 |
| F5 | `THINKING_STYLE_VALUES`（lt-ui/state.rs:809）是 thinking.rs:14 的逐字拷贝；`OVERRIDE_KEYS`（state.rs:811）**不是**镜像——overrides 经 `BTreeMap` 透传（translator.rs:243），translate 侧无键枚举 | 读码实测 |
| F6 | `Cmd::Stop` 唯一生产者 = setup.rs:198（向导路径）；shell.rs:296 有处理臂 | grep 实测 |
| F7 | `StartupFlow::Wizard/DownloadMissing` 全仓零生产者（向导未接线，D-19 既有裁决）；`WinId::Setup` 引用 103 处（setup.rs 21 / app.rs 35 / state.rs 47） | grep 计数实测 |
| F8 | `quit_requested` 的活跃生产者是 confirm.rs:70（退出确认模态，**存活**）；setup.rs:199 只是向导副本——**向导改造/重写不得波及 confirm 流** | 读码实测 |
| F9 | 引擎字符串比较 37 处 / 7 文件；其中域内散点仅 orchestrator（settings_bus.rs、pipeline.rs）与 lt-ui（vad.rs）；lt-asr（manager/engines/profile）、lt-models（cache）、lt-app/main（worker 分派）的比较是**单点分派边界**，属合法豁免 | grep 实测 + 读码定性 |
| F10 | `Hub` 已在 lt-proto/layout.rs（W6 迁入）；`ProxyMode` 仍在 lt-download/lib.rs:76 | grep 实测 |
| F11 | `derive()`（settings_bus.rs:91-111）消费 8 个字段；新增 Settings 字段忘接线时编译与测试全静默 | 读码实证 |
| F12 | orchestrator 的 `serde_json` 声明零引用（src 含内嵌测试零命中） | grep 实测 |
| F13 | 两个 Supervisor 实例：main.rs:90（app 侧：动脉桥/日志桥/探测/基准/文件框/下载会话）与 pipeline.rs:503（管道侧：capture/ASR/tl/AudioBridge），各有独立 monitor | 读码实测 |
| F14 | `Cmd::SetPadding`/`Cmd::IncrementalAsr` 的载荷被 shell 丢弃（shell.rs:166-180 只 publish），生效依赖 UI 先写草稿（vad.rs:474/493 预写）；同枚举 `SetAsrLanguage/SetTimeout/SetAudioDevice` 等却是 shell 侧写入 | 读码实测 |
| F15 | supervisor.rs:72 `spawn` 不检查 `stopping`；INV4 只封了 monitor 的 respawn | 读码实测 |
| F16 | 托盘线程死亡无 ThreadDied（lt-ui 裸 spawn 白名单，无 ThreadRole::Tray）；方案线程表写「ThreadDied 可见」与实现不符；`lt-singleton` 方案表写「Supervisor(Never) 线程」，实际无线程（message-only 窗） | 读码实测 |
| F17 | lt-app 1187 行（验收口径 ≤900→≤1200 已修订，有透明记录；属「指标漂移」第二例） | 实测 + 文档 |

---

## 2. 目标状态（定稿后验收锚）

1. 常驻线程确定性 panic 时，重生频率有界且最终放弃并告警（方案 §3.2.1 承诺兑现，D-80）；
2. `lt-ui` 的内部依赖 = **proto / i18n / models** 三条（纯投影 crate）；
3. 全仓不存在「UI 域与引擎域手工拷贝的同名常量清单」（镜像清零）；
4. orchestrator 与 lt-ui 的引擎/hub/proxy 判定零字面量比较（枚举透镜唯一，D-79）；
5. 新增 Settings 字段忘接总线派生 → **测试红**；
6. 契约枚举出现零生产者/消费者变体 → **脚本硬拦截**（预留白名单显式编目）；
7. 向导子系统**保留**（D-78），其接口/架构/契约现状地图成文（§5.4），后续开发有据可依；
8. 方案文档线程表、Policy 枚举、监督器实例数与代码逐字一致；
9. 每个新功能开工前，开发者能从功能落位表（§5.1）直接读出「改哪几个 crate、动哪些契约、配哪些测试」。

---

## 3. 关键决策（ADR 续编，含落选理由与裁决状态）

### ADR-8：监督器维持两实例，文档如实登记（不合并）——已裁决执行

**决策**：保留 app 侧与管道侧两个 `Supervisor` 实例，方案线程表改双实例分栏。

**论证**：合并的唯一收益是「一张真实的表」；但合并要求 `Pipeline::stop`/`StartGuard` 的回滚只 join 管道线程，必须引入**线程分组（group）实体**——为一个纯文档问题新增一套机制。两实例实测代价仅第二个 monitor 的 500ms 周期唤醒；生命周期语义上各守其界（管道线程随 `Pipeline` 生灭、app 线程随进程生灭），边界本身是正确内聚。
**落选**：合并+分组（新增实体，收益仅文档）；维持现状不改文档（容忍文档撒谎，与资产 A10 冲突）。

### ADR-9：值域类型化 = 「String 持久层 + proto 枚举透镜」，不改 Settings 字段类型——已裁决执行（D-79）

**决策**：`Settings.asr_engine/hub/download_proxy` 保持 `String`（与原版 user_settings.json 逐键兼容是硬约束）；lt-proto 新增 `EngineKey` 枚举、迁入 `ProxyMode`，提供 `Settings::engine_key()/hub()/proxy_mode()` 三个转换透镜；域内判定全走透镜；`Cmd::StartDownload` 载荷改型为枚举（D-79）。

**论证**：直接枚举化 Settings 字段会让「未知值毒化整份配置加载」，与 R17「坏值回退+告警」语义冲突，且波及全部 `Settings{..}` 字面量构造点。透镜方案把转换收敛到字段旁的唯一函数（R25 `resolve_funasr_entry` 已验证模式），未知值回退+warn 单点化。F9 实测域内散点只有 2 个文件——改动面天然有限；lt-asr/lt-models/lt-app-main 的字符串比较是单点分派边界（IPC 入口、注册表入口），保持字符串是边界契约的一部分。
**落选**：Settings 字段直接枚举化（毒化加载 + 改动面失控）；只加枚举不加透镜（转换散落，等于没做）。

### ADR-10：常量上移 lt-proto，lt-ui 裁掉 lt-translate 边——已裁决执行

**决策**：`DEFAULT_PROMPT`/`PROMPT_PRESETS`/`THINKING_STYLES` 迁 lt-proto；lt-translate 引 proto（白名单 +1 边 `tl→proto`）；lt-ui 删 lt-translate 依赖；删 `THINKING_STYLE_VALUES` 镜像。

**论证**：F4 实测这条边的全部现存用途就是三组常量——「bench 直调」的原始理由随 W5 迁移消亡。迁移后 lt-ui 成为纯投影 crate（目标状态 #2）。proto 是全网底部（F10 证明 Hub 已循此前例入驻）。`OVERRIDE_KEYS` 经 F5 实证**不是**镜像，保留 lt-ui，仅修订误导性注释。
**落选**：维持现状（边的存在理由已死 + 镜像持续缴漂移税）；常量复制进 proto 而不删 translate 侧定义（双真源，比现状更糟）。

### ADR-11：向导子系统保留并定位为待开发面（D-78）——已裁决（2026-09-09，维持并强化 D-19）

**决策**：**不删除**向导子系统。其代码、接口、架构、契约全部原样保留，作为用户后续开发的首启引导功能面；本方案 §5.4 给出其完整现状地图与接线点，作为后续开发的设计底稿。

**论证**：用户裁决（2026-09-09）：向导子系统后续会接着开发，接口/架构/契约要留出空间。原 review 曾以「死代码缴税」主张删除并预占 D-78 作撤销登记；裁决后 D-78 转为**保留裁决**的登记号。保留决策同时使 `Cmd::Stop` 契约臂与 Setup 窗维持合法存在地位——§5.4 地图是「留空间」的具体形态：不是留着不管，而是**让后续开发者不必重新考古**。
**附带修正**：死契约守卫（ADR-13）的白名单须包含向导域预留项，避免保留裁决与守卫脚本互相打架。

### ADR-12：Backoff 按方案 §3.2.1 原设计补齐（D-80）——已裁决执行（参数按默认）

**决策**：`Policy` 增 `Backoff { base_ms, max_ms, give_up_after, reset_after_ms }`；常驻线程（Capture/AsrMain/TlWorker/LogBridge）迁移至 `Policy::backoff()` 预设（500ms/30s/8 次/60s）；**动脉桥保持 Always**（UI 活性本身，死透=界面全死，宁可无限重试；其循环体仅 pop/send/take 三个无 panic 源操作，风险可控）。

**论证**：兑现方案既有承诺而非新设计（F3）。参数推导：`base_ms=500` 与 monitor 扫描周期同阶；`max_ms=30s` 使持续失败时 crash 文件速率从 2/s 降至 1/30s；`give_up_after=8` 累计尝试窗约 91s（0.5+1+2+4+8+16+30+30）后放弃并推 `ThreadDied{restarted:false}`——一条日志窗 error 行是用户可感知的最终态；`reset_after_ms=60s` 使偶发 panic 不积累。放弃后的呈现**先只做日志行**；实机走查若发现不够醒目，再升级识别页红字（复用 `pipeline_error` 通道，纯增量）。
**落选**：维持 Always（风暴场景无解）；只加延迟不加放弃（30s 一次的永久告警流仍是噪声）；引第三方 backoff crate（30 行内自写，零依赖原则，ADR-3 同精神）。

### ADR-13：死契约治理 = 「派生覆盖率测试 + 零引用守卫脚本」——已裁决执行（严格度 = A 硬 gate）

**决策**：①lt-proto 增测试：Settings serde 字段全集必须分类为「总线派生接线 / raw 直读白名单」之一，漏归类即红（治 F11）；②`scripts/check_dead_contract.ps1`：抽取 `Cmd/UiEvent/AppCommand/ThreadRole/DownloadPhase` 变体名，统计定义文件外非注释引用数，<2 且不在预留白名单 → 拦截（严格度已裁决 = A 硬 gate，非零退出进 CI）。

**预留白名单**（保留裁决的配套）：`DownloadPhase::Start`、`DownloadPhase::Integrity`（文档明示契约预留）；向导域若后续新增预留变体在此编目。
**论证**：冻结规则的「纯加法豁免」必须配「加了没人用」的回收机制，否则契约只增不减（`Cmd::Start` 靠人工 review 发现、`Cmd::Stop` 因向导保留而合法存续——两者共同证明需要显式编目而非静默）。覆盖率测试是无反射环境下 serde_json 取键集的最务实等价物（proto 已依赖 serde_json，零新依赖）。
**落选**：rustc/Clippy 层检测（无跨 crate 变体生产-消费分析）；cargo-udeps（查不到变体级）。

### ADR-14：process 纪律三条（已裁决采纳，2026-09-09）

1. **验收指标漂移登记**：验收口径（行数/性能/基线）修订必须在该波文档以 ADR 注记（F17：≤900→≤1200 为第二例，不许第三例无痕修订）；
2. **方案偏离留痕**：实现删改方案明文设计必须在方案文档对应小节加施工注记（F3 反例：Backoff 被悄悄简化）；
3. **收口两问**：每波收口自查——本波触碰的方案小节逐条回读了吗？新实体各有几个消费者（<2 的当面说清是预留还是删掉）？

**执行语义（用户已确认）**：违反时在 commit message 自曝并交用户裁决，不悄悄补文档。

---

## 4. 波次施工卡（E1→E6，每波全绿独立可 revert；节奏 = 用户裁决的 A 案：E1+E2+E3 一批交付、E4+E5+E6 一批交付）

通用纪律：每波 1~2 commit、全测绿（基线 439+7 不回退、净增见各卡）、三守护脚本过、行为差异登记 D-xx、收口更新 AGENTS 标记、ADR-14 三条纪律生效。

### E1 卫生与一致性（半天，零风险）

| # | 改动 | 精确位置 | 验收 |
|---|---|---|---|
| 1 | 删 `serde_json` 依赖声明 | `crates/lt-orchestrator/Cargo.toml` [dependencies] | `cargo tree -p lt-orchestrator` 无 serde_json 直边；全测绿（F12 证零引用） |
| 2 | 修漂移注释 | `crates/lt-ui/src/windows/bench.rs:8` 「跑 run_benchmark」→「输出经 `Cmd::RunBench` → shell 监督线程（W5/R22 迁出）」 | 人工比对 |
| 3 | `Supervisor::spawn` 停机拒绝：`begin_shutdown` 后调用 `tracing::error!` + 立即 return（不 panic）；附测试 `spawn_after_shutdown_is_noop` | `crates/lt-orchestrator/src/supervisor.rs:72` | 测试绿；F15 消项 |
| 4 | 命令写入点统一（已裁决 A 案）：写入逻辑收敛进纯函数 `apply_settings_side_effects(&mut Settings, &Cmd)`（shell.rs 新增，handle_cmd 各写入场共用）；`Cmd::SetPadding` 臂补 `settings.sensevoice/whisper_pad_seconds = secs`（按 engine 分派）后再 publish+persist；`Cmd::IncrementalAsr` 臂补 `settings.incremental_asr/interim_interval`；lt-ui vad.rs:474/493/819/839 删发送前草稿预写（保留 dirty 标记） | `crates/lt-app/src/shell.rs:160-180`、`crates/lt-ui/src/windows/panel/vad.rs` | 纯函数单测（AppShell 需 winit 事件循环不可 headless 构造——测试直接驱动 `apply_settings_side_effects`）：`SetPadding{engine:"funasr",secs:1.5}` 后 `sensevoice_pad_seconds==1.5`；`IncrementalAsr` 后 `incremental_asr/interim_interval` 落位 |
| 5 | `OVERRIDE_KEYS` 注释修订：删「镜像」措辞，改为「UI 编辑对话框呈现清单（overrides 为 BTreeMap 透传，translate 侧无键枚举——增键即改此处）」 | `crates/lt-ui/src/state.rs:811` | 人工 |

**规模**：~80 行 + 2 测。**行为偏差**：无（E1-4 行为等价，仅写入点唯一化）。

### E2 值域枚举化（1 天，已裁决执行，D-79）

1. **lt-proto 新增**：
   - `pub enum EngineKey { FunAsr, Whisper, Qwen3 }` + `EngineKey::from_settings_str(&str) -> Self`（未知值 → `FunAsr` + `tracing::warn!` 单点回退）+ `impl Settings { pub fn engine_key(&self) -> EngineKey }`；roundtrip 测试（`ASR_ENGINES` 三值 + 未知值回退）。
   - `ProxyMode` 自 lt-download/lib.rs:76 迁入 proto（`Hub` 已在 layout.rs 不动，F10）+ 双向映射：`ProxyMode::from_settings(&str)`（迁入 orchestrator download.rs:136 `proxy_mode_from` 语义，**该本地函数随迁删除**——单一来源）与 `to_settings_str(&self) -> String`（写回 `settings.download_proxy` 持久层）；lt-download `pub use lt_proto::ProxyMode;` 保持兼容。
   - `Hub` 补双向映射 `Hub::from_settings(&str) -> Self`（`"hf"→Hf`，其余→`Ms` + warn——**消灭 download.rs:153 的静默 else**）与 `as_settings_str(&self) -> &'static str`（`Ms→"ms"`/`Hf→"hf"`，写回 `settings.hub` 持久层）。
2. **总线**：`EffectiveSettings` 增 `engine: EngineKey`，`derive()` 用 `raw.engine_key()` 填充（方案 §3.2.3 原设计字段复位）。
3. **散点切换**（F9 两个域内文件）：settings_bus.rs:94 钳制判定、pipeline.rs 引擎判定点、lt-ui vad.rs 的 `== "funasr"` 改 `settings.engine_key() == EngineKey::FunAsr`。
4. **契约改型**：`Cmd::StartDownload { hub: String, proxy: String }` → `{ hub: Hub, proxy: ProxyMode }`——**全链枚举化**：UI 发送点经 `settings.hub()`/`settings.proxy_mode()` 转换 → shell.rs:289-292 直收枚举 → `DownloadManager::start` 签名改收枚举 → `run_download` 的 `if hub_s == "hf"` 字符串分派删除；`succeed()` 写回持久层经 `hub.as_settings_str()`（`DownloadSucceeded.settings.hub` 保持 String——持久层不变，向导 13 键块不受影响）。**PROTO_VERSION 3→4**，登记 **D-79**（改型须评审——ADR-9 即评审记录）。
5. **derive 覆盖率测试**（ADR-13①）：settings_bus.rs 新增 `every_settings_field_is_classified`——serde_json 序列化 `Settings::default()` 取键集，与手工维护的 `WIRED_IN_DERIVE`/`RAW_DIRECT_READERS` 并集比对，差集非空即 panic 并打印缺失键名。

**验收**：`grep -rn '== "qwen3"\|== "funasr"\|== "whisper"' crates/lt-orchestrator crates/lt-ui` 非测试行 0 命中（lt-asr/lt-models/lt-app-main 单点分派边界保留，check_guards 注记豁免）；`Cmd::StartDownload` 全仓无 String 字段残留；覆盖率测试绿。**规模**：~300 行 + 6 测。

### E3 常量上移 + UI 边裁剪（半天，已裁决执行）

1. `DEFAULT_PROMPT`/`PROMPT_PRESETS`（translator.rs:78-89）与 `THINKING_STYLES`（thinking.rs:14）迁 lt-proto（新模块 `lt-proto/src/prompts.rs` + lib.rs re-export）；translator.rs/thinking.rs 改引 proto。
2. lt-ui 删 `THINKING_STYLE_VALUES`（state.rs:809），索引函数改用 `lt_proto::THINKING_STYLES`；`translation.rs`/`bench.rs` 全部 `lt_translate::` 引用改 `lt_proto::`。
3. `crates/lt-ui/Cargo.toml` 删 `lt-translate`；`check_deps.ps1` 白名单：lt-ui 允许集改 `proto, i18n, models`，新增 tl→proto 边；AGENTS §工作区结构同步。

**验收**：`grep -rn "lt_translate" crates/lt-ui`（含 tests/）0 命中；`cargo tree -p lt-ui` 无 lt-translate；全测绿。**规模**：~150 行移动 + 白名单 2 行。

### E4 监督器补强 + 文档真源勘正（1 天，已裁决执行，D-80 参数按默认）

1. **Backoff 实装**（supervisor.rs）：
   - `Entry` 增 `consecutive: u32`、`next_eligible: Option<Instant>`、`last_born: Instant`；
   - monitor 死亡分支：`Backoff` 时 `consecutive += 1`；超 `give_up_after` → 推 `ThreadDied{restarted:false, detail:"{name} 连续 {n} 次异常退出，已放弃重启（backoff 超限）"}` 并 retain 移除条目；否则 `next_eligible = now + min(base<<(n-1), max)`，到期 tick 才重生；线程存活超 `reset_after_ms` 清零计数；
   - `Policy::backoff()` 预设；Capture/AsrMain/TlWorker/LogBridge 四类出生点改用（pipeline.rs:581/694 + JobPool:88 + logging.rs bridge）；**动脉桥保持 Always**（注释写明理由）；
   - 测试 4 条：`backoff_respects_delay`、`backoff_gives_up_and_reports`、`backoff_resets_after_healthy_run`、`backoff_caps_at_max`。
2. **文档真源勘正**（F3/F13/F16）：architecture-v2.md §3.2.1 线程表改双实例分栏（ADR-8 收录）；§3.3 `ThreadRole` 清单勘正（Tray/Singleton 不入枚举的理由注记）；§3.7 行为偏差表补 D-78~D-80 三行指针（指向本方案 §6）；`Policy::Backoff` 实装注记；AGENTS 同步。
3. **行为登记 D-80**：常驻线程 panic 重启从「无限即时」→「指数退避、8 连败放弃并告警」。

**验收**：4 新测绿；panic 注入探针复跑（连续注入 8 次 → 第 9 次不再重生 + 放弃行可见）；方案文档与 `Policy` 枚举逐字一致。**规模**：~200 行 + 4 测。

### E5 向导保留标注 + 文档杂项清欠（半天，已裁决内容）

ADR-11 裁决后，本波由「删除」改为「**留空间的具象化**」：

1. **向导子系统现状地图**落档（内容见 §5.4，作为独立小节随本波同步进 AGENTS 待办区或本方案附录）；
2. **向导域标注**：setup.rs 与 `StartupFlow`/`WizardState` 顶部加模块注释「D-78：向导子系统保留为待开发面（2026-09-09 用户裁决），现状地图见 docs/architecture-v2-improvements.md §5.4；接线点三处：initial_visibility / Setup show 分支 / DownloadSucceeded→start_pipeline 链」；
3. **WD-6 首启轻引导横幅接口预留**（用户裁决「先留接口，之后开发」）——只落文档与注释，不实现：
   - 设置键预留（**本波仅设计登记，不新增键**）：WD-6 开工时以加法豁免新增 `first_run_banner_dismissed: bool`（serde default）；
   - UI 挂点预留：面板识别页顶部提示条（与 `pipeline_error` 同通道形态，互斥显示）；
   - 触发语义预留：settings.json 首次创建（`settings_io::load` 返回 None 分支）时视为首启；
4. **data-lifecycle 裁决回写**：docs/data-lifecycle.md §8 表增「裁决」列——C3=A（文档承认 VC Redist 前置，2026-09-09 用户裁决：装 Redist 对用户只有好处，重复分发微软系统组件多此一举）；C1/C2=无限积累不清理（用户裁决，理由：日志/转写属用户数据与排障证据，自动删除越权）；C11/C8 标注已被后续工作解决（uv 钉版 96ac36a / W1 fatal_early）；
5. **文档杂项清欠**（用户授权「其余你自己处理好」）：**C3 VC Redist 前置文档化（§6 已裁决 A 案的执行落点）：distribution.md 分发要求节 + README.txt 明写「需预装 Microsoft Visual C++ 2015-2022 x64 Redist」**/ C4 卸载说明补 lnk+models_dir（distribution.md + README.txt 核对补齐）/ C5 AGENTS silero 措辞改「onnxruntime.dll 启动解压；silero 内存直载」/ C6 用户文档补 models_dir 不跟随 logs/ort 的提示 / C7 数据页清理提示行加「仅清理注册内模型」（一行 UI + i18n 键）/ C9 记录不处理（D-75 `.bak` 链已兜底，ReplaceFileW 收益不抵风险）/ C10 distribution.md 数字刷新（59MB→实测值重录）/ C12 AGENTS ignored 计数勘正（7）/ AGENTS「data-lifecycle 12 项整改候选待裁决」行勘正（候选已全部裁决，指针指本文件 §6）。

**验收**：全测绿；data-lifecycle §8 有裁决列；向导域注释三处就位。**规模**：~120 行（多为文档）+ 1 行 UI。

### E6 守护与 CI 收口（半天，已裁决：严格度 = A 硬 gate）

1. `scripts/check_dead_contract.ps1`（ADR-13②）：正则抽取 **lt-proto 全部契约 `pub enum`**（events.rs：Cmd/UiEvent/AppCommand/ThreadRole/DownloadPhase/BenchEvent/QueueId/CaptureEvent/AudioRole/ExportFileMode；asr_result.rs 契约枚举）变体名 → 逐个统计 crates 内定义文件之外的非注释命中 → 命中 <2 且不在 `$ReservedWhitelist`（含 `DownloadPhase::Start/Integrity`）→ 非零退出并列嫌疑变体。以既有活变体做自检校准。
2. `check_guards.ps1` 增补注记：单点分派边界字符串比较豁免清单（lt-asr/manager+engines、lt-models/cache、lt-app/main worker 分派——ADR-9 边界论）。
3. `.github/workflows/ci.yml` 增两个脚本调用。
4. AGENTS：§5.1 功能落位表收录 + ADR-14 三条纪律收录 + 本方案链接。

**验收**：CI 全绿含新脚本；人为加一个无人消费变体验证脚本红、回滚后绿。**规模**：~120 行脚本。

**波次依赖**：E1→E2→E3（E2/E3 同触 proto 与 settings_bus，禁并行）→E4→E5→E6（E6 校准依赖 E5 完成）。总规模 ≈ 净增 500 行、净删 200 行（E5 删除项取消后大幅缩小）、测试净增 ~13，预计 3.5~4 个工作日。

---

## 5. 开发流程体系（之后每个功能怎么开发）

### 5.1 功能落位表（开工前先查表，防「在错的地方长肉」复发）

| 功能类型 | 触达 crate（按依赖序） | 契约面 | 必配测试 |
|---|---|---|---|
| 新 ASR 引擎 | lt-models 注册表 → lt-asr（engine+profile+worker 臂）→ lt-app worker 分派 → 总线 derive（若超时档案）→ lt-ui 三值表 | `ASR_ENGINES` 值域 + `WorkerOptions` + Settings 字段 | 注册表 roundtrip + fake worker + 探测/缺失清单 |
| 新翻译模型/能力 | lt-translate → `ModelConfig` 字段 → shell `to_bench_model` → lt-ui translation 页 | Settings 字段（走覆盖率测试归类！） | translator 集成测 + 面板 headless |
| 新窗口/浮层 | 仅 lt-ui：`WinId` + `windows/` 模块 + dispatch 解构臂 + WindowManager 可见性 | 无（WinAction 即可） | headless 冒烟 + widget_stability |
| 新设置项 | lt-proto Settings（+覆盖率测试归类）→ 面板页 → 若需运行时生效：**必须进 `derive()` 视图** | Settings 加法（豁免评审，过覆盖率测试） | 往返测试 + 视图跟随测试 |
| 新下载源/hub | lt-download → lt-proto layout → 下载 targets（`current_missing`） | Hub/布局契约 | download_integration |
| 新托盘/菜单项 | lt-ui tray ids → proto `AppCommand` 变体 + `from_menu_id` | AppCommand 加法 | menu_id 映射测试 |
| 新后台任务线程 | **只许** `Supervisor::spawn`（新 `ThreadRole` 变体）；一次性任务 Policy::Never | ThreadRole 加法 | 监督策略测试 |
| 首启引导/向导演进 | lt-ui setup.rs + StartupFlow/WizardState + shell `DownloadSucceeded` 链 | `Cmd::StartDownload`/`Cmd::Stop`/`DownloadSucceeded`（§5.4 地图） | headless + 启动流状态测试 |

### 5.2 波次纪律（ADR-14，已裁决生效）

见 §3 ADR-14 三条。执行语义：违反时 commit message 自曝 + 交用户裁决。

### 5.3 契约治理三层（定稿后常驻）

加法豁免冻结规则（不变）→ derive 覆盖率测试（E2，防设置静默不生效）→ 死契约守卫（E6，防变体只增不减；向导域预留项入白名单，ADR-11 配套）。

### 5.4 向导子系统现状地图（D-78 交付物：后续开发的设计底稿）

> 本节就是「留出空间」的具象形态：所有后续开发需要的入口、链路、契约已盘点在案，无需重新考古。**向导当前未接线（D-19）**：`StartupFlow::Wizard/DownloadMissing` 零生产者，Setup 窗创建即隐藏。

**存量代码面**：

| 面 | 位置 | 内容 |
|---|---|---|
| 窗口 UI | `crates/lt-ui/src/windows/setup.rs`（239 行） | 三态对话框：向导 `wizard_dialog_ui` / 缺模型下载 `download_dialog_ui` / 模型加载 `load_dialog_ui` |
| 状态机 | `crates/lt-ui/src/state.rs` | `WinId::Setup`、`StartupFlow{Wizard, DownloadMissing, Ready}`、`WizardPhase`、`WizardState`、`AppUi.startup: StartupUi` 域 |
| 宿主分支 | `crates/lt-ui/src/app.rs` | Setup 窗创建（:152）、标题动态化（:158-164）、显示/隐藏分支（:745/:752/:764/:819/:1218） |
| 可见性总开关 | `crates/lt-ui/src/state.rs:2145-2163` | `initial_visibility` 的 `startup_pending` 门——**向导期隐藏主界面三窗、只显 Setup 窗**；这是未来接线的总开关（现为恒 false 路径） |
| 退出契约 | `Cmd::Stop`（events.rs:353） | 向导「关闭」按钮 → 请求退出；shell.rs:296 处理臂 + :502-507 特判转 `AppCommand::Quit` |
| 完成链路（**活跃保留**） | `crates/lt-app/src/shell.rs:444-462` | `UiEvent::DownloadSucceeded` → `if !self.started { start_pipeline }`——**向导完成后管道冷启动的入口现成且活着** |
| 首启下载语义 | `crates/lt-orchestrator/src/download.rs` | `DownloadManager.first_launch` 标志 + `succeed()` 13 键默认块（:319-335，原版向导 accept 语义：固定 sensevoice-small + 推荐参数） |
| 共用下载链 | `Cmd::StartDownload`/`CancelDownload` | 与识别页按需下载**同链路**（活跃） |
| i18n | zh/en yaml | 向导相关键集在册（window_setup/window_download/btn_* 等） |

**三处接线点（未来开发入口）**：

1. **首启判定**：`AppUi::new` 或 boot 期（settings.json 首次创建时）把 `StartupFlow::Ready` 置为 `Wizard`/`DownloadMissing`——`initial_visibility` 的 `startup_pending` 门随即生效（主界面隐藏、Setup 窗显示）；
2. **Setup 窗 show**：app.rs:1218 分支已有（当前不可达），激活即可；
3. **完成收尾**：`DownloadSucceeded`→`start_pipeline`（冷启动）与 `Cmd::Stop`→退出确认两条链**已存活**，无需新建。

**与 confirm 流的边界（F8，勿越界）**：`ModalUi.quit_requested` 的活跃生产者是退出确认模态（confirm.rs:70）；向导副本（setup.rs:199）改造时**不得**动 confirm 流语义。

**WD-6 首启轻引导横幅（接口预留，用户裁决待开发）**：见 E5-3 三条预留（设置键/UI 挂点/触发语义）。横幅与向导的关系建议届时裁决：横幅是向导的轻量替代还是并存（互斥显示）。

---

## 6. 裁决记录（2026-09-09 用户裁决汇总）

| # | 事项 | 裁决 |
|---|---|---|
| 1 | 本方案落档为单一文档 | ✓ 本文件（docs/architecture-v2-improvements.md） |
| 2 | 向导子系统 | **保留并定位为待开发面**（D-78，维持并强化 D-19）；§5.4 地图交付 |
| 8 | 流程纪律三条（ADR-14） | ✓ 采纳，违反时 commit 自曝 |
| 9 | 验收节奏 | **A 案**：E1+E2+E3 一批交付审看、E4+E5+E6 第二批 |
| C3 | VC Redist | **A：文档承认前置**。用户理由：装 Redist 对用户只有好处；自己重复分发微软系统级组件多此一举（执行落点 E5-5） |
| C1 | logs 累积 | **无限积累，不清理**（用户裁决） |
| C2 | transcripts 存量 | **无限积累，不清理**（用户裁决） |
| C4~C12 其余 | 文档小改 | 授权项目侧处理 → E5-5 承接（C9 记录不处理，D-75 已兜底） |
| 11 | 实机走查 | 后续单独拉方案逐项过（不进本方案） |
| WD-6 | 首启轻引导横幅 | 先留接口（E5-3 预留），之后开发 |
| WD-8 | 检查更新按钮 | **需要，更新源 = GitHub Releases**（2026-09-09 二轮裁决 a 案：发布走 GitHub，按钮查最新 tag；进 §8 路线图第一优先） |
| PH-6 | 参考图 GPU 中性化重拍 | **不做**（用户立场：GPU 长期不支持，不值得投入） |
| UX 三期 | 保存失败 UI 流/错误译文样式 | 缓，后续开发时考虑 |
| whisper sha256 | 其余五档完整性校验值补齐 | **a 案：有网时段安排补齐**（届时提醒用户在真机跑一次登记脚本；AH-5 链路收尾） |
| 3~7 | D-79 改型 / E3 白名单 / D-80 Backoff / 死契约守卫严格度 / 写入点方向 | **全部按推荐案通过**（2026-09-09 二轮裁决）：③ D-79 执行；④ E3 白名单变更通过；⑤ D-80 接受、参数默认（500ms/30s/8 次/60s，动脉桥 Always）；⑥ A 硬 gate；⑦ A 案（后台写入点唯一） |

---

## 7. 非目标（明确不做）

| 项 | 理由 |
|---|---|
| Settings 字段直接枚举化 | ADR-9：毒化加载语义 + 改动面失控；透镜已达成类型安全收益 |
| `vad_mode`/`silence_mode` 枚举化 | F9 实测比较已收敛在单点边界（R2 机制），无散点可治——枚举化是无读者实体 |
| 合并双监督器 | ADR-8：需引入分组实体，收益仅文档 |
| 删除向导子系统 | ADR-11：用户裁决保留为待开发面（D-78） |
| logs/transcripts 清理策略 | 用户裁决无限积累（§6 C1/C2） |
| VC Redist 随包分发 / crt-static | 用户裁决 C3=A 文档承认前置 |
| PH-6 参考图重拍 | 用户裁决不做（GPU 长期不支持立场） |
| `pipeline.rs`（2146 行）内部拆分 | 无当下痛感；**触发条件**：下次任何波次需同时触碰其中 ≥3 个职责域时，先拆（asr_thread/tl_rig/capture_factory 三模块）后改 |
| lt-ui 体量削减 | 域拆分已由 AppUi+借用检查强制；**触发条件**：单域文件超 3.5k 行或跨域写意图再出现 3 处以上 |
| is_thread_finished 句柄化 | 单一查询者（DownloadManager）；出现第二个查询者再做 |
| egui/winit 升级、双窗 DComp、流式 ASR | 沿用 architecture-v2 §7 既有裁决 |
| GPU 任何形态的支持 | 用户长期立场 + 既有硬约束（纯 CPU） |

---

## 8. 后续路线推荐（E 波之后，「继续开发」的方向建议）

基于本轮裁决的信号（向导要继续开发、WD-6/WD-8 都要、C3 走文档前置、UX 三期缓），项目轨迹明显指向**面向真实用户的分发成熟**。推荐主线：

### 第一优先：分发成熟线（与用户裁决直接衔接）

1. **E 波收尾 + 实机走查**（单独方案，用户已要求逐项过）——架构与行为的地基确认；
2. **向导子系统正式开发**（用户主诉的后续开发）：以 §5.4 地图为底稿，裁决横幅与向导的关系（WD-6 轻引导 vs 完整向导：推荐**横幅先行**——一次会话顶部提示条 + 「查看引导」入口，成本低、可先验证需求，向导完整流留作二期）；
3. **WD-8 检查更新**：渠道已裁决 = **GitHub Releases**（查最新 tag 对比当前版本）。设计要点随开工波另立工作包：按钮在设置/关于区（WD-2 版本行旁）、无网络/请求失败静默提示不打扰、版本比较用 semver 语义、跳转 Releases 页下载（不内置自更新——单 exe 分发模型下用户手动替换最稳）。**前置依赖：公开发布（D-18 翻转）后才有真实可查对象；按钮可先行落地为「检查更新（需联网，当前版本为最新/查询失败）」形态**；
4. **公开发布决策（D-18 翻转时机）**：WD-6/WD-8/用户文档齐备后，这是「从本地 zip 到公开」的唯一闸门，届时一并定版本号策略与发布渠道。

### 第二优先：体验补全线（已有钩子，增量小）

5. **UX 三期**：设置保存失败 UI 流（`SettingsPersistFailed` 事件 W2 已备好，只差 UI 消费）+ 错误译文样式——都是「有事件无界面」的收尾；
6. **WP-9 性能预算实机**（启动<2s/空闲 CPU<1%/8h 长跑/内存回收）：可并入实机走查批次。

### 第三优先：能力扩展线（架构已备好接位）

7. **第 4 个 ASR 引擎**（按 §5.1 落位表，模式已在 nano/qwen3 验证两轮，边际成本递减）；
8. **多语言 UI 扩展**（i18n 体系现成，加语言 = 加 yaml + 键集测试自动守护）；
9. **whisper 其余五档 sha256 补齐**（已裁决 a 案：有网时段执行——登记脚本 + 真机跑一遍五档下载，回填注册表校验值；到时提醒用户配合跑一次）。

**不建议现在排**：跨平台（硬约束维持）、GPU（用户长期立场）、流式 ASR worker（无需求拉动）。

---

## 9. 风险与回滚

- **E2 是唯一契约改型波**：UI/shell 双端同波原子迁移（违者编译失败，无静默窗口）；PROTO_VERSION 递增；单独 revert 可行。
- **E4 Backoff 参数**：give_up_after=8 ≈ 91s 尝试窗，实机走查后可调，常量改动无结构风险。
- **E5 无删除风险**（裁决后本波纯文档/标注/一行 UI）。
- 每波全绿 + 单独 revert；节奏 A 案下第一批（E1~E3）交付审看通过后才开第二批。
- 死契约守卫（E6）若误报：预留白名单显式编目 + 自检校准测试；裁决严格度时已知此权衡。

---

## 10. 局限性

- 行号锚基于 `1d608d2` 基线，施工时漂移以「锚点符号名 + grep 验收」为准（每卡验收列已按此写法）。
- Backoff 参数、死契约守卫阈值未经实机/长期运行校准，走查批次留调参项。
- E2-5 覆盖率测试只走查 Settings **顶层键**——嵌套结构（Style/SubtitleMode/ModelConfig 内部）新增字段不在断言面；该局限写入测试注释，嵌套面靠 ADR-14「收口两问」纪律兜底。
- whisper sha256「到时提醒」是无定时的人工约定——登记为下一次有网会话的待办（§8 路线 9），不设自动提醒。
- §8 路线为推荐非承诺，逐项开工前仍按本项目惯例立工作包文档。
