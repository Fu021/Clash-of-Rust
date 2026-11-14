use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

pub const NAME: &str = "clash-of-rust-cli.service";

pub fn path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    let base = base.filter(|path| path.is_absolute());
    Ok(base
        .ok_or_else(|| anyhow::anyhow!("无法确定用户配置目录"))?
        .join("systemd/user")
        .join(NAME))
}

pub fn definition(executable: &Path, data: &Path, resources: Option<&Path>) -> Result<String> {
    let mut arguments = vec![
        executable.as_os_str(),
        "run".as_ref(),
        "--data-dir".as_ref(),
        data.as_os_str(),
    ];
    if let Some(resources) = resources {
        arguments.extend(["--resources".as_ref(), resources.as_os_str()]);
    }
    let arguments = arguments
        .into_iter()
        .map(|argument| {
            let argument = argument
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("服务路径必须是 UTF-8"))?;
            ensure!(
                !argument.contains(['\0', '\n', '\r']),
                "服务路径不能包含控制字符"
            );
            Ok(format!(
                "\"{}\"",
                argument
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('%', "%%")
                    .replace('$', "$$")
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(" ");
    Ok(format!(
        "[Unit]\nDescription=Clash of Rust CLI\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={arguments}\nTimeoutStopSec=45\n\n[Install]\nWantedBy=default.target\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_preserves_installed_units_and_quotes_paths() {
        let text = definition(
            Path::new("/opt/clash-of-rust/cor"),
            Path::new("/home/example/profile \"$x%\\"),
            Some(Path::new("/opt/clash-of-rust/resources")),
        )
        .unwrap();
        assert!(text.contains(
            "ExecStart=\"/opt/clash-of-rust/cor\" \"run\" \"--data-dir\" \"/home/example/profile \\\"$$x%%\\\\\" \"--resources\" \"/opt/clash-of-rust/resources\"\n"
        ));
        assert!(text.contains("Type=simple\n"));
        assert!(text.contains("WantedBy=default.target\n"));
        for bad in ["/profile\nother", "/profile\r", "/profile\0"] {
            assert!(definition(Path::new("/cor"), Path::new(bad), None).is_err());
        }
    }
}
