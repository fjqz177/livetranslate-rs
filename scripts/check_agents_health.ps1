# ============================================================
# 总纲与文档引用网健康守护（AO-4 起家，D-93/ADR-17 扩容；规格史 = docs/archive/agents-md-overhaul.md 附录 D + docs/archive/doc-network-hardening.md）
#
# 断言八组（编号 2~7 沿用防引用断裂；8 号曾为水位线断言，ADR-18 退役后由
# ADR-19 复用为脚本登记面——史档中的「断言 8」均指水位线，现行定义以本头注为准；
# 9 号曾为看板双真源对账（ADR-22），D-102 看板退役后改为流程装机完整性；
# 原 1/1b 额度与水位线已退役）：
#   2. 引用路径存在：AGENTS.md 内引用的仓库内路径全部存在
#      （docs/ scripts/ crates/ assets/ .github/ .cargo/ .githooks/ 前缀；
#        docs/drafts/ 与含通配符的路径跳过）
#   3. G-编号无死引用：AGENTS 出现的每个 G-编号均在 docs/gotchas.md 实存
#      + gotchas 编号连续（防速查与全书漂移）
#   4. AGENTS §8 段内状态/流程双真源指针（GitHub Issues + docs/agents/workflow.md，D-102）
#   5. 引用路径存在（扩面）：docs 顶层 md / docs/agents / scripts / workflows / 根 README；
#      scripts|workflows 指向 docs/drafts/<文件> = 违规，docs 顶层|agents 指向 = WARN
#   6. G-编号全仓无死引用（同扫描集，gotchas 本身 = 定义源不扫；非数字残留一并逮）
#   7. 归档机械面：docs/archive-index.md 归档表 ↔ docs/archive 目录双向对账
#      + 每份档案前 8 行含「已归档/归档注记」标记
#   8. 脚本登记面：scripts/ 下每个 .ps1/.py 须在 docs 顶层 scripts.md 有节
#      （### `scripts/<名>`），节指向的脚本须实存——双向对账，无豁免
#      （对账读目录页与文件清单，不存在内容自触发，守护自身同样须登记）
#   9. 流程装机完整性：docs/agents/ 四件套实存（workflow.md = 流程真源，
#      issue-tracker / triage-labels / domain = 装机配置；D-102）
#
# 实现约束（G-20，docs/gotchas.md）：文本匹配用 PowerShell 原生正则，勿调外部 grep。
# 触发面：precommit.ps1 第二项 + CI gate job；本地钩子触发面 = 「门禁读谁、谁触发」
# （docs/ scripts/ .github/ rust-toolchain*，D-93 DH-18）。
# 用法：  pwsh -File scripts/check_agents_health.ps1（本仓 pwsh-only，ADR-16）
# 参数：  -RepoRoot（默认 = 脚本上级目录，一般不用传）
# 前置：  pwsh 7；AGENTS.md / docs/gotchas.md / docs/README.md / docs/archive/ 在位。
# 退出码：0 = 通过（可带 WARN）；非 0 = 存在违规（逐条输出）。
# 产物 / 副作用：无（只读）。
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $RepoRoot

$violations = @()
$warn = @()

$agentsPath = Join-Path $RepoRoot 'AGENTS.md'
if (-not (Test-Path -LiteralPath $agentsPath)) {
    Write-Host "AGENTS.md 不存在——无处守护" -ForegroundColor Red
    exit 1
}
$content = [System.IO.File]::ReadAllText($agentsPath)

# 行数按换行符计数（等价 wc -l：结尾换行不补一行）
$totalLines = ($content -split "`n").Count
if ($content.EndsWith("`n")) { $totalLines-- }
$totalBytes = (Get-Item -LiteralPath $agentsPath).Length

$marker = [regex]::Match($content, '(?m)^## 8\.')
if ($marker.Success) {
    $head = $content.Substring(0, $marker.Index)
    $mainLines = ($head -split "`n").Count - 1   # head 以 `n 结尾，末尾空段不计
    $mainBytes = [System.Text.Encoding]::UTF8.GetByteCount($head)
} else {
    $mainLines = $totalLines
    $mainBytes = $totalBytes
    $violations += "状态与工单：找不到 '^## 8.' 分节标记，主体口径按全文件从严计算"
}
# ── 断言 2：引用路径存在 ──
$refs = [regex]::Matches($content, '(?:docs|scripts|crates|assets|\.github|\.cargo|\.githooks)/[A-Za-z0-9_\-./]+') |
    ForEach-Object { $_.Value } | Sort-Object -Unique
