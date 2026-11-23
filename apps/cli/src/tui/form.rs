use crate::args::{Args, COMMANDS, Spec, VALUES};
use anyhow::{Result, ensure};
use clash_client_core::config::Language;

pub(super) struct Field {
    pub label: String,
    pub value: String,
    pub option: Option<&'static str>,
    pub secret: bool,
    pub locked: bool,
    pub choices: Vec<(String, String)>,
}

pub(super) struct Form {
    pub spec: &'static Spec,
    language: Language,
    pub fields: Vec<Field>,
    pub selected: usize,
    pub editing: bool,
    pub draft: String,
    pub setting: Option<String>,
    pub network: Option<super::network::Editor>,
}

pub(super) fn commands() -> impl Iterator<Item = &'static Spec> {
    COMMANDS
        .iter()
        .filter(|spec| !matches!(spec.path, "ui" | "run") && !spec.retired())
}

pub(super) fn setting_title(key: &str, language: Language) -> &str {
    match key {
        "mixed_port" => language.text("混合端口", "Mixed port"),
        "direct_download_port" => language.text("内核直连端口", "Core direct port"),
        "secret" => language.text("控制器密钥", "Controller secret"),
        "dark" => language.text("深色外观", "Dark appearance"),
        "theme_mode" => language.text("主题", "Theme"),
        "accent_color" => language.text("强调色", "Accent color"),
        "text_scale" => language.text("字号百分比", "Text scale (%)"),
        "tray_speed" => language.text("托盘网速", "Tray transfer rates"),
        "update_prereleases" => language.text("包含预发布版本", "Include prereleases"),
        "update_check_interval_minutes" => language.text(
            "自动检查更新（分钟，0 关闭）",
            "Update check interval (min, 0 disables)",
        ),
        "dns_override" => language.text("DNS 覆盖", "DNS override"),
        "fake_ip" => "Fake-IP",
        "fake_ip_exclusions" => language.text("Fake-IP 排除项", "Fake-IP exclusions"),
        "fake_ip_disabled_defaults" => {
            language.text("停用的默认排除项", "Disabled default exclusions")
        }
        "ipv6" => language.text("启用 IPv6", "Enable IPv6"),
        "udp" => language.text("启用 UDP", "Enable UDP"),
        "claude_protection" => language.text("Claude 保护", "Claude protection"),
        "delay_interval_minutes" => language.text(
            "定时测速（分钟，0 关闭）",
            "Latency test interval (min, 0 disables)",
        ),
        "subscription_update_interval_minutes" => language.text(
            "订阅自动更新（分钟，0 关闭）",
            "Subscription update interval (min, 0 disables)",
        ),
        "active_profile" => language.text("当前订阅", "Active subscription"),
        "run_mode" => language.text("运行模式", "Routing mode"),
        "proxy_mode" => language.text("代理方式", "Proxy mode"),
        "node_sort" => language.text("节点排序", "Node sort order"),
        "tui_node_sort" => language.text("终端节点排序", "Terminal node sort order"),
        "language" => clash_client_core::messages::settings_language_title(language),
        "rule_overrides" => language.text("快捷规则", "Rule shortcuts"),
        "rule_profiles" => language.text("订阅规则", "Subscription rules"),
        "application_rules" => language.text("应用规则", "Application rules"),
        _ => key,
    }
}

impl Form {
    #[cfg(test)]
    pub fn new(spec: &'static Spec) -> Self {
        Self::new_in(spec, Language::SimplifiedChinese)
    }

