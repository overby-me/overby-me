//! D-Bus proxy integration for Flatpak sandboxes.
//!
//! Launches `xdg-dbus-proxy` to provide filtered D-Bus access inside the
//! sandbox. The proxy listens on a Unix socket that gets bind-mounted into
//! the sandbox, filtering bus name access according to the app's metadata
//! policies.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// D-Bus bus policy level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BusPolicy {
    None,
    See,
    Talk,
    Own,
}

impl BusPolicy {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "own" => BusPolicy::Own,
            "talk" => BusPolicy::Talk,
            "see" => BusPolicy::See,
            _ => BusPolicy::None,
        }
    }

    pub fn as_flag(&self) -> &str {
        match self {
            BusPolicy::Own => "--own",
            BusPolicy::Talk => "--talk",
            BusPolicy::See => "--see",
            BusPolicy::None => "--none", // not a real flag, but won't be used
        }
    }
}

/// Configuration for a single D-Bus proxy instance.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ProxyConfig {
    /// The bus address to connect to (e.g., the session bus socket).
    pub bus_address: String,
    /// Path for the proxy's listening socket (inside a temp dir).
    pub proxy_socket: PathBuf,
    /// Per-name policies.
    pub policies: HashMap<String, BusPolicy>,
    /// Whether to enable logging.
    pub log: bool,
    /// Whether to enable filtering at all (false = pass everything through).
    pub filtering: bool,
}

/// A running D-Bus proxy process.
pub struct RunningProxy {
    pub child: Child,
    pub socket_path: PathBuf,
    pub temp_dir: PathBuf,
}

