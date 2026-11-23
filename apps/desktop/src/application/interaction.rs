use super::*;

pub(super) enum CopyRequest {
    Text(String),
    ClaudeScript,
}

impl CopyRequest {
    pub(super) fn into_text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::ClaudeScript => clash_of_rust::claude_check::browser::site_script(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CopyFeedback {
    ScriptCopied,
    Failed,
}

impl CopyFeedback {
    pub(super) fn text(self, language: Language) -> &'static str {
        match self {
            Self::ScriptCopied => messages::claude_script_copied(language),
            Self::Failed => messages::clipboard_write_failed(language),
        }
    }
}

#[derive(Clone)]
pub(crate) enum Pending {
    Delete(String),
    CloseProfile,
    CloseImport,
    Rules(rules_view::Event),
    Exit,
    NotificationSettings,
    InstallUpdate(String),
}

pub(crate) struct Confirmation {
    pub title: String,
    pub description: String,
    pub accept: &'static str,
    pub pending: Pending,
    pub ready_at: Option<std::time::Instant>,
}
impl Confirmation {
    pub fn wait(&self) -> u64 {
        self.ready_at.map_or(0, |ready| {
            let remaining = ready.saturating_duration_since(std::time::Instant::now());
            remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
        })
    }
}
pub(crate) struct ProfileEditor {
    pub id: String,
    pub name: String,
    pub source: String,
    original: (String, String),
    pub error: String,
}
impl ProfileEditor {
    fn dirty(&self) -> bool {
        self.name.trim() != self.original.0 || self.source.trim() != self.original.1
    }
}

impl App {
    pub(super) fn finish_copy(&mut self, revision: i32, script: bool, written: bool) {
        self.copy_result = Some((revision, written));
        if script {
            self.claude.status = claude_view::Status::Copy(if written {
                CopyFeedback::ScriptCopied
            } else {
                CopyFeedback::Failed
            });
            self.claude.error = !written;
        }
        if !written {
            self.show_copy_failure();
        }
    }

