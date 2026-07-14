//! Versioned, atomically persisted download queue.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::atomic_write;
use crate::error::{Error, Result};

const SCHEMA_VERSION: u32 = 1;

/// Output container persisted with each queue item.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueueFormat {
    #[default]
    Matroska,
    Mp4,
}

impl std::fmt::Display for QueueFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Matroska => formatter.write_str("matroska"),
            Self::Mp4 => formatter.write_str("mp4"),
        }
    }
}

/// User selections stored independently of frontend widget state.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct QueueSelection {
    pub(crate) audio_locales: Vec<String>,
    pub(crate) all_audio: bool,
    pub(crate) subtitle_locales: Vec<String>,
    pub(crate) no_subtitles: bool,
    pub(crate) max_height: Option<u32>,
    pub(crate) replace: bool,
    pub(crate) no_chapters: bool,
    pub(crate) format: QueueFormat,
}

/// Durable lifecycle state for one media target.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum QueueState {
    Pending,
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for QueueState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => formatter.write_str("pending"),
            Self::Running => formatter.write_str("downloading"),
            Self::Completed => formatter.write_str("completed"),
            Self::Failed => formatter.write_str("failed"),
        }
    }
}

/// One durable download job.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueItem {
    pub(crate) id: uuid::Uuid,
    pub(crate) target: crunchydl::MediaTarget,
    #[serde(default)]
    pub(crate) title: Option<String>,
    pub(crate) selection: QueueSelection,
    pub(crate) state: QueueState,
    pub(crate) attempts: u32,
    pub(crate) output: Option<PathBuf>,
    pub(crate) failure: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    items: Vec<QueueItem>,
}

/// In-memory queue backed by one JSON document.
pub(crate) struct Queue {
    path: PathBuf,
    document: Document,
}

impl Queue {
    pub(crate) fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut document = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice::<Document>(&bytes).map_err(|_| Error::InvalidConfig {
                    path: path.clone(),
                    message: "invalid queue document".to_string(),
                })?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Document {
                schema_version: SCHEMA_VERSION,
                items: Vec::new(),
            },
            Err(_) => return Err(io_error("read queue", &path)),
        };
        if document.schema_version != SCHEMA_VERSION {
            return Err(Error::InvalidConfig {
                path,
                message: format!(
                    "unsupported queue schema version {}",
                    document.schema_version
                ),
            });
        }
        for item in &mut document.items {
            if item.state == QueueState::Running {
                item.state = QueueState::Pending;
                item.failure = None;
            }
        }
        Ok(Self { path, document })
    }

    pub(crate) fn add(
        &mut self,
        targets: impl IntoIterator<Item = crunchydl::MediaTarget>,
        selection: QueueSelection,
    ) -> Result<Vec<uuid::Uuid>> {
        self.add_named(targets.into_iter().map(|target| (target, None)), selection)
    }

    pub(crate) fn add_named(
        &mut self,
        targets: impl IntoIterator<Item = (crunchydl::MediaTarget, Option<String>)>,
        selection: QueueSelection,
    ) -> Result<Vec<uuid::Uuid>> {
        let mut added = Vec::new();
        for (target, title) in targets {
            let id = uuid::Uuid::new_v4();
            self.document.items.push(QueueItem {
                id,
                target,
                title,
                selection: selection.clone(),
                state: QueueState::Pending,
                attempts: 0,
                output: None,
                failure: None,
            });
            added.push(id);
        }
        self.save()?;
        Ok(added)
    }

    pub(crate) fn set_title(&mut self, id: uuid::Uuid, title: String) -> Result<()> {
        let item = self.item_mut(id)?;
        if item.title.as_deref() == Some(&title) {
            return Ok(());
        }
        item.title = Some(title);
        self.save()
    }

    pub(crate) fn pending(&self) -> Vec<QueueItem> {
        self.document
            .items
            .iter()
            .filter(|item| item.state == QueueState::Pending)
            .cloned()
            .collect()
    }

    pub(crate) fn items(&self) -> &[QueueItem] {
        &self.document.items
    }

    pub(crate) fn mark_running(&mut self, id: uuid::Uuid) -> Result<()> {
        let item = self.item_mut(id)?;
        item.state = QueueState::Running;
        item.attempts = item.attempts.saturating_add(1);
        item.failure = None;
        self.save()
    }

    pub(crate) fn mark_completed(&mut self, id: uuid::Uuid, output: PathBuf) -> Result<()> {
        let item = self.item_mut(id)?;
        item.state = QueueState::Completed;
        item.output = Some(output);
        item.failure = None;
        self.save()
    }

    pub(crate) fn mark_failed(&mut self, id: uuid::Uuid, failure: &str) -> Result<()> {
        let item = self.item_mut(id)?;
        item.state = QueueState::Failed;
        item.failure = Some(failure.to_string());
        self.save()
    }

    pub(crate) fn mark_pending(&mut self, id: uuid::Uuid) -> Result<()> {
        let item = self.item_mut(id)?;
        item.state = QueueState::Pending;
        item.failure = None;
        self.save()
    }

    pub(crate) fn retry_failed(&mut self) -> Result<usize> {
        let mut count = 0;
        for item in &mut self.document.items {
            if item.state == QueueState::Failed {
                item.state = QueueState::Pending;
                item.failure = None;
                count += 1;
            }
        }
        self.save()?;
        Ok(count)
    }

    pub(crate) fn retry(&mut self, id: uuid::Uuid) -> Result<bool> {
        let item = self.item_mut(id)?;
        if item.state != QueueState::Failed {
            return Ok(false);
        }
        item.state = QueueState::Pending;
        item.failure = None;
        self.save()?;
        Ok(true)
    }

    pub(crate) fn remove(&mut self, id: uuid::Uuid) -> Result<bool> {
        let previous = self.document.items.len();
        self.document
            .items
            .retain(|item| item.id != id || item.state == QueueState::Running);
        let removed = previous != self.document.items.len();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    pub(crate) fn clear_completed(&mut self) -> Result<usize> {
        let previous = self.document.items.len();
        self.document
            .items
            .retain(|item| item.state != QueueState::Completed);
        let removed = previous - self.document.items.len();
        self.save()?;
        Ok(removed)
    }

    fn item_mut(&mut self, id: uuid::Uuid) -> Result<&mut QueueItem> {
        self.document
            .items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| Error::InvalidTarget(format!("queue item {id} does not exist")))
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.document)
            .map_err(|_| io_error("serialize queue", &self.path))?;
        atomic_write(&self.path, &bytes)
    }
}

fn io_error(operation: &'static str, path: &Path) -> Error {
    Error::Filesystem {
        operation,
        path: path.to_path_buf(),
    }
}
