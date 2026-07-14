//! The `crunchyroll-rs` API seam and its production adapter.
//!
//! The pipeline talks to Crunchyroll through the [`CrunchyrollApi`] trait so it
//! can be exercised against fixtures without network access. [`ProductionApi`]
//! forwards to a real, authenticated client.
//!
//! In this phase the trait covers media lookup and the series/season helpers.
//! Playback, subtitle, skip-event, license-token, and invalidation operations
//! are added when playback lands in a later phase.

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

/// Fetch and normalize an episode by id.
pub(crate) async fn resolve_episode_id<A: CrunchyrollApi>(
    api: &A,
    id: &str,
) -> Result<ResolvedMedia, Error> {
    Ok(ResolvedMedia::from_episode(&api.episode_from_id(id).await?))
}

/// Fetch and normalize a movie by id.
pub(crate) async fn resolve_movie_id<A: CrunchyrollApi>(
    api: &A,
    id: &str,
) -> Result<ResolvedMedia, Error> {
    Ok(ResolvedMedia::from_movie(&api.movie_from_id(id).await?))
}

/// Fetch and normalize a music video by id.
pub(crate) async fn resolve_music_video_id<A: CrunchyrollApi>(
    api: &A,
    id: &str,
) -> Result<ResolvedMedia, Error> {
    Ok(ResolvedMedia::from_music_video(
        &api.music_video_from_id(id).await?,
    ))
}

/// Fetch the concrete service object identified by a stable target.
pub(crate) async fn media_request_from_target<A: CrunchyrollApi>(
    api: &A,
    target: &MediaTarget,
) -> Result<MediaRequest, Error> {
    match target {
        MediaTarget::Episode(id) => Ok(api.episode_from_id(id).await?.into()),
        MediaTarget::Movie(id) => Ok(api.movie_from_id(id).await?.into()),
        MediaTarget::MusicVideo(id) => Ok(api.music_video_from_id(id).await?.into()),
    }
}

/// Fetch and normalize a stable media target.
pub(crate) async fn resolve_target<A: CrunchyrollApi>(
    api: &A,
    target: &MediaTarget,
) -> Result<ResolvedMedia, Error> {
    Ok(media_request_from_target(api, target).await?.resolve())
}

/// Fetch a season by id and resolve every episode into normalized media.
pub(crate) async fn resolve_season_id<A: CrunchyrollApi>(
    api: &A,
    id: &str,
) -> Result<Vec<ResolvedMedia>, Error> {
    let season = api.season_from_id(id).await?;
    let episodes = api.season_episodes(&season).await?;
    Ok(crate::canonicalize_episode_batch(
        episodes.iter().map(ResolvedMedia::from_episode),
    )?)
}

/// Fetch a series by id and resolve every episode of every season into
/// normalized media.
pub(crate) async fn resolve_series_id<A: CrunchyrollApi>(
    api: &A,
    id: &str,
) -> Result<Vec<ResolvedMedia>, Error> {
    let series = api.series_from_id(id).await?;
    let seasons = api.series_seasons(&series).await?;
    let mut resolved = Vec::new();
    for season in &seasons {
        let episodes = api.season_episodes(season).await?;
        resolved.extend(episodes.iter().map(ResolvedMedia::from_episode));
    }
    Ok(crate::canonicalize_episode_batch(resolved)?)
}

