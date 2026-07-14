//! Rating conversion helpers for catalog view models.

use super::*;

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
