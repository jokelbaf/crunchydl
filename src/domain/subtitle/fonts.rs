use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{DownloadWarning, Error, SubtitleTrack};

const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

/// Behavior when an ASS-referenced font cannot be resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontPolicy {
    /// Continue and record a structured warning.
    #[default]
    Warn,
    /// Fail subtitle processing before muxing.
    Error,
}

/// A validated font ready for a container attachment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFont {
    /// Canonical family name requested by the subtitle.
    pub family: String,
    /// Attachment filename.
    pub filename: String,
    /// Matroska attachment MIME type.
    pub mime_type: String,
    /// Validated OpenType or TrueType bytes.
    pub data: Vec<u8>,
}

/// Resolves one ASS family name without performing implicit network access.
pub trait FontResolver: Send + Sync {
    /// Resolve a family to validated font bytes, or return `None` when absent.
    ///
    /// # Errors
    ///
    /// Returns a font error for unreadable or invalid candidate data.
    fn resolve(&self, family: &str) -> Result<Option<ResolvedFont>, Error>;
}

/// A local/cache directory resolver with optional explicit family aliases.
#[derive(Clone, Debug, Default)]
pub struct DirectoryFontResolver {
    roots: Vec<PathBuf>,
    aliases: BTreeMap<String, PathBuf>,
}

impl DirectoryFontResolver {
    /// Search the given directories in order.
    #[must_use]
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
            aliases: BTreeMap::new(),
        }
    }

    /// Associate a family with a relative filename inside each search root.
    #[must_use]
    pub fn with_alias(mut self, family: impl Into<String>, filename: impl Into<PathBuf>) -> Self {
        self.aliases
            .insert(normalize_family(&family.into()), filename.into());
        self
    }

    fn candidates(&self, family: &str) -> Result<Vec<PathBuf>, Error> {
        let key = normalize_family(family);
        if let Some(relative) = self.aliases.get(&key) {
            return Ok(self.roots.iter().map(|root| root.join(relative)).collect());
        }
        let mut candidates = Vec::new();
        for root in &self.roots {
            let entries = match fs::read_dir(root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err(Error::Font(
                        "font cache directory is unreadable".to_string(),
                    ));
                }
            };
            for entry in entries {
                let entry =
                    entry.map_err(|_| Error::Font("font cache entry is unreadable".to_string()))?;
                let path = entry.path();
                let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
                    continue;
                };
                if normalize_family(stem) == key && supported_extension(&path) {
                    candidates.push(path);
                }
            }
        }
        candidates.sort();
        Ok(candidates)
    }
}

impl FontResolver for DirectoryFontResolver {
    fn resolve(&self, family: &str) -> Result<Option<ResolvedFont>, Error> {
        for path in self.candidates(family)? {
            if !path.is_file() {
                continue;
            }
            return Ok(Some(load_font(family, &path)?));
        }
        Ok(None)
    }
}

/// Resolve the union of fonts referenced by selected subtitle tracks.
///
/// No unreferenced font is queried or returned. Families are deduplicated
/// case-insensitively in deterministic order.
///
/// # Errors
///
/// Returns a font error when a candidate is invalid or a missing family is
/// configured as [`FontPolicy::Error`].
pub fn resolve_referenced_fonts(
    tracks: &[SubtitleTrack],
    resolver: &dyn FontResolver,
    policy: FontPolicy,
) -> Result<(Vec<ResolvedFont>, Vec<DownloadWarning>), Error> {
    let mut families = BTreeMap::<String, String>::new();
    for track in tracks {
        for family in &track.referenced_fonts {
            families
                .entry(normalize_family(family))
                .or_insert_with(|| family.clone());
        }
    }
    let mut resolved = Vec::new();
    let mut warnings = Vec::new();
    let mut filenames = BTreeSet::new();
    for family in families.into_values() {
        match resolver.resolve(&family)? {
            Some(font) => {
                let identity = font.filename.to_ascii_lowercase();
                if filenames.insert(identity) {
                    resolved.push(font);
                }
            }
            None if policy == FontPolicy::Warn => {
                warnings.push(DownloadWarning::MissingFont { family });
            }
            None => {
                return Err(Error::Font(format!(
                    "referenced font {family} was not found"
                )));
            }
        }
    }
    Ok((resolved, warnings))
}

fn load_font(family: &str, path: &Path) -> Result<ResolvedFont, Error> {
    if !supported_extension(path) {
        return Err(Error::Font("font has an unsupported extension".to_string()));
    }
    let metadata =
        fs::metadata(path).map_err(|_| Error::Font("font metadata is unreadable".to_string()))?;
    if metadata.len() < 12 || metadata.len() > MAX_FONT_BYTES {
        return Err(Error::Font(
            "font size is outside the supported bounds".to_string(),
        ));
    }
    let data = fs::read(path).map_err(|_| Error::Font("font data is unreadable".to_string()))?;
    if !matches!(data.get(..4), Some(b"OTTO" | b"\0\x01\0\0")) {
        return Err(Error::Font(
            "font has invalid OpenType/TrueType magic".to_string(),
        ));
    }
    let filename = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| Error::Font("font filename is not UTF-8".to_string()))?
        .to_string();
    let mime_type = match path.extension().and_then(OsStr::to_str) {
        Some(extension) if extension.eq_ignore_ascii_case("otf") => "application/vnd.ms-opentype",
        _ => "application/x-truetype-font",
    }
    .to_string();
    Ok(ResolvedFont {
        family: family.to_string(),
        filename,
        mime_type,
        data,
    })
}

fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
        })
}

fn normalize_family(family: &str) -> String {
    family
        .trim()
        .trim_matches(['\'', '"'])
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-' && *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}
