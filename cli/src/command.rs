//! Command-line parsing and headless command dispatch.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use crunchyroll_rs::{Crunchyroll, Locale};
use zeroize::Zeroizing;

use crate::config::{Config, DrmBackend};
use crate::error::{Error, Result};
use crate::paths::AppPaths;
use crate::presentation::{
    CliProgress, account_summary, catalog_rating_label, ellipsize, kind_label,
    locale_label_from_code, print_account, print_success, print_warning, queue_state_style,
    safe_failure, selection_label, yes_no,
};
use crate::queue::{Queue, QueueFormat, QueueItem, QueueSelection};

/// Download Crunchyroll media from a terminal.
#[derive(Debug, Parser)]
#[command(name = "crunchydl", version, about)]
pub(crate) struct Arguments {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Sign in and save only the rotated refresh token in the OS credential store.
    Login(LoginArguments),
    /// Delete the saved account session.
    Logout,
    /// Validate the saved login and show subscription status.
    Status,
    /// Search the live Crunchyroll catalog.
    Search(SearchArguments),
    /// Browse seasons, episodes, or movies beneath a catalog result.
    Browse(BrowseArguments),
    /// Download one playable item or an expanded collection.
    Download(DownloadArguments),
    /// Manage crash-recoverable downloads.
    Queue(QueueArguments),
    /// Inspect or update non-secret preferences.
    Config(ConfigArguments),
}

#[derive(Debug, Args)]
struct LoginArguments {
    /// Account email. Prompted when omitted.
    #[arg(long)]
    email: Option<String>,
}

#[derive(Debug, Args)]
struct SearchArguments {
    /// Search phrase.
    query: String,
    /// Maximum number of top results.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    limit: u16,
    /// Emit stable JSON view models.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum BrowseKind {
    Series,
    Season,
    MovieListing,
}

impl BrowseKind {
    fn catalog_kind(self) -> crunchydl::CatalogKind {
        match self {
            Self::Series => crunchydl::CatalogKind::Series,
            Self::Season => crunchydl::CatalogKind::Season,
            Self::MovieListing => crunchydl::CatalogKind::MovieListing,
        }
    }
}

#[derive(Debug, Args)]
struct BrowseArguments {
    /// Kind of parent returned by search or a previous browse command.
    #[arg(value_enum)]
    kind: BrowseKind,
    /// Crunchyroll series, season, or movie-listing id.
    id: String,
    /// Emit stable JSON view models.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TargetKind {
    Episode,
    Movie,
    MusicVideo,
    Season,
    Series,
    MovieListing,
}

#[derive(Debug, Args)]
struct DownloadArguments {
    /// Kind of downloadable catalog item.
    #[arg(value_enum)]
    kind: TargetKind,
    /// Crunchyroll media id.
    id: String,
    /// Audio locale. Repeat to select multiple dubs.
    #[arg(long = "audio", conflicts_with = "all_audio")]
    audio_locales: Vec<String>,
    /// Include every available dub.
    #[arg(long)]
    all_audio: bool,
    /// Subtitle locale. Repeat for multiple tracks; defaults to all.
    #[arg(long = "subtitle", conflicts_with = "no_subtitles")]
    subtitle_locales: Vec<String>,
    /// Do not embed subtitles.
    #[arg(long)]
    no_subtitles: bool,
    /// Maximum video height, such as 1080.
    #[arg(long)]
    max_height: Option<u32>,
    /// Replace an existing final output.
    #[arg(long)]
    replace: bool,
    /// Native output container. MP4 requires --no-subtitles and --no-chapters.
    #[arg(long, value_enum, default_value_t = QueueFormat::Matroska)]
    format: QueueFormat,
    /// Disable chapter markers.
    #[arg(long)]
    no_chapters: bool,
    /// Exclude specials when expanding a season or series.
    #[arg(long)]
    exclude_specials: bool,
    /// Include only these season numbers when expanding a series.
    #[arg(long = "season")]
    season_numbers: Vec<u32>,
}

#[derive(Debug, Args)]
struct QueueArguments {
    #[command(subcommand)]
    command: QueueCommand,
}

