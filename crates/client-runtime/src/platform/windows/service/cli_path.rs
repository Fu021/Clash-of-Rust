use super::security;
use anyhow::{Context, Result, ensure};
use std::{fs, io::ErrorKind, ptr};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use winreg::{
    RegKey,
    enums::{
        HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_EXPAND_SZ, REG_SZ,
    },
    types::{FromRegValue, ToRegValue},
};

const PRODUCT: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\ClashOfRust";
const ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
const OWNED: &str = "CliPathOwned";

pub(super) fn set(enabled: bool) -> Result<()> {
    ensure!(security::administrator()?, "更新系统 PATH 需要管理员权限");
    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("无法定位客户端目录")?;
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let access = KEY_QUERY_VALUE | KEY_SET_VALUE | KEY_WOW64_64KEY;
    let product = match machine.open_subkey_with_flags(PRODUCT, access) {
        Ok(product) => product,
        Err(error) if !enabled && error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let installed: String = product.get_value("InstallLocation")?;
    let directory = fs::canonicalize(directory)?;
    ensure!(
        directory == fs::canonicalize(&installed)?,
        "客户端路径与安装记录不一致"
    );
    let owned: Option<String> = match product.get_value(OWNED) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if !enabled && owned.is_none() {
        return Ok(());
    }
    let program_files = fs::canonicalize(security::program_files()?)?;
    let path = directory.to_str().context("客户端路径不是有效的 Unicode")?;
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    if enabled {
        ensure!(
            directory != program_files
                && directory.starts_with(&program_files)
                && directory.join("cor.exe").is_file()
                && !path.contains([';', '%', '"', '\r', '\n']),
            "仅支持将 Program Files 下的客户端添加到系统 PATH"
        );
    } else {
        ensure!(owned.as_deref() == Some(path), "PATH 注册与安装目录不一致");
    }
    let environment = machine.open_subkey_with_flags(ENVIRONMENT, access)?;
    let (current, kind) = match environment.get_raw_value("Path") {
        Ok(value) => {
            ensure!(
                matches!(value.vtype, REG_SZ | REG_EXPAND_SZ),
                "系统 PATH 的注册表类型无效"
            );
            let text = std::ffi::OsString::from_reg_value(&value)?;
            (
                text.into_string()
                    .map_err(|_| anyhow::anyhow!("系统 PATH 不是有效的 Unicode"))?,
                value.vtype,
            )
        }
        Err(error) if error.kind() == ErrorKind::NotFound => (String::new(), REG_EXPAND_SZ),
        Err(error) => return Err(error.into()),
    };
    ensure!(!current.contains('\0'), "系统 PATH 包含无效字符");
    let updated = edit(&current, path, enabled);
    if let Some(updated) = updated {
        ensure!(
            updated.encode_utf16().count() < 32_767,
            "更新后的系统 PATH 超过长度限制"
        );
        if enabled && owned.is_none() {
            product.set_value(OWNED, &path)?;
        }
        let mut value = updated.to_reg_value();
        value.vtype = kind;
        if let Err(error) = environment.set_raw_value("Path", &value) {
            if enabled && owned.is_none() {
                let _ = product.delete_value(OWNED);
            }
            return Err(error.into());
        }
        let setting = security::wide("Environment");
        // SAFETY: The terminated string is live for the synchronous broadcast;
        // a null result pointer discards receiver-specific return values.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                setting.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                200,
                ptr::null_mut(),
            );
        }
    }
    if !enabled {
        product.delete_value(OWNED)?;
    }
    Ok(())
}

fn edit(current: &str, directory: &str, enabled: bool) -> Option<String> {
    let mut entries: Vec<_> = current.split(';').collect();
    if enabled {
        let matches = |entry: &str| {
            entry
                .trim()
                .trim_matches('"')
                .trim_end_matches(['\\', '/'])
                .replace('/', "\\")
                .eq_ignore_ascii_case(directory)
        };
        if entries.iter().any(|entry| matches(entry)) {
            return None;
        }
        if current.is_empty() {
            entries.clear();
        }
        entries.push(directory);
    } else {
        let index = entries.iter().rposition(|entry| *entry == directory)?;
        entries.remove(index);
    }
    Some(entries.join(";"))
}

#[cfg(test)]
mod tests {
    use super::edit;

    const INSTALLATION: &str = r"C:\Program Files\测试目录";

    #[test]
    fn appends_after_existing_entries_and_keeps_registry_text() {
        let current = r"%SystemRoot%\system32;;C:\tools";
        let updated = format!("{current};{INSTALLATION}");
        assert_eq!(edit(current, INSTALLATION, true), Some(updated.clone()));
        assert_eq!(edit(&updated, INSTALLATION, true), None);
        assert_eq!(
            edit(&updated, INSTALLATION, false).as_deref(),
            Some(current)
        );
        assert_eq!(edit("", INSTALLATION, true).as_deref(), Some(INSTALLATION));
    }

    #[test]
    fn avoids_duplicate_entries_and_removes_only_the_recorded_spelling() {
        let existing = r#""c:/program files/测试目录/";C:\other"#;
        assert_eq!(edit(existing, INSTALLATION, true), None);
        assert_eq!(edit(existing, INSTALLATION, false), None);
        let duplicate = format!("{INSTALLATION};C:\\other;{INSTALLATION}");
        assert_eq!(
            edit(&duplicate, INSTALLATION, false),
            Some(format!("{INSTALLATION};C:\\other"))
        );
    }
}
