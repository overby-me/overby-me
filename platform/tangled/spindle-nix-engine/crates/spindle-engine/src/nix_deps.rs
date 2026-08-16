//! Nix dependency resolution for workflow execution.
//!
//! Transforms workflow `dependencies` maps into Nix build environments.
//! The dependency format matches the upstream Go spindle / Nixery convention:
//!
//! ```yaml
//! dependencies:
//!   nixpkgs:
//!     - nodejs
//!     - go
//!   nixpkgs/nixpkgs-unstable:
//!     - bun
//!   git+https://tangled.org/@example.com/my_pkg:
//!     - my_pkg
//! ```
//!
//! Each source is resolved to a Nix expression:
//! - `nixpkgs` → `import <nixpkgs> {}`
//! - `nixpkgs/{channel}` → `import (builtins.fetchTarball { url = "…/{channel}.tar.gz"; }) {}`
//! - `git+{url}` → `import (builtins.fetchGit { url = "{url}"; }) {}`
//!
//! The generated expression uses `buildEnv` to produce a combined environment
//! with all requested packages on `PATH`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::traits::{EngineError, EngineResult};
use spindle_models::WorkflowLogger;

/// A parsed dependency source with its requested packages.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DepSource {
    /// Nix variable name (e.g. `nixpkgs`, `nixpkgs-unstable`, `custom_0`).
    var_name: String,
    /// Nix expression to import this source.
    import_expr: String,
    /// Package attribute paths to extract from this source.
    packages: Vec<String>,
}

/// Parsed and validated dependency specification for a workflow.
#[derive(Debug, Clone)]
pub struct NixDeps {
    /// Ordered list of dependency sources.
    sources: Vec<DepSource>,
    /// Content hash of the dependency specification (for caching).
    content_hash: String,
}

impl NixDeps {
    /// Parse a dependency map from the workflow YAML.
    ///
    /// The map keys are source identifiers, values are lists of package names.
    /// Returns `None` if the map is empty (no dependencies requested).
    pub fn parse(deps: &HashMap<String, Vec<String>>) -> Option<Self> {
        if deps.is_empty() {
            return None;
        }

        // Use BTreeMap for deterministic ordering (important for content hashing).
        let sorted: BTreeMap<&String, &Vec<String>> = deps.iter().collect();

        let mut sources = Vec::new();
        let mut custom_idx = 0u32;

        for (source_key, packages) in &sorted {
            if packages.is_empty() {
                continue;
            }

            let (var_name, import_expr) = resolve_source(source_key, &mut custom_idx);

            sources.push(DepSource {
                var_name,
                import_expr,
                packages: packages.to_vec(),
            });
        }

        if sources.is_empty() {
            return None;
        }

        // Compute content hash for caching.
        let content_hash = compute_content_hash(&sorted);

        Some(Self {
            sources,
            content_hash,
        })
    }

    /// Generate a Nix expression that produces a combined environment.
    ///
    /// The expression uses `pkgs.buildEnv` to merge all requested packages
    /// into a single output with `bin/` and `sbin/` directories.
    pub fn to_nix_expr(&self) -> String {
        let mut expr = String::from("let\n");

        for src in &self.sources {
            writeln!(expr, "  {} = {};", src.var_name, src.import_expr).unwrap();
        }

        expr.push_str("in\n");
        // Use the first nixpkgs source for buildEnv, or fall back to <nixpkgs>.
        let build_env_source = self
            .sources
            .iter()
            .find(|s| s.var_name == "nixpkgs")
            .map(|s| s.var_name.as_str())
            .unwrap_or("(import (builtins.fetchTarball { url = \"https://github.com/NixOS/nixpkgs/archive/nixpkgs-unstable.tar.gz\"; }) {})");

        writeln!(expr, "{build_env_source}.buildEnv {{").unwrap();
        expr.push_str("  name = \"spindle-workflow-env\";\n");
        expr.push_str("  paths = [\n");

        for src in &self.sources {
            for pkg in &src.packages {
                writeln!(expr, "    {}.{}", src.var_name, pkg).unwrap();
            }
        }

        expr.push_str("  ];\n");
        expr.push_str("}\n");

        expr
    }

