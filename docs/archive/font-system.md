# 字体系统全链路改造施工计划（font-system）

> 【已归档】阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档。2026-09-07 起本版不再追求与原版 1:1，Python 原版仅作行为参考；本文仅作决策史，不再作为施工依据（活跃文档见 AGENTS.md 与 docs/README.md）。
> W-1~W-7 已全部完成（D-17/R-14 已回写决策史）。


> 制定日期：2026-09-07　基线：HEAD `a395862`（291 测全绿，release 59MB 单 exe）
> 制定方式：双面核实——① lt-ui/lt-proto/lt-app 现有代码逐点读证（文中所有 file:line 均以基线为准，执行时先 Read 再改）；② egui 0.36.1 / epaint 0.36.1 源码能力核实（skrifa 字体栈、FontFamily::Name、FontData API）；③ Python 原版副本 `LiveTranslate/` 字体用法清点。
> **本文档是后续会话的施工依据。** 按 W-1～W-5 推进，每 WP 独立可交付、可测试、可提交。

---

## 0. 使用说明（执行会话必读）

### 0.1 怎么用本文档

- 按 W-1 → W-5 顺序推进；W-1（纯资产）与 W-2（纯契约）可先行，两者互不依赖但都先于 W-3。
- 每个 WP 内含：**目标 → 现状证据 → 改造步骤（文件与函数级）→ 边界 → 完成标准 → 风险**。**行号以基线 `a395862` 为准，执行时以实际代码为准（先 Read 再改）。**
- 遇到「⚑ 决策点」：本文档已含 2026-09-07 用户裁决的三个结论（见 §2.1），其余默认为已定方案；执行中若出现偏离需求的新局面，停下来问用户，不要自行拍板。
- 收尾规约沿用 AGENTS.md：`cargo test --workspace` 全绿 → 自主中文 commit（`feat(scope): 主题`、docs 类用 `docs(scope): 主题`）→ 提交前 `git status`/`git diff` 复核。多会话并行时只改本 WP 涉及文件。

### 0.2 参照基准口径（三层）

1. **1:1 权威** = 工作区 `LiveTranslate/` Python 副本（用户 2026-09-06 明确）。本文档"原版行为"均引该副本 `file:line`。
2. **已裁决偏差** = rewrite-research.md §1.5 的 D-1～D-16。本文档新增偏差 **D-17**（字体默认值/级联语义，用户 2026-09-07 裁决），执行时必须同步回写 rewrite-research.md。
3. **新风险** = rewrite-research.md R-14（系统字体依存风险），执行时必须同步回写。

### 0.3 硬规约（每个 WP 都适用）

| 规约 | 出处 |
|---|---|
| egui repaint 回环：`if resp.repaint && !matches!(event, WindowEvent::RedrawRequested) { window.request_redraw(); }` | AGENTS.md 坑 1 |
| `ctx.fonts()`/`fonts_mut()` 是闭包 API；`Color32` 仅 premultiplied 常量可 const | AGENTS.md 坑 2 |
| lt-ui 内引用 windows crate 必须写 `::windows::`（`crate::windows` 模块遮蔽） | AGENTS.md 坑 3 |
| **不得扩 lt-proto 契约来承载 UI 能力**：本文档对 lt-proto 的改动仅是 Settings 数据键（有 `models_dir` 先例，见 §4.2.4），**不新增任何 Cmd/Event 变体** | AGENTS.md 分层规则 |
| edition 2021 if-let 锁临时值自死锁：共享锁+分支派发先绑定再分支 | AGENTS.md 坑 11 |
| i18n zh/en 两份 yaml 必须同步修改 | AGENTS.md 约定 |
| 新增资产必须记 `assets/SOURCES.md`（来源 + sha256） | rewrite-plan.md §2.5 |

---

## 1. 背景与问题

### 1.1 现状（为什么改）

当前字体全量依赖系统文件，`crates/lt-ui/src/app.rs:1453-1551` 的 `install_cjk_fonts` 在启动时依次尝试读取：

| 用途 | 文件 | 现状 |
|---|---|---|
| 等宽 chrome（悬浮窗标题/统计行） | `C:\Windows\Fonts\consola.ttf` | 读到才装，缺则回退默认 |
| 中文（Proportional 族首） | `msyh.ttc` → `msyh.ttf` → `simhei.ttf` | 微软雅黑，Win7 起一般存在但非保证 |
| 韩文（族尾回落） | `malgun.ttf` → gulim/dotum/batang | Win7 无韩国语补丁即缺 → **方块**（a395862 已加回落，仍依赖系统存在） |
| 符号 ✓ ✗ ▲ ▼ | `seguisym.ttf` | Win10+ 一般存在 |

问题链：

1. **缺字形即方块**：已有实锤（`ko - 한국어` 三格方块，a395862 修复）；缺雅黑/缺符号字体的系统同样会方块。
2. **渲染不可复现**：字体度量随系统版本漂移（雅黑 0.87 版 vs 0.98 版、回退默认字体），换行/宽度/视觉对齐在别的机器上不同——对"原版 1:1 parity"项目是系统性误差源。
3. **契约里 4 个字体键是装饰性的**：`style.original_font_family`、`style.translation_font_family`、`subtitle.lines[].font_family` 在 UI 里可编辑（`panel/style.rs:255-281` TextEdit、`panel/subtitle_page.rs:481` TextEdit），但渲染端**全部无视**——`subtitle.rs:298`/`overlay.rs:464-466` 一律 `FontId::proportional(...)`，`subtitle.rs:19` 注释明写"行级 font_family 走全局 CJK 字体栈"。用户填了不生效。
4. **测试依赖本机字体**：`app.rs:1539-1557` 的 hangul 回归测试读 `C:\Windows\Fonts\malgun.ttf`，缺失即跳过（假绿）。

### 1.2 原版参照（权威副本）

- 字幕窗行字体：`LiveTranslate/subtitle_window.py:131` `QFont("Microsoft YaHei", 24)`、`:182` `QFont(cfg.get("font_family", "Microsoft YaHei"), cfg.get("font_size", 24))`——行级字体真实生效（Qt 系统字形回退渲染，缺字即方块）。
- 悬浮窗样式默认：`LiveTranslate/subtitle_overlay.py` DEFAULT_STYLE 的 `original_font_family`/`translation_font_family` = "Microsoft YaHei"（契约锚点 `crates/lt-proto/src/settings.rs:315-316`）。
- 字幕窗默认行：`LiveTranslate/subtitle_window.py:68,85` 两行 `font_family: "Microsoft YaHei"`（契约锚点 `settings.rs:356`）。
- 面板 chrome：`LiveTranslate/main.py:146` `p.setFont(QFont("Consolas", 28, QFont.Weight.Bold))`——等宽 chrome 用 Consolas。
- **原版没有任何字体选择器**（面板为 Qt 默认字体），用户不可调不可预览——本计划是**全新增能力**（D-17 附带项），不是对齐原版。

