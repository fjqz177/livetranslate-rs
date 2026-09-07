# 模型下载链路审计与改造方案（docs/download-overhaul.md）

> 状态：**阶段二活跃文档**（2026-09-08 审计定稿）。审计范围：`lt-models`（download/cache/registry）+ `lt-app`（backend/shell 编排）+ `lt-ui`（四态卡片/进度渲染）。
> 结论：骨架合理（断点续传/双 hub 同约定布局/失败前缀分类/四态卡片），但存在 **2 高危 + 1 功能失效 + 4 中 + 6 低** 共 13 项问题。
> 施工包 DL-1~DL-6：P0 = DL-1/2/3（约 1.5 天），P1 = DL-4/5，P2 = DL-6。**DL-1 是 asr-engine-expansion WP-A（nano 实装）的前置**。

## 0. TL;DR

- **F1（高危）**：缓存探测把 `.incomplete` 半截文件计入体积 → 下载失败后点「重试」直接假报成功、UI 显示「✓ 已缓存」但引擎永远起不来，用户无 UI 手段恢复。
- **F2（高危）**：funasr-nano 注册表 `estimated_bytes` 是占位值（1.1GB），MS 侧完整性阈值 = est/2 = 550MB 远超实际体积 → nano 经 MS 完整下载后 UI 永报「未缓存」，反复假下载不收敛。
- **F3（功能失效）**：UI 进度条解析函数取 token 拿到的是单位词（`"MB"`）而非数值，**所有真实进度行解析失败** → 进度条自上线起从未走过（恒 0% + 纯日志模式），且零单测覆盖。
- 其余：下载器跳过已存在文件不验尺寸、404 也全量退避白等 21s、下载占死 backend 线程 + 成功时旧镜像落盘回踩设置、无取消能力、多文件进度回跳、每帧扫盘等。
- 改造主线：**探测 manifest 化**（一招灭 F1/F2/F13）→ **下载器完整性 + 快速失败** → **进度协议精确化（不扩 lt-proto）** → **会话化下载线程 + 取消 + 落盘归一** → **hub 缺失回落机制（落地 D-21 下载层半）**。

## 1. 审计结论总表

| # | 严重度 | 一句话 | 证据位置 | 归属施工包 |
|---|---|---|---|---|
| F1 | 高危 | 探测把 `.incomplete` 计入体积，失败重试变假成功 + 死局 | `lt-models/src/cache.rs:37-85,141-156` | DL-1 |
| F2 | 高危 | nano 占位 estimated + 双 hub 阈值不一致 → 完整下载判「未缓存」 | `lt-models/src/registry.rs:36-44`、`cache.rs:134-138` | DL-1 |
| F3 | 功能失效 | 进度条解析 dead code，进度条从未工作 | `lt-ui/src/app.rs:1445-1466,863-875` | DL-3 |
| F4 | 中 | 跳过已存在文件不验尺寸 + rename 前 no fsync → 半截终版文件永久跳过 | `lt-models/src/download/mod.rs:171-176,294-305` | DL-2 |
| F5 | 中 | 404/401 等不可重试错误也走 1/4/16s 全退避，白等 ~21s | `lt-models/src/download/mod.rs:53-57,186-210` | DL-2 |
| F6 | 中 | 下载占死 backend 线程；成功时旧镜像落盘 + 事件回踩 UI 设置 | `lt-app/src/backend.rs:23-69,143-165,184-211`、`lt-ui/src/app.rs:786-788` | DL-4 |
| F7 | 中 | 下载无取消能力，唯一中止方式 = 退出应用 | `lt-ui/src/state.rs:962-976`、`vad.rs:677-764` | DL-4 |
| F8 | 低 | hub 缺失无回落（hf-mirror 键按 D-21 明确不做，回落机制缺位） | `lt-app/src/backend.rs:122-137` | DL-5 |
| F9 | 低 | 泵收尾竞态丢最后几条下载日志 | `lt-app/src/backend.rs:143-161` | DL-6 |
| F10 | 低 | 多文件下载进度回跳 + 人工单位浮点损耗 | `lt-ui/src/app.rs:863-875` | DL-3 |
| F11 | 低 | MS URL `FilePath` 未做 URL 编码 | `lt-models/src/download/ms.rs:4-6` | DL-6 |
| F12 | 低 | 识别页每帧递归扫盘探测缓存 | `lt-ui/src/windows/panel/vad.rs:668-685` | DL-6 |
| F13 | 低 | `local_model_dir` MS 分支只查 exists，与探测语义不一致 | `lt-models/src/cache.rs:265-287` | DL-1 |