#[derive(Debug, Subcommand)]
enum QueueCommand {
    /// Expand a target and add its playable items without starting them.
    Add(DownloadArguments),
    /// Run every pending item sequentially.
    Run,
    /// Show every persisted queue item.
    List,
    /// Move one failed item, or every failed item, back to pending.
    Retry {
        /// Queue item UUID. Omit to retry every failed item.
        id: Option<uuid::Uuid>,
    },
    /// Remove one item from the queue.
    Remove {
        /// Queue item UUID shown by `queue list`.
        id: uuid::Uuid,
    },
    /// Remove completed items from queue history.
    ClearCompleted,
}

#[derive(Debug, Args)]
struct ConfigArguments {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Update one or more settings.
    Set(ConfigSetArguments),
    /// Print every application-owned path.
    Paths,
}

#[derive(Debug, Args)]
struct ConfigSetArguments {
    /// Directory beneath which completed downloads are stored.
    #[arg(long)]
    output_dir: Option<PathBuf>,
    /// Filename template used when output_layout is disabled.
    #[arg(long)]
    filename: Option<String>,
    /// Slash-separated hierarchical output template.
    #[arg(long, conflicts_with = "flat_output")]
    output_layout: Option<String>,
    /// Disable hierarchical output and use filename only.
    #[arg(long)]
    flat_output: bool,
    /// Override DRM backend detection from the device extension.
    #[arg(long, value_enum)]
    drm_backend: Option<DrmBackend>,
    /// User-supplied `.prd` or `.wvd` DRM device file.
    #[arg(long)]
    drm_device: Option<PathBuf>,
    /// Override the built-in Crunchyroll license endpoint.
    #[arg(long)]
    license_endpoint: Option<String>,
    /// Remove a saved license endpoint override and use the built-in endpoint.
    #[arg(long, conflicts_with = "license_endpoint")]
    clear_license_endpoint: bool,
}

pub(crate) async fn run(arguments: Arguments, paths: AppPaths) -> Result<()> {
    match arguments.command {
        Some(Command::Login(arguments)) => login(&paths, arguments).await,
        Some(Command::Logout) => logout(&paths).await,
        Some(Command::Status) => status(&paths).await,
        Some(Command::Search(arguments)) => search(&paths, arguments).await,
        Some(Command::Browse(arguments)) => browse(&paths, arguments).await,
        Some(Command::Download(arguments)) => download(&paths, arguments).await,
        Some(Command::Queue(arguments)) => queue(&paths, arguments).await,
        Some(Command::Config(arguments)) => config(&paths, arguments),
        None => crate::tui::run(&paths).await,
    }
}

async fn login(paths: &AppPaths, arguments: LoginArguments) -> Result<()> {
    let email = match arguments.email {
        Some(email) => email,
        None => prompt("Email: ")?,
    };
    let password = rpassword::prompt_password("Password: ")
        .map(Zeroizing::new)
        .map_err(|_| Error::PasswordInput)?;
    let client = crate::auth::login(paths, email.trim(), password).await?;
    print_account(&account_summary(&client).await);
    Ok(())
}

async fn logout(paths: &AppPaths) -> Result<()> {
    if crate::auth::logout(paths).await? {
        print_success("Signed out and removed the saved session.");
    } else {
        print_warning("No saved login was present.");
    }
    Ok(())
}

async fn status(paths: &AppPaths) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    print_account(&account_summary(&client).await);
    Ok(())
}

async fn search(paths: &AppPaths, arguments: SearchArguments) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    let items = crate::catalog::search(&client, &arguments.query, arguments.limit.into()).await?;
    if arguments.json {
        let document = serde_json::to_string_pretty(&items).map_err(|_| Error::OutputEncoding)?;
        println!("{document}");
    } else if items.is_empty() {
        print_warning("No catalog results found.");
    } else {
        println!(
            "{:<14} {:<12} {:<9} {:<18} TITLE",
            "TYPE", "ID", "PREMIUM", "RATING"
        );
        println!("{}", "─".repeat(96));
        for item in items {
            println!(
                "{:<14} {:<12} {:<9} {:<18} {}",
                kind_label(item.kind),
                item.id,
                yes_no(item.premium_only),
                catalog_rating_label(item.rating.as_ref()),
                item.title
            );
        }
        println!("\nBrowse into a series: crunchydl browse series <SERIES_ID>");
    }
    Ok(())
}

