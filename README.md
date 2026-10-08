<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 mihomo 的原生 Rust 代理客户端。支持 **Windows（x64 / ARM64）** 和 **Debian/Ubuntu（amd64 / ARM64）**。当前版本 **0.4.10**。

## Features

- **轻量**：原生桌面界面，无需 WebView，页面数据按需加载。
- **代理模式**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点**：支持 YAML 订阅、策略组选择和批量延迟测试；订阅导入与更新自动尝试代理和直连。
- **离线就绪**：内置 mihomo 内核和 Geo 数据，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项，覆盖出口 IP、GitHub、AI 服务与流媒体地区访问；支持批量检测、取消和搜索。
- **便捷使用**：托盘、后台开机自启、单实例、连接与日志查看、网络诊断和主题切换。
- **客户端更新**：自动选择代理并回退，显示下载进度，支持取消下载；校验完成后自动安装与重启，失败时显示原因。

## 安装与使用

从 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 下载对应系统和架构的安装包，附带 `.sha256` 校验文件。

| 平台 | 安装包 |
| --- | --- |
| Windows x64 | `Clash-of-Rust-0.4.10-windows-x64-setup.exe` |
| Windows ARM64 | `Clash-of-Rust-0.4.10-windows-arm64-setup.exe` |
| Debian/Ubuntu amd64 | `Clash-of-Rust-0.4.10-linux-amd64.deb` |
| Debian/Ubuntu ARM64 | `Clash-of-Rust-0.4.10-linux-arm64.deb` |

Windows 运行安装程序；Debian/Ubuntu 使用 `sudo apt install ./安装包文件名.deb`。升级保留订阅和个人设置。

客户端更新与订阅更新依次尝试当前内核代理、系统代理和直连。下载期间可点击“取消下载”，之后可重新更新；进入安装阶段后不能通过该按钮取消。Windows 自动安装需要 UAC 授权，Linux 需要 `pkexec` 与桌面授权服务。

Linux 安装包在 Ubuntu 22.04 构建，需要满足 DEB 声明的依赖。WSL2 需要 WSLg；没有托盘宿主时，关闭窗口会退出。

可能造成 Windows Defender 误判查杀，需自行规避。

## 内存占用

使用 [内存基准程序](examples/memory_benchmark.rs)，在 Windows x64、0.4.5 Release 构建下，以实际 API 解析 50,000 条数据。每项独立进程运行 3 次，[原始结果](docs/benchmarks/memory-0.4.5.txt)。

| 数据 | 响应 JSON | 解析新增堆内存峰值 | 解析完成后新增堆内存 |
| --- | ---: | ---: | ---: |
| 50,000 条规则 | 3.33 MiB | 6.98–7.00 MiB | 6.11 MiB |
| 50,000 条连接 | 12.00 MiB | 24.20 MiB | 23.30 MiB |

统计扣除了测量前基线（规则 3.42 MiB、连接 12.08 MiB，含本地服务数据与运行时），解析完成时保留结果对象。这是 Rust 堆分配量，不包含 GUI、字体、渲染缓存、系统分配器额外开销或 mihomo 内存，不能代表客户端总内存占用。

## 编译与打包

安装 Rust stable、Python 3.11+，在对应系统和架构上原生构建，输出位于 `dist/`。

**Windows**：安装 Visual Studio C++ Build Tools（对应架构工具与 Windows SDK），执行：

```text
python scripts/build-installer.py
```

**Debian/Ubuntu**：安装构建依赖后执行：

```text
sudo apt install build-essential pkg-config dpkg-dev libglib2.0-dev libxkbcommon-dev libwayland-dev libx11-dev libx11-xcb-dev
python3 scripts/build-installer.py
```

资源已准备好可加 `--skip-prepare`；下载需要代理时可加 `--proxy http://127.0.0.1:7897`。Windows 使用 WSL 构建 Linux 包可执行 `python scripts/build-wsl.py --distro Ubuntu-22.04`，项目会同步到 Linux 文件系统编译。

## 开发与发布

```text
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python scripts/test-build-tools.py
```

CI 在 Windows、Ubuntu 的 x64 和 ARM64 环境分别检查、测试和打包。正式发布等待对应提交的四项 CI 全部通过，使用该次 Actions 产物作为 Release 附件。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生逻辑保留 **AGPL-3.0-only**，见 [授权及修改说明](vendor/region-restriction-check/SOURCE.md)；组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

mihomo 采用 GPL-3.0，Iced 采用 MIT。第三方软件与数据来源见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)。
