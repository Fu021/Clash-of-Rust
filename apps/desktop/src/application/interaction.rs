use super::*;
#[derive(Clone)]
pub(crate) enum Pending {
    Delete(String),
    CloseProfile,
    CloseImport,
    Rules(rules_view::Event),
    WslRestore(clash_client_core::wsl::WslSettings),
    Exit,
}

pub(crate) struct Confirmation {
    pub title: String,
    pub description: String,
    pub accept: &'static str,
    pub pending: Pending,
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
        let description = format!(
            "将删除“{}”及其缓存配置和自定义规则。{}",
            profile.name,
            if active {
                "此订阅正在使用，将先切换到默认直连；新连接将不再通过此订阅。"
            } else {
                "此操作无法撤销。"
            }
        );
        self.ask("删除订阅？", description, "删除订阅", Pending::Delete(id))
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
            let task = self.ask(
                "退出并放弃修改？",
                "有尚未保存或应用的修改。退出后将丢弃这些修改，并关闭内核、恢复系统代理。".into(),
                "放弃并退出",
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
                    self.ask(
                        "放弃订阅草稿？",
                        "名称或来源尚未导入，关闭后会丢弃本次草稿。".into(),
                        "放弃修改",
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
            Message::Copy(value) => desktop_runtime::clipboard::write(value.clone()),
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
                if self.working || self.exiting {
                    return Some(none());
                }
                match self.confirmation.take().map(|c| c.pending) {
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
                    Some(Pending::WslRestore(draft)) => {
                        self.wsl_event(wsl::Event::RestoreConfirmed(draft))
                    }
                    Some(Pending::Exit) => self.exit(),
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
                    self.ask(
                        "放弃订阅修改？",
                        "配置名称或来源的修改尚未保存。".into(),
                        "放弃修改",
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
                    if p.name.trim().is_empty() || p.source.trim().is_empty() {
                        p.error = "配置名称与来源不能为空。".into();
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
                let task =
                    runtime::native::call(move |_| match clash_of_rust::file_dialog::begin() {
                        Ok(receiver) => Message::FilePickStarted(
                            edit,
                            Arc::new(std::sync::Mutex::new(Some(receiver))),
                        ),
                        Err(error) => Message::FilePicked(edit, Err(error)),
                    });
                #[cfg(not(target_os = "macos"))]
                let task = Task::perform(clash_of_rust::file_dialog::choose(), move |r| {
                    Message::FilePicked(edit, r)
                });
                task
            }
            #[cfg(target_os = "macos")]
            Message::FilePickStarted(edit, session) => {
                let edit = *edit;
                if let Some(receiver) = session.lock().unwrap().take() {
                    Task::perform(
                        async move {
                            receiver
                                .await
                                .unwrap_or_else(|_| Err("文件选择器已关闭".into()))
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
                        std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
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
                        let (namespace, key, value): (String, String, zbus::zvariant::OwnedValue) =
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
