//! Full-screen Ratatui frontend for discovery, selection, queue, and settings.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, List, ListItem, ListState, Padding, Paragraph, Wrap,
};
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tokio::sync::mpsc;

use crate::config::{Config, DrmBackend};
use crate::error::{Error, Result};
use crate::paths::AppPaths;
use crate::presentation::{
    AccountSummary, account_summary, ellipsize, human_bytes, kind_label, locale_label_from_code,
    locale_name, safe_failure, selection_label, yes_no,
};
use crate::queue::{Queue, QueueFormat, QueueItem, QueueSelection, QueueState};

const SEARCH_DEBOUNCE: Duration = Duration::from_millis(300);
const ACCENT: Color = Color::Rgb(92, 200, 255);
const SURFACE: Color = Color::Rgb(35, 39, 52);
const MUTED: Color = Color::Rgb(130, 138, 160);
const SUCCESS: Color = Color::Rgb(100, 210, 140);
const WARNING: Color = Color::Rgb(245, 190, 80);
const DANGER: Color = Color::Rgb(245, 105, 120);

type Backend = CrosstermBackend<io::Stdout>;

struct TerminalSession {
    terminal: Terminal<Backend>,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        enable_raw_mode().map_err(|_| Error::TerminalInput)?;
        let mut stdout = io::stdout();
        if execute!(stdout, EnterAlternateScreen).is_err() {
            let _ = disable_raw_mode();
            return Err(Error::TerminalInput);
        }
        let terminal =
            Terminal::new(CrosstermBackend::new(stdout)).map_err(|_| Error::TerminalInput)?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Search,
    Browse,
    Selection,
    Queue,
    Settings,
    Account,
    Help,
}

#[derive(Clone, Copy, Debug)]
enum NoticeKind {
    Info,
    Success,
    Warning,
    Error,
}

struct Notice {
    kind: NoticeKind,
    text: String,
}

impl Notice {
    fn info(text: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            text: text.into(),
        }
    }
}

enum Message {
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
enum SelectionSource {
    Media(crunchydl::MediaTarget),
    Collection(crunchydl::CollectionTarget),
}

struct Selection {
    source: Option<SelectionSource>,
    title: String,
    capabilities: Option<crunchydl::MediaCapabilities>,
    catalog_audio: Vec<String>,
    catalog_subtitles: Vec<String>,
    audio_index: usize,
    subtitle_index: usize,
    quality_index: usize,
    format: QueueFormat,
    chapters: bool,
    include_specials: bool,
    replace: bool,
    loading: bool,
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
    fn is_collection(&self) -> bool {
        matches!(self.source, Some(SelectionSource::Collection(_)))
    }

    fn audio_choices(&self) -> Vec<String> {
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

    fn subtitle_choices(&self) -> Vec<String> {
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

    fn quality_choices(&self) -> Vec<u32> {
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

    fn queue_selection(&self) -> QueueSelection {
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
enum SettingsField {
    OutputDirectory,
    Filename,
    FolderLayout,
    DrmBackend,
    DrmDevice,
    LicenseEndpoint,
}

impl SettingsField {
    const ALL: [Self; 6] = [
        Self::OutputDirectory,
        Self::Filename,
        Self::FolderLayout,
        Self::DrmBackend,
        Self::DrmDevice,
        Self::LicenseEndpoint,
    ];

    fn label(self) -> &'static str {
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

enum Confirmation {
    Remove(uuid::Uuid),
    ClearCompleted,
    Logout,
}

#[derive(Default)]
struct DownloadProgress {
    label: String,
    detail: String,
    completed: u64,
    total: u64,
}

impl DownloadProgress {
    fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.completed as f64 / self.total as f64).clamp(0.0, 1.0)
        }
    }
}

struct App {
    client: crunchyroll_rs::Crunchyroll,
    account: AccountSummary,
    config: Config,
    paths: AppPaths,
    screen: Screen,
    previous_screen: Screen,
    query: String,
    items: Vec<crunchydl::CatalogItem>,
    selected: usize,
    queue_items: Vec<QueueItem>,
    queue_selected: usize,
    settings_selected: usize,
    settings_editing: Option<SettingsField>,
    edit_buffer: String,
    confirmation: Option<Confirmation>,
    notice: Notice,
    search_deadline: Option<Instant>,
    generation: u64,
    sender: mpsc::UnboundedSender<Message>,
    selection: Selection,
    browse_parents: Vec<(Vec<crunchydl::CatalogItem>, usize)>,
    queue_running: bool,
    queue_cancellation: Option<crunchydl::CancellationToken>,
    progress: DownloadProgress,
    picker: Picker,
    thumbnail: Option<(String, StatefulProtocol)>,
    thumbnail_loading: Option<String>,
    image_client: reqwest::Client,
    should_quit: bool,
}

impl App {
    fn set_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Notice {
            kind,
            text: text.into(),
        };
    }

    fn current(&self) -> Option<&crunchydl::CatalogItem> {
        self.items.get(self.selected)
    }

    fn current_queue(&self) -> Option<&QueueItem> {
        self.queue_items.get(self.queue_selected)
    }

    fn reload_queue(&mut self) -> Result<()> {
        self.queue_items = Queue::load(&self.paths.queue)?.items().to_vec();
        self.queue_selected = self
            .queue_selected
            .min(self.queue_items.len().saturating_sub(1));
        Ok(())
    }

    fn show(&mut self, screen: Screen) {
        self.previous_screen = self.screen;
        self.screen = screen;
        if screen == Screen::Queue
            && let Err(error) = self.reload_queue()
        {
            self.set_notice(NoticeKind::Error, error.to_string());
        }
    }

    fn schedule_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.search_deadline = if self.query.trim().is_empty() {
            self.items.clear();
            self.selected = 0;
            self.set_notice(
                NoticeKind::Info,
                "Start typing to search the Crunchyroll catalog",
            );
            None
        } else {
            self.set_notice(NoticeKind::Info, "Waiting for input…");
            Some(Instant::now() + SEARCH_DEBOUNCE)
        };
    }

    fn start_search(&mut self) {
        let client = self.client.clone();
        let query = self.query.clone();
        let generation = self.generation;
        let sender = self.sender.clone();
        self.search_deadline = None;
        self.set_notice(NoticeKind::Info, "Searching…");
        tokio::spawn(async move {
            let result = crate::catalog::search(&client, &query, 40).await;
            let _ = sender.send(Message::Search { generation, result });
        });
    }

    fn request_thumbnail(&mut self) {
        let source = self
            .current()
            .and_then(crunchydl::CatalogItem::best_artwork)
            .map(|image| image.source.clone());
        let Some(source) = source else {
            self.thumbnail = None;
            self.thumbnail_loading = None;
            return;
        };
        if self
            .thumbnail
            .as_ref()
            .is_some_and(|(current, _)| current == &source)
            || self.thumbnail_loading.as_ref() == Some(&source)
        {
            return;
        }
        self.thumbnail = None;
        self.thumbnail_loading = Some(source.clone());
        let client = self.image_client.clone();
        let cache = self.paths.thumbnail_cache.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            match crate::thumbnail::load(&client, &cache, &source).await {
                Ok(image) => {
                    let _ = sender.send(Message::Thumbnail { source, image });
                }
                Err(()) => {
                    let _ = sender.send(Message::ThumbnailFailed(source));
                }
            }
        });
    }

