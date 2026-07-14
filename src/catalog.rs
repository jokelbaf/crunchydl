//! Stable catalog view models for terminal and graphical frontends.

use crunchyroll_rs::common::Image;
use crunchyroll_rs::media::{EpisodeRating, Rating};
use crunchyroll_rs::search::{
    SearchEpisode, SearchMediaCollection, SearchMovieListing, SearchMusicVideo, SearchSeries,
};
use crunchyroll_rs::{Episode, Locale, Movie, MovieListing, MusicVideo, Season, Series};

use crate::MediaTarget;

/// Catalog media kinds, including non-downloadable parent collections.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum CatalogKind {
    /// An anime series.
    Series,
    /// One season of a series.
    Season,
    /// A downloadable episode.
    Episode,
    /// A collection containing one or more movies.
    MovieListing,
    /// A downloadable movie.
    Movie,
    /// A downloadable music video.
    MusicVideo,
    /// A concert catalog item not supported by the downloader yet.
    Concert,
}

/// How an image is intended to be displayed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum CatalogImageKind {
    /// Portrait poster artwork.
    PosterTall,
    /// Landscape poster artwork.
    PosterWide,
    /// Episode, movie, or music-video thumbnail.
    Thumbnail,
}

/// A safe catalog image descriptor.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct CatalogImage {
    /// Public image source URL.
    pub source: String,
    /// Pixel width advertised by the catalog.
    pub width: u32,
    /// Pixel height advertised by the catalog.
    pub height: u32,
    /// Intended presentation shape.
    pub kind: CatalogImageKind,
}

/// User-rating summary returned by the catalog.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogRating {
    /// A one-to-five-star rating used for series and movie listings.
    Stars {
        /// Average number of stars.
        average: f64,
        /// Number of submitted ratings when reported.
        total: Option<u32>,
    },
    /// Positive-vote percentage used for episodes.
    Approval {
        /// Approximate percentage of positive votes.
        percentage: f64,
        /// Number of submitted votes when reported.
        total: Option<u32>,
    },
}

impl CatalogRating {
    /// Convert the rating returned by a series or movie-listing detail endpoint.
    #[must_use]
    pub fn from_star_rating(rating: &Rating) -> Self {
        Self::Stars {
            average: rating.average,
            total: Some(rating.total),
        }
    }

    /// Convert the positive/negative rating returned by an episode endpoint.
    #[must_use]
    pub fn from_episode_rating(rating: &EpisodeRating) -> Option<Self> {
        let value = serde_json::to_value(rating).ok()?;
        let total = value
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let positive = vote_count(value.get("up")?)?;
        let negative = vote_count(value.get("down")?)?;
        let votes = positive + negative;
        (votes > 0.0).then(|| Self::Approval {
            percentage: positive * 100.0 / votes,
            total,
        })
    }
}

/// Frontend-facing metadata for one catalog result or detail page.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CatalogItem {
    /// Modern Crunchyroll catalog id.
    pub id: String,
    /// Media or collection kind.
    pub kind: CatalogKind,
    /// Download target when the item itself is downloadable.
    pub target: Option<MediaTarget>,
    /// Localized title.
    pub title: String,
    /// Localized short description.
    pub description: String,
    /// Localized extended description when supplied separately.
    pub extended_description: Option<String>,
    /// Artwork ordered as returned by the service.
    pub images: Vec<CatalogImage>,
    /// User rating when included by the endpoint or loaded for a detail page.
    pub rating: Option<CatalogRating>,
    /// Release or launch year when known.
    pub release_year: Option<u32>,
    /// Season number for season and episode rows when known.
    pub season_number: Option<u32>,
    /// Service-provided episode label, including decimal and special labels.
    pub episode_number: Option<String>,
    /// Number of seasons when this is a series.
    pub season_count: Option<u32>,
    /// Number of episodes when reported.
    pub episode_count: Option<u32>,
    /// Duration in milliseconds for playable items.
    pub duration_millis: Option<u64>,
    /// Whether some or all of the item is premium-only.
    pub premium_only: bool,
    /// Whether subtitle metadata is advertised.
    pub is_subbed: bool,
    /// Whether dubbed metadata is advertised.
    pub is_dubbed: bool,
    /// Audio locales advertised without opening playback.
    pub audio_locales: Vec<Locale>,
    /// Subtitle locales advertised without opening playback.
    pub subtitle_locales: Vec<Locale>,
}

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

fn thumbnail_images(values: &[Image]) -> Vec<CatalogImage> {
    catalog_images(values, CatalogImageKind::Thumbnail)
}

fn catalog_images(values: &[Image], kind: CatalogImageKind) -> Vec<CatalogImage> {
    values
        .iter()
        .map(|image| CatalogImage {
            source: image.source.clone(),
            width: image.width,
            height: image.height,
            kind,
        })
        .collect()
}

fn largest_image<'a>(images: impl Iterator<Item = &'a CatalogImage>) -> Option<&'a CatalogImage> {
    images.max_by_key(|image| {
        (
            u64::from(image.width) * u64::from(image.height),
            image.width,
            image.height,
        )
    })
}

