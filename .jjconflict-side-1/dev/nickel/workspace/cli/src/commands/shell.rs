#![allow(dead_code)]
//! `nix-workspace shell` — Enter a development shell from the workspace.
//!
//! This command delegates to `nix develop` under the hood. If the workspace
//! has no `flake.nix`, one is generated on-the-fly in a temporary directory
//! so that `nix develop` can still be used.
//!
//! # Examples
//!
//! ```shell
//! # Enter the default dev shell
//! nix-workspace shell
//!
//! # Enter a named dev shell
//! nix-workspace shell my-shell
//!
//! # Run a command inside the shell instead of entering it interactively
//! nix-workspace shell -- cargo build
//!
//! # Enter a shell in JSON output mode (for CI/tooling)
//! nix-workspace shell --format json
//! ```

use crate::diagnostics::{self, Diagnostic, DiagnosticReport, OutputFormat};
use crate::flake_gen;
use crate::workspace;
use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::path::PathBuf;
use std::process::Command;

/// Arguments for `nix-workspace shell`.
#[derive(Debug, clap::Args)]
pub struct ShellArgs {
    /// Shell name to enter.
    ///
    /// If omitted, enters the default dev shell. This corresponds to
    /// `nix develop .#<name>` or `nix develop .` (for the default).
    pub shell: Option<String>,

    /// Target system for the shell environment.
    ///
    /// If omitted, uses the current system. Corresponds to
    /// `--system <system>` in `nix develop`.
    #[arg(long)]
    pub system: Option<String>,

    /// Run a command inside the shell instead of entering it interactively.
    ///
    /// Passes `--command <cmd>` to `nix develop`.
    #[arg(short, long, value_name = "CMD")]
    pub command: Option<String>,

    /// Extra arguments to pass through to `nix develop`.
    ///
    /// Everything after `--` is forwarded verbatim. When combined with
    /// `--command`, the extra args are appended as arguments to the command.
    #[arg(last = true)]
    pub nix_args: Vec<String>,
}

