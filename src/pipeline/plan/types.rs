//! Public and internal playback-plan data structures.

use std::fmt;
use std::time::Duration;

use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{MediaStreamDRM, StreamDrm};

use crate::DownloadWarning;
use crate::chapters::Chapter;
use crate::selection::{
    AudioSelection, ChapterSelection, FallbackPolicy, HardSubSelection, QualitySelection,
    SubtitleSelection, VideoSelection,
};

/// Options used to turn playback manifests into an immutable plan.
#[derive(Clone, Debug, Default)]
pub struct PlanningOptions {
    /// Audio versions to open.
    pub audio: AudioSelection,
    /// Whether equivalent video is shared between dubs.
    pub video: VideoSelection,
    /// Raw video or one exact hardsub locale.
    pub hardsub: HardSubSelection,
    /// Subtitle and caption selection.
    pub subtitles: SubtitleSelection,
    /// Video quality policy.
    pub video_quality: QualitySelection,
    /// Audio quality policy.
    pub audio_quality: QualitySelection,
    /// Quality and locale fallback policy.
    pub fallback: FallbackPolicy,
    /// Chapter source.
    pub chapters: ChapterSelection,
}

/// The kind of a planned media track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PlannedTrackKind {
    /// AVC or another selected video representation.
    Video,
    /// An audio representation.
    Audio,
}

/// Safe, immutable diagnostics for one selected representation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PlannedTrack {
    /// Video or audio.
    pub kind: PlannedTrackKind,
    /// Version content id that supplied this track.
    pub version_id: String,
    /// Audio locale for audio tracks and the source version for video tracks.
    pub locale: Locale,
    /// Codec descriptor from the MPD.
    pub codec: String,
    /// Representation bandwidth.
    pub bandwidth: u64,
    /// Video dimensions when this is a video track.
    pub dimensions: Option<(u32, u32)>,
    /// Audio sampling rate when this is an audio track.
    pub sampling_rate: Option<u32>,
    /// Number of media segments, excluding initialization.
    pub segment_count: usize,
    /// Stable descriptor that excludes signed URL query values.
    pub representation_fingerprint: String,
    /// Whether the representation carries DRM metadata.
    pub encrypted: bool,
}

/// A selected subtitle or caption resource without its signed URL.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PlannedSubtitle {
    /// Subtitle locale.
    pub locale: Locale,
    /// Source format, such as `ass` or `vtt`.
    pub format: String,
    /// Whether this resource is a closed caption.
    pub is_caption: bool,
    /// Whether this is a signs track for one selected audio locale.
    pub is_signs: bool,
    /// Stable resource identity with query and fragment removed.
    pub resource_identity: String,
    /// Stable output track title.
    pub title: String,
    /// Whether players should enable this track by default.
    pub default: bool,
    /// Whether players should force this track on.
    pub forced: bool,
}

pub(crate) struct PreparedSubtitle {
    pub(crate) diagnostic: PlannedSubtitle,
    pub(crate) url: String,
}

/// A redacted immutable playback plan suitable for diagnostics and persistence.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct DownloadPlan {
    /// Requested media id.
    pub media_id: String,
    /// Exact selected tracks in output order.
    pub tracks: Vec<PlannedTrack>,
    /// Deduplicated subtitle and caption resources.
    pub subtitles: Vec<PlannedSubtitle>,
    /// Neutral chapter points.
    #[serde(skip)]
    pub chapters: Vec<Chapter>,
    /// Every configured fallback used during planning.
    #[serde(skip)]
    pub warnings: Vec<DownloadWarning>,
    /// Stable SHA-256 fingerprint over only redacted plan descriptors.
    pub fingerprint: String,
}

pub(crate) struct SourceSegment {
    pub(crate) url: String,
    pub(crate) range: Option<(u64, u64)>,
    pub(crate) duration: Duration,
}

pub(crate) struct PlannedSource {
    pub(crate) diagnostic: PlannedTrack,
    #[allow(dead_code)]
    pub(crate) init: SourceSegment,
    pub(crate) media: Vec<SourceSegment>,
    #[allow(dead_code)]
    pub(crate) drm: Option<MediaStreamDRM>,
    pub(crate) playback_drm: StreamDrm,
}

impl fmt::Debug for PlannedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlannedSource")
            .field("diagnostic", &self.diagnostic)
            .field("media_segments", &self.media.len())
            .finish_non_exhaustive()
    }
}

pub(crate) struct PreparedPlan {
    pub(crate) public: DownloadPlan,
    #[allow(dead_code)]
    pub(crate) sources: Vec<PlannedSource>,
    #[allow(dead_code)]
    pub(crate) subtitles: Vec<PreparedSubtitle>,
}
