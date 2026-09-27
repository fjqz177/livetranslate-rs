# scripts 脚本评审与整改方案 v2 —— 2026-09-23

> **状态：已归档（2026-09-27 由 docs/drafts/ 草稿收编入库，docs/archive-index.md 有行）**——S1/S2 已施工收口（提交 09b53ae 主体 + 35826a2 遗漏复查补丁，2026-09-23）：S2-1/2/3/6 迁 Rust 载体、S2-4/5 弃，验收 632+0 全绿 + precommit 三项绿 + fresh clone 复验绿；ADR-21 已登记。两轮全仓复查清干净提法级残留（含 .githooks/rust-toolchain.toml/架构图本地生成物/三处代码注释）。**S3（check_agents_health 九条小修）与 S4（保留脚本打磨）未施工**，细单见 §二，随用随做（涉 .ps1，须代码解禁）。

> **归档核销注记（2026-09-27，非全量复核）**：S3-8（头注「第六项」改「第二项」）已被 S1-1 顺手修掉，销；S3-4 残骸仍实存（现 L56「断言 1：两档预算」）；ADR-22 新增断言 9 后 S3 所引行号全漂，S4 fetch 项须对照 ADR-23（内嵌资产改 uv 直供 .venv，37752e7）后现状——施工时按锚文本对现树逐条重定位，勿照抄行号。
>
> **状态链**：9 脚本评审（2026-09-23 三子代理 + 实跑验证）→ 用户裁决"检测型 PS 守护方向性否决"→ 亲手删 4 守护（git D 未提交）→ check_agents_health 专项研究判"留 PS"→ **本版 = 完整修改方案（S1~S4 四施工包，待拍板/待解禁）**。
> **约束**：零代码整理阶段（D-96）——S1 大部与 S2 定稿登记属文档治理可做；涉 .ps1/.rs/.toml 的施工须用户明示解禁。
> **标注**：[文档] = 文档治理可做；[需解禁] = 涉码施工。

---

## 一、总判定与现状

| 脚本 | 判定 | 一句话 |
|---|---|---|
| check_guards.ps1 | **已删** | 五组禁令：PS 转义死穴（P1 死 needle 实证）；规则去向见 S2-2~S2-4 |
| check_deps.ps1 | **已删** | toml 状态机解析勉强；白名单去向见 S2-1 |
| check_dead_contract.ps1 | **已删** | 大小写+注释双宽容本就不可靠；去向见 S2-5 |
| check_personal_paths.ps1 | **已删** | LF 查与 .gitattributes 冗余、正则转义盲区；去向见 S2-6 |
| check_agents_health.ps1 | **留 PS** | 文档网对账 = shell 舒适区，无更好载体（专项研究见 §五） |
| precommit.ps1 | 留，瘦身 | 调度器本职；七项→三项（S1-1） |
| fetch_sherpa_libs / package_release / release | 留，打磨 | 纯机械流程；各自小修（S4） |

**git 现状**：四删未提交；precommit.ps1 L53~56 挂四个死调用（跑到第 2 项必炸）；check_agents_health 实跑 15 处违规（全部是删脚本的同步缺口）。全仓引用面已 grep 清点完毕——**除守护抓到的 4 处路径死引用外，还有约 10 处提法级死引用**（"七项门禁""三查机械拦截""拓扑权威=check_deps.ps1"等，守护扫不到的散文提法），S1 逐处列全。

---

## 二、修改方案（四个施工包）

### S1 删除收尾同步 —— 把删脚本的账清干净（一次提交做完）

