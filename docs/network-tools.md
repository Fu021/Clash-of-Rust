# 网络工具箱

桌面、Android、CLI 和 TUI 共用检查逻辑；iOS 只有界面原型。工具按需运行，不常驻扫描，不保留历史。输入、操作和当前结果放在同一张卡片；切换页面或工具时取消检查并清空，迟到结果不得回填。

## 工具与路径

| 工具 | CLI 子命令 `cor network …` | 检查内容 |
| --- | --- | --- |
| 网站检查 | `check URL` | 系统/内核解析、HTTPS 和 HTTP、采用协议及真实策略链 |
| DNS 对照 | `dns HOST` | 比较系统与内核解析；结果不同不单独视为故障 |
| 连接耗时 | `phase URL` | 独立内核 DNS、本机 SOCKS5、目标 TCP、TLS、首字节 |
| IPv4 / IPv6 | `family URL` | 分别用目标地址族经本机代理连接；无 AAAA 不伪造连接失败 |
| 规则与出口 | `explain URL` | 从本次真实请求查规则和策略链，不用当前节点代替 |
| TLS 证书 | `tls HOST` | 验证域名、有效期、信任链，显示证书摘要，不绕过验证 |
| 短时稳定性 | `stability URL` | 有限次请求、失败数量和响应头耗时分布 |
| UDP 连通 | `udp SERVER[:PORT]` | 经本机 SOCKS5 UDP 中继发送 DNS 探针，默认端口 53 |
| Ping | `ping HOST` | 系统 ICMP；不代表代理节点延迟 |
| 路由追踪 | `trace HOST` | 系统逐跳响应；部分跳不响应不等于断网 |
| 限量下载 | `download URL` | 有时间和流量上限的接收检查，正文即时丢弃 |
| TCP 端口 | `tcp HOST[:PORT]` | 经本机 SOCKS5 连接指定端口，不发送应用数据 |
| HTTP 响应详情 | `http URL` | HEAD/GET/OPTIONS、状态、受限响应头、最多 4 KiB 正文长度；正文丢弃 |
| 规则预判 | `rule URL` | 不连接目标；按最后生成配置预判，动态条件返回未知 |
| 配置差异 | `config-diff` | 原订阅与生成配置的类型、数量和是否变化，不展示敏感值 |
| 流量分解 | `traffic` | 当前连接按主机、进程、策略链、规则类型聚合，每类最多八组 |
| 内核运行 | `performance` | 单次读取版本、连接计数及托管堆，不把托管堆当 RSS |
| 节点运行详情 | `node NODE` | 节点类型、最近一次测速、所属组与当前策略链，不主动测速 |

