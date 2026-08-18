//! `nix-workspace init` — Initialize a new workspace.
//!
//! Creates a workspace scaffold with:
//! - `workspace.ncl` — workspace configuration
//! - `flake.nix` — Nix flake entry point (optional, enabled by default)
//! - Convention directories (`packages/`, `shells/`, etc.)
//! - Sample files in convention directories
//! - `.gitignore` with Nix-relevant entries

use crate::diagnostics::OutputFormat;
use crate::flake_gen::{self, InitOptions};
use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use std::path::PathBuf;

/// Arguments for `nix-workspace init`.
#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// Directory to initialize (defaults to current directory).
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Workspace name (defaults to the directory name).
    #[arg(short, long)]
    pub name: Option<String>,

    /// Workspace description.
    #[arg(short, long)]
    pub description: Option<String>,

    /// Target systems (comma-separated).
    ///
    /// Defaults to "x86_64-linux,aarch64-linux".
    #[arg(
        short,
        long,
        value_delimiter = ',',
        default_values_t = vec![
            "x86_64-linux".to_string(),
            "aarch64-linux".to_string(),
        ]
    )]
    pub systems: Vec<String>,

    /// Convention directories to create (comma-separated).
    ///
    /// Defaults to "packages,shells".
    #[arg(
        short,
        long,
        value_delimiter = ',',
        default_values_t = vec![
            "packages".to_string(),
            "shells".to_string(),
        ]
    )]
    pub conventions: Vec<String>,

    /// Plugins to enable (comma-separated).
    ///
    /// Example: --plugins nix-workspace-rust,nix-workspace-go
    #[arg(short, long, value_delimiter = ',')]
    pub plugins: Vec<String>,

    /// Skip generating a flake.nix file.
    ///
    /// Use this if you want to manage flake.nix yourself or if you're
    /// using nix-workspace in standalone mode exclusively.
    #[arg(long)]
    pub no_flake: bool,
}

/// Run the `nix-workspace init` command.
pub fn run(args: &InitArgs, format: OutputFormat) -> Result<()> {
    let root = if args.path.is_absolute() {
        args.path.clone()
    } else {
        std::env::current_dir()
            .context("Failed to get current directory")?
            .join(&args.path)
    };

    // Derive the workspace name from --name, or the directory name
    let name = args
        .name
        .clone()
        .or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "my-workspace".to_string());

    let opts = InitOptions {
        name: name.clone(),
        description: args.description.clone(),
        systems: args.systems.clone(),
        with_flake: !args.no_flake,
        conventions: args.conventions.clone(),
        plugins: args.plugins.clone(),
    };

    let created = flake_gen::init_workspace(&root, &opts)?;

    // Output results
    match format {
        OutputFormat::Json => {
            let output = serde_json::json!({
                "workspace": name,
                "root": root.display().to_string(),
                "created": created.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        OutputFormat::Human => {
            println!(
                "{} Initialized workspace {} in {}\n",
                "✓".green().bold(),
                name.bold(),
                root.display().cyan()
            );

            println!("  {} files created:", "Created".green().bold());
            for path in &created {
                let icon = if path.extension().is_some() {
                    "📄"
                } else {
                    "📁"
                };
                println!("    {icon} {}", path.display().dimmed());
            }

            println!();
            println!("  {} steps:", "Next".blue().bold());
            println!(
                "    1. Edit {} to configure your workspace",
                "workspace.ncl".bold()
            );
            if !args.no_flake {
                println!("    2. Run {} to validate", "nix-workspace check".bold());
                println!("    3. Run {} to enter the dev shell", "nix develop".bold());
            } else {
                println!("    2. Run {} to validate", "nix-workspace check".bold());
                println!(
                    "    3. Run {} to enter the dev shell",
                    "nix-workspace shell".bold()
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_default_args() {
        let tmp = TempDir::new().unwrap();
        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("test-project".into()),
            description: None,
            systems: vec!["x86_64-linux".into()],
            conventions: vec!["packages".into(), "shells".into()],
            plugins: vec![],
            no_flake: false,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_ok());

        assert!(tmp.path().join("workspace.ncl").is_file());
        assert!(tmp.path().join("flake.nix").is_file());
        assert!(tmp.path().join("packages").is_dir());
        assert!(tmp.path().join("shells").is_dir());
        assert!(tmp.path().join("shells/default.ncl").is_file());
    }

    #[test]
    fn test_init_no_flake() {
        let tmp = TempDir::new().unwrap();
        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("no-flake".into()),
            description: None,
            systems: vec!["x86_64-linux".into()],
            conventions: vec!["packages".into()],
            plugins: vec![],
            no_flake: true,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_ok());

        assert!(tmp.path().join("workspace.ncl").is_file());
        assert!(!tmp.path().join("flake.nix").exists());
    }

    #[test]
    fn test_init_json_output() {
        let tmp = TempDir::new().unwrap();
        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("json-test".into()),
            description: Some("JSON test workspace".into()),
            systems: vec!["x86_64-linux".into()],
            conventions: vec!["packages".into()],
            plugins: vec![],
            no_flake: false,
        };

        // JSON output should not panic
        let result = run(&args, OutputFormat::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_init_with_plugins() {
        let tmp = TempDir::new().unwrap();
        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("plugin-project".into()),
            description: None,
            systems: vec!["x86_64-linux".into()],
            conventions: vec!["packages".into()],
            plugins: vec!["nix-workspace-rust".into()],
            no_flake: false,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_ok());

        // The workspace.ncl should mention the plugin
        let content = std::fs::read_to_string(tmp.path().join("workspace.ncl")).unwrap();
        assert!(content.contains("nix-workspace-rust"));
    }

    #[test]
    fn test_init_refuses_existing_workspace() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();

        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("existing".into()),
            description: None,
            systems: vec!["x86_64-linux".into()],
            conventions: vec![],
            plugins: vec![],
            no_flake: false,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_init_derives_name_from_directory() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("my-cool-project");
        std::fs::create_dir(&project_dir).unwrap();

        let args = InitArgs {
            path: project_dir.clone(),
            name: None, // should derive from directory name
            description: None,
            systems: vec!["x86_64-linux".into()],
            conventions: vec![],
            plugins: vec![],
            no_flake: true,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(project_dir.join("workspace.ncl")).unwrap();
        assert!(content.contains("my-cool-project"));
    }

    #[test]
    fn test_init_with_description() {
        let tmp = TempDir::new().unwrap();
        let args = InitArgs {
            path: tmp.path().to_path_buf(),
            name: Some("desc-test".into()),
            description: Some("A workspace with a description".into()),
            systems: vec!["x86_64-linux".into()],
            conventions: vec![],
            plugins: vec![],
            no_flake: true,
        };

        let result = run(&args, OutputFormat::Human);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(tmp.path().join("workspace.ncl")).unwrap();
        assert!(content.contains("A workspace with a description"));
    }
}
