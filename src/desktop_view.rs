//! Transient desktop interactions. Connection details resolve an ID in the
//! current sample; neither details nor native menus keep connection history.
use super::*;
use iced::widget::column;

// winit alone does not track GNOME's color-scheme preference on Wayland/X11.
// Reuse the desktop portal bus, and listen only while following the system.
pub(crate) fn theme_subscription(enabled: bool) -> Subscription<Message> {
    #[cfg(target_os = "linux")]
    if enabled {
        return Subscription::run(|| {
            iced::stream::channel(2, async |mut output| {
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
                                    iced::theme::Mode::Dark
                                } else {
                                    iced::theme::Mode::Light
                                }))
                                .await?;
                        }
                        while let Some(signal) = changed.next().await {
                            let (namespace, key, value): (
                                String,
                                String,
                                zbus::zvariant::OwnedValue,
                            ) = signal.body().deserialize()?;
                            if namespace == "org.freedesktop.appearance"
                                && key == "color-scheme"
                                && let Ok(value) = u32::try_from(&value)
                            {
                                output
                                    .send(Message::SystemTheme(if value == 1 {
                                        iced::theme::Mode::Dark
                                    } else {
                                        iced::theme::Mode::Light
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
            })
        });
    }
    let _ = enabled;
    Subscription::none()
}

#[derive(Clone)]
pub(crate) enum Pending {
    Delete(String),
    CloseProfile,
    Rules(rules_view::Event),
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
        if self.rule_state.dirty()
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
            return Task::batch([task, iced::window::latest().map(Message::ShowWindow)]);
        }
        self.exit()
    }
    pub(crate) fn desktop_message(&mut self, message: &Message) -> Option<Task<Message>> {
        let none = Task::none;
        Some(match message {
            Message::SystemTheme(mode) => {
                self.system_dark = *mode == iced::theme::Mode::Dark;
                self.sync_appearance();
                none()
            }
            Message::Appearance(mode) => self.dispatch(Action::Appearance(*mode)),
            Message::TextScale(scale) => self.dispatch(Action::TextScale(*scale)),
            Message::TraySpeed(enabled) => self.dispatch(Action::TraySpeed(*enabled)),
            Message::Copy(value) => iced::clipboard::write(value.clone()),
            Message::ConnectionDetail(id) => {
                self.connection_detail = Some(id.clone());
                none()
            }
            Message::CloseConnectionDetail => {
                self.connection_detail = None;
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
                    Some(Pending::CloseProfile) => {
                        self.profile_editor = None;
                        none()
                    }
                    Some(Pending::Rules(event)) => self.update_rules(event),
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
                    return Some(iced::widget::operation::focus("profile-edit-name"));
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
                let task = iced::window::latest().and_then(move |id| {
                    iced::window::run(id, move |_| match clash_of_rust::file_dialog::begin() {
                        Ok(receiver) => Message::FilePickStarted(
                            edit,
                            Arc::new(std::sync::Mutex::new(Some(receiver))),
                        ),
                        Err(error) => Message::FilePicked(edit, Err(error)),
                    })
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
                        self.notice.clone_from(error);
                        self.error = true;
                        self.notice_visible = true;
                    }
                    Ok(None) => {}
                }
                none()
            }
            Message::Key(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                use iced::keyboard::{Key, key::Named};
                if !self.visible || self.exiting {
                    return Some(none());
                }
                match key {
                    Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("f") => {
                        if self.confirmation.is_none()
                            && self.profile_editor.is_none()
                            && self.connection_detail.is_none()
                            && !(self.page == Page::Rules && self.rule_state.modal_open())
                        {
                            iced::widget::operation::focus("page-search")
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
                    Key::Named(Named::Tab) => {
                        if self.confirmation.is_some()
                            || self.profile_editor.is_some()
                            || self.connection_detail.is_some()
                            || (self.page == Page::Rules && self.rule_state.modal_open())
                        {
                            use iced::advanced::widget::operation::{focusable, scope};
                            if modifiers.shift() {
                                iced::advanced::widget::operate(scope(
                                    "desktop-modal".into(),
                                    focusable::focus_previous(),
                                ))
                            } else {
                                iced::advanced::widget::operate(scope(
                                    "desktop-modal".into(),
                                    focusable::focus_next(),
                                ))
                            }
                        } else if modifiers.shift() {
                            iced::widget::operation::focus_previous()
                        } else {
                            iced::widget::operation::focus_next()
                        }
                    }
                    Key::Named(Named::Escape) => {
                        if self.confirmation.is_some() {
                            self.confirmation = None;
                            none()
                        } else if self.profile_editor.is_some() {
                            self.desktop_message(&Message::CloseProfileEdit).unwrap()
                        } else if self.connection_detail.is_some() {
                            self.connection_detail = None;
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
    fn detail_row(&self, label: &str, value: String) -> Element<'_, Message> {
        aligned_row![
            self.caption(label.to_owned()).width(110),
            self.label(value.clone())
                .size(self.font_size(14))
                .width(Length::Fill)
                .wrapping(text::Wrapping::WordOrGlyph),
            button(self.label("复制"))
                .padding([5, 8])
                .style(rounded_text)
                .on_press_maybe(
                    (!value.is_empty() && value != "—").then_some(Message::Copy(value))
                )
        ]
        .spacing(10)
        .into()
    }
    pub(crate) fn desktop_overlay<'a>(
        &'a self,
        page: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let dialog: Option<Element<'a, Message>> = if let Some(c) = &self.confirmation {
            Some(
                column![
                    self.title(&c.title),
                    self.label(&c.description)
                        .wrapping(text::Wrapping::WordOrGlyph),
                    aligned_row![
                        Space::new().width(Length::Fill),
                        button(self.label("取消"))
                            .style(rounded_secondary)
                            .padding([8, 14])
                            .on_press(Message::CancelConfirm),
                        button(self.label(c.accept))
                            .style(ui_style::danger_button)
                            .padding([8, 14])
                            .on_press_maybe(
                                (!self.working && !self.exiting).then_some(Message::Confirm)
                            )
                    ]
                    .spacing(10)
                ]
                .spacing(18)
                .into(),
            )
        } else if let Some(p) = &self.profile_editor {
            Some(
                column![
                    self.title("编辑订阅"),
                    self.caption("配置名称"),
                    text_input("配置名称", &p.name)
                        .id("profile-edit-name")
                        .size(self.font_size(15))
                        .padding(9)
                        .style(rounded_input)
                        .on_input_maybe((!self.profile_busy()).then_some(Message::EditName))
                        .on_submit(Message::SaveProfileEdit),
                    self.caption("订阅链接或本地 YAML 路径"),
                    text_input("来源", &p.source)
                        .id("profile-edit-source")
                        .size(self.font_size(15))
                        .padding(9)
                        .style(rounded_input)
                        .on_input_maybe((!self.profile_busy()).then_some(Message::EditSource))
                        .on_submit(Message::SaveProfileEdit),
                    self.caption("保存名称与来源；配置内容由订阅页的更新按钮重新载入。"),
                    self.label(&p.error)
                        .color(ui_style::tone(&self.theme(), ui_style::Tone::Danger)),
                    aligned_row![
                        button(self.label("选择文件"))
                            .style(rounded_secondary)
                            .padding([8, 12])
                            .on_press_maybe(
                                (!self.picking_file && !self.working)
                                    .then_some(Message::PickFile(true))
                            ),
                        Space::new().width(Length::Fill),
                        button(self.label("取消"))
                            .style(rounded_secondary)
                            .padding([8, 12])
                            .on_press_maybe(
                                (!self.profile_busy()).then_some(Message::CloseProfileEdit)
                            ),
                        button(self.label("保存"))
                            .style(rounded_primary)
                            .padding([8, 12])
                            .on_press_maybe(
                                (!self.profile_busy() && !self.exiting && !self.picking_file)
                                    .then_some(Message::SaveProfileEdit)
                            )
                    ]
                    .spacing(8)
                ]
                .spacing(10)
                .into(),
            )
        } else if let Some(id) = &self.connection_detail {
            let mut body = column![
                aligned_row![
                    self.title("连接详情"),
                    Space::new().width(Length::Fill),
                    button(self.label("关闭详情"))
                        .style(rounded_secondary)
                        .padding([6, 10])
                        .on_press(Message::CloseConnectionDetail)
                ]
                .spacing(8)
            ]
            .spacing(12);
            if let Some(c) = self
                .snapshot
                .connections
                .connections
                .iter()
                .find(|c| c.id.as_ref() == id)
            {
                let rate =
                    |v: Option<u64>| v.map_or_else(|| "—".into(), |v| format!("{}/s", bytes(v)));
                for (label, value) in [
                    ("连接名称", connection::name(c).to_owned()),
                    ("域名", c.metadata.host.to_string()),
                    ("嗅探域名", c.metadata.sniff_host.to_string()),
                    ("目标 IP", c.metadata.destination_ip.to_string()),
                    ("目标端口", c.metadata.destination_port.to_string()),
                    ("协议", c.metadata.network.to_string()),
                    (
                        "匹配规则",
                        if c.rule_payload.is_empty() {
                            c.rule.to_string()
                        } else {
                            format!("{},{}", c.rule, c.rule_payload)
                        },
                    ),
                    ("代理链", c.chains.join(" → ")),
                    ("上传速度", rate(c.rates.map(|r| r.upload))),
                    ("下载速度", rate(c.rates.map(|r| r.download))),
                    ("累计上传", bytes(c.upload)),
                    ("累计下载", bytes(c.download)),
                    ("开始时间", c.start.to_string()),
                    ("连接 ID", c.id.to_string()),
                ] {
                    body = body.push(self.detail_row(
                        label,
                        if value.is_empty() {
                            "—".into()
                        } else {
                            value
                        },
                    ));
                }
            } else {
                body = body.push(self.label("此连接已结束，详情不再保留。"));
            }
            Some(body.into())
        } else {
            None
        };
        if let Some(dialog) = dialog {
            let overlay = self.modal(dialog);
            stack![
                iced::widget::opaque(page.map(|_| Message::Noop)),
                iced::widget::opaque(overlay)
            ]
            .into()
        } else {
            page
        }
    }
    pub(crate) fn modal<'a>(&self, dialog: Element<'a, Message>) -> Element<'a, Message> {
        let height = self
            .window_height
            .parse::<f32>()
            .unwrap_or(700.0)
            .max(450.0)
            - 64.0;
        container(
            container(scrollable(dialog).spacing(10).height(Length::Shrink))
                .id("desktop-modal")
                .padding(22)
                .width(590)
                .max_height(height)
                .style(panel),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.52).into()),
            ..Default::default()
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        App::with_engine(Err(anyhow::anyhow!("isolated desktop test")), None).0
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
        let _ = app.update(Message::SystemTheme(iced::theme::Mode::Light));
        assert!(!app.dark);
        let _ = app.update(Message::SystemTheme(iced::theme::Mode::Dark));
        assert!(app.dark);
        app.settings.theme_mode = Some(ThemeMode::Light);
        let _ = app.update(Message::SystemTheme(iced::theme::Mode::Dark));
        assert!(!app.dark);
        app.settings.text_scale = 125;
        assert_eq!(app.font_size(16), 20.0);
        assert_eq!(app.window_state.size(), size);
    }
}
