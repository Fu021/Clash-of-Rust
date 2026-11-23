//! OS-native YAML selection, loaded only when requested. No GTK UI or browser.
use crate::{config::Language, messages};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
pub type Session = tokio::sync::oneshot::Receiver<Result<Option<PathBuf>, String>>;

#[cfg(target_os = "macos")]
pub fn closed_message(language: Language) -> &'static str {
    messages::file_dialog_closed(language)
}

#[cfg(target_os = "macos")]
pub fn begin(language: Language) -> Result<Session, String> {
    use objc2::Message;
    use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().ok_or(messages::file_dialog_main_thread(language))?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setCanChooseFiles(true);
    panel.setAllowsMultipleSelection(false);
    let app = NSApplication::sharedApplication(mtm);
    let window = app
        .windows()
        .iter()
        .find(|w| w.title().to_string().starts_with("Clash of Rust"))
        .map(|w| w.retain())
        .ok_or(messages::file_dialog_missing_window(language))?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = std::cell::RefCell::new(Some(sender));
    let selected_panel = panel.clone();
    let block = block2::RcBlock::new(move |response: objc2_app_kit::NSModalResponse| {
        let result = if response == NSModalResponseOK {
            selected_panel
                .URLs()
                .firstObject()
                .and_then(|url| url.path())
                .map(|path| Some(PathBuf::from(path.to_string())))
                .ok_or_else(|| messages::file_dialog_path_unreadable(language).into())
        } else {
            Ok(None)
        };
        // Complete asynchronously: never nest an AppKit run loop inside Slint.
        selected_panel.orderOut(None);
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(result);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &block);
    Ok(receiver)
}

#[cfg(windows)]
pub async fn choose(language: Language) -> Result<Option<PathBuf>, String> {
    tokio::task::spawn_blocking(move || windows_choose(language))
        .await
        .map_err(|e| e.to_string())?
}
#[cfg(windows)]
fn windows_choose(language: Language) -> Result<Option<PathBuf>, String> {
    use windows::Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        UI::Shell::{
            FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FileOpenDialog, IFileOpenDialog,
            SIGDN_FILESYSPATH,
        },
    };
    // A new worker thread owns a single STA and balances initialization on all
    // paths. The GUI stays unprivileged; the chooser never invokes elevation.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        let title: Vec<u16> = messages::file_dialog_title(language)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        dialog
            .SetTitle(windows::core::PCWSTR(title.as_ptr()))
            .map_err(|e| e.to_string())?;
        dialog
            .SetOptions(FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
            .map_err(|e| e.to_string())?;
        if let Err(error) = dialog.Show(None) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(error.to_string());
        }
        let item = dialog.GetResult().map_err(|e| e.to_string())?;
        let raw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        let path = raw.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(raw.0.cast()));
        path.map(|p| Some(PathBuf::from(p)))
    }
}

#[cfg(target_os = "linux")]
pub async fn choose(language: Language) -> Result<Option<PathBuf>, String> {
    async fn open(language: Language) -> anyhow::Result<Option<PathBuf>> {
        use futures_util::StreamExt;
        use zbus::{OwnedObjectPath, OwnedVariant, Proxy, Variant};
        let connection = zbus::Connection::session().await?;
        let token = format!("clash{}", uuid::Uuid::new_v4().simple());
        let sender = connection
            .unique_name()
            .ok_or_else(|| anyhow::anyhow!(messages::file_dialog_no_session(language)))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
        let request = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .await?;
        // Subscribe before OpenFile: a fast selection/cancel must not be lost.
        let mut responses = request.receive_signal("Response").await?;
        let chooser = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.FileChooser",
        )
        .await?;
        let options = std::collections::HashMap::from([
            ("handle_token", Variant::from(token.as_str())),
            ("multiple", Variant::from(false)),
            ("directory", Variant::from(false)),
        ]);
        let handle: OwnedObjectPath = chooser
            .call(
                "OpenFile",
                &("", messages::file_dialog_title(language), options),
            )
            .await?;
        anyhow::ensure!(
            handle.as_str() == path,
            messages::file_dialog_invalid_session(language)
        );
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(600), responses.next()).await;
        let response = match response {
            Ok(Some(response)) => response,
            _ => {
                let _: Result<(), _> = request.call("Close", &()).await;
                anyhow::bail!(messages::file_dialog_timeout(language));
            }
        };
        let (code, mut values): (u32, std::collections::HashMap<String, OwnedVariant>) =
            response.body().deserialize()?;
        if code == 1 {
            return Ok(None);
        }
        anyhow::ensure!(code == 0, messages::file_dialog_unsuccessful(language));
        let uris: Vec<String> = values
            .remove("uris")
            .ok_or_else(|| anyhow::anyhow!(messages::file_dialog_missing_path(language)))?
            .try_into()?;
        let uri = uris
            .first()
            .ok_or_else(|| anyhow::anyhow!(messages::file_dialog_no_selection(language)))?;
        let path = reqwest::Url::parse(uri)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!(messages::file_dialog_requires_local_file(language)))?;
        Ok(Some(path))
    }
    open(language)
        .await
        .map_err(|error| messages::file_dialog_failed(language, format_args!("{error:#}")))
}
