# 个人路径卫生守护（docs/archive/path-hygiene.md PH-5）
# 用法：powershell -File scripts/check_personal_paths.ps1
# 扫描全部已跟踪文本文件，命中即退出码 1（提交前自查用，配合 AGENTS.md 约定）。
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

$exclude = '\.(png|jpg|ico|icns|onnx|dll|br|ttf|otf|woff2?|zip|gz)$'
$files = @(git ls-files) | Where-Object { $_ -and ($_ -notmatch $exclude) }

$user = $env:USERNAME
$hits = 0
$warns = 0
foreach ($f in $files) {
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
  Write-Output "FAIL: $hits 处命中个人/非白名单路径（处置见 docs/archive/path-hygiene.md）"
  exit 1
}
Write-Output "OK: 已跟踪文本文件零个人路径命中（archive Tier2 告警 $warns 处不阻断）"
exit 0
