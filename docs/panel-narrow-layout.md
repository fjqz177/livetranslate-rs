# 面板窄窗布局塌陷修复（D-120）

> **本文是什么**：D-120 施工定稿——控制面板（WinId::Panel）窄窗右缘裁剪的根因、三层防线设计与验收清单。
> 现象证据 = 用户 2026-09-30 两帧截图（默认宽 535 / 最窄 480）。收口后本文迁 `docs/archive/`。

## 一、现象

- 最窄（480）时 tab 条最右「日志」页签被裁一半；多个 tab 页右侧控件（按钮 / 标签 / 下拉）整块消失，无滚动条、无报错。
- en 界面下顶部橙黄横幅文案贴边超宽（zh ≈250px 在 480 下完整——用户已确认以截图为准，zh 横幅本身无病灶）。

## 二、根因（三条叠加）

1. **布局机制**：egui `ui.horizontal` 横排行不换行、不收缩、不滚动，子元素超出 clip rect 即被窗口右缘静默裁剪；tab 条与横幅是 painter 定宽直画，连布局收缩的假机会都没有。
2. **窗口约束失配**：`min_inner_size(480,420)` 照搬原版 Qt `setMinimumSize(480, 420)`（`crates/lt-ui/src/app.rs:251-253`）——Qt 布局器有 stretch / 换行 / minimumSizeHint 联动，480 在 Qt 下成立；egui 无等价机制，最小宽不构成「内容放得下」的保证，480 反成 bug 温床。
3. **测试盲区**：现有 headless 冒烟一律 `RawInput::default()`（无 screen_rect ≈ 无限宽画布，`panel/mod.rs:592-609`），窄宽溢出在测试里结构性不可见。

溢出清单（480 窗宽 → 内容可用 ≈454px；宽度按 12.5 号字估算，精确定值交 L3 实测）：

| 位置 | 行 | zh 估算 | en 估算 | 判定 |
|---|---|---|---|---|
| tab 条 8 页签定宽横排 | `panel/mod.rs:270-312` | ≈490 | ≈550 | zh 就溢出（截图实证） |
| 样式页字体行（label+combo240+刷新+实际字体 label） | `font_picker.rs:56-112` | ≈600 | ≈590 | 双语溢出 |
| 缓存页管理行（两按钮后串 hint） | `data.rs:213-235` | ≈520 | 更长 | 双语溢出 |
| 翻译页编辑器厂商预设行（label+combo220+串长 hint） | `translation.rs:819-844` | ≈860 | 同量级 | 模态内必溢出 |
| 样式页预设行（combo260+两按钮） | `style.rs:124-150` | ≈420 | ≈535 | en 溢出 / zh 贴边 |
| 字幕页背景图片行（label+TextEdit260+两按钮） | `subtitle_page.rs:430` | ≈450 | ≈500 | en 溢出 / zh 贴边 |
| 缓存页转录行（checkbox+按钮） | `data.rs:110-133` | ≈380 | ≈495 | en 溢出 |
| VAD 麦克发行（checkbox+combo300） | `vad.rs:622-640` | ≈425 | ≈465 | en 溢出 |
| 横幅（全宽 allocate + 居中不换行直画） | `panel/mod.rs:185-222` | ≈250 安全 | ≈430-460 | en 贴边超宽 |

## 三、裁决（D-120，2026-09-30 用户拍板）

| # | 项 | 裁决 |
|---|---|---|
| 1 | 总策略 | **三层防线**（否决「只抬最小窗宽」= 隐患仍在；否决「全面响应式重排」= 工作量×3） |
| 2 | 最小窗宽 | **headless 实测安全宽**（估算 600±20，施工以 L3 测试迭代定值；偏离原版 480 已知并接受） |
| 3 | tab 条运行时保底 | **不做**（奥卡姆——L1+L3 锁死；未来文案变长测试必红提醒） |
| 4 | 横幅 | **换行自适应重画**（超宽自动折行、高度随行数走，消灭双侧裁） |
| 5 | 范围 | **仅面板窗**；悬浮窗（min 同 480）同病另立 issue 不阻塞 |
| 6 | 验收 | L3 全绿 + **用户实机走查** |
| 7 | 横幅现象 | 以截图为准（zh 完整，真病灶 = 各页内容右缘裁剪；en 横幅随 #4 一并根治） |

