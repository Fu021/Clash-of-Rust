# 三条工作流的缓存覆盖

[返回 CI](ci.md)。本页记录客户端接入范围；成功执行、完整保存和实际恢复须分别以同提交 Actions 日志及私网清单验收，步骤存在或绿色 job 不代表全部缓存已写入。

所有类别先恢复 GitHub，未命中再取私网，均不可用时走官方安装或编译。GitHub 命中仍检查私网副本；正常归档继续使用安全提交能力门禁，Rust 额外检查索引引用。工作流不调用私网 DELETE、不按 GitHub 淘汰结果通知服务端删除，也不对私网目录执行清理。服务端自己的清理策略必须保护仍发布的索引与在途事务。

| 内容 | 使用工作流 | 私网类别 |
| --- | --- | --- |
| Cargo registry 下载、索引与 Git 数据库 | CI、Memory 的 Rust 缓存 | `rust/*`；旧 `dependencies/cargo-source` 仅保留历史 |
| 已确认依赖快照 | CI，打包复用同运行附件 | `dependencies/build-inputs` |
| Rust 原生、Android、内存构建对象 | CI、Memory | `rust/<目标>`，32 分片及最终索引 |
| Go 模块下载、gomobile/gobind、按 ABI 编译对象 | CI | `tools/android-go`、`compiler/android-go-<arch>` |
| Gradle 下载、转换及分发包 | CI | `dependencies/gradle-linux` |
| ARMv7 Skia 构建脚本下载的源码 | CI | `dependencies/skia-armv7` |
| mihomo、Geo、各端预置资源 | CI、Memory；打包复用已验证附件 | `resources/<目标>`、`resources/android`、`resources/memory-linux-x64` |
| Python 与 Tailscale 的安装目录/安装包 | CI、打包、Memory | `tools/runtime-<系统>-<架构>` 的 `bootstrap` key |
| Rust 工具链与目标标准库 | CI、Memory | 同上，`rust` key 按版本、宿主和交叉目标区分 |
| Go 工具链、Temurin JDK | CI，Android 打包使用 JDK | 同上，分别为 `go`、`java` key |
| Android 命令行工具 23、platform-tools 37.0.1、API 37.2、build-tools 37、NDK 30 | CI、Android 打包 | 同上，`android` key |
| 模拟器 37.2.13 及 API 36 r7 Google APIs x86_64 系统镜像 | CI 与 Android 打包的 x64 | 同上，`emulator` key；审计检查 API 与 revision，启动前核对实际安装 revision，AVD 和用户状态不入缓存 |
| NSIS 安装编译器 | Windows 打包 | `tools/nsis` |
| Ubuntu 20.04 独立 Python、原始云镜像 | Linux CI、打包及旧内核验证 | `tools/linux-python-<arch>`、`tools/linux-cloud-<arch>` |
| Ubuntu 基础层、构建工具及库组成的已构建镜像 | Linux CI 与打包 | `dependencies/linux-image-<arch>` |
| 宿主 QEMU、兼容验证、Memory 的 APT 下载 | Linux 打包与 Memory；Linux CI 的补充依赖 | `dependencies/host-apt-linux-<arch>` |
| guest 的 APT 下载 | Ubuntu 20.04 安装验证 | `dependencies/linux-apt-<arch>` |

`runtime-cache` 只选白名单工具目录，排除 home、密钥、签名材料、SDK 用户目录、AVD、Rustup 配置与凭据。每个目录单独保存为含内部链接的 tar，外层继续使用原有普通归档协议，避免丢失 NDK/工具链链接。恢复先校验清单与摘要，再用安全过滤器解到暂存目录；链接不得逃出 payload，完成后才发布。已有工具目录交官方 setup action 校验和补齐；快照不能覆盖正在使用的解释器。工具缺失、外部链接或权限问题在日志中记录，不能据此宣称镜像完整。

Temurin JDK 单独保存同一已确认版本的官方原始归档及固定 SHA256，避免跟随 runner 预装目录指向系统文件的链接。恢复验证后在工具缓存中暂存解包并发布标准架构目录及完成标记，再由既有 setup-java 校验使用；也可通过下述上游代拉协议补镜像。当前只有实际使用的 Linux x64 JDK 身份，新增宿主须先补对应官方归档。

