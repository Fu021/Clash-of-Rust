//! Transient notices share one cancellable lifetime, independent of core state.
use super::*;
use std::time::Instant;

pub(super) const LIFETIME: Duration = Duration::from_secs(5);

fn append_notice_text(notice: &mut String, text: &str) {
    if !notice.is_empty() {
        notice.push_str("\n\n");
    }
    notice.push_str(text);
}

pub(super) fn localize_notice(text: &str, error: bool, language: Language) -> Cow<'_, str> {
    if language == Language::English
        && let Some(translated) = english_desktop_notice(text)
    {
        return Cow::Owned(translated);
    }
    if error {
        clash_client_core::error_message::localize(text, language)
    } else {
        Cow::Borrowed(text)
    }
}

pub(super) fn localize_update_message(message: &str, language: Language) -> Cow<'_, str> {
    if language == Language::English
        && let Some(reason) = message.strip_prefix("更新失败：")
    {
        let english = match reason {
            "客户端无法正常退出，请检查代理恢复或内核状态" => {
                "the client could not exit. Check proxy recovery and core status."
            }
            "安装助手无法启动，请重试" => {
                "Could not start the installation helper; try again"
            }
            clash_client_core::protection::READ_ONLY_NOTICE => {
                return format!("Update failed: {}", localize_notice(reason, true, language))
                    .into();
            }
            _ => return update::localize_installation_message(message, language),
        };
        return format!("Update failed: {english}").into();
    }
    update::localize_installation_message(message, language)
}

pub(super) struct UnreachableNotice {
    context: Option<String>,
    count: usize,
    protected: bool,
}

impl UnreachableNotice {
    pub(super) fn display(&self, language: Language) -> String {
        let context = self
            .context
            .as_deref()
            .unwrap_or_else(|| language.text("本次定时测速", "Scheduled latency test"));
        let detail = if self.protected {
            language.text(
                "请检查 Claude 保护线路。",
                "Check the Claude protection route.",
            )
        } else {
            language.text(
                "请更换节点；更新失败后会自动尝试直连。",
                "Choose another node; failed updates will retry using a direct connection.",
            )
        };
        match language {
            Language::SimplifiedChinese => {
                format!("{context}：{} 个节点不可达。{detail}", self.count)
            }
            Language::English => format!("{context}: {} nodes unreachable. {detail}", self.count),
        }
    }
}

impl App {
    pub(super) fn fail_update(&mut self, reason: String) {
        let directory = self.updates.install_directory.clone();
        self.installation_notice = None;
        self.updates.fail(reason);
        self.pending_update_failure = self.updates.failure.clone();
        self.restore_update_failure();
        if let Some(directory) = directory {
            self.read_installation_notice(Some(&directory));
        }
    }

    pub(super) fn update_outcome(&mut self, message: String, error: bool) {
        self.installation_notice = None;
        if error {
            self.pending_update_failure = Some(message.clone());
            self.updates.failure = Some(message);
            self.restore_update_failure();
        } else {
            self.updates.outcome = Some(message);
        }
    }

    pub(super) fn read_installation_notice(&mut self, directory: Option<&std::path::Path>) {
        if let Some(root) = self.installation_root.clone() {
            self.load_installation_notice(&root, directory);
        }
    }

    fn load_installation_notice(
        &mut self,
        data_dir: &std::path::Path,
        directory: Option<&std::path::Path>,
    ) {
        let status = match update::installation_status(data_dir) {
            Ok(Some(status)) if status.needs_notification() => status,
            Ok(_) => return,
            Err(error) => {
                self.show_notice_followup(format!("读取上次安装结果失败：{error:#}"), true);
                return;
            }
        };
        if directory.is_some_and(|directory| !status.matches_directory(directory))
            || (directory.is_none()
                && (self.updates.outcome.is_some()
                    || self
                        .pending_update_failure
                        .as_ref()
                        .is_some_and(|failure| failure != &status.message)))
        {
            return;
        }
        if self.pending_update_failure.as_ref() != Some(&status.message) {
            self.update_outcome(status.message.clone(), true);
        }
        self.installation_notice = Some((data_dir.to_owned(), status));
    }

