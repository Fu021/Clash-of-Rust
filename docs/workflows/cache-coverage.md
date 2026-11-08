# 三条工作流的缓存分工

[返回 CI](ci.md)。缓存用于缩短关键路径；实际恢复、编译和收尾耗时分别记录，绿色 job 不等于私网已完整保存。

## 编译对象只用 GitHub

CI 与 Memory Benchmark 的 Rust 对象只恢复和保存 GitHub Actions Cache；未命中正常编译，不查询私网 Rust 索引或分片，也不在 job 结束后镜像对象。Publish Release 继续复用同提交成功 CI 的构建附件，不重新编译 Rust。

GitHub 热缓存预算为 9 GiB。CI 的主分支、开发分支及同仓库 PR 成功后均尝试保存各平台的新编译缓存；Memory Benchmark 不再限定主分支保存。精确键已存在时由不可变缓存规则复用，未命中正常编译；fork PR 仍只读，不增加缓存保存或私网权限。Publish Release 复用 CI 候选包，不重新编译或另建 Rust 缓存。macOS ARM64 按维护者要求最高优先，减少稀缺 runner 分配后重新编译的损失；缓存不会缩短 runner 排队。其余按最近六次成功完整 CI 的“Rust 缓存恢复＋编译/测试”中位耗时排序：

| 保留优先级 | 平台 | 中位耗时（分钟） |
| --- | --- | --- |
| 1 | macOS ARM64 | 9.2 |
| 2 | macOS Intel | 16.6 |
| 3 | Windows ARM64 | 12.3 |
| 4 | Windows x64 | 11.9 |
| 5 | Android ARM64 | 7.3 |
| 6 | Android x64 | 7.0 |
| 7 | Android ARMv7 | 6.7 |
| 8 | Linux x64 | 5.8 |
| 9 | Linux ARM64 | 5.3 |
| 10 | Memory Benchmark | 最先让出空间 |

2026-11-06 核对的来源为 `42063675832`、`42044579484`、`42031368400`、`41979081254`、`41973243758`、`41966802804`。桌面计 Rust 恢复、兼容 workspace 恢复及编译/测试，Android 计 Rust 恢复和应用/内核编译；排除排队、私网接入、模拟器测试与清单收尾。它们并非纯 Rust 编译或冷/热对照，不能当作缓存净节约。策略经审阅后固定，不随每轮数据自动变化。

同一逻辑键组/分支只留最近一代；Windows/macOS 不同源码的 workspace 键归入同一平台族，保留最新兼容对象并共享总预算。容量不足可以淘汰，不向私网复制。`rust-cache` 自带的 Cargo registry/Git 数据与 Rust 对象一起管理，不再另建下载缓存。测试报告和安装包仅使用短期 artifact。

仓库自行管理的 Go 编译、Gradle 转换、工具安装目录、资源目录和 Docker 构建镜像不再占用 GitHub 缓存。官方 Actions 的引导及托管 runner 镜像不由本仓库接管。macOS 的官方 Tailscale Action 会源码编译 CLI/daemon，因此只在 macOS 开启该 Action 自带的 GitHub 二进制缓存，避免每个 CI、打包和内存 job 重编；其他平台保持关闭。此引导缓存按架构/分支保留最近版本，与 Intel Rust 同级、低于 ARM64 Rust，仍计入 9 GiB 总预算，不上传私网。预算任务在运行取消时跳过，只操作 GitHub API。

