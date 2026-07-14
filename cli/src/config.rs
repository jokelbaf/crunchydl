//! Versioned user configuration.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::AppPaths;

const SCHEMA_VERSION: u32 = 1;

/// Selected DRM backend.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum DrmBackend {
    #[default]
    Auto,
    PlayReady,
    Widevine,
}

impl std::fmt::Display for DrmBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => formatter.write_str("auto (.prd/.wvd)"),
            Self::PlayReady => formatter.write_str("playready"),
            Self::Widevine => formatter.write_str("widevine"),
        }
    }
}

impl DrmBackend {
    pub(crate) fn resolve(self, device: &Path) -> Result<Self> {
        if !matches!(self, Self::Auto) {
            return Ok(self);
        }
        match device
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("prd") => Ok(Self::PlayReady),
            Some("wvd") => Ok(Self::Widevine),
            _ => Err(Error::CannotDetectDrmBackend),
        }
    }
}

/// Preferences that are safe to store as plain text.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    schema_version: u32,
    pub(crate) output_dir: PathBuf,
    pub(crate) filename: String,
    pub(crate) output_layout: Option<String>,
    pub(crate) drm_backend: DrmBackend,
    pub(crate) drm_device: Option<PathBuf>,
    pub(crate) license_endpoint: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let output_dir = directories::UserDirs::new()
            .and_then(|directories| directories.download_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("crunchydl");
        Self {
            schema_version: SCHEMA_VERSION,
            output_dir,
            filename: "{series} - {episode} - {title} [{media_id}]".to_string(),
            output_layout: Some(
                "{series}/Season {season_number}/{episode} - {title} [{media_id}]".to_string(),
            ),
            drm_backend: DrmBackend::Auto,
            drm_device: None,
            license_endpoint: None,
        }
    }
}

impl Config {
    pub(crate) fn load(paths: &AppPaths) -> Result<Self> {
        let source = match fs::read_to_string(&paths.config) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(io_error("read configuration", &paths.config)),
        };
        let config: Self = toml::from_str(&source).map_err(|error| Error::InvalidConfig {
            path: paths.config.clone(),
            message: error.to_string(),
        })?;
        if config.schema_version != SCHEMA_VERSION {
            return Err(Error::InvalidConfig {
                path: paths.config.clone(),
                message: format!("unsupported schema version {}", config.schema_version),
            });
        }
        Ok(config)
    }

    pub(crate) fn save(&mut self, paths: &AppPaths) -> Result<()> {
        self.schema_version = SCHEMA_VERSION;
        let bytes = toml::to_string_pretty(self).map_err(|error| Error::InvalidConfig {
            path: paths.config.clone(),
            message: error.to_string(),
        })?;
        atomic_write(&paths.config, bytes.as_bytes())
    }
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::Filesystem {
        operation: "resolve parent directory",
        path: path.to_path_buf(),
    })?;
    fs::create_dir_all(parent).map_err(|_| io_error("create directory", parent))?;
    let temporary = path.with_extension("tmp");
    write_private(&temporary, bytes)?;
    fs::rename(&temporary, path).map_err(|_| io_error("replace file", path))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| io_error("create file", path))?;
    std::io::Write::write_all(&mut file, bytes).map_err(|_| io_error("write file", path))?;
    file.sync_all().map_err(|_| io_error("sync file", path))
}

fn io_error(operation: &'static str, path: &Path) -> Error {
    Error::Filesystem {
        operation,
        path: path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_backend_uses_the_device_extension_case_insensitively() {
        assert!(matches!(
            DrmBackend::Auto.resolve(Path::new("device.prd")).unwrap(),
            DrmBackend::PlayReady
        ));
        assert!(matches!(
            DrmBackend::Auto.resolve(Path::new("DEVICE.WVD")).unwrap(),
            DrmBackend::Widevine
        ));
        assert!(DrmBackend::Auto.resolve(Path::new("device.bin")).is_err());
    }

    #[test]
    fn explicit_backend_overrides_the_device_extension() {
        assert!(matches!(
            DrmBackend::Widevine
                .resolve(Path::new("device.prd"))
                .unwrap(),
            DrmBackend::Widevine
        ));
    }
}
