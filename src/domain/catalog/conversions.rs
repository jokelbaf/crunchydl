//! Conversions from Crunchyroll catalog objects.

use super::*;

impl CatalogItem {
    /// Select the highest-resolution artwork suited to this item's detail view.
    #[must_use]
    pub fn best_artwork(&self) -> Option<&CatalogImage> {
        let preferred = match self.kind {
            CatalogKind::Series | CatalogKind::MovieListing => CatalogImageKind::PosterWide,
            CatalogKind::Episode | CatalogKind::Movie | CatalogKind::MusicVideo => {
                CatalogImageKind::Thumbnail
            }
            _ => CatalogImageKind::PosterTall,
        };
        largest_image(self.images.iter().filter(|image| image.kind == preferred))
            .or_else(|| largest_image(self.images.iter()))
    }

    /// Convert a search result without retaining its API executor.
    #[must_use]
    pub fn from_search(value: &SearchMediaCollection) -> Self {
        match value {
            SearchMediaCollection::Series(series) => Self::from_search_series(series),
            SearchMediaCollection::Episode(episode) => Self::from_search_episode(episode),
            SearchMediaCollection::MovieListing(listing) => {
                Self::from_search_movie_listing(listing)
            }
            SearchMediaCollection::MusicVideo(video) => Self::from_search_music_video(video),
            SearchMediaCollection::Concert(concert) => Self {
                id: concert.id.clone(),
                kind: CatalogKind::Concert,
                target: None,
                title: concert.title.clone(),
                description: concert.description.clone(),
                extended_description: None,
                images: thumbnail_images(&concert.images.thumbnail),
                rating: None,
                release_year: None,
                season_number: None,
                episode_number: None,
                season_count: None,
                episode_count: None,
                duration_millis: duration_millis(concert.duration.num_milliseconds()),
                premium_only: concert.is_premium_only,
                is_subbed: false,
                is_dubbed: false,
                audio_locales: Vec::new(),
                subtitle_locales: Vec::new(),
            },
        }
    }

    /// Convert series metadata.
    #[must_use]
    pub fn from_series(series: &Series) -> Self {
        let mut images = catalog_images(&series.images.poster_tall, CatalogImageKind::PosterTall);
        images.extend(catalog_images(
            &series.images.poster_wide,
            CatalogImageKind::PosterWide,
        ));
        Self {
            id: series.id.clone(),
            kind: CatalogKind::Series,
            target: None,
            title: series.title.clone(),
            description: series.description.clone(),
            extended_description: non_empty(&series.extended_description),
            images,
            rating: None,
            release_year: series.series_launch_year,
            season_number: None,
            episode_number: None,
            season_count: Some(series.season_count),
            episode_count: Some(series.episode_count),
            duration_millis: None,
            premium_only: false,
            is_subbed: series.is_subbed,
            is_dubbed: series.is_dubbed,
            audio_locales: series.audio_locales.clone(),
            subtitle_locales: series.subtitle_locales.clone(),
        }
    }

    /// Convert series metadata returned by a search endpoint, including rating.
    #[must_use]
    pub fn from_search_series(series: &SearchSeries) -> Self {
        let mut item = Self::from_series(series);
        item.rating = Some(CatalogRating::Stars {
            average: series.search_rating.average,
            total: Some(series.search_rating.total),
        });
        item
    }

    /// Convert season metadata.
    #[must_use]
    pub fn from_season(season: &Season) -> Self {
        Self {
            id: season.id.clone(),
            kind: CatalogKind::Season,
            target: None,
            title: season.title.clone(),
            description: season.description.clone(),
            extended_description: None,
            images: Vec::new(),
            rating: None,
            release_year: None,
            season_number: Some(season.season_number),
            episode_number: None,
            season_count: None,
            episode_count: Some(season.number_of_episodes),
            duration_millis: None,
            premium_only: false,
            is_subbed: season.is_subbed,
            is_dubbed: season.is_dubbed,
            audio_locales: season.audio_locales.clone(),
            subtitle_locales: season.subtitle_locales.clone(),
        }
    }

