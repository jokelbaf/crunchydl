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

/// How to behave when an exact selection cannot be satisfied.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FallbackPolicy {
    /// Fail if the exact request cannot be met.
    #[default]
    Exact,
    /// Prefer the nearest option that is not higher than requested.
    NearestLower,
    /// Use the best available option.
    BestAvailable,
}

/// Which audio versions to download.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AudioSelection {
    /// The original-language version.
    #[default]
    Original,
    /// Exactly one locale.
    Locale(Locale),
    /// An ordered list of locales.
    Locales(Vec<Locale>),
    /// Every available version.
    All,
}

/// Whether a single video is reused across dubs or one is kept per version.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoSelection {
    /// Download one video and reuse it across all selected audio versions.
    #[default]
    SharedAcrossDubs,
    /// Keep one video per selected audio version.
    PerVersion,
}

/// Whether to request a hardsubbed video and, if so, in which locale.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HardSubSelection {
    /// Request raw (non-hardsubbed) video.
    #[default]
    None,
    /// Request video with the given locale burned in.
    Locale(Locale),
}

/// Which subtitle and caption tracks to download.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubtitleSelection {
    /// Which locales to include.
    pub locales: SubtitleLocales,
    /// Include closed-caption tracks in addition to subtitle tracks.
    pub include_captions: bool,
    /// Retain signs tracks (a non-caption track matching the audio locale).
    pub include_signs: bool,
    /// Apply normalization transforms instead of retaining the raw source.
    pub normalize: bool,
}

impl Default for SubtitleSelection {
    fn default() -> Self {
        Self {
            locales: SubtitleLocales::All,
            include_captions: true,
            include_signs: true,
            normalize: false,
        }
    }
}

impl SubtitleSelection {
    /// Replace the selected subtitle locale set while retaining the remaining
    /// caption, signs, and normalization policy.
    #[must_use]
    pub fn with_locales(mut self, locales: SubtitleLocales) -> Self {
        self.locales = locales;
        self
    }
}

/// The locale set of a [`SubtitleSelection`].
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SubtitleLocales {
    /// No subtitle tracks.
    None,
    /// Every available locale.
    #[default]
    All,
    /// An explicit, ordered list of locales.
    Explicit(Vec<Locale>),
}

/// A video or audio quality policy.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QualitySelection {
    /// The best available quality.
    #[default]
    Best,
    /// Exactly the given video height in pixels.
    ExactHeight(u32),
    /// The best quality no taller than the given video height in pixels.
    MaxHeight(u32),
    /// Exactly the given bandwidth in bits per second.
    ExactBandwidth(u64),
    /// The representation at the given rank after deterministic sorting.
    RankedIndex(usize),
}

/// Which CDN to use when more than one is offered.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CdnSelection {
    /// Choose automatically (the first CDN after deterministic sorting).
    #[default]
    Automatic,
    /// A stable index after deterministic sorting.
    Index(usize),
}

/// Where chapter markers come from.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ChapterSelection {
    /// No chapters.
    Disabled,
    /// Build chapters from Crunchyroll skip events.
    #[default]
    SkipEvents,
}

/// The output container format.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Container {
    /// Matroska (`.mkv`).
    #[default]
    Matroska,
    /// ISO base media file format (`.mp4`) with AVC/AAC tracks only.
    Mp4,
}

/// What to do when the output path already exists.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverwritePolicy {
    /// Fail if the output already exists.
    #[default]
    Fail,
    /// Replace the existing output.
    Replace,
    /// Resume a previous download into the existing output.
    Resume,
}

