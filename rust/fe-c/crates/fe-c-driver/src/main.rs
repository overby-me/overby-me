//! `fe-c-driver`: rustc-as-a-library (rustc_public where possible).
//!
//! Task A5 gives this binary its census mode; B2 adds MIR rewriting. Until
//! then it only identifies itself, so the workspace and nix wiring can be
//! validated end to end.

fn main() {
    eprintln!(
        "fe-c-driver {}: MIR census and rewriting not yet implemented (Task A5)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2);
}
