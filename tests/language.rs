//! Locale display-name resolution.

use crunchyroll_rs::Locale;

use crunchydl::locale_display_name;

#[test]
fn known_locales_use_service_display_names_and_unknowns_survive() {
    assert_eq!(locale_display_name(&Locale::de_DE), "Deutsch");
    assert_eq!(
        locale_display_name(&Locale::es_419),
        "Español (América Latina)"
    );
    assert_eq!(
        locale_display_name(&Locale::Custom("x-future".into())),
        "x-future"
    );
}
