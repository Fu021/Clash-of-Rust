# CI 候选包与安装验收

[CI](ci.md) · [发布](release.md) · [预装测试环境](prepared-vm-images.md)

CI 负责发行构建、候选安装包及基本代理/TUN 验收，Publish Release 复用已通过的桌面包，只在公开模式重新签名和验证 Android。GUI、更新回滚等扩展回归按需运行；尚未取得新提交 Actions 结果时，不能把接线完成写成验收通过。

候选打包器及其工具随 CI 平台 job 准备：Windows 准备 NSIS，Linux 准备 DEB 工具，macOS 复用本机 DMG 工具，Android 准备 Gradle、JDK 与 SDK。发布的不发布模式只需要核验工具；公开模式的 Android job 只准备 JDK、签名/对齐工具和 x64 模拟器，不安装 Gradle 或重新构建 Rust/Go。Memory Benchmark 只编译所选桌面源码并采样，不生成候选安装包。各原包的实际私网覆盖以[下载路径表](cache-coverage.md#原始下载官方优先私网回退)为准，不能把 setup 成功视为私网可恢复。

## 平台范围

| 平台 | 同轮 CI 内执行的检查 |
| --- | --- |
| Windows x64 / ARM64 | 生成并扫描 EXE；验证安装、服务注册、普通用户代理/TUN、卸载及用户文件保留。GUI、写入、更新失败恢复及服务异常退出归扩展测试。PE 图标、扫描版本和逐文件 SHA256 进入收据。 |
| macOS Intel / Apple Silicon | 生成 DMG；核对版本、架构、资源及 ad-hoc 签名，并验证安装后代理/TUN；窗口和界面重建归扩展测试。界面辅助包使用独立身份及 `LSUIElement`；没有 Developer ID 与 Apple 公证。 |
| Linux x64 / ARM64 | 同一份 DEB 先在 Ubuntu 20.04 构建容器安装，再在 Ubuntu 22.04 原生 runner 验证安装后的代理/TUN及授权；GUI 归扩展测试。 |
| Ubuntu latest x64 | 独立 CI job 等待本架构 DEB 与测试程序，验证同包安装、授权和代理/TUN；窗口及托盘归扩展测试。 |
| Ubuntu 20.04 x64 / ARM64 | 两个 CI job 在共享输入就绪后先恢复或准备模板，仅等待本架构候选包；在 5.4 GA 内核虚拟机执行安装及代理/TUN 检查，GUI 归扩展测试。记录 KVM/TCG、冷/暖准备、包等待与应用测试耗时。 |
| Android ARM64 / ARMv7 / x64 | 三种 ABI 候选 APK 核对签名、架构和 16 KiB 对齐；x64 在 Android 16 模拟器执行真实代理/TUN。ARM 实机、键盘与截图专项不因此冒充已验收。 |

普通构建保留 Clippy，完整源码回归和单元测试在扩展模式执行。安装测试使用同一轮实际导出的程序，并沿用平台夹具对助手、服务、临时账户及授权的隔离和清理；安装路径的检查不能被源目录运行结果代替。Linux 基线容器共享宿主内核，不能替代 20.04 虚拟机的旧内核证据。

更新、CLI 维护或控制交接相关变更会按需启用 Linux 更新回归：两个原生构建运行更新模块的 Rust 测试，Ubuntu 22.04 的已安装候选包使用临时普通账户执行 `test-cli-update.py`。实际调用生产安装助手、非交互 sudo 和 dpkg，检查授权拒绝时不停止后台、损坏 DEB 安装失败后分别恢复后台或内核，以及同版本候选 DEB 重装后保留设置并保存成功结果。该安装阶段测试使用本轮已校验的本地候选包，不连接线上 Release；更新发现、下载与摘要错误由更新模块回归覆盖。交互密码输入、断电和半安装损坏仍不在自动验收范围。

Ubuntu 兼容 job 不等待整个桌面矩阵；候选包与测试程序必须来自当前 CI、当前完整提交及正确架构，重复、过期、摘要不符或生产 job 失败均终止等待。环境先准备，等待预算最多 60 分钟；只有安装包实际通过原生安装验证才上传 `package-linux-*`。

Ubuntu 18.04、真实硬件与人工授权仍不在自动矩阵内。临时 guest 只跳过无关文档索引，GUI 依赖仍随环境安装；窗口断言在扩展模式执行，见[测试环境边界](../agent-failures/test-environments.md#11-测试稳定性与外部环境假设)。

## 附件与证据

- 六组桌面 `build-*` 和三组 Android `build-*` 保存不可变编译输入；桌面候选包、APK、SHA256 和 `package-receipt.json` 各组成一份 `package-*`，合计九份。
- 收据绑定完整提交、版本、CI run、平台/架构、包 SHA256，以及上传后的构建 artifact id/name/digest；只有该包的安装验证成功后才写 `tests_passed=true`。Ubuntu 兼容 job 还必须单独成功，不能以原生收据代替。
- Android 收据用 `distribution_signed=false` 标识开发签名候选包。公开发布只接受签名后重新验证并记录 `distribution_signed=true` 的 APK；测试包不能混入正式附件。
- 最终发布核对 CI 全部必需 job、原附件外层 digest、内部完整成员、构建身份和包摘要。不发布模式读取九份 CI 候选包；公开模式读取六份 CI 桌面包和本次发布的三份发行签名 APK。
- `macos-ci-report-*`、`linux-package-report-*` 与 `android-report-*` 保留对应运行的证据和失败诊断，Windows 扩展窗口报告按需保存；常规 Windows 代理/TUN 结果见 job 日志。包及源码附件保留 3 天。证据过期则重新运行 CI，不悄悄重建另一份包。

## 资源与权限

桌面打包复用本次编译 runner 的工具和本地已核验归档，不重复编译、不再申请第二个 macOS runner。Ubuntu VM 与构建并行准备；预装模板、基础盘和写入层仍只存私网或 runner 临时目录，禁止进入 GitHub Cache、artifact 或 Release。原包清单与回执按[缓存分工](cache-coverage.md)保存，不删除私网数据。

CI 不取得发行签名或 Release 写权限；fork 不接入维护者私网，不增加 `pull_request_target`。只有 Publish Release 的最终 job 取得公开发布权限。源码或构建输入改变仍须验证新提交；无代码变化的 runner/网络故障按[重试规则](../workflows.md#actions-跟进节奏)处理，不能通过忽略断言或减少矩阵取得成功。
