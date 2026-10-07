//! KDE proxy configuration; preserves all unrelated KConfig entries.
use super::*;
const KEYS: [&str; 7] = [
    "ProxyType",
    "httpProxy",
    "httpsProxy",
    "ftpProxy",
    "socksProxy",
    "NoProxyFor",
    "ReversedException",
];

pub(super) fn active() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|d| d.eq_ignore_ascii_case("KDE"))
}
fn path() -> Result<std::path::PathBuf> {
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .context("无法确定 KDE 配置目录")?;
    Ok(directory.join("kioslaverc"))
}
fn contents() -> Result<String> {
    match std::fs::read_to_string(path()?) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}
fn entries(contents: &str) -> BTreeMap<String, String> {
    let mut active = false;
    let mut values = BTreeMap::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            active = trimmed == "[Proxy Settings]";
        } else if active
            && !trimmed.starts_with(['#', ';'])
            && let Some((key, value)) = trimmed.split_once('=')
        {
            values.insert(key.to_owned(), value.to_owned());
        }
    }
    values
}
pub(super) fn read() -> Result<ProxyState> {
    let values = entries(&contents()?);
    Ok(KEYS
        .into_iter()
        .map(|key| {
            (
                format!("kde|{key}"),
                values.get(key).cloned().map(ProxyValue::Text),
            )
        })
        .collect())
}
pub(super) fn desired(port: u16) -> ProxyState {
    let proxy = format!("http://127.0.0.1 {port}");
    let values = [
        "1".into(),
        proxy.clone(),
        proxy.clone(),
        proxy,
        format!("socks://127.0.0.1 {port}"),
        "localhost,127.0.0.0/8,::1".into(),
        "false".into(),
    ];
    KEYS.into_iter()
        .zip(values)
        .map(|(key, value)| (format!("kde|{key}"), Some(ProxyValue::Text(value))))
        .collect()
}
pub(super) fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    let values = entries(&contents()?);
    if values.get("ProxyType").map(String::as_str) != Some("1") {
        return Ok(None);
    }
    let Some(proxy) = values
        .get(&format!("{scheme}Proxy"))
        .or_else(|| values.get("httpProxy"))
    else {
        return Ok(None);
    };
    Ok(Some(if let Some((host, port)) = proxy.rsplit_once(' ') {
        format!("{host}:{port}")
    } else {
        proxy.clone()
    }))
}
fn update(contents: &str, state: &ProxyState) -> Result<String> {
    let mut values = entries(contents);
    for (key, value) in state {
        let name = key.strip_prefix("kde|").context("无效 KDE 代理恢复记录")?;
        anyhow::ensure!(KEYS.contains(&name), "无效 KDE 代理设置键");
        match value {
            None => {
                values.remove(name);
            }
            Some(ProxyValue::Text(value)) => {
                anyhow::ensure!(!value.contains(['\n', '\r']), "无效 KDE 代理设置值");
                values.insert(name.into(), value.clone());
            }
            _ => bail!("无效 KDE 代理值"),
        }
    }
    let mut output = Vec::new();
    let mut inside = false;
    let mut written = false;
    for line in contents.lines() {
        if line.trim().starts_with('[') {
            if inside && !written {
                output.extend(values.iter().map(|(k, v)| format!("{k}={v}")));
                written = true;
            }
            inside = line.trim() == "[Proxy Settings]";
            output.push(line.into());
        } else if !inside || line.trim().is_empty() || line.trim().starts_with(['#', ';']) {
            output.push(line.into());
        }
    }
    if !written {
        if !inside {
            output.push("[Proxy Settings]".into());
        }
        output.extend(values.iter().map(|(k, v)| format!("{k}={v}")));
    }
    Ok(output.join("\n") + "\n")
}
pub(super) fn write(state: &ProxyState) -> Result<()> {
    let current = contents()?;
    let updated = update(&current, state)?;
    // Obtain a working desktop session before changing the persisted setting.
    super::gio::notify_kde()?;
    atomic_write(&path()?, updated.as_bytes())?;
    if let Err(error) = super::gio::notify_kde() {
        atomic_write(&path()?, current.as_bytes())?;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_other_groups_unknown_keys_and_absent_original_values() {
        let original =
            "[Other]\nkey=untouched\n[Proxy Settings]\nhttpProxy=http://old 8080\nCustom=keep\n";
        let applied = update(original, &desired(7897)).unwrap();
        assert!(applied.contains("key=untouched") && applied.contains("Custom=keep"));
        assert_eq!(
            entries(&applied).get("httpProxy").unwrap(),
            "http://127.0.0.1 7897"
        );
        let restore = KEYS
            .into_iter()
            .map(|key| {
                (
                    format!("kde|{key}"),
                    entries(original).get(key).cloned().map(ProxyValue::Text),
                )
            })
            .collect();
        let restored = update(&applied, &restore).unwrap();
        assert_eq!(entries(&restored), entries(original));
    }
}
