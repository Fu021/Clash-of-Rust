# Linux Mint 兼容验证

[工作流索引](../workflows.md) · [新平台完整验收](install-validation.md#新平台完整验收)

Linux Mint Compatibility 是独立的手动工作流，目标为 21.3 和 22.3 x64。两个目标均使用 `ubuntu-24.04` runner 上的精简用户态容器，基于固定的 Jammy 或 Noble 镜像。容器安装 SHA256 固定的 Mint `base-files`，使用 [Mint 官方签名软件源](https://linuxmint.com/mirrors.php)及对应 Ubuntu 源。安装测试复用同提交成功 CI 的 x64 DEB、依赖快照和 TUN 测试程序。

显式跨版本更新使用 Cinnamon 官方 ISO。安装器在没有网卡的一次性 VM 中从 ISO 安装系统，从磁盘启动后接入虚拟网卡。SSH 首次准备采用 5 秒、15 秒有界重试。

```bash
gh workflow run linuxmint-compatibility.yml --ref main -f run_id=<成功的CI编号>
```

常规工作流安装候选 DEB，运行系统代理与 TUN 检查，验证 DNS 和退出恢复。`linux-package-report-linuxmint-<版本>-x64` 记录容器、runner 内核与安装诊断。两个目标都成功后生成 `linuxmint-compatibility-attempt-<次数>`，绑定提交、CI 附件身份、基础镜像和 Mint 软件包摘要。显式跨版本更新的回执绑定两份 ISO。报告与回执保留 3 天。

## 首轮完整验收

首次验收覆盖原生 Cinnamon X11 和 Wayland 的窗口、托盘和三端共享后台，另在 X11 核对真实 LightDM 登录自启动、旧入口迁移和图形授权。22.3 的探针等 Cinnamon 进入运行态后再触发认证。网络与安装包括系统代理恢复、TUN/DNS、升级、卸载和数据保留。MATE、Xfce、ARM64 和物理设备在矩阵之外。

提交 `280d5679` 的 [Mint 两版复验](https://github.com/Fu021/Clash-of-Rust/actions/runs/44642545840)完成上述检查，回执绑定同提交 [CI 44639212301](https://github.com/Fu021/Clash-of-Rust/actions/runs/44639212301)。

自启动和菜单验收结束后，测试移除专用账号的自启动夹具。原生 X11 和 Wayland 分别在重新启动后的登录中验收，启动独立 GUI 前确认该账号没有运行中的内核。Wayland 探针以该登录账号检查其私有运行时目录内的套接字，不放宽目录权限。

首次完整回执来自 [Mint 44512435266](https://github.com/Fu021/Clash-of-Rust/actions/runs/44512435266)，使用 `a28aa277` 的 [CI 44509232155](https://github.com/Fu021/Clash-of-Rust/actions/runs/44509232155)。21.3 和 22.3 各从实际安装的系统盘启动，在不同引导中验证原生 Cinnamon X11 与 Wayland，并通过托盘、授权、三端共享后台、终端更新及升级卸载数据保留检查。
