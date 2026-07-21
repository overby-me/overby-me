//! `cargo-fe-c`: the cargo subcommand front end.
//!
//! Grows the `RUSTC_WRAPPER` orchestration once the driver can instrument
//! (Task B2 onward). Until then it only identifies itself, so the workspace
//! and nix wiring can be validated end to end.

fn main() {
    eprintln!(
        "cargo-fe-c {}: instrumented builds not yet implemented (Tasks A5/B2)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2);
}
