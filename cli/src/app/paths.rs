//! OS-specific application paths.

use std::path::PathBuf;

use directories::ProjectDirs;

use crate::error::{Error, Result};

/// Every persistent path owned by the CLI.
#[derive(Clone, Debug)]
pub(crate) struct AppPaths {
    pub(crate) config: PathBuf,
    pub(crate) session: PathBuf,
    pub(crate) archive: PathBuf,
    pub(crate) queue: PathBuf,
    pub(crate) thumbnail_cache: PathBuf,
}

impl AppPaths {
    pub(crate) fn discover() -> Result<Self> {
        let project =
            ProjectDirs::from("dev", "jokelbaf", "crunchydl").ok_or(Error::AppDirectories)?;
        Ok(Self {
            config: project.config_dir().join("config.toml"),
            session: project.data_local_dir().join("session.json"),
            archive: project.data_local_dir().join("archive.json"),
            queue: project.data_local_dir().join("queue.json"),
            thumbnail_cache: project.cache_dir().join("thumbnails"),
        })
    }
}
