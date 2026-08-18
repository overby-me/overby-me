//! RPC backend — communicates with a remote `tramp-agent` process via
//! MsgPack-RPC over piped stdin/stdout (through an SSH session).
//!
//! This backend replaces the shell-parsing approach used by [`SshBackend`]
//! with native RPC calls to the agent, providing:
//!
//! - **Lower latency**: no shell startup overhead per operation
//! - **Binary safety**: file contents use MsgPack's native `bin` type
//! - **Batch operations**: multiple ops in a single round-trip
//! - **Richer metadata**: native `lstat()` syscalls instead of parsing
//!   `stat --format=…` text output
//!
//! The backend is created by the deployment module after successfully
//! uploading and starting the agent on the remote host.

use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use super::rpc_client::{
    RpcClient, get_array, get_bin, get_bool, get_i64, get_str, get_u64, make_params, val_bin,
    val_bool, val_str, val_str_array, val_u64,
};
use super::{
    Backend, DirEntry, EntryKind, ExecResult, Metadata, PtyHandle, PtyReadResult, WatchInfo,
    WatchNotification,
};
use crate::errors::{TrampError, TrampResult};

// ---------------------------------------------------------------------------
// RPC Backend
// ---------------------------------------------------------------------------

/// A backend that delegates all operations to a running `tramp-agent` via
/// MsgPack-RPC.
///
/// The RPC client communicates over the agent's stdin/stdout, which are
/// piped through the SSH connection.  The `RpcBackend` owns the client and
/// the description string for display purposes.
pub struct RpcBackend<R, W> {
    /// The RPC client communicating with the remote agent.
    client: Arc<RpcClient<R, W>>,
    /// Human-readable description for `tramp connections` output.
    host: String,
}

impl<R, W> RpcBackend<R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    /// Create a new RPC backend wrapping an already-started agent.
    pub fn new(client: RpcClient<R, W>, host: String) -> Self {
        Self {
            client: Arc::new(client),
            host,
        }
    }

    /// Get a reference to the underlying RPC client.
    pub fn client(&self) -> &RpcClient<R, W> {
        &self.client
    }
}

