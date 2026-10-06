<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 **mihomo、Rust 和 Iced** 的原生代理客户端。目前支持 **Windows**，未来计划支持 **Linux**。

## Features

- **轻量、低内存占用**：原生桌面界面，占用内存远低于 WebView。
- **灵活代理**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点管理**：支持 YAML 订阅、删除与配置目录访问、策略组选择和批量延迟测试。
- **离线就绪**：安装包内置 mihomo 内核和 Geo 数据，启动无需额外下载，支持一键更新 Geo 数据。
- **便捷使用**：提供托盘、后台开机自启、单实例运行、连接与日志查看、网络诊断、网站连通性测试和主题切换。

## 如何使用

**推荐：使用安装包。** 在本仓库的 GitHub Releases 页面下载已编译、打包好的安装程序，运行安装程序即可。

**自行编译并打包（Windows x64）：** 先安装 Rust 的 MSVC 工具链，以及 Visual Studio Build Tools 中的“使用 C++ 的桌面开发”组件。下载仓库源码并解压，或克隆仓库后，在项目根目录打开 **PowerShell**，依次执行：

# 检查 Rust 工具链
rustup default stable-x86_64-pc-windows-msvc
rustc --version
cargo --version

# 编译并打包：自动准备内核、Geo 数据和 NSIS
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1

# 查看生成的安装包
Get-ChildItem .\dist\*-setup.exe

内核和 Geo 数据已准备好后，再次编译打包可跳过资源下载：

powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1 -SkipPrepare

## License

本项目采用 **GPL-3.0-only**，完整许可证见 [LICENSE](LICENSE)。

所附 mihomo 内核采用 GPL-3.0；Iced 采用 MIT。第三方软件与 Geo 数据的来源说明见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)，相应版权与许可证归原作者所有。
