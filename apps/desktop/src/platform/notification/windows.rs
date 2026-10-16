use ::windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{NotificationSetting, ToastNotification, ToastNotificationManager},
    Win32::{
        Foundation::PROPERTYKEY,
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile,
                StructuredStorage::PROPVARIANT,
            },
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
        UI::Shell::{IShellLinkW, PropertiesSystem::IPropertyStore, ShellLink},
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
    // A dedicated worker owns the WinRT apartment; never initialize COM on Slint's thread.
    unsafe { RoInitialize(RO_INIT_MULTITHREADED) }?;
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { RoUninitialize() };
        }
    }
    let _apartment = Apartment;
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
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?;
    ensure!(
        notifier.Setting()? == NotificationSetting::Enabled,
        "Windows 已禁用应用通知，请在系统设置中允许 Clash of Rust 发送通知"
    );
    notifier.Show(&ToastNotification::CreateToastNotification(&document)?)?;
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
        properties.SetValue(&APP_ID_KEY, &PROPVARIANT::from(APP_ID))?;
        properties.Commit()?;
        let file: IPersistFile = shortcut.cast()?;
        file.Save(PCWSTR(destination.as_ptr()), true)?;
    }
    Ok(())
}
