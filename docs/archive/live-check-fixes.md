# 实机走查问题修复（A1/A2/B2，D-92）

> **状态**：**完工**（2026-09-17；决策登记 decisions.md **D-92**；定稿 37d5d49 → 实现 3e0f0d4 / dffbdd2 / 4c64b59 → 收口见 §六）。
> 完工标准：`cargo test --workspace` **615+9 全绿**、clippy 0、precommit 七项过、release 重出。
> **触发**：1.0.0 发布前第一批实机走查（悬浮窗模式切换 / 按钮反馈 / 面板禁用态）暴露的问题。
> **契约影响**：零——不动 lt-proto / Settings / PROTO_VERSION；A2 护栏只改宿主几何持久化。
> **前缀**：无（本档不使用局部编号）。

---

## 一、背景取证（走查现象 → 行级根因）

走查单：`docs/drafts/1.0.0-live-check-batch1*.md`（草稿，不入库）。三处真问题：

### 1.1 A2 悬浮窗「完整」按钮无反应 —— 精简形态从未被任何用户到达过

- **现象**：完整形态下点「完整」，界面无任何变化。
- **根因**（`crates/lt-ui/src/windows/overlay.rs`）：
  - `:76` `let compact = overlay.state.mode == OverlayMode::Compact;`（当前模式）
  - `:276` `toggle_mode(overlay, session, compact);` —— 把**当前**当**目标**传入
  - `:471` `fn toggle_mode(..., to_compact: bool)` → 模式被设成它自己 = 空操作
  - 函数注释自称「UI 侧仅翻转模式」，代码与注释矛盾。
  - 对照 Python 原版 `subtitle_overlay.py:856-859`：`new_mode = "compact" if self._mode == "full" else "full"` —— 原版翻转，移植漏 `!`。
- **影响放大**：全仓检索确认，进精简模式的入口**仅此一个按钮**（无托盘项 / 无设置项 / 无热键）；
  `OverlayMode::default() = Full`。即：命中本 bug 起，**精简形态 100% 不可达**——不是"按钮没反应"，是一整个功能从未被任何用户用到。
- **附带事实（核准）**：模式不持久化（重启回完整）——核对原版 `mode_changed` 只接动画、从未写 settings，**原版也不持久化**，本版保持一致，不新增设置键。
- **附带陷阱（修好后才可达）**：精简态下拖动悬浮窗 → `WindowEvent::Moved` → `on_pos_save_tick`（`app.rs:2075`）把当前几何（高 = 200）写入 `settings.overlay_h`；重启后完整形态按 200px 高开场 = 控件挤压。原版同构（`position_changed → _save_overlay_pos` 也存 size），但原版同样不可达故从未暴露。

### 1.2 A1 按钮三态反馈系统性失效（悬浮窗 / 确认窗 / 字幕窗顶条）

- **现象**：悬浮窗行1按钮悬停、按住均无任何视觉变化（文字不位移是**正常的**，D-32 的红线，不动）。
- **根因（两层，缺一不可）**：
  1. `overlay.rs:949` `small_btn` 内 `.fill(opa(fill, opa_pct))` —— egui 0.36 `Button::fill` 文档原文
     "Note that this will override any on-hover effects"（`egui-0.36.1/src/widgets/button.rs:140`），显式 fill 短路全部状态底色；
  2. `overlay.rs:110-113` 三态色写在 `widgets.*.bg_fill`，而按钮底色取 **`weak_bg_fill`**
     （`egui-0.36.1/src/widget_style.rs::button_style` → `fill: visuals.weak_bg_fill`）——两处均未接上。
  - 结论：D-32 归档文档「按压反馈 = 仅底色加深 ✅ 一并落地」**实际从未生效**（`widget_stability.rs` 的文字零位移测试有效，不受影响）。
- **同模式三处**：确认窗 `confirm.rs:199/214/228` 三个按钮助手（显式 fill + 三态写 `bg_fill`，双重失效）；字幕窗顶条 `subtitle.rs:725`（显式 fill，连三态设置都没有）。

### 1.3 B2a 面板禁用态按钮失去按钮外观

- **现象**：缓存列表为空时「删除全部并退出」读作"不存在"（用户实测）。
- **根因**：egui 里禁用控件状态 = `WidgetState::Noninteractive`（与标签同态，`widget_style.rs:84-90` 仅四态，无独立 Disabled）；`panel/mod.rs::panel_visuals` 把该态设为
  `weak_bg_fill = TRANSPARENT` + `fg_stroke = NATIVE.text`（黑）→ 禁用按钮只剩一圈 `card_stroke` 细边的黑字。
