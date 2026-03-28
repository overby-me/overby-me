//! Configuration for rust-systemd.
//!
//! As a drop-in replacement for systemd, rust-systemd uses the same well-known
//! default paths that systemd uses.  There is no config file and no
//! rust-systemd-specific environment variables — unit directories, the default
//! target, and all other settings match systemd's compiled-in defaults.
//!
//! In addition to the standard system-wide unit directories, rust-systemd
//! discovers its own package's `lib/systemd/system/` directory by walking
//! up from its executable path.  This mirrors what systemd does with its
//! compile-time `rootlibdir` setting (e.g. on NixOS the systemd package
//! searches its own store path for shipped unit files).

use log::{info, trace};
use std::path::PathBuf;

#[derive(Debug)]
pub struct LoggingConfig {
    pub log_to_stdout: bool,
    pub log_to_disk: bool,
    pub log_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub unit_dirs: Vec<PathBuf>,
    pub target_unit: String,
    pub notification_sockets_dir: PathBuf,
    pub self_path: PathBuf,
}

/// Well-known systemd unit search directories (system instance), in priority
/// order.  This matches the paths systemd itself searches.
const SYSTEM_UNIT_DIRS: &[&str] = &[
    "/run/systemd/transient",
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/local/lib/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
];

/// Try to find a `lib/systemd/system/` directory that belongs to the same
/// package / prefix as the running executable.  This mirrors systemd's
/// compile-time `rootlibdir` — on NixOS the systemd package includes its
/// own store path as a unit search directory so that upstream-shipped targets
/// (e.g. `time-set.target`, `time-sync.target`) are found.
///
/// We walk up from the executable's directory (at most 5 levels) and check
/// whether `<ancestor>/lib/systemd/system` exists.  This handles both
/// `$out/bin/rust-systemd` and `$out/lib/systemd/systemd` layouts.
fn package_unit_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..5 {
        let candidate = dir.join("lib/systemd/system");
        if candidate.is_dir() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// Augment the process's `PATH` environment variable with binary directories
/// derived from the unit search paths and the running executable.
///
/// Many upstream systemd unit files use bare command names in `ExecStart=`
/// (e.g. `systemd-tmpfiles`, `udevadm`).  Real systemd resolves these via
/// compiled-in prefix paths.  rust-systemd instead adds the relevant package
/// directories to `PATH` so that both `which` look-ups and `Command::new`
/// find the binaries.
///
/// Directories added (if they exist):
/// - For each unit dir of the form `<root>/lib/systemd/system`, add
///   `<root>/bin` and `<root>/lib/systemd` (libexec-style helpers).
/// - For each unit dir of the form `<root>/etc/systemd/system`, add
///   `<root>/bin` and `<root>/lib/systemd`.
/// - The NixOS well-known path `/run/current-system/sw/bin`.
pub fn augment_path_from_unit_dirs(unit_dirs: &[PathBuf]) {
    let current_path = std::env::var("PATH").unwrap_or_default();
    let mut extra_dirs: Vec<PathBuf> = Vec::new();

    for unit_dir in unit_dirs {
        // Walk up from the unit directory to find the package root.
        // Typical layouts:
        //   <root>/lib/systemd/system   → root is 3 levels up
        //   <root>/etc/systemd/system   → root is 3 levels up
        //   <root>/example/systemd/system → root is 3 levels up
        if let Some(root) = unit_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let bin_dir = root.join("bin");
            if bin_dir.is_dir() && !extra_dirs.contains(&bin_dir) {
                extra_dirs.push(bin_dir);
            }
            // lib/systemd/ contains libexec-style helpers like systemd-udevd,
            // systemd-journald, systemd-logind, etc.
            let libexec_dir = root.join("lib/systemd");
            if libexec_dir.is_dir() && !extra_dirs.contains(&libexec_dir) {
                extra_dirs.push(libexec_dir);
            }
        }
    }

    // NixOS well-known system binary path (created by activation script
    // before the service manager starts).
    let nixos_sw_bin = PathBuf::from("/run/current-system/sw/bin");
    if nixos_sw_bin.is_dir() && !extra_dirs.contains(&nixos_sw_bin) {
        extra_dirs.push(nixos_sw_bin);
    }

    // NixOS suid wrappers path.
    let nixos_wrappers = PathBuf::from("/run/wrappers/bin");
    if nixos_wrappers.is_dir() && !extra_dirs.contains(&nixos_wrappers) {
        extra_dirs.push(nixos_wrappers);
    }

    if extra_dirs.is_empty() {
        return;
    }

    // Filter out directories already present in PATH.
    let existing: Vec<&str> = current_path.split(':').collect();
    let new_dirs: Vec<String> = extra_dirs
        .iter()
        .filter(|d| {
            let s = d.to_string_lossy();
            !existing.iter().any(|e| *e == s.as_ref())
        })
        .map(|d| d.to_string_lossy().into_owned())
        .collect();

    if new_dirs.is_empty() {
        return;
    }

    let new_path = if current_path.is_empty() {
        new_dirs.join(":")
    } else {
        format!("{}:{}", current_path, new_dirs.join(":"))
    };

    trace!(
        "Augmenting PATH with package directories: {:?} → PATH={}",
        new_dirs, new_path
    );
    // SAFETY: This is called early in startup, before any other threads are
    // spawned, so modifying the environment is safe.
    unsafe { std::env::set_var("PATH", &new_path) };
}

