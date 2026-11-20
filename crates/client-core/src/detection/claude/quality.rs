//! Bounded provider observations for each measured exit address.
use super::{Evidence, Group, Signal, response_bytes};
use futures_util::future::join_all;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::net::IpAddr;

#[derive(Clone, Copy)]
enum Provider {
    IpQuery,
    IpLogs,
    Layer3,
}

impl Provider {
    fn name(self) -> &'static str {
        match self {
            Self::IpQuery => "IPQuery",
            Self::IpLogs => "IPLogs",
            Self::Layer3 => "Layer3 Intel",
        }
    }

    fn supports(self, ip: IpAddr) -> bool {
        !matches!(self, Self::Layer3) || ip.is_ipv4()
    }

    fn host(self) -> &'static str {
        match self {
            Self::IpQuery => "api.ipquery.io",
            Self::IpLogs => "iplogs.com",
            Self::Layer3 => "api.layer3intel.com",
        }
    }

    fn parse(self, value: &Value, expected: IpAddr) -> Option<Observation> {
        let ip = match self {
            Self::IpLogs => value.pointer("/ip_info/ip"),
            _ => value.get("ip"),
        }?
        .as_str()?
        .parse::<IpAddr>()
        .ok()?;
        if !self.supports(expected) || ip != expected || value.get("error").is_some() {
            return None;
        }
        if matches!(self, Self::IpLogs) {
            let probed = value.get("probed").and_then(Value::as_bool);
            let fast = value.get("mode").and_then(Value::as_str) == Some("fast");
            if probed == Some(true) || (!fast && probed != Some(false)) {
                return None;
            }
        }
        let flag = |pointer| value.pointer(pointer).and_then(Value::as_bool);
        let reputation = match (self, value.get("reputation").and_then(Value::as_str)) {
            (Self::IpLogs, Some("allowlist")) => Some("允许名单"),
            (Self::IpLogs, Some("greylist")) => Some("灰名单"),
            (Self::IpLogs, Some("blocklist")) => Some("阻止名单"),
            _ => None,
        };
        let flags = match self {
            Self::IpQuery => [
                flag("/risk/is_datacenter"),
                flag("/risk/is_vpn"),
                flag("/risk/is_proxy"),
                flag("/risk/is_tor"),
                None,
                None,
                flag("/risk/is_mobile"),
            ],
            Self::IpLogs => [
                flag("/is_hosting"),
                flag("/is_vpn"),
                flag("/is_proxy"),
                flag("/is_tor"),
                flag("/is_crawler"),
                match value.get("reputation").and_then(Value::as_str) {
                    Some("blocklist") => Some(true),
                    Some("allowlist") => Some(false),
                    _ => None,
                },
                None,
            ],
            Self::Layer3 => [
                flag("/flags/hosting"),
                flag("/flags/vpn"),
                flag("/flags/proxy"),
                flag("/flags/tor"),
                None,
                None,
                None,
            ],
        };
        Some(Observation { flags, reputation })
    }

    async fn fetch(self, client: &Client, ip: IpAddr) -> Result<Observation, LookupFailure> {
        let host = self.host();
        let request = match self {
            Self::IpQuery => client.get(format!("https://{host}/{ip}")),
            Self::IpLogs => client
                .post(format!("https://{host}/v1/check"))
                .json(&json!({"ip": ip.to_string(), "mode": "fast"})),
            Self::Layer3 => client.get(format!("https://{host}/open/{ip}")),
        };
        let response = request.send().await.map_err(LookupFailure::request)?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            return Err(LookupFailure::RateLimited);
        }
        if !response.status().is_success() {
            return Err(LookupFailure::Unavailable);
        }
        let bytes = response_bytes(response).await.map_err(|error| {
            error
                .downcast::<reqwest::Error>()
                .map_or(LookupFailure::Invalid, LookupFailure::request)
        })?;
        let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| LookupFailure::Invalid)?;
        self.parse(&value, ip).ok_or(LookupFailure::Invalid)
    }
}

struct Observation {
    flags: [Option<bool>; 7],
    reputation: Option<&'static str>,
}

#[derive(Clone, Copy)]
enum LookupFailure {
    Timeout,
    RateLimited,
    Unavailable,
    Invalid,
}