/// Expand a stable collection into durable, filtered download targets.
pub(crate) async fn expand_collection<A: CrunchyrollApi>(
    api: &A,
    target: &crate::CollectionTarget,
    options: &crate::BatchOptions,
) -> Result<Vec<MediaTarget>, Error> {
    match target {
        crate::CollectionTarget::Season(id) => {
            let episodes = resolve_season_id(api, id).await?;
            Ok(crate::select_batch_targets(&episodes, options))
        }
        crate::CollectionTarget::Series(id) => {
            let episodes = resolve_series_id(api, id).await?;
            Ok(crate::select_batch_targets(&episodes, options))
        }
        crate::CollectionTarget::MovieListing(id) => {
            let listing = api.movie_listing_from_id(id).await?;
            let mut movies = api.movie_listing_movies(&listing).await?;
            movies.sort_by(|left, right| {
                left.title
                    .cmp(&right.title)
                    .then_with(|| left.id.cmp(&right.id))
            });
            Ok(movies
                .into_iter()
                .map(|movie| MediaTarget::Movie(movie.id))
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    /// A fixture-backed [`CrunchyrollApi`] for testing the resolver flow without
    /// network access. Only the maps a test needs must be populated.
    #[derive(Default)]
    struct MockApi {
        episodes_by_id: HashMap<String, Episode>,
        seasons_by_series: HashMap<String, Vec<Season>>,
        episodes_by_season: HashMap<String, Vec<Episode>>,
    }

    impl CrunchyrollApi for MockApi {
        type Session = ();

        async fn episode_from_id(&self, id: &str) -> Result<Episode, Error> {
            self.episodes_by_id
                .get(id)
                .cloned()
                .ok_or_else(|| Error::Unavailable(format!("mock has no episode {id}")))
        }
        async fn movie_from_id(&self, id: &str) -> Result<Movie, Error> {
            Err(Error::Unavailable(format!("mock has no movie {id}")))
        }
        async fn music_video_from_id(&self, id: &str) -> Result<MusicVideo, Error> {
            Err(Error::Unavailable(format!("mock has no music video {id}")))
        }
        async fn series_from_id(&self, id: &str) -> Result<Series, Error> {
            Ok(series_with_id(id))
        }
        async fn season_from_id(&self, id: &str) -> Result<Season, Error> {
            Ok(season_with_id(id))
        }
        async fn series_seasons(&self, series: &Series) -> Result<Vec<Season>, Error> {
            Ok(self
                .seasons_by_series
                .get(&series.id)
                .cloned()
                .unwrap_or_default())
        }
        async fn season_episodes(&self, season: &Season) -> Result<Vec<Episode>, Error> {
            Ok(self
                .episodes_by_season
                .get(&season.id)
                .cloned()
                .unwrap_or_default())
        }
        async fn open_playback(
            &self,
            _content_id: &str,
            _platform: &StreamPlatform,
        ) -> Result<Self::Session, Error> {
            Err(Error::Unavailable(
                "mock playback not configured".to_string(),
            ))
        }
        fn session_metadata(&self, _session: &Self::Session) -> SessionMetadata {
            unreachable!("mock playback not configured")
        }
        async fn stream_data(
            &self,
            _session: &Self::Session,
            _hardsub: Option<crunchyroll_rs::Locale>,
        ) -> Result<Option<StreamData>, Error> {
            Err(Error::Unavailable(
                "mock playback not configured".to_string(),
            ))
        }
        async fn invalidate_playback(&self, _session: Self::Session) -> Result<(), Error> {
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

    fn episodes() -> Vec<Episode> {
        serde_json::from_str(include_str!("../tests/fixtures/api/season_episodes.json"))
            .expect("valid episode fixture json")
    }

    fn season_with_id(id: &str) -> Season {
        serde_json::from_str(&format!(r#"{{"id":"{id}"}}"#)).expect("valid season json")
    }

    fn series_with_id(id: &str) -> Series {
        serde_json::from_str(&format!(r#"{{"id":"{id}"}}"#)).expect("valid series json")
    }

    #[tokio::test]
    async fn resolves_episode_by_id() {
        let mut api = MockApi::default();
        api.episodes_by_id
            .insert("EP1".to_string(), episodes().swap_remove(0));

        let media = resolve_episode_id(&api, "EP1")
            .await
            .expect("episode resolves");
        assert_eq!(media.content_id, "EP1");
        assert_eq!(media.episode_number, Some(1));
        assert_eq!(media.versions.len(), 2);
    }

    #[tokio::test]
    async fn resolves_season_episodes_through_the_seam() {
        let mut api = MockApi::default();
        api.episodes_by_season
            .insert("SEASON1".to_string(), episodes());

        let resolved = resolve_season_id(&api, "SEASON1")
            .await
            .expect("resolution succeeds");
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].episode_number, Some(1));
        assert_eq!(resolved[1].episode_number, Some(2));
        assert!(resolved[2].is_special);
        assert_eq!(resolved[2].episode_number, None);
    }

    #[tokio::test]
    async fn resolves_series_episodes_across_seasons() {
        let mut api = MockApi::default();
        api.seasons_by_series
            .insert("SERIES1".to_string(), vec![season_with_id("SEASON1")]);
        api.episodes_by_season
            .insert("SEASON1".to_string(), episodes());

        let resolved = resolve_series_id(&api, "SERIES1")
            .await
            .expect("resolution succeeds");
        assert_eq!(resolved.len(), 3);
    }
}
