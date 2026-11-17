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
    Foundation::{
        ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    },
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
        // SAFETY: Module callers pass newly owned handles from successful Win32
        // creation/open calls, never borrowed or pseudo-handles. These APIs use
        // CloseHandle; after this transfer only OwnedHandle closes the handle.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
    pub fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
}

pub struct Descriptor(*mut std::ffi::c_void);
impl Descriptor {
    pub fn new(sddl: &str) -> Result<Self> {
        let sddl = wide(sddl);
        let mut raw = std::ptr::null_mut();
        // SAFETY: The terminated SDDL and writable output live through the call.
        // On success raw is a LocalAlloc-owned descriptor, freed once by Drop.
        ensure!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
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
        // SAFETY: new stored the descriptor returned by the conversion API;
        // no borrowed SECURITY_ATTRIBUTES outlives its owning Descriptor.
        unsafe {
            LocalFree(self.0);
        }
    }
}

struct FolderPath(*mut u16);
impl Drop for FolderPath {
    fn drop(&mut self) {
        // SAFETY: SHGetKnownFolderPath transfers task-allocator storage even on
        // failure. This guard owns it uniquely; CoTaskMemFree also accepts null.
        unsafe {
            CoTaskMemFree(self.0.cast());
        }
    }
}

fn known_folder_path(folder: &windows_sys::core::GUID) -> Result<PathBuf> {
    let mut path = FolderPath(std::ptr::null_mut());
    // SAFETY: folder and the writable output pointer live through the call;
    // the guard frees any returned allocation on success and every error path.
    ensure!(
        unsafe { SHGetKnownFolderPath(folder, 0, std::ptr::null_mut(), &mut path.0) } >= 0,
        "无法定位系统应用目录"
    );
    ensure!(!path.0.is_null(), "无法定位系统应用目录");
    let mut size = 0;
    // SAFETY: A successful query returns a terminated UTF-16 allocation, checked
    // non-null above and kept alive by path until the string has been copied.
    unsafe {
        while *path.0.add(size) != 0 {
            size += 1;
        }
    }
    // SAFETY: size spans the initialized prefix of the same live allocation.
    let text = unsafe { String::from_utf16(std::slice::from_raw_parts(path.0, size)) }?;
    Ok(PathBuf::from(text))
}

pub fn installed_directory() -> Result<PathBuf> {
    Ok(known_folder_path(&FOLDERID_ProgramFiles)?.join("Clash of Rust Network"))
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
    let path = wide(path);
    // SAFETY: The terminated path and descriptor remain alive through this
    // synchronous call; only owner and DACL fields present in the SDDL are used.
    ensure!(
        unsafe {
            SetFileSecurityW(
                path.as_ptr(),
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
    let administrators = wide("S-1-5-32-544");
    let mut sid = std::ptr::null_mut();
    // SAFETY: The terminated fixed SID and writable output live through the call.
    // A successful allocation is kept through membership checking and freed below.
    ensure!(
        unsafe { ConvertStringSidToSidW(administrators.as_ptr(), &mut sid) } != 0,
        "无法读取管理员身份"
    );
    let mut member = 0;
    // SAFETY: Null selects the current effective token, sid is still allocated,
    // and member is writable for this synchronous call.
    let result = unsafe { CheckTokenMembership(std::ptr::null_mut(), sid, &mut member) };
    // SAFETY: The conversion API transferred this LocalAlloc-owned SID once;
    // CheckTokenMembership does not retain it and all paths free it here.
    unsafe {
        LocalFree(sid);
    }
    ensure!(result != 0, "无法检查管理员身份");
    Ok(member != 0)
}

pub fn process(pid: u32) -> Result<Handle> {
    // SAFETY: OpenProcess validates the PID, creates a non-inheritable owned
    // handle with query/wait rights, and Handle adopts it only on success.
    Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000, 0, pid) })
}

