# 桌面联网恢复

[返回索引](README.md)。GUI 与 `cor` 使用同一后台恢复入口，Android VPN 的网络生命周期由移动宿主管理；IPv6 DNS 配置由各端共享。

## 断网启动与联网恢复

- **症状与定位：** Windows 无网络时启动，联网后代理目标持续 `unreach`，TUN 下 tailnet 也不可达，重启客户端恢复；订阅未指定接口。这与 Fake-IP 排除不同：桌面后台原先只有控制器健康检查，缺少独立的联网恢复入口。mihomo `v1.19.33` 的默认接口监控依附 TUN；重新应用配置会刷新接口缓存与 DNS 连接。仅 `/version` 可用不能证明出站已恢复。
- **处理：** Windows IP Helper、Linux `/proc` 与 macOS SystemConfiguration 只读默认路由，后台每 5 秒采样；在线变化须连续两次一致才刷新，初始在线不刷新，无路由时等待。忽略自身 TUN 分裂路由，读取失败保留观察状态；启动重新建立基线，旧进程采样丢弃，失败按 60 秒至 5 分钟退避。
- **状态边界：** 重载当前运行 YAML，使用实际模式与 TUN 开关，保留手动节点选择；不重新绑定入口、不改系统代理或保护 DNS、不清 Fake-IP 映射。相同 TUN 配置由内核复用现有监听器并执行 `OnReload`，不重新申请授权。成功后清 DNS 缓存和旧连接，订阅、设置、运行文件和恢复记录不变；恢复不依赖 GUI 窗口。
- **验证入口：** [路由采样与状态机](../../crates/client-runtime/src/control/network.rs)、[平台读取](../../crates/client-runtime/src/platform/network/mod.rs)、[恢复入口](../../crates/client-runtime/src/engine/recovery.rs)、[控制 API](../../crates/client-core/src/controller/mod.rs)与[真实内核集成](../../apps/desktop/tests/core_integration.rs)。覆盖离线恢复、路由抖动、退避、重启失效及代理/TUN 刷新后的状态和磁盘保留。
- **证据与边界：** 上游行为核对 [ApplyConfig / updateGeneral](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/hub/executor/executor.go) 与 [ReCreateTun](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/listener/listener.go)。同提交 CI 和安装包结果见[最近复盘](reviews.md)及任务汇报；不能替代 Windows 开机断网后联网、真实 tailnet 与休眠恢复的设备验收。合并及公开发布须有用户授权，最终版本仍需同提交 CI 与安装包验证。

## 全部节点不可达时的更新与警告

手动全量/组测速或定时测速全部失败后，桌面 GUI、CLI 所有者和 Android 会再读取全部真实叶子节点，依次向两个独立 HTTPS 目标复测，避免把单个站点失败或某一组失败误判为所有节点失联。复测不重置 URLTest/Fallback 的手动选择；没有叶子节点、任一目标可达、节点集合改变或控制器不可用时，不宣称确认全失联。整轮最多 120 秒，节点过多而未完成复测时同样保留设置。

