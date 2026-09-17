# 开发配置规范化审计与改造清单（2026-09-11）

> **状态：完工已归档（2026-09-17）**。主体经 ci-workflow-suite（D-88/D-89）与 release-engine（D-90）/ changelog-scheme（D-91）取代性落地，未走「定稿→施工」正路；逐项对账见文末 §4——正文中的时效性陈述（如「无 remote / CI 从未运行」「SHERPA_ONNX_ARCHIVE_DIR」）以 §4 为准。
> 触发：用户提问「仓库开发配置是否都已按行业主流规范落地？是否基本没有必须手动跑的脚本？希望少一点自定义脚本、多来点主流规范操作；GitHub CI 重新审视重写」。
> 方法：三路子代理取证 + 主线程逐条复验（关键结论均带 `文件:行` 证据，见各条）。

---

## 0. 结论

**已达标甚至超配**的部分：`Cargo.lock` 入库、`[workspace.dependencies]` 统一版本、fmt/clippy/四守护双端门禁（2026-09-11 刚落）、版本化 git 钩子、单 exe 自包含（ort dll 内嵌 + sherpa 静态链）、零本机绝对路径（`.cargo/config.toml` 全 `relative=true`）。

**真问题三类**：

1. **两个「不跑就构建失败」的人工前置：一个可消除，一个实测不可消除**——`fetch_sherpa_libs.ps1`（120MB 预取）可回归 crate 自带的自动下载（源码级已确证，实施前补实验）；`uv sync`（libclang）**实测为硬需求**——crate 自带绑定是 Linux 版，Windows 上编译不过（A1 实验，见 §2.A1）。
2. **自定义脚本偏多**：6 个 PS 治理脚本 + 2 个 Python 调研脚本。其中 4 个架构守卫（路径卫生/依赖白名单/源码禁令/死契约）更适合用 `cargo test` 表达（主流形态：架构不变量即测试），跑测试即全跑，无需记「提交前要跑哪几个脚本」。
3. **缺一批主流配置**：dependabot、cargo-deny、`rust-toolchain.toml`、`rust-version`(MSRV)、`[workspace.lints]`、`.editorconfig`、CI 的 `permissions` 与 actions SHA 钉版。

**另外一个必须知道的事实：本仓库没有 git remote（`git remote -v` 为空），CI 从未在 GitHub 上运行过。** `.github/workflows/ci.yml` 目前是「死配置」——它没过过一次真跑，也就没人能说它是绿的。

> **用户政策（2026-09-11 追加）**：所有构建/开发操作**在仓库内完成**；系统级只允许常规开发工具（如 uv）由用户自己装。→ 推论：① **A1'b（改用系统 LLVM）否决**，A1'a（维持 uv 路线）生效；② 新增 **A0：CMake 也收进仓库内（已实施）**；③ 后续任何"新增前置"提案先自问：能否用 uv / 仓库内缓存解决。

---

## 1. 事实盘点

### 1.1 必须人工执行的步骤（当前）

| 步骤 | 何时 | 性质 | 不做会怎样 |
|---|---|---|---|
| rustup + VS Build Tools(C++) + CMake + uv 安装 | 首次 clone | 标准工具安装 | 硬报错（无法回避：MSVC 链接器与 cmake 必须本机有） |
| **`uv sync`** | 首次 clone / `.venv` 丢 | 标准工具调用 | **硬报错**：bindgen `expect("Unable to find libclang")` panic |
| **`powershell -File scripts/fetch_sherpa_libs.ps1`** | 首次 clone / `.cache` 丢 | **自定义脚本** | **硬报错**：sherpa-onnx-sys build.rs 不回落联网，直接 panic |
| `git config core.hooksPath .githooks` | 每 clone 一次 | 标准 git 配置 | 静默降级（本地无钩子，只剩 CI 拦） |
| `powershell -File scripts/precommit.ps1` | 每次提交（钩子未启用时） | **自定义脚本** | 静默（忘了就漏门禁） |
| `cargo test --workspace` | 每次收工 | 标准工具调用 | 静默漏回归 |
| 手写 `settings.json` + 设 `LIVETRANSLATE_CONFIG_DIR` | GUI 冒烟 | 手工编辑文件 | 静默：探测全失败、显示 unavailable |
| 改 `Cargo.toml` version 三处同源 | 发布 | 手工编辑文件 | 静默漂移 |
| `powershell -File scripts/package_release.ps1` | 发布 | **自定义脚本** | 无自动触发，纯人工 |

