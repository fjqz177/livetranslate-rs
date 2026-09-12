# 硬编码路径全面排查与清理计划（path-hygiene）

> 状态：**调研完成、计划定稿，待施工**（2026-09-09）。行号均以 commit `64e3c1d` 为准，施工时如有漂移以符号名为锚。
> 发起：用户规则（2026-09-09）——**入库文件不写个人电脑绝对路径**，相对路径或系统级确定性路径（`C:\Windows`、`C:\Program Files\LLVM` 等标准位置）之外一律禁止；亦不暴露个人机器其他信息。起因：参考图重拍时把本机 venv 绝对路径写进了脚本 docstring 与 AGENTS.md（已在 `5ccdefb` 修复该两处 + AGENTS 冒烟示例行）。
> 关联：`docs/archive/data-lifecycle.md` ③（LIBCLANG_PATH 相对路径化已根治，本计划是其同类问题的全量收口）。

## 1. 目标与非目标

**目标**

1. 仓库工作区（HEAD）**零 P0**（个人路径：含用户名、指向本机私有目录布局）与**零 P1**（本机盘符布局痕迹）。
2. 测试/探针**机器无关化**且保留可用性：真实模型探针改为环境变量驱动（沿用 `LT_WHISPER_MODEL` 既有先例），任何机器上 `cargo test --workspace` 基线不回退。
3. 防复发守护：入库一把手脚本 + AGENTS 纪律行。
4. 形成公开发布前检查单（git 历史含旧路径的事实处置）。

**非目标**

- 不改运行时路径行为：`crates/lt-models/src/paths.rs` 的 `~/.config/livetranslate` 字面拼接是 D-9 用户裁决，保持不动。
- 不重写 git 历史：历史提交中的个人路径保留（现仓库暂不公开，D-18）；处置列入 §9 检查单。
- 不动 P3 豁免（§4.4）：系统级确定性路径与显式占位符。
- 不删探针测试：备选方案"直接删除各 `*_tmp` 探针模块"被否决——AH-5 遗留的 whisper sha256 补齐与未来引擎复验都要复用它们，env 化成本极低而探针重建成本高。

## 2. 排查方法与覆盖面（可复现）

- **范围**：`git ls-files` 全部已跟踪文件，排除二进制后缀后共 **153 个文本文件**（rs/md/toml/yaml/ps1/py/gitignore 等）。gitignored 内容（`LiveTranslate/` 参考副本、`target/`、`.cache/`、`.codegraph/`）不属于仓库范围。
- **模式组**：
  - A 用户名：以本机 `$env:USERNAME` 动态匹配（守护脚本 PH-5 内置；字面量不入库）
  - B 盘符：`\b[A-Za-z]:[\\/]`（剔除 `https?://`、`ftp:` 协议误报）
  - C home 风格：`/home/<x>`、`/Users/`、`$HOME`、`%USERPROFILE%`
  - D 其他：`file://`、UNC `\\\\`、字符串值中的 `"~/`
  - E 无盘符盘路径：`/d/tmp` 风格（B 组盲区，`git grep` 补扫命中 README Git Bash 块）
- **复现命令**：

```bash
git ls-files | grep -vE '\.(png|jpg|ico|icns|onnx|dll|br|ttf|otf|woff2?|zip|gz)$' > /tmp/tf.txt
# A 用户名：以本机 $env:USERNAME grep（此处不写字面，避免本文件自命中；守护脚本已内置）
xargs grep -nE "\b[A-Za-z]:[\\/]" < /tmp/tf.txt | grep -vE "https?:|ftp:"   # B
xargs grep -nE "/home/[a-z]|/Users/|\\$\{?HOME\}?|%USERPROFILE%" < /tmp/tf.txt  # C
xargs grep -nE "file://" < /tmp/tf.txt; xargs grep -nE '"~/' < /tmp/tf.txt     # D
git grep -nE "/d/tmp" -- README.md                                    # E
```

