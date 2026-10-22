use super::{Size, Task, native};
use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Id(pub u64);
impl Id {
    pub fn unique() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub size: Size,
    pub visible: bool,
}
#[derive(Debug, Clone, Copy)]
pub enum Event {
    Opened { preserve_native_state: bool },
    Rescaled,
    Resized(Size),
    Focused,
    CloseRequested,
}
pub enum Command {
    Destroy,
    Show,
    Minimize(bool),
    Maximize(bool),
    Focus,
    Resize(Size),
    MinSize(Option<Size>),
}
pub fn latest() -> Task<Option<Id>> {
    native::call(|host| host.current_window())
}
pub fn size(id: Id) -> Task<Size> {
    native::call(move |host| host.window_size(id))
}
pub fn fit_size(id: Id, desired: Size) -> Task<Size> {
    native::call(move |host| host.fit_window_size(id, desired))
}
pub fn is_maximized(id: Id) -> Task<bool> {
    native::call(move |host| host.maximized(id))
}
pub fn is_minimized(id: Id) -> Task<Option<bool>> {
    native::call(move |host| host.minimized(id))
}
pub fn open(settings: Settings) -> (Id, Task<Id>) {
    let id = Id::unique();
    (
        id,
        native::call(move |host| {
            host.open_window(id, settings);
            id
        }),
    )
}
fn command<T: Send + 'static>(id: Id, command: Command) -> Task<T> {
    native::call(move |host| host.window_command(id, command)).discard()
}
pub fn show<T: Send + 'static>(id: Id) -> Task<T> {
    command(id, Command::Show)
}
pub fn destroy<T: Send + 'static>(id: Id) -> Task<T> {
    command(id, Command::Destroy)
}
pub fn minimize<T: Send + 'static>(id: Id, value: bool) -> Task<T> {
    command(id, Command::Minimize(value))
}
pub fn maximize<T: Send + 'static>(id: Id, value: bool) -> Task<T> {
    command(id, Command::Maximize(value))
}
pub fn gain_focus<T: Send + 'static>(id: Id) -> Task<T> {
    command(id, Command::Focus)
}
pub fn resize<T: Send + 'static>(id: Id, value: Size) -> Task<T> {
    command(id, Command::Resize(value))
}
pub fn set_min_size<T: Send + 'static>(id: Id, value: Option<Size>) -> Task<T> {
    command(id, Command::MinSize(value))
}
