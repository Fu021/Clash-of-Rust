# 订阅与自动更新

## 客户端兼容与格式

桌面 GUI、`cor` 与 Android 共用订阅下载器。请求的 User-Agent 为 `clash.meta mihomo ClashMeta clash-verge ClashVerge ClashMetaForAndroid clash-of-rust`，本客户端标识不携带版本号；同时接受 YAML 响应。标识只参与服务端格式识别，不转换节点协议。

每条可用路线使用同一 User-Agent 请求，单条路线最多等待 30 秒；响应不兼容、HTTP 错误或网络故障可尝试其他可用代理及[内核直连入口](agent-failures/downloads.md#tun-下的更新直连)，不重复发送相同的客户端标识。直连回退不要求切换节点、规则模式或订阅；最后的系统网络仍可能受其他 VPN/TUN 影响。仅含复制同一 SS 配置的“不支持本机场协议”提示节点时拒绝更新；提示旁仍有真实节点或非空 provider 时保留配置，避免误拒可用订阅。

订阅 URL 和显式格式参数保持原样；若链接固定了旧客户端格式，应在机场取得 Mihomo/Clash Meta YAML 链接。最终仍只接受有效的 mihomo YAML，协商全部失败不会覆盖已有订阅或运行状态。

遇到节点链接、Base64、sing-box/Xray 等其他内核配置，或 `proxies` / `proxy-providers` 结构错误时，桌面与 Android 顶部通知会明确提示“订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅”；`cor` 返回相同原因及失败退出码。不转换订阅，也不显示响应原文、节点凭据或订阅令牌。其他路线获取有效 YAML 时不报错，HTTP 拒绝和网络故障不冒充格式问题。定时更新的格式错误同样通知并记日志，桌面按事件去重，刷新不重复弹出；订阅已改变或关闭定时更新时，丢弃迟到的格式通知。

实现参考 Clash Verge Rev 的[默认 UA 与自定义入口](https://github.com/clash-verge-rev/clash-verge-rev/blob/4337bd85758d54d7b9044b2da3c9ac1176eae1f3/src-tauri/src/utils/network.rs#L244)，问题来源见 [Issue #30](https://github.com/Fu021/Clash-of-Rust/issues/30)。服务端实际匹配规则及用户订阅仍需现场验证，不能仅凭截图认定故意封禁。

## 订阅操作

桌面宫格、列表保留“使用／使用中”“更新”“编辑”和“详情”入口，详情内保留“删除”。移动端宫格、列表常显“使用／使用中”和“详情”，展开详情后统一显示“更新”“编辑”“删除”，减少卡片占用。远程来源统一标为“订阅”，文件路径或空来源标为“本地”，不按来源是否为空区分。本地导入的订阅可以编辑、删除，“更新”置灰；改名读取已保存的 YAML，不要求原始导入文件仍存在。内置默认直连的“更新”“编辑”“删除”均置灰，后端仍拒绝编辑和删除默认直连。各端同一区域内的不可用操作不隐藏，避免不同类型订阅的按钮位置跳动。

当前订阅用强调色显示“使用中”，不能重复点击；其他不可用按钮置灰。Claude 保护及切换期间继续禁止修改，详情仍可查看。本地导入配置改名保留已有 YAML；编辑来源不等于已经成功更新配置，仍按各端既有校验和保存流程处理。

## 自动更新

桌面 GUI、`cor` 后台与 Android 共用 `subscription_update_interval_minutes`，默认 360 分钟，范围 0–10080；0 关闭。桌面和 Android 设置页可修改，CLI 使用 `cor config set subscription_update_interval_minutes 360`。该间隔与节点定时测速独立。

桌面调度由当前数据目录的后台所有者执行，GUI 重开或 CLI 附加不会启动第二份任务。Android 调度属于 Activity 创建的移动会话：界面隐藏而会话仍在时可继续检查，Activity 销毁时取消会话并等待其运行时与写入任务退出，再允许新 Activity 打开同一配置。VPN 由独立服务维持，不随界面会话停止；重新打开界面后恢复定时检查，不提供无 Activity 时的独立订阅调度。

每 30 秒检查一次，只选择 HTTP(S) 来源且已超过间隔的订阅；本地 YAML 和默认直连跳过。Claude 保护及切换期间整个客户端只读，所有订阅的定时和手动更新暂停，恢复后重新等待间隔；桌面定时和手动下载共用门禁，忙时跳过本轮检查而不消耗重试间隔，停止内核或关闭后台会取消下载。Android 同一会话处理手动订阅操作前取消定时任务，并丢弃已排队的旧结果；界面退出路径即使遇到错误，也通过会话所有权回收后台任务。

下载沿用订阅现有代理/直连、慢速与断流回退。定时下载成功后先校验 YAML，再核对当前所有者已载入的订阅、来源、开关和保护状态，以及文件是否被编辑；任一变化即丢弃结果。成功才更新内容、用量和更新时间，刷新界面并清理旧测速与检测结果；保留未应用的规则草稿。失败记录原因及“保留原配置与运行状态”，格式错误另按上文通知；不改内容、设置、草稿或检测结果。下次重试从本次尝试开始计时，不会持续请求。跨 Activity 取消回归与真实设备验证分别记录于[Android 生命周期复盘](agent-failures/android.md)，实现接入不代表已经完成设备验收。

权威实现见[共享调度](../crates/client-core/src/subscription/schedule.rs)、[桌面/CLI 后台](../crates/client-runtime/src/control/subscriptions.rs)与[Android 宿主](../apps/android/native/src/android.rs)。