    fn open_current(&mut self) {
        let Some(item) = self.current().cloned() else {
            return;
        };
        match item.kind {
            crunchydl::CatalogKind::Series
            | crunchydl::CatalogKind::Season
            | crunchydl::CatalogKind::MovieListing => {
                let client = self.client.clone();
                let sender = self.sender.clone();
                self.set_notice(NoticeKind::Info, format!("Loading {}…", item.title));
                tokio::spawn(async move {
                    let _ = sender.send(Message::Children(
                        crate::catalog::children(&client, &item).await,
                    ));
                });
            }
            _ => self.configure_current(),
        }
    }

    fn configure_current(&mut self) {
        let Some(item) = self.current().cloned() else {
            return;
        };
        let return_screen = self.screen;
        if let Some(target) = item.target {
            let client = self.client.clone();
            let sender = self.sender.clone();
            self.selection = Selection {
                source: Some(SelectionSource::Media(target.clone())),
                title: item.title,
                loading: true,
                ..Selection::default()
            };
            self.previous_screen = return_screen;
            self.screen = Screen::Selection;
            self.set_notice(NoticeKind::Info, "Inspecting available tracks…");
            tokio::spawn(async move {
                let downloader = crunchydl::Downloader::builder(client).build();
                let result = async {
                    let media = downloader.resolve_target(&target).await?;
                    downloader
                        .inspect(&media, &crunchydl::CancellationToken::new())
                        .await
                }
                .await
                .map_err(Error::Download);
                let _ = sender.send(Message::Capabilities(result));
            });
            return;
        }
        let source = match item.kind {
            crunchydl::CatalogKind::Series => crunchydl::CollectionTarget::Series(item.id.clone()),
            crunchydl::CatalogKind::Season => crunchydl::CollectionTarget::Season(item.id.clone()),
            crunchydl::CatalogKind::MovieListing => {
                crunchydl::CollectionTarget::MovieListing(item.id.clone())
            }
            _ => return,
        };
        self.selection = Selection {
            source: Some(SelectionSource::Collection(source)),
            title: item.title,
            catalog_audio: item
                .audio_locales
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            catalog_subtitles: item
                .subtitle_locales
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            ..Selection::default()
        };
        self.previous_screen = return_screen;
        self.screen = Screen::Selection;
        self.set_notice(
            NoticeKind::Info,
            "Choose defaults that will be applied to every item in this batch",
        );
    }

    fn add_selection_to_queue(&mut self) -> Result<()> {
        if self.selection.loading {
            return Ok(());
        }
        let source = self
            .selection
            .source
            .clone()
            .ok_or_else(|| Error::InvalidTarget("no selected media".to_string()))?;
        let selection = self.selection.queue_selection();
        match source {
            SelectionSource::Media(target) => {
                Queue::load(&self.paths.queue)?
                    .add_named([(target, Some(self.selection.title.clone()))], selection)?;
                self.show(Screen::Queue);
                self.set_notice(NoticeKind::Success, "Added to the download queue");
            }
            SelectionSource::Collection(collection) => {
                let client = self.client.clone();
                let sender = self.sender.clone();
                let options = crunchydl::BatchOptions {
                    include_specials: self.selection.include_specials,
                    season_numbers: Vec::new(),
                };
                self.set_notice(NoticeKind::Info, "Expanding collection…");
                tokio::spawn(async move {
                    let downloader = crunchydl::Downloader::builder(client).build();
                    let result = downloader
                        .expand_collection(&collection, &options)
                        .await
                        .map_err(Error::Download);
                    let _ = sender.send(Message::Expanded { result, selection });
                });
            }
        }
        Ok(())
    }

    fn start_queue(&mut self) {
        if self.queue_running {
            return;
        }
        let paths = self.paths.clone();
        let sender = self.sender.clone();
        let event_sender = sender.clone();
        let cancellation = crunchydl::CancellationToken::new();
        let runner_cancellation = cancellation.clone();
        let sink: std::sync::Arc<dyn crunchydl::EventSink> = std::sync::Arc::new(move |event| {
            let _ = event_sender.send(Message::DownloadEvent(event));
        });
        self.queue_running = true;
        self.queue_cancellation = Some(cancellation);
        self.progress = DownloadProgress::default();
        self.set_notice(NoticeKind::Info, "Starting pending downloads…");
        tokio::spawn(async move {
            let result = crate::command::run_queue_with_sink(&paths, sink, runner_cancellation)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(Message::QueueFinished(result));
        });
    }

    fn retry_selected(&mut self) -> Result<()> {
        let Some(item) = self.current_queue() else {
            return Ok(());
        };
        let id = item.id;
        if Queue::load(&self.paths.queue)?.retry(id)? {
            self.reload_queue()?;
            self.set_notice(NoticeKind::Success, "Moved item back to pending");
        } else {
            self.set_notice(NoticeKind::Warning, "Only failed items can be retried");
        }
        Ok(())
    }

    fn retry_all(&mut self) -> Result<()> {
        let count = Queue::load(&self.paths.queue)?.retry_failed()?;
        self.reload_queue()?;
        self.set_notice(
            NoticeKind::Success,
            format!("Moved {count} failed item(s) back to pending"),
        );
        Ok(())
    }

    fn remove_selected(&mut self) {
        if let Some(item) = self.current_queue() {
            if item.state == QueueState::Running {
                self.set_notice(
                    NoticeKind::Warning,
                    "Cancel the active download before removing it",
                );
            } else {
                self.confirmation = Some(Confirmation::Remove(item.id));
            }
        }
    }

    fn confirm(&mut self) -> Result<()> {
        let Some(action) = self.confirmation.take() else {
            return Ok(());
        };
        match action {
            Confirmation::Remove(id) => {
                Queue::load(&self.paths.queue)?.remove(id)?;
                self.reload_queue()?;
                self.set_notice(NoticeKind::Success, "Removed queue item");
            }
            Confirmation::ClearCompleted => {
                let count = Queue::load(&self.paths.queue)?.clear_completed()?;
                self.reload_queue()?;
                self.set_notice(
                    NoticeKind::Success,
                    format!("Removed {count} completed item(s)"),
                );
            }
            Confirmation::Logout => {
                let paths = self.paths.clone();
                let sender = self.sender.clone();
                self.set_notice(NoticeKind::Info, "Signing out…");
                tokio::spawn(async move {
                    let _ = sender.send(Message::LoggedOut(crate::auth::logout(&paths).await));
                });
            }
        }
        Ok(())
    }

