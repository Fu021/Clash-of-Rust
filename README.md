<p align="center">
  <img src="resources/icons/app.png" width="80" height="80" alt="Clash of Rust 图标">
</p>
<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="resources/icons/wordmark-dark.svg">
    <img src="resources/icons/wordmark-light.svg" width="348" alt="Clash of Rust">
  </picture>
</h1>

<p align="center">
  <a href="README.en.md" lang="en">English</a>
</p>
<p align="center">原生界面 · 命令行 · 应用分流 · 网络诊断</p>
<p align="center">
  <a href="https://github.com/Fu021/Clash-of-Rust/releases/latest"><img src="https://img.shields.io/github/v/release/Fu021/Clash-of-Rust?label=stable&amp;color=2b9b78" alt="最新正式版"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/releases"><img src="https://img.shields.io/github/v/release/Fu021/Clash-of-Rust?include_prereleases&amp;sort=date&amp;label=latest&amp;color=c77d45" alt="最新发布（含预发布）"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Fu021/Clash-of-Rust/ci.yml?branch=main&amp;label=CI" alt="main 分支 CI 状态"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/releases"><img src="https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/total?label=downloads&amp;color=577cad" alt="全部版本累计下载次数"></a>
</p>
<p align="center">
  <a href="https://clash-of-rust.fu021.shop/"><img src="resources/icons/website-button.svg" width="176" height="44" alt="访问项目网站：功能介绍与软件下载"></a>
  <a href="https://clash-of-rust.fu021.shop/download"><img src="resources/icons/download-button.svg" width="136" height="44" alt="下载安装：选择平台与架构"></a>
</p>

Clash of Rust 是基于 **Rust + Slint + mihomo** 的原生代理客户端。桌面 GUI、TUI 和 `cor` 命令行共用后台，Android 提供独立客户端。

下表列出 **0.7.3 正式版**的安装范围和实测环境。

| 平台 | 安装支持 | 实测环境 |
| --- | --- | --- |
| Windows | x64 / ARM64 | Windows Server 2025（x64）、Windows 11（x64 / ARM64） |
| Linux | DEB、RPM（x64 / ARM64），Arch 原生包（x64） | Ubuntu 20.04+、Debian、Linux Mint、Pop!_OS、Zorin OS、Fedora、openSUSE、Arch Linux |
| macOS | 14+，Intel / Apple Silicon | macOS 15.7.9（Intel）、26.7.2（Apple Silicon） |
| Android | 8.0+，ARM64 / ARMv7 / x86_64 | Android 8.0 至 16（11 个 x86_64 模拟器版本） |

## 功能一览

| 功能 | 能做什么 |
| --- | --- |
| 🖥️ 原生界面 | 简体中文与英文、深浅主题、强调色、宫格与列表，适配手机和平板。 |
| ⌨️ 命令行 | 支持 Windows、Linux、macOS 无界面运行，可通过 SSH 操作。 |
| 📱 应用分流 | Android 可纳入或排除应用。Android 10+ 可为单个应用指定直连、策略组或节点。 |
| 🌐 网络检测 | 检查出口 IP 及 GitHub、AI 和流媒体连通性，支持筛选、重测和导出报告。 |
| 🛡️ Claude 保护 | 检查地区、DNS、WebRTC 和项目配置。开启保护后锁定节点和网络修改，启用 TUN、加密 DNS 与异常连接拦截，关闭后恢复设置。 |
| 📦 内置资源与更新 | 内置 mihomo 与 Geo 数据。支持 Geo 和客户端更新、下载进度与取消，并校验安装包 SHA256。桌面可切回正式版，Android 安装需系统确认。 |
| ⚙️ 日常使用 | 托盘切换节点和显示速度，支持后台自启动、单实例及窗口位置记忆。关窗释放界面进程，可在 DNS 覆盖中管理 Fake-IP 和排除项。 |

## 界面预览

0.7.3 的桌面组件预览和隔离测试中的实际终端画面。

| 浅色首页 | 深色首页 |
| --- | --- |
| [![浅色首页](website/src/assets/app-home.png)](website/src/assets/app-home.png) | [![深色首页](docs/images/home-0.7.3-dark.png)](docs/images/home-0.7.3-dark.png) |

| TUI 首页 |
| :---: |
| [![TUI 首页](docs/images/tui-0.7.3.png)](docs/images/tui-0.7.3.png) |

