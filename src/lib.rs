//! Pure-Rust Crunchyroll downloader library.
//!
//! `crunchydl` discovers Crunchyroll media, selects versions and tracks,
//! downloads DASH media and subtitles, obtains content keys from a caller-supplied
//! DRM device, decrypts fragmented MP4 media, and muxes a finished Matroska file -
//! all without invoking any external executable.
//!
//! # Status
//!
//! The core Phase 0–9 pipeline is implemented. It exposes:
//!
//! - [`Downloader`], constructed from an authenticated
//!   [`crunchyroll_rs::Crunchyroll`] client, with series and season helpers.
//! - [`MediaRequest`] and [`MediaRequest::resolve`] to normalize a fetched
//!   `crunchyroll-rs` media object into a stable [`ResolvedMedia`].
//! - Track [selection](crate#selection) policies and the resolvers that apply
//!   them.
//!
//! Playback planning, resumable transfer, optional PlayReady/Widevine license
//! acquisition, CENC-family decryption, fragmented-MP4 probing, subtitle/font
//! processing, native Matroska muxing, atomic commit, and optional archives.
//!
//! # Selection
//!
//! Selection never silently degrades. When a configured [`FallbackPolicy`] causes
//! a deviation, the resolver records a [`DownloadWarning`]; otherwise it returns a
//! [`SelectionError`] listing what was available.

mod api;
mod archive;
mod batch;
mod cancellation;
mod capability;
mod catalog;
mod chapters;
mod error;
mod event;
mod filename;
mod font;
mod language;
mod model;
mod mp4_output;
mod plan;
mod raw_api;
mod redaction;
mod selection;
mod session;
mod staging;
mod subtitle;
mod transfer;

pub use archive::{Archive, ArchiveEntry, ArchiveKey, JsonArchive};
pub use batch::{
    BatchError, BatchOptions, CollectionTarget, canonicalize_episode_batch, select_batch_targets,
};
pub use cancellation::CancellationToken;
pub use capability::{
    AudioCapability, DrmCapability, MediaCapabilities, SubtitleCapability, VersionCapabilities,
    VideoCapability,
};
pub use catalog::{CatalogImage, CatalogImageKind, CatalogItem, CatalogKind, CatalogRating};
pub use chapters::Chapter;
pub use download::{
    DownloadRequest, DownloadResult, DrmSystem, OutputOptions, OutputTrack, RetentionPolicy,
    SynchronizationOptions,
};
#[cfg(feature = "drm-playready")]
pub use drm::PlayReadyProvider;
#[cfg(feature = "drm-widevine")]
pub use drm::WidevineProvider;
pub use drm::{
    CencDecrypter, ContentKey, ContentType, DrmProvider, DrmRequest, EncryptionInfo,
    EncryptionScheme, KeyId, KeySet, LicenseRequest, LicenseResponse, LicenseTransport,
    acquire_track_keys, inspect_encryption,
};
pub use error::{ApiError, ApiErrorKind, Error};
pub use event::{DownloadEvent, DownloadWarning, EventSink, JobState, NoopSink, TransferTrack};
pub use filename::{FilenameTemplate, OutputLayoutTemplate};
pub use font::{
    DirectoryFontResolver, FontPolicy, FontResolver, ResolvedFont, resolve_referenced_fonts,
};
pub use language::locale_display_name;
pub use model::{MediaKind, MediaRequest, MediaTarget, ResolvedMedia, ResolvedVersion};
pub use plan::{DownloadPlan, PlannedSubtitle, PlannedTrack, PlannedTrackKind, PlanningOptions};
pub use raw_api::CrunchyrollLicenseTransport;
pub use selection::{
    AudioPlan, AudioQualityCandidate, AudioSelection, CdnSelection, ChapterSelection, Container,
    FallbackPolicy, HardSubSelection, OverwritePolicy, QualityChoice, QualitySelection,
    SelectionError, SubtitleLocales, SubtitlePlan, SubtitleSelection, SubtitleTrackInfo,
    VideoQualityCandidate, VideoSelection, select_audio, select_audio_quality, select_cdn,
    select_hardsub, select_subtitles, select_video_quality,
};
pub use subtitle::{
    AssNormalization, SubtitleFormat, SubtitleMetadata, SubtitleProcessingOptions, SubtitleTrack,
    process_subtitle,
};
pub use transfer::{
    NoRefresh, RepresentationRefresher, RepresentationTransferPlan, SegmentRequest, TransferEngine,
    TransferOptions, TransferResult,
};