async fn browse(paths: &AppPaths, arguments: BrowseArguments) -> Result<()> {
    let client = crate::auth::restore(paths).await?;
    let items =
        crate::catalog::browse(&client, arguments.kind.catalog_kind(), &arguments.id).await?;
    if arguments.json {
        let document = serde_json::to_string_pretty(&items).map_err(|_| Error::OutputEncoding)?;
        println!("{document}");
        return Ok(());
    }
    if items.is_empty() {
        print_warning("This collection has no items.");
        return Ok(());
    }
    match arguments.kind {
        BrowseKind::Series => print_seasons(&items),
        BrowseKind::Season => print_episodes(&items),
        BrowseKind::MovieListing => print_movies(&items),
    }
    Ok(())
}

fn print_seasons(items: &[crunchydl::CatalogItem]) {
    println!("{:<8} {:<10} {:<12} TITLE", "SEASON", "EPISODES", "ID");
    println!("{}", "─".repeat(96));
    for item in items {
        println!(
            "{:<8} {:<10} {:<12} {}",
            item.season_number
                .map_or_else(|| "-".to_string(), |number| number.to_string()),
            item.episode_count
                .map_or_else(|| "-".to_string(), |count| count.to_string()),
            item.id,
            item.title
        );
        print_catalog_languages(item);
    }
    println!("\nView episodes: crunchydl browse season <SEASON_ID>");
    println!("Download a season: crunchydl download season <SEASON_ID>");
}

fn print_episodes(items: &[crunchydl::CatalogItem]) {
    println!(
        "{:<10} {:<10} {:<9} {:<12} TITLE",
        "EPISODE", "DURATION", "PREMIUM", "ID"
    );
    println!("{}", "─".repeat(96));
    for item in items {
        println!(
            "{:<10} {:<10} {:<9} {:<12} {}",
            item.episode_number.as_deref().unwrap_or("-"),
            duration_label(item.duration_millis),
            yes_no(item.premium_only),
            item.id,
            item.title
        );
        print_catalog_languages(item);
    }
    println!("\nDownload an episode: crunchydl download episode <EPISODE_ID>");
}

fn print_movies(items: &[crunchydl::CatalogItem]) {
    println!("{:<10} {:<9} {:<12} TITLE", "DURATION", "PREMIUM", "ID");
    println!("{}", "─".repeat(96));
    for item in items {
        println!(
            "{:<10} {:<9} {:<12} {}",
            duration_label(item.duration_millis),
            yes_no(item.premium_only),
            item.id,
            item.title
        );
        print_catalog_languages(item);
    }
    println!("\nDownload a movie: crunchydl download movie <MOVIE_ID>");
}

fn print_catalog_languages(item: &crunchydl::CatalogItem) {
    let audio = locale_list(&item.audio_locales);
    let subtitles = locale_list(&item.subtitle_locales);
    if audio != "not listed" || subtitles != "not listed" {
        println!("  └─ Audio: {audio} • Subtitles: {subtitles}");
    }
}

fn locale_list(locales: &[Locale]) -> String {
    if locales.is_empty() {
        return "not listed".to_string();
    }
    locales
        .iter()
        .map(ToString::to_string)
        .map(|locale| locale_label_from_code(&locale))
        .collect::<Vec<_>>()
        .join(", ")
}

fn duration_label(duration_millis: Option<u64>) -> String {
    duration_millis.map_or_else(
        || "-".to_string(),
        |millis| {
            let seconds = millis / 1_000;
            let hours = seconds / 3_600;
            let minutes = seconds % 3_600 / 60;
            let seconds = seconds % 60;
            if hours == 0 {
                format!("{minutes:02}:{seconds:02}")
            } else {
                format!("{hours}:{minutes:02}:{seconds:02}")
            }
        },
    )
}

async fn download(paths: &AppPaths, arguments: DownloadArguments) -> Result<()> {
    validate_format_arguments(&arguments)?;
    let client = crate::auth::restore(paths).await?;
    let targets = expand_targets(&client, &arguments).await?;
    let selection = QueueSelection {
        audio_locales: arguments.audio_locales,
        all_audio: arguments.all_audio,
        subtitle_locales: arguments.subtitle_locales,
        no_subtitles: arguments.no_subtitles,
        max_height: arguments.max_height,
        replace: arguments.replace,
        no_chapters: arguments.no_chapters,
        format: arguments.format,
    };
    let mut queue = Queue::load(&paths.queue)?;
    let ids = queue.add(targets, selection)?;
    run_queue(paths, Some(&ids)).await
}

