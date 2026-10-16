<p align="center">
  <img src="resources/icons/app.png" width="112" height="112" alt="Clash of Rust 图标">
</p>
<h1 align="center">Clash of Rust</h1>

基于 mihomo 的原生 Rust 代理客户端。支持 **Windows（x64 / ARM64）**、**Debian/Ubuntu（amd64 / ARM64，Ubuntu 仅支持 22.04 及以上）** 和 **macOS 14 及以上（Intel / Apple Silicon）**。版本 **0.5.11**，更新内容见 [发布说明](docs/releases/0.5.11.md)。

## Features

- **轻量**：Rust + Slint 原生桌面界面，无需 WebView，页面数据按需加载。
- **规则管理**：按订阅管理自定义规则，支持新增、编辑、启停与顺序调整，校验后应用；五项快捷规则默认收起且全部关闭，仅补充缺少的规则，保留原始订阅 YAML。
- **离线就绪**：内置 mihomo 内核和 Geo 数据，支持一键更新 Geo 数据。
- **IP 检测**：183 个检测项，覆盖出口 IP、GitHub、AI 服务与流媒体地区访问；一键检测显示分类报告，点击分类或状态查看详情，支持筛选与分类重测。
- **A÷检测**：采集 Claude 地区、DNS、WebRTC 与浏览器线索，扫描已记录项目配置并分类展示；可选一键保护，固定节点并启用 TUN、加密 DNS 与异常连接拦截，关闭后恢复原设置，详见 [检测说明](docs/claude-region-detection.md)。
- **便捷使用**：托盘快捷切换节点，可选显示上传与下载速度；支持后台开机自启、单实例与网络诊断，记住普通窗口尺寸；内核启动失败时可直接重试。
- **客户端更新**：仅检查正式版，自动选择代理并回退，支持下载进度与取消；校验完成后自动安装与重启，失败时显示原因。

## 安装与使用

