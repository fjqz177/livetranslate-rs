# ============================================================
# 死契约守卫（架构 v2.1 E6，docs/architecture-v2-improvements.md ADR-13②）
#
# 断言 lt-proto 契约枚举的每个变体在定义之外至少有一次真实引用（生产或
# 消费）——「纯加法豁免」冻结规则的回收机制：变体只进不出的熵增通道封死。
# 判定（非定义文件、非注释行的 \b 变体名\b 命中数）：
#   0 命中 → 硬失败（死变体，删除或进 $ReservedWhitelist 显式编目）
#   1 命中 → WARN（半死：只有生产无消费或反之，人工复核）
#   ≥2    → 通过
#
# 预留白名单 = 文档明示的契约预留位（删保留裁决项时同步维护）：
#   DownloadPhase.Start / DownloadPhase.Integrity —— E2 契约预留（下载
#   起止相位，下载器当前仅产 Progress）
#
# 用法：  powershell -File scripts/check_dead_contract.ps1
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

# 预留白名单（Enum.Variant）——增删须与 docs/architecture-v2-improvements.md §3 ADR-13 同步
$ReservedWhitelist = @(
    'DownloadPhase.Start',
    'DownloadPhase.Integrity',
    'UiEvent.ModelLoadDone'   # D-78 向导保留面：加载对话框关窗契约（实际关窗由
                              # AsrDevice/AsrUnavailable 边沿代偿）；向导接线时激活
)

# 已定性合法单边形态（WARN 常驻项，2026-09-09 校准确认——不必逐次复核）：
#   AppCommand.OverlayToggle —— 生产经 from_menu_id 字符串映射（proto 内单点），
#     变体名只出现在消费端；同型的 Pause/ShowPanel/Quit 因托盘/悬浮窗双生产点≥2
#   ThreadRole.LogBridge / ThreadRole.ArteryBridge / ThreadRole.AudioBridge ——
#     角色仅出生点按名（死亡事件携带值不携带名），监督器按 Policy 泛型处理
# 新增同类单边形态时在此登记，其余 WARN 仍需人工定性

# 扫描面：lt-proto 契约枚举所在文件（events/layout/asr_result；settings.rs 的
# 值域常量清单非变体契约，不在本守卫面）
$protoFiles = @(
    (Join-Path $RepoRoot 'crates/lt-proto/src/events.rs'),
    (Join-Path $RepoRoot 'crates/lt-proto/src/layout.rs'),
    (Join-Path $RepoRoot 'crates/lt-proto/src/asr_result.rs')
)

# 全仓 .rs（计数面 = 定义之外、非注释行）
$allRs = Get-ChildItem -Path (Join-Path $RepoRoot 'crates') -Recurse -Filter *.rs -File

# 预裁剪：每文件去掉注释行后拼成单串（\b 计数用；性能——避免逐行正则 ×60 变体）
$filtered = @{}
foreach ($f in $allRs) {
    $lines = Get-Content $f.FullName -Encoding UTF8 | Where-Object { $_ -notmatch '^\s*//' }
    $filtered[$f.FullName] = ($lines -join "`n")
}

# ── 抽取契约枚举变体（enum 体 = 到列 0 的 `}`；变体 = 4 空格缩进大写开头）──
$variants = [System.Collections.Generic.List[hashtable]]::new()
foreach ($pf in $protoFiles) {
    if (-not (Test-Path $pf)) { Write-Error "契约文件缺失: $pf" }
    $inEnum = $false
    $enumName = ''
    foreach ($ln in (Get-Content $pf -Encoding UTF8)) {
        if (-not $inEnum) {
            if ($ln -match '^\s*pub enum (\w+)') {
                $inEnum = $true
                $enumName = $Matches[1]
            }
            continue
        }
        if ($ln -match '^\}') {
            $inEnum = $false
            continue
        }
        if ($ln -match '^\s{4}([A-Z]\w*)\s*(\{|$|\(|,)') {
            $variants.Add(@{ Enum = $enumName; Variant = $Matches[1] })
        }
    }
}

if ($variants.Count -eq 0) { Write-Error '未抽取到任何契约变体——解析逻辑或文件布局漂移，请人工检查' }

# ── 逐变体计数（proto 定义目录不计——定义/映射表所在）──
$dead = @()
$halfDead = @()
foreach ($v in $variants) {
    $name = $v.Variant
    $hits = 0
    foreach ($f in $allRs) {
        if ($f.FullName -like '*lt-proto*') { continue }
        if ($filtered[$f.FullName] -match "\b$name\b") {
            $hits++
            if ($hits -ge 2) { break }
        }
    }
    $id = "$($v.Enum).$name"
    if ($ReservedWhitelist -contains $id) { continue }
    if ($hits -eq 0) { $dead += $id }
    elseif ($hits -eq 1) { $halfDead += $id }
}

if ($halfDead.Count -gt 0) {
    Write-Host "WARN（半死变体，单边引用——人工确认是生产缺消费还是反之）:" $(${halfDead} -join ', ')
}

if ($dead.Count -gt 0) {
    Write-Host "FAIL（死契约变体，定义之外零引用）:" $(${dead} -join ', ')
    Write-Host '处置：删除该变体（走契约评审 D-xx + PROTO_VERSION 递增），或确认预留性质后编入脚本顶部 $ReservedWhitelist（注明裁决依据）'
    exit 1
}

Write-Host "check_dead_contract: OK（$($variants.Count) 个契约变体，0 死 / $($halfDead.Count) 半死 WARN）"
exit 0
