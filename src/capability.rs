//! Redacted playback capability inspection for interactive frontends.

use crunchyroll_rs::Locale;
use crunchyroll_rs::media::{MediaStreamDRM, MediaStreamDRMType, StreamData, StreamPlatform};

use crate::api::{ApiSubtitle, CrunchyrollApi};
use crate::session::SessionGuard;
use crate::{CancellationToken, Error, ResolvedMedia};

/// DRM systems advertised by a selected manifest.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum DrmCapability {
    /// Microsoft PlayReady metadata is available.
    PlayReady,
    /// Google Widevine metadata is available.
    Widevine,
}

/// One available video representation without signed source data.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VideoCapability {
    /// Codec descriptor reported by the manifest.
    pub codec: String,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Representation bandwidth in bits per second.
    pub bandwidth: u64,
    /// Frames per second.
    pub fps: f64,
    /// Whether the representation is encrypted.
    pub encrypted: bool,
    /// DRM systems advertised for this representation.
    pub drm: Vec<DrmCapability>,
}

/// One available audio representation without signed source data.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AudioCapability {
    /// Codec descriptor reported by the manifest.
    pub codec: String,
    /// Representation bandwidth in bits per second.
    pub bandwidth: u64,
    /// Sampling rate in hertz.
    pub sampling_rate: u32,
    /// Whether the representation is encrypted.
    pub encrypted: bool,
    /// DRM systems advertised for this representation.
    pub drm: Vec<DrmCapability>,
}

/// One subtitle or closed-caption resource available to a version.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SubtitleCapability {
    /// Subtitle locale.
    pub locale: Locale,
    /// Source format such as `ass` or `vtt`.
    pub format: String,
    /// Whether this is a closed-caption resource.
    pub is_caption: bool,
}

/// Playback choices available for one audio-language version.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VersionCapabilities {
    /// Version content id.
    pub version_id: String,
    /// Audio locale confirmed by the playback session.
    pub audio_locale: Locale,
    /// Whether metadata marks this as the original-language version.
    pub original: bool,
    /// Hardsub locales available for separate playback requests.
    pub hardsubs: Vec<Locale>,
    /// Raw-video representations.
    pub video: Vec<VideoCapability>,
    /// Audio representations.
    pub audio: Vec<AudioCapability>,
    /// Subtitle and caption resources.
    pub subtitles: Vec<SubtitleCapability>,
}

/// Complete redacted choices available before a download is configured.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MediaCapabilities {
    /// Requested logical media id.
    pub media_id: String,
    /// Choices grouped by audio-language version.
    pub versions: Vec<VersionCapabilities>,
}

pub(crate) async fn inspect<A: CrunchyrollApi>(
    api: &A,
    platform: &StreamPlatform,
    media: &ResolvedMedia,
    cancellation: &CancellationToken,
) -> Result<MediaCapabilities, Error> {
    if media.versions.is_empty() {
        return Err(Error::Unavailable("media has no playback versions".into()));
    }
    let mut versions = Vec::with_capacity(media.versions.len());
    for version in &media.versions {
        versions.push(inspect_version(api, platform, version, cancellation).await?);
    }
    versions.sort_by(|left, right| {
        right
            .original
            .cmp(&left.original)
            .then_with(|| {
                left.audio_locale
                    .to_string()
                    .cmp(&right.audio_locale.to_string())
            })
            .then_with(|| left.version_id.cmp(&right.version_id))
    });
    Ok(MediaCapabilities {
        media_id: media.content_id.clone(),
        versions,
    })
}

