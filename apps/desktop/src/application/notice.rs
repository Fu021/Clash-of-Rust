//! Transient notices share one cancellable lifetime, independent of core state.
use super::*;
use std::time::Instant;

pub(super) const LIFETIME: Duration = Duration::from_secs(8);

impl App {
    pub(super) fn show_notice(&mut self, text: String, error: bool) {
        self.notice = text;
        self.error = error;
        self.notice_visible = true;
        self.notice_started = Some(Instant::now());
        self.notice_revision = self.notice_revision.wrapping_add(1);
    }

    pub(super) fn schedule_notice(&mut self, previous: u64, task: Task<Message>) -> Task<Message> {
        if previous == self.notice_revision {
            return task;
        }
        if let Some(timer) = self.notice_timer.take() {
            timer.abort();
        }
        if !self.notice_visible {
            return task;
        }
        let revision = self.notice_revision;
        let (expiry, timer) =
            Task::perform(async { tokio::time::sleep(LIFETIME).await }, move |_| {
                Message::NoticeExpired(revision)
            })
            .abortable();
        self.notice_timer = Some(timer);
        Task::batch([task, expiry])
    }

    pub(super) fn dismiss_notice(&mut self) {
        self.notice_visible = false;
        self.notice_started = None;
        self.notice_revision = self.notice_revision.wrapping_add(1);
        if let Some(timer) = self.notice_timer.take() {
            timer.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeated_notice_restarts_its_lifetime_and_ignores_old_expiry() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.show_notice("已保存".into(), false);
        let old = app.notice_revision;
        app.show_notice("已保存".into(), false);
        let current = app.notice_revision;
        let _ = app.update(Message::NoticeExpired(old));
        assert!(app.notice_visible);
        let _ = app.update(Message::NoticeExpired(current));
        assert!(!app.notice_visible);
        assert!(
            app.core_failure.is_some(),
            "expiring a notice must not erase the core failure"
        );
    }
}
