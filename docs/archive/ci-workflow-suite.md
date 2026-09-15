# CI 工作流全家桶——社区规范调研与落地

> 状态：完工已归档 | 日期：2026-09-15 | 触发：用户令「完善一整套符合社区主流规范的 GitHub Actions 工作流」| 前缀：无局部号（引用全局 D-88）

## 一、背景取证（2026-09-15 实抓，非记忆拼凑）

四路并行调研 17 个高星仓 workflow 正文（raw 文件实抓）+ 6 个生态议题现状核证：

- **官方/基础库**：rust(118k)/cargo/tokio(33k)/serde/clap——rust 系 action 全 SHA+版本注释；cargo 的 conclusion 聚合 job 揭穿「skip 被记为成功」坑；tokio 并发组用 PR 号；serde 把重检查移出 PR 路径靠 cron；clap 月度金丝雀抓依赖新版本破坏。
- **终端应用**（形态最像本仓）：ripgrep(68k)/bat(60k)/fd(44k)/starship(60k)/helix(46k)/alacritty(66k)——发布链最大公约数 = tag→draft release→构建→**sha256 边车**→upload→转正；ripgrep 的「tag 与 Cargo.toml 版本一致性硬校验」；fd 的供应链卫生（SHA 钉+persist-credentials: false）；alacritty 的 Windows job 就是单 exe 免安装范本；**Windows 不签名是 17 仓常态**（仅 starship 云签）。
- **大厂级**：ruff/uv/zed/tauri/polars/meilisearch——clippy 全参数范本 `--workspace --all-targets --all-features --locked -- -D warnings`；zed 的「本地/CI 同脚本」与 xtask 生成 workflow；meili 事件分层（重 job 只夜间）；rust-cache `save-if` 主干门控；`CARGO_PROFILE_DEV_DEBUG=0` 护缓存。
- **生态现状**：actions-rs 确认废弃（三件套 = dtolnay/rust-toolchain + taiki-e/install-action + 裸 cargo）；**cargo-deny 取代 cargo-audit**（官方 action 是 Docker，只能 Linux）；cargo-dist 2025 死而复生但对单 exe 不推荐；**attestation 私有仓不可用（需 GHE Cloud）**；dependabot 2025-08 起支持更新 rust-toolchain.toml；17 仓中 8 个根本没有 dependabot（ripgrep/starship/alacritty/rust/cargo/serde/tauri/zed）。

## 二、本仓映射（2026-09-15 两轮 review 结论）

已达社区线：顶层最小权限、clippy `-D warnings`、fmt --check、`--version` 冒烟、`if-no-files-found: error`、打包单一真源（package_release.ps1）。
缺口：工具链浮动（P1-2）、无 `--locked`（P1-1）、PR+push 双跑且并发组互不取消（P2-1）、cache-on-failure 未开（P2-2）、无发布/安全/依赖更新 workflow、门禁清单手抄 ci.yml 漂移实证（P3-1「前六项」）。

## 三、方案裁决（用户 2026-09-15 逐批拍板，D-88）

| 项 | 裁决 | 说明 |
|---|---|---|
| A | **A1 整脚本入 CI 单 job** | CI 第一步原样跑 precommit.ps1（七项=唯一门禁真源），消灭手抄清单漂移；旧双 job 退役 |
| B | **rust-toolchain.toml 钉 exact** | 本地=CI 同链，治 P1-2；钉 1.98.1（2026-09-15 本机实取）；~~dependabot 可代更~~ → 被 G-b 收窄取代：rust-toolchain 是独立 ecosystem 且未配置，手动改号（见该文件头注流程） |
| C | **--locked 全加** | cargo 三处 + `uv sync --locked`（两把锁入库真源，CI 不现场重解） |
| D | **cache-on-failure: true** | 修红期间不清缓存 |
| E | **E2 gh CLI 手写发布**（修正原推荐 E1） | taiki-e 自带打包会绕开 package_release.ps1 造成两套打包真相；修正留痕 |
| F | **cargo-deny 四表** | ubuntu job（Docker action 只能 Linux，且纯元数据扫描与目标平台无关、成本一半）；advisories 单独 continue-on-error |
| G | **G-b 收窄**（修正原推荐 weekly 双生态） | necessity review 判其为六件最弱一环（删之无硬能力损失 + 与主干直推摩擦 + 近半头部仓没有）；收敛为只盯 github-actions 生态、monthly、cooldown 7 天；cargo 依赖手动升 + security 兜底 |
| H | **nextest 暂不启用**（候选） | 每测一进程隔离需 611 测全量重验，留 ci.yml 注释待命 |
| I | **typos 暂不上**（候选） | 纪律：CI 不跑本地没有的门禁——须先进 precommit.ps1（白名单初始化有量），另包施工 |
| J | **J1 删 pull_request 触发** | 主干直推下是死配置且双跑源头；公开时随 distribution 前置清单回摆 |
| K | **第三方 action SHA 钉版** | 官方 checkout/cache/artifact 留大标签；后续 dependabot 接管升版 |
| L | **明确不做** | cargo-dist / release-plz / MSRV job / attest+签名 / 多 OS 矩阵 / cargo-audit |

