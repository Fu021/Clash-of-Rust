# 更新、测速与跨用户 TUN 链路

[文档索引](README.md) · [内部诊断字段](diagnostics.md) · [下载回退与安装恢复](agent-failures/downloads.md)

## 各平台入口

| 操作 | Windows / macOS / Linux | Android |
| --- | --- | --- |
| 软件更新检查 | [运行时发现](../crates/client-runtime/src/update/discovery.rs)查询 GitHub API；正式版还可核对网页重定向 | [共享核心](../crates/client-core/src/update.rs)查询 GitHub API，不读取桌面代理设置 |
| 安装包下载 | [桌面下载器](../crates/client-runtime/src/update/mod.rs)，SHA256/长度校验后交平台安装器 | 共享核心下载器，校验后交系统安装页面 |
| Geo 更新 | [运行时入口](../crates/client-runtime/src/engine/assets.rs)读取桌面备用代理，调用共享核心 | 调用同一 [Geo 下载器](../crates/client-core/src/geodata/mod.rs) |
| 节点测速 | 本机 [控制 API](../crates/client-core/src/controller/mod.rs)让 mihomo 经指定节点探测 | 同一控制 API；内核出站另经 [Android socket 保护器](../apps/android/core/mobile.go) |

软件与 Geo 下载都使用[相同 HTTP/TLS 构建器及显式路线](../crates/client-core/src/subscription/routes.rs)、随包公共根证书、256 KiB 缓冲写盘和共享低速看门狗。未发现 Windows 专属限速；桌面安装包连接超时为 10 秒，Android 软件更新为 5 秒，Geo 各端为 10 秒。更新检查的请求预算和回退发现方式也不同，但 Windows/macOS/Linux 的安装包传输参数一致，不能用 Android 的超时差别解释桌面速度差异。

路线依次为内核代理、不同的系统代理、独立内核 DIRECT、系统网络，相同代理去重。桌面备用代理分别读取 Windows 当前用户 Internet Settings、macOS SystemConfiguration、Linux GNOME/KDE；不把环境代理隐式叠加进已经选择的路线。受管 mixed 入口都是本机 HTTP 代理，HTTPS 经 CONNECT；DIRECT 入口使用 `socks5h` 传递原域名，由内核解析并直连。没有应用层代理不代表绕过系统 TUN。

Geo 原先没有读取桌面备用系统代理，元数据及每个文件也各自创建客户端；现已补齐备用路线并复用同批连接池。缓冲写盘减少小网络块触发的磁盘任务。相关源码回归和桌面基础代理/TUN 已通过，证据见[最近复盘](agent-failures/reviews.md)；没有真实跨平台同时间下载对照，不能据此宣称已解决 Windows 慢速。

Android VPN 由系统建立接口和应用范围，内核新 socket 通过 `protect(fd)` 绕过自身 VPN。普通 Rust 请求与内核出站不是同一个 socket；仅应用分流、系统 DNS 和 VPN 状态也可能改变前者的系统网络路线。保护器失败计数保留在[诊断](diagnostics.md#android-现场)，不以 VPN 图标或内核运行为成功依据。

## 测速与下载为何不能直接比较

单节点测速通常访问 `https://www.gstatic.com/generate_204`，自动策略组可能采用订阅自定义目标；控制客户端总超时 8 秒、传给内核的探测超时 5 秒。`unified-delay` 的复用连接测量与新连接、安装包下载速率是不同指标。控制请求不读取系统代理，测试由内核指定的节点执行，不能拿下载器的 DIRECT 回退解释一次节点延迟。

同一节点名称不足以证明实际出站一致。软件更新进度只有匹配到自身 TCP 地址、源端口和目标对应的控制器记录后才显示出站；匹配失败保持未知，不使用当前选中节点代替。每条下载路线在还有后续路线时，20 秒窗口持续低于 32 KiB/s 会切换，最后一路保留空闲与总超时；一次慢速尝试与后续成功可能用了不同路线。

安装包与 Geo 下载遇到超时、连接重置或响应体中断时，每条路线最多重建一次连接，再沿上述顺序回退；新连接交给自动策略组重新选择实际节点，不把下载绑定到最初选择的节点。等待响应头总共最多 30 秒，不因零碎响应无限延期。重试从头下载，丢弃本次部分文件并重新校验长度和 SHA256；持续低速直接换路线，摘要错误、磁盘权限或无效元数据不在同一路线重试。所有路线均失败时结束并保留错误，取消仍可立即终止；这些边界不能证明公网自动节点切换必然恢复。

比较需固定版本、内核、目标文件与节点，记录代理模式、下载路线/实际出站、DNS 地址族、重定向目标、首字节、总耗时和落盘耗时，再分别比较 mixed HTTP、SOCKS 与系统网络；不关闭证书校验。GitHub CDN、运营商、IPv6、MTU、安全软件及已建立连接仍是候选，未经同时间证据不归因为某个操作系统。

现有[桌面 CI 回环测速](workflows/ci.md#桌面代理回环测速)可区分 HTTP、CONNECT/TLS、SOCKS 和落盘代价，结果限于 runner 本机；不经过用户节点或公网 CDN，不能代替 Windows/macOS 实机复现。本轮未取得可用于定因的同提交跨平台结果。

## 跨用户路由检查

TUN 不按启动账户自动隔离。同一网络命名空间中，是否接管某 UID 取决于路由、UID/mark/接口等条件；[mihomo UID 限制](https://wiki.metacubex.one/config/inbound/tun/#include-uid)可改变范围。另一用户的 TUN 开着，既不能证明本用户已被接管，也不能证明本用户一定绕过。

[主机观察器](../crates/client-runtime/src/platform/host_network.rs)同时保留普通 IPv4/IPv6 查询和当前有效 UID 的 TCP/443 查询，后者用于普通覆盖检查。导出诊断另补 root 或 `SUDO_UID` 的对照及策略规则，不以另一用户身份发送请求。查询只读取路由，不修改 DNS、Tailscale、防火墙或任何用户的 TUN。

只用同 UID、TCP、443 的有效样本判断该样本；无法查询、元数据不匹配或权限不足保持未知，不借用 root 或普通样本冒充当前用户结论。旧报告没有 UID 样本时仍可读取普通路由。普通查询进 TUN、当前 UID 的 TCP 查询走物理网卡时记录 `tun_current_uid_route_bypass`，提示核对策略；不自动清除 UID 限制或重启外部实例。

启用 `auto-redirect` 时 TCP 可能经包过滤器标记/重定向，`ip route get` 看不到完整转发链；启动门禁保留原普通路由检查，不仅凭 UID TCP 样本回滚。`tun_configuration.auto_redirect` 用于辨别这类报告。任何路由样本都不等于真实 HTTPS 成功，不覆盖所有目标、源端口、带 mark/绑定网卡的 socket、旧连接或历史状态。长期 TLS 故障仍须结合同时段应用错误、DNS、控制响应与内核日志核对，见[网络恢复](agent-failures/network-recovery.md#长时间运行后的服务不可达)。