性能分析须将私网接入与 Rust 恢复分开：[CI 41973243758](https://github.com/Fu021/Clash-of-Rust/actions/runs/41973243758) 的 macOS Intel 分别用时 376 秒接入 Tailscale、74 秒恢复 GitHub 对象、269 秒编译与测试；ARM64 分别为 135、28、172 秒。两种架构均恢复有效 workspace，对 452 个源码/目录时间戳完成核验且没有变化的 crate；官方 action 在 macOS 从源码构建 Tailscale 的行为见[上游问题](https://github.com/tailscale/github-action/issues/287)。这些单次结果不能据总 job 时间认定 Rust 缓存无效，也不能代替冷编译对照。

## 原始下载：官方优先，私网回退

共享下载器先复用本 job 已校验文件，再尝试官方外网；有界重试仍失败时，才通过 `upstream/files` 精确查找原始文件。外层归档与原文件分别校验 SHA256，单个普通文件原子替换目标。未命中或校验失败保留已有文件，交原安装器报错或正常重建，不使用旧目录快照冒充原包。

| 路径 | 当前接入方式 |
| --- | --- |
| 宿主下载的桌面 mihomo 原始 ZIP/GZ、Geo DAT/MMDB | `build_support.download` 下载及回退；解压、资源准备在 runner 完成 |
| Linux 构建容器内的 mihomo/Geo | 先在宿主用同一冻结快照下载及校验，必要时私网回退，再挂载已准备资源；容器不接收私网凭据。容器后续仍复核资源身份 |
| Android Geo 原始文件 | 同一共享下载器；不镜像 `offline-assets` 目录 |
| NSIS 原始安装程序 | 同一共享下载器；不镜像安装后的 NSIS 目录 |
| Ubuntu 20.04 独立 Python 与原始云镜像 | `linux_ci.download` 共用上述下载器；不保存构建容器、VM 系统盘或运行状态 |
| Go 模块原始 ZIP/MOD | `go mod verify` 后登记公开 Go proxy URL；仍由原生 Go 工具下载、校验并在本轮 artifact 内分发 |
| 宿主 APT 原始 DEB | 当轮签名索引确认 URL、大小及 SHA256；APT 先正常下载，失败后整批最多用 120 秒尝试私网，再交 APT 复核 |
| 容器和 guest 的 APT | 沿用签名索引校验；不向其传入私网凭据，不能据此声称已接入私网恢复 |
| Python/Tailscale 引导、Rust/Go/JDK、Gradle、Android SDK/NDK/模拟器、Maven | 保留官方 setup/包管理器安装；停止目录快照。未通过共享下载器或已验证原包登记的路径不计入私网覆盖 |

最后一行是当前实现边界，不能用“工具有官方下载链接”推断客户端已能从私网安装。需要新增原包适配时，先补明确来源、强校验、安装器入口及失败回退回归；不恢复目录快照或 runner 归档上传。Python/Tailscale 在私网登录前的引导还依赖 GitHub/runner，现有方案不是离线 runner。

## 尽早提交完整清单，收尾保留回执

`upstream-cache` 在 runner 临时目录保存认证配置；三条工作流的 `upstream-report` 均使用[持久清单协议](cache-server.md#持久清单补拉)。共享 Go、原生资源、Linux 原包、VM 和内存基准宿主依赖准备完成后设置检查点，尽早提交当前全部已校验文件；收尾追加后续下载并查询已接收清单。只交官方 URL、大小、SHA256 和运行序号，没有文件内容、runner 绝对路径或凭据。记录不依赖临时原包继续存在，相同内容按 SHA256 合并。

检查点预算 20 秒、不轮询；末尾总预算 60 秒，其中最多 15 秒用于一次分页查询，各元数据请求最多 8 秒，外层子进程分别限 30/90 秒。先保存全部批次，再提交全部批次，最后查询；不会等待单 worker 下载完全部原包。持续轮询不重置失败任务，断线重发固定请求与序号。预算不足、拒绝或能力缺失保留未提交项，绝不改为 `PUT` 归档。只有旧能力时明确使用兼容单项导入，不宣称已使用持久队列。

各下载 job（含发布 Ubuntu latest）均保存 `upstream-downloads-<job>-<index>` artifact，内含完整清单、批次请求、manifest id、已核验分页、固定错误码。summary 区分未接收、pending、ready 和失败；只有清单 ready、ready=total 且所有分页身份和单项状态核对完成才算整批入库。附件只是证据，服务端不会扫描附件。job 结束仍 pending 的任务由服务端继续处理，之后验收须重新查在线状态，不能将旧附件或中间 inventory 缺项当最终结论。

工作流不调用私网 DELETE，不同步 GitHub 淘汰结果，也不修改或上传 README.txt。服务端协作与保留范围见[协议](cache-server.md)和[可保留白名单](cache-retention.md)。

维护者于 2026-11-08 授权以独立 `vm/prepared` 复用 Ubuntu 20.04 ARM64 及 x64 预装测试环境；镜像只存私网，禁止进入 GitHub Actions Cache、artifact 或 Release。客户端与发布入口按 health 的 `vm_limits` 启用明确声明的架构；双架构真实模板上传、暖恢复及安装后 GUI/代理/TUN 已由 Actions 验证，证据和单次测速边界见[预装镜像方案](prepared-vm-images.md#首次双架构暖恢复证据)。未声明、限制不符或恢复失败时冷回退；不复用原包接口，不跳过应用测试。16 GiB 和两个环境名额由两种架构共享。
