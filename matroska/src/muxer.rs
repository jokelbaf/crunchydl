use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;

use crate::attachment::Attachment;
use crate::chapter::Chapter;
use crate::cue::{self, Cue};
use crate::ebml::{id_bytes, size_bytes};
use crate::element::{float64, master, string, uint};
use crate::error::{Error, Result};
use crate::seek::{self, Entry};
use crate::track::{self, Track, TrackType};

const SEEK_RESERVE: usize = 512;

/// One compressed packet assigned to an output track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// One-based output track number.
    pub track_number: u64,
    /// Signed decode timestamp in milliseconds, used for input interleaving.
    pub decode_time_ms: i64,
    /// Signed presentation timestamp in milliseconds.
    pub presentation_time_ms: i64,
    /// Packet duration used to derive segment duration.
    pub duration: Duration,
    /// Whether this packet is a random-access point.
    pub keyframe: bool,
    /// Exact compressed payload.
    pub data: Vec<u8>,
}

/// Segment-level muxing metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuxOptions {
    /// Segment title.
    pub title: Option<String>,
    /// Muxing application identifier.
    pub muxing_application: String,
    /// Writing application identifier.
    pub writing_application: String,
}

impl Default for MuxOptions {
    fn default() -> Self {
        Self {
            title: None,
            muxing_application: "crunchydl".to_string(),
            writing_application: "crunchydl".to_string(),
        }
    }
}

/// Streaming Matroska writer.
pub struct Muxer;

impl Muxer {
    /// Write a complete seekable Matroska segment.
    ///
    /// Packets must be ordered by nondecreasing decode time. Presentation time
    /// may differ, including for AVC B-frames.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported metadata, invalid ordering/timestamps,
    /// overflow, or output I/O failure.
    pub fn write<W, I>(
        output: &mut W,
        tracks: &[Track],
        packets: I,
        chapters: &[Chapter],
        attachments: &[Attachment],
        options: &MuxOptions,
    ) -> Result<()>
    where
        W: Write + Seek,
        I: IntoIterator<Item = Packet>,
    {
        Self::write_fallible(
            output,
            tracks,
            packets.into_iter().map(Ok),
            chapters,
            attachments,
            options,
        )
    }

