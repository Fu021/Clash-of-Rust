# Ubuntu 兼容性验证

[工作流索引](../workflows.md) · [CI](ci.md) · [安装验收](install-validation.md) · [发布门禁](release.md#兼容性发布门禁)

`Ubuntu Compatibility` 在四个 LTS 版本验收同提交 DEB。ARM64 始终在同架构 runner 中运行按摘要固定的 Canonical 容器。容器使用 runner 的内核。

| Ubuntu 版本 | x64 | ARM64 |
| --- | --- | --- |
| 20.04 LTS | focal 容器 | focal 容器 |
| 22.04 LTS | `ubuntu-22.04` | jammy 容器 |
| 24.04 LTS | `ubuntu-24.04` | noble 容器 |
| 26.04 LTS | `ubuntu-26.04` | resolute 容器 |

Ubuntu 20.04 的 GitHub 托管镜像已于 2025 年 4 月 15 日退役。显式选择 x64 跨版本更新时使用同源 CI 快照固定的 Canonical 云镜像和 5.4 GA 内核。依据 [GitHub 托管 runner 清单](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)及 [Ubuntu 20.04 runner 退役公告](https://github.blog/changelog/2025-01-15-github-actions-ubuntu-20-runner-image-brownout-dates-and-other-breaking-changes/)。

托管验收覆盖 GitHub 镜像实际运行的 Azure 内核。22.04 x64 因此从先前云镜像的 5.15 generic 转为 6.8 Azure。GitHub 预计于 **2027 年 4 月 17 日**退役 22.04 的两个标签，退役前须重新安排 22.04 兼容与依赖这些标签的常规 CI 和内存基准。依据 [GitHub 镜像说明](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2204-Readme.md)及[退役公告](https://github.com/actions/runner-images/issues/14254)。

## 启动与验收

在 Actions 手动运行 `Ubuntu Compatibility`，选择目标提交所在分支，填写同提交成功 CI 的 `run_id`。每个版本和架构使用独立 job，最多并行四项，复用该次 CI 的 DEB 和测试程序。

`scripts/compatibility.py` 核对源 CI 的仓库、提交、成功终态以及 `package-linux-x64/arm64`、`linux-tun-test-x64/arm64`、`dependency-snapshot` 的唯一性、有效期和输入身份。各执行路径均复核 DEB、测试程序及包收据摘要，核对实际发行版版本与架构，实际安装候选包并执行基础系统代理/TUN、DNS 和关闭恢复。扩展 GUI、多用户及更新专项按常规 CI 策略验证。

Ubuntu [20.04 ARM64](https://github.com/Fu021/Clash-of-Rust/actions/runs/45366946510/job/138351129788) 和 [22.04 ARM64](https://github.com/Fu021/Clash-of-Rust/actions/runs/45373616570/job/138404196661) 已通过首轮全量验收，包括 GUI/TUI/CLI、两次真实 GDM 登录、包升级卸载和公开版本更新。

GitHub 的 ARM64 作业仅验证基础安装及流量，即使指定跨版本更新参数也不启动虚拟机或执行更新专项。已有完整桌面验收是独立的历史证据。

托管 runner 创建临时 GUI 账户时跳过登录历史初始化，单次最多等待 90 秒。创建超时后按用户名、目录和 UID 清理可能已创建的账户，再尝试，最多三次。Ubuntu 20.04 和 22.04 的图形更新夹具使用旧版 polkit 的 `.pkla`，仍执行真实密码认证和安装失败恢复。

旧版预发布确认框的单行 OCR 未识别到完整标签时，在同一启用按钮的范围内复核灰度文字。倒计时按钮不具备夹具设置的启用强调色。

最后 `Linux / Record compatibility` 上传 `ubuntu-compatibility-attempt-<attempt>`，保留三天，绑定源 CI、提交、全部输入的 id/name/digest、运行 attempt 和基础检查范围。本地全量验收保存单独的来宾报告。Publish Release 在解析和最终核验时检查回执及整个矩阵的成功结果。缺失、失败、跳过或过期均阻止发布。新提交或不同 CI 产物不能沿用旧回执。

## 镜像与软件包来源

- VM 目标的云镜像与测试用 Python 均校验 SHA256。镜像下载、校验及 VM 启动与候选包下载并行。guest 就绪后再传入本架构 DEB 和测试程序。
- VM 只接收 Python 验证脚本、必要元数据、CLI 参数契约、来源提交标记及本架构冻结 APT 索引。来源标记用于隔离 Actions 来宾和[一次性本地来宾](linux-expansion.md#本地一次性来宾)，宿主读取实际 Git 提交。
- 20.04 优先用 CI 冻结的 Ubuntu 官方签名 APT 索引和软件包摘要准备工具。冻结计划不可用时，记录回退并使用当前签名索引。较新版本直接使用该版本的当前签名索引。
- VM 安装测试等待 SSH 和 cloud-init 就绪，核对实际发行版、版本、架构、内核及候选包身份，再执行网络检查。

云镜像、系统盘及写入层仅保存在当次 runner；CI 附件携带冻结 APT 输入，兼容验证附件携带诊断报告和回执。

`linux-package-report-ubuntu-<版本>-<架构>` 保留三天。托管 job 记录 runner 标签、镜像版本、实际内核及验收耗时。容器 job 记录固定基础镜像和实际 runner 内核。VM job 记录云镜像身份、APT 路径、KVM/TCG、环境准备、候选包下载、传输及安装验收耗时。失败时保留已有报告。
