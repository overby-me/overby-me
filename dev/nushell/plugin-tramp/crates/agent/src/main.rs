//! `tramp-agent` — Lightweight RPC agent for nu-plugin-tramp.
//!
//! This binary runs on the remote host and speaks a length-prefixed
//! MsgPack-RPC protocol over stdin/stdout.  It is started by the plugin
//! (piped through the SSH connection) and handles all filesystem, process,
//! and system operations natively — avoiding the overhead of spawning shell
//! commands and parsing their text output.
//!
//! ## Wire protocol
//!
//! ```text
//! ┌──────────────────┬──────────────────────────┐
//! │ 4 bytes BE u32   │  MessagePack payload      │
//! │ (payload length) │  (Request | Response | …) │
//! └──────────────────┴──────────────────────────┘
//! ```
//!
//! The agent reads [`Request`] messages from stdin, dispatches them to the
//! appropriate handler, and writes [`Response`] messages to stdout.  It also
//! sends unsolicited [`Notification`] messages (e.g. `fs.changed`) when
//! watched paths change.
//!
//! ## Shutdown
//!
//! The agent exits cleanly when:
//! - stdin reaches EOF (the SSH connection was closed)
//! - it receives SIGTERM or SIGINT

mod ops;
mod rpc;

use std::sync::Arc;

use ops::process::ProcessTable;
use ops::pty::PtyTable;
use ops::watch::WatchState;
use rpc::{Incoming, Response, error_code, read_message, write_notification, write_response};
use tokio::io::{self, AsyncRead, AsyncWrite, BufReader, BufWriter};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Method dispatch
// ---------------------------------------------------------------------------

