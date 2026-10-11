//! Connection sampling and ordering without retaining a second metadata snapshot.
use crate::api::{Connection, ConnectionRates};
use std::{cmp::Ordering, time::Duration};

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
        if self.field == Some(field) {
            self.descending = !self.descending;
        } else {
            self.field = Some(field);
            self.descending = !matches!(field, Field::Name | Field::Rule | Field::Chain);
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
    if connection.metadata.host.is_empty() {
        &connection.metadata.destination_ip
    } else {
        &connection.metadata.host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let old = vec![row("closed", 100), row("a", 100), row("b", 600)];
        let mut current = vec![row("b", 500), row("new", 900), row("a", 600)];
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
        let mut rows = vec![
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
