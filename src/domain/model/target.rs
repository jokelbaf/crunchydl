//! Stable media targets used by queues and frontends.

use super::{MediaKind, ResolvedMedia};

/// Stable, serializable identity of one downloadable media item.
///
/// Unlike [`crate::MediaRequest`], this type does not retain a `crunchyroll-rs`
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
