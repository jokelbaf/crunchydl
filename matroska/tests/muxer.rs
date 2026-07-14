//! End-to-end muxing verified with an independent Matroska reader.

use std::io::Cursor;
use std::time::Duration;

use matroska::{
    Attachment, AudioSettings, Chapter, Language, MuxOptions, Muxer, Packet, Track, TrackCodec,
    TrackSettings, TrackType, VideoSettings,
};

fn language() -> Language {
    Language {
        legacy: "eng".into(),
        ietf: "en-US".into(),
    }
}

fn tracks() -> Vec<Track> {
    vec![
        Track {
            number: 1,
            uid: 11,
            track_type: TrackType::Video,
            codec: TrackCodec::Avc(vec![1, 100, 0, 31]),
            settings: TrackSettings::Video(VideoSettings {
                width: 1920,
                height: 1080,
            }),
            name: Some("Video".into()),
            language: None,
            default: true,
            forced: false,
            hearing_impaired: false,
            visual_impaired: false,
            original: true,
            commentary: false,
        },
        Track {
            number: 2,
            uid: 12,
            track_type: TrackType::Audio,
            codec: TrackCodec::Aac(vec![0x12, 0x10]),
            settings: TrackSettings::Audio(AudioSettings {
                sampling_frequency: 48_000.0,
                channels: 2,
            }),
            name: Some("English".into()),
            language: Some(language()),
            default: true,
            forced: false,
            hearing_impaired: false,
            visual_impaired: false,
            original: true,
            commentary: false,
        },
        Track {
            number: 3,
            uid: 13,
            track_type: TrackType::Subtitle,
            codec: TrackCodec::Ass(
                "[Script Info]\n[V4+ Styles]\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".into(),
            ),
            settings: TrackSettings::Subtitle,
            name: Some("English".into()),
            language: Some(language()),
            default: false,
            forced: true,
            hearing_impaired: false,
            visual_impaired: false,
            original: false,
            commentary: false,
        },
    ]
}

#[test]
fn independent_reader_sees_tracks_chapters_attachments_and_duration() {
    let mut output = Cursor::new(Vec::new());
    let packets = vec![
        Packet {
            track_number: 1,
            decode_time_ms: 0,
            presentation_time_ms: 40,
            duration: Duration::from_millis(40),
            keyframe: true,
            data: vec![1, 2, 3],
        },
        Packet {
            track_number: 2,
            decode_time_ms: 0,
            presentation_time_ms: 0,
            duration: Duration::from_millis(21),
            keyframe: true,
            data: vec![4, 5],
        },
        Packet {
            track_number: 3,
            decode_time_ms: 0,
            presentation_time_ms: 0,
            duration: Duration::from_secs(1),
            keyframe: true,
            data: b"0,0,Default,,0,0,0,,Hello".to_vec(),
        },
    ];
    let chapters = [Chapter {
        start: Duration::ZERO,
        title: "Episode".into(),
        language: language(),
    }];
    let attachments = [Attachment {
        filename: "font.ttf".into(),
        mime_type: "application/x-truetype-font".into(),
        uid: 7,
        data: vec![0, 1, 0, 0, 1],
    }];
    Muxer::write(
        &mut output,
        &tracks(),
        packets,
        &chapters,
        &attachments,
        &MuxOptions::default(),
    )
    .unwrap();
    output.set_position(0);
    let parsed = matroska_reader::Matroska::open(output).unwrap();
    assert_eq!(parsed.tracks.len(), 3);
    assert_eq!(parsed.tracks[0].codec_id, "V_MPEG4/ISO/AVC");
    assert_eq!(parsed.tracks[1].codec_id, "A_AAC");
    assert_eq!(parsed.tracks[2].codec_id, "S_TEXT/ASS");
    assert!(parsed.tracks[2].forced);
    assert_eq!(parsed.chapters.len(), 1);
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.info.duration, Some(Duration::from_secs(1)));
}

#[test]
fn rejects_decode_order_regression() {
    let packets = [
        Packet {
            track_number: 1,
            decode_time_ms: 1_000,
            presentation_time_ms: 1_000,
            duration: Duration::from_millis(40),
            keyframe: true,
            data: vec![1],
        },
        Packet {
            track_number: 1,
            decode_time_ms: 0,
            presentation_time_ms: 0,
            duration: Duration::from_millis(40),
            keyframe: false,
            data: vec![2],
        },
    ];
    assert!(
        Muxer::write(
            &mut Cursor::new(Vec::new()),
            &tracks()[..1],
            packets,
            &[],
            &[],
            &MuxOptions::default()
        )
        .is_err()
    );
}
