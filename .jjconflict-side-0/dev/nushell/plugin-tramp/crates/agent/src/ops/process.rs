//! Process operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method          | Description                                           |
//! |-----------------|-------------------------------------------------------|
//! | `process.run`   | Run a command synchronously, collect stdout/stderr     |
//! | `process.start` | Start a long-running process, return a handle          |
//! | `process.read`  | Read buffered stdout/stderr from a started process     |
//! | `process.write` | Write data to a started process's stdin                |
//! | `process.kill`  | Kill a started process by handle                       |

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use rmpv::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

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
                0,
                error_code::INVALID_PARAMS,
                format!("missing or invalid parameter: {key}"),
            )
        })
}

/// Extract an optional array of string parameters from a MsgPack map by key.
fn get_str_array_param<'a>(params: &'a Value, key: &str) -> Option<Vec<&'a str>> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
    })
}

/// Extract an optional binary parameter from a MsgPack map by key.
fn get_bin_param<'a>(params: &'a Value, key: &str) -> Option<&'a [u8]> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_slice())
    })
}

/// Extract a required u64 parameter from a MsgPack map by key.
fn get_u64_param(params: &Value, key: &str) -> Result<u64, Response> {
    params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_u64())
        })
        .ok_or_else(|| {
            Response::err(
                0,
                error_code::INVALID_PARAMS,
                format!("missing or invalid parameter: {key}"),
            )
        })
}

/// Extract an optional string parameter from a MsgPack map by key.
fn get_optional_str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_str())
    })
}

/// Extract an optional map of string→string environment variables from a
/// MsgPack map by key.
fn get_env_param(params: &Value, key: &str) -> Option<Vec<(String, String)>> {
    params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some(key))
            .and_then(|(_, v)| v.as_map())
            .map(|pairs| {
                pairs
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.as_str()?.to_owned())))
                    .collect()
            })
    })
}

// ---------------------------------------------------------------------------
// Process table
// ---------------------------------------------------------------------------

/// Global counter for process handles.
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

/// A managed child process with buffered I/O.
struct ManagedProcess {
    child: Child,
    /// Accumulated stdout that hasn't been read yet.
    stdout_buf: Vec<u8>,
    /// Accumulated stderr that hasn't been read yet.
    stderr_buf: Vec<u8>,
}

/// Shared process table.  Maps handle IDs to running child processes.
///
/// The table is wrapped in a `Mutex` because process reads/writes must be
/// serialised (we're dealing with streaming I/O on a single child).
pub struct ProcessTable {
    processes: Mutex<HashMap<u64, ManagedProcess>>,
}

