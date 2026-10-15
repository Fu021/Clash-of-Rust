# GitHub Actions 工作流

| 工作流 | 触发 | 职责 | 产物 |
| --- | --- | --- | --- |
| Rust / CI | main push、PR | 六平台编译、静态检查、代码与原生接口测试 | `build-{system}-{arch}`，保留 14 天 |
| Publish Release | 手动 | 复用成功 CI 的程序，打包、安装后测试、发布 | 六个安装包及 SHA256；测试产物保留 14 天 |
| Memory Benchmark | 手动 | 测量一个远程分支、标签或提交的内存 | 原始 JSONL、报告、截图、构建日志；保留 30 天 |
| UI Screenshots | 手动 | 选择成功 CI 与平台，生成原生界面截图 | `ui-screenshots-{system}-{arch}`；保留 14 天 |

六个平台为 Windows x64 / ARM64、Linux x64 / ARM64、macOS Intel / Apple Silicon。Linux 安装包支持 Debian/Ubuntu，Ubuntu 仅支持 22.04 及以上；Ubuntu 最新版兼容验证在发布工作流中执行。

## CI

- 六平台使用同一个 Release profile 完成整个 workspace 的格式检查、Clippy、编译和 Rust 常规测试，分别保存共享业务库与桌面库的测试程序。移动端占位目录不参与构建。
- 原生基础检查验证系统代理设置写入、HTTP 转发、TUN 接管无显式代理的流量，以及关闭后的清理。Linux 使用隔离网络命名空间及 GSettings 内存后端，不覆盖真实登录桌面的 dconf 持久化；Mac 使用真实 launchd 网络助手。Windows 只用管理员权限注册服务，实际代理/TUN 检查以中等完整性、非管理员令牌执行，并验证保存 TUN 后重新初始化、服务停止后普通用户重新启动、GUI 崩溃后的内核和端口清理，以及特权助手文件的写入限制。
- CI 聚焦基础代理/TUN 与常规代码测试；已结束的专项排障脚本和临时更新工作流已移除。
- CI 不执行截图，也不额外编译截图入口；截图由独立手动工作流按需生成。
- Linux x64 检查生成代码；通用 Python 构建工具和产物协议测试在 Linux x64 与两种 Windows 架构执行；Windows 另核对 PE 图标。
- main push、PR 更新触发 CI，过时的同分支运行自动取消。现有依赖与资源缓存策略保持不变。
- CI 不生成安装包。构建产物附带提交 SHA、版本、平台、Cargo.lock 校验值、工具链和逐文件 SHA256；发布继续严格验证来源。

Mac 的失败日志、系统与网络状态、崩溃报告保存在 `macos-ci-report-{arch}`；Windows 基础检查耗时保存在 `windows-ci-report-{arch}`。人工授权窗口、真实重启登录和物理显示器操作需要人工验收。

## Publish Release

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的 main CI 运行编号；留空选择当前提交的 CI。开发分支测试包必须填写该分支当前提交的成功 PR CI 编号。
- `check_only`：勾选后完成打包与基础安装/代理测试，最终不创建或修改 Release。可在开发分支执行，无需发布说明；安装包保存在本次运行的 `package-{system}-{arch}` 产物中。

发布流程固定到所选 CI 的提交，不在打包阶段重新编译，也不重新下载资源。构建产物过期时需要重新运行 CI。

1. 正式发布检查当前 main 与成功的 main push CI；开发分支仅测试时检查同仓库 PR CI 与当前分支提交一致。两种模式都要求六平台检查成功、六份构建产物完整。
2. 六个平台并行获取、校验并打包已有程序。
3. Windows 验证最终包安装、资源、网络服务注册、普通用户系统代理/TUN、客户端更新所用的 `/UPDATE` 安装路径、服务更新与卸载，以及用户文件保留；更新病毒库并扫描 Defender。更新、扫描或校验失败阻止上传。
4. Linux 安装 DEB，验证普通用户的系统代理、HTTP 转发、TUN 接管与关闭清理。
5. macOS 验证 `.app` / DMG 架构、版本、资源和签名，启动安装后的 GUI/内核，再验证系统代理和 TUN。当前为 ad-hoc 签名，未配置 Developer ID 与 Apple 公证。
6. 最新版 Ubuntu 安装同一份 Ubuntu 22.04 DEB 并运行基础代理/TUN 检查，不重复编译或打包。
7. 最终步骤核对所有测试 job、安装包校验值、测试成功记录与来源 CI；十二个附件上传并核对后才公开 Release。开发版本标记为 prerelease，不覆盖稳定版 latest。

