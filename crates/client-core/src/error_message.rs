use crate::{config::Language, messages};
use std::borrow::Cow;

fn legacy_message(message: &str, language: Language) -> Option<&'static str> {
    Some(match message {
        "请输入有效的网站地址" => messages::network_tool_invalid_website(language),
        "请输入域名或 HTTP/HTTPS 地址" => {
            messages::network_tool_invalid_domain_or_url(language)
        }
        "请输入 HTTP/HTTPS 地址" => messages::network_tool_invalid_url(language),
        "测试地址不支持嵌入用户名密码" => {
            messages::network_tool_url_credentials(language)
        }
        crate::protection::READ_ONLY_NOTICE => {
            "Disable Claude protection first; network changes and updates are locked while protection is active"
        }
        "订阅更新失败，已保留原配置与运行状态" => {
            "Subscription update failed; the previous configuration and running state were retained"
        }
        "订阅定时更新失败，已保留原配置与运行状态" => {
            "Scheduled subscription update failed; the previous configuration and running state were retained"
        }
        "订阅定时更新失败，已保留原配置" => {
            "Scheduled subscription update failed. Previous configuration retained"
        }
        "订阅定时下载失败" => "Scheduled subscription download failed",
        "订阅定时下载失败，已保留原配置与运行状态" => {
            "Scheduled subscription download failed. Configuration and running state retained"
        }
        "全部节点不可达，TUN 仍开启。请更换节点；更新失败后会自动尝试直连。" => {
            "All nodes are unreachable; TUN remains enabled. Choose another node; failed updates will retry using a direct connection."
        }
        "全部节点不可达，系统代理仍开启。请更换节点；更新失败后会自动尝试直连。" => {
            "All nodes are unreachable; system proxy remains enabled. Choose another node; failed updates will retry using a direct connection."
        }
        "订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅" => {
            "Incompatible subscription; use a Mihomo/Clash Meta YAML subscription"
        }
        "订阅服务器返回客户端兼容提示节点，请选择 Mihomo/Clash Meta YAML 订阅" => {
            "The subscription server returned a compatibility notice; use a Mihomo/Clash Meta YAML subscription"
        }
        "订阅下载失败" => "Subscription download failed",
        "订阅下载超时" => "Subscription download timed out",
        "下载响应不完整（HTTP 206）" => "Incomplete download response (HTTP 206)",
        "下载中断或超时" => "Download interrupted or timed out",
        "下载等待响应超时" => "Download response timed out",
        "下载超过 30 秒没有收到数据" => "No download data received for 30 seconds",
        "下载持续低于 32 KiB/s" => "Download stayed below 32 KiB/s",
        "下载速度持续下降" => "Download speed remained low after a drop",
        "检查更新失败" => "Update check failed",
        "下载更新失败" => "Update download failed",
        "更新失败" => "Update failed",
        "当前更新通道没有可安装的版本" => {
            "No installable version on the current update channel"
        }
        "更新通道已改变，请重新检查更新" => {
            "The update channel changed. Check for updates again"
        }
        "桌面客户端已变化，请重新执行更新" => {
            "The desktop client changed. Retry the update"
        }
        "当前平台不支持 Linux 终端安装交接" => {
            "Linux terminal installation handoff is unavailable on this platform"
        }
        "自动启动仅支持终端安装" => {
            "Automatic startup is available only for terminal installations"
        }
        "更新交接失败，已重新启动内核" => {
            "Update handoff failed and the core restarted"
        }
        "更新交接失败，重新启动内核也失败" => {
            "Update handoff failed and the core could not restart"
        }
        "更新交接失败，已保留旧版本" => {
            "Update handoff failed and the previous version was kept"
        }
        "无法创建更新连接" => "Could not create an update connection",
        "更新信息超过大小限制" => "Update information exceeds the size limit",
        "当前客户端版本号无效" => "The current client version is invalid",
        "安装包平台无效" => "Invalid package platform",
        "安装包大小无效" => "Invalid package size",
        "更新地址无效" => "Invalid update URL",
        "更新缓存路径无效" => "Invalid update cache path",
        "发布元数据与校验文件不一致" => {
            "Release metadata does not match the checksum file"
        }
        "更新校验文件格式无效" => "The update checksum file is invalid",
        "安装包大小与发布信息不一致" => "Package size does not match the release",
        "安装包超过预期大小" => "Package exceeds the expected size",
        "安装包大小或 SHA256 校验失败" => "Package size or SHA256 verification failed",
        "无法创建订阅下载连接" => "Could not create the subscription download connection",
        "订阅超过 10 MiB 限制" => "Subscription exceeds the 10 MiB limit",
        "订阅在下载期间已改变，已保留现有配置" => {
            "Subscription changed during download; the existing configuration was retained"
        }
        "订阅已修改，请刷新后重试" => "Subscription changed; refresh and try again",
        "订阅不存在" | "订阅已不存在" => "Subscription no longer exists",
        "当前订阅已丢失" => "The current subscription is missing",
        "尚未选择订阅" | "请先选择订阅" => "Select a subscription first",
        "请先选择订阅配置" | "请先导入并选择一个订阅配置" => {
            "Import and select a subscription first"
        }
        "配置名称与来源不能为空" => "Configuration name and source are required",
        "订阅已切换，请重新打开应用规则" => {
            "The subscription changed. Reopen application rules"
        }
        "当前系统不支持按应用指定出口，需要 Android 10 或更新版本" => {
            "Per-app outbound rules require Android 10 or later"
        }
        "应用规则标识无效" => "Invalid application rule ID",
        "规则名称需为 1～40 个字符，不含换行" => {
            "Rule names must contain 1–40 characters without line breaks"
        }
        "请至少选择一个应用" => "Select at least one app",
        "请选择规则组的出口" => "Choose an outbound for the rule group",
        "规则名称已存在，请换一个名称" => {
            "Rule name already exists. Choose another name"
        }
        "部分应用已属于其他规则组，请先编辑原规则组" => {
            "Some apps already belong to another rule group. Edit that group first"
        }
        "应用规则最多 256 条" => "Application rules are limited to 256 entries",
        "应用包名无效" => "Invalid application package name",
        "同一应用只能有一条应用规则" => "Each app can have only one application rule",
        "同一规则组的名称、出口和失效策略必须一致" => {
            "A rule group's name, outbound and fallback policy must match"
        }
        "应用规则目标无效" => "Invalid application rule target",
        "请选择策略组或节点" => "Select a proxy group or node",
        "节点身份无效" => "Invalid node identity",
        "配置无效" => "Invalid configuration",
        "策略组配置无效" => "Invalid proxy group configuration",
        "应用内部策略组名称冲突" => "Internal application proxy group name conflicts",
        "订阅 URL 无效" => "Invalid subscription URL",
        "订阅链接仅支持 HTTP 或 HTTPS" => "Subscription URLs must use HTTP or HTTPS",
        "请输入订阅 URL 或本地 YAML 路径" => {
            "Enter a subscription URL or local YAML path"
        }
        "默认直连配置不能编辑" => "The default direct configuration cannot be edited",
        "默认直连配置不能编辑，或订阅已不存在" => {
            "The default direct configuration cannot be edited, or the subscription no longer exists"
        }
        "默认直连配置不能删除" => "The default direct configuration cannot be deleted",
        "默认配置缺失" => "The default configuration is missing",
        "订阅名称不能为空" => "Subscription name cannot be empty",
        "订阅名称已存在" => "Subscription name already exists",
        "本地配置需要导入文件" => "Import a file for the local configuration",
        "本地配置没有更新地址" => "The local configuration has no update URL",
        "删除订阅失败，原运行配置恢复失败" => {
            "Subscription deletion failed; the previous running configuration could not be restored"
        }
        "删除订阅失败，磁盘恢复也失败" => {
            "Subscription deletion failed and the previous files could not be restored"
        }
        "YAML 配置格式错误" | "本地订阅 YAML 格式错误" => {
            "Invalid YAML configuration"
        }
        "订阅必须是 mihomo YAML 配置，而非节点链接或 Base64 文本" => {
            "The subscription must be a Mihomo YAML configuration"
        }
        "配置中没有 proxies 或 proxy-providers" => {
            "The configuration has no proxies or proxy-providers"
        }
        "订阅 sniffer 必须是配置对象" => "The subscription's sniffer must be a mapping",
        "订阅 sniff 必须是配置对象" => "The subscription's sniff must be a mapping",
        "订阅嗅探协议必须是配置对象" => {
            "The subscription's sniff protocol must be a mapping"
        }
        "订阅 dns 必须是配置对象" => "The subscription's DNS must be a mapping",
        "DNS 配置缺失" => "DNS configuration is missing",
        "fake-ip-filter-mode 必须是字符串" => "fake-ip-filter-mode must be a string",
        "fake-ip-filter 必须是列表" => "fake-ip-filter must be a list",
        "fake-ip-filter 条目必须是字符串" => "fake-ip-filter entries must be strings",
        "不支持的 fake-ip-filter-mode" => "Unsupported fake-ip-filter-mode",
        "rule-providers 必须是配置对象" => "rule-providers must be a mapping",
        "代理集合 override 必须是配置对象" => "Provider override must be a mapping",
        "代理集合 override-expr 必须是列表" => "Provider override-expr must be a list",
        "订阅 rules 必须是规则列表" => "The subscription's rules must be a list",
        "订阅规则必须是字符串" => "Subscription rules must be strings",
        "配置超过 10 MiB 限制" => "Configuration exceeds the 10 MiB limit",
        "设置已被另一客户端修改，请刷新后重试" => {
            "Settings changed in another client; refresh and try again"
        }
        "当前订阅已变化，请返回快捷规则页重新选择" => {
            "The active subscription changed; return to rule shortcuts and select again"
        }
        "订阅已包含此规则，无需重复添加；请返回刷新" => {
            "The subscription already contains this rule; return and refresh"
        }
        "规则已被另一客户端修改，草稿已保留；请刷新并检查修改后重新应用" => {
            "Rules changed in another client; the draft was retained. Refresh and review it before applying"
        }
        "已有相同的启用规则，请修改原规则或调整顺序" => {
            "An identical rule is already enabled; edit it or change its order"
        }
        "规则已变化，请重新选择" => "Rules changed; select again",
        "目标规则已变化，请重新拖动" => "The target rule changed; drag again",
        "只有自定义规则可以停用" => "Only custom rules can be disabled",
        "请先应用或放弃当前规则修改" => {
            "Apply or discard the current rule changes first"
        }
        "未找到规则；使用 rule list 查看稳定 ID" => {
            "Rule not found; use rule list to view stable IDs"
        }
        "规则序号超出范围" => "Rule number is out of range",
        "规则位置超出范围" => "Rule position is out of range",
        "快捷规则只适用于非内置订阅" => {
            "Rule shortcuts require a non-default subscription"
        }
        "未知快捷规则" => "Unknown rule shortcut",
        "订阅原规则只能调整顺序；快捷规则请使用 rule shortcut" => {
            "Subscription rules can only be reordered; use rule shortcut for shortcuts"
        }
        "自定义规则 ID 无效" => "Invalid custom rule ID",
        "未找到自定义规则" => "Custom rule not found",
        "规则应为单行文本，且不超过 8192 字节" => {
            "A rule must be a single line of at most 8192 bytes"
        }
        "规则括号不匹配" => "Rule parentheses do not match",
        "规则类型无效，应使用 DOMAIN、GEOIP、MATCH 等大写类型" => {
            "Invalid rule type; use uppercase types such as DOMAIN, GEOIP, or MATCH"
        }
        "规则格式应为 类型,内容,策略（MATCH 为 MATCH,策略）" => {
            "Use TYPE,CONTENT,POLICY for a rule, or MATCH,POLICY for MATCH"
        }
        "请选择规则策略" => "Select a rule policy",
        "规则内容不能为空" => "Rule content cannot be empty",
        "请先打开规则编辑器" => "Open the rule editor first",
        "逻辑规则等复杂类型请使用规则文本编辑。" => {
            "Use the text editor for logical and other complex rules."
        }
        "自定义规则不能超过 1000 条" => "Custom rules cannot exceed 1,000 entries",
        "插入位置应为正整数" => "The insertion position must be a positive integer",
        "请先开启 DNS 覆盖" => "Enable DNS override first",
        "请先开启 DNS 覆盖，再启用 Fake-IP" => "Enable DNS override before Fake-IP",
        "请先关闭 Claude 保护" => "Disable Claude protection first",
        "正在切换保护，等待接管验证完成" => {
            "Switching protection; waiting for verification"
        }
        "内核未运行，Claude 保护尚未生效" => {
            "The core is not running; Claude protection is not active"
        }
        "TUN 未连接，Claude 保护接管未就绪" => {
            "TUN is not connected; Claude protection is not ready"
        }
        "VPN 授权已取消或失效" => "VPN authorization was cancelled or expired",
        "正在确认保护连接与实时拦截状态" => {
            "Checking the protection connection and live blocking"
        }
        "暂时无法确认保护状态，正在等待重新验证" => {
            "Could not confirm protection status; waiting to retry"
        }
        "控制接口刷新超时" => "Control interface refresh timed out",
        "全部节点复测超时" => "Timed out rechecking all nodes",
        "节点复测失败" => "Node recheck failed",
        "测速失败" => "Speed test failed",
        "定时测速失败" => "Scheduled speed test failed",
        "节点状态复测未完成，已保留网络设置" => {
            "Node state recheck did not finish. Network settings retained"
        }
        "读取默认路由失败，网络恢复监控将在下次重试" => {
            "Could not read the default route. Network recovery monitoring will retry"
        }
        "网络变化后 Claude 保护的 TUN 覆盖不完整" => {
            "Claude protection TUN coverage is incomplete after the network changed"
        }
        "网络恢复未完成，将退避重试；保留原订阅与设置" => {
            "Network recovery incomplete. Retrying after a delay. Subscription and settings retained"
        }
        "手动网络刷新未完成" => "Manual network refresh incomplete",
        "网络刷新未完成，部分步骤可能已经生效，请检查状态" => {
            "Network refresh incomplete. Some changes may have applied. Check the current status"
        }
        "当前用户内核未运行" => "The current user's core is not running",
        "内核启动失败" => "Core startup failed",
        "内核已意外停止，请导出诊断后重试启动。" => {
            "The core stopped unexpectedly. Export diagnostics, then try starting it again."
        }
        "下载已取消，缓存清理失败" => "Download cancelled. Cache cleanup failed",
        "保护验证超时" => "Protection verification timed out",
        "确认保护节点超时" => "Timed out checking the protected node",
        "保护节点无效" | "Claude 保护节点无效" => "Invalid Claude protection node",
        "保护服务锁不可用" => "Protection service lock unavailable",
        "无法启动后台保护服务" => "Could not start the background protection service",
        "保护服务不可用" => "Protection service unavailable",
        "保护服务已停止" => "The protection service stopped",
        "保护服务未确认状态切换" => "The protection service did not confirm the change",
        "保护状态不可用" => "Protection status unavailable",
        "保护状态异常" => "Protection status error",
        "订阅已变化，请重新开启保护" => {
            "The subscription changed; enable protection again"
        }
        "VPN 授权入口不可用" => "VPN authorization is unavailable",
        "VPN 授权等待超时" => "VPN authorization timed out",
        "VPN 状态正在切换，请重试" => "The VPN state is changing. Try again.",
        "保护开启失败" => "Failed to enable protection",
        "保护恢复失败" => "Failed to restore protection",
        "关闭保护失败" => "Failed to disable protection",
        "保护切换超时" => "Protection change timed out",
        "保护验证失败" => "Protection verification failed",
        "保护回滚失败" => "Protection rollback failed",
        "无法确认 VPN 启动状态" => "Could not confirm VPN start status",
        "正在等待 Android VPN 服务结束原请求" => {
            "Waiting for the Android VPN service to finish the original request"
        }
        "正在确认当前代理节点…" => "Checking the current proxy node…",
        "正在等待 VPN 授权，当前网络设置保持不变…" => {
            "Waiting for VPN authorization; current network settings are unchanged…"
        }
        "正在关闭旧连接并验证保护节点与加密 DNS…" => {
            "Closing old connections and checking the protected node and encrypted DNS…"
        }
        "正在恢复保护 VPN…" => "Restoring the protected VPN…",
        "恢复原网络状态失败" => "Failed to restore the previous network state",
        "正在恢复原网络状态…" => "Restoring the previous network state…",
        "暂时无法读取保护状态" => "Protection status temporarily unavailable",
        "正在确认 VPN 提交…" => "Confirming the VPN commit…",
        "正在连接保护 VPN…" => "Connecting protected VPN…",
        "Claude 保护已开启" => "Claude protection enabled",
        "保护已保存，实时状态待确认" => "Protection saved; live status unconfirmed",
        "Claude 保护已关闭，VPN 状态待确认" => {
            "Claude protection disabled; VPN state unconfirmed"
        }
        "Claude 保护已关闭，原网络状态已恢复" => {
            "Claude protection disabled; previous network state restored"
        }
        "Claude 保护已关闭" => "Claude protection disabled",
        "控制接口暂不可用，实时拦截未确认；保护规则仍保留。" => {
            "Control interface unavailable; protection rules retained."
        }
        "实时拦截已停止，请重新开启 Claude 保护。" => {
            "Live blocking stopped; restart Claude protection."
        }
        "实时拦截停止" => "Live blocking stopped",
        "异常连接关闭失败" => "Failed to close an unexpected connection",
        "异常连接关闭失败，请检查内核状态。" => {
            "Failed to close an unexpected connection; check the core."
        }
        "连接已处理，但系统通知失败" => {
            "Connection handled; system notification failed"
        }
        "连接已处理，但系统通知任务失败" => {
            "Connection handled; system notification task failed"
        }
        "请先关闭 Claude 保护，再修改 Fake-IP" => {
            "Disable Claude protection before changing Fake-IP"
        }
        "请先关闭 Claude 保护，再编辑排除项" => {
            "Disable Claude protection before editing exclusions"
        }
        "请使用 Claude 保护开关" => "Use the Claude protection switch",
        "当前全局出口为 DIRECT，请先选择代理节点再开启 Claude 保护" => {
            "The global route is DIRECT; select a proxy node before enabling Claude protection"
        }
        "请先选择具体代理节点" | "请选择具体代理节点" => {
            "Select a specific proxy node"
        }
        "当前代理节点不存在" => "The current proxy node no longer exists",
        "Fake-IP 排除项过长" => "Fake-IP exclusion is too long",
        "Fake-IP 排除项只能包含域名、通配符和 . - _ + *" => {
            "Fake-IP exclusions may contain domains, wildcards, and . - _ + *"
        }
        "Fake-IP 排除项格式无效" => "Invalid Fake-IP exclusion",
        "这些排除项已存在" => "These exclusions already exist",
        "未找到排除项，请刷新后重试" => "Exclusion not found; refresh and try again",
        "排除项操作无效" => "Invalid exclusion operation",
        "字号比例应为 80–150%" => "Text scale must be 80–150%",
        "运行模式无效" => "Invalid routing mode",
        "控制端口与代理端口必须非零且不能相同" => {
            "Control and proxy ports must be nonzero and different"
        }
        "代理端口必须非零" => "Proxy port must be nonzero",
        "已有客户端使用这个数据目录，请关闭另一个实例" => {
            "Another client is using this data directory; close it first"
        }
        "本机控制端点必须归当前用户所有且仅当前用户可读写" => {
            "The current user must own the local control endpoint and have exclusive read and write access"
        }
        "已有不同配置的 CLI 登录服务，请先从原数据目录移除它" => {
            "A CLI login service is configured for another data directory. Remove it from that directory first"
        }
        "无法读取 CLI 登录自启配置" => {
            "Could not read the CLI login service configuration"
        }
        "无法读取设置文件" => "Could not read the settings file",
        "设置文件格式错误" => "Invalid settings file",
        "日志缓冲区不可用" => "Log buffer unavailable",
        "保存窗口状态任务失败" => "Window state save task failed",
        "无法读取本地设置" => "Could not read local settings",
        "本地设置格式错误" => "Invalid local settings",
        "本地设置文件过大" => "The local settings file is too large",
        "节点或策略组缺少名称" => "A node or proxy group is missing its name",
        "节点或策略组名称为空" => "A node or proxy group has an empty name",
        "无法读取订阅索引" => "Could not read the subscription index",
        "订阅索引格式错误" => "Invalid subscription index",
        "订阅索引或设置缺失，但订阅文件仍在；请恢复原文件后重试" => {
            "The subscription index or settings are missing while subscription files remain; restore the original files and try again"
        }
        "订阅索引为空，但订阅文件仍在；请恢复原索引后重试" => {
            "The subscription index is empty while subscription files remain; restore the original index and try again"
        }
        "上次配置提交尚未恢复，请重新启动客户端" => {
            "The previous configuration transaction has not been recovered; restart the client"
        }
        "此设置修改需要重载网络，请使用完整保存入口" => {
            "This change requires reloading the network; use the full save path"
        }
        "系统通知内容无效" => "Invalid system notification",
        "TUN 配置无效" => "Invalid TUN configuration",
        "系统没有 /dev/net/tun，请检查 TUN 驱动；容器或 WSL 需要提供 TUN 设备" => {
            "/dev/net/tun is missing. Check the TUN driver; containers and WSL need access to a TUN device"
        }
        "TUN 设备不是字符设备" => "The TUN device is not a character device",
        "无法访问 /dev/net/tun，请检查设备访问权限" => {
            "Cannot access /dev/net/tun; check device permissions"
        }
        "启动环境禁止获取网络权限，请从系统应用菜单启动客户端" => {
            "This environment blocks network permissions. Launch the client from the system app menu"
        }
        "启动环境限制了 TUN 网络权限，请检查容器或沙箱设置" => {
            "This environment restricts TUN permissions; check container or sandbox settings"
        }
        "无法启动 TUN 授权助手" => "Could not start the TUN authorization helper",
        "无法创建 TUN 会话进程" => messages::macos_tun_spawn_failed(language),
        "无法读取 TUN 会话启动状态" => {
            messages::macos_tun_start_status_unavailable(language)
        }
        "TUN 助手响应无效" => messages::macos_tun_invalid_helper_response(language),
        "TUN 授权未生效，请检查安装助手的网络权限与 DNS 助手" => {
            "TUN authorization did not take effect. Check the installation helper's network permissions and the DNS helper"
        }
        "TUN 授权已取消，当前代理模式保持不变" => {
            "TUN authorization cancelled; proxy mode is unchanged"
        }
        "无法获得 TUN 管理员授权，请检查 polkit 服务及终端认证" => {
            "Could not get administrator authorization for TUN; check polkit and terminal authentication"
        }
        "无法获得 TUN 授权，请检查 polkit 与桌面认证代理" => {
            "Could not authorize TUN; check polkit and the desktop authentication agent"
        }
        "TUN 授权失败" => "TUN authorization failed",
        "已取消系统授权，原设置未改变" => {
            messages::system_authorization_cancelled(language)
        }
        "管理员授权失败" => messages::system_authorization_failed(language),
        "无法启动系统授权" => messages::system_authorization_start_failed(language),
        "系统授权超时，已停止本次操作，请重试" => {
            messages::system_authorization_timed_out(language)
        }
        "DNS 配置无效" => "Invalid DNS configuration",
        "响应没有出口 IP" => "The response did not contain an exit IP address",
        "内核拒绝配置后恢复失败" => {
            "Could not restore the core after it rejected the configuration"
        }
        "配置写入失败，磁盘恢复也失败" => {
            "Configuration write failed; disk recovery also failed"
        }
        "配置保存失败，内核恢复也失败" => {
            "Configuration save failed; core recovery also failed"
        }
        "Geo 更新失败" => "Geo update failed",
        "Geo 数据正在更新，请稍候" => messages::geo_update_in_progress(language),
        "Geo 发布缺少 SHA256 校验信息" => messages::geo_release_checksum_missing(language),
        "Geo 数据校验信息无效" => messages::geo_release_verification_invalid(language),
        "Geo 下载源不是官方 HTTPS 发布" => messages::geo_release_source_invalid(language),
        "Geo 发布信息超过大小限制" => messages::geo_release_metadata_too_large(language),
        "Geo 更新失败，已恢复旧数据" => "Geo update failed; previous data restored",
        "Geo 更新失败，恢复旧文件也失败" => {
            "Geo update failed; previous files could not be restored"
        }
        "Geo 文件已恢复，但旧内核配置重载失败" => {
            "Geo files were restored, but the previous core configuration could not be reloaded"
        }
        "Geo 数据清单缺失" => "Geo data manifest is missing",
        "Geo 数据清单不完整" => "Geo data manifest is incomplete",
        "Geo 数据清单缺少必要文件" => "Geo data manifest is missing a required file",
        "Geo 数据清单格式无效" => "Invalid Geo data manifest",
        "新下载的 Geo 数据无效" => "Downloaded Geo data is invalid",
        "Geo 数据大小与发布信息不一致" => "Geo data size does not match the release",
        "Geo 数据超过预期大小" => "Geo data exceeds the expected size",
        "Geo 数据 SHA256/大小校验失败，已有数据保持不变" => {
            "Geo data SHA256 or size verification failed; existing data was retained"
        }
        "Geo 发布信息获取失败" => "Could not retrieve Geo release information",
        "Geo 下载失败" => "Geo download failed",
        "Geo 更新备份不完整，无法自动恢复" => {
            "Geo update backup is incomplete and cannot be restored automatically"
        }
        "Geo 更新备份无法读取，无法自动恢复" => {
            "Geo update backup cannot be read or restored automatically"
        }
        "Geo 更新备份记录无效，无法自动恢复" => {
            "Geo update backup record is invalid and cannot be restored automatically"
        }
        "Geo 更新备份校验失败，无法自动恢复" => {
            "Geo update backup verification failed and cannot be restored automatically"
        }
        "检测超时" => messages::ip_check_timed_out(language),
        "检测响应不是有效 JSON" => messages::ip_check_invalid_json(language),
        _ => return None,
    })
}

