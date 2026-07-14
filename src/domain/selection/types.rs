//! Selection policies and the errors returned when a request cannot be met.

use crunchyroll_rs::Locale;

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
