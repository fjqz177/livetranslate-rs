# 界面语言热切换（D-121）

> **本文是什么**：D-121 施工定稿——设置中切换界面语言（`settings.ui_lang`）即时生效、无需重启、无可感知卡顿的根因、设计与验收清单。
> 触发 = 用户 2026-09-30 请求「切换语言以后立马生效，没有任何卡顿」。收口后本文迁 `docs/archive/`。

## 一、现状与根因

- 面板通用页语言下拉（`vad.rs` 界面语言行）选中即写 `settings.ui_lang` 草稿 + 防抖落盘，但旁边提示「界面语言在下次启动应用时生效」（`ui_lang_restart_hint`）——切换唯一生效点 = boot（`lt-app/src/main.rs:47-53` 调 `lt_i18n::set_lang`）。
- **热切换无技术障碍**：`lt_i18n::t()` 每帧渲染时实时查全局表（`OnceLock<RwLock<State>>`，`lt-i18n/src/lib.rs:23-84`），egui 逐帧重绘 → 运行期调 `set_lang` 后下一帧全部 egui 窗口文案自动切换。`set_lang` 仅重建几百键 HashMap（亚毫秒），`t()` 查表 O(1)——无可感知卡顿。
- 编排域文案经 `Msg::new(lt_i18n::t, lt_i18n::get_lang)` 持函数指针、调用时读全局（`shell.rs:142`）→ 自动跟随，零改动。

## 二、摸底事实（需要小补的三个边角 + 一个顺带缺陷）

1. **托盘菜单**：五项文案在托盘线程启动时建一次（`tray.rs`）；状态行/暂停/悬浮窗三项已有 `set_text` 消息臂，显示面板/退出两项无。
2. **原生窗口标题**：面板/日志/基准窗标题只在创建时 `with_title(WinId::title())` 固化一次（`WinId::title()` 本身走 i18n，`state.rs:40`）；切语言后需补一次 `set_title`。Setup 窗已有动态标题通道；悬浮/字幕/确认窗标题恒定不涉 i18n。
3. **设置导入**：`import_settings_json` 仅解析函数 + 测试，**UI 触发点未实装**（导出链 `Cmd::PickExportFile` 已通）——本包不为其接线；未来接线时须调用语言应用入口（见 §四入口单源）。
4. **顺带缺陷（P2，本包修）**：托盘状态行「运行/暂停」硬编码中文（`app.rs` `update_tray_status`），yaml 现成键 `tray_state_running`/`tray_state_paused`（zh.yaml:319-320）无人使用——en 界面下托盘状态行恒中文。托盘重推是本包必经路径，顺带改读现成键，不新增键。

## 三、裁决（D-121，2026-09-30 用户拍板）

| # | 项 | 裁决 |
|---|---|---|
| 1 | 托盘 | **纳入热切换**（复用托盘消息通道加「静态两项文案」臂；状态/暂停/悬浮窗走既有重推） |
| 2 | 「跟随系统」第三项 | **不加**（对齐原版两项；`"system"` 仅档案兼容值，应用入口统一 resolve） |
| 3 | 历史快照 | **不回改**（日志历史行/已开确认框正文/在途 toast 保持生成时刻语言；此后新文案立即新语言） |
| 4 | 生效时机 | **选中即切**（同帧生效 + 照旧防抖落盘，不加「应用后生效」缓冲） |
| 5 | 范围 | 全部 egui 窗口 + 原生标题（面板/日志/基准）+ 托盘五项；设置导入接线不做（见 §二.3） |

## 四、设计

### 入口单源——`apply_ui_lang`（panel/vad.rs 附近助手）

combo clicked 分支**先写 `settings.ui_lang` 草稿，再调唯一入口**（写入留调用方是借用边界下的设计决定——`page()` 的 `settings` 与 `session` 并列可变借用，入口再写草稿需二次分派，评审后维持此形；定稿初稿写「三步进入口」与实现不符，已如实修正为本节）。入口两步：

1. `lt_i18n::set_lang(lt_i18n::resolve_ui_lang(lang))`：resolve 收敛在 lt-i18n 单点（`"system"` 档案值 → 检测语言，boot 与热切换入口共用；D-121 评审后自内联 if 收敛下沉）；Err 时 `warn!` 保持现状（内嵌资产 boot 期已解析过同表，运行期失败几乎不可达）。
2. `request_settings_apply`（既有防抖落盘链路不变）+ `enqueue_action(WinId::Panel, WinAction::ApplyUiLang)` → 宿主刷新标题与托盘。