fn localize_routes(message: &str, language: Language) -> Option<String> {
    message
        .split('；')
        .map(|part| {
            let (label, detail) = part.split_once('：')?;
            let route = crate::network::localized_route_label(label, language)?;
            Some(messages::download_route_error(
                language,
                localize(detail, language),
                route,
            ))
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join(messages::download_route_error_separator(language)))
}

pub fn localize(message: &str, language: Language) -> Cow<'_, str> {
    if language == Language::SimplifiedChinese {
        return Cow::Borrowed(message);
    }
    if let Some(translated) = legacy_message(message, language) {
        return Cow::Borrowed(translated);
    }
    if let Some((reason, status)) = message
        .strip_prefix("TUN 会话准备失败：")
        .and_then(|detail| detail.strip_suffix("；请保留网络助手日志"))
        .and_then(|detail| detail.split_once("；子进程状态："))
    {
        match reason {
            "准备内核或通信接口超时" => {
                return messages::macos_tun_ready_timeout(language, status).into();
            }
            "子进程未返回有效就绪回执" => {
                return messages::macos_tun_invalid_ready_receipt(language, status).into();
            }
            _ => {}
        }
    }
    if let Some(detail) = message
        .strip_prefix("TUN 授权助手未就绪：")
        .and_then(|detail| detail.strip_suffix("；请检查安装包与系统日志"))
    {
        return messages::macos_tun_helper_not_ready(language, localize(detail, language)).into();
    }
    if let Some(asset) = message.strip_prefix("Geo 发布缺少 ")
        && crate::geodata::RELEASE_ASSETS
            .iter()
            .any(|(remote, _)| *remote == asset)
    {
        return messages::geo_release_asset_missing(language, asset).into();
    }
    for (prefix, summary) in [
        ("更新服务器返回 HTTP ", "The update server returned HTTP"),
        ("安装包下载返回 HTTP ", "Package download returned HTTP"),
    ] {
        if let Some(status) = message.strip_prefix(prefix).filter(|value| {
            let number = value.split_once(' ').map_or(*value, |(number, _)| number);
            reqwest::StatusCode::from_bytes(number.as_bytes()).is_ok()
        }) {
            return format!("{summary} {status}").into();
        }
    }
    if let Some((version, platform)) = message
        .strip_prefix("版本 ")
        .and_then(|value| value.strip_suffix('）'))
        .and_then(|value| value.split_once(" 缺少本平台安装包或校验文件（"))
        && semver::Version::parse(version).is_ok()
        && !platform.is_empty()
        && platform
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
    {
        return format!("Release {version} is missing the package or checksum for {platform}")
            .into();
    }
    if let Some(status) = message
        .strip_prefix("出口 IP 查询未返回有效结果（HTTP ")
        .and_then(|value| value.strip_suffix('）'))
        .filter(|value| reqwest::StatusCode::from_bytes(value.as_bytes()).is_ok())
    {
        return format!("Exit IP lookup returned no valid result (HTTP {status})").into();
    }
    if let Some(body) = message.strip_suffix("（已尝试可用代理和直连）")
        && let Some(details) = localize_routes(body, language)
    {
        return messages::download_route_error_tried_all(language, details).into();
    }
    if let Some(details) = localize_routes(message, language) {
        return details.into();
    }
    for delimiter in ["：", ": "] {
        if let Some((prefix, detail)) = message.split_once(delimiter)
            && let Some(translated) = legacy_message(prefix, language)
        {
            if matches!(prefix, "保护回滚失败" | "无法确认 VPN 启动状态")
                && let Some((reason, external_detail)) = detail.split_once('\n')
            {
                return format!(
                    "{translated}: {}\n{external_detail}",
                    localize(reason, language)
                )
                .into();
            }
            return format!("{translated}: {}", localize(detail, language)).into();
        }
    }
    for (prefix, label) in [
        ("位置应为 1–", "Position"),
        ("插入位置应为 1-", "Insertion position"),
        ("移动位置应为 1-", "Move position"),
    ] {
        if let Some(limit) = message.strip_prefix(prefix)
            && !limit.is_empty()
            && limit.bytes().all(|byte| byte.is_ascii_digit())
        {
            return format!("{label} must be between 1 and {limit}").into();
        }
    }
    for (prefix, label) in [
        ("Fake-IP 排除项最多 ", "Fake-IP exclusions"),
        ("Fake-IP 自定义排除项最多 ", "Custom Fake-IP exclusions"),
    ] {
        if let Some(limit) = message
            .strip_prefix(prefix)
            .and_then(|value| value.strip_suffix(" 个"))
            && !limit.is_empty()
            && limit.bytes().all(|byte| byte.is_ascii_digit())
        {
            return format!("{label} are limited to {limit} entries").into();
        }
    }
    Cow::Borrowed(message)
}

