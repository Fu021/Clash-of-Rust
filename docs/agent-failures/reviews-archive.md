# 较早任务复盘

[最近复盘](reviews.md)记录当前任务；本页保留较早任务的结论与验证证据，稳定经验仍以[主题索引](README.md)为准。

| 日期 | 任务 | 结果与主题 | 证据与验证边界 |
| --- | --- | --- | --- |
| 2026-11-07 | 共享诊断与移动测试职责 | [共享诊断](../cli/runtime.md#故障反馈与诊断报告)有界脱敏、错误分级及导出；桌面不代跑移动源码测试。 | `b17a1fc` CI `42258969558`、预发布 `42263689068` 通过，九包/18 附件核验；两版本输入专项通过。受限用户、人工授权及真实 Geo 恢复未全面验收。 |
| 2026-11-05 | 输入与 Geo 恢复 | [输入与 Geo 恢复](android-ime.md)统一键盘、窗口和文件事务，不把中断更新当成功。 | `c1762c3` CI `42076490014`、预发布 `42082073711` 通过，六桌面各 11 项 Geo 回归及 x64 输入通过。MIUI/K30、第三方输入法及原 Geo 备份丢失根因未验证或定位。 |
| 2026-11-04 | 依赖与构建 | [依赖与构建](build.md)升级适配，保留 tray-icon 等兼容约束和上游提示。 | `2ac829d` CI `41823452467`、内存 `41823632065`、不发布 `41828458776` 通过，九包和旧 Release 核验。历史 507/502/断连不能据绿色 job 判为服务恢复；部署由服务端维护。 |
| 2026-11-04 | 缓存职责与反馈退役 | [缓存职责](../workflows/cache-retention.md)改为 Rust 仅 GitHub、原包官方优先与私网回退、末尾交 URL 清单；反馈通道已退役。 | `bcdd903` CI `41864138011`、内存 `41864138050`、不发布 `41885018880` 通过。原包补齐、持久接收和实际恢复分别验收，不恢复 README 或 agent 通信。 |
| 2026-11-04 | macOS 助手与 Actions 契约 | [macOS 助手](macos-lifecycle.md#内核退出后的端口复用)分离单次连接失败与服务退出；[Actions](build.md)同步缓存和脚本契约。 | `715e02e` CI `41694322211`、不发布 `41697700116` 及 `69bb4e6` CI/打包通过。两种 macOS TUN 重启及旧 Ubuntu 完成；真机、人工交互和历史 TLS 不在范围内。 |
| 2026-10-28 | 断网启动恢复与正式发布 | GUI/CLI 共用后台监控默认路由，稳定后恢复出站/DNS 并保留状态，见[联网恢复](network-recovery.md)；完成 loop。 | `848beed` 的 [CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/40662840032) 和[测试打包](https://github.com/Fu021/Clash-of-Rust/actions/runs/40681164792)通过并获用户确认；正式版 `2040e87` 的 [CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/40710958862) 与[发布](https://github.com/Fu021/Clash-of-Rust/actions/runs/40717923064)通过。[追加复验](https://github.com/Fu021/Clash-of-Rust/actions/runs/40720999626)重试未获 runner 的 macOS ARM64，13 项通过，九组包、18 附件及 33 个 Release 未变。本机仅静态检查与代表尺寸预览；真实 tailnet、休眠和触感待验收。 |
| 2026-10-28 | Linux 普通用户、sudo 与 root 订阅 | 按[用户与授权](../cli/linux-users.md)保留每用户目录；sudo 降权复用调用者后台，root 可只读总览或显式选用户。清空环境后按白名单传递，显式选用户隔离管理员令牌与代理凭据；SSH 固定助手仍需管理员认证。 | `691ddb1` 的 CI 40647623124、不发布打包 40650865789 和 fix.9 预发布 40652400030 全部通过，九组包和 SHA256 已核对；公网操作、交互认证代理及真实登录待验收。 |
| 2026-10-28 | 保护锁定、订阅定时更新与代理组定位 | 保护期间灰置代理模式和整个规则页，保留业务校验与草稿；新增默认 360 分钟的远程[订阅自动更新](../subscriptions.md)，唯一所有者调度、过期丢弃、失败保留、成功才清理旧检测及刷新规则。桌面进入代理页时将实际策略组置顶并展开；[loop](../loop-workflow.md)补双向盘点已有便利交互。 | `efde5bb` 的 [CI 40624892505](https://github.com/Fu021/Clash-of-Rust/actions/runs/40624892505) 11 项通过，修正前轮多余 `mut`；未单独打包，随本轮用户隔离重验。真实下载、长时间定时及设备手势未验收。 |
| 2026-10-28 | 完整 loop、保护状态与文档清理 | Android 保护错误改归风险，读取停止线程的失败快照；共享控制接口恢复后清除暂时错误；短暂失联不覆盖通知或拦截失败，保留异常记录。按[循环审查](../loop-workflow.md)核对各端、UI 与 cor，拆分长文和精简复盘。 | `9926a2f` 的 [CI][ci-loop] 11 项及[不发布打包][package-loop] 13 项通过，九组包/18 附件核验通过，31 个 Release/324 个附件未变。本机仅静态检查与三种尺寸预览；实机、真实 tailnet 与吞吐未验收。 |
| 2026-10-27 | fix.6 预发布与 DNS 国家误报 | Google 官方数据修正错误标签；[真实国家差异仍属风险](../claude-protection.md#递归-dns-国家排查)。递归 DNS 与 IP 检测复用国旗，港澳台仅文字。 | `4696c81` 的 [CI 40562836032][ci-fix6] 11 项、[预发布 40567590202][release-fix6] 13 项通过，九组包及九份 SHA256 已独立核对；保留 fix.5，稳定 latest 为 v0.6.4。 |
| 2026-10-27 | macOS Tailscale DNS 与 CLI 重启 | [动态 DNS 共存](macos-dns.md)保留私有分域、接口与恢复记录；原生随机探针确认接管，延迟重发不外流。[端口复用](macos-lifecycle.md#内核退出后的端口复用)区分监听者与暂未释放。 | `2a3e09b` 的 [CI 40542749551][ci-tailscale] 11 项、[不发布打包 40546066504][package-tailscale] 13 项通过；九组包/18 附件及来源核验通过，Release 未变。真实 tailnet、接口/出口变化未验收。 |
| 2026-10-26 | 慢速回退与稳定工具栏 | 共享慢速/断流检测及缓冲下载；Geo 整组事务安装不重连 VPN。规则双向边缘滚动，移动键盘与空结果不重建搜索框，见[稳定列表](ui-and-tests.md#移动虚拟列表重叠与宽屏空间)。 | `457ec52` 的 [CI 40304558635][ci-toolbars] 11 项、[不发布打包 40308495398][package-toolbars] 13 项通过；九组包/18 附件核验通过，Release 未变。真实吞吐和系统字号仍需设备验收。 |
| 2026-10-25 | Defender 告警与规则/订阅循环 | 补[未压缩 EXE/DLL 扫描](desktop.md#9-windows-普通用户服务与安装器)；修复移动规则校验与拖动取消，失败不污染草稿。告警样本未经核验不能断言误报。 | `1d3e13e` 的 [CI 40210562854][ci-rules] 与[不发布打包 40212996727][package-rules]通过；Windows 两架构包及各五个 EXE 无威胁，用户样本仍待微软鉴定。 |
| 2026-10-25 | 订阅删除、Pad 与 fix.3 预发布 | [当前订阅删除与重复提交](ui-and-tests.md#订阅删除与重复提交)按事务处理；手机/Pad 工具导航、规则来源及完整原文同步，拖动按稳定 ID 修改完整草稿。 | [预发布 40283018496][release-fix3] 的 13 项任务、18 附件核验通过；保留 fix.2，未改稳定 latest；物理 Pad 未验收。 |
| 2026-10-25 | fix.2 预发布 | 按[来源与版本约束](release.md#10-发布产物版本与依赖快照)复用 `9623f45` 的成功 CI，使用 Android 长期发行签名。 | [CI 40176079496][ci-fix2] 与[预发布 40198947679][release-fix2]通过，九组包/18 附件核验通过，原 27 个 Release 未变。 |

[ci-loop]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40610758771
[package-loop]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40613948109
[ci-fix6]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40562836032
[release-fix6]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40567590202
[ci-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40304558635
[package-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40308495398
[ci-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40542749551
[package-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40546066504
[ci-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40210562854
[package-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40212996727
[release-fix3]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40283018496
[ci-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40176079496
[release-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40198947679
