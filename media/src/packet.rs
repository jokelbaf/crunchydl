use crate::Timestamp;

/// One compressed sample read from a media fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    /// Track identifier matching [`crate::Track::id`].
    pub track_id: u32,
    /// Decode timestamp after applying the track edit offset.
    pub dts: Timestamp,
    /// Presentation timestamp after composition and edit offsets.
    pub pts: Timestamp,
    /// Sample duration in track ticks.
    pub duration: u32,
    /// Whether this sample is a random-access point.
    pub is_keyframe: bool,
    /// Exact compressed sample payload.
    pub data: Vec<u8>,
}