macOS Python 使用官方实际安装的对应次版本 Framework 目录作为白名单根，避免从工具缓存中的别名跨到根目录外；镜像不覆盖运行中的解释器，首次安装和系统 Framework 注册仍由官方 setup-python 负责。

Python 3.15.2 已核对各 OS/架构的官方下载清单；固定版本须具有目标平台二进制，不能将仅 Linux 提供的补丁号直接用于 Windows/macOS。版本引导只使用旧 runner Python 可用的标准库；加载已确认解释器后才导入 `tomllib` 执行实际构建逻辑。升级审计检查跨次版本候选，并核对目标 OS/架构的下载清单。

输入任务、Memory、平台矩阵与打包统一使用 Tailscale `1.104.5`；工具快照按实际版本计算身份。

工具快照在 post 阶段先捕获，再保存私网及 GitHub；同一目录未变化时复用已有 tar。容器按固定基础 digest、Python SHA256、Dockerfile、架构和 UID/GID 生成 key，加载时核对归档及镜像 ID；首次构建后导出一次供后续 Linux CI/打包复用。宿主补充 DEB 通过显式挂载进入容器，依然验证当轮签名索引。

服务端提前拒绝上传时，客户端对断管、连接重置和 Windows 连接中止均先尝试读取已返回的 HTTP 状态：序号冲突保留较新副本，容量或其他拒绝如实记为未保存；不能把发送中断或绿色 job 当作私网已完整覆盖。连接无法取得应答时保留原网络错误并关闭连接。HTTP 502/507 或仍无应答的中断须按真实日志交由服务端排查，客户端覆盖这些异常类型不代表服务已恢复。

Android 打包仅恢复命令行工具、platform-tools 和 build-tools，不传输打包不使用的 NDK/API。版本和系统镜像由工具基线的 `upstream.android` 统一声明并参与快照身份，CI 和打包共用 `prepare_sdk`。固定命令行工具 23 中的原生 `android sdk install` 负责安装，platform-tools、模拟器和系统镜像用 `@revision` 精确选版；`--force` 仅在可丢弃 runner 内落实冻结版本，不运行自更新。缺少原生 CLI 的旧 runner 只在引导工具包时使用旧入口。SDK/NDK 及模拟器下载共用 `install_sdk_packages`，只对明确的损坏归档或网络错误最多尝试三次，每次换 SDK 用户缓存；配置、许可和编译错误直接失败。

普通输入任务不运行 Cargo 解析，也不安装 Rust 或下载单独的 Cargo registry 快照；显式依赖刷新运行安装已确认 Rust 后解析更新并导出锁文件。实际编译任务仍完整镜像 Rust 下载与对象。此调整只移除重复入口，不删除私网历史类别。首次工具/容器冷缓存会增加捕获与传输成本，耗时收益须按相同环境、冷暖状态及验证范围对照后续运行。

缓存不能接管 GitHub 托管 runner 镜像、其内置 Node/Git/系统工具，或在 checkout 前由 GitHub 下载的 Actions 运行包。Python/Tailscale 首次启动也必须先具备 runner 可用解释器及登录通道，之后才能镜像；私网不可达时无法用它启动自身。当前方案覆盖可复用的项目工具与下载，不构成完全脱离 GitHub/上游的离线 runner。若要求启动环境也能从私网恢复，需要维护者另行提供预置镜像/自托管 runner 或引导镜像服务，不能靠当前缓存 job 实现。

Android SDK 的包标识、声明版本和 runner 镜像版本参与身份；包标识内的上游 revision、APT 签名索引及托管 runner 预装内容并非仓库源码锁文件。工具维护遵循[loop 更新约定](../loop-workflow.md#每轮怎么做)；严格固定这些环境的每一项 revision 需要进一步锁定官方归档或预置镜像，不能把当前结果当作逐字节离线复现。

公开、带固定 SHA256 的原始归档可以使用[服务端代拉协议](cache-server.md#可选的上游代拉下载)。当前先接入 JDK、Linux Python 与云镜像；目录快照、构建对象、Gradle 转换结果和已构建 Docker 镜像没有适用的公开 URL，继续从 runner 上传。即使服务端接口就绪，也不能将排队应答当作已经保存并可恢复。
