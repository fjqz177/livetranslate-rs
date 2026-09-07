# ASR 子系统加固与状态机收口方案（docs/asr-hardening.md）

> 状态：**阶段二活跃文档**（2026-09-08 五路并行审计定稿：进程/IPC、引擎实现、pipeline 集成、注册表/下载链路、测试/文档一致性五切面；全部 P0/P1 发现经主线程逐条复核源码确认）。施工包 **AH-1~AH-10**，新偏差自 **D-26** 起。
>
> **r1.1（2026-09-08）：✗ AH-1~AH-10 施工完成。** 363 测全绿（净增 14），新码 clippy 零告警；提交链 53823ae(AH-9a)→96df48a(AH-2)→f6a5881(AH-1)→8d4bf55(AH-3)→37f2902(AH-4)→8601e3c(AH-6)→b0f623b(AH-7)→5fdb375(AH-8)→b96e6ca(AH-5)→4ab7f1f(AH-10)。要点回填：①sha256 实测登记 14/20 文件（sensevoice2+nano6+qwen36+whisper tiny，本地缓存 sha256sum，qwen3 decoder 前缀与 §4.2-asr 交叉核对一致；whisper 其余五档待有网络实机补齐=渐进登记）②新增 FailKind::Checksum（确定性损坏不重试不回落）③total=None 拒绝收尾 + 下载客户端 no_gzip ④磁盘预检 GetDiskFreeSpaceExW。遗留：GUI 冒烟脚本 A/B/C 待实机（mic/loopback 实时环境）；sensevoice.rs 迁入 engines/ 目录（纯组织项）缓行；T1 qwen3 长样例校准待实机。
> 行号基准 = 2026-09-08 现场（e2d2035）。审计方法：五子代理独立深审 + 主线程对全部 P0/P1 及关键 P2 证据链二次核实；原版对照直接回读工作区副本 `LiveTranslate/main.py`。

---

## 0. TL;DR

整体结论：**三引擎参数基线、崩溃恢复链、下载链测试覆盖扎实（349 测实测全绿复核通过），但存在 1 个新用户必经路径上的 P0 死角与 4 个 P1 级缺陷**。问题集中四条主线：

1. **恢复状态机只接了一半入口**——"请求间隙死亡"与 `client=None` 空窗都走进死胡同；
2. **ASR 线程设置快照陈旧**——运行时改识别语言可致字幕全灭；
3. **"引擎→条目"分派臂散布 8 处**——data.rs 已实际漂移一次（qwen3 不可见不可删）;
4. **可观测性缺失**——丢段/mic 失效/unavailable 刷屏全部静默，8h 长跑（WP-9）前置条件不成立。

### 发现总表

