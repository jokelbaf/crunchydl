#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Search,
    Browse,
    Selection,
    Queue,
    Settings,
    Account,
    Help,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

pub(crate) struct Notice {
    pub(crate) kind: NoticeKind,
    pub(crate) text: String,
}

impl Notice {
    pub(crate) fn info(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            text: text.into(),
        }
    }
}

pub(crate) enum Message {
    Search {
        generation: u64,
        result: Result<Vec<crunchydl::CatalogItem>>,
    },
    Children(Result<Vec<crunchydl::CatalogItem>>),
    Capabilities(Result<crunchydl::MediaCapabilities>),
    Expanded {
        result: Result<Vec<crunchydl::MediaTarget>>,
        selection: QueueSelection,
    },
    DownloadEvent(crunchydl::DownloadEvent),
    QueueFinished(std::result::Result<(), String>),
    Thumbnail {
        source: String,
        image: image::DynamicImage,
    },
    ThumbnailFailed(String),
    LoggedOut(Result<bool>),
}

#[derive(Clone)]
pub(crate) enum SelectionSource {
    Media(crunchydl::MediaTarget),
    Collection(crunchydl::CollectionTarget),
}

pub(crate) struct Selection {
    pub(crate) source: Option<SelectionSource>,
    pub(crate) title: String,
    pub(crate) capabilities: Option<crunchydl::MediaCapabilities>,
    pub(crate) catalog_audio: Vec<String>,
    pub(crate) catalog_subtitles: Vec<String>,
    pub(crate) audio_index: usize,
    pub(crate) subtitle_index: usize,
    pub(crate) quality_index: usize,
    pub(crate) format: QueueFormat,
    pub(crate) chapters: bool,
    pub(crate) include_specials: bool,
    pub(crate) replace: bool,
    pub(crate) loading: bool,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            source: None,
            title: String::new(),
            capabilities: None,
            catalog_audio: Vec::new(),
            catalog_subtitles: Vec::new(),
            audio_index: 0,
            subtitle_index: 0,
            quality_index: 0,
            format: QueueFormat::Matroska,
            chapters: true,
            include_specials: true,
            replace: false,
            loading: false,
        }
    }
}

impl Selection {
    pub(crate) fn is_collection(&self) -> bool {
        matches!(self.source, Some(SelectionSource::Collection(_)))
    }

    pub(crate) fn audio_choices(&self) -> Vec<String> {
        self.capabilities.as_ref().map_or_else(
            || self.catalog_audio.clone(),
            |capabilities| {
                capabilities
                    .versions
                    .iter()
                    .map(|version| version.audio_locale.to_string())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            },
        )
    }

    pub(crate) fn subtitle_choices(&self) -> Vec<String> {
        self.capabilities.as_ref().map_or_else(
            || self.catalog_subtitles.clone(),
            |capabilities| {
                capabilities
                    .versions
                    .iter()
                    .flat_map(|version| version.subtitles.iter())
                    .map(|subtitle| subtitle.locale.to_string())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            },
        )
    }

    pub(crate) fn quality_choices(&self) -> Vec<u32> {
        self.capabilities.as_ref().map_or_else(
            || vec![1080, 720, 480, 360],
            |capabilities| {
                capabilities
                    .versions
                    .iter()
                    .flat_map(|version| version.video.iter())
                    .map(|video| video.height)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            },
        )
    }

    pub(crate) fn queue_selection(&self) -> QueueSelection {
        let audio_choices = self.audio_choices();
        let subtitle_choices = self.subtitle_choices();
        let quality_choices = self.quality_choices();
        let mut selection = QueueSelection::default();
        match self.audio_index {
            0 => {}
            1 => selection.all_audio = true,
            index => {
                if let Some(locale) = audio_choices.get(index.saturating_sub(2)) {
                    selection.audio_locales.push(locale.clone());
                }
            }
        }
        match self.subtitle_index {
            0 => {}
            1 => selection.no_subtitles = true,
            index => {
                if let Some(locale) = subtitle_choices.get(index.saturating_sub(2)) {
                    selection.subtitle_locales.push(locale.clone());
                }
            }
        }
        selection.max_height = self
            .quality_index
            .checked_sub(1)
            .and_then(|index| quality_choices.get(index).copied());
        selection.format = self.format;
        selection.no_chapters = !self.chapters;
        selection.replace = self.replace;
        if selection.format == QueueFormat::Mp4 {
            selection.no_subtitles = true;
            selection.no_chapters = true;
        }
        selection
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SettingsField {
    OutputDirectory,
    Filename,
    FolderLayout,
    DrmBackend,
    DrmDevice,
    LicenseEndpoint,
}

impl SettingsField {
    pub(crate) const ALL: [Self; 6] = [
        Self::OutputDirectory,
        Self::Filename,
        Self::FolderLayout,
        Self::DrmBackend,
        Self::DrmDevice,
        Self::LicenseEndpoint,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::OutputDirectory => "Output directory",
            Self::Filename => "Filename template",
            Self::FolderLayout => "Folder layout",
            Self::DrmBackend => "DRM backend",
            Self::DrmDevice => "DRM device",
            Self::LicenseEndpoint => "License endpoint override",
        }
    }
}

pub(crate) enum Confirmation {
    Remove(uuid::Uuid),
    ClearCompleted,
    Logout,
}

#[derive(Default)]
pub(crate) struct DownloadProgress {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) completed: u64,
    pub(crate) total: u64,
}

impl DownloadProgress {
    pub(crate) fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.completed as f64 / self.total as f64).clamp(0.0, 1.0)
        }
    }
}
use super::*;
