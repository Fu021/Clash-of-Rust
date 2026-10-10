use super::*;
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};

const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

pub fn installed_executable(executable: &Path) -> Result<bool> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY};
    let key = match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\ClashOfRust",
        KEY_READ | KEY_WOW64_64KEY,
    ) {
        Ok(key) => key,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let directory: String = key.get_value("InstallLocation")?;
    Ok(
        std::fs::canonicalize(Path::new(&directory).join("clash-of-rust.exe"))?
            == std::fs::canonicalize(executable)?,
    )
}

pub struct UpdateInstaller(windows_sys::Win32::Foundation::HANDLE);
impl UpdateInstaller {
    pub fn wait(&mut self) -> Result<u32> {
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        if unsafe { WaitForSingleObject(self.0, u32::MAX) } != 0 {
            bail!("无法读取安装进度");
        }
        let mut code = 0;
        if unsafe { GetExitCodeProcess(self.0, &mut code) } == 0 {
            bail!("无法读取安装结果");
        }
        Ok(code)
    }
}
impl Drop for UpdateInstaller {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub fn launch_update_installer(package: &Path) -> Result<UpdateInstaller> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
    };
    let file: Vec<u16> = package.as_os_str().encode_wide().chain([0]).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
    let parameters: Vec<u16> = "/S /UPDATE".encode_utf16().chain([0]).collect();
    let initialized =
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = 1;
    let success = unsafe { ShellExecuteExW(&mut info) };
    let error = std::io::Error::last_os_error();
    if initialized {
        unsafe {
            CoUninitialize();
        }
    }
    if success == 0 {
        if error.raw_os_error() == Some(1223) {
            bail!("安装授权已取消");
        }
        bail!("无法启动安装程序，请检查权限或安全软件");
    }
    if info.hProcess.is_null() {
        bail!("无法跟踪安装程序");
    }
    Ok(UpdateInstaller(info.hProcess))
}
pub fn configured_proxy(scheme: &str) -> Result<Option<String>> {
    let state = read()?;
    if state.get("ProxyEnable") != Some(&Some(ProxyValue::Number(1))) {
        return Ok(None);
    }
    let Some(Some(ProxyValue::Text(server))) = state.get("ProxyServer") else {
        return Ok(None);
    };
    Ok(proxy_address(server, scheme))
}
fn proxy_address(server: &str, scheme: &str) -> Option<String> {
    let address = if server.contains('=') {
        let fields: BTreeMap<_, _> = server
            .split(';')
            .filter_map(|field| field.trim().split_once('='))
            .collect();
        if let Some(address) = fields.get(scheme).or_else(|| fields.get("http")) {
            (*address).to_owned()
        } else {
            let address = fields.get("socks")?;
            format!("socks5h://{address}")
        }
    } else {
        server.trim().to_owned()
    };
    if address.is_empty() {
        None
    } else if address.contains("://") {
        Some(address)
    } else {
        Some(format!("http://{address}"))
    }
}

#[cfg(test)]
mod subscription_proxy_tests {
    use super::proxy_address;
    #[test]
    fn follows_protocol_specific_system_proxy_and_socks() {
        assert_eq!(
            proxy_address("http=127.0.0.1:7001;https=127.0.0.1:7002", "https"),
            Some("http://127.0.0.1:7002".into())
        );
        assert_eq!(
            proxy_address("socks=127.0.0.1:7897", "https"),
            Some("socks5h://127.0.0.1:7897".into())
        );
        assert_eq!(
            proxy_address("127.0.0.1:7897", "https"),
            Some("http://127.0.0.1:7897".into())
        );
        assert!(proxy_address("", "https").is_none());
    }
}
pub fn autostart_enabled() -> Result<bool> {
    super::windows_autostart::Backend::current()?.enabled()
}
pub fn ensure_tray_available() -> Result<()> {
    let host = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
            wide("Shell_TrayWnd").as_ptr(),
            std::ptr::null(),
        )
    };
    if host.is_null() {
        bail!("桌面托盘服务尚未就绪或当前系统没有托盘");
    }
    Ok(())
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    let backend = super::windows_autostart::Backend::current()?;
    match backend.set(enabled) {
        Err(error) if !is_elevated() && super::windows_autostart::access_denied(&error) => {
            repair_autostart(enabled).context("修复旧开机启动任务失败")?;
            if backend.enabled()? != enabled {
                bail!("开机启动设置未生效，请重试");
            }
            Ok(())
        }
        result => result,
    }
}

