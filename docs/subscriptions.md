# 订阅自动更新

桌面 GUI、`cor` 后台与 Android 共用 `subscription_update_interval_minutes`，默认 360 分钟，范围 0–10080；0 关闭。桌面和 Android 设置页可修改，CLI 使用 `cor config set subscription_update_interval_minutes 360`。该间隔与节点定时测速独立。

调度由当前数据目录的唯一后台所有者在运行时执行，GUI 重开或 CLI 附加不会启动第二份任务。每 30 秒检查一次，只选择 HTTP(S) 来源且已超过间隔的订阅；本地 YAML、默认直连、被 Claude 保护的订阅跳过。一次只更新一个，避免并发下载和重载争抢网络、内核与界面状态。

下载沿用订阅现有代理/直连、慢速与断流回退。成功后先校验 YAML，再在持有最新状态时核对订阅仍存在、来源未变、文件未被编辑、间隔仍开启且未被保护锁定；任一变化即丢弃结果。成功才更新内容、用量和更新时间，刷新界面并清理旧测速与检测结果；保留未应用的规则草稿。失败只记录“保留原配置与运行状态”，不改内容、设置、草稿或检测结果。下次重试从本次尝试开始计时，不会持续请求。

权威实现见[共享调度](../crates/client-core/src/subscription/schedule.rs)、[桌面/CLI 后台](../crates/client-runtime/src/control/subscriptions.rs)与[Android 宿主](../apps/android/native/src/android.rs)。
