use ::windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::TypedEventHandler,
    UI::Notifications::{
        NotificationSetting, ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
    },
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemAlloc, IPersistFile,
                StructuredStorage::{PROPVARIANT, PVCHF_DEFAULT, PropVariantChangeType},
            },
            Variant::{VT_CLSID, VT_LPWSTR},
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
        UI::Shell::{
            IShellLinkW, PropertiesSystem::IPropertyStore, SHCNE_CREATE, SHCNE_UPDATEITEM,
            SHCNF_FLUSHNOWAIT, SHCNF_PATHW, SHChangeNotify,
            SetCurrentProcessExplicitAppUserModelID, ShellLink,
        },
    },
    core::{GUID, HSTRING, Interface, PCWSTR},
};
use anyhow::{Context, Result, ensure};
use std::{path::Path, sync::Mutex, time::Duration};
const APP_ID: &str = "org.clashofrust.desktop";
const APP_ID_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};
// Microsoft's supported no-COM desktop toast path: a stub CLSID persists the
// notification in Action Center, while protocol activation opens our instance.
// https://learn.microsoft.com/windows/apps/design/shell/tiles-and-notifications/toast-desktop-apps
const ACTIVATOR: GUID = GUID::from_u128(0x55fe17e9_64d7_4538_8274_a9c87c23ab72);
const ACTIVATOR_TEXT: &str = "{55FE17E9-64D7-4538-8274-A9C87C23AB72}";
const ACTIVATOR_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_ID_KEY.fmtid,
    pid: 26,
};
const PROTOCOL: &str = "clash-of-rust-notification";
const TOAST_XML: &str = r#"<toast activationType="protocol" launch="clash-of-rust-notification:open"><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>"#;
static REGISTERED: Mutex<bool> = Mutex::new(false);

pub(super) fn prepare_process() -> Result<()> {
    // Set the window identity before Slint creates the first native window.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID)) }?;
    Ok(())
}

pub(super) async fn initialize() -> Result<()> {
    tokio::task::spawn_blocking(|| with_apartment(ensure_registered)).await?
}

pub(super) async fn show(title: &str, message: &str) -> Result<()> {
    let title = title.to_owned();
    let message = message.to_owned();
    tokio::task::spawn_blocking(move || deliver(&title, &message)).await?
}
fn with_apartment<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    // Keep WinRT and shell COM calls on a worker, away from Slint's UI thread.
    let initialized = match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => true,
        // Tokio may reuse a worker already initialized as an STA by a shell
        // operation. Its existing apartment is valid; do not uninitialize it.
        Err(error) if error.code().0 == 0x80010106u32 as i32 => false,
        Err(error) => return Err(error.into()),
    };
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() };
            }
        }
    }
    let _apartment = Apartment(initialized);
    operation()
}

fn ensure_registered() -> Result<()> {
    let mut registered = REGISTERED.lock().unwrap_or_else(|e| e.into_inner());
    if !*registered {
        register_identity().context("无法注册 Windows 通知应用")?;
        register_shortcut().context("无法注册应用通知身份")?;
        // Do not rewrite the Start shortcut immediately before each toast:
        // Explorer may still be reindexing it when Setting/Show runs.
        *registered = true;
    }
    Ok(())
}

fn deliver(title: &str, message: &str) -> Result<()> {
    with_apartment(|| {
        ensure_registered()?;
        send(title, message)
    })
}

fn send(title: &str, message: &str) -> Result<()> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(TOAST_XML))?;
    let texts = document.GetElementsByTagName(&HSTRING::from("text"))?;
    texts
        .Item(0)?
        .AppendChild(&document.CreateTextNode(&HSTRING::from(title))?)?;
    texts
        .Item(1)?
        .AppendChild(&document.CreateTextNode(&HSTRING::from(message))?)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
        .context("Windows 无法创建应用通知发送器")?;
    check_setting(notifier.Setting())?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    // Show can return successfully and reject the payload asynchronously.
    let (failed, failure) = std::sync::mpsc::sync_channel(1);
    let token = toast.Failed(
        &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(move |_, args| {
            if let Some(args) = args.as_ref() {
                let _ = failed.try_send(args.ErrorCode()?);
            }
            Ok(())
        }),
    )?;
    let result = notifier.Show(&toast).context("Windows 未接受通知");
    let error = if result.is_ok() {
        failure.recv_timeout(Duration::from_secs(2)).ok()
    } else {
        None
    };
    let _ = toast.RemoveFailed(token);
    result?;
    if let Some(code) = error {
        return Err(::windows::core::Error::from_hresult(code)).context("Windows 通知发送失败");
    }
    Ok(())
}

