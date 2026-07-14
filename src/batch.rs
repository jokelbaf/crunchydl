//! Stable collection targets and logical episode canonicalization.

use std::collections::HashMap;

use crunchyroll_rs::Locale;

use crate::{MediaKind, MediaTarget, ResolvedMedia, ResolvedVersion};

/// Stable identity of a catalog collection that expands into download targets.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub enum CollectionTarget {
    /// Every logical episode in one season.
    Season(String),
    /// Every logical episode across every season in one series.
    Series(String),
    /// Every movie in a movie listing.
    MovieListing(String),
}

/// Options applied while expanding a collection.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct BatchOptions {
    /// Include specials whose integral episode number is absent.
    pub include_specials: bool,
    /// Restrict a series to these displayed season numbers when nonempty.
    pub season_numbers: Vec<u32>,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self {
            include_specials: true,
            season_numbers: Vec::new(),
        }
    }
}

/// Invalid input supplied to episode batch canonicalization.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BatchError {
    /// A non-episode was supplied to an episode-only batch.
    #[error("episode batch contains non-episode media: {0:?}")]
    NonEpisode(MediaKind),
}

/// Merge localized duplicate episode records into logical episodes.
///
/// Entries with the same service identifier are treated as one episode. When
/// identifiers are absent, series id, logical season position, episode label,
/// and sequence number form the fallback key. Audio versions and subtitle
/// locales are unioned deterministically.
///
/// # Errors
///
/// Returns [`BatchError::NonEpisode`] if the input contains another media kind.
pub fn canonicalize_episode_batch(
    episodes: impl IntoIterator<Item = ResolvedMedia>,
) -> Result<Vec<ResolvedMedia>, BatchError> {
    let mut canonical = Vec::<ResolvedMedia>::new();
    let mut positions = HashMap::<LogicalEpisodeKey, usize>::new();

    for episode in episodes {
        if episode.kind != MediaKind::Episode {
            return Err(BatchError::NonEpisode(episode.kind));
        }
        let key = LogicalEpisodeKey::from_media(&episode);
        if let Some(index) = positions.get(&key).copied() {
            merge_episode(&mut canonical[index], episode);
        } else {
            positions.insert(key, canonical.len());
            canonical.push(episode);
        }
    }

    for episode in &mut canonical {
        normalize_locales(&mut episode.subtitle_locales);
        normalize_versions(&mut episode.versions);
    }
    canonical.sort_by(|left, right| {
        left.series_id
            .cmp(&right.series_id)
            .then_with(|| {
                compare_optional_f32(left.season_sequence_number, right.season_sequence_number)
            })
            .then_with(|| left.season_number.cmp(&right.season_number))
            .then_with(|| left.sequence_number.total_cmp(&right.sequence_number))
            .then_with(|| left.content_id.cmp(&right.content_id))
    });
    Ok(canonical)
}

/// Apply frontend batch filters and return durable media targets.
#[must_use]
pub fn select_batch_targets(
    episodes: &[ResolvedMedia],
    options: &BatchOptions,
) -> Vec<MediaTarget> {
    episodes
        .iter()
        .filter(|episode| options.include_specials || !episode.is_special)
        .filter(|episode| {
            options.season_numbers.is_empty()
                || episode
                    .season_number
                    .is_some_and(|number| options.season_numbers.contains(&number))
        })
        .map(MediaTarget::from)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum LogicalEpisodeKey {
    Identifier(String),
    Position {
        series: String,
        season: Option<u32>,
        episode: String,
        sequence: u32,
    },
}

impl LogicalEpisodeKey {
    fn from_media(media: &ResolvedMedia) -> Self {
        if let Some(identifier) = media.identifier.as_ref().filter(|value| !value.is_empty()) {
            return Self::Identifier(identifier.clone());
        }
        Self::Position {
            series: media.series_id.clone().unwrap_or_default(),
            season: media.season_sequence_number.map(f32::to_bits),
            episode: media.episode.clone().unwrap_or_default(),
            sequence: media.sequence_number.to_bits(),
        }
    }
}

fn merge_episode(existing: &mut ResolvedMedia, incoming: ResolvedMedia) {
    existing.subtitle_locales.extend(incoming.subtitle_locales);
    existing.versions.extend(incoming.versions);
    existing.is_premium_only &= incoming.is_premium_only;
    if existing.availability_status != "available" && incoming.availability_status == "available" {
        existing.availability_status = incoming.availability_status;
    }
}

fn normalize_locales(locales: &mut Vec<Locale>) {
    locales.sort_by_key(ToString::to_string);
    locales.dedup();
}

fn normalize_versions(versions: &mut Vec<ResolvedVersion>) {
    versions.sort_by(|left, right| {
        right
            .original
            .cmp(&left.original)
            .then_with(|| locale_key(&left.audio_locale).cmp(&locale_key(&right.audio_locale)))
            .then_with(|| left.content_id.cmp(&right.content_id))
    });
    versions.dedup_by(|left, right| left.content_id == right.content_id);
}

fn locale_key(locale: &Option<Locale>) -> String {
    locale.as_ref().map(ToString::to_string).unwrap_or_default()
}

fn compare_optional_f32(left: Option<f32>, right: Option<f32>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn episode(id: &str, identifier: &str, locale: Locale, special: bool) -> ResolvedMedia {
        ResolvedMedia {
            kind: MediaKind::Episode,
            content_id: id.into(),
            series_id: Some("SERIES".into()),
            season_id: Some("SEASON".into()),
            series_title: Some("Series".into()),
            season_title: Some("Season 1".into()),
            identifier: Some(identifier.into()),
            season_number: Some(1),
            season_sequence_number: Some(1.0),
            title: "Episode".into(),
            episode: (!special).then(|| "1".into()),
            episode_number: (!special).then_some(1),
            sequence_number: if special { 1.5 } else { 1.0 },
            is_special: special,
            duration: Duration::from_secs(1),
            audio_locale: Some(locale.clone()),
            subtitle_locales: vec![Locale::en_US],
            is_premium_only: false,
            availability_status: "available".into(),
            versions: vec![ResolvedVersion {
                content_id: id.into(),
                audio_locale: Some(locale.clone()),
                original: locale == Locale::ja_JP,
                is_premium_only: false,
                roles: Vec::new(),
            }],
        }
    }

    #[test]
    fn localized_records_merge_into_one_logical_episode() {
        let episodes = canonicalize_episode_batch([
            episode("EP-JA", "SERIES|S1|E1", Locale::ja_JP, false),
            episode("EP-EN", "SERIES|S1|E1", Locale::en_US, false),
        ])
        .unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].versions.len(), 2);
        assert!(episodes[0].versions[0].original);
    }

    #[test]
    fn filters_seasons_and_specials_before_queueing() {
        let episodes = vec![
            episode("EP1", "SERIES|S1|E1", Locale::ja_JP, false),
            episode("SP1", "SERIES|S1|SP1", Locale::ja_JP, true),
        ];
        let targets = select_batch_targets(
            &episodes,
            &BatchOptions {
                include_specials: false,
                season_numbers: vec![1],
            },
        );
        assert_eq!(targets, vec![MediaTarget::Episode("EP1".into())]);
    }
}
