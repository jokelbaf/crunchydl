//! End-to-end download orchestration and output policy.

use std::fs::File;
use std::future::Future;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{MediaStreamDRMType, StreamDrm, StreamPlatform};
use drm::{ContentKey, ContentType, DrmProvider, DrmRequest, inspect_encryption};
use matroska_writer as mkv;
use media::{Codec, FragmentedMp4, TrackKind};
use serde::{Deserialize, Serialize};

use crate::api::ProductionApi;
use crate::archive::{Archive, ArchiveKey};
use crate::filename::{FilenameTemplate, OutputLayoutTemplate, output_path};
use crate::plan::{PlannedSource, PlannedTrackKind, PreparedPlan};
use crate::redaction::url_identity;
use crate::session::SessionGuard;
use crate::subtitle::download_selected;
use crate::{
    CancellationToken, CrunchyrollLicenseTransport, DownloadEvent, DownloadWarning, Error,
    EventSink, FontPolicy, FontResolver, JobState, MediaRequest, NoopSink, OverwritePolicy,
    PlanningOptions, ResolvedMedia, SegmentRequest, SubtitleProcessingOptions, TransferEngine,
    TransferOptions,
};

mod verification;
use verification::*;

/// DRM system whose PSSH should be sent to the configured provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrmSystem {
    /// Microsoft PlayReady.
    PlayReady,
    /// Google Widevine.
    Widevine,
}

impl DrmSystem {
    fn matches_name(self, name: &str) -> bool {
        name.eq_ignore_ascii_case(self.endpoint_name())
    }

    const fn endpoint_name(self) -> &'static str {
        match self {
            Self::PlayReady => "playReady",
            Self::Widevine => "widevine",
        }
    }
}

/// Whether staging is retained after success or failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetentionPolicy {
    /// Remove staging after every terminal result.
    #[default]
    Delete,
    /// Retain staging after failure or cancellation, but remove it after success.
    KeepOnFailure,
    /// Always retain staging.
    KeepAlways,
}

/// Filesystem and metadata options for a completed output.
#[derive(Clone, Debug)]
pub struct OutputOptions {
    /// Root beneath which the output is committed.
    pub root: PathBuf,
    /// Compiled filename template.
    pub filename: FilenameTemplate,
    /// Optional hierarchical layout. When set, this takes precedence over
    /// [`OutputOptions::filename`].
    pub layout: Option<OutputLayoutTemplate>,
    /// Native output container.
    pub container: crate::Container,
    /// Existing-output policy.
    pub overwrite: OverwritePolicy,
    /// Staging retention policy.
    pub retention: RetentionPolicy,
    /// Maximum sanitized filename component length before the `.mkv` suffix.
    pub max_component_length: usize,
}

impl OutputOptions {
    /// Create Matroska output options rooted at `root`.
    ///
    /// # Errors
    ///
    /// Returns an error only if the built-in filename template is invalid.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, Error> {
        Ok(Self {
            root: root.into(),
            filename: FilenameTemplate::compile("{series} - {episode} - {title} [{media_id}]")?,
            layout: None,
            container: crate::Container::Matroska,
            overwrite: OverwritePolicy::Fail,
            retention: RetentionPolicy::Delete,
            max_component_length: 180,
        })
    }
}

/// Complete typed request for one download job.
pub struct DownloadRequest {
    /// Caller-selected Crunchyroll media object.
    pub media: MediaRequest,
    /// Version, track, quality, subtitle, and chapter selection.
    pub planning: PlanningOptions,
    /// Output and staging behavior.
    pub output: OutputOptions,
    /// Segment concurrency and retry behavior.
    pub transfer: TransferOptions,
    /// Subtitle normalization behavior.
    pub subtitles: SubtitleProcessingOptions,
    /// Strict multi-audio timeline compatibility and explicit offsets.
    pub synchronization: SynchronizationOptions,
    /// Cooperative cancellation handle.
    pub cancellation: CancellationToken,
}

/// Multi-audio timeline compatibility policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynchronizationOptions {
    /// Reject materially different audio durations when true.
    pub strict: bool,
    /// Maximum duration difference accepted without content-based alignment.
    pub tolerance: Duration,
    /// Explicit signed millisecond offsets keyed by audio locale.
    pub offsets: Vec<(Locale, i64)>,
}

impl Default for SynchronizationOptions {
    fn default() -> Self {
        Self {
            strict: true,
            tolerance: Duration::from_secs(2),
            offsets: Vec::new(),
        }
    }
}

/// Safe output-track diagnostics returned after verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputTrack {
    /// Container codec identifier.
    pub codec: String,
    /// Optional BCP 47 language.
    pub language: Option<String>,
    /// Optional human-readable name.
    pub name: Option<String>,
    /// Default-track flag.
    pub default: bool,
    /// Forced-track flag.
    pub forced: bool,
}

/// Result of a committed or archive-reused download.
#[derive(Clone, Debug)]
pub struct DownloadResult {
    /// Final committed output.
    pub output: PathBuf,
    /// Crunchyroll media id.
    pub media_id: String,
    /// Verified output tracks.
    pub tracks: Vec<OutputTrack>,
    /// Structured non-fatal warnings.
    pub warnings: Vec<DownloadWarning>,
}

pub(crate) struct DrmConfiguration {
    pub(crate) provider: Arc<dyn DrmProvider>,
    pub(crate) system: DrmSystem,
    pub(crate) endpoint_override: Option<String>,
}

pub(crate) struct RuntimeConfiguration {
    pub(crate) drm: Option<DrmConfiguration>,
    pub(crate) archive: Option<Arc<dyn Archive>>,
    pub(crate) font_resolver: Option<Arc<dyn FontResolver>>,
    pub(crate) font_policy: FontPolicy,
    pub(crate) events: Arc<dyn EventSink>,
}

mod assemble;
mod license;
mod mux;
mod output;
mod pipeline;
mod runner;
mod subtitle_packets;

pub(crate) use assemble::assemble_track;
pub(crate) use license::{PlaybackRefresher, license_endpoint, select_pssh};
pub(crate) use mux::{language, mux_inputs};
pub(crate) use output::{mux_matroska, source_transfer_plan, validate_container_contract};
pub(crate) use pipeline::execute_pipeline;
pub(crate) use runner::run;
pub(crate) use subtitle_packets::ass_packets;
pub(crate) struct StateEmitter {
    events: Arc<dyn EventSink>,
    cancellation: CancellationToken,
}

struct MissingFontResolver;

impl FontResolver for MissingFontResolver {
    fn resolve(&self, _family: &str) -> Result<Option<crate::ResolvedFont>, Error> {
        Ok(None)
    }
}
impl StateEmitter {
    fn new(events: Arc<dyn EventSink>, cancellation: CancellationToken) -> Self {
        Self {
            events,
            cancellation,
        }
    }
    fn enter(&self, state: JobState) -> Result<(), Error> {
        self.cancellation.check()?;
        self.events.emit(DownloadEvent::StateChanged(state));
        self.cancellation.check()
    }
}

impl Default for RuntimeConfiguration {
    fn default() -> Self {
        Self {
            drm: None,
            archive: None,
            font_resolver: None,
            font_policy: FontPolicy::Warn,
            events: Arc::new(NoopSink),
        }
    }
}
