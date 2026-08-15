//! `completion` and `man`: generated from the clap definition itself, so
//! they cannot drift from the commands that actually exist.

use anyhow::Result;
use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs, Shell};

pub fn completion(args: CompletionArgs) -> Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    let mut out = std::io::stdout();
    match args.shell {
        // Nushell is not one of clap_complete's built-in shells; it lives in
        // its own crate. This repo standardises on nushell, so it would be a
        // strange omission here.
        Shell::Nushell => {
            clap_complete::generate(clap_complete_nushell::Nushell, &mut cmd, name, &mut out)
        }
        Shell::Bash => {
            clap_complete::generate(clap_complete::Shell::Bash, &mut cmd, name, &mut out)
        }
        Shell::Zsh => clap_complete::generate(clap_complete::Shell::Zsh, &mut cmd, name, &mut out),
        Shell::Fish => {
            clap_complete::generate(clap_complete::Shell::Fish, &mut cmd, name, &mut out)
        }
        Shell::Elvish => {
            clap_complete::generate(clap_complete::Shell::Elvish, &mut cmd, name, &mut out)
        }
        Shell::Pwsh => {
            clap_complete::generate(clap_complete::Shell::PowerShell, &mut cmd, name, &mut out)
        }
    }
    Ok(())
}

pub fn man() -> Result<()> {
    let cmd = Cli::command();
    clap_mangen::Man::new(cmd).render(&mut std::io::stdout())?;
    Ok(())
}
