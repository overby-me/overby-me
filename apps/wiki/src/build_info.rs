//! Which build this is.
//!
//! `CARGO_PKG_VERSION` is `0.1.0` for every build ever made, so on its own it
//! cannot tie a crash report or a piece of feedback back to the code that
//! produced it. The commit can, and it is baked in at build time: the justfile
//! resolves it and passes `GIT_COMMIT` down to cargo (see also `default.nix`,
//! which does the same for the Nix build).
//!
//! A build made without it says `unknown` rather than guessing — a wrong commit
//! on a report is worse than none, since it would send a reader to code that was
//! never running.

/// The commit the bundle was built from (short form), or `"unknown"`.
pub const COMMIT: &str = match option_env!("GIT_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};
