use super::*;
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    if super::kde::active() {
        return super::kde::configured_proxy(scheme);
    }
    let state = read()?;
    if !matches!(state.get("org.gnome.system.proxy|mode"), Some(Some(ProxyValue::Text(mode))) if mode.trim_matches('\'') == "manual")
    {
        return Ok(None);
    }
    let kind = if scheme == "https" { "https" } else { "http" };
    for kind in [kind, "http"] {
        if let (Some(Some(ProxyValue::Text(host))), Some(Some(ProxyValue::Text(port)))) = (
            state.get(&format!("org.gnome.system.proxy.{kind}|host")),
            state.get(&format!("org.gnome.system.proxy.{kind}|port")),
        ) && !host.trim_matches('\'').is_empty()
            && let Ok(port) = port.parse::<u16>()
            && port > 0
        {
            let host = host.trim_matches('\'');
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.to_owned()
            };
            return Ok(Some(format!("http://{host}:{port}")));
        }
    }
    Ok(None)
}

fn startup_path() -> Result<std::path::PathBuf> {
    let base = if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        std::path::PathBuf::from(path)
    } else {
        std::path::PathBuf::from(std::env::var_os("HOME").context("无法确定用户配置目录")?)
            .join(".config")
    };
    Ok(base.join("autostart/clash-of-rust.desktop"))
}
pub fn autostart_enabled() -> Result<bool> {
    Ok(startup_path()?.exists())
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    let path = startup_path()?;
    if enabled {
        std::fs::create_dir_all(path.parent().context("自启路径无效")?)?;
        let executable = std::env::current_exe()?
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('`', "\\`")
            .replace('$', "\\$")
            .replace('%', "%%");
        atomic_write(&path, format!("[Desktop Entry]\nType=Application\nName=Clash of Rust\nExec=\"{executable}\" --background\nTerminal=false\n").as_bytes())?;
    } else if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
pub fn open_directory(path: &Path) -> Result<()> {
    super::gio::open_directory(path)
}
pub fn open_url(url: &str) -> Result<()> {
    super::gio::open_uri(url)
}

pub fn description() -> &'static str {
    "Linux GNOME / KDE 原生系统代理"
}

fn keys() -> [&'static str; 7] {
    [
        "org.gnome.system.proxy|mode",
        "org.gnome.system.proxy.http|host",
        "org.gnome.system.proxy.http|port",
        "org.gnome.system.proxy.https|host",
        "org.gnome.system.proxy.https|port",
        "org.gnome.system.proxy|ignore-hosts",
        "org.gnome.system.proxy|use-same-proxy",
    ]
}

pub fn read() -> Result<ProxyState> {
    if super::kde::active() {
        return super::kde::read();
    }
    let mut state = ProxyState::new();
    for entry in keys() {
        let (schema, key) = entry.split_once('|').unwrap();
        if super::gio::setting(&["writable", schema, key])? != "true" {
            bail!("GNOME 系统代理设置不可写");
        }
        state.insert(
            entry.into(),
            Some(ProxyValue::Text(super::gio::setting(&[
                "get", schema, key,
            ])?)),
        );
    }
    Ok(state)
}

pub fn desired(port: u16) -> ProxyState {
    if super::kde::active() {
        return super::kde::desired(port);
    }
    let values = [
        "'manual'".into(),
        "'127.0.0.1'".into(),
        port.to_string(),
        "'127.0.0.1'".into(),
        port.to_string(),
        "['localhost', '127.0.0.0/8', '::1']".into(),
        "false".into(),
    ];
    keys()
        .into_iter()
        .zip(values)
        .map(|(key, value)| (key.into(), Some(ProxyValue::Text(value))))
        .collect()
}

pub fn write(state: &ProxyState) -> Result<()> {
    if state.keys().any(|k| k.starts_with("kde|")) {
        return super::kde::write(state);
    }
    // Apply the mode last, after host/port are configured.
    for (entry, value) in state
        .iter()
        .filter(|(key, _)| !key.ends_with("|mode"))
        .chain(state.iter().filter(|(key, _)| key.ends_with("|mode")))
    {
        let (schema, key) = entry.split_once('|').context("无效代理恢复记录")?;
        let Some(ProxyValue::Text(value)) = value else {
            bail!("无效 GNOME 代理值");
        };
        super::gio::setting(&["set", schema, key, value])?;
    }
    super::gio::sync();
    Ok(())
}