/// Parse the kernel command line from `/proc/cmdline` and return a target
/// unit override if one is specified.
///
/// Supports the following mechanisms (in priority order):
///
/// 1. `systemd.unit=<target>` — explicit target override (highest priority)
/// 2. `emergency` — equivalent to `systemd.unit=emergency.target`
/// 3. `rescue`, `single`, `s`, `S` — equivalent to `systemd.unit=rescue.target`
/// 4. `1` — SysV runlevel 1 (rescue mode)
/// 5. `2`, `3`, `4`, `5` — SysV runlevels mapped to `multi-user.target` (3)
///    or `graphical.target` (5); 2 and 4 map to `multi-user.target`
///
/// See `systemd(1)` and `kernel-command-line(7)` for details.
///
/// This function reads from `/proc/cmdline`.  If the file cannot be read
/// (e.g. in a test environment), `None` is returned.
pub fn target_unit_from_kernel_cmdline() -> Option<String> {
    target_unit_from_cmdline_str(&std::fs::read_to_string("/proc/cmdline").ok()?)
}

/// Inner implementation that works on an already-read command line string.
/// Separated for testability.
fn target_unit_from_cmdline_str(cmdline: &str) -> Option<String> {
    // `systemd.unit=` takes highest priority — use the last occurrence
    // (matching systemd behaviour where later parameters override earlier).
    let mut explicit_target: Option<String> = None;
    // SysV compat keywords are only used when no explicit `systemd.unit=`
    // is present.
    let mut sysv_target: Option<&str> = None;

    for param in cmdline.split_whitespace() {
        if let Some(unit) = param.strip_prefix("systemd.unit=") {
            if !unit.is_empty() {
                explicit_target = Some(unit.to_owned());
            }
        } else if let Some(unit) = param.strip_prefix("rd.systemd.unit=") {
            // rd.systemd.unit= is for initrd only — ignore in the real root,
            // but we still parse it so we don't fall through to SysV compat.
            let _ = unit;
        } else {
            // SysV compatibility keywords
            match param {
                "emergency" => sysv_target = Some("emergency.target"),
                "rescue" | "single" | "s" | "S" => sysv_target = Some("rescue.target"),
                "-s" | "-S" => sysv_target = Some("rescue.target"),
                "1" => sysv_target = Some("rescue.target"),
                "2" | "3" | "4" => sysv_target = Some("multi-user.target"),
                "5" => sysv_target = Some("graphical.target"),
                _ => {}
            }
        }
    }

    // Explicit systemd.unit= always wins over SysV keywords.
    if let Some(target) = explicit_target {
        info!("Kernel command line: systemd.unit={target}");
        Some(target)
    } else if let Some(target) = sysv_target {
        info!("Kernel command line: SysV compat keyword → {target}");
        Some(target.to_owned())
    } else {
        None
    }
}

