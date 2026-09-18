# 脚本管理与文档体系方案

> **状态**：施工中（2026-09-19 用户拍板开工；ADR-19。定稿提交先于第一行施工代码——docs/README.md 纪律③）
> **日期**：2026-09-19（v1.2 同日三子代理评审修订：对抗评审 / 事实核查 / 施工预演三路并行，发现汇总见 §13）
> **触发**：用户提出「每个脚本头注写全全部用法与参数 + docs 放一份集中文档给人类阅读」，经讨论与两轮评审收敛为本方案
> **局部前缀**：SD-（定稿时按 docs/README.md 纪律⑥校验全局唯一，含 archive）

---

## 0. 一句话总纲（全方案就这一句）

**每个脚本自己的头几行 = 它唯一的说明书。其他所有文档只指向它，不许抄它的内容（唯一既有例外 = AGENTS §2 的高频调用行，且其存量参数级细节随 SD-2 一并收敛，见 §9）。**

这句话是整个体系的地基。后面所有规矩、守护、流程，都是为了让这句话长期成立。

---

## 1. 背景：现在是什么样，差在哪

### 1.1 现状

- `scripts/` 共 11 个脚本（9 个 `.ps1` + 2 个 `.py`，实测 2026-09-19）。
- 本仓已有约定：**每个脚本的用法，真源就是它自己的头注**（AGENTS §4「写/改守护脚本 → 各守护脚本头注」；ci.yml 头注明写「细节真源 = 本文 + scripts/precommit.ps1 头注」，AGENTS §2 同样声明）。这个方向已经对了，本方案不是推翻它，是把它执行到位。
- 集中文档：没有。AGENTS §2 只收了日常高频命令的调用行。
- 仓库现状变了：**public 仓（2026-09-09 起）+ v1.0.0 已正式发布（2026-09-19）**。GitHub 上任何访客点开 `scripts/` 目录，只能一个个点开脚本看头注，没有导览页。

### 1.2 问题清单（按「用法卡」严标准核对；v1.2 经事实核查代理逐条实证修正）

| 编号 | 问题 | 在哪 |
|---|---|---|
| P1 | **6 个 ps1** 有 `$RepoRoot` 参数，头注全都没提：check_agents_health / check_dead_contract / check_deps / check_guards / package_release / precommit。（check_personal_paths **无 param 块**，用 `$PSScriptRoot` 自定位仓库根；fetch_sherpa_libs 参数是 `$Mirror`；release.ps1 参数是 `$Verb/$NotesFile/$Rehearsal`——各自按实际写，不虚构） | 上列 6 文件 |
| P2 | release.ps1 动词依赖链**残余缺口**：draft 对 pack 产物的显式前置（代码 `Assert-Packed` 强制、头注未写）+ 演练模式下被拒动词未逐个点名（代码拒 draft/promote/release，头注只写「禁写动词」）。（promote 的 CHANGELOG 硬闸、rehearse/release 组成已在头注铁律里，不算缺） | release.ps1 |
| P3 | 头注说「五组禁令」但只点名四组（INV3/INV1/INV9/panic hook），第五组「契约旁路」全名只在脚本末行 OK 消息里 | check_guards.ps1 |
| P4 | 退出码数值语义没有专门交代（判定分级 0/WARN/≥2 已写、WARN 算过可推出，但缺「退出码：」行——对照 check_deps 有此行） | check_dead_contract.ps1 |
| P5 | 强制重下方法没写（真值：删 `.cache/sherpa-onnx/` 下归档档 = 重下+重解；只删 `extracted/` = 只重解不重下；只删归档不删 extracted 会秒退无效） | fetch_sherpa_libs.ps1 |
| P6 | 打包残留与覆盖语义没写（staging 目录打包后保留；zip 先删旧再建新；四件文档「存在才拷入、缺件不失败」——五件套强制是 release.ps1 pack 的职责，此分工头注未写） | package_release.ps1 |
| P7 | 依赖没写（要 onnxruntime + numpy；对照组 grab_reference_ui.py 写了 PyQt6+PyYAML+psutil+openai） | silero_reference.py |
| P8 | 没有集中导览页，公众访客无从下手 | docs/ |
| P9 | 新增脚本没有「必须登记进文档」的机制——漏了没人拦（本方案草稿期自身就漏登了 AGENTS §8 看板，见 §13 #3，属该病的现形） | 守护 |

这些缺口的模式高度一致：**缺的全是「参数、前置、失败语义、副作用」四类**。解法 = 统一格式固定四类（§3）。

---

## 2. 体系：信息住在哪（分工表）

| 信息 | 住哪 | 为什么 |
|---|---|---|
| 参数、默认值、前置条件、退出码、失败行为、副作用（写哪/覆盖什么/残留什么） | **脚本头注**，且只住这一处 | 细节只存一份，改的时候只有一处要同步（依据：D-88，七项清单曾在两处手抄，改一漏一，靠废除手抄根治） |
| 「每个脚本是什么、什么时候用、怎么调」一览 | **docs 顶层 scripts.md（目录页）**，新建 | 给人和公众访客当目录；刻意不抄默认值等细节——抄了就会腐化，不抄几乎不会错 |
| 日常命令入口 | AGENTS §2（已有） | 动手前最先看的地方；**只收高频调用行**，存量参数级细节（-Mirror 语义句、「九动词」计数）随 SD-2 收敛为最小调用行+指针——否则 release.ps1 加第十个动词时，头注和目录页都会被守护逼着改，唯独 §2 的「九」字静默变错 |
| 「谁忘了登记」 | **守护脚本自动查** | 能让机器管的绝不靠记性 |

