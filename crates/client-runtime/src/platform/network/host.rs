//! Read-only view of the addresses and resolvers of the physical default-route
//! interfaces. The core's own TUN uses split routes, so it never counts as one.
use super::Routes;
use anyhow::Result;
use std::net::IpAddr;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostInfo {
    pub addresses: Vec<IpAddr>,
    pub dns: Vec<IpAddr>,
}

fn global(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()),
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                // fe80::/10 link-local and the fec0:0:0:ffff::/64 legacy resolver anycasts.
                || first & 0xffc0 == 0xfe80
                || first == 0xfec0)
        }
    }
}

/// IPv4 first, then at most one IPv6; resolvers keep the system's own order.
fn tidy(info: HostInfo) -> HostInfo {
    let mut addresses: Vec<_> = info
        .addresses
        .into_iter()
        .filter(|ip| global(*ip))
        .collect();
    addresses.sort_by_key(|ip| ip.is_ipv6());
    addresses.dedup();
    if let Some(first) = addresses.iter().position(IpAddr::is_ipv6) {
        addresses.truncate(first + 1);
    }
    let mut dns = Vec::new();
    for ip in info.dns {
        if global(ip) && !dns.contains(&ip) {
            dns.push(ip);
        }
    }
    dns.truncate(4);
    HostInfo { addresses, dns }
}

#[cfg(any(unix, test))]
fn parse_resolv(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            (fields.next()? == "nameserver").then_some(())?;
            // Scoped link-local resolvers (`fe80::1%eth0`) fail to parse and are skipped.
            fields.next()?.parse().ok()
        })
        .collect()
}

#[cfg(unix)]
fn resolvers() -> Vec<IpAddr> {
    let read = |path| std::fs::read_to_string(path).map(|text| parse_resolv(&text));
    let mut servers = read("/etc/resolv.conf").unwrap_or_default();
    // A local stub (systemd-resolved) hides the real upstream list.
    if servers.iter().all(|ip| ip.is_loopback())
        && let Ok(upstream) = read("/run/systemd/resolve/resolv.conf")
        && !upstream.is_empty()
    {
        servers = upstream;
    }
    servers
}

#[cfg(unix)]
pub(super) fn read(routes: &Routes) -> Result<HostInfo> {
    use std::{ffi::CStr, ptr};
    struct List(*mut libc::ifaddrs);
    impl Drop for List {
        fn drop(&mut self) {
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let mut head = ptr::null_mut();
    anyhow::ensure!(
        unsafe { libc::getifaddrs(&mut head) } == 0,
        "读取网卡地址失败"
    );
    let list = List(head);
    let mut addresses = Vec::new();
    let mut cursor = list.0;
    while !cursor.is_null() {
        let entry = unsafe { &*cursor };
        cursor = entry.ifa_next;
        if entry.ifa_addr.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }.to_string_lossy();
        if !routes.iter().any(|route| route.interface == name) {
            continue;
        }
        // Only the union arm selected by the reported family is read.
        match i32::from(unsafe { (*entry.ifa_addr).sa_family }) {
            libc::AF_INET => {
                let socket: libc::sockaddr_in =
                    unsafe { ptr::read_unaligned(entry.ifa_addr.cast()) };
                addresses.push(IpAddr::from(socket.sin_addr.s_addr.to_ne_bytes()));
            }
            libc::AF_INET6 => {
                let socket: libc::sockaddr_in6 =
                    unsafe { ptr::read_unaligned(entry.ifa_addr.cast()) };
                addresses.push(IpAddr::from(socket.sin6_addr.s6_addr));
            }
            _ => {}
        }
    }
    Ok(tidy(HostInfo {
        addresses,
        dns: resolvers(),
    }))
}

#[cfg(windows)]
pub(super) fn read(routes: &Routes) -> Result<HostInfo> {
    use std::{ffi::c_void, ptr};
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::{GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH},
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6},
    };
    const SKIP_ANYCAST_MULTICAST: u32 = 0x0002 | 0x0004;
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    fn ip(socket: *const SOCKADDR) -> Option<IpAddr> {
        if socket.is_null() {
            return None;
        }
        // The family selects the structure that was actually written.
        match unsafe { (*socket).sa_family } {
            AF_INET => {
                let socket = unsafe { ptr::read_unaligned(socket.cast::<SOCKADDR_IN>()) };
                Some(IpAddr::from(
                    unsafe { socket.sin_addr.S_un.S_addr }.to_ne_bytes(),
                ))
            }
            AF_INET6 => {
                let socket = unsafe { ptr::read_unaligned(socket.cast::<SOCKADDR_IN6>()) };
                Some(IpAddr::from(unsafe { socket.sin6_addr.u.Byte }))
            }
            _ => None,
        }
    }
    let mut size = 16 * 1024_u32;
    let mut buffer: Vec<u64> = Vec::new();
    let mut status = ERROR_BUFFER_OVERFLOW;
    for _ in 0..4 {
        buffer.resize(size as usize / 8 + 1, 0);
        status = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_UNSPEC),
                SKIP_ANYCAST_MULTICAST,
                ptr::null::<c_void>(),
                buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut size,
            )
        };
        if status != ERROR_BUFFER_OVERFLOW {
            break;
        }
    }
    anyhow::ensure!(status == 0, "读取网卡地址失败：{status}");
    let mut info = HostInfo::default();
    let mut adapter = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    let mut visited = 0;
    while !adapter.is_null() && visited < 1024 {
        visited += 1;
        let entry = unsafe { &*adapter };
        adapter = entry.Next;
        let luid = unsafe { entry.Luid.Value }.to_string();
        if !routes.iter().any(|route| route.interface == luid) {
            continue;
        }
        let mut unicast = entry.FirstUnicastAddress;
        while !unicast.is_null() {
            let item = unsafe { &*unicast };
            info.addresses.extend(ip(item.Address.lpSockaddr));
            unicast = item.Next;
        }
        let mut server = entry.FirstDnsServerAddress;
        while !server.is_null() {
            let item = unsafe { &*server };
            info.dns.extend(ip(item.Address.lpSockaddr));
            server = item.Next;
        }
    }
    Ok(tidy(info))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn read(_routes: &Routes) -> Result<HostInfo> {
    Ok(HostInfo::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolv_conf_keeps_only_parsable_nameservers() {
        let text = "# generated\nsearch lan\nnameserver 192.168.1.1\nnameserver fe80::1%eth0\nnameserver 2001:4860:4860::8888\noptions edns0\n";
        assert_eq!(
            parse_resolv(text),
            vec![
                "192.168.1.1".parse::<IpAddr>().unwrap(),
                "2001:4860:4860::8888".parse().unwrap()
            ]
        );
    }

    #[test]
    fn tidy_drops_local_scopes_and_prefers_one_global_ipv6() {
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        let info = tidy(HostInfo {
            addresses: vec![
                ip("fe80::1"),
                ip("2408:8000::5"),
                ip("169.254.3.4"),
                ip("192.168.1.23"),
                ip("2408:8000::6"),
                ip("127.0.0.1"),
            ],
            dns: vec![
                ip("fec0:0:0:ffff::1"),
                ip("223.5.5.5"),
                ip("223.5.5.5"),
                ip("::1"),
            ],
        });
        assert_eq!(info.addresses, vec![ip("192.168.1.23"), ip("2408:8000::5")]);
        assert_eq!(info.dns, vec![ip("223.5.5.5")]);
    }
}