| # | 严重度 | 一句话 | 证据位置 | 归属 |
|---|---|---|---|---|
| H1 | **P0** | 待命态（模型未缓存）不消费 `tl_switch`，"下载完成→引擎切换"无法唤醒，首启用户必须重启 | `pipeline.rs:789-798`（待命循环只排段）；`pipeline.rs:823`（tl_switch 唯一消费点在主循环）；`shell.rs:245-260`（DownloadSucceeded → SwitchEngine，注释自证设计意图） | AH-1 |
| H2 | **P1** | worker 死于"两次请求之间"永不自动重启：预检 `Status` 错误不走 recover，每段重复 Failed，状态行恒挂"[cpu]" | `client.rs:260-265`（预检）；`manager.rs:253-256`（Status/Io 直接上抛） | AH-2 |
| H3 | **P1** | recover/RSS 回收中 spawn 失败留下 `client=None` 且非 unavailable，永不重试重建（一次瞬态失败=永久死亡） | `manager.rs:370-379`、`manager.rs:408-417`、`manager.rs:223-225` | AH-2 |
| H4 | **P1** | ASR 线程持启动时 settings 快照：运行时改 `asr_language` 后段过滤仍用旧值（zh→en 场景**丢弃全部段、字幕静默全灭**）；pad 同理在引擎切换时静默回退 | `pipeline.rs:426`（快照随闭包固定）；`pipeline.rs:990`/`1199`（过滤读快照）；`pipeline.rs:858-862`（切换装配读快照 pad）；对照 `target_language` 有同款运行时快照（`pipeline.rs:747,841-846`） | AH-3 |
| H5 | **P1** | interim 的 peek→识别→trim 非原子，识别期间 VAD flush 会致 trim 误裁**下一段**头部音频。**原版同拓扑**（见 §1.3），非移植偏差，但竞态后果真实 | `pipeline.rs:1068`（peek 即解锁）、`:1077`（秒级推理无锁）、`:1129-1131`（回头 trim）；`capture.rs:140-144`（采集线程持锁写 VAD） | AH-4 |
| H6 | P2 | `shutdown()` 文档承诺"超时→kill"但实现无 kill，`finish_stop` 无界 `child.wait()`；worker 异常滞留时**整个应用退出挂死** | `client.rs:218`（文档）、`:219-244`（实现）、`shell.rs:53-55`（UI 线程 join） | AH-2 |
| H7 | P2 | total 未知时跳过长度校验（`gzip` feature 使 total=None 现实可达）+ 注册表下限刻意远低实际 → 截断文件可被永久判"已缓存"成死局（探测已缓存→下载幂等直成功→引擎加载失败） | `download/mod.rs:441-450`；`registry.rs:21-25`（qwen3 decoder 实际 756MB/下限 350MB）；`backend.rs:174-178`（幂等直成功）；`Cargo.toml:66`（gzip feature） | AH-5 |
| H8 | P2 | 全链路无内容哈希校验；docs 仅记录 sha256 **前 16 位**（§2.2/§4.2），代码零消费。D-24 下 hub=ms 唯一路径即第三方镜像 hf-mirror | `registry.rs:5-26`（无哈希字段）；`docs/asr-engine-expansion.md:297`（"LFS sha256（前 16 位）"） | AH-5 |
| H9 | P2 | 数据/存储页缓存扫描漏 qwen3：**941MB 模型在应用内不可见、不可删（含"删除全部"）**——分派臂漂移的既成事实 | `data.rs:44-77`（只扫 FUNASR_KEYS + 硬编码 whisper 仓，grep qwen3 零命中） | AH-6 |
| H10 | P2 | "引擎→条目"分派臂 8 处手写无编译期强制；`is_asr_cached` 自称统一入口却无生产调用方 | `cache.rs:146/179`、`vad.rs:197-242/444-457`、`pipeline.rs:648-724`、`data.rs:56`、`vad.rs:24`（ENGINES 表） | AH-6 |
| H11 | P2 | 两级音频队列满=静默丢最旧，无日志无计数：引擎装载期/峰值时段丢段完全不可诊断 | `audio/mod.rs:169-176`（push 丢最旧零记录）；`pipeline.rs:42,344`（cap=16/100） | AH-7 |
| H12 | P2 | mic 流读错误静默 break、无重开（对照 loopback 有完整 warn+0.5s 重开链路）：长跑中 mic 被抢占后无声消失 | `wasapi_win.rs:337-353`（drain_mic）vs `:476-491`（loopback 恢复） | AH-7 |
| H13 | P2 | unavailable 稳态下每段重复 warn + AsrUnavailable 事件刷屏（UI/日志窗被无效流量占据） | `manager.rs:223-225`（不短路）；`pipeline.rs:1022-1027`（每段发事件） | AH-7 |
| H14 | P2 | worker 子进程 tracing 全部失效（`--asr-worker` 分支在 `logging::init` 之前 return，无 subscriber）；sherpa 系加载失败错误止步"创建识别器失败"（根因只在 C++ stderr，需开 debug 才可见） | `main.rs:23-25` vs `:42`；`nano.rs:119-121`/`qwen3.rs:97-99`/`sensevoice.rs:129-130`（同型 `.ok_or_else`） | AH-7 |
| H15 | P2 | qwen3 `MAX_TOTAL_LEN=512` 的安全性论证依赖"VAD 默认 8s"，但 UI 滑杆允许 30s 且引擎层无守卫——超长段输出可能静默截尾 | `qwen3.rs:19`；`vad.rs:206`（VadSettings::default 15s）；UI 滑杆 2–30s；`settings.rs:94`（默认 8.0） | AH-8 |
| H16 | P2 | nano `set_language` 重建识别器（重载 963MB，本机 4.15s）受客户端 10s 命令超时约束：慢机超时→kill+重启烧配额 | `nano.rs:155-164`；`client.rs:200-216`（`min(10s, request_timeout)`） | AH-8 |
| H17 | P2 | 真实 `worker::run` 主循环零测试：fake worker 独立复刻主循环（含行为差异，如 SetLanguage 错误处理），14 个 IPC/manager 测试全部只经 fake 循环 | `fake_asr_worker.rs:58-115` vs `worker.rs:30-100`；对照 `worker.rs:13` 已有 `EngineFactory` 注入设计 | AH-9 |
| H18 | P2 | ignored 探针硬编码本机绝对路径；wav 缺失时"跳过 continue"，**全部 wav 缺失则转写断言静默滑过而测试仍绿** | `nano.rs:209`、`qwen3.rs:197`；`nano.rs:227-229`/`qwen3.rs:250-253` | AH-9 |
| H19 | P1(文档) | AGENTS.md"lt-ui 不得依赖 lt-models/lt-translate"与 `lt-ui/Cargo.toml:10,13` 主依赖直接矛盾（Cargo 注释证明系有意决策，文档过时）；"313 测/2 ignored"过时（实测 349/5） | AGENTS.md 分层规则与构建节 | AH-10 |
| H20 | P3 | 帧写入逐样本 `write_all` 打无缓冲管道：每 8s 段 ≈12.8 万次 syscall | `frame.rs:119-131`；`client.rs:117`（裸 ChildStdin） | AH-10 |
| H21 | P3 | 杂项：strip 扫描器与原版正则在畸形标签语义分歧且三处手抄 / `ort` 死依赖 / `from_value_compatible` 类型不匹配静默全量回退默认 / SOURCES.md 三处瑕疵 / `language_name`·`words` 契约字段零消费空转 | `engines/mod.rs:41-57` vs 原版 `asr_funasr_nano.py:106`；`lt-asr/Cargo.toml:8`；`settings.rs:151`；`assets/SOURCES.md:22,24,75` | AH-10 |

**明确不施工（观察项）**，见 §7：帧协议魔数/重同步（stdout 污染纵深）、退出最坏 60s 阻塞、MAX_FRAME 256MB 分档。

---

## 1. 审计证据基线（复核确认的关键事实）

### 1.1 实测基线

- `cargo test --workspace`：**349 passed / 0 failed / 5 ignored**（AGENTS.md"313 测/2 ignored"已过时——nano/qwen3 探针转正后 ignored 实为 5 个：whisper×2 双门控合规、nano/qwen3 探针硬编码路径、hf-mirror 真网络下载）。
- `cargo clippy --workspace --all-targets`：60 warning / 0 error（存量分布与"新码零告警"声称相符）。

### 1.2 H1 的设计意图与实现脱节（P0 定性依据）

`shell.rs:249-251` 注释明写：运行时下载完成后"以当前设置重发引擎切换，worker 用刚下载的模型装配，**同时解除未缓存切换时的 AsrUnavailable 待命态**"——但 `pipeline.rs:789-798` 的待命循环只 `pop_timeout` 段队列，`tl_switch.try_recv` 仅存在于主循环空闲分支（`:823`），待命路径在 `:798` 就 `return` 了。**D-19 首启直进主界面是默认路径 → 这是新用户的必经旅程。**

