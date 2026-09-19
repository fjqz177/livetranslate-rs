# 文本卫生方案：全仓强制「无 BOM 合法 UTF-8 + LF」

> **状态**：完工已归档（2026-09-19 当日定稿当日完工；ADR-20。TH-1~TH-6 全步执行，验收基线全过见 §6）
> **日期**：2026-09-19（v1.1 同日三子代理评审修订：对抗评审 / 事实核查 / 施工预演，发现汇总见 §12）
> **触发**：用户拍板两件事：① 全仓统一 LF；② 全仓强制 UTF-8 且「不为老的史山代码留兼容」——推翻 G-23 的 BOM 保守对策
> **局部前缀**：TH-（定稿时按 docs/README.md 纪律⑥校验全局唯一，含 archive）
> **关联**：docs/archive/scripts-doc-system.md（ADR-19，**已完工归档**——本方案 TH-5 头注按其用法卡骨架写，并履行其 §5 规矩②的目录页连带；其 :139/:187 已预写「若 text-hygiene 先落地按其新政策执行」的让位注记，双向自洽）

---

## 0. 一句话总纲

**全仓文本文件 = 无 BOM 的合法 UTF-8 + LF，入库前机器强制，人不背这个记性。**

---

## 1. 背景：两族编码问题的历史与现状

### 1.1 两族问题（历史依据，细节见 docs/gotchas.md G-23 与各脚本头注）

**BOM 族**：2026-09-13 实锤（G-23）——无 BOM 的 UTF-8 .ps1 被 Windows PowerShell 5.1 按系统 ANSI（中文系统 = GBK）解码，中文乱码（「预算」→「棰勭畻」）且解析失败。当年对策 = 入库 .ps1 一律带 BOM（文件头三个隐形字节 `EF BB BF`）。次日（09-14）全仓切 pwsh 7（ADR-16）：**pwsh 对无 BOM 文件默认按 UTF-8 读，BOM 的保护对象从此只剩「误用 powershell.exe」**。BOM 依赖面全仓清点（三路核查实证）：无签名、无 Rust 侧引用 .ps1、无守护断言 BOM 存在性、调用路径全显式 pwsh——**零机械依赖**；README 的 settings.json 写入警示与 .ps1 政策无关。

**换行族**：Windows 换行 = `\r\n` 两字节，其余世界 = `\n`；git autocrlf 在 Windows 机器常开。本仓已有三道防御：release.ps1 行尾归一（「LF / CRLF 都吃」）、.githooks/pre-commit 文件名尾部容忍 `\r`（fail-safe 方向）、check_agents_health 行计数按 `\n` 等价 wc -l。仓库侧 .gitattributes 已有一行 `.githooks/* text eol=lf`（钩子是 sh 脚本，CRLF 直接炸门禁，故唯一硬钉）。**必要性现成实证（预演实测）**：`git clone -c core.autocrlf=true . <tmp>` 模拟「只装系统层 autocrlf=true」的别人机器——198 个文本检出即 `w/crlf`，唯 .githooks/pre-commit 因 attribute 行幸存 `w/lf`。

**本机 autocrlf 现状**：system `true`（Git 安装器）/ global `input`（用户，生效）。`input` 只管本机——别人的 clone 你管不着，仓库级防线必须立。

### 1.2 现状实测（2026-09-19 三路核查复测；数字以施工日实测为准）

- 283 个已跟踪文件 = **199 文本 + 84 二进制**（v1.0 写 281/197，ADR-19 完工新增 docs/scripts.md 等 +2；结论不受影响）。84 二进制 = png 57 / bin 20（crates/lt-audio/tests/fixtures/）/ br 3 / onnx 1 / ico 1 / icns 1 / dll 1。
- **BOM：恰好 9 个 `.ps1`，其余全部干净**（ADR-19 头注改动未动 BOM）。
- **行尾：199 文本 index 与工作区全 LF，零 CRLF、零混合**。
- **编码：零非法 UTF-8 文本**（严格转换实测 0 失败）。
- **存量发现（v1.1 新增）**：仓库根 **.editorconfig 已存在**（dev-config-audit C6，2026-09-17 入库），其中 `[*.ps1] charset = utf-8-bom` 是旧 BOM 政策唯一被编辑器机器执行的载体，注释明言「换行符不钉死交给 autocrlf」——与本方案两处直接冲突，TH-1/TH-3/TH-4 连锁处理。

