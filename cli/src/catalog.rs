//! Authenticated catalog operations shared by headless and TUI modes.

use crunchyroll_rs::Crunchyroll;
use crunchyroll_rs::media::Media;
use crunchyroll_rs::search::SearchMediaCollection;
use futures_util::{StreamExt, TryStreamExt};

use crate::error::{Error, Result};

pub(crate) async fn search(
    client: &Crunchyroll,
    query: &str,
    limit: usize,
) -> Result<Vec<crunchydl::CatalogItem>> {
    let results: Vec<SearchMediaCollection> = client
        .query(query)
        .top_results
        .take(limit)
        .try_collect()
        .await
        .map_err(Error::from)?;
    Ok(results
        .iter()
        .map(crunchydl::CatalogItem::from_search)
        .collect())
}

pub(crate) async fn children(
    client: &Crunchyroll,
    item: &crunchydl::CatalogItem,
) -> Result<Vec<crunchydl::CatalogItem>> {
    browse(client, item.kind, &item.id).await
}

pub(crate) async fn browse(
    client: &Crunchyroll,
    kind: crunchydl::CatalogKind,
    id: &str,
) -> Result<Vec<crunchydl::CatalogItem>> {
    match kind {
        crunchydl::CatalogKind::Series => {
            let series = crunchyroll_rs::Series::from_id(client, id)
                .await
                .map_err(Error::from)?;
            let (seasons, rating) = tokio::join!(series.seasons(), series.rating());
            let rating = rating
                .ok()
                .map(|rating| crunchydl::CatalogRating::from_star_rating(&rating));
            Ok(seasons
                .map_err(Error::from)?
                .iter()
                .map(|season| {
                    let mut item = crunchydl::CatalogItem::from_season(season);
                    item.rating.clone_from(&rating);
                    item
                })
                .collect())
        }
        crunchydl::CatalogKind::Season => {
            let season = crunchyroll_rs::Season::from_id(client, id)
                .await
                .map_err(Error::from)?;
            let episodes = season.episodes().await.map_err(Error::from)?;
            Ok(futures_util::stream::iter(episodes)
                .map(|episode| async move {
                    let rating = episode
                        .rating()
                        .await
                        .ok()
                        .as_ref()
                        .and_then(crunchydl::CatalogRating::from_episode_rating);
                    let mut item = crunchydl::CatalogItem::from_episode(&episode);
                    item.rating = rating;
                    item
                })
                .buffered(8)
                .collect()
                .await)
        }
        crunchydl::CatalogKind::MovieListing => {
            let listing = crunchyroll_rs::MovieListing::from_id(client, id)
                .await
                .map_err(Error::from)?;
            Ok(listing
                .movies()
                .await
                .map_err(Error::from)?
                .iter()
                .map(crunchydl::CatalogItem::from_movie)
                .collect())
        }
        _ => Ok(Vec::new()),
    }
}
