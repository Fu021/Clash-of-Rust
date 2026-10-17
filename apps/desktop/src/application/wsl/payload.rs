//! Compiled payload manifest. Pin every file that the broker may execute/import.
use std::path::Path;

const FILES: &[(&str, &[u8])] = &[
    (
        "DockerEgressGuard.cs",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/DockerEgressGuard.cs"
        )),
    ),
    (
        "Guardian-Audit.ps1",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/Guardian-Audit.ps1"
        )),
    ),
    (
        "Uninstall-Check.ps1",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/Uninstall-Check.ps1"
        )),
    ),
    (
        "Manage-WslSecureEgress.ps1",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/Manage-WslSecureEgress.ps1"
        )),
    ),
    (
        "WslSecureEgress.psm1",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/WslSecureEgress.psm1"
        )),
    ),
    (
        "linux/manager",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/manager"
        )),
    ),
    (
        "linux/manager.py",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/manager.py"
        )),
    ),
    (
        "linux/secure-egress",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/secure-egress"
        )),
    ),
    (
        "linux/cor-egress-manager",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/cor-egress-manager"
        )),
    ),
    (
        "linux/install-payload.py",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/install-payload.py"
        )),
    ),
    (
        "linux/CONTRACT.md",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/linux/CONTRACT.md"
        )),
    ),
    (
        "server/cor-wsl-egress.service",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/server/cor-wsl-egress.service"
        )),
    ),
    (
        "server/manage.py",
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/wsl-secure-egress/server/manage.py"
        )),
    ),
];

/// Keep Windows files and directories open without write/delete sharing until
/// the child exits, closing the verify-to-import replacement window.
pub(super) struct Verified {
    _locks: Vec<std::fs::File>,
}

pub(super) fn verify(package: &Path) -> Result<Verified, ()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut locks = Vec::new();
    for path in [
        package.to_owned(),
        package.join("linux"),
        package.join("server"),
    ] {
        if !plain_directory(&path) {
            return Err(());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            locks.push(
                std::fs::OpenOptions::new()
                    .read(true)
                    .share_mode(1)
                    .custom_flags(0x02000000 | 0x00200000)
                    .open(&path)
                    .map_err(|_| ())?,
            );
        }
    }
    // The PS installer enumerates this directory; an injected extra payload
    // must fail verification even if all known files have their correct hash.
    for directory in ["linux", "server"] {
        let mut count = 0;
        for entry in std::fs::read_dir(package.join(directory)).map_err(|_| ())? {
            let entry = entry.map_err(|_| ())?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(())?;
            if !FILES
                .iter()
                .any(|(path, _)| path.strip_prefix(&(directory.to_owned() + "/")) == Some(name))
            {
                return Err(());
            }
            count += 1;
        }
        if count
            != FILES
                .iter()
                .filter(|(path, _)| path.starts_with(&(directory.to_owned() + "/")))
                .count()
        {
            return Err(());
        }
    }
    for (name, expected) in FILES {
        let path = package.join(name);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| ())?;
        if !metadata.is_file() || is_link(&metadata) || metadata.len() != expected.len() as u64 {
            return Err(());
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let mut file = options.open(&path).map_err(|_| ())?;
        if is_link(&file.metadata().map_err(|_| ())?) {
            return Err(());
        }
        let mut content = Vec::with_capacity(expected.len());
        (&mut file)
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| ())?;
        if Sha256::digest(&content) != Sha256::digest(expected) {
            return Err(());
        }
        locks.push(file);
    }
    Ok(Verified { _locks: locks })
}

fn plain_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !is_link(&m))
}

fn is_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("linux")).unwrap();
        std::fs::create_dir(root.path().join("server")).unwrap();
        for (name, bytes) in FILES {
            std::fs::write(root.path().join(name), bytes).unwrap();
        }
        root
    }

    #[test]
    fn manifest_rejects_module_and_linux_replacements_and_additions() {
        let root = package();
        assert!(verify(root.path()).is_ok());
        for name in ["WslSecureEgress.psm1", "linux/manager.py"] {
            let original = std::fs::read(root.path().join(name)).unwrap();
            let mut changed = original.clone();
            changed[0] ^= 1;
            std::fs::write(root.path().join(name), changed).unwrap();
            assert!(verify(root.path()).is_err(), "{name} must be pinned");
            std::fs::write(root.path().join(name), original).unwrap();
        }
        std::fs::write(root.path().join("linux/injected"), b"untrusted").unwrap();
        assert!(verify(root.path()).is_err());
    }

    #[test]
    fn manifest_rejects_missing_and_truncated_payloads() {
        let root = package();
        std::fs::remove_file(root.path().join("linux/manager")).unwrap();
        assert!(verify(root.path()).is_err());
        let root = package();
        std::fs::write(root.path().join("Manage-WslSecureEgress.ps1"), b"").unwrap();
        assert!(verify(root.path()).is_err());
    }
}
