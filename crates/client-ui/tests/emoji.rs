use slint::{
    ComponentHandle, Rgb8Pixel,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

slint::slint! {
    import "../../../resources/fonts/SourceHanSansCN-Regular.otf";
    export component EmojiProbe inherits Window {
        width: 512px;
        height: 144px;
        background: #ffffff;
        default-font-family: "Source Han Sans CN";
        default-font-size: 40px;
        in-out property <string> sample;
        in property <color> foreground: #171717;
        in property <float> alpha: 1;
        in property <length> clip-width: root.width;
        Rectangle {
            width: root.clip-width;
            height: parent.height;
            clip: true;
            Text {
                x: 10px; y: 0px;
                width: root.width - 20px; height: 72px;
                text: root.sample; color: root.foreground; opacity: root.alpha;
            }
            TextInput {
                x: 10px; y: 72px;
                width: root.width - 20px; height: 72px;
                text <=> root.sample; color: root.foreground; opacity: root.alpha;
            }
        }
    }
}

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn render(adapter: &MinimalSoftwareWindow, pixels: &mut [Rgb8Pixel], reuse: bool) {
    adapter.window().request_redraw();
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(if reuse {
            RepaintBufferType::ReusedBuffer
        } else {
            RepaintBufferType::NewBuffer
        });
        renderer.render(pixels, 512);
    }));
}

fn colored(pixel: &&Rgb8Pixel) -> bool {
    let channels = [pixel.r, pixel.g, pixel.b];
    channels.iter().max().unwrap() - channels.iter().min().unwrap() > 40
}

#[test]
fn offline_emoji_render_in_labels_and_inputs_with_opacity_clipping_and_partial_redraws() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(Headless(adapter.clone()))).unwrap();
    clash_client_ui::fonts::register_emoji().unwrap();
    let window = EmojiProbe::new().unwrap();
    adapter.set_size(slint::PhysicalSize::new(512, 144));
    window.show().unwrap();
    let mut pixels = vec![Rgb8Pixel::default(); 512 * 144];
    for sample in ["🇺🇸", "🇯🇵", "🇩🇪", "🚀", "😀", "❤️", "👩🏽‍💻", "🇭🇰"]
    {
        window.set_sample(sample.into());
        render(&adapter, &mut pixels, true);
        assert_eq!(window.get_sample(), sample);
        for row in [0, 72] {
            let count = pixels[row * 512..(row + 72) * 512]
                .iter()
                .filter(colored)
                .count();
            assert!(count > 20, "{sample}: missing color glyph in row {row}");
        }
        let mut reference = vec![Rgb8Pixel::default(); pixels.len()];
        render(&adapter, &mut reference, false);
        assert_eq!(
            pixels, reference,
            "{sample}: partial redraw retained old pixels"
        );
    }
    window.set_sample("🚀 🇯🇵 US-日本 01 ❤️ 👩🏽‍💻".into());
    render(&adapter, &mut pixels, false);
    assert_eq!(window.get_sample(), "🚀 🇯🇵 US-日本 01 ❤️ 👩🏽‍💻");
    window.set_sample("🇺🇸".into());
    render(&adapter, &mut pixels, false);
    let opaque = pixels.clone();
    window.set_foreground(slint::Color::from_argb_u8(128, 23, 23, 23));
    render(&adapter, &mut pixels, true);
    assert_ne!(pixels, opaque, "emoji must follow the text color's alpha");
    assert!(pixels.iter().filter(colored).count() > 20);
    for (faded, original) in pixels.iter().zip(&opaque) {
        assert!(
            faded.r.saturating_add(1) >= original.r
                && faded.g.saturating_add(1) >= original.g
                && faded.b.saturating_add(1) >= original.b,
            "half-opacity emoji must blend toward the white background"
        );
    }
    window.set_foreground(slint::Color::from_rgb_u8(23, 23, 23));
    window.set_alpha(0.5);
    render(&adapter, &mut pixels, true);
    assert_ne!(
        pixels, opaque,
        "emoji must follow the parent item's opacity"
    );
    window.set_alpha(1.0);
    window.set_clip_width(28.0);
    render(&adapter, &mut pixels, true);
    assert!(
        pixels
            .iter()
            .enumerate()
            .all(|(index, pixel)| { index % 512 < 28 || *pixel == Rgb8Pixel::new(255, 255, 255) })
    );
    assert!(pixels.iter().filter(colored).count() > 20);
    window.hide().unwrap();
}
