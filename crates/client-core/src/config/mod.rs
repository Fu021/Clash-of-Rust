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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccentColor {
    #[default]
    Teal,
    Blue,
    Purple,
    Rose,
    Amber,
    Slate,
}
impl AccentColor {
    pub const ALL: [Self; 6] = [
        Self::Teal,
        Self::Blue,
        Self::Purple,
        Self::Rose,
        Self::Amber,
        Self::Slate,
    ];
}
impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::System => "跟随系统",
            Self::Light => "浅色",
            Self::Dark => "深色",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub controller_port: u16,
    pub mixed_port: u16,
    pub secret: String,
    pub dark: bool,
    // None preserves the explicit theme of settings saved by older versions.
    pub theme_mode: Option<ThemeMode>,
    pub accent_color: AccentColor,
    pub text_scale: u16,
    pub tray_speed: bool,
    pub dns_override: bool,
    pub ipv6: bool,
    pub udp: bool,
    pub claude_protection: Option<std::sync::Arc<crate::claude_check::protection::Protection>>,
    pub ip_reputation_key: Option<String>,
    #[serde(default)]
    pub wsl: crate::wsl::WslSettings,
    pub delay_interval_minutes: u32,
    pub active_profile: Option<String>,
    pub run_mode: String,
    pub proxy_mode: crate::model::ProxyMode,
    pub node_sort: crate::proxy_order::NodeSort,
    pub rule_overrides: RuleOverrides,
    pub rule_profiles:
        std::collections::BTreeMap<String, std::sync::Arc<crate::rule_manager::ProfileRules>>,
}

