//! Real Win32 renderer lifecycle; no Engine, TUN, user configuration or screenshots.
#![cfg(windows)]

use clash_desktop_protocol::{self as wire, HostPacket, UiEvent, ViewState, WindowCommand};
use std::{
    io,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, EnumWindows, GetSystemMetrics, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, PostMessageW,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOWMINNOACTIVE, SendMessageTimeoutW, ShowWindowAsync,
        WM_CLOSE, WM_NULL,
    },
};

struct Renderer {
    child: Child,
    input: ChildStdin,
    events: Receiver<UiEvent>,
}
impl Renderer {
    fn open(view: ViewState) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_clash-desktop-ui"))
            .arg("--desktop-renderer")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        unsafe { AllowSetForegroundWindow(child.id()) };
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(Some(bytes)) = wire::read_frame(&mut output) {
                let event = wire::decode::<UiEvent>(&bytes).unwrap();
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        let mut renderer = Self {
            child,
            input,
            events,
        };
        renderer.send(HostPacket::Open {
            protocol: wire::PROTOCOL,
            version: clash_client_core::VERSION.into(),
            id: 1,
            size: wire::Size {
                width: 950.,
                height: 700.,
            },
            visible: true,
            view: Some(view),
            frame: wire::Frame::default(),
            view_ready: true,
        });
        renderer
    }
    fn send(&mut self, packet: HostPacket) {
        wire::write_frame(&mut self.input, &wire::encode(&packet).unwrap()).unwrap();
    }
    fn command(&mut self, command: WindowCommand) {
        self.send(HostPacket::Command(command));
    }
    fn event(&self, matches: impl Fn(&UiEvent) -> bool) -> UiEvent {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let event = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("renderer did not acknowledge the window operation");
            if let UiEvent::Error(error) = &event {
                panic!("renderer error: {error}");
            }
            if matches(&event) {
                return event;
            }
        }
    }
    fn window(&mut self) -> HWND {
        let pid = self.child.id();
        let mut hwnd = std::ptr::null_mut();
        wait_until(|| {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "renderer exited prematurely"
            );
            hwnd = find_window(pid);
            !hwnd.is_null()
        });
        hwnd
    }
    fn stopped(&mut self) {
        wait_until(|| {
            self.child.try_wait().unwrap().is_some_and(|status| {
                assert!(status.success(), "renderer shutdown failed: {status}");
                true
            })
        });
    }
}
impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[track_caller]
fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "native window operation timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn find_window(pid: u32) -> HWND {
    struct Search {
        pid: u32,
        hwnd: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, data: LPARAM) -> i32 {
        let search = unsafe { &mut *(data as *mut Search) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == search.pid {
            let mut title = [0u16; 128];
            let length = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
            if String::from_utf16_lossy(&title[..length as usize])
                == clash_client_core::WINDOW_TITLE
            {
                search.hwnd = hwnd;
                return 0;
            }
        }
        1
    }
    let mut search = Search {
        pid,
        hwnd: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(visit), (&mut search as *mut Search) as LPARAM) };
    search.hwnd
}
#[track_caller]
fn assert_visible_and_responsive(hwnd: HWND, maximized: bool) {
    wait_until(|| unsafe {
        IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0 && (IsZoomed(hwnd) != 0) == maximized
    });
    let mut rectangle = RECT::default();
    assert_ne!(unsafe { GetWindowRect(hwnd, &mut rectangle) }, 0);
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    assert!(
        rectangle.right > left
            && rectangle.left < left + width
            && rectangle.bottom > top
            && rectangle.top < top + height,
        "window is outside the desktop: {},{}, {},{}",
        rectangle.left,
        rectangle.top,
        rectangle.right,
        rectangle.bottom
    );
    let mut result = 0;
    assert_ne!(
        unsafe { SendMessageTimeoutW(hwnd, WM_NULL, 0, 0, SMTO_ABORTIFHUNG, 2000, &mut result) },
        0,
        "renderer owns a window but is not processing messages: {}",
        io::Error::last_os_error()
    );
}

#[test]
fn recreated_renderer_is_reachable_and_show_restores_native_visibility_without_losing_maximize() {
    for (cycle, (position, maximized)) in
        [((40, 40), false), ((30000, 30000), false), ((40, 40), true)]
            .into_iter()
            .enumerate()
    {
        let started = Instant::now();
        let mut renderer = Renderer::open(ViewState {
            position: Some(position),
            maximized,
            ..Default::default()
        });
        let UiEvent::Opened { metrics, .. } =
            renderer.event(|event| matches!(event, UiEvent::Opened { .. }))
        else {
            unreachable!()
        };
        eprintln!("renderer cycle {cycle}: initialized {metrics:?}");
        assert!(metrics.size.width >= 800. && metrics.size.height >= 450.);
        let hwnd = renderer.window();
        assert_visible_and_responsive(hwnd, maximized);
        // Show must repair the actual native state even when Slint still
        // considers the window visible. This is also the fresh-window path.
        unsafe { ShowWindowAsync(hwnd, SW_SHOWMINNOACTIVE) };
        wait_until(|| unsafe { IsIconic(hwnd) != 0 });
        eprintln!("renderer cycle {cycle}: restore minimized window");
        renderer.command(WindowCommand::Show);
        assert_visible_and_responsive(hwnd, maximized);
        unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
        wait_until(|| unsafe { IsWindowVisible(hwnd) == 0 });
        eprintln!("renderer cycle {cycle}: restore hidden window");
        renderer.command(WindowCommand::Show);
        assert_visible_and_responsive(hwnd, maximized);
        assert_ne!(unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) }, 0);
        renderer.event(|event| matches!(event, UiEvent::Close(_)));
        renderer.command(WindowCommand::Destroy);
        renderer.stopped();
        eprintln!(
            "renderer close/reopen cycle {cycle}: {:?}",
            started.elapsed()
        );
    }
}
