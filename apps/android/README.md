# Android 宿主

当前版本为 `0.6.3`，最低 Android 8.0（API 26），支持 ARM64、ARMv7 和 x86_64。iOS 仍单独保留界面与宿主占位，不参与此次实现。

## 结构

- `native`：Slint NativeActivity 入口、JNI、异步任务和界面模型；正式入口为 `crates/client-ui/ui/mobile/android/window.slint`。
- `app`：Android 生命周期、文件选择、通知、应用分流及前台 `VpnService`。关闭界面不等于断开 VPN。
- `core`：按 CI 依赖快照锁定的 mihomo Go 模块及 gomobile 桥接；使用上游 `cmfa,with_gvisor` 构建标签，路由与应用排除交给 `VpnService`，出站套接字经 `protect` 排除，避免进入自身 TUN。
- `crates/mobile-ffi`：不依赖 Slint、JNI 或 Android SDK 的共享会话、配置、订阅、规则和控制器操作。原生能力由 `NativeHost` 注入，iOS 可以另接宿主。

## 当前能力

实际接入订阅添加、文件导入、编辑、更新与切换；策略组、节点选择、排序、单组与全部测速；VPN 连接与断开、快捷设置磁贴、应用排除；连接速率、排序、详情与关闭；规则草稿、快捷规则与应用；日志筛选、复制与导出；网络诊断、IP 分类检测和浏览器 A÷检测；主题、强调色、DNS/IPv6/UDP 与端口设置。字号跟随 Android 系统设置，使用系统 SP 换算（含 Android 14 的非线性缩放），不使用桌面端保存的字号；卡片间距与内容高度适应字号，触摸目标保留最小尺寸。

下拉系统快捷设置并编辑磁贴，可添加 Clash of Rust 开关；首次需要 VPN 授权时会进入应用，后续直接连接/断开，包括应用进程已退出的情况；磁贴实时更新连接状态。

手动测速使用自动策略组自己的测试地址，完成后让内核按该组配置重新选优；测速结果直接更新列表。Claude 保护开启时禁止切换节点，并在运行配置中冻结自动组，关闭后恢复原订阅的自动策略。

Claude 保护先取得系统 VPN 授权，再应用保护配置；等待授权或取消时保留原网络和 DNS，已取消操作不会因迟到的授权而连接。启用时固定当前具体节点，临时启用 VPN、IPv6、UDP、代理加密 DNS 与实时拦截，并覆盖应用排除；关闭后恢复原设置与应用排除。连接就绪后在有限时间内重试同一代理加密 DNS 路径，冷启动尚未完成时不立即判定失败。网络或授权失败会恢复；恢复记录持久化，后台服务冷启动先恢复未完成的配置事务。图标通知测试与拦截通知使用相同系统通知通道。

Android 不沿用桌面系统代理/提权服务，使用系统 VPN 授权与前台服务管理路由。订阅不能自行开启 TUN 或指定文件描述符；控制器固定在本机并使用随机密钥。关闭 IPv6 时不放行 VPN 外的 IPv6。VPN 必须保留内核 DNS 服务；关闭 DNS 覆盖会保留订阅上游与解析策略，缺少上游时才补共享默认值。

VPN 重建先建立新接口，等待系统网络回调确认新接口、地址与默认路由就绪后，再交接内核收包栈并报告连接完成，最后释放旧接口；授权仍有效时，新接口创建失败会保留原连接。

A÷检测目前支持网络、DNS 和自动浏览器采样；开启 Claude 保护成功后自动检测，关闭或启动恢复时不自动打开浏览器。桌面项目扫描不适用于 Android。应用启动及每六小时自动检查更新，也可手动检查；默认仅正式版，可开启“包含预发布版本”，安装预发布版前需阅读风险提示并等待 3 秒确认。关闭此选项可检查最新正式版；Android 不允许覆盖安装较低 versionCode 的旧包，此时需先备份配置，再由用户手动卸载重装；应用不会自动卸载或删除数据。按当前架构下载并验证 SHA256、包名、签名、版本与架构后交给 Android 安装界面确认，首次安装需允许应用安装更新。APK 保存在应用缓存，升级完成后清理。前台服务支持系统回收后的恢复，可在系统 VPN 设置中启用 Always-on VPN；厂商额外的电池限制可通过“电池与后台运行”调整，强行停止应用仍会中断连接。

## 本地构建

本地构建与 GitHub Actions 均可使用，不要求先提交代码。当前 `scripts/android.py` 的 NDK 路径按 Linux x64 主机配置；准备 Rust、Go（版本满足 `core/go.mod`）、Java 17、兼容当前 Android Gradle Plugin 的 Gradle，以及 Android SDK 36、Build Tools 36.0.0、NDK 28.2.13676358。设置 `ANDROID_HOME`、`ANDROID_NDK_ROOT`，将 Go 的 bin 目录加入 PATH，并为目标 ABI 安装 Rust target：

```sh
rustup target add aarch64-linux-android
python3 scripts/android.py build --arch arm64
```

APK 输出到 `apps/android/app/build/outputs/apk/release/`；其他架构使用 `armv7` / `x64` 并安装相应 Rust target。本地构建可通过 `ANDROID_KEYSTORE_PATH`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS`、`ANDROID_KEY_PASSWORD` 指定自己的 PKCS12 签名；未签名的 Release APK 不能直接安装。密钥与密码不应提交到仓库。

## CI 与打包

使用 CI / Publish Release 时无需在本地安装 Android 工具链。

CI 使用 SDK 36、NDK r28 与 Java 17；Rust/Go 源码依赖与兼容的 Gradle 工具每次查询更新，网络失败使用兼容缓存。本轮锁文件、实际工具版本和校验值随构建记录，打包复用这份已测试快照。三种 ABI 分别编译，验证 APK 架构和 64 位 ELF 的 16 KiB 对齐。x86_64 Android 模拟器实际启动 Slint 宿主，验证界面内核状态、默认策略组、订阅、标签导航、设置触摸滚动、系统小字号与无障碍大字号同步、后台返回与横竖屏恢复，实际操作首次 VPN 授权对话框，覆盖拒绝、取消后的迟到授权、通过和已授权路径，再验证本机代理 HTTP 转发、系统 TUN 接管、DNS、界面进入后台时的 TUN 转发、VPN 重建、快捷磁贴冷启动静默连接/断开、磁贴实际显示状态及路由恢复。结果与 logcat 在 `android-report-*` 附件。

成功 CI 上传 `build-android-{arm64,armv7,x64}`。Publish Release 必须复用对应提交的产物，`check_only` 生成测试 APK 与 SHA256，不合并、不发布；x86_64 包在最终签名后再次运行代理/TUN 测试。APK 按架构提供，现代手机和平板通常选择 ARM64。

开发 CI 使用固定签名：`ANDROID_DEV_KEYSTORE`（base64 PKCS12）与 `ANDROID_DEV_KEYSTORE_PASSWORD`，别名 `cor-android-dev`。这些值只存 Actions Secrets；私钥不进入仓库或附件。无密钥的外部 PR 使用临时测试签名。正式发布另需 `ANDROID_KEYSTORE`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS` 与 `ANDROID_KEY_PASSWORD`，签名不同时不能直接覆盖已有 APK。

Slint 界面可用 `python3 scripts/preview-slint.py --mobile --platform android --check` 静态检查；预览固定数据不代替真实 VPN 测试。
