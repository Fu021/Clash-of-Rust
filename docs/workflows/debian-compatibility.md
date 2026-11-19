# Debian 兼容性验证

[工作流索引](../workflows.md) · [安装验收](install-validation.md)

在 Actions 手动运行 `Debian Compatibility`，选择目标提交所在分支并填写同提交成功 CI 的 `run_id`。工作流按架构复用候选 DEB 和 TUN 测试程序，并使用同一份依赖快照。

| Debian 版本 | 架构 | 官方 generic 云镜像构建 |
| --- | --- | --- |
| 12 Bookworm | x64、ARM64 | `20261117-2665` |
| 13 Trixie | x64、ARM64 | `20261116-2664` |

下载按[固定的 SHA512](../../scripts/linux_compatibility_targets.py)校验。VM 检查实际发行版、版本、架构及内核，使用 Debian 签名 APT 索引安装依赖，再验证实际安装后的系统代理、TUN、DNS 和关闭恢复。ARM64 在 `ubuntu-24.04-arm` runner 上运行，来宾使用 Virtio GPU；无 KVM 时，桌面来宾按宿主可用核数分配至多四个 TCG vCPU。报告记录 KVM 或 TCG 和阶段耗时。TCG 下依赖安装和完整应用测试使用更长的有界等待。镜像与写入层留在当次 runner。

工作流安装 QEMU 后检查 `/dev/kvm`：存在时开放本次临时 runner 的访问权限，来宾启动时优先使用 KVM；不存在时使用 TCG。已验证的 GitHub ARM64 runner 未提供该设备，x64 运行曾使用 KVM。GitHub 的 [ARM64 KVM 说明](https://github.com/actions/runner-images/issues/14062)将其归因于托管宿主未向 runner 暴露嵌套虚拟化；这不是 ARM 架构本身的限制。GitHub 不保证托管 runner 的嵌套虚拟化可用。

两种架构都最多等待 GDM 登录 180 秒。确认本次会话后，首次登录的窗口与托盘验收最多等待 110 秒，后台登录最多等待 135 秒。确认候选进程并读取到托盘初始化明确失败时立即结束该阶段。后续托盘和窗口操作各限 45 秒。桌面阶段异常时，在关闭来宾前通过 [QEMU 私有监控套接字](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html#command-screendump)尝试截图；实际 GNOME 登录后失败时还尝试使用来宾的 `gnome-screenshot` 截图，[GNOME Shell 会限制截图接口的调用方](https://github.com/GNOME/gnome-shell/blob/43.9/js/ui/screenshot.js)。同时保留启动轨迹、托盘宿主状态、登录日志和进程快照，截图失败时记录原因。

两个版本都在普通用户会话验证 X11、GNOME Wayland、托盘、授权允许与取消。以安装后的 `cor` 完成全套 PTY 测试，并检查 TUI 能读取 GUI 后台的订阅且共用同一个内核。登录专项让 VM 两次重启，由 GDM 启动实际 GNOME X11 会话，并通过 `dbus-user-session` 提供本次登录的用户总线。首次登录检查旧入口迁移，第二次检查后台启动、GNOME 的 AppIndicator 托盘注册和窗口隐藏，再验证托盘打开、关闭回托盘及应用菜单重开均保持同一后台 PID。登录阶段分别保留 boot ID、logind 会话、应用启动记录及托盘项。

终端更新回归验证授权拒绝、失败恢复及用户服务接续。另从同架构且固定 SHA256 的 `v0.7.2` 官方 DEB 升级到本轮候选包，核对设置与订阅身份，再通过 APT 卸载并核对保留的用户数据。首轮覆盖范围见[新平台完整验收](install-validation.md#新平台完整验收)。

四个目标都通过后上传 `debian-compatibility-attempt-<attempt>`，绑定提交、同源 CI、输入附件身份、完整验收范围和运行尝试。每个 VM 的 `linux-package-report-debian-<版本>-<架构>` 与最终回执保留三天。真实 VM 结果用于评估公开支持与发布门禁。

首轮完整验收覆盖 Debian 12 x64（内核 `6.1.0-56-amd64`）和 Debian 13 x64（内核 `6.12.117+deb13-amd64`）。普通用户运行覆盖 GNOME X11 与 Wayland，实际 GDM 登录和 AppIndicator 托盘覆盖 GNOME X11。安装生命周期覆盖从 `v0.7.2` 升级和卸载后的数据保留。证据见 [Debian Compatibility 44330024889](https://github.com/Fu021/Clash-of-Rust/actions/runs/44330024889)。
