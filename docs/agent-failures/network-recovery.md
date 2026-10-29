# 桌面联网恢复

[返回索引](README.md)。GUI 与 `cor` 使用同一后台恢复入口，Android VPN 的网络生命周期由移动宿主管理；IPv6 DNS 配置由各端共享。

## 断网启动与联网恢复

- **症状与定位：** Windows 无网络时启动，联网后代理目标持续 `unreach`，TUN 下 tailnet 也不可达，重启客户端恢复；订阅未指定接口。这与 Fake-IP 排除不同：桌面后台原先只有控制器健康检查，缺少独立的联网恢复入口。mihomo `v1.19.33` 的默认接口监控依附 TUN；重新应用配置会刷新接口缓存与 DNS 连接。仅 `/version` 可用不能证明出站已恢复。
- **处理：** Windows IP Helper、Linux `/proc` 与 macOS SystemConfiguration 只读默认路由，后台每 5 秒采样；在线变化须连续两次一致才刷新，初始在线不刷新，无路由时等待。忽略自身 TUN 分裂路由，读取失败保留观察状态；启动重新建立基线，旧进程采样丢弃，失败按 60 秒至 5 分钟退避。
- **状态边界：** 重载当前运行 YAML，使用实际模式与 TUN 开关，保留手动节点选择；不重新绑定入口、不改系统代理或保护 DNS、不清 Fake-IP 映射。相同 TUN 配置由内核复用现有监听器并执行 `OnReload`，不重新申请授权。成功后清 DNS 缓存和旧连接，订阅、设置、运行文件和恢复记录不变；恢复不依赖 GUI 窗口。
- **验证入口：** [路由采样与状态机](../../crates/client-runtime/src/control/network.rs)、[平台读取](../../crates/client-runtime/src/platform/network/mod.rs)、[恢复入口](../../crates/client-runtime/src/engine/recovery.rs)、[控制 API](../../crates/client-core/src/controller/mod.rs)与[真实内核集成](../../apps/desktop/tests/core_integration.rs)。覆盖离线恢复、路由抖动、退避、重启失效及代理/TUN 刷新后的状态和磁盘保留。
- **证据与边界：** 上游行为核对 [ApplyConfig / updateGeneral](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/hub/executor/executor.go) 与 [ReCreateTun](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/listener/listener.go)。同提交 CI 和安装包结果见[最近复盘](reviews.md)及任务汇报；不能替代 Windows 开机断网后联网、真实 tailnet 与休眠恢复的设备验收。合并及公开发布须有用户授权，最终版本仍需同提交 CI 与安装包验证。

## IPv6 节点域名解析

[不发布打包 40883823532](https://github.com/Fu021/Clash-of-Rust/actions/runs/40883823532) 的 Ubuntu 20.04 ARM64 隔离测试在保护 DNS 返回 SERVFAIL 时记录 IPv6 节点 `no ip address`。上游 [mihomo 解析器](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/dns/resolver.go)在 A 查询结束后默认只等待 AAAA 100 毫秒；慢 AAAA 是与日志吻合的可能原因，并非已证明所有现场超时都由此造成。

共享运行配置在订阅未指定 `dns.ipv6-timeout` 时使用 1000 毫秒，保留显式值和原订阅；正常及保护模式均适用，可能延长慢 AAAA 查询的等待。Linux 隔离上游提供快速空 A 与延迟 350 毫秒的 AAAA，检查实际节点解析和保护 DNS，仍需同提交 CI/安装后结果及各端真实网络验收。
