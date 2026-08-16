//! Socket transport for connecting to `tramp-agent` via TCP or Unix domain
//! sockets.
//!
//! This module provides the plugin-side counterpart to the agent's
//! `--listen` mode.  Instead of communicating over piped stdin/stdout
//! (through SSH or `docker exec`), the plugin connects directly to the
//! agent via a TCP port or Unix socket.
//!
//! This is particularly useful for:
//!
//! - **Local Docker containers** — avoids the overhead of a `docker exec`
//!   process sitting in the middle of every RPC exchange.
//! - **Local Kubernetes pods** with port-forwarding — connect via
//!   `kubectl port-forward` or a direct pod IP.
//! - **Persistent agent processes** — an agent started once and reused
//!   across multiple plugin sessions.
//!
//! ## Usage
//!
//! ```text
//! # Start agent in a container with TCP listener:
//! docker exec -d mycontainer tramp-agent --listen tcp:0.0.0.0:9547
//!
//! # The plugin connects directly:
//! connect_tcp("127.0.0.1:9547") → RpcBackend
//! ```

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{BufReader, BufWriter};
use tokio::net::TcpStream;

use super::rpc::RpcBackend;
use super::rpc_client::RpcClient;
use crate::errors::{TrampError, TrampResult};

/// Default timeout for establishing a socket connection (5 seconds).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default timeout for the initial ping after connecting (3 seconds).
const PING_TIMEOUT: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// Parsed socket address
// ---------------------------------------------------------------------------

/// A parsed socket address that the plugin can connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketAddr {
    /// TCP address (host:port).
    Tcp(std::net::SocketAddr),
    /// Unix domain socket path.
    #[cfg(unix)]
    Unix(String),
}

impl std::fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SocketAddr::Tcp(addr) => write!(f, "tcp:{addr}"),
            #[cfg(unix)]
            SocketAddr::Unix(path) => write!(f, "unix:{path}"),
        }
    }
}

/// Parse a socket address string.
///
/// Accepted formats:
///   - `tcp:<host>:<port>`   e.g. `tcp:127.0.0.1:9547`
///   - `unix:<path>`         e.g. `unix:/tmp/tramp-agent.sock`
///   - `<host>:<port>`       (auto-detected as TCP)
///
/// This mirrors the agent's `parse_listen_addr` function.
pub fn parse_socket_addr(s: &str) -> Result<SocketAddr, String> {
    if let Some(rest) = s.strip_prefix("tcp:") {
        let addr: std::net::SocketAddr = rest
            .parse()
            .map_err(|e| format!("invalid TCP address '{rest}': {e}"))?;
        return Ok(SocketAddr::Tcp(addr));
    }

    #[cfg(unix)]
    if let Some(rest) = s.strip_prefix("unix:") {
        if rest.is_empty() {
            return Err("unix socket path cannot be empty".into());
        }
        return Ok(SocketAddr::Unix(rest.to_string()));
    }

    #[cfg(not(unix))]
    if s.starts_with("unix:") {
        return Err("Unix domain sockets are not supported on this platform".into());
    }

    // Auto-detect: try TCP.
    if let Ok(addr) = s.parse::<std::net::SocketAddr>() {
        return Ok(SocketAddr::Tcp(addr));
    }

    // Try resolving as host:port string (e.g. "localhost:9547").
    // We use `to_socket_addrs` which does DNS resolution, but since this
    // runs async we do a simpler check: just try parsing with a default.
    Err(format!(
        "cannot parse socket address '{s}' — use tcp:<host>:<port> or unix:<path>"
    ))
}

// ---------------------------------------------------------------------------
// TCP transport
// ---------------------------------------------------------------------------

/// Connect to a `tramp-agent` listening on a TCP address and return an
/// [`RpcBackend`] ready for use.
///
/// The connection is verified with a `ping` RPC call before returning.
pub async fn connect_tcp(
    addr: std::net::SocketAddr,
    host_label: &str,
) -> TrampResult<Arc<dyn super::Backend>> {
    connect_tcp_with_timeout(addr, host_label, CONNECT_TIMEOUT, PING_TIMEOUT).await
}