    /// Convert episode metadata.
    #[must_use]
    pub fn from_episode(episode: &Episode) -> Self {
        Self {
            id: episode.id.clone(),
            kind: CatalogKind::Episode,
            target: Some(MediaTarget::Episode(episode.id.clone())),
            title: episode.title.clone(),
            description: episode.description.clone(),
            extended_description: None,
            images: thumbnail_images(&episode.images),
            rating: None,
            release_year: None,
            season_number: Some(episode.season_number),
            episode_number: episode_label(episode),
            season_count: None,
            episode_count: None,
            duration_millis: duration_millis(episode.duration.num_milliseconds()),
            premium_only: episode.is_premium_only,
            is_subbed: episode.is_subbed,
            is_dubbed: episode.is_dubbed,
            audio_locales: unique_locales(
                std::iter::once(episode.audio_locale.clone()).chain(
                    episode
                        .versions
                        .iter()
                        .map(|version| version.audio_locale.clone()),
                ),
            ),
            subtitle_locales: episode.subtitle_locales.clone(),
        }
    }

    /// Convert episode metadata returned by a search endpoint, including votes.
    #[must_use]
    pub fn from_search_episode(episode: &SearchEpisode) -> Self {
        let mut item = Self::from_episode(episode);
        item.rating = episode
            .search_rating
            .as_ref()
            .and_then(CatalogRating::from_episode_rating);
        item
    }

    /// Convert movie-listing metadata.
    #[must_use]
    pub fn from_movie_listing(listing: &MovieListing) -> Self {
        let mut images = catalog_images(&listing.images.poster_tall, CatalogImageKind::PosterTall);
        images.extend(catalog_images(
            &listing.images.poster_wide,
            CatalogImageKind::PosterWide,
        ));
        Self {
            id: listing.id.clone(),
            kind: CatalogKind::MovieListing,
            target: None,
            title: listing.title.clone(),
            description: listing.description.clone(),
            extended_description: non_empty(&listing.extended_description),
            images,
            rating: None,
            release_year: Some(listing.movie_release_year),
            season_number: None,
            episode_number: None,
            season_count: None,
            episode_count: None,
            duration_millis: None,
            premium_only: listing.is_premium_only,
            is_subbed: listing.is_subbed,
            is_dubbed: listing.is_dubbed,
            audio_locales: listing
                .audio_locale
                .iter()
                .cloned()
                .chain(
                    listing
                        .versions
                        .iter()
                        .map(|version| version.audio_locale.clone()),
                )
                .collect(),
            subtitle_locales: listing.subtitle_locales.clone(),
        }
    }

    /// Convert movie-listing search metadata, including rating.
    #[must_use]
    pub fn from_search_movie_listing(listing: &SearchMovieListing) -> Self {
        let mut item = Self::from_movie_listing(listing);
        item.rating = Some(CatalogRating::Stars {
            average: listing.search_rating.average,
            total: Some(listing.search_rating.total),
        });
        item
    }

    /// Convert movie metadata.
    #[must_use]
    pub fn from_movie(movie: &Movie) -> Self {
        Self {
            id: movie.id.clone(),
            kind: CatalogKind::Movie,
            target: Some(MediaTarget::Movie(movie.id.clone())),
            title: movie.title.clone(),
            description: movie.description.clone(),
            extended_description: None,
            images: thumbnail_images(&movie.images.thumbnail),
            rating: None,
            release_year: None,
            season_number: None,
            episode_number: None,
            season_count: None,
            episode_count: None,
            duration_millis: duration_millis(movie.duration.num_milliseconds()),
            premium_only: movie.is_premium_only,
            is_subbed: movie.is_subbed,
            is_dubbed: movie.is_dubbed,
            audio_locales: Vec::new(),
            subtitle_locales: Vec::new(),
        }
    }

    /// Convert music-video metadata.
    #[must_use]
    pub fn from_music_video(video: &MusicVideo) -> Self {
        Self {
            id: video.id.clone(),
            kind: CatalogKind::MusicVideo,
            target: Some(MediaTarget::MusicVideo(video.id.clone())),
            title: video.title.clone(),
            description: video.description.clone(),
            extended_description: None,
            images: thumbnail_images(&video.images.thumbnail),
            rating: None,
            release_year: None,
            season_number: None,
            episode_number: None,
            season_count: None,
            episode_count: None,
            duration_millis: duration_millis(video.duration.num_milliseconds()),
            premium_only: video.is_premium_only,
            is_subbed: false,
            is_dubbed: false,
            audio_locales: Vec::new(),
            subtitle_locales: Vec::new(),
        }
    }

    /// Convert music-video search metadata.
    #[must_use]
    pub fn from_search_music_video(video: &SearchMusicVideo) -> Self {
        Self::from_music_video(video)
    }
}
