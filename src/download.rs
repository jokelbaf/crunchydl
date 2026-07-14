//! End-to-end download orchestration and output policy.

use std::fs::File;
use std::future::Future;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{MediaStreamDRMType, StreamPlatform};
use drm::{ContentKey, ContentType, DrmProvider, DrmRequest, inspect_encryption};
use matroska_writer as mkv;
use media::{Codec, FragmentedMp4, TrackKind};
use serde::{Deserialize, Serialize};

use crate::api::ProductionApi;
use crate::archive::{Archive, ArchiveEntry, ArchiveKey};
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
    /// Return the production Crunchyroll license endpoint for this DRM system.
    ///
    /// Callers normally do not need to configure an endpoint. An explicit
    /// override remains available through
    /// [`DownloaderBuilder::drm_with_license_endpoint`](crate::DownloaderBuilder::drm_with_license_endpoint)
    /// for testing, proxies, or future service changes.
    #[must_use]
    pub const fn default_license_endpoint(self) -> &'static str {
        match self {
            Self::PlayReady => {
                "https://cr-license-proxy.prd.crunchyrollsvc.com/v1/license/playReady"
            }
            Self::Widevine => "https://cr-license-proxy.prd.crunchyrollsvc.com/v1/license/widevine",
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
    pub(crate) endpoint: String,
}

pub(crate) struct RuntimeConfiguration {
    pub(crate) drm: Option<DrmConfiguration>,
    pub(crate) archive: Option<Arc<dyn Archive>>,
    pub(crate) font_resolver: Option<Arc<dyn FontResolver>>,
    pub(crate) font_policy: FontPolicy,
    pub(crate) events: Arc<dyn EventSink>,
}

pub(crate) async fn run(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: DownloadRequest,
) -> Result<DownloadResult, Error> {
    let cancellation = request.cancellation.clone();
    let states = StateEmitter::new(runtime.events.clone(), cancellation.clone());
    let result = run_inner(api, crunchyroll, platform, runtime, request, &states).await;
    match result {
        Ok(value) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Completed));
            Ok(value)
        }
        Err(Error::Cancelled) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Cancelled));
            Err(Error::Cancelled)
        }
        Err(error) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Failed));
            Err(error)
        }
    }
}

