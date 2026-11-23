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
            "{}",
            args.language().text(
                "--user 与 --all-users 目前仅适用于 Linux",
                "--user and --all-users are only supported on Linux"
            )
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
        anyhow::bail!(
            "{}",
            args.language().text(
                "跨用户订阅总览目前仅适用于 Linux",
                "Listing subscriptions across users is only supported on Linux"
            )
        )
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use anyhow::{Context, bail, ensure};
    use clash_client_core::config::{Language, Profile};
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

    fn account(args: &Args, line: &str) -> Result<Account> {
        let fields: Vec<_> = line.split(':').collect();
        ensure!(
            fields.len() == 7,
            "{}",
            args.language()
                .text("系统用户记录格式无效", "Invalid system user record")
        );
        let home = PathBuf::from(fields[5]);
        ensure!(
            !fields[0].is_empty() && home.is_absolute(),
            "{}",
            args.language().text(
                "系统用户记录缺少名称或主目录",
                "The system user record is missing a name or an absolute home directory"
            )
        );
        Ok(Account {
            name: fields[0].into(),
            uid: fields[2].parse()?,
            home,
        })
    }

    fn accounts(args: &Args, key: Option<&str>) -> Result<Vec<Account>> {
        let mut command = Command::new("/usr/bin/getent");
        command.args(["--", "passwd"]);
        if let Some(key) = key {
            command.arg(key);
        }
        let output = command.output().with_context(|| {
            args.language().text(
                "无法查询本机用户（需要 getent）",
                "Could not look up local users (getent is required)",
            )
        })?;
        accounts_from_output(args, output)
    }

    fn accounts_from_output(args: &Args, output: std::process::Output) -> Result<Vec<Account>> {
        ensure!(
            output.status.success(),
            "{}",
            args.language().text(
                "找不到指定的系统用户",
                "The specified system user was not found"
            )
        );
        String::from_utf8(output.stdout)?
            .lines()
            .map(|line| account(args, line))
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

    fn forward_environment(key: &str, explicit_user: bool) -> bool {
        matches!(
            key,
            "LANG"
                | "LANGUAGE"
                | "LC_ALL"
                | "LC_CTYPE"
                | "LC_NUMERIC"
                | "LC_TIME"
                | "LC_COLLATE"
                | "LC_MONETARY"
                | "LC_MESSAGES"
                | "TERM"
                | "COLORTERM"
                | "TZ"
        ) || (!explicit_user
            && matches!(
                key,
                "CLASH_OF_RUST_DATA_DIR"
                    | "DISPLAY"
                    | "WAYLAND_DISPLAY"
                    | "XAUTHORITY"
                    | "GSETTINGS_BACKEND"
                    | "GSETTINGS_SCHEMA_DIR"
                    | "http_proxy"
                    | "https_proxy"
                    | "all_proxy"
                    | "no_proxy"
                    | "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "ALL_PROXY"
                    | "NO_PROXY"
                    | "SSL_CERT_FILE"
                    | "SSL_CERT_DIR"
                    | "CURL_CA_BUNDLE"
            ))
    }

    fn require_user_scope(args: &Args, root: bool) -> Result<()> {
        ensure!(
            root || !args.flag("all-users"),
            "{}",
            args.language().text(
                "只有 root 可以使用 --all-users；跨用户 TUN 检查请运行 sudo cor status --all-users",
                "Only root can use --all-users; to inspect TUN across users, run sudo cor status --all-users"
            )
        );
        ensure!(
            root || args.get("user").is_none(),
            "{}",
            args.language().text(
                "只有 root 可以使用 --user；普通用户只能管理自己的订阅",
                "Only root can use --user; other users can only manage their own subscriptions"
            )
        );
        Ok(())
    }

    pub(super) fn prepare(args: &Args) -> Result<()> {
        let root = platform::is_elevated();
        require_user_scope(args, root)?;
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
        let target = accounts(args, Some(&selected))?
            .into_iter()
            .next()
            .with_context(|| {
                args.language()
                    .text("找不到目标用户", "The target user was not found")
            })?;
        if target.uid == 0 {
            return Ok(());
        }
        // Never load user YAML in this root process. Only the fixed helper may
        // run elevated; the command and its backend run as the selected account.
        if matches!(args.path.as_str(), "start" | "restart" | "run")
            && !args.flag("no-core")
            && !args.flag("network-only")
            || (args.path == "proxy" && args.positional.first().is_some_and(|v| v == "tun"))
            || (args.path == "claude protect" && args.positional.first().is_some_and(|v| v == "on"))
        {
            platform::authorize_tun_for_user()?;
        }
        let mut command = Command::new("/usr/sbin/runuser");
        command
            .args(["--user", &target.name, "--"])
            .arg(executable)
            .args(delegated_arguments(args))
            .env_clear()
            .env(
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            );
        let explicit_user = args.get("user").is_some();
        for (key, value) in std::env::vars_os() {
            if key
                .to_str()
                .is_some_and(|key| forward_environment(key, explicit_user))
            {
                command.env(key, value);
            }
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
            if !explicit_user && owned {
                command.env(key, std::env::var_os(key).expect("owned environment path"));
            }
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
        Err(command.exec()).with_context(|| {
            args.language().text(
                "无法切换到目标用户（需要 util-linux 的 runuser）",
                "Could not switch to the target user (runuser from util-linux is required)",
            )
        })
    }

    fn read_index(path: &Path, uid: u32, language: Language) -> Result<Option<Value>> {
        let file: File = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => bail!(
                "{}{}",
                language.text(
                    "无法安全读取用户订阅索引：",
                    "Could not safely read the user's subscription index: "
                ),
                path.display()
            ),
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.uid() == uid && metadata.len() <= INDEX_LIMIT,
            "{}{}",
            language.text(
                "用户订阅索引的所有者、类型或大小无效：",
                "Invalid owner, file type, or size for the user's subscription index: "
            ),
            path.display()
        );
        let mut bytes = Vec::new();
        file.take(INDEX_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= INDEX_LIMIT,
            "{}",
            language.text(
                "用户订阅索引过大",
                "The user's subscription index is too large"
            )
        );
        serde_json::from_slice(&bytes).map(Some).map_err(|_| {
            anyhow::anyhow!(
                "{}{}",
                language.text(
                    "用户订阅索引格式无效：",
                    "Invalid format for the user's subscription index: "
                ),
                path.display()
            )
        })
    }

    pub(super) fn profiles(args: &Args) -> Result<Value> {
        ensure!(
            platform::is_elevated(),
            "{}",
            args.language().text(
                "只有 root 可以查看跨用户订阅",
                "Only root can list subscriptions across users"
            )
        );
        profiles_for_accounts(args, accounts(args, None)?)
    }

    fn profiles_for_accounts(args: &Args, accounts: Vec<Account>) -> Result<Value> {
        let language = args.language();
        let filter = args.get("filter").unwrap_or("").to_lowercase();
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        for account in accounts {
            if !seen.insert(account.uid) {
                continue;
            }
            let root = account.home.join(".local/share/clashofrust");
            let profiles_path = root.join("profiles.json");
            let Some(index) = read_index(&profiles_path, account.uid, language)? else {
                continue;
            };
            let profiles: Vec<Profile> = serde_json::from_value(index).map_err(|_| {
                let name = &account.name;
                anyhow::anyhow!(match language {
                    Language::SimplifiedChinese => format!("用户 {name} 的订阅索引格式无效"),
                    Language::English =>
                        format!("Invalid subscription index format for user {name}"),
                })
            })?;
            let settings =
                read_index(&root.join("settings.json"), account.uid, language)?.unwrap_or_default();
            for profile in profiles {
                if !crate::output::profile_list_matches(args, &profile, &filter)
                    && !account.name.to_lowercase().contains(&filter)
                {
                    continue;
                }
                let mut row = json!({
                    "id": profile.id,
                    "name": crate::output::profile_list_name(args, &profile),
                    "owner": account.name,
                    "uid": account.uid,
                    "data_dir": root,
                    "active": settings["active_profile"] == profile.id,
                    "source": profile.source,
                    "updated": profile.updated,
                    "usage": profile.usage,
                });
                if !crate::output::writes_json(args) && !profile.is_default() && !profile.is_local()
                {
                    row["show_usage"] = Value::Bool(true);
                }
                rows.push(row);
            }
        }
        Ok(json!(rows))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn delegation_only_forwards_known_environment_and_never_admin_credentials() {
            for explicit in [false, true] {
                for key in [
                    "COR_MATRIX_ADMIN_TOKEN",
                    "GH_TOKEN",
                    "AWS_SECRET_ACCESS_KEY",
                    "LD_PRELOAD",
                    "SUDO_UID",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "XDG_DATA_HOME",
                ] {
                    assert!(!forward_environment(key, explicit), "{key}");
                }
                assert!(forward_environment("LANG", explicit));
                assert!(forward_environment("TERM", explicit));
            }
            for key in ["HTTPS_PROXY", "SSL_CERT_FILE", "CLASH_OF_RUST_DATA_DIR"] {
                assert!(forward_environment(key, false));
                assert!(!forward_environment(key, true));
            }
        }
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
            assert!(account(&args, "bad").is_err());
            assert!(account(&args, "alice:x:1000:1000::relative:/bin/sh").is_err());
            assert_eq!(
                account(&args, "alice:x:1000:1000::/home/alice:/bin/sh")
                    .unwrap()
                    .uid,
                1000
            );
        }

        #[test]
        fn user_scope_checks_and_missing_accounts_follow_the_selected_language() {
            use std::os::unix::process::ExitStatusExt;

            for (language, all_users, user, missing) in [
                (
                    "en",
                    "Only root can use --all-users; to inspect TUN across users, run sudo cor status --all-users",
                    "Only root can use --user; other users can only manage their own subscriptions",
                    "The specified system user was not found",
                ),
                (
                    "zh-CN",
                    "只有 root 可以使用 --all-users；跨用户 TUN 检查请运行 sudo cor status --all-users",
                    "只有 root 可以使用 --user；普通用户只能管理自己的订阅",
                    "找不到指定的系统用户",
                ),
            ] {
                for (option, expected) in [("--all-users", all_users), ("--user=fixture", user)] {
                    let args = crate::args::parse(
                        ["status", "--lang", language, option].map(str::to_owned),
                    )
                    .unwrap();
                    assert_eq!(
                        require_user_scope(&args, false).unwrap_err().to_string(),
                        expected
                    );
                    assert!(require_user_scope(&args, true).is_ok());
                    if args.get("user").is_some() {
                        let output = std::process::Output {
                            status: std::process::ExitStatus::from_raw(2 << 8),
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                        };
                        assert_eq!(
                            accounts_from_output(&args, output).unwrap_err().to_string(),
                            missing
                        );
                    }
                }
            }
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
        fn cross_user_profile_list_preserves_raw_export_names_and_filters() {
            let home = tempfile::tempdir().unwrap();
            let data = home.path().join(".local/share/clashofrust");
            std::fs::create_dir_all(&data).unwrap();
            let profiles = json!([
                {"id":"builtin","name":"默认直连","source":"default.yaml","updated":0,"builtin":true},
                {"id":"imported","name":"默认直连","source":"/imports/default.yaml","updated":0,"builtin":false},
                {"id":"remote","name":"远程线路","source":"http://example.test/","updated":0,"builtin":false}
            ]);
            std::fs::write(data.join("profiles.json"), profiles.to_string()).unwrap();
            let args = |options: &[&str]| {
                crate::args::parse(
                    ["profile", "list", "--all-users", "--lang=en", "--quiet"]
                        .into_iter()
                        .chain(options.iter().copied())
                        .map(str::to_owned),
                )
                .unwrap()
            };
            let list = |args: &Args| {
                profiles_for_accounts(
                    args,
                    vec![Account {
                        name: "fixture".into(),
                        uid: data.metadata().unwrap().uid(),
                        home: home.path().to_owned(),
                    }],
                )
                .unwrap()
            };
            let human = list(&args(&["--filter=Default direct"]));
            assert_eq!(human.as_array().unwrap().len(), 1);
            assert_eq!(human[0]["id"], "builtin");
            assert_eq!(human[0]["name"], "Default direct");

            let human = list(&args(&[]));
            let machine = list(&args(&["--json"]));
            for (index, id) in ["builtin", "imported", "remote"].into_iter().enumerate() {
                assert_eq!(human[index]["id"], id);
                assert_eq!(human[index]["show_usage"] == true, id == "remote");
                assert_eq!(machine[index]["id"], id);
                assert!(machine[index].get("show_usage").is_none());
            }

            let output = home.path().join("export.json");
            let path = output.to_str().unwrap();
            let raw = args(&["--output", path, "--filter=默认直连"]);
            crate::output::emit(&raw, list(&raw)).unwrap();
            let saved: Value = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
            assert_eq!(saved["command"], "profile list");
            assert_eq!(saved["data"].as_array().unwrap().len(), 2);
            for (entry, id) in saved["data"]
                .as_array()
                .unwrap()
                .iter()
                .zip(["builtin", "imported"])
            {
                assert_eq!(entry["id"], id);
                assert_eq!(entry["name"], "默认直连");
                assert_eq!(entry["owner"], "fixture");
            }
            let alias = args(&["--output", path, "--filter=Default direct"]);
            crate::output::emit(&alias, list(&alias)).unwrap();
            let saved: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
            assert_eq!(saved["data"], json!([]));
        }

        #[test]
        fn inventory_never_follows_symlinks_or_creates_missing_data() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("profiles.json");
            let uid = dir.path().metadata().unwrap().uid();
            let language = Language::SimplifiedChinese;
            assert!(read_index(&path, uid, language).unwrap().is_none());
            assert!(!path.exists());
            std::fs::write(&path, b"[]").unwrap();
            assert_eq!(read_index(&path, uid, language).unwrap(), Some(json!([])));
            assert!(read_index(&path, uid.wrapping_add(1), language).is_err());
            let link = dir.path().join("link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read_index(&link, uid, language).is_err());
            assert!(read_index(dir.path(), uid, language).is_err());
            std::fs::write(&path, b"invalid json").unwrap();
            assert!(read_index(&path, uid, language).is_err());
            File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(INDEX_LIMIT + 1)
                .unwrap();
            assert!(read_index(&path, uid, language).is_err());
        }

        #[test]
        fn cross_user_index_errors_use_the_cli_language_and_preserve_paths() {
            let home = tempfile::tempdir().unwrap();
            let data = home.path().join(".local/share/clashofrust");
            std::fs::create_dir_all(&data).unwrap();
            let uid = data.metadata().unwrap().uid();
            let profiles = data.join("profiles.json");
            let settings = data.join("settings.json");
            let link = data.join("链接.json");
            std::os::unix::fs::symlink(&profiles, &link).unwrap();
            std::fs::write(&settings, "invalid json").unwrap();

            for (locale, unsafe_read, invalid_metadata, invalid_settings, invalid_profiles) in [
                (
                    "zh-CN",
                    "无法安全读取用户订阅索引：",
                    "用户订阅索引的所有者、类型或大小无效：",
                    "用户订阅索引格式无效：",
                    "用户 测试用户 的订阅索引格式无效",
                ),
                (
                    "en",
                    "Could not safely read the user's subscription index: ",
                    "Invalid owner, file type, or size for the user's subscription index: ",
                    "Invalid format for the user's subscription index: ",
                    "Invalid subscription index format for user 测试用户",
                ),
            ] {
                let args = crate::args::parse(
                    ["profile", "list", "--all-users", "--lang", locale].map(str::to_owned),
                )
                .unwrap();
                let list = || {
                    profiles_for_accounts(
                        &args,
                        vec![Account {
                            name: "测试用户".into(),
                            uid,
                            home: home.path().to_owned(),
                        }],
                    )
                    .unwrap_err()
                    .to_string()
                };

                std::fs::write(&profiles, "{}").unwrap();
                assert_eq!(list(), invalid_profiles);
                assert_eq!(
                    read_index(&link, uid, args.language())
                        .unwrap_err()
                        .to_string(),
                    format!("{unsafe_read}{}", link.display())
                );
                assert_eq!(
                    read_index(&profiles, uid.wrapping_add(1), args.language())
                        .unwrap_err()
                        .to_string(),
                    format!("{invalid_metadata}{}", profiles.display())
                );
                std::fs::write(&profiles, "[]").unwrap();
                assert_eq!(list(), format!("{invalid_settings}{}", settings.display()));
            }
        }
    }
}