- **影响面**：全面板 19 处 `add_enabled`。页面本身有空态提示 `no_cached_models`（`data.rs:204`），缺的只是按钮形状。

### 1.4 B2b 文案与行为不符 + Qt 转义残留

- `btn_delete_all_exit`「删除全部并退出」/ `dialog_delete_title`「删除全部模型并退出」/ `dialog_delete_msg`「…并退出应用程序」——本版**不退出**（`delete_all_done_msg` 明写"应用继续运行"；yaml 内已有注记但文案未同步）。
- `en.yaml:194` `"Delete All && Exit"` —— Qt 助记符转义残留，egui 无此语义，英文界面照字面显示两个 `&`。

### 1.5 非缺陷三项（走查误报，机制已核）

| 项 | 设计逻辑（判据） |
|---|---|
| 清除不弹确认 | 判据 = "清掉后数据是否另有副本"：自动保存开 → transcripts 文件在 → 免确认；关 → 列表是唯一副本 → 弹确认。面板日志页/日志窗的清除永远免确认（日志另有落盘文件）。（`overlay.rs:250-252`） |
| 恢复默认仅两页弹确认 | 判据 = "重置是否毁掉用户手填数据"：翻译页清 API 配置、字幕页清整套字幕配置 → 确认；样式页/VAD 页只重置外观与阈值 → 直接改。（`subtitle_page.rs:131` / `translation.rs:332` 有确认；`style.rs:71` / `vad.rs:310` 无）且按钮仅在该页有偏离默认值时才出现（`reset_toolbar` n==0 早退）。 |
| 无主题开关 | 面板恒浅色（`panel_visuals` = Windows 原生浅色）、悬浮窗/字幕窗/确认窗恒暗色（D-87 v3.2 定稿）。原走查项「浅色面板下确认窗恒暗色」= "面板是浅的、确认窗恒暗"，非切换项。 |

---

## 二、方案裁决（2026-09-17，用户逐条点头）

1. **模式按钮文案语义**：**保持原版**（按钮显示"当前模式"，点击切到另一态）——与紧邻的「运行/暂停」按钮同构（显示当前状态、点击翻转），本行已有先例；改语义另立（需 D-xx）。
2. **A1 修复范围**：**三处一起**（悬浮窗行1 + 确认窗 + 字幕窗顶条），同一根因、共享助手一次收干净。
3. **hover 字色**：**底 + 字色一起变**（对齐原版 `_BTN_CSS` 的 `#aaa→#ddd`）；几何不受影响（D-32 红线仍守）。
4. **B2b 文案**：**改**（文案与行为不符属真缺陷，零代码风险）。
5. **B2a 档位**：**灰底 + 灰字**（Windows 惯例；`Palette::NATIVE.weak` 本就注释为"弱化文本（禁用/提示）"）。代价：未显式着色的标签会一起变灰 → 施工时逐处补显式色（面板粗筛约 6 处）。
6. **A2 护栏**：**做**（精简态保存几何时高度取 `height_before_compact`）——修了按钮才可达，必须兜住。

**新增决策 D-92**：面板「删除全部」不退出应用（原版 `_delete_all_and_exit` 退出）——行为差异登记 + 按钮/对话框文案与行为对齐。

---

## 三、施工卡

**提交 0（本文档）**：定稿本档 + D-92 登记 + README 活跃表。

**提交 1 — `fix(overlay): 模式切换按钮传参漏取反 + 精简态几何落盘护栏（A2）`**
- `overlay.rs:276`：`toggle_mode(overlay, session, !compact)`。
- `app.rs::on_pos_save_tick`：精简态保存几何时高度取 `overlay.state.height_before_compact.unwrap_or(500)`（其余 x/y/w 照旧）。
- 回归测试（`overlay.rs` 测试模块，新增 `overlay_mode_button_toggles_and_enqueues`）：
  走 UI 点击（bug 在调用点，直接调 `toggle_mode` 测不出）：跑一帧扫 `Shape::Text` 取「完整」矩形 →
  中心发 PointerMoved + PointerButton{pressed:true/false} → 断言 `mode == Compact` 且队列含
  `(WinId::Overlay, WinAction::ToggleMode)` → 再点「精简」断言回 Full。

**提交 2 — `feat(ui): 按钮三态色反馈统一修复——作用域 visuals 择色（A1）`**
- 新增共享助手 `lt-ui/src/style.rs::skin_button`：`ui.scope` 内设三态 `weak_bg_fill / bg_stroke(=1.0) / fg_stroke`，
  **不设显式 fill**，由 egui 按上一帧响应择色（1 帧 ≈16ms）；几何参数（字号/圆角/最小尺寸）由调用点传入。
