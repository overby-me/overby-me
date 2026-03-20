//! Nix engine implementation.
//!
//! Replaces Docker+Nixery with native Nix builds and child process execution.
//! Each workflow runs inside a single hakoniwa container with PID, IPC, and
//! mount namespace isolation. Steps execute sequentially within the container
//! via a stdin command protocol.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
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

/// Sentinel written to stdout after each step to signal completion.
/// Format: `\nSPINDLE_STEP_EXIT:<exit_code>\n`
const STEP_SENTINEL_PREFIX: &str = "SPINDLE_STEP_EXIT:";

/// Per-workflow runtime state, stored while the workflow is active.
struct WorkflowState {
    /// Path to the built Nix environment (store path with `bin/`, `sbin/`).
    nix_env_path: Option<PathBuf>,
    /// Host path managed by WorkspaceManager.
    workspace_dir: PathBuf,
    /// The running hakoniwa container child process.
    /// Stdin is used to send step commands, stdout/stderr for output.
    container: Option<hakoniwa::Child>,
}

// hakoniwa::Child contains PipeReader/PipeWriter which aren't Debug
impl std::fmt::Debug for WorkflowState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowState")
            .field("nix_env_path", &self.nix_env_path)
            .field("workspace_dir", &self.workspace_dir)
            .field("container", &self.container.as_ref().map(|c| c.id()))
            .finish()
    }
}

/// Per-workflow resource limits.
#[derive(Debug, Clone, Default)]
pub struct WorkflowLimits {
    /// Maximum virtual memory per workflow in bytes (hakoniwa --limit-as).
    pub limit_as: Option<u64>,
    /// Maximum wall time per step in seconds (hakoniwa --limit-walltime).
    pub limit_walltime: Option<u64>,
    /// Maximum number of open file descriptors (hakoniwa --limit-nofile).
    pub limit_nofile: Option<u64>,
}

/// The Nix engine: builds Nix closures from dependency specs and executes
/// workflow steps inside hakoniwa containers.
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
    /// Per-workflow resource limits.
    workflow_limits: WorkflowLimits,
}

impl NixEngine {
    /// Create a new Nix engine.
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        cache_dir: impl Into<PathBuf>,
        timeout: Duration,
        extra_nix_flags: Vec<String>,
        dev_mode: bool,
        workflow_limits: WorkflowLimits,
    ) -> Self {
        let bash_path = resolve_bash();

        Self {
            workspace_mgr: WorkspaceManager::new(workspace_root),
            cache_dir: cache_dir.into(),
            timeout,
            extra_nix_flags,
            dev_mode,
            states: Arc::new(Mutex::new(HashMap::new())),
            bash_path,
            workflow_limits,
        }
    }
}

