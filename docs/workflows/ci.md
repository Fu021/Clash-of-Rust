# CI 与依赖缓存

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

## CI

网站变化由独立的 `Website validation` job 验证，不阻塞客户端输入解析和编译启动；网站或 CI 工作流未变化时只做范围判定。静态构建、链接、双语与截图来源、下载快照及浏览器/多尺寸长图均在 runner 验证，覆盖首页缩放、中文默认/英文切换、独立的 GUI/TUI/CLI 手册、手机菜单、自动轮播及无脚本回退。资源受限开发机只做轻量静态检查与必要组件配图。浏览器禁用外网并在只读权限下运行，不保存额外浏览器缓存；范围、运行预算及附件见[网站维护](../../website/README.md#每次发布都同步)。

CLI 源码变化会选择 Linux 用户专项，并运行完整 `cor` 单元测试（参数、输出、只读目录、交互筛选和服务状态），避免只跑后台启动测试而遗漏新增入口。TUI 复用更新命令，其源码变化也选择更新专项。更新相关源码或 `console` 集成测试变化时，Linux x64 还运行完整命令行集成测试，检查系统语言、旧设置与安装记录。终端更新专项在安装后的临时普通账户中验证无数据目录的只读查询、停止内核的查询、systemd 的启用/运行/PID/归属，以及安装成功和失败后的恢复；不修改开发主机的服务或网络。

真实 PTY 驱动 `scripts/cli_tui.py` 或其回归变化时，同样选择 Linux 用户专项，实际运行设置保存/拒绝、跨客户端刷新与终端恢复；不能只执行 Python 夹具单测就导出候选包。根依赖声明/锁文件、桌面宿主或 shell 的依赖声明变化时选择桌面 UI 专项，避免只提交升级后的锁文件而漏掉渲染和原生窗口回归。

Windows 平台路径变化沿用更新专项，并在两种原生架构上运行 `platform::windows_service::` 单元回归，覆盖服务协议、快照路径及令牌/系统目录查询的成功和拒绝；不把只编译发行程序当作这些回归已执行。服务安装、普通用户身份和真实代理/TUN 仍由候选包验收覆盖。

共享检测与网络工具源码变化也会选择 Linux x64 共享业务和界面专项，包含工具参数、出口、TLS、UDP、进程取消及迟到结果回归，并实际运行 CLI 参数与 TUI 测试，不能只因本轮没有修改 CLI 文件就遗漏新增工具的命令入口。该平台一旦选择源码专项，Clippy 同时检查测试目标；不能只检查发行入口、漏掉尚未编译的测试代码。修补提交仍须按实际触及路径选择回归，完整可选集合另用 `extended_tests=true` 验证。

先生成一次 `dependency-snapshot`，复制仓库的 Rust/Go 锁文件与 Android 声明，结合 `.github/build-versions.json` 中已确认的工具与内核身份；不执行 `cargo update`、`go get` 或 Maven 自动升级，也不应用旧缓存中曾自动解析出的新版源码依赖。四份 Geo 原包每轮从官方 `latest` 解析附件身份、大小和 SHA256，下载并校验后随该轮快照分发；解析后附件被上游替换而返回 404 时重新解析并有限重试。各平台应用时再次校验，Publish Release 复用同一轮快照；缺失、损坏或摘要不符仍失败。不同 CI 轮次可使用不同 Geo 数据，同一次构建及发布不混用。

依赖维护可手动运行现有 CI，设置 `refresh_dependencies=true`：只在 Actions 解析 Cargo/Go 更新并导出带来源提交和 SHA256 的 `dependency-updates`，不运行平台矩阵，也不作为打包依据。将已审阅的锁文件提交后，须取得新提交的普通 push CI，并按[实际改动范围](../development.md#按改动选择验证)选择兼容、内存或发行核验；受限本机不执行依赖下载或构建。

常规 CI 的运行时集成验证聚焦系统代理/TUN：真实授权、代理转发、TUN 实际流量、DNS 和断开后的路由恢复。各平台仍编译发行程序、执行 Clippy 和校验候选安装包，但不再构建和运行全部 Rust 测试程序；Python 构建与产物契约检查保留一次。三个 Android job 保留宿主请求门禁及会话退出的小型回归，完整移动源码回归在应用分流专项或扩展模式执行。Android 应用分流或其测试入口发生变化时自动追加分流专项；Linux 多用户/TUN 相关入口发生变化时追加隔离用户专项。更新、CLI 维护或控制交接相关路径变化时，六种桌面启用更新模块回归，Windows/macOS 验证锁外交接；平台安装回归见[候选包验收](install-validation.md#平台范围)。桌面界面或窗口宿主发生变化时，Linux x64 追加桌面组件和窗口回归；移动界面及共享组件变化时，Android x64 在自己的 job 执行移动软件窗口回归。共享组件变化可选择两端，但分别编译与测试；这些测试不替代真实窗口和真机验收。变更范围无法判定时保守运行扩展测试，选取结果写入 job summary。

共享连接详情、控制器与 DNS 解析代码变化时，同样选择 Linux CLI、桌面界面和 Android 移动界面的相关回归，各端仍在各自作业构建。

设置、保护判定或控制接口变化时，Linux x64 专项覆盖共享设置白名单和后台控制门禁；共享设置、保护规则或移动会话变化时，Android x64 分别执行移动界面、会话与分流回归。移动强调色真实点击覆盖保护关闭、切换中和已开启；后台门禁在对应任务中执行。

需要完整源码、窗口、应用分流、多用户及其他既有回归时，可手动运行 CI 并设置 `extended_tests=true`；各专项测试入口保留，不删除断言。常规 CI 不覆盖完整 UI、存储、单元测试或全部生命周期，绿色结果不能当作这些专项也已通过。键盘、键盘高度、Back 编辑和多版本截图继续使用单独手动入口，不随常规 CI、loop 或打包自动重跑。

### 测试精简后的耗时

2026-11-09，提交 `35d2d7e` 的 CI `42596299301` 共 14 项全部通过，总耗时 22 分 17 秒；未启用完整扩展回归，Android 分流和 Linux 多用户专项因本分支包含相关变更而执行。下面对比上一轮 `42587460890` 的 job 实际执行时间，不含等待分配 runner：

| Job | 上一轮 | 本轮 |
| --- | --- | --- |
| macOS x64 | 41.0 分钟 | 13.2 分钟 |
| macOS ARM64 | 15.7 分钟 | 12.2 分钟 |
| Windows x64 | 36.6 分钟 | 9.9 分钟 |
| Windows ARM64 | 37.6 分钟 | 13.2 分钟 |
| Ubuntu 20.04 ARM64 | 31.2 分钟 | 19.9 分钟 |

macOS x64 上一轮构建全部测试程序耗时 20 分 32 秒，本轮只准备网络集成测试程序耗时约 16 秒；两轮范围和缓存状态均有变化，这不是同覆盖范围的性能基准。当时 ARM64 内核验证使用 TCG，是关键路径。完整单元、GUI、更新回滚及其他生命周期未在本轮运行。

工作流已移除输入法专项的参数、APK 中转和多版本矩阵。保留 `scripts/android.py test --arch x64 --ime-only` 等手工调查脚本与既有证据，只有维护者明确要求重新调查时才使用，不因版本升级或 loop 自动恢复到 CI；缺图、日志边界丢失和输入法崩溃仍使手工专项失败。历史十版本范围、证据要求及 MIUI/实机边界见[输入法专页](../agent-failures/android-ime.md#当前修复与待验收)。

### 发布前独立兼容验证

常规 CI 包含 13 个必需 job。Ubuntu 多版本验证由 [Ubuntu Compatibility](ubuntu-compatibility.md) 承担。CI 成功后，[Android](android-compatibility.md)、[Ubuntu](ubuntu-compatibility.md) 及 [Debian](debian-compatibility.md) 的工作流可并行复用同提交、同源候选包。[发布兼容门禁](release.md#兼容性发布门禁)规定所需回执。

### 桌面代理回环测速

手动 CI 的 `proxy_benchmark=true`，或下载传输、平台系统代理和测速夹具相关改动，会在六种桌面 runner 的安装后代理检查中追加测速；普通 UI 改动不会单独启用。复用同一个真实 mihomo 进程和已经启用的系统代理，依次比较直接回环、从系统设置读回的 HTTP 代理、mixed 入口的 SOCKS5。代理地址显式传给共享客户端路线，避免系统对 localhost 的绕过规则把代理样本变成直连，并校验实际连接的端口。

每条路线分别访问本机 HTTP 和 HTTPS；HTTPS 经过 HTTP 代理时建立 CONNECT 隧道，TLS 只信任本次临时生成的测试证书，校验未关闭。每个样本 16 MiB，三轮轮换路线顺序；分别只接收并计算 SHA256，以及使用与安装包相同的 256 KiB 缓冲写临时文件并 flush/sync。另对三条 HTTPS 路线分别重复十轮，每轮复用同一客户端，先取 29 份 64 KiB 文件，最后取 16 MiB 文件；逐份核对长度和 SHA256，记录最后大文件速度，检查连接池及连续下载而非只测新连接。报告保留 36 份普通样本、25 份测速及 30 份连续下载序列，缺项不能算完成。总预算 180 秒，速度不设跨平台硬阈值，连接失败、缺报告及数据不完整仍失败。Mac 同时执行子会话就绪、提前退出、错误回执和卡住后的清理回归。

另用设定等待 25 ms 的本机 HEAD 端点比较五轮：直接请求、系统 HTTP 代理、SOCKS、mihomo 控制接口的 DIRECT，以及通过本机 mixed 入口转发的合成 SOCKS 节点。客户端样本额外保留端点实测等待时间，区分服务端等待与代理开销；不能假定 `sleep(25 ms)` 在所有 runner 上恰好用时 25 ms。控制接口分别记录请求总耗时和内核返回的测速值；默认 `unified-delay` 在第二次请求成功时采用复用连接的耗时，而其他路线记录新建连接后的响应耗时，两者不能直接等同。这些测量与用户订阅节点的延迟是不同的测试。

结果在 job summary 和 `proxy-benchmark-{system}-{arch}` 附件中，含 JSON 和 Markdown；失败时保留已经完成的样本。Linux 夹具在现有隔离网络命名空间内启动，不向 VM 兼容 job 重复追加。这些样本限于 IPv4、HTTP/1.1，不经过公网 DNS、远端节点或 GitHub CDN，也不覆盖 HTTP/2、QUIC 和 IPv6，不能证明相同订阅节点在用户 Windows/macOS 上的公网表现；先看各 runner 内的代理/直接回环比值，再结合故障设备的出站链和同目标下载对照。runner CPU、虚拟化、磁盘及安全软件差异不能只用绝对 MiB/s 归因于操作系统。

### Android 工具与原包

Android 使用固定 Python、JDK 和 Go；`build_inputs.py prepare-gradle` 按冻结快照中的版本及已提交的官方 SHA256 下载并校验官方 Gradle ZIP，解压后加入 PATH。编译前执行 `build_inputs.py check-tools --tools go java gradle` 核对实际版本。环境变量里记录的版本或早先 setup 成功不能代替当前 PATH 上的工具检查；版本漂移先失败，不放宽 `go mod tidy -diff` 或自动下载另一套编译器。Android 编译脚本将同一冻结 NDK 版本传给 Gradle，避免原生库编译和 APK 符号剥离各自选择工具链；`RUNNER_DEBUG=1` 时保留 Gradle 详细诊断及弃用栈。

Linux x64 的固定 Android Emulator 37.2.13 使用 [Google 官方归档](https://developer.android.com/studio/emulator_archive)及其 SHA256，在当次 runner 的临时目录安装并核对模拟器版本。SDK CLI 仍按固定修订安装平台工具和系统镜像，启动前核对镜像内的实际修订。

共享 Go 下载在矩阵启动前执行 `go mod tidy -diff`，先拒绝未规范化的锁文件，避免三种 Android ABI 完成 Rust 编译后才发现同一问题。

`Prepare Android Maven dependencies` 按固定 runner、工具基线、Gradle 配置和依赖声明恢复精确缓存。未命中时，单个 runner 使用实际 Go AAR 和应用资源在线执行 `assembleRelease`、`assembleReleaseAndroidTest` 和 `lintRelease`，只复制 `caches/modules-2` 的构件与解析元数据，再从独立 Gradle home 清理并离线重跑这三个任务。此作业只产出 Maven 依赖，安装候选仍由三种 ABI 各自完整构建和验收。

准备作业通过同轮 `android-maven-dependencies` 附件向三个 ABI 提供相同的依赖，三者都用 `--offline` 执行正式 Gradle 任务。仅 `main` 保存跨运行缓存，每次失效后重新在线准备。Gradle 转换及构建产物在各自 runner 生成。调整预热任务覆盖范围时更新缓存键的方案版本。

提交 `8a176d3c` 的 [冷缓存 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/44534000149)确认缺失键时成功在线准备并离线重跑，[暖缓存 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/44538353609)确认命中精确键后跳过在线准备。两次均为 13/13，三个 ABI 的 Release APK、测试 APK 和 Lint 均离线通过。暖 CI 读取约 214 MiB 压缩缓存。

CI 与 Memory 的编译基线保存在 [GitHub Actions 缓存](cache-coverage.md)，未命中时从公开上游准备输入并编译。Publish Release 使用同提交的 CI 附件。

Ubuntu 20.04 构建容器使用官方 HTTPS APT 源，首次安装证书包前临时使用 runner 的系统 CA；安装后删除引导证书与专用配置，继续使用容器自己的证书包。下载连接/读取限时 30 秒、最多重试两次，保留 APT 签名及 TLS 校验和既有容器构建总预算。下载超过重试预算时报错。

每轮 loop 执行只读依赖审计，CI 在最后的缓存 job 中输出 `dependency-audit` 附件及 summary，避免候选查询阻塞平台矩阵启动。报告覆盖 Cargo 锁定包、Go 模块、Android Maven 声明、Actions、已确认工具及 Linux/资源元数据，标出撤回、弃用和上游归档信号；查询失败与未检查项单列。按[loop 更新约定](../loop-workflow.md#每轮怎么做)修改声明、锁文件、工具基线及 Actions SHA 并验证。第三方 Actions 固定官方不可变提交，版本标签保存在工具基线中供审计，不随上游标签移动自动更新。

缓存容量、逐平台保留范围、优先级和费用统一见[缓存分工](cache-coverage.md#编译对象只用-github)，不在各工作流重复列数值。main 的 Linux、macOS 和 Windows x64 保存 workspace 并尝试兼容恢复；Windows ARM64 的完整基线保留依赖缓存，避免每次业务源码变化都重压大归档。开发分支只恢复兼容的 main 基线，不保存完整对象或差异包。复用时逐文件核对源码内容，相同内容才恢复旧时间戳，变化的 crate 及使用共享输入的构建脚本会失效重编，SVG 也参与源码身份。Linux 容器挂载同一源码回执；测试报告和安装包不进入编译缓存。

Go 审计只查询声明的模块路径，不执行更新。Android 公共任务一次准备已校验的模块下载包和绑定工具，经本轮附件分发给三个 ABI；各 ABI 的 `GOCACHE` 只存 GitHub。

### Ubuntu 20.04 编译与验证环境

Linux CI 从固定 digest 的 Ubuntu 容器编译。宿主先下载并校验内核与 Geo 文件，挂载后由容器复核。编译容器按架构及镜像 digest 使用 GitHub Actions 缓存，未命中时从官方镜像构建。

DEB 安装验收前，编译容器与 Linux 兼容 runner 共用签名 APT 索引刷新。刷新失败后分别等待 5 秒和 15 秒再试，每次刷新单独限时，三次仍失败则终止验收。

共享输入阶段为 Ubuntu 20.04 x64 来宾准备官方云镜像、独立 Python 和冻结的签名 APT 索引。镜像及工具归档按 CI 快照的 SHA256 校验。[Ubuntu Compatibility](ubuntu-compatibility.md) 启动该来宾，用冻结 APT 输入准备测试工具，必要时记录回退并使用当前签名索引。

### 测试覆盖

修改前后的入口、状态、坐标及进程生命周期审查见[测试契约](test-contracts.md)。推送前核对受影响的全部测试调用者，不以预览或上一提交通过结果代替当前验收。

- 六种桌面平台使用 Release profile 完成格式检查、桌面相关包的 Clippy 和发行构建，常规只额外编译 `core_integration` 供代理/TUN 检查；扩展模式构建并运行全部桌面源码测试。桌面范围排除移动宿主及移动测试，Android 三个 ABI job 自行执行移动相关 Clippy、Android 宿主单元测试及实际 Slint/Java/Go 构建；移动 UI 被选中时仅 Android x64 执行移动软件窗口测试，扩展模式追加共享移动运行时的源码测试。移动测试包含共享 iOS 原型组件，iOS 原生宿主未构建。
- Linux x64/ARM64 在同一轮冻结的 Ubuntu 20.04 镜像中构建；独立 Python 与镜像按上游校验值固定。逐个检查发行 ELF 与集成测试程序的依赖和 `GLIBC` 符号上限 2.31，将镜像、系统依赖及文件摘要写入产物；基线安装检查 `pkexec` 与终端代理均可用。容器共享宿主内核，旧内核的安装及代理/TUN 验证由独立 Ubuntu Compatibility 完成，见[安装验收](install-validation.md)。
- 单元测试仍保留配置、网络、认证脱敏、状态恢复、列表和界面交互等回归，完整集合在扩展模式执行。界面及保护相关路径选择前述 Linux x64 专项，同时核对共享模型、窗口桥接及控制 API／概览探测源码测试；控制分发另验证保护期间外观保存不启动内核、测速不隐式启动、混合网络提交整笔拒绝，不增加完整桌面测试矩阵。Rust panic 保留调用栈，Linux 基线构建容器也传递 `RUST_BACKTRACE=1`；扩展测试使用 `--no-fail-fast`，失败不能进入候选包生成。Slint Viewer 预览不能代替生成 Rust 后的布局回归。
- 代理页专项覆盖首次快照缺组后的自动定位、手动操作优先、直连、全部组/八组分页、组内独立分页、保护期间切换和大量组的虚拟滚动。窗口状态模块验证宫格/列表独立保存、旧格式默认值、无原生位置恢复及并发落盘。精确筛选的桌面测试在离线驱动中核对目标存在，防止只编译新增回归而未执行或改名后零项运行。
- 更新或共享配置路径变化时，Linux x64 另执行共享更新调度、配置保存与 CLI／GUI 更新设置回归；覆盖 0 关闭、旧配置默认值、手动检查、非法输入及失败保留草稿。仍复用现有源码单元测试，不增加模拟器或临时工作流。
- 真实系统集成测试只运行系统代理与 TUN：验证代理设置写入、HTTP 转发、TUN 接管无显式代理的流量、模式恢复及关闭清理。Linux 使用隔离网络命名空间和 GSettings 内存后端；macOS 使用真实 launchd 网络助手；Windows 先注册服务，再以普通用户令牌运行代理/TUN、恢复和服务清理检查。
- Linux 多用户专项在相关路径变化或扩展模式下验证普通用户、sudo、指定用户与直接 root 的 CLI；临时账户和认证策略只在私有挂载命名空间存在。Linux x64 还执行终端认证代理的启动、就绪、失败和退出清理及重启前授权源码回归；实际密码交互、真实登录自启和公网检测不在此矩阵中。
- Linux TUN 测试在独立双栈上游中使用正式保护覆盖，验证域名节点的引导解析、经节点的 DoH A/AAAA、TCP/UDP DNS 接管、HTTP 转发、冷启动及原设置恢复；额外阻断固定公网引导 DNS 并延迟 IPv6 节点 AAAA，验证订阅解析器与慢解析仍可用。上游记录公网明文 DNS 与绕过节点的公网 DNS，发生泄漏即失败。保护 DNS 全部不可达时必须拒绝开启并恢复原网络。隔离环境使用专用测试 CA，保留 TLS 证书校验，不访问外网。
- 相关 Linux 用户/TUN 专项另在同一隔离上游移除和恢复上联 IPv6，验证默认偏好下的启动、重启、IPv4-only Claude 保护、网络地址族增减与 TUN 重连；核对真实 HTTP、当前 UID 路由和设置未被改写，见[IPv6 回归](../ipv6-tun.md#回归与验收边界)。不在开发主机执行，相关源码、Python 夹具、Rust 基础网络/保护 DNS 用例及选择规则本身的变化均自动选入现有专项。
- macOS 代理/TUN 集成还在可丢弃 runner 中建立合成 VPN 补充解析器，检查本机系统域名请求、私有分域、搜索设置与 DNS 租约的启用和恢复，以及保护 DNS 上游超时时不向冲突的 VPN 解析器回退；真实 Tailscale 分域和出口节点仍需设备验收。
- macOS 安装测试无论成功或失败都恢复测试前网络设置，上传报告前另执行有界端点检查。助手清理、原生 DNS/HTTPS 及实际 Actions 主机结果同时进入日志和报告；恢复失败不放行，原测试失败不因恢复成功而改为通过。边界与取证见[runner 网络恢复](../agent-failures/macos-dns.md#安装测试后的网络与附件)。
- Python 构建辅助与产物校验单元测试在前置依赖解析任务中执行一次，失败时不启动各平台下载与编译。内存基准 example 不作为 CI 测试程序额外构建或运行。
- Linux 原生 runner 的基础代理/TUN 验证准备网络检查所需工具和应用 DEB 的运行依赖。完整图形桌面和授权代理由扩展验收准备。
- main push、PR 更新触发 CI；过时运行自动取消。本地生成的编译、打包产物和缓存已在 `.gitignore` 中排除，不应提交。
- 桌面后台、界面程序、`cor` 和网络助手一起导出，并在同一 runner 生成安装包，执行[安装验收](install-validation.md)。构建附件上传后用实际 id/digest 和本地归档校验暂存，不再次下载，也不额外申请 macOS runner。Android 在候选 APK 上执行代理/TUN 集成，不对同一开发签名包重复测试。九组候选包、SHA256、安装收据与源码附件保留 3 天，全部必需 job 成功才能发布。产物记录提交、版本、架构、Cargo.lock、工具链及逐文件校验值；仅附带必要的 `core_integration` 测试程序，不上传源码单元测试程序。桌面构建产物采用 schema 3，只与同一提交配套使用。

Android x86_64 使用 Android 16（API 36）模拟器验证真实 VPN 授权、代理请求、TUN 流量、DNS 和断开恢复。[应用分流](../android-application-routing.md#范围与验证)相关变化或扩展模式会追加三种分流模式；完整授权取消、Activity 重建与冷磁贴归扩展模式。选中移动界面回归时，x64 作业安装软件渲染测试所需的字体开发库，并启用 `mobile-ui-tests` 执行宿主界面 Clippy 和测试。三个作业分别对各自 Android 目标执行 Clippy 和编译。ARM64/ARMv7 完成 Gradle lint 与架构校验，仍需真机验收。Android 测试结果、体积报告和 logcat 保存在 `android-report-{arch}`，固定开发签名与构建说明见 [Android 宿主](../../apps/android/README.md)。

Android x64 候选包收集常规报告后运行保护恢复的两阶段回归。普通 `test` 及 `package --arch x64` 的常规测试阶段使用当前 ADB 目标，可向该目标安装 APK 并修改 VPN 授权。CI 为这些操作预先启动可丢弃的模拟器。本地单独运行进程死亡回归时，用同一个 `RUNNER_TEMP` 启动可丢弃的 API 36 `cor-ci` AVD 和执行 `python3 scripts/android.py test --arch x64 --protection-death-only`。

Android x64 候选包另执行[APK 安装授权回归](test-contracts.md#入口与职责)。

进程死亡夹具先锁定报告目录，持有到清理结束且最终回执写完。争锁失败的运行直接退出，保留当前运行的回执和报告。取得锁后先将 `process-death-status.json` 的 `passed` 设为 `false`，再独占 `127.0.0.1:18081`。源站启动失败时将错误写入本轮回执和独立的 `origin-failure-*.json`，保留已有日志和诊断报告，也不接触设备。取得源站后才清除上次夹具报告。安装 APK、清除应用数据、修改授权、启动应用、写入日志标记、执行插桩及每项清理前均重新核对固定 AVD 路径、模拟器序列号、API、启动标识和启动计数。脚本拒绝经过符号链接的 AVD 路径。校验与后续 ADB 命令各自发出，同一序列号在两者之间被替换时仍有竞态。

第一阶段使用虚构本地配置建立 TUN，写入模拟的中断记录，再向应用自身进程发送 SIGKILL。两阶段均用 `am instrument --no-restart`。第二阶段在新 PID 中核对 Android 退出历史里的 SIGKILL，检查冷启动保持暂停，再点击重试并验证原配置和保存的 VPN 计划恢复。通过本机测试源、直连负对照和控制器连接记录验证恢复后的 TUN 实际转发。阶段成功或失败后先采证，再有界清理应用数据与授权。身份复核失败时停止后续清理并保留已执行的命令结果。清理失败附在原始错误后。采证、设备清理及源站退出均成功后才写入成功回执。`target/android-report/x64/protection-process-death/` 保留安装和插桩输出、回执、清理结果和日志。夹具覆盖中断记录落盘之后的进程死亡与显式恢复，保护启用事务的真实中断点、保护 DNS 和厂商系统行为仍需独立验证。

常规测试、输入法专项及打包共用模拟器准备入口。当前[模拟器实现](https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android-qemu2-glue/main.cpp)会将首次启动的 API 24+ userdata 提高到至少 6 GiB，并要求分区大小的 1.2 倍空闲；因此配置显式使用 6144 MiB，再为测试留出 1 GiB，启动前合计要求约 8.2 GiB。此前 3072 MiB / 4 GiB 的预检遗漏了这一最低值，`1139dcc` 的 x64 仍在约 5255 MiB 空闲时被模拟器要求 7372.8 MiB 而拒绝，不能把配置写入或较宽裕 runner 的偶然成功当作修复。

仅在 GitHub 托管 Linux 临时 runner 空间不足时，移除 Android 测试不用的 `/usr/share/dotnet`，再核对实际空间；本地、自托管及其他系统不会执行此操作。`storage.json` 保留预算、处理前后空间和移除的工具路径；仍不足则在改动 AVD 前失败。编译缓存、SDK 和仓库内容不受该操作影响；空间不足不能省略本次已选择的代理/TUN 或手动专项断言。

macOS 失败日志、系统与网络状态、崩溃报告保存在 `macos-ci-report-{arch}`；桌面 UI 变更或扩展模式执行首次显示及五次窗口重建，验证实际安装包的原生窗口与托盘，不能仅靠编译判断菜单依赖兼容。Windows 普通用户 GUI 和服务崩溃恢复属扩展模式，更新闸门及写入回滚按更新路径变化启用；相关窗口报告在 `windows-ci-report-{arch}`，安装回归见 job 日志。人工授权窗口、真实重启登录和物理显示器行为仍需人工验收。

## 审核与缓存边界

测速控制器、共享诊断和 Android 宿主相关改动由 `android_diagnostics_tests` 选择真实 APK 的测速／诊断导出回归，不在每次文档或无关改动时重跑。场景、故障夹具及多版本专项边界见[Android 测速与诊断专项](android-compatibility.md#测速与诊断专项)。

外部 fork 每次运行须维护者批准，不绕过隔离、网络或 Secrets 限制；贡献者使用自己的工具链和缓存。附件保留 3 天，缓存沿用现有版本身份、校验与容量预算；新增工作流先经维护者同意，不擅自取消他人运行或删除 Actions 历史、附件与缓存。
