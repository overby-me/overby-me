//! Generic exec-based backend for Docker, Kubernetes, and Sudo.
//!
//! The [`ExecBackend`] implements the [`Backend`] trait by wrapping every
//! command with a configurable prefix.  For example:
//!
//! - **Docker**: `docker exec [-u user] <container> <cmd> <args…>`
//! - **Kubernetes**: `kubectl exec [-c container] <pod> -- <cmd> <args…>`
//! - **Sudo**: `sudo -n -u <user> -- <cmd> <args…>`
//!
//! All file operations (`read`, `write`, `list`, `stat`, `delete`) are
//! implemented by executing standard POSIX utilities (`cat`, `stat`, `rm`,
//! `base64`, …) through the prefix.
//!
//! The backend receives a [`CommandRunner`](super::runner::CommandRunner)
//! at construction time, which determines *where* the prefixed commands
//! are actually executed:
//!
//! - [`LocalRunner`](super::runner::LocalRunner) — commands run on the
//!   local machine (standalone `/docker:…` or `/sudo:…` paths).
//! - [`RemoteRunner`](super::runner::RemoteRunner) — commands run through
//!   a parent backend (chained paths like `/ssh:host|docker:ctr:/path`).

use async_trait::async_trait;
use bytes::Bytes;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use super::runner::CommandRunner;
use super::{Backend, DirEntry, EntryKind, ExecResult, Metadata};
use crate::errors::{TrampError, TrampResult};

// ---------------------------------------------------------------------------
// ExecBackend
// ---------------------------------------------------------------------------

