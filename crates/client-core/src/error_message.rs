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
        "默认直连配置不能删除" => "The default direct configuration cannot be deleted",
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
        "Fake-IP 排除项过长" => "Fake-IP exclusion is too long",
        "Fake-IP 排除项只能包含域名、通配符和 . - _ + *" => {
            "Fake-IP exclusions may contain domains, wildcards, and . - _ + *"
        }
        "Fake-IP 排除项格式无效" => "Invalid Fake-IP exclusion",
        "这些排除项已存在" => "These exclusions already exist",
        "未找到排除项，请刷新后重试" => "Exclusion not found; refresh and try again",
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
    if let Some(limit) = message.strip_prefix("位置应为 1–")
        && !limit.is_empty()
        && limit.bytes().all(|byte| byte.is_ascii_digit())
    {
        return format!("Position must be between 1 and {limit}").into();
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
}