    pub fn new_in(spec: &'static Spec, language: Language) -> Self {
        let mut fields = spec
            .usage
            .split_whitespace()
            .skip(spec.path.split_whitespace().count())
            .take(spec.max)
            .enumerate()
            .map(|(index, word)| Field {
                label: format!(
                    "{}{}",
                    word.trim_matches(['[', ']']),
                    if index < spec.min {
                        language.text("（必填）", " (required)")
                    } else {
                        language.text("（可选）", " (optional)")
                    }
                ),
                value: String::new(),
                option: None,
                secret: false,
                locked: false,
                choices: Vec::new(),
            })
            .collect::<Vec<_>>();
        fields.extend(spec.options.split_whitespace().map(|key| Field {
            label: format!(
                "--{key}{}",
                if spec
                    .required_options()
                    .iter()
                    .any(|(required, _)| *required == key)
                {
                    language.text("（必填）", " (required)")
                } else if VALUES.split_whitespace().any(|value| value == key) {
                    language.text("（可选）", " (optional)")
                } else {
                    language.text("（true 开启，留空关闭）", " (true to enable, empty to disable)")
                }
            ),
            value: String::new(),
            option: Some(key),
            secret: matches!(key, "source"),
            locked: false,
            choices: Vec::new(),
        }));
        if matches!(spec.path, "profile add" | "config set") {
            fields[1].secret = true;
        }
        Self {
            spec,
            language,
            fields,
            selected: 0,
            editing: false,
            draft: String::new(),
            setting: None,
            network: None,
        }
    }

    pub fn setting(key: &str, value: &serde_json::Value, language: Language) -> Self {
        let mut form = Self::new_in(
            commands().find(|spec| spec.path == "config set").unwrap(),
            language,
        );
        form.fields[0].value = key.into();
        form.fields[0].secret = false;
        form.fields[0].locked = true;
        form.setting = Some(key.into());
        form.fields[1].label = language.text("值", "Value").into();
        form.fields[1].secret = key == "secret";
        form.fields[1].value = if key == "secret" {
            String::new()
        } else {
            match value.as_str() {
                Some(value) => value.to_owned(),
                None => value.to_string(),
            }
        };
        form.selected = 1;
        form
    }

    pub fn navigate(&mut self, backwards: bool) {
        let mut current = self.selected;
        loop {
            let next = if backwards {
                current.saturating_sub(1)
            } else {
                (current + 1).min(self.fields.len())
            };
            if next == current {
                break;
            }
            current = next;
            if self.fields.get(next).is_none_or(|field| !field.locked) {
                self.selected = next;
                break;
            }
        }
    }

    pub fn cycle(&mut self, backwards: bool) -> bool {
        let Some(field) = self
            .fields
            .get_mut(self.selected)
            .filter(|field| !field.locked && !field.choices.is_empty())
        else {
            return false;
        };
        let count = field.choices.len();
        let index = field
            .choices
            .iter()
            .position(|(value, _)| *value == field.value);
        let next = index.map_or(0, |index| {
            if backwards {
                (index + count - 1) % count
            } else {
                (index + 1) % count
            }
        });
        field.value.clone_from(&field.choices[next].0);
        true
    }

    pub fn begin_edit(&mut self) {
        if let Some(field) = self.fields.get(self.selected).filter(|field| !field.locked) {
            self.draft.clone_from(&field.value);
            self.editing = true;
        }
    }

    pub fn apply_edit(&mut self) {
        self.fields[self.selected].value = std::mem::take(&mut self.draft);
        self.editing = false;
    }

    pub fn append(&mut self, text: &str) {
        for character in text.chars().filter(|character| !character.is_control()) {
            if self.draft.len() + character.len_utf8() > 8192 {
                break;
            }
            self.draft.push(character);
        }
    }