    fn begin_setting_edit(&mut self) {
        let field = SettingsField::ALL[self.settings_selected];
        if matches!(field, SettingsField::DrmBackend) {
            self.config.drm_backend = match self.config.drm_backend {
                DrmBackend::Auto => DrmBackend::PlayReady,
                DrmBackend::PlayReady => DrmBackend::Widevine,
                DrmBackend::Widevine => DrmBackend::Auto,
            };
            self.save_config();
            return;
        }
        self.edit_buffer = match field {
            SettingsField::OutputDirectory => self.config.output_dir.display().to_string(),
            SettingsField::Filename => self.config.filename.clone(),
            SettingsField::FolderLayout => self.config.output_layout.clone().unwrap_or_default(),
            SettingsField::DrmDevice => self
                .config
                .drm_device
                .as_deref()
                .map_or_else(String::new, |path| path.display().to_string()),
            SettingsField::LicenseEndpoint => String::new(),
            SettingsField::DrmBackend => unreachable!(),
        };
        self.settings_editing = Some(field);
    }

    fn apply_setting_edit(&mut self) {
        let Some(field) = self.settings_editing.take() else {
            return;
        };
        let value = self.edit_buffer.trim().to_string();
        let validation = match field {
            SettingsField::OutputDirectory if value.is_empty() => {
                Err("Output directory cannot be empty")
            }
            SettingsField::Filename => crunchydl::FilenameTemplate::compile(&value)
                .map(|_| ())
                .map_err(|_| "Filename template is invalid"),
            SettingsField::FolderLayout if !value.is_empty() => {
                crunchydl::OutputLayoutTemplate::compile(&value)
                    .map(|_| ())
                    .map_err(|_| "Folder layout template is invalid")
            }
            _ => Ok(()),
        };
        if let Err(message) = validation {
            self.settings_editing = Some(field);
            self.set_notice(NoticeKind::Error, message);
            return;
        }
        match field {
            SettingsField::OutputDirectory => self.config.output_dir = PathBuf::from(value),
            SettingsField::Filename => self.config.filename = value,
            SettingsField::FolderLayout => {
                self.config.output_layout = (!value.is_empty()).then_some(value);
            }
            SettingsField::DrmDevice => {
                self.config.drm_device = (!value.is_empty()).then(|| PathBuf::from(value));
            }
            SettingsField::LicenseEndpoint => {
                self.config.license_endpoint = (!value.is_empty()).then_some(value);
            }
            SettingsField::DrmBackend => {}
        }
        self.edit_buffer.clear();
        self.save_config();
    }

    fn save_config(&mut self) {
        match self.config.save(&self.paths) {
            Ok(()) => self.set_notice(NoticeKind::Success, "Configuration saved"),
            Err(error) => self.set_notice(NoticeKind::Error, error.to_string()),
        }
    }
}

pub(crate) async fn run(paths: &AppPaths) -> Result<()> {
    let client = match crate::auth::restore(paths).await {
        Ok(client) => client,
        Err(Error::NotLoggedIn | Error::SessionExpired) => login_before_tui(paths).await?,
        Err(error) => return Err(error),
    };
    let account = account_summary(&client).await;
    let config = Config::load(paths)?;
    let queue_items = Queue::load(&paths.queue)?.items().to_vec();
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut app = App {
        client,
        account,
        config,
        paths: paths.clone(),
        screen: Screen::Search,
        previous_screen: Screen::Search,
        query: String::new(),
        items: Vec::new(),
        selected: 0,
        queue_items,
        queue_selected: 0,
        settings_selected: 0,
        settings_editing: None,
        edit_buffer: String::new(),
        confirmation: None,
        notice: Notice::info("Start typing to search the Crunchyroll catalog"),
        search_deadline: None,
        generation: 0,
        sender,
        selection: Selection::default(),
        browse_parents: Vec::new(),
        queue_running: false,
        queue_cancellation: None,
        progress: DownloadProgress::default(),
        picker,
        thumbnail: None,
        thumbnail_loading: None,
        image_client: reqwest::Client::new(),
        should_quit: false,
    };
    let mut session = TerminalSession::enter()?;
    let mut events = EventStream::new();
    while !app.should_quit {
        session
            .terminal
            .draw(|frame| draw(frame, &mut app))
            .map_err(|_| Error::TerminalInput)?;
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind == crossterm::event::KeyEventKind::Press => {
                    handle_key(&mut app, key)?;
                }
                Some(Ok(Event::Resize(_, _))) | Some(Ok(_)) => {}
                Some(Err(_)) | None => return Err(Error::TerminalInput),
            },
            Some(message) = receiver.recv() => handle_message(&mut app, message),
            _ = tokio::time::sleep(search_wakeup(&app)) => {
                if app.search_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    app.start_search();
                }
            }
        }
    }
    Ok(())
}

fn search_wakeup(app: &App) -> Duration {
    app.search_deadline
        .map_or(Duration::from_secs(3600), |deadline| {
            deadline.saturating_duration_since(Instant::now())
        })
}

async fn login_before_tui(paths: &AppPaths) -> Result<crunchyroll_rs::Crunchyroll> {
    println!("Welcome to crunchydl - sign in to continue.\n");
    print!("Email: ");
    io::Write::flush(&mut io::stdout()).map_err(|_| Error::TerminalInput)?;
    let mut email = String::new();
    io::stdin()
        .read_line(&mut email)
        .map_err(|_| Error::TerminalInput)?;
    let password = rpassword::prompt_password("Password: ")
        .map(zeroize::Zeroizing::new)
        .map_err(|_| Error::PasswordInput)?;
    crate::auth::login(paths, email.trim(), password).await
}

