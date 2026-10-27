//! Reversible supplemental DNS for protection, preserving VPN split resolvers.
use super::{cf, native_proxy};
use crate::platform::ProxyState;
#[cfg(test)]
use crate::platform::ProxyValue;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io::Read, net::IpAddr, path::Path, ptr};

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(
        allocator: cf::Ref,
        name: cf::Ref,
        callback: Option<unsafe extern "C" fn(cf::Ref, cf::Ref, *mut std::ffi::c_void)>,
        context: *mut std::ffi::c_void,
    ) -> cf::Ref;
    fn SCDynamicStoreCopyValue(store: cf::Ref, key: cf::Ref) -> cf::Ref;
    fn SCDynamicStoreSetValue(store: cf::Ref, key: cf::Ref, value: cf::Ref) -> u8;
    fn SCDynamicStoreRemoveValue(store: cf::Ref, key: cf::Ref) -> u8;
}

const SERVERS: [&str; 2] = ["1.1.1.1", "8.8.8.8"];
#[path = "dns/tailscale.rs"]
mod tailscale;
pub(crate) use tailscale::snapshot as split_servers;

fn dynamic_store() -> Result<cf::Owned> {
    let name = cf::string("Clash of Rust protection")?;
    cf::Owned::new(unsafe { SCDynamicStoreCreate(ptr::null(), name.0, None, ptr::null_mut()) })
}

fn dynamic_value(key: &str) -> Result<Option<Value>> {
    let store = dynamic_store()?;
    let key = cf::string(key)?;
    let value = unsafe { SCDynamicStoreCopyValue(store.0, key.0) };
    if value.is_null() {
        return Ok(None);
    }
    let value = cf::Owned::new(value)?;
    Ok(Some(cf::json(value.0)?))
}

fn write_resolver(key: &str, value: Option<&Value>) -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "DNS 必须通过已授权的网络助手修改"
    );
    let store = dynamic_store()?;
    let key = cf::string(key)?;
    let ok = if let Some(value) = value {
        let value = cf::plist(value)?;
        unsafe { SCDynamicStoreSetValue(store.0, key.0, value.0) }
    } else {
        unsafe { SCDynamicStoreRemoveValue(store.0, key.0) }
    };
    ensure!(ok != 0, "无法更新 macOS 保护 DNS 解析器");
    Ok(())
}