## 四、施工卡（六文件，一文件一件事）

| 文件 | 一件事 | 关键点 |
|---|---|---|
| `.github/workflows/ci.yml` | 主干质量门禁+测试+分发演练 | 单 job；precommit.ps1 整脚本；--locked；cache-on-failure；触发 = push 全分支+手动 |
| `.github/workflows/release.yml` | tag→Draft GitHub Release | tag↔Cargo.toml 版本硬校验；打包走 package_release.ps1 单一真源；sha256 边车；persist-credentials: false；转正人工 |
| `.github/workflows/security.yml` | 依赖供应链扫描 | cargo-deny 四表；advisories continue-on-error；push 路径+周一 cron |
| `.github/dependabot.yml` | actions 版本月更 PR 化 | 仅 github-actions 生态；cooldown 7 天；SHA 钉的 action 也会被更新 |
| `rust-toolchain.toml` | 工具链单一真源 | 1.98.1 + minimal + rustfmt/clippy；rustup 原生消费，零 action |
| `deny.toml` | cargo-deny 策略四表 | 首跑必红是预期，逐条校准，豁免须带理由 |

SHA 钉版登记（2026-09-15 实取）：setup-uv v10.1.0 = `bec219d`、rust-cache v2 = `6323deb`、cargo-deny-action v2 = `3c6349`。
伴随改动：precommit.ps1 改序「便宜先死」（fmt → 五守护 → clippy，秒级项提前）；AGENTS §2/§8 同步。

## 五、验收基线

YAML/TOML 机械校验（PyYAML/tomllib）通过；`uv lock --check` 与 `cargo metadata --locked` 预检通过（CI 首跑不会死于 --locked）；precommit 七项过；`cargo test --workspace` 基线绿（610+9 起滚动）。

## 六、遗留走查（完工后随用随验）

0. **四路子代理复审（2026-09-15 同日）**：零 P0/P1。P2×2 已修（precommit clippy 补 `--locked` 堵「静默重写锁架空不变式」；release 幂等 create + `--clobber` + 边车改 LF 行尾）；P3 批已修（并发组回摆变体、artifact 分支 retention 3 天、rust-cache `shared-key: lt` 跨 job 共享〔源码实证 shared-key 即替代 job 键〕、sherpa 下载 tar 完整性闸门〔实测 2m8s 仅冷路径〕、security paths 补 .cargo/rust-toolchain + `arguments --locked` + timeout 20、AGENTS 基线滚 610+9、双边同步残留清除）。留实机项：cooldown 对 actions 生态实效、dependabot 升 SHA 的来源首核、cargo-deny 首跑红单清单。

1. **deny.toml 首跑校准**：security 首次实跑必红，按输出逐条补 allow/ignore（禁无脑放行）。
2. **release 全链实机演练**：打测试 tag → draft release → 下载试跑 → 转正 → `gh release delete` + 删 tag 清场。
3. **CI 首跑观察**：单 job 墙钟变化；rust-toolchain 钉版与 runner 预装不一致时的工具链下载时长。
4. 候选：nextest 引入（H）、typos 入 precommit（I）、rust-cache `save-if` 主干门控。
5. 公开前三件套：attest-build-provenance（私有仓不可用）、immutable releases、tag protection + PR 触发器回摆（J 回摆）。
6. hook 触发面缺口候选：scripts/*.ps1 与 .github/** 改动不触发本地门禁（本次全批绕过钩子即实证），可评估纳入。

## 七、证据

全部 raw 实抓 URL 清单见调研原稿（会话留档）；action SHA 经 `git ls-remote`/GitHub API 实取；attestation 私有仓限制 = docs.github.com artifact-attestations；immutable releases = github.blog 2025-10-28。
