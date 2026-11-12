# Linux IPv6 与 TUN 覆盖

[文档索引](README.md) · [诊断字段](diagnostics.md#桌面-tun-启动与覆盖) · [网络恢复](agent-failures/network-recovery.md)

IPv6 默认开启是用户偏好，不保证本机当前具有 IPv6 出口。普通 TUN 和 Claude 保护都不能把“本机没有可用 IPv6 地址”误报为绕行，也不能把读取失败当成没有 IPv6。

## 判定

Linux 只读 `/proc/net/if_inet6` 和接口状态，区分 `available`、`unavailable`、`unknown`。回环、链路本地、组播、未指定地址、自身及其他 TUN 地址不能证明上联具有 IPv6；tentative、DAD 失败及已关闭接口不计为可用。ULA 保留为可用地址，避免忽略使用 ULA/NAT66 的网络；deprecated 地址仍可能发送现有连接，不能直接当成不可用。

该结果只说明本机地址条件，不证明公网、节点或某个域名可达。内核文档明确指出，读取 `conf/all/disable_ipv6` 不能判断各接口是否真的禁用 IPv6，因此不以这个值代替地址观察，见[内核说明](https://docs.kernel.org/networking/ip-sysctl.html#conf-all-disable-ipv6-boolean)。

| 本机条件 | 普通 TUN | Claude 保护 |
| --- | --- | --- |
| 确认没有可用 IPv6 上联地址 | 保留可用的 IPv4，不报 IPv6 绕行 | 相同处理，不因缺少 IPv6 拒绝保护 |
| IPv6 开启且本机有可用地址 | 核对 IPv6 路由，确认绕行须报告 | 即使原偏好关闭 IPv6，也核对保护要求的 IPv6 接管 |
| 用户关闭 IPv6，未开启保护 | 不要求 IPv6 接管；原始证据保留 | 不适用，保护使用自己的覆盖要求 |
| 地址读取失败、超时或格式异常 | 记未知，不借此放过已观察到的绕行 | 相同处理，不能据此认定没有 IPv6 |
| IPv4 确认绕行 | 始终报告，启动时回滚 | 始终拒绝不完整的保护提交 |

启动同时读取内核实际配置和路由；运行中警告结合当前设置与同次上联地址观察。内核没有创建 IPv6 TUN 地址也不能独自证明本机没有 IPv6，必须有上联地址观察才能免除检查；无地址证据的旧报告保持未知，不能从它反推历史网络状态。`auto-redirect` 与当前 UID/mark 的采样边界见[跨用户路由检查](network-paths.md#跨用户路由检查)。

## 网络变化

GUI/CLI 的唯一后台所有者每 5 秒只读采样，除默认路由外还比较非 TUN IPv6 地址和状态；相同网关下的地址出现、消失或更换也会重新评估。连续两次稳定才处理，旧内核进程的采样丢弃，失败退避重试。地址读取失败保留前次地址观察，不把它伪装成 IPv6 消失，默认路由变化仍可单独处理。

普通模式重新加载当前运行配置、刷新出站与 DNS，再核对实际 TUN 覆盖，保留订阅、节点、代理方式和用户的 IPv6 偏好。mihomo 在解析配置时按系统地址决定是否创建 IPv6 TUN；配置相同则复用监听器，地址族配置发生变化时可能重建 TUN，不能承诺零连接中断，见上游 [parseIPV6](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/config/config.go)、[verifyIP6](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/config/utils.go) 与 [ReCreateTun](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/listener/listener.go)。

Claude 保护期间不自动改写锁定的运行配置；网络变化后重新检查覆盖。IPv6 从无到有且确认没有接管时保留错误与诊断，不静默显示完整接管。IPv6 消失后，新的地址观察应清除对应的日常绕行提示；历史错误仍保留供排查。没有可用上联时，内核保留已有 IPv6 TUN 地址也不构成错误。

## 回归与验收边界

纯逻辑回归覆盖地址过滤、DAD、deprecated、ULA、关闭接口、无 IPv6、双栈、IPv6 开关、保护、读取失败、当前 UID 与原始路由证据，以及无 → 有 → 更换 → 无的稳定观察和重试。

[`tun_ipv6_lifecycle.py`](../scripts/tun_ipv6_lifecycle.py)由现有 Linux 隔离测试调用：默认 IPv6 开启但移除上联 IPv6 时启动、重启，再开启 Claude 保护；核对保护期间拒绝停止内核且状态不变。保护保持启用时分别从有、无 IPv6 开始往返切换，核对覆盖告警、IPv4 请求和锁定配置不变。普通模式恢复 IPv6 后检查自动刷新、真实 IPv4/IPv6 HTTP 和 TUN 路由，再验证 IPv6 消失、恢复及 TUN 重连，核对设置未被自动改写。

测试只在可丢弃网络/挂载命名空间执行，不改变开发主机网络。隔离后在独立挂载点创建只读 sysfs，同时绑定 `class/net` 与其符号链接目标 `devices/virtual/net`，保留 cgroup 等宿主子挂载供 systemd-resolved 启动；再核对上联接口索引，保证 `/proc`、网络类视图与路由来自同一命名空间。只绑定 `class/net` 会让隔离网卡的链接仍指向宿主设备树，不能用于判定隔离接口，见[内核说明](https://docs.kernel.org/networking/sysfs-tagging.html)。

新增 Rust 与真实网络回归尚待同提交 Actions；静态检查不等于已通过上述验收。Wi-Fi/有线切换、真实 RA/DHCPv6、休眠、IPv6-only/NAT64、厂商网络和长期保护仍需设备证据。