    /// Return the content hash of this dependency specification.
    ///
    /// Two workflows with identical dependency maps will produce the same hash,
    /// allowing the built Nix closure to be reused without rebuilding.
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

/// Resolve a source key into a Nix variable name and import expression.
///
/// Uses `builtins.fetchTarball` instead of `<nixpkgs>` to avoid depending
/// on `NIX_PATH`, which may not be set in sandboxed environments
/// (e.g. systemd DynamicUser).
fn resolve_source(key: &str, custom_idx: &mut u32) -> (String, String) {
    if key == "nixpkgs" {
        (
            "nixpkgs".into(),
            "import (builtins.fetchTarball { url = \"https://github.com/NixOS/nixpkgs/archive/nixpkgs-unstable.tar.gz\"; }) {}".into(),
        )
    } else if let Some(channel) = key.strip_prefix("nixpkgs/") {
        let var_name = channel.replace(['-', '.'], "_");
        let import = format!(
            "import (builtins.fetchTarball {{ url = \"https://github.com/NixOS/nixpkgs/archive/{channel}.tar.gz\"; }}) {{}}"
        );
        (var_name, import)
    } else if let Some(url) = key.strip_prefix("git+") {
        let idx = *custom_idx;
        *custom_idx += 1;
        let var_name = format!("custom_{idx}");
        let import = format!("import (builtins.fetchGit {{ url = \"{url}\"; }}) {{}}");
        (var_name, import)
    } else {
        // Unknown source format — treat as a git URL.
        let idx = *custom_idx;
        *custom_idx += 1;
        let var_name = format!("custom_{idx}");
        let import = format!("import (builtins.fetchGit {{ url = \"{key}\"; }}) {{}}");
        (var_name, import)
    }
}

/// Compute a deterministic content hash of the dependency specification.
fn compute_content_hash(deps: &BTreeMap<&String, &Vec<String>>) -> String {
    let mut hasher = Sha256::new();
    for (source, packages) in deps {
        hasher.update(source.as_bytes());
        hasher.update(b"\0");
        for pkg in *packages {
            hasher.update(pkg.as_bytes());
            hasher.update(b"\0");
        }
        hasher.update(b"\n");
    }
    let result = hasher.finalize();
    hex::encode(result)
}

/// Build a Nix closure from a dependency specification.
///
/// Writes a temporary `.nix` file and invokes `nix build --no-link --print-out-paths`.
/// Returns the store path of the built environment.
///
/// If a cached closure exists for this content hash, returns it immediately.
pub async fn build_nix_env(
    deps: &NixDeps,
    cache_dir: &Path,
    extra_nix_flags: &[String],
    logger: &dyn WorkflowLogger,
) -> EngineResult<PathBuf> {
    let hash = deps.content_hash();

    // Check cache.
    let cache_file = cache_dir.join(format!("env-{hash}.path"));
    if let Ok(cached_path) = tokio::fs::read_to_string(&cache_file).await {
        let path = PathBuf::from(cached_path.trim());
        if path.exists() {
            info!(%hash, ?path, "using cached Nix environment");
            return Ok(path);
        }
        // Cache entry points to a GC'd path — rebuild.
        debug!(%hash, "cached Nix environment was garbage-collected, rebuilding");
    }

    // Write the Nix expression to a temp file.
    let nix_file = cache_dir.join(format!("env-{hash}.nix"));
    let nix_expr = deps.to_nix_expr();
    tokio::fs::create_dir_all(cache_dir)
        .await
        .map_err(|e| EngineError::SetupFailed(format!("failed to create cache dir: {e}")))?;
    tokio::fs::write(&nix_file, &nix_expr)
        .await
        .map_err(|e| EngineError::SetupFailed(format!("failed to write nix expression: {e}")))?;

    debug!(?nix_file, "building Nix environment");

    // Log the build start.
    let mut setup_writer = logger.data_writer(0, "stdout".into());
    use std::io::Write;
    let _ = write!(setup_writer, "Building Nix environment (hash: {hash})...");

    // Build the Nix closure.
    // Set HOME to the cache directory so nix can write to ~/.cache/nix
    // (under DynamicUser, HOME may be unset or point to a read-only path).
    let mut cmd = Command::new("nix");
    cmd.env("HOME", cache_dir)
        .arg("build")
        .arg("--no-link")
        .arg("--print-out-paths")
        .arg("-f")
        .arg(&nix_file);

    for flag in extra_nix_flags {
        cmd.arg(flag);
    }

    // Capture both stdout (store path) and stderr (build log).
    let output = cmd
        .output()
        .await
        .map_err(|e| EngineError::SetupFailed(format!("failed to spawn nix build: {e}")))?;

    // Stream stderr to the logger.
    if !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        for line in stderr.lines() {
            let _ = write!(setup_writer, "{line}");
        }
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(EngineError::SetupFailed(format!(
            "nix build failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let store_path = stdout
        .lines()
        .next()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .ok_or_else(|| EngineError::SetupFailed("nix build produced no output paths".into()))?;

    let path = PathBuf::from(store_path);
    if !path.exists() {
        return Err(EngineError::SetupFailed(format!(
            "nix build output path does not exist: {store_path}"
        )));
    }

    // Cache the result.
    if let Err(e) = tokio::fs::write(&cache_file, store_path).await {
        warn!(%hash, "failed to write cache file: {e}");
    }

    info!(%hash, %store_path, "built Nix environment");
    let _ = write!(setup_writer, "Nix environment ready: {store_path}");

    Ok(path)
}

/// Parse the `dependencies` field from a workflow YAML's `data` value.
///
/// The YAML format is:
/// ```yaml
/// dependencies:
///   nixpkgs:
///     - nodejs
///     - go
/// ```
///
/// Returns an empty map if no dependencies are specified.
pub fn parse_dependencies_from_yaml(raw_yaml: &str) -> EngineResult<HashMap<String, Vec<String>>> {
    #[derive(serde::Deserialize)]
    struct WorkflowYaml {
        #[serde(default)]
        dependencies: HashMap<String, Vec<String>>,
    }

    let parsed: WorkflowYaml = serde_yaml::from_str(raw_yaml)
        .map_err(|e| EngineError::InvalidWorkflow(format!("failed to parse workflow YAML: {e}")))?;

    Ok(parsed.dependencies)
}

/// Parse user-defined steps from a workflow YAML.
///
/// Returns a list of (name, command) pairs.
pub fn parse_steps_from_yaml(raw_yaml: &str) -> EngineResult<Vec<(String, String)>> {
    #[derive(serde::Deserialize)]
    struct WorkflowYaml {
        #[serde(default)]
        steps: Vec<StepYaml>,
    }

    #[derive(serde::Deserialize)]
    struct StepYaml {
        name: String,
        #[serde(alias = "command", alias = "run")]
        run: String,
    }

    let parsed: WorkflowYaml = serde_yaml::from_str(raw_yaml)
        .map_err(|e| EngineError::InvalidWorkflow(format!("failed to parse workflow YAML: {e}")))?;

    Ok(parsed.steps.into_iter().map(|s| (s.name, s.run)).collect())
}

/// Parse workflow-level environment variables from YAML.
///
/// Supports both `env:` and `environment:` keys for compatibility with
/// the upstream Go spindle workflow format.
pub fn parse_env_from_yaml(raw_yaml: &str) -> EngineResult<HashMap<String, String>> {
    #[derive(serde::Deserialize)]
    struct WorkflowYaml {
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        environment: HashMap<String, String>,
    }

    let parsed: WorkflowYaml = serde_yaml::from_str(raw_yaml)
        .map_err(|e| EngineError::InvalidWorkflow(format!("failed to parse workflow YAML: {e}")))?;

    // Merge both, with `env` taking precedence over `environment`.
    let mut result = parsed.environment;
    result.extend(parsed.env);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nixpkgs_only() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs".into(), vec!["nodejs".into(), "go".into()]);

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        assert!(expr.contains("nixpkgs = import (builtins.fetchTarball"));
        assert!(expr.contains("nixpkgs.nodejs"));
        assert!(expr.contains("nixpkgs.go"));
        assert!(expr.contains("buildEnv"));
    }

    #[test]
    fn parse_nixpkgs_unstable() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs/nixpkgs-unstable".into(), vec!["bun".into()]);

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        assert!(expr.contains("nixpkgs_unstable"));
        assert!(expr.contains("nixpkgs-unstable.tar.gz"));
        assert!(expr.contains("nixpkgs_unstable.bun"));
    }

    #[test]
    fn parse_custom_git_source() {
        let mut deps = HashMap::new();
        deps.insert(
            "git+https://tangled.org/@example.com/my_pkg".into(),
            vec!["my_pkg".into()],
        );

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        assert!(expr.contains("custom_0"));
        assert!(expr.contains("https://tangled.org/@example.com/my_pkg"));
        assert!(expr.contains("custom_0.my_pkg"));
    }

    #[test]
    fn parse_mixed_sources() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs".into(), vec!["nodejs".into(), "go".into()]);
        deps.insert("nixpkgs/nixpkgs-unstable".into(), vec!["bun".into()]);
        deps.insert(
            "git+https://tangled.org/@example.com/my_pkg".into(),
            vec!["my_pkg".into()],
        );

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        assert!(expr.contains("nixpkgs = import (builtins.fetchTarball"));
        assert!(expr.contains("nixpkgs.nodejs"));
        assert!(expr.contains("nixpkgs.go"));
        assert!(expr.contains("nixpkgs_unstable.bun"));
        assert!(expr.contains("custom_0.my_pkg"));
    }

    #[test]
    fn empty_deps_returns_none() {
        let deps = HashMap::new();
        assert!(NixDeps::parse(&deps).is_none());
    }

    #[test]
    fn empty_package_list_skipped() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs".into(), vec![]);
        assert!(NixDeps::parse(&deps).is_none());
    }

