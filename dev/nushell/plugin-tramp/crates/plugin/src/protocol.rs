//! TRAMP URI parser.
//!
//! Parses paths of the form:
//!
//! ```text
//! /<backend>:<user>@<host>#<port>:<remote-path>
//! ```
//!
//! Chained (multi-hop) paths use `|` as a separator:
//!
//! ```text
//! /ssh:jumpbox|ssh:myvm:/etc/config
//! ```
//!
//! The parser produces a [`TrampPath`] containing a `Vec<Hop>` (to support
//! future chaining) and the final remote path.

use crate::errors::{TrampError, TrampResult};
use std::fmt;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A fully parsed TRAMP URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrampPath {
    /// One or more hops (the last hop owns the `remote_path`).
    pub hops: Vec<Hop>,
    /// The absolute path on the final remote host.
    pub remote_path: String,
}

/// A single hop in a (possibly chained) TRAMP path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub backend: BackendKind,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

/// Known backend transport types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendKind {
    Ssh,
    Docker,
    Kubernetes,
    Sudo,
}

// ---------------------------------------------------------------------------
// Display / formatting (round-trip support)
// ---------------------------------------------------------------------------

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendKind::Ssh => write!(f, "ssh"),
            BackendKind::Docker => write!(f, "docker"),
            BackendKind::Kubernetes => write!(f, "k8s"),
            BackendKind::Sudo => write!(f, "sudo"),
        }
    }
}

impl fmt::Display for Hop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.backend)?;
        write!(f, ":")?;
        if let Some(ref user) = self.user {
            write!(f, "{user}@")?;
        }
        write!(f, "{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, "#{port}")?;
        }
        Ok(())
    }
}

impl fmt::Display for TrampPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/")?;
        for (i, hop) in self.hops.iter().enumerate() {
            if i > 0 {
                write!(f, "|")?;
            }
            write!(f, "{hop}")?;
        }
        write!(f, ":{}", self.remote_path)
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Known backend name strings.
const KNOWN_BACKENDS: &[(&str, BackendKind)] = &[
    ("ssh", BackendKind::Ssh),
    ("docker", BackendKind::Docker),
    ("k8s", BackendKind::Kubernetes),
    ("kubernetes", BackendKind::Kubernetes),
    ("sudo", BackendKind::Sudo),
];

/// Parse a backend name into a [`BackendKind`].
fn parse_backend(name: &str) -> TrampResult<BackendKind> {
    for &(label, kind) in KNOWN_BACKENDS {
        if name.eq_ignore_ascii_case(label) {
            return Ok(kind);
        }
    }
    Err(TrampError::UnknownBackend(name.to_string()))
}

/// Returns `true` if `path` looks like a TRAMP URI (quick check without full
/// parsing).  This is intentionally cheap so callers can bail out early for
/// normal local paths.
pub fn is_tramp_path(path: &str) -> bool {
    let Some(path) = path.strip_prefix('/') else {
        return false;
    };
    // Must start with a known backend name followed by ':'
    let Some(colon_idx) = path.find(':') else {
        return false;
    };
    let candidate = &path[..colon_idx];
    // The candidate might include a pipe from chaining — take the first
    // segment before any '|'.
    let first_segment = candidate.split('|').next().unwrap_or(candidate);
    KNOWN_BACKENDS
        .iter()
        .any(|&(label, _)| first_segment.eq_ignore_ascii_case(label))
}