**分工的本质**：头注管深（细节），目录页管浅（导览），守护管漏（登记）。三层互不重叠，所以没有「双真源」。

---

## 3. 制度一：头注「用法卡」标准格式

11 个脚本统一按这个骨架写（py 脚本用 docstring 同构；**注释形式不拘**——release.ps1 是全仓唯一的 `<# #>` 块注释头注，七字段齐全即可，不必改成 `#` 行注释）：

```
# 〈一句话：这是什么〉
# 用法：
#   pwsh -File scripts/<脚本名>.ps1 〈形态1〉   ← 一句话：这一步干什么
#   pwsh -File scripts/<脚本名>.ps1 〈形态2〉   ← 一句话：这一步干什么
# 参数：（逐个：名字 / 默认值 / 作用。$RepoRoot 这类内部参数也要列，注明「一般不用传」；
#        无参脚本明写「参数：无」——check_personal_paths 另注「$PSScriptRoot 自定位仓库根」）
# 前置：（跑之前什么必须就位——含运行环境：pwsh 7 必须、py 脚本要哪些第三方包；
#        以及形态/动词之间的先后依赖）
# 退出码 / 失败行为：（0 是什么、非 0 是什么；WARN 算过还是拦）
# 产物 / 副作用：（写哪些文件、覆盖什么、留下什么残留）
# 详见：（关联文档 / D-xx / G-编号（实存号））
```

- 没有的项写「无」，不省略行——「写明无」本身也是信息（读者不用猜）。
- **合格标准**：一个没读过代码的人，只看头注就能 ① 用对它 ② 预测错它时会发生什么。两条都满足才算写完。
- 改 .ps1 头注时不得丢 UTF-8 BOM（G-23）。验证手段见 §9 SD-1。
- **头注引用守护肤罩的范围**：断言 5/6 扫描面在 scripts/ 下只收 `*.ps1`（check_agents_health.ps1:110）——9 个 ps1 头注里的仓库路径与 G-编号写错即 precommit 红（免费自动校验）；**两个 .py 的 docstring 不在此面内，引用写错无守护拦**，SD-1 时须人工核对（SD-3 会顺手把 `*.py` 扩进断言 5 扫描面，补上这个洞）。
- 每脚本要补什么 = §1.2 对号 + **附录 A 真值底稿**（三路核查后的逐脚本参数/退出码/缺口实值，施工直接照抄，不再自行考古）。

---

## 4. 制度二：docs 顶层 scripts.md 目录页规范

- 每个脚本一节，固定四件套：
  1. 一句话：是什么、什么时候用；
  2. 典型调用一行（最常用的一两个形态，统一 `pwsh -File scripts/…` 前缀——fetch_sherpa_libs 头注现存的直呼形式顺带在 SD-1 统一）；
  3. 参数概览：**只列参数名 + 一句话作用，不抄默认值**（多动词的列动词清单——`ValidateSet` 钉死，改动词是大动作，腐化风险极低）；逐节清单见**附录 B**；
  4. 每节末尾固定一句：**「参数细节与失败行为以脚本头注为准」**。
- 页首声明：本页是目录不是手册，细节永远看头注。
- **机器可解析格式（断言 8 依赖，必须钉死）**：每节的标题行 = 该脚本的仓库相对路径，反引号包裹，如：
  `` ### `scripts/release.ps1` ``
  守护按 `(?m)^### `(`scripts/[^`]+)`\s*$` 抽取（捕获组即仓库相对路径；终止锚 = 行尾，防节标题拖注解被误吸）。**正文任何位置不得出现此格式的字面量**（写格式示例时用行内代码，不用标题样式），否则册→磁盘向误报孤儿条目。格式改了 = 断言 8 失明，改格式必须连 SD-3 一起改。
- 禁写易漂数字（「共 N 个脚本」之类）——写一次错一次，保安管不了内容，别给它添会过时的内容。

**为什么目录页敢列参数名却不敢抄默认值**：参数名是导览的核心价值（读者得知道有 `-Mirror` 这个东西存在才会去用）；默认值是最常改、改了最容易忘的。列名字丢数值，是「有用」和「抗腐化」的最优切分点。残余风险：改参数名时目录页该节会过时——已并入 §5 规矩②兜住（守护只认节标题路径，正文参数名不在机械面）。

---

## 5. 制度三：日常流程（四条规矩，覆盖建/改/用/删+改名）