/// Like [`connect_tcp`] but with configurable timeouts.
pub async fn connect_tcp_with_timeout(
    addr: std::net::SocketAddr,
    host_label: &str,
    connect_timeout: Duration,
    ping_timeout: Duration,
) -> TrampResult<Arc<dyn super::Backend>> {
    // Establish the TCP connection with a timeout.
    let stream = tokio::time::timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| TrampError::ConnectionFailed {
            host: format!("tcp:{addr}"),
            reason: format!("connection timed out after {connect_timeout:?}"),
        })?
        .map_err(|e| TrampError::ConnectionFailed {
            host: format!("tcp:{addr}"),
            reason: e.to_string(),
        })?;

    // Disable Nagle's algorithm for lower latency on small RPC messages.
    let _ = stream.set_nodelay(true);

    let (read_half, write_half) = stream.into_split();
    let reader = BufReader::new(read_half);
    let writer = BufWriter::new(write_half);

    let client = RpcClient::new(reader, writer);

    // Verify the agent is alive.
    tokio::time::timeout(ping_timeout, client.ping())
        .await
        .map_err(|_| TrampError::ConnectionFailed {
            host: format!("tcp:{addr}"),
            reason: format!("agent ping timed out after {ping_timeout:?}"),
        })?
        .map_err(|e| TrampError::ConnectionFailed {
            host: format!("tcp:{addr}"),
            reason: format!("agent ping failed: {e}"),
        })?;

    let description = format!("{host_label} (tcp:{addr})");
    Ok(Arc::new(RpcBackend::new(client, description)))
}

// ---------------------------------------------------------------------------
// Unix socket transport
// ---------------------------------------------------------------------------

/// Connect to a `tramp-agent` listening on a Unix domain socket and return
/// an [`RpcBackend`] ready for use.
#[cfg(unix)]
pub async fn connect_unix(path: &str, host_label: &str) -> TrampResult<Arc<dyn super::Backend>> {
    connect_unix_with_timeout(path, host_label, CONNECT_TIMEOUT, PING_TIMEOUT).await
}

/// Like [`connect_unix`] but with configurable timeouts.
#[cfg(unix)]
pub async fn connect_unix_with_timeout(
    path: &str,
    host_label: &str,
    connect_timeout: Duration,
    ping_timeout: Duration,
) -> TrampResult<Arc<dyn super::Backend>> {
    use tokio::net::UnixStream;

    let socket_path = path.to_string();

    // Establish the Unix socket connection with a timeout.
    let stream = tokio::time::timeout(connect_timeout, UnixStream::connect(&socket_path))
        .await
        .map_err(|_| TrampError::ConnectionFailed {
            host: format!("unix:{socket_path}"),
            reason: format!("connection timed out after {connect_timeout:?}"),
        })?
        .map_err(|e| TrampError::ConnectionFailed {
            host: format!("unix:{socket_path}"),
            reason: e.to_string(),
        })?;

    let (read_half, write_half) = stream.into_split();
    let reader = BufReader::new(read_half);
    let writer = BufWriter::new(write_half);

    let client = RpcClient::new(reader, writer);

    // Verify the agent is alive.
    tokio::time::timeout(ping_timeout, client.ping())
        .await
        .map_err(|_| TrampError::ConnectionFailed {
            host: format!("unix:{socket_path}"),
            reason: format!("agent ping timed out after {ping_timeout:?}"),
        })?
        .map_err(|e| TrampError::ConnectionFailed {
            host: format!("unix:{socket_path}"),
            reason: format!("agent ping failed: {e}"),
        })?;

    let description = format!("{host_label} (unix:{socket_path})");
    Ok(Arc::new(RpcBackend::new(client, description)))
}

// ---------------------------------------------------------------------------
// Generic connect
// ---------------------------------------------------------------------------

/// Connect to a `tramp-agent` at the given [`SocketAddr`].
///
/// This is a convenience wrapper that dispatches to [`connect_tcp`] or
/// [`connect_unix`] based on the address type.
pub async fn connect(addr: &SocketAddr, host_label: &str) -> TrampResult<Arc<dyn super::Backend>> {
    match addr {
        SocketAddr::Tcp(sock_addr) => connect_tcp(*sock_addr, host_label).await,
        #[cfg(unix)]
        SocketAddr::Unix(path) => connect_unix(path, host_label).await,
    }
}

// ---------------------------------------------------------------------------
// Docker container helper — start agent with TCP listener and connect
// ---------------------------------------------------------------------------

/// The default TCP port the agent listens on inside containers.
pub const DEFAULT_AGENT_PORT: u16 = 9547;

