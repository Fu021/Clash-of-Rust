use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub const SUBSCRIPTION_RULES: [&str; 5] = [
    "GEOSITE,category-ads-all,REJECT",
    "GEOSITE,private,DIRECT",
    "GEOIP,Private,DIRECT,no-resolve",
    "GEOSITE,CN,DIRECT",
    "GEOIP,CN,DIRECT,no-resolve",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuleOverrides {
    pub enabled: [bool; 5],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub controller_port: u16,
    pub mixed_port: u16,
    pub secret: String,
    pub dark: bool,
    pub delay_interval_minutes: u32,
    pub active_profile: Option<String>,
    pub run_mode: String,
    pub proxy_mode: crate::engine::ProxyMode,
    pub node_sort: crate::proxy_order::NodeSort,
    pub rule_overrides: RuleOverrides,
}

impl Default for Settings {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct Defaults {
            controller_port: u16,
            mixed_port: u16,
            dark: bool,
        }
        let defaults: Defaults =
            serde_json::from_str(include_str!("../resources/settings-defaults.json"))
                .expect("bundled settings defaults must be valid JSON");
        Self {
            controller_port: defaults.controller_port,
            mixed_port: defaults.mixed_port,
            secret: uuid::Uuid::new_v4().to_string(),
            dark: defaults.dark,
            delay_interval_minutes: 5,
            active_profile: None,
            run_mode: "rule".into(),
            proxy_mode: crate::engine::ProxyMode::Off,
            node_sort: crate::proxy_order::NodeSort::default(),
            rule_overrides: RuleOverrides::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.run_mode.as_str(), "rule" | "global" | "direct") {
            bail!("运行模式无效");
        }
        if self.delay_interval_minutes > 1440 {
            bail!("定时测速间隔应为 0–1440 分钟，0 表示关闭");
        }
        if self.controller_port == 0
            || self.mixed_port == 0
            || self.controller_port == self.mixed_port
        {
            bail!("控制端口与代理端口必须非零且不能相同");
        }
        if self.secret.is_empty() {
            bail!("控制接口密钥不能为空");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source: String,
    pub updated: u64,
    pub usage: Option<String>,
}

impl Profile {
    pub fn is_default(&self) -> bool {
        self.name == "默认直连"
            && Path::new(&self.source)
                .file_name()
                .is_some_and(|name| name == "default.yaml")
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn discover() -> Result<Self> {
        let root = if let Some(path) = std::env::var_os("CLASH_OF_RUST_DATA_DIR") {
            PathBuf::from(path)
        } else {
            ProjectDirs::from("org", "clash-of-rust", "ClashOfRust")
                .context("无法确定应用数据目录")?
                .data_local_dir()
                .to_owned()
        };
        Self::at(root)
    }
    pub fn at(root: PathBuf) -> Result<Self> {
        for name in ["profiles", "runtime"] {
            let directory = root.join(name);
            std::fs::create_dir_all(&directory)
                .with_context(|| format!("无法创建数据目录：{}", directory.display()))?;
        }
        Ok(Self { root })
    }
    pub fn load_settings(&self) -> Result<Settings> {
        let path = self.root.join("settings.json");
        if !path.exists() {
            let settings = Settings::default();
            self.save_settings(&settings)?;
            return Ok(settings);
        }
        let settings: Settings = serde_json::from_slice(&std::fs::read(path)?)?;
        settings.validate()?;
        Ok(settings)
    }
    pub fn instance_lock(&self) -> Result<std::fs::File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("instance.lock"))?;
        file.try_lock()
            .context("已有客户端使用这个数据目录，请关闭另一个实例")?;
        Ok(file)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        atomic_write(
            &self.root.join("settings.json"),
            &serde_json::to_vec_pretty(settings)?,
        )
    }
    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let path = self.root.join("profiles.json");
        if !path.exists() {
            return Ok(vec![]);
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn save_profiles(&self, profiles: &[Profile]) -> Result<()> {
        atomic_write(
            &self.root.join("profiles.json"),
            &serde_json::to_vec_pretty(profiles)?,
        )
    }
    pub fn profile_path(&self, id: &str) -> Result<PathBuf> {
        uuid::Uuid::parse_str(id).context("配置 ID 无效")?;
        Ok(self.root.join("profiles").join(format!("{id}.yaml")))
    }
    pub fn runtime(&self) -> PathBuf {
        self.root.join("runtime")
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("无效文件路径")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("无法创建文件目录：{}", parent.display()))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("无法创建保存临时文件：{}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
        .with_context(|| format!("写入文件失败：{}", path.display()))?;
    file.as_file()
        .sync_all()
        .with_context(|| format!("同步文件失败：{}", path.display()))?;
    persist_file(file, path)
}

/// Keep atomic replacement and the original file on failure. Windows scanners
/// and readers can briefly deny rename/delete access to the destination.
pub(crate) fn persist_file(file: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    // Close our writer before rename, retaining ownership of the temporary path.
    let mut temporary = Some(file.into_temp_path());
    retry_file_operation(|| match temporary.take().unwrap().persist(path) {
        Ok(()) => Ok(()),
        Err(error) => {
            temporary = Some(error.path);
            Err(error.error)
        }
    })
    .with_context(|| format!("保存文件失败：{}", path.display()))
}

/// Transaction recovery must tolerate the same short-lived Windows locks as
/// atomic replacement. Never delete the original file as a rename workaround.
pub(crate) fn remove_file(path: &Path) -> std::io::Result<()> {
    retry_file_operation(|| std::fs::remove_file(path))
}

fn retry_file_operation<T>(
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    #[cfg(windows)]
    for attempt in 0..8 {
        match operation() {
            Err(error) if matches!(error.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
            }
            result => return result,
        }
    }
    operation()
}

pub const PROFILE_LIMIT: usize = 10 * 1024 * 1024;

/// Bounds allocation even if a file grows after its metadata was checked.
pub fn read_profile(path: &Path) -> Result<String> {
    read_config_text(path, PROFILE_LIMIT)
}

pub(crate) fn read_runtime_config(path: &Path) -> Result<String> {
    read_config_text(path, 64 * 1024 * 1024)
}

fn read_config_text(path: &Path, limit: usize) -> Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .with_context(|| format!("配置文件无法读取：{}", path.display()))?;
    if file.metadata()?.len() > limit as u64 {
        bail!("配置超过 {} MiB 限制", limit / 1024 / 1024);
    }
    let mut raw = String::new();
    file.take(limit as u64 + 1).read_to_string(&mut raw)?;
    if raw.len() > limit {
        bail!("配置超过 {} MiB 限制", limit / 1024 / 1024);
    }
    Ok(raw)
}

pub fn parse_profile(raw: &str) -> Result<Mapping> {
    if raw.len() > PROFILE_LIMIT {
        bail!("配置超过 10 MiB 限制");
    }
    let value: Value = serde_yaml::from_str(raw).context("YAML 配置格式错误")?;
    let Value::Mapping(map) = value else {
        bail!("订阅必须是 mihomo YAML 配置，而非节点链接或 Base64 文本");
    };
    if !map.contains_key(Value::from("proxies"))
        && !map.contains_key(Value::from("proxy-providers"))
    {
        bail!("配置中没有 proxies 或 proxy-providers");
    }
    Ok(map)
}

pub fn runtime_config(raw: &str, settings: &Settings) -> Result<String> {
    settings.validate()?;
    let mut map = parse_profile(raw)?;
    apply_rules(&mut map, &settings.rule_overrides)?;
    // The application owns all ingress and controller settings. A subscription cannot
    // expose the API, load a dashboard, or silently enable TUN.
    for key in [
        "external-ui",
        "external-ui-url",
        "external-controller-tls",
        "external-controller-unix",
        "external-controller-pipe",
        "external-controller-cors",
        "listeners",
        "authentication",
    ] {
        map.remove(Value::from(key));
    }
    for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
        map.insert(Value::from(key), Value::from(0));
    }
    map.insert(Value::from("mixed-port"), Value::from(settings.mixed_port));
    map.insert(
        Value::from("external-controller"),
        Value::from(format!("127.0.0.1:{}", settings.controller_port)),
    );
    map.insert(Value::from("secret"), Value::from(settings.secret.clone()));
    map.insert(Value::from("allow-lan"), Value::from(false));
    map.insert(Value::from("bind-address"), Value::from("127.0.0.1"));
    map.insert(Value::from("mode"), Value::from(settings.run_mode.clone()));
    map.insert(Value::from("log-level"), Value::from("info"));
    // Process discovery is not used by this client's connection view.
    map.insert(Value::from("find-process-mode"), Value::from("off"));
    // Geo downloads are explicitly managed by the application. Even the core's
    // missing-file fallback must not attempt Internet downloads during startup.
    map.insert(Value::from("geo-auto-update"), Value::from(false));
    map.insert(Value::from("geox-url"), serde_yaml::from_str("geoip: http://127.0.0.1:1/offline\ngeosite: http://127.0.0.1:1/offline\nmmdb: http://127.0.0.1:1/offline\nasn: http://127.0.0.1:1/offline\n")?);
    let mut tun = match map.remove(Value::from("tun")) {
        Some(Value::Mapping(tun)) => tun,
        _ => Mapping::new(),
    };
    tun.insert(Value::from("enable"), Value::from(false));
    tun.entry(Value::from("stack"))
        .or_insert(Value::from("mixed"));
    tun.insert(Value::from("auto-route"), Value::from(true));
    tun.insert(Value::from("auto-detect-interface"), Value::from(true));
    tun.entry(Value::from("dns-hijack"))
        .or_insert(serde_yaml::to_value(["any:53"])?);
    map.insert(Value::from("tun"), Value::Mapping(tun));
    if !map.contains_key(Value::from("dns")) {
        map.insert(Value::from("dns"), serde_yaml::from_str("enable: true\nenhanced-mode: fake-ip\nnameserver:\n  - https://dns.alidns.com/dns-query\n  - https://cloudflare-dns.com/dns-query\n")?);
    }
    Ok(serde_yaml::to_string(&map)?)
}

fn apply_rules(map: &mut Mapping, overrides: &RuleOverrides) -> Result<()> {
    if !overrides.enabled.iter().any(|enabled| *enabled) {
        return Ok(());
    }
    let previous = match map.remove(Value::from("rules")) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(rules)) => rules,
        _ => bail!("订阅 rules 必须是规则列表"),
    };
    let same_rule = |a: &str, b: &str| {
        let mut a = a.split(',').map(str::trim);
        let mut b = b.split(',').map(str::trim);
        loop {
            match (a.next(), b.next()) {
                (None, None) => break true,
                (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => {}
                _ => break false,
            }
        }
    };
    let mut rules = Vec::with_capacity(previous.len() + SUBSCRIPTION_RULES.len());
    for (index, rule) in SUBSCRIPTION_RULES.iter().enumerate() {
        if overrides.enabled[index]
            && !previous
                .iter()
                .any(|value| value.as_str().is_some_and(|line| same_rule(line, rule)))
        {
            rules.push(Value::from(*rule));
        }
    }
    for rule in previous {
        rule.as_str().context("订阅规则必须是字符串")?;
        rules.push(rule);
    }
    map.insert(Value::from("rules"), Value::Sequence(rules));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn atomic_save_recovers_from_a_temporary_windows_file_lock() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        atomic_write(&path, b"old settings").unwrap();
        // Readers that omit FILE_SHARE_DELETE prevent atomic replacement.
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let destination = path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            sender.send(()).unwrap();
            atomic_write(&destination, b"new settings")
        });
        receiver.recv().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert_eq!(std::fs::read(&path).unwrap(), b"old settings");
        drop(reader);
        writer.join().unwrap().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new settings");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn failed_atomic_save_keeps_the_original_and_identifies_the_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        atomic_write(&path, b"original").unwrap();
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        let error = atomic_write(&path, b"replacement").unwrap_err();
        assert!(format!("{error:#}").contains("settings.json"));
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        drop(reader);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn runtime_disables_process_lookup_even_when_subscription_enables_it() {
        for mode in ["always", "strict", "off"] {
            let raw = format!("proxies: []\nfind-process-mode: {mode}\n");
            let config: Value =
                serde_yaml::from_str(&runtime_config(&raw, &Settings::default()).unwrap()).unwrap();
            assert_eq!(config["find-process-mode"].as_str(), Some("off"));
            assert_eq!(
                parse_profile(&raw).unwrap()[Value::from("find-process-mode")].as_str(),
                Some(mode)
            );
        }
    }

    #[test]
    fn legacy_settings_default_to_rule_and_proxy_off() {
        let settings: Settings = serde_json::from_str(
            r#"{"controller_port":9090,"mixed_port":7897,"secret":"test","dark":true}"#,
        )
        .unwrap();
        settings.validate().unwrap();
        assert_eq!(settings.run_mode, "rule");
        assert_eq!(settings.proxy_mode, crate::engine::ProxyMode::Off);
        assert_eq!(settings.rule_overrides, RuleOverrides::default());
        assert_eq!(Settings::default().rule_overrides.enabled, [false; 5]);
        assert_eq!(settings.rule_overrides.enabled, [false; 5]);
    }

    #[test]
    fn additional_rules_have_priority_without_changing_subscription_fallbacks() {
        let raw = "proxies: []\nproxy-groups: [{name: Proxy, type: select, proxies: [DIRECT]}]\nrules:\n  - MATCH,DIRECT\n  - GEOSITE,cn,DIRECT\n  - DOMAIN,example.test,DIRECT\n  - MATCH,Proxy\n";
        let settings = Settings {
            rule_overrides: RuleOverrides { enabled: [true; 5] },
            ..Settings::default()
        };
        let config: Value = serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
        let rules = config["rules"].as_sequence().unwrap();
        let expected: Vec<_> = SUBSCRIPTION_RULES
            .into_iter()
            .filter(|rule| *rule != "GEOSITE,CN,DIRECT")
            .chain([
                "MATCH,DIRECT",
                "GEOSITE,cn,DIRECT",
                "DOMAIN,example.test,DIRECT",
                "MATCH,Proxy",
            ])
            .map(Value::from)
            .collect();
        assert_eq!(*rules, expected);
        let again = runtime_config(&serde_yaml::to_string(&config).unwrap(), &settings).unwrap();
        assert_eq!(
            serde_yaml::from_str::<Value>(&again).unwrap()["rules"],
            config["rules"]
        );
        let original: Value = serde_yaml::from_str(raw).unwrap();
        let disabled: Value =
            serde_yaml::from_str(&runtime_config(raw, &Settings::default()).unwrap()).unwrap();
        assert_eq!(disabled["rules"], original["rules"]);
    }

    #[test]
    fn subscription_rules_already_present_are_unchanged_by_toggles() {
        for (index, rule) in SUBSCRIPTION_RULES.iter().enumerate() {
            let existing = rule.to_ascii_lowercase().replace(',', " , ");
            let raw = format!(
                "proxies: []\nrules:\n  - DOMAIN,example.test,DIRECT\n  - {existing}\n  - MATCH,DIRECT\n"
            );
            let original: Value = serde_yaml::from_str(&raw).unwrap();
            for enabled in [true, false] {
                let mut settings = Settings::default();
                settings.rule_overrides.enabled[index] = enabled;
                let config: Value =
                    serde_yaml::from_str(&runtime_config(&raw, &settings).unwrap()).unwrap();
                assert_eq!(config["rules"], original["rules"]);
            }
        }
        let raw = format!(
            "proxies: []\nrules:\n  - {}\n  - MATCH,DIRECT\n",
            SUBSCRIPTION_RULES.join("\n  - ")
        );
        let original: Value = serde_yaml::from_str(&raw).unwrap();
        let settings = Settings {
            rule_overrides: RuleOverrides { enabled: [true; 5] },
            ..Settings::default()
        };
        let config: Value =
            serde_yaml::from_str(&runtime_config(&raw, &settings).unwrap()).unwrap();
        assert_eq!(config["rules"], original["rules"]);
        // LAN has a different address range and must not suppress Private.
        let raw = "proxies: []\nrules: [\"GEOIP,LAN,DIRECT,no-resolve\", \"MATCH,DIRECT\"]";
        let mut settings = Settings::default();
        settings.rule_overrides.enabled[2] = true;
        let config: Value = serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
        assert_eq!(
            config["rules"][0].as_str(),
            Some("GEOIP,Private,DIRECT,no-resolve")
        );
        assert_eq!(
            config["rules"][1].as_str(),
            Some("GEOIP,LAN,DIRECT,no-resolve")
        );
        assert_eq!(config["rules"].as_sequence().unwrap().len(), 3);
    }

    #[test]
    fn individual_rules_preserve_existing_fallback_and_reject_invalid_rule_lists() {
        let raw = "proxies: []\nrules:\n  - DOMAIN,example.test,DIRECT\n  - MATCH,DIRECT\n";
        for (index, rule) in SUBSCRIPTION_RULES.iter().enumerate() {
            let mut settings = Settings::default();
            settings.rule_overrides.enabled[index] = true;
            let value: Value =
                serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
            assert_eq!(value["rules"][0].as_str(), Some(*rule));
            assert_eq!(value["rules"][2].as_str(), Some("MATCH,DIRECT"));
        }
        let settings = Settings {
            rule_overrides: RuleOverrides { enabled: [true; 5] },
            ..Settings::default()
        };
        assert!(runtime_config("proxies: []\nrules: invalid", &settings).is_err());
        assert!(runtime_config("proxies: []\nrules: [123]", &settings).is_err());
        let empty: Value =
            serde_yaml::from_str(&runtime_config("proxies: []", &settings).unwrap()).unwrap();
        assert_eq!(empty["rules"].as_sequence().unwrap().len(), 5);
    }

    #[test]
    fn saved_modes_survive_reload_and_control_startup_config() {
        use crate::engine::ProxyMode;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().to_owned()).unwrap();
        for run_mode in ["rule", "global", "direct"] {
            for proxy_mode in [ProxyMode::Off, ProxyMode::System, ProxyMode::Tun] {
                let settings = Settings {
                    run_mode: run_mode.into(),
                    proxy_mode,
                    ..Settings::default()
                };
                store.save_settings(&settings).unwrap();
                let loaded = store.load_settings().unwrap();
                assert_eq!(loaded.run_mode, run_mode);
                assert_eq!(loaded.proxy_mode, proxy_mode);
                let config: Value =
                    serde_yaml::from_str(&runtime_config("proxies: []", &loaded).unwrap()).unwrap();
                assert_eq!(config["mode"].as_str(), Some(run_mode));
                // TUN is only restored after startup health checks and permission handling.
                assert_eq!(config["tun"]["enable"].as_bool(), Some(false));
            }
        }
        let invalid = Settings {
            run_mode: "invalid".into(),
            ..Settings::default()
        };
        assert!(store.save_settings(&invalid).is_err());
        assert_eq!(store.load_settings().unwrap().run_mode, "direct");
    }

    #[test]
    fn subscription_cannot_expose_controller_or_enable_tun() {
        let raw = "proxies: []\nexternal-controller: 0.0.0.0:9090\nexternal-ui: ui\nlisteners: [{name: evil}]\nallow-lan: true\ntun: {enable: true}\n";
        let settings = Settings::default();
        let result: Value = serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
        assert_eq!(
            result["external-controller"].as_str(),
            Some("127.0.0.1:9090")
        );
        assert_eq!(result["tun"]["enable"].as_bool(), Some(false));
        assert_eq!(result["allow-lan"].as_bool(), Some(false));
        assert!(result["listeners"].is_null());
        assert!(result["external-ui"].is_null());
        assert_eq!(result["geo-auto-update"].as_bool(), Some(false));
        assert_eq!(
            result["geox-url"]["geoip"].as_str(),
            Some("http://127.0.0.1:1/offline")
        );
        assert_eq!(result["secret"].as_str(), Some(settings.secret.as_str()));
    }
    #[test]
    fn rejects_node_links_and_invalid_ports() {
        assert!(parse_profile("ss://example").is_err());
        let settings = Settings {
            mixed_port: Settings::default().controller_port,
            ..Settings::default()
        };
        assert!(runtime_config("proxies: []", &settings).is_err());
    }
    #[test]
    fn atomic_storage_and_profile_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().to_owned()).unwrap();
        let mut settings = store.load_settings().unwrap();
        settings.dark = false;
        store.save_settings(&settings).unwrap();
        assert!(!store.load_settings().unwrap().dark);
        assert!(store.profile_path("../../settings").is_err());
    }
}

#[cfg(test)]
mod bounded_profile_tests {
    use super::*;
    #[test]
    fn profile_reader_accepts_boundary_and_rejects_oversized_files_before_reading() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.yaml");
        std::fs::write(&path, vec![b'a'; PROFILE_LIMIT]).unwrap();
        assert_eq!(read_profile(&path).unwrap().len(), PROFILE_LIMIT);
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap();
        assert!(
            read_profile(&path)
                .unwrap_err()
                .to_string()
                .contains("10 MiB")
        );
    }
}
