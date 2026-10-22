//! Slint is confined to this process and its testable data adapter.
use clash_client_ui as ui;
use clash_desktop_protocol as wire;
use slint::{Model, ModelRc, VecModel};
pub mod flags;
mod process;
mod repaint;
mod view_state;
pub use process::run;
fn list<T: Clone + PartialEq + 'static>(old: ModelRc<T>, rows: Vec<T>) -> ModelRc<T> {
    if old.row_count() == rows.len()
        && rows
            .iter()
            .enumerate()
            .all(|(i, v)| old.row_data(i).as_ref() == Some(v))
    {
        old
    } else {
        ModelRc::new(VecModel::from(rows))
    }
}
include!(concat!(env!("OUT_DIR"), "/bridge.rs"));
pub fn invalidate_window(window: &slint::Window) {
    repaint::invalidate(window);
}

/// An in-flight frame must not overwrite input typed after it was produced.
pub fn apply_if_current(
    frame: &wire::Frame,
    data: &ui::UiData<'_>,
    models: &mut ui::models::ViewModels,
    accepted: u64,
    current: u64,
) -> bool {
    if accepted < current {
        return false;
    }
    apply(frame, data, models);
    true
}
