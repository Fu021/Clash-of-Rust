# Agent 失败复盘与防回归清单

本文件汇总仓库历史修复提交与 GitHub Actions 失败记录，供开发 agent 在修改前排查同类风险、在完成任务后复盘。它记录的是历史证据和可复用检查项，不代表当前提交已经通过这些验证。

每次完成任务后都必须更新本文末的“最近复盘”；新增失败模式应同时说明症状、根因、修复、预防措施和证据。没有新模式时也要追加一条，并引用已覆盖的章节。

## 证据范围

- 扫描日期：2026-10-24。
- 提交范围：`git log --all`，包括已合入 main、开发分支、修复提交和已移除的临时验证分支。
- Actions 范围：GitHub Actions 中状态为 `failure` 的 62 次运行、146 个含失败步骤的 job 记录；读取失败 job 日志、失败步骤和公开运行信息。
- 失败运行分布：

| 工作流 | 失败运行 | 含失败步骤的 job |
| --- | ---: | ---: |
| CI | 26 | 86 |
| Publish Release | 13 | 21 |
| Memory Benchmark | 11 | 11 |
| Claude Protection Validation（临时） | 8 | 24 |
| macOS Fix Validation（临时） | 1 | 2 |
| Rust（旧工作流） | 1 | 1 |
| UI Screenshots（已移除） | 1 | 1 |
| release.yml（工作流级失败） | 1 | 0 |

146 条 job 记录中有 144 个结论为 `failure`，另有 2 个 CI job 在 `Post Restore private native Rust cache` 步骤失败后被取消；表中按“含失败步骤”合并统计。失败步骤以桌面“Compile, test and export native binaries”最多（61 个），其次是临时 Claude 原生验证（19 个）、安装包验证（15 个）、Android 编译（14 个）和 Android 真实代理/TUN（7 个）。一个运行常因同一根因让多个矩阵 job 同时失败，不能把 job 数量当作独立事故数量。

## 排查顺序

1. 先找最早失败的 job，而不是从大量后续矩阵失败中逐项猜测。`Resolve build inputs`、`Resolve verified CI` 或缓存恢复失败通常会跳过或带倒整组平台。
2. 区分四类问题：源码/生成代码错误、测试假设或竞态、外部服务或 runner 环境、产物/缓存/发布前置条件。
3. 对照本文件的错误签名和“首先检查”清单；再查看失败步骤、代码差异与关联修复提交。
4. 修复根因并补充最小回归测试。不要只为某台 runner、某个固定端口或某个临时路径加特判。
5. 任务收尾时更新本文“最近复盘”，并同步受影响的 README、模块说明、工作流文档和命令示例。

## 高频失败模式

### 1. 共用构建/发布脚本先失败

**典型症状**

- CI 在 `Resolve build inputs` 的 “Validate common build and package logic” 失败，其他平台尚未开始或全部被跳过。
- Publish Release 在 `Resolve verified CI` 的 “Verify CI and freeze source” 失败。
- Python 测试出现 `IndexError`、`ValueError`，但日志只显示最终 `Process completed with exit code 1`。

**代表证据**

