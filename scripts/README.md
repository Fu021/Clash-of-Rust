# 脚本维护

本目录只保留构建、验证、打包、缓存、内存基准和 UI 预览入口。应用运行时不依赖这些脚本；新增脚本前先检查能否扩展现有共享模块或入口，不要为单次排障复制包装脚本。

维护者本次授权的 `cosmic-startup-investigation.yml` 临时调用 `investigate-cosmic-startup.py` 与 VM 内的 `cosmic_startup_guest.py`，使用官方 Pop!_OS live 镜像复现 COSMIC 登录、菜单链接迁移和手动启动；复用同提交成功 CI 安装包，不访问私网缓存、不传递凭据。验证结束后删除这三个临时入口，通用结论保留在[静默自启动与托盘告警](../docs/agent-failures/desktop.md#静默自启动与托盘告警)。

## 职责清单

| 脚本 | 类型 | 职责 |
| --- | --- | --- |
| `build_support.py` | 共享模块 | 平台识别、版本、资源、下载、架构和暂存目录等标准库辅助。 |
| `proxy_benchmark.py` | 测试夹具模块 | 为桌面代理集成测试提供本机 HTTP/可信 TLS 数据源和延迟端点，保存吞吐与合成节点测速报告；由代理/TUN 包装器按 CI 选取启动，不修改用户网络。 |
| `build_inputs.py` | CLI / 模块 | 冻结并应用同一份 Rust、Go、Android、Linux 基线和资源依赖快照；统一取得四份 Geo 原包，随快照校验及分发；按冻结身份下载、校验和安装 Gradle，支持原包私网回退及实际 PATH 版本检查。 |
| `upstream_cache.py` | CLI / 模块 | 官方原始下载的私网回退、持久清单分批提交、分页回执与有界状态查询；不上传 runner 归档。 |
| `dependency_audit.py` | 模块 | 只读探查升级、撤回和替换信号；由 `build_inputs.py audit` 调用，实际变更遵循维护者的 loop 更新授权并验证。 |
| `runtime_cache.py` | 历史协议辅助 | 旧工具快照的离线兼容回归；当前工作流不调用。 |
| `workflow_artifacts.py` | CLI / 模块 | 导出、就地暂存、下载和校验不可变 CI 产物；就地暂存绑定实际上传 id/digest，避免同 runner 再下载；同 CI 的 Linux 包就绪后启动兼容验证。 |
| `action_cache.py` | CLI / 模块 | 认证私网缓存协议、恢复预算、校验及清单。 |
| `cache_shards.py` | 历史协议辅助 | 旧 Rust 分片的离线兼容回归；当前工作流不调用。 |
| `workflow_storage.py` | CLI | main 每个目标只留最新缓存，清除其他 ref 缓存；`--dry-run` 预览删除计划。 |
| `prepare-resources.py` | CLI | 校验并准备 mihomo、Geo 和桌面共享资源。 |
| `prepare-assets.py` | 维护 CLI | 生成图标、CLDR 国家名称，或核验 Windows 嵌入图标；普通构建不运行，不生成国旗图片。 |
| `android.py` | CLI | Android 编译、模拟器、产物导出与 APK 打包；普通 CI 按变更选择专项。`application-preview --arch x64 --api LEVEL --build DIRECTORY --compatibility` 复用 CI APK 验证基础代理/TUN；可加 `--screenshots`、`--verify-diagnostics`，见[兼容验证](../docs/workflows/android-compatibility.md)。不带 `--compatibility` 的本地入口仍只截图或诊断。 |
| `build-installer.py` | CLI | 只组装已验证产物为 Windows、Linux 或 macOS 安装包。 |
| `macos_package.py` | 模块 | macOS App / DMG 组装；被打包器和 macOS 验证延迟导入。 |
| `publish-release.py` | CLI / 模块 | 核验成功 CI 候选包及安装收据，公开模式改取发行签名 Android 包；不重新打包桌面，按版本生成下载与下载量徽章。 |
| `run-ci.py` | CLI | 单个原生平台的编译、Clippy 和代理/TUN 集成程序导出；完整源码测试按 CI 策略选择。 |
| `ci_test_policy.py` | CLI / 模块 | 选择基础网络、相关变更专项和手动扩展回归，并向 CI 各平台传递选择结果。 |
| `test-cli-update.py` | 隔离安装测试 | 仅在 Actions 的已安装 Linux 环境运行，以临时普通账户验证无桌面的 sudo 拒绝、真实 dpkg 失败恢复和候选 DEB 重装，保存逐项结果。 |
| `compatibility.py` | CLI / 模块 | 共用 Android／Ubuntu 兼容矩阵、成功 CI 来源解析与发布回执；`resolve --suite android/ubuntu --run-id ID`，`record` 绑定实际成功 job 及输入 id/name/digest；发布前核验完整回执。替代 `android_previews.py`。 |
| `linux_ci.py` | CLI / 模块 | Linux 基线容器、ELF 审计和旧内核虚拟机；容器及 VM 必须运行在可丢弃 Actions 环境。 |
| `prepared_vm.py` | 共享模块 | 私网专用 VM 协议、严格元数据、能力门禁、有界完整下载及流式上传；不写 GitHub 缓存、不发 DELETE。 |
| `prepared_vm_environment.py` | 共享模块 | 冻结隔离 APT 环境、独立配方、guest 包清单和模板清理；由输入解析和 Linux VM 入口调用，不在开发服务器准备环境。 |
| `linux_apt.py` | 模块 | 限时安装系统依赖；按本轮 APT 签名索引校验虚拟机 DEB 下载缓存，失败回退正常下载。 |
| `linux_gui.py` | CLI / 模块 | Linux 安装后 X11/Wayland 窗口、托盘、共享后台与图形授权验证。 |
| `windows_ci.py` | 模块 | Windows 普通用户、服务、安装器、更新及安装包/未压缩 EXE/DLL 的 Defender 验证，扫描证据进入打包收据。 |
| `macos_ci.py` | CLI / 模块 | macOS 原生验证、安装检查和诊断报告。 |
| `test-proxy-modes.py` | CLI | Windows、Linux、macOS 共用的系统代理、HTTP 和 TUN 流量检查。 |
| `test-linux-tun.py` | root CLI | Linux 隔离网络/挂载命名空间内的用户身份、订阅隔离、TUN 授权、DNS 和恢复测试。 |
| `tun_dns_fixture.py` | 子进程夹具 | Linux TUN 测试的隔离上游；必须作为独立进程启动。 |
| `tun_ipv6_lifecycle.py` | 隔离测试模块 | 由 Linux TUN 测试调用，验证默认 IPv6、无 IPv6 的启动/保护、地址族增减与 TUN 重连；不单独操作开发主机网络。 |
| `test-release-package.py` | CLI | 统一调度安装包结构、安装后验证和 Linux 兼容性检查。 |
| `run-memory-benchmark.py` | CLI | 隔离运行 API、GUI、托盘和 CLI 内存基准。 |
| `memory_desktop.py` | 模块 / 子进程 | 托盘宿主与窗口夹具；测量进程外运行，不能并入基准入口。 |
| `preview-slint.py` | CLI | 使用依赖版本一致的 Viewer 检查语法，主线 Viewer 配合随包字体渲染正式组件；拒绝旧工具输出缺字截图，不调用 Cargo。 |
| `test-build-tools.py` | 离线测试 | 构建输入、缓存、打包、Windows/macOS 辅助逻辑的单元测试。 |
| `test-workflow-artifacts.py` | 离线测试 | 不可变产物、安装包和内存工作流的离线回归测试。 |

## 仓库内其他脚本

`.github/actions/upstream-cache/index.cjs` 只准备私网配置和有界收尾；`upstream-report` 支持早期检查点和末尾清单/回执附件，工作流共用持久队列客户端。旧 `local-cache`/`runtime-cache` action 已删除，历史协议辅助不作为服务端部署入口。`resources/claude-check/browser.js` 是检测页面资源，随客户端提供，保留在资源目录。

私网缓存的客户端写入能力和服务端职责见[缓存服务说明](../docs/workflows/cache-server.md)。工作流不修改或上传 `.github/cache-server/README.txt`，不传递 agent 协作请求或读取 agent 反馈；需要服务端配合的事项由用户转达。

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

上表随调用关系维护，不用历史脚本数量判断当前入口是否完整。历史协议辅助须有离线兼容回归的实际调用；恢复生产入口或新增上传方式仍须核对[当前缓存分工](../docs/workflows/cache-coverage.md)。
