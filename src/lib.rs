//! Pure-Rust Crunchyroll downloader library.
//!
//! `crunchydl` discovers Crunchyroll media, selects versions and tracks,
//! downloads DASH media and subtitles, obtains content keys from a caller-supplied
//! DRM device, decrypts fragmented MP4 media, and muxes a finished Matroska file -
//! all without invoking any external executable.

mod cancellation;
mod domain;
mod downloader;
mod error;
mod event;
mod pipeline;
mod service;

pub(crate) use domain::subtitle::fonts as font;
pub(crate) use domain::{batch, catalog, chapters, filename, language, model, selection, subtitle};
pub(crate) use pipeline::job::{archive, session, staging};
pub(crate) use pipeline::mp4 as mp4_output;
pub(crate) use pipeline::{download, plan, redaction, transfer};
pub(crate) use service::{api, capability, raw_api};

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

pub use crunchyroll_rs;
pub use downloader::{Downloader, DownloaderBuilder};