**结论：没有存量要洗，全部工作量 = 剥 9 个 BOM + 翻转政策成文 + 锁死未来。**

---

## 2. 目标态（三条硬规则）

1. 所有文本文件是**合法 UTF-8 字节流**（谁拿 GBK 存中文混进来 → 拒收）；
2. 所有文本文件**无 BOM**——含 `.ps1`，政策翻转，不留豁免；
3. 所有文本文件在 **git 记录（index）里全 LF**；工作区由 `.gitattributes eol=lf` 保证检出即 LF。

二进制文件不适用以上任何一条（`text=auto` 按 NUL 字节自动识别为二进制并跳过；84 个现有二进制全部识别正确，git 的判据与守护用的是同一启发式，实证一致）。

---

## 3. 设计依据（每条为什么这么定）

| 设计 | 依据 |
|---|---|
| 敢剥 .ps1 的 BOM | ADR-16 pwsh-only + 零机械依赖清点（§1.1）；误用 powershell.exe = 当场乱码+语法炸，**可见快速失败**，符合本仓「报错必须显眼」哲学 |
| 强制落守护而非 git | **git 对 BOM 和字符编码零原生能力**——gitattributes/autocrlf/safecrlf 只管行尾。强制唯一落点 = 守护断言 + precommit + CI 同源（D-88 单一真源） |
| 行尾只判 index 侧 | 工作区状态受各 clone 机检出配置影响，index 才是仓库真值；工作区侧由 `.gitattributes eol=lf` 保证（随仓走、压过任何人的 autocrlf） |
| `* text=auto` 而非 `* text` | 不带 auto 会把二进制当文本洗行尾，洗坏文件；auto 按 NUL 启发式跳过二进制（84 个实证识别正确） |
| 守护并进 check_personal_paths | 同款扫描面（`git ls-files` + 二进制后缀排除），高内聚；新开脚本 = 清单逻辑抄两份。排除清单唯一漏网后缀 = `.bin`（20 个 fixtures，干跑实证），补上即闭环 |
| 严格解码而非 Get-Content | 实测：pwsh 7 下 `Get-Content -Encoding UTF8` 对非法字节**静默替换 U+FFFD 不报错**；`UTF8Encoding($false,$true)` 严格解码才 throw——现有守护对「合法 UTF-8」零校验，TH-5 是真增强 |
| .editorconfig `charset = utf-8` | 该值即**无 BOM**（带 BOM 是另一个值 `utf-8-bom`）；管编辑器新建/保存，与守护（管入库）分层 |
| fresh clone 验证 | G-29：本地绿 ≠ CI 绿。本轮补强：另做 `clone -c core.autocrlf=true` 强实证，模拟别人的机器 |
| TH-2 字节手术而非文本往返 | `Get-Content`/`Set-Content` 是字符串层操作：Windows pwsh 默认 CRLF 写回 + 行数组重组丢尾行状态——文本往返会把 9 文件全量翻成 CRLF。字节切片（ReadAllBytes → 去头 3 → WriteAllBytes）与内容零纠缠，实测 cmp 恰 3 字节差异 |

---

## 4. 被否掉的备选（留证防重提）