    #[test]
    fn content_hash_deterministic() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs".into(), vec!["nodejs".into(), "go".into()]);

        let a = NixDeps::parse(&deps).unwrap();
        let b = NixDeps::parse(&deps).unwrap();

        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_differs_for_different_deps() {
        let mut deps1 = HashMap::new();
        deps1.insert("nixpkgs".into(), vec!["nodejs".into()]);

        let mut deps2 = HashMap::new();
        deps2.insert("nixpkgs".into(), vec!["go".into()]);

        let a = NixDeps::parse(&deps1).unwrap();
        let b = NixDeps::parse(&deps2).unwrap();

        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn parse_dependencies_from_yaml_basic() {
        let yaml = r#"
dependencies:
  nixpkgs:
    - nodejs
    - go
  nixpkgs/nixpkgs-unstable:
    - bun
steps:
  - name: test
    run: echo hello
"#;
        let deps = parse_dependencies_from_yaml(yaml).unwrap();
        assert_eq!(deps["nixpkgs"], vec!["nodejs", "go"]);
        assert_eq!(deps["nixpkgs/nixpkgs-unstable"], vec!["bun"]);
    }

    #[test]
    fn parse_dependencies_from_yaml_empty() {
        let yaml = "steps:\n  - name: test\n    run: echo hello\n";
        let deps = parse_dependencies_from_yaml(yaml).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_steps_from_yaml_basic() {
        let yaml = r#"
steps:
  - name: Build
    run: cargo build
  - name: Test
    run: cargo test
"#;
        let steps = parse_steps_from_yaml(yaml).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0], ("Build".into(), "cargo build".into()));
        assert_eq!(steps[1], ("Test".into(), "cargo test".into()));
    }

