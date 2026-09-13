# ============================================================
# 总纲健康守护（AO-4；docs/drafts/agents-md-overhaul.md 附录 D）
#
# 断言四组（消融后无重复检查）：
#   1. 两档预算：`## 8.` 之前主体 ≤170 行 / 17KB（宪法面）；
#      全文件 ≤210 行 / 22KB（看板活状态波动硬顶，低于旧 23,010B）
#   2. 引用路径存在：AGENTS.md 内引用的仓库内路径全部存在
#      （docs/ scripts/ crates/ assets/ .github/ .cargo/ .githooks/ 前缀；
#        docs/drafts/ 与含通配符的路径跳过）
#   3. G-编号无死引用：AGENTS 出现的每个 G-n 在 docs/gotchas.md 实存
#      + gotchas 编号连续（防速查与全书漂移）
#   4. 看板三小节标题（待拍板/施工中/遗留）存在
#
# 实现约束（G-20，docs/gotchas.md）：文本匹配用 PowerShell 原生正则，勿调外部 grep。
# 触发面：precommit.ps1 第七项 + CI gate job（.githooks 触发条件含本守护关注的 docs 路径）。
# 用法：  pwsh -File scripts/check_agents_health.ps1（本仓 pwsh-only，ADR-16）
# 退出码：0 = 通过；非 0 = 存在违规（逐条输出）
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $RepoRoot

$violations = @()

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
$MainLineCap = 170
$MainByteCap = 17 * 1024
$TotalLineCap = 210
$TotalByteCap = 22 * 1024

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
if (-not (Test-Path -LiteralPath $gotchasPath)) {
    $violations += "docs/gotchas.md 不存在（坑册是 AGENTS §7 速查的真源）"
} else {
    $gotchas = [System.IO.File]::ReadAllText($gotchasPath)
    $defs = [regex]::Matches($gotchas, '(?m)^## G-(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
    if ($defs.Count -eq 0) {
        $violations += "docs/gotchas.md 无 '## G-n' 条目（格式漂移？）"
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

# ── 汇总 ──
if ($violations.Count -gt 0) {
    Write-Host ""
    foreach ($v in $violations) { Write-Host "VIOLATION: $v" -ForegroundColor Red }
    Write-Host ""
    Write-Host "总纲健康守护未过（$($violations.Count) 处）——已中止" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "ok：主体 $mainLines 行/$mainBytes B + 全文件 $totalLines 行/$totalBytes B（两档预算内）；引用 / G-编号 / 看板三小节全过" -ForegroundColor Green
exit 0
