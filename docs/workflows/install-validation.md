# CI 候选包与安装验收

[CI](ci.md) · [发布](release.md) · [预装测试环境](prepared-vm-images.md)

CI 负责源码与候选安装包的验收，Publish Release 复用已通过的桌面包，只在公开模式重新签名和验证 Android。迁移不缩减原有安装、GUI 或代理/TUN 断言；尚未取得新提交 Actions 结果时，不能把接线完成写成验收通过。

候选打包器及其工具随 CI 平台 job 准备：Windows 准备 NSIS，Linux 准备 DEB 工具，macOS 复用本机 DMG 工具，Android 准备 Gradle、JDK 与 SDK。发布的不发布模式只需要核验工具；公开模式的 Android job 只准备 JDK、签名/对齐工具和 x64 模拟器，不安装 Gradle 或重新构建 Rust/Go。Memory Benchmark 只编译所选桌面源码并采样，不生成候选安装包。各原包的实际私网覆盖以[下载路径表](cache-coverage.md#原始下载官方优先私网回退)为准，不能把 setup 成功视为私网可恢复。

## 平台范围

| 平台 | 同轮 CI 内执行的检查 |
| --- | --- |
| Windows x64 / ARM64 | 生成 EXE；更新 Defender 库并扫描安装包及未压缩 EXE/DLL；验证安装、服务注册、普通用户代理/TUN、更新路径、失败恢复、卸载与用户文件保留。普通身份 GUI 核对窗口尺寸、GUI/CLI 共享状态及最小化/恢复；写入边界见[更新与存储](versions.md#写入目录与权限边界)。PE 图标、扫描版本和逐文件 SHA256 进入收据。 |
| macOS Intel / Apple Silicon | 生成 DMG；核对 App/DMG 版本、架构、资源及 ad-hoc 签名；安装后 GUI/内核、代理/TUN、连续五次界面重建、应用注册、Dock 最近应用与内核 PID。界面辅助包使用独立身份及 `LSUIElement`；没有 Developer ID 与 Apple 公证。 |
| Linux x64 / ARM64 | 同一份 DEB 先在 Ubuntu 20.04 构建容器安装并快速检查 X11，再在 Ubuntu 22.04 原生 runner 验证安装后的 GUI、代理/TUN及授权；不为不同发行版重新打包。 |
| Ubuntu latest x64 | 独立 CI job 等待本架构 DEB 与测试程序，验证同包的 X11/Wayland、托盘、窗口重建、GUI/CLI 共用内核、授权及代理/TUN。 |
| Ubuntu 20.04 x64 / ARM64 | 两个 CI job 在共享输入就绪后先恢复或准备模板，仅等待本架构候选包；在 5.4 GA 内核虚拟机执行安装及现有 GUI/代理/TUN 检查。记录 KVM/TCG、冷/暖准备、包等待与应用测试耗时。 |
| Android ARM64 / ARMv7 / x64 | 三种 ABI 候选 APK 核对签名、架构和 16 KiB 对齐；x64 在 Android 16 模拟器执行真实代理/TUN及生命周期测试。ARM 实机、键盘与截图专项不因此冒充已验收。 |

普通构建的源码回归、Clippy 与单元测试继续保留。安装测试使用同一轮实际导出的程序，并沿用平台夹具对助手、服务、临时账户及授权的隔离和清理；安装路径的检查不能被源目录运行结果代替。Linux 基线容器共享宿主内核，不能替代 20.04 虚拟机的旧内核证据。

Ubuntu 兼容 job 不等待整个桌面矩阵；候选包与测试程序必须来自当前 CI、当前完整提交及正确架构，重复、过期、摘要不符或生产 job 失败均终止等待。环境先准备，等待预算最多 60 分钟；只有安装包实际通过原生安装验证才上传 `package-linux-*`。

Ubuntu 18.04、真实硬件与人工授权仍不在自动矩阵内。临时 guest 只跳过无关文档索引，GUI 依赖与功能断言照常执行，见[测试环境边界](../agent-failures/test-environments.md#11-测试稳定性与外部环境假设)。

## 附件与证据

- 六组桌面 `build-*` 和三组 Android `build-*` 保存不可变编译输入；桌面候选包、APK、SHA256 和 `package-receipt.json` 各组成一份 `package-*`，合计九份。
- 收据绑定完整提交、版本、CI run、平台/架构、包 SHA256，以及上传后的构建 artifact id/name/digest；只有该包的安装验证成功后才写 `tests_passed=true`。Ubuntu 兼容 job 还必须单独成功，不能以原生收据代替。
- Android 收据用 `distribution_signed=false` 标识开发签名候选包。公开发布只接受签名后重新验证并记录 `distribution_signed=true` 的 APK；测试包不能混入正式附件。
- 最终发布核对 CI 全部必需 job、原附件外层 digest、内部完整成员、构建身份和包摘要。不发布模式读取九份 CI 候选包；公开模式读取六份 CI 桌面包和本次发布的三份发行签名 APK。
- `macos-ci-report-*`、`windows-ci-report-*`、`linux-package-report-*` 与 `android-report-*` 保留成功证据和失败诊断；包及源码附件保留 3 天。证据过期则重新运行 CI，不悄悄重建另一份包。

## 资源与权限

桌面打包复用本次编译 runner 的工具和本地已核验归档，不重复编译、不再申请第二个 macOS runner。Ubuntu VM 与构建并行准备；预装模板、基础盘和写入层仍只存私网或 runner 临时目录，禁止进入 GitHub Cache、artifact 或 Release。原包清单与回执按[缓存分工](cache-coverage.md)保存，不删除私网数据。

CI 不取得发行签名或 Release 写权限；fork 不接入维护者私网，不增加 `pull_request_target`。只有 Publish Release 的最终 job 取得公开发布权限。源码或构建输入改变仍须验证新提交；无代码变化的 runner/网络故障按[重试规则](../workflows.md#actions-跟进节奏)处理，不能通过忽略断言或减少矩阵取得成功。
