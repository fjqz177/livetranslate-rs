<#
  LiveTranslate-rs 发布引擎 —— 本地与 CI 共用同一份（D-90；与 .github/workflows/release.yml 成对）

  用法：pwsh -File scripts/release.ps1 <动词>
    check     发布前校验：版本一致 / 工作区干净 / 在 main / 该提交 ci 绿 / 构建前置就位
    build     编译单 exe → --version 冒烟（取真退出码）→ 核对 exe 内嵌版本
    pack      打包 zip + sha256 边车 + build-info.txt（全部落 dist/）
    draft     建/刷新 GitHub 草稿并上传（已发布则拒绝覆盖；上传前核对 tag 指向本提交）
    verify    回读校验：从 Release 下载 zip 与边车，比对 sha256
    rehearse  演练 = check+build+pack（不发布，不碰任何 Release）
    release   本地一条龙 = check+build+pack+draft+verify
    promote   转正 draft → published（唯一不可逆；必须 -NotesFile <更新日志.md>）

  版本口径：只发正式版本——版本号必须是不带后缀的 x.y.z（预发布在 check ① 直接拒，不白等构建）

  三条铁律：
    ① 发布物必须对应 tag 指向的那个提交（draft / promote 前核对远端 tag ↔ 本地 HEAD）
    ② 已发布的字节不可改（只有草稿允许覆盖）
    ③ 转正是唯一不可逆动作，必须显式给出更新日志文件

  两个通道：
    正式 = 打 tag 推送 → CI 按序跑 check→build→pack→draft→verify（本机 release 是等价备胎）
    演练 = 本机 rehearse，或 CI 手动触发（自动进入演练模式：零 Release 写操作）

  打包内容不在本文件：zip 里装什么由 scripts/package_release.ps1 说了算（单一真源）。
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('check', 'build', 'pack', 'draft', 'verify', 'rehearse', 'release', 'promote')]
    [string]$Verb,
    [string]$NotesFile,          # 仅 promote 用：写好的更新日志 md 路径
    [switch]$Rehearsal           # 演练模式：禁写动词、跳过「主线 / ci 绿」两道闸
)
$ErrorActionPreference = 'Stop'

# ───────────────────────────── 输出与通用工具（每个只做一件事） ─────────────────────────────
function Step([string]$Title, [scriptblock]$Body) { Write-Host "── $Title" -ForegroundColor Cyan; & $Body }
function Note([string]$Text) { Write-Host "   $Text" -ForegroundColor DarkGray }
function Die([string]$Text) { Write-Host ''; Write-Host "✗ $Text" -ForegroundColor Red; exit 1 }
function Summary([string]$Md) { if ($env:GITHUB_STEP_SUMMARY) { Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value $Md -Encoding utf8 } }
function Sha256([string]$Path) { (Get-FileHash -Algorithm SHA256 $Path).Hash.ToLower() }

# gh 包装。两个坑（都实测过）：① 函数名不能叫 Gh——PowerShell 命令名大小写不敏感且函数优先于
# 外部命令，函数里的 `& gh` 会递归调回自己；② 参数一律走数组（Invoke-Gh @('run','list','-w',...)），
# 裸写 -w 会被当成包装函数自己的参数（-WarningAction 歧义）。
function Invoke-Gh([string[]]$A) {
    $out = & gh @A
    if ($LASTEXITCODE -ne 0) { Die "gh $($A -join ' ') 失败（exit $LASTEXITCODE）" }
    $out
}
function Test-GhReady {          # gh 可用 = 装了 + 已登录（CI 里由 GH_TOKEN 满足）
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { return $false }
    & gh auth status *> $null
    return ($LASTEXITCODE -eq 0)
}
function Assert-GhReady { if (-not (Test-GhReady)) { Die '需要可用的 gh（安装 + gh auth login）；不愿本机发就改用 CI：推 tag 即可' } }

# ───────────────────────────── 仓库根与常量（版本是唯一输入） ─────────────────────────────
# 向上找带 Cargo.toml 的目录：草稿在 docs/drafts、转正后在 scripts/ 都能直接跑
$Root = $PSScriptRoot
while ($Root -and -not (Test-Path (Join-Path $Root 'Cargo.toml'))) { $Root = Split-Path -Parent $Root }
if (-not $Root) { Die '找不到仓库根（向上找 Cargo.toml 失败）' }
Set-Location -LiteralPath $Root

