//! Backend trait and registry.
//!
//! Each transport (SSH, Docker, Kubernetes, sudo, …) implements the [`Backend`]
//! trait, which provides the primitive file-system and exec operations that the
//! VFS layer delegates to.
//!
//! Backends may optionally support chunked/streaming I/O via [`Backend::read_range`],
//! [`Backend::write_range`], and [`Backend::file_size`].  The default implementations
//! fall back to the regular whole-file [`Backend::read`] and [`Backend::write`].

#![allow(dead_code)]

use async_trait::async_trait;
use bytes::Bytes;
use std::time::SystemTime;

pub mod deploy;
pub mod deploy_exec;
pub mod exec;
pub mod rpc;
pub mod rpc_client;
pub mod runner;
pub mod socket;
pub mod ssh;

// ---------------------------------------------------------------------------
// Types returned by backend operations
// ---------------------------------------------------------------------------

/// The kind of a directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryKind {
    #[default]
    File,
    Dir,
    Symlink,
}

/// A single entry returned by [`Backend::list`].
#[derive(Debug, Clone, Default)]
pub struct DirEntry {
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub permissions: Option<u32>,
    /// Number of hard links.
    pub nlinks: Option<u64>,
    /// Inode number.
    pub inode: Option<u64>,
    /// Owner user name (resolved from uid when available).
    pub owner: Option<String>,
    /// Owner group name (resolved from gid when available).
    pub group: Option<String>,
    /// Symlink target path (only set when `kind == Symlink`).
    pub symlink_target: Option<String>,
}

/// Metadata for a remote path returned by [`Backend::stat`].
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub permissions: Option<u32>,
    /// Number of hard links.
    pub nlinks: Option<u64>,
    /// Inode number.
    pub inode: Option<u64>,
    /// Owner user name.
    pub owner: Option<String>,
    /// Owner group name.
    pub group: Option<String>,
    /// Symlink target path (only set when `kind == Symlink`).
    pub symlink_target: Option<String>,
}

/// Result of running a command on the remote via [`Backend::exec`].
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub stdout: Bytes,
    pub stderr: Bytes,
    pub exit_code: i32,
}

// ---------------------------------------------------------------------------
// Watch types
// ---------------------------------------------------------------------------

/// A single filesystem change notification received from a remote watch.
#[derive(Debug, Clone)]
pub struct WatchNotification {
    /// The affected filesystem paths.
    pub paths: Vec<String>,
    /// The kind of change: `"create"`, `"modify"`, `"remove"`, `"access"`, etc.
    pub kind: String,
}

/// Information about an active filesystem watch.
#[derive(Debug, Clone)]
pub struct WatchInfo {
    /// The watched filesystem path.
    pub path: String,
    /// Whether the watch is recursive (includes subdirectories).
    pub recursive: bool,
}

// ---------------------------------------------------------------------------
// PTY types
// ---------------------------------------------------------------------------

/// Handle returned by [`Backend::pty_start`] identifying a running PTY process.
#[derive(Debug, Clone)]
pub struct PtyHandle {
    /// Opaque handle ID used to reference this process in subsequent calls.
    pub handle: u64,
    /// The PID of the child process on the remote host.
    pub pid: u64,
}

/// Result of [`Backend::pty_read`].
#[derive(Debug, Clone)]
pub struct PtyReadResult {
    /// Output data read from the PTY (combined stdout+stderr via the PTY).
    pub data: Vec<u8>,
    /// Whether the process is still running.
    pub running: bool,
    /// Exit code if the process has exited, `None` while still running.
    pub exit_code: Option<i32>,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// A transport backend capable of performing remote file I/O and command
/// execution.
///
/// All operations are async.  The VFS layer is responsible for creating a
/// tokio runtime and blocking on these futures when called from the
/// synchronous Nushell plugin interface.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Read the entire contents of a remote file.
    async fn read(&self, path: &str) -> Result<Bytes, crate::errors::TrampError>;

    /// Write `data` to a remote file, creating or truncating it.
    async fn write(&self, path: &str, data: Bytes) -> Result<(), crate::errors::TrampError>;

    /// List the entries in a remote directory.
    async fn list(&self, path: &str) -> Result<Vec<DirEntry>, crate::errors::TrampError>;

    /// Get metadata for a remote path.
    async fn stat(&self, path: &str) -> Result<Metadata, crate::errors::TrampError>;

    /// Execute a command on the remote and collect its output.
    async fn exec(&self, cmd: &str, args: &[&str])
    -> Result<ExecResult, crate::errors::TrampError>;

    /// Delete a remote file (or empty directory).
    async fn delete(&self, path: &str) -> Result<(), crate::errors::TrampError>;

    /// Check whether the connection is still alive.
    ///
    /// Implementations should run a cheap no-op command (e.g. `true` or
    /// `echo ok`) and return `Ok(())` if the remote responds, or an `Err`
    /// if the connection appears dead or unresponsive.
    ///
    /// The VFS layer calls this before reusing a pooled connection; on
    /// failure it will drop the stale backend and open a fresh one.
    async fn check(&self) -> Result<(), crate::errors::TrampError>;

    /// A human-readable description of this backend connection, used for
    /// display in `tramp connections` and diagnostics.
    fn description(&self) -> String;

    // -----------------------------------------------------------------------
    // Watch operations (optional — only supported by RPC backends)
    // -----------------------------------------------------------------------