1. **新建脚本**：头注按用法卡写全 + 目录页按 §4 格式加一节。忘了加？断言 8 直接报红（见 §6）。**进 `scripts/` 就必须登记，没有豁免**——真一次性的临时脚本别放 `scripts/`（放不入库的临时目录），否则守护就得开豁免表口子。
2. **改了脚本行为**（加参数/改动词/改前置/改产物）：顺手改头注对应行，**并扫一眼目录页对应节的两样东西**：节标题的路径还对不对（没改名就对）、参数名清单还对不对（参数名变了才要动正文）。通常不用动。这是全套体系里**唯一靠自觉**的一条，机器查不出「注释说的和代码做的不一样」——评审时按 docs/prompts/review.md 的文档漂移自查兜底，残余风险如实接受（拍板项 #4）。
3. **用脚本**：记不住命令 → AGENTS §2；要细节 → 打开脚本看头 30 行；公众访客 → docs 顶层 scripts.md。
4. **删脚本 / 脚本改名**：删 = 脚本和目录页节一起删，漏删断言 8 报红（孤儿条目）。改名 = 预期中的**双向双红**（旧名成孤儿 + 新名漏登记）——这是守护在正确工作，不是守护坏了；按红提示把目录页节标题与正文一并改到新名即可。拆分 = 删旧+建新，同款流程。

---

## 6. 制度四：守护兜底（断言 8）

在 `check_agents_health.ps1` 新增一条断言（编号 8，紧接断言 7；模式照抄断言 7 的归档表双向对账）：