pub fn image(process: &Handle) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let mut count = buffer.len() as u32;
    // SAFETY: process stays open; buffer is writable for the character capacity
    // passed in count. On success count identifies the initialized UTF-16 prefix.
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
    // SAFETY: Callers keep the process handle alive, or pass GetCurrentProcess's
    // pseudo-handle. The writable output receives a newly owned query token.
    ensure!(
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } != 0,
        "无法验证调用账户"
    );
    let token = Handle::new(token)?;
    let mut needed = 0;
    // SAFETY: A null buffer with length zero probes the size. needed is writable
    // and the owned TOKEN_QUERY handle stays open. Capture its error immediately.
    let (result, error) = unsafe {
        (
            GetTokenInformation(token.raw(), TokenUser, std::ptr::null_mut(), 0, &mut needed),
            GetLastError(),
        )
    };
    ensure!(
        result == 0 && error == ERROR_INSUFFICIENT_BUFFER,
        "无法探测调用账户大小：{error}"
    );
    ensure!(
        needed as usize >= std::mem::size_of::<TOKEN_USER>(),
        "调用账户信息长度无效"
    );
    let capacity = needed;
    let mut bytes = vec![0usize; (capacity as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: usize storage meets TOKEN_USER's pointer alignment on Windows.
    // Its allocated byte capacity is at least capacity; the token, buffer and
    // returned length stay alive through this synchronous call.
    ensure!(
        unsafe {
            GetTokenInformation(
                token.raw(),
                TokenUser,
                bytes.as_mut_ptr().cast(),
                capacity,
                &mut needed,
            )
        } != 0,
        "无法读取调用账户"
    );
    ensure!(
        (std::mem::size_of::<TOKEN_USER>()..=capacity as usize).contains(&(needed as usize)),
        "调用账户信息长度无效"
    );
    // SAFETY: A successful TokenUser query initialized the aligned TOKEN_USER
    // and its referenced SID inside bytes; bytes remains alive through conversion.
    let sid = unsafe { &*bytes.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
    let mut text = std::ptr::null_mut();
    // SAFETY: sid belongs to the live token-information buffer and text is a
    // writable output. On success the resulting string must be LocalFree'd.
    ensure!(
        unsafe { ConvertSidToStringSidW(sid, &mut text) } != 0,
        "无法读取调用账户 SID"
    );
    let mut count = 0;
    // SAFETY: The successful conversion returned a terminated UTF-16 allocation,
    // which remains live until its initialized prefix has been copied.
    unsafe {
        while *text.add(count) != 0 {
            count += 1;
        }
    }
    // SAFETY: count covers only initialized UTF-16 units in the live allocation.
    let value = unsafe { String::from_utf16(std::slice::from_raw_parts(text, count)) };
    // SAFETY: This is the one LocalFree for the successful SID conversion,
    // after copying and also before propagating a UTF-16 decoding error.
    unsafe {
        LocalFree(text.cast());
    }
    Ok(value?)
}

pub fn current_user() -> Result<String> {
    // SAFETY: GetCurrentProcess takes no pointers; user only borrows the returned
    // pseudo-handle and never wraps it in an owning Handle.
    user(unsafe { GetCurrentProcess() })
}

pub fn parent(pid: u32) -> Result<Handle> {
    // SAFETY: The process-only snapshot takes no pointers and returns a newly
    // owned handle for Handle to close, or an invalid value rejected by Handle.
    let snapshot = Handle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?;
    // SAFETY: PROCESSENTRY32W has only integer and fixed-buffer fields; zero is
    // valid initial storage, with dwSize set before passing it to the API.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    // SAFETY: snapshot is open and entry is writable with its required size.
    // Fields are read only after a successful enumeration call.
    let mut ready = unsafe { Process32FirstW(snapshot.raw(), &mut entry) } != 0;
    while ready {
        if entry.th32ProcessID == pid {
            return process(entry.th32ParentProcessID);
        }
        // SAFETY: The same snapshot and sized entry remain alive for the next
        // synchronous enumeration call; failure prevents further field reads.
        ready = unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    Err(anyhow::anyhow!("无法验证助手所属的客户端进程")).context("客户端可能已经退出")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_matches_opened_process_token() {
        let process = process(std::process::id()).unwrap();
        let expected = current_user().unwrap();
        assert!(expected.starts_with("S-1-"));
        assert_eq!(user(process.raw()).unwrap(), expected);
    }

    #[test]
    fn null_process_cannot_supply_a_user_token() {
        assert!(user(std::ptr::null_mut()).is_err());
    }

    #[test]
    fn known_folder_failure_does_not_read_an_output_path() {
        let unknown = windows_sys::core::GUID::from_u128(0);
        assert!(known_folder_path(&unknown).is_err());
    }

    #[test]
    fn installed_directory_stays_below_program_files() {
        let program_files = known_folder_path(&FOLDERID_ProgramFiles).unwrap();
        assert!(program_files.is_absolute());
        assert_eq!(
            installed_directory().unwrap(),
            program_files.join("Clash of Rust Network")
        );
    }
}
