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
    arguments: String,
    sid: String,
    task_name: String,
    marker_key: String,
    run_name: String,
}

impl Backend {
    pub fn current() -> Result<Self> {
        let sid = user_sid()?;
        Ok(Self {
            executable: crate::platform::autostart_executable()?,
            arguments: "--background".into(),
            task_name: format!("ClashOfRust-{sid}"),
            sid,
            marker_key: "Software\\ClashOfRust\\Autostart".into(),
            run_name: "ClashOfRust".into(),
        })
    }

    fn command(&self) -> String {
        format!("\"{}\" {}", self.executable.display(), self.arguments)
    }

    pub fn headless(data: &Path, resources: Option<&Path>) -> Result<Self> {
        let mut backend = Self::current()?;
        backend.executable = crate::platform::autostart_executable()?;
        backend.task_name = format!("ClashOfRustCLI-{}", backend.sid);
        backend.marker_key = "Software\\ClashOfRust\\CLIStartup".into();
        backend.run_name = "ClashOfRustCLI".into();
        let path = data.to_string_lossy();
        anyhow::ensure!(!path.contains(['\"', '\n', '\r', '\0']), "后台数据目录无效");
        let trailing = path.chars().rev().take_while(|c| *c == '\\').count();
        backend.arguments = format!(
            "--cli-background --data-dir \"{path}{}\"",
            "\\".repeat(trailing)
        );
        if let Some(resources) = resources {
            let path = resources.to_string_lossy();
            anyhow::ensure!(!path.contains(['\"', '\n', '\r', '\0']), "资源目录无效");
            let trailing = path.chars().rev().take_while(|c| *c == '\\').count();
            backend.arguments += &format!(" --resources \"{path}{}\"", "\\".repeat(trailing));
        }
        Ok(backend)
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
            && key
                .get_value::<String, _>(&self.run_name)
                .ok()
                .is_some_and(|command| {
                    if self.run_name == "ClashOfRustCLI" {
                        command.starts_with(&format!(
                            "\"{}\" --cli-background ",
                            self.executable.display()
                        ))
                    } else {
                        command == self.command()
                    }
                })
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
        let arguments = xml_escape(&self.arguments);
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
 <Actions Context="User"><Exec><Command>{executable}</Command><Arguments>{arguments}</Arguments><WorkingDirectory>{directory}</WorkingDirectory></Exec></Actions>
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
}
