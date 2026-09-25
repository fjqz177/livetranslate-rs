# 窗口闪现整治——启动只出主界面 / 运行期零弹窗 / 退出即走无白帧

> **状态**：完工已归档（2026-09-25 定稿并施工，当日用户实机验收三条线通过；D-100）
> **日期**：2026-09-25（取证与定稿同日）
> **触发**：用户报三症状——启动弹模型加载窗（含 0.7s 白屏）、运行期反复弹同类窗（切模型等）、退出闪"主界面突然变成丑陋 Windows 窗"；当日 30fps 录屏 + Win32 取证三根因实锤，用户拍板三条验收线
> **前缀**：WFC-（本文档局部号）

## 一、背景取证

取证方法：ffmpeg gdigrab 30fps 全桌面录屏逐帧 + Win32 EnumWindows/GetWindowRect 实测（本机，release exe）。

| # | 症状 | 实测 | 根因 |
|---|---|---|---|
| 1 | 启动白屏 ~0.7s | 白窗 6.6s 出现、7.4s 才有内容 | `create_window` 按应显表把悬浮窗创建成可见（app.rs:231），但第一帧 ~0.7s 后才画（串行建 6 窗 + 托盘期间事件循环未派发）；白底 = winit 窗口类背景刷子。加载窗（WinId::Setup）另弹在左上级联位（`needs_position` 仅 Panel，app.rs:322） |
| 2 | 运行期反复弹加载窗 | 一次 qwen3 加载占屏 ~3s | `ModelLoadStart`（管道启动/切引擎/待命唤醒/修复重载都发，pipeline.rs:2240/2469）→ UI 侧显示 Setup 窗（app.rs:1544-1549）并 `focus_window` 抢焦点（app.rs:680） |
| 3 | 退出闪"丑陋 Windows 窗" ~0.25s | 白窗带原生标题栏 + 应用图标，出现在悬浮窗原位，随后消失 | 退出收尾期 wgpu surface 先于窗口销毁（字段序 painter 先于 windows drop），失内容窗口被类刷子刷白；winit 0.30 无边框窗靠 NCCALCSIZE 抑制的 WS_CAPTION（实测 style=0x16CF0000 恒置位）在非客户区重画时弹回 |

取证附带事实：**`ModelLoadDone` 全仓无生产者**（grep 仅 events.rs 定义 + app.rs 消费 + setup.rs 注释），加载结束实际由 `AsrDevice`/`AsrUnavailable` 驱动；确认窗（Confirm）出现无白闪（清屏色黑，过渡 0.4s）；CUA 无法捕获 LAYERED 分层窗；DPI 虚拟化截图坐标与物理坐标非线性，自动化点击必须 DPI-aware 进程取物理坐标。

## 二、方案裁决（D-100）

用户拍板三条验收线（2026-09-25）：

1. **启动**：只出主界面（悬浮窗 + 按设置的字幕窗），第一帧即完整内容，无白窗无加载窗。
2. **运行全程**：切模型/换设备/坏文件重载/断线恢复不再弹任何独立窗口，加载状态并入悬浮窗状态行；用户主动操作（面板/基准/确认框/导出对话框等）不受影响。
3. **退出**：点确认后所有窗口 1~2 帧内整体消失，后台安静收尾；确认到进程结束屏幕零白帧零怪窗。

三项技术裁决：

- **WFC-A 先画后显**：所有窗口隐藏创建，`setup()` 尾对"应显表"为 true 的窗口同步画首帧后揭示（F1）。
- **WFC-B 加载态内联**：`ModelLoadStart` 只写悬浮窗状态行，独立加载窗与 `load_dialog` 状态整体退役（F2，行为差异——对齐 2026-09-01 less-is-more 启动哲学，模态加载框是复刻期遗产）。
- **WFC-C 先离屏后收尾**：退出分支在 `event_loop.exit()` 前全窗隐藏（F3）。

**边界**：退出确认窗保留（D-87 裁决）；`lt-proto` 契约零改动（事件保留、只改消费），PROTO_VERSION 不增；首启向导/缺模型下载流（D-78 保留面）代码不动。

## 三、施工卡

### WFC-1 启动白屏（crates/lt-ui/src/app.rs）

