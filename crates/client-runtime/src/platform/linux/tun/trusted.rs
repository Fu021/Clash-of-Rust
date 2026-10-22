//! Fixed installation paths and descriptor-based validation shared with the launcher.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::CStr,
    fs::File,
    os::unix::{
        fs::MetadataExt,
        io::{AsRawFd, FromRawFd},
    },
};

pub(crate) const CORE: &str = "/opt/clash-of-rust/resources/mihomo";
pub(crate) const LAUNCHER: &str = "/opt/clash-of-rust/clash-tun-launcher";
pub(crate) const DNS_HELPER: &str = "/opt/clash-of-rust/libexec/resolvectl";
pub(crate) const NETWORK_CAPABILITIES: u64 = (1 << 12) | (1 << 13);

pub(crate) fn trusted_metadata(metadata: &std::fs::Metadata, directory: bool) -> Result<()> {
    ensure!(
        metadata.uid() == 0 && metadata.mode() & 0o022 == 0,
        "安装文件必须由 root 所有，且其他用户不可写，请重新安装 DEB"
    );
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file() && metadata.nlink() == 1
        },
        "安装文件类型或硬链接数量无效，请重新安装 DEB"
    );
    Ok(())
}

pub(crate) fn open_at(parent: &File, name: &CStr, directory: bool) -> Result<File> {
    let flags = libc::O_RDONLY
        | libc::O_NOFOLLOW
        | libc::O_CLOEXEC
        | if directory { libc::O_DIRECTORY } else { 0 };
    let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("无法安全打开安装文件");
    }
    let file = unsafe { File::from_raw_fd(fd) };
    trusted_metadata(&file.metadata()?, directory)?;
    Ok(file)
}

fn installed_app() -> Result<File> {
    let root = File::open("/")?;
    trusted_metadata(&root.metadata()?, true)?;
    let opt = open_at(&root, c"opt", true)?;
    let app = open_at(&opt, c"clash-of-rust", true)?;
    // Validate the fixed executable as well as the target, without following links.
    let _executable = open_at(&app, c"clash-of-rust", false)?;
    let _launcher = open_at(&app, c"clash-tun-launcher", false)?;
    Ok(app)
}

pub(crate) fn installed_dns_helper() -> Result<File> {
    let app = installed_app()?;
    let helpers = open_at(&app, c"libexec", true)?;
    let name = std::ffi::CString::new(
        std::path::Path::new(DNS_HELPER)
            .file_name()
            .unwrap()
            .as_encoded_bytes(),
    )?;
    open_at(&helpers, &name, false)
}

pub(crate) fn installed_resources() -> Result<File> {
    let app = installed_app()?;
    let _dns = installed_dns_helper()?;
    open_at(&app, c"resources", true)
}
