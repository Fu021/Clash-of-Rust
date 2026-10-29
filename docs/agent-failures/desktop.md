# 桌面系统集成

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 系统代理状态与安全恢复

- **症状与根因：** 系统代理实际开启，GUI/托盘却显示关闭。整表精确比较把 macOS 非托管属性、网络服务增删，以及 Windows/GNOME/KDE 的绕过项变化当作失去代理所有权；退出时还可能直接丢弃恢复记录，留下本机代理。界面重建复用后台快照，本身不负责改写系统代理。`cor proxy` / `status` 曾直接输出持久偏好，实际代理变化后也可能与 GUI 不同。
- **修复：** 状态只比较托管路由字段，macOS 忽略新增或已删除服务，至少一个原服务仍存在才可判为持有。缺失/布尔/数值形式的关闭标志按同一语义比较；地址、端口或开关被改仍视为外部修改。macOS 只恢复仍匹配的服务，在偏好锁内读取最新字典并合并托管键，保留新服务、例外列表、PAC 地址等非托管属性；各桌面平台保留后来改动的绕过项，旧 GNOME journal 不接管 SOCKS。
- **助手升级：** macOS 网络助手协议为 `8`，新操作要求新助手，避免 mihomo 文件未变时沿用旧 Rust 助手的 DNS 和恢复逻辑；仅允许协议 `2..7` 执行已有代理/DNS 清理，关闭 DNS 的请求省略新端口字段，兼容旧助手拒绝未知字段的协议。修改助手行为后须更新协议，让已安装版本加载修复；清理不安装服务或弹授权，正常退出、所有者异常退出、助手租约交接及启动恢复共用托管字段语义。
- **防回归与证据：** 纯逻辑回归覆盖辅助字段、服务增删、实际路由冲突及合并恢复；窗口重建投影回归覆盖后台实时状态与持久偏好不一致。GUI 与 CLI 从同一快照判定代理模式，原生集成验证实际关闭但持久偏好仍为系统代理时，两端一致显示关闭，随后可重新启用。现有 macOS 代理/TUN 集成场景实际修改例外列表，核对后台状态、关闭恢复及助手租约清理。物理网络切换、休眠及用户现场仍需实机核对；同提交 CI/打包结果记录在[最近复盘](reviews.md)与任务汇报。

## 断网启动与联网恢复

离线启动后代理持续 `unreach`、TUN 下 tailnet 不可达而重启恢复时，按[桌面联网恢复](network-recovery.md)排查默认路由、内核出站与 DNS；不要直接归因于 Fake-IP。实现、验证与设备验收边界均在该主题维护。

## 7. Linux 代理、TUN、DNS 与通知

**用户目录与无显示环境：** `sudo cor` 应在解析用户配置前降权并复用原用户后台；直接 root 保留独立目录。跨用户总览只读索引，不启动后台。polkit 的 SSH/非活动会话仍需 `auth_admin`，只授权固定安装助手；不能让用户 YAML 在 root 后台运行。回归需覆盖两用户同名订阅、环境白名单与管理员凭据隔离、sudo/指定用户操作、stdin/原样参数、文件 UID、后台复用、前台信号及退出恢复，见[用户与授权](../cli/linux-users.md)。Wayland compositor 错误来自桌面入口，服务器使用 `cor`，不通过为 root 配置显示服务器规避。

**安装后 GUI 验证：** `sudo -u` 不创建 logind 登录会话，图形 polkit 代理会因 `No session for pid` 注册失败。测试使用隔离的临时 PAM 登录会话，验证进程归属、真实授权的取消与接受，并隔离用户和 D-Bus 配置；不注入批准规则。模拟环境中代理进程存活不等于已注册，仅对明确的 `No authentication agent found` 有限重试授权请求。GNOME 42 的 `--unsafe-mode` 是隐藏选项，不能仅凭 `--help` 判断；旧版需启用 `development-tools`，窗口结果也只能编码一次。GNOME 3.36 的 `get_pid()` 对 Wayland 不返回客户端 PID，隔离会话只能在恰好一个原生 Wayland 窗口和一个渲染进程时回退匹配，并以窗口稳定序号关闭；新版本仍核对客户端 PID。[打包 40827251291](https://github.com/Fu021/Clash-of-Rust/actions/runs/40827251291) 的 20.04 x64 窗口识别、ARM64 授权代理注册失败；修复后 `9f92c5f` 的[打包 40845419892](https://github.com/Fu021/Clash-of-Rust/actions/runs/40845419892)通过两架构验证。

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

助手、应用身份、端口复用和安装后验证见 [macOS 生命周期](macos-lifecycle.md)；DNS 与 Tailscale 共存见 [macOS DNS](macos-dns.md)。

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
- Defender 同时扫描最终安装包与已验证 CI payload 中的全部 EXE/DLL，记录引擎、平台、检测库版本及逐文件哈希；命中威胁、扫描失败或文件变化时拒绝上传。状态查询超时可有限重试，不能跳过版本证据或文件扫描；ARM64 曾在[打包 40805841630](https://github.com/Fu021/Clash-of-Rust/actions/runs/40805841630)超时。只扫描压缩安装器不足以核对用户报告的主程序告警，历史成功也不能证明当前文件为误报；排查与提交见 [Defender 告警](../workflows/release.md#windows-defender-告警)。
- 清理只删除本次 fixture，不触碰用户已有安装或网络配置。
