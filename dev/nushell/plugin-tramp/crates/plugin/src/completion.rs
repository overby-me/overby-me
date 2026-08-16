//! Tab completion for TRAMP paths.
//!
//! Provides dynamic completions for all commands that accept TRAMP URI
//! arguments.  The completion logic handles three stages:
//!
//! 1. **Backend prefix** — when the user has typed `/` or `/ss…`, suggest
//!    known backend prefixes like `/ssh:`, `/docker:`, `/k8s:`, `/sudo:`.
//!
//! 2. **Host** — when the user has typed `/ssh:` (backend + colon, but no
//!    `:/` yet), suggest host names from `~/.ssh/config` and active VFS
//!    connections.
//!
//! 3. **Remote path** — when the user has typed `/ssh:host:/some/pa…`,
//!    list the parent directory on the remote and offer matching entries.

use std::sync::Mutex;

use nu_protocol::{DynamicSuggestion, Span, SuggestionKind, ast};

use crate::backend::EntryKind;
use crate::protocol::TrampPath;
use crate::vfs::Vfs;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Produce completions for a positional argument that is expected to be a
/// TRAMP path.
///
/// `partial` is the string the user has typed so far for the argument.
/// Returns `None` to fall back to default Nushell completions (e.g. when
/// the input clearly isn't a TRAMP path).
pub fn complete_tramp_path(
    vfs: &Vfs,
    remote_cwd: &Mutex<Option<TrampPath>>,
    partial: &str,
    span: Span,
) -> Option<Vec<DynamicSuggestion>> {
    // If the input doesn't start with '/' at all, check for a remote CWD.
    // If we have one, treat it as a relative remote path.
    if !partial.starts_with('/') {
        return complete_relative_path(vfs, remote_cwd, partial, span);
    }

    // Determine which stage we are in.
    let inner = &partial[1..]; // strip leading '/'

    // If there's no colon yet, we're still completing the backend name.
    let Some(first_colon) = inner.find(':') else {
        return Some(complete_backend_prefix(partial, span));
    };

    // Extract the first backend name (before any '|' chain separator).
    let before_colon = &inner[..first_colon];
    let first_segment = before_colon.split('|').next().unwrap_or(before_colon);

    // Validate that the first segment is a known backend.
    if !is_known_backend(first_segment) {
        return None; // not a TRAMP path
    }

    // Check whether we have a `:/' marking the start of the remote path.
    if let Some(remote_sep) = inner.find(":/") {
        // We have backend + host + at least the start of a remote path.
        let hops_str = &inner[..remote_sep];
        let remote_partial = &inner[remote_sep + 1..]; // includes leading '/'
        return complete_remote_path(vfs, hops_str, remote_partial, span);
    }

    // We have a colon but no ':/' — could be:
    //   /ssh:          — need host
    //   /ssh:myho      — partial host
    //   /ssh:host:     — host present but remote path not started (missing '/')
    //   /ssh:host|docker:   — chained, need host for last hop

    // Check if there's a trailing ':' that isn't part of ':/'.
    // This means the user typed e.g. `/ssh:host:` but hasn't typed '/' yet.
    if partial.ends_with(':') && !partial.ends_with(":/") {
        // Count colons — if there are at least 2 in inner (one for backend,
        // one for the trailing), it means "backend:host:" so suggest '/'.
        let colon_count = inner.chars().filter(|&c| c == ':').count();
        if colon_count >= 2 {
            let suggestion = DynamicSuggestion {
                value: format!("{partial}/"),
                description: Some("start remote path".into()),
                append_whitespace: false,
                kind: Some(SuggestionKind::Directory),
                span: Some(span),
                ..Default::default()
            };
            return Some(vec![suggestion]);
        }
    }

    // Otherwise we're completing a host name.
    Some(complete_host(vfs, partial, inner, span))
}

