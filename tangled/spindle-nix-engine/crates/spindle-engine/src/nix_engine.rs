//! Nix engine implementation.
//!
//! Replaces Docker+Nixery with native Nix builds and child process execution.
//! Each workflow step runs as a child process of the runner daemon, inheriting
//! the systemd service's sandboxing automatically.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::nix_deps::{
    NixDeps, build_nix_env, parse_dependencies_from_yaml, parse_env_from_yaml,
    parse_steps_from_yaml,
};
use crate::traits::{Engine, EngineError, EngineResult, PipelineWorkflow};
use crate::workspace::WorkspaceManager;
use spindle_models::{
    CloneStep, Pipeline, UnlockedSecret, UserStep, Workflow, WorkflowId, WorkflowLogger,
};

/// Per-workflow runtime state, stored while the workflow is active.
#[derive(Debug)]
struct WorkflowState {
    /// Path to the built Nix environment (store path with `bin/`, `sbin/`).
    nix_env_path: Option<PathBuf>,
    /// Path to the workspace directory.
    workspace_dir: PathBuf,
}

/// Per-workflow resource limits applied via systemd scopes.
#[derive(Debug, Clone, Default)]
pub struct WorkflowLimits {
    /// Hard memory limit per workflow (e.g. `"4G"`).
    pub memory_max: Option<String>,
    /// Maximum tasks (processes/threads) per workflow.
    pub tasks_max: Option<u32>,
}

/// The Nix engine: builds Nix closures from dependency specs and executes
/// workflow steps as child processes.
pub struct NixEngine {
    /// Workspace manager for creating/destroying per-workflow directories.
    workspace_mgr: WorkspaceManager,
    /// Directory for caching built Nix environments.
    cache_dir: PathBuf,
    /// Configured workflow timeout.
    timeout: Duration,
    /// Extra flags to pass to `nix build`.
    extra_nix_flags: Vec<String>,
    /// Whether dev mode is enabled (affects repo URL generation).
    dev_mode: bool,
    /// Active workflow states, keyed by workflow ID string.
    states: Arc<Mutex<HashMap<String, WorkflowState>>>,
    /// Resolved path to the bash binary (found from PATH at construction time).
    bash_path: PathBuf,
    /// Resolved path to `systemd-run` (if available).
    systemd_run_path: Option<PathBuf>,
    /// Per-workflow resource limits.
    workflow_limits: WorkflowLimits,
}

impl NixEngine {
    /// Create a new Nix engine.
    ///
    /// # Arguments
    /// * `workspace_root` — Root directory for per-workflow workspace dirs.
    /// * `cache_dir` — Directory for caching built Nix environments.
    /// * `timeout` — Default workflow timeout.
    /// * `extra_nix_flags` — Extra flags passed to `nix build`.
    /// * `dev_mode` — Whether dev mode is enabled.
    /// * `workflow_limits` — Per-workflow resource limits for systemd scopes.
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        timeout: Duration,
        extra_nix_flags: Vec<String>,
        dev_mode: bool,
        workflow_limits: WorkflowLimits,
    ) -> Self {
        // Resolve bash from the current process's PATH. On NixOS, /bin/bash
        // doesn't exist — bash lives in the Nix store and is only reachable
        // via PATH set by the systemd service.
        let bash_path = resolve_bash();
        let systemd_run_path = resolve_from_path("systemd-run");

        if let Some(ref path) = systemd_run_path {
            info!(?path, "systemd-run found, workflow isolation enabled");
        } else {
            info!("systemd-run not found, workflow isolation disabled");
        }

        Self {
            workspace_mgr: WorkspaceManager::new(workspace_root),
            cache_dir: cache_dir.into(),
            timeout,
            extra_nix_flags,
            dev_mode,
            states: Arc::new(Mutex::new(HashMap::new())),
            bash_path,
            systemd_run_path,
            workflow_limits,
        }
    }
}

