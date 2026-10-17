## 功能目标

为 Windows 版 Clash of Rust 增加原生、可持久化的 WSL2 受管网络功能：WSL 普通原生程序的公网 TCP/UDP 流量，不依赖应用代理环境变量，经过 Clash of Rust 管理的同一个 Mihomo 核心，固定交给用户指定的 VPS。代理或透明转发层不可用时，受管工作负载应失败关闭，不回退到宿主公网直连。

这是本 fork 的功能规划，不是声称当前版本已经支持，也不是生产漏洞报告。

## 范围与明确排除项

- WSL 不需要访问校园服务，不为其增加校园网、LAN 或宿主 DNS 直连例外。
- Windows 现有校园认证、正常系统联网及其他应用的正常通路必须保留；不得为了 WSL 接管而将整个 Windows 强制全局代理。
- 继续由 Windows 正式 Clash of Rust 独占管理同一个核心；不在 WSL 启动第二份 Mihomo 或平行的直连 VPS 客户端。
- 面向普通非特权工作负载，不承诺抵御 Windows 管理员、WSL root 或可控制 Docker daemon 的主体。
- 不以登录 Claude/Anthropic、模型请求或真实业务检测页作为开发验收手段；使用自有或获准中立端点。
- 不承诺完整复制 VPS 的硬件/内核/浏览器指纹，不伪造 `ip addr` 等命令输出，也不保证第三方平台永久认可某个地区。

## 当前能力缺口

本次设计依据包含 v0.5.9 的配置生成逻辑；这里不将该版本称为当前最新版本。

1. `src/config.rs` 的运行配置生成器移除自定义 `listeners` 和 `authentication`，重写普通入口并限制 loopback 绑定。因此临时编辑订阅/runtime YAML 添加 WSL listener 不能视为受支持的持久机制。
2. 原生 Claude 保护按指定目的域名/IP 生成规则，并保留其他普通规则，不等于全部 WSL 目标都固定至指定 VPS。
3. 普通 mixed-port 和 `HTTP_PROXY`/`ALL_PROXY` 不能强制接管忽略代理变量的程序，也不能独自建立完整 UDP、DNS 和失败关闭边界。
4. WSL mirrored/NAT、DNS tunneling、Docker Desktop 和 Windows interop 的通路不同，不能只凭 Linux OUTPUT 规则就认定全覆盖。

## 希望实现的能力

### 1. 原生同核心固定策略与持久化

- 提供 WSL 专用入口或等效、经过验证的 WSL 独立流量身份，不依赖普通 Windows 代理组最终分流。
- 策略固定到指定节点，或仅含获准节点的组；禁止 DIRECT/PASS 或其他未获准节点回退。节点缺失、入口校验失败时拒绝连接。
- 受支持的配置生成流程保留入口、认证和绑定，明确原生保护、用户设置与 WSL 策略的唯一配置所有者。
- 普通模式/节点/profile 切换、订阅更新和配置再生不能悄然改变 WSL 固定出口。若相关状态不能兼容，应明确拒绝操作或将 WSL 标为不可用且失败关闭。
- 避免向 LAN 暴露通用代理或核心控制器；入口具备最小绑定范围、必要认证和明确调用者边界。
- 不通过多个定时维护器互相覆盖运行配置实现持久化，不把完整配置热更新描述成必然无网络副作用。

### 2. WSL 透明 TCP/UDP 与双栈

- 普通应用即使清空全部大小写代理变量、设置 NO_PROXY 或不支持代理，TCP/UDP 仍进入受管路径。
- 如采用 TUN-to-SOCKS，完整验证标准 SOCKS5 UDP ASSOCIATE：relay 地址/端口可达、认证、控制连接生命周期、来源校验、返回路径、MTU/分片与超时。
- 分开处理 IPv4、IPv6 字面量、AAAA-only 和 Happy Eyeballs，不因只接管 IPv4 而保留 IPv6 直连旁路。
- 客户端以 IPv6 连接 VPS，与 VPS 以 IPv4 连接目标是不同段。若用户选择严格 IPv4 出站，明确 IPv6-only 目标的功能限制，不将拒绝 IPv6 计为 IPv6 功能通过。
- ICMP、广播、组播及其他 IP 协议单列支持范围；普通 SOCKS/HTTP 代理不自动等于任意协议三层 VPN。

