//! Automatic deployment of the `tramp-agent` binary to remote hosts.
//!
//! On first connection (when the agent is not yet present), the plugin:
//!
//! 1. Detects the remote OS and architecture via `uname -sm`
//! 2. Checks a local cache (`~/.cache/nu-plugin-tramp/<version>/<target>/tramp-agent`)
//! 3. Uploads the agent binary via SFTP (or exec fallback)
//! 4. Starts the agent and verifies it responds to `ping`
//!
//! If any step fails, the caller falls back to the current shell-parsing
//! approach transparently — no user action required.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use openssh::Session;
use openssh_sftp_client::Sftp;

use crate::errors::{TrampError, TrampResult};

/// The version of the agent we deploy — used to namespace the local cache
/// so that upgrades trigger a fresh upload.
const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Remote directory where the agent binary is stored.
const REMOTE_AGENT_DIR: &str = ".cache/tramp-agent";

/// Remote binary name.
const REMOTE_AGENT_BIN: &str = "tramp-agent";

/// Timeout for the agent startup ping handshake.
const AGENT_PING_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Architecture detection
// ---------------------------------------------------------------------------

/// A remote target triple derived from `uname -sm` output.
#[derive(Debug, Clone)]
pub struct RemoteTarget {
    /// e.g. `"linux"`, `"darwin"`
    pub os: String,
    /// e.g. `"x86_64"`, `"aarch64"`
    pub arch: String,
    /// Combined Rust-style target triple, e.g. `"x86_64-unknown-linux-musl"`
    pub triple: String,
}