#[async_trait]
impl Engine for NixEngine {
    fn init_workflow(&self, twf: PipelineWorkflow, pipeline: &Pipeline) -> EngineResult<Workflow> {
        // Validate engine field.
        let engine = twf.engine.to_lowercase();
        if engine != "nix" && engine != "nixery" {
            return Err(EngineError::InvalidWorkflow(format!(
                "unsupported engine: {:?} (expected \"nix\" or \"nixery\")",
                twf.engine
            )));
        }

        let mut workflow = Workflow::new(&twf.name);

        // Parse the raw YAML.
        let deps = parse_dependencies_from_yaml(&twf.raw)?;
        let user_steps = parse_steps_from_yaml(&twf.raw)?;
        let env = parse_env_from_yaml(&twf.raw)?;

        // Store the parsed dependency map as workflow data for use in setup_workflow.
        if let Some(nix_deps) = NixDeps::parse(&deps) {
            workflow.data = Some(serde_json::to_value(&deps).map_err(|e| {
                EngineError::InvalidWorkflow(format!("failed to serialize deps: {e}"))
            })?);
            debug!(hash = %nix_deps.content_hash(), "parsed Nix dependencies");
        }

        // Merge workflow-level env vars.
        workflow.environment = env;

        // Build the clone step (unless skipped).
        let clone_opts = twf.clone.as_ref().map(|c| spindle_models::step::CloneOpts {
            depth: c.depth,
            skip: c.skip,
            submodules: c.submodules,
        });

        // Build the repo URL from the pipeline's owner/name.
        let repo_url = if self.dev_mode {
            format!(
                "http://localhost/{}/{}",
                pipeline.repo_owner, pipeline.repo_name
            )
        } else {
            format!(
                "https://tangled.org/{}/{}",
                pipeline.repo_owner, pipeline.repo_name
            )
        };

        let clone_step = CloneStep::build(
            &repo_url,
            pipeline.commit_sha.as_deref(),
            clone_opts.as_ref(),
        );

        if !clone_step.is_empty() {
            workflow.add_step(clone_step);
        }

        // Add user-defined steps.
        for (name, command) in user_steps {
            workflow.add_step(UserStep::new(name, command));
        }

        if workflow.steps.is_empty() {
            return Err(EngineError::InvalidWorkflow("workflow has no steps".into()));
        }

        Ok(workflow)
    }

    async fn setup_workflow(
        &self,
        wid: &WorkflowId,
        workflow: &Workflow,
        logger: &dyn WorkflowLogger,
    ) -> EngineResult<()> {
        // Create the workspace directory.
        let workspace_dir = self.workspace_mgr.create(wid).await?;

        // Build the Nix environment if dependencies are specified.
        let nix_env_path = if let Some(data) = &workflow.data {
            let deps: HashMap<String, Vec<String>> =
                serde_json::from_value(data.clone()).map_err(|e| {
                    EngineError::SetupFailed(format!("failed to deserialize deps: {e}"))
                })?;

            if let Some(nix_deps) = NixDeps::parse(&deps) {
                let path = build_nix_env(&nix_deps, &self.cache_dir, &self.extra_nix_flags, logger)
                    .await?;
                Some(path)
            } else {
                None
            }
        } else {
            None
        };

        // Store the state.
        let state = WorkflowState {
            nix_env_path,
            workspace_dir,
        };
        self.states.lock().await.insert(wid.to_string(), state);

        Ok(())
    }

    fn workflow_timeout(&self) -> Duration {
        self.timeout
    }

    async fn destroy_workflow(&self, wid: &WorkflowId) -> EngineResult<()> {
        // Remove the state.
        self.states.lock().await.remove(&wid.to_string());

        // Destroy the workspace directory.
        self.workspace_mgr.destroy(wid).await?;

        Ok(())
    }

