# Linux 扩展兼容验证

每个发行版有独立 `workflow_dispatch` 入口，指定同提交成功的 `CI` 运行编号 `run_id`。普通 push/PR 不启动这些 VM；CI 一次编译并生成 DEB，Linux x64 job 另从同一候选组装 RPM。发布时按 [Linux 范围](release.md)要求相应回执。本页描述测试目标，不是兼容通过或可下载声明。

| 手动入口 | 固定来宾（x64） | 安装与桌面 |
| --- | --- | --- |
| Pop!_OS Compatibility | System76 22.04 build 58、24.04 build 20 | 离线 `distinst` 后从系统盘启动；DEB；分别验证 Pop GNOME 和 COSMIC |
| Zorin OS Compatibility | Zorin 17.3 Core r2、18.1 Core | 离线 Ubiquity；DEB；Zorin GNOME 与发行版原生托盘 |
| Fedora Compatibility | Fedora Cloud Base Generic 44-1.7、45-1.4 | DNF 安装同源 RPM；GNOME Wayland |
| openSUSE Compatibility | Leap 16.0 Build 18.95、16.1 Build 3.18；Tumbleweed 20261117 | 可写 Cloud VM，Zypper 安装同源 RPM；GNOME Wayland |
| Arch Linux Compatibility | 官方 Cloud 20261115.625457 | `makepkg` 由同源 DEB 或固定的升级夹具还原安装目录；`pacman` 验收；GNOME Wayland |

镜像 URL、完整摘要与架构由 `scripts/linux_compatibility_targets.py` 固定并校验，滚动版使用相应快照仓库。Tumbleweed 镜像直接从 openSUSE 官方内容源取固定快照，避免镜像跳转指向尚未同步该文件的站点。新工作流在固定 `ubuntu-24.04` 托管 runner 上运行所需的 QEMU 来宾。COSMIC live ISO 的独立启动诊断保留，直到安装版取得等价的真实登录和失败取证证据。

排查时先对照各目标 `linux-vm.json` 的 `failed_stage`，区分输入下载、来宾启动、源码传入、依赖准备和已安装候选验收。宿主工具安装失败看之前独立的 APT 步骤。来宾已启动时，报告同时记录内核与 KVM/TCG。桌面探针失败可检查来宾报告中的 `desktop/gui.json` 和 `desktop-session-journal.log`。openSUSE 显式安装提供 `dbus-run-session` 的 `dbus-1-daemon`，供探针启动独立用户总线。探针直接更新该总线的激活环境，等 Openbox 就绪后再操作 X11 授权窗口。等待 GNOME Shell 完成启动、Wayland 套接字和 AppIndicator 宿主就绪后才启动应用，先验证真实托盘注册，再发起 CLI/TUI 负载。实际登录仍覆盖应用与托盘宿主并行启动。

全部目标均需实际验证普通用户 GUI/TUI/CLI、共享后台、代理/TUN/DNS、图形授权拒绝与允许、登录自启动迁移、托盘与菜单恢复同一进程、原生包升级/卸载后保留数据。Fedora、openSUSE、Arch 的 GNOME 采用明确记录的 AppIndicator 扩展；不将该验证写成默认桌面自带托盘。首个 RPM/Arch 升级夹具使用经 SHA256 校验的公开旧 DEB 重新封装，不冒充历史已发布的原生附件。

RPM 与 Arch 安装由各自包管理器更新，应用不调用 DEB 安装器覆盖其文件。Arch 配方和 `.SRCINFO` 使用本次已验证的候选，未提交 AUR，也不接管第三方包。

原生包的预发布版本字段使用 RPM 的 `~` 或 pacman 的 `pre.` 标记，版本后缀中的连字符转换为 `+`。RPM 发行附件名使用原始发行版本，Arch 候选包名使用原生 `pkgver`。兼容来宾用原生比较器校验预发布编号递增及正式版顺序。隔离 DNS 夹具按来宾实际存在的系统 CA 文件装载临时测试根，并在同一挂载命名空间中恢复。

提交 `628ef7b4` 使用 [CI 44544140980](https://github.com/Fu021/Clash-of-Rust/actions/runs/44544140980) 完成 [Zorin 17.3 和 18.1](https://github.com/Fu021/Clash-of-Rust/actions/runs/44548065940) 及 [Arch x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/44548065854) 的完整兼容验收。报告和回执记录各来宾使用的候选身份、真实登录和托盘、原生升级与卸载的数据保留。新提交仍按发布范围取得同源回执。
