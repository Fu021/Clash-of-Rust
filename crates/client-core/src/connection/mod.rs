//! Connection sampling and ordering without retaining a second metadata snapshot.
use crate::api::{Connection, ConnectionRates};
use crate::config::Language;
use crate::messages;
use std::{cmp::Ordering, time::Duration};

pub fn hostname(connection: &Connection) -> &str {
    if connection.metadata.sniff_host.is_empty() {
        connection.metadata.host.as_ref()
    } else {
        connection.metadata.sniff_host.as_ref()
    }
}

/// Resolve a missing remote address through the core controller.
/// The mixed-port argument is retained for existing callers.
pub async fn resolve_target(
    api: &crate::api::Api,
    host: &str,
    _mixed_port: u16,
) -> anyhow::Result<String> {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return Ok(ip.to_string());
    }
    anyhow::ensure!(!host.is_empty(), "域名为空");
    let ips = tokio::time::timeout(Duration::from_secs(16), api.resolve_domain(host))
        .await
        .map_err(|_| anyhow::anyhow!("目标地址查询超时"))??;
    Ok(ips
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n"))
}

pub fn detail_values(connection: &Connection, resolved: &str) -> Vec<(&'static str, String)> {
    detail_values_in(connection, resolved, Language::SimplifiedChinese)
}

pub fn detail_values_in(
    connection: &Connection,
    resolved: &str,
    language: Language,
) -> Vec<(&'static str, String)> {
    fn bytes(value: u64) -> String {
        let mut size = value as f64;
        let mut index = 0;
        let units = ["B", "KiB", "MiB", "GiB", "TiB"];
        while size >= 1024. && index < units.len() - 1 {
            size /= 1024.;
            index += 1;
        }
        if index == 0 {
            format!("{value} B")
        } else {
            format!("{size:.1} {}", units[index])
        }
    }
    let rate = |value: Option<u64>| value.map_or_else(|| "—".into(), |v| format!("{}/s", bytes(v)));
    vec![
        (
            messages::connection_title(language),
            name(connection).to_owned(),
        ),
        (
            messages::connection_domain(language),
            connection.metadata.host.to_string(),
        ),
        (
            messages::connection_sniffed_domain(language),
            connection.metadata.sniff_host.to_string(),
        ),
        (
            messages::connection_destination_ip(language),
            if connection.metadata.destination_ip.is_empty() {
                resolved.to_owned()
            } else {
                connection.metadata.destination_ip.to_string()
            },
        ),
        (
            messages::connection_destination_port(language),
            connection.metadata.destination_port.to_string(),
        ),
        (
            messages::connection_protocol(language),
            connection.metadata.network.to_string(),
        ),
        (
            messages::connection_process(language),
            connection.metadata.process.to_string(),
        ),
        (
            messages::connection_matched_rule(language),
            format!("{} {}", connection.rule, connection.rule_payload)
                .trim()
                .to_owned(),
        ),
        (
            messages::connection_proxy_chain(language),
            connection
                .chains
                .iter()
                .rev()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" → "),
        ),
        (
            messages::connection_upload_speed(language),
            rate(connection.rates.map(|r| r.upload)),
        ),
        (
            messages::connection_download_speed(language),
            rate(connection.rates.map(|r| r.download)),
        ),
        (
            messages::connection_uploaded(language),
            bytes(connection.upload),
        ),
        (
            messages::connection_downloaded(language),
            bytes(connection.download),
        ),
        (
            messages::connection_started(language),
            connection.start.to_string(),
        ),
        (messages::connection_id(language), connection.id.to_string()),
    ]
}

