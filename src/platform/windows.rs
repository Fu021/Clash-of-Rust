use super::*;
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};

const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
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
        } else if let Some(address) = fields.get("socks") {
            format!("socks5h://{address}")
        } else {
            return None;
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
fn startup_command() -> Result<String> {
    Ok(format!(
        "\"{}\" --background",
        std::env::current_exe()?.display()
    ))
}
pub fn autostart_enabled() -> Result<bool> {
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(RUN_KEY) {
        Ok(key) => key,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    Ok(key.get_value::<String, _>("ClashOfRust").ok().as_deref()
        == Some(startup_command()?.as_str()))
}
pub fn set_autostart(enabled: bool) -> Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        key.set_value("ClashOfRust", &startup_command()?)?;
    } else {
        match key.delete_value("ClashOfRust") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn open_directory(path: &Path) -> Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()?;
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
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let handle = unsafe {
        CreateEventW(
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
            0,
            0,
            wide(name).as_ptr(),
        )
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
    let parameters = wide(&format!("--elevated-tun \"{}\"", path.display()));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("runas").as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            1,
        )
    };
    if result <= 32 {
        bail!("提权未完成（UAC 取消或拒绝，错误 {result}），当前程序继续运行");
    }
    Ok(())
}

pub fn shutdown_existing(executable: &Path) -> Result<()> {
    if executable
        .file_name()
        .is_none_or(|name| name != "clash-of-rust.exe")
    {
        bail!("安装程序关闭目标无效");
    }
    let event = unsafe { OpenEventW(2, 0, wide("Local\\ClashOfRust.Exit").as_ptr()) };
    if !event.is_null() {
        unsafe {
            SetEvent(event);
            CloseHandle(event);
        }
        // Let the GUI restore system proxy settings and stop its own core first.
        for _ in 0..60 {
            if !desktop_running() {
                return restore_after_shutdown();
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    // Older clients have no shutdown event. Only terminate this installation's
    // executable and its bundled core; other mihomo installations are untouched.
    let script = r#"$ErrorActionPreference='Stop'
$target=[IO.Path]::GetFullPath($env:CLASH_SHUTDOWN_EXE)
$core=Join-Path ([IO.Path]::GetDirectoryName($target)) 'resources\mihomo.exe'
$apps=@(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $target })
foreach ($app in $apps) {
  Get-CimInstance Win32_Process | Where-Object { $_.ParentProcessId -eq $app.ProcessId -and $_.ExecutablePath -eq $core } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  Stop-Process -Id $app.ProcessId -Force -ErrorAction SilentlyContinue
}"#;
    use std::os::windows::process::CommandExt;
    let result = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CLASH_SHUTDOWN_EXE", executable)
        .creation_flags(0x08000000)
        .status()?;
    if !result.success() {
        bail!("无法关闭当前安装的客户端");
    }
    for _ in 0..20 {
        if !desktop_running() {
            return restore_after_shutdown();
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    bail!("客户端仍在退出，安装尚未开始")
}
fn restore_after_shutdown() -> Result<()> {
    let store = crate::config::Store::discover()?;
    super::restore(&store.root.join("system-proxy.json"))?;
    Ok(())
}
fn desktop_running() -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    }
    let handle = unsafe { OpenMutexW(0x100000, 0, wide("Local\\ClashOfRust.Desktop").as_ptr()) };
    if handle.is_null() {
        false
    } else {
        unsafe {
            CloseHandle(handle);
        }
        true
    }
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
