# 文档导航

按要完成的事情选择入口。安装包、支持平台和功能概览见[项目首页](../README.md)；代码位置见[仓库目录与职责](development.md#目录与职责)。

## 使用与排障

| 想做什么 | 阅读位置 |
| --- | --- |
| 安装、升级与开始使用 | [项目首页](../README.md#安装与使用)；[Android 使用与兼容性](../apps/android/README.md) |
| 使用 `cor` 命令行 | [CLI 入口](cli.md)，按需查[运行与诊断](cli/runtime.md)、[订阅与规则](cli/profiles-and-rules.md)、[维护与更新](cli/maintenance.md) |
| 确认 Linux 用户、sudo、root 与 TUN 归属 | [Linux 用户与授权](cli/linux-users.md) |
| 导入或更新订阅、了解自动更新与失败保留 | [订阅说明](subscriptions.md) |
| 选择哪些 Android 应用进入 VPN | [Android 应用分流](android-application-routing.md) |
| 使用 Claude 保护、了解接管与恢复 | [保护说明](claude-protection.md) |
| 理解 A÷检测的项目、结果和边界 | [地区检测](claude-region-detection.md) |
| 遇到错误，导出诊断或新建 Issue | [故障反馈与诊断报告](cli/runtime.md#故障反馈与诊断报告)；GUI 入口及错误分级见[通知与故障反馈](../crates/client-ui/README.md#通知与故障反馈) |

## 开发与界面

| 想做什么 | 阅读位置 |
| --- | --- |
| 找代码、理解模块与数据流 | [开发与验证](development.md)，含平台边界、占用锁与验证要求；[全仓职责树](structure-review.md#目录与职责) |
| 校正文档冲突、拆分大文件或规划目录重构 | [文档与代码结构复查](structure-review.md) |
| 贡献代码或文档 | [贡献者须知](../.github/CONTRIBUTING.md)；agent 另读[开发约束](../AGENTS.md) |
| 修改桌面或移动界面、预览正式组件 | [界面设计与审阅](ui-design.md)；[Slint 界面说明](../crates/client-ui/README.md)；[预览与本地工具](workflows/local-development.md) |
| 修改 Android 宿主、VPN 或输入法 | [Android 开发说明](../apps/android/README.md)；[输入法兼容性复盘](agent-failures/android-ime.md) |
| 完成功能或修复后的循环审查 | [loop 工作流](loop-workflow.md)、[逐项清单](loop-checklist.md)及[代表尺寸与 UI 检查](loop-ui.md) |
| 查构建脚本入口、职责与调用关系 | [脚本维护说明](../scripts/README.md) |

iOS 当前只有宿主占位与界面原型；预览不代表已有可安装、可连接的客户端。平台支持和设备验证以项目首页及各端说明为准。

## 构建、缓存与发布

先看[工作流总览](workflows.md)，再按任务进入对应说明：

| 想做什么 | 阅读位置 |
| --- | --- |
| 理解 CI 的平台、源码测试和真实网络验证 | [CI](workflows/ci.md) |
| 核对候选安装包、平台安装检查与成功收据 | [CI 候选包与安装验收](workflows/install-validation.md) |
| 不发布打包、核验安装包或公开发布 | [Publish Release](workflows/release.md)；[发布文案模板](releases/TEMPLATE.md) |
| 测量内存和窗口关闭后的资源释放 | [Memory Benchmark](workflows/memory.md) |
| 复现 COSMIC 登录、自启动入口迁移与后台窗口行为 | [COSMIC 启动诊断](workflows/cosmic-startup.md)，按需手动执行 |
| 在本机编译、打包与预览 | [本地开发](workflows/local-development.md) |
| 理解版本、更新通道与依赖快照 | [版本和依赖](workflows/versions.md)；[版本管理](development.md#版本管理) |
| 排查测速失败与 Android 断网 | [内部诊断与错误码](diagnostics.md)；[故障报告导出](cli/runtime.md#故障反馈与诊断报告) |
| 对比各平台更新、Geo、测速和跨用户 TUN 路径 | [网络链路与取证边界](network-paths.md) |
| 排查无 IPv6、双栈与 TUN 地址族变化 | [IPv6 判定、保护与网络切换](ipv6-tun.md) |
| 理解 GitHub 与私网的缓存分工 | [缓存覆盖](workflows/cache-coverage.md)、[保留白名单](workflows/cache-retention.md)、[服务边界与协议](workflows/cache-server.md) |
| 对接 Ubuntu 20.04 预装测试环境镜像 | [镜像复用与测速及验收证据](workflows/prepared-vm-images.md)、[v1 对接约定及共用向量](workflows/prepared-vm-protocol.md) |

资源受限的机器只做静态检查与必要预览，具体限制见[开发约束](../AGENTS.md#资源受限环境)。`dependency-audit.csv` 是历史核对快照；当前依赖与工具以源码声明、锁文件和[构建工具基线](../.github/build-versions.json)为准。

## 发布历史、复盘与第三方来源

- [`releases/`](releases/)仅保留当前正式版、最近公开预发布、当前开发版本及[文案模板](releases/TEMPLATE.md)；旧版文案查 Git 历史或 GitHub Releases。已发布正式版入口见[项目首页](../README.md)，开发分支说明不等于已发布版本。
- 排查已知失败模式从[复盘索引](agent-failures/README.md)进入；[最近复盘](agent-failures/reviews.md)记录证据和验证范围，较早记录在[历史复盘](agent-failures/reviews-archive.md)。
- 软件、字体及数据的许可见[第三方声明](../resources/THIRD-PARTY-NOTICES.txt)；[字体来源](../resources/fonts/SOURCE.md)、[国家和地区数据](../resources/countries/SOURCE.md)、[IP 检测来源](../resources/ip-check/SOURCE.md)分别保留来源与修改说明。
- 界面依赖的局部兼容修复见 [Android 后端补丁](../vendor/slint-android-activity/PATCHES.md)、[软件渲染补丁](../vendor/slint-renderer-software/PATCHES.md)和 [winit X11 同步补丁](../vendor/winit/PATCHES.md)。

这里维护查阅入口。命令、行为和协议只在对应主题文档中说明；修改时更新主题及必要链接，避免在多个入口复制整套规则。
