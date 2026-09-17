# 悬浮窗隐藏→托盘重显示后失去半透明（纯黑）——winit 重写 EXSTYLE 清 LAYERED（D-34）
> 归档注记（2026-09-17）：本文已完工归档，头注状态行为归档前原貌，以本注记为准。

> 用户反馈（2026-09-08，D-33 改造后体验）：悬浮窗点「隐藏」后，从托盘菜单
> 「显示悬浮窗」重新打开，悬浮窗的半透明没了，变成纯黑。
> 结论：**既有潜伏 bug**——winit 在 set_visible/flags 翻转时整体重写窗口
> EXSTYLE，清掉本项目手工挂的 `WS_EX_LAYERED`；先前从未隐藏再显示过故未暴露。

## 1. 根因链（源码 + 实机探针双重实证）

1. 悬浮窗半透明实现 = `apply_layered`（`crates/lt-ui/src/app.rs`）：
   `SetWindowLongPtrW(GWL_EXSTYLE, style | WS_EX_LAYERED)` +
   `SetLayeredWindowAttributes(alpha, LWA_ALPHA)`——**手工挂的层属性，
   winit 内部状态不知道**。
2. winit 0.30.13 `set_visible` → `WindowState::set_window_flags` →
   `WindowFlags::apply_diff`（`winit-0.30.13/src/platform_impl/windows/
   window_state.rs:395-410`）：**diff 非空时用自身 flags 表整体重写**
   `SetWindowLongW(GWL_EXSTYLE, style_ex as i32)`（VISIBLE 位翻转必然 diff 非空）
   ——EXSTYLE 由 winit 按自己的 window_flags 重建，**不含 WS_EX_LAYERED** →
   层位被清、`SetLayeredWindowAttributes` 的整窗 alpha 随之失效。
3. 原有 alpha 刷新（run_frame 尾部）用**缓存比对**：`layer_alpha == Some(ta)`
   相等即跳过 `apply_layered`——缓存盲于外部清位，LAYERED 丢失后**永远不复位**。
4. 实机探针（临时 `win_layer_spike`，与生产同构：无边框+置顶+skip_taskbar+
   active(false) 创建 → 挂 LAYERED → set_visible(false/true)）：

   ```
   created: LAYERED=true → hide: LAYERED=false → show: LAYERED=false
   → 缺位检测+重挂: LAYERED=true（自愈逻辑验证通过）
   ```

   与 apply_diff 源码逐行吻合。**为什么此前未暴露**：启动路径（创建可见+
   apply_layered 后）没有任何 flags diff（置顶/任务栏初始化与创建属性一致），
   所以半透明一直正常；隐藏/重显示是第一个触发 VISIBLE 翻转的 diff。

## 2. 修复（D-34）

- run_frame 尾部 alpha 段：`layer_alpha != Some(ta) || !window_has_layered(&window)`
  → 任一成立即重挂 `apply_layered`——**每帧实测 EXSTYLE**（GetWindowLongW 用户态
  调用，成本可忽略），缺位即自愈，覆盖全部清位路径（隐藏/显示/改置顶/改任务栏/
  SWP_FRAMECHANGED）。
- 新增 `window_has_layered`（app.rs）：读回 EXSTYLE 检查 LAYERED 位。
- 字幕窗同为 LAYERED 窗口，同一机制一并自愈（用户未报，预防性覆盖）。

## 3. 偏差登记

- **D-34**：悬浮窗/字幕窗层属性改为「每帧实测+缺位重挂」自愈式维持
  （winit apply_diff 会整体重写 EXSTYLE 清掉手工 LAYERED 位；原缓存比对
  无法感知外部清位）。行为结果不变（半透明正常），实现从「一次性设置+缓存」
  变为「持续保证」。后续新偏差自 D-35 起。

## 4. 验收

- [x] 探针实锤：hide 后 LAYERED true→false；show 后同样 false；重挂后 true；
- [x] `cargo test --workspace` 全绿（386 测不回退）、新码 clippy 零告警；
- [x] release 构建通过；
- [ ] 实机：隐藏→托盘「显示悬浮窗」→ 悬浮窗保持半透明（用户确认）。
