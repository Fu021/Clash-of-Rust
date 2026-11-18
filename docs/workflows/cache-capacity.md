# GitHub Actions 缓存容量与耗时

[缓存范围与保留规则](cache-coverage.md) · [工作流索引](../workflows.md)

## 当前上限与保留预算

本仓库授权预算为每月 **1 美元**；2026-11-10 仓库 `max_cache_size_gb` 从 15 调整为 **24 GB**，设置 API 已读回确认。GitHub 此处的 GB 按 2³⁰ 字节计算，即 GiB。10 GiB 免费，超出部分每 GiB 月 0.07 美元；本仓库满额整月约 0.98 美元，账户其他仓库和已产生费用须一并计入预算。账户预算不自动提高仓库上限，两个设置均需配置，见 [GitHub 仓库缓存设置](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository)及[缓存计费](https://docs.github.com/en/billing/concepts/product-billing/github-actions)。

缓存按每小时占用峰值累计，不按配置上限包月。以 30 天、720 小时计算，费用为 `Σ max(每小时峰值 GiB − 10, 0) × 0.07 ÷ 720`。24 GiB 只出现一个计费小时约 0.00136 美元；每天一个小时、其余时间不超过 10 GiB，整月约 0.041 美元。若常驻 10.91 GiB、每天一个小时升到 24 GiB，约 0.102 美元。短于一小时也按该小时峰值计，跨两个计费小时可能同时计入；这些是假设估算，实际以账单为准。

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

## main 基线与分支只读

2026-11-11 的 CI `42978665228` 和 Memory `42978712845` 均通过。保留策略按用途合并新旧键名，每个目标仅保存 main 最新代。开发分支直接恢复可用基线，省掉差异归档的全目录扫描、构建后比较与叠加成本。
