# macOS Tailscale DNS 共存

## Runner DNS 恢复

当前固定的官方 Tailscale Action 在 macOS 且启用 MagicDNS 时，会[单独执行 DNS 设置](https://github.com/tailscale/github-action/blob/064b10558093f38325e838ddea35486bdc6b1d5c/src/main.ts)，即使传入 `--accept-dns=false`，仍将 Ethernet DNS 改为 `100.100.100.100`。不能只依赖 daemon 参数避免这项副作用。此处仅讨论 CI 私网接入，不等同于下方应用的 Claude 保护 DNS 接管。

CI `43567907725` 的 Intel Mac 在恢复命令返回成功后，Rust 工具链查询 `static.rust-lang.org` 失败，后续附件上传也出现 `ENOTFOUND`；应用尚未编译或运行，不能归因为应用 TUN。历史 CI `43506898037` 的 PyPI 解析失败发生在 Tailscale 登录前，说明不能把所有 runner DNS 故障都归为接入副作用。另一次 TUN 子会话 socket 未就绪属于独立问题，见[生命周期](macos-lifecycle.md#内核退出后的端口复用)。

`macos_runner_network.py` 通过既有 `action_cache.py network-save/network-restore` 入口工作：接入前保存显式 DNS、搜索域、实际 resolver、默认路由及 GitHub/API/Rust/PyPI 的原生解析基线；接入后恢复原值并读回核对，仅在 Actions 的 Mac runner 刷新一次系统解析缓存，再用独立有超时的 `getaddrinfo` 进程确认解析恢复，同时核对实际默认解析器，避免缓存答案掩盖配置尚未生效。恢复窗口为 30 秒，单轮最多增加 6 秒在途检查时间；失败不继续工具链安装，不替换公共 DNS，也不改变 Tailscale 路由或系统代理。

报告 `target/cache-network-report.json` 区分接入前已故障和接入后未恢复，并记录恢复前 resolver、命令结果、逐次原生解析及失败时向原默认服务器发出的直接 DNS 对照。DNS 设置写入成功不代表原生解析成功，直接 DNS 回答也不代替原生就绪。缓存刷新失败保留实际返回值，最终仍以原生探测为准。报告复制进现有 `macos-ci-report-*` 附件，失败摘要同时写入 job 日志，避免上传故障时丢失全部依据。Mac 后续步骤失败时再收一份只读解析现场。离线回归覆盖自动/显式配置、延迟恢复、持续失败、恢复命令失败及非 runner 拒绝；原始间歇故障的首次触发仍需同环境证据，不能以重试成功宣称根因已经排除。

## 应用 DNS 接管

Tailscale 的独立补充分域解析器可以继续成为 macOS 全局 DNS，即使本应用已修改各网络服务的 DNS；因此不能把网络服务写入成功当成保护生效，也不应要求用户关闭 Tailscale DNS。[CI 40470411381](https://github.com/Fu021/Clash-of-Rust/actions/runs/40470411381) 证实独立临时解析器可能未被系统选中；[CI 40478327297](https://github.com/Fu021/Clash-of-Rust/actions/runs/40478327297) 与[故障实验 40481813828](https://github.com/Fu021/Clash-of-Rust/actions/runs/40481813828) 进一步证明默认解析器竞争和 DNS 故障回退可落到 VPN 或物理网络，不能仅调整优先级或仅验证首选路径。

修复采用可撤销的动态服务 DNS 覆盖：默认解析器统一指向助手独占的 `127.0.0.1:53`，仅转发到内核随机高端口，根分域中的私有/搜索后缀克隆为独立分域，保留原服务器、端口和接口，不复制 configd 生成的内部身份。覆盖 State 和 Setup 运行缓存而不写 SCPreferences；Apple 的 [configd 合并逻辑](https://github.com/apple-oss-distributions/configd/blob/main/Plugins/IPMonitor/ip_plugin.c)优先采用 Setup 服务器，不能只改 State。标准端口兼容不识别 `ServerPort` 的 `resolv.conf` 客户端，回归包括原生解析和 `dig` TCP/UDP；端口占用须先拒绝接管，转发应有并发与超时限制并随租约清理。SCDynamicStore 通知由同一助手线程串行处理重连与恢复；先保存新旧并集的恢复记录，再原子写入全部变化。恢复按托管地址/端口和分域字段组比较，保留外部修改，不重建已删除服务。运行中出现冲突时隔离为本机 DNS 并报告，不能以恢复 VPN 默认 DNS 处理错误；失败后限频重试。旧 journal 仍可清理，助手协议升级见[代理状态与安全恢复](desktop.md#系统代理状态与安全恢复)。

重连测试须等待助手明确报告分域冲突，而非任意检查错误；全局 DNS 的瞬态不匹配可能早于隔离完成。[CI 40507892365](https://github.com/Fu021/Clash-of-Rust/actions/runs/40507892365) 暴露了此等待条件过宽的问题。仍须断言实际原生解析到内核、VPN 未收到公共查询；失败时在清理前记录原生解析器，不能仅凭配置中旧地址消失判定隔离完成。

重连新增私有搜索后缀时，默认保护探针不代表系统已消费新分域。先用独立名称确认新后缀返回 VPN 夹具地址，且该夹具实际收到查询，再执行原业务名称断言；该就绪等待与冲突恢复复用，不重试业务名称或放宽公共 DNS 不得回退的断言。

[CI 40510518605](https://github.com/Fu021/Clash-of-Rust/actions/runs/40510518605) 的重连查询仍返回 VPN 答案，而清理前 `scutil --dns` 已显示本机默认解析器；动态配置发布与 mDNSResponder 实际消费之间存在窗口。就绪确认须通过公开 DNS-SD 接口发出随机名称，并由本机转发器匹配当前探针后终止，不能只读配置、固定等待或重试业务域名到成功；不调用私有 ACK 接口冒充系统确认。默认探针不包含用户域名，每次使用新的随机末级标签以避开私有后缀及旧缓存；到达本机后不交给内核上游。限制单个活动探针、等待时限与取消资源，状态检查同时核对全部托管记录和当前计划，避免新变化沿用旧的成功状态。重连和冲突恢复先等待真实就绪，再断言实际业务查询及无公网回退。

[不发布打包 40869085991](https://github.com/Fu021/Clash-of-Rust/actions/runs/40869085991) 的 Intel 安装后测试再次在冲突分域隔离后收到 VPN 答案，尽管默认解析器探针已通过、配置也显示本机地址。默认路径不能代表分域已切换：刷新时还需逐一检查前后有冲突的分域，所有探针共享超时预算，失败保持未就绪并限频重试。分域探针由随机标签和系统配置的冲突后缀组成，不含用户查询；切换期间探针可能到达原分域解析器，实际业务查询仍不得回退。本机转发器须终止符合受保护分域格式的延迟探针；原生集成继续验证实际 Claude 和诊断域名。

[预发布校验 41439610552](https://github.com/Fu021/Clash-of-Rust/actions/runs/41439610552) 在 Intel 上再次收到隔离前的 VPN 答案。仅收到本机探针请求仍不足以确认原生解析器已消费新路径：内部随机探针改用短 TTL 的本机回环答案，必须同时确认入口收到当前 A 查询及 DNS-SD 回调返回同名、同类型的预期答案，并消费当前结果批次。外部答案、错误、旧 nonce 或仅请求到达均不得标就绪；响应仍限内部名称，不转发到上游。保留原生业务查询和无外流断言，不重试业务查询到成功，真实设备仍需验收。

[CI 40516183395](https://github.com/Fu021/Clash-of-Rust/actions/runs/40516183395) 的两种 macOS 架构均因 `library 'dns_sd' not found` 在链接阶段失败，尚未执行原生回归。Apple 的 [DNS-SD 示例构建配置](https://github.com/apple-oss-distributions/mDNSResponder/blob/mDNSResponder-2200.140.11/Clients/Makefile)明确说明这些符号已包含在隐式链接的 `libSystem`；macOS 的 FFI 声明不应再要求独立的 `-ldns_sd`，也不应借此安装第三方替代库。移除多余链接声明后，仍须由同提交的两种原生架构 CI 验证链接和实际接管。

[不发布打包 40524184066](https://github.com/Fu021/Clash-of-Rust/actions/runs/40524184066) 的 Intel 安装后测试发现原生探针进入内核：仅在活动登记期间拦截，无法处理登记结束后的延迟或重发请求。转发器须始终在本机终止严格匹配内部随机名称格式的查询，只有当前登记的 nonce 才能确认就绪；旧探针不能确认下一轮检查，普通域名不能误拦截。UDP/TCP 共用同一判断，保留原生集成中的探针无外流断言；回归覆盖注销后重发、新旧探针交错、大小写及畸形请求，不以重跑或放宽断言掩盖竞态。

没有分域时也要保留 Quad100 接口，避免节点引导查询误走物理网卡。私有分域超过支持上限或格式无法处理时应拒绝接管，不能静默丢弃；单标签分域与重复服务器地址不应使有效分域失效。系统分域与保护域名或其上级域名重叠时先拒绝开启，包括非 Quad100 的实际分域；搜索后缀不等同于系统分域，但转换为私有解析策略的搜索后缀仍需检查重叠。公网保护线路与私有 DNS 不得互为失败回退。[Tailscale DNS 状态](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/cmd/tailscale/tsdnsjsonv0/status.go)还包含网络侧配置的私有分流，其[系统 DNS 配置](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/net/dns/manager.go)也可能直接使用私有上游地址；内核策略必须同时保留可见的实际私有上游，只为 Quad100 补充其默认接口，不能将该接口套给其他 DNS。不能假定 `scutil --dns` 一定列出了全部分域或都使用 Quad100。模拟解析与真实 Tailscale、接口变化及出口节点应分开验证。

不能简单自动关闭 Tailscale DNS 后继续假定 Quad100 保留全部私有转发：[Tailscale 配置生成](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/ipn/ipnlocal/node_backend.go)在 `CorpDNS` 关闭时不会继续加载网络下发的分域路由。替代方案必须保留这些路由及其变化，并验证故障和退出后的恢复。

[返回复盘索引](README.md)。当前用户行为见[Claude 保护](../claude-protection.md#dns-与-tailscale)；本文只保存失败证据与防回归约束。