### 1.3 目标（用户 2026-09-07 裁决）

1. **默认全盘统一内嵌思源**：界面、悬浮窗原文/译文、字幕窗行字体，默认值全部指向内嵌开源思源黑体（Noto Sans CJK SC = Source Han Sans SC，OFL 1.1），所有机器渲染一致、永不方块。
2. **独立可调（人性化）**：用户可分别调整「界面字体」「字幕字体」两把主旋钮；行级（悬浮窗原文/译文、字幕原文/译文行）支持跟随主设置或独立指定，**所有字体选择自带预览**。
3. 系统字体自动探查（Windows 注册表），选择器类似 GUI 终端（搜索 + 预览 + 刷新）。

---

## 2. 设计总览

### 2.1 三个已裁决决策（⚑ 无需再问）

| # | 决策 | 依据 |
|---|---|---|
| D-1 | 默认覆盖范围 = **全盘**（界面 + 悬浮窗 + 字幕行全部默认思源，含契约默认值改写） | 用户 2026-09-07："字体这块默认内嵌就应该做成全部统一默认的" |
| D-2 | 旋钮粒度 = **两把主旋钮**（`ui_font_family`、`subtitle_font_family`）+ 行级跟随/覆盖 | 用户 2026-09-07："分别调整 UI 界面字体、字幕字体" |
| D-3 | 行级键语义 = **级联**（非拷贝）：行级键为空 `""` = 跟随主设置；显式非空 = 独立指定 | 本计划推荐并经用户确认方向；理由见 §4.2.3 |

### 2.2 级联模型（本设计的核心）

```
                    ┌──────────────────────────┐
  ui_font_family ──►│ 界面字体（面板/控件/菜单） │
                    └──────────────────────────┘
                    ┌──────────────────────────┐
 subtitle_font_family►│ 字幕/悬浮窗显示文本主字体 │
                    └──────────────────────────┘
        ▲ 级联：行级键 == "" 时生效
        │
   ┌────┴─────┬───────────────┬────────────────┐
 style.original_font_family │ style.translation_font_family │ subtitle.lines[].font_family
```

- 渲染解析：`resolve_family(row: &str, master: &str) -> &str = if row.is_empty() { master } else { row }`。
- **旧文件三级语义**（`settings.json` / 原版导入）：

| 文件状态 | 解析结果 |
|---|---|
| 无新键 / 键缺省 | 默认思源（`"Noto Sans CJK SC"`），与 D-1 一致 |
| 旧文件显式 `"Microsoft YaHei"`（原版导入） | 尊重原值 → 有雅黑系统渲染雅黑（1:1），无雅黑系统回落内嵌思源（不方块） |
| 行级键显式 `""` | 跟随主设置 |

### 2.3 分层改动图

```
assets/fonts/           lt-proto/src/settings.rs        lt-ui/src/fonts.rs (新)
┌──────────────────┐   ┌──────────────────────────┐   ┌─────────────────────────────┐
│ NotoSansCJKsc-   │   │ Settings.ui_font_family   │   │ 常量/系统扫描/级联解析/       │
│ Regular.otf (嵌) │   │ Settings.subtitle_font_   │   │ 链构建/缓存/字形校验          │
│ OFL.txt          │   │ family (新)               │   │……──── apply_fonts(ctx)      │
└──────────────────┘   │ 行级 4 键默认值 → "" (D-17)│   └──────┬──────────────────────┘
        ▲ include_bytes│                          │          │ ctx.set_fonts(共享单 Context)
        └──────────────┴──────────────────────────┘          ▼
                                  panel 样式页「字体」组 + 行级 font_picker_row（预览/搜索/刷新）
                                                  │ mark_settings_dirty（300ms 防抖 ApplySettings 落盘）
                                                  ▼
                        subtitle.rs / overlay.rs 按 resolve_family 取 FontFamily::Name 渲染
```

---

## 3. 现状证据（已核实，执行时可不再重复调研）

### 3.1 契约与设置链路

- `Cmd::ApplySettings(Box<Settings>)` / `PersistSettings(Box<Settings>)`：`crates/lt-proto/src/events.rs:83-85`。
- 面板改动 → 落盘全链路：`panel/mod.rs:85` `mark_settings_dirty` → `state.rs:1420-1425` `schedule_panel_apply[_at]`（防抖 300ms）→ `app.rs:610` `on_panel_apply_tick` → `Cmd::ApplySettings` → `crates/lt-app/src/backend.rs:40-44` 更新镜像并转发 → UI 循环/AppShell 落盘。
- `Settings` 定义：`crates/lt-proto/src/settings.rs:38-121`（`#[serde(default, rename_all="snake_case")]`，`Default` 手写）；旧文件缺键天然回退默认（`from_value_compatible` `serde_json::from_value().unwrap_or_default()` + `sanitize()`，`settings.rs:126-147`）；原版导入兼容已有测试 `settings.rs:439-556`。
- Rust 专属键先例：`models_dir: Option<PathBuf>`（`settings.rs:78-79`，注释明写"Rust 版新增"）——**冻结契约库允许承载 Rust 专属数据键**，本计划的新增键按同一先例处理。

### 3.2 渲染与字体栈

- 全窗口共享单个 `egui::Context`：`app.rs:58-66` `MultiWindowApp{ ctx, painter }`，Painter 由 `ctx.clone()` 构造；各窗口经 `self.ctx` 进入 → **`ctx.set_fonts()` 一次生效所有窗口**（热切换无架构障碍）。
- egui/epaint 0.36.1 字体能力（cargo registry 源码核证）：
  - 字体解析引擎 = **skrifa**（`epaint-0.36.1/src/text/font.rs` `use skrifa::{GlyphId, MetadataProvider};` + `vello_cpu`），OTF/CFF/TTC/变量字体全支持——思源 OTF（CFF 轮廓）可直接内嵌。
  - `FontFamily::Name(Arc<str>)`（`epaint-0.36.1/src/text/fonts.rs:97`）：按名注册 `FontDefinitions.families` 后可按族名渲染——行级字体分列渲染的关键。
  - `FontData { font: Cow<'static,[u8]>, index: u32, tweak }`，`from_static(&'static [u8])` 零拷贝（内嵌用）、`from_owned(Vec<u8>)`（系统字体用）、`index` 指定 TTC face（默认 0，与现状 `msyh.ttc` 同样语义）：`epaint-0.36.1/src/text/fonts.rs:118-144`。
  - 字形存在性查询：`ctx.fonts_mut(|f| f.has_glyphs(&FontId, s))`（`epaint-0.36.1/src/text/fonts.rs:858-863`；注意 `FontsView` 走 `fonts_mut` 而非 `fonts`）。
