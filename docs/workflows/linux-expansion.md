# Linux 扩展兼容验证

每个发行版有独立 `workflow_dispatch` 入口，指定同提交成功的 `CI` 运行编号 `run_id`。普通 push/PR 不启动这些 VM。CI 每个 Linux 架构只编译一次并生成 DEB，另从 x64 DEB 组装 Fedora 和 openSUSE 所需的 RPM。发布时按 [Linux 范围](release.md)要求相应回执。

每次兼容运行实际安装同源候选包，执行和 CI 相同的基础系统代理和 TUN 流量检查，包含 DNS 与退出恢复。回执列出 `system_proxy` 与 `tun`。新增平台的完整验收可在目标配置中启用 `full_desktop`，覆盖 [三端、桌面登录与包生命周期](install-validation.md#新平台完整验收)。

| 手动入口 | 固定来宾及架构 | 原生安装路径 |
| --- | --- | --- |
| Pop!_OS Compatibility | System76 22.04 build 58 和 24.04 build 20（x64） | 离线 `distinst` 后从系统盘启动，用 APT 安装 DEB |
| Zorin OS Compatibility | Zorin 17.3 Core r2、18.1 Core（x64） | 离线 Ubiquity 安装系统，再用 APT 安装 DEB |
| Fedora Compatibility | Fedora Cloud Base Generic 44-1.7、45-1.4（x64） | DNF 安装同源 RPM |
| openSUSE Compatibility | Leap 16.0 Build 18.95、16.1 Build 3.20、Tumbleweed 20261119（x64） | 可写 Cloud VM，Zypper 安装同源 RPM |
| Arch Linux Compatibility | 官方 Cloud 20261115.625457（x64） | `makepkg` 从同源 DEB 生成原生包，再由 `pacman` 安装 |

镜像 URL、完整摘要与架构由 `scripts/linux_compatibility_targets.py` 固定并校验，滚动版使用相应快照仓库。openSUSE 原站返回 404 或 410 时，从[列名镜像站](https://mirrors.opensuse.org/)尝试下载相同文件，沿用固定摘要，报告记录实际使用的地址。若两处均已移除，从官方索引核对替代镜像、摘要及匹配的签名软件源，再更新目标并复验。这里的 QEMU 来宾统一用 x64 `ubuntu-24.04` runner。

新增或替换目标时核对发行版的维护期限和镜像来源。上游宣布停止维护后，复查本项目是否仍需该目标作为明确的兼容基线。替换受支持目标须先验证替代版本，再同步目标矩阵、公开支持范围和发布门禁。滚动版每次更换快照都记录日期与摘要，使用配套的软件源并完整复验。Ubuntu 的最低内核基线见 [Ubuntu 兼容验证](ubuntu-compatibility.md)。

排查时先对照各目标 `linux-vm.json` 的 `failed_stage`，区分输入下载、来宾启动、源码传入、依赖准备和已安装候选验收。报告记录 runner 标签、镜像版本、KVM 或 TCG，以及实际内核和阶段耗时。基础运行的托管 runner、云镜像 VM、ISO VM 分别限时 30、45、75 分钟。来宾引导或候选检查失败时保留串口日志，并尝试通过私有 QEMU 监控截图。

`linux-vm.json` 单列官方镜像与 Python 工具的下载校验时间。ISO 来宾分别记录安装器和安装盘启动的耗时。安装器失败时记录 `iso_install` 失败阶段和已用时间。候选检查内部的依赖准备由 `compatibility-environment.json` 单列，VM 在来宾归档内保存，直接使用托管 runner 时随报告上传。

隔离网络检查分别记录 root CLI、普通账户矩阵和原生代理/TUN 流量阶段。开启 ARM64 扩展账户检查时，命名空间为环境准备和账户检查留出 560 秒，再计入流量探针的独立时限。流量探针通常最多 240 秒，启用代理基准时为 360 秒。各阶段的子检查仍保留自己的超时。外层到达时限时终止整个测试 PID 命名空间，避免后台进程继续占用 SSH 通道、延迟失败截图和来宾清理。进入命名空间后恢复 `SIGTERM` 和 `SIGINT` 的处理，避免旧版 `unshare` 让 DNS 和总线夹具继承忽略信号的设置。

## 完整验收与诊断

首轮完整验收覆盖普通用户 GUI/TUI/CLI、共享后台、代理/TUN/DNS、图形授权拒绝与允许、登录自启动迁移、托盘与菜单恢复同一进程、原生包升级及卸载后的数据保留。COSMIC 保留 TUN 与应用代理，系统代理请求返回明确错误并保留原有路由。Fedora、openSUSE、Arch 在 GNOME 使用 AppIndicator 扩展验证托盘。RPM 和 Arch 的首个升级夹具使用经 SHA256 校验的公开旧 DEB 重新封装。

复查 COSMIC 登录、旧自启动入口迁移和窗口恢复时，在 Pop!_OS 24.04 目标启用 `full_desktop` 并运行 Pop!_OS Compatibility。现场日志可按[桌面启动诊断](../diagnostics.md#桌面启动与托盘)核对。

Fedora 的隔离 X11 授权探针使用发行版提供的 `xfce-polkit`。该代理可在收到 [PAM 密码请求](https://github.com/ncopa/xfce-polkit/blob/6d3282cc1734c305850d48f5bf4b4d94e88885e9/src/xfce-polkit-listener.c)前显示窗体。验收同时等待可见窗体和本次会话的密码请求，取消后重新注册代理，原始代理调试输出只用于探测。来宾先从 Fedora 官方仓库更新 `pam`、`liblastlog2` 和 `selinux-policy-targeted`。后两者分别包含 [Fedora 44 PAM 模块加载修复](https://bugzilla.redhat.com/show_bug.cgi?id=2453457)及 [polkit socket 助手域转换修复](https://github.com/fedora-selinux/selinux-policy/commit/8ad5d5112fa8b47426e0e6674616d5577f825fde)。失败报告记录已装版本和相关审核事件。

隔离 GNOME 探针看到新的 Wayland 套接字后才查询 Shell 的 D-Bus 对象，合成器就绪限时为 60 秒。GNOME 50 [完成 UI 初始化后才导出该对象](https://github.com/GNOME/gnome-shell/blob/50.6/js/ui/main.js#L163-L176)。进程提前退出会立即结束验收。候选阶段出错或超时均尝试保存 QEMU 画面，嵌套桌面状态另见来宾日志。

Debian、openSUSE 和 Arch 的隔离 X11 探针使用 MATE 授权代理。在可见窗体和 [polkit 的隐藏输入请求](https://github.com/polkit-org/polkit/blob/127/src/polkitagent/polkitagentsession.c)都就绪后才发送操作；每次重启代理同时清空私有探测输出。报告记录验收结果，原始认证调试流只在探针运行期间使用。

COSMIC 登录探针在 logind 标记桌面会话为活动状态后，等待[来宾的 polkit 日志](https://github.com/polkit-org/polkit/blob/124/src/polkitbackend/polkitbackendinteractiveauthority.c#L2424-L2442)确认 `cosmic-osd` 已为同一次登录注册认证代理，再发起授权。宿主依次用原图、去黑边灰度图和全幅灰度图定位实际授权对话框。取消阶段点击 `Cancel`，接受阶段点击 `Password` 输入框，再用 [COSMIC 密码框的回车提交](https://github.com/pop-os/cosmic-osd/blob/master/src/components/polkit_dialog.rs)完成图形授权。两阶段均匹配应用授权文本，验收确认取消保留原权限、接受后安装助手取得预期权限。实际登录通过窗口列表和托盘注册确认应用状态。关闭操作要求窗口列表中恰有一个匹配项且合成器声明支持关闭，随后确认窗口消失、原 PID 仍在。原生 Wayland GUI 阶段单独检查窗口尺寸。宿主捕获授权取证和失败画面，探针提前退出时另存来宾结果及有限长度的 `pkexec` 输出。

COSMIC 的[合成器刷新](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs)异步发送几何事件。探针识别出目标窗口后最多等待一秒接收几何，原生 Wayland 验收仍检查 800×450 的最低尺寸。

Tumbleweed 在来宾关闭预设仓库，只启用 openSUSE 历史目录中与云镜像日期一致的签名仓库，并禁用自动刷新。Tumbleweed 的 [GDM 打包补丁](https://api.opensuse.org/public/source/openSUSE:Factory/gdm/gdm-sysconfig-settings.patch)使 `/etc/sysconfig/displaymanager` 的 `DISPLAYMANAGER_AUTOLOGIN` 优先于 `custom.conf`。在一次性 VM 仅替换该键的值，保留实际 GDM 启动 Wayland 会话与登录自启动的断言。

真实 GNOME Wayland 登录前，探针通过短生命周期的 D-Bus 会话写入扩展设置，指定 GIO 的本地文件实现以免启动 GVfs。若 SSH 阶段超时而 `desktop-login/prepare.json` 已写入成功结果，核对该命令是否启动了仍持有 SSH 输出的进程。真实 X11 登录中，窗口枚举遇到已销毁的 X11 客户窗口时短暂重试，成功读取窗口列表后再判断是否前台显示。

RPM 与 Arch 安装由各自包管理器更新，应用不调用 DEB 安装器覆盖其文件。Arch 配方和 `.SRCINFO` 使用本次已验证的候选，未提交 AUR，也不接管第三方包。

原生包的预发布版本字段使用 RPM 的 `~` 或 pacman 的 `pre.` 标记，版本后缀中的连字符转换为 `+`。RPM 发行附件名使用原始发行版本，Arch 候选包名使用原生 `pkgver`。兼容来宾用原生比较器校验预发布编号递增及正式版顺序。隔离 DNS 夹具按来宾实际存在的系统 CA 文件装载临时测试根，并在同一挂载命名空间中恢复。夹具仅给节点引导的正向解析结果 30 秒 TTL，以减少 ARM64 TCG 上重复的加密握手。测试仍要求每次冷启动重新解析节点，并验证 UDP、TCP 和 DNS 未旁路。

提交 `0ccca669` 使用 CI `44696750124` 完整验收当前矩阵中的 Ubuntu、Debian、Linux Mint、Zorin OS、Fedora、openSUSE 和 Arch Linux。Pop!_OS 24.04 的 COSMIC 授权探针提前退出，补上认证代理就绪检查后用 `8999697d` 的 CI `44705538634` 复验。Pop!_OS 22.04 GNOME 和 24.04 COSMIC 均通过，并核对真实登录、图形授权、托盘及菜单复用、23 项 TUI PTY、原生升级和卸载保留数据。两轮合计覆盖当前 21 个目标。新发布按实际范围取得对应提交的回执。[首轮 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/44696750124) · [Pop!_OS 完整复验](https://github.com/Fu021/Clash-of-Rust/actions/runs/44709399603)

更早的完整验证中，`a433d221` 使用 CI `44670222142` 完成 Pop!_OS、Zorin OS 和 Arch Linux 的 x64 目标，均使用 KVM。`280d5679` 使用 CI `44639212301` 完成 openSUSE 两版 Leap 和 Tumbleweed 的 x64、ARM64 目标，分别使用 KVM 和 TCG。历史回执用于追溯相应候选与镜像的验证结果。[Pop!_OS](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672608619) · [Zorin](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672632359) · [Arch](https://github.com/Fu021/Clash-of-Rust/actions/runs/44672635871) · [openSUSE](https://github.com/Fu021/Clash-of-Rust/actions/runs/44642538483)

提交 `3b62e2ef` 使用 CI `44625972854` 完成 [Fedora 44/45 x64](https://github.com/Fu021/Clash-of-Rust/actions/runs/44628532038)的完整兼容验收，覆盖图形授权、真实登录与托盘、三端和 RPM 升级及卸载。该历史回执只覆盖 x64。

提交 `0f48d91d` 使用 CI `44580277926` 完成 [openSUSE 三个 x64 目标](https://github.com/Fu021/Clash-of-Rust/actions/runs/44584087058) 的完整兼容验收，回执经发布门禁复核。Tumbleweed 的实际 GNOME 登录、托盘和菜单复用、23 项真实 PTY 与 RPM 升级和卸载均通过。

## 精简验收耗时

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
