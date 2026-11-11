//! Bounded supplementary evidence for a measured public exit.
use super::{Evidence, Group, Report, Signal, public_ip, response_bytes};
use anyhow::{Result, bail, ensure};
use reqwest::Client;
use serde_json::Value;
use std::{net::IpAddr, time::Duration};

fn exit_ip(report: &Report) -> Option<IpAddr> {
    report
        .signals
        .iter()
        .filter(|signal| signal.group == Group::Exit && signal.evidence == Evidence::Measured)
        .filter_map(|signal| signal.ip.parse::<IpAddr>().ok())
        .find(|ip| public_ip(*ip))
}

pub async fn extend(client: &Client, report: &mut Report) {
    let Some(ip) = exit_ip(report) else {
        report.signals.extend([
            Signal::unknown(
                Group::Quality,
                "IP 注册信息",
                "RDAP",
                "未取得实测公网出口 IP，未发起注册查询。",
            ),
            Signal::unknown(
                Group::Connectivity,
                "全球连通性",
                "Globalping 远端探针",
                "未取得实测公网出口 IP，未发起远端探测。",
            ),
        ]);
        return;
    };
    let ip = ip.to_string();
    let (registration, ping) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(15), registration(client, &ip)),
        tokio::time::timeout(Duration::from_secs(30), global_ping(client, &ip)),
    );
    report
        .signals
        .extend(registration.ok().and_then(Result::ok).unwrap_or_else(|| {
            vec![Signal::unknown(
                Group::Quality,
                "IP 注册信息",
                "RDAP",
                "注册服务超时、拒绝请求或未返回有效记录；不影响其他检测结果。",
            )]
        }));
    report
        .signals
        .extend(ping.ok().and_then(Result::ok).unwrap_or_else(|| {
            vec![Signal::unknown(
                Group::Connectivity,
                "全球连通性",
                "Globalping 远端探针",
                "远端探测超时、额度不足或服务不可用；不能据此判定出口故障。",
            )]
        }));
}

