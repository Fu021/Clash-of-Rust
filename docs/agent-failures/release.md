# 打包、发布与工作流生命周期

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 10. 发布产物、版本与依赖快照

**典型症状**

- Publish Release 发现上游 CI 未通过，或 artifact 下载 DNS/网络失败。
- manifest 缺少 native payload、commit/arch/schema 不符、Android 依赖快照与 CI 不一致。
- 版本、安装包名称、更新通道或安装时间戳判断错误。

**代表证据**

- Publish Release [39784808500](https://github.com/Fu021/Clash-of-Rust/actions/runs/39784808500)：macOS 获取 artifact 时 DNS 失败。
- Android 打包 [38989293563](https://github.com/Fu021/Clash-of-Rust/actions/runs/38989293563)：依赖快照与已验证 CI 不一致。
- 修复提交 `8b7ff41`、`9d705d5`、`1aaa1da`、`465fec6`。

**首先检查**

- `ci_run_id` 是否属于同一提交、同一轮依赖快照和成功的必需 job。
- 从前置解析到最终核验完成，来源分支是否始终指向该 CI 提交；即使是文档提交也应等打包结束后推送，避免分支头变化使正确产物被拒绝。
- artifact 的 schema、commit、version、system、arch、profile、逐文件校验值和测试程序是否完整。
- 发布的安装包是否由已验证产物组装，而不是重新编译。

**防回归**

- 开发分支测试包保持 `check_only`；未获授权不创建正式 Release。
- 瞬时下载失败只做有界重试，不能绕过校验或改用手工未验证文件。
- 版本号和发布记录只在发布任务中修改，并同步 README、安装包名和更新测试。

## 12. 手动运行参数与工作流生命周期

**典型症状**

- 手动 benchmark 的 `ref` 误填短 SHA，checkout 把 `6473bbe` 当作 ref 模式拉取并失败。
- 已删除的临时工作流仍被手动触发，或依赖已移除的测试钩子。
- 大缓存恢复阻塞 fan-out，或旧 artifact 在新 schema 下继续使用。

**代表证据**

- Memory Benchmark [37971443677](https://github.com/Fu021/Clash-of-Rust/actions/runs/37971443677)：checkout `6473bbe` 的 fetch 失败。
- UI Screenshots [38595379460](https://github.com/Fu021/Clash-of-Rust/actions/runs/38595379460) 和旧临时验证工作流均已移除。
- 修复提交 `0a29d3b`、`c177290`、`e660a95`、`7034acf`、`b07d078`、`5b70648`。

**首先检查**

- 手动运行参数是否是完整 SHA/分支/标签；短 SHA 只能作为展示，不应作为 checkout ref。
- 工作流文件、artifact schema、脚本入口和文档是否同时删除或更新。
- 被跳过的临时工作流是否仍保留手动触发入口；弃用后应删除文件并由维护者在 GitHub 禁用。

**防回归**

- 每轮只保留 CI、Publish Release、Memory Benchmark 三个长期工作流。
- 手动 workflow 输入必须有校验和明确错误信息。
- 缓存恢复不能让整个平台矩阵无限等待；必须有超时、部分失败和正常下载回退。
