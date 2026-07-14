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

mod ratings;

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

mod conversions;

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
