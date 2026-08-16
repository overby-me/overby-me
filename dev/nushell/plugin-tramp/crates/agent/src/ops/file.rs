//! File operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method             | Description                                      |
//! |--------------------|--------------------------------------------------|
//! | `file.stat`        | Stat a single path (lstat — doesn't follow links) |
//! | `file.stat_batch`  | Stat multiple paths in one round-trip             |
//! | `file.truename`    | Resolve symlinks to a canonical path              |
//! | `file.read`        | Read entire file contents (binary)                |
//! | `file.read_range`  | Read a byte range from a file (chunked reads)     |
//! | `file.write`       | Write data to a file (create / truncate)          |
//! | `file.write_range` | Write data at a specific offset (chunked writes)  |
//! | `file.size`        | Get just the file size (cheap, no full stat)      |
//! | `file.copy`        | Copy a file on the remote filesystem              |
//! | `file.rename`      | Rename / move a file on the remote filesystem     |
//! | `file.delete`      | Delete a file                                     |
//! | `file.set_modes`   | Set permission bits on a file                     |

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

use rmpv::Value;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

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

/// Extract a required binary parameter from a MsgPack map by key.
fn get_bin_param<'a>(params: &'a Value, key: &str) -> Result<&'a [u8], Response> {
    params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_slice())
        })
        .ok_or_else(|| {
            Response::err(
                0,
                error_code::INVALID_PARAMS,
                format!("missing or invalid binary parameter: {key}"),
            )
        })
}

/// Extract an optional u64 parameter from a MsgPack map by key.
fn get_u64_param(params: &Value, key: &str) -> Option<u64> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_u64())
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

/// Convert [`std::fs::Metadata`] into a MsgPack [`Value`] map.
fn metadata_to_value(meta: &std::fs::Metadata) -> Value {
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

    let mut entries = vec![
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
        entries.push((
            Value::String("modified_ns".into()),
            Value::Integer(ns.into()),
        ));
    }

    if let Some(owner) = uid_to_name(uid) {
        entries.push((Value::String("owner".into()), Value::String(owner.into())));
    }

    if let Some(group) = gid_to_name(gid) {
        entries.push((Value::String("group".into()), Value::String(group.into())));
    }

    // If it's a symlink, include the target.
    if meta.is_symlink() {
        // We can't resolve the target from Metadata alone — the caller
        // should use `stat` which also reads the link target.  We include
        // a placeholder here; the `stat` handler below fills it in.
    }

    Value::Map(entries)
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

// ---------------------------------------------------------------------------
// RPC method handlers
// ---------------------------------------------------------------------------

/// `file.stat` — lstat a single path.
///
/// Params: `{ path: "<path>" }`
///
/// Result: a metadata map with `kind`, `size`, `permissions`, `nlinks`,
/// `inode`, `uid`, `gid`, `modified_ns`, `owner`, `group`, and optionally
/// `symlink_target`.
pub async fn stat(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    // Use symlink_metadata (lstat) so we don't follow symlinks.
    let meta = match fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) => return io_err_to_response(id, path, e),
    };

    let mut value = metadata_to_value(&meta);

    // If it's a symlink, also read the link target.
    if meta.is_symlink()
        && let Ok(target) = fs::read_link(path).await
        && let Value::Map(ref mut entries) = value
    {
        entries.push((
            Value::String("symlink_target".into()),
            Value::String(target.to_string_lossy().into_owned().into()),
        ));
    }

    Response::ok(id, value)
}

/// `file.stat_batch` — lstat multiple paths in one round-trip.
///
/// Params: `{ paths: ["<path1>", "<path2>", ...] }`
///
/// Result: an array of `{ path: "...", stat: {...} }` or `{ path: "...", error: "..." }`.
pub async fn stat_batch(id: u64, params: &Value) -> Response {
    let paths = match params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some("paths"))
            .and_then(|(_, v)| v.as_array())
    }) {
        Some(p) => p,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: paths (expected array)",
            );
        }
    };

    let mut results = Vec::with_capacity(paths.len());

    for path_val in paths {
        let Some(path) = path_val.as_str() else {
            results.push(Value::Map(vec![
                (Value::String("path".into()), path_val.clone()),
                (
                    Value::String("error".into()),
                    Value::String("invalid path value (expected string)".into()),
                ),
            ]));
            continue;
        };

        match fs::symlink_metadata(path).await {
            Ok(meta) => {
                let mut stat_val = metadata_to_value(&meta);

                // Read symlink target if applicable.
                if meta.is_symlink()
                    && let Ok(target) = fs::read_link(path).await
                    && let Value::Map(ref mut entries) = stat_val
                {
                    entries.push((
                        Value::String("symlink_target".into()),
                        Value::String(target.to_string_lossy().into_owned().into()),
                    ));
                }

                results.push(Value::Map(vec![
                    (Value::String("path".into()), Value::String(path.into())),
                    (Value::String("stat".into()), stat_val),
                ]));
            }
            Err(e) => {
                results.push(Value::Map(vec![
                    (Value::String("path".into()), Value::String(path.into())),
                    (
                        Value::String("error".into()),
                        Value::String(e.to_string().into()),
                    ),
                ]));
            }
        }
    }

    Response::ok(id, Value::Array(results))
}

