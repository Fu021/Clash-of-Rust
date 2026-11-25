<p align="center">
  <img src="resources/icons/app.png" width="80" height="80" alt="Clash of Rust icon">
</p>
<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="resources/icons/wordmark-dark.svg">
    <img src="resources/icons/wordmark-light.svg" width="348" alt="Clash of Rust">
  </picture>
</h1>

<p align="center">
  <a href="README.md" lang="zh-CN">简体中文</a>
</p>
<p align="center">Native UI · Command line · Per-app routing · Network diagnostics</p>
<p align="center">
  <a href="https://github.com/Fu021/Clash-of-Rust/releases/latest"><img src="https://img.shields.io/github/v/release/Fu021/Clash-of-Rust?label=stable&amp;color=2b9b78" alt="Latest stable release"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/releases"><img src="https://img.shields.io/github/v/release/Fu021/Clash-of-Rust?include_prereleases&amp;sort=date&amp;label=latest&amp;color=c77d45" alt="Latest release, including pre-releases"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Fu021/Clash-of-Rust/ci.yml?branch=main&amp;label=CI" alt="CI status for the main branch"></a>
  <a href="https://github.com/Fu021/Clash-of-Rust/releases"><img src="https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/total?label=downloads&amp;color=577cad" alt="Total downloads across all releases"></a>
</p>
<p align="center">
  <a href="https://clash-of-rust.fu021.shop/en/"><img src="resources/icons/website-button-en.svg" width="176" height="44" alt="Project website — features and downloads"></a>
  <a href="https://clash-of-rust.fu021.shop/en/download"><img src="resources/icons/download-button-en.svg" width="136" height="44" alt="Download the installer for your platform and architecture"></a>
</p>

Clash of Rust is a native proxy client built with **Rust + Slint + mihomo**. The desktop GUI, TUI, and `cor` command line share a backend. Android has a dedicated client.

The table lists supported installs and tested environments for **stable release 0.7.2**.

| Platform | Supported installs | Tested environments |
| --- | --- | --- |
| Windows | x64 / ARM64 | Windows desktop (x64), Windows 11 (ARM64), Windows Server 2025 (x64) |
| Linux | Debian or Ubuntu 20.04+, amd64 / ARM64 | Ubuntu 20.04 and 22.04 (x64 / ARM64), 24.04 and 26.04 (x64) |
| macOS | 14+, Intel / Apple Silicon | macOS 15.7.9 (Intel), 26.7.2 (Apple Silicon) |
| Android | 8.0+, ARM64 / ARMv7 / x86_64 | Android 8.0 through 16 (11 x86_64 emulator versions) |

## Features

| Feature | What you can do |
| --- | --- |
| 🖥️ Native interface | Light and dark themes, accent colors, grid and list views, and layouts for phones and tablets. |
| ⌨️ Command line | Run headless on Windows, Linux, and macOS. Manage the client over SSH. |
| 📱 Per-app routing | Include or exclude Android apps. Android 10+ can assign an app to a direct connection, policy group, or node. |
| 🌐 Network diagnostics | Check exit IPs and connectivity to GitHub, AI services, and streaming platforms. Filter, rerun, and export reports. |
| 🛡️ Claude protection | Check region, DNS, WebRTC, and project configuration. Protection locks node and network changes, enables TUN and encrypted DNS, and blocks anomalous connections. Disabling it restores settings. |
| 📦 Bundled resources and updates | Includes mihomo and Geo data. Client and Geo downloads show progress and can be canceled. Installer SHA256 is verified. Desktop can switch back to stable, and Android prompts for installation approval. |
| ⚙️ Everyday use | Switch nodes and view speeds in the tray. Supports autostart, a single instance, and window position recall. Closing the window ends the UI process. DNS overrides let you manage Fake-IP and exclusions. |

## Interface Preview

| Home screen, light theme | Home screen, dark theme |
| --- | --- |
| [![Home screen, light theme](docs/images/home-0.7.0-light.png)](docs/images/home-0.7.0-light.png) | [![Home screen, dark theme](docs/images/home-0.7.0-dark.png)](docs/images/home-0.7.0-dark.png) |

| TUI home screen |
| :---: |
| [![TUI home screen](docs/images/tui-0.7.2.png)](docs/images/tui-0.7.2.png) |

## Installation and Usage

