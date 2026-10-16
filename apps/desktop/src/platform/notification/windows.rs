use ::windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{NotificationSetting, ToastNotification, ToastNotificationManager},
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile,
                StructuredStorage::{PROPVARIANT, PVCHF_DEFAULT, PropVariantChangeType},
            },
            Variant::VT_LPWSTR,
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
        UI::Shell::{
            IShellLinkW, PropertiesSystem::IPropertyStore, SetCurrentProcessExplicitAppUserModelID,
            ShellLink,
        },
    },
    core::{GUID, HSTRING, Interface, PCWSTR},
};
use anyhow::{Context, Result, ensure};
const APP_ID: &str = "org.clashofrust.desktop";
const APP_ID_KEY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

pub(super) async fn show(title: &str, message: &str) -> Result<()> {
    let title = title.to_owned();
    let message = message.to_owned();
    tokio::task::spawn_blocking(move || deliver(&title, &message)).await?
}
fn deliver(title: &str, message: &str) -> Result<()> {
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
    register_identity().context("无法注册 Windows 通知应用")?;
    register_shortcut().context("无法注册应用通知身份")?;
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(r#"<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>"#))?;
    let texts = document.GetElementsByTagName(&HSTRING::from("text"))?;
    texts
        .Item(0)?
        .AppendChild(&document.CreateTextNode(&HSTRING::from(title))?)?;
    texts
        .Item(1)?
        .AppendChild(&document.CreateTextNode(&HSTRING::from(message))?)?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
        .context("Windows 无法创建应用通知发送器")?;
    ensure!(
        notifier.Setting().context("Windows 无法读取应用通知设置")? == NotificationSetting::Enabled,
        "Windows 已禁用应用通知，请在系统设置中允许 Clash of Rust 发送通知"
    );
    notifier
        .Show(&ToastNotification::CreateToastNotification(&document)?)
        .context("Windows 未接受通知")?;
    Ok(())
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
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(::windows::core::w!("org.clashofrust.desktop"))
    }?;
    Ok(())
}
fn register_shortcut() -> Result<()> {
    let executable = std::env::current_exe()?;
    let roaming = std::env::var_os("APPDATA").context("无法定位当前用户开始菜单")?;
    // Per-user identity also supports the portable package without administrator access.
    let directory = std::path::PathBuf::from(roaming)
        .join("Microsoft/Windows/Start Menu/Programs/Clash of Rust");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("Clash of Rust.lnk");
    let wide = |p: &std::path::Path| {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let exe = wide(&executable);
    let destination = wide(&path);
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
        properties.Commit()?;
        let file: IPersistFile = shortcut.cast()?;
        file.Save(PCWSTR(destination.as_ptr()), true)?;
    }
    Ok(())
}
