//! Address filtering and merging for the controller resolver.
use anyhow::{Result, bail};
use serde::Deserialize;
use std::net::IpAddr;

#[derive(Deserialize)]
pub(crate) struct Answer {
    #[serde(rename = "Status")]
    status: u16,
    #[serde(default, rename = "Answer")]
    records: Vec<Record>,
}
#[derive(Deserialize)]
struct Record {
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}
impl Answer {
    pub(crate) fn addresses(self) -> Result<Vec<IpAddr>> {
        if self.status != 0 {
            bail!("DNS 查询未返回地址");
        }
        Ok(self
            .records
            .into_iter()
            .take(64)
            .filter_map(|record| {
                let ip: IpAddr = record.data.parse().ok()?;
                let family_matches =
                    matches!((record.kind, ip), (1, IpAddr::V4(_)) | (28, IpAddr::V6(_)));
                (family_matches && !ip.is_unspecified() && !synthetic(ip)).then_some(ip)
            })
            .take(16)
            .collect())
    }
}
fn synthetic(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.octets()[..2] == [198, 18] || ip.octets()[..2] == [198, 19],
        IpAddr::V6(ip) => ip.segments()[..4] == [0xfdfe, 0xdcba, 0x9876, 0],
    }
}
pub(crate) fn combine(v4: Result<Vec<IpAddr>>, v6: Result<Vec<IpAddr>>) -> Result<Vec<IpAddr>> {
    let mut addresses = Vec::with_capacity(16);
    for ip in v4.into_iter().chain(v6).flatten() {
        if !addresses.contains(&ip) && addresses.len() < 16 {
            addresses.push(ip);
        }
    }
    if addresses.is_empty() {
        bail!("未查到目标地址");
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_aliases_fake_ips_and_wrong_record_families() {
        let answer: Answer = serde_json::from_str(r#"{"Status":0,"Answer":[{"type":5,"data":"alias.test."},{"type":1,"data":"198.18.0.1"},{"type":1,"data":"2001:db8::1"},{"type":28,"data":"fdfe:dcba:9876::2"},{"type":28,"data":"2001:db8::2"},{"type":1,"data":"203.0.113.2"}]}"#).unwrap();
        assert_eq!(
            answer.addresses().unwrap(),
            [
                "2001:db8::2".parse::<IpAddr>().unwrap(),
                "203.0.113.2".parse().unwrap()
            ]
        );
        assert!(combine(Ok(vec![]), Ok(vec![])).is_err());
    }
}
