use super::*;
use std::process::Command;
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
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
    Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

pub fn description() -> &'static str {
    "Linux GNOME 系统代理（gsettings）；KDE 尚待实现"
}

fn command(args: &[&str]) -> Result<String> {
    let output = Command::new("gsettings")
        .args(args)
        .output()
        .context("需要 GNOME gsettings 和有效桌面会话")?;
    if !output.status.success() {
        bail!("gsettings 操作失败，请检查 GNOME 桌面会话和代理设置权限");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
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
    let mut state = ProxyState::new();
    for entry in keys() {
        let (schema, key) = entry.split_once('|').unwrap();
        if command(&["writable", schema, key])? != "true" {
            bail!("GNOME 系统代理设置不可写");
        }
        state.insert(
            entry.into(),
            Some(ProxyValue::Text(command(&["get", schema, key])?)),
        );
    }
    Ok(state)
}

pub fn desired(port: u16) -> ProxyState {
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
        command(&["set", schema, key, value])?;
    }
    Ok(())
}
