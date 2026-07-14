pub(crate) mod fonts;

use std::time::Duration;

use crunchyroll_rs::Locale;

use crate::Error;
use crate::api::CrunchyrollApi;
use crate::plan::PreparedSubtitle;

pub(crate) trait SubtitleFetcher {
    async fn fetch(&self, url: &str) -> Result<String, Error>;
}

impl<T: CrunchyrollApi> SubtitleFetcher for T {
    async fn fetch(&self, url: &str) -> Result<String, Error> {
        self.fetch_subtitle(url).await
    }
}

/// A supported subtitle source format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubtitleFormat {
    /// Advanced SubStation Alpha.
    Ass,
    /// Web Video Text Tracks.
    WebVtt,
}

impl SubtitleFormat {
    pub(crate) fn parse(value: &str) -> Result<Self, Error> {
        match value.to_ascii_lowercase().as_str() {
            "ass" | "ssa" => Ok(Self::Ass),
            "vtt" | "webvtt" => Ok(Self::WebVtt),
            _ => Err(Error::Subtitle("unsupported subtitle format".to_string())),
        }
    }
}

/// Stable metadata carried from selection into the output subtitle track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleMetadata {
    /// Subtitle locale.
    pub locale: Locale,
    /// Output track title.
    pub title: String,
    /// Whether this is a closed-caption resource.
    pub is_caption: bool,
    /// Whether this is a signs resource matching selected audio.
    pub is_signs: bool,
    /// Whether players should enable the track by default.
    pub default: bool,
    /// Whether players should force the track on.
    pub forced: bool,
}

/// Explicit structural ASS normalization choices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssNormalization {
    /// Add missing PlayResX/PlayResY with these values.
    pub play_resolution: Option<(u32, u32)>,
    /// Add missing LayoutResX/LayoutResY with these values.
    pub layout_resolution: Option<(u32, u32)>,
    /// Add or replace WrapStyle.
    pub wrap_style: Option<u8>,
    /// Add or replace Timer.
    pub timer: Option<f64>,
    /// Add or replace ScaledBorderAndShadow.
    pub scaled_border_and_shadow: Option<bool>,
    /// Remove semicolon comments and the Aegisub Project Garbage section.
    pub remove_project_garbage: bool,
    /// Clamp dialogue ends and remove dialogue starts outside media duration.
    pub clamp_to_duration: bool,
}

/// Options for converting or structurally normalizing one subtitle resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SubtitleProcessingOptions {
    /// Normalization settings; `None` preserves raw ASS exactly.
    pub normalization: Option<AssNormalization>,
    /// Known media duration used only when clamping is explicitly enabled.
    pub media_duration: Option<Duration>,
}

/// A processed ASS subtitle ready for Matroska muxing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleTrack {
    /// Exact output metadata selected by the caller.
    pub metadata: SubtitleMetadata,
    /// ASS document, including script info, styles, and events.
    pub ass: String,
    /// Canonical font family names actually referenced by styles or overrides.
    pub referenced_fonts: Vec<String>,
}

/// Convert or normalize a subtitle resource without losing track metadata.
///
/// Raw ASS is byte-for-byte preserved when no normalization is requested.
/// WebVTT is converted to ASS with cue order, basic inline styling, class
/// styles, line breaks, and percentage positioning retained.
///
/// # Errors
///
/// Returns a subtitle error for malformed required sections, timestamps, or
/// unsupported input.
pub fn process_subtitle(
    source: &str,
    format: SubtitleFormat,
    metadata: SubtitleMetadata,
    options: &SubtitleProcessingOptions,
) -> Result<SubtitleTrack, Error> {
    let mut ass = match format {
        SubtitleFormat::Ass => source.to_string(),
        SubtitleFormat::WebVtt => vtt_to_ass(source, &metadata.title)?,
    };
    if let Some(normalization) = &options.normalization {
        ass = normalize_ass(&ass, normalization, options.media_duration)?;
    }
    let referenced_fonts = extract_fonts(&ass)?;
    Ok(SubtitleTrack {
        metadata,
        ass,
        referenced_fonts,
    })
}

#[allow(dead_code)]
pub(crate) async fn download_selected<A: SubtitleFetcher>(
    api: &A,
    subtitles: &[PreparedSubtitle],
    options: &SubtitleProcessingOptions,
) -> Result<Vec<SubtitleTrack>, Error> {
    let mut tracks = Vec::with_capacity(subtitles.len());
    for subtitle in subtitles {
        let source = api.fetch(&subtitle.url).await?;
        let diagnostic = &subtitle.diagnostic;
        tracks.push(process_subtitle(
            &source,
            SubtitleFormat::parse(&diagnostic.format)?,
            SubtitleMetadata {
                locale: diagnostic.locale.clone(),
                title: diagnostic.title.clone(),
                is_caption: diagnostic.is_caption,
                is_signs: diagnostic.is_signs,
                default: diagnostic.default,
                forced: diagnostic.forced,
            },
            options,
        )?);
    }
    Ok(tracks)
}

mod ass;
mod font_refs;
mod vtt;

use ass::normalize_ass;
use font_refs::extract_fonts;
use vtt::vtt_to_ass;