/// Dispatch a single RPC request to the appropriate handler and return the
/// response.
async fn dispatch(
    method: &str,
    id: u64,
    params: &rmpv::Value,
    process_table: &ProcessTable,
    pty_table: &PtyTable,
    watch_state: &WatchState,
) -> Response {
    match method {
        // -- File operations --------------------------------------------------
        "file.stat" => ops::file::stat(id, params).await,
        "file.stat_batch" => ops::file::stat_batch(id, params).await,
        "file.truename" => ops::file::truename(id, params).await,
        "file.read" => ops::file::read(id, params).await,
        "file.read_range" => ops::file::read_range(id, params).await,
        "file.write" => ops::file::write(id, params).await,
        "file.write_range" => ops::file::write_range(id, params).await,
        "file.size" => ops::file::size(id, params).await,
        "file.copy" => ops::file::copy(id, params).await,
        "file.rename" => ops::file::rename(id, params).await,
        "file.delete" => ops::file::delete(id, params).await,
        "file.set_modes" => ops::file::set_modes(id, params).await,

        // -- Directory operations ---------------------------------------------
        "dir.list" => ops::dir::list(id, params).await,
        "dir.create" => ops::dir::create(id, params).await,
        "dir.remove" => ops::dir::remove(id, params).await,

        // -- Process operations -----------------------------------------------
        "process.run" => ops::process::run(id, params).await,
        "process.start" => ops::process::start(id, params, process_table).await,
        "process.read" => {
            // Route to PTY table if the handle is in the PTY range.
            let handle = params.as_map().and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("handle"))
                    .and_then(|(_, v)| v.as_u64())
            });
            if handle.is_some_and(ops::pty::is_pty_handle) {
                ops::pty::read(id, params, pty_table).await
            } else {
                ops::process::read(id, params, process_table).await
            }
        }
        "process.write" => {
            let handle = params.as_map().and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("handle"))
                    .and_then(|(_, v)| v.as_u64())
            });
            if handle.is_some_and(ops::pty::is_pty_handle) {
                ops::pty::write(id, params, pty_table).await
            } else {
                ops::process::write(id, params, process_table).await
            }
        }
        "process.kill" => {
            let handle = params.as_map().and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("handle"))
                    .and_then(|(_, v)| v.as_u64())
            });
            if handle.is_some_and(ops::pty::is_pty_handle) {
                ops::pty::kill(id, params, pty_table).await
            } else {
                ops::process::kill(id, params, process_table).await
            }
        }

        // -- PTY operations ---------------------------------------------------
        "process.start_pty" => ops::pty::start_pty(id, params, pty_table).await,
        "process.resize" => ops::pty::resize(id, params, pty_table).await,

        // -- System operations ------------------------------------------------
        "system.info" => ops::system::info(id, params).await,
        "system.getenv" => ops::system::getenv(id, params).await,
        "system.statvfs" => ops::system::statvfs(id, params).await,

        // -- Watch operations -------------------------------------------------
        "watch.add" => ops::watch::add(id, params, watch_state).await,
        "watch.remove" => ops::watch::remove(id, params, watch_state).await,
        "watch.list" => ops::watch::list(id, params, watch_state).await,

        // -- Batch operations -------------------------------------------------
        "batch" => {
            ops::batch::batch(
                id,
                params,
                Arc::new(ProcessTable::new()), // batch gets isolated table for safety
                Arc::new(WatchState::new()),   // batch gets isolated watches
            )
            .await
        }

        // -- Ping (health check) ---------------------------------------------
        "ping" => Response::ok(
            id,
            rmpv::Value::Map(vec![
                (
                    rmpv::Value::String("status".into()),
                    rmpv::Value::String("ok".into()),
                ),
                (
                    rmpv::Value::String("version".into()),
                    rmpv::Value::String(env!("CARGO_PKG_VERSION").into()),
                ),
                (
                    rmpv::Value::String("pid".into()),
                    rmpv::Value::Integer((std::process::id() as u64).into()),
                ),
            ]),
        ),

        // -- Unknown method ---------------------------------------------------
        _ => Response::err(
            id,
            error_code::METHOD_NOT_FOUND,
            format!("unknown method: {method}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Listen address parsing
// ---------------------------------------------------------------------------

/// Parsed `--listen` address.
#[derive(Debug, Clone)]
enum ListenAddr {
    /// TCP listener on `addr:port`.  Port 0 means pick a random free port.
    Tcp(std::net::SocketAddr),
    /// Unix domain socket listener at the given path.
    #[cfg(unix)]
    Unix(String),
}

/// Parse a `--listen` argument string.
///
/// Accepted formats:
///   - `tcp:<host>:<port>`   e.g. `tcp:127.0.0.1:9547` or `tcp:0.0.0.0:0`
///   - `unix:<path>`         e.g. `unix:/tmp/tramp-agent.sock`
///
/// When no scheme prefix is present, the value is treated as a TCP address
/// if it parses as `<host>:<port>`, otherwise as a Unix socket path.
fn parse_listen_addr(s: &str) -> Result<ListenAddr, String> {
    if let Some(rest) = s.strip_prefix("tcp:") {
        let addr: std::net::SocketAddr = rest
            .parse()
            .map_err(|e| format!("invalid TCP address '{rest}': {e}"))?;
        return Ok(ListenAddr::Tcp(addr));
    }

    #[cfg(unix)]
    if let Some(rest) = s.strip_prefix("unix:") {
        if rest.is_empty() {
            return Err("unix socket path cannot be empty".into());
        }
        return Ok(ListenAddr::Unix(rest.to_string()));
    }

    #[cfg(not(unix))]
    if s.starts_with("unix:") {
        return Err("Unix domain sockets are not supported on this platform".into());
    }

    // Auto-detect: try TCP first, then Unix path.
    if let Ok(addr) = s.parse::<std::net::SocketAddr>() {
        return Ok(ListenAddr::Tcp(addr));
    }

    #[cfg(unix)]
    {
        Ok(ListenAddr::Unix(s.to_string()))
    }

    #[cfg(not(unix))]
    Err(format!(
        "cannot parse listen address '{s}' — use tcp:<host>:<port>"
    ))
}

// ---------------------------------------------------------------------------
// Connection serving (transport-agnostic)
// ---------------------------------------------------------------------------

/// Serve a single RPC connection over the given reader/writer pair.
///
/// This is the core request loop extracted from `main()` so it can be
/// reused for stdin/stdout, TCP, and Unix socket transports.
///
/// `label` is a human-readable connection identifier used in log messages
/// (e.g. `"stdin"`, `"tcp:127.0.0.1:42345"`, `"unix:/tmp/agent.sock"`).
async fn serve_connection<R, W>(
    reader: R,
    writer: W,
    label: &str,
    process_table: Arc<ProcessTable>,
    pty_table: Arc<PtyTable>,
    watch_state: Arc<WatchState>,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader);
    let writer = Arc::new(Mutex::new(BufWriter::new(writer)));

    // Spawn a task to forward filesystem watch notifications to the writer.
    {
        let writer = Arc::clone(&writer);
        let ws = Arc::clone(&watch_state);
        tokio::spawn(async move {
            let Some(mut rx) = ws.take_receiver().await else {
                return;
            };
            while let Some(notification) = rx.recv().await {
                let mut w = writer.lock().await;
                if let Err(e) = write_notification(&mut *w, &notification).await {
                    eprintln!("tramp-agent: failed to send notification: {e}");
                    break;
                }
            }
        });
    }

    // Main request loop — read requests and dispatch them sequentially.
    loop {
        let incoming = match read_message(&mut reader).await {
            Ok(msg) => msg,
            Err(rpc::RpcError::ConnectionClosed) => {
                eprintln!("tramp-agent: {label} closed, shutting down connection");
                break;
            }
            Err(e) => {
                eprintln!("tramp-agent: {label} read error: {e}");
                if matches!(e, rpc::RpcError::Io(_)) {
                    break;
                }
                continue;
            }
        };

        match incoming {
            Incoming::Request(req) => {
                let response = dispatch(
                    &req.method,
                    req.id,
                    &req.params,
                    &process_table,
                    &pty_table,
                    &watch_state,
                )
                .await;

                let mut w = writer.lock().await;
                if let Err(e) = write_response(&mut *w, &response).await {
                    eprintln!("tramp-agent: {label} write error: {e}");
                    break;
                }
            }
        }
    }

    eprintln!("tramp-agent: {label} connection ended");
}

// ---------------------------------------------------------------------------
// Listener modes
// ---------------------------------------------------------------------------

/// Run as a TCP listener, accepting connections sequentially.
///
/// Each accepted connection is served until it closes, then the next
/// connection is accepted.  The agent prints the actual bound address to
/// stderr (useful when the port is 0 / ephemeral).  It also writes a
/// machine-readable `LISTEN:<addr>` line so the plugin can discover the
/// port programmatically.
async fn run_tcp_listener(addr: std::net::SocketAddr) {
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tramp-agent: failed to bind TCP {addr}: {e}");
            std::process::exit(1);
        }
    };

    let local_addr = listener
        .local_addr()
        .expect("failed to get local address from TCP listener");

    // Machine-readable line for the plugin to parse (port discovery).
    eprintln!("LISTEN:tcp:{local_addr}");
    eprintln!("tramp-agent: listening on tcp:{local_addr}");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tramp-agent: TCP accept error: {e}");
                continue;
            }
        };

        let label = format!("tcp:{peer}");
        eprintln!("tramp-agent: accepted connection from {peer}");

        let (read_half, write_half) = stream.into_split();

        let process_table = Arc::new(ProcessTable::new());
        let pty_table = Arc::new(PtyTable::new());
        let watch_state = Arc::new(WatchState::new());

        serve_connection(
            read_half,
            write_half,
            &label,
            process_table,
            pty_table,
            watch_state,
        )
        .await;

        eprintln!("tramp-agent: {label} disconnected, waiting for next connection");
    }
}