/// A structured reason a selection could not be satisfied.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SelectionError {
    /// The media had no audio versions at all.
    #[error("no audio versions available")]
    NoAudioVersions,
    /// A requested audio locale was unavailable and no fallback was permitted.
    #[error("audio locale {requested} unavailable (available: {})", format_locales(.available))]
    AudioLocaleUnavailable {
        /// The requested locale.
        requested: Locale,
        /// The locales that were available.
        available: Vec<Locale>,
    },
    /// A requested hardsub locale was unavailable.
    #[error("hardsub locale {requested} unavailable (available: {})", format_locales(.available))]
    HardSubUnavailable {
        /// The requested locale.
        requested: Locale,
        /// The hardsub locales that were available.
        available: Vec<Locale>,
    },
    /// There were no video representations to choose from.
    #[error("no video representations available")]
    NoVideoRepresentations,
    /// There were no audio representations to choose from.
    #[error("no audio representations available")]
    NoAudioRepresentations,
    /// A requested quality was unavailable and no fallback was permitted.
    #[error("quality {requested} unavailable")]
    QualityUnavailable {
        /// A human-readable description of the requested quality.
        requested: String,
    },
    /// A stable index was out of range for the available options.
    #[error("index {index} out of range ({available} available)")]
    IndexOutOfRange {
        /// The requested index.
        index: usize,
        /// The number of available options.
        available: usize,
    },
}

fn format_locales(locales: &[Locale]) -> String {
    locales
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

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

    // Media without per-version locale metadata (movies, music videos) has a
    // single audio track; return it and let stream-time metadata refine it.
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
    // Original first (when the request does not specify), then stable by content
    // id for determinism.
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

    // For a single-locale request we substitute the original rather than
    // returning zero audio tracks, and record the deviation.
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

/// A neutral video representation for quality selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoQualityCandidate {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bandwidth in bits per second.
    pub bandwidth: u64,
}

/// A neutral audio representation for quality selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioQualityCandidate {
    /// Bandwidth in bits per second.
    pub bandwidth: u64,
    /// Sampling rate in Hz.
    pub sampling_rate: u32,
}

/// The outcome of a quality selection: an index into the candidate slice plus an
/// optional fallback warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityChoice {
    /// The index of the chosen candidate.
    pub index: usize,
    /// A warning if a fallback quality was chosen.
    pub warning: Option<DownloadWarning>,
}

// Quality keys are ordered so that a numerically greater key is better quality.
fn video_quality_key(candidate: &VideoQualityCandidate) -> (u32, u64, u32) {
    (candidate.height, candidate.bandwidth, candidate.width)
}

fn audio_quality_key(candidate: &AudioQualityCandidate) -> (u64, u32) {
    (candidate.bandwidth, candidate.sampling_rate)
}

fn video_describe(candidate: &VideoQualityCandidate) -> String {
    format!("{}p", candidate.height)
}

fn audio_describe(candidate: &AudioQualityCandidate) -> String {
    format!("{} bps", candidate.bandwidth)
}

/// Return the index of the best candidate matching `pred`, or [`None`] if none
/// match. Ties resolve to the first candidate.
fn best_matching<T, K: Ord>(
    items: &[T],
    key: impl Fn(&T) -> K,
    pred: impl Fn(&T) -> bool,
) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (index, item) in items.iter().enumerate() {
        if !pred(item) {
            continue;
        }
        match best {
            Some(current) if key(&items[current]) >= key(item) => {}
            _ => best = Some(index),
        }
    }
    best
}

/// Select a video representation by [`QualitySelection`].
///
/// Video is ranked by height, then bandwidth, then width. `ExactHeight`,
/// `ExactBandwidth`, and `RankedIndex` are subject to `fallback`; `Best` and
/// `MaxHeight` always succeed when at least one candidate exists.
///
/// # Errors
///
/// Returns [`SelectionError`] when there are no candidates, or when an exact
/// request cannot be met under [`FallbackPolicy::Exact`].
pub fn select_video_quality(
    candidates: &[VideoQualityCandidate],
    selection: &QualitySelection,
    fallback: FallbackPolicy,
) -> Result<QualityChoice, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoVideoRepresentations);
    }

    match *selection {
        QualitySelection::Best => Ok(QualityChoice {
            index: argmax(candidates, video_quality_key),
            warning: None,
        }),
        QualitySelection::MaxHeight(max) => {
            if let Some(index) = best_matching(candidates, video_quality_key, |c| c.height <= max) {
                Ok(QualityChoice {
                    index,
                    warning: None,
                })
            } else {
                // Nothing satisfies the ceiling; fall back to the lowest
                // available and warn.
                let index = argmin(candidates, video_quality_key);
                Ok(QualityChoice {
                    index,
                    warning: Some(DownloadWarning::VideoQualityFallback {
                        requested: format!("<={max}p"),
                        used: video_describe(&candidates[index]),
                    }),
                })
            }
        }
        QualitySelection::ExactHeight(height) => resolve_video_target(
            candidates,
            |c| u64::from(c.height),
            u64::from(height),
            format!("{height}p"),
            fallback,
        ),
        QualitySelection::ExactBandwidth(bandwidth) => resolve_video_target(
            candidates,
            |c| c.bandwidth,
            bandwidth,
            format!("{bandwidth} bps"),
            fallback,
        ),
        QualitySelection::RankedIndex(rank) => {
            let mut order: Vec<usize> = (0..candidates.len()).collect();
            // Rank 0 is the best representation.
            order.sort_by(|&a, &b| {
                video_quality_key(&candidates[b]).cmp(&video_quality_key(&candidates[a]))
            });
            match order.get(rank) {
                Some(&index) => Ok(QualityChoice {
                    index,
                    warning: None,
                }),
                None if fallback == FallbackPolicy::Exact => Err(SelectionError::IndexOutOfRange {
                    index: rank,
                    available: candidates.len(),
                }),
                None => {
                    let index = order[order.len() - 1];
                    Ok(QualityChoice {
                        index,
                        warning: Some(DownloadWarning::VideoQualityFallback {
                            requested: format!("rank {rank}"),
                            used: video_describe(&candidates[index]),
                        }),
                    })
                }
            }
        }
    }
}