pub fn sample_rates(current: &mut [Connection], previous: &[Connection], elapsed: Duration) {
    for connection in current.iter_mut() {
        connection.rates = None;
    }
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 || previous.is_empty() {
        return;
    }
    let mut baseline: Vec<_> = previous.iter().collect();
    baseline.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    for connection in current {
        let Ok(index) = baseline.binary_search_by(|old| old.id.cmp(&connection.id)) else {
            continue;
        };
        let old = baseline[index];
        // An ID with changed start time or reset counters has no valid baseline.
        if connection.start != old.start {
            continue;
        }
        if let (Some(upload), Some(download)) = (
            connection.upload.checked_sub(old.upload),
            connection.download.checked_sub(old.download),
        ) {
            connection.rates = Some(ConnectionRates {
                upload: (upload as f64 / seconds) as u64,
                download: (download as f64 / seconds) as u64,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Rule,
    Chain,
    UploadRate,
    DownloadRate,
    UploadTotal,
    DownloadTotal,
    Start,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Sort {
    pub field: Option<Field>,
    pub descending: bool,
}

impl Sort {
    pub fn toggle(&mut self, field: Field) {
        let initial_descending = !matches!(field, Field::Name | Field::Rule | Field::Chain);
        if self.field != Some(field) {
            self.field = Some(field);
            self.descending = initial_descending;
        } else if self.descending == initial_descending {
            self.descending = !initial_descending;
        } else {
            *self = Self::default();
        }
    }

    pub fn compare(&self, a: &Connection, b: &Connection) -> Ordering {
        let Some(field) = self.field else {
            return Ordering::Equal;
        };
        let order = match field {
            Field::Name => name(a).cmp(name(b)).then_with(|| {
                a.metadata
                    .destination_port
                    .as_str()
                    .parse::<u16>()
                    .unwrap_or(0)
                    .cmp(
                        &b.metadata
                            .destination_port
                            .as_str()
                            .parse::<u16>()
                            .unwrap_or(0),
                    )
            }),
            Field::Rule => a.rule.as_str().cmp(b.rule.as_str()),
            Field::Chain => a
                .chains
                .iter()
                .map(|s| s.as_str())
                .cmp(b.chains.iter().map(|s| s.as_str())),
            Field::UploadTotal => a.upload.cmp(&b.upload),
            Field::DownloadTotal => a.download.cmp(&b.download),
            Field::Start => chrono::DateTime::parse_from_rfc3339(&a.start)
                .ok()
                .cmp(&chrono::DateTime::parse_from_rfc3339(&b.start).ok()),
            Field::UploadRate | Field::DownloadRate => match (a.rates, b.rates) {
                (Some(a), Some(b)) => match field {
                    Field::UploadRate => a.upload.cmp(&b.upload),
                    _ => a.download.cmp(&b.download),
                },
                // Unknown samples stay last in either direction.
                (Some(_), None) => return Ordering::Less,
                (None, Some(_)) => return Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
        };
        (if self.descending {
            order.reverse()
        } else {
            order
        })
        .then_with(|| a.id.cmp(&b.id))
    }
}

pub fn name(connection: &Connection) -> &str {
    if !connection.metadata.sniff_host.is_empty() {
        connection.metadata.sniff_host.as_str()
    } else if connection.metadata.host.is_empty() {
        &connection.metadata.destination_ip
    } else {
        &connection.metadata.host
    }
}

pub fn matches_query(connection: &Connection, lower_query: &str) -> bool {
    lower_query.split_whitespace().all(|word| {
        [
            connection.metadata.host.as_ref(),
            connection.metadata.sniff_host.as_str(),
            connection.metadata.destination_ip.as_ref(),
            connection.rule.as_str(),
            connection.rule_payload.as_str(),
        ]
        .into_iter()
        .chain(connection.chains.iter().map(|name| name.as_str()))
        .any(|field| crate::model::contains_query(field, word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn target_resolution_stays_on_the_controller_when_dns_fails() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };

        let controller = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mixed_port = mixed.local_addr().unwrap().port();
        let api = crate::api::Api::new(&crate::config::Settings {
            controller_port: controller.local_addr().unwrap().port(),
            mixed_port,
            ..Default::default()
        })
        .unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = controller.accept().await.unwrap();
                let mut request = [0; 4096];
                let size = stream.read(&mut request).await.unwrap();
                assert!(request[..size].starts_with(b"GET /dns/query?name=fixture.test&type="));
                stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        let result = tokio::select! {
            result = tokio::time::timeout(
                Duration::from_secs(3),
                resolve_target(&api, "fixture.test", mixed_port)
            ) => result.unwrap(),
            _ = mixed.accept() => panic!("DNS lookup contacted the mixed port"),
        };
        assert!(result.is_err());
        server.await.unwrap();
    }

    fn row(id: &str, download: u64) -> Connection {
        Connection {
            id: id.into(),
            start: "2026-10-11T10:00:00Z".into(),
            download,
            ..Default::default()
        }
    }

    #[test]
    fn samples_use_ids_and_actual_elapsed_time_and_reject_resets() {
        let old = [row("closed", 100), row("a", 100), row("b", 600)];
        let mut current = [row("b", 500), row("new", 900), row("a", 600)];
        sample_rates(&mut current, &old, Duration::from_millis(2500));
        assert_eq!(current[2].rates.unwrap().download, 200);
        assert_eq!(current[2].rates.unwrap().upload, 0);
        assert!(current[0].rates.is_none());
        assert!(current[1].rates.is_none());
        current[2].start = "new session".into();
        sample_rates(&mut current, &old, Duration::from_secs(1));
        assert!(current[2].rates.is_none());
    }

    #[test]
    fn numeric_order_toggles_and_unknown_rates_remain_last() {
        let mut rows = [
            row("unknown", 0),
            row("small", 900 * 1024),
            row("large", 2 * 1024 * 1024),
        ];
        rows[1].rates = Some(ConnectionRates {
            upload: 0,
            download: 900 * 1024,
        });
        rows[2].rates = Some(ConnectionRates {
            upload: 0,
            download: 2 * 1024 * 1024,
        });
        let mut sort = Sort::default();
        sort.toggle(Field::DownloadRate);
        rows.sort_unstable_by(|a, b| sort.compare(a, b));
        assert_eq!(
            rows.iter().map(|c| c.id.as_ref()).collect::<Vec<_>>(),
            ["large", "small", "unknown"]
        );
        sort.toggle(Field::DownloadRate);
        rows.sort_unstable_by(|a, b| sort.compare(a, b));
        assert_eq!(
            rows.iter().map(|c| c.id.as_ref()).collect::<Vec<_>>(),
            ["small", "large", "unknown"]
        );
        sort.toggle(Field::DownloadTotal);
        assert!(sort.descending);
        rows.sort_unstable_by(|a, b| sort.compare(a, b));
        assert_eq!(rows[0].id.as_ref(), "large");
    }

    #[test]
    fn every_column_cycles_back_to_source_order_and_switching_columns_restarts() {
        for field in [
            Field::Name,
            Field::Rule,
            Field::Chain,
            Field::UploadRate,
            Field::DownloadRate,
            Field::UploadTotal,
            Field::DownloadTotal,
            Field::Start,
        ] {
            let mut sort = Sort::default();
            let first_descending = !matches!(field, Field::Name | Field::Rule | Field::Chain);
            sort.toggle(field);
            assert_eq!(sort.field, Some(field));
            assert_eq!(sort.descending, first_descending);
            sort.toggle(field);
            assert_eq!(sort.field, Some(field));
            assert_eq!(sort.descending, !first_descending);
            sort.toggle(field);
            assert!(sort.field.is_none());
            assert!(!sort.descending);
            assert_eq!(sort.compare(&row("a", 10), &row("b", 20)), Ordering::Equal);
            sort.toggle(field);
            assert_eq!(sort.descending, first_descending);
        }
        let mut sort = Sort::default();
        sort.toggle(Field::DownloadRate);
        sort.toggle(Field::DownloadRate);
        sort.toggle(Field::UploadRate);
        assert_eq!(sort.field, Some(Field::UploadRate));
        assert!(sort.descending);
    }

    #[test]
    fn name_uses_ip_fallback_and_equal_values_have_deterministic_order() {
        let mut a = row("a", 0);
        let mut b = row("b", 0);
        a.metadata.destination_ip = "192.0.2.1".into();
        b.metadata.host = "example.com".into();
        let mut sort = Sort::default();
        sort.toggle(Field::Name);
        assert_eq!(sort.compare(&a, &b), Ordering::Less);
        sort.toggle(Field::Name);
        assert_eq!(sort.compare(&a, &b), Ordering::Greater);
        sort.toggle(Field::UploadRate);
        assert_eq!(sort.compare(&a, &b), Ordering::Less);
    }

    #[test]
    fn sniffed_domain_is_retained_when_the_connection_only_has_an_ip() {
        let connection: Connection = serde_json::from_value(serde_json::json!({
            "id":"sniffed", "metadata":{"host":"", "sniffHost":"mail.google.com", "destinationIP":"192.0.2.1"},
            "rule":"DomainSuffix", "rulePayload":"google.com"
        })).unwrap();
        assert_eq!(name(&connection), "mail.google.com");
        assert_eq!(connection.rule_payload.as_str(), "google.com");
    }

    #[test]
    fn translated_connection_details_keep_original_values_and_order() {
        let connection: Connection = serde_json::from_value(serde_json::json!({
            "id":"example-id",
            "metadata":{"host":"example.test","sniffHost":"sniffed.test","destinationIP":"192.0.2.1","network":"tcp"},
            "rule":"DomainSuffix","rulePayload":"test","chains":["日本出口","Proxy"],
            "upload":1024,"download":4096
        }))
        .unwrap();
        let chinese = detail_values(&connection, "");
        let english = detail_values_in(&connection, "", Language::English);
        assert!(
            chinese
                .iter()
                .map(|(_, value)| value)
                .eq(english.iter().map(|(_, value)| value))
        );
        assert!(english.contains(&("Domain", "example.test".into())));
        assert!(english.contains(&("Sniffed domain", "sniffed.test".into())));
        assert!(english.contains(&("Proxy chain", "Proxy → 日本出口".into())));
        assert!(english.contains(&("Connection ID", "example-id".into())));
    }

    #[test]
    fn original_and_sniffed_domains_stay_distinct_and_both_participate_in_search() {
        let mut connection: Connection = serde_json::from_value(serde_json::json!({
            "id":"domains", "metadata":{"host":"original.test", "sniffHost":"SNIFFED.test", "destinationIP":"192.0.2.1"},
            "rule":"DomainSuffix", "rulePayload":"test", "chains":["日本出口"]
        })).unwrap();
        let details = detail_values(&connection, "");
        assert!(details.contains(&("域名", "original.test".into())));
        assert!(details.contains(&("嗅探域名", "SNIFFED.test".into())));
        for query in [
            "original.test",
            "sniffed.test",
            "192.0.2",
            "domainsuffix 日本",
            "original.test sniffed.test",
            "sniffed.test 192.0.2 日本",
        ] {
            assert!(matches_query(&connection, query), "{query}");
        }
        assert!(!matches_query(&connection, "missing.test"));
        assert!(!matches_query(&connection, "sniffed.test missing.test"));
        connection.metadata.sniff_host = "".into();
        assert_eq!(name(&connection), "original.test");
        assert!(detail_values(&connection, "").contains(&("嗅探域名", String::new())));
        connection.metadata.host = "".into();
        assert_eq!(name(&connection), "192.0.2.1");
        assert!(detail_values(&connection, "").contains(&("域名", String::new())));
    }

    #[test]
    fn start_order_compares_instants_including_fractional_seconds_and_offsets() {
        let mut rows = [row("half", 0), row("whole", 0), row("later", 0)];
        rows[0].start = "2026-10-11T10:00:00.5Z".into();
        rows[1].start = "2026-10-11T11:00:00+01:00".into();
        rows[2].start = "2026-10-11T10:00:01Z".into();
        let mut sort = Sort::default();
        sort.toggle(Field::Start);
        rows.sort_unstable_by(|a, b| sort.compare(a, b));
        assert_eq!(
            rows.iter().map(|c| c.id.as_ref()).collect::<Vec<_>>(),
            ["later", "half", "whole"]
        );
        sort.toggle(Field::Start);
        rows.sort_unstable_by(|a, b| sort.compare(a, b));
        assert_eq!(rows[0].id.as_ref(), "whole");
    }

    #[test]
    fn empty_or_zero_duration_baselines_clear_old_rates() {
        let mut current = [row("a", 100)];
        current[0].rates = Some(ConnectionRates {
            upload: 2,
            download: 3,
        });
        sample_rates(&mut current, &[], Duration::from_secs(1));
        assert!(current[0].rates.is_none());
        let old = [row("a", 10)];
        sample_rates(&mut current, &old, Duration::ZERO);
        assert!(current[0].rates.is_none());
    }
}
