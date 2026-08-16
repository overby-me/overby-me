#![allow(dead_code)]
//! Workspace discovery — Rust-side directory scanning.
//!
//! This module mirrors the Nix-side discovery logic in `lib/discover.nix`
//! but runs natively in the CLI for fast, no-evaluation workspace inspection.
//!
//! It scans for:
//! - `workspace.ncl` files (root and subworkspaces)
//! - Convention directories (`packages/`, `shells/`, `machines/`, etc.)
//! - `.ncl` files within convention directories
//! - Subworkspaces (subdirectories containing `workspace.ncl`)
//! - `flake.nix` presence (to decide whether on-the-fly generation is needed)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ── Convention definitions ────────────────────────────────────────

/// A convention directory mapping, matching `lib/discover.nix` defaults.
#[derive(Debug, Clone)]
pub struct Convention {
    /// Convention name (e.g., `"packages"`).
    pub name: &'static str,
    /// Directory path relative to workspace root (e.g., `"packages"`).
    pub dir: &'static str,
    /// Flake output type (e.g., `"packages"`).
    pub output: &'static str,
}

/// The default convention directories, kept in sync with `lib/discover.nix`.
pub const DEFAULT_CONVENTIONS: &[Convention] = &[
    Convention {
        name: "packages",
        dir: "packages",
        output: "packages",
    },
    Convention {
        name: "shells",
        dir: "shells",
        output: "devShells",
    },
    Convention {
        name: "modules",
        dir: "modules",
        output: "nixosModules",
    },
    Convention {
        name: "home",
        dir: "home",
        output: "homeModules",
    },
    Convention {
        name: "overlays",
        dir: "overlays",
        output: "overlays",
    },
    Convention {
        name: "machines",
        dir: "machines",
        output: "nixosConfigurations",
    },
    Convention {
        name: "templates",
        dir: "templates",
        output: "templates",
    },
    Convention {
        name: "checks",
        dir: "checks",
        output: "checks",
    },
    Convention {
        name: "lib",
        dir: "lib",
        output: "lib",
    },
];

/// Well-known directories that should never be treated as subworkspaces.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".github",
    ".gitlab",
    "node_modules",
    "result",
    ".direnv",
    ".devenv",
    "target",
    "_build",
    ".jj",
];

// ── Discovered workspace ──────────────────────────────────────────

/// A discovered `.ncl` file within a convention directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredEntry {
    /// Output name derived from the filename (sans `.ncl` extension).
    /// For `packages/hello.ncl` this is `"hello"`.
    pub name: String,

    /// Absolute path to the `.ncl` file.
    pub path: PathBuf,
}

/// Discovered outputs for a single convention directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConventionOutputs {
    /// Convention name (e.g., `"packages"`).
    pub convention: String,

    /// Directory path relative to workspace root.
    pub dir: String,

    /// Flake output type (e.g., `"packages"`).
    pub output: String,

    /// Whether the convention directory exists on disk.
    pub exists: bool,

    /// Discovered `.ncl` files within the convention directory.
    pub entries: Vec<DiscoveredEntry>,
}

/// A discovered subworkspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSubworkspace {
    /// Directory name (used as the namespace prefix).
    pub name: String,

    /// Absolute path to the subworkspace root.
    pub path: PathBuf,

    /// Convention outputs discovered within the subworkspace.
    pub conventions: BTreeMap<String, ConventionOutputs>,
}

/// The full result of workspace discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiscovery {
    /// Absolute path to the workspace root.
    pub root: PathBuf,

    /// Whether `workspace.ncl` exists at the root.
    pub has_workspace_ncl: bool,

    /// Whether `flake.nix` exists at the root.
    pub has_flake_nix: bool,

    /// Whether `flake.lock` exists at the root.
    pub has_flake_lock: bool,

    /// Convention outputs discovered at the root level.
    pub conventions: BTreeMap<String, ConventionOutputs>,

    /// Discovered subworkspaces.
    pub subworkspaces: Vec<DiscoveredSubworkspace>,
}

impl WorkspaceDiscovery {
    /// Total number of discovered `.ncl` entries across all root conventions.
    pub fn root_entry_count(&self) -> usize {
        self.conventions.values().map(|c| c.entries.len()).sum()
    }

    /// Total number of discovered subworkspaces.
    pub fn subworkspace_count(&self) -> usize {
        self.subworkspaces.len()
    }

    /// Get the names of all convention directories that have at least one entry.
    pub fn active_conventions(&self) -> Vec<&str> {
        self.conventions
            .values()
            .filter(|c| !c.entries.is_empty())
            .map(|c| c.convention.as_str())
            .collect()
    }