- 渲染触点：`subtitle.rs:298`（`FontId::proportional(pt(cfg.font_size))`，行宽测量 `text_width(ui, &font, s)` 已按 `&FontId` 参数化 `subtitle.rs:71-75`）、`overlay.rs:464-466`（head_font/trans_font）；chrome 行 `overlay.rs:132,350` 用 `FontId::monospace`（Consolas，本计划不改）。
- 现值换算既有约定：`subtitle.rs:59-61` `pt(u32) = size * 4/3`（pt→px）。

### 3.3 UI 现状

- 样式页（7 Tab 之一）：`panel/style.rs`——`group_card(ui, pal, &t("group_preset"))` 等 4 组结构；字体行 `font_row`（`style.rs:255-281`）= label + `TextEdit::singleline(200px)` + `t("label_resolved_font")` 弱提示（当前提示是假的：直接回显输入值，未做解析）。
- 字幕页：`subtitle_page.rs:42` 行预览文案 `{family} {size}pt`、`:481` 行字体 `TextEdit`。
- 既有下拉样例：`vad.rs:285-297` `ComboBox::from_id_salt("panel_asr_lang")`；`subtitle_page.rs` `combo_index` 工具（样式页也复用）——选择器外观与此对齐。
- 面板所有文字走 Proportional 族（默认 egui 字体 + 雅黑族首）；**无一个独立把"面板字体"接进设置的键**。

### 3.4 测试现状

- `lt-proto/src/settings.rs:546-556` `style_and_subtitle_defaults_match_original`：断言 `Style`/`SubtitleLine` 默认 == 原版（含 "Microsoft YaHei"）——**D-17 后必须修改**（改断言为 D-17 默认值并注原版对照值）。
- `lt-ui/src/app.rs:1539-1557` `hangul_syllables_resolve_via_system_font_fallback`：读 `C:\Windows\Fonts\malgun.ttf`，缺则跳过——**改为内嵌链字形覆盖测试**（机器无关，见 §5）。
- 其余字体相关测试仅为数据往返（`state.rs:1955` SimHei 是测试数据，无生产硬编码；`subtitle.rs:655` 是字符串逻辑测试）。

### 3.5 平台 API（已核证）

- `windows` crate = **0.62.2**（workspace，`Cargo.toml:35`）；lt-ui 已启用 `Win32_System_Registry` 特性（`crates/lt-ui/Cargo.toml:36`，现有用途：开机自启 HKCU Run 键）——**字体扫描零新依赖**。
- 注册表枚举 API（`windows-0.62.2/src/Win32/System/Registry/mod.rs`）：
  - `RegOpenKeyExW(hkey, lpsubkey, None, KEY_READ, &mut hkey_out) -> WIN32_ERROR`
  - `RegEnumValueW(hkey, index, name_buf: Option<PWSTR>, &mut name_len, None, None, data: Option<*mut u8>, Some(&mut data_len)) -> WIN32_ERROR`（`ERROR_NO_MORE_ITEMS` 结束）
  - `RegQueryValueExW`（本方案直接读值为 `REG_SZ` 字符串路径，可用它或 RegEnumValueW 顺带取数据——取数据路径更省一次查询）
  - `RegCloseKey(hkey)`
- 字体键路径：`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts` + `HKCU` 同路径。

### 3.6 资产现状

- `assets/` 现有：`ort/onnxruntime.dll`（17.7MB）、`silero_vad.onnx`（2.3MB）、`icons/`（442KB）、`reference/`（556KB）、`i18n/`、`SOURCES.md`（资产来源记录，含 silero/ort/icons 条目与规约示例）；**无 fonts/ 目录**。
- `assets/SOURCES.md` 记录规约：来源、版本、大小、sha256（见其 silero_vad.onnx 条目：2,327,524 字节 + sha256 的样式）。

---

## 4. 详细设计

### 4.1 W-1 资产：内嵌思源黑体 SC Regular

**目标**：把 OFL 1.1 的思源黑体（Noto Sans CJK SC）Regular 单字重静态 OTF 收入 `assets/fonts/`，并把来源/校验信息记入 `assets/SOURCES.md`。

**采购**：

- 来源：GitHub `notofonts/noto-cjk` 发布 **Sans2.004** 的资产 `08_NotoSansCJKsc.zip`（unzip 内含 7 字重：Thin/Light/Regular/Medium/Bold/Heavy/Black，全字形库——含汉字/假名/韩文音节/拉丁，区域差异仅是 Han 缺省字形归属）。发布页：`https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004`。
- 取用 `NotoSansCJKsc-Regular.otf`（CFF 轮廓，skrifa 可渲染）；**同一设计** = Adobe Source Han Sans SC（思源黑体），OFL 1.1 可自由嵌入商业闭源分发（须随附 OFL.txt）。
- 文件落位（git 入库）：

```
assets/fonts/NotoSansCJKsc-Regular.otf   （约 13–16MB，以实际为准）
assets/fonts/OFL.txt                     （SIL Open Font License 1.1 全文，随附许可）
```

- 校验：`certutil -hashfile assets/fonts/NotoSansCJKsc-Regular.otf SHA256` 记入 SOURCES.md；SOURCES.md 新条目模板：

```markdown
## fonts/NotoSansCJKsc-Regular.otf

- 字体：Noto Sans CJK SC Regular（= Source Han Sans SC，思源黑体；3 语言区域 Han 缺省字形归属为 SC）
- 来源：notofonts/noto-cjk 发布 Sans2.004，资产 08_NotoSansCJKsc.zip 内解出
  `https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004`
- 大小：<实际字节>
- sha256：<certutil 实测>
- 许可：SIL Open Font License 1.1（OFL.txt 随附；允许内嵌商业分发）
```

**为什么单字重 / 不子集**：

