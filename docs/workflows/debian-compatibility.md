# Debian 兼容性验证

[工作流索引](../workflows.md) · [安装验收](install-validation.md)

在 Actions 手动运行 `Debian Compatibility`，选择目标提交所在分支并填写同提交成功 CI 的 `run_id`。工作流按架构复用候选 DEB 和 TUN 测试程序，并使用同一份依赖快照。

| Debian 版本 | 架构 | 基础验收容器 |
| --- | --- | --- |
| 12 Bookworm | x64、ARM64 | `bookworm-20261104-slim` |
| 13 Trixie | x64、ARM64 | `trixie-20261104-slim` |

基础作业使用按[摘要固定](../../scripts/linux_compatibility_targets.py)的官方 Debian 容器，在同架构 Ubuntu 24.04 runner 上运行。检查容器的发行版、版本和架构，使用 Debian 签名 APT 索引安装候选，然后在独立网络命名空间验证系统代理、TUN、DNS 和退出恢复。TUN 使用 runner 的内核。

显式选择[跨版本更新](linux-expansion.md#linux-跨版本更新)时，沿用 SHA512 固定的 Debian 12 `20261117-2665` 与 Debian 13 `20261116-2664` generic 云镜像。两版双架构的真实桌面与包生命周期见[首轮完整验收](#首轮完整验收)。

隔离测试在模拟网络切换后和退出 TUN 后重启后台，让各组订阅直连回退检查从当前路由建立监控基线。保护状态下的后台退出只发送一次 `SIGTERM`，35 秒内未退出即记录进程状态和信号掩码。

四个目标都通过后上传 `debian-compatibility-attempt-<attempt>`，绑定提交、同源 CI、输入附件身份、容器摘要或跨版本更新模式、验收范围和运行尝试。各目标的 `linux-package-report-debian-<版本>-<架构>` 与最终回执保留三天。基础报告记录容器身份、runner 内核和耗时。

## 首轮完整验收

首次登录和后台登录各有 300 秒的状态等待预算。GDM 登录最多等待 180 秒，确认本次会话后，首登的候选启动最多等待 110 秒，后台登录最多等待 135 秒，后续托盘和窗口操作各限 45 秒。每次等待以该阶段剩余预算为上限。确认候选进程并读取到托盘初始化明确失败时立即结束验收。登录探针及来宾取证合计最多运行 420 秒，重启单独计时。

桌面阶段异常时，关闭来宾前通过 [QEMU 私有监控套接字](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html#command-screendump)尝试截图。实际 GNOME 登录后失败时也尝试使用 `gnome-screenshot`，以满足 [GNOME Shell 的截图调用方限制](https://github.com/GNOME/gnome-shell/blob/43.9/js/ui/screenshot.js)。报告尽可能保留启动轨迹、托盘宿主状态、登录日志和进程快照，截图失败时记录原因。

两个版本都在普通用户会话验证 X11、GNOME Wayland、托盘、授权允许与取消。以安装后的 `cor` 完成全套 PTY 测试，并检查 TUI 能读取 GUI 后台的订阅且共用同一个内核。登录专项让 VM 两次重启，由 GDM 启动实际 GNOME X11 会话，并通过 `dbus-user-session` 提供本次登录的用户总线。首次登录检查旧入口迁移，第二次检查后台启动、GNOME 的 AppIndicator 托盘注册和窗口隐藏，再验证托盘打开、关闭回托盘及应用菜单重开均保持同一后台 PID。登录阶段分别保留 boot ID、logind 会话、应用启动记录及托盘项。

终端更新回归验证授权拒绝、失败恢复及用户服务接续。另从同架构且固定 SHA256 的 `v0.7.2` 官方 DEB 升级到本轮候选包，核对设置与订阅身份，再通过 APT 卸载并核对保留的用户数据。首轮覆盖范围见[新平台完整验收](install-validation.md#新平台完整验收)。

提交 `a7167e2c` 使用 CI `44652374432` 取得 Debian 12/13 的 x64、ARM64 四目标回执，并经发布门禁复核。各目标均通过普通用户的桌面和终端探针、真实 GNOME X11 登录与 AppIndicator 托盘，以及从 `v0.7.2` 升级和卸载后的数据保留。两版 ARM64 使用 TCG，作业分别耗时 66 分 55 秒和 47 分 36 秒。[Debian Compatibility 44654293182](https://github.com/Fu021/Clash-of-Rust/actions/runs/44654293182)
