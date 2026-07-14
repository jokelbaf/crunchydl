//! The `crunchyroll-rs` API seam and its production adapter.
//!
//! The pipeline talks to Crunchyroll through the [`CrunchyrollApi`] trait so it
//! can be exercised against fixtures without network access. [`ProductionApi`]
//! forwards to a real, authenticated client.

use crunchyroll_rs::media::{SkipEvents, Stream, StreamData, StreamDrm, StreamPlatform};
use crunchyroll_rs::{Crunchyroll, Episode, Movie, MovieListing, MusicVideo, Season, Series};

use crate::error::Error;
use crate::model::{MediaRequest, MediaTarget, ResolvedMedia};

#[derive(Clone)]
pub(crate) struct ApiSubtitle {
    pub(crate) locale: crunchyroll_rs::Locale,
    pub(crate) url: String,
    pub(crate) format: String,
    pub(crate) is_caption: bool,
}

pub(crate) struct SessionMetadata {
    pub(crate) audio_locale: crunchyroll_rs::Locale,
    pub(crate) hardsubs: Vec<crunchyroll_rs::Locale>,
    pub(crate) subtitles: Vec<ApiSubtitle>,
    pub(crate) drm: StreamDrm,
}

/// Internal seam over the `crunchyroll-rs` operations the downloader needs.
pub(crate) trait CrunchyrollApi {
    /// Concrete playback-session handle owned by this adapter.
    type Session: Send;
    /// Look up an episode by its modern content id.
    async fn episode_from_id(&self, id: &str) -> Result<Episode, Error>;
    /// Look up a movie by its modern content id.
    async fn movie_from_id(&self, id: &str) -> Result<Movie, Error>;
    /// Look up a music video by its modern content id.
    async fn music_video_from_id(&self, id: &str) -> Result<MusicVideo, Error>;
    /// Look up a series by its modern content id.
    async fn series_from_id(&self, id: &str) -> Result<Series, Error>;
    /// Look up a season by its modern content id.
    async fn season_from_id(&self, id: &str) -> Result<Season, Error>;
    /// Look up a movie listing by its modern content id.
    async fn movie_listing_from_id(&self, _id: &str) -> Result<MovieListing, Error> {
        Err(Error::Unavailable(
            "movie-listing lookup is unavailable in this API adapter".into(),
        ))
    }
    /// Return every season of a series.
    async fn series_seasons(&self, series: &Series) -> Result<Vec<Season>, Error>;
    /// Return every episode of a season.
    async fn season_episodes(&self, season: &Season) -> Result<Vec<Episode>, Error>;
    /// Return every movie in a movie listing.
    async fn movie_listing_movies(&self, _listing: &MovieListing) -> Result<Vec<Movie>, Error> {
        Err(Error::Unavailable(
            "movie-listing expansion is unavailable in this API adapter".into(),
        ))
    }
    /// Open a playback session for a media version.
    async fn open_playback(
        &self,
        content_id: &str,
        platform: &StreamPlatform,
    ) -> Result<Self::Session, Error>;
    /// Copy safe selection metadata out of an open session.
    fn session_metadata(&self, session: &Self::Session) -> SessionMetadata;
    /// Parse the selected manifest exactly once for a playback session.
    async fn stream_data(
        &self,
        session: &Self::Session,
        hardsub: Option<crunchyroll_rs::Locale>,
    ) -> Result<Option<StreamData>, Error>;
    /// Explicitly invalidate a playback session.
    async fn invalidate_playback(&self, session: Self::Session) -> Result<(), Error>;
    /// Fetch neutral skip-event source data for an episode or movie.
    async fn skip_events(
        &self,
        content_id: &str,
        kind: crate::MediaKind,
    ) -> Result<Option<SkipEvents>, Error>;
    /// Download one selected subtitle resource as bounded UTF-8 text.
    async fn fetch_subtitle(&self, _url: &str) -> Result<String, Error> {
        Err(Error::Subtitle(
            "subtitle fetching is unavailable in this API adapter".to_string(),
        ))
    }
}

/// The production [`CrunchyrollApi`] backed by an authenticated client.
pub(crate) struct ProductionApi {
    crunchyroll: Crunchyroll,
}

impl ProductionApi {
    pub(crate) fn new(crunchyroll: Crunchyroll) -> Self {
        Self { crunchyroll }
    }
}

impl CrunchyrollApi for ProductionApi {
    type Session = Stream;

    async fn episode_from_id(&self, id: &str) -> Result<Episode, Error> {
        Ok(self.crunchyroll.media_from_id::<Episode>(id).await?)
    }