pub fn load_config() -> (LoggingConfig, Config) {
    // Collect unit directories: standard system paths + package-local dir.
    // Only include directories that actually exist on this system.
    let mut unit_dirs: Vec<PathBuf> = SYSTEM_UNIT_DIRS
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect();

    if let Some(pkg_dir) = package_unit_dir()
        && !unit_dirs.contains(&pkg_dir)
    {
        unit_dirs.push(pkg_dir);
    }

    let self_path = std::env::current_exe().expect("Could not determine own executable path");

    // Determine the boot target:
    //   1. Kernel command line override (systemd.unit=, emergency, rescue, single, …)
    //   2. default.target (the standard systemd default)
    let target_unit =
        target_unit_from_kernel_cmdline().unwrap_or_else(|| "default.target".to_owned());

    let config = Config {
        unit_dirs,
        target_unit,
        notification_sockets_dir: PathBuf::from("/run/systemd/rust-systemd-notify"),
        self_path,
    };

    let logging_config = LoggingConfig {
        log_to_stdout: true,
        log_to_disk: false,
        log_dir: PathBuf::from("/var/log/rust-systemd"),
    };

    (logging_config, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── target_unit_from_cmdline_str tests ───────────────────────────────

    #[test]
    fn test_no_override_returns_none() {
        assert_eq!(target_unit_from_cmdline_str("quiet splash"), None);
    }

    #[test]
    fn test_empty_cmdline_returns_none() {
        assert_eq!(target_unit_from_cmdline_str(""), None);
    }

    #[test]
    fn test_systemd_unit_explicit() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet systemd.unit=emergency.target"),
            Some("emergency.target".to_owned()),
        );
    }

    #[test]
    fn test_systemd_unit_last_wins() {
        assert_eq!(
            target_unit_from_cmdline_str(
                "systemd.unit=rescue.target quiet systemd.unit=multi-user.target"
            ),
            Some("multi-user.target".to_owned()),
        );
    }

    #[test]
    fn test_systemd_unit_empty_value_ignored() {
        // systemd.unit= with no value should not override — fall through
        assert_eq!(target_unit_from_cmdline_str("systemd.unit="), None);
    }

    #[test]
    fn test_emergency_keyword() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet emergency"),
            Some("emergency.target".to_owned()),
        );
    }

    #[test]
    fn test_rescue_keyword() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet rescue"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_single_keyword() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet single"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_lowercase_s() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet s"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_uppercase_s() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet S"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_dash_s() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet -s"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_runlevel_1() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet 1"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_runlevel_3() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet 3"),
            Some("multi-user.target".to_owned()),
        );
    }

    #[test]
    fn test_runlevel_5() {
        assert_eq!(
            target_unit_from_cmdline_str("quiet 5"),
            Some("graphical.target".to_owned()),
        );
    }

    #[test]
    fn test_explicit_overrides_sysv() {
        // When both systemd.unit= and a SysV keyword are present,
        // the explicit parameter wins.
        assert_eq!(
            target_unit_from_cmdline_str("emergency systemd.unit=multi-user.target"),
            Some("multi-user.target".to_owned()),
        );
    }

    #[test]
    fn test_sysv_last_keyword_wins() {
        // Multiple SysV keywords — the last one wins (left-to-right parse).
        assert_eq!(
            target_unit_from_cmdline_str("single emergency"),
            Some("emergency.target".to_owned()),
        );
    }

    #[test]
    fn test_rd_systemd_unit_ignored_in_real_root() {
        // rd.systemd.unit= is for initrd only and should not set the target
        // in the real root filesystem.
        assert_eq!(
            target_unit_from_cmdline_str("rd.systemd.unit=initrd.target"),
            None,
        );
    }

    #[test]
    fn test_rd_systemd_unit_does_not_block_sysv() {
        // rd.systemd.unit= should not prevent SysV keywords from working
        assert_eq!(
            target_unit_from_cmdline_str("rd.systemd.unit=initrd.target rescue"),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_typical_nixos_cmdline() {
        // A realistic NixOS kernel command line — no target override expected
        let cmdline = "init=/nix/store/abc-rust-systemd/lib/systemd/systemd \
                        loglevel=4 console=ttyS0";
        assert_eq!(target_unit_from_cmdline_str(cmdline), None);
    }

    #[test]
    fn test_typical_emergency_cmdline() {
        let cmdline = "init=/nix/store/abc-rust-systemd/lib/systemd/systemd \
                        loglevel=4 console=ttyS0 systemd.unit=emergency.target";
        assert_eq!(
            target_unit_from_cmdline_str(cmdline),
            Some("emergency.target".to_owned()),
        );
    }

    #[test]
    fn test_custom_target() {
        assert_eq!(
            target_unit_from_cmdline_str("systemd.unit=my-custom.target"),
            Some("my-custom.target".to_owned()),
        );
    }

    #[test]
    fn test_runlevel_2() {
        assert_eq!(
            target_unit_from_cmdline_str("2"),
            Some("multi-user.target".to_owned()),
        );
    }

    #[test]
    fn test_runlevel_4() {
        assert_eq!(
            target_unit_from_cmdline_str("4"),
            Some("multi-user.target".to_owned()),
        );
    }

    #[test]
    fn test_whitespace_variations() {
        // Tabs and multiple spaces should be handled correctly
        assert_eq!(
            target_unit_from_cmdline_str("  quiet\t\tsingle  "),
            Some("rescue.target".to_owned()),
        );
    }

    #[test]
    fn test_newline_at_end() {
        // /proc/cmdline typically has a trailing newline
        assert_eq!(
            target_unit_from_cmdline_str("quiet emergency\n"),
            Some("emergency.target".to_owned()),
        );
    }
}