async fn run_inner(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: DownloadRequest,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    let cancellation = request.cancellation.clone();
    states.enter(JobState::Created)?;
    states.enter(JobState::ResolvingMedia)?;
    let media = request.media.resolve();
    validate_media(&media)?;
    let archive_key = ArchiveKey {
        media_id: media.content_id.clone(),
        selection_fingerprint: crate::redaction::fingerprint([
            format!("{:?}", request.planning),
            format!("{:?}", request.subtitles),
            format!("{:?}", request.synchronization),
            format!("{:?}", request.output.container),
        ]),
    };
    if let Some(archive) = &runtime.archive
        && let Some(entry) = archive.find(&archive_key)?
    {
        return Ok(DownloadResult {
            output: entry.output,
            media_id: media.content_id,
            tracks: entry.tracks,
            warnings: Vec::new(),
        });
    }
    std::fs::create_dir_all(&request.output.root)
        .map_err(|error| path_error(&request.output.root, error))?;

    states.enter(JobState::OpeningPlaybackSessions)?;
    let mut guard = SessionGuard::new(api);
    states.enter(JobState::PlanningTracks)?;
    let prepared = crate::plan::prepare_with_guard(
        api,
        platform,
        &media,
        &request.planning,
        &cancellation,
        &mut guard,
    )
    .await;
    let result = match prepared {
        Ok(prepared) => {
            run_prepared(
                api,
                crunchyroll,
                platform,
                runtime,
                &request,
                &media,
                &archive_key,
                prepared,
                states,
            )
            .await
        }
        Err(error) => Err(error),
    };
    let finalization = guard.finalize().await;
    match (result, finalization) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) | (Err(error), _) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_prepared(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: PreparedPlan,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    validate_container_contract(request, &prepared)?;
    for warning in &prepared.public.warnings {
        runtime.events.emit(DownloadEvent::Warning(warning.clone()));
    }
    let height = prepared
        .public
        .tracks
        .iter()
        .filter_map(|track| track.dimensions.map(|(_, height)| height))
        .max();
    let audio = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Audio)
        .map(|track| track.locale.to_string())
        .collect::<Vec<_>>();
    let mut relative = request.output.layout.as_ref().map_or_else(
        || {
            request
                .output
                .filename
                .render(media, height, &audio, request.output.max_component_length)
                .into()
        },
        |layout| layout.render(media, height, &audio, request.output.max_component_length),
    );
    relative.set_extension(match request.output.container {
        crate::Container::Matroska => "mkv",
        crate::Container::Mp4 => "mp4",
    });
    let destination = output_path(&request.output.root, &relative)?;
    if destination.exists() && request.output.overwrite != OverwritePolicy::Replace {
        return Err(Error::Filesystem(format!(
            "output already exists: {}",
            destination.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| path_error(parent, error))?;
    }
    let staging = request
        .output
        .root
        .join(".crunchydl-staging")
        .join(safe_component(&media.content_id))
        .join(&prepared.public.fingerprint[..16]);
    std::fs::create_dir_all(&staging).map_err(|error| path_error(&staging, error))?;
    let outcome = execute_pipeline(
        api,
        crunchyroll,
        platform,
        runtime,
        request,
        media,
        archive_key,
        &prepared,
        &staging,
        &destination,
        states,
    )
    .await;
    cleanup_staging(&staging, request.output.retention, outcome.is_ok());
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn execute_pipeline(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: &PreparedPlan,
    staging: &Path,
    destination: &Path,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    let refresher = PlaybackRefresher::new(
        api,
        platform,
        media,
        &request.planning,
        &request.cancellation,
    );
    let result = execute_pipeline_inner(
        api,
        crunchyroll,
        runtime,
        request,
        media,
        archive_key,
        prepared,
        staging,
        destination,
        states,
        &refresher,
    )
    .await;
    let finalization = refresher.finalize().await;
    match (result, finalization) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) | (Err(error), _) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_pipeline_inner(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: &PreparedPlan,
    staging: &Path,
    destination: &Path,
    states: &StateEmitter,
    refresher: &PlaybackRefresher<'_>,
) -> Result<DownloadResult, Error> {
    let transfer = TransferEngine::with_client(crunchyroll.client(), request.transfer.clone())?
        .event_sink(runtime.events.clone());
    let transport = CrunchyrollLicenseTransport::new(crunchyroll.clone());
    let mut warnings = prepared.public.warnings.clone();
    states.enter(JobState::AcquiringLicenses)?;
    let mut licensed = Vec::with_capacity(prepared.sources.len());
    for (index, source) in prepared.sources.iter().enumerate() {
        let transfer_plan =
            source_transfer_plan(source, &prepared.public.fingerprint, &media.content_id);
        let init_path = transfer
            .transfer_init(&transfer_plan, staging, &request.cancellation)
            .await?;
        let init = std::fs::read(&init_path).map_err(|error| path_error(&init_path, error))?;
        let key = if let Some(drm) = &source.drm {
            let config = runtime
                .drm
                .as_ref()
                .ok_or_else(|| Error::License(drm::Error::License))?;
            let info = inspect_encryption(&init)?;
            let pssh = select_pssh(drm, config.system)?;
            let request_data = DrmRequest {
                endpoint: config.endpoint.clone(),
                content_id: source.diagnostic.version_id.clone(),
                playback_token: drm.token.clone(),
                content_type: match source.diagnostic.kind {
                    PlannedTrackKind::Video => ContentType::Video,
                    PlannedTrackKind::Audio => ContentType::Audio,
                },
                pssh,
            };
            let keys = config
                .provider
                .acquire_keys(request_data, &transport)
                .await?;
            Some(keys.require(info.default_kid)?.clone())
        } else {
            None
        };
        licensed.push((source, transfer_plan, key));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::AcquiringLicenses,
            completed: index + 1,
            total: prepared.sources.len(),
        });
    }

    states.enter(JobState::Downloading)?;
    let mut transferred = Vec::with_capacity(licensed.len());
    let licensed_count = licensed.len();
    for (index, (source, transfer_plan, key)) in licensed.into_iter().enumerate() {
        let result = transfer
            .transfer_with_refresh(transfer_plan, staging, &request.cancellation, refresher)
            .await?;
        transferred.push((source, result, key));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::Downloading,
            completed: index + 1,
            total: licensed_count,
        });
    }

    states.enter(JobState::Decrypting)?;
    let mut assembled = Vec::with_capacity(transferred.len());
    let transferred_count = transferred.len();
    for (index, (source, transferred, key)) in transferred.into_iter().enumerate() {
        let output = staging.join(format!("track-{index:03}.mp4"));
        assemble_track(
            &transferred.init,
            &transferred.segments,
            &output,
            key.as_ref(),
            &request.cancellation,
        )?;
        let file = File::open(&output).map_err(|error| path_error(&output, error))?;
        let parsed = FragmentedMp4::open(BufReader::new(file))?;
        if parsed.tracks().len() != 1 {
            return Err(Error::MediaParse(media::Error::Unsupported(
                "assembled representation must contain one track",
            )));
        }
        assembled.push((output, source.diagnostic.clone()));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::Decrypting,
            completed: index + 1,
            total: transferred_count,
        });
    }

    states.enter(JobState::ProcessingSubtitles)?;
    let subtitles = download_selected(api, &prepared.subtitles, &request.subtitles).await?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::ProcessingSubtitles,
        completed: subtitles.len(),
        total: prepared.subtitles.len(),
    });
    let missing = MissingFontResolver;
    let resolver: &dyn FontResolver = runtime.font_resolver.as_deref().unwrap_or(&missing);
    let (fonts, font_warnings) =
        crate::resolve_referenced_fonts(&subtitles, resolver, runtime.font_policy)?;
    for warning in &font_warnings {
        runtime.events.emit(DownloadEvent::Warning(warning.clone()));
    }
    warnings.extend(font_warnings);

    states.enter(JobState::Muxing)?;
    let temporary = staging.join(match request.output.container {
        crate::Container::Matroska => "output.mkv.part",
        crate::Container::Mp4 => "output.mp4.part",
    });
    let output_tracks = match request.output.container {
        crate::Container::Matroska => mux_matroska(
            &temporary,
            &assembled,
            &subtitles,
            &fonts,
            &prepared.public.chapters,
            media,
            &request.synchronization,
            &request.cancellation,
        )?,
        crate::Container::Mp4 => crate::mp4_output::write_and_verify(
            &temporary,
            &assembled,
            &request.synchronization,
            &request.cancellation,
        )?,
    };
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Muxing,
        completed: 1,
        total: 1,
    });

    states.enter(JobState::Verifying)?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Verifying,
        completed: 1,
        total: 1,
    });
    states.enter(JobState::Committing)?;
    commit_output(
        &temporary,
        destination,
        request.output.overwrite,
        runtime.archive.as_deref(),
        archive_key,
        &output_tracks,
    )?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Committing,
        completed: 1,
        total: 1,
    });
    runtime.events.emit(DownloadEvent::OutputCommitted {
        output: destination.to_path_buf(),
    });
    Ok(DownloadResult {
        output: destination.to_path_buf(),
        media_id: media.content_id.clone(),
        tracks: output_tracks,
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn mux_matroska(
    temporary: &Path,
    assembled: &[(PathBuf, crate::PlannedTrack)],
    subtitles: &[crate::SubtitleTrack],
    fonts: &[crate::ResolvedFont],
    source_chapters: &[crate::Chapter],
    media: &ResolvedMedia,
    synchronization: &SynchronizationOptions,
    cancellation: &CancellationToken,
) -> Result<Vec<OutputTrack>, Error> {
    let (tracks, packets) = mux_inputs(assembled, subtitles, synchronization, cancellation)?;
    let chapters = source_chapters
        .iter()
        .map(|chapter| mkv::Chapter {
            start: chapter.start,
            title: chapter.title.clone(),
            language: language(&Locale::en_US),
        })
        .collect::<Vec<_>>();
    let attachments = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| mkv::Attachment {
            filename: font.filename.clone(),
            mime_type: font.mime_type.clone(),
            uid: index as u64 + 1,
            data: font.data.clone(),
        })
        .collect::<Vec<_>>();
    let mut output =
        BufWriter::new(File::create(temporary).map_err(|error| path_error(temporary, error))?);
    mkv::Muxer::write_fallible(
        &mut output,
        &tracks,
        packets,
        &chapters,
        &attachments,
        &mkv::MuxOptions {
            title: Some(media.title.clone()),
            ..mkv::MuxOptions::default()
        },
    )
    .map_err(|error| match error {
        mkv::Error::Cancelled => Error::Cancelled,
        error => Error::Mux(error.to_string()),
    })?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| path_error(temporary, error))?;
    drop(output);
    verify(temporary, tracks.len(), attachments.len(), chapters.len())
}