fn handle_message(app: &mut App, message: Message) {
    match message {
        Message::Search { generation, result } if generation == app.generation => match result {
            Ok(items) => {
                app.set_notice(
                    NoticeKind::Success,
                    format!("Found {} result(s)", items.len()),
                );
                app.items = items;
                app.selected = 0;
                app.request_thumbnail();
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::Search { .. } => {}
        Message::Children(result) => match result {
            Ok(items) if items.is_empty() => {
                app.set_notice(NoticeKind::Warning, "This collection is empty");
            }
            Ok(items) => {
                app.browse_parents.push((app.items.clone(), app.selected));
                app.items = items;
                app.selected = 0;
                app.screen = Screen::Browse;
                app.set_notice(NoticeKind::Success, format!("{} item(s)", app.items.len()));
                app.request_thumbnail();
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::Capabilities(result) => match result {
            Ok(capabilities) => {
                app.selection.loading = false;
                app.selection.capabilities = Some(capabilities);
                app.set_notice(NoticeKind::Info, "Choose tracks and press Enter to queue");
            }
            Err(error) => {
                app.selection.loading = false;
                app.set_notice(NoticeKind::Error, error.to_string());
            }
        },
        Message::Expanded { result, selection } => match result {
            Ok(targets) if targets.is_empty() => {
                app.set_notice(NoticeKind::Warning, "No playable items matched this batch");
            }
            Ok(targets) => match Queue::load(&app.paths.queue)
                .and_then(|mut queue| queue.add(targets, selection))
            {
                Ok(ids) => {
                    app.show(Screen::Queue);
                    app.set_notice(
                        NoticeKind::Success,
                        format!(
                            "Added {} batch item(s) with your selected tracks",
                            ids.len()
                        ),
                    );
                }
                Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
            },
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::DownloadEvent(event) => handle_download_event(app, event),
        Message::QueueFinished(result) => {
            app.queue_running = false;
            app.queue_cancellation = None;
            let _ = app.reload_queue();
            match result {
                Ok(()) => app.set_notice(NoticeKind::Success, "Queue finished successfully"),
                Err(error) => app.set_notice(NoticeKind::Error, error),
            }
        }
        Message::Thumbnail { source, image } if app.thumbnail_loading.as_ref() == Some(&source) => {
            app.thumbnail = Some((source, app.picker.new_resize_protocol(image)));
            app.thumbnail_loading = None;
        }
        Message::Thumbnail { .. } => {}
        Message::ThumbnailFailed(source) if app.thumbnail_loading.as_ref() == Some(&source) => {
            app.thumbnail_loading = None;
        }
        Message::ThumbnailFailed(_) => {}
        Message::LoggedOut(result) => match result {
            Ok(_) => {
                app.set_notice(NoticeKind::Success, "Signed out");
                app.should_quit = true;
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
    }
}

fn handle_download_event(app: &mut App, event: crunchydl::DownloadEvent) {
    match event {
        crunchydl::DownloadEvent::StateChanged(state) => {
            app.progress.label = job_state_label(state).to_string();
            app.progress.detail.clear();
            let _ = app.reload_queue();
        }
        crunchydl::DownloadEvent::SegmentCompleted {
            completed,
            total,
            completed_bytes,
            total_bytes,
            track,
            ..
        } => {
            app.progress.label = track.map_or_else(
                || "Downloading media".to_string(),
                |track| {
                    format!(
                        "Downloading {:?} • {}",
                        track.kind,
                        locale_name(&track.locale)
                    )
                },
            );
            if let Some(total_bytes) = total_bytes {
                app.progress.completed = completed_bytes;
                app.progress.total = total_bytes;
                app.progress.detail = format!(
                    "{} / {} • {completed}/{total} segments",
                    human_bytes(completed_bytes),
                    human_bytes(total_bytes)
                );
            } else {
                app.progress.completed = completed as u64;
                app.progress.total = total as u64;
                app.progress.detail = format!("{completed}/{total} segments");
            }
        }
        crunchydl::DownloadEvent::StageProgress {
            state,
            completed,
            total,
        } => {
            app.progress.label = job_state_label(state).to_string();
            app.progress.completed = completed as u64;
            app.progress.total = total as u64;
            app.progress.detail = format!("{completed}/{total}");
        }
        crunchydl::DownloadEvent::TransferRetry { attempt, delay, .. } => app.set_notice(
            NoticeKind::Warning,
            format!(
                "Network interrupted - retry {attempt} in {:.1}s",
                delay.as_secs_f64()
            ),
        ),
        crunchydl::DownloadEvent::Warning(warning) => {
            app.set_notice(NoticeKind::Warning, warning.to_string());
        }
        crunchydl::DownloadEvent::OutputCommitted { output } => {
            app.set_notice(NoticeKind::Success, format!("Saved {}", output.display()));
            let _ = app.reload_queue();
        }
        _ => {}
    }
}

fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.confirmation.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => app.confirm()?,
            KeyCode::Char('n') | KeyCode::Esc => app.confirmation = None,
            _ => {}
        }
        return Ok(());
    }
    if app.settings_editing.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.settings_editing = None;
                app.edit_buffer.clear();
            }
            KeyCode::Enter => app.apply_setting_edit(),
            KeyCode::Backspace => {
                app.edit_buffer.pop();
            }
            KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.edit_buffer.push(character);
            }
            _ => {}
        }
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        if let Some(cancellation) = &app.queue_cancellation {
            cancellation.cancel();
            app.set_notice(NoticeKind::Warning, "Cancelling the active download…");
        } else {
            app.should_quit = true;
        }
        return Ok(());
    }
    match key.code {
        KeyCode::F(1) => app.show(Screen::Search),
        KeyCode::F(2) => app.show(Screen::Queue),
        KeyCode::F(3) => app.show(Screen::Settings),
        KeyCode::F(4) => app.show(Screen::Account),
        KeyCode::F(5) | KeyCode::Char('?') => app.show(Screen::Help),
        _ => match app.screen {
            Screen::Search => handle_search_key(app, key),
            Screen::Browse => handle_browse_key(app, key),
            Screen::Selection => handle_selection_key(app, key)?,
            Screen::Queue => handle_queue_key(app, key)?,
            Screen::Settings => handle_settings_key(app, key),
            Screen::Account => handle_account_key(app, key),
            Screen::Help => handle_help_key(app, key),
        },
    }
    Ok(())
}

fn handle_search_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.configure_current();
        }
        KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.query.push(character);
            app.schedule_search();
        }
        KeyCode::Backspace => {
            app.query.pop();
            app.schedule_search();
        }
        KeyCode::Up => select_catalog(app, -1),
        KeyCode::Down => select_catalog(app, 1),
        KeyCode::PageUp => select_catalog(app, -8),
        KeyCode::PageDown => select_catalog(app, 8),
        KeyCode::Enter => app.open_current(),
        _ => {}
    }
}

fn handle_browse_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            if let Some((items, selected)) = app.browse_parents.pop() {
                app.items = items;
                app.selected = selected;
                app.screen = if app.browse_parents.is_empty() {
                    Screen::Search
                } else {
                    Screen::Browse
                };
                app.request_thumbnail();
            } else {
                app.screen = Screen::Search;
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.configure_current();
        }
        KeyCode::Up => select_catalog(app, -1),
        KeyCode::Down => select_catalog(app, 1),
        KeyCode::PageUp => select_catalog(app, -8),
        KeyCode::PageDown => select_catalog(app, 8),
        KeyCode::Enter => app.open_current(),
        _ => {}
    }
}

fn select_catalog(app: &mut App, delta: isize) {
    app.selected = move_index(app.selected, app.items.len(), delta);
    app.request_thumbnail();
}

