pub mod api;
pub mod auth;
pub mod browse;
pub mod git_credential;
pub mod issue;
pub mod knot;
pub mod pr;
pub mod repo;
pub mod spindle;
pub mod ssh_key;
pub mod status;

use anyhow::Result;

use crate::cli::{Cli, Command};

pub async fn dispatch(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Auth(cmd) => auth::run(&cli, cmd.clone()).await,
        Command::Repo(cmd) => repo::run(&cli, cmd.clone()).await,
        Command::Issue(cmd) => issue::run(&cli, cmd.clone()).await,
        Command::Pr(cmd) => pr::run(&cli, cmd.clone()).await,
        Command::Knot(cmd) => knot::run(&cli, cmd.clone()).await,
        Command::Spindle(cmd) => spindle::run(&cli, cmd.clone()).await,
        Command::Browse(args) => browse::run(&cli, args.clone()).await,
        Command::Api(cmd) => api::run(&cli, cmd.clone()).await,
        Command::Status => status::run(&cli).await,
        Command::GitCredential(args) => git_credential::run(&cli, args.clone()).await,
        Command::SshKey(cmd) => ssh_key::run(&cli, cmd.clone()).await,
    }
}