fn check_setting(setting: ::windows::core::Result<NotificationSetting>) -> Result<()> {
    match setting {
        Ok(NotificationSetting::Enabled) => Ok(()),
        // A fresh unpackaged AUMID may not have a settings entry until its
        // first toast. This is not the same as the user disabling notifications.
        Err(error) if error.code().0 == 0x80070490u32 as i32 => Ok(()),
        Err(error) => Err(error).context("Windows 无法读取应用通知设置"),
        Ok(_) => anyhow::bail!("Windows 已禁用应用通知，请在系统设置中允许 Clash of Rust 发送通知"),
    }
}
fn register_identity() -> Result<()> {
    // Portable Win32 apps have no MSIX identity. A shortcut alone can remain
    // invisible to the notification platform until Explorer indexes it, causing
    // ERROR_NOT_FOUND. Register the same AUMID explicitly for the current user.
    let data = std::env::var_os("APPDATA").context("无法定位应用通知目录")?;
    // Keep the icon inside the same per-user data tree covered by the uninstaller.
    let directory = std::path::PathBuf::from(data).join("clash-of-rust/ClashOfRust/notifications");
    std::fs::create_dir_all(&directory)?;
    let icon = directory.join("app.png");
    if !icon.is_file() {
        std::fs::write(
            &icon,
            include_bytes!("../../../../../resources/icons/app.png"),
        )?;
    }
    let (key, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))?;
    key.set_value("DisplayName", &"Clash of Rust")?;
    key.set_value("IconUri", &icon.to_string_lossy().as_ref())?;
    key.set_value("ShowInSettings", &1u32)?;
    key.set_value("CustomActivator", &ACTIVATOR_TEXT)?;
    let (protocol, _) = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey(format!(r"Software\Classes\{PROTOCOL}"))?;
    protocol.set_value("", &"URL:Clash of Rust notification")?;
    protocol.set_value("URL Protocol", &"")?;
    let (command, _) = protocol.create_subkey(r"shell\open\command")?;
    command.set_value(
        "",
        &activation_command(&crate::platform::autostart_executable()?),
    )?;
    Ok(())
}

fn activation_command(executable: &Path) -> String {
    // No URL interpolation: this handler only brings the existing window back.
    format!("\"{}\" --notification", executable.display())
}

