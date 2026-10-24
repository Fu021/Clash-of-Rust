//! Exercise the production mobile virtual-list wrapper with a software window.
use slint::{
    ComponentHandle,
    platform::{
        Platform, PlatformError, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

slint::slint! {
    import { ListView } from "../ui/mobile/shared/controls.slint";

    export component MobileListProbe inherits Window {
        preferred-width: 300px; preferred-height: 240px;
        in property <[float]> rows;
        out property <length> extent <=> list.content-height;
        list := ListView {
            width: parent.width; height: parent.height;
            for row[index] in root.rows: Rectangle {
                height: row * 1px;
                background: index == 0 ? #ff0000 : index == 1 ? #00ff00 : #0000ff;
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

fn frame(adapter: &MinimalSoftwareWindow, width: usize, height: usize) -> Vec<slint::Rgb8Pixel> {
    let mut pixels = vec![slint::Rgb8Pixel::default(); width * height];
    adapter.window().request_redraw();
    assert!(adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width);
    }));
    pixels
}

#[test]
fn mobile_list_stacks_rows_and_keeps_sparse_content_at_the_top() {
    let adapter = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(adapter.clone()))).unwrap();
    adapter.set_size(slint::PhysicalSize::new(300, 240));
    let probe = MobileListProbe::new().unwrap();
    let rows = Rc::new(slint::VecModel::from(vec![48.; 20]));
    probe.set_rows(rows.clone().into());
    probe.show().unwrap();

    let pixels = frame(&adapter, 300, 240);
    assert!(
        probe.get_extent() >= 20. * 48.,
        "all rows must contribute to the scrollable content extent"
    );
    assert_eq!(pixels[10 * 300 + 10], slint::Rgb8Pixel::new(255, 0, 0));
    assert_eq!(pixels[58 * 300 + 10], slint::Rgb8Pixel::new(0, 255, 0));
    assert_eq!(pixels[106 * 300 + 10], slint::Rgb8Pixel::new(0, 0, 255));

    rows.set_vec(vec![48.]);
    let pixels = frame(&adapter, 300, 240);
    assert_eq!(pixels[10 * 300 + 10], slint::Rgb8Pixel::new(255, 0, 0));
    assert!(
        probe.get_extent() < 100.,
        "removed rows must release their scroll extent"
    );

    adapter.set_size(slint::PhysicalSize::new(900, 240));
    assert_eq!(probe.window().size(), slint::PhysicalSize::new(900, 240));
    rows.set_vec(vec![40., 64., 80.]);
    let pixels = frame(&adapter, 900, 240);
    assert_eq!(pixels[5 * 900 + 700], slint::Rgb8Pixel::new(255, 0, 0));
    assert_eq!(pixels[50 * 900 + 700], slint::Rgb8Pixel::new(0, 255, 0));
    assert_eq!(pixels[110 * 900 + 700], slint::Rgb8Pixel::new(0, 0, 255));
}
