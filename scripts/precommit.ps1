# ============================================================
# 提交前门禁（2026-09-11 起；2026-09-23 ADR-21 守护瘦身七项并三项）：单一入口，三项顺序执行、任一失败即停
#
#   1. cargo fmt --all -- --check                    ← 格式（秒级）
#   2. scripts/check_agents_health.ps1               ← 总纲健康（ADR-15 起家：路径/G-编号/看板 + D-93 引用网扩面
#                                                     与归档对账 + ADR-19 脚本登记面；断言 2~8）
#   3. cargo clippy --workspace --all-targets --locked -- -D warnings   ← 编译级检查（分钟级，垫底；
#                                                     含 clippy.toml disallowed 禁令——原 check_guards
#                                                     禁令 1/2/4；依赖白名单 topology.rs、文本卫生
#                                                     repo_hygiene.rs、契约纯度 contract_purity.rs
#                                                     均并入 cargo test，ADR-21）
#
# 顺序 = 便宜先死（D-88）：秒级文本扫描全过才付 clippy 的编译等待；
# 任一失败即停，语义与旧序（clippy 居第 2）完全等价，纯延迟优化。
#
# 与 .github/workflows/ci.yml 同源（CI 跑同样三项）——本地过关 ⇔ CI 过关。
# .githooks/pre-commit 调用本脚本（启用：git config core.hooksPath .githooks）。
# 全量测试（cargo test --workspace）不在此列：属收工门禁，见 AGENTS.md §2「命令与门禁」。
#
# 用法：  pwsh -File scripts/precommit.ps1（本仓 pwsh-only，ADR-16）
# 参数：  -RepoRoot（默认 = 脚本上级目录，一般不用传）
# 前置：  pwsh 7 + rust-toolchain.toml 工具链 + .venv（libclang/cmake，G-5）+
#         .cache/sherpa-onnx（clippy 需全量编译，fetch_sherpa_libs.ps1 预取）。
# 退出码：0 = 三项全过；非 0 = 透传首个失败项自身的退出码（通常 1；clippy 编译错可
#         为 101），任一失败即停不跑后续项。
#   应急跳过：git commit --no-verify（CI 仍会拦截）
# 产物 / 副作用：无仓库内文件改动（target/ 编译缓存正常增长）。
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
    try { & $Body } catch {
        Write-Host ""
        Write-Host "门禁未过：$Name（未能执行：$($_.Exception.Message)）——已中止" -ForegroundColor Red
        exit 1
    }
    $code = $LASTEXITCODE
    if ($null -eq $code) { $code = 1 }   # 命令未落退出码（如未安装）时兜底，保证失败横幅与退出码出现
    $secs = [math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
    if ($code -ne 0) {
        Write-Host ""
        Write-Host "门禁未过：$Name（退出码 $code，$secs 秒）——已中止" -ForegroundColor Red
        exit $code
    }
    Write-Host "   ok  $secs 秒" -ForegroundColor DarkGray
}

Invoke-Gate 'cargo fmt --all -- --check' { cargo fmt --all -- --check }
Invoke-Gate 'check_agents_health.ps1' { & (Join-Path $PSScriptRoot 'check_agents_health.ps1') }
Invoke-Gate 'cargo clippy --workspace --all-targets --locked -- -D warnings' {
    # --locked（D-88 复审 P2）：clippy 是 CI 中第一个做依赖解析的 cargo 命令，无 --locked
    # 会静默重写失同步的 Cargo.lock，架空其后 test/build 的 --locked 与「锁是入库真源」不变式
    cargo clippy --workspace --all-targets --locked -- -D warnings
}

Write-Host ""
Write-Host "全部通过：fmt / 总纲健康 / clippy（可以提交）" -ForegroundColor Green
exit 0
