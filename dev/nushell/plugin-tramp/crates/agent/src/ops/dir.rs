//! Directory operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method       | Description                                        |
//! |--------------|----------------------------------------------------|
//! | `dir.list`   | List entries in a directory (with full lstat info)  |
//! | `dir.create` | Create a directory (optionally with parents)        |
//! | `dir.remove` | Remove a directory (optionally recursively)         |

use std::os::unix::fs::MetadataExt;
use std::time::UNIX_EPOCH;

use rmpv::Value;
use tokio::fs;

use crate::rpc::{Response, error_code};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a required string parameter from a MsgPack map by key.
fn get_str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Response> {
    params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_str())
        })
        .ok_or_else(|| {
            Response::err(
                0, // caller must fix up the id
                error_code::INVALID_PARAMS,
                format!("missing or invalid parameter: {key}"),
            )
        })
}

/// Extract an optional boolean parameter from a MsgPack map by key.
fn get_bool_param(params: &Value, key: &str) -> Option<bool> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_bool())
    })
}

/// Resolve a uid to a username, returning `None` on failure.
fn uid_to_name(uid: u32) -> Option<String> {
    // SAFETY: getpwuid is a standard POSIX call.  We copy the name
    // immediately so the pointer isn't held across any other calls.
    unsafe {
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr((*pw).pw_name);
        Some(name.to_string_lossy().into_owned())
    }
}

/// Resolve a gid to a group name, returning `None` on failure.
fn gid_to_name(gid: u32) -> Option<String> {
    // SAFETY: getgrgid is a standard POSIX call.  We copy the name
    // immediately.
    unsafe {
        let gr = libc::getgrgid(gid);
        if gr.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr((*gr).gr_name);
        Some(name.to_string_lossy().into_owned())
    }
}

/// Map an `std::io::Error` to an appropriate RPC error response.
fn io_err_to_response(id: u64, path: &str, err: std::io::Error) -> Response {
    let (code, msg) = match err.kind() {
        std::io::ErrorKind::NotFound => (
            error_code::NOT_FOUND,
            format!("no such file or directory: {path}"),
        ),
        std::io::ErrorKind::PermissionDenied => (
            error_code::PERMISSION_DENIED,
            format!("permission denied: {path}"),
        ),
        _ => (error_code::IO_ERROR, format!("{path}: {err}")),
    };
    Response::err(id, code, msg)
}

/// Build a MsgPack value representing a single directory entry, including
/// full lstat metadata.
fn entry_to_value(name: &str, meta: &std::fs::Metadata, symlink_target: Option<String>) -> Value {
    let kind = if meta.is_dir() {
        "dir"
    } else if meta.is_symlink() {
        "symlink"
    } else {
        "file"
    };

    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64);

    let uid = meta.uid();
    let gid = meta.gid();

    let mut fields: Vec<(Value, Value)> = vec![
        (Value::String("name".into()), Value::String(name.into())),
        (Value::String("kind".into()), Value::String(kind.into())),
        (
            Value::String("size".into()),
            Value::Integer(meta.size().into()),
        ),
        (
            Value::String("permissions".into()),
            Value::Integer((meta.mode() & 0o7777).into()),
        ),
        (
            Value::String("nlinks".into()),
            Value::Integer(meta.nlink().into()),
        ),
        (
            Value::String("inode".into()),
            Value::Integer(meta.ino().into()),
        ),
        (Value::String("uid".into()), Value::Integer(uid.into())),
        (Value::String("gid".into()), Value::Integer(gid.into())),
    ];

    if let Some(ns) = modified_ns {
        fields.push((
            Value::String("modified_ns".into()),
            Value::Integer(ns.into()),
        ));
    }

    if let Some(owner) = uid_to_name(uid) {
        fields.push((Value::String("owner".into()), Value::String(owner.into())));
    }

    if let Some(group) = gid_to_name(gid) {
        fields.push((Value::String("group".into()), Value::String(group.into())));
    }

    if let Some(target) = symlink_target {
        fields.push((
            Value::String("symlink_target".into()),
            Value::String(target.into()),
        ));
    }

    Value::Map(fields)
}

// ---------------------------------------------------------------------------
// RPC method handlers
// ---------------------------------------------------------------------------

/// `dir.list` — list entries in a directory with full lstat metadata.
///
/// Params: `{ path: "<path>" }`
///
/// Result: `{ entries: [ { name, kind, size, permissions, nlinks, inode,
///   uid, gid, modified_ns, owner, group, symlink_target? }, … ] }`
pub async fn list(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let mut read_dir = match fs::read_dir(path).await {
        Ok(rd) => rd,
        Err(e) => return io_err_to_response(id, path, e),
    };

    let mut entries = Vec::new();

    loop {
        match read_dir.next_entry().await {
            Ok(Some(entry)) => {
                let name = entry.file_name().to_string_lossy().into_owned();

                // Use symlink_metadata (lstat) to avoid following symlinks.
                let meta = match entry.metadata().await {
                    Ok(m) => m,
                    Err(e) => {
                        // If we can't stat this entry, include an error
                        // marker but continue listing.
                        entries.push(Value::Map(vec![
                            (Value::String("name".into()), Value::String(name.into())),
                            (
                                Value::String("error".into()),
                                Value::String(e.to_string().into()),
                            ),
                        ]));
                        continue;
                    }
                };

                // Read symlink target if applicable.
                let symlink_target = if meta.is_symlink() {
                    fs::read_link(entry.path())
                        .await
                        .ok()
                        .map(|t| t.to_string_lossy().into_owned())
                } else {
                    None
                };

                entries.push(entry_to_value(&name, &meta, symlink_target));
            }
            Ok(None) => break,
            Err(e) => return io_err_to_response(id, path, e),
        }
    }

    Response::ok(
        id,
        Value::Map(vec![(
            Value::String("entries".into()),
            Value::Array(entries),
        )]),
    )
}