## 2. 证据详录

### F1 探测计入 `.incomplete`（高危，默认模型 sensevoice 直接受害）

注释与实现矛盾——`cache.rs:37-38` 声明「忽略孤儿 .incomplete blob」，但两处探测共用的 `walk_files`（`cache.rs:71-85`）无任何后缀过滤：

```rust
// cache.rs:37-38（声明）
/// HF repo 是否"存在且下载完成"（E-10：统计可解析文件总字节；
/// 忽略孤儿 .incomplete blob；损坏符号链接视为不完整）
// cache.rs:50-60（实现，walk_files 产出所有文件含 *.incomplete，逐个累加）
for f in files {
    match f.metadata() {
        Ok(m) => total += m.len(),   // .incomplete 的字节数全部计入
```

MS 侧同病：`dir_size_gte`（`cache.rs:141-156`）同样不过滤。whisper 因按精确文件名探测（`is_whisper_cached` 查 `ggml-*.bin` 本体）幸免。

**触发链**（sensevoice，HF 侧阈值 50MB，`cache.rs:11`）：
1. 下载 `model.int8.onnx`（239MB）中途断网，`model.int8.onnx.incomplete` 已写 150MB，重试耗尽 → 卡片显示失败（正确）；
2. 点「重试」→ `missing_models` → `is_funasr_cached`：150MB ≥ 50MB → **判定已缓存** → `backend.rs:108-112` `targets` 为空直接 `succeed()` → UI 弹「下载完成」；
3. 卡片回「✓ 已缓存 250MB」，shell 随即 SwitchEngine → 引擎加载找不到真正的 `model.int8.onnx` → ASR unavailable；
4. 此后 UI 永不出现下载按钮（探测恒「已缓存」），用户只能手删 `.incomplete`。下载中途重启应用同样落阱。

现有测试 `hf_broken_snapshot_falls_through`（cache.rs:321-330）恰好没覆盖「仅含 .incomplete 的目录」形态，313 测全绿未拦截。

### F2 nano 阈值注定误判（高危，WP-A 实装即踩）

`registry.rs:36-44`：`FUNASR_NANO.estimated_bytes = 1_100_000_000`，注释自认「M5 实装；注册表占位」。MS 侧阈值 `ms_threshold = max(est/2, 50MB)`（`cache.rs:134-138`）= **550MB**，而 nano 实际 int8 包远小于此；HF 侧又是另一个常数 50MB（`cache.rs:129`）。后果：nano 经 MS **完整**下载后 `ms_ok` 永远 false、若经 HF 下载则 MS 侧判定照样失效 → UI 永报「未缓存」→ 每次点下载 `download_files` skip-all 后报「成功」→ 探测不变 → 无限假循环。nano 现已可在 UI 选择（`funasr_entry("funasr-nano-2512")` 返回条目，mlt 才是灰显）。

### F3 进度条解析是死代码（功能失效，上线即坏）

backend 进度行由 `format_size`（`backend.rs:237-247`）生成，**数值与单位恒以空格分隔**：`"[{repo}] {file} 1.0 KB / 2.0 KB"`。UI 侧 `parse_progress_tokens`（`app.rs:1445-1452`）：

```rust
let done = parse_human_size(line[..idx].split_whitespace().last()?)?;   // 取到 "KB"，不是 "1.0 KB"
let total = parse_human_size(line[idx + marker.len()..].split_whitespace().next()?)?;  // 取到 "2.0"，同样失败
```

