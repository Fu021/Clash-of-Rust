#[cfg(any(windows, test))]
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Failure {
    schema: u32,
    stage: String,
    file: String,
    win32_error: u32,
    exit_code: u32,
}

pub(super) fn read_failure(directory: &Path) -> Option<Failure> {
    let file = File::open(directory.join("installer-failure.json")).ok()?;
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let failure: Failure = serde_json::from_slice(&bytes).ok()?;
    let valid_file = match failure.file.rsplit_once('/') {
        Some((".", name)) => matches!(
            name,
            "clash-of-rust.exe"
                | "clash-desktop-ui.exe"
                | "cor.exe"
                | "clash-tun-launcher.exe"
                | "LICENSE"
                | "uninstall.exe"
        ),
        Some(("resources/ip-check", name)) => matches!(name, "LICENSE" | "SOURCE.md"),
        Some(("resources", name)) => {
            matches!(
                name,
                "mihomo.exe"
                    | "GeoIP.dat"
                    | "GeoSite.dat"
                    | "Country.mmdb"
                    | "ASN.mmdb"
                    | "geodata.json"
                    | "core.json"
                    | "default.yaml"
                    | "settings-defaults.json"
                    | "THIRD-PARTY-NOTICES.txt"
                    | "SourceHanSans-LICENSE.txt"
                    | "NotoEmoji-LICENSE.txt"
                    | "Unicode-LICENSE.txt"
            ) || (name.starts_with("clash-of-rust-")
                && name.ends_with(".ico")
                && name.len() <= 160
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-._".contains(&byte)))
        }
        _ => false,
    };
    (failure.schema == 1
        && valid_file
        && matches!(
            (failure.stage.as_str(), failure.exit_code),
            ("preflight", 68) | ("extract", 6) | ("uninstaller", 64)
        )
        && (failure.stage != "uninstaller" || failure.file == "./uninstall.exe"))
        .then_some(failure)
}

#[cfg(any(windows, test))]
pub(super) fn result(code: u32, directory: &Path) -> Result<()> {
    if code == 0 {
        return Ok(());
    }
    if let Some(failure) = read_failure(directory).filter(|failure| failure.exit_code == code) {
        let reason = match failure.win32_error {
            5 => "没有写入权限、文件只读或被安全软件阻止",
            19 => "磁盘为只读",
            32 | 33 => "文件仍被占用",
            39 | 112 => "磁盘空间不足",
            _ => "无法写入文件",
        };
        let file = failure.file.strip_prefix("./").unwrap_or(&failure.file);
        bail!("安装未完成：{reason}（{file}），请处理后重试");
    }
    match code {
        2 => bail!("安装已取消"),
        3 | 5 => bail!("客户端未能正常退出或文件被占用，请退出后重试"),
        4 => bail!("安装包架构与当前 Windows 不兼容，请下载匹配的安装包"),
        6 => bail!("安装文件写入失败，具体原因未确认，请导出诊断后重试"),
        61 => bail!("无法替换主程序：文件被占用或安装目录不可写，已保留旧版本"),
        62 => bail!("无法替换界面程序：文件被占用或安装目录不可写，已恢复旧版本"),
        64 => bail!("无法写入卸载程序：文件被占用或安装目录不可写，已恢复旧版本"),
        65 => bail!("Windows 网络助手安装失败，已恢复旧版主程序与界面"),
        66 => bail!("无法写入安装登记信息，已恢复旧版主程序与界面"),
        67 => bail!("安装交接未确认或已超时，已保留旧版本"),
        68 => bail!("安装目录中的文件不可写或仍被占用，未替换旧文件，请稍后重试"),
        _ => bail!("安装程序失败（代码 {code}），请重试"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installer_errors_preserve_the_phase_and_distinguish_locks_permissions_and_space() {
        let directory = tempfile::tempdir().unwrap();
        for (code, message) in [(32, "仍被占用"), (5, "权限"), (112, "空间不足")] {
            let failure = serde_json::json!({
                "schema": 1, "stage": "preflight", "file": "resources/GeoIP.dat",
                "win32_error": code, "exit_code": 68
            });
            std::fs::write(
                directory.path().join("installer-failure.json"),
                serde_json::to_vec(&failure).unwrap(),
            )
            .unwrap();
            let error = result(68, directory.path()).unwrap_err().to_string();
            assert!(error.contains(message) && error.contains("resources/GeoIP.dat"));
            assert_eq!(read_failure(directory.path()).unwrap().stage, "preflight");
            assert!(
                !result(65, directory.path())
                    .unwrap_err()
                    .to_string()
                    .contains("GeoIP")
            );
            assert!(result(0, directory.path()).is_ok());
        }
    }

    #[test]
    fn malformed_or_untrusted_installer_details_do_not_replace_the_exit_status() {
        let directory = tempfile::tempdir().unwrap();
        let valid = serde_json::json!({
            "schema": 1, "stage": "extract", "file": "./cor.exe",
            "win32_error": 32, "exit_code": 6
        });
        for (field, value) in [
            ("schema", serde_json::json!(2)),
            ("stage", serde_json::json!("untrusted text")),
            ("stage", serde_json::json!("preflight")),
            ("file", serde_json::json!("../private/token")),
            ("file", serde_json::json!("resources/../../secret.ico")),
            ("file", serde_json::json!("resources/clash-of-rust-\".ico")),
            ("extra", serde_json::json!("unexpected")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            std::fs::write(
                directory.path().join("installer-failure.json"),
                serde_json::to_vec(&invalid).unwrap(),
            )
            .unwrap();
            assert!(read_failure(directory.path()).is_none());
            assert!(
                result(6, directory.path())
                    .unwrap_err()
                    .to_string()
                    .contains("写入失败")
            );
        }
        std::fs::write(
            directory.path().join("installer-failure.json"),
            vec![b' '; 4097],
        )
        .unwrap();
        assert!(read_failure(directory.path()).is_none());
        std::fs::remove_file(directory.path().join("installer-failure.json")).unwrap();
        assert!(
            result(68, directory.path())
                .unwrap_err()
                .to_string()
                .contains("未替换旧文件")
        );
    }
}
