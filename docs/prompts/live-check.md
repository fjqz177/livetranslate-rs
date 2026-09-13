# 实机走查卡——GUI / 实机验证方法

1. 冒烟环境：临时 `LIVETRANSLATE_CONFIG_DIR` + 其 settings.json 显式 `models_dir` 指真实模型缓存，
   否则引擎探测全部失败；`LIVETRANSLATE_SHOW_PANEL=1` 启动直开控制面板（走查旗标）；
   `livetranslate.exe --version` = 产物能起的最小证明（无 GUI 打印版本即退）。
2. **一律不用 CUA**（用户 2026-09-08 明示，两次强调）。
3. GUI 取证主干 = headless 探针法：run_ui + RawInput 模拟指针 + 扫 Shape::Text（位移/状态断言）。
4. 需要视觉证据 → 请用户截图；自己验收用全屏截图（PrintWindow 会抓旧帧，G-10）。
5. 已知视觉陷阱：字幕窗 179 灰 = 正常 alpha 合成（G-8）；参照图重拍相关怪癖 = G-18。
6. 走查项登记：AGENTS §8 遗留区一行 + 对应归档文档章节；通过即清零删行。
