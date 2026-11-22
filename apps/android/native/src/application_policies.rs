use anyhow::{Context, Result, ensure};
use clash_mobile_runtime::{Session, core};
use core::application_rules::{Rule, Target, TargetKind, groups};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Default, Deserialize)]
struct Application {
    name: String,
    package: String,
    uid: u32,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    launchable: bool,
    #[serde(default = "installed")]
    installed: bool,
}

fn installed() -> bool {
    true
}

#[derive(Deserialize)]
struct Catalog {
    applications: Vec<Application>,
    mode: String,
    selected: BTreeSet<String>,
    #[serde(default)]
    visibility_limited: bool,
}

#[derive(Default)]
pub(super) struct Editor {
    profile: String,
    profile_name: String,
    supported: bool,
    applications: Vec<Application>,
    targets: Vec<Target>,
    query: String,
    filter: usize,
    show_system: bool,
    scope: String,
    selected: BTreeSet<String>,
    revision: i32,
    draft: Option<groups::Group>,
    baseline: Vec<Rule>,
    source_hash: String,
    providers_loaded: bool,
    visibility_limited: bool,
}

#[derive(Clone, Default)]
pub(super) struct ApplicationView {
    pub name: String,
    pub package: String,
    pub icon: String,
    pub selected: bool,
    pub available: bool,
    pub assignment: String,
}

#[derive(Clone, Default)]
pub(super) struct View {
    pub profile: String,
    pub supported: bool,
    pub applications: Vec<ApplicationView>,
    pub targets: Vec<String>,
    pub rules: Vec<(String, String, String, String, i32, String)>,
    pub revision: i32,
    pub editing: bool,
    pub form: (String, String, bool),
    pub query: String,
    pub filter: i32,
    pub show_system: bool,
    pub selected: i32,
    pub status: String,
}

fn captured(scope: &str, selected: &BTreeSet<String>, package: &str) -> bool {
    match scope {
        "only" => selected.contains(package),
        "exclude" => !selected.contains(package),
        _ => true,
    }
}

impl Editor {
    pub fn clear(&mut self) {
        *self = Self {
            revision: self.revision.wrapping_add(1),
            ..Default::default()
        };
    }

    fn visible(&self, application: &Application) -> bool {
        let selected = self
            .draft
            .as_ref()
            .is_some_and(|draft| draft.packages.contains(&application.package));
        let query = self.query.trim().to_lowercase();
        (self.show_system || application.launchable || !application.system || selected)
            && (self.filter != 1 || selected)
            && (self.filter != 2 || !selected)
            && (query.is_empty()
                || application.name.to_lowercase().contains(&query)
                || application.package.to_lowercase().contains(&query))
    }

    fn assignment(&self, package: &str) -> Option<Option<&str>> {
        self.baseline
            .iter()
            .find(|rule| {
                rule.package == package
                    && self
                        .draft
                        .as_ref()
                        .is_none_or(|draft| draft.label.id != groups::identity(rule))
            })
            .map(|rule| rule.group.as_ref().map(|group| group.name.as_str()))
    }

    fn selectable(&self, application: &Application) -> bool {
        application.installed
            && captured(&self.scope, &self.selected, &application.package)
            && self.assignment(&application.package).is_none()
    }

