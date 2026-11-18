# GitHub Actions 工作流

| 工作流 | 触发与职责 | 产物 |
| --- | --- | --- |
| [CI](workflows/ci.md) | main push、PR；源码检查、编译及九组候选包的安装回归 | 九组 `build-*`、九组 `package-*`、SHA256 和收据，保留 3 天 |
| [Publish Release](workflows/release.md) | 手动；核验同提交成功 CI 及两套兼容性回执；公开模式另验 Android 发行签名并发布 | 桌面复用 CI 附件；公开模式追加三组发行签名 APK，保留 3 天 |
| [Memory Benchmark](workflows/memory.md) | 手动；指定提交的轻负载或压力内存采样 | 原始数据、报告和日志，保留 3 天 |
| [Android Compatibility](workflows/android-compatibility.md) | 手动；复用成功 CI 的 x64 APK，Android 8.0～16 基础代理/TUN，截图与诊断可选 | 状态和日志保留 7 天，发布回执保留 3 天 |
| [Ubuntu Compatibility](workflows/ubuntu-compatibility.md) | 手动；在固定版本的 x64／ARM64 托管 runner 或 Canonical 云镜像上验收同提交 DEB 的安装与代理/TUN | 小型报告及发布回执保留 3 天 |
| [Debian Compatibility](workflows/debian-compatibility.md) | 手动；Debian 12 和 13 x64 的原生内核 VM，验收同提交 DEB 的安装生命周期、GUI/TUI/CLI、登录自启动与代理/TUN | 小型报告及兼容回执保留 3 天 |
| [Linux Mint Compatibility](workflows/linuxmint-compatibility.md) | 手动；Linux Mint 21.3 和 22.3 Cinnamon x64 的官方 ISO VM，验收同提交 DEB、原生桌面、登录自启动及代理/TUN | 小型报告及兼容回执保留 3 天 |
| [COSMIC Startup Investigation](workflows/cosmic-startup.md) | 手动诊断；官方 Pop!_OS live VM，核对旧自启动入口迁移、后台登录和手动打开 | JSON、原始截图与 VM 日志，保留 3 天 |

普通 CI 共 12 个必需 job（共享输入 1、桌面 6、Android 3、网站验证 1、缓存预算 1）。网站未变化时，网站任务只验证本轮范围。发布需要同提交、同源 CI 的 Android 和 Ubuntu 兼容回执。Debian 是独立的平台验收。Publish Release 的 `check_only` 执行解析和最终核验，公开发布另执行三个 Android 签名 job。Memory Benchmark 执行采样与缓存预算两个 job。依赖维护模式不作为发布来源。

COSMIC 诊断按需手动执行一个 VM job，保留供后续桌面启动排障复用，不加入常规 CI 或发布必需门禁。

桌面平台为 Windows x64/ARM64、Linux x64/ARM64、macOS Intel/Apple Silicon。Linux 构建基线为 Ubuntu 20.04，安装目标见各发行版工作流。iOS 仅有宿主占位与 UI 原型，不参与构建。

