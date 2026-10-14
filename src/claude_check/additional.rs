//! Explicit supplementary probes; no automatic bandwidth tests or remote pings.
use super::{Evidence, Group, Signal, public_ip, response_bytes};
use anyhow::{Result, bail, ensure};
use reqwest::Client;
use serde_json::Value;
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy)]
pub enum Kind {
    Registration,
    GlobalPing,
    Speed,
}
impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Registration => "注册信息",
            Self::GlobalPing => "全球探针",
            Self::Speed => "小流量测速",
        }
    }
}
pub async fn run(client: &Client, kind: Kind, ip: &str) -> Result<Vec<Signal>> {
    match kind {
        Kind::Registration => registration(client, ip).await,
        Kind::GlobalPing => global_ping(client, ip).await,
        Kind::Speed => speed(client).await,
    }
}
fn valid_target(ip: &str) -> Result<IpAddr> {
    let ip: IpAddr = ip.parse()?;
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
        "IP 注册与归属",
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
    record["results"].as_array().into_iter().flatten().take(20).map(|entry| {
        let probe=&entry["probe"];
        let stats=&entry["result"]["stats"];
        let title=format!("全球探针 · {} / {}",probe["country"].as_str().unwrap_or("未提供"),probe["city"].as_str().unwrap_or("未提供"));
        let numbers=["min","avg","max","loss"].map(|key|stats[key].as_f64().filter(|v|v.is_finite() && *v>=0.0));
        if let [Some(min),Some(avg),Some(max),Some(loss)]=numbers {Signal::new(Group::Connectivity,&title,format!("最小 {min:.1} · 平均 {avg:.1} · 最大 {max:.1} ms · 丢包 {loss:.0}%"),"Globalping 远端探针",Evidence::Measured,&format!("探针到出口 IP {ip} 的 ICMP 测量，不是本机到 Claude 的延迟；出口防火墙可拒绝 ICMP。"))}
        else {Signal::unknown(Group::Connectivity,&title,"Globalping 远端探针","没有获得完整测量，可能受 ICMP、防火墙或探针可用性影响。")}
    }).collect()
}
pub async fn speed(client: &Client) -> Result<Vec<Signal>> {
    let download = download_rate(
        client,
        "https://speed.cloudflare.com/__down?bytes=10485760",
        10 * 1024 * 1024,
    )
    .await?;
    let started = Instant::now();
    let chunks =
        futures_util::stream::iter((0..32).map(|_| Ok::<_, std::io::Error>(vec![0u8; 64 * 1024])));
    let response = client
        .post("https://speed.cloudflare.com/__up")
        .timeout(Duration::from_secs(30))
        .header(reqwest::header::CONTENT_LENGTH, 2 * 1024 * 1024)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await?;
    ensure!(response.status().is_success(), "上传测量未完成");
    let upload = (2 * 1024 * 1024) as f64 * 8.0
        / started.elapsed().as_secs_f64().max(0.000001)
        / 1_000_000.0;
    Ok(vec![
        Signal::new(
            Group::Connectivity,
            "小流量下载测量",
            format!("{download:.2} Mbps"),
            "Cloudflare speed endpoint",
            Evidence::Measured,
            "读取并立即丢弃最多 10 MiB 数据；含连接开销，小样本不代表最大带宽。",
        ),
        Signal::new(
            Group::Connectivity,
            "小流量上传测量",
            format!("{upload:.2} Mbps"),
            "Cloudflare speed endpoint",
            Evidence::Measured,
            "以 64 KiB 块上传 2 MiB 数据；只测试当前探测线路，不向 Claude 上传数据。",
        ),
    ])
}
async fn download_rate(client: &Client, url: &str, limit: usize) -> Result<f64> {
    let started = Instant::now();
    let mut response = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;
    ensure!(response.status().is_success(), "下载测量未完成");
    let mut size = 0;
    while let Some(chunk) = response.chunk().await? {
        size += chunk.len();
        ensure!(size <= limit, "测速响应超过流量限制");
    }
    ensure!(size == limit, "下载测量未收到完整样本");
    Ok(size as f64 * 8.0 / started.elapsed().as_secs_f64().max(0.000001) / 1_000_000.0)
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
        assert_eq!(signals[1].evidence, Evidence::Unknown);
        assert!(signals.iter().all(|s| s.country.is_empty()));
    }
}
