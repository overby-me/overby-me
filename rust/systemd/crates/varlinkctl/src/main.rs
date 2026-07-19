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

    // A leading `--more`/`-m` (before the command) enables streaming replies for
    // `call`, matching upstream `varlinkctl --more call ...`.
    let mut more = false;
    let mut rest: Vec<String> = Vec::new();
    for a in &args[1..] {
        match a.as_str() {
            "--more" | "-m" | "-E" => more = true,
            "--json=short" | "--json=pretty" | "-J" => {} // accepted, no-op formatting hints
            _ => rest.push(a.clone()),
        }
    }
    if rest.is_empty() {
        eprintln!("Usage: varlinkctl [--more] <command> [args...]");
        return ExitCode::FAILURE;
    }

    match rest[0].as_str() {
        "call" => cmd_call(&rest[1..], more),
        "introspect" => cmd_introspect(&rest[1..]),
        "info" => cmd_info(&rest[1..]),
        "list-methods" => cmd_list_methods(&rest[1..]),
        other => {
            eprintln!("varlinkctl: unknown command '{other}'");
            ExitCode::FAILURE
        }
    }
}

/// varlinkctl info <target> — print the service's org.varlink.service GetInfo.
fn cmd_info(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: varlinkctl info <target>");
        return ExitCode::FAILURE;
    }
    let request = serde_json::json!({
        "method": "org.varlink.service.GetInfo",
        "parameters": {},
    });
    match varlink_request(&args[0], &request) {
        Ok(response) => {
            if let Some(error) = response.get("error") {
                eprintln!("varlinkctl: error: {error}");
                return ExitCode::FAILURE;
            }
            let params = response.get("parameters").unwrap_or(&response);
            println!("{}", serde_json::to_string_pretty(params).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("varlinkctl: {e}");
            ExitCode::FAILURE
        }
    }
}

