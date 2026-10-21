//! SYSTEM-owned runtime snapshots. User files are opened under impersonation;
//! the privileged core never uses a user-writable working directory or loader path.
use super::{
    Request,
    security::{self, Handle, wide},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs::File,
    io::Read,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    os::windows::io::FromRawHandle,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation},
    Security::{
        ImpersonateLoggedOnUser, RevertToSelf, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_QUERY,
    },
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::{CreatePipe, ImpersonateNamedPipeClient},
        SystemInformation::GetWindowsDirectoryW,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            GetCurrentThread, GetExitCodeProcess, OpenThreadToken, PROCESS_INFORMATION,
            ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess,
            WaitForSingleObject,
        },
    },
};

const FILE_LIMIT: u64 = 128 * 1024 * 1024;
const TOTAL_LIMIT: u64 = 512 * 1024 * 1024;
pub struct Impersonation;
impl Impersonation {
    pub fn pipe(pipe: HANDLE) -> Result<Self> {
        ensure!(
            unsafe { ImpersonateNamedPipeClient(pipe) } != 0,
            "无法确认调用用户的文件权限"
        );
        Ok(Self)
    }
    pub fn token(token: &Handle) -> Result<Self> {
        ensure!(
            unsafe { ImpersonateLoggedOnUser(token.raw()) } != 0,
            "无法恢复用户缓存权限"
        );
        Ok(Self)
    }
}
impl Drop for Impersonation {
    fn drop(&mut self) {
        unsafe {
            RevertToSelf();
        }
    }
}

pub struct Snapshot {
    pub directory: tempfile::TempDir,
    original: PathBuf,
    token: Handle,
}

fn collect(
    root: &Path,
    relative: &Path,
    files: &mut Vec<(PathBuf, File, u64)>,
    total: &mut u64,
    visited: &mut usize,
) -> Result<()> {
    ensure!(relative.components().count() <= 16, "内核运行目录嵌套过深");
    for entry in std::fs::read_dir(root.join(relative))? {
        *visited += 1;
        ensure!(*visited <= 4096, "内核运行目录项目过多");
        let entry = entry?;
        let child = relative.join(entry.file_name());
        if child.starts_with("temp") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "内核运行目录不能包含链接"
        );
        if metadata.is_dir() {
            collect(root, &child, files, total, visited)?;
            continue;
        }
        ensure!(
            metadata.is_file() && metadata.len() <= FILE_LIMIT && files.len() < 4096,
            "内核运行数据超出限制"
        );
        let extension = child
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        ensure!(
            !matches!(
                extension.as_str(),
                "exe" | "dll" | "com" | "bat" | "cmd" | "ps1" | "vbs" | "lnk"
            ),
            "运行目录中不能放置可执行程序"
        );
        let file = File::open(entry.path())?;
        let size = file.metadata()?.len();
        *total = total.checked_add(size).context("内核文件大小无效")?;
        ensure!(*total <= TOTAL_LIMIT, "内核运行数据超过 512 MiB");
        files.push((child, file, size));
    }
    Ok(())
}

pub fn snapshot(pipe: HANDLE, request: &Request) -> Result<Snapshot> {
    let root = security::installed_directory()?.join("runtime");
    std::fs::create_dir_all(&root)?;
    security::protect(&root, false)?;
    let directory = tempfile::Builder::new().prefix("core-").tempdir_in(&root)?;
    let impersonation = Impersonation::pipe(pipe)?;
    let mut token = std::ptr::null_mut();
    ensure!(
        unsafe {
            OpenThreadToken(
                GetCurrentThread(),
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_IMPERSONATE,
                1,
                &mut token,
            )
        } != 0,
        "无法保留用户文件权限"
    );
    let token = Handle::new(token)?;
    let original = request.directory.canonicalize()?;
    ensure!(
        original.is_dir() && request.config.canonicalize()? == original.join("config.yaml"),
        "内核配置路径无效"
    );
    let mut files = Vec::new();
    collect(&original, Path::new(""), &mut files, &mut 0, &mut 0)?;
    drop(impersonation);
    for (relative, source, size) in files {
        let destination = directory.path().join(relative);
        std::fs::create_dir_all(destination.parent().context("运行文件路径无效")?)?;
        let mut destination = File::create(destination)?;
        let count = std::io::copy(&mut source.take(size + 1), &mut destination)?;
        ensure!(count == size, "复制时内核数据发生变化，请重试");
    }
    let config_file = directory.path().join("config.yaml");
    ensure!(
        std::fs::metadata(&config_file)?.len() <= 32 * 1024 * 1024,
        "内核配置过大"
    );
    let bytes = std::fs::read(config_file)?;
    ensure!(bytes.len() <= 32 * 1024 * 1024, "内核配置过大");
    let mut config: serde_yaml::Mapping = serde_yaml::from_slice(&bytes)?;
    // Safe paths remain enabled. The service only exposes the app's loopback
    // controller; a supplied configuration cannot create a privileged pipe.
    for key in [
        "external-ui",
        "external-ui-url",
        "external-controller-unix",
        "external-controller-pipe",
        "external-controller-tls",
        "listeners",
    ] {
        config.remove(serde_yaml::Value::from(key));
    }
    let controller = config
        .get(serde_yaml::Value::from("external-controller"))
        .and_then(serde_yaml::Value::as_str)
        .context("控制接口缺失")?;
    ensure!(
        controller.starts_with("127.0.0.1:")
            && controller[10..].parse::<u16>().is_ok_and(|p| p != 0),
        "服务仅支持本机控制接口"
    );
    ensure!(
        config
            .get(serde_yaml::Value::from("secret"))
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|s| !s.is_empty()),
        "控制接口必须设置密钥"
    );
    std::fs::write(
        directory.path().join("config.yaml"),
        serde_yaml::to_string(&config)?,
    )?;
    std::fs::create_dir(directory.path().join("temp"))?;
    Ok(Snapshot {
        directory,
        original,
        token,
    })
}

