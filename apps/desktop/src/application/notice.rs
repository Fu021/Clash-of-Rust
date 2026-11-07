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

    pub(super) fn show_unreachable_nodes(&mut self, context: &str, count: usize) {
        let detail = if self.settings.claude_protection.is_some() {
            "请在 Claude 保护页检查保护线路。"
        } else if self.settings.proxy_mode == ProxyMode::Tun {
            "更新会先尝试代理，失败后尝试独立直连，无需关闭 TUN；其他应用仍可能无法联网。"
        } else {
            "更新会先尝试代理，失败后尝试独立直连，无需关闭系统代理；其他应用仍可能无法联网。"
        };
        let revision = self.notice_revision;
        self.show_notice(
            format!("{context}的 {count} 个节点均未通过测速。{detail}"),
            true,
        );
        if self.notice_revision != revision {
            self.notice_network_recovery = true;
        }
    }

    pub(super) fn show_notice_followup(&mut self, text: String, error: bool) {
        if self.notice_visible && self.error {
            self.present_notice(
                format!("{}\n\n{text}", self.notice),
                true,
                self.notice_critical,
            );
        } else {
            self.show_notice(text, error);
        }
    }

    fn present_notice(&mut self, text: String, error: bool, critical: bool) {
        if error {
            self.issue_events
                .push_back(clash_client_core::support::incident(&text));
            while self.issue_events.len() > 20 {
                self.issue_events.pop_front();
            }
        }
        if self.notice_visible && ((self.notice_critical && !critical) || (self.error && !error)) {
            return;
        }
        self.notice = text;
        self.error = error;
        self.notice_critical = critical;
        self.notice_network_recovery = false;
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
        self.notice_network_recovery = false;
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
        assert_eq!(app.notice, "failure 24");
        let _ = app.update(Message::NoticeExpired(app.notice_revision));
        assert!(app.notice_visible);
        app.dismiss_notice();
        app.show_notice("已保存".into(), false);
        let _ = app.update(Message::NoticeExpired(app.notice_revision));
        assert!(!app.notice_visible);
    }

    #[test]
    fn unreachable_nodes_offer_recovery_without_changing_the_network() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        let previous = app.settings.clone();
        app.show_unreachable_nodes("本次测速", 3);
        assert!(app.error && app.notice_network_recovery);
        assert!(app.notice.contains("3 个节点均未通过测速"));
        assert_eq!(app.settings.proxy_mode, previous.proxy_mode);
        assert_eq!(app.settings.run_mode, previous.run_mode);
        app.dismiss_notice();
        assert!(!app.notice_network_recovery);
    }

    #[test]
    fn only_critical_errors_block_and_background_results_cannot_replace_them() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        assert!(app.notice_critical && app.error);
        app.dismiss_notice();
        app.show_notice("Geo 更新失败，旧数据仍可用".into(), true);
        assert!(app.error && !app.notice_critical);
        app.show_notice_followup("诊断已导出：测试路径".into(), false);
        assert!(app.notice.starts_with("Geo 更新失败"));
        assert!(app.notice.contains("诊断已导出"));
        assert!(app.error && !app.notice_critical);
        app.show_critical("内核无法启动".into());
        app.show_notice("检查更新失败".into(), true);
        app.show_notice("Geo 数据已更新".into(), false);
        assert_eq!(app.notice, "内核无法启动");
        assert!(app.notice_critical && app.error);
        assert_eq!(app.issue_events.back().unwrap()["error"], "检查更新失败");
        app.show_notice_followup("诊断导出失败：测试磁盘不可写".into(), true);
        assert!(app.notice.starts_with("内核无法启动"));
        assert!(app.notice.contains("诊断导出失败"));
        assert!(app.notice_critical);
        app.dismiss_notice();
        app.show_notice("已保存".into(), false);
        assert!(!app.notice_critical && !app.error);
    }
}