| # | 文件:位置 | 改什么 | 性质 |
|---|---|---|---|
| 1 | scripts/precommit.ps1 L53~56 + 头注 L5~8 | 删四个 Invoke-Gate；头注"七项"重写为三项（fmt → check_agents_health → clippy），顺序理由"便宜先死"保留；顺手修评审 P3：`Invoke-Gate` 加 `$LASTEXITCODE` 空值兜底 + try/catch 失败横幅 | [需解禁] |
| 2 | docs/scripts.md L9/L14 | "七项顺序执行"改三项；节标题"守护（precommit 五项）"改"守护"；删四个孤儿节；precommit 节描述同步 | [文档] |
| 3 | AGENTS.md L36（§2） | "七项清单真源"→"三项清单真源" | [文档] |
| 4 | AGENTS.md §2 钩子触发面注（L38~41） | 理由"守护读谁、谁触发"改写为"门禁读谁、谁触发"（fmt/clippy 读 .rs/Cargo.toml/.cargo；check_agents_health 读 docs/scripts/AGENTS/README）；路径列表本身不变（每类仍各有读者），D-93 原则保留 | [文档] |
| 5 | AGENTS.md L65（§3） | "依赖白名单真源 = scripts/check_deps.ps1"——随 S2-1 拍板改指新测试；若 S2-1 选弃则改"真源 = 本表" | [文档] |
| 6 | AGENTS.md L66（§3） | "（check_guards 禁令 3 机械拦截；禁令 5 = 契约旁路）"括号删除或改"一经发现按 P1 立案"纯纪律提法（随 S2-2/S2-3 拍板） | [文档] |
| 7 | AGENTS.md L137（§7 G-23） | "check_personal_paths 三查机械拦截"提法随 S2-6 拍板改写 | [文档] |
| 8 | README.md L154 | "机器强制（scripts/check_deps.ps1 + CI）"改指新去处（随 S2-1） | [文档] |
| 9 | docs/gotchas.md L200（G-23 对策段） | "入库强制 = check_personal_paths.ps1 三查（precommit 第 2 项）"随 S2-6 拍板改写；坑本体与史不改 | [文档] |
| 10 | docs/prompts/implement.md L3 / review.md L18 | "check_deps.ps1 会拦"／"check_guards 五禁令"提法随 S2 拍板改写（精神面自查条目保留，机械提法换） | [文档] |
| 11 | docs/README.md L53 | 归档表"拓扑权威=check_deps.ps1"改指新去处；L35/L44 的史实提法（描述当时事实）不改 | [文档] |
| 12 | 三份归档的"现行权威"注记 | architecture-v2.md L3、architecture-v2-improvements.md L2、path-hygiene.md L2 的"现行权威 = scripts/check_xxx.ps1"补一句"（2026-09-23 起该守护退役，现行权威 = …）"——死导航必须改，史本体不动 | [文档] |
| 13 | .github/workflows/ci.yml L6/L41/L82 | 三处"七项"提法改三项（头注 + 两个 step 名）；调用方式不变（整脚本同源原则 D-88 不动） | [文档] |
| 14 | docs/decisions.md | 随本批定稿同一提交登记 **ADR-21**（守护体系瘦身：四检测守护退役 + 规则选择性迁移 + check_agents_health 留任），条目按先例含档案指向 | [文档] |

注：.githooks/pre-commit 无"七项"提法，不动；docs/archive/ 其余引用均为史档正文叙述，按"归档不扫"原则不动。

### S2 原守护规则的迁移（六子项，逐条拍板；全部 [需解禁]）

#### S2-1 依赖白名单（原 check_deps）→ 推荐：迁 `crates/lt-app/tests/topology.rs`

- **形态**：lt-app dev-dep 加 `toml`；测试用 `CARGO_MANIFEST_DIR` 上溯两级遍历 `crates/*/Cargo.toml`；白名单从 check_deps.ps1 平移为常量表（10 crate → 允许 lt-* 集 + dev 特批）；解析 `[dependencies]`/`[dev-dependencies]`/`[target.*.dependencies]`/`[build-dependencies]` 四节——**顺手修 PS 版 M6 盲区**（PS 连 `[target.'cfg(windows)']` 节头都解析不了）。
- **断言消息**：违规边一行一条"crate X 的 dependencies 含 lt-Y，白名单允许 {…}"。
- **运行位**：cargo test --workspace（收工门禁）自然覆盖；可选拒进 precommit（`cargo test -p lt-app --test topology`，clippy 已全量编译后增量秒级）。
- **为什么放 lt-app**：十 crate 里唯一允许依赖全部的 组合根，拓扑断言放它名下语义最顺；避免新建 crate。
- 备选：弃（白名单真源=AGENTS §3 表，靠 review 与 implement.md 自查卡）。

#### S2-2 三组方法禁令（原 check_guards 禁令 1/2/4）→ 推荐：迁根级 `clippy.toml`

官方机制 `disallowed-methods`：编译器级精确匹配方法调用（注释/字符串里的假阳性天然不存在——PS 版靠"剥注释"勉强绕，clippy 根本不看注释）；报错带 reason；豁免 = 代码现场 `#[allow(clippy::disallowed_methods)]` + 理由注释，比集中白名单表更贴近代码。**precommit 已跑 clippy，零新增步骤。**

草案（施工时以 winit 实际版本路径为准）：

