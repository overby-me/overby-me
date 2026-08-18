//! `nix-workspace check` — Validate workspace configuration against contracts.
//!
//! This command evaluates `workspace.ncl` with the `WorkspaceConfig` contract
//! applied (or an extended contract when plugins are loaded) and reports any
//! validation errors.
//!
//! Exit codes:
//! - 0 — Workspace configuration is valid
//! - 1 — Validation errors found
//! - 2 — Tool or infrastructure error (e.g., `nickel` not found)
//!
//! The command mirrors the Nix-side contract checking done in
//! `lib/eval-nickel.nix`, but runs directly via the Nickel CLI for fast
//! feedback without a full Nix evaluation.

use crate::diagnostics::{self, Diagnostic, DiagnosticReport, OutputFormat};
use crate::nickel;
use crate::workspace;
use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::path::PathBuf;

/// Arguments for `nix-workspace check`.
#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    /// Also typecheck individual contract files.
    ///
    /// When enabled, runs `nickel typecheck` on each `.ncl` file in
    /// convention directories before the full workspace validation.
    /// This catches type errors that contract validation alone might miss.
    #[arg(long)]
    pub typecheck: bool,

    /// Only perform discovery-based checks (no Nickel evaluation).
    ///
    /// Checks for structural issues like missing convention directories,
    /// naming conflicts, and missing `workspace.ncl` without invoking
    /// Nickel. Useful when `nickel` is not installed.
    #[arg(long)]
    pub discovery_only: bool,

    /// Path to the nix-workspace contracts directory.
    ///
    /// By default, the CLI searches for contracts relative to its own
    /// binary, via `$NIX_WORKSPACE_CONTRACTS`, or in the workspace tree.
    #[arg(long, value_name = "DIR", env = "NIX_WORKSPACE_CONTRACTS")]
    pub contracts_dir: Option<PathBuf>,

    /// Path to the nix-workspace plugins directory.
    ///
    /// By default, the CLI searches for plugins relative to its own
    /// binary, via `$NIX_WORKSPACE_PLUGINS`, or in the workspace tree.
    #[arg(long, value_name = "DIR", env = "NIX_WORKSPACE_PLUGINS")]
    pub plugins_dir: Option<PathBuf>,
}

