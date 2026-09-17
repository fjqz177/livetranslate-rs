# 文档体系加固（doc-network-hardening）

> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准——「施工中」为收口前原貌，实际已完工（八断言 + 红绿演练 + 624+8 全绿）。
> 修订注记（2026-09-17，ADR-18）：用户裁决取消机械额度与水位线——本文 DH-12（额度句指针化）、DH-17（水位线）与 ADR-17 的额度方案随之退役；宪法回吐、守护断言 5~7 与钩子触发面不受影响。
> 状态：**施工中**（2026-09-17 定稿即开工，完工即归档）。定稿依据 = 三轮调研评审稿（v1.0 调研 → v1.1 八原则评审 → v1.3 完整性修订；研究稿在工作区草稿区，不入库）。
> 触发：用户令「文档体系从 README 起全面审视关系网 → 严肃评审 → 动工加固」。
> 前缀：**DH**（本档专属；正文 E/N/P 等为叙述性标签非工作项号）。
> 决策：**D-93**（行为/流程批）+ **ADR-17**（容量重校准与真源指针化）。
> 证据时点：HEAD = 7e02d21（2026-09-17 晚）；全部 file:line 经主线程亲手复核。

## 1. 背景取证（摘要）

三类结构性病灶 + 一处机制校准失灵（详细证据行号见 git 历史研究稿与决策评审记录）：

1. **说谎的指针**：distribution.md 六处被代码/决策史证伪（WD-2/3/5 状态、包名、包内容、头注自相矛盾——其中 WD-2 三件实已全落地：`build.rs:11-14`、`main.rs:32`、`changelog_tab.rs:32-34`；WD-4 实证未做）；archive 37 份中 8 份头注仍写归档前状态（「待施工」「未提交」等）、24 份无任何归档标记。
2. **单点依赖**：七张流程卡互不连线，流程闭环悬在 AGENTS §4 一张表上，无反向孤儿守护；`kickoff.md:16` 引用「AGENTS §5」属宪法单点。
3. **摘要漂移**：测试基线三值（README「603+9」/ AGENTS「610+9」/ 最新收口档「615+9」）；AGENTS §7 精选停在 G-25（全本 27）；README 门禁描述「六项检查」「gate+build 双 job」均已过期；decisions.md D-88 行引用不存在的非数字坑号（G- 残留，出自 ci-workflow-suite 局部号未带路径，违纪律⑥）。
4. **容量校准失灵（ADR-17 立案）**：AGENTS 主体 17225/17408 B（98.9% 顶死，余 183 B）——额度按 2026-09-14 重写当日 + 极小余量定、无增长政策；额度数字双真源（AGENTS:6 与 check_agents_health.ps1:42-45 各写一遍）；已诱发字节等量编辑杂技并疑似诱发「怕动宪法」式漂移（基线 610 过期，推断级）。
5. **权威倒挂**：9 个活机器文件（守护脚本×6、Cargo.toml×2、assets/SOURCES.md）把只读 archive 当规范源，仅 architecture-v2.md 有降级注记。

## 2. 方案裁决

**D-93（文档网络加固）**：① 全仓清淤（§3 DH-1~13）；② 守护扩容断言 5~8（DH-14~17）；③ 钩子触发面改规则「守护读谁、谁触发」（DH-18）；④ 流程卡串成流水线（DH-19~25）。

**ADR-17（容量重校准，拍-6 选①）**：AGENTS 两档额度上调为**主体 ≤190 行 / 19KB、全文件 ≤230 行 / 24KB**；额度数字唯一真源移入 `scripts/check_agents_health.ps1`（宪法头注只留指针）；新增 85% 水位线 WARN（不失败、退出码恒 0）；加字优先以「回吐」支付（与流程卡重复的内容迁回卡片）。

**拍板六项结果**（用户 2026-09-17 令动工，按推荐执行）：拍-1 看板留宪法｜拍-2 不抽规范文档、挂降级注记｜拍-3 不加 decisions 现行视图｜拍-4 随拍-6 联动 → **补 G-27 进 §7**｜拍-5 近孤儿 4 档不补指针｜拍-6 = 重校准①。

## 3. 施工卡

