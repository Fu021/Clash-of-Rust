# 历史证据范围

[返回索引](README.md)。以下统计是扫描当时的快照，不随最近任务自动累加；具体公开证据见各主题页。

- 扫描日期：2026-10-24。
- 提交范围：`git log --all`，包括已合入 main、开发分支、修复提交和已移除的临时验证分支。
- Actions 范围：GitHub Actions 中状态为 `failure` 的 62 次运行、146 个含失败步骤的 job 记录；读取失败 job 日志、失败步骤和公开运行信息。
- 失败运行分布：

| 工作流 | 失败运行 | 含失败步骤的 job |
| --- | ---: | ---: |
| CI | 26 | 86 |
| Publish Release | 13 | 21 |
| Memory Benchmark | 11 | 11 |
| Claude Protection Validation（临时） | 8 | 24 |
| macOS Fix Validation（临时） | 1 | 2 |
| Rust（旧工作流） | 1 | 1 |
| UI Screenshots（已移除） | 1 | 1 |
| release.yml（工作流级失败） | 1 | 0 |

146 条 job 记录中有 144 个结论为 `failure`，另有 2 个 CI job 在 `Post Restore private native Rust cache` 步骤失败后被取消；表中按“含失败步骤”合并统计。失败步骤以桌面“Compile, test and export native binaries”最多（61 个），其次是临时 Claude 原生验证（19 个）、安装包验证（15 个）、Android 编译（14 个）和 Android 真实代理/TUN（7 个）。一个运行常因同一根因让多个矩阵 job 同时失败，不能把 job 数量当作独立事故数量。
