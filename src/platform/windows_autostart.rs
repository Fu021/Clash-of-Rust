//! Per-user logon tasks avoid Explorer's delayed Run-key startup queue.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use windows::{
    Win32::System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        TaskScheduler::{
            IRegisteredTask, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE,
            TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler,
        },
        Variant::VARIANT,
    },
    core::{BSTR, IUnknown},
};
use winreg::{RegKey, enums::HKEY_CURRENT_USER};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn with_folder<T>(operation: impl FnOnce(&ITaskFolder) -> Result<T>) -> Result<T> {
    let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // An existing STA can also use the scheduler; only balance successful init.
    if status.is_err() && status.0 as u32 != 0x80010106 {
        status.ok()?;
    }
    let _apartment = Apartment(status.is_ok());
    let service: ITaskService =
        unsafe { CoCreateInstance(&TaskScheduler, None::<&IUnknown>, CLSCTX_INPROC_SERVER)? };
    let empty = VARIANT::default();
    unsafe { service.Connect(&empty, &empty, &empty, &empty)? };
    let folder = unsafe { service.GetFolder(&BSTR::from("\\"))? };
    operation(&folder)
}

pub(super) fn user_sid() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, LocalFree},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    let mut handle = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let token = Token(handle);
    let mut length = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut length) };
    if length == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // Align the buffer for TOKEN_USER, not just bytes.
    let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut count = 0;
    unsafe {
        while *text.add(count) != 0 {
            count += 1;
        }
        let result = String::from_utf16(std::slice::from_raw_parts(text, count));
        LocalFree(text.cast());
        Ok(result?)
    }
}

pub(super) fn access_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<windows::core::Error>()
            .is_some_and(|error| error.code().0 as u32 == 0x80070005)
            || cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.raw_os_error() == Some(5))
    })
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(super) struct Backend {
    executable: PathBuf,
    sid: String,
    task_name: String,
    marker_key: String,
    run_name: String,
}

impl Backend {
    pub fn current() -> Result<Self> {
        let sid = user_sid()?;
        Ok(Self {
            executable: std::env::current_exe()?,
            task_name: format!("ClashOfRust-{sid}"),
            sid,
            marker_key: "Software\\ClashOfRust\\Autostart".into(),
            run_name: "ClashOfRust".into(),
        })
    }

    fn command(&self) -> String {
        format!("\"{}\" --background", self.executable.display())
    }

