# 脚本维护

本目录只保留构建、验证、打包、缓存、内存基准和 UI 预览入口。应用运行时不依赖这些脚本；新增脚本前先检查能否扩展现有共享模块或入口，不要为单次排障复制包装脚本。

## 职责清单

| 脚本 | 类型 | 职责 |
| --- | --- | --- |
| `build_support.py` | 共享模块 | 平台识别、版本、资源、下载、架构和暂存目录等标准库辅助。 |
| `proxy_benchmark.py` | 测试夹具模块 | 为桌面代理集成测试提供本机 HTTP/可信 TLS 数据源和延迟端点，保存吞吐与合成节点测速报告；由代理/TUN 包装器按 CI 选取启动，不修改用户网络。 |
| `build_inputs.py` | CLI / 模块 | 解析并应用同一轮 Rust、Go、Android、Linux 基线和资源依赖快照，校验及分发四份 Geo 原包，按官方摘要安装 Gradle 并核对 PATH 版本。 |
| `android_apk.py` | 共享模块 | 最高 DEFLATE 级别压缩 APK 内核库与 Geo 资源，校验解压内容、对齐和原签名身份后原子替换；由 Android 构建入口在测试前调用。 |
| `dependency_audit.py` | 模块 | 只读探查升级、撤回和替换信号；由 `build_inputs.py audit` 调用，实际变更遵循维护者的 loop 更新授权并验证。 |
| `workflow_artifacts.py` | CLI / 模块 | 导出、就地暂存、下载和校验不可变 CI 产物；就地暂存绑定实际上传 id/digest，避免同 runner 再下载；同 CI 的 Linux 包就绪后启动兼容验证。 |
| `action_cache.py` | 共享模块 | GitHub 编译缓存兼容身份、工作区源码回执、不完整恢复清理和同次 CI 共享的 Go 源码归档。 |
| `test-cache-restore.cjs` | 离线测试 | 验证 CI 缓存恢复的未命中、临时故障、清理和重试上限。 |
| `workflow_storage.py` | CLI | main 每个目标只留最新缓存，清除其他 ref 缓存；`--dry-run` 预览删除计划。 |
| `prepare-resources.py` | CLI | 校验并准备 mihomo、Geo 和桌面共享资源。 |
| `prepare-assets.py` | 维护 CLI | 生成图标、CLDR 国家名称，或核验 Windows 嵌入图标；普通构建不运行，不生成国旗图片。 |
| `android.py` | CLI / 模块 | Android 编译、模拟器、产物导出与 APK 打包。[CI](../docs/workflows/ci.md#android-工具与原包)复用同一 Gradle 任务预热依赖及离线构建。`application-preview --arch x64 --api LEVEL --build DIRECTORY --compatibility` 复用 CI APK 验证基础代理/TUN，可加 `--screenshots`、`--verify-diagnostics`，见[兼容验证](../docs/workflows/android-compatibility.md)。不带 `--compatibility` 的本地入口仍只截图或诊断。 |
| `build-installer.py` | CLI | 只组装已验证产物为 Windows、Linux 或 macOS 安装包。 |
| `windows_update_tests.py` | 隔离安装测试模块 | 由 Windows 候选包验证按需调用；使用固定提交、固定摘要的旧安装器脚本和同一份 CI payload，对照文件占用、只读与 ACL 拒绝写入的旧/新退出码；另以独占端口和带身份标记的 HTTP 来源验证安装目录外独立 Mihomo 的流量、更新替换及外部进程/文件保留。首次启动等待有界的转发就绪并保留错误，正式更新前后拒绝错误响应，不重试掩盖失败。 |
| `macos_package.py` | 模块 | macOS App / DMG 组装；被打包器和 macOS 验证延迟导入。 |
| `publish-release.py` | CLI / 模块 | 核验成功 CI 候选包及安装收据，公开模式改取发行签名 Android 包；不重新打包桌面，按版本生成下载与下载量徽章。 |
| `run-ci.py` | CLI | 单个原生平台的编译、Clippy 和代理/TUN 集成程序导出；完整源码测试按 CI 策略选择，Windows 更新相关变更另执行服务与进程边界回归，macOS 代理相关变更包含 CF 转换及偏好数据回归。 |
| `ci_test_policy.py` | CLI / 模块 | 选择基础网络、相关变更专项和手动扩展回归，并向 CI 各平台传递选择结果。 |
| `test-cli-update.py` | 隔离安装测试 | 仅在 Actions 的已安装 Linux 环境运行，以临时普通账户验证只读查询、无桌面的 sudo 拒绝、真实 dpkg 失败恢复、成功重装后的自动启动/保留关闭状态及 systemd 服务状态/归属，保存逐项结果。 |
| `compatibility.py` | CLI / 模块 | 生成 Android 和 Linux 兼容矩阵，解析同提交成功 CI。`resolve --suite SUITE --run-id ID` 解析候选，`record` 绑定成功 job 及输入 id/name/digest。 |
| `linux_compatibility_targets.py` | 模块 | 固定 Linux 发行版版本、架构和官方镜像摘要。矩阵与实际 VM 共享同一目标定义。 |
| `linux_ci.py` | CLI / 模块 | Linux 基线容器、ELF 审计和 `verify` 兼容入口。兼容目标直接使用可丢弃的 Actions runner 或同架构 VM。 |
| `ubuntu_vm_environment.py` | 共享模块 | 冻结 Ubuntu 双架构的官方 APT 计划与签名索引，在隔离 VM 中校验及安装。 |
| `linux_apt.py` | 模块 | 限时安装系统依赖，按签名索引校验同次运行中已下载的 DEB。 |
| `linux_gui.py` | CLI / 模块 | Linux 安装后 X11/Wayland 窗口、托盘、共享后台与图形授权验证。 |
| `linux_desktop_login.py` | 兼容 VM 内 CLI | 实际 GDM、LightDM 或 COSMIC 登录、旧自启动迁移、托盘注册及窗口恢复；各发行版会话范围见工作流文档。 |
| `linux_mint_desktop.py` | Mint 兼容 VM 内 CLI / 模块 | 在 Cinnamon X11 登录内驱动图形授权，核验取消和允许后的安装文件权限。为原生会话复用已安装 GUI、托盘和共享后台回归。 |
| `linux_mint_install.py` / `pop_live_install.py` | Linux 兼容模块 | Mint、Zorin 的 Ubiquity 和 Pop!_OS 的 distinst 安装器，共用一次性磁盘和 SSH 引导合同。 |
| `linux_native_package.py` / `linux_native_guest.py` | Linux 兼容模块 | 由已验候选准备 RPM、Arch 包和收据；来宾内用发行版原生包管理器验证安装与生命周期。 |
| `linux_gnome_login.py` / `linux_cosmic_*.py` | Linux 兼容模块 | 从真实 Wayland 会话验证窗口、桌面授权或代理，限定于一次性来宾。 |
| `linux_vm_console.py` / `linux_vm_monitor.py` | Linux VM 模块 | 隔离串口、QMP 和虚拟显示截图，供 ISO 安装及 COSMIC 诊断复用。 |
| `windows_ci.py` | 模块 | Windows 普通用户、服务、含空格和中文的自定义目录安装与更新，以及安装包/未压缩 EXE/DLL 的 Defender 验证；扫描证据进入打包收据。 |
| `macos_ci.py` | CLI / 模块 | macOS 原生验证、安装检查和诊断报告。 |
| `macos_runner_network.py` | 共享模块 | 供 macOS 安装验收采集 runner 原生 DNS、路由与解析探针。 |
| `macos_test_network.py` | 共享模块 | 可丢弃 Mac runner 安装测试前的网络快照、测试助手取证/清理、失败恢复和附件端点检查；由安装验收及上传前检查调用，不用于用户网络。 |
| `test-proxy-modes.py` | CLI | Windows、Linux、macOS 共用的系统代理、HTTP 和 TUN 流量检查。 |
| `test-linux-tun.py` | root CLI | Linux 隔离网络/挂载命名空间内的用户身份、订阅隔离、TUN 授权、DNS 和恢复测试。 |
| `tun_dns_fixture.py` | 子进程夹具 | Linux TUN 测试的隔离上游；必须作为独立进程启动。 |
| `tun_ipv6_lifecycle.py` | 隔离测试模块 | 由 Linux TUN 测试调用，验证默认 IPv6、无 IPv6 的启动/保护、地址族增减与 TUN 重连；不单独操作开发主机网络。 |
| `test-release-package.py` | CLI | 统一调度安装包结构、安装后验证和 Linux 兼容性检查。 |
| `run-memory-benchmark.py` | CLI | 准备、分片测量和汇总 API、GUI、托盘及 CLI/TUI 内存基准。 |
| `cli_tui.py` | 测试夹具模块 | 真实 PTY 验证终端恢复、排序、导航、手机换行与缩放、连接详情、工具参数、结果滚动和导出；核对语言即时切换、跨客户端同步、显式覆盖、无后台保存及独立后台的模式、设置、订阅、规则和快捷规则操作，验证磁盘持久化与跨客户端冲突。内存驱动按实际导航识别页码，兼容历史版本及独立快捷规则页；夹具关闭自动更新，不接管宿主网络，由 CI 与内存基准调用。 |
| `cli_tui_preview.py` | 测试夹具模块 | 复用 PTY 驱动捕获桌面八页、子页面、字段输入态、候选值和全部有效管理命令表单，生成带逐页操作说明、来源与尺寸索引的完整验收 HTML；由既有 TUI 验证调用，不增加工作流或执行宿主授权/安装。 |
| `memory_desktop.py` | 模块 / 子进程 | 托盘宿主与窗口夹具；测量进程外运行，不能并入基准入口。 |
| `memory_lifecycle.py`、`memory_shards.py` | 模块 | 分析生命周期与分页结果，校验输入及分片完整性。 |
| `preview-slint.py` | CLI | 使用依赖版本一致的 Viewer 与 edition 1.19 检查和渲染正式组件，沿用随包字体；拒绝旧工具输出缺字截图，不调用 Cargo。 |
| `preview-page.py` | CLI | 单个 Markdown、HTML 或网页的 PNG 截图，网页默认生成整页长图。见[本地截图说明](../docs/workflows/local-development.md#markdown-与网页截图)。 |
| `test-build-tools.py` | 离线测试 | 构建输入、缓存、打包、Windows/macOS 辅助逻辑的单元测试。 |
| `test-workflow-artifacts.py` | 离线测试 | 不可变产物、安装包和内存工作流的离线回归测试。 |

## 仓库内其他脚本

`resources/claude-check/browser.js` 是随客户端提供的检测页面资源。

以下 Rust 构建脚本由各自的 Cargo 包调用，按包的构建生命周期保留；协议生成共用 `crates/desktop-protocol/schema.rs`，不另建包装入口：

| 路径 | 职责 |
| --- | --- |
| `apps/desktop/build.rs` | 注入应用版本；Windows 下编译图标和版本资源。 |
| `apps/desktop-ui/build.rs` | 从正式 Slint 状态生成窗口进程与共享协议之间的数据转换。 |
| `apps/android/native/build.rs` | 仅在 Android target 编译正式 Android Slint 入口并嵌入资源。 |
| `crates/client-ui/build.rs` | 编译桌面正式 Slint 入口。 |
| `crates/desktop-protocol/build.rs` | 从正式 Slint 状态生成序列化协议、默认值和状态访问接口。 |

## 合并与删除规则

- 新行为属于现有生命周期时，扩展已有共享模块或 CLI，不新增同义入口。
- 只有不同操作系统权限、独立进程边界、独立离线测试套件或清晰用户入口才保留独立脚本。
- `memory_desktop.py`、`tun_dns_fixture.py` 必须保持可独立启动；`macos_package.py`、`windows_ci.py` 保持可跨平台延迟导入，不能并入只适用于单一宿主的入口。
- 删除脚本前先用 `rg` 检查 `.github/`、文档、测试和生产脚本调用；同步删除引用、测试和缓存键影响。
- 共享路径、下载、版本、资源和暂存逻辑优先复用 `build_support.py`；排障逻辑进入现有平台模块或离线测试。
- 每次最终合并并推送 `main` 前，审查上述脚本及工作流内嵌命令，按职责和调用关系决定合并、删除或保留，并更新相关文档与失败复盘。

上表随调用关系维护。新增缓存或附件写入入口前核对 [GitHub Actions 缓存](../docs/workflows/cache-coverage.md)。
