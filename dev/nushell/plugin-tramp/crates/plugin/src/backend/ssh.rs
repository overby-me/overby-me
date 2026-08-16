//! SSH backend implementation.
//!
//! Uses the [`openssh`] crate (which shells out to the system's OpenSSH
//! binary) for session management and [`openssh_sftp_client`] for the SFTP
//! subsystem when available.
//!
//! **SFTP fast-path** (Phase 2): file read/write/delete operations use the
//! SFTP subsystem for efficient binary-safe transfer without base64 encoding
//! or shell argument limits.  If the remote host does not support the SFTP
//! subsystem, the backend falls back transparently to exec-based operations
//! (`cat`, `base64`, `rm`, etc.).
//!
//! **Exec path** (always available): `list`, `stat`, `exec`, and `check`
//! are implemented via remote command execution, which gives structured
//! output (GNU `stat --format=…`) and works on any POSIX remote.
//!
//! This gives us:
//!
//! - Full `~/.ssh/config` support
//! - SSH agent forwarding
//! - `ControlMaster` multiplexing (fast subsequent operations)
//! - Key management delegated entirely to the user's existing setup
//! - Efficient binary file transfers via SFTP
//! - Graceful fallback when SFTP is unavailable

use async_trait::async_trait;
use bytes::Bytes;
use openssh::{KnownHosts, Session, SessionBuilder};
use openssh_sftp_client::{Sftp, SftpOptions};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use super::{Backend, DirEntry, EntryKind, ExecResult, Metadata};
use crate::errors::{TrampError, TrampResult};

// ---------------------------------------------------------------------------
// SSH backend
// ---------------------------------------------------------------------------

/// An SSH backend backed by a live [`openssh::Session`] and an optional
/// [`Sftp`] channel for efficient file I/O.
///
/// The session is wrapped in `Arc` so it can be shared between the exec
/// path and the SFTP subsystem (via `Sftp::from_clonable_session`).
pub struct SshBackend {
    session: Arc<Session>,
    /// SFTP channel — `None` if the remote doesn't support the SFTP
    /// subsystem or if initialisation failed.
    sftp: Option<Sftp>,
    host: String,
}

impl SshBackend {
    /// Open a new SSH connection to `host`, optionally as `user` and/or on a
    /// non-default `port`.
    ///
    /// After the SSH session is established, the backend attempts to open an
    /// SFTP channel.  If that fails (e.g. the server has disabled the SFTP
    /// subsystem), the backend continues with exec-only mode — no error is
    /// raised.
    pub async fn connect(host: &str, user: Option<&str>, port: Option<u16>) -> TrampResult<Self> {
        let mut builder = SessionBuilder::default();
        builder.known_hosts_check(KnownHosts::Accept);

        if let Some(user) = user {
            builder.user(user.to_string());
        }
        if let Some(port) = port {
            builder.port(port);
        }

        let session = builder
            .connect(host)
            .await
            .map_err(|e| TrampError::ConnectionFailed {
                host: host.to_string(),
                reason: e.to_string(),
            })?;

        let session = Arc::new(session);

        // Try to open an SFTP channel.  This is best-effort — if it fails
        // we fall back to exec-based file I/O.
        let sftp = Sftp::from_clonable_session(session.clone(), SftpOptions::default())
            .await
            .ok();

        Ok(Self {
            session,
            sftp,
            host: host.to_string(),
        })
    }

    /// Whether this backend has an active SFTP channel.
    #[allow(dead_code)]
    pub fn has_sftp(&self) -> bool {
        self.sftp.is_some()
    }

    /// Get a reference to the underlying SSH session (wrapped in `Arc`).
    ///
    /// Used by the VFS layer to pass the session to the agent deployment
    /// module without creating a new connection.
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// Get a reference to the SFTP channel, if available.
    ///
    /// Used by the VFS layer for agent binary uploads.
    pub fn sftp(&self) -> Option<&Sftp> {
        self.sftp.as_ref()
    }

    // -----------------------------------------------------------------------
    // Exec helpers (unchanged from the original implementation)
    // -----------------------------------------------------------------------