foreach ($p in $refs) {
    # 提法正则不匹配通配符（捕获面纯字母数字斜杠）；目录提法按目录 Test-Path
    if ($p -like 'docs/drafts/*') { continue }            # 草稿区 CI 上不存在
    if (-not (Test-Path -LiteralPath $p)) { $violations += "死引用：$p（AGENTS.md 引用但仓库无此路径）" }
}

# ── 断言 3：G-编号无死引用 + gotchas 连续 ──
$gotchasPath = Join-Path $RepoRoot 'docs/gotchas.md'
$defs = @()   # 提升作用域：断言 6（G 编号全仓）复用；空 = gotchas 缺失/无条目，断言 6 自短路
if (-not (Test-Path -LiteralPath $gotchasPath)) {
    $violations += "docs/gotchas.md 不存在（坑册是 AGENTS §7 防呆行与 §4 坑册路由的真源）"
} else {
    $gotchas = [System.IO.File]::ReadAllText($gotchasPath)
    $defs = [regex]::Matches($gotchas, '(?m)^## G-(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
        if ($defs.Count -eq 0) {
            $violations += "docs/gotchas.md 无『## G-数字』条目（格式漂移？）"
        } else {
        $missing = @(1..($defs[-1]) | Where-Object { $defs -notcontains $_ })
        if ($missing.Count -gt 0) {
            $violations += "坑册编号不连续：缺 $($missing -join ', ')"
        }
        # lookbehind 对齐断言 6：防把「AG-1」「SVG-12」之类局部串误当 G-编号
        $used = [regex]::Matches($content, '(?<![A-Za-z0-9])G-(\d+)') |
            ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
        foreach ($g in $used) {
            if ($defs -notcontains $g) { $violations += "死引用：G-$g（AGENTS.md 引用但坑册无此条）" }
        }
    }
}

# ── 断言 4：AGENTS §8 状态/流程双真源指针（D-102）──
$sec8 = [regex]::Match($content, '(?m)^## 8\.[\s\S]*$')
$sec8Text = if ($sec8.Success) { $sec8.Value } else { '' }
if ($sec8Text -notmatch 'GitHub Issues') {
    $violations += "AGENTS.md §8 段内无 'GitHub Issues' 状态真源指针（D-102）"
}
if ($sec8Text -notmatch 'docs/agents/workflow\.md') {
    $violations += "AGENTS.md §8 段内无 docs/agents/workflow.md 流程真源指针（D-102）"
}

# ── 断言 5：引用路径存在扩面（D-93 DH-14；D-102 scan 集 prompts → docs/agents）──
# 扫描集 = docs 顶层 *.md（不递归）+ docs/agents/*.md + scripts/*.ps1|.py + .github/workflows/*.yml + 根 README.md
# 排除：docs/archive/（史档引用为当时快照）、docs/gotchas.md（断言 6 定义源）、gitignored 三目录
# 草稿区分级：scripts|workflows 指向 docs/drafts/<文件> = 违规；docs 顶层|agents 指向 = WARN（AGENTS 保留既有豁免）
$scanRel = @()
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'docs') -Filter '*.md' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'docs/agents') -Filter '*.md' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'scripts') -Filter '*.ps1' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'scripts') -Filter '*.py' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot '.github/workflows') -Filter '*.yml' -File).FullName
$readmeRoot = Join-Path $RepoRoot 'README.md'
if (Test-Path -LiteralPath $readmeRoot) { $scanRel += $readmeRoot }
# 豁免表：键 = '<相对路径>|<引用>'，值 = 理由（干跑发现误报时在此登记，禁无理由豁免）
# 登记史：①2026-09-19 五条——README/closeout 的「生成物不入库」政策提法 ×3，与 G-29 收编后
# 坑册正文以掩盖目录作反例举证 ×2（fresh clone 验证法当场抓到，见 G-29/G-30）；
# 根因均为本机未入库同名目录掩盖 Test-Path 判定、43 个积压提交首推 CI 才暴露。
# ② 2026-09-28 D-102——退役 closeout 卡键，新增 domain.md 政策提法与台账史行退役路径提法三条。
$refExempt = @{
    'README.md|docs/architecture/'       = '正文讲架构图等生成物不入库的政策提法，非仓库内链接'
    'README.md|docs/ui-audit/'           = '正文讲走查截图等生成物不入库的政策提法，非仓库内链接'
    'docs/gotchas.md|docs/architecture/' = 'G-29 以掩盖目录为反例举证（坑册合法引用），非仓库内链接'
    'docs/gotchas.md|docs/ui-audit/'     = 'G-29 以掩盖目录为反例举证（坑册合法引用），非仓库内链接'
    'docs/agents/domain.md|docs/adr/'    = '政策提法（禁建该目录），非仓库内链接（D-102 扩扫 docs/agents 进网）'
    'docs/decisions.md|docs/prompts/'    = '台账史行（D-97/ADR-15/D-102）提及退役卡区路径；台账行禁改写，只加取代注记（D-102）'
    'docs/decisions.md|docs/board.md'    = '台账史行（ADR-22/D-102）提及退役看板路径；台账行禁改写，只加取代注记（D-102）'
}
foreach ($f in $scanRel) {
    $rel = ([IO.Path]::GetRelativePath($RepoRoot, $f)) -replace '\\', '/'
    if ($rel -eq 'scripts/check_agents_health.ps1') { continue }  # 豁免表定义源不扫（同 gotchas 之于 G-编号）：表键必然含路径字面量
    $t = [System.IO.File]::ReadAllText($f)
    foreach ($m in [regex]::Matches($t, '(?:docs|scripts|crates|assets|\.github|\.cargo|\.githooks)/[A-Za-z0-9_\-./]+')) {
        $p = $m.Value
        if ($refExempt.ContainsKey("$rel|$p")) { continue }
        if ($p -eq 'docs/drafts/') { continue }   # 裸目录提法（无文件名）不算引用
        if ($p -like 'docs/drafts/*') {
            if ($rel -like 'scripts/*' -or $rel -like '.github/*') {
                $violations += "死引用：$rel → $p（脚本/workflow 指向草稿区——草稿不入库，转正后须改指 archive）"
            } elseif ($rel -ne 'AGENTS.md') {
                $warn += "文档指向草稿区（fresh clone 不存在）：$rel → $p"
            }
            continue
        }
        if (-not (Test-Path -LiteralPath $p)) { $violations += "死引用：$rel → $p（引用但仓库无此路径）" }
    }
}