    async fn movie_from_id(&self, id: &str) -> Result<Movie, Error> {
        Ok(self.crunchyroll.media_from_id::<Movie>(id).await?)
    }

    async fn music_video_from_id(&self, id: &str) -> Result<MusicVideo, Error> {
        Ok(self.crunchyroll.media_from_id::<MusicVideo>(id).await?)
    }

    async fn series_from_id(&self, id: &str) -> Result<Series, Error> {
        Ok(self.crunchyroll.media_from_id::<Series>(id).await?)
    }

    async fn season_from_id(&self, id: &str) -> Result<Season, Error> {
        Ok(self.crunchyroll.media_from_id::<Season>(id).await?)
    }

    async fn movie_listing_from_id(&self, id: &str) -> Result<MovieListing, Error> {
        Ok(self.crunchyroll.media_from_id::<MovieListing>(id).await?)
    }

    async fn series_seasons(&self, series: &Series) -> Result<Vec<Season>, Error> {
        Ok(series.seasons().await?)
    }

    async fn season_episodes(&self, season: &Season) -> Result<Vec<Episode>, Error> {
        Ok(season.episodes().await?)
    }

    async fn movie_listing_movies(&self, listing: &MovieListing) -> Result<Vec<Movie>, Error> {
        Ok(listing.movies().await?)
    }

    async fn open_playback(
        &self,
        content_id: &str,
        platform: &StreamPlatform,
    ) -> Result<Self::Session, Error> {
        Ok(Stream::from_id(&self.crunchyroll, content_id, platform).await?)
    }

    fn session_metadata(&self, session: &Self::Session) -> SessionMetadata {
        let subtitles = session
            .subtitles
            .iter()
            .map(|subtitle| ApiSubtitle {
                locale: subtitle.locale.clone(),
                url: subtitle.url.clone(),
                format: subtitle.format.clone(),
                is_caption: false,
            })
            .chain(session.captions.iter().map(|subtitle| ApiSubtitle {
                locale: subtitle.locale.clone(),
                url: subtitle.url.clone(),
                format: subtitle.format.clone(),
                is_caption: true,
            }))
            .collect();
        let mut hardsubs = session.hard_subs.keys().cloned().collect::<Vec<_>>();
        hardsubs.sort_by_key(ToString::to_string);
        SessionMetadata {
            audio_locale: session.audio_locale.clone(),
            hardsubs,
            subtitles,
            drm: session.drm.clone(),
        }
    }

    async fn stream_data(
        &self,
        session: &Self::Session,
        hardsub: Option<crunchyroll_rs::Locale>,
    ) -> Result<Option<StreamData>, Error> {
        Ok(session.stream_data(hardsub).await?)
    }

    async fn invalidate_playback(&self, session: Self::Session) -> Result<(), Error> {
        Ok(session.invalidate().await?)
    }

    async fn skip_events(
        &self,
        content_id: &str,
        kind: crate::MediaKind,
    ) -> Result<Option<SkipEvents>, Error> {
        match kind {
            crate::MediaKind::Episode => Ok(self
                .crunchyroll
                .media_from_id::<Episode>(content_id)
                .await?
                .skip_events()
                .await?),
            crate::MediaKind::Movie => Ok(self
                .crunchyroll
                .media_from_id::<Movie>(content_id)
                .await?
                .skip_events()
                .await?),
            crate::MediaKind::MusicVideo => Ok(None),
        }
    }

    async fn fetch_subtitle(&self, url: &str) -> Result<String, Error> {
        const MAX_SUBTITLE_BYTES: u64 = 16 * 1024 * 1024;
        let response = self
            .crunchyroll
            .client()
            .get(url)
            .send()
            .await
            .map_err(|_| Error::Subtitle("subtitle request failed".to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Subtitle(format!(
                "subtitle request returned HTTP {}",
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SUBTITLE_BYTES)
        {
            return Err(Error::Subtitle(
                "subtitle resource exceeds size limit".to_string(),
            ));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|_| Error::Subtitle("subtitle response body failed".to_string()))?;
        if bytes.len() as u64 > MAX_SUBTITLE_BYTES {
            return Err(Error::Subtitle(
                "subtitle resource exceeds size limit".to_string(),
            ));
        }
        String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::Subtitle("subtitle resource is not UTF-8".to_string()))
    }
}

mod resolution;
pub(crate) use resolution::{
    expand_collection, media_request_from_target, resolve_episode_id, resolve_movie_id,
    resolve_music_video_id, resolve_season_id, resolve_series_id, resolve_target,
};
