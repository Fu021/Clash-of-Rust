use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use super::HolderScope;

fn read_small(path: &Path) -> Option<String> {
    let mut contents = String::new();
    File::open(path)
        .ok()?
        .take(8193)
        .read_to_string(&mut contents)
        .ok()?;
    (contents.len() <= 8192).then_some(contents)
}

fn start_time(process: &Path) -> Option<String> {
    let stat = read_small(&process.join("stat"))?;
    Some(stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.into())
}

fn effective_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn held_interface(info: &str) -> Option<&str> {
    info.lines()
        .find_map(|line| line.strip_prefix("iff:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

fn holders(
    proc_root: &Path,
    interfaces: &[String],
    budget: Duration,
    observer_uid: u32,
    scope: HolderScope,
) -> Value {
    let started = Instant::now();
    let mut result: BTreeMap<String, Vec<Value>> = interfaces
        .iter()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    let mut complete = true;
    let mut skipped_other_users = 0_u64;
    let namespace = std::fs::read_link(proc_root.join("self/ns/net")).ok();
    let Some(namespace) = namespace else {
        return json!({"state":"unavailable","reason":"network_namespace_unavailable","scope":scope.label(),"interfaces":result});
    };
    let Ok(processes) = std::fs::read_dir(proc_root) else {
        return json!({"state":"unavailable","reason":"processes_unavailable","scope":scope.label(),"interfaces":result});
    };
    let mut process_count = 0;
    let mut descriptor_count = 0;
    'processes: for entry in processes {
        if started.elapsed() >= budget {
            complete = false;
            break;
        }
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        process_count += 1;
        if process_count > 4096 {
            complete = false;
            break;
        }
        let process = entry.path();
        let Some(uid) = read_small(&process.join("status"))
            .as_deref()
            .and_then(effective_uid)
        else {
            complete = false;
            continue;
        };
        if scope == HolderScope::CurrentUser && uid != observer_uid {
            skipped_other_users += 1;
            continue;
        }
        match std::fs::read_link(process.join("ns/net")) {
            Ok(value) if value == namespace => {}
            Ok(_) => continue,
            Err(_) => {
                complete = false;
                continue;
            }
        }
        let Some(before) = start_time(&process) else {
            complete = false;
            continue;
        };
        let Ok(descriptors) = std::fs::read_dir(process.join("fd")) else {
            complete = false;
            continue;
        };
        let mut matched = Vec::new();
        for descriptor in descriptors {
            descriptor_count += 1;
            if descriptor_count > 16384 || started.elapsed() >= budget {
                complete = false;
                break 'processes;
            }
            let Ok(descriptor) = descriptor else {
                complete = false;
                continue;
            };
            let target = match std::fs::read_link(descriptor.path()) {
                Ok(target) => target,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            if target != Path::new("/dev/net/tun") {
                continue;
            }
            let Some(info) = read_small(&process.join("fdinfo").join(descriptor.file_name()))
            else {
                complete = false;
                continue;
            };
            if let Some(interface) = held_interface(&info)
                && result.contains_key(interface)
                && !matched.iter().any(|name| name == interface)
            {
                matched.push(interface.to_owned());
            }
        }
        if matched.is_empty() {
            continue;
        }
        if start_time(&process).as_ref() != Some(&before)
            || std::fs::read_link(process.join("ns/net")).ok().as_ref() != Some(&namespace)
            || read_small(&process.join("status"))
                .as_deref()
                .and_then(effective_uid)
                != Some(uid)
        {
            complete = false;
            continue;
        }
        let name = read_small(&process.join("comm")).map(|name| name.trim().to_owned());
        for interface in matched {
            result.get_mut(&interface).unwrap().push(json!({
                "pid":pid,"uid":uid,"process":name,"start_ticks":before,
                "evidence":"tun_fdinfo"
            }));
        }
    }
    for owners in result.values_mut() {
        owners.sort_by_key(|owner| owner["pid"].as_u64());
    }
    json!({
        "state":if complete {"complete"} else {"partial"},
        "scope":scope.label(),
        "skipped_other_users":skipped_other_users,
        "namespace":namespace.to_string_lossy(),
        "interfaces":result
    })
}

pub(super) fn attach_holders(mut routes: Value, scope: HolderScope) -> Value {
    let interfaces: Vec<_> = ["ipv4", "ipv6"]
        .into_iter()
        .filter(|family| routes[*family]["kind"] == "tun")
        .filter_map(|family| routes[family]["interface"].as_str().map(str::to_owned))
        .collect();
    if !interfaces.is_empty() {
        routes["tun_holders"] = holders(
            Path::new("/proc"),
            &interfaces,
            Duration::from_millis(250),
            unsafe { libc::geteuid() },
            scope,
        );
    }
    routes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn process(root: &Path, pid: u32, uid: u32, namespace: &str, interface: &str) {
        let directory = root.join(pid.to_string());
        for name in ["ns", "fd", "fdinfo"] {
            std::fs::create_dir_all(directory.join(name)).unwrap();
        }
        symlink(namespace, directory.join("ns/net")).unwrap();
        symlink("/dev/net/tun", directory.join("fd/7")).unwrap();
        std::fs::write(directory.join("fdinfo/7"), format!("iff:\t{interface}\n")).unwrap();
        std::fs::write(
            directory.join("status"),
            format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
        )
        .unwrap();
        std::fs::write(directory.join("comm"), "mihomo\n").unwrap();
        let fields = std::iter::repeat_n("0", 18).collect::<Vec<_>>().join(" ");
        std::fs::write(
            directory.join("stat"),
            format!("{pid} (name with ) parens) S {fields} 123\n"),
        )
        .unwrap();
    }

    #[test]
    fn holder_identity_requires_the_interface_and_current_namespace() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        std::fs::create_dir_all(root.join("self/ns")).unwrap();
        symlink("net:[1]", root.join("self/ns/net")).unwrap();
        process(root, 11, 0, "net:[1]", "Meta");
        process(root, 12, 1000, "net:[2]", "Meta");
        process(root, 13, 1000, "net:[1]", "unrelated");
        process(root, 14, 1000, "net:[1]", "Meta");
        let value = holders(
            root,
            &["Meta".into()],
            Duration::from_secs(2),
            0,
            HolderScope::CurrentUser,
        );
        assert_eq!(value["state"], "complete");
        assert_eq!(value["scope"], "current_user");
        assert_eq!(value["skipped_other_users"], 3);
        let owners = value["interfaces"]["Meta"].as_array().unwrap();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0]["pid"], 11);
        assert_eq!(owners[0]["uid"], 0);
        assert_eq!(owners[0]["start_ticks"], "123");
        assert_eq!(owners[0]["evidence"], "tun_fdinfo");
        let all_users = holders(
            root,
            &["Meta".into()],
            Duration::from_secs(2),
            0,
            HolderScope::AllUsers,
        );
        assert_eq!(all_users["state"], "complete");
        assert_eq!(all_users["scope"], "all_users");
        assert_eq!(all_users["skipped_other_users"], 0);
        let owners = all_users["interfaces"]["Meta"].as_array().unwrap();
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0]["pid"], 11);
        assert_eq!(owners[1]["pid"], 14);
        assert_eq!(owners[1]["uid"], 1000);
    }

    #[test]
    fn missing_permissions_and_exhausted_budget_never_claim_no_owner() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let interfaces = ["Meta".into()];
        for scope in [HolderScope::CurrentUser, HolderScope::AllUsers] {
            let value = holders(root, &interfaces, Duration::from_secs(2), 0, scope);
            assert_eq!(value["state"], "unavailable");
            assert_eq!(value["scope"], scope.label());
            assert_eq!(value["interfaces"]["Meta"], json!([]));
        }
        std::fs::create_dir_all(root.join("self/ns")).unwrap();
        symlink("net:[1]", root.join("self/ns/net")).unwrap();
        std::fs::create_dir(root.join("22")).unwrap();
        let value = holders(
            root,
            &interfaces,
            Duration::from_secs(2),
            0,
            HolderScope::CurrentUser,
        );
        assert_eq!(value["state"], "partial");
        assert_eq!(value["interfaces"]["Meta"], json!([]));
        assert_eq!(
            holders(
                root,
                &interfaces,
                Duration::ZERO,
                0,
                HolderScope::CurrentUser,
            )["state"],
            "partial"
        );
        assert_eq!(held_interface("flags:\t1\n"), None);
    }
}