/// Run the `nix-workspace check` command.
pub fn run(args: &CheckArgs, format: OutputFormat, workspace_dir: Option<&PathBuf>) -> Result<()> {
    // ── Resolve workspace root ────────────────────────────────────
    let root = workspace::resolve_workspace_root(workspace_dir.map(|p| p.as_path()))?;

    // ── Discovery phase ───────────────────────────────────────────
    let discovery =
        workspace::discover_workspace(&root).context("Failed to discover workspace structure")?;

    let mut report = DiagnosticReport::new();

    // Check: workspace.ncl must exist
    if !discovery.has_workspace_ncl {
        report.push(
            Diagnostic::error(
                diagnostics::codes::MISSING_WORKSPACE_NCL,
                format!("No workspace.ncl found at {}", root.display()),
            )
            .with_hint(
                "Run `nix-workspace init` to create a new workspace, \
                 or create workspace.ncl manually.",
            ),
        );

        // Can't proceed without workspace.ncl
        return print_report(&report, format);
    }

    // Check: look for naming conflicts between discovered outputs
    check_naming_conflicts(&discovery, &mut report);

    // If discovery-only, stop here
    if args.discovery_only {
        if report.is_empty() {
            print_success(
                &root,
                format,
                "Discovery checks passed (no Nickel evaluation)",
            );
        }
        return print_report(&report, format);
    }

    // ── Nickel availability ───────────────────────────────────────
    if nickel::find_nickel().is_err() {
        report.push(nickel::missing_tool_diagnostic(
            "nickel",
            "Install Nickel (https://nickel-lang.org/) or run within a nix-workspace dev shell.\n\
             You can also use --discovery-only to skip Nickel evaluation.",
        ));
        return print_report(&report, format);
    }

    // ── Optional typecheck phase ──────────────────────────────────
    if args.typecheck {
        run_typecheck_phase(&discovery, &mut report)?;
    }

    // ── Contract validation phase ─────────────────────────────────
    //
    // First, try to read the workspace config (without contracts) to
    // determine which plugins are loaded. Then apply the appropriate
    // contract (with or without plugin extensions).

    let contracts_dir = args
        .contracts_dir
        .clone()
        .or_else(nickel::find_contracts_dir);

    let plugins_dir = args.plugins_dir.clone().or_else(nickel::find_plugins_dir);

    match &contracts_dir {
        Some(cdir) => {
            // Try to load workspace config to discover plugins
            let plugin_names = match nickel::load_workspace_config(&root)? {
                Ok(config) => config.plugins,
                Err(_eval_report) => {
                    // If raw eval fails, we can't determine plugins.
                    // Proceed with base contract validation — the contract
                    // validation itself will catch the real errors.
                    Vec::new()
                }
            };

            let validation_result = if plugin_names.is_empty() {
                nickel::validate_workspace(&root, cdir)?
            } else {
                match &plugins_dir {
                    Some(pdir) => {
                        nickel::validate_workspace_with_plugins(&root, cdir, pdir, &plugin_names)?
                    }
                    None => {
                        report.push(
                            Diagnostic::warning(
                                diagnostics::codes::PLUGIN_ERROR,
                                format!(
                                    "Workspace uses plugins {:?} but the plugins directory \
                                     could not be found. Validating without plugin contracts.",
                                    plugin_names
                                ),
                            )
                            .with_hint(
                                "Set $NIX_WORKSPACE_PLUGINS or --plugins-dir to the \
                                 nix-workspace plugins/ directory.",
                            ),
                        );
                        nickel::validate_workspace(&root, cdir)?
                    }
                }
            };

            if !validation_result.success {
                // Merge diagnostics from Nickel evaluation
                for diag in validation_result.diagnostics.diagnostics {
                    report.push(diag);
                }
            }
        }
        None => {
            // No contracts directory found — do a basic eval without contracts
            report.push(
                Diagnostic::warning(
                    diagnostics::codes::DISCOVERY_ERROR,
                    "Could not find nix-workspace contracts directory. \
                     Performing raw evaluation without contract validation.",
                )
                .with_hint(
                    "Set $NIX_WORKSPACE_CONTRACTS or --contracts-dir to the \
                     nix-workspace contracts/ directory.",
                ),
            );

            let result = nickel::eval_workspace_config(&root)?;
            if !result.success {
                for diag in result.diagnostics.diagnostics {
                    report.push(diag);
                }
            }
        }
    }

    // ── Report results ────────────────────────────────────────────
    if !report.has_errors() {
        let detail = if report.warning_count() > 0 {
            format!("Workspace is valid ({} warnings)", report.warning_count())
        } else {
            "Workspace configuration is valid".to_string()
        };
        print_success(&root, format, &detail);
    }

    print_report(&report, format)
}

/// Check for naming conflicts in discovered outputs.
///
/// This mirrors the `checkDiscoveryConflicts` function from `lib/discover.nix`.
fn check_naming_conflicts(
    discovery: &workspace::WorkspaceDiscovery,
    report: &mut DiagnosticReport,
) {
    use std::collections::HashMap;

    // Build a registry: { convention -> { output_name -> [sources] } }
    let mut registry: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();

    // Register root outputs
    for (conv_name, conv_outputs) in &discovery.conventions {
        let conv_registry = registry.entry(conv_name.clone()).or_default();
        for entry in &conv_outputs.entries {
            conv_registry
                .entry(entry.name.clone())
                .or_default()
                .push("root".to_string());
        }
    }

    // Register subworkspace outputs (with namespacing)
    for sw in &discovery.subworkspaces {
        for (conv_name, conv_outputs) in &sw.conventions {
            let conv_registry = registry.entry(conv_name.clone()).or_default();
            for entry in &conv_outputs.entries {
                // Apply namespacing: default → sw_name, other → sw_name-other
                let namespaced = if entry.name == "default" {
                    sw.name.clone()
                } else {
                    format!("{}-{}", sw.name, entry.name)
                };

                conv_registry
                    .entry(namespaced)
                    .or_default()
                    .push(format!("subworkspace:{}", sw.name));
            }
        }
    }

    // Find conflicts (entries with multiple sources)
    for (convention, names) in &registry {
        for (name, sources) in names {
            if sources.len() > 1 {
                report.push(
                    Diagnostic::error(
                        diagnostics::codes::NAMESPACE_CONFLICT,
                        format!(
                            "Namespace conflict: output '{}' in '{}' is produced by {} sources: {}",
                            name,
                            convention,
                            sources.len(),
                            sources.join(", ")
                        ),
                    )
                    .with_field(format!("{convention}.{name}"))
                    .with_hint(
                        "Rename one of the conflicting outputs or use a \
                         different subworkspace directory name.",
                    ),
                );
            }
        }
    }
}