combo 选中态判定与选中写入共用 `UI_LANG_CHOICES: ["en", "zh"]` 候选表（映射不漂移）；选中索引经 resolve 判定——`"system"` 档案随检测语言高亮对应项（评审 C-1 修复）。

### 宿主臂——`WinAction::ApplyUiLang`（app.rs）

- **标题刷新**：对 Panel / Log / Benchmark 三窗（存在者）`set_title(WinId::title())`——`WinId::title()` 已走 i18n，切换后重取即新语言。
- **托盘五项重推**：
  - 状态行：既有 `update_tray_status()`（硬编码「运行/暂停」改读 `tray_state_running`/`tray_state_paused`）。
  - 暂停项：抽 `sync_pause_label()`（从 `session.running` 取态；`AppCommand::Pause` 臂换用同函数，消灭双份）。
  - 悬浮窗项：抽 `sync_overlay_toggle_label()`（从宿主可见性表取态）。
  - 显示面板/退出：新增 `Tray::set_static_labels(panel, quit)` 消息臂（主线程把 `t("tray_show_panel")`/`t("quit")` 算好发过去，托盘线程 `set_text`——线程模型与既有 `SetPauseLabel` 臂同款，安全面一致）。

### 文案键

- 删 `ui_lang_restart_hint`（zh/en 两 yaml 同步；键集一致性测试 `zh_en_key_sets_identical` 守护）。
- 不新增任何键（托盘状态词用现成键）。

### 分层与契约

- `WinAction` 是 lt-ui 内部意图通道（`state.rs`），**lt-proto 零改动、PROTO_VERSION 不动**；lt-i18n / lt-app / lt-orchestrator 零改动。
- 托盘线程纪律不变：主线程只发消息（非阻塞），`set_text` 全在 `lt-tray` 线程执行。

## 五、测试（一红一绿）

1. **热切换渲染断言**（headless，vad.rs tests）：渲染通用页 → 调 `apply_ui_lang` 切语言 → 下一帧断言页面标签（`label_ui_lang` 译文）已按新语言渲染（zh/en 锚文本两表互斥）；渲染前不重设全局语言——锚的正是入口已切表这一事实。
2. **托盘文案跟随**：状态行构造抽纯函数 `tray_status_line`，en/zh 键值断言（`t_for_lang`，不触碰全局）。
3. 既有防线自动覆盖：`zh_en_key_sets_identical`（键删除同步性）+ D-120 zh/en×8 页溢出断言（双语言布局安全）。
4. **施工中途补裁决（登记 D-121）**：语言敏感 headless 测试并行互打（本包新测试全量跑实证）→ 引入 crate 级互斥锁 `lang_test_guard`（lib.rs），5 处语言敏感测试持锁串行化——D-120「复核重试」保留为纵深防线，锁为根修。

## 六、验收

1. `cargo test --workspace` 全绿 + `precommit.ps1` 三项过。
2. 用户实机走查：面板下拉 zh↔en 即时切换无重启无卡顿；托盘菜单五项、面板/日志窗标题跟随；悬浮窗按钮即时跟随。

## 七、范围外 / 遗留

- 设置导入 UI 未实装；未来接线须调 `apply_ui_lang`（在 `import_settings_json` doc comment 留要求）。
- 已生成快照不回改（裁决 #3）——确认窗打开中切换语言，正文保持旧语言。
- 编排域已生成日志行的语言随生成时刻（`Msg` 只影响此后文案）。

## 八、评审修复留痕（2026-09-30，两轴评审后用户拍板全修）

- **P2**：`"system"` resolve 内联双份（本包 vad.rs + boot main.rs）→ 收敛为 `lt_i18n::resolve_ui_lang` 单点（lt-app / lt-ui 共用）；`settings.rs` 的 `ui_lang` 值域注释 `en | zh` 漂移 → 对齐 `en | zh | system`。
- **C-1**：combo 选中索引未 resolve——`"system"` 档案恒高亮 en → 选中判定经 resolve（`ui_lang_combo_index`），候选表 `UI_LANG_CHOICES` 与选中写入同源。
- **P3**：本文 §四「三步进入口」与实现不符 → 如实改为「写入留调用方 + 入口两步」（借用边界设计决定，见 §四）。
- 记账不修：sync 双函数同形（可容忍）；时序耦合有 doc 缓解；commit 4121f26（D-120 遗留）按判据记范围外加码。