```toml
# 仓库纪律真源 = AGENTS.md §3；本文件 = 机械强制面（原 check_guards 禁令 1/2/4，ADR-21）
disallowed-methods = [
    { path = "std::thread::spawn",  reason = "线程出生点必须走 Supervisor::spawn（监督树 INV3）" },
    { path = "std::thread::Builder::new", reason = "同上" },
    { path = "std::thread::scope",  reason = "同上——scope 出生同样逃出监督树（PS 守护曾漏此项，本条为净增强）" },
    { path = "std::panic::set_hook", reason = "panic hook 仅限 lt-app：main.rs 安装 / panic_hook.rs 本体" },
    { path = "winit::event_loop::EventLoopProxy::send_event", reason = "事件发送唯一通道 = lt-app 动脉桥（INV1）；例外已就地 #[allow]" },
]
```

豁免迁移表（原白名单逐条去向，施工时对照 `git show HEAD:scripts/check_guards.ps1`）：

| 原白名单条目 | 去向 |
|---|---|
| supervisor.rs（监督器本体）、tray.rs（D-35 托盘）、client.rs（I/O 泵）、wasapi_win.rs（读环）、bench.rs（基准）、download.rs（下载 worker） | 各 spawn 调用处 `#[allow]` + 原注释理由随行 |
| fake_asr_worker.rs、capture.rs/probe.rs（cfg(test)）、lt-download lib.rs（#[ignore] 探针）、tests/* 夹具 | 各文件/模块头 `#[allow]`（测试基建豁免显式化） |
| proxy 白名单 4 条（artery/shell/singleton/app.rs） | 各 send_event 调用处 `#[allow]` |
| panic hook 2 条（main.rs/panic_hook.rs） | `#[allow]` 就地 |
| lt-translate/lib.rs（评审 P2 死条目，0 命中无注释） | **不迁**（随守护消亡） |

- 成本：clippy.toml 新文件 + 约 15 处 `#[allow]` 标注，一次性。
- 备选：弃（监督树/proxy 纪律只剩 AGENTS §3 文字）。

#### S2-3 契约旁路（原禁令 5：events.rs 禁 serde_json::Value 载荷）→ 推荐：`crates/lt-proto/tests/contract_purity.rs` ~20 行

单文件单模式（读 `src/events.rs` 源文本查 `serde_json::Value`），clippy 表达不了按文件差异，但 20 行测试足够。备选：弃（events.rs 是最核心契约文件，review 必看）。

#### S2-4 字符串协议禁令（原禁令 3）→ 弃（已定）

PS 4 形状本就拦不全、死 needle 实证其烂；换载体仍是文本匹配，拦不住想绕的人。回归 review 纪律（prompts/review.md 精神面自查条目已有位置）。

#### S2-5 死契约检查（原 check_dead_contract）→ 推荐：弃

理由：①现有实现大小写+注释双宽容（评审实证），本就拦不准；②WARN 需人工定性，不是纯机械——恰属你否决的那类"伪机械"；③低频巡检非门禁事；④真死契约 rustc dead_code 不跨 crate 报 public 项，但攒到显眼处 review 可逮。备选：并入 lt-app/tests 一个 ~80 行文本计数测试（Rust 字面量无转义坑，顺手修大小写宽容）。

#### S2-6 文本卫生 + 个人路径（原 check_personal_paths）→ 推荐：`crates/lt-app/tests/repo_hygiene.rs` ~80 行

- **LF 查：弃**——.gitattributes 已锁 index LF（ADR-20），重复真源才是危害。
- **BOM（EF BB BF）+ 严格 UTF-8**：git 不管，测试字节扫描保留（pwsh 7 only 后 BOM 危害面已收窄，属防手滑低频项）。
- **个人路径（PH-5，用户硬纪律）**：无现成工具；Rust 正则 `r"C:[/\\]{1,2}Users"` 无 PS 转义歧义，**顺手修 Tier1 双反斜杠盲区**；archive 内 Tier2 降为 eprintln 提示不 fail（平移原降档语义）。
- 扫描面：`git ls-files`（std::process::Command，最准）+ 二进制后缀排除（平移原清单）。
- 备选：a) editorconfig-checker 管 BOM/EOL（CI 现成二进制，但个人路径仍无载体、CI 多一外部依赖）；b) 全弃（个人路径纪律退化为 review）。

#### S2 拍板后的连锁（并入 S1 对应行执行）

S2-1 → S1-5/8/11 改指；S2-2/3 → S1-6/10 改指；S2-6 → S1-7/9 改指。若某子项选弃，对应提法改"纪律条目，无机械强制"。

### S3 check_agents_health.ps1 打磨（[需解禁]，9 条小修）

