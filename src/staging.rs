use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::Error;

pub(crate) const JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct CompletedSegment {
    pub(crate) index: usize,
    pub(crate) bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ResumeJournal {
    pub(crate) schema_version: u32,
    pub(crate) media_id: String,
    pub(crate) version_id: String,
    pub(crate) plan_fingerprint: String,
    pub(crate) representation_fingerprint: String,
    pub(crate) init_identity: String,
    pub(crate) init_bytes: Option<u64>,
    pub(crate) segment_identities: Vec<String>,
    pub(crate) completed: Vec<CompletedSegment>,
    pub(crate) created_unix_seconds: u64,
    pub(crate) updated_unix_seconds: u64,
}

impl ResumeJournal {
    pub(crate) fn new(
        media_id: String,
        version_id: String,
        plan_fingerprint: String,
        representation_fingerprint: String,
        init_identity: String,
        segment_identities: Vec<String>,
    ) -> Self {
        let now = unix_seconds();
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            media_id,
            version_id,
            plan_fingerprint,
            representation_fingerprint,
            init_identity,
            init_bytes: None,
            segment_identities,
            completed: Vec::new(),
            created_unix_seconds: now,
            updated_unix_seconds: now,
        }
    }

    pub(crate) async fn load(path: &Path) -> Result<Option<Self>, Error> {
        let bytes = match tokio::fs::read(path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(filesystem_error(path, error)),
        };
        let journal: Self = serde_json::from_slice(&bytes)
            .map_err(|error| Error::ResumeMismatch(format!("invalid journal schema: {error}")))?;
        if journal.schema_version != JOURNAL_SCHEMA_VERSION {
            return Err(Error::ResumeMismatch(format!(
                "unsupported journal schema version {}",
                journal.schema_version
            )));
        }
        Ok(Some(journal))
    }

    pub(crate) async fn save(&mut self, path: &Path) -> Result<(), Error> {
        self.updated_unix_seconds = unix_seconds();
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| Error::Filesystem(format!("serialize resume journal: {error}")))?;
        atomic_write(path, &bytes).await
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StagingLayout {
    pub(crate) init: PathBuf,
    pub(crate) segments: PathBuf,
    pub(crate) journal: PathBuf,
}

impl StagingLayout {
    pub(crate) fn new(root: &Path, media_id: &str, representation: &str) -> Self {
        let directory = root.join(format!(
            "{}-{}",
            safe_component(media_id),
            &representation[..representation.len().min(16)]
        ));
        Self {
            init: directory.join("init.part"),
            segments: directory.join("segments"),
            journal: directory.join("resume.json"),
        }
    }

    pub(crate) async fn create(&self) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.segments)
            .await
            .map_err(|error| filesystem_error(&self.segments, error))
    }

    pub(crate) fn segment(&self, index: usize) -> PathBuf {
        self.segments.join(format!("{index:06}.part"))
    }
}

pub(crate) async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::Filesystem(format!("invalid staging path: {}", path.display())))?;
    let temporary = path.with_file_name(format!("{file_name}.tmp"));
    let mut file = tokio::fs::File::create(&temporary)
        .await
        .map_err(|error| filesystem_error(&temporary, error))?;
    file.write_all(bytes)
        .await
        .map_err(|error| filesystem_error(&temporary, error))?;
    file.flush()
        .await
        .map_err(|error| filesystem_error(&temporary, error))?;
    file.sync_all()
        .await
        .map_err(|error| filesystem_error(&temporary, error))?;
    drop(file);
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|error| filesystem_error(path, error))
}

pub(crate) fn filesystem_error(path: &Path, error: std::io::Error) -> Error {
    Error::Filesystem(format!("{}: {error}", path.display()))
}

fn safe_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.is_empty() {
        "media".to_string()
    } else {
        safe
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_cannot_escape_its_root() {
        let layout = StagingLayout::new(Path::new("/tmp/root"), "../../EP:1", "abcdef");
        assert!(layout.journal.starts_with("/tmp/root"));
        assert!(!layout.journal.to_string_lossy().contains(".."));
    }
}