/// varlinkctl list-methods <target> — list the methods of each interface the
/// service exposes, via GetInfo + GetInterfaceDescription.
fn cmd_list_methods(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("Usage: varlinkctl list-methods <target>");
        return ExitCode::FAILURE;
    }
    let target = &args[0];
    let info_req = serde_json::json!({
        "method": "org.varlink.service.GetInfo",
        "parameters": {},
    });
    let info = match varlink_request(target, &info_req) {
        Ok(r) if r.get("error").is_none() => r,
        Ok(r) => {
            eprintln!("varlinkctl: error: {}", r.get("error").unwrap());
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("varlinkctl: {e}");
            return ExitCode::FAILURE;
        }
    };
    let interfaces = info
        .get("parameters")
        .and_then(|p| p.get("interfaces"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for iface in interfaces {
        let name = match iface.as_str() {
            Some(n) => n,
            None => continue,
        };
        if name == "org.varlink.service" {
            continue;
        }
        let desc_req = serde_json::json!({
            "method": "org.varlink.service.GetInterfaceDescription",
            "parameters": { "interface": name },
        });
        if let Ok(r) = varlink_request(target, &desc_req)
            && let Some(desc) = r
                .get("parameters")
                .and_then(|p| p.get("description"))
                .and_then(|v| v.as_str())
        {
            for line in desc.lines() {
                let t = line.trim_start();
                if let Some(m) = t.strip_prefix("method ") {
                    let method = m.split('(').next().unwrap_or(m).trim();
                    println!("{name}.{method}");
                }
            }
        }
    }
    ExitCode::SUCCESS
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

/// Fork+exec `exe` as a varlink server, passing a connected socket as fd 3 with
/// `LISTEN_FDS=1`/`LISTEN_PID=<child>` (the systemd socket-activation
/// convention). Returns the running child and our end of the connection.
fn spawn_varlink_server(exe: &str) -> Result<(std::process::Child, UnixStream), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;

    let (parent, child) = UnixStream::pair().map_err(|e| format!("socketpair failed: {e}"))?;
    let child_fd = child.as_raw_fd();
    let listen_pid_key = std::ffi::CString::new("LISTEN_PID").unwrap();
    let listen_fds_key = std::ffi::CString::new("LISTEN_FDS").unwrap();
    let listen_fds_val = std::ffi::CString::new("1").unwrap();

    let mut cmd = std::process::Command::new(exe);
    // NB: do NOT set LISTEN_FDS via cmd.env(). Setting any env through Command
    // makes Rust build an explicit envp array that REPLACES `environ` *after*
    // our pre_exec closure runs, which would discard the LISTEN_PID we setenv()
    // below (its value, the child's own pid, is unknowable before fork). Set
    // both LISTEN_* vars via setenv() in pre_exec so the inherited `environ`
    // (used by execvp) carries them through to the server.
    unsafe {
        cmd.pre_exec(move || {
            // Move the child socket to fd 3 (dup2 clears CLOEXEC so it survives exec).
            if libc::dup2(child_fd, 3) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::setenv(listen_fds_key.as_ptr(), listen_fds_val.as_ptr(), 1);
            // LISTEN_PID must be the server's own pid (getpid in this child).
            if let Ok(val) = std::ffi::CString::new(std::process::id().to_string()) {
                libc::setenv(listen_pid_key.as_ptr(), val.as_ptr(), 1);
            }
            Ok(())
        });
    }

    let server = cmd
        .spawn()
        .map_err(|e| format!("Failed to exec {exe}: {e}"))?;
    drop(child); // only the server should hold the child end now
    Ok((server, parent))
}

/// Exec `exe` as a varlink server and run one request/response exchange.
fn exec_and_request(
    exe: &str,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (mut server, parent) = spawn_varlink_server(exe)?;
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

/// varlinkctl [-E|--more|-m] call <target> <method> [parameters_json]
///
/// Flags may appear after the `call` verb too (upstream accepts, e.g.,
/// `varlinkctl call -E ADDRESS METHOD PARAMS`). `-E` is short for
/// `--more --timeout=infinity`.
fn cmd_call(args: &[String], more_leading: bool) -> ExitCode {
    let mut more = more_leading;
    let mut positional: Vec<String> = Vec::with_capacity(args.len());
    for a in args {
        match a.as_str() {
            "-E" | "--more" | "-m" => more = true,
            "-O" | "--oneway" | "-J" | "--collect" | "-q" | "--quiet" => {} // accepted, no-op
            s if s.starts_with("--timeout") || s.starts_with("--json") => {} // accepted, no-op
            _ => positional.push(a.clone()),
        }
    }

    if positional.len() < 2 {
        eprintln!("Usage: varlinkctl [-E|--more] call <target> <method> [parameters_json]");
        return ExitCode::FAILURE;
    }

    let socket_path = &positional[0];
    let method = &positional[1];
    let parameters: serde_json::Value = if positional.len() >= 3 {
        match serde_json::from_str(&positional[2]) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("varlinkctl: invalid parameters JSON: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        serde_json::json!({})
    };

    let mut request = serde_json::json!({
        "method": method,
        "parameters": parameters,
    });
    if more {
        request["more"] = serde_json::json!(true);
    }

    let print_reply = |response: &serde_json::Value| {
        if let Some(params) = response.get("parameters") {
            println!("{}", serde_json::to_string_pretty(params).unwrap());
        } else {
            println!("{}", serde_json::to_string_pretty(response).unwrap());
        }
    };

    if more {
        match varlink_call_more(socket_path, &request) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("varlinkctl: {e}");
                ExitCode::FAILURE
            }
        }
    } else {
        match varlink_request(socket_path, &request) {
            Ok(response) => {
                if let Some(error) = response.get("error") {
                    eprintln!("varlinkctl: error: {error}");
                    return ExitCode::FAILURE;
                }
                print_reply(&response);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("varlinkctl: {e}");
                ExitCode::FAILURE
            }
        }
    }
}

/// A `--more`/`-E` call: send `request` over an established connection (socket
/// target) or a freshly exec'd server (executable target), then print each
/// NUL-framed reply until one lacks `"continues": true` (or EOF).
///
/// The write side is deliberately NOT shut down. Keep-open methods reply with
/// `continues=true` and hold the call open for the lifetime of the connection;
/// they observe our disconnect only when this process exits or is killed. That
/// matches upstream `-E` (= `--more --timeout=infinity`): the read blocks
/// indefinitely (no read timeout) until the server sends a final reply or we
/// are terminated.
fn varlink_call_more(target: &str, request: &serde_json::Value) -> Result<ExitCode, String> {
    let is_socket = std::fs::metadata(target)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false);

    if is_socket {
        let stream =
            UnixStream::connect(target).map_err(|e| format!("Failed to connect to {target}: {e}"))?;
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        stream_more(&stream, request)
    } else {
        let (mut server, parent) = spawn_varlink_server(target)?;
        parent.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let result = stream_more(&parent, request);
        // Closing our end lets a keep-open server observe EOF and restore.
        drop(parent);
        let _ = server.wait();
        result
    }
}

/// Send `request` (without shutting the write side) and print each streamed
/// reply until one is final (`continues` unset/false) or the peer closes.
fn stream_more(stream: &UnixStream, request: &serde_json::Value) -> Result<ExitCode, String> {
    let mut writer = stream;
    let mut msg = serde_json::to_vec(request).map_err(|e| format!("JSON encode error: {e}"))?;
    msg.push(0);
    writer
        .write_all(&msg)
        .map_err(|e| format!("Failed to send request: {e}"))?;

    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = Vec::new();
        let n = reader
            .read_until(0, &mut buf)
            .map_err(|e| format!("Failed to read response: {e}"))?;
        if n == 0 {
            break; // EOF: peer closed
        }
        if buf.last() == Some(&0) {
            buf.pop();
        }
        if buf.is_empty() {
            break;
        }
        let reply: serde_json::Value =
            serde_json::from_slice(&buf).map_err(|e| format!("Invalid JSON response: {e}"))?;
        if let Some(error) = reply.get("error") {
            eprintln!("varlinkctl: error: {error}");
            return Ok(ExitCode::FAILURE);
        }
        if let Some(params) = reply.get("parameters") {
            println!("{}", serde_json::to_string_pretty(params).unwrap());
        } else {
            println!("{}", serde_json::to_string_pretty(&reply).unwrap());
        }
        let continues = reply.get("continues").and_then(|v| v.as_bool()).unwrap_or(false);
        if !continues {
            break;
        }
    }
    Ok(ExitCode::SUCCESS)
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