### 1.3 H5 的原版对照（决定修法）

原版 `_do_interim_asr`（`LiveTranslate/main.py:1411`）同样**只在 peek 时持 `_vad_lock`（:1414-1416），识别期间不持锁**，trim 时再锁——竞态窗口原版同样存在。结论：这是**原版既有拓扑**而非移植偏差；修复属"超原版加固"，取 D-27 编号。同时排除"全程持锁"方案（见 DEC-3）。

### 1.4 复用既有机制可消灭大部分改造量（关键盘点）

| 既有机制 | 位置 | 本次复用 |
|---|---|---|
| `EngineFactory<E>` 泛型注入 | `worker.rs:13` + `main.rs:108-176` 四臂示范 | AH-9a：fake worker 直接调 `worker::run`，**零新抽象** |
| `target_language` 运行时快照模式 | `pipeline.rs:747` + `TlSwitch::TargetLanguage`（:841-846） | AH-3：`asr_language`/双 pad 照抄同款 |
| `recover()` 统一恢复入口 | `manager.rs:354-381` | AH-2：把漏接的错误类别汇入即可 |
| 编译期不变量测试先例 | `registry.rs:168,225,241-243,253-266` | AH-6：分派臂一致性测试照此风格 |
| 注册表 `files`/`files_min_bytes` 等长 zip | `registry.rs:20-25` | AH-5：`files_sha256` 平行数组同构 |
| loopback 读错误恢复链 | `wasapi_win.rs:476-491` | AH-7b：mic 端镜像同款 |
| 探针 env 双门控先例 | `whisper.rs:259,283`（`LT_WHISPER_MODEL`） | AH-9c：nano/qwen3 探针参数化照此 |

---

## 2. 设计决策（DEC）

### DEC-1 恢复语义统一：非 Worker 类错误一律进 `recover()`（D-26）

现状把 `AsrClientError` 分成"等待期可见的 Exited/Timeout → recover"与"预检/写期的 Status/Io → 按用法错误上抛"两类。但 client 层无法区分"真用法错误"与"死亡的新观察点"：预检 `Status("worker 未就绪: Exited")`（`client.rs:260-265` + `status()` 的 `try_wait` 刷新）就是死亡；写失败 Io 基本就是管道断（进程死）；响应 id 错位是流污染——流已脏，串行请求-响应协议下无可重同步手段，kill+重启是唯一正解。**规则收敛为一句话：`Worker{recoverable}` 走三振，其余一切错误 → `recover()`。** 与原版的差异（原版 `except Exception` 仅上抛）登记 D-26。

被否方案：manager 侧逐错误类查 `client.status()` 再决定——client 层预检已经把死亡折成 Status，manager 再反查是二次补丁；不如在错误产生点（client）就归类为 `Exited`，manager 侧 catch-all 兜底。

### DEC-2 待命态 = 可被 `ReplaceEngine` 唤醒的状态，不做周期重探

下载成功链**必然**发 `SwitchEngine`（`shell.rs:245-260`，`started` 单次闸保证不会重启整条管道），命令驱动完备；周期重探（每 N 秒扫缓存）引入无谓磁盘轮询与第二条唤醒路径。待命循环改为：排段的同时消费 `tl_switch`，收到 `ReplaceEngine` 就用**挂起参数**重走 `build_worker_config`，装配成功即带着值跳出待命、落入与首启相同的"manager 创建 + ensure_started"路径。翻译器四臂（ReplaceRig/TargetLanguage/Timeout/TestTranslator）提取共享 helper，两处循环不重抄。

### DEC-3 interim 竞态用"代际号"修，不回全程持锁

全程持锁 = 识别期间（qwen3 RTF≈0.3 时 10s 缓冲约 3s）采集线程阻塞在 chunk 队列，实时场景不可接受；原版也未持锁（§1.3）。代际号方案：`VadProcessor.generation` 在 `reset()` 与 `split_at_best_pause()`（缓冲头部的两个变更漏斗）各 +1，`peek_buffer` 返回代际，trim 前校验同代——代际不符即放弃本次 trim（丢的是"重复识别一次"的效率，不是音频正确性）。**D-27**。

### DEC-4 sha256 只校验"新下载完成"的文件，存量假阳性靠删除重下恢复

skip 判定前做全文件哈希 = 每次 StartDownload 幂等重入都读 941MB（3–5s 起），不可接受；且探测（`cache.rs` manifest）与下载 skip 共用下限语义，单侧加哈希会撕裂同源性。收口组合：① `total=None` 拒绝收尾 + 下载客户端 `.no_gzip()`（堵住"长度校验被绕过"的主通道）；② finalize 前 sha256 校验（哈希已在注册表，新下载必对）；③ 存量假阳性死局的恢复路径 = 数据页"删除该缓存"（AH-6 修复 H9 后 qwen3 可见可删）+ 加载失败错误文案引导（AH-7e）。审计判定的死局需要"total=None + 截断恰落在 [下限, 实际) 区间"两个低概率条件叠加，属可接受残余。

### DEC-5 qwen3 长段防护：VAD 应用点钳制 + S0 校准，不动 UI 滑杆

Qwen2/Qwen3-Audio 族音频编码按 30s 窗设计，`MAX_TOTAL_LEN=512` 为 audio tokens 与输出共享预算，但**token/秒速率未实测**（docs 既有观察点）。方案：常量 `QWEN3_MAX_SEGMENT_SECS: f64 = 15.0`（保守取默认 8s 与滑杆上限 30s 的几何中间偏保守值），在 VAD 设置的三个应用点钳制生效值（UI 保存原值），**并安排 S0 校准任务实测 30s 段行为后再定是否放宽**。D-28。