/// Run the `nix-workspace shell` command.
pub fn run(args: &ShellArgs, format: OutputFormat, workspace_dir: Option<&PathBuf>) -> Result<()> {
    // ── Resolve workspace root ────────────────────────────────────
    let root = workspace::resolve_workspace_root(workspace_dir.map(|p| p.as_path()))?;

    // ── Ensure `nix` is available ─────────────────────────────────
    let nix = find_nix()?;

    // ── Determine flake reference ─────────────────────────────────
    //
    // If the workspace has a `flake.nix`, use it directly.
    // Otherwise, generate a temporary flake on-the-fly.

    let temp_flake: Option<flake_gen::TempFlake>;
    let flake_root: PathBuf;

    if flake_gen::needs_flake_generation(&root) {
        if format == OutputFormat::Human {
            eprintln!(
                "{} No flake.nix found — generating temporary flake for shell...",
                "ℹ".blue()
            );
        }

        let config = flake_gen::FlakeGenConfig::default();
        let tf = flake_gen::create_temp_flake(&root, &config)
            .context("Failed to generate temporary flake")?;
        flake_root = tf.flake_root.clone();
        temp_flake = Some(tf);
    } else {
        flake_root = root.clone();
        temp_flake = None;
    }

    // ── Build the flake reference ─────────────────────────────────
    let flake_ref = match &args.shell {
        Some(name) => format!("path:{}#{}", flake_root.display(), name),
        None => format!("path:{}", flake_root.display()),
    };

    // ── Assemble `nix develop` command ────────────────────────────
    let mut cmd = Command::new(&nix);
    cmd.arg("develop");
    cmd.arg(&flake_ref);

    // Standard nix experimental features
    cmd.arg("--extra-experimental-features");
    cmd.arg("nix-command flakes");

    // System override
    if let Some(ref system) = args.system {
        cmd.arg("--system");
        cmd.arg(system);
    }

    // Command to run inside the shell
    if let Some(ref shell_cmd) = args.command {
        cmd.arg("--command");
        cmd.arg(shell_cmd);
    }

    // Impure flag when using temporary flake (symlinks may need it)
    if temp_flake.is_some() {
        cmd.arg("--impure");
    }

    // Pass-through extra nix args
    for arg in &args.nix_args {
        cmd.arg(arg);
    }

    // ── Run the shell ─────────────────────────────────────────────
    if format == OutputFormat::Human {
        let shell_display = args.shell.as_deref().unwrap_or("default");
        if args.command.is_some() {
            eprintln!(
                "{} Running command in shell {} ...",
                "▸".blue().bold(),
                shell_display.bold()
            );
        } else {
            eprintln!(
                "{} Entering shell {} ...",
                "▸".blue().bold(),
                shell_display.bold()
            );
        }
    }

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute: {} develop", nix.display()))?;

    // ── Handle result ─────────────────────────────────────────────
    if status.success() {
        match format {
            OutputFormat::Json => {
                let output = serde_json::json!({
                    "success": true,
                    "shell": args.shell.as_deref().unwrap_or("default"),
                    "flake_ref": flake_ref,
                    "temporary_flake": temp_flake.is_some(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            OutputFormat::Human => {
                // Shell exited normally — nothing extra to print
            }
        }
        Ok(())
    } else {
        let exit_code = status.code().unwrap_or(1);

        match format {
            OutputFormat::Json => {
                let mut report = DiagnosticReport::new();
                report.push(
                    Diagnostic::error(
                        diagnostics::codes::TOOL_FAILED,
                        format!(
                            "`nix develop` failed with exit code {} for shell '{}'",
                            exit_code,
                            args.shell.as_deref().unwrap_or("default")
                        ),
                    )
                    .with_hint("Check the output above for details."),
                );
                println!("{}", report.format_json()?);
            }
            OutputFormat::Human => {
                eprintln!("{} Shell exited with code {}", "✗".red().bold(), exit_code);
            }
        }

        std::process::exit(exit_code);
    }
}

/// Find the `nix` binary on `$PATH`.
fn find_nix() -> Result<PathBuf> {
    which::which("nix").context(
        "Could not find `nix` on $PATH. \
         Install Nix (https://nixos.org/download.html) to use shell commands.",
    )
}

/// List available shells in the workspace for shell completions and
/// error suggestions.
///
/// This performs a fast filesystem scan — no Nix evaluation required.
pub fn list_available_shells(workspace_root: &std::path::Path) -> Vec<String> {
    let mut shells = Vec::new();

    if let Ok(discovery) = workspace::discover_workspace(workspace_root) {
        // Root shells
        for name in discovery.shell_names() {
            shells.push(name.to_string());
        }

        // Subworkspace shells (namespaced)
        for sw in &discovery.subworkspaces {
            if let Some(conv) = sw.conventions.get("shells") {
                for entry in &conv.entries {
                    let namespaced = if entry.name == "default" {
                        sw.name.clone()
                    } else {
                        format!("{}-{}", sw.name, entry.name)
                    };
                    shells.push(namespaced);
                }
            }
        }
    }

    shells.sort();
    shells
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_find_nix() {
        // Environment-dependent: passes if nix is installed
        match find_nix() {
            Ok(path) => assert!(path.exists()),
            Err(e) => assert!(e.to_string().contains("nix")),
        }
    }

    #[test]
    fn test_list_available_shells_empty() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();

        let shells = list_available_shells(tmp.path());
        assert!(shells.is_empty());
    }

    #[test]
    fn test_list_available_shells_with_entries() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir(tmp.path().join("shells")).unwrap();
        fs::write(tmp.path().join("shells/default.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("shells/rust.ncl"), "{}").unwrap();

        let shells = list_available_shells(tmp.path());
        assert_eq!(shells, vec!["default", "rust"]);
    }

    #[test]
    fn test_list_available_shells_with_subworkspaces() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir_all(tmp.path().join("shells")).unwrap();
        fs::write(tmp.path().join("shells/default.ncl"), "{}").unwrap();

        // Subworkspace "frontend" with a dev shell
        fs::create_dir_all(tmp.path().join("frontend/shells")).unwrap();
        fs::write(tmp.path().join("frontend/workspace.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("frontend/shells/default.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("frontend/shells/storybook.ncl"), "{}").unwrap();

        let shells = list_available_shells(tmp.path());
        assert!(shells.contains(&"default".to_string()));
        assert!(shells.contains(&"frontend".to_string())); // default → subworkspace name
        assert!(shells.contains(&"frontend-storybook".to_string())); // namespaced
    }

    #[test]
    fn test_list_available_shells_sorted() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir(tmp.path().join("shells")).unwrap();
        fs::write(tmp.path().join("shells/rust.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("shells/default.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("shells/go.ncl"), "{}").unwrap();

        let shells = list_available_shells(tmp.path());
        assert_eq!(shells, vec!["default", "go", "rust"]);
    }

    #[test]
    fn test_list_available_shells_nonexistent_dir() {
        let shells = list_available_shells(std::path::Path::new("/nonexistent/path"));
        assert!(shells.is_empty());
    }
}