/// Re-export of the underlying `crunchyroll-rs` crate so callers can construct
/// requests and browse the catalog without depending on it directly.
pub use crunchyroll_rs;

use crunchyroll_rs::Crunchyroll;
use crunchyroll_rs::media::StreamPlatform;

use crate::api::ProductionApi;
use std::sync::Arc;

/// Downloads Crunchyroll media using an authenticated `crunchyroll-rs` client.
///
/// Construct one with [`Downloader::builder`]. A caller that already holds a
/// `crunchyroll-rs` media object can normalize it directly with
/// [`MediaRequest::resolve`] instead of fetching by id here.
pub struct Downloader {
    api: ProductionApi,
    crunchyroll: Crunchyroll,
    platform: StreamPlatform,
    runtime: download::RuntimeConfiguration,
}

impl Downloader {
    /// Start building a downloader from an already authenticated client.
    #[must_use]
    pub fn builder(crunchyroll: Crunchyroll) -> DownloaderBuilder {
        DownloaderBuilder {
            crunchyroll,
            platform: StreamPlatform::default(),
            runtime: download::RuntimeConfiguration::default(),
        }
    }

    /// The stream platform playback sessions will be opened with.
    #[must_use]
    pub fn stream_platform(&self) -> &StreamPlatform {
        &self.platform
    }

    /// Fetch an episode by id and resolve it into normalized [`ResolvedMedia`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the request fails.
    pub async fn resolve_episode(&self, id: &str) -> Result<ResolvedMedia, Error> {
        api::resolve_episode_id(&self.api, id).await
    }

    /// Fetch a movie by id and resolve it into normalized [`ResolvedMedia`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the request fails.
    pub async fn resolve_movie(&self, id: &str) -> Result<ResolvedMedia, Error> {
        api::resolve_movie_id(&self.api, id).await
    }

    /// Fetch a music video by id and resolve it into normalized
    /// [`ResolvedMedia`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the request fails.
    pub async fn resolve_music_video(&self, id: &str) -> Result<ResolvedMedia, Error> {
        api::resolve_music_video_id(&self.api, id).await
    }

    /// Fetch and normalize a stable media target.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the target cannot be fetched.
    pub async fn resolve_target(&self, target: &MediaTarget) -> Result<ResolvedMedia, Error> {
        api::resolve_target(&self.api, target).await
    }

    /// Fetch the concrete service object for a stable media target.
    ///
    /// Frontends can persist [`MediaTarget`] values and call this immediately
    /// before constructing a [`DownloadRequest`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the target cannot be fetched.
    pub async fn media_request(&self, target: &MediaTarget) -> Result<MediaRequest, Error> {
        api::media_request_from_target(&self.api, target).await
    }

    /// Fetch a season by id and resolve every episode into normalized
    /// [`ResolvedMedia`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if the request fails.
    pub async fn season_episodes(&self, season_id: &str) -> Result<Vec<ResolvedMedia>, Error> {
        api::resolve_season_id(&self.api, season_id).await
    }

    /// Fetch a series by id and resolve every episode of every season into
    /// normalized [`ResolvedMedia`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Api`] if any request fails.
    pub async fn series_episodes(&self, series_id: &str) -> Result<Vec<ResolvedMedia>, Error> {
        api::resolve_series_id(&self.api, series_id).await
    }

    /// Expand a season, series, or movie listing into durable download targets.
    ///
    /// Localized duplicate episodes are canonicalized before filters are
    /// applied, so selecting every audio version does not enqueue the same
    /// logical episode more than once.
    ///
    /// # Errors
    ///
    /// Returns a typed API or batch-canonicalization error.
    pub async fn expand_collection(
        &self,
        target: &CollectionTarget,
        options: &BatchOptions,
    ) -> Result<Vec<MediaTarget>, Error> {
        api::expand_collection(&self.api, target, options).await
    }