1. 断言 6 违规输出改相对路径（现只给叶子名，根/ docs 两 README 分不清）。[改文案]
2. 删两处死 `Contains('*')` 分支 + 注释改如实（"目录提法按目录 Test-Path"）。[改结构]
3. gotchas.md 缺失时短路断言 6（根因已由断言 3 报过，防噪音雪崩）。[改结构]
4. L69"预算"残骸文案 + L55~59 五个退役常量删除（留名防旧引用的使命已尽，ADR-21 登记 retiring 后无引用风险）。[改文案]
5. 断言 7 节提取尾锚改 `\r?\n## |\z`（免疫"归档文档是最后一节"）。[改结构]
6. 重复注释行（L123~124）、漂移行号引用（:128→:137）、`ReadAllLines`→`Get-Content -TotalCount 8`。[改文案]
7. 断言 3 正则补 lookbehind 对齐断言 6。[改行为]
8. 头注 L23"precommit.ps1 第六项"→"第二项"（S1-1 连锁）；L197"11 节"注释随 scripts.md 瘦身刷新为动态表述。[改文案]

### S4 保留脚本打磨（[需解禁]，从评审提炼，已剔除指向死脚本的条目）

**fetch_sherpa_libs.ps1**：① tar 探测循环 `$ver` 覆盖改 `$tarVer`（P2，首跑下载提示打垃圾版本号）[改结构]；② 416 补救文案补"删 .tmp 重跑"半句 [改文案]；③ 成功后清旧版归档与 `.probe-tmp` [改行为]；④ M2 sha256 校验 + SOURCES.md 补条目（对齐 D-83 凡加载必验精神）[改行为]。

**package_release.ps1**：① 打包后列出 dist/ 旧版残留提示 [改文案]；② 版本提取锚定 `[workspace.package]` 节（防文件序脆弱）[改结构]；③ 与 release.ps1 收拢五件套清单/版本正则两处重复真源（原 L13~L15）[改结构]。

**release.ps1**：① 头注 L25"无远端依赖"改实（check ④⑤ 软依赖 gh）[改文案]；② check ⑥ 探针改文件级（对齐 fetch）[改结构]；③ **本地收尾打"草稿等转正：promote"两行提示**（最大新手陷阱：release 不发布、promote 才发布，该提示现仅 CI 可见）[改文案]；④ `$match` 改名 `$verLine`（遮蔽自动变量埋雷）[改结构]；⑤ 微项四件：非 promote 传 `-NotesFile` 警告 / verify 清理挪 finally / pack 哈希算一次 / 头注 L34 补"成功后" [改文案+改结构]。

**precommit.ps1**：除 S1-1 瘦身外无单独项（兜底修并入 S1-1）。

---

## 三、已删四脚本的评审存档（死因各一句，详情见 git 历史与本文件 v1）

- **check_guards**：P1 死 needle（PS 双引号转义语义 → needle 匹配 Rust 里不存在的形状）+ thread::scope 漏网 + 死白名单条目。死因定性：**匹配代码字面形状超出 PS 能力，且有 clippy 官方载体**。
- **check_deps**：toml 状态机解析（`[target.'cfg(windows)']` 节解析不了、build-dependencies 不在审计面）。死因定性：**结构化格式解析无生态，且有 toml crate 载体**。
- **check_dead_contract**：大小写+注释双宽容通道、头注数字漂移、lt-proto 排除被 clone 目录名劫持。死因定性：**规则本身需人工定性（WARN），伪机械**。
- **check_personal_paths**：LF 查与 .gitattributes 完全冗余；Tier1 正则双反斜杠盲区；读取失败静默跳过。死因定性：**一半冗余一半载体错配**。

## 四、保留五脚本的评审详情

### check_agents_health.ps1（233 行）

**实际干什么**：对 AGENTS.md 与文档网跑七组断言（2 引用路径实存 / 3 G-编号对账+连续 / 4 看板三小节 / 5 引用网扩面分级 / 6 全仓 G-死引用 / 7 归档双向对账+注记 / 8 scripts.md 双向对账）。头注基本如实。

**判对项**：G-29/G-30 机械落实（豁免表五条全带理由、定义源自豁免）；断言 8 真双向无豁免、守护自身须登记实测有效；草稿区分级与头注一致；lookbehind 正则讲究；绿一行/红逐条/WARN 黄三层分明；自扫靠真值不靠豁免；平铺非递归局限自我声明。

**问题**（除已入 S3 的 9 条外无遗漏）：问题清单已全部转入 S3，此处不重复。

### precommit.ps1（66 行）

**实际干什么**：门禁调度器，便宜先死、任一失败即停透传退出码。"同源"声明经核实为真（ci.yml 整跑 + .githooks exec）。