| 备选 | 为什么不选 |
|---|---|
| 维持现状（.ps1 带 BOM 双读兼容，G-23 原对策） | 守护永远带「.ps1 豁免」例外；.editorconfig `[*.ps1] utf-8-bom` 继续每次保存回加 BOM。用户明示不为史山留兼容——**本方案核心拍板** |
| 只强制 .ps1、其他文本不管 | 反向不彻底，GBK 的 md/yaml 混进来没人拦 |
| `.gitattributes` 管编码/BOM | git 无此能力（§3），写了也是自欺 |
| 守护里用正则猜非法 UTF-8 字节 | 严格解码是标准做法，正则猜字节必漏 |
| 逐个给二进制后缀显式 `binary` 标注（常设） | 84 个实证全对，逐个列 = 每添二进制都要记得维护。**注意与 §7 修复路径的关系**：事故驱动时给具体后缀加 `*.xxx -text` 覆盖是正确断根手段（见 §7），本条否决的是「预先给全部后缀常设标注」——两者不矛盾 |
| 工作区行尾也进守护断言 | w 侧不归仓管（各 clone 机检出配置不同），判了出环境性误报；w 侧由 eol=lf 保证 |

---

## 5. 施工卡（拍板后按此执行；当日完工流程 = TH-1 定稿归档 → TH-2 → TH-3 → TH-4 → TH-5 → TH-6，提交切分见 §6）

### TH-1：政策翻转 + 方案定稿归档（纪律③：定稿先于第一行施工代码）

- 本草稿 git mv 至 **docs/archive/text-hygiene.md**（当日完工流程：定稿即归档，scripts-doc-system 先例），头注改「完工已归档」（前 8 行含注记，断言 7(b) 合规）。定稿改稿时按 scripts-doc-system SD-0 先例处理前向引用。
- decisions.md 发 **ADR-20**（表尾已核实 = ADR-19；对标 ADR-16 pwsh-only 先例，本政策同为工程约定）。
- **BOM 政策引用全清单**（三路 grep 交叉核实；必改 5 处 + 不改 3 类）：

