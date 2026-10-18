# GitHub Actions 工作流

| 工作流 | 触发 | 职责 | 产物 |
| --- | --- | --- | --- |
| CI | main push、PR | 六种桌面平台与三种 Android ABI 编译、检查、源码单元测试、代理与 TUN 集成测试 | `build-{system}-{arch}`，保留 3 天 |
| Publish Release | 手动 | 复用成功 CI 的程序，打包、安装后验证；可仅打包 | 六个桌面安装包、三个 APK 及 SHA256，测试产物保留 3 天 |
| Memory Benchmark | 手动 | 对指定代码运行低强度或压力内存测试 | 原始数据、报告与构建日志，保留 3 天 |

六个平台为 Windows x64 / ARM64、Linux x64 / ARM64、macOS Intel / Apple Silicon。Linux 安装包支持 Debian/Ubuntu，Ubuntu 仅支持 22.04 及以上；Ubuntu 最新版兼容验证在发布工作流中执行。

贡献者可使用本地工具链编译、Clippy、源码单元测试与打包，也可使用 Actions；无需为了本地验证而推送代码。正式发布继续复用成功 CI 的产物，真实系统代理/TUN 自动化测试在可丢弃的 Actions 环境中执行。界面可使用 `scripts/preview-slint.py` 直接预览正式 Slint 组件，不依赖桌面程序编译；弃用工作流删除文件后，也需在 GitHub 上禁用。

新增工作流须事先获得维护者同意，具体要求见[贡献约定](../.github/CONTRIBUTING.md)。`.github/` 的变更合并前须经 @Fu021 审核；所有外部贡献者的 fork PR 工作流每次触发均需批准，既往贡献不豁免。

## 本地打包

桌面打包须在与目标系统、架构一致的机器上进行。除 README 中的原生编译依赖外，还需 Go（版本满足 `apps/android/core/go.mod`，供共享依赖快照脚本使用）；Linux 打包需 `dpkg-dev`，macOS 需系统 `hdiutil`、`codesign`，Windows 的 NSIS 由脚本下载并校验。

在仓库根目录执行以下命令。依赖快照脚本会查询兼容更新，可能修改 `Cargo.lock`、Android Go 锁文件及 Gradle 依赖声明，提交前应检查这些差异。

```sh
python3 scripts/build_inputs.py resolve
cargo build -p clash-of-rust --release --locked --bins
python3 scripts/prepare-resources.py
```

再导出带校验清单的本地程序与测试程序，并打包。以下命令可用于 Bash 或 PowerShell；Windows 可用 `python` 替代 `python3`：

```sh
python3 -X utf8 -c "
import sys
from pathlib import Path
sys.path.insert(0, 'scripts')
from build_support import host_system, host_arch, cargo_messages
from workflow_artifacts import test_binaries, export_build, extract_build
messages = cargo_messages(['cargo', 'test', '-p', 'clash-of-rust', '--test', 'core_integration', '--release', '--locked', '--no-run', '--message-format=json'])
tests = test_binaries(messages)
archive = export_build(host_system(), host_arch(), tests, Path('dist/local-export'))
extract_build(archive, Path('dist/local-build'))
"
python3 scripts/build-installer.py --prebuilt dist/local-build
```

安装包与 SHA256 输出到 `dist/`。重复打包前清理自己生成的 `dist/local-export`、`dist/local-build`，或在命令中换用新目录。`--no-run` 仅编译集成测试程序，不执行代理/TUN 系统测试；本地产物清单不代表已经通过 CI，也不会创建 Release。`build-installer.py --prebuilt` 需要上述完整产物目录，不能直接传入 `target/release/`。`run-ci.py` 包含专用系统集成测试，不作为普通本地构建入口。

## CI

先生成一次 `dependency-snapshot`：每次查询源码依赖、内核、Geo 数据与独立工具更新，在声明的兼容约束内选取新版本。上游超时、503 等网络错误使用兼容的已验证缓存；缓存缺失或校验失败时明确报告。所有平台应用同一份快照，Publish Release 继续复用这些已测试版本。

缓存先从 GitHub 恢复，未命中或服务故障时再访问 Tailscale 私网中的 `/home/coder/github_action_cache`，两边都没有才从上游获取。每轮依然查询上游更新；工具链、锁文件和资源版本参与缓存身份，兼容的旧编译对象仍由 Cargo 校验和重编。Publish Release 复用本轮成功 CI 的依赖快照，不重新选版本。

私有 Rust 镜像分为 32 个校验分片，只回传变更部分，索引最后提交；中断后复用已完成的分片，缺片由正常下载或重编补齐，旧运行不能覆盖较新的内容。GitHub 缓存预算 9 GiB，优先保留公共下载、必要工具及较慢的 macOS/Windows 编译结果；命中后不重复下载私有副本。工作区源码逐文件核对内容，相同内容才恢复旧时间戳，变化的 crate 及使用共享输入的构建脚本会失效重编。测试报告和安装包不进入编译缓存。

