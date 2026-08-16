//! `spindle-run` — Run Tangled CI workflows locally.
//!
//! Reads `.tangled/workflows/*.yml` files and executes them in an isolated
//! environment using `systemd-run --user`, skipping the clone step
//! (you're already in the repo).

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use spindle_engine::nix_deps::{
    NixDeps, build_nix_env, parse_dependencies_from_yaml, parse_env_from_yaml,
    parse_steps_from_yaml,
};
use spindle_models::WorkflowLogger;

/// Run Tangled CI workflows locally.
#[derive(Parser, Debug)]
#[command(name = "spindle-run", version, about)]
struct Cli {
    /// Workflow file(s) to run. If omitted, runs all workflows in
    /// `.tangled/workflows/`.
    workflows: Vec<PathBuf>,

    /// Working directory (defaults to current directory).
    #[arg(short = 'C', long)]
    workdir: Option<PathBuf>,

    /// Timeout per step in seconds.
    #[arg(long, default_value = "3600")]
    timeout: u64,

    /// Extra flags to pass to `nix build` when building dependencies.
    #[arg(long = "nix-flag")]
    nix_flags: Vec<String>,

    /// Only list the steps without executing them.
    #[arg(long)]
    dry_run: bool,

    /// Disable systemd-run isolation (run steps directly).
    #[arg(long)]
    no_isolate: bool,
}

/// A workflow logger that writes step output directly to the terminal.
struct TerminalLogger;

impl WorkflowLogger for TerminalLogger {
    fn close(&self) -> io::Result<()> {
        Ok(())
    }

    fn data_writer(&self, _step_id: usize, stream: String) -> Box<dyn Write + Send> {
        match stream.as_str() {
            "stderr" => Box::new(StderrWriter),
            _ => Box::new(StdoutWriter),
        }
    }

    fn control_writer(
        &self,
        _step_id: usize,
        _step: &dyn spindle_models::step::Step,
        _step_status: spindle_models::log_line::StepStatus,
    ) -> Box<dyn Write + Send> {
        Box::new(io::sink())
    }
}

struct StdoutWriter;
impl Write for StdoutWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let trimmed = text.trim_end_matches(['\r', '\n']);
        if !trimmed.is_empty() {
            println!("{trimmed}");
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

struct StderrWriter;
impl Write for StderrWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let trimmed = text.trim_end_matches(['\r', '\n']);
        if !trimmed.is_empty() {
            eprintln!("{trimmed}");
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        io::stderr().flush()
    }
}

fn discover_workflows(base: &Path) -> io::Result<Vec<PathBuf>> {
    let workflow_dir = base.join(".tangled").join("workflows");
    if !workflow_dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no workflow directory found at {}", workflow_dir.display()),
        ));
    }

    let mut files: Vec<PathBuf> = std::fs::read_dir(&workflow_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("yml" | "yaml") => Some(path),
                _ => None,
            }
        })
        .collect();

    files.sort();
    Ok(files)
}

/// Build a systemd-run command for isolated step execution.
///
/// Uses `systemd-run --user --pipe --wait` with:
/// - `PrivateTmp=yes` — isolated /tmp
/// - `WorkingDirectory` — run in the repo directory
/// - Ephemeral home in `~/.cache/spindle-run/home` for $HOME writes
/// - Clean environment with only specified vars
fn build_systemd_run_cmd(
    workdir: &Path,
    home_dir: &Path,
    command: &str,
    env_vars: &[(String, String)],
    timeout_secs: u64,
    bash_path: &Path,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("systemd-run");
    cmd.args([
        "--user",
        "--pipe",
        "--wait",
        "--collect",
        "--quiet",
        // Timeout
        "-p",
        &format!("RuntimeMaxSec={timeout_secs}"),
        // Working directory
        &format!("--working-directory={}", workdir.display()),
    ]);

    // PrivateTmp gives the process its own /tmp, but conflicts with
    // workdirs that live under /tmp (systemd can't CHDIR into the
    // private mount). Only enable when the workdir is outside /tmp.
    if !workdir.starts_with("/tmp") {
        cmd.args(["-p", "PrivateTmp=yes"]);
    }

    // Set environment variables.
    for (k, v) in env_vars {
        cmd.arg("--setenv");
        cmd.arg(format!("{k}={v}"));
    }

    // Set HOME and XDG directories to the ephemeral home dir.
    // Tools like cachix and nix resolve config paths via XDG dirs
    // or the passwd entry rather than $HOME, so we must set these
    // explicitly to keep writes inside the ephemeral home.
    cmd.arg("--setenv");
    cmd.arg(format!("HOME={}", home_dir.display()));
    cmd.arg("--setenv");
    cmd.arg(format!(
        "XDG_CONFIG_HOME={}",
        home_dir.join(".config").display()
    ));
    cmd.arg("--setenv");
    cmd.arg(format!(
        "XDG_CACHE_HOME={}",
        home_dir.join(".cache").display()
    ));
    cmd.arg("--setenv");
    cmd.arg(format!(
        "XDG_DATA_HOME={}",
        home_dir.join(".local/share").display()
    ));
    cmd.arg("--setenv");
    cmd.arg(format!(
        "XDG_STATE_HOME={}",
        home_dir.join(".local/state").display()
    ));

    // The actual command: bash -euo pipefail -c '...'
    cmd.arg("--");
    cmd.arg(bash_path);
    cmd.args(["--norc", "--noprofile", "-euo", "pipefail", "-c", command]);

    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    cmd
}