fn handle_selection_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.screen = app.previous_screen,
        KeyCode::Char('a') | KeyCode::Right => {
            let count = app.selection.audio_choices().len() + 2;
            app.selection.audio_index = (app.selection.audio_index + 1) % count.max(1);
        }
        KeyCode::Char('s') => {
            let count = app.selection.subtitle_choices().len() + 2;
            app.selection.subtitle_index = (app.selection.subtitle_index + 1) % count.max(1);
        }
        KeyCode::Char('q') => {
            let count = app.selection.quality_choices().len() + 1;
            app.selection.quality_index = (app.selection.quality_index + 1) % count.max(1);
        }
        KeyCode::Char('f') => {
            app.selection.format = match app.selection.format {
                QueueFormat::Matroska => QueueFormat::Mp4,
                QueueFormat::Mp4 => QueueFormat::Matroska,
            };
            if app.selection.format == QueueFormat::Mp4 {
                app.selection.subtitle_index = 1;
                app.selection.chapters = false;
            }
        }
        KeyCode::Char('c') if app.selection.format == QueueFormat::Matroska => {
            app.selection.chapters = !app.selection.chapters;
        }
        KeyCode::Char('i') if app.selection.is_collection() => {
            app.selection.include_specials = !app.selection.include_specials;
        }
        KeyCode::Char('o') => app.selection.replace = !app.selection.replace,
        KeyCode::Enter => app.add_selection_to_queue()?,
        _ => {}
    }
    Ok(())
}

fn handle_queue_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), -1);
        }
        KeyCode::Down => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), 1);
        }
        KeyCode::PageUp => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), -8);
        }
        KeyCode::PageDown => {
            app.queue_selected = move_index(app.queue_selected, app.queue_items.len(), 8);
        }
        KeyCode::Home => app.queue_selected = 0,
        KeyCode::End => app.queue_selected = app.queue_items.len().saturating_sub(1),
        KeyCode::Char('s') if !app.queue_running => app.start_queue(),
        KeyCode::Char('x') if app.queue_running => {
            if let Some(cancellation) = &app.queue_cancellation {
                cancellation.cancel();
                app.set_notice(NoticeKind::Warning, "Cancelling active download…");
            }
        }
        KeyCode::Char('r') if !app.queue_running => app.retry_selected()?,
        KeyCode::Char('R') if !app.queue_running => app.retry_all()?,
        KeyCode::Char('c') if !app.queue_running => {
            app.confirmation = Some(Confirmation::ClearCompleted);
        }
        KeyCode::Delete | KeyCode::Char('d') if !app.queue_running => app.remove_selected(),
        _ => {}
    }
    Ok(())
}

fn handle_settings_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Up => {
            app.settings_selected = move_index(app.settings_selected, SettingsField::ALL.len(), -1);
        }
        KeyCode::Down => {
            app.settings_selected = move_index(app.settings_selected, SettingsField::ALL.len(), 1);
        }
        KeyCode::Left | KeyCode::Right
            if matches!(
                SettingsField::ALL[app.settings_selected],
                SettingsField::DrmBackend
            ) =>
        {
            app.begin_setting_edit();
        }
        KeyCode::Enter => app.begin_setting_edit(),
        _ => {}
    }
}

fn handle_account_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.show(Screen::Search),
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('l') => app.confirmation = Some(Confirmation::Logout),
        _ => {}
    }
}

fn handle_help_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => app.screen = app.previous_screen,
        KeyCode::Char('q') => app.should_quit = true,
        _ => {}
    }
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize).min(len - 1)
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(Color::Rgb(20, 23, 32))),
        area,
    );
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    draw_navigation(frame, app, layout[0]);
    match app.screen {
        Screen::Search | Screen::Browse => draw_catalog(frame, app, layout[1]),
        Screen::Selection => draw_selection(frame, app, layout[1]),
        Screen::Queue => draw_queue(frame, app, layout[1]),
        Screen::Settings => draw_settings(frame, app, layout[1]),
        Screen::Account => draw_account(frame, app, layout[1]),
        Screen::Help => draw_help(frame, layout[1]),
    }
    draw_footer(frame, app, layout[2]);
    if let Some(confirmation) = &app.confirmation {
        draw_confirmation(frame, confirmation);
    }
}

fn draw_navigation(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let queue_count = app.queue_items.len();
    let queue_title = format!("Queue {queue_count}");
    let compact = area.width < 100;
    let left = vec![
        Span::styled(
            " crunchydl ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        nav_span(
            "F1",
            if compact { "Home" } else { "Discover" },
            matches!(app.screen, Screen::Search | Screen::Browse),
        ),
        Span::raw("  "),
        nav_span("F2", &queue_title, app.screen == Screen::Queue),
        Span::raw("  "),
        nav_span(
            "F3",
            if compact { "Setup" } else { "Settings" },
            app.screen == Screen::Settings,
        ),
        Span::raw("  "),
        nav_span(
            "F4",
            if compact { "Me" } else { "Account" },
            app.screen == Screen::Account,
        ),
        Span::raw("  "),
        nav_span(
            "F5",
            if compact { "?" } else { "Help" },
            app.screen == Screen::Help,
        ),
    ];
    let premium = if app.account.premium {
        "PREMIUM"
    } else {
        "FREE"
    };
    let right = if compact {
        format!("{premium} ")
    } else {
        format!("{}  {premium} ", ellipsize(&app.account.name, 24))
    };
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(40),
            Constraint::Length(right.chars().count() as u16),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(left)).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(SURFACE)),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right)
            .alignment(Alignment::Right)
            .style(Style::default().fg(if app.account.premium { WARNING } else { MUTED }))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(SURFACE)),
            ),
        columns[1],
    );
}

fn nav_span<'a>(key: &'a str, label: &'a str, active: bool) -> Span<'a> {
    if active {
        Span::styled(
            format!(" {key} {label} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(format!("{key} {label}"), Style::default().fg(MUTED))
    }
}

fn draw_footer(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let color = match app.notice.kind {
        NoticeKind::Info => ACCENT,
        NoticeKind::Success => SUCCESS,
        NoticeKind::Warning => WARNING,
        NoticeKind::Error => DANGER,
    };
    let icon = match app.notice.kind {
        NoticeKind::Info => "●",
        NoticeKind::Success => "✓",
        NoticeKind::Warning => "!",
        NoticeKind::Error => "×",
    };
    let help = footer_help(app.screen, app.queue_running, area.width < 100);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ellipsize(
                    &app.notice.text,
                    columns[0].width.saturating_sub(5) as usize,
                ),
                Style::default().fg(color),
            ),
        ]))
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(SURFACE)),
        ),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(help)
            .alignment(Alignment::Right)
            .style(Style::default().fg(MUTED))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(SURFACE)),
            ),
        columns[1],
    );
}

