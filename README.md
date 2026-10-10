<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 mihomo 的原生 Rust 代理客户端。支持 **Windows（x64 / ARM64）** 和 **Debian/Ubuntu（amd64 / ARM64）**。版本 **0.5.1**，修复 Windows 开机自启权限与错误提示，改善配置保存及恢复；更新内容见 [发布说明](docs/releases/0.5.1.md)。

## Features

- **轻量**：原生桌面界面，无需 WebView，页面数据按需加载。
- **代理模式**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点**：支持 YAML 订阅、策略组选择、批量延迟测试，以及延迟升序、降序和名称排序；订阅导入与更新自动尝试代理和直连。
- **附加规则**：五项广告拦截与本地、国内直连规则，默认全部关闭；只补充订阅缺少的规则，保留原有规则与兜底。
- **离线就绪**：内置 mihomo 内核和 Geo 数据，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项，覆盖出口 IP、GitHub、AI 服务与流媒体地区访问；支持批量检测、取消、分类总结、组合筛选和结果详情。
- **便捷使用**：统一深浅主题、托盘、后台开机自启、单实例、连接与日志查看及网络诊断；内核启动失败时可直接重试。
- **客户端更新**：自动选择代理并回退，显示下载进度，支持取消下载；校验完成后自动安装与重启，失败时显示原因。

## 安装与使用

从 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 下载对应系统和架构的安装包，附带 `.sha256` 校验文件。

| 平台 | 安装包 |
| --- | --- |
| Windows x64 | [Clash-of-Rust-0.5.1-windows-x64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.1/Clash-of-Rust-0.5.1-windows-x64-setup.exe) |
| Windows ARM64 | [Clash-of-Rust-0.5.1-windows-arm64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.1/Clash-of-Rust-0.5.1-windows-arm64-setup.exe) |
| Debian/Ubuntu amd64 | [Clash-of-Rust-0.5.1-linux-amd64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.1/Clash-of-Rust-0.5.1-linux-amd64.deb) |
| Debian/Ubuntu ARM64 | [Clash-of-Rust-0.5.1-linux-arm64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.1/Clash-of-Rust-0.5.1-linux-arm64.deb) |

Windows 运行安装程序；Debian/Ubuntu 使用 `sudo apt install ./安装包文件名.deb`。升级保留订阅和个人设置。

可能造成 Windows Defender 误判查杀，需自行规避。

## 内存占用

参考实测版本 **0.5.0**（0.5.1 未重新测量）；Ubuntu 22.04 x64、Release 构建，附加规则关闭，固定输入为 2,000 节点、40 组和 20,000 条规则：

| 场景 | GUI RSS | GUI 与内核合计 PSS |
| --- | ---: | ---: |
| 首页 | 46.65 MiB | 97.95 MiB |
| 代理组折叠 | 50.39 MiB | 137.44 MiB |
| 节点搜索 | 55.15 MiB | 142.38 MiB |

五项附加规则全开时，首页 GUI RSS 为 48.77 MiB，合计 PSS 为 125.94 MiB。以上为该次测试的观测峰值，实际占用随配置和使用情况变化；RSS 含共享页，PSS 按比例分摊共享页。[测试记录](https://github.com/Fu021/Clash-of-Rust/actions/runs/38034324607)。

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
cargo clippy --release --all-targets --locked -- -D warnings
cargo test --release --all-targets --locked -- --test-threads=1
python scripts/test-build-tools.py
python scripts/test-workflow-artifacts.py
```

开发分支通过 PR 运行四平台 CI，完成编译、测试并保存原生程序；手动 Publish Release 复用 main 的成功 CI 产物，完成打包和安装测试后发布，支持只测试不发布。手动 Memory Benchmark 可测指定提交的内存，并可附加实际界面预览。操作与产物说明见 [工作流文档](docs/workflows.md)，客户端内更新测试见 [更新测试说明](docs/automatic-update.md)。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生逻辑保留 **AGPL-3.0-only**，见 [授权及修改说明](vendor/region-restriction-check/SOURCE.md)；组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

mihomo 采用 GPL-3.0，Iced 采用 MIT。第三方软件与数据来源见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)。
