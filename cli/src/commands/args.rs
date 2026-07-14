//! Clap declarations kept separate from command execution.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::app::config::DrmBackend;
use crate::app::queue::QueueFormat;

/// Download Crunchyroll media from a terminal.
#[derive(Debug, Parser)]
#[command(name = "crunchydl", version, about)]
pub(crate) struct Arguments {
    #[command(subcommand)]
    pub(super) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(super) enum Command {
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
pub(super) struct LoginArguments {
    /// Account email. Prompted when omitted.
    #[arg(long)]
    pub(super) email: Option<String>,
}

#[derive(Debug, Args)]
pub(super) struct SearchArguments {
    /// Search phrase.
    pub(super) query: String,
    /// Maximum number of top results.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub(super) limit: u16,
    /// Emit stable JSON view models.
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum BrowseKind {
    Series,
    Season,
    MovieListing,
}

impl BrowseKind {
    pub(super) fn catalog_kind(self) -> crunchydl::CatalogKind {
        match self {
            Self::Series => crunchydl::CatalogKind::Series,
            Self::Season => crunchydl::CatalogKind::Season,
            Self::MovieListing => crunchydl::CatalogKind::MovieListing,
        }
    }
}

#[derive(Debug, Args)]
pub(super) struct BrowseArguments {
    /// Kind of parent returned by search or a previous browse command.
    #[arg(value_enum)]
    pub(super) kind: BrowseKind,
    /// Crunchyroll series, season, or movie-listing id.
    pub(super) id: String,
    /// Emit stable JSON view models.
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum TargetKind {
    Episode,
    Movie,
    MusicVideo,
    Season,
    Series,
    MovieListing,
}

#[derive(Debug, Args)]
pub(super) struct DownloadArguments {
    /// Kind of downloadable catalog item.
    #[arg(value_enum)]
    pub(super) kind: TargetKind,
    /// Crunchyroll media id.
    pub(super) id: String,
    /// Audio locale. Repeat to select multiple dubs.
    #[arg(long = "audio", conflicts_with = "all_audio")]
    pub(super) audio_locales: Vec<String>,
    /// Include every available dub.
    #[arg(long)]
    pub(super) all_audio: bool,
    /// Subtitle locale. Repeat for multiple tracks; defaults to all.
    #[arg(long = "subtitle", conflicts_with = "no_subtitles")]
    pub(super) subtitle_locales: Vec<String>,
    /// Do not embed subtitles.
    #[arg(long)]
    pub(super) no_subtitles: bool,
    /// Maximum video height, such as 1080.
    #[arg(long)]
    pub(super) max_height: Option<u32>,
    /// Replace an existing final output.
    #[arg(long)]
    pub(super) replace: bool,
    /// Native output container. MP4 requires --no-subtitles and --no-chapters.
    #[arg(long, value_enum, default_value_t = QueueFormat::Matroska)]
    pub(super) format: QueueFormat,
    /// Disable chapter markers.
    #[arg(long)]
    pub(super) no_chapters: bool,
    /// Exclude specials when expanding a season or series.
    #[arg(long)]
    pub(super) exclude_specials: bool,
    /// Include only these season numbers when expanding a series.
    #[arg(long = "season")]
    pub(super) season_numbers: Vec<u32>,
}

#[derive(Debug, Args)]
pub(super) struct QueueArguments {
    #[command(subcommand)]
    pub(super) command: QueueCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum QueueCommand {
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
pub(super) struct ConfigArguments {
    #[command(subcommand)]
    pub(super) command: Option<ConfigCommand>,
}

#[derive(Debug, Subcommand)]
pub(super) enum ConfigCommand {
    /// Update one or more settings.
    Set(ConfigSetArguments),
    /// Print every application-owned path.
    Paths,
}

#[derive(Debug, Args)]
pub(super) struct ConfigSetArguments {
    /// Directory beneath which completed downloads are stored.
    #[arg(long)]
    pub(super) output_dir: Option<PathBuf>,
    /// Filename template used when output_layout is disabled.
    #[arg(long)]
    pub(super) filename: Option<String>,
    /// Slash-separated hierarchical output template.
    #[arg(long, conflicts_with = "flat_output")]
    pub(super) output_layout: Option<String>,
    /// Disable hierarchical output and use filename only.
    #[arg(long)]
    pub(super) flat_output: bool,
    /// Override DRM backend detection from the device extension.
    #[arg(long, value_enum)]
    pub(super) drm_backend: Option<DrmBackend>,
    /// User-supplied `.prd` or `.wvd` DRM device file.
    #[arg(long)]
    pub(super) drm_device: Option<PathBuf>,
    /// Override the license endpoint returned by Crunchyroll playback metadata.
    #[arg(long)]
    pub(super) license_endpoint: Option<String>,
    /// Remove a saved license endpoint override and use playback metadata.
    #[arg(long, conflicts_with = "license_endpoint")]
    pub(super) clear_license_endpoint: bool,
}
