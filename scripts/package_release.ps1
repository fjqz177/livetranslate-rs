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
# 前置：  先 `cargo build --release -p lt-app`（exe 缺 = exit 1 并提示）；版本号取
#         cargo metadata 的 lt-app 版本（= workspace 版本，与 VERSIONINFO 同源；缺 = exit 1）。
#         CI 同源调用见 scripts/release.ps1 pack。
# 退出码：0 = 打包完成；1 = exe 缺失或版本解析失败 / zip 缺件。
# 产物 / 副作用：只写 dist/ 两样——staging 目录 dist/LiveTranslate-<version>/（打包后
#         **保留不删**）与 zip（已存在则**先删再建**）；不动仓库内任何文件。
# 分工：  zip 五件套清单的**唯一真源在本文件**（打包后自检，S4-③ 收拢——release.ps1
#         pack 不再各养一份）；release.ps1 pack 只查 zip+边车+build-info 三件存在。
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

# ── 版本号（S4-③ 收拢：cargo metadata 单一来源，不再各养一份 ^version 正则；
#    workspace 版本随成员 lt-app 暴露，与 VERSIONINFO/打包产物名同源） ──
$metaRaw = & cargo metadata --no-deps --format-version 1 --offline 2>$null
if ($LASTEXITCODE -ne 0 -or -not $metaRaw) {
    Write-Host "cargo metadata 失败——版本号无从解析（在仓库根内跑？）" -ForegroundColor Red
    exit 1
}
$version = ($metaRaw | ConvertFrom-Json).packages |
    Where-Object { $_.name -eq 'lt-app' } |
    Select-Object -First 1 -ExpandProperty version
if (-not $version) { Write-Host "cargo metadata 里找不到 lt-app 版本" -ForegroundColor Red; exit 1 }
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

# ── 五件套自检（S4-③：清单唯一真源在此——缺一件即 exit 1，不劳 release.ps1 复查） ──
$wantList = @('livetranslate.exe', 'README.txt', 'LICENSE', 'NOTICES.md', 'OFL.txt')
$archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $zip).Path)
try { $haveList = @($archive.Entries | ForEach-Object FullName) } finally { $archive.Dispose() }
$missing = @($wantList | Where-Object { $_ -notin $haveList })
if ($missing) {
    Write-Host "zip 缺件：$($missing -join ', ')（五件套清单在本文件顶部循环+本表，改清单两处同步）" -ForegroundColor Red
    exit 1
}

# ── dist/ 旧版残留提示（S4-①：不自动删——只张目，处置权在维护者） ──
$stale = @()
$distDir = Join-Path $RepoRoot 'dist'
$stale += @(Get-ChildItem -Path $distDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'LiveTranslate-*.zip' -and $_.Name -ne "LiveTranslate-$version.zip" } |
    ForEach-Object { $_.Name })
$stale += @(Get-ChildItem -Path $distDir -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'LiveTranslate-*' -and $_.Name -ne "LiveTranslate-$version" } |
    ForEach-Object { $_.Name + '/' })
if ($stale.Count -gt 0) {
    Write-Host "⚠ dist/ 有旧版残留（未自动删）：$($stale -join '、')" -ForegroundColor Yellow
}

$zipSize = (Get-Item $zip).Length
$exeSize = (Get-Item $exe).Length
Write-Host "打包完成：" -ForegroundColor Green
Write-Host "  产物  $zip  ($([math]::Round($zipSize / 1MB, 1)) MB)"
Write-Host "  exe   $([math]::Round($exeSize / 1MB, 1)) MB"
exit 0
