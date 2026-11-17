# 本地权限与更新信任边界

本页说明实现与验证范围。静态风险不等于已复现漏洞；订阅约束见[订阅说明](subscriptions.md)，安装验收见[候选包验证](workflows/install-validation.md)。

## 运行配置与特权会话

GUI、CLI 和 Android 共用运行配置清洗，原始订阅不变。Windows SYSTEM 与 macOS root 会话另复核本机监听和控制密钥。macOS 助手协议为 10；旧助手只能恢复代理/DNS，新操作沿用系统授权更新助手。

macOS 内核程序和运行数据分别固定在 root 私有临时目录。数据按已打开的目录描述符读取，拒绝符号链接、硬链接文件、可执行文件、其他属主、其他用户可写数据及越界 provider 路径，限制大小、数量和深度。本目录绝对 provider 路径转换为相对路径，运行与重载沿用同一转换。退出后通过目录描述符原子回写节点选择和 provider 缓存，不回写顶层配置、Geo 数据及宿主事务，不跟随目标链接；失败也回收私有快照。缓存回写的保留名称 `.cor-cache-<UUID>` 暂存文件不作为内核输入，强制中断留下的未完成传输不会阻止下一次会话；普通相似名称的 provider 文件照常保留。

Mihomo 路径限制仍是边界的一部分。特权启动清空继承环境，不设置 `SAFE_PATHS` 或 `SKIP_SAFE_PATH_CHECK`，固定工作与临时目录；上游语义见 [v1.19.34 的 `constant/path.go`](https://github.com/MetaCubeX/mihomo/blob/v1.19.34/constant/path.go)。这不是所有内核字段的允许列表，也不隔离已取得控制密钥的同用户进程或内核自身漏洞。

### Linux 原生调用

内核启动器借用存活文件句柄查询扩展属性，以 C 布局的 [capset V3 头和两个数据项](https://man7.org/linux/man-pages/man2/capget.2.html)提交 capabilities；版本头提供可写指针以接纳内核在版本不匹配时的回写，数据项只读，内存只在同步调用内借用。启动器及终端认证代理的 `prctl` 可变参数显式使用 `libc::c_ulong`，未使用参数为零，遵循 [Linux 手册的参数宽度约束](https://man7.org/linux/man-pages/man2/prctl.2.html#CAVEATS)，不依赖默认整数宽度。

终端认证代理的 `pre_exec` 只执行不分配、不持锁的进程设置和身份查询，并保留父 PID 复核以拒绝设置期间的重新托管；[父死亡信号](https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html)的触发对象是创建子进程的父线程，多线程程序中该线程结束时，其他线程可能仍在运行。认证子进程由一个 `Child` 持有；退出时若它尚未被回收，先发 SIGTERM，再有界等待、强制终止并回收。源码的 SAFETY 说明记录具体指针、布局和生命周期前提，不代表全仓 FFI 已完成安全审计或已复现漏洞；真实授权与网络验证仍须使用可丢弃环境。

### Windows 服务通信与账户查询

管道创建使用的安全描述符和 `SECURITY_ATTRIBUTES` 保持存活直到同步调用返回，句柄随后只交给一个 `OwnedHandle`。管道没有启用 overlapped I/O，读写缓冲区和结果长度只在调用期间借用，写入长度受 64 KiB 消息上限约束；非等待模式不改变同步缓冲区生命周期。客户端保留管道名称存储，在 `CreateFileW` 失败后立即保存 [GetLastError](https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-getlasterror)，再判断是否重试，避免错误格式化或临时存储释放影响线程错误码。

系统目录查询从调用前就用单一所有者托管返回指针，遵循 [SHGetKnownFolderPath](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath) 在成功和失败时都需释放输出的契约；空指针或失败状态不会进入 UTF-16 读取。账户查询先检查 [GetTokenInformation](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-gettokeninformation) 的大小探测错误和最小结构长度，再分配指针对齐的存储，成功且返回长度在容量内才读取 `TOKEN_USER`；SID 转换期间保留原缓冲区，转换结果使用 `LocalFree` 释放。当前进程伪句柄只借用，进程/令牌/快照等新句柄才交给 `OwnedHandle`。

无窗口后台只继承显式复制的输入、输出和错误句柄；属性列表按查询大小分配对齐存储，[继承白名单的数组](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)及其中的句柄一直存活到列表销毁，失败返回也按逆序析构。普通身份重启客户端时，令牌、UTF-16 命令行、桌面名称、启动结构及双 NUL 环境在进程创建的同步调用期间保持存活；[用户环境块](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createenvironmentblock)先复制后释放，不继承安装程序的用户环境。这里记录服务通信、账户查询及进程创建的局部不变量；SCM 和其他 FFI 仍按各自调用约束逐步复核，不代表全仓审计完成。

### macOS 系统属性与偏好会话

CoreFoundation 桥接按 [Create/Copy 与 Get 所有权规则](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFMemoryMgmt/Concepts/Ownership.html)区分释放责任：`Owned` 只接管非空的创建/复制结果，Get 返回的字典、数组及字符串在父对象仍存活且未修改时复制成 Rust 数据。转换入口拒绝空值和类型不符，数组/字典的计数与递归深度有界；字符串长度先确认可由 `CFIndex` 表达，UTF-8 缓冲区容量另行检查。转换结果还按 UTF-16 单元数核对原始长度，遇到嵌入式空字符时停止修改，避免 C 字符串截断后静默写回；缺失对象不进入类型查询。

系统偏好由单个本地 `SCPreferences` 会话读写，路径字符串及属性列表在同步调用期间保持存活。锁失败时立即返回，读改写结束后的普通错误出口也执行解锁；若 Rust 在此前展开，`Owned` 仍释放会话。Apple 的[会话析构实现](https://github.com/apple-oss-distributions/configd/blob/main/SystemConfiguration.fproj/SCPOpen.c)清理锁状态、锁文件描述符及 helper 会话，因此不能仅因没有额外解锁 guard 就认定泄漏。现有界面和授权语义保持原有入口；局部原生转换测试不等于真实网络配置的故障注入。

这两个桥接模块局部启用 `clippy::undocumented_unsafe_blocks`，由现有原生 CI 的严格 Clippy 检查覆盖；没有扩大为全工作区规则，注释存在也不能代替指针、所有权和失败清理的实际复核。

## 安装包固定与校验

助手在内存中固定本次计划的 SHA256，校验失败即停止；安装程序读取时采用以下机制：

| 平台 | 机制 | 边界 |
| --- | --- | --- |
| Windows | 持有仅允许共享读的包句柄，并锁定路径上的目录，持续到安装结束 | 阻止校验后的写入、删除与目录替换，不代替发布者认证 |
| Linux GUI/终端 | 有界复制到 `memfd`，封印写入、增长、截断及后续 seal 变更，再校验；`dpkg` 读取存活助手的 `/proc/<pid>/fd/<fd>` | 原下载文件的替换不改变安装字节；临时内存与包大小相当，沿用 512 MiB 上限，退出释放 |
| macOS 普通安装 | 只读挂载并校验应用、内核和资源，替换前校验暂存应用 | 此路径不增加权限 |
| macOS 管理员安装 | 提权后在 root 私有目录复制 DMG，按固定 SHA256 校验副本，只读挂载并校验暂存应用后交换 | 不再从用户可修改的挂载目录复制特权安装内容；失败保留旧应用 |

这些机制处理固定校验值之后的替换窗口。发布渠道先被替换、助手启动前同用户篡改整个计划或已控制客户端进程，不在其身份保证内。SHA256、GitHub asset digest 和 ad-hoc 签名都不构成独立发布者认证。

## 多用户网络授权

当前授权包含整机能力和用户会话两层：

| 平台 | 整机能力 | 会话约束 |
| --- | --- | --- |
| Windows | 管理员安装受保护服务，允许交互式用户连接 | 核对已安装桥接程序、父进程和桥接账户一致；按调用者身份读文件，内核使用 SYSTEM 私有快照 |
| Linux | 管理员为固定安装启动器授予网络 capabilities；没有授权 UID 名单 | 保留调用者 UID，只向固定内核继承网络能力；代理与后台按用户分开，TUN 可影响同一网络命名空间 |
| macOS | 管理员安装网络助手 | 按 UID、私有 socket、对端 UID 与随机 token 绑定会话，不接受任意可执行文件 |

现状不保证“仅首次授权用户可使用”。维护者决定保持既有授权语义，当前不引入 SID/UID 白名单。共享机器若将来要求此限制，需要独立的授权、撤销和管理员恢复设计。完整验收须在可丢弃多用户系统覆盖允许用户、其他用户、切换、撤销和异常退出；现有归属检查不能代替多用户攻击复现，也不能证明三平台隔离等价。

## 独立发布者认证方案

维护者决定保持现有更新校验、平台签名和发行流程，当前不引入独立发布者签名。以下仅为评估备忘，不属于实施待办：

1. 客户端内置离线根公钥，私钥在发布渠道之外保管。根签署带版本、有效期的发布密钥清单；日常密钥只签目标清单。
2. 目标绑定产品、通道、版本、来源提交、平台、架构、文件名、长度与 SHA256，拒绝重复键与歧义编码。先验证签名角色，再校验并固定安装字节。
3. 轮换由仍受信根签署，根轮换同时取得旧根和新根授权；撤销使用单调版本、有限有效期及本地已接受版本，防重放和回滚。离线、时钟异常与主动降级须明确策略，不能静默退回无签名安装。
4. 首个内置根的客户端仍依赖现有分发，旧客户端没有此前不存在的身份保证；过渡版本说明支持范围及强制校验启用点。Android APK 覆盖签名、Windows Authenticode、macOS Developer ID/公证分别验证，互不替代。
5. 实施前由维护者确定根公钥、密钥保管、轮换/撤销责任、过期/降级策略和旧版过渡。测试包含错产品/通道/平台/架构、损坏签名、未知密钥、过期、重放、撤销、取消与正常更新。

参照 [The Update Framework 规范](https://theupdateframework.github.io/specification/latest/)评估成熟实现，避免自行设计不完整信任链。本轮没有创建密钥、取得签名 Secrets、改变包签名或公开发行。