fn resolve_video_target(
    candidates: &[VideoQualityCandidate],
    value: impl Fn(&VideoQualityCandidate) -> u64,
    target: u64,
    requested: String,
    fallback: FallbackPolicy,
) -> Result<QualityChoice, SelectionError> {
    if let Some(index) = best_matching(candidates, video_quality_key, |c| value(c) == target) {
        return Ok(QualityChoice {
            index,
            warning: None,
        });
    }

    let index = match fallback {
        FallbackPolicy::Exact => return Err(SelectionError::QualityUnavailable { requested }),
        FallbackPolicy::NearestLower => {
            best_matching(candidates, video_quality_key, |c| value(c) < target)
                .unwrap_or_else(|| argmin(candidates, video_quality_key))
        }
        FallbackPolicy::BestAvailable => argmax(candidates, video_quality_key),
    };
    Ok(QualityChoice {
        index,
        warning: Some(DownloadWarning::VideoQualityFallback {
            requested,
            used: video_describe(&candidates[index]),
        }),
    })
}

/// Select an audio representation by [`QualitySelection`].
///
/// Audio is ranked by bandwidth, then sampling rate. Height-based policies map to
/// the best available bandwidth.
///
/// # Errors
///
/// Returns [`SelectionError`] when there are no candidates, or when an exact
/// bandwidth request cannot be met under [`FallbackPolicy::Exact`].
pub fn select_audio_quality(
    candidates: &[AudioQualityCandidate],
    selection: &QualitySelection,
    fallback: FallbackPolicy,
) -> Result<QualityChoice, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoAudioRepresentations);
    }

    match *selection {
        QualitySelection::ExactBandwidth(bandwidth) => {
            if let Some(index) =
                best_matching(candidates, audio_quality_key, |c| c.bandwidth == bandwidth)
            {
                return Ok(QualityChoice {
                    index,
                    warning: None,
                });
            }
            let index = match fallback {
                FallbackPolicy::Exact => {
                    return Err(SelectionError::QualityUnavailable {
                        requested: format!("{bandwidth} bps"),
                    });
                }
                FallbackPolicy::NearestLower => {
                    best_matching(candidates, audio_quality_key, |c| c.bandwidth < bandwidth)
                        .unwrap_or_else(|| argmin(candidates, audio_quality_key))
                }
                FallbackPolicy::BestAvailable => argmax(candidates, audio_quality_key),
            };
            Ok(QualityChoice {
                index,
                warning: Some(DownloadWarning::AudioQualityFallback {
                    requested: format!("{bandwidth} bps"),
                    used: audio_describe(&candidates[index]),
                }),
            })
        }
        QualitySelection::RankedIndex(rank) => {
            let mut order: Vec<usize> = (0..candidates.len()).collect();
            order.sort_by(|&a, &b| {
                audio_quality_key(&candidates[b]).cmp(&audio_quality_key(&candidates[a]))
            });
            match order.get(rank) {
                Some(&index) => Ok(QualityChoice {
                    index,
                    warning: None,
                }),
                None if fallback == FallbackPolicy::Exact => Err(SelectionError::IndexOutOfRange {
                    index: rank,
                    available: candidates.len(),
                }),
                None => {
                    let index = order[order.len() - 1];
                    Ok(QualityChoice {
                        index,
                        warning: Some(DownloadWarning::AudioQualityFallback {
                            requested: format!("rank {rank}"),
                            used: audio_describe(&candidates[index]),
                        }),
                    })
                }
            }
        }
        // Best / ExactHeight / MaxHeight: audio has no height, so pick the best
        // bandwidth.
        _ => Ok(QualityChoice {
            index: argmax(candidates, audio_quality_key),
            warning: None,
        }),
    }
}

