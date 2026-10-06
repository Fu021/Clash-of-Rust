# Clash of Rust 开发交接资料

更新于 2026-10-06，基线版本 **0.4.0 开发预览版**。
仓库：<https://github.com/Fu021/Clash-of-Rust>。这是可随仓库迁移的项目资料库；新 Codex 会话应先阅读根目录 `AGENTS.md` 和本文。

## 目标与现状

Rust 2024 + Iced 0.14 + mihomo 的原生桌面代理客户端，追求轻量、低内存占用。渲染使用 tiny-skia，不使用 WebView，也没有启用 wgpu 后端。Windows 为当前开发平台；Linux 仅有部分适配和 CI 编译检查，尚未发布可用安装包。

Windows 用户曾反馈系统代理正常、客户端约 7.8 MB 内存；这是用户特定环境的观察，不是可重复的跨平台性能保证。0.4.0 标记为 prerelease，明确“不保证任何功能”。

## 源码职责

| 路径 | 职责 |
| --- | --- |
| `src/main.rs` | Iced 页面、消息调度、窗口、托盘交互、自动启动、提权交接 |
| `src/engine.rs` | 内核生命周期、订阅、配置热加载/重启、模式恢复、日志、Geo 更新 |
| `src/config.rs` | 设置与订阅存储、原子写入、运行配置隔离 |
| `src/api.rs` | mihomo 控制 API、连接/规则/策略组、并发测速 |
| `src/proxy.rs` | 系统代理 / TUN / 关闭互斥切换及失败回退 |
| `src/platform/` | OS 代理恢复日志；Windows 注册表/UAC/单实例；Linux GNOME/XDG 初步适配 |
| `src/tray.rs` | 原生托盘线程、菜单、模式勾选与状态图标 |
| `src/assets.rs` | 安装资源发现、Geo 校验、事务替换与异常恢复 |
| `src/probe.rs` | 网络诊断、站点延迟、目标网站报告的地区 |
| `src/typography.rs`、`src/presentation.rs` | 字体回退、字号与可读时间/流量格式 |
| `resources/` | 默认 YAML/设置、透明图标、第三方来源说明 |
| `installer/`、`scripts/` | NSIS 安装、资源准备、打包、图标维护与发布 |
| `tests/`、`.github/workflows/ci.yml` | 自动测试、需明确启用的真实环境测试、Windows/Ubuntu CI |

`Cargo.lock` 必须提交。`target/`、`bin/`、`bundle/`、`dist/`、`tools/` 被忽略；不要将 Windows 构建缓存复制到 Linux。图标源 PNG 和已生成的 ICO/RGBA 都需保留，普通编译不依赖 Pillow。

## 已确定的产品行为

- 九页：首页、代理、订阅、连接、规则、日志、网络诊断、网站连通性、设置。
- 默认配置为本地“默认直连”，首次启动自动启动内核；本地订阅不显示更新按钮。远程订阅支持直接更新和通过代理更新。
- 运行模式为规则 / 全局 / 直连；代理模式为系统代理 / TUN / 关闭，互斥且可直接切换。当前选择不可重复点击；托盘菜单显示勾选。
- 0.4.0 设置新增 `run_mode` 与 `proxy_mode`，选择成功后原子保存。旧设置缺少字段时默认规则 + 关闭。退出清理和临时重启不覆盖偏好；重启恢复运行模式，健康检查后恢复代理模式。
- Windows 恢复 TUN 时可能弹 UAC；取消会报错且保留偏好。本次修正提权交接的重复启动，实际 UAC 行为仍需用户复测。后台启动标志在提权时保留；普通开机自启仍无法绕过 UAC。
- 系统代理恢复以所有权日志为依据，只恢复本程序仍拥有的值；其他程序后来改过的配置不能被覆盖。
- 关闭窗口默认入托盘；退出会恢复 OS 代理、关闭 TUN 和内核。Windows 第二次启动唤起已有窗口；包括普通启动唤起提权实例。
- 默认窗口 950×700，最小 800×450。设置可输入尺寸、拖动同步、恢复默认；尺寸当前不跨会话保存。
- 控制 API 默认 127.0.0.1:9090，混合端口 7897；随机控制密钥存储在用户设置。端口保存会自动重启运行内核。
- 页面可见时每秒刷新，隐藏时每五秒检查内核健康；避免后台刷新改变操作按钮布局。连接/规则不分页，策略组节点按需展开；大型策略组仍分页。
- 节点延迟：绿 <300 ms，黄 300–999 ms，红 ≥1000 ms/超时。定时测速默认五分钟，0 关闭；GUI 测速不替代内核 URLTest 的自动选路。
- 网站卡片：ChatGPT、Gemini、Claude、GitHub、YouTube、Netflix。地区必须是目标网站提供的信息；没有可靠证据时显示网站未提供地区，不能用 IP 地理位置代替。
- 字体固定缩放，无字号滑块；Windows 中文微软雅黑、英文 Times New Roman；Linux 有 Noto Sans CJK SC / Liberation Serif 回退。按钮和同行文本使用垂直居中。深色主题文本白色，保留绿色强调、延迟色和浅色输入提示。
- 图标只有透明蓝/橙/绿：蓝色默认、橙色系统代理、绿色 TUN。安装快捷方式引用带版本号的蓝色 ICO 避免旧缓存；黑白图标已废弃。
- README 面向使用者，仅保留简介、Features、使用方法、License；不将 README 打包进安装程序。