/// Run as a Unix domain socket listener, accepting connections sequentially.
#[cfg(unix)]
async fn run_unix_listener(path: &str) {
    // Remove stale socket file if it exists.
    let _ = std::fs::remove_file(path);

    let listener = match tokio::net::UnixListener::bind(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("tramp-agent: failed to bind Unix socket {path}: {e}");
            std::process::exit(1);
        }
    };

    // Machine-readable line for the plugin to parse.
    eprintln!("LISTEN:unix:{path}");
    eprintln!("tramp-agent: listening on unix:{path}");

    // Install a shutdown handler to clean up the socket file.
    let socket_path = path.to_string();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = std::fs::remove_file(&socket_path);
        std::process::exit(0);
    });

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("tramp-agent: Unix accept error: {e}");
                continue;
            }
        };

        let label = format!("unix:{path}");
        eprintln!("tramp-agent: accepted Unix connection");

        let (read_half, write_half) = stream.into_split();

        let process_table = Arc::new(ProcessTable::new());
        let pty_table = Arc::new(PtyTable::new());
        let watch_state = Arc::new(WatchState::new());

        serve_connection(
            read_half,
            write_half,
            &label,
            process_table,
            pty_table,
            watch_state,
        )
        .await;

        eprintln!("tramp-agent: Unix client disconnected, waiting for next connection");
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    // Handle --version flag before entering the RPC loop.
    // This is used by the deployment module to check the remote agent's
    // version and trigger re-deployment on mismatch.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("tramp-agent {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Parse --listen flag for TCP/Unix socket transport.
    let listen_addr = args
        .windows(2)
        .find(|w| w[0] == "--listen")
        .map(|w| w[1].clone())
        .or_else(|| {
            args.iter()
                .find(|a| a.starts_with("--listen="))
                .map(|a| a.strip_prefix("--listen=").unwrap().to_string())
        });

    // Log to stderr so it doesn't interfere with the MsgPack protocol on
    // stdout.  In production the plugin captures stderr for diagnostics.
    eprintln!(
        "tramp-agent v{} starting (pid {})",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    );

    // If --listen was provided, run in listener mode instead of stdin/stdout.
    if let Some(addr_str) = listen_addr {
        let addr = match parse_listen_addr(&addr_str) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("tramp-agent: invalid --listen address: {e}");
                std::process::exit(1);
            }
        };

        match addr {
            ListenAddr::Tcp(sock_addr) => {
                run_tcp_listener(sock_addr).await;
            }
            #[cfg(unix)]
            ListenAddr::Unix(path) => {
                run_unix_listener(&path).await;
            }
        }
        return;
    }

    // Default mode: serve a single connection over stdin/stdout.
    let process_table = Arc::new(ProcessTable::new());
    let pty_table = Arc::new(PtyTable::new());
    let watch_state = Arc::new(WatchState::new());

    let stdin = io::stdin();
    let stdout = io::stdout();

    serve_connection(
        stdin,
        stdout,
        "stdin",
        process_table,
        pty_table,
        watch_state,
    )
    .await;

    eprintln!("tramp-agent: exiting");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn empty_params() -> Value {
        Value::Map(vec![])
    }

    #[tokio::test]
    async fn dispatch_ping() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let resp = dispatch("ping", 1, &empty_params(), &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "ping should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        let status = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("status"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(status, "ok");

        let version = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("version"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn dispatch_unknown_method() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let resp = dispatch("nonexistent.method", 42, &empty_params(), &pt, &pty, &ws).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, error_code::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn dispatch_file_stat() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let params = Value::Map(vec![(
            Value::String("path".into()),
            Value::String("/".into()),
        )]);
        let resp = dispatch("file.stat", 2, &params, &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "stat / should succeed: {:?}",
            resp.error
        );

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
    async fn dispatch_system_info() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let resp = dispatch("system.info", 3, &empty_params(), &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "system.info should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();

        // Should have os field.
        let os = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("os"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert!(!os.is_empty());
    }

    #[tokio::test]
    async fn dispatch_dir_list() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let params = Value::Map(vec![(
            Value::String("path".into()),
            Value::String("/tmp".into()),
        )]);
        let resp = dispatch("dir.list", 4, &params, &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "dir.list /tmp should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        assert!(
            map.iter().any(|(k, _)| k.as_str() == Some("entries")),
            "response should have entries field"
        );
    }

    #[tokio::test]
    async fn dispatch_process_run() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let params = Value::Map(vec![
            (
                Value::String("program".into()),
                Value::String("echo".into()),
            ),
            (
                Value::String("args".into()),
                Value::Array(vec![Value::String("dispatch_test".into())]),
            ),
        ]);
        let resp = dispatch("process.run", 5, &params, &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "process.run should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let stdout = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(stdout).trim(), "dispatch_test");
    }

    #[tokio::test]
    async fn dispatch_system_getenv() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let params = Value::Map(vec![(
            Value::String("name".into()),
            Value::String("HOME".into()),
        )]);
        let resp = dispatch("system.getenv", 6, &params, &pt, &pty, &ws).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let value = &map
            .iter()
            .find(|(k, _)| k.as_str() == Some("value"))
            .unwrap()
            .1;
        assert!(!value.is_nil(), "HOME should be set");
    }

    #[tokio::test]
    async fn dispatch_system_statvfs() {
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let params = Value::Map(vec![(
            Value::String("path".into()),
            Value::String("/".into()),
        )]);
        let resp = dispatch("system.statvfs", 7, &params, &pt, &pty, &ws).await;
        assert!(
            resp.error.is_none(),
            "statvfs / should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let total = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("total_bytes"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert!(total > 0);
    }

    /// Verify that a full request → response round-trip works through the
    /// wire format (serialize request, write to buffer, read back, dispatch,
    /// write response, read response).
    #[tokio::test]
    async fn full_wire_round_trip() {
        use rpc::{Incoming, Request, read_message, write_response};

        // Build a request.
        let req = Request::new(100, "ping", Value::Map(vec![]));

        // Manually construct the length-prefixed wire bytes (simulating
        // what the client side would send).
        let mut wire = Vec::new();
        let payload = rmp_serde::to_vec_named(&req).unwrap();
        let len = payload.len() as u32;
        wire.extend_from_slice(&len.to_be_bytes());
        wire.extend_from_slice(&payload);

        // Read the request back (simulating the agent's read path).
        let mut cursor = std::io::Cursor::new(&wire);
        let incoming = read_message(&mut cursor).await.unwrap();

        let Incoming::Request(parsed_req) = incoming;
        assert_eq!(parsed_req.id, 100);
        assert_eq!(parsed_req.method, "ping");

        // Dispatch it.
        let pt = ProcessTable::new();
        let pty = PtyTable::new();
        let ws = WatchState::new();
        let resp = dispatch(
            &parsed_req.method,
            parsed_req.id,
            &parsed_req.params,
            &pt,
            &pty,
            &ws,
        )
        .await;

        assert!(resp.error.is_none());
        assert_eq!(resp.id, 100);

        // Write the response to a buffer.
        let mut resp_wire = Vec::new();
        write_response(&mut resp_wire, &resp).await.unwrap();

        // Verify we can decode the response payload.
        let resp_payload = &resp_wire[4..];
        let value: Value = rmp_serde::from_slice(resp_payload).unwrap();
        let map = value.as_map().unwrap();

        let id = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("id"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert_eq!(id, 100);
    }

    // -----------------------------------------------------------------------
    // parse_listen_addr tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_listen_addr_tcp_explicit() {
        let addr = parse_listen_addr("tcp:127.0.0.1:9547").unwrap();
        match addr {
            ListenAddr::Tcp(a) => {
                assert_eq!(a.ip(), std::net::Ipv4Addr::new(127, 0, 0, 1));
                assert_eq!(a.port(), 9547);
            }
            #[cfg(unix)]
            _ => panic!("expected Tcp"),
        }
    }

    #[test]
    fn parse_listen_addr_tcp_all_interfaces() {
        let addr = parse_listen_addr("tcp:0.0.0.0:0").unwrap();
        match addr {
            ListenAddr::Tcp(a) => {
                assert_eq!(a.port(), 0);
            }
            #[cfg(unix)]
            _ => panic!("expected Tcp"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn parse_listen_addr_unix_explicit() {
        let addr = parse_listen_addr("unix:/tmp/tramp-agent.sock").unwrap();
        match addr {
            ListenAddr::Unix(path) => {
                assert_eq!(path, "/tmp/tramp-agent.sock");
            }
            _ => panic!("expected Unix"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn parse_listen_addr_unix_empty_rejected() {
        assert!(parse_listen_addr("unix:").is_err());
    }

    #[test]
    fn parse_listen_addr_auto_tcp() {
        let addr = parse_listen_addr("127.0.0.1:8080").unwrap();
        match addr {
            ListenAddr::Tcp(a) => {
                assert_eq!(a.port(), 8080);
            }
            #[cfg(unix)]
            _ => panic!("expected Tcp"),
        }
    }

    #[test]
    fn parse_listen_addr_invalid() {
        // "not-valid" is not a valid TCP address and on unix it would
        // be treated as a Unix path, so this test is platform-specific.
        #[cfg(not(unix))]
        assert!(parse_listen_addr("not-valid").is_err());
    }

    // -----------------------------------------------------------------------
    // TCP serve_connection integration test
    // -----------------------------------------------------------------------

    /// Verify that `serve_connection` works over a real TCP stream by
    /// starting a listener on an ephemeral port, connecting a client,
    /// sending a `ping` request, and verifying the response.
    #[tokio::test]
    async fn serve_connection_over_tcp() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Bind to an ephemeral port on localhost.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind TCP listener");
        let local_addr = listener.local_addr().unwrap();

        // Spawn the server side in a background task.
        let server_handle = tokio::spawn(async move {
            let (stream, _peer) = listener.accept().await.expect("accept failed");
            let (read_half, write_half) = stream.into_split();

            let pt = Arc::new(ProcessTable::new());
            let pty = Arc::new(PtyTable::new());
            let ws = Arc::new(WatchState::new());
            serve_connection(read_half, write_half, "test-tcp", pt, pty, ws).await;
        });

        // Client side: connect and send a ping request.
        let mut stream = tokio::net::TcpStream::connect(local_addr)
            .await
            .expect("connect failed");

        // Build a ping request.
        let req = rpc::Request::new(1, "ping", Value::Map(vec![]));
        let payload = rmp_serde::to_vec_named(&req).unwrap();
        let len = payload.len() as u32;

        // Send the request (length-prefixed).
        stream.write_all(&len.to_be_bytes()).await.unwrap();
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();

        // Read the response.
        let mut resp_len_buf = [0u8; 4];
        stream.read_exact(&mut resp_len_buf).await.unwrap();
        let resp_len = u32::from_be_bytes(resp_len_buf) as usize;

        let mut resp_buf = vec![0u8; resp_len];
        stream.read_exact(&mut resp_buf).await.unwrap();

        let resp_value: Value = rmp_serde::from_slice(&resp_buf).unwrap();
        let map = resp_value.as_map().unwrap();

        // Verify the response has id=1 and a result with status=ok.
        let id = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("id"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();
        assert_eq!(id, 1);

        let result = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("result"))
            .unwrap()
            .1
            .as_map()
            .unwrap();

        let status = result
            .iter()
            .find(|(k, _)| k.as_str() == Some("status"))
            .unwrap()
            .1
            .as_str()
            .unwrap();
        assert_eq!(status, "ok");

        // Close the connection — this signals EOF which will end
        // serve_connection.
        drop(stream);

        // Wait for the server task to finish.
        tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
            .await
            .expect("server task timed out")
            .expect("server task panicked");
    }

    /// Verify that `serve_connection` handles multiple sequential requests
    /// over the same TCP connection.
    #[tokio::test]
    async fn serve_connection_multiple_requests() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local_addr = listener.local_addr().unwrap();

        let server_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, w) = stream.into_split();
            let pt = Arc::new(ProcessTable::new());
            let pty = Arc::new(PtyTable::new());
            let ws = Arc::new(WatchState::new());
            serve_connection(r, w, "test-multi", pt, pty, ws).await;
        });

        let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();

        // Helper: send a request and read the response id.
        async fn round_trip(stream: &mut tokio::net::TcpStream, id: u64, method: &str) -> u64 {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let req = rpc::Request::new(id, method, Value::Map(vec![]));
            let payload = rmp_serde::to_vec_named(&req).unwrap();
            let len = payload.len() as u32;
            stream.write_all(&len.to_be_bytes()).await.unwrap();
            stream.write_all(&payload).await.unwrap();
            stream.flush().await.unwrap();

            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await.unwrap();
            let resp_len = u32::from_be_bytes(len_buf) as usize;

            let mut resp_buf = vec![0u8; resp_len];
            stream.read_exact(&mut resp_buf).await.unwrap();

            let value: Value = rmp_serde::from_slice(&resp_buf).unwrap();
            let map = value.as_map().unwrap();
            map.iter()
                .find(|(k, _)| k.as_str() == Some("id"))
                .unwrap()
                .1
                .as_u64()
                .unwrap()
        }

        // Send three requests sequentially on the same connection.
        assert_eq!(round_trip(&mut stream, 1, "ping").await, 1);
        assert_eq!(round_trip(&mut stream, 2, "ping").await, 2);
        assert_eq!(round_trip(&mut stream, 3, "system.info").await, 3);

        drop(stream);

        tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
            .await
            .expect("server timed out")
            .expect("server panicked");
    }
}
