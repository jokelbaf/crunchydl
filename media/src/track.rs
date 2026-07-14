use std::time::Duration;

use crate::{Codec, TimeBase};

/// The media role of a parsed track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackKind {
    /// A video track.
    Video,
    /// An audio track.
    Audio,
}

/// One movie edit converted into the track time base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edit {
    /// Presentation duration of the edit.
    pub duration: u64,
    /// Starting media time, or `-1` for an empty edit.
    pub media_time: i64,
}

/// Neutral metadata for one parsed AVC or AAC track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// ISO-BMFF track identifier.
    pub id: u32,
    /// Video or audio role.
    pub kind: TrackKind,
    /// Track media time base.
    pub time_base: TimeBase,
    /// Codec and exact codec-private bytes.
    pub codec: Codec,
    /// Display dimensions for video.
    pub dimensions: Option<(u32, u32)>,
    /// Sampling rate for audio.
    pub sample_rate: Option<u32>,
    /// Channel count for audio.
    pub channels: Option<u16>,
    /// Declared or fragment-derived duration.
    pub duration: Duration,
    /// Ordered edit-list entries, expressed in track ticks.
    pub edits: Vec<Edit>,
}