### DEC-6 测试栈：fake worker 复用 `worker::run`，不造新框架

`worker.rs:13` 的 `EngineFactory<E> = fn(&WorkerConfig) -> anyhow::Result<E>` 本就是为注入设计的（生产四臂在 `main.rs:108-176`）。fake 只需：`EchoEngine` 工厂函数读 `options.fake` 注入项，主循环删除（`fake_asr_worker.rs:58-115` 整段），"装载失败→error(recoverable=false) 帧"路径由工厂返回 Err 天然覆盖。顺带消灭 fake 与真实主循环的行为漂移（如 `fake_asr_worker.rs:107` 的 `set_language(...).unwrap()` panic vs 真实 `worker.rs:75-84` 的 error 帧继续运行）。

### DEC-7 明确不施工项（记录否决理由）

| 项 | 否决理由 |
|---|---|
| 帧协议加魔数/重同步（stdout 污染纵深） | 当前版本组合实测零污染（GUI 冒烟 worker ready 零告警）；whisper 打印已全关（`whisper.rs:157-161`）；协议重构侵入大。列 §7 观察项 |
| 退出最坏 60s 阻塞（transcribe 超时窗口内 join） | 有界 + Job Object 兜底 + stop 已有 500ms 节拍；stop 时提前 terminate 需跨层传 flag，收益低 |
| `MAX_FRAME_BYTES` 256MB 分档预读 | 依赖帧魔数方案，同上缓行 |
| skip 判定前全量哈希 | 见 DEC-4 |

---

## 3. 施工包

> 每包独立可施工、独立验收、测试全绿即可提交（约定式中文 commit）。标注"先行"的子项是其它包的前置。预估总量 ≈6–7 人日。

### AH-1 待命态可唤醒（P0，0.5d）

**改法**（全部在 `crates/lt-app/src/pipeline.rs`）：

1. 提取共享路由 helper（消灭两处循环的翻译器四臂重复，`:825-851` 与 `:901-943`）：

```rust
/// 处理翻译器域命令；ReplaceEngine 原样返回给调用方（待命态与主循环的
/// 引擎处理不同：前者只装配，后者 ensure_started+事件+回滚）
fn route_translator_switch(
    sw: TlSwitch, tl: &mut Option<Arc<TlRig>>, target_language: &mut String,
    proxy: &EventLoopProxy<UiMsg>, settings: &lt_proto::Settings,
) -> Option<TlSwitch>
```

2. 待命分支（`:789-798`）重写为可唤醒循环：

```
let mut worker = build_worker_config(...);          // 首选启动装配
if worker.is_none() {
    send AsrUnavailable + warn（现状保留）
    'standby: while !stop {
        segment_queue.pop_timeout(1s);              // 吞段照旧（None 与 Some 都继续）
        while let Ok(sw) = tl_switch.try_recv() {
            let Some(engine_sw) = route_translator_switch(sw, …) else { continue };
            let TlSwitch::ReplaceEngine { engine, funasr_model, whisper_model_size, language } = engine_sw else { continue };
            // 用挂起参数重装配（pad/language 用 AH-3 的运行时镜像）
            match build_worker_config(&models_dir, &engine, &funasr_model,
                                      sensevoice_pad镜像, &language, &whisper_model_size, whisper_pad镜像) {
                Some(cd) => { worker = Some(cd); break 'standby; }
                None => warn!("切换目标仍未缓存: {engine}/{model_key}，继续待命"),
            }
        }
    }
}
let (config, display) = worker.expect("待命循环仅在装配成功后退出");
```

3. `models_dir` 不可用路径（`:751-758`）维持 return——目录错误属环境故障，不在待命语义内。

**验收**：GUI 冒烟脚本 A——把当前引擎模型目录改名模拟未缓存 → 启动（悬浮窗 unavailable）→ 识别页下载 → 完成后**不重启**，悬浮窗在装载对话框后变为 `{engine} [cpu]`，说话出字幕。
**测试**：`route_translator_switch` 纯函数单测（四臂各自生效 + ReplaceEngine 透传）；`build_worker_config` 未缓存→已缓存转换已有 `qwen3_dispatch_and_uncached`（:1299）覆盖模式可仿。

### AH-2 恢复语义统一（P1-1/H2+H3+H6，1d；依赖 AH-9a 先行）

**client.rs**：

1. `request()` 预检（`:260-265`）：`status()` 刷新后若为 `Exited | Failed` → 返回 `AsrClientError::Exited(format!("预检发现 worker 非 Ready: {st:?}"))`；仅 `Starting/Loading/Busy` 等真异常时序才保留 `Status`。
2. `finish_stop()`（`:241-243`）补 kill 兜底，兑现 `:218` 文档承诺：

```rust
fn finish_stop(&mut self) {
    let _ = self.child.kill();   // 已退出时为无害 no-op
    let _ = self.child.wait();
    self.status = Status::Stopped;
}
```

**manager.rs**：

3. `transcribe` 错误臂（`:249-256`）收敛：

```rust
Err(AsrClientError::Worker { message, recoverable }) => { /* 三振逻辑原样 */ }
Err(e) => Err(self.recover(&format!("{e}"))),   // Exited/Timeout/Status/Io 全部恢复（DEC-1，D-26）
```

4. `simple_request`（`:343-349`）同样把 `Status/Io` 汇入 recover。
5. `transcribe` 早退分支（`:223-225`）拆分：