### 1.2 关键证据（消除前置的可行性）

- **libclang 只被 whisper-rs-sys 需要**：sherpa-onnx-sys 的 build-dependencies 仅 `bzip2/tar/ureq/zip`（registry `sherpa-onnx-sys-1.13.7/Cargo.toml:55-66`），无 bindgen。
- **whisper-rs-sys 有官方逃生门**：`build.rs:119-121` 读到 `WHISPER_DONT_GENERATE_BINDINGS` 即直接拷贝包内预生成绑定 `src/bindings.rs`（204,235 B，实测存在），且该变量被上游 README 记录为受支持用法。现仓库用 `.cargo/config.toml:7` 的 `LIBCLANG_PATH`（`relative`+`force`）钉死指向 `.venv`，才使 `uv sync` 成为硬前置。
- **sherpa-onnx-sys 默认自己下载**：`build.rs:112-130`（无 `SHERPA_ONNX_LIB_DIR` 即进下载分支）→ `:144-204` 从 GitHub Releases 拉取并缓存到 `target/sherpa-onnx-prebuilt/`。本仓库 `.cargo/config.toml:13` 设 `SHERPA_ONNX_ARCHIVE_DIR` 把 build.rs 锁进「只从本地复制、缺文件硬报错」分支（`:181-191`）。上游**无镜像 URL 能力**（URL 硬编码 `:13`，只认 `HTTP(S)_PROXY`），**无 sha256 校验**。
- **ort 本来就零前置**：`ort-sys-2.0.0-rc.13/build/main.rs:25-33` 在 `load-dynamic` 下构建期直接 return；运行时 dll 已入库（`assets/ort/onnxruntime.dll`）并由 `crates/lt-audio/src/vad.rs:16,20-47` 内嵌解压。
- **无 remote**：`git remote -v` 空 → `gh run list` 报 `no git remotes found`。
- 现存配置缺口（实测不存在）：`rust-toolchain.toml` / `rust-version` / `[workspace.lints]` / `deny.toml` / `.github/dependabot.yml` / `.editorconfig` / `.github/` 下除 ci.yml 外全无；CI 无顶层 `permissions`，4 个 `uses:` 全是浮动 tag（`ci.yml:22,24,30,38`）。

---

## 2. 决策清单（逐条：推荐 / 备选 / 代价）

### A. 构建零前置（直接回答「少手动」）

**A0. CMake 收进仓库内（uv 管理）** —— **2026-09-11 已实施并实测**
- 做法：`pyproject.toml` dev 组加 `cmake==4.4.3`；`.cargo/config.toml` 加 `CMAKE = { value = ".venv/Scripts/cmake.exe", relative = true, force = true }`（依据：cmake crate 的 `cmake_executable()` 优先读 `CMAKE` 环境变量，cmake-0.1.58 `src/lib.rs:900-903`——原设计里"cmake 必须在 PATH"是默认路径，不是硬要求）。
- 实测：把 PATH 中两份系统 cmake（`D:\Program Files\CMake\bin` 与 mingw64/bin）**全部摘掉**后 `cargo clean -p whisper-rs-sys && cargo build -p lt-asr --lib` → 退出码 0（52 秒）；构建目录 `CMakeCache.txt` 验明 `CMAKE_COMMAND` = 仓库内 `.venv/.../cmake.exe`、`CMAKE_GENERATOR` = **Visual Studio 17 2022**（不依赖 ninja）。
- 效果：系统前置由「Rustup + VS Build Tools + CMake + uv」减为「**Rustup + VS Build Tools + uv**」；README §1 表格/自检/排错、AGENTS 大坑 5、data-lifecycle §0/§1 修订注记同步。
- 风险：CMake 4.x 拒绝 `cmake_minimum_required < 3.5` 的工程——whisper.cpp 顶层恰为 3.5、ggml 3.14，实测通过；若未来升版撞上不兼容，在 pyproject 降钉 cmake 版本即可（仍是仓库内、零系统安装）。

**A1. 去掉 `uv sync` / libclang 依赖** —— **实测否决（2026-09-11 实验，原推荐撤回）**

