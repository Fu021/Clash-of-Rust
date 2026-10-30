# 打包与发布

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

第三方 mihomo 声明按实际依赖生成：桌面取已校验的 `core.json`，Android 取本轮 Go 模块快照；声明包含对应源码版本或伪版本的提交。来源无法确定时构建失败，避免随自动依赖更新保留过期版本。

## Publish Release

公开 Release 或 Pre-release 前须完成[循环审查工作流](../loop-workflow.md)，核对待发布提交的检查范围、遗留项及验证结果。

在 Actions 中选择 **Publish Release → Run workflow**：

- `ci_run_id`：成功的当前提交 CI 编号；留空选择当前提交的 main CI。开发分支测试包必须填写当前分支、当前提交成功的 PR CI 编号。
- `check_only`：勾选后完成打包与安装后验证，不创建或修改 Release；默认验收使用此模式。安装包在本次运行的 `package-{system}-{arch}` 附件中，无需下载到本地。
- `prerelease`：经维护者确认后显式发布预发布版，须关闭 `check_only`。可在开发/修复分支上运行，必须指定同仓库、同分支头成功的 PR CI，源码使用合法预发布版本，并提交 `docs/releases/<完整版本>.md`。不能用此开关发布无后缀的正式版本；两种模式同时勾选会在前置校验失败。

从修复分支发布时，选择该分支，填写其成功 CI 编号，设置 `check_only=false`、`prerelease=true`。流程照常验证九平台安装包与来源，最终以预发布标签公开，不更新稳定版 latest、不合并或删除来源分支。上传结束还会重新核对该来源分支头；期间分支前移则保留未公开草稿并失败。未显式选择预发布时，公开发布仍只接受 main 的 push CI。预发布与正式发布使用相同的发行签名要求，Android 不能用临时 CI 测试签名冒充发行签名。

