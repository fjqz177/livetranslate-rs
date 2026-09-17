# ============================================================
# 总纲与文档引用网健康守护（AO-4 起家，D-93/ADR-17 扩容；规格史 = docs/archive/agents-md-overhaul.md 附录 D + docs/doc-network-hardening.md）
#
# 断言八组（消融后无重复检查；5~7 共用同一扫描集，见断言 5 注）：
#   1. 两档预算：`## 8.` 之前主体 ≤190 行 / 19KB（宪法面，ADR-17 重校准）；
#      全文件 ≤230 行 / 24KB（看板活状态波动硬顶）——数字唯一真源 = 本文件，宪法头注只留指针
#   1b. 水位线：主体字节 > 额度 85% → WARN（不失败；加字优先以回吐支付，ADR-17）
#   2. 引用路径存在：AGENTS.md 内引用的仓库内路径全部存在
#      （docs/ scripts/ crates/ assets/ .github/ .cargo/ .githooks/ 前缀；
#        docs/drafts/ 与含通配符的路径跳过）
#   3. G-编号无死引用：AGENTS 出现的每个 G-编号均在 docs/gotchas.md 实存
#      + gotchas 编号连续（防速查与全书漂移）
#   4. 看板三小节标题（待拍板/施工中/遗留）存在
#   5. 引用路径存在（扩面）：docs 顶层 md / prompts / scripts / workflows / 根 README；
#      scripts|workflows 指向 docs/drafts/<文件> = 违规，docs 顶层/prompts 指向 = WARN
#   6. G-编号全仓无死引用（同扫描集，gotchas 本身 = 定义源不扫；非数字残留一并逮）
#   7. 归档机械面：docs/README.md 归档表 ↔ docs/archive 目录双向对账
#      + 每份档案前 8 行含「已归档/归档注记」标记
#
# 实现约束（G-20，docs/gotchas.md）：文本匹配用 PowerShell 原生正则，勿调外部 grep。
# 触发面：precommit.ps1 第六项 + CI gate job；本地钩子触发面 = 「守护读谁、谁触发」
# （docs/ scripts/ .github/ rust-toolchain*，D-93 DH-18）。
# 用法：  pwsh -File scripts/check_agents_health.ps1（本仓 pwsh-only，ADR-16）
# 退出码：0 = 通过（可带 WARN）；非 0 = 存在违规（逐条输出）
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

# ── 断言 1：两档预算 ──
$MainLineCap = 190                # ADR-17 重校准（2026-09-17）：原 170/17KB 两天即 98.9% 顶死，改「用量+15% 余量」政策
$MainByteCap = 19 * 1024
$TotalLineCap = 230
$TotalByteCap = 24 * 1024
$WaterLevel = 0.85                # 断言 1b：主体字节水位线，超此比例打 WARN 不失败

$marker = [regex]::Match($content, '(?m)^## 8\.')
if ($marker.Success) {
    $head = $content.Substring(0, $marker.Index)
    $mainLines = ($head -split "`n").Count - 1   # head 以 `n 结尾，末尾空段不计
    $mainBytes = [System.Text.Encoding]::UTF8.GetByteCount($head)
} else {
    $mainLines = $totalLines
    $mainBytes = $totalBytes
    $violations += "看板：找不到 '^## 8.' 分节标记，主体预算按全文件从严计算"
}
if ($mainLines -gt $MainLineCap) { $violations += "预算：主体 $mainLines 行 > $MainLineCap 行" }
if ($mainBytes -gt $MainByteCap) { $violations += "预算：主体 $mainBytes B > $($MainByteCap) B (17KB)" }
if ($totalLines -gt $TotalLineCap) { $violations += "预算：全文件 $totalLines 行 > $TotalLineCap 行" }
if ($totalBytes -gt $TotalByteCap) { $violations += "预算：全文件 $totalBytes B > $($TotalByteCap) B (22KB)" }

if ($mainBytes -gt [int]($MainByteCap * $WaterLevel)) {
    $warn += "水位线：主体 $mainBytes B 已超额度 $($MainByteCap) B 的 85%——加字优先以回吐支付（ADR-17）"
}

# ── 断言 2：引用路径存在 ──
$refs = [regex]::Matches($content, '(?:docs|scripts|crates|assets|\.github|\.cargo|\.githooks)/[A-Za-z0-9_\-./]+') |
    ForEach-Object { $_.Value } | Sort-Object -Unique
foreach ($p in $refs) {
    if ($p.Contains('*')) { continue }                    # 通配符（如 docs/prompts/*）跳过
    if ($p -like 'docs/drafts/*') { continue }            # 草稿区 CI 上不存在
    if (-not (Test-Path -LiteralPath $p)) { $violations += "死引用：$p（AGENTS.md 引用但仓库无此路径）" }
}

# ── 断言 3：G-编号无死引用 + gotchas 连续 ──
$gotchasPath = Join-Path $RepoRoot 'docs/gotchas.md'
$defs = @()   # 提升作用域：断言 6（G 编号全仓）复用
if (-not (Test-Path -LiteralPath $gotchasPath)) {
    $violations += "docs/gotchas.md 不存在（坑册是 AGENTS §7 速查的真源）"
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
        $used = [regex]::Matches($content, 'G-(\d+)') |
            ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
        foreach ($g in $used) {
            if ($defs -notcontains $g) { $violations += "死引用：G-$g（AGENTS.md 引用但坑册无此条）" }
        }
    }
}