- 原推荐：删 `LIBCLANG_PATH`、加 `WHISPER_DONT_GENERATE_BINDINGS=1`，用 crate 自带的预生成绑定。
- **实验结果（`cargo clean -p whisper-rs-sys` 后逐项实测）**：
  - 对照组（`LIBCLANG_PATH` 指向空目录、不开逃生门）→ build script panic：`Unable to find libclang: couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll'] ... (invalid: [])` —— 证明本机确无任何后备 libclang（无 Program Files/LLVM、无 System32 dll、PATH 无 clang），当前构建完全依赖 `.venv` 那一份。
  - 实验组（同空目录 + `WHISPER_DONT_GENERATE_BINDINGS=1`）→ **编译不过**：`whisper-rs-sys` 3 个 E0080（常量求值溢出）：
    ```
    ["Size of _G_fpos_t"][::std::mem::size_of::<_G_fpos_t>() - 16usize];      // 12 - 16 溢出
    ["Size of _G_fpos64_t"][::std::mem::size_of::<_G_fpos64_t>() - 16usize];
    ["Size of _IO_FILE"][::std::mem::size_of::<_IO_FILE>() - 216usize];       // 208 - 216 溢出
    ```
  - 根因：crate 自带的 `src/bindings.rs` 是 **Linux/glibc 版**（含 `__GLIBC__`、`_IO_FILE`、`__off_t` 等），其中带 glibc 结构体**尺寸断言**，在 Windows 上尺寸不符即编译失败。上游的逃生门只对 Linux 可用（该文件在 Linux CI 生成）。
- 佐证（说明问题不在 whisper 本身）：把两份绑定的 **whisper 前缀面**单独抽出来对比——函数/常量/类型 **172/172 完全一致**，结构体定义 **144/144 逐字段完全一致**；且自带绑定的 whisper 段落只引用 `c_int/c_uint`，不含 `long`/`FILE`/`time_t` 等平台相关类型。差异全部集中在 CRT/stdlib 噪声区。
- **结论：在这套依赖组合（whisper-rs 0.16 / whisper-rs-sys 0.15.0 + Windows）下，libclang 是硬需求。** libclang 也只被 whisper-rs-sys 需要（sherpa-onnx-sys 的 build-dependencies 仅 bzip2/tar/ureq/zip，无 bindgen）——所以它是「whisper 这一条边的代价」，不是全局代价。
- **修正后的三选一**：
  - **A1'a（保守，推荐）**：维持现状（uv 钉版 libclang 18.1.1 → `.venv`，`relative+force` 零本机路径）。理由：它已经是"精确钉版 + 仓库内自包含 + CI 无需系统 LLVM"的较优解，只是披了 Python 工具链的皮。
  - **A1'b（更常规，可选）**：保留 bindgen，但把 libclang 来源换成标准 LLVM 安装（`winget/choco install llvm`，与既有的 VS Build Tools/CMake 并列；`.cargo/config.toml` 的 `LIBCLANG_PATH` 指 `C:\Program Files\LLVM\bin` 属"系统级确定性路径"白名单，或删除该行靠 PATH 自动发现）→ 删 `pyproject.toml`/`uv.lock`/`uv sync`。代价：失去 libclang 精确钉版（bindings 本就每次现生成，影响仅格式）；每台机器多装一个 ~1GB 工具。CI：runner 镜像自带 LLVM 20.1.8（是否上 PATH 需 CI 实跑确认）。
  - **A1'c（不推荐）**：`[patch.crates-io]` vendor fork whisper-rs-sys，把我们生成的 Windows `bindings.rs` 作为 in-tree 补丁 → 能彻底去掉 libclang，但需维护含 whisper.cpp 全量源码的 fork，仓库显著变重。

**A2. 去掉 `fetch_sherpa_libs.ps1` 硬前置** —— 推荐：**B 案（默认自动 + 脚本降级为可选加速器）**
- 做法：删 `.cargo/config.toml:13` 的 `SHERPA_ONNX_ARCHIVE_DIR`（默认走 crate 自动下载，缓存落 `target/`）；`fetch_sherpa_libs.ps1` 保留但改定位为「墙内慢/离线时的可选加速器」，用法改为显式传环境变量（`$env:SHERPA_ONNX_ARCHIVE_DIR='.cache/sherpa-onnx'`）。
- 收益：本地零前置；脚本从「必须」降为「可选」。CI 侧我建议**继续用预取**（CI 配置里跑脚本 + cache，=快且不依赖构建期网络），对用户仍是零手动。
- 代价：放弃「构建期零联网」的强制纪律（首次构建必拉 120MB GitHub；直连慢）；`cargo clean` 后重下；上游无镜像/校验和（要镜像只能走 `HTTPS_PROXY`）。
- 备选：C 案保留现状（离线纪律优先）。代价 = 永久保留一个硬前置脚本。
- 备选：D 案 `[patch.crates-io]` fork sherpa-onnx-sys 的 build.rs（自建镜像+校验+缓存）——能力最全，但引入长期维护的 fork。