impl Snapshot {
    pub fn save_cache(&self) -> Result<()> {
        let mut files = Vec::new();
        collect(
            self.directory.path(),
            Path::new(""),
            &mut files,
            &mut 0,
            &mut 0,
        )?;
        // Geo/config sources remain owned by the GUI. Mutable provider caches
        // and node selections must survive the transition back to a user core.
        files.retain(|(path, _, _)| {
            path.components().count() > 1
                || !matches!(
                    path.to_str(),
                    Some(
                        "config.yaml"
                            | "candidate.yaml"
                            | "GeoIP.dat"
                            | "GeoSite.dat"
                            | "Country.mmdb"
                            | "ASN.mmdb"
                            | "geodata.json"
                    )
                )
        });
        let _impersonation = Impersonation::token(&self.token)?;
        for (relative, source, size) in files {
            if relative.starts_with("temp") {
                continue;
            }
            let path = self.original.join(relative);
            std::fs::create_dir_all(path.parent().context("缓存路径无效")?)?;
            let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            ensure!(
                std::io::copy(&mut source.take(size + 1), &mut staged)? == size,
                "缓存数据发生变化"
            );
            staged.as_file().sync_all()?;
            staged.persist(path).map_err(|e| e.error)?;
        }
        Ok(())
    }
}

pub fn verify_core(root: &Path) -> Result<File> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("core.json"))?)?;
    let path = root.join("mihomo.exe");
    let locked = File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&path)?;
    ensure!(
        crate::file_hash::hash_file(&path)?.0
            == manifest["exe_sha256"]
                .as_str()
                .context("内核校验信息缺失")?,
        "Windows 服务内核校验失败，请修复安装"
    );
    Ok(locked)
}

pub struct Core {
    process: Handle,
    job: Handle,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
    _image_lock: File,
}
impl Core {
    pub fn start(snapshot: &Snapshot) -> Result<Self> {
        let root = security::installed_directory()?;
        let image_lock = verify_core(&root)?;
        let job = Handle::new(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        ensure!(
            unsafe {
                SetInformationJobObject(
                    job.raw(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } != 0,
            "无法创建内核生命周期隔离"
        );
        let attributes = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>()
                as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        fn output(
            attributes: &windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
        ) -> Result<(File, Handle)> {
            let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
            ensure!(
                unsafe { CreatePipe(&mut read, &mut write, attributes, 4096) } != 0,
                "无法创建内核日志通道"
            );
            let read = Handle::new(read)?;
            let write = Handle::new(write)?;
            ensure!(
                unsafe { SetHandleInformation(read.raw(), HANDLE_FLAG_INHERIT, 0) } != 0,
                "无法隔离日志通道"
            );
            use std::os::windows::io::IntoRawHandle;
            Ok((
                unsafe { File::from_raw_handle(read.0.into_raw_handle()) },
                write,
            ))
        }
        let (stdout, stdout_write) = output(&attributes)?;
        let (stderr, stderr_write) = output(&attributes)?;
        let input = Handle::new(unsafe {
            CreateFileW(
                wide("NUL").as_ptr(),
                0x80000000,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &attributes,
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        })?;
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of_val(&startup) as u32;
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = input.raw();
        startup.hStdOutput = stdout_write.raw();
        startup.hStdError = stderr_write.raw();
        let directory = snapshot.directory.path();
        let mut command = wide(format!(
            "\"{}\" -d \"{}\" -f \"{}\"",
            root.join("mihomo.exe").display(),
            directory.display(),
            directory.join("config.yaml").display()
        ));
        let mut system = vec![0u16; 32768];
        let length =
            unsafe { GetWindowsDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
        ensure!(length > 0 && length < system.len(), "无法定位系统目录");
        let system = PathBuf::from(String::from_utf16(&system[..length])?);
        let mut environment = Vec::new();
        for (key, value) in [
            ("PATH", system.join("System32")),
            ("SystemRoot", system.clone()),
            ("TEMP", directory.join("temp")),
            ("TMP", directory.join("temp")),
            ("USERPROFILE", directory.to_owned()),
            ("WINDIR", system),
        ] {
            environment.extend(wide(format!("{key}={}", value.display())));
        }
        environment.push(0);
        let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        ensure!(
            unsafe {
                CreateProcessW(
                    wide(root.join("mihomo.exe")).as_ptr(),
                    command.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                    environment.as_ptr().cast(),
                    wide(&root).as_ptr(),
                    &startup,
                    &mut information,
                )
            } != 0,
            "服务启动内核失败：{}",
            std::io::Error::last_os_error()
        );
        let process = Handle::new(information.hProcess)?;
        let thread = Handle::new(information.hThread)?;
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0
            || unsafe { ResumeThread(thread.raw()) } == u32::MAX
        {
            unsafe {
                TerminateProcess(process.raw(), 1);
            }
            bail!("无法将内核放入受控生命周期");
        }
        Ok(Self {
            process,
            job,
            stdout: Some(stdout),
            stderr: Some(stderr),
            _image_lock: image_lock,
        })
    }
    pub fn exit(&self) -> Option<i32> {
        if unsafe { WaitForSingleObject(self.process.raw(), 0) } != 0 {
            return None;
        }
        let mut code = 1;
        unsafe {
            GetExitCodeProcess(self.process.raw(), &mut code);
        }
        Some(code as i32)
    }
    pub fn stop(&self) {
        unsafe {
            TerminateJobObject(self.job.raw(), 0);
            WaitForSingleObject(self.process.raw(), 5000);
        }
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        self.stop();
    }
}
