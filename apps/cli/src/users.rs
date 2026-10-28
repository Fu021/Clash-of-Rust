//! User selection happens before the runtime or any user-controlled configuration.
use crate::args::Args;
use anyhow::Result;
use serde_json::Value;

pub fn prepare(args: &Args) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::prepare(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::ensure!(
            args.get("user").is_none() && !args.flag("all-users"),
            "--user 与 --all-users 目前仅适用于 Linux"
        );
        Ok(())
    }
}

pub fn profiles(args: &Args) -> Result<Value> {
    #[cfg(target_os = "linux")]
    {
        linux::profiles(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("跨用户订阅总览目前仅适用于 Linux")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use clash_client_runtime::platform;
    use serde_json::json;
    use std::{
        collections::BTreeSet,
        fs::{File, OpenOptions},
        io::Read,
        os::unix::{
            fs::{MetadataExt, OpenOptionsExt},
            process::CommandExt,
        },
        path::{Path, PathBuf},
        process::Command,
    };

    const INDEX_LIMIT: u64 = 8 * 1024 * 1024;

    #[derive(Debug)]
    struct Account {
        name: String,
        uid: u32,
        home: PathBuf,
    }

    fn account(line: &str) -> Result<Account> {
        let fields: Vec<_> = line.split(':').collect();
        ensure!(fields.len() == 7, "系统用户记录格式无效");
        let home = PathBuf::from(fields[5]);
        ensure!(
            !fields[0].is_empty() && home.is_absolute(),
            "系统用户记录缺少名称或主目录"
        );
        Ok(Account {
            name: fields[0].into(),
            uid: fields[2].parse()?,
            home,
        })
    }

    fn accounts(key: Option<&str>) -> Result<Vec<Account>> {
        let mut command = Command::new("/usr/bin/getent");
        command.args(["--", "passwd"]);
        if let Some(key) = key {
            command.arg(key);
        }
        let output = command
            .output()
            .context("无法查询本机用户（需要 getent）")?;
        ensure!(output.status.success(), "找不到指定的系统用户");
        String::from_utf8(output.stdout)?
            .lines()
            .map(account)
            .collect()
    }

    fn sudo_invokes_cli(command: &str, executable: &Path) -> bool {
        command
            .split_whitespace()
            .next()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .is_some_and(|path| path == executable)
    }

    fn delegated_arguments(args: &Args) -> Vec<String> {
        let mut result: Vec<_> = args.path.split_whitespace().map(str::to_owned).collect();
        for (key, value) in &args.options {
            if key == "user" {
                continue;
            }
            result.push(format!("--{key}"));
            if crate::args::VALUES
                .split_whitespace()
                .any(|name| name == key)
            {
                result.push(value.clone());
            }
        }
        result.push("--".into());
        result.extend(args.positional.iter().cloned());
        result
    }

    pub(super) fn prepare(args: &Args) -> Result<()> {
        let root = platform::is_elevated();
        ensure!(
            root || (args.get("user").is_none() && !args.flag("all-users")),
            "只有 root 可以使用 --user 或 --all-users；普通用户只能管理自己的订阅"
        );
        if !root || args.flag("all-users") {
            return Ok(());
        }
        let executable = std::env::current_exe()?;
        let sudo_user = std::env::var("SUDO_COMMAND")
            .ok()
            .filter(|command| sudo_invokes_cli(command, &executable))
            .and_then(|_| std::env::var("SUDO_UID").ok());
        let selected = args.get("user").map(str::to_owned).or(sudo_user);
        let Some(selected) = selected else {
            return Ok(());
        };
        let target = accounts(Some(&selected))?
            .into_iter()
            .next()
            .context("找不到目标用户")?;
        if target.uid == 0 {
            return Ok(());
        }
        // Never load user YAML in this root process. Only the fixed helper may
        // run elevated; the command and its backend run as the selected account.
        if matches!(args.path.as_str(), "start" | "restart" | "run") && !args.flag("no-core")
            || (args.path == "proxy" && args.positional.first().is_some_and(|v| v == "tun"))
            || (args.path == "claude protect" && args.positional.first().is_some_and(|v| v == "on"))
        {
            platform::authorize_tun_for_user()?;
        }
        let mut command = Command::new("/usr/sbin/runuser");
        command
            .args(["--user", &target.name, "--"])
            .arg(executable)
            .args(delegated_arguments(args));
        for key in [
            "SUDO_UID",
            "SUDO_GID",
            "SUDO_USER",
            "SUDO_COMMAND",
            "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS",
        ] {
            command.env_remove(key);
        }
        for key in [
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            let owned = std::env::var_os(key).is_some_and(|value| {
                let path = PathBuf::from(value);
                path.is_absolute()
                    && path
                        .ancestors()
                        .find_map(|parent| parent.metadata().ok())
                        .is_some_and(|metadata| metadata.uid() == target.uid)
            });
            if args.get("user").is_some() || !owned {
                command.env_remove(key);
            }
        }
        if args.get("user").is_some() {
            command.env_remove("CLASH_OF_RUST_DATA_DIR");
        }
        command
            .env("HOME", &target.home)
            .env("USER", &target.name)
            .env("LOGNAME", &target.name);
        let runtime = PathBuf::from(format!("/run/user/{}", target.uid));
        if runtime
            .metadata()
            .is_ok_and(|m| m.is_dir() && m.uid() == target.uid)
        {
            command.env("XDG_RUNTIME_DIR", &runtime);
            if runtime.join("bus").exists() {
                command.env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    format!("unix:path={}/bus", runtime.display()),
                );
            }
        }
        Err(command.exec()).context("无法切换到目标用户（需要 util-linux 的 runuser）")
    }

    fn read_index(path: &Path, uid: u32) -> Result<Option<Value>> {
        let file: File = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => bail!("无法安全读取用户订阅索引：{}", path.display()),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.uid() == uid && metadata.len() <= INDEX_LIMIT,
            "用户订阅索引的所有者、类型或大小无效：{}",
            path.display()
        );
        let mut bytes = Vec::new();
        file.take(INDEX_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= INDEX_LIMIT, "用户订阅索引过大");
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| anyhow::anyhow!("用户订阅索引格式无效：{}", path.display()))
    }

    pub(super) fn profiles(args: &Args) -> Result<Value> {
        ensure!(platform::is_elevated(), "只有 root 可以查看跨用户订阅");
        let filter = args.get("filter").unwrap_or("").to_lowercase();
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for account in accounts(None)? {
            if !seen.insert(account.uid) {
                continue;
            }
            let root = account.home.join(".local/share/clashofrust");
            let Some(index) = read_index(&root.join("profiles.json"), account.uid)? else {
                continue;
            };
            let profiles: Vec<clash_client_core::config::Profile> =
                serde_json::from_value(index)
                    .map_err(|_| anyhow::anyhow!("用户 {} 的订阅索引格式无效", account.name))?;
            let settings =
                read_index(&root.join("settings.json"), account.uid)?.unwrap_or_default();
            for profile in profiles {
                if !profile.name.to_lowercase().contains(&filter)
                    && !profile.id.contains(&filter)
                    && !account.name.to_lowercase().contains(&filter)
                {
                    continue;
                }
                rows.push(json!({"id":profile.id,"name":profile.name,"owner":account.name,
                    "uid":account.uid,"data_dir":root,"active":settings["active_profile"]==profile.id,
                    "source":profile.source,"updated":profile.updated,"usage":profile.usage}));
            }
        }
        Ok(json!(rows))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn delegation_preserves_literal_arguments_and_removes_only_user_selection() {
            let args = crate::args::parse(
                [
                    "--user",
                    "alice",
                    "profile",
                    "add",
                    "name",
                    "--",
                    "--user=literal",
                ]
                .map(str::to_owned),
            )
            .unwrap();
            let reparsed = crate::args::parse(delegated_arguments(&args)).unwrap();
            assert_eq!(reparsed.path, args.path);
            assert_eq!(reparsed.positional, args.positional);
            assert!(reparsed.get("user").is_none());
            assert!(account("bad").is_err());
            assert!(account("alice:x:1000:1000::relative:/bin/sh").is_err());
            assert_eq!(
                account("alice:x:1000:1000::/home/alice:/bin/sh")
                    .unwrap()
                    .uid,
                1000
            );
        }
        #[test]
        fn root_shells_do_not_inherit_the_sudo_cli_context() {
            let exe = std::env::current_exe().unwrap();
            assert!(sudo_invokes_cli(&format!("{} status", exe.display()), &exe));
            assert!(!sudo_invokes_cli("/bin/bash", &exe));
            assert!(!sudo_invokes_cli(
                "/usr/bin/python3 test-linux-tun.py",
                &exe
            ));
        }
        #[test]
        fn inventory_never_follows_symlinks_or_creates_missing_data() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("profiles.json");
            let uid = dir.path().metadata().unwrap().uid();
            assert!(read_index(&path, uid).unwrap().is_none());
            assert!(!path.exists());
            std::fs::write(&path, b"[]").unwrap();
            assert_eq!(read_index(&path, uid).unwrap(), Some(json!([])));
            assert!(read_index(&path, uid.wrapping_add(1)).is_err());
            let link = dir.path().join("link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_index(&link, uid).is_err());
            assert!(read_index(dir.path(), uid).is_err());
            std::fs::write(&path, b"invalid json").unwrap();
            assert!(read_index(&path, uid).is_err());
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(INDEX_LIMIT + 1)
                .unwrap();
            assert!(read_index(&path, uid).is_err());
        }
    }
}
