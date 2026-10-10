# GitHub Actions 工作流

| 工作流 | 触发 | 职责 | 产物 |
| --- | --- | --- | --- |
| Rust / CI | main push、PR | 四平台编译、静态检查、代码与原生接口测试 | `build-{system}-{arch}`，保留 14 天 |
| Publish Release | 手动 | 复用成功 CI 的程序，打包、安装后测试、发布 | 四个安装包及 SHA256；测试产物保留 14 天 |
| Memory Benchmark | 手动 | 测量一个远程分支、标签或提交的内存 | 原始 JSONL、报告、截图、构建日志；保留 30 天 |
| Windows version regression investigation | 手动 | 使用正式安装包对照 0.4.12、0.4.13、0.5.0 的普通权限启动和升级 | 逐场景退出码及调查报告；保留 7 天 |

四个平台为 Windows x64 / ARM64、Linux x64 / ARM64。Ubuntu 最新版兼容验证在发布工作流中执行。

## CI

- 使用同一个 Release profile 执行 Clippy、构建、全部 target 的测试，复用依赖编译缓存。
- 生成代码检查集中在 Linux x64；通用 Python 构建工具和产物协议测试在 Linux x64 与两种 Windows 架构执行，Windows 同时检查原生 PE 版本。
- 四个平台运行真实 mihomo 集成测试。Linux 原生 GSettings、KDE、托盘测试和 Windows 原生托盘、自启动、图标、更新辅助程序测试保留在 CI。
- 已构建的测试程序直接执行，避免每次过滤测试都调用 Cargo。
- main 推送和 PR 更新触发 CI，避免同一 feature 分支的 push 与 PR 同时执行两套矩阵；分支开发通过 PR 验证。同分支的过时 CI 会取消。资源按平台与日期缓存，使用前仍校验官方哈希；应用和资源打包时使用白名单。
- CI 不生成安装包。编译产物使用 tar 保存执行权限，附带提交 SHA、版本、平台、Cargo.lock 校验值、工具链和逐文件校验清单。

Windows 的逐项覆盖、权限环境和发布包测试范围见 [Windows CI 测试清单](windows-ci-checklist.md)。原生测试按 8 组记录耗时与结果，失败时仍保留 `windows-ci-report-{arch}` 产物。

## Publish Release

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的 main CI 运行编号；留空选择当前提交的 CI。开发分支测试包必须填写该分支当前提交的成功 PR CI 编号。
- `check_only`：勾选后仍完成全部打包和安装测试，最终不创建或修改 Release。可在开发分支执行，无需发布说明；安装包保存在本次运行的 `package-{system}-{arch}` 产物中。

发布流程固定到所选 CI 的提交，不在打包阶段重新编译，也不重新下载资源。构建产物过期时需要重新运行 CI。

1. 正式发布检查当前 main 与成功的 main push CI；开发分支仅测试时检查同仓库 PR CI 与当前分支提交一致。两种模式都要求四平台检查成功、四份构建产物完整。
2. 四个平台并行获取、校验并打包已有程序。
3. Windows 测试最终安装包的安装、同版本更新、卸载及用户文件保留；继续执行隔离的失败恢复、退出、重装和旧版迁移测试；扫描 Defender。
4. Linux 验证 DEB 内容、安装、GUI、后台启动、TUN 授权、DNS、路由与清理。
5. 最新版 Ubuntu 使用官方 HTTPS APT 镜像和有界下载等待，避免 Azure 镜像挂起；安装同一份 Ubuntu 22.04 构建的 DEB，再验证 GUI 和 TUN；不重复编译或打包。
6. 最终步骤核对所有测试 job、安装包校验值、测试成功记录与来源 CI；八个附件上传并核对后才公开 Release。开发版本标记为 prerelease，不覆盖稳定版 latest。

只有最终发布 job 有 `contents: write`；前面的编译和打包测试使用读取权限。

本地打包命令 `python scripts/build-installer.py` 仍支持编译并打包。工作流使用 `--prebuilt` 入口，拒绝不匹配的提交、版本、平台或文件校验值。

## Memory Benchmark

在 Actions 中选择 **Memory Benchmark → Run workflow**：

- `ref`：需要测量的远程分支、标签或提交 SHA（支持短 SHA）；留空测所选工作流分支。
- `profile`：`release` 或 `dev`，默认 release。
- `rows`：每项 API 测试的条数，范围 1–50,000，默认 50,000。
- `repetitions`：每项 API 测试的独立进程次数，范围 1–10，默认 3。
- `sample_seconds`：每个 GUI 场景的采样时间，范围 1–120 秒，默认 10 秒。
- `preview`：额外生成实际界面截图，使用隔离的固定演示数据；dev.2 及之后支持，默认关闭。截图测试入口通过可选 feature 编译，不进入安装包。

一次运行只测一个代码版本，不自动选择或比较另一个版本。比较时分别运行两次，并保持工具链、profile、输入和环境一致。

统一 Rust 基准程序运行规则、连接、代理三类数据的流式/缓冲解析、1/5 次刷新，每项独立进程重复采样。GUI 使用隔离数据目录和 Xvfb，依次测首页、折叠代理组、搜索 `node-`、返回首页；固定输入为 2,000 节点、40 组各含全部节点、20,000 条规则，关闭系统代理与定时测速，不生成业务流量。

为了让较旧的提交也能接受相同测试，工作流仅替换所选代码的 example 基准程序，应用源码保持所选提交内容；记录应用提交和基准程序提交。该入口面向与 0.4.12 API 兼容的代码。

产物包含 `metadata.json`、资源清单、完整输入订阅、API 数据、四个 GUI 场景的采样和截图、README 报告及构建日志。API 数值为 Rust 堆分配量；GUI/内核分别统计 RSS、PSS、USS，不能混用两类口径。采样峰值为观测峰值，内核内存波动不能直接归因于 Rust 改动。
