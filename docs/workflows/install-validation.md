# CI 候选包与安装验收

[CI](ci.md) · [发布](release.md) · [预装测试环境](prepared-vm-images.md)

CI 负责发行构建、候选安装包及基本代理/TUN 验收，Publish Release 复用已通过的桌面包，只在公开模式重新签名和验证 Android。GUI、更新回滚等扩展回归按需运行；尚未取得新提交 Actions 结果时，不能把接线完成写成验收通过。

候选打包器及其工具随 CI 平台 job 准备：Windows 准备 NSIS，Linux 准备 DEB 工具，macOS 复用本机 DMG 工具，Android 准备 Gradle、JDK 与 SDK。发布的不发布模式只需要核验工具；公开模式的 Android job 只准备 JDK、签名/对齐工具和 x64 模拟器，不安装 Gradle 或重新构建 Rust/Go。Memory Benchmark 只编译所选桌面源码并采样，不生成候选安装包。各原包的实际私网覆盖以[下载路径表](cache-coverage.md#原始下载官方优先私网回退)为准，不能把 setup 成功视为私网可恢复。

## 安装包压缩

发行构建使用体积优化和符号剥离，按 [Cargo 配置语义](https://doc.rust-lang.org/cargo/reference/profiles.html#lto)以 `lto = "off"` 完全关闭 LTO；`false` 仍可能启用 crate 内部 ThinLTO，不用于此策略。包级 profile 覆盖会重置继承的 `strip`，因此所有带覆盖的发行包都显式设置 `strip = "symbols"`，Linux 导出还会拒绝残留 `.symtab`/`.strtab` 的 ELF。安装包压缩在打包阶段覆盖内核、Geo 和其余内容，不为提高压缩率改变运行时数据格式。Windows 使用 solid LZMA，x64 字典为 128 MiB，ARM64 为 64 MiB；这是已通过打包器初始化的边界，ARM64 更大字典曾失败，不再盲目提高。DEB 使用最高 xz 预设 9 和 extreme 策略。macOS 优先生成并验证历史两架构均最小的 ULMO，成功即复用，不再重复生成另外三种格式。ULMO 失败时才从同一份已签名 App 比较 UDBZ、ULFO 与 zlib 级别 9 的 UDZO，逐个验证后选最小；全部失败则终止，不覆盖旧包。公证及装订在最终选择之后执行。

Android 按 ABI 分包，在 CI 候选包阶段用 DEFLATE 级别 9 重压已压缩成员与四份 Geo 资源；原有不压缩的 DEX、资源表和原生库保持其方式，不破坏 mmap 和对齐约束。启动图标使用从源图生成的 432px Android 前景成品，避免把 1254px 源图原样存入每个 APK；逐项校验解压后长度与 SHA256，然后重新进行 16 KiB 对齐、同身份签名和签名验证，失败保留原包，结果不更小时仍使用原包。证书身份兼容 apksigner 的编号、SDK 区间以及 Build Tools 37 的 `Signer:`、`V3.0 Signer` 等输出，读取签名者的证书 SHA256 并去重比较；非法摘要直接拒绝，Source Stamp 和公钥摘要不作为安装签名。格式以实际批准 SDK 分发包为准，[AOSP ApkSignerTool](https://android.googlesource.com/platform/tools/apksig/+/refs/heads/main/src/apksigner/java/com/android/apksigner/ApkSignerTool.java)主线源码只作参考。体积变化写入 `android-report/<ABI>/build.json` 的 `compression`；此操作发生在网络验收前，发布只复用已验收内容并重新发行签名。

压缩不移除完整中文/emoji 字体、离线 Geo、许可证或诊断；不增加运行时自解压可执行层。字体覆盖、JNI 名称及 Android 的 16 KiB 对齐仍须保留。Java 混淆和资源裁剪未开启，避免在尚未验证 JNI/反射保留规则时影响宿主调用。发布不改写已签名 APK 的 ZIP 内容。

压缩收益以同提交九组候选包的实际字节和安装结果为准，同时记录编译及打包耗时。关闭 ThinLTO 并升级编译器后从空的 GitHub 缓存重建，旧包仅作历史对照，不能把工具链变化、源码优化和归档压缩的效果混为一谈。首轮冷编译不能与历史暖缓存直接比较，缓存占用与恢复、保存、重编耗时须分别核对，见[容量规划](cache-capacity.md)。安装包必须继续通过本页各平台安装、签名和基本网络检查，不能仅以压缩成功作为兼容证据。

`0.7.2-fix.1.rust100-cache` 的公开安装包（Android 为发行签名）合计 **531,643,212 字节**，0.7.1 为 **589,984,384 字节**，减少 **9.89%**。Linux 两架构约减 6.68～7.43%、Windows 约减 4.17～4.58%、macOS 约减 26.97～28.96%；三种 Android APK 仍比 0.7.1 大约 0.21～0.57%，不能宣称所有平台缩小。相较此前启用 ThinLTO 的压缩候选，九包合计增加约 **2.00%**；这是同时改变工具链、应用修复与 LTO 的实际取舍，不能当作纯压缩算法对照。九包、九摘要、安装收据和公开附件均已核对，来源提交与验收见[最近复盘](../agent-failures/reviews.md)。

后续符号剥离和 Android 图标调整在 `4bb7d8c` CI `43465685765` 完成验收，九组候选包合计 **521,476,651 字节**，比上述公开包再减约 **1.91%**：Linux 两架构减约 3.24～4.58%、macOS 减约 2.75～3.02%、Android 减约 1.20～1.28%；Windows 两包仅约 −0.003%／+0.012% 的波动，不能记为有效压缩收益。Android 此处是开发签名候选，签名差异也计入实际字节；同提交兼容与不发布核验证据见[最近复盘](../agent-failures/reviews.md)，不改写已公开版本的体积数据。

## 平台范围

| 平台 | 验证位置与检查 |
| --- | --- |
| Windows x64 / ARM64 | 生成并扫描 EXE；验证安装、服务注册、普通用户代理/TUN、卸载及用户文件保留。更新改动按需检查安装闸门及写入失败回滚；GUI 和服务异常退出归扩展测试。PE 图标、扫描版本和逐文件 SHA256 进入收据。 |
| macOS Intel / Apple Silicon | 生成 DMG；核对版本、架构、资源及 ad-hoc 签名，并验证安装后代理/TUN；窗口和界面重建归扩展测试。界面辅助包使用独立身份及 `LSUIElement`；没有 Developer ID 与 Apple 公证。 |
| Linux x64 / ARM64 | 同一份 DEB 先在 Ubuntu 20.04 构建容器安装，再在 Ubuntu 22.04 原生 runner 验证安装后的代理/TUN及授权；GUI 归扩展测试。 |
| Ubuntu latest x64 | 独立 CI job 等待本架构 DEB 与测试程序，验证同包安装、授权和代理/TUN；窗口及托盘归扩展测试。 |
| Ubuntu 20.04 x64 / ARM64 | 独立 [Ubuntu Compatibility](ubuntu-compatibility.md) 复用成功 CI，恢复／准备模板与本架构候选包获取并行；在 5.4 GA 内核 VM 安装并验证基础代理/TUN。记录 KVM/TCG、冷/暖准备、包下载、传输及应用测试耗时。 |
| Android ARM64 / ARMv7 / x64 | 三种 ABI 候选 APK 核对签名、架构和 16 KiB 对齐；x64 在 Android 16 模拟器执行真实代理/TUN。ARM 实机、键盘与截图专项不因此冒充已验收。 |
| Android 8.0～16 | 独立 [Android Compatibility](android-compatibility.md) 复用成功 CI 的 x64 APK，11 版本各验证基础代理/TUN；截图及诊断显式按需选择。 |

普通构建保留 Clippy，完整源码回归和单元测试在扩展模式执行。安装测试使用同一轮实际导出的程序，并沿用平台夹具对助手、服务、临时账户及授权的隔离和清理；安装路径的检查不能被源目录运行结果代替。Linux 基线容器共享宿主内核，不能替代 20.04 虚拟机的旧内核证据。

更新、CLI 维护或控制交接相关变更按需启用更新回归：六个桌面构建运行更新模块的 Rust 测试，Windows/macOS 另检查锁外准备与交接失效。Windows 候选安装器检查未提交、父进程未退出时不替换，取消保留旧文件，随后完成同包安装，并验证写入失败回滚。Ubuntu 22.04 的已安装候选包使用临时普通账户执行 `test-cli-update.py`，实际调用生产安装助手、非交互 sudo 和 dpkg，检查授权拒绝时不停止后台、损坏 DEB 安装失败后分别恢复后台或内核，以及同版本重装后保留设置并保存结果。安装测试使用本轮已校验的本地候选包，不连接线上 Release；发现、下载与摘要错误由源码回归覆盖。真实 UAC、交互密码、断电和半安装损坏仍不在自动验收范围。

Windows 写入复现由 [windows_update_tests.py](../../scripts/windows_update_tests.py) 执行八个场景：主程序、界面、CLI、内核、Geo 和卸载器被占用，Geo 只读，以及单文件 ACL 拒绝写入。旧/新安装器使用相同已校验 payload，旧脚本固定在明确提交及 SHA256；临时对照包关闭归档压缩以减少测试耗时，安装指令不改动，不发布或上传该包。核对实际退出码与旧提示的映射、当前阶段/文件/系统码、预检拒绝前文件逐字节不变，以及解除限制后成功安装。只读是独立对照：旧安装器可能主动清除该属性，不能假定它与文件占用返回相同错误。保留 `windows-update-report.json` 的逐场景结果与可用空间；中途失败也保留已取得证据。此矩阵不操作真实用户目录，不禁用 Defender，不把 ACL 拒绝等同于已复现第三方安全软件的具体行为。

NSIS 的磁盘路径使用反斜杠，诊断中的相对文件名使用正斜杠；两者在清单宏内转换。安装后另核对 `resources/ip-check` 的许可证与来源文件，防止 NSIS 的路径清理将目录名拼接，见 [NSIS 路径实现](https://github.com/kichik/nsis/blob/master/Source/exehead/util.c)。

Ubuntu latest 在 CI 内仅等待本架构包；Ubuntu 20.04 则在成功 CI 后独立运行。两者的候选包与测试程序都须属于所选 CI、完整提交及正确架构，重复、过期、摘要不符或生产 job 失败均终止。只有候选包实际通过原生安装验证才上传 `package-linux-*`，旧内核与多版本 Android 的兼容结果须另外成功。

Ubuntu 18.04、真实硬件与人工授权仍不在自动矩阵内。临时 guest 只跳过无关文档索引，GUI 依赖仍随环境安装；窗口断言在扩展模式执行，见[测试环境边界](../agent-failures/test-environments.md#11-测试稳定性与外部环境假设)。

## 附件与证据

- 六组桌面 `build-*` 和三组 Android `build-*` 保存不可变编译输入；桌面候选包、APK、SHA256 和 `package-receipt.json` 各组成一份 `package-*`，合计九份。
- CI 部分重跑后，GitHub 可能同时保留旧、新尝试的同名附件。按 attempt 合并各平台最后一次实际执行的 job，只接收创建时间位于该生产 job 执行区间、且 run/commit 一致的附件；未重跑的平台沿用自己的最近成功 job。同一区间仍重复、缺失、过期或时间身份不完整时拒绝，不按最大 ID 猜测，不删除旧记录。发布、兼容输入和候选包获取共用此筛选，安装收据仍须精确绑定本次构建 artifact 的 ID 与摘要。
- 收据绑定完整提交、版本、CI run、平台/架构、包 SHA256，以及上传后的构建 artifact id/name/digest；只有该包的安装验证成功后才写 `tests_passed=true`。Ubuntu 兼容 job 还必须单独成功，不能以原生收据代替。
- Android 收据用 `distribution_signed=false` 标识开发签名候选包。公开发布只接受签名后重新验证并记录 `distribution_signed=true` 的 APK；测试包不能混入正式附件。
- 最终发布核对 CI 全部必需 job、两套兼容性回执、附件 digest、内部完整成员、构建身份和包摘要。不发布模式读取九份 CI 候选包；公开模式读取六份 CI 桌面包和本次发布的三份发行签名 APK。
- `macos-ci-report-*`、`linux-package-report-*` 与 `android-report-*` 保留对应运行的证据和失败诊断，Windows 扩展窗口与更新专项报告按需保存；常规 Windows 代理/TUN 结果见 job 日志。包及源码附件保留 3 天。证据过期则重新运行 CI，不悄悄重建另一份包。
- Ubuntu 20.04 的 `focal-guest-report-status.json` 区分 guest 报告已保存、未生成及传输失败；只有完整传输成功才保留归档，安装和网络测试结果仍以实际任务及收据为准。

## 资源与权限

桌面打包复用本次编译 runner 的工具和本地已核验归档，不重复编译、不再申请第二个 macOS runner。Ubuntu VM 准备与候选包下载并行；模板、基础盘和写入层只存私网或 runner 临时目录，禁止进入 GitHub Cache、artifact 或 Release。原包清单与回执按[缓存分工](cache-coverage.md)保存，不删除私网数据。

CI 不取得发行签名或 Release 写权限；fork 不接入维护者私网，不增加 `pull_request_target`。只有 Publish Release 的最终 job 取得公开发布权限。源码或构建输入改变仍须验证新提交；无代码变化的 runner/网络故障按[重试规则](../workflows.md#actions-跟进节奏)处理，不能通过忽略断言或减少矩阵取得成功。
