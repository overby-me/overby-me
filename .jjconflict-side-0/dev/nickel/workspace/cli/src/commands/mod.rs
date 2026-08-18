//! CLI command implementations.
//!
//! Each subcommand lives in its own module and exposes a `run()` function
//! that takes the parsed arguments and returns a result.

pub mod build;
pub mod check;
pub mod info;
pub mod init;
pub mod shell;

use crate::diagnostics::OutputFormat;
use clap::Subcommand;
use std::path::PathBuf;

/// Top-level CLI subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a new nix-workspace project.
    ///
    /// Creates `workspace.ncl`, `flake.nix`, and convention directories
    /// with sample files. Run this in an empty directory or provide a path.
    Init(init::InitArgs),

    /// Validate workspace configuration against contracts.
    ///
    /// Evaluates `workspace.ncl` with the WorkspaceConfig contract applied
    /// and reports any validation errors. Exit code 0 means the workspace
    /// is valid; exit code 1 means there are errors.
    Check(check::CheckArgs),

    /// Show workspace structure and discovered outputs.
    ///
    /// Scans the workspace for convention directories, subworkspaces,
    /// and configuration, then displays a summary. Does not require
    /// Nickel evaluation for basic discovery; use `--eval` to also
    /// show the evaluated config.
    Info(info::InfoArgs),

    /// Build a package from the workspace.
    ///
    /// Delegates to `nix build`. If the workspace has no `flake.nix`,
    /// one is generated on-the-fly in a temporary directory.
    Build(build::BuildArgs),

    /// Enter a development shell from the workspace.
    ///
    /// Delegates to `nix develop`. If the workspace has no `flake.nix`,
    /// one is generated on-the-fly in a temporary directory.
    Shell(shell::ShellArgs),
}

/// Global CLI arguments shared across all subcommands.
#[derive(Debug, clap::Args, Clone)]
pub struct GlobalArgs {
    /// Output format for diagnostics and structured output.
    #[arg(long, value_enum, default_value = "human", global = true)]
    pub format: OutputFormat,

    /// Override the workspace root directory.
    ///
    /// By default, nix-workspace searches upward from the current directory
    /// for a `workspace.ncl` file.
    #[arg(long, value_name = "DIR", global = true)]
    pub workspace_dir: Option<PathBuf>,
}
