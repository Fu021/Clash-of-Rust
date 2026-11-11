# Android 兼容性验证

[工作流索引](../workflows.md) · [CI](ci.md) · [发布门禁](release.md#兼容性发布门禁) · [应用分流](../android-application-routing.md)

`Android Compatibility` 替代原 `Android UI Previews`。默认复用成功 CI 的 `build-android-x64` 与 `dependency-snapshot`，在 Android 8.0～16 的 11 个可丢弃模拟器中各验证基础代理/TUN；不重新编译、不默认运行全页截图、键盘、分流或诊断专项。发布前必须完成本工作流及独立 [Ubuntu Compatibility](ubuntu-compatibility.md)，不能用 Android 16 的普通 CI 结果代替旧系统验收。

## 启动与来源

在 Actions 手动运行 `.github/workflows/android-compatibility.yml`，选择目标提交所在分支，填写该提交成功 CI 的 `run_id`。不自动随 push 运行，不保留旧 PR 标签入口，不使用 `pull_request_target`。

`scripts/compatibility.py` 核对 CI 路径、成功终态、同仓库、完整提交，以及输入附件的唯一性、有效期、来源 run/commit 和 digest。矩阵固定 checkout 该提交、应用同一冻结依赖，校验内部构建清单、版本、锁文件、APK 摘要与实际设备版本。不能拿另一提交或重新编译的 APK 替代。

所有版本成功后，`Record Android compatibility` 生成 `android-compatibility-attempt-<attempt>` 附件，保留三天，内容包含源 CI、提交、输入 artifact id/name/digest、兼容运行及 attempt、`system_proxy`/`tun` 检查范围。发布前置解析和最终核验都检查该回执及所有必需 job；失败、跳过、过期、摘要不符或更换 CI 输入均阻止发布。新 push 后必须验证新提交。

## 默认验证与资源

| Android | API |
| --- | --- |
| 8.0 | 26 |
| 8.1 | 27 |
| 9 | 28 |
| 10 | 29 |
| 11 | 30 |
| 12 | 31 |
| 12L | 32 |
| 13 | 33 |
| 14 | 34 |
| 15 | 35 |
| 16 | 36 |

默认每版仅执行 `basicSystemProxyAndTun`：真实首次 VPN 授权、代理请求、TUN 流量、DNS 及关闭恢复。失败保留 instrumentation、设备信息、scoped logcat 和 `runtime-status.json`；应用崩溃、Rust panic、递归及日志边界缺失仍失败。SystemUI 崩溃单独报告 warning，不能当作模拟器系统完全稳定。

矩阵最多六个 job 并行、每个最多 45 分钟，失败不取消其他版本。并发上限不保证 GitHub 一定同时分配六个 runner，仍须分别记录排队和执行时间。只准备 Python、Java、SDK、ADB 和模拟器，不准备 Cargo、Go、Gradle。Android 8.1 使用固定摘要的官方 AOSP `default/x86_64` 镜像，其余使用 Google APIs x64 镜像；不额外编译 x86 APK。

原包官方优先、私网回退，尽早交持久下载清单。模拟器系统镜像原包仍存私网；AVD 安装目录、运行中快照与测试写入层不保存到 GitHub Cache，不上传为私网模板，不删除私网内容。

## 测速与诊断专项

手动选择 `verify_diagnostics=true` 后，在基础代理/TUN 外增加 `measurementDiagnosticsDistinguishFailuresAndExport`，不重复基础测试。使用隔离 HTTP 源站及故意拒绝连接的端口，验证一个成功和一个失败节点；临时改变隔离内核认证密钥，确认准备阶段错误不误报节点失联，再恢复复测。

VPN 前后导出生产诊断报告，经生产文件写入回调写入模拟器目录并逐字节核对。每版保留四份报告及 `result.json`，检查新鲜度、四轮上限、成功／节点失败／控制失败分类、旧系统字段边界及脱敏。文件 URI 模拟保存位置，不代表系统文件选择器或厂商 ROM 已验收。

历史提交 `d033b60` 的专项 `43030588512` 为 11 版本通过、22 项 instrumentation、44 份 JSON，应用及 SystemUI 崩溃为零；不替代拆分后的新工作流验收。

## 截图与边界

按需选择 `capture_screenshots=true`，在基础检查后采集生产 Slint 页面及真实应用列表。可与诊断选项组合，但截图本身不作为代理/TUN 成功证据，也不重跑键盘专项。

截图覆盖首页、代理／订阅宫格与列表、编辑、工具页、规则、设置及应用分流，包含手机、横屏、平板竖横屏。三种分流模式往返切换并核对补集、列表收起、图标和保存摘要，但截图过程不保存分流；实际分流效果仍以普通 CI 按源码变更选择的专项为准。

清单 schema 3 保存 PixelCopy 窗口 PNG 与原始 `*-screen.png`，不裁切、缩放或重绘；校验逻辑尺寸、缩放因子、窗口原点、滚动偏移与图像摘要，黑图、缺图、只剩本应用及滚动无效均失败。布局切换等待窗口焦点、Slint live 和真实尺寸一致，手机与平板方向按顺序固定；只关闭非模态通知，阻塞错误仍失败。

证据读取失败时，只有确认是支持调试的模拟器才允许重启其 adbd 为 root 重取，不能用于真机。历史 `42876368177` 在 11 版本取得 627 对图，抽查手机／平板分流及设置未见此前裁切；Android 12 的一次模拟器 SystemUI 导航栏异常单独保留。这些历史结果不是新提交验证。

各版 `android-<版本>-compatibility` 报告保留七天。默认只有网络证据，选了专项才要求对应报告或截图；不得因默认没有图片误报失败。真机、厂商 ROM、ARM 设备、长时间 VPN 和公网波动仍需独立验收，iOS 仅 UI 原型。

## 应用出口专项

维护者选择 `verify_application_rules=true` 时，在基础代理/TUN 后执行应用范围与指定出口回归。与诊断同时启用时，先执行包含基础代理/TUN 的诊断组，再执行应用规则；可选截图始终最后执行。复用已经完成的 VPN 授权，报告另存 `x64/application-rules/`，不覆盖基础网络证据。Android 8/9 核对实际能力、页面不支持状态及范围分流；Android 10+ 另外检查指定节点、DIRECT、REJECT、Fake-IP 原始元组、缺失目标、UDP 拒绝和不同 UID。

该选项默认关闭，仅在相关功能验收时使用。它不等于验证所有厂商 ROM 或每一种应用进程模型；UI、持久化及内核选路分别保留源码与设备回归。

不同 UID 的流量由 `app_process` 从测试 APK 启动独立 Java 探针，核对实际 UID、源站正文和新增规则命中。`UiAutomation.executeShellCommand` 不解释 shell 管道，不能把带 `printf | nc` 的字符串当作已发出的 HTTP 请求；探针仅存在于测试 APK，不进入发行应用。