fn validate_container_contract(
    request: &DownloadRequest,
    prepared: &PreparedPlan,
) -> Result<(), Error> {
    if request.output.container != crate::Container::Mp4 {
        return Ok(());
    }
    if !prepared.subtitles.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not silently discard subtitles; select no subtitles or use Matroska"
                .into(),
        ));
    }
    if !prepared.public.chapters.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not silently discard chapters; disable chapters or use Matroska"
                .into(),
        ));
    }
    if !request.synchronization.offsets.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not support explicit track offsets".into(),
        ));
    }
    let video = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Video)
        .count();
    let audio = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Audio)
        .count();
    if video != 1 || audio == 0 {
        return Err(Error::Mux(
            "MP4 output requires exactly one AVC video and at least one AAC audio track".into(),
        ));
    }
    Ok(())
}

fn source_transfer_plan(
    source: &PlannedSource,
    fingerprint: &str,
    media_id: &str,
) -> crate::RepresentationTransferPlan {
    let request = |segment: &crate::plan::SourceSegment| {
        SegmentRequest::new(
            segment.url.clone(),
            segment.range,
            url_identity(&segment.url),
            segment
                .range
                .map(|(start, end)| end.saturating_sub(start).saturating_add(1)),
        )
    };
    crate::RepresentationTransferPlan {
        media_id: media_id.to_string(),
        version_id: source.diagnostic.version_id.clone(),
        plan_fingerprint: fingerprint.to_string(),
        representation_fingerprint: source.diagnostic.representation_fingerprint.clone(),
        track: Some(crate::TransferTrack {
            kind: source.diagnostic.kind,
            locale: source.diagnostic.locale.clone(),
        }),
        init: request(&source.init),
        segments: source.media.iter().map(request).collect(),
    }
}