- **结果速览**：P0 ×8（7 处 Rust + 1 处 docs）、P1 ×2（docs）、P2 ×15（13 处代码/测试 + 2 处 docs 示例）、P3 豁免约 20 处、误报若干（i18n yaml 的 `Rules:\n` 转义、URL 协议）。配置面（`.cargo/config.toml`、`pyproject.toml`、`scripts/*.ps1|py`）**已验证干净**：两项 cargo `[env]` 均 `relative = true`（LIBCLANG_PATH、SHERPA_ONNX_ARCHIVE_DIR），脚本仅含 URL。

## 3. 分级定义

| 级别 | 定义 | 处置 |
|---|---|---|
| P0 | 含用户名或指向本机私有目录的绝对路径（隐私 + 不可移植） | 必须修（PH-1/PH-3） |
| P1 | 无用户名但暴露本机磁盘布局（如外部仓盘符路径） | 必须修（PH-3） |
| P2 | 合成/示例性质的绝对路径字面量（不暴露个人信息，但违反"不写绝对路径"规则字面） | 规范化（PH-2/PH-4） |
| P3 | 系统级确定性路径（`C:\Windows`、`C:\Program Files\LLVM`）与显式占位符（`<你的用户名>`、`<u>`） | 豁免保留（附依据） |

## 4. 发现总表（证据）

### 4.1 P0：个人路径（8 处）

> 本节证据引文已同步脱敏（用户名 → `<原开发者>`、本机盘符布局 → `<本机位置>`）；带字面原文见施工前 git 历史（a9a5cc0 及更早）。

**Rust 测试/探针常量（7 行，两个消费类别）**

| # | 位置 | 证据（原文截取） | 消费方式 |
|---|---|---|---|
| 1 | `crates/lt-asr/src/engines/nano.rs:221` | `const SNAPSHOT: &str = "C:/Users/<原开发者>/.config/livetranslate/models/huggingface/hub/models--csukuangfj--…-int8-2025-12-30/snapshots/main"` | `mod probe_real_nano_tmp`（`#[ignore]`，WP-A 验收探针）；wav 级已有"未下载即跳过"守卫 |
| 2 | `crates/lt-asr/src/engines/qwen3.rs:224` | 同构，`models--csukuangfj2--…qwen3-0.6B…` | `mod probe_real_qwen3_tmp`（`#[ignore]`，WP-B S0 探针） |
| 3 | `crates/lt-asr/src/sensevoice.rs:314` | `…modelscope/models/pengzhendong--sherpa-onnx-sense-voice…` | `mod probe_real_sensevoice_tmp`（`#[ignore]`，D-30 探针） |
| 4 | `crates/lt-asr/src/sensevoice.rs:315` | `const NANO_WAVS = …/test_wavs`（跨引擎借用 nano 仓音频） | 同上 |
| 5 | `crates/lt-models/src/cache.rs:610` | `probe_tmp::probe_real_cache` 内 `Path::new("C:/Users/<原开发者>/.config/livetranslate/models")` | **无 `#[ignore]`**，随常规 `cargo test` 运行；仅 println 无断言故恒绿 |
| 6 | `crates/lt-models/src/cache.rs:629` | `probe_settings_tmp::probe_load_from_smoke_dir` 内 `set_var("LIVETRANSLATE_CONFIG_DIR", "C:/Users/<原开发者>/.zcode/tmp/lt_smoke")` | **无 `#[ignore]`**；且**测试内改进程环境变量且未持 `crate::ENV_LOCK`**——与持锁的 `paths.rs::config_dir_respects_env_override` 并行运行时存在真实竞态（mutator 不拿锁，锁形同虚设） |
| 7 | `crates/lt-models/src/download/mod.rs:881` | `probe_nano_download_via_hf_mirror` 内 `Path::new("C:/Users/<原开发者>/.config/livetranslate/models")` | `#[ignore]`（真实网络 ~1GB 写真实缓存） |

