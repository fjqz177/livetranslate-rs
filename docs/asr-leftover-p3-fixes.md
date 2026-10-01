# 识别链留档 P3 缺陷批修（D-130）

> **状态**：定稿 ｜ **日期**：2026-10-01 ｜ **触发**：用户令「把还存在的 bug 都修复，按 Matt 流程」——对象 = 2026-10-01 逐条对现行代码复核后仍成立的 6 条留档缺陷（asr-chain-robustness §10 K1/K3/K4/K5/K6/K7；复核结论见该日提交 df9b676 的复核记录） ｜ **前缀**：LF（leftover-fixes）
> 完工后本文归 `docs/archive/`。

## 一、背景取证（六条缺陷现行证据，2026-10-01 复核）

| # | 缺陷 | 现行证据 | 用户可见症状 |
|---|---|---|---|
| K1 | 退出瞬间恰逢识别超时 → `recover` 无停机感知，原地重生 worker 等 `ready_timeout` 180s | `lt-asr/manager.rs:141`（180s）+ `:361-392`（recover 直接 spawn_ready）+ `supervisor.rs:247`（join_role 无预算 join）+ `pipeline.rs:1771`（停机序第 4 步定点等） | 退出后进程逗留分钟级（任务管理器「退不掉」） |
| K3 | capture 循环体 6 处裸 `vad.lock().unwrap()`，锁中毒即 capture 死、尾巴丢、退出白等一个预算 | `lt-audio/audio/capture.rs:171/187/192/207/211/250`；退出尾巴 `:230-233` 已有 `into_inner` 防御（不对称） | 极低概率：退出多等 ≤15s + 最后半句丢 |
| K4 | `finish_ok` 先推 UI 回执后落盘转录，缝隙 panic →「屏幕有译文、all 文件记无译文」 | `pipeline.rs:857`（push）早于 `:863`（write）；对照 `fail()` 反序（`:883` 落盘 → `:899` 回执） | 极低概率：转录与屏幕矛盾 |
| K5 | panic 兜底文案硬编码中文 `"panic（载荷非字符串）"` | `pipeline.rs:131`；TlJob panic 分支 `:99` 直用 | en 界面 tooltip 混出中文 |
| K6 | `Shell::shutdown` 取消了 probe 却漏了 bench：退出 join_all 等基准线程跑完剩余模型（timeout×轮次×模型数可达分钟级） | `lt-app/shell.rs:175-185`（只 store probe_cancel）+ `bench_cancel` 字段 `:95` 仅 CancelBench 置位；基准线程 Policy::Never 被监督，join_all 必等 | 退出时若基准在跑 → 进程逗留 |
| K7 | `write_translation` 在转录 disabled 时早退不清 pending，与 `finalize_no_translation`（先 remove 再判 enabled）不一致；残留靠 `close()` 兜底 | `lt-audio/transcript.rs:97-99` vs `:127` | 会话中途关转录再开会话，all 文件错配对（低概率） |

测试盲区说明：K1/K3/K6 的退出路径此前零自动化覆盖（需真引擎/真线程序，asr-chain §10.2 已如实登记）。

## 二、方案裁决（D-130；按档案既载药方 + 复核时补勘，全部按推荐自主施工）

