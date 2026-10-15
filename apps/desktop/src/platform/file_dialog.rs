//! OS-native YAML selection, loaded only when requested. No GTK UI or browser.
use std::path::PathBuf;

#[cfg(target_os = "macos")]
pub type Session = tokio::sync::oneshot::Receiver<Result<Option<PathBuf>, String>>;

#[cfg(target_os = "macos")]
pub fn begin() -> Result<Session, String> {
    use objc2::Message;
    use objc2_app_kit::{NSApplication, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::MainThreadMarker;
    let mtm = MainThreadMarker::new().ok_or("文件选择器必须在 macOS 主线程运行")?;
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
        .ok_or("没有可附加文件选择器的窗口")?;
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
                .ok_or_else(|| "无法读取所选文件路径".into())
        } else {
            Ok(None)
        };
        // Complete asynchronously: never nest an AppKit run loop inside Iced.
        selected_panel.orderOut(None);
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(result);
        }
    });
    panel.beginSheetModalForWindow_completionHandler(&window, &block);
    Ok(receiver)
}

#[cfg(windows)]
pub async fn choose() -> Result<Option<PathBuf>, String> {
    tokio::task::spawn_blocking(windows_choose)
        .await
        .map_err(|e| e.to_string())?
}
#[cfg(windows)]
fn windows_choose() -> Result<Option<PathBuf>, String> {
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
        dialog
            .SetTitle(windows::core::w!("Clash of Rust · 选择订阅文件"))
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
pub async fn choose() -> Result<Option<PathBuf>, String> {
    async fn open() -> anyhow::Result<Option<PathBuf>> {
        use futures_util::StreamExt;
        use zbus::{
            Proxy,
            zvariant::{OwnedObjectPath, OwnedValue, Value},
        };
        let connection = zbus::Connection::session().await?;
        let token = format!("clash{}", uuid::Uuid::new_v4().simple());
        let sender = connection
            .unique_name()
            .ok_or_else(|| anyhow::anyhow!("桌面会话无连接标识"))?
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
            ("handle_token", Value::from(token.as_str())),
            ("multiple", Value::from(false)),
            ("directory", Value::from(false)),
        ]);
        let handle: OwnedObjectPath = chooser
            .call("OpenFile", &("", "选择订阅 YAML 文件", options))
            .await?;
        anyhow::ensure!(
            handle.as_str() == path,
            "桌面文件选择器返回了不同的会话路径"
        );
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(600), responses.next()).await;
        let response = match response {
            Ok(Some(response)) => response,
            _ => {
                let _: Result<(), _> = request.call("Close", &()).await;
                anyhow::bail!("文件选择超时或桌面会话已结束");
            }
        };
        let (code, mut values): (u32, std::collections::HashMap<String, OwnedValue>) =
            response.body().deserialize()?;
        if code == 1 {
            return Ok(None);
        }
        anyhow::ensure!(code == 0, "桌面文件选择失败");
        let uris: Vec<String> = values
            .remove("uris")
            .ok_or_else(|| anyhow::anyhow!("未返回文件路径"))?
            .try_into()?;
        let uri = uris.first().ok_or_else(|| anyhow::anyhow!("未选择文件"))?;
        let path = reqwest::Url::parse(uri)?
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("请选择本地文件"))?;
        Ok(Some(path))
    }
    open()
        .await
        .map_err(|e| format!("无法打开系统文件选择器：{e:#}；也可直接填写 YAML 路径。"))
}
