# ============================================================
# 源码禁令守护（架构 2.0 W7，docs/architecture-v2.md §6.2 禁令表）
#
# 断言五组源码禁令全部被白名单封口：违规（白名单外命中）→ 非零退出。
# 白名单 = INV3 线程出生唯一 / INV1 proxy 生产者唯一的实测例外编目 + 测试
# 夹具目录（tests/ 与 cfg(test) 内的 spawn 是测试自身基建，非生产线程树）。
# 字符串协议禁令（INV9）与 panic hook 位置（§3.2.1）零白名单。
#
# 用法：  powershell -File scripts/check_guards.ps1
# ============================================================

param(
    [string]$RepoRoot = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'

# 扫描面：crates/ 全部 .rs（含 tests/、examples/；内部无目标目录）
$rsFiles = Get-ChildItem -Path (Join-Path $RepoRoot 'crates') -Recurse -Filter *.rs -File

function Test-Whitelisted([string]$path, [string[]]$whitelist) {
    foreach ($w in $whitelist) {
        if ($path -like $w) { return $true }
    }
    return $false
}

$violations = @()

# ── 禁令 1：裸线程出生（INV3：全仓合法出生点 = Supervisor::spawn） ──
# 白名单 = 监督器本体 + 已记录例外（I/O 泵/子线程 join/测试夹具）
$SpawnWhite = @(
    '*\lt-orchestrator\src\supervisor.rs',   # 监督器本体（monitor + spawn）
    '*\lt-ui\src\tray.rs',                   # D-35 托盘专用线程（A5）
    '*\lt-asr\src\client.rs',                # worker stderr/reader I/O 泵（随子进程生命周期，join/EOF 收敛）
    '*\lt-audio\src\audio\wasapi_win.rs',    # wasapi 读环（W3 表外补充：依赖方向所限，AudioStatus 边沿事件承接可观测性）
    '*\lt-translate\src\bench.rs',           # 基准内层线程（lt-bench 监督器包装 + join 兜底，W5/R22）
    '*\lt-orchestrator\src\download.rs',     # 下载 worker 子线程（父会话线程 join，W7 注记）
    '*\lt-asr\src\bin\fake_asr_worker.rs',   # 测试用假 worker
    '*\lt-audio\src\audio\capture.rs',       # 仅 cfg(test) 夹具（生产侧无 spawn）
    '*\lt-orchestrator\src\probe.rs',        # 仅 cfg(test) 夹具（探测单测的 mock 服务端 accept 线程，D-85）
    '*\lt-download\src\lib.rs',              # 仅 #[ignore] 网络探针
    '*\tests\*',                             # 集成测试夹具
    '*\lt-translate\src\lib.rs'
)
foreach ($f in $rsFiles) {
    if (Test-Whitelisted $f.FullName $SpawnWhite) { continue }
    foreach ($ln in (Get-Content $f.FullName)) {
        if ($ln -match 'thread::(spawn|Builder)') {
            $violations += "裸线程出生：$($f.FullName.Substring($RepoRoot.Length + 1))  → $ln.Trim()"
        }
    }
}

# ── 禁令 2：直发 proxy（INV1：生产者仅动脉桥 + 已记录例外） ──
$ProxyWhite = @(
    '*\lt-app\src\artery.rs',    # 动脉桥（唯一正式生产者）
    '*\lt-app\src\shell.rs',     # Cmd::Stop → AppCommand::Quit（W4 注记：about_to_wait 无 ActiveEventLoop，持构造期代理）
    '*\lt-app\src\singleton.rs', # SecondInstance 激活（W6 注记：系统消息 WIN32 契约外通道）
    '*\lt-ui\src\app.rs'         # D-35 托盘子线程回流闭包（A5 专用协议）
)
foreach ($f in $rsFiles) {
    if (Test-Whitelisted $f.FullName $ProxyWhite) { continue }
    foreach ($ln in (Get-Content $f.FullName)) {
        if ($ln -match '\.send_event\(') {
            $violations += "直发 proxy：$($f.FullName.Substring($RepoRoot.Length + 1))  → $ln.Trim()"
        }
    }
}

# ── 禁令 3：字符串协议（INV9：分隔符/哨兵/前缀恢复一律禁止） ── 零白名单
$StringWhite = @()
foreach ($f in $rsFiles) {
    if (Test-Whitelisted $f.FullName $StringWhite) { continue }
    foreach ($ln in (Get-Content $f.FullName)) {
        $hit = $false
        if ($ln.Contains("split_once('\\t')") -or $ln.Contains('split("\t")')) { $hit = $true }
        elseif ($ln.Contains('__DONE__')) { $hit = $true }
        elseif ($ln.Contains('Menu("')) { $hit = $true }
        elseif ($ln -match '(strip_prefix|starts_with|remove_prefix|trim_start_matches)\("\[net\] ') { $hit = $true }
        if ($hit) {
            $violations += "字符串协议：$($f.FullName.Substring($RepoRoot.Length + 1))  → $ln.Trim()"
        }
    }
}

# ── 禁令 4：panic hook 位置（§3.2.1：hook 安装构造仅限 lt-app 且单一） ──
# 白名单 = main.rs（入口安装）+ panic_hook.rs（install 函数所在——hook
# 构造与其模块内聚，main 仅调用 install()，全仓无第二安装点）
foreach ($f in $rsFiles) {
    if ($f.FullName -like '*\lt-app\src\main.rs') { continue }
    if ($f.FullName -like '*\lt-app\src\panic_hook.rs') { continue }
    foreach ($ln in (Get-Content $f.FullName)) {
        if ($ln -match 'panic::set_hook') {
            $violations += "panic hook 越位：$($f.FullName.Substring($RepoRoot.Length + 1))"
        }
    }
}

# ── 禁令 5：契约旁路（lt-proto 契约面禁止 serde_json::Value 载荷） ──
# 白名单 = settings.rs（ModelConfig.overrides/extra_body 为 LLM API 透传的
# 数据契约字段，非命令/事件走私）；检查面收窄至 events.rs（命令/事件面）
foreach ($f in $rsFiles) {
    if ($f.FullName -notlike '*\lt-proto\src\events.rs') { continue }
    foreach ($ln in (Get-Content $f.FullName)) {
        if ($ln -match 'serde_json::Value') {
            $violations += "契约旁路：$($f.FullName.Substring($RepoRoot.Length + 1))  → $ln.Trim()"
        }
    }
}

if ($violations.Count -gt 0) {
    Write-Host "check_guards: 失败（$($violations.Count) 条禁令违例）" -ForegroundColor Red
    $violations | ForEach-Object { Write-Host "  VIOLATION: $_" -ForegroundColor Red }
    exit 1
}
Write-Host "check_guards: OK（spawn/proxy/字符串协议/panic hook/契约旁路 五组禁令全部闭合）" -ForegroundColor Green
exit 0
