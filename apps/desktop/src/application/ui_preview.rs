//! Feature-gated harness. No Engine, network, system proxy or privileges.
use super::*;
use clash_client_ui::{AppWindow, UiActions, UiData, models::ViewModels};
use slint::{ComponentHandle, Timer, TimerMode};

pub(crate) fn run() -> anyhow::Result<()> {
    let scene = std::env::var("CLASH_UI_PREVIEW_SCENE")?;
    let size = if scene.contains("-compact") {
        MIN_WINDOW_SIZE
    } else {
        DEFAULT_WINDOW_SIZE
    };
    let headless = std::env::var_os("CLASH_UI_HEADLESS").is_some();
    if headless {
        ui_testing::initialize(size.width as u32, size.height as u32);
    }
    let app = std::rc::Rc::new(RefCell::new(fixtures::fixture(&scene)));
    let window = AppWindow::new()?;
    window
        .window()
        .set_size(slint::LogicalSize::new(size.width, size.height));
    let models = std::rc::Rc::new(RefCell::new(ViewModels::default()));
    models.borrow().attach(&window.global::<UiData>());
    app.borrow()
        .present(&window.global::<UiData>(), &mut models.borrow_mut());
    let weak = window.as_weak();
    window
        .global::<UiActions>()
        .on_dispatch(move |intent, a, b| {
            // Deliberately drop effects: previews cannot perform native/network actions.
            let _ = app.borrow_mut().intent(intent, a.as_str(), b.as_str());
            if let Some(window) = weak.upgrade() {
                app.borrow()
                    .present(&window.global::<UiData>(), &mut models.borrow_mut());
            }
        });
    window.show()?;
    if let Some(output) = std::env::var_os("CLASH_UI_PREVIEW_OUTPUT") {
        let output = std::path::PathBuf::from(output);
        let capture = move || -> anyhow::Result<()> {
            let pixels = window.window().take_snapshot()?;
            std::fs::write(output.with_extension("rgba"), pixels.as_bytes())?;
            std::fs::write(
                output.with_extension("json"),
                serde_json::to_vec(&[pixels.width(), pixels.height()])?,
            )?;
            Ok(())
        };
        if headless {
            capture()?;
        } else {
            let timer = Timer::default();
            timer.start(TimerMode::SingleShot, Duration::from_secs(2), move || {
                capture().expect("native Slint render capture");
                slint::quit_event_loop().expect("stop capture loop");
            });
            slint::run_event_loop()?;
        }
    } else {
        window.run()?;
    }
    Ok(())
}
