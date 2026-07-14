//! Interactive and headless Crunchyroll downloader.

mod auth;
mod catalog;
mod command;
mod config;
mod error;
mod paths;
mod presentation;
mod queue;
mod thumbnail;
mod tui;

use clap::Parser;

use crate::command::Arguments;
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
    command::run(arguments, paths).await
}
