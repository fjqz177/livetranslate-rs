# 悬浮窗（主界面）对齐原版方案——2026-09-06

> 【已归档】阶段一（Python 1:1 复刻期，2026-09-05～09-07）文档。2026-09-07 起本版不再追求与原版 1:1，Python 原版仅作行为参考；本文仅作决策史，不再作为施工依据（活跃文档见 AGENTS.md 与 docs/README.md）。
> 主要修复已随提交 2e94bf3 与视觉工作包入库。


> 范围：**仅主界面**（程序启动时打开的黑色半透明悬浮窗）。控制面板/托盘/字幕窗不在本方案内。
> 参考权威 = 工作区内 `LiveTranslate/subtitle_overlay.py`（1286 行；用户明确：与外部 `D:\biancheng\LiveTranslate` 等仓库无关）。
> 用户反馈两点：① 半透明没实现（整窗全黑不透明）；② UI 观感"非常怪"。
> 证据：实机截图（Rust 实拍、双版头部并排对照）均不入仓库（涉个人隐私，走查后已删除），结论以文字为准。

---

## 0. 结论（TL;DR）

两个根因，都不是机制缺失，是**实现刻度/排版错误**：

1. **不透明 = 一处单位错位**：`overlay.rs` 的 `opa()` 按 0-100 百分比处理透明度（`.min(100)`），而设置模型存的是 Qt 0-255 alpha（`bg_opacity=240`、`header_opacity=230`）→ `min(100)` 全部钳成 100% → 容器与头部完全不透明。字幕窗同一套透明背板机制下工作正常（`with_alpha`/`/255` 用法正确），仅 overlay 用错。
2. **观感怪 = 三件事叠加**：
   a. **消息流排版错**：原文头部与译文塞在同一个 `horizontal_wrapped` 里，译文跟在原文后面同行流排；原版每条消息是 QVBoxLayout **两行**（头部行 + 译文行，各自独立换行）。
   b. **头部控件全是 egui 默认大尺寸**：原版是 18-20px 高的紧凑 Qt 控件（头部总高 62 逻辑 px），Rust 的按钮/下拉/复选用 egui 默认规格，头部实测约 90+ 逻辑 px，按钮行挤满整行。
   c. **Consolas 从未生效**：`install_cjk_fonts` 把雅黑 `insert(0)` 到 Proportional 和 Monospace 两个族**首位**，拉丁字形全部命中雅黑；原版头部 chrome（标题/按钮/复选/下拉/标签/统计行）全是 Consolas。

---

## 1. 行级证据