The table links to stable version 0.7.2. Find `.sha256` files and other versions on the [download page](https://clash-of-rust.fu021.shop/en/download) and in [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases).

| Platform | Installers and download counts |
| --- | --- |
| Windows | [![Windows Setup x64](https://img.shields.io/badge/Setup-x64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-windows-x64-setup.exe) [![Windows x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-windows-x64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-windows-x64-setup.exe)<br>[![Windows Setup ARM64](https://img.shields.io/badge/Setup-ARM64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-windows-arm64-setup.exe) [![Windows ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-windows-arm64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-windows-arm64-setup.exe) |
| Debian / Ubuntu 20.04+ | [![Linux DEB x64](https://img.shields.io/badge/DEB-x64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-linux-amd64.deb) [![Linux x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-linux-amd64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-linux-amd64.deb)<br>[![Linux DEB ARM64](https://img.shields.io/badge/DEB-ARM64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-linux-arm64.deb) [![Linux ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-linux-arm64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-linux-arm64.deb) |
| macOS | [![macOS DMG Apple Silicon](https://img.shields.io/badge/DMG-Apple_Silicon-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-macos-arm64.dmg) [![macOS Apple Silicon downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-macos-arm64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-macos-arm64.dmg)<br>[![macOS DMG Intel](https://img.shields.io/badge/DMG-Intel_x64-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-macos-x64.dmg) [![macOS Intel downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-macos-x64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-macos-x64.dmg) |
| Android | [![Android APK ARM64](https://img.shields.io/badge/APK-ARM64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-arm64.apk) [![Android ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-android-arm64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-arm64.apk) (most phones and tablets)<br>[![Android APK ARMv7](https://img.shields.io/badge/APK-ARMv7-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-armv7.apk) [![Android ARMv7 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-android-armv7.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-armv7.apk)<br>[![Android APK x64](https://img.shields.io/badge/APK-x64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-x64.apk) [![Android x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.2/Clash-of-Rust-0.7.2-android-x64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.2/Clash-of-Rust-0.7.2-android-x64.apk) |

On Windows, run the installer. On Debian or Ubuntu, run `sudo apt install ./package.deb` with the filename you downloaded. On macOS, drag the app from the DMG to Applications. On Android, install the APK and approve the first VPN connection. You can add a toggle to Quick Settings.

Open the app, import a subscription on the Subscriptions page, and select a node on the Proxies page. From Home, enable System Proxy or TUN. On Android, connect to the VPN from Home.

Upgrades preserve subscriptions and settings. The Windows uninstaller keeps data by default and offers an option to delete it. If Defender raises an alert, use PowerShell's `Get-FileHash` to compare the installer with the published SHA256. Pause installation if the hash differs or the alert remains unresolved.

## Command Line

Desktop installers include `cor`. Run `cor ui` to open the terminal interface, `cor start` to start the backend, and `cor help` for available commands. On servers, run `cor proxy tun` to enable TUN. The first use may require system authorization.

Starting with `0.7.3-dev.3.memory`, running `cor` in an interactive terminal also opens the terminal interface. When working with an AI agent, ask it to run `cor agent guide` first for the built-in guide. The Windows installer for this version adds `cor` to PATH by default. On macOS, drag the app into Applications and open it once to configure PATH for your user automatically. Open a new terminal to run `cor`.

## Memory Usage

Measured on **0.7.2** using Ubuntu 22.04 x64 and a release build with software rendering. The profile had 100 nodes, 5 policy groups, and 1,000 rules, with no application traffic. Values are peak MiB over 10 seconds per scenario.

| Scenario | Client RSS | Client PSS | Combined client and core PSS |
| --- | ---: | ---: | ---: |
| GUI home screen, tray available | 56.18 | 46.25 | 92.39 |
| GUI closed, tray still running | 23.36 | 18.23 | 64.50 |
| First open of the TUI home screen | 29.42 | 14.86 | 57.74 |
| TUI closed after browsing pages | 18.25 | 14.53 | 61.15 |
| TUI home screen reopened | 33.71 | 19.12 | 65.74 |
| TUI closed again | 18.31 | 14.59 | 61.27 |
| Headless CLI backend running | 15.65 | 11.93 | 54.68 |

Client columns include the backend and any open GUI or TUI. The combined column adds the core. Closing the GUI or exiting the TUI ends the interface process while the backend and core keep running.

Other pages and quick-rule scenarios follow.

| Scenario | Client RSS | Client PSS | Combined client and core PSS |
| --- | ---: | ---: | ---: |
| Proxy groups collapsed | 54.73 | 45.00 | 94.04 |
| Node search | 65.77 | 56.04 | 108.07 |
| Back to the home screen after searching | 58.01 | 48.28 | 100.70 |
| Home screen with all five quick rules enabled | 55.18 | 45.33 | 109.68 |

RSS counts shared pages in full. PSS counts a proportional share. Actual usage varies with device, configuration, and traffic. [Raw benchmark results](https://github.com/Fu021/Clash-of-Rust/actions/runs/44103803255).

## Building and Packaging

### Local Build

Install stable Rust, Python 3.11+, and your platform's native tools. Windows uses MSVC. Linux needs C/C++ tools, pkg-config, and development libraries for GLib, Fontconfig/FreeType, XKB, and Wayland/X11. macOS uses the Xcode Command Line Tools. Run these commands from the repository root.

```sh
python3 scripts/prepare-resources.py
cargo build --target-dir target/desktop -p clash-of-rust -p clash-desktop-ui -p clash-cli --bins --locked
```

Desktop binaries go into `target/desktop/debug/`. Core and Geo resources go into `bundle/`. Keep the desktop host and UI executable in the same directory. On Windows, you can use `python` for the first command.

### GitHub Actions

[CI](https://github.com/Fu021/Clash-of-Rust/actions/workflows/ci.yml) builds and verifies six desktop targets and three Android ABIs. Open **Artifacts** on the successful run and download the `package-*` archive for your platform. It contains the installer and `.sha256` file. **Publish Release** can reuse those artifacts to verify the installers in `check_only` mode.

## Contributing

Use the [issue forms](https://github.com/Fu021/Clash-of-Rust/issues/new/choose) to report problems or suggest improvements. To report a failure, export diagnostics from the error notification or Logs page, or run `cor diagnose --output cor-diagnose.json`. Review the JSON before attaching it to the form. AI-assisted PRs are welcome. Run the applicable local checks before submitting.

Find detailed usage and development material in the [documentation index](docs/README.md).

## License

Original code uses [GPL-3.0-only](LICENSE). IP checks reused or derived from RegionRestrictionCheck retain [AGPL-3.0-only](resources/ip-check/SOURCE.md). The combined distribution follows section 13 of GPLv3 and AGPLv3, including offering complete corresponding source to remote network users where applicable. mihomo and Slint are used under GPL-3.0. See the [third-party notices](resources/THIRD-PARTY-NOTICES.txt) for other sources.
