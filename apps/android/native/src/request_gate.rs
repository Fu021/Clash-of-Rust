use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub(crate) struct RequestGate {
    next: AtomicU64,
    pending: AtomicU64,
}

impl RequestGate {
    pub(crate) fn begin(&self) -> Option<u64> {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        self.pending
            .compare_exchange(0, ticket, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ticket)
    }

    pub(crate) fn finish(&self, ticket: u64) -> bool {
        self.pending
            .compare_exchange(ticket, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn cancel(&self) {
        self.pending.store(0, Ordering::Release);
    }

    pub(crate) fn busy(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_submissions_and_stale_completions_cannot_replace_a_pending_request() {
        let gate = RequestGate::default();
        let first = gate.begin().unwrap();
        assert!(gate.busy());
        assert!(gate.begin().is_none());
        gate.finish(first);
        assert!(!gate.busy());
        let second = gate.begin().unwrap();
        gate.finish(first);
        assert!(gate.busy());
        assert!(gate.begin().is_none());
        gate.finish(second);
        assert!(!gate.busy());
    }

    #[test]
    fn queued_geo_result_is_discarded_after_cancellation_even_after_a_new_download_starts() {
        let gate = RequestGate::default();
        let (updates, results) = std::sync::mpsc::sync_channel(1);
        let ticket = gate.begin().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let staged_path = stage.path().to_owned();
        updates.send((ticket, stage)).unwrap();

        gate.cancel();
        let retry = gate.begin().unwrap();
        let (queued_ticket, stage) = results.recv().unwrap();
        assert!(!gate.finish(queued_ticket));
        drop(stage);
        assert!(!staged_path.exists());
        assert!(gate.busy());
        assert!(gate.finish(retry));
        assert!(!gate.busy());
    }
}
