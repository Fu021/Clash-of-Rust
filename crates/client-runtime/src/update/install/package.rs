#![deny(clippy::undocumented_unsafe_blocks)]

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek},
    path::{Path, PathBuf},
};

pub(super) struct Verified {
    pub path: PathBuf,
    _file: File,
    #[cfg(windows)]
    _directories: Vec<File>,
}

pub(super) fn open(path: &Path, expected: &str) -> Result<Verified> {
    #[cfg(windows)]
    let (path, directories) = lock_directories(path)?;
    #[cfg(not(windows))]
    let path = path.to_owned();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let source = options.open(&path).context("安装包已被删除、替换或拦截")?;
    let metadata = source.metadata()?;
    ensure!(metadata.is_file(), "安装包必须是普通文件");
    ensure!(
        metadata.len() <= super::super::PACKAGE_LIMIT,
        "安装包超过大小限制"
    );
    #[cfg(target_os = "linux")]
    let (mut file, path) = freeze(source)?;
    #[cfg(not(target_os = "linux"))]
    let mut file = source;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).context("无法读取安装包")?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    ensure!(
        clash_client_core::file_hash::hex(hash.finalize()) == expected,
        "安装包校验失败，已停止安装"
    );
    file.rewind()?;
    Ok(Verified {
        path,
        _file: file,
        #[cfg(windows)]
        _directories: directories,
    })
}

#[cfg(windows)]
fn lock_directories(path: &Path) -> Result<(PathBuf, Vec<File>)> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let path = path.canonicalize().context("安装包路径无效")?;
    let mut ancestors: Vec<_> = path.ancestors().skip(1).collect();
    ancestors.reverse();
    let mut directories = Vec::new();
    for directory in ancestors {
        directories.push(
            OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(directory)
                .context("无法锁定安装包目录，请关闭占用程序后重试")?,
        );
    }
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let wide: Vec<_> = path.as_os_str().encode_wide().collect();
    let shell_path = if wide.starts_with(&[92, 92, 63, 92]) {
        let normal = if wide[4..].starts_with(&[85, 78, 67, 92]) {
            [vec![92, 92], wide[8..].to_vec()].concat()
        } else {
            wide[4..].to_vec()
        };
        PathBuf::from(std::ffi::OsString::from_wide(&normal))
    } else {
        path
    };
    Ok((shell_path, directories))
}

#[cfg(target_os = "linux")]
fn freeze(source: File) -> Result<(File, PathBuf)> {
    use std::os::fd::{AsRawFd, FromRawFd};
    // SAFETY: the name is NUL terminated; a successful call returns a new owned fd.
    let descriptor = unsafe {
        libc::memfd_create(
            c"cor-update".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    ensure!(
        descriptor >= 0,
        "无法创建只读安装快照：{}",
        std::io::Error::last_os_error()
    );
    // SAFETY: descriptor is valid and its ownership is transferred exactly once.
    let mut file = unsafe { File::from_raw_fd(descriptor) };
    let size = source.metadata()?.len();
    ensure!(size <= super::super::PACKAGE_LIMIT, "安装包超过大小限制");
    ensure!(
        std::io::copy(&mut source.take(size + 1), &mut file).context("无法暂存安装包")? == size,
        "暂存时安装包大小发生变化"
    );
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: file owns the fd, F_ADD_SEALS takes an integer mask, and no mapping exists.
    ensure!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } == 0,
        "无法冻结安装包：{}",
        std::io::Error::last_os_error()
    );
    file.rewind()?;
    let path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        file.as_raw_fd()
    ));
    Ok((file, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> String {
        clash_client_core::file_hash::hex(Sha256::digest(b"verified package"))
    }

    #[test]
    fn corrupt_payloads_and_directories_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("package");
        std::fs::write(&source, b"tampered").unwrap();
        assert!(open(&source, &expected()).is_err());
        assert!(open(directory.path(), &expected()).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn verified_bytes_survive_source_replacement_and_reject_writes_through_proc() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("package.deb");
        std::fs::write(&source, b"verified package").unwrap();
        let verified = open(&source, &expected()).unwrap();
        std::fs::write(&source, b"tampered").unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, b"replacement").unwrap();
        assert_eq!(std::fs::read(&verified.path).unwrap(), b"verified package");
        assert!(std::fs::write(&verified.path, b"tampered").is_err());
        assert!(
            OpenOptions::new()
                .write(true)
                .open(&verified.path)
                .unwrap()
                .set_len(0)
                .is_err()
        );
        let path = verified.path.clone();
        drop(verified);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    fn verified_package_and_its_directory_cannot_be_replaced_until_installation_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("package.exe");
        std::fs::write(&source, b"verified package").unwrap();
        let verified = open(&source, &expected()).unwrap();
        assert!(std::fs::write(&source, b"tampered").is_err());
        assert!(std::fs::remove_file(&source).is_err());
        assert!(
            std::fs::rename(directory.path(), directory.path().with_extension("moved")).is_err()
        );
        assert_eq!(std::fs::read(&verified.path).unwrap(), b"verified package");
        drop(verified);
        std::fs::write(&source, b"replacement").unwrap();
        assert!(open(&source, &expected()).is_err());
    }
}