    /// Write from a fallible streaming packet source.
    ///
    /// This variant lets a demuxer surface an input error without first
    /// collecting the complete media stream.
    ///
    /// # Errors
    ///
    /// Returns the first packet-source, validation, overflow, or I/O error.
    pub fn write_fallible<W, I>(
        output: &mut W,
        tracks: &[Track],
        packets: I,
        chapters: &[Chapter],
        attachments: &[Attachment],
        options: &MuxOptions,
    ) -> Result<()>
    where
        W: Write + Seek,
        I: IntoIterator<Item = Result<Packet>>,
    {
        validate_tracks(tracks)?;
        output.write_all(&ebml_header()?)?;
        output.write_all(&id_bytes(0x1853_8067)?)?;
        let segment_size_position = output.stream_position()?;
        output.write_all(&size_bytes(0, Some(8))?)?;
        let segment_start = output.stream_position()?;
        let seek_position = output.stream_position()?;
        write_void(output, SEEK_RESERVE)?;

        let info_position = relative(output.stream_position()?, segment_start)?;
        let duration_payload_position = write_info(output, options)?;
        let tracks_position = relative(output.stream_position()?, segment_start)?;
        output.write_all(&track::encode(tracks)?)?;
        let chapters_position = if let Some(encoded) = crate::chapter::encode(chapters)? {
            let position = relative(output.stream_position()?, segment_start)?;
            output.write_all(&encoded)?;
            Some(position)
        } else {
            None
        };
        let attachments_position = if let Some(encoded) = crate::attachment::encode(attachments)? {
            let position = relative(output.stream_position()?, segment_start)?;
            output.write_all(&encoded)?;
            Some(position)
        } else {
            None
        };

        let primary_video = tracks
            .iter()
            .enumerate()
            .find(|(_, track)| track.track_type == TrackType::Video)
            .map(|(i, t)| effective_number(i, t));
        let mut cues = Vec::new();
        let mut cluster_time = None;
        let mut last_cluster_time = 0_i64;
        let mut cluster_payload = Vec::new();
        let mut cluster_position = 0;
        let mut last_decode = i64::MIN;
        let mut maximum_end_ms = 0_i64;
        let mut saw_packet = false;

        for packet in packets {
            let packet = packet?;
            let track = tracks.iter().enumerate().find_map(|(index, track)| {
                (effective_number(index, track) == packet.track_number).then_some(track)
            });
            if packet.track_number == 0 || track.is_none() || packet.data.is_empty() {
                return Err(Error::Invalid(
                    "packet references an invalid track or empty payload",
                ));
            }
            if saw_packet && packet.decode_time_ms < last_decode {
                return Err(Error::Invalid("packets are not ordered by decode time"));
            }
            saw_packet = true;
            last_decode = packet.decode_time_ms;
            let pts_ms = packet.presentation_time_ms;
            let needs_cluster = cluster_time.is_none_or(|base| {
                let relative = i128::from(pts_ms) - i128::from(base);
                !(-32_768..=32_767).contains(&relative)
            });
            if needs_cluster {
                if cluster_time.is_some() {
                    write_cluster(output, &cluster_payload)?;
                }
                cluster_payload.clear();
                cluster_position = relative(output.stream_position()?, segment_start)?;
                let base = pts_ms.max(0).max(last_cluster_time);
                let relative = i128::from(pts_ms) - i128::from(base);
                if !(-32_768..=32_767).contains(&relative) {
                    return Err(Error::Invalid(
                        "presentation timestamp is too negative for a cluster",
                    ));
                }
                cluster_time = Some(base);
                last_cluster_time = base;
                cluster_payload.extend(uint(0xe7, base as u64)?);
            }
            let base = cluster_time.expect("cluster initialized");
            let relative_ms = i16::try_from(i128::from(pts_ms) - i128::from(base))
                .map_err(|_| Error::Overflow("block relative timecode"))?;
            let is_primary_keyframe = Some(packet.track_number) == primary_video && packet.keyframe;
            if is_primary_keyframe && let Ok(time_ms) = u64::try_from(pts_ms) {
                cues.push(Cue {
                    time_ms,
                    track: packet.track_number,
                    cluster_position,
                });
            }
            cluster_payload.extend(
                if track.expect("track validated").track_type == TrackType::Subtitle {
                    crate::block::group(
                        packet.track_number,
                        relative_ms,
                        packet.duration,
                        &packet.data,
                    )?
                } else {
                    crate::block::simple(
                        packet.track_number,
                        relative_ms,
                        packet.keyframe,
                        &packet.data,
                    )?
                },
            );
            let duration_ms = i64::try_from(packet.duration.as_millis())
                .map_err(|_| Error::Overflow("packet duration"))?;
            maximum_end_ms = maximum_end_ms.max(
                pts_ms
                    .checked_add(duration_ms)
                    .ok_or(Error::Overflow("packet end"))?,
            );
            if cluster_payload.len() >= 4 * 1024 * 1024 {
                write_cluster(output, &cluster_payload)?;
                cluster_time = None;
                cluster_payload.clear();
            }
        }
        if cluster_time.is_some() {
            write_cluster(output, &cluster_payload)?;
        }
        if !saw_packet {
            return Err(Error::Invalid("no media packets"));
        }

        let cues_position = relative(output.stream_position()?, segment_start)?;
        output.write_all(&cue::encode(&cues)?)?;
        let end = output.stream_position()?;
        patch_duration(
            output,
            duration_payload_position,
            Duration::from_millis(maximum_end_ms.max(0) as u64),
        )?;
        patch_seek_head(
            output,
            seek_position,
            &[
                Entry {
                    id: 0x1549_a966,
                    position: info_position,
                },
                Entry {
                    id: 0x1654_ae6b,
                    position: tracks_position,
                },
                Entry {
                    id: 0x1c53_bb6b,
                    position: cues_position,
                },
            ],
            chapters_position,
            attachments_position,
        )?;
        patch_segment_size(
            output,
            segment_size_position,
            end.checked_sub(segment_start)
                .ok_or(Error::Overflow("segment size"))?,
        )?;
        output.seek(SeekFrom::Start(end))?;
        output.flush()?;
        Ok(())
    }
}

fn validate_tracks(tracks: &[Track]) -> Result<()> {
    if tracks.is_empty() {
        return Err(Error::Invalid("no tracks"));
    }
    let mut numbers = Vec::new();
    let mut video_defaults = 0;
    for (index, track) in tracks.iter().enumerate() {
        track.validate()?;
        let number = effective_number(index, track);
        if number == 0 || numbers.contains(&number) {
            return Err(Error::Invalid("track numbers must be unique and nonzero"));
        }
        numbers.push(number);
        if track.track_type == TrackType::Video && track.default {
            video_defaults += 1;
        }
    }
    if video_defaults > 1 {
        return Err(Error::Invalid("multiple default video tracks"));
    }
    Ok(())
}

fn effective_number(index: usize, track: &Track) -> u64 {
    if track.number == 0 {
        index as u64 + 1
    } else {
        track.number
    }
}

