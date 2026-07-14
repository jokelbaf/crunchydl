use std::fmt;
use std::time::Duration;

use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{
    AudioMediaStream, MediaStreamDRM, StreamData, StreamDrm, VideoMediaStream,
};

use crate::api::{ApiSubtitle, CrunchyrollApi};
use crate::chapters::{Chapter, from_skip_events};
use crate::redaction::{fingerprint, url_identity};
use crate::selection::{
    AudioQualityCandidate, AudioSelection, ChapterSelection, FallbackPolicy, HardSubSelection,
    QualitySelection, SubtitleLocales, SubtitleSelection, SubtitleTrackInfo, VideoQualityCandidate,
    VideoSelection, select_audio, select_audio_quality, select_hardsub, select_subtitles,
    select_video_quality,
};
use crate::session::SessionGuard;
use crate::{CancellationToken, DownloadWarning, Error, ResolvedMedia};

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
    // Consumed by end-to-end orchestration in Phase 9; keeping it private
    // prevents signed segment URLs from entering public diagnostics.
    #[allow(dead_code)]
    pub(crate) sources: Vec<PlannedSource>,
    #[allow(dead_code)]
    pub(crate) subtitles: Vec<PreparedSubtitle>,
}

pub(crate) async fn prepare<A: CrunchyrollApi>(
    api: &A,
    platform: &crunchyroll_rs::media::StreamPlatform,
    media: &ResolvedMedia,
    options: &PlanningOptions,
    cancellation: &CancellationToken,
) -> Result<PreparedPlan, Error> {
    let mut guard = SessionGuard::new(api);
    let result = prepare_with_guard(api, platform, media, options, cancellation, &mut guard).await;
    let finalization = guard.finalize().await;
    match (result, finalization) {
        (Ok(plan), Ok(())) => Ok(plan),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), _) => Err(error),
    }
}

pub(crate) async fn prepare_with_guard<A: CrunchyrollApi>(
    api: &A,
    platform: &crunchyroll_rs::media::StreamPlatform,
    media: &ResolvedMedia,
    options: &PlanningOptions,
    cancellation: &CancellationToken,
    guard: &mut SessionGuard<'_, A>,
) -> Result<PreparedPlan, Error> {
    let audio_plan = select_audio(media, &options.audio, options.fallback)?;
    let mut warnings = audio_plan.warnings;
    let mut sources = Vec::new();
    let mut all_subtitles = Vec::new();
    let mut selected_audio_locales = Vec::new();

    for version in audio_plan.versions {
        let session_index = guard
            .open(&version.content_id, platform, cancellation)
            .await?;
        let metadata = guard.metadata(session_index);
        let hardsub = select_hardsub(&metadata.hardsubs, &options.hardsub)?;
        let data = guard
            .stream_data(session_index, hardsub, cancellation)
            .await?;
        selected_audio_locales.push(metadata.audio_locale.clone());
        all_subtitles.extend(metadata.subtitles);

        let video = choose_video(
            &version.content_id,
            &metadata.audio_locale,
            &metadata.drm,
            &data,
            options,
        )
        .await?;
        if let Some(warning) = video.1 {
            warnings.push(warning);
        }
        let keep_video = options.video == VideoSelection::PerVersion
            || !sources.iter().any(|source: &PlannedSource| {
                source.diagnostic.kind == PlannedTrackKind::Video
                    && source.diagnostic.representation_fingerprint
                        == video.0.diagnostic.representation_fingerprint
            });
        if keep_video {
            sources.push(video.0);
        }

        let audio = choose_audio(
            &version.content_id,
            &metadata.audio_locale,
            &metadata.drm,
            &data,
            options,
        )
        .await?;
        if let Some(warning) = audio.1 {
            warnings.push(warning);
        }
        sources.push(audio.0);
    }

    let (subtitles, subtitle_sources) = plan_subtitles(
        all_subtitles,
        &selected_audio_locales,
        options,
        &mut warnings,
    );
    let chapters = match options.chapters {
        ChapterSelection::Disabled => Vec::new(),
        ChapterSelection::SkipEvents => from_skip_events(
            api.skip_events(&media.content_id, media.kind).await?,
            media.duration,
        ),
    };
    let tracks = sources
        .iter()
        .map(|source| source.diagnostic.clone())
        .collect::<Vec<_>>();
    let plan_fingerprint = plan_fingerprint(&media.content_id, &tracks, &subtitles);
    Ok(PreparedPlan {
        public: DownloadPlan {
            media_id: media.content_id.clone(),
            tracks,
            subtitles,
            chapters,
            warnings,
            fingerprint: plan_fingerprint,
        },
        sources,
        subtitles: subtitle_sources,
    })
}

