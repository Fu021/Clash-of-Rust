//! Native requests are executed on the desktop event thread by an installed host.
use super::{Size, Task, window};
use futures_util::StreamExt;
use std::sync::OnceLock;

pub trait Host {
    fn current_window(&self) -> Option<window::Id>;
    fn open_window(&mut self, id: window::Id, settings: window::Settings);
    fn window_size(&self, id: window::Id) -> Size;
    fn fit_window_size(&self, id: window::Id, desired: Size) -> Size;
    fn maximized(&self, id: window::Id) -> bool;
    fn minimized(&self, id: window::Id) -> Option<bool>;
    fn window_command(&mut self, id: window::Id, command: window::Command);
    fn read_clipboard(&mut self) -> Option<String>;
    fn write_clipboard(&mut self, text: String);
    fn focus(&mut self, target: &str);
    fn system_theme(&self) -> theme::Mode;
    #[cfg(target_os = "macos")]
    fn pick_file(&mut self) -> Result<clash_of_rust::file_dialog::Session, String> {
        Err("没有可用的界面窗口".into())
    }
    fn quit(&mut self);
}
pub type Call = Box<dyn FnOnce(&mut dyn Host) + Send>;
static DISPATCH: OnceLock<Box<dyn Fn(Call) + Send + Sync>> = OnceLock::new();
pub fn install(dispatch: impl Fn(Call) + Send + Sync + 'static) {
    assert!(
        DISPATCH.set(Box::new(dispatch)).is_ok(),
        "desktop host already installed"
    );
}
pub fn call<T: Send + 'static>(
    operation: impl FnOnce(&mut dyn Host) -> T + Send + 'static,
) -> Task<T> {
    let future = async move {
        let dispatch = DISPATCH.get()?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        dispatch(Box::new(move |host| {
            let _ = sender.send(operation(host));
        }));
        receiver.await.ok()
    };
    Task(
        futures_util::stream::once(future)
            .filter_map(std::future::ready)
            .boxed(),
    )
}
pub mod theme {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Mode {
        Light,
        Dark,
    }
}
pub mod keyboard {
    #[derive(Debug, Clone)]
    pub enum Event {
        KeyPressed { key: Key, modifiers: Modifiers },
    }
    #[derive(Debug, Clone)]
    pub enum Key {
        Character(String),
        Named(key::Named),
    }
    pub mod key {
        #[derive(Debug, Clone, Copy)]
        pub enum Named {
            Escape,
        }
    }
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Modifiers {
        pub command: bool,
    }
    impl Modifiers {
        pub fn command(self) -> bool {
            self.command
        }
    }
}
pub mod clipboard {
    use super::*;
    pub fn read() -> Task<Option<String>> {
        call(|host| host.read_clipboard())
    }
    pub fn write<T: Send + 'static>(text: String) -> Task<T> {
        call(move |host| host.write_clipboard(text)).discard()
    }
}
pub mod system {
    use super::*;
    pub fn theme() -> Task<theme::Mode> {
        call(|host| host.system_theme())
    }
}
pub mod widget {
    pub mod operation {
        use super::super::*;
        pub fn focus<T: Send + 'static>(target: &str) -> Task<T> {
            let target = target.to_owned();
            call(move |host| host.focus(&target)).discard()
        }
    }
}