/// A generic backend that delegates all operations to a [`CommandRunner`],
/// prepending a configurable command prefix to every invocation.
pub struct ExecBackend {
    /// The underlying command runner (local or remote).
    runner: Arc<dyn CommandRunner>,
    /// The full prefix prepended to every command.
    ///
    /// For Docker this is e.g. `["docker", "exec", "mycontainer"]`.
    /// The first element is the program; the rest are its leading arguments.
    prefix: Vec<String>,
    /// Human-readable target identifier (container name, pod, user, …).
    target: String,
    /// Backend kind label used in descriptions and diagnostics.
    kind: &'static str,
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl ExecBackend {
    /// Create a Docker backend.
    ///
    /// Commands are executed as:
    /// ```text
    /// docker exec [--user <user>] <container> <cmd> <args…>
    /// ```
    ///
    /// The `container` parameter is the container name or ID.
    /// The optional `user` maps to `--user`.
    pub fn docker(runner: Arc<dyn CommandRunner>, container: &str, user: Option<&str>) -> Self {
        let mut prefix = vec!["docker".into(), "exec".into()];
        if let Some(u) = user {
            prefix.push("--user".into());
            prefix.push(u.into());
        }
        prefix.push(container.into());

        Self {
            runner,
            prefix,
            target: container.into(),
            kind: "docker",
        }
    }

    /// Create a Kubernetes (`kubectl exec`) backend.
    ///
    /// Commands are executed as:
    /// ```text
    /// kubectl exec [-c <container>] <pod> -- <cmd> <args…>
    /// ```
    ///
    /// The `pod` parameter is the pod name (or `namespace/pod`).
    /// The optional `container` maps to `-c` (for multi-container pods).
    pub fn kubernetes(runner: Arc<dyn CommandRunner>, pod: &str, container: Option<&str>) -> Self {
        let mut prefix = vec!["kubectl".into(), "exec".into()];
        if let Some(c) = container {
            prefix.push("-c".into());
            prefix.push(c.into());
        }
        prefix.push(pod.into());
        prefix.push("--".into());

        Self {
            runner,
            prefix,
            target: pod.into(),
            kind: "kubernetes",
        }
    }

    /// Create a Sudo backend.
    ///
    /// Commands are executed as:
    /// ```text
    /// sudo -n -u <user> -- <cmd> <args…>
    /// ```
    ///
    /// The `-n` flag ensures non-interactive execution (no password prompt).
    /// The `user` is the target user (e.g. `root`).
    pub fn sudo(runner: Arc<dyn CommandRunner>, user: &str) -> Self {
        let prefix = vec![
            "sudo".into(),
            "-n".into(),
            "-u".into(),
            user.into(),
            "--".into(),
        ];

        Self {
            runner,
            prefix,
            target: user.into(),
            kind: "sudo",
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Run a command through the prefix.
    ///
    /// Constructs `<prefix[0]> <prefix[1..]> <program> <args…>` and passes
    /// it to the underlying runner.
    async fn run_prefixed(&self, program: &str, args: &[&str]) -> TrampResult<ExecResult> {
        let actual_program = &self.prefix[0];
        let actual_args: Vec<&str> = self.prefix[1..]
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(program))
            .chain(args.iter().copied())
            .collect();

        self.runner.run(actual_program, &actual_args).await
    }

    /// Run a shell script through the prefix.
    ///
    /// Equivalent to `run_prefixed("sh", &["-c", script])`.
    async fn run_prefixed_shell(&self, script: &str) -> TrampResult<ExecResult> {
        self.run_prefixed("sh", &["-c", script]).await
    }

    /// Inspect an [`ExecResult`] and convert non-zero exits into a typed
    /// [`TrampError`] based on the stderr contents.
    fn check_result(result: &ExecResult, path: &str) -> TrampResult<()> {
        if result.exit_code == 0 {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&result.stderr);
        let msg = stderr.trim();

        if msg.contains("No such file")
            || msg.contains("cannot access")
            || msg.contains("not found")
        {
            Err(TrampError::NotFound(path.into()))
        } else if msg.contains("Permission denied") || msg.contains("permission denied") {
            Err(TrampError::PermissionDenied(path.into()))
        } else if msg.is_empty() {
            Err(TrampError::RemoteError(format!(
                "command failed with exit code {} for path: {path}",
                result.exit_code
            )))
        } else {
            Err(TrampError::RemoteError(msg.into()))
        }
    }
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Backend for ExecBackend {
    async fn read(&self, path: &str) -> TrampResult<Bytes> {
        let escaped = shell_escape(path);
        let result = self.run_prefixed_shell(&format!("cat {escaped}")).await?;
        Self::check_result(&result, path)?;
        Ok(result.stdout)
    }

    async fn write(&self, path: &str, data: Bytes) -> TrampResult<()> {
        use base64::Engine;

        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let escaped = shell_escape(path);

        // Detect whether the target has base64(1).
        let probe = self
            .run_prefixed_shell("command -v base64 >/dev/null 2>&1 && echo yes || echo no")
            .await?;
        let has_base64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .eq_ignore_ascii_case("yes");

        if has_base64 {
            // Use a heredoc so we never hit shell argument-length limits.
            let script =
                format!("base64 -d > {escaped} <<'__TRAMP_EOF__'\n{encoded}\n__TRAMP_EOF__");
            let result = self.run_prefixed_shell(&script).await?;
            Self::check_result(&result, path)?;
        } else {
            // Fallback: printf.  Only works for text content (no NUL bytes).
            let text = String::from_utf8(data.to_vec()).map_err(|_| {
                TrampError::RemoteError(
                    "target lacks base64(1) and file contains binary data".into(),
                )
            })?;
            let escaped_content = text.replace('\\', "\\\\").replace('\'', "'\\''");
            let script = format!("printf '%s' '{escaped_content}' > {escaped}");
            let result = self.run_prefixed_shell(&script).await?;
            Self::check_result(&result, path)?;
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> TrampResult<Vec<DirEntry>> {
        let escaped = shell_escape(path.trim_end_matches('/'));

        // GNU stat format: gather all metadata in one shot (batch-stat pattern
        // inspired by emacs-tramp-rpc) to minimise round-trips.
        //   %n=filename  %F=type  %s=size  %Y=mtime  %a=octal perms
        //   %h=nlinks  %i=inode  %U=owner  %G=group
        let script = format!(
            r#"for f in {escaped}/* {escaped}/.*; do
  case "$(basename "$f")" in .|..) continue;; esac
  [ -e "$f" ] || [ -L "$f" ] || continue
  stat --format='%n\t%F\t%s\t%Y\t%a\t%h\t%i\t%U\t%G' "$f" 2>/dev/null
done"#
        );

        let result = self.run_prefixed_shell(&script).await?;

        if result.exit_code != 0 && result.stdout.is_empty() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            if stderr.contains("No such file")
                || stderr.contains("cannot access")
                || stderr.contains("not a directory")
            {
                return Err(TrampError::NotFound(path.into()));
            }
        }

        let stdout = String::from_utf8_lossy(&result.stdout);
        let mut entries = Vec::new();
        let mut symlink_paths: Vec<(usize, String)> = Vec::new();

        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(9, '\t').collect();
            if parts.len() < 5 {
                continue;
            }

            let full_name = parts[0];
            let name = Path::new(full_name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| full_name.into());

            let kind = parse_file_type(parts[1]);
            let size = parts[2].parse::<u64>().ok();
            let modified = parts[3]
                .parse::<u64>()
                .ok()
                .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
            let permissions = parts[4].parse::<u32>().ok();
            let nlinks = parts.get(5).and_then(|s| s.parse::<u64>().ok());
            let inode = parts.get(6).and_then(|s| s.parse::<u64>().ok());
            let owner = parts
                .get(7)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let group = parts
                .get(8)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            let idx = entries.len();
            if kind == EntryKind::Symlink {
                symlink_paths.push((idx, full_name.to_string()));
            }

            entries.push(DirEntry {
                name,
                kind,
                size,
                modified,
                permissions,
                nlinks,
                inode,
                owner,
                group,
                symlink_target: None,
            });
        }

        // Batch-resolve symlink targets in a single remote command.
        if !symlink_paths.is_empty() {
            let readlink_args: Vec<String> =
                symlink_paths.iter().map(|(_, p)| shell_escape(p)).collect();
            let readlink_script = format!("readlink -f {}", readlink_args.join(" "));
            if let Ok(rl_result) = self.run_prefixed_shell(&readlink_script).await
                && rl_result.exit_code == 0
            {
                let rl_stdout = String::from_utf8_lossy(&rl_result.stdout);
                for (target_line, (idx, _)) in rl_stdout.lines().zip(symlink_paths.iter()) {
                    let target = target_line.trim();
                    if !target.is_empty() {
                        entries[*idx].symlink_target = Some(target.to_string());
                    }
                }
            }
        }

        Ok(entries)
    }

    async fn stat(&self, path: &str) -> TrampResult<Metadata> {
        // Gather all metadata in one shot (batch-stat pattern from tramp-rpc).
        let escaped = shell_escape(path);
        let script = format!("stat --format='%F\\t%s\\t%Y\\t%a\\t%h\\t%i\\t%U\\t%G' {escaped}");
        let result = self.run_prefixed_shell(&script).await?;
        Self::check_result(&result, path)?;

        let stdout = String::from_utf8_lossy(&result.stdout);
        let line = stdout.trim();
        let parts: Vec<&str> = line.splitn(8, '\t').collect();
        if parts.len() < 4 {
            return Err(TrampError::RemoteError(format!(
                "unexpected stat output: {line}"
            )));
        }

        let kind = parse_file_type(parts[0]);
        let size = parts[1].parse::<u64>().unwrap_or(0);
        let modified = parts[2]
            .parse::<u64>()
            .ok()
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
        let permissions = parts[3].parse::<u32>().ok();
        let nlinks = parts.get(4).and_then(|s| s.parse::<u64>().ok());
        let inode = parts.get(5).and_then(|s| s.parse::<u64>().ok());
        let owner = parts
            .get(6)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let group = parts
            .get(7)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        // Resolve symlink target if applicable.
        let symlink_target = if kind == EntryKind::Symlink {
            let rl_script = format!("readlink -f {escaped}");
            self.run_prefixed_shell(&rl_script)
                .await
                .ok()
                .filter(|r| r.exit_code == 0)
                .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        };

        Ok(Metadata {
            kind,
            size,
            modified,
            permissions,
            nlinks,
            inode,
            owner,
            group,
            symlink_target,
        })
    }

    async fn exec(&self, cmd: &str, args: &[&str]) -> TrampResult<ExecResult> {
        self.run_prefixed(cmd, args).await
    }

    async fn delete(&self, path: &str) -> TrampResult<()> {
        let escaped = shell_escape(path);
        let result = self.run_prefixed_shell(&format!("rm -f {escaped}")).await?;
        Self::check_result(&result, path)?;
        Ok(())
    }

    async fn check(&self) -> TrampResult<()> {
        let result = self.run_prefixed("true", &[]).await?;
        if result.exit_code == 0 {
            Ok(())
        } else {
            Err(TrampError::ConnectionFailed {
                host: self.target.clone(),
                reason: format!("{} health check failed", self.kind),
            })
        }
    }

    fn description(&self) -> String {
        format!("{}:{}", self.kind, self.target)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shell-escape a string for safe embedding in `sh -c '…'` commands.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Parse GNU stat's `%F` output into an [`EntryKind`].
fn parse_file_type(type_str: &str) -> EntryKind {
    let s = type_str.to_ascii_lowercase();
    if s.contains("directory") {
        EntryKind::Dir
    } else if s.contains("symbolic link") || s.contains("symlink") {
        EntryKind::Symlink
    } else {
        EntryKind::File
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::runner::LocalRunner;

    // -- Construction --------------------------------------------------------

    #[test]
    fn docker_prefix_without_user() {
        let backend = ExecBackend::docker(Arc::new(LocalRunner), "mycontainer", None);
        assert_eq!(backend.prefix, ["docker", "exec", "mycontainer"]);
        assert_eq!(backend.target, "mycontainer");
        assert_eq!(backend.kind, "docker");
    }

    #[test]
    fn docker_prefix_with_user() {
        let backend = ExecBackend::docker(Arc::new(LocalRunner), "ctr", Some("root"));
        assert_eq!(backend.prefix, ["docker", "exec", "--user", "root", "ctr"]);
    }

    #[test]
    fn kubernetes_prefix_without_container() {
        let backend = ExecBackend::kubernetes(Arc::new(LocalRunner), "mypod", None);
        assert_eq!(backend.prefix, ["kubectl", "exec", "mypod", "--"]);
        assert_eq!(backend.target, "mypod");
        assert_eq!(backend.kind, "kubernetes");
    }

    #[test]
    fn kubernetes_prefix_with_container() {
        let backend = ExecBackend::kubernetes(Arc::new(LocalRunner), "mypod", Some("sidecar"));
        assert_eq!(
            backend.prefix,
            ["kubectl", "exec", "-c", "sidecar", "mypod", "--"]
        );
    }

    #[test]
    fn sudo_prefix() {
        let backend = ExecBackend::sudo(Arc::new(LocalRunner), "root");
        assert_eq!(backend.prefix, ["sudo", "-n", "-u", "root", "--"]);
        assert_eq!(backend.target, "root");
        assert_eq!(backend.kind, "sudo");
    }

    // -- Description ---------------------------------------------------------

    #[test]
    fn description_docker() {
        let backend = ExecBackend::docker(Arc::new(LocalRunner), "web", None);
        assert_eq!(backend.description(), "docker:web");
    }

    #[test]
    fn description_kubernetes() {
        let backend = ExecBackend::kubernetes(Arc::new(LocalRunner), "api-pod", None);
        assert_eq!(backend.description(), "kubernetes:api-pod");
    }

    #[test]
    fn description_sudo() {
        let backend = ExecBackend::sudo(Arc::new(LocalRunner), "root");
        assert_eq!(backend.description(), "sudo:root");
    }

    // -- Helpers -------------------------------------------------------------

    #[test]
    fn shell_escape_simple() {
        assert_eq!(shell_escape("/etc/config"), "'/etc/config'");
    }

    #[test]
    fn shell_escape_with_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn parse_file_type_regular() {
        assert_eq!(parse_file_type("regular file"), EntryKind::File);
        assert_eq!(parse_file_type("regular empty file"), EntryKind::File);
    }

    #[test]
    fn parse_file_type_dir() {
        assert_eq!(parse_file_type("directory"), EntryKind::Dir);
    }

    #[test]
    fn parse_file_type_symlink() {
        assert_eq!(parse_file_type("symbolic link"), EntryKind::Symlink);
    }

    // -- check_result --------------------------------------------------------

    #[test]
    fn check_result_ok() {
        let result = ExecResult {
            stdout: Bytes::new(),
            stderr: Bytes::new(),
            exit_code: 0,
        };
        assert!(ExecBackend::check_result(&result, "/test").is_ok());
    }

    #[test]
    fn check_result_not_found() {
        let result = ExecResult {
            stdout: Bytes::new(),
            stderr: Bytes::from("stat: cannot access '/missing': No such file or directory"),
            exit_code: 1,
        };
        let err = ExecBackend::check_result(&result, "/missing").unwrap_err();
        assert!(matches!(err, TrampError::NotFound(_)));
    }

    #[test]
    fn check_result_permission_denied() {
        let result = ExecResult {
            stdout: Bytes::new(),
            stderr: Bytes::from("cat: /etc/shadow: Permission denied"),
            exit_code: 1,
        };
        let err = ExecBackend::check_result(&result, "/etc/shadow").unwrap_err();
        assert!(matches!(err, TrampError::PermissionDenied(_)));
    }

    #[test]
    fn check_result_generic_error() {
        let result = ExecResult {
            stdout: Bytes::new(),
            stderr: Bytes::new(),
            exit_code: 42,
        };
        let err = ExecBackend::check_result(&result, "/path").unwrap_err();
        assert!(matches!(err, TrampError::RemoteError(_)));
    }
}
