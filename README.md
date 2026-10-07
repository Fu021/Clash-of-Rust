<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 **mihomo、Rust 和 Iced** 的原生代理客户端。当前版本 **0.4.5**，提供 **Windows x64** 的 `.exe` 安装程序和 **Linux amd64** 的 `.deb` 安装程序。安装包内置内核与 Geo 数据；应用运行无需 Python、PowerShell、Bash 或 WebView。

## Features

- **轻量、低内存占用**：原生桌面界面，无需 WebView；页面数据按需加载，日志与后台缓冲设有容量限制。
- **中英文字体**：中文使用内嵌的思源黑体（Source Han Sans CN），英文与数字在 Windows 使用 Segoe UI、Linux 使用 Noto Sans 或系统回退字体，支持中英文混排和输入框，无需额外安装中文字体。
- **灵活代理**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点管理**：支持 YAML 订阅、删除与配置目录访问、策略组选择和批量延迟测试。
- **离线就绪**：安装包内置 mihomo 内核和 Geo 数据，启动无需额外下载，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项全部由 Rust 直接请求和解析，其中 181 项采用 RegionRestrictionCheck 的检测逻辑；支持出口 IP、GitHub 连通性、AI 服务与流媒体平台地区访问检测，提供一键检测、取消及结果搜索。
- **便捷使用**：提供托盘、后台开机自启、单实例运行、连接与日志查看、网络诊断和主题切换。
- **更新检测**：启动时及每 6 小时检查 GitHub Release，发现新版本时提示；支持手动检查和打开 Release 页面下载更新。

## 内存与运行效率

- 连接和规则分页展示，搜索覆盖完整列表；非当前页面的数据在切页和隐藏窗口时释放。
- 大体积 API 响应边接收边解析，减少原始 JSON 和解析结果同时驻留的峰值；API 响应上限为 32 MiB，超出限制会提示错误。
- 减少 YAML 配置深拷贝、重复解析和轮询时的订阅数据复制，及时释放启动与重载过程中不再需要的缓冲区。
- 日志保留最多 500 条、每条最多 1000 字符；限制日志帧与配置校验输出，托盘只保留最新状态，清理已失效的节点与策略组缓存。

使用 [内存基准程序](examples/memory_benchmark.rs)，于 2026-10-08 在 Windows x64、0.4.5 Release 构建下测量。每项使用独立进程运行 3 次，以本地 HTTP 服务返回 50,000 条数据，经实际 `Api::get` 流式解析，并校验解析条数。[原始结果](docs/benchmarks/memory-0.4.5.txt)。

| 数据 | 响应 JSON 大小 | 解析新增堆内存峰值 | 解析完成后新增堆内存 |
| --- | ---: | ---: | ---: |
| 50,000 条规则 | 3.33 MiB | 6.98–7.00 MiB | 6.11 MiB |
| 50,000 条连接 | 12.00 MiB | 24.20 MiB | 23.30 MiB |

统计由计数分配器记录的 Rust 堆分配量，扣除了测量前基线（规则 3.42 MiB、连接 12.08 MiB，含本地服务响应数据与运行时）。解析完成时保留结果对象。该测量不包含完整 GUI、字体、渲染缓存、系统分配器额外开销或 mihomo 的进程内存，不能代表客户端总工作集；结果对象的占用随数据量增长。

复测时从项目根目录执行，每次命令启动一个独立进程：

```text
cargo run --release --locked --example memory_benchmark -- rules streamed 50000
cargo run --release --locked --example memory_benchmark -- connections streamed 50000
```

## 如何使用

### 使用安装包（推荐）

在 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 页面选择已发布版本。各安装包同时提供 `.sha256` 校验文件；发行附件仅提供 Windows EXE、Linux DEB 及其校验文件。

| 平台 | 0.4.5 安装包 | 安装方式 |
| --- | --- | --- |
| Windows x64 | `Clash-of-Rust-0.4.5-windows-x64-setup.exe` | 运行安装程序，按提示安装 |
| Debian/Ubuntu amd64 | `Clash-of-Rust-0.4.5-linux-amd64.deb` | `sudo apt install ./Clash-of-Rust-0.4.5-linux-amd64.deb` |