1. `create_window`（:231 附近）：`with_visible(visible)` → 恒 `with_visible(false)`；局部 `visible`（真值表值）保留供尾部 `if visible { request_redraw() }`（:329）判定，注释说明解耦。
2. `setup()` 尾部（托盘/Z 序/监视节拍之后）加揭示段：收集真值表应显窗口 → 逐个 `run_frame(id)` 同步画首帧 → `apply_needs_position(id)`（Panel 居中，`LIVETRANSLATE_SHOW_PANEL=1` 路径依赖）→ 裸 `window.set_visible(true)` 揭示。
3. `set_visible` 内联的 `needs_position` 定位块（:664-678）抽成 `apply_needs_position(id)` 供两处复用。
4. 揭示用裸 `set_visible(true)` 而非 `self.set_visible(id, true)`：后者 `focus_window()` 抢焦点——悬浮窗 NOACTIVATE，今天启动从不抢焦点，保持。

依据：白屏 = 可见在先绘制在后的时序问题；`run_frame`（:337）就是事件循环首绘同一函数，提前调用零新状态；surface 在 `set_window`（:321）创建时按**几何恢复之后**的 `inner_size` 配置（egui-wgpu set_window 读当时尺寸，Windows 上 request_inner_size 同步生效），首帧无错尺寸；同步这一帧同时落 LWA_ALPHA（创建期已挂）与 SetWindowRgn（run_frame 内 region_key 维护，:437）；帧尾 `process_actions`（:443）启动期无动作可消费。

风险与备选：隐藏态定位被级联覆盖（W5 旧坑）→ 走查核对落点，复现则揭示后一帧内重申几何；若"隐藏时 present 的内容显示后不保持"（理论不应发生）→ B 计划 = `pending_show` 标志在 `run_frame` 真实首帧后揭示。

### WFC-2 加载弹窗退役（app.rs + state.rs + setup.rs + 两份 i18n yaml）

1. app.rs `ModelLoadStart` 分支（:1544）改为只写悬浮窗状态行 + `redraw(WinId::Overlay)`：
   `self.app_state.overlay.asr_label = Some(lt_i18n::t("loading_model_short").replace("{name}", &label));`
2. `ModelLoadDone` 臂保留空臂 `=> {}`（`on_event` 穷尽匹配无兜底臂，app.rs:1575 收尾；事件无生产者，注释说明）。
3. 删 `close_load_dialog`（:875-883）及三处调用（`AsrDevice`/`AsrUnavailable`/`ModelLoadDone` 分支）——终态覆写由既有臂完成（成功 → `AsrDevice` 写真实标签；失败 → `AsrUnavailable` 写"ASR 不可用"；两事件恒后于 `ModelLoadStart`，pipeline.rs:2468-2492 顺序）。悬浮窗渲染位现成（overlay.rs:584），**overlay.rs 零改动**；托盘状态行同源读 `asr_label` 自动跟随。
4. 删 `state.rs` 的 `load_dialog` 字段（:2112/:2212）、`needs_setup_tick` 传参（:2621 + setup.rs:53 签名）。
5. 删 `setup.rs` 的 `load_dialog_ui`（:214-224）、`setup_ui` 加载框分派分支（:36-39）；模块头注"三合一"改"二合一"并记 D-100；windows/mod.rs Setup 分派注释同步。
6. `finish_startup`（:1040-1048）"加载框已开则保留窗口"判断简化为无条件隐藏 Setup 窗。
7. i18n：删 `loading_model`（zh.yaml:294 / en.yaml:292，唯一消费者即被删的 load_dialog_ui——消融：无消费者即删）；加 `loading_model_short`：zh `"正在加载 {name}…"` / en `"Loading {name}…"`，两份同步（键集一致性测试兜底）。

依据：悬浮窗状态行本就渲染 `asr_label`，零新增 UI 结构；**"杜绝"完备性 = 全量枚举运行期 `set_visible(true)` 调用点**（app.rs :518 确认窗 / :614 面板 / :835 悬浮窗 / :1105 二次启动 / :1699+ WinAction 组 / :1716 基准 / :1720 字幕——全为用户主动动作；:1046 finish_startup 仅向导保留面）——移除 :1546 后不变量成立：**运行期弹窗 ⟺ 用户主动操作**。lt-proto 事件全保留，契约冻结零触发。

### WFC-3 退出白帧（app.rs）

`about_to_wait` 退出分支（:2567）在 `event_loop.exit()` 前全窗隐藏：

```rust
let ids: Vec<WinId> = self.windows.iter().map(|w| w.id).collect();
for id in ids {
    self.set_visible(id, false);
}
event_loop.exit();
```