    /// Open the required playback sessions and build a redacted immutable plan.
    ///
    /// Every opened session is explicitly invalidated before this method
    /// returns, including on selection, manifest, and cancellation errors.
    ///
    /// # Errors
    ///
    /// Returns a typed API, selection, manifest, playback, or cancellation
    /// error. Session invalidation errors are returned when planning itself
    /// succeeded.
    pub async fn plan(
        &self,
        media: &ResolvedMedia,
        options: &PlanningOptions,
        cancellation: &CancellationToken,
    ) -> Result<DownloadPlan, Error> {
        Ok(
            plan::prepare(&self.api, &self.platform, media, options, cancellation)
                .await?
                .public,
        )
    }

    /// Inspect every audio version and return all redacted playback choices.
    ///
    /// Every playback session opened for inspection is explicitly invalidated
    /// before this method returns.
    ///
    /// # Errors
    ///
    /// Returns a typed API, playback, manifest, unavailable, or cancellation
    /// error.
    pub async fn inspect(
        &self,
        media: &ResolvedMedia,
        cancellation: &CancellationToken,
    ) -> Result<MediaCapabilities, Error> {
        capability::inspect(&self.api, &self.platform, media, cancellation).await
    }

    /// Execute resolution, playback, licensing, transfer, decryption, subtitle
    /// processing, Matroska muxing, verification, atomic commit, and archive
    /// persistence for one request.
    ///
    /// # Errors
    ///
    /// Returns the first typed pipeline error. Every opened playback session is
    /// explicitly invalidated before this method returns.
    pub async fn download(&self, request: DownloadRequest) -> Result<DownloadResult, Error> {
        download::run(
            &self.api,
            &self.crunchyroll,
            &self.platform,
            &self.runtime,
            request,
        )
        .await
    }
}

/// Builder for [`Downloader`].
pub struct DownloaderBuilder {
    crunchyroll: Crunchyroll,
    platform: StreamPlatform,
    runtime: download::RuntimeConfiguration,
}

impl DownloaderBuilder {
    /// Set the stream platform playback sessions will be opened with.
    ///
    /// Defaults to [`StreamPlatform::TvAndroid`].
    #[must_use]
    pub fn stream_platform(mut self, platform: StreamPlatform) -> Self {
        self.platform = platform;
        self
    }

    /// Configure the DRM provider and explicitly selected system.
    ///
    /// The production Crunchyroll license endpoint is selected automatically.
    #[must_use]
    pub fn drm(mut self, provider: Arc<dyn DrmProvider>, system: DrmSystem) -> Self {
        let endpoint = system.default_license_endpoint().to_string();
        self.runtime.drm = Some(download::DrmConfiguration {
            provider,
            system,
            endpoint,
        });
        self
    }

    /// Configure DRM with an explicit license endpoint override.
    ///
    /// Prefer [`DownloaderBuilder::drm`] for normal Crunchyroll downloads.
    /// This override exists for testing, proxies, and service endpoint changes.
    #[must_use]
    pub fn drm_with_license_endpoint(
        mut self,
        provider: Arc<dyn DrmProvider>,
        system: DrmSystem,
        license_endpoint: impl Into<String>,
    ) -> Self {
        self.runtime.drm = Some(download::DrmConfiguration {
            provider,
            system,
            endpoint: license_endpoint.into(),
        });
        self
    }

    /// Enable opt-in archive lookup and persistence.
    #[must_use]
    pub fn archive(mut self, archive: Arc<dyn Archive>) -> Self {
        self.runtime.archive = Some(archive);
        self
    }

    /// Configure local font resolution and missing-font behavior.
    #[must_use]
    pub fn fonts(mut self, resolver: Arc<dyn FontResolver>, policy: FontPolicy) -> Self {
        self.runtime.font_resolver = Some(resolver);
        self.runtime.font_policy = policy;
        self
    }

    /// Deliver structured state, warning, retry, and segment events to `sink`.
    #[must_use]
    pub fn event_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.runtime.events = Arc::new(sink);
        self
    }

    /// Build the [`Downloader`].
    #[must_use]
    pub fn build(self) -> Downloader {
        let api = ProductionApi::new(self.crunchyroll.clone());
        Downloader {
            api,
            crunchyroll: self.crunchyroll,
            platform: self.platform,
            runtime: self.runtime,
        }
    }
}
mod download;
