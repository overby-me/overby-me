// Shared helpers for the pw-* CLI tools.

use crate::pipewire_lib::version::PIPEWIRE_API_VERSION;

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
/// Only splits clusters whose every character appears in `no_arg`. If
/// any character is unknown or required-arg, the cluster is left intact
/// so the caller can produce the usual error or consume the rest as an
/// inline value.
pub fn expand_short_clusters(args: &[String], no_arg: &[char]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if a.len() > 2
            && a.starts_with('-')
            && !a.starts_with("--")
            && a.chars().skip(1).all(|c| no_arg.contains(&c))
        {
            for c in a.chars().skip(1) {
                out.push(format!("-{c}"));
            }
        } else {
            out.push(a.clone());
        }
    }
    out
}