    /// Run a command via the SSH session and return its collected output.
    async fn run(&self, program: &str, args: &[&str]) -> TrampResult<ExecResult> {
        let mut cmd = self.session.command(program);
        for arg in args {
            cmd.arg(arg);
        }
        let output = cmd
            .output()
            .await
            .map_err(|e| TrampError::from_ssh(&self.host, e))?;

        Ok(ExecResult {
            stdout: Bytes::from(output.stdout),
            stderr: Bytes::from(output.stderr),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    /// Run a shell snippet (`sh -c '<script>'`) and return its output.
    async fn run_sh(&self, script: &str) -> TrampResult<ExecResult> {
        self.run("sh", &["-c", script]).await
    }

    /// Check the exit code / stderr of a command result and turn failures
    /// into an appropriate `TrampError`.
    fn check_result(result: &ExecResult, path: &str, host: &str) -> TrampResult<()> {
        if result.exit_code == 0 {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&result.stderr);
        let msg = stderr.trim();

        if msg.contains("No such file")
            || msg.contains("cannot access")
            || msg.contains("not found")
        {
            Err(TrampError::NotFound(path.to_string()))
        } else if msg.contains("Permission denied") || msg.contains("permission denied") {
            Err(TrampError::PermissionDenied(path.to_string()))
        } else if msg.is_empty() {
            Err(TrampError::RemoteError(format!(
                "command failed with exit code {} for path: {path}",
                result.exit_code
            )))
        } else {
            Err(TrampError::from_ssh(host, msg))
        }
    }

    // -----------------------------------------------------------------------
    // SFTP helpers
    // -----------------------------------------------------------------------

    /// Classify an SFTP error into the appropriate `TrampError`.
    fn classify_sftp_error(err: openssh_sftp_client::Error, path: &str) -> TrampError {
        let msg = err.to_string();
        if msg.contains("No such file")
            || msg.contains("not found")
            || msg.contains("does not exist")
            // SFTP status code 2 = SSH_FX_NO_SUCH_FILE
            || msg.contains("SSH_FX_NO_SUCH_FILE")
        {
            TrampError::NotFound(path.to_string())
        } else if msg.contains("Permission denied")
            || msg.contains("permission denied")
            // SFTP status code 3 = SSH_FX_PERMISSION_DENIED
            || msg.contains("SSH_FX_PERMISSION_DENIED")
        {
            TrampError::PermissionDenied(path.to_string())
        } else {
            TrampError::SftpError(msg)
        }
    }

    // -----------------------------------------------------------------------
    // Exec-based file operations (fallback)
    // -----------------------------------------------------------------------

    /// Read a file via `cat` over SSH exec.
    async fn read_exec(&self, path: &str) -> TrampResult<Bytes> {
        let escaped = shell_escape(path);
        let result = self.run_sh(&format!("cat {escaped}")).await?;
        Self::check_result(&result, path, &self.host)?;
        Ok(result.stdout)
    }

    /// Write a file via base64 encoding over SSH exec.
    async fn write_exec(&self, path: &str, data: Bytes) -> TrampResult<()> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        let escaped = shell_escape(path);

        // Check if the remote has base64(1).
        let check = self
            .run_sh("command -v base64 >/dev/null 2>&1 && echo yes || echo no")
            .await?;
        let has_base64 = String::from_utf8_lossy(&check.stdout)
            .trim()
            .eq_ignore_ascii_case("yes");

        if has_base64 {
            let script =
                format!("base64 -d > {escaped} <<'__TRAMP_EOF__'\n{encoded}\n__TRAMP_EOF__");
            let result = self.run_sh(&script).await?;
            Self::check_result(&result, path, &self.host)?;
        } else {
            // Fallback: plain printf.  Only works for text content.
            let text = String::from_utf8(data.to_vec()).map_err(|_| {
                TrampError::RemoteError(
                    "remote host lacks base64(1) and file contains binary data".to_string(),
                )
            })?;
            let escaped_content = text.replace('\\', "\\\\").replace('\'', "'\\''");
            let script = format!("printf '%s' '{escaped_content}' > {escaped}");
            let result = self.run_sh(&script).await?;
            Self::check_result(&result, path, &self.host)?;
        }

        Ok(())
    }

    /// Delete a file via `rm` over SSH exec.
    async fn delete_exec(&self, path: &str) -> TrampResult<()> {
        let escaped = shell_escape(path);
        let result = self.run_sh(&format!("rm -f {escaped}")).await?;
        Self::check_result(&result, path, &self.host)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // SFTP-based file operations (fast path)
    // -----------------------------------------------------------------------

    /// Read a file via the SFTP subsystem.
    async fn read_sftp(&self, sftp: &Sftp, path: &str) -> TrampResult<Bytes> {
        let mut fs = sftp.fs();
        let data = fs
            .read(path)
            .await
            .map_err(|e| Self::classify_sftp_error(e, path))?;
        Ok(data.freeze())
    }

    /// Write a file via the SFTP subsystem.
    async fn write_sftp(&self, sftp: &Sftp, path: &str, data: Bytes) -> TrampResult<()> {
        let mut fs = sftp.fs();
        fs.write(path, &data[..])
            .await
            .map_err(|e| Self::classify_sftp_error(e, path))?;
        Ok(())
    }

    /// Delete a file via the SFTP subsystem.
    async fn delete_sftp(&self, sftp: &Sftp, path: &str) -> TrampResult<()> {
        let mut fs = sftp.fs();
        fs.remove_file(path)
            .await
            .map_err(|e| Self::classify_sftp_error(e, path))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl Backend for SshBackend {
    async fn read(&self, path: &str) -> TrampResult<Bytes> {
        // Try SFTP first for efficient binary-safe reads.
        if let Some(ref sftp) = self.sftp {
            match self.read_sftp(sftp, path).await {
                Ok(data) => return Ok(data),
                Err(TrampError::SftpError(_)) => {
                    // SFTP operation failed for a non-semantic reason
                    // (e.g. channel issue).  Fall through to exec.
                }
                Err(e) => return Err(e), // NotFound, PermissionDenied, etc.
            }
        }
        self.read_exec(path).await
    }

    async fn write(&self, path: &str, data: Bytes) -> TrampResult<()> {
        // Try SFTP first — avoids base64 encoding and shell arg limits.
        if let Some(ref sftp) = self.sftp {
            match self.write_sftp(sftp, path, data.clone()).await {
                Ok(()) => return Ok(()),
                Err(TrampError::SftpError(_)) => {
                    // Fall through to exec.
                }
                Err(e) => return Err(e),
            }
        }
        self.write_exec(path, data).await
    }

    async fn list(&self, path: &str) -> TrampResult<Vec<DirEntry>> {
        // Always use exec for listing — GNU stat gives us structured output
        // with permissions in a format consistent with our data model.
        //
        // Inspired by emacs-tramp-rpc: gather all metadata in a single
        // remote command (batch stat) to minimise round-trips.
        let escaped = shell_escape(path.trim_end_matches('/'));

        // Format: %n\t%F\t%s\t%Y\t%a\t%h\t%i\t%U\t%G
        //   %n = filename, %F = file type string, %s = size,
        //   %Y = mtime (epoch seconds), %a = octal permissions,
        //   %h = number of hard links, %i = inode number,
        //   %U = owner user name, %G = owner group name
        //
        // Symlink targets are resolved in a second pass below only for
        // entries that are symlinks, keeping the common case fast.
        let script = format!(
            r#"for f in {escaped}/* {escaped}/.*; do
  case "$(basename "$f")" in .|..) continue;; esac
  [ -e "$f" ] || [ -L "$f" ] || continue
  stat --format='%n\t%F\t%s\t%Y\t%a\t%h\t%i\t%U\t%G' "$f" 2>/dev/null
done"#
        );

        let result = self.run_sh(&script).await?;

        if result.exit_code != 0 && result.stdout.is_empty() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            if stderr.contains("No such file")
                || stderr.contains("cannot access")
                || stderr.contains("not a directory")
            {
                return Err(TrampError::NotFound(path.to_string()));
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
                .unwrap_or_else(|| full_name.to_string());

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
            if let Ok(rl_result) = self.run_sh(&readlink_script).await
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
        // Use exec for stat — GNU stat gives consistent structured output.
        // Gather all metadata in one shot (batch-stat pattern from tramp-rpc).
        let escaped = shell_escape(path);
        let script = format!("stat --format='%F\\t%s\\t%Y\\t%a\\t%h\\t%i\\t%U\\t%G' {escaped}");
        let result = self.run_sh(&script).await?;
        Self::check_result(&result, path, &self.host)?;

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
            self.run_sh(&rl_script)
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
        self.run(cmd, args).await
    }

    async fn delete(&self, path: &str) -> TrampResult<()> {
        // Try SFTP first.
        if let Some(ref sftp) = self.sftp {
            match self.delete_sftp(sftp, path).await {
                Ok(()) => return Ok(()),
                Err(TrampError::SftpError(_)) => {
                    // Fall through to exec.
                }
                Err(e) => return Err(e),
            }
        }
        self.delete_exec(path).await
    }

    async fn check(&self) -> TrampResult<()> {
        let result = self.run("true", &[]).await?;
        if result.exit_code == 0 {
            Ok(())
        } else {
            Err(TrampError::ConnectionFailed {
                host: self.host.clone(),
                reason: "health check failed: `true` returned non-zero".to_string(),
            })
        }
    }

    fn description(&self) -> String {
        if self.sftp.is_some() {
            format!("ssh+sftp:{}", self.host)
        } else {
            format!("ssh:{}", self.host)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Shell-escape a string for safe embedding in `sh -c '…'` commands.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Drop for SshBackend {
    fn drop(&mut self) {
        // `Session::close` and `Sftp::close` are async; we can't await here.
        // The `openssh` crate cleans up the ControlMaster socket when the
        // `Session` is dropped, and `Sftp` cleans up its subsystem child.
        // Dropping `sftp` first (before session) is the natural field order.
    }
}
