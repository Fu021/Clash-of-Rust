//! Exercise the production chooser against a real isolated D-Bus session,
//! including cancellation and paths that require URI decoding.
#[cfg(windows)]
#[test]
#[ignore = "requires a native Windows desktop; opens and closes the real STA file chooser"]
fn native_file_picker_cancels_without_elevation() {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let _ = sender.send(runtime.block_on(clash_of_rust::file_dialog::choose()));
    });
    let title: Vec<_> = "Clash of Rust · 选择订阅文件\0".encode_utf16().collect();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        unsafe {
            let window = FindWindowW(std::ptr::null(), title.as_ptr());
            if !window.is_null() {
                let mut owner = 0;
                GetWindowThreadProcessId(window, &mut owner);
                assert_eq!(
                    owner,
                    std::process::id(),
                    "only cancel this test's own dialog"
                );
                assert_ne!(PostMessageW(window, WM_CLOSE, 0, 0), 0);
                break;
            }
        }
        assert!(
            Instant::now() < deadline,
            "native file chooser did not appear"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap(),
        None
    );
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
    };
    use zbus::{
        Connection,
        message::Header,
        zvariant::{OwnedObjectPath, OwnedValue, Value},
    };
    struct Portal {
        code: Arc<AtomicU32>,
        uri: String,
    }
    #[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
    impl Portal {
        async fn open_file(
            &self,
            _parent: &str,
            _title: &str,
            options: HashMap<String, OwnedValue>,
            #[zbus(header)] header: Header<'_>,
            #[zbus(connection)] connection: &Connection,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let sender = header
                .sender()
                .unwrap()
                .as_str()
                .trim_start_matches(':')
                .replace('.', "_");
            let token = <&str>::try_from(options.get("handle_token").unwrap()).unwrap();
            let path: OwnedObjectPath =
                format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
                    .try_into()
                    .unwrap();
            let result_path = path.clone();
            let connection = connection.clone();
            let uri = self.uri.clone();
            let code = self.code.load(Ordering::Relaxed);
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                let results = HashMap::from([("uris", Value::from(vec![uri]))]);
                connection
                    .emit_signal(
                        None::<&str>,
                        result_path,
                        "org.freedesktop.portal.Request",
                        "Response",
                        &(code, results),
                    )
                    .await
                    .unwrap();
            });
            Ok(path)
        }
    }
    #[tokio::test]
    #[ignore = "requires dbus-run-session and exclusive ownership of the portal service name"]
    async fn native_portal_selection_and_cancel_preserve_unicode_paths() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("我的 订阅.yaml");
        let code = Arc::new(AtomicU32::new(0));
        let portal = Portal {
            code: code.clone(),
            uri: reqwest::Url::from_file_path(&path).unwrap().to_string(),
        };
        let _server = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.portal.Desktop")
            .unwrap()
            .serve_at("/org/freedesktop/portal/desktop", portal)
            .unwrap()
            .build()
            .await
            .unwrap();
        assert_eq!(
            clash_of_rust::file_dialog::choose().await.unwrap(),
            Some(path)
        );
        code.store(1, Ordering::Relaxed);
        assert_eq!(clash_of_rust::file_dialog::choose().await.unwrap(), None);
        code.store(2, Ordering::Relaxed);
        assert!(clash_of_rust::file_dialog::choose().await.is_err());
    }
}