struct PlaybackRefresher<'a> {
    api: &'a ProductionApi,
    platform: &'a StreamPlatform,
    media: &'a ResolvedMedia,
    options: &'a PlanningOptions,
    cancellation: &'a CancellationToken,
    sessions: tokio::sync::Mutex<Vec<SessionGuard<'a, ProductionApi>>>,
}

impl<'a> PlaybackRefresher<'a> {
    fn new(
        api: &'a ProductionApi,
        platform: &'a StreamPlatform,
        media: &'a ResolvedMedia,
        options: &'a PlanningOptions,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            api,
            platform,
            media,
            options,
            cancellation,
            sessions: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    async fn finalize(&self) -> Result<(), Error> {
        let sessions = {
            let mut sessions = self.sessions.lock().await;
            std::mem::take(&mut *sessions)
        };
        let mut first_error = None;
        for guard in sessions {
            if let Err(error) = guard.finalize().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl crate::RepresentationRefresher for PlaybackRefresher<'_> {
    fn refresh<'a>(
        &'a self,
        expired: &'a crate::RepresentationTransferPlan,
    ) -> Pin<Box<dyn Future<Output = Result<crate::RepresentationTransferPlan, Error>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut guard = SessionGuard::new(self.api);
            let prepared = match crate::plan::prepare_with_guard(
                self.api,
                self.platform,
                self.media,
                self.options,
                self.cancellation,
                &mut guard,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = guard.finalize().await;
                    return Err(error);
                }
            };
            let source = prepared
                .sources
                .iter()
                .find(|source| {
                    source.diagnostic.version_id == expired.version_id
                        && source.diagnostic.representation_fingerprint
                            == expired.representation_fingerprint
                })
                .ok_or_else(|| {
                    Error::ResumeMismatch(
                        "refreshed playback omitted the selected representation".into(),
                    )
                })?;
            let refreshed =
                source_transfer_plan(source, &expired.plan_fingerprint, &expired.media_id);
            self.sessions.lock().await.push(guard);
            Ok(refreshed)
        })
    }
}

fn select_pssh(
    drm: &crunchyroll_rs::media::MediaStreamDRM,
    system: DrmSystem,
) -> Result<Vec<u8>, Error> {
    let encoded = drm
        .types
        .iter()
        .find_map(|kind| match (system, kind) {
            (DrmSystem::PlayReady, MediaStreamDRMType::Playready { pro, pssh }) => pssh
                .as_ref()
                .and_then(|values| values.first())
                .or(pro.as_ref()),
            (DrmSystem::Widevine, MediaStreamDRMType::Widevine { pssh }) => pssh.first(),
            _ => None,
        })
        .ok_or_else(|| Error::License(drm::Error::License))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| Error::License(drm::Error::License))
}

fn assemble_track(
    init_path: &Path,
    segments: &[PathBuf],
    output_path: &Path,
    key: Option<&ContentKey>,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    cancellation.check()?;
    let init = std::fs::read(init_path).map_err(|error| path_error(init_path, error))?;
    let mut output =
        BufWriter::new(File::create(output_path).map_err(|error| path_error(output_path, error))?);
    if let Some(key) = key {
        let decrypter = crate::CencDecrypter::new(&init, key)
            .map_err(|error| Error::Decrypt(error.to_string()))?;
        decrypter
            .assemble(&init, std::iter::empty(), &mut output)
            .map_err(|error| Error::Decrypt(error.to_string()))?;
        for segment in segments {
            cancellation.check()?;
            let encrypted = std::fs::read(segment).map_err(|error| path_error(segment, error))?;
            output
                .write_all(
                    &decrypter
                        .decrypt_fragment(encrypted)
                        .map_err(|error| Error::Decrypt(error.to_string()))?,
                )
                .map_err(|error| path_error(output_path, error))?;
        }
    } else {
        output
            .write_all(&init)
            .map_err(|error| path_error(output_path, error))?;
        for segment in segments {
            cancellation.check()?;
            let mut input = File::open(segment).map_err(|error| path_error(segment, error))?;
            std::io::copy(&mut input, &mut output)
                .map_err(|error| path_error(output_path, error))?;
        }
    }
    output
        .flush()
        .map_err(|error| path_error(output_path, error))?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| path_error(output_path, error))
}

