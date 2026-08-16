//! nix-workspace — A Nickel-powered workspace manager for Nix flakes.
//!
//! This is the standalone CLI for nix-workspace. It provides commands for
//! initializing, validating, inspecting, building, and developing with
//! nix-workspace projects without requiring users to write Nix code.
//!
//! # Commands
//!
//! - `nix-workspace init`  — Initialize a new workspace
//! - `nix-workspace check` — Validate workspace configuration against contracts
//! - `nix-workspace info`  — Show workspace structure and discovered outputs
//! - `nix-workspace build` — Build a package (delegates to `nix build`)
//! - `nix-workspace shell` — Enter a dev shell (delegates to `nix develop`)
//!
//! # Global flags
//!
//! - `--format human|json` — Output format (default: human)
//! - `--workspace-dir DIR` — Override workspace root directory
//!
//! # Exit codes
//!
//! - 0 — Success
//! - 1 — Validation errors or build failure
//! - 2 — Infrastructure error (missing tool, bad arguments, etc.)

mod commands;
mod diagnostics;
mod flake_gen;
mod nickel;
mod workspace;

use clap::Parser;
use commands::{Command, GlobalArgs};

/// A Nickel-powered workspace manager for Nix flakes.
///
/// nix-workspace replaces complex flake.nix boilerplate with validated
/// workspace.ncl configuration. It leverages Nickel's contract system
/// for clear error messages and gradual typing.
///
/// Quick start:
///   nix-workspace init my-project
///   cd my-project
///   nix-workspace check
///   nix-workspace build
#[derive(Debug, Parser)]
#[command(
    name = "nix-workspace",
    version,
    about,
    long_about = None,
    propagate_version = true,
    after_help = "See 'nix-workspace <command> --help' for more information on a specific command."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[command(flatten)]
    global: GlobalArgs,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init(ref args) => commands::init::run(args, cli.global.format),
        Command::Check(ref args) => {
            commands::check::run(args, cli.global.format, cli.global.workspace_dir.as_ref())
        }
        Command::Info(ref args) => {
            commands::info::run(args, cli.global.format, cli.global.workspace_dir.as_ref())
        }
        Command::Build(ref args) => {
            commands::build::run(args, cli.global.format, cli.global.workspace_dir.as_ref())
        }
        Command::Shell(ref args) => {
            commands::shell::run(args, cli.global.format, cli.global.workspace_dir.as_ref())
        }
    };

    if let Err(err) = result {
        // Format the error according to the output format
        match cli.global.format {
            diagnostics::OutputFormat::Json => {
                let mut report = diagnostics::DiagnosticReport::new();
                report.push(diagnostics::Diagnostic::error(
                    diagnostics::codes::TOOL_FAILED,
                    format!("{err:#}"),
                ));
                if let Ok(json) = report.format_json() {
                    eprintln!("{json}");
                } else {
                    eprintln!(
                        "{{\"diagnostics\":[{{\"code\":\"NW501\",\"severity\":\"error\",\"message\":\"{err}\"}}]}}"
                    );
                }
            }
            diagnostics::OutputFormat::Human => {
                use owo_colors::OwoColorize;
                eprintln!("{} {err:#}", "error:".red().bold());
            }
        }

        std::process::exit(2);
    }
}