async fn queue(paths: &AppPaths, arguments: QueueArguments) -> Result<()> {
    match arguments.command {
        QueueCommand::Add(arguments) => {
            validate_format_arguments(&arguments)?;
            let client = crate::auth::restore(paths).await?;
            let targets = expand_targets(&client, &arguments).await?;
            let selection = QueueSelection {
                audio_locales: arguments.audio_locales,
                all_audio: arguments.all_audio,
                subtitle_locales: arguments.subtitle_locales,
                no_subtitles: arguments.no_subtitles,
                max_height: arguments.max_height,
                replace: arguments.replace,
                no_chapters: arguments.no_chapters,
                format: arguments.format,
            };
            let mut queue = Queue::load(&paths.queue)?;
            let ids = queue.add(targets, selection)?;
            print_success(&format!(
                "Added {} item(s) to the download queue.",
                ids.len()
            ));
            Ok(())
        }
        QueueCommand::Run => run_queue(paths, None).await,
        QueueCommand::List => {
            let queue = Queue::load(&paths.queue)?;
            if queue.items().is_empty() {
                print_warning("The download queue is empty.");
            }
            let colors = std::io::IsTerminal::is_terminal(&std::io::stdout());
            println!(
                "{:<15} {:<12} {:<12} SELECTION",
                "STATUS", "TYPE", "MEDIA ID"
            );
            println!("{}", "─".repeat(92));
            for item in queue.items() {
                println!(
                    "{} {:<12} {:<12} {}",
                    queue_state_style(item.state, colors),
                    crate::presentation::target_kind_label(&item.target),
                    item.target.id(),
                    selection_label(item)
                );
                if let Some(title) = &item.title {
                    println!("  ├─ {title}");
                }
                println!("  ├─ Job ID: {}", item.id);
                if let Some(error) = &item.failure {
                    println!("  └─ {}", ellipsize(&safe_failure(error), 110));
                } else if let Some(output) = &item.output {
                    println!("  └─ {}", ellipsize(&output.display().to_string(), 110));
                }
            }
            Ok(())
        }
        QueueCommand::Retry { id } => {
            let mut queue = Queue::load(&paths.queue)?;
            if let Some(id) = id {
                if queue.retry(id)? {
                    print_success("Moved the selected item back to pending.");
                } else {
                    print_warning("That queue item is not failed, so it was left unchanged.");
                }
            } else {
                let count = queue.retry_failed()?;
                print_success(&format!("Moved {count} failed item(s) back to pending."));
            }
            Ok(())
        }
        QueueCommand::Remove { id } => {
            let mut queue = Queue::load(&paths.queue)?;
            if queue.remove(id)? {
                print_success("Removed the selected queue item.");
            } else {
                print_warning(
                    "The item was not removed; it may be downloading or no longer exist.",
                );
            }
            Ok(())
        }
        QueueCommand::ClearCompleted => {
            let mut queue = Queue::load(&paths.queue)?;
            let count = queue.clear_completed()?;
            print_success(&format!("Removed {count} completed item(s)."));
            Ok(())
        }
    }
}

async fn expand_targets(
    client: &Crunchyroll,
    arguments: &DownloadArguments,
) -> Result<Vec<crunchydl::MediaTarget>> {
    let downloader = crunchydl::Downloader::builder(client.clone()).build();
    let collection = match arguments.kind {
        TargetKind::Episode => {
            return Ok(vec![crunchydl::MediaTarget::Episode(arguments.id.clone())]);
        }
        TargetKind::Movie => {
            return Ok(vec![crunchydl::MediaTarget::Movie(arguments.id.clone())]);
        }
        TargetKind::MusicVideo => {
            return Ok(vec![crunchydl::MediaTarget::MusicVideo(
                arguments.id.clone(),
            )]);
        }
        TargetKind::Season => crunchydl::CollectionTarget::Season(arguments.id.clone()),
        TargetKind::Series => crunchydl::CollectionTarget::Series(arguments.id.clone()),
        TargetKind::MovieListing => crunchydl::CollectionTarget::MovieListing(arguments.id.clone()),
    };
    let options = crunchydl::BatchOptions {
        include_specials: !arguments.exclude_specials,
        season_numbers: arguments.season_numbers.clone(),
    };
    let targets = downloader
        .expand_collection(&collection, &options)
        .await
        .map_err(Error::Download)?;
    if targets.is_empty() {
        return Err(Error::InvalidTarget(
            "the collection contains no items matching the filters".to_string(),
        ));
    }
    Ok(targets)
}

