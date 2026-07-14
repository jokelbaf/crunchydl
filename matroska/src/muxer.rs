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

mod helpers;
use helpers::{
    ebml_header, effective_number, patch_duration, patch_seek_head, patch_segment_size, relative,
    validate_tracks, write_cluster, write_info, write_void,
};