// ---------------------------------------------------------------------------
// Backend trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl<R, W> Backend for RpcBackend<R, W>
where
    R: tokio::io::AsyncRead + Unpin + Send + Sync + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + Sync + 'static,
{
    async fn read(&self, path: &str) -> TrampResult<Bytes> {
        let params = make_params(vec![("path", val_str(path))]);
        let result = self.client.call("file.read", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("file.read: expected map result".into()))?;

        let data = get_bin(map, "data").ok_or_else(|| {
            TrampError::Internal("file.read: missing 'data' field in response".into())
        })?;

        Ok(Bytes::copy_from_slice(data))
    }

    async fn write(&self, path: &str, data: Bytes) -> TrampResult<()> {
        let params = make_params(vec![("path", val_str(path)), ("data", val_bin(&data))]);
        let _result = self.client.call("file.write", params).await?;
        Ok(())
    }

    async fn list(&self, path: &str) -> TrampResult<Vec<DirEntry>> {
        let params = make_params(vec![("path", val_str(path))]);
        let result = self.client.call("dir.list", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("dir.list: expected map result".into()))?;

        let entries_val = get_array(map, "entries").ok_or_else(|| {
            TrampError::Internal("dir.list: missing 'entries' field in response".into())
        })?;

        let mut entries = Vec::with_capacity(entries_val.len());
        for entry_val in entries_val {
            let entry_map = match entry_val.as_map() {
                Some(m) => m,
                None => continue,
            };

            // Skip entries that had errors on the remote side.
            if get_str(entry_map, "error").is_some() {
                continue;
            }

            let name = get_str(entry_map, "name").unwrap_or("").to_string();
            let kind = parse_kind(get_str(entry_map, "kind").unwrap_or("file"));
            let size = get_u64(entry_map, "size");
            let modified =
                get_u64(entry_map, "modified_ns").map(|ns| UNIX_EPOCH + Duration::from_nanos(ns));
            let permissions = get_u64(entry_map, "permissions").map(|p| p as u32);
            let nlinks = get_u64(entry_map, "nlinks");
            let inode = get_u64(entry_map, "inode");
            let owner = get_str(entry_map, "owner").map(|s| s.to_string());
            let group = get_str(entry_map, "group").map(|s| s.to_string());
            let symlink_target = get_str(entry_map, "symlink_target").map(|s| s.to_string());

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
                symlink_target,
            });
        }

        Ok(entries)
    }

    async fn stat(&self, path: &str) -> TrampResult<Metadata> {
        let params = make_params(vec![("path", val_str(path))]);
        let result = self.client.call("file.stat", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("file.stat: expected map result".into()))?;

        parse_stat_map(map)
    }

    async fn exec(&self, cmd: &str, args: &[&str]) -> TrampResult<ExecResult> {
        let mut param_entries = vec![("command", val_str(cmd))];

        // Build the args array.  We hold onto the Value so the borrow
        // into `param_entries` remains valid.
        let args_val = val_str_array(args);
        param_entries.push(("args", args_val));

        let params = make_params(param_entries);
        let result = self.client.call("process.run", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("process.run: expected map result".into()))?;

        let exit_code = get_i64(map, "exit_code").unwrap_or(-1) as i32;

        // stdout / stderr can be either binary or string in the response.
        let stdout = extract_bytes(map, "stdout");
        let stderr = extract_bytes(map, "stderr");

        Ok(ExecResult {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn delete(&self, path: &str) -> TrampResult<()> {
        let params = make_params(vec![("path", val_str(path))]);
        let _result = self.client.call("file.delete", params).await?;
        Ok(())
    }

    async fn check(&self) -> TrampResult<()> {
        self.client.ping().await
    }

    fn description(&self) -> String {
        format!("rpc:{}", self.host)
    }

    // -- Watch operations (RPC agent supports these natively) -----------------

    async fn watch_add(&self, path: &str, recursive: bool) -> TrampResult<()> {
        let params = make_params(vec![
            ("path", val_str(path)),
            ("recursive", val_bool(recursive)),
        ]);
        let _result = self.client.call("watch.add", params).await?;
        Ok(())
    }

    async fn watch_remove(&self, path: &str) -> TrampResult<()> {
        let params = make_params(vec![("path", val_str(path))]);
        let _result = self.client.call("watch.remove", params).await?;
        Ok(())
    }

    async fn watch_list(&self) -> TrampResult<Vec<WatchInfo>> {
        let params = make_params(vec![]);
        let result = self.client.call("watch.list", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("watch.list: expected map result".into()))?;

        let watches_val = get_array(map, "watches").ok_or_else(|| {
            TrampError::Internal("watch.list: missing 'watches' field in response".into())
        })?;

        let mut watches = Vec::with_capacity(watches_val.len());
        for entry_val in watches_val {
            let entry_map = match entry_val.as_map() {
                Some(m) => m,
                None => continue,
            };
            let path = get_str(entry_map, "path").unwrap_or("").to_string();
            let recursive = get_bool(entry_map, "recursive").unwrap_or(false);
            watches.push(WatchInfo { path, recursive });
        }

        Ok(watches)
    }

    async fn watch_poll(&self) -> TrampResult<Vec<WatchNotification>> {
        let notifications = self.client.drain_notifications().await;

        let mut result = Vec::with_capacity(notifications.len());
        for notif in &notifications {
            if notif.method != "fs.changed" {
                continue;
            }
            let map = match notif.params.as_map() {
                Some(m) => m,
                None => continue,
            };
            let paths = get_array(map, "paths")
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let kind = get_str(map, "kind").unwrap_or("unknown").to_string();
            result.push(WatchNotification { paths, kind });
        }

        Ok(result)
    }

    fn supports_watch(&self) -> bool {
        true
    }

    // -----------------------------------------------------------------------
    // Chunked / streaming I/O
    // -----------------------------------------------------------------------

    async fn file_size(&self, path: &str) -> TrampResult<u64> {
        let params = make_params(vec![("path", val_str(path))]);
        let result = self.client.call("file.size", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("file.size: expected map result".into()))?;

        let size = get_u64(map, "size").ok_or_else(|| {
            TrampError::Internal("file.size: missing 'size' field in response".into())
        })?;

        Ok(size)
    }

    async fn read_range(
        &self,
        path: &str,
        offset: u64,
        length: u64,
    ) -> TrampResult<(Vec<u8>, bool)> {
        let params = make_params(vec![
            ("path", val_str(path)),
            ("offset", val_u64(offset)),
            ("length", val_u64(length)),
        ]);
        let result = self.client.call("file.read_range", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("file.read_range: expected map result".into()))?;

        let data = get_bin(map, "data")
            .ok_or_else(|| {
                TrampError::Internal("file.read_range: missing 'data' field in response".into())
            })?
            .to_vec();

        let eof = get_bool(map, "eof").unwrap_or(true);

        Ok((data, eof))
    }

    async fn write_range(
        &self,
        path: &str,
        offset: u64,
        data: Bytes,
        truncate: bool,
    ) -> TrampResult<u64> {
        let mut kv = vec![
            ("path", val_str(path)),
            ("offset", val_u64(offset)),
            ("data", val_bin(&data)),
        ];
        if truncate {
            kv.push(("truncate", val_u64(1)));
        }
        let params = make_params(kv);
        let result = self.client.call("file.write_range", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("file.write_range: expected map result".into()))?;

        let written = get_u64(map, "written").unwrap_or(data.len() as u64);
        Ok(written)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    // -- PTY operations ---------------------------------------------------

    async fn pty_start(
        &self,
        program: &str,
        args: &[&str],
        rows: u16,
        cols: u16,
    ) -> TrampResult<PtyHandle> {
        let params = make_params(vec![
            ("program", val_str(program)),
            ("args", val_str_array(args)),
            ("rows", val_u64(rows as u64)),
            ("cols", val_u64(cols as u64)),
        ]);
        let result = self.client.call("process.start_pty", params).await?;

        let map = result
            .as_map()
            .ok_or_else(|| TrampError::Internal("process.start_pty: expected map result".into()))?;

        let handle = get_u64(map, "handle")
            .ok_or_else(|| TrampError::Internal("process.start_pty: missing handle".into()))?;
        let pid = get_u64(map, "pid").unwrap_or(0);

        Ok(PtyHandle { handle, pid })
    }

    async fn pty_read(&self, handle: u64) -> TrampResult<PtyReadResult> {
        let params = make_params(vec![("handle", val_u64(handle))]);
        let result = self.client.call("process.read", params).await?;

        let map = result.as_map().ok_or_else(|| {
            TrampError::Internal("process.read (pty): expected map result".into())
        })?;

        let data = get_bin(map, "stdout")
            .map(|b| b.to_vec())
            .unwrap_or_default();
        let running = get_bool(map, "running").unwrap_or(false);
        let exit_code = get_i64(map, "exit_code").map(|c| c as i32);

        Ok(PtyReadResult {
            data,
            running,
            exit_code,
        })
    }

    async fn pty_write(&self, handle: u64, data: Bytes) -> TrampResult<()> {
        let params = make_params(vec![("handle", val_u64(handle)), ("data", val_bin(&data))]);
        self.client.call("process.write", params).await?;
        Ok(())
    }

    async fn pty_resize(&self, handle: u64, rows: u16, cols: u16) -> TrampResult<()> {
        let params = make_params(vec![
            ("handle", val_u64(handle)),
            ("rows", val_u64(rows as u64)),
            ("cols", val_u64(cols as u64)),
        ]);
        self.client.call("process.resize", params).await?;
        Ok(())
    }

    async fn pty_kill(&self, handle: u64) -> TrampResult<()> {
        let params = make_params(vec![("handle", val_u64(handle))]);
        self.client.call("process.kill", params).await?;
        Ok(())
    }

    fn supports_pty(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a `kind` string from the agent into an [`EntryKind`].
fn parse_kind(kind: &str) -> EntryKind {
    match kind {
        "dir" | "directory" => EntryKind::Dir,
        "symlink" | "symbolic link" => EntryKind::Symlink,
        _ => EntryKind::File,
    }
}

/// Parse a stat metadata map from the agent into a [`Metadata`] struct.
fn parse_stat_map(map: &[(rmpv::Value, rmpv::Value)]) -> TrampResult<Metadata> {
    let kind = parse_kind(get_str(map, "kind").unwrap_or("file"));
    let size = get_u64(map, "size").unwrap_or(0);
    let modified = get_u64(map, "modified_ns").map(|ns| UNIX_EPOCH + Duration::from_nanos(ns));
    let permissions = get_u64(map, "permissions").map(|p| p as u32);
    let nlinks = get_u64(map, "nlinks");
    let inode = get_u64(map, "inode");
    let owner = get_str(map, "owner").map(|s| s.to_string());
    let group = get_str(map, "group").map(|s| s.to_string());
    let symlink_target = get_str(map, "symlink_target").map(|s| s.to_string());

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

/// Extract a byte buffer from a MsgPack map field.
///
/// The agent may encode stdout/stderr as either `bin` (binary) or `str`
/// (string) depending on the content.  This helper handles both.
fn extract_bytes(map: &[(rmpv::Value, rmpv::Value)], key: &str) -> Bytes {
    // Try binary first.
    if let Some(data) = get_bin(map, key) {
        return Bytes::copy_from_slice(data);
    }
    // Fall back to string.
    if let Some(s) = get_str(map, key) {
        return Bytes::copy_from_slice(s.as_bytes());
    }
    Bytes::new()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::rpc_client;
    use super::*;
    use rmpv::Value;

    #[test]
    fn parse_kind_variants() {
        assert_eq!(parse_kind("file"), EntryKind::File);
        assert_eq!(parse_kind("dir"), EntryKind::Dir);
        assert_eq!(parse_kind("directory"), EntryKind::Dir);
        assert_eq!(parse_kind("symlink"), EntryKind::Symlink);
        assert_eq!(parse_kind("symbolic link"), EntryKind::Symlink);
        assert_eq!(parse_kind("other"), EntryKind::File);
    }

    #[test]
    fn parse_stat_map_basic() {
        let map = vec![
            (Value::String("kind".into()), Value::String("file".into())),
            (Value::String("size".into()), Value::Integer(1024u64.into())),
            (
                Value::String("permissions".into()),
                Value::Integer(0o644u64.into()),
            ),
            (Value::String("nlinks".into()), Value::Integer(1u64.into())),
            (
                Value::String("inode".into()),
                Value::Integer(12345u64.into()),
            ),
            (Value::String("owner".into()), Value::String("root".into())),
            (Value::String("group".into()), Value::String("root".into())),
        ];

        let meta = parse_stat_map(&map).unwrap();
        assert_eq!(meta.kind, EntryKind::File);
        assert_eq!(meta.size, 1024);
        assert_eq!(meta.permissions, Some(0o644));
        assert_eq!(meta.nlinks, Some(1));
        assert_eq!(meta.inode, Some(12345));
        assert_eq!(meta.owner.as_deref(), Some("root"));
        assert_eq!(meta.group.as_deref(), Some("root"));
        assert!(meta.symlink_target.is_none());
    }

    #[test]
    fn parse_stat_map_symlink() {
        let map = vec![
            (
                Value::String("kind".into()),
                Value::String("symlink".into()),
            ),
            (Value::String("size".into()), Value::Integer(11u64.into())),
            (
                Value::String("symlink_target".into()),
                Value::String("/etc/resolv.conf".into()),
            ),
        ];

        let meta = parse_stat_map(&map).unwrap();
        assert_eq!(meta.kind, EntryKind::Symlink);
        assert_eq!(meta.symlink_target.as_deref(), Some("/etc/resolv.conf"));
    }

    #[test]
    fn parse_stat_map_with_modified_ns() {
        let ns: u64 = 1_700_000_000_000_000_000; // ~2023-11-14
        let map = vec![
            (Value::String("kind".into()), Value::String("file".into())),
            (Value::String("size".into()), Value::Integer(0u64.into())),
            (
                Value::String("modified_ns".into()),
                Value::Integer(ns.into()),
            ),
        ];

        let meta = parse_stat_map(&map).unwrap();
        assert!(meta.modified.is_some());
        let dur = meta.modified.unwrap().duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(dur.as_nanos() as u64, ns);
    }

    #[test]
    fn extract_bytes_binary() {
        let map = vec![(
            Value::String("stdout".into()),
            Value::Binary(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        )];

        let data = extract_bytes(&map, "stdout");
        assert_eq!(&data[..], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn extract_bytes_string_fallback() {
        let map = vec![(
            Value::String("stderr".into()),
            Value::String("error message".into()),
        )];

        let data = extract_bytes(&map, "stderr");
        assert_eq!(&data[..], b"error message");
    }

    #[test]
    fn extract_bytes_missing_key() {
        let map: Vec<(Value, Value)> = vec![];
        let data = extract_bytes(&map, "stdout");
        assert!(data.is_empty());
    }

    /// End-to-end test using a mock reader/writer pair that simulates the
    /// agent responding to `file.stat`.
    #[tokio::test]
    async fn rpc_backend_stat_mock() {
        // Build a mock response for file.stat.
        let resp = rpc_client::Response {
            version: "2.0".into(),
            id: 1,
            result: Some(Value::Map(vec![
                (Value::String("kind".into()), Value::String("file".into())),
                (Value::String("size".into()), Value::Integer(42u64.into())),
                (
                    Value::String("permissions".into()),
                    Value::Integer(0o755u64.into()),
                ),
                (Value::String("nlinks".into()), Value::Integer(1u64.into())),
                (
                    Value::String("inode".into()),
                    Value::Integer(9999u64.into()),
                ),
                (
                    Value::String("owner".into()),
                    Value::String("nobody".into()),
                ),
                (
                    Value::String("group".into()),
                    Value::String("nogroup".into()),
                ),
            ])),
            error: None,
        };

        let payload = rmp_serde::to_vec_named(&resp).unwrap();
        let len = payload.len() as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&payload);

        let reader = std::io::Cursor::new(frame);
        let writer = Vec::<u8>::new();

        let client = RpcClient::new(reader, writer);
        let backend = RpcBackend::new(client, "mockhost".into());

        let meta = backend.stat("/etc/hosts").await.unwrap();
        assert_eq!(meta.kind, EntryKind::File);
        assert_eq!(meta.size, 42);
        assert_eq!(meta.permissions, Some(0o755));
        assert_eq!(meta.nlinks, Some(1));
        assert_eq!(meta.inode, Some(9999));
        assert_eq!(meta.owner.as_deref(), Some("nobody"));
        assert_eq!(meta.group.as_deref(), Some("nogroup"));
    }

    /// End-to-end test using a mock reader/writer pair that simulates the
    /// agent responding to `dir.list`.
    #[tokio::test]
    async fn rpc_backend_list_mock() {
        let resp = rpc_client::Response {
            version: "2.0".into(),
            id: 1,
            result: Some(Value::Map(vec![(
                Value::String("entries".into()),
                Value::Array(vec![
                    Value::Map(vec![
                        (Value::String("name".into()), Value::String("hosts".into())),
                        (Value::String("kind".into()), Value::String("file".into())),
                        (Value::String("size".into()), Value::Integer(256u64.into())),
                    ]),
                    Value::Map(vec![
                        (Value::String("name".into()), Value::String("ssl".into())),
                        (Value::String("kind".into()), Value::String("dir".into())),
                        (Value::String("size".into()), Value::Integer(4096u64.into())),
                    ]),
                ]),
            )])),
            error: None,
        };

        let payload = rmp_serde::to_vec_named(&resp).unwrap();
        let len = payload.len() as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&payload);

        let reader = std::io::Cursor::new(frame);
        let writer = Vec::<u8>::new();

        let client = RpcClient::new(reader, writer);
        let backend = RpcBackend::new(client, "mockhost".into());

        let entries = backend.list("/etc").await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "hosts");
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[0].size, Some(256));
        assert_eq!(entries[1].name, "ssl");
        assert_eq!(entries[1].kind, EntryKind::Dir);
    }

    /// Test that RPC errors are correctly mapped to TrampError variants.
    #[tokio::test]
    async fn rpc_backend_not_found_error() {
        let resp = rpc_client::Response {
            version: "2.0".into(),
            id: 1,
            result: None,
            error: Some(rpc_client::RpcErrorData {
                code: rpc_client::error_code::NOT_FOUND,
                message: "no such file or directory: /tmp/nope".into(),
            }),
        };

        let payload = rmp_serde::to_vec_named(&resp).unwrap();
        let len = payload.len() as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&payload);

        let reader = std::io::Cursor::new(frame);
        let writer = Vec::<u8>::new();

        let client = RpcClient::new(reader, writer);
        let backend = RpcBackend::new(client, "mockhost".into());

        let err = backend.stat("/tmp/nope").await.unwrap_err();
        assert!(
            matches!(err, TrampError::NotFound(_)),
            "expected NotFound, got: {err:?}"
        );
    }

    /// Test that process.run responses are correctly parsed.
    #[tokio::test]
    async fn rpc_backend_exec_mock() {
        let resp = rpc_client::Response {
            version: "2.0".into(),
            id: 1,
            result: Some(Value::Map(vec![
                (
                    Value::String("exit_code".into()),
                    Value::Integer(0i64.into()),
                ),
                (
                    Value::String("stdout".into()),
                    Value::Binary(b"hello world\n".to_vec()),
                ),
                (Value::String("stderr".into()), Value::Binary(vec![])),
            ])),
            error: None,
        };

        let payload = rmp_serde::to_vec_named(&resp).unwrap();
        let len = payload.len() as u32;
        let mut frame = Vec::new();
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&payload);

        let reader = std::io::Cursor::new(frame);
        let writer = Vec::<u8>::new();

        let client = RpcClient::new(reader, writer);
        let backend = RpcBackend::new(client, "mockhost".into());

        let result = backend.exec("echo", &["hello", "world"]).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(&result.stdout[..], b"hello world\n");
        assert!(result.stderr.is_empty());
    }
}