只有最终发布 job 有 `contents: write`；前面的编译和打包测试使用读取权限。

本地打包命令 `python scripts/build-installer.py` 仍支持编译并打包。工作流使用 `--prebuilt` 入口，拒绝不匹配的提交、版本、平台或文件校验值。

## UI Screenshots

独立手动工作流接受 `ci_run_id`（留空选最新成功 main CI），`platform`（全部/Windows/Linux/macOS），`architecture`（全部/x64/ARM64），`sizes`（普通/最小/两者）。可以选择仍有完整产物的历史 CI，不要求分支仍停在原提交。

先确认六平台 CI 成功、源码来自本仓库、构建产物未过期，再固定到该提交并核对产物及依赖锁的哈希。正式程序不带固定演示场景入口，因此截图任务另编译隔离的截图程序：沿用所选提交的页面和演示场景，仅附加像素捕获适配器，不改变应用视图或 CI 二进制。额外编译和截图均不进入常规 CI。

六种原生 runner 使用 Iced 的渲染像素截图，Mac 不依赖系统屏幕录制权限。输出每页深浅色、普通/最小窗口及详情/确认框 PNG、索引、源码与 CI 产物身份、逐图哈希和日志。固定演示数据不包含账户或实际业务流量，不等同于真实网络检测结果。支持包含 `ui-preview` 场景的项目版本。

## 静态资源维护

图标和国旗使用统一入口，日常构建直接使用仓库中的成品：

```text
python scripts/prepare-assets.py icons
python scripts/prepare-assets.py flags
python scripts/prepare-assets.py verify-icons 路径/clash-of-rust.exe 路径/安装包.exe
```

`icons` 与 `flags` 仅在更新资源时执行，需要 Pillow；`flags` 可加 `--proxy`。图标源图保存在 `resources/icon-concepts/`，国旗与地区名称版本及许可见 [来源说明](../resources/flags/SOURCE.md)。`verify-icons` 只使用 Python 标准库，CI 和 Windows 打包继续执行此检查。

## Memory Benchmark

在 Actions 中选择 **Memory Benchmark → Run workflow**：

- `ref`：需要测量的远程分支、标签或提交 SHA（支持短 SHA）；留空测所选工作流分支。
- `profile`：`release` 或 `dev`，默认 release。
- `workload`：默认 `light`（低强度）；`stress` 保留原压力配置。
- `rows`：可选覆盖每项 API 测试的条数，范围 1–50,000；留空时 `light` 使用 1,000，`stress` 使用 50,000。
- `repetitions`：每项 API 测试的独立进程次数，范围 1–10，默认 3。
- `sample_seconds`：每个 GUI 场景的采样时间，范围 1–120 秒，默认 10 秒。
- `preview`：额外生成实际界面截图，使用隔离的固定演示数据；dev.2 及之后支持，默认关闭。截图测试入口通过可选 feature 编译，不进入安装包。

一次运行只测一个代码版本，不自动选择或比较另一个版本。比较时分别运行两次，并保持工具链、profile、输入和环境一致。

统一 Rust 基准程序运行规则、连接、代理三类数据的流式/缓冲解析、1/5 次刷新，每项独立进程重复采样。GUI 使用隔离数据目录和 Xvfb，依次测首页、折叠代理组、搜索 `node-`、返回首页；默认低强度输入为 100 节点、5 组各含全部节点、1,000 条规则。`stress` 使用 2,000 节点、40 组、20,000 条规则。均关闭系统代理与定时测速，不生成业务流量；支持的版本会额外在五项快捷规则全开时重复四个场景。

为了让较旧的提交也能接受相同测试，工作流仅替换所选代码的 example 基准程序，应用源码保持所选提交内容；记录应用提交和基准程序提交。基准程序位于 `crates/client-core/examples`，运行器兼容旧版根目录布局。该入口面向与 0.4.12 API 兼容的代码。

产物包含 `metadata.json`、资源清单、完整输入订阅、API 数据、四个 GUI 场景的采样和截图、README 报告及构建日志。API 数值为 Rust 堆分配量；GUI/内核分别统计 RSS、PSS、USS，不能混用两类口径。采样峰值为观测峰值，内核内存波动不能直接归因于 Rust 改动。