impl Default for Settings {
    fn default() -> Self {
        #[derive(Deserialize)]
        struct Defaults {
            controller_port: u16,
            mixed_port: u16,
            dark: bool,
        }
        let defaults: Defaults = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../resources/settings-defaults.json"
        )))
        .expect("bundled settings defaults must be valid JSON");
        Self {
            controller_port: defaults.controller_port,
            mixed_port: defaults.mixed_port,
            secret: uuid::Uuid::new_v4().to_string(),
            dark: defaults.dark,
            theme_mode: None,
            accent_color: AccentColor::default(),
            text_scale: 100,
            tray_speed: false,
            dns_override: true,
            ipv6: true,
            udp: true,
            claude_protection: None,
            ip_reputation_key: None,
            wsl: crate::wsl::WslSettings::default(),
            delay_interval_minutes: 5,
            active_profile: None,
            run_mode: "rule".into(),
            proxy_mode: crate::model::ProxyMode::Off,
            node_sort: crate::proxy_order::NodeSort::default(),
            rule_overrides: RuleOverrides::default(),
            rule_profiles: Default::default(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        self.wsl.validate()?;
        if !(80..=150).contains(&self.text_scale) {
            bail!("字号比例应为 80–150%");
        }
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
        if self
            .ip_reputation_key
            .as_ref()
            .is_some_and(|key| key.len() > 512 || key.chars().any(char::is_control))
        {
            bail!("IP 信誉服务密钥格式无效");
        }
        for (id, rules) in &self.rule_profiles {
            uuid::Uuid::parse_str(id).context("规则所属的订阅 ID 无效")?;
            rules.validate()?;
        }
        if let Some(protection) = &self.claude_protection {
            protection.validate()?;
            if self.active_profile.as_deref() != Some(&protection.profile_id) {
                bail!("Claude 保护的订阅与当前订阅不一致");
            }
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
pub fn persist_file(file: tempfile::NamedTempFile, path: &Path) -> Result<()> {
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

pub fn read_runtime_config(path: &Path) -> Result<String> {
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
    runtime_config_with_rules(raw, settings, None, false)
}

pub fn runtime_config_with_rules(
    raw: &str,
    settings: &Settings,
    rules: Option<&crate::rule_manager::ProfileRules>,
    builtin: bool,
) -> Result<String> {
    settings.validate()?;
    let mut map = parse_profile(raw)?;
    if let Some(rules) = rules {
        crate::rule_manager::apply(&mut map, rules, builtin)?;
    } else if !builtin {
        apply_rules(&mut map, &settings.rule_overrides)?;
    }
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
    map.insert(Value::from("tcp-concurrent"), Value::from(true));
    map.insert(Value::from("unified-delay"), Value::from(true));
    map.insert(Value::from("keep-alive-interval"), Value::from(30));
    map.insert(Value::from("ipv6"), Value::from(settings.ipv6));
    map.insert(
        Value::from("geodata-loader"),
        Value::from("memconservative"),
    );
    // Geo downloads are explicitly managed by the application. Even the core's
    // missing-file fallback must not attempt Internet downloads during startup.
    map.insert(Value::from("geo-auto-update"), Value::from(false));
    map.insert(Value::from("geox-url"), serde_yaml::from_str("geoip: http://127.0.0.1:1/offline\ngeosite: http://127.0.0.1:1/offline\nmmdb: http://127.0.0.1:1/offline\nasn: http://127.0.0.1:1/offline\n")?);
    let mut tun = match map.remove(Value::from("tun")) {
        Some(Value::Mapping(tun)) => tun,
        _ => Mapping::new(),
    };
    tun.insert(Value::from("enable"), Value::from(false));
    tun.insert(Value::from("stack"), Value::from("mips"));
    tun.insert(Value::from("auto-route"), Value::from(true));
    tun.insert(Value::from("auto-detect-interface"), Value::from(true));
    tun.entry(Value::from("dns-hijack"))
        .or_insert(serde_yaml::to_value(["any:53"])?);
    map.insert(Value::from("tun"), Value::Mapping(tun));
    apply_sniffer(&mut map)?;
    apply_dns(&mut map, settings)?;
    crate::claude_check::protection::apply(&mut map, settings)?;
    apply_udp(
        &mut map,
        settings.udp || settings.claude_protection.is_some(),
    )?;
    Ok(serde_yaml::to_string(&map)?)
}

/// Learn unknown HTTP/SNI/QUIC hostnames without rewriting established destinations.
/// Keep subscription port/skip choices; protection applies its stricter policy later.
fn apply_sniffer(map: &mut Mapping) -> Result<()> {
    // Mutate in place: Mapping::remove uses swap removal. A new root key
    // between tun and dns otherwise changes their order on every regeneration.
    let sniffer = map
        .entry(Value::from("sniffer"))
        .or_insert(Value::Mapping(Mapping::new()));
    if sniffer.is_null() {
        *sniffer = Value::Mapping(Mapping::new());
    }
    let sniffer = sniffer
        .as_mapping_mut()
        .context("订阅 sniffer 必须是配置对象")?;
    for key in ["enable", "parse-pure-ip"] {
        sniffer.insert(Value::from(key), Value::from(true));
    }
    // A sniffed ECH outer name also changes rule matching in mihomo, even
    // with override-destination=false. Existing DNS hostnames stay authoritative.
    sniffer.insert(Value::from("force-dns-mapping"), Value::from(false));
    sniffer.insert(Value::from("force-domain"), Value::Sequence(vec![]));
    sniffer.insert(Value::from("override-destination"), Value::from(false));
    let protocols = sniffer
        .entry(Value::from("sniff"))
        .or_insert(Value::Mapping(Mapping::new()));
    if protocols.is_null() {
        *protocols = Value::Mapping(Mapping::new());
    }
    let protocols = protocols
        .as_mapping_mut()
        .context("订阅 sniff 必须是配置对象")?;
    for (protocol, ports) in [
        ("HTTP", "[80, 8080-8880]"),
        ("TLS", "[443, 8443]"),
        ("QUIC", "[443, 8443]"),
    ] {
        let options = protocols
            .entry(Value::from(protocol))
            .or_insert(Value::Mapping(Mapping::new()));
        if options.is_null() {
            *options = Value::Mapping(Mapping::new());
        }
        let options = options
            .as_mapping_mut()
            .context("订阅嗅探协议必须是配置对象")?;
        options
            .entry(Value::from("ports"))
            .or_insert(serde_yaml::from_str(ports)?);
        options.insert(Value::from("override-destination"), Value::from(false));
    }
    Ok(())
}

fn apply_udp(map: &mut Mapping, enabled: bool) -> Result<()> {
    if let Some(proxies) = map
        .get_mut(Value::from("proxies"))
        .and_then(Value::as_sequence_mut)
    {
        for proxy in proxies.iter_mut().filter_map(Value::as_mapping_mut) {
            proxy.insert(Value::from("udp"), Value::from(enabled));
        }
    }
    if let Some(groups) = map
        .get_mut(Value::from("proxy-groups"))
        .and_then(Value::as_sequence_mut)
    {
        for group in groups.iter_mut().filter_map(Value::as_mapping_mut) {
            group.insert(Value::from("disable-udp"), Value::from(!enabled));
        }
    }
    if let Some(providers) = map
        .get_mut(Value::from("proxy-providers"))
        .and_then(Value::as_mapping_mut)
    {
        for provider in providers.values_mut().filter_map(Value::as_mapping_mut) {
            let overrides = provider
                .entry(Value::from("override"))
                .or_insert(Value::Mapping(Mapping::new()));
            if overrides.is_null() {
                *overrides = Value::Mapping(Mapping::new());
            }
            let overrides = overrides
                .as_mapping_mut()
                .context("代理集合 override 必须是配置对象")?;
            overrides.insert(Value::from("udp"), Value::from(enabled));
            // Expressions run after fixed provider overrides. Keep the user's
            // transforms, but make the client preference authoritative even
            // after a provider refresh or a conflicting '.udp = false'.
            if let Some(expressions) = overrides.get_mut(Value::from("override-expr"))
                && !expressions.is_null()
            {
                let expressions = expressions
                    .as_sequence_mut()
                    .context("代理集合 override-expr 必须是列表")?;
                let expression = Value::from(format!(".udp = {enabled}"));
                if expressions.last() != Some(&expression) {
                    expressions.push(expression);
                }
            }
        }
    }
    Ok(())
}

/// Shared fallback used when a host needs DNS but the subscription provides none.
pub const DEFAULT_DNS_NAMESERVERS: [&str; 2] = [
    "https://doh.pub/dns-query",
    "https://dns.alidns.com/dns-query",
];

fn apply_dns(map: &mut Mapping, settings: &Settings) -> Result<()> {
    let mut dns = match map.remove(Value::from("dns")) {
        None | Some(Value::Null) => Mapping::new(),
        Some(Value::Mapping(dns)) => dns,
        _ => bail!("订阅 dns 必须是配置对象"),
    };
    // FlClash's baseline owns these three fields, leaving provider-specific
    // resolution policies, filters, hosts and listener settings intact.
    if settings.dns_override || dns.is_empty() {
        dns.insert(Value::from("enable"), Value::from(true));
        dns.insert(Value::from("enhanced-mode"), Value::from("fake-ip"));
        dns.insert(
            Value::from("nameserver"),
            serde_yaml::to_value(DEFAULT_DNS_NAMESERVERS)?,
        );
    }
    // The IPv6 preference is independent of DNS override and authoritative
    // for both the core's outbound support and its DNS AAAA responses.
    dns.insert(Value::from("ipv6"), Value::from(settings.ipv6));
    map.insert(Value::from("dns"), Value::Mapping(dns));
    Ok(())
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
    #[test]
    fn data_directory_is_exclusive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = super::Store::at(tmp.path().to_owned()).unwrap();
        let first = store.instance_lock().unwrap();
        assert!(store.instance_lock().is_err());
        drop(first);
        assert!(store.instance_lock().is_ok());
    }

    #[test]
    fn old_theme_is_preserved_and_new_appearance_options_have_small_defaults() {
        let old: super::Settings = serde_json::from_str(r#"{"dark":false}"#).unwrap();
        assert!(!old.dark);
        assert!(old.theme_mode.is_none());
        assert_eq!(old.accent_color, AccentColor::Teal);
        assert_eq!(old.text_scale, 100);
        assert!(!old.tray_speed);
        for scale in [0, 79, 151, u16::MAX] {
            assert!(
                super::Settings {
                    text_scale: scale,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn accent_selection_is_saved_and_restored_without_changing_network_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::at(directory.path().to_owned()).unwrap();
        let mut settings = store.load_settings().unwrap();
        let initial_port = settings.mixed_port;
        for color in AccentColor::ALL {
            settings.accent_color = color;
            store.save_settings(&settings).unwrap();
            let loaded = store.load_settings().unwrap();
            assert_eq!(loaded.accent_color, color);
            assert_eq!(loaded.mixed_port, initial_port);
        }
    }
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
        assert_eq!(settings.proxy_mode, crate::model::ProxyMode::Off);
        assert!(settings.dns_override);
        assert!(settings.ipv6);
        assert!(settings.udp);
        assert_eq!(settings.rule_overrides, RuleOverrides::default());
        assert_eq!(Settings::default().rule_overrides.enabled, [false; 5]);
        assert_eq!(settings.rule_overrides.enabled, [false; 5]);
    }

    #[test]
    fn client_overrides_subscription_transport_defaults_without_modifying_the_source() {
        let raw = "proxies: []\ntcp-concurrent: false\nunified-delay: false\nkeep-alive-interval: 5\nipv6: false\nlog-level: debug\ngeodata-loader: standard\ntun: {stack: gvisor, auto-detect-interface: false, mtu: 1400}\n";
        let result: Value =
            serde_yaml::from_str(&runtime_config(raw, &Settings::default()).unwrap()).unwrap();
        assert_eq!(result["tcp-concurrent"].as_bool(), Some(true));
        assert_eq!(result["unified-delay"].as_bool(), Some(true));
        assert_eq!(result["keep-alive-interval"].as_u64(), Some(30));
        assert_eq!(result["geodata-loader"].as_str(), Some("memconservative"));
        assert_eq!(result["log-level"].as_str(), Some("info"));
        assert_eq!(result["tun"]["stack"].as_str(), Some("mips"));
        assert_eq!(result["tun"]["auto-detect-interface"].as_bool(), Some(true));
        assert_eq!(result["tun"]["mtu"].as_u64(), Some(1400));
        assert_eq!(
            parse_profile(raw).unwrap()[Value::from("tcp-concurrent")].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn sniffer_observes_unknown_hosts_without_rewriting_destinations_or_source() {
        let raw = "proxies: []\nsniffer: {enable: false, parse-pure-ip: false, force-dns-mapping: true, force-domain: ['+.*'], override-destination: true, skip-domain: ['+.example.test'], sniff: {TLS: {ports: [9443], override-destination: true}}}\n";
        let runtime: Value =
            serde_yaml::from_str(&runtime_config(raw, &Settings::default()).unwrap()).unwrap();
        assert_eq!(runtime["sniffer"]["enable"], true);
        assert_eq!(runtime["sniffer"]["parse-pure-ip"], true);
        assert_eq!(runtime["sniffer"]["force-dns-mapping"], false);
        assert!(
            runtime["sniffer"]["force-domain"]
                .as_sequence()
                .unwrap()
                .is_empty()
        );
        assert_eq!(runtime["sniffer"]["override-destination"], false);
        assert_eq!(
            runtime["sniffer"]["sniff"]["TLS"]["override-destination"],
            false
        );
        assert_eq!(runtime["sniffer"]["sniff"]["TLS"]["ports"][0], 9443);
        assert_eq!(runtime["sniffer"]["skip-domain"][0], "+.example.test");
        assert_eq!(
            parse_profile(raw).unwrap()[Value::from("sniffer")]["enable"],
            false
        );
    }

    #[test]
    fn udp_preference_covers_static_provider_and_group_settings_and_protection_forces_it_on() {
        let raw = "proxies: [{name: node, type: anytls, server: 127.0.0.1, port: 443, password: fixture, udp: false, tfo: true, mptcp: false}]\nproxy-groups: [{name: Main, type: select, proxies: [node], disable-udp: true}]\nproxy-providers:\n  Remote:\n    type: http\n    url: https://example.org/nodes.yaml\n    override: {udp: false, tfo: true, udp-over-tcp: false, override-expr: ['.udp = false', '.name |= upcase']}\n  Local: {type: file, path: nodes.yaml, override: null}\n  Inline: {type: inline, payload: [{name: inline, type: socks5, server: 127.0.0.1, port: 1080}]}\nrules: ['MATCH,Main']\n";
        for enabled in [false, true] {
            let mut settings = Settings {
                udp: enabled,
                ..Default::default()
            };
            let plain = runtime_config(raw, &settings).unwrap();
            assert_eq!(
                runtime_config(&plain, &settings).unwrap(),
                plain,
                "runtime overlay must be idempotent"
            );
            for protected in [false, true] {
                if protected {
                    let id = uuid::Uuid::new_v4().to_string();
                    settings.active_profile = Some(id.clone());
                    settings.claude_protection = Some(std::sync::Arc::new(
                        crate::claude_check::protection::Protection::new(
                            id,
                            "node".into(),
                            crate::claude_check::protection::Previous {
                                mode: "rule".into(),
                                proxy_mode: crate::model::ProxyMode::Off,
                                selectors: vec![],
                            },
                        ),
                    ));
                }
                let runtime: Value =
                    serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
                let effective = enabled || protected;
                assert_eq!(runtime["proxies"][0]["udp"], effective);
                assert_eq!(runtime["proxies"][0]["tfo"], true);
                assert_eq!(runtime["proxies"][0]["mptcp"], false);
                for group in runtime["proxy-groups"].as_sequence().unwrap() {
                    assert_eq!(group["disable-udp"], !effective);
                }
                for name in ["Remote", "Local", "Inline"] {
                    assert_eq!(
                        runtime["proxy-providers"][name]["override"]["udp"],
                        effective
                    );
                }
                let overrides = &runtime["proxy-providers"]["Remote"]["override"];
                assert_eq!(overrides["tfo"], true);
                assert_eq!(
                    overrides["udp-over-tcp"], false,
                    "UDP forwarding must not force a protocol-specific UoT setting"
                );
                let expressions = overrides["override-expr"].as_sequence().unwrap();
                assert_eq!(expressions[1], ".name |= upcase");
                assert_eq!(
                    expressions.last().unwrap(),
                    &Value::from(format!(".udp = {effective}"))
                );
                assert_eq!(
                    settings.udp, enabled,
                    "protection must preserve the original preference"
                );
            }
        }
    }

    #[test]
    fn dns_and_ipv6_preferences_are_independent_and_preserve_provider_policies() {
        let raw = "proxies: []\nipv6: false\ndns:\n  enable: false\n  enhanced-mode: redir-host\n  ipv6: false\n  nameserver: [1.1.1.1]\n  default-nameserver: [223.5.5.5]\n  nameserver-policy: {example.com: 119.29.29.29}\n  proxy-server-nameserver: [223.5.5.5]\n  direct-nameserver: [119.29.29.29]\n  fake-ip-filter: ['*.lan', localhost]\n  fake-ip-range: 198.19.0.1/16\n  cache-algorithm: arc\n  cache-max-size: 2048\n  listen: 127.0.0.1:1053\n";
        let original = Value::Mapping(parse_profile(raw).unwrap());
        for dns_override in [false, true] {
            for ipv6 in [false, true] {
                let settings = Settings {
                    dns_override,
                    ipv6,
                    ..Settings::default()
                };
                let result: Value =
                    serde_yaml::from_str(&runtime_config(raw, &settings).unwrap()).unwrap();
                assert_eq!(result["ipv6"].as_bool(), Some(ipv6));
                let mut expected = original["dns"].clone();
                expected["ipv6"] = Value::from(ipv6);
                if dns_override {
                    expected["enable"] = Value::from(true);
                    expected["enhanced-mode"] = Value::from("fake-ip");
                    expected["nameserver"] = serde_yaml::to_value([
                        "https://doh.pub/dns-query",
                        "https://dns.alidns.com/dns-query",
                    ])
                    .unwrap();
                }
                assert_eq!(result["dns"], expected);
                // Applying the same settings again must not grow or erase DNS fields.
                let reapplied: Value = serde_yaml::from_str(
                    &runtime_config(&serde_yaml::to_string(&result).unwrap(), &settings).unwrap(),
                )
                .unwrap();
                assert_eq!(reapplied, result);
            }
        }
    }

    #[test]
    fn dns_fallback_and_saved_preferences_survive_relaunch() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::at(tmp.path().to_owned()).unwrap();
        let settings = Settings {
            dns_override: false,
            ipv6: false,
            udp: false,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings().unwrap();
        assert!(!loaded.dns_override);
        assert!(!loaded.ipv6);
        assert!(!loaded.udp);
        for raw in [
            "proxies: []",
            "proxies: []\ndns: null",
            "proxies: []\ndns: {}",
        ] {
            let result: Value =
                serde_yaml::from_str(&runtime_config(raw, &loaded).unwrap()).unwrap();
            assert_eq!(result["dns"]["enable"].as_bool(), Some(true));
            assert_eq!(result["dns"]["ipv6"].as_bool(), Some(false));
            assert_eq!(result["ipv6"].as_bool(), Some(false));
            assert_eq!(result["dns"]["nameserver"].as_sequence().unwrap().len(), 2);
        }
        for raw in ["proxies: []\ndns: invalid", "proxies: []\ndns: []"] {
            assert!(runtime_config(raw, &loaded).is_err());
        }
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
        use crate::model::ProxyMode;
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
