//! Native progressive MP4 muxing with independent structural verification.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use crunchyroll_rs::Locale;
use media::{Codec, FragmentedMp4, Track, TrackKind};

use crate::{CancellationToken, Error, OutputTrack, PlannedTrack, SynchronizationOptions};

pub(crate) fn write_and_verify(
    path: &Path,
    sources: &[(PathBuf, PlannedTrack)],
    synchronization: &SynchronizationOptions,
    cancellation: &CancellationToken,
) -> Result<Vec<OutputTrack>, Error> {
    if !synchronization.offsets.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not support explicit track offsets".into(),
        ));
    }
    let output = BufWriter::new(File::create(path).map_err(|_| Error::Mux("create MP4".into()))?);
    let config = mp4::Mp4Config {
        major_brand: "isom"
            .parse()
            .map_err(|_| Error::Mux("invalid MP4 brand".into()))?,
        minor_version: 512,
        compatible_brands: ["isom", "iso2", "avc1", "mp41"]
            .into_iter()
            .map(str::parse)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::Mux("invalid MP4 compatible brand".into()))?,
        timescale: 1_000,
    };
    let mut writer =
        mp4::Mp4Writer::write_start(output, &config).map_err(|_| Error::Mux("start MP4".into()))?;
    for (source, diagnostic) in sources {
        cancellation.check()?;
        let parsed = open(source)?;
        let track = parsed
            .tracks()
            .first()
            .ok_or_else(|| Error::Mux("MP4 input contains no track".into()))?;
        writer
            .add_track(&track_config(track, diagnostic)?)
            .map_err(|_| Error::Mux("add MP4 track".into()))?;
    }
    for (index, (source, _)) in sources.iter().enumerate() {
        let parsed = open(source)?;
        for packet in parsed.packets() {
            cancellation.check()?;
            let packet = packet?;
            let rendering_offset = packet
                .pts
                .ticks()
                .checked_sub(packet.dts.ticks())
                .and_then(|ticks| i32::try_from(ticks).ok())
                .ok_or_else(|| Error::Mux("MP4 composition offset overflow".into()))?;
            writer
                .write_sample(
                    u32::try_from(index + 1)
                        .map_err(|_| Error::Mux("too many MP4 tracks".into()))?,
                    &mp4::Mp4Sample {
                        start_time: u64::try_from(packet.dts.ticks()).unwrap_or(0),
                        duration: packet.duration,
                        rendering_offset,
                        is_sync: packet.is_keyframe,
                        bytes: Bytes::from(packet.data),
                    },
                )
                .map_err(|_| Error::Mux("write MP4 sample".into()))?;
        }
    }
    writer
        .write_end()
        .map_err(|_| Error::Mux("finalize MP4".into()))?;
    let mut output = writer.into_writer();
    output.flush().map_err(|_| Error::Mux("flush MP4".into()))?;
    output
        .get_ref()
        .sync_all()
        .map_err(|_| Error::Mux("sync MP4".into()))?;
    drop(output);
    verify(path, sources)
}

fn open(path: &Path) -> Result<FragmentedMp4<BufReader<File>>, Error> {
    let file = File::open(path).map_err(|_| Error::Mux("open assembled track".into()))?;
    FragmentedMp4::open(BufReader::new(file)).map_err(Error::from)
}

fn track_config(track: &Track, diagnostic: &PlannedTrack) -> Result<mp4::TrackConfig, Error> {
    let language = iso639_2(&diagnostic.locale).to_string();
    let timescale = track.time_base.ticks_per_second();
    let media_conf = match (&track.kind, &track.codec) {
        (TrackKind::Video, Codec::Avc { configuration }) => {
            let (sps, pps) = parse_avcc(configuration)?;
            let (width, height) = track
                .dimensions
                .ok_or_else(|| Error::Mux("MP4 video dimensions missing".into()))?;
            mp4::MediaConfig::AvcConfig(mp4::AvcConfig {
                width: u16::try_from(width)
                    .map_err(|_| Error::Mux("MP4 video width is too large".into()))?,
                height: u16::try_from(height)
                    .map_err(|_| Error::Mux("MP4 video height is too large".into()))?,
                seq_param_set: sps,
                pic_param_set: pps,
            })
        }
        (
            TrackKind::Audio,
            Codec::Aac {
                audio_specific_config,
            },
        ) => {
            let (profile, freq_index, chan_conf) =
                parse_audio_specific_config(audio_specific_config)?;
            mp4::MediaConfig::AacConfig(mp4::AacConfig {
                bitrate: u32::try_from(diagnostic.bandwidth).unwrap_or(u32::MAX),
                profile,
                freq_index,
                chan_conf,
            })
        }
        _ => return Err(Error::Mux("MP4 supports only AVC and AAC tracks".into())),
    };
    Ok(mp4::TrackConfig {
        track_type: match track.kind {
            TrackKind::Video => mp4::TrackType::Video,
            TrackKind::Audio => mp4::TrackType::Audio,
            _ => return Err(Error::Mux("unsupported MP4 track kind".into())),
        },
        timescale,
        language,
        media_conf,
    })
}

