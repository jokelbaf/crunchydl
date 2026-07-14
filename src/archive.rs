//! Optional versioned JSON archive updated only after output commit.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::{Error, OutputTrack};

const SCHEMA_VERSION: u32 = 1;

/// Stable archive identity for one completed selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveKey {
    /// Crunchyroll media id.
    pub media_id: String,
    /// Deterministic requested-selection fingerprint.
    pub selection_fingerprint: String,
}

/// A committed download stored in an archive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveEntry {
    /// Stable selection identity.
    pub key: ArchiveKey,
    /// Committed output path.
    pub output: PathBuf,
    /// Output track diagnostics.
    pub tracks: Vec<OutputTrack>,
}

/// Persistence seam for opt-in download archives.
pub trait Archive: Send + Sync {
    /// Find an entry. A stale entry whose output no longer exists must return
    /// `None` so it cannot suppress a new download.
    ///
    /// # Errors
    ///
    /// Returns a filesystem or schema error.
    fn find(&self, key: &ArchiveKey) -> Result<Option<ArchiveEntry>, Error>;

    /// Persist a committed entry.
    ///
    /// # Errors
    ///
    /// Returns a filesystem or serialization error.
    fn record(&self, entry: &ArchiveEntry) -> Result<(), Error>;
}

/// A versioned JSON archive using atomic replacement.
pub struct JsonArchive {
    path: PathBuf,
    access: Mutex<()>,
}

impl JsonArchive {
    /// Create an archive stored at `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            access: Mutex::new(()),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Document {
    schema_version: u32,
    entries: Vec<ArchiveEntry>,
}

impl Archive for JsonArchive {
    fn find(&self, key: &ArchiveKey) -> Result<Option<ArchiveEntry>, Error> {
        let _guard = self
            .access
            .lock()
            .map_err(|_| Error::Filesystem("archive lock poisoned".into()))?;
        let document = load(&self.path)?;
        Ok(document
            .entries
            .into_iter()
            .find(|entry| &entry.key == key && entry.output.is_file()))
    }

    fn record(&self, entry: &ArchiveEntry) -> Result<(), Error> {
        let _guard = self
            .access
            .lock()
            .map_err(|_| Error::Filesystem("archive lock poisoned".into()))?;
        let mut document = load(&self.path)?;
        document
            .entries
            .retain(|existing| existing.key != entry.key);
        document.entries.push(entry.clone());
        document.entries.sort_by(|left, right| {
            left.key.media_id.cmp(&right.key.media_id).then_with(|| {
                left.key
                    .selection_fingerprint
                    .cmp(&right.key.selection_fingerprint)
            })
        });
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|error| Error::Filesystem(format!("serialize archive: {error}")))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        }
        atomic_write(&self.path, &bytes)
    }
}

fn load(path: &Path) -> Result<Document, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document {
                schema_version: SCHEMA_VERSION,
                entries: Vec::new(),
            });
        }
        Err(error) => return Err(io_error(path, error)),
    };
    let document: Document = serde_json::from_slice(&bytes).map_err(|error| {
        Error::Filesystem(format!(
            "invalid archive schema at {}: {error}",
            path.display()
        ))
    })?;
    if document.schema_version != SCHEMA_VERSION {
        return Err(Error::Filesystem(format!(
            "unsupported archive schema version {}",
            document.schema_version
        )));
    }
    Ok(document)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temporary = path.with_extension("tmp");
    let mut file =
        std::fs::File::create(&temporary).map_err(|error| io_error(&temporary, error))?;
    use std::io::Write as _;
    file.write_all(bytes)
        .map_err(|error| io_error(&temporary, error))?;
    file.sync_all()
        .map_err(|error| io_error(&temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| io_error(path, error))
}

fn io_error(path: &Path, error: std::io::Error) -> Error {
    Error::Filesystem(format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_entries_do_not_suppress_downloads() {
        let root = std::env::temp_dir().join(format!("crunchydl-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let archive = JsonArchive::new(root.join("archive.json"));
        let key = ArchiveKey {
            media_id: "M1".into(),
            selection_fingerprint: "abc".into(),
        };
        archive
            .record(&ArchiveEntry {
                key: key.clone(),
                output: root.join("missing.mkv"),
                tracks: Vec::new(),
            })
            .unwrap();
        assert!(archive.find(&key).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
