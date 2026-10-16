use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsStr,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            ConvertStringSidToSidW,
        },
        CheckTokenMembership, DACL_SECURITY_INFORMATION, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        SetFileSecurityW, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        Com::CoTaskMemFree,
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        Threading::{
            GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath},
};

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain([0]).collect()
}

pub struct Handle(pub OwnedHandle);
impl Handle {
    pub fn new(raw: HANDLE) -> Result<Self> {
        ensure!(
            !raw.is_null() && raw != INVALID_HANDLE_VALUE,
            "Windows 句柄无效：{}",
            std::io::Error::last_os_error()
        );
        // Ownership transfers from the successful Win32 call exactly once.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
    pub fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
}

pub struct Descriptor(*mut std::ffi::c_void);
impl Descriptor {
    pub fn new(sddl: &str) -> Result<Self> {
        let mut raw = std::ptr::null_mut();
        ensure!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide(sddl).as_ptr(),
                    1,
                    &mut raw,
                    std::ptr::null_mut(),
                )
            } != 0,
            "无法创建助手权限：{}",
            std::io::Error::last_os_error()
        );
        Ok(Self(raw))
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
    pub fn raw(&self) -> *mut std::ffi::c_void {
        self.0
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

pub fn installed_directory() -> Result<PathBuf> {
    let mut raw = std::ptr::null_mut();
    ensure!(
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, std::ptr::null_mut(), &mut raw) }
            >= 0,
        "无法定位系统应用目录"
    );
    let mut size = 0;
    unsafe {
        while *raw.add(size) != 0 {
            size += 1;
        }
    }
    let text = unsafe { String::from_utf16(std::slice::from_raw_parts(raw, size)) };
    unsafe {
        CoTaskMemFree(raw.cast());
    }
    Ok(PathBuf::from(text?).join("Clash of Rust Network"))
}

pub fn protect(path: &Path, readable: bool) -> Result<()> {
    // The owner must be Administrators, not the installing user's SID: the
    // latter could rewrite its own DACL from an unelevated process afterwards.
    let sddl = if readable {
        "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;IU)"
    } else {
        "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    };
    let descriptor = Descriptor::new(sddl)?;
    ensure!(
        unsafe {
            SetFileSecurityW(
                wide(path).as_ptr(),
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                descriptor.raw(),
            )
        } != 0,
        "无法保护网络助手文件：{}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

pub fn administrator() -> Result<bool> {
    let mut sid = std::ptr::null_mut();
    ensure!(
        unsafe { ConvertStringSidToSidW(wide("S-1-5-32-544").as_ptr(), &mut sid) } != 0,
        "无法读取管理员身份"
    );
    let mut member = 0;
    let result = unsafe { CheckTokenMembership(std::ptr::null_mut(), sid, &mut member) };
    unsafe {
        LocalFree(sid);
    }
    ensure!(result != 0, "无法检查管理员身份");
    Ok(member != 0)
}

pub fn process(pid: u32) -> Result<Handle> {
    Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000, 0, pid) })
}

pub fn image(process: &Handle) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let mut count = buffer.len() as u32;
    ensure!(
        unsafe { QueryFullProcessImageNameW(process.raw(), 0, buffer.as_mut_ptr(), &mut count) }
            != 0,
        "无法验证助手进程"
    );
    Ok(PathBuf::from(String::from_utf16(
        &buffer[..count as usize],
    )?))
}

pub fn user(process: HANDLE) -> Result<String> {
    let mut token = std::ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
        "无法验证调用账户"
    );
    let token = Handle::new(token)?;
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token.raw(), TokenUser, std::ptr::null_mut(), 0, &mut needed);
    }
    let mut bytes = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.raw(),
                TokenUser,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } != 0,
        "无法读取调用账户"
    );
    let sid = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
    let mut text = std::ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(sid, &mut text) } != 0,
        "无法读取调用账户 SID"
    );
    let mut count = 0;
    unsafe {
        while *text.add(count) != 0 {
            count += 1;
        }
    }
    let value = unsafe { String::from_utf16(std::slice::from_raw_parts(text, count)) };
    unsafe {
        LocalFree(text.cast());
    }
    Ok(value?)
}

pub fn current_user() -> Result<String> {
    user(unsafe { GetCurrentProcess() })
}

pub fn parent(pid: u32) -> Result<Handle> {
    let snapshot = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut ready = unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0;
    while ready {
        if entry.th32ProcessID == pid {
            return process(entry.th32ParentProcessID);
        }
        ready = unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    Err(anyhow::anyhow!("无法验证助手所属的客户端进程")).context("客户端可能已经退出")
}
