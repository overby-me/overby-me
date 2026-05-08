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