    /// Start watching a remote path for filesystem changes.
    ///
    /// When `recursive` is `true`, subdirectories are also watched.
    /// Returns `Ok(())` on success.  Backends that don't support watching
    /// return an error.
    async fn watch_add(
        &self,
        _path: &str,
        _recursive: bool,
    ) -> Result<(), crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "filesystem watching is not supported by this backend (requires RPC agent)".into(),
        ))
    }

    /// Stop watching a previously added path.
    async fn watch_remove(&self, _path: &str) -> Result<(), crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "filesystem watching is not supported by this backend (requires RPC agent)".into(),
        ))
    }

    /// List all currently active watches.
    async fn watch_list(&self) -> Result<Vec<WatchInfo>, crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "filesystem watching is not supported by this backend (requires RPC agent)".into(),
        ))
    }

    /// Drain any pending filesystem change notifications.
    ///
    /// Returns all notifications that have been buffered since the last
    /// call.  Non-blocking — returns an empty vec if nothing is pending.
    async fn watch_poll(&self) -> Result<Vec<WatchNotification>, crate::errors::TrampError> {
        Ok(vec![])
    }

    /// Whether this backend supports filesystem watching.
    fn supports_watch(&self) -> bool {
        false
    }

    // -----------------------------------------------------------------------
    // Chunked / streaming I/O (optional — only supported by RPC backends)
    // -----------------------------------------------------------------------

    /// Get the size of a remote file in bytes.
    ///
    /// This is a lightweight alternative to [`Backend::stat`] when only the
    /// size is needed, e.g. for planning chunked reads.  The default
    /// implementation falls back to `stat().size`.
    async fn file_size(&self, path: &str) -> Result<u64, crate::errors::TrampError> {
        let meta = self.stat(path).await?;
        Ok(meta.size)
    }

    /// Read a byte range from a remote file.
    ///
    /// Returns `(data, eof)` where `data` contains up to `length` bytes
    /// starting at `offset`, and `eof` is `true` when the end of the file
    /// has been reached (i.e. fewer than `length` bytes were available).
    ///
    /// The default implementation reads the entire file and slices it,
    /// which defeats the purpose of streaming.  Backends that support the
    /// RPC agent override this with a native `file.read_range` call.
    async fn read_range(
        &self,
        path: &str,
        offset: u64,
        length: u64,
    ) -> Result<(Vec<u8>, bool), crate::errors::TrampError> {
        let data = self.read(path).await?;
        let start = (offset as usize).min(data.len());
        let end = (start + length as usize).min(data.len());
        let chunk = data[start..end].to_vec();
        let eof = end >= data.len();
        Ok((chunk, eof))
    }

    /// Write data at a specific byte offset in a remote file.
    ///
    /// When `truncate` is `true` the file is truncated before writing
    /// (useful for the first chunk of a new file).  The file is created
    /// if it does not exist.
    ///
    /// Returns the number of bytes written.
    ///
    /// The default implementation ignores the offset and writes the whole
    /// file (only correct when called once with offset=0 and truncate=true).
    /// RPC backends override this with a native `file.write_range` call.
    async fn write_range(
        &self,
        path: &str,
        offset: u64,
        data: bytes::Bytes,
        truncate: bool,
    ) -> Result<u64, crate::errors::TrampError> {
        if offset != 0 || !truncate {
            return Err(crate::errors::TrampError::Internal(
                "write_range with non-zero offset requires RPC agent support".into(),
            ));
        }
        let len = data.len() as u64;
        self.write(path, data).await?;
        Ok(len)
    }

    /// Whether this backend supports efficient chunked I/O.
    ///
    /// When `true`, [`read_range`](Backend::read_range) and
    /// [`write_range`](Backend::write_range) use native agent calls
    /// instead of the whole-file fallback.
    fn supports_streaming(&self) -> bool {
        false
    }

    // -----------------------------------------------------------------------
    // PTY operations (optional — only supported by RPC backends on Unix)
    // -----------------------------------------------------------------------

    /// Start a process with a pseudo-terminal allocated.
    ///
    /// Returns a [`PtyHandle`] containing the process handle and PID.
    /// The handle can be used with [`pty_read`](Backend::pty_read),
    /// [`pty_write`](Backend::pty_write), [`pty_resize`](Backend::pty_resize),
    /// and [`pty_kill`](Backend::pty_kill).
    ///
    /// `rows` and `cols` set the initial terminal dimensions (default 24×80).
    async fn pty_start(
        &self,
        _program: &str,
        _args: &[&str],
        _rows: u16,
        _cols: u16,
    ) -> Result<PtyHandle, crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "PTY support is not available on this backend (requires RPC agent on Unix)".into(),
        ))
    }

    /// Read available output from a PTY process.
    ///
    /// Returns `(stdout_data, running, exit_code)`.  `exit_code` is `None`
    /// while the process is still running.
    async fn pty_read(&self, _handle: u64) -> Result<PtyReadResult, crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "PTY support is not available on this backend".into(),
        ))
    }

    /// Write data to a PTY process (i.e. send it to the process's stdin).
    async fn pty_write(
        &self,
        _handle: u64,
        _data: bytes::Bytes,
    ) -> Result<(), crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "PTY support is not available on this backend".into(),
        ))
    }

    /// Resize a PTY process's terminal window.
    ///
    /// Sends a `SIGWINCH` to the process so it can adapt to the new size.
    async fn pty_resize(
        &self,
        _handle: u64,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "PTY support is not available on this backend".into(),
        ))
    }

    /// Kill a PTY process by handle.
    async fn pty_kill(&self, _handle: u64) -> Result<(), crate::errors::TrampError> {
        Err(crate::errors::TrampError::Internal(
            "PTY support is not available on this backend".into(),
        ))
    }

    /// Whether this backend supports PTY (pseudo-terminal) operations.
    fn supports_pty(&self) -> bool {
        false
    }
}
