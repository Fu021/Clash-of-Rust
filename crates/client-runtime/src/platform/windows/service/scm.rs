use super::{
    NAME, STOP, runtime,
    security::{self, Descriptor, wide},
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::atomic::{AtomicPtr, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS,
        ERROR_SERVICE_NOT_ACTIVE, GetLastError,
    },
    Security::DACL_SECURITY_INFORMATION,
    System::Services::*,
};

struct Service(SC_HANDLE);
impl Service {
    fn new(raw: SC_HANDLE) -> Result<Self> {
        ensure!(
            !raw.is_null(),
            "Windows 服务操作失败：{}",
            std::io::Error::last_os_error()
        );
        Ok(Self(raw))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

fn manager(access: u32) -> Result<Service> {
    Service::new(unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) })
}
fn open(access: u32) -> Result<Service> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    Service::new(unsafe { OpenServiceW(manager.0, wide(NAME).as_ptr(), access) })
}
fn state(service: &Service) -> Result<u32> {
    let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    ensure!(
        unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                std::mem::size_of_val(&status) as u32,
                &mut needed,
            )
        } != 0,
        "无法读取网络助手状态"
    );
    Ok(status.dwCurrentState)
}

pub fn start() -> Result<()> {
    let service = open(SERVICE_START | SERVICE_QUERY_STATUS)?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match state(&service)? {
            SERVICE_RUNNING => return Ok(()),
            SERVICE_STOPPED => {
                if unsafe { StartServiceW(service.0, 0, std::ptr::null()) } == 0 {
                    ensure!(
                        unsafe { GetLastError() } == ERROR_SERVICE_ALREADY_RUNNING,
                        "无法启动 Windows 网络助手：{}",
                        std::io::Error::last_os_error()
                    );
                }
            }
            SERVICE_START_PENDING | SERVICE_STOP_PENDING => {}
            other => bail!("网络助手状态异常：{other}"),
        }
        ensure!(Instant::now() < deadline, "Windows 网络助手启动超时");
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn exists() -> bool {
    open(SERVICE_QUERY_STATUS).is_ok()
}
pub fn server_pid() -> Result<u32> {
    let service = open(SERVICE_QUERY_STATUS)?;
    let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    ensure!(
        unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                std::mem::size_of_val(&status) as u32,
                &mut needed,
            )
        } != 0
            && status.dwCurrentState == SERVICE_RUNNING
            && status.dwProcessId != 0,
        "网络助手身份尚未确认"
    );
    Ok(status.dwProcessId)
}

