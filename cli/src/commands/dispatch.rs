//! Headless command dispatch and command handlers.

use super::args::{
    Arguments, BrowseArguments, BrowseKind, Command, ConfigArguments, ConfigCommand,
    DownloadArguments, LoginArguments, QueueArguments, QueueCommand, SearchArguments, TargetKind,
};

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;

use crunchyroll_rs::{Crunchyroll, Locale};
use zeroize::Zeroizing;

use crate::config::{Config, DrmBackend};
use crate::error::{Error, Result};
use crate::paths::AppPaths;
use crate::presentation::{
    CliProgress, account_summary, catalog_rating_label, ellipsize, ellipsize_middle, kind_label,
    locale_name, paint, print_account, print_success, print_warning, queue_state_style,
    safe_failure, selection_label,
};
use crate::queue::{Queue, QueueFormat, QueueItem, QueueSelection, QueueState};

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

mod catalog;
mod config;
mod download;
mod drm;
mod queue_runner;

pub(crate) use catalog::{
    browse, login, logout, print_actions, print_list_heading, search, status, terminal_width,
};
pub(crate) use config::{config, parse_locales, prompt, validate_format_arguments};
pub(crate) use download::{compact_path, download, queue};
pub(crate) use drm::downloader;
pub(crate) use queue_runner::{run_queue, run_queue_with_sink};
