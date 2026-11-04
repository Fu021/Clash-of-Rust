# 三条工作流的缓存分工

[返回 CI](ci.md)。缓存用于缩短关键路径；实际恢复、编译和收尾耗时分别记录，绿色 job 不等于私网已完整保存。

## 编译对象只用 GitHub

CI 与 Memory Benchmark 的 Rust 对象只恢复和保存 GitHub Actions Cache；未命中正常编译，不查询私网 Rust 索引或分片，也不在 job 结束后镜像对象。Publish Release 继续复用同提交成功 CI 的构建附件，不重新编译 Rust。

GitHub 热缓存预算为 9 GiB，主分支保存，其他来源可读取兼容缓存。优先保留 Windows ARM64、macOS Intel、Windows x64、macOS ARM64，其次 Android ARMv7，再到其他 Android/Linux，Memory 最先让出空间。同平台/分支只留最近一代；容量不足可以淘汰，不向私网复制。`rust-cache` 自带的 Cargo registry/Git 数据与 Rust 对象一起管理，不再另建下载缓存。测试报告和安装包仅使用短期 artifact。

仓库自行管理的 Go 编译、Gradle 转换、工具安装目录、资源目录和 Docker 构建镜像不再占用 GitHub 缓存。官方 Actions 的引导及托管 runner 镜像不由本仓库接管；Tailscale 自带缓存也显式关闭。预算任务在运行取消时跳过，只操作 GitHub API。

## 原始下载：官方优先，私网回退

共享下载器先复用本 job 已校验文件，再尝试官方外网；有界重试仍失败时，才通过 `upstream/files` 精确查找原始文件。外层归档与原文件分别校验 SHA256，单个普通文件原子替换目标。未命中或校验失败保留已有文件，交原安装器报错或正常重建，不使用旧目录快照冒充原包。

| 路径 | 当前接入方式 |
| --- | --- |
| 桌面 mihomo 原始 ZIP/GZ、Geo DAT/MMDB | `build_support.download` 下载及回退；解压、资源准备在 runner 完成 |
| Android Geo 原始文件 | 同一共享下载器；不镜像 `offline-assets` 目录 |
| NSIS 原始安装程序 | 同一共享下载器；不镜像安装后的 NSIS 目录 |
| Ubuntu 20.04 独立 Python 与原始云镜像 | `linux_ci.download` 共用上述下载器；不保存构建容器、VM 系统盘或运行状态 |
| Go 模块原始 ZIP/MOD | `go mod verify` 后登记公开 Go proxy URL；仍由原生 Go 工具下载、校验并在本轮 artifact 内分发 |
| 宿主 APT 原始 DEB | 当轮签名索引确认 URL、大小及 SHA256；APT 先正常下载，失败后整批最多用 120 秒尝试私网，再交 APT 复核 |
| 容器和 guest 的 APT | 沿用签名索引校验；不向其传入私网凭据，不能据此声称已接入私网恢复 |
| Python/Tailscale 引导、Rust/Go/JDK、Gradle、Android SDK/NDK/模拟器、Maven | 保留官方 setup/包管理器安装；停止目录快照。未通过共享下载器或已验证原包登记的路径不计入私网覆盖 |

最后一行是当前实现边界，不能用“工具有官方下载链接”推断客户端已能从私网安装。需要新增原包适配时，先补明确来源、强校验、安装器入口及失败回退回归；不恢复目录快照或 runner 归档上传。Python/Tailscale 在私网登录前的引导还依赖 GitHub/runner，现有方案不是离线 runner。

## job 末尾只交下载清单

`upstream-cache` 在 runner 临时目录保存认证配置，`upstream-report` 在非取消任务末尾盘点已验证下载，提交 URL、逻辑位置、大小和 SHA256；没有文件内容、runner 绝对路径或凭据。记录不依赖临时原包继续存在，解压后删除临时原包不会丢失来源。相同内容按 SHA256 合并。

每 job 提交总预算 60 秒，Node/子进程还有 90 秒硬上限。不支持代拉、拒绝、超时或能力不足只记为未提交/不可用，绝不回退 `PUT` 归档。已存在、排队、已就绪、失败和未提交逐条写入 `upstream-downloads-<job>-<index>` artifact 与人可读 summary。`queued` 不是实际下载或恢复成功。

工作流不调用私网 DELETE，不同步 GitHub 淘汰结果，也不修改或上传 README.txt。服务端协作与保留范围见[协议](cache-server.md)和[可保留白名单](cache-retention.md)。
