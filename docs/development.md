# 开发与验证

按本次改动阅读职责、数据流、平台或验证的相关章节；跨任务约束与收尾流程见 [AGENTS](../AGENTS.md)。功能和修复的循环检查按[循环审查工作流](loop-workflow.md)执行。其他主题从[文档导航](README.md)进入。

## 目录与职责

[全仓职责树](structure-review.md#目录与职责)集中说明各端宿主、共享模块、脚本和资源入口；文档冲突、大文件拆分和整体重构按[结构复查](structure-review.md)执行。GUI/CLI 共用运行时，平台权限、文件访问和网络请求保留在宿主/共享业务层。

## 平台编译边界

桌面端的构建、Clippy 和测试不得编译移动端专属宿主、界面或任何移动端测试；移动端的构建、Clippy 和测试也不得编译桌面端专属宿主、界面或任何桌面端测试。该规则包括本地开发、普通及扩展 CI、基准和打包前的编译；测试使用什么操作系统运行不改变测试归属。格式检查不编译代码。

共享业务、数据模型和不依赖特定平台状态的通用界面组件可以被两端各自编译；组件可以维护自己的交互状态，通过属性、回调或轻量接口接入宿主能力，不引用某一端的状态、动作、页面或宿主。跨端通用的业务逻辑留在共享层；平台状态、权限、动作及其适配留在所属端，只有一端使用的代码也留在该端。修改共享接口时同步两端调用者，不能为了复用模型而导入另一端的完整状态。

桌面消息协议与渲染桥都由共享模型和桌面状态生成；调整声明位置或内容时同步两个生成入口及其 Cargo 变更追踪，并验证消息往返和实际界面桥接。

测试由所属宿主包编译，不能借用另一端的宿主或将两端专属界面混入一个测试夹具。桌面 UI 测试放在 `crates/client-ui/tests/`，移动 UI 测试放在 `apps/android/native/tests/`；同一行为需要覆盖两端时分别建立测试。调整共享组件时分别在两端的任务中验证，不能将其中一端的测试塞进另一端。脚本回归检查两端的 workspace 依赖方向和 Slint 导入图，并核对共享改动触发两端各自的回归。

桌面构建通过 `desktop_cargo_scope` 限定已确认的桌面和共享包，新的 workspace 成员默认排除，确认职责后才可加入；Android 只选择移动宿主和移动运行时及其共享依赖。不得直接用未限定范围的 `cargo build/test/clippy --workspace` 代替平台构建。桌面和移动编译目录、CI 缓存族彼此独立；发现混入对方产物时清理受影响缓存，并用新的缓存版本分别重建，不能回退到旧缓存。缓存处理见[缓存分工](workflows/cache-coverage.md)。

本地分别运行界面回归的示例：

```sh
CARGO_TARGET_DIR=target/desktop cargo test -p clash-client-ui --test desktop_lists --release --locked -- --test-threads=1
CARGO_TARGET_DIR=target/mobile cargo test -p clash-android --features mobile-ui-tests --lib --test mobile_lists --test scroll_remount --test claude_report_blocks --release --locked -- --test-threads=1
```

Android 的生产窗口回归通过 `--lib` 共用一次 Slint 编译；列表、滚动和报告探针继续使用各自的轻量窗口。

本地 `target` 体积和可回收构建产物按[磁盘管理](workflows/local-development.md#本地-rust-构建与磁盘管理)检查；不能把另一端已有的构建结果当作当前端的验证。
本地 `scripts/android.py build` 默认使用 `target/mobile`，也可指定其他独立的 `CARGO_TARGET_DIR`；手工执行 Cargo 时同样显式选择所属端的目录。

## 仓库占用锁

Linux 本地磁盘上的开发、预览和构建会话在整个使用期间持有共享 `flock`。锁文件固定为 `git rev-parse --path-format=absolute --git-common-dir` 返回目录中的 `repo-usage.lock`，关联 worktree 共用同一把锁；不提交到仓库。文件内容为空，是否占用只看内核锁，不能按文件存在、PID 或时间判断。

在实际执行工作的同一 Bash 会话中获取锁，以下描述符须一直保留到会话结束；获取失败退出码为 `75`：

```bash
cor_git_common=$(git rev-parse --path-format=absolute --git-common-dir) || exit
umask 077
exec 9>>"$cor_git_common/repo-usage.lock" || exit
flock --shared --nonblock --conflict-exit-code 75 9 || exit "$?"
```

专用维护 agent 必须在同一路径获取**排他锁**，使用 `flock --exclusive --nonblock --conflict-exit-code 75 9`；若占用则跳过该仓库。维护期间持续持锁，不能检查后解锁再操作。所有使用方须先遵守此协议；共享锁只防维护操作与使用会话重叠，不负责串行化开发者之间的 Git 写操作。

会话正常退出、异常退出或关闭描述符时自动释放，完成后也可执行 `exec 9>&-`。不删除或替换锁文件，不删除包含它的 Git 公共目录，否则其他进程可能锁住不同 inode。不同主机的独立 clone 各自加锁；跨主机共享磁盘须先确认文件系统的 `flock` 语义。

## 版本管理

维护者的修复与新功能直接在 `main` 开发，不要求另建分支或因开始任务自动改版本。准备新版本时，先确认发布授权；预发布使用合法 SemVer，可在下一 patch 后附类型、递增序号和简短内容。正式版为 `0.7.0` 时：

| 版本用途 | 版本示例 |
| --- | --- |
| 功能预发布 | `0.7.1-dev.1.fake-ip-sources` |
| 修复预发布 | `0.7.1-fix.1.android-protection` |

序号在相同基础版本和类型中递增，首次为 `1`；再次提供该类型的预发布安装包时递增序号，内容描述不承担排序。描述只用 ASCII 字母、数字和连字符，多个标识用点分隔；`0.7.1-dev:描述` 不符合 Cargo/打包器的 SemVer 格式。`0.7.0 < 0.7.1-dev.1.fake-ip-sources < 0.7.1`；`dev` 与 `fix` 仍按 SemVer 字典序比较，不代表开发先后顺序。

版本的权威来源为根 `Cargo.toml` 的 `[workspace.package].version`。同步 `Cargo.lock` 中所有 workspace 包、Android Gradle 的版本默认值和离线预览 fixture；不改第三方包版本或历史 Release 文档。正式发布去除后缀、跳版本及其他例外仍需维护者明确授权。版本变化后必须重新取得同提交 CI，不能复用旧版本的构建产物。

安装包版本核验及高版本本地包的更新行为见[版本与更新通道](workflows/versions.md#版本与更新通道)。

## 翻译资源

仓库根目录的 [`translations/`](../translations/) 集中维护客户端与 Windows 安装器的翻译资源。以下目录以 `translations/` 为起点。

| 资源 | 路径与构建入口 |
| --- | --- |
| Rust | `shared/`、`cli/`、`runtime/`、`desktop/rust/`、`android/rust/` 分别由 `client-core`、CLI、`client-runtime`、桌面宿主、Android Rust 宿主的 `build.rs` 通过 [`compile_catalog.rs`](../translations/compile_catalog.rs) 生成各自的 `OUT_DIR/messages.rs`。 |
| Slint | `client-ui` 和 Android Rust 宿主分别内嵌 `desktop/slint/<Slint 标签>/LC_MESSAGES/clash-client-ui.po` 和 `android/slint/<Slint 标签>/LC_MESSAGES/clash-android.po`。简体中文使用 `@tr` 的源文。 |
| Android Java | `android/system/values*/strings.xml` 由 `scripts/android.py` 复制到 Gradle 生成资源目录，Java 按资源 ID 读取。 |
| Windows 安装器 | `windows/nsis/<语言标签>.nsh` 维护安装和卸载文案，由 [`clash-of-rust.nsi`](../installer/clash-of-rust.nsi) 加载。 |

Rust 新增文案时，为对应目录的各个 `<语言标签>.json` 添加相同的 `snake_case` 消息键和 `{name}` 参数集合，并在调用处使用生成的 `messages::消息键(language, ...)`。参数按名称排序传入，模板可按各语言习惯调整顺序，字面花括号写成 `{{` 和 `}}`。编译期校验目录中的语言文件、消息键和参数，英文目录提供回退文案。界面静态文本使用 `@tr` 或 Android 资源 ID。

部分旧 Rust 提示仍由 `Language::text` 或错误投影直接提供中英文，新语言会沿用英文。补译这些提示时先迁入所属词库，旧报告和后台错误的原始文本继续由展示层按已知格式识别。

### 新增语言

在同一次变更中补齐客户端、Windows 安装与卸载向导、Linux 应用菜单。

1. 在 [`languages.rs`](../translations/languages.rs) 注册语言标签、菜单名称、Slint 标签及系统匹配规则，补齐表中的五套 Rust 目录、两套 Slint 目录和 Java 资源。GUI、TUI 和 Android 语言菜单、CLI 可选标签及 Android 系统语言匹配共用注册表。
2. 完成 [Windows 安装与卸载语言清单](#windows-安装与卸载语言清单)。
3. 同步 [`clash-of-rust.desktop`](../resources/linux/clash-of-rust.desktop) 的 `Comment[语言]`。Linux 应用菜单按桌面会话语言读取对应注释。
4. 按[平台编译边界](#平台编译边界)分别运行桌面与移动测试，在四端检查系统匹配、手动切换和英文回退，覆盖旧设置、长文本、带参数消息与 Android 资源限定符。行为规则见[语言与展示](ui-design.md#语言与展示)，自动选测见 [CI](workflows/ci.md#ci)。

#### Windows 安装与卸载语言清单

安装和卸载向导直接打开，语言跟随当前用户的 Windows 显示语言。NSIS 先匹配完整语言，再匹配主语言，均未匹配时用首个加载的英文词库。参见 [NSIS 手册 §4.10.1](https://nsis.sourceforge.io/Docs/Chapter4.html)。

- 添加 `translations/windows/nsis/<语言标签>.nsh`，文件名与客户端注册表的语言标签一致。以 `en.nsh` 为模板，使用对应的 NSIS `MUI_LANGUAGE` 和语言 ID，补齐 `InstallerVersionInfo` 及全部 `LangString`，保留变量、路径和换行占位符。
- 在 [`clash-of-rust.nsi`](../installer/clash-of-rust.nsi) 的语言加载区沿用 `INSTALLER_LANGUAGES` 和 `COMPILER_PATH_SEPARATOR`，以 UTF-8 加载词库，保持 `en.nsh` 第一个加载。将新语言的卸载快捷方式名登记到 `DeleteUninstallShortcuts`，让重装和卸载能够清理旧语言留下的快捷方式。
- 运行以下检查。缺少词库、漏加载、缺消息键或漏登快捷方式清理项都会失败。CI 也运行这两项检查。

  ```sh
  python3 scripts/test-build-tools.py \
    BuildTests.test_language_registry_has_bundled_platform_resources \
    BuildTests.test_windows_installer_uses_complete_localized_messages
  ```

- 编译 Windows x64 和 ARM64 安装包。在隔离 Windows 环境分别使用新增语言及一种主语言尚未受支持的显示语言，核对安装和卸载向导、英文回退、切换系统语言后重装或卸载的快捷方式清理，以及静默更新。

## 修改前后都要检查

- GUI/CLI 共享数据目录、后台所有者和内核；检查两端刷新、退出与重新附加，避免重复内核、轮询或测速。
- 语言仅在展示边界转换。内置默认直连名称由共享层按当前语言生成。协议键、持久化值、筛选与排序标识、用户命名、JSON 字段及外部原始证据保持原样。新增可见文案同时检查两种语言、旧数据和长文本布局，语言选择规则见[界面设计](ui-design.md#语言与展示)。
- 界面选项贯通 Slint 状态/回调、协议、宿主、业务、持久化和刷新，并检查桌面/移动端。协议变更同步收发端、序列化、预览和测试，明确兼容性与版本变化。
- 外部更新不得覆盖规则、订阅或设置草稿；保留并发校验、删除确认、原订阅与配置原子保存/回滚，覆盖拒绝、超时、退出和重启恢复。
- 规则新增、编辑和启用统一拒绝重复的启用规则，表单拒绝越界位置；数量上限等校验通过后才替换草稿，拒绝不得污染原草稿。拖动按起始规则 ID 提交，筛选或分页变化后立即失效，不能只等待异步状态回调清理。
- 订阅先完整下载并校验有效 YAML，再事务保存/重载；不可达、超时、HTTP 错误、空白/错误页及截断均失败，保留内容、用量、时间、设置、草稿和运行状态，不切默认直连。成功才失效测速与检测结果。定时任务由唯一所有者执行，提交前复核订阅、来源、文件、开关及保护，丢弃过期结果；调度、通知及 Android 生命周期见[订阅说明](subscriptions.md)。
- Geo 与安装包按[共享下载约束](../crates/client-core/src/download.rs)回退并缓冲写盘；逐路线下载和校验，最后路线允许慢网，不跳过 SHA256/大小校验或改节点。Geo 整组暂存、事务安装及恢复见 [Geo 复盘](agent-failures/android.md#geo-与安装包下载)；安装及必要重载成功才更新时间，Android 保留 VPN。
- 控制器只监听本机，使用随机密钥并处理内部端口冲突；密钥不得进入日志、示例或界面。
- 后台 `Command::check_protection` 使用穷尽匹配声明只读、外观差异和写入策略，新增命令必须明确分类；保持 JSON 命令名称和旧字段默认值，不以默认匹配隐式决定保护权限。
- 系统代理、TUN、DNS、提权、自启动、通知与更新沿用平台入口，检查授权、拒绝、重复操作和失败清理。
- Linux 发行构建与最老系统验证边界见[CI](workflows/ci.md#ci)和[打包验证](workflows/release.md#publish-release)；共享宿主内核的容器不等于旧系统内核验证，不以新版系统成功替代旧系统证据。
- macOS 保护的动态 DNS、Tailscale 分域、原生就绪与失败隔离按[保护说明](claude-protection.md#dns-与-tailscale)执行；实现前查[DNS 共存复盘](agent-failures/macos-dns.md)。在可丢弃原生环境验证接管、重连、故障及恢复，不以配置写入成功代替实际解析，不要求用户调整 DNS。
- 系统代理使用后台实际托管状态，窗口重建只投影快照；辅助字段变化不判为关闭。恢复只合并仍托管的字段，保留外部修改与网络服务增删，详见[代理状态与安全恢复](agent-failures/desktop.md#系统代理状态与安全恢复)。
- 桌面网络恢复由 GUI/CLI 唯一所有者监控，不依赖窗口轮询或公网探测；离线转联网与默认路由变化稳定后刷新当前运行配置、出站/DNS 及旧连接，保留实际模式、节点、TUN、保护与磁盘数据。读取失败不能冒充断网，恢复失败退避重试，见[断网启动恢复](agent-failures/network-recovery.md)。
- Android VPN/通知独立于 Activity，关闭窗口不停止 VPN；保留套接字绕过自身 TUN。macOS 原生 UI/Dock/主线程、Windows 消息循环与窗口重建遵循各自宿主生命周期。
- 维护脚本优先复用共享模块；独立入口的保留依据见[脚本说明](../scripts/README.md)。

## FFI 与特权边界维护

平台 FFI、`unsafe` 和特权入口持续随相关改动逐步复核。各平台的实现、已验证范围及限制集中在[本地权限与更新信任边界](security-boundaries.md)；完成一批改进后，后续改动仍按以下要求维护。

- 新增或修改相关调用时，在对应 `unsafe` 处写明实际适用的 `SAFETY` 前提：指针是否可空、长度与对齐、ABI 和初始化状态、句柄及缓冲区生命周期、所有权与释放责任，以及线程或同步约束。说明必须对应调用链和平台 API 契约，避免通用模板注释。
- 可复用的不变量收敛到小范围安全接口，资源沿用明确的单一所有者和 RAII 清理；复核部分初始化、提前返回和错误出口。需要保留系统错误码时，在其他调用、格式化或资源释放前立即捕获。
- 按改动风险在既有测试中覆盖相关失败与清理行为，例如空值或类型错误、无效句柄、授权拒绝、子进程异常退出及资源回收。无法注入或缺少原生环境的路径明确记录验证边界。
- 保留已经启用的 `clippy::undocumented_unsafe_blocks` 检查；关键模块完成实际复核后逐步扩大局部覆盖，再评估工作区统一规则。注释和 lint 均须与所有权及失败路径审查配合，不能以批量机械注释或放宽检查代替。
- 复用受影响平台的现有原生 CI 和适用兼容验证，按[循环审查](loop-workflow.md)复查调用者及相邻清理路径。局部回归和一个平台的成功仅证明其实际覆盖范围，后续维护继续保留未验证项与限制。

## 内存与响应速度

- 日志、连接、列表和缓存有界，按页面构建数据、复用模型，释放隐藏页面与关闭窗口的闲置资源。
- UI 线程或引擎锁内不等待长下载，任务可取消，迟到结果不覆盖新状态；轮询复用后台所有者。订阅、Geo 与 CLI 更新的断开及安装边界见[下载任务](agent-failures/downloads.md)。
- 性能结论必须实测并注明版本、环境、输入和方法；区分 RSS/PSS、窗口开/关与相同负载。基准按任务需要安排，常规使用低强度场景，压力场景另标；预览不作基准。

## 按改动选择验证

先根据实际 diff、调用链和风险确定本任务的验证范围，不把客户端发布门禁套用于每项待办。同一提交有多个任务时分别记录各自证据；CI 自动执行的其他 job 不构成追加手动工作流的理由。

| 改动 | 验证范围 |
| --- | --- |
| 纯文档 | 内容、链接、锚点及 diff；随功能提交，不单独触发 CI。 |
| 独立网站内容、布局与交互 | [网站循环审查](loop-website.md)规定的静态检查、浏览器及长图、部署和约定验收；不追加客户端编译、安装、Android/Ubuntu 兼容、内存或不发布核验。 |
| GUI、TUI、CLI 功能或共享业务 | 受影响调用链的源码、交互、状态、持久化和生命周期回归；按实际影响选择平台及安装验证，不因入口数量机械重跑全部平台。 |
| 客户端运行库、渲染器或平台依赖升级 | 受影响平台的编译、渲染及宿主生命周期；影响旧系统支持时增加对应兼容验证，影响资源使用时安排有对照的内存基准。 |
| 安装、更新、打包或公开发布 | 按受影响安装合同核验；公开发布按[发布门禁](workflows/release.md#兼容性发布门禁)完成同提交所需兼容及发行核验。 |

只有任务确实需要核对候选安装包或发行合同才派发不发布核验。工作流变更、审核和发布权限按 [Agent 提交规则](../AGENTS.md#actions提交与发布)执行。缺少相关验证时记录具体缺口，不用无关测试的成功补齐。

## 本地验证与 UI 预览

- 资源充足时优先在本地复现、修复和验证，沿用同一 Release profile 与 `--locked`。本地通过后，仍须按[改动范围](#按改动选择验证)完成同提交 CI，并验证受影响的操作系统、架构和发行版。文档改动检查内容与链接，新增测试覆盖行为、边界或恢复。构建与 CI 入口见 [README](../README.md#编译与打包)及[工作流说明](workflows/ci.md#ci)。
- 模块拆分或新增 `#[path]` 后，从受影响 crate 的 `lib.rs` / `main.rs` 递归运行 `rustfmt --edition 2024 --check`，不能只逐个检查改动文件；资源充足时再执行 `cargo fmt --all -- --check`。属性指定的模块路径会影响子模块查找，必要时显式指定相对路径；格式检查不能替代类型检查。
- UI 使用持久安装、版本匹配的官方 Slint Viewer 直接预览正式组件，可用 `--viewer PATH` 指定；参数见 [UI 预览](workflows/local-development.md#ui-预览与静态资源)及[Slint 说明](../crates/client-ui/README.md)。
- 桌面默认 950×700、最小 800×450，移动端按[代表性尺寸与可用性检查](loop-workflow.md#多尺寸-ui-检查)覆盖手机横竖屏、平板与系统字号；检查主题、强调色、滚动、弹窗。复用主题控件与间距，语义颜色不被强调色覆盖；安全区/键盘只扣一次，内容合理顶部对齐。
- 固定演示数据只能验证外观，外观改动附实际截图；交互、后端与系统功能另验。真实代理/TUN 测试只在可丢弃环境执行，不擅自改用户路由、防火墙、代理或重启网络。
- 汇报区分“通过 / 失败 / 未执行 / 仅静态检查”，说明资源、设备、授权或工具链缺口；其他提交的 CI 不算本次验证。

## 依赖维护

- [依赖核对表](dependency-audit.csv)仅为 2026-10-24 历史快照。当前版本以源码、锁文件和[工具基线](../.github/build-versions.json)为准；升级同步声明及兼容说明，取得同提交的[审计附件](workflows/ci.md#ci)和验证，历史日期不保证当前安全。
- YAML 使用维护中的 `yaml_serde` / `libyaml-rs`，替换已停止维护的 `serde_yaml` / `unsafe-libyaml`；保留原订阅、别名、标签和引号标量语义。桌面私有管道使用 `postcard`，协议号以 [`PROTOCOL`](../crates/desktop-protocol/src/lib.rs) 为准，收发端须一起更新，保留帧长度、截断和尾随数据检查。报告国旗使用 UTF-8 文本，与普通标签共用 `Text`，不传递图像代号。
- HTTP 客户端统一使用 `clash_client_core::network::client_builder`，保留内置公共根证书；`reqwest` 0.13 的平台证书校验默认值不能直接用于未接入 Java verifier 的 Android 宿主。JNI 0.22 原生入口先经 `EnvUnowned::with_env`，工作线程经 VM 回调获取 `Env`，全局引用和异常处理遵循其生命周期。
- macOS `tray-icon` 固定为与 Slint 1.19 共用 `muda` 0.21 原生菜单类的 0.26.1；升级时同时复核桌面宿主与 shell 的声明、锁文件和原生启动，不能保留两份菜单类实现。Slint/ksni 的 zbus 5、字体栈的 `ttf-parser` 及 `typed-index-collections` 的可选 bincode 尚受上游约束；不以强制替换不兼容主版本掩盖这些链路。
