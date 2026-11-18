# Linux Mint 兼容验证

[工作流索引](../workflows.md) · [新平台完整验收](install-validation.md#新平台完整验收)

Linux Mint Compatibility 是独立的手动工作流，目标为 Cinnamon x64 的 21.3 和 22.3。两个目标各用一个固定 `ubuntu-24.04` runner。runner 下载并校验固定的官方 ISO，然后在一次性 VM 磁盘上安装 Linux Mint。镜像和磁盘随 runner 销毁。安装测试复用同提交成功 CI 的 x64 DEB、依赖快照和 TUN 测试程序。

```bash
gh workflow run linuxmint-compatibility.yml --ref main -f run_id=<成功的CI编号>
```

首次验收覆盖原生 Cinnamon X11 和 Wayland 的窗口、托盘和三端共享后台，另在 X11 核对真实 LightDM 登录自启动、旧入口迁移和图形授权。网络与安装包括系统代理恢复、TUN/DNS、升级、卸载和数据保留。MATE、Xfce、ARM64 和物理设备在矩阵之外。

`linux-package-report-linuxmint-<版本>-x64` 包含来宾及安装诊断。两个安装验证 job 都成功后，工作流生成 `linuxmint-compatibility-attempt-<次数>` 回执，绑定提交、CI 附件身份和两份 ISO 的摘要。报告与回执保留 3 天。
