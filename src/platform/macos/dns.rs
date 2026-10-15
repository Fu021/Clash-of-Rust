//! Reversible system DNS ownership for Claude protection. DHCP remains DHCP
//! after restoration: the configured dictionary, not DHCP's addresses, is saved.
use super::{cf, native_proxy};
use crate::platform::{ProxyState, ProxyValue};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io::Read, net::IpAddr, path::Path, ptr};

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyValue(store: cf::Ref, key: cf::Ref) -> cf::Ref;
}

pub fn system_servers() -> Result<Vec<String>> {
    let key = cf::string("State:/Network/Global/DNS")?;
    let value = cf::Owned::new(unsafe { SCDynamicStoreCopyValue(ptr::null(), key.0) })?;
    let value = cf::json(value.0)?;
    let servers: Vec<String> = value["ServerAddresses"]
        .as_array()
        .context("无法读取 macOS 当前 DNS 服务器")?
        .iter()
        .filter_map(Value::as_str)
        .filter(|s| {
            s.parse::<IpAddr>()
                .is_ok_and(|ip| !ip.is_unspecified() && !ip.is_multicast())
        })
        .take(16)
        .map(str::to_owned)
        .collect();
    ensure!(!servers.is_empty(), "macOS 当前没有可用 DNS 服务器");
    Ok(servers)
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Lease {
    original: ProxyState,
    owned: ProxyState,
    owner: u32,
    started: [u64; 2],
}
fn process_start(pid: u32) -> Option<[u64; 2]> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some([info.pbi_start_tvsec, info.pbi_start_tvusec])
}
impl Lease {
    pub fn owner_exited(&self) -> bool {
        process_start(self.owner) != Some(self.started)
    }
    pub fn owned_by(&self, pid: u32) -> bool {
        self.owner == pid || self.owner_exited()
    }
}

pub fn configured() -> Result<ProxyState> {
    native_proxy::read_dns()
}

pub(super) fn load(path: &Path) -> Result<Option<Lease>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(256 * 1024)
        .read_to_end(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn desired(original: &ProxyState) -> Result<ProxyState> {
    original
        .iter()
        .map(|(id, value)| {
            let mut value = match value {
                Some(ProxyValue::Text(text)) => serde_json::from_str::<Value>(text)?,
                None => json!({}),
                _ => anyhow::bail!("网络服务 DNS 属性无效"),
            };
            ensure!(value.is_object(), "网络服务 DNS 属性无效");
            // Public destinations are intercepted by TUN on macOS; requests to a
            // DHCP router on the local subnet are not. Upstream is still the pinned
            // node's DoH from the runtime overlay, never plaintext to these IPs.
            value["ServerAddresses"] = json!(["1.1.1.1", "8.8.8.8"]);
            Ok((id.clone(), Some(ProxyValue::Text(value.to_string()))))
        })
        .collect()
}

fn restoration(current: &ProxyState, lease: &Lease) -> ProxyState {
    lease
        .original
        .iter()
        .filter(|(id, _)| current.contains_key(*id) && current.get(*id) == lease.owned.get(*id))
        .map(|(id, value)| (id.clone(), value.clone()))
        .collect()
}

pub(super) fn restore(path: &Path, lease: &mut Option<Lease>) -> Result<()> {
    if let Some(owned) = lease {
        let restore = restoration(&native_proxy::read_dns()?, owned);
        if !restore.is_empty() {
            native_proxy::write_dns(&restore)?;
        }
        fs::remove_file(path)?;
        *lease = None;
    }
    Ok(())
}

pub(super) fn enable(path: &Path, lease: &mut Option<Lease>, owner: u32) -> Result<()> {
    let current = native_proxy::read_dns()?;
    let mut next = lease.clone();
    if let Some(previous) = &mut next {
        ensure!(
            previous.owner == owner,
            "DNS 正由其他客户端持有，请先退出该客户端"
        );
        ensure!(
            previous
                .owned
                .iter()
                .all(|(id, value)| current.get(id).is_none_or(|v| v == value)),
            "系统 DNS 已被其他程序修改，请关闭保护后重试"
        );
        for (id, value) in &current {
            previous
                .original
                .entry(id.clone())
                .or_insert_with(|| value.clone());
        }
        previous.owned = desired(&current)?;
    } else {
        next = Some(Lease {
            original: current.clone(),
            owned: desired(&current)?,
            owner,
            started: process_start(owner).context("无法识别 DNS 所属客户端")?,
        });
    }
    // The root-owned journal precedes the system change, including partial
    // apply failures and helper crashes. Preserve the original recovery data.
    let saved = next.as_ref().unwrap();
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("DNS 恢复路径无效")?)?;
    serde_json::to_writer(&mut file, saved)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    *lease = next;
    let applied = (|| -> Result<()> {
        native_proxy::write_dns(&lease.as_ref().unwrap().owned)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if system_servers().is_ok_and(|s| s == ["1.1.1.1", "8.8.8.8"]) {
                return Ok(());
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "macOS 系统 DNS 未切换到保护模式"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    })();
    if let Err(error) = applied {
        let recovered = restore(path, lease);
        return Err(error.context(format!(
            "接管 DNS 失败；恢复：{}",
            recovered.err().map_or("成功".into(), |e| e.to_string())
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn text(value: Value) -> Option<ProxyValue> {
        Some(ProxyValue::Text(value.to_string()))
    }
    #[test]
    fn restores_dhcp_and_static_dns_without_overwriting_external_changes() {
        let original = ProxyState::from([
            ("dhcp".into(), None),
            (
                "static".into(),
                text(json!({"ServerAddresses":["192.168.1.1"],"SearchDomains":["office.lan"]})),
            ),
            ("changed".into(), None),
            ("removed".into(), None),
        ]);
        let owned = desired(&original).unwrap();
        let mut current = owned.clone();
        current.insert(
            "changed".into(),
            text(json!({"ServerAddresses":["9.9.9.9"]})),
        );
        current.remove("removed");
        current.insert("new".into(), None);
        let plan = restoration(
            &current,
            &Lease {
                original: original.clone(),
                owned: owned.clone(),
                owner: 123,
                started: [0, 0],
            },
        );
        assert_eq!(
            plan,
            ProxyState::from([
                ("dhcp".into(), None),
                ("static".into(), original["static"].clone())
            ])
        );
        let static_dns: Value = match &owned["static"] {
            Some(ProxyValue::Text(text)) => serde_json::from_str(text).unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(static_dns["SearchDomains"], json!(["office.lan"]));
        assert_eq!(static_dns["ServerAddresses"], json!(["1.1.1.1", "8.8.8.8"]));
    }
    #[test]
    fn reused_pid_does_not_keep_a_previous_runs_dns_lease_alive() {
        let owner = std::process::id();
        let started = process_start(owner).unwrap();
        let mut lease = Lease {
            original: ProxyState::new(),
            owned: ProxyState::new(),
            owner,
            started,
        };
        assert!(!lease.owner_exited());
        lease.started[0] = lease.started[0].saturating_sub(1);
        assert!(lease.owner_exited());
    }
}
