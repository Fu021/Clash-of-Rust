use crate::config::Language;
use std::borrow::Cow;

fn english(message: &str) -> Option<&'static str> {
    Some(match message {
        crate::protection::READ_ONLY_NOTICE => {
            "Disable Claude protection first; network changes and updates are locked while protection is active"
        }
        "订阅更新失败，已保留原配置与运行状态" => {
            "Subscription update failed; the previous configuration and running state were retained"
        }
        "订阅定时更新失败，已保留原配置与运行状态" => {
            "Scheduled subscription update failed; the previous configuration and running state were retained"
        }
        "订阅格式不兼容，请选择 Mihomo/Clash Meta YAML 订阅" => {
            "Incompatible subscription; use a Mihomo/Clash Meta YAML subscription"
        }
        "订阅服务器返回客户端兼容提示节点，请选择 Mihomo/Clash Meta YAML 订阅" => {
            "The subscription server returned a compatibility notice; use a Mihomo/Clash Meta YAML subscription"
        }
        "订阅下载失败" => "Subscription download failed",
        "订阅下载超时" => "Subscription download timed out",
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
        "YAML 配置格式错误" | "本地订阅 YAML 格式错误" => {
            "Invalid YAML configuration"
        }
        "订阅必须是 mihomo YAML 配置，而非节点链接或 Base64 文本" => {
            "The subscription must be a Mihomo YAML configuration"
        }
        "配置中没有 proxies 或 proxy-providers" => {
            "The configuration has no proxies or proxy-providers"
        }
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
        "插入位置应为正整数" => "The insertion position must be a positive integer",
        "请先开启 DNS 覆盖" => "Enable DNS override first",
        "请先开启 DNS 覆盖，再启用 Fake-IP" => "Enable DNS override before Fake-IP",
        "请先关闭 Claude 保护" => "Disable Claude protection first",
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
        "无法读取设置文件" => "Could not read the settings file",
        "设置文件格式错误" => "Invalid settings file",
        "无法读取订阅索引" => "Could not read the subscription index",
        "订阅索引格式错误" => "Invalid subscription index",
        "系统通知内容无效" => "Invalid system notification",
        "TUN 配置无效" => "Invalid TUN configuration",
        "DNS 配置无效" => "Invalid DNS configuration",
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
        _ => return None,
    })
}

pub fn localize(message: &str, language: Language) -> Cow<'_, str> {
    if language != Language::English {
        return Cow::Borrowed(message);
    }
    if let Some(translated) = english(message) {
        return Cow::Borrowed(translated);
    }
    for delimiter in ["：", ": "] {
        if let Some((prefix, detail)) = message.split_once(delimiter)
            && let Some(translated) = english(prefix)
        {
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
    Cow::Borrowed(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_configuration_and_rule_errors_follow_the_display_language() {
        let subscription = crate::config::parse_profile("name: example").unwrap_err();
        let rule = crate::rule_manager::parts("DOMAIN,,DIRECT").err().unwrap();
        for (error, expected) in [
            (
                subscription.to_string(),
                "The configuration has no proxies or proxy-providers",
            ),
            (rule.to_string(), "Rule content cannot be empty"),
        ] {
            assert_eq!(localize(&error, Language::English), expected);
            assert_eq!(localize(&error, Language::SimplifiedChinese), error);
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
    }

    #[test]
    fn nested_geo_and_scheduled_subscription_failures_keep_external_details() {
        for (message, expected) in [
            (
                "Geo 更新失败：Geo 更新失败，已恢复旧数据: 東京；provider",
                "Geo update failed: Geo update failed; previous data restored: 東京；provider",
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
}
