# ============================================================
# 提交前门禁（2026-09-11 起；2026-09-14 增第七项）：单一入口，七项顺序执行、任一失败即停
#
#   1. cargo fmt --all -- --check                    ← 格式（秒级）
#   2. scripts/check_personal_paths.ps1              ← 个人路径卫生（PH-5）
#   3. scripts/check_deps.ps1                        ← §3.1 依赖白名单（W7）
#   4. scripts/check_guards.ps1                      ← §6.2 源码禁令（W7）
#   5. scripts/check_dead_contract.ps1               ← 死契约（E6）
#   6. scripts/check_agents_health.ps1               ← 总纲健康（ADR-15：两档预算/路径/G-编号/看板）
#   7. cargo clippy --workspace --all-targets --locked -- -D warnings   ← 编译级检查（分钟级，垫底）
#
# 顺序 = 便宜先死（D-88）：秒级文本扫描全过才付 clippy 的编译等待；
# 任一失败即停，语义与旧序（clippy 居第 2）完全等价，纯延迟优化。
#
# 与 .github/workflows/ci.yml 同源（CI 跑同样七项）——本地过关 ⇔ CI 过关。
# .githooks/pre-commit 调用本脚本（启用：git config core.hooksPath .githooks）。
# 全量测试（cargo test --workspace）不在此列：属收工门禁，见 AGENTS.md §2「命令与门禁」。
#
# 用法：  pwsh -File scripts/precommit.ps1（本仓 pwsh-only，ADR-16）
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
Invoke-Gate 'check_personal_paths.ps1' { & (Join-Path $PSScriptRoot 'check_personal_paths.ps1') }
Invoke-Gate 'check_deps.ps1' { & (Join-Path $PSScriptRoot 'check_deps.ps1') }
Invoke-Gate 'check_guards.ps1' { & (Join-Path $PSScriptRoot 'check_guards.ps1') }
Invoke-Gate 'check_dead_contract.ps1' { & (Join-Path $PSScriptRoot 'check_dead_contract.ps1') }
Invoke-Gate 'check_agents_health.ps1' { & (Join-Path $PSScriptRoot 'check_agents_health.ps1') }
Invoke-Gate 'cargo clippy --workspace --all-targets --locked -- -D warnings' {
    # --locked（D-88 复审 P2）：clippy 是 CI 中第一个做依赖解析的 cargo 命令，无 --locked
    # 会静默重写失同步的 Cargo.lock，架空其后 test/build 的 --locked 与「锁是入库真源」不变式
    cargo clippy --workspace --all-targets --locked -- -D warnings
}

Write-Host ""
Write-Host "全部通过：fmt / clippy / 五守护（可以提交）" -ForegroundColor Green
exit 0
