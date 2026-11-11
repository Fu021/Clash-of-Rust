# 打包与发布

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

第三方 mihomo 声明按实际依赖生成：桌面取已校验的 `core.json`，Android 取本轮 Go 模块快照；声明包含对应源码版本或伪版本的提交。来源无法确定时构建失败，避免依赖经批准更新后保留过期版本。

## Publish Release

公开 Release 或 Pre-release 前须完成[循环审查工作流](../loop-workflow.md)，核对待发布提交的检查范围、遗留项及验证结果。

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的当前提交 CI 编号；留空选择当前提交的 main CI。开发分支测试包必须填写当前分支、当前提交成功的 PR CI 编号。
- `check_only`：默认勾选，核验同提交 CI 已完成安装验证的九组候选包、SHA256 和收据，不重新打包、不创建或修改 Release。安装包保存在所选 CI 的 `package-{system}-{arch}` 附件中，无需下载到本地。
- `prerelease`：经维护者确认后显式发布预发布版，须关闭 `check_only`。可在开发/修复分支上运行，必须指定同仓库、同分支头成功的 PR CI，源码使用合法预发布版本，并提交 `docs/releases/<完整版本>.md`。不能用此开关发布无后缀的正式版本；两种模式同时勾选会在前置校验失败。

正式版和预发布版的文字按[发布文案模板](../releases/TEMPLATE.md)准备并随源码提交。发布程序校验该提交的文案后，在首段之后、更新详情之前放置完整下载区，避免 GitHub Releases 列表截断长正文时切断徽章链接。九种安装包及其计数均固定到本次标签及具体文件，正式版与预发布共用按标签查询的计数入口。下载量由 Shields.io 读取 GitHub 附件计数，可能因缓存延迟更新；计数包含重复下载与自动校验，不代表独立用户数。SHA256 文件仍在 Release 附件中。

从修复分支发布时，选择该分支，填写其成功 CI 编号，设置 `check_only=false`、`prerelease=true`。流程照常验证九平台安装包与来源，最终以预发布标签公开，不更新稳定版 latest、不合并或删除来源分支。上传结束还会重新核对该来源分支头；期间分支前移则保留未公开草稿并失败。未显式选择预发布时，公开发布仍只接受 main 的 push CI。预发布与正式发布使用相同的发行签名要求，Android 不能用临时 CI 测试签名冒充发行签名。