/// Run `nickel typecheck` on each discovered `.ncl` file.
fn run_typecheck_phase(
    discovery: &workspace::WorkspaceDiscovery,
    report: &mut DiagnosticReport,
) -> Result<()> {
    // Typecheck workspace.ncl
    if discovery.has_workspace_ncl {
        let workspace_ncl = discovery.root.join("workspace.ncl");
        let result = nickel::typecheck(&workspace_ncl)?;
        if !result.success {
            report.push(
                Diagnostic::error(
                    diagnostics::codes::INVALID_NCL_FILE,
                    "Typecheck failed for workspace.ncl".to_string(),
                )
                .with_file("workspace.ncl")
                .with_hint(result.stderr.lines().take(5).collect::<Vec<_>>().join("\n")),
            );
        }
    }

    // Typecheck all discovered .ncl files in convention directories
    for conv_outputs in discovery.conventions.values() {
        for entry in &conv_outputs.entries {
            let result = nickel::typecheck(&entry.path)?;
            if !result.success {
                let rel_path = format!("{}/{}.ncl", conv_outputs.dir, entry.name);
                report.push(
                    Diagnostic::error(
                        diagnostics::codes::INVALID_NCL_FILE,
                        format!("Typecheck failed for {}", rel_path),
                    )
                    .with_file(&rel_path)
                    .with_hint(result.stderr.lines().take(5).collect::<Vec<_>>().join("\n")),
                );
            }
        }
    }

    Ok(())
}

/// Print a success message.
fn print_success(root: &std::path::Path, format: OutputFormat, detail: &str) {
    match format {
        OutputFormat::Json => {
            // For JSON, success is indicated by an empty diagnostics array
            // (printed by print_report). Nothing extra needed here.
        }
        OutputFormat::Human => {
            eprintln!(
                "{} {} ({})",
                "✓".green().bold(),
                detail,
                root.display().dimmed()
            );
        }
    }
}

