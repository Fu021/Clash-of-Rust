# 桌面系统集成

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 7. Linux 代理、TUN、DNS 与通知

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

## 8. macOS 应用身份、launchd 与打包

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

## 9. Windows 普通用户、服务与安装器

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