$match = Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1
if (-not $match) { Die 'Cargo.toml 里找不到 ^version = "…"（结构变了？先对齐本脚本的版本正则）' }
$Ver      = $match.Matches[0].Groups[1].Value
$Tag      = "v$Ver"
$Artifact = "LiveTranslate-$Ver.zip"
$Zip      = "dist/$Artifact"
$Sidecar  = "$Zip.sha256"
# 变量名大小写"不敏感"（PowerShell 视作同一个名字）——路径类一律叫 *Path，内容类用别的短名，别撞车
$BuildInfoPath = 'dist/build-info.txt'
$Exe      = 'target/release/livetranslate.exe'
$InCi     = [bool]$env:GITHUB_ACTIONS
if ($env:GITHUB_EVENT_NAME -eq 'workflow_dispatch') { $Rehearsal = $true }   # CI 手动触发 = 演练通道

function Assert-Packed {
    foreach ($f in @($Zip, $Sidecar, $BuildInfoPath)) { if (-not (Test-Path $f)) { Die "缺 $f —— 先跑 pack" } }
}
# 发布物必须挂在"本提交"那个 tag 下：远端 tag 存在 ≠ 它指向你手上这份代码（旧 tag 复用即踩）
function Assert-TagPointsHere {
    $tagSha = (Invoke-Gh @('api', "repos/{owner}/{repo}/commits/$Tag", '--jq', '.sha')).Trim()
    $head   = (git rev-parse HEAD)
    if ($tagSha -ne $head) {
        Die "远端 tag $Tag 指向 $($tagSha.Substring(0,7))，本地 HEAD 是 $($head.Substring(0,7))——先对齐（改 tag 或重打），否则发布物与 tag 货不对单"
    }
    Note "远端 tag 指向本提交 $($head.Substring(0, 7))"
}

# ─────────────────────────────────── check：五道发布前校验 ───────────────────────────────────
function Do-Check {
    Step "check ① 版本一致：Cargo.toml $Ver ↔ tag $Tag" {
        # 只发正式版本：带后缀的（如 0.2.0-rc.1）在这里就拒——build ② 的版本比对只认三段数字
        if ($Ver -match '-') { Die "版本号 $Ver 带后缀——本仓不支持预发布，请改成 x.y.z 三段数字的正式版本" }
        if ($env:GITHUB_EVENT_NAME -eq 'push' -and $env:GITHUB_REF_NAME -ne $Tag) {
            Die "tag $env:GITHUB_REF_NAME 与 Cargo.toml $Ver 不一致——先对齐版本号再打 tag"
        }
        Note "tag 由版本号派生：$Tag"
    }
    Step 'check ② 工作区干净（发布物必须能对应到某个提交）' {
        if ($Rehearsal) { Note '演练模式：跳过（演练不发布）' }
        elseif ($InCi) { Note 'CI 检出即干净：跳过' }
        else {
            $dirty = @(git status --porcelain)
            if ($dirty) { Die "工作区有 $($dirty.Count) 项未提交改动——先提交；只想演练可加 -Rehearsal 跳过本检查" }
        }
    }
    Step 'check ③ 提交在主线 main 上' {
        if ($Rehearsal) { Note '演练模式：跳过' }
        elseif (-not (Test-GhReady)) { Note '⚠ 没有可用的 gh：跳过——CI 发布路径上是硬闸' }
        else {
            $sha = (git rev-parse HEAD)
            # 直接问 GitHub："main 相对这个提交多出几个提交"——ahead_by=0 即本提交已在 main 里。
            # 比查本地 origin/main 引用新鲜，也不受 github.com 直连不通影响（走 api 即可）。
            # 提交压根没推到远端时该接口 404 → 给一句人话（本地忘了 push 是最常见的坑）。
            $ahead = & gh api "repos/{owner}/{repo}/compare/main...$sha" --jq '.ahead_by' 2>$null
            if ($LASTEXITCODE -ne 0) { Die "GitHub 上查不到提交 $($sha.Substring(0, 7))——先 git push（发布只从远端主线上的提交出）" }
            if ([int]$ahead -ne 0) { Die "提交 $($sha.Substring(0, 7)) 不在 main 上（比 main 多 $ahead 个提交）——发布只从主线出" }
            Note '本提交已在 main 上'
        }
    }
    Step 'check ④ 该提交有成功的 ci 运行' {
        if ($Rehearsal) { Note '演练模式：跳过' }
        elseif (-not (Test-GhReady)) { Note '⚠ 本机没有可用的 gh（未装或未登录）：跳过——CI 发布路径上是硬闸' }
        else {
            $sha  = (git rev-parse HEAD)
            $runs = @(Invoke-Gh @('run', 'list', '-w', 'ci.yml', '-c', $sha, '-L', '10', '--json', 'headSha,conclusion') | ConvertFrom-Json)
            $ok   = @($runs | Where-Object { $_.headSha -eq $sha -and $_.conclusion -eq 'success' })
            if ($ok.Count -eq 0) { Die "提交 $($sha.Substring(0, 7)) 没有成功的 ci 运行——先让 ci 绿再发" }
            Note "命中 $($ok.Count) 次成功的 ci 运行"
        }
    }
    Step 'check ⑤ 构建前置就位（uv 工具链 / sherpa 预解包库）' {
        if ($InCi) { Note 'CI：由后续 workflow 步骤保证，跳过' }
        else {
            if (-not (Test-Path '.venv')) { Die '缺 .venv（libclang + cmake）——先跑 uv sync' }
            if (-not (Test-Path '.cache/sherpa-onnx/extracted/lib')) { Die '缺 sherpa 预解包库——先跑 scripts/fetch_sherpa_libs.ps1' }
            Note '工具链与 sherpa 库就位'
        }
    }
}