/// `file.truename` — resolve a path to its canonical form (resolving all
/// symlinks and `..` / `.` components).
///
/// Params: `{ path: "<path>" }`
///
/// Result: `{ path: "<canonical_path>" }`
pub async fn truename(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    match fs::canonicalize(path).await {
        Ok(canonical) => Response::ok(
            id,
            Value::Map(vec![(
                Value::String("path".into()),
                Value::String(canonical.to_string_lossy().into_owned().into()),
            )]),
        ),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `file.read` — read the entire contents of a file.
///
/// Params: `{ path: "<path>" }`
///
/// Result: `{ data: <binary> }` — uses MsgPack `bin` type (no base64).
pub async fn read(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    match fs::read(path).await {
        Ok(data) => Response::ok(
            id,
            Value::Map(vec![(Value::String("data".into()), Value::Binary(data))]),
        ),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `file.write` — write data to a file (create or truncate).
///
/// Params: `{ path: "<path>", data: <binary> }`
///
/// Optional params:
/// - `mode`: permission bits (u64) to set after writing.
///
/// Result: `{}` (empty map on success).
pub async fn write(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let data = match get_bin_param(params, "data") {
        Ok(d) => d,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    // Ensure parent directories exist.
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent).await
    {
        return io_err_to_response(id, path, e);
    }

    if let Err(e) = fs::write(path, data).await {
        return io_err_to_response(id, path, e);
    }

    // Optionally set permissions after writing.
    if let Some(mode) = get_u64_param(params, "mode") {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode as u32);
        if let Err(e) = fs::set_permissions(path, perms).await {
            return io_err_to_response(id, path, e);
        }
    }

    Response::ok(id, Value::Map(vec![]))
}

/// `file.copy` — copy a file within the remote filesystem.
///
/// Params: `{ src: "<source_path>", dst: "<destination_path>" }`
///
/// Result: `{}` (empty map on success).
pub async fn copy(id: u64, params: &Value) -> Response {
    let src = match get_str_param(params, "src") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let dst = match get_str_param(params, "dst") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    // Ensure destination parent directory exists.
    if let Some(parent) = Path::new(dst).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent).await
    {
        return io_err_to_response(id, dst, e);
    }

    match fs::copy(src, dst).await {
        Ok(_) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, src, e),
    }
}

/// `file.rename` — rename / move a file within the remote filesystem.
///
/// Params: `{ src: "<source_path>", dst: "<destination_path>" }`
///
/// Result: `{}` (empty map on success).
pub async fn rename(id: u64, params: &Value) -> Response {
    let src = match get_str_param(params, "src") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let dst = match get_str_param(params, "dst") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    // Ensure destination parent directory exists.
    if let Some(parent) = Path::new(dst).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent).await
    {
        return io_err_to_response(id, dst, e);
    }

    match fs::rename(src, dst).await {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, src, e),
    }
}