/// Extract a partial argument string from the AST call at a given positional
/// index.
///
/// Returns `None` if the argument can't be extracted as a string.
pub fn extract_positional_string(call: &ast::Call, idx: usize, strip: bool) -> Option<String> {
    let expr = call.positional_nth(idx)?;
    let s = match &expr.expr {
        ast::Expr::String(s) => s.clone(),
        ast::Expr::Filepath(s, _) => s.clone(),
        ast::Expr::Directory(s, _) => s.clone(),
        ast::Expr::GlobPattern(s, _) => s.clone(),
        // For garbage expressions (incomplete parse), try to return empty
        // so we can still offer backend-prefix completions.
        ast::Expr::Garbage => String::new(),
        _ => return None,
    };

    if strip && !s.is_empty() {
        // The parser may have appended a placeholder character.  Strip it.
        let mut chars: Vec<char> = s.chars().collect();
        chars.pop();
        Some(chars.into_iter().collect())
    } else {
        Some(s)
    }
}

/// Return the span of a positional argument expression, if available.
pub fn positional_span(call: &ast::Call, idx: usize) -> Option<Span> {
    call.positional_nth(idx).map(|e| e.span)
}

// ---------------------------------------------------------------------------
// Backend prefix completion
// ---------------------------------------------------------------------------

/// All known backend names and their display descriptions.
const BACKENDS: &[(&str, &str)] = &[
    ("ssh", "SSH remote host"),
    ("docker", "Docker container"),
    ("k8s", "Kubernetes pod"),
    ("sudo", "Sudo / privilege escalation"),
];

fn is_known_backend(name: &str) -> bool {
    BACKENDS.iter().any(|(b, _)| b.eq_ignore_ascii_case(name))
}

/// Complete backend prefixes like `/ssh:`, `/docker:`, etc.
fn complete_backend_prefix(partial: &str, span: Span) -> Vec<DynamicSuggestion> {
    let mut suggestions = Vec::new();
    for &(backend, desc) in BACKENDS {
        let candidate = format!("/{backend}:");
        if candidate.starts_with(partial) {
            suggestions.push(DynamicSuggestion {
                value: candidate,
                description: Some(desc.into()),
                append_whitespace: false,
                kind: Some(SuggestionKind::Flag),
                span: Some(span),
                ..Default::default()
            });
        }
    }
    suggestions
}

// ---------------------------------------------------------------------------
// Host completion
// ---------------------------------------------------------------------------

