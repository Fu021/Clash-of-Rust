use super::*;
use clash_desktop_protocol::Intent;

impl App {
    fn category(&self, text: &str) -> CategoryFilter {
        ip_report::Report::from_results(&self.site_results)
            .categories
            .into_iter()
            .find(|category| category.name == text)
            .map_or(CategoryFilter::All, |category| {
                CategoryFilter::Group(category.name)
            })
    }
    fn region(&self, text: &str) -> RegionFilter {
        if text == "未识别地区" {
            return RegionFilter::Unidentified;
        }
        self.site_results
            .iter()
            .filter_map(|result| ip_report::region(result.as_ref()))
            .find(|code| RegionFilter::Country(code).to_string() == text)
            .map_or(RegionFilter::All, RegionFilter::Country)
    }
    pub(super) fn intent(&mut self, intent: Intent, a: &str, b: &str) -> Task<Message> {
        use claude_view::Event as C;
        use rules_view::Event as R;
        let boolean = |value: &str| value == "true";
        let message = match intent {
            Intent::PickFile => {
                if !boolean(a) {
                    self.import_open = true;
                }
                Message::PickFile(boolean(a))
            }
            Intent::RuleToggle => match a.parse::<usize>() {
                Ok(index) if index < 5 => Message::Rules(R::Toggle(index, boolean(b))),
                _ => Message::Noop,
            },
            Intent::RuleProfile => self.profiles.iter().find(|profile| profile.id == a).map_or(
                Message::Noop,
                |profile| {
                    Message::Rules(R::Profile(rules_view::ProfileChoice {
                        id: profile.id.clone(),
                        name: profile.name.clone(),
                    }))
                },
            ),
            Intent::ConnectionSort => Message::ConnectionSort(match a {
                "rule" => ConnectionField::Rule,
                "chain" => ConnectionField::Chain,
                "upload-rate" => ConnectionField::UploadRate,
                "download-rate" => ConnectionField::DownloadRate,
                "upload-total" => ConnectionField::UploadTotal,
                "download-total" => ConnectionField::DownloadTotal,
                "start" => ConnectionField::Start,
                _ => ConnectionField::Name,
            }),
            Intent::Confirm => return self.accept_dialog(),
            Intent::CancelConfirm | Intent::CloseDetail => return self.cancel_dialog(),
            Intent::Navigate => Message::Navigate(
                Page::ALL
                    .get(a.parse::<usize>().unwrap_or(0))
                    .copied()
                    .unwrap_or(Page::Home),
            ),
            Intent::Query => Message::Query(a.into()),
            Intent::DismissNotice => Message::DismissNotice,
            Intent::SystemNotification => Message::SystemNotification,
            Intent::RestartCore => Message::RestartCore,
            Intent::Mode => Message::Action(Action::Mode(
                ["rule", "global", "direct"]
                    .get(a.parse::<usize>().unwrap_or(0))
                    .unwrap_or(&"rule")
                    .to_string(),
            )),
            Intent::ProxyMode => Message::Action(Action::ProxyMode(match a {
                "0" => ProxyMode::System,
                "1" => ProxyMode::Tun,
                _ => ProxyMode::Off,
            })),
            Intent::NodeSort => Message::NodeSort(
                NodeSort::ALL
                    .get(a.parse::<usize>().unwrap_or(0))
                    .copied()
                    .unwrap_or_default(),
            ),
            Intent::ToggleGroup => Message::ToggleGroup(a.into()),
            Intent::GroupDelay => Message::GroupDelay(a.into()),
            Intent::GroupPage => Message::GroupPage(a.into(), boolean(b)),
            Intent::SelectNode => Message::Action(Action::Select(a.into(), b.into())),
            Intent::NodeDelay => Message::Action(Action::Delay(a.into())),
            Intent::ListPage => Message::ListPage(a.parse().unwrap_or(0)),
            Intent::ConnectionDetail => Message::ConnectionDetail(a.into()),
            Intent::CloseConnection => Message::Action(Action::Close(a.into())),
            Intent::CloseAll => Message::Action(Action::CloseAll),
            Intent::ProfileName => Message::ProfileName(a.into()),
            Intent::ProfileSource => Message::ProfileSource(a.into()),
            Intent::ImportProfile => Message::ImportProfile,
            Intent::OpenProfiles => Message::Action(Action::OpenProfiles),
            Intent::ActivateProfile => Message::Action(Action::Activate(a.into())),
            Intent::UpdateProfile => Message::Action(Action::UpdateProfile(a.into())),
            Intent::DeleteProfile => return self.confirm_delete(a.into()),
            Intent::EditProfile => Message::EditProfile(a.into()),
            Intent::AddProfile => Message::ImportDialog(true),
            Intent::EditName => Message::EditName(a.into()),
            Intent::EditSource => Message::EditSource(a.into()),
            Intent::SaveProfile => Message::SaveProfileEdit,
            Intent::CloseProfile => Message::CloseProfileEdit,
            Intent::RuleConfirmProfile => Message::Rules(R::ConfirmProfile),
            Intent::RuleCancelProfile => Message::Rules(R::CancelProfile),
            Intent::RuleCollapse => Message::Rules(R::Collapse),
            Intent::RuleFilter => Message::Rules(R::Filter(match a {
                "1" => rules_view::SourceFilter::Custom,
                "2" => rules_view::SourceFilter::Shortcut,
                "3" => rules_view::SourceFilter::Subscription,
                _ => rules_view::SourceFilter::All,
            })),
            Intent::RuleSelect => Message::Rules(R::Select(a.into())),
            Intent::RuleAdd => Message::Rules(R::Add),
            Intent::RuleEdit => Message::Rules(R::Edit),
            Intent::RuleDelete => Message::Rules(R::Delete),
            Intent::RuleEnable => Message::Rules(R::Enable),
            Intent::RuleMove => Message::Rules(R::Move(a.parse().unwrap_or(0))),
            Intent::RuleMoveOpen => Message::Rules(R::MoveOpen),
            Intent::RuleMoveInput => Message::Rules(R::MoveInput(a.into())),
            Intent::RuleMoveTo => Message::Rules(R::MoveTo),
            Intent::RuleMoveEdge => Message::Rules(R::MoveEdge(boolean(a))),
            Intent::RuleRestore => Message::Rules(R::Restore),
            Intent::RuleDiscard => Message::Rules(R::Discard),
            Intent::RuleApply => Message::Rules(R::Apply),
            Intent::RuleCloseEditor => Message::Rules(R::CloseEditor),
            Intent::RuleTextMode => Message::Rules(R::TextMode(boolean(a))),
            Intent::RuleRaw => Message::Rules(R::Raw(a.into())),
            Intent::RuleKind => Message::Rules(R::Kind(a.into())),
            Intent::RulePayload => Message::Rules(R::Payload(a.into())),
            Intent::RulePolicy => Message::Rules(R::Policy(a.into())),
            Intent::RuleNoResolve => Message::Rules(R::NoResolve(boolean(a))),
            Intent::RuleEnabled => Message::Rules(R::Enabled(boolean(a))),
            Intent::RulePosition => Message::Rules(R::Position(a.into())),
            Intent::RuleSave => Message::Rules(R::SaveEditor),
            Intent::LogLevel => Message::LogLevel(logs::Level::parse(a)),
            Intent::LogDetail => Message::LogDetail(a.into()),
            Intent::ClearLogs => Message::Action(Action::ClearLogs),
            Intent::ExportLogs => Message::Action(Action::ExportLogs),
            Intent::TestUrl => Message::TestUrl(a.into()),
            Intent::DnsHost => Message::DnsHost(a.into()),
            Intent::Probe => Message::Probe(boolean(a)),
            Intent::Dns => Message::Dns,
            Intent::SiteAll => Message::SiteAll,
            Intent::SiteCancel => Message::SiteCancel,
            Intent::SiteCategory => Message::SiteCategory(self.category(a)),
            Intent::SiteOpenCategory => {
                Message::SiteSummaryFilter(self.category(a), StatusFilter::All)
            }
            Intent::SiteStatus => {
                let status = Status::ALL
                    .into_iter()
                    .find(|status| status.to_string() == a)
                    .map_or(StatusFilter::All, StatusFilter::Status);
                if b == "all" {
                    Message::SiteSummaryFilter(CategoryFilter::All, status)
                } else {
                    Message::SiteStatus(status)
                }
            }
            Intent::SiteRegion => Message::SiteRegion(self.region(a)),
            Intent::SiteDetail => ip_check::services()
                .iter()
                .position(|service| service.id == a)
                .map_or(Message::Noop, Message::SiteDetail),
            Intent::SiteReset => Message::SiteResetFilters,
            Intent::SiteReport => Message::SiteReport,
            Intent::SiteRetest => Message::SiteRetest,
            Intent::SiteProbe => Message::SiteProbe(a.into()),
            Intent::ClaudeStart => Message::Claude(C::Start),
            Intent::ClaudeCloseSession => Message::Claude(C::CloseSession),
            Intent::ClaudeCancel => Message::Claude(C::Cancel),
            Intent::ClaudeProtection => Message::Claude(C::Protection(boolean(a))),
            Intent::ClaudeConfirmProtection => Message::Claude(C::ConfirmProtection),
            Intent::ClaudeFilter => Message::Claude(C::ResultsFilter(match a {
                "0" => Some(claude_view::ResultKind::Normal),
                "1" => Some(claude_view::ResultKind::Risk),
                "2" => Some(claude_view::ResultKind::Unknown),
                _ => None,
            })),
            Intent::ClaudePage => Message::Claude(C::ResultsPage(a.parse().unwrap_or(0))),
            Intent::ClaudeGuide => Message::Claude(C::Guide(match a {
                "diagnostics" => Some(claude_view::Guide::Diagnostics),
                "supplement" => Some(claude_view::Guide::Supplement),
                _ => None,
            })),
            Intent::ClaudeBrowser => Message::Claude(C::Browser),
            Intent::ClaudeCopyScript => Message::Claude(C::CopySiteScript),
            Intent::ClaudeImport => Message::Claude(C::Import),
            Intent::ClaudeExport => Message::Claude(C::Export),
            Intent::ClaudeKey => Message::Claude(C::ReputationKey(a.into())),
            Intent::ClaudeSaveKey => Message::Claude(C::SaveReputationKey),
            Intent::ClaudeAdditional => Message::Claude(C::Additional(match a {
                "registration" => clash_of_rust::claude_check::additional::Kind::Registration,
                "global-ping" => clash_of_rust::claude_check::additional::Kind::GlobalPing,
                _ => clash_of_rust::claude_check::additional::Kind::Speed,
            })),
            Intent::Appearance => Message::Appearance(match a {
                "0" => ThemeMode::Dark,
                "1" => ThemeMode::Light,
                _ => ThemeMode::System,
            }),
            Intent::TextScale => Message::TextScale(a.trim_end_matches('%').parse().unwrap_or(100)),
            Intent::AccentPicker => Message::AccentDialog(true),
            Intent::AccentColor => {
                let Some(color) = a
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| AccentColor::ALL.get(i).copied())
                else {
                    return Task::none();
                };
                self.accent_open = false;
                Message::Action(Action::Accent(color))
            }
            Intent::TraySpeed => Message::TraySpeed(boolean(a)),
            Intent::DnsOverride => Message::Action(Action::DnsOverride(boolean(a))),
            Intent::FakeIp => Message::Action(Action::FakeIp(boolean(a))),
            Intent::FakeIpEditor => Message::FakeIpEditor(boolean(a)),
            Intent::FakeIpInput => Message::FakeIpInput(a.into()),
            Intent::FakeIpAdd => Message::FakeIpAdd,
            Intent::FakeIpRemove => Message::FakeIpRemove(a.into()),
            Intent::FakeIpRestoreDefaults => Message::FakeIpRestoreDefaults,
            Intent::Ipv6 => Message::Action(Action::Ipv6(boolean(a))),
            Intent::Udp => Message::Action(Action::Udp(boolean(a))),
            Intent::Autostart => Message::Action(Action::Autostart(boolean(a))),
            Intent::MixedPort => Message::MixedPort(a.into()),
            Intent::SavePorts => Message::SavePorts,
            Intent::WindowWidth => Message::WindowWidth(a.into()),
            Intent::WindowHeight => Message::WindowHeight(a.into()),
            Intent::ResetWindow => Message::ResetWindowSize,
            Intent::IntervalInput => Message::IntervalInput(a.into()),
            Intent::SaveInterval => Message::SaveInterval,
            Intent::UpdateGeo => Message::Action(Action::UpdateGeo),
            Intent::CheckUpdate => Message::CheckUpdate,
            Intent::UpdatePrereleases => Message::Action(Action::UpdatePrereleases(boolean(a))),
            Intent::InstallUpdate => Message::InstallUpdate,
            Intent::CancelUpdate => Message::CancelUpdateDownload,
            Intent::OpenRelease => Message::OpenRelease,
            Intent::Copy => Message::Copy(a.into()),
            Intent::Quit => return self.request_exit(),
            Intent::FocusSearch => Message::Key(runtime::keyboard::Event::KeyPressed {
                key: runtime::keyboard::Key::Character("f".into()),
                modifiers: runtime::keyboard::Modifiers { command: true },
            }),
            Intent::Escape => Message::Key(runtime::keyboard::Event::KeyPressed {
                key: runtime::keyboard::Key::Named(runtime::keyboard::key::Named::Escape),
                modifiers: runtime::keyboard::Modifiers::default(),
            }),
        };
        self.update(message)
    }
    fn accept_dialog(&mut self) -> Task<Message> {
        if self.confirmation.is_some() {
            return self.update(Message::Confirm);
        }
        if self.profile_editor.is_some() {
            return self.update(Message::SaveProfileEdit);
        }
        if self.import_open {
            return self.update(Message::ImportProfile);
        }
        if self.rule_state.pending_profile.is_some() {
            return self.update(Message::Rules(rules_view::Event::ConfirmProfile));
        }
        if self.rule_state.editor.is_some() {
            return self.update(Message::Rules(rules_view::Event::SaveEditor));
        }
        if self.rule_state.move_open {
            return self.update(Message::Rules(rules_view::Event::MoveTo));
        }
        if self.claude.guide == Some(claude_view::Guide::Protection) {
            return self.update(Message::Claude(claude_view::Event::ConfirmProtection));
        }
        self.accent_open = false;
        Task::none()
    }
    fn cancel_dialog(&mut self) -> Task<Message> {
        if self.confirmation.is_some() {
            return self.update(Message::CancelConfirm);
        }
        if self.profile_editor.is_some() {
            return self.update(Message::CloseProfileEdit);
        }
        if self.import_open {
            return self.update(Message::ImportDialog(false));
        }
        if self.rule_state.pending_profile.is_some() {
            return self.update(Message::Rules(rules_view::Event::CancelProfile));
        }
        if self.rule_state.modal_open() {
            return self.update(Message::Rules(rules_view::Event::CloseEditor));
        }
        if self.claude.guide.is_some() {
            return self.update(Message::Claude(claude_view::Event::Guide(None)));
        }
        if self.connection_detail.is_some() {
            return self.update(Message::CloseConnectionDetail);
        }
        if self.log_detail.is_some() {
            return self.update(Message::CloseLogDetail);
        }
        self.site_detail = None;
        self.accent_open = false;
        Task::none()
    }
}
