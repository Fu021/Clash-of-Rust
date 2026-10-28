# Android 宿主

版本统一取自根 [Cargo.toml](../../Cargo.toml) 的 workspace 声明，分支规则见[开发指南](../../docs/development.md#分支版本)。最低 Android 8.0（API 26），支持 ARM64、ARMv7 和 x86_64。iOS 仍单独保留界面与宿主占位，不参与此次实现。

## 结构

- `native`：Slint NativeActivity 入口、JNI、异步任务和界面模型；正式入口为 `crates/client-ui/ui/mobile/android/window.slint`。
- `app`：Android 生命周期、文件选择、通知、应用分流及前台 `VpnService`。关闭界面不等于断开 VPN。
- `core`：按 CI 依赖快照锁定的 mihomo Go 模块及 gomobile 桥接；使用上游 `cmfa,with_gvisor` 构建标签，路由与应用排除交给 `VpnService`，出站套接字经 `protect` 排除，避免进入自身 TUN。
- `crates/mobile-ffi`：不依赖 Slint、JNI 或 Android SDK 的共享会话、配置、订阅、规则和控制器操作。原生能力由 `NativeHost` 注入，iOS 可以另接宿主。

## 当前能力

实际接入订阅添加、文件导入、编辑、手动/定时更新与切换；设置可调整自动更新间隔（默认 360 分钟，0 关闭），仅处理远程源且失败保留原状态。策略组、节点选择、排序、单组与全部测速；VPN 连接与断开、快捷设置磁贴、应用排除；连接速率、排序、详情与关闭；规则草稿、快捷规则与应用；日志筛选、复制与导出；网络诊断、IP 分类检测和浏览器 A÷检测；主题、强调色、DNS/Fake-IP/IPv6/UDP 与端口设置。字号跟随 Android 系统设置，使用系统 SP 换算（含 Android 14 的非线性缩放），不使用桌面端保存的字号；卡片间距与内容高度适应字号，触摸目标保留最小尺寸。

删除正在使用的订阅时，确认后自动切换到默认直连，再删除订阅及其规则；内核重载或文件提交失败会恢复原状态。默认直连配置不能编辑或删除，旧版内置配置会保留原 ID 并补齐标识；缺少内置配置时只补建，不覆盖已有订阅。添加和编辑订阅提交后立即显示保存状态，暂时锁定表单并阻止重复提交；失败保留输入供重试，成功返回列表。Claude 保护开启时，仍需先关闭保护才能删除当前订阅。

连接、规则与日志使用从顶部排列的虚拟列表，列表与筛选框适应可用宽度。Pad 完整侧栏与手机保持相同的导航层次，连接、规则、日志和三项检测均收在“工具”内；宽屏分页与规则保存操作采用紧凑单行，手机及大字号保留适合触摸的换行布局，详见[移动界面说明](../../crates/client-ui/README.md#android--ios-界面)。

规则表单与桌面共用 18 种类型，支持 MATCH、适用类型的 `no-resolve`、启停和完整文本编辑。顶部订阅、快捷规则与筛选随规则列表滚动，窄屏搜索单独成行；按来源筛选后，长按行左侧手柄拖动排序，靠近列表边缘持续滚动。右箭头提供完整规则原文、来源和策略，并可复制、编辑、启停、删除、指定跨页位置及恢复订阅顺序，均先保存草稿再统一应用。Fake-IP 排除项紧接开关下方，来源、内置项还原共用桌面配置语义。IP 分类结果支持搜索、结果/地区筛选、详情和单项重测；更新下载可取消并清理未完成缓存，Release 说明由系统浏览器打开。

Geo 与安装包下载复用[共享下载约束](../../docs/development.md#修改前后都要检查)，先尝试内核代理，再按失败或持续慢速情况回退直连；不自动切换用户节点。Geo 下载期间继续使用旧数据，四个文件完整校验后统一安装并重载，失败恢复整组旧文件，保留 VPN、订阅与规则草稿；后台冷启动也恢复未完成事务。下载速度受实际节点、源站与网络影响，需用同一设备和下载任务验证。

下拉系统快捷设置并编辑磁贴，可添加 Clash of Rust 开关；首次需要 VPN 授权时会进入应用，后续直接连接/断开，包括应用进程已退出的情况；磁贴实时更新连接状态。

手动测速使用自动策略组自己的测试地址，完成后让内核按该组配置重新选优；测速结果直接更新列表。Claude 保护开启时禁止切换节点、编辑、更新或删除当前订阅，并在运行配置中冻结自动组，关闭后恢复原订阅的自动策略。订阅校验由共享会话执行，其他订阅仍可编辑。

Claude 保护先取得系统 VPN 授权，再应用保护配置；等待授权或取消时保留原网络和 DNS，已取消操作不会因迟到的授权而连接。启用时固定当前具体节点，临时启用 VPN、IPv6、UDP、代理加密 DNS 与实时拦截，并覆盖应用排除；关闭后恢复原设置与应用排除。连接就绪后在有限时间内重试同一代理加密 DNS 路径，冷启动尚未完成时不立即判定失败。网络或授权失败会恢复；恢复记录持久化，后台服务冷启动先恢复未完成的配置事务。图标通知测试与拦截通知使用相同系统通知通道。

Android 不沿用桌面系统代理/提权服务，使用系统 VPN 授权与前台服务管理路由。订阅不能自行开启 TUN 或指定文件描述符；控制器固定在本机并使用随机密钥。关闭 IPv6 时不放行 VPN 外的 IPv6。VPN 必须保留内核 DNS 服务；关闭 DNS 覆盖会保留订阅上游与解析策略，缺少上游时才补共享默认值。

网络设置提供独立 Fake-IP 开关。关闭 DNS 覆盖会同时关闭 Fake-IP；重新开启 DNS 覆盖后，可用该开关手动恢复，设置重启后保留。Claude 保护期间显示临时强制开启状态并锁定开关，关闭保护后恢复用户偏好。

VPN 重建先建立新接口，等待系统网络回调确认新接口、地址与默认路由就绪后，再交接内核收包栈并报告连接完成，最后释放旧接口；授权仍有效时，新接口创建失败会保留原连接。

A÷检测支持网络、DNS、WebRTC 和自动浏览器采样，可在检测选项中保存/清除可选的 ipapi.is 密钥，检测中可停止，保留上次完成的报告。离开检测页会取消当前采样并忽略迟到结果；开启 Claude 保护成功后自动检测，关闭或启动恢复时不自动打开浏览器。Android 不扫描 Claude Desktop、CLI 或桌面项目文件，也不提供桌面终端命令。保护线程停止或拦截失败等明确错误仍进入风险；控制接口实际恢复后清除暂时错误，保留历史异常记录。

应用启动及每六小时自动检查更新，也可手动检查；默认仅正式版，可开启“包含预发布版本”，安装预发布版前需阅读风险提示并等待 3 秒确认，确认页以红字提醒无法保证在保留数据的情况下恢复正式版。关闭此选项时一律以线上最新正式版为目标，包括本地为 `0.999.0` 等更高版本的情况；开启后仅提供比本地更高的正式或预发布版，见[更新通道](../../docs/workflows/versions.md#版本与更新通道)。Android 不允许覆盖安装较低 versionCode 的旧包，此时需先备份配置，再由用户手动卸载重装；应用不会自动卸载或删除数据。按当前架构下载并验证 SHA256、包名、签名、版本与架构后交给 Android 安装界面确认，首次安装需允许应用安装更新。APK 保存在应用缓存，升级完成后清理。前台服务支持系统回收后的恢复，可在系统 VPN 设置中启用 Always-on VPN；厂商额外的电池限制可通过“电池与后台运行”调整，强行停止应用仍会中断连接。

## 本地构建

本地构建与 GitHub Actions 均可使用，不要求先提交代码。AI agent 在 1 核 CPU、约 1 GiB 内存等资源受限环境中不得执行以下构建，应遵循[资源受限规则](../../AGENTS.md#资源受限环境)，只做静态检查或最小 Slint 预览并清理工作区。资源充足时，当前 `scripts/android.py` 的 NDK 路径按 Linux x64 主机配置；准备 Rust、Go（版本满足 `core/go.mod`）、Java 17、兼容当前 Android Gradle Plugin 的 Gradle，以及 Android SDK 36、Build Tools 36.0.0、NDK 28.2.13676358。设置 `ANDROID_HOME`、`ANDROID_NDK_ROOT`，将 Go 的 bin 目录加入 PATH，并为目标 ABI 安装 Rust target：

```sh
rustup target add aarch64-linux-android
python3 scripts/android.py build --arch arm64
```

APK 输出到 `apps/android/app/build/outputs/apk/release/`；其他架构使用 `armv7` / `x64` 并安装相应 Rust target。本地构建可通过 `ANDROID_KEYSTORE_PATH`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` 指定自己的 PKCS12 签名；未签名的 Release APK 不能直接安装。密钥与密码不应提交到仓库。

## CI 与打包

使用 CI / Publish Release 时无需在本地安装 Android 工具链。

CI 在三个 ABI 编译、执行 Gradle lint 并校验 APK 架构与 64 位 ELF 的 16 KiB 对齐；x86_64 模拟器还验证正式界面、字号、横竖屏、授权/取消、后台、磁贴及真实代理/TUN。工具链、依赖快照、完整测试范围和 `android-report-*` 日志见[CI 说明](../../docs/workflows/ci.md)。ARM 架构仍需真机验收。
成功 CI 上传 `build-android-{arm64,armv7,x64}`。Publish Release 必须复用对应提交的产物，`check_only` 生成测试 APK 与 SHA256，不合并、不发布；x86_64 包在最终签名后再次运行代理/TUN 测试。APK 按架构提供，现代手机和平板通常选择 ARM64。

开发 CI 使用固定签名：`ANDROID_DEV_KEYSTORE`（base64 PKCS12）与 `ANDROID_DEV_KEYSTORE_PASSWORD`，别名 `cor-android-dev`。这些值只存 Actions Secrets；私钥不进入仓库或附件。无密钥的外部 PR 使用临时测试签名。正式发布另需 `ANDROID_KEYSTORE`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS` 与 `ANDROID_KEY_PASSWORD`，签名不同时不能直接覆盖已有 APK。

Slint 界面可用 `python3 scripts/preview-slint.py --mobile --platform android --check` 静态检查；预览固定数据不代替真实 VPN 测试。

## 连接超时排查

`[TCP] dial <节点或 DIRECT> ... i/o timeout` 表示该次出站连接未在期限内建立；`DIRECT` 是路由结果，不能仅凭超时认定 VPN 回环或真实 IP 泄漏。应保留完整脱敏行，区分业务目标与错误末尾实际拨号的节点地址、端口及 IPv4/IPv6。代理节点和 DIRECT 都超时时，分别检查目标可达性、底层网络、节点端口、DNS 结果与 VPN 套接字绕过路径，不能只据一条日志关闭 IPv6 或改成直连。当前 Go 出站钩子同时覆盖代理和 DIRECT，VPN 宿主在建立接口前注册 `protect(fd)`；真实网络结论仍需设备日志或隔离环境验证。