fn valid_target(ip: &str) -> Result<IpAddr> {
    let ip: IpAddr = ip
        .parse()
        .map_err(|_| anyhow::anyhow!("尚未取得有效公网出口 IP"))?;
    ensure!(public_ip(ip), "请先检测并获得有效公网出口 IP");
    Ok(ip)
}
pub async fn registration(client: &Client, ip: &str) -> Result<Vec<Signal>> {
    valid_target(ip)?;
    let mut url = reqwest::Url::parse(&format!("https://rdap.org/ip/{ip}"))?;
    for _ in 0..3 {
        let response = client.get(url.clone()).send().await?;
        if response.status().is_redirection() {
            let next = url.join(
                response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .ok_or_else(|| anyhow::anyhow!("注册查询未提供目标"))?
                    .to_str()?,
            )?;
            ensure!(
                next.scheme() == "https"
                    && next.username().is_empty()
                    && next.password().is_none()
                    && next.port_or_known_default() == Some(443)
                    && next.host_str().is_some_and(|host| [
                        "arin.net",
                        "ripe.net",
                        "apnic.net",
                        "lacnic.net",
                        "afrinic.net",
                        "registro.br",
                        "nic.ad.jp",
                        "nic.or.kr"
                    ]
                    .iter()
                    .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))),
                "注册查询返回未认可的服务地址"
            );
            url = next;
            continue;
        }
        ensure!(response.status().is_success(), "注册服务未返回有效结果");
        let record: Value = serde_json::from_slice(&response_bytes(response).await?)?;
        return registration_record(&record, ip, url.host_str().unwrap_or("RDAP"));
    }
    bail!("注册查询跳转过多")
}
fn registration_record(record: &Value, ip: &str, source: &str) -> Result<Vec<Signal>> {
    let ip = valid_target(ip)?;
    let start: IpAddr = record["startAddress"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("注册记录缺少起始地址"))?
        .parse()?;
    let end: IpAddr = record["endAddress"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("注册记录缺少结束地址"))?
        .parse()?;
    ensure!(
        std::mem::discriminant(&ip) == std::mem::discriminant(&start) && ip >= start && ip <= end,
        "注册记录不包含所查 IP"
    );
    let name = record["name"]
        .as_str()
        .unwrap_or("未提供")
        .chars()
        .take(256)
        .collect::<String>();
    let country = record["country"].as_str().unwrap_or("未提供");
    let events = record["events"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .take(4)
                .map(|v| {
                    format!(
                        "{}：{}",
                        v["eventAction"].as_str().unwrap_or("事件"),
                        v["eventDate"].as_str().unwrap_or("未提供")
                    )
                })
                .collect::<Vec<_>>()
                .join("；")
        })
        .unwrap_or_default();
    Ok(vec![Signal::new(
        Group::Quality,
        "IP 注册信息",
        format!("{name} · {start}–{end} · 注册地区 {country}"),
        source,
        Evidence::Measured,
        &format!("RDAP 注册信息；注册地区不代表出口物理位置。{events}"),
    )])
}
pub async fn global_ping(client: &Client, ip: &str) -> Result<Vec<Signal>> {
    valid_target(ip)?;
    let response=client.post("https://api.globalping.io/v1/measurements").json(&serde_json::json!({"type":"ping","target":ip,"limit":20,"locations":[],"measurementOptions":{"packets":3}})).send().await?;
    ensure!(
        response.status().is_success(),
        "全球探针服务拒绝请求或额度不足"
    );
    let created: Value = serde_json::from_slice(&response_bytes(response).await?)?;
    let id = created["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("探针服务未返回编号"))?;
    ensure!(
        id.len() <= 64 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "探针编号无效"
    );
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let response = client
            .get(format!("https://api.globalping.io/v1/measurements/{id}"))
            .send()
            .await?;
        ensure!(response.status().is_success(), "探针结果读取失败");
        let result: Value = serde_json::from_slice(&response_bytes(response).await?)?;
        if result["status"] == "finished" {
            return Ok(ping_results(&result, ip));
        }
    }
    bail!("全球探针尚未完成，请稍后重试；没有将超时判定为节点故障")
}
fn ping_results(record: &Value, ip: &str) -> Vec<Signal> {
    let results: Vec<_> = record["results"]
        .as_array()
        .into_iter()
        .flatten()
        .take(20)
        .collect();
    let mut delays = Vec::new();
    for entry in &results {
        let stats = &entry["result"]["stats"];
        let average = stats["avg"]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0);
        let loss = stats["loss"]
            .as_f64()
            .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
        if let (Some(average), Some(loss)) = (average, loss)
            && loss < 100.0
        {
            delays.push(average);
        }
    }
    if delays.is_empty() {
        return vec![Signal::unknown(
            Group::Connectivity,
            "全球连通性",
            "Globalping 远端探针",
            "未取得有效 ICMP 响应；可能受出口防火墙、探针或服务可用性影响，不能据此判定出口故障。",
        )];
    }
    delays.sort_by(f64::total_cmp);
    vec![Signal::new(
        Group::Connectivity,
        "全球连通性",
        format!(
            "{}/{} 个探针取得响应 · 中位 {:.1} ms",
            delays.len(),
            results.len(),
            delays[delays.len() / 2]
        ),
        "Globalping 远端探针",
        Evidence::Measured,
        &format!(
            "远端探针到出口 IP {ip} 的 ICMP 测量，不是本机到 Claude 的延迟。有效探针平均延迟范围 {:.1}–{:.1} ms；其余探针未取得有效响应，可能受防火墙或探针可用性影响。",
            delays[0],
            delays[delays.len() - 1],
        ),
    )]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_location_and_remote_ping_do_not_become_client_region_verdicts() {
        let record = serde_json::json!({"startAddress":"1.0.0.0","endAddress":"1.255.255.255","name":"Example","country":"AU"});
        let signal = registration_record(&record, "1.2.3.4", "fixture")
            .unwrap()
            .remove(0);
        assert!(signal.country.is_empty() && !signal.risk);
        assert!(registration_record(&record, "8.8.8.8", "fixture").is_err());
        let signals = ping_results(
            &serde_json::json!({"results":[{"probe":{"country":"DE","city":"Berlin"},"result":{"stats":{"min":10,"avg":12,"max":15,"loss":0}}},{"probe":{"country":"US"},"result":{}}]}),
            "1.2.3.4",
        );
        assert_eq!(signals[0].evidence, Evidence::Measured);
        assert_eq!(signals.len(), 1);
        assert!(signals[0].value.starts_with("1/2"));
        assert_eq!(
            ping_results(&serde_json::json!({}), "1.2.3.4")[0].evidence,
            Evidence::Unknown
        );
        assert_eq!(
            ping_results(
                &serde_json::json!({"results":[{"result":{"stats":{"avg":0,"loss":100}}}]}),
                "1.2.3.4"
            )[0]
            .evidence,
            Evidence::Unknown
        );
        assert!(signals.iter().all(|s| s.country.is_empty()));
    }

    #[tokio::test]
    async fn missing_or_unmeasured_exit_skips_external_queries() {
        let client = Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .build()
            .unwrap();
        for (ip, evidence) in [
            ("", Evidence::Measured),
            ("invalid", Evidence::Measured),
            ("127.0.0.1", Evidence::Measured),
            ("1.2.3.4", Evidence::Inferred),
        ] {
            let mut report = Report::new(super::super::Environment::Core, "fixture");
            let mut exit = Signal::new(Group::Exit, "出口", ip, "fixture", evidence, "");
            exit.ip = ip.into();
            report.signals.push(exit);
            extend(&client, &mut report).await;
            assert_eq!(report.signals.len(), 3);
            assert!(report.signals[1..].iter().all(|signal| {
                signal.evidence == Evidence::Unknown
                    && !signal.title.is_empty()
                    && signal.detail.contains("未发起")
            }));
        }
    }
}