- **磁盘 → 册**：`scripts/` 下每个 `.ps1`/`.py` 必须在 docs 顶层 scripts.md 有自己的节（按 §4 钉死的节标题格式抽取），缺 = 红（漏登记，堵 P9）。**枚举 = 平铺非递归**（现 scripts/ 为平铺；实现注释里写明「出现子目录须回改」），显式含 `.py`（勿照抄断言 5 的 `*.ps1` 过滤器）。
- **册 → 磁盘**：目录页里抽出的每个 `scripts/<名>` 路径必须实存，缺 = 红（孤儿条目）。与断言 5 部分重叠，重叠无害（裁定存档：各自独立成立，不去重）。
- **11 节全强制，无任何豁免代码——包括 check_agents_health.ps1 自己**（v1.2 澄清：对账断言读的是目录页与文件清单，不存在「扫到自己内容」的自触发，断言 5 那种整文件自豁免（:128，成因 = 豁免表键必然含路径字面量）在此**范畴不适用**；v1.1 曾错误类推「自身豁免」条款，与「无豁免表」自相矛盾，已删）。
- **docs 顶层 scripts.md 不存在 → 记 violation**（与断言 7 对「## 归档文档」节缺失同款处理；正常流程 SD-2 先于 SD-3，此为健壮性兜底）。
- **无豁免表**：脚本就 11 个，双向对账是硬规则；真有例外场景就别把脚本放进 `scripts/`（§5 规矩①）。
- **编号 8 复用注记**：史档中「断言 8」曾指水位线（D-93/ADR-17 设、ADR-18 退役），且该指称已永久钉在 decisions.md D-93 行与归档表（纪律⑥不删行）。断言 8 头注明写一句「8 号曾为水位线断言，ADR-18 退役后复用；史档中的断言 8 均指水位线，现行定义以本头注为准」。
- **触发面现成覆盖，零改动**：docs/** 与 scripts/** 本来就在钩子触发面（D-93「守护读谁谁触发」）——改目录页或改脚本都会自动跑全套守护。
- **CI 兜底**：本地钩子可被 `--no-verify` 绕过，但 ci.yml 与 precommit.ps1 同源整脚本入 CI（D-88），断言 8 在 CI 再跑一遍——本地绕过 CI 拦，与本仓「CI 是最后防线」结构一致。

**位置选 `docs 顶层 scripts.md` 而不是 `scripts 目录 README` 的依据**：断言 5 的扫描面是 `docs/*.md`（顶层）+ prompts + `scripts/*.ps1` + workflows + 根 README（check_agents_health.ps1:104-113）——**`scripts/*.md` 不在扫描面内**。放 docs/ 下，目录页自己的死引用自动被断言 5 盯着，守护零改动；放 scripts/ 下则要先扩一行扫描面。附注：`.githooks/pre-commit` 自身也不在断言 5 扫描面（它无扩展名、不在任何被扫目录集），其头注引用不受机械校验——现状接受。

**落地纪律**：守护改动必须 ① 干跑通过 ② fresh clone 复跑（G-29：本地绿 ≠ CI 绿）③ 故意删一条目录页节验证真的会红。实现级细节（正则/枚举/文案风格）与伪代码见 §9 SD-3 + **附录 C**。

---

## 7. 依据汇总（每条设计为什么这么定）

| 设计 | 依据 |
|---|---|
| 细节只存头注一处 | D-88 实战教训：同一份清单两处手抄必漂移 |
| 目录页不抄默认值 | 腐化面 = 抄了什么；不抄最常变的那层 = 最抗腐化 |
| 节标题 = 精确路径 + 行尾终止锚 | 断言 8 需要机器可解析锚点；仓内同款先例 = 坑册 `^## G-(\d+)`、归档表行正则 |
| 目录页放 docs/ | 断言 5 扫描面覆盖 docs/*.md、不覆盖 scripts/*.md（行号见 §6） |
| 对账断言 | 断言 7（归档表 ↔ archive 双向对账）现成同款模式，照抄改靶子 |
| fresh clone 验证 | G-29（本机未入库目录掩盖 Test-Path，43 个积压提交首推才暴露） |
| .ps1 保 BOM | G-23（注意：text-hygiene 草稿（docs 草稿区） 若拍板将翻转此政策，两案联动的 BOM 验证以届时政策为准） |
| 集中导览页立项 | 仓库 public（2026-09-09）+ v1.0.0 正式发布（2026-09-19），公众读者从假设变成现实 |
| 不上「头注与代码同步」的机械守护 | 需要语义解析，守护会特判化，比病还贵；靠规矩②+评审自查 |
| 不做迁移 Get-Help / 不做生成器 | 见 §8 |
| 一次性脚本无豁免 | 豁免表 = 新维护面；「入 scripts/ 即登记」边界更简单且可执行 |
| AGENTS §2 存量收敛 | §2 的参数级细节无守护面，「九动词」类计数在动词增删时静默变错——与目录页禁写易漂数字同一把刀 |

---

## 8. 被否掉的备选（留证，防止以后重新提一遍）

| 备选 | 为什么不选 |
|---|---|
| PowerShell comment-based help（`.SYNOPSIS`/`.EXAMPLE`，可用 Get-Help 渲染） | 仓内 11 个头注已是统一中文自由格式、实际查阅习惯是打开脚本看头注而非跑 Get-Help、全量换格式是大 diff 零行为收益。只借鉴字段清单定 §3 骨架 |
| 写个生成器自动产出目录页 | 消灭手抄但引入：新脚本 + 生成物入库 diff 噪音 + 生成器自身维护。11 个脚本、低频变动，成本倒挂 |
| 目录页放 scripts 目录 README | GitHub 浏览体验更好；但不在断言 5 扫描面，要先扩一行。**保留为备选**：若更看重浏览体验，代价 = 断言 5 扫描面加 `scripts/*.md` 一行 + 目录页自身死引用改由断言 8 册→磁盘向兜住 |
| 机械守护「头注与代码同步」 | 同 §7；语义解析守护必然特判化 |
| 断言 8 加豁免表 | 同 §6；边界问题在入口解决（别放 scripts/），不在守护处开洞 |
| AGENTS §2 原样不动只加指针 | §2 存量参数级细节（-Mirror 语义、「九动词」计数）无守护面，是体系外的第二真源残留——收敛成最小调用行+指针才闭环（v1.2 新增否决项） |

---

## 9. 施工卡（拍板后按此执行；定稿 → SD-1 → SD-2 → SD-3 → SD-4，有依赖）

### SD-0：方案定稿（本文档转正）

- 本文档 git mv 至 docs/ 顶层，头注改「定稿」，**随定稿提交连带**（挪卡连带改索引，纪律⑤，一项都不能漏）：
  1. AGENTS §8 **施工中板块新建一行**（草稿期漏登了待拍板板块——P9 病的现形，本次补课；下一个 ADR 号实测 = **ADR-19**，decisions.md 插在 ADR-18 之后节内序；表尾 D-95 存在节内错位属既有状态，只记录不在本包修）；
  2. docs/README.md **活跃文档表加行**（状态 = 施工中；该表不在断言 7 对账面内，漏行无人拦，靠本清单防）；
  3. decisions.md 登记 ADR-19 行（一句话文案：脚本管理与文档体系：头注用法卡 + 目录页 + 对账断言）。
- **前向引用改写（定稿提交能过门禁的前提）**：本文档定稿落进 docs/ 顶层后即进断言 5 扫描面，而正文里 `docs 顶层 scripts.md`（SD-2 才建）与 `docs/archive/ 同名档`（SD-4 才存在）在当时都不存在 → **定稿提交必红**。定稿改稿时把两类前向引用写成断言 5 正则不匹配的形式（如「docs 顶层 scripts.md（SD-2 建）」「归档至 docs/archive/ 同名档」——正则 `(?:docs|scripts|crates|assets|\.github|\.cargo|\.githooks)/[A-Za-z0-9_\-./]+` 不命中无文件名的裸目录提法）。
- **定稿提交先于第一行施工代码**（纪律③）。

### SD-1：11 个脚本头注补全（还清 P1~P7 的旧债）

- 逐脚本按 §3 骨架 + **附录 A 真值底稿**重写头注（底稿已含每脚本的参数/退出码/缺口的代码级真值，照抄即可，不再考古）。纯注释改动，零行为变更。
- 提交粒度：按脚本分批或一个提交包 11 文件均可，**pathspec 逐文件点名**（G-19：禁 `git add -A`）。
- **验证矩阵（不是所有脚本都能随便跑——release 链有真发布动作）**：

| 脚本 | 施工后怎么验证 |
|---|---|
| check_personal_paths / check_deps / check_guards / check_dead_contract / check_agents_health | 直接跑（秒级，无副作用） |
| precommit.ps1 | 直接跑（全套门禁，本来就必须过） |
| fetch_sherpa_libs.ps1 | 直接跑（幂等，库已在 = 秒退） |
| package_release.ps1 | 需 release exe 先构建；若本机没有，核过头注即可，不强求真跑（真跑也只是打包，无发布动作，无害） |
| **release.ps1** | **只跑只读动词 `notes`**；`rehearse` 可选（= check+build+pack，不碰 Release）。**禁跑 draft / promote / release**。`check` 动词要求工作区干净，施工期间必红，属预期 |
| grab_reference_ui.py / silero_reference.py | 头注/docstring 改动不涉代码，跳过实跑（要外部 venv / onnxruntime 环境），人工核对 docstring 与代码事实一致（.py 不在守护扫描面，SD-3 前唯一防线是人工） |

- **BOM 机械验证**（改过 .ps1 就验，不信编辑器）：`pwsh -c "Get-Content -AsByteStream -TotalCount 3 scripts/<脚本名>.ps1"` 应输出 `239 187 191`。丢了就补。（若 text-hygiene 方案先行拍板落地，此项按其新政策执行。）
- ps1 头注里的新路径/G-编号引用写完即被断言 5/6 检查（§3 末条）——precommit 红了先查引用是否照抄准确。
- 完工标准：§3 两条合格标准逐脚本过一遍。

### SD-2：新建 docs 顶层 scripts.md

- 依赖 SD-1（参数概览要先有准头）。按 §4 规范写 11 节（节标题 = 精确路径）+ 页首声明；参数概览逐节清单照抄**附录 B**。
- 同时改三处（挪卡连带改索引，纪律⑤）：
  1. AGENTS §2 末尾**加一行 bullet**（与既有 bullet 同构，不进代码块）：脚本目录页 = docs 顶层 scripts.md（一脚本一节；细节以头注为准）；
  2. **AGENTS §2 存量收敛**：fetch_sherpa_libs 行删「-Mirror <前缀>/SHERPA_ONNX_MIRROR 走 Release 镜像」语义句（头注已全有）、release 行删「九动词」计数改「发布链引擎（动词详见脚本头注）」——消除体系外第二真源（§7 末行、§8 末行）；
  3. AGENTS §4 路由表加一行：查脚本用法 / 新增脚本 → docs 顶层 scripts.md + 各脚本头注（唯一细节真源）。

### SD-3：守护断言 8

- 依赖 SD-2（册子存在才能对账）+ §4 节标题格式（抽取锚点）。
- 实现 = **附录 C 伪代码**（插断言 7 之后、汇总输出之前；`$violations +=` 文案风格照抄既有断言）。
- **连带改动清单（漏一项就是文档漂移）**：
  1. check_agents_health.ps1 头注：「断言六组（编号保留 2~7…）」→ 七组（2~8），写明 8 号复用注记（§6）；
  2. 同文件末行 OK 消息加新检查项名；
  3. AGENTS.md 头注「防回胀靠……守护断言 2~7」→「2~8」（触发改文纪律：commit message 说明信息去向）；
  4. precommit.ps1 头注第 6 项注记「ADR-15：两档预算/路径/G-编号/看板」补「/引用网扩面/归档/脚本登记」（D-93 起已陈旧，顺手还清）；
  5. 断言 5 扫描面 `scripts` 过滤器 `*.ps1` → `*.ps1|*.py`（补 .py 头注无校验的洞，§3 末条；存量 .py 引用已干跑预检零误报）。
- 三步验证：干跑 / fresh clone（G-29）/ 故意删一条目录页节验证变红。
- docs/README.md「长期文档」表加 docs 顶层 scripts.md 一行（完结条件：随项目终结）。

### SD-4：收口（本方案文档的归宿，纪律③一气呵成）

- 三步全完工 → 本文档 git mv 至 docs/archive/ 同名档，头注前 8 行加归档注记（断言 7(b) 会查）。
- docs/README.md：归档表加顶行（完工日期倒序）+ **删活跃文档表该行**（SD-0 加的，两端都要动——该表无守护，靠本清单）。
- decisions.md：ADR-19 行**落档路径更新**为归档路径（decisions.md 头注明文要求）。
- AGENTS §8「施工中」行清零。
- 收口三问过一遍（纪律⑤）：`ls docs/` 健康吗？decisions.md 漏行吗？表和文件对上没？

### 全程通用门禁

- precommit 七项全绿；cargo test --workspace 全绿（纯注释/文档/守护改动，不应有波动，但门禁照跑——纪律④）。
- 涉守护改动（SD-3）必须 fresh clone 复跑（G-29）。

---

## 10. 验收基线（整个体系立起来的判据）

1. 11/11 脚本头注符合 §3 骨架；抽查法 = 只看头注回答「怎么调 / 参数默认值多少 / 失败什么表现」，答不对即不合格。
2. 9/9 改过的 .ps1 BOM 实存（机械验证；全仓只有 9 个 .ps1——v1.1 误写 11/11，v1.2 修正）；.py 侧人工核对记录在案。
3. docs 顶层 scripts.md **11 节齐全（含 check_agents_health 自身那一节）**，节标题全部 = 精确路径，每节含「以头注为准」句。
4. 断言 8 三步验证全过（干跑绿 / fresh clone 绿 / 造漏登记红）。
5. AGENTS §2（含存量收敛）、§4、docs/README.md（长期文档表 + 活跃文档表生命周期完整）、decisions.md（ADR-19 + 落档路径）同步更新完毕；方案文档按 SD-4 归档。
6. 此后新建任何脚本，流程 = §5 规矩①，漏登记被守护拦——制度自动运转，不再依赖本方案文档本身。

---

## 11. 明确不做（本方案边界）

- 不给「头注与代码内容同步」上机械守护（§7 已述）。
- 不迁 Get-Help、不做生成器、不加断言豁免表（§8 已述）。
- 不给 .py 脚本引入 argparse「为了参数化而参数化」——grab_reference_ui.py 无参是现状合理（唯一 `sys.argv` 是 QApplication 惯用传参，非 CLI 解析）。
- 本方案只管 `scripts/` 目录；`.github/workflows/*.yml` 头注已被断言 5 扫描；`.githooks/pre-commit` 不在断言 5 面内（§6 附注），现状接受，头注格式可日后向 §3 骨架看齐，不强求。
- 目录页正文（节标题以外的行）不进机械守护面——守护只认节标题路径；正文准确性靠规矩②与评审自查。
- AGENTS §8 看板「草稿期漏登记待拍板行」的病根本身不在本包修（那是 AGENTS 看板纪律的守护化，另一个话题）；本包只做 SD-0 的补课登记。

---

## 12. 待拍板清单（逐条点头/改选）

| # | 问题 | 推荐 | 备选与代价 |
|---|---|---|---|
| 1 | 方案整体是否批准（v1.2 含三路评审修订） | 批准，按定稿→SD-1→2→3→4 开工 | — |
| 2 | 目录页位置 | `docs 顶层 scripts.md`（守护零改动） | `scripts 目录 README`：GitHub 浏览体验更好，代价 = 断言 5 扩一行扫描面；两者都做 = docs 为主、scripts 放一行指向 |
| 3 | 断言 8 是否上 | 上（堵 P9 的唯一自动手段） | 不上：新增脚本漏登记无拦截，靠自觉 |
| 4 | 残余风险接受与否 | 接受「改脚本忘改头注」这条唯一自觉项（评审自查兜底） | 不接受则无便宜方案，只能加重复守护（§7 已论证不划算） |
| 5 | AGENTS §2 存量收敛是否随 SD-2 做 | 做（删参数级语义句与动词计数，收敛为最小调用行+指针） | 不做：§2 保留第二真源残留，动词增删时静默变错 |

---

## 13. 修订记录

### v1.0 → v1.1（2026-09-19 一审，自审 8 条）

对抗式自查发现 8 条全部织回：验证矩阵（release 禁跑写动词）/ 节标题格式钉死 / 头注新引用入扫描面 / 方案自身生命周期（SD-0/SD-4）/ 改名双红与一次性脚本边界 / BOM 机械验证 / 决策号与看板时序 / CI 兜底。详见上一版记录，此处存目。

### v1.1 → v1.2（2026-09-19 二审，三子代理并行：对抗评审 / 事实核查 / 施工预演）

| # | 发现 | 来源 | 严重度 | 去向 |
|---|---|---|---|---|
| 1 | **前向死引用**：定稿移入 docs/ 顶层后，正文提 docs 顶层 scripts.md（SD-2 才建）与归档路径（SD-4 才存在）→ 定稿提交必被断言 5 拦红 | 施工预演 | 高 | §9 SD-0 前向引用改写规则 |
| 2 | **P1 事实错误**：$RepoRoot 实为 6 个非 8 个；check_personal_paths 无 param 块（三代理独立交叉证伪） | 三路一致 | 高 | §1.2 P1 改写 + 附录 A |
| 3 | **SD-0 看板行无源可挪**：AGENTS §8 待拍板板块无本方案条目（草稿期漏登记——P9 病现形） | 评审+预演 | 高 | SD-0 改「施工中新建行 + 补课」 |
| 4 | **「自身豁免」条款自相矛盾**：对账断言无自触发，豁免是范畴错误；正解 = 11 节全强制零豁免代码 | 评审+预演 | 高 | §6 删豁免条款并注明成因 |
| 5 | §10.2「11/11 改过的 .ps1 BOM」错误：全仓只有 9 个 .ps1 | 评审 | 中 | §10 改 9/9 |
| 6 | P2/P4 收窄：promote CHANGELOG 闸与 WARN 分级头注已写，真缺口只有 draft←pack、演练动词点名、退出码行 | 核查 | 中 | §1.2 P2/P4 改写 |
| 7 | 断言 8 编号与史档撞号（D-93 时代 8 = 水位线，已钉死在 decisions/归档表） | 评审+预演 | 中 | §6 复用注记 + SD-3 连带 1 |
| 8 | SD-3 连带改动漏列（check_agents_health 头注 2~7→2~8、OK 消息、AGENTS.md:6、precommit 头注第 6 项注记） | 评审+预演 | 中 | SD-3 连带清单 5 项 |
| 9 | 活跃文档表生命周期两端漏（SD-0 未列加行 / SD-4 未列删行；该表无守护） | 评审+预演 | 中 | SD-0/SD-4 补 |
| 10 | AGENTS §2 存量参数级细节与总纲矛盾（-Mirror 语义句、「九动词」计数无守护面） | 评审 | 中 | §0 精确化 + SD-2 收敛 + 拍板项 #5 |
| 11 | §11「pre-commit 被断言 5 扫描」错误（不在扫描面） | 核查 | 中 | §11/§6 改写 |
| 12 | §1.1 precommit 自我声明归属错位（声明在 ci.yml/AGENTS §2，不在其自身头注） | 核查 | 低 | §1.1 改写 |
| 13 | 断言 8 实现四细节未钉死（非递归枚举/抽取正则与终止锚/册缺失行为/文案风格） | 评审+预演 | 低 | §4/§6 + 附录 C |
| 14 | .py 头注不在断言 5/6 面内未申报；顺手扩 `*.py` 补洞 | 评审+预演 | 低 | §3 末条 + SD-3 连带 5 |
| 15 | release.ps1 块注释形式 / AGENTS §2 加行位置 / decisions 插入位置与 D-95 节内错位 / .py 无参佐证 | 预演 | 低 | §3/SD-0/SD-2/§11 各补 |

**判对项（三路攻击未动）**：三层分工与参数名/默认值切分、断言 8 双向对账 fail-closed、docs/ 位置论据（行号逐字核实）、触发面零改动、SD-1 验证矩阵、三条否决（Get-Help/生成器/同步守护）、改名双红与无豁免边界。评审代理原话：「这已是同类方案里少见的不肥」。

---

## 附录 A：11 脚本用法卡真值底稿（SD-1 施工直接照抄；真值取自代码，v1.2 三路核查）

> 每条格式：【参数】【退出码】【SD-1 要补的】。共性前置（pwsh 7）不重复写。

1. **check_personal_paths.ps1**：无 param（`$PSScriptRoot` 自定位仓库根，头注注明）。退出 0 = 过；1 = 命中（Tier1 用户名/实名 Users 路径；Tier2 白名单外盘符）。docs/archive/ 内 Tier2 仅 WARN 计数**不拦**。扫描面 = `git ls-files` 已跟踪文件 + 二进制后缀排除。只读无副作用。详见已有（path-hygiene PH-5）。
2. **check_deps.ps1**：`-RepoRoot`（默认脚本上级，一般不用传）。退出 0 = 过；1 = 违规边逐条 VIOLATION。补一句「$Whitelist/$DevExtra 白名单真源 = 本脚本」（AGENTS §3 已声明、头注未自认）。只读。
3. **check_guards.ps1**：`-RepoRoot`。**补退出码行**：0 = 过；1 = 违例逐条 VIOLATION。**补五组禁令全名**（spawn/proxy/字符串协议/panic hook/契约旁路——第五组现只在末行 OK 消息）。前置：crates/ 源码树（白名单含 tests/ 夹具）。只读。
4. **check_dead_contract.ps1**：`-RepoRoot`。**补退出码行**：0 = 通过（**WARN 算过**——半死变体仅提示人工定性）；1 = 死变体或契约文件缺失。判定分级 0/1/≥2 头注已有。前置：lt-proto 三个契约文件在位。$ReservedWhitelist 与 ADR-13 同步的维护点提一行。只读。
5. **check_agents_health.ps1**：`-RepoRoot`。退出 0 = 过（可带 WARN）；1 = 违规。前置：AGENTS/gotchas/docs-README/archive 在位。只读。（SD-3 将连带改其头注断言数与 OK 消息。）
6. **fetch_sherpa_libs.ps1**：`-Mirror`（默认 = `$env:SHERPA_ONNX_MIRROR`，皆无 = 直连）。退出 0 = 就位（秒退或新解包成功）；throw = 失败（非 0）。前置：curl + **GNU tar**（自动找 Git for Windows 的 usr\bin\tar；bsdtar 读不了 bz2 会被跳过）；版本从 Cargo.lock 解析。**强制重下真值**：删 `.cache/sherpa-onnx/` 下归档档 = 重下+重解；只删 `extracted/` = 只重解**不**重下；只删归档不删 extracted = 秒退无效。产物：.cache/sherpa-onnx/（档 ~120MB + 解包 ~1.1GB，已 gitignore）。用法行统一 `pwsh -File` 前缀。详见可加 G-25。
7. **package_release.ps1**：`-RepoRoot`。前置：`cargo build --release -p lt-app`（exe 缺 = exit 1）；版本 = 仓根 Cargo.toml 首个 `^version` 行。退出 0 = 完成；1 = 前置缺。**残留/覆盖真值**：staging 目录 `dist/LiveTranslate-<ver>/` 打包后**保留**；zip 先删旧再建新；README.txt/LICENSE/NOTICES.md/OFL.txt「存在才拷入、缺件不失败」——五件套强制是 release.ps1 pack ② 的职责（写明分工）。副作用：只写 dist/。
8. **precommit.ps1**：`-RepoRoot`。**补退出码行**：0 = 七项全过；非 0 = **透传首个失败项自身退出码**（通常 1；clippy 编译错可 101）；任一失败即停。前置：rust-toolchain 工具链 + .venv（libclang/cmake）+ sherpa 库（clippy 需编译）。副作用：仅 target/ 缓存增长。
9. **release.ps1**：`$Verb`（位置 0，ValidateSet 九值——**省略动词 = 打印用法 exit 2**）；`-NotesFile`（仅 promote）；`-Rehearsal`（开关：**拒 draft/promote/release 三动词**，跳过 check 的主线/ci 绿两道闸）。退出：0 = 成功；1 = Die；2 = 未给动词。**依赖链真值**：draft ← pack 三件齐（zip+sha256+build-info）且远端 tag 指向本地 HEAD；promote ← 草稿存在 + CHANGELOG 双语本版段落（或 -NotesFile）；verify ← 远端已有草稿/发布；rehearse = 强制演练的 check+build+pack；notes 只读。前置：gh 已装已登录（写动词硬需；本机 check 无 gh 降级跳过）。副作用：dist/ 重打包；draft 建/刷新草稿 + 上传；promote 转正（唯一不可逆）；verify 下载比对后清理。CI workflow_dispatch 自动进演练。
10. **grab_reference_ui.py**：无参（docstring 明写）。依赖：PyQt6 + PyYAML + psutil + openai。前置：外部 venv（路径不入库）+ 工作区 `LiveTranslate/` 副本在位（缺 = sys.exit）。退出 0 = 完成；非 0 = sys.exit（副本缺/保存失败/结构变更）。产物：assets/reference/ 20 张 PNG（zh/en 各 10），同名覆盖，目录自动建；不读 user_settings、不写回副本。详见 G-18（**.py 引用 SD-3 前无守护，人工核对**）。
11. **silero_reference.py**：无参。**补前置**：numpy + onnxruntime；`assets/silero_vad.onnx`；`target/vad_input.f32` 与 `target/vad_conf_rust.json`（由 `cargo run -p lt-audio --example vad_check` 产出，example 实存）。退出 0 = PASS（max_abs_err < 1e-4）；1 = FAIL（附最大 5 chunk 明细）。只读无副作用。

## 附录 B：目录页 11 节「参数概览」清单（SD-2 照抄；只列名不抄默认值）

| 节标题（钉死格式） | 参数概览 |
|---|---|
| `scripts/check_personal_paths.ps1` | 无参数 |
| `scripts/check_deps.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/check_guards.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/check_dead_contract.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/check_agents_health.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/fetch_sherpa_libs.ps1` | `-Mirror`（另有环境变量 `SHERPA_ONNX_MIRROR`） |
| `scripts/package_release.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/precommit.ps1` | `-RepoRoot`（一般不用传） |
| `scripts/release.ps1` | 动词九选一：`check` `build` `pack` `draft` `notes` `verify` `rehearse` `release` `promote`；参数 `-NotesFile`（仅 promote）、`-Rehearsal` |
| `scripts/grab_reference_ui.py` | 无参数 |
| `scripts/silero_reference.py` | 无参数 |

## 附录 C：断言 8 参考实现（SD-3；风格对齐既有断言，插断言 7 之后）

```powershell
# ── 断言 8：脚本登记面（scripts-doc-system SD-3；8 号曾为水位线断言，ADR-18 退役后复用）──
# (a) 磁盘 → 册：scripts/ 下每个 .ps1/.py 须在目录页有节（### `scripts/<名>`）
# (b) 册 → 磁盘：册上抽出的每个 scripts/… 路径须实存（孤儿条目）
# 11 节全强制含本守护自身，无豁免代码（对账断言不存在内容自触发；断言 5 的整文件
# 自豁免成因 = 豁免表路径字面量，在此范畴不适用）。枚举平铺非递归——出现子目录须回改。
$scriptsMd = Join-Path $RepoRoot ('docs' + '/scripts.md')   # 拼接书写防自命中
if (-not (Test-Path -LiteralPath $scriptsMd)) {
    $violations += '脚本目录页（docs 顶层 scripts.md）不存在'
} else {
    $book = [System.IO.File]::ReadAllText($scriptsMd)
    $bookPaths = [regex]::Matches($book, '(?m)^### `(scripts/[^`]+)`\s*$') |
                 ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique
    $diskScripts = Get-ChildItem -Path (Join-Path $RepoRoot 'scripts') -File |
                   Where-Object { $_.Extension -in '.ps1', '.py' } |
                   ForEach-Object { 'scripts/' + $_.Name } | Sort-Object -Unique
    $unregistered = @($diskScripts | Where-Object { $bookPaths -notcontains $_ })
    $orphans      = @($bookPaths   | Where-Object { $diskScripts -notcontains $_ })
    if ($unregistered.Count -gt 0) { $violations += "脚本未登记：$($unregistered -join ', ')（目录页缺对应节）" }
    if ($orphans.Count -gt 0)      { $violations += "目录页孤儿条目：$($orphans -join ', ')（册上有节、磁盘无脚本）" }
}
```