**A3. 冒烟配置免手抄** —— 推荐：**做（小）**：入库一份 `settings.smoke.json` 模板（或 `cargo run -p lt-app -- --smoke` 参数），README 指过去；省掉「手抄 30 行 JSON 且必须 ASCII 编码」的坑。
- 代价：多一个入库文件 / 或一个小 CLI 参数。

### B. 自定义脚本 → 主流形态

**B1. 四个架构守卫（路径卫生/依赖白名单/源码禁令/死契约）移植为 `cargo test`** —— 推荐：**做**
- 形态：新建 dev 专用 crate（如 `crates/lt-checks`）或挂在既有 crate 的 `tests/`，四条守卫各一个测试；`cargo test --workspace` 即全跑。
- 收益：CI 少 4 个步骤、本地少 4 个自定义脚本、`precommit.ps1` 可瘦身甚至删除（钩子只剩 fmt+clippy）；守卫结果在 IDE 里可见；不需要记「提交前跑哪几个脚本」。
- 代价：~500 行 PowerShell → Rust 的移植工作量；存在解析语义差异风险（缓解：以现有脚本为金标准，同批样本双跑对照）。测试基线数字会变（+4）。
- 备选：保留 PowerShell（现状），只做「6 脚本合并成 1」。代价 = 自定义脚本长期存在。

**B2. `precommit.ps1` + `.githooks` 的归宿** —— 推荐：**B1 之后瘦身为「fmt + clippy」**（约 10s），全量检查交给 `cargo test` 与 CI；钩子本体保留（2026-09-11 刚落地，`core.hooksPath` 一次性启用）。
- 备选：彻底去掉钩子，只靠 CI（多数主流 Rust 仓库的做法）。代价 = 本地无拦截，回归要等 CI。

### C. 缺失的主流配置（缺口清单）

| 编号 | 项目 | 推荐 | 代价/备注 |
|---|---|---|---|
| C1 | `rust-toolchain.toml` 钉版（channel=1.98.1 + rustfmt/clippy 组件） | **做** | 新机器多一次工具链下载；换来本地/CI 完全一致（也解决 2026-09-11 我留的「fmt 门禁可能因 stable 漂移抖动」隐患） |
| C2 | `rust-version`（MSRV）字段 | **做（=1.98）** | 与 C1 同批；声明式 MSRV，cargo 解析依赖时会遵守 |
| C3 | `[workspace.lints]` + 各 crate `[lints] workspace = true` | **做** | 把 lint 策略从命令行挪进 Cargo.toml（IDE/本地与 CI 一致）；首次可能需修一批告警 |
| C4 | `cargo-deny`（`deny.toml` + CI 一个 job） | **做** | 分发 exe 带 200+ 依赖，许可证合规现在全靠手写 `NOTICES.md`、安全公告零扫描；初次需处理 advisory 噪音（`ignore` 列表） |
| C5 | `.github/dependabot.yml`（cargo + github-actions 双生态） | **做** | 零代价；无 remote 时暂不生效，入库即待用 |
| C6 | `.editorconfig` | **做** | 零代价；多语言仓库（Rust/PS1/Py/YAML/TOML），减少格式回漂 |
| C7 | CI 加固：顶层 `permissions: contents: read` + actions 按 SHA 钉版 | **做** | 供应链加固；SHA 需联网查一次，dependabot 负责后续自动升级 |
| C8 | `.gitignore` 补漏（`*.pdb`、`.DS_Store`、`Thumbs.db`、`.idea/`、`.vscode/` 等） | **做（小）** | 零代价 |
| C9 | edition 2024 迁移 | **暂缓** | 2021 仍完全主流；FFI 双栈项目收益低、风险非零 |
| C10 | `cargo-nextest`（per-test timeout + ignored 探针 filterset） | **暂缓** | 有价值（历史有慢收敛/挂起记录），但非当前痛点；引入第二个测试运行器增加心智负担 |
| C11 | SECURITY.md / CONTRIBUTING.md / ISSUE/PR 模板 / CODEOWNERS | **只做 SECURITY.md（可选）** | 单人开发，其余为噪音 |