fn footer_help(screen: Screen, running: bool, compact: bool) -> &'static str {
    if compact {
        return match screen {
            Screen::Search | Screen::Browse => "↑↓ Move  Enter Open  Ctrl-D Add ",
            Screen::Selection => "A/S/Q Choose  Enter Queue ",
            Screen::Queue if running => "↑↓ Scroll  X Cancel ",
            Screen::Queue => "↑↓ Scroll  S Start  R Retry  D Delete ",
            Screen::Settings => "↑↓ Move  Enter Edit ",
            Screen::Account => "L Sign out ",
            Screen::Help => "Esc Back ",
        };
    }
    match screen {
        Screen::Search | Screen::Browse => "↑↓ Navigate  Enter Open  Ctrl-D Download ",
        Screen::Selection => "A Audio  S Subtitles  Q Quality  Enter Queue ",
        Screen::Queue if running => "↑↓ Scroll  X Cancel  Ctrl-C Cancel ",
        Screen::Queue => "↑↓ Scroll  S Start  R Retry  D Delete  C Clear ",
        Screen::Settings => "↑↓ Navigate  Enter Edit  Esc Back ",
        Screen::Account => "L Sign out  Esc Back ",
        Screen::Help => "Esc Back  Ctrl-C Quit ",
    }
}

fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE))
        .padding(Padding::horizontal(1))
}

fn draw_catalog(frame: &mut ratatui::Frame<'_>, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(area);
    let prompt = if app.screen == Screen::Search {
        format!(" {}", app.query)
    } else {
        " Browsing collection".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "⌕",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(prompt, Style::default().fg(Color::White)),
            if app.query.is_empty() && app.screen == Screen::Search {
                Span::styled(
                    "Search anime, movies, and music…",
                    Style::default().fg(MUTED),
                )
            } else {
                Span::raw("")
            },
        ]))
        .block(panel(" Discover ")),
        rows[0],
    );
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);
    let list_items = app.items.iter().map(|item| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!(" {:<11} ", kind_label(item.kind)),
                Style::default()
                    .fg(kind_color(item.kind))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(item.title.clone(), Style::default().fg(Color::White)),
        ]))
    });
    let list = List::new(list_items)
        .block(panel(format!(" Results • {} ", app.items.len())))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(app.current().map(|_| app.selected));
    frame.render_stateful_widget(list, columns[0], &mut state);
    draw_catalog_details(frame, app, columns[1]);
}

fn draw_catalog_details(frame: &mut ratatui::Frame<'_>, app: &mut App, area: Rect) {
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
        .split(area);
    let artwork = panel(" Artwork ");
    let image_area = artwork.inner(right[0]);
    frame.render_widget(artwork, right[0]);
    if let Some((_, protocol)) = &mut app.thumbnail {
        frame.render_stateful_widget(StatefulImage::default(), image_area, protocol);
    } else {
        frame.render_widget(
            Paragraph::new(if app.thumbnail_loading.is_some() {
                "Loading artwork…"
            } else {
                "No artwork available"
            })
            .alignment(Alignment::Center)
            .style(Style::default().fg(MUTED)),
            image_area,
        );
    }
    let Some(item) = app.current() else {
        frame.render_widget(
            Paragraph::new("No item selected")
                .style(Style::default().fg(MUTED))
                .block(panel(" Details ")),
            right[1],
        );
        return;
    };
    let mut lines = vec![
        Line::styled(
            item.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![label("Type"), value(kind_label(item.kind))]),
        Line::from(vec![
            label("Rating"),
            value(
                &item
                    .rating
                    .as_ref()
                    .map_or_else(|| "Not rated".to_string(), rating_label),
            ),
        ]),
        Line::from(vec![label("Premium"), bool_value(item.premium_only)]),
        Line::from(vec![label("Subtitled"), bool_value(item.is_subbed)]),
        Line::from(vec![label("Dubbed"), bool_value(item.is_dubbed)]),
    ];
    if let Some(count) = item.episode_count {
        lines.push(Line::from(vec![
            label("Episodes"),
            value(&count.to_string()),
        ]));
    }
    lines.extend([
        Line::raw(""),
        Line::styled(
            item.extended_description
                .as_deref()
                .unwrap_or(&item.description)
                .to_string(),
            Style::default().fg(Color::Gray),
        ),
        Line::raw(""),
        Line::from(vec![
            label("ID"),
            Span::styled(item.id.clone(), Style::default().fg(MUTED)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Details ")),
        right[1],
    );
}

fn label(text: &str) -> Span<'static> {
    Span::styled(format!("{text:<12}"), Style::default().fg(MUTED))
}

fn value(text: &str) -> Span<'static> {
    Span::styled(text.to_string(), Style::default().fg(Color::White))
}

fn bool_value(value: bool) -> Span<'static> {
    Span::styled(
        yes_no(value).to_string(),
        Style::default().fg(if value { SUCCESS } else { MUTED }),
    )
}

fn kind_color(kind: crunchydl::CatalogKind) -> Color {
    match kind {
        crunchydl::CatalogKind::Series | crunchydl::CatalogKind::Season => ACCENT,
        crunchydl::CatalogKind::Episode => SUCCESS,
        crunchydl::CatalogKind::Movie | crunchydl::CatalogKind::MovieListing => WARNING,
        _ => Color::Magenta,
    }
}

fn draw_selection(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 84, 26);
    if app.selection.loading {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    "Inspecting playback options",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    "Checking audio versions, subtitles, video quality, and DRM metadata…",
                    Style::default().fg(Color::Gray),
                ),
                Line::raw(""),
                Line::styled(
                    "This may take a few seconds for titles with many dubs.",
                    Style::default().fg(MUTED),
                ),
            ]))
            .alignment(Alignment::Center)
            .block(panel(" Configure download ")),
            content,
        );
        return;
    }
    let audio = selection_audio_label(&app.selection);
    let subtitles = selection_subtitle_label(&app.selection);
    let quality = selection_quality_label(&app.selection);
    let mut lines = vec![
        Line::styled(
            app.selection.title.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(
            if app.selection.is_collection() {
                "These choices apply to every episode in the batch."
            } else {
                "Choose exactly what to include in the output."
            },
            Style::default().fg(MUTED),
        ),
        Line::raw(""),
        option_line("A", "Audio", &audio, ACCENT),
        option_line("S", "Subtitles", &subtitles, SUCCESS),
        option_line("Q", "Quality", &quality, WARNING),
        option_line(
            "F",
            "Container",
            &app.selection.format.to_string(),
            Color::Magenta,
        ),
        option_line("C", "Chapters", yes_no(app.selection.chapters), Color::Cyan),
        option_line(
            "O",
            "Replace existing",
            yes_no(app.selection.replace),
            DANGER,
        ),
    ];
    if app.selection.is_collection() {
        lines.push(option_line(
            "I",
            "Include specials",
            yes_no(app.selection.include_specials),
            Color::Yellow,
        ));
    }
    if app.selection.format == QueueFormat::Mp4 {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "MP4 supports AVC/AAC only; subtitles and chapters are disabled.",
                Style::default().fg(WARNING),
            ),
        ]);
    }
    lines.extend([
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Black)
                    .bg(SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Add to queue", Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled(" Esc ", Style::default().fg(Color::Black).bg(MUTED)),
            Span::styled(" Back", Style::default().fg(Color::White)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Configure download ")),
        content,
    );
}

fn option_line(key: &str, name: &str, choice: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{name:<20}"), Style::default().fg(MUTED)),
        Span::styled(
            choice.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn selection_audio_label(selection: &Selection) -> String {
    match selection.audio_index {
        0 => "Original audio".to_string(),
        1 => "All available dubs".to_string(),
        index => selection.audio_choices().get(index - 2).map_or_else(
            || "Original audio".to_string(),
            |locale| format!("{} audio", locale_label_from_code(locale)),
        ),
    }
}

fn selection_subtitle_label(selection: &Selection) -> String {
    match selection.subtitle_index {
        0 => "All available subtitles".to_string(),
        1 => "No subtitles".to_string(),
        index => selection.subtitle_choices().get(index - 2).map_or_else(
            || "All available subtitles".to_string(),
            |locale| format!("{} subtitles", locale_label_from_code(locale)),
        ),
    }
}

fn selection_quality_label(selection: &Selection) -> String {
    selection
        .quality_index
        .checked_sub(1)
        .and_then(|index| selection.quality_choices().get(index).copied())
        .map_or_else(
            || "Best available".to_string(),
            |height| format!("Up to {height}p"),
        )
}

fn draw_queue(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(if app.queue_running { 4 } else { 0 }),
        ])
        .split(area);
    draw_queue_summary(frame, app, rows[0]);
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[1]);
    let list_items = app.queue_items.iter().map(queue_list_item);
    let list = List::new(list_items)
        .block(panel(" Downloads "))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE));
    let mut state = ListState::default()
        .with_selected((!app.queue_items.is_empty()).then_some(app.queue_selected));
    frame.render_stateful_widget(list, columns[0], &mut state);
    draw_queue_detail(frame, app, columns[1]);
    if app.queue_running {
        let gauge = Gauge::default()
            .block(
                panel(format!(" {} ", app.progress.label)).title_style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .gauge_style(
                Style::default()
                    .fg(ACCENT)
                    .bg(SURFACE)
                    .add_modifier(Modifier::BOLD),
            )
            .ratio(app.progress.ratio())
            .label(app.progress.detail.clone());
        frame.render_widget(gauge, rows[2]);
    }
}

