# 较早任务复盘

[最近复盘](reviews.md)记录当前任务；本页保留较早任务的结论与验证证据，稳定经验仍以[主题索引](README.md)为准。

| 日期 | 任务 | 结果与主题 | 证据与验证边界 |
| --- | --- | --- | --- |
| 2026-10-27 | macOS Tailscale DNS 与 CLI 重启 | [动态 DNS 共存](macos-dns.md)保留私有分域、接口与恢复记录；原生随机探针确认接管，延迟重发不外流。[端口复用](macos-lifecycle.md#内核退出后的端口复用)区分监听者与暂未释放。 | `2a3e09b` 的 [CI 40542749551][ci-tailscale] 11 项、[不发布打包 40546066504][package-tailscale] 13 项通过；九组包/18 附件及来源核验通过，Release 未变。真实 tailnet、接口/出口变化未验收。 |
| 2026-10-26 | 慢速回退与稳定工具栏 | 共享慢速/断流检测及缓冲下载；Geo 整组事务安装不重连 VPN。规则双向边缘滚动，移动键盘与空结果不重建搜索框，见[稳定列表](ui-and-tests.md#移动虚拟列表重叠与宽屏空间)。 | `457ec52` 的 [CI 40304558635][ci-toolbars] 11 项、[不发布打包 40308495398][package-toolbars] 13 项通过；九组包/18 附件核验通过，Release 未变。真实吞吐和系统字号仍需设备验收。 |
| 2026-10-25 | Defender 告警与规则/订阅循环 | 补[未压缩 EXE/DLL 扫描](desktop.md#9-windows-普通用户服务与安装器)；修复移动规则校验与拖动取消，失败不污染草稿。告警样本未经核验不能断言误报。 | `1d3e13e` 的 [CI 40210562854][ci-rules] 与[不发布打包 40212996727][package-rules]通过；Windows 两架构包及各五个 EXE 无威胁，用户样本仍待微软鉴定。 |
| 2026-10-25 | 订阅删除、Pad 与 fix.3 预发布 | [当前订阅删除与重复提交](ui-and-tests.md#订阅删除与重复提交)按事务处理；手机/Pad 工具导航、规则来源及完整原文同步，拖动按稳定 ID 修改完整草稿。 | [预发布 40283018496][release-fix3] 的 13 项任务、18 附件核验通过；保留 fix.2，未改稳定 latest；物理 Pad 未验收。 |
| 2026-10-25 | fix.2 预发布 | 按[来源与版本约束](release.md#10-发布产物版本与依赖快照)复用 `9623f45` 的成功 CI，使用 Android 长期发行签名。 | [CI 40176079496][ci-fix2] 与[预发布 40198947679][release-fix2]通过，九组包/18 附件核验通过，原 27 个 Release 未变。 |

[ci-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40304558635
[package-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40308495398
[ci-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40542749551
[package-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40546066504
[ci-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40210562854
[package-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40212996727
[release-fix3]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40283018496
[ci-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40176079496
[release-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40198947679
