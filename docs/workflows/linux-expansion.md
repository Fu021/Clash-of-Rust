# Linux 扩展兼容验证

每个发行版有独立 `workflow_dispatch` 入口，指定同提交成功的 `CI` 运行编号 `run_id`。普通 push/PR 不启动这些 VM；CI 每个 Linux 架构只编译一次并生成 DEB，再从同架构 DEB 组装 RPM。发布时按 [Linux 范围](release.md)要求相应回执。本页描述测试目标，不是兼容通过或可下载声明。

| 手动入口 | 固定来宾及架构 | 安装与桌面 |
| --- | --- | --- |
| Pop!_OS Compatibility | System76 22.04 build 58 和 24.04 build 20（x64），24.04 Generic build 3（ARM64） | 离线 `distinst` 后从系统盘启动，分别验证 Pop GNOME 和 COSMIC。COSMIC 保留 TUN 与应用代理，系统代理请求返回明确错误并保留原有路由 |
| Zorin OS Compatibility | Zorin 17.3 Core r2、18.1 Core（x64） | 离线 Ubiquity；DEB；Zorin GNOME 与发行版原生托盘 |
| Fedora Compatibility | Fedora Cloud Base 44-1.7、45-1.4（x64、ARM64） | 45 ARM64 使用 UEFI-UKI，其余使用 Generic；DNF 安装同源 RPM；GNOME Wayland |
| openSUSE Compatibility | Leap 16.0 Build 18.95、16.1 Build 3.18（x64、ARM64）；Tumbleweed 20261118（x64）、20261117（ARM64） | 可写 Cloud VM，Zypper 安装同架构的同源 RPM；GNOME Wayland |
| Arch Linux Compatibility | 官方 Cloud 20261115.625457（x64） | `makepkg` 由同源 DEB 或固定的升级夹具还原安装目录；`pacman` 验收；GNOME Wayland |

