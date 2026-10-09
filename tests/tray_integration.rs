use clash_of_rust::tray;
use std::time::{Duration, Instant};

fn await_tray(startup: &tray::Startup) -> Result<tray::Guard, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(result) = startup.poll() {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "tray startup exceeded the test deadline"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "requires an interactive desktop session"]
fn native_tray_can_start_and_shutdown() {
    #[cfg(windows)]
    if clash_of_rust::platform::ensure_tray_available().is_err() {
        // Hosted Windows sessions may have no Explorer tray. They must return
        // an error rather than an invisible guard that would hide the GUI.
        let startup = tray::Startup::new(Duration::from_millis(350));
        assert!(await_tray(&startup).is_err());
        return;
    }
    let started = Instant::now();
    let startup = tray::Startup::new(Duration::from_secs(5));
    let guard = await_tray(&startup).expect("native tray should be created");
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(guard);
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    struct Watcher {
        registered: mpsc::Sender<String>,
        host_ready: Arc<AtomicBool>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        fn register_status_notifier_item(&self, service: &str) {
            let _ = self.registered.send(service.to_owned());
        }

        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            self.host_ready.load(Ordering::Acquire)
        }
    }

    #[test]
    #[ignore = "run on an isolated session bus with dbus-run-session"]
    fn login_tray_retries_until_desktop_host_is_ready() {
        assert!(clash_of_rust::platform::ensure_tray_available().is_err());

        let absent = tray::Startup::new(Duration::from_millis(350));
        assert!(
            await_tray(&absent).is_err(),
            "no host must not produce an invisible guard"
        );
        drop(absent);

        let cancelled = tray::Startup::new(Duration::from_secs(5));
        drop(cancelled);

        let startup = tray::Startup::new(Duration::from_secs(5));
        // Emulate a desktop service appearing after the autostart process. This
        // also blocks the caller like cold font/Geo loading, not the tray worker.
        std::thread::sleep(Duration::from_millis(600));
        assert!(startup.poll().is_none());
        let (registered_tx, registered) = mpsc::channel();
        let host_ready = Arc::new(AtomicBool::new(false));
        let _host = zbus::blocking::connection::Builder::session()
            .unwrap()
            .serve_at(
                "/StatusNotifierWatcher",
                Watcher {
                    registered: registered_tx,
                    host_ready: host_ready.clone(),
                },
            )
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .build()
            .unwrap();
        std::thread::sleep(Duration::from_millis(350));
        assert!(
            startup.poll().is_none(),
            "a watcher with no registered host is not a usable tray"
        );
        assert!(registered.try_recv().is_err());
        host_ready.store(true, Ordering::Release);
        let guard = await_tray(&startup).expect("late desktop host must recover automatically");
        let service = registered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!service.is_empty());
        assert!(
            registered.recv_timeout(Duration::from_millis(300)).is_err(),
            "cancelled startup must not register another icon"
        );

        // Verify the host sees the icon, rather than just a successful thread.
        let connection = zbus::blocking::Connection::session().unwrap();
        let item = zbus::blocking::Proxy::new(
            &connection,
            service.as_str(),
            "/StatusNotifierItem",
            "org.kde.StatusNotifierItem",
        )
        .unwrap();
        assert_eq!(
            item.get_property::<String>("Title").unwrap(),
            "Clash of Rust"
        );
        assert_eq!(item.get_property::<String>("Status").unwrap(), "Active");
        item.call::<_, _, ()>("Activate", &(0_i32, 0_i32)).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        assert!(matches!(
            runtime.block_on(async {
                tokio::time::timeout(Duration::from_secs(2), tray::next()).await
            }),
            Ok(Some(tray::Command::Show))
        ));
        drop(guard);
    }
}
