use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: varlinkctl <command> [args...]");
        return ExitCode::FAILURE;
    }

    match args[1].as_str() {
        "call" => cmd_call(&args[2..]),
        "introspect" => cmd_introspect(&args[2..]),
        other => {
            eprintln!("varlinkctl: unknown command '{other}'");
            ExitCode::FAILURE
        }
    }
}

/// Send a request to a varlink target. The target is either a socket path
/// (connect to it) or an executable (exec it as a varlink server on fd 3, the
/// systemd socket-activation convention).
fn varlink_request(
    target: &str,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let is_socket = std::fs::metadata(target)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false);

    if is_socket {
        let stream = UnixStream::connect(target)
            .map_err(|e| format!("Failed to connect to {target}: {e}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        varlink_request_stream(&stream, request)
    } else {
        exec_and_request(target, request)
    }
}

/// Exec `exe` as a varlink server, passing a connected socket as fd 3 with
/// `LISTEN_FDS=1`/`LISTEN_PID=<child>`, and run one request/response exchange.
fn exec_and_request(
    exe: &str,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let (parent, child) = UnixStream::pair().map_err(|e| format!("socketpair failed: {e}"))?;
    let child_fd = child.as_raw_fd();
    let listen_pid_key = std::ffi::CString::new("LISTEN_PID").unwrap();

    let mut cmd = std::process::Command::new(exe);
    cmd.env("LISTEN_FDS", "1");
    unsafe {
        cmd.pre_exec(move || {
            // Move the child socket to fd 3 (dup2 clears CLOEXEC so it survives exec).
            if libc::dup2(child_fd, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // LISTEN_PID must be the server's own pid (getpid in this child).
            if let Ok(val) = std::ffi::CString::new(std::process::id().to_string()) {
                libc::setenv(listen_pid_key.as_ptr(), val.as_ptr(), 1);
            }
            Ok(())
        });
    }

    let mut server = cmd
        .spawn()
        .map_err(|e| format!("Failed to exec {exe}: {e}"))?;
    drop(child); // only the server should hold the child end now

    parent.set_read_timeout(Some(Duration::from_secs(30))).ok();
    parent.set_write_timeout(Some(Duration::from_secs(5))).ok();
    let result = varlink_request_stream(&parent, request);

    // Closing our end lets the server observe EOF and exit.
    drop(parent);
    let _ = server.wait();
    result
}

/// Run one NUL-framed request/response exchange over an established stream.
fn varlink_request_stream(
    stream: &UnixStream,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut writer = stream;
    let mut msg = serde_json::to_vec(request).map_err(|e| format!("JSON encode error: {e}"))?;
    msg.push(0); // NUL terminator
    writer
        .write_all(&msg)
        .map_err(|e| format!("Failed to send request: {e}"))?;

    // Shut down write side so the server knows we're done sending
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("Failed to shutdown write: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    reader
        .read_until(0, &mut buf)
        .map_err(|e| format!("Failed to read response: {e}"))?;

    // Strip trailing NUL
    if buf.last() == Some(&0) {
        buf.pop();
    }

    if buf.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        serde_json::from_slice(&buf).map_err(|e| format!("Invalid JSON response: {e}"))
    }
}

/// varlinkctl call <socket_path> <method> [parameters_json]
fn cmd_call(args: &[String]) -> ExitCode {
    if args.len() < 2 {
        eprintln!("Usage: varlinkctl call <socket_path> <method> [parameters_json]");
        return ExitCode::FAILURE;
    }

    let socket_path = &args[0];
    let method = &args[1];
    let parameters: serde_json::Value = if args.len() >= 3 {
        match serde_json::from_str(&args[2]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("varlinkctl: invalid parameters JSON: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        serde_json::json!({})
    };

    let request = serde_json::json!({
        "method": method,
        "parameters": parameters,
    });

    match varlink_request(socket_path, &request) {
        Ok(response) => {
            if let Some(error) = response.get("error") {
                eprintln!("varlinkctl: error: {error}");
                return ExitCode::FAILURE;
            }
            // Print the response (parameters field if present, otherwise the whole thing)
            if let Some(params) = response.get("parameters") {
                println!("{}", serde_json::to_string_pretty(params).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&response).unwrap());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("varlinkctl: {e}");
            ExitCode::FAILURE
        }
    }
}

/// varlinkctl introspect <socket_path> [interface]
fn cmd_introspect(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: varlinkctl introspect <socket_path> [interface]");
        return ExitCode::FAILURE;
    }

    let socket_path = &args[0];

    // First get the service info
    let request = serde_json::json!({
        "method": "org.varlink.service.GetInfo",
        "parameters": {},
    });

    match varlink_request(socket_path, &request) {
        Ok(response) => {
            if let Some(error) = response.get("error") {
                eprintln!("varlinkctl: error: {error}");
                return ExitCode::FAILURE;
            }
            if let Some(params) = response.get("parameters") {
                println!("{}", serde_json::to_string_pretty(params).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&response).unwrap());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("varlinkctl: {e}");
            ExitCode::FAILURE
        }
    }
}