fn draw_queue_summary(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let count = |state| {
        app.queue_items
            .iter()
            .filter(|item| item.state == state)
            .count()
    };
    let line = Line::from(vec![
        Span::styled(
            " DOWNLOAD QUEUE ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        summary_badge("○", count(QueueState::Pending), "pending", MUTED),
        Span::raw("   "),
        summary_badge("●", count(QueueState::Running), "active", ACCENT),
        Span::raw("   "),
        summary_badge("✓", count(QueueState::Completed), "complete", SUCCESS),
        Span::raw("   "),
        summary_badge("×", count(QueueState::Failed), "failed", DANGER),
    ]);
    frame.render_widget(Paragraph::new(line).block(panel(" Overview ")), area);
}

fn summary_badge(icon: &str, count: usize, label: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!("{icon} {count} {label}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn queue_list_item(item: &QueueItem) -> ListItem<'static> {
    let (icon, color) = queue_state_visual(item.state);
    let title = item.title.clone().unwrap_or_else(|| {
        item.output
            .as_deref()
            .and_then(std::path::Path::file_name)
            .and_then(std::ffi::OsStr::to_str)
            .map_or_else(
                || {
                    format!(
                        "{} • {}",
                        crate::presentation::target_kind_label(&item.target),
                        item.target.id()
                    )
                },
                |name| name.to_string(),
            )
    });
    ListItem::new(vec![
        Line::from(vec![
            Span::styled(
                format!("{icon} "),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                ellipsize(&title, 72),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(selection_label(item), Style::default().fg(MUTED)),
        ]),
    ])
}

fn queue_state_visual(state: QueueState) -> (&'static str, Color) {
    match state {
        QueueState::Pending => ("○", MUTED),
        QueueState::Running => ("●", ACCENT),
        QueueState::Completed => ("✓", SUCCESS),
        QueueState::Failed => ("×", DANGER),
    }
}

fn draw_queue_detail(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let Some(item) = app.current_queue() else {
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    "Your queue is empty",
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    "Find a title in Discover and press Ctrl-D to configure it.",
                    Style::default().fg(MUTED),
                ),
            ]))
            .alignment(Alignment::Center)
            .block(panel(" Details ")),
            area,
        );
        return;
    };
    let (icon, color) = queue_state_visual(item.state);
    let mut lines = vec![
        Line::from(vec![Span::styled(
            format!(" {icon} {} ", item.state),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::raw(""),
        Line::from(vec![
            label("Type"),
            value(crate::presentation::target_kind_label(&item.target)),
        ]),
        Line::from(vec![label("Media ID"), value(item.target.id())]),
        Line::from(vec![label("Job ID"), value(&item.id.to_string())]),
        Line::from(vec![label("Attempts"), value(&item.attempts.to_string())]),
        Line::raw(""),
        Line::styled(
            "Download choices",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::styled(selection_label(item), Style::default().fg(Color::White)),
    ];
    if let Some(output) = &item.output {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "Saved to",
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                output.display().to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]);
    }
    if let Some(error) = &item.failure {
        lines.extend([
            Line::raw(""),
            Line::styled(
                "What went wrong",
                Style::default().fg(DANGER).add_modifier(Modifier::BOLD),
            ),
            Line::styled(safe_failure(error), Style::default().fg(Color::LightRed)),
            Line::raw(""),
            Line::styled("Press R to retry this item.", Style::default().fg(MUTED)),
        ]);
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Details ")),
        area,
    );
}

fn draw_settings(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 96, 28);
    let items = SettingsField::ALL.into_iter().map(|field| {
        let display = setting_value(app, field);
        ListItem::new(Line::from(vec![
            Span::styled(format!("{:<22}", field.label()), Style::default().fg(MUTED)),
            Span::styled(display, Style::default().fg(Color::White)),
        ]))
    });
    let list = List::new(items)
        .block(panel(" Settings • Enter to edit "))
        .highlight_symbol("› ")
        .highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD));
    let mut state = ListState::default().with_selected(Some(app.settings_selected));
    frame.render_stateful_widget(list, content, &mut state);
    if let Some(field) = app.settings_editing {
        let popup = centered(content, 76, 7);
        frame.render_widget(Clear, popup);
        let displayed = if matches!(field, SettingsField::LicenseEndpoint) {
            "•".repeat(app.edit_buffer.chars().count())
        } else {
            app.edit_buffer.clone()
        };
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::styled(
                    field.label(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    if displayed.is_empty() {
                        "Type a new value…".to_string()
                    } else {
                        displayed
                    },
                    Style::default().fg(Color::White),
                ),
            ]))
            .block(panel(" Edit • Enter save • Esc cancel ")),
            popup,
        );
    }
}

