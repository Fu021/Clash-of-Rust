# 内部诊断与测速错误码

[文档索引](README.md) · [导出方式与隐私边界](cli/runtime.md#故障反馈与诊断报告)

错误码供维护者分析 JSON 报告，不作为界面标签或要求用户理解的提示。GUI 和 CLI 仍显示具体、可读的失败原因，提供导出诊断入口；不会自动上传日志或修改网络。

## 桌面启动与托盘

桌面窗口导出的 `data.context.desktop_startup` 保存本进程是否收到 `--background`、是否为更新后重启、窗口可见状态和会话类型。`autostart` 在导出时读取实际配置，区分 `collected` 与 `unavailable`，读取失败不视作未启用。`tray_state` 区分 `initializing`、`created`、`failed`、`not_started`，`tray_error` 保留本次托盘初始化失败的原因；用户关闭警告后仍保留。`created` 只说明原生托盘对象创建成功，不保证用户能在桌面面板看见图标。

`history` 保留至多 96 项启动/显示事件、来源、接收时间和状态变化；重复未变化轮询不填满记录，丢弃数单列。无需先导出，数据目录的 `desktop-startup.json` 与 `desktop-startup.previous.json` 即保留当前/上次记录，CLI 的在线与离线诊断也会读取。`tray_initialization` 保存尝试次数、时限、首尾错误和完成耗时。Linux 导出另有 `linux.autostart`（存在、大小、修改时间、后台参数、是否匹配当前入口、隐藏/禁用）和 `linux.tray_bus`（host、watcher owner、最多 16 个注册项及 PID、是否本进程）。启动时异步采集的同类快照写入 `linux_at_startup`，与导出时的现场分开；查询失败分别记未知。

报告不包含完整命令行、环境变量、D-Bus 连接地址或自启动文件正文。注册项是有界 D-Bus 标识，不包含订阅或控制凭据。自启动配置存在不证明登录会话实际传入了后台参数；原生对象创建成功也不能反推托盘从未故障。旧报告缺少这些字段时保持未知，不凭“内核启动成功”或没有错误条幅认定托盘就绪。排查入口及行为见[静默自启动与托盘告警](agent-failures/desktop.md#静默自启动与托盘告警)。

## 桌面 TUN 启动与覆盖

桌面导出和 `cor diagnose`／`--offline` 共用 `data.tun_configuration`，只提取生成运行配置的地址族及路由/DNS 覆盖字段，最多读取 4 MiB，不读取链接或特殊文件，不输出订阅节点、密码、控制密钥或上游 DNS 正文。`source=generated_runtime_config` 明确它是文件快照；`ipv6_enabled` 不证明实际存在 IPv6 TUN。启动校验结合控制接口实际配置、上联 IPv6 地址条件和路由，`tun.inet6-address` 为空不能独自证明本机没有 IPv6。缺少、不可读、过大与无效文件分开记录。

| 内部标记 | 保存位置 | 含义与处理 |
| --- | --- | --- |
| `tun_ipv6_not_configured` | 内核/客户端缓冲日志 | 允许 IPv6，但内核未创建 IPv6 TUN 地址；结合上联地址和实际路由判断，不单凭此项停止 IPv4 TUN |
| `tun_ipv4_route_bypass` | `host_routes.warnings` | IPv4 示例仍经过非 TUN 接口；启动校验失败并回滚，运行中显示绕行提示 |
| `tun_ipv6_route_bypass` | `host_routes.warnings` | 当前需要接管的 IPv6 样本经过非 TUN 接口；本机确认没有可用 IPv6 时不生成，保护期间同样适用 |
| `tun_current_uid_route_bypass` | `host_routes.warnings` | 普通路由进 TUN，但当前 UID 的 TCP/443 路由样本走物理接口；核对 UID、mark 及重定向策略，不等于已证实 TLS 故障 |
| `route_unavailable`／`route_query_timeout` | `host_routes.ipv4/ipv6.reason` | 未取得可判定样本；未知不是网络成功或失败 |

[地址族校验与验证边界](agent-failures/network-recovery.md#linux-tun-地址族与启动校验)见网络恢复说明；不把物理 IPv6 路由单独归因于桌面环境或权限。

`host_routes.ipv6_uplink` 保存本机上联地址条件及每接口可用、tentative、重复地址计数，不导出完整地址；地址读取失败为 `unknown`。`coverage_expectations` 保存本次检查要求和来源（`no_uplink_ipv6`、`ipv6_disabled`、`claude_protection`、`current_settings`、`unknown`）。缺失 IPv6 与 IPv6 绕行是不同状态，普通 TUN 和保护均按[同一边界](ipv6-tun.md#判定)判断。

Linux 的桌面导出及 `cor diagnose`／`--offline` 另保存：

- `host_routes.network_namespace`：采样者所在网络命名空间，不能用另一命名空间的 TUN 证明本命名空间已接管。
- `host_routes.tcp_uid_samples`：普通观察保留当前有效 UID 的两个固定目标 TCP/443 查询；诊断另补 root 对照，root 通过 sudo 运行时改与 `SUDO_UID` 比较。不以另一用户身份发送网络请求。
- `host_routes.policy_rules`：两种地址族的策略表号、优先级及 UID、mark、接口、协议/端口选择条件，不导出任意字段。每族限 64 KiB、256 条，截断记 `partial`；不可读、超时及格式无效保持 `unknown`，不能推定没有限制。
- `tun_configuration` 追加 UID/UID 范围、接口及排除地址配置，区分“没有配置限制”和“显式限制其他用户”；仍为生成文件快照，不替代实时内核状态。

普通覆盖检查优先当前 UID 的有效 TCP 样本，不借用 root 或未知样本；`auto-redirect` 的转发标记例外、旧报告兼容及查询边界见[跨用户路由检查](network-paths.md#跨用户路由检查)。`tun_configuration.auto_redirect` 保留生成配置开关。只有对照用户和策略表查询限于诊断导出；`policy_unavailable`、`policy_query_timeout`、`invalid_policy_response`、`policy_response_too_large`、`tcp_route_unavailable` 是取证标记，不直接弹窗。

## 测速记录

桌面和 Android 报告的 `data.context.measurements`、`cor diagnose` 的 `data.measurements` 保存当前进程/后台最近四次已完成的测速摘要。CLI 后台通过可选 `Meta.measurements` 传给附加前端；旧后台没有该字段时视为无记录。当前进程中重新应用设置或切换控制端口会保留记录，进程退出后不保证仍有这些内存记录。

每项含 `schema=1`、`scope`、UTC 开始时间、耗时、结束阶段、请求次数、成功数、节点失败数、控制请求失败数、错误码计数及 HTTP 状态计数。`completed=true` 表示这一轮请求处理完成，节点仍可能失败；不能据此认定所有节点可用。当前并发上限为 6，单次内核探测参数为 5000 ms，控制客户端总超时为 8 秒，两者不能混淆。

目标只记录 `gstatic_generate_204`、`cloudflare_trace` 或 `subscription_custom`，不保存自定义测速 URL、查询参数、节点地址、订阅令牌或任意控制响应正文。记录阶段包括 `query_nodes`、`query_providers`、`measure`、`reset_automatic_group`、`read_automatic_selection`、`verify_controller`、`complete`。准备、provider 查询及组状态更新失败可能使 `attempted=0` 或保留之前已完成请求的计数，应结合 `phase` 与 `failure` 判断。

| 内部错误码 | 依据 | 计入节点失败 | 排查方向 |
| --- | --- | --- | --- |
| `node_timeout` | mihomo 测速端点返回 HTTP 504 | 是 | 节点、DNS、线路及测速目标；这不是控制连接超时 |
| `node_failed` | 测速端点返回 HTTP 503，或有效响应中的延迟为 0 | 是 | 探测失败但原因未细分，结合同时段内核日志；不能直接断言节点服务器离线 |
| `controller_connect` | 无法建立控制接口连接 | 否 | 内核/端口/本机访问路径 |
| `controller_timeout` | 控制请求连接、等待响应或读取超时 | 否 | 控制接口响应与负载，不把全部节点标为失联 |
| `controller_authentication` | HTTP 401/403 | 否 | 当前实例与控制密钥是否匹配；密钥不进入报告 |
| `controller_http` | 其他异常 HTTP 状态，或准备/组操作的非成功状态 | 否 | API 兼容性、内核状态及对应阶段 |
| `controller_response` | JSON 无效、缺少 `delay`、类型错误、额外内容或响应超过 4096 字节 | 否 | 控制响应格式与版本；不把缺字段默认为 0 |
| `missing_node` | 测速端点及 provider 路径无法找到目标 | 否 | 节点是否已被订阅更新、删除，provider 是否完成加载 |
| `invalid_probe` | 测速端点返回 HTTP 400/422 | 否 | 测速 URL、预期状态及参数 |
| `query_failed` | 其他无法归类的准备、传输或组操作错误 | 否 | 保留阶段和可读错误，不能猜为网络或节点故障 |

非节点类错误使本轮测速明确失败，不再用 `unwrap_or(0)` 吞掉错误；不发布不完整结果，不触发“全部节点不可达”恢复确认，旧测量值也不冒充本次结果。只有收到有效节点失败结果后，才沿用双目标确认与保留接管的策略。成功响应仍保持 CLI 原有的节点到延迟数值映射。

状态分类依据固定依赖 mihomo `v1.19.33` 的 [`getProxyDelay`](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/hub/route/proxies.go) 和 [provider 路由](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/hub/route/provider.go)；后续升级须同时核对这些语义。

## Android 现场

平台摘要另外记录活动网络是否为 VPN、是否通过系统验证、最多八个底层网络的类型、DNS 地址族数量、Android 10 起的 MTU 和 Android 9 起的私人 DNS 状态；系统不支持的字段不会出现。不记录 DNS 地址、Wi-Fi 名称、网络标识或私人 DNS 主机名；系统验证失败和私人 DNS 开启都只是观察结果，不能单独作为断网原因。

`socket_protection.required` 表示当前是否已安装保护器并要求新 socket 绕过 VPN，另有本进程累计调用/失败计数；计数来自实际 `protect(fd)` 回调和底层 socket 控制错误，不包含 FD、节点或目标地址。失败数为 0 不证明历史所有流量正常；未启用 VPN 时保护器未安装是正常状态。VPN 连接、待授权/启动、代次和最近错误分别记录，避免把内核运行当成系统 VPN 已连接。

`application_routing` 保存按应用出站识别的累计计数，不包含应用清单、节点名或目标地址。`supported` 表示 Android 10+ 的宿主接口可用；`owner_queries`/`owner_failures` 为连接归属查询和失败，`matches` 为应用规则选择，`missing_targets` 为目标缺失或 PASS，`udp_rejected` 为指定目标不支持 UDP 的拦截次数。模式和保护导致规则暂停时不增加命中，累计计数非本轮结果，也不能替代逐连接的 UID/出站验证。

| 内部标记 | 含义与边界 |
| --- | --- |
| `APP_OWNER_LOOKUP_FAILED` | 原始元组缺失、系统拒绝、隔离 UID 或无可识别连接；一分钟至多记录一次，连接跟随配置，不伪造应用归属 |
| `APP_OWNER_IDENTITY_CHANGED` | 绑定包名已卸载或其当前 UID 与连接不符；计入归属失败，连接跟随配置，避免 UID 复用继承旧绑定 |
| `missing_targets` 增加 | 当前规则目标缺失，按该规则失效策略跟随配置或拦截；不等于节点超时 |
| `udp_rejected` 增加 | 目标仍存在但不支持 UDP，拒绝该 UDP，不能继续落到其他出站 |

Android 内核缓冲日志附 `[observed_at=UTC时间]`，代表客户端收到日志的时间，不等同内核产生事件的精确时间；仍受现有缓冲及单条长度上限约束，不能恢复已丢失的历史。

Android 文档选择、导出和导入失败另外在应用日志中记录 `DOCUMENT_PICKER_FAILED`、`DOCUMENT_EXPORT_FAILED`、`DOCUMENT_IMPORT_FAILED` 及异常类型，不写入 URI 或文件正文。界面保留具体错误；诊断导出回归先核对实际待写入内容，再检查保存结果和写入错误，不能将空文件或文件选择器已经打开当作导出成功。

## 回归与现场导出

普通 CI 按测速、诊断和 Android 宿主相关源码改动选择实际 APK 回归；全版本诊断专项须维护者显式选择 `verify_diagnostics`，见[测试入口和验证边界](workflows/android-compatibility.md#测速与诊断专项)。覆盖成功与拒绝连接混合、VPN 开关、控制认证失败、恢复后复测及真实诊断文件写入；模拟器中的确定性故障不能直接解释用户现场。

混合测速夹具经本地 HTTP 代理连接隔离源站；测速 URL 的目标必须与源站允许的 CONNECT 地址一致，否则两个节点都会真实失败，不能用于验证“一成功、一失败”。批量控制异常夹具须把节点放入策略组，确保实际发出请求，避免空批次的成功结果掩盖分类问题。

用户复现后应尽快在日志页点“诊断”保存 `cor-diagnose.json`，附上操作步骤和发生时间；不要先重启进程再导出，否则可能失去内存中的测速摘要。CLI 对应 `cor diagnose --output cor-diagnose.json`，后台无法连接时用 `cor diagnose --offline --output cor-diagnose.json`。报告留在本地，分享前检查敏感内容。

## 现场证据边界

核对错误与采集时间、内核及 VPN 状态。稍后导出的报告不一定保留故障发生时的网络条件；DIRECT 拨号拒绝、IPv4/IPv6 超时不能代替逐节点测速状态和套接字保护结果，`pending` 也不能当作检测失败。

测速路径保留真实错误分类。上游 [`dialer.go`](https://github.com/MetaCubeX/mihomo/blob/v1.19.33/component/dialer/dialer.go) 在安装 `DefaultSocketHook` 后，TCP/UDP 拨号已跳过 `interface-name` 与 `routing-mark`；本应用初始化时就安装该 hook，不能仅凭订阅含桌面网卡字段认定它导致 Android 故障。没有充分证据时不改写订阅或 provider override，也不改变节点协议、DNS、证书验证及出站链。先取得同一次复现的报告，核对阶段、HTTP 分类、VPN 和保护计数，再决定是否需要具体节点协议或 DNS 专项。
