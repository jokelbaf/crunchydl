//! Structured progress events and warnings emitted during a download.
//!
//! Library code never writes to a terminal. Progress is reported through
//! [`DownloadEvent`] values delivered to an [`EventSink`], and non-fatal
//! deviations from the requested plan are reported as [`DownloadWarning`] values.

use std::fmt;

use crunchyroll_rs::Locale;

use crate::PlannedTrackKind;

/// A non-fatal deviation from the exact requested plan.
///
/// The library never silently falls back to a lower quality, a different
/// language, or a hardsub. Whenever a configured fallback policy is used, the
/// corresponding warning is recorded and returned to the caller.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadWarning {
    /// A requested audio locale was unavailable and a fallback locale was used.
    AudioFallback {
        /// The audio locale the caller asked for.
        requested: Locale,
        /// The audio locale that was selected instead.
        used: Locale,
    },
    /// A requested audio locale was unavailable and dropped from a multi-locale
    /// selection.
    AudioLocaleUnavailable {
        /// The audio locale the caller asked for.
        requested: Locale,
    },
    /// A requested subtitle locale was not present in any playback version.
    SubtitleUnavailable {
        /// The subtitle locale the caller asked for.
        requested: Locale,
    },
    /// A requested hardsub locale was not available for the stream.
    HardSubUnavailable {
        /// The hardsub locale the caller asked for.
        requested: Locale,
    },
    /// The requested video quality was unavailable and a fallback was chosen.
    VideoQualityFallback {
        /// A human-readable description of the requested quality.
        requested: String,
        /// A human-readable description of the quality that was selected.
        used: String,
    },
    /// The requested audio quality was unavailable and a fallback was chosen.
    AudioQualityFallback {
        /// A human-readable description of the requested quality.
        requested: String,
        /// A human-readable description of the quality that was selected.
        used: String,
    },
    /// A referenced subtitle font could not be resolved under warning policy.
    MissingFont {
        /// Canonical family name referenced by the ASS document.
        family: String,
    },
}

impl fmt::Display for DownloadWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AudioFallback { requested, used } => {
                write!(f, "audio locale {requested} unavailable, using {used}")
            }
            Self::AudioLocaleUnavailable { requested } => {
                write!(f, "audio locale {requested} unavailable")
            }
            Self::SubtitleUnavailable { requested } => {
                write!(f, "subtitle locale {requested} unavailable")
            }
            Self::HardSubUnavailable { requested } => {
                write!(f, "hardsub locale {requested} unavailable")
            }
            Self::VideoQualityFallback { requested, used } => {
                write!(f, "video quality {requested} unavailable, using {used}")
            }
            Self::AudioQualityFallback { requested, used } => {
                write!(f, "audio quality {requested} unavailable, using {used}")
            }
            Self::MissingFont { family } => write!(f, "referenced font {family} was not found"),
        }
    }
}

/// The orchestration state of a download job.
///
/// The states mirror the internal job state machine. Every non-terminal state
/// may transition to [`JobState::Cancelled`] or [`JobState::Failed`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    /// The job has been created but not started.
    Created,
    /// Requested media is being resolved and validated.
    ResolvingMedia,
    /// Playback sessions are being opened for the selected versions.
    OpeningPlaybackSessions,
    /// An immutable track plan is being built.
    PlanningTracks,
    /// Content decryption licenses are being acquired.
    AcquiringLicenses,
    /// Media segments are being downloaded.
    Downloading,
    /// Downloaded fragments are being decrypted and assembled.
    Decrypting,
    /// Subtitles and captions are being processed.
    ProcessingSubtitles,
    /// Tracks are being muxed into the output container.
    Muxing,
    /// The output file is being verified.
    Verifying,
    /// The verified output is being committed atomically.
    Committing,
    /// The job finished successfully.
    Completed,
    /// The job was cancelled by the caller.
    Cancelled,
    /// The job failed.
    Failed,
}

/// A structured event describing download progress.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum DownloadEvent {
    /// The job transitioned to a new [`JobState`].
    StateChanged(JobState),
    /// A non-fatal [`DownloadWarning`] was recorded.
    Warning(DownloadWarning),
    /// One segment was committed atomically to staging.
    SegmentCompleted {
        /// Requested media id.
        media_id: String,
        /// Playback version that owns this representation.
        version_id: String,
        /// Stable redacted representation identity.
        representation_fingerprint: String,
        /// Track role when supplied by the orchestrator.
        track: Option<TransferTrack>,
        /// Zero-based media-segment index.
        index: usize,
        /// Number of media segments now complete.
        completed: usize,
        /// Total number of media segments in the representation.
        total: usize,
        /// Bytes committed by this segment.
        bytes: u64,
        /// Bytes committed for this representation, including initialization.
        completed_bytes: u64,
        /// Expected representation bytes when every request declared a length.
        total_bytes: Option<u64>,
    },
    /// A retryable transfer failed and will be attempted again.
    TransferRetry {
        /// Requested media id.
        media_id: String,
        /// Playback version that owns this representation.
        version_id: String,
        /// Stable redacted representation identity.
        representation_fingerprint: String,
        /// Track role when supplied by the orchestrator.
        track: Option<TransferTrack>,
        /// Zero-based media-segment index, or [`None`] for initialization.
        index: Option<usize>,
        /// Next one-based attempt number.
        attempt: u32,
        /// Delay before the next attempt.
        delay: std::time::Duration,
    },
    /// Bounded item progress within a non-transfer pipeline stage.
    StageProgress {
        /// Active job state.
        state: JobState,
        /// Number of items completed in this stage.
        completed: usize,
        /// Total number of items in this stage.
        total: usize,
    },
    /// A verified output was committed successfully.
    OutputCommitted {
        /// Final output path.
        output: std::path::PathBuf,
    },
}

/// Safe track identity attached to transfer progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferTrack {
    /// Video or audio role.
    pub kind: PlannedTrackKind,
    /// Playback audio locale associated with this representation.
    pub locale: Locale,
}

/// A sink for [`DownloadEvent`] values.
///
/// Any `Fn(DownloadEvent)` closure implements this trait, so callers can pass a
/// closure directly instead of defining a type.
pub trait EventSink: Send + Sync {
    /// Handle a single [`DownloadEvent`].
    fn emit(&self, event: DownloadEvent);
}

impl<F> EventSink for F
where
    F: Fn(DownloadEvent) + Send + Sync,
{
    fn emit(&self, event: DownloadEvent) {
        self(event);
    }
}

impl<T: EventSink + ?Sized> EventSink for std::sync::Arc<T> {
    fn emit(&self, event: DownloadEvent) {
        (**self).emit(event);
    }
}

/// An [`EventSink`] that discards every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: DownloadEvent) {}
}