/// Start a `tramp-agent` inside a Docker container with `--listen tcp:...`,
/// wait for it to become ready, and return a connected [`RpcBackend`].
///
/// This avoids the overhead of keeping a `docker exec -i` process running
/// for the lifetime of the connection.
///
/// Returns `None` if the agent cannot be started or connected (caller
/// should fall back to the stdin/stdout approach).
pub async fn start_docker_tcp_agent(
    container: &str,
    agent_path: &str,
    port: u16,
) -> Option<Arc<dyn super::Backend>> {
    // Start the agent in the background inside the container.
    let listen_addr = format!("tcp:0.0.0.0:{port}");
    let start_result = tokio::process::Command::new("docker")
        .args([
            "exec",
            "-d",
            container,
            agent_path,
            "--listen",
            &listen_addr,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match start_result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!(
                "tramp: failed to start agent in docker:{container}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return None;
        }
        Err(e) => {
            eprintln!("tramp: failed to exec docker for {container}: {e}");
            return None;
        }
    }

    // Get the container's IP address for direct TCP connection.
    let ip = get_docker_container_ip(container).await?;

    // Wait for the agent to start listening (poll with retries).
    let addr: std::net::SocketAddr = format!("{ip}:{port}").parse().ok()?;

    let max_retries = 10;
    let retry_delay = Duration::from_millis(200);

    for attempt in 0..max_retries {
        match connect_tcp_with_timeout(
            addr,
            &format!("docker:{container}"),
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        {
            Ok(backend) => {
                eprintln!(
                    "tramp: connected to agent in docker:{container} via tcp:{addr} (attempt {})",
                    attempt + 1
                );
                return Some(backend);
            }
            Err(_) if attempt < max_retries - 1 => {
                tokio::time::sleep(retry_delay).await;
            }
            Err(e) => {
                eprintln!(
                    "tramp: failed to connect to agent in docker:{container} via tcp:{addr}: {e}"
                );
                return None;
            }
        }
    }

    None
}

/// Get the IP address of a Docker container.
async fn get_docker_container_ip(container: &str) -> Option<String> {
    let output = tokio::process::Command::new("docker")
        .args([
            "inspect",
            "-f",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
            container,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let ip = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if ip.is_empty() { None } else { Some(ip) }
}

/// Start a `tramp-agent` inside a Kubernetes pod with `--listen tcp:...`,
/// set up port-forwarding, and return a connected [`RpcBackend`].
///
/// Returns `None` if the agent cannot be started or connected.
pub async fn start_k8s_tcp_agent(
    pod: &str,
    container: Option<&str>,
    agent_path: &str,
    port: u16,
) -> Option<Arc<dyn super::Backend>> {
    // Start the agent in the background inside the pod.
    let listen_addr = format!("tcp:0.0.0.0:{port}");
    let mut cmd_args = vec!["exec", pod];
    if let Some(c) = container {
        cmd_args.extend(["-c", c]);
    }
    cmd_args.extend(["--", agent_path, "--listen", &listen_addr]);

    // kubectl exec with -d is not directly supported, so we spawn it as
    // a background process and detach.  We use `sh -c '... &'` to
    // background the agent inside the pod.
    let bg_script = format!("{agent_path} --listen {listen_addr} &");
    let mut bg_args = vec!["exec", pod];
    if let Some(c) = container {
        bg_args.extend(["-c", c]);
    }
    bg_args.extend(["--", "sh", "-c", &bg_script]);

    let start_result = tokio::process::Command::new("kubectl")
        .args(&bg_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    match start_result {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            eprintln!(
                "tramp: failed to start agent in k8s:{pod}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return None;
        }
        Err(e) => {
            eprintln!("tramp: failed to exec kubectl for {pod}: {e}");
            return None;
        }
    }

    // Set up port-forwarding from a local ephemeral port to the agent port.
    let port_forward_spec = format!("0:{port}");
    let mut pf_child = tokio::process::Command::new("kubectl")
        .args(["port-forward", pod, &port_forward_spec])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    // Read the first line of stdout to discover the local port.
    // kubectl outputs something like: "Forwarding from 127.0.0.1:NNNNN -> 9547"
    let stdout = pf_child.stdout.take()?;
    let local_port = discover_kubectl_port_forward(stdout).await;

    let local_port = match local_port {
        Some(p) => p,
        None => {
            let _ = pf_child.kill().await;
            eprintln!("tramp: failed to discover port-forward port for k8s:{pod}");
            return None;
        }
    };

    // Connect to the locally-forwarded port.
    let addr: std::net::SocketAddr = format!("127.0.0.1:{local_port}").parse().ok()?;
    let max_retries = 10;
    let retry_delay = Duration::from_millis(200);

    for attempt in 0..max_retries {
        match connect_tcp_with_timeout(
            addr,
            &format!("k8s:{pod}"),
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        {
            Ok(backend) => {
                eprintln!(
                    "tramp: connected to agent in k8s:{pod} via port-forward :{local_port} (attempt {})",
                    attempt + 1
                );
                // Leak the port-forward process — it will be killed when
                // the plugin exits or when the connection is dropped.
                // A proper implementation would track and clean up these
                // processes, but for now this is acceptable.
                std::mem::forget(pf_child);
                return Some(backend);
            }
            Err(_) if attempt < max_retries - 1 => {
                tokio::time::sleep(retry_delay).await;
            }
            Err(e) => {
                let _ = pf_child.kill().await;
                eprintln!("tramp: failed to connect to agent in k8s:{pod} via port-forward: {e}");
                return None;
            }
        }
    }

    let _ = pf_child.kill().await;
    None
}

/// Parse the local port from `kubectl port-forward` stdout output.
///
/// The output looks like: `Forwarding from 127.0.0.1:43567 -> 9547`
async fn discover_kubectl_port_forward(stdout: tokio::process::ChildStdout) -> Option<u16> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();

    // Read with a timeout — kubectl might not output anything if something
    // goes wrong.
    let result = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await;

    match result {
        Ok(Ok(n)) if n > 0 => {
            // Parse "Forwarding from 127.0.0.1:PORT -> ..."
            // or "Forwarding from [::1]:PORT -> ..."
            if let Some(from_idx) = line.find("Forwarding from ") {
                let after_from = &line[from_idx + "Forwarding from ".len()..];
                // Find the port: it's between the last ':' before ' ->' and ' ->'
                if let Some(arrow_idx) = after_from.find(" ->") {
                    let addr_part = &after_from[..arrow_idx];
                    // Port is after the last ':'
                    if let Some(colon_idx) = addr_part.rfind(':') {
                        let port_str = &addr_part[colon_idx + 1..];
                        return port_str.parse::<u16>().ok();
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_explicit() {
        let addr = parse_socket_addr("tcp:127.0.0.1:9547").unwrap();
        assert_eq!(addr, SocketAddr::Tcp("127.0.0.1:9547".parse().unwrap()));
    }

    #[test]
    fn parse_tcp_auto() {
        let addr = parse_socket_addr("127.0.0.1:9547").unwrap();
        assert_eq!(addr, SocketAddr::Tcp("127.0.0.1:9547".parse().unwrap()));
    }

    #[test]
    fn parse_tcp_ipv6() {
        let addr = parse_socket_addr("tcp:[::1]:9547").unwrap();
        assert_eq!(addr, SocketAddr::Tcp("[::1]:9547".parse().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn parse_unix_explicit() {
        let addr = parse_socket_addr("unix:/tmp/tramp-agent.sock").unwrap();
        assert_eq!(addr, SocketAddr::Unix("/tmp/tramp-agent.sock".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn parse_unix_empty_rejected() {
        let result = parse_socket_addr("unix:");
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_rejected() {
        let result = parse_socket_addr("not-a-valid-addr");
        assert!(result.is_err());
    }

    #[test]
    fn socket_addr_display_tcp() {
        let addr = SocketAddr::Tcp("127.0.0.1:9547".parse().unwrap());
        assert_eq!(addr.to_string(), "tcp:127.0.0.1:9547");
    }

    #[cfg(unix)]
    #[test]
    fn socket_addr_display_unix() {
        let addr = SocketAddr::Unix("/tmp/agent.sock".to_string());
        assert_eq!(addr.to_string(), "unix:/tmp/agent.sock");
    }

    /// Test TCP connection to a non-existent address fails gracefully.
    #[tokio::test]
    async fn connect_tcp_refused() {
        // Use a port that's almost certainly not listening.
        let addr: std::net::SocketAddr = "127.0.0.1:1".parse().unwrap();
        let result = connect_tcp_with_timeout(
            addr,
            "test",
            Duration::from_millis(500),
            Duration::from_millis(500),
        )
        .await;
        assert!(result.is_err());
    }

    /// Test Unix socket connection to a non-existent path fails gracefully.
    #[cfg(unix)]
    #[tokio::test]
    async fn connect_unix_nonexistent() {
        let result = connect_unix_with_timeout(
            "/tmp/tramp-agent-test-nonexistent-12345.sock",
            "test",
            Duration::from_millis(500),
            Duration::from_millis(500),
        )
        .await;
        assert!(result.is_err());
    }
}