# ── 断言 4：看板三小节标题 ──
foreach ($h in '### 待拍板', '### 施工中', '### 遗留') {
    if (-not [regex]::IsMatch($content, [regex]::Escape($h))) {
        $violations += "看板缺小节：'$h'（纪律①机制的机械面）"
    }
}

# ── 断言 5：引用路径存在扩面（D-93 DH-14）──
# 扫描集 = docs 顶层 *.md（不递归）+ docs/prompts/*.md + scripts/*.ps1 + .github/workflows/*.yml + 根 README.md
# 排除：docs/archive/（史档引用为当时快照）、docs/gotchas.md（断言 6 定义源）、gitignored 三目录
# 草稿区分级：scripts|workflows 指向 docs/drafts/<文件> = 违规；docs 顶层/prompts 指向 = WARN（AGENTS 保留既有豁免）
$scanRel = @()
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'docs') -Filter '*.md' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'docs/prompts') -Filter '*.md' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot 'scripts') -Filter '*.ps1' -File).FullName
$scanRel += (Get-ChildItem -Path (Join-Path $RepoRoot '.github/workflows') -Filter '*.yml' -File).FullName
$readmeRoot = Join-Path $RepoRoot 'README.md'
if (Test-Path -LiteralPath $readmeRoot) { $scanRel += $readmeRoot }
# 豁免表：键 = '<相对路径>|<引用>'，值 = 理由（干跑发现误报时在此登记，禁无理由豁免）
$refExempt = @{}
foreach ($f in $scanRel) {
    $t = [System.IO.File]::ReadAllText($f)
    $rel = ([IO.Path]::GetRelativePath($RepoRoot, $f)) -replace '\\', '/'
    foreach ($m in [regex]::Matches($t, '(?:docs|scripts|crates|assets|\.github|\.cargo|\.githooks)/[A-Za-z0-9_\-./]+')) {
        $p = $m.Value
        if ($p.Contains('*')) { continue }
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
$gScan = @($scanRel | Where-Object { (([IO.Path]::GetRelativePath($RepoRoot, $_)) -replace '\\', '/') -ne 'docs/gotchas.md' })
foreach ($f in $gScan) {
    $t = [System.IO.File]::ReadAllText($f)
    $fn = Split-Path -Leaf $f
    foreach ($m in [regex]::Matches($t, '(?<![A-Za-z0-9])G-([A-Za-z0-9]+)')) {
        $g = $m.Groups[1].Value
        if ($g -match '^\d+$') {
            if ($defs -notcontains [int]$g) { $violations += "死引用：G-$g（$fn 引用但坑册无此条）" }
        } else {
            $violations += "死引用：G-$g（$fn 非数字编号——坑册无此条，疑局部号残留未带路径）"
        }
    }
}

# ── 断言 7：归档机械面（D-93 DH-16）──
# (a) docs/README.md 归档表 ↔ docs/archive 目录双向对账（缺行/多行/孤儿都红）
# (b) 每份档案前 8 行必须含「已归档|归档注记」（单点进入不再被归档前状态误导）
$readmeDocs = [System.IO.File]::ReadAllText((Join-Path $RepoRoot 'docs/README.md'))
$secMatch = [regex]::Match($readmeDocs, '## 归档文档(?<sec>.*?)\r?\n## ', [System.Text.RegularExpressions.RegexOptions]::Singleline)
$indexNames = @()
if ($secMatch.Success) {
    foreach ($m in [regex]::Matches($secMatch.Groups['sec'].Value, '(?m)^\|\s*`([a-z0-9\-]+\.md)`')) {
        $indexNames += $m.Groups[1].Value
    }
} else {
    $violations += "docs/README.md 找不到「## 归档文档」节（索引结构漂移？）"
}
$dirNames = (Get-ChildItem -Path (Join-Path $RepoRoot 'docs/archive') -Filter '*.md' -File).Name
$missingInIndex = @($dirNames | Where-Object { $indexNames -notcontains $_ })
$missingOnDisk  = @($indexNames | Where-Object { $dirNames -notcontains $_ })
if ($missingInIndex.Count -gt 0) { $violations += "归档索引缺行：$($missingInIndex -join ', ')" }
if ($missingOnDisk.Count -gt 0)  { $violations += "索引指向不存在的档案：$($missingOnDisk -join ', ')" }
foreach ($n in $dirNames) {
    $head = ([System.IO.File]::ReadAllLines((Join-Path $RepoRoot "docs/archive/$n")) | Select-Object -First 8) -join "`n"
    if ($head -notmatch '已归档|归档注记') { $violations += "归档缺注记：docs/archive/$n（前 8 行无『已归档/归档注记』）" }
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
Write-Host "ok：主体 $mainLines 行/$mainBytes B + 全文件 $totalLines 行/$totalBytes B（两档预算内）；引用 / G-编号 / 看板 / 引用网扩面 / 归档机械面全过（提醒 $($warn.Count) 条）" -ForegroundColor Green
exit 0