async fn choose_video(
    version_id: &str,
    locale: &Locale,
    playback_drm: &StreamDrm,
    data: &StreamData,
    options: &PlanningOptions,
) -> Result<(PlannedSource, Option<DownloadWarning>), Error> {
    let candidates = data
        .video
        .iter()
        .map(|video| VideoQualityCandidate {
            width: u32::try_from(video.resolution.width).unwrap_or(u32::MAX),
            height: u32::try_from(video.resolution.height).unwrap_or(u32::MAX),
            bandwidth: video.bandwidth,
        })
        .collect::<Vec<_>>();
    let choice = select_video_quality(&candidates, &options.video_quality, options.fallback)?;
    Ok((
        video_source(version_id, locale, playback_drm, &data.video[choice.index]).await?,
        choice.warning,
    ))
}

async fn choose_audio(
    version_id: &str,
    locale: &Locale,
    playback_drm: &StreamDrm,
    data: &StreamData,
    options: &PlanningOptions,
) -> Result<(PlannedSource, Option<DownloadWarning>), Error> {
    let candidates = data
        .audio
        .iter()
        .map(|audio| AudioQualityCandidate {
            bandwidth: audio.bandwidth,
            sampling_rate: audio.sampling_rate,
        })
        .collect::<Vec<_>>();
    let choice = select_audio_quality(&candidates, &options.audio_quality, options.fallback)?;
    Ok((
        audio_source(version_id, locale, playback_drm, &data.audio[choice.index]).await?,
        choice.warning,
    ))
}

async fn video_source(
    version_id: &str,
    locale: &Locale,
    playback_drm: &StreamDrm,
    stream: &VideoMediaStream,
) -> Result<PlannedSource, Error> {
    let dimensions = Some((
        u32::try_from(stream.resolution.width).unwrap_or(u32::MAX),
        u32::try_from(stream.resolution.height).unwrap_or(u32::MAX),
    ));
    source_from_segments(
        PlannedTrackKind::Video,
        version_id,
        locale,
        stream.codecs.clone(),
        stream.bandwidth,
        dimensions,
        None,
        stream.drm.clone(),
        playback_drm.clone(),
        stream.segments().await?,
    )
}

async fn audio_source(
    version_id: &str,
    locale: &Locale,
    playback_drm: &StreamDrm,
    stream: &AudioMediaStream,
) -> Result<PlannedSource, Error> {
    source_from_segments(
        PlannedTrackKind::Audio,
        version_id,
        locale,
        stream.codecs.clone(),
        stream.bandwidth,
        None,
        Some(stream.sampling_rate),
        stream.drm.clone(),
        playback_drm.clone(),
        stream.segments().await?,
    )
}

#[allow(clippy::too_many_arguments)]
fn source_from_segments(
    kind: PlannedTrackKind,
    version_id: &str,
    locale: &Locale,
    codec: String,
    bandwidth: u64,
    dimensions: Option<(u32, u32)>,
    sampling_rate: Option<u32>,
    drm: Option<MediaStreamDRM>,
    playback_drm: StreamDrm,
    segments: Vec<crunchyroll_rs::media::StreamSegment>,
) -> Result<PlannedSource, Error> {
    let mut segments = segments.into_iter().map(|segment| SourceSegment {
        url: segment.url,
        range: segment.range,
        duration: segment.length,
    });
    let init = segments.next().ok_or_else(|| {
        Error::Manifest("representation has no initialization segment".to_string())
    })?;
    let media = segments.collect::<Vec<_>>();
    let mut identity = vec![
        version_id.to_string(),
        format!("{kind:?}"),
        codec.clone(),
        bandwidth.to_string(),
        format!("{dimensions:?}"),
        format!("{sampling_rate:?}"),
        url_identity(&init.url),
        format!("{:?}", init.range),
    ];
    identity.extend(media.iter().flat_map(|segment| {
        [
            url_identity(&segment.url),
            format!("{:?}", segment.range),
            segment.duration.as_millis().to_string(),
        ]
    }));
    let representation_fingerprint = fingerprint(identity.iter());
    Ok(PlannedSource {
        diagnostic: PlannedTrack {
            kind,
            version_id: version_id.to_string(),
            locale: locale.clone(),
            codec,
            bandwidth,
            dimensions,
            sampling_rate,
            segment_count: media.len(),
            representation_fingerprint,
            encrypted: drm.is_some(),
        },
        init,
        media,
        drm,
        playback_drm,
    })
}

