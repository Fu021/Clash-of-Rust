# 订阅与自动更新

## 客户端兼容与格式

桌面 GUI、`cor` 与 Android 共用订阅下载器。请求的 User-Agent 为 `clash.meta mihomo ClashMeta clash-verge ClashVerge ClashMetaForAndroid clash-of-rust`，本客户端标识不携带版本号；同时接受 YAML 响应。标识只参与服务端格式识别，不转换节点协议。

每条可用路线使用同一 User-Agent 请求，单条路线最多等待 30 秒；响应不兼容、HTTP 错误或网络故障可尝试其他可用代理及[内核直连入口](agent-failures/downloads.md#tun-下的更新直连)，不重复发送相同的客户端标识。直连回退不要求切换节点、规则模式或订阅；最后的系统网络仍可能受其他 VPN/TUN 影响。仅含复制同一 SS 配置的“不支持本机场协议”提示节点时拒绝更新；提示旁仍有真实节点或非空 provider 时保留配置，避免误拒可用订阅。

订阅 URL 和显式格式参数保持原样；若链接固定了旧客户端格式，应在机场取得 Mihomo/Clash Meta YAML 链接。最终仍只接受有效的 mihomo YAML，协商全部失败不会覆盖已有订阅或运行状态。

遇到节点链接、Base64、sing-box/Xray 等其他内核配置，或 `proxies` / `proxy-providers` 结构错误时，桌面与 Android 顶部通知会明确提示“订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅”；`cor` 返回相同原因及失败退出码。不转换订阅，也不显示响应原文、节点凭据或订阅令牌。其他路线获取有效 YAML 时不报错，HTTP 拒绝和网络故障不冒充格式问题。定时更新的格式错误同样通知并记日志，桌面按事件去重，刷新不重复弹出；订阅已改变或关闭定时更新时，丢弃迟到的格式通知。

实现参考 Clash Verge Rev 的[默认 UA 与自定义入口](https://github.com/clash-verge-rev/clash-verge-rev/blob/4337bd85758d54d7b9044b2da3c9ac1176eae1f3/src-tauri/src/utils/network.rs#L244)，问题来源见 [Issue #30](https://github.com/Fu021/Clash-of-Rust/issues/30)。服务端实际匹配规则及用户订阅仍需现场验证，不能仅凭截图认定故意封禁。

## 自动更新

桌面 GUI、`cor` 后台与 Android 共用 `subscription_update_interval_minutes`，默认 360 分钟，范围 0–10080；0 关闭。桌面和 Android 设置页可修改，CLI 使用 `cor config set subscription_update_interval_minutes 360`。该间隔与节点定时测速独立。

调度由当前数据目录的唯一后台所有者在运行时执行，GUI 重开或 CLI 附加不会启动第二份任务。每 30 秒检查一次，只选择 HTTP(S) 来源且已超过间隔的订阅；本地 YAML、默认直连、被 Claude 保护的订阅跳过。一次只更新一个，避免并发下载和重载争抢网络、内核与界面状态。

下载沿用订阅现有代理/直连、慢速与断流回退。成功后先校验 YAML，再在持有最新状态时核对订阅仍存在、来源未变、文件未被编辑、间隔仍开启且未被保护锁定；任一变化即丢弃结果。成功才更新内容、用量和更新时间，刷新界面并清理旧测速与检测结果；保留未应用的规则草稿。失败记录原因及“保留原配置与运行状态”，格式错误另按上文通知；不改内容、设置、草稿或检测结果。下次重试从本次尝试开始计时，不会持续请求。

权威实现见[共享调度](../crates/client-core/src/subscription/schedule.rs)、[桌面/CLI 后台](../crates/client-runtime/src/control/subscriptions.rs)与[Android 宿主](../apps/android/native/src/android.rs)。