/// `file.delete` — delete a file (not a directory — see `dir.remove`).
///
/// Params: `{ path: "<path>" }`
///
/// Result: `{}` (empty map on success).
pub async fn delete(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    match fs::remove_file(path).await {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `file.set_modes` — set permission bits on a file.
///
/// Params: `{ path: "<path>", mode: <u64> }`
///
/// Result: `{}` (empty map on success).
pub async fn set_modes(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let mode = match get_u64_param(params, "mode") {
        Some(m) => m,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: mode (expected u64)",
            );
        }
    };

    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(mode as u32);

    match fs::set_permissions(path, perms).await {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `file.size` — get the size of a file in bytes.
///
/// Params: `{ path: "<path>" }`
///
/// Result: `{ size: <u64> }`
///
/// This is a lightweight alternative to `file.stat` when only the size is
/// needed (e.g. for planning chunked reads).
pub async fn size(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    match fs::metadata(path).await {
        Ok(meta) => Response::ok(
            id,
            Value::Map(vec![(
                Value::String("size".into()),
                Value::Integer(meta.len().into()),
            )]),
        ),
        Err(e) => io_err_to_response(id, path, e),
    }
}

/// `file.read_range` — read a byte range from a file.
///
/// Params: `{ path: "<path>", offset: <u64>, length: <u64> }`
///
/// Result: `{ data: <binary>, eof: <bool> }`
///
/// Reads up to `length` bytes starting at `offset`.  If the read reaches
/// the end of the file, `eof` is `true` and `data` may be shorter than
/// `length`.  This enables efficient chunked / streaming reads of large
/// files without loading the entire contents into memory at once.
pub async fn read_range(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let offset = match get_u64_param(params, "offset") {
        Some(o) => o,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: offset (expected u64)",
            );
        }
    };

    let length = match get_u64_param(params, "length") {
        Some(l) => l,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: length (expected u64)",
            );
        }
    };

    // Cap at 16 MB per chunk to prevent excessive memory use.
    let length = length.min(16 * 1024 * 1024);

    let mut file = match fs::File::open(path).await {
        Ok(f) => f,
        Err(e) => return io_err_to_response(id, path, e),
    };

    if let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await {
        return io_err_to_response(id, path, e);
    }

    let mut buf = vec![0u8; length as usize];
    let mut total_read = 0usize;

    loop {
        match file.read(&mut buf[total_read..]).await {
            Ok(0) => break, // EOF
            Ok(n) => {
                total_read += n;
                if total_read >= length as usize {
                    break;
                }
            }
            Err(e) => return io_err_to_response(id, path, e),
        }
    }

    buf.truncate(total_read);
    let eof = total_read < length as usize;

    Response::ok(
        id,
        Value::Map(vec![
            (Value::String("data".into()), Value::Binary(buf)),
            (Value::String("eof".into()), Value::Boolean(eof)),
        ]),
    )
}

