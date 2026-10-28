//! Correct resolver location metadata without suppressing country differences.
use super::{Evidence, Signal, response_bytes};
use crate::probe::country_name;
use reqwest::Client;
use std::{net::IpAddr, time::Duration};

const GOOGLE_FEED: &str = "https://www.gstatic.com/ipranges/publicdns_geofeed.txt";

pub(super) async fn enrich(client: &Client, signals: &mut [Signal]) {
    enrich_at(client, signals, GOOGLE_FEED).await;
}

async fn enrich_at(client: &Client, signals: &mut [Signal], url: &str) {
    if signals.is_empty() {
        return;
    }
    // One bounded metadata request per report, through the diagnostic client.
    let feed = async {
        let response = client
            .get(url)
            .timeout(Duration::from_secs(4))
            .send()
            .await?;
        let bytes = response_bytes(response).await?;
        Ok::<_, anyhow::Error>(String::from_utf8(bytes)?)
    }
    .await;
    for signal in signals {
        let Ok(ip) = signal.ip.parse::<IpAddr>() else {
            continue;
        };
        let official = feed.as_deref().ok().and_then(|feed| country(feed, ip));
        if let Some(code) = official {
            let original = country_name(&signal.country).unwrap_or_else(|| "未确认".into());
            let name = country_name(&code).expect("validated country");
            signal.value = format!("{ip} · {name}");
            signal.detail.push_str(&format!(
                " 地区采用 Google Public DNS 官方 geofeed：{GOOGLE_FEED}；官方标签为{name}，bash.ws 原标签为{original}。官方位置数据也不能证明客户端的解析路径。"
            ));
            signal.country = code;
            signal.evidence = Evidence::Measured;
        } else {
            signal.detail.push_str(
                " Google 官方位置清单不可用或没有唯一匹配；保留 bash.ws 原标签与差异判定，未作官方修正。",
            );
        }
    }
}

fn country(feed: &str, ip: IpAddr) -> Option<String> {
    let mut best = None;
    let mut ambiguous = false;
    for line in feed.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split(',');
        let Some(prefix) = fields.next() else {
            continue;
        };
        let Some(code) = fields.next() else {
            continue;
        };
        let code = code.trim().to_ascii_uppercase();
        if country_name(&code).is_none() {
            continue;
        }
        let Some(bits) = matching_prefix(ip, prefix.trim()) else {
            continue;
        };
        match &best {
            Some((length, _)) if *length > bits => {}
            Some((length, previous)) if *length == bits => {
                ambiguous |= *previous != code;
            }
            _ => {
                best = Some((bits, code));
                ambiguous = false;
            }
        }
    }
    if ambiguous {
        None
    } else {
        best.map(|(_, code)| code)
    }
}

