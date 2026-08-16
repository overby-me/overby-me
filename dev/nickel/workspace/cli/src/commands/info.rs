//! `nix-workspace info` — Show workspace structure and discovered outputs.
//!
//! This command scans the workspace for convention directories, subworkspaces,
//! and configuration, then displays a structured summary.
//!
//! By default, only filesystem-level discovery is performed (no Nickel
//! evaluation). Pass `--eval` to also load and display the evaluated
//! workspace config from `workspace.ncl`.
//!
//! Output is formatted for human reading by default, or as JSON with
//! `--format json`.

use crate::diagnostics::OutputFormat;
use crate::nickel;
use crate::workspace::{self, WorkspaceConfig, WorkspaceSummary};
use anyhow::{Context, Result};
use owo_colors::OwoColorize;

/// Arguments for `nix-workspace info`.
#[derive(Debug, clap::Args)]
pub struct InfoArgs {
    /// Evaluate `workspace.ncl` to show config values (name, systems, plugins, etc.).
    ///
    /// Without this flag, only filesystem discovery is performed.
    /// Requires `nickel` to be on `$PATH`.
    #[arg(long)]
    pub eval: bool,

    /// Show detailed information (all convention entries, subworkspace contents).
    #[arg(short, long)]
    pub verbose: bool,
}

/// Run the `nix-workspace info` command.
pub fn run(
    args: &InfoArgs,
    format: OutputFormat,
    workspace_dir: Option<&std::path::PathBuf>,
) -> Result<()> {
    // ── Resolve workspace root ────────────────────────────────────
    let root = workspace::resolve_workspace_root(workspace_dir.map(|p| p.as_path()))?;

    // ── Discovery phase ───────────────────────────────────────────
    let discovery =
        workspace::discover_workspace(&root).context("Failed to discover workspace structure")?;

    // ── Optional evaluation phase ─────────────────────────────────
    let config: Option<WorkspaceConfig> = if args.eval {
        match nickel::find_nickel() {
            Ok(_) => match nickel::load_workspace_config(&root)? {
                Ok(cfg) => Some(cfg),
                Err(diag_report) => {
                    // Evaluation failed — print diagnostics but continue
                    // with discovery-only info.
                    match format {
                        OutputFormat::Json => {
                            eprintln!(
                                "{}",
                                diag_report
                                    .format_json()
                                    .unwrap_or_else(|_| "{}".to_string())
                            );
                        }
                        OutputFormat::Human => {
                            eprintln!(
                                "{} Failed to evaluate workspace.ncl — showing discovery only\n",
                                "⚠".yellow().bold()
                            );
                            eprint!("{}", diag_report.format_human());
                        }
                    }
                    None
                }
            },
            Err(_) => {
                if format == OutputFormat::Human {
                    eprintln!(
                        "{} `nickel` not found — showing discovery only (install Nickel or use --no-eval)\n",
                        "⚠".yellow().bold()
                    );
                }
                None
            }
        }
    } else {
        None
    };

    // ── Build summary ─────────────────────────────────────────────
    let summary = WorkspaceSummary::from_discovery(&discovery, config.as_ref());

    // ── Output ────────────────────────────────────────────────────
    match format {
        OutputFormat::Json => print_json(&summary, &discovery, args.verbose),
        OutputFormat::Human => print_human(&summary, &discovery, args.verbose, args.eval),
    }
}