    pub async fn request(
        &mut self,
        session: &mut Session,
        action: &str,
        value: &str,
        language: core::config::Language,
    ) -> Result<String> {
        if action == "close" {
            self.clear();
            return Ok(String::new());
        }
        if action == "open" || action == "refresh" {
            ensure!(
                self.draft.is_none(),
                "{}",
                language.text(
                    "请先保存或撤销当前规则",
                    "Save or undo the current rule first"
                )
            );
            let catalog: Catalog =
                serde_json::from_value(session.native.request(json!({"op":"applications-list"}))?)?;
            let mut targets = core::application_rules::targets(&session.raw()?)?;
            self.providers_loaded = false;
            if session.running
                && let Ok(sources) = session.api.application_provider_targets().await
            {
                self.providers_loaded = true;
                for (provider, names) in sources {
                    for name in names {
                        targets.push(Target {
                            kind: TargetKind::Node,
                            name,
                            provider: provider.clone(),
                            ..Default::default()
                        });
                    }
                }
            }
            self.profile = session.profile().id.clone();
            self.source_hash =
                core::file_hash::hash_file(&session.store.profile_path(&self.profile)?)?.0;
            self.profile_name = session.profile().name.clone();
            self.supported = session.application_policy_supported()?;
            self.applications = catalog.applications;
            self.visibility_limited = catalog.visibility_limited;
            self.scope = catalog.mode;
            self.selected = catalog.selected;
            self.targets = targets;
            self.baseline = session
                .settings
                .application_rules
                .get(&self.profile)
                .map(|rules| rules.as_ref().clone())
                .unwrap_or_default();
            for rule in &self.baseline {
                if !self
                    .applications
                    .iter()
                    .any(|app| app.package == rule.package)
                {
                    self.applications.push(Application {
                        name: rule.package.clone(),
                        package: rule.package.clone(),
                        launchable: true,
                        ..Default::default()
                    });
                }
            }
            self.query.clear();
            self.revision = self.revision.wrapping_add(1);
            return Ok(String::new());
        }
        match action {
            "query" => self.query = value.into(),
            "filter" => {
                let filter = value
                    .parse::<usize>()
                    .context(language.text("应用筛选无效", "Invalid app filter"))?;
                ensure!(
                    filter <= 2,
                    "{}",
                    language.text("应用筛选无效", "Invalid app filter")
                );
                self.filter = filter;
            }
            "system" => self.show_system = value == "true",
            "cancel" => {
                self.draft = None;
                self.revision = self.revision.wrapping_add(1);
            }
            _ => {}
        }
        if matches!(action, "query" | "filter" | "system" | "cancel") {
            return Ok(String::new());
        }
        ensure!(
            session.settings.active_profile.as_deref() == Some(self.profile.as_str()),
            "{}",
            language.text(
                "订阅已切换，请重新打开应用规则",
                "The subscription changed; reopen application rules"
            )
        );
        if action == "edit" {
            ensure!(
                self.draft.is_none(),
                "{}",
                language.text(
                    "请先保存或撤销当前规则",
                    "Save or undo the current rule first"
                )
            );
            self.draft = Some(
                groups::list(&self.baseline)
                    .into_iter()
                    .find(|group| group.label.id == value)
                    .context(language.text("应用规则不存在", "Application rule not found"))?,
            );
            self.query.clear();
            self.filter = 0;
            self.revision = self.revision.wrapping_add(1);
            return Ok(String::new());
        }
        session.settings.require_mutable()?;
        ensure!(
            self.supported,
            "{}",
            language.text(
                "当前系统不支持按应用指定出口，需要 Android 10 或更新版本",
                "Per-app outbound rules require Android 10 or later"
            )
        );
        if action == "new" {
            ensure!(
                self.draft.is_none(),
                "{}",
                language.text(
                    "请先保存或撤销当前规则",
                    "Save or undo the current rule first"
                )
            );
            self.draft = Some(groups::Group::default());
            self.query.clear();
            self.filter = 0;
            self.revision = self.revision.wrapping_add(1);
            return Ok(String::new());
        }
        if action == "toggle" || action == "select" {
            let packages: Vec<_> = self
                .applications
                .iter()
                .filter(|app| {
                    if action == "toggle" {
                        app.package == value
                    } else {
                        self.visible(app)
                    }
                })
                .filter(|app| {
                    self.selectable(app)
                        || (value != "all"
                            && self
                                .draft
                                .as_ref()
                                .is_some_and(|draft| draft.packages.contains(&app.package)))
                })
                .map(|app| app.package.clone())
                .collect();
            ensure!(
                action != "toggle" || !packages.is_empty(),
                "{}",
                language.text(
                    "此应用未进入 VPN 或已属于其他规则组",
                    "This app is outside the VPN or assigned to another group"
                )
            );
            ensure!(
                action != "select" || matches!(value, "all" | "invert" | "clear"),
                "{}",
                language.text("应用批量操作无效", "Invalid bulk app action")
            );
            let draft = self
                .draft
                .as_mut()
                .context(language.text("请先打开规则编辑器", "Open the rule editor first"))?;
            for package in packages {
                if (action == "toggle" || value == "invert") && draft.packages.remove(&package) {
                    continue;
                }
                if value == "clear" && action == "select" {
                    draft.packages.remove(&package);
                } else {
                    draft.packages.insert(package);
                }
            }
            return Ok(String::new());
        }
        ensure!(
            core::file_hash::hash_file(&session.store.profile_path(&self.profile)?)?.0
                == self.source_hash,
            "{}",
            language.text(
                "订阅内容已更新，请重新打开应用规则",
                "The subscription was updated; reopen application rules"
            )
        );
        let saved = session
            .settings
            .application_rules
            .get(&self.profile)
            .map(|rules| rules.as_slice())
            .unwrap_or_default();
        ensure!(
            saved == self.baseline,
            "{}",
            language.text(
                "应用规则已变化，请重新打开",
                "Application rules changed; reopen them"
            )
        );
        let rules = if action == "delete" {
            ensure!(
                groups::list(saved)
                    .iter()
                    .any(|group| group.label.id == value),
                "{}",
                language.text("应用规则不存在", "Application rule not found")
            );
            groups::remove(saved, value)
        } else {
            ensure!(
                action == "save",
                "{}",
                language.text("应用规则操作无效", "Invalid application rule action")
            );
            let mut parts = value.splitn(3, '\n');
            let name = parts
                .next()
                .context(language.text("请填写规则名称", "Enter a rule name"))?
                .trim();
            let target_label = parts
                .next()
                .context(language.text("请选择出站", "Choose an outbound"))?;
            let reject_missing = match parts.next() {
                Some("true") => true,
                Some("false") => false,
                _ => anyhow::bail!(
                    "{}",
                    language.text("目标失效策略无效", "Invalid unavailable-outbound policy")
                ),
            };
            let target = self
                .targets
                .iter()
                .find(|target| target.label() == target_label)
                .context(language.text(
                    "目标已失效，请重新选择",
                    "The outbound is unavailable; select it again",
                ))?
                .clone();
            ensure!(
                target.provider.is_empty() || (session.running && self.providers_loaded),
                "{}",
                language.text(
                    "动态节点尚未确认，请连接 VPN 后刷新规则",
                    "Dynamic nodes are unconfirmed; connect the VPN and refresh rules"
                )
            );
            let mut draft = self
                .draft
                .clone()
                .context(language.text("请先打开规则编辑器", "Open the rule editor first"))?;
            draft.label.name = name.into();
            draft.target = target;
            draft.reject_missing = reject_missing;
            for package in &draft.packages {
                ensure!(
                    self.applications
                        .iter()
                        .any(|application| application.package == *package && application.installed)
                        && captured(&self.scope, &self.selected, package),
                    "{}",
                    language.text(
                        "部分应用已卸载或未进入 VPN，请重新选择",
                        "Some apps were uninstalled or are outside the VPN; select them again"
                    )
                );
            }
            groups::replace(saved, &draft)?
        };
        let current = if action == "save" {
            let catalog: Catalog =
                serde_json::from_value(session.native.request(json!({"op":"applications-list"}))?)?;
            ensure!(
                catalog.mode == self.scope && catalog.selected == self.selected,
                "{}",
                language.text(
                    "应用分流范围已变化，请重新打开规则",
                    "Application routing changed; reopen the rules"
                )
            );
            if let Some(draft) = &self.draft {
                ensure!(
                    draft.packages.iter().all(|package| catalog
                        .applications
                        .iter()
                        .any(|app| app.package == *package)),
                    "{}",
                    language.text(
                        "部分应用已卸载，请重新选择",
                        "Some apps were uninstalled; select them again"
                    )
                );
            }
            Some(catalog)
        } else {
            None
        };
        let applications = current
            .as_ref()
            .map(|catalog| &catalog.applications)
            .unwrap_or(&self.applications);
        for application in applications {
            if let Some(rule) = rules
                .iter()
                .find(|rule| rule.package == application.package)
            {
                for other in applications {
                    if other.uid == application.uid
                        && other.package != application.package
                        && let Some(other_rule) =
                            rules.iter().find(|rule| rule.package == other.package)
                    {
                        ensure!(
                            other_rule.target == rule.target
                                && other_rule.reject_missing == rule.reject_missing,
                            "{}",
                            language.text(
                                "这些应用共享系统身份，需要指定相同出站",
                                "These apps share a system identity and need the same outbound"
                            )
                        );
                    }
                }
            }
        }
        session.save_application_rules(&self.profile, rules.clone())?;
        self.baseline = rules;
        self.draft = None;
        self.revision = self.revision.wrapping_add(1);
        if session.vpn {
            if let Err(error) = session
                .native
                .request(json!({"op":"applications-reconnect"}))
            {
                anyhow::bail!(
                    "{}{error:#}",
                    language.text(
                        "应用规则已保存，但 VPN 重连未发起：",
                        "Application rules saved, but VPN reconnection did not start: "
                    )
                );
            }
            Ok(language
                .text(
                    "应用规则已保存，正在重新连接 VPN",
                    "Application rules saved; reconnecting VPN",
                )
                .into())
        } else {
            Ok(language
                .text(
                    "应用规则已保存，下次连接时生效",
                    "Application rules saved; applies on the next connection",
                )
                .into())
        }
    }

