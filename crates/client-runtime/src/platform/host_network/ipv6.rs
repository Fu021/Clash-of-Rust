use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{collections::BTreeMap, io::Read, net::Ipv6Addr, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Observation {
    pub state: &'static str,
    pub reason: &'static str,
    interfaces: BTreeMap<String, Interface>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Interface {
    index: u32,
    up: bool,
    usable: usize,
    tentative: usize,
    duplicate: usize,
    #[serde(skip)]
    addresses: Vec<(Ipv6Addr, u32, u32)>,
}

impl Observation {
    pub(crate) fn read() -> Result<Self> {
        read_at(Path::new("/proc/net/if_inet6"), Path::new("/sys/class/net"))
    }

    pub(crate) fn report() -> serde_json::Value {
        match Self::read() {
            Ok(observation) => serde_json::json!(observation),
            Err(_) => serde_json::json!({"state":"unknown","reason":"ipv6_address_read_failed"}),
        }
    }
}

fn read_at(address_path: &Path, sysfs: &Path) -> Result<Observation> {
    let mut text = String::new();
    std::fs::File::open(address_path)?
        .take(256 * 1024 + 1)
        .read_to_string(&mut text)?;
    ensure!(text.len() <= 256 * 1024, "IPv6 地址列表过大");
    parse(&text, sysfs)
}

pub(crate) fn parse(text: &str, sysfs: &Path) -> Result<Observation> {
    let mut interfaces = BTreeMap::<String, Interface>::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields: Vec<_> = line.split_whitespace().collect();
        ensure!(fields.len() == 6, "IPv6 地址记录无效");
        let address = Ipv6Addr::from(u128::from_str_radix(fields[0], 16)?);
        let index = u32::from_str_radix(fields[1], 16)?;
        let prefix = u32::from_str_radix(fields[2], 16)?;
        let scope = u32::from_str_radix(fields[3], 16)?;
        let flags = u32::from_str_radix(fields[4], 16)?;
        let name = fields[5];
        ensure!(
            !name.is_empty()
                && name.len() <= 64
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
                && fields[0].len() == 32
                && index != 0
                && prefix <= 128,
            "IPv6 接口记录无效"
        );
        if scope != 0
            || address.is_loopback()
            || address.is_unspecified()
            || address.is_multicast()
            || address.is_unicast_link_local()
        {
            continue;
        }
        let path = sysfs.join(name);
        match std::fs::metadata(path.join("tun_flags")) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && path.is_dir() => {}
            Err(error) => return Err(error).context("读取 IPv6 接口类型失败"),
        }
        let interface_flags = std::fs::read_to_string(path.join("flags"))?;
        let mut up =
            u32::from_str_radix(interface_flags.trim().trim_start_matches("0x"), 16)? & 1 != 0;
        match std::fs::read_to_string(path.join("operstate")) {
            Ok(state) if matches!(state.trim(), "down" | "notpresent" | "lowerlayerdown") => {
                up = false
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("读取 IPv6 接口状态失败"),
        }
        let interface = interfaces.entry(name.into()).or_insert_with(|| Interface {
            index,
            up,
            usable: 0,
            tentative: 0,
            duplicate: 0,
            addresses: Vec::new(),
        });
        ensure!(
            interface.index == index && interface.up == up,
            "IPv6 接口采样期间发生变化"
        );
        interface.addresses.push((address, prefix, flags));
        if flags & 0x08 != 0 {
            interface.duplicate += 1;
        } else if flags & 0x40 != 0 {
            interface.tentative += 1;
        } else if up {
            interface.usable += 1;
        }
    }
    for interface in interfaces.values_mut() {
        interface.addresses.sort();
    }
    let available = interfaces.values().any(|interface| interface.usable > 0);
    Ok(Observation {
        state: if available {
            "available"
        } else {
            "unavailable"
        },
        reason: if available {
            "usable_uplink_address"
        } else {
            "no_usable_uplink_address"
        },
        interfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for name in ["lo", "eth0", "Meta", "tailscale0"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
            std::fs::write(root.path().join(name).join("flags"), "0x1").unwrap();
        }
        for name in ["Meta", "tailscale0"] {
            std::fs::write(root.path().join(name).join("tun_flags"), "0x1").unwrap();
        }
        root
    }

    #[test]
    fn no_addresses_loopback_link_local_and_tun_addresses_do_not_prove_uplink_ipv6() {
        let root = fixture();
        for text in [
            "",
            "00000000000000000000000000000001 01 80 10 80 lo\n",
            "fe800000000000000000000000000001 02 40 20 80 eth0\n",
            "fdfe0000000000000000000000000001 03 7e 00 80 Meta\n",
            "fd7a115ca1e000000000000000000001 04 80 00 80 tailscale0\n",
        ] {
            assert_eq!(parse(text, root.path()).unwrap().state, "unavailable");
        }
    }

    #[test]
    fn address_read_failure_is_unknown_and_does_not_masquerade_as_ipv6_absence() {
        let root = fixture();
        assert!(read_at(&root.path().join("missing"), root.path()).is_err());
        for text in [
            "broken",
            "20010db8000000000000000000000001 02 40 00 80 ../private\n",
            "20010db8000000000000000000000001 02 ff 00 80 eth0\n",
            "20010db8000000000000000000000001 02 40 00 80 missing\n",
        ] {
            assert!(parse(text, root.path()).is_err());
        }
    }

    #[test]
    fn tentative_duplicate_down_and_deprecated_addresses_have_distinct_outcomes() {
        let root = fixture();
        for (flags, expected) in [
            ("40", "unavailable"),
            ("48", "unavailable"),
            ("08", "unavailable"),
            ("80", "available"),
            ("a0", "available"),
        ] {
            let text = format!("20010db8000000000000000000000001 02 40 00 {flags} eth0\n");
            assert_eq!(parse(&text, root.path()).unwrap().state, expected);
        }
        std::fs::write(root.path().join("eth0/flags"), "0x0").unwrap();
        assert_eq!(
            parse(
                "fd000000000000000000000000000001 02 40 00 80 eth0\n",
                root.path()
            )
            .unwrap()
            .state,
            "unavailable"
        );
        std::fs::write(root.path().join("eth0/flags"), "0x1").unwrap();
        std::fs::write(root.path().join("eth0/operstate"), "down").unwrap();
        assert_eq!(
            parse(
                "20010db8000000000000000000000001 02 40 00 80 eth0\n",
                root.path()
            )
            .unwrap()
            .state,
            "unavailable"
        );
    }

    #[test]
    fn address_changes_are_detectable_without_exporting_private_addresses() {
        let root = fixture();
        let first = parse(
            "20010db8000000000000000000000001 02 40 00 80 eth0\n",
            root.path(),
        )
        .unwrap();
        let next = parse(
            "20010db8000000000000000000000002 02 40 00 80 eth0\n",
            root.path(),
        )
        .unwrap();
        assert_ne!(first, next);
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&next).unwrap()
        );
        assert!(!serde_json::to_string(&first).unwrap().contains("2001"));
        assert_eq!(
            parse(
                "fd000000000000000000000000000001 02 40 00 80 eth0\n",
                root.path()
            )
            .unwrap()
            .state,
            "available"
        );
    }
}
