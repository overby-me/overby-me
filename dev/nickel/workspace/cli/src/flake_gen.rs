#![allow(dead_code)]
//! On-the-fly `flake.nix` generation for non-flake workspaces.
//!
//! When a workspace has `workspace.ncl` but no `flake.nix`, the CLI can
//! generate a temporary `flake.nix` that wraps the workspace using
//! nix-workspace. This enables `nix-workspace build` and `nix-workspace shell`
//! to work without the user having to write any Nix code.
//!
//! The generated flake is written to a temporary directory alongside a
//! symlink to the workspace root, so that Nix evaluation can find all the
//! workspace files while the flake itself lives outside the workspace
//! (avoiding dirty-git issues).
//!
//! # Environment variables
//!
//! - `NIX_WORKSPACE_FLAKE_REF` — Override the nix-workspace flake reference
//!   used in the generated `flake.nix`. Defaults to
//!   `"git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace"`.
//!
//! # Limitations
//!
//! - The generated flake uses `path:` to reference the workspace, which
//!   requires the workspace to be a git repo (or uses `--impure`).
//! - Flake lock is not persisted — each invocation may fetch inputs.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ── Constants ─────────────────────────────────────────────────────

/// Default flake reference for nix-workspace.
const DEFAULT_FLAKE_REF: &str = "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace";

/// Environment variable to override the nix-workspace flake reference.
const FLAKE_REF_ENV: &str = "NIX_WORKSPACE_FLAKE_REF";

// ── Flake generation ──────────────────────────────────────────────

/// Configuration for flake generation.
#[derive(Debug, Clone)]
pub struct FlakeGenConfig {
    /// The nix-workspace flake reference (e.g., `"git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace"`).
    pub nix_workspace_ref: String,

    /// Additional flake inputs to include (name → url).
    pub extra_inputs: Vec<(String, String)>,
}

impl Default for FlakeGenConfig {
    fn default() -> Self {
        let nix_workspace_ref =
            std::env::var(FLAKE_REF_ENV).unwrap_or_else(|_| DEFAULT_FLAKE_REF.to_string());

        Self {
            nix_workspace_ref,
            extra_inputs: Vec::new(),
        }
    }
}

/// Generate the content of a `flake.nix` that wraps a workspace.
///
/// The generated flake follows the standard nix-workspace integration
/// pattern from SPEC.md:
///
/// ```nix
/// {
///   inputs = {
///     nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
///     nix-workspace.url = "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace";
///   };
///   outputs = inputs:
///     inputs.nix-workspace ./. {
///       inherit inputs;
///     };
/// }
/// ```
///
/// # Arguments
///
/// * `workspace_root` — Absolute path to the workspace directory. This is
///   embedded as a `path:` reference in the generated flake so that Nix
///   can find the workspace files.
/// * `config` — Generation configuration (flake ref overrides, extra inputs).
pub fn generate_flake_nix(_workspace_root: &Path, config: &FlakeGenConfig) -> String {
    let mut inputs = String::new();

    // nixpkgs is always included
    inputs.push_str("    nixpkgs.url = \"github:NixOS/nixpkgs/nixos-unstable\";\n");

    // nix-workspace input
    inputs.push_str(&format!(
        "    nix-workspace.url = \"{}\";\n",
        config.nix_workspace_ref
    ));

    // Extra inputs
    for (name, url) in &config.extra_inputs {
        inputs.push_str(&format!("    {name}.url = \"{url}\";\n"));
    }

    format!(
        r#"{{
  description = "Auto-generated flake for nix-workspace";

  inputs = {{
{inputs}  }};

  outputs = inputs:
    inputs.nix-workspace {workspace_path} {{
      inherit inputs;
    }};
}}"#,
        inputs = inputs,
        workspace_path = "./.",
    )
}

/// Generate a `flake.nix` for in-place use within the workspace directory.
///
/// This writes the flake directly into the workspace root. The caller is
/// responsible for cleaning it up (or the user can keep it).
///
/// Returns the path to the generated `flake.nix`.
pub fn generate_flake_in_place(workspace_root: &Path, config: &FlakeGenConfig) -> Result<PathBuf> {
    let flake_path = workspace_root.join("flake.nix");

    if flake_path.exists() {
        anyhow::bail!(
            "flake.nix already exists at {}. \
             Use the existing flake or remove it first.",
            flake_path.display()
        );
    }

    let content = generate_flake_nix(workspace_root, config);
    std::fs::write(&flake_path, &content)
        .with_context(|| format!("Failed to write {}", flake_path.display()))?;

    Ok(flake_path)
}

