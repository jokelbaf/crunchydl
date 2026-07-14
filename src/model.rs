//! Service-neutral domain types and normalization of `crunchyroll-rs` media.
//!
//! [`MediaRequest`] keeps the original `crunchyroll-rs` media object at the API
//! boundary so callers can search and browse with that crate directly.
//! Normalization turns those objects into a stable [`ResolvedMedia`] that the
//! rest of the pipeline depends on.

use std::time::Duration;

use crunchyroll_rs::{Episode, Locale, Movie, MusicVideo};

/// A request to resolve (and, in later phases, download) a specific media item.
///
/// The caller obtains the wrapped object from `crunchyroll-rs` - for example via
/// `crunchyroll.media_from_id::<Episode>(id)` - and hands ownership to the
/// downloader. The media objects are boxed because they differ substantially in
/// size; construct a request ergonomically with [`From`]/[`Into`]
/// (`episode.into()`).
#[non_exhaustive]
pub enum MediaRequest {
    /// A single episode.
    Episode(Box<Episode>),
    /// A single movie.
    Movie(Box<Movie>),
    /// A single music video.
    MusicVideo(Box<MusicVideo>),
}

impl From<Episode> for MediaRequest {
    fn from(episode: Episode) -> Self {
        Self::Episode(Box::new(episode))
    }
}

impl From<Movie> for MediaRequest {
    fn from(movie: Movie) -> Self {
        Self::Movie(Box::new(movie))
    }
}

impl From<MusicVideo> for MediaRequest {
    fn from(music_video: MusicVideo) -> Self {
        Self::MusicVideo(Box::new(music_video))
    }
}

impl MediaRequest {
    /// The kind of the wrapped media.
    #[must_use]
    pub fn kind(&self) -> MediaKind {
        match self {
            Self::Episode(_) => MediaKind::Episode,
            Self::Movie(_) => MediaKind::Movie,
            Self::MusicVideo(_) => MediaKind::MusicVideo,
        }
    }

    /// Normalize the wrapped media into a [`ResolvedMedia`].
    #[must_use]
    pub fn resolve(&self) -> ResolvedMedia {
        match self {
            Self::Episode(episode) => ResolvedMedia::from_episode(episode),
            Self::Movie(movie) => ResolvedMedia::from_movie(movie),
            Self::MusicVideo(music_video) => ResolvedMedia::from_music_video(music_video),
        }
    }
}

/// The kind of a media item.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum MediaKind {
    /// A series episode (including specials).
    Episode,
    /// A movie.
    Movie,
    /// A music video.
    MusicVideo,
}

/// Stable, serializable identity of one downloadable media item.
///
/// Unlike [`MediaRequest`], this type does not retain a `crunchyroll-rs`
/// executor and can therefore be stored safely in a frontend queue. Resolve it
/// against an authenticated [`crate::Downloader`] immediately before planning
/// or downloading.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub enum MediaTarget {
    /// A single episode id.
    Episode(String),
    /// A single movie id.
    Movie(String),
    /// A single music-video id.
    MusicVideo(String),
}

impl MediaTarget {
    /// The kind of media identified by this target.
    #[must_use]
    pub fn kind(&self) -> MediaKind {
        match self {
            Self::Episode(_) => MediaKind::Episode,
            Self::Movie(_) => MediaKind::Movie,
            Self::MusicVideo(_) => MediaKind::MusicVideo,
        }
    }

    /// The modern Crunchyroll content id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Episode(id) | Self::Movie(id) | Self::MusicVideo(id) => id,
        }
    }
}

impl From<&ResolvedMedia> for MediaTarget {
    fn from(media: &ResolvedMedia) -> Self {
        match media.kind {
            MediaKind::Episode => Self::Episode(media.content_id.clone()),
            MediaKind::Movie => Self::Movie(media.content_id.clone()),
            MediaKind::MusicVideo => Self::MusicVideo(media.content_id.clone()),
        }
    }
}

/// A single audio-language version of a media item.
///
/// Episodes expose one version per dubbed language. Movies and music videos have
/// a single version whose audio locale is only known once a playback stream is
/// opened, so [`ResolvedVersion::audio_locale`] is [`None`] for them.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResolvedVersion {
    /// The modern content id used to open a playback stream for this version.
    pub content_id: String,
    /// The audio locale of this version, if known from metadata.
    pub audio_locale: Option<Locale>,
    /// Whether this version's audio is the media's native language.
    pub original: bool,
    /// Whether this version requires a premium subscription.
    pub is_premium_only: bool,
    /// Raw role tags reported by Crunchyroll (for example `dub`).
    pub roles: Vec<String>,
}