    pub(super) fn prepare_notifications(&mut self) -> Task<Message> {
        if self.notification_checking || self.exiting {
            return Task::none();
        }
        self.notification_prepared = true;
        self.notification_checking = true;
        Task::perform(
            async {
                let store = startup_store().map_err(|error| format!("{error:#}"))?;
                platform::notification::prepare_startup(&store)
                    .await
                    .map_err(|error| format!("{error:#}"))
            },
            Message::NotificationPrepared,
        )
    }
    pub(super) fn offer_notification_settings(&mut self) {
        if self.notification_guide_pending
            && self.visible
            && !self.working
            && !self.exiting
            && self.confirmation.is_none()
            && self.profile_editor.is_none()
            && !self.import_open
            && !self.accent_open
            && self.connection_detail.is_none()
            && self.log_detail.is_none()
            && self.claude.guide.is_none()
            && !(self.page == Page::Rules && self.rule_state.modal_open())
            && self.site_detail.is_none()
        {
            self.notification_guide_pending = false;
            let language = self.presentation_language;
            let _ = self.ask(
                messages::notification_permission_title(language),
                messages::notification_permission_detail(language).into(),
                messages::notification_open_settings(language),
                Pending::NotificationSettings,
            );
        }
    }
    fn profile_busy(&self) -> bool {
        self.working
            || self
                .queued_actions
                .iter()
                .any(|a| matches!(a, Action::EditProfile(..)))
    }
    pub(crate) fn appearance_mode(&self) -> ThemeMode {
        self.settings.theme_mode.unwrap_or(if self.settings.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        })
    }
    pub(crate) fn sync_appearance(&mut self) {
        self.dark = match self.appearance_mode() {
            ThemeMode::System => self.system_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
    }
    pub(crate) fn ask(
        &mut self,
        title: &str,
        description: String,
        accept: &'static str,
        pending: Pending,
    ) -> Task<Message> {
        self.confirmation = Some(Confirmation {
            title: title.into(),
            description,
            accept,
            pending,
            ready_at: None,
        });
        Task::none()
    }
    pub(crate) fn confirm_delete(&mut self, id: String) -> Task<Message> {
        if self.working || self.exiting {
            return Task::none();
        }
        let Some(profile) = self.profiles.iter().find(|p| p.id == id && !p.is_default()) else {
            return Task::none();
        };
        let active = self.settings.active_profile.as_deref() == Some(&id);
        let language = self.presentation_language;
        let description = if active {
            messages::subscription_delete_active(language, &profile.name)
        } else {
            messages::subscription_delete_inactive(language, &profile.name)
        };
        self.ask(
            messages::subscription_delete_title(language),
            description,
            messages::subscription_delete_confirm(language),
            Pending::Delete(id),
        )
    }
    pub(crate) fn request_exit(&mut self) -> Task<Message> {
        if (self.import_open && (!self.profile_name.is_empty() || !self.profile_source.is_empty()))
            || self.rule_state.dirty()
            || self.rule_state.editor_modified()
            || self
                .profile_editor
                .as_ref()
                .is_some_and(ProfileEditor::dirty)
        {
            let language = self.presentation_language;
            let task = self.ask(
                messages::quit_discard_title(language),
                messages::quit_discard_detail(language).into(),
                messages::quit_discard_confirm(language),
                Pending::Exit,
            );
            return Task::batch([
                task,
                desktop_runtime::window::latest().map(Message::ShowWindow),
            ]);
        }
        self.exit()
    }
    pub(crate) fn desktop_message(&mut self, message: &Message) -> Option<Task<Message>> {
        let none = Task::none;
        Some(match message {
            Message::AccentDialog(open) => {
                self.accent_open = *open;
                none()
            }
            Message::ImportDialog(true) => {
                self.import_open = true;
                none()
            }
            Message::ImportDialog(false) => {
                if !self.profile_name.is_empty() || !self.profile_source.is_empty() {
                    let language = self.presentation_language;
                    self.ask(
                        messages::subscription_discard_draft_title(language),
                        messages::subscription_discard_draft_detail(language).into(),
                        messages::discard_changes(language),
                        Pending::CloseImport,
                    )
                } else {
                    self.import_open = false;
                    none()
                }
            }

            Message::SystemTheme(mode) => {
                self.system_dark = *mode == desktop_runtime::theme::Mode::Dark;
                self.sync_appearance();
                none()
            }
            Message::Appearance(mode) => self.dispatch(Action::Appearance(*mode)),
            Message::TextScale(scale) => self.dispatch(Action::TextScale(*scale)),
            Message::TraySpeed(enabled) => self.dispatch(Action::TraySpeed(*enabled)),
            Message::Copy(value) => {
                self.copy_request = Some(CopyRequest::Text(value.clone()));
                none()
            }
            Message::ExportDiagnostics(context) => self.export_diagnostics(context),
            Message::ReportIssue(context) => self.prepare_diagnostics(context, true),
            Message::DiagnosticsExported(result) => {
                self.diagnosing = false;
                match result {
                    Ok((path, url)) => {
                        self.show_notice_followup(format!("诊断已导出：{path}"), false);
                        if let Some(url) = url {
                            let url = url.clone();
                            return Some(Task::perform(
                                async move {
                                    tokio::task::spawn_blocking(move || platform::open_url(&url))
                                        .await
                                        .map_err(|error| error.to_string())?
                                        .map_err(|error| error.to_string())
                                },
                                Message::IssueOpened,
                            ));
                        }
                    }
                    Err(error) => {
                        self.show_notice_followup(format!("诊断采集或导出失败：{error}"), true)
                    }
                }
                none()
            }
            Message::IssueOpened(result) => {
                if let Err(error) = result {
                    self.show_notice_followup(format!("无法打开问题反馈页面：{error}"), true);
                }
                none()
            }
            Message::LogLevel(level) => {
                self.log_level = *level;
                self.list_offset = 0;
                none()
            }
            Message::LogDetail(line) => {
                self.log_detail = Some(line.clone());
                none()
            }
            Message::CloseLogDetail => {
                self.log_detail = None;
                none()
            }
            Message::ConnectionDetail(id) => self.open_connection_detail(id),
            Message::ConnectionResolved(generation, result) => {
                self.accept_connection_addresses(*generation, result);
                none()
            }
            Message::CloseConnectionDetail => {
                self.clear_connection_detail();
                none()
            }
            Message::CancelConfirm => {
                self.confirmation = None;
                none()
            }
            Message::Confirm => {
                if self.working
                    || self.exiting
                    || self
                        .confirmation
                        .as_ref()
                        .is_some_and(|confirmation| confirmation.wait() > 0)
                {
                    return Some(none());
                }
                match self.confirmation.take().map(|c| c.pending) {
                    Some(Pending::InstallUpdate(version)) => {
                        self.update(Message::InstallUpdateConfirmed(version))
                    }
                    Some(Pending::Delete(id)) => self.dispatch(Action::DeleteProfile(id)),
                    Some(Pending::CloseImport) => {
                        self.import_open = false;
                        self.profile_name.clear();
                        self.profile_source.clear();
                        none()
                    }
                    Some(Pending::CloseProfile) => {
                        self.profile_editor = None;
                        none()
                    }
                    Some(Pending::Rules(event)) => self.update_rules(event),
                    Some(Pending::Exit) => self.exit(),
                    Some(Pending::NotificationSettings) => Task::perform(
                        async {
                            tokio::task::spawn_blocking(platform::notification::open_settings)
                                .await
                                .map_err(|error| error.to_string())?
                                .map_err(|error| format!("{error:#}"))
                        },
                        Message::NotificationSettingsOpened,
                    ),
                    None => none(),
                }
            }
            Message::EditProfile(id) => {
                if let Some(p) = self
                    .profiles
                    .iter()
                    .find(|p| &p.id == id && !p.is_default())
                {
                    self.profile_editor = Some(ProfileEditor {
                        id: id.clone(),
                        name: p.name.clone(),
                        source: p.source.clone(),
                        original: (p.name.clone(), p.source.clone()),
                        error: String::new(),
                    });
                    return Some(desktop_runtime::widget::operation::focus(
                        "profile-edit-name",
                    ));
                }
                none()
            }
            Message::EditName(value) => {
                if let Some(p) = &mut self.profile_editor {
                    p.name.clone_from(value);
                }
                none()
            }
            Message::EditSource(value) => {
                if let Some(p) = &mut self.profile_editor {
                    p.source.clone_from(value);
                }
                none()
            }
            Message::CloseProfileEdit => {
                if self.profile_busy() {
                    return Some(none());
                }
                if self
                    .profile_editor
                    .as_ref()
                    .is_some_and(ProfileEditor::dirty)
                {
                    let language = self.presentation_language;
                    self.ask(
                        messages::subscription_discard_edit_title(language),
                        messages::subscription_discard_edit_detail(language).into(),
                        messages::discard_changes(language),
                        Pending::CloseProfile,
                    )
                } else {
                    self.profile_editor = None;
                    none()
                }
            }
            Message::SaveProfileEdit => {
                if self.profile_busy() || self.exiting || self.picking_file {
                    return Some(none());
                }
                if let Some(p) = &mut self.profile_editor {
                    if p.name.trim().is_empty()
                        || (p.source.trim().is_empty() && !p.original.1.is_empty())
                    {
                        p.error = messages::subscription_name_and_source_required(
                            self.presentation_language,
                        )
                        .into();
                        return Some(none());
                    }
                    let action = Action::EditProfile(
                        p.id.clone(),
                        p.name.trim().into(),
                        p.source.trim().into(),
                    );
                    self.dispatch(action)
                } else {
                    none()
                }
            }
            Message::ImportProfile => {
                if self.profile_source.trim().is_empty() || self.working || self.exiting {
                    none()
                } else {
                    self.dispatch(Action::Import(
                        self.profile_name.clone(),
                        self.profile_source.trim().into(),
                    ))
                }
            }
            Message::PickFile(edit) => {
                if self.picking_file || self.exiting || self.working {
                    return Some(none());
                }
                self.picking_file = true;
                self.picking_profile = if *edit {
                    self.profile_editor.as_ref().map(|p| p.id.clone())
                } else {
                    None
                };
                let edit = *edit;
                #[cfg(target_os = "macos")]
                let task = runtime::native::call(move |host| match host.pick_file() {
                    Ok(receiver) => Message::FilePickStarted(
                        edit,
                        Arc::new(std::sync::Mutex::new(Some(receiver))),
                    ),
                    Err(error) => Message::FilePicked(edit, Err(error)),
                });
                #[cfg(not(target_os = "macos"))]
                let task = Task::perform(
                    clash_of_rust::file_dialog::choose(self.presentation_language),
                    move |r| Message::FilePicked(edit, r),
                );
                task
            }
            #[cfg(target_os = "macos")]
            Message::FilePickStarted(edit, session) => {
                let edit = *edit;
                if let Some(receiver) = session.lock().unwrap().take() {
                    let language = self.presentation_language;
                    Task::perform(
                        async move {
                            receiver.await.unwrap_or_else(|_| {
                                Err(clash_of_rust::file_dialog::closed_message(language).into())
                            })
                        },
                        move |result| Message::FilePicked(edit, result),
                    )
                } else {
                    none()
                }
            }
            Message::FilePicked(edit, result) => {
                self.picking_file = false;
                let target = self.picking_profile.take();
                match result {
                    Ok(Some(path)) => {
                        if *edit {
                            if let Some(p) = &mut self.profile_editor
                                && target.as_deref() == Some(&p.id)
                            {
                                p.source = path.to_string_lossy().into();
                            }
                        } else {
                            if self.profile_name.trim().is_empty() {
                                self.profile_name = path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into();
                            }
                            self.profile_source = path.to_string_lossy().into();
                        }
                    }
                    Err(error) => {
                        self.show_notice(error.clone(), true);
                    }
                    Ok(None) => {}
                }
                none()
            }
            Message::Key(desktop_runtime::keyboard::Event::KeyPressed {
                key, modifiers, ..
            }) => {
                use desktop_runtime::keyboard::{Key, key::Named};
                if !self.visible || self.exiting {
                    return Some(none());
                }
                match key {
                    Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("f") => {
                        if self.confirmation.is_none()
                            && self.profile_editor.is_none()
                            && !self.import_open
                            && !self.accent_open
                            && self.connection_detail.is_none()
                            && self.claude.guide.is_none()
                            && !(self.page == Page::Rules && self.rule_state.modal_open())
                        {
                            desktop_runtime::widget::operation::focus("page-search")
                        } else {
                            none()
                        }
                    }
                    Key::Character(c)
                        if cfg!(target_os = "macos")
                            && modifiers.command()
                            && c.eq_ignore_ascii_case("q") =>
                    {
                        self.request_exit()
                    }
                    Key::Named(Named::Escape) => {
                        if self.confirmation.is_some() {
                            self.confirmation = None;
                            none()
                        } else if self.import_open {
                            self.desktop_message(&Message::ImportDialog(false)).unwrap()
                        } else if self.accent_open {
                            self.accent_open = false;
                            none()
                        } else if self.profile_editor.is_some() {
                            self.desktop_message(&Message::CloseProfileEdit).unwrap()
                        } else if self.log_detail.is_some() {
                            self.log_detail = None;
                            none()
                        } else if self.connection_detail.is_some() {
                            self.clear_connection_detail();
                            none()
                        } else if self.claude.guide.is_some() {
                            self.claude.guide = None;
                            none()
                        } else if self.fake_ip_editor {
                            self.fake_ip_editor = false;
                            self.fake_ip_input.clear();
                            self.fake_ip_error.clear();
                            none()
                        } else if self.page == Page::Rules {
                            self.update_rules(rules_view::Event::CloseEditor)
                        } else if self.page == Page::Websites {
                            self.site_detail = None;
                            none()
                        } else {
                            none()
                        }
                    }
                    _ => none(),
                }
            }
            _ => return None,
        })
    }
}
pub(super) fn theme_updates() -> Task<Message> {
    #[cfg(target_os = "linux")]
    return Task::run(
        desktop_runtime::stream::channel(2, async |output| {
            loop {
                let result: anyhow::Result<()> = async {
                    use futures_util::StreamExt;
                    let connection = zbus::Connection::session().await?;
                    let settings = zbus::Proxy::new(
                        &connection,
                        "org.freedesktop.portal.Desktop",
                        "/org/freedesktop/portal/desktop",
                        "org.freedesktop.portal.Settings",
                    )
                    .await?;
                    let mut changed = settings.receive_signal("SettingChanged").await?;
                    let values: std::collections::HashMap<
                        String,
                        std::collections::HashMap<String, zbus::OwnedVariant>,
                    > = settings
                        .call("ReadAll", &(["org.freedesktop.appearance"].as_slice(),))
                        .await?;
                    if let Some(value) = values
                        .get("org.freedesktop.appearance")
                        .and_then(|v| v.get("color-scheme"))
                        && let Ok(value) = u32::try_from(value)
                    {
                        output
                            .send(Message::SystemTheme(if value == 1 {
                                desktop_runtime::theme::Mode::Dark
                            } else {
                                desktop_runtime::theme::Mode::Light
                            }))
                            .await?;
                    }
                    while let Some(signal) = changed.next().await {
                        let (namespace, key, value): (String, String, zbus::OwnedVariant) =
                            signal.body().deserialize()?;
                        if namespace == "org.freedesktop.appearance"
                            && key == "color-scheme"
                            && let Ok(value) = u32::try_from(&value)
                        {
                            output
                                .send(Message::SystemTheme(if value == 1 {
                                    desktop_runtime::theme::Mode::Dark
                                } else {
                                    desktop_runtime::theme::Mode::Light
                                }))
                                .await?;
                        }
                    }
                    Ok(())
                }
                .await;
                if output.is_closed() {
                    break;
                }
                let _ = result; // A missing portal leaves the native theme fallback.
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }),
        |message| message,
    );
    #[cfg(not(target_os = "linux"))]
    Task::none()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        App::initial_state(Err(anyhow::anyhow!("isolated desktop test")), None)
    }

    #[test]
    fn clipboard_feedback_attaches_both_outcomes_to_the_request_revision() {
        use clash_desktop_protocol::{Intent, UiData, models::ViewModels};

        let mut app = app();
        app.dismiss_notice();
        app.presentation_language = Language::English;
        let data = UiData::default();
        let mut models = ViewModels::default();
        models.attach(&data);
        let text = "[info] fictional log";

        let _ = app.intent(Intent::Copy, text, "");
        assert!(app.copy_result.is_none());
        assert_eq!(app.copy_request.take().unwrap().into_text(), text);
        app.finish_copy(4, false, true);
        app.present(&data, &mut models);
        assert_eq!(data.get_copy_revision(), 4);
        assert!(!data.get_copy_failed());
        assert!(data.get_copy_value().as_str().is_empty());

        let _ = app.intent(Intent::Copy, text, "");
        assert_eq!(app.copy_request.take().unwrap().into_text(), text);
        app.finish_copy(5, false, false);
        app.present(&data, &mut models);
        assert_eq!(data.get_copy_revision(), 5);
        assert!(data.get_copy_failed());
        assert!(data.get_copy_value().as_str().is_empty());
        assert!(data.get_notice_error());
        assert_eq!(
            data.get_notice().as_str(),
            "Could not write to the system clipboard. Please try again."
        );
    }

    #[test]
    fn clipboard_script_status_waits_for_native_result() {
        use clash_desktop_protocol::Intent;

        let mut app = app();
        app.dismiss_notice();
        app.presentation_language = Language::SimplifiedChinese;
        app.claude.status = "先前状态".into();
        let _ = app.intent(Intent::ClaudeCopyScript, "", "");
        assert_eq!(
            app.claude.status.display(app.presentation_language),
            "先前状态"
        );
        assert!(matches!(
            app.copy_request.take(),
            Some(CopyRequest::ClaudeScript)
        ));
        app.finish_copy(7, true, false);
        assert_eq!(app.copy_result, Some((7, false)));
        assert_eq!(
            app.claude.status,
            claude_view::Status::Copy(CopyFeedback::Failed)
        );
        assert!(app.claude.error);

        app.dismiss_notice();
        let _ = app.intent(Intent::ClaudeCopyScript, "", "");
        assert!(matches!(
            app.copy_request.take(),
            Some(CopyRequest::ClaudeScript)
        ));
        app.finish_copy(8, true, true);
        assert_eq!(app.copy_result, Some((8, true)));
        assert_eq!(
            app.claude.status,
            claude_view::Status::Copy(CopyFeedback::ScriptCopied)
        );
        assert!(!app.claude.error);
    }

    #[test]
    fn preview_install_requires_three_seconds_and_cancelling_or_reopening_resets_consent() {
        let mut app = app();
        app.settings.update_prereleases = true;
        app.updates.available = Some(update::Available {
            version: "0.6.4-dev.1".into(),
            url: update::RELEASES_URL.into(),
            preview: true,
            package: None,
            package_manager: None,
        });
        let _ = app.update(Message::InstallUpdate);
        assert_eq!(app.confirmation.as_ref().unwrap().wait(), 3);
        let _ = app.update(Message::Confirm);
        assert!(app.confirmation.is_some());
        assert!(!app.updates.busy());
        let _ = app.update(Message::CancelConfirm);
        assert!(app.confirmation.is_none());
        let _ = app.update(Message::Confirm);
        assert!(!app.updates.busy());
        let _ = app.update(Message::InstallUpdate);
        app.confirmation.as_mut().unwrap().ready_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(1));
        assert_eq!(app.confirmation.as_ref().unwrap().wait(), 0);
        let _ = app.update(Message::CancelConfirm);
        let _ = app.update(Message::InstallUpdate);
        assert_eq!(app.confirmation.as_ref().unwrap().wait(), 3);
    }