fn repair_autostart(enabled: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            Threading::{GetExitCodeProcess, WaitForSingleObject},
        },
        UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
    };
    let executable = std::env::current_exe()?;
    let file: Vec<u16> = executable.as_os_str().encode_wide().chain([0]).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain([0]).collect();
    let sid = super::windows_autostart::user_sid()?;
    let parameters: Vec<u16> = format!(
        "{} --autostart-user {sid}",
        if enabled {
            "--autostart-enable"
        } else {
            "--autostart-remove"
        }
    )
    .encode_utf16()
    .chain([0])
    .collect();
    let initialized =
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0;
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    let launched = unsafe { ShellExecuteExW(&mut info) };
    let error = std::io::Error::last_os_error();
    if initialized {
        unsafe { CoUninitialize() };
    }
    if launched == 0 {
        if error.raw_os_error() == Some(1223) {
            bail!("开机启动修复授权已取消");
        }
        return Err(error).context("无法启动开机启动修复助手");
    }
    if info.hProcess.is_null() {
        bail!("无法跟踪开机启动修复助手");
    }
    let wait = unsafe { WaitForSingleObject(info.hProcess, 300_000) };
    let mut code = 0;
    let read = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    unsafe { CloseHandle(info.hProcess) };
    if wait != 0 {
        bail!("开机启动修复助手未完成，请稍后重试");
    }
    if read == 0 || code != 0 {
        bail!("无法修复开机启动任务，请使用当前 Windows 账号完成管理员授权");
    }
    Ok(())
}

pub fn migrate_autostart() {
    let Ok(backend) = super::windows_autostart::Backend::current() else {
        return;
    };
    if is_elevated() {
        // A previous version may have created an admin-only task. Restore the
        // user's access when an elevated TUN instance is already authorized.
        let _ = backend.repair_permissions();
    }
    if backend.legacy_enabled().unwrap_or(false) {
        // Keep the old Run value if scheduling is unavailable or denied.
        let _ = backend.set(true);
    }
}

pub fn autostart_helper_main() -> bool {
    if std::env::args_os().any(|arg| arg == "--autostart-status") {
        match autostart_enabled() {
            Ok(enabled) => std::process::exit(i32::from(!enabled)),
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(2);
            }
        }
    }
    let enabled = if std::env::args_os().any(|arg| arg == "--autostart-enable") {
        true
    } else if std::env::args_os().any(|arg| arg == "--autostart-remove") {
        false
    } else {
        return false;
    };
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "--autostart-user" {
            let expected = arguments.next();
            let actual = super::windows_autostart::user_sid();
            if !matches!((expected, actual), (Some(expected), Ok(actual)) if expected == actual.as_str())
            {
                eprintln!("开机启动修复必须使用当前 Windows 账号授权");
                std::process::exit(3);
            }
        }
    }
    if let Err(error) = set_autostart(enabled) {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
    true
}
pub fn open_directory(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()?;
    Ok(())
}

pub fn open_url(url: &str) -> Result<()> {
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(url).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        bail!("启动默认浏览器失败（系统错误 {result}）");
    }
    Ok(())
}

