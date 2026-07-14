//! Resolution helpers for stable media requests and collections.

use super::*;
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