### D. 发布与 CI 的形态

**D1. CI 重写成什么** —— 推荐形态（待 A1/A2/B1 拍板后一次成稿）：

```yaml
permissions: { contents: read }          # C7
concurrency: ci-${{ github.ref }}        # 保留
jobs:
  fmt:    ubuntu-latest, cargo fmt --all -- --check      # 10 秒级快失败
  check:  windows-latest, needs: fmt
          steps: checkout → rust-cache → (A2 拍板：CI 内预取 sherpa + cache) →
                 clippy -D warnings → cargo test --workspace
  deny:   需要时 cargo-deny（C4）        # 不编译代码，可与上面并行
  release: 触发条件与形态见 D2（现在不建）
```
- 关键变化：fmt 单独快 job 且 `needs` 串联（格式错 10 秒报红，不烧 15 分钟 Windows 机时）；`uv sync`/`fetch` 步骤随 A1/A2 决定去留；actions 全部 SHA 钉版。

**D2. 发布自动化（`package_release.ps1` 的归宿）** —— 推荐：**等有远程仓再做**
- 做法：tag 触发（或 workflow_dispatch）→ windows 构建 → 打包 zip（含 README/LICENSE/NOTICES/OFL + **sha256**）→ `gh release create` 上传。
- 代价：当前无 remote **无法验证**（写了也是死代码）；且 D-18 已裁决「暂不公开发布」。建议与「WD-8 检查更新（GitHub Releases）」同批做。
- 顺带发现：`package_release.ps1` 与 `docs/distribution.md` 已漂移——文档要求 zip 名 `LivetranslateRS-<版本>-win-x64.zip` + `sha256.txt`，脚本产物是 `LiveTranslate-<版本>.zip` 且无校验和。

**D3. 远程仓与 CI 的「活过来」** —— 推荐：**做（需要你决定）**
- 事实：无 remote → CI 从未运行，4 个 action 的版本、`uv sync` 在 runner 上的可用性、603 测在 CI 的耗时与超时（现 `timeout-minutes: 120`）全是未验证假设。
- 建议：建一个**私有**远程仓（不违反 D-18「暂不公开发布」），推 `main`/`arch-v2`，让 CI 真跑一次；否则 CI 只是仪式。

### E. 顺手清理（低风险）

- `silero_reference.py` 用法注释仍是旧 crate 名 `lt-pipeline`（已改名 `lt-audio`）——改注释。
- `.cache/w5_narrow.py`：一次性重构残留（未入库、`.cache/` 已 gitignore），确认无后续用途可删。
- `scripts/grab_reference_ui.py` 依赖「外部原版仓 venv」（本机路径有意不入库）——属调研工具，建议在文件头标注「非开发必需」。

---

## 3. 推荐包（若整包同意）

**A1'a（维持 uv/libclang 现状；A1'b 换标准 LLVM 为备选）+ A2(B案) + A3 + B1 + B2瘦身 + C1~C8 + D1 + D2缓 + D3 + E** —— 效果：

- 首次构建：仍需 `uv sync` 一步（libclang 实测硬需求，A1'a；若选 A1'b 则改为"装 LLVM"）；sherpa 预取与其余步骤全部自动；
- 提交门禁：钩子跑 fmt+clippy（10s）；`cargo test --workspace` 含全部架构守卫；
- 自定义脚本从 6 个降到 1 个（`fetch_sherpa_libs.ps1` 降级为可选加速器）+ 1 个发布脚本（缓做）；
- CI：5 分钟级重写稿成文，等 D3 建仓后一次性验证。

**实验纪律（A1 教训）**：凡「消除某前置/依赖」类结论，实施前必须跑**对照 + 实验**两组实测（对照证明现状确实依赖它，实验证明替代路径真的可用），不得只凭上游源码的逃生门存在就下结论——A1 的原始推荐正是这样被证伪的。