impl Drop for RunningProxy {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

/// Find the xdg-dbus-proxy binary.
fn find_dbus_proxy() -> Option<String> {
    if let Ok(path) = env::var("PATH") {
        for dir in path.split(':') {
            let candidate = format!("{dir}/xdg-dbus-proxy");
            if Path::new(&candidate).exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Get the session bus address.
pub fn session_bus_address() -> Option<String> {
    if let Ok(addr) = env::var("DBUS_SESSION_BUS_ADDRESS") {
        return Some(addr);
    }
    // Fall back to the default abstract socket path.
    let uid = unsafe { libc::getuid() };
    let path = format!("/run/user/{uid}/bus");
    if Path::new(&path).exists() {
        return Some(format!("unix:path={path}"));
    }
    None
}

/// Get the system bus address.
pub fn system_bus_address() -> Option<String> {
    if let Ok(addr) = env::var("DBUS_SYSTEM_BUS_ADDRESS") {
        return Some(addr);
    }
    let path = "/run/dbus/system_bus_socket";
    if Path::new(path).exists() {
        return Some(format!("unix:path={path}"));
    }
    None
}

/// Extract the socket path from a D-Bus address string.
fn socket_path_from_address(addr: &str) -> Option<String> {
    // Handle "unix:path=/some/path" or "unix:path=/some/path,key=val"
    for part in addr.split(',') {
        if let Some(rest) = part.strip_prefix("unix:path=") {
            return Some(rest.to_string());
        }
        if let Some(rest) = part.strip_prefix("path=") {
            return Some(rest.to_string());
        }
    }
    None
}

/// Launch a D-Bus proxy for the session bus.
///
/// Returns the running proxy and the socket path to bind-mount into the sandbox.
pub fn launch_session_proxy(
    app_id: &str,
    policies: &HashMap<String, BusPolicy>,
    instance_id: &str,
) -> Result<RunningProxy, String> {
    let proxy_bin = find_dbus_proxy().ok_or("xdg-dbus-proxy not found on PATH")?;
    let bus_addr = session_bus_address().ok_or("session bus not available")?;

    let uid = unsafe { libc::getuid() };
    let temp_dir = PathBuf::from(format!("/run/user/{uid}/.dbus-proxy-{instance_id}"));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("create proxy dir: {e}"))?;

    let proxy_socket = temp_dir.join("session-bus");

    let mut cmd = Command::new(&proxy_bin);
    cmd.arg(&bus_addr);
    cmd.arg(&proxy_socket);

    // If there are policies, enable filtering.
    if !policies.is_empty() {
        cmd.arg("--filter");

        for (name, policy) in policies {
            if *policy == BusPolicy::None {
                continue;
            }
            cmd.arg(format!("{}={}", policy.as_flag(), name));
        }

        // Always allow the app's own bus name.
        cmd.arg(format!("--own={app_id}"));
        cmd.arg(format!("--own={app_id}.*"));
    }

    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn xdg-dbus-proxy: {e}"))?;

    // Wait for the proxy socket to appear.
    wait_for_socket(&proxy_socket, 50)?;

    Ok(RunningProxy {
        child,
        socket_path: proxy_socket,
        temp_dir,
    })
}

/// Launch a D-Bus proxy for the system bus.
pub fn launch_system_proxy(
    _app_id: &str,
    policies: &HashMap<String, BusPolicy>,
    instance_id: &str,
) -> Result<RunningProxy, String> {
    let proxy_bin = find_dbus_proxy().ok_or("xdg-dbus-proxy not found on PATH")?;
    let bus_addr = system_bus_address().ok_or("system bus not available")?;

    let uid = unsafe { libc::getuid() };
    let temp_dir = PathBuf::from(format!("/run/user/{uid}/.dbus-proxy-system-{instance_id}"));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("create proxy dir: {e}"))?;

    let proxy_socket = temp_dir.join("system-bus");

    let mut cmd = Command::new(&proxy_bin);
    cmd.arg(&bus_addr);
    cmd.arg(&proxy_socket);

    if !policies.is_empty() {
        cmd.arg("--filter");

        for (name, policy) in policies {
            if *policy == BusPolicy::None {
                continue;
            }
            cmd.arg(format!("{}={}", policy.as_flag(), name));
        }
    }

    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn xdg-dbus-proxy (system): {e}"))?;

    wait_for_socket(&proxy_socket, 50)?;

    Ok(RunningProxy {
        child,
        socket_path: proxy_socket,
        temp_dir,
    })
}

/// Wait for a Unix socket to appear on the filesystem.
fn wait_for_socket(path: &Path, max_attempts: u32) -> Result<(), String> {
    for _ in 0..max_attempts {
        if path.exists() {
            // Verify it's actually a socket by trying to connect.
            if UnixListener::bind(path).is_err() {
                // bind fails if it already exists as a socket — that's what we want.
                return Ok(());
            }
            // It existed but wasn't a socket yet, or we could bind (shouldn't happen).
            let _ = fs::remove_file(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Accept if the file exists even if we couldn't verify.
    if path.exists() {
        return Ok(());
    }
    Err(format!(
        "timed out waiting for proxy socket: {}",
        path.display()
    ))
}

/// Build D-Bus proxy configuration from app metadata policies and context.
pub fn build_session_policies(
    session_bus_policy: &HashMap<String, String>,
    has_session_bus_socket: bool,
) -> (bool, HashMap<String, BusPolicy>) {
    // If the app has the session-bus socket permission, give unfiltered access.
    if has_session_bus_socket {
        return (false, HashMap::new()); // No filtering needed.
    }

    let mut policies = HashMap::new();

    // Default allowed names for all Flatpak apps.
    let default_talk = [
        "org.freedesktop.portal.*",
        "org.freedesktop.Flatpak",
        "org.freedesktop.DBus",
        "org.gtk.vfs.*",
        "org.gtk.vfs",
        "ca.desrt.dconf",
    ];
    for name in &default_talk {
        policies.insert(name.to_string(), BusPolicy::Talk);
    }

    // Merge app-specific policies.
    for (name, level) in session_bus_policy {
        let policy = BusPolicy::from_str(level);
        if policy == BusPolicy::None {
            policies.remove(name);
        } else {
            policies.insert(name.clone(), policy);
        }
    }

    (true, policies) // Filtering enabled.
}

/// Build system bus proxy configuration.
pub fn build_system_policies(
    system_bus_policy: &HashMap<String, String>,
    has_system_bus_socket: bool,
) -> (bool, HashMap<String, BusPolicy>) {
    if has_system_bus_socket {
        return (false, HashMap::new());
    }

    let mut policies = HashMap::new();

    // Default allowed system bus names.
    let default_talk = [
        "org.freedesktop.portal.*",
        "org.freedesktop.DBus",
        "org.freedesktop.Accounts",
        "org.freedesktop.NetworkManager",
        "org.freedesktop.login1",
        "org.freedesktop.timedate1",
        "org.freedesktop.locale1",
        "org.freedesktop.hostname1",
        "org.freedesktop.resolve1",
    ];
    for name in &default_talk {
        policies.insert(name.to_string(), BusPolicy::Talk);
    }

    for (name, level) in system_bus_policy {
        let policy = BusPolicy::from_str(level);
        if policy == BusPolicy::None {
            policies.remove(name);
        } else {
            policies.insert(name.clone(), policy);
        }
    }

    (true, policies)
}

/// Get the well-known session bus socket path for binding into the sandbox.
pub fn session_bus_socket_path() -> Option<PathBuf> {
    let uid = unsafe { libc::getuid() };
    let path = PathBuf::from(format!("/run/user/{uid}/bus"));
    if path.exists() {
        Some(path)
    } else {
        session_bus_address().and_then(|a| socket_path_from_address(&a).map(PathBuf::from))
    }
}

/// Get the system bus socket path.
pub fn system_bus_socket_path() -> Option<PathBuf> {
    let path = PathBuf::from("/run/dbus/system_bus_socket");
    if path.exists() {
        Some(path)
    } else {
        system_bus_address().and_then(|a| socket_path_from_address(&a).map(PathBuf::from))
    }
}
