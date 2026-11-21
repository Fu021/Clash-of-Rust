use super::{Plan, package};
use crate::update::{UpdateSource, package_manager_update_hint};
use anyhow::{Context, Result, bail, ensure};
use std::{path::Path, process::Command};

#[derive(Clone, Copy)]
pub(super) enum Installer {
    Deb,
    Arch,
}

impl Installer {
    pub(super) fn for_executable(executable: &Path) -> Result<Self> {
        match UpdateSource::at(executable) {
            UpdateSource::Standard => Ok(Self::Deb),
            UpdateSource::ArchRelease => Ok(Self::Arch),
            UpdateSource::External(manager) => bail!("{}", package_manager_update_hint(manager)),
        }
    }

    pub(super) fn tool(self) -> &'static str {
        match self {
            Self::Deb => "/usr/bin/dpkg",
            Self::Arch => "/usr/bin/pacman",
        }
    }

    pub(super) fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::Deb => &["-i"],
            Self::Arch => &["-U", "--noconfirm", "--"],
        }
    }

    pub(super) fn format(self) -> &'static str {
        match self {
            Self::Deb => "DEB",
            Self::Arch => "Arch 原生包",
        }
    }

    pub(super) fn validate(self, plan: &Plan, verified: &package::Verified) -> Result<()> {
        ensure!(
            Path::new(self.tool()).is_file(),
            "当前系统缺少 {}，无法安装 {}",
            self.tool(),
            self.format()
        );
        match self {
            Self::Deb => ensure!(
                plan.package
                    .extension()
                    .is_some_and(|suffix| suffix == "deb"),
                "当前安装需要 DEB 更新包"
            ),
            Self::Arch => {
                let expected_name = UpdateSource::ArchRelease
                    .package_name(&plan.version)
                    .context("无法确定 Arch 更新包名称")?;
                ensure!(
                    plan.package
                        .file_name()
                        .is_some_and(|name| name == expected_name.as_str()),
                    "当前安装需要官方 Arch 更新包"
                );
                ensure!(
                    self.query("-Qp", &verified.path)? == arch_version(&plan.version)?,
                    "Arch 安装包的版本与 Release 不一致"
                );
                ensure!(
                    self.query("-Q", Path::new("clash-of-rust"))? == arch_version(crate::VERSION)?,
                    "已安装的 Arch 版本发生变化，请重新打开客户端后检查更新"
                );
            }
        }
        Ok(())
    }

    fn query(self, flag: &str, path: &Path) -> Result<String> {
        let result = Command::new(self.tool())
            .args([flag, "--"])
            .arg(path)
            .output()
            .context("无法读取 Arch 包信息")?;
        let output = std::str::from_utf8(&result.stdout).context("Arch 包信息格式无效")?;
        let fields: Vec<_> = output.split_whitespace().collect();
        match fields.as_slice() {
            ["clash-of-rust", version] if result.status.success() => Ok((*version).into()),
            _ => bail!("Arch 包的身份与官方安装不一致"),
        }
    }
}

fn arch_version(value: &str) -> Result<String> {
    let version = semver::Version::parse(value).context("Arch 安装包版本号无效")?;
    ensure!(version.build.is_empty(), "Arch 安装包版本号无效");
    let base = format!("{}.{}.{}", version.major, version.minor, version.patch);
    if version.pre.is_empty() {
        Ok(format!("{base}-1"))
    } else {
        Ok(format!(
            "{base}pre.{}-1",
            version.pre.as_str().replace('-', "+")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::arch_version;

    #[test]
    fn arch_versions_match_the_native_package_recipe() {
        assert_eq!(arch_version("0.7.2").unwrap(), "0.7.2-1");
        assert_eq!(
            arch_version("0.7.3-dev.1.tui-and-lists").unwrap(),
            "0.7.3pre.dev.1.tui+and+lists-1"
        );
        assert!(arch_version("0.7.3+invalid").is_err());
    }
}
