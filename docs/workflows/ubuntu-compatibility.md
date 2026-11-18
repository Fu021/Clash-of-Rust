# Ubuntu 兼容性验证

[工作流索引](../workflows.md) · [CI](ci.md) · [安装验收](install-validation.md) · [发布门禁](release.md#兼容性发布门禁)

`Ubuntu Compatibility` 按固定版本在原生内核 VM 验证同提交 DEB。覆盖项目的 Ubuntu 20.04 基线及处于标准支持期的 Ubuntu 分支。每个目标都使用官方云镜像并分别验收 x64、ARM64。

| Ubuntu 版本 | 目标 | 镜像 |
| --- | --- | --- |
| 20.04 LTS | x64、ARM64，必须运行 5.4 GA 内核 | Canonical focal，日期与 SHA256 由同源 CI 快照固定 |
| 22.04 LTS | x64、ARM64 | Canonical jammy `20261109` |
| 24.04 LTS | x64、ARM64 | Canonical noble `20261017` |
| 26.04 LTS | x64、ARM64 | Canonical resolute `20261108` |
| 26.10 | x64、ARM64 | Canonical stonking `20261105` |

版本范围依据 [Ubuntu 发行列表](https://ubuntu.com/project/docs/release-team/list-of-releases/)。固定镜像与摘要见 [Linux 验证目标](../../scripts/linux_compatibility_targets.py)。常规 CI 保留 20.04 用户态构建和 22.04 runner 上的安装回归。

## 启动与验收

在 Actions 手动运行 `Ubuntu Compatibility`，选择目标提交所在分支，填写同提交成功 CI 的 `run_id`。每个版本和架构使用独立 VM，最多并行四项，复用该次 CI 的 DEB 和测试程序。

`scripts/compatibility.py` 核对源 CI 的仓库、提交、成功终态以及 `package-linux-x64/arm64`、`linux-tun-test-x64/arm64`、`dependency-snapshot` 的唯一性、有效期和输入身份。VM 中再次检查 DEB、测试程序及包收据摘要，实际安装候选包并执行基础系统代理/TUN、DNS 和关闭恢复；扩展 GUI、多用户及更新专项不默认重复。

最后 `Linux / Record compatibility` 上传 `ubuntu-compatibility-attempt-<attempt>`，保留三天，绑定源 CI、提交、全部输入的 id/name/digest、运行 attempt 和检查范围。Publish Release 在解析和最终核验时检查它及整个矩阵的成功结果。缺失、失败、跳过或过期均阻止发布。新提交或不同 CI 产物不能沿用旧回执。

## 镜像与软件包来源

- 每个目标的云镜像与测试用 Python 均校验 SHA256。镜像下载、校验及 VM 启动与候选包下载并行。guest 就绪后再传入本架构 DEB 和测试程序。
- VM 只接收 Python 验证脚本、必要元数据、CLI 参数契约、来源提交标记及本架构冻结 APT 索引；不再传入整个仓库、Git 历史或其他架构索引。来源标记仅在隔离 Actions guest 使用，宿主仍读取实际 Git 提交。
- 20.04 优先用 CI 冻结的 Ubuntu 官方签名 APT 索引和软件包摘要准备工具。冻结计划不可用时，记录回退并使用当前签名索引。较新版本直接使用该版本的当前签名索引。
- 安装测试等待 SSH 和 cloud-init 就绪，核对实际发行版、版本、架构、内核及候选包身份，再执行网络检查。ARM64 runner 没有 KVM 时记录为 TCG。

云镜像、系统盘及写入层仅保存在当次 runner；CI 附件携带冻结 APT 输入，兼容验证附件携带诊断报告和回执。

`linux-package-report-ubuntu-<版本>-<架构>` 保留三天，记录云镜像身份、APT 路径、KVM/TCG、环境准备、候选包下载、传输及安装验收耗时。失败时保留已有报告。