新工作流和 `.github/` 变更按[贡献要求](../.github/CONTRIBUTING.md)审核；弃用工作流删除文件后还需在 GitHub 禁用。脚本职责与合并/删除依据见[脚本维护](../scripts/README.md)。受限执行环境遵守 [AGENTS](../AGENTS.md#资源受限环境)，本地不编译、测试或打包。

## Actions 跟进节奏

先确认一次运行编号、来源提交和启动状态，再参考同工作流近期耗时低频查询。候选包和安装验证前移、常规测试精简后的实测见[CI 耗时记录](workflows/ci.md#测试精简后的耗时)。迁移前的 CI `41966802804` 用时 24 分 17 秒、Memory `41966849950` 用时 8 分 46 秒、Publish Release `41969428145` 用时 31 分 50 秒；范围不同，不能直接与现在的工作流比较。CI 建议 10 分钟首查、此后约 5 分钟；发布 7 分钟首查、临近完成每 2～3 分钟，纯 `check_only` 可在已启动且完成后立即核对。排队和长步骤延长间隔；失败后才读取必要日志，不反复下载整份日志。

若运行已失败，且 job 注解或日志明确显示长时间未分配到 runner，Agent 自动重试对应失败 job，复用同提交的成功结果并继续低频跟进，无需用户提醒；先等待本轮结束，再用 `gh run rerun <run-id> --job <job-id>`。job ID 取 `gh run view <run-id> --json jobs` 中的 `databaseId`，不要与 run ID 混淆。仍在排队时不取消或重跑；无法确认原因、代码、测试及安装校验失败均先定位，不能当作 runner 故障重试。相同容量故障再次出现时延长等待，持续记录原因；适用于 CI、打包和其他已获授权的 Actions，不增加工作流或访问权限。

同一轮有多个已确认因 runner 未分配而中断的 job 时，可用 `gh run rerun <run-id> --failed` 一起重试，保留成功结果。平台故障可能使顶层状态和 job 列表延迟更新；核对来源提交、`run_attempt`、runner 分配及实际步骤，不把 API 的 `started_at` 单独视为执行证据。最终结论以完整 job 结果、注解和日志复核，平台排队与重试耗时单列，不用来判断缓存提速。

### 运行日志与耗时审查

每次 loop 开始时核对同仓库最近一次 CI 和 Publish Release，记录运行编号、分支、完整提交、attempt、状态与参数；最近失败或取消的运行也须检查，最近成功运行可作补充基线。运行中的任务按上述节奏等到结束，不新增公开发布。历史运行只用于定位，最终验证必须属于本次提交。

读取任务注解及实际步骤日志，包括 stderr、缓存回退、`continue-on-error` 和 post 阶段，检查 warning、弃用 API、action 运行时、Gradle/SDK 与构建工具提示。逐项记录来源、根因、修复及验证；绿色 job 不能代替日志审查。不用全局关闭 warning、降低断言或删除验证消除提示；平台容量公告和外部组件尚无可用修复的提示须保留来源、影响与后续条件，不能标为已修复。

按实际时间区分排队与执行，核对最长 job、步骤及依赖等待，优先消除无关平台阻塞、重复下载、无效缓存和重复初始化。优化保留适用验证的同提交身份、校验及安装、GUI/CLI 与代理/TUN 覆盖；比较相同架构、验证范围和缓存状态，记录冷缓存与 TCG/KVM 差异，不用一次暖缓存运行推断普遍提速。本任务适用的 CI 与核验完成后再检查本节，出现新问题继续 loop。

## 本地打包

原生依赖、源码构建与产物导出命令见[本地打包](workflows/local-development.md#本地打包)。仅适用于资源充足的环境；本地产物不代表已通过 CI。

## CI

依赖快照、各平台测试与日志位置见 [CI](workflows/ci.md)，缓存来源与保留规则见 [GitHub Actions 缓存](workflows/cache-coverage.md)。真实网络验证仅在可丢弃环境执行。

## Publish Release

参数、来源提交固定、签名、九平台验收及仅打包/预发布操作见[打包与发布](workflows/release.md)。默认 Agent 验收使用 `check_only=true`，公开发布须有明确授权。

### Windows Defender 告警

文件哈希核对、扫描收据及微软误报提交入口见[Defender 告警处理](workflows/release.md#windows-defender-告警)。

## 版本与更新通道

平台版本字段、正式/预发布选择、Android 安装限制见[版本与更新通道](workflows/versions.md)。

## 发布后说明与助手清理

Actions 已验证范围、硬件专属边界与 macOS 助手卸载见[发布后说明](workflows/release.md#发布后说明与助手清理)。

每次公开发行后另执行 [loop-website](loop-website.md)，核对实际下载、三类教程、双语与手机布局；浏览器验证复用现有 CI 的网站任务。

## UI 预览与静态资源

最小预览命令与资源维护见[UI 预览](workflows/local-development.md#ui-预览与静态资源)。布局检查尺寸见[循环审查](loop-workflow.md#多尺寸-ui-检查)。

## Memory Benchmark

参数、采样场景与 RSS/PSS/堆分配口径见[内存基准](workflows/memory.md)。常规 CI 和打包不自动运行。