> 提交切分：① 定稿（本提交）→ ② 脚本注释与七卡 → ③ 门面与账本 → ④ 宪法回吐+额度 → ⑤ 归档加注 → ⑥ 守护上线。每提交前手动跑 precommit（⑥ 落地前钩子不覆盖 scripts 与 docs 面）。

### DH-1 `docs/decisions.md` D-88 行
行内遗留的非数字坑号残号（dependabot 收窄项）改为带路径实指：「（局部号见 docs/archive/ci-workflow-suite.md §二 G 项）」。

### DH-2 `scripts/check_agents_health.ps1` 头注
`:2` drafts 旧路径 → `docs/archive/agents-md-overhaul.md`；`:15`「precommit.ps1 第七项」→「第六项」。

### DH-3 `scripts/precommit.ps1:17`
「见 AGENTS.md『约定』」→「见 AGENTS.md §2『命令与门禁』」。

### DH-4 根 `README.md` 六处
① `:67`「17 条实测踩过的坑都在那」→「大坑速查都在那（坑册全本 = docs/gotchas.md）」；② `:104` 基线「603 通过」→「约 600+ 通过（滚动值；操作基线见 AGENTS §2）」；③ `:108`「约 76MB」→「约 72MB（滚动值）」；④ `:129` 门禁句重写为「自动跑 scripts/precommit.ps1 全套门禁（秒级守护 + clippy），全过才生成提交；仅 assets/ 资产类提交自动跳过；应急 --no-verify（CI 仍拦）」；⑤ `:131`「gate 静态门禁 + build 编译门禁」→「单 job：与本地同源的 precommit.ps1 + 全量测试 + 分发演练」。

### DH-5 `README.txt`
`:33`「设置 → 识别」→「设置 → VAD / ASR」；`:45`「完整说明见项目 README.md」→「完整说明见 GitHub 仓库 README.md」。

### DH-6 `docs/distribution.md`
`:3` 状态行 →「长期文档（常青）：WD-6/7/8 未完，其余已落地」；`:8`「分发层完全空白」完成时化；`:29` 体积改滚动值；`:43` 包名 → `LiveTranslate-<版本>.zip`；`:44` 包内容 → zip 实况五件套 + sha256 旁车（真源 = scripts/package_release.ps1）；§3.3 加状态列（WD-1 已完工 / WD-2 已完工 / WD-3 已完工 / WD-4 未做（预案）/ WD-5 已完工（D-73）/ WD-6 待点头）；§4 WD-7「✅」→「已完工」；§7 收敛为指针 + 保留「关联既有偏差」行；WD-9 行加「README.md 用户节与 README.txt 去重」。

### DH-7 `docs/prompts/live-check.md:6`
CUA 禁令（2026-09-08）→ 节制使用三条件（用户 2026-09-14 改口）：只读探针优先；操作须有明确验收点；有更轻替代时不为用而用。

### DH-8 `docs/prompts/handoff.md`
memory 动作补落点说明：项目记忆存于仓外 ZCode 工作区（`~/.zcode`），不在本仓库内。

### DH-9 `AGENTS.md` 去数字
`:69`「共七张」删除；`docs/README.md` 长期表「开发流程七卡」→「开发流程卡」（归档表内「七卡」为史档原文不动）。

### DH-10 `AGENTS.md:27` 基线
「610+9」→「615+9」（字节等量）。

### DH-11 宪法回吐（commit message 写明信息去向）
`:41` 冒烟行 → 「- 冒烟（临时配置目录 / models_dir / --version 旗标）= docs/prompts/live-check.md。」；`:92` 离线纪律行删除（真源 = implement.md）；`:97` 子代理分工行删除（真源 = kickoff.md）；`:90` 主干直推行只留「提交直接落 main，无 PR 流程（CI 全分支 push 兜底）」。

### DH-12 `AGENTS.md:6` 额度句指针化
「预算两档——§0~§7 主体 ≤170 行、≤17KB，全文件（含 §8 看板活状态）≤210 行、≤22KB；」→「预算两档现值唯一真源 = `scripts/check_agents_health.ps1` 断言 1（加字优先以回吐支付）；」。

### DH-13 拍-4：`AGENTS.md` §7 尾补 G-27 一条
对齐坑册 G-27 对策句（pwsh 裸调 GUI exe 不等待 → `Start-Process -Wait -PassThru`）。