    fn marked(&self) -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(&self.marker_key)
            .ok()
            .and_then(|key| key.get_value::<String, _>("Executable").ok())
            .is_some_and(|path| path.eq_ignore_ascii_case(&self.executable.to_string_lossy()))
    }

    fn owned_task(&self, folder: &ITaskFolder) -> Result<Option<IRegisteredTask>> {
        let task = match unsafe { folder.GetTask(&BSTR::from(self.task_name.as_str())) } {
            Ok(task) => task,
            Err(error) if matches!(error.code().0 as u32, 0x80070002 | 0x80070003) => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let xml = unsafe { task.Xml()? }.to_string();
        let document = roxmltree::Document::parse(&xml)?;
        let command = document
            .descendants()
            .find(|node| node.has_tag_name("Command"))
            .and_then(|node| node.text());
        if !command
            .is_some_and(|path| path.eq_ignore_ascii_case(&self.executable.to_string_lossy()))
        {
            bail!("登录任务属于另一安装目录，未修改该任务");
        }
        Ok(Some(task))
    }

    pub fn legacy_enabled(&self) -> Result<bool> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let command = root
            .open_subkey(RUN_KEY)
            .ok()
            .and_then(|key| key.get_value::<String, _>(&self.run_name).ok());
        if command.as_deref() != Some(self.command().as_str()) {
            return Ok(false);
        }
        // Preserve a user's decision to disable the old item in Task Manager.
        let approved = root
            .open_subkey(APPROVED_KEY)
            .ok()
            .and_then(|key| key.get_raw_value(&self.run_name).ok());
        Ok(approved.is_none_or(|value| matches!(value.bytes.first(), Some(2 | 6))))
    }

    pub fn enabled(&self) -> Result<bool> {
        if self.legacy_enabled()? {
            return Ok(true);
        }
        if self.marked() {
            return with_folder(|folder| match self.owned_task(folder)? {
                Some(task) => Ok(unsafe { task.Enabled()? }.0 != 0),
                None => Ok(false),
            });
        }
        Ok(false)
    }

    fn remove_legacy(&self) -> Result<()> {
        if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_READ | winreg::enums::KEY_WRITE)
            && key.get_value::<String, _>(&self.run_name).ok().as_deref()
                == Some(self.command().as_str())
        {
            key.delete_value(&self.run_name)?;
        }
        Ok(())
    }

    fn register_fallback(&self, scheduler_error: anyhow::Error) -> Result<()> {
        // An ownership conflict or a failed registry write is not a scheduler
        // outage. Do not create a duplicate entry for an existing marked task.
        if scheduler_error
            .downcast_ref::<windows::core::Error>()
            .is_none()
            || self.marked()
        {
            return Err(scheduler_error).context("无法修改开机启动登录任务");
        }
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = root
            .create_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE)
            .context("无法创建普通开机启动项")?;
        key.set_value(&self.run_name, &self.command())
            .context("无法保存普通开机启动项")?;
        // The user explicitly enabled startup; undo a Task Manager disable bit
        // for this entry, without touching any other startup item.
        if let Ok(key) = root.open_subkey_with_flags(APPROVED_KEY, winreg::enums::KEY_SET_VALUE) {
            match key.delete_value(&self.run_name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("无法启用普通开机启动项"),
            }
        }
        if !self.legacy_enabled()? {
            bail!("普通开机启动项仍被系统禁用");
        }
        // Successful fallback is a successful enable operation.
        Ok(())
    }

    fn xml(&self) -> String {
        let sid = xml_escape(&self.sid);
        let executable = xml_escape(&self.executable.to_string_lossy());
        let directory = xml_escape(
            &self
                .executable
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy(),
        );
        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
 <RegistrationInfo><Description>Clash of Rust user logon startup</Description></RegistrationInfo>
 <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
 <Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
 <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><Enabled>true</Enabled><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Priority>5</Priority></Settings>
 <Actions Context="User"><Exec><Command>{executable}</Command><Arguments>--background</Arguments><WorkingDirectory>{directory}</WorkingDirectory></Exec></Actions>
</Task>"#
        )
    }

    fn task_security(&self) -> BSTR {
        BSTR::from(format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{})", self.sid))
    }

    /// Upgrade an existing task's permissions without changing its enabled
    /// state, command, principal or trigger. Only an owned task can be repaired.
    pub fn repair_permissions(&self) -> Result<()> {
        if !self.marked() {
            return Ok(());
        }
        with_folder(|folder| {
            if let Some(task) = self.owned_task(folder)? {
                unsafe { task.SetSecurityDescriptor(&self.task_security(), 0)? };
            }
            Ok(())
        })
    }

    pub fn set(&self, enabled: bool) -> Result<()> {
        let root = RegKey::predef(HKEY_CURRENT_USER);
        if !enabled {
            if self.marked() {
                with_folder(|folder| {
                    if self.owned_task(folder)?.is_some() {
                        unsafe { folder.DeleteTask(&BSTR::from(self.task_name.as_str()), 0)? };
                    }
                    Ok(())
                })?;
                root.delete_subkey_all(&self.marker_key)?;
            }
            return self.remove_legacy();
        }
        let registered = with_folder(|folder| {
            // Refuse to overwrite a task pointing at someone else's installation.
            self.owned_task(folder)?;
            let empty = VARIANT::default();
            let user = VARIANT::from(BSTR::from(self.sid.as_str()));
            let security = VARIANT::from(self.task_security());
            unsafe {
                folder.RegisterTask(
                    &BSTR::from(self.task_name.as_str()),
                    &BSTR::from(self.xml()),
                    TASK_CREATE_OR_UPDATE.0,
                    &user,
                    &empty,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &security,
                )?;
            }
            let recorded = root
                .create_subkey(&self.marker_key)
                .and_then(|(marker, _)| {
                    marker.set_value("Executable", &self.executable.to_string_lossy().as_ref())
                });
            if let Err(error) = recorded {
                unsafe { folder.DeleteTask(&BSTR::from(self.task_name.as_str()), 0)? };
                return Err(error.into());
            }
            Ok(())
        });
        if let Err(error) = registered {
            return self.register_fallback(error);
        }
        self.remove_legacy()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::{
        Foundation::VARIANT_BOOL,
        System::TaskScheduler::{TASK_RUNLEVEL_HIGHEST, TASK_RUNLEVEL_LUA},
    };

    fn grants_current_user_access(task: &IRegisteredTask, sid: &str) -> bool {
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                ACCESS_ALLOWED_ACE,
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
                },
                EqualSid, GetAce, GetSecurityDescriptorDacl,
            },
        };
        struct Allocation(*mut std::ffi::c_void);
        impl Drop for Allocation {
            fn drop(&mut self) {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
        let security = unsafe { task.GetSecurityDescriptor(4).unwrap() }.to_string();
        let security: Vec<u16> = security.encode_utf16().chain(Some(0)).collect();
        let sid: Vec<u16> = sid.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = Allocation(std::ptr::null_mut());
        let mut user = Allocation(std::ptr::null_mut());
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = std::ptr::null_mut();
        unsafe {
            assert_ne!(
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    security.as_ptr(),
                    1,
                    &mut descriptor.0,
                    std::ptr::null_mut()
                ),
                0
            );
            assert_ne!(ConvertStringSidToSidW(sid.as_ptr(), &mut user.0), 0);
            assert_ne!(
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted),
                0
            );
            assert_ne!(present, 0);
            assert!(!acl.is_null());
            for index in 0..(*acl).AceCount {
                let mut ace = std::ptr::null_mut();
                assert_ne!(GetAce(acl, u32::from(index), &mut ace), 0);
                let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                if ace.Header.AceType == 0
                    && EqualSid((&ace.SidStart as *const u32).cast_mut().cast(), user.0) != 0
                    && (ace.Mask & 0x10000000 != 0 || ace.Mask & 0x1f01ff == 0x1f01ff)
                {
                    return true;
                }
            }
        }
        false
    }

    fn account_sid(account: &str) -> Result<String> {
        use windows_sys::Win32::{
            Foundation::{ERROR_INSUFFICIENT_BUFFER, LocalFree},
            Security::{Authorization::ConvertSidToStringSidW, LookupAccountNameW, SidTypeUser},
        };
        // Task Scheduler can return either a SID or a resolved account name.
        if account.starts_with("S-1-") {
            return Ok(account.into());
        }
        let account: Vec<u16> = account.encode_utf16().chain(Some(0)).collect();
        let mut sid_length = 0;
        let mut domain_length = 0;
        let mut kind = SidTypeUser;
        unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                account.as_ptr(),
                std::ptr::null_mut(),
                &mut sid_length,
                std::ptr::null_mut(),
                &mut domain_length,
                &mut kind,
            );
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error.into());
        }
        let mut sid = vec![0usize; (sid_length as usize).div_ceil(std::mem::size_of::<usize>())];
        let mut domain = vec![0u16; domain_length as usize];
        if unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                account.as_ptr(),
                sid.as_mut_ptr().cast(),
                &mut sid_length,
                domain.as_mut_ptr(),
                &mut domain_length,
                &mut kind,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast_mut().cast(), &mut text) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut length = 0;
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
            let result = String::from_utf16(std::slice::from_raw_parts(text, length));
            LocalFree(text.cast());
            Ok(result?)
        }
    }

    struct Cleanup(Backend);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.set(false);
        }
    }

    #[test]
    fn scheduler_access_denied_falls_back_to_working_user_startup() {
        let mut backend = Backend::current().unwrap();
        let name = format!("ClashOfRust.CI.{}", uuid::Uuid::new_v4());
        backend.task_name = name.clone();
        backend.run_name = name.clone();
        backend.marker_key = format!("Software\\ClashOfRust\\Tests\\{name}");
        let cleanup = Cleanup(backend);
        let backend = &cleanup.0;
        let root = RegKey::predef(HKEY_CURRENT_USER);
        root.create_subkey(APPROVED_KEY)
            .unwrap()
            .0
            .set_raw_value(
                &backend.run_name,
                &winreg::RegValue {
                    vtype: winreg::enums::REG_BINARY,
                    bytes: vec![3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                },
            )
            .unwrap();
        let denied =
            windows::core::Error::from_hresult(windows::core::HRESULT(0x80070005u32 as i32));
        backend.register_fallback(denied.into()).unwrap();
        assert!(backend.enabled().unwrap());
        backend.set(false).unwrap();
        assert!(!backend.enabled().unwrap());
        assert!(!backend.marked());
    }

    #[test]
    #[ignore = "requires the native Windows Task Scheduler; creates only a unique test task"]
    fn native_autostart_migrates_and_cleans_up_a_user_logon_task() {
        let mut backend = Backend::current().unwrap();
        let name = format!("ClashOfRust.CI.{}", uuid::Uuid::new_v4());
        backend.task_name = name.clone();
        backend.run_name = name.clone();
        backend.marker_key = format!("Software\\ClashOfRust\\Tests\\{name}");
        let cleanup = Cleanup(backend);
        let backend = &cleanup.0;
        let root = RegKey::predef(HKEY_CURRENT_USER);
        root.create_subkey(RUN_KEY)
            .unwrap()
            .0
            .set_value(&backend.run_name, &backend.command())
            .unwrap();
        assert!(backend.legacy_enabled().unwrap());
        backend.set(true).unwrap();
        assert!(backend.enabled().unwrap());
        assert!(!backend.legacy_enabled().unwrap());
        with_folder(|folder| {
            let task = backend.owned_task(folder)?.unwrap();
            assert!(
                grants_current_user_access(&task, &backend.sid),
                "the user must retain task access after elevated registration"
            );
            // Task Scheduler omits default-valued fields from its exported XML.
            // Check the effective registered settings through COM instead.
            let definition = unsafe { task.Definition()? };
            let principal = unsafe { definition.Principal()? };
            let settings = unsafe { definition.Settings()? };
            let mut user_id = BSTR::new();
            let mut logon_type = TASK_LOGON_INTERACTIVE_TOKEN;
            let mut run_level = TASK_RUNLEVEL_HIGHEST;
            let mut disallow_battery = VARIANT_BOOL(-1);
            let mut stop_on_battery = VARIANT_BOOL(-1);
            let mut network_required = VARIANT_BOOL(-1);
            let mut start_when_available = VARIANT_BOOL(0);
            let mut time_limit = BSTR::new();
            let mut priority = 0;
            unsafe {
                principal.UserId(&mut user_id)?;
                principal.LogonType(&mut logon_type)?;
                principal.RunLevel(&mut run_level)?;
                settings.DisallowStartIfOnBatteries(&mut disallow_battery)?;
                settings.StopIfGoingOnBatteries(&mut stop_on_battery)?;
                settings.RunOnlyIfNetworkAvailable(&mut network_required)?;
                settings.StartWhenAvailable(&mut start_when_available)?;
                settings.ExecutionTimeLimit(&mut time_limit)?;
                settings.Priority(&mut priority)?;
            }
            assert_eq!(account_sid(&user_id.to_string())?, backend.sid);
            assert_eq!(logon_type, TASK_LOGON_INTERACTIVE_TOKEN);
            assert_eq!(run_level, TASK_RUNLEVEL_LUA);
            assert_eq!(disallow_battery.0, 0);
            assert_eq!(stop_on_battery.0, 0);
            assert_eq!(network_required.0, 0);
            assert_ne!(start_when_available.0, 0);
            assert_eq!(time_limit.to_string(), "PT0S");
            assert_eq!(priority, 5);
            let xml = unsafe { task.Xml()? }.to_string();
            let document = roxmltree::Document::parse(&xml)?;
            let value = |name| {
                document
                    .descendants()
                    .find(|node| node.has_tag_name(name))
                    .and_then(|node| node.text())
            };
            assert!(value("Delay").is_none_or(|delay| delay == "PT0S"));
            assert_eq!(value("Arguments"), Some("--background"));
            unsafe { task.SetEnabled(VARIANT_BOOL(0))? };
            Ok(())
        })
        .unwrap();
        assert!(!backend.enabled().unwrap());
        // Simulate an older task whose DACL only lets the user's ordinary token
        // read it. Repair it as admin without accidentally re-enabling it.
        with_folder(|folder| {
            let task = backend.owned_task(folder)?.unwrap();
            let legacy = BSTR::from(format!(
                "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGX;;;{})",
                backend.sid
            ));
            // Preserve the deliberately restricted fixture: otherwise Task
            // Scheduler may automatically add an allow ACE for the principal.
            unsafe { task.SetSecurityDescriptor(&legacy, 0x10)? };
            assert!(!grants_current_user_access(&task, &backend.sid));
            Ok(())
        })
        .unwrap();
        backend.repair_permissions().unwrap();
        assert!(!backend.enabled().unwrap());
        with_folder(|folder| {
            assert!(grants_current_user_access(
                &backend.owned_task(folder)?.unwrap(),
                &backend.sid
            ));
            Ok(())
        })
        .unwrap();
        backend.set(true).unwrap();
        assert!(backend.enabled().unwrap());
        let other = Cleanup(Backend {
            executable: backend
                .executable
                .with_file_name("different-installation.exe"),
            sid: backend.sid.clone(),
            task_name: backend.task_name.clone(),
            marker_key: backend.marker_key.clone(),
            run_name: backend.run_name.clone(),
        });
        assert!(
            other.0.set(true).is_err(),
            "a foreign task must not be overwritten"
        );
        assert!(
            !other.0.legacy_enabled().unwrap(),
            "an ownership conflict must not create a duplicate Run entry"
        );
        other.0.set(false).unwrap();
        drop(other);
        assert!(backend.enabled().unwrap());
        backend.set(false).unwrap();
        backend.set(false).unwrap();
        assert!(!backend.enabled().unwrap());
        with_folder(|folder| {
            assert!(backend.owned_task(folder)?.is_none());
            Ok(())
        })
        .unwrap();
    }
}
