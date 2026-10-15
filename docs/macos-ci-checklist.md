# macOS 开发与验收

最低系统目标为 macOS 14；CI 原生覆盖 Intel (`macos-15-intel`) 和 Apple Silicon (`macos-latest`)，分别输出 x64 和 arm64 安装包。

## 自动检查

CI 原生覆盖 Intel 与 Apple Silicon，运行格式检查、Clippy、编译和 Rust 常规测试。基础检查使用真实 mihomo、launchd 网络助手与系统网络设置，验证系统代理写入、HTTP 转发、TUN 接管及关闭清理。

Publish Release 复用 CI 产物，核对 DMG/App 版本、架构、资源与签名，启动安装后的真实 GUI/内核，并运行基础代理/TUN 检查。开发分支可用 `check_only=true` 打包而不发布；授权弹窗和首次下载后的 Gatekeeper 操作需要人工验收。

每种架构上传报告、逐项耗时、完整日志、系统/网络状态与崩溃信息。界面外观通过本地 Slint Viewer 直接预览，不保留截图工作流。

## 实现与安装

- 托盘创建、更新及销毁在 AppKit 主线程完成，确认桌面事件循环已启动后初始化，失败按时限重试；菜单栏仅显示系统模板图标。图标的原生状态窗口丢失时自动重新注册，重试有间隔限制。关闭窗口后可用 Dock 或菜单栏重新显示，菜单栏图标缺失时仍保留 Dock 恢复入口。Windows/Linux 保持现有线程实现。
- 系统代理与 TUN 共用按用户安装的 root 网络助手，首次使用时授权安装，后续启动、模式切换与退出通过本地 socket 操作，不重复创建界面授权会话；退出恢复不会安装或重新授权助手。助手通过内核提供的用户身份和进程身份校验请求，记录原系统代理，客户端异常退出或助手停止时仅恢复自己仍持有的设置；开机启动仅在已保存的系统代理模式、端口和当前设置均属于本客户端时沿用恢复记录。系统代理使用 SystemConfiguration 原生接口，保留各网络服务原先的 PAC、例外列表及代理设置；恢复前检查当前设置仍由客户端持有。
- 开机启动开关只调整用户的 plist，避免卸载正在运行的客户端 LaunchAgent。开机启动采用当前用户的 `~/Library/LaunchAgents/org.clashofrust.desktop.plist`，以普通用户后台启动。启用后下次登录生效。
- TUN 助手单独编译，不依赖桌面界面库。系统代理或 TUN 首次使用时统一请求管理员授权，助手由 launchd 管理；GUI 不提权。会话使用私有 Unix socket、用户身份与随机令牌验证，固定执行已验证并冻结的 mihomo。桥接断开后停止内核，未使用的内核会话在五分钟后退出；小型代理/TUN 控制助手保留，由 launchd 在重启后恢复，不设置 setuid。后台登录只复用已有助手，缺失或需要修复时提示用户手动操作，不弹出授权。助手内核快照发生变化时需要重新授权安装。
- DMG 中的 `.app` 拖入 Applications 即可安装。数据目录使用 macOS 用户 Application Support，不写入应用包。
- 当前安装包使用 ad-hoc 签名，可验证包完整性；它不提供 Apple Developer ID 信任或公证。尚未配置 Developer ID 签名和公证凭据，首次下载后的 Gatekeeper 提示见 [Apple 说明](https://support.apple.com/zh-cn/102445)。

## 仍需交互式 Mac 验收

1. 首次安装的 Gatekeeper 提示、网络助手的一次安装授权，以及取消授权后保持原模式；系统代理、TUN 切换应复用助手。
2. 托盘菜单、关闭窗口后后台运行、重新显示窗口；切换模式后菜单和图标一致。
3. 开机启动在真正注销/登录后生效；自动后台启动不会连续弹出权限提示。助手需要修复时，手动切换代理或关闭 Claude 保护可以完成授权；取消关闭保护的授权后，原设置与恢复记录应保持完整。
4. TUN 开启时实际访问网站、DNS 解析及断网/恢复；关闭或退出后恢复正常网络。
5. 自动更新成功、取消授权、空间不足、旧版恢复；更新后重新启动和版本正确。
6. 浅色/深色、Retina 缩放、多显示器、窗口尺寸恢复及休眠唤醒。

Actions 可以验证真实系统 API 和基本桌面运行；授权弹窗操作、真实重启、物理显示器和睡眠行为需要远程桌面或实体 Mac 补充确认。

## 移除网络助手

卸载应用前关闭代理并退出客户端；网络助手按安装时的用户 ID 保存。可在终端移除当前用户的助手（这一步需要管理员权限）：

```sh
service_label="org.clashofrust.network.$(id -u)"
sudo launchctl bootout "system/$service_label"
sudo rm -rf "/Library/PrivilegedHelperTools/$service_label" "/Library/LaunchDaemons/$service_label.plist"
```

助手停止前尝试恢复自己持有的系统代理并终止其内核会话，其他客户端后续写入的代理设置保持不动。基础检查结束后移除本次安装的助手，并拒绝覆盖 runner 上预先存在的安装。