| # | 项 | 裁决 | 理由 / 备选取舍 |
|---|---|---|---|
| 1 | K1 | **停机抑制 recover 重生**：`AsrManager` 增 `shutdown: Arc<AtomicBool>` 注入位（新构造参数，默认 never-stop 保旧测试零改动），`recover` / `transcribe` 空窗重建 / `maybe_recycle_if_idle` 重建三口在 spawn 前查标志——置位即不重生，返回 `Unavailable("停机中…")`；`run_asr_thread` 把既有 `stop` 标志接线进去 | 档案 §10.1 K1 载明的两条治法之一（「退出期抑制 recover」）；另一条「停机中断在途识别」须动 worker 协议（取消令牌跨 IPC），在途识别本身有引擎档案封顶（qwen3 最坏 ~70s），收益/风险比不值——**不做**，留档 |
| 2 | K3 | **中毒容忍助手**：lt-audio 新增 `pub fn lock_vad(&Mutex<VadProcessor>) -> MutexGuard`（`unwrap_or_else(into_inner)`），替换 capture 循环 6 处 + pipeline 侧 4 处（`:2621/:2874/:2892/:2925`） | 中毒数据本身不坏（VAD 状态是启发式缓存），取回即用与退出尾巴既有防御同哲学；只修 capture 6 处会把同一死法留给 ASR 线程（pipeline 侧 unwrap 中毒即死）——一并替换为类灭。测试侧 `vad_obs`（capture.rs:508/521）与 `monitors` 锁不在本批（测试代码/非 vad 锁，留档不动） |
| 3 | K4 | **反序对齐**：`finish_ok` 落盘（write_translation）提前到推回执（UpdateTranslation/snapshot）之前 | 档案 §10 载明「对齐 fail() 的先落盘后回执序可根除」；**严格序的机器断言无缝可入**（TranscriptWriter 内部为 parking_lot 非毒锁、EventSink 为具体队列，两调用间无法注入观察点）——按 diagnosing-bugs 纪律如实记录「缝隙不存在」，行为测试锁双副作用，序靠代码评审兜 |
| 4 | K5 | **文案经 Msg 注入**：i18n 新键 `panic_non_string_payload`（zh/en 同步）；`Msg` 存入 JobPool → TlJob，`panic_detail(payload, fallback)` 增参 | orchestrator 禁依赖 lt-i18n 的既有注入纪律（AGENTS §3）；String/&str 载荷分支本就可读，仅兜底分支换键 |
| 5 | K6 | **停机取消基准**：`Shell::shutdown` 增 `bench_cancel.store(true)`（先于 pipeline.stop，与 probe_cancel 并排）；`ThreadRole::Bench` 文档注释补「停机禁 join_role(Bench)，经 bench_cancel 模型边界收敛」 | K6 复核时补勘的真实缺口（档案原记录只载「误用隐患」，实查发现退出等基准是活症状）；lt-proto 仅注释、零契约变更 |
| 6 | K7 | **pending 清理一致性**：`write_translation` 把 `pending.remove` 提到 enabled 检查之前（与 finalize 同构） | 一行级；语义 = 「译文已产出」这个事实不因转录开关而丢账 |

**契约面**：零 lt-proto 结构变更（K6 仅注释）；i18n 纯新增键（zh/en 同步）；`AsrManager` 构造签名变更属 crate 内 API（生产唯一调用点 pipeline.rs 同步）。

## 三、施工卡（每条一红一绿，TDD）

1. **K7**（transcript.rs 内文件测试）：`write_original → set_enabled(false) → write_translation` → 断言 pending 空（同文件测试可达私有字段）。红：现实现 pending 残留。绿：remove 前置。
2. **K5**（pipeline.rs 测试）：`panic_detail` 纯函数直测——非字符串载荷 + en/zh 两 fallback 文案各返其文；i18n 键存在性由既有键集一致守护兜。红：现签名无参（编译红=接口红，行为红=键缺失）。
3. **K4**（pipeline.rs 测试）：`finish_ok` 行为锁——tempdir 转录 + 断言 all 文件含译文块且事件已推（现绿，防回归锚）；严格序无缝如实记录（§二.3）。
4. **K3**（vad.rs 测试）：毒化真 `std Mutex<VadProcessor>`（持锁 panic 线程）→ `lock_vad` 取回数据可用（`is_speaking` 不 panic）。红：等价实现 `.unwrap()` 下毒锁必 panic。
5. **K1**（manager_flow.rs 集成测试，真 fake worker）：计数 spawner + `ensure_started_explicit` 起活 worker → `transcribe_base_secs=0` 造 Timeout 进 recover 臂 → 断言：置 shutdown 后 spawner 计数不变 + 错误为 Unavailable；对照组（不置）计数 +1 走重生。红：现实现无视标志必然重生。
6. **K6**：`Shell::shutdown` 一行（与 probe_cancel 同款，无廉价测试缝——headless Shell 构造重，靠评审 + 实机走查「基准在跑退出即时收敛」兜）。

## 四、验收基线

- `cargo test --workspace` 全绿（基线 696 + 新增 ~5）；precommit 三闸过。
- 收口：本文归档 + decisions 已随定稿登记（D-130）+ 走查 issue（清单见 §五）。

## 五、遗留走查（移交实机）

1. 常规退出无回归（秒级，无新告警）——K1/K6 改动面在停机序上，须过一遍。
2. 基准测试在跑时退出：进程即时收敛（K6；原为等剩余模型跑完）。
3. en 界面触发非字符串 panic 载荷（极难人为制造，可信静态断言）→ 仅核对 tooltip 无中文（K5）。
4. K1 的「退出×识别超时」三重巧合无法人工复现——抑制逻辑由集成测试锁，实机仅验退出路径无回归。