Go 按声明的模块路径查询更新，避免探测无关父模块；不降级，网络错误保留兼容版本。Android 公共任务一次准备已校验的模块下载包和绑定工具，经本轮附件分发给三个 ABI；只缓存下载包，解压内容由 Go 重建，编译结果按 ABI 缓存。大型私有依赖镜像在 x64 测试通过后回写，不阻塞其他平台启动。

私网服务使用独立访问令牌，只可写自己的数据、索引和日志目录。各任务总结记录缓存来源、版本、大小与分阶段耗时，最终 `private-cache-inventory` 列出本地快照的校验值和占用。

单次可选缓存保存最多等待 5 分钟，未完成上传不会替换有效分片。健康的冷恢复不按总耗时强行中断，网络停滞由连接超时处理；解包使用原子写入，失败时清除不完整编译对象和解压源码，再正常下载与编译，保留依赖快照和测试报告。

- 六种桌面平台使用相同的 Release profile 完成整个 workspace 的格式检查、Clippy、编译和源码单元测试，包括共享移动会话；Android 另用三种 ABI 构建实际 Slint/Java/Go 宿主，iOS 不参与构建。
- 单元测试保留配置与规则、网络响应解析、认证信息脱敏、状态恢复、任务取消、列表复用与边界、实际指针交互等验证。已弃用的截图场景、CLI 专项集成测试和长期忽略的联网/平台手工回归入口已移除。
- 真实系统集成测试只运行系统代理与 TUN：验证代理设置写入、HTTP 转发、TUN 接管无显式代理的流量、模式恢复及关闭清理。Linux 使用隔离网络命名空间和 GSettings 内存后端；macOS 使用真实 launchd 网络助手；Windows 先注册服务，再以普通用户令牌运行代理/TUN、恢复和服务清理检查。
- Linux TUN 测试在独立双栈上游中使用正式保护覆盖，验证域名节点的引导解析、经节点的 DoH A/AAAA、TCP/UDP DNS 接管、HTTP 转发、冷启动及原设置恢复；额外阻断固定公网引导 DNS，验证订阅解析器仍可用。上游记录公网明文 DNS 与绕过节点的公网 DNS，发生泄漏即失败。保护 DNS 全部不可达时必须拒绝开启并恢复原网络。隔离环境使用专用测试 CA，保留 TLS 证书校验，不访问外网。
- Python 构建辅助与产物校验单元测试在前置依赖解析任务中执行一次，失败时不启动各平台下载与编译。内存基准 example 不作为 CI 测试程序额外构建或运行。
- main push、PR 更新触发 CI；过时运行自动取消。本地生成的编译、打包产物和缓存已在 `.gitignore` 中排除，不应提交。
- 桌面 CI 不生成安装包；Android CI 生成供安装测试与发布复用的 APK。产物记录提交、版本、架构、Cargo.lock、工具链及逐文件校验值；仅附带发布后复用的 `core_integration` 代理/TUN 测试程序，不上传源码单元测试程序。清理后的产物采用 schema 2，只与同一提交的打包流程配套使用。

Android x86_64 使用 API 35 模拟器运行真实代理/TUN 流量测试，验证正式界面的内核状态、默认策略组、订阅导航和设置触摸滚动，以及系统 VPN 授权、DNS、套接字保护、快捷磁贴连接/断开和路由恢复；ARM64/ARMv7 完成实际编译、Gradle lint 与架构校验，仍需真机验收。Android 测试结果、体积报告和 logcat 保存在 `android-report-{arch}`，固定开发签名与构建说明见 [Android 宿主](../apps/android/README.md)。

macOS 失败日志、系统与网络状态、崩溃报告保存在 `macos-ci-report-{arch}`，Windows 集成检查结果保存在 `windows-ci-report-{arch}`。人工授权窗口、真实重启登录和物理显示器行为仍需人工验收。

## Publish Release

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的当前提交 CI 编号；留空选择当前提交的 main CI。开发分支测试包必须填写当前分支、当前提交成功的 PR CI 编号。
- `check_only`：勾选后完成打包与安装后验证，不创建或修改 Release；开发分支必须勾选。安装包在本次运行的 `package-{system}-{arch}` 附件中，无需下载到本地。

流程固定到所选 CI 的提交与依赖快照，核对六种桌面平台及三种 Android ABI 的成功记录和产物身份，不重新编译，也不重新下载应用资源。过期或旧格式产物需要重新运行当前代码的 CI。

