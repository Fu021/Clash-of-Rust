//! A detached console must never inherit the caller's result pipes or locks.
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::ExitStatusExt,
    },
    path::Path,
    process::ExitStatus,
};
use windows_sys::Win32::{
    Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::{
        GetTokenInformation, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION,
        TOKEN_LINKED_TOKEN, TOKEN_QUERY, TokenElevation, TokenLinkedToken,
    },
    System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
    System::Threading::*,
    UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId},
};

pub struct BackgroundChild {
    handle: OwnedHandle,
}
impl BackgroundChild {
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.handle.as_raw_handle(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.handle.as_raw_handle(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}
fn inheritable(file: &File) -> io::Result<OwnedHandle> {
    let mut raw = std::ptr::null_mut();
    let current = unsafe { GetCurrentProcess() };
    if unsafe {
        DuplicateHandle(
            current,
            file.as_raw_handle(),
            current,
            &mut raw,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}
fn argument(line: &mut Vec<u16>, value: &OsStr) -> Result<()> {
    if !line.is_empty() {
        line.push(b' ' as u16);
    }
    line.push(b'"' as u16);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        ensure!(unit != 0, "进程参数包含空字符");
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    line.push(b'"' as u16);
    Ok(())
}
struct Attributes(Vec<usize>);
impl Attributes {
    fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.pointer()) };
    }
}

fn process_token(process: HANDLE) -> Result<OwnedHandle> {
    let mut token = std::ptr::null_mut();
    ensure!(
        unsafe {
            OpenProcessToken(
                process,
                TOKEN_QUERY | TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY,
                &mut token,
            )
        } != 0,
        "无法读取桌面用户身份：{}",
        io::Error::last_os_error()
    );
    Ok(unsafe { OwnedHandle::from_raw_handle(token) })
}

fn ordinary_token() -> Result<OwnedHandle> {
    let shell = unsafe { GetShellWindow() };
    let token = if shell.is_null() {
        let current = process_token(unsafe { GetCurrentProcess() })?;
        let mut linked: TOKEN_LINKED_TOKEN = unsafe { std::mem::zeroed() };
        let mut count = 0;
        ensure!(
            unsafe {
                GetTokenInformation(
                    current.as_raw_handle(),
                    TokenLinkedToken,
                    (&mut linked as *mut TOKEN_LINKED_TOKEN).cast(),
                    std::mem::size_of_val(&linked) as u32,
                    &mut count,
                )
            } != 0,
            "无法取得普通用户身份，请从开始菜单启动客户端"
        );
        unsafe { OwnedHandle::from_raw_handle(linked.LinkedToken) }
    } else {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(shell, &mut pid) };
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        ensure!(
            !process.is_null(),
            "无法读取当前桌面用户，请从开始菜单启动客户端"
        );
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        process_token(process.as_raw_handle())?
    };
    let mut elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut count = 0;
    ensure!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of_val(&elevation) as u32,
                &mut count,
            )
        } != 0
            && elevation.TokenIsElevated == 0,
        "没有可用的普通用户桌面身份，请从开始菜单启动客户端"
    );
    Ok(token)
}

struct UserEnvironment(*mut std::ffi::c_void);
impl Drop for UserEnvironment {
    fn drop(&mut self) {
        unsafe { DestroyEnvironmentBlock(self.0) };
    }
}