| # | 问题 | Rust 位置 | 原版依据（subtitle_overlay.py） |
|---|---|---|---|
| 1 | 透明度单位错位（根因①） | `overlay.rs:29-33`（`opa()` `.min(100)`）、`:61`（bg 传 `st.bg_opacity=240`）、`:88-91`（header 传 230） | `settings.rs:292-294` 注释 `// 0-255`；正确写法见 `panel/style.rs:158-160`、`subtitle.rs:362` |
| 2 | `window_opacity`（95%）未乘入容器/头部填充，只乘了文字 | `overlay.rs:61,88`（填充无 opa_pct） | `apply_style` → `setWindowOpacity(0.95)` 作用于**整窗**（py:1201） |
| 3 | 消息原文+译文同行混排 | `overlay.rs:455-501`（单 `horizontal_wrapped`） | ChatMessage = QVBoxLayout 头部 label + 译文 label，两行独立换行（py:216-239） |
| 4 | 消息头部行未设字号（egui 默认 ~10.5px） | `overlay.rs:459-462`（无 FontId） | 头部 label = YaHei **11pt ≈ 14.7px**（py:221-224）；译文 14pt ≈ 18.7px（py:233-235，Rust 已设 ✓） |
| 5 | Consolas 不生效（字体族首位被雅黑占据） | `app.rs:1283-1287`（`insert(0, "cjk")` 双族） | 标题/按钮/复选/下拉标签/统计行均 `QFont("Consolas",…)`（py:638,648,704,743,405-451） |
| 6 | 头部控件偏大（按钮 min_h20 但 egui 内边距大；ComboBox 默认高；行距 item_spacing 默认 ~8） | `overlay.rs:546-552`、row2 各处 | 按钮 h20/字 11px/padding 0-6（py:357-370,644-652）；下拉 h18（py:752）；DragHandle 总高 **62**，margins(8,2,8,2) spacing 2（py:617-623） |
| 7 | cost 显示在电平行右端 | `overlay.rs:313-324`（注释自认照"新版参照截图"） | cost 在**统计行末尾** `¥x.xxxx` #fa5（py:526-530,541） |
| 8 | 统计行数值颜色用 #cccccc（original_color） | `overlay.rs:384-395` | 统计 label 底色 **#888**，仅 CPU/RAM/GPU/ASR/TL/Tok 标签着色，数值继承 #888（py:451,516-542） |
| 9 | `resize_grip` 双重调用 | `overlay.rs:80` + `:433` | QSizeGrip 只有一个（py:1013-1019） |
| 10 | 电平条定宽 120-180px，行右留白 | `overlay.rs:335` | QProgressBar stretch 填满整行（py:399-447，H14 ✓） |
| 11 | 消息区左右缩进仅 4px（Frame 内边距） | `overlay.rs:73` | ChatMessage 自带 margins(8,4,8,4)；消息间距 spacing 2（py:216-218,1006-1007） |
| 12 | 无保存几何时无默认位置（落在系统默认处） | `state.rs:1286-1289`（None → 不设） | 首启 `x=avail.right-620-20, y=avail.bottom-500-60`（py:939-945） |

规格基准（原版实测值，施工时直接对表）：窗口 620×500、min 480×200；容器 #000000@240 圆角 8；头部 #1a1a2e@230 圆角 4；整窗不透明度 95%。按钮：bg white20/border white40/#aaa/11px/h20/r3；退出红 40/80；暂停琥珀 50 + #ddb；字幕开绿 40/80。复选：Consolas 8、#888、indicator 12、spacing 6；置顶✓ 自动滚动✓ 任务栏✗。下拉 h18、拉伸 3:2:2、`{code} - {label}` 格式。电平条 H14、bg white15/border white30、chunk MIC #c586c0 / RMS #4ec9b0 / VAD #dcdcaa、值 `min(100, rms×500)`。统计行 Consolas 8 #888、CPU/RAM/GPU #6cf、ASR #8b8、TL #db8、Tok #c9c、(↑↓) #666、设备 cuda?#4ec9b0:#dcdcaa。消息：头部 11pt [ts]#888899 [lang]#6cf 原文#cccccc ASR 9pt #8b8；译文 14pt `>` #ffffff TL 9pt #db8；翻译中 #999 斜体；同语言 #aaa 斜体；上限 50 条。

---

## 2. 修复方案（三步，各自独立可验收）

### Step A：透明度贯通（最小改动，先出效果）

1. `opa()` 改为显式双参数（或新增 `fade`）：`alpha_0_255` × `window_pct` 两级乘算，预乘空间合成（现有实现已是 premultiplied，扩即可）：`k = a/255 × w/100`。
2. 容器填充 = `fade(bg_color, bg_opacity, window_opacity)`；头部填充 = `fade(header_color, header_opacity, window_opacity)`；电平条/按钮/文字照旧过 `window_opacity`。
3. 验收：默认样式容器有效 alpha ≈ 89%（240/255×95%）、头部 ≈ 86%；全屏截图压亮色桌面可见微透；样式页 transparent 预设（120/120/70）下明显透视；字幕窗回归不受影响。

### Step B：消息流排版对齐（"怪"的大头）