pub fn system_servers() -> Result<Vec<String>> {
    let value =
        dynamic_value("State:/Network/Global/DNS")?.context("无法读取 macOS 当前 DNS 配置")?;
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resolvers: Vec<ResolverLease>,
}
#[derive(Clone, Serialize, Deserialize)]
struct ResolverLease {
    key: String,
    owned: Value,
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

#[cfg(test)]
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

fn resolver_config(split: &[crate::claude_check::protection::SplitDns]) -> Value {
    use crate::claude_check::protection::{
        ASSOCIATED_DOMAINS, DIAGNOSTIC_DNS_SUFFIXES, DIAGNOSTIC_HOSTS, OWNED_DOMAINS,
        protected_dns_host,
    };
    // Install a separate supplemental default without changing the VPN's
    // private suffixes or network preferences. macOS can retain another
    // default even with a lower SearchOrder; enable verifies and rolls back.
    // "*" also covers interface-scoped queries.
    let mut domains = vec![""];
    domains.extend(OWNED_DOMAINS);
    domains.extend(ASSOCIATED_DOMAINS);
    domains.extend(DIAGNOSTIC_HOSTS);
    domains.extend(DIAGNOSTIC_DNS_SUFFIXES);
    for resolver in split {
        if protected_dns_host(&resolver.domain) && !domains.contains(&resolver.domain.as_str()) {
            domains.push(&resolver.domain);
        }
    }
    json!({
        "ServerAddresses": SERVERS,
        "SearchOrder": 1,
        "SupplementalMatchDomains": domains,
        "SupplementalMatchDomainsNoSearch": 1,
        "InterfaceName": "*",
    })
}

fn resolver_plan(uid: u32) -> Result<Vec<ResolverLease>> {
    let split = split_servers()?.split_dns;
    ensure_nonconflicting_split(&split)?;
    let search = dynamic_value("State:/Network/Global/DNS")?
        .and_then(|value| value.get("SearchDomains").cloned());
    Ok(resolver_plan_for(uid, &split, search))
}

fn ensure_nonconflicting_split(split: &[crate::claude_check::protection::SplitDns]) -> Result<()> {
    ensure!(
        split.iter().all(|resolver| {
            !crate::claude_check::protection::protected_dns_split(&resolver.domain)
        }),
        "Tailscale 分域与保护范围重叠，无法可靠阻止 DNS 回退，未启用保护"
    );
    Ok(())
}

fn resolver_plan_for(
    uid: u32,
    split: &[crate::claude_check::protection::SplitDns],
    search: Option<Value>,
) -> Vec<ResolverLease> {
    let prefix = format!("State:/Network/Service/org.clashofrust.protection.{uid}");
    let mut public = resolver_config(split);
    if let Some(search) = search {
        public["SearchDomains"] = search;
    }
    // Keep the VPN's original split resolvers instead of copying their server
    // addresses: its native resolver owns the interface binding and updates.
    vec![ResolverLease {
        key: format!("{prefix}/DNS"),
        owned: public,
    }]
}

fn resolver_id(key: &str) -> &str {
    key.trim_start_matches("State:/Network/Service/")
        .trim_end_matches("/DNS")
}

fn is_primary(value: &Value, resolver: &ResolverLease) -> bool {
    value["ServerAddresses"] == resolver.owned["ServerAddresses"]
        && value["__CONFIGURATION_ID__"]
            .as_str()
            .is_some_and(|id| id.contains(resolver_id(&resolver.key)))
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
        for resolver in owned.resolvers.iter().rev() {
            if dynamic_value(&resolver.key)?.as_ref() == Some(&resolver.owned) {
                write_resolver(&resolver.key, None)?;
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while dynamic_value("State:/Network/Global/DNS")?
            .as_ref()
            .is_some_and(|value| {
                owned
                    .resolvers
                    .iter()
                    .any(|resolver| is_primary(value, resolver))
            })
        {
            ensure!(
                std::time::Instant::now() < deadline,
                "macOS 正在恢复原 DNS，请稍后重试"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // Older helpers modified persistent service DNS. Keep their journals
        // readable so an upgrade restores DHCP/static settings before takeover.
        if !owned.original.is_empty() {
            let restore = restoration(&native_proxy::read_dns()?, owned);
            if !restore.is_empty() {
                native_proxy::write_dns(&restore)?;
            }
        }
        fs::remove_file(path)?;
        *lease = None;
    }
    Ok(())
}

pub(super) fn enable(path: &Path, lease: &mut Option<Lease>, owner: u32, uid: u32) -> Result<()> {
    let resolvers = if let Some(previous) = lease.as_ref() {
        ensure!(
            previous.owner == owner,
            "DNS 正由其他客户端持有，请先退出该客户端"
        );
        ensure!(!previous.resolvers.is_empty(), "请先关闭旧版保护后重试");
        for resolver in &previous.resolvers {
            ensure!(
                dynamic_value(&resolver.key)?.is_none_or(|value| value == resolver.owned),
                "保护 DNS 已被其他程序修改，请关闭保护后重试"
            );
        }
        previous.resolvers.clone()
    } else {
        let resolvers = resolver_plan(uid)?;
        for resolver in &resolvers {
            ensure!(
                dynamic_value(&resolver.key)?.is_none(),
                "保护 DNS 解析器已存在，请先退出其他客户端"
            );
        }
        resolvers
    };
    let next = Lease {
        original: ProxyState::new(),
        owned: ProxyState::new(),
        owner,
        started: process_start(owner).context("无法识别 DNS 所属客户端")?,
        resolvers,
    };
    // The root-owned journal precedes the system change, including partial
    // apply failures and helper crashes. Preserve the original recovery data.
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("DNS 恢复路径无效")?)?;
    serde_json::to_writer(&mut file, &next)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    *lease = Some(next);
    let applied = (|| -> Result<()> {
        let resolvers = &lease.as_ref().unwrap().resolvers;
        for resolver in resolvers.iter().rev() {
            write_resolver(&resolver.key, Some(&resolver.owned))?;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let current = dynamic_value("State:/Network/Global/DNS")?;
            if current
                .as_ref()
                .is_some_and(|value| is_primary(value, &resolvers[0]))
            {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                let servers_match = current.as_ref().is_some_and(|value| {
                    value["ServerAddresses"] == resolvers[0].owned["ServerAddresses"]
                });
                let source_matches = current.as_ref().is_some_and(|value| {
                    value["__CONFIGURATION_ID__"]
                        .as_str()
                        .is_some_and(|id| id.contains(resolver_id(&resolvers[0].key)))
                });
                let installed =
                    dynamic_value(&resolvers[0].key)?.as_ref() == Some(&resolvers[0].owned);
                anyhow::bail!(
                    "macOS 系统 DNS 未切换到保护模式（临时配置存在：{installed}，服务器匹配：{servers_match}，来源匹配：{source_matches}）"
                );
            }
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
                resolvers: vec![],
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
            resolvers: vec![],
        };
        assert!(!lease.owner_exited());
        lease.started[0] = lease.started[0].saturating_sub(1);
        assert!(lease.owner_exited());
    }
    #[test]
    fn supplemental_default_preserves_private_dns_and_covers_protected_suffixes() {
        let value = resolver_config(&[]);
        assert_eq!(value["ServerAddresses"], json!(SERVERS));
        assert_eq!(value["SearchOrder"], 1);
        assert_eq!(value["InterfaceName"], "*");
        assert_eq!(value["SupplementalMatchDomainsNoSearch"], 1);
        let domains = value["SupplementalMatchDomains"].as_array().unwrap();
        assert_eq!(domains[0], "");
        assert!(domains.contains(&json!("claude.ai")));
        assert!(domains.contains(&json!("bash.ws")));
        assert!(domains.contains(&json!("stun.l.google.com")));
        assert!(!domains.contains(&json!("ts.net")));
        assert!(value["SearchDomains"].is_null());
        let legacy: Lease = serde_json::from_value(json!({
            "original": {}, "owned": {}, "owner": 123, "started": [0, 0]
        }))
        .unwrap();
        assert!(legacy.resolvers.is_empty());
    }
    #[test]
    fn resolver_plan_keeps_private_domains_and_covers_protected_domains() {
        use crate::claude_check::protection::SplitDns;
        let split = ["tail-fixture.ts.net", "private.example"]
            .into_iter()
            .map(|domain| SplitDns {
                domain: domain.into(),
                servers: vec!["100.100.100.100".parse().unwrap()],
                interface: Some("utun4".into()),
            })
            .collect::<Vec<_>>();
        let plan = resolver_plan_for(501, &split, Some(json!(["tail-fixture.ts.net"])));
        assert_eq!(plan.len(), 1);
        assert_eq!(
            plan[0].owned["SearchDomains"],
            json!(["tail-fixture.ts.net"])
        );
        assert!(
            plan[0].owned["SupplementalMatchDomains"]
                .as_array()
                .unwrap()
                .contains(&json!("claude.ai"))
        );
        assert!(
            plan[0].owned["SupplementalMatchDomains"]
                .as_array()
                .unwrap()
                .contains(&json!("bash.ws"))
        );
        assert!(
            !plan[0].owned["SupplementalMatchDomains"]
                .as_array()
                .unwrap()
                .contains(&json!("tail-fixture.ts.net"))
        );
        assert!(
            !plan[0].owned["SupplementalMatchDomains"]
                .as_array()
                .unwrap()
                .contains(&json!("private.example"))
        );
    }
    #[test]
    fn refuses_tailscale_splits_that_can_retry_protected_dns() {
        use crate::claude_check::protection::SplitDns;
        let resolver = |domain: &str| SplitDns {
            domain: domain.into(),
            servers: vec!["100.100.100.100".parse().unwrap()],
            interface: Some("utun4".into()),
        };
        assert!(ensure_nonconflicting_split(&[resolver("tail-fixture.ts.net")]).is_ok());
        for domain in [
            "claude.ai",
            "api.claude.ai",
            "ai",
            "auth0.com",
            "com",
            "bash.ws",
            "ws",
        ] {
            assert!(ensure_nonconflicting_split(&[resolver(domain)]).is_err());
        }
    }
}