fn setting_value(app: &App, field: SettingsField) -> String {
    match field {
        SettingsField::OutputDirectory => app.config.output_dir.display().to_string(),
        SettingsField::Filename => app.config.filename.clone(),
        SettingsField::FolderLayout => app
            .config
            .output_layout
            .clone()
            .unwrap_or_else(|| "Disabled".to_string()),
        SettingsField::DrmBackend => app.config.drm_backend.to_string(),
        SettingsField::DrmDevice => app.config.drm_device.as_deref().map_or_else(
            || "Not configured".to_string(),
            |path| path.display().to_string(),
        ),
        SettingsField::LicenseEndpoint => {
            if app.config.license_endpoint.is_some() {
                "Custom override".to_string()
            } else {
                "Automatic".to_string()
            }
        }
    }
}

fn draw_account(frame: &mut ratatui::Frame<'_>, app: &App, area: Rect) {
    let content = centered(area, 72, 20);
    let lines = vec![
        Line::styled(
            "Crunchyroll account",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![label("Profile"), value(&app.account.name)]),
        Line::from(vec![
            label("Email"),
            value(app.account.email.as_deref().unwrap_or("Not available")),
        ]),
        Line::from(vec![label("Premium"), bool_value(app.account.premium)]),
        Line::raw(""),
        Line::styled(
            "Application data",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::from(vec![
            label("Config"),
            value(&app.paths.config.display().to_string()),
        ]),
        Line::from(vec![
            label("Queue"),
            value(&app.paths.queue.display().to_string()),
        ]),
        Line::from(vec![
            label("Archive"),
            value(&app.paths.archive.display().to_string()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " L ",
                Style::default()
                    .fg(Color::Black)
                    .bg(DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Sign out", Style::default().fg(Color::White)),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: true })
            .block(panel(" Account ")),
        content,
    );
}

fn draw_help(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let content = centered(area, 94, 30);
    let lines = vec![
        Line::styled(
            "Keyboard shortcuts",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        help_line("F1", "Discover and browse the catalog"),
        help_line("F2", "Open the scrollable download queue"),
        help_line("F3", "Edit output and DRM settings"),
        help_line("F4", "View account details or sign out"),
        help_line("F5 / ?", "Open this help screen"),
        Line::raw(""),
        Line::styled(
            "Catalog",
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD),
        ),
        help_line("Enter", "Open a collection or configure a playable item"),
        help_line("Ctrl-D", "Configure the selected item or entire collection"),
        help_line("↑ ↓ PgUp PgDn", "Navigate long result lists"),
        Line::raw(""),
        Line::styled(
            "Queue",
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        ),
        help_line("S", "Start all pending downloads"),
        help_line("R / Shift-R", "Retry selected / all failed downloads"),
        help_line("D / Delete", "Remove the selected queue item"),
        help_line("C", "Remove all completed entries"),
        help_line("X / Ctrl-C", "Cancel the active download"),
        Line::raw(""),
        help_line("Esc", "Go back"),
        help_line("Ctrl-C", "Exit when no download is active"),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).block(panel(" Help ")),
        content,
    );
}

fn help_line(key: &str, description: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<16}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), Style::default().fg(MUTED)),
    ])
}

fn draw_confirmation(frame: &mut ratatui::Frame<'_>, confirmation: &Confirmation) {
    let area = centered(frame.area(), 60, 9);
    frame.render_widget(Clear, area);
    let message = match confirmation {
        Confirmation::Remove(_) => "Remove this item from the queue?",
        Confirmation::ClearCompleted => "Remove every completed queue item?",
        Confirmation::Logout => "Sign out and remove the saved session?",
    };
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::styled(
                message,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::from(vec![
                Span::styled(
                    " Y / Enter ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(SUCCESS)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Confirm    "),
                Span::styled(" N / Esc ", Style::default().fg(Color::Black).bg(MUTED)),
                Span::raw(" Cancel"),
            ]),
        ]))
        .alignment(Alignment::Center)
        .block(panel(" Confirm action ")),
        area,
    );
}

fn centered(area: Rect, max_width: u16, max_height: u16) -> Rect {
    let width = area.width.min(max_width);
    let height = area.height.min(max_height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
    .inner(Margin {
        horizontal: 0,
        vertical: 0,
    })
}

fn rating_label(rating: &crunchydl::CatalogRating) -> String {
    match rating {
        crunchydl::CatalogRating::Stars { average, total } => total.map_or_else(
            || format!("{average:.1}/5"),
            |total| format!("{average:.1}/5 • {total} ratings"),
        ),
        crunchydl::CatalogRating::Approval { percentage, total } => total.map_or_else(
            || format!("{percentage:.0}% positive"),
            |total| format!("{percentage:.0}% positive • {total} votes"),
        ),
        _ => "Not rated".to_string(),
    }
}

fn job_state_label(state: crunchydl::JobState) -> &'static str {
    match state {
        crunchydl::JobState::Created => "Starting",
        crunchydl::JobState::ResolvingMedia => "Resolving media",
        crunchydl::JobState::OpeningPlaybackSessions => "Opening playback",
        crunchydl::JobState::PlanningTracks => "Selecting tracks",
        crunchydl::JobState::AcquiringLicenses => "Acquiring licenses",
        crunchydl::JobState::Downloading => "Downloading",
        crunchydl::JobState::Decrypting => "Decrypting",
        crunchydl::JobState::ProcessingSubtitles => "Processing subtitles",
        crunchydl::JobState::Muxing => "Building output",
        crunchydl::JobState::Verifying => "Verifying",
        crunchydl::JobState::Committing => "Saving",
        crunchydl::JobState::Completed => "Complete",
        crunchydl::JobState::Cancelled => "Cancelled",
        crunchydl::JobState::Failed => "Failed",
        _ => "Working",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lists_move_and_clamp() {
        assert_eq!(move_index(0, 20, -8), 0);
        assert_eq!(move_index(3, 20, 8), 11);
        assert_eq!(move_index(18, 20, 8), 19);
        assert_eq!(move_index(0, 0, 1), 0);
    }

    #[test]
    fn batch_selection_is_explicit_and_human_readable() {
        let selection = Selection {
            catalog_audio: vec!["en-US".to_string(), "ja-JP".to_string()],
            catalog_subtitles: vec!["en-US".to_string()],
            audio_index: 2,
            subtitle_index: 2,
            quality_index: 2,
            ..Selection::default()
        };
        let queued = selection.queue_selection();
        assert_eq!(queued.audio_locales, vec!["en-US"]);
        assert_eq!(queued.subtitle_locales, vec!["en-US"]);
        assert_eq!(queued.max_height, Some(720));
    }
}