/// A temporary flake environment for running Nix commands against a
/// workspace that doesn't have its own `flake.nix`.
///
/// The temporary directory is cleaned up when this struct is dropped.
#[derive(Debug)]
pub struct TempFlake {
    /// The temporary directory containing the generated `flake.nix`.
    _temp_dir: tempfile::TempDir,

    /// Path to the generated `flake.nix`.
    pub flake_nix_path: PathBuf,

    /// Path to the temporary directory (used as the flake root for nix commands).
    pub flake_root: PathBuf,

    /// The original workspace root (for display purposes).
    pub workspace_root: PathBuf,
}

impl TempFlake {
    /// Get the flake reference string suitable for `nix build` / `nix develop`.
    ///
    /// Returns a `path:` reference to the temporary directory.
    pub fn flake_ref(&self) -> String {
        format!("path:{}", self.flake_root.display())
    }

    /// Get a flake reference for a specific output.
    ///
    /// Example: `path:/tmp/nix-workspace-XXXX#hello`
    pub fn flake_ref_with_output(&self, output: &str) -> String {
        format!("{}#{}", self.flake_ref(), output)
    }
}

/// Create a temporary flake environment for a workspace without `flake.nix`.
///
/// This:
/// 1. Creates a temp directory
/// 2. Symlinks all workspace contents into it
/// 3. Writes a generated `flake.nix` into the temp directory
///
/// The returned `TempFlake` can be used to run Nix commands. It cleans
/// up the temp directory on drop.
///
/// # Arguments
///
/// * `workspace_root` — Absolute path to the workspace directory.
/// * `config` — Flake generation configuration.
pub fn create_temp_flake(workspace_root: &Path, config: &FlakeGenConfig) -> Result<TempFlake> {
    let workspace_root = workspace_root.canonicalize().with_context(|| {
        format!(
            "Workspace root does not exist: {}",
            workspace_root.display()
        )
    })?;

    let temp_dir = tempfile::Builder::new()
        .prefix("nix-workspace-flake-")
        .tempdir()
        .context("Failed to create temporary directory for flake")?;

    let flake_root = temp_dir.path().to_path_buf();

    // Symlink all workspace contents into the temp directory.
    // This allows Nix to find workspace.ncl, packages/, etc.
    let read_dir = std::fs::read_dir(&workspace_root).with_context(|| {
        format!(
            "Failed to read workspace directory: {}",
            workspace_root.display()
        )
    })?;

    for entry in read_dir {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip flake.nix and flake.lock if they exist (we generate our own)
        if name_str == "flake.nix" || name_str == "flake.lock" {
            continue;
        }

        let src = entry.path();
        let dst = flake_root.join(&name);

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, &dst)
            .with_context(|| format!("Failed to symlink {} -> {}", src.display(), dst.display()))?;

        #[cfg(not(unix))]
        {
            // On non-Unix, fall back to copying (Windows doesn't have
            // symlinks without elevated privileges).
            if src.is_dir() {
                copy_dir_recursive(&src, &dst)?;
            } else {
                std::fs::copy(&src, &dst).with_context(|| {
                    format!("Failed to copy {} -> {}", src.display(), dst.display())
                })?;
            }
        }
    }

    // Write the generated flake.nix
    let flake_content = generate_flake_nix(&workspace_root, config);
    let flake_nix_path = flake_root.join("flake.nix");
    std::fs::write(&flake_nix_path, &flake_content)
        .context("Failed to write generated flake.nix")?;

    Ok(TempFlake {
        _temp_dir: temp_dir,
        flake_nix_path,
        flake_root,
        workspace_root,
    })
}

/// Determine whether a workspace needs on-the-fly flake generation.
///
/// Returns `true` if the workspace has `workspace.ncl` but no `flake.nix`.
pub fn needs_flake_generation(workspace_root: &Path) -> bool {
    let has_workspace_ncl = workspace_root.join("workspace.ncl").is_file();
    let has_flake_nix = workspace_root.join("flake.nix").is_file();

    has_workspace_ncl && !has_flake_nix
}

// ── Init scaffolding ──────────────────────────────────────────────

/// Options for `nix-workspace init`.
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// Workspace name.
    pub name: String,

    /// Optional description.
    pub description: Option<String>,

    /// Target systems.
    pub systems: Vec<String>,

    /// Whether to generate a `flake.nix` alongside `workspace.ncl`.
    pub with_flake: bool,

    /// Convention directories to create.
    pub conventions: Vec<String>,

    /// Plugins to include.
    pub plugins: Vec<String>,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            systems: vec!["x86_64-linux".into(), "aarch64-linux".into()],
            with_flake: true,
            conventions: vec!["packages".into(), "shells".into()],
            plugins: Vec::new(),
        }
    }
}