**判对项**：每项带实测秒数；失败横幅"门禁未过：X（退出码 N）"；`$PSScriptRoot` 绝对路径不依赖 cwd；正确地不提供跳过参数（保"本地过关 ⇔ CI 过关"）。

**问题**：已全部转入 S1-1（瘦身 + 兜底）。

### fetch_sherpa_libs.ps1（118 行）

**实际干什么**：Cargo.lock 解析 sherpa 版本 → 选 tar → 断点续传下载 120MB → 解包探针校验原子换名。头注如实。

**判对项**：失败路径卫生好（临时目录先清后建、两步换名防半成品）；文件级探针；curl 参数讲究；MSYS tar 两坑规避有注释说理；`$ErrorActionPreference='Stop'` + 逐命令查退出码无遗漏。

**问题与建议**：已全部转入 S4。

### package_release.ps1（66 行）

**实际干什么**：抠版本 → 校验 exe → 重建 staging 拷五件 → 打 zip。头注如实（连"缺件不失败"都自曝）。

**判对项**：头注零漂移；缺 exe 报错给出要跑的命令；staging 先删后建；收尾打印体积。

**问题与建议**：已全部转入 S4（含 dist/ 残留 0.1.0 zip 实测、版本提取文件序脆弱）。

### release.ps1（434 行）

**实际干什么**：九动词发布引擎，本地/CI 同源；三大不变量（CI 永不可发布、字节冻结、promote 三重闸）代码里真实成立。`notes` 实测跑通。

**判对项**：434 行无死代码、结构清晰；G-27 无遗漏面；CI 不可发布三道闸结构性成立；全链幂等可重跑；报错带"哪步+原因+补救"；非法动词报错自解释；CHANGELOG 解析考虑周全；check ④ 用 API ahead_by。

**问题与建议**：已全部转入 S4（含本地收尾语义误导——最大新手陷阱）。

---

## 五、check_agents_health 专项研究：为什么它适合 PowerShell

1. **任务性质落在 shell 舒适区**：七组断言全是"正则抽引用 → Test-Path 查存在 → 集合比对"的文档网对账——胶水活，非代码语义分析。被删四个的死因它一条不占：无结构化格式解析、无代码字面量匹配（正则模式无反斜杠陷阱）、无官方替代载体（clippy/toml crate 管不了 Markdown）、无冗余机制。
2. **评审证据反读**：8 条问题全是 P3（零 P1/P2），归因均为手滑/维护漂移——换任何语言照样发生；PS 真正的语言性陷阱在它的正则模式里没有风险面。
3. **迁移收益已消失**：上轮"迁 Rust 测试"的前提是与其他检测统一载体——四个删掉后统一对象不复存在。单迁它 = cargo test 里职责错位的孤例（十 crate 无"仓库卫生"宿主），且连锁改 precommit、钩子触发面、AGENTS §2、断言 8 自指结构——连锁成本大于 233 行重写本身。
4. **迭代成本不对称**：断言随文档治理高频演化（2~8 即随 D-93/ADR-19 长出），PS 改完即跑；Rust 每次要编译。
5. **战功实证（当场）**：用户删四脚本后立即实跑，15 处违规精确覆盖全部同步缺口，一处不漏。

---

## 六、跨脚本共性观察（保留五脚本范围，已删过时项）

1. **头注里的精确数字是头号腐烂点**：×60 变体、10 crates、"11 节"、行号引用——教训：少写会漂移的数字，多写不变的理由；要指位置优先写可 grep 的锚文本。
2. **失败恢复是发布链三脚本共同强项**：fetch 续传、package 重建 staging、release 全链幂等——全部闭眼重跑不留半成品，别动它。
3. **性能都不是问题**：实测 0.4~2.0s，precommit 序列里无感。多遍重读留一句"够用不改"，防好心优化引入新坑。
4. **dist/ 与 .cache 生命周期没人管**：旧产物（0.1.0 zip、旧版归档、.probe-tmp）安静堆积，三处都缺"清旧"或提示（S4-1③/S4-2①）。
5. **原生命令纪律一致且无漏**：统一 `$ErrorActionPreference='Stop'` + 逐命令查退出码——全仓底子最好的部分。

---

## 七、附带发现

- **noncode-infra-issues-2026.md 已落后于树**：其 L2/L19 在现树已修但草稿未销账，建议刷新，否则按单施工会扑空。
- **守护链实测基线**：check_agents_health 0.4s / fetch 类未动——保留脚本全部健康。
- **删除动作本身的风险提示**：四守护承载的规则（依赖白名单、裸线程禁令、panic hook 位置、文本卫生、个人路径）在 S2 施工前处于**无机械守护状态**——S1/S2 越早收尾，裸奔期越短。