/// Select a hardsub locale from the locales a stream offers.
///
/// # Errors
///
/// Returns [`SelectionError::HardSubUnavailable`] when a specific locale was
/// requested but not offered. A hardsub is never silently dropped.
pub fn select_hardsub(
    available: &[Locale],
    selection: &HardSubSelection,
) -> Result<Option<Locale>, SelectionError> {
    match selection {
        HardSubSelection::None => Ok(None),
        HardSubSelection::Locale(locale) => {
            if available.contains(locale) {
                Ok(Some(locale.clone()))
            } else {
                Err(SelectionError::HardSubUnavailable {
                    requested: locale.clone(),
                    available: available.to_vec(),
                })
            }
        }
    }
}

/// Select a CDN index from the number of options after deterministic sorting.
///
/// # Errors
///
/// Returns [`SelectionError::IndexOutOfRange`] when an explicit index exceeds the
/// number of available CDNs.
pub fn select_cdn(available: usize, selection: &CdnSelection) -> Result<usize, SelectionError> {
    match *selection {
        CdnSelection::Automatic if available > 0 => Ok(0),
        CdnSelection::Automatic => Err(SelectionError::IndexOutOfRange {
            index: 0,
            available,
        }),
        CdnSelection::Index(index) if index < available => Ok(index),
        CdnSelection::Index(index) => Err(SelectionError::IndexOutOfRange { index, available }),
    }
}

/// A neutral subtitle track for subtitle selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleTrackInfo {
    /// The subtitle locale.
    pub locale: Locale,
    /// Whether this is a closed-caption track rather than a translation.
    pub is_caption: bool,
    /// The subtitle format (for example `ass` or `vtt`).
    pub format: String,
}

/// The resolved set of subtitle tracks plus any warnings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubtitlePlan {
    /// Indices into the available-track slice, in output order.
    pub tracks: Vec<usize>,
    /// Warnings recorded while resolving the selection.
    pub warnings: Vec<DownloadWarning>,
}

/// Resolve a [`SubtitleSelection`] against the available subtitle tracks.
///
/// Signs handling (marking a non-caption track that matches the audio locale) is
/// applied later during planning; this resolver selects by locale and caption
/// flag only.
#[must_use]
pub fn select_subtitles(
    available: &[SubtitleTrackInfo],
    selection: &SubtitleSelection,
) -> SubtitlePlan {
    match &selection.locales {
        SubtitleLocales::None => SubtitlePlan::default(),
        SubtitleLocales::All => {
            let tracks = available
                .iter()
                .enumerate()
                .filter(|(_, track)| selection.include_captions || !track.is_caption)
                .map(|(index, _)| index)
                .collect();
            SubtitlePlan {
                tracks,
                warnings: Vec::new(),
            }
        }
        SubtitleLocales::Explicit(locales) => {
            let mut tracks = Vec::new();
            let mut warnings = Vec::new();
            for locale in locales {
                let matched: Vec<usize> = available
                    .iter()
                    .enumerate()
                    .filter(|(_, track)| {
                        &track.locale == locale && (selection.include_captions || !track.is_caption)
                    })
                    .map(|(index, _)| index)
                    .collect();
                if matched.is_empty() {
                    warnings.push(DownloadWarning::SubtitleUnavailable {
                        requested: locale.clone(),
                    });
                } else {
                    tracks.extend(matched);
                }
            }
            SubtitlePlan { tracks, warnings }
        }
    }
}

