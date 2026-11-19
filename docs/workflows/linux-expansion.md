# Linux 扩展兼容验证

每个发行版有独立 `workflow_dispatch` 入口，指定同提交成功的 `CI` 运行编号 `run_id`。普通 push/PR 不启动这些 VM；CI 每个 Linux 架构只编译一次并生成 DEB，再从同架构 DEB 组装 RPM。发布时按 [Linux 范围](release.md)要求相应回执。本页描述测试目标，不是兼容通过或可下载声明。

| 手动入口 | 固定来宾及架构 | 安装与桌面 |
| --- | --- | --- |
| Pop!_OS Compatibility | System76 22.04 build 58、24.04 build 20（x64） | 离线 `distinst` 后从系统盘启动；DEB；分别验证 Pop GNOME 和 COSMIC |
| Zorin OS Compatibility | Zorin 17.3 Core r2、18.1 Core（x64） | 离线 Ubiquity；DEB；Zorin GNOME 与发行版原生托盘 |
| Fedora Compatibility | Fedora Cloud Base Generic 44-1.7、45-1.4（x64） | DNF 安装同源 RPM；GNOME Wayland |
| openSUSE Compatibility | Leap 16.0 Build 18.95、16.1 Build 3.18（x64、ARM64）；Tumbleweed 20261118（x64） | 可写 Cloud VM，Zypper 安装同源 RPM；GNOME Wayland |
| Arch Linux Compatibility | 官方 Cloud 20261115.625457（x64） | `makepkg` 由同源 DEB 或固定的升级夹具还原安装目录；`pacman` 验收；GNOME Wayland |

镜像 URL、完整摘要与架构由 `scripts/linux_compatibility_targets.py` 固定并校验，滚动版使用相应快照仓库。Tumbleweed 镜像直接从 openSUSE 官方内容源取固定快照，避免镜像跳转指向尚未同步该文件的站点。QEMU 来宾运行在同架构的 `ubuntu-24.04` 或 `ubuntu-24.04-arm` 托管 runner。COSMIC live ISO 的独立启动诊断保留，直到安装版取得等价的真实登录和失败取证证据。

新增或替换目标时核对发行版的维护期限和镜像来源。上游宣布停止维护后，复查本项目是否仍需该目标作为明确的兼容基线。替换受支持目标须先验证替代版本，再同步目标矩阵、公开支持范围和发布门禁。滚动版每次更换快照都记录日期与摘要，使用配套的软件源并完整复验。Ubuntu 的最低内核基线见 [Ubuntu 兼容验证](ubuntu-compatibility.md)。

排查时先对照各目标 `linux-vm.json` 的 `failed_stage`，区分输入下载、来宾启动、源码传入、依赖准备和已安装候选验收。宿主工具安装失败看之前独立的 APT 步骤。来宾已启动时，报告同时记录内核与 KVM/TCG。桌面探针失败可检查来宾报告中的 `desktop/gui.json` 和 `desktop-session-journal.log`。openSUSE 显式安装提供 `dbus-run-session` 的 `dbus-1-daemon`，供探针启动独立用户总线。探针直接更新该总线的激活环境，等 Openbox 就绪后再给 X11 授权窗口焦点。COSMIC 用单线程 OCR 识别实际弹窗，单帧超时后重新截图识别。等待 GNOME Shell 完成启动、Wayland 套接字和 AppIndicator 宿主就绪后才启动应用，先验证真实托盘注册，再发起 CLI/TUI 负载。实际登录仍覆盖应用与托盘宿主并行启动。

全部目标均需实际验证普通用户 GUI/TUI/CLI、共享后台、代理/TUN/DNS、图形授权拒绝与允许、登录自启动迁移、托盘与菜单恢复同一进程、原生包升级/卸载后保留数据。Fedora、openSUSE、Arch 的 GNOME 采用明确记录的 AppIndicator 扩展；不将该验证写成默认桌面自带托盘。首个 RPM/Arch 升级夹具使用经 SHA256 校验的公开旧 DEB 重新封装，不冒充历史已发布的原生附件。

Fedora 的隔离 X11 授权探针使用发行版提供的 `xfce-polkit`。该代理可在收到 [PAM 密码请求](https://github.com/ncopa/xfce-polkit/blob/6d3282cc1734c305850d48f5bf4b4d94e88885e9/src/xfce-polkit-listener.c)前显示窗体。验收同时等待可见窗体和本次会话的密码请求，取消后重新注册代理，原始代理调试输出只用于探测。来宾先从 Fedora 官方仓库更新 `pam` 和 `liblastlog2`，后者包含 [Fedora 44 已知 PAM 模块加载故障](https://bugzilla.redhat.com/show_bug.cgi?id=2453457)的修复。失败报告记录已装版本和相关审核事件。

COSMIC 探针依次用原图、去黑边灰度图和全幅灰度图定位实际授权对话框。取消阶段点击 `Cancel`，接受阶段点击 `Password` 输入框，再用 [COSMIC 密码框的回车提交](https://github.com/pop-os/cosmic-osd/blob/master/src/components/polkit_dialog.rs)完成图形授权。两阶段均匹配应用授权文本，验收确认取消保留原权限、接受后安装助手取得预期权限。

Tumbleweed 的 [GDM 打包补丁](https://api.opensuse.org/public/source/openSUSE:Factory/gdm/gdm-sysconfig-settings.patch)使 `/etc/sysconfig/displaymanager` 的 `DISPLAYMANAGER_AUTOLOGIN` 优先于 `custom.conf`。在一次性 VM 仅替换该键的值，保留实际 GDM 启动 Wayland 会话与登录自启动的断言。

真实 GNOME Wayland 登录前，探针通过短生命周期的 D-Bus 会话写入扩展设置，指定 GIO 的本地文件实现以免启动 GVfs。若 SSH 阶段超时而 `desktop-login/prepare.json` 已写入成功结果，核对该命令是否启动了仍持有 SSH 输出的进程。真实 X11 登录中，窗口枚举遇到已销毁的 X11 客户窗口时短暂重试，成功读取窗口列表后再判断是否前台显示。

RPM 与 Arch 安装由各自包管理器更新，应用不调用 DEB 安装器覆盖其文件。Arch 配方和 `.SRCINFO` 使用本次已验证的候选，未提交 AUR，也不接管第三方包。

原生包的预发布版本字段使用 RPM 的 `~` 或 pacman 的 `pre.` 标记，版本后缀中的连字符转换为 `+`。RPM 发行附件名使用原始发行版本，Arch 候选包名使用原生 `pkgver`。兼容来宾用原生比较器校验预发布编号递增及正式版顺序。隔离 DNS 夹具按来宾实际存在的系统 CA 文件装载临时测试根，并在同一挂载命名空间中恢复。

提交 `45bdc6aa` 使用 CI `44570854731` 完成 [Zorin 17.3 和 18.1](https://github.com/Fu021/Clash-of-Rust/actions/runs/44573997866) 及 [Arch x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/44573996263) 的完整兼容验收，两份回执均经发布门禁复核。来宾报告记录真实登录与托盘、三端和原生升级、卸载后的数据保留。新提交仍按发布范围取得同源回执。

提交 `0f48d91d` 使用 CI `44580277926` 完成 [openSUSE 三个 x64 目标](https://github.com/Fu021/Clash-of-Rust/actions/runs/44584087058) 的完整兼容验收，回执经发布门禁复核。Tumbleweed 的实际 GNOME 登录、托盘和菜单复用、23 项真实 PTY 与 RPM 升级和卸载均通过。