fn mux_inputs(
    paths: &[(PathBuf, crate::PlannedTrack)],
    subtitles: &[crate::SubtitleTrack],
    synchronization: &SynchronizationOptions,
    cancellation: &CancellationToken,
) -> Result<(Vec<mkv::Track>, PacketStreams), Error> {
    let mut tracks = Vec::new();
    let mut streams = Vec::new();
    let mut audio_default = true;
    let mut video_default = true;
    let mut reference_audio_duration: Option<Duration> = None;
    for (path, diagnostic) in paths {
        let parsed = FragmentedMp4::open(BufReader::new(
            File::open(path).map_err(|error| path_error(path, error))?,
        ))?;
        let media_track = parsed.tracks()[0].clone();
        let number = tracks.len() as u64 + 1;
        let offset = synchronization
            .offsets
            .iter()
            .find_map(|(locale, offset)| (locale == &diagnostic.locale).then_some(*offset))
            .unwrap_or(0);
        let (track_type, codec, settings, default) = match (&media_track.kind, &media_track.codec) {
            (TrackKind::Video, Codec::Avc { configuration }) => (
                mkv::TrackType::Video,
                mkv::TrackCodec::Avc(configuration.clone()),
                mkv::TrackSettings::Video(mkv::VideoSettings {
                    width: media_track
                        .dimensions
                        .ok_or_else(|| Error::Mux("video dimensions missing".into()))?
                        .0,
                    height: media_track
                        .dimensions
                        .ok_or_else(|| Error::Mux("video dimensions missing".into()))?
                        .1,
                }),
                {
                    let default = video_default;
                    video_default = false;
                    default
                },
            ),
            (
                TrackKind::Audio,
                Codec::Aac {
                    audio_specific_config,
                },
            ) => {
                let actual_duration = FragmentedMp4::open(BufReader::new(
                    File::open(path).map_err(|error| path_error(path, error))?,
                ))?
                .probe()?
                .duration;
                if let Some(reference) = reference_audio_duration {
                    let difference = reference.abs_diff(actual_duration);
                    if synchronization.strict && difference > synchronization.tolerance {
                        return Err(Error::IncompatibleTimelines {
                            locale: diagnostic.locale.to_string(),
                        });
                    }
                } else {
                    reference_audio_duration = Some(actual_duration);
                }
                let value = (
                    mkv::TrackType::Audio,
                    mkv::TrackCodec::Aac(audio_specific_config.clone()),
                    mkv::TrackSettings::Audio(mkv::AudioSettings {
                        sampling_frequency: f64::from(
                            media_track
                                .sample_rate
                                .ok_or_else(|| Error::Mux("audio sampling rate missing".into()))?,
                        ),
                        channels: media_track
                            .channels
                            .ok_or_else(|| Error::Mux("audio channels missing".into()))?,
                    }),
                    audio_default,
                );
                audio_default = false;
                value
            }
            _ => return Err(Error::Mux("unsupported media codec".into())),
        };
        tracks.push(mkv::Track {
            number,
            uid: 0,
            track_type,
            codec,
            settings,
            name: (track_type == mkv::TrackType::Audio)
                .then(|| crate::locale_display_name(&diagnostic.locale)),
            language: (track_type == mkv::TrackType::Audio).then(|| language(&diagnostic.locale)),
            default,
            forced: false,
            hearing_impaired: false,
            visual_impaired: false,
            original: default,
            commentary: false,
        });
        let packets = parsed.packets().map(move |packet| {
            let packet = packet.map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            let decode_time_ms = timestamp_millis(packet.dts)
                .map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            let presentation_time_ms = timestamp_millis(packet.pts)
                .map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            Ok(mkv::Packet {
                track_number: number,
                decode_time_ms: decode_time_ms
                    .checked_add(offset)
                    .ok_or(mkv::Error::Overflow("explicit track decode offset"))?,
                presentation_time_ms: presentation_time_ms
                    .checked_add(offset)
                    .ok_or(mkv::Error::Overflow("explicit track presentation offset"))?,
                duration: packet
                    .pts
                    .time_base()
                    .duration(u64::from(packet.duration))
                    .map_err(|error| mkv::Error::PacketSource(error.to_string()))?,
                keyframe: packet.is_keyframe,
                data: packet.data,
            })
        });
        streams.push(PacketStream::new(Box::new(packets)));
    }
    for subtitle in subtitles {
        let number = tracks.len() as u64 + 1;
        let (header, packets) = ass_packets(&subtitle.ass, number)?;
        tracks.push(mkv::Track {
            number,
            uid: 0,
            track_type: mkv::TrackType::Subtitle,
            codec: mkv::TrackCodec::Ass(header),
            settings: mkv::TrackSettings::Subtitle,
            name: Some(subtitle.metadata.title.clone()),
            language: Some(language(&subtitle.metadata.locale)),
            default: subtitle.metadata.default,
            forced: subtitle.metadata.forced,
            hearing_impaired: subtitle.metadata.is_caption,
            visual_impaired: false,
            original: false,
            commentary: false,
        });
        streams.push(PacketStream::new(Box::new(packets.into_iter().map(Ok))));
    }
    Ok((
        tracks,
        PacketStreams {
            streams,
            cancellation: cancellation.clone(),
            cancellation_reported: false,
        },
    ))
}

