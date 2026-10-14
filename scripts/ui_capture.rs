//! Screenshot-only adapter appended to the selected source's existing fixtures.
use super::*;

struct RenderState {
    app: App,
}

#[derive(Debug, Clone)]
enum RenderMessage {
    Take,
    Pixels(iced::window::Screenshot),
}

fn render_update(_: &mut RenderState, message: RenderMessage) -> Task<RenderMessage> {
    match message {
        RenderMessage::Take => iced::window::latest()
            .and_then(|id| iced::window::screenshot(id).map(RenderMessage::Pixels)),
        RenderMessage::Pixels(pixels) => {
            let path =
                std::path::PathBuf::from(std::env::var_os("CLASH_UI_PREVIEW_OUTPUT").unwrap());
            std::fs::write(path.with_extension("rgba"), pixels.as_ref()).unwrap();
            std::fs::write(
                path.with_extension("json"),
                serde_json::to_vec(&[pixels.size.width, pixels.size.height]).unwrap(),
            )
            .unwrap();
            iced::exit()
        }
    }
}

fn render_view(state: &RenderState) -> Element<'_, RenderMessage> {
    state.app.view().map(|_| RenderMessage::Take)
}

pub(crate) fn run_render_capture() -> iced::Result {
    typography::initialize();
    let scene = std::env::var("CLASH_UI_PREVIEW_SCENE").unwrap();
    let size = if scene.contains("-compact") {
        MIN_WINDOW_SIZE
    } else {
        DEFAULT_WINDOW_SIZE
    };
    iced::application(
        move || {
            let mut app = fixture(&scene);
            app.window_width = size.width.to_string();
            app.window_height = size.height.to_string();
            (
                RenderState { app },
                Task::perform(
                    async { tokio::time::sleep(Duration::from_secs(2)).await },
                    |_| RenderMessage::Take,
                ),
            )
        },
        render_update,
        render_view,
    )
    .title("Clash of Rust · UI capture")
    .theme(|state: &RenderState| state.app.theme())
    .default_font(typography::ENGLISH_FONT)
    .settings(iced::Settings {
        default_text_size: iced::Pixels(15.0),
        ..Default::default()
    })
    .window(iced::window::Settings {
        size,
        min_size: Some(MIN_WINDOW_SIZE),
        ..Default::default()
    })
    .run()
}