1. 六个平台获取并校验已有程序，再生成 Windows EXE、Linux DEB、macOS DMG。
2. Windows 验证安装、服务注册、普通用户代理/TUN、更新安装路径、卸载与用户文件保留，并扫描 Defender；PE 图标在打包阶段核验。
3. Linux 安装 DEB 后验证普通用户系统代理、HTTP 转发、TUN 接管与清理；最新版 Ubuntu 使用同一份 Ubuntu 22.04 DEB 再验证一次。
4. macOS 核验 App/DMG 版本、架构、资源及签名，启动安装后的 GUI/内核并验证代理/TUN。当前使用 ad-hoc 签名，没有 Developer ID 与 Apple 公证。
5. Android 复用三种 ABI 的已编译 APK，检查签名、架构与 16 KiB 对齐；x86_64 最终包再运行代理/TUN 测试。`check_only` 保留固定开发签名，正式发布要求单独配置长期签名。
6. 核对安装包、校验值、成功记录及来源后，正式模式才公开 Release；`check_only` 仅保留 Actions 附件。开发版本标记为 prerelease，不覆盖稳定版 latest。

只有最终发布 job 有 `contents: write`。发布工作流向 `scripts/build-installer.py --prebuilt` 传入已验证的 CI 产物；本地打包可传入按上述步骤生成的本地产物。打包器只校验与组装，不隐式调用 Cargo。

平台检查分别集中在 `scripts/windows_ci.py`、`scripts/macos_ci.py` 和 Linux 的隔离网络夹具中；`scripts/test-release-package.py` 统一调度。Windows 安装、更新、卸载与 Defender 扫描复用同一模块，不再保留独立的命令行包装脚本。

## 手动验收与助手清理

Actions 验证基础安装、系统代理和 TUN；首次授权与取消、真实注销登录和重启、休眠、多显示器、系统通知仍需实机检查。更新验收应从旧版本开始，确认下载可取消和重试、校验失败不安装、安装失败可恢复旧版，升级后保留订阅与个人设置。

Windows 更新需要完成 UAC，Linux 需要 `pkexec` 与桌面授权服务。macOS 数据保存在用户 Application Support，系统代理与 TUN 共用 launchd 网络助手，GUI 以普通用户运行；已有助手可供后台自启复用，退出恢复不应重新索取授权。首次下载后的 Gatekeeper 提示按 [Apple 说明](https://support.apple.com/zh-cn/102445)处理。

macOS 卸载前关闭代理并退出客户端，再按需移除当前用户的网络助手：

```sh
service_label="org.clashofrust.network.$(id -u)"
sudo launchctl bootout "system/$service_label"
sudo rm -rf "/Library/PrivilegedHelperTools/$service_label" "/Library/LaunchDaemons/$service_label.plist"
```

助手停止前会尝试恢复自己仍持有的代理设置并终止内核会话。测试夹具仅移除本次创建的助手，拒绝覆盖已有安装。

## UI 预览与静态资源

持久安装官方 Slint Viewer 后运行：

```text
python3 scripts/preview-slint.py --dialogs --pairs
```

脚本直接加载正式 `.slint` 组件和演示数据，不运行 Cargo 或真实内核。需要时生成 `dist/slint-preview`；历史截图与字体缓存可随 `dist` 删除。

图标、国旗及许可证属于必要资源；日常构建使用仓库成品。仅在维护资源时使用 `scripts/prepare-assets.py icons` / `flags`。图标源图保留于 `resources/icon-concepts`，第三方来源见各资源目录的说明。平台检测直接维护 Rust，必要的公开检测数据与 AGPL 来源说明统一位于 `resources/ip-check`，不再保留 `vendor` 或 Shell 生成工具。

## Memory Benchmark

手动选择 `ref`（分支、标签或提交），默认测当前所选分支：

- `profile`：release 或 dev，默认 release。
- `workload`：默认 light；stress 保留较大的压力输入。
- `rows`：可选 API 行数，1–50,000；默认 light 为 1,000，stress 为 50,000。
- `repetitions`：每项 API 测试的独立进程次数，1–10，默认 3。
- `sample_seconds`：每个 GUI 场景采样 1–120 秒，默认 10 秒。

一次只测一个版本。比较时分别运行，保持工具链、profile、输入和环境一致；常规 CI 和打包不自动运行内存基准。

API 基准覆盖规则、连接、代理三类数据的流式/缓冲解析和 1/5 次刷新。GUI 在隔离数据目录、Xvfb 中采样首页、代理组折叠、节点搜索、返回首页；关闭系统代理与定时测速，不生成业务流量。light 为 100 节点、5 个代理组、1,000 条规则；stress 为 2,000 节点、40 个代理组、20,000 条规则。支持的版本另测五项快捷规则全开的同样场景。

工作流只替换所选版本的 example 基准程序，兼容旧版目录布局，不改应用源码。产物包含版本与环境、提交身份、资源清单、输入订阅、API 数据、GUI/内核内存采样和报告，不生成截图。API 为 Rust 堆分配量；GUI/内核分别统计 RSS、PSS、USS，需区分口径；采样峰值为观测峰值。