async fn run_queue(paths: &AppPaths, only: Option<&[uuid::Uuid]>) -> Result<()> {
    let progress = Arc::new(CliProgress::new());
    let sink: Arc<dyn crunchydl::EventSink> = progress.clone();
    let cancellation = crunchydl::CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let result = run_queue_inner(
        paths,
        only,
        true,
        sink,
        cancellation,
        Some(progress.clone()),
    )
    .await;
    progress.finish();
    signal.abort();
    result
}

pub(crate) async fn run_queue_with_sink(
    paths: &AppPaths,
    sink: Arc<dyn crunchydl::EventSink>,
    cancellation: crunchydl::CancellationToken,
) -> Result<()> {
    run_queue_inner(paths, None, false, sink, cancellation, None).await
}

async fn run_queue_inner(
    paths: &AppPaths,
    only: Option<&[uuid::Uuid]>,
    terminal_output: bool,
    sink: Arc<dyn crunchydl::EventSink>,
    cancellation: crunchydl::CancellationToken,
    progress: Option<Arc<CliProgress>>,
) -> Result<()> {
    let mut queue = Queue::load(&paths.queue)?;
    let mut pending = queue.pending();
    if let Some(ids) = only {
        pending.retain(|item| ids.contains(&item.id));
    }
    if pending.is_empty() {
        if terminal_output {
            print_warning("There are no pending downloads.");
        }
        return Ok(());
    }
    let config = Config::load(paths)?;
    let client = crate::auth::restore(paths).await?;
    let downloader = downloader(&client, &config, paths, sink).await?;
    let mut failed = 0;
    let mut last_error = None;
    let total = pending.len();
    for (index, item) in pending.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(Error::Download(crunchydl::Error::Cancelled));
        }
        queue.mark_running(item.id)?;
        if let Some(progress) = &progress {
            progress.start(&item, index + 1, total);
        }
        let result = match downloader.media_request(&item.target).await {
            Ok(media) => {
                queue.set_title(item.id, media.resolve().title)?;
                download_item(&downloader, &config, &item, media, &cancellation).await
            }
            Err(error) => Err(Error::Download(error)),
        };
        match result {
            Ok(output) => {
                if let Some(progress) = &progress {
                    progress.success(&output);
                }
                queue.mark_completed(item.id, output)?;
            }
            Err(error) => {
                if cancellation.is_cancelled() {
                    queue.mark_pending(item.id)?;
                    return Err(error);
                }
                if let Some(progress) = &progress {
                    progress.failure(&error.to_string());
                }
                queue.mark_failed(item.id, &error.to_string())?;
                last_error = Some(error);
                failed += 1;
            }
        }
    }
    match (failed, last_error) {
        (0, _) => Ok(()),
        (1, Some(error)) => Err(error),
        (count, _) => Err(Error::QueueFailed(count)),
    }
}