升级时保留订阅和个人设置。Windows Defender 可能误判查杀，需自行规避。Linux amd64 包在 Ubuntu 24.04 构建，其他 Debian 系发行版需要满足 DEB 声明的依赖；GNOME/KDE 桌面集成及 Linux TUN 仍需在对应环境验证。

WSL2 需要 WSLg 图形支持，应用自动使用 X11/XWayland。没有托盘宿主时，关闭窗口会退出，`--background` 启动也会显示窗口。

### 自行编译并打包（Windows x64）

安装 Rust stable（MSVC）、Python 3.11+ 和 Visual Studio Build Tools 的“使用 C++ 的桌面开发”组件（含 Windows SDK），在项目根目录执行：

```text
python scripts/build-installer.py
```

首次构建自动准备内核、Geo 数据和 NSIS，内核与 Geo 数据按 SHA256 校验。资源已准备好可加 `--skip-prepare`；下载需要代理时可加 `--proxy http://127.0.0.1:7897`。输出为 `dist/` 下的 EXE 安装程序及 `.sha256`。

### Linux 原生构建与 Debian 打包

安装 Rust stable 和 Python 3.11+，在 Debian/Ubuntu 的项目根目录执行：

```text
sudo apt install build-essential pkg-config dpkg-dev libglib2.0-dev libxkbcommon-dev libwayland-dev libx11-dev libx11-xcb-dev
python3 scripts/build-installer.py
```

输出为 `dist/` 下对应主机架构的 DEB 安装程序及 `.sha256`，资源已准备好可加 `--skip-prepare`。

Windows 使用 WSL 构建时，先在 WSL 内安装上述工具与依赖，再从 Windows 项目根目录执行：

```text
python scripts/build-wsl.py --distro Ubuntu-24.04
```

脚本自动同步到 WSL 的 Linux 文件系统编译，并将 DEB 与校验文件复制回 Windows 的 `dist/`。

### 开发校验

```text
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python scripts/test-build-tools.py
python scripts/test-installer.py
```

最后一项仅适用于 Windows。Linux 包检查使用 `sudo python3 scripts/test-deb.py dist/Clash-of-Rust-0.4.5-linux-amd64.deb`；WSLg/X11 图形测试使用 `dbus-run-session -- python3 scripts/test-linux-gui.py --deb dist/Clash-of-Rust-0.4.5-linux-amd64.deb`。CI 在 Windows/Linux 分别执行构建、测试和打包。

## 项目目录

| 目录 | 内容 |
| --- | --- |
| `src/` | Rust 界面、内核管理、平台集成及原生地区检测 |
| `resources/` | 内嵌字体、图标、旗帜、默认配置和来源说明 |
| `vendor/` | 固定上游检测参考源码、生成输入、数据与许可证 |
| `scripts/` | Python 资源维护、构建、打包、测试和发布工具 |
| `installer/` | NSIS 安装器与隔离测试用的旧版安装器定义 |
| `tests/`、`examples/` | 回归测试、内存基准程序和示例配置 |
| `docs/` | 改造记录、目录清理记录与各版本 Release 文案 |
| `bundle/` | 已校验的本地内核和 Geo 数据、构建后的应用，Git 忽略 |
| `target/`、`tools/`、`bin/` | Cargo 缓存、NSIS 工具和资源下载缓存，Git 忽略 |
| `dist/` | 当前安装包与校验文件，Git 忽略 |

历史改造明细见 [脚本依赖改造记录](docs/script-migration-0.4.4.md)，本次目录整理见 [0.4.5 整理记录](docs/project-cleanup-0.4.5.md)，本版 Release 文案见 [0.4.5](docs/releases/0.4.5.md)。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，完整许可证见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生 Rust 检测逻辑保留 **AGPL-3.0-only**，见 [RegionRestrictionCheck 授权及修改说明](vendor/region-restriction-check/SOURCE.md) 和该目录的完整许可证。组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

所附 mihomo 内核采用 GPL-3.0；Iced 采用 MIT。第三方软件与 Geo 数据的来源说明见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)，相应版权与许可证归原作者所有。