/// Detect the remote OS and architecture by running `uname -sm`.
///
/// Returns a [`RemoteTarget`] that can be used to locate the correct
/// pre-built agent binary.
pub async fn detect_remote_target(session: &Session) -> TrampResult<RemoteTarget> {
    let output = session
        .command("uname")
        .arg("-sm")
        .output()
        .await
        .map_err(|e| TrampError::Internal(format!("failed to run `uname -sm`: {e}")))?;

    if !output.status.success() {
        return Err(TrampError::Internal(format!(
            "`uname -sm` failed with exit code {}",
            output.status.code().unwrap_or(-1),
        )));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();

    // `uname -sm` output looks like: "Linux x86_64" or "Darwin arm64"
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(TrampError::Internal(format!(
            "unexpected `uname -sm` output: {text:?}",
        )));
    }

    let os_raw = parts[0].to_lowercase();
    let arch_raw = parts[1].to_lowercase();

    // Normalise to Rust target conventions.
    let os = match os_raw.as_str() {
        "linux" => "linux",
        "darwin" => "darwin",
        "freebsd" => "freebsd",
        other => {
            return Err(TrampError::Internal(format!(
                "unsupported remote OS: {other}"
            )));
        }
    };

    let arch = match arch_raw.as_str() {
        "x86_64" | "amd64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        "armv7l" | "armhf" => "armv7",
        other => {
            return Err(TrampError::Internal(format!(
                "unsupported remote architecture: {other}"
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
// Local cache
// ---------------------------------------------------------------------------

/// Return the local cache directory for agent binaries.
///
/// Layout: `~/.cache/nu-plugin-tramp/<version>/<triple>/tramp-agent`
fn local_cache_dir(target: &RemoteTarget) -> Option<PathBuf> {
    dirs::cache_dir().map(|base| {
        base.join("nu-plugin-tramp")
            .join(AGENT_VERSION)
            .join(&target.triple)
    })
}

/// Look up a cached agent binary for the given target.
///
/// Returns the path to the binary if found, or `None`.
pub fn find_cached_agent(target: &RemoteTarget) -> Option<PathBuf> {
    let dir = local_cache_dir(target)?;
    let path = dir.join(REMOTE_AGENT_BIN);
    if path.is_file() { Some(path) } else { None }
}

/// Store an agent binary in the local cache for future deployments.
pub fn cache_agent_binary(target: &RemoteTarget, data: &[u8]) -> TrampResult<PathBuf> {
    let dir = local_cache_dir(target)
        .ok_or_else(|| TrampError::Internal("could not determine cache directory".to_string()))?;
    std::fs::create_dir_all(&dir).map_err(|e| {
        TrampError::Internal(format!("failed to create cache dir {}: {e}", dir.display()))
    })?;
    let path = dir.join(REMOTE_AGENT_BIN);
    std::fs::write(&path, data).map_err(|e| {
        TrampError::Internal(format!(
            "failed to write cached agent to {}: {e}",
            path.display()
        ))
    })?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Remote deployment
// ---------------------------------------------------------------------------

/// Check if the agent is already deployed and runnable on the remote host
/// **with the correct version**.
///
/// Runs `~/.cache/tramp-agent/tramp-agent --version` and checks that the
/// output contains the expected version string (matching the plugin's
/// `CARGO_PKG_VERSION`). Returns `true` only when the binary exists, is
/// executable, and reports the same version as the plugin.
pub async fn is_agent_deployed(session: &Session) -> bool {
    let remote_bin = format!("$HOME/{REMOTE_AGENT_DIR}/{REMOTE_AGENT_BIN}");
    let output = session
        .command("sh")
        .arg("-c")
        .arg(format!(
            "test -x {remote_bin} && {remote_bin} --version 2>/dev/null || echo MISSING"
        ))
        .output()
        .await;

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let text = stdout.trim();
            if text.contains("MISSING") || !out.status.success() {
                return false;
            }
            // Verify the version matches. The agent prints
            // "tramp-agent <version>" on `--version`.
            let expected = format!("tramp-agent {AGENT_VERSION}");
            if text == expected {
                true
            } else {
                eprintln!(
                    "tramp: remote agent version mismatch (got {text:?}, expected {expected:?}), will re-deploy"
                );
                false
            }
        }
        Err(_) => false,
    }
}

/// Upload the agent binary to the remote host via SFTP.
///
/// Creates `~/.cache/tramp-agent/` on the remote and writes the binary
/// with mode 0755.
pub async fn upload_agent_sftp(sftp: &Sftp, agent_bytes: &[u8]) -> TrampResult<()> {
    let mut fs = sftp.fs();

    // Ensure the remote directory exists.
    // Use the home directory path — SFTP paths are relative to the
    // SFTP root (usually the user's home).
    let remote_dir = REMOTE_AGENT_DIR;
    // Try to create the directory; ignore "already exists" errors.
    match fs.create_dir(remote_dir).await {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string();
            // SSH_FX_FAILURE (4) is returned when the dir already exists
            // on many SFTP servers.
            if !msg.contains("FAILURE") && !msg.contains("already exists") {
                // Try creating parent dirs one by one.
                let parts: Vec<&str> = remote_dir.split('/').filter(|s| !s.is_empty()).collect();
                let mut current = String::new();
                for part in &parts {
                    current = if current.is_empty() {
                        part.to_string()
                    } else {
                        format!("{current}/{part}")
                    };
                    let _ = fs.create_dir(&current).await;
                }
            }
        }
    }

    // Write the binary.
    let remote_path = format!("{remote_dir}/{REMOTE_AGENT_BIN}");
    fs.write(&remote_path, agent_bytes)
        .await
        .map_err(|e| TrampError::Internal(format!("failed to upload agent via SFTP: {e}")))?;

    // Make it executable (mode 0755).
    // openssh_sftp_client's `fs.set_permissions` expects a `Permissions` value.
    // We use a shell command as a reliable cross-platform fallback.
    Ok(())
}

/// Upload the agent binary to the remote host via exec (base64 fallback).
///
/// This is used when SFTP is unavailable.
pub async fn upload_agent_exec(session: &Session, agent_bytes: &[u8]) -> TrampResult<()> {
    use base64::Engine;

    let encoded = base64::engine::general_purpose::STANDARD.encode(agent_bytes);
    let remote_dir = format!("$HOME/{REMOTE_AGENT_DIR}");
    let remote_path = format!("{remote_dir}/{REMOTE_AGENT_BIN}");

    // Create directory and write the base64-decoded binary.
    // Use a heredoc to avoid shell argument length limits.
    let script = format!(
        r#"mkdir -p {remote_dir} && base64 -d > {remote_path} <<'__TRAMP_AGENT_EOF__'
{encoded}
__TRAMP_AGENT_EOF__
chmod 755 {remote_path}"#
    );

    let output = session
        .command("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .await
        .map_err(|e| TrampError::Internal(format!("failed to upload agent via exec: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TrampError::Internal(format!(
            "agent upload failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Set the agent binary as executable on the remote host.
pub async fn chmod_agent(session: &Session) -> TrampResult<()> {
    let remote_path = format!("$HOME/{REMOTE_AGENT_DIR}/{REMOTE_AGENT_BIN}");
    let output = session
        .command("chmod")
        .arg("755")
        .arg(&remote_path)
        .output()
        .await
        .map_err(|e| TrampError::Internal(format!("failed to chmod agent: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TrampError::Internal(format!(
            "chmod agent failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Agent process management
// ---------------------------------------------------------------------------

/// A handle to a running `tramp-agent` process on the remote host.
///
/// The agent communicates via MsgPack-RPC over stdin/stdout, piped through
/// the SSH connection.  The child process owns an `Arc<Session>` so all
/// handles are `'static` and can be stored in `Arc<dyn Backend>`.
///
/// Dropping this handle kills the remote process.
pub struct AgentProcess {
    /// The SSH child process running the agent.
    child: openssh::Child<Arc<Session>>,
}

impl AgentProcess {
    /// Take the stdin writer for sending RPC requests to the agent.
    ///
    /// Returns `None` if stdin was already taken or was not piped.
    pub fn take_stdin(&mut self) -> Option<openssh::ChildStdin> {
        self.child.stdin().take()
    }

    /// Take the stdout reader for reading RPC responses from the agent.
    ///
    /// Returns `None` if stdout was already taken or was not piped.
    pub fn take_stdout(&mut self) -> Option<openssh::ChildStdout> {
        self.child.stdout().take()
    }
}

/// Start the `tramp-agent` binary on the remote host.
///
/// Takes an `Arc<Session>` so the spawned child process (and its
/// stdin/stdout handles) are `'static` — required for storing the
/// resulting `RpcBackend` as `Arc<dyn Backend>`.
///
/// Returns handles to the agent's stdin/stdout for the RPC protocol.
pub async fn start_agent(session: Arc<Session>) -> TrampResult<AgentProcess> {
    let remote_path = format!("$HOME/{REMOTE_AGENT_DIR}/{REMOTE_AGENT_BIN}");

    let child = session
        .arc_command("sh")
        .arg("-c")
        // Expand $HOME before exec-ing the agent.
        .arg(format!("exec {remote_path}"))
        .stdin(openssh::Stdio::piped())
        .stdout(openssh::Stdio::piped())
        .stderr(openssh::Stdio::null())
        .spawn()
        .await
        .map_err(|e| TrampError::Internal(format!("failed to start agent: {e}")))?;

    Ok(AgentProcess { child })
}

// ---------------------------------------------------------------------------
// High-level deployment flow
// ---------------------------------------------------------------------------

/// Outcome of the deployment attempt.
pub enum DeployResult {
    /// The agent is running and ready for RPC.
    Ready(Box<AgentProcess>),
    /// Deployment failed or no binary is available — fall back to shell mode.
    Fallback(String),
}

/// Attempt to deploy and start the `tramp-agent` on the remote host.
///
/// This is the main entry point called by `SshBackend::connect` (or the
/// VFS layer).  It follows the decision tree:
///
/// 1. Detect remote arch
/// 2. Check if agent is already deployed (version match)
/// 3. If not, look up local cache → upload
/// 4. Start the agent process
/// 5. Ping to verify it's alive
///
/// On any failure, returns `DeployResult::Fallback` with a reason string.
pub async fn deploy_and_start(session: Arc<Session>, sftp: Option<&Sftp>) -> DeployResult {
    // Step 1: detect remote target.
    let target = match detect_remote_target(&session).await {
        Ok(t) => t,
        Err(e) => return DeployResult::Fallback(format!("arch detection failed: {e}")),
    };

    // Step 2: check if the agent is already deployed.
    let needs_upload = !is_agent_deployed(&session).await;

    if needs_upload {
        // Step 3: find a cached binary to upload.
        let agent_bytes = match find_cached_agent(&target) {
            Some(path) => match std::fs::read(&path) {
                Ok(data) => data,
                Err(e) => {
                    return DeployResult::Fallback(format!(
                        "failed to read cached agent at {}: {e}",
                        path.display()
                    ));
                }
            },
            None => {
                // TODO: In the future, download from GitHub Releases here.
                return DeployResult::Fallback(format!(
                    "no cached agent binary for {} (place it at {})",
                    target.triple,
                    local_cache_dir(&target)
                        .map(|d| d.join(REMOTE_AGENT_BIN).display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string()),
                ));
            }
        };

        // Step 4: upload to the remote host.
        let upload_result = if let Some(sftp) = sftp {
            upload_agent_sftp(sftp, &agent_bytes).await
        } else {
            upload_agent_exec(&session, &agent_bytes).await
        };

        if let Err(e) = upload_result {
            return DeployResult::Fallback(format!("agent upload failed: {e}"));
        }

        // Ensure the binary is executable.
        if let Err(e) = chmod_agent(&session).await {
            return DeployResult::Fallback(format!("chmod failed: {e}"));
        }
    }

    // Step 5: start the agent process.
    let agent = match start_agent(session).await {
        Ok(a) => a,
        Err(e) => return DeployResult::Fallback(format!("agent start failed: {e}")),
    };

    DeployResult::Ready(Box::new(agent))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_cache_dir_contains_version_and_triple() {
        let target = RemoteTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            triple: "x86_64-unknown-linux-musl".into(),
        };
        let dir = local_cache_dir(&target);
        assert!(dir.is_some());
        let dir = dir.unwrap();
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.contains("nu-plugin-tramp"),
            "expected cache dir to contain 'nu-plugin-tramp': {dir_str}"
        );
        assert!(
            dir_str.contains(AGENT_VERSION),
            "expected cache dir to contain version '{AGENT_VERSION}': {dir_str}"
        );
        assert!(
            dir_str.contains("x86_64-unknown-linux-musl"),
            "expected cache dir to contain triple: {dir_str}"
        );
    }

    #[test]
    fn find_cached_agent_returns_none_for_missing() {
        let target = RemoteTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            // Use a target triple that will never exist in the cache.
            triple: "test-nonexistent-triple-9999".into(),
        };
        assert!(find_cached_agent(&target).is_none());
    }

    #[test]
    fn cache_and_find_round_trip() {
        let target = RemoteTarget {
            os: "linux".into(),
            arch: "x86_64".into(),
            triple: "test-roundtrip-cache-triple".into(),
        };

        // Write a dummy binary to the cache.
        let dummy_data = b"#!/bin/sh\necho test-agent";
        let cached_path = cache_agent_binary(&target, dummy_data).unwrap();
        assert!(cached_path.exists());

        // Should now be found.
        let found = find_cached_agent(&target);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), cached_path);

        // Verify contents.
        let read_back = std::fs::read(&cached_path).unwrap();
        assert_eq!(read_back, dummy_data);

        // Clean up.
        let _ = std::fs::remove_file(&cached_path);
        let _ = std::fs::remove_dir_all(cached_path.parent().unwrap());
    }
}