从 [GitHub Releases](https://github.com/Fu021/Clash-of-Rust/releases) 下载当前正式版对应系统和架构的安装包，附带 `.sha256` 校验文件。

| 平台 | 安装包 |
| --- | --- |
| Windows x64 | [Clash-of-Rust-0.5.11-windows-x64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-windows-x64-setup.exe) |
| Windows ARM64 | [Clash-of-Rust-0.5.11-windows-arm64-setup.exe](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-windows-arm64-setup.exe) |
| Debian / Ubuntu 22.04+ amd64 | [Clash-of-Rust-0.5.11-linux-amd64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-linux-amd64.deb) |
| Debian / Ubuntu 22.04+ ARM64 | [Clash-of-Rust-0.5.11-linux-arm64.deb](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-linux-arm64.deb) |
| macOS Intel | [Clash-of-Rust-0.5.11-macos-x64.dmg](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-macos-x64.dmg) |
| macOS Apple Silicon | [Clash-of-Rust-0.5.11-macos-arm64.dmg](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.5.11/Clash-of-Rust-0.5.11-macos-arm64.dmg) |

Windows 运行安装程序；Debian/Ubuntu（Ubuntu 仅支持 22.04 及以上）使用 `sudo apt install ./安装包文件名.deb`。升级保留订阅和个人设置；Windows 卸载可选删除全部配置和数据，默认不勾选。

macOS 打开 DMG，将 **Clash of Rust.app** 拖入 **Applications（应用程序）** 后启动。系统代理和 TUN 共用本机网络助手，首次安装或助手需要更新时请求管理员授权，之后复用；正常启动、切换模式和退出恢复代理无需重复授权。登录自启以普通用户后台运行，开关自启无需管理员授权。当前 Mac 安装包使用 ad-hoc 签名，尚未进行 Developer ID 签名和 Apple 公证；首次打开若提示无法验证开发者，可确认来源及校验值后，在“系统设置 → 隐私与安全性”中选择“仍要打开”，见 [Apple 说明](https://support.apple.com/zh-cn/102445)。

可能造成 Windows Defender 误判查杀，需自行规避。

## 内存占用

实测版本 **0.5.11-dev.1**（`5b09faf`）；Ubuntu 22.04 x64、Release 构建、Slint 软件渲染，低强度固定输入为 **100 节点、5 个策略组、1,000 条规则**。DNS 覆盖、IPv6 与 UDP 开启，Claude 保护、快捷规则、系统代理和 TUN 关闭，无业务流量：

| 场景 | GUI RSS | GUI 与内核合计 PSS |
| --- | ---: | ---: |
| 首页 | 43.46 MiB | 81.67 MiB |
| 代理组折叠 | 44.36 MiB | 86.34 MiB |
| 节点搜索 | 53.07 MiB | 97.51 MiB |
| 返回首页 | 53.14 MiB | 97.60 MiB |

五项快捷规则全开时，首页 GUI RSS 为 43.43 MiB，合计 PSS 为 99.07 MiB。以上为每场景采样 10 秒、间隔 250 ms 的观测峰值，实际占用随系统、配置和使用情况变化；RSS 含共享页，PSS 按比例分摊共享页。[测试记录](https://github.com/Fu021/Clash-of-Rust/actions/runs/38878584013)。

## 编译与打包

[GitHub Actions](https://github.com/Fu021/Clash-of-Rust/actions) 覆盖六种系统与架构的编译、静态检查、单元测试与系统代理/TUN 测试；界面可使用 Slint Viewer 独立预览，无需编译桌面程序。

测试安装包：等待对应提交的 CI 成功，手动运行 **Publish Release**，填写 `ci_run_id` 并勾选 `check_only`；安装包保存在 Actions 附件中。打包直接复用 CI 产物，不再编译。正式发布使用同一工作流，详见 [工作流说明](docs/workflows.md)。

## 开发与发布

项目使用 Cargo workspace：`crates/client-core` 保存共享配置、订阅、规则、检测、实时保护、Geo 更新和代理切换策略；`crates/client-ui` 保存 Slint 页面、共用控件与增量列表模型；`apps/desktop/src/application` 负责应用状态、任务调度和界面绑定，`apps/desktop` 的其他模块负责桌面内核管理及系统集成。共享库不依赖桌面应用，通知、系统代理等操作由各平台提供实现；移动端后续复用同一套业务逻辑。Android、iOS、移动桥接与移动内核目录目前仅占位。CI 统一编译整个工作区；资源与打包输出统一放在仓库根目录。

Windows 使用按需启动的网络服务承接 TUN 权限，首次安装或修复时授权，GUI 保持普通权限；正常重启后复用服务注册，空闲服务自动退出。

界面预览无需编译桌面程序，使用持久安装的 Slint Viewer：

```text
python3 scripts/preview-slint.py --dialogs --pairs
```

保留配置、规则、检测解析、状态恢复、内存边界和界面交互的源码单元测试；真实系统集成测试只覆盖系统代理与 TUN。旧版 Shell 参考实现已移除，平台检测直接维护原生 Rust。

macOS 的自动检查与人工验收范围见 [测试清单](docs/macos-ci-checklist.md)。Memory Benchmark 按需手动运行，保存采样数据与报告。操作与产物说明见 [工作流文档](docs/workflows.md)，客户端内更新测试见 [更新测试说明](docs/automatic-update.md)。

## License

Rust 界面及原创通用代码采用 **GPL-3.0-only**，见 [LICENSE](LICENSE)。IP 检测复用的 RegionRestrictionCheck 源码及派生逻辑保留 **AGPL-3.0-only**，见 [授权及修改说明](resources/ip-check/SOURCE.md)；组合发行遵守 GPLv3/AGPLv3 第 13 条，包括适用时向远程网络用户提供完整对应源码。

mihomo 采用 GPL-3.0，Slint 按 GPL-3.0 使用。第三方软件与数据来源见 [THIRD-PARTY-NOTICES.txt](resources/THIRD-PARTY-NOTICES.txt)。
