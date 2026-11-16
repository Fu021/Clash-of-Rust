#![deny(clippy::undocumented_unsafe_blocks)]

use anyhow::{Context, Result, ensure};
use std::{
    ffi::{CStr, CString},
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};

const FILE_LIMIT: u64 = 128 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 512 * 1024 * 1024;

pub(super) struct Snapshot {
    pub directory: tempfile::TempDir,
    original: File,
    uid: u32,
}

fn open_at(directory: &File, name: &CStr, flags: i32, mode: libc::mode_t) -> Result<File> {
    // SAFETY: directory owns a live fd and name is NUL terminated; only a new fd is returned.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            libc::c_uint::from(mode),
        )
    };
    ensure!(
        descriptor >= 0,
        "无法安全打开运行数据：{}",
        std::io::Error::last_os_error()
    );
    // SAFETY: the successful openat returned a new fd whose ownership transfers once.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

fn check(file: &File, uid: u32, directory: bool) -> Result<std::fs::Metadata> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.uid() == uid
            && metadata.mode() & 0o022 == 0
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file() && metadata.nlink() == 1 && metadata.mode() & 0o111 == 0
            },
        "运行数据必须是当前用户的私有目录或普通文件"
    );
    Ok(metadata)
}

fn host_state(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".cor-cache-"))
        .is_some_and(|suffix| uuid::Uuid::parse_str(suffix).is_ok())
    {
        return true;
    }
    let name = path
        .components()
        .next()
        .unwrap()
        .as_os_str()
        .to_string_lossy();
    name == "temp"
        || name == "geo-transaction.json"
        || [".geo-download-", ".geo-backup-", ".geo-recovery-"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn collect(
    directory: &File,
    relative: &Path,
    uid: u32,
    files: &mut Vec<(PathBuf, File, u64)>,
    total: &mut u64,
    visited: &mut usize,
) -> Result<()> {
    ensure!(relative.components().count() <= 16, "内核运行目录嵌套过深");
    // SAFETY: F_DUPFD_CLOEXEC duplicates a live directory fd; fdopendir takes ownership on success.
    let descriptor = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    ensure!(descriptor >= 0, "无法读取运行目录");
    // SAFETY: descriptor is a new directory fd, transferred to the DIR on success.
    let entries = unsafe { libc::fdopendir(descriptor) };
    if entries.is_null() {
        // SAFETY: fdopendir failed and left the new descriptor owned by this function.
        unsafe { libc::close(descriptor) };
        anyhow::bail!("无法列出运行目录");
    }
    struct Entries(*mut libc::DIR);
    impl Drop for Entries {
        fn drop(&mut self) {
            // SAFETY: this guard uniquely owns the live DIR created by fdopendir.
            unsafe { libc::closedir(self.0) };
        }
    }
    let entries = Entries(entries);
    loop {
        // SAFETY: __error returns the calling thread's errno pointer.
        unsafe { *libc::__error() = 0 };
        // SAFETY: entries owns a live DIR and no concurrent code accesses its cursor.
        let entry = unsafe { libc::readdir(entries.0) };
        if entry.is_null() {
            ensure!(
                std::io::Error::last_os_error().raw_os_error() == Some(0),
                "读取运行目录失败"
            );
            break;
        }
        // SAFETY: readdir returned a live entry with a NUL terminated name, used before the next call.
        let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        *visited += 1;
        ensure!(*visited <= 4096, "内核运行目录项目过多");
        use std::os::unix::ffi::OsStrExt;
        let child = relative.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
        if host_state(&child) {
            continue;
        }
        let file = open_at(directory, name, libc::O_RDONLY, 0)?;
        if file.metadata()?.is_dir() {
            check(&file, uid, true)?;
            ensure!(
                file.metadata()?.dev() == directory.metadata()?.dev(),
                "运行目录不能跨文件系统"
            );
            collect(&file, &child, uid, files, total, visited)?;
        } else {
            let metadata = check(&file, uid, false)?;
            ensure!(metadata.len() <= FILE_LIMIT, "内核文件超过 128 MiB");
            *total = total
                .checked_add(metadata.len())
                .context("内核文件大小无效")?;
            ensure!(*total <= TOTAL_LIMIT, "内核运行数据超过 512 MiB");
            files.push((child, file, metadata.len()));
        }
    }
    Ok(())
}

impl Snapshot {
    pub fn new(original: &Path, uid: u32) -> Result<Self> {
        let root = File::options()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(original)?;
        check(&root, uid, true)?;
        let directory = tempfile::Builder::new()
            .prefix("clash-runtime-root-")
            .tempdir_in("/private/tmp")?;
        let mut files = Vec::new();
        collect(&root, Path::new(""), uid, &mut files, &mut 0, &mut 0)?;
        for (relative, source, size) in files {
            let destination = directory.path().join(relative);
            std::fs::create_dir_all(destination.parent().context("运行文件路径无效")?)?;
            let mut destination = File::options()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(destination)?;
            ensure!(
                std::io::copy(&mut source.take(size + 1), &mut destination)? == size,
                "复制时运行数据发生变化"
            );
        }
        let path = directory.path().join("config.yaml");
        let file = File::open(&path)?;
        ensure!(file.metadata()?.len() <= 32 * 1024 * 1024, "内核配置过大");
        let mut config: yaml_serde::Mapping = yaml_serde::from_reader(file)?;
        clash_client_core::config::ingress::privileged(&mut config)?;
        clash_client_core::config::ingress::provider_paths(&mut config, original)?;
        std::fs::write(path, yaml_serde::to_string(&config)?)?;
        std::fs::create_dir(directory.path().join("temp"))?;
        Ok(Self {
            directory,
            original: root,
            uid,
        })
    }

    pub fn save_cache(&self) -> Result<()> {
        let source = File::open(self.directory.path())?;
        let mut files = Vec::new();
        // SAFETY: geteuid has no pointer arguments and returns the current process identity.
        let owner = unsafe { libc::geteuid() };
        collect(&source, Path::new(""), owner, &mut files, &mut 0, &mut 0)?;
        for (relative, source, size) in files {
            if relative.components().count() == 1
                && !matches!(relative.to_str(), Some("cache.db" | ".cache"))
            {
                continue;
            }
            let mut parent = self.original.try_clone()?;
            for component in relative.parent().context("缓存路径无效")?.components() {
                let Component::Normal(component) = component else {
                    anyhow::bail!("缓存路径无效")
                };
                let name = CString::new(component.as_encoded_bytes())?;
                // SAFETY: parent is a live directory fd and name is a single NUL terminated component.
                let created =
                    unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } == 0;
                let child = open_at(&parent, &name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                if created {
                    set_owner(&child, self.uid, self.original.metadata()?.gid())?;
                }
                check(&child, self.uid, true)?;
                parent = child;
            }
            check(&parent, self.uid, true)?;
            let temporary = CString::new(format!(".cor-cache-{}", uuid::Uuid::new_v4()))?;
            let name = CString::new(
                relative
                    .file_name()
                    .context("缓存路径无效")?
                    .as_encoded_bytes(),
            )?;
            let mut staged = open_at(
                &parent,
                &temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            let result = (|| -> Result<()> {
                ensure!(
                    std::io::copy(&mut source.take(size + 1), &mut staged)? == size,
                    "缓存数据发生变化"
                );
                staged.sync_all()?;
                set_owner(&staged, self.uid, self.original.metadata()?.gid())?;
                ensure!(
                    // SAFETY: both names are single components in the same open directory; rename replaces an entry without following it.
                    unsafe {
                        libc::renameat(
                            parent.as_raw_fd(),
                            temporary.as_ptr(),
                            parent.as_raw_fd(),
                            name.as_ptr(),
                        )
                    } == 0,
                    "无法保存运行缓存"
                );
                Ok(())
            })();
            if result.is_err() {
                // SAFETY: only this function's uniquely named staging entry is removed from the open parent.
                unsafe { libc::unlinkat(parent.as_raw_fd(), temporary.as_ptr(), 0) };
            }
            result?;
        }
        Ok(())
    }
}

fn set_owner(file: &File, uid: u32, gid: u32) -> Result<()> {
    ensure!(
        // SAFETY: file owns a live fd; ownership is applied to that inode without resolving a path.
        unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } == 0,
        "无法设置运行缓存所有者"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("config.yaml"),
            "proxies: []\nexternal-controller: 127.0.0.1:9090\nsecret: test-secret\n",
        )
        .unwrap();
        directory
    }

    #[test]
    fn private_snapshot_rejects_links_executables_and_outside_provider_paths() {
        // SAFETY: getuid takes no arguments and returns the test process identity.
        let uid = unsafe { libc::getuid() };
        let directory = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("data"), b"unchanged").unwrap();
        symlink(
            outside.path().join("data"),
            directory.path().join("cache.db"),
        )
        .unwrap();
        assert!(Snapshot::new(directory.path(), uid).is_err());
        std::fs::remove_file(directory.path().join("cache.db")).unwrap();
        std::fs::write(directory.path().join("program"), b"executable").unwrap();
        std::fs::set_permissions(
            directory.path().join("program"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(Snapshot::new(directory.path(), uid).is_err());
        std::fs::remove_file(directory.path().join("program")).unwrap();
        for path in ["../outside", "/private/etc/hosts"] {
            let mut config: yaml_serde::Mapping =
                yaml_serde::from_str(&format!("proxy-providers: {{example: {{path: '{path}'}}}}"))
                    .unwrap();
            assert!(
                clash_client_core::config::ingress::provider_paths(&mut config, directory.path())
                    .is_err()
            );
        }
        assert_eq!(
            std::fs::read(outside.path().join("data")).unwrap(),
            b"unchanged"
        );
    }

    #[test]
    fn cache_restore_never_follows_user_links_or_overwrites_host_configuration() {
        // SAFETY: getuid takes no arguments and returns the test process identity.
        let uid = unsafe { libc::getuid() };
        let original = fixture();
        let outside = fixture();
        let snapshot = Snapshot::new(original.path(), uid).unwrap();
        std::fs::write(
            snapshot.directory.path().join("cache.db"),
            b"node selection",
        )
        .unwrap();
        std::fs::write(
            snapshot.directory.path().join("config.yaml"),
            b"private config",
        )
        .unwrap();
        symlink(
            outside.path().join("config.yaml"),
            original.path().join("cache.db"),
        )
        .unwrap();
        snapshot.save_cache().unwrap();
        assert_eq!(
            std::fs::read(original.path().join("cache.db")).unwrap(),
            b"node selection"
        );
        assert_ne!(
            std::fs::read(original.path().join("config.yaml")).unwrap(),
            b"private config"
        );
        assert_ne!(
            std::fs::read(outside.path().join("config.yaml")).unwrap(),
            b"node selection"
        );
        std::fs::create_dir(snapshot.directory.path().join("providers")).unwrap();
        std::fs::write(
            snapshot.directory.path().join("providers/data"),
            b"new cache",
        )
        .unwrap();
        symlink(outside.path(), original.path().join("providers")).unwrap();
        assert!(snapshot.save_cache().is_err());
        assert!(!outside.path().join("data").exists());
        let path = snapshot.directory.path().to_owned();
        drop(snapshot);
        assert!(!path.exists());
    }

    #[test]
    fn private_cache_roundtrip_preserves_geo_and_host_transactions() {
        // SAFETY: getuid takes no arguments and returns the test process identity.
        let uid = unsafe { libc::getuid() };
        let original = fixture();
        std::fs::create_dir(original.path().join("providers")).unwrap();
        std::fs::write(
            original.path().join("providers/nodes.yaml"),
            b"old provider",
        )
        .unwrap();
        std::fs::write(original.path().join("GeoIP.dat"), b"host geo").unwrap();
        std::fs::write(
            original.path().join("geo-transaction.json"),
            b"host transaction",
        )
        .unwrap();
        let snapshot = Snapshot::new(original.path(), uid).unwrap();
        assert!(
            !snapshot
                .directory
                .path()
                .join("geo-transaction.json")
                .exists()
        );
        std::fs::write(
            snapshot.directory.path().join("providers/nodes.yaml"),
            b"new provider",
        )
        .unwrap();
        std::fs::write(snapshot.directory.path().join("GeoIP.dat"), b"stale geo").unwrap();
        snapshot.save_cache().unwrap();
        assert_eq!(
            std::fs::read(original.path().join("providers/nodes.yaml")).unwrap(),
            b"new provider"
        );
        assert_eq!(
            std::fs::metadata(original.path().join("providers/nodes.yaml"))
                .unwrap()
                .uid(),
            uid
        );
        assert_eq!(
            std::fs::read(original.path().join("GeoIP.dat")).unwrap(),
            b"host geo"
        );
        assert_eq!(
            std::fs::read(original.path().join("geo-transaction.json")).unwrap(),
            b"host transaction"
        );
    }

    #[test]
    fn interrupted_cache_staging_does_not_block_the_next_session() {
        // SAFETY: getuid takes no arguments and returns the test process identity.
        let uid = unsafe { libc::getuid() };
        let original = fixture();
        std::fs::create_dir(original.path().join("providers")).unwrap();
        for parent in [
            original.path().to_owned(),
            original.path().join("providers"),
        ] {
            let name = format!(".cor-cache-{}", uuid::Uuid::new_v4());
            let staged = parent.join(&name);
            std::fs::write(&staged, b"incomplete cache transfer").unwrap();
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0)).unwrap();
        }
        let provider = Path::new("providers/.cor-cache-nodes.yaml");
        std::fs::write(original.path().join(provider), b"provider cache").unwrap();
        let snapshot = Snapshot::new(original.path(), uid).unwrap();
        assert!(
            std::fs::read_dir(snapshot.directory.path())
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .as_encoded_bytes()
                    .starts_with(b".cor-cache-"))
        );
        assert_eq!(
            std::fs::read(snapshot.directory.path().join(provider)).unwrap(),
            b"provider cache"
        );
        assert_eq!(
            std::fs::read_dir(snapshot.directory.path().join("providers"))
                .unwrap()
                .count(),
            1
        );
        snapshot.save_cache().unwrap();
    }
}
