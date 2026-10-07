# 0.4.4 脚本依赖改造

运行时的系统集成与地区检测由 Rust 完成；开发、资源维护、测试及打包工具使用 Python。NSIS 保留为 Windows 安装器定义，固定上游 Bash 源码仅作为授权和迁移参考。

| 原有依赖或问题 | 0.4.4 实现 | 主要文件 |
| --- | --- | --- |
| PowerShell 准备内核及 Geo 资源 | Python 标准库下载、官方 SHA256 校验、失败保留旧文件，按 Windows/Linux 与 x64/arm64 分目录 | `scripts/prepare-resources.py`、`scripts/build_support.py` |
| PowerShell 编译和 NSIS 打包 | Python 调用 Cargo、验证版本和资源，使用新的文件清单暂存目录；Windows 输出 EXE 安装程序，Linux 输出 Debian 包，无 Shell 启动器或维护脚本 | `scripts/build-installer.py`、`scripts/build_deb.py` |
| PowerShell 安装器冒烟测试 | Python 直接调用 Win32 和注册表 API，在独立测试目录和同步对象中验证安装生命周期 | `scripts/test-installer.py` |
| 181 个平台检测调用 Bash、curl、jq、GNU 工具和 OpenSSL | 固定上游请求与判断迁移为原生 Rust 函数，使用 reqwest、serde_json、正则及 HMAC-SHA1 | `src/region_check.rs`、`src/region_check/generated.rs` |
| 检测工具链依赖本机 Git for Windows，安装包携带 MSYS 的 EXE/DLL | 删除工具链提取和运行脚本适配工具；安装资源仅携带 AGPL 许可证与来源说明 | 删除 `prepare-ip-check.py`、`adapt-region-check.py`；修改安装器文件清单 |
| 检测中的外部 DNS 辅助进程 | Tokio 原生 DNS 查询，删除应用专用 `--ip-check-resolve` 入口 | `src/region_check.rs`、`src/ip_check.rs`、`src/main.rs` |
| 检测子进程、进程树终止和临时响应文件 | 原生异步请求，取消时丢弃请求；响应大小和超时受限，Cookie 存放内存并按标准作用域发送 | `src/region_check.rs` |
| GNOME 代理读写调用 gsettings 命令 | 直接使用 GIO/GSettings API，保留原设置恢复机制 | `src/platform/gio.rs`、`src/platform/linux.rs` |
| KDE 缺少对应代理后端 | 维护 kioslaverc 的代理项并发送原生 D-Bus 刷新信号，保留其他配置和原值 | `src/platform/kde.rs`、`src/platform/gio.rs` |
| Linux 打开链接和目录调用 xdg-open 脚本 | GIO 默认应用 API | `src/platform/gio.rs`、`src/platform/linux.rs` |
| Linux 测试资源路径与 Windows 混用 | 按平台、架构寻找开发资源，集成测试统一读取覆盖参数或原生目录；Linux 用 geteuid 检查 root | `src/assets.rs`、`tests/core_integration.rs`、`src/platform/mod.rs` |
| CI 默认 Windows PowerShell 与系统路径差异 | Python 执行步骤，在 Windows/Ubuntu 分别构建、测试和打包；增加生成代码一致性检查与真实内核测试 | `.github/workflows/ci.yml` |
| WSL 从 Windows 挂载盘读取源码和构建文件 | Python 将源码及资源同步到 Linux 原生目录，Cargo 产物与缓存保留于 Linux，仅复制最终 DEB 与 SHA256 回 Windows | `scripts/build-wsl.py` |
| 旗帜资源维护工具写死本机代理 | 显式可选 `--proxy` 参数 | `scripts/prepare-flags.py` |
| 文档、许可声明引用旧运行工具 | 更新 Python 构建入口、Linux 依赖、原生检测来源和 AGPL 派生代码说明 | `README.md`、`resources/THIRD-PARTY-NOTICES.txt`、两处 `SOURCE.md` |
| 内存示例只打印峰值并展示历史提升比例 | 校验解析条数，同时输出基线、解析新增峰值与完成后保留量；README 仅记录实测占用 | `examples/memory_benchmark.rs`、`README.md` |

所有检测请求经过当前 mihomo 混合代理端口；认证头与 Cookie 不跨站重定向转发。三处上游变量拼写问题在生成时修正；生成器遇到不支持的结构会失败，普通应用构建使用已提交的 Rust 输出。

本地响应测试覆盖全部 181 个派生检测函数，以及允许/受限/畸形响应、浏览器验证、签名、强制代理、多步 Cookie 检测、跨站重定向、响应上限和取消。公开平台接口可能变化，本地测试不代表所有平台均已实网验证。Linux 原生 API 在隔离的内存设置后端和 D-Bus 会话中验证，实际 GNOME/KDE 桌面、托盘及 TUN 的完整行为仍需桌面环境验证。

内存测试记录于 README，范围为 API 解析新增 Rust 堆分配，不能代表完整 GUI 与 mihomo 的进程工作集。