pub fn localize_diagnostic(message: &str, language: Language) -> Cow<'_, str> {
    if language == Language::SimplifiedChinese {
        return Cow::Borrowed(message);
    }
    let (summary, detail) = message
        .split_once(": ")
        .map_or((message, None), |(summary, detail)| (summary, Some(detail)));
    let translated = match summary {
        "当前用户后台未运行" => "The current user's backend is not running",
        "连接当前用户后台超过 5 秒；可用 --offline 保存本地现场" => {
            "Connecting to the current user's backend timed out after 5 seconds (use --offline to save local diagnostics)"
        }
        "无法确定应用数据目录" => "Could not determine the application data directory",
        "本机控制端点文件无效" => "Invalid local control endpoint file",
        "本机控制端点文件过大" => "Local control endpoint file is too large",
        "本机控制协议不兼容，请升级客户端或退出旧版 GUI／cor 后台后重试" => {
            "Incompatible local control protocol. Update the client or quit the older GUI or cor backend, then try again"
        }
        "等待后台退出时读取控制端点失败" => {
            "Could not read the control endpoint while waiting for the backend to exit"
        }
        "等待后台退出超时" => "Timed out waiting for the backend to exit",
        "无法连接本机客户端后台" => "Could not connect to the local client backend",
        "命令超过 8 MiB 限制" => "Command exceeds the 8 MiB limit",
        "无法检查后台的占用锁" => "Could not check the backend lock",
        "后台响应超过 32 MiB 限制" => "Backend response exceeds the 32 MiB limit",
        "单项检测超过 15 秒" => "Probe timed out after 15 seconds",
        "DNS 查询请输入域名，例如 github.com" => {
            "Enter a domain name for DNS lookup, for example github.com"
        }
        "未查到目标地址" => "No addresses found for the target",
        "请输入有效的网站地址"
        | "请输入域名或 HTTP/HTTPS 地址"
        | "请输入 HTTP/HTTPS 地址"
        | "测试地址不支持嵌入用户名密码" => {
            legacy_message(summary, language).unwrap()
        }
        "IP 查询响应过大" => "IP lookup response is too large",
        "IP 查询响应不是有效 UTF-8 文本" => "IP lookup response is not valid UTF-8 text",
        "IP 查询响应不是有效 IP 地址或包含有效 ip 字段的 JSON" => {
            "IP lookup response must be a valid IP address or JSON with a valid ip field"
        }
        _ => {
            if let Some(status) = message
                .strip_prefix("IP 查询返回 HTTP ")
                .and_then(|status| status.strip_suffix("，未获得有效出口地址"))
                && reqwest::StatusCode::from_bytes(status.as_bytes())
                    .is_ok_and(|code| !code.is_success())
            {
                return format!("IP lookup returned HTTP {status} without a valid exit address")
                    .into();
            }
            return localize(message, language);
        }
    };
    match detail {
        Some(detail) => format!("{translated}: {detail}").into(),
        None => Cow::Borrowed(translated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_input_errors_use_the_same_messages_in_reports_and_direct_errors() {
        let malformed = crate::probe::diagnostic_url("").unwrap_err();
        let credentials =
            crate::probe::diagnostic_url("https://fixture@example.invalid").unwrap_err();
        for (raw, expected) in [
            (malformed.to_string(), "Enter a valid website address"),
            (credentials.to_string(), "Enter a domain or HTTP/HTTPS URL"),
            (
                credentials.root_cause().to_string(),
                "Test URLs cannot include credentials",
            ),
        ] {
            assert_eq!(localize(&raw, Language::English), expected);
            assert_eq!(localize_diagnostic(&raw, Language::English), expected);
            assert_eq!(localize(&raw, Language::SimplifiedChinese), raw);
        }
    }

    #[test]
    fn actual_configuration_and_rule_errors_follow_the_display_language() {
        let subscription = crate::config::parse_profile("name: example").unwrap_err();
        let rule = crate::rule_manager::parts("DOMAIN,,DIRECT").err().unwrap();
        let malformed_sniffer = crate::config::runtime_config(
            "proxies: []\nsniffer: invalid\n",
            &crate::config::Settings::default(),
        )
        .unwrap_err();
        for (error, expected) in [
            (
                subscription.to_string(),
                "The configuration has no proxies or proxy-providers",
            ),
            (
                malformed_sniffer.to_string(),
                "The subscription's sniffer must be a mapping",
            ),
            (rule.to_string(), "Rule content cannot be empty"),
            (
                "自定义规则不能超过 1000 条".into(),
                "Custom rules cannot exceed 1,000 entries",
            ),
        ] {
            assert_eq!(localize(&error, Language::English), expected);
            assert_eq!(localize(&error, Language::SimplifiedChinese), error);
        }
    }

    #[test]
    fn malformed_subscription_dns_errors_keep_provider_details() {
        for (raw, expected) in [
            (
                "proxies: []\ndns: invalid\n",
                "The subscription's DNS must be a mapping",
            ),
            (
                "proxies: []\ndns: {fake-ip-filter-mode: 42}\n",
                "fake-ip-filter-mode must be a string",
            ),
            (
                "proxies: []\ndns: {fake-ip-filter: invalid}\n",
                "fake-ip-filter must be a list",
            ),
            (
                "proxies: []\ndns: {fake-ip-filter-mode: provider-value}\n",
                "Unsupported fake-ip-filter-mode: provider-value",
            ),
        ] {
            let error = crate::config::runtime_config(raw, &crate::config::Settings::default())
                .unwrap_err()
                .to_string();
            assert_eq!(localize(&error, Language::English), expected);
            assert_eq!(localize(&error, Language::SimplifiedChinese), error);
        }
    }

    #[test]
    fn update_handoff_errors_describe_the_recovery_and_keep_external_details() {
        for (original, translated) in [
            (
                "更新通道已改变，请重新检查更新",
                "The update channel changed. Check for updates again",
            ),
            (
                "更新交接失败，已重新启动内核: native error",
                "Update handoff failed and the core restarted: native error",
            ),
            (
                "更新交接失败，重新启动内核也失败：native error 原文",
                "Update handoff failed and the core could not restart: native error 原文",
            ),
            (
                "更新交接失败，已保留旧版本: native error",
                "Update handoff failed and the previous version was kept: native error",
            ),
        ] {
            assert_eq!(localize(original, Language::English), translated);
            assert_eq!(localize(original, Language::SimplifiedChinese), original);
        }
    }

    #[test]
    fn known_contexts_translate_without_changing_unknown_details() {
        let message = "订阅更新失败，已保留原配置与运行状态：YAML 配置格式错误: 東京；provider";
        assert_eq!(
            localize(message, Language::English),
            "Subscription update failed; the previous configuration and running state were retained: Invalid YAML configuration: 東京；provider"
        );
        assert_eq!(
            localize("provider: 规则内容不能为空；東京", Language::English),
            "provider: 规则内容不能为空；東京"
        );
        assert_eq!(
            localize("位置应为 1–12", Language::English),
            "Position must be between 1 and 12"
        );
        for (message, expected) in [
            (
                "订阅索引或设置缺失，但订阅文件仍在；请恢复原文件后重试",
                "The subscription index or settings are missing while subscription files remain; restore the original files and try again",
            ),
            (
                "订阅索引为空，但订阅文件仍在；请恢复原索引后重试",
                "The subscription index is empty while subscription files remain; restore the original index and try again",
            ),
            (
                "上次配置提交尚未恢复，请重新启动客户端",
                "The previous configuration transaction has not been recovered; restart the client",
            ),
            (
                "此设置修改需要重载网络，请使用完整保存入口",
                "This change requires reloading the network; use the full save path",
            ),
            (
                "节点复测失败：全部节点复测超时",
                "Node recheck failed: Timed out rechecking all nodes",
            ),
            (
                "恢复原网络状态失败：保护验证超时",
                "Failed to restore the previous network state: Protection verification timed out",
            ),
        ] {
            assert_eq!(localize(message, Language::English), expected);
            assert_eq!(localize(message, Language::SimplifiedChinese), message);
        }
    }

    #[test]
    fn nested_failures_keep_external_details() {
        for (message, expected) in [
            (
                "Geo 更新失败：Geo 更新失败，已恢复旧数据: 東京；provider",
                "Geo update failed: Geo update failed; previous data restored: 東京；provider",
            ),
            (
                "Geo 更新失败：Geo 发布缺少 geoip.dat",
                "Geo update failed: The Geo release is missing geoip.dat",
            ),
            (
                "Geo 更新失败：Geo 数据校验信息无效",
                "Geo update failed: Invalid Geo verification data",
            ),
            (
                "管理员授权失败：操作系统错误 (-60005)",
                "Administrator authorization failed: 操作系统错误 (-60005)",
            ),
            (
                "已取消系统授权，原设置未改变",
                "System authorization cancelled. Settings were kept.",
            ),
            (
                "订阅定时更新失败，已保留原配置与运行状态：订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅",
                "Scheduled subscription update failed; the previous configuration and running state were retained: Incompatible subscription; use a Mihomo/Clash Meta YAML subscription",
            ),
        ] {
            assert_eq!(localize(message, Language::English), expected);
            assert_eq!(localize(message, Language::SimplifiedChinese), message);
        }
    }

    #[test]
    fn download_route_failures_follow_the_display_language() {
        for (message, expected) in [
            (
                "内核代理：下载响应不完整（HTTP 206）；直连：下载响应不完整（HTTP 206）（已尝试可用代理和直连）",
                "Core proxy: Incomplete download response (HTTP 206); Direct: Incomplete download response (HTTP 206) (tried available proxies and direct connections)",
            ),
            (
                "Geo 更新失败：Geo 发布信息获取失败：系统网络：下载响应不完整（HTTP 206）",
                "Geo update failed: Could not retrieve Geo release information: System network: Incomplete download response (HTTP 206)",
            ),
            (
                "下载更新失败：系统代理：下载中断或超时: 東京",
                "Update download failed: System proxy: Download interrupted or timed out: 東京",
            ),
            (
                "系统代理：安装包下载返回 HTTP 404 Not Found；直连：安装包大小或 SHA256 校验失败（已尝试可用代理和直连）",
                "System proxy: Package download returned HTTP 404 Not Found; Direct: Package size or SHA256 verification failed (tried available proxies and direct connections)",
            ),
        ] {
            assert_eq!(localize(message, Language::English), expected);
            assert_eq!(localize(message, Language::SimplifiedChinese), message);
        }
        let external = "外部提供方：東京；直连：下载响应不完整（HTTP 206）";
        assert_eq!(localize(external, Language::English), external);
    }

    #[test]
    fn protection_errors_translate_only_the_application_prefix() {
        let message = "连接已处理，但系统通知失败：通知已禁用 · example";
        assert_eq!(
            localize(message, Language::English),
            "Connection handled; system notification failed: 通知已禁用 · example"
        );
        assert_eq!(localize(message, Language::SimplifiedChinese), message);
        assert_eq!(
            localize("正在确认保护连接与实时拦截状态", Language::English),
            "Checking the protection connection and live blocking"
        );
    }

    #[test]
    fn platform_permission_failures_keep_operating_system_details() {
        let endpoint = "本机控制端点必须归当前用户所有且仅当前用户可读写";
        assert_eq!(
            localize_diagnostic(endpoint, Language::English),
            localize(endpoint, Language::English)
        );
        assert_ne!(localize(endpoint, Language::English), endpoint);
        let tun = "无法访问 /dev/net/tun，请检查设备访问权限: 系统返回 EPERM";
        assert_eq!(
            localize(tun, Language::English),
            "Cannot access /dev/net/tun; check device permissions: 系统返回 EPERM"
        );
        assert_eq!(localize(tun, Language::SimplifiedChinese), tun);
    }

    #[test]
    fn macos_tun_startup_errors_keep_helper_evidence() {
        for (message, expected) in [
            (
                "TUN 会话准备失败：准备内核或通信接口超时；子进程状态：Ok(None)；请保留网络助手日志",
                "Timed out starting the TUN core or socket. Process status: Ok(None). Keep the network helper logs.",
            ),
            (
                "TUN 会话准备失败：子进程未返回有效就绪回执；子进程状态：Ok(Some(ExitStatus(unix_wait_status(256))))；请保留网络助手日志",
                "The TUN process did not report readiness. Process status: Ok(Some(ExitStatus(unix_wait_status(256)))). Keep the network helper logs.",
            ),
            (
                "无法创建 TUN 会话进程: No such file or directory (os error 2)",
                "Could not start the TUN process: No such file or directory (os error 2)",
            ),
            (
                "TUN 授权助手未就绪：TUN 助手响应无效；请检查安装包与系统日志",
                "The TUN helper is not ready. Invalid response from the TUN helper. Check the installer and system logs.",
            ),
        ] {
            assert_eq!(localize(message, Language::English), expected);
            assert_eq!(localize(message, Language::SimplifiedChinese), message);
        }
    }
}
