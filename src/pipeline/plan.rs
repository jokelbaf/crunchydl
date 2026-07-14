use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{
    AudioMediaStream, MediaStreamDRM, StreamData, StreamDrm, VideoMediaStream,
};

use crate::api::{ApiSubtitle, CrunchyrollApi};
use crate::chapters::from_skip_events;
use crate::redaction::{fingerprint, url_identity};
use crate::selection::{
    AudioQualityCandidate, ChapterSelection, SubtitleLocales, SubtitleTrackInfo,
    VideoQualityCandidate, VideoSelection, select_audio, select_audio_quality, select_hardsub,
    select_subtitles, select_video_quality,
};
use crate::session::SessionGuard;
use crate::{CancellationToken, DownloadWarning, Error, ResolvedMedia};

mod sources;
mod subtitles;
mod types;

use sources::{choose_audio, choose_video};
use subtitles::plan_subtitles;
pub use types::{DownloadPlan, PlannedSubtitle, PlannedTrack, PlannedTrackKind, PlanningOptions};
pub(crate) use types::{PlannedSource, PreparedPlan, PreparedSubtitle, SourceSegment};

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