async fn download_item(
    downloader: &crunchydl::Downloader,
    config: &Config,
    item: &QueueItem,
    media: crunchydl::MediaRequest,
    cancellation: &crunchydl::CancellationToken,
) -> Result<PathBuf> {
    let audio = if item.selection.all_audio {
        crunchydl::AudioSelection::All
    } else if item.selection.audio_locales.is_empty() {
        crunchydl::AudioSelection::Original
    } else {
        let locales = parse_locales(item.selection.audio_locales.clone())?;
        if locales.len() == 1 {
            crunchydl::AudioSelection::Locale(locales[0].clone())
        } else {
            crunchydl::AudioSelection::Locales(locales)
        }
    };
    let subtitle_locales = if item.selection.no_subtitles {
        crunchydl::SubtitleLocales::None
    } else if item.selection.subtitle_locales.is_empty() {
        crunchydl::SubtitleLocales::All
    } else {
        crunchydl::SubtitleLocales::Explicit(parse_locales(
            item.selection.subtitle_locales.clone(),
        )?)
    };
    let planning = crunchydl::PlanningOptions {
        audio,
        subtitles: crunchydl::SubtitleSelection::default().with_locales(subtitle_locales),
        video_quality: item.selection.max_height.map_or(
            crunchydl::QualitySelection::Best,
            crunchydl::QualitySelection::MaxHeight,
        ),
        chapters: if item.selection.no_chapters {
            crunchydl::ChapterSelection::Disabled
        } else {
            crunchydl::ChapterSelection::SkipEvents
        },
        ..crunchydl::PlanningOptions::default()
    };
    let mut output =
        crunchydl::OutputOptions::new(&config.output_dir).map_err(|_| Error::InvalidTemplate)?;
    output.filename = crunchydl::FilenameTemplate::compile(&config.filename)
        .map_err(|_| Error::InvalidTemplate)?;
    output.layout = config
        .output_layout
        .as_deref()
        .map(crunchydl::OutputLayoutTemplate::compile)
        .transpose()
        .map_err(|_| Error::InvalidTemplate)?;
    if item.selection.replace {
        output.overwrite = crunchydl::OverwritePolicy::Replace;
    }
    output.container = match item.selection.format {
        QueueFormat::Matroska => crunchydl::Container::Matroska,
        QueueFormat::Mp4 => crunchydl::Container::Mp4,
    };
    let request = crunchydl::DownloadRequest {
        media,
        planning,
        output,
        transfer: crunchydl::TransferOptions::default(),
        subtitles: crunchydl::SubtitleProcessingOptions::default(),
        synchronization: crunchydl::SynchronizationOptions::default(),
        cancellation: cancellation.clone(),
    };
    downloader
        .download(request)
        .await
        .map(|result| result.output)
        .map_err(Error::Download)
}

async fn downloader(
    client: &Crunchyroll,
    config: &Config,
    paths: &AppPaths,
    events: Arc<dyn crunchydl::EventSink>,
) -> Result<crunchydl::Downloader> {
    let device_path = config.drm_device.as_ref().ok_or(Error::DrmNotConfigured)?;
    let backend = config.drm_backend.resolve(device_path)?;
    let bytes = tokio::fs::read(device_path)
        .await
        .map_err(|_| Error::InvalidDrmDevice)?;
    let (provider, system): (Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem) = match backend
    {
        DrmBackend::Auto => unreachable!("auto backend was resolved above"),
        DrmBackend::PlayReady => playready_provider(&bytes)?,
        DrmBackend::Widevine => widevine_provider(&bytes)?,
    };
    let archive = Arc::new(crunchydl::JsonArchive::new(&paths.archive));
    let builder = crunchydl::Downloader::builder(client.clone());
    let builder = if let Some(endpoint) = config
        .license_endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.trim().is_empty())
    {
        builder.drm_with_license_endpoint(provider, system, endpoint)
    } else {
        builder.drm(provider, system)
    };
    Ok(builder.archive(archive).event_sink(events).build())
}

#[cfg(feature = "drm-playready")]
fn playready_provider(
    bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    let provider = crunchydl::PlayReadyProvider::from_device_bytes(bytes)
        .map_err(|_| Error::InvalidDrmDevice)?;
    Ok((Arc::new(provider), crunchydl::DrmSystem::PlayReady))
}

#[cfg(not(feature = "drm-playready"))]
fn playready_provider(
    _bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    Err(Error::DrmNotCompiled("PlayReady"))
}

#[cfg(feature = "drm-widevine")]
fn widevine_provider(
    bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    let provider = crunchydl::WidevineProvider::from_device_bytes(bytes)
        .map_err(|_| Error::InvalidDrmDevice)?;
    Ok((Arc::new(provider), crunchydl::DrmSystem::Widevine))
}

#[cfg(not(feature = "drm-widevine"))]
fn widevine_provider(
    _bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    Err(Error::DrmNotCompiled("Widevine"))
}