逐步代演：`line[..idx] = "[a/b] m.onnx 1.0 KB"` → `split_whitespace().last() = "KB"` → `parse_human_size("KB")`（要求 `"N unit"` 带空格）→ `None` → **整条进度被丢弃**。`"512 B"` 形态同样失败（last 取到 `"B"`）。结论：`update_download_progress`（`app.rs:863-875`）从未生效，`done_bytes/total_bytes` 恒 0 → 进度条恒 0%、字节计数恒隐藏、永远退化为日志模式。该函数**零单测**（全仓 grep 无 `#[test]` 触及），是 313 测盲区。

### F4 跳过不验尺寸 + 无 fsync（中）

`download/mod.rs:171-176`：`if target.is_file() { 跳过 }`——不看大小。叠加 ①`total == None`（无 Content-Length）时不做长度校验（`mod.rs:296-303`）即 rename 成终版；②rename 前只有 `flush()` 无 `sync_all()`（`mod.rs:294-305`），掉电/崩溃可留半截终版文件。此后每次下载都跳过该坏文件并报成功，与 F1 叠加成不可恢复循环。

### F5 不可重试错误全量退避（中）

`download/mod.rs:186-210`：重试循环对**一切**错误等 1s/4s/16s 再试。仓库缺失（404）、私有仓（401）属永久错误，用户要白等 ~21 秒 + 4 次请求才看到「建议切下载源」。

### F6 下载占死 backend 线程 + 旧镜像落盘回踩（中）

- backend 单线程 loop（`backend.rs:23-69`），`StartDownload` 后整个下载期间阻塞在泵循环（`backend.rs:143-161`），`PersistSettings/ApplySettings/SwitchEngine/Stop` 全部排队。
- 成功时 `succeed()`（`backend.rs:184-211`）把**可能过期的镜像**（排队中的设置变更尚未并入）`settings_io::save` 落盘，随后 `UiEvent::DownloadSucceeded{settings}` 在 `app.rs:787` 直接 `self.app_state.settings = *settings` **回踩 UI 状态**——用户下载期间的修改先被回滚、再被排队命令重放；若用户恰在「成功事件已处理、排队 PersistSettings 未消化」窗口退出，shutdown（`shell.rs:57`）会把旧值持久化。
- 现状 `first_launch` 恒为 false（`main.rs:59` 传死值，D-19 向导不接线），向导 13 键默认块分支为死代码，但同样写盘。

### F7 无取消（中）

`DownloadUiState` 仅 Idle/Downloading/Failed 三态（`state.rs:962-976`），Downloading 卡片无取消按钮（`vad.rs:677-720`），协议无 `CancelDownload`。1.1GB 的 nano 弱网下载只能杀进程。

### F8~F13（低，摘要）

- **F8**：`backend.rs:122-137` 按所选 hub 单选仓库，404 即失败；D-21 已裁决「缺失回落另一 hub」，机制未落地（whisper 的 MS 镜像条目属 distribution WD-4，本方案只做机制半）。
- **F9**：泵循环 `worker.is_finished()` 即 break（`backend.rs:157-159`），channel 未排空，尾部日志（如「快照就绪」）可丢。
- **F10**：`total_bytes` 只在为 0 时锁一次（`app.rs:871-873`），sensevoice 两文件时进度从 239/239 跳回 0.x/239；人工单位字符串往返有浮点损耗。
- **F11**：`ms.rs:4-6` `FilePath={path}` 裸拼，无百分号编码（现注册表文件名安全，稳健性欠账）。
- **F12**：`vad.rs:668-685` 每帧调用 `model_cache_status` → `is_funasr_cached` 递归 stat 模型目录，面板开着就是持续 IO。
- **F13**：`local_model_dir`（`cache.rs:265-287`）MS 分支只查 `p.exists()` 无完整性判断，HF 分支取「字典序最后」快照不验内容，与探测语义不一致。

## 3. 改造目标（目标态 + 量化验收）

改造后的下载链路必须同时满足：