1. `message_block` 拆两个独立换行块：头部行 `horizontal_wrapped`（[ts] [lang] 原文 ASR ms），译文行另起（`> 译文` TL ms / 翻译中 / 同语言）。
2. 头部行显式 `FontId::proportional(pt(original_font_size))`；ASR/TL 维持 `pt(9)`。
3. 消息块 margins (8,4,8,4)、块间 `add_space(2)`；统计行数值色改 #888、cost 移统计行尾；去双 grip。
4. 验收：喂假消息与原版并排——[ts] 灰、[ja] 青、原文 14.7px 白、译文 18.7px 白且**必另起一行**、TL 橙 12px。

### Step C：头部紧凑化 + 字体归位

1. 字体：注册 `consola.ttf` + `consolab.ttf` 进 **Monospace 族首位**（雅黑退为兜底）；Proportional 族保持雅黑优先 → 标题/按钮/复选/下拉/统计恢复 Consolas 观感。
2. 控件紧凑：行1 `set_min_height` 收紧、按钮水平 padding ≈6px；row2 复选/下拉行 `item_spacing` 3-4、ComboBox 高度压到 ~18-20；头部 Frame 总高逼近 62（full）/24（compact）。
3. 电平条改 stretch 填满整行（去 120-180 定宽）；无保存几何时默认位置右下 `(avail.right-620-20, avail.bottom-500-60)`。
4. 验收：头部高度 62±4 逻辑 px；与原版 42 并排图逐按钮/逐行对位；compact 切换高度动画保持。

---

## 3. 风险与注意

- egui 0.36 `Button` 内边距用 `Button::margin(Margin)`；复选/下拉无法 100% 复刻 Qt 质感（既有决策：近似即可）。
- Consolas bold 需同时注册，否则标题伪加粗；无 Consolas 环境（非常见）回退雅黑不致缺字。
- 走查一律**全屏截图**（egui 窗口 PrintWindow 抓旧帧，见 ui-realign.md §1 教训）。
- Step A 不动布局、Step B 不动控件规格、Step C 不动数据通路——三步可分三个 commit，互不阻塞。

---

## 4. 实机双版同屏对照（2026-09-06 21:33，2560×1600 物理）

两版同机同屏运行（Python 原版经工作区副本 `.venv` 启动；Rust 为 release 重建后启动），全分辨率截图走查后已删除（涉个人隐私不入仓库），实拍结论如下（与 §0/§1 推断一致）：

1. **透明度**：原版截图可透视底下窗口文字（RMS 37% 时仍透），Rust 版为纯黑矩形把底下内容全部遮死——`opa()` 刻度 bug 的直观呈现。
2. **消息两行制**：原版截图中 `[21:33:47] [ko] あ。 ASR 158ms` 与下一行斜体 `翻译中...` 各占一行；同屏期 Rust 未能产出消息（见下），两行/混排对照待 Step B 喂假消息验收补拍。
3. **头部观感**：双版头部并排对照——Rust 按钮/下拉/复选均大一圈、下拉带大箭头按钮、电平条定宽右侧留白、全部字形为雅黑；原版按钮 20px 紧凑、下拉扁平 18px、电平条拉伸占满整行、chrome 全 Consolas。
4. **统计行格式两版一致** ✓（`SenseVoice Small [cuda|cpu] | CPU x% RAM x GPU N/A | ASR n TL n Tok n (p↑c↓)`；Python 跑 CUDA、Rust 纯 CPU 符合硬约束，设备色差 #4ec9b0/#dcdcaa 两版均正确）。

附带观察（非 GUI，建议单独验证）：Rust 管线在"禁用/系统默认/麦克风(2- NICEHCK)"三种设备选择 + 暂停/运行循环后 RMS 仍恒 0%，而 Python 同期从同一流持续转写正常——疑似回环采集语义（系统默认应落扬声器 loopback）或设备热切换未生效；本轮测试后 `settings.json` 的 `mic_device` 停在 `__default__`（初始为"禁用"），如需改回在识别页设备下拉两击即可。
