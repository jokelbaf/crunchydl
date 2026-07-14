//! Track selection policies and the logic that resolves them against available
//! options.
//!
//! Selection never silently degrades. When a configured [`FallbackPolicy`] causes
//! a deviation from the exact request, the resolver records a
//! [`DownloadWarning`]; when no fallback is permitted, it returns a
//! [`SelectionError`] that lists what was available.

use crunchyroll_rs::Locale;

use crate::event::DownloadWarning;
use crate::model::{ResolvedMedia, ResolvedVersion};

mod quality;
mod subtitles;
mod types;
pub use quality::{
    AudioQualityCandidate, QualityChoice, VideoQualityCandidate, select_audio_quality,
    select_video_quality,
};
pub use subtitles::{
    SubtitlePlan, SubtitleTrackInfo, select_cdn, select_hardsub, select_subtitles,
};
pub use types::*;

/// The resolved set of audio versions plus any fallback warnings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioPlan {
    /// The selected versions, in output order.
    pub versions: Vec<ResolvedVersion>,
    /// Warnings recorded while resolving the selection.
    pub warnings: Vec<DownloadWarning>,
}

/// Resolve an [`AudioSelection`] against a [`ResolvedMedia`].
///
/// Movies and music videos have a single version whose locale is unknown from
/// metadata; their only version is always returned regardless of `selection`.
///
/// # Errors
///
/// Returns [`SelectionError`] when the media has no versions, or when an exact
/// locale request cannot be met under [`FallbackPolicy::Exact`].
pub fn select_audio(
    media: &ResolvedMedia,
    selection: &AudioSelection,
    fallback: FallbackPolicy,
) -> Result<AudioPlan, SelectionError> {
    if media.versions.is_empty() {
        return Err(SelectionError::NoAudioVersions);
    }

    if media.versions.iter().all(|v| v.audio_locale.is_none()) {
        return Ok(AudioPlan {
            versions: media.versions.clone(),
            warnings: Vec::new(),
        });
    }

    match selection {
        AudioSelection::Original => Ok(AudioPlan {
            versions: vec![pick_original(media)],
            warnings: Vec::new(),
        }),
        AudioSelection::All => Ok(AudioPlan {
            versions: sorted_all(media),
            warnings: Vec::new(),
        }),
        AudioSelection::Locale(locale) => select_single_locale(media, locale, fallback),
        AudioSelection::Locales(locales) => select_locale_list(media, locales, fallback),
    }
}

fn version_locale_matches(version: &ResolvedVersion, locale: &Locale) -> bool {
    version.audio_locale.as_ref() == Some(locale)
}

fn available_locales(media: &ResolvedMedia) -> Vec<Locale> {
    media
        .versions
        .iter()
        .filter_map(|v| v.audio_locale.clone())
        .collect()
}

fn pick_original(media: &ResolvedMedia) -> ResolvedVersion {
    if let Some(original) = media.versions.iter().find(|v| v.original) {
        return original.clone();
    }
    if let Some(matching) = media
        .versions
        .iter()
        .find(|v| v.audio_locale == media.audio_locale)
    {
        return matching.clone();
    }
    media.versions[0].clone()
}

fn sorted_all(media: &ResolvedMedia) -> Vec<ResolvedVersion> {
    let mut versions = media.versions.clone();
    versions.sort_by(|a, b| {
        b.original
            .cmp(&a.original)
            .then_with(|| a.content_id.cmp(&b.content_id))
    });
    versions
}

fn select_single_locale(
    media: &ResolvedMedia,
    locale: &Locale,
    fallback: FallbackPolicy,
) -> Result<AudioPlan, SelectionError> {
    if let Some(version) = media
        .versions
        .iter()
        .find(|v| version_locale_matches(v, locale))
    {
        return Ok(AudioPlan {
            versions: vec![version.clone()],
            warnings: Vec::new(),
        });
    }

    if fallback == FallbackPolicy::Exact {
        return Err(SelectionError::AudioLocaleUnavailable {
            requested: locale.clone(),
            available: available_locales(media),
        });
    }

    let used = pick_original(media);
    let warning = DownloadWarning::AudioFallback {
        requested: locale.clone(),
        used: used.audio_locale.clone().unwrap_or_else(|| locale.clone()),
    };
    Ok(AudioPlan {
        versions: vec![used],
        warnings: vec![warning],
    })
}

fn select_locale_list(
    media: &ResolvedMedia,
    locales: &[Locale],
    fallback: FallbackPolicy,
) -> Result<AudioPlan, SelectionError> {
    let mut versions = Vec::new();
    let mut warnings = Vec::new();
    for locale in locales {
        if let Some(version) = media
            .versions
            .iter()
            .find(|v| version_locale_matches(v, locale))
        {
            if !versions
                .iter()
                .any(|v: &ResolvedVersion| v.content_id == version.content_id)
            {
                versions.push(version.clone());
            }
        } else if fallback == FallbackPolicy::Exact {
            return Err(SelectionError::AudioLocaleUnavailable {
                requested: locale.clone(),
                available: available_locales(media),
            });
        } else {
            warnings.push(DownloadWarning::AudioLocaleUnavailable {
                requested: locale.clone(),
            });
        }
    }
    Ok(AudioPlan { versions, warnings })
}
