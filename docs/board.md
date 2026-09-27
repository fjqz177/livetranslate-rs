# 工作看板（待拍板 / 施工中 / 遗留）

> **定位**：易变状态区（常青参考，docs/README.md 纪律①）。**条目真源 = 本档**——AGENTS §8 只留机制与三小节指针，每次工作包增删行只改本档，不再动 AGENTS。
> **机制**：一行一包、清零即删、细节只活在文档里；**待拍板条目命中 = 用户未拍板，未点头禁动工**（先走 `docs/prompts/decision-request.md`）。
> **维护面**：收口卡第 6 步 / 交接卡第 1 步 / 走查卡第 6 步（三张卡均已按本档指向改述）；三小节与 AGENTS §8 同名同序——断言 4 目前扫的是 AGENTS §8，本档是条目真源。
> **草稿指针写法**：只记 `docs/drafts/` 目录 + 文件名（守护对「docs 顶层指向草稿区」判 WARN，AGENTS §8 同款豁免意图）。

## 待拍板（等用户裁决；草稿在 `docs/drafts/` 不入库）

- **ASR 模型选型**——近一年开源模型调研（FireRed2-CTC / Cohere-14lang / Dolphin / 标点闸门候选，A~H 清单）；草稿 = `asr-model-survey-2026.md`
- **增量识别的目标形态**——「做成什么样」未定案（设计无关缺陷批已拆出完工归档 = D-94；余包 B/D/G 与 J7 待定案）；草稿 = `incremental-asr-overhaul.md`
- **跨平台分期 ①~⑤**——2026-09-11 可行性评估（无草稿，结论在会话记忆；P0 = 宿主 trait 化）
- **llm 遗留⑪**——规则 4/5 偏离可见（`docs/archive/llm-api-round2.md`）
- **UX 三期可选项**——前缀码枚举化 / 设置保存失败 UI 流 / 错误译文样式（`docs/archive/ux-feedback.md`）
- **PH-6**——参考图 GPU 型号中性化重拍（可选）（`docs/archive/path-hygiene.md`）
- **非代码面七块问题清单**——正式 33 条目（轻 21/中 6/待拍板 6，含 8 条老账重提）待逐条裁决；草稿 = `noncode-infra-issues-2026.md`

## 施工中

- （无）

## 遗留（完工包的实机走查与未了项，一行一包，清零即删）

- window-flash-cleanup（D-100）：面板/确认窗/基准窗**首次显示**是否存在 ≤2 帧白帧存疑（取证采样 200ms 间隔未证实）——走查若扎眼，按先画后显机制扩展 `set_visible` 显示路径另立小包（`docs/archive/window-flash-cleanup.md` §五）
- docs 指令体系元方法化（ADR-22）：三项未了——M11 句式归一未做 /「滚动测试基线」是否设真源待裁（现无真源，悬空指针已拆）/ 断言 9 指针检查为「§8 段内出现即过」（P3）（`docs/archive/agents-meta-restructure.md` §五）
- scripts-usability-review（ADR-21）：S3（check_agents_health 九条小修）/ S4（fetch/package/release 三脚本打磨）未施工，随用随做、须代码解禁——施工时按档案核销注记对现树重定位（`docs/archive/scripts-usability-review.md` §二）
- 遗留清账批（D-96）：本阶段三项已执行（Q1 开关 / 瘦身 `21bd5a6` / Q4 收编 `d2cbad9`，记录 = `docs/archive/legacy-cleanup.md`）；余缓办队列 Q2/Q3/Q6/Q9/Q10 与 Q12 backlog 代码解禁后按档 §1 归队，走查 51 条转 issue 驱动按档 §3 单条验证——队列消纳完删本行
- asr-chain-robustness（D-94）：实机走查 8 项（档 §5）；评审遗留 8 条（档 §10）——清空是否管字幕窗 + 三项测试缺口随「遗留清账批」，余 5 条 P3 留档（§10-4~8）（`docs/archive/asr-chain-robustness.md` §5/§10）
- quit-flow-redesign（D-87）：§九遗留走查 6 组随用随验（`docs/archive/quit-flow-redesign.md` §九）
- 架构 v2/2.1：实机走查 11 项（`docs/archive/architecture-v2.md` §6.4；原「W5 6 项」为该表子集已并）
- translator（D-85）：实机走查 13 项（`docs/archive/translator-probe-hotswap.md` §6.2）
- model-trust（D-83）：实机走查——改坏一个模型文件应自动隔离+重下+装载（`docs/archive/model-trust-repair.md`；档内无锚点，条目以本行为准）
- context-turns（D-84）：实机走查 4 项（`docs/archive/context-turns-ui.md` §4）
- asr-hardening：GUI 冒烟 A/B/C + T1 qwen3 长样例校准（whisper sha256 子项已销项）（`docs/archive/asr-hardening.md` §6）
- download-overhaul：S1/S6 全程真实网络走查（`docs/archive/download-overhaul.md` §6.2）
- 复刻期：WP-9 M6 性能调优五步（启动<2s / 空闲 CPU<1% / 8h 长跑 / 内存回收 / 端到端——需单独方案）；WP-5/WP-8 随「遗留清账批」（`docs/archive/parity-closure.md`）
- distribution：WD-4/6/8、干净机端到端、更新日志渲染立包、8 死 i18n 键随「遗留清账批」；状态真源 = `docs/distribution.md` §3.3/§4
