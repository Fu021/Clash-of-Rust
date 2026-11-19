# GitHub Actions 缓存

[工作流索引](../workflows.md) · [容量与历史实测](cache-capacity.md)

## 编译对象只用 GitHub

CI 与 Memory Benchmark 使用 GitHub Actions Cache。`main` 每个用途及目标只保留最新一代，开发分支和 PR 只恢复兼容基线。Rust Cache 以编译器、平台及编译环境定兼容范围。工作区源码回执逐文件校验内容，匹配后恢复时间戳；变更的 crate、共享构建输入及 SVG 等资源由 Cargo 重新编译或失效重建。

| 工作流 | GitHub Actions Cache | 同次 CI 附件与公开下载 |
| --- | --- | --- |
| CI | 桌面六平台与 Android 三 ABI 的 Rust 对象、Android 分 ABI 的 Go 对象、Android 共享 Maven 构件及元数据、Ubuntu 20.04 两架构编译容器 | 分发依赖快照、已校验的 Go 源码、本轮共享的 Maven 依赖、候选包及报告 |
| Memory Benchmark | 各 profile 的 Linux x64 Rust 对象 | 复用冻结输入，保存本次基准的分片和报告 |
| Android Compatibility | 无 | 复用 CI APK，SDK、模拟器及系统镜像来自 Google 官方源 |
| Linux 发行版兼容验证 | VM 宿主工具的 Ubuntu 24.04 DEB 下载，按 runner 与所需工具保留最新一代 | 复用同提交候选包，使用各发行版的固定官方镜像；Ubuntu 20.04 来宾复用冻结 APT 索引 |
| Publish Release | 无 | 校验同提交候选包和[所需兼容回执](release.md#兼容性发布门禁)，公开模式增加 Android 签名和签名包复验 |
| COSMIC Startup Investigation | 无 | 使用同提交候选 DEB 和 System76 官方镜像，保存诊断附件 |

`scripts/workflow_storage.py` 按创建时间保留最新缓存，移除旧代、已退役键及非 `main` 缓存。`--dry-run` 列出计划删除的条目，`--read-only` 只盘点。CI 和 Memory 共用串行收尾。当最新基线超过 **20 GiB** 时，脚本记录告警并保留这些基线。仓库上限与费用见[缓存容量](cache-capacity.md#当前上限与保留预算)。

## 下载与产物

- Gradle、NSIS、Mihomo、Geo、独立 Python 和 Linux 云镜像从公开上游下载，按固定摘要校验。Linux APT 软件包通过各发行版的签名索引核验。
- CI 将锁文件、资源和输入身份作为短期附件分发。Android 三种 ABI 共享已验证的 Go 源码下载包与绑定工具，独立保存各 ABI 的 `GOCACHE`。
- Android 的 [Maven 缓存](ci.md#android-工具与原包)在 `main` 共用一代，范围限于 `caches/modules-2` 中的 Maven 构件和解析元数据。Gradle 锁文件、缓存维护标记、转换产物、签名材料、APK 和报告均排除在该缓存之外。
- 构建容器按架构和官方镜像 digest 恢复，未命中时重新构建。VM 系统盘及运行写入层只存当次 runner。安装包、回执、日志和内存报告以附件保留，生命周期见各工作流。
- VM 宿主从 Ubuntu 官方 HTTPS 源更新签名索引，缓存中的 DEB 按当前索引的大小与 SHA256 校验后才提供给 APT。