type FalliblePackets = Box<dyn Iterator<Item = Result<mkv::Packet, mkv::Error>>>;

struct PacketStream {
    packets: FalliblePackets,
    head: Option<Result<mkv::Packet, mkv::Error>>,
}

impl PacketStream {
    fn new(mut packets: FalliblePackets) -> Self {
        let head = packets.next();
        Self { packets, head }
    }
}

struct PacketStreams {
    streams: Vec<PacketStream>,
    cancellation: CancellationToken,
    cancellation_reported: bool,
}

impl Iterator for PacketStreams {
    type Item = Result<mkv::Packet, mkv::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cancellation.is_cancelled() && !self.cancellation_reported {
            self.cancellation_reported = true;
            return Some(Err(mkv::Error::Cancelled));
        }
        let index = self
            .streams
            .iter()
            .position(|stream| stream.head.as_ref().is_some_and(Result::is_err))
            .or_else(|| {
                self.streams
                    .iter()
                    .enumerate()
                    .filter_map(|(index, stream)| {
                        stream
                            .head
                            .as_ref()
                            .and_then(|packet| packet.as_ref().ok())
                            .map(|packet| (index, packet.decode_time_ms, packet.track_number))
                    })
                    .min_by_key(|(_, decode_time_ms, track_number)| {
                        (*decode_time_ms, *track_number)
                    })
                    .map(|(index, _, _)| index)
            })?;
        let result = self.streams[index].head.take();
        self.streams[index].head = self.streams[index].packets.next();
        result
    }
}

fn timestamp_millis(timestamp: media::Timestamp) -> Result<i64, media::Error> {
    let numerator = i128::from(timestamp.ticks())
        .checked_mul(1_000)
        .ok_or(media::Error::Overflow("timestamp milliseconds"))?;
    i64::try_from(numerator / i128::from(timestamp.time_base().ticks_per_second()))
        .map_err(|_| media::Error::Overflow("timestamp milliseconds"))
}

fn ass_packets(ass: &str, track_number: u64) -> Result<(String, Vec<mkv::Packet>), Error> {
    let normalized = ass.replace("\r\n", "\n").replace('\r', "\n");
    let mut header = Vec::new();
    let mut packets = Vec::new();
    let mut read_order = 0_u64;
    for line in normalized.lines() {
        let Some(dialogue) = line.strip_prefix("Dialogue:") else {
            header.push(line);
            continue;
        };
        let fields = dialogue.trim_start().splitn(10, ',').collect::<Vec<_>>();
        if fields.len() != 10 {
            return Err(Error::Subtitle("malformed ASS dialogue".into()));
        }
        let start = ass_time(fields[1])?;
        let end = ass_time(fields[2])?;
        if end < start {
            return Err(Error::Subtitle("ASS dialogue ends before it starts".into()));
        }
        let payload = format!(
            "{read_order},{},{},{},{},{},{},{},{}",
            fields[0], fields[3], fields[4], fields[5], fields[6], fields[7], fields[8], fields[9]
        );
        packets.push(mkv::Packet {
            track_number,
            decode_time_ms: i64::try_from(start.as_millis())
                .map_err(|_| Error::Subtitle("subtitle timestamp overflow".into()))?,
            presentation_time_ms: i64::try_from(start.as_millis())
                .map_err(|_| Error::Subtitle("subtitle timestamp overflow".into()))?,
            duration: end - start,
            keyframe: true,
            data: payload.into_bytes(),
        });
        read_order += 1;
    }
    packets.sort_by_key(|packet| packet.decode_time_ms);
    Ok((header.join("\r\n") + "\r\n", packets))
}

fn ass_time(value: &str) -> Result<Duration, Error> {
    let (hours, rest) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".into()))?;
    let (minutes, seconds) = rest
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".into()))?;
    let hours: u64 = hours
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    let minutes: u64 = minutes
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    let seconds: f64 = seconds
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return Err(Error::Subtitle("invalid ASS timestamp".into()));
    }
    Ok(Duration::from_millis(
        hours
            .saturating_mul(3_600_000)
            .saturating_add(minutes * 60_000)
            .saturating_add((seconds * 1000.0).round() as u64),
    ))
}

fn language(locale: &Locale) -> mkv::Language {
    let ietf = locale.to_string();
    let primary = ietf.split('-').next().unwrap_or("und");
    let legacy = match primary {
        "ar" => "ara",
        "ca" => "cat",
        "de" => "ger",
        "en" => "eng",
        "es" => "spa",
        "fr" => "fre",
        "hi" => "hin",
        "id" => "ind",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "ms" => "may",
        "pl" => "pol",
        "pt" => "por",
        "ru" => "rus",
        "ta" => "tam",
        "te" => "tel",
        "th" => "tha",
        "tr" => "tur",
        "vi" => "vie",
        "zh" => "chi",
        _ => "und",
    };
    mkv::Language {
        legacy: legacy.into(),
        ietf,
    }
}

