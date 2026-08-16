#![allow(dead_code)]
//! `nix-workspace build` — Build a package from the workspace.
//!
//! This command delegates to `nix build` under the hood. If the workspace
//! has no `flake.nix`, one is generated on-the-fly in a temporary directory
//! so that `nix build` can still be used.
//!
//! # Examples
//!
//! ```shell
//! # Build a specific package
//! nix-workspace build hello
//!
//! # Build the default package
//! nix-workspace build
//!
//! # Build with extra nix flags
//! nix-workspace build hello -- --no-link --print-out-paths
//!
//! # Build in JSON output mode (for CI/tooling)
//! nix-workspace build hello --format json
//! ```

use crate::diagnostics::{self, Diagnostic, DiagnosticReport, OutputFormat};
use crate::flake_gen;
use crate::workspace;
use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::path::PathBuf;
use std::process::Command;

/// Arguments for `nix-workspace build`.
#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Package name to build.
    ///
    /// If omitted, builds the default package. This corresponds to
    /// `nix build .#<name>` or `nix build .` (for the default).
    pub package: Option<String>,

    /// Target system to build for.
    ///
    /// If omitted, uses the current system. Corresponds to
    /// `--system <system>` in `nix build`.
    #[arg(long)]
    pub system: Option<String>,

    /// Print the output store path(s) instead of creating a `result` symlink.
    ///
    /// Passes `--no-link --print-out-paths` to `nix build`.
    #[arg(long)]
    pub print_out_paths: bool,

    /// Don't create a `result` symlink.
    #[arg(long)]
    pub no_link: bool,

    /// Output path for the `result` symlink.
    ///
    /// Passes `--out-link <path>` to `nix build`.
    #[arg(short, long, value_name = "PATH")]
    pub out_link: Option<PathBuf>,

    /// Extra arguments to pass through to `nix build`.
    ///
    /// Everything after `--` is forwarded verbatim.
    #[arg(last = true)]
    pub nix_args: Vec<String>,
}

/// Run the `nix-workspace build` command.
pub fn run(args: &BuildArgs, format: OutputFormat, workspace_dir: Option<&PathBuf>) -> Result<()> {
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
                "{} No flake.nix found — generating temporary flake for build...",
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
    let flake_ref = match &args.package {
        Some(pkg) => format!("path:{}#{}", flake_root.display(), pkg),
        None => format!("path:{}", flake_root.display()),
    };

    // ── Assemble `nix build` command ──────────────────────────────
    let mut cmd = Command::new(&nix);
    cmd.arg("build");
    cmd.arg(&flake_ref);

    // Standard nix experimental features
    cmd.arg("--extra-experimental-features");
    cmd.arg("nix-command flakes");

    // System override
    if let Some(ref system) = args.system {
        cmd.arg("--system");
        cmd.arg(system);
    }

    // Output options
    if args.print_out_paths {
        cmd.arg("--no-link");
        cmd.arg("--print-out-paths");
    } else if args.no_link {
        cmd.arg("--no-link");
    }

    if let Some(ref out_link) = args.out_link {
        cmd.arg("--out-link");
        cmd.arg(out_link);
    }

    // Impure flag when using temporary flake (symlinks may need it)
    if temp_flake.is_some() {
        cmd.arg("--impure");
    }

    // Pass-through extra nix args
    for arg in &args.nix_args {
        cmd.arg(arg);
    }

    // ── Run the build ─────────────────────────────────────────────
    if format == OutputFormat::Human {
        let pkg_display = args.package.as_deref().unwrap_or("(default)");
        eprintln!("{} Building {} ...", "▸".blue().bold(), pkg_display.bold());
    }

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute: {} build", nix.display()))?;

    // ── Handle result ─────────────────────────────────────────────
    if status.success() {
        match format {
            OutputFormat::Json => {
                let output = serde_json::json!({
                    "success": true,
                    "package": args.package.as_deref().unwrap_or("default"),
                    "flake_ref": flake_ref,
                    "temporary_flake": temp_flake.is_some(),
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            OutputFormat::Human => {
                let pkg_display = args.package.as_deref().unwrap_or("(default)");
                eprintln!("{} Build succeeded: {}", "✓".green().bold(), pkg_display);
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
                            "`nix build` failed with exit code {} for package '{}'",
                            exit_code,
                            args.package.as_deref().unwrap_or("default")
                        ),
                    )
                    .with_hint("Check the build output above for details."),
                );
                println!("{}", report.format_json()?);
            }
            OutputFormat::Human => {
                eprintln!(
                    "{} Build failed (exit code {})",
                    "✗".red().bold(),
                    exit_code
                );
            }
        }

        std::process::exit(exit_code);
    }
}

/// Find the `nix` binary on `$PATH`.
fn find_nix() -> Result<PathBuf> {
    which::which("nix").context(
        "Could not find `nix` on $PATH. \
         Install Nix (https://nixos.org/download.html) to use build commands.",
    )
}

/// List available packages in the workspace for shell completions and
/// error suggestions.
///
/// This performs a fast filesystem scan — no Nix evaluation required.
pub fn list_available_packages(workspace_root: &std::path::Path) -> Vec<String> {
    let mut packages = Vec::new();

    if let Ok(discovery) = workspace::discover_workspace(workspace_root) {
        // Root packages
        for name in discovery.package_names() {
            packages.push(name.to_string());
        }

        // Subworkspace packages (namespaced)
        for sw in &discovery.subworkspaces {
            if let Some(conv) = sw.conventions.get("packages") {
                for entry in &conv.entries {
                    let namespaced = if entry.name == "default" {
                        sw.name.clone()
                    } else {
                        format!("{}-{}", sw.name, entry.name)
                    };
                    packages.push(namespaced);
                }
            }
        }
    }

    packages.sort();
    packages
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
    fn test_list_available_packages_empty() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();

        let packages = list_available_packages(tmp.path());
        assert!(packages.is_empty());
    }

    #[test]
    fn test_list_available_packages_with_entries() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir(tmp.path().join("packages")).unwrap();
        fs::write(tmp.path().join("packages/hello.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("packages/world.ncl"), "{}").unwrap();

        let packages = list_available_packages(tmp.path());
        assert_eq!(packages, vec!["hello", "world"]);
    }

    #[test]
    fn test_list_available_packages_with_subworkspaces() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir_all(tmp.path().join("packages")).unwrap();
        fs::write(tmp.path().join("packages/root-pkg.ncl"), "{}").unwrap();

        // Subworkspace "lib-a" with a default package
        fs::create_dir_all(tmp.path().join("lib-a/packages")).unwrap();
        fs::write(tmp.path().join("lib-a/workspace.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("lib-a/packages/default.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("lib-a/packages/extra.ncl"), "{}").unwrap();

        let packages = list_available_packages(tmp.path());
        assert!(packages.contains(&"root-pkg".to_string()));
        assert!(packages.contains(&"lib-a".to_string())); // default → subworkspace name
        assert!(packages.contains(&"lib-a-extra".to_string())); // namespaced
    }

    #[test]
    fn test_list_available_packages_sorted() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::create_dir(tmp.path().join("packages")).unwrap();
        fs::write(tmp.path().join("packages/zebra.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("packages/alpha.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("packages/middle.ncl"), "{}").unwrap();

        let packages = list_available_packages(tmp.path());
        assert_eq!(packages, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn test_list_available_packages_nonexistent_dir() {
        let packages = list_available_packages(std::path::Path::new("/nonexistent/path"));
        assert!(packages.is_empty());
    }
}
