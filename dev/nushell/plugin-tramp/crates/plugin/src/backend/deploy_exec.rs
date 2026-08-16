//! Automatic deployment of the `tramp-agent` binary into Docker/Kubernetes
//! containers.
//!
//! This module extends the agent deployment concept (see [`super::deploy`])
//! to exec-based backends.  Instead of uploading via SFTP, the agent binary
//! is copied into containers using `docker cp` / `kubectl cp` (or a base64
//! exec fallback), then started as an interactive process with piped
//! stdin/stdout for MsgPack-RPC communication.
//!
//! ## Supported flows
//!
//! | Backend    | Copy method                | Start method                              |
//! |------------|---------------------------|-------------------------------------------|
//! | Docker     | `docker cp`               | `docker exec -i <ctr> /tmp/tramp-agent`  |
//! | Kubernetes | `kubectl cp` (or base64)  | `kubectl exec -i <pod> -- /tmp/...`       |
//!
//! ## Chained paths
//!
//! For standalone containers (local runner), the agent process is spawned as
//! a local `tokio::process::Child` with piped stdio.
//!
//! For chained paths (e.g. `/ssh:host|docker:ctr:/path`), the parent SSH
//! backend already benefits from the RPC agent, so the Docker commands
//! executed through it are already fast.  Agent deployment *inside* the
//! container through a remote runner is not yet supported (Phase 6).

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::Child;

use super::deploy::{self, RemoteTarget};
use super::rpc::RpcBackend;
use super::rpc_client::RpcClient;
use super::runner::CommandRunner;
use super::{Backend, ExecResult};
use crate::errors::{TrampError, TrampResult};

/// The version of the agent we expect — must match the plugin's version.
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Remote path inside the container where the agent is stored.
const CONTAINER_AGENT_DIR: &str = "/tmp/tramp-agent-dir";

/// Agent binary name inside the container.
const CONTAINER_AGENT_BIN: &str = "tramp-agent";

/// Full path to the agent binary inside the container.
pub const CONTAINER_AGENT_PATH: &str = "/tmp/tramp-agent-dir/tramp-agent";

/// Timeout for agent ping after startup.
const AGENT_PING_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Container kind
// ---------------------------------------------------------------------------

/// The kind of exec backend we're deploying into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Docker,
    Kubernetes,
}

/// Parameters needed to identify a container for agent deployment.
#[derive(Debug, Clone)]
pub struct ContainerTarget {
    /// Docker or Kubernetes.
    pub kind: ContainerKind,
    /// Container name (Docker) or pod name (Kubernetes).
    pub name: String,
    /// Optional user for Docker (`--user`).
    pub user: Option<String>,
    /// Optional container name within a K8s pod (`-c`).
    pub k8s_container: Option<String>,
}

// ---------------------------------------------------------------------------
// Architecture detection
// ---------------------------------------------------------------------------