/// Generate the content of a `workspace.ncl` file for `nix-workspace init`.
pub fn generate_workspace_ncl(opts: &InitOptions) -> String {
    let mut fields = Vec::new();

    // name
    fields.push(format!("  name = \"{}\",", opts.name));

    // description
    if let Some(ref desc) = opts.description {
        fields.push(format!("  description = \"{desc}\","));
    }

    // systems
    let systems_str: Vec<String> = opts.systems.iter().map(|s| format!("\"{s}\"")).collect();
    fields.push(format!("  systems = [{}],", systems_str.join(", ")));

    // plugins
    if !opts.plugins.is_empty() {
        let plugins_str: Vec<String> = opts.plugins.iter().map(|p| format!("\"{p}\"")).collect();
        fields.push(format!("\n  plugins = [{}],", plugins_str.join(", ")));
    }

    format!(
        r#"# {name} — workspace configuration
#
# Validated by nix-workspace against the WorkspaceConfig contract.
# See: https://tangled.org/@overby.me/overby.me/tree/main/nix-workspace

{{
{fields}
}}
"#,
        name = opts.name,
        fields = fields.join("\n"),
    )
}

/// Generate a default shell configuration file.
pub fn generate_default_shell_ncl() -> &'static str {
    r#"# Default development shell
#
# Enter with: nix develop (or nix-workspace shell)

{
  packages = [],
  shell-hook = m%"
    echo "Welcome to the dev shell!"
  "%,
}
"#
}

/// Generate a sample package configuration file.
pub fn generate_sample_package_ncl(name: &str) -> String {
    format!(
        r#"# Package: {name}
#
# Build with: nix build .#{name} (or nix-workspace build {name})

{{
  description = "The {name} package",
  build-system = "generic",
}}
"#
    )
}

/// Initialize a new workspace in the given directory.
///
/// Creates:
/// - `workspace.ncl`
/// - `flake.nix` (if `opts.with_flake` is true)
/// - Convention directories (e.g., `packages/`, `shells/`)
/// - Sample files in convention directories
///
/// Returns a list of created file paths (relative to the workspace root).
pub fn init_workspace(root: &Path, opts: &InitOptions) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();

    // Refuse to overwrite an existing workspace
    let workspace_ncl = root.join("workspace.ncl");
    if workspace_ncl.exists() {
        anyhow::bail!(
            "workspace.ncl already exists at {}. \
             Remove it first or use a different directory.",
            workspace_ncl.display()
        );
    }

    // Create the root directory if needed
    if !root.exists() {
        std::fs::create_dir_all(root)
            .with_context(|| format!("Failed to create directory: {}", root.display()))?;
    }

    // Write workspace.ncl
    let workspace_content = generate_workspace_ncl(opts);
    std::fs::write(&workspace_ncl, &workspace_content)
        .with_context(|| format!("Failed to write {}", workspace_ncl.display()))?;
    created.push(PathBuf::from("workspace.ncl"));

    // Write flake.nix
    if opts.with_flake {
        let flake_path = root.join("flake.nix");
        if !flake_path.exists() {
            let flake_config = FlakeGenConfig::default();
            let flake_content = generate_flake_nix(root, &flake_config);
            std::fs::write(&flake_path, &flake_content)
                .with_context(|| format!("Failed to write {}", flake_path.display()))?;
            created.push(PathBuf::from("flake.nix"));
        }
    }

    // Create convention directories and sample files
    for conv in &opts.conventions {
        let dir = root.join(conv);
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
            created.push(PathBuf::from(conv));
        }

        // Create sample files
        match conv.as_str() {
            "shells" => {
                let default_shell = dir.join("default.ncl");
                if !default_shell.exists() {
                    std::fs::write(&default_shell, generate_default_shell_ncl())
                        .with_context(|| format!("Failed to write {}", default_shell.display()))?;
                    created.push(PathBuf::from(format!("{conv}/default.ncl")));
                }
            }
            "packages" => {
                let sample = dir.join(format!("{}.ncl", opts.name));
                if !sample.exists() {
                    std::fs::write(&sample, generate_sample_package_ncl(&opts.name))
                        .with_context(|| format!("Failed to write {}", sample.display()))?;
                    created.push(PathBuf::from(format!("{conv}/{}.ncl", opts.name)));
                }
            }
            _ => {
                // Create a .gitkeep so the directory is tracked by git
                let gitkeep = dir.join(".gitkeep");
                if !gitkeep.exists() {
                    std::fs::write(&gitkeep, "")
                        .with_context(|| format!("Failed to write {}", gitkeep.display()))?;
                    created.push(PathBuf::from(format!("{conv}/.gitkeep")));
                }
            }
        }
    }

    // Write .gitignore if it doesn't exist (ignore result/ directory)
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, "# Nix build result symlink\nresult\nresult-*\n")
            .with_context(|| format!("Failed to write {}", gitignore.display()))?;
        created.push(PathBuf::from(".gitignore"));
    }

    Ok(created)
}

