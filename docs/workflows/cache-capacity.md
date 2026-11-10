# 全平台编译缓存容量估算

[缓存分工](cache-coverage.md) · [工作流索引](../workflows.md)

日期：2026-11-10。按当前全部构建平台、单代兼容环境和压缩后的归档大小估算，不以保留预算倒推。早期清单约 8.47 GiB，只能说明当时已保存部分的大小；下表是扩大覆盖的容量规划，并非完整上传后的实测。

| 类别 | 单代预估（GiB） |
| --- | ---: |
| Windows x64／ARM64 Rust 对象 | 3.5～5.0 |
| macOS Intel／ARM64 Rust 对象 | 3.2～4.5 |
| Linux x64／ARM64 Rust 对象 | 2.0～3.6 |
| Android 三 ABI 的 Rust／原生构建对象 | 2.7～4.5 |
| Android 三 ABI 的 Go 编译缓存 | 1.5～3.0 |
| Gradle 转换与构建缓存 | 0.9～2.4 |
| Linux x64 内存基准的 release／dev 对象 | 3.0～5.8 |
| macOS 两架构 Tailscale 及 gomobile／gobind 工具 | 0.1～0.3 |
| **合计** | **16.9～29.1** |

Rust 行包含其 Cargo registry/Git 数据和相关原生依赖构建结果，不再另加一份 Skia、HarfBuzz 或同一目录；不同平台或独立缓存归档之间实际重复的内容仍会占用空间。现有两套 macOS Tailscale 二进制合计约 63 MiB，工具行另为生成的 Android 绑定工具留余量。Go、Gradle 与本地 crate 的完整对象尚未全量留存，相应增量需要在实际启用后测量。

若未来启用表中全部缓存类型，单开发线一代含交替余量可按 **40～50 GiB** 估算，两代完整对象约 **33.8～58.2 GiB**。这些是假设扩展覆盖后的容量估算，不是现行配置建议，也不会自动启用新缓存类型。当前只保留全仓各目标最新一代，按下方实测设置容量，不为 main 与 PR 另留整套空间。

## 计入与排除

- 覆盖现有六个桌面目标、三个 Android ABI，以及内存基准支持的两种 profile。Ubuntu 20.04 兼容验证复用 Linux 候选包，不虚增另一套编译目标；Android 截图工作流复用 APK，不重新编译。
- 日志确认当前 `CARGO_INCREMENTAL=0`，合计不包含增量目录。若未来明确启用并保存，另预留约 5～10 GiB，再以实测修正。
- 官方分发包、SDK／NDK／JDK 安装目录、预装 VM 镜像、安装包、截图和日志不属于本表。预装 VM 仍只存私网，不能放进 GitHub Cache。
- iOS 仍为宿主占位，未建立正式构建工作流，不把不存在的 iOS 编译缓存计入。将来启用后应单独测量并扩充预算。

估算依据为提交 `5a21964` 的工作流、CI `42712647995` 日志与缓存清单；这是存储规划，不是冷／热编译性能对照。是否值得新增 Go、Gradle 或更多项目对象缓存，应再比较恢复、保存与重编耗时。

## 当前上限与保留预算

维护者已设置每月 2 美元的账户预算；2026-11-10 按实际清单收紧容量，仓库 `max_cache_size_gb` 设为 **15 GB**，设置 API 读回已确认。账户预算不自动提高仓库上限，两个设置均需配置，见 [GitHub 仓库缓存设置](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository)。

北京时间 2026-11-10 16:27 的逐项清单为 **11 条、10.09 GiB**：九个平台 Rust 缓存和两份 macOS Tailscale 工具；这是已保存部分的压缩大小，没有 Memory profile、Go 或 Gradle 缓存，不能替代上表全覆盖估算。

CI 与 Memory 共用 `scripts/workflow_storage.py` 的 **13 GiB** 保留预算，低于仓库 **15 GB** 配置上限；两处沿用各自标示单位，余量不保证容纳任何完整归档。全仓同一逻辑目标仅保留按创建时间选出的最新一代，访问旧记录不改变代际顺序，容量不足再按[编译优先级](cache-coverage.md#编译对象只用-github)淘汰。GitHub 的分支读取作用域仍然生效，不将这个保留策略描述为跨分支共享存储；费用以 GitHub 实际[缓存计费](https://docs.github.com/en/billing/concepts/product-billing/github-actions)为准。

本次未新增 Go/Gradle 编译缓存；上表仍保留全平台容量规划，不能把估算总量写成已经覆盖。Ubuntu 20.04 两种预装镜像继续仅存私网，不占此额度。

## 近期 CI 耗时依据

2026-11-10 核对六轮完整成功 CI：`42712647995`、`42705403797`、`42697699786`、`42690416513`、`42679544543`、`42655456299`；排除只刷新依赖的运行。各平台恢复与编译的中位数及保留顺序见[缓存分工](cache-coverage.md#编译对象只用-github)。

恢复包含兼容 workspace，桌面编译包含检查与测试，Android 编译包含应用、Go 内核和 Rust，不是纯 Rust 耗时。排队、安装验证、模拟器及清单收尾不用于排序，也不将中位数当作缓存净节约。macOS Intel 整个 job 中位数约 16.6 分钟，其中恢复约 2.3 分钟、编译约 8 分钟；Ubuntu 20.04 ARM64 约 15.8 分钟，包含 TCG 启动、候选包等待及安装验证。它们是主要耗时项，但 VM 继续走私网预装模板，不能因耗时较长就加入 GitHub Cache。

Tailscale 同样影响 CI：macOS ARM64 的 `42679544543` 日志记录从源码安装，接入步骤 166 秒；`42712647995` 两架构命中 31,794,999／34,199,007 字节缓存，接入步骤分别为 3／7 秒。六轮接入中位数为 5.5／7 秒；热态数字短恰好说明小工具缓存有效，不能反过来作为淘汰依据。这不是同一提交的严格冷暖实验，但足以保留约 63 MiB 的两架构工具，避免重复编译。上游源码构建行为见 [Tailscale Action 问题](https://github.com/tailscale/github-action/issues/287)。