**docs 引证（1 处）**

| # | 位置 | 证据 | 说明 |
|---|---|---|---|
| 8 | `docs/archive/data-lifecycle.md:49` | LIBCLANG 行引用 `C:/Users/<原开发者>/AppData/Local/Programs/Python/Python313/Lib/site-packages/clang/native` | 病灶历史证据（该行自述"本机绝对路径，指向原开发者的…"）；保留证据价值、脱敏用户名 |

### 4.2 P1：本机外部布局（2 处）

| # | 位置 | 证据 | 说明 |
|---|---|---|---|
| 9 | `AGENTS.md:7` | "与 `<本机位置>\LiveTranslate`、`LiveTranslate-NG` 等外部仓库无关" | 语义 = 声明副本与外部仓无关；可去掉盘符保留语义 |
| 10 | `docs/archive/overlay-realign.md:8` | "与外部 `<本机位置>\LiveTranslate` 等仓库无关" | 归档只读文档，需脱敏例外处理（§5 PH-3） |

### 4.3 P2：合成/示例绝对路径（15 处）

> 本节"原值"列已脱敏为文字描述（合成值本身非个人信息，但为使本文件通过 PH-5 守护脚本、保持"入库零盘符字面量"自洽，不以字面复现）；精确原文见施工前 git 历史（1f0ebbb 及其父提交）。

**Rust 测试合成值（12）**——共同点：不暴露个人信息，但属绝对路径字面量；语义全部可由 `std::env::temp_dir()` 派生等价替换：

| # | 位置 | 原值 | 被测语义 |
|---|---|---|---|
| 11 | `crates/lt-asr/src/engines/nano.rs:209` | Z 盘合成缺失目录 | 目录不存在 → `EngineError::Load` |
| 12 | `crates/lt-asr/src/engines/qwen3.rs:198` | 同上 | 同上 |
| 13 | `crates/lt-asr/src/engines/whisper.rs:259` | Z 盘合成缺失模型 | 模型文件缺失报错 |
| 14 | `crates/lt-asr/src/sensevoice.rs:300` | Z 盘合成缺失目录 | 同 #11 |
| 15 | `crates/lt-app/src/backend.rs:498` | D 盘合成模型路径 | whisper 本地路径 → 不触发下载 |
| 16 | `crates/lt-models/src/cache.rs:537` | D 盘合成命名路径 | 本地路径不进 missing 清单 |
| 17 | `crates/lt-models/src/cache.rs:599` | D 盘合成模型路径 | 本地路径直通不命中缓存 |
| 18 | `crates/lt-ui/src/windows/panel/vad.rs:1347` | D 盘合成模型路径 | 本地路径不在档位下拉 |
| 19 | `crates/lt-ui/src/windows/panel/vad.rs:1356` | C 盘合成命名路径 | 本地路径显示"本地: <stem>" |
| 20 | `crates/lt-ui/src/app.rs:2135` | 快照就绪: C 盘根样例 | 任意日志行透传（字符串任意性） |
| 21 | `crates/lt-ui/src/state.rs:2915` | D 盘背景图样例 | 字幕行 bg_image 字段往返 |
| 22 | `crates/lt-ui/src/fonts.rs:536`（554 为其断言镜像） | D 盘字体路径样例 | 注册表字体条目绝对路径不被系统根拼接 |
| 26 | `crates/lt-app/src/pipeline.rs:1851` | Z 盘合成缺失模型 | 不存在的本地 whisper 路径 → None（**PH-2 施工中由守护脚本试运行抓出的建表遗漏，419adb8 补漏**） |

**UI 文案（1）**

| # | 位置 | 原值 | 说明 |
|---|---|---|---|
| 23 | `crates/lt-ui/src/windows/panel/subtitle_page.rs:447` | hint_text（D 盘背景图样例） | 用户可见的输入框幽灵提示；改为 `…/bg.png` 占位（无盘符字面量） |

