# ============================================================
# 本地 zip 打包（D-18：暂不公开发布——本地 zip + 直进主界面 D-19）
#
# 产物：dist/LiveTranslate-<version>.zip
#   ├─ livetranslate.exe   （release 单 exe；要求先 `cargo build --release -p lt-app`）
#   ├─ README.txt          （简版用户手册）
#   ├─ LICENSE             （MIT）
#   ├─ NOTICES.md          （第三方组件与许可）
#   └─ OFL.txt             （三内嵌字体许可）
#
# 用法：  pwsh -File scripts/package_release.ps1
# 参数：  -RepoRoot（默认 = 脚本上级目录，一般不用传）
# 前置：  先 `cargo build --release -p lt-app`（exe 缺 = exit 1 并提示）；版本号取仓根
#         Cargo.toml 首个 ^version 行（缺 = exit 1）。CI 同源调用见 scripts/release.ps1 pack。
# 退出码：0 = 打包完成；1 = exe 缺失或版本解析失败。
# 产物 / 副作用：只写 dist/ 两样——staging 目录 dist/LiveTranslate-<version>/（打包后
#         **保留不删**）与 zip（已存在则**先删再建**）；不动仓库内任何文件。
# 分工：  README.txt/LICENSE/NOTICES.md/OFL.txt 四件「存在才拷入、缺件不失败」；
#         zip 五件套的强制校验属 scripts/release.ps1 pack 的职责（发布链以此为准）。
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

# ── 版本号（Cargo.toml workspace 前缀；打包产物名与 VERSIONINFO 同源） ──
$workspaceToml = Join-Path $RepoRoot 'Cargo.toml'
$versionLine = Get-Content $workspaceToml | Select-String '^version\s*=\s*"(.*)"' | Select-Object -First 1
if (-not $versionLine) { Write-Host "Cargo.toml 未找到版本号" -ForegroundColor Red; exit 1 }
$version = $versionLine.Matches[0].Groups[1].Value
Write-Host "版本：$version"

# ── 前置校验：release exe 必须已构建 ──
$exe = Join-Path $RepoRoot 'target\release\livetranslate.exe'
if (-not (Test-Path $exe)) {
    Write-Host "未找到 $exe —— 先运行：cargo build --release -p lt-app" -ForegroundColor Red
    exit 1
}

# ── 组装暂存目录 ──
$staging = Join-Path $RepoRoot "dist\LiveTranslate-$version"
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Path $staging -Force | Out-Null

Copy-Item $exe (Join-Path $staging 'livetranslate.exe')
foreach ($doc in @('README.txt', 'LICENSE', 'NOTICES.md')) {
    $src = Join-Path $RepoRoot $doc
    if (Test-Path $src) { Copy-Item $src $staging }
}
# 三字体 OFL 许可（内嵌字体的再分发义务）
$ofl = Join-Path $RepoRoot 'assets\fonts\OFL.txt'
if (Test-Path $ofl) { Copy-Item $ofl (Join-Path $staging 'OFL.txt') }

# ── 压缩 ──
$zip = Join-Path $RepoRoot "dist\LiveTranslate-$version.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip -CompressionLevel Optimal

$zipSize = (Get-Item $zip).Length
$exeSize = (Get-Item $exe).Length
Write-Host "打包完成：" -ForegroundColor Green
Write-Host "  产物  $zip  ($([math]::Round($zipSize / 1MB, 1)) MB)"
Write-Host "  exe   $([math]::Round($exeSize / 1MB, 1)) MB"
exit 0
