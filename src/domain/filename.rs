//! Compiled filename templates and cross-platform component sanitization.

use std::path::{Component, Path, PathBuf};

use crate::{Error, ResolvedMedia};

/// A validated filename template compiled once per output configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilenameTemplate {
    parts: Vec<Part>,
}

/// A validated relative output layout made of independently sanitized
/// components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputLayoutTemplate {
    components: Vec<FilenameTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Literal(String),
    Field(Field),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Series,
    Season,
    Title,
    Episode,
    Sequence,
    MediaId,
    SeasonNumber,
    Height,
    Audio,
}

impl FilenameTemplate {
    /// Compile a template containing literals and `{field}` placeholders.
    ///
    /// Supported fields are `series`, `season`, `title`, `episode`, `sequence`,
    /// `media_id`, `season_number`, `height`, and `audio`.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error for unmatched braces or unknown fields.
    pub fn compile(template: &str) -> Result<Self, Error> {
        let mut parts = Vec::new();
        let mut remaining = template;
        while let Some(open) = remaining.find('{') {
            let (literal, tail) = remaining.split_at(open);
            if !literal.is_empty() {
                parts.push(Part::Literal(literal.to_string()));
            }
            let close = tail
                .find('}')
                .ok_or_else(|| Error::Filesystem("unclosed filename placeholder".into()))?;
            let field = match &tail[1..close] {
                "series" => Field::Series,
                "season" => Field::Season,
                "title" => Field::Title,
                "episode" => Field::Episode,
                "sequence" => Field::Sequence,
                "media_id" => Field::MediaId,
                "season_number" => Field::SeasonNumber,
                "height" => Field::Height,
                "audio" => Field::Audio,
                _ => return Err(Error::Filesystem("unknown filename placeholder".into())),
            };
            parts.push(Part::Field(field));
            remaining = &tail[close + 1..];
        }
        if remaining.contains('}') {
            return Err(Error::Filesystem("unmatched filename brace".into()));
        }
        if !remaining.is_empty() {
            parts.push(Part::Literal(remaining.to_string()));
        }
        if parts.is_empty() {
            return Err(Error::Filesystem("empty filename template".into()));
        }
        Ok(Self { parts })
    }

    pub(crate) fn render(
        &self,
        media: &ResolvedMedia,
        height: Option<u32>,
        audio: &[String],
        max_length: usize,
    ) -> String {
        format!(
            "{}.mkv",
            self.render_component(media, height, audio, max_length)
        )
    }

    fn render_component(
        &self,
        media: &ResolvedMedia,
        height: Option<u32>,
        audio: &[String],
        max_length: usize,
    ) -> String {
        sanitize_component(&render_parts(&self.parts, media, height, audio), max_length)
    }
}

impl OutputLayoutTemplate {
    /// Compile a `/`-separated relative layout.
    ///
    /// Each component supports the same placeholders as [`FilenameTemplate`].
    /// The final component receives the `.mkv` suffix during rendering.
    ///
    /// # Errors
    ///
    /// Returns an error for absolute, empty, current-directory, parent, or
    /// backslash-separated components and for invalid placeholders.
    pub fn compile(template: &str) -> Result<Self, Error> {
        if template.is_empty()
            || template.starts_with('/')
            || template.ends_with('/')
            || template.contains('\\')
        {
            return Err(Error::Filesystem("invalid output layout template".into()));
        }
        let components = template
            .split('/')
            .map(|component| {
                if component.is_empty() || matches!(component, "." | "..") {
                    return Err(Error::Filesystem(
                        "output layout contains an invalid component".into(),
                    ));
                }
                FilenameTemplate::compile(component)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { components })
    }

    pub(crate) fn render(
        &self,
        media: &ResolvedMedia,
        height: Option<u32>,
        audio: &[String],
        max_length: usize,
    ) -> PathBuf {
        let last = self.components.len().saturating_sub(1);
        self.components
            .iter()
            .enumerate()
            .map(|(index, component)| {
                let rendered = component.render_component(media, height, audio, max_length);
                if index == last {
                    format!("{rendered}.mkv")
                } else {
                    rendered
                }
            })
            .collect()
    }
}

fn render_parts(
    parts: &[Part],
    media: &ResolvedMedia,
    height: Option<u32>,
    audio: &[String],
) -> String {
    let mut value = String::new();
    for part in parts {
        match part {
            Part::Literal(literal) => value.push_str(literal),
            Part::Field(field) => match field {
                Field::Series => {
                    value.push_str(media.series_title.as_deref().unwrap_or(&media.title))
                }
                Field::Season => value.push_str(media.season_title.as_deref().unwrap_or("")),
                Field::Title => value.push_str(&media.title),
                Field::Episode => value.push_str(media.episode.as_deref().unwrap_or("")),
                Field::Sequence => value.push_str(&format!("{}", media.sequence_number)),
                Field::MediaId => value.push_str(&media.content_id),
                Field::SeasonNumber => {
                    if let Some(number) = media.season_number {
                        value.push_str(&number.to_string());
                    }
                }
                Field::Height => {
                    if let Some(height) = height {
                        value.push_str(&height.to_string());
                    }
                }
                Field::Audio => value.push_str(&audio.join("+")),
            },
        }
    }
    value
}

pub(crate) fn output_path(root: &Path, relative: &Path) -> Result<PathBuf, Error> {
    if root
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(Error::Filesystem(
            "output root contains parent traversal".into(),
        ));
    }
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::Filesystem("filename escaped output root".into()));
    }
    Ok(root.join(relative))
}

fn sanitize_component(value: &str, max_length: usize) -> String {
    let mut safe = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    safe = safe.trim().trim_end_matches(['.', ' ']).to_string();
    if safe == "." || safe == ".." {
        safe = "_".into();
    } else if windows_reserved(&safe) {
        safe.insert(0, '_');
    }
    if safe.is_empty() {
        safe = "media".into();
    }
    if safe.chars().count() > max_length {
        safe = safe.chars().take(max_length).collect();
    }
    if safe.is_empty() {
        safe = "media".into();
    } else if windows_reserved(&safe) {
        safe.insert(0, '_');
    }
    safe
}

fn windows_reserved(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