/// The runner script that executes inside the hakoniwa container.
/// It reads commands from stdin (null-byte delimited) and executes each one,
/// printing a sentinel line with the exit code after each command.
/// Stderr is redirected to stdout so all output goes through one pipe —
/// this avoids the issue of a background stderr reader consuming output
/// across step boundaries.
fn build_runner_script() -> String {
    format!(
        r#"while IFS= read -r -d $'\0' __cmd; do
    bash -euo pipefail -c "$__cmd" 2>&1
    printf '\n{STEP_SENTINEL_PREFIX}%d\n' $?
done"#,
        STEP_SENTINEL_PREFIX = STEP_SENTINEL_PREFIX,
    )
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

        // Build the hakoniwa container.
        let mut container = hakoniwa::Container::new();
        container.unshare(hakoniwa::Namespace::Pid);
        container.unshare(hakoniwa::Namespace::Ipc);

        // Mount system paths read-only (/etc excluded — handled separately for DNS).
        for dir in ["/bin", "/lib", "/lib64", "/lib32", "/sbin", "/usr", "/nix"] {
            if Path::new(dir).exists() {
                container.bindmount_ro(dir, dir);
            }
        }

        // /etc: write resolv.conf directly (the host's may be a symlink to /run
        // which can't be bind-mounted in a user namespace). Also write a minimal
        // passwd/group so tools that look up users work.
        container.dir("/etc", 0o755);
        if let Ok(contents) = std::fs::read_to_string("/etc/resolv.conf") {
            container.file("/etc/resolv.conf", &contents);
        }
        if let Ok(contents) = std::fs::read_to_string("/etc/ssl/certs/ca-certificates.crt") {
            container.dir("/etc/ssl", 0o755);
            container.dir("/etc/ssl/certs", 0o755);
            container.file("/etc/ssl/certs/ca-certificates.crt", &contents);
        }
        // Minimal passwd/group for the container user.
        let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());
        container.file(
            "/etc/passwd",
            &format!("{user}:x:0:0::/workspace:/bin/bash\n"),
        );
        container.file("/etc/group", &format!("{user}:x:0:\n"));

        // Writable workspace, /dev, /tmp, /proc.
        container.tmpfsmount("/workspace");
        container.dir("/proc", 0o555);
        container.devfsmount("/dev");
        container.tmpfsmount("/tmp");

        if let Some(limit) = self.workflow_limits.limit_as {
            container.setrlimit(hakoniwa::Rlimit::As, limit, limit);
        }
        if let Some(limit) = self.workflow_limits.limit_nofile {
            container.setrlimit(hakoniwa::Rlimit::Nofile, limit, limit);
        }

        // Build environment variables for the container.
        let path = build_path(nix_env_path.as_deref());
        let user = std::env::var("USER").unwrap_or_else(|_| "nobody".into());

        // Spawn the container with a runner script that reads commands from stdin.
        let runner_script = build_runner_script();
        let mut hako_cmd = container.command(&self.bash_path.to_string_lossy());
        hako_cmd.args(["-c", &runner_script]);
        hako_cmd.current_dir("/workspace");
        hako_cmd.stdin(hakoniwa::Stdio::piped());
        hako_cmd.stdout(hakoniwa::Stdio::piped());
        hako_cmd.stderr(hakoniwa::Stdio::piped());

        hako_cmd.env("PATH", &path);
        hako_cmd.env("HOME", "/workspace");
        hako_cmd.env("USER", &user);
        hako_cmd.env("CI", "true");

        // Add workflow-level env vars.
        for (k, v) in &workflow.environment {
            hako_cmd.env(k, v);
        }

        let child = hako_cmd.spawn().map_err(|e| {
            EngineError::SetupFailed(format!("hakoniwa container spawn failed: {e}"))
        })?;

        info!(%wid, pid = child.id(), "hakoniwa container started");

        let state = WorkflowState {
            nix_env_path,
            workspace_dir,
            container: Some(child),
        };
        self.states.lock().await.insert(wid.to_string(), state);

        Ok(())
    }

    fn workflow_timeout(&self) -> Duration {
        self.timeout
    }

    async fn destroy_workflow(&self, wid: &WorkflowId) -> EngineResult<()> {
        // Remove the state and kill the container.
        if let Some(mut state) = self.states.lock().await.remove(&wid.to_string())
            && let Some(mut child) = state.container.take()
        {
            // Close stdin to signal the runner script to exit.
            drop(child.stdin.take());
            if let Err(e) = child.kill() {
                warn!(%e, %wid, "failed to kill hakoniwa container");
            }
        }

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

        let command_str = step.command();

        if command_str.is_empty() {
            debug!(%wid, step_idx, "skipping empty step");
            return Ok(());
        }

        // Build the step command with optional secrets sourcing.
        // Secrets are written to a temp file inside /workspace (container tmpfs),
        // sourced, then deleted before the actual command runs.
        let full_command = if !secrets.is_empty() {
            let mut secrets_script = String::new();
            for secret in secrets {
                let escaped = secret.value.replace('\'', "'\\''");
                secrets_script.push_str(&format!("export {}='{}'\n", secret.key, escaped));
            }
            format!(
                "eval {secrets}; {cmd}",
                secrets = shell_escape::unix::escape(secrets_script.into()),
                cmd = command_str,
            )
        } else {
            command_str
        };

        info!(%wid, step_idx, name = step.name(), "executing step");

        // Send the command to the container's stdin and read output.
        // This runs in a blocking task because hakoniwa uses sync I/O.
        // Stderr is redirected to stdout in the runner script, so all output
        // comes through the stdout pipe with sentinel markers.
        let wid_str = wid.to_string();
        let step_name = step.name().to_string();
        let mut output_writer = logger.data_writer(step_idx, "stdout".into());

        let mut states = self.states.lock().await;
        let state = states
            .get_mut(&wid_str)
            .ok_or_else(|| EngineError::Other(format!("no state for workflow {wid}")))?;

        let mut child = state.container.take().ok_or_else(|| {
            EngineError::Other(format!("container already consumed for workflow {wid}"))
        })?;

        // Drop the lock before the blocking task.
        drop(states);

        let result = tokio::task::spawn_blocking(move || {
            // Write the command to stdin (null-byte delimited).
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| EngineError::StepFailed {
                    exit_code: -1,
                    message: "container stdin not available".into(),
                })?;
            let mut cmd_bytes = full_command.into_bytes();
            cmd_bytes.push(0); // null byte delimiter
            stdin
                .write_all(&cmd_bytes)
                .map_err(|e| EngineError::StepFailed {
                    exit_code: -1,
                    message: format!("failed to write command to container stdin: {e}"),
                })?;
            stdin.flush().map_err(|e| EngineError::StepFailed {
                exit_code: -1,
                message: format!("failed to flush container stdin: {e}"),
            })?;

            // Read stdout line by line until we see the sentinel.
            // Stderr is merged into stdout by the runner script (2>&1).
            let mut exit_code: Option<i32> = None;
            if let Some(ref mut stdout) = child.stdout {
                let reader = std::io::BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(code_str) = line.strip_prefix(STEP_SENTINEL_PREFIX) {
                        exit_code = code_str.trim().parse().ok();
                        break;
                    }
                    let _ = writeln!(output_writer, "{line}");
                }
            }

            Ok::<(hakoniwa::Child, Option<i32>), EngineError>((child, exit_code))
        })
        .await
        .map_err(|e| EngineError::StepFailed {
            exit_code: -1,
            message: format!("step task panicked: {e}"),
        })?;

        let (child, exit_code) = result?;

        // Put the container back into the state for the next step.
        let mut states = self.states.lock().await;
        if let Some(state) = states.get_mut(&wid.to_string()) {
            state.container = Some(child);
        }
        drop(states);

        match exit_code {
            Some(0) => {
                info!(%wid, step_idx, name = %step_name, "step completed successfully");
                Ok(())
            }
            Some(code) => {
                error!(%wid, step_idx, exit_code = code, name = %step_name, "step failed");
                Err(EngineError::StepFailed {
                    exit_code: code,
                    message: format!("step {step_name:?} exited with code {code}"),
                })
            }
            None => {
                error!(%wid, step_idx, name = %step_name, "step failed: no exit code (container may have crashed)");
                Err(EngineError::StepFailed {
                    exit_code: -1,
                    message: format!("step {step_name:?}: no exit code received from container"),
                })
            }
        }
    }
}

