# ============================================================
# 依赖方向守护（架构 2.0 W7，docs/archive/architecture-v2.md §6.1 / §3.1 白名单表）
#
# 断言十 crate 内部依赖图严格符合白名单：表内未列的内部依赖一律非法
# （含 dev-dependencies——测试代码同样受分层约束；dev-dependencies 例外
# 见 $DevExtra，均为「探针/集成测试需真实 crate」的已记录决策）。
#
# 用法：  pwsh -File scripts/check_deps.ps1
# 退出码：0 = 通过；非 0 = 存在违规边（输出逐条 VIOLATION）
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

# ── §3.1 白名单（源 = docs/archive/architecture-v2.md §3.1 白名单表 + E3/ADR-10 修订；
#    lt-app 允许十库全部） ──
$Whitelist = [ordered]@{
    'lt-proto'       = @()
    'lt-i18n'        = @()
    'lt-models'      = @('lt-proto')
    'lt-download'    = @('lt-proto')
    'lt-audio'       = @('lt-models')
    'lt-asr'         = @('lt-proto')          # lt-models 仅 dev-dep（探针共享，W6）
    'lt-translate'   = @('lt-proto')          # E3：翻译域常量上移 proto（ADR-10）
    'lt-orchestrator'= @('lt-proto', 'lt-models', 'lt-download', 'lt-audio', 'lt-asr', 'lt-translate')
    'lt-ui'          = @('lt-proto', 'lt-i18n', 'lt-models')   # E3：纯投影 crate（ADR-10，常量边裁除）
    'lt-app'         = @('lt-proto', 'lt-i18n', 'lt-models', 'lt-download', 'lt-audio',
                         'lt-asr', 'lt-translate', 'lt-orchestrator', 'lt-ui')
}

# dev-dependencies 特批（仅 dev 段生效；normal 段仍受 Whitelist 严格约束）
$DevExtra = [ordered]@{
    'lt-asr'       = @('lt-models')   # 探针共享 probe_models_root（PH-1）
    'lt-download'  = @('lt-models')   # 下载集成测试（真实缓存路径语义）
    'lt-orchestrator' = @('serde_json') # E2：覆盖率测试 serde 键集（ADR-13①；非内部边不受限，登记以示明） 
}

# ── 解析 Cargo.toml 某段的内部依赖名（格式兼容 `lt-x = {...}` 与 `lt-x.workspace = true`） ──
function Get-InternalDeps([string]$TomlPath, [string]$Section) {
    $res = @()
    foreach ($ln in (Get-Content $TomlPath)) {
        if ($ln -match '^\[([A-Za-z0-9._-]+)\]') {
            if ($Matches[1] -eq $Section) { $inSection = $true } else { $inSection = $false }
            continue
        }
        if ($inSection -and $ln -match '^\s*lt-([a-z0-9-]+)\s*[=.]') {
            $res += "lt-$($Matches[1])"
        }
    }
    return $res
}

# ── 执行断言 ──
$violations = @()
$edges = 0
foreach ($crate in $Whitelist.Keys) {
    $toml = Join-Path (Join-Path $RepoRoot 'crates') "$crate/Cargo.toml"
    if (-not (Test-Path $toml)) { $violations += "${crate}: Cargo.toml 不存在"; continue }

    foreach ($d in (Get-InternalDeps $toml 'dependencies')) {
        $edges++
        if ($Whitelist[$crate] -notcontains $d) {
            $violations += "$crate（dependencies）→ $d 违反白名单"
        }
    }
    $allowedDev = @($Whitelist[$crate]) + @($DevExtra[$crate]) | Select-Object -Unique
    foreach ($d in (Get-InternalDeps $toml 'dev-dependencies')) {
        $edges++
        if ($allowedDev -notcontains $d) {
            $violations += "$crate（dev-dependencies）→ $d 违反白名单"
        }
    }
}

if ($violations.Count -gt 0) {
    Write-Host "check_deps: 失败（$($violations.Count) 条违规边）" -ForegroundColor Red
    $violations | ForEach-Object { Write-Host "  VIOLATION: $_" -ForegroundColor Red }
    exit 1
}
Write-Host "check_deps: OK（10 crates，$edges 条内部边全部符合 §3.1 白名单）" -ForegroundColor Green
exit 0
