<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 mihomo 的原生 Rust 代理客户端。支持 **Windows（x64 / ARM64）** 和 **Debian/Ubuntu（amd64 / ARM64）**。当前开发版本 **0.4.13-dev.1**，稳定版下载为 **0.4.12**。

## Features

- **轻量**：原生桌面界面，无需 WebView，页面数据按需加载。
- **代理模式**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点**：支持 YAML 订阅、策略组选择和批量延迟测试；订阅导入与更新自动尝试代理和直连。
- **离线就绪**：内置 mihomo 内核和 Geo 数据，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项，覆盖出口 IP、GitHub、AI 服务与流媒体地区访问；支持批量检测、取消、分类总结、组合筛选和结果详情。
- **便捷使用**：托盘、后台开机自启、单实例、连接与日志查看、网络诊断和主题切换。
- **客户端更新**：自动选择代理并回退，显示下载进度，支持取消下载；校验完成后自动安装与重启，失败时显示原因。

## 安装与使用

从 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 下载对应系统和架构的安装包，附带 `.sha256` 校验文件。

| 平台 | 安装包 |
| --- | --- |
| Windows x64 | [Clash-of-Rust-0.4.12-windows-x64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.4.12/Clash-of-Rust-0.4.12-windows-x64-setup.exe) |
| Windows ARM64 | [Clash-of-Rust-0.4.12-windows-arm64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.4.12/Clash-of-Rust-0.4.12-windows-arm64-setup.exe) |
| Debian/Ubuntu amd64 | [Clash-of-Rust-0.4.12-linux-amd64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.4.12/Clash-of-Rust-0.4.12-linux-amd64.deb) |
| Debian/Ubuntu ARM64 | [Clash-of-Rust-0.4.12-linux-arm64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.4.12/Clash-of-Rust-0.4.12-linux-arm64.deb) |

Windows 运行安装程序；Debian/Ubuntu 使用 `sudo apt install ./安装包文件名.deb`。升级保留订阅和个人设置。

可能造成 Windows Defender 误判查杀，需自行规避。

## 内存占用

`0.4.12` → `0.4.13-dev.1` 同条件对照：Ubuntu 22.04 x64、dev 构建，API 每项 50,000 条，三次独立进程取中位数。

| API 快照 | 保留堆内存：前 → 后 | 降低 |
| --- | ---: | ---: |
| 规则 | 6.11 → 3.13 MiB | 48.7% |
| 连接 | 23.30 → 12.37 MiB | 46.9% |
| 代理节点 | 21.09 → 8.98 MiB | 57.4% |

2,000 节点、40 组、20,000 规则的全匹配搜索场景中，GUI RSS 从 127.33 降到 79.05 MiB；GUI 与内核合计 PSS 从 215.50 降到 166.49 MiB。首页内存基本不变，解析耗时有所增加。以上是固定负载的 dev 测量，不能直接作为 Release 日常占用。

全部解析/刷新场景、耗时和原始记录见 [内存报告](docs/benchmarks/0.4.13-dev.1/README.md)。[统一基准](examples/memory_benchmark.rs)支持 API 堆内存及 GUI/内核 RSS、PSS、USS 采样：

```bash
cargo build --release --locked --example memory_benchmark -j 1
# 全部解析测试完成后，再采样已经打开的 GUI/内核
./target/release/examples/memory_benchmark all GUI_PID results
# 规则、连接、代理；流式/缓冲解析；单次/连续刷新；每项三次独立进程
./target/release/examples/memory_benchmark suite 50000 3 results.jsonl
# 正常打开客户端后传入 GUI PID，采样期间可切换页面、更新订阅或批量检测
./target/release/examples/memory_benchmark app GUI_PID 30 250 app-results.jsonl
```

输出文件使用独占创建，避免覆盖以前的结果。API 堆测量与进程 RSS/PSS/USS 属于不同口径；对比时应使用相同构建模式、输入和场景。

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

CI 只编译、测试并保存原生程序；手动 Publish Release 复用这些程序完成打包和安装测试后发布，支持只测试不发布。手动 Memory Benchmark 可测指定提交的内存。操作与产物说明见 [工作流文档](docs/workflows.md)，客户端内更新测试见 [更新测试说明](docs/automatic-update.md)。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生逻辑保留 **AGPL-3.0-only**，见 [授权及修改说明](vendor/region-restriction-check/SOURCE.md)；组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

mihomo 采用 GPL-3.0，Iced 采用 MIT。第三方软件与数据来源见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)。