fn verify(
    path: &Path,
    track_count: usize,
    attachment_count: usize,
    chapter_count: usize,
) -> Result<Vec<OutputTrack>, Error> {
    let parsed =
        matroska_reader::open(path).map_err(|error| Error::Verification(error.to_string()))?;
    if parsed.tracks.len() != track_count
        || parsed.attachments.len() != attachment_count
        || parsed
            .chapters
            .iter()
            .map(|edition| edition.chapters.len())
            .sum::<usize>()
            != chapter_count
        || parsed
            .info
            .duration
            .is_none_or(|duration| duration.is_zero())
    {
        return Err(Error::Verification(
            "Matroska structural verification failed".into(),
        ));
    }
    let markers = scan_markers(path)?;
    if !markers.cluster || !markers.block {
        return Err(Error::Verification(
            "Matroska contains no media blocks".into(),
        ));
    }
    Ok(parsed
        .tracks
        .into_iter()
        .map(|track| OutputTrack {
            codec: track.codec_id,
            language: track.language.map(|language| match language {
                matroska_reader::Language::ISO639(value)
                | matroska_reader::Language::IETF(value) => value,
            }),
            name: track.name,
            default: track.default,
            forced: track.forced,
        })
        .collect())
}

struct StructuralMarkers {
    cluster: bool,
    block: bool,
}

fn scan_markers(path: &Path) -> Result<StructuralMarkers, Error> {
    let mut reader = BufReader::new(File::open(path).map_err(|error| path_error(path, error))?);
    let mut carry = Vec::new();
    let mut markers = StructuralMarkers {
        cluster: false,
        block: false,
    };
    loop {
        let mut chunk = vec![0; 64 * 1024];
        let read = reader
            .read(&mut chunk)
            .map_err(|error| path_error(path, error))?;
        if read == 0 {
            break;
        }
        chunk.truncate(read);
        carry.extend(chunk);
        markers.cluster |= carry
            .windows(4)
            .any(|window| window == [0x1f, 0x43, 0xb6, 0x75]);
        markers.block |= carry.contains(&0xa3);
        if carry.len() > 7 {
            carry.drain(..carry.len() - 7);
        }
    }
    Ok(markers)
}

fn commit(temporary: &Path, destination: &Path, overwrite: OverwritePolicy) -> Result<(), Error> {
    if destination.exists() && overwrite != OverwritePolicy::Replace {
        return Err(Error::Filesystem(format!(
            "output already exists: {}",
            destination.display()
        )));
    }
    match std::fs::rename(temporary, destination) {
        Ok(()) => {}
        Err(error)
            if overwrite == OverwritePolicy::Replace
                && error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            std::fs::remove_file(destination).map_err(|error| path_error(destination, error))?;
            std::fs::rename(temporary, destination)
                .map_err(|error| path_error(destination, error))?;
        }
        Err(error) => return Err(path_error(destination, error)),
    }
    if let Some(parent) = destination.parent()
        && let Ok(directory) = File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn commit_output(
    temporary: &Path,
    destination: &Path,
    overwrite: OverwritePolicy,
    archive: Option<&dyn Archive>,
    archive_key: &ArchiveKey,
    tracks: &[OutputTrack],
) -> Result<(), Error> {
    commit(temporary, destination, overwrite)?;
    if let Some(archive) = archive {
        archive.record(&ArchiveEntry {
            key: archive_key.clone(),
            output: destination.to_path_buf(),
            tracks: tracks.to_vec(),
        })?;
    }
    Ok(())
}

fn cleanup_staging(path: &Path, retention: RetentionPolicy, succeeded: bool) {
    let retain = matches!(
        (retention, succeeded),
        (RetentionPolicy::KeepAlways, _) | (RetentionPolicy::KeepOnFailure, false)
    );
    if !retain {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn validate_media(media: &ResolvedMedia) -> Result<(), Error> {
    if media.content_id.is_empty()
        || media
            .availability_status
            .eq_ignore_ascii_case("unavailable")
    {
        return Err(Error::Unavailable(media.content_id.clone()));
    }
    Ok(())
}

fn safe_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "media".into()
    } else {
        value
    }
}
fn path_error(path: &Path, error: std::io::Error) -> Error {
    Error::Filesystem(format!("{}: {error}", path.display()))
}

struct StateEmitter {
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn drm_systems_select_their_production_license_endpoints() {
        assert!(
            DrmSystem::PlayReady
                .default_license_endpoint()
                .ends_with("/playReady")
        );
        assert!(
            DrmSystem::Widevine
                .default_license_endpoint()
                .ends_with("/widevine")
        );
    }

    struct RecordingArchive {
        records: Mutex<Vec<ArchiveEntry>>,
    }

    impl Archive for RecordingArchive {
        fn find(&self, _key: &ArchiveKey) -> Result<Option<ArchiveEntry>, Error> {
            Ok(None)
        }

