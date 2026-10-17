<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 mihomo 的原生 Rust 代理客户端。支持 **Windows（x64 / ARM64）**、**Debian/Ubuntu（amd64 / ARM64，Ubuntu 仅支持 22.04 及以上）**、**macOS 14 及以上（Intel / Apple Silicon）** 和 **Android 8.0 及以上（ARM64 / ARMv7 / x86_64）**。版本 **0.6.0**，更新内容见 [发布说明](docs/releases/0.6.0.md)。

## Features

- **轻量**：Rust + Slint 原生界面，无需 WebView，页面数据按需加载。
- **离线就绪**：内置 mihomo 内核和 Geo 数据，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项，覆盖出口 IP、GitHub、AI 服务与流媒体地区访问；一键检测显示分类报告，点击分类或状态查看详情，支持筛选与分类重测。
- **A÷检测**：采集 Claude 地区、DNS、WebRTC 与浏览器线索，扫描已记录项目配置并分类展示；可选一键保护，固定节点并启用 TUN、加密 DNS 与异常连接拦截，关闭后恢复原设置，详见 [检测说明](docs/claude-region-detection.md)。
- **便捷使用**：托盘快捷切换节点，可选显示上传与下载速度；支持后台开机自启、单实例与网络诊断，记住普通窗口尺寸；内核启动失败时可直接重试。
- **客户端更新**：仅检查正式版，自动选择代理并回退，支持下载进度与取消；校验完成后自动安装与重启，失败时显示原因。
- **WSL Secure Egress（定制分支）**：原生管理页提供预检、安装、连接、状态、测试、回退和卸载。Windows 原有双栈网络不变；WSL 业务流量固定经德国 VPS 的 IPv4 WireGuard 出口，IPv6 与隧道故障保持失败关闭。Tailscale 严格冷启动边界未验证前保持 BLOCKED。实现状态、限制与恢复步骤见 [WSL Secure Egress](docs/wsl-secure-egress/README.md)。
- **客户端更新**：仅检查正式版，支持下载进度与取消；校验完成后桌面自动安装与重启，Android 由系统确认安装，失败时显示原因。

## 安装与使用

从 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 下载当前正式版对应系统和架构的安装包，附带 `.sha256` 校验文件。

| 平台 | 安装包 |
| --- | --- |
| Windows x64 | [Clash-of-Rust-0.6.0-windows-x64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-windows-x64-setup.exe) |
| Windows ARM64 | [Clash-of-Rust-0.6.0-windows-arm64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-windows-arm64-setup.exe) |
| Debian / Ubuntu 22.04+ amd64 | [Clash-of-Rust-0.6.0-linux-amd64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-linux-amd64.deb) |
| Debian / Ubuntu 22.04+ ARM64 | [Clash-of-Rust-0.6.0-linux-arm64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-linux-arm64.deb) |
| macOS Intel | [Clash-of-Rust-0.6.0-macos-x64.dmg](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-macos-x64.dmg) |
| macOS Apple Silicon | [Clash-of-Rust-0.6.0-macos-arm64.dmg](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-macos-arm64.dmg) |
| Android ARM64（多数手机与平板） | [Clash-of-Rust-0.6.0-android-arm64.apk](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-android-arm64.apk) |
| Android ARMv7 | [Clash-of-Rust-0.6.0-android-armv7.apk](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-android-armv7.apk) |
| Android x86_64 | [Clash-of-Rust-0.6.0-android-x64.apk](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.0/Clash-of-Rust-0.6.0-android-x64.apk) |

Windows 运行安装程序；Debian/Ubuntu（Ubuntu 仅支持 22.04 及以上）使用 `sudo apt install ./安装包文件名.deb`。升级保留订阅和个人设置；Windows 卸载可选删除全部配置和数据，默认不勾选。

macOS 打开 DMG，将 **Clash of Rust.app** 拖入 **Applications（应用程序）** 后启动。Android 安装对应架构的 APK，首次连接允许 VPN 授权；可在系统快捷设置中添加开关磁贴。

可能造成 Windows Defender 误判查杀，需自行规避。

## 内存占用

实测版本 **0.6.0**（`843ed2a`）；Ubuntu 22.04 x64、Rust 1.99.1、Release 构建、Slint 软件渲染，mihomo v1.19.32；低强度固定输入为 **100 节点、5 个策略组、1,000 条规则**。DNS 覆盖、IPv6 与 UDP 开启，Claude 保护、快捷规则、系统代理和 TUN 关闭，无业务流量：

| 场景 | GUI RSS | GUI 与内核合计 PSS |
| --- | ---: | ---: |
| 首页 | 44.32 MiB | 82.65 MiB |
| 代理组折叠 | 45.11 MiB | 86.71 MiB |
| 节点搜索 | 53.92 MiB | 98.83 MiB |
| 返回首页 | 53.99 MiB | 98.90 MiB |

五项快捷规则全开时，首页 GUI RSS 为 43.41 MiB，合计 PSS 为 98.30 MiB。以上为每场景采样 10 秒、间隔 250 ms 的观测峰值，实际占用随系统、配置和使用情况变化；RSS 含共享页，PSS 按比例分摊共享页。[测试记录](https://github.com/Fu021/Clash-of-Rust/actions/runs/39024865167)。

## 编译与打包

[GitHub Actions](https://github.com/Fu021/Clash-of-Rust/actions) 覆盖六种桌面系统与架构及三种 Android ABI 的编译、检查、单元测试与代理/TUN 测试；界面可使用 Slint Viewer 独立预览，无需编译桌面程序。

测试安装包：等待对应提交的 CI 成功，手动运行 **Publish Release**，填写 `ci_run_id` 并勾选 `check_only`；安装包保存在 Actions 附件中。打包直接复用 CI 产物，不再编译。正式发布使用同一工作流，详见 [工作流说明](docs/workflows.md)。

## 开发与发布

Cargo workspace 分为共享业务（`crates/client-core`）、Slint 界面（`crates/client-ui`）与桌面宿主（`apps/desktop`）。Android 使用独立 Slint 原生宿主与系统 VPN，并通过移动桥接（`crates/mobile-ffi`）复用共享业务，见 [Android 开发说明](apps/android/README.md)；iOS 保留独立界面与宿主占位。

安装 Slint Viewer 后可独立预览界面：

```text
python3 scripts/preview-slint.py --dialogs --pairs
```

测试、内存基准与发布流程见 [工作流文档](docs/workflows.md)；平台验收与更新验证见 [macOS 测试清单](docs/macos-ci-checklist.md) 和 [更新测试说明](docs/automatic-update.md)。此 fork 禁用官方自动更新，避免上游包覆盖 WSL 策略资源；升级与回退见 [WSL 运维指南](docs/wsl-secure-egress/OPERATIONS.md)。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生逻辑保留 **AGPL-3.0-only**，见 [授权及修改说明](resources/ip-check/SOURCE.md)；组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

mihomo 采用 GPL-3.0，Slint 按 GPL-3.0 使用。第三方软件与数据来源见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)。
