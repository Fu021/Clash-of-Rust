use std::thread::JoinHandle;
use tokio::sync::oneshot;

pub(crate) struct SessionWorker {
    stop: Option<oneshot::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl SessionWorker {
    pub(crate) fn spawn(
        run: impl FnOnce(oneshot::Receiver<()>) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (stop, stopped) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("mobile-runtime".into())
            .spawn(move || run(stopped))?;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
        })
    }
}

impl Drop for SessionWorker {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    struct PendingWrite(Arc<AtomicBool>);

    impl Drop for PendingWrite {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }

    #[test]
    fn recreation_waits_for_cancelled_downloads_and_runtime_cleanup() {
        let writing = Arc::new(AtomicBool::new(false));
        let pending = writing.clone();
        let (ready, started) = mpsc::channel();
        let worker = SessionWorker::spawn(move |stopped| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                tokio::spawn(async move {
                    pending.store(true, Ordering::Release);
                    let _write = PendingWrite(pending);
                    ready.send(()).unwrap();
                    std::future::pending::<()>().await;
                });
                let _ = stopped.await;
            });
        })
        .unwrap();
        started
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(writing.load(Ordering::Acquire));
        drop(worker);
        assert!(!writing.load(Ordering::Acquire));
        let next = SessionWorker::spawn(move |_| {
            assert!(!writing.load(Ordering::Acquire));
        })
        .unwrap();
        drop(next);
    }

    #[test]
    fn finished_worker_can_be_closed_without_a_live_receiver() {
        let (finished, completion) = mpsc::channel();
        let worker = SessionWorker::spawn(move |_| finished.send(()).unwrap()).unwrap();
        completion
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        drop(worker);
    }
}
