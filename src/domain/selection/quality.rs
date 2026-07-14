//! Deterministic video and audio representation ranking.

use crate::event::DownloadWarning;

use super::{FallbackPolicy, QualitySelection, SelectionError};

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
/// Video is ranked by height, then bandwidth, then width. Exact requests use
/// the configured fallback policy.
///
/// # Errors
///
/// Returns [`SelectionError`] when no candidate exists or an exact request
/// cannot be met under [`FallbackPolicy::Exact`].
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
/// Audio is ranked by bandwidth, then sampling rate.
///
/// # Errors
///
/// Returns [`SelectionError`] when no candidate exists or an exact request
/// cannot be met under [`FallbackPolicy::Exact`].
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
        _ => Ok(QualityChoice {
            index: argmax(candidates, audio_quality_key),
            warning: None,
        }),
    }
}

fn argmax<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> usize {
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
