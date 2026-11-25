# Linux 扩展兼容验证

每个发行版有独立 `workflow_dispatch` 入口，指定同提交成功的 `CI` 运行编号 `run_id`。CI 每个 Linux 架构只编译一次并生成 DEB，再从同架构 DEB 组装 Fedora 和 openSUSE 验收使用的 RPM。发布时按 [Linux 范围](release.md)要求相应回执。

基础验收实际安装并启动同源候选包，验证系统代理、TUN 流量、DNS 和退出恢复。全部 ARM64 目标使用按摘要固定的同架构容器，x64 使用发行版容器或托管 runner。容器覆盖发行版用户态和包管理，TUN 使用 runner 内核。桌面及更新的完整结果保存在独立专项或历史报告。

## Linux 跨版本更新

验证 x64 的公开跨版本更新时，在发行版入口同时填写旧版成功 push CI 的 `update_from_ci` 和目标公开标签 `update_release`。ARM64 继续执行同一套基础容器检查。x64 作业先校验旧候选和目标 Release 的包及摘要，再在一次性 runner 或虚拟机中验证升级。DEB 和 Arch 通过 `cor` 更新；RPM 验证 DNF 或 Zypper 提示并用原生包管理器安装。`published-update.json` 记录实际版本和设置、订阅及原内核状态的保留。旧 CI 附件或 Arch 回执过期后须选择可验证的来源。

Ubuntu 24.04 x64 托管 runner 和 Arch x64 来宾在终端验收后补测 GUI。经实际 polkit 授权验证取消、包管理器锁冲突、解锁重试以及升级后的版本与数据。图形结果以 `published-gui-update.json` 为准。

已安装旧版明确报告 GitHub 限流时，终端的查版本、下载和安装前查版本，以及 GUI 的版本发现，各最多重试三次。旧版只向夹具返回限流提示，夹具按 60、120、240 秒退避，每次另加独立的 0～30 秒随机延迟，在原有验收总时限内错开各 runner 的请求。GUI 关闭本次限流提示后再查版本。安装记录已经产生时直接保留结果，授权失败、包锁和校验失败走各自的验收路径。