1. **可信**：卡片显示「✓ 已缓存」当且仅当引擎能加载成功——探测只认注册表 manifest 终版文件，任何 `.incomplete`、半截文件、空文件都不可能被判「已缓存」；下载器跳过/成功判定与探测使用同一套 manifest 标准（消灭 F1/F2/F4/F13 的判定分裂）。
2. **可见**：进度条真实工作——百分比随下载推进、按当前文件显示「第 k/n 个文件 + 字节进度」，total 未知时明确显示日志模式而非 0% 假条（消灭 F3/F10）。
3. **快败可退**：永久性错误（404/401/403）≤2s 内报分类建议；所选 hub 缺失自动回落另一 hub（消灭 F5/F8）。
4. **可控**：下载可取消（卡片取消按钮），取消后保留续传现场；下载期间 backend 线程照常处理全部命令，成功/失败不回踩用户设置（消灭 F6/F7）。
5. **零回归**：`cargo test --workspace` 全绿（现有 313 测不破），新增单测 ≥ 16（清单见 §6.1）；lt-proto 契约仅**追加** `Cmd::CancelDownload` 与 `UiEvent::DownloadCancelled` 两个成员（既有成员语义零改动，沿 ux-feedback 期 `Cmd::TestTranslator` 扩展先例）。
6. **性能红线**：识别页打开时缓存探测 IO 频率 ≤ 0.5Hz（2s 节流 + 事件失效）；下载期主进程无自旋。

## 4. 设计决策（取舍与被拒方案）

| # | 决策 | 理由 | 被拒方案 |
|---|---|---|---|
| DEC-1 | 探测从「递归体积 ≥ 阈值」改为 **manifest 校验**：注册表新增与 `files` 平行的 `files_min_bytes: &'static [u64]`（每文件下限），`is_funasr_cached` = 解析目录后逐文件「存在 + len ≥ 下限」 | 假阴性（多下一次）安全可自愈，假阳性（判已缓存却加载失败）即死局——阈值启发式天然偏假阳性；同时消灭 est/2 占位与双 hub 阈值分裂 | 保留体积阈值只加 `.incomplete` 过滤：修不了 F2（nano 占位 est）与 F13，阈值常数继续积累 |
| DEC-2 | 进度协议**不扩 lt-proto**：`DownloadProgress(String)` 载荷尾部加机器段（`\t` 分隔：`k n done_bytes total_bytes`，total 未知为 0），人读段保持原样进日志 | lt-proto 冻结纪律 + 精确字节无浮点损耗 + 旧格式行（无 `\t`）优雅忽略保持向后兼容 | 新增结构化 `DownloadProgressV2` 事件：破契约冻结；浮点 human 串继续解析：F3 根因不动 |
| DEC-3 | 取消用 `AtomicBool` 检查点注入 Downloader（每文件边界 + 每重试 sleep + 每读块），取消保留 `.incomplete` 续传现场 | Downloader 保持同步阻塞风格（D 系偏差既定），闭包注入零耦合 | reqwest 退异步：推翻「阻塞式专用线程」既定设计，工程量失控 |
| DEC-4 | 落盘权归一：`settings_io::save` 只剩 `shell.rs` 一条路径（`persist_settings`），backend `succeed()` 不再写盘，成功收尾经既有 SwitchEngine→persist 链 | 单写者消除 F6 竞态；UI 是 settings 事实源，backend 镜像仅用于下载目标重算 | backend 下载前拷贝全量新镜像：治标，镜像同步面继续扩大 |
| DEC-5 | hub 回落做成下载层通用机制（`next_hub()` 纯函数 + 仓库级一次回落），whisper 的 MS 镜像条目留给 distribution WD-4 实测登记 | D-21 的机制半与数据半解耦，本方案不阻塞网络实测 | 本卡连带 whisper MS 仓实测：网络依赖重、卡体积失控 |

## 5. 工作包

### DL-1（P0）探测 manifest 化 —— 灭 F1/F2/F13，WP-A 前置（0.5d）

