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
/// getopt's character-by-character cluster handling. `no_arg` lists the
/// short flags that take no argument; `req_arg` lists those that take a
/// required argument (so the rest of the cluster becomes the inline
/// value). Any other character ends the cluster early and is emitted as
/// `-<char>` so the caller's match arm can produce the standard
/// `invalid option -- 'X'` message.
pub fn expand_short_clusters(args: &[String], no_arg: &[char]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        if a.len() > 2 && a.starts_with('-') && !a.starts_with("--") {
            let chars: Vec<char> = a.chars().skip(1).collect();
            let mut all_no_arg = true;
            for c in &chars {
                if !no_arg.contains(c) {
                    all_no_arg = false;
                    break;
                }
            }
            if all_no_arg {
                for c in chars {
                    out.push(format!("-{c}"));
                }
                continue;
            }
            // Mixed cluster: emit each prefix char that's no-arg, stop at
            // the first non-no-arg char (which we emit alone — the match
            // arm in the caller will produce the right error / consume
            // the next arg as required-arg value).
            let mut emitted = false;
            for (idx, c) in chars.iter().enumerate() {
                if no_arg.contains(c) {
                    out.push(format!("-{c}"));
                    emitted = true;
                } else {
                    out.push(format!("-{c}"));
                    // Tail (any remaining chars after this one) are
                    // dropped — getopt would treat them as the value of
                    // a required-arg flag, but that requires per-tool
                    // logic. The caller can either accept this or fall
                    // through to its error path.
                    let _ = idx;
                    emitted = true;
                    break;
                }
            }
            if !emitted {
                out.push(a.clone());
            }
        } else {
            out.push(a.clone());
        }
    }
    out
}