/// Build the PATH environment variable.
fn build_path(nix_env: Option<&Path>) -> String {
    let mut parts = Vec::new();

    if let Some(env) = nix_env {
        parts.push(format!("{}/bin", env.display()));
        parts.push(format!("{}/sbin", env.display()));
    }

    // Include the parent process's PATH.
    if let Ok(parent_path) = std::env::var("PATH") {
        parts.push(parent_path);
    }

    parts.extend(["/usr/local/bin".into(), "/usr/bin".into(), "/bin".into()]);

    parts.join(":")
}

/// Resolve the bash binary from PATH.
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

    let fallback = PathBuf::from("/bin/bash");
    info!(bash = %fallback.display(), "using fallback bash path");
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::PipelineCloneOpts;

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
        assert_eq!(wf.steps.len(), 3);
        assert_eq!(wf.steps[0].name(), "Clone repository into workspace");
        assert_eq!(wf.steps[1].name(), "Build");
        assert_eq!(wf.steps[2].name(), "Test");
        assert_eq!(wf.environment["NODE_ENV"], "test");
        assert!(wf.data.is_some());
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

    #[test]
    fn build_path_with_nix_env() {
        let path = build_path(Some(Path::new("/nix/store/abc123-env")));
        assert!(path.starts_with("/nix/store/abc123-env/bin:"));
        assert!(path.contains("/nix/store/abc123-env/sbin:"));
    }

    #[test]
    fn build_path_without_nix_env() {
        let path = build_path(None);
        assert!(path.contains("/usr/bin"));
        assert!(path.contains("/bin"));
    }

    #[test]
    fn runner_script_contains_sentinel() {
        let script = build_runner_script();
        assert!(script.contains(STEP_SENTINEL_PREFIX));
        assert!(script.contains("read -r -d"));
    }
}
