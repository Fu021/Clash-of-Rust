# 三条工作流的缓存分工

[返回 CI](ci.md)。缓存用于缩短关键路径；实际恢复、编译和收尾耗时分别记录，绿色 job 不等于私网已完整保存。

独立于现行保留预算的全平台编译对象容量规划见[容量估算](cache-capacity.md)；估算不会自动启用新缓存或扩大预算。

## 编译对象只用 GitHub

CI 与 Memory Benchmark 的 Rust 对象只恢复和保存 GitHub Actions Cache；未命中正常编译，不查询私网 Rust 索引或分片，也不在 job 结束后镜像对象。Publish Release 继续复用同提交成功 CI 的构建附件，不重新编译 Rust。

仓库缓存上限为 **24 GB**（API 字段 `max_cache_size_gb=24`），工作流热缓存保留预算为 **20 GiB**，为保存期间的新旧对象交替留余量。维护者本轮授权的每月缓存预算为 **1 美元**；账户预算、仓库上限和工作流保留预算分别设置。GitHub 自身的过期和容量淘汰仍可能先发生，工作流优先级只能约束本仓库收尾清理。容量及设置依据见[容量规划](cache-capacity.md#当前上限与保留预算)。

CI 的 main 构建保存各平台完整 Rust 基线，同仓库开发分支与 PR 在恢复兼容的 main 基线后只尝试保存有大小上限的对象差异。Memory Benchmark 仅在 main 工作流测量当前 main 提交时保存完整基线，其他分支测量自身当前提交时使用差异包；选择其他版本时不保存编译对象。fork PR 仍只读，不增加缓存保存或私网权限。Publish Release 复用 CI 候选包，不重新编译或另建 Rust 缓存。以下是此前六次成功 CI 的耗时依据；当前 main 缓存均受保护，不能据此宣称某个 main 对象一定会被淘汰：

| 保留优先级 | 平台或工具 | 恢复／编译耗时依据 |
| --- | --- | --- |
| 1 | macOS Tailscale 引导工具（两架构） | 共约 63 MiB；ARM64 冷构建一次 166 秒，命中后接入中位数 5.5／7 秒 |
| 2 | macOS ARM64 Rust | 恢复 20 秒、编译 213.5 秒；维护者指定最高 Rust 优先级 |
| 3 | macOS Intel Rust | 恢复 138 秒、编译 479.5 秒 |
| 4 | Android ARM64 Rust | 恢复 13 秒、应用及内核编译 446.5 秒 |
| 5 | Android ARMv7 Rust | 恢复 8 秒、应用及内核编译 432 秒 |
| 6 | Android x64 Rust | 恢复 14 秒、应用及内核编译 420 秒 |
| 7 | Windows ARM64 Rust | 恢复 150.5 秒、编译 268 秒 |
| 8 | Linux x64 Rust | 恢复 4.5 秒、编译 296.5 秒 |
| 9 | Windows x64 Rust | 恢复 43.5 秒、编译 239.5 秒 |
| 10 | Linux ARM64 Rust | 恢复 8.5 秒、编译 221.5 秒 |
| 11 | Memory Benchmark | 最先让出空间 |

2026-11-10 按六轮完整成功 CI 核对，恢复与编译分别取中位数，策略不随每轮数据自动变化。采样来源、Tailscale 冷暖对照及测量边界见[耗时依据](cache-capacity.md#近期-ci-耗时依据)。

清理脚本不删除 main 的任何条目。其他 ref 的完整编译归档退出保留，每个 ref、平台及 profile 最多保留最新一代差异包；与 main 的 key 和 cache version 均相同的分支副本也会删除。CI 的分支任务与 Memory 清理任务从 main 加载保留策略，避免旧分支的脚本删除 main 基线；旧分支仍需同步工作流本身以采用新规则。超出 20 GiB 工作预算时先回收其他 ref；若 main 本身已超预算，只报告警告，不删除 main。GitHub 自身的过期与仓库容量淘汰不受此保护，20 GiB 因而是工作预算而非绝对上限。`rust-cache` 的 Cargo registry/Git 数据与 main 对象一起管理；测试报告和安装包仅使用短期 artifact。

这是保留规则，不会改变 GitHub 的[缓存读取作用域](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache)：PR 的 merge ref 缓存仍不能被 main 或其他 PR 读取，切换运行分支可能冷编译。不能通过同名 key 将 PR 缓存迁移到主分支，也不扩大不可信 PR 的写权限。

共享是单向的：开发分支及 PR 可以恢复默认分支 main 的兼容缓存，main 不能反向读取 PR 缓存。[差异包](cache-capacity.md#main-基线与分支差异包)由独立的恢复步骤校验、叠加，GitHub 不会自动合并两份归档；缺少基线、损坏或超过上限时正常重编。新命名空间只由可信 main 写入完整基线，首次 main 运行可从旧 main 归档迁移；不通过切换触发事件或提升 PR 权限绕过作用域。

Linux 和 Android 保留 workspace 编译结果及经过内容核对的源码时间戳，不再精确命中旧依赖缓存后永久跳过新对象保存；Linux 容器同时挂载源码回执，避免宿主和容器读取不同位置。Windows ARM64 保留依赖缓存，只在本地 vendor 源码变化时更新键，不因每次业务源码变动重压整个 workspace；其他桌面端继续保留 workspace。全部使用 zstd 快速压缩级别 1，缓存仍不可变，首次填充和后续暖恢复需分别测量。

缓存 Action 默认会删除 workspace 根目录内、却不属于成员的 path 依赖，两个本地 Slint 修复库因此曾每轮重编并连带重建 UI。现在按包名把这两库的编译对象、构建输出及指纹保留在现有源码回执目录内；恢复先核对完整文件清单及摘要，缺失、损坏或越界时整体忽略，源码改变后仍失效相应 crate。不会把第三方库加入项目测试成员或关闭其告警；迁移首次仍可恢复旧归档中的兼容依赖。

Android `GOCACHE` 按工具版本、Go 源码及锁文件分键；Linux 编译容器按官方基础 digest、Python 摘要、Dockerfile 和构建 UID/GID 分键。这两类完整对象只由 main 保存，分支未命中时正常生成而不保存完整副本。容器归档只含编译环境，不含应用、测试写入层或 VM；恢复先校验归档长度、SHA256 和环境身份，再核对 Docker 加载结果，失效则冷准备。Gradle 转换、工具安装目录及资源目录仍不缓存。

macOS 的官方 Tailscale Action 会源码编译 CLI/daemon，因此 CI 的两个 macOS job 开启该 Action 自带的 GitHub 二进制缓存；内存基准、发布签名与 Android 截图均在 Linux 上运行并关闭此工具缓存。此引导缓存按架构保留最近版本，计入 20 GiB 总预算，不上传私网。预算任务在运行取消时跳过，只操作 GitHub API。

| 工作流 | GitHub 编译缓存与收尾 | 私网预装镜像 |
| --- | --- | --- |
| CI | 九个目标的 main Rust 基线及受限分支差异，加 main 的 Linux 编译容器与 Android Go；优先保护 main | Ubuntu 20.04 x64、ARM64 只走 `vm/prepared` |
| Memory Benchmark | main 当前提交保存 profile 基线，其他分支保存受限差异；使用同一预算 | 不使用 |
| Publish Release | 复用同提交 CI 候选包，不重复编译或另存对象 | 复用 CI 验收结果，不下载或上传 VM |
| Android UI Previews（按需） | 复用成功 CI 的 APK，只留截图 artifact | 不使用 |

## 原始下载：官方优先，私网回退

共享下载器先复用本 job 已校验文件，再尝试官方外网；有界重试仍失败时，才通过 `upstream/files` 精确查找原始文件。外层归档与原文件分别校验 SHA256，单个普通文件原子替换目标。未命中或校验失败保留已有文件，交原安装器报错或正常重建，不使用旧目录快照冒充原包。

| 路径 | 当前接入方式 |
| --- | --- |
| 宿主下载的桌面 mihomo 原始 ZIP/GZ、Geo DAT/MMDB | `build_support.download` 下载及回退；解压、资源准备在 runner 完成 |
| Linux 构建容器内的 mihomo/Geo | 先在宿主用同一冻结快照下载及校验，必要时私网回退，再挂载已准备资源；容器不接收私网凭据。容器后续仍复核资源身份 |
| Android Geo 原始文件 | 同一共享下载器；不镜像 `offline-assets` 目录 |
| NSIS 原始安装程序 | 同一共享下载器；不镜像安装后的 NSIS 目录 |
| Gradle 分发 ZIP | CI 三个 Android ABI job 使用 `build_inputs.py prepare-gradle`，按快照版本和已提交官方 SHA256 走共享下载及私网回退；解压到 runner 临时目录，不保存安装目录或转换缓存。发布只重签 APK，内存基准不使用 Gradle |
| Ubuntu 20.04 独立 Python 与原始云镜像 | `linux_ci.download` 共用上述下载器；编译容器另走 GitHub，预装 VM 仅走私网，不保存运行状态 |
| Go 模块原始 ZIP/MOD | `go mod verify` 后登记公开 Go proxy URL；仍由原生 Go 工具下载、校验并在本轮 artifact 内分发 |
| 宿主 APT 原始 DEB | 当轮签名索引确认 URL、大小及 SHA256；APT 先正常下载，失败后整批最多用 120 秒尝试私网，再交 APT 复核 |
| 容器和 guest 的 APT | 沿用签名索引校验；不向其传入私网凭据，不能据此声称已接入私网恢复 |
| Python/Tailscale 引导、Rust/Go/JDK、Android SDK/NDK/模拟器、Maven | 保留官方 setup/包管理器安装；停止目录快照。未通过共享下载器或已验证原包登记的路径不计入私网覆盖 |

最后一行是当前实现边界，不能用“工具有官方下载链接”推断客户端已能从私网安装。需要新增原包适配时，先补明确来源、强校验、安装器入口及失败回退回归；不恢复目录快照或 runner 归档上传。Python/Tailscale 在私网登录前的引导还依赖 GitHub/runner，现有方案不是离线 runner。

构建收据按任务记录实际使用的工具：桌面只查询 Python、Rust 与 Cargo；Android 另查询 Go、Java、Gradle 与 NDK。桌面不调用 runner 预装的 Chocolatey Gradle 或其他移动工具；Gradle 私网回退只解决原包下载，不解决无关工具启动或版本探测超时。

## 尽早提交完整清单，收尾保留回执

`upstream-cache` 在 runner 临时目录保存认证配置；三条工作流的 `upstream-report` 均使用[持久清单协议](cache-server.md#持久清单补拉)。共享 Go、原生资源、Linux 原包、VM 和内存基准宿主依赖准备完成后设置检查点，尽早提交当前全部已校验文件；收尾追加后续下载并查询已接收清单。只交官方 URL、大小、SHA256 和运行序号，没有文件内容、runner 绝对路径或凭据。记录不依赖临时原包继续存在，相同内容按 SHA256 合并。

检查点预算 20 秒、不轮询；末尾总预算 60 秒，其中最多 15 秒用于一次分页查询，各元数据请求最多 8 秒，外层子进程分别限 30/90 秒。先保存全部批次，再提交全部批次，最后查询；不会等待单 worker 下载完全部原包。持续轮询不重置失败任务，断线重发固定请求与序号。预算不足、拒绝或能力缺失保留未提交项，绝不改为 `PUT` 归档。只有旧能力时明确使用兼容单项导入，不宣称已使用持久队列。

各下载 job（CI 的输入、Android、桌面及 Ubuntu 兼容任务，公开发布的 Android 签名任务，以及内存基准任务）均保存 `upstream-downloads-<job>-<index>` artifact，内含完整清单、批次请求、manifest id、已核验分页、固定错误码。summary 区分未接收、pending、ready 和失败；只有清单 ready、ready=total 且所有分页身份和单项状态核对完成才算整批入库。附件只是证据，服务端不会扫描附件。job 结束仍 pending 的任务由服务端继续处理，之后验收须重新查在线状态，不能将旧附件或中间 inventory 缺项当最终结论。

工作流不调用私网 DELETE，不同步 GitHub 淘汰结果，也不修改或上传 README.txt。服务端协作与保留范围见[协议](cache-server.md)和[可保留白名单](cache-retention.md)。

维护者于 2026-11-08 授权以独立 `vm/prepared` 复用 Ubuntu 20.04 ARM64 及 x64 预装测试环境；镜像只存私网，禁止进入 GitHub Actions Cache、artifact 或 Release。客户端与发布入口按 health 的 `vm_limits` 启用明确声明的架构；双架构真实模板上传、暖恢复及安装后 GUI/代理/TUN 已由 Actions 验证，证据和单次测速边界见[预装镜像方案](prepared-vm-images.md#首次双架构暖恢复证据)。未声明、限制不符或恢复失败时冷回退；不复用原包接口，不跳过应用测试。16 GiB 和两个环境名额由两种架构共享。