- egui 无粗体合成（Bold 为假强调，`RichText::strong` 不换字形）；UI 全部强调弱化由字号/颜色承担——Regular 单字重足够（与原版视觉重量本就不同，D-17 一并归档）。
- 不静态子集：字幕/悬浮窗渲染任意语音文本，字符集不可预知；全字库（约 65k 码位）是要保底的面。
- 此抉择代价：exe 59MB → 约 74MB（体积已由用户接受，属"无脑垄断"代价）。

**边界**：只嵌 `SC` 一个区域；日文汉字按 SC 缺省字形渲染（如「語」等地区性字形差异）——可接受，属 D-17 标注的渲染差异。embed 时**不**嵌 JP/KR 区域包（+~30MB，收益小）。

**完成标准**：两个文件入库；SOURCES.md 有记录；`cargo build --release -p lt-app` 成功且体积 +13~16MB 量级；后续 W-3 的 `has_glyphs("中文日本語한국어")` 测试可过。

**风险**：CFF 与 skrifa 意外不兼容（理论无）→ 兜底换 Google Fonts `NotoSansSC-Regular.ttf`（同设计 TTF 转栅格）：`https://fonts.google.com/specimen/Noto+Sans+SC`；两版任选且 SOURCES.md 记录最终版本。

### 4.2 W-2 契约：新增两键 + 行级键默认值改写（D-17）

**目标**：`lt-proto/src/settings.rs` 增加 `ui_font_family`/`subtitle_font_family`，行级 4 键默认改 `""`（跟随），并落地 D-17 注释与测试更新。**不新增任何 Cmd/Event 变体**。

**4.2.1 新键定义**（`Settings` 结构体，加在 `ui_lang` 附近"─ UI ─"段）：

| 键 | 类型 | 默认 | 序列化 | 语义 |
|---|---|---|---|---|
| `ui_font_family` | `String` | `"Noto Sans CJK SC"` | 恒写盘（与现有标量一致） | 界面字体族名（可为系统字体名） |
| `subtitle_font_family` | `String` | `"Noto Sans CJK SC"` | 同上 | 字幕/悬浮窗显示文本主字体族名 |

- 两侧都走 `#[serde(default)]`（结构体级已开）+ `Default::default()` 手写值——旧文件缺键自动回退，**零迁移代码**。
- `sanitize()`（`settings.rs:150-209`）追加：`ui_font_family`/`subtitle_font_family` 为空串 → 回退默认并记入 `fixed`（防手改/损坏 JSON 写空）。

**4.2.2 行级键默认值改写**（D-17 核心）：

| 键 | 原默认 | 新默认 | 语义 |
|---|---|---|---|
| `style.original_font_family`（`settings.rs:315`） | `"Microsoft YaHei"` | `""` | 空 = 跟随 `subtitle_font_family` |
| `style.translation_font_family`（`settings.rs:316`） | `"Microsoft YaHei"` | `""` | 同上 |
| `SubtitleLine::default().font_family`（`settings.rs:356`） | `"Microsoft YaHei"` | `""` | 同上（两行默认行均生效，`default_pair` 用 `..Default::default()` 继承） |

- 每处修改**必须**附注释：`// D-17（2026-09-07 用户裁决）：空串=跟随 subtitle_font_family；原版默认 "Microsoft YaHei"`。
- **`lt-ui/src/style.rs` 的 `preset_style`（:167-174）基于 `Style::default()` 生成**——无需单独改，自动继承新默认（切预设/恢复默认同步变成"跟随"）。

**4.2.3 为什么级联而不用拷贝**（决策记录）：

- 拷贝模型（用户改主键时把 `subtitle_font_family` 写入全部行级键）：改主键会**覆盖**行级定制（数据破坏）；且落盘后主键与行级出现两份真相。
- 级联模型：主键改 → 所有未定制行立即跟随；行级定制不丢；旧文件显式值被尊重（1:1 导入保持）。
- 新增语义仅"空串 = 跟随"，与既有 `Option`/`None` 惯用法一致，解析为单点纯函数（`resolve_family`），可测试。

**4.2.4 冻结契约注记**：lt-proto 是数据契约，本次改动在既有 `models_dir` 先例（`settings.rs:78-79`）之下承认 Rust 专属数据键；**不触碰** `events.rs` 的 Cmd/Event、不动 `asr_result.rs`。执行时在 `settings.rs` 头部注释的"兼容策略"段补一行：`Rust 版新增键：ui_font_family / subtitle_font_family（D-17，原版无对应键，缺省回退默认）。`

**边界**：`ui_lang`（界面语言）等既有键语义不动；`Style` 其余 12 键（颜色/字号/不透明度）不动；`SubtitleLine` 其余 14 键不动。

**测试**（`settings.rs` tests 模块）：
- 新增 `default_font_keys_follow_embedded_source_han`：`Settings::default()` 两新键 == `"Noto Sans CJK SC"`；`Style::default()`/`SubtitleLine::default()` 行级 == `""`；`default_pair()` 两行 === ""。
- 修改 `style_and_subtitle_defaults_match_original`（:546-556）：保留"其余键与原版一致"断言，字体三键改为 D-17 断言并注释原版值。
- 新增 `legacy_yahei_values_respected`：`from_value_compatible(json!({"style":{"original_font_family":"Microsoft YaHei"}}))` → 行级保留 `"Microsoft YaHei"`（旧文件 1:1 尊重）；空串缺键 → 默认跟随。
- 新增 `font_family_empty_sanitized_to_default`：构造两个新键为 `""` 的 JSON → `sanitize()` 后回默认且 `fixed` 非空。
- 新增 roundtrip：`serde_json::to_value(Settings::default())` 含两新键。

**完成标准**：新增/修改测试全绿；`from_value_compatible` 兼容原版文件行为不变（既有 439-521 段测试不动）。

### 4.3 W-3 lt-ui 字体模块 + 选择器 UI（核心）

**目标**：新建 `crates/lt-ui/src/fonts.rs`，以"内嵌思源 + 系统扫描 + 级联解析 + 运行时热应用"取代 `install_cjk_fonts`；样式页新增「字体」组（两把主旋钮 + 预览）；行级字体行接入统一选择器。

**4.3.1 新模块 `crates/lt-ui/src/fonts.rs` 规格**

常量（单源）：

