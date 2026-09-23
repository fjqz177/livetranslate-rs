# scripts 脚本目录页（索引，非手册）

> 本页只回答「有哪些脚本、各是什么、怎么调」。**参数细节与失败行为永远以脚本自身头注为准**（唯一细节真源，AGENTS §4）。
> 新增 / 改名 / 删除脚本必须同步本页：守护断言 8 做双向对账——磁盘有而本页无节（漏登记）、本页有节而磁盘无（孤儿条目）都会红。

## 门禁

### `scripts/precommit.ps1`
提交前门禁：三项顺序执行（fmt → 总纲健康守护 → clippy），任一失败即停；git 钩子与 CI 同源调用本脚本（ADR-21 七项并三项）。
典型调用：`pwsh -File scripts/precommit.ps1`
参数概览：`-RepoRoot`（一般不用传）。
参数细节与失败行为以脚本头注为准。

## 守护

> 另三处机械守护自 ADR-21 起以 Rust 形态存在（非 scripts/ 脚本，不设节）：
> 依赖白名单 = `crates/lt-app/tests/topology.rs`；文本卫生+个人路径 = `crates/lt-app/tests/repo_hygiene.rs`；
> 契约纯度 = `crates/lt-proto/tests/contract_purity.rs`；源码禁令 = 根级 `clippy.toml`。均随 `cargo test` / `cargo clippy` 执行。

### `scripts/check_agents_health.ps1`
总纲与文档引用网健康：AGENTS 引用路径 / G-编号死引用 / 看板小节 / 引用网扩面 / 归档对账 / 看板双真源（断言 2~7 + 9；断言 8 = 本目录页的双向对账，详见该脚本头注）。
典型调用：`pwsh -File scripts/check_agents_health.ps1`
参数概览：`-RepoRoot`（一般不用传）。
参数细节与失败行为以脚本头注为准。

## 发布与环境

### `scripts/fetch_sherpa_libs.ps1`
首次 / 换机预取 sherpa-onnx 预编译静态库到 .cache/sherpa-onnx（缺失则构建硬报错）。
典型调用：`pwsh -File scripts/fetch_sherpa_libs.ps1`（GitHub 慢可走镜像，参数见头注）
参数概览：`-Mirror`（另有环境变量 `SHERPA_ONNX_MIRROR`）。
参数细节与失败行为以脚本头注为准。

### `scripts/package_release.ps1`
本地 zip 打包：release 单 exe + 手册与许可五件 → dist 下的 LiveTranslate-<version>.zip。
典型调用：`pwsh -File scripts/package_release.ps1`
参数概览：`-RepoRoot`（一般不用传）。
参数细节与失败行为以脚本头注为准。

### `scripts/release.ps1`
发布链引擎：校验 / 构建 / 打包 / 草稿 / 回读 / 转正一条链，本地与 CI 同一份（D-90）。
典型调用：`pwsh -File scripts/release.ps1 rehearse`（演练，不碰任何 Release）；`pwsh -File scripts/release.ps1 notes`（只读，本版正文预览）
参数概览：动词 `check` `build` `pack` `draft` `notes` `verify` `rehearse` `release` `promote`（先后依赖见头注）；`-NotesFile`（仅 promote）；`-Rehearsal`。
参数细节与失败行为以脚本头注为准。

## 工具

### `scripts/grab_reference_ui.py`
原版参照截图重拍：程序化拍工作区 LiveTranslate/ 副本 UI，产出 assets/reference/ 对照图（zh/en 各一套）。
典型调用：`python scripts/grab_reference_ui.py`（解释器须带 PyQt6 等，详见头注）
参数概览：无参数。
参数细节与失败行为以脚本头注为准。

### `scripts/silero_reference.py`
Silero VAD 置信度对照：与 Rust 实现逐点比误差（< 1e-4 判 PASS）。
典型调用：先 `cargo run -p lt-audio --example vad_check`，再 `python scripts/silero_reference.py`
参数概览：无参数。
参数细节与失败行为以脚本头注为准。
