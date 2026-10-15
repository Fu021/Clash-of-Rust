//! Small effect runner: futures and messages, without any GUI toolkit types.
pub mod native;
pub mod window;
use futures_util::{Stream, StreamExt, stream::BoxStream};
pub use native::{clipboard, keyboard, system, theme, widget};
use std::future::Future;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
impl Size {
    pub const ZERO: Self = Self::new(0.0, 0.0);
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[must_use]
pub struct Task<T>(pub BoxStream<'static, T>);
impl<T: Send + 'static> Task<T> {
    pub fn none() -> Self {
        Self(futures_util::stream::empty().boxed())
    }
    pub fn done(value: T) -> Self {
        Self(futures_util::stream::once(std::future::ready(value)).boxed())
    }
    pub fn perform<A: Send + 'static>(
        future: impl Future<Output = A> + Send + 'static,
        map: impl FnOnce(A) -> T + Send + 'static,
    ) -> Self {
        Self(futures_util::stream::once(async move { map(future.await) }).boxed())
    }
    pub fn run<A: Send + 'static>(
        source: impl Stream<Item = A> + Send + 'static,
        map: impl FnMut(A) -> T + Send + 'static,
    ) -> Self {
        Self(source.map(map).boxed())
    }
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self(futures_util::stream::select_all(tasks.into_iter().map(|task| task.0)).boxed())
    }
    pub fn chain(self, next: Self) -> Self {
        Self(self.0.chain(next.0).boxed())
    }
    pub fn map<U: Send + 'static>(self, map: impl FnMut(T) -> U + Send + 'static) -> Task<U> {
        Task(self.0.map(map).boxed())
    }
    pub fn then<U: Send + 'static>(
        self,
        mut next: impl FnMut(T) -> Task<U> + Send + 'static,
    ) -> Task<U> {
        Task(self.0.flat_map(move |value| next(value).0).boxed())
    }
    pub fn discard<U: Send + 'static>(self) -> Task<U> {
        Task(self.0.filter_map(|_| std::future::ready(None)).boxed())
    }
    pub fn abortable(self) -> (Self, task::Handle) {
        let (handle, registration) = futures_util::future::AbortHandle::new_pair();
        (
            Self(futures_util::stream::Abortable::new(self.0, registration).boxed()),
            handle,
        )
    }
}

pub mod task {
    pub type Handle = futures_util::future::AbortHandle;
}
pub mod stream {
    use super::*;
    pub fn channel<T: Send + 'static, F, Fut>(capacity: usize, producer: F) -> BoxStream<'static, T>
    where
        F: FnOnce(futures_channel::mpsc::Sender<T>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (sender, receiver) = futures_channel::mpsc::channel(capacity);
        let producer = futures_util::stream::once(async move {
            producer(sender).await;
            None
        })
        .filter_map(std::future::ready);
        futures_util::stream::select(receiver, producer).boxed()
    }
}
pub fn exit<T: Send + 'static>() -> Task<T> {
    native::call(|host| host.quit()).discard()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn streaming_effects_interleave_and_abort_without_delivering_stale_results() {
        let (pending, handle) = Task::perform(
            async {
                std::future::pending::<()>().await;
                99
            },
            |value| value,
        )
        .abortable();
        handle.abort();
        let source = stream::channel(1, |mut sender| async move {
            use futures_util::SinkExt;
            sender.send(1).await.unwrap();
            sender.send(2).await.unwrap();
        });
        let messages = Task::batch([pending, Task::run(source, |value| value)])
            .chain(Task::done(3))
            .0
            .collect::<Vec<_>>()
            .await;
        assert_eq!(messages, [1, 2, 3]);
    }
}
