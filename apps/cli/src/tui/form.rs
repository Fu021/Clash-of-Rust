use crate::args::{Args, COMMANDS, Spec, VALUES};
use anyhow::{Result, ensure};

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

pub(super) fn setting_title(key: &str) -> &str {
    match key {
        "mixed_port" => "混合端口",
        "direct_download_port" => "内核直连端口",
        "secret" => "控制器密钥",
        "dark" => "深色外观",
        "theme_mode" => "主题",
        "accent_color" => "强调色",
        "text_scale" => "字号百分比",
        "tray_speed" => "托盘网速",
        "update_prereleases" => "包含预发布版本",
        "update_check_interval_minutes" => "自动检查更新（分钟，0 关闭）",
        "dns_override" => "DNS 覆盖",
        "fake_ip" => "Fake-IP",
        "fake_ip_exclusions" => "Fake-IP 排除项",
        "fake_ip_disabled_defaults" => "停用的默认排除项",
        "ipv6" => "启用 IPv6",
        "udp" => "启用 UDP",
        "claude_protection" => "Claude 保护",
        "delay_interval_minutes" => "定时测速（分钟，0 关闭）",
        "subscription_update_interval_minutes" => "订阅自动更新（分钟，0 关闭）",
        "active_profile" => "当前订阅",
        "run_mode" => "运行模式",
        "proxy_mode" => "代理方式",
        "node_sort" => "节点排序",
        "tui_node_sort" => "终端节点排序",
        "rule_overrides" => "快捷规则",
        "rule_profiles" => "订阅规则",
        "application_rules" => "应用规则",
        _ => key,
    }
}

impl Form {
    pub fn new(spec: &'static Spec) -> Self {
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
                        "（必填）"
                    } else {
                        "（可选）"
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
                    "（必填）"
                } else if VALUES.split_whitespace().any(|value| value == key) {
                    "（可选）"
                } else {
                    "（true 开启，留空关闭）"
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
            fields,
            selected: 0,
            editing: false,
            draft: String::new(),
            setting: None,
            network: None,
        }
    }

    pub fn setting(key: &str, value: &serde_json::Value) -> Self {
        let mut form = Self::new(commands().find(|spec| spec.path == "config set").unwrap());
        form.fields[0].value = key.into();
        form.fields[0].secret = false;
        form.fields[0].locked = true;
        form.fields[0].label = "KEY（固定）".into();
        form.setting = Some(key.into());
        form.fields[1].label = "值".into();
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
        for key in ["user", "data-dir", "resources", "timeout", "no-color"] {
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
                    ensure!(field.value == "true", "--{key} 请填 true、false 或留空");
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
                ensure!(!missing, "请按顺序填写参数，不能跳过前一个参数");
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
            "表单内容过大"
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