### DH-14 守护断言 5（路径存在扩面）
扫描集 = `docs/*.md`（仅顶层）+ `docs/prompts/*.md` + `scripts/*.ps1` + `.github/workflows/*.yml` + 根 `README.md`；正则同断言 2；通配与裸目录提法跳过；AGENTS 保留 drafts 豁免；**脚本/workflow 头注指向 `docs/drafts/<具体文件>` 即违规，docs 顶层/prompts 指向降为 WARN**（看板机制本就合法指向草稿区）；排除 archive（史档）与 gotchas（定义源）；误报入脚本内显式豁免表（带理由注释）。

### DH-15 守护断言 6（G 编号全仓）
同 DH-14 扫描集，`G-([A-Za-z0-9]+)` 每个命中必须 ∈ 坑册 defs。

### DH-16 守护断言 7（归档机械面）
(a) docs/README.md「## 归档文档」节反引号 `*.md` 令牌集合 == `docs/archive/*.md` 集合（双向）；(b) 每份档案前 8 行含「已归档|归档注记」。

### DH-17 守护断言 8（水位线，ADR-17）
主体字节 > 额度 85% → WARN 行；不失败、退出码恒 0。

### DH-18 钩子触发面（规则：守护读谁、谁触发）
`.githooks/pre-commit:22` case → `*.rs* | *Cargo.toml* | .cargo/* | AGENTS.md* | docs/* | scripts/* | .github/* | rust-toolchain*`；hook 头注与 `AGENTS.md` §2 钩子行同步（「仅 assets/ 资产类提交放行」）。deny.toml 不入（本地无 cargo-deny 检查步）。

### DH-19~25 流程卡流水线（卡尾各加一行）
kickoff「→ 出口后：施工期自查 = implement.md」（并删 `:16` 的「（AGENTS §5）」引用，配合 DH-11）｜implement「→ 完工前：review.md（八原则）；涉 GUI/实机另过 live-check.md」｜review「→ 评审过后：closeout.md」｜live-check「→ 走查过后：closeout.md」｜closeout「→ 提交后：handoff.md（会话收尾）」+ 三问行加注「表↔目录一致已由守护断言 7 机械化」+ 第 8 步加「测试总数有变 → 同一提交更新 AGENTS §2 滚动基线」｜decision-request「→ 拍板后：回 kickoff.md 立档开工；未拍板稿留 docs/drafts/ 不入库」｜handoff 终点不加。

### DH-26 归档加注（纯加注，不改原状态句——纪律①）
对「前 8 行无『已归档/归档注记』标记」的 archive 档案（实扫 24 份）逐份在标题行下加：`> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。`；其中 4 份被活文件当规范引用者加权威半句：architecture-v2-improvements（现行权威 = scripts/check_dead_contract.ps1）、path-hygiene（= scripts/check_personal_paths.ps1）、rewrite-plan §2.5（= assets/SOURCES.md）、hide-quit-flow-overhaul 附录 B（已实装，见 crates/lt-ui/src/notifications.rs）。

## 4. 验收基线

1. `check_agents_health.ps1` exit=0（八断言全绿）。
2. 红绿演练五项（破坏→复原，对应断言红→绿）：脚本头注假路径→断言5；decisions 写不存在的坑号→断言6；删归档表一行→断言7a；抹归档注记→断言7b；主体塞超 85% → WARN 现且退出码 0（断言8）。
3. 基线口径：README 无具体测试计数；AGENTS §2 数字 == 最新收口档（615+9）。
4. 额度唯一性（收口后执行，本档归档后其路径退出扫描面）：`grep -rn "19KB\|24KB"` 在 AGENTS/README/docs 顶层/prompts/scripts 与 workflows 仅命中 check_agents_health.ps1。
5. 「AGENTS §5」悬空引用为零；全仓无指向不存在路径的引用。
6. 触发面：改 scripts/ 或 docs/README.md 均触发本地门禁。
7. precommit 全过 + `cargo test --workspace` 全绿（615+9）。

## 5. 遗留走查

- 断言 5 上线首扫若出误报 → 当场入豁免表并在此登记条数。
- 可选杠杆未做（拍板未取）：§7 精选规模 25→10~12 条（再释 ~2KB）；rust-toolchain 已入触发面，deny.toml 维持不入。
- 卡链执行率无机械守护（人为习惯面，消融声明过：冗余保险非关键路径）。
