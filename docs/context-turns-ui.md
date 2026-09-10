# 「上下文数」设置界面补完（D-84）

> **偏差登记**：**D-84**（翻译页「模型配置」组新增**当前活跃模型**的「上下文数」直达行——
> 原版只有模型编辑对话框内的 QSpinBox；差异为**新增入口**，不是语义变更）。
> **背景**：用户反馈「翻译时携带 n 条上下文这个功能在设置 UI 里根本就没做好」。
> **后端事实**：取值链路本已修好（`docs/llm-api-round2.md` R2-2：W3 回归导致
> `set_context_turns` 设在不参与请求的实例上，第二轮已完全修复并有回归测试）。
> 本轮只处置**界面**：控件找不到、说明挂错组。

## 1. 现状缺陷（无头探针取证，修复前）

翻译页实际渲染坐标（`egui::Context::run_ui` + 扫 `Shape::Text`，面板顶为 y=0）：

| 位置 | 文本 | 判定 |
|---|---|---|
| y=330 | 「网络配置」组标题 | — |
| y=349.5 | 「模型连接超时 [15 s]」 | — |
| **y=369** | **「携带最近 N 条翻译历史作为上下文…0 = 不携带上下文。」** | **错位**：该组内没有任何上下文控件 |
| y≈5185（对话框） | 「上下文数:」+ 裸 DragValue | 控件真实位置在模型编辑对话框**高级参数区**，无 tooltip、无说明 |

三个问题同源（移植期遗留 + W1 参数面重排）：

1. **说明与控件分居两处**：`context_turns_hint` 自 M4.4 起就渲染在「网络配置」组下
   （`translation.rs`），用户在本页看得见说明、找不到控件；
2. **控件裸放**：模型编辑对话框该行缺原版 `setToolTip(t("context_turns_hint"))`
   的等价物（Python `dialogs.py:523`）——W1 把该行从基本区移入高级区时未补；
3. **主页面零入口**：翻译页是用户改翻译行为的首选落点，此处根本没有该设置。

## 2. 修复内容（2026-09-10）

| # | 改动 | 位置 |
|---|---|---|
| 1 | 翻译页「模型配置」组新增「上下文数」行：标签 + DragValue(0–20) + 说明行（hover 亦显示说明） | `lt-ui/src/windows/panel/translation.rs` `page()` |
| 2 | 删除「网络配置」组下错挂的 `context_turns_hint` | 同上 |
| 3 | 模型编辑对话框该行补 `.on_hover_text(context_turns_hint)`（对齐原版 tooltip）+ `speed(1.0)` | 同上 `editor_fields()` |

**写回语义（面板行 = 当前活跃模型）**：改 `settings.models[active].context_turns`
→ `schedule_prompt_apply`（600ms 防抖）重建翻译器 + `mark_settings_dirty`（300ms 落盘）。
与编辑对话框「确定」同路（对话框走 `Cmd::SwitchTranslator` 即时重建）；
两处读写同一真源 `Settings.models[].context_turns`，不存在双份状态。
后端 `switch_translator` → `Translator::set_context_turns(mc.context_turns)`（`pipeline.rs`），
配置指纹 `config_fingerprint` 已含该字段（改值必然重建）。

**与原版的差异（D-84）**：原版该值只在模型编辑对话框（基本区）内可改，翻译页无此控件。
Rust 版新增面板直达行——理由是面板是本应用改翻译参数的常用落点，且该值只影响翻译行为。

## 3. 测试与验收

- `context_hint_sits_in_model_group_not_network_group`：无头渲染翻译页，断言说明文本
  **只出现一次**且坐标满足「模型配置组标题 < 标签 < 说明 < 网络配置组标题」，
  并断言标签同一行右侧存在数值控件（回归本次错位）。
- `panel_context_drag_writes_active_model_and_schedules_rebuild`：无头指针拖拽该行
  （悬停 → 按下 → 单帧横拖 15px → 抬起，`speed=1.0`）→ 断言
  `context_turns == 15`、`prompt_apply_due` 已登记、`PromptApply` 节拍已排、
  `settings_apply_pending` 已置（端到端证明入口可用）。
- i18n 竞争：两条用例一律用 `lt_i18n::t_for_lang` 取 zh/en 双候选匹配（并行测试
  线程会切全局语言，`t()` 断言会被打飞——实测过一次失败后修正）。
- 全量：**540+9 测全绿**（基线 538+9，净增 2）+ `cargo clippy --workspace --all-targets -D warnings`
  零告警 + 三守护脚本（`check_personal_paths` / `check_deps` / `check_guards`）过。
- 零 lt-proto 契约变更（无新字段、无新命令）。

## 4. 待实机走查（用户）

1. 翻译页「模型配置」组看到「上下文数: [0]」+ 说明行；拖拽/单击改值；
2. 改为 n>0 后翻译两段以上文本，第二段起请求应携带前 n 条原文/译文
   （可在日志或端点侧观察 messages 含历史对）；
3. 切到另一模型行（或改模型编辑对话框的值）→ 面板行数值随之刷新，两处一致；
4. 「恢复本页」应把上下文数一并归 0（`restore_translation_page` 重置 models）。
