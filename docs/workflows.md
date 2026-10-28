# GitHub Actions 工作流

| 工作流 | 触发与职责 | 产物 |
| --- | --- | --- |
| [CI](workflows/ci.md) | main push、PR；六种桌面平台、三种 Android ABI 的检查、编译、源码测试及代理/TUN 集成 | `build-{system}-{arch}`，保留 3 天 |
| [Publish Release](workflows/release.md) | 手动；复用同提交成功 CI，打包及安装后验证，可仅打包或显式发布预发布 | 九组安装包及 SHA256，Actions 附件保留 3 天 |
| [Memory Benchmark](workflows/memory.md) | 手动；指定提交的轻负载或压力内存采样 | 原始数据、报告和日志，保留 3 天 |

桌面平台为 Windows x64/ARM64、Linux x64/ARM64、macOS Intel/Apple Silicon。Linux 包支持 Debian/Ubuntu，Ubuntu 最低为 22.04；最新版兼容验证在发布流程执行。iOS 仅有宿主占位与 UI 原型，不参与构建。

新工作流和 `.github/` 变更按[贡献要求](../.github/CONTRIBUTING.md)审核；弃用工作流删除文件后还需在 GitHub 禁用。脚本职责与合并/删除依据见[脚本维护](../scripts/README.md)。受限执行环境遵守 [AGENTS](../AGENTS.md#资源受限环境)，本地不编译、测试或打包。

## Actions 跟进节奏

先确认一次运行编号、来源提交和启动状态，再参考同工作流近期耗时低频查询。近期 [CI 40562836032](https://github.com/Fu021/Clash-of-Rust/actions/runs/40562836032) 用时约 29 分钟，[Publish Release 40567590202](https://github.com/Fu021/Clash-of-Rust/actions/runs/40567590202) 约 12 分钟：CI 建议 10 分钟首查、此后约 5 分钟；打包 7 分钟首查、临近完成每 2～3 分钟。排队和长步骤延长间隔，其他 Actions 同样先参考历史；失败后才读取必要日志，不反复下载整份日志。

## 本地打包

原生依赖、源码构建与产物导出命令见[本地打包](workflows/local-development.md#本地打包)。仅适用于资源充足的环境；本地产物不代表已通过 CI。

## CI

依赖快照、缓存预算、各平台测试与日志位置见 [CI 与依赖缓存](workflows/ci.md)。真实网络验证仅在可丢弃环境执行。

## Publish Release

参数、来源提交固定、签名、九平台验收及仅打包/预发布操作见[打包与发布](workflows/release.md)。默认 Agent 验收使用 `check_only=true`，公开发布须有明确授权。

### Windows Defender 告警

文件哈希核对、扫描收据及微软误报提交入口见[Defender 告警处理](workflows/release.md#windows-defender-告警)。

## 版本与更新通道

平台版本字段、正式/预发布选择、Android 安装限制见[版本与更新通道](workflows/versions.md)。

## 手动验收与助手清理

首次授权、重启/休眠、升级与 macOS 助手卸载见[手动验收](workflows/release.md#手动验收与助手清理)。

## UI 预览与静态资源

最小预览命令、输出清理与资源维护见[UI 预览](workflows/local-development.md#ui-预览与静态资源)。布局检查尺寸见[循环审查](loop-workflow.md#多尺寸-ui-检查)。

## Memory Benchmark

参数、采样场景与 RSS/PSS/堆分配口径见[内存基准](workflows/memory.md)。常规 CI 和打包不自动运行。
