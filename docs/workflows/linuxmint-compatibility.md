# Linux Mint 兼容验证

[工作流索引](../workflows.md) · [新平台完整验收](install-validation.md#新平台完整验收)

Linux Mint Compatibility 是独立的手动工作流，目标为 Cinnamon x64 的 21.3 和 22.3。两个目标各用一个固定 `ubuntu-24.04` runner。runner 下载并校验固定的官方 ISO，然后在没有网卡的一次性 VM 中从 ISO 安装 Linux Mint，安装服务日志随串口记录，不启用安装器的密码调试日志。镜像和磁盘随 runner 销毁。安装测试复用同提交成功 CI 的 x64 DEB、依赖快照和 TUN 测试程序。

从安装好的磁盘启动后才接入虚拟网卡。来宾使用 [Mint 官方 HTTPS 镜像](https://linuxmint.com/mirrors.php)读取 Mint 软件包索引，APT 使用系统自带密钥校验签名。Ubuntu 源沿用系统默认配置。

```bash
gh workflow run linuxmint-compatibility.yml --ref main -f run_id=<成功的CI编号>
```

首次验收覆盖原生 Cinnamon X11 和 Wayland 的窗口、托盘和三端共享后台，另在 X11 核对真实 LightDM 登录自启动、旧入口迁移和图形授权。网络与安装包括系统代理恢复、TUN/DNS、升级、卸载和数据保留。MATE、Xfce、ARM64 和物理设备在矩阵之外。

自启动和菜单验收结束后，测试移除专用账号的自启动夹具。原生 X11 和 Wayland 分别在重新启动后的登录中验收，启动独立 GUI 前确认该账号没有运行中的内核。Wayland 探针以该登录账号检查其私有运行时目录内的套接字，不放宽目录权限。

`linux-package-report-linuxmint-<版本>-x64` 包含来宾及安装诊断。两个安装验证 job 都成功后，工作流生成 `linuxmint-compatibility-attempt-<次数>` 回执，绑定提交、CI 附件身份和两份 ISO 的摘要。报告与回执保留 3 天。

首次完整回执来自 [Mint 44512435266](https://github.com/Fu021/Clash-of-Rust/actions/runs/44512435266)，使用 `a28aa277` 的 [CI 44509232155](https://github.com/Fu021/Clash-of-Rust/actions/runs/44509232155)。21.3 和 22.3 各从实际安装的系统盘启动，在不同引导中验证原生 Cinnamon X11 与 Wayland，并通过托盘、授权、三端共享后台、终端更新及升级卸载数据保留检查。