| 项 | 内容 |
|---|---|
| registry | `ModelEntry` 增 `files_min_bytes: &'static [u64]`（与 `files` 等长 zip）。sensevoice：`[50_000_000, 1_024]`；nano：`[50_000_000, 1_024]`（下限刻意远低于实际值，首日经 HF API 核对实际字节数后校准登记，沿 whisper【M5 首日核实】先例）；whisper 六档：`[estimated_bytes/2]`（已是实测校准，语义不变）；`MissingModel` 同步携带。下限校验测试：`files.len() == files_min_bytes.len()` |
| cache.rs | 新 `fn dir_has_manifest(dir, files, mins) -> bool`（逐文件存在 + len ≥ 下限）。`is_funasr_cached` 重写：MS 侧 `ms_model_path` 命中目录 + manifest → true；否则 HF 侧取**含完整 manifest 的**字典序最后快照 → true。删除 `hf_repo_complete`/`dir_size_gte`/`ms_threshold`/`FUNASR_MIN_BYTES`（连同「忽略 .incomplete」失实注释）。`local_model_dir` 两侧分支统一用 manifest 校验（F13）。`missing_models` 对外签名不变 |
| vad.rs | `model_cache_status` 无接口变化，自动受益 |
| 测试 | ①仅 `.incomplete` 目录 → 未缓存（F1 回归）；②`.incomplete` 150MB + 完整 tokens.txt → 未缓存；③双 hub 各自 manifest 齐全 → 已缓存；④manifest 缺一文件 → 未缓存；⑤`local_model_dir` 跳过含 .incomplete 的较新快照回退旧快照；⑥注册表 min 字段一致性 |

### DL-2（P0）下载器完整性与快速失败 —— 灭 F4/F5（0.5d）

| 项 | 内容 |
|---|---|
| 签名 | `download_files(hub, repo, files: &[(&str, u64)], tx)`——`(文件名, 下限字节)`，0 = 仅要求存在；backend 由 `MissingModel` zip 传入 |
| 跳过校验 | 目标存在且 `len ≥ 下限` → 跳过；存在但不足 → 记日志「尺寸异常（X < Y），重下」删除后重下（F4） |
| 持久性 | rename 前 `f.sync_all()`；rename 前若 target 已存在（双写竞态）best-effort `remove_file` 再 rename（Windows rename 语义） |
| 快速失败 | 内部 `DlError { err, retryable }`：`KIND_NET`、HTTP 5xx/429、`KIND_LENGTH`（续传可救）→ retryable；HTTP 其余 4xx（404/401/403…）→ 立即失败不再退避。分类前缀契约（`[net]/[http]/[http-404]/[disk]/[length]`）不变 |
| 测试 | 重试分类纯函数表测（404/401→false；429/500/net/length→true）；跳过校验（足额跳过/不足重下）；rename 目标已存在路径；`incomplete_path` 既有测试保留 |

### DL-3（P0）进度协议精确化（不扩契约）—— 灭 F3/F10（0.5d）

| 项 | 内容 |
|---|---|
| backend | `format_event` Progress 行变为 `"{人读段}\t{k} {n} {done_bytes} {total_bytes}"`：k/n = 当前文件序号/本模型清单长，`total_bytes` 未知为 `0`。人读段原样（日志可读性不变） |
| lt-ui | `update_download_progress` 重写：先 `split_once('\t')`，人读段进 `push_log`，机器段解析 `"{u64} {u64} {u64} {u64}"` 精确字节。删除 `parse_progress_tokens`/`parse_human_size`（连同 F3 病灶） |
| 状态 | `DownloadUiState::Downloading { file: String, k: u32, n: u32, done_bytes, total_bytes, log }`；卡片显示「{file}（k/n）」+ 进度条按**当前文件**；`total_bytes == 0` 走日志模式（明确文案，非 0% 假条）。文件切换由 backend 数据自然携带，bar 随文件归零且标签可见，消除「回跳」误解 |
| 测试 | 新解析器表测：标准行 / total 未知 / 0 字节边界 / >4GB / 无 `\t` 的旧式与外来行忽略；`Downloading` 状态机推进测试 |