# ─────────────────────────────────── build：编译 + 冒烟 + 自检 ───────────────────────────────
function Do-Build {
    Step 'build ① 编译单 exe（--locked）' {
        cargo build --release -p lt-app --locked
        if ($LASTEXITCODE -ne 0) { Die 'cargo build 失败' }
    }
    Step 'build ② 冒烟 + 内嵌版本自检' {
        # exe 是 GUI 子系统程序：pwsh 里 `& exe` 不等待、$LASTEXITCODE 也不会被设置——
        # 那样写"冒烟"的话 exe 崩了也是绿的。必须 Start-Process -Wait -PassThru 取真退出码。
        $out = Join-Path ([IO.Path]::GetTempPath()) "livetranslate-version-$PID.txt"   # 唯一名，防并行互踩
        $p = Start-Process -FilePath $Exe -ArgumentList '--version' -NoNewWindow -Wait -PassThru -RedirectStandardOutput $out
        $banner = if (Test-Path -LiteralPath $out) { (Get-Content -LiteralPath $out -Raw).Trim() } else { '' }
        Remove-Item -LiteralPath $out -Force -ErrorAction SilentlyContinue
        if ($p.ExitCode -ne 0) { Die "exe --version 退出码 $($p.ExitCode)（冒烟失败）" }
        # exe 的文件版本字段是四段数字（build.rs 把 x.y.z 补成 x.y.z.0），比对前照样补一段
        $want = (@($Ver.Split('.') + @('0', '0', '0'))[0..3]) -join '.'
        $got  = (Get-Item $Exe).VersionInfo.FileVersion
        if ($got -ne $want) { Die "exe 内嵌版本 $got ≠ $want —— 改了版本号忘重建？" }
        Note "冒烟通过：$banner；内嵌版本 $got"
    }
}