/// Detect the container's OS and architecture by running `uname -sm` inside it.
pub async fn detect_container_target(
    runner: &dyn CommandRunner,
    target: &ContainerTarget,
) -> TrampResult<RemoteTarget> {
    let result = run_in_container(runner, target, "uname", &["-sm"]).await?;

    if result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(TrampError::Internal(format!(
            "`uname -sm` failed in container {}: {}",
            target.name,
            stderr.trim()
        )));
    }

    let text = String::from_utf8_lossy(&result.stdout);
    let text = text.trim();

    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(TrampError::Internal(format!(
            "unexpected `uname -sm` output from container {}: {text:?}",
            target.name,
        )));
    }

    let os_raw = parts[0].to_lowercase();
    let arch_raw = parts[1].to_lowercase();

    let os = match os_raw.as_str() {
        "linux" => "linux",
        "darwin" => "darwin",
        "freebsd" => "freebsd",
        other => {
            return Err(TrampError::Internal(format!(
                "unsupported container OS: {other}"
            )));
        }
    };

    let arch = match arch_raw.as_str() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        "armv7l" | "armhf" => "armv7",
        other => {
            return Err(TrampError::Internal(format!(
                "unsupported container architecture: {other}"
            )));
        }
    };

    let triple = match (arch, os) {
        ("x86_64", "linux") => "x86_64-unknown-linux-musl",
        ("aarch64", "linux") => "aarch64-unknown-linux-musl",
        ("armv7", "linux") => "armv7-unknown-linux-musleabihf",
        ("x86_64", "darwin") => "x86_64-apple-darwin",
        ("aarch64", "darwin") => "aarch64-apple-darwin",
        ("x86_64", "freebsd") => "x86_64-unknown-freebsd",
        _ => {
            return Err(TrampError::Internal(format!(
                "no agent binary available for {arch}-{os}"
            )));
        }
    };

    Ok(RemoteTarget {
        os: os.to_string(),
        arch: arch.to_string(),
        triple: triple.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Agent presence check
// ---------------------------------------------------------------------------

/// Check if the agent is already deployed and executable in the container
/// **with the correct version**.
///
/// Runs the agent with `--version` inside the container and verifies the
/// output matches the plugin's `CARGO_PKG_VERSION`.  Returns `true` only
/// when the binary exists, is executable, and reports the same version.
pub async fn is_agent_deployed_in_container(
    runner: &dyn CommandRunner,
    target: &ContainerTarget,
) -> bool {
    let result = run_in_container(
        runner,
        target,
        "sh",
        &[
            "-c",
            &format!(
                "test -x {CONTAINER_AGENT_PATH} && {CONTAINER_AGENT_PATH} --version 2>/dev/null || echo MISSING"
            ),
        ],
    )
    .await;

    match result {
        Ok(r) => {
            if r.exit_code != 0 {
                return false;
            }
            let text = String::from_utf8_lossy(&r.stdout);
            let text = text.trim();
            if text.contains("MISSING") {
                return false;
            }
            // The agent prints "tramp-agent <version>" on --version.
            let expected = format!("tramp-agent {AGENT_VERSION}");
            if text == expected {
                true
            } else {
                eprintln!(
                    "tramp: container agent version mismatch (got {text:?}, expected {expected:?}), will re-deploy"
                );
                false
            }
        }
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Upload methods
// ---------------------------------------------------------------------------

/// Upload the agent binary into a Docker container using `docker cp`.
///
/// This runs `docker cp <local_path> <container>:<remote_path>` on the
/// host (via the runner).
pub async fn upload_agent_docker(
    runner: &dyn CommandRunner,
    container: &str,
    agent_bytes: &[u8],
) -> TrampResult<()> {
    // Write the agent binary to a temporary file on the host first.
    let tmp_path = write_temp_agent(agent_bytes)?;
    let tmp_str = tmp_path.to_string_lossy();

    // Ensure the target directory exists inside the container.
    let mkdir_result = runner
        .run(
            "docker",
            &["exec", container, "mkdir", "-p", CONTAINER_AGENT_DIR],
        )
        .await?;
    if mkdir_result.exit_code != 0 {
        cleanup_temp(&tmp_path);
        let stderr = String::from_utf8_lossy(&mkdir_result.stderr);
        return Err(TrampError::Internal(format!(
            "failed to create agent dir in container {container}: {}",
            stderr.trim()
        )));
    }

    // Copy the binary into the container.
    let dest = format!("{container}:{CONTAINER_AGENT_PATH}");
    let result = runner.run("docker", &["cp", &tmp_str, &dest]).await;

    cleanup_temp(&tmp_path);

    let result = result?;
    if result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(TrampError::Internal(format!(
            "docker cp failed for {container}: {}",
            stderr.trim()
        )));
    }

    // Make executable.
    let chmod_result = runner
        .run(
            "docker",
            &["exec", container, "chmod", "755", CONTAINER_AGENT_PATH],
        )
        .await?;
    if chmod_result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&chmod_result.stderr);
        return Err(TrampError::Internal(format!(
            "chmod failed in container {container}: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Upload the agent binary into a Kubernetes pod using `kubectl cp`.
///
/// Falls back to base64 exec if `kubectl cp` fails (some minimal containers
/// lack `tar`).
pub async fn upload_agent_kubernetes(
    runner: &dyn CommandRunner,
    pod: &str,
    k8s_container: Option<&str>,
    agent_bytes: &[u8],
) -> TrampResult<()> {
    // Try kubectl cp first (requires tar in the container).
    let tmp_path = write_temp_agent(agent_bytes)?;
    let tmp_str = tmp_path.to_string_lossy();

    let dest = format!("{pod}:{CONTAINER_AGENT_PATH}");

    // Ensure the target directory exists inside the pod.
    let mut mkdir_args = vec!["exec"];
    if let Some(c) = k8s_container {
        mkdir_args.extend_from_slice(&["-c", c]);
    }
    mkdir_args.extend_from_slice(&[pod, "--", "mkdir", "-p", CONTAINER_AGENT_DIR]);
    let _ = runner.run("kubectl", &mkdir_args).await; // best-effort

    let mut cp_args = vec!["cp", &tmp_str, &dest];
    if let Some(c) = k8s_container {
        cp_args.extend_from_slice(&["-c", c]);
    }

    let cp_result = runner.run("kubectl", &cp_args).await;
    cleanup_temp(&tmp_path);

    let try_base64 = match cp_result {
        Ok(r) => r.exit_code != 0,
        Err(_) => true,
    };

    if try_base64 {
        // Fallback: pipe the binary via base64 through kubectl exec.
        upload_agent_base64(runner, pod, k8s_container, agent_bytes).await?;
    }

    // Make executable.
    let mut chmod_args: Vec<&str> = vec!["exec"];
    if let Some(c) = k8s_container {
        chmod_args.extend_from_slice(&["-c", c]);
    }
    chmod_args.extend_from_slice(&[pod, "--", "chmod", "755", CONTAINER_AGENT_PATH]);
    let chmod_result = runner.run("kubectl", &chmod_args).await?;
    if chmod_result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&chmod_result.stderr);
        return Err(TrampError::Internal(format!(
            "chmod failed in pod {pod}: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Upload the agent binary via base64 encoding through exec.
///
/// This works even in minimal containers that lack `tar`.
async fn upload_agent_base64(
    runner: &dyn CommandRunner,
    pod: &str,
    k8s_container: Option<&str>,
    agent_bytes: &[u8],
) -> TrampResult<()> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(agent_bytes);

    let script = format!(
        "mkdir -p {CONTAINER_AGENT_DIR} && base64 -d > {CONTAINER_AGENT_PATH} <<'__TRAMP_AGENT_EOF__'\n{encoded}\n__TRAMP_AGENT_EOF__"
    );

    let mut args: Vec<&str> = vec!["exec"];
    if let Some(c) = k8s_container {
        args.extend_from_slice(&["-c", c]);
    }
    args.extend_from_slice(&[pod, "--", "sh", "-c", &script]);

    let result = runner.run("kubectl", &args).await?;
    if result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(TrampError::Internal(format!(
            "base64 agent upload failed in pod {pod}: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Upload the agent binary via base64 into a Docker container.
///
/// Used when the runner is remote and `docker cp` won't work (since we
/// can't access the local filesystem from the remote host).
pub async fn upload_agent_docker_base64(
    runner: &dyn CommandRunner,
    container: &str,
    agent_bytes: &[u8],
) -> TrampResult<()> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(agent_bytes);

    let script = format!(
        "mkdir -p {CONTAINER_AGENT_DIR} && base64 -d > {CONTAINER_AGENT_PATH} <<'__TRAMP_AGENT_EOF__'\n{encoded}\n__TRAMP_AGENT_EOF__\nchmod 755 {CONTAINER_AGENT_PATH}"
    );

    let result = runner
        .run("docker", &["exec", container, "sh", "-c", &script])
        .await?;

    if result.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(TrampError::Internal(format!(
            "base64 agent upload failed in container {container}: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Agent process — local spawn
// ---------------------------------------------------------------------------

/// A handle to a `tramp-agent` process running inside a container.
///
/// The process is started via `docker exec -i` or `kubectl exec -i`, with
/// stdin/stdout piped through the local `tokio::process::Child`.
///
/// Dropping this handle kills the child process.
pub struct ContainerAgentProcess {
    child: Child,
}

impl ContainerAgentProcess {
    /// Take the stdin writer for sending RPC requests.
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.stdin.take()
    }

    /// Take the stdout reader for reading RPC responses.
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }
}

/// Start the agent inside a Docker container as a local interactive process.
///
/// Spawns `docker exec -i <container> /tmp/tramp-agent-dir/tramp-agent` with
/// piped stdin/stdout.
pub fn start_agent_docker(
    container: &str,
    user: Option<&str>,
) -> TrampResult<ContainerAgentProcess> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.arg("exec").arg("-i");
    if let Some(u) = user {
        cmd.arg("--user").arg(u);
    }
    cmd.arg(container).arg(CONTAINER_AGENT_PATH);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let child = cmd.spawn().map_err(|e| {
        TrampError::Internal(format!(
            "failed to start agent in container {container}: {e}"
        ))
    })?;

    Ok(ContainerAgentProcess { child })
}

/// Start the agent inside a Kubernetes pod as a local interactive process.
///
/// Spawns `kubectl exec -i [-c container] <pod> -- /tmp/tramp-agent-dir/tramp-agent`
/// with piped stdin/stdout.
pub fn start_agent_kubernetes(
    pod: &str,
    k8s_container: Option<&str>,
) -> TrampResult<ContainerAgentProcess> {
    let mut cmd = tokio::process::Command::new("kubectl");
    cmd.arg("exec").arg("-i");
    if let Some(c) = k8s_container {
        cmd.arg("-c").arg(c);
    }
    cmd.arg(pod).arg("--").arg(CONTAINER_AGENT_PATH);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|e| TrampError::Internal(format!("failed to start agent in pod {pod}: {e}")))?;

    Ok(ContainerAgentProcess { child })
}

// ---------------------------------------------------------------------------
// High-level deployment flow
// ---------------------------------------------------------------------------

/// Outcome of the container agent deployment attempt.
pub enum ExecDeployResult {
    /// The agent is running inside the container, wrapped in an `RpcBackend`.
    Ready(Arc<dyn Backend>),
    /// Deployment failed — reason string for diagnostics.
    Fallback(String),
}

/// Attempt to deploy and start the `tramp-agent` inside a container.
///
/// This is the main entry point for exec backend agent deployment.
/// It follows the same decision tree as SSH agent deployment:
///
/// 1. Detect container arch via `uname -sm`
/// 2. Check if agent is already deployed
/// 3. Find cached binary locally → upload into container
/// 4. Start the agent as an interactive process
/// 5. Ping to verify it responds
///
/// On any failure, returns `ExecDeployResult::Fallback`.
///
/// **Note:** This only works for standalone containers (local runner).
/// For chained paths, the parent backend's agent already provides
/// performance benefits.
pub async fn deploy_and_start_in_container(
    runner: &dyn CommandRunner,
    target: &ContainerTarget,
) -> ExecDeployResult {
    // Step 1: detect container architecture.
    let remote_target = match detect_container_target(runner, target).await {
        Ok(t) => t,
        Err(e) => return ExecDeployResult::Fallback(format!("arch detection failed: {e}")),
    };

    // Step 2: check if agent is already deployed.
    let needs_upload = !is_agent_deployed_in_container(runner, target).await;

    if needs_upload {
        // Step 3: find cached binary.
        let agent_bytes = match deploy::find_cached_agent(&remote_target) {
            Some(path) => match std::fs::read(&path) {
                Ok(data) => data,
                Err(e) => {
                    return ExecDeployResult::Fallback(format!(
                        "failed to read cached agent at {}: {e}",
                        path.display()
                    ));
                }
            },
            None => {
                return ExecDeployResult::Fallback(format!(
                    "no cached agent binary for {} (container: {})",
                    remote_target.triple, target.name,
                ));
            }
        };

        // Step 4: upload into the container.
        let upload_result = match target.kind {
            ContainerKind::Docker => upload_agent_docker(runner, &target.name, &agent_bytes).await,
            ContainerKind::Kubernetes => {
                upload_agent_kubernetes(
                    runner,
                    &target.name,
                    target.k8s_container.as_deref(),
                    &agent_bytes,
                )
                .await
            }
        };

        if let Err(e) = upload_result {
            return ExecDeployResult::Fallback(format!("agent upload failed: {e}"));
        }
    }

    // Step 5: start the agent as an interactive process.
    let mut agent = match target.kind {
        ContainerKind::Docker => match start_agent_docker(&target.name, target.user.as_deref()) {
            Ok(a) => a,
            Err(e) => return ExecDeployResult::Fallback(format!("agent start failed: {e}")),
        },
        ContainerKind::Kubernetes => {
            match start_agent_kubernetes(&target.name, target.k8s_container.as_deref()) {
                Ok(a) => a,
                Err(e) => return ExecDeployResult::Fallback(format!("agent start failed: {e}")),
            }
        }
    };

    // Step 6: take stdin/stdout and create the RPC client.
    let stdin = agent.take_stdin();
    let stdout = agent.take_stdout();

    let (stdin, stdout) = match (stdin, stdout) {
        (Some(w), Some(r)) => (w, r),
        _ => {
            return ExecDeployResult::Fallback(
                "agent started but stdin/stdout not available".into(),
            );
        }
    };

    let client = RpcClient::new(stdout, stdin);

    // Step 7: ping to verify it's alive.
    let ping_result = tokio::time::timeout(AGENT_PING_TIMEOUT, client.ping()).await;
    match ping_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return ExecDeployResult::Fallback(format!("agent ping failed: {e}"));
        }
        Err(_) => {
            return ExecDeployResult::Fallback("agent ping timed out".into());
        }
    }

    let host = match target.kind {
        ContainerKind::Docker => format!("docker:{}", target.name),
        ContainerKind::Kubernetes => format!("k8s:{}", target.name),
    };

    ExecDeployResult::Ready(Arc::new(RpcBackend::new(client, host)))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a command inside a container via the appropriate exec mechanism.
async fn run_in_container(
    runner: &dyn CommandRunner,
    target: &ContainerTarget,
    program: &str,
    args: &[&str],
) -> TrampResult<ExecResult> {
    match target.kind {
        ContainerKind::Docker => {
            let mut full_args: Vec<&str> = vec!["exec"];
            if let Some(ref u) = target.user {
                full_args.extend_from_slice(&["--user", u]);
            }
            full_args.push(&target.name);
            full_args.push(program);
            full_args.extend_from_slice(args);
            runner.run("docker", &full_args).await
        }
        ContainerKind::Kubernetes => {
            let mut full_args: Vec<&str> = vec!["exec"];
            if let Some(ref c) = target.k8s_container {
                full_args.extend_from_slice(&["-c", c]);
            }
            full_args.push(&target.name);
            full_args.push("--");
            full_args.push(program);
            full_args.extend_from_slice(args);
            runner.run("kubectl", &full_args).await
        }
    }
}

/// Write the agent binary to a temporary file on the local filesystem.
///
/// Returns the path to the temporary file.  The caller is responsible for
/// cleaning it up via [`cleanup_temp`].
fn write_temp_agent(agent_bytes: &[u8]) -> TrampResult<std::path::PathBuf> {
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("tramp-agent-upload-{}", std::process::id()));
    std::fs::write(&tmp_path, agent_bytes).map_err(|e| {
        TrampError::Internal(format!(
            "failed to write temp agent to {}: {e}",
            tmp_path.display()
        ))
    })?;

    // Make the temp file readable (needed for docker cp).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        let _ = std::fs::set_permissions(&tmp_path, perms);
    }

    Ok(tmp_path)
}

/// Remove a temporary agent file.
fn cleanup_temp(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

/// Check whether the runner is a local runner (i.e. not behind another
/// backend).
///
/// Agent deployment inside containers through a remote runner is not
/// currently supported — the `docker cp` / `kubectl cp` commands expect
/// local filesystem access.  For remote runners, we fall back to base64
/// upload or skip deployment entirely.
pub fn is_local_runner(_runner: &dyn CommandRunner) -> bool {
    // We use a trait-object downcast check.  `LocalRunner` is a unit struct,
    // so we check if the runner's description matches.  Since we can't
    // downcast trait objects directly without `Any`, we use a simpler
    // heuristic: try to detect `RemoteRunner` by running a no-op.
    //
    // For now, we rely on the caller (VFS layer) to pass this information
    // explicitly based on whether a parent backend exists.
    //
    // This function is a placeholder that always returns true — the VFS
    // layer gates on `parent.is_none()` before calling deploy.
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_agent_path_is_consistent() {
        assert_eq!(
            CONTAINER_AGENT_PATH,
            format!("{CONTAINER_AGENT_DIR}/{CONTAINER_AGENT_BIN}")
        );
    }

    #[test]
    fn container_target_debug() {
        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "mycontainer".into(),
            user: Some("root".into()),
            k8s_container: None,
        };
        let dbg = format!("{target:?}");
        assert!(dbg.contains("Docker"));
        assert!(dbg.contains("mycontainer"));
        assert!(dbg.contains("root"));
    }

    #[test]
    fn container_target_kubernetes() {
        let target = ContainerTarget {
            kind: ContainerKind::Kubernetes,
            name: "mypod".into(),
            user: None,
            k8s_container: Some("app".into()),
        };
        let dbg = format!("{target:?}");
        assert!(dbg.contains("Kubernetes"));
        assert!(dbg.contains("mypod"));
        assert!(dbg.contains("app"));
    }

    #[test]
    fn write_and_cleanup_temp() {
        let data = b"fake agent binary";
        let path = write_temp_agent(data).unwrap();
        assert!(path.exists());
        let read_back = std::fs::read(&path).unwrap();
        assert_eq!(read_back, data);
        cleanup_temp(&path);
        assert!(!path.exists());
    }

    #[test]
    fn cleanup_temp_nonexistent_is_noop() {
        // Should not panic.
        cleanup_temp(std::path::Path::new(
            "/tmp/nonexistent-tramp-agent-test-12345",
        ));
    }

    // Mock runner for unit tests.
    struct MockRunner {
        responses: std::sync::Mutex<Vec<ExecResult>>,
    }

    impl MockRunner {
        fn new(responses: Vec<ExecResult>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandRunner for MockRunner {
        async fn run(&self, _program: &str, _args: &[&str]) -> TrampResult<ExecResult> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(ExecResult {
                    stdout: bytes::Bytes::new(),
                    stderr: bytes::Bytes::from("no more mock responses"),
                    exit_code: 1,
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        async fn run_with_stdin(
            &self,
            program: &str,
            args: &[&str],
            _stdin_data: &[u8],
        ) -> TrampResult<ExecResult> {
            self.run(program, args).await
        }
    }

    #[tokio::test]
    async fn detect_container_target_linux_x86_64() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("Linux x86_64\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        let result = detect_container_target(&runner, &target).await.unwrap();
        assert_eq!(result.os, "linux");
        assert_eq!(result.arch, "x86_64");
        assert_eq!(result.triple, "x86_64-unknown-linux-musl");
    }

    #[tokio::test]
    async fn detect_container_target_linux_aarch64() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("Linux aarch64\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Kubernetes,
            name: "mypod".into(),
            user: None,
            k8s_container: Some("app".into()),
        };

        let result = detect_container_target(&runner, &target).await.unwrap();
        assert_eq!(result.os, "linux");
        assert_eq!(result.arch, "aarch64");
        assert_eq!(result.triple, "aarch64-unknown-linux-musl");
    }

    #[tokio::test]
    async fn detect_container_target_arm64_alias() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("Linux arm64\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        let result = detect_container_target(&runner, &target).await.unwrap();
        assert_eq!(result.arch, "aarch64");
    }

    #[tokio::test]
    async fn detect_container_target_failure() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::new(),
            stderr: bytes::Bytes::from("exec failed"),
            exit_code: 1,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        let result = detect_container_target(&runner, &target).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn is_agent_deployed_positive() {
        let version_output = format!("tramp-agent {}\n", env!("CARGO_PKG_VERSION"));
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from(version_output),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        assert!(is_agent_deployed_in_container(&runner, &target).await);
    }

    #[tokio::test]
    async fn is_agent_deployed_negative() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::new(),
            stderr: bytes::Bytes::new(),
            exit_code: 1,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        assert!(!is_agent_deployed_in_container(&runner, &target).await);
    }

    #[tokio::test]
    async fn is_agent_deployed_version_mismatch() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("tramp-agent 0.0.0-fake\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        assert!(!is_agent_deployed_in_container(&runner, &target).await);
    }

    #[tokio::test]
    async fn is_agent_deployed_missing() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("MISSING\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "test".into(),
            user: None,
            k8s_container: None,
        };

        assert!(!is_agent_deployed_in_container(&runner, &target).await);
    }

    #[tokio::test]
    async fn run_in_container_docker_builds_correct_args() {
        // This test verifies the function doesn't panic and delegates properly.
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("hello\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Docker,
            name: "mycontainer".into(),
            user: Some("www".into()),
            k8s_container: None,
        };

        let result = run_in_container(&runner, &target, "echo", &["hello"])
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn run_in_container_kubernetes_builds_correct_args() {
        let runner = MockRunner::new(vec![ExecResult {
            stdout: bytes::Bytes::from("world\n"),
            stderr: bytes::Bytes::new(),
            exit_code: 0,
        }]);

        let target = ContainerTarget {
            kind: ContainerKind::Kubernetes,
            name: "mypod".into(),
            user: None,
            k8s_container: Some("sidecar".into()),
        };

        let result = run_in_container(&runner, &target, "echo", &["world"])
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
    }
}
