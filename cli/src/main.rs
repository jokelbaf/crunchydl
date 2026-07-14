//! Interactive and headless Crunchyroll downloader.

mod app;
mod commands;
mod error;

pub(crate) use app::{auth, catalog, config, paths, queue, thumbnail};
pub(crate) use commands as command;
pub(crate) use ui::{presentation, tui};
mod ui;

use clap::Parser;

use crate::commands::Arguments;
use crate::error::Result;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        presentation::print_error(&error.to_string());
        std::process::exit(error.exit_code());
    }
}

async fn run() -> Result<()> {
    let arguments = Arguments::parse();
    let paths = paths::AppPaths::discover()?;
    commands::run(arguments, paths).await
}