impl ProcessTable {
    /// Create an empty process table.
    pub fn new() -> Self {
        Self {
            processes: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// RPC method handlers
// ---------------------------------------------------------------------------

/// Maximum number of bytes to read from a process's stdout/stderr in a
/// single `process.read` call.
const READ_BUF_SIZE: usize = 64 * 1024;

/// `process.run` — run a command synchronously and collect its output.
///
/// Params:
/// - `program`: the program to execute (string, required)
/// - `args`: arguments (array of strings, optional, default `[]`)
/// - `cwd`: working directory (string, optional)
/// - `env`: environment variables to set (map of string→string, optional)
/// - `stdin_data`: data to pipe to stdin (binary, optional)
///
/// Result: `{ stdout: <binary>, stderr: <binary>, exit_code: <integer> }`
pub async fn run(id: u64, params: &Value) -> Response {
    let program = match get_str_param(params, "program") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let args = get_str_array_param(params, "args").unwrap_or_default();
    let cwd = get_optional_str_param(params, "cwd");
    let env_vars = get_env_param(params, "env");
    let stdin_data = get_bin_param(params, "stdin_data");

    let has_stdin = stdin_data.is_some();

    let mut cmd = Command::new(program);
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if has_stdin {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    if let Some(vars) = &env_vars {
        for (k, v) in vars {
            cmd.env(k, v);
        }
    }

    let result = if let Some(data) = stdin_data {
        // Spawn and write to stdin before waiting.
        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(mut stdin_handle) = child.stdin.take() {
                    // Ignore write errors — the child may have already exited.
                    let _ = stdin_handle.write_all(data).await;
                    drop(stdin_handle);
                }
                child.wait_with_output().await
            }
            Err(e) => {
                return Response::err(
                    id,
                    error_code::IO_ERROR,
                    format!("failed to spawn `{program}`: {e}"),
                );
            }
        }
    } else {
        cmd.output().await
    };

    match result {
        Ok(output) => Response::ok(
            id,
            Value::Map(vec![
                (Value::String("stdout".into()), Value::Binary(output.stdout)),
                (Value::String("stderr".into()), Value::Binary(output.stderr)),
                (
                    Value::String("exit_code".into()),
                    Value::Integer(output.status.code().unwrap_or(-1).into()),
                ),
            ]),
        ),
        Err(e) => Response::err(
            id,
            error_code::IO_ERROR,
            format!("failed to execute `{program}`: {e}"),
        ),
    }
}

/// `process.start` — start a long-running process and return a handle.
///
/// Params:
/// - `program`: the program to execute (string, required)
/// - `args`: arguments (array of strings, optional, default `[]`)
/// - `cwd`: working directory (string, optional)
/// - `env`: environment variables to set (map of string→string, optional)
///
/// Result: `{ handle: <u64> }`
pub async fn start(id: u64, params: &Value, table: &ProcessTable) -> Response {
    let program = match get_str_param(params, "program") {
        Ok(p) => p,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let args = get_str_array_param(params, "args").unwrap_or_default();
    let cwd = get_optional_str_param(params, "cwd");
    let env_vars = get_env_param(params, "env");

    let mut cmd = Command::new(program);
    cmd.args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    if let Some(vars) = &env_vars {
        for (k, v) in vars {
            cmd.env(k, v);
        }
    }

    match cmd.spawn() {
        Ok(child) => {
            let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            let managed = ManagedProcess {
                child,
                stdout_buf: Vec::new(),
                stderr_buf: Vec::new(),
            };
            table.processes.lock().await.insert(handle, managed);

            Response::ok(
                id,
                Value::Map(vec![(
                    Value::String("handle".into()),
                    Value::Integer(handle.into()),
                )]),
            )
        }
        Err(e) => Response::err(
            id,
            error_code::IO_ERROR,
            format!("failed to spawn `{program}`: {e}"),
        ),
    }
}

/// `process.read` — read buffered output from a started process.
///
/// This performs a non-blocking read: it returns whatever data is currently
/// available (up to 64 KiB from each stream), plus whether the process has
/// exited.
///
/// Params:
/// - `handle`: process handle from `process.start` (u64, required)
///
/// Result: `{ stdout: <binary>, stderr: <binary>, running: <bool>,
///   exit_code: <integer|null> }`
pub async fn read(id: u64, params: &Value, table: &ProcessTable) -> Response {
    let handle = match get_u64_param(params, "handle") {
        Ok(h) => h,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let mut processes = table.processes.lock().await;

    let Some(managed) = processes.get_mut(&handle) else {
        return Response::err(
            id,
            error_code::NOT_FOUND,
            format!("no process with handle {handle}"),
        );
    };

    // Try to read available data from stdout.
    let mut stdout_chunk = vec![0u8; READ_BUF_SIZE];
    if let Some(ref mut stdout) = managed.child.stdout {
        match tokio::time::timeout(
            std::time::Duration::from_millis(10),
            stdout.read(&mut stdout_chunk),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => {
                managed.stdout_buf.extend_from_slice(&stdout_chunk[..n]);
            }
            _ => {}
        }
    }

    // Try to read available data from stderr.
    let mut stderr_chunk = vec![0u8; READ_BUF_SIZE];
    if let Some(ref mut stderr) = managed.child.stderr {
        match tokio::time::timeout(
            std::time::Duration::from_millis(10),
            stderr.read(&mut stderr_chunk),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => {
                managed.stderr_buf.extend_from_slice(&stderr_chunk[..n]);
            }
            _ => {}
        }
    }

    // Drain the accumulated buffers.
    let stdout_data = std::mem::take(&mut managed.stdout_buf);
    let stderr_data = std::mem::take(&mut managed.stderr_buf);

    // Check if the process has exited.
    let (running, exit_code) = match managed.child.try_wait() {
        Ok(Some(status)) => (false, Some(status.code().unwrap_or(-1))),
        Ok(None) => (true, None),
        Err(_) => (false, Some(-1)),
    };

    let exit_code_val = match exit_code {
        Some(code) => Value::Integer(code.into()),
        None => Value::Nil,
    };

    // If the process has exited, remove it from the table.
    if !running {
        processes.remove(&handle);
    }

    Response::ok(
        id,
        Value::Map(vec![
            (Value::String("stdout".into()), Value::Binary(stdout_data)),
            (Value::String("stderr".into()), Value::Binary(stderr_data)),
            (Value::String("running".into()), Value::Boolean(running)),
            (Value::String("exit_code".into()), exit_code_val),
        ]),
    )
}

/// `process.write` — write data to a started process's stdin.
///
/// Params:
/// - `handle`: process handle from `process.start` (u64, required)
/// - `data`: binary data to write (binary, required)
/// - `close`: if `true`, close stdin after writing (boolean, optional, default `false`)
///
/// Result: `{}` (empty map on success).
pub async fn write(id: u64, params: &Value, table: &ProcessTable) -> Response {
    let handle = match get_u64_param(params, "handle") {
        Ok(h) => h,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let data = match params.as_map().and_then(|m| {
        m.iter()
            .find(|(k, _)| k.as_str() == Some("data"))
            .and_then(|(_, v)| v.as_slice())
    }) {
        Some(d) => d,
        None => {
            return Response::err(
                id,
                error_code::INVALID_PARAMS,
                "missing or invalid binary parameter: data",
            );
        }
    };

    let close = params
        .as_map()
        .and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some("close"))
                .and_then(|(_, v)| v.as_bool())
        })
        .unwrap_or(false);

    let mut processes = table.processes.lock().await;

    let Some(managed) = processes.get_mut(&handle) else {
        return Response::err(
            id,
            error_code::NOT_FOUND,
            format!("no process with handle {handle}"),
        );
    };

    if let Some(ref mut stdin_handle) = managed.child.stdin {
        if let Err(e) = stdin_handle.write_all(data).await {
            return Response::err(
                id,
                error_code::IO_ERROR,
                format!("failed to write to process stdin: {e}"),
            );
        }
        if let Err(e) = stdin_handle.flush().await {
            return Response::err(
                id,
                error_code::IO_ERROR,
                format!("failed to flush process stdin: {e}"),
            );
        }
    } else {
        return Response::err(
            id,
            error_code::IO_ERROR,
            "process stdin is not available (already closed?)",
        );
    }

    // Close stdin if requested (signals EOF to the child).
    if close {
        managed.child.stdin.take();
    }

    Response::ok(id, Value::Map(vec![]))
}

/// `process.kill` — kill a started process.
///
/// Params:
/// - `handle`: process handle from `process.start` (u64, required)
///
/// Result: `{}` (empty map on success).
pub async fn kill(id: u64, params: &Value, table: &ProcessTable) -> Response {
    let handle = match get_u64_param(params, "handle") {
        Ok(h) => h,
        Err(mut e) => {
            e.id = id;
            return e;
        }
    };

    let mut processes = table.processes.lock().await;

    let Some(mut managed) = processes.remove(&handle) else {
        return Response::err(
            id,
            error_code::NOT_FOUND,
            format!("no process with handle {handle}"),
        );
    };

    match managed.child.kill().await {
        Ok(()) => Response::ok(id, Value::Map(vec![])),
        Err(e) => Response::err(
            id,
            error_code::IO_ERROR,
            format!("failed to kill process: {e}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    #[tokio::test]
    async fn run_simple_command() {
        let params = make_params(vec![
            ("program", Value::String("echo".into())),
            (
                "args",
                Value::Array(vec![Value::String("hello world".into())]),
            ),
        ]);
        let resp = run(1, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(stdout).trim(), "hello world");

        let exit_code = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("exit_code"))
            .unwrap()
            .1
            .as_i64()
            .unwrap();
        assert_eq!(exit_code, 0);
    }

    #[tokio::test]
    async fn run_failing_command() {
        let params = make_params(vec![("program", Value::String("false".into()))]);
        let resp = run(2, &params).await;
        assert!(resp.error.is_none()); // The RPC itself succeeds

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let exit_code = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("exit_code"))
            .unwrap()
            .1
            .as_i64()
            .unwrap();
        assert_ne!(exit_code, 0);
    }

    #[tokio::test]
    async fn run_nonexistent_command() {
        let params = make_params(vec![(
            "program",
            Value::String("__tramp_agent_no_such_command_99999__".into()),
        )]);
        let resp = run(3, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::IO_ERROR);
    }

    #[tokio::test]
    async fn run_with_stdin_data() {
        let params = make_params(vec![
            ("program", Value::String("cat".into())),
            ("stdin_data", Value::Binary(b"piped input".to_vec())),
        ]);
        let resp = run(4, &params).await;
        assert!(resp.error.is_none(), "expected ok, got: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(stdout, b"piped input");
    }

    #[tokio::test]
    async fn run_with_cwd() {
        let params = make_params(vec![
            ("program", Value::String("pwd".into())),
            ("cwd", Value::String("/tmp".into())),
        ]);
        let resp = run(5, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        // /tmp may be a symlink (e.g. to /private/tmp on macOS), so
        // we just check it contains "tmp".
        let output = String::from_utf8_lossy(stdout);
        assert!(output.contains("tmp"), "unexpected pwd output: {output}");
    }

    #[tokio::test]
    async fn run_with_env() {
        let params = make_params(vec![
            ("program", Value::String("sh".into())),
            (
                "args",
                Value::Array(vec![
                    Value::String("-c".into()),
                    Value::String("echo $TRAMP_TEST_VAR".into()),
                ]),
            ),
            (
                "env",
                Value::Map(vec![(
                    Value::String("TRAMP_TEST_VAR".into()),
                    Value::String("agent_value".into()),
                )]),
            ),
        ]);
        let resp = run(6, &params).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(stdout).trim(), "agent_value");
    }

    #[tokio::test]
    async fn run_missing_program_param() {
        let params = Value::Map(vec![]);
        let resp = run(99, &params).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn start_read_kill_lifecycle() {
        let table = ProcessTable::new();

        // Start a long-running process (sleep).
        let start_params = make_params(vec![
            ("program", Value::String("sleep".into())),
            ("args", Value::Array(vec![Value::String("60".into())])),
        ]);
        let resp = start(10, &start_params, &table).await;
        assert!(resp.error.is_none(), "start failed: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        // Read — process should be running with no output yet.
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let resp = read(11, &read_params, &table).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let running = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("running"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert!(running);

        // Kill the process.
        let kill_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let resp = kill(12, &kill_params, &table).await;
        assert!(resp.error.is_none(), "kill failed: {:?}", resp.error);

        // Reading from the killed handle should fail (removed from table).
        let resp = read(13, &read_params, &table).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn start_and_write_stdin() {
        let table = ProcessTable::new();

        // Start `cat` which reads stdin and echoes to stdout.
        let start_params = make_params(vec![("program", Value::String("cat".into()))]);
        let resp = start(20, &start_params, &table).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        // Write data and close stdin.
        let write_params = make_params(vec![
            ("handle", Value::Integer(handle.into())),
            ("data", Value::Binary(b"hello from agent\n".to_vec())),
            ("close", Value::Boolean(true)),
        ]);
        let resp = write(21, &write_params, &table).await;
        assert!(resp.error.is_none(), "write failed: {:?}", resp.error);

        // Give cat a moment to process and exit.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Read output — cat should have exited.
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let resp = read(22, &read_params, &table).await;
        assert!(resp.error.is_none(), "read failed: {:?}", resp.error);

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(stdout, b"hello from agent\n");

        let running = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("running"))
            .unwrap()
            .1
            .as_bool()
            .unwrap();
        assert!(!running);
    }

    #[tokio::test]
    async fn kill_nonexistent_handle() {
        let table = ProcessTable::new();

        let params = make_params(vec![("handle", Value::Integer(999999.into()))]);
        let resp = kill(30, &params, &table).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn write_to_nonexistent_handle() {
        let table = ProcessTable::new();

        let params = make_params(vec![
            ("handle", Value::Integer(888888.into())),
            ("data", Value::Binary(b"data".to_vec())),
        ]);
        let resp = write(31, &params, &table).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }

    #[tokio::test]
    async fn read_nonexistent_handle() {
        let table = ProcessTable::new();

        let params = make_params(vec![("handle", Value::Integer(777777.into()))]);
        let resp = read(32, &params, &table).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::NOT_FOUND);
    }
}