镜像 URL、完整摘要与架构由 `scripts/linux_compatibility_targets.py` 固定并校验，滚动版使用相应快照仓库。Tumbleweed 镜像直接从 openSUSE 官方内容源取固定快照。QEMU 来宾通常使用同架构的 `ubuntu-24.04` 或 `ubuntu-24.04-arm`。Fedora 45 ARM64 使用 `ubuntu-26.04-arm` 的 QEMU 10.2 和 EDK II 2025.11 系列。COSMIC live ISO 的独立启动诊断保留，直到安装版取得等价的真实登录和失败取证证据。依据 [QEMU 软件包](https://packages.ubuntu.com/resolute-updates/qemu-system-arm)和 [EDK II 软件包](https://packages.ubuntu.com/source/resolute/edk2)。

新增或替换目标时核对发行版的维护期限和镜像来源。上游宣布停止维护后，复查本项目是否仍需该目标作为明确的兼容基线。替换受支持目标须先验证替代版本，再同步目标矩阵、公开支持范围和发布门禁。滚动版每次更换快照都记录日期与摘要，使用配套的软件源并完整复验。Ubuntu 的最低内核基线见 [Ubuntu 兼容验证](ubuntu-compatibility.md)。

排查时先对照各目标 `linux-vm.json` 的 `failed_stage`，区分输入下载、来宾启动、源码传入、依赖准备和已安装候选验收。宿主工具安装失败看之前独立的 APT 步骤。报告记录 runner 标签和镜像版本，以及所选的 KVM/TCG 和启动耗时。来宾启动后再记录其实际内核。串口出现 emergency mode 或 kernel panic 时结束该次启动并保留日志，桌面来宾同时尝试截图。桌面探针失败可检查来宾报告中的 `desktop/gui.json` 和 `desktop-session-journal.log`。openSUSE 显式安装提供 `dbus-run-session` 的 `dbus-1-daemon`，供探针启动独立用户总线。探针直接更新该总线的激活环境，等 Openbox 就绪后再给 X11 授权窗口焦点。COSMIC 用单线程 OCR 识别实际弹窗，单帧超时后重新截图识别。等待 GNOME Shell 完成启动、Wayland 套接字和 AppIndicator 宿主就绪后才启动应用，先验证真实托盘注册，再发起 CLI/TUI 负载。实际登录仍覆盖应用与托盘宿主并行启动。

`linux-vm.json` 单列官方镜像与 Python 工具的下载校验时间。ISO 来宾分别记录安装器和安装盘启动的耗时。安装器失败时记录 `iso_install` 失败阶段和已用时间。已安装候选检查、桌面登录和包升级卸载分别计时。候选检查内部的依赖准备由 `compatibility-environment.json` 单列，VM 在来宾归档内保存，直接使用托管 runner 时随报告上传。候选检查超时或桌面阶段失败时，通过私有 QEMU 监控尝试保存画面并记录截图失败原因。实际登录一旦确认候选托盘初始化失败，立即结束该阶段。状态等待和截图均有时限，来宾报告仍保留启动状态、日志与进程快照。

隔离网络检查分别记录 root CLI、普通账户矩阵和原生代理/TUN 流量阶段。开启 ARM64 扩展账户检查时，命名空间为环境准备和账户检查留出 560 秒，再计入流量探针的独立时限。流量探针通常最多 240 秒，启用代理基准时为 360 秒。各阶段的子检查仍保留自己的超时。外层到达时限时终止整个测试 PID 命名空间，避免后台进程继续占用 SSH 通道、延迟失败截图和来宾清理。

全部目标均需实际验证普通用户 GUI/TUI/CLI、共享后台、代理/TUN/DNS、图形授权拒绝与允许、登录自启动迁移、托盘与菜单恢复同一进程、原生包升级/卸载后保留数据。Fedora、openSUSE、Arch 的 GNOME 采用明确记录的 AppIndicator 扩展；不将该验证写成默认桌面自带托盘。首个 RPM/Arch 升级夹具使用经 SHA256 校验的公开旧 DEB 重新封装，不冒充历史已发布的原生附件。

Fedora 的隔离 X11 授权探针使用发行版提供的 `xfce-polkit`。该代理可在收到 [PAM 密码请求](https://github.com/ncopa/xfce-polkit/blob/6d3282cc1734c305850d48f5bf4b4d94e88885e9/src/xfce-polkit-listener.c)前显示窗体。验收同时等待可见窗体和本次会话的密码请求，取消后重新注册代理，原始代理调试输出只用于探测。来宾先从 Fedora 官方仓库更新 `pam`、`liblastlog2` 和 `selinux-policy-targeted`。后两者分别包含 [Fedora 44 PAM 模块加载修复](https://bugzilla.redhat.com/show_bug.cgi?id=2453457)及 [polkit socket 助手域转换修复](https://github.com/fedora-selinux/selinux-policy/commit/8ad5d5112fa8b47426e0e6674616d5577f825fde)。失败报告记录已装版本和相关审核事件。

Fedora 45 ARM64 的 Generic 镜像在 TCG 上切换系统根后曾两次进入紧急模式。矩阵固定到同版官方 [UEFI-UKI](https://fedoraproject.org/wiki/Changes/Unified_Kernel_Support_Phase_2) 镜像，给每个来宾单独复制可写的 AAVMF 变量存储，支持该镜像的 UEFI 启动项。

Pop!_OS ARM64 使用 AAVMF、`ttyAMA0` 和只读 VirtIO SCSI 光驱。Live 系统先等待 D-Bus、logind 和目标磁盘的 udev 初始化，避免早期串口抢在系统总线之前启动安装器。Live 系统仅启动直接调用的 `distinst`，屏蔽独立的 `distinst-v2` 服务。安装器按一次性磁盘的序列号校验写入目标。系统从安装盘重启后，用非 VGA VirtIO GPU 接入实际 COSMIC 会话和宿主画面探针。兼容回执覆盖三个目标及两种架构的 CI 候选。

隔离 GNOME 探针看到新的 Wayland 套接字后才查询 Shell 的 D-Bus 对象。GNOME 50 [完成 UI 初始化后才导出该对象](https://github.com/GNOME/gnome-shell/blob/50.6/js/ui/main.js#L163-L176)。ARM64 TCG 的合成器就绪限时为 120 秒，其余目标为 60 秒。进程提前退出会立即结束验收。候选阶段出错或超时均尝试保存 QEMU 画面，嵌套桌面状态另见来宾日志。

Debian、openSUSE 和 Arch 的隔离 X11 探针使用 MATE 授权代理。在可见窗体和 [polkit 的隐藏输入请求](https://github.com/polkit-org/polkit/blob/127/src/polkitagent/polkitagentsession.c)都就绪后才发送操作；每次重启代理同时清空私有探测输出。报告记录验收结果，原始认证调试流只在探针运行期间使用。

COSMIC 登录探针等待 logind 标记本次桌面会话为活动状态后发起授权。宿主依次用原图、去黑边灰度图和全幅灰度图定位实际授权对话框。取消阶段点击 `Cancel`，接受阶段点击 `Password` 输入框，再用 [COSMIC 密码框的回车提交](https://github.com/pop-os/cosmic-osd/blob/master/src/components/polkit_dialog.rs)完成图形授权。两阶段均匹配应用授权文本，验收确认取消保留原权限、接受后安装助手取得预期权限。实际登录通过窗口列表和托盘注册确认应用状态。关闭操作要求窗口列表中恰有一个匹配项且合成器声明支持关闭，随后确认窗口消失、原 PID 仍在。原生 Wayland GUI 阶段单独检查窗口尺寸。宿主捕获授权取证和失败画面，探针在预期弹窗前终止时另存来宾结果。

COSMIC 的[合成器刷新](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs)异步发送几何事件。探针识别出目标窗口后最多等待一秒接收几何，原生 Wayland 验收仍检查 800×450 的最低尺寸。

Tumbleweed 在来宾关闭预设仓库，只启用与云镜像日期一致的签名仓库，并禁用自动刷新。x64 使用 openSUSE 历史目录。ARM64 的索引 SHA256 固定到 [openSUSE openQA 快照](https://openqa.opensuse.org/api/v1/assets/repo/openSUSE-Tumbleweed-oss-aarch64-Snapshot20261117)。官方 CDN 的索引逐字节匹配时优先使用 CDN，否则使用 openQA。Zypper 验签后再次核对缓存中的索引 SHA256，匹配才继续安装。openQA 会清理非永久资产，固定源不可访问时须重新选择并校验一对匹配的镜像与仓库。Tumbleweed 的 [GDM 打包补丁](https://api.opensuse.org/public/source/openSUSE:Factory/gdm/gdm-sysconfig-settings.patch)使 `/etc/sysconfig/displaymanager` 的 `DISPLAYMANAGER_AUTOLOGIN` 优先于 `custom.conf`。在一次性 VM 仅替换该键的值，保留实际 GDM 启动 Wayland 会话与登录自启动的断言。

真实 GNOME Wayland 登录前，探针通过短生命周期的 D-Bus 会话写入扩展设置，指定 GIO 的本地文件实现以免启动 GVfs。若 SSH 阶段超时而 `desktop-login/prepare.json` 已写入成功结果，核对该命令是否启动了仍持有 SSH 输出的进程。真实 X11 登录中，窗口枚举遇到已销毁的 X11 客户窗口时短暂重试，成功读取窗口列表后再判断是否前台显示。

RPM 与 Arch 安装由各自包管理器更新，应用不调用 DEB 安装器覆盖其文件。Arch 配方和 `.SRCINFO` 使用本次已验证的候选，未提交 AUR，也不接管第三方包。

原生包的预发布版本字段使用 RPM 的 `~` 或 pacman 的 `pre.` 标记，版本后缀中的连字符转换为 `+`。RPM 发行附件名使用原始发行版本，Arch 候选包名使用原生 `pkgver`。兼容来宾用原生比较器校验预发布编号递增及正式版顺序。隔离 DNS 夹具按来宾实际存在的系统 CA 文件装载临时测试根，并在同一挂载命名空间中恢复。夹具仅给节点引导的正向解析结果 30 秒 TTL，以减少 ARM64 TCG 上重复的加密握手。测试仍要求每次冷启动重新解析节点，并验证 UDP、TCP 和 DNS 未旁路。

提交 `a433d221` 使用 CI `44670222142` 完成 Pop!_OS 22.04/24.04、Zorin 17.3/18.1 及 Arch 的 x64 验收，来宾均使用 KVM。提交 `280d5679` 使用 CI `44639212301` 完成 openSUSE Leap 16.0/16.1 及 Tumbleweed 的 x64 和 ARM64 验收，分别使用 KVM 和 TCG。回执均经发布门禁复核，来宾报告记录真实登录与托盘、三端、原生升级及卸载后保留数据。新提交仍按实际发布范围取得同源回执。[Pop!_OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672608619) · [Zorin](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672632359) · [Arch](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672635871) · [openSUSE](https://github.com/Fu021/Clash-of-Rust/actions/runs/44642538483)

提交 `3b62e2ef` 使用 CI `44625972854` 完成 [Fedora 44/45 x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/44628532038)的完整兼容验收，覆盖图形授权、真实登录与托盘、三端和 RPM 升级及卸载。该历史回执只覆盖 x64。

提交 `0f48d91d` 使用 CI `44580277926` 完成 [openSUSE 三个 x64 目标](https://github.com/Fu021/Clash-of-Rust/actions/runs/44584087058) 的完整兼容验收，回执经发布门禁复核。Tumbleweed 的实际 GNOME 登录、托盘和菜单复用、23 项真实 PTY 与 RPM 升级和卸载均通过。
