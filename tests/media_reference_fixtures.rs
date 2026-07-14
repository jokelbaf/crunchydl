//! Cross-package conformance against the checked-in VSD CENC-family fixtures.

use std::io::Cursor;
use std::time::Duration;

use crunchydl::{CencDecrypter, ContentKey};
use matroska_writer as mkv;
use media::{Codec, FragmentedMp4, TrackKind};

fn verify(init: &[u8], fragment: &[u8], key: &[u8; 16], expected_kind: TrackKind) {
    let key = ContentKey::try_from_slice(key).expect("fixture key");
    let decrypter = CencDecrypter::new(init, &key).expect("fixture init");
    let mut clear = Vec::new();
    decrypter
        .assemble(init, [(0, fragment.to_vec())], &mut clear)
        .expect("decrypt fixture");
    let media = FragmentedMp4::open(Cursor::new(clear)).expect("parse decrypted fixture");
    assert_eq!(media.tracks().len(), 1);
    assert_eq!(media.tracks()[0].kind, expected_kind);
    let probe = media.probe().expect("probe decrypted fixture");
    assert!(probe.packet_count > 0);
    assert!(probe.byte_count > 0);
}

macro_rules! fixture_test {
    ($name:ident, $scheme:literal, $track:literal, $key:expr, $kind:expr) => {
        #[test]
        fn $name() {
            verify(
                include_bytes!(concat!(
                    "fixtures/drm/vsd/",
                    $scheme,
                    "/",
                    $track,
                    "_init.mp4"
                )),
                include_bytes!(concat!("fixtures/drm/vsd/", $scheme, "/", $track, "_1.m4s")),
                $key,
                $kind,
            );
        }
    };
}

const VIDEO_KEY: &[u8; 16] = &[
    0x10, 0x0b, 0x6c, 0x20, 0x94, 0x0f, 0x77, 0x9a, 0x45, 0x89, 0x15, 0x2b, 0x57, 0xd2, 0xda, 0xcb,
];
const AUDIO_KEY: &[u8; 16] = &[
    0x3b, 0xda, 0x33, 0x29, 0x15, 0x8a, 0x47, 0x89, 0x88, 0x08, 0x16, 0xa7, 0x0e, 0x7e, 0x43, 0x6d,
];

fixture_test!(cenc_video, "cenc", "video", VIDEO_KEY, TrackKind::Video);
fixture_test!(cenc_audio, "cenc", "audio", AUDIO_KEY, TrackKind::Audio);
fixture_test!(cens_video, "cens", "video", VIDEO_KEY, TrackKind::Video);
fixture_test!(cens_audio, "cens", "audio", AUDIO_KEY, TrackKind::Audio);
fixture_test!(cbc1_video, "cbc1", "video", VIDEO_KEY, TrackKind::Video);
fixture_test!(cbc1_audio, "cbc1", "audio", AUDIO_KEY, TrackKind::Audio);
fixture_test!(cbcs_video, "cbcs", "video", VIDEO_KEY, TrackKind::Video);
fixture_test!(cbcs_audio, "cbcs", "audio", AUDIO_KEY, TrackKind::Audio);