## 四、设计

### L1 窗口层——最小窗宽单源化

- 新常量 `PANEL_MIN_WIDTH`（`crates/lt-ui/src/windows/panel/mod.rs`，pub）单源；`app.rs` 的 `min_inner_size` 与默认创建宽 `max(535, MIN)` 共用。
- 实测记录（2026-09-30，内嵌思源字体 = 跨机确定）：595 绿 / 590 红——绑定约束 = **en tab 条自然宽 ≈591px**；取整定 600（下限 + 5px 余量），测试与常量同源，回归必红。

### L2 行布局纪律（成文规则 + 本批行清单）

规则（后续新增行照此自查）：
- **R1**：`hint_line` 提示文案禁与控件 / 按钮同行横排串联——一律独立成行（竖排上下文 label 自动 wrap）。
- **R2**：横排行内固定宽控件（ComboBox ≥240 / TextEdit desired_width ≥240）新增时须自证「行自然宽 ≤ PANEL_MIN_WIDTH 可用宽」；否则改弹性（`available_width` 参与）或拆行。
- **R3**：painter 直画文本（tab 条 / 横幅 / 组框标题）必须以 `layout(text, font, color, wrap_width=可用宽)` 预测行数定高，禁 `layout_no_wrap` + 居中直画（tab 条页签单字定宽除外——页签本身短文本，宽度由 L3 锁）。

本批改造：
1. **横幅**（`mod.rs:185-222`）：`layout(wrap_width=可用宽−16)` 预测行数定高，背景 rect 随高走，galley 居中绘制；点击语义不变。
2. **字体行**（`font_picker.rs:56-112`）：第一行 = label + ComboBox(240) + 「刷新字体列表」按钮；第二行 = 「实际使用字体：×××」label 独立成行（长族名自动 wrap）。
3. **管理行**（`data.rs:213-235`）：`cache_select_hint` 移出横排、独立成行（与下方 `cache_scan_scope_hint` 同款式）。
4. **厂商预设行**（`translation.rs:819-844`）：`preset_hint` 移出横排、独立成行。
5. **不改、交 L3 裁决**：样式预设行 / 字幕背景图片行 / 转录行 / 麦克发行 / benchmark 控制行——最小窗宽抬高后估算全数放得下；L3 若红再按 R1/R2 拆行或抬宽。
6. **tab_strip 不动**：L1 覆盖 zh/en 后 8 页签全放得下，L3 锁死（裁决 #3）。

### L3 机械防线——溢出断言测试

- `panel_no_horizontal_overflow_at_min_width`（`crates/lt-ui/src/windows/panel/mod.rs` tests）：zh/en × 8 页，`RawInput.screen_rect = (PANEL_MIN_WIDTH, 420)`（真实最小高，滚动条出现即最严可用宽），渲染 2 帧后扫全部 `ClippedShape` 的 `visual_bounding_rect()`，断言**左右双缘**均不越视口（±2.0 = 描边 Outside 余量）。
- 语言切换：全局语言表与同二进制其他测试共享（translation.rs 两个 i18n 测试与 debounce 冒烟也 `set_lang`，cargo 默认并行）——每帧渲染后 `get_lang` 复核，被并行翻走即整帧重渲染（8 次有界兜底，宁红不静默按错语言绿）。
- 横幅可见性随本机缓存状态浮动不影响断言——两种态都须干净。

## 五、验收

1. `cargo test --workspace` 全绿（含新溢出测试）+ `precommit.ps1` 三项过。
2. 用户实机走查：拖到最窄逐 tab 查右缘完整；切 en 复核；缺模型横幅换行观感。

## 六、遗留

- 悬浮窗（Overlay min 480 同病）→ 另立 GitHub issue。
- 模型行按钮内长 URL 截断（按钮内 truncate，诚实降级）→ P3 不动。
- egui::Window 模态（模型编辑器 default_width 600）→ 小屏 clamp 后可用宽足够，不在断言面。
