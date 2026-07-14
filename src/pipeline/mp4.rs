//! Native progressive MP4 muxing with independent structural verification.

mod language;
use language::iso639_2;

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;
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
