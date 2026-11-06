//! Transient notices share one cancellable lifetime, independent of core state.
use super::*;
use std::time::Instant;

pub(super) const LIFETIME: Duration = Duration::from_secs(8);

impl App {
    pub(super) fn show_notice(&mut self, text: String, error: bool) {
        self.present_notice(text, error, false);
    }

    pub(super) fn show_critical(&mut self, text: String) {
        self.present_notice(text, true, true);
    }

    fn present_notice(&mut self, text: String, error: bool, critical: bool) {
        if error {
            self.issue_events
                .push_back(clash_client_core::support::incident(&text));
            while self.issue_events.len() > 20 {
                self.issue_events.pop_front();
            }
        }
        if self.notice_visible && self.notice_critical && !critical {
            return;
        }
        self.notice = text;
        self.error = error;
        self.notice_critical = critical;
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
        if !self.notice_visible || self.error {
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
        self.notice_critical = false;
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
        app.dismiss_notice();
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

    #[test]
    fn an_error_remains_actionable_and_its_history_is_bounded() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        for index in 0..25 {
            app.show_notice(format!("failure {index}"), true);
        }
        let _ = app.update(Message::NoticeExpired(app.notice_revision));
        assert!(app.notice_visible);
        assert_eq!(app.issue_events.len(), 20);
        assert_eq!(app.issue_events.front().unwrap()["error"], "failure 5");
        app.show_notice("已保存".into(), false);
        assert_eq!(app.issue_events.back().unwrap()["error"], "failure 24");
        let _ = app.update(Message::NoticeExpired(app.notice_revision));
        assert!(!app.notice_visible);
    }

    #[test]
    fn only_critical_errors_block_and_background_results_cannot_replace_them() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        assert!(app.notice_critical && app.error);
        app.dismiss_notice();
        app.show_notice("Geo 更新失败，旧数据仍可用".into(), true);
        assert!(app.error && !app.notice_critical);
        app.show_critical("内核无法启动".into());
        app.show_notice("检查更新失败".into(), true);
        app.show_notice("Geo 数据已更新".into(), false);
        assert_eq!(app.notice, "内核无法启动");
        assert!(app.notice_critical && app.error);
        assert_eq!(app.issue_events.back().unwrap()["error"], "检查更新失败");
        app.dismiss_notice();
        app.show_notice("已保存".into(), false);
        assert!(!app.notice_critical && !app.error);
    }
}