    /// Get all discovered package names (from the `packages` convention).
    pub fn package_names(&self) -> Vec<&str> {
        self.conventions
            .get("packages")
            .map(|c| c.entries.iter().map(|e| e.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all discovered shell names (from the `shells` convention).
    pub fn shell_names(&self) -> Vec<&str> {
        self.conventions
            .get("shells")
            .map(|c| c.entries.iter().map(|e| e.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all discovered machine names (from the `machines` convention).
    pub fn machine_names(&self) -> Vec<&str> {
        self.conventions
            .get("machines")
            .map(|c| c.entries.iter().map(|e| e.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all discovered module names (from the `modules` convention).
    pub fn module_names(&self) -> Vec<&str> {
        self.conventions
            .get("modules")
            .map(|c| c.entries.iter().map(|e| e.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all discovered home module names (from the `home` convention).
    pub fn home_names(&self) -> Vec<&str> {
        self.conventions
            .get("home")
            .map(|c| c.entries.iter().map(|e| e.name.as_str()).collect())
            .unwrap_or_default()
    }
}

// ── Discovery functions ───────────────────────────────────────────

/// Discover `.ncl` files in a single convention directory.
///
/// Returns a sorted list of discovered entries. Only regular files with the
/// `.ncl` extension are included.
fn discover_ncl_files(convention_dir: &Path) -> Result<Vec<DiscoveredEntry>> {
    let mut entries = Vec::new();

    if !convention_dir.is_dir() {
        return Ok(entries);
    }

    // Only scan the immediate directory — convention directories are flat
    let read_dir = std::fs::read_dir(convention_dir)
        .with_context(|| format!("Failed to read directory: {}", convention_dir.display()))?;

    for dir_entry in read_dir {
        let dir_entry = dir_entry?;
        let path = dir_entry.path();

        // Only regular files (or symlinks to files)
        if !path.is_file() {
            continue;
        }

        // Only .ncl files
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("ncl") {
            continue;
        }

        // Derive the output name from the filename
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());

        if let Some(name) = name {
            entries.push(DiscoveredEntry {
                name,
                path: path.canonicalize().unwrap_or(path),
            });
        }
    }

    // Sort by name for deterministic output
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(entries)
}

/// Discover all convention outputs for a workspace root.
fn discover_conventions(root: &Path) -> Result<BTreeMap<String, ConventionOutputs>> {
    let mut result = BTreeMap::new();

    for conv in DEFAULT_CONVENTIONS {
        let dir_path = root.join(conv.dir);
        let exists = dir_path.is_dir();
        let entries = if exists {
            discover_ncl_files(&dir_path)?
        } else {
            Vec::new()
        };

        result.insert(
            conv.name.to_string(),
            ConventionOutputs {
                convention: conv.name.to_string(),
                dir: conv.dir.to_string(),
                output: conv.output.to_string(),
                exists,
                entries,
            },
        );
    }

    Ok(result)
}

/// Check if a directory name is a convention directory.
fn is_convention_dir(name: &str) -> bool {
    DEFAULT_CONVENTIONS.iter().any(|c| c.dir == name)
}

/// Discover subworkspaces in the workspace root.
///
/// A subworkspace is any immediate subdirectory of the root that contains
/// a `workspace.ncl` file. Hidden directories, well-known non-workspace
/// directories (e.g., `node_modules`, `.git`), and convention directories
/// are skipped.
fn discover_subworkspaces(root: &Path) -> Result<Vec<DiscoveredSubworkspace>> {
    let mut subworkspaces = Vec::new();

    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return Ok(subworkspaces),
    };

    for dir_entry in read_dir {
        let dir_entry = dir_entry?;
        let path = dir_entry.path();

        // Must be a directory (or symlink to one)
        if !path.is_dir() {
            continue;
        }

        let name = match dir_entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }

        // Skip well-known non-workspace directories
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }

        // Skip convention directories — they are not subworkspaces
        if is_convention_dir(&name) {
            continue;
        }

        // Must contain workspace.ncl
        let workspace_ncl = path.join("workspace.ncl");
        if !workspace_ncl.is_file() {
            continue;
        }

        // Discover convention outputs within the subworkspace
        let conventions = discover_conventions(&path)?;

        subworkspaces.push(DiscoveredSubworkspace {
            name,
            path: path.canonicalize().unwrap_or(path),
            conventions,
        });
    }

    // Sort by name for deterministic output
    subworkspaces.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(subworkspaces)
}

/// Perform full workspace discovery from a root path.
///
/// This is the main entry point. It:
/// 1. Checks for `workspace.ncl` and `flake.nix`
/// 2. Scans all convention directories for `.ncl` files
/// 3. Discovers subworkspaces (subdirectories with `workspace.ncl`)
/// 4. Recursively discovers convention outputs in each subworkspace
pub fn discover_workspace(root: &Path) -> Result<WorkspaceDiscovery> {
    let root = root
        .canonicalize()
        .with_context(|| format!("Workspace root does not exist: {}", root.display()))?;

    let has_workspace_ncl = root.join("workspace.ncl").is_file();
    let has_flake_nix = root.join("flake.nix").is_file();
    let has_flake_lock = root.join("flake.lock").is_file();

    let conventions = discover_conventions(&root)?;
    let subworkspaces = discover_subworkspaces(&root)?;

    Ok(WorkspaceDiscovery {
        root,
        has_workspace_ncl,
        has_flake_nix,
        has_flake_lock,
        conventions,
        subworkspaces,
    })
}

/// Find the workspace root by walking up from the given directory.
///
/// Looks for a directory containing `workspace.ncl`. Stops at filesystem
/// boundaries or after 100 levels (safety limit).
pub fn find_workspace_root(start: &Path) -> Result<PathBuf> {
    let start = start
        .canonicalize()
        .with_context(|| format!("Path does not exist: {}", start.display()))?;

    let mut current = start.as_path();
    let mut depth = 0;
    let max_depth = 100;

    loop {
        if current.join("workspace.ncl").is_file() {
            return Ok(current.to_path_buf());
        }

        depth += 1;
        if depth > max_depth {
            anyhow::bail!(
                "Could not find workspace.ncl within {} levels above {}",
                max_depth,
                start.display()
            );
        }

        match current.parent() {
            Some(parent) if parent != current => {
                current = parent;
            }
            _ => {
                anyhow::bail!(
                    "Could not find workspace.ncl in any parent directory of {}",
                    start.display()
                );
            }
        }
    }
}

/// Resolve the workspace root, checking --workspace-dir override first,
/// then searching upwards from the current directory.
pub fn resolve_workspace_root(workspace_dir: Option<&Path>) -> Result<PathBuf> {
    match workspace_dir {
        Some(dir) => {
            let dir = dir.canonicalize().with_context(|| {
                format!("Workspace directory does not exist: {}", dir.display())
            })?;
            if !dir.join("workspace.ncl").is_file() {
                anyhow::bail!(
                    "No workspace.ncl found in specified directory: {}",
                    dir.display()
                );
            }
            Ok(dir)
        }
        None => {
            let cwd = std::env::current_dir().context("Failed to get current directory")?;
            find_workspace_root(&cwd)
        }
    }
}

// ── Workspace config (partial, from nickel export) ────────────────

/// Partial workspace config as read from `nickel export workspace.ncl`.
///
/// This is a subset of the full WorkspaceConfig — just enough for the
/// CLI to display info and make routing decisions. Full validation
/// happens through Nickel contracts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Workspace name.
    #[serde(default)]
    pub name: String,

    /// Description.
    #[serde(default)]
    pub description: Option<String>,

    /// Target systems.
    #[serde(default)]
    pub systems: Vec<String>,

    /// Plugin names.
    #[serde(default)]
    pub plugins: Vec<String>,

    /// Declared packages (may be empty if all are auto-discovered).
    #[serde(default)]
    pub packages: BTreeMap<String, serde_json::Value>,

    /// Declared shells.
    #[serde(default)]
    pub shells: BTreeMap<String, serde_json::Value>,

    /// Declared machines.
    #[serde(default)]
    pub machines: BTreeMap<String, serde_json::Value>,

    /// Declared modules.
    #[serde(default)]
    pub modules: BTreeMap<String, serde_json::Value>,

    /// Declared home modules.
    #[serde(default)]
    pub home: BTreeMap<String, serde_json::Value>,

    /// Declared dependencies (subworkspace references).
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

// ── Summary types for display ─────────────────────────────────────

/// Summary of a workspace suitable for `nix-workspace info` display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    /// Workspace name (from config, or directory name as fallback).
    pub name: String,

    /// Description (from config).
    pub description: Option<String>,

    /// Workspace root path.
    pub root: PathBuf,

    /// Target systems.
    pub systems: Vec<String>,

    /// Whether a `flake.nix` is present.
    pub has_flake: bool,

    /// Active plugins.
    pub plugins: Vec<String>,

    /// Summary of convention outputs.
    pub conventions: Vec<ConventionSummary>,

    /// Subworkspace summaries.
    pub subworkspaces: Vec<SubworkspaceSummary>,
}

/// Summary of a single convention directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConventionSummary {
    /// Convention name (e.g., `"packages"`).
    pub name: String,

    /// Directory path relative to root.
    pub dir: String,

    /// Flake output type.
    pub output: String,

    /// Number of discovered entries.
    pub count: usize,

    /// Entry names.
    pub entries: Vec<String>,
}

/// Summary of a subworkspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubworkspaceSummary {
    /// Subworkspace directory name.
    pub name: String,

    /// Number of discovered entries across all conventions.
    pub entry_count: usize,

    /// Active convention names.
    pub active_conventions: Vec<String>,
}

impl WorkspaceSummary {
    /// Build a summary from discovery results and optional workspace config.
    pub fn from_discovery(
        discovery: &WorkspaceDiscovery,
        config: Option<&WorkspaceConfig>,
    ) -> Self {
        let name = config
            .map(|c| c.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                discovery
                    .root
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown")
                    .to_string()
            });

        let description = config.and_then(|c| c.description.clone());

        let systems = config
            .map(|c| c.systems.clone())
            .unwrap_or_else(|| vec!["x86_64-linux".into(), "aarch64-linux".into()]);

        let plugins = config.map(|c| c.plugins.clone()).unwrap_or_default();

        let conventions: Vec<ConventionSummary> = discovery
            .conventions
            .values()
            .filter(|c| !c.entries.is_empty())
            .map(|c| ConventionSummary {
                name: c.convention.clone(),
                dir: c.dir.clone(),
                output: c.output.clone(),
                count: c.entries.len(),
                entries: c.entries.iter().map(|e| e.name.clone()).collect(),
            })
            .collect();

        let subworkspaces: Vec<SubworkspaceSummary> = discovery
            .subworkspaces
            .iter()
            .map(|sw| {
                let entry_count: usize = sw.conventions.values().map(|c| c.entries.len()).sum();
                let active_conventions: Vec<String> = sw
                    .conventions
                    .values()
                    .filter(|c| !c.entries.is_empty())
                    .map(|c| c.convention.clone())
                    .collect();
                SubworkspaceSummary {
                    name: sw.name.clone(),
                    entry_count,
                    active_conventions,
                }
            })
            .collect();

        Self {
            name,
            description,
            root: discovery.root.clone(),
            has_flake: discovery.has_flake_nix,
            systems,
            plugins,
            conventions,
            subworkspaces,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test workspace with the given structure.
    fn setup_workspace(dirs: &[&str], files: &[&str]) -> TempDir {
        let tmp = TempDir::new().unwrap();
        for dir in dirs {
            fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        for file in files {
            let path = tmp.path().join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "# test").unwrap();
        }
        tmp
    }

    #[test]
    fn test_discover_empty_workspace() {
        let tmp = setup_workspace(&[], &["workspace.ncl"]);
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert!(discovery.has_workspace_ncl);
        assert!(!discovery.has_flake_nix);
        assert!(!discovery.has_flake_lock);
        assert_eq!(discovery.root_entry_count(), 0);
        assert_eq!(discovery.subworkspace_count(), 0);
    }

    #[test]
    fn test_discover_with_packages() {
        let tmp = setup_workspace(
            &["packages"],
            &["workspace.ncl", "packages/hello.ncl", "packages/world.ncl"],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert_eq!(discovery.package_names(), vec!["hello", "world"]);
        assert_eq!(discovery.root_entry_count(), 2);
    }

    #[test]
    fn test_discover_with_shells_and_machines() {
        let tmp = setup_workspace(
            &["shells", "machines"],
            &[
                "workspace.ncl",
                "shells/default.ncl",
                "machines/gravitas.ncl",
                "machines/sleeper.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert_eq!(discovery.shell_names(), vec!["default"]);
        assert_eq!(discovery.machine_names(), vec!["gravitas", "sleeper"]);
        assert_eq!(discovery.root_entry_count(), 3);
    }

    #[test]
    fn test_discover_ignores_non_ncl_files() {
        let tmp = setup_workspace(
            &["packages"],
            &[
                "workspace.ncl",
                "packages/hello.ncl",
                "packages/readme.md",
                "packages/.hidden.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        // .hidden.ncl is a valid file name with .ncl extension, so it's included
        let names = discovery.package_names();
        assert!(names.contains(&"hello"));
        // readme.md should NOT be included
        assert!(!names.iter().any(|n| n.contains("readme")));
    }

    #[test]
    fn test_discover_subworkspaces() {
        let tmp = setup_workspace(
            &["lib-a/packages", "app-b/packages"],
            &[
                "workspace.ncl",
                "lib-a/workspace.ncl",
                "lib-a/packages/default.ncl",
                "app-b/workspace.ncl",
                "app-b/packages/default.ncl",
                "app-b/packages/extra.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert_eq!(discovery.subworkspace_count(), 2);
        assert_eq!(discovery.subworkspaces[0].name, "app-b");
        assert_eq!(discovery.subworkspaces[1].name, "lib-a");
    }

    #[test]
    fn test_discover_skips_convention_dirs_as_subworkspaces() {
        // Even if packages/ contains a workspace.ncl, it should NOT be a subworkspace
        let tmp = setup_workspace(
            &["packages"],
            &[
                "workspace.ncl",
                "packages/workspace.ncl",
                "packages/hello.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert_eq!(discovery.subworkspace_count(), 0);
        assert_eq!(discovery.package_names(), vec!["hello", "workspace"]);
    }

    #[test]
    fn test_discover_skips_hidden_and_known_dirs() {
        let tmp = setup_workspace(
            &[".git", "node_modules", "result"],
            &[
                "workspace.ncl",
                ".git/workspace.ncl",
                "node_modules/workspace.ncl",
                "result/workspace.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert_eq!(discovery.subworkspace_count(), 0);
    }

    #[test]
    fn test_discover_flake_presence() {
        let tmp = setup_workspace(&[], &["workspace.ncl", "flake.nix", "flake.lock"]);
        let discovery = discover_workspace(tmp.path()).unwrap();

        assert!(discovery.has_workspace_ncl);
        assert!(discovery.has_flake_nix);
        assert!(discovery.has_flake_lock);
    }

    #[test]
    fn test_find_workspace_root() {
        let tmp = setup_workspace(&["sub/deep/nested"], &["workspace.ncl"]);
        let nested = tmp.path().join("sub/deep/nested");
        let found = find_workspace_root(&nested).unwrap();
        assert_eq!(found, tmp.path().canonicalize().unwrap());
    }

    #[test]
    fn test_find_workspace_root_not_found() {
        let tmp = TempDir::new().unwrap();
        let result = find_workspace_root(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_workspace_root_explicit() {
        let tmp = setup_workspace(&[], &["workspace.ncl"]);
        let root = resolve_workspace_root(Some(tmp.path())).unwrap();
        assert_eq!(root, tmp.path().canonicalize().unwrap());
    }

    #[test]
    fn test_resolve_workspace_root_explicit_missing() {
        let tmp = TempDir::new().unwrap();
        // No workspace.ncl
        let result = resolve_workspace_root(Some(tmp.path()));
        assert!(result.is_err());
    }

    #[test]
    fn test_active_conventions() {
        let tmp = setup_workspace(
            &["packages", "shells"],
            &["workspace.ncl", "packages/hello.ncl", "shells/default.ncl"],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();
        let active = discovery.active_conventions();

        assert!(active.contains(&"packages"));
        assert!(active.contains(&"shells"));
        assert!(!active.contains(&"machines"));
    }

    #[test]
    fn test_workspace_summary_from_discovery() {
        let tmp = setup_workspace(
            &["packages", "lib-a/packages"],
            &[
                "workspace.ncl",
                "packages/hello.ncl",
                "lib-a/workspace.ncl",
                "lib-a/packages/default.ncl",
            ],
        );
        let discovery = discover_workspace(tmp.path()).unwrap();

        let config = WorkspaceConfig {
            name: "test-project".into(),
            description: Some("A test".into()),
            systems: vec!["x86_64-linux".into()],
            plugins: vec!["nix-workspace-rust".into()],
            ..Default::default()
        };

        let summary = WorkspaceSummary::from_discovery(&discovery, Some(&config));

        assert_eq!(summary.name, "test-project");
        assert_eq!(summary.description.as_deref(), Some("A test"));
        assert_eq!(summary.systems, vec!["x86_64-linux"]);
        assert_eq!(summary.plugins, vec!["nix-workspace-rust"]);
        assert!(summary.has_flake == false);
        assert_eq!(summary.conventions.len(), 1); // only packages has entries
        assert_eq!(summary.subworkspaces.len(), 1);
        assert_eq!(summary.subworkspaces[0].name, "lib-a");
    }
}
