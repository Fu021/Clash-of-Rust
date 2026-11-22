use super::{AndroidWindow, IpStatusText, MobileData};
use clash_mobile_runtime::core::{config::Language, countries};
use slint::ComponentHandle;

pub(super) fn current(ui: &AndroidWindow) -> Language {
    if ui.global::<MobileData>().get_presentation_language() == "zh-CN" {
        Language::SimplifiedChinese
    } else {
        Language::English
    }
}

pub(super) fn install(ui: &AndroidWindow) {
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
        slint::select_bundled_translation(match language {
            Language::SimplifiedChinese => "",
            Language::English => "en",
        })?;
        data.set_presentation_language(language.tag().into());
    }
    Ok(())
}
