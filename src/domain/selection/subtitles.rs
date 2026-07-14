//! Hardsub, CDN, and soft-subtitle selection.

use crunchyroll_rs::Locale;

use crate::event::DownloadWarning;

use super::{CdnSelection, HardSubSelection, SelectionError, SubtitleLocales, SubtitleSelection};

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
/// Signs handling is applied later during planning; this resolver selects by
/// locale and caption flag only.
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