### 3. DNS 独立受控

- 系统解析、应用内 DoH/DoT、bootstrap、备用 resolver、冷缓存及 Docker DNS 分别记录通路，公共解析经固定 VPS 到核实的非大陆解析服务。
- DNS 是并行解析链，不是全部网页数据的串行下一跳；经 VPS、TLS 校验成功和 resolver 服务点位于境外须分别给出证据。
- 核对 DNS tunneling 的有效状态，不能只依据 `.wslconfig` 文本或 Linux UDP/53 抓包推导宿主委托解析安全。
- 上述 v0.5.9 保护生成逻辑中的整个 `in-addr.arpa`/`ip6.arpa` system 例外也涉及公网 PTR，需单独验收，不能统称为仅私网反查。
- 不修改全局 NRPT 去锁定虚拟 DNS，不封锁整个 Windows Dnscache；不以禁用 TLS 校验或 HTTPS 中间人来证明 DNS/信息最小化。
- 对任意 HTTPS 内的自主解析，明确应用信任边界；不能以封锁 53/853 宣称所有不可信程序均只使用指定 resolver。

### 4. 失败关闭、隔离与状态可见性

- 默认拒绝边界先于受管工作负载开放，透明转发器退出、认证失败、节点失效、配置再生、恢复或地址变化时不自动开放直连。
- 明确普通 shell、systemd 服务、后台任务和子进程如何纳管；只包装一个 launcher 不能称为全部 WSL 接管完成。
- 可选 netns/UTS 最小视图，让普通负载真实只看到回环 lo、私有 TUN/veth 和独立 hostname；不把它描述为完整机器身份隔离。
- Docker Desktop daemon/容器、原生 Docker FORWARD、Windows `.exe` interop、WSLg/IDE/本地 IPC 分别纳管或显式标为未覆盖。必要本地例外精确到调用者/socket/用途，不放开整个 LAN 或全部 loopback 端口。
- UI 展示入口、固定节点、配置代次、透明层、DNS、协议能力、失败关闭和覆盖状态，避免仅凭“进程运行/端口监听”显示全部正常。

## 验收标准

- [ ] 不读代理变量的中立 TCP/UDP 探针均成功经指定 VPS，接收端源地址与请求 nonce 可关联；宿主出口无该探针的目标直连。
- [ ] 标准 SOCKS UDP relay、IPv4/IPv6、QUIC（若支持）、字面量和动态目标分别验证；UDP/IPv6 被阻断只能记录为安全降级，不能算完整功能通过。
- [ ] 系统 DNS、DoH/DoT、冷启动/备用路径、公网 PTR、DNS tunneling 与容器解析有独立证据；未知路径保留未验收状态。
- [ ] 普通 rule/global/direct、组节点/DIRECT、profile 切换和配置再生后，WSL 仍固定指定 VPS 或失败关闭，不漂移到普通分流出口。
- [ ] 在隔离环境测试入口消失、认证失败、透明器退出、节点失败、重启/恢复和网络变化；不停止生产共享核心、切断宿主 DNS 或网卡注入故障。
- [ ] 每次可能影响宿主网络的变更前后，正常通路的 Windows 关键服务 DNS、有效 TLS 证书校验和 HTTP 语义保持正常；准备紧邻变更备份和仅本轮对象的定向回滚。
- [ ] 普通终端、systemd、容器、interop 和本地 IPC 的覆盖/例外逐项声明；功能成功、安全无旁路、地区核验三种结果分开报告。
- [ ] 保留现有用户配置与账号资料；Issue、日志和测试产物不公开真实 VPS 地址、凭据、订阅或私密配置。

## 建议实施顺序

1. 先审查同核心固定策略与配置持久化，不先迁移工作负载。
2. 隔离验证实际核心、入口认证、标准 SOCKS UDP、普通模式独立性及生命周期。
3. 选择一种透明适配器并建立最小权限/失败关闭边界；tun2proxy、hev-socks5-tunnel、tun2socks 仅为待核验候选，不同时部署多套。
4. 接入受控 DNS，逐步覆盖服务、容器和跨系统边界。
5. 完成分项验收、状态 UI 和定向回滚说明后，再宣告对应范围可用。
