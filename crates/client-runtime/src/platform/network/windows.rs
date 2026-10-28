use super::{Route, Routes};
use anyhow::{Result, ensure};
use std::{collections::BTreeMap, ptr};
use windows_sys::Win32::{
    NetworkManagement::IpHelper::{
        FreeMibTable, GetIfEntry2, GetIpForwardTable2, GetIpInterfaceEntry, MIB_IF_ROW2,
        MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_ROW,
    },
    Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_INET},
};

struct Table(*mut MIB_IPFORWARD_TABLE2);
impl Drop for Table {
    fn drop(&mut self) {
        unsafe { FreeMibTable(self.0.cast()) };
    }
}

fn address(value: &SOCKADDR_INET) -> Result<String> {
    // Only read the arm selected by the family returned by IP Helper.
    unsafe {
        match value.si_family {
            AF_INET => Ok(
                std::net::Ipv4Addr::from(value.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()).to_string(),
            ),
            AF_INET6 => Ok(std::net::Ipv6Addr::from(value.Ipv6.sin6_addr.u.Byte).to_string()),
            _ => anyhow::bail!("默认路由地址类型无效"),
        }
    }
}

pub(super) fn read() -> Result<Routes> {
    let mut table = ptr::null_mut();
    let status = unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table) };
    ensure!(status == 0, "读取系统默认路由失败：{status}");
    ensure!(!table.is_null(), "系统路由表为空指针");
    let table = Table(table);
    let count = unsafe { (*table.0).NumEntries as usize };
    ensure!(count <= 65536, "系统路由表过大");
    let rows = unsafe { std::slice::from_raw_parts((*table.0).Table.as_ptr(), count) };
    let mut best = BTreeMap::<u16, (u32, Routes)>::new();
    for row in rows {
        if row.DestinationPrefix.PrefixLength != 0 {
            continue;
        }
        let family = unsafe { row.DestinationPrefix.Prefix.si_family };
        if !matches!(family, AF_INET | AF_INET6) {
            continue;
        }
        let mut interface = MIB_IF_ROW2 {
            InterfaceLuid: row.InterfaceLuid,
            ..Default::default()
        };
        let status = unsafe { GetIfEntry2(&mut interface) };
        ensure!(status == 0, "读取默认路由接口失败：{status}");
        // Match the core's physical default-route selection. Its own TUN uses
        // split routes; a virtual/loopback default must not trigger a reload loop.
        if interface.OperStatus != 1 || matches!(interface.Type, 24 | 53) {
            continue;
        }
        let mut ip = MIB_IPINTERFACE_ROW {
            Family: family,
            InterfaceLuid: row.InterfaceLuid,
            ..Default::default()
        };
        let status = unsafe { GetIpInterfaceEntry(&mut ip) };
        ensure!(status == 0, "读取默认路由优先级失败：{status}");
        if !ip.Connected {
            continue;
        }
        let metric = row.Metric.saturating_add(ip.Metric);
        let gateway = address(&row.NextHop)?;
        if gateway == "25.255.255.254" {
            continue;
        }
        let route = Route {
            interface: unsafe { row.InterfaceLuid.Value }.to_string(),
            gateway,
        };
        let entry = best.entry(family).or_insert_with(|| (metric, Vec::new()));
        if metric < entry.0 {
            *entry = (metric, vec![route]);
        } else if metric == entry.0 {
            entry.1.push(route);
        }
    }
    Ok(best.into_values().flat_map(|(_, routes)| routes).collect())
}