**docs 示例（2）**

| # | 位置 | 原值 | 说明 |
|---|---|---|---|
| 24 | `README.md:133`（PS 块）+ `:145-147`（Git Bash 块 3 行） | D 盘临时目录样例（PS/Bash 两风格） | 冒烟临时目录示例；改 `$env:TEMP` / `/tmp` |
| 25 | `docs/archive/font-system.md:473` | LIVETRANSLATE_CONFIG_DIR= D 盘临时目录样例 | 归档文档；无个人信息，**保留不改**（记录当时命令原貌；守护脚本对 archive 仅 WARN） |

### 4.4 P3：豁免保留（附依据）

- **`C:\Windows` 系**（系统根，标准位置）：`crates/lt-ui/src/fonts.rs:180/184`（`SystemRoot` env 兜底缺省——正确姿势本体）、`:479` 注释、`:538/551/552/553/565/567`（系统字体目录语义即被测对象）、`crates/lt-ui/src/windows/panel/font_picker.rs:174`（注册表字体路径缓存键，系统常量拼接）；docs 引述：`docs/archive/font-system.md:50/60/161/300`、`rewrite-plan.md:171`、`rewrite-research.md:570`。
- **`C:\Program Files\LLVM`**：`README.md:74`（LLVM 标准安装位，clang-sys 内置搜索路径，属"系统级确定性"豁免的典型）。
- **显式占位符**：`README.md:135/148` `C:/Users/<你的用户名>/…`、`docs/archive/data-lifecycle.md:101/172/177` `C:\Users\<u>\…`、`docs/archive/rewrite-research.md:479` `C:\Users\<u>\…`。注意 README 两处**必须保留绝对写法**：`settings.json` 的 `models_dir` 值由 `paths.rs` 作字面路径使用，`~` 不会被展开。
- **误报剔除**：`assets/i18n/en.yaml` 的 `Rules:\n`（字符串转义）；各处 `https://` URL；`\\t`、`\\'` 转义序列。

### 4.5 附注：非路径的个人机器信息（单独裁决）

- `assets/reference/` panel 图"设备"下拉显示本机 GPU 型号（`cuda:0 (NVIDIA GeForce RTX 4060 Laptop GPU)`）——原版应用真实的设备枚举行为，非路径；是否中性化见 PH-6（可选）。

## 5. 修复设计

### PH-1 探针/测试路径 env 化（P0 #1-#7）

**设计原则**：沿用 `whisper.rs:266-276` 既有先例（`LT_WHISPER_MODEL` 环境变量驱动 + 未设时静默跳过）；根目录派生语义与 `paths.rs::config_dir()`（`paths.rs:9-17`）完全一致：`LIVETRANSLATE_CONFIG_DIR` 优先，回落 home 拼接。

**lt-models 内（#5/#6/#7）**——crate 内直接用现成函数：

```rust
// cache.rs probe_tmp::probe_real_cache（#5）
let md = crate::paths::models_dir(None).expect("models_dir 解析失败");
// 语义说明：models_dir(None) == config_dir()/models（paths.rs:25-31），
// env 未设时与原硬编码值同义；env 设定时可重定向（可沙箱化，比原值更强）

// cache.rs probe_settings_tmp::probe_load_from_smoke_dir（#6）
#[test]
#[ignore = "手动探针：外部设 LIVETRANSLATE_CONFIG_DIR 指向冒烟目录后 --ignored 运行"]
fn probe_load_from_smoke_dir() {
    // 不再在测试内 set_var（进程级竞态）；env 本身就是 settings_io::load 的输入
    let s = crate::settings_io::load();
    println!("load = {s:?}");
    // …打印部分保持不变…
}

// download/mod.rs probe_nano_download_via_hf_mirror（#7）
let md = crate::paths::models_dir(None).expect("models_dir 解析失败");
```

