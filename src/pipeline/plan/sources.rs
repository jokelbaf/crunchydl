//! Selection and redacted source construction from playback manifests.

use super::types::SourceSegment;
use super::*;

pub(super) async fn choose_video(
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

pub(super) async fn choose_audio(
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
