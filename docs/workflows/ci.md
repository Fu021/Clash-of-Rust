# CI 与依赖缓存

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

## CI

先生成一次 `dependency-snapshot`，复制仓库的 Rust/Go 锁文件与 Android 声明，结合 `.github/build-versions.json` 中已确认的工具、内核和资源身份；不执行 `cargo update`、`go get` 或 Maven 自动升级，也不应用旧缓存中曾自动解析出的新版源码依赖。四份 Geo 原包在此时按固定摘要和大小下载、校验并随快照分发，各平台应用快照时再次校验，再准备各自的原生资源。这样不会因平台排队期间上游替换 `latest` 附件或私网临时不可用而重新下载 Geo；快照缺失或损坏仍失败，不改用其他内容。所有平台和 Publish Release 复用同一份快照，打包不再刷新安装工具版本。

依赖维护可手动运行现有 CI，设置 `refresh_dependencies=true`：只在 Actions 解析 Cargo/Go 更新并导出带来源提交和 SHA256 的 `dependency-updates`，不运行平台矩阵，也不作为打包依据。将已审阅的锁文件提交后，仍须取得新提交的普通 push CI 与不发布打包验证；受限本机不执行依赖下载或构建。

输入法专项设置 `android_ime_only=true`：只构建一次 Android x64 测试 APK，最多三个 runner 并发覆盖 API 26、28、29、30、31、32、33、34、35、36；`android_ime_apis` 默认 `all`，也接受 `30,32,34` 等已批准版本子集。补测选择尚未通过的版本，已完成的 job 不占 runner，也不因下一轮补测重复全量矩阵；常规 CI 保留 API 36 和 31 的回归。每个系统先三轮正式路径截图，再三轮严格回归；前两轮动画倍率为 1，第三轮为 0，不在测试进程中修改生产回调。字段切换、高度一致、Back、前后台与旋转的断言和验收边界见[输入法专页](../agent-failures/android-ime.md#当前修复与待验收)。

附件 `android-ime-api-{api}` 按 `production` / `regression` 及轮次保留截图、JSON、输入法/窗口 dumpsys 和本次日志；`device.json`、`input-method-status.json` 记录实际输入法及崩溃。缺少必需证据、输入法崩溃或日志边界丢失均失败。截图导出权限不足时，仅在确认是可调试模拟器后尝试 root adbd。截图探测成功不等于键盘绘制成功，须核对截图与严格回归。专项不覆盖 MIUI、API 27 或完整兼容验收，不作为发布依据、不导出发布编译附件、不运行桌面矩阵/缓存审计/打包，与依赖维护模式分别运行；常规 CI 保留严格输入法回归。

Android 在 Gradle setup 后重新激活固定 Python、JDK 和 Go，编译前执行 `build_inputs.py check-tools` 核对实际版本。环境变量里记录的版本或早先 setup 成功不能代替当前 PATH 上的工具检查；版本漂移先失败，不放宽 `go mod tidy -diff` 或自动下载另一套编译器。

共享 Go 下载在矩阵启动前执行 `go mod tidy -diff`，先拒绝未规范化的锁文件，避免三种 Android ABI 完成 Rust 编译后才发现同一问题。

CI 与 Memory 的 Rust 编译缓存仅使用 GitHub，未命中直接编译；Publish Release 复用同提交 CI 附件。原始下载优先官方外网，失败后才取私网，准备完成后尽早提交已验证的 URL 清单，末尾追加及查询持久回执。私网不再保存编译对象、安装目录、Gradle 转换缓存或已构建容器；当前逐项覆盖及未接入的原生包管理器见[缓存分工](cache-coverage.md)，服务端接口和[可保留白名单](cache-retention.md)独立维护。工作流不删除私网条目。

每轮 loop 执行只读依赖审计，CI 在最后的缓存 job 中输出 `dependency-audit` 附件及 summary，避免候选查询阻塞平台矩阵启动。报告覆盖 Cargo 锁定包、Go 模块、Android Maven 声明、Actions、已确认工具及 Linux/资源元数据，标出撤回、弃用和上游归档信号；查询失败与未检查项单列。按[loop 更新约定](../loop-workflow.md#每轮怎么做)修改声明、锁文件、工具基线及 Actions SHA 并验证。第三方 Actions 固定官方不可变提交，版本标签保存在工具基线中供审计，不随上游标签移动自动更新。

GitHub 缓存预算为 9 GiB，按[慢平台优先级](cache-coverage.md#编译对象只用-github)保留，取消运行不执行预算清理。macOS/Windows 先尝试包含源码身份的缓存，未恢复有效对象时再尝试同平台旧 GitHub workspace；精确键未命中不代表冷编译。复用时逐文件核对源码内容，相同内容才恢复旧时间戳，变化的 crate 及使用共享输入的构建脚本会失效重编，SVG 也参与源码身份。测试报告和安装包不进入编译缓存。

Go 审计只查询声明的模块路径，不执行更新。Android 公共任务一次准备已校验的模块下载包和绑定工具，经本轮附件分发给三个 ABI；Go 编译与 Gradle 转换在 runner 完成，不再镜像这些目录。已验证的 Go 原始 ZIP/MOD 单独登记来源，不能用本轮分发的 runner tar 代替原始下载。

私网服务使用独立访问令牌；配置仅写 runner 临时文件，不进入下载清单。CI 的输入解析和各平台 job 输出原包清单 artifact 与 summary，其他工作流的报告范围见[清单提交](cache-coverage.md#尽早提交完整清单收尾保留回执)。最终 `private-cache-inventory` 仅盘点，未删除旧缓存不代表新工作流仍在使用它们。

需要服务端配合的事项由用户转达，职责见[缓存服务边界](cache-server.md)。工作流不修改或上传 README.txt，不自动交换 agent 反馈；服务器文档由服务端 agent 维护。

原包私网回退与下载清单提交均有总预算，导入超时或失败不再回传归档。排队、就绪、未提交及实际恢复须分别验收，见[接口约定](cache-server.md#现有接口的原包用法)。

### Ubuntu 20.04 环境下载缓存

CI 和打包通过 `.github/actions/linux-download-cache` 准备独立 Python 与原始云镜像，冻结上游校验值。Linux CI 在宿主先准备并校验内核与 Geo 文件，再将工作区挂载给不含私网凭据的构建容器；容器再次核验已有资源。容器从固定基础 digest 在 runner 构建，不导出 Docker 镜像缓存；本 job 已构建的镜像可复用。详细类别及边界见[覆盖清单](cache-coverage.md)。

Python 和原始云镜像使用官方优先、原包私网回退，全部新导入共用 `upstream/files`；旧类别只作为服务端迁移候选。虚拟机下载目录只在本 job 使用，不回传其目录快照。

宿主、容器补充包和虚拟机 DEB 均以本轮签名 APT 索引核对包名、大小和 SHA256，缺失、损坏或弱校验时正常下载，不缓存应用 DEB、系统盘或测试结果。只向私网登记具备来源证据的原包，不镜像整个 DEB 目录。环境与 GUI/代理/TUN 检查照常进行，KVM 不可用时仍记录 TCG；缓存不能消除包配置、启动和软件模拟耗时。

### 测试覆盖

- 六种桌面平台使用相同的 Release profile 完成格式检查及桌面相关包的 Clippy、编译和源码单元测试；编译与测试明确排除 `clash-mobile-runtime` 和 `clash-android`，共享业务库仍按桌面依赖正常验证。Android 三个 ABI job 自行执行移动会话及请求门禁的宿主单元测试和 Clippy，再编译对应 Android target 的实际 Slint/Java/Go 宿主；宿主测试不等于 ARM 真机验收。iOS 不参与构建。
- Linux x64/ARM64 在同一轮冻结的 Ubuntu 20.04 镜像中构建；独立 Python 与镜像按上游校验值固定。逐个检查发行 ELF 与集成测试程序的依赖和 `GLIBC` 符号上限 2.31，将镜像、系统依赖及文件摘要写入产物。镜像共享宿主内核，旧内核及完整安装/GUI 验证另见[发布流程](release.md#publish-release)。
- 单元测试保留配置与规则、网络响应解析、认证信息脱敏、状态恢复、任务取消、列表复用与边界、实际指针交互等验证。`cor` 的命令解析、帮助和无显示环境入口也在源码测试内验证；历史截图和长期忽略的联网/平台手工回归入口已移除。
- 真实系统集成测试只运行系统代理与 TUN：验证代理设置写入、HTTP 转发、TUN 接管无显式代理的流量、模式恢复及关闭清理。Linux 使用隔离网络命名空间和 GSettings 内存后端；macOS 使用真实 launchd 网络助手；Windows 先注册服务，再以普通用户令牌运行代理/TUN、恢复和服务清理检查。
- Linux 隔离夹具还验证普通用户、sudo、指定用户与直接 root 的 CLI：临时账户/认证策略只在私有挂载命名空间存在，检查同名订阅隔离、root 只读总览、后台与文件 UID、常用配置/规则/导出操作、真实 TUN 及信号清理。帮助覆盖命令表；公网下载/安装、交互式认证代理、真实登录自启和公网检测不在此矩阵中。
- Linux TUN 测试在独立双栈上游中使用正式保护覆盖，验证域名节点的引导解析、经节点的 DoH A/AAAA、TCP/UDP DNS 接管、HTTP 转发、冷启动及原设置恢复；额外阻断固定公网引导 DNS 并延迟 IPv6 节点 AAAA，验证订阅解析器与慢解析仍可用。上游记录公网明文 DNS 与绕过节点的公网 DNS，发生泄漏即失败。保护 DNS 全部不可达时必须拒绝开启并恢复原网络。隔离环境使用专用测试 CA，保留 TLS 证书校验，不访问外网。
- macOS 代理/TUN 集成还在可丢弃 runner 中建立合成 VPN 补充解析器，检查本机系统域名请求、私有分域、搜索设置与 DNS 租约的启用和恢复，以及保护 DNS 上游超时时不向冲突的 VPN 解析器回退；夹具不登录 Tailscale，不替代真实 tailnet、出口节点、网络变化或端到端保护线路验证。
- Python 构建辅助与产物校验单元测试在前置依赖解析任务中执行一次，失败时不启动各平台下载与编译。内存基准 example 不作为 CI 测试程序额外构建或运行。
- main push、PR 更新触发 CI；过时运行自动取消。本地生成的编译、打包产物和缓存已在 `.gitignore` 中排除，不应提交。
- 桌面 CI 不生成安装包；桌面后台、界面程序、`cor` 和网络助手一起导出，现有代理/TUN 集成场景同时验证真实控制台命令及后台生命周期。Android CI 生成供安装测试与发布复用的 APK。产物记录提交、版本、架构、Cargo.lock、工具链及逐文件校验值；仅附带发布后复用的 `core_integration` 代理/TUN 测试程序，不上传源码单元测试程序。产物采用 schema 3，只与同一提交的打包流程配套使用。

Android x86_64 使用 API 36 模拟器运行真实代理/TUN 流量测试及上述订阅输入法回归，验证正式界面的内核状态、默认策略组、订阅导航和设置触摸滚动，以及系统 VPN 授权、DNS、套接字保护、快捷磁贴连接/断开、Activity 重建和路由恢复；另在 API 31 串行复验输入法。取消后到达的普通授权、过期服务启动与排队连接不得重新开启 VPN，连接中必须提供取消入口；ARM64/ARMv7 完成实际编译、Gradle lint 与架构校验，仍需真机验收。Android 测试结果、体积报告和 logcat 保存在 `android-report-{arch}`，固定开发签名与构建说明见 [Android 宿主](../../apps/android/README.md)。

常规测试、输入法专项及打包共用模拟器准备入口。当前[模拟器实现](https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android-qemu2-glue/main.cpp)会将首次启动的 API 24+ userdata 提高到至少 6 GiB，并要求分区大小的 1.2 倍空闲；因此配置显式使用 6144 MiB，再为测试留出 1 GiB，启动前合计要求约 8.2 GiB。此前 3072 MiB / 4 GiB 的预检遗漏了这一最低值，`1139dcc` 的 x64 仍在约 5255 MiB 空闲时被模拟器要求 7372.8 MiB 而拒绝，不能把配置写入或较宽裕 runner 的偶然成功当作修复。

仅在 GitHub 托管 Linux 临时 runner 空间不足时，移除 Android 测试不用的 `/usr/share/dotnet`，再核对实际空间；本地、自托管及其他系统不会执行此操作。`storage.json` 保留预算、处理前后空间和移除的工具路径；仍不足则在改动 AVD 前失败。编译缓存、SDK、仓库内容与私网数据不受该操作影响；空间不足不能省略代理/TUN 或输入法断言。

macOS 失败日志、系统与网络状态、崩溃报告保存在 `macos-ci-report-{arch}`，Windows 集成检查结果保存在 `windows-ci-report-{arch}`。人工授权窗口、真实重启登录和物理显示器行为仍需人工验收。

## 审核与缓存边界

外部 fork 每次运行须维护者批准，不绕过隔离、网络或 Secrets 限制；贡献者使用自己的工具链和缓存，不索取维护者私网凭据。附件保留 3 天，缓存沿用现有版本身份、校验与容量预算；新增工作流先经维护者同意，不擅自取消他人运行或删除 Actions 历史、附件与缓存。
