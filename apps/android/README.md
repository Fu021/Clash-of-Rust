# Android 宿主

开发版为 `0.5.12-dev.1`，最低 Android 8.0（API 26），支持 ARM64、ARMv7 和 x86_64。iOS 仍单独保留界面与宿主占位，不参与此次实现。

## 结构

- `native`：Slint NativeActivity 入口、JNI、异步任务和界面模型；正式入口为 `crates/client-ui/ui/mobile/android/window.slint`。
- `app`：Android 生命周期、文件选择、通知、应用分流及前台 `VpnService`。关闭界面不等于断开 VPN。
- `core`：固定版本的 mihomo Go 模块及 gomobile 桥接；使用上游 `cmfa,with_gvisor` 构建标签，路由与应用排除交给 `VpnService`，出站套接字经 `protect` 排除，避免进入自身 TUN。
- `crates/mobile-ffi`：不依赖 Slint、JNI 或 Android SDK 的共享会话、配置、订阅、规则和控制器操作。原生能力由 `NativeHost` 注入，iOS 可以另接宿主。

## 当前能力

实际接入订阅添加、文件导入、编辑、更新与切换；策略组、节点选择、排序及测速；VPN 连接与断开、应用排除；连接速率、排序、详情与关闭；规则草稿、快捷规则与应用；日志筛选、复制与导出；网络诊断、IP 分类检测和浏览器 A÷检测；主题、字号、强调色、DNS/IPv6/UDP 与端口设置。

Android 不沿用桌面系统代理/提权服务，使用系统 VPN 授权与前台服务管理路由。订阅不能自行开启 TUN 或指定文件描述符；控制器固定在本机并使用随机密钥。关闭 IPv6 时不放行 VPN 外的 IPv6。

A÷检测目前支持网络、DNS 和自动浏览器采样；桌面项目扫描不适用于 Android。Claude 一键保护暂不开放，需先完成移动端固定节点、状态恢复与拦截链路验证。应用内自动安装更新、开机连接与 Always-on VPN 尚未实现；检查更新打开 Releases，安装测试 APK 前请自行确认文件来源。

## CI 与打包

所有编译、测试和构建在现有 CI / Publish Release 中执行，本地无需 Android SDK 或 Rust 构建。

CI 使用 SDK 36、NDK r28、Java 17、Gradle 8.13、Go 1.26 与 Slint 1.18.1；三种 ABI 分别编译，验证 APK 架构和 64 位 ELF 的 16 KiB 对齐。x86_64 Android 模拟器实际启动 Slint 宿主，验证本机代理 HTTP 转发、系统 TUN 接管、DNS、连续两次连接/断开及路由恢复。结果与 logcat 在 `android-report-*` 附件。

成功 CI 上传 `build-android-{arm64,armv7,x64}`。Publish Release 必须复用对应提交的产物，`check_only` 生成测试 APK 与 SHA256，不合并、不发布；x86_64 包在最终签名后再次运行代理/TUN 测试。APK 按架构提供，现代手机和平板通常选择 ARM64。

开发 CI 使用固定签名：`ANDROID_DEV_KEYSTORE`（base64 PKCS12）与 `ANDROID_DEV_KEYSTORE_PASSWORD`，别名 `cor-android-dev`。这些值只存 Actions Secrets；私钥不进入仓库或附件。无密钥的外部 PR 使用临时测试签名。正式发布另需 `ANDROID_KEYSTORE`、`ANDROID_KEYSTORE_PASSWORD`、`ANDROID_KEY_ALIAS` 与 `ANDROID_KEY_PASSWORD`，签名不同时不能直接覆盖已有 APK。

Slint 界面可用 `python3 scripts/preview-slint.py --mobile --platform android --check` 静态检查；预览固定数据不代替真实 VPN 测试。
