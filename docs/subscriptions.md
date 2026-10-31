# 订阅与自动更新

## 客户端兼容与格式

桌面 GUI、`cor` 与 Android 共用订阅下载器。请求声明 Clash Meta/mihomo 能力，并携带 `clash-verge`、`ClashVerge`、`ClashMetaForAndroid`、`v2rayN`、`v2rayNG` 及本客户端标识，兼容按客户端名称下发配置的服务端；同时接受 YAML 响应。标识只参与格式协商，不转换节点协议。

部分面板优先匹配 v2rayN/NG，会返回节点链接或 Base64。响应格式不符、收到 HTTP 400/403/406/415/422，或只有复制同一 SS 配置的“不支持本机场协议”提示节点时，同一路线最多再发一次仅含 Clash Meta/mihomo 与自身标识的请求；两次共用 30 秒上限。认证失败、限流、服务端故障或网络错误沿用路线回退，不额外重试 UA。提示旁仍有真实节点或非空 provider 时保留配置，避免误拒可用订阅。

订阅 URL 和显式格式参数保持原样；若链接固定了旧客户端格式，应在机场取得 Mihomo/Clash Meta YAML 链接。最终仍只接受有效的 mihomo YAML，协商全部失败不会覆盖已有订阅或运行状态。

实现参考 Clash Verge Rev 的[默认 UA 与自定义入口](https://github.com/clash-verge-rev/clash-verge-rev/blob/4337bd85758d54d7b9044b2da3c9ac1176eae1f3/src-tauri/src/utils/network.rs#L244)，问题来源见 [Issue #30](https://github.com/Fu021/Clash-of-Rust/issues/30)。服务端实际匹配规则及用户订阅仍需现场验证，不能仅凭截图认定故意封禁。

## 自动更新

桌面 GUI、`cor` 后台与 Android 共用 `subscription_update_interval_minutes`，默认 360 分钟，范围 0–10080；0 关闭。桌面和 Android 设置页可修改，CLI 使用 `cor config set subscription_update_interval_minutes 360`。该间隔与节点定时测速独立。

调度由当前数据目录的唯一后台所有者在运行时执行，GUI 重开或 CLI 附加不会启动第二份任务。每 30 秒检查一次，只选择 HTTP(S) 来源且已超过间隔的订阅；本地 YAML、默认直连、被 Claude 保护的订阅跳过。一次只更新一个，避免并发下载和重载争抢网络、内核与界面状态。

下载沿用订阅现有代理/直连、慢速与断流回退。成功后先校验 YAML，再在持有最新状态时核对订阅仍存在、来源未变、文件未被编辑、间隔仍开启且未被保护锁定；任一变化即丢弃结果。成功才更新内容、用量和更新时间，刷新界面并清理旧测速与检测结果；保留未应用的规则草稿。失败只记录“保留原配置与运行状态”，不改内容、设置、草稿或检测结果。下次重试从本次尝试开始计时，不会持续请求。

权威实现见[共享调度](../crates/client-core/src/subscription/schedule.rs)、[桌面/CLI 后台](../crates/client-runtime/src/control/subscriptions.rs)与[Android 宿主](../apps/android/native/src/android.rs)。
