<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 **mihomo、Rust 和 Iced** 的原生代理客户端。目前支持 **Windows**，未来计划支持 **Linux**。

## Features

- **轻量、低内存占用**：原生桌面界面，无需 WebView；API 数据流式解析，连接与规则每页最多显示 60 条，切换页面或隐藏窗口时释放非当前页面数据，日志与后台缓冲设有容量限制。
- **中英文字体**：中文使用内嵌的思源黑体（Source Han Sans CN），英文与数字使用 Windows 提供的 Segoe UI，支持中英文混排和输入框，无需额外安装中文字体。
- **灵活代理**：支持系统代理、TUN，以及规则、全局和直连模式。
- **订阅与节点管理**：支持 YAML 订阅、删除与配置目录访问、策略组选择和批量延迟测试。
- **离线就绪**：安装包内置 mihomo 内核和 Geo 数据，启动无需额外下载，支持一键更新 Geo 数据。
- **IP 检测**：集成 RegionRestrictionCheck，支持出口 IP、GitHub 连通性、AI 服务与流媒体平台地区访问检测；提供一键检测、取消及结果搜索。
- **便捷使用**：提供托盘、后台开机自启、单实例运行、连接与日志查看、网络诊断和主题切换。

0.4.1 更新包含字体更换、IP 检测与内存优化，详见 [0.4.1 发布说明](docs/releases/0.4.1.md)。

## 如何使用

### 使用安装包（推荐）

在 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 页面选择版本，下载 `Clash-of-Rust-版本号-windows-x64-setup.exe`，运行并按照提示完成安装。

安装包已包含 mihomo 内核、Geo 数据和默认配置，无需自行编译或另行下载这些资源。

启动后在“订阅”页添加 mihomo YAML 配置或订阅地址，再到“代理”页选择节点，在首页选择运行模式与代理模式。开启 TUN 时会请求管理员权限；关闭窗口后默认保留在托盘，可从托盘显示窗口或退出。

目前为开发预览版。IP 检测遵循当前代理规则；平台验证拦截、接口变化及策略组出口变化可能影响结果，检测通过也不代表已验证账号登录或实际播放。

### IP 检测

启动内核后，打开“IP检测”页，点击“一键检测”即可检测全部项目；检测过程中可取消。页面展示检测结果、地区和响应耗时，支持按平台、地区或结果搜索。检测项目覆盖 ChatGPT、Claude、Gemini、Netflix、Disney+、YouTube 等服务及多个地区的流媒体平台。

检测通过当前代理进行，实际出口由当前规则与策略组决定。出口 IP 查询显示当前出口地址；其他平台的地区信息以该平台返回的证据为准。

### 内存与运行效率

- 连接和规则分页展示，搜索覆盖完整列表；非当前页面的数据在切页和隐藏窗口时释放。
- 大体积 API 响应边接收边解析，减少原始 JSON 和解析结果同时驻留的峰值；API 响应上限为 32 MiB，超出限制会提示错误。
- 减少 YAML 配置深拷贝、重复解析和轮询时的订阅数据复制，及时释放启动与重载过程中不再需要的缓冲区。
- 日志保留最多 500 条、每条最多 1000 字符；限制日志帧与配置校验输出，托盘只保留最新状态，清理已失效的节点与策略组缓存。

以 50,000 条数据进行本地基准测试，API 解析阶段的新增 Rust 堆峰值：规则从 9.44 MiB 降至 6.98 MiB（约 26%），连接从 35.30 MiB 降至 24.20 MiB（约 31%）。测量不包含整个客户端、字体与渲染缓存或 mihomo 的工作集，结果对象的内存仍随数据量增长。可使用 [内存基准程序](examples/memory_benchmark.rs) 复测。

## 自行编译并打包（Windows x64）

先安装 Rust、近期版本的 Git for Windows（默认路径 `C:\Program Files\Git`）、Python 3，以及 Visual Studio Build Tools 中的“使用 C++ 的桌面开发”组件（包含 MSVC 和 Windows SDK）。Python 安装时勾选加入 PATH。下载仓库源码并解压，或克隆仓库后，在**项目根目录打开 PowerShell**，依次执行以下命令。

**检查 Rust 工具链**

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustc --version
cargo --version
```

**编译并生成安装包**

首次打包会自动准备 mihomo 内核、Geo 数据、IP 检测工具和 NSIS，需要联网。IP 检测运行工具从本机 Git for Windows 提取最小依赖，安装包使用者无需安装 Git 或 Python。

如果 Git 安装在自定义目录，先执行 `python .\scripts\prepare-ip-check.py --git-root "你的 Git 安装目录"`。中文字体、图标和地区旗帜已包含在源码中，正常编译不需要重新生成。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1
```

下载遇到网络问题时，可在命令末尾添加 `-Proxy http://127.0.0.1:7897`，地址和端口按本机代理修改。

**查看生成的安装包**

```powershell
Get-ChildItem .\dist\*-setup.exe
```

生成的安装包位于项目根目录下的 `dist` 文件夹，可直接运行安装。

**再次编译（跳过资源准备）**

内核和 Geo 数据已准备好后，再次编译打包可跳过资源下载：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1 -SkipPrepare
```

## License

Rust 界面及桥接代码采用 **GPL-3.0-only**，完整许可证见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 适配脚本保留 **AGPL-3.0**，见 [RegionRestrictionCheck 授权及修改说明](vendor/region-restriction-check/SOURCE.md) 和该目录的完整许可证。组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

所附 mihomo 内核采用 GPL-3.0；Iced 采用 MIT。第三方软件与 Geo 数据的来源说明见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)，相应版权与许可证归原作者所有。
