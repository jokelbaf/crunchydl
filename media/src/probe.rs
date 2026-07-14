use std::time::Duration;

use crate::Track;

/// Container-neutral media probe results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// Parsed track metadata.
    pub tracks: Vec<Track>,
    /// Maximum end timestamp observed while iterating packets.
    pub duration: Duration,
    /// Total compressed packet count.
    pub packet_count: u64,
    /// Total compressed packet payload bytes.
    pub byte_count: u64,
}