/// Complete host names from SSH config and active connections.
fn complete_host(vfs: &Vfs, full_partial: &str, inner: &str, span: Span) -> Vec<DynamicSuggestion> {
    // Determine the hop prefix up to the last '|' (for chained paths).
    // Then find what backend the current hop uses and the partial host.
    let (prefix_before_last_hop, last_hop_str) = if let Some(pipe_idx) = inner.rfind('|') {
        // +2 accounts for the leading '/' and the '|' itself
        (&full_partial[..pipe_idx + 2], &inner[pipe_idx + 1..])
    } else {
        ("/", inner)
    };

    // last_hop_str looks like "ssh:partial_host" or "docker:partial"
    let (backend_str, partial_host) = match last_hop_str.split_once(':') {
        Some((b, h)) => (b, h),
        None => return vec![],
    };

    // Build the prefix that each suggestion will start with.
    let suggestion_prefix = if prefix_before_last_hop == "/" {
        format!("/{backend_str}:")
    } else {
        format!("{prefix_before_last_hop}{backend_str}:")
    };

    let mut hosts: Vec<String> = Vec::new();

    // 1. Gather hosts from active VFS connections.
    for info in vfs.active_connections_detailed() {
        if !hosts.contains(&info.host) {
            hosts.push(info.host.clone());
        }
    }

    // 2. Parse ~/.ssh/config for Host entries (SSH backend only).
    if backend_str.eq_ignore_ascii_case("ssh")
        && let Some(ssh_hosts) = parse_ssh_config_hosts()
    {
        for h in ssh_hosts {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }

    // 3. For docker, try to list running containers.
    if backend_str.eq_ignore_ascii_case("docker")
        && let Some(docker_hosts) = list_docker_containers()
    {
        for h in docker_hosts {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }

    // 4. For k8s/kubernetes, try to list pods.
    if (backend_str.eq_ignore_ascii_case("k8s") || backend_str.eq_ignore_ascii_case("kubernetes"))
        && let Some(k8s_pods) = list_k8s_pods()
    {
        for h in k8s_pods {
            if !hosts.contains(&h) {
                hosts.push(h);
            }
        }
    }

    // Filter hosts by partial match.
    let mut suggestions = Vec::new();
    for host in &hosts {
        if host.starts_with(partial_host) || partial_host.is_empty() {
            suggestions.push(DynamicSuggestion {
                value: format!("{suggestion_prefix}{host}:"),
                description: Some(format!("{backend_str} host")),
                append_whitespace: false,
                kind: Some(SuggestionKind::Value(nu_protocol::Type::String)),
                span: Some(span),
                ..Default::default()
            });
        }
    }

    suggestions.sort_by(|a, b| a.value.cmp(&b.value));
    suggestions
}

/// Parse `~/.ssh/config` and extract `Host` entries.
///
/// Skips wildcard patterns (containing `*` or `?`).
fn parse_ssh_config_hosts() -> Option<Vec<String>> {
    let home = dirs::home_dir()?;
    let config_path = home.join(".ssh").join("config");
    let contents = std::fs::read_to_string(config_path).ok()?;

    let mut hosts = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        // Match lines like "Host myserver" or "Host foo bar baz"
        if let Some(rest) = trimmed
            .strip_prefix("Host ")
            .or_else(|| trimmed.strip_prefix("Host\t"))
        {
            for entry in rest.split_whitespace() {
                // Skip wildcard patterns.
                if entry.contains('*') || entry.contains('?') {
                    continue;
                }
                let entry = entry.to_string();
                if !hosts.contains(&entry) {
                    hosts.push(entry);
                }
            }
        }
    }

    Some(hosts)
}

/// List running Docker container names (best-effort, returns None on failure).
fn list_docker_containers() -> Option<Vec<String>> {
    let output = std::process::Command::new("docker")
        .args(["ps", "--format", "{{.Names}}"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Some(containers)
}

/// List Kubernetes pod names in the current context (best-effort).
fn list_k8s_pods() -> Option<Vec<String>> {
    let output = std::process::Command::new("kubectl")
        .args([
            "get",
            "pods",
            "--no-headers",
            "-o",
            "custom-columns=:metadata.name",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let pods: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Some(pods)
}

// ---------------------------------------------------------------------------
// Remote path completion
// ---------------------------------------------------------------------------

/// Complete a remote file path after the TRAMP prefix has been parsed.
///
/// `hops_str` is e.g. `"ssh:myvm"` or `"ssh:jumpbox|docker:ctr"`.
/// `remote_partial` is the partial remote path including the leading `/`,
/// e.g. `"/etc/hos"` or `"/home/"`.
fn complete_remote_path(
    vfs: &Vfs,
    hops_str: &str,
    remote_partial: &str,
    span: Span,
) -> Option<Vec<DynamicSuggestion>> {
    // Split the remote partial into parent directory and partial name.
    let (parent_dir, partial_name) = match remote_partial.rfind('/') {
        Some(idx) => (&remote_partial[..=idx], &remote_partial[idx + 1..]),
        None => ("/", remote_partial),
    };

    // Reconstruct a valid TRAMP URI pointing at the parent directory so we
    // can call VFS list.
    let tramp_uri = format!("/{hops_str}:{parent_dir}");

    let tramp_path = match crate::protocol::parse(&tramp_uri) {
        Ok(Some(p)) => p,
        _ => return None,
    };

    // List the parent directory.  If the connection doesn't exist yet or
    // the listing fails, just return no suggestions rather than blocking.
    let entries = match vfs.list(&tramp_path) {
        Ok(e) => e,
        Err(_) => return Some(vec![]),
    };

    let tramp_prefix = format!("/{hops_str}:");

    let mut suggestions = Vec::new();
    for entry in &entries {
        if !partial_name.is_empty() && !entry.name.starts_with(partial_name) {
            continue;
        }

        let is_dir = entry.kind == EntryKind::Dir;

        // Build the full completed path.
        let completed_remote = if parent_dir.ends_with('/') {
            format!("{parent_dir}{}", entry.name)
        } else {
            format!("{parent_dir}/{}", entry.name)
        };

        // For directories, append '/' so the user can keep typing.
        let value = if is_dir {
            format!("{tramp_prefix}{completed_remote}/")
        } else {
            format!("{tramp_prefix}{completed_remote}")
        };

        let kind = if is_dir {
            SuggestionKind::Directory
        } else {
            SuggestionKind::File
        };

        let desc = if is_dir {
            Some("dir".into())
        } else {
            entry.size.map(format_size)
        };

        suggestions.push(DynamicSuggestion {
            value,
            description: desc,
            append_whitespace: !is_dir,
            kind: Some(kind),
            span: Some(span),
            ..Default::default()
        });
    }

    suggestions.sort_by(|a, b| {
        // Sort directories first, then files.
        let a_dir = a.kind == Some(SuggestionKind::Directory);
        let b_dir = b.kind == Some(SuggestionKind::Directory);
        b_dir.cmp(&a_dir).then(a.value.cmp(&b.value))
    });

    Some(suggestions)
}

// ---------------------------------------------------------------------------
// Relative path completion (when a remote CWD is set)
// ---------------------------------------------------------------------------

/// Complete a relative path when a remote CWD is active.
fn complete_relative_path(
    vfs: &Vfs,
    remote_cwd: &Mutex<Option<TrampPath>>,
    partial: &str,
    span: Span,
) -> Option<Vec<DynamicSuggestion>> {
    let cwd_guard = remote_cwd.lock().ok()?;
    let cwd = cwd_guard.as_ref()?;

    // Resolve the partial relative path against the CWD.
    let (parent_relative, partial_name) = match partial.rfind('/') {
        Some(idx) => (&partial[..=idx], &partial[idx + 1..]),
        None => ("", partial),
    };

    // Build the absolute remote parent by resolving against CWD.
    let parent_remote = if parent_relative.is_empty() {
        cwd.remote_path.clone()
    } else {
        crate::resolve_relative(&cwd.remote_path, parent_relative)
    };

    // Construct a TRAMP path for listing.
    let mut list_path = cwd.clone();
    list_path.remote_path = parent_remote;

    let entries = match vfs.list(&list_path) {
        Ok(e) => e,
        Err(_) => return Some(vec![]),
    };

    let mut suggestions = Vec::new();
    for entry in &entries {
        if !partial_name.is_empty() && !entry.name.starts_with(partial_name) {
            continue;
        }

        let is_dir = entry.kind == EntryKind::Dir;

        // For relative completions, the value is just the relative path.
        let value = if parent_relative.is_empty() {
            if is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            }
        } else if is_dir {
            format!("{}{}/", parent_relative, entry.name)
        } else {
            format!("{}{}", parent_relative, entry.name)
        };

        let kind = if is_dir {
            SuggestionKind::Directory
        } else {
            SuggestionKind::File
        };

        let desc = if is_dir {
            Some("dir".into())
        } else {
            entry.size.map(format_size)
        };

        suggestions.push(DynamicSuggestion {
            value,
            description: desc,
            append_whitespace: !is_dir,
            kind: Some(kind),
            span: Some(span),
            ..Default::default()
        });
    }

    suggestions.sort_by(|a, b| {
        let a_dir = a.kind == Some(SuggestionKind::Directory);
        let b_dir = b.kind == Some(SuggestionKind::Directory);
        b_dir.cmp(&a_dir).then(a.value.cmp(&b.value))
    });

    Some(suggestions)
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Format a byte size into a human-readable string.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_prefix_from_slash() {
        let span = Span::test_data();
        let results = complete_backend_prefix("/", span);
        assert_eq!(results.len(), 4);
        let values: Vec<&str> = results.iter().map(|s| s.value.as_str()).collect();
        assert!(values.contains(&"/ssh:"));
        assert!(values.contains(&"/docker:"));
        assert!(values.contains(&"/k8s:"));
        assert!(values.contains(&"/sudo:"));
    }

    #[test]
    fn backend_prefix_partial_ssh() {
        let span = Span::test_data();
        let results = complete_backend_prefix("/ss", span);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "/ssh:");
    }

    #[test]
    fn backend_prefix_partial_d() {
        let span = Span::test_data();
        let results = complete_backend_prefix("/d", span);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "/docker:");
    }

    #[test]
    fn backend_prefix_partial_s() {
        let span = Span::test_data();
        let results = complete_backend_prefix("/s", span);
        assert_eq!(results.len(), 2); // ssh and sudo
        let values: Vec<&str> = results.iter().map(|s| s.value.as_str()).collect();
        assert!(values.contains(&"/ssh:"));
        assert!(values.contains(&"/sudo:"));
    }

    #[test]
    fn backend_prefix_no_match() {
        let span = Span::test_data();
        let results = complete_backend_prefix("/xyz", span);
        assert!(results.is_empty());
    }

    #[test]
    fn is_known_backend_checks() {
        assert!(is_known_backend("ssh"));
        assert!(is_known_backend("SSH"));
        assert!(is_known_backend("docker"));
        assert!(is_known_backend("k8s"));
        assert!(is_known_backend("sudo"));
        assert!(!is_known_backend("ftp"));
        assert!(!is_known_backend(""));
    }

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(42), "42 B");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(2048), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn parse_ssh_config_hosts_works() {
        // Just verify it doesn't panic — actual results depend on system.
        let _result = parse_ssh_config_hosts();
    }

    #[test]
    fn extract_positional_string_from_string_expr() {
        let mut call = ast::Call::new(Span::test_data());
        call.add_positional(nu_protocol::ast::Expression {
            expr: ast::Expr::String("/ssh:host:/etc".into()),
            span: Span::test_data(),
            span_id: nu_protocol::SpanId::new(0),
            ty: nu_protocol::Type::String,
        });

        let result = extract_positional_string(&call, 0, false);
        assert_eq!(result, Some("/ssh:host:/etc".into()));
    }

    #[test]
    fn extract_positional_string_with_strip() {
        let mut call = ast::Call::new(Span::test_data());
        call.add_positional(nu_protocol::ast::Expression {
            expr: ast::Expr::String("/ssh:host:/etc\0".into()),
            span: Span::test_data(),
            span_id: nu_protocol::SpanId::new(0),
            ty: nu_protocol::Type::String,
        });

        let result = extract_positional_string(&call, 0, true);
        assert_eq!(result, Some("/ssh:host:/etc".into()));
    }

    #[test]
    fn extract_positional_string_out_of_range() {
        let call = ast::Call::new(Span::test_data());
        assert_eq!(extract_positional_string(&call, 0, false), None);
    }

    #[test]
    fn complete_tramp_path_returns_none_for_non_tramp() {
        let vfs = Vfs::new().unwrap();
        let cwd = Mutex::new(None);
        let span = Span::test_data();

        // Without a remote CWD, non-TRAMP paths return None.
        let result = complete_tramp_path(&vfs, &cwd, "some_file.txt", span);
        assert!(result.is_none());
    }

    #[test]
    fn complete_tramp_path_backend_prefix() {
        let vfs = Vfs::new().unwrap();
        let cwd = Mutex::new(None);
        let span = Span::test_data();

        let result = complete_tramp_path(&vfs, &cwd, "/", span);
        assert!(result.is_some());
        let suggestions = result.unwrap();
        assert_eq!(suggestions.len(), 4);
    }

    #[test]
    fn complete_tramp_path_partial_backend() {
        let vfs = Vfs::new().unwrap();
        let cwd = Mutex::new(None);
        let span = Span::test_data();

        let result = complete_tramp_path(&vfs, &cwd, "/doc", span);
        assert!(result.is_some());
        let suggestions = result.unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].value, "/docker:");
    }

    #[test]
    fn complete_tramp_path_host_colon_suggests_slash() {
        let vfs = Vfs::new().unwrap();
        let cwd = Mutex::new(None);
        let span = Span::test_data();

        let result = complete_tramp_path(&vfs, &cwd, "/ssh:myhost:", span);
        assert!(result.is_some());
        let suggestions = result.unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].value, "/ssh:myhost:/");
    }
}
