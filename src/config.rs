use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

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
        std::fs::create_dir_all(root.join("profiles"))?;
        std::fs::create_dir_all(root.join("runtime"))?;
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
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|e| e.error)
        .context("保存文件失败")?;
    Ok(())
}

pub fn parse_profile(raw: &str) -> Result<Mapping> {
    if raw.len() > 10 * 1024 * 1024 {
        bail!("配置超过 10 MiB 限制");
    }
    let value: Value = serde_yaml::from_str(raw).context("YAML 配置格式错误")?;
    let map = value
        .as_mapping()
        .context("订阅必须是 mihomo YAML 配置，而非节点链接或 Base64 文本")?;
    if !map.contains_key(Value::from("proxies"))
        && !map.contains_key(Value::from("proxy-providers"))
    {
        bail!("配置中没有 proxies 或 proxy-providers");
    }
    Ok(map.clone())
}

pub fn runtime_config(raw: &str, settings: &Settings) -> Result<String> {
    settings.validate()?;
    let mut map = parse_profile(raw)?;
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
    // Geo downloads are explicitly managed by the application. Even the core's
    // missing-file fallback must not attempt Internet downloads during startup.
    map.insert(Value::from("geo-auto-update"), Value::from(false));
    map.insert(Value::from("geox-url"), serde_yaml::from_str("geoip: http://127.0.0.1:1/offline\ngeosite: http://127.0.0.1:1/offline\nmmdb: http://127.0.0.1:1/offline\nasn: http://127.0.0.1:1/offline\n")?);
    let mut tun = map
        .get(Value::from("tun"))
        .and_then(Value::as_mapping)
        .cloned()
        .unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_default_to_rule_and_proxy_off() {
        let settings: Settings = serde_json::from_str(
            r#"{"controller_port":9090,"mixed_port":7897,"secret":"test","dark":true}"#,
        )
        .unwrap();
        settings.validate().unwrap();
        assert_eq!(settings.run_mode, "rule");
        assert_eq!(settings.proxy_mode, crate::engine::ProxyMode::Off);
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
