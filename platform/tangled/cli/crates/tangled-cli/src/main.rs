mod cli;
mod commands;
mod target;
mod util;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

// The runtime tokio::main builds expects its own construction to succeed, and
// that expansion lands inside a function returning Result.
#[allow(clippy::unwrap_in_result)]
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    util::set_profile(cli.profile.clone());
    commands::dispatch(cli).await
}
