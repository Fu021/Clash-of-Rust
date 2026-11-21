# GitHub Actions 缓存

[工作流索引](../workflows.md) · [容量与历史实测](cache-capacity.md)

## 编译对象只用 GitHub

CI 与 Memory Benchmark 使用 GitHub Actions Cache。`main` 每个用途及目标只保留最新一代，开发分支和 PR 只恢复兼容基线。Rust Cache 以编译器、平台及编译环境定兼容范围。工作区源码回执逐文件校验内容，匹配后恢复时间戳；变更的 crate、共享构建输入及 SVG 等资源由 Cargo 重新编译或失效重建。

桌面和 Android Rust 缓存使用独立的 `native-main-v4`、`android-main-v4` 族；每个目标只写入对应平台和共享依赖的编译对象。此前桌面缓存包含移动界面测试，隔离调整后必须清理受影响的旧族并分别冷建，不能通过兼容前缀回退到旧版本。移动端测试迁移后同样重建 Android 缓存，防止旧宿主产物混入；共用的注册表源码并非编译产物。后续改变平台边界时重复检查、更新族版本和清理受影响对象，保持[平台编译边界](../development.md#平台编译边界)。

CI 的桌面和 Android 编译缓存先查询当前源码对应的缓存，再回退到兼容缓存，只下载选中的一份；Android Maven/Go 缓存及 Linux 编译容器也先查询再恢复。临时查询或恢复故障最多尝试 100 次，超过 20 分钟后不再发起新一轮；正在进行的下载仍受缓存组件的分段超时及步骤、作业时限约束。恢复失败后清理该用途的部分数据，真正未命中时才冷构建；权限或配置错误以及重试耗尽会使作业失败，避免把故障当作未命中。Memory Benchmark 沿用自身缓存入口。

Android 恢复了旧编译对象且交叉 Clippy 报 Slint 或 `slint_macros` 缺失时，清理该作业的宿主和当前 ABI 两个 Release 目录并重跑一次 Clippy。交叉编译同时加载宿主生成的过程宏，仅重建 ABI 仍可能遇到旧宏。通过目录控制清理范围，规避 [`cargo clean` 无 `--package` 时忽略 `--target`](https://github.com/rust-lang/cargo/issues/17311) 的行为。重建成功后继续生成 APK、测试 APK 并运行 Lint；仍失败则保留编译错误。

| 工作流 | GitHub Actions Cache | 同次 CI 附件与公开下载 |
| --- | --- | --- |
| CI | 桌面六平台与 Android 三 ABI 的 Rust 对象、Android 分 ABI 的 Go 对象、Android 共享 Maven 构件及元数据、Ubuntu 20.04 两架构编译容器 | 分发依赖快照、已校验的 Go 源码、本轮共享的 Maven 依赖、候选包及报告 |
| Memory Benchmark | 各 profile 的 Linux x64 Rust 对象 | 复用冻结输入，保存本次基准的分片和报告 |
| Android Compatibility | 无 | 复用 CI APK，SDK、模拟器及系统镜像来自 Google 官方源 |
| Linux 发行版兼容验证 | Ubuntu 24.04 x64 VM 宿主工具的 DEB 下载 | 复用同提交候选包，使用各发行版的固定官方镜像；Ubuntu 20.04 x64 来宾复用冻结 APT 索引 |
| Publish Release | 无 | 校验同提交候选包和[所需兼容回执](release.md#兼容性发布门禁)，公开模式增加 Android 签名和签名包复验 |

`scripts/workflow_storage.py` 按创建时间保留最新缓存，移除旧代、已退役键及非 `main` 缓存。VM 宿主工具的前缀由当前 Linux 矩阵定义，撤出矩阵的工具配置随收尾清理。`--dry-run` 列出计划删除的条目，`--read-only` 只盘点。CI 和 Memory 共用串行收尾。当最新基线超过 **30 GiB** 时，脚本记录告警并保留这些基线。仓库上限与费用见[缓存容量](cache-capacity.md#当前上限与保留预算)。

## 下载与产物

- Gradle、NSIS、Mihomo、Geo、独立 Python 和 Linux 云镜像从公开上游下载，按固定摘要校验。Linux APT 软件包通过各发行版的签名索引核验。
- CI 将锁文件、资源和输入身份作为短期附件分发。Android 三种 ABI 共享已验证的 Go 源码下载包与绑定工具，独立保存各 ABI 的 `GOCACHE`。
- Android 的 [Maven 缓存](ci.md#android-工具与原包)在 `main` 共用一代，范围限于 `caches/modules-2` 中的 Maven 构件和解析元数据。Gradle 锁文件、缓存维护标记、转换产物、签名材料、APK 和报告均排除在该缓存之外。
- 构建容器按架构和官方镜像 digest 恢复，未命中时重新构建。运行 VM 的写入层只存当次 runner。安装包、回执、日志和内存报告以附件保留，生命周期见各工作流。
- VM 宿主从 Ubuntu 官方 HTTPS 源更新签名索引，缓存中的 DEB 按当前索引的大小与 SHA256 校验后才提供给 APT。
