# macOS Tailscale DNS 共存

Tailscale 的独立补充分域解析器可以继续成为 macOS 全局 DNS，即使本应用已修改各网络服务的 DNS；因此不能把网络服务写入成功当成保护生效，也不应要求用户关闭 Tailscale DNS。[CI 40470411381](https://github.com/Fu021/Clash-of-Rust/actions/runs/40470411381) 证实独立临时解析器可能未被系统选中；[CI 40478327297](https://github.com/Fu021/Clash-of-Rust/actions/runs/40478327297) 与[故障实验 40481813828](https://github.com/Fu021/Clash-of-Rust/actions/runs/40481813828) 进一步证明默认解析器竞争和 DNS 故障回退可落到 VPN 或物理网络，不能仅调整优先级或仅验证首选路径。

修复采用可撤销的动态服务 DNS 覆盖：默认解析器统一指向助手独占的 `127.0.0.1:53`，仅转发到内核随机高端口，根分域中的私有/搜索后缀克隆为独立分域，保留原服务器、端口和接口，不复制 configd 生成的内部身份。覆盖 State 和 Setup 运行缓存而不写 SCPreferences；Apple 的 [configd 合并逻辑](https://github.com/apple-oss-distributions/configd/blob/main/Plugins/IPMonitor/ip_plugin.c)优先采用 Setup 服务器，不能只改 State。标准端口兼容不识别 `ServerPort` 的 `resolv.conf` 客户端，回归包括原生解析和 `dig` TCP/UDP；端口占用须先拒绝接管，转发应有并发与超时限制并随租约清理。SCDynamicStore 通知由同一助手线程串行处理重连与恢复；先保存新旧并集的恢复记录，再原子写入全部变化。恢复按托管地址/端口和分域字段组比较，保留外部修改，不重建已删除服务。运行中出现冲突时隔离为本机 DNS 并报告，不能以恢复 VPN 默认 DNS 处理错误；失败后限频重试。旧 journal 仍可清理，助手协议升级见[代理状态与安全恢复](desktop.md#系统代理状态与安全恢复)。

重连测试须等待助手明确报告分域冲突，而非任意检查错误；全局 DNS 的瞬态不匹配可能早于隔离完成。[CI 40507892365](https://github.com/Fu021/Clash-of-Rust/actions/runs/40507892365) 暴露了此等待条件过宽的问题。仍须断言实际原生解析到内核、VPN 未收到公共查询；失败时在清理前记录原生解析器，不能仅凭配置中旧地址消失判定隔离完成。

[CI 40510518605](https://github.com/Fu021/Clash-of-Rust/actions/runs/40510518605) 的重连查询仍返回 VPN 答案，而清理前 `scutil --dns` 已显示本机默认解析器；动态配置发布与 mDNSResponder 实际消费之间存在窗口。就绪确认须通过公开 DNS-SD 接口发出随机名称，并由本机转发器匹配当前探针后终止，不能只读配置、固定等待或重试业务域名到成功；不调用私有 ACK 接口冒充系统确认。探针不包含用户域名，每次使用新的随机末级标签以避开私有后缀及旧缓存；到达本机后不交给内核上游。限制单个活动探针、等待时限与取消资源，状态检查同时核对全部托管记录和当前计划，避免新变化沿用旧的成功状态。重连和冲突恢复先等待真实就绪，再断言实际业务查询及无公网回退。

[CI 40516183395](https://github.com/Fu021/Clash-of-Rust/actions/runs/40516183395) 的两种 macOS 架构均因 `library 'dns_sd' not found` 在链接阶段失败，尚未执行原生回归。Apple 的 [DNS-SD 示例构建配置](https://github.com/apple-oss-distributions/mDNSResponder/blob/mDNSResponder-2200.140.11/Clients/Makefile)明确说明这些符号已包含在隐式链接的 `libSystem`；macOS 的 FFI 声明不应再要求独立的 `-ldns_sd`，也不应借此安装第三方替代库。移除多余链接声明后，仍须由同提交的两种原生架构 CI 验证链接和实际接管。

[不发布打包 40524184066](https://github.com/Fu021/Clash-of-Rust/actions/runs/40524184066) 的 Intel 安装后测试发现原生探针进入内核：仅在活动登记期间拦截，无法处理登记结束后的延迟或重发请求。转发器须始终在本机终止严格匹配内部随机名称格式的查询，只有当前登记的 nonce 才能确认就绪；旧探针不能确认下一轮检查，普通域名不能误拦截。UDP/TCP 共用同一判断，保留原生集成中的探针无外流断言；回归覆盖注销后重发、新旧探针交错、大小写及畸形请求，不以重跑或放宽断言掩盖竞态。

没有分域时也要保留 Quad100 接口，避免节点引导查询误走物理网卡。私有分域超过支持上限或格式无法处理时应拒绝接管，不能静默丢弃；单标签分域与重复服务器地址不应使有效分域失效。系统分域与保护域名或其上级域名重叠时先拒绝开启，包括非 Quad100 的实际分域；搜索后缀不等同于系统分域，但转换为私有解析策略的搜索后缀仍需检查重叠。公网保护线路与私有 DNS 不得互为失败回退。[Tailscale DNS 状态](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/cmd/tailscale/tsdnsjsonv0/status.go)还包含网络侧配置的私有分流，其[系统 DNS 配置](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/net/dns/manager.go)也可能直接使用私有上游地址；内核策略必须同时保留可见的实际私有上游，只为 Quad100 补充其默认接口，不能将该接口套给其他 DNS。不能假定 `scutil --dns` 一定列出了全部分域或都使用 Quad100。模拟解析与真实 Tailscale、接口变化及出口节点应分开验证。

不能简单自动关闭 Tailscale DNS 后继续假定 Quad100 保留全部私有转发：[Tailscale 配置生成](https://github.com/tailscale/tailscale/blob/f3ec43d7dd/ipn/ipnlocal/node_backend.go)在 `CorpDNS` 关闭时不会继续加载网络下发的分域路由。替代方案必须保留这些路由及其变化，并验证故障和退出后的恢复。

[返回复盘索引](README.md)。当前用户行为见[Claude 保护](../claude-protection.md#dns-与-tailscale)；本文只保存失败证据与防回归约束。
