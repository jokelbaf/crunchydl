//! Golden subtitle conversion fixtures.

use crunchydl::crunchyroll_rs::Locale;
use crunchydl::{SubtitleFormat, SubtitleMetadata, SubtitleProcessingOptions, process_subtitle};

#[test]
fn positioned_signs_vtt_matches_ass_golden() {
    let track = process_subtitle(
        include_str!("fixtures/subtitles/positioned_signs.vtt"),
        SubtitleFormat::WebVtt,
        SubtitleMetadata {
            locale: Locale::en_US,
            title: "English Signs".to_string(),
            is_caption: false,
            is_signs: true,
            default: false,
            forced: true,
        },
        &SubtitleProcessingOptions::default(),
    )
    .expect("convert golden fixture");
    let actual = track.ass.replace("\r\n", "\n");
    let expected = include_str!("fixtures/subtitles/positioned_signs.ass").replace("\r\n", "\n");
    assert_eq!(actual, expected);
    assert_eq!(track.referenced_fonts, ["Arial", "Trebuchet MS"]);
    assert!(track.metadata.is_signs && track.metadata.forced);
}