| # | 位置 | 处置 |
|---|---|---|
| 1 | docs/gotchas.md G-23 **对策段** | 翻转：全仓文本一律无 BOM 合法 UTF-8（.ps1 不再豁免，既有 9 个已剥）；误用 powershell.exe = 预期快速失败；入库强制 = check_personal_paths 文本卫生三查（precommit 第 2 项）；顺手修「四守护脚本」计数失真（实 9 个） |
| 2 | docs/gotchas.md G-23 **注记段**（「BOM 保留零 churn 不剥」） | 随对策一并翻转（v1.0 漏估）——无 BOM 是常态，带 BOM 反被守护拒收 |
| 3 | AGENTS §7 G-23 速查行（「一律带 UTF-8 BOM」「BOM 保留双读兼容」两词） | 翻转 + 指向三查守护（改文纪律：commit message 说明信息去向） |
| 4 | AGENTS §4 路由表「写 / 改守护脚本」行括注「G-23（.ps1 带 BOM）」 | 改「G-23（文本卫生：无 BOM/合法 UTF-8/LF）」（v1.0 漏列） |
| 5 | **.editorconfig `[*.ps1] charset = utf-8-bom` 行** | 随 TH-4 删除（旧政策唯一编辑器机器载体，不删 = 每次保存回加 BOM 与守护永久打架） |
| — | README 的 settings.json 写入警示 / release.ps1「no-BOM UTF-8+LF」产物描述 / docs/scripts.md / 11 脚本头注 / docs/archive/* | **不改**（前三者语义仍真或恰同向；史档冻结，ADR-19 档已自带让位注记） |

### TH-2：剥 9 个 .ps1 的 BOM（字节手术，禁文本往返）

```bash
pwsh -NoProfile -Command "Get-ChildItem scripts -Filter *.ps1 | ForEach-Object { $b=[IO.File]::ReadAllBytes($_.FullName); if ($b.Length -ge 3 -and $b[0] -eq 239 -and $b[1] -eq 187 -and $b[2] -eq 191) { [IO.File]::WriteAllBytes($_.FullName, $b[3..($b.Length-1)]); Write-Output ('stripped: ' + $_.Name) } }"
```

验证四连（预演已实测）：
1. 头 3 字节复查：9 行均不得再出现 `239 187 191`；
2. `git diff --numstat`：恰 9 行且每行 `1  1`（只动首行），多一行即停；
3. 字节级证明：`git cat-file blob HEAD:scripts/<名>.ps1` 与现文件 `cmp`，证明「新文件 == 原文件去头 3 字节」；
4. 解析零错：`[System.Management.Automation.Language.Parser]::ParseFile` 九文件零错误输出。
- 收尾跑 `pwsh -File scripts/precommit.ps1` 全套——钩子暂存 scripts/* 触发的正是刚剥完 BOM 的守护本体，**提交 2 本身即真实验证**。

### TH-3：`.gitattributes` 锁行尾（时序硬约束：TH-2 必须先提交）

- 追加 `* text=auto eol=lf`（保留既有 .githooks 行）。
- **时序硬约束**：`git add --renormalize .` = 对全部 tracked 文件按当前 filters 重走 clean 后重新暂存——**会把工作区一切未提交改动搬进暂存区**。只能在「除 .gitattributes 外干净」的树上跑。
- 零变化实证：renormalize 前后 `git status --porcelain` 均只剩 .gitattributes；`git ls-files --eol | cut -f1 | sort | uniq -c` 期望 199 `i/lf w/lf` + 84 `i/-text`（**禁写「≠ i/lf 即红」式断言**——二进制是 `i/-text`，写错全红 84 个）。
- 多一个文件变化即停下查，不许盲提交。

### TH-4：改造**既有** .editorconfig（v1.0 误写「新建」——文件已存在，dev-config-audit C6 成果）

- `[*]` 段**加** `end_of_line = lf`；**整段删除** `[*.ps1]`（utf-8-bom，见 TH-1 #5）；「换行符不钉死交给 autocrlf」注释改写为「换行已由 .gitattributes `* text=auto eol=lf` 全仓钉死；此处管编辑器新建/保存，与守护分层」。
- **C6 既有承诺去留 = 小拍板（§10 #4）**：`insert_final_newline` / `trim_trailing_whitespace`（含 `[*.md] trim=false` 的硬换行保护）守护不查——推荐**原样保留**（删除是额外 churn 且伤 md 硬换行；editorconfig 管编辑器层本不以守护为界，§3 分层论据自洽）。
- 绝不允许整文件覆写：C6 的缩进规则与 md 保护是既成成果。

### TH-5：守护三查（「强制」本体；落点 check_personal_paths.ps1）

- 排除清单补 `.bin`（现清单唯一漏网二进制后缀，20 个 fixtures，干跑实证恰够）：
  `$exclude = '\.(png|jpg|ico|icns|onnx|dll|br|bin|ttf|otf|woff2?|zip|gz)$'`
- 三查插在文件循环结构内（共用现有 FAIL 输出格式 `FAIL <file>: <原因>`，文件级检查无行号）：

```powershell
$strict = [System.Text.UTF8Encoding]::new($false, $true)   # throwOnInvalidBytes
# eolMap：git ls-files --eol 按 TAB 分段取「元数据列→路径」，文件名含空格也安全
foreach ($f in $files) {
  $b = [System.IO.File]::ReadAllBytes((Join-Path (Get-Location).Path $f))  # .NET API 走进程 cwd，须显式拼仓库根
  # 查一：头 3 字节 = EF BB BF → FAIL（BOM）
  # 查二：try { [void]$strict.GetString($b) } catch [System.Text.DecoderFallbackException] { FAIL（非法 UTF-8） }
  #       ——必须 try/catch 转 FAIL 行（$ErrorActionPreference='Stop' 下裸 throw 是未格式化终止）
  # 查三：eolMap[$f] 为 i/crlf 或 i/mixed → FAIL（index 行尾）
  #       ——禁写「≠ i/lf 即红」：二进制是 i/-text，写错全红 84 个
}
```

- 空文件三查天然全过（头字节不足 3、解码空串合法、无行尾），明写一行注释即可。
- 头注按 **ADR-19 用法卡骨架**重写（三查职责入卡），汇总 FAIL 行与「详见」补 docs/archive/text-hygiene.md。
- **同提交连带**：docs/scripts.md 的 check_personal_paths 小节一句话扩写（「个人路径卫生」→「个人路径 + 文本卫生三查」，ADR-19 §5 规矩②）；小节其余四件套格式不动。
- **禁在脚本里写 docs/drafts/ 引用**（断言 5：scripts→drafts = 违规）；指 archive 路径安全（TH-1 已归档在先）。

### TH-6：验证闭环（命令形态预演已实测）

1. `pwsh -File scripts/precommit.ps1` 全套 + `cargo test --workspace`；
2. `pwsh -File scripts/release.ps1 notes`（CHANGELOG 解析正常）；
3. G-29 fresh clone：clone 后跑 check_personal_paths（三查绿）+ `git ls-files --eol | cut -f1 | sort | uniq -c` 全 `i/lf w/lf`；
4. **强实证（本轮新增）**：`git clone -c core.autocrlf=true . <tmp>` 模拟别人的机器——检出后应全 `w/lf`（改造前实测 198 个 `w/crlf`，对照鲜明）；
5. 收口三问过一遍（纪律⑤）。TH-6 无文件改动，结果写入归档文档验收基线。

### 提交切分（4 个提交，顺序即依赖）

| # | 提交 | 文件 | 要点 |
|---|---|---|---|
| 1 | 定稿归档 + G-23 翻转（ADR-20） | docs/archive/text-hygiene.md、docs/README.md、docs/decisions.md、docs/gotchas.md、AGENTS.md（§4+§7）、.editorconfig（删 BOM 行——与 TH-4 改造拆开亦可，推荐随 TH-1 一步翻转到位） | 纪律③先于施工；AGENTS 触发改文纪律 |
| 2 | 剥 9 个 BOM | scripts/*.ps1 ×9 | numstat 自查 9×(1,1)；钩子跑的即无 BOM 守护=真实验证 |
| 3 | .gitattributes + .editorconfig 改造 | 两文件 | renormalize 零变化实证后提交；**两文件不在钩子触发面**——按 G-28 精神自觉手动跑 precommit（触发面扩否见 §10 #2） |
| 4 | 守护三查 + 目录页小节 | scripts/check_personal_paths.ps1、docs/scripts.md | 新代码自身必须无 BOM（被自己拦）；对已剥 BOM 的 9 文件应绿 |

---

## 6. 验收基线

1. 全量扫描：文本文件零 BOM、零非法 UTF-8、index 零 CRLF/mixed（三条都有机器断言）。
2. 9/9 .ps1 剥 BOM 且 precommit 全套绿（pwsh 7 读无 BOM 中文脚本无恙的实证）；`git diff --numstat` 型字节手术证明（每文件恰 1 行改动）。
3. `git add --renormalize .` 零变化实证；`clone -c core.autocrlf=true` 强实证检出全 `w/lf`。
4. fresh clone 复跑守护绿（G-29）。
5. G-23 两段翻转、AGENTS §4/§7 翻转、.editorconfig BOM 行已删、decisions ADR-20 已登记、方案已归档、docs/scripts.md 小节已同步。
6. 此后任何 BOM/GBK/CRLF 入库尝试 = precommit 红（本地触发面内）或 CI 红（兜底）——制度自动运转。

---

## 7. 风险账本（诚实账）

| 风险 | 评估 |
|---|---|
| 有人误用 powershell.exe 跑脚本 | 当场乱码+语法炸，可见快速失败；仓内零调用路径。**接受** |
| 守护拦不住本地未提交的草稿文件 | 分层：守护管入库，editorconfig 管编辑器。**接受** |
| `text=auto` 误判新二进制为文本 | **两层修复，勿混**：① 守护排除清单加后缀 = 消误红；② .gitattributes 加 `*.xxx -text` = 断绝 git add 时的行尾洗涤（守护看不见 git 层腐蚀，只有 ② 断根）。② 即 §4 否决项的事故驱动启用条件，非常设负担。缓解面：严格解码对几乎一切真实二进制 throw，大概率当场红（工作区文件未被 git 动过，可无损恢复）；真正静默类（合法 UTF-8 + 无 NUL + 含 CRLF 对的二进制）极窄 |
| 钩子触发面盲区 | `.gitattributes`/`.editorconfig`/`assets/**` 不在 .githooks 触发 case——只动这些文件的提交本地跳过门禁，CI push 后兜底；TH-3/4 自己的提交就在盲区里，按 G-28 自觉手动跑。是否按 D-93「守护读谁谁触发」扩触发面 = **§10 拍板项 #2** |
| 第三方在老工具链（Win7 时代）跑本仓脚本 | Windows 10+ / pwsh 7 是本仓明确前提（ADR-16）。**接受** |

---

## 8. 明确不做（边界）

- 不动用户 global 的 `core.autocrlf=input`：attributes 全覆盖后它无发言权。
- 不给守护加「工作区行尾」断言（§4 已述）。
- 不预先给全部二进制后缀常设 `binary` 标注（§4；事故驱动的 `*.xxx -text` 覆盖不算违背）。
- **不动 .githooks/pre-commit 的 CR 容忍逻辑**（双保险 fail-safe 不拆）；触发面 case 清单扩否是另一回事，见 §10 #2——本条只冻结容忍逻辑，不冻结触发面。
- 不给 md/yaml 单独定编码规则——一条规则管全部文本。
- 不动 docs/archive/*（史档；ADR-19 档自带让位注记，自洽）。

---

## 9. 与 scripts-doc-system（ADR-19）的关系（v1.1 重写——其已完工归档，排队协议失效）

- ADR-19 已于 2026-09-19 完工归档：头注用法卡、docs 顶层 scripts.md、断言 8、断言 5 扩 .py 均已上线。本方案的确定依赖：
  - TH-5 头注按 **ADR-19 用法卡七字段**写（无未定分支）；
  - TH-5 同提交履行 ADR-19 §5 规矩②：docs/scripts.md 对应小节同步扩写；
  - TH-5 脚本引用方案文档须指 **docs/archive/** 路径（scripts→drafts 引用 = 断言 5 违规），故 TH-1 定稿归档必须先行；
  - 定稿改稿按 scripts-doc-system SD-0 先例处理前向引用（本方案草稿期在 drafts 不进扫描面，定稿落 archive 后亦被断言 5 排除——史档零负担）。
- AGENTS §8 看板无本方案行（草稿期未上架待拍板板块——ADR-19 施工时的 P9 病同款）；TH-1 随定稿直接走「完工归档」，无看板清行动作。

---

## 10. 待拍板清单（逐条点头/改选）

| # | 问题 | 推荐 | 备选与代价 |
|---|---|---|---|
| 1 | 方案是否批准（v1.1 含三路评审修订；含 G-23 政策翻转、TH-4 改既有 .editorconfig） | 批准 | 不批 = 维持双轨 BOM 政策 + .editorconfig 每次保存回加 BOM 的潜在打架 |
| 2 | 钩子触发面是否扩 `.gitattributes*`/`.editorconfig*` | 扩（D-93「守护读谁谁触发」原则对齐；两行 case + AGENTS §2 触发面行同步） | 不扩：按 G-28 自觉手动跑，CI 兜底（接受盲区） |
| 3 | 守护落点 | 并进 check_personal_paths.ps1（同扫描面，高内聚） | 新开脚本：清单逻辑抄两份 |
| 4 | .editorconfig C6 既有承诺（insert_final_newline / trim_trailing_whitespace / [*.md] trim=false）去留 | 原样保留（churn 最小；md 硬换行保护是内容安全；editorconfig 管编辑器层本不以守护为界） | 删除：符合「只写守护查得到的承诺」字面原则，但伤 md 硬换行且与 C6 决策打架 |

---

## 11. 明确不做（本方案范围外，存档防歧义）

- AGENTS §8 看板「草稿期漏登待拍板行」的病根本身（守护化）不在本包修；本包无看板清行动作（§9 已述）。
- assets/SOURCES.md 等 assets 文本的 BOM/LF：守护扫描面覆盖（ls-files 全量），但 assets-only 提交钩子放行（G-28 现状）——CI 兜底，与全仓现状一致，不单独处理。

---

## 12. 修订记录

### v1.0 → v1.1（2026-09-19 三子代理评审：对抗评审 / 事实核查 / 施工预演）

| # | 发现 | 来源 | 严重度 | 去向 |
|---|---|---|---|---|
| 1 | **TH-4 前提错误**：.editorconfig 已存在（C6 入库），「新建」= 覆写销毁缩进/md 保护成果；且 `[*.ps1] utf-8-bom` 行是旧政策唯一编辑器机器载体，「换行不钉死」注释与 TH-3 矛盾 | 三路一致（评审/核查/预演独立发现） | 高 | §1.2 现状 + TH-1 #5 + TH-4 重写 + 拍板项 4 重写 |
| 2 | **§9/头注关联全面失效**：scripts-doc-system 已完工归档，条件分支应定化；引用路径过时 | 评审+核查 | 高 | §9 重写 + 头注关联行 |
| 3 | **TH-2 文本往返陷阱**：「自动认 BOM 重写」与「只去头三字节」两半矛盾；Get-Content/Set-Content 路线会 CRLF 写回+丢尾行 | 评审+预演 | 中 | TH-2 钉死字节手术 + 四连验证（实测 cmp 恰 3 字节） |
| 4 | **TH-1 连锁面修正**：补 AGENTS §4 括注、G-23 注记段、.editorconfig 行三处遗漏；确认 README/release 头注/目录页/脚本头注/史档五类**不改**（连锁面比 v1.0 预想小且明确） | 核查+预演 | 中 | TH-1 全清单表 |
| 5 | **TH-3 时序硬约束**：renormalize 会搬走一切未提交改动 → TH-2 必须先提交；renormalize 只在干净树上跑 | 评审+预演 | 中 | TH-3 |
| 6 | **触发面盲区未申报**：.gitattributes/.editorconfig/assets 不在钩子触发面，「入库前机器强制」存在本地盲区，CI 兜底；D-93 原则与实现不对齐 | 评审+预演 | 中 | §7 风险账本新行 + 拍板项 #2 |
| 7 | §7 第 3 行与 §4 第五行矛盾：守护排除清单消误红 ≠ 断绝 git 洗涤；断根手段恰是 §4 否决的标注——改为「事故驱动启用」消解 | 评审 | 中 | §7 两层修复改写 + §4 括注 |
| 8 | TH-5 三条实现细节：i/-text 陷阱（禁「≠i/lf 即红」）、DecoderFallbackException 窄 catch + try/catch 转 FAIL、.NET API 走进程 cwd 须显式拼根 | 评审+预演 | 中 | TH-5 伪代码 |
| 9 | **转正落点错误**：v1.0 写「docs/ 顶层」；先例与规则指向直接归档 archive（且 TH-5 脚本引用方案须 archive 路径——scripts→drafts = 断言 5 违规） | 预演 | 中 | 头注 + TH-1 |
| 10 | 现状计数漂移：281/197 → 283/199（ADR-19 新增文件）；二进制列表漏 icns；release.ps1 行号 110→132 | 核查 | 低 | §1.2（注明以施工日实测为准） |
| 11 | 决策号应发 ADR-20（对标 ADR-16）；docs/scripts.md 小节随 TH-5 同提交扩写；TH-6 去重（sh 钩子与 precommit 同链路）；G-23 对策段「四守护」计数失真顺手修 | 预演 | 低 | TH-1/TH-5/TH-6 |

**判对项（三路攻击未动）**：BOM 零机械依赖清点、守护落点与 .bin 闭环、index/worktree 切分、`* text=auto` 消融、「git 对编码零原生能力」第一性判断、D-88 同源零改动、charset=utf-8 语义、验证选点（fresh clone + notes + 不拆 CR 容忍）。预演代理总评：「TH-2/3/5/6 全部命令与断言逻辑已实测可行」。