维护者委派给 Agent 的任务按[收尾规则](../../AGENTS.md#actions提交与发布)持续跟进：最终推送后等待同提交 CI 和两套独立兼容验证成功，再以该 `ci_run_id` 和 `check_only=true` 核验候选包。经授权的公开发布完成同提交验证后，不追加重复核验。失败先定位最早失败步骤，修复后验证新提交，不以已触发、排队或运行中收尾。最终核对平台安装、兼容回执、九组包、SHA256 和安装收据；不发布模式还须确认 Release 未变，外部阻塞明确说明。

复盘和其他文档随功能改动纳入最终提交，再启动验证；完成后的运行编号、来源提交和附件结果写入最终汇报，不为补写成功记录再次推送。后续纯文档补充按[收尾规则](../../AGENTS.md#actions提交与发布)保留本地；确需修复代码时，以新提交重新完成 CI 与不发布打包。

流程固定到所选 CI 的提交与依赖快照，核对六种桌面平台、三种 Android ABI、Ubuntu latest 和两套独立兼容性结果，不重新编译或下载应用资源。候选包、源码产物及回执保留 3 天；过期或旧格式证据需要重新运行当前代码。完整测试职责见[安装验收](install-validation.md)。

Ubuntu 20.04 ARM64 和 x64 的模板准备、恢复及安装验证归独立 [Ubuntu Compatibility](ubuntu-compatibility.md)，按线上 `vm_limits` 明确声明的架构启用[私网预装环境镜像](prepared-vm-images.md)，不满足条件时冷回退。SSH 短时超时在总预算内重试，不由缓存命中代替就绪检查。Publish Release 只核对兼容结果，不重启 VM；预装镜像、基础盘及写入层均不进入 GitHub Cache、artifact 或 Release。

从启动到最终核验完成，来源分支必须仍指向该 CI 的提交；`publish-release.py` 在前置解析和最终核验时都会检查分支头。即使只是文档改动，也应等本轮打包结束后再推送，避免 `Source branch has changed`。测试包以本次工作流固定的提交为准，后续提交需另做验证。

1. 前置解析要求同提交 CI 及两套兼容性检查成功，九组候选包和九组构建附件唯一、未过期，冻结来源与依赖快照。
2. 不发布模式直接校验 CI 内已通过安装测试的九组候选包，不申请平台或虚拟机 runner。
3. 公开模式复用六组桌面候选包；三个 Android job 取得独立发行签名凭据，重新签名 CI 的实际 APK 与 instrumentation，核对签名、架构、16 KiB 对齐，并在 x64 最终签名包执行基础代理/TUN 验证。开发签名候选包不能作为发行签名证据；不运行生命周期、键盘或截图专项。
4. 最终 job 核对九组包、外层 artifact digest、安装收据、来源构建身份及包摘要，输出九份安装包和对应 SHA256。公开模式才创建 Release；开发版本标记为 prerelease，不覆盖稳定版 latest。

Ubuntu 18.04 未纳入此基线或兼容矩阵，不能从 20.04 的验证结果推断其受支持。

打包与安装验证所需原包遵循[工作流缓存分工](cache-coverage.md)，不向私网保存安装目录或构建对象。原包准备后尽早交持久清单，下载 job 在末尾保存清单和分页回执附件；pending 与整批 ready 分别报告。Ubuntu 环境准备与安装矩阵见[独立兼容验证](ubuntu-compatibility.md)与[安装验收](install-validation.md)。

只有最终发布 job 有 `contents: write`。桌面 CI 向 `scripts/build-installer.py --prebuilt` 传入已验证的本地构建产物；本地打包沿用相同入口。打包器只校验与组装，不隐式调用 Cargo。

平台检查集中在 `scripts/windows_ci.py`、`scripts/macos_ci.py`、`scripts/linux_gui.py` 和 Linux 隔离网络夹具中；`scripts/test-release-package.py` 统一调度，`scripts/linux_ci.py` 负责基线容器和虚拟机。各 GUI 结果与失败日志保存在对应 `*-package-report-*` 附件中。

### 候选包验收前移

安装包生成、平台安装后检查和 Ubuntu latest 在 CI 内执行，Ubuntu 20.04 和多版本 Android 则在独立兼容性工作流中复用候选包。CI 成功不等于两套兼容验证已通过。平台范围、环境隔离、权限和收据约定见[安装验收](install-validation.md)；新接线尚未取得同提交 Actions 结果时不能写成验收通过。未来桌面引入发行签名时仍须验证实际字节。

此调整会增加 CI 时长和候选包存储，目标是更早发现安装问题并减少 CI→发布失败→修改→再 CI 的重复，不承诺单次 CI 更快。发布阶段的网络或 runner 故障且没有代码改动时，可以复用同提交成功 CI，仅重试失败发布 job；实际代码或构建输入变化仍需新提交验证。

### 兼容性发布门禁

普通 CI 成功后，以同一 `ci_run_id` 分别运行 [Android Compatibility](android-compatibility.md) 和 [Ubuntu Compatibility](ubuntu-compatibility.md)，两者可并行。Android 8.0～16 共 11 版本与 Ubuntu 20.04 两架构的基础系统代理/TUN 均成功，且各自最终回执 job 成功后，才可运行 Publish Release。截图和诊断专项可选，不代替基础网络检查。

`scripts/compatibility.py` 自动查找同提交最新的兼容运行，并核对 workflow 路径、同仓库、成功终态、所有必需 job、attempt 及回执 ZIP 的 digest。回执必须精确绑定所选 CI 的输入 artifact id/name/digest；只检查分支名、工作流绿色、旧运行成功或过期附件均不够。最新兼容运行尚未完成或失败时阻止发布，不静默退回旧结果。前置解析和最终核验都执行门禁，`check_only`、正式版与预发布一视同仁。

两套验证只增加发布前验收，不随普通 push 重跑旧系统。保持来源分支不变，按顺序完成 CI → 两套兼容验证 → 不发布核验或已授权公开发布；期间新 push 需重新验证新提交。弃用的 Android UI Previews 文件已移除，维护者还需在 GitHub 禁用其旧工作流记录。

### Windows Defender 告警

历史扫描通过不代表后续检测库或云端检测仍给出相同结果。出现告警时先记录版本、下载来源、检测名及被拦截文件的 SHA256，核对安装包的公开 SHA256；安装包和安装后主程序是不同文件，不能直接比较两者哈希。Windows 打包收据中的 `defender.files` 同时记录安装包和未压缩程序，可按文件名核对主程序。

确认来源后，可通过 [Microsoft Security Intelligence 官方提交入口](https://www.microsoft.com/en-us/wdsi/filesubmission)选择软件开发者和误报类别，提交具体报毒文件，并附检测名、版本、哈希及公开来源。截图或 `!ml` 后缀本身不足以证明误报；提交前也不以修改二进制、关闭防护或设置排除项代替核验。主程序已有图标和版本资源，保留符号、Authenticode 签名与检测结果之间没有本项目已验证的因果结论。

## 发布后说明与助手清理

Actions 已覆盖基础安装、系统代理和 TUN；同提交的打包结果、安装验证与收据是发布验收基线。发布完成后不再要求维护者在本机重复安装包检查，也不为相同范围再次运行打包。真实设备首次授权与取消、注销登录和重启、休眠、多显示器、系统通知仍受硬件和人工交互限制；这些项目不冒充已验证，出现反馈时按具体 issue 复现，不能仅凭 Actions 通过就宣称所有设备均已完成验收。

更新路径中的下载取消与重试、校验失败不安装、安装失败恢复旧版及升级后保留订阅和个人设置，只有在同提交自动化覆盖时才按结果声明；未覆盖的设备组合不在每次发布后重复全量手工验收，按具体 issue 跟踪。

Windows 更新需要完成 UAC；Linux GUI 使用 `pkexec` 与桌面授权服务，纯终端更新使用前台 `sudo` 授权并等待安装结果，授权拒绝不停止后台，安装失败尝试恢复原运行状态。DEB 推荐安装 `pkexec` 或旧系统中的 `policykit-1`。macOS 数据保存在用户 Application Support，系统代理与 TUN 共用 launchd 网络助手，GUI 以普通用户运行；已有助手可供后台自启复用，退出恢复不应重新索取授权。首次下载后的 Gatekeeper 提示按 [Apple 说明](https://support.apple.com/zh-cn/102445)处理。

Windows 安装完成页调用原生普通用户启动入口，优先使用当前桌面身份，无桌面时只接受当前账户的非提权关联身份；不能取得时不启动管理员 GUI。下载、安装助手和用户设置使用该用户环境，只有安装程序与网络服务取得系统权限；更新后显式保留原数据目录。普通用户窗口、可写下载/配置及写入失败回滚归扩展安装测试。Linux 的 DEB 写入由 `dpkg`/`pkexec` 完成，macOS 按应用目录权限选择原子替换或授权，Android 使用私有缓存及只读 URI 授权交给系统安装器；不能把它们概括为需要给应用目录开放普通用户写权限。

macOS 卸载前关闭代理并退出客户端，再按需移除当前用户的网络助手：

```sh
service_label="org.clashofrust.network.$(id -u)"
sudo launchctl bootout "system/$service_label"
sudo rm -rf "/Library/PrivilegedHelperTools/$service_label" "/Library/LaunchDaemons/$service_label.plist"
```

助手停止前会尝试恢复自己仍持有的代理设置并终止内核会话。测试夹具仅移除本次创建的助手，拒绝覆盖已有安装。
