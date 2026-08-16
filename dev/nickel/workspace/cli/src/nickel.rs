#![allow(dead_code)]
//! Nickel evaluation wrapper — shells out to the `nickel` CLI.
//!
//! This module provides functions to:
//! - Locate the `nickel` binary
//! - Evaluate `workspace.ncl` files (raw export, no contract)
//! - Validate `workspace.ncl` files against contracts (with contract application)
//! - Typecheck individual `.ncl` files
//!
//! All evaluation is done by invoking `nickel export` or `nickel typecheck`
//! as a subprocess. The CLI does not embed the Nickel evaluator directly —
//! this keeps the binary small and avoids version-coupling with libnickel.

use crate::diagnostics::{self, Diagnostic, DiagnosticReport};
use crate::workspace::WorkspaceConfig;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Nickel binary resolution ──────────────────────────────────────

/// Find the `nickel` binary on `$PATH`.
///
/// Returns the full path to the binary, or an error if it's not found.
pub fn find_nickel() -> Result<PathBuf> {
    which::which("nickel").context(
        "Could not find `nickel` on $PATH. \
         Install Nickel (https://nickel-lang.org/) or enter a nix-workspace dev shell.",
    )
}

/// Check that the `nickel` binary is available and return its version string.
pub fn nickel_version() -> Result<String> {
    let nickel = find_nickel()?;
    let output = Command::new(&nickel)
        .arg("--version")
        .output()
        .with_context(|| format!("Failed to execute: {}", nickel.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.trim().to_string();

    if version.is_empty() {
        // Some versions print to stderr
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(stderr.trim().to_string())
    } else {
        Ok(version)
    }
}

// ── Raw evaluation ────────────────────────────────────────────────

/// Result of a Nickel evaluation.
#[derive(Debug)]
pub struct NickelResult {
    /// Whether the evaluation succeeded (exit code 0).
    pub success: bool,

    /// Stdout from `nickel` (typically JSON on success).
    pub stdout: String,

    /// Stderr from `nickel` (typically error messages on failure).
    pub stderr: String,

    /// Parsed diagnostics from stderr (if evaluation failed).
    pub diagnostics: DiagnosticReport,
}

/// Evaluate a `.ncl` file with `nickel export` and return the raw JSON output.
///
/// This performs a raw export without applying any workspace contracts.
/// Useful for reading the config structure without full validation.
pub fn eval_raw(ncl_path: &Path) -> Result<NickelResult> {
    let nickel = find_nickel()?;

    let output = Command::new(&nickel)
        .arg("export")
        .arg(ncl_path)
        .output()
        .with_context(|| format!("Failed to execute nickel export on {}", ncl_path.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    let diagnostics = if success {
        DiagnosticReport::new()
    } else {
        diagnostics::parse_nickel_error(&stderr)
    };

    Ok(NickelResult {
        success,
        stdout,
        stderr,
        diagnostics,
    })
}

/// Evaluate a `.ncl` file with `nickel export` using the provided Nickel
/// source string as input (via a temporary file).
///
/// This is used for wrapper scripts that apply contracts to workspace configs.
fn eval_source(source: &str, working_dir: &Path) -> Result<NickelResult> {
    let nickel = find_nickel()?;

    // Write the source to a temporary file in the working directory so that
    // relative imports (e.g., `import "./workspace.ncl"`) resolve correctly.
    let tmp_file = tempfile::Builder::new()
        .prefix("nix-workspace-eval-")
        .suffix(".ncl")
        .tempfile_in(working_dir)
        .context("Failed to create temporary .ncl file")?;

    std::fs::write(tmp_file.path(), source).context("Failed to write temporary .ncl file")?;

    let output = Command::new(&nickel)
        .arg("export")
        .arg(tmp_file.path())
        .current_dir(working_dir)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute nickel export in {}",
                working_dir.display()
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    let diagnostics = if success {
        DiagnosticReport::new()
    } else {
        diagnostics::parse_nickel_error(&stderr)
    };

    Ok(NickelResult {
        success,
        stdout,
        stderr,
        diagnostics,
    })
}

// ── Workspace evaluation ──────────────────────────────────────────

/// Evaluate `workspace.ncl` in the given workspace root without contract
/// validation. Returns the parsed `WorkspaceConfig` on success.
///
/// This is a fast path for `nix-workspace info` — it reads the config
/// structure without applying contracts, so it works even if the config
/// has validation errors.
pub fn eval_workspace_config(workspace_root: &Path) -> Result<NickelResult> {
    let workspace_ncl = workspace_root.join("workspace.ncl");

    if !workspace_ncl.is_file() {
        anyhow::bail!("No workspace.ncl found at {}", workspace_ncl.display());
    }

    eval_raw(&workspace_ncl)
}

/// Parse the JSON output of a successful Nickel evaluation into a
/// `WorkspaceConfig`.
pub fn parse_workspace_config(json: &str) -> Result<WorkspaceConfig> {
    serde_json::from_str(json).context("Failed to parse workspace config JSON from Nickel output")
}

/// Evaluate and parse the workspace config in one step.
///
/// Returns `Ok(config)` on success, or a diagnostic report on failure.
pub fn load_workspace_config(
    workspace_root: &Path,
) -> Result<std::result::Result<WorkspaceConfig, DiagnosticReport>> {
    let result = eval_workspace_config(workspace_root)?;

    if result.success {
        let config = parse_workspace_config(&result.stdout)?;
        Ok(Ok(config))
    } else {
        Ok(Err(result.diagnostics))
    }
}

// ── Contract validation ───────────────────────────────────────────

/// Validate `workspace.ncl` against the `WorkspaceConfig` contract.
///
/// This generates a wrapper `.ncl` file that imports the workspace config
/// and applies the contract, mirroring what `lib/eval-nickel.nix` does on
/// the Nix side.
///
/// # Arguments
///
/// * `workspace_root` — Path to the workspace directory containing `workspace.ncl`
/// * `contracts_dir` — Path to the nix-workspace contracts directory
///   (typically from the nix-workspace flake or a local checkout)
pub fn validate_workspace(workspace_root: &Path, contracts_dir: &Path) -> Result<NickelResult> {
    let workspace_ncl = workspace_root.join("workspace.ncl");
    let contracts_dir = contracts_dir.canonicalize().with_context(|| {
        format!(
            "Contracts directory does not exist: {}",
            contracts_dir.display()
        )
    })?;

    if !workspace_ncl.is_file() {
        anyhow::bail!("No workspace.ncl found at {}", workspace_ncl.display());
    }

    let workspace_ncl_abs = workspace_ncl.canonicalize()?;

    // Generate a wrapper that imports the workspace config and applies
    // the WorkspaceConfig contract.
    let wrapper_source = format!(
        r#"let {{ WorkspaceConfig, .. }} = import "{contracts}/workspace.ncl" in
(import "{workspace}") | WorkspaceConfig
"#,
        contracts = contracts_dir.display(),
        workspace = workspace_ncl_abs.display(),
    );

    eval_source(&wrapper_source, workspace_root)
}

/// Validate a workspace config with plugin contract extensions.
///
/// When plugins are loaded, the wrapper applies extended contracts that
/// include plugin-specific fields.
///
/// # Arguments
///
/// * `workspace_root` — Path to the workspace directory
/// * `contracts_dir` — Path to the nix-workspace contracts directory
/// * `plugins_dir` — Path to the nix-workspace plugins directory
/// * `plugin_names` — List of plugin names (e.g., `["nix-workspace-rust"]`)
pub fn validate_workspace_with_plugins(
    workspace_root: &Path,
    contracts_dir: &Path,
    plugins_dir: &Path,
    plugin_names: &[String],
) -> Result<NickelResult> {
    let workspace_ncl = workspace_root.join("workspace.ncl");
    let contracts_dir = contracts_dir.canonicalize().with_context(|| {
        format!(
            "Contracts directory does not exist: {}",
            contracts_dir.display()
        )
    })?;
    let plugins_dir = plugins_dir.canonicalize().with_context(|| {
        format!(
            "Plugins directory does not exist: {}",
            plugins_dir.display()
        )
    })?;

    if !workspace_ncl.is_file() {
        anyhow::bail!("No workspace.ncl found at {}", workspace_ncl.display());
    }

    if plugin_names.is_empty() {
        return validate_workspace(workspace_root, &contracts_dir);
    }

    let workspace_ncl_abs = workspace_ncl.canonicalize()?;

    // Build the plugin preamble: import each plugin and build extended contracts
    let mut source = String::new();

    // Import base contracts
    source.push_str(&format!(
        "let {{ WorkspaceConfig, mkWorkspaceConfig, .. }} = import \"{}/workspace.ncl\" in\n",
        contracts_dir.display()
    ));
    source.push_str(&format!(
        "let {{ PackageConfig, .. }} = import \"{}/package.ncl\" in\n",
        contracts_dir.display()
    ));
    source.push_str(&format!(
        "let {{ ShellConfig, .. }} = import \"{}/shell.ncl\" in\n",
        contracts_dir.display()
    ));
    source.push_str(&format!(
        "let {{ PluginConfig, .. }} = import \"{}/plugin.ncl\" in\n",
        contracts_dir.display()
    ));

    // Import each plugin
    for (i, plugin_name) in plugin_names.iter().enumerate() {
        // Plugin names are like "nix-workspace-rust" → directory is "rust"
        let short_name = plugin_name
            .strip_prefix("nix-workspace-")
            .unwrap_or(plugin_name);

        let plugin_ncl = plugins_dir.join(short_name).join("plugin.ncl");
        if !plugin_ncl.is_file() {
            anyhow::bail!(
                "Plugin '{}' not found at {}",
                plugin_name,
                plugin_ncl.display()
            );
        }

        source.push_str(&format!(
            "let plugin_{i} = (import \"{}\") | PluginConfig in\n",
            plugin_ncl.display()
        ));
    }

    // Build extended package contract by merging plugin extensions
    source.push_str("let ExtPkg = PackageConfig");
    for (i, _) in plugin_names.iter().enumerate() {
        source.push_str(&format!(" & (plugin_{i}.extend.PackageConfig)"));
    }
    source.push_str(" in\n");

    // Build extended shell contract
    source.push_str("let ExtShell = ShellConfig");
    for (i, _) in plugin_names.iter().enumerate() {
        source.push_str(&format!(" & (plugin_{i}.extend.ShellConfig)"));
    }
    source.push_str(" in\n");

    // Build the extended workspace contract and apply it
    source.push_str("let ExtWorkspaceConfig = mkWorkspaceConfig ExtPkg ExtShell in\n");
    source.push_str(&format!(
        "(import \"{}\") | ExtWorkspaceConfig\n",
        workspace_ncl_abs.display()
    ));

    eval_source(&source, workspace_root)
}

// ── Typecheck ─────────────────────────────────────────────────────

/// Typecheck a single `.ncl` file with `nickel typecheck`.
pub fn typecheck(ncl_path: &Path) -> Result<NickelResult> {
    let nickel = find_nickel()?;

    let output = Command::new(&nickel)
        .arg("typecheck")
        .arg(ncl_path)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute nickel typecheck on {}",
                ncl_path.display()
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    let diagnostics = if success {
        DiagnosticReport::new()
    } else {
        diagnostics::parse_nickel_error(&stderr)
    };

    Ok(NickelResult {
        success,
        stdout,
        stderr,
        diagnostics,
    })
}

// ── Contracts directory resolution ────────────────────────────────

/// Attempt to find the nix-workspace contracts directory.
///
/// Search order:
/// 1. `$NIX_WORKSPACE_CONTRACTS` environment variable
/// 2. Sibling `contracts/` directory relative to the CLI binary
/// 3. Well-known Nix store paths (from `nix eval`)
///
/// Returns `None` if the contracts directory cannot be found. In that case,
/// the CLI can still do discovery-based operations but cannot validate
/// contracts.
pub fn find_contracts_dir() -> Option<PathBuf> {
    // 1. Environment variable
    if let Ok(dir) = std::env::var("NIX_WORKSPACE_CONTRACTS") {
        let path = PathBuf::from(dir);
        if path.is_dir() && path.join("workspace.ncl").is_file() {
            return Some(path);
        }
    }

    // 2. Relative to the CLI binary (works for local development)
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        // Check ../contracts (if CLI is in cli/target/debug or similar)
        for ancestor in exe_dir.ancestors().take(5) {
            let candidate = ancestor.join("contracts");
            if candidate.is_dir() && candidate.join("workspace.ncl").is_file() {
                return Some(candidate);
            }
        }
    }

    // 3. Check if we're inside a nix-workspace checkout (workspace root has contracts/)
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors().take(10) {
            let candidate = ancestor.join("contracts");
            if candidate.is_dir()
                && candidate.join("workspace.ncl").is_file()
                && ancestor.join("flake.nix").is_file()
            {
                return Some(candidate);
            }
        }
    }

    None
}

/// Attempt to find the nix-workspace plugins directory.
///
/// Mirrors the search logic of `find_contracts_dir` but for `plugins/`.
pub fn find_plugins_dir() -> Option<PathBuf> {
    // 1. Environment variable
    if let Ok(dir) = std::env::var("NIX_WORKSPACE_PLUGINS") {
        let path = PathBuf::from(dir);
        if path.is_dir() {
            return Some(path);
        }
    }

    // 2. Relative to the CLI binary
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        for ancestor in exe_dir.ancestors().take(5) {
            let candidate = ancestor.join("plugins");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }

    // 3. Check workspace checkout
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors().take(10) {
            let candidate = ancestor.join("plugins");
            if candidate.is_dir() && ancestor.join("flake.nix").is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

// ── Diagnostic helpers ────────────────────────────────────────────

/// Create a diagnostic for a missing tool.
pub fn missing_tool_diagnostic(tool: &str, install_hint: &str) -> Diagnostic {
    Diagnostic::error(
        diagnostics::codes::MISSING_TOOL,
        format!("`{tool}` is not installed or not on $PATH"),
    )
    .with_hint(install_hint)
}

/// Create a diagnostic for a tool execution failure.
pub fn tool_failed_diagnostic(tool: &str, stderr: &str) -> Diagnostic {
    Diagnostic::error(
        diagnostics::codes::TOOL_FAILED,
        format!("`{tool}` exited with an error"),
    )
    .with_hint(stderr.lines().take(5).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_nickel_returns_path_or_error() {
        // This test is environment-dependent — it passes if nickel is installed
        // and fails gracefully if not.
        match find_nickel() {
            Ok(path) => assert!(path.exists()),
            Err(e) => assert!(e.to_string().contains("nickel")),
        }
    }

    #[test]
    fn test_missing_tool_diagnostic() {
        let diag =
            missing_tool_diagnostic("nickel", "Install via nix profile install nixpkgs#nickel");
        assert_eq!(diag.code, diagnostics::codes::MISSING_TOOL);
        assert_eq!(diag.severity, diagnostics::Severity::Error);
        assert!(diag.message.contains("nickel"));
        assert!(diag.hint.is_some());
    }

    #[test]
    fn test_tool_failed_diagnostic() {
        let diag = tool_failed_diagnostic("nickel", "error: something went wrong\ndetails here");
        assert_eq!(diag.code, diagnostics::codes::TOOL_FAILED);
        assert!(diag.hint.unwrap().contains("something went wrong"));
    }

    #[test]
    fn test_find_contracts_dir_env() {
        // Test with a temp directory
        let tmp = tempfile::TempDir::new().unwrap();
        let contracts = tmp.path().join("contracts");
        std::fs::create_dir(&contracts).unwrap();
        std::fs::write(contracts.join("workspace.ncl"), "{}").unwrap();

        // SAFETY: This test is not run in parallel with other tests that
        // read NIX_WORKSPACE_CONTRACTS.
        unsafe {
            std::env::set_var("NIX_WORKSPACE_CONTRACTS", contracts.to_str().unwrap());
        }
        let result = find_contracts_dir();
        unsafe {
            std::env::remove_var("NIX_WORKSPACE_CONTRACTS");
        }

        assert!(result.is_some());
        assert_eq!(result.unwrap(), contracts);
    }
}
