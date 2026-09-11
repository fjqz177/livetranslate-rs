# ============================================================
# 提交前门禁（2026-09-11）：单一入口，六项顺序执行、任一失败即停
#
#   1. cargo fmt --all -- --check                    ← 格式（新增）
#   2. cargo clippy --workspace --all-targets -- -D warnings
#   3. scripts/check_personal_paths.ps1              ← 个人路径卫生（PH-5）
#   4. scripts/check_deps.ps1                        ← §3.1 依赖白名单（W7）
#   5. scripts/check_guards.ps1                      ← §6.2 源码禁令（W7）
#   6. scripts/check_dead_contract.ps1               ← 死契约（E6）
#
# 与 .github/workflows/ci.yml 同源（CI 跑同样六项）——本地过关 ⇔ CI 过关。
# .githooks/pre-commit 调用本脚本（启用：git config core.hooksPath .githooks）。
# 全量测试（cargo test --workspace）不在此列：属收工门禁，见 AGENTS.md「约定」。
#
# 用法：  powershell -File scripts/precommit.ps1
#   应急跳过：git commit --no-verify（CI 仍会拦截）
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $RepoRoot

function Invoke-Gate {
    param([string]$Name, [scriptblock]$Body)
    Write-Host "── $Name" -ForegroundColor Cyan
    $t0 = Get-Date
    & $Body
    $code = $LASTEXITCODE
    $secs = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
    if ($code -ne 0) {
        Write-Host ""
        Write-Host "门禁未过：$Name（退出码 $code，$secs 秒）——已中止" -ForegroundColor Red
        exit $code
    }
    Write-Host "   ok  $secs 秒" -ForegroundColor DarkGray
}

Invoke-Gate 'cargo fmt --all -- --check' { cargo fmt --all -- --check }
Invoke-Gate 'cargo clippy --workspace --all-targets -- -D warnings' {
    cargo clippy --workspace --all-targets -- -D warnings
}
Invoke-Gate 'check_personal_paths.ps1' { & (Join-Path $PSScriptRoot 'check_personal_paths.ps1') }
Invoke-Gate 'check_deps.ps1' { & (Join-Path $PSScriptRoot 'check_deps.ps1') }
Invoke-Gate 'check_guards.ps1' { & (Join-Path $PSScriptRoot 'check_guards.ps1') }
Invoke-Gate 'check_dead_contract.ps1' { & (Join-Path $PSScriptRoot 'check_dead_contract.ps1') }

Write-Host ""
Write-Host "全部通过：fmt / clippy / 四守护（可以提交）" -ForegroundColor Green
exit 0