```rust
if self.unavailable { return Err(Unavailable(...)); }
if self.client.is_none() {
    // recover/RSS 回收 spawn 失败留下的空窗：有限重建（H3）
    let Some(config) = self.config.clone() else { return Err(Unavailable("无配置可重建".into())); };
    tracing::warn!("ASR worker 缺位，尝试重建");
    return match self.spawn_ready(&config) {
        Ok(()) => Err(Restarted("worker 已重建，本段丢弃")),
        Err(e) => { self.mark_unavailable(&format!("重建失败: {e}")); Err(Unavailable(...)) },
    };
}
```

有界性：失败即 `mark_unavailable`（unavailable 置位后走第一分支），不会每段重试；复活路径 = 换引擎或 AH-1 的重装配，既有 `manager_flow` "不可用复活"用例已覆盖。

**新测试**（`manager_flow.rs` / `client_protocol.rs`，注入项见 AH-9a）：
- `crash_between_requests_restarts_on_next_transcribe`：`exit_after_first_transcribe` → 第 1 次成功、第 2 次返 `Restarted`、第 3 次成功（钉死 H2 回归）；
- `rebuild_after_spawn_failure_marks_unavailable`：spawner 第二次起失败 → transcribe → `Unavailable`；换配置 `ensure_started` 复活；
- `shutdown_kills_hung_worker`：`hang_ms=10s` + `set_request_timeout` 收紧 → `shutdown()` 在 ack 窗口（5s）+ε 内返回且进程已死；
- `load_failure_frame_is_fatal`：`fail_load` → `ensure_started` 返 Failed 且 worker 进程正常退出（覆盖 `wait_ready` 的 `!resp.ok` 路径，H17 附带）；
- `set_language_recoverable_fail_still_commits`：`fail_set_language` → pending 被提交且后续 transcribe 正常（qwen3 非 auto 语言的日常路径，`manager.rs:285-287` 现无测试）。

### AH-3 ASR 线程运行时设置贯通（P1-2/H4，0.5d）

**改法**：

1. `TlSwitch`（`pipeline.rs:295-313`）新增两变体：

```rust
/// 识别语言运行时镜像同步（段过滤/commit_text 用；pending 机制照旧驱动 worker）
AsrLanguage(String),
/// padding 运行时镜像同步（family = "funasr" | "whisper"）
Pad { family: String, secs: f32 },
```

2. `run_asr_thread`（`:740-745`）增三个本地镜像并全量替换读点：

```rust
let mut asr_language = settings.asr_language.clone();
let mut sensevoice_pad = settings.sensevoice_pad_seconds;
let mut whisper_pad = settings.whisper_pad_seconds;
```

读点替换：`build_worker_config` 两处（`:784-787` 启动、`:858-862` 切换）、`reject_segment`（`:990`）、`run_interim_pass`/`commit_text`/`commit_interim_final` 的参数（`settings.asr_language` 唯一用途即语言过滤——把这三个函数的 `settings: &Settings` 参数收窄为 `asr_language: &str`，`commit_text:1199` 同步）。
3. `route_translator_switch`（AH-1 产物）增两臂更新镜像；`Pad { family, secs }` 按 family 更新对应值。
4. `Pipeline` 增 `sync_asr_language(&self, lang)` / `sync_padding(&self, family, secs)`（发送 TlSwitch）；`shell.rs` 接线：`Cmd::SetAsrLanguage`（`:86-93`）在 `set_pending_language` 旁调 `sync_asr_language`；`Cmd::SetPadding`（`:95-101`）调 `sync_padding(&engine, secs)`。
5. 顺手（同触面）：`ReplaceEngine` 成功分支（`:874-883`）补 `interim_state.reset()` + `interim.last_interim_samples/last_check_ms` 清零（与 `:1030-1032` VadFlush 尾部等值）——审计 P2"切换后首个收尾段绕过语言过滤"。

**验收**：GUI 冒烟脚本 B——启动 `asr_language=zh` → 运行中改 `en` → 说英语 → **字幕正常出现**（修复前：REJECT_LANGUAGE 全灭）；反向 auto→zh 说英语 → 被语言过滤（预期行为不变）。
**测试**：镜像路由单测；既有 `reject_segment` 十用例（`:1364-1410`）保持不动。

### AH-4 interim 裁剪代际校验（P1-3/H5，D-27，0.5d）

**改法**（`crates/lt-pipeline/src/vad.rs` + `pipeline.rs`）：

1. `VadProcessor` 增字段 `generation: u64`（初始 0）；`reset()`（`:577-584`）与 `split_at_best_pause()`（`:507-543`，缓冲头部变更但不走 reset 的唯一路径）各自 `generation += 1`。
2. `peek_buffer`（`:587-594`）签名改为返回 `Option<(Vec<f32>, f64, u64)>`（携带代际；调用方仅 `pipeline.rs:1068` 一处）。
3. 新增：

```rust
/// 增量 ASR 消费裁剪（D-27）：peek 之后若 VAD 已收段/切分（代际推进），
/// 本次 trim 依据的音频边界已失效——放弃裁剪，防误裁新段头部
pub fn trim_front_checked(&mut self, n_samples: usize, generation: u64) -> bool {
    if self.generation != generation { return false; }
    self.trim_front(n_samples);
    true
}
```

4. `run_interim_pass`：`:1068` 接新签名取 `gen`；`:1129-1131` 改 `trim_front_checked(trim, gen)`，代际不符时 `debug!` 一条（"interim 期间 VAD 已收段，跳过裁剪"）。

**测试**（vad.rs 单测）：同代 peek→trim 成功；peek→`flush()`/`reset()`→trim_checked 返 false 且缓冲未被裁；`split_at_best_pause` 后同样拒绝。

### AH-6 分派臂收敛（P2/H9+H10，0.5d）

**立即修**：