    #[test]
    fn parse_steps_command_alias() {
        let yaml = r#"
steps:
  - name: Build
    command: cargo build
"#;
        let steps = parse_steps_from_yaml(yaml).unwrap();
        assert_eq!(steps[0], ("Build".into(), "cargo build".into()));
    }

    #[test]
    fn parse_env_from_yaml_basic() {
        let yaml = r#"
env:
  FOO: bar
  BAZ: "42"
steps:
  - name: test
    run: echo $FOO
"#;
        let env = parse_env_from_yaml(yaml).unwrap();
        assert_eq!(env["FOO"], "bar");
        assert_eq!(env["BAZ"], "42");
    }

    #[test]
    fn parse_env_from_yaml_empty() {
        let yaml = "steps:\n  - name: test\n    run: echo hello\n";
        let env = parse_env_from_yaml(yaml).unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn nix_expr_uses_nixpkgs_for_build_env() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs".into(), vec!["bash".into()]);

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        // buildEnv should come from the nixpkgs source
        assert!(expr.contains("nixpkgs.buildEnv {"));
    }

    #[test]
    fn nix_expr_without_nixpkgs_uses_fallback() {
        let mut deps = HashMap::new();
        deps.insert("nixpkgs/nixpkgs-unstable".into(), vec!["bun".into()]);

        let nix_deps = NixDeps::parse(&deps).unwrap();
        let expr = nix_deps.to_nix_expr();

        // Should fall back to fetching nixpkgs for buildEnv
        assert!(expr.contains("(import (builtins.fetchTarball"));
    }

