# GitHub Actions 缓存容量与耗时

[缓存范围与保留规则](cache-coverage.md) · [工作流索引](../workflows.md)

## 当前上限与保留预算

维护者于 2026-11-21 同意增加 GitHub Actions 缓存，费用保持低于 **2 美元/月**。仓库 `max_cache_size_gb` 从 24 提高到 **34 GB**，已用 GitHub API 读回。GitHub 每仓库含 10 GB 缓存，超额每 GB 月 0.07 美元；账户预算还需计入适用的其他 Actions 用量。见 [GitHub 仓库缓存设置](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository)及[缓存计费](https://docs.github.com/en/billing/concepts/product-billing/github-actions)。

缓存按每小时占用峰值累计，不按配置上限包月。以 30 天、720 小时计算，费用为 `Σ max(每小时峰值 GB − 10, 0) × 0.07 ÷ 720`。34 GB 整月占满约 **1.68 美元**；短于一小时也按该小时峰值计，跨两个计费小时可能同时计入。这只是缓存费估算，实际以账单为准，不能视为账户总费用的硬上限。

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

## main 基线与分支只读

2026-11-11 的 CI `42978665228` 和 Memory `42978712845` 均通过。保留策略按用途合并新旧键名，每个目标仅保存 main 最新代。开发分支直接恢复可用基线，省掉差异归档的全目录扫描、构建后比较与叠加成本。