    async fn run_step(
        &self,
        wid: &WorkflowId,
        workflow: &Workflow,
        step_idx: usize,
        secrets: &[UnlockedSecret],
        logger: &dyn WorkflowLogger,
    ) -> EngineResult<()> {
        let step = workflow
            .steps
            .get(step_idx)
            .ok_or_else(|| EngineError::Other(format!("step index {step_idx} out of bounds")))?;

        let states = self.states.lock().await;
        let state = states
            .get(&wid.to_string())
            .ok_or_else(|| EngineError::Other(format!("no state for workflow {wid}")))?;

        let workspace_dir = &state.workspace_dir;
        let command_str = step.command();

        if command_str.is_empty() {
            debug!(%wid, step_idx, "skipping empty step");
            return Ok(());
        }

        // Build PATH: nix env bins + standard system paths.
        let path = build_path(state.nix_env_path.as_deref());

        // Build environment variables.
        let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());
        let mut env_vars: Vec<(String, String)> = vec![
            ("PATH".into(), path),
            ("HOME".into(), workspace_dir.to_string_lossy().into_owned()),
            ("USER".into(), user),
            ("CI".into(), "true".into()),
        ];

        // Add workflow-level env vars.
        for (k, v) in &workflow.environment {
            env_vars.push((k.clone(), v.clone()));
        }

        // Write secrets to a temporary file that the step sources, rather than
        // passing them as env vars (which are visible via /proc/*/environ to
        // concurrent workflows running under the same UID).
        let secrets_file = workspace_dir.join(format!(".spindle-secrets-{step_idx}"));
        if !secrets.is_empty() {
            use std::os::unix::fs::OpenOptionsExt;
            let mut contents = String::new();
            for secret in secrets {
                // Shell-escape the value using single quotes with embedded quote handling.
                let escaped = secret.value.replace('\'', "'\\''");
                contents.push_str(&format!("export {}='{}'\n", secret.key, escaped));
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&secrets_file)
                .map_err(|e| EngineError::StepFailed {
                    exit_code: -1,
                    message: format!("failed to write secrets file: {e}"),
                })?;
            std::fs::write(&secrets_file, &contents).map_err(|e| EngineError::StepFailed {
                exit_code: -1,
                message: format!("failed to write secrets file: {e}"),
            })?;
        }

        // Build the command: source the secrets file (if present) then run the step.
        let full_command = if !secrets.is_empty() {
            format!(
                ". '{}' && rm -f '{}' && {command_str}",
                secrets_file.display(),
                secrets_file.display()
            )
        } else {
            command_str
        };

        info!(%wid, step_idx, name = step.name(), "executing step");

        // Spawn the child process, optionally wrapped in a systemd scope for
        // per-workflow isolation (own cgroup with resource limits).
        // Scopes only support cgroup properties (MemoryMax, TasksMax, etc.),
        // not execution properties (PrivateTmp, WorkingDirectory).
        let mut child = 'spawn: {
            if let Some(ref systemd_run) = self.systemd_run_path {
                let scope_name =
                    format!("spindle-{}-{step_idx}", wid.to_string().replace('/', "-"));
                let mut cmd = Command::new(systemd_run);
                cmd.args(["--scope", "--collect", "--quiet"]);
                cmd.arg(format!("--unit={scope_name}"));

                if let Some(ref mem) = self.workflow_limits.memory_max {
                    cmd.args(["-p", &format!("MemoryMax={mem}")]);
                }
                if let Some(tasks) = self.workflow_limits.tasks_max {
                    cmd.args(["-p", &format!("TasksMax={tasks}")]);
                }

                // Pass environment variables via --setenv.
                for (k, v) in &env_vars {
                    cmd.arg("--setenv");
                    cmd.arg(format!("{k}={v}"));
                }

                cmd.arg("--");
                cmd.arg(&self.bash_path);
                cmd.args([
                    "--norc",
                    "--noprofile",
                    "-euo",
                    "pipefail",
                    "-c",
                    &full_command,
                ]);

                match cmd
                    .current_dir(workspace_dir)
                    .env_clear()
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                {
                    Ok(child) => break 'spawn child,
                    Err(e) => {
                        warn!(%e, "systemd-run scope failed, falling back to direct execution");
                    }
                }
            }

