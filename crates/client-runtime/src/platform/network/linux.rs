use super::{Route, Routes};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};

pub(super) fn read() -> Result<Routes> {
    let v4 = fs::read_to_string("/proc/net/route").context("读取 IPv4 路由失败")?;
    let v6 = match fs::read_to_string("/proc/net/ipv6_route") {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("读取 IPv6 路由失败"),
    };
    let mut routes = parse(&v4, &v6)?;
    routes.retain(|route| {
        let path = Path::new("/sys/class/net")
            .join(&route.interface)
            .join("operstate");
        // Unknown is normal on some VPN/point-to-point interfaces.
        !fs::read_to_string(path)
            .is_ok_and(|state| matches!(state.trim(), "down" | "notpresent" | "lowerlayerdown"))
    });
    Ok(routes)
}

fn parse(v4: &str, v6: &str) -> Result<Routes> {
    let mut routes = Vec::new();
    for line in v4.lines().skip(1).filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(fields.len() >= 8, "IPv4 路由记录无效");
        let flags = u32::from_str_radix(fields[3], 16)?;
        if fields[1] == "00000000"
            && fields[7] == "00000000"
            && flags & 0x201 == 1
            && fields[0] != "lo"
        {
            routes.push(Route {
                interface: fields[0].into(),
                gateway: format!("4:{}", fields[2]),
            });
        }
    }
    for line in v6.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(fields.len() >= 10, "IPv6 路由记录无效");
        let flags = u32::from_str_radix(fields[8], 16)?;
        if fields[0] == "00000000000000000000000000000000"
            && fields[1] == "00"
            && flags & 0x201 == 1
            && fields[9] != "lo"
        {
            routes.push(Route {
                interface: fields[9].into(),
                gateway: format!("6:{}", fields[4]),
            });
        }
    }
    Ok(routes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_ignore_tun_split_routes_and_tailnet_routes() {
        let v4 = "Iface Destination Gateway Flags RefCnt Use Metric Mask\n\
eth0 00000000 0100000A 0003 0 0 100 00000000\n\
Meta 00000000 010012C6 0003 0 0 0 00000080\n\
tailscale0 00004064 00000000 0001 0 0 0 0000C0FF\n\
eth1 00000000 0200000A 0201 0 0 100 00000000\n";
        let v6 = "\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000000 00000000 00000003 eth0\n\
00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000000 00000000 00200200 lo\n";
        let routes = parse(v4, v6).unwrap();
        assert_eq!(routes.len(), 2);
        assert!(routes.iter().all(|route| route.interface == "eth0"));
        assert!(parse("Iface\nbroken", "").is_err());
        assert!(parse("Iface\n", "broken").is_err());
        assert!(parse("Iface\n", "").unwrap().is_empty());
    }
}