fn parse_avcc(configuration: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Error> {
    if configuration.len() < 8 || configuration[0] != 1 {
        return Err(Error::Mux("invalid AVC decoder configuration".into()));
    }
    let mut cursor = 6;
    let sps_count = usize::from(configuration[5] & 0x1f);
    let mut sps = None;
    for _ in 0..sps_count {
        let value = read_nal(configuration, &mut cursor)?;
        if sps.is_none() {
            sps = Some(value);
        }
    }
    let pps_count = *configuration
        .get(cursor)
        .ok_or_else(|| Error::Mux("invalid AVC decoder configuration".into()))?;
    cursor += 1;
    let mut pps = None;
    for _ in 0..pps_count {
        let value = read_nal(configuration, &mut cursor)?;
        if pps.is_none() {
            pps = Some(value);
        }
    }
    let sps = sps.ok_or_else(|| Error::Mux("AVC configuration has no SPS".into()))?;
    if sps.len() < 4 {
        return Err(Error::Mux("AVC SPS is too short".into()));
    }
    Ok((
        sps,
        pps.ok_or_else(|| Error::Mux("AVC configuration has no PPS".into()))?,
    ))
}

fn read_nal(configuration: &[u8], cursor: &mut usize) -> Result<Vec<u8>, Error> {
    let length_bytes = configuration
        .get(*cursor..cursor.saturating_add(2))
        .ok_or_else(|| Error::Mux("truncated AVC parameter set".into()))?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    *cursor = cursor.saturating_add(2);
    let value = configuration
        .get(*cursor..cursor.saturating_add(length))
        .ok_or_else(|| Error::Mux("truncated AVC parameter set".into()))?
        .to_vec();
    *cursor = cursor.saturating_add(length);
    Ok(value)
}

fn parse_audio_specific_config(
    configuration: &[u8],
) -> Result<
    (
        mp4::AudioObjectType,
        mp4::SampleFreqIndex,
        mp4::ChannelConfig,
    ),
    Error,
> {
    if configuration.len() < 2 {
        return Err(Error::Mux("invalid AAC AudioSpecificConfig".into()));
    }
    let object_type = configuration[0] >> 3;
    let frequency_index = ((configuration[0] & 0x07) << 1) | (configuration[1] >> 7);
    let channels = (configuration[1] >> 3) & 0x0f;
    Ok((
        mp4::AudioObjectType::try_from(object_type)
            .map_err(|_| Error::Mux("unsupported AAC object type".into()))?,
        mp4::SampleFreqIndex::try_from(frequency_index)
            .map_err(|_| Error::Mux("unsupported AAC sample frequency".into()))?,
        mp4::ChannelConfig::try_from(channels)
            .map_err(|_| Error::Mux("unsupported AAC channel configuration".into()))?,
    ))
}

fn verify(path: &Path, sources: &[(PathBuf, PlannedTrack)]) -> Result<Vec<OutputTrack>, Error> {
    let file = File::open(path).map_err(|_| Error::Verification("open MP4".into()))?;
    let size = file
        .metadata()
        .map_err(|_| Error::Verification("read MP4 metadata".into()))?
        .len();
    let mut parsed = mp4::Mp4Reader::read_header(BufReader::new(file), size)
        .map_err(|_| Error::Verification("MP4 structural verification failed".into()))?;
    if parsed.is_fragmented()
        || parsed.duration().is_zero()
        || parsed.tracks().len() != sources.len()
    {
        return Err(Error::Verification(
            "MP4 structural verification failed".into(),
        ));
    }
    let mut tracks = parsed.tracks().keys().copied().collect::<Vec<_>>();
    tracks.sort_unstable();
    let mut video_default = true;
    let mut audio_default = true;
    let mut output = Vec::with_capacity(tracks.len());
    for id in tracks {
        let (sample_count, kind, codec, language) = {
            let track = parsed
                .tracks()
                .get(&id)
                .ok_or_else(|| Error::Verification("MP4 track disappeared".into()))?;
            (
                track.sample_count(),
                track
                    .track_type()
                    .map_err(|_| Error::Verification("MP4 track type is invalid".into()))?,
                track
                    .box_type()
                    .map_err(|_| Error::Verification("MP4 codec is invalid".into()))?
                    .to_string(),
                track.language().to_string(),
            )
        };
        if sample_count == 0
            || parsed
                .read_sample(id, 1)
                .map_err(|_| Error::Verification("MP4 sample verification failed".into()))?
                .is_none()
        {
            return Err(Error::Verification("MP4 contains an empty track".into()));
        }
        let default = match kind {
            mp4::TrackType::Video => std::mem::replace(&mut video_default, false),
            mp4::TrackType::Audio => std::mem::replace(&mut audio_default, false),
            _ => false,
        };
        output.push(OutputTrack {
            codec,
            language: Some(language),
            name: None,
            default,
            forced: false,
        });
    }
    Ok(output)
}

fn iso639_2(locale: &Locale) -> &'static str {
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crunchyroll_rs::Locale;

    use super::{parse_audio_specific_config, parse_avcc, write_and_verify};

    fn atom(kind: &[u8; 4], payload: impl AsRef<[u8]>) -> Vec<u8> {
        let payload = payload.as_ref();
        let mut output = Vec::with_capacity(payload.len() + 8);
        output.extend_from_slice(&(payload.len() as u32 + 8).to_be_bytes());
        output.extend_from_slice(kind);
        output.extend_from_slice(payload);
        output
    }

    fn full_atom(kind: &[u8; 4], version: u8, flags: u32, payload: impl AsRef<[u8]>) -> Vec<u8> {
        let mut body = vec![
            version,
            (flags >> 16) as u8,
            (flags >> 8) as u8,
            flags as u8,
        ];
        body.extend_from_slice(payload.as_ref());
        atom(kind, body)
    }

    fn container(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        atom(kind, children.concat())
    }

    fn init_track(id: u32, video: bool) -> Vec<u8> {
        let mut mvhd = vec![0; 8];
        mvhd.extend_from_slice(&1000_u32.to_be_bytes());
        mvhd.extend_from_slice(&1000_u32.to_be_bytes());
        let mut tkhd = vec![0; 8];
        tkhd.extend_from_slice(&id.to_be_bytes());
        tkhd.extend_from_slice(&0_u32.to_be_bytes());
        let timescale: u32 = if video { 1000 } else { 48_000 };
        let mut mdhd = vec![0; 8];
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        let handler = if video { b"vide" } else { b"soun" };
        let mut hdlr = vec![0; 4];
        hdlr.extend_from_slice(handler);
        let sample_entry = if video {
            let mut payload = vec![0; 78];
            payload[24..26].copy_from_slice(&640_u16.to_be_bytes());
            payload[26..28].copy_from_slice(&360_u16.to_be_bytes());
            payload.extend_from_slice(&atom(
                b"avcC",
                [
                    1, 100, 0, 40, 0xff, 0xe1, 0, 4, 0x67, 100, 0, 40, 1, 0, 2, 0x68, 0xee,
                ],
            ));
            atom(b"avc1", payload)
        } else {
            let mut payload = vec![0; 28];
            payload[16..18].copy_from_slice(&2_u16.to_be_bytes());
            payload[24..28].copy_from_slice(&(48_000_u32 << 16).to_be_bytes());
            let mut esds = vec![0, 0, 0, 0];
            esds.extend_from_slice(&[5, 2, 0x11, 0x90]);
            payload.extend_from_slice(&atom(b"esds", esds));
            atom(b"mp4a", payload)
        };
        let mut stsd = 1_u32.to_be_bytes().to_vec();
        stsd.extend_from_slice(&sample_entry);
        let stbl = container(b"stbl", &[full_atom(b"stsd", 0, 0, stsd)]);
        let minf = container(b"minf", &[stbl]);
        let mdia = container(
            b"mdia",
            &[
                full_atom(b"mdhd", 0, 0, mdhd),
                full_atom(b"hdlr", 0, 0, hdlr),
                minf,
            ],
        );
        let trak = container(b"trak", &[full_atom(b"tkhd", 0, 0, tkhd), mdia]);
        let mut trex = id.to_be_bytes().to_vec();
        trex.extend_from_slice(&1_u32.to_be_bytes());
        trex.extend_from_slice(&(if video { 40_u32 } else { 1024_u32 }).to_be_bytes());
        trex.extend_from_slice(&0_u32.to_be_bytes());
        trex.extend_from_slice(&0_u32.to_be_bytes());
        let moov = container(
            b"moov",
            &[
                full_atom(b"mvhd", 0, 0, mvhd),
                trak,
                container(b"mvex", &[full_atom(b"trex", 0, 0, trex)]),
            ],
        );
        [atom(b"ftyp", b"iso6\0\0\0\0iso6"), moov].concat()
    }

    fn fragment(id: u32, duration: u32, data: &[u8]) -> Vec<u8> {
        fn trun(duration: u32, size: u32, offset: i32) -> Vec<u8> {
            let mut payload = 1_u32.to_be_bytes().to_vec();
            payload.extend_from_slice(&offset.to_be_bytes());
            payload.extend_from_slice(&duration.to_be_bytes());
            payload.extend_from_slice(&size.to_be_bytes());
            payload.extend_from_slice(&0x0200_0000_u32.to_be_bytes());
            payload.extend_from_slice(&0_i32.to_be_bytes());
            full_atom(b"trun", 1, 0x000f01, payload)
        }
        let tfhd = full_atom(b"tfhd", 0, 0x020000, id.to_be_bytes());
        let tfdt = full_atom(b"tfdt", 0, 0, 0_u32.to_be_bytes());
        let placeholder = container(
            b"moof",
            &[container(
                b"traf",
                &[
                    tfhd.clone(),
                    tfdt.clone(),
                    trun(duration, data.len() as u32, 0),
                ],
            )],
        );
        let moof = container(
            b"moof",
            &[container(
                b"traf",
                &[
                    tfhd,
                    tfdt,
                    trun(duration, data.len() as u32, (placeholder.len() + 8) as i32),
                ],
            )],
        );
        [moof, atom(b"mdat", data)].concat()
    }

    fn write_fixture(path: &Path, id: u32, video: bool) {
        let duration = if video { 40 } else { 1024 };
        std::fs::write(
            path,
            [init_track(id, video), fragment(id, duration, b"sample")].concat(),
        )
        .expect("write fixture");
    }

    #[test]
    fn parses_avc_decoder_configuration() {
        let configuration = [
            1, 100, 0, 40, 0xff, 0xe1, 0, 4, 0x67, 100, 0, 40, 1, 0, 2, 0x68, 0xee,
        ];
        let (sps, pps) = parse_avcc(&configuration).expect("parse avcC");
        assert_eq!(sps, [0x67, 100, 0, 40]);
        assert_eq!(pps, [0x68, 0xee]);
    }

    #[test]
    fn parses_aac_lc_stereo_configuration() {
        let (profile, frequency, channels) =
            parse_audio_specific_config(&[0x11, 0x90]).expect("parse ASC");
        assert_eq!(profile, mp4::AudioObjectType::AacLowComplexity);
        assert_eq!(frequency, mp4::SampleFreqIndex::Freq48000);
        assert_eq!(channels, mp4::ChannelConfig::Stereo);
    }

    #[test]
    fn remuxes_and_independently_verifies_progressive_mp4() {
        let root = std::env::temp_dir().join(format!("crunchydl-mp4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp directory");
        let video = root.join("video.mp4");
        let audio = root.join("audio.mp4");
        let output = root.join("output.mp4");
        write_fixture(&video, 1, true);
        write_fixture(&audio, 2, false);
        let planned = |kind: crate::PlannedTrackKind,
                       locale: Locale,
                       codec: &str,
                       dimensions: Option<(u32, u32)>,
                       sampling_rate: Option<u32>| crate::PlannedTrack {
            kind,
            version_id: "version".to_string(),
            locale,
            codec: codec.to_string(),
            bandwidth: 128_000,
            dimensions,
            sampling_rate,
            segment_count: 1,
            representation_fingerprint: "fingerprint".to_string(),
            encrypted: false,
        };
        let sources = vec![
            (
                video,
                planned(
                    crate::PlannedTrackKind::Video,
                    Locale::ja_JP,
                    "avc1",
                    Some((640, 360)),
                    None,
                ),
            ),
            (
                audio,
                planned(
                    crate::PlannedTrackKind::Audio,
                    Locale::ja_JP,
                    "mp4a",
                    None,
                    Some(48_000),
                ),
            ),
        ];
        let tracks = write_and_verify(
            &output,
            &sources,
            &crate::SynchronizationOptions::default(),
            &crate::CancellationToken::new(),
        )
        .expect("native MP4 remux");
        assert_eq!(tracks.len(), 2);
        assert!(output.is_file());
        let _ = std::fs::remove_dir_all(root);
    }
}