    pub(super) fn acknowledge_update_failure(&mut self) -> bool {
        let Some((root, status)) = &self.installation_notice else {
            return true;
        };
        match status.acknowledge(root) {
            Ok(_) => {
                self.installation_notice = None;
                true
            }
            Err(error) => {
                let detail = format!("无法保存安装失败确认：{error:#}；请稍后重试关闭提示。");
                if self.notice != status.message
                    || self.notice_followups.as_slice() != std::slice::from_ref(&detail)
                {
                    self.notice.clone_from(&status.message);
                    self.notice_followups.clear();
                    self.show_notice_followup(detail, true);
                }
                self.notice_update_failure = true;
                false
            }
        }
    }

    pub(super) fn restore_update_failure(&mut self) {
        if let Some(message) = self.pending_update_failure.clone() {
            let revision = self.notice_revision;
            self.show_notice(message, true);
            if self.notice_revision != revision {
                self.notice_update_failure = true;
            }
        }
    }

    pub(super) fn show_notice(&mut self, text: String, error: bool) {
        self.present_notice(text, error, false, false);
    }

    pub(super) fn show_critical(&mut self, text: String) {
        self.present_notice(text, true, true, false);
    }

    pub(super) fn show_copy_failure(&mut self) {
        if self.notice_visible {
            self.record_notice_issue(
                desktop_view::CopyFeedback::Failed.text(Language::SimplifiedChinese),
                false,
            );
            self.notice_copy_failed = true;
        } else {
            self.present_notice(String::new(), true, false, true);
        }
    }

    pub(super) fn show_unreachable_nodes(&mut self, context: &str, count: usize) {
        self.show_unreachable_notice(Some(context.to_owned()), count);
    }

    pub(super) fn show_scheduled_unreachable_nodes(&mut self, count: usize) {
        self.show_unreachable_notice(None, count);
    }

    fn show_unreachable_notice(&mut self, context: Option<String>, count: usize) {
        let notice = UnreachableNotice {
            context,
            count,
            protected: self.settings.claude_protection.is_some(),
        };
        let revision = self.notice_revision;
        self.show_notice(notice.display(Language::SimplifiedChinese), true);
        if self.notice_revision != revision {
            self.notice_network_recovery = true;
            self.unreachable_notice = Some(notice);
        }
    }

    pub(super) fn update_unreachable_notice(&mut self, text: String) {
        if self.notice_visible && self.notice_network_recovery {
            self.present_notice(text, true, false, self.notice_copy_failed);
            self.notice_network_recovery = true;
        }
    }

    pub(super) fn show_notice_followup(&mut self, text: String, error: bool) {
        if self.notice_visible && self.error {
            if error {
                self.record_notice_issue(&text, self.notice_critical);
            }
            self.notice_followups.push(text);
            self.notice_network_recovery = false;
            self.notice_started = Some(Instant::now());
            self.notice_revision = self.notice_revision.wrapping_add(1);
        } else {
            self.show_notice(text, error);
        }
    }