fn plan_subtitles(
    subtitles: Vec<ApiSubtitle>,
    audio_locales: &[Locale],
    options: &PlanningOptions,
    warnings: &mut Vec<DownloadWarning>,
) -> (Vec<PlannedSubtitle>, Vec<PreparedSubtitle>) {
    let mut unique = Vec::<ApiSubtitle>::new();
    for subtitle in subtitles {
        let identity = url_identity(&subtitle.url);
        if !unique.iter().any(|current| {
            current.locale == subtitle.locale
                && current.format == subtitle.format
                && current.is_caption == subtitle.is_caption
                && url_identity(&current.url) == identity
        }) {
            unique.push(subtitle);
        }
    }
    let infos = unique
        .iter()
        .map(|subtitle| SubtitleTrackInfo {
            locale: subtitle.locale.clone(),
            is_caption: subtitle.is_caption,
            format: subtitle.format.clone(),
        })
        .collect::<Vec<_>>();
    let selected = select_subtitles(&infos, &options.subtitles);
    warnings.extend(selected.warnings);
    let mut prepared = selected
        .tracks
        .into_iter()
        .filter_map(|index| unique.get(index))
        .filter(|subtitle| {
            options.subtitles.include_signs
                || subtitle.is_caption
                || !audio_locales.contains(&subtitle.locale)
        })
        .map(|subtitle| {
            let is_signs = !subtitle.is_caption && audio_locales.contains(&subtitle.locale);
            let language = crate::locale_display_name(&subtitle.locale);
            let title = if subtitle.is_caption {
                format!("{language} (CC)")
            } else if is_signs {
                format!("{language} (Signs)")
            } else {
                language
            };
            let diagnostic = PlannedSubtitle {
                locale: subtitle.locale.clone(),
                format: subtitle.format.clone(),
                is_caption: subtitle.is_caption,
                is_signs,
                resource_identity: url_identity(&subtitle.url),
                title,
                default: false,
                forced: is_signs,
            };
            PreparedSubtitle {
                diagnostic,
                url: subtitle.url.clone(),
            }
        })
        .collect::<Vec<_>>();
    // When the user asks for a specific subtitle language, make the first
    // matching non-forced track player-visible by default. Tracks selected via
    // "all" remain opt-in so multilingual downloads do not pick an arbitrary
    // language.
    if matches!(options.subtitles.locales, SubtitleLocales::Explicit(_))
        && let Some(subtitle) = prepared
            .iter_mut()
            .find(|subtitle| !subtitle.diagnostic.forced)
    {
        subtitle.diagnostic.default = true;
    }
    let diagnostics = prepared
        .iter()
        .map(|subtitle| subtitle.diagnostic.clone())
        .collect();
    (diagnostics, prepared)
}

fn plan_fingerprint(
    media_id: &str,
    tracks: &[PlannedTrack],
    subtitles: &[PlannedSubtitle],
) -> String {
    let mut parts = vec![media_id.to_string()];
    parts.extend(tracks.iter().map(|track| {
        format!(
            "{:?}|{}|{}|{}|{}|{:?}|{:?}|{}|{}",
            track.kind,
            track.version_id,
            track.locale,
            track.codec,
            track.bandwidth,
            track.dimensions,
            track.sampling_rate,
            track.segment_count,
            track.representation_fingerprint
        )
    }));
    parts.extend(subtitles.iter().map(|subtitle| {
        format!(
            "{}|{}|{}|{}|{}",
            subtitle.locale,
            subtitle.format,
            subtitle.is_caption,
            subtitle.is_signs,
            subtitle.resource_identity
        )
    }));
    fingerprint(parts.iter())
}

#[cfg(test)]
mod subtitle_tests {
    use super::*;

    #[test]
    fn subtitle_resources_deduplicate_without_query_secrets_and_keep_signs_metadata() {
        let subtitles = vec![
            ApiSubtitle {
                locale: Locale::ja_JP,
                url: "https://cdn.test/signs.ass?token=one".to_string(),
                format: "ass".to_string(),
                is_caption: false,
            },
            ApiSubtitle {
                locale: Locale::ja_JP,
                url: "https://cdn.test/signs.ass?token=two".to_string(),
                format: "ass".to_string(),
                is_caption: false,
            },
        ];
        let mut warnings = Vec::new();
        let (diagnostics, prepared) = plan_subtitles(
            subtitles,
            &[Locale::ja_JP],
            &PlanningOptions::default(),
            &mut warnings,
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(prepared.len(), 1);
        assert!(diagnostics[0].is_signs && diagnostics[0].forced);
        assert_eq!(diagnostics[0].title, "Japanese (Signs)");
        assert_eq!(
            diagnostics[0].resource_identity,
            "https://cdn.test/signs.ass"
        );
        assert!(!format!("{:?}", diagnostics).contains("token="));
    }

    #[test]
    fn explicitly_selected_subtitle_is_default() {
        let subtitles = vec![
            ApiSubtitle {
                locale: Locale::en_US,
                url: "https://cdn.test/en.ass".to_string(),
                format: "ass".to_string(),
                is_caption: true,
            },
            ApiSubtitle {
                locale: Locale::de_DE,
                url: "https://cdn.test/de.ass".to_string(),
                format: "ass".to_string(),
                is_caption: false,
            },
        ];
        let options = PlanningOptions {
            subtitles: SubtitleSelection::default()
                .with_locales(SubtitleLocales::Explicit(vec![Locale::en_US])),
            ..PlanningOptions::default()
        };
        let mut warnings = Vec::new();
        let (diagnostics, _) = plan_subtitles(subtitles, &[Locale::ja_JP], &options, &mut warnings);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].default);
        assert_eq!(diagnostics[0].title, "English (CC)");
    }
}