impl LookupFailure {
    fn request(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else {
            Self::Unavailable
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Timeout => "请求超时",
            Self::RateLimited => "达到查询限额",
            Self::Unavailable => "服务暂不可用",
            Self::Invalid => "响应未通过校验",
        }
    }
}

const PROVIDERS: [Provider; 3] = [Provider::IpQuery, Provider::IpLogs, Provider::Layer3];

fn summarize(ip: IpAddr, lookups: &[(Provider, Result<Observation, LookupFailure>)]) -> Signal {
    let fields = [
        ("机房或托管网络", "各供应商对托管网络的分类，可能包含 CDN。"),
        (
            "VPN 地址",
            "各供应商分类口径不同，例如 IPLogs 的 VPN 标记也可能包含 Tor 或隐私中继。",
        ),
        (
            "代理地址",
            "各供应商对代理的定义与覆盖可能不同；差异需要结合来源核对。",
        ),
        ("Tor 地址", "各供应商的 Tor 名单与更新时间可能不同。"),
        ("爬虫地址", "供应商按地址范围识别，不证明当前请求来自爬虫。"),
        (
            "滥用记录",
            "依据供应商近期滥用名单，名单未命中无法排除历史滥用。",
        ),
        ("移动网络", "供应商按网络归属判断，不代表实际设备类型。"),
    ];
    let mut highlights = Vec::new();
    let mut details = Vec::new();
    let mut unknown = 0;
    for (index, (title, note)) in fields.iter().enumerate() {
        let mut yes = false;
        let mut no = false;
        let mut values = Vec::new();
        for (provider, result) in lookups {
            if let Ok(observation) = result {
                let value = match observation.flags[index] {
                    Some(true) => {
                        yes = true;
                        "是"
                    }
                    Some(false) => {
                        no = true;
                        "否"
                    }
                    None => "未提供",
                };
                values.push(format!("{} {value}", provider.name()));
            }
        }
        match (yes, no) {
            (true, true) => highlights.push(format!("{title}来源不一致")),
            (true, false) => highlights.push(format!("有来源标记{title}")),
            (false, false) => unknown += 1,
            (false, true) => {}
        }
        let values = if values.is_empty() {
            "无有效响应".into()
        } else {
            values.join("、")
        };
        details.push(format!("{title}：{values}。\n{note}"));
    }
    for (provider, result) in lookups {
        match result {
            Ok(observation) => {
                if let Some(reputation) = observation.reputation {
                    details.push(format!("{} 信誉：{reputation}", provider.name()));
                }
            }
            Err(failure) => details.push(format!("{}：{}", provider.name(), failure.label())),
        }
    }
    let evidence = if unknown == fields.len() {
        highlights.push("各项均待确认".into());
        Evidence::Unknown
    } else {
        if highlights.is_empty() {
            highlights.push("有结果的属性暂未见标记".into());
        }
        if unknown != 0 {
            highlights.push(format!("{unknown} 项待确认"));
        }
        Evidence::Inferred
    };
    let mut signal = Signal::new(
        Group::Quality,
        &format!("出口 IP 质量 · {ip}"),
        highlights.join(" · "),
        &lookups
            .iter()
            .map(|(provider, _)| provider.name())
            .collect::<Vec<_>>()
            .join("、"),
        evidence,
        &details.join("\n"),
    );
    signal.ip = ip.to_string();
    signal
}