            Command::new(&self.bash_path)
                .args(["-euo", "pipefail", "-c", &full_command])
                .current_dir(workspace_dir)
                .env_clear()
                .envs(env_vars)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| EngineError::StepFailed {
                    exit_code: -1,
                    message: format!("failed to spawn step process: {e}"),
                })?
        };

        // Stream stdout and stderr to the logger.
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let mut stdout_writer = logger.data_writer(step_idx, "stdout".into());
        let mut stderr_writer = logger.data_writer(step_idx, "stderr".into());

        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = writeln!(stdout_writer, "{line}");
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = writeln!(stderr_writer, "{line}");
            }
        });

        // Wait for the process to complete.
        let status = child.wait().await.map_err(|e| EngineError::StepFailed {
            exit_code: -1,
            message: format!("failed to wait for step process: {e}"),
        })?;

        // Wait for stream tasks to finish.
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        // Always clean up the secrets file (the step rm's it on success, but
        // it may still exist if the step failed before reaching that point).
        let _ = std::fs::remove_file(&secrets_file);

        if !status.success() {
            let exit_code = status.code().unwrap_or(-1);
            error!(%wid, step_idx, exit_code, name = step.name(), "step failed");
            return Err(EngineError::StepFailed {
                exit_code,
                message: format!("step {:?} exited with code {exit_code}", step.name()),
            });
        }

        info!(%wid, step_idx, name = step.name(), "step completed successfully");
        Ok(())
    }
}

/// Build the PATH environment variable.
///
/// Includes the Nix environment's `bin` and `sbin` directories (if present),
/// the parent process's PATH (which on NixOS contains git, nix, etc. from
/// the systemd service's `path` attribute), and standard system paths.
fn build_path(nix_env: Option<&Path>) -> String {
    let mut parts = Vec::new();

    if let Some(env) = nix_env {
        parts.push(format!("{}/bin", env.display()));
        parts.push(format!("{}/sbin", env.display()));
    }

    // Include the parent process's PATH. On NixOS, tools like git and nix
    // live in the Nix store and are only reachable via PATH set by the
    // systemd service unit.
    if let Ok(parent_path) = std::env::var("PATH") {
        parts.push(parent_path);
    }

    // Standard system paths as fallback.
    parts.extend([
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/local/sbin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ]);

    parts.join(":")
}

/// Resolve the path to the `bash` binary from the current process's `PATH`.
///
/// On NixOS, `/bin/bash` doesn't exist. Bash lives in the Nix store and is
/// only reachable via PATH (set by the systemd service unit's `path`
/// attribute). This function finds bash at construction time so step
/// execution doesn't depend on hardcoded paths.
fn resolve_bash() -> PathBuf {
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let candidate = PathBuf::from(dir).join("bash");
            if candidate.exists() {
                info!(bash = %candidate.display(), "resolved bash binary from PATH");
                return candidate;
            }
        }
    }

    // Fallback to /bin/bash for non-NixOS systems.
    let fallback = PathBuf::from("/bin/bash");
    info!(bash = %fallback.display(), "using fallback bash path");
    fallback
}

/// Resolve a binary by name from PATH. Returns `None` if not found.
fn resolve_from_path(name: &str) -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path| {
        path.split(':')
            .map(|dir| PathBuf::from(dir).join(name))
            .find(|candidate| candidate.exists())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::PipelineCloneOpts;

    #[test]
    fn build_path_with_nix_env() {
        let path = build_path(Some(Path::new("/nix/store/abc123-env")));
        assert!(path.starts_with("/nix/store/abc123-env/bin:"));
        assert!(path.contains("/nix/store/abc123-env/sbin:"));
        assert!(path.contains("/usr/bin"));
    }

    #[test]
    fn build_path_without_nix_env() {
        let path = build_path(None);
        // PATH should contain standard system paths as fallbacks.
        assert!(path.contains("/usr/local/bin"));
        assert!(path.contains("/usr/bin"));
        assert!(path.contains("/bin"));
    }

    #[test]
    fn init_workflow_rejects_unknown_engine() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(300),
            vec![],
            false,
            WorkflowLimits::default(),
        );

        let twf = PipelineWorkflow {
            name: "test".into(),
            engine: "docker".into(),
            raw: "steps:\n  - name: test\n    run: echo hi\n".into(),
            clone: None,
        };
        let pipeline = Pipeline {
            repo_owner: "did:plc:test".into(),
            repo_name: "my-repo".into(),
            commit_sha: None,
            workflows: vec![],
        };

        let result = engine.init_workflow(twf, &pipeline);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported engine")
        );
    }

    #[test]
    fn init_workflow_basic() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(300),
            vec![],
            false,
            WorkflowLimits::default(),
        );

        let yaml = r#"