# ─────────────────────────────────── pack：打包 + 边车 + 清单校验 ────────────────────────────
function Do-Pack {
    Step 'pack ① 打包（真源 = scripts/package_release.ps1）' {
        & (Join-Path $Root 'scripts/package_release.ps1')
        if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { Die 'package_release.ps1 失败' }
    }
    Step 'pack ② 校验产物齐备（zip 内容 + 边车 + build-info）' {
        if (-not (Test-Path $Zip)) { Die "package_release.ps1 未产出 $Zip" }
        # 发布物清单：少一件都不许发（README/LICENSE/NOTICES/OFL 是分发义务）
        $wantList = @('livetranslate.exe', 'README.txt', 'LICENSE', 'NOTICES.md', 'OFL.txt')
        $archive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $Zip).Path)
        try { $haveList = @($archive.Entries | ForEach-Object FullName) } finally { $archive.Dispose() }
        $missing = @($wantList | Where-Object { $_ -notin $haveList })
        if ($missing) { Die "zip 缺件：$($missing -join ', ')（清单在本步骤；故意增删请同步这里）" }
        # 边车：行尾必须 LF（sha256sum -c 遇 CRLF 会报 No such file），格式 = "<hash>␣␣<文件名>"
        [IO.File]::WriteAllText((Join-Path $Root $Sidecar), "$(Sha256 $Zip)  $Artifact`n")
        $buildInfo = @(
            "version: $Ver",
            "commit:  $(git rev-parse HEAD)",
            "zip:     $Artifact ($([math]::Round((Get-Item $Zip).Length / 1MB, 1)) MB) sha256 $(Sha256 $Zip)",
            "exe:     livetranslate.exe ($([math]::Round((Get-Item $Exe).Length / 1MB, 1)) MB) sha256 $(Sha256 $Exe)",
            "rustc:   $((& rustc -V) -join ' ')",
            "runner:  $env:ImageOS $env:ImageVersion".TrimEnd()
        )
        [IO.File]::WriteAllText((Join-Path $Root $BuildInfoPath), (($buildInfo -join "`n") + "`n"))    # 统一 LF
        Note "zip 五件套齐备；边车与 build-info 就绪（zip sha256 = $(Sha256 $Zip)）"
    }
}