/// Normalized, service-neutral metadata for a requested media item.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResolvedMedia {
    /// The kind of media.
    pub kind: MediaKind,
    /// The modern content id of the media item itself.
    pub content_id: String,
    /// The parent series (or movie listing) id, if any.
    pub series_id: Option<String>,
    /// The parent season id, if any.
    pub season_id: Option<String>,
    /// The parent series (or movie listing) title, if any.
    pub series_title: Option<String>,
    /// The parent season title, if any.
    pub season_title: Option<String>,
    /// Stable service identifier shared by localized versions when available.
    pub identifier: Option<String>,
    /// Displayed season number when available.
    pub season_number: Option<u32>,
    /// Logical season ordering number when available.
    pub season_sequence_number: Option<f32>,
    /// The media title.
    pub title: String,
    /// The episode label as reported by Crunchyroll (may be non-numeric).
    pub episode: Option<String>,
    /// The numeric episode number, if present. Absent for many specials.
    pub episode_number: Option<u32>,
    /// A sequence number that is always populated, including for specials.
    pub sequence_number: f32,
    /// Whether this item is a special (no integral episode number).
    pub is_special: bool,
    /// The media duration.
    pub duration: Duration,
    /// The primary audio locale, if known from metadata.
    pub audio_locale: Option<Locale>,
    /// Subtitle locales advertised for this media.
    pub subtitle_locales: Vec<Locale>,
    /// Whether the media requires a premium subscription.
    pub is_premium_only: bool,
    /// The raw availability status string reported by Crunchyroll.
    pub availability_status: String,
    /// All audio versions of this media.
    pub versions: Vec<ResolvedVersion>,
}

impl ResolvedMedia {
    pub(crate) fn from_episode(episode: &Episode) -> Self {
        // `crunchyroll-rs` normally backfills a single version (see
        // `fix_empty_episode_versions`); mirror that when metadata is sparse so
        // audio selection always has at least one candidate.
        let versions = if episode.versions.is_empty() {
            vec![ResolvedVersion {
                content_id: episode.id.clone(),
                audio_locale: Some(episode.audio_locale.clone()),
                original: true,
                is_premium_only: episode.is_premium_only,
                roles: episode.roles.clone(),
            }]
        } else {
            episode
                .versions
                .iter()
                .map(|version| ResolvedVersion {
                    content_id: version.id.clone(),
                    audio_locale: Some(version.audio_locale.clone()),
                    original: version.original,
                    is_premium_only: version.is_premium_only,
                    roles: version.roles.clone(),
                })
                .collect()
        };

        Self {
            kind: MediaKind::Episode,
            content_id: episode.id.clone(),
            series_id: non_empty(&episode.series_id),
            season_id: non_empty(&episode.season_id),
            series_title: non_empty(&episode.series_title),
            season_title: non_empty(&episode.season_title),
            identifier: non_empty(&episode.identifier),
            season_number: Some(episode.season_number),
            season_sequence_number: Some(episode.season_sequence_number),
            title: episode.title.clone(),
            episode: non_empty(&episode.episode),
            episode_number: episode.episode_number,
            sequence_number: episode.sequence_number,
            is_special: episode.episode_number.is_none(),
            duration: millis_to_duration(episode.duration.num_milliseconds()),
            audio_locale: Some(episode.audio_locale.clone()),
            subtitle_locales: episode.subtitle_locales.clone(),
            is_premium_only: episode.is_premium_only,
            availability_status: episode.availability_status.clone(),
            versions,
        }
    }

    pub(crate) fn from_movie(movie: &Movie) -> Self {
        Self {
            kind: MediaKind::Movie,
            content_id: movie.id.clone(),
            series_id: non_empty(&movie.movie_listing_id),
            season_id: None,
            series_title: non_empty(&movie.movie_listing_title),
            season_title: None,
            identifier: None,
            season_number: None,
            season_sequence_number: None,
            title: movie.title.clone(),
            episode: None,
            episode_number: None,
            sequence_number: 0.0,
            is_special: false,
            duration: millis_to_duration(movie.duration.num_milliseconds()),
            audio_locale: None,
            subtitle_locales: Vec::new(),
            is_premium_only: movie.is_premium_only,
            availability_status: movie.availability_status.clone(),
            versions: vec![ResolvedVersion {
                content_id: movie.id.clone(),
                audio_locale: None,
                original: true,
                is_premium_only: movie.is_premium_only,
                roles: Vec::new(),
            }],
        }
    }