    // -----------------------------------------------------------------------
    // Fuzz-style robustness tests
    // -----------------------------------------------------------------------

    /// Ensure the YAML dependency parser never panics on malformed input.
    #[test]
    fn fuzz_parse_dependencies_malformed_yaml() {
        let inputs = [
            "",
            "   ",
            "\n\n\n",
            "null",
            "42",
            "true",
            "[]",
            "\"just a string\"",
            "{",
            "}",
            "{{{{",
            "dependencies:",
            "dependencies: null",
            "dependencies: 42",
            "dependencies: true",
            "dependencies: []",
            "dependencies: \"string\"",
            "dependencies:\n  nixpkgs: null",
            "dependencies:\n  nixpkgs: 42",
            "dependencies:\n  nixpkgs: \"not a list\"",
            "dependencies:\n  nixpkgs:\n    - 42",
            "dependencies:\n  : \n    - pkg",
            "dependencies:\n  nixpkgs:\n    - ",
            // Deeply nested
            "a:\n  b:\n    c:\n      d:\n        e: f",
            // Very long key
            &format!("dependencies:\n  {}: [pkg]", "a".repeat(10000)),
            // Unicode
            "dependencies:\n  nixpkgs:\n    - 日本語パッケージ",
            // Control characters
            "dependencies:\n  nixpkgs:\n    - \x00\x01\x02",
            // YAML bomb (alias expansion)
            "a: &a [*a]",
            // Tab vs spaces
            "dependencies:\n\tnixpkgs:\n\t\t- nodejs",
        ];

        for input in &inputs {
            // Should never panic — either Ok or Err is fine
            let _ = parse_dependencies_from_yaml(input);
        }
    }

    /// Ensure the YAML steps parser never panics on malformed input.
    #[test]
    fn fuzz_parse_steps_malformed_yaml() {
        let inputs = [
            "",
            "null",
            "steps: null",
            "steps: 42",
            "steps: \"string\"",
            "steps: []",
            "steps:\n  - name: test",
            "steps:\n  - run: echo hello",
            "steps:\n  - {}",
            "steps:\n  - null",
            "steps:\n  - 42",
            "steps:\n  - name: test\n    run: null",
            "steps:\n  - name: null\n    run: echo hello",
            // Missing required fields
            "steps:\n  - other: value",
            // Extra fields (should be ignored)
            "steps:\n  - name: test\n    run: echo\n    extra: ignored",
            // Very long command
            &format!("steps:\n  - name: test\n    run: {}", "x".repeat(100000)),
        ];

        for input in &inputs {
            let _ = parse_steps_from_yaml(input);
        }
    }

    /// Ensure the YAML env parser never panics on malformed input.
    #[test]
    fn fuzz_parse_env_malformed_yaml() {
        let inputs = [
            "",
            "null",
            "env: null",
            "env: 42",
            "env: []",
            "env: \"string\"",
            "env:\n  KEY: null",
            "env:\n  KEY: 42",
            "env:\n  KEY: []",
            "env:\n  KEY: {nested: value}",
            "env:\n  : empty_key",
            &format!("env:\n  {}: value", "K".repeat(10000)),
        ];

        for input in &inputs {
            let _ = parse_env_from_yaml(input);
        }
    }