```rust
/// 内嵌思源黑体（W-1 入库，OFL 1.1）；零拷贝（FontData::from_static）
pub const EMBEDDED_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJKsc-Regular.otf");
/// 内嵌字体「展示族名」（注册进 FontFamily::Name / 显示在选择器首项）
pub const EMBEDDED_FAMILY: &str = "Noto Sans CJK SC";
/// 字体样例（UI 预览卡 + 字形覆盖测试共用同一常量，防漂移）
pub const SAMPLE_TEXT: &str = "天地玄黄 宇宙洪荒 The quick brown fox 한국어 어둠 0123456789";
/// 内嵌思源作为回退链尾的注册名
pub const EMBEDDED_FALLBACK_KEY: &str = "sans-cjk-sc";
```

- Comment banner: 模块说明——取代 install_cjk_fonts 的系统字体依赖；分段"内嵌/扫描/解析/链构建/缓存/校验"。

扫描与规范化（纯函数与 IO 分离）：

```rust
pub struct SystemFont { pub display: String, pub path: PathBuf }   // display = 清洗后族名
/// 纯函数：注册表 (值名, 值数据) 原始对 → 系统字体列表
/// 规则：值名去后缀 " (TrueType)"/" (OpenType)"/" (All res)"；值数据相对路径→拼
///       SystemRoot；过滤 .ttf/.ttc/.otf；族名大小写不敏感去重（HKCU 优先）；
///       与 EMBEDDED_FAMILY 同族名的条目丢弃（内嵌置顶，无需系统重复项）。
pub fn normalize_registry_entries(raw: &[(String, String)]) -> Vec<SystemFont>
pub fn scan_system_fonts() -> anyhow::Result<Vec<SystemFont>>
```

- `scan_system_fonts` 内部：`RegOpenKeyExW(HKEY_LOCAL_MACHINE|HKEY_CURRENT_USER, "...Windows NT\\CurrentVersion\\Fonts", None, KEY_READ, ...)` → `RegEnumValueW` 循环（名字缓冲 1024 字符、数据缓冲 4096；`ERROR_NO_MORE_ITEMS` 结束；`RegCloseKey` 收尾——沿用 `crate::windows` 遮蔽注意，引用写 `::windows::`（AGENTS.md 坑 3））。SystemRoot = `std::env::var("SystemRoot")` 兜底 `r"C:\Windows"`。
- 扫描是注册表枚举（<10ms），启动一次 + 「刷新」按钮重扫；失败/空 → `tracing::warn!` + 仅内嵌可用（降级不崩）。

级联解析（纯函数）：

```rust
/// 行级键 → 实际族名：空 = 跟随主设置（D-17 级联模型）
pub fn resolve_family<'a>(row: &'a str, master: &'a str) -> &'a str {
    if row.is_empty() { master } else { row }
}
```

缓存与链构建：

```rust
pub struct FontsState {
    pub system: Vec<SystemFont>,                 // 扫描结果（AppState 持有，重启可重扫）
    pub loaded: HashMap<PathBuf, Arc<FontData>>, // 懒加载：仅选中/被注册的族才读字节
}
pub fn apply_fonts(ctx: &egui::Context, settings: &Settings, fonts: &mut FontsState)
```

- 缓存 **`Arc<FontData>`**（而非 `Arc<Vec<u8>>`）：每次热应用重建 `FontDefinitions` 时 `font_data` 直接 `Arc::clone`，系统字体不重复拷贝（FontData::from_owned 仅在首次读文件时发生）；内嵌用 `FontData::from_static(EMBEDDED_BYTES)` 零拷贝。
- `build_definitions`（`apply_fonts` 内部或拆出以便测试取 `FontDefinitions`）：

| 族 | 链（优先级从高到低） |
|---|---|
| `Proportional` | [选中界面字体（内嵌或系统，注册失败跳过）] → [内嵌思源 `sans-cjk-sc`] → [内嵌符号 `sans-symbols`] → [egui 默认族尾] |
| `Monospace` | [Consolas（系统，锦上添花）] → [内嵌拉丁等宽 `sans-mono`（Noto Sans Mono，W-7：CJK 回落思源——与有 Consolas 机器现状一致）] → [选中界面字体] → [内嵌思源] → [内嵌符号] → [默认族尾] |
| `Name(族名)`（行级/字幕字体） | [该族字体字节] → [内嵌思源] → [内嵌符号] |
（W-6 修订：seguisym 移除——符号由思源+Noto Sans Symbols 2（✗ 等）内嵌覆盖；Consolas 保留为系统增强，缺省回退内嵌等宽；系统符号字体不再读取。）

- **命中集**：`ui_font_family`、`subtitle_font_family`、全部非空行级键（style 两键、字幕行键、行级预设）→ 每个族名 = 一把"手柄"。解析流程：内嵌族名 → 直接映射 `EMBEDDED_BYTES`；否则在 `system` 里按族名（大小写不敏感）找路径 → 读文件入缓存；**找不到 → 不注册该 Name**，渲染自然落回 `Proportional` 全局链（内嵌思源尾仍兜底——永不方块）。
- 渲染侧 `FontFamily::Name(族名)` 的字体键名 = 族名本身（`Name(Arc::from(family))`），一个族名一条 families 链。

**4.3.2 启动接线**（`app.rs`）

- `MultiWindowApp::new`（`app.rs:58-66`）内 `install_cjk_fonts(&ctx)` 替换为：

```rust
let mut fonts = FontsState { system: fonts::scan_system_fonts().unwrap_or_default(), loaded: Default::default() };
fonts::apply_fonts(&ctx, &app_state.settings, &mut fonts);
```

- `AppState`（`state.rs`）新增 `pub fonts: crate::fonts::FontsState` 字段（初始时扫描一次；字段在 AppState 构造处初始化），供面板选择器行控件与渲染侧读取。
- 删除旧 `install_cjk_fonts`（含其 tests 段 hangul 系统字体测试，迁走见 §5）。

**4.3.3 运行时热应用钩子**

- **任何字体键变更（主旋钮或行级）** → 立即 `crate::fonts::apply_fonts(ui.ctx(), &state.settings, &mut state.fonts)`（选择器行控件内可直接拿 `ui.ctx()`——全窗口共享的根 Context，`set_fonts` 下一帧对全部窗口生效）→ `panel::mark_settings_dirty(state)`（300ms 防抖经既有 `ApplySettings` 链路持久化，重启恢复自动成立）。
- egui `set_fonts` 会重建字体/图集（含已用字形栅格重算），仅在用户交互时发生（微秒~毫秒级），可接受；不做额外节流（300ms 防抖已经覆盖高频拖动场景）。

**4.3.4 选择器行控件 `font_picker_row`**（新文件 `crates/lt-ui/src/windows/panel/font_picker.rs`）

