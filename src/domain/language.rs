//! Human-readable language metadata used by frontends and output containers.

use crunchyroll_rs::Locale;

/// Return Crunchyroll's display name for a known locale.
///
/// Unknown future locales retain their BCP 47 code instead of being guessed.
#[must_use]
pub fn locale_display_name(locale: &Locale) -> String {
    let name = match locale {
        Locale::en_US => "English",
        Locale::en_IN => "English (India)",
        Locale::id_ID => "Bahasa Indonesia",
        Locale::ms_MY => "Bahasa Melayu",
        Locale::ca_ES => "Català",
        Locale::de_DE => "Deutsch",
        Locale::es_419 => "Español (América Latina)",
        Locale::es_ES => "Español (España)",
        Locale::fr_FR => "Français",
        Locale::it_IT => "Italiano",
        Locale::pl_PL => "Polski",
        Locale::pt_BR => "Português (Brasil)",
        Locale::pt_PT => "Português (Portugal)",
        Locale::vi_VN => "Tiếng Việt",
        Locale::tr_TR => "Türkçe",
        Locale::ru_RU => "Русский",
        Locale::ar_SA => "العربية",
        Locale::hi_IN => "हिंदी",
        Locale::ta_IN => "தமிழ்",
        Locale::te_IN => "తెలుగు",
        Locale::zh_CN => "中文 (普通话)",
        Locale::zh_HK => "中文 (粵語)",
        Locale::zh_TW => "中文 (國語)",
        Locale::ko_KR => "한국어",
        Locale::th_TH => "ไทย",
        Locale::ja_JP => "Japanese",
        Locale::Custom(value) => value,
    };
    name.to_string()
}