    /// Benchmark: Nix expression generation from a realistic dependency set.
    ///
    /// Measures the time to parse dependencies and generate the Nix expression,
    /// which is the critical path that replaces Docker/Nixery image pulls.
    #[test]
    fn bench_nix_expr_generation() {
        let mut deps = HashMap::new();
        deps.insert(
            "nixpkgs".into(),
            vec![
                "nodejs".into(),
                "go".into(),
                "python3".into(),
                "rustc".into(),
                "cargo".into(),
                "git".into(),
                "curl".into(),
                "jq".into(),
            ],
        );
        deps.insert("nixpkgs/nixpkgs-unstable".into(), vec!["bun".into()]);
        deps.insert(
            "git+https://tangled.org/@example.com/my_pkg".into(),
            vec!["my_pkg".into()],
        );

        let iterations = 1000;
        let start = std::time::Instant::now();

        for _ in 0..iterations {
            let nix_deps = NixDeps::parse(&deps).unwrap();
            let _expr = nix_deps.to_nix_expr();
            let _hash = nix_deps.content_hash();
        }

        let elapsed = start.elapsed();
        let per_op = elapsed / iterations;

        // Nix expression generation should be sub-millisecond.
        // Docker image pull + Nixery resolution is typically 5-30 seconds.
        assert!(
            per_op.as_millis() < 1,
            "Nix expression generation took {per_op:?} per operation, expected < 1ms"
        );
    }

    /// Benchmark: YAML workflow parsing for a realistic manifest.
    #[test]
    fn bench_yaml_workflow_parsing() {
        let yaml = r#"
dependencies:
  nixpkgs:
    - nodejs
    - go
    - python3
    - rustc
    - cargo
    - git
    - curl
    - jq
  nixpkgs/nixpkgs-unstable:
    - bun
  git+https://tangled.org/@example.com/my_pkg:
    - my_pkg
env:
  CI: "true"
  NODE_ENV: production
  RUST_LOG: info
steps:
  - name: Checkout
    run: git clone --depth=1 https://example.com/repo .
  - name: Install
    run: npm install
  - name: Build
    run: npm run build
  - name: Test
    run: npm test
  - name: Lint
    run: npm run lint
"#;

        let iterations = 1000;
        let start = std::time::Instant::now();

        for _ in 0..iterations {
            let _deps = parse_dependencies_from_yaml(yaml).unwrap();
            let _steps = parse_steps_from_yaml(yaml).unwrap();
            let _env = parse_env_from_yaml(yaml).unwrap();
        }

        let elapsed = start.elapsed();
        let per_op = elapsed / iterations;

        assert!(
            per_op.as_millis() < 1,
            "YAML workflow parsing took {per_op:?} per operation, expected < 1ms"
        );
    }

    /// Ensure NixDeps::parse handles adversarial source keys safely.
    #[test]
    fn fuzz_nix_deps_adversarial_source_keys() {
        let long_key = "a".repeat(10000);
        let adversarial_keys = [
            "",
            " ",
            "nixpkgs",
            "nixpkgs/",
            "nixpkgs//double",
            "git+",
            "git+://invalid",
            "git+https://example.com/$(rm -rf /)",
            "nixpkgs/$(whoami)",
            "../../../etc/passwd",
            "https://evil.com\"; import /etc/shadow; #",
            "\n\t\r",
            long_key.as_str(),
            "nixpkgs/a/b/c",
        ];

        for key in &adversarial_keys {
            let mut deps = HashMap::new();
            deps.insert(key.to_string(), vec!["pkg".into()]);
            // Should never panic
            if let Some(nix_deps) = NixDeps::parse(&deps) {
                // Generated Nix expression should also not panic
                let _expr = nix_deps.to_nix_expr();
                let _hash = nix_deps.content_hash();
            }
        }
    }
}
