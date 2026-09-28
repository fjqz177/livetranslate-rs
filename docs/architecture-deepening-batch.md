# 架构深化批:引擎身份单源化 + 防呆双补(C1~C4)

> **本文是什么**:2026-09-29 全仓架构扫描(`improve-codebase-architecture`,四路只读子代理并行走查十 crate + git 热区量化)Strong 四候选的定稿。每项含实证锚点、拍板结论、修法与验收;实施后逐项回报,完工后本文 `git mv` 至 `docs/archive/`。
> **决策**:D-116(C1+C2 引擎身份单源化)/ D-117(C3 草稿写入面穷尽防线)/ D-118(C4 hub 死契约清理),同批登记 `docs/decisions.md`。

## 一、病灶与实证(扫描取证)

1. **超时档案键错位(C1,已实伤)**:`crates/lt-asr/src/engines/profile.rs:12-22` 表键是 worker 级 `"sensevoice"|"nano"|"whisper"`(→ 5.0+段长×2)与 `"qwen3"`(→ 10.0+段长×4);唯一生产调用方 `crates/lt-orchestrator/src/settings_bus.rs:100` 传的却是 settings 级 `raw.asr_engine`(值域 funasr/whisper/qwen3)——`"funasr"` 不在表内恒落 `_ => (60.0, 0.0)`。**funasr 族(sensevoice/nano)的 fast 档自 R6 引入起从未生效**,悬死 worker 失败感知 ~9s 劣化为 60s+;`profile.rs:29-38` 测试恰好只测 qwen3 与未知键,漏掉唯一真实到来的 `"funasr"`。
2. **引擎身份五处平行词表(C2,C1 的根因面)**:「这引擎是谁」散布五文件——settings 键(`lt-proto/settings.rs:17,24`,EngineKey 透镜单点转换已存在)→ worker 键拼装(`lt-orchestrator/pipeline.rs:1877`,`"funasr-nano-2512"→"nano"`,全仓唯一生产点 `build_worker_config`:1855-1910)→ 家族表(`lt-asr/manager.rs:66-73`)→ 档案键(profile.rs)→ worker 入口 match(`lt-app/main.rs:170-203`,单点分派豁免边界)。`manager.rs:502-505` 自认「改名须同步」——同步义务靠注释人肉维持,C1 即第一张断裂账单。
3. **草稿写入面可静默分叉(C3)**:`crates/lt-app/src/shell.rs:35-74` `apply_settings_side_effects` 九臂是设置草稿唯一写入落点(E1-4),但「带设置载荷的 Cmd 变体 ⇒ 写入臂」对应关系无任何机器验证;新变体漏登记 → 防抖到期 `take_due_panel_apply`(`lt-ui/state.rs:2251`)克隆旧值,`ApplySettings` 重放**把用户改动静默回滚**。shell 现有 4 测试仅覆盖 3 臂,无穷举性防护。
4. **SwitchEngine.hub 死契约字段(C4)**:`crates/lt-proto/src/events.rs:550-556`;两生产者(`shell.rs:570`、`lt-ui/windows/panel/mod.rs:112`)都填,两消费者(`shell.rs:62` 用 `..`、`shell.rs:296` 用 `hub: _`)全忽略,`switch_engine` 签名不收;状态回写臂也不写 `s.hub`。删除测试满分:两生产者各少填一行,复杂度纯消失。

## 二、拍板记录(2026-09-29,grilling 五问全按推荐)

| # | 问题 | 拍板 |
|---|---|---|
| Q2 | C3 防线形状 | **契约穷尽标记**:Cmd 加编译期穷尽分类方法,新变体编译期强迫表态 |
| Q3 | 施工编排 | **两张 issue**:①引擎身份单源化(C2 含 C1)②防呆双补(C3+C4);每张一个新会话 /implement |
| Q4 | C1 修复语义 | **让 fast 档生效**:60s 恒定是键错位的意外结果而非设计;下一版 CHANGELOG 提及 |
| Q5 | C2 seam 形状 | **lt-asr 新建 WorkerEngine 四变体枚举**;lt-proto 零改动;lt-app 入口豁免边界保持 |
| Q6 | C4 深度 | **只删 hub**(PROTO_VERSION 7→8 一次);engine 保持 String 不夹带类型化 |

## 三、D-116 · C1 止血:超时档案对齐 EngineKey

- **修法**:`transcribe_timeout_profile` 改收 `EngineKey`(lt-asr 公共 interface 变化,经 `lib.rs:16` 重导出同步 lt-orchestrator):`FunAsr → (5.0, 2.0)`、`Whisper → (5.0, 2.0)`、`Qwen3 → (10.0, 4.0)`;`settings_bus.rs:100` 改传 `raw.engine_key()`。
- **语义澄清**:档案全表 sensevoice/nano/whisper 同值(5.0, 2.0),档案本就只需 settings 级粒度——原 `_` 回落臂随枚举化消失,未知字符串的防护由 `EngineKey::from_settings_str` 的 warn+回退承担(既有行为,`settings.rs:33-43`);原测试 `unknown_engine_falls_back_to_constant` 相应改写。**评审记档(方向反转)**:未知键后果由「60s 恒定(宁慢勿杀)」变为「FunAsr 快档(宁快勿挂)」——上游 settings sanitize 已钳制键值域,实际不可达,但语义方向变化如实记一笔。
- **行为变化**(Q4 拍板):funasr 族超时预算 60s 恒定 → base 5s + 段长×2,失败感知回 ~9s 级;CHANGELOG 下一版提及。
- **验收**:新增「从 settings 键出发」回归测试——`EngineKey::FunAsr` 得 (5.0, 2.0) 不落 60s;`cargo test --workspace` 全绿。

