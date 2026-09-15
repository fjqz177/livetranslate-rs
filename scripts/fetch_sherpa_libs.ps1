# 预取并解包 sherpa-onnx-sys 构建期所需的预编译静态库到仓库内 .cache/sherpa-onnx/。
# 默认从 GitHub Releases 下载归档（约 120MB），可用 -Mirror <前缀> 或 $env:SHERPA_ONNX_MIRROR
# 走任意 GitHub Release 镜像（如 https://ghproxy.com/）。
#
# 为什么解包到 .cache 而不是让 build.rs 自己解到 target/（2026-09-15 CI 实锤）：
# build.rs 的自动路径把 1.1GB 静态库解到 <target>/sherpa-onnx-prebuilt/，而 rust-cache 的
# 清理规则会把 target/ 下不属于 cargo 结构的文件全部删掉（其 cleanup.ts：非 profile 目录递归、
# 目录里的文件一律 rm）——缓存恢复后只剩空目录，build.rs 的 is_dir() 守卫照样放行（于是不重解）
# → 链接期 could not find native static library `sherpa-onnx-c-api`。改为预解包到仓库内后，
# .cargo/config.toml 的 SHERPA_ONNX_LIB_DIR 指向 extracted/lib，build.rs 命中即返回，
# 彻底不依赖 target/ 缓存；顺带 cargo clean 也不再逼出一次 ~3 分钟重解包。
#
# 幂等：extracted/lib 就位且来源档名一致即秒退；档损坏/截断在解包处即失败——旧的
# 「tar -tf 全量列举」完整性闸门已由"真解包 + 产物探针"取代（同样是冷路径一次）。
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
$archiveStem = $fileName -replace '\.tar\.bz2$', ''
$destDir = Join-Path $repoRoot ".cache\sherpa-onnx"
$dest = Join-Path $destDir $fileName
$extractDir = Join-Path $destDir 'extracted'                 # ← SHERPA_ONNX_LIB_DIR 指向其 lib/
$libProbe = Join-Path $extractDir 'lib\sherpa-onnx-c-api.lib'  # 精确探针：目录在但库被裁也要能识破
$marker = Join-Path $extractDir '.archive'                    # 记录解包来源档名（版本变更即重解）

# tar 选用：必须是能读 bz2 的那把——CI runner 上 PATH 里的 tar 是 Windows 自带 bsdtar，
# 它对 .tar.bz2 要外挂 bzip2 且实测直接失败（Child process exited）。Git for Windows 自带的
# usr\bin\tar.exe 是 GNU tar（内置 bz2），两个环境都稳。git 的安装布局有两种
# （<根>\cmd\git.exe 与 <根>\mingw64\bin\git.exe），故逐级向上找 usr\bin\tar.exe 候选。
$tar = $null
$gitCmd = Get-Command git -ErrorAction SilentlyContinue
if ($gitCmd) {
    $p = Split-Path -Parent $gitCmd.Source
    foreach ($i in 1..3) {
        $cand = Join-Path $p 'usr\bin\tar.exe'
        if (Test-Path $cand) {
            $ver = (& $cand --version 2>&1 | Select-Object -First 1)
            if ($ver -match 'GNU tar') { $tar = $cand; break }
        }
        $p = Split-Path -Parent $p
    }
}
if (-not $tar) {
    $fallback = (Get-Command tar -ErrorAction SilentlyContinue).Source
    if ($fallback -and ((& $fallback --version 2>&1 | Select-Object -First 1) -match 'GNU tar')) { $tar = $fallback }
}
if (-not $tar) { throw '找不到能读 bz2 的 GNU tar（Git for Windows 自带 <根>\usr\bin\tar.exe）' }

# ── 常态路径：已解包且来源档名一致 → 秒退 ──
if ((Test-Path $libProbe) -and (Test-Path $marker) -and ((Get-Content -Raw $marker).Trim() -eq $fileName)) {
    Write-Host "已解包，跳过：$extractDir\lib"
    exit 0
}

# ── 需要解包：先确保归档在本地（在则跳过下载） ──
if (-not (Test-Path $dest)) {
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
    Move-Item -Force "$dest.tmp" $dest
} else {
    Write-Host "归档已存在，跳过下载：$dest"
}

# ── 解包到临时目录再原子换名（不依赖 --strip-components，任何能读 bz2 的 tar 都能干） ──
# 两个 GNU tar 在 Windows 下的坑：① 反斜杠被 MSYS 版当转义吃掉（报 Cannot open，路径成了
# 一串字面反斜杠）；② 盘符冒号命中它的 remote-archive 语法（报 Cannot connect to <盘符>，
# 盘符被当成主机名）。故喂给 tar 的路径一律转正斜杠 + 加 --force-local。
$tmp = Join-Path $destDir '.extract-tmp'
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $tmp | Out-Null
Write-Host "解包 $fileName（产物约 1.1GB，冷路径 ~3 分钟）..."
& $tar --force-local -xf ($dest -replace '\\', '/') -C ($tmp -replace '\\', '/')
if ($LASTEXITCODE -ne 0) {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    throw "解包失败（档损坏/截断，或 tar 不支持 bz2）：$dest —— 删除该档重跑本脚本可自愈"
}
$staged = Join-Path $tmp $archiveStem
if (-not (Test-Path (Join-Path $staged 'lib\sherpa-onnx-c-api.lib'))) {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    throw "解包产物不完整（缺 lib\sherpa-onnx-c-api.lib）：$dest —— 删除该档重跑本脚本可自愈"
}
Remove-Item -Recurse -Force $extractDir -ErrorAction SilentlyContinue
Move-Item -Force $staged $extractDir
Set-Content -Path (Join-Path $extractDir '.archive') -Value $fileName -Encoding utf8 -NoNewline
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
Write-Host "完成：$extractDir\lib（.cargo/config.toml 的 SHERPA_ONNX_LIB_DIR 指向它）"