pub fn stop() -> Result<()> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let raw = unsafe {
        OpenServiceW(
            manager.0,
            wide(NAME).as_ptr(),
            SERVICE_STOP | SERVICE_QUERY_STATUS,
        )
    };
    if raw.is_null() && unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST {
        return Ok(());
    }
    let service = Service::new(raw)?;
    if state(&service)? == SERVICE_STOPPED {
        return Ok(());
    }
    let mut status = unsafe { std::mem::zeroed() };
    if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) } == 0 {
        ensure!(
            unsafe { GetLastError() } == ERROR_SERVICE_NOT_ACTIVE,
            "无法停止旧网络助手"
        );
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while state(&service)? != SERVICE_STOPPED {
        ensure!(
            Instant::now() < deadline,
            "旧网络助手未退出，请关闭 TUN 后重试"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

struct Replacement {
    root: std::path::PathBuf,
    backup: tempfile::TempDir,
    files: Vec<(&'static str, bool)>,
    committed: bool,
}
impl Drop for Replacement {
    fn drop(&mut self) {
        if !self.committed {
            for (name, existed) in self.files.iter().rev() {
                let _ = std::fs::remove_file(self.root.join(name));
                if *existed {
                    let _ = std::fs::rename(self.backup.path().join(name), self.root.join(name));
                }
            }
        }
    }
}

pub fn install() -> Result<()> {
    ensure!(
        security::administrator()?,
        "安装 Windows 网络助手需要管理员授权"
    );
    let source = std::env::current_exe()?;
    let bundle = source.parent().context("无法定位安装资源")?;
    let root = security::installed_directory()?;
    // This path is inside the real Known Folder, never an environment override.
    std::fs::create_dir_all(&root)?;
    ensure!(
        std::os::windows::fs::MetadataExt::file_attributes(&std::fs::symlink_metadata(&root)?)
            & 0x400
            == 0,
        "网络助手目录不能是链接"
    );
    security::protect(&root, true)?;
    let helper = root.join("clash-tun-launcher.exe");
    ensure!(source != helper, "请从客户端安装目录运行助手安装或修复");
    let staged = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(&root)?;
    security::protect(staged.path(), false)?;
    for (from, name) in [
        (source.clone(), "clash-tun-launcher.exe"),
        (bundle.join("resources/mihomo.exe"), "mihomo.exe"),
        (bundle.join("resources/core.json"), "core.json"),
    ] {
        ensure!(from.is_file(), "网络助手安装资源缺失：{}", from.display());
        let to = staged.path().join(name);
        std::fs::copy(from, &to)?;
        security::protect(&to, true)?;
    }
    drop(runtime::verify_core(staged.path())?);
    // Verify every new byte before stopping or replacing the old installation.
    stop()?;
    let backup = tempfile::Builder::new()
        .prefix("backup-")
        .tempdir_in(&root)?;
    security::protect(backup.path(), false)?;
    let mut replacement = Replacement {
        root: root.clone(),
        backup,
        files: Vec::new(),
        committed: false,
    };
    for name in ["clash-tun-launcher.exe", "mihomo.exe", "core.json"] {
        let to = root.join(name);
        let existed = to.exists();
        let deadline = Instant::now() + Duration::from_secs(10);
        if existed {
            loop {
                match std::fs::rename(&to, replacement.backup.path().join(name)) {
                    Ok(()) => break,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::PermissionDenied
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    Err(error) => return Err(error).context("无法退出旧网络助手"),
                }
            }
        }
        replacement.files.push((name, existed));
        std::fs::rename(staged.path().join(name), to)?;
    }
    let manager = manager(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
    let binary = wide(format!("\"{}\" --windows-service", helper.display()));
    let mut raw = unsafe {
        CreateServiceW(
            manager.0,
            wide(NAME).as_ptr(),
            wide("Clash of Rust Network").as_ptr(),
            SERVICE_ALL_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            binary.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if raw.is_null() && unsafe { GetLastError() } == ERROR_SERVICE_EXISTS {
        raw = unsafe { OpenServiceW(manager.0, wide(NAME).as_ptr(), SERVICE_ALL_ACCESS) };
        ensure!(!raw.is_null(), "无法更新网络助手服务");
        if unsafe {
            ChangeServiceConfigW(
                raw,
                SERVICE_NO_CHANGE,
                SERVICE_DEMAND_START,
                SERVICE_NO_CHANGE,
                binary.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            unsafe {
                CloseServiceHandle(raw);
            }
            bail!("无法更新网络助手启动配置");
        }
    }
    let service = Service::new(raw)?;
    // Interactive users can query/start it, but cannot stop, reconfigure or
    // replace the service. Every core lease is independently authenticated.
    let descriptor = Descriptor::new("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;CCLCRP;;;IU)")?;
    ensure!(
        unsafe { SetServiceObjectSecurity(service.0, DACL_SECURITY_INFORMATION, descriptor.raw()) }
            != 0,
        "无法设置网络助手访问权限"
    );
    replacement.committed = true;
    Ok(())
}

pub fn uninstall() -> Result<()> {
    ensure!(
        security::administrator()?,
        "移除 Windows 网络助手需要管理员授权"
    );
    stop()?;
    let manager = manager(SC_MANAGER_CONNECT)?;
    let raw = unsafe { OpenServiceW(manager.0, wide(NAME).as_ptr(), 0x10000) };
    if !raw.is_null() {
        let service = Service::new(raw)?;
        ensure!(
            unsafe { DeleteService(service.0) } != 0,
            "无法删除网络助手服务"
        );
    } else {
        ensure!(
            unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST,
            "无法访问网络助手服务"
        );
    }
    let root = security::installed_directory()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while root.exists() {
        match std::fs::remove_dir_all(&root) {
            Ok(()) => break,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error).context("无法清理网络助手文件"),
        }
    }
    Ok(())
}

static STATUS: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
fn report(state: u32, code: u32) {
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: if state == SERVICE_START_PENDING {
            20_000
        } else {
            0
        },
    };
    unsafe {
        SetServiceStatus(STATUS.load(Ordering::SeqCst), &status);
    }
}
unsafe extern "system" fn control(
    code: u32,
    _: u32,
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> u32 {
    if matches!(code, SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN) {
        STOP.store(true, Ordering::SeqCst);
    }
    0
}
unsafe extern "system" fn service_main(_: u32, _: *mut *mut u16) {
    let handle = unsafe {
        RegisterServiceCtrlHandlerExW(wide(NAME).as_ptr(), Some(control), std::ptr::null())
    };
    if handle.is_null() {
        return;
    }
    STATUS.store(handle, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0);
    let result = std::panic::catch_unwind(|| {
        ensure!(
            security::current_user()? == "S-1-5-18",
            "网络助手服务必须由系统管理器启动"
        );
        let root = security::installed_directory()?;
        ensure!(
            std::env::current_exe()?.canonicalize()?
                == root.join("clash-tun-launcher.exe").canonicalize()?,
            "助手安装路径无效"
        );
        runtime::verify_core(&root)?;
        report(SERVICE_RUNNING, 0);
        super::serve()
    });
    report(
        SERVICE_STOPPED,
        if matches!(result, Ok(Ok(()))) {
            0
        } else {
            1066
        },
    );
}
pub fn dispatch() -> Result<()> {
    let mut name = wide(NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    ensure!(
        unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } != 0,
        "无法接入 Windows 服务管理器：{}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