## 资源与数据

Windows 打包使用 mihomo **v1.19.32**；安装包包含 `mihomo.exe`、`GeoIP.dat`、`GeoSite.dat`、`Country.mmdb`、`ASN.mmdb`、默认 YAML 和来源/校验清单。
应用启动不下载内核或 Geo；只有用户点击更新 Geo 才下载，校验全部文件后事务替换。NSIS 升级保留用户数据，确认升级后自动关闭现有实例，结束页快捷方式和立即启动默认勾选。

用户数据由 `directories::ProjectDirs` 决定，可用 `CLASH_OF_RUST_DATA_DIR` 覆盖：包含 `settings.json`、`profiles.json`、`profiles/`、`runtime/`、代理恢复日志和实例锁。不要提交或发布真实订阅与密钥。迁移源码不要求迁移个人数据；跨 OS 默认配置的来源绝对路径可能过时，需 Linux 工作检查。

## Ubuntu 接手：先做什么

首先克隆仓库并阅读本文，再检查 GitHub Actions 状态。Windows 本地通过不代表 Ubuntu CI 已通过。上一版 Windows CI 的新版 Clippy `question_mark` 检查失败已在 0.4.0 修复；仍以新提交的实际 CI 结果为准。
以下构建依赖列表参考现有 Ubuntu CI，需按实际 Ubuntu 版本补充；安装稳定版 Rust 后执行：

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libxkbcommon-dev libwayland-dev libx11-dev libx11-xcb-dev fonts-noto-cjk fonts-liberation
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

构建后还不能直接当成完整安装程序运行：当前资源准备脚本只下载 Windows 内核，需要新增 Linux 架构匹配的资源准备/打包流程。
Linux 运行时查找可执行文件旁的 `resources/mihomo`，需可执行权限、四份 Geo 文件和有效清单。调试构建支持回退到 `bundle/resources`，release 不回退。

按优先级继续：

1. 补 Linux 资源准备和安装布局，内置内核/Geo；不要启动后自动下载。支持 amd64 后再考虑 arm64。
2. 实测 GNOME 系统代理的设置/恢复、Wayland/X11 GUI 和 StatusNotifier 托盘。Ubuntu 默认 GNOME 是否显示托盘取决于桌面环境/扩展；托盘失败时关闭窗口必须可靠退出。
3. 实现 Linux 单实例与窗口唤起。目前非 Windows `application_guard` / show / exit IPC 没有等价实现，只有数据目录实例锁。
4. 实现 Linux TUN 权限方案（例如特权辅助进程/PolicyKit，具体设计待讨论）。目前非 Windows `is_elevated()` 返回 true 是占位，不能视为实际权限检查，也不应让整个 GUI 长期 root 运行。
5. 检查 XDG 开机自启、后台窗口恢复、字体、DPI、订阅来源路径及 Linux 文件权限；当前系统代理仅 GNOME，KDE 未实现。
6. 确定 Linux 安装格式、资源目录发现、升级/卸载清理和桌面快捷方式；同时回归 Windows。

## 验证与发布

普通自动测试使用临时目录/mock API，不修改当前 OS 代理。真实内核、在线 Geo、原生托盘测试是显式 opt-in。0.4.0 另有 `remembered_modes_survive_core_and_client_restart`，使用临时端口/数据验证退出后恢复全局模式，仅代理关闭，不改 OS 网络设置。
GUI、UAC、系统代理、TUN、安装/升级效果由用户复测，未经实际验证应保留限制说明。

Windows PowerShell 打包：`powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-installer.ps1`；已有离线资源时加 `-SkipPrepare`。有网络问题可传 `-Proxy http://127.0.0.1:7897`，这是原开发机代理，迁移后不要假定存在。

Release 附件放安装包及 `.sha256`，源码通过 tag 提供。0.4.0 说明保存在 `docs/releases/0.4.0.md`，必须标记 GitHub prerelease。后续发布需用户明确授权。
`scripts/publish-release.py` 使用 Python 标准库及本机 Git 凭据，先创建草稿、上传并检查附件 SHA-256，再发布；不打印或写入凭据。使用前需已有 GitHub 登录和仓库权限，可先运行 `python3 scripts/publish-release.py --check`。
项目 GPL-3.0-only，mihomo GPL-3.0，Iced MIT；保留 LICENSE 与第三方声明。