```rust
/// 通用字体行：ComboBox（内嵌置顶 + 系统扫描 + 搜索过滤）+ 行内小样 + 可选"跟随"首项
pub fn font_picker_row(
    ui: &mut Ui, state: &mut AppState,
    id: &str, label: &str,
    family: &mut String,              // 行级键的当前值；"" = 跟随
    is_line_level: bool,              // true = 首项「跟随（默认）」
)
```

- 项构造（纯函数供测试）：`fn picker_entries(family: &str, system: &[SystemFont], follow: bool, filter: &str) -> Vec<PickerEntry>`——`follow` 首项 `PickerEntry::Follow`；`EMBEDDED_FAMILY`（标注 `t("label_font_embedded")` 尾部）固定第二项；其余系统族名按 display 字母序；`filter` 子串过滤（大小写不敏感）。
- 布局对齐既有行控件（`style.rs:255-281`）：`ui.horizontal` 内 label + `ComboBox::from_id_salt(id).width(200.0)`（宽 200 对齐现 TextEdit）；尾随「刷新」按钮（`t("btn_font_rescan")`，仅系统字体持有者显示）触发 `scan_system_fonts()` 重扫。
- ComboBox 弹层内：顶部 `TextEdit` 搜索框（过滤实时生效），下方 `ScrollArea` 项列表（数百字体可用，不虚拟化）。
- **预览（选中即所见）**：本行小样 = `RichText::new(SAMPLE_TEXT).size(12.0).font(FontId::new(12.0, FontFamily::Name(Arc::from(resolve_family(family, master)))))`——因为选中族已注册进 `Name` 链，**预览渲染的就是真实字体**；`resolve_family(..., "")` 收到未注册族名时 `FontFamily::Name` 找不到 families 链 → 渲染端统一映射回 `Proportional`（预览与正文同规则，不闪错）。
- **缺字提示**：选中族样本 `has_glyphs` 校验（`ctx.fonts_mut` 闭包），CJK 样例缺失 → 行尾弱提示 `t("font_cjk_fallback_hint")`（"缺字由内嵌思源兜底"），不阻断。
- 选择后行为：写回 `*family`（主旋钮直接写族名；行级选择「跟随」写 `""`）→ `apply_fonts`（立即）→ `mark_settings_dirty`。

**4.3.5 样式页「字体」组**（`style.rs` `page()` 顶部，位于 `group_preset` 之前）

```rust
group_card(ui, pal, &lt_i18n::t("group_fonts"), |ui| {
    font_picker_row(ui, state, "font_ui",     &t("label_ui_font"),      &mut state.settings.ui_font_family, false);
    font_picker_row(ui, state, "font_sub",    &t("label_subtitle_font"), &mut state.settings.subtitle_font_family, false);
    // 说明小字：字幕字体影响悬浮窗原文/译文与字幕窗行级（行级未单独设置时）
    ui.label(RichText::new(t("hint_subtitle_font_scope")).size(11.0).color(ui.visuals().weak_text_color()));
});
```

- 两条主旋钮各自伴随**预览卡**（多行样例：`SAMPLE_TEXT` 中/英/韩/数字各一行，用该族 `FontFamily::Name` 渲染）——"选中即所见"。
- 取消原「恢复默认」对字体键的隐含影响：`reset_style`（`style.rs:196-200`）基于 `preset_style("default")` = `Style::default()` → 行级键变回 `""`（跟随）——语义自洽，**无需额外代码**，但要在重置后 `apply_fonts` 一下（`reset_style` 调用点补钩子，或统一在 `page()` 末尾比对"字体相关键是否有变"再 apply——推荐后者：`page()` 返回后由调用方比对 `Style`/新键快照）。

**4.3.6 行级接入**

- `style.rs` `font_row`（:255-281）**整体替换**为 `font_picker_row`（`is_line_level: true`）：首项「跟随（默认）」`t("label_font_follow")`；`label_resolved_font` 弱提示改为真实解析结果：`t("label_resolved_font").replace("{family}", resolve_family(...))`。
- `subtitle_page.rs:481` 行字体 `TextEdit` → `font_picker_row(is_line_level: true)`；`:42` 行预览文案 `{family} {size}pt` 中的 `{family}` 空值显示为 `t("label_font_follow")`。
- 字幕页行循环处注意：每行独立 `id`（如 `format!("sub_line_font_{i}")`）。

**4.3.7 i18n 新键全表**（zh/en 同步，`assets/i18n/zh.yaml` / `en.yaml`，放"样式/设置"段）：

| 键 | zh | en |
|---|---|---|
| `group_fonts` | 字体 | Fonts |
| `label_ui_font` | 界面字体 | UI interface font |
| `label_subtitle_font` | 字幕字体 | Subtitle font |
| `label_font_follow` | 跟随（默认） | Follow (default) |
| `label_font_embedded` | 内嵌思源黑体 SC | Embedded Source Han Sans SC |
| `btn_font_rescan` | 刷新字体列表 | Refresh font list |
| `font_cjk_fallback_hint` | 该字体缺 CJK 字形，已由内嵌思源兜底 | Missing CJK glyphs; Source Han Sans will fill in |
| `font_scan_failed` | 系统字体扫描失败，仅内嵌字体可用 | System font scan failed; only embedded font available |
| `hint_subtitle_font_scope` | 影响悬浮窗原文/译文与字幕窗行（行级未单独设置时生效） | Applies to overlay original/translation and subtitle lines (when not overridden per line) |
| `label_resolved_font` | 实际使用字体：{family}（改为真实解析值；空=显示跟随文案） | Resolved font: {family} |

**边界**：本 WP 只做"选得中、查得到、渲染链可解析、预览真实"；**行级字体渲染真实化在 W-4**（W-3 只把键接入选择器并保证全局链正确）。

**完成标准**：选择器三处接入（字体组两行、样式文字组两行、字幕页每行）；预览卡在多字体切换下即时变化；重启持久恢复；`cargo test --workspace` 全绿（含 §5 新增）。

### 4.4 W-4 渲染真实化：行级字体键生效

**目标**：悬浮窗/字幕窗按 `resolve_family` 出的族名用 `FontFamily::Name` 渲染。

- `subtitle.rs:298`：

```rust
let fam = crate::fonts::font_family_for(&cfg.font_family, &sm.subtitle_font_family); // helper: 有名→Name，空/未注册→Proportional
let font = FontId::new(pt(cfg.font_size), fam);
```

