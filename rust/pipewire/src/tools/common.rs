// Shared helpers for the pw-* CLI tools.

use crate::pipewire_lib::version::PIPEWIRE_API_VERSION;

/// Map a `connect_default()` failure's I/O error to the strerror string
/// libpipewire's pw_protocol_native uses. ENOENT → "Host is down",
/// ECONNREFUSED → "Connection refused", everything else → native
/// to_string().
pub fn connect_failure_msg() -> String {
    // Try probing the default socket once to find out *why* connect
    // failed; we accept that this is a second attempt vs. plumbing the
    // original error through, but every tool that needs this only calls
    // connect_default() once and returns immediately on failure.
    let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
        .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let core = std::env::var("PIPEWIRE_REMOTE")
        .or_else(|_| std::env::var("PIPEWIRE_CORE"))
        .unwrap_or_else(|_| "pipewire-0".to_string());
    let path = if core.starts_with('/') {
        std::path::PathBuf::from(&core)
    } else {
        std::path::PathBuf::from(runtime).join(core)
    };
    match std::os::unix::net::UnixStream::connect(&path) {
        Ok(_) => "Host is down".to_string(),
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => "Host is down".to_string(),
            std::io::ErrorKind::ConnectionRefused => "Connection refused".to_string(),
            _ => e.to_string(),
        },
    }
}

/// Mirror the C tools' `--version` output:
///
/// ```text
/// <argv0>
/// Compiled with libpipewire <version>
/// Linked with libpipewire <version>
/// ```
pub fn print_version(argv0: &str) {
    println!("{argv0}");
    println!("Compiled with libpipewire {PIPEWIRE_API_VERSION}");
    println!("Linked with libpipewire {PIPEWIRE_API_VERSION}");
}

/// Expand short-option clusters like `-hV` → [`-h`, `-V`], emulating
/// getopt's character-by-character cluster handling.
///
/// Splits the cluster only if the FIRST char is in `no_arg`. After
/// that first char, keep splitting char-by-char until we hit a char
/// NOT in `no_arg`; emit that char alone (the caller's match handles
/// it) and stop. This lets `-hxx` short-circuit at -h (which prints
/// help and exits) before -x triggers an error, while leaving
/// required-arg clusters like `-pfoo` intact for inline-value handling.
pub fn expand_short_clusters(args: &[String], no_arg: &[char]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if a.len() > 2 && a.starts_with('-') && !a.starts_with("--") {
            let chars: Vec<char> = a.chars().skip(1).collect();
            // Only enter cluster-split mode if the first char is no-arg.
            if !chars.first().map(|c| no_arg.contains(c)).unwrap_or(false) {
                out.push(a.clone());
                continue;
            }
            for c in chars.iter() {
                out.push(format!("-{c}"));
                if !no_arg.contains(c) {
                    // Stop here; the caller's match handles this char.
                    // Drop the rest of the cluster.
                    break;
                }
            }
        } else {
            out.push(a.clone());
        }
    }
    out
}