/// Build a direct (non-isolated) command for step execution.
fn build_direct_cmd(
    workdir: &Path,
    home_dir: &Path,
    command: &str,
    env_vars: &[(String, String)],
    bash_path: &Path,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(bash_path);
    cmd.args(["--norc", "--noprofile", "-euo", "pipefail", "-c", command]);
    cmd.current_dir(workdir);
    cmd.env_clear();
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    cmd.env("HOME", home_dir);
    cmd.env("XDG_CONFIG_HOME", home_dir.join(".config"));
    cmd.env("XDG_CACHE_HOME", home_dir.join(".cache"));
    cmd.env("XDG_DATA_HOME", home_dir.join(".local/share"));
    cmd.env("XDG_STATE_HOME", home_dir.join(".local/state"));
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    cmd
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "spindle_run=info,spindle_engine=info,warn".into()),
        )
        .with_target(false)
        .without_time()
        .init();

    // Resolve working directory.
    let workdir = match &cli.workdir {
        Some(dir) => std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    };

    if !workdir.is_dir() {
        eprintln!("error: {} is not a directory", workdir.display());
        return ExitCode::FAILURE;
    }

    // Discover or validate workflow files.
    let workflow_files = if cli.workflows.is_empty() {
        match discover_workflows(&workdir) {
            Ok(files) if files.is_empty() => {
                eprintln!("error: no workflow files found in .tangled/workflows/");
                return ExitCode::FAILURE;
            }
            Ok(files) => files,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        cli.workflows
            .iter()
            .map(|p| {
                if p.is_absolute() {
                    p.clone()
                } else {
                    workdir.join(p)
                }
            })
            .collect()
    };

    let logger = TerminalLogger;

    // Set up cache and ephemeral home directories.
    let cache_dir = workdir.join(".tangled").join(".cache");
    let user_cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".cache")
        });
    let run_home = user_cache.join("spindle-run").join("home");
    for dir in [&cache_dir, &run_home] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("error: failed to create {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    let bash_path = resolve_bash();
    let isolate = !cli.no_isolate;

    if isolate {
        eprintln!("\x1b[1;90m(using systemd-run for isolation, --no-isolate to disable)\x1b[0m");
    }

    let mut any_failed = false;

    for wf_path in &workflow_files {
        let wf_name = wf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        eprintln!("\n\x1b[1;36m=== Workflow: {wf_name} ===\x1b[0m\n");

        let raw_yaml = match std::fs::read_to_string(wf_path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("error: failed to read {}: {e}", wf_path.display());
                any_failed = true;
                continue;
            }
        };

        // Parse the workflow.
        let steps = match parse_steps_from_yaml(&raw_yaml) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to parse steps from {wf_name}: {e}");
                any_failed = true;
                continue;
            }
        };

        let deps = parse_dependencies_from_yaml(&raw_yaml).unwrap_or_default();
        let env_vars = parse_env_from_yaml(&raw_yaml).unwrap_or_default();

        if steps.is_empty() {
            eprintln!("warning: {wf_name} has no steps, skipping");
            continue;
        }

        // Dry-run: just list steps.
        if cli.dry_run {
            for (i, (name, command)) in steps.iter().enumerate() {
                eprintln!("  Step {}: {name}", i + 1);
                for line in command.lines() {
                    eprintln!("    {line}");
                }
            }
            if !deps.is_empty() {
                eprintln!("\n  Dependencies:");
                for (source, pkgs) in &deps {
                    eprintln!("    {source}: {}", pkgs.join(", "));
                }
            }
            continue;
        }

        // Build Nix environment if dependencies are specified.
        let nix_env_path = if let Some(nix_deps) = NixDeps::parse(&deps) {
            eprintln!("\x1b[1;33m> Building Nix dependencies...\x1b[0m");
            match build_nix_env(&nix_deps, &cache_dir, &cli.nix_flags, &logger).await {
                Ok(path) => {
                    eprintln!(
                        "\x1b[1;32m> Nix environment ready: {}\x1b[0m",
                        path.display()
                    );
                    Some(path)
                }
                Err(e) => {
                    eprintln!("\x1b[1;31m> Failed to build Nix dependencies: {e}\x1b[0m");
                    any_failed = true;
                    continue;
                }
            }
        } else {
            None
        };

        // Build PATH.
        let path = build_local_path(nix_env_path.as_deref());

        // Execute each step.
        for (i, (name, command)) in steps.iter().enumerate() {
            eprintln!("\x1b[1;34m> Step {}: {name}\x1b[0m", i + 1);

            let mut step_env: Vec<(String, String)> =
                vec![("PATH".into(), path.clone()), ("CI".into(), "true".into())];

            for (k, v) in &env_vars {
                step_env.push((k.clone(), v.clone()));
            }

            // Inherit TERM for colored output.
            if let Ok(term) = std::env::var("TERM") {
                step_env.push(("TERM".into(), term));
            }

            // Override USER with the real user running spindle-run.
            // Workflows may set USER for CI environments (e.g. "nobody"
            // for DynamicUser), but locally the real user is needed for
            // Nix daemon trust and other user-dependent tooling.
            if let Ok(real_user) = std::env::var("USER") {
                step_env.retain(|(k, _)| k != "USER");
                step_env.push(("USER".into(), real_user));
            }

            let mut child = if isolate {
                build_systemd_run_cmd(
                    &workdir,
                    &run_home,
                    command,
                    &step_env,
                    cli.timeout,
                    &bash_path,
                )
            } else {
                build_direct_cmd(&workdir, &run_home, command, &step_env, &bash_path)
            };

            let mut spawned = match child.spawn() {
                Ok(child) => child,
                Err(e) => {
                    eprintln!("\x1b[1;31m> Failed to spawn step: {e}\x1b[0m");
                    any_failed = true;
                    break;
                }
            };

            let status = match spawned.wait().await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("\x1b[1;31m> Failed to wait for step: {e}\x1b[0m");
                    any_failed = true;
                    break;
                }
            };

            if !status.success() {
                let code = status.code().unwrap_or(-1);
                eprintln!(
                    "\x1b[1;31m> Step {}: {name} failed (exit code {code})\x1b[0m",
                    i + 1
                );
                any_failed = true;
                break;
            }

            eprintln!("\x1b[1;32m> Step {}: {name} passed\x1b[0m", i + 1);
        }
    }

    // Clean up ephemeral home.
    let _ = std::fs::remove_dir_all(&run_home);

    if any_failed {
        eprintln!("\n\x1b[1;31mSome workflows failed.\x1b[0m");
        ExitCode::FAILURE
    } else {
        eprintln!("\n\x1b[1;32mAll workflows passed.\x1b[0m");
        ExitCode::SUCCESS
    }
}

/// Build PATH for local execution.
fn build_local_path(nix_env: Option<&Path>) -> String {
    let mut parts = Vec::new();

    if let Some(env) = nix_env {
        parts.push(format!("{}/bin", env.display()));
        parts.push(format!("{}/sbin", env.display()));
    }

    if let Ok(parent_path) = std::env::var("PATH") {
        parts.push(parent_path);
    }

    parts.extend(["/usr/local/bin".into(), "/usr/bin".into(), "/bin".into()]);

    parts.join(":")
}

/// Resolve bash from PATH (same logic as the engine).
fn resolve_bash() -> PathBuf {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = PathBuf::from(dir).join("bash");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from("/bin/bash")
}