fn matching_prefix(ip: IpAddr, prefix: &str) -> Option<u8> {
    let (network, bits) = prefix.split_once('/')?;
    let network = network.parse::<IpAddr>().ok()?;
    let bits = bits.parse::<u8>().ok()?;
    let matches = match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(network)) if bits <= 32 => {
            let mask = u32::MAX.checked_shl(32 - u32::from(bits)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) if bits <= 128 => {
            let mask = u128::MAX.checked_shl(128 - u32::from(bits)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(network) & mask
        }
        _ => false,
    };
    matches.then_some(bits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_check::{Environment, Group, Report};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const FEED: &str = "# Offline fixture based on the provider's published format\n\
        172.217.33.128/25,DE,DE-HE,Frankfurt,\n\
        172.217.34.0/26,DE,DE-HE,Frankfurt,\n\
        172.253.1.192/26,DE,DE-HE,Frankfurt,\n\
        2a00:1450:4001::/48,DE,DE-HE,Frankfurt,\n";

    #[test]
    fn provider_metadata_covers_reported_ips_without_an_embedded_allowlist() {
        for value in [
            "172.217.33.144",
            "172.217.33.150",
            "172.217.33.152",
            "172.217.33.153",
            "172.217.33.212",
            "172.217.34.17",
            "172.217.34.25",
            "172.217.34.28",
            "172.253.1.214",
            "2a00:1450:4001::1234",
        ] {
            assert_eq!(country(FEED, value.parse().unwrap()).as_deref(), Some("DE"));
        }
        for value in [
            "172.217.33.127",
            "172.217.34.64",
            "172.253.1.191",
            "2a00:1450:4002::1",
        ] {
            assert_eq!(country(FEED, value.parse().unwrap()), None);
        }
    }

    #[test]
    fn invalid_conflicting_and_overlapping_metadata_are_handled_conservatively() {
        let ip = "172.217.33.153".parse().unwrap();
        for feed in [
            "not a geofeed",
            "172.217.33.128/33,DE",
            "172.217.33.128/25,XX",
            "172.217.33.128/25,DE\n172.217.33.128/25,US",
            "::ffff:172.217.33.128/121,DE",
        ] {
            assert_eq!(country(feed, ip), None);
        }
        assert_eq!(
            country("172.217.0.0/16,US\n172.217.33.128/25,DE\n", ip).as_deref(),
            Some("DE"),
        );
        assert_eq!(matching_prefix(ip, "172.217.33.153/32"), Some(32));
        assert_eq!(matching_prefix(ip, "0.0.0.0/0"), Some(0));
        assert_eq!(
            matching_prefix("::1".parse().unwrap(), "::1/128"),
            Some(128)
        );
        assert_eq!(matching_prefix("::1".parse().unwrap(), "::/129"), None);
    }

    #[tokio::test]
    async fn enrichment_corrects_false_locations_without_hiding_country_risks() {
        for (status, body, expected) in [
            ("200 OK", FEED.to_owned(), "DE"),
            ("200 OK", "172.217.33.128/25,US\n".into(), "US"),
            ("502 Bad Gateway", "upstream unavailable".into(), "US"),
            ("200 OK", "unrelated metadata".into(), "US"),
            (
                "200 OK",
                "172.217.33.128/25,DE\n172.217.33.128/25,US".into(),
                "US",
            ),
            ("200 OK", "x".repeat(super::super::REPORT_LIMIT + 1), "US"),
        ] {
            let official = if status == "200 OK" {
                country(&body, "172.217.33.153".parse().unwrap())
            } else {
                None
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let read = stream.read(&mut request).await.unwrap();
                assert!(read > 0);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
            let mut signal = Signal::new(
                Group::Dns,
                "实际递归 DNS",
                "172.217.33.153 · 美国",
                "browser",
                Evidence::Measured,
                "bash.ws",
            );
            signal.ip = "172.217.33.153".into();
            signal.country = "US".into();
            let mut unrelated = signal.clone();
            unrelated.ip = "9.9.9.9".into();
            unrelated.value = "9.9.9.9 · 美国".into();
            let mut unknown = signal.clone();
            unknown.country.clear();
            unknown.evidence = Evidence::Unknown;
            let mut signals = [signal, unrelated, unknown];
            let client = crate::claude_check::client(None).unwrap();
            enrich_at(&client, &mut signals, &format!("http://{address}/feed")).await;
            server.await.unwrap();
            assert_eq!(signals[0].country, expected);
            assert_eq!(signals[0].source, "browser");
            assert_eq!(signals[0].evidence, Evidence::Measured);
            assert!(!signals[0].context_only());
            assert!(!signals[0].risk);
            assert_eq!(signals[1].country, "US");
            assert_eq!(signals[2].country, official.as_deref().unwrap_or_default());
            assert_eq!(
                signals[2].evidence,
                if official.is_some() {
                    Evidence::Measured
                } else {
                    Evidence::Unknown
                },
            );
            let mut report = Report::new(Environment::Core, "test");
            let mut exit = Signal::new(Group::Exit, "出口", "德国", "test", Evidence::Measured, "");
            exit.country = "DE".into();
            report.signals.push(exit);
            report.signals.push(signals[0].clone());
            assert_eq!(report.differs(&signals[0]), expected != "DE");
            assert_eq!(
                report.counts(),
                if expected == "DE" {
                    (2, 0, 0)
                } else {
                    (1, 1, 0)
                },
            );
            if expected == "DE" {
                assert!(signals[0].detail.contains("bash.ws 原标签为美国"));
            }
        }
    }
}