    #[test]
    fn update_managed_by_a_package_manager_never_prompts_for_installation() {
        for (manager, hint) in [
            ("zypper", "请使用 zypper 更新"),
            ("pacman", "更新配方后运行 makepkg -si"),
        ] {
            let mut app = app();
            assert!(app.updates.begin());
            let _ = app.update(Message::UpdateChecked(
                app.updates.check_id,
                Ok(Some(update::Available {
                    version: "99.0.0".into(),
                    url: update::RELEASES_URL.into(),
                    preview: false,
                    package: None,
                    package_manager: Some(manager),
                })),
            ));
            assert!(app.updates.status.contains(hint));
            let _ = app.update(Message::InstallUpdate);
            assert!(app.confirmation.is_none());
            assert!(!app.updates.busy());
        }
    }

    #[test]
    fn a_late_update_check_from_the_previous_channel_cannot_replace_the_current_check() {
        let mut app = app();
        assert!(app.updates.begin());
        let previous = app.updates.check_id;
        app.updates.reset_channel();
        assert!(app.updates.begin());
        let current = app.updates.check_id;
        let offer = || {
            Some(update::Available {
                version: "0.6.4-dev.1".into(),
                url: update::RELEASES_URL.into(),
                preview: true,
                package: None,
                package_manager: None,
            })
        };
        let _ = app.update(Message::UpdateChecked(previous, Ok(offer())));
        assert!(app.updates.checking);
        assert!(app.updates.available.is_none());
        let _ = app.update(Message::UpdateChecked(current, Ok(offer())));
        assert!(!app.updates.checking);
        assert!(app.updates.available.is_some());
    }
    fn rules(app: &mut App) {
        let doc = rule_manager::Document::from_raw(
            "preview",
            false,
            "proxies: []\nrules: ['MATCH,DIRECT']\n",
        )
        .unwrap();
        app.rule_state
            .load(Arc::new(doc), Arc::new(ProfileRules::default()));
    }
    #[test]
    fn unchanged_editor_can_close_after_switching_form_and_text() {
        let mut app = app();
        rules(&mut app);
        for event in [
            rules_view::Event::Add,
            rules_view::Event::TextMode(true),
            rules_view::Event::TextMode(false),
            rules_view::Event::CloseEditor,
        ] {
            let _ = app.update(Message::Rules(event));
        }
        assert!(!app.rule_state.has_editor());
        assert!(app.confirmation.is_none());
    }
    #[test]
    fn cancelling_discard_preserves_rule_editor_and_confirming_preserves_other_drafts() {
        let mut app = app();
        rules(&mut app);
        let _ = app.update(Message::Rules(rules_view::Event::Toggle(0, true)));
        let _ = app.update(Message::Rules(rules_view::Event::Add));
        let _ = app.update(Message::Rules(rules_view::Event::Payload(
            "example.com".into(),
        )));
        let _ = app.update(Message::Rules(rules_view::Event::CloseEditor));
        assert!(app.confirmation.is_some());
        assert!(app.rule_state.editor_modified());
        let _ = app.update(Message::CancelConfirm);
        assert!(app.rule_state.editor_modified());
        let _ = app.update(Message::Rules(rules_view::Event::CloseEditor));
        let _ = app.update(Message::Confirm);
        assert!(!app.rule_state.has_editor());
        assert!(app.rule_state.dirty());
        let _ = app.update(Message::Rules(rules_view::Event::Discard));
        assert!(app.rule_state.dirty());
        let _ = app.update(Message::Confirm);
        assert!(!app.rule_state.dirty());
    }
    #[test]
    fn active_subscription_delete_is_explicit_and_editor_cancel_keeps_source() {
        let mut app = app();
        app.profiles.push(Profile {
            builtin: false,
            id: "test".into(),
            name: "订阅".into(),
            source: "https://example.com/sub".into(),
            updated: 0,
            usage: None,
        });
        app.settings.active_profile = Some("test".into());
        let _ = app.update(Message::Action(Action::DeleteProfile("test".into())));
        assert!(
            app.confirmation
                .as_ref()
                .unwrap()
                .description
                .contains("默认直连")
        );
        let _ = app.update(Message::CancelConfirm);
        assert_eq!(app.profiles.len(), 1);
        let _ = app.update(Message::EditProfile("test".into()));
        let _ = app.update(Message::EditSource("https://example.org/new".into()));
        let _ = app.update(Message::CloseProfileEdit);
        assert!(app.confirmation.is_some());
        let _ = app.update(Message::CancelConfirm);
        assert_eq!(
            app.profile_editor.as_ref().unwrap().source,
            "https://example.org/new"
        );
        let _ = app.update(Message::CloseProfileEdit);
        let _ = app.update(Message::Confirm);
        assert!(app.profile_editor.is_none());
        assert_eq!(app.profiles[0].source, "https://example.com/sub");
    }
    #[test]
    fn english_delete_confirmation_keeps_the_active_subscription_and_network_consequences() {
        let mut app = app();
        app.presentation_language = Language::English;
        app.profiles.push(Profile {
            builtin: false,
            id: "test".into(),
            name: "我的订阅".into(),
            source: "https://example.com/sub".into(),
            updated: 0,
            usage: None,
        });
        app.settings.active_profile = Some("test".into());
        let _ = app.update(Message::Action(Action::DeleteProfile("test".into())));
        let confirmation = app.confirmation.as_ref().unwrap();
        assert_eq!(confirmation.title, "Delete subscription?");
        assert_eq!(confirmation.accept, "Delete subscription");
        assert!(confirmation.description.contains("我的订阅"));
        assert!(
            confirmation
                .description
                .contains("default direct connection")
        );
        assert!(confirmation.description.contains("New connections"));
        let _ = app.update(Message::CancelConfirm);
        assert!(app.profiles.iter().any(|profile| profile.id == "test"));
    }
    #[test]
    fn english_diagnostics_feedback_preserves_paths_and_failure_details() {
        let mut app = app();
        app.dismiss_notice();
        app.presentation_language = Language::English;
        app.diagnosing = true;
        let _ = app.update(Message::DiagnosticsExported(Ok((
            "/tmp/诊断.json".into(),
            None,
        ))));
        assert!(!app.diagnosing);
        assert_eq!(app.display_notice(), "Diagnostics exported: /tmp/诊断.json");
        app.dismiss_notice();

        let _ = app.update(Message::DiagnosticsExported(Err("磁盘已满".into())));
        assert!(
            app.display_notice()
                .contains("Failed to collect or export diagnostics: 磁盘已满")
        );
        assert!(app.error);
        app.dismiss_notice();

        let _ = app.update(Message::IssueOpened(Err("browser unavailable".into())));
        assert!(
            app.display_notice()
                .contains("Failed to open the issue page: browser unavailable")
        );
        assert!(app.error);
    }
    #[test]
    fn theme_changes_and_font_scale_do_not_modify_window_geometry() {
        let mut app = app();
        let size = app.window_state.size();
        app.settings.theme_mode = Some(ThemeMode::System);
        let _ = app.update(Message::SystemTheme(desktop_runtime::theme::Mode::Light));
        assert!(!app.dark);
        let _ = app.update(Message::SystemTheme(desktop_runtime::theme::Mode::Dark));
        assert!(app.dark);
        app.settings.theme_mode = Some(ThemeMode::Light);
        let _ = app.update(Message::SystemTheme(desktop_runtime::theme::Mode::Dark));
        assert!(!app.dark);
        app.settings.text_scale = 125;
        assert_eq!(app.settings.text_scale, 125);
        assert_eq!(app.window_state.size(), size);
    }
}
