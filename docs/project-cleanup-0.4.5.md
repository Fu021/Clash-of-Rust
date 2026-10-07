# 0.4.5 项目目录整理

## 清理内容

| 位置 | 清理内容 | 原因 |
| --- | --- | --- |
| `dist/` | 0.4.3、0.4.4 安装包、校验文件和临时验证结果 | 本地发行目录只保留 0.4.5 的 EXE、DEB 与 SHA256 |
| `tools/linux-toolchain/` | 引导脚本、Rust 安装归档、解压文件和 Windows 盘上的 Cargo 缓存 | WSL 已使用 Linux 文件系统中的工具链和构建镜像 |
| `tools/` | 临时 WSL 诊断、进程清理及验证结果更新脚本 | 调试已结束；可复用的图形验证迁入 `scripts/test-linux-gui.py` |
| `target/linux/` | 曾在 Windows 挂载盘生成的 Linux 构建缓存 | Linux 构建移到 WSL 原生文件系统 |
| `target/optimization-baseline/` | 内存优化前的源码副本 | README 只记录实际内存占用，参考副本不参与构建 |
| `target/flycheck0/`、`target/tmp/`、`target/package-staging/` | 编辑器及打包暂存目录 | 均可重新生成 |
| `bin/` | 已解压且校验过的内核下载归档 | `bundle/` 保留已校验的内核与 Geo 数据 |
| `bundle/resources/ip-check/` | 旧 Bash/MSYS 工具链、适配脚本和过时运行说明 | 原生检测不再需要；仅保留许可证和来源说明 |
| Python 缓存 | `scripts/__pycache__/` 等本地字节码缓存 | 不属于项目源码 |

## 保留内容

- Rust 源码、测试、内存基准程序、示例与历史 Release 文案。
- 内嵌字体、图标原稿及打包工具、旗帜、服务清单和默认配置。
- `vendor/region-restriction-check/` 的固定上游参考、生成输入、Cookie/IATA 数据和完整许可证。生成器与 Rust 的 `include_str!` 仍使用这些文件。
- `installer/legacy-test.nsi`，用于验证旧版安装升级和用户文件保留。
- 已校验的 Windows/Linux 内核与 Geo 数据、NSIS 编译器、当前 Windows Cargo 编译缓存，支持离线增量构建。
- Git 历史、仓库配置和既有源码修改。

源码未包含运行时 PowerShell、Bash 或 Python 依赖。安装资源使用明确的文件清单，发行附件为 Windows EXE、Linux DEB 及其 SHA256。WSL 构建目录、Cargo 缓存和 Rust 工具链留在 Linux 文件系统。