// ── Helpers ───────────────────────────────────────────────────────

/// Recursively copy a directory (fallback for non-Unix platforms).
#[cfg(not(unix))]
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_generate_flake_nix_default() {
        let config = FlakeGenConfig {
            nix_workspace_ref: "git+https://tangled.org/@overby.me/overby.me?dir=nix-workspace"
                .into(),
            extra_inputs: vec![],
        };
        let content = generate_flake_nix(Path::new("/tmp/test"), &config);

        assert!(content.contains("nixpkgs.url"));
        assert!(content.contains("nix-workspace.url"));
        assert!(content.contains("tangled.org/@overby.me/overby.me?dir=nix-workspace"));
        assert!(content.contains("inputs.nix-workspace ./."));
        assert!(content.contains("inherit inputs"));
    }

    #[test]
    fn test_generate_flake_nix_custom_ref() {
        let config = FlakeGenConfig {
            nix_workspace_ref: "path:/home/user/nix-workspace".into(),
            extra_inputs: vec![],
        };
        let content = generate_flake_nix(Path::new("/tmp/test"), &config);

        assert!(content.contains("path:/home/user/nix-workspace"));
    }

    #[test]
    fn test_generate_flake_nix_extra_inputs() {
        let config = FlakeGenConfig {
            nix_workspace_ref: DEFAULT_FLAKE_REF.into(),
            extra_inputs: vec![(
                "home-manager".into(),
                "github:nix-community/home-manager".into(),
            )],
        };
        let content = generate_flake_nix(Path::new("/tmp/test"), &config);

        assert!(content.contains("home-manager.url"));
        assert!(content.contains("github:nix-community/home-manager"));
    }

    #[test]
    fn test_needs_flake_generation() {
        let tmp = TempDir::new().unwrap();

        // No workspace.ncl → doesn't need generation
        assert!(!needs_flake_generation(tmp.path()));

        // workspace.ncl but no flake.nix → needs generation
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        assert!(needs_flake_generation(tmp.path()));

        // Both exist → doesn't need generation
        fs::write(tmp.path().join("flake.nix"), "{}").unwrap();
        assert!(!needs_flake_generation(tmp.path()));
    }

    #[test]
    fn test_generate_workspace_ncl() {
        let opts = InitOptions {
            name: "my-project".into(),
            description: Some("A cool project".into()),
            systems: vec!["x86_64-linux".into()],
            plugins: vec!["nix-workspace-rust".into()],
            ..Default::default()
        };

        let content = generate_workspace_ncl(&opts);

        assert!(content.contains("name = \"my-project\""));
        assert!(content.contains("description = \"A cool project\""));
        assert!(content.contains("\"x86_64-linux\""));
        assert!(content.contains("\"nix-workspace-rust\""));
    }

    #[test]
    fn test_generate_workspace_ncl_minimal() {
        let opts = InitOptions {
            name: "minimal".into(),
            description: None,
            systems: vec!["x86_64-linux".into(), "aarch64-linux".into()],
            plugins: vec![],
            ..Default::default()
        };

        let content = generate_workspace_ncl(&opts);

        assert!(content.contains("name = \"minimal\""));
        assert!(!content.contains("description"));
        assert!(!content.contains("plugins"));
    }

    #[test]
    fn test_generate_default_shell_ncl() {
        let content = generate_default_shell_ncl();
        assert!(content.contains("packages"));
        assert!(content.contains("shell-hook"));
    }

    #[test]
    fn test_generate_sample_package_ncl() {
        let content = generate_sample_package_ncl("hello");
        assert!(content.contains("hello"));
        assert!(content.contains("build-system"));
        assert!(content.contains("generic"));
    }

    #[test]
    fn test_init_workspace_creates_structure() {
        let tmp = TempDir::new().unwrap();

        let opts = InitOptions {
            name: "test-init".into(),
            description: Some("Test workspace".into()),
            systems: vec!["x86_64-linux".into()],
            with_flake: true,
            conventions: vec!["packages".into(), "shells".into()],
            plugins: vec![],
        };

        let created = init_workspace(tmp.path(), &opts).unwrap();

        // workspace.ncl should exist
        assert!(tmp.path().join("workspace.ncl").is_file());
        assert!(created.contains(&PathBuf::from("workspace.ncl")));

        // flake.nix should exist
        assert!(tmp.path().join("flake.nix").is_file());
        assert!(created.contains(&PathBuf::from("flake.nix")));

        // packages/ and shells/ directories should exist
        assert!(tmp.path().join("packages").is_dir());
        assert!(tmp.path().join("shells").is_dir());

        // Sample files should exist
        assert!(tmp.path().join("packages/test-init.ncl").is_file());
        assert!(tmp.path().join("shells/default.ncl").is_file());

        // .gitignore should exist
        assert!(tmp.path().join(".gitignore").is_file());
        let gitignore = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(gitignore.contains("result"));
    }

    #[test]
    fn test_init_workspace_refuses_overwrite() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();

        let opts = InitOptions {
            name: "test".into(),
            ..Default::default()
        };

        let result = init_workspace(tmp.path(), &opts);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_init_workspace_without_flake() {
        let tmp = TempDir::new().unwrap();

        let opts = InitOptions {
            name: "no-flake".into(),
            with_flake: false,
            conventions: vec![],
            ..Default::default()
        };

        let created = init_workspace(tmp.path(), &opts).unwrap();

        assert!(tmp.path().join("workspace.ncl").is_file());
        assert!(!tmp.path().join("flake.nix").exists());
        assert!(!created.contains(&PathBuf::from("flake.nix")));
    }

    #[test]
    fn test_generate_flake_in_place_refuses_existing() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("flake.nix"), "existing").unwrap();

        let config = FlakeGenConfig::default();
        let result = generate_flake_in_place(tmp.path(), &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_flake_in_place_creates_file() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();

        let config = FlakeGenConfig::default();
        let path = generate_flake_in_place(tmp.path(), &config).unwrap();

        assert!(path.is_file());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("nix-workspace"));
    }

    #[test]
    fn test_create_temp_flake() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{ name = \"test\" }").unwrap();
        fs::create_dir(tmp.path().join("packages")).unwrap();
        fs::write(tmp.path().join("packages/hello.ncl"), "{}").unwrap();

        let config = FlakeGenConfig::default();
        let temp_flake = create_temp_flake(tmp.path(), &config).unwrap();

        // The temp flake should have a generated flake.nix
        assert!(temp_flake.flake_nix_path.is_file());

        // The workspace files should be accessible via symlinks
        assert!(temp_flake.flake_root.join("workspace.ncl").exists());
        assert!(temp_flake.flake_root.join("packages").exists());
        assert!(temp_flake.flake_root.join("packages/hello.ncl").exists());

        // Flake ref should be a path reference
        let flake_ref = temp_flake.flake_ref();
        assert!(flake_ref.starts_with("path:"));

        // Output-specific ref
        let output_ref = temp_flake.flake_ref_with_output("hello");
        assert!(output_ref.ends_with("#hello"));
    }

    #[test]
    fn test_create_temp_flake_skips_existing_flake() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("workspace.ncl"), "{}").unwrap();
        fs::write(tmp.path().join("flake.nix"), "old flake").unwrap();

        let config = FlakeGenConfig::default();
        let temp_flake = create_temp_flake(tmp.path(), &config).unwrap();

        // The generated flake should NOT be the old one
        let content = fs::read_to_string(&temp_flake.flake_nix_path).unwrap();
        assert!(!content.contains("old flake"));
        assert!(content.contains("nix-workspace"));
    }

    #[test]
    fn test_init_with_extra_conventions() {
        let tmp = TempDir::new().unwrap();

        let opts = InitOptions {
            name: "extra".into(),
            conventions: vec!["packages".into(), "modules".into(), "machines".into()],
            ..Default::default()
        };

        let created = init_workspace(tmp.path(), &opts).unwrap();

        assert!(tmp.path().join("packages").is_dir());
        assert!(tmp.path().join("modules").is_dir());
        assert!(tmp.path().join("machines").is_dir());

        // modules and machines get .gitkeep files
        assert!(tmp.path().join("modules/.gitkeep").is_file());
        assert!(tmp.path().join("machines/.gitkeep").is_file());

        assert!(created.contains(&PathBuf::from("modules/.gitkeep")));
    }
}