#[test]
fn decrypted_avc_aac_ass_chapters_and_font_mux_structurally() {
    let clear = |init: &[u8], fragment: &[u8], key: &[u8; 16]| {
        let key = ContentKey::try_from_slice(key).unwrap();
        let decrypter = CencDecrypter::new(init, &key).unwrap();
        let mut output = Vec::new();
        decrypter
            .assemble(init, [(0, fragment.to_vec())], &mut output)
            .unwrap();
        output
    };
    let video = clear(
        include_bytes!("fixtures/drm/vsd/cenc/video_init.mp4"),
        include_bytes!("fixtures/drm/vsd/cenc/video_1.m4s"),
        VIDEO_KEY,
    );
    let audio = clear(
        include_bytes!("fixtures/drm/vsd/cenc/audio_init.mp4"),
        include_bytes!("fixtures/drm/vsd/cenc/audio_1.m4s"),
        AUDIO_KEY,
    );
    let mut tracks = Vec::new();
    let mut packets = Vec::new();
    for (index, clear) in [video, audio].into_iter().enumerate() {
        let parsed = FragmentedMp4::open(Cursor::new(clear)).unwrap();
        let track = parsed.tracks()[0].clone();
        let number = index as u64 + 1;
        let (track_type, codec, settings) = match track.codec {
            Codec::Avc { configuration } => (
                mkv::TrackType::Video,
                mkv::TrackCodec::Avc(configuration),
                mkv::TrackSettings::Video(mkv::VideoSettings {
                    width: track.dimensions.unwrap().0,
                    height: track.dimensions.unwrap().1,
                }),
            ),
            Codec::Aac {
                audio_specific_config,
            } => (
                mkv::TrackType::Audio,
                mkv::TrackCodec::Aac(audio_specific_config),
                mkv::TrackSettings::Audio(mkv::AudioSettings {
                    sampling_frequency: f64::from(track.sample_rate.unwrap()),
                    channels: track.channels.unwrap(),
                }),
            ),
            _ => panic!("unsupported fixture codec"),
        };
        tracks.push(mkv::Track {
            number,
            uid: 0,
            track_type,
            codec,
            settings,
            name: None,
            language: None,
            default: true,
            forced: false,
            hearing_impaired: false,
            visual_impaired: false,
            original: true,
            commentary: false,
        });
        packets.extend(parsed.packets().map(|packet| {
            let packet = packet.unwrap();
            let duration = |ticks: i64| {
                packet
                    .pts
                    .time_base()
                    .duration(u64::try_from(ticks).unwrap())
                    .unwrap()
            };
            mkv::Packet {
                track_number: number,
                decode_time_ms: i64::try_from(
                    i128::from(packet.dts.ticks()) * 1_000
                        / i128::from(packet.dts.time_base().ticks_per_second()),
                )
                .unwrap(),
                presentation_time_ms: i64::try_from(
                    i128::from(packet.pts.ticks()) * 1_000
                        / i128::from(packet.pts.time_base().ticks_per_second()),
                )
                .unwrap(),
                duration: duration(i64::from(packet.duration)),
                keyframe: packet.is_keyframe,
                data: packet.data,
            }
        }));
    }
    let language = mkv::Language {
        legacy: "eng".into(),
        ietf: "en-US".into(),
    };
    tracks.push(mkv::Track {
        number: 3,
        uid: 0,
        track_type: mkv::TrackType::Subtitle,
        codec: mkv::TrackCodec::Ass("[Script Info]\n[V4+ Styles]\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".into()),
        settings: mkv::TrackSettings::Subtitle,
        name: Some("English".into()),
        language: Some(language.clone()),
        default: false,
        forced: true,
        hearing_impaired: false,
        visual_impaired: false,
        original: false,
        commentary: false,
    });
    packets.push(mkv::Packet {
        track_number: 3,
        decode_time_ms: 0,
        presentation_time_ms: 0,
        duration: Duration::from_secs(1),
        keyframe: true,
        data: b"0,0,Default,,0,0,0,,Fixture".to_vec(),
    });
    packets.sort_by_key(|packet| (packet.decode_time_ms, packet.track_number));
    let mut output = Cursor::new(Vec::new());
    mkv::Muxer::write(
        &mut output,
        &tracks,
        packets,
        &[mkv::Chapter {
            start: Duration::ZERO,
            title: "Episode".into(),
            language,
        }],
        &[mkv::Attachment {
            filename: "fixture.ttf".into(),
            mime_type: "application/x-truetype-font".into(),
            uid: 1,
            data: b"\0\x01\0\0fixture".to_vec(),
        }],
        &mkv::MuxOptions::default(),
    )
    .unwrap();
    assert!(
        output
            .get_ref()
            .windows(4)
            .any(|window| window == [0x9b, 0x82, 0x03, 0xe8]),
        "subtitle packet must retain its one-second BlockDuration"
    );
    output.set_position(0);
    let parsed = matroska_reader::Matroska::open(output).unwrap();
    assert_eq!(parsed.tracks.len(), 3);
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.chapters.len(), 1);
}
