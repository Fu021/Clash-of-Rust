# GitHub Actions 缓存容量与耗时

[缓存范围与保留规则](cache-coverage.md) · [工作流索引](../workflows.md)

## 当前上限与保留预算

维护者授权在 **5 美元/月**的 Actions 预算内为缓存换代留出空间。2026-11-28 通过 GitHub API 将仓库 `max_cache_size_gb` 从 34 提高到 **38 GB**，已读回确认。GitHub 每仓库含 10 GB 缓存，超额每 GB 月 0.07 美元；账户预算还需计入适用的其他 Actions 用量。见 [GitHub 仓库缓存设置](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository)及[缓存计费](https://docs.github.com/en/billing/concepts/product-billing/github-actions)。

缓存按每小时占用峰值累计，不按配置上限包月。以 30 天、720 小时计算，费用为 `Σ max(每小时峰值 GB − 10, 0) × 0.07 ÷ 720`。38 GB 整月占满约 **1.96 美元**；短于一小时也按该小时峰值计，跨两个计费小时可能同时计入。这只是缓存费估算，账户总费用以账单为准。

精简矩阵的 [CI 44712218319](https://github.com/Fu021/Clash-of-Rust/actions/runs/44712218319) 收尾从 22.40 降到 17.25 GiB；更早的多平台同时换代峰值曾达到 31.85 GiB。GitHub 计费中的 GB 定义为 2³⁰ 字节，与日志和脚本使用的 GiB 相同。继续观察缓存逐出、恢复耗时和账单，容量增加不能代替浪费排查。

## 近期 CI 耗时依据

提交 `107ddd2` 的冷 CI `43401803549` 从空缓存开始，耗时 35 分 41 秒；同提交暖 CI `43406564916` 耗时 17 分 31 秒。两轮共 12 项均通过。下表合计 Rust 缓存恢复、平台编译与测试、保存各步骤，Android 也包含 Go 和 APK 打包。runner 及排队条件会影响实测。

| 目标 | 冷填充 | 暖恢复 |
| --- | ---: | ---: |
| Windows x64 | 11 分 43 秒 | 1 分 14 秒 |
| Windows ARM64 | 11 分 32 秒 | 6 分 33 秒 |
| macOS Intel | 21 分 12 秒 | 4 分 18 秒 |
| macOS ARM64 | 12 分 11 秒 | 1 分 21 秒 |
| Linux x64 | 7 分 5 秒 | 30 秒 |
| Linux ARM64 | 7 分 19 秒 | 25 秒 |
| Android ARM64 | 11 分 7 秒 | 3 分 3 秒 |
| Android ARMv7 | 18 分 49 秒 | 2 分 42 秒 |
| Android x64 | 10 分 15 秒 | 3 分 14 秒 |

2026-11-21 修正 Windows 保存和恢复使用的源码回执路径后，[CI 44943304678](https://github.com/Fu021/Clash-of-Rust/actions/runs/44943304678) 两个 Windows 作业均恢复了已有缓存。「编译、测试与导出」步骤 x64 从上一轮的 11 分 58 秒缩短到 33 秒，ARM64 从 9 分 38 秒缩短到 4 分 36 秒。Maven 同轮命中，仓库缓存约 9.90 GiB。早期 Maven 缓存消失的原因仍缺少服务端逐出或删除记录。

2026-11-26 更新 Slint、Go 和 Gradle 后，[CI 45759645444](https://github.com/Fu021/Clash-of-Rust/actions/runs/45759645444) 13 项通过，耗时 40 分 40 秒。六个桌面 Rust 目标、两套 Linux 构建镜像和三个 Android Go 目标恢复缓存。Android 三个 Rust 目标和 Maven 缓存未命中，均重建并保存。最长步骤是 Android x64 编译，耗时 23 分 17 秒。收尾移除 8 条旧记录，缓存由 28.38 降到 17.57 GiB，保留的 19 条均属于 `main`。

`0.7.3` 的 [CI 45766776690](https://github.com/Fu021/Clash-of-Rust/actions/runs/45766776690) 13 项通过。macOS Intel 恢复了约 2.09 GiB 的 `main` Rust 缓存，编译、测试和候选包上传通过。本轮 Rust 缓存归档在压缩时达到步骤的 5 分钟上限，保留上一份可用缓存。预算作业将缓存从 32.05 降到 18.82 GiB，移除 9 条，余下 19 条均属 `main`。清理后的只读复查显示 0 条待清理。

正式版来源 `24a1c967` 的 [CI 45783249363](https://github.com/Fu021/Clash-of-Rust/actions/runs/45783249363) 13 项通过。九份 Rust 缓存均报告 `Cache up-to-date.`，包含 macOS Intel 和 ARM64。三个 Android Go 目标、共享 Maven 和两套 Linux 构建镜像均成功复用。[Memory Benchmark 45788967631](https://github.com/Fu021/Clash-of-Rust/actions/runs/45788967631) 的 Rust 缓存同样报告 `Cache up-to-date.`。两轮预算作业均无需移除条目，基准结束后共 19 条、19.19 GiB，均属 `main`，其中一条用于内存基准。

`70bd5c55` 的 [CI 45843335474](https://github.com/Fu021/Clash-of-Rust/actions/runs/45843335474) 13 项通过。macOS Intel 将 Rust 缓存保存时限增至 12 分钟，并用新键避开无法预留的旧键。该目标上传完整的 2.30 GiB，新键已在缓存清单中；其余八个 Rust 目标报告 `Cache up-to-date.`。Android 三个 Go 目标、Maven 和 Linux 双架构构建缓存均命中。预算作业移除旧 Intel 键，保留 19 条、19.26 GiB，均属 `main`。

Windows x64 在 [CI 46154880045](https://github.com/Fu021/Clash-of-Rust/actions/runs/46154880045) 的 Rust 后置步骤达到 5 分钟时限。其后 [CI 46161554685](https://github.com/Fu021/Clash-of-Rust/actions/runs/46161554685) 用约 2 分 30 秒保存，[CI 46165265219](https://github.com/Fu021/Clash-of-Rust/actions/runs/46165265219) 用约 4 分 12 秒上传 2.86 GiB。因保存耗时波动，Windows x64 的步骤时限调至 8 分钟。

`a87998f8` 的 [CI 46131889672](https://github.com/Fu021/Clash-of-Rust/actions/runs/46131889672) 13 项通过。九份 Rust 新缓存写入 17.67 GiB。Go、Maven 和两套 Linux 编译镜像命中。预算作业移除 8 条旧 Rust 缓存，收尾清单保留 `main` 的 14 条、19.26 GiB，其中没有本轮恢复过的 Maven 键。预算作业的删除列表没有该键，服务端未提供消失原因。按约 19.5 GiB 常驻和 17.7 GiB 一轮写入估算，两代重叠约 37.2 GiB。38 GB 仓库上限留出换代空间，预算作业仍按 30 GiB 目标收尾。

`a49947c5` 的 [CI 46134443632](https://github.com/Fu021/Clash-of-Rust/actions/runs/46134443632) 13 项通过。八份 Rust 新缓存写入，Windows ARM64 精确命中并复用；三份 Go 与两份 Linux 编译镜像命中，Maven 重建并保留。预算作业将 36.34 GiB 收至 19.48 GiB，移除 8 条旧项；最终 15 条均属 `main`。本轮未发现缓存保存失败。

## main 基线与分支只读

2026-11-11 的 CI `42978665228` 和 Memory `42978712845` 均通过。保留策略按用途合并新旧键名，每个目标仅保存 main 最新代。开发分支直接恢复可用基线，省掉差异归档的全目录扫描、构建后比较与叠加成本。