fn user_environment(token: HANDLE, data_dir: Option<&Path>) -> Result<Vec<u16>> {
    let mut environment = std::ptr::null_mut();
    ensure!(
        unsafe { CreateEnvironmentBlock(&mut environment, token, 0) } != 0,
        "无法准备桌面用户环境：{}",
        io::Error::last_os_error()
    );
    let environment = UserEnvironment(environment);
    let pointer = environment.0.cast::<u16>();
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < 1024 * 1024 && unsafe { *pointer.add(offset) } != 0 {
        let start = offset;
        while offset < 1024 * 1024 && unsafe { *pointer.add(offset) } != 0 {
            offset += 1;
        }
        ensure!(offset < 1024 * 1024, "桌面用户环境过大");
        let entry = unsafe { std::slice::from_raw_parts(pointer.add(start), offset - start) };
        let name = String::from_utf16_lossy(entry);
        if !(data_dir.is_some()
            && name
                .split_once('=')
                .is_some_and(|(key, _)| key.eq_ignore_ascii_case("CLASH_OF_RUST_DATA_DIR")))
        {
            entries.push(entry.to_vec());
        }
        offset += 1;
    }
    ensure!(offset < 1024 * 1024, "桌面用户环境过大");
    if let Some(data_dir) = data_dir {
        ensure!(data_dir.is_absolute(), "更新数据目录无效");
        let mut entry: Vec<_> = "CLASH_OF_RUST_DATA_DIR=".encode_utf16().collect();
        entry.extend(data_dir.as_os_str().encode_wide());
        ensure!(!entry.contains(&0), "更新数据目录包含空字符");
        entries.push(entry);
    }
    entries.sort_by_cached_key(|entry| String::from_utf16_lossy(entry).to_lowercase());
    let mut block = Vec::new();
    for entry in entries {
        block.extend(entry);
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

pub fn spawn_desktop(
    executable: &Path,
    arguments: &[OsString],
    data_dir: Option<&Path>,
) -> Result<BackgroundChild> {
    let elevated = super::is_elevated();
    let token = if elevated {
        ordinary_token()?
    } else {
        process_token(unsafe { GetCurrentProcess() })?
    };
    let environment = user_environment(token.as_raw_handle(), data_dir)?;
    let mut line = Vec::new();
    argument(&mut line, executable.as_os_str())?;
    for value in arguments {
        argument(&mut line, value)?;
    }
    line.push(0);
    let application: Vec<_> = executable.as_os_str().encode_wide().chain([0]).collect();
    let directory = executable.parent().context("客户端安装目录无效")?;
    let directory: Vec<_> = directory.as_os_str().encode_wide().chain([0]).collect();
    let mut desktop: Vec<_> = "winsta0\\default".encode_utf16().chain([0]).collect();
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of_val(&startup) as u32;
    startup.lpDesktop = desktop.as_mut_ptr();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let result = unsafe {
        if elevated {
            CreateProcessWithTokenW(
                token.as_raw_handle(),
                LOGON_WITH_PROFILE,
                application.as_ptr(),
                line.as_mut_ptr(),
                CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup,
                &mut info,
            )
        } else {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
                environment.as_ptr().cast(),
                directory.as_ptr(),
                &startup,
                &mut info,
            )
        }
    };
    if result == 0 {
        return Err(io::Error::last_os_error()).context("无法以普通用户启动客户端");
    }
    let _thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    Ok(BackgroundChild {
        handle: unsafe { OwnedHandle::from_raw_handle(info.hProcess) },
    })
}

pub fn spawn(executable: &Path, arguments: &[OsString], stderr: &File) -> Result<BackgroundChild> {
    let null = File::options().read(true).write(true).open("NUL")?;
    let null = inheritable(&null)?;
    let stderr = inheritable(stderr)?;
    let mut inherited: [HANDLE; 2] = [null.as_raw_handle(), stderr.as_raw_handle()];
    let mut size = 0;
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
    ensure!(size > 0, "无法计算进程句柄列表大小");
    let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe { InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 1, 0, &mut size) }
            != 0,
        "无法初始化进程句柄列表：{}",
        io::Error::last_os_error()
    );
    let mut attributes = Attributes(storage);
    ensure!(
        unsafe {
            UpdateProcThreadAttribute(
                attributes.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                inherited.as_mut_ptr().cast(),
                std::mem::size_of_val(&inherited),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } != 0,
        "无法限制后台句柄继承：{}",
        io::Error::last_os_error()
    );
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = inherited[0];
    startup.StartupInfo.hStdOutput = inherited[0];
    startup.StartupInfo.hStdError = inherited[1];
    startup.lpAttributeList = attributes.pointer();
    let mut line = Vec::new();
    argument(&mut line, executable.as_os_str())?;
    for value in arguments {
        argument(&mut line, value)?;
    }
    line.push(0);
    let application: Vec<_> = executable.as_os_str().encode_wide().chain([0]).collect();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut info,
        )
    } == 0
    {
        return Err(io::Error::last_os_error()).context("无法启动无窗口后台");
    }
    let _thread = unsafe { OwnedHandle::from_raw_handle(info.hThread) };
    Ok(BackgroundChild {
        handle: unsafe { OwnedHandle::from_raw_handle(info.hProcess) },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::ffi::OsStringExt;

    #[test]
    fn desktop_environment_uses_only_the_explicit_data_directory() {
        let token = process_token(unsafe { GetCurrentProcess() }).unwrap();
        let path = std::env::current_dir().unwrap().join("用户 数据");
        let block = user_environment(token.as_raw_handle(), Some(&path)).unwrap();
        let entries: Vec<_> = block
            .split(|&unit| unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(String::from_utf16_lossy)
            .collect();
        let keys: Vec<_> = entries
            .iter()
            .filter(|entry| entry.starts_with("CLASH_OF_RUST_DATA_DIR="))
            .cloned()
            .collect();
        assert_eq!(keys, [format!("CLASH_OF_RUST_DATA_DIR={}", path.display())]);
        assert!(user_environment(token.as_raw_handle(), Some(Path::new("relative"))).is_err());
    }

    #[test]
    fn ordinary_desktop_probe() {
        if let Some(root) = std::env::args().find_map(|arg| {
            arg.strip_prefix("desktop-probe:")
                .map(std::path::PathBuf::from)
        }) {
            assert!(!super::super::is_elevated());
            let data = std::env::var_os("CLASH_OF_RUST_DATA_DIR").unwrap();
            assert_eq!(std::path::PathBuf::from(data), root.join("profile"));
            std::fs::create_dir(&root).unwrap();
            let store = crate::config::Store::at(root.join("profile")).unwrap();
            let mut settings = store.load_settings().unwrap();
            settings.update_prereleases = true;
            store.save_settings(&settings).unwrap();
            assert!(store.load_settings().unwrap().update_prereleases);
            let profile = crate::config::Profile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "ordinary-user fixture".into(),
                source: "https://example.test/subscription".into(),
                updated: 0,
                usage: None,
            };
            crate::config::atomic_write(
                &store.profile_path(&profile.id).unwrap(),
                b"proxies: []\n",
            )
            .unwrap();
            store.save_profiles(&[profile]).unwrap();
            assert_eq!(store.profiles().unwrap().len(), 1);
            for relative in [
                "runtime/config.yaml",
                "runtime/GeoIP.dat",
                "window-state.json",
                "diagnostics/report.json",
                "logs/export.log",
            ] {
                let path = store.root.join(relative);
                crate::config::atomic_write(&path, b"original").unwrap();
                let reader = std::fs::File::open(&path).unwrap();
                crate::config::atomic_write(&path, b"updated").unwrap();
                drop(reader);
                assert_eq!(std::fs::read(&path).unwrap(), b"updated");
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(store.root.join("daemon.log"))
                .unwrap();
            let temporary = tempfile::tempdir().unwrap();
            let package = temporary.path().join("update.exe");
            std::fs::write(&package, b"verified update fixture").unwrap();
            std::fs::copy(&package, temporary.path().join("helper.exe")).unwrap();
            std::fs::write(root.join("success"), b"ordinary-user storage verified").unwrap();
        }
    }

    #[test]
    fn installer_launch_creates_writable_state_with_an_ordinary_identity() {
        let current = process_token(unsafe { GetCurrentProcess() }).unwrap();
        let token = if super::super::is_elevated() {
            match ordinary_token() {
                Ok(token) => token,
                Err(_) => {
                    assert!(spawn_desktop(&std::env::current_exe().unwrap(), &[], None).is_err());
                    eprintln!("No ordinary desktop identity: elevated GUI launch refused");
                    return;
                }
            }
        } else {
            current
        };
        let environment = user_environment(token.as_raw_handle(), None).unwrap();
        let temporary = environment
            .split(|&unit| unit == 0)
            .find_map(|entry| {
                let entry = String::from_utf16_lossy(entry);
                let (key, value) = entry.split_once('=')?;
                key.eq_ignore_ascii_case("TEMP")
                    .then(|| std::path::PathBuf::from(value))
            })
            .unwrap();
        let root = temporary.join(format!("cor-desktop-launch-{}", uuid::Uuid::new_v4()));
        assert!(!root.exists());
        let arguments = [
            "--exact".into(),
            "platform::windows_process::tests::ordinary_desktop_probe".into(),
            "--nocapture".into(),
            "--skip".into(),
            format!("desktop-probe:{}", root.display()).into(),
        ];
        let mut child = spawn_desktop(
            &std::env::current_exe().unwrap(),
            &arguments,
            Some(&root.join("profile")),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                unsafe {
                    TerminateProcess(child.handle.as_raw_handle(), 1);
                    WaitForSingleObject(child.handle.as_raw_handle(), 5000);
                }
                panic!("ordinary desktop probe did not exit");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(root.join("success").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_inheritance_probe() {
        if let Some(value) =
            std::env::args().find_map(|arg| arg.strip_prefix("probe-handle:").map(str::to_owned))
        {
            let raw = value.parse::<usize>().unwrap() as HANDLE;
            unsafe { SetEvent(raw) };
            eprintln!("inheritance probe executed");
        }
    }
    #[test]
    fn detached_console_excludes_inheritable_parent_handles() {
        let attributes = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>()
                as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let raw = unsafe { CreateEventW(&attributes, 1, 0, std::ptr::null()) };
        assert!(!raw.is_null());
        let event = unsafe { OwnedHandle::from_raw_handle(raw) };
        let log = tempfile::NamedTempFile::new().unwrap();
        let arguments = [
            "--exact".into(),
            "platform::windows_process::tests::child_inheritance_probe".into(),
            "--nocapture".into(),
            "--skip".into(),
            format!("probe-handle:{}", raw as usize).into(),
        ];
        let mut child =
            spawn(&std::env::current_exe().unwrap(), &arguments, log.as_file()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "handle inheritance probe did not exit"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            unsafe { WaitForSingleObject(event.as_raw_handle(), 0) },
            WAIT_TIMEOUT
        );
        assert!(
            std::fs::read_to_string(log.path())
                .unwrap()
                .contains("inheritance probe executed")
        );
    }
    #[test]
    fn quoted_arguments_preserve_unicode_quotes_and_trailing_backslashes() {
        let values = [
            OsString::from(r"C:\Program Files\cor.exe"),
            OsString::from("节点 名称"),
            OsString::from("quote\"slash\\\""),
            OsString::from(r"C:\data dir\\"),
            OsString::from(""),
        ];
        let mut line = Vec::new();
        for value in &values {
            argument(&mut line, value).unwrap();
        }
        line.push(0);
        let mut count = 0;
        let argv =
            unsafe { windows_sys::Win32::UI::Shell::CommandLineToArgvW(line.as_ptr(), &mut count) };
        assert!(!argv.is_null());
        let result: Vec<_> = (0..count as usize)
            .map(|i| unsafe {
                let pointer = *argv.add(i);
                let mut end = 0;
                while *pointer.add(end) != 0 {
                    end += 1;
                }
                OsString::from_wide(std::slice::from_raw_parts(pointer, end))
            })
            .collect();
        unsafe { windows_sys::Win32::Foundation::LocalFree(argv.cast()) };
        assert_eq!(result, values);
    }
}