/// Print workspace info as JSON.
fn print_json(
    summary: &WorkspaceSummary,
    discovery: &workspace::WorkspaceDiscovery,
    verbose: bool,
) -> Result<()> {
    if verbose {
        // Verbose JSON includes the full discovery tree
        let output = serde_json::json!({
            "summary": summary,
            "discovery": discovery,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", serde_json::to_string_pretty(summary)?);
    }
    Ok(())
}

/// Print workspace info for human reading.
fn print_human(
    summary: &WorkspaceSummary,
    discovery: &workspace::WorkspaceDiscovery,
    verbose: bool,
    evaluated: bool,
) -> Result<()> {
    // ── Header ────────────────────────────────────────────────────
    println!("{} {}", "Workspace:".blue().bold(), summary.name.bold());

    if let Some(ref desc) = summary.description {
        println!("  {} {desc}", "Description:".dimmed());
    }

    println!("  {} {}", "Root:".dimmed(), summary.root.display().cyan());

    // Flake status
    let flake_status = if discovery.has_flake_nix && discovery.has_flake_lock {
        "flake.nix + flake.lock".green().to_string()
    } else if discovery.has_flake_nix {
        "flake.nix (no lock file)".yellow().to_string()
    } else {
        "no flake.nix (standalone mode)".dimmed().to_string()
    };
    println!("  {} {flake_status}", "Flake:".dimmed());

    // Systems (only shown when evaluated)
    if evaluated || !summary.systems.is_empty() {
        println!("  {} {}", "Systems:".dimmed(), summary.systems.join(", "));
    }

    // Plugins
    if !summary.plugins.is_empty() {
        println!("  {} {}", "Plugins:".dimmed(), summary.plugins.join(", "));
    }

    println!();

    // ── Convention directories ─────────────────────────────────────
    let active_conventions: Vec<_> = summary.conventions.iter().filter(|c| c.count > 0).collect();

    if active_conventions.is_empty() {
        println!(
            "  {} No outputs discovered in convention directories.",
            "ℹ".blue()
        );
        println!(
            "    Create .ncl files in {} to define outputs.",
            "packages/, shells/, machines/, modules/, home/".bold()
        );
    } else {
        println!("{}", "Outputs:".blue().bold());

        for conv in &active_conventions {
            let icon = convention_icon(&conv.name);
            println!(
                "  {icon} {} ({} → {}, {} {})",
                conv.name.bold(),
                conv.dir.dimmed(),
                conv.output.dimmed(),
                conv.count,
                if conv.count == 1 { "entry" } else { "entries" }
            );

            if verbose || conv.count <= 8 {
                for entry_name in &conv.entries {
                    println!("      {} {entry_name}", "•".dimmed());
                }
            } else {
                // Show first 5 and a "... and N more"
                for entry_name in conv.entries.iter().take(5) {
                    println!("      {} {entry_name}", "•".dimmed());
                }
                println!("      {} ... and {} more", "•".dimmed(), conv.count - 5);
            }
        }
    }

    // ── Inactive convention directories (verbose only) ────────────
    if verbose {
        let inactive: Vec<_> = discovery
            .conventions
            .values()
            .filter(|c| c.entries.is_empty() && c.exists)
            .collect();

        if !inactive.is_empty() {
            println!();
            println!("  {} Empty convention directories:", "ℹ".dimmed());
            for conv in &inactive {
                println!("      {} {}/  (no .ncl files)", "○".dimmed(), conv.dir);
            }
        }
    }

    // ── Subworkspaces ─────────────────────────────────────────────
    if !summary.subworkspaces.is_empty() {
        println!();
        println!(
            "{} ({} found)",
            "Subworkspaces:".blue().bold(),
            summary.subworkspaces.len()
        );

        for sw in &summary.subworkspaces {
            let conventions_str = if sw.active_conventions.is_empty() {
                "(no outputs)".dimmed().to_string()
            } else {
                sw.active_conventions.join(", ")
            };

            println!(
                "  {} {} — {} {}, conventions: {}",
                "📦".dimmed(),
                sw.name.bold(),
                sw.entry_count,
                if sw.entry_count == 1 {
                    "entry"
                } else {
                    "entries"
                },
                conventions_str
            );

            // In verbose mode, show subworkspace convention details
            if verbose
                && let Some(full_sw) = discovery.subworkspaces.iter().find(|s| s.name == sw.name)
            {
                for (conv_name, conv_outputs) in &full_sw.conventions {
                    if conv_outputs.entries.is_empty() {
                        continue;
                    }
                    println!("      {} {conv_name}/:", "└".dimmed());
                    for entry in &conv_outputs.entries {
                        // Show namespaced name
                        let namespaced = if entry.name == "default" {
                            sw.name.clone()
                        } else {
                            format!("{}-{}", sw.name, entry.name)
                        };
                        println!(
                            "          {} {} → {}",
                            "•".dimmed(),
                            entry.name.dimmed(),
                            namespaced
                        );
                    }
                }
            }
        }
    }

    // ── Summary line ──────────────────────────────────────────────
    println!();
    let total_root: usize = active_conventions.iter().map(|c| c.count).sum();
    let total_sub: usize = summary.subworkspaces.iter().map(|s| s.entry_count).sum();
    let total = total_root + total_sub;
    let sub_count = summary.subworkspaces.len();

    let mut parts = Vec::new();
    parts.push(format!(
        "{} {}",
        total,
        if total == 1 { "output" } else { "outputs" }
    ));
    if sub_count > 0 {
        parts.push(format!(
            "{} {}",
            sub_count,
            if sub_count == 1 {
                "subworkspace"
            } else {
                "subworkspaces"
            }
        ));
        parts.push(format!("{total_root} root + {total_sub} sub"));
    }

    println!("  {} {}", "Total:".dimmed(), parts.join(" · "));

    if !evaluated {
        println!(
            "\n  {} Run with {} to include evaluated config (name, systems, plugins).",
            "💡".dimmed(),
            "--eval".bold()
        );
    }

    Ok(())
}

/// Get a display icon for a convention type.
fn convention_icon(convention: &str) -> &'static str {
    match convention {
        "packages" => "📦",
        "shells" => "🐚",
        "machines" => "🖥️",
        "modules" => "🧩",
        "home" => "🏠",
        "overlays" => "🔧",
        "templates" => "📋",
        "checks" => "✅",
        "lib" => "📚",
        _ => "📄",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        ConventionOutputs, ConventionSummary, DiscoveredEntry, DiscoveredSubworkspace,
        SubworkspaceSummary, WorkspaceDiscovery,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn mock_discovery() -> WorkspaceDiscovery {
        WorkspaceDiscovery {
            root: PathBuf::from("/tmp/test-workspace"),
            has_workspace_ncl: true,
            has_flake_nix: true,
            has_flake_lock: true,
            conventions: BTreeMap::new(),
            subworkspaces: Vec::new(),
        }
    }

    fn mock_summary() -> WorkspaceSummary {
        WorkspaceSummary {
            name: "test-workspace".into(),
            description: Some("A test workspace".into()),
            root: PathBuf::from("/tmp/test-workspace"),
            has_flake: true,
            systems: vec!["x86_64-linux".into(), "aarch64-linux".into()],
            plugins: vec![],
            conventions: vec![],
            subworkspaces: vec![],
        }
    }

    #[test]
    fn test_convention_icon_known() {
        assert_eq!(convention_icon("packages"), "📦");
        assert_eq!(convention_icon("shells"), "🐚");
        assert_eq!(convention_icon("machines"), "🖥️");
        assert_eq!(convention_icon("modules"), "🧩");
        assert_eq!(convention_icon("home"), "🏠");
    }

    #[test]
    fn test_convention_icon_unknown() {
        assert_eq!(convention_icon("custom"), "📄");
    }

    #[test]
    fn test_print_json_minimal() {
        let summary = mock_summary();
        let discovery = mock_discovery();

        // Should not panic
        let result = print_json(&summary, &discovery, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_json_verbose() {
        let summary = mock_summary();
        let discovery = mock_discovery();

        let result = print_json(&summary, &discovery, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_empty_workspace() {
        let summary = mock_summary();
        let discovery = mock_discovery();

        // Should not panic — prints "no outputs discovered"
        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_with_outputs() {
        let mut summary = mock_summary();
        summary.conventions = vec![
            ConventionSummary {
                name: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                count: 3,
                entries: vec!["hello".into(), "world".into(), "cli".into()],
            },
            ConventionSummary {
                name: "shells".into(),
                dir: "shells".into(),
                output: "devShells".into(),
                count: 1,
                entries: vec!["default".into()],
            },
        ];

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
                    DiscoveredEntry {
                        name: "cli".into(),
                        path: PathBuf::from("/tmp/test/packages/cli.ncl"),
                    },
                ],
            },
        );

        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_with_subworkspaces() {
        let mut summary = mock_summary();
        summary.subworkspaces = vec![
            SubworkspaceSummary {
                name: "lib-a".into(),
                entry_count: 2,
                active_conventions: vec!["packages".into()],
            },
            SubworkspaceSummary {
                name: "app-b".into(),
                entry_count: 1,
                active_conventions: vec!["packages".into(), "shells".into()],
            },
        ];

        let mut discovery = mock_discovery();

        // Add subworkspace details for verbose mode
        let mut sub_a_conventions = BTreeMap::new();
        sub_a_conventions.insert(
            "packages".into(),
            ConventionOutputs {
                convention: "packages".into(),
                dir: "packages".into(),
                output: "packages".into(),
                exists: true,
                entries: vec![
                    DiscoveredEntry {
                        name: "default".into(),
                        path: PathBuf::from("/tmp/test/lib-a/packages/default.ncl"),
                    },
                    DiscoveredEntry {
                        name: "extra".into(),
                        path: PathBuf::from("/tmp/test/lib-a/packages/extra.ncl"),
                    },
                ],
            },
        );
        discovery.subworkspaces.push(DiscoveredSubworkspace {
            name: "lib-a".into(),
            path: PathBuf::from("/tmp/test/lib-a"),
            conventions: sub_a_conventions,
        });

        // Non-verbose should not panic
        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());

        // Verbose should not panic and show namespaced names
        let result = print_human(&summary, &discovery, true, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_with_plugins() {
        let mut summary = mock_summary();
        summary.plugins = vec!["nix-workspace-rust".into(), "nix-workspace-go".into()];

        let discovery = mock_discovery();
        let result = print_human(&summary, &discovery, false, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_no_flake() {
        let mut summary = mock_summary();
        summary.has_flake = false;

        let mut discovery = mock_discovery();
        discovery.has_flake_nix = false;
        discovery.has_flake_lock = false;

        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_flake_no_lock() {
        let mut discovery = mock_discovery();
        discovery.has_flake_lock = false;

        let summary = mock_summary();
        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_print_human_many_entries_truncation() {
        let mut summary = mock_summary();
        let entries: Vec<String> = (0..20).map(|i| format!("pkg-{i}")).collect();
        summary.conventions = vec![ConventionSummary {
            name: "packages".into(),
            dir: "packages".into(),
            output: "packages".into(),
            count: entries.len(),
            entries: entries.clone(),
        }];

        let discovery = mock_discovery();

        // Non-verbose: should truncate to 5 + "... and N more"
        let result = print_human(&summary, &discovery, false, false);
        assert!(result.is_ok());

        // Verbose: should show all 20
        let result = print_human(&summary, &discovery, true, false);
        assert!(result.is_ok());
    }
}