依据：白帧本质 = "没内容的窗口还挂在屏幕上"；隐藏立即离屏，与销毁时序彻底解耦。**不调字段 drop 序**——`painter.set_window`（:321）持 `Arc<Window>`，窗口真身销毁时机由 painter 侧引用计数决定，重排无效。复用 `set_visible(id, false)` 白拿拖动中断清理与真值表同步。退出路径完备：`quit_requested` 是唯一 `event_loop.exit()` 入口（确认 Quit 与缺模型下载失败两条来路汇于此）。管道收尾序（INV4 / ACR-1a/1b）一行不动。

风险（如实）：收尾预算 `EXIT_GRACE = 15s`（pipeline.rs:56）×ASR 尾巴与翻译收敛两段，病态（断网+翻译挂起）进程可滞留 ~30s——今天看得到冻结窗口，改后是"窗口没了托盘还在"；正常 0.57s 无感。滞留期内二次双击入口退化为"仅退出"（singleton.rs:17 旧语义），秒级可接受。

## 四、验收基线

1. **录屏三段**（gdigrab 30fps 逐帧）：启动 0 白帧、第一眼即完整悬浮窗、落点与保存几何一致、无多余窗口；切模型 0 新窗 0 焦点变化、状态行"正在加载…"→终态标签、失败路径落"ASR 不可用"；退出确认后 ≤2 帧全窗消失、至进程结束 0 白帧 0 标题栏弹回。
2. `cargo test --workspace` 全绿 + `precommit.ps1` 三项过（fmt → 总纲健康 → clippy）。
3. 实机走查附加：拖动字幕窗/悬浮窗中途退出无残留输入捕获；`LIVETRANSLATE_SHOW_PANEL=1` 启动 Panel 定位正确；断网+翻译进行中退出滞留可接受。

回归防护（诚实版）：宿主侧处理（`on_event`）依赖 winit 事件循环无法 headless 单测——既有盲区结构，不虚构测试；机械防护 = i18n 键集一致性测试（既有）+ 验收走查。

**验收结果（2026-09-25，当日闭环）**：三条线用户实机验收通过——启动只出主界面且首帧完整、无加载窗；切模型零弹窗零抢焦点、状态行"正在加载…"→终态标签收敛正常；退出无白帧无怪窗。开发者冒烟另双确认启动线（release exe：悬浮窗揭示即完整内容、AsrDevice 覆写状态行正常）。说明：30fps 逐帧录屏复核未单独执行（用户目验通过替代）；面板/确认窗首显白帧存疑项转遗留（见 §五）。

## 五、遗留走查（收口后清账）

- 三段录屏归档（启动/切模型/退出）随收口。
- 面板/确认窗/基准窗**首次显示**是否存在 ≤2 帧白帧：取证采样间隔 200ms 未证实，存疑；若走查扎眼，同机制（先画后显）扩展到 `set_visible` 显示路径（pending_show 统一机制），另立小包。
- 收口动作：gotchas 候选入册（「winit 0.30 无边框窗 WS_CAPTION 恒置位 + surface 先亡窗口后亡 = 退出白帧标题栏弹回；对策 = 退出先全窗离屏」）；decisions.md 落档路径核对；archive 归档 + archive-index 加行 + README 移行 + board 挪卡（同一提交）。

## 附：评审记录（v1.0 草稿 → 定稿，八原则自查 2026-09-25）

| # | 发现 | 等级 | 处置 |
|---|---|---|---|
| 1 | v1.0 揭示段裸 `set_visible` 绕过 Panel `needs_position`，SHOW_PANEL 路径面板弹级联位 | P1 | 揭示段补居中（施工卡 WFC-1.2） |
| 2 | v1.0 称 ModelLoadDone 臂可删——`on_event` 穷尽匹配无兜底臂，删则编译失败 | P1 | 保留空臂 + 注释（WFC-2.2） |
| 3 | v1.0 闭环论证引了不存在的生产者（ModelLoadDone 全仓无生产者） | P1 | 闭环改引 AsrDevice/AsrUnavailable；事实入档（§一/WFC-2.2） |
| 4 | v1.0 备选"调字段 drop 序"无效：surface 持 `Arc<Window>`（app.rs:321），重排动不了销毁时机 | P2 | 删备选，先离屏为唯一修法（WFC-3） |
| 5 | "杜绝弹窗"缺完备性论证；`loading_model` 键删 UI 后成孤儿 | P2 | 补全量枚举；键随消费者一并删（WFC-2.7） |
| 6 | 退出滞留未量化（EXIT_GRACE=15s×2）+ 单实例退化未提；面板首显白帧存疑未声明 | P3 | 风险量化入档；首显白帧列遗留走查 |