**lt-asr 内（#1-#4）**——lt-asr 不依赖 lt-models（`crates/lt-asr/Cargo.toml` dependencies 证据），为不扩依赖面，在各 probe 模块内加同一份本地 helper（每模块 ~10 行）：

```rust
/// 探针模型根：LIVETRANSLATE_CONFIG_DIR 优先，回落 ~/.config/livetranslate
/// （与 lt-models::paths::config_dir 同语义；lt-asr 不依赖 lt-models 故本地实现）
fn probe_models_root() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("LIVETRANSLATE_CONFIG_DIR") {
        return std::path::PathBuf::from(d).join("models");
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .expect("探针需要 LIVETRANSLATE_CONFIG_DIR 或 USERPROFILE/HOME");
    std::path::PathBuf::from(home).join(".config").join("livetranslate").join("models");
}
```

- `nano.rs`：`SNAPSHOT` 常量 → `let md = probe_models_root().join("huggingface/hub/models--csukuangfj--sherpa-onnx-funasr-nano-int8-2025-12-30/snapshots/main");`（仓库名等相对段保持字面——它们是模型仓 ID，非本机路径）。
- `sensevoice.rs`：`SNAPSHOT`/`NANO_WAVS` 两个常量 → 两个 `fn snapshot()` / `fn nano_test_wavs()`（`format!("{NANO_WAVS}/…")` 调用点同步改 `nano_test_wavs().join(rel)`）。
- `qwen3.rs`：同 nano。

**探针健壮性增强（三处引擎探针统一）**：快照目录缺失时对齐 wav 级"跳过（未下载）"语义，不再 `expect` 硬崩：

```rust
if !md.is_dir() {
    println!("跳过（模型未缓存；设 LIVETRANSLATE_CONFIG_DIR 指向含 models 的配置目录）: {}", md.display());
    return;
}
```

**基线影响**：不增删 `#[test]` 函数（#6 只加 `#[ignore]`）→ 总数 405 不变；常规面 399 → 398 绿（#6 转入 ignored 面），ignored 6 → 7。AGENTS 基线表述随 PH-1 提交同步更新为"398 测全绿 + 7 ignored"。

### PH-2 合成路径 temp 派生（P2 #11-#22、#26）

统一模式（零绝对字面量、语义等价、机器无关）：

```rust
let base = std::env::temp_dir();
// "不存在目录"类（原 Z 盘合成样例）：base.join("lt_no_such_dir")
// "本地模型路径"类（原 D 盘合成样例）：base.join("lt_local").join("ggml-tiny.bin")
//   —— stem 语义保留（vad.rs:1356 断言"本地: my-model"不受影响）
// "注册表绝对路径条目"（fonts.rs:536）：base.join("lt_userfonts").join("abs.ttf")
// "任意字符串透传"（app.rs:2135）：format!("快照就绪: {}/x", base.display())
```

注意事项：

1. 断言涉及字符串形式且含分隔符时，沿用仓库既有 `.replace('\\', "/")` 归一先例（`cache.rs:583/595`、`download/mod.rs:822`）。逐条核对表：#15/16/17/18 断言为 `is_empty()`/`None`（无字符串比较，安全）；#19 断言 `starts_with("本地: ")` + stem（stem 从 `file_name()` 取，不受分隔符影响，安全）；#11-14 断言 `matches!(Err(Load))`（安全）；#20 透传断言用原串比较（安全）；#21 字段往返（安全）；#22 断言 `PathBuf` 相等（`base.join()` 两侧同源，安全）。
2. Z 盘系（#11-14）另有一层动机：某些机器可能真有 Z 盘映射，Z 盘合成样例虽仍几乎必然不存在，但 temp 派生消除该假设。
3. **`subtitle_page.rs:447`（#23）**：用户可见文案，hint 由 D 盘背景图样例改为 `…/bg.png` 占位（不 temp 化——UI 提示不宜展示运行期临时目录；也不用 C 盘字面量——守护脚本 Tier2 零盘符字面量，见 419adb8）。
4. `docs/archive/font-system.md:473`（#25）保留不动（归档只读 + 无个人信息；守护脚本对 archive 仅 WARN）。

