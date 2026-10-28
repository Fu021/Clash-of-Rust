# 最近复盘

[主题索引](README.md)保存稳定经验；本页只记近期结论和验证范围，最多 20 条，不复制排障过程。新增时先合并同类任务和过时待办；运行记录不代表当前提交已通过。

| 日期 | 任务 | 结果与主题 | 证据与验证边界 |
| --- | --- | --- | --- |
| 2026-10-28 | Ubuntu 20.04、GUI 与 Geo | 增加旧系统及各端生命周期验证；Geo 安装成功时间可持久显示，设置页合并计划任务及更新卡片。[Android 重建](android.md#6-android-vpndns-与系统-ui-时序)回移上游窗口释放补丁；隔离 DNS 的测试 CA 使用独立配置避免重复扩展，并校验证书包挂载与证书链。 | [CI 40738476623](https://github.com/Fu021/Clash-of-Rust/actions/runs/40738476623) 暴露 Linux 测速与 Android EGL 失败；[CI 40745218037](https://github.com/Fu021/Clash-of-Rust/actions/runs/40745218037) 发现 CLI Geo 时间格式化编译错误；[CI 40755635088](https://github.com/Fu021/Clash-of-Rust/actions/runs/40755635088) 确认 Linux 证书包未识别测试根。本机仅静态检查和预览，修复效果待新提交 CI，实机待验收。 |
| 2026-10-28 | 断网启动恢复与正式发布 | GUI/CLI 唯一后台监控默认路由，稳定后刷新出站/DNS 并保留状态，见[联网恢复](network-recovery.md)。loop 复查功能、UI、cor、跨端与资源边界，拆出主题并精简长文。 | `848beed` 的 [CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/40662840032) 和[测试打包](https://github.com/Fu021/Clash-of-Rust/actions/runs/40681164792)通过，用户确认测试包；正式版 `2040e87` 的 [CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/40710958862) 与[发布](https://github.com/Fu021/Clash-of-Rust/actions/runs/40717923064)通过。[追加复验](https://github.com/Fu021/Clash-of-Rust/actions/runs/40720999626)按注解重试未获 runner 的 macOS ARM64，13 项通过，九组包、18 附件及全部 33 个 Release 未变。本机仅静态检查与代表尺寸预览；真实 tailnet、休眠和触感仍需设备验收。 |
| 2026-10-28 | Linux 普通用户、sudo 与 root 订阅 | 按[用户与授权](../cli/linux-users.md)保留每用户目录；sudo 降权复用调用者后台，root 可只读总览或显式选用户。清空环境后按白名单传递，显式选用户隔离管理员令牌与代理凭据；SSH 固定助手仍需管理员认证。 | `691ddb1` 的 CI 40647623124、不发布打包 40650865789 和 fix.9 预发布 40652400030 全部通过，九组包和 SHA256 已核对；公网操作、交互认证代理及真实登录待验收。 |
| 2026-10-28 | 保护锁定、订阅定时更新与代理组定位 | 保护期间灰置代理模式和整个规则页，保留业务校验与草稿；新增默认 360 分钟的远程[订阅自动更新](../subscriptions.md)，唯一所有者调度、过期丢弃、失败保留、成功才清理旧检测及刷新规则。桌面进入代理页时将实际策略组置顶并展开；[loop](../loop-workflow.md)补双向盘点已有便利交互。 | `efde5bb` 的 [CI 40624892505](https://github.com/Fu021/Clash-of-Rust/actions/runs/40624892505) 11 项通过，修正前轮多余 `mut`；未单独打包，随本轮用户隔离重验。真实下载、长时间定时及设备手势未验收。 |
| 2026-10-28 | loop 统一收尾 | [loop 工作流](../loop-workflow.md#统一收尾)默认包含脚本、文档、复盘、清洁及适用的提交验证；用户无需另行提醒，不扩大推送或发布授权。 | 本轮仅文档，检查差异、本地链接/锚点、长度与整洁；保留本地随下次功能更新提交，不触发 CI/打包。 |
| 2026-10-28 | 完整 loop、保护状态与文档清理 | Android 保护错误改归风险，读取停止线程的失败快照；共享控制接口恢复后清除暂时错误；短暂失联不覆盖通知或拦截失败，保留异常记录。按[循环审查](../loop-workflow.md)核对各端、UI 与 cor，拆分长文和精简复盘。 | `9926a2f` 的 [CI][ci-loop] 11 项及[不发布打包][package-loop] 13 项通过，九组包/18 附件核验通过，31 个 Release/324 个附件未变。本机仅静态检查与三种尺寸预览；实机、真实 tailnet 与吞吐未验收。 |
| 2026-10-27 | fix.6 预发布与 DNS 国家误报 | Google 官方数据修正错误标签；[真实国家差异仍属风险](../claude-protection.md#递归-dns-国家排查)。递归 DNS 与 IP 检测复用国旗，港澳台仅文字。 | `4696c81` 的 [CI 40562836032][ci-fix6] 11 项、[预发布 40567590202][release-fix6] 13 项通过，九组包及九份 SHA256 已独立核对；保留 fix.5，稳定 latest 为 v0.6.4。 |
| 2026-10-27 | macOS Tailscale DNS 与 CLI 重启 | [动态 DNS 共存](macos-dns.md)保留私有分域、接口与恢复记录；原生随机探针确认接管，延迟重发不外流。[端口复用](desktop.md#8-macos-应用身份launchd-与打包)区分监听者与暂未释放。 | `2a3e09b` 的 [CI 40542749551][ci-tailscale] 11 项、[不发布打包 40546066504][package-tailscale] 13 项通过；九组包/18 附件及来源核验通过，Release 未变。真实 tailnet、接口/出口变化未验收。 |
| 2026-10-27 | 旧连接、桌面拖动与 Geo 恢复 | 保护应用后关闭内核旧连接再验证 DNS，失败回滚；桌面拖动修正卡片命中偏移；[Geo 事务](android.md#geo-与安装包下载)按旧文件实际内容备份，损坏或缺失不阻止有效更新。 | 当时仅本机静态检查；相关变更已纳入后续 fix.6。Windows 实机拖动、Android Geo 原现场及 macOS 虚拟机现场未复现。 |
| 2026-10-27 | 桌面代理状态与恢复 | 按[托管字段](desktop.md#系统代理状态与安全恢复)比较真实代理，GUI/CLI 同一快照；恢复合并外部修改，不覆盖绕过项、新增服务或用户后来编辑。 | 补纯逻辑、窗口投影及 macOS 原生恢复回归；本机未编译或做网络试验，后续集成证据见 fix.6。 |
| 2026-10-26 | 规则拖动跟随 | 桌面/Android 共用浮起预览，源行回收后仍跟随，自动滚动不带离指针；松手、Esc、筛选与禁用取消，见[规则交互](ui-and-tests.md#移动虚拟列表重叠与宽屏空间)。 | 本机仅 Slint、格式和单页预览；源行回收、位置偏移与取消回归纳入 CI。物理设备触感未验收。 |
| 2026-10-26 | 清理 Go 缓存 | 指定 `/home/coder/go` 已删除；沙箱只读视图经权限流程解决，无需改宿主挂载，见[清理约束](build.md#本地缓存清理与沙箱只读视图)。 | 复查路径不存在；`.cache/go-build` 未扩大清理。只做静态检查，未触发 Actions。 |
| 2026-10-26 | 慢速回退与稳定工具栏 | 共享慢速/断流检测及缓冲下载；Geo 整组事务安装不重连 VPN。规则双向边缘滚动，移动键盘与空结果不重建搜索框，见[稳定列表](ui-and-tests.md#移动虚拟列表重叠与宽屏空间)。 | `457ec52` 的 [CI 40304558635][ci-toolbars] 11 项、[不发布打包 40308495398][package-toolbars] 13 项通过；九组包/18 附件核验通过，Release 未变。真实吞吐和系统字号仍需设备验收。 |
| 2026-10-26 | 循环审查入口 | 将功能、UI、cor、跨端与 GUI/CLI 同步检查固定为[循环工作流](../loop-workflow.md)；新增功能、修复及公开发布前触发。 | 文档任务仅核对内容、链接及整洁，未触发编译或 Actions。 |
| 2026-10-25 | Defender 告警与规则/订阅循环 | 补[未压缩 EXE/DLL 扫描](desktop.md#9-windows-普通用户服务与安装器)；修复移动规则校验与拖动取消，失败不污染草稿。告警样本未经核验不能断言误报。 | `1d3e13e` 的 [CI 40210562854][ci-rules] 与[不发布打包 40212996727][package-rules]通过；Windows 两架构包及各五个 EXE 无威胁，用户样本仍待微软鉴定。 |
| 2026-10-25 | 订阅删除、Pad 与 fix.3 预发布 | [当前订阅删除与重复提交](ui-and-tests.md#订阅删除与重复提交)按事务处理；手机/Pad 工具导航、规则来源及完整原文同步，拖动按稳定 ID 修改完整草稿。 | [预发布 40283018496][release-fix3] 的 13 项任务、18 附件核验通过；保留 fix.2，未改稳定 latest；物理 Pad 未验收。 |
| 2026-10-25 | fix.2 预发布 | 按[来源与版本约束](release.md#10-发布产物版本与依赖快照)复用 `9623f45` 的成功 CI，使用 Android 长期发行签名。 | [CI 40176079496][ci-fix2] 与[预发布 40198947679][release-fix2]通过，九组包/18 附件核验通过，原 27 个 Release 未变。 |
| 2026-10-25 | 订阅更新失败与 Android 功能对齐 | [下载失败保留旧状态](ui-and-tests.md#订阅更新失败与界面状态)，成功才刷新报告。Fake-IP 来源/还原、规则表单、IP 详情/筛选、取消下载补齐；预发布安装提示数据回退风险。 | 本机仅静态检查与必要预览，回环错误/取消/恢复及原生授权回归交 CI；ARM 真机与用户现场未验收。能力现状见[Android](../../apps/android/README.md#当前能力)。 |
| 2026-10-24 | 分支版本、更新通道及移动布局 | 采用合法[分支 SemVer](../development.md#分支版本)；关闭预发布时高本地版本也以正式版为目标。列表重叠、Pad 操作栏与设置导航已修正。 | `279f653` 的 [CI 40110166410][ci-layout]成功；新版本须另取同提交验证。安装回退限制见[版本通道](../workflows/versions.md)，不能将其等同于 Android 可覆盖安装。 |

[ci-loop]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40610758771
[package-loop]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40613948109
[ci-fix6]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40562836032
[release-fix6]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40567590202
[ci-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40542749551
[package-tailscale]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40546066504
[ci-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40304558635
[package-toolbars]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40308495398
[ci-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40210562854
[package-rules]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40212996727
[release-fix3]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40283018496
[ci-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40176079496
[release-fix2]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40198947679
[ci-layout]: https://github.com/Fu021/Clash-of-Rust/actions/runs/40110166410