/// `dir.create` — create a directory.
///
/// Params: `{ path: "<path>" }`
///
/// Optional params:
/// - `parents`: `true` to create parent directories as needed (like `mkdir -p`).
///   Defaults to `false`.
///
/// Result: `{}` (empty map on success).
pub async fn create(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let parents = get_bool_param(params, "parents").unwrap_or(false);

    let result = if parents {
        fs::create_dir_all(path).await
    } else {
        fs::create_dir(path).await
    };

    match result {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `dir.remove` — remove a directory.
///
/// Params: `{ path: "<path>" }`
///
/// Optional params:
/// - `recursive`: `true` to remove the directory and all contents (like
///   `rm -rf`).  Defaults to `false` (only removes empty directories).
///
/// Result: `{}` (empty map on success).
pub async fn remove(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let recursive = get_bool_param(params, "recursive").unwrap_or(false);

    let result = if recursive {
        fs::remove_dir_all(path).await
    } else {
        fs::remove_dir(path).await
    };

    match result {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, path, e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;
    use tempfile::TempDir;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    #[tokio::test]
    async fn list_directory_entries() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"aaa").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"bb").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = list(1, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let entries = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("entries"))
            .unwrap()
            .1
            .as_array()
            .unwrap();

        assert_eq!(entries.len(), 3);

        // Collect names for easier assertion (order is filesystem-dependent).
        let mut names: Vec<&str> = entries
            .iter()
            .filter_map(|e| {
                e.as_map().and_then(|m| {
                    m.iter()
                        .find(|(k, _)| k.as_str() == Some("name"))
                        .and_then(|(_, v)| v.as_str())
                })
            })
            .collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt", "subdir"]);

        // Verify metadata fields are present on first entry.
        let first = entries[0].as_map().unwrap();
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("kind")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("size")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("permissions")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("nlinks")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("inode")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("uid")));
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("gid")));
    }

    #[tokio::test]
    async fn list_empty_directory() {
        let dir = TempDir::new().unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = list(2, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let entries = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("entries"))
            .unwrap()
            .1
            .as_array()
            .unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn list_nonexistent_directory() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_nodir_12345__".into()),
        )]);
        let resp = list(3, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_with_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("real.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, b"content").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = list(4, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let entries = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("entries"))
            .unwrap()
            .1
            .as_array()
            .unwrap();

        // Find the symlink entry.
        let link_entry = entries
            .iter()
            .find(|e| {
                e.as_map()
                    .and_then(|m| {
                        m.iter()
                            .find(|(k, _)| k.as_str() == Some("name"))
                            .and_then(|(_, v)| v.as_str())
                    })
                    .is_some_and(|n| n == "link.txt")
            })
            .unwrap()
            .as_map()
            .unwrap();

        let kind = link_entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("kind"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(kind, "symlink");

        let sym_target = link_entry
            .iter()
            .find(|(k, _)| k.as_str() == Some("symlink_target"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(sym_target, target.to_str().unwrap());
    }

    #[tokio::test]
    async fn create_single_directory() {
        let dir = TempDir::new().unwrap();
        let new_dir = dir.path().join("newdir");

        let params = make_params(vec![(
            "path",
            Value::String(new_dir.to_str().unwrap().into()),
        )]);
        let resp = create(5, &params).await;
        assert!(resp.error.is_none());
        assert!(new_dir.is_dir());
    }

    #[tokio::test]
    async fn create_nested_without_parents_fails() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("a").join("b").join("c");

        let params = make_params(vec![("path", Value::String(deep.to_str().unwrap().into()))]);
        let resp = create(6, &params).await;
        assert!(resp.error.is_some());
        assert!(!deep.exists());
    }

    #[tokio::test]
    async fn create_nested_with_parents() {
        let dir = TempDir::new().unwrap();
        let deep = dir.path().join("x").join("y").join("z");

        let params = make_params(vec![
            ("path", Value::String(deep.to_str().unwrap().into())),
            ("parents", Value::Boolean(true)),
        ]);
        let resp = create(7, &params).await;
        assert!(resp.error.is_none());
        assert!(deep.is_dir());
    }

    #[tokio::test]
    async fn remove_empty_directory() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("empty");
        std::fs::create_dir(&target).unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(target.to_str().unwrap().into()),
        )]);
        let resp = remove(8, &params).await;
        assert!(resp.error.is_none());
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn remove_nonempty_without_recursive_fails() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("nonempty");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("file.txt"), b"data").unwrap();

        let params = make_params(vec![(
            "path",
            Value::String(target.to_str().unwrap().into()),
        )]);
        let resp = remove(9, &params).await;
        assert!(resp.error.is_some());
        assert!(target.exists());
    }

    #[tokio::test]
    async fn remove_nonempty_recursive() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("tree");
        std::fs::create_dir_all(target.join("sub")).unwrap();
        std::fs::write(target.join("sub").join("file.txt"), b"data").unwrap();

        let params = make_params(vec![
            ("path", Value::String(target.to_str().unwrap().into())),
            ("recursive", Value::Boolean(true)),
        ]);
        let resp = remove(10, &params).await;
        assert!(resp.error.is_none());
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn remove_nonexistent_directory() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_rmdir_nonexistent__".into()),
        )]);
        let resp = remove(11, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_missing_param() {
        let params = Value::Map(vec![]);
        let resp = list(99, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn create_missing_param() {
        let params = Value::Map(vec![]);
        let resp = create(98, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }
}