fn argmax<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> usize {
    // `max_by_key` returns the last maximal element; iterate manually so ties
    // resolve to the first candidate deterministically.
    let mut best = 0;
    let mut best_key = key(&items[0]);
    for (index, item) in items.iter().enumerate().skip(1) {
        let candidate_key = key(item);
        if candidate_key > best_key {
            best = index;
            best_key = candidate_key;
        }
    }
    best
}

fn argmin<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> usize {
    let mut best = 0;
    let mut best_key = key(&items[0]);
    for (index, item) in items.iter().enumerate().skip(1) {
        let candidate_key = key(item);
        if candidate_key < best_key {
            best = index;
            best_key = candidate_key;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::model::MediaKind;

    use super::*;

    fn version(locale: Option<Locale>, original: bool) -> ResolvedVersion {
        ResolvedVersion {
            content_id: locale
                .as_ref()
                .map_or_else(|| "SINGLE".to_string(), ToString::to_string),
            audio_locale: locale,
            original,
            is_premium_only: false,
            roles: Vec::new(),
        }
    }

    fn media(versions: Vec<ResolvedVersion>) -> ResolvedMedia {
        ResolvedMedia {
            kind: MediaKind::Episode,
            content_id: "M".to_string(),
            series_id: None,
            season_id: None,
            series_title: None,
            season_title: None,
            identifier: None,
            season_number: None,
            season_sequence_number: None,
            title: "T".to_string(),
            episode: None,
            episode_number: None,
            sequence_number: 0.0,
            is_special: false,
            duration: Duration::ZERO,
            audio_locale: Some(Locale::ja_JP),
            subtitle_locales: Vec::new(),
            is_premium_only: false,
            availability_status: "available".to_string(),
            versions,
        }
    }

    fn dubbed() -> ResolvedMedia {
        media(vec![
            version(Some(Locale::ja_JP), true),
            version(Some(Locale::en_US), false),
            version(Some(Locale::de_DE), false),
        ])
    }

    fn locales(plan: &AudioPlan) -> Vec<Locale> {
        plan.versions
            .iter()
            .filter_map(|v| v.audio_locale.clone())
            .collect()
    }

    #[test]
    fn audio_original_picks_marked_version() {
        let plan = select_audio(&dubbed(), &AudioSelection::Original, FallbackPolicy::Exact)
            .expect("original resolves");
        assert_eq!(locales(&plan), vec![Locale::ja_JP]);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn audio_exact_locale_found() {
        let plan = select_audio(
            &dubbed(),
            &AudioSelection::Locale(Locale::en_US),
            FallbackPolicy::Exact,
        )
        .expect("locale resolves");
        assert_eq!(locales(&plan), vec![Locale::en_US]);
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn audio_exact_locale_missing_errors() {
        let err = select_audio(
            &dubbed(),
            &AudioSelection::Locale(Locale::fr_FR),
            FallbackPolicy::Exact,
        )
        .expect_err("missing locale errors");
        assert!(matches!(
            err,
            SelectionError::AudioLocaleUnavailable { requested, .. } if requested == Locale::fr_FR
        ));
    }

    #[test]
    fn audio_missing_locale_falls_back_to_original_with_warning() {
        let plan = select_audio(
            &dubbed(),
            &AudioSelection::Locale(Locale::fr_FR),
            FallbackPolicy::NearestLower,
        )
        .expect("fallback resolves");
        assert_eq!(locales(&plan), vec![Locale::ja_JP]);
        assert_eq!(
            plan.warnings,
            vec![DownloadWarning::AudioFallback {
                requested: Locale::fr_FR,
                used: Locale::ja_JP,
            }]
        );
    }

    #[test]
    fn audio_locale_list_preserves_order() {
        let plan = select_audio(
            &dubbed(),
            &AudioSelection::Locales(vec![Locale::en_US, Locale::de_DE]),
            FallbackPolicy::Exact,
        )
        .expect("list resolves");
        assert_eq!(locales(&plan), vec![Locale::en_US, Locale::de_DE]);
    }

    #[test]
    fn audio_locale_list_drops_missing_with_warning_under_fallback() {
        let plan = select_audio(
            &dubbed(),
            &AudioSelection::Locales(vec![Locale::en_US, Locale::fr_FR]),
            FallbackPolicy::BestAvailable,
        )
        .expect("list resolves");
        assert_eq!(locales(&plan), vec![Locale::en_US]);
        assert_eq!(
            plan.warnings,
            vec![DownloadWarning::AudioLocaleUnavailable {
                requested: Locale::fr_FR,
            }]
        );
    }

    #[test]
    fn audio_all_orders_original_first_then_stable() {
        let plan =
            select_audio(&dubbed(), &AudioSelection::All, FallbackPolicy::Exact).expect("all");
        // Original (ja) first, then remaining by content id: de-DE before en-US.
        assert_eq!(
            locales(&plan),
            vec![Locale::ja_JP, Locale::de_DE, Locale::en_US]
        );
    }

    #[test]
    fn audio_single_unlabeled_version_passes_through() {
        let single = media(vec![version(None, true)]);
        let plan = select_audio(
            &single,
            &AudioSelection::Locale(Locale::fr_FR),
            FallbackPolicy::Exact,
        )
        .expect("single version passthrough");
        assert_eq!(plan.versions.len(), 1);
        assert!(plan.warnings.is_empty());
    }

    fn videos() -> Vec<VideoQualityCandidate> {
        vec![
            VideoQualityCandidate {
                width: 1920,
                height: 1080,
                bandwidth: 8_000_000,
            },
            VideoQualityCandidate {
                width: 1280,
                height: 720,
                bandwidth: 4_000_000,
            },
            VideoQualityCandidate {
                width: 640,
                height: 360,
                bandwidth: 1_000_000,
            },
        ]
    }

    #[test]
    fn video_best_and_exact() {
        assert_eq!(
            select_video_quality(&videos(), &QualitySelection::Best, FallbackPolicy::Exact)
                .unwrap()
                .index,
            0
        );
        assert_eq!(
            select_video_quality(
                &videos(),
                &QualitySelection::ExactHeight(720),
                FallbackPolicy::Exact
            )
            .unwrap()
            .index,
            1
        );
    }

    #[test]
    fn video_exact_missing_errors_under_exact_policy() {
        let err = select_video_quality(
            &videos(),
            &QualitySelection::ExactHeight(480),
            FallbackPolicy::Exact,
        )
        .expect_err("480 is unavailable");
        assert!(matches!(err, SelectionError::QualityUnavailable { .. }));
    }

    #[test]
    fn video_nearest_lower_picks_below_target() {
        let choice = select_video_quality(
            &videos(),
            &QualitySelection::ExactHeight(480),
            FallbackPolicy::NearestLower,
        )
        .unwrap();
        assert_eq!(choice.index, 2); // 360p is the nearest below 480p
        assert!(choice.warning.is_some());
    }

    #[test]
    fn video_best_available_picks_top() {
        let choice = select_video_quality(
            &videos(),
            &QualitySelection::ExactHeight(480),
            FallbackPolicy::BestAvailable,
        )
        .unwrap();
        assert_eq!(choice.index, 0);
        assert!(choice.warning.is_some());
    }

    #[test]
    fn video_max_height_respects_and_falls_back() {
        assert_eq!(
            select_video_quality(
                &videos(),
                &QualitySelection::MaxHeight(720),
                FallbackPolicy::Exact
            )
            .unwrap()
            .index,
            1
        );
        let below = select_video_quality(
            &videos(),
            &QualitySelection::MaxHeight(200),
            FallbackPolicy::Exact,
        )
        .unwrap();
        assert_eq!(below.index, 2); // lowest available when nothing fits the cap
        assert!(below.warning.is_some());
    }

    #[test]
    fn video_ranked_index() {
        assert_eq!(
            select_video_quality(
                &videos(),
                &QualitySelection::RankedIndex(0),
                FallbackPolicy::Exact
            )
            .unwrap()
            .index,
            0
        );
        assert_eq!(
            select_video_quality(
                &videos(),
                &QualitySelection::RankedIndex(2),
                FallbackPolicy::Exact
            )
            .unwrap()
            .index,
            2
        );
        assert!(matches!(
            select_video_quality(
                &videos(),
                &QualitySelection::RankedIndex(9),
                FallbackPolicy::Exact
            ),
            Err(SelectionError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn video_empty_errors() {
        assert!(matches!(
            select_video_quality(&[], &QualitySelection::Best, FallbackPolicy::Exact),
            Err(SelectionError::NoVideoRepresentations)
        ));
    }

    fn audios() -> Vec<AudioQualityCandidate> {
        vec![
            AudioQualityCandidate {
                bandwidth: 256_000,
                sampling_rate: 48_000,
            },
            AudioQualityCandidate {
                bandwidth: 128_000,
                sampling_rate: 44_100,
            },
        ]
    }

    #[test]
    fn audio_quality_exact_and_fallback() {
        assert_eq!(
            select_audio_quality(
                &audios(),
                &QualitySelection::ExactBandwidth(256_000),
                FallbackPolicy::Exact
            )
            .unwrap()
            .index,
            0
        );
        assert!(matches!(
            select_audio_quality(
                &audios(),
                &QualitySelection::ExactBandwidth(999),
                FallbackPolicy::Exact
            ),
            Err(SelectionError::QualityUnavailable { .. })
        ));
        let fallback = select_audio_quality(
            &audios(),
            &QualitySelection::ExactBandwidth(999),
            FallbackPolicy::BestAvailable,
        )
        .unwrap();
        assert_eq!(fallback.index, 0);
        assert!(fallback.warning.is_some());
    }

    #[test]
    fn audio_quality_best_maps_to_top_bandwidth() {
        assert_eq!(
            select_audio_quality(&audios(), &QualitySelection::Best, FallbackPolicy::Exact)
                .unwrap()
                .index,
            0
        );
    }

    #[test]
    fn hardsub_selection() {
        let available = [Locale::en_US, Locale::de_DE];
        assert_eq!(
            select_hardsub(&available, &HardSubSelection::None).unwrap(),
            None
        );
        assert_eq!(
            select_hardsub(&available, &HardSubSelection::Locale(Locale::en_US)).unwrap(),
            Some(Locale::en_US)
        );
        assert!(matches!(
            select_hardsub(&available, &HardSubSelection::Locale(Locale::fr_FR)),
            Err(SelectionError::HardSubUnavailable { .. })
        ));
    }

    #[test]
    fn cdn_selection() {
        assert_eq!(select_cdn(3, &CdnSelection::Automatic).unwrap(), 0);
        assert_eq!(select_cdn(3, &CdnSelection::Index(2)).unwrap(), 2);
        assert!(select_cdn(2, &CdnSelection::Index(5)).is_err());
        assert!(select_cdn(0, &CdnSelection::Automatic).is_err());
    }

    fn subtitle_tracks() -> Vec<SubtitleTrackInfo> {
        vec![
            SubtitleTrackInfo {
                locale: Locale::en_US,
                is_caption: false,
                format: "ass".to_string(),
            },
            SubtitleTrackInfo {
                locale: Locale::en_US,
                is_caption: true,
                format: "ass".to_string(),
            },
            SubtitleTrackInfo {
                locale: Locale::de_DE,
                is_caption: false,
                format: "vtt".to_string(),
            },
        ]
    }

    #[test]
    fn subtitles_all_with_and_without_captions() {
        let with_captions = select_subtitles(&subtitle_tracks(), &SubtitleSelection::default());
        assert_eq!(with_captions.tracks, vec![0, 1, 2]);

        let no_captions = SubtitleSelection {
            include_captions: false,
            ..SubtitleSelection::default()
        };
        let plan = select_subtitles(&subtitle_tracks(), &no_captions);
        assert_eq!(plan.tracks, vec![0, 2]);
    }

    #[test]
    fn subtitles_explicit_and_missing() {
        let selection = SubtitleSelection {
            locales: SubtitleLocales::Explicit(vec![Locale::de_DE, Locale::fr_FR]),
            ..SubtitleSelection::default()
        };
        let plan = select_subtitles(&subtitle_tracks(), &selection);
        assert_eq!(plan.tracks, vec![2]);
        assert_eq!(
            plan.warnings,
            vec![DownloadWarning::SubtitleUnavailable {
                requested: Locale::fr_FR,
            }]
        );
    }
}