        fn record(&self, entry: &ArchiveEntry) -> Result<(), Error> {
            assert!(entry.output.is_file());
            self.records.lock().unwrap().push(entry.clone());
            Ok(())
        }
    }

    fn test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "crunchydl-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn ass_dialogue_becomes_timed_matroska_packet() {
        let (header, packets) = ass_packets("[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello, world", 3).unwrap();
        assert!(header.contains("Format:"));
        assert_eq!(packets[0].track_number, 3);
        assert_eq!(packets[0].presentation_time_ms, 1_000);
        assert_eq!(packets[0].duration, Duration::from_millis(1500));
        assert!(
            String::from_utf8(packets[0].data.clone())
                .unwrap()
                .ends_with("Hello, world")
        );
    }

    #[test]
    fn ass_packets_are_stably_ordered_by_timestamp() {
        let source = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,Later in file\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Earlier in time";
        let (_, packets) = ass_packets(source, 3).unwrap();
        assert_eq!(packets[0].decode_time_ms, 1_000);
        assert_eq!(packets[1].decode_time_ms, 5_000);
        assert!(String::from_utf8_lossy(&packets[0].data).starts_with("1,"));
        assert!(String::from_utf8_lossy(&packets[1].data).starts_with("0,"));
    }

    #[test]
    fn structural_marker_scan_does_not_treat_subtitle_text_as_mp4_signaling() {
        let root =
            std::env::temp_dir().join(format!("crunchydl-marker-scan-{}", std::process::id()));
        std::fs::write(&root, b"\x1f\x43\xb6\x75\xa3se encaixam").unwrap();
        let markers = scan_markers(&root).unwrap();
        assert!(markers.cluster);
        assert!(markers.block);
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn cancellation_is_checked_on_both_sides_of_every_transition() {
        let token = CancellationToken::new();
        token.cancel();
        let states = StateEmitter::new(Arc::new(NoopSink), token);
        for state in [
            JobState::Created,
            JobState::ResolvingMedia,
            JobState::OpeningPlaybackSessions,
            JobState::PlanningTracks,
            JobState::AcquiringLicenses,
            JobState::Downloading,
            JobState::Decrypting,
            JobState::ProcessingSubtitles,
            JobState::Muxing,
            JobState::Verifying,
            JobState::Committing,
        ] {
            assert!(matches!(states.enter(state), Err(Error::Cancelled)));
        }
    }

    #[test]
    fn forced_failure_at_every_state_obeys_staging_retention() {
        for state in [
            JobState::Created,
            JobState::ResolvingMedia,
            JobState::OpeningPlaybackSessions,
            JobState::PlanningTracks,
            JobState::AcquiringLicenses,
            JobState::Downloading,
            JobState::Decrypting,
            JobState::ProcessingSubtitles,
            JobState::Muxing,
            JobState::Verifying,
            JobState::Committing,
        ] {
            for (retention, expected_to_exist) in [
                (RetentionPolicy::Delete, false),
                (RetentionPolicy::KeepOnFailure, true),
                (RetentionPolicy::KeepAlways, true),
            ] {
                let staging = test_directory("failure-retention");
                std::fs::create_dir_all(&staging).unwrap();
                std::fs::write(staging.join("partial"), b"partial").unwrap();
                let cancellation = CancellationToken::new();
                let cancel_from_sink = cancellation.clone();
                let states = StateEmitter::new(
                    Arc::new(move |event| {
                        if matches!(event, DownloadEvent::StateChanged(observed) if observed == state)
                        {
                            cancel_from_sink.cancel();
                        }
                    }),
                    cancellation,
                );
                assert!(matches!(states.enter(state), Err(Error::Cancelled)));
                cleanup_staging(&staging, retention, false);
                assert_eq!(staging.exists(), expected_to_exist);
                let _ = std::fs::remove_dir_all(staging);
            }
        }
    }

    #[test]
    fn archive_is_recorded_only_after_output_commit() {
        let root = test_directory("commit-archive");
        std::fs::create_dir_all(&root).unwrap();
        let temporary = root.join("output.part");
        let destination = root.join("output.mkv");
        std::fs::write(&temporary, b"verified output").unwrap();
        let archive = RecordingArchive {
            records: Mutex::new(Vec::new()),
        };
        let key = ArchiveKey {
            media_id: "M1".into(),
            selection_fingerprint: "selection".into(),
        };
        commit_output(
            &temporary,
            &destination,
            OverwritePolicy::Fail,
            Some(&archive),
            &key,
            &[],
        )
        .unwrap();
        assert!(destination.is_file());
        assert_eq!(archive.records.lock().unwrap().len(), 1);

        let failed_temporary = root.join("missing.part");
        let failed_destination = root.join("missing-parent/output.mkv");
        assert!(
            commit_output(
                &failed_temporary,
                &failed_destination,
                OverwritePolicy::Fail,
                Some(&archive),
                &key,
                &[],
            )
            .is_err()
        );
        assert_eq!(archive.records.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