1. `data.rs:41-82` 扫描补 qwen3：`registry::qwen3_entry()` 的 hf repo → `hf_cache_root` 下 `models--{org}--{name}` 条目；whisper 行（`:70`）的硬编码 `"models--ggerganov--whisper.cpp"` 改为从 `registry::whisper_repo` 派生。**目录名拼装收敛为单一 helper**：`lt_models::cache` 新增 `pub fn hf_repo_dir(models_dir, repo) -> PathBuf`（`cache.rs:16` 的 `format!("models--{org}--{name}")` 提升而来），data.rs 与 cache.rs 内部共用——顺带消灭 data.rs:58 的第二份格式化。
2. `vad.rs:444-457` `hf_only_key`（硬编码 nano/qwen3 两键）改为查注册表：当前引擎/模型键 → `funasr_entry/whisper_entry_for/qwen3_entry` → `entry.ms.is_none()` 即显示 HF-only 提示——whisper 档自然纳入（hub=ms 实际走 hf-mirror 的诚实提示，行为同构性补齐）；i18n 增通用键 `model_hf_only_hint`（zh/en 双份），原 `model_nano_hf_only`/`model_qwen3_hf_only` 保留或收编为别名。

**防再漂移（测试强制）**：

3. lt-ui data.rs 新增测试：为 nano/qwen3/whisper 造 sparse 缓存目录 → `scan_cache_entries` 三者全部可见、体积非零——**该测试在现状必红（qwen3 缺失），即 H9 的回归钉**。
4. lt-app 既有分派单测（`pipeline.rs:1269-1315`）扩展：对 `lt_proto::ASR_ENGINES` 全集断言 `build_worker_config`/`engine_model_key`/`cache::missing_models`/UI `model_cache_status` 各臂无 `_ =>` 盲兜（qwen3 键拼错静默变"无需下载"的 `cache.rs:179` 缺陷即此）。`is_asr_cached`（`cache.rs:146`）无生产调用方 → 删除或 `#[cfg(test)]` 收编。

### AH-7 可观测性五项（P2/H11~H14，1d，五项互相独立可拆）

a. **丢段计数与告警**：`BoundedDropQueue`（`audio/mod.rs:153`）增 `name: &'static str` 与 `dropped: AtomicU64`；`push`/`push_front`（`:169-176,199-206`）丢弃时计数，`dropped==1 || dropped%200==0` 时 `warn!("音频队列[{name}]已满，累计丢弃 {dropped} 段")`（确定性节奏，免时钟）。构造点 `pipeline.rs:344`（"chunk"）与 `:370`（"segment"）。引擎装载期（wait_ready ≤180s）段积压丢弃由此可见。
b. **mic 读错误恢复**：`drain_mic`（`wasapi_win.rs:337-353`）返 `bool`（读错误即 false）；`read_loop` 两处调用点（`:495-497`、`:506-508`）遇 false：`warn!("麦克风读取失败，尝试重开")` → `close_mic` + 0.5s sleep + `open_mic`（镜像 loopback `:482-491` 语义；不清 chunk 队列——loopback 未受影响）。
c. **AsrUnavailable 边沿触发**：`run_asr_thread` 增 `unavailable_notified: bool`；`:1022-1027` 仅 false→true 沿发事件；transcribe `Ok` 或引擎切换/启动成功（AsrDevice 发出时）复位 false。
d. **worker 进程日志**：`main.rs::asr_worker_entry`（`:108`）开头初始化 stderr 订阅者（`tracing_subscriber::fmt().with_ansi(false).with_writer(std::io::stderr).with_max_level(INFO)`；tracing-subscriber 已在 lt-app 依赖）——引擎 `info!`（加载耗时等）经父进程 `drain_stderr` → `debug!(target:"asr_worker")` → 日志窗（debug 档可见），`main.rs:23-25` 的日志黑洞消除。
e. **sherpa 加载失败根因增强**：`nano.rs:119-121`/`qwen3.rs:97-99`/`sensevoice.rs:129-130` 的 `.ok_or_else` 错误文案追加：模型目录、清单文件实测大小列表（load 前置检查 `nano.rs:47-63` 已逐文件点名，复用其数据）+ 指引"文件在位但 ORT 加载失败；详见日志窗 debug（asr_worker stderr）；若怀疑缓存损坏，可在 设置→数据与存储 删除该模型后重新下载"（衔接 DEC-4③ 恢复路径）。

### AH-5 下载完整性收口（P2/H7+H8，1d；哈希登记需实测）

1. **哈希登记**（`registry.rs`）：`ModelEntry` 增 `files_sha256: &'static [&'static str]`（与 files 等长 zip，`""`= 未登记不校验）；`manifest_min_bytes_parallel_to_files`（`:253-266`）扩为三数组等长断言。**实施第一步**：对本地真实缓存（sensevoice/nano/qwen3 + 本地已有的 whisper 档）`Get-FileHash` 计算完整 64 位哈希；本地缺档的 whisper 档经 HF API LFS metadata 补齐——注意 docs §2.2/§4.2 只记了前 16 位（`asr-engine-expansion.md:297`），**不能直接抄**。目标：20 个清单文件（2+6+6+6）全登记。
2. **下载器校验**：`MissingModel`（`cache.rs:151-165`）与 `download_files`（`download/mod.rs:264-271`）的 `files: &[(&str, u64)]` 升格为 `&[FileSpec]`（`path, min_bytes, sha256`）；`try_download` 收尾段（`:440-452`）在长度校验后、`finalize_incomplete` 前：sha256 已登记则流式校验（复用 `.incomplete` 文件读），不匹配 → 删 `.incomplete` + `FailKind::Length("sha256 不匹配")`。skip 幂等路径不读文件（DEC-4）。
3. **total=None 拒绝收尾**：`:441` 的 `if let Some(total)` 改为 `let Some(total) = total else { return Err(DlError::new(FailKind::Length, "服务端未提供 Content-Length，拒绝收尾")) }`；`http_client`（`:246-257`）加 `.no_gzip()`——模型二进制无需自动解压，且 gzip 解码是 total=None/长度语义失配的主要来源（workspace `Cargo.toml:66` 开了 gzip feature）。
4. **磁盘空间预检**（P3 顺手）：`backend.rs::run_download`（`:149`）targets 确定后：`Σ estimated_bytes + 256MB` 余量 vs `GetDiskFreeSpaceExW`（lt-app 已有 windows 依赖，加 `Win32_Storage_FileSystem` feature）；不足 → `fail("磁盘剩余空间不足：本模型约需 X，可用 Y")`。非 Windows/探测失败 → 跳过（不阻断）。

