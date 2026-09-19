# 个人路径卫生 + 文本卫生守护（path-hygiene PH-5 + text-hygiene ADR-20）
# 用法：pwsh -File scripts/check_personal_paths.ps1
#   （无参数；precommit 第 2 项自动调用）
# 扫描全部已跟踪文本文件：
#   个人路径两档（PH-5）：Tier1 用户名/实名 Users 路径硬失败（任何文件含归档）；
#     Tier2 白名单外盘符路径硬失败（archive 内仅 WARN 计数，不拦）。
#   文本卫生三查（ADR-20）：① 头 3 字节 UTF-8 BOM；② 严格 UTF-8 解码（非法字节
#     throw——Get-Content 容错路线只会静默替换 U+FFFD，逮不住）；③ index 行尾
#     i/crlf|i/mixed（git 记录必须全 LF；二进制 i/-text 不查）。
# 参数：无（$PSScriptRoot 自定位仓库根）。
# 前置：pwsh 7 + git；扫描面 = git ls-files 已跟踪文件，排除二进制后缀
#   png/jpg/ico/icns/onnx/dll/br/bin/ttf/otf/woff2/zip/gz（未跟踪/忽略文件不扫）。
# 退出码：0 = 全过（archive 内 Tier2 仅 WARN 计数，不拦）；1 = 任一命中（逐条 FAIL）。
# 产物 / 副作用：无（只读）。
# 详见：docs/archive/path-hygiene.md（PH-5、§4.4 豁免规则）+
#   docs/archive/text-hygiene.md（ADR-20 三查规格与验收）。
#
# Tier1 硬失败（任何文件，含归档）：
#   - 本机用户名动态匹配（$env:USERNAME，字面不入库）
#   - 实名 Users 绝对路径：C:/Users/<名字>…（占位符 < 开头豁免：
#     <你的用户名>/<u>/<原开发者>，见 path-hygiene §4.4）
# Tier2 硬失败（docs/archive/* 之外；归档为只读决策史，仅 WARN）：
#   - 白名单外盘符路径。白名单 = URL 协议、C:\Windows 系、C:\Program Files
#     （系统级确定性路径豁免，path-hygiene §4.4）+ 占位符 Users 路径。
# 注意：源码里的 C:\\Windows（Rust 字符串转义双反斜杠）以 [/\\]{1,2} 兼容。
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath (Join-Path $PSScriptRoot '..')

$exclude = '\.(png|jpg|ico|icns|onnx|dll|br|bin|ttf|otf|woff2?|zip|gz)$'
$files = @(git ls-files) | Where-Object { $_ -and ($_ -notmatch $exclude) }

# index 行尾映射（三查③）：ls-files --eol 按 TAB 分段取「元数据列 → 路径」，文件名含空格也安全
$eolMap = @{}
foreach ($l in @(git ls-files --eol)) {
  $pair = $l -split "`t", 2
  if ($pair.Count -eq 2) { $eolMap[$pair[1].Trim()] = (($pair[0] -split '\s+')[0]) }
}
$strict = [System.Text.UTF8Encoding]::new($false, $true)   # throwOnInvalidBytes：非法字节即 throw

$user = $env:USERNAME
$hits = 0
$warns = 0
foreach ($f in $files) {
  # ── 文本卫生三查（ADR-20；字节级，先于行级检查；空文件天然全过）──
  try { $bytes = [System.IO.File]::ReadAllBytes((Join-Path (Get-Location).Path $f)) }  # .NET API 走进程 cwd，显式拼仓库根
  catch { continue }   # 读取失败对齐原 Get-Content SilentlyContinue 语义（跳过）
  if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
    Write-Output ("FAIL {0}: UTF-8 BOM（头三字节 EF BB BF）——全仓文本一律无 BOM（ADR-20）" -f $f)
    $hits++
  }
  try { [void]$strict.GetString($bytes) }
  catch [System.Text.DecoderFallbackException] {
    Write-Output ("FAIL {0}: 非法 UTF-8（严格解码失败，疑非 UTF-8 编码入库）" -f $f)
    $hits++
  }
  $ieol = $eolMap[$f]
  if ($ieol -eq 'i/crlf' -or $ieol -eq 'i/mixed') {
    Write-Output ("FAIL {0}: index 行尾 {1}——git 记录必须全 LF（ADR-20）" -f $f, $ieol)
    $hits++
  }
  $lines = Get-Content -LiteralPath $f -Encoding UTF8 -ErrorAction SilentlyContinue
  if ($null -eq $lines) { continue }
  for ($i = 0; $i -lt $lines.Count; $i++) {
    $line = $lines[$i]
    $n = $i + 1
    $isArchive = $f -match '(^|[\\/])docs[\\/]archive[\\/]'
    # Tier1：用户名 / 实名 Users 路径（占位符 < 开头豁免）
    $t1 = ($user -and ($line -cmatch [regex]::Escape($user))) -or ($line -imatch 'C:[/\\]Users[/\\](?!<)')
    if ($t1) {
      Write-Output ("FAIL {0}:{1}: {2}" -f $f, $n, $line.Trim())
      $hits++
      continue
    }
    # Tier2：先剥 URL / 系统级白名单 / 占位符 Users 路径，再看剩余盘符路径
    $scrub = $line -replace '(?i)\b(?:https?|ftp)://\S+', '' -replace '(?i)\bC:[/\\]{1,2}(?:Windows|Program Files)\b\S*', '' -replace '(?i)\bC:[/\\]{1,2}Users[/\\]{1,2}<[^>>]*>', ''
    if ($scrub -imatch '\b[A-Za-z]:[/\\]') {
      if ($isArchive) {
        Write-Output ("WARN(archive) {0}:{1}: {2}" -f $f, $n, $line.Trim())
        $warns++
      } else {
        Write-Output ("FAIL {0}:{1}: {2}" -f $f, $n, $line.Trim())
        $hits++
      }
    }
  }
}
if ($hits -gt 0) {
  Write-Output "FAIL: $hits 处命中（个人路径 + 文本卫生三查；处置见 docs/archive/path-hygiene.md 与 docs/archive/text-hygiene.md）"
  exit 1
}
Write-Output "OK: 已跟踪文本文件零个人路径命中、文本卫生三查全过（archive Tier2 告警 $warns 处不阻断）"
exit 0
