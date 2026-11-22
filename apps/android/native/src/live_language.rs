use anyhow::Result;
use clash_mobile_runtime::{
    NativeHost, Session,
    core::config::{Language, LanguagePreference},
};
use serde_json::{Value, json};

pub(super) fn resolve(preference: LanguagePreference, native: &dyn NativeHost) -> Language {
    let system_languages = if preference == LanguagePreference::System {
        native.request(json!({"op": "system-languages"})).ok()
    } else {
        None
    };
    preference.resolve(
        system_languages
            .as_ref()
            .and_then(|value| value["languages"].as_array())
            .into_iter()
            .flatten()
            .filter_map(Value::as_str),
    )
}

pub(super) fn save(session: &mut Session, preference: LanguagePreference) -> Result<Language> {
    let mut settings = session.settings.clone();
    settings.language = preference;
    session.commit_settings(settings)?;
    Ok(resolve(preference, session.native.as_ref()))
}