确认失联后只报告可恢复错误，不自动改变系统代理、TUN 或 Android VPN。订阅、Geo 和客户端更新在这些状态下均保持代理优先、失败后独立 DIRECT，不要求关闭接管、切换订阅或全局模式。其他应用仍可能依赖失效节点；警告提供可选的“停止接管”，仅停止当前实例管理的 TUN、Android VPN 或系统代理，不作为更新前置步骤。关闭警告只隐藏消息，不能当作已解除接管或已经恢复网络。启用 Claude 保护后客户端进入只读状态，所有更新、下载及设置修改暂停，直到用户关闭保护；保护期间不利用独立 DIRECT 下载，避免与保护 DNS 和网络隔离交叉。独立 DIRECT 的实现与 DNS 边界见[下载回退](downloads.md#tun-下的更新直连)。

复测期间不持有桌面后台锁；用户更换节点、模式、订阅、配置，停止或重启内核会取消或使旧结果失效。提交前再次核对设置、所有者、内核 PID/generation 与视图版本。Claude 保护期间不自动降级；不停止其他实例的内核或其他 VPN，也不保证第三方环境变量、外部 TUN 或 DNS 本身已恢复。Android Activity 销毁后不承诺继续测速，VPN 后台生命周期保持原有边界。

本机 HTTP 夹具覆盖双目标、任一目标成功、空节点、节点集合变化及控制器失败；所有者回归覆盖停止状态保留设置和取消闸门。隔离 Linux 安装测试覆盖失效节点下开启 TUN、关闭 TUN、系统代理三种状态的订阅更新，保留同一活动订阅和规则模式。这些 Rust 和真实网络回归交同提交 Actions，本机不操作实际网络；不等同于已定位下面的长期服务不可达问题。

## 长时间运行后的服务不可达

`curl` 对某个网站成功、`nslookup` 返回结果，只能证明对应请求和解析路径当时有响应，不能证明目标应用的 DNS、协议、认证或长期连接正常。目前没有针对该症状的可丢弃环境复现，不能认定唯一原因。

在故障用户和相同网络命名空间中先用 `cor diagnose --offline --output cor-tun-local.json` 保留本地现场，再记录 `cor status`，并执行 `cor diagnose --host 故障域名 --url https://故障域名/ --output cor-tun-network.json`；URL 不携带凭据。沿用原 `--data-dir` 等参数，[报告边界](../cli/runtime.md#故障反馈与诊断报告)不等于完整历史日志。分别核对系统路由 HTTP、本地代理 HTTP、系统/内核 DNS、IPv4/IPv6、程序自己的代理环境变量、规则与节点、TLS 或认证返回，以及持续连接是否中途断开。必要时进一步检查 MTU/路径 MTU、IPv6 与 UDP；这些是候选原因，需用目标程序的实际错误和可复现对照验证。日志可能包含域名、IP 或敏感信息，分享前先检查。

0.6.6 正式版的 `diagnose` 出口查询遗漏 `format=json`，共享探测器却按 JSON 解析，纯文本 IPv4 可触发 `trailing characters`；GUI 已带该参数，不能用 GUI 成功代替 CLI 回归。修复同时对齐请求格式，并在共享层严格接受文本/JSON IP、拒绝非成功 HTTP、错误页及无效地址；该诊断缺陷不证明长期 TUN 故障的根因。有限日志和持久采集边界见[日志说明](../cli/runtime.md#连接与日志)，排查时不要只筛选 warning 而遗漏 error。

对于 Codex CLI，可先用 `codex doctor --help` 确认版本支持；支持时在故障用户、目录及环境中运行 `codex doctor --network-only --json > codex-network-before.json`。该命令会检查当前配置、网络环境、提供商 HTTP 与 Responses WebSocket；若采取网络刷新，之后另存一份结果以便对照。诊断会访问配置的提供商，报告虽经脱敏，分享前仍须检查。不同目标、协议或连接的诊断结果不能代替故障现场的实际错误，也不能单凭此命令确认 TUN 是根因。命令边界见 [OpenAI 文档](https://developers.openai.com/codex/changelog/)。

路由未变化时现有后台不会仅因服务不可达而自动刷新。确认可能中断当前内核的其他连接后，可运行 `cor restart --network-only`；它使用既有刷新路径而非重启 TUN，不证明故障根因已修复。保留失败前后的诊断和日志，在可丢弃环境分别验证系统路由与本地代理；真实设备、长时间运行和跨用户场景仍需验收。

2026-11-05 的 root / 0.6.6 现场在停止前出现本机 `/configs` 超时，内核仍继续记录部分流量；其中“停止内核前关闭 TUN 失败”的完整错误链对应关闭 TUN 的 `PATCH /configs` 超时，客户端因此未继续结束内核，不能直接断言整个控制器都无响应。23:23（Asia/Shanghai）日志显示一次路由变化触发的刷新完成，随后有停止和启动记录。用户确认最终是成功执行 `cor stop` 后，再手动执行 `cor start` 恢复网络；不能将恢复归因于自动刷新，也不能以日志未记录命令文本推断未执行启动。完整重启后症状消失，仍未区分出站连接、DNS、TUN 路由及配置处理阻塞的原因；匹配规则和建立出站的日志不证明目标 HTTP 请求已成功。

### 阿里云、Tailscale 与 Fake-IP 的共存现场

2026-11-06 的只读检查确认：`systemd-resolved` 的 eth0 使用阿里云 `100.100.2.136/138`，tailscale0 为 MagicDNS 后缀和反向域提供分域解析，Meta 的 `~.` 接收其余域名。按 eth0 查询 GitHub 返回真实公网 IP，默认查询返回 `198.18.0.14`；MagicDNS 正反向查询均成功。阿里云 DNS 的显式路由走 eth0，Tailscale DNS `100.100.100.100` 走 table 52。`~.` 不覆盖更长的分域匹配，见 [systemd-resolved 路由规则](https://www.freedesktop.org/software/systemd/man/latest/systemd-resolved.service.html)。

阿里云 DNS 位于 Tailscale 使用的 `100.64.0.0/10` 范围内，存在入站反欺骗规则的冲突风险；该环境的 `tailscale-allow-alidns.service` 每 5 秒核对 ts-input 链，只让 eth0 上来自两个精确 DNS 地址、源端口 53、ESTABLISHED 的 UDP/TCP 应答返回后续防火墙，不放行整个网段，也不修改 DNS、路由或 TUN。服务已运行，实际防火墙规则与计数仍需 root 核对；参考 [Tailscale 地址冲突](https://tailscale.com/docs/reference/troubleshooting/cgnat-conflicts)与[阿里云内网 DNS](https://www.alibabacloud.com/help/en/dns/what-is-alibaba-cloud-public-dns)。

本次 GitHub HTTPS 失败时，对照网站可达，使用真实 GitHub IP 并保留 SNI 也可达；用户更新订阅后，Git 及 API 均恢复，而默认 Fake-IP 地址未改变。因此不能把 Fake-IP 结果本身、服务处于 active 或订阅更新后的恢复认作唯一根因。仍需失败前后的内核规则、选定出站和 DNS 结果对照。`tailscale-online.target` / wait-online 未启用或未被依赖时不活跃，不等同于 tailscaled 离线。检查未修改任何系统服务、DNS、路由或订阅。

进一步指定 UDP/TCP 目的端口 53 后，未绑定网卡的阿里云 DNS 请求经 table 2022 进入 Meta；指定 `oif eth0` 则沿 eth0 到网关，`resolvectl query --interface=eth0 api.github.com` 实测返回真实公网 IP。因此普通 `ip route get` 不足以验证 DNS 流量，须同时核对端口和绑定网卡。现有防火墙服务只处理入站应答，不能保证内核的 DNS 引导与代理节点域名解析绕过自身 TUN；实际配置在 root 数据目录，本次未取得，不能认定已形成解析循环，也不应据此关闭 Tailscale DNS 或改用户网络。

## IPv6 节点域名解析

[不发布打包 40883823532](https://github.com/Fu021/Clash-of-Rust/actions/runs/40883823532) 的 Ubuntu 20.04 ARM64 隔离测试在保护 DNS 返回 SERVFAIL 时记录 IPv6 节点 `no ip address`。上游 [mihomo 解析器](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/dns/resolver.go)在 A 查询结束后默认只等待 AAAA 100 毫秒；慢 AAAA 是与日志吻合的可能原因，并非已证明所有现场超时都由此造成。

共享运行配置在订阅未指定 `dns.ipv6-timeout` 时使用 1000 毫秒，保留显式值和原订阅；正常及保护模式均适用，可能延长慢 AAAA 查询的等待。Linux 隔离上游提供快速空 A 与延迟 350 毫秒的 AAAA，检查实际节点解析和保护 DNS，仍需同提交 CI/安装后结果及各端真实网络验收。
