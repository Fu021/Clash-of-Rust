//! Reversible runtime DNS overlays, preserving native VPN split resolvers.
use super::{cf, native_proxy};
use crate::platform::ProxyState;
#[cfg(test)]
use crate::platform::ProxyValue;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    net::{IpAddr, TcpListener, UdpSocket},
    path::Path,
    ptr,
};

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCreate(
        allocator: cf::Ref,
        name: cf::Ref,
        callback: Option<unsafe extern "C" fn(cf::Ref, cf::Ref, *mut std::ffi::c_void)>,
        context: *mut std::ffi::c_void,
    ) -> cf::Ref;
    fn SCDynamicStoreCopyValue(store: cf::Ref, key: cf::Ref) -> cf::Ref;
    fn SCDynamicStoreCopyMultiple(store: cf::Ref, keys: cf::Ref, patterns: cf::Ref) -> cf::Ref;
    fn SCDynamicStoreSetMultiple(
        store: cf::Ref,
        values: cf::Ref,
        remove: cf::Ref,
        notify: cf::Ref,
    ) -> u8;
}

#[path = "dns/overlay.rs"]
mod overlay;
#[path = "dns/relay.rs"]
mod relay;
#[path = "dns/tailscale.rs"]
mod tailscale;
#[path = "dns/watch.rs"]
mod watch;
pub(super) use relay::Relay;
pub(crate) use tailscale::snapshot as split_servers;
pub(super) use watch::Changes;

// resolv.conf clients do not understand ServerPort. The root helper owns this
// listener and relays exclusively to the core's unprivileged loopback port.
pub const PORT: u16 = 53;
const DNS_PATTERNS: &[&str] = &[
    "State:/Network/Service/.*/DNS",
    "Setup:/Network/Service/.*/DNS",
];