/// Parse a single hop segment like `ssh:user@host#port`.
///
/// The segment must NOT include the leading `/` or any trailing `:<remote-path>`.
fn parse_hop(segment: &str) -> TrampResult<Hop> {
    // Split on the first ':' to get backend and the host-info part.
    let (backend_str, host_info) = segment
        .split_once(':')
        .ok_or_else(|| TrampError::ParseError(format!("expected ':' in hop '{segment}'")))?;

    let backend = parse_backend(backend_str)?;

    if host_info.is_empty() {
        return Err(TrampError::MissingHost);
    }

    // host_info is like "user@host#port" or "host#port" or "host" or "user@host"
    let (user, remainder) = if let Some(at_idx) = host_info.find('@') {
        let user = &host_info[..at_idx];
        if user.is_empty() {
            (None, &host_info[at_idx + 1..])
        } else {
            (Some(user.to_string()), &host_info[at_idx + 1..])
        }
    } else {
        (None, host_info)
    };

    // remainder is like "host#port" or "host"
    let (host, port) = if let Some(hash_idx) = remainder.find('#') {
        let host = &remainder[..hash_idx];
        let port_str = &remainder[hash_idx + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|_| TrampError::InvalidPort(port_str.to_string()))?;
        (host.to_string(), Some(port))
    } else {
        (remainder.to_string(), None)
    };

    if host.is_empty() {
        return Err(TrampError::MissingHost);
    }

    Ok(Hop {
        backend,
        user,
        host,
        port,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a TRAMP URI string into a [`TrampPath`].
///
/// Returns `Ok(None)` if the path is not a TRAMP URI at all (i.e. a normal
/// local path). Returns `Err` if it looks like a TRAMP URI but is malformed.
pub fn parse(path: &str) -> TrampResult<Option<TrampPath>> {
    if !is_tramp_path(path) {
        return Ok(None);
    }

    // Strip the leading '/'
    let path = &path[1..];

    // Find the *last* unambiguous colon that starts the remote path.
    // The remote path always starts with '/', so we look for ":/" to locate it.
    // This avoids confusion with the ':' separators inside hop segments.
    let remote_sep = path.find(":/").ok_or(TrampError::MissingRemotePath)?;

    let hops_str = &path[..remote_sep];
    let remote_path = &path[remote_sep + 1..]; // includes the leading '/'

    if remote_path.is_empty() {
        return Err(TrampError::MissingRemotePath);
    }

    // Split hops on '|'
    let hop_segments: Vec<&str> = hops_str.split('|').collect();
    let mut hops = Vec::with_capacity(hop_segments.len());
    for seg in hop_segments {
        hops.push(parse_hop(seg)?);
    }

    if hops.is_empty() {
        return Err(TrampError::ParseError(
            "no hops found in tramp path".to_string(),
        ));
    }

    Ok(Some(TrampPath {
        hops,
        remote_path: remote_path.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- is_tramp_path -------------------------------------------------------

    #[test]
    fn detect_ssh_path() {
        assert!(is_tramp_path("/ssh:myvm:/etc/config"));
    }

    #[test]
    fn detect_docker_path() {
        assert!(is_tramp_path("/docker:mycontainer:/app"));
    }

    #[test]
    fn detect_k8s_path() {
        assert!(is_tramp_path("/k8s:mypod:/tmp"));
    }

    #[test]
    fn detect_sudo_path() {
        assert!(is_tramp_path("/sudo:root:/etc/shadow"));
    }

    #[test]
    fn reject_normal_path() {
        assert!(!is_tramp_path("/etc/config"));
        assert!(!is_tramp_path("/home/user/.config"));
        assert!(!is_tramp_path("relative/path"));
        assert!(!is_tramp_path(""));
    }

    #[test]
    fn reject_unknown_backend() {
        assert!(!is_tramp_path("/ftp:host:/file"));
    }

    #[test]
    fn detect_chained_path() {
        assert!(is_tramp_path("/ssh:jumpbox|ssh:myvm:/etc/config"));
    }

    // -- parse: basic --------------------------------------------------------

    #[test]
    fn parse_simple_ssh() {
        let result = parse("/ssh:myvm:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops.len(), 1);
        assert_eq!(result.hops[0].backend, BackendKind::Ssh);
        assert_eq!(result.hops[0].host, "myvm");
        assert_eq!(result.hops[0].user, None);
        assert_eq!(result.hops[0].port, None);
        assert_eq!(result.remote_path, "/etc/config");
    }

    #[test]
    fn parse_ssh_with_user() {
        let result = parse("/ssh:admin@myvm:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops[0].user.as_deref(), Some("admin"));
        assert_eq!(result.hops[0].host, "myvm");
    }

    #[test]
    fn parse_ssh_with_user_and_port() {
        let result = parse("/ssh:admin@myvm#2222:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops[0].user.as_deref(), Some("admin"));
        assert_eq!(result.hops[0].host, "myvm");
        assert_eq!(result.hops[0].port, Some(2222));
        assert_eq!(result.remote_path, "/etc/config");
    }

    #[test]
    fn parse_ssh_with_port_no_user() {
        let result = parse("/ssh:myvm#22:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops[0].user, None);
        assert_eq!(result.hops[0].host, "myvm");
        assert_eq!(result.hops[0].port, Some(22));
    }

    #[test]
    fn parse_ssh_ip_address() {
        let result = parse("/ssh:192.168.1.10:/var/log").unwrap().unwrap();
        assert_eq!(result.hops[0].host, "192.168.1.10");
        assert_eq!(result.remote_path, "/var/log");
    }

    #[test]
    fn parse_deep_remote_path() {
        let result = parse("/ssh:myvm:/a/b/c/d/e.txt").unwrap().unwrap();
        assert_eq!(result.remote_path, "/a/b/c/d/e.txt");
    }

    #[test]
    fn parse_root_remote_path() {
        let result = parse("/ssh:myvm:/").unwrap().unwrap();
        assert_eq!(result.remote_path, "/");
    }

    // -- parse: chained ------------------------------------------------------

    #[test]
    fn parse_two_hops() {
        let result = parse("/ssh:jumpbox|ssh:myvm:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops.len(), 2);
        assert_eq!(result.hops[0].backend, BackendKind::Ssh);
        assert_eq!(result.hops[0].host, "jumpbox");
        assert_eq!(result.hops[1].backend, BackendKind::Ssh);
        assert_eq!(result.hops[1].host, "myvm");
        assert_eq!(result.remote_path, "/etc/config");
    }

    #[test]
    fn parse_mixed_chain() {
        let result = parse("/ssh:myvm|docker:mycontainer:/app/config.toml")
            .unwrap()
            .unwrap();
        assert_eq!(result.hops.len(), 2);
        assert_eq!(result.hops[0].backend, BackendKind::Ssh);
        assert_eq!(result.hops[0].host, "myvm");
        assert_eq!(result.hops[1].backend, BackendKind::Docker);
        assert_eq!(result.hops[1].host, "mycontainer");
        assert_eq!(result.remote_path, "/app/config.toml");
    }

    #[test]
    fn parse_three_hops() {
        let result = parse("/ssh:jumpbox|ssh:myvm|sudo:root:/etc/shadow")
            .unwrap()
            .unwrap();
        assert_eq!(result.hops.len(), 3);
        assert_eq!(result.hops[2].backend, BackendKind::Sudo);
        assert_eq!(result.hops[2].host, "root");
    }

    // -- parse: case insensitivity -------------------------------------------

    #[test]
    fn parse_case_insensitive_backend() {
        let result = parse("/SSH:myvm:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops[0].backend, BackendKind::Ssh);
    }

    #[test]
    fn parse_kubernetes_alias() {
        let result = parse("/kubernetes:mypod:/tmp").unwrap().unwrap();
        assert_eq!(result.hops[0].backend, BackendKind::Kubernetes);
    }

    // -- parse: non-tramp paths return None -----------------------------------

    #[test]
    fn parse_normal_path_returns_none() {
        assert!(parse("/etc/config").unwrap().is_none());
        assert!(parse("/home/user/.config").unwrap().is_none());
        assert!(parse("relative/path").unwrap().is_none());
    }

    // -- parse: errors -------------------------------------------------------

    #[test]
    fn parse_missing_remote_path() {
        let result = parse("/ssh:myvm");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TrampError::MissingRemotePath));
    }

    #[test]
    fn parse_missing_host() {
        let result = parse("/ssh::/etc/config");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TrampError::MissingHost));
    }

    #[test]
    fn parse_invalid_port() {
        let result = parse("/ssh:myvm#notaport:/etc/config");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TrampError::InvalidPort(_)));
    }

    #[test]
    fn parse_empty_user_treated_as_none() {
        let result = parse("/ssh:@myvm:/etc/config").unwrap().unwrap();
        assert_eq!(result.hops[0].user, None);
        assert_eq!(result.hops[0].host, "myvm");
    }

    // -- round-trip ----------------------------------------------------------

    #[test]
    fn round_trip_simple() {
        let original = "/ssh:myvm:/etc/config";
        let parsed = parse(original).unwrap().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_with_user() {
        let original = "/ssh:admin@myvm:/etc/config";
        let parsed = parse(original).unwrap().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_with_user_and_port() {
        let original = "/ssh:admin@myvm#2222:/etc/config";
        let parsed = parse(original).unwrap().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_chained() {
        let original = "/ssh:jumpbox|ssh:myvm:/etc/config";
        let parsed = parse(original).unwrap().unwrap();
        assert_eq!(parsed.to_string(), original);
    }

    #[test]
    fn round_trip_complex_chain() {
        let original = "/ssh:admin@jumpbox#2222|docker:mycontainer:/app/config.toml";
        let parsed = parse(original).unwrap().unwrap();
        assert_eq!(parsed.to_string(), original);
    }
}
