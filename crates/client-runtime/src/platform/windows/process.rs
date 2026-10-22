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
    System::Threading::*,
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