**测试**：mock 服务器无 Content-Length → Length 快速失败且 `.incomplete` 保留可续传；sha256 不匹配 → Length + `.incomplete` 清理；哈希匹配正常 finalize（download_integration.rs 现有 9 用例风格延伸）。

### AH-8 qwen3 长段钳制 + nano 语言切换超时（P2/H15+H16，D-28，0.5d）

1. `pipeline.rs` 增常量与 helper：

```rust
/// qwen3 生效段长上限（D-28）：MAX_TOTAL_LEN=512 为 audio+输出共享预算，
/// 30s 段有静默截尾风险；S0 校准（§7-T1）实测后可调
const QWEN3_MAX_SEGMENT_SECS: f64 = 15.0;
fn clamp_vad_for_engine(engine: &str, mut s: VadSettings) -> VadSettings {
    if engine == "qwen3" && s.max_speech_duration > QWEN3_MAX_SEGMENT_SECS {
        tracing::info!("qwen3: max_speech_duration 生效值钳制为 {QWEN3_MAX_SEGMENT_SECS}s（设置 {}s）", s.max_speech_duration);
        s.max_speech_duration = QWEN3_MAX_SEGMENT_SECS;
    }
    s
}
```

生效点三处：`Pipeline::start`（`:355-368`，按 `settings.asr_engine`）、`shell.rs::ApplySettings`（`:195-203`，settings 镜像有引擎值）、`ReplaceEngine` 成功分支（`:874-883`，按新 engine 重算并写入 `vad_update` 槽——`AsrThreadCtx`（`:727`）增 `vad_update: Arc<Mutex<Option<VadSettings>>>` 字段，Pipeline::start 装配时传入）。UI 滑杆与 settings 保存值不动（D-28：生效值引擎相关）。
2. `client.rs:200-216` `set_language`/`set_input_padding` 超时 `Duration::from_secs(10).min(self.request_timeout)` → `self.request_timeout`（60s）+注释（nano 重建识别器本机 4.15s，慢机余量不足 10s；代价=真挂死时 apply_pending 等待与 transcribe 同级，语义一致）。
3. **S0 校准任务**（§7-T1）实测后回填常量依据。

### AH-9 测试栈补强（P2/H17+H18，1d；**AH-9a 为 AH-2 前置**）

1. **fake 复用真实 run**（AH-9a，先行）：重写 `fake_asr_worker.rs`——删除主循环（`:58-115`），改为：

```rust
fn echo_factory(cfg: &WorkerConfig) -> anyhow::Result<EchoEngine> {
    if opt_bool(&cfg.options, "fail_load") { anyhow::bail!("注入的装载失败"); }
    Ok(EchoEngine::from_options(&cfg.options))   // 非捕获闭包/函数指针，EngineFactory 直接可用
}
fn main() {
    ... 解析 config ...
    if opt_bool(&options, "crash_on_ready") { std::process::exit(3); }   // ready 前消失照旧
    let _ = lt_asr::worker::run(std::io::stdin().lock(), std::io::stdout().lock(), config, echo_factory);
}
```

新增注入项：`fail_load`（覆盖"装载失败→error(recoverable=false) 帧"真实路径）、`exit_after_first_transcribe`（响应后 `process::exit`，H2 用例）、`fail_set_language`（返 `EngineError::Runtime{recoverable:true}`）。fake 的 `SetLanguage.unwrap()` panic 差异随之消灭。
2. **探针参数化**（H18）：`nano.rs:209`/`qwen3.rs:197` 硬编码路径改 env 双门控（对齐 `whisper.rs:259` `LT_WHISPER_MODEL` 先例，如 `LT_NANO_MODEL_DIR`）；wav 缺失分支（`nano.rs:227-229`/`qwen3.rs:250-253`）改计数，探针末尾断言"实际转写 wav 数 ≥1"，杜绝全缺失静默绿。
3. AH-2/AH-5/AH-6/AH-4 各自新增用例见各包。**收工门**：`cargo test --workspace` 全绿（预期净增 ≥15 测）+ 新码 clippy 零告警。

### AH-10 文档纠偏与杂项（0.5d，随各包捎带 + 收尾批次）