fn config(paths: &AppPaths, arguments: ConfigArguments) -> Result<()> {
    match arguments.command {
        Some(ConfigCommand::Set(arguments)) => {
            let mut config = Config::load(paths)?;
            if let Some(output_dir) = arguments.output_dir {
                config.output_dir = output_dir;
            }
            if let Some(filename) = arguments.filename {
                crunchydl::FilenameTemplate::compile(&filename)
                    .map_err(|_| Error::InvalidTemplate)?;
                config.filename = filename;
            }
            if let Some(layout) = arguments.output_layout {
                crunchydl::OutputLayoutTemplate::compile(&layout)
                    .map_err(|_| Error::InvalidTemplate)?;
                config.output_layout = Some(layout);
            } else if arguments.flat_output {
                config.output_layout = None;
            }
            if let Some(backend) = arguments.drm_backend {
                config.drm_backend = backend;
            }
            if let Some(device) = arguments.drm_device {
                config.drm_device = Some(device);
            }
            if let Some(endpoint) = arguments.license_endpoint {
                config.license_endpoint = Some(endpoint);
            } else if arguments.clear_license_endpoint {
                config.license_endpoint = None;
            }
            config.save(paths)?;
            print_success(&format!(
                "Configuration saved to {}.",
                paths.config.display()
            ));
            Ok(())
        }
        Some(ConfigCommand::Paths) => {
            println!("{:<13} PATH", "DATA");
            println!("{}", "─".repeat(72));
            println!("{:<13} {}", "Config", paths.config.display());
            println!("{:<13} {}", "Session", paths.session.display());
            println!("{:<13} {}", "Archive", paths.archive.display());
            println!("{:<13} {}", "Queue", paths.queue.display());
            println!("{:<13} {}", "Thumbnails", paths.thumbnail_cache.display());
            Ok(())
        }
        None => {
            let config = Config::load(paths)?;
            println!("crunchydl configuration");
            println!("{}", "─".repeat(72));
            println!("{:<18} {}", "Output directory", config.output_dir.display());
            println!("{:<18} {}", "Filename", config.filename);
            println!(
                "{:<18} {}",
                "Folder layout",
                config.output_layout.as_deref().unwrap_or("<disabled>")
            );
            println!("{:<18} {}", "DRM backend", config.drm_backend);
            println!(
                "{:<18} {}",
                "DRM device",
                config
                    .drm_device
                    .as_deref()
                    .map_or_else(|| "<unset>".to_string(), |path| path.display().to_string())
            );
            println!(
                "{:<18} {}",
                "License endpoint",
                if config.license_endpoint.is_some() {
                    "<custom override>"
                } else {
                    "<automatic>"
                }
            );
            Ok(())
        }
    }
}

fn parse_locales(values: Vec<String>) -> Result<Vec<Locale>> {
    values
        .into_iter()
        .map(|value| {
            let locale = Locale::from(value.clone());
            if matches!(locale, Locale::Custom(_)) {
                Err(Error::InvalidLocale(value))
            } else {
                Ok(locale)
            }
        })
        .collect()
}

fn validate_format_arguments(arguments: &DownloadArguments) -> Result<()> {
    if matches!(arguments.format, QueueFormat::Mp4)
        && (!arguments.no_subtitles || !arguments.no_chapters)
    {
        return Err(Error::InvalidTarget(
            "MP4 currently preserves AVC/AAC only; pass --no-subtitles and --no-chapters explicitly, or use Matroska"
                .to_string(),
        ));
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush().map_err(|_| Error::TerminalInput)?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|_| Error::TerminalInput)?;
    Ok(value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_hierarchy_has_explicit_parent_kinds() {
        let arguments =
            Arguments::try_parse_from(["crunchydl", "browse", "season", "G00000000", "--json"])
                .expect("browse command");
        assert!(matches!(
            arguments.command,
            Some(Command::Browse(BrowseArguments {
                kind: BrowseKind::Season,
                id,
                json: true,
            })) if id == "G00000000"
        ));
    }

    #[test]
    fn locale_parser_accepts_every_known_enum_variant() {
        assert_eq!(
            parse_locales(vec!["zh-HK".into()]).unwrap(),
            vec![Locale::zh_HK]
        );
        assert!(parse_locales(vec!["not-a-locale".into()]).is_err());
    }

    #[test]
    fn durations_are_compact_and_aligned() {
        assert_eq!(duration_label(Some(1_445_000)), "24:05");
        assert_eq!(duration_label(Some(3_661_000)), "1:01:01");
        assert_eq!(duration_label(None), "-");
    }
}
