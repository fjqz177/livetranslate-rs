# 收口卡——完工归档一气呵成（纪律真源 = docs/README.md 纪律⑤；本卡 = 可执行化）

按序、同一个提交内完成 1~7：

1. 工作包文档标「完工」（不用 ✗）。
2. `git mv docs/<包>.md docs/archive/`（未跟踪草稿不能 git mv——先定稿入库再归档）。
3. `docs/archive-index.md`：归档表加行（一行一档）+ `docs/README.md` 活跃表移行。
4. decisions.md：落档路径更新为 archive 路径；施工中欠登的裁决补行。
5. 全仓旧路径替换：grep 复盘（含 .rs 注释、AGENTS.md、docs 互引）。
6. 看板该包收敛（`docs/board.md`）：无遗留整段删；有遗留压一行进遗留区；清零即删（AGENTS §8 只留指针，无需改文）。
7. 收口三问：`ls docs/` 健康吗？decisions.md 漏行吗？表和文件对上没？（表↔目录一致已由守护断言机械化，本问聚焦 decisions 漏行）

然后：
8. 门禁：precommit.ps1 全过 + `cargo test --workspace` 全绿（收工门禁）。
9. 提交：显式 pathspec（禁 add -A）；副产物（docs/architecture/、ui-audit/）不入库。

→ 提交后：handoff.md（会话收尾）
