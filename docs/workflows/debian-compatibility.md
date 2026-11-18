# Debian 兼容性验证

[工作流索引](../workflows.md) · [安装验收](install-validation.md)

在 Actions 手动运行 `Debian Compatibility`，选择目标提交所在分支并填写同提交成功 CI 的 `run_id`。工作流复用 x64 候选 DEB、TUN 测试程序和依赖快照。

| Debian 版本 | 架构 | 官方 generic 云镜像构建 |
| --- | --- | --- |
| 12 Bookworm | x64 | `20261117-2665` |
| 13 Trixie | x64 | `20261116-2664` |

下载按[固定的 SHA512](../../scripts/linux_compatibility_targets.py)校验。VM 检查实际发行版、版本、架构及内核，使用 Debian 签名 APT 索引安装依赖，再验证实际安装后的系统代理、TUN、DNS 和关闭恢复。镜像与写入层留在当次 runner。

两个目标都通过后上传 `debian-compatibility-attempt-<attempt>`，绑定提交、同源 CI、输入附件身份和运行尝试。每个 VM 的 `linux-package-report-debian-<版本>-x64` 与最终回执保留三天。此工作流用于首轮平台验收。完整桌面会话、更新及卸载验收另行完成，通过后再评估公开支持与发布门禁。
