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

fn user_sid() -> Result<String> {
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
            unsafe {
                folder.RegisterTask(
                    &BSTR::from(self.task_name.as_str()),
                    &BSTR::from(self.xml()),
                    TASK_CREATE_OR_UPDATE.0,
                    &empty,
                    &empty,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &empty,
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
            root.create_subkey(RUN_KEY)?
                .0
                .set_value(&self.run_name, &self.command())?;
            return Err(error).context("登录任务创建失败，已保留普通开机启动项");
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

    struct Cleanup(Backend);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = self.0.set(false);
        }
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
            assert_eq!(user_id.to_string(), backend.sid);
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
            other.0.legacy_enabled().unwrap(),
            "failed registration must keep a Run fallback"
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