### PH-3 docs 脱敏（P0 #8 + P1 #9/#10）

- `data-lifecycle.md:49`：路径中用户名脱敏为 `C:/Users/<原开发者>/AppData/Local/Programs/Python/Python313/Lib/site-packages/clang/native`——保留"病灶长什么样"的证据价值，去掉用户名。活跃文档直接改。
- `AGENTS.md:7`：改写为"与工作区外的外部仓库（LiveTranslate、LiveTranslate-NG 等，本机具体位置不入库）无关"。
- `docs/archive/overlay-realign.md:8`：归档文档"只读 + 仅允许修订注记"惯例的**脱敏例外**——就地脱敏（`外部仓 LiveTranslate（本机位置不入库）`）并在行尾加 `【2026-09-09 脱敏修订：移除本机盘符路径，见 docs/archive/path-hygiene.md】`。理由：隐私脱敏不可逆、必须彻底，注记本身不得复现被删路径；这是修订注记惯例的唯一允许变体，仅限 P0/P1 脱敏场景。

### PH-4 README 冒烟示例 %TEMP% 化（P2 #24）

```powershell
$env:LIVETRANSLATE_CONFIG_DIR = "$env:TEMP\lt-smoke"     # 原 D 盘临时目录样例
```

```bash
export LIVETRANSLATE_CONFIG_DIR=/tmp/lt-smoke            # Git Bash /tmp 映射用户 %TEMP%
mkdir -p /tmp/lt-smoke
cat > /tmp/lt-smoke/settings.json <<'EOF'
```

`settings.json` 内 `models_dir` 的 `C:/Users/<你的用户名>/…` 占位**保留**（P3 豁免 + tilde 不展开，见 §4.4）。

### PH-5 防复发守护（新脚本 + 纪律行）

`scripts/check_personal_paths.ps1`（新文件，~50 行，风格对齐 `fetch_sherpa_libs.ps1` 的 UTF-8 BOM 头）：

- 输入：`git ls-files` 文本文件集（同 §2 排除后缀）。
- **Tier1 硬失败**（退出码 1）：本机 `$env:USERNAME` 动态匹配；`C:[/]Users[/]<实名字母>`（负向断言排除 `<`、`<u>`、`<你的用户名>`、`<原开发者>` 占位）。
- **Tier2 硬失败**：任意盘符路径 `\b[A-Za-z]:[\\/]`，白名单 `C:[/]Windows`、`C:[/]Program Files`（大小写不敏感）、`https?://`、`ftp://`。
- 命中输出 `文件:行: 内容`；零命中退出码 0。
- AGENTS.md「约定」段加一行：**提交前自查跑 `powershell -File scripts/check_personal_paths.ps1`（路径卫生守护，见 docs/archive/path-hygiene.md）**。

备选否决：build.rs 内嵌检查（拖慢每次构建）、xtask（新增 crate 过重）、CI（无 CI，WD-7 归阶段二——脚本届时可直接挂 CI）。

### PH-6（可选，单独裁决）参考图 GPU 中性化

`scripts/grab_reference_ui.py` 的 `build_saved_settings()` 中 `"asr_device": "cuda"` → `"cpu"`，重跑生成 20 张替换。收益：不暴露硬件型号；代价：`cuda`→`cpu` 与出厂 `config.yaml`（device: cuda）不再一致，参考保真度略降。**默认不做**，等用户点头。

## 6. 验收标准（可执行）