# ── 断言 6：G-编号全仓无死引用（D-93 DH-15；gotchas=定义源不扫，archive=史档不扫）──
# gotchas 缺失/无条目时短路（根因已由断言 3 报过，防全仓雪崩噪音，S3-3）
if ($defs.Count -gt 0) {
    $gScan = @($scanRel | Where-Object { (([IO.Path]::GetRelativePath($RepoRoot, $_)) -replace '\\', '/') -ne 'docs/gotchas.md' })
    foreach ($f in $gScan) {
        $t = [System.IO.File]::ReadAllText($f)
        # 相对路径定位（S3-1）：根 README 与 docs/README 分得清
        $fn = ([IO.Path]::GetRelativePath($RepoRoot, $f)) -replace '\\', '/'
        foreach ($m in [regex]::Matches($t, '(?<![A-Za-z0-9])G-([A-Za-z0-9]+)')) {
            $g = $m.Groups[1].Value
            if ($g -match '^\d+$') {
                if ($defs -notcontains [int]$g) { $violations += "死引用：G-$g（$fn 引用但坑册无此条）" }
            } else {
                $violations += "死引用：G-$g（$fn 非数字编号——坑册无此条，疑局部号残留未带路径）"
            }
        }
    }
}

# ── 断言 7：归档机械面（D-93 DH-16；ADR-22 起索引真源 = docs/archive-index.md，docs/README.md 只留指针）──
# (a) docs/archive-index.md 归档表 ↔ docs/archive 目录双向对账（缺行/多行/孤儿都红）
# (b) 每份档案前 8 行必须含「已归档|归档注记」（单点进入不再被归档前状态误导）
$archiveIndexPath = Join-Path $RepoRoot 'docs/archive-index.md'
$indexNames = @()
$indexParsed = $false
if (-not (Test-Path -LiteralPath $archiveIndexPath)) {
    $violations += "归档索引（docs/archive-index.md）不存在（归档表真源缺失）"
} else {
    $indexDocs = [System.IO.File]::ReadAllText($archiveIndexPath)
    # 行首锚定（(?m)^）：正文散文里提到节标题字面量时不再抢匹配（G-30 同族自检互搏坑）；
    # 尾锚双态（\r?\n## |\z，S3-5）：「归档文档」是最后一节时也能命中
    $secMatch = [regex]::Match($indexDocs, '(?m)^## 归档文档(?<sec>.*?)(\r?\n## |\z)', [System.Text.RegularExpressions.RegexOptions]::Singleline)
    if ($secMatch.Success) {
        $indexParsed = $true
        foreach ($m in [regex]::Matches($secMatch.Groups['sec'].Value, '(?m)^\|\s*`([a-z0-9\-]+\.md)`')) {
            $indexNames += $m.Groups[1].Value
        }
    } else {
        $violations += "docs/archive-index.md 找不到「## 归档文档」节（索引结构漂移？）"
    }
}
$dirNames = (Get-ChildItem -Path (Join-Path $RepoRoot 'docs/archive') -Filter '*.md' -File).Name
$missingInIndex = @()
if ($indexParsed) { $missingInIndex = @($dirNames | Where-Object { $indexNames -notcontains $_ }) }
$missingOnDisk  = @($indexNames | Where-Object { $dirNames -notcontains $_ })
if ($missingInIndex.Count -gt 0) { $violations += "归档索引缺行：$($missingInIndex -join ', ')" }
if ($missingOnDisk.Count -gt 0)  { $violations += "索引指向不存在的档案：$($missingOnDisk -join ', ')" }
foreach ($n in $dirNames) {
    $head = (Get-Content -LiteralPath (Join-Path $RepoRoot "docs/archive/$n") -TotalCount 8) -join "`n"
    if ($head -notmatch '已归档|归档注记') { $violations += "归档缺注记：docs/archive/$n（前 8 行无『已归档/归档注记』）" }
}