pub fn available_port() -> Result<u16> {
    for _ in 0..32 {
        let tcp = TcpListener::bind(("127.0.0.1", 0))?;
        let port = tcp.local_addr()?.port();
        if UdpSocket::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    anyhow::bail!("无法为保护 DNS 分配本机端口")
}

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

fn services() -> Result<overlay::Services> {
    let store = dynamic_store()?;
    let patterns = cf::plist(&json!(DNS_PATTERNS))?;
    let values =
        cf::Owned::new(unsafe { SCDynamicStoreCopyMultiple(store.0, ptr::null(), patterns.0) })?;
    let values = cf::json(values.0)?;
    let values = values.as_object().context("系统 DNS 服务列表无效")?;
    ensure!(values.len() <= 512, "系统 DNS 服务列表过大");
    Ok(values
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn write_multiple(values: &overlay::Services, remove: &[String]) -> Result<()> {
    if values.is_empty() && remove.is_empty() {
        return Ok(());
    }
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "DNS 必须通过已授权的网络助手修改"
    );
    let store = dynamic_store()?;
    let values = cf::plist(&serde_json::to_value(values)?)?;
    let remove = cf::plist(&json!(remove))?;
    ensure!(
        unsafe { SCDynamicStoreSetMultiple(store.0, values.0, remove.0, ptr::null()) } != 0,
        "无法原子更新 macOS 保护 DNS"
    );
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
    #[serde(default)]
    uid: u32,
    #[serde(default)]
    port: u16,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct ResolverLease {
    key: String,
    owned: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    original: Option<Value>,
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
    pub(super) fn port(&self) -> u16 {
        self.port
    }
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
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "DNS 恢复记录过大");
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

fn ensure_nonconflicting_split(split: &[crate::claude_check::protection::SplitDns]) -> Result<()> {
    ensure!(
        split.iter().all(|resolver| {
            !crate::claude_check::protection::protected_dns_split(&resolver.domain)
        }),
        "Tailscale 分域与保护范围重叠，无法可靠阻止 DNS 回退，未启用保护"
    );
    Ok(())
}

fn save(path: &Path, lease: &Lease) -> Result<()> {
    let bytes = serde_json::to_vec(lease)?;
    ensure!(bytes.len() <= 1024 * 1024, "DNS 恢复记录过大");
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("DNS 恢复路径无效")?)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

pub(super) fn refresh(path: &Path, lease: &mut Option<Lease>) -> Result<()> {
    let Some(previous) = lease.as_ref() else {
        return Ok(());
    };
    ensure!(previous.port >= 1024, "请先关闭旧版保护后重试");
    let current = services()?;
    let prefix = format!(
        "State:/Network/Service/org.clashofrust.protection.{}",
        previous.uid,
    );
    let sources = overlay::originals(&current, &previous.resolvers, &prefix);
    let (plan, fault) = match overlay::plan(&sources, &prefix, PORT) {
        Ok(plan) => (plan, None),
        Err(error) => (overlay::quarantine(&sources, &prefix, PORT), Some(error)),
    };
    let mut values = overlay::Services::new();
    let mut remove = Vec::new();
    for entry in &previous.resolvers {
        if !plan.iter().any(|next| next.key == entry.key)
            && let Some(value) = current.get(&entry.key)
        {
            if let Some(restored) = overlay::restored(value, entry) {
                if restored != *value {
                    values.insert(entry.key.clone(), restored);
                }
            } else if *value == entry.owned {
                remove.push(entry.key.clone());
            }
        }
    }
    for entry in &plan {
        if current.get(&entry.key) == Some(&entry.owned) {
            continue;
        }
        if entry.original.is_none() {
            ensure!(
                current.get(&entry.key).is_none_or(|value| {
                    previous
                        .resolvers
                        .iter()
                        .any(|old| old.key == entry.key && old.owned == *value)
                }),
                "保护 DNS 记录被其他程序修改，已停止接管"
            );
        }
        values.insert(entry.key.clone(), entry.owned.clone());
    }
    if values.is_empty() && remove.is_empty() && plan == previous.resolvers {
        return fault.map_or(Ok(()), Err);
    }
    let mut next = previous.clone();
    next.resolvers = plan;
    // Write the recovery journal before publishing the entire overlay in one
    // transaction. Removed network services are never recreated by recovery.
    let mut pending = next.clone();
    pending.resolvers.extend(
        previous
            .resolvers
            .iter()
            .filter(|old| !next.resolvers.contains(old))
            .cloned(),
    );
    save(path, &pending)?;
    *lease = Some(pending);
    write_multiple(&values, &remove)?;
    save(path, &next)?;
    *lease = Some(next);
    fault.map_or(Ok(()), Err)
}

pub(super) fn verify(lease: &Option<Lease>, port: u16) -> Result<()> {
    ensure!(
        lease.as_ref().is_some_and(|lease| lease.port == port),
        "保护 DNS 恢复记录缺失或监听端口不匹配"
    );
    let current = dynamic_value("State:/Network/Global/DNS")?;
    ensure!(
        current.as_ref().is_some_and(|value| {
            value["ServerAddresses"] == json!(["127.0.0.1"]) && value["ServerPort"] == PORT
        }),
        "系统 DNS 尚未进入保护入口"
    );
    Ok(())
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
        let current = services()?;
        let mut values = overlay::Services::new();
        let mut remove = Vec::new();
        for (key, value) in &current {
            let entries = owned.resolvers.iter().filter(|entry| entry.key == *key);
            let entry = entries
                .clone()
                .find(|entry| entry.owned == *value)
                .or_else(|| entries.clone().next());
            if let Some(entry) = entry {
                if let Some(restored) = overlay::restored(value, entry) {
                    if restored != *value {
                        values.insert(key.clone(), restored);
                    }
                } else if owned
                    .resolvers
                    .iter()
                    .any(|entry| entry.key == *key && entry.owned == *value)
                {
                    remove.push(key.clone());
                }
            }
        }
        write_multiple(&values, &remove)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while dynamic_value("State:/Network/Global/DNS")?
            .as_ref()
            .is_some_and(|value| {
                value["ServerAddresses"] == json!(["127.0.0.1"]) && value["ServerPort"] == PORT
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
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        *lease = None;
    }
    Ok(())
}

pub(super) fn enable(
    path: &Path,
    lease: &mut Option<Lease>,
    owner: u32,
    uid: u32,
    port: u16,
) -> Result<()> {
    ensure!(port >= 1024, "保护 DNS 本机端口无效");
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(500),
    )
    .context("保护 DNS 本机监听尚未就绪")?;
    if let Some(previous) = lease.as_ref() {
        ensure!(
            previous.owner == owner,
            "DNS 正由其他客户端持有，请先退出该客户端"
        );
        ensure!(previous.port == port, "请先关闭旧版保护后重试");
    } else {
        let snapshot = split_servers()?;
        ensure!(
            !snapshot.protected_conflict,
            "系统分域 DNS 与保护范围重叠，无法可靠阻止 DNS 回退，未启用保护"
        );
        ensure_nonconflicting_split(&snapshot.split_dns)?;
        let prefix = format!("State:/Network/Service/org.clashofrust.protection.{uid}");
        ensure!(
            services()?.keys().all(|key| !key.starts_with(&prefix)),
            "保护 DNS 解析器已存在，请先退出其他客户端"
        );
        *lease = Some(Lease {
            original: ProxyState::new(),
            owned: ProxyState::new(),
            owner,
            started: process_start(owner).context("无法识别 DNS 所属客户端")?,
            resolvers: vec![],
            uid,
            port,
        });
    }
    let applied = (|| -> Result<()> {
        refresh(path, lease)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let current = dynamic_value("State:/Network/Global/DNS")?;
            if current.as_ref().is_some_and(|value| {
                value["ServerAddresses"] == json!(["127.0.0.1"]) && value["ServerPort"] == PORT
            }) {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!("macOS 系统 DNS 未切换到保护模式");
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
                uid: 501,
                port: 25353,
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
            uid: 501,
            port: 25353,
        };
        assert!(!lease.owner_exited());
        lease.started[0] = lease.started[0].saturating_sub(1);
        assert!(lease.owner_exited());
    }
    #[test]
    fn older_dns_journals_remain_readable_for_cleanup() {
        let legacy: Lease = serde_json::from_value(json!({
            "original": {}, "owned": {}, "owner": 123, "started": [0, 0]
        }))
        .unwrap();
        assert!(legacy.resolvers.is_empty());
        assert_eq!(legacy.port, 0);
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