async fn inspect_version<A: CrunchyrollApi>(
    api: &A,
    platform: &StreamPlatform,
    version: &crate::ResolvedVersion,
    cancellation: &CancellationToken,
) -> Result<VersionCapabilities, Error> {
    let mut guard = SessionGuard::new(api);
    let result = async {
        let index = guard
            .open(&version.content_id, platform, cancellation)
            .await?;
        let metadata = guard.metadata(index);
        let data = guard.stream_data(index, None, cancellation).await?;
        Ok(version_capabilities(
            &version.content_id,
            version.original,
            metadata.audio_locale,
            metadata.hardsubs,
            metadata.subtitles,
            data,
        ))
    }
    .await;
    let finalization = guard.finalize().await;
    match (result, finalization) {
        (Ok(capabilities), Ok(())) => Ok(capabilities),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), _) => Err(error),
    }
}

fn version_capabilities(
    version_id: &str,
    original: bool,
    audio_locale: Locale,
    mut hardsubs: Vec<Locale>,
    subtitles: Vec<ApiSubtitle>,
    data: StreamData,
) -> VersionCapabilities {
    hardsubs.sort_by_key(ToString::to_string);
    hardsubs.dedup();
    let mut video = data
        .video
        .iter()
        .map(|stream| VideoCapability {
            codec: stream.codecs.clone(),
            width: u32::try_from(stream.resolution.width).unwrap_or(u32::MAX),
            height: u32::try_from(stream.resolution.height).unwrap_or(u32::MAX),
            bandwidth: stream.bandwidth,
            fps: stream.fps,
            encrypted: stream.drm.is_some(),
            drm: drm_capabilities(stream.drm.as_ref()),
        })
        .collect::<Vec<_>>();
    video.sort_by(|left, right| {
        right
            .height
            .cmp(&left.height)
            .then_with(|| right.width.cmp(&left.width))
            .then_with(|| right.bandwidth.cmp(&left.bandwidth))
    });
    let mut audio = data
        .audio
        .iter()
        .map(|stream| AudioCapability {
            codec: stream.codecs.clone(),
            bandwidth: stream.bandwidth,
            sampling_rate: stream.sampling_rate,
            encrypted: stream.drm.is_some(),
            drm: drm_capabilities(stream.drm.as_ref()),
        })
        .collect::<Vec<_>>();
    audio.sort_by(|left, right| {
        right
            .bandwidth
            .cmp(&left.bandwidth)
            .then_with(|| right.sampling_rate.cmp(&left.sampling_rate))
            .then_with(|| left.codec.cmp(&right.codec))
    });
    let mut subtitles = subtitles
        .into_iter()
        .map(|subtitle| SubtitleCapability {
            locale: subtitle.locale,
            format: subtitle.format,
            is_caption: subtitle.is_caption,
        })
        .collect::<Vec<_>>();
    subtitles.sort_by(|left, right| {
        left.locale
            .to_string()
            .cmp(&right.locale.to_string())
            .then_with(|| left.is_caption.cmp(&right.is_caption))
            .then_with(|| left.format.cmp(&right.format))
    });
    subtitles.dedup();
    VersionCapabilities {
        version_id: version_id.to_string(),
        audio_locale,
        original,
        hardsubs,
        video,
        audio,
        subtitles,
    }
}