后七项及 DNS 记录类型从 `0.7.2-fix.4.network-inspection` 提供。DNS、TLS、测速和真实出口沿用已有入口，不另外增加重复工具。与 [Surge CLI](https://manual.nssurge.com/tools/cli.html) 的轻量检查对照；MITM、脚本环境、网关及远程组网属于架构扩展，未包含在本轮工具中。

网站及 HTTP 检查经现有内核代理，不自动启动内核、切换节点或回退 DIRECT。HTTP 403、500 等只证明收到响应，不证明服务可用；下载则要求成功状态。重定向不会自动跳转。输入允许省略协议；网站检查的双协议选择见 [CLI 检查](cli/maintenance.md#配置诊断与检测)，其余 HTTP 工具使用解析后的目标，TLS 工具强制 HTTPS。

系统 Ping/追踪不经过系统 HTTP 代理，TUN 是否接管由路由决定。缺组件、权限不足、超时和非零退出分别报告，不用 TCP 冒充 ICMP；取消会终止自有子进程。Android 系统可能没有 traceroute，此时工具明确不可用。

## 参数与边界

默认 5 次、间隔 1000 毫秒、单次时限 10 秒、下载上限 8 MiB、最大 8 跳。GUI 显示对应工具的参数，切换工具恢复默认值；CLI 接受下列对应选项。TUI 使用默认值，按 `[` / `]` 切换工具，`Esc` 取消；自定义参数使用 CLI。

| 选项 | 有效范围 | 使用工具 |
| --- | --- | --- |
| `--count` | 1～20 | 稳定性、Ping |
| `--interval-ms` | 100～5000 | 稳定性 |
| `--seconds` | 1～30 | 耗时、地址族、TLS、稳定性、UDP、下载 |
| `--max-bytes` | 1024～16777216 | 下载；GUI 以整数 MiB 输入 |
| `--max-hops` | 1～16 | 追踪 |
| `--type` | ADDR、A、AAAA、CNAME、MX、TXT、NS、SOA、PTR、CAA | DNS；默认 ADDR（系统 A/AAAA 与内核真实地址对照） |
| `--method` | HEAD、GET、OPTIONS | HTTP 响应详情；默认 HEAD |

记录类型和方法接受大小写。指定 DNS 类型时仍保留系统 A/AAAA 对照，详细记录来自内核的 `/dns/query`；展示 TTL 与返回的 AA/RA 标志，缺失时明确“未提供”，不推算权威服务器。TXT/SOA 正文不进入报告，最多展示 16 条回答。PTR 输入反向查询域名，例如 `1.2.0.192.in-addr.arpa`。TCP 省略端口时为 443，HTTP URL 则使用其明确端口或协议默认端口；空端口、0、越界值拒绝。

配置差异只读当前实例的数据目录，不创建文件，不显示动态字段名、节点地址、密钥、DNS 正文或订阅内容。规则预判支持 DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、字面 IP 的 IP-CIDR/IP-CIDR6、单个 DST-PORT、NETWORK 和 MATCH；遇到 RULE-SET、Geo、进程或其他未支持条件，停止并报告第一个未知项，不能跳过未知规则声称后续命中。它不解析 DNS、不读取组的实时选择，也不替代真实请求的“规则与出口”；保护开启或状态未知时不声称预测了保护出口。

流量分解只统计当前仍存在连接的累计字节，不是全日用量；每类最多 128 个分组加“其他”，显示前八组。进程字段取决于内核和系统权限。内存指标读取 `/memory` 的首个有界样本后立即关闭流，不常驻采样；内核不提供的运行时间等字段不猜测。节点使用完整名称，支持 emoji；历史测速不代表当前健康，负载均衡的具体出站须以实际连接为准。

稳定性总预算最多 120 秒；次数、间隔和单次时限组合过长时返回已取得的结果并明确超时。HTTP/SOCKS 请求和系统进程也有总预算；不能无限等待。系统输出最多各保留 8 KiB，报告最多 48 项，每项最多 2048 字符；原始 URL 路径、查询、账号密码和下载正文不进入报告。证书只展示受限字段。

## 保护与报告

Claude 保护开启或状态未知时，不查询系统 DNS，不发送 UDP、ICMP 或逐跳探针；这些工具返回不可用原因。用户关闭 UDP 时不发送 UDP 探针。其他工具仍经本机代理和现有规则，不借诊断绕过保护。

新增工具的 JSON 包含 `schema`、`tool`、脱敏源站 `target`、`elapsed_ms`、`status` 和逐项 `entries`，每项含 `label`、`detail`、`success`、`code`。`success=null` 表示未确认或说明；`complete`、`partial`、`failed`、`unavailable` 不能互相替代。系统输出及正常达到流量/时限上限属于说明，不单独把成功检查降为部分完成；未捕获实际线路仍明确标为未确认。CLI 对 `failed/unavailable` 返回非零退出码，仍先输出报告；原有 `network check` 保留原 JSON 结构。

GUI 诊断携带当前工具、结果和错误码；CLI 用 `--json` 或 `--output FILE` 保存。切页后当前结果被清除，需要时应先导出。错误码定义见[诊断说明](diagnostics.md#网络工具)。