- CI [40005445463](https://github.com/Fu021/Clash-of-Rust/actions/runs/40005445463)：`action_cache.py` 对空 `prefix` 取值触发 `IndexError`。
- CI [39771037547](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771037547)：共用构建测试失败。
- Publish Release [39815271615](https://github.com/Fu021/Clash-of-Rust/actions/runs/39815271615)：上游 CI 必需 job 未通过，发布被正确拒绝。

**首先检查**

- `scripts/build_inputs.py`、`scripts/workflow_artifacts.py`、`scripts/cache_shards.py`、`scripts/action_cache.py` 与 `.github/actions/local-cache/` 的接口和清单格式是否同步。
- 新字段是否允许旧缓存、旧 manifest 和空/缺失输入；失败时应安全回退，而不是越界。
- `.github/workflows/*.yml` 中冻结输入、artifact schema、job 名称和脚本参数是否一致。

**防回归**

- 同时修改生产脚本和 `scripts/test-build-tools.py` / `scripts/test-workflow-artifacts.py`。
- 对空输入、缺字段、过期 schema、重复 artifact、下载中断和校验失败写测试。
- 不把“重新运行 Actions”当作根因修复；若只是外部瞬时故障，记录证据并在“最近复盘”标明。

### 2. 缓存不完整或跨版本复用

**典型症状**

- `serde_core` 缺少 `OUT_DIR/private.rs`，或 Cargo 在尚未编译的依赖目录中报找不到构建产物。
- `skia-bindings` 的 Git 拉取出现 `expected 'acknowledgments'`、`expected 'packfile'` 或 503。
- 缓存下载超时、分片不完整，随后正常编译也失败。

**代表证据**

- CI [39946180066](https://github.com/Fu021/Clash-of-Rust/actions/runs/39946180066)：部分恢复的 Cargo 产物缺少 `serde_core` 生成文件。
- Android CI [38929932155](https://github.com/Fu021/Clash-of-Rust/actions/runs/38929932155)、[38925737608](https://github.com/Fu021/Clash-of-Rust/actions/runs/38925737608)：Skia 源获取失败并留下损坏 Git 状态。
- 修复提交 `91df487`、`eb08c8d`、`5b70648`、`adeb083`、`f0b61b1`。

**首先检查**

- 缓存是否按源代码、工具链、`Cargo.lock`、Gradle/Go 锁和构建脚本共同失效。
- 恢复失败后是否丢弃不完整编译对象，同时保留已验证源码快照和测试报告。
- 远端限流、超时和 DNS 故障是否有有界重试、兼容缓存和正常下载回退。

**防回归**

- 缓存归档必须校验内容、路径、符号链接和中断状态，不能只校验文件存在。
- 自动测试必须覆盖“部分恢复仍可正常重编”和“旧格式缓存不污染当前构建”。
- 可选缓存上传要有硬超时，不能拖垮已经通过的主构建。

### 3. Rust 编译与 Clippy 跨 target 差异

**典型症状**

- `-D warnings` 下出现死代码、未使用函数、未构造枚举变体、多余 `return` 或可合并 `if`。
- 某个平台通过而另一个平台因 `cfg`、依赖 feature 或测试 target 失败。
- 测试 target 缺少仅在主二进制启用的模块、crate 或 import。

**代表证据**

- CI [39805637475](https://github.com/Fu021/Clash-of-Rust/actions/runs/39805637475)：六个桌面平台均因 `timestamp`、`traffic` 未被使用被 Clippy 拒绝。
- CI [39771203085](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771203085)：枚举变体未构造导致 Clippy 失败。
- CI [39752822767](https://github.com/Fu021/Clash-of-Rust/actions/runs/39752822767)：多余 `return`。
- CI [39017685938](https://github.com/Fu021/Clash-of-Rust/actions/runs/39017685938)：`ManuallyDrop` 联合字段与 `chunks_exact` 的严格 lint。
- CI [39004271043](https://github.com/Fu021/Clash-of-Rust/actions/runs/39004271043)、[39002539543](https://github.com/Fu021/Clash-of-Rust/actions/runs/39002539543)：缺少 `log`/`url` 导入、Android 类型或稳定通道不支持的 API。

**首先检查**

- 条件编译后的每个 target、lib、bin、test 和 example 是否都实际使用新增代码。
- 公开常量、错误枚举、序列化分支和平台 helper 是否只在测试中被引用。
- 是否使用了当前 Rust stable 不支持的 API 或只在某个 target 存在的类型。

**防回归**

- 工具链充足时执行与 CI 一致的 workspace Release Clippy 和测试；资源受限时明确标注未执行。
- 测试与生产代码共享 helper 时检查 `cfg(test)` 和平台边界，避免测试专用分支进入生产编译。
- 不以放宽 lint 掩盖未使用代码；无用代码应删除，确需保留时说明原因并限制作用域。

### 4. Slint 界面契约与预览漂移

**典型症状**

- `.slint` 属性、回调、页面索引或演示 fixture 与正式入口不一致。
- 编译生成代码出现语法或类型错误，例如 “this looks like an `else if` but the `else` is missing”。
- 旧 UI 截图/预览脚本依赖已删除的测试钩子或固定坐标，出现找不到选择器、卡片或 tray 函数。

**代表证据**

- CI [39773451589](https://github.com/Fu021/Clash-of-Rust/actions/runs/39773451589)：桌面 UI target 在一次界面变更后出现大量生成代码编译错误。
- UI Screenshots [38595379460](https://github.com/Fu021/Clash-of-Rust/actions/runs/38595379460)：旧临时任务引用已不存在的 `native_present`、`remove_native_item_for_test`、`verify_native_state`。
- Memory Benchmark [38102403454](https://github.com/Fu021/Clash-of-Rust/actions/runs/38102403454)、[38058824407](https://github.com/Fu021/Clash-of-Rust/actions/runs/38058824407)、[38058444877](https://github.com/Fu021/Clash-of-Rust/actions/runs/38058444877)：旧预览脚本依赖固定坐标和分类卡片布局。

**首先检查**

- Slint 状态/回调、协议、宿主投影、正式窗口入口和预览 fixture 是否同步。
- `scripts/preview-slint.py --check` 是否能加载正式入口；预览数据是否含已删除属性。
- UI 测试是否通过正式消息路径，而不是复制状态或依赖临时截图脚本。

**防回归**

- 界面外观只使用持久安装、版本匹配的官方 Slint Viewer；不要恢复截图工作流。
- 交互测试使用稳定标识/可访问文本和状态等待，不依赖屏幕绝对坐标。
- 预览固定数据只能证明外观，不能写成真实交互、VPN 或系统集成已通过。

### 5. Android 工具链与原生依赖

**典型症状**

- SDK/NDK 下载包损坏：`Error on ZipFile unknown archive`。
- Skia、HarfBuzz 或 Chromium 源站 503，Git 结果不完整。
- Gradle lint、aapt2 元数据、GLES 依赖、Java/Rust 类型或 Android API 限制导致编译失败。

**代表证据**

- Android CI [39984691528](https://github.com/Fu021/Clash-of-Rust/actions/runs/39984691528)：NDK 28.2 archive 损坏，`sdkmanager` 返回非零。
- Android CI [39032112812](https://github.com/Fu021/Clash-of-Rust/actions/runs/39032112812)、[39004271043](https://github.com/Fu021/Clash-of-Rust/actions/runs/39004271043)、[39002539543](https://github.com/Fu021/Clash-of-Rust/actions/runs/39002539543)：三种 ABI 的编译或 lint 错误。
- CI [38966108986](https://github.com/Fu021/Clash-of-Rust/actions/runs/38966108986)：Android Clippy 拒绝可合并 `if`。
- 修复提交 `3fe26d8`、`023b126`、`4827013`、`65f3962`、`65446a7`、`7a24898` 等 Android 依赖与 native host 修复。

**首先检查**

- `scripts/android.py`、Gradle 配置、NDK/SDK 版本、Go module、Rust target 和构建标签是否一致。
- 下载缓存是否完整并校验；失败后是否清理不完整压缩包和 Git 源。
- Android target 是否遵守 stable Rust、`minSdk 26`、`compileSdk 36` 和 16 KiB ELF 对齐要求。

**防回归**

- 三种 ABI 都要编译；x86_64 还要运行模拟器集成测试。
- 新增 native 依赖必须固定来源、版本和校验，并保留许可证。
- 不把桌面端 feature、线程或文件系统假设带入 Android 宿主。

### 6. Android VPN、DNS 与系统 UI 时序

**典型症状**

- 真实代理/TUN 测试没有观察到 TUN 连接，或 VPN 重建期间出现直连窗口。
- 快捷磁贴未渲染、冷启动状态错误，或读取系统 UI 设置被拒绝。
- 模拟器连接异常、路由未就绪、DNS 不可用。

**代表证据**

- CI [39028411244](https://github.com/Fu021/Clash-of-Rust/actions/runs/39028411244)、[38993577546](https://github.com/Fu021/Clash-of-Rust/actions/runs/38993577546)：真实 TUN/模拟器连接失败或超时。
- CI [39771203085](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771203085)：快捷磁贴未被 System UI 渲染。
- CI [39005660080](https://github.com/Fu021/Clash-of-Rust/actions/runs/39005660080)：target SDK 36 读取受保护设置触发 `SecurityException`。
- 修复提交 `a706150`、`0f7d6a3`、`a2efffa`、`7cd86ae`、`bd23c5b`、`ee44de5`。

**首先检查**

- 新 `VpnService` 接口、地址和默认路由是否已由系统回调确认，再启动 kernel packet readers。
- 测试是否设置 direct-only 负对照；若直连也能通过，测试结论无效。
- 快捷磁贴是否通过可访问的系统 UI/广播状态验证，而不是读取普通应用无权访问的 `Settings.Secure` 键。

**防回归**

- 覆盖首次授权、拒绝、取消后的迟到授权、已授权、VPN 重建和关闭清理。
- 关闭界面不得停止 VPN；套接字必须绕过自身 TUN。
- 测试辅助状态必须有稳定来源，等待状态变化而不是固定 sleep。

### 7. Linux 代理、TUN、DNS 与通知

**典型症状**

- 网络命名空间内无法连接测试上游，或 IPv6 TUN/路由不可达。
- 系统通知服务不可用，导致 Claude 保护测试失败。
- 端口冲突、权限不足、证书校验失败，或系统代理/TUN 未在失败后恢复。
- X11/Wayland/compositor 崩溃被误判为应用失败。

**代表证据**

- Claude 临时验证 [38630109555](https://github.com/Fu021/Clash-of-Rust/actions/runs/38630109555)：Linux 通知服务不可用，Windows/macOS IPv6 TUN 路由失败。
- 修复提交 `0745177`、`532a0cf`、`7bee44c`、`8419283`、`2fbd14b`、`f5374b6`、`c87afd1`、`9960618`。
- 桌面 GUI 修复提交 `84c3a2e`、`2efd99b`、`4a3d177`。

**首先检查**

- 测试会话是否启动 D-Bus、Xvfb/合成器和通知服务。
- TUN 地址范围是否避开协议保留段；IPv4/IPv6、TCP/UDP 和 DNS 是否分别验证。
- 失败路径是否恢复代理、路由、DNS、证书和临时规则。

**防回归**

- 只在可丢弃环境修改系统代理、路由或 TUN，不在用户当前网络会话中试验。
- 测试端口动态分配并检查 TCP/UDP 同时可用；不要固定占用易冲突端口。
- 对正常节点、保护节点和 DIRECT 候选分别断言，避免“有流量”被误当成“经过代理”。

### 8. macOS 应用身份、launchd 与打包

**典型症状**

- 安装后的 GUI 崩溃（如 `GUI exited early: -11`）或内核启动退出 101。
- launchd 网络助手授权过期、更新交接失败或残留旧服务。
- Dock/托盘状态项重复或丢失，DMG 资源忙，安装包测试无法清理。

**代表证据**

- Publish Release [38791325938](https://github.com/Fu021/Clash-of-Rust/actions/runs/38791325938)：macOS 安装后 GUI 提前退出。
- Publish Release [38118779683](https://github.com/Fu021/Clash-of-Rust/actions/runs/38118779683)：安装后 core 退出 101，多项真实内核测试失败。
- 临时 macOS 验证 [38699305933](https://github.com/Fu021/Clash-of-Rust/actions/runs/38699305933)：`update-launchd` 失败。
- 修复提交 `95e213b`、`12b53f2`、`1c442bb`、`0ecbce2`、`f11e558`、`703528d`、`7a74cb5`、`a921814`。

**首先检查**

- 主应用、无 Dock 图标界面助手和内核/网络助手的 bundle identity、签名和 `LSUIElement` 是否各自正确。
- launchd label、用户 UID、helper 路径、租约和过期授权是否一致。
- 安装后测试是否从 DMG/应用资源实际启动，而不是引用 source tree 中的路径。
- AppKit 托盘、Dock 和窗口操作是否在主线程、事件循环正确轮次执行。

**防回归**

- 同时验证 Intel/Apple Silicon、安装/升级/重启/卸载、连续重建窗口和通知权限。
- DMG 操作保留有限重试，但测试结束必须卸载卷并清理本次创建的服务。
- 不把 ad-hoc 签名测试结果写成 Developer ID 或公证已经通过。

### 9. Windows 普通用户、服务与安装器

**典型症状**

- 非管理员原生检查超时，事件、管道或计划任务 ACL 拒绝访问。
- 安装/卸载文件被占用，`uninstall.exe` 清理出现 `WinError 5`。
- NSIS 标签缺失、Defender 签名更新占用或退出码异常。
- Windows Toast 未注册或排序指示未重绘。

**代表证据**

- CI [39625198442](https://github.com/Fu021/Clash-of-Rust/actions/runs/39625198442)、[39621393212](https://github.com/Fu021/Clash-of-Rust/actions/runs/39621393212)：非管理员原生检查超时。
- Publish Release [39784808500](https://github.com/Fu021/Clash-of-Rust/actions/runs/39784808500)：NSIS 打包失败。
- Publish Release [38595372576](https://github.com/Fu021/Clash-of-Rust/actions/runs/38595372576)：卸载器仍被占用，测试清理 `PermissionError`。
- Publish Release [38531074228](https://github.com/Fu021/Clash-of-Rust/actions/runs/38531074228)：Defender 签名更新失败。
- 修复提交 `257a03a`、`0ecd3a2`、`4bcc369`、`128c148`、`638a8e0`、`6cd64e8`、`8ba423b`、`1aaa1da`。

**首先检查**

- 测试是否以普通用户运行；安全描述符是否同时允许服务、事件所有者、管道和测试进程。
- 安装后验证是否等待安装器/卸载器进程真正退出，再删除临时目录。
- NSIS 宏、标签和版本参数是否与脚本一致。

**防回归**

- 不使用管理员权限掩盖普通用户失败；需要提权时只通过正式服务/安装入口。
- 对首次授权、拒绝、重复安装、升级、卸载、文件占用和 Defender 忙重试保留回归测试。
- 清理只删除本次 fixture，不触碰用户已有安装或网络配置。

### 10. 发布产物、版本与依赖快照

**典型症状**

- Publish Release 发现上游 CI 未通过，或 artifact 下载 DNS/网络失败。
- manifest 缺少 native payload、commit/arch/schema 不符、Android 依赖快照与 CI 不一致。
- 版本、安装包名称、更新通道或安装时间戳判断错误。

**代表证据**

- Publish Release [39784808500](https://github.com/Fu021/Clash-of-Rust/actions/runs/39784808500)：macOS 获取 artifact 时 DNS 失败。
- Android 打包 [38989293563](https://github.com/Fu021/Clash-of-Rust/actions/runs/38989293563)：依赖快照与已验证 CI 不一致。
- 修复提交 `8b7ff41`、`9d705d5`、`1aaa1da`、`465fec6`。

**首先检查**

- `ci_run_id` 是否属于同一提交、同一轮依赖快照和成功的必需 job。
- artifact 的 schema、commit、version、system、arch、profile、逐文件校验值和测试程序是否完整。
- 发布的安装包是否由已验证产物组装，而不是重新编译。

**防回归**

- 开发分支测试包保持 `check_only`；未获授权不创建正式 Release。
- 瞬时下载失败只做有界重试，不能绕过校验或改用手工未验证文件。
- 版本号和发布记录只在发布任务中修改，并同步 README、安装包名和更新测试。

### 11. 测试稳定性与外部环境假设

**典型症状**

- 单元测试依赖真实订阅、默认节点或固定端口，换 target 后失败。
- fixture 文件已存在、端口绑定失败、TLS 根证书或反向 DNS 不可用。
- UI/窗口测试固定等待时间，偶发超时或把合成器崩溃算作应用失败。

**代表证据**

- CI [39618278378](https://github.com/Fu021/Clash-of-Rust/actions/runs/39618278378)：规则测试因 fixture 没有 `proxies`/`proxy-providers` 失败。
- CI [39005660080](https://github.com/Fu021/Clash-of-Rust/actions/runs/39005660080)：provider 自动组断言缺项，另有构建工具 `KeyError`。
- 临时验证 [38640723255](https://github.com/Fu021/Clash-of-Rust/actions/runs/38640723255)：重复创建 `target/claude-build` 触发 `FileExistsError`。
- Memory Benchmark [39803098220](https://github.com/Fu021/Clash-of-Rust/actions/runs/39803098220)：等待恢复窗口超时。
- 修复提交 `9960618`、`2fbd14b`、`f5374b6`、`c87afd1`、`84c3a2e`、`2efd99b`。

**首先检查**

- fixture 是否自包含、固定时钟/输入，且不继承运行环境的 `HOME`、代理、DNS 或数据目录。
- 每个测试是否独占临时目录、动态端口和明确清理路径。
- 等待条件是否可观测并与超时分开；失败时是否保留脱敏诊断。

**防回归**

- 不把固定 `sleep`、固定坐标、固定端口或系统默认路由当同步机制。
- 测试不应读取用户真实订阅、Cookie、密钥或历史数据。
- 记录“测试失败”与“测试前置失败”的区别，不能把后者写成功能回归。

### 12. 手动运行参数与工作流生命周期

**典型症状**

- 手动 benchmark 的 `ref` 误填短 SHA，checkout 把 `6473bbe` 当作 ref 模式拉取并失败。
- 已删除的临时工作流仍被手动触发，或依赖已移除的测试钩子。
- 大缓存恢复阻塞 fan-out，或旧 artifact 在新 schema 下继续使用。

**代表证据**

- Memory Benchmark [37971443677](https://github.com/Fu021/Clash-of-Rust/actions/runs/37971443677)：checkout `6473bbe` 的 fetch 失败。
- UI Screenshots [38595379460](https://github.com/Fu021/Clash-of-Rust/actions/runs/38595379460) 和旧临时验证工作流均已移除。
- 修复提交 `0a29d3b`、`c177290`、`e660a95`、`7034acf`、`b07d078`、`5b70648`。

**首先检查**

- 手动运行参数是否是完整 SHA/分支/标签；短 SHA 只能作为展示，不应作为 checkout ref。
- 工作流文件、artifact schema、脚本入口和文档是否同时删除或更新。
- 被跳过的临时工作流是否仍保留手动触发入口；弃用后应删除文件并由维护者在 GitHub 禁用。

**防回归**

- 每轮只保留 CI、Publish Release、Memory Benchmark 三个长期工作流。
- 手动 workflow 输入必须有校验和明确错误信息。
- 缓存恢复不能让整个平台矩阵无限等待；必须有超时、部分失败和正常下载回退。

## 任务收尾检查

- 重新阅读实际 diff，不只看自述；确认没有无关格式化、锁文件升级、临时调试代码或生成物。
- 执行与改动相称的最小验证；资源受限时只做静态检查或最小 UI 预览，并明确标注未执行项。
- 运行 `git diff --check` 和 `git status --short`，确认没有 `target/`、`dist/`、`bundle/`、`ci-build/`、缓存或日志。
- 检查新增/修改的路径、命令、链接、版本、平台支持和历史发布说明是否同步。
- 更新下方“最近复盘”，然后才能给出最终完成报告。

## 最近复盘

每次任务新增一行，最新在上。没有新模式时引用已有章节；重复记录只保留日期、任务、证据和验证范围。表格保持有界，超过 20 行时将已稳定的经验合并到上方模式章节。

| 日期 | 任务 | 复盘结果 | 验证与未验证项 |
| --- | --- | --- | --- |
| 2026-10-24 | 审阅并合并 Fake-IP 设置与排除项分支 | `f4fd846` 共修改 19 个文件，新增 DNS 覆盖下的 Fake-IP 独立开关、关闭覆盖时的 `redir-host` 回退、最多 64 个校验/去重排除项、桌面管理页、CLI 配置和协议 v4；以 `80739ba` 合并。沿用第 1、3、4、11、12 节，未发现新的失败模式。 | 已检查完整 diff、`git diff --check`、Markdown 链接/锚点、提交引用，桌面与 Android/iOS Slint `--check`，并生成单页设置预览；仅见仓库原有的 viewport 弃用警告。未编译、测试或打包。 |
| 2026-10-24 | 全仓文档治理与失败历史扫描 | 建立本文件；确认 62 次失败运行、146 个含失败步骤的 job（144 `failure`、2 `cancelled`）。主要问题集中在共用构建脚本、缓存完整性、跨 target Clippy、Android/Linux/Windows/macOS 平台集成和发布前置校验。 | 已做 30 个 Markdown 文件相对链接/锚点检查、53 个提交引用校验、仓库产物扫描和 `git diff --check`；未编译、测试、打包或运行 UI 预览。 |