1. **AGENTS.md**：分层规则改写为"lt-ui 允许只读依赖 lt-models（注册表/缓存探测）与 lt-translate（bench 直调）——2026-09-06 f266a8b 批次的有意决策，见 lt-ui/Cargo.toml 注释；lt-ui 仍不得依赖 lt-app"；构建节测试基线改"349 测（滚动）+5 ignored"并注明随 WP 滚动更新；"lt-proto 已冻结"补注"值域扩展（如 ASR_ENGINES 增项）允许，结构/Cmd/Event 增删需评审"。
2. **assets/SOURCES.md** 三处：onnxruntime.dll 的 sha256"见 git 历史"循环引用 → 实测补齐；icons 来源路径 `D:iancheng\LiveTranslatessets\icons`（`\b`/`\a` 转义吞蚀）→ 修复为字面路径；CJK 字体 br 的 sha256 截断"…"→ 补全。
3. **代码杂项**：`frame.rs:119-131` `FrameWriter` 内包 `BufWriter`（`write_frame` 末尾显式 flush——响应等待前必须落管）；`lt-asr/Cargo.toml:8` 删 `ort` 死依赖（grep 全库零 `use ort`）；`pipeline.rs:544` 删过时 `#[allow(dead_code)]`（`shell.rs:88` 实际调用）；`settings.rs:151` `unwrap_or_default()` 失败分支补 `tracing::warn!`（含 serde 错误详情）；`asr_result.rs:11-13` `language_name`/`words` 字段 doc 补"当前无 UI 消费/恒 None"现状注记。
4. **引擎共享收敛**（P3，随 AH-7e 同批）：`engines/mod.rs:41-57` strip 扫描器对齐原版正则语义 `re.sub(r"<\|[^|]+\|>", "")`——扫描到 `<|` 后，若到下一个 `|>` 之间出现任何 `|` 则整体不匹配原样保留（现状会吞到更远的 `|>` 误删）；`sensevoice.rs:73-86` 的手抄副本改调共享实现；`normalize_language` 三份（`whisper.rs:115-120`/`nano.rs:90-95`/`sensevoice.rs:109-114`）提升 `engines/mod.rs`；`sensevoice.rs` 迁入 `engines/`（lib.rs 再导出保持 API 不变）。

---

## 4. 施工顺序与依赖

```
AH-9a（fake 复用 run）──► AH-2（恢复统一 + 回归用例）
AH-1（P0 待命唤醒）                        ← 优先级最高，可与 AH-9a 并行
AH-3（设置贯通）──► AH-4（interim 代际）    ← 同触 pipeline.rs，建议连续施工
AH-6（分派臂收敛：data.rs 立即修 + 测试钉）
AH-7（可观测性五项，可拆散捎带）
AH-5（下载收口：哈希登记需实机/网络实测）
AH-8（qwen3 钳制 + 超时）
AH-10（文档批次收尾）
```

提交规范：每包独立 `fix(scope)/feat(scope): 中文主题`（pathspec 显式列出）；docs 本方案已先行独立提交；各包完成时回填本文勾销标记。

## 5. 偏差登记（决策史，AGENTS.md §1.5 顺延）

| 编号 | 内容 | 理由 |
|---|---|---|
| **D-26** | client 层非 Worker 类错误（Exited/Timeout/Status/Io）一律走 `recover()` 自动重启；原版 `except Exception` 仅按失败上抛 | 原版恢复只覆盖"等待期死亡"；预检/写期的死亡观察点不接恢复 = 永久僵尸态（H2/H3）。流污染（id 错位）在串行协议下唯一正解即重建 |
| **D-27** | interim 裁剪引入代际校验：VAD 收段/切分后放弃本次 trim | 原版同拓扑存在竞态（main.py:1414-1416 peek 持锁、识别不持锁）；Rust 版以代际号消除"误裁新段头部"的正确性风险，不回全程持锁（识别期阻塞采集不可接受） |
| **D-28** | qwen3 引擎下 `max_speech_duration` 生效值钳制 ≤15s（UI/持久化值不动） | `MAX_TOTAL_LEN=512` 为 audio+输出共享预算，30s 段有静默截尾风险；S0 校准（T1）后再定放宽 |

## 6. 全局验收标准

1. `cargo test --workspace` 全绿（净增 ≥15 测；AH-6 数据页扫描测试在施工前分支上必须可复现红色）。
2. `cargo clippy --workspace --all-targets` 新码零告警。
3. GUI 冒烟三脚本实机过：
   - **A（H1）**：模拟未缓存启动 → 面板下载 → 完成后不重启即 ready 并出字幕；
   - **B（H4）**：运行中 zh→en 切换源语言，字幕不灭；
   - **C（H2）**：运行中任务管理器杀 asr-worker 进程 → ≤60s 内日志见"自动重启"、状态行恢复、字幕继续（间隙死亡路径：杀进程须在两段之间的空闲窗口）。
4. T1（AH-5/AH-8 附带实测）：qwen3 长样例（≥20s 连续语料）转写，记录 token 预算表现 → 回填 `QWEN3_MAX_SEGMENT_SECS` 依据至本文与 asr-engine-expansion.md。
5. AGENTS.md 待办勾销回写（随最后一批提交）。

## 7. 观察项（不施工，登记触发条件）

| 项 | 触发条件 | 届时动作 |
|---|---|---|
| 帧协议魔数/重同步（stdout 污染纵深，审计 P2） | 升级 sherpa-onnx/ORT 后出现"worker 反复重启 3 次烧完配额→unavailable"，且 stderr 无 panic 痕迹 | 帧头加 `[magic u16][version u8][len u32]` + reader 有限重同步；fake 加 `print_to_stdout` 注入 |
| 退出最坏 60s 阻塞 | 用户报告退出偶发卡死（worker 挂死叠加退出） | stop 路径 join 带 2–3s 超时后 detach（Job Object 已兜底孤儿） |
| 帧分配上限 256MB 单档 | 同第一项（协议重构时一并做 JSON/音频分档预读） | — |
| `get_result` JSON 形状漂移（sherpa binding 内部 `serde_json` 失败静默返 None） | 升级 sherpa-onnx 后三引擎"识别恒空" | 把"结果 JSON 字段形状"列入 sherpa 升级回归项 |
| EventLoopProxy 无界事件队列 UI 高负载积压 | WP-9 8h 长跑出现 UI 滞后 | 届时按长跑数据决策 |