/// Print a diagnostic report and return an appropriate exit result.
///
/// Returns `Ok(())` if there are no errors (warnings are acceptable),
/// or `Err` if there are errors (to signal a non-zero exit code).
fn print_report(report: &DiagnosticReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                report
                    .format_json()
                    .context("Failed to serialize diagnostics as JSON")?
            );
        }
        OutputFormat::Human => {
            if !report.is_empty() {
                eprint!("{}", report.format_human());
            }
        }
    }

    if report.has_errors() {
        // Use a specific exit code for validation errors
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        ConventionOutputs, DiscoveredEntry, DiscoveredSubworkspace, WorkspaceDiscovery,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Build a minimal WorkspaceDiscovery for testing.
    fn mock_discovery() -> WorkspaceDiscovery {
        WorkspaceDiscovery {
            root: PathBuf::from("/tmp/test"),
            has_workspace_ncl: true,
            has_flake_nix: false,
            has_flake_lock: false,
            conventions: BTreeMap::new(),
            subworkspaces: Vec::new(),
        }
    }

    #[test]
    fn test_no_naming_conflicts_empty() {
        let discovery = mock_discovery();
        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);
        assert!(report.is_empty());
    }

    #[test]
    fn test_no_naming_conflicts_distinct_names() {
        let mut discovery = mock_discovery();
        discovery.conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![
                    DiscoveredEntry {
                        name: "hello".into(),
                        path: PathBuf::from("/tmp/test/packages/hello.ncl"),
                    },
                    DiscoveredEntry {
                        name: "world".into(),
                        path: PathBuf::from("/tmp/test/packages/world.ncl"),
                    },
                ],
            },
        );

        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);
        assert!(report.is_empty());
    }

    #[test]
    fn test_naming_conflict_root_vs_subworkspace() {
        let mut discovery = mock_discovery();

        // Root has a package named "lib-a"
        discovery.conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![DiscoveredEntry {
                    name: "lib-a".into(),
                    path: PathBuf::from("/tmp/test/packages/lib-a.ncl"),
                }],
            },
        );

        // Subworkspace "lib-a" has a default package → namespaced as "lib-a"
        let mut sub_conventions = BTreeMap::new();
        sub_conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![DiscoveredEntry {
                    name: "default".into(),
                    path: PathBuf::from("/tmp/test/lib-a/packages/default.ncl"),
                }],
            },
        );

        discovery.subworkspaces.push(DiscoveredSubworkspace {
            name: "lib-a".into(),
            path: PathBuf::from("/tmp/test/lib-a"),
            conventions: sub_conventions,
        });

        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);

        assert!(report.has_errors());
        assert_eq!(report.error_count(), 1);
        let diag = &report.diagnostics[0];
        assert_eq!(diag.code, diagnostics::codes::NAMESPACE_CONFLICT);
        assert!(diag.message.contains("lib-a"));
        assert!(diag.message.contains("2 sources"));
    }

    #[test]
    fn test_naming_conflict_between_subworkspaces() {
        let mut discovery = mock_discovery();
        discovery.conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![],
            },
        );

        // Two subworkspaces that both produce "foo-bar" in packages
        for sub_name in &["foo", "foo"] {
            let mut sub_conventions = BTreeMap::new();
            sub_conventions.insert(
                "packages".into(),
                ConventionOutputs {
                    convention: "packages".into(),
                    dir: "packages".into(),
                    output: "packages".into(),
                    exists: true,
                    entries: vec![DiscoveredEntry {
                        name: "bar".into(),
                        path: PathBuf::from(format!("/tmp/test/{sub_name}/packages/bar.ncl")),
                    }],
                },
            );
            discovery.subworkspaces.push(DiscoveredSubworkspace {
                name: sub_name.to_string(),
                path: PathBuf::from(format!("/tmp/test/{sub_name}")),
                conventions: sub_conventions,
            });
        }

        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);

        // "foo-bar" appears from two "foo" subworkspaces
        assert!(report.has_errors());
        assert_eq!(report.error_count(), 1);
        let diag = &report.diagnostics[0];
        assert!(diag.message.contains("foo-bar"));
    }

    #[test]
    fn test_subworkspace_default_namespacing() {
        let mut discovery = mock_discovery();
        discovery.conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![],
            },
        );

        // Subworkspace "my-lib" with default.ncl → namespaced as "my-lib"
        let mut sub_conventions = BTreeMap::new();
        sub_conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![DiscoveredEntry {
                    name: "default".into(),
                    path: PathBuf::from("/tmp/test/my-lib/packages/default.ncl"),
                }],
            },
        );
        discovery.subworkspaces.push(DiscoveredSubworkspace {
            name: "my-lib".into(),
            path: PathBuf::from("/tmp/test/my-lib"),
            conventions: sub_conventions,
        });

        // No conflict — just "my-lib" from one source
        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);
        assert!(!report.has_errors());
    }

    #[test]
    fn test_subworkspace_non_default_namespacing() {
        let mut discovery = mock_discovery();
        discovery.conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![],
            },
        );

        // Subworkspace "app" with "cli.ncl" → namespaced as "app-cli"
        let mut sub_conventions = BTreeMap::new();
        sub_conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![DiscoveredEntry {
                    name: "cli".into(),
                    path: PathBuf::from("/tmp/test/app/packages/cli.ncl"),
                }],
            },
        );
        discovery.subworkspaces.push(DiscoveredSubworkspace {
            name: "app".into(),
            path: PathBuf::from("/tmp/test/app"),
            conventions: sub_conventions,
        });

        let mut report = DiagnosticReport::new();
        check_naming_conflicts(&discovery, &mut report);
        assert!(!report.has_errors());
    }
}