## 安装与使用

下表提供 0.7.3 正式版安装包。`.sha256` 校验文件和其他版本见[下载页](https://clash-of-rust.fu021.shop/download)及 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases)。

| 平台 | 安装包与下载次数 |
| --- | --- |
| Windows | [![Windows Setup x64](docs/releases/badges/windows-x64.svg)][windows-x64] [![Windows x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-windows-x64-setup.exe?label=downloads&displayAssetName=false)][windows-x64]<br>[![Windows Setup ARM64](docs/releases/badges/windows-arm64.svg)][windows-arm64] [![Windows ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-windows-arm64-setup.exe?label=downloads&displayAssetName=false)][windows-arm64] |
| Linux | [![Linux DEB x64](docs/releases/badges/linux-x64.svg)][linux-deb-x64] [![Linux DEB x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-amd64.deb?label=downloads&displayAssetName=false)][linux-deb-x64]<br>[![Linux DEB ARM64](docs/releases/badges/linux-arm64.svg)][linux-deb-arm64] [![Linux DEB ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-arm64.deb?label=downloads&displayAssetName=false)][linux-deb-arm64]<br>[![Linux RPM x64](docs/releases/badges/linux-rpm-x64.svg)][linux-rpm-x64] [![Linux RPM x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.rpm?label=downloads&displayAssetName=false)][linux-rpm-x64]<br>[![Linux RPM ARM64](docs/releases/badges/linux-rpm-arm64.svg)][linux-rpm-arm64] [![Linux RPM ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-aarch64.rpm?label=downloads&displayAssetName=false)][linux-rpm-arm64]<br>[![Linux Arch pkg x64](docs/releases/badges/linux-arch-x64.svg)][linux-arch-x64] [![Linux Arch x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.pkg.tar.zst?label=downloads&displayAssetName=false)][linux-arch-x64] |
| macOS | [![macOS DMG Apple Silicon](docs/releases/badges/macos-arm64.svg)][macos-arm64] [![macOS Apple Silicon downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-macos-arm64.dmg?label=downloads&displayAssetName=false)][macos-arm64]<br>[![macOS DMG Intel x64](docs/releases/badges/macos-x64.svg)][macos-x64] [![macOS Intel x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-macos-x64.dmg?label=downloads&displayAssetName=false)][macos-x64] |
| Android | [![Android APK ARM64](docs/releases/badges/android-arm64.svg)][android-arm64] [![Android ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-android-arm64.apk?label=downloads&displayAssetName=false)][android-arm64] （多数手机和平板）<br>[![Android APK ARMv7](docs/releases/badges/android-armv7.svg)][android-armv7] [![Android ARMv7 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-android-armv7.apk?label=downloads&displayAssetName=false)][android-armv7]<br>[![Android APK x64](docs/releases/badges/android-x64.svg)][android-x64] [![Android x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-android-x64.apk?label=downloads&displayAssetName=false)][android-x64] |

[windows-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-windows-x64-setup.exe
[windows-arm64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-windows-arm64-setup.exe
[linux-deb-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-amd64.deb
[linux-deb-arm64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-arm64.deb
[linux-rpm-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.rpm
[linux-rpm-arm64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-aarch64.rpm
[linux-arch-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.pkg.tar.zst
[macos-arm64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-macos-arm64.dmg
[macos-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-macos-x64.dmg
[android-arm64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-android-arm64.apk
[android-armv7]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-android-armv7.apk
[android-x64]: https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-android-x64.apk

Windows 运行安装程序。Linux 按发行版选择 DEB、RPM 或 Arch 原生包，命令见[安装与校验](https://clash-of-rust.fu021.shop/download#linux)。macOS 将 DMG 内的应用拖入「应用程序」。Android 安装 APK 后，在首次连接时允许 VPN 授权，可在系统快捷设置中添加开关磁贴。

打开应用后，在「订阅」页导入订阅，在「代理」页选择节点，再从首页启用系统代理或 TUN。Android 从首页连接 VPN。

升级保留订阅和设置。Windows 卸载时默认保留数据，可主动勾选删除。如遇 Defender 告警，用 PowerShell 的 `Get-FileHash` 核对安装包和发布附带的 SHA256。若摘要不匹配或告警未核实，先暂停安装。

## 命令行

桌面安装包均附带 `cor`。在交互终端运行 `cor` 打开终端界面，`cor start` 启动后台，`cor help` 查看命令。在服务器上可运行 `cor proxy tun` 启用 TUN，首次开启可能需要系统授权。

让 AI Agent 协助时，可让它先运行 `cor agent guide` 阅读内置指南。Windows 安装器默认将 `cor` 加入 PATH。macOS 从「应用程序」打开一次后，会为当前用户配置 PATH。之后打开新的登录终端即可运行 `cor`。

## 内存占用

**0.7.3** 在 Ubuntu 22.04 x64、Xvfb/X11 的 Release 构建上测量。配置含 100 个节点、5 个策略组、1,000 条规则，关闭代理和定时测速，无业务流量。前台为 10 秒内的观测峰值，后台为该场景开始后约 60 秒的驻留值，单位 MiB。

| 场景 | 客户端 RSS | 客户端 PSS | 客户端与内核合计 PSS |
| --- | ---: | ---: | ---: |
| GUI 首页、托盘可用 | 56.39 | 46.70 | 94.57 |
| 关闭 GUI、驻留托盘 | 22.69 | 17.76 | 65.64 |
| GUI 重开 20 次后关闭 | 23.87 | 18.95 | 72.68 |
| 首次打开 TUI 首页 | 31.00 | 15.85 | 59.53 |
| 浏览各页后退出 TUI | 17.12 | 13.44 | 60.57 |
| TUI 重开 20 次后退出 | 17.15 | 13.46 | 62.77 |
| 纯 CLI 后台运行 | 16.48 | 12.79 | 56.44 |

客户端列统计后台和当前 GUI/TUI，合计列纳入内核。关窗或退出 TUI 后界面进程结束，后台与内核继续运行，合计 PSS 仍高于纯 CLI 基线。

其他页面与快捷规则场景：

| 场景 | 客户端 RSS | 客户端 PSS | 客户端与内核合计 PSS |
| --- | ---: | ---: | ---: |
| 代理组折叠 | 55.20 | 45.57 | 97.47 |
| 节点搜索 | 61.02 | 51.39 | 104.74 |
| 搜索后返回首页 | 56.92 | 47.29 | 100.65 |
| 五项快捷规则全开时的首页 | 54.44 | 44.66 | 109.90 |

RSS 完整计入共享页，PSS 按比例分摊。这些是 Linux 短时测量，实际占用随设备、配置和流量变化。[0.7.3 原始基准数据](https://github.com/Fu021/Clash-of-Rust/actions/runs/45788967631)。

## 编译与打包

### 本地编译

准备 Rust stable、Python 3.11+ 和原生工具链。Windows 使用 MSVC，macOS 使用 Xcode Command Line Tools，Linux 需要 C/C++、pkg-config 及 GLib、Fontconfig/FreeType、XKB、Wayland/X11 开发库。在仓库根目录运行以下命令。

```sh
python3 scripts/prepare-resources.py
cargo build --target-dir target/desktop -p clash-of-rust -p clash-desktop-ui -p clash-cli --bins --locked
```

桌面程序生成在 `target/desktop/debug/`，内核与 Geo 资源在 `bundle/`。桌面宿主与界面程序需放在同一目录。Windows 可用 `python` 运行第一条命令。

### GitHub Actions

[CI](https://github.com/Fu021/Clash-of-Rust/actions/workflows/ci.yml) 编译并验证六个桌面目标及三个 Android ABI。通过后打开该次运行的 **Artifacts**，下载对应平台的 `package-*`，内含安装包和 `.sha256`。**Publish Release** 的 `check_only` 模式可复用同提交产物核验安装包。

## 参与贡献

通过 [Issue 表单](https://github.com/Fu021/Clash-of-Rust/issues/new/choose)提交问题或建议。遇到故障，可从报错通知或日志页选择「导出诊断」，也可运行 `cor diagnose --output cor-diagnose.json`。确认 JSON 可分享后，将它附到问题表单。欢迎 AI 辅助的 PR，提交前请完成适用的本地验证。

详细使用和开发资料集中在[文档导航](docs/README.md)。

## License

原创代码采用 [GPL-3.0-only](LICENSE)，复用及派生的 RegionRestrictionCheck IP 检测代码保留 [AGPL-3.0-only](resources/ip-check/SOURCE.md)。组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。mihomo 与 Slint 按 GPL-3.0 使用，其他来源见[第三方声明](resources/THIRD-PARTY-NOTICES.txt)。