# ── 断言 8：脚本登记面（scripts-doc-system SD-3 / ADR-19；8 号曾为水位线断言，ADR-18 退役后复用）──
# (a) 磁盘 → 册：scripts/ 下每个 .ps1/.py 须在目录页（docs 顶层 scripts.md）有节（### `scripts/<名>`）
# (b) 册 → 磁盘：册上抽出的每个 scripts/… 路径须实存（孤儿条目）
# 全部节强制含本守护自身，无豁免代码（对账读目录页与文件清单，不存在内容自触发；
# 节数随脚本增删浮动，以两侧对账为准）。
# 枚举平铺非递归——scripts/ 出现子目录时须回改此处。正文出现节标题样式字面量会误报，禁。
$scriptsMd = Join-Path $RepoRoot ('docs' + '/scripts.md')   # 拼接书写：断言 5 对本文件有定义源自豁免，
                                                            # 直写亦无碍；拼接防该豁免未来被改时自我字面量误伤（防御性冗余）
if (-not (Test-Path -LiteralPath $scriptsMd)) {
    $violations += '脚本目录页（docs 顶层 scripts.md）不存在（新增脚本前先建册）'
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

# ── 断言 9：流程装机完整性（D-102）──
# docs/agents/ 四件套实存（workflow.md = 流程真源；issue-tracker / triage-labels / domain = 装机配置）
foreach ($f in 'docs/agents/workflow.md', 'docs/agents/issue-tracker.md', 'docs/agents/triage-labels.md', 'docs/agents/domain.md') {
    if (-not (Test-Path -LiteralPath (Join-Path $RepoRoot $f))) {
        $violations += "流程装机缺件：$f（D-102 四件套）"
    }
}

# ── 汇总 ──
if ($violations.Count -gt 0) {
    Write-Host ""
    foreach ($v in $violations) { Write-Host "VIOLATION: $v" -ForegroundColor Red }
}
if ($warn.Count -gt 0) {
    Write-Host ""
    foreach ($w in $warn) { Write-Host "WARN: $w" -ForegroundColor Yellow }
}
if ($violations.Count -gt 0) {
    Write-Host ""
    Write-Host "总纲健康守护未过（$($violations.Count) 处违规；$($warn.Count) 处提醒）——已中止" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "ok：主体 $mainLines 行/$mainBytes B + 全文件 $totalLines 行/$totalBytes B（不设额度，ADR-18）；引用 / G-编号 / 状态指针 / 引用网扩面 / 归档机械面 / 脚本登记面 / 装机四件套全过（提醒 $($warn.Count) 条）" -ForegroundColor Green
exit 0
