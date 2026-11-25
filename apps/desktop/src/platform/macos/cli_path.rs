use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::{fs::OpenOptionsExt, fs::symlink},
    path::Path,
};

const BIN: &str = ".local/share/clash-of-rust/bin";
const MARKER: &str = "# >>> Clash of Rust cor >>>";
const SH_CONFIG: &str = r#"
# >>> Clash of Rust cor >>>
if [ -x "$HOME/.local/share/clash-of-rust/bin/cor" ] && ! command -v cor >/dev/null 2>&1; then
    export PATH="${PATH:+$PATH:}$HOME/.local/share/clash-of-rust/bin"
fi
# <<< Clash of Rust cor <<<
"#;
const FISH_CONFIG: &str = r#"
# >>> Clash of Rust cor >>>
if test -x "$HOME/.local/share/clash-of-rust/bin/cor"; and not type -q cor
    set -gx PATH $PATH "$HOME/.local/share/clash-of-rust/bin"
end
# <<< Clash of Rust cor <<<
"#;

#[cfg(target_os = "macos")]
pub(crate) fn install() {
    let Some(home) = directories::BaseDirs::new() else {
        return;
    };
    let home = home.home_dir();
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let Ok(executable) = executable.canonicalize() else {
        return;
    };
    let Some(app) = executable.ancestors().nth(3) else {
        return;
    };
    if app.extension().is_none_or(|extension| extension != "app")
        || executable != app.join("Contents/MacOS/clash-of-rust")
        || ![
            Path::new("/Applications").to_path_buf(),
            home.join("Applications"),
        ]
        .into_iter()
        .any(|directory| {
            directory
                .canonicalize()
                .is_ok_and(|directory| app.parent() == Some(directory.as_path()))
        })
    {
        return;
    }
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
    let shell = Path::new(&shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("zsh");
    let config_root = match shell {
        "zsh" => std::env::var_os("ZDOTDIR"),
        "fish" => std::env::var_os("XDG_CONFIG_HOME"),
        _ => None,
    };
    let _ = configure(
        home,
        shell,
        config_root.as_deref().map(Path::new),
        &app.join("Contents/MacOS/cor"),
    );
}

fn configure(
    home: &Path,
    shell: &str,
    config_root: Option<&Path>,
    executable: &Path,
) -> io::Result<()> {
    let bin = home.join(BIN);
    let link = bin.join("cor");
    if !executable.is_file() {
        return Ok(());
    }
    let config_root = config_root.filter(|directory| directory.is_absolute());
    let (profile, contents) = match shell {
        "zsh" => (config_root.unwrap_or(home).join(".zprofile"), SH_CONFIG),
        "bash" => (
            [".bash_profile", ".bash_login", ".profile"]
                .iter()
                .map(|name| home.join(name))
                .find(|path| path.exists() || path.is_symlink())
                .unwrap_or_else(|| home.join(".bash_profile")),
            SH_CONFIG,
        ),
        "sh" | "ksh" => (home.join(".profile"), SH_CONFIG),
        "fish" => (
            config_root
                .unwrap_or(&home.join(".config"))
                .join("fish/conf.d/clash-of-rust.fish"),
            FISH_CONFIG,
        ),
        _ => return Ok(()),
    };
    fs::create_dir_all(&bin)?;
    match symlink(executable, &link) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::read_link(&link).as_deref().ok() != Some(executable) {
                return Ok(());
            }
        }
        Err(error) => return Err(error),
    }
    if let Some(directory) = profile.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(profile)?;
    if !file.metadata()?.is_file() {
        return Ok(());
    }
    file.lock()?;
    let mut previous = Vec::new();
    const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
    (&mut file)
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut previous)?;
    if previous.len() as u64 <= MAX_PROFILE_BYTES
        && !previous
            .split(|byte| *byte == b'\n')
            .any(|line| line == MARKER.as_bytes())
    {
        file.write_all(contents.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_one_profile_entry_and_preserves_existing_profile() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let home = root.path();
        let executable = home.join("Clash of Rust.app/Contents/MacOS/cor");
        fs::create_dir_all(executable.parent().unwrap())?;
        fs::write(&executable, b"fixture")?;
        let profile = home.join(".bash_login");
        fs::write(&profile, "export EXISTING=1")?;
        configure(home, "bash", None, &executable)?;
        configure(home, "bash", None, &executable)?;
        assert_eq!(fs::read_link(home.join(BIN).join("cor"))?, executable);
        assert_eq!(
            fs::read_to_string(profile)?,
            format!("export EXISTING=1{SH_CONFIG}")
        );
        assert!(!home.join(".bash_profile").exists());
        Ok(())
    }

    #[test]
    fn leaves_a_different_cor_and_profile_unchanged() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let home = root.path();
        let executable = home.join("cor");
        fs::write(&executable, b"fixture")?;
        let bin = home.join(BIN);
        fs::create_dir_all(&bin)?;
        let link = bin.join("cor");
        fs::write(&link, b"user command")?;
        configure(home, "zsh", None, &executable)?;
        assert_eq!(fs::read(&link)?, b"user command");
        assert!(!home.join(".zprofile").exists());
        Ok(())
    }

    #[test]
    fn shell_loads_the_link_only_when_no_cor_is_already_available() -> io::Result<()> {
        use std::{os::unix::fs::PermissionsExt, process::Command};

        let root = tempfile::tempdir()?;
        let home = root.path().join("home with spaces");
        fs::create_dir(&home)?;
        let config = home.join("shell config");
        let executable = home.join("original cor");
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n")?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))?;
        configure(&home, "zsh", Some(&config), &executable)?;
        configure(&home, "zsh", Some(&config), &executable)?;
        assert!(!home.join(".zprofile").exists());
        let shell = |line: &str| {
            Command::new("/bin/sh")
                .env_clear()
                .env("HOME", &home)
                .env("PATH", "/usr/bin:/bin")
                .env("ZDOTDIR", &config)
                .args(["-c", line])
                .output()
        };
        let launched = shell(r#". "$ZDOTDIR/.zprofile"; cor 'argument with spaces'"#)?;
        assert!(launched.status.success(), "{:?}", launched.stderr);
        assert_eq!(launched.stdout, b"argument with spaces\n");
        let existing = shell(r#"cor() { printf '%s\n' existing; }; . "$ZDOTDIR/.zprofile"; cor"#)?;
        assert!(existing.status.success(), "{:?}", existing.stderr);
        assert_eq!(existing.stdout, b"existing\n");
        fs::remove_file(&executable)?;
        let removed = shell(r#". "$ZDOTDIR/.zprofile"; test "$PATH" = /usr/bin:/bin"#)?;
        assert!(removed.status.success(), "{:?}", removed.stderr);
        Ok(())
    }
}