fn drm_capabilities(drm: Option<&MediaStreamDRM>) -> Vec<DrmCapability> {
    let mut capabilities = drm
        .into_iter()
        .flat_map(|drm| drm.types.iter())
        .map(|kind| match kind {
            MediaStreamDRMType::Playready { .. } => DrmCapability::PlayReady,
            MediaStreamDRMType::Widevine { .. } => DrmCapability::Widevine,
        })
        .collect::<Vec<_>>();
    capabilities.sort_by_key(|kind| match kind {
        DrmCapability::PlayReady => 0,
        DrmCapability::Widevine => 1,
    });
    capabilities.dedup();
    capabilities
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crunchyroll_rs::media::{SkipEvents, StreamData};
    use crunchyroll_rs::{Episode, Movie, MusicVideo, Season, Series};

    use super::*;

    struct MockApi {
        next_session: AtomicUsize,
        invalidated: Mutex<Vec<usize>>,
    }

    impl CrunchyrollApi for MockApi {
        type Session = usize;

        async fn episode_from_id(&self, _id: &str) -> Result<Episode, Error> {
            unreachable!()
        }
        async fn movie_from_id(&self, _id: &str) -> Result<Movie, Error> {
            unreachable!()
        }
        async fn music_video_from_id(&self, _id: &str) -> Result<MusicVideo, Error> {
            unreachable!()
        }
        async fn series_from_id(&self, _id: &str) -> Result<Series, Error> {
            unreachable!()
        }
        async fn season_from_id(&self, _id: &str) -> Result<Season, Error> {
            unreachable!()
        }
        async fn series_seasons(&self, _series: &Series) -> Result<Vec<Season>, Error> {
            unreachable!()
        }
        async fn season_episodes(&self, _season: &Season) -> Result<Vec<Episode>, Error> {
            unreachable!()
        }
        async fn open_playback(
            &self,
            _content_id: &str,
            _platform: &StreamPlatform,
        ) -> Result<Self::Session, Error> {
            Ok(self.next_session.fetch_add(1, Ordering::Relaxed))
        }
        fn session_metadata(&self, _session: &Self::Session) -> crate::api::SessionMetadata {
            crate::api::SessionMetadata {
                audio_locale: Locale::ja_JP,
                hardsubs: vec![Locale::en_US],
                subtitles: vec![ApiSubtitle {
                    locale: Locale::en_US,
                    url: "https://example.test/sub.ass?secret".into(),
                    format: "ass".into(),
                    is_caption: false,
                }],
                drm: crunchyroll_rs::media::StreamDrm::default(),
            }
        }
        async fn stream_data(
            &self,
            _session: &Self::Session,
            _hardsub: Option<Locale>,
        ) -> Result<Option<StreamData>, Error> {
            Ok(Some(StreamData {
                audio: Vec::new(),
                video: Vec::new(),
                subtitle: None,
            }))
        }
        async fn invalidate_playback(&self, session: Self::Session) -> Result<(), Error> {
            self.invalidated.lock().unwrap().push(session);
            Ok(())
        }
        async fn skip_events(
            &self,
            _content_id: &str,
            _kind: crate::MediaKind,
        ) -> Result<Option<SkipEvents>, Error> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn inspection_is_redacted_and_invalidates_its_session() {
        let api = MockApi {
            next_session: AtomicUsize::new(0),
            invalidated: Mutex::new(Vec::new()),
        };
        let media = ResolvedMedia {
            kind: crate::MediaKind::Episode,
            content_id: "EP".into(),
            series_id: Some("SERIES".into()),
            season_id: Some("SEASON".into()),
            series_title: Some("Series".into()),
            season_title: Some("Season".into()),
            identifier: Some("SERIES|S1|E1".into()),
            season_number: Some(1),
            season_sequence_number: Some(1.0),
            title: "Episode".into(),
            episode: Some("1".into()),
            episode_number: Some(1),
            sequence_number: 1.0,
            is_special: false,
            duration: Duration::from_secs(1),
            audio_locale: Some(Locale::ja_JP),
            subtitle_locales: vec![Locale::en_US],
            is_premium_only: false,
            availability_status: "available".into(),
            versions: [
                ("EP-JA", Locale::ja_JP, true),
                ("EP-EN", Locale::en_US, false),
                ("EP-DE", Locale::de_DE, false),
            ]
            .into_iter()
            .map(
                |(content_id, audio_locale, original)| crate::ResolvedVersion {
                    content_id: content_id.into(),
                    audio_locale: Some(audio_locale),
                    original,
                    is_premium_only: false,
                    roles: Vec::new(),
                },
            )
            .collect(),
        };
        let capabilities = inspect(
            &api,
            &StreamPlatform::default(),
            &media,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(capabilities.versions[0].hardsubs, vec![Locale::en_US]);
        assert_eq!(capabilities.versions[0].subtitles.len(), 1);
        assert!(!format!("{capabilities:?}").contains("secret"));
        assert_eq!(*api.invalidated.lock().unwrap(), vec![0, 1, 2]);
    }
}
