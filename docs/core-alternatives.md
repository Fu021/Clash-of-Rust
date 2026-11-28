# 内核备选方案评估

本次评估比较项目现用 [Mihomo v1.19.34](https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.34) 与 [meow-rs v0.30.0](https://github.com/meow-rs/meow-rs/releases/tag/v0.30.0)。结论是保留 Mihomo 作为默认。若后续实施，Linux 桌面普通代理可作为手动切换的首个实验范围。内核接入、依赖变更与发布另行评审。

## 隔离实测

同一 Linux ARM64 主机在独立网络命名空间运行上游发行二进制。DNS、HTTP、策略组与文件规则集均用回环夹具，关闭外部下载。meow 版本对应 `8e4e948464656380f46f3a98cb168fc35d7708db`，采用上游锁定构建和默认 `full` 特性。表中内存为三次启动后的进程中位数，启动时间从启动进程到控制器认证就绪，时延与吞吐为回环 HTTP。

| 项目 | Mihomo | meow |
| --- | ---: | ---: |
| 解包后内核大小 | 57,344,126 B | 15,924,816 B |
| 空闲 RSS / PSS / USS | 38,196 / 38,188 / 38,188 KiB | 14,648 / 12,969 / 12,940 KiB |
| 冷启动样本 | 11～13 ms | 11～23 ms |
| 回环请求中位数（20 次） | 1.22 ms | 1.16 ms |
| 4 MiB 回环请求吞吐（8 次中位数） | 383.4 MiB/s | 331.8 MiB/s |

另两轮回环吞吐有重叠，未建立性能排名。加载正式包 Geo 后的五条规则夹具另有单次采样。Mihomo RSS / PSS / USS 为 61,020 / 61,012 / 61,012 KiB，meow 为 15,020 / 13,403 / 13,376 KiB，两者采取的加载器和 GEOIP 规则覆盖量有差异。真实节点、TUN 与客户端托盘总内存仍需独立基准。下载归档 SHA256 分别为 `daaf78a6240a282cdfd75bcbf98fe321f6bd52765cc29e79f344191198f0b50f` 与 `7b6578c1e088710cdca8f29fd1dca7b41b49c37693f33440d34d493bfee4ac3b`。

| 用例 | 观察 |
| --- | --- |
| 控制器认证、查询和事件 | 两端未认证返回 401。`version`、`configs`、`rules`、`proxies`、`connections`、provider 列表能读取。模式 PATCH、节点切换、关闭连接、A 查询、普通 HTTP 日志流、`DIRECT` 测速通过。测速夹具对 GET 和 HEAD 都返回 204。 |
| 分流、provider 和热重载 | 文件规则集可分流，修改后 PUT 刷新并改变实际出口。`PUT /configs` 在 Mihomo 用原始 YAML，在 meow 用 Base64。两端均读回变更并复验流量。`force=false` 的网络恢复语义、远程 provider 下载及缓存、节点测速组合和连接响应的全部字段仍待验证。 |
| 受控重启与恢复 | 同一监督进程分别验证 meow 重启自身和 Mihomo 切至 meow。非法 YAML 预检失败时旧内核仍服务。切换后同端口可再次连接，已有 HTTP CONNECT 收到 EOF。候选控制器可用但规则行为不符时，恢复旧内核、配置、选择与 DNS。合成样本从停止到探测通过约 19～55 ms，实际系统网络窗口需另测。 |
| DNS 与 Fake-IP | meow 实测 A 和 AAAA 均分配 Fake-IP，但把 `fake-ip-filter-mode: rule` 降级为 `blacklist`，原排除域名仍返回双栈 Fake-IP。Mihomo 对同一排除域名返回真实 A 和 AAAA。Mihomo 源码接受 IPv6 池，夹具的普通域名 AAAA 为空，尚未确认触发条件。meow 的 `dns.response-rules` 需要单独验证可表达的规则，不能直接复用 Mihomo 语义。 |

## 兼容性与宿主

- **订阅与协议。** 固定版本两者的解析入口共有 Shadowsocks、SOCKS5、HTTP、VMess、VLESS、Snell、Trojan、Hysteria2、WireGuard、AnyTLS 和 Direct。Mihomo 还接受 SSR、Hysteria v1、TUIC、ShadowQUIC、Gost Relay、`dns`、显式 `reject`、`rematch`、SSH、Mieru、Sudoku、MASQUE、TrustTunnel、OpenVPN、Tailscale、ZeroTier 和 EasyTier。meow 有内置 `REJECT`，TrustTunnel 需要单独启用对应构建特性。双方的同名协议仍有参数、传输、混淆和 UDP 差异。meow 源码有 `select`、`url-test`、`fallback`、`load-balance` 和 `relay`，本次实测覆盖 `select` 和文件规则集。预检要逐项报告不兼容节点、引用和规则，避免仅靠成功退出码接受被跳过的内容。
- **Geo。** 用 `v0.7.3` Linux ARM64 正式包内四份 Geo 数据复验。meow 加载 `Country.mmdb` 和 `ASN.mmdb`。未指定 `GeoSite.dat` 路径时校验返回成功，但规则不匹配；通过 `geodata.geosite-path` 指定后，`GEOSITE,Private` 与 Mihomo 均加载 133 条。meow 启动后报告 `GEOIP,private` 27 条和 `GEOIP,telegram` 10 条，Mihomo 的 DAT 后端分别报告 28 和 12 条。即使分类名可用，也要检查网络范围是否等价。meow 忽略 DAT 模式及 `geodata-loader`，依赖 `GeoIP.dat` 的规则需在预检报告差异。未逐类逐地址比对分流。
- **桌面。** 上游有 Linux、Windows、macOS 的 x64 和 ARM64 构建。普通代理可沿用 GUI、TUI、CLI 共享的后台所有权。Linux 特权启动与 DNS 检查校验安装路径及执行文件，Windows TUN 服务使用受保护目录中的已校验 `mihomo.exe`，macOS 沿现有网络助手与 DNS 租约授权路径。第二个内核需要经过各平台明确的可信安装和授权设计，系统代理、TUN、DNS 及网络恢复按[平台边界](security-boundaries.md)验收。
- **Android。** 本项目直接嵌入 Mihomo Go 库，经 `sing_tun` 接收 VPN 描述符，负责 socket Protect 与应用分流。[上游 Android 客户端](https://github.com/meow-rs/meow-android) 提供 Rust JNI 与 tun2socks 参考。接入仍须适配 `VpnService`、描述符所有权、前台通知、应用 UID、IPv6、进程重建和项目支持的 arm64、armv7、x64 ABI。此轮只读源码，未编译 APK 或实测 VPN。
- **分发。** meow 为 MIT，`full` 依赖 BoringSSL 等 Rust 与原生组件。若纳入发布，维护独立的版本、源码与产物哈希、锁定构建和传递许可证清单。双内核会增加下载和 CI 覆盖。分别验证平台最低版本、签名、更新与回退，再决定是否随安装包分发。

节点类型依据 [Mihomo 解析器](https://raw.githubusercontent.com/MetaCubeX/mihomo/v1.19.34/adapter/parser.go)、[meow 解析器](https://github.com/meow-rs/meow-rs/blob/v0.30.0/crates/meow-config/src/proxy_parser.rs)和[构建特性](https://github.com/meow-rs/meow-rs/blob/v0.30.0/crates/meow-app/Cargo.toml)。控制端差异参见 [meow 控制 API](https://meow-rs.github.io/meow-rs/guide/reference/rest-api)。

## 桌面受控重启方案

1. 由当前唯一后台所有者执行，沿用 Claude 保护和恢复中网络修改门禁。将候选配置与内核缓存隔离，在旧内核继续服务时检查配置、协议能力、端口规划及必要授权。缺口在启动前提示。
2. 记录当前模式、选中节点、TUN、系统代理和 DNS 所有权，复用 `Engine::restart_state` 与 `resume` 的状态恢复顺序。GUI 与托盘持续运行并显示进度，编辑草稿保留。用内核专属适配器选择热重载协议，需要重启时提前提示已有连接会中断。
3. 由后台停止旧进程并处理网络接管，再启动和验证新进程。检查控制接口、配置版本、监听、分流及实际网络后，提交选择和配置。拒绝静默改用另一出口；系统接管由原记录和平台权限约束。
4. 任一步失败时恢复上一份已验证的配置、内核和网络状态。启动中断时从未完成事务恢复，上次成功选择才作为冷启动默认。回退成功给可恢复提示，旧内核仍无法恢复时报告严重错误及诊断。

可按 Linux 普通代理、桌面特权路径、Android VPN 的顺序推进。第一阶段先验证真实 GUI、活动连接中断、配置与状态回退、端口冲突和断电恢复。现有连接跨内核迁移需要会话、NAT 和 Fake-IP 状态转移，超出该方案的验收范围。