```bash
# 1. P0+P1+P2 归零：守护脚本一把手（内置实名 Users/本机用户名动态匹配 + 白名单外盘符检查）
powershell -File scripts/check_personal_paths.ps1   # 期望 OK、退出码 0（archive 仅 WARN 不阻断）
# 2. 兜底人工通查（盘符通查；命中应仅剩 P3 白名单：C 盘系统级路径）
git ls-files | grep -vE '\.(png|jpg|ico|icns|onnx|dll|br|ttf|otf|woff2?|zip|gz)$' \
  | xargs grep -nE "\b[A-Za-z]:[\\/]" | grep -vE 'https?:|ftp:' \
  | grep -viE 'C.[/\\]+Windows|C.[/\\]+Program Files'
# 3. 基线不回退（PH-1 后：常规面 398 绿 + 7 ignored，总数 405 不变）
cargo test --workspace
# 4. 探针可用性（本机验证）
LIVETRANSLATE_CONFIG_DIR=<本机真实配置目录> cargo test -p lt-asr probe_real -- --ignored --nocapture
# 5. 守护脚本
powershell -File scripts/check_personal_paths.ps1   # 退出码 0
```

## 7. 施工顺序与提交切分

| 序 | WP | 提交 | 文件 |
|---|---|---|---|
| 1 | PH-1 asr | `test(asr): 引擎探针真实缓存路径 env 化 + 快照缺失跳过（P0 去个人路径）` | nano.rs / qwen3.rs / sensevoice.rs |
| 2 | PH-1 models | `test(models): cache/download 探针改 paths 派生；smoke 探针转 ignored 并移除测试内 set_var 竞态` | cache.rs / download/mod.rs |
| 3 | PH-2 | `test(app/ui): 合成绝对路径 temp 派生 + 字幕页 hint 中性化` | backend.rs / cache.rs / vad.rs / app.rs / state.rs / fonts.rs / subtitle_page.rs |
| 4 | PH-3/4 | `docs(hygiene): 病灶/外部仓路径脱敏 + README 冒烟示例 %TEMP% 化` | data-lifecycle.md / AGENTS.md / overlay-realign.md / README.md |
| 5 | PH-5 | `chore(scripts): 个人路径守护脚本 + AGENTS 提交前自查纪律` | scripts/check_personal_paths.ps1 / AGENTS.md |
| 6 | PH-6 | （如裁决）`assets(reference): 参考图设备中性化重拍` | grab_reference_ui.py + 20 png |

顺序依据：P0 先于 P1/P2；代码先于守护脚本（脚本验收以 P0 已清为前提）；每步 `cargo test --workspace` 全绿再进下一步。

## 8. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 探针在无 HOME 的异常环境（计划任务/服务上下文）`expect` 崩 | 仅 `--ignored` 手动运行才会触达；报错信息含明确设置指引（LIVETRANSLATE_CONFIG_DIR） |
| #6 转 `#[ignore]` 改变用法（原可直接跑） | 原语义本就绑定本机特定冒烟目录；迁移后 = 外部设 env + `--ignored`，语义等价且消除 set_var 进程级竞态（该竞态与持锁测试 `config_dir_respects_env_override` 并行时可真实复现） |
| temp 派生路径含反斜杠影响字符串断言 | §5 PH-2 逐条核对表 + `.replace('\\', "/")` 既有先例 |
| ignored 计数 6→7 与 AGENTS 基线表述漂移 | PH-1 提交内同步更新 AGENTS 基线行 |
| 归档文档脱敏违反"只读"惯例 | §5 PH-3 的脱敏例外 + 行内修订标记，先例入档 |
| git 历史仍含旧路径 | 非目标；公开发布前 filter-repo（§9） |

## 9. 公开发布前检查单（关联 D-18"暂不公开发布"）

1. 本计划 PH-1~PH-5 落地后，对全历史跑 `git filter-repo --replace-text`（替换清单 = §4.1/4.2 原值 → 脱敏形式），或评估" squash 重建公开仓"（历史短，成本可控）。
2. PH-6 裁决（参考图 GPU 型号）。
3. `assets/SOURCES.md` 复核无本机信息；`assets/reference/` 20 图人工过一遍（日志窗示例行、缓存页模型体积等均无机器信息，已核）。
