use super::{AndroidWindow, IpStatusText, LanguageOptions, MobileData};
use clash_mobile_runtime::core::{
    config::{Language, LanguagePreference},
    countries,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

pub(super) fn current(ui: &AndroidWindow) -> Language {
    Language::from_tag(
        ui.global::<MobileData>()
            .get_presentation_language()
            .as_str(),
    )
    .unwrap_or(Language::English)
}

pub(super) fn install(ui: &AndroidWindow) {
    let choices = ui.global::<LanguageOptions>();
    choices.set_tags(ModelRc::new(VecModel::from(
        LanguagePreference::TAGS
            .iter()
            .map(|tag| SharedString::from(*tag))
            .collect::<Vec<_>>(),
    )));
    choices.on_native_name(|tag| {
        Language::from_tag(tag.as_str())
            .map(|language| language.native_name().into())
            .unwrap_or(tag)
    });
    let weak = ui.as_weak();
    ui.global::<IpStatusText>().on_region_name(move |value| {
        let language = weak.upgrade().map_or(Language::English, |ui| current(&ui));
        countries::localized_name(value.as_str(), language).into()
    });
}

pub(super) fn apply(
    ui: &AndroidWindow,
    language: Language,
) -> Result<(), slint::SelectBundledTranslationError> {
    let data = ui.global::<MobileData>();
    if data.get_presentation_language().as_str() != language.tag() {
        slint::select_bundled_translation(language.slint_tag())?;
        data.set_presentation_language(language.tag().into());
    }
    Ok(())
}