GUI 夹具按 Linux 内核的 [`fcntl` 封印 ABI](https://github.com/torvalds/linux/blob/master/include/uapi/linux/fcntl.h)校验 pkexec 收到的文件。固定来宾 Python 缺少这些常量的导出，测试直接使用内核定义的命令号和四项封印位。

GUI 夹具收到中断信号后完成清理并保留原始失败。测试账户和临时授权规则使用非交互 `sudo`，由提权后的独立超时约束命令。独立捕获 `userdel` 的输出，让 SSH 日志流关闭时仍可读到正确退出码并完成账户清理。Ubuntu 20.04 的旧版 polkit 使用 `.pkla` 规则，仅为本次临时账户的 `pkexec` 动作要求输入该账户密码；验收结束时删除规则和账户。旧版规则不能按程序路径缩小范围，较新 polkit 的规则会另外限定包管理器路径。x64 和 ARM64 来宾使用 TCG 时，识别页面标题只处理标题区域，并为 OCR、脱敏截图生成和整轮图形操作预留更长时间；超时回执只记录允许的工具名称与时限。整页 OCR 未命中旧版白字按钮时，裁取固定测试强调色内的按钮，先放大再二值化以保留笔画。仅在识别到完整就绪标签时才点击；安装失败后点击提示内的关闭按钮并检查结果确认状态。图形代理重启后仍有注册时序风险。[polkit 的公开 D-Bus 接口](https://polkit.pages.freedesktop.org/polkit/eggdbus-interface-org.freedesktop.PolicyKit1.Authority.html)没有会话代理注册查询。若实际安装未弹出授权窗口，应查看对应会话的 polkit 注册日志。

2026-11-22 使用 `59b91a03`、CI `45011206632`、旧候选 CI `44943304678` 和公开预发布 `v0.7.3-dev.2.live-language`，以下八套共 21 个目标的跨版本终端报告通过：[Ubuntu](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456673)、[Debian](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456896)、[Mint](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456401)、[Pop!_OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456816)、[Zorin](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456535)、[Fedora](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456876)、[openSUSE](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013456820)及 [Arch](https://github.com/Fu021/Clash-of-Rust/actions/runs/45013457070)。Arch 旧包来自 CI 候选；GUI 图形授权与失败恢复不在此批报告范围。

2026-11-23，[Ubuntu 24.04 x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/45144739726/attempts/2)实际通过 GUI 授权取消、dpkg 锁失败、解锁后升级及数据保留。另用官方 Ubuntu 24.04 ARM64 云镜像和 QEMU 软件仿真，以普通用户从 TUI 中将同一旧 CI 的 DEB 更新到同一公开预发布版。实际通过 TUI 与终端确认、sudo 密码授权、DEB 安装、终端恢复和新后台与内核启动。设置和订阅保留，原来未分配的直连下载端口在首次启动内核时自动分配。

同日，[Arch x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/45155581626)从旧 CI 原生包升级到同一公开预发布版。终端发现限流后分别等待 65.3、126.7 和 252.5 秒，第四次发现成功。图形路径实际通过 pkexec 密封包校验、取消、`pacman` 数据库锁冲突、释放锁后升级、设置及订阅保留、GUI 重开和内核启动。兼容回执绑定该轮测试的 Arch 候选包及同源 DEB。

## 原生包兼容矩阵

| 手动入口 | 基础验收环境及架构 | 原生安装路径 |
| --- | --- | --- |
| Pop!_OS Compatibility | 22.04（x64）、24.04（x64、ARM64）的精简用户态容器 | 同版 Ubuntu 基础镜像加 System76 官方签名软件源和 `pop-default-settings`，用 APT 安装 DEB |
| Zorin OS Compatibility | Zorin 17.3、18.1（x64）的精简用户态容器 | 同版 Ubuntu 基础镜像加 Zorin 签名补丁软件源及原生 `base-files`，用 APT 安装 DEB |
| Fedora Compatibility | Fedora 44、45 官方容器（x64、ARM64） | DNF 安装同源 RPM |
| openSUSE Compatibility | Leap 16.0、16.1、Tumbleweed 20261123（均有 x64、ARM64） | 基础验收使用官方容器，Zypper 安装同源 RPM |
| Arch Linux Compatibility | 官方 `base-20261108.0.623074` 容器（x64），使用 2026-11-15 签名软件源 | `makepkg` 从同源 DEB 生成原生包，再由 `pacman` 安装 |

Pop!_OS 22.04 的 ARM64 介质为 [Raspberry Pi 4 板级镜像](https://iso.pop-os.org/22.04/arm64/raspi/4/pop-os_22.04_arm64_raspi_4.img.xz)，该版本仅在矩阵中覆盖 x64。Pop!_OS 24.04 ARM64 的历史本地完整验收使用 [官方通用镜像](https://system76.com/download-pop/)。

x64 跨版本更新使用 System76 22.04 build 58、24.04 build 20，Zorin 17.3 Core r2、18.1 Core ISO，以及 Fedora Cloud Base 44-1.7、45-1.4。ISO 来宾离线安装后从系统盘启动。

镜像 URL、完整摘要与架构由 `scripts/linux_compatibility_targets.py` 固定并校验，滚动版使用相应快照仓库。Linux Mint 与 Zorin OS 使用[发行版列名的镜像](https://www.linuxmint.com/edition.php?id=311)、[Zorin 官方镜像](https://zorin.com/os/mirrors/)，下载 ISO 前并行抽样排序；非最后一个地址持续低速或网络失败时尝试下一地址，最后一个地址允许慢速完成。openSUSE 镜像不可用时也会尝试列名镜像。全部来源使用相同的固定摘要，报告记录实际地址；不能取得匹配文件时复核官方索引、摘要及匹配的签名软件源。QEMU 来宾使用同架构的 runner。openSUSE ARM64 三个目标已完成[历史全量验收](#历史兼容证据)，当前矩阵验收原生包安装、基础系统代理和 TUN。

容器作业记录 `linux-container.json`，包含固定基础镜像、发行版、架构、实际 runner 内核和耗时。Pop!_OS、Linux Mint 和 Zorin 的汇总回执还绑定各自的签名软件源密钥或原生基础包摘要。[System76 Release](https://github.com/pop-os/repo-release) · [Linux Mint 镜像](https://linuxmint.com/mirrors.php) · [Zorin OS Patches](https://launchpad.net/~zorinos/+archive/ubuntu/patches)

显式跨版本更新时，openSUSE 使用 Leap 16.0 Build 18.97、16.1 Build 3.23 和 Tumbleweed 20261123 的官方云镜像。

新增或替换目标时核对发行版的维护期限和镜像来源。上游宣布停止维护后，复查本项目是否仍需该目标作为明确的兼容基线。替换受支持目标须先验证替代版本，再同步目标矩阵、公开支持范围和发布门禁。滚动版每次更换快照都记录日期与摘要，使用配套的软件源并完整复验。Ubuntu 的最低内核基线见 [Ubuntu 兼容验证](ubuntu-compatibility.md)。

基础验收的容器作业限时 15 分钟，原生 runner 作业限时 30 分钟。显式跨版本更新的作业至少预留 120 分钟。排查该专项的 VM 时，先按 `linux-vm.json` 的 `failed_stage` 区分输入下载、来宾启动、源码传入、依赖准备及候选验收。报告记录 runner 标签、镜像版本、KVM 或 TCG、实际内核及各阶段耗时。KVM 来宾使用不超过 runner 可用 CPU 数的四个 vCPU。来宾引导或候选检查失败时保留串口日志，并尝试通过私有 QEMU 监控截图。

`linux-vm.json` 单列官方镜像与 Python 工具的下载校验时间。ISO 来宾分别记录安装器和安装盘启动的耗时。安装器失败时记录 `iso_install` 失败阶段和已用时间。候选检查内部的依赖准备由 `compatibility-environment.json` 单列，VM 在来宾归档内保存，直接使用托管 runner 时随报告上传。

隔离网络检查分别记录 root CLI、普通账户矩阵和原生代理/TUN 流量阶段。非 TCG 的 ARM64 扩展验收为环境准备和账户检查留出 560 秒，再计入流量探针的独立时限。流量探针通常最多 240 秒，启用代理基准时为 360 秒。TCG 来宾为扩展账户阶段预留 1800 秒，为流量探针预留 1200 秒。ARM64 TCG 的每轮保护 DNS 与 HTTP 检查总时限为 90 秒，保留单条请求原有时限。外层到达时限时终止整个测试 PID 命名空间，避免后台进程继续占用 SSH 通道、延迟失败截图和来宾清理。进入命名空间后恢复 `SIGTERM` 和 `SIGINT` 的处理，避免旧版 `unshare` 让 DNS 和总线夹具继承忽略信号的设置。

## 完整验收与诊断

首轮完整验收覆盖普通用户 GUI/TUI/CLI、共享后台、代理/TUN/DNS、图形授权拒绝与允许、登录自启动迁移、托盘与菜单恢复同一进程、原生包升级及卸载后的数据保留。COSMIC 保留 TUN 与应用代理，系统代理请求返回明确错误并保留原有路由。Fedora、openSUSE、Arch 在 GNOME 使用 AppIndicator 扩展验证托盘。RPM 和 Arch 的首个升级夹具使用经 SHA256 校验的公开旧 DEB 重新封装。

### 本地一次性来宾

从官方镜像和已核验摘要创建一次性 QEMU/KVM 来宾，基盘只读，系统改动写入独立磁盘。用同提交成功 CI 的候选 DEB、`package-receipt.json` 和 `linux-tun-test`；源码快照中的 `source-commit.txt` 必须与回执提交一致。代理、TUN、真实登录及安装操作均在来宾内执行。

在一次性来宾的初始化阶段创建 `/etc/clash-of-rust-compatibility-vm`，所有者为 `root`、权限为 `0444`，内容为一行 `clash-of-rust disposable compatibility guest v1`，以换行结束。脚本同时要求 `systemd-detect-virt --vm` 返回 `qemu` 或 `kvm`、普通用户身份及 `COR_LINUX_GUEST=true`。完整验收设置 `COR_CI_EXTENDED_TESTS=true`。候选入口为 `scripts/test-release-package.py --linux-compat`，真实登录继续走 `scripts/linux_desktop_login.py`。

真实桌面登录需要为 ARM64 来宾提供图形设备，例如 `virtio-gpu-pci`。Ubuntu 20.04、22.04 和 24.04 走 GDM 的 GNOME X11 登录，26.04 走 GNOME Wayland。隔离 GUI 夹具另测图形授权和窗口。`prepare` 在一次性来宾屏蔽 `sleep.target`，避免桌面空闲后挂起、打断跨重启验收。运行 `prepare` 后重启来宾执行 `first-login`，再重启执行 `background-login`，两次都核对 boot ID。QEMU 应在来宾重启时复位，使用 `-no-reboot` 会使进程退出。

包生命周期可以由宿主下载公开旧版 DEB，再传入来宾。`deb_upgrade_and_uninstall` 和 `native_upgrade_and_uninstall` 的 `previous_deb` 参数只接受一次性本地来宾中与固定发布摘要匹配的文件。

本地逐套运行。下载前、解包后、验收中及清理后记录文件系统余量、镜像与写入盘的实际占用，记录 KVM 或 TCG 及各阶段耗时。本地 TCG 来宾设置 `COR_LINUX_VM_ACCELERATION=tcg`，为 APT 依赖准备预留一小时。ARM64 隔离桌面验收预留 15 分钟，其中 GNOME 合成器启动限时 5 分钟。每套结束关机并回收脱敏报告，清理镜像、写入盘、种子盘和临时固件状态。工具和镜像的临时目录在本轮结束后清理。

Ubuntu TCG 来宾可在初始化时跳过命令提示和 MOTD 更新统计索引。这两个钩子与依赖解析无关，仿真时会显著延迟 APT 返回。保留软件源的包索引、签名验证和正常 `dpkg` 配置过程。

在一次性 ARM64 TCG 来宾准备 Ubuntu GNOME X11 登录时，`prepare` 为 GNOME Shell 的用户服务保留 360 秒启动时间。本地 Ubuntu 24.04 ARM64 在默认 90 秒内反复重启 GNOME Shell，延长后会话继续初始化并执行应用自启动。

Ubuntu 20.04 x64 在 ARM64 宿主上用 TCG 验收时，嵌套 GNOME Wayland 的合成器发生段错误，原生会话的 `llvmpipe` 工作线程也发生段错误，限制为单工作线程后仍复现。保留失败报告，并在一次性来宾的测试账户中为 `gnome-shell-wayland.service` 设置 `LIBGL_ALWAYS_SOFTWARE=1` 和 `GALLIUM_DRIVER=softpipe`，重启 GDM 后重新验收。该配置下通过了原生 `seat0` 显示、托盘恢复、关窗和应用菜单重开，三步复用同一后台宿主。验收结果记录所用渲染器。Mesa 的软件渲染器选择见[环境变量说明](https://docs.mesa3d.org/envvars.html#gallium-driver)。

TCG 来宾的 GUI 跨版本更新对已知区域裁图后再识别文字。整窗识别、页面等待及图形会话使用软件仿真时限，适用于 x64 和 ARM64 来宾。Ubuntu 20.04 x64 曾在打开设置页后达到整窗识别的 20 秒时限。

Fedora 45 ARM64 的 UEFI-UKI 镜像在本地 TCG 冷启动时，`/dev/gpt-auto-root` 的等待先于 udev 完成而超时。Fedora ARM64 UKI 的 TCG 启动通过 [systemd-stub 的 SMBIOS 参数](https://www.freedesktop.org/software/systemd/man/257/sd-stub.html#SMBIOS%20Type%2011%20Strings)将设备等待时间设为 180 秒。首启保留官方 GPT 布局，完成 cloud-init 后，完整桌面验收将写入盘扩大到 24 GiB，并扩展已确认的 Btrfs 根分区。

Fedora 来宾的 NoCloud 种子使用 `preserve_hostname: true` 保留镜像主机名。验收通过本地转发端口连接来宾，以 `/etc/os-release` 和 `uname` 核对系统版本及架构。cloud-init 状态和其他初始化告警仍保存在回执。

一次性 Fedora ARM64 TCG 来宾为 `org.gnome.Shell@.service` 保留 360 秒启动时间。GNOME Wayland 登录用 logind 的本地主会话、用户总线上的 GNOME Shell 进程和实际 Wayland socket 核对会话身份。

### 桌面与发行版诊断

复查 COSMIC 登录、旧自启动入口迁移和窗口恢复时，在 Pop!_OS 24.04 隔离来宾中执行完整验收。现场日志可按[桌面启动诊断](../diagnostics.md#桌面启动与托盘)核对。

COSMIC 的欢迎向导会在新用户首次登录时启动，可能与应用启动和图形授权同时占用桌面。完整验收只为临时测试账户预置[向导完成标记](https://github.com/pop-os/cosmic-initial-setup/blob/master/src/main.rs)，登录后继续核对应用首次前台启动、托盘和实际图形授权。

Fedora 的隔离 X11 授权探针使用发行版提供的 `xfce-polkit`。该代理可在收到 [PAM 密码请求](https://github.com/ncopa/xfce-polkit/blob/6d3282cc1734c305850d48f5bf4b4d94e88885e9/src/xfce-polkit-listener.c)前显示窗体。验收同时等待可见窗体和本次会话的密码请求，取消后重新注册代理，原始代理调试输出只用于探测。来宾先从 Fedora 官方仓库更新 `pam`、`liblastlog2` 和 `selinux-policy-targeted`。后两者分别包含 [Fedora 44 PAM 模块加载修复](https://bugzilla.redhat.com/show_bug.cgi?id=2453457)及 [polkit socket 助手域转换修复](https://github.com/fedora-selinux/selinux-policy/commit/8ad5d5112fa8b47426e0e6674616d5577f825fde)。失败报告记录已装版本和相关审核事件。

隔离 GNOME 探针看到新的 Wayland 套接字后才查询 Shell 的 D-Bus 对象，合成器就绪限时为 60 秒。GNOME 50 [完成 UI 初始化后才导出该对象](https://github.com/GNOME/gnome-shell/blob/50.6/js/ui/main.js#L163-L176)。进程提前退出会立即结束验收。候选阶段出错或超时均尝试保存 QEMU 画面，嵌套桌面状态另见来宾日志。

Debian、openSUSE 和 Arch 的隔离 X11 探针使用 MATE 授权代理。在可见窗体和 [polkit 的隐藏输入请求](https://github.com/polkit-org/polkit/blob/127/src/polkitagent/polkitagentsession.c)都就绪后才发送操作；每次重启代理同时清空私有探测输出。报告记录验收结果，原始认证调试流只在探针运行期间使用。

COSMIC 登录探针在 logind 标记桌面会话为活动状态后，等待[来宾的 polkit 日志](https://github.com/polkit-org/polkit/blob/124/src/polkitbackend/polkitbackendinteractiveauthority.c#L2424-L2442)确认 `cosmic-osd` 已为同一次登录注册认证代理，再发起授权。宿主识别实际授权对话框，整图漏识别取消按钮时，按标题和密码字段定位附近的按钮行，同时核对 `Cancel` 与 `Authenticate`。取消阶段点击 `Cancel`，接受阶段点击 `Password` 输入框，再用 [COSMIC 密码框的回车提交](https://github.com/pop-os/cosmic-osd/blob/master/src/components/polkit_dialog.rs)完成图形授权。两阶段均匹配应用授权文本，验收确认取消保留原权限、接受后安装助手取得预期权限。实际登录通过窗口列表和托盘注册确认应用状态。关闭操作要求窗口列表中恰有一个匹配项且合成器声明支持关闭，随后确认窗口消失、原 PID 仍在。原生 Wayland GUI 阶段单独检查窗口尺寸。宿主捕获授权取证和失败画面，探针提前退出时另存来宾结果及有限长度的 `pkexec` 输出。

COSMIC 的[合成器刷新](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs)异步发送几何事件。探针识别出目标窗口后最多等待一秒接收几何，原生 Wayland 验收仍检查 800×450 的最低尺寸。

ARM64 的后台登录探针给 `gtk-launch` 最多 60 秒启动应用菜单项。菜单启动后核对实际窗口恢复和桌面宿主 PID 连续。独立 COSMIC Wayland 探针等待该会话的托盘宿主注册，再启动候选的原生窗口。在 ARM64 TCG 下为合成器报告符合尺寸的窗口保留 120 秒。

Tumbleweed 在来宾关闭预设仓库，使用与云镜像日期一致的签名仓库并禁用自动刷新。官方 CDN 的 `repomd.xml` 与固定快照摘要一致时优先使用 CDN，否则使用历史仓库；刷新后再次校验实际元数据。Tumbleweed 的 [GDM 打包补丁](https://api.opensuse.org/public/source/openSUSE:Factory/gdm/gdm-sysconfig-settings.patch)使 `/etc/sysconfig/displaymanager` 的 `DISPLAYMANAGER_AUTOLOGIN` 优先于 `custom.conf`。在一次性 VM 仅替换该键的值，保留实际 GDM 启动 Wayland 会话与登录自启动的断言。

真实 GNOME Wayland 登录前，探针通过短生命周期的 D-Bus 会话写入扩展设置，指定 GIO 的本地文件实现以免启动 GVfs。若 SSH 阶段超时而 `desktop-login/prepare.json` 已写入成功结果，核对该命令是否启动了仍持有 SSH 输出的进程。真实 X11 登录中，窗口枚举遇到已销毁的 X11 客户窗口时短暂重试，成功读取窗口列表后再判断是否前台显示。

原生包的预发布版本字段使用 RPM 的 `~` 或 pacman 的 `pre.` 标记，版本后缀中的连字符转换为 `+`。发布附件名使用原始发行版本。兼容来宾用原生比较器校验预发布编号递增及正式版顺序。隔离 DNS 夹具按来宾实际存在的系统 CA 文件装载临时测试根，并在同一挂载命名空间中恢复。夹具仅给节点引导的正向解析结果 30 秒 TTL，以减少 ARM64 TCG 上重复的加密握手。测试仍要求每次冷启动重新解析节点，并验证 UDP、TCP 和 DNS 未旁路。

### Arch Linux 原生包

来宾的 pacman 按镜像日期使用固定的官方归档。主站不可达时从同日镜像同步数据库。更新来宾、安装测试工具和安装本地候选包共用归档回退。快照包路径下载失败时，根据 pacman 解析出的事务，从镜像的按包名归档补齐仓库依赖及其 `.sig` 文件。逐包核对数据库中的 SHA256，再由 pacman 校验签名并安装。[Arch 官方归档镜像](https://github.com/archlinux/infrastructure/blob/main/docs/servers.md#archive-mirrors)。

发布范围包含 Arch Linux x86_64 时，Release 将提供原生二进制包。首次安装时从包含该包的版本下载 `Clash-of-Rust-<版本>-linux-x86_64.pkg.tar.zst` 及 `.sha256`，核对摘要后执行 `sudo pacman -U ./Clash-of-Rust-<版本>-linux-x86_64.pkg.tar.zst`，其中 `<版本>` 替换为实际版本。包名是 `clash-of-rust`，配方保留 `/opt/clash-of-rust`、随包内核和固定的 polkit 助手路径，并写入 `pacman-release` 标识。

Arch 工作流使用 CI 的 x64 DEB 在 Arch 环境内重封装，检查安装后的包身份、实际终端更新助手、基础系统代理和 TUN。基础验收使用按摘要固定的 [Arch 官方 OCI 镜像](https://github.com/archlinux/archlinux-docker)，显式跨版本更新使用官方 Cloud 20261115.625457 来宾。两条路径均校验实际装过的原包与回执再上传。记录作业绑定实际执行验收的 attempt 与归档摘要，发布阶段用同一字节生成校验文件并加入 Release。应用内更新见[版本与更新通道](versions.md#版本与更新通道)。

### 历史兼容证据

提交 `0ccca669` 使用 CI `44696750124` 完整验收当时矩阵中的 Debian、Linux Mint、Zorin OS、Fedora、openSUSE 和 Arch Linux。Ubuntu 的同轮回执只覆盖安装后的基础系统代理与 TUN，不包含完整桌面登录和包生命周期。Pop!_OS 24.04 的 COSMIC 授权探针提前退出，补上认证代理就绪检查后用 `8999697d` 的 CI `44705538634` 复验。Pop!_OS 22.04 GNOME 和 24.04 COSMIC 均通过，并核对真实登录、图形授权、托盘及菜单复用、23 项 TUI PTY、原生升级和卸载保留数据。这是当时矩阵的历史回执，新发布按实际范围取得对应提交的回执。[首轮 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/44696750124) · [Pop!_OS 完整复验](https://github.com/Fu021/Clash-of-Rust/actions/runs/44709399603)

更早的完整验证中，`a433d221` 使用 CI `44670222142` 完成 Pop!_OS、Zorin OS 和 Arch Linux 的 x64 目标，均使用 KVM。`280d5679` 使用 CI `44639212301` 完成 openSUSE 两版 Leap 和 Tumbleweed 的 x64、ARM64 目标，分别使用 KVM 和 TCG。历史回执用于追溯相应候选与镜像的验证结果。[Pop!_OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672608619) · [Zorin](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672632359) · [Arch](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672635871) · [openSUSE](https://github.com/Fu021/Clash-of-Rust/actions/runs/44642538483)

提交 `3b62e2ef` 使用 CI `44625972854` 完成 [Fedora 44/45 x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/44628532038)的完整兼容验收，覆盖图形授权、真实登录与托盘、三端和 RPM 升级及卸载。该历史回执只覆盖 x64。

提交 `0f48d91d` 使用 CI `44580277926` 完成 [openSUSE 三个 x64 目标](https://github.com/Fu021/Clash-of-Rust/actions/runs/44584087058) 的完整兼容验收，回执经发布门禁复核。Tumbleweed 的实际 GNOME 登录、托盘和菜单复用、23 项真实 PTY 与 RPM 升级和卸载均通过。

## 历史虚拟机耗时

`37f4f2e3` 的 [CI 44712218319](https://github.com/Fu021/Clash-of-Rust/actions/runs/44712218319) 通过 13 个 job。以下八套验收共 21 个目标通过，逐一核对安装报告和绑定该 CI 的回执。环境准备合计来宾初始索引与候选所需依赖，ISO 安装另计。目标 job 从作业开始计时，整套从派发计至记录回执。VM 目标均使用 KVM，Ubuntu 22.04、24.04、26.04 的 x64 和 ARM64 直接使用托管 runner。

| 发行版 | 镜像下载与校验 | ISO 安装 | 环境准备 | 目标 job | 整套 |
| --- | ---: | ---: | ---: | ---: | ---: |
| [Ubuntu](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714877856) | 20.04 为 5.6 秒 | — | 20.04 为 39.1 秒，托管 8.8–28.5 秒 | x64 1:20–2:16，ARM64 1:14–1:53 | 3:41 |
| [Debian](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714874913) | 5.2–8.4 秒 | — | 4.7–34.9 秒 | 1:51–3:23 | 3:49 |
| [Linux Mint](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714880381) | 31.1–61.8 秒 | 6:43–6:59 | 5.2–7.2 秒 | 9:23–10:10 | 10:40 |
| [Pop!_OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714878105) | 10.1–11.4 秒 | 3:01–3:43 | 6.5–11.4 秒 | 5:12–6:13 | 6:38 |
| [Zorin OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714882847) | 11:28–12:08 | 4:14–4:49 | 两版 10.6 秒 | 18:35–19:35 | 20:02 |
| [Fedora](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714882642) | 75.8–98.6 秒 | — | 12.7–13.1 秒 | 2:57–3:31 | 4:11 |
| [openSUSE](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714883479) | 4.1–7.9 秒 | — | 8.6–15.8 秒 | 2:05–2:40 | 3:19 |
| [Arch Linux](https://github.com/Fu021/Clash-of-Rust/actions/runs/44714880300) | 5.6 秒 | — | 3.8 秒 | 2:32 | 2:54 |

本轮 Mint 的主要耗时是实际 ISO 安装，Zorin 两版的主要耗时是 ISO 传输。ISO 传输速率随上游镜像和 runner 变化。

2026-11-22 的跨版本验收中，Mint 两版 ISO 下载约 46／62 秒、安装约 437／442 秒；Zorin 两版下载约 202／382 秒、安装约 372／300 秒；openSUSE Tumbleweed 依赖准备约 7 秒。这些是不同运行条件下的单次样本，不代表稳定速度或受控加速对比。
