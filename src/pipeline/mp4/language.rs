//! ISO 639-2 language tags for MP4 tracks.

use crunchyroll_rs::Locale;

pub(super) fn iso639_2(locale: &Locale) -> &'static str {
    match locale.to_string().split('-').next().unwrap_or("und") {
        "ar" => "ara",
        "ca" => "cat",
        "de" => "deu",
        "en" => "eng",
        "es" => "spa",
        "fr" => "fra",
        "hi" => "hin",
        "id" => "ind",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "ms" => "msa",
        "pl" => "pol",
        "pt" => "por",
        "ru" => "rus",
        "ta" => "tam",
        "te" => "tel",
        "th" => "tha",
        "tr" => "tur",
        "vi" => "vie",
        "zh" => "zho",
        _ => "und",
    }
}