工作量粗估：A1+A2 ~0.5 天（含全量测试与文档同步）；B1 ~1 天（移植 + 双跑对照）；C1~C8 ~0.5 天；D1 ~0.5 天。合计约 2.5 天净施工。

---

## 4. 收口对账（2026-09-17 归档，逐项下场）

本稿未走「定稿→施工」正路：主体在 2026-09-15~16 经 **ci-workflow-suite（D-88/D-89）** 与 **release-engine（D-90）+ changelog-scheme（D-91）** 落地，本文件以原貌归档留证。**零新增决策号**——各裁决均已登记为 D-88~D-91，此处只做对账。

| 项 | 下场 |
|---|---|
| A0 CMake 入 uv | 已实施（2026-09-11 当天，见 §2.A0） |
| A1 libclang | 按 **A1'a** 维持 uv 路线（用户政策：仓库内自包含，A1'b 否决）；§2.A1 实验记录（对照组/实验组/glibc 绑定根因）为**孤本**，保留备查 |
| A2 sherpa 预取 | **反向选择**：保留硬前置（= 本稿 C 案），未采 B 案自动下载；G-25 后机制演进为预解包 `.cache/sherpa-onnx/extracted` + `SHERPA_ONNX_LIB_DIR`（CI 预取 + cache），离线纪律保留 |
| A3 冒烟配置免手抄 | 未做——挂 AGENTS §8 遗留区一行，随下次冒烟改造定生死 |
| B1 守卫移植 cargo test | **反向选择**：D-88 A1 把 `precommit.ps1` 整脚本搬进 CI，守卫保持 PowerShell 形态，不再移植 |
| B2 precommit 瘦身 | 同上被 D-88 路线取代（七项清单保留；本地钩子与 CI 同一份脚本） |
| C1 rust-toolchain.toml | 已做（channel 1.98.1，D-88） |
| C2 rust-version（MSRV） | 2026-09-17 补做（03e8e75）：`[workspace.package] rust-version = "1.98.1"` + 十 crate `rust-version.workspace = true` |
| C3 [workspace.lints] | 2026-09-17 补做（03e8e75）：`rust.warnings = "deny"` + 十 crate `[lints] workspace = true`——零警告策略从命令行钉进清单，普通 build/test 同拦（既有 clippy `-D warnings` 全绿 ⇒ 平移零修码） |
| C4 cargo-deny | 已做（`deny.toml` + security workflow，D-88；校准记录含 MPL-2.0 放行见 ci-workflow-suite） |
| C5 dependabot.yml | 已做（文件入库；仓设置 alerts/security updates 关闭是另一遗留，见 AGENTS §8 ci-workflow-suite 行） |
| C6 .editorconfig | 2026-09-17 补做（03e8e75） |
| C7 CI permissions + SHA 钉版 | 已做（D-88） |
| C8 .gitignore 补漏 | 2026-09-17 补做（03e8e75） |
| C9 / C10 / C11 | edition 2024 维持暂缓 / nextest 维持暂缓（候选在 ci-workflow-suite §六）/ SECURITY.md 等不做（单人开发） |
| D1 CI 重写 | 已做但形态不同：单 job 整脚本入 CI（D-88 A1），fmt 未拆独立快 job；触发 = push 全分支 + PR + 手动（D-89） |
| D2 发布自动化 | **超额完成**：D-90 发布链（`release.ps1` 八动词 + `release.yml`；草稿原建议「缓做」）+ D-91 更新日志机制；`package_release.ps1` 与 distribution.md 的命名/校验和漂移已在 D-90 链中收编（zip 五件套 + sha256） |
| D3 远程仓 + CI 活过来 | 已做：2026-09-15 建仓推送，ci 首跑 13m43s 绿——§0「无 remote / CI 死配置」与 §1.2「无 remote」证据**已过时** |

E 顺手清理：E1 `silero_reference.py` 旧 crate 名勘正 2026-09-17 补做（03e8e75）；E2 `.cache/w5_narrow.py` 本机未入库文件，与仓库无关；E3 `grab_reference_ui.py` 头注已含外部 venv 用法说明，不再另标。

§2/§3 中与上表冲突的「推荐」一律以上表为准——尤其 **A2/B1/B2 三处反向选择**：当时的「默认自动下载」「移植 cargo test」「钩子瘦身」均已被 D-88 的「离线纪律 + 整脚本入 CI」路线实质否决，勿再重提。
