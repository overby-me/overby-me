//! Command execution abstraction.
//!
//! The [`CommandRunner`] trait provides a uniform interface for executing
//! commands either locally (via [`LocalRunner`]) or remotely through a
//! parent [`Backend`] (via [`RemoteRunner`]).
//!
//! This abstraction is the key enabler for **path chaining**: backends like
//! Docker, Kubernetes, and Sudo use a `CommandRunner` to execute their
//! wrapped commands.  When used standalone, they get a [`LocalRunner`];
//! when chained behind another hop (e.g. SSH), they get a [`RemoteRunner`]
//! that delegates through the parent backend's `exec` method.

use async_trait::async_trait;
use bytes::Bytes;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

use super::{Backend, ExecResult};
use crate::errors::{TrampError, TrampResult};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// A provider capable of executing commands and returning their output.
#[async_trait]
pub trait CommandRunner: Send + Sync {
    /// Execute `program` with `args` and collect stdout/stderr.
    async fn run(&self, program: &str, args: &[&str]) -> TrampResult<ExecResult>;

    /// Execute a shell script via `sh -c '<script>'`.
    async fn run_shell(&self, script: &str) -> TrampResult<ExecResult> {
        self.run("sh", &["-c", script]).await
    }

    /// Execute `program` with `args`, piping `stdin_data` to its standard
    /// input.
    ///
    /// Not all runners support true stdin piping.  The [`RemoteRunner`]
    /// falls back to base64-encoding the data into a shell pipeline.
    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_data: &[u8],
    ) -> TrampResult<ExecResult>;
}

// ---------------------------------------------------------------------------
// LocalRunner — execute on the local machine
// ---------------------------------------------------------------------------

/// Executes commands on the local machine via [`tokio::process::Command`].
pub struct LocalRunner;

#[async_trait]
impl CommandRunner for LocalRunner {
    async fn run(&self, program: &str, args: &[&str]) -> TrampResult<ExecResult> {
        let output = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| TrampError::Internal(format!("failed to execute `{program}`: {e}")))?;

        Ok(ExecResult {
            stdout: Bytes::from(output.stdout),
            stderr: Bytes::from(output.stderr),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_data: &[u8],
    ) -> TrampResult<ExecResult> {
        let mut child = tokio::process::Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| TrampError::Internal(format!("failed to spawn `{program}`: {e}")))?;

        // Write all stdin data before waiting for the process to finish.
        if let Some(mut stdin_handle) = child.stdin.take() {
            stdin_handle
                .write_all(stdin_data)
                .await
                .map_err(|e| TrampError::Internal(format!("failed to write stdin: {e}")))?;
            // Drop the handle to close stdin, signalling EOF to the child.
            drop(stdin_handle);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| TrampError::Internal(format!("failed to wait for `{program}`: {e}")))?;

        Ok(ExecResult {
            stdout: Bytes::from(output.stdout),
            stderr: Bytes::from(output.stderr),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

// ---------------------------------------------------------------------------
// RemoteRunner — execute through a parent backend
// ---------------------------------------------------------------------------

/// Executes commands by delegating to a parent [`Backend`]'s `exec` method.
///
/// This is used for chained paths: e.g. when a Docker backend is behind an
/// SSH hop, the Docker backend's commands are executed on the remote host
/// through the SSH session.
pub struct RemoteRunner {
    parent: Arc<dyn Backend>,
}

impl RemoteRunner {
    /// Create a new `RemoteRunner` that delegates to `parent`.
    pub fn new(parent: Arc<dyn Backend>) -> Self {
        Self { parent }
    }
}

#[async_trait]
impl CommandRunner for RemoteRunner {
    async fn run(&self, program: &str, args: &[&str]) -> TrampResult<ExecResult> {
        self.parent.exec(program, args).await
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin_data: &[u8],
    ) -> TrampResult<ExecResult> {
        // We cannot directly pipe stdin through a backend's `exec` method.
        // Instead, base64-encode the data and construct a shell pipeline
        // that decodes it and pipes into the target command.
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(stdin_data);

        // Build the target command string with proper escaping.
        let cmd_parts: Vec<String> = std::iter::once(shell_escape(program))
            .chain(args.iter().map(|a| shell_escape(a)))
            .collect();
        let cmd_str = cmd_parts.join(" ");

        // Use a heredoc to avoid shell argument length limits for large payloads.
        let script = format!(
            "base64 -d <<'__TRAMP_STDIN_EOF__' | {cmd_str}\n{encoded}\n__TRAMP_STDIN_EOF__"
        );

        self.parent.exec("sh", &["-c", &script]).await
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shell-escape a string for safe embedding in `sh -c '…'` commands.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_runner_echo() {
        let runner = LocalRunner;
        let result = runner.run("echo", &["hello"]).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    }

    #[tokio::test]
    async fn local_runner_shell() {
        let runner = LocalRunner;
        let result = runner.run_shell("echo world").await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "world");
    }

    #[tokio::test]
    async fn local_runner_with_stdin() {
        let runner = LocalRunner;
        let result = runner
            .run_with_stdin("cat", &[], b"stdin data")
            .await
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(String::from_utf8_lossy(&result.stdout), "stdin data");
    }

    #[tokio::test]
    async fn local_runner_nonexistent_command() {
        let runner = LocalRunner;
        let result = runner.run("__nonexistent_command_1234__", &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn local_runner_failing_command() {
        let runner = LocalRunner;
        let result = runner.run("false", &[]).await.unwrap();
        assert_ne!(result.exit_code, 0);
    }

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_escape_with_spaces() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }
}
