# 施工卡——施工期间滚动自查

1. **分层**：每写一个跨 crate use，对照 AGENTS §3 白名单（check_deps.ps1 会拦，别攒到提交才红）。
2. **质量**：行为改动带测试；写测试守离线纪律（禁真模型/真网络/真设备，探针 #[ignore]）+ 临时目录
   唯一化（G-22）；fmt + clippy -D warnings 滚动跑，别攒到最后。
3. **提交**：里程碑 + 全绿即自主中文 commit（`feat(scope): 中文主题`）；多代理并行 → 提交前
   `git status` / `git diff` 逐项复核，显式 pathspec，禁 `git add -A`（G-19）。
4. **用户文案**：orchestrator 域经 `Msg` 注入；只有 lt-ui 碰 i18n yaml（zh/en 必须同步改）。
5. **踩新坑**：当场记 docs/gotchas.md 候选条目 + AGENTS §7 速查行，别只留在会话里。
6. **中途新裁决**：当场登记 decisions.md，别攒到收口。
