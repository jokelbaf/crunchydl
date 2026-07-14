//! Synthetic fragmented-MP4 conformance fixtures.

use std::io::Cursor;
use std::time::Duration;

use media::{Codec, Error, FragmentedMp4, TrackKind};

mod fixtures;
use fixtures::*;

#[test]
fn video_packets_preserve_b_frames_edits_keyframes_and_codec_private() {
    let rows = [
        (40, 2, 0x0200_0000, 40),
        (40, 3, 0x0001_0000, -40),
        (40, 1, 0x0200_0000, 0),
    ];
    let bytes = [init_video(), fragment(1, 100, &rows, b"abcdef")].concat();
    let media = FragmentedMp4::open(Cursor::new(bytes)).expect("valid fixture");
    let track = &media.tracks()[0];
    assert_eq!(track.kind, TrackKind::Video);
    assert_eq!(track.dimensions, Some((1920, 1080)));
    assert_eq!(track.codec.private_data(), [1, 100, 0, 40, 0xff]);
    assert_eq!(track.edits.len(), 2);

    let packets = media
        .packets()
        .collect::<Result<Vec<_>, _>>()
        .expect("packets");
    assert_eq!(packets.len(), 3);
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.data.len())
            .sum::<usize>(),
        6
    );
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.dts.ticks())
            .collect::<Vec<_>>(),
        [150, 190, 230]
    );
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.pts.ticks())
            .collect::<Vec<_>>(),
        [190, 150, 230]
    );
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.duration)
            .collect::<Vec<_>>(),
        [40, 40, 40]
    );
    assert_eq!(
        packets
            .iter()
            .map(|packet| packet.is_keyframe)
            .collect::<Vec<_>>(),
        [true, false, true]
    );
}

#[test]
fn audio_probe_reports_sample_shape_counts_and_duration() {
    let rows = [(1024, 2, 0, 0), (1024, 2, 0, 0)];
    let bytes = [init_audio(), fragment(2, 0, &rows, b"aac!")].concat();
    let media = FragmentedMp4::open(Cursor::new(bytes)).expect("valid fixture");
    let track = &media.tracks()[0];
    assert_eq!(track.kind, TrackKind::Audio);
    assert_eq!(track.sample_rate, Some(48_000));
    assert_eq!(track.channels, Some(2));
    assert_eq!(
        track.codec,
        Codec::Aac {
            audio_specific_config: vec![0x12, 0x10]
        }
    );

    let probe = media.probe().expect("probe");
    assert_eq!(probe.packet_count, 2);
    assert_eq!(probe.byte_count, 4);
    assert_eq!(probe.duration, Duration::from_nanos(42_666_666));
}

#[test]
fn hostile_fragment_sample_count_is_rejected_before_allocation() {
    let tfhd = full_atom(b"tfhd", 0, 0x020000, 1_u32.to_be_bytes());
    let tfdt = full_atom(b"tfdt", 0, 0, 0_u32.to_be_bytes());
    let mut payload = 1_000_001_u32.to_be_bytes().to_vec();
    payload.extend_from_slice(&0_i32.to_be_bytes());
    let trun = full_atom(b"trun", 0, 1, payload);
    let moof = container(b"moof", &[container(b"traf", &[tfhd, tfdt, trun])]);
    let media =
        FragmentedMp4::open(Cursor::new([init_video(), moof].concat())).expect("valid init");
    let error = media
        .packets()
        .next()
        .expect("one error")
        .expect_err("rejected");
    assert!(matches!(
        error,
        Error::Unsupported("excessive samples in one fragment")
    ));
}

#[test]
fn extended_sizes_and_zero_sized_optional_tail_are_bounded_correctly() {
    let mut bytes = init_video();
    let original_ftyp_size = u32::from_be_bytes(bytes[..4].try_into().expect("size")) as usize;
    let extended_ftyp = extended_atom(b"ftyp", &bytes[8..original_ftyp_size]);
    bytes.splice(..original_ftyp_size, extended_ftyp);
    bytes.extend_from_slice(&fragment(1, 0, &[(40, 1, 0, 0)], b"x"));
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(b"free");
    let media = FragmentedMp4::open(Cursor::new(bytes)).expect("extended fixture");
    assert_eq!(
        media
            .packets()
            .collect::<Result<Vec<_>, _>>()
            .expect("packets")
            .len(),
        1
    );
}

#[test]
fn box_declared_past_end_is_rejected() {
    let mut bytes = atom(b"ftyp", b"iso6");
    bytes.extend_from_slice(&100_u32.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    let error = FragmentedMp4::open(Cursor::new(bytes))
        .err()
        .expect("rejected");
    assert!(matches!(
        error,
        Error::Truncated {
            context: "box payload"
        }
    ));
}