type Handle = *mut std::ffi::c_void;
static EXIT_EVENT: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static SHOW_EVENT: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateEventW(attributes: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
}
#[link(name = "shell32")]
unsafe extern "system" {
    fn IsUserAnAdmin() -> i32;
    fn ShellExecuteW(
        window: Handle,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}
pub fn initialize_events() -> Result<()> {
    for (target, name) in [
        (&EXIT_EVENT, "Local\\ClashOfRust.Exit"),
        (&SHOW_EVENT, "Local\\ClashOfRust.Show"),
    ] {
        let handle = if name.ends_with(".Show") {
            create_show_event(name)?
        } else {
            unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, wide(name).as_ptr()) }
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        target.store(handle, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}

fn create_show_event(name: &str) -> Result<Handle> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        System::Threading::CreateEventExW,
    };
    // A normal shortcut must be able to wake an elevated TUN instance. This
    // session-local event only reveals the window; the exit event stays private.
    let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x100002;;;IU)S:(ML;;NW;;;ME)");
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        // Interactive users are granted signal/wait rights by the DACL above.
        // Request those rights rather than CreateEventW's EVENT_ALL_ACCESS.
        CreateEventExW(&attributes, wide(name).as_ptr(), 0, 0x100002)
    };
    let error = std::io::Error::last_os_error();
    unsafe {
        LocalFree(descriptor);
    }
    if handle.is_null() {
        return Err(error.into());
    }
    Ok(handle)
}
fn requested(event: &std::sync::atomic::AtomicPtr<std::ffi::c_void>) -> bool {
    let handle = event.load(std::sync::atomic::Ordering::Acquire);
    !handle.is_null() && unsafe { WaitForSingleObject(handle, 0) } == 0
}
pub fn exit_requested() -> bool {
    requested(&EXIT_EVENT)
}
pub fn show_requested() -> bool {
    requested(&SHOW_EVENT)
}
pub fn signal_show() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, SW_RESTORE,
        SetForegroundWindow, ShowWindowAsync,
    };
    let window = unsafe {
        FindWindowW(
            std::ptr::null(),
            wide("Clash of Rust · 原生代理客户端").as_ptr(),
        )
    };
    if !window.is_null() {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(window, &mut pid);
            AllowSetForegroundWindow(pid);
            ShowWindowAsync(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
    let event = unsafe { OpenEventW(2, 0, wide("Local\\ClashOfRust.Show").as_ptr()) };
    if !event.is_null() {
        unsafe {
            SetEvent(event);
            CloseHandle(event);
        }
    }
}

#[cfg(test)]
mod show_event_tests {
    use super::*;

    #[test]
    fn wake_event_is_signalable_and_consumed_once() {
        use std::os::windows::process::CommandExt;
        if let Ok(name) = std::env::var("CLASH_TEST_WAKE_EVENT") {
            let sender = unsafe { OpenEventW(2, 0, wide(&name).as_ptr()) };
            assert!(!sender.is_null());
            unsafe {
                assert_ne!(SetEvent(sender), 0);
                CloseHandle(sender);
            }
            return;
        }
        let name = format!("Local\\ClashOfRust.Show.Test.{}", uuid::Uuid::new_v4());
        let owner = create_show_event(&name).unwrap();
        // Opening an existing event must also work for an ordinary user,
        // whose advertised DACL rights do not include EVENT_ALL_ACCESS.
        let reopened = create_show_event(&name).unwrap();
        unsafe { CloseHandle(reopened) };
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::windows::show_event_tests::wake_event_is_signalable_and_consumed_once",
            ])
            .env("CLASH_TEST_WAKE_EVENT", name)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        unsafe {
            assert_eq!(WaitForSingleObject(owner, 0), 0);
            assert_eq!(WaitForSingleObject(owner, 0), 258);
            CloseHandle(owner);
        }
    }
}
pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}
pub fn elevate(path: &Path) -> Result<()> {
    let executable = wide(&std::env::current_exe()?.to_string_lossy());
    let background = std::env::args_os().any(|arg| arg == "--background");
    let parameters = wide(&format!(
        "--elevated-tun \"{}\"{}",
        path.display(),
        if background { " --background" } else { "" }
    ));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("runas").as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            if background { 0 } else { 1 },
        )
    };
    if result <= 32 {
        bail!("提权未完成（UAC 取消或拒绝，错误 {result}），当前程序继续运行");
    }
    Ok(())
}

pub fn description() -> &'static str {
    "Windows 当前用户系统代理"
}

pub fn read() -> Result<ProxyState> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_READ)?;
    let mut state = ProxyState::new();
    for name in ["ProxyServer", "ProxyOverride", "AutoConfigURL"] {
        let value = match key.get_value::<String, _>(name) {
            Ok(v) => Some(ProxyValue::Text(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        state.insert(name.into(), value);
    }
    let enable = match key.get_value::<u32, _>("ProxyEnable") {
        Ok(v) => Some(ProxyValue::Number(v)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    state.insert("ProxyEnable".into(), enable);
    Ok(state)
}

pub fn desired(port: u16) -> ProxyState {
    BTreeMap::from([
        ("ProxyEnable".into(), Some(ProxyValue::Number(1))),
        (
            "ProxyServer".into(),
            Some(ProxyValue::Text(format!("127.0.0.1:{port}"))),
        ),
        (
            "ProxyOverride".into(),
            Some(ProxyValue::Text("localhost;127.*;[::1];<local>".into())),
        ),
        ("AutoConfigURL".into(), None),
    ])
}

pub fn write(state: &ProxyState) -> Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_WRITE)?;
    for (name, value) in state {
        match value {
            Some(ProxyValue::Text(value)) => key.set_value(name, value)?,
            Some(ProxyValue::Number(value)) => key.set_value(name, value)?,
            None => match key.delete_value(name) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            },
        }
    }
    // WinINet clients must be notified after registry writes.
    #[link(name = "wininet")]
    unsafe extern "system" {
        fn InternetSetOptionW(
            handle: *mut std::ffi::c_void,
            option: u32,
            buffer: *mut std::ffi::c_void,
            length: u32,
        ) -> i32;
    }
    unsafe {
        InternetSetOptionW(std::ptr::null_mut(), 39, std::ptr::null_mut(), 0);
        InternetSetOptionW(std::ptr::null_mut(), 37, std::ptr::null_mut(), 0);
    }
    Ok(())
}