    pub fn args(&self, context: &Args) -> Result<Args> {
        let mut words = self
            .spec
            .path
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for key in [
            "user",
            "data-dir",
            "resources",
            "timeout",
            "no-color",
            "lang",
        ] {
            if let Some(value) = context.get(key) {
                words.push(if VALUES.split_whitespace().any(|item| item == key) {
                    format!("--{key}={value}")
                } else {
                    format!("--{key}")
                });
            }
        }
        for field in &self.fields {
            if let Some(key) = field.option {
                if field.value.is_empty()
                    || field.value == "false" && !VALUES.split_whitespace().any(|item| item == key)
                {
                    continue;
                }
                if VALUES.split_whitespace().any(|item| item == key) {
                    words.push(format!("--{key}={}", field.value));
                } else {
                    ensure!(
                        field.value == "true",
                        "{}",
                        match self.language {
                            Language::SimplifiedChinese => {
                                format!("--{key} 请填 true、false 或留空")
                            }
                            Language::English => {
                                format!("--{key} must be true, false, or empty")
                            }
                        }
                    );
                    words.push(format!("--{key}"));
                }
            }
        }
        words.push("--".into());
        let mut missing = false;
        for (index, field) in self
            .fields
            .iter()
            .filter(|field| field.option.is_none())
            .enumerate()
        {
            if field.value.is_empty() {
                missing = true;
            } else {
                ensure!(
                    !missing,
                    "{}",
                    self.language.text(
                        "请按顺序填写参数，不能跳过前一个参数",
                        "Fill arguments in order without skipping an earlier argument"
                    )
                );
                let value = if self.setting.as_deref().is_some_and(|key| {
                    matches!(key, "fake_ip_exclusions" | "fake_ip_disabled_defaults")
                }) && self.spec.path == "config set"
                    && index == 1
                    && !field.value.trim_start().starts_with('[')
                {
                    serde_json::to_string(&clash_client_core::config::parse_fake_ip_exclusions(
                        &field.value,
                    )?)?
                } else {
                    field.value.clone()
                };
                words.push(value);
            }
        }
        ensure!(
            words.iter().map(String::len).sum::<usize>() <= 65536,
            "{}",
            self.language
                .text("表单内容过大", "Form contents are too large")
        );
        crate::args::parse(words)
    }

    pub fn visible_value(&self, index: usize) -> String {
        let field = &self.fields[index];
        let value = if self.editing && self.selected == index {
            &self.draft
        } else {
            &field.value
        };
        if field.secret && !value.is_empty() {
            "••••••".into()
        } else if let Some((_, label)) = field.choices.iter().find(|(choice, _)| choice == value) {
            label.clone()
        } else {
            value.clone()
        }
    }
}

pub(super) fn setting_value(
    settings: &clash_client_core::config::Settings,
    key: &str,
) -> serde_json::Value {
    use serde_json::json;
    match key {
        "mixed_port" => json!(settings.mixed_port),
        "direct_download_port" => json!(settings.direct_download_port),
        "secret" => json!("[已隐藏，编辑时输入新值]"),
        "dark" => json!(settings.dark),
        "theme_mode" => json!(settings.theme_mode),
        "accent_color" => json!(settings.accent_color),
        "text_scale" => json!(settings.text_scale),
        "tray_speed" => json!(settings.tray_speed),
        "update_prereleases" => json!(settings.update_prereleases),
        "update_check_interval_minutes" => json!(settings.update_check_interval_minutes),
        "dns_override" => json!(settings.dns_override),
        "fake_ip" => json!(settings.fake_ip),
        "fake_ip_exclusions" => json!(settings.fake_ip_exclusions),
        "fake_ip_disabled_defaults" => json!(settings.fake_ip_disabled_defaults),
        "ipv6" => json!(settings.ipv6),
        "udp" => json!(settings.udp),
        "claude_protection" => json!(settings.claude_protection.is_some()),
        "delay_interval_minutes" => json!(settings.delay_interval_minutes),
        "subscription_update_interval_minutes" => {
            json!(settings.subscription_update_interval_minutes)
        }
        "active_profile" => json!(settings.active_profile),
        "run_mode" => json!(settings.run_mode),
        "proxy_mode" => json!(settings.proxy_mode),
        "node_sort" => json!(settings.node_sort),
        "tui_node_sort" => json!(settings.tui_node_sort),
        "language" => json!(settings.language),
        "proxy_groups_paginated" => json!(settings.proxy_groups_paginated),
        "profiles_paginated" => json!(settings.profiles_paginated),
        "rule_overrides" => json!(settings.rule_overrides),
        "rule_profiles" => json!(format!("{} 份订阅的规则", settings.rule_profiles.len())),
        "application_rules" => json!(format!(
            "{} 份订阅的应用规则",
            settings.application_rules.len()
        )),
        _ => serde_json::Value::Null,
    }
}