    #[cfg(test)]
    pub(super) fn raw_notice(&self) -> Cow<'_, str> {
        if self.notice_followups.is_empty() {
            return Cow::Borrowed(&self.notice);
        }
        let mut text = self.notice.clone();
        for followup in &self.notice_followups {
            append_notice_text(&mut text, followup);
        }
        Cow::Owned(text)
    }

    pub(super) fn display_notice(&self) -> Cow<'_, str> {
        self.display_notice_in(self.presentation_language)
    }

    pub(super) fn display_notice_in(&self, language: Language) -> Cow<'_, str> {
        if !self.notice_visible {
            return Cow::Borrowed("");
        }
        let primary = if self.notice_update_failure {
            localize_update_message(&self.notice, language)
        } else if let Some(notice) = &self.unreachable_notice {
            Cow::Owned(notice.display(language))
        } else {
            localize_notice(&self.notice, self.error, language)
        };
        if self.notice_followups.is_empty() && !self.notice_copy_failed {
            return primary;
        }
        let mut text = primary.into_owned();
        for followup in &self.notice_followups {
            append_notice_text(&mut text, &localize_notice(followup, true, language));
        }
        if self.notice_copy_failed {
            append_notice_text(&mut text, desktop_view::CopyFeedback::Failed.text(language));
        }
        Cow::Owned(text)
    }

    fn record_notice_issue(&mut self, text: &str, critical: bool) {
        self.issue_events
            .push_back(clash_client_core::support::incident(text, critical));
        while self.issue_events.len() > 20 {
            self.issue_events.pop_front();
        }
    }

    fn present_notice(&mut self, text: String, error: bool, critical: bool, copy_failed: bool) {
        if error {
            let issue = if copy_failed && text.is_empty() {
                desktop_view::CopyFeedback::Failed.text(Language::SimplifiedChinese)
            } else {
                &text
            };
            self.record_notice_issue(issue, critical);
        }
        if self.notice_visible && ((self.notice_critical && !critical) || (self.error && !error)) {
            return;
        }
        self.notice = text;
        self.notice_followups.clear();
        self.error = error;
        self.notice_critical = critical;
        self.notice_network_recovery = false;
        self.unreachable_notice = None;
        self.notice_update_failure = false;
        self.notice_copy_failed = copy_failed;
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
        self.unreachable_notice = None;
        self.notice_update_failure = false;
        self.notice_copy_failed = false;
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

    fn apply_language_setting(app: &mut App, language: LanguagePreference) {
        app.updates.started = true;
        let _ = app.update(Message::Finished(Box::new(Reply {
            view_revision: app.view_revision,
            remote: false,
            profile_edit: false,
            profile_import: false,
            profile_updated: false,
            scope: Scope::Other,
            start_requested: false,
            settings: Settings {
                language,
                ..app.settings.clone()
            },
            profiles: None,
            rule_document: None,
            rules_applied: false,
            snapshot: None,
            notice: Ok(String::new()),
            running: false,
            geo_status: String::new(),
            autostart: false,
            protection_changed: false,
            tray_nodes: None,
        })));
    }

    #[test]
    fn clipboard_feedback_follows_language_changes_without_another_copy() {
        use clash_desktop_protocol::{UiData, models::ViewModels};

        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.page = Page::Claude;
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);
        for (revision, script, written) in [(4, true, true), (5, true, false), (6, false, false)] {
            app.dismiss_notice();
            app.claude.status = "Existing Claude status".into();
            app.claude.error = false;
            apply_language_setting(&mut app, LanguagePreference::SimplifiedChinese);
            app.finish_copy(revision, script, written);

            for (language, copied, failure) in [
                (
                    LanguagePreference::SimplifiedChinese,
                    "脚本已复制",
                    "无法写入系统剪贴板，请重试。",
                ),
                (
                    LanguagePreference::English,
                    "Script copied",
                    "Could not write to the system clipboard. Please try again.",
                ),
                (
                    LanguagePreference::SimplifiedChinese,
                    "脚本已复制",
                    "无法写入系统剪贴板，请重试。",
                ),
            ] {
                let notice_revision = app.notice_revision;
                apply_language_setting(&mut app, language);
                app.present(&data, &mut models);
                assert_eq!(data.get_copy_revision(), revision);
                assert_eq!(data.get_copy_failed(), !written);
                assert_eq!(data.get_claude_error(), script && !written);
                if !script {
                    assert_eq!(data.get_claude_status().as_str(), "Existing Claude status");
                } else if written {
                    let status = data.get_claude_status();
                    assert!(
                        status.as_str().starts_with(copied),
                        "{language:?}: {status:?}"
                    );
                } else {
                    assert_eq!(data.get_claude_status().as_str(), failure);
                }
                assert_eq!(
                    data.get_notice().as_str(),
                    if written { "" } else { failure }
                );
                assert_eq!(app.notice_revision, notice_revision);
            }
        }
    }

    #[test]
    fn clipboard_failure_preserves_the_active_notice_and_its_actions() {
        use clash_desktop_protocol::{UiData, models::ViewModels};

        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);
        type NoticeCase = (&'static str, fn(&mut App));
        let cases: [NoticeCase; 4] = [
            ("3 nodes unreachable", |app| {
                app.show_unreachable_nodes("Test", 3)
            }),
            ("被占用", |app| app.fail_update("安装文件被占用".into())),
            ("Critical test notice", |app| {
                app.show_critical("Critical test notice".into())
            }),
            ("Saved test settings", |app| {
                app.show_notice("Saved test settings".into(), false)
            }),
        ];
        for (original_english, show_notice) in cases {
            app.dismiss_notice();
            apply_language_setting(&mut app, LanguagePreference::SimplifiedChinese);
            show_notice(&mut app);
            let original = app.notice.clone();
            let revision = app.notice_revision;
            let started = app.notice_started;
            let flags = (
                app.error,
                app.notice_critical,
                app.notice_network_recovery,
                app.notice_update_failure,
            );
            app.finish_copy(11, false, false);
            app.finish_copy(12, false, false);
            assert_eq!(app.notice, original);
            assert_eq!(app.notice_revision, revision);
            assert_eq!(app.notice_started, started);
            assert_eq!(
                (
                    app.error,
                    app.notice_critical,
                    app.notice_network_recovery,
                    app.notice_update_failure
                ),
                flags
            );

            apply_language_setting(&mut app, LanguagePreference::English);
            app.present(&data, &mut models);
            let message = data.get_notice();
            assert!(message.as_str().contains(original_english), "{message:?}");
            assert_eq!(
                message
                    .as_str()
                    .matches("Could not write to the system clipboard. Please try again.")
                    .count(),
                1
            );
            app.dismiss_notice();
            app.show_notice("New test notice".into(), false);
            app.present(&data, &mut models);
            assert_eq!(data.get_notice().as_str(), "New test notice");
        }
    }

    fn stored_installation_failure(
        data_dir: &std::path::Path,
        message: &str,
    ) -> std::path::PathBuf {
        let directory = data_dir.join("finished-update");
        std::fs::write(
            data_dir.join("update-install.json"),
            serde_json::to_vec(&serde_json::json!({
                "version":"0.7.2-fix.4.example", "state":"failed",
                "message":message, "error":true, "updated_at":1,
                "directory":directory,
            }))
            .unwrap(),
        )
        .unwrap();
        directory
    }

    #[test]
    fn manually_reopening_recovers_an_install_failure_until_its_visible_notice_is_confirmed() {
        let data = tempfile::tempdir().unwrap();
        let failure = "更新失败：安装程序退出（代码 68）";
        stored_installation_failure(data.path(), failure);
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.installation_root = Some(data.path().to_owned());
        app.prepare_initialization(None);
        app.read_installation_notice(None);
        assert_eq!(app.notice, failure);
        assert!(app.notice_visible && app.error && !app.notice_critical);
        app.show_critical("内核无法启动".into());
        let _ = app.update(Message::DismissNotice);
        assert_eq!(app.notice, failure);
        assert!(
            update::installation_status(data.path())
                .unwrap()
                .unwrap()
                .needs_notification()
        );
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible && app.updates.failure.is_none());
        let status = update::installation_status(data.path()).unwrap().unwrap();
        assert!(!status.needs_notification());
        assert_eq!(status.message, failure);
        let mut reopened = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        reopened.installation_root = Some(data.path().to_owned());
        reopened.prepare_initialization(None);
        reopened.read_installation_notice(None);
        assert!(!reopened.notice_visible && reopened.updates.failure.is_none());
    }

    #[test]
    fn old_install_records_cannot_replace_an_explicit_new_startup_result() {
        let data = tempfile::tempdir().unwrap();
        stored_installation_failure(data.path(), "上一次安装失败");
        for error in [false, true] {
            let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
            app.installation_root = Some(data.path().to_owned());
            app.prepare_initialization(Some(("本次安装结果".into(), error)));
            app.read_installation_notice(None);
            assert!(app.installation_notice.is_none());
            if error {
                assert_eq!(app.notice, "本次安装结果");
                assert_eq!(app.updates.failure.as_deref(), Some("本次安装结果"));
            } else {
                assert!(!app.notice_visible);
                assert_eq!(app.updates.outcome.as_deref(), Some("本次安装结果"));
            }
        }
    }

    #[test]
    fn failed_confirmation_preserves_the_notice_and_allows_retry_without_repeating_details() {
        let data = tempfile::tempdir().unwrap();
        let failure = "更新失败：安装目录不可写";
        stored_installation_failure(data.path(), failure);
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.installation_root = Some(data.path().to_owned());
        app.prepare_initialization(None);
        app.read_installation_notice(None);
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.path().join("update-install.lock"))
            .unwrap();
        lock.lock().unwrap();
        let previous_events = app.issue_events.len();
        for _ in 0..3 {
            let _ = app.update(Message::DismissNotice);
            assert!(app.notice_visible && app.error && app.notice_update_failure);
            assert!(app.notice.starts_with(failure));
            assert_eq!(app.raw_notice().matches("无法保存安装失败确认").count(), 1);
            assert_eq!(app.issue_events.len(), previous_events + 1);
        }
        assert!(
            update::installation_status(data.path())
                .unwrap()
                .unwrap()
                .needs_notification()
        );
        drop(lock);
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible && app.updates.failure.is_none());
        assert!(
            !update::installation_status(data.path())
                .unwrap()
                .unwrap()
                .needs_notification()
        );
    }

    #[test]
    fn english_failed_confirmation_localizes_the_record_and_preserves_it_for_retry() {
        let data = tempfile::tempdir().unwrap();
        let failure = "更新失败：安装授权已取消";
        stored_installation_failure(data.path(), failure);
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.presentation_language = Language::English;
        app.installation_root = Some(data.path().to_owned());
        app.prepare_initialization(None);
        app.read_installation_notice(None);
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.path().join("update-install.lock"))
            .unwrap();
        lock.lock().unwrap();
        let _ = app.update(Message::DismissNotice);
        assert!(
            app.display_notice()
                .starts_with("Update failed: Installation authorization cancelled")
        );
        assert!(
            app.display_notice()
                .contains("Could not acknowledge the installation failure")
        );
        assert_eq!(app.notice, failure);
        assert_eq!(
            update::installation_status(data.path())
                .unwrap()
                .unwrap()
                .message,
            failure
        );
        drop(lock);
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible && app.updates.failure.is_none());
    }

    #[test]
    fn only_the_matching_installation_can_supply_the_handoff_failure_record() {
        let data = tempfile::tempdir().unwrap();
        let failure = "更新失败：安装程序退出（代码 68）";
        let directory = stored_installation_failure(data.path(), failure);
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.installation_root = Some(data.path().to_owned());
        app.prepare_initialization(None);
        app.updates.install_directory = Some(data.path().join("another-update"));
        app.fail_update("本次助手失败".into());
        assert!(app.installation_notice.is_none());
        assert_eq!(app.notice, "更新失败：本次助手失败");
        app.updates.install_directory = Some(directory);
        app.fail_update(failure.into());
        assert!(app.installation_notice.is_some());
        assert_eq!(app.notice, failure);
        assert_eq!(app.updates.failure.as_deref(), Some(failure));
    }

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
        assert!(app.notice.contains("3 个节点不可达"));
        assert_eq!(app.settings.proxy_mode, previous.proxy_mode);
        assert_eq!(app.settings.run_mode, previous.run_mode);
        app.dismiss_notice();
        assert!(!app.notice_network_recovery);
    }

    #[test]
    fn node_confirmation_replaces_its_warning_without_reopening_or_overwriting_other_errors() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        app.show_unreachable_nodes("本次测速", 3);
        app.update_unreachable_notice("全部节点不可达，TUN 仍开启。".into());
        assert_eq!(app.notice, "全部节点不可达，TUN 仍开启。");
        assert!(app.error && app.notice_network_recovery);
        app.show_notice_followup("诊断已导出：测试路径".into(), false);
        let diagnostic = app.raw_notice().into_owned();
        app.update_unreachable_notice("迟到的节点复测结果".into());
        assert_eq!(app.raw_notice(), diagnostic);
        app.dismiss_notice();
        app.update_unreachable_notice("关闭后的节点复测结果".into());
        assert!(!app.notice_visible);
        app.show_critical("内核无法启动".into());
        app.update_unreachable_notice("迟到的节点复测失败".into());
        assert_eq!(app.notice, "内核无法启动");
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
        assert!(app.raw_notice().contains("诊断已导出"));
        assert!(app.error && !app.notice_critical);
        app.show_critical("内核无法启动".into());
        app.show_notice("检查更新失败".into(), true);
        app.show_notice("Geo 数据已更新".into(), false);
        assert_eq!(app.notice, "内核无法启动");
        assert!(app.notice_critical && app.error);
        assert_eq!(app.issue_events.back().unwrap()["error"], "检查更新失败");
        app.show_notice_followup("诊断导出失败：测试磁盘不可写".into(), true);
        assert!(app.notice.starts_with("内核无法启动"));
        assert!(app.raw_notice().contains("诊断导出失败"));
        assert!(app.notice_critical);
        app.dismiss_notice();
        app.show_notice("已保存".into(), false);
        assert!(!app.notice_critical && !app.error);
    }

    #[test]
    fn update_failures_have_recoverable_diagnostics_and_cannot_hide_a_critical_error() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        app.fail_update("安装文件被占用".into());
        assert!(app.error && app.notice_visible && !app.notice_critical);
        assert_eq!(app.notice, app.updates.failure.clone().unwrap());
        assert_eq!(app.issue_events.back().unwrap()["error"], app.notice);
        let _ = app.update(Message::NoticeExpired(app.notice_revision));
        assert!(app.notice_visible);
        app.show_critical("内核无法启动".into());
        app.update_outcome("更新失败：没有写入权限".into(), true);
        assert!(app.notice_critical);
        assert_eq!(app.notice, "内核无法启动");
        app.update_outcome("安装成功".into(), false);
        assert!(app.error && app.notice_critical);
        assert!(app.updates.failure.as_ref().unwrap().contains("写入权限"));
    }

    #[test]
    fn dismissing_a_critical_error_reveals_the_unacknowledged_install_failure() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("内核无法启动")), None);
        app.update_outcome("更新失败：安装程序退出（代码 68）".into(), true);
        assert!(app.notice_critical);
        assert!(!app.notice_update_failure);
        let _ = app.update(Message::DismissNotice);
        assert!(app.notice_visible && app.error && !app.notice_critical);
        assert!(app.notice_update_failure);
        assert!(app.notice.contains("代码 68"));
        assert!(app.updates.failure.is_some());
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible);
        assert!(app.updates.failure.is_none());
        assert!(app.core_failure.is_some());
    }

    #[test]
    fn update_state_resets_do_not_consume_the_pending_install_warning() {
        for protection in [false, true] {
            let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
            let failure = "更新失败：安装程序退出（代码 68）";
            app.prepare_initialization(Some((failure.into(), true)));
            if protection {
                app.updates.pause_for_protection();
            } else {
                app.updates.reset_channel();
            }
            assert!(app.updates.failure.is_none());
            app.show_critical("内核无法启动".into());
            let _ = app.update(Message::DismissNotice);
            assert!(app.notice_visible && app.error && !app.notice_critical);
            assert!(app.notice_update_failure);
            assert_eq!(app.notice, failure);
            assert_eq!(app.pending_update_failure.as_deref(), Some(failure));
            let _ = app.update(Message::DismissNotice);
            assert!(!app.notice_visible);
            assert!(app.pending_update_failure.is_none());
        }
    }

    #[test]
    fn confirming_an_older_warning_preserves_a_different_update_failure() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("initializing")), None);
        app.prepare_initialization(Some(("更新失败：上一次安装失败".into(), true)));
        app.updates.failure = Some("更新失败：另一次安装失败".into());
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible && app.pending_update_failure.is_none());
        assert_eq!(
            app.updates.failure.as_deref(),
            Some("更新失败：另一次安装失败")
        );
    }

    #[test]
    fn dismissing_install_diagnostics_acknowledges_the_same_failure() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.prepare_initialization(Some(("更新失败：安装目录不可写".into(), true)));
        app.show_notice_followup("诊断已导出".into(), false);
        assert!(app.notice.contains("安装目录不可写") && app.raw_notice().contains("诊断已导出"));
        let _ = app.update(Message::DismissNotice);
        assert!(!app.notice_visible);
        assert!(app.updates.failure.is_none());
    }

    #[test]
    fn retrying_the_core_does_not_consume_the_pending_install_warning() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("内核无法启动")), None);
        app.update_outcome("更新失败：安装文件被占用".into(), true);
        let _ = app.update(Message::RestartCore);
        assert!(app.busy && app.working);
        assert!(app.notice_visible && app.error && !app.notice_critical);
        assert!(app.notice_update_failure);
        assert!(app.notice.contains("安装文件被占用"));
        assert!(app.updates.failure.is_some());
    }

    #[test]
    fn checking_downloading_and_handoff_failures_all_reach_recoverable_feedback() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        app.updates.checking = true;
        let _ = app.update(Message::UpdateChecked(
            app.updates.check_id,
            Err("连接超时".into()),
        ));
        assert!(app.notice.contains("检查更新失败") && app.error && !app.notice_critical);
        let (download, _) = app.updates.begin_download(100);
        let failure = Arc::new(std::sync::Mutex::new(Some(Err("安装包校验失败".into()))));
        let _ = app.update(Message::UpdateDownloaded(download, failure));
        assert!(app.notice.contains("校验失败") && app.error && !app.notice_critical);
        let _ = app.update(Message::UpdateHandoff(Err("安装授权已取消".into())));
        assert!(app.notice.contains("授权已取消") && app.error && !app.notice_critical);
        assert_eq!(app.issue_events.back().unwrap()["level"], "recoverable");
    }

    #[test]
    fn stale_checks_and_cancelled_downloads_do_not_reopen_error_feedback() {
        let mut app = App::initial_state(Err(anyhow::anyhow!("test")), None);
        app.dismiss_notice();
        app.updates.checking = true;
        let _ = app.update(Message::UpdateChecked(
            app.updates.check_id.wrapping_sub(1),
            Err("旧检查失败".into()),
        ));
        assert!(!app.notice_visible && app.updates.checking);
        let (download, _) = app.updates.begin_download(100);
        assert!(app.updates.cancel_download());
        let failure = Arc::new(std::sync::Mutex::new(Some(Err("已取消的下载失败".into()))));
        let events = app.issue_events.len();
        let _ = app.update(Message::UpdateDownloaded(download, failure));
        assert!(!app.notice_visible && app.updates.failure.is_none());
        assert_eq!(app.issue_events.len(), events);
    }
}