/// `file.write_range` — write data at a specific offset in a file.
///
/// Params: `{ path: "<path>", offset: <u64>, data: <binary> }`
///
/// Optional params:
/// - `create`: bool (default true) — create the file if it doesn't exist.
/// - `truncate`: bool (default false) — truncate the file before writing
///   (useful for the first chunk of a new file).
///
/// Result: `{ written: <u64> }`
///
/// This enables chunked / streaming writes of large files.  For a new file,
/// send the first chunk with `truncate: true` and subsequent chunks with
/// increasing offsets.
pub async fn write_range(id: u64, params: &Value) -> Response {
    let path = match get_str_param(params, "path") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let offset = match get_u64_param(params, "offset") {
        Some(o) => o,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid parameter: offset (expected u64)",
            );
        }
    };

    let data = match get_bin_param(params, "data") {
        Ok(d) => d,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let create = get_u64_param(params, "create")
        .map(|v| v != 0)
        .unwrap_or(true);
    let truncate = get_u64_param(params, "truncate")
        .map(|v| v != 0)
        .unwrap_or(false);

    // Ensure parent directories exist when creating.
    if create
        && let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent).await
    {
        return io_err_to_response(id, path, e);
    }

    let mut open_opts = fs::OpenOptions::new();
    open_opts.write(true);
    if create {
        open_opts.create(true);
    }
    if truncate {
        open_opts.truncate(true);
    }

    let mut file = match open_opts.open(path).await {
        Ok(f) => f,
        Err(e) => return io_err_to_response(id, path, e),
    };

    if (offset > 0 || !truncate)
        && let Err(e) = file.seek(std::io::SeekFrom::Start(offset)).await
    {
        return io_err_to_response(id, path, e);
    }

    if let Err(e) = file.write_all(data).await {
        return io_err_to_response(id, path, e);
    }

    // Flush to ensure all data reaches disk before we return success.
    if let Err(e) = file.flush().await {
        return io_err_to_response(id, path, e);
    }

    Response::ok(
        id,
        Value::Map(vec![(
            Value::String("written".into()),
            Value::Integer((data.len() as u64).into()),
        )]),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
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
    async fn stat_existing_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, b"hello world").unwrap();

        let params = make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]);
        let resp = stat(1, &params).await;

        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let kind = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("kind"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(kind, "file");

        let size = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("size"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert_eq!(size, 11);
    }

    #[tokio::test]
    async fn stat_nonexistent_file() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_nonexistent_12345__".into()),
        )]);
        let resp = stat(2, &params).await;

        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn stat_directory() {
        let dir = TempDir::new().unwrap();
        let params = make_params(vec![(
            "path",
            Value::String(dir.path().to_str().unwrap().into()),
        )]);
        let resp = stat(3, &params).await;

        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let kind = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("kind"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(kind, "dir");
    }

    #[tokio::test]
    async fn stat_batch_mixed() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("exists.txt");
        std::fs::write(&file, b"data").unwrap();

        let params = make_params(vec![(
            "paths",
            Value::Array(vec![
                Value::String(file.to_str().unwrap().into()),
                Value::String("/tmp/__tramp_agent_nonexistent_batch__".into()),
            ]),
        )]);
        let resp = stat_batch(4, &params).await;

        assert!(resp.error.is_none());
        let results = resp.result.unwrap();
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 2);

        // First entry should have "stat".
        let first = arr[0].as_map().unwrap();
        assert!(first.iter().any(|(k, _)| k.as_str() == Some("stat")));

        // Second entry should have "error".
        let second = arr[1].as_map().unwrap();
        assert!(second.iter().any(|(k, _)| k.as_str() == Some("error")));
    }

    #[tokio::test]
    async fn size_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sized.bin");
        {
            let mut f = std::fs::File::create(&file).unwrap();
            f.write_all(&[0u8; 4096]).unwrap();
        }
        let params = make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]);
        let resp = size(1, &params).await;
        assert!(resp.error.is_none(), "expected ok: {:?}", resp);
        let sz = resp
            .result
            .as_ref()
            .unwrap()
            .as_map()
            .unwrap()
            .iter()
            .find(|(k, _)| k.as_str() == Some("size"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert_eq!(sz, 4096);
    }

    #[tokio::test]
    async fn size_nonexistent() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/tramp_agent_test_no_such_file_size".into()),
        )]);
        let resp = size(1, &params).await;
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn read_range_full() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ranged.bin");
        std::fs::write(&file, b"Hello, World!").unwrap();

        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(0.into())),
            ("length", Value::Integer(1024.into())),
        ]);
        let resp = read_range(1, &params).await;
        assert!(resp.error.is_none(), "expected ok: {:?}", resp);
        let map = resp.result.as_ref().unwrap().as_map().unwrap();
        let data = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        let eof = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("eof"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert_eq!(data, b"Hello, World!");
        assert!(eof);
    }

    #[tokio::test]
    async fn read_range_partial() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ranged2.bin");
        std::fs::write(&file, b"ABCDEFGHIJ").unwrap();

        // Read bytes 3..7 (4 bytes: "DEFG")
        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(3.into())),
            ("length", Value::Integer(4.into())),
        ]);
        let resp = read_range(1, &params).await;
        assert!(resp.error.is_none());
        let map = resp.result.as_ref().unwrap().as_map().unwrap();
        let data = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        let eof = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("eof"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert_eq!(data, b"DEFG");
        assert!(!eof); // 4 bytes requested, 4 bytes read, not at EOF
    }

    #[tokio::test]
    async fn read_range_at_eof() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ranged3.bin");
        std::fs::write(&file, b"AB").unwrap();

        // Read starting past the data
        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(10.into())),
            ("length", Value::Integer(100.into())),
        ]);
        let resp = read_range(1, &params).await;
        assert!(resp.error.is_none());
        let map = resp.result.as_ref().unwrap().as_map().unwrap();
        let data = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        let eof = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("eof"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert!(data.is_empty());
        assert!(eof);
    }

    #[tokio::test]
    async fn read_range_missing_params() {
        let params = make_params(vec![("path", Value::String("/tmp/x".into()))]);
        let resp = read_range(1, &params).await;
        assert!(resp.error.is_some()); // missing offset
    }

    #[tokio::test]
    async fn write_range_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("wr_new.bin");

        // First chunk — truncate + create
        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(0.into())),
            ("data", Value::Binary(b"Hello".to_vec())),
            ("truncate", Value::Integer(1.into())),
        ]);
        let resp = write_range(1, &params).await;
        assert!(resp.error.is_none(), "expected ok: {:?}", resp);

        // Second chunk — append at offset 5
        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(5.into())),
            ("data", Value::Binary(b", World!".to_vec())),
        ]);
        let resp = write_range(2, &params).await;
        assert!(resp.error.is_none());

        let contents = std::fs::read(&file).unwrap();
        assert_eq!(&contents, b"Hello, World!");
    }

    #[tokio::test]
    async fn write_range_overwrites_middle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("wr_mid.bin");
        std::fs::write(&file, b"AAAAAAAAAA").unwrap(); // 10 'A's

        // Overwrite bytes 3..6 with "BBB"
        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(3.into())),
            ("data", Value::Binary(b"BBB".to_vec())),
        ]);
        let resp = write_range(1, &params).await;
        assert!(resp.error.is_none());

        let contents = std::fs::read(&file).unwrap();
        assert_eq!(&contents, b"AAABBBAAAA");
    }

    #[tokio::test]
    async fn write_range_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep").join("nested").join("wr.bin");

        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("offset", Value::Integer(0.into())),
            ("data", Value::Binary(b"data".to_vec())),
            ("truncate", Value::Integer(1.into())),
        ]);
        let resp = write_range(1, &params).await;
        assert!(resp.error.is_none());
        assert_eq!(std::fs::read(&file).unwrap(), b"data");
    }

    #[tokio::test]
    async fn write_range_missing_params() {
        let params = make_params(vec![
            ("path", Value::String("/tmp/x".into())),
            ("offset", Value::Integer(0.into())),
            // missing "data"
        ]);
        let resp = write_range(1, &params).await;
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn read_write_round_trip() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("rw_test.bin");

        let data = b"\x00\x01\x02\xff binary data";
        let write_params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("data", Value::Binary(data.to_vec())),
        ]);
        let resp = write(5, &write_params).await;
        assert!(resp.error.is_none(), "write failed: {:?}", resp.error);

        let read_params = make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]);
        let resp = read(6, &read_params).await;
        assert!(resp.error.is_none(), "read failed: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let read_data = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(read_data, data);
    }

    #[tokio::test]
    async fn write_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a").join("b").join("c.txt");

        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("data", Value::Binary(b"nested".to_vec())),
        ]);
        let resp = write(7, &params).await;
        assert!(resp.error.is_none());
        assert!(file.exists());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "nested");
    }

    #[tokio::test]
    async fn write_with_mode() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("executable.sh");

        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("data", Value::Binary(b"#!/bin/sh\necho hi".to_vec())),
            ("mode", Value::Integer(0o755.into())),
        ]);
        let resp = write(8, &params).await;
        assert!(resp.error.is_none());

        let meta = std::fs::metadata(&file).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    }

    #[tokio::test]
    async fn copy_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        std::fs::write(&src, b"copy me").unwrap();

        let params = make_params(vec![
            ("src", Value::String(src.to_str().unwrap().into())),
            ("dst", Value::String(dst.to_str().unwrap().into())),
        ]);
        let resp = copy(9, &params).await;
        assert!(resp.error.is_none());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "copy me");
        // Source should still exist.
        assert!(src.exists());
    }

    #[tokio::test]
    async fn rename_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("old.txt");
        let dst = dir.path().join("new.txt");
        std::fs::write(&src, b"move me").unwrap();

        let params = make_params(vec![
            ("src", Value::String(src.to_str().unwrap().into())),
            ("dst", Value::String(dst.to_str().unwrap().into())),
        ]);
        let resp = rename(10, &params).await;
        assert!(resp.error.is_none());
        assert!(!src.exists());
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "move me");
    }

    #[tokio::test]
    async fn delete_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("to_delete.txt");
        std::fs::write(&file, b"bye").unwrap();

        let params = make_params(vec![("path", Value::String(file.to_str().unwrap().into()))]);
        let resp = delete(11, &params).await;
        assert!(resp.error.is_none());
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn delete_nonexistent() {
        let params = make_params(vec![(
            "path",
            Value::String("/tmp/__tramp_agent_del_nonexistent__".into()),
        )]);
        let resp = delete(12, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn set_modes_changes_permissions() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("chmod_test.txt");
        std::fs::write(&file, b"test").unwrap();

        let params = make_params(vec![
            ("path", Value::String(file.to_str().unwrap().into())),
            ("mode", Value::Integer(0o644.into())),
        ]);
        let resp = set_modes(13, &params).await;
        assert!(resp.error.is_none());

        let meta = std::fs::metadata(&file).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o644);
    }

    #[tokio::test]
    async fn truename_resolves_path() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("real.txt");
        std::fs::write(&file, b"").unwrap();

        // Use a path with `.` to verify canonicalization.
        let messy_path = dir.path().join(".").join("real.txt");
        let params = make_params(vec![(
            "path",
            Value::String(messy_path.to_str().unwrap().into()),
        )]);
        let resp = truename(14, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let canonical = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("path"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(canonical, file.canonicalize().unwrap().to_str().unwrap());
    }

    #[tokio::test]
    async fn stat_missing_param() {
        let params = Value::Map(vec![]);
        let resp = stat(99, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stat_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        std::fs::write(&target, b"target content").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let params = make_params(vec![("path", Value::String(link.to_str().unwrap().into()))]);
        let resp = stat(15, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let kind = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("kind"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(kind, "symlink");

        let symlink_target = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("symlink_target"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(symlink_target, target.to_str().unwrap());
    }
}