fn ebml_header() -> Result<Vec<u8>> {
    master(
        0x1a45_dfa3,
        [
            uint(0x4286, 1)?,
            uint(0x42f7, 1)?,
            uint(0x42f2, 4)?,
            uint(0x42f3, 8)?,
            string(0x4282, "matroska")?,
            uint(0x4287, 4)?,
            uint(0x4285, 2)?,
        ],
    )
}

fn write_info<W: Write + Seek>(output: &mut W, options: &MuxOptions) -> Result<u64> {
    let mut children = vec![
        uint(0x002a_d7b1, 1_000_000)?,
        string(0x4d80, &options.muxing_application)?,
        string(0x5741, &options.writing_application)?,
    ];
    if let Some(title) = &options.title {
        children.push(string(0x7ba9, title)?);
    }
    let info_start = output.stream_position()?;
    let duration = float64(0x4489, 0.0)?;
    children.push(duration);
    let encoded = master(0x1549_a966, children)?;
    let marker = id_bytes(0x4489)?;
    let offset = encoded
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(Error::Invalid("duration marker missing"))?;
    let payload_position = info_start + offset as u64 + marker.len() as u64 + 1;
    output.write_all(&encoded)?;
    Ok(payload_position)
}

fn write_cluster<W: Write>(output: &mut W, payload: &[u8]) -> Result<()> {
    output.write_all(&crate::element::raw(0x1f43_b675, payload)?)?;
    Ok(())
}
fn relative(position: u64, start: u64) -> Result<u64> {
    position
        .checked_sub(start)
        .ok_or(Error::Overflow("relative file offset"))
}
fn write_void<W: Write>(output: &mut W, total: usize) -> Result<()> {
    let id = id_bytes(0xec)?;
    for width in 1..=8 {
        if total > id.len() + width {
            let payload = total - id.len() - width;
            if let Ok(size) = size_bytes(payload as u64, Some(width)) {
                output.write_all(&id)?;
                output.write_all(&size)?;
                output.write_all(&vec![0; payload])?;
                return Ok(());
            }
        }
    }
    Err(Error::Invalid("void reserve cannot be encoded"))
}

fn patch_duration<W: Write + Seek>(
    output: &mut W,
    position: u64,
    duration: Duration,
) -> Result<()> {
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&(duration.as_secs_f64() * 1000.0).to_be_bytes())?;
    Ok(())
}

fn patch_seek_head<W: Write + Seek>(
    output: &mut W,
    position: u64,
    base: &[Entry],
    chapters: Option<u64>,
    attachments: Option<u64>,
) -> Result<()> {
    let mut entries = base
        .iter()
        .map(|entry| Entry {
            id: entry.id,
            position: entry.position,
        })
        .collect::<Vec<_>>();
    if let Some(position) = chapters {
        entries.push(Entry {
            id: 0x1043_a770,
            position,
        });
    }
    if let Some(position) = attachments {
        entries.push(Entry {
            id: 0x1941_a469,
            position,
        });
    }
    let encoded = seek::encode(&entries)?;
    if encoded.len() + 2 > SEEK_RESERVE {
        return Err(Error::Overflow("seek head reserve"));
    }
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&encoded)?;
    write_void(output, SEEK_RESERVE - encoded.len())
}

fn patch_segment_size<W: Write + Seek>(output: &mut W, position: u64, size: u64) -> Result<()> {
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&size_bytes(size, Some(8))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{AudioSettings, Language, TrackCodec, TrackSettings, VideoSettings};
    use std::io::Cursor;

    fn language() -> Language {
        Language {
            legacy: "eng".into(),
            ietf: "en-US".into(),
        }
    }
    fn tracks() -> Vec<Track> {
        vec![
        Track { number: 1, uid: 11, track_type: TrackType::Video, codec: TrackCodec::Avc(vec![1, 100, 0, 31]), settings: TrackSettings::Video(VideoSettings { width: 1920, height: 1080 }), name: Some("Video".into()), language: None, default: true, forced: false, hearing_impaired: false, visual_impaired: false, original: true, commentary: false },
        Track { number: 2, uid: 12, track_type: TrackType::Audio, codec: TrackCodec::Aac(vec![0x12, 0x10]), settings: TrackSettings::Audio(AudioSettings { sampling_frequency: 48_000.0, channels: 2 }), name: Some("English".into()), language: Some(language()), default: true, forced: false, hearing_impaired: false, visual_impaired: false, original: true, commentary: false },
        Track { number: 3, uid: 13, track_type: TrackType::Subtitle, codec: TrackCodec::Ass("[Script Info]\n[V4+ Styles]\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".into()), settings: TrackSettings::Subtitle, name: Some("English".into()), language: Some(language()), default: false, forced: true, hearing_impaired: false, visual_impaired: false, original: false, commentary: false },
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
}