- `overlay.rs:464-466`：`head_font`/`trans_font` 同样按 `style.original_font_family`/`translation_font_family` + `settings.subtitle_font_family` 解析。
- `subtitle.rs:19` 注释更新：`行级 font_family 按 D-17 级联（空=跟随字幕字体）真实生效；未注册族名回落全局链（内嵌思源兜底）`。
- 行宽测度 `text_width(ui,&font,s)` 已参数化（`subtitle.rs:71-75`），换用同 FontId 即自动一致——**无二次改动**。
- 实现注意事项：`FontFamily::Name(Arc<str>)` 每处构造 Arc 一次（避免每帧 clone 开销）；把 `font_family_for` 缓存结果挂 `FontsState`（family→Option<FontFamily>，键应用时刷新）——**每帧只查 HashM哈图不重复 Arc**。
- chrome（`overlay.rs:132,350` monospace）**不动**（Consolas 保持，与原版 `main.py:146` 一致）。
- 视觉差异归档：全默认思源后与原版雅黑度量不同（行宽/换行阈值变化）——记入 `docs/archive/visual-parity.md` 复核项，面板 535×781 等固定布局尺寸**不调整**（仅文字宽度微差）。

**完成标准**：改两处渲染 + 注释；字幕两行分别选不同字体实机可见（截图：全屏截图验收——egui 窗口 PrintWindow 会抓到旧帧，AGENTS.md 坑 10）；`cargo test --workspace` 全绿。

### 4.5 W-5 测试、文档与档案

**测试改造**（目标：字体相关测试与"本机字体"零耦合）：

| 用例 | 位置 | 断言 | 依赖 |
|---|---|---|---|
| `embedded_chain_covers_cjk_hangul_latin`（**替换** app.rs:1539 hangul 测试） | lt-ui fonts.rs tests | `egui::Context::default` + `begin_pass(RawInput::default())` + `ctx.fonts_mut(has_glyphs(FontId::proportional(16.0), SAMPLE_TEXT))`；**移除 malgun.ttf 存在性跳过** | 仅 include_bytes 内嵌字节 |
| `resolve_family_cascade` | fonts.rs | 空→master；显式→行值；两者同值；空 master（不应发生，防御）→ 兜底默认 | 纯函数 |
| `normalize_registry_entries_*`（～4 例） | fonts.rs | 后缀清洗/大小写去重（HKCU 优先）/相对路径拼接（SystemRoot 注入）/非字体扩展过滤/丢弃同内嵌族名 | 纯函数（喂合成数据） |
| `picker_entries_*`（～2 例） | font_picker.rs | 跟随首项位置；内嵌固定第二；过滤子串；排序 | 纯函数（喂合成 SystemFont） |
| `font_family_for_registered_vs_unknown` | fonts.rs | 已注册名→`FontFamily::Name`；未注册/空→Proportional | 构造 FontsState 可注入 |
| `default_font_keys_*` / `legacy_yahei_values_respected` / `font_family_empty_sanitized` | lt-proto settings.rs | 见 §4.2.4 | 纯数据 |
| `style.preset_reset_follows_master` | lt-ui style.rs | `reset_style` 后行级 == ""；`preset_style("default")` 行级 == "" | 纯函数 |

- 现有 `state.rs:1955` SimHei 测试数据**保留**（仅测往返，与默认值无关）；`subtitle.rs:655` 韩国语字符串测试保留。
- 测试数预估：291 → 约 308（+17 净增；-1 替换）。

**文档**：
- `rewrite-research.md`：**D-17** = "字体默认统一内嵌思源（Noto Sans CJK SC，OFL 1.1）+ 行级级联（空=跟随主设置）+ 新增键 ui_font_family/subtitle_font_family + 系统字体选择器/预览"（原文引用户 2026-09-07 裁决；标注"原版无对应设置，原版默认 Microsoft YaHei 的 1:1 行为变为'显式客制才生效'"); **R-14** = 系统字体依赖（缺失→内嵌兜底；度量差异→parity 重基线；扫描范围=HKLM+HKCU）。
- `AGENTS.md`：字体策略段落（内嵌思源默认/级联/选择器）+ 更新"291 测"为测试后实测数 + 大坑补充（若产生新坑：如 `font_family_for` 每帧 Arc 注意点）。
- `assets/SOURCES.md`：W-1 条目。
- `docs/archive/visual-parity.md`：加复核项"字体度量重基线（思源 vs 雅黑）"。

---

## 5. 执行顺序与提交（每步全绿才提交）