pub async fn check(client: &Client, ip: IpAddr) -> Signal {
    let lookups = join_all(
        PROVIDERS
            .into_iter()
            .filter(|provider| provider.supports(ip))
            .map(|provider| async move { (provider, provider.fetch(client, ip).await) }),
    )
    .await;
    summarize(ip, &lookups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_check::{Environment, Report};

    #[test]
    fn providers_use_the_protected_diagnostic_route_and_dns() {
        for provider in PROVIDERS {
            let host = provider.host();
            assert!(crate::protection::DIAGNOSTIC_HOSTS.contains(&host));
            assert!(crate::protection::protected_dns_host(host));
            assert!(!crate::protection::associated_host(host));
        }
    }

    #[test]
    fn compares_provider_results_without_turning_missing_data_into_a_verdict() {
        let ip = "185.220.101.34".parse().unwrap();
        let fixtures = [
            (
                Provider::IpQuery,
                &json!({"ip":"185.220.101.34","risk":{"is_datacenter":true,"is_proxy":false,"is_tor":true}}),
            ),
            (
                Provider::IpLogs,
                &json!({"ip_info":{"ip":"185.220.101.34"},"mode":"fast",
                    "is_hosting":false,"is_proxy":true,"is_tor":true}),
            ),
            (
                Provider::Layer3,
                &json!({"ip":"185.220.101.34","flags":{"hosting":true,"proxy":false,"tor":true}}),
            ),
        ];
        let lookups = fixtures.map(|(provider, value)| {
            (
                provider,
                provider.parse(value, ip).ok_or(LookupFailure::Invalid),
            )
        });
        let mut report = Report::new(Environment::Core, "test");
        let signal = summarize(ip, &lookups);
        assert!(signal.value.contains("机房或托管网络来源不一致"));
        assert!(signal.value.contains("代理地址来源不一致"));
        assert!(signal.value.contains("有来源标记Tor 地址"));
        assert!(signal.value.contains("4 项待确认"));
        assert!(
            signal
                .detail
                .contains("IPQuery 是、IPLogs 否、Layer3 Intel 是")
        );
        assert!(!signal.risk);
        report.signals.push(signal);
        assert_eq!(report.counts(), (0, 0, 1));
        report.validate().unwrap();
    }

    #[test]
    fn rejects_wrong_addresses_active_checks_and_invalid_fields() {
        let ip = "1.1.1.1".parse().unwrap();
        assert!(
            Provider::IpQuery
                .parse(&json!({"ip":"8.8.8.8","risk":{"is_datacenter":true}}), ip)
                .is_none()
        );
        assert!(
            Provider::IpLogs
                .parse(
                    &json!({"ip_info":{"ip":"1.1.1.1"},"mode":"fast","probed":true}),
                    ip
                )
                .is_none()
        );
        assert!(
            Provider::IpLogs
                .parse(&json!({"ip_info":{"ip":"1.1.1.1"}}), ip)
                .is_none()
        );
        assert!(
            Provider::IpLogs
                .parse(&json!({"ip_info":{"ip":"1.1.1.1"}}), ip)
                .is_none()
        );
        let observation = Provider::IpQuery
            .parse(
                &json!({"ip":"1.1.1.1","risk":{"is_datacenter":"true","is_mobile":true}}),
                ip,
            )
            .unwrap();
        assert_eq!(observation.flags[0], None);
        assert_eq!(observation.flags[6], Some(true));
    }

    #[test]
    fn accepts_ipv6_and_preserves_provider_specific_abuse_evidence() {
        let ip = "2606:4700:4700::1111".parse().unwrap();
        assert!(!Provider::Layer3.supports(ip));
        let observation = Provider::IpLogs
            .parse(
                &json!({"ip_info":{"ip":"2606:4700:4700:0:0:0:0:1111"},"probed":false,
                    "is_hosting":true,"is_crawler":false,"reputation":"blocklist"}),
                ip,
            )
            .unwrap();
        assert_eq!(observation.flags[0], Some(true));
        assert_eq!(observation.flags[4], Some(false));
        assert_eq!(observation.flags[5], Some(true));
        let report = summarize(ip, &[(Provider::IpLogs, Ok(observation))]);
        assert!(report.value.contains("有来源标记滥用记录"));
        assert!(report.detail.contains("爬虫地址：IPLogs 否"));
        assert!(report.detail.contains("IPLogs 信誉：阻止名单"));
        assert!(!report.risk);
    }

    #[test]
    fn distinguishes_missing_observations_from_lookups_that_did_not_finish() {
        let ip = "1.1.1.1".parse().unwrap();
        let observation = Provider::IpLogs
            .parse(
                &json!({"ip_info":{"ip":"1.1.1.1"},"probed":false,"reputation":"greylist"}),
                ip,
            )
            .unwrap();
        assert_eq!(observation.flags[5], None);
        let signal = summarize(
            ip,
            &[
                (Provider::IpQuery, Err(LookupFailure::RateLimited)),
                (Provider::IpLogs, Ok(observation)),
                (Provider::Layer3, Err(LookupFailure::Timeout)),
            ],
        );
        assert_eq!(signal.evidence, Evidence::Unknown);
        assert_eq!(signal.value, "各项均待确认");
        for detail in [
            "IPQuery：达到查询限额",
            "IPLogs 信誉：灰名单",
            "Layer3 Intel：请求超时",
        ] {
            assert!(signal.detail.contains(detail));
        }
    }
}
