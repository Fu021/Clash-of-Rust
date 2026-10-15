# GitHub Actions 工作流

| 工作流 | 触发 | 职责 | 产物 |
| --- | --- | --- | --- |
| Rust / CI | main push、PR | 六平台编译、静态检查、源码单元测试、系统代理与 TUN 集成测试 | `build-{system}-{arch}`，保留 14 天 |
| Publish Release | 手动 | 复用成功 CI 的程序，打包、安装后验证；可仅打包 | 六个安装包及 SHA256，测试产物保留 14 天 |
| Memory Benchmark | 手动 | 对指定代码运行低强度或压力内存测试 | 原始数据、报告与构建日志，保留 30 天 |

六个平台为 Windows x64 / ARM64、Linux x64 / ARM64、macOS Intel / Apple Silicon。Linux 安装包支持 Debian/Ubuntu，Ubuntu 仅支持 22.04 及以上；Ubuntu 最新版兼容验证在发布工作流中执行。

编译、Clippy、测试与打包均在 Actions 执行，本地不构建。界面截图只使用 `scripts/preview-slint.py` 直接预览正式 Slint 组件，不再保留截图工作流、额外截图编译入口或旧 Iced 适配器。弃用工作流删除文件后，也需在 GitHub 上禁用。

## CI

- 六平台使用相同的 Release profile 完成整个 workspace 的格式检查、Clippy、编译和源码单元测试；移动端占位目录不参与构建。
- 单元测试保留配置与规则、网络响应解析、认证信息脱敏、状态恢复、任务取消、列表复用与边界、实际指针交互等验证。已弃用的截图场景、CLI 专项集成测试和长期忽略的联网/平台手工回归入口已移除。
- 真实系统集成测试只运行系统代理与 TUN：验证代理设置写入、HTTP 转发、TUN 接管无显式代理的流量、模式恢复及关闭清理。Linux 使用隔离网络命名空间和 GSettings 内存后端；macOS 使用真实 launchd 网络助手；Windows 先注册服务，再以普通用户令牌运行代理/TUN、恢复和服务清理检查。
- Python 构建辅助与产物校验单元测试只在 Linux x64 执行一次。内存基准 example 不作为 CI 测试程序额外构建或运行。
- main push、PR 更新触发 CI；过时运行自动取消。Actions 的依赖与资源缓存保留，本地构建产物与缓存不保留。
- CI 不生成安装包。产物记录提交、版本、架构、Cargo.lock、工具链及逐文件校验值；仅附带发布后复用的 `core_integration` 代理/TUN 测试程序，不上传源码单元测试程序。清理后的产物采用 schema 2，只与同一提交的打包流程配套使用。

macOS 失败日志、系统与网络状态、崩溃报告保存在 `macos-ci-report-{arch}`，Windows 集成检查结果保存在 `windows-ci-report-{arch}`。人工授权窗口、真实重启登录和物理显示器行为仍需人工验收。

## Publish Release

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的当前提交 CI 编号；留空选择当前提交的 main CI。开发分支测试包必须填写当前分支、当前提交成功的 PR CI 编号。
- `check_only`：勾选后完成打包与安装后验证，不创建或修改 Release；开发分支必须勾选。安装包在本次运行的 `package-{system}-{arch}` 附件中，无需下载到本地。

流程固定到所选 CI 的提交，核对六平台成功记录和产物身份，不重新编译，也不重新下载应用资源。过期或旧格式产物需要重新运行当前代码的 CI。

1. 六个平台获取并校验已有程序，再生成 Windows EXE、Linux DEB、macOS DMG。
2. Windows 验证安装、服务注册、普通用户代理/TUN、更新安装路径、卸载与用户文件保留，并扫描 Defender；PE 图标在打包阶段核验。
3. Linux 安装 DEB 后验证普通用户系统代理、HTTP 转发、TUN 接管与清理；最新版 Ubuntu 使用同一份 Ubuntu 22.04 DEB 再验证一次。
4. macOS 核验 App/DMG 版本、架构、资源及签名，启动安装后的 GUI/内核并验证代理/TUN。当前使用 ad-hoc 签名，没有 Developer ID 与 Apple 公证。
5. 核对安装包、校验值、成功记录及来源后，正式模式才公开 Release；`check_only` 仅保留 Actions 附件。开发版本标记为 prerelease，不覆盖稳定版 latest。

只有最终发布 job 有 `contents: write`。`scripts/build-installer.py` 必须接收 `--prebuilt` 的已验证 CI 产物，已移除本地 Cargo 构建回退和 WSL 构建脚本。

## UI 预览与静态资源

持久安装官方 Slint Viewer 后运行：

```text
python3 scripts/preview-slint.py --dialogs --pairs
```

脚本直接加载正式 `.slint` 组件和演示数据，不运行 Cargo 或真实内核。需要时生成 `dist/slint-preview`；历史截图与字体缓存可随 `dist` 删除。

图标、国旗及许可证属于必要资源；日常构建使用仓库成品。仅在维护资源时使用 `scripts/prepare-assets.py icons` / `flags`。图标源图保留于 `resources/icon-concepts`，第三方来源见各资源目录的说明。平台检测直接维护 Rust，必要的公开检测数据与 AGPL 来源说明统一位于 `resources/ip-check`，不再保留 `vendor` 或 Shell 生成工具。

## Memory Benchmark

手动选择 `ref`（分支、标签或提交），默认测当前所选分支：

- `profile`：release 或 dev，默认 release。
- `workload`：默认 light；stress 保留较大的压力输入。
- `rows`：可选 API 行数，1–50,000；默认 light 为 1,000，stress 为 50,000。
- `repetitions`：每项 API 测试的独立进程次数，1–10，默认 3。
- `sample_seconds`：每个 GUI 场景采样 1–120 秒，默认 10 秒。

一次只测一个版本。比较时分别运行，保持工具链、profile、输入和环境一致；常规 CI 和打包不自动运行内存基准。

API 基准覆盖规则、连接、代理三类数据的流式/缓冲解析和 1/5 次刷新。GUI 在隔离数据目录、Xvfb 中采样首页、代理组折叠、节点搜索、返回首页；关闭系统代理与定时测速，不生成业务流量。light 为 100 节点、5 个代理组、1,000 条规则；stress 为 2,000 节点、40 个代理组、20,000 条规则。支持的版本另测五项快捷规则全开的同样场景。

工作流只替换所选版本的 example 基准程序，兼容旧版目录布局，不改应用源码。产物包含版本与环境、提交身份、资源清单、输入订阅、API 数据、GUI/内核内存采样和报告，不生成截图。API 为 Rust 堆分配量；GUI/内核分别统计 RSS、PSS、USS，需区分口径；采样峰值为观测峰值。