dependencies:
  nixpkgs:
    - nodejs
steps:
  - name: Build
    run: npm build
  - name: Test
    run: npm test
env:
  NODE_ENV: test
"#;

        let twf = PipelineWorkflow {
            name: "ci".into(),
            engine: "nix".into(),
            raw: yaml.into(),
            clone: None,
        };
        let pipeline = Pipeline {
            repo_owner: "did:plc:test".into(),
            repo_name: "my-repo".into(),
            commit_sha: None,
            workflows: vec![],
        };

        let wf = engine.init_workflow(twf, &pipeline).unwrap();
        assert_eq!(wf.name, "ci");
        // Clone step + 2 user steps
        assert_eq!(wf.steps.len(), 3);
        assert_eq!(wf.steps[0].name(), "Clone repository into workspace");
        assert_eq!(wf.steps[1].name(), "Build");
        assert_eq!(wf.steps[2].name(), "Test");
        assert_eq!(wf.environment["NODE_ENV"], "test");
        assert!(wf.data.is_some()); // Dependencies stored
    }

    #[test]
    fn init_workflow_skip_clone() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(300),
            vec![],
            false,
            WorkflowLimits::default(),
        );

        let yaml = "steps:\n  - name: Hello\n    run: echo hello\n";

        let twf = PipelineWorkflow {
            name: "test".into(),
            engine: "nix".into(),
            raw: yaml.into(),
            clone: Some(PipelineCloneOpts {
                depth: 0,
                skip: true,
                submodules: false,
            }),
        };
        let pipeline = Pipeline {
            repo_owner: "did:plc:test".into(),
            repo_name: "repo".into(),
            commit_sha: None,
            workflows: vec![],
        };

        let wf = engine.init_workflow(twf, &pipeline).unwrap();
        // Only user step, no clone step
        assert_eq!(wf.steps.len(), 1);
        assert_eq!(wf.steps[0].name(), "Hello");
    }

    #[test]
    fn init_workflow_accepts_nixery_engine() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(300),
            vec![],
            false,
            WorkflowLimits::default(),
        );

        let yaml = "steps:\n  - name: test\n    run: echo hi\n";

        let twf = PipelineWorkflow {
            name: "test".into(),
            engine: "nixery".into(),
            raw: yaml.into(),
            clone: None,
        };
        let pipeline = Pipeline {
            repo_owner: "did:plc:test".into(),
            repo_name: "repo".into(),
            commit_sha: None,
            workflows: vec![],
        };

        // Should succeed — we accept "nixery" as an alias for compatibility.
        assert!(engine.init_workflow(twf, &pipeline).is_ok());
    }

    #[test]
    fn init_workflow_no_steps_error() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(300),
            vec![],
            false,
            WorkflowLimits::default(),
        );

        let yaml = "steps: []\n";

        let twf = PipelineWorkflow {
            name: "empty".into(),
            engine: "nix".into(),
            raw: yaml.into(),
            clone: Some(PipelineCloneOpts {
                skip: true,
                depth: 0,
                submodules: false,
            }),
        };
        let pipeline = Pipeline {
            repo_owner: "did:plc:test".into(),
            repo_name: "repo".into(),
            commit_sha: None,
            workflows: vec![],
        };

        let result = engine.init_workflow(twf, &pipeline);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no steps"));
    }

    #[test]
    fn workflow_timeout_returns_configured_value() {
        let engine = NixEngine::new(
            "/tmp/ws",
            "/tmp/cache",
            Duration::from_secs(600),
            vec![],
            false,
            WorkflowLimits::default(),
        );
        assert_eq!(engine.workflow_timeout(), Duration::from_secs(600));
    }
}
