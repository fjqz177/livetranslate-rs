# 预取 sherpa-onnx-sys 构建期所需的预编译静态库到仓库内 .cache/sherpa-onnx/。
# 默认从 GitHub Releases 下载（约 120MB），可用 -Mirror <前缀> 或 $env:SHERPA_ONNX_MIRROR
# 走任意 GitHub Release 镜像（如 https://ghproxy.com/）。.cargo/config.toml 的
# SHERPA_ONNX_ARCHIVE_DIR 指向该目录，cargo 构建时从此本地复制：全程零联网、
# cargo clean 也不丢失缓存。注意 build.rs 对 ARCHIVE_DIR 缺失不回落联网（硬报错），
# 首次构建前必须运行本脚本一次。
#
# 用法（PowerShell）：
#   scripts\fetch_sherpa_libs.ps1
#   scripts\fetch_sherpa_libs.ps1 -Mirror https://ghproxy.com/
#   $env:SHERPA_ONNX_MIRROR = "https://ghproxy.com/"; scripts\fetch_sherpa_libs.ps1
param(
    [string]$Mirror = $env:SHERPA_ONNX_MIRROR
)
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$lock = Get-Content -Raw "$repoRoot\Cargo.lock"
$m = [regex]::Match($lock, 'name = "sherpa-onnx-sys"\s+version = "([^"]+)"')
if (-not $m.Success) { throw 'Cargo.lock 中找不到 sherpa-onnx-sys 版本' }
$ver = $m.Groups[1].Value

$fileName = "sherpa-onnx-v$ver-win-x64-static-MT-Release-lib.tar.bz2"
$destDir = Join-Path $repoRoot ".cache\sherpa-onnx"
$dest = Join-Path $destDir $fileName
if (Test-Path $dest) {
    Write-Host "已存在，跳过下载：$dest"
    exit 0
}

$baseUrl = "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$ver"
if ($Mirror) {
    $Mirror = $Mirror.TrimEnd('/')
    $url = "$Mirror$baseUrl/$fileName"
} else {
    $url = "$baseUrl/$fileName"
}

New-Item -ItemType Directory -Force $destDir | Out-Null
Write-Host "下载 sherpa-onnx 预编译静态库 v$ver（约 120MB）到 $destDir ..."
curl.exe -L -C - --fail --retry 3 --connect-timeout 30 -o "$dest.tmp" "$url"
if ($LASTEXITCODE -ne 0) { throw "下载失败：$url（可试 -Mirror <镜像前缀>，或设置 HTTP_PROXY/HTTPS_PROXY 走代理）" }
# 完整性闸门（D-88 复审）：全量列举校验 tar/bz2 结构——拦下载中断与镜像假档（HTTP 200 非 bzip2），
# 防坏档进 actions/cache 被缓存键固化放大。代价 ~2 分钟全量解压，只在冷路径（刚下载完）付出。
tar -tf "$dest.tmp" > $null
if ($LASTEXITCODE -ne 0) { Remove-Item -Force "$dest.tmp"; throw "下载档完整性校验失败（tar 无法列举，疑似截断或假档）：$dest.tmp" }
Move-Item -Force "$dest.tmp" $dest
Write-Host "完成：$dest"