## 四、D-116 · C2 深化:lt-asr WorkerEngine 枚举

- **新枚举**:`WorkerEngine { SenseVoice, Nano, Whisper, Qwen3, Echo }`,放 **lt-asr**(worker 身份是它的域内概念;lt-proto 零改动)。**Echo 为测试假 worker 专用身份**(fake_asr_worker 跨进程,`#[cfg(test)]` 变体不可行;先例 = `WorkerOptions::Echo`),生产入口显式拒绝;评审补记——书面规格原文只列四变体,Echo 系枚举化必要连带。
- **唯一生产点**:`build_worker_config`(pipeline.rs:1855-1910)产出 WorkerEngine;`"funasr-nano-2512"→Nano` 判定保留在此(对照 `registry::funasr_entry`),该字符串字面量此后只存在于模型注册表词表(合法词汇,非引擎词表)。
- **承重替换**:`WorkerConfig.engine: String → WorkerEngine`(serde 改型;同 exe 同构建 IPC,父子进程恒同版本,自洽);`engine_family`(manager.rs:66-73)变枚举方法,padding 分派语义不变;`lt-app/main.rs:170-203` 入口改 match 枚举——**单点分派豁免边界保持**(AGENTS §3),漏同步从注释义务变编译错误。
- **验收**:`nano_family_and_padding_invariants`(manager.rs:501-511)上移 manager_flow **行为级三件**(nano_gets_no_padding / qwen3_ignores_pending_whisper_pad / echo_identity_treated_as_whisper_family——第三件为评审补,钉 Echo 沿旧通配行为视同 whisper 家族);wire 稳定性测试钉 serde↔as_str 双编码;现有 worker 生命周期测试链(fake_asr_worker 14 例)全绿。

## 五、D-117 · C3:草稿写入面穷尽防线

- **契约穷尽标记**:lt-proto 的 `Cmd` 加分类方法(如 `touches_settings_draft(&self) -> bool`,名施工定),**match 无通配臂**——新增任何变体编译器强迫表态。判真集合 = 现写入九臂对应变体:SetAsrLanguage、SetPadding、IncrementalAsr、SetTargetLanguage、SwitchTranslator、SwitchEngine、SetAudioDevice、SetMicDevice、ApplySettings、PersistSettings(后二者共臂,均判真)。
- **对齐测试**:lt-app 断言「判真集合 ⇔ `apply_settings_side_effects` 臂集合」双向穷尽——逐判真变体验证草稿确被写入(带设置效果),判假变体验证不动草稿。
- **PROTO_VERSION 不递增**:纯新增方法,变体/字段/wire 零变化(判定规则 `lt-proto/lib.rs:9-24`;本条即评审记录)。
- **验收**:手造漏登记场景(临时注释一臂)测试红;恢复后绿。
- **已知残面(评审记档)**:编译期强制仅及「新变体必须表态」;若新变体被误判为假而实际携带设置语义,判假表(静态 vec)无完备性强制——靠本档注释与评审纪律兜底。SetPadding 对值域外 engine 仅 warn 不写(shell 头注已声明 pad 值域=两族),判真⇔确被写入在该边缘不成立,属有意设计。

## 六、D-118 · C4:SwitchEngine.hub 死契约清理

- **删字段**:`Cmd::SwitchEngine.hub`(`events.rs:550-556`);两生产者各删一行,消费端 `..` 与 `hub: _` 收紧。
- **PROTO_VERSION 7 → 8**:既有字段删除 = 结构变更(D-81 先例:删两变体 4→5)。
- **不夹带**:engine 保持 String——settings 级词表,消费端 `engine_key()` 透镜现成,worker 级身份由 D-116 WorkerEngine 承载,契约面不重复投资(Q6 拍板)。
- **验收**:SwitchEngine 上下文 grep hub 零残留;proto 序列化往返测试更新;全量测试绿。

## 七、施工编排与门禁

- issue A「引擎身份单源化」:C1 独立 commit 先止血 → C2(可多 commit)。issue B「防呆双补」:C3 → C4。每张一个新会话 `/implement`,内部 `/tdd` 一红一绿,收尾 `/code-review`。
- 门禁不变:`cargo test --workspace` 全绿 + `scripts/precommit.ps1`;显式 pathspec 禁 `add -A`(G-19);里程碑自主中文 commit,随做随推。
- 涉 lt-proto 的两处(D-116 零改动、D-117 方法、D-118 字段删除)评审留痕 = 本文 + decisions.md;走查批施工纪律(`docs/agents/workflow.md` §3)全文适用。

## 八、扫描判对项(不修清单,备查免重查)

pipeline.rs pub interface 仅 9 方法非 god-module;state.rs 域聚合 + 58 内联测试非巨石;lt-ui 内联测试实为 ~5000 行(约 25%),泛泛补测=重复建设;lt-translate 供应商差异零泄漏(D-82 成立)、29 集成测试逐字段断言请求体;lt-asr worker 生命周期 14 测试打真子进程;事件动脉类型化无字符串协议;lt-download 13 mock 用例全经公开 interface;lt-models/lt-i18n 内联测试充分(.bak 三态、撞键回归);UI→宿主命令链两跳直排无冗余层。Speculative 八小项(wasapi 读环析出、WinAction 序列测试、app.rs 纯函数 headless、InterimState 死字段、探测判据单源化、16k 握手、is_asr_cached、panel 数值行)见扫描报告,暂不排期,增殖再议。