fn episode_label(episode: &Episode) -> Option<String> {
    non_empty(&episode.episode)
        .or_else(|| episode.episode_number.map(|number| number.to_string()))
        .or_else(|| {
            (episode.sequence_number > 0.0).then(|| {
                episode
                    .sequence_number
                    .to_string()
                    .trim_end_matches(".0")
                    .to_string()
            })
        })
}

fn unique_locales(locales: impl IntoIterator<Item = Locale>) -> Vec<Locale> {
    let mut locales = locales
        .into_iter()
        .filter(|locale| !locale.to_string().is_empty())
        .collect::<Vec<_>>();
    locales.sort_by_key(ToString::to_string);
    locales.dedup();
    locales
}

fn duration_millis(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn vote_count(value: &serde_json::Value) -> Option<f64> {
    let displayed = value.get("displayed")?.as_str()?.parse::<f64>().ok()?;
    let multiplier = match value.get("unit").and_then(serde_json::Value::as_str) {
        None | Some("") => 1.0,
        Some("K") | Some("k") => 1_000.0,
        Some("M") | Some("m") => 1_000_000.0,
        Some(_) => return None,
    };
    Some(displayed * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_catalog_item_keeps_artwork_and_download_target() {
        let mut episode: Episode = serde_json::from_str(
            r#"{
                "id":"EP1","title":"One","description":"Description",
                "duration_ms":120000,"audio_locale":"ja-JP",
                "subtitle_locales":["en-US"],"is_subbed":true,
                "images":{"thumbnail":[[{"source":"https://img.test/one.jpg","width":640,"height":360}]]}
            }"#,
        )
        .unwrap();
        episode.versions.clear();
        let item = CatalogItem::from_episode(&episode);
        assert_eq!(item.target, Some(MediaTarget::Episode("EP1".into())));
        assert_eq!(item.duration_millis, Some(120_000));
        assert_eq!(item.images[0].kind, CatalogImageKind::Thumbnail);
    }

    #[test]
    fn series_catalog_item_keeps_hierarchy_and_poster_shapes() {
        let series: Series = serde_json::from_str(
            r#"{
                "id":"S1","title":"Series","description":"Short",
                "extended_description":"Long","series_launch_year":2024,
                "episode_count":12,"season_count":1,"is_subbed":true,
                "audio_locales":["ja-JP"],"subtitle_locales":["en-US"],
                "images":{"poster_tall":[{"source":"https://img.test/tall.jpg","width":400,"height":600}],"poster_wide":[{"source":"https://img.test/wide.jpg","width":800,"height":450}]}
            }"#,
        )
        .unwrap();
        let item = CatalogItem::from_series(&series);
        assert_eq!(item.season_count, Some(1));
        assert_eq!(item.episode_count, Some(12));
        assert_eq!(item.images[0].kind, CatalogImageKind::PosterTall);
        assert_eq!(item.images[1].kind, CatalogImageKind::PosterWide);
    }

    #[test]
    fn best_artwork_prefers_largest_suitable_rendition() {
        let mut item = CatalogItem {
            id: "S1".into(),
            kind: CatalogKind::Series,
            target: None,
            title: "Series".into(),
            description: String::new(),
            extended_description: None,
            images: vec![
                CatalogImage {
                    source: "tall-large".into(),
                    width: 1200,
                    height: 1800,
                    kind: CatalogImageKind::PosterTall,
                },
                CatalogImage {
                    source: "wide-small".into(),
                    width: 320,
                    height: 180,
                    kind: CatalogImageKind::PosterWide,
                },
                CatalogImage {
                    source: "wide-large".into(),
                    width: 1280,
                    height: 720,
                    kind: CatalogImageKind::PosterWide,
                },
            ],
            rating: None,
            release_year: None,
            season_number: None,
            episode_number: None,
            season_count: None,
            episode_count: None,
            duration_millis: None,
            premium_only: false,
            is_subbed: false,
            is_dubbed: false,
            audio_locales: Vec::new(),
            subtitle_locales: Vec::new(),
        };
        assert_eq!(
            item.best_artwork().map(|image| image.source.as_str()),
            Some("wide-large")
        );
        item.kind = CatalogKind::Episode;
        item.images = vec![
            CatalogImage {
                source: "thumbnail-small".into(),
                width: 320,
                height: 180,
                kind: CatalogImageKind::Thumbnail,
            },
            CatalogImage {
                source: "thumbnail-large".into(),
                width: 1920,
                height: 1080,
                kind: CatalogImageKind::Thumbnail,
            },
        ];
        assert_eq!(
            item.best_artwork().map(|image| image.source.as_str()),
            Some("thumbnail-large")
        );
    }

    #[test]
    fn episode_vote_rating_becomes_an_approval_percentage() {
        let rating: EpisodeRating = serde_json::from_str(
            r#"{"up":{"displayed":"1.2","unit":"K"},"down":{"displayed":"50","unit":""},"total":1250}"#,
        )
        .unwrap();
        assert_eq!(
            CatalogRating::from_episode_rating(&rating),
            Some(CatalogRating::Approval {
                percentage: 96.0,
                total: Some(1250),
            })
        );
    }
}