    pub(crate) fn from_music_video(music_video: &MusicVideo) -> Self {
        Self {
            kind: MediaKind::MusicVideo,
            content_id: music_video.id.clone(),
            series_id: None,
            season_id: None,
            series_title: None,
            season_title: None,
            identifier: None,
            season_number: None,
            season_sequence_number: None,
            title: music_video.title.clone(),
            episode: None,
            episode_number: None,
            sequence_number: music_video.sequence_number,
            is_special: false,
            duration: millis_to_duration(music_video.duration.num_milliseconds()),
            audio_locale: None,
            subtitle_locales: Vec::new(),
            is_premium_only: music_video.is_premium_only,
            availability_status: if music_video.is_public {
                "available".to_string()
            } else {
                "unavailable".to_string()
            },
            versions: vec![ResolvedVersion {
                content_id: music_video.id.clone(),
                audio_locale: None,
                original: true,
                is_premium_only: music_video.is_premium_only,
                roles: Vec::new(),
            }],
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn millis_to_duration(millis: i64) -> Duration {
    Duration::from_millis(u64::try_from(millis).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episodes() -> Vec<Episode> {
        serde_json::from_str(include_str!("../tests/fixtures/api/season_episodes.json"))
            .expect("valid episode fixture json")
    }

    #[test]
    fn normalizes_multi_version_episode() {
        let media = ResolvedMedia::from_episode(&episodes()[0]);

        assert_eq!(media.kind, MediaKind::Episode);
        assert_eq!(media.content_id, "EP1");
        assert_eq!(media.series_id.as_deref(), Some("SERIES1"));
        assert_eq!(media.series_title.as_deref(), Some("My Series"));
        assert_eq!(media.season_id.as_deref(), Some("SEASON1"));
        assert_eq!(media.season_title.as_deref(), Some("Season 1"));
        assert_eq!(media.episode.as_deref(), Some("1"));
        assert_eq!(media.episode_number, Some(1));
        assert!(!media.is_special);
        assert_eq!(media.duration, Duration::from_secs(1440));
        assert_eq!(media.audio_locale, Some(Locale::ja_JP));
        assert_eq!(media.subtitle_locales, vec![Locale::en_US, Locale::de_DE]);

        assert_eq!(media.versions.len(), 2);
        assert_eq!(media.versions[0].content_id, "EP1");
        assert_eq!(media.versions[0].audio_locale, Some(Locale::ja_JP));
        assert!(media.versions[0].original);
        assert_eq!(media.versions[1].content_id, "EP1-EN");
        assert_eq!(media.versions[1].audio_locale, Some(Locale::en_US));
        assert!(!media.versions[1].original);
        assert!(media.versions[1].is_premium_only);
        assert_eq!(media.versions[1].roles, vec!["dub".to_string()]);
    }

    #[test]
    fn synthesizes_single_version_when_absent() {
        // EP2 has an empty `versions` array.
        let media = ResolvedMedia::from_episode(&episodes()[1]);
        assert_eq!(media.versions.len(), 1);
        assert_eq!(media.versions[0].content_id, "EP2");
        assert_eq!(media.versions[0].audio_locale, Some(Locale::ja_JP));
        assert!(media.versions[0].original);
    }

    #[test]
    fn marks_special_without_episode_number() {
        let media = ResolvedMedia::from_episode(&episodes()[2]);
        assert!(media.is_special);
        assert_eq!(media.episode_number, None);
        assert_eq!(media.episode.as_deref(), Some("SP"));
        assert!((media.sequence_number - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn normalizes_movie() {
        let movie: Movie = serde_json::from_str(include_str!("../tests/fixtures/api/movie.json"))
            .expect("valid movie fixture json");
        let media = ResolvedMedia::from_movie(&movie);

        assert_eq!(media.kind, MediaKind::Movie);
        assert_eq!(media.content_id, "MOVIE1");
        assert_eq!(media.series_id.as_deref(), Some("ML1"));
        assert_eq!(media.series_title.as_deref(), Some("My Movie Listing"));
        assert_eq!(media.duration, Duration::from_secs(5400));
        assert_eq!(media.audio_locale, None);
        assert!(media.is_premium_only);
        assert_eq!(media.versions.len(), 1);
        assert_eq!(media.versions[0].content_id, "MOVIE1");
        assert_eq!(media.versions[0].audio_locale, None);
    }

    #[test]
    fn normalizes_music_video() {
        let music_video: MusicVideo =
            serde_json::from_str(include_str!("../tests/fixtures/api/music_video.json"))
                .expect("valid music video fixture json");
        let media = ResolvedMedia::from_music_video(&music_video);

        assert_eq!(media.kind, MediaKind::MusicVideo);
        assert_eq!(media.content_id, "MV1");
        assert_eq!(media.title, "My Music Video");
        assert_eq!(media.duration, Duration::from_secs(210));
        assert_eq!(media.availability_status, "available");
        assert_eq!(media.versions.len(), 1);
        assert_eq!(media.audio_locale, None);
    }
}