### DL-4（P1）会话化下载线程 + 取消 + 落盘归一 —— 灭 F6/F7（1d）

| 项 | 内容 |
|---|---|
| 契约 | lt-proto **追加** `Cmd::CancelDownload` 与 `UiEvent::DownloadCancelled`（仅追加，既有成员不动；ux-feedback 期 `Cmd::TestTranslator` 先例）。偏差登记见 §9 |
| backend | `StartDownload` 改为 spawn「下载会话线程」（现 run_download 主体整体搬移，泵+worker 归会话线程），backend 主 loop 立即返回继续 recv——下载期间 PersistSettings/SwitchEngine 照常消化，镜像不再过期。会话在途时收到 StartDownload → 回日志「已有下载进行中」忽略。`CancelDownload` → 置会话 `AtomicBool` |
| Downloader | 构造或调用参注入 `cancel: &dyn Fn() -> bool`（DEC-3）：每文件边界 / 每次退避 sleep 后 / 每 64KB 读块检查；命中则留 `.incomplete` 返回 Cancelled 错误 |
| 收尾 | `succeed()` 删除 `settings_io::save`（DEC-4）：运行期成功链 = shell 收 `DownloadSucceeded` → 重发 SwitchEngine → `handle_cmd` 内 `persist_settings`（既有路径）；向导/缺模型路径由 app.rs 收到成功事件后发 `Cmd::PersistSettings` 落盘。`app.rs:787` 不再用事件载荷覆盖 `app_state.settings`（UI 是事实源，载荷保留仅为契约兼容）。`first_launch=true` 死分支维持编译但标注 D-19 不接线 |
| UI | 失败/下载中卡片增「取消」按钮（Downloading 态）；`DownloadCancelled` → 卡片回 Idle + 提示「已取消，进度已保留」；i18n zh/en 同步加键 |
| 测试 | 会话互斥（在途拒二次启动）；cancel 闭包立即 true → 不发任何 HTTP 即返回；UI 状态机 Downloading→Cancelled→Idle；backend 收 CancelDownload 置位 |

### DL-5（P1）hub 缺失回落机制（落地 D-21 机制半）—— 灭 F8（0.5d）

| 项 | 内容 |
|---|---|
| 机制 | 会话线程 per-model 下载：先选用户 hub；遇 `[http-404]` 或 `KIND_NET` 且 `entry` 另一 hub 有仓 → 日志「{hub} 缺失/不可达，回落 {other}」→ 换 hub 重试一次（仅仓库级一次，不逐文件振荡）。`always_hf` 且无 MS 条目维持单源（whisper MS 镜像条目 = distribution WD-4 数据半，不在本卡） |
| 纯函数 | `next_hub(hub, entry, err_kind) -> Option<Hub>` 独立可测 |
| 测试 | `next_hub` 表测（双 hub 404→回落；always_hf→None；net 错误→回落；已回退过→None）；端到端回落走手动 S9 |

### DL-6（P2）杂项收尾 —— 灭 F9/F11/F12（0.5d）

| 项 | 内容 |
|---|---|
| F9 | 会话线程 join 后再 drain 一次事件 channel 再发成功/失败（尾部「快照就绪」不再丢） |
| F11 | MS URL 改 `reqwest::Url::parse_with_params` 构造（`Revision`/`FilePath` 正确编码），零新依赖 |
| F12 | `PanelUiState` 增缓存探测结果缓存（2s TTL + `DownloadSucceeded`/`SwitchEngine`/`DownloadCancelled` 事件即时失效），`model_cache_status` 不再每帧扫盘 |

## 6. 测试与验收

### 6.1 新增单测清单（≥16，全部离线无网络）

DL-1⑥ 项注册表一致性 / ①~⑤ 探测六项；DL-2 分类表测 / 跳过校验 / rename 竞态；DL-3 解析表测（≥5 用例）/ 状态机；DL-4 会话互斥 / cancel 即停 / 状态机三态 / backend 置位；DL-5 `next_hub` 表测（≥4 用例）。合并即 ≥16。