    pub fn view(&self, session: &Session, language: core::config::Language) -> View {
        if self.profile.is_empty() {
            return View::default();
        }
        let applications = self
            .applications
            .iter()
            .filter(|application| self.draft.is_some() && self.visible(application))
            .map(|application| ApplicationView {
                name: application.name.clone(),
                package: application.package.clone(),
                icon: application.icon.clone(),
                selected: self
                    .draft
                    .as_ref()
                    .is_some_and(|draft| draft.packages.contains(&application.package)),
                available: self.selectable(application),
                assignment: if !application.installed {
                    language.text("应用未安装", "App not installed").into()
                } else if !captured(&self.scope, &self.selected, &application.package) {
                    language.text("未进入 VPN", "Outside the VPN").into()
                } else {
                    self.assignment(&application.package)
                        .map(|name| match (language, name) {
                            (core::config::Language::SimplifiedChinese, Some(name)) => {
                                format!("属于 {name}")
                            }
                            (core::config::Language::English, Some(name)) => {
                                format!("Assigned to {name}")
                            }
                            (core::config::Language::SimplifiedChinese, None) => {
                                "属于已有规则".into()
                            }
                            (core::config::Language::English, None) => {
                                "Assigned to an existing rule".into()
                            }
                        })
                        .unwrap_or_default()
                },
            })
            .collect();
        let rules = groups::list(&self.baseline)
            .into_iter()
            .map(|group| {
                let status = if !self.supported {
                    language.text("系统不支持", "Unsupported")
                } else if session.settings.claude_protection.is_some() {
                    language.text("Claude 保护期间暂停", "Paused during Claude protection")
                } else if session.settings.run_mode != "rule" {
                    language.text("仅规则模式生效", "Active in rule mode only")
                } else if !group.target.provider.is_empty()
                    && (!session.running || !self.providers_loaded)
                {
                    language.text("动态目标待确认", "Dynamic outbound unconfirmed")
                } else if !self
                    .targets
                    .iter()
                    .any(|candidate| group.target.matches(candidate))
                {
                    if group.reject_missing {
                        language.text("目标失效 · 阻止连接", "Outbound unavailable · blocking")
                    } else {
                        language.text(
                            "目标失效 · 跟随配置",
                            "Outbound unavailable · following subscription",
                        )
                    }
                } else if group.packages.iter().any(|package| {
                    !self
                        .applications
                        .iter()
                        .any(|app| app.package == *package && app.installed)
                        || !captured(&self.scope, &self.selected, package)
                }) {
                    language.text(
                        "部分应用未安装或未进入 VPN",
                        "Some apps are uninstalled or outside the VPN",
                    )
                } else if !session.vpn {
                    language.text("等待连接 VPN", "Waiting for VPN connection")
                } else {
                    language.text("已应用 · 新连接生效", "Applied · affects new connections")
                };
                let summary = group
                    .packages
                    .iter()
                    .map(|package| {
                        self.applications
                            .iter()
                            .find(|app| app.package == *package)
                            .map(|app| app.name.as_str())
                            .unwrap_or(package)
                    })
                    .collect::<Vec<_>>()
                    .join(language.text("、", ", "));
                (
                    group.label.id,
                    group.label.name,
                    group.target.label(),
                    status.into(),
                    group.packages.len() as i32,
                    summary,
                )
            })
            .collect();
        View {
            profile: self.profile_name.clone(),
            supported: self.supported,
            applications,
            targets: self
                .targets
                .iter()
                .filter(|target| target.kind != TargetKind::Follow)
                .map(Target::label)
                .collect(),
            rules,
            revision: self.revision,
            editing: self.draft.is_some(),
            form: self
                .draft
                .as_ref()
                .map(|draft| {
                    (
                        draft.label.name.clone(),
                        if draft.target.kind == TargetKind::Follow {
                            String::new()
                        } else {
                            draft.target.label()
                        },
                        draft.reject_missing,
                    )
                })
                .unwrap_or_else(|| (String::new(), String::new(), true)),
            query: self.query.clone(),
            filter: self.filter as i32,
            show_system: self.show_system,
            selected: self
                .draft
                .as_ref()
                .map_or(0, |draft| draft.packages.len() as i32),
            status: if !self.supported {
                language.text("指定出口需要 Android 10 或更新版本；应用范围分流仍可使用。", "Per-app outbounds require Android 10 or later; app selection remains available.").into()
            } else if session.settings.active_profile.as_deref() != Some(&self.profile) {
                language
                    .text(
                        "订阅已切换，请重新打开应用规则。",
                        "The subscription changed; reopen application rules.",
                    )
                    .into()
            } else if self.visibility_limited {
                language.text("系统未返回完整应用列表；请在应用分流页检查应用信息并刷新。", "The app list is incomplete; check app info and refresh application routing.").into()
            } else if session.settings.run_mode != "rule" {
                language
                    .text(
                        "应用规则仅在规则模式生效。",
                        "Application rules apply in rule mode only.",
                    )
                    .into()
            } else {
                language
                    .text(
                        "只作用于进入 VPN 的应用；其余应用跟随订阅规则。",
                        "Rules apply to VPN apps; other apps follow the subscription.",
                    )
                    .into()
            },
        }
    }
}
