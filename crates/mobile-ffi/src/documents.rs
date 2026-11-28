use super::{Session, core, mobile_runtime_with_override};
use anyhow::{Context, Result, ensure};
use core::config::{
    Settings,
    overrides::{Change, GlobalOverride},
    source_view::{self, Input, Page, Request, View},
};

pub struct SubscriptionSource {
    input: Input,
    settings: Settings,
    overrides: Option<GlobalOverride>,
    request: Request,
    applied: bool,
    builtin: bool,
    application_rules: bool,
}

impl SubscriptionSource {
    pub fn read(self, cancelled: impl Fn() -> bool) -> Result<Page> {
        let source = self.input.read(&cancelled)?;
        let source = source.as_str().context("已保存的 YAML 编码无效")?;
        let generation = !self.applied && matches!(self.request.view, View::Runtime);
        let generated = if generation {
            ensure!(!cancelled(), "源码请求已取消");
            let render = || {
                let runtime = mobile_runtime_with_override(
                    source,
                    &self.settings,
                    &self.request.profile,
                    self.builtin,
                    self.overrides.as_ref(),
                )?;
                core::application_rules::runtime(
                    source,
                    &runtime,
                    &self.settings,
                    &self.request.profile,
                    self.application_rules,
                )
            };
            Some(render().map_err(|_| anyhow::anyhow!("无法按已保存设置生成运行 YAML"))?)
        } else {
            None
        };
        source_view::build_page(
            generated.as_deref().unwrap_or(source),
            &self.settings.secret,
            &self.request,
            self.applied,
            cancelled,
        )
    }
}

impl Session {
    pub fn subscription_source(&self, request: Request) -> Result<SubscriptionSource> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == request.profile)
            .context("订阅不存在")?;
        let runtime = matches!(request.view, View::Runtime);
        let applied = runtime && self.running && self.profile().id == profile.id;
        let builtin = profile.is_default();
        let override_enabled = runtime && !applied && self.settings.global_override_enabled;
        let path = if applied {
            self.store.runtime().join("config.yaml")
        } else {
            self.store.profile_path(&profile.id)?
        };
        Ok(SubscriptionSource {
            input: Input::open(&path, applied)?,
            settings: self.settings.clone(),
            overrides: GlobalOverride::applied(&self.store, override_enabled, builtin)?,
            application_rules: runtime
                && !applied
                && self.application_runtime_supported(&self.settings, &profile.id)?,
            request,
            applied,
            builtin,
        })
    }

    pub fn check_global_override(&self, change: Change) -> Result<()> {
        self.ensure_storage_ready()?;
        self.settings.require_mutable()?;
        let draft = change.checked(&self.store, self.settings.global_override_enabled)?;
        let profile = self.profile();
        if !profile.is_default() {
            let raw = self.raw()?;
            let candidate = mobile_runtime_with_override(
                &raw,
                &self.settings,
                &profile.id,
                false,
                Some(&draft),
            )?;
            self.application_runtime(&raw, &candidate, &self.settings, &profile.id)?;
        }
        Ok(())
    }
}