维护者委派给 Agent 的任务按[收尾规则](../../AGENTS.md#actions提交与发布)持续跟进：最终推送后等待该提交的全部必需 CI job 成功，日常验收以其 `ci_run_id` 和 `check_only=true` 启动本流程。经授权的正式版发布流程完成同提交的打包与安装验证后，不再在发布后追加不发布打包。失败先定位最早失败步骤，修复后跟进对应提交的后续运行，不把已触发、排队或运行中视为完成。打包结束后核对各平台安装后验证、Ubuntu 兼容性、最终核验和九组安装包及 SHA256 附件；不发布验收须确认没有创建或修改 Release，外部阻塞须明确说明。

复盘和其他文档随功能改动纳入最终提交，再启动验证；完成后的运行编号、来源提交和附件结果写入最终汇报，不为补写成功记录再次推送。后续纯文档补充按[收尾规则](../../AGENTS.md#actions提交与发布)保留本地；确需修复代码时，以新提交重新完成 CI 与不发布打包。

流程固定到所选 CI 的提交与依赖快照，核对六种桌面平台及三种 Android ABI 的成功记录和产物身份，不重新编译，也不重新下载应用资源。过期或旧格式产物需要重新运行当前代码的 CI。

从启动到最终核验完成，来源分支必须仍指向该 CI 的提交；`publish-release.py` 在前置解析和最终核验时都会检查分支头。即使只是文档改动，也应等本轮打包结束后再推送，避免 `Source branch has changed`。测试包以本次工作流固定的提交为准，后续提交需另做验证。

1. 六个平台获取并校验已有程序，再生成 Windows EXE、Linux DEB、macOS DMG。
2. Windows 更新 Defender 检测库，逐个扫描安装包及未压缩的全部 EXE/DLL，再验证安装、服务注册、普通用户代理/TUN、更新安装路径、卸载与用户文件保留；安装后以普通用户启动窗口，检查尺寸、GUI/CLI 共享状态和最小化/恢复。PE 图标在打包阶段核验。扫描版本和逐文件 SHA256 写入 `package-receipt.json`。
3. Linux 使用 Ubuntu 20.04 构建的同一份 DEB：基线容器先做安装及 X11 窗口快速检查；Ubuntu 22.04 x64/ARM64、最新版 x64 与 Ubuntu 20.04 x64/ARM64 虚拟机分别验证安装后的 X11/Wayland 窗口、托盘、窗口重建、GUI/CLI 共用内核、图形授权取消/接受和代理/TUN。GUI 测试用户使用独立临时目录，仅保留诊断报告；20.04 虚拟机要求 5.4 GA 内核，实际硬件仍需单独验收。结构、代理/TUN 不在同一环境重复运行。disposable guest 只跳过无关文档索引，GUI 依赖与全部功能检查仍执行，见[测试环境复盘](../agent-failures/test-environments.md#11-测试稳定性与外部环境假设)。
4. macOS 核验 App/DMG 版本、架构、资源及签名，启动安装后的 GUI/内核并验证代理/TUN；界面辅助包使用独立身份与 `LSUIElement`，连续重建界面 5 次，核对应用注册、Dock 最近应用条目及内核 PID。当前使用 ad-hoc 签名，没有 Developer ID 与 Apple 公证。
5. Android 复用三种 ABI 的已编译 APK，检查签名、架构与 16 KiB 对齐；x86_64 最终包再运行代理/TUN 测试。`check_only` 保留固定开发签名，正式发布要求单独配置长期签名。
6. 核对安装包、校验值、成功记录及来源后，正式模式才公开 Release；`check_only` 仅保留 Actions 附件。开发版本标记为 prerelease，不覆盖稳定版 latest。

Ubuntu 18.04 未纳入此基线或兼容矩阵，不能从 20.04 的验证结果推断其受支持。

只有最终发布 job 有 `contents: write`。发布工作流向 `scripts/build-installer.py --prebuilt` 传入已验证的 CI 产物；本地打包可传入按上述步骤生成的本地产物。打包器只校验与组装，不隐式调用 Cargo。

平台检查集中在 `scripts/windows_ci.py`、`scripts/macos_ci.py`、`scripts/linux_gui.py` 和 Linux 隔离网络夹具中；`scripts/test-release-package.py` 统一调度，`scripts/linux_ci.py` 负责基线容器和虚拟机。各 GUI 结果与失败日志保存在对应 `*-package-report-*` 附件中。

### Windows Defender 告警

历史扫描通过不代表后续检测库或云端检测仍给出相同结果。出现告警时先记录版本、下载来源、检测名及被拦截文件的 SHA256，核对安装包的公开 SHA256；安装包和安装后主程序是不同文件，不能直接比较两者哈希。Windows 打包收据中的 `defender.files` 同时记录安装包和未压缩程序，可按文件名核对主程序。

确认来源后，可通过 [Microsoft Security Intelligence 官方提交入口](https://www.microsoft.com/en-us/wdsi/filesubmission)选择软件开发者和误报类别，提交具体报毒文件，并附检测名、版本、哈希及公开来源。截图或 `!ml` 后缀本身不足以证明误报；提交前也不以修改二进制、关闭防护或设置排除项代替核验。主程序已有图标和版本资源，保留符号、Authenticode 签名与检测结果之间没有本项目已验证的因果结论。

## 手动验收与助手清理

Actions 验证基础安装、系统代理和 TUN；首次授权与取消、真实注销登录和重启、休眠、多显示器、系统通知仍需实机检查。更新验收应从旧版本开始，确认下载可取消和重试、校验失败不安装、安装失败可恢复旧版，升级后保留订阅与个人设置。

Windows 更新需要完成 UAC，Linux 需要 `pkexec` 与桌面授权服务；DEB 推荐安装 `pkexec` 或旧系统中的 `policykit-1`。macOS 数据保存在用户 Application Support，系统代理与 TUN 共用 launchd 网络助手，GUI 以普通用户运行；已有助手可供后台自启复用，退出恢复不应重新索取授权。首次下载后的 Gatekeeper 提示按 [Apple 说明](https://support.apple.com/zh-cn/102445)处理。

macOS 卸载前关闭代理并退出客户端，再按需移除当前用户的网络助手：

```sh
service_label="org.clashofrust.network.$(id -u)"
sudo launchctl bootout "system/$service_label"
sudo rm -rf "/Library/PrivilegedHelperTools/$service_label" "/Library/LaunchDaemons/$service_label.plist"
```

助手停止前会尝试恢复自己仍持有的代理设置并终止内核会话。测试夹具仅移除本次创建的助手，拒绝覆盖已有安装。