- 色族表落地（`BtnSkin`）：默认 / 退出 / 暂停 / 字幕开 / 确认窗主+危险 / 字幕窗顶条；
  hover/pressed 推导色以实机目测收口。
- 三处站点改造：`overlay.rs::small_btn`（删除自身实现，改调助手，7 个调用点）、
  `confirm.rs` 三个按钮助手、`subtitle.rs` 顶条按钮。
- `widget_stability.rs`：把手抄的 visuals 链 + 占位按钮改为**直接调用生产 `skin_button`**（消双份记账，D-88 单一真源精神）；D-32 两个零位移断言必须保持绿。
- **红线**：三态 `bg_stroke` 宽恒 1.0、圆角三态同值、字号/内边距不变 → 文字零位移。

**提交 3 — `fix(panel): 禁用态按钮恢复按钮外观（B2a）+ 删除全部文案与行为对齐（B2b，D-92）`**
- `panel_visuals`：`noninteractive.weak_bg_fill` → 浅灰实底（`#F0F0F0`）；`fg_stroke` → `Palette::NATIVE.weak`；
  `bg_fill` 保持 `TRANSPARENT`（标签底不动）。
- 标签审计：未显式着色的 `ui.label(RichText::new(...))` 逐处补 `pal.text`（按当前语义选色）。
- i18n（zh/en 同步）：`btn_delete_all_exit` → 「删除全部」/ "Delete All"；
  `dialog_delete_title` → 「删除全部模型」/ "Delete All Models"；`dialog_delete_msg` 删「并退出应用程序」句；
  yaml 内注释说明键名保留原因。

**收口提交 4**：本文档 `git mv` 进 `docs/archive/` + README 两表更新 + decisions.md 落档路径更新 +
`button-press-feedback.md`（D-32）补注「底色加深实际生效范围」+ AGENTS §8 遗留区按本轮结果改写。

---

## 四、验收基线

- `cargo test --workspace` 全绿（基线 612+9；新增 A2 回归测试后应为 613+9）。
- `pwsh -File scripts/precommit.ps1` 七项全过（fmt / 五守护 / clippy `-D warnings` 0）。
- `cargo build --release -p lt-app` 重出 exe（走查用发布产物，改完必须重出）。

**实机复验（用户，改完用新 exe）**：
1. 悬浮窗点「完整」→ 收到 200px 小条（「清除/字幕」与模型下拉消失）；点「精简」→ 展开还原。
2. 精简下拖动 → 退出 → 重启：按正常高度开场（护栏生效）。
3. 悬浮窗行1 / 确认窗确定取消 / 字幕窗顶条：悬停亮一档、按住深一档、文字全程零位移。
4. 缓存列表为空时「删除全部」可见但灰（点不动）；文案无"并退出"；英文界面无 `&&`。

---

## 五、遗留走查

- **修后复验（用户，第二轮）**：复验单 = `docs/drafts/1.0.0-live-check-round2.md`（草稿不入库；四条修复复验 + B1/B3 按条件补验 + B4 作废）。
- A1 推导色（退出/暂停/字幕开/确认窗/字幕窗各族 hover+pressed）**首次落地**，数值以实机目测收口；不通过只调常量，不动结构。
- B2a 的字色弱化落实在 `panel::panel_btn`（面板 8 处禁用按钮已改走它）；施工期发现全局改灰会波及 40+ 处未着色标签，**未采用**全局方案——后续若再有人想动 `noninteractive.fg_stroke`，先读本档 §1.3。
- 死 i18n 键 8 个（`theme_dark/light`、`btn_check_update`、`btn_open_repo/issues`、`hotkey_overlay/subtitle`、`changelog_title`）——
  不单独立包，WD-8 落地时一并决定生死。

---

## 六、提交记录

| 提交 | 内容 |
|---|---|
| `37d5d49` | docs：本档定稿 + D-92 登记 + README 活跃表 |
| `3e0f0d4` | fix(overlay)：A2 传参取反 + 精简态几何落盘护栏 + 回归测试（反例实测） |
| `dffbdd2` | feat(ui)：A1 共享 `skin_button` + 三窗站点 + 三态底色回归测试（反例实测） |
| `4c64b59` | fix(panel)：B2a 禁用态外观 + `panel_btn` + B2b i18n 文案（zh/en） |
| 收口 | 本档归档 + AGENTS §8 + D-32 补注 + 走查单更正 |