# ─────────────────────────────────── draft：幂等建草稿 + 上传 ────────────────────────────────
function Do-Draft {
    Assert-GhReady
    Step 'draft ① 核对远端 tag 指向本提交' { Assert-TagPointsHere }
    Step "draft ② 确保 $Tag 的草稿存在（幂等）" {
        $exists = @(Invoke-Gh @('release', 'list', '--limit', '1000', '--json', 'tagName') | ConvertFrom-Json) |
                  Where-Object { $_.tagName -eq $Tag }
        if ($exists) { Note '草稿已存在（上次运行残留）——沿用' }
        else {
            Invoke-Gh @('release', 'create', $Tag, '--draft', '--verify-tag', '--title', $Tag) | Out-Null
            Note "已创建草稿（Releases → $Tag，尚未发布）"
        }
    }
    Step 'draft ③ 上传 zip + 边车（已发布 = 字节冻结，拒绝覆盖）' {
        Assert-Packed
        if ((Invoke-Gh @('release', 'view', $Tag, '--json', 'isDraft', '--jq', '.isDraft')) -ne 'true') {
            Die "$Tag 已发布——已发布的字节不可改；要重发请把版本号抬上去（或先 gh release delete $Tag）"
        }
        Invoke-Gh @('release', 'upload', '--clobber', $Tag, $Zip, $Sidecar) | Out-Null
        Note "已上传 $Artifact 与 $Artifact.sha256"
    }
    $buildInfo = (Get-Content -LiteralPath $BuildInfoPath -Raw).Trim()
    Summary (@'
## 发布草稿已就绪：{0}

```text
{1}
```

| 人工步骤 | 命令 |
|---|---|
| 回读校验 | `pwsh -File scripts/release.ps1 verify` |
| 转正 | `pwsh -File scripts/release.ps1 promote -NotesFile <日志.md>` |

更新日志真源（与应用内「更新日志」页同源）：`assets/i18n/CHANGELOG_zh.md` / `_en.md`
'@ -f $Tag, $buildInfo)
}

# ─────────────────────────────────── verify：回读比对 sha256 ─────────────────────────────────
function Do-Verify([switch]$Strict) {      # Strict：release 一条龙用——本地字节必须与远端一致
    Assert-GhReady
    Step "verify 从 Release 回读 $Tag 并比对 sha256" {
        $tmp = 'dist/.reread'
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        Invoke-Gh @('release', 'download', $Tag, '--pattern', '*.zip', '--pattern', '*.sha256', '--dir', $tmp, '--clobber') | Out-Null
        $gotZip = Join-Path $tmp $Artifact
        $gotSum = "$gotZip.sha256"
        if (-not (Test-Path $gotZip)) { Die "Release 上找不到 $Artifact" }
        if (-not (Test-Path $gotSum)) { Die "Release 上找不到 $Artifact.sha256（资产不完整？先补跑 draft）" }
        $want = (Get-Content -LiteralPath $gotSum -Raw).Trim().Split(' ')[0]     # 远端自洽：zip ↔ 边车
        $have = Sha256 $gotZip
        if ($want -ne $have) { Die "远端 zip 与远端边车不一致：$have ≠ $want" }
        if (Test-Path $Sidecar) {
            $local = (Get-Content -LiteralPath $Sidecar -Raw).Trim().Split(' ')[0]
            if ($local -ne $have) {
                if ($Strict) { Die "本地 $Artifact 与远端不一致：本地 $local ≠ 远端 $have（上传错了对象或本地被污染）" }
                Note "⚠ 本地字节与远端不同（本地 $local）——两边各自构建过时属正常"
            }
        } elseif ($Strict) { Die "缺本地边车 $Sidecar —— 一条龙里应当刚 pack 过" }
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        Note "一致：$have"
        Summary "verify 通过：Release $Tag 的 zip 与边车 sha256 自洽（$have）"
    }
}

# ─────────────────────────────────── promote：转正（唯一不可逆） ────────────────────────────
function Do-Promote {
    Assert-GhReady
    if (-not $NotesFile) { Die 'promote 必须给 -NotesFile <写好的更新日志.md>（真源 assets/i18n/CHANGELOG_*.md）' }
    if (-not (Test-Path -LiteralPath $NotesFile)) { Die "找不到日志文件：$NotesFile" }
    Step 'promote ① 前置核对（是草稿 / 资产齐 / tag 指向本提交）' {
        if ((Invoke-Gh @('release', 'view', $Tag, '--json', 'isDraft', '--jq', '.isDraft')) -ne 'true') { Die "$Tag 不是草稿（已转正或不存在）" }
        $count = [int](Invoke-Gh @('release', 'view', $Tag, '--json', 'assets', '--jq', '.assets | length'))
        if ($count -lt 2) { Die "草稿资产只有 $count 个（应有 zip + sha256）——先补跑 draft，别把残缺草稿发出去" }
        Assert-TagPointsHere
    }
    Step "promote ② 转正 $Tag（draft → published）" {
        Invoke-Gh @('release', 'edit', $Tag, '--notes-file', $NotesFile, '--draft=false') | Out-Null
        Note '已转正：GitHub Releases 上现在可见'
    }
}

# ─────────────────────────────────────────── 派发 ───────────────────────────────────────────
if (-not $Verb) {
    Write-Host '用法：pwsh -File scripts/release.ps1 <动词>'
    Write-Host '  check | build | pack | draft | verify | rehearse | release | promote（-NotesFile 仅 promote 用）'
    Write-Host '  演练用 rehearse，或给任意只读动词加 -Rehearsal'
    exit 2
}
if ($Rehearsal -and $Verb -in @('draft', 'promote', 'release')) {
    Die "演练模式禁止 $Verb（它会写 GitHub）——演练只允许 check / build / pack / verify"
}

switch ($Verb) {
    'check'    { Do-Check }
    'build'    { Do-Build }
    'pack'     { Do-Pack }
    'draft'    { Do-Draft }
    'verify'   { Do-Verify }
    'rehearse' { $Rehearsal = $true; Do-Check; Do-Build; Do-Pack; Note '演练到此为止：未创建 / 修改任何 Release' }
    'release'  { Do-Check; Do-Build; Do-Pack; Do-Draft; Do-Verify -Strict }
    'promote'  { Do-Promote }
}
Write-Host ''
Write-Host "✓ $Verb 完成（$Tag）" -ForegroundColor Green
if ($InCi) { Summary $(if ($Rehearsal) { "## 演练通道：$Verb 完成（未创建 / 修改任何 Release）" } else { "## 发布引擎：$Verb 完成（$Tag）" }) }