### 6.2 手动验收场景（实机，全过才算收口）

| # | 场景 | 预期 |
|---|---|---|
| S1 | 下载 sensevoice 中途断网 → 恢复后点「重试」 | 真实续传/重下，不再假「下载完成」；`.incomplete` 从不被判已缓存（F1） |
| S2 | 正常下载观察卡片 | 进度条推进、显示「文件名（k/n）」与字节计数；两文件时随文件切换归零且标签清晰（F3/F10） |
| S3 | 停用网络点下载（或指一个 404 mock 端点） | ≤2s 报「http-404 建议切下载源」，无 21s 白等（F5） |
| S4 | 下载中点「取消」 | 立即停，卡片回 Idle 带「已取消，进度已保留」；再点下载从断点续传（F7） |
| S5 | 下载期间改设置（语言/主题/VAD）并等下载完成 | 设置不被回踩，`settings.json` 与 UI 一致；下载期间引擎热切换照常生效（F6） |
| S6 | 选 nano 经 MS 下载完成后回识别页 | 卡片显示「✓ 已缓存」；不再反复「未缓存→下载→成功」（F2） |
| S7 | 已完整缓存时点下载（重试幂等路径） | 秒回成功，无网络请求（DL-2 跳过校验不误伤） |
| S8 | 手工把缓存里 `model.int8.onnx` 截掉一半 | 卡片显「未缓存」，点下载自动重下该文件而非跳过（F4） |
| S9 | hub 选 MS 但 MS 仓 404（改 hosts 或 mock） | 自动回落 HF 成功，日志可见回落行（F8） |

## 7. 风险与回滚

| 风险 | 缓解 |
|---|---|
| manifest 下限校准过紧 → 假阴性重下循环 | 下限刻意取「远低于实际」（50MB vs 实际 239MB 量级）+ 首日 API 实测登记；即便误判，DL-2 跳过校验用同一套下限 → 重下是真实重下，系统收敛不假成功 |
| lt-proto 追加成员 | 仅追加不改动；Cmd/UiEvent 走进程内通道不涉 worker IPC 序列化兼容；先例 `Cmd::TestTranslator` |
| 会话线程重构动编排主干 | run_download 主体整体搬移不改内部逻辑；DL-4 独立 commit 可单独 revert |
| fsync 增时 | 仅模型大文件收尾一次，~ms 级，进度感知无影响 |
| 阈值/探测语义变化影响既有缓存用户 | manifest 判定对「真完整」目录必真（文件本就在），只把「假完整」翻成未缓存 → 下次下载真实补齐，属自愈 |

## 8. 提交纪律

- 顺序：`docs(download-overhaul)` 本文档先行（已满足）→ DL-1 → DL-2 → DL-3（P0 逐卡独立 commit，`feat(dl-N): …`）→ DL-4 → DL-5 → DL-6。
- 每卡完成前提：`cargo test --workspace` 全绿 + 对应手动场景抽测；禁 `git add -A`，逐 pathspec。
- 文档状态行随卡同步（本文档 §5 各包完成后标注 ✗ + commit hash）。

## 9. 新偏差登记（提案，落地时回写 `docs/archive/rewrite-research.md` §1.5）

| 提案编号 | 内容 | 与原版关系 |
|---|---|---|
| D-22（提案） | 缓存完整性探测从原版体积阈值（min_bytes 50MB/半体积）改为注册表 manifest 逐文件校验 | 行为收紧：原版阈值启发式存在本方案 F1/F2 同类误判 |
| D-23（提案） | 下载可取消（`Cmd::CancelDownload`/`UiEvent::DownloadCancelled`），取消保留续传现场 | 新增能力：原版下载对话框无取消按钮 |
| —— | hub 缺失回落不占新编号：D-21 已裁决，DL-5 为其机制落地 | —— |

> 编号冲突规则：若 asr-engine-expansion WP-B 先于本方案落地并占用 D-22，则顺延；最终以回写归档决策史时登记为准。