| WP | 提交（约定式中文） | 预估人时 | 关键文件 |
|---|---|---|---|
| W-1 | `chore(assets): 内嵌思源黑体 SC Regular(OFL)+SOURCES.md` | 0.5h | assets/fonts/*, SOURCES.md |
| W-2 | `feat(fonts-contract): Settings 增 ui/subtitle_font_family+行级跟随默认(D-17)` | 1h | lt-proto/settings.rs |
| W-3 | `feat(ui-fonts): 内嵌思源默认+系统字体扫描+字体组选择器/实时预览` | 6-8h | lt-ui/fonts.rs(新), font_picker.rs(新), app.rs, state.rs, style.rs, subtitle_page.rs, i18n×2 |
| W-4 | `feat(ui-fonts): 悬浮窗/字幕行字体键真实生效(D-17 级联)` | 2-3h | subtitle.rs, overlay.rs |
| W-5 | `docs(fonts): D-17/R-14 归档+字体测试机器无关化+测试数更新` | 1-2h | rewrite-research.md, AGENTS.md, visual-parity.md.md, 各测试 |

**冒烟走查（每个含渲染 WP 后必做）**：

```bash
# 1) 全量测试
cargo test --workspace
# 2) 单 exe 构建 + 体积记录
cargo build --release -p lt-app && ls -la target/release/lt-app.exe
# 3) GUI 冒烟（临时 config，models_dir 指向真实模型缓存）
LIVETRANSLATE_CONFIG_DIR=D:/tmp/lt_smoke cargo run -p lt-app
#   走查项：样式页「字体」组三语预览卡正确；选系统字体（如 微软雅黑）立即生效；
#   行级选"微软雅黑"而主键=思源 → 悬浮窗原文行=雅黑、译文行=思源；
#   搜索过滤可用；刷新按钮可用；重启后选择持久；缺字提示出现。
```

**验收清单（DoD）**：

- [ ] `cargo test --workspace` 全绿（测试数与 AGENTS.md 同步）
- [ ] 无 `C:\Windows\Fonts` 依赖的字体测试（全部机器无关）
- [ ] 选择器三处接入 + 两预览卡 + 搜索/刷新 + 缺字提示
- [ ] 悬浮窗/字幕行级字体真实生效（截图验收——全屏截图，非 PrintWindow）
- [ ] 旧文件（原版导入/显式雅黑）行为遵从§2.2 三级语义
- [ ] `settings.json` 重启恢复（含新键）
- [ ] rewrite-research.md D-17/R-14 与 AGENTS.md 已更新
- [ ] exe 体积实测已记录（预期 ~74MB）

---

## 6. 风险与对策

| # | 风险 | 概率 | 对策 |
|---|---|---|---|
| R-14a | 系统字体文件不存在/字体名漂移 | 中 | 内嵌思源恒在链尾；未注册族名不注册，回落全局链（永不方块） |
| R-14b | 扫描注册表异常/权限（策略禁读） | 低 | `scan_system_fonts` 返回 Err → 降级仅内嵌 + `font_scan_failed` 提示 |
| R-14c | 字体度量差异影响视觉对齐（思源 vs 雅黑） | 确定发生 | D-17 归档 + visual-parity 复核项；固定布局尺寸不调 |
| R-14d | 用户选无 CJK 系统字体 | 中 | 缺字提示（非阻断），内嵌兜底保证可读 |
| R-1（体积） | exe +13~16MB | 确定发生 | 用户已认可（D-17 记录）；单字重已是最小可行 |
| — | CFF 与 skrifa 兼容意外 | 低 | 兜底 Google Fonts NotoSansSC-Regular.ttf（同设计） |
| — | TTC 系统字体仅 index 0 | 低 | 与现状 msyh.ttc 同语义；选择器展示族名=index 0 face |
| — | 数百系统字体的列表性能/内存 | 低 | 只存 (族名,路径) 字符串；字节仅选中/注册时懒加载；Arc<FontData> 复用 |
| — | 用户自装字体不在注册表（第三方字体管理） | 低 | 文档注明扫描范围=HKLM+HKCU；限制可接受 |
| — | `set_fonts` 高频重建成本 | 低 | 仅用户交互触发；300ms 防抖已覆盖连续操作 |
| — | 杀软对 exe 内嵌资源报毒 | 无关 | 字体内嵌为常规资源，不在本计划范围（与既有 ort 内嵌同一暴露面） |

---

## 7. 附录

### 7.1 关键 API 签名速查（windows 0.62.2）

```rust
use ::windows::Win32::System::Registry::{
    RegCloseKey, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_VALUE_TYPE,
};
// RegEnumValueW(hkey, dwindex, Some(PWSTR(buf)), &mut len, None, None, data.0, Some(&mut data_len))
// 循环直到返回 ERROR_NO_MORE_ITEMS（windows::Win32::Foundation::ERROR_NO_MORE_ITEMS，用 WIN32_ERROR 比较；注意 0x103 值）
```

### 7.2 链构建伪码（W-3 参考实现）

```rust
fn build_definitions(settings: &Settings, system: &[SystemFont], cache: &mut FontCache) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    // 1) 收集手柄：ui_family + subtitle_family + 全部非空行级 + EMBEDDED_FAMILY
    // 2) 内嵌：font_data.insert(EMBEDDED_FALLBACK_KEY, FontData::from_static(EMBEDDED_BYTES))
    // 3) 系统：逐手柄在 system 按名找 → cache 取 Arc<FontData>；找不到 → 记 warn，不注册
    // 4) 链装配：Proportional / Monospace / Name(族名) 三表（见 §4.3.1 表格）
    // 5) Consolas/seguisym 存在性读取（沿用旧 install_cjk_fonts 的容错写法，集中到一处）
    fonts
}
```

### 7.3 决策点备案（执行中若偏移，均需回停）

- ⚑ 若行级键留白语义（`""`）与未来的"字体大小/行距"扩展冲突，本设计的 `resolve_family` 单点可平移（扩展新键再 cascode 一层），不影响已落盘数据。
- ⚑ 「界面字体」是否也作用于 Monospace chrome：**否**（Consolas 保持，原版 1:1）；如需"等宽字体"第三把旋钮，属后续扩展，本计划不实现。

---

## 8. 关联文档索引

- 契约/偏差权威：`rewrite-research.md`（D-17、R-14 待回写）
- 施工总图：`rewrite-plan.md`
- 字体相关既有方案：`docs/archive/overlay-realign.md`、`docs/archive/visual-parity.md`
- 本计划执行纪要（完成后追加）：`docs/archive/font-system.md` 底部"执行纪要"段（每 WP 一行：commit 号 + 测试数 + 遗留项）

---

## 9. 执行纪要（2026-09-07 施工）

| WP | commit | 测试数 | 遗留项 |
|---|---|---|---|
| W-1 资产 | `ddfa8ae` | —（纯资产） | 无（SOURCES.md 已记来源/sha256/OFL） |
| W-2 契约 | `8be585a` | 294 | 无；`style_and_subtitle_defaults_match_original` 已按 D-17 改断言 |
| W-3 字体模块+选择器 | `a2ae876` | 303 | 行级选择器在模态内即时预览依赖上次 apply（提交后生效），可接受；字体列表未做虚拟化（搜索框缓解） |
| W-4 渲染真实化 | `29155a6` | 303 | chrome（Overlay 头/统计行）保持 Consolas 不动（原版 1:1）；TTC 系统字体仅 face 0 |
| W-5 文档 | 随本提交 | 303 | docs/archive/font-system.md 附录 A 决策点三项均已按用户裁决落地，无未决 |
| W-7 体积压缩 | （随本提交） | 305 | brotli(q11) 入库 ~12.7MB（原 34MB）运行时一次性解压（字形零损失，启动 +~100ms）；等宽 chrome 换拉丁 Noto Sans Mono（CJK 回落思源=有 Consolas 机器现状）；区域子集经实测缺失韩文音节（한국어）已否决不采纳；exe ~92MB → ~70MB |
| W-6 仓库自洽 | （随本提交） | 305 | 收敛系统依赖：内嵌等宽 MonoCJK + 符号 NotoSansSymbols2 取代 seguisym（✗ 由符号字体补足）；Consolas 仅作系统增强（原版 1:1，缺则内嵌等宽一致）；注册表扫描 cfg(windows) 门控（非 Windows 空列表，仓库仍可完整渲染）；覆盖测试改 skrifa 解析级（epaint has_glyph 对 replacement-face 有启发式假阴性，见其 TODO） |
