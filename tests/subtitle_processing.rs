//! WebVTT-to-ASS conversion, raw ASS preservation, and normalization.

use std::time::Duration;

use crunchyroll_rs::Locale;

use crunchydl::{
    AssNormalization, SubtitleFormat, SubtitleMetadata, SubtitleProcessingOptions, process_subtitle,
};

fn metadata() -> SubtitleMetadata {
    SubtitleMetadata {
        locale: Locale::en_US,
        title: "English".to_string(),
        is_caption: false,
        is_signs: true,
        default: false,
        forced: true,
    }
}

#[test]
fn vtt_preserves_styles_position_breaks_and_metadata() {
    let source = "WEBVTT\n\nSTYLE\n::cue(.sign) { font-family: 'Trebuchet MS'; color: #12A0FF; font-weight: bold; }\n\n00:00:01.005 --> 00:00:02.995 line:10% position:25%\n<c.sign><i>Hello</i>\nworld</c>\n";
    let track = process_subtitle(
        source,
        SubtitleFormat::WebVtt,
        metadata(),
        &SubtitleProcessingOptions::default(),
    )
    .expect("convert");
    assert!(track.ass.contains("Style: Vtt_sign,Trebuchet MS,42"));
    assert!(
        track
            .ass
            .contains("{\\pos(320,72)}{\\i1}Hello{\\i0}\\Nworld{\\r}")
    );
    assert!(track.ass.contains("0:00:01.01,0:00:03.00,Vtt_sign"));
    assert_eq!(track.referenced_fonts, ["Arial", "Trebuchet MS"]);
    assert!(track.metadata.is_signs && track.metadata.forced);
}

#[test]
fn raw_ass_is_preserved_and_normalized_ass_is_structural() {
    let source = "[Script Info]\n; generated\nScriptType: v4.00+\n\n[Aegisub Project Garbage]\nLast Style Storage: Sign\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Sign,\"Noto Sans Thai\",30\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:12.00,Sign,,0,0,0,,{\\fnArial}Positioned, comma\nDialogue: 0,0:00:11.00,0:00:12.00,Sign,,0,0,0,,Too late\n";
    let raw = process_subtitle(
        source,
        SubtitleFormat::Ass,
        metadata(),
        &SubtitleProcessingOptions::default(),
    )
    .expect("raw");
    assert_eq!(raw.ass, source);
    assert_eq!(raw.referenced_fonts, ["Noto Sans Thai", "Arial"]);

    let options = SubtitleProcessingOptions {
        normalization: Some(AssNormalization {
            play_resolution: Some((1920, 1080)),
            layout_resolution: Some((1920, 1080)),
            wrap_style: Some(0),
            timer: Some(100.0),
            scaled_border_and_shadow: Some(true),
            remove_project_garbage: true,
            clamp_to_duration: true,
        }),
        media_duration: Some(Duration::from_secs(10)),
    };
    let normalized =
        process_subtitle(source, SubtitleFormat::Ass, metadata(), &options).expect("normalized");
    assert!(!normalized.ass.contains("Project Garbage"));
    assert!(!normalized.ass.contains("; generated"));
    assert!(normalized.ass.contains("PlayResX: 1920"));
    assert!(normalized.ass.contains("LayoutResY: 1080"));
    assert!(
        normalized
            .ass
            .contains("0:00:10.00,Sign,,0,0,0,,{\\fnArial}Positioned, comma")
    );
    assert!(!normalized.ass.contains("Too late"));
}