fn register_shortcut() -> Result<()> {
    let executable = crate::platform::autostart_executable()?;
    let roaming = std::env::var_os("APPDATA").context("无法定位当前用户开始菜单")?;
    // Per-user identity also supports the portable package without administrator access.
    let directory = std::path::PathBuf::from(roaming)
        .join("Microsoft/Windows/Start Menu/Programs/Clash of Rust");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("Clash of Rust.lnk");
    let existed = path.exists();
    save_shortcut(&path, &executable)?;
    let destination = wide(&path);
    unsafe {
        SHChangeNotify(
            if existed {
                SHCNE_UPDATEITEM
            } else {
                SHCNE_CREATE
            },
            SHCNF_PATHW | SHCNF_FLUSHNOWAIT,
            Some(destination.as_ptr().cast()),
            None,
        );
    }
    Ok(())
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

fn activator_property() -> Result<PROPVARIANT> {
    // IPropertyStore expects VT_CLSID, not a GUID formatted as a string.
    // PROPVARIANT owns this COM allocation and releases it in PropVariantClear.
    let pointer = unsafe { CoTaskMemAlloc(std::mem::size_of::<GUID>()) }.cast::<GUID>();
    ensure!(!pointer.is_null(), "无法分配 Windows 通知标识");
    let mut value = PROPVARIANT::default();
    unsafe {
        pointer.write(ACTIVATOR);
        let variant = &mut *value.Anonymous.Anonymous;
        variant.vt = VT_CLSID;
        variant.Anonymous.puuid = pointer;
    }
    Ok(value)
}

fn save_shortcut(path: &Path, executable: &Path) -> Result<()> {
    let exe = wide(executable);
    let destination = wide(path);
    // COM copies both strings and the property; all buffers outlive these calls.
    unsafe {
        let shortcut: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        shortcut.SetPath(PCWSTR(exe.as_ptr()))?;
        shortcut.SetDescription(::windows::core::w!("Clash of Rust"))?;
        let properties: IPropertyStore = shortcut.cast()?;
        let mut app_id = PROPVARIANT::default();
        PropVariantChangeType(
            &mut app_id,
            &PROPVARIANT::from(APP_ID),
            PVCHF_DEFAULT,
            VT_LPWSTR,
        )?;
        properties.SetValue(&APP_ID_KEY, &app_id)?;
        properties.SetValue(&ACTIVATOR_KEY, &activator_property()?)?;
        properties.Commit()?;
        let file: IPersistFile = shortcut.cast()?;
        file.Save(PCWSTR(destination.as_ptr()), true)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::windows::{
        Win32::System::Com::STGM_READ,
        core::{BSTR, Error, HRESULT},
    };

    #[test]
    fn missing_first_notification_settings_do_not_block_sending_but_denials_do() {
        assert!(check_setting(Ok(NotificationSetting::Enabled)).is_ok());
        assert!(check_setting(Err(Error::from_hresult(HRESULT(0x80070490u32 as i32)))).is_ok());
        for setting in [
            NotificationSetting::DisabledForApplication,
            NotificationSetting::DisabledForUser,
            NotificationSetting::DisabledByGroupPolicy,
            NotificationSetting::DisabledByManifest,
        ] {
            assert!(check_setting(Ok(setting)).is_err());
        }
        let error =
            check_setting(Err(Error::from_hresult(HRESULT(0x80070005u32 as i32)))).unwrap_err();
        assert!(format!("{error:#}").contains("0x80070005"));
    }

    #[test]
    fn notification_shortcut_persists_aumid_and_typed_stub_activator() {
        // Exercise the production shell serialization without installing a
        // shortcut or changing the CI account's notification/registry settings.
        with_apartment(|| {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("notification.lnk");
            save_shortcut(&path, &std::env::current_exe()?)?;
            unsafe {
                let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
                link.cast::<IPersistFile>()?
                    .Load(PCWSTR(wide(&path).as_ptr()), STGM_READ)?;
                let store: IPropertyStore = link.cast()?;
                assert_eq!(
                    BSTR::try_from(&store.GetValue(&APP_ID_KEY)?)?.to_string(),
                    APP_ID
                );
                let activator = store.GetValue(&ACTIVATOR_KEY)?;
                assert_eq!(activator.vt(), VT_CLSID);
                assert_eq!(*activator.Anonymous.Anonymous.Anonymous.puuid, ACTIVATOR);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn notification_activation_uses_a_quoted_fixed_command_and_protocol_template() {
        assert_eq!(
            activation_command(Path::new(
                r"C:\Program Files\Clash of Rust\clash-of-rust.exe"
            )),
            r#""C:\Program Files\Clash of Rust\clash-of-rust.exe" --notification"#
        );
        assert_eq!(format!("{{{ACTIVATOR:?}}}").to_uppercase(), ACTIVATOR_TEXT);
        let document = roxmltree::Document::parse(TOAST_XML).unwrap();
        let toast = document.root_element();
        assert_eq!(toast.attribute("activationType"), Some("protocol"));
        assert_eq!(
            toast.attribute("launch"),
            Some(format!("{PROTOCOL}:open").as_str())
        );
    }
}
