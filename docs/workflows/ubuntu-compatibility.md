# Ubuntu 兼容性验证

[工作流索引](../workflows.md) · [CI](ci.md) · [安装验收](install-validation.md) · [发布门禁](release.md#兼容性发布门禁) · [预装镜像](prepared-vm-images.md)

`Ubuntu Compatibility` 将 Ubuntu 20.04 x64／ARM64 的 5.4 GA 内核验证从常规 CI 拆出。常规 CI 保留 Ubuntu 20.04 用户态构建、22.04 原生安装及 Ubuntu latest；旧内核结果由本工作流单独提供。发布前两种架构均须成功，不能拿容器共享的宿主内核替代。

## 启动与验收

在 Actions 手动运行 `.github/workflows/ubuntu-compatibility.yml`，选择目标提交所在分支，填写同提交成功 CI 的 `run_id`。两个架构并行，分别使用 x64／ARM64 runner；不重新编译或打包。

`scripts/compatibility.py` 核对源 CI 的仓库、提交、成功终态以及 `package-linux-x64/arm64`、`linux-tun-test-x64/arm64`、`dependency-snapshot` 的唯一性、有效期和输入身份。VM 中再次检查 DEB、测试程序及包收据摘要，实际安装候选包并执行基础系统代理/TUN、DNS 和关闭恢复；扩展 GUI、多用户及更新专项不默认重复。

最后 `Record Ubuntu compatibility` 上传 `ubuntu-compatibility-attempt-<attempt>`，保留三天，绑定源 CI、提交、全部输入的 id/name/digest、运行 attempt 和检查范围。Publish Release 在解析和最终核验时检查它及两个架构的成功结果，缺失、失败、跳过或过期均阻止发布。新提交或不同 CI 产物不能沿用旧回执。

## 提速与缓存

- 预装镜像恢复、校验及 VM 启动，与候选包下载及本机摘要校验并行；guest 就绪后再传入本架构 DEB 和测试程序。包下载或校验失败仍使验证失败。
- VM 只接收 Python 验证脚本、必要元数据、CLI 参数契约、来源提交标记及本架构冻结 APT 索引；不再传入整个仓库、Git 历史或其他架构索引。来源标记仅在隔离 Actions guest 使用，宿主仍读取实际 Git 提交。
- 镜像命中后核对完整环境及 5.4 内核，避免重装测试工具；未命中或协议不可用时保持冷准备和同样的应用验证。
- 不跳过 SSH、cloud-init、包身份或真实网络检查，不假定增加 TCG vCPU 就能提速。ARM64 runner 没有 KVM 时仍如实标为 TCG。

预装镜像、官方云镜像和大型原包继续放私网；模板按 `vm/prepared` 协议管理，官方原包按 `upstream/files` 导入。系统盘、写入层和镜像二次归档均不得进入 GitHub Cache、artifact 或 Release；工作流不删除私网数据。更多协议、冷暖边界及历史耗时见[预装镜像说明](prepared-vm-images.md)。

`linux-package-report-ubuntu20-<架构>` 保留三天，记录冷／暖路径、KVM/TCG、源归档大小、环境准备、候选包下载、剩余等待、传输、安装验证与上传收尾；失败时保留已有报告。拆分和并行接线的静态回归不能证明具体节省多少时间，需新提交 Actions 对照后再更新实测。
