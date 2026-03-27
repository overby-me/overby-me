//! Systemd unit specifier expansion.
//!
//! This module implements the specifier expansion rules documented in
//! `systemd.unit(5)`.  Specifiers are `%X` sequences in unit file values
//! that are replaced with runtime information when the unit is loaded.
//!
//! ## Supported specifiers
//!
//! | Specifier | Description                                              |
//! |-----------|----------------------------------------------------------|
//! | `%a`      | Architecture (short string from `uname`)                 |
//! | `%A`      | OS image version (`IMAGE_VERSION=` from os-release)      |
//! | `%b`      | Boot ID                                                  |
//! | `%B`      | OS build ID (`BUILD_ID=` from os-release)                |
//! | `%C`      | Cache directory root (`/var/cache` or `$XDG_CACHE_HOME`) |
//! | `%d`      | Credentials directory (runtime, `CREDENTIALS_DIRECTORY`) |
//! | `%E`      | Configuration directory root (`/etc` or `$XDG_CONFIG_HOME`) |
//! | `%f`      | Unescaped instance name with leading `/`, or prefix      |
//! | `%g`      | User group name (from service manager user)              |
//! | `%G`      | User GID (numeric)                                       |
//! | `%h`      | Home directory of the user running the service manager   |
//! | `%H`      | Hostname (short)                                         |
//! | `%i`      | Instance name (escaped)                                  |
//! | `%I`      | Instance name (unescaped)                                |
//! | `%j`      | Prefix of the instance or prefix (part before first `-`) |
//! | `%J`      | Unescaped version of `%j`                                |
//! | `%l`      | Short hostname (up to the first `.`)                     |
//! | `%L`      | Log directory (`/var/log` or `$XDG_STATE_HOME/log`)      |
//! | `%m`      | Machine ID                                               |
//! | `%M`      | OS pretty name (`PRETTY_NAME=` from os-release)          |
//! | `%n`      | Full unit name                                           |
//! | `%N`      | Unit name without suffix (prefix + instance)             |
//! | `%o`      | OS ID (`ID=` from os-release)                            |
//! | `%p`      | Prefix (unit name without suffix, or template prefix)    |
//! | `%P`      | Prefix (unescaped)                                       |
//! | `%q`      | OS version ID (`VERSION_ID=` from os-release)            |
//! | `%s`      | Shell of the user running the service manager            |
//! | `%S`      | State directory (`/var/lib` or `$XDG_STATE_HOME`)        |
//! | `%t`      | Runtime directory (`/run` or `$XDG_RUNTIME_DIR`)         |
//! | `%T`      | Temporary directory (`/tmp`)                             |
//! | `%u`      | User name running the service manager                    |
//! | `%U`      | User UID (numeric)                                       |
//! | `%v`      | Kernel release (`uname -r`)                              |
//! | `%V`      | Temporary directory with large files (`/var/tmp`)        |
//! | `%w`      | OS version (`VERSION=` from os-release)                  |
//! | `%W`      | OS variant ID (`VARIANT_ID=` from os-release)            |
//! | `%%`      | Literal `%`                                              |
//!
//! ## Usage
//!
//! ```rust,no_run
//! use libsystemd::specifier::{SpecifierContext, resolve_specifiers};
//!
//! let ctx = SpecifierContext::for_system();
//! let result = resolve_specifiers("ExecStart=/usr/bin/getty %I", "serial-getty@ttyS0.service", "ttyS0", &ctx);
//! assert_eq!(result, "ExecStart=/usr/bin/getty ttyS0");
//! ```

use crate::unit_name::unit_name_unescape;
use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// SpecifierContext — runtime information needed for expansion
// ---------------------------------------------------------------------------

/// Runtime context for specifier expansion.
///
/// This struct caches all the system information that specifiers may
/// reference.  It should be created once (via [`SpecifierContext::for_system`]
/// or [`SpecifierContext::for_user`]) and reused across all unit loads.
#[derive(Debug, Clone)]
pub struct SpecifierContext {
    /// User name of the user running the service manager.
    pub user_name: String,
    /// Numeric UID of the user running the service manager.
    pub user_uid: String,
    /// Home directory of the user running the service manager.
    pub user_home: String,
    /// Shell of the user running the service manager.
    pub user_shell: String,
    /// User group name of the user running the service manager.
    pub user_group: String,
    /// Numeric GID of the user running the service manager.
    pub user_gid: String,
    /// Machine ID (contents of `/etc/machine-id`, trimmed).
    pub machine_id: String,
    /// Boot ID (contents of `/proc/sys/kernel/random/boot_id`, trimmed,
    /// without hyphens).
    pub boot_id: String,
    /// Short hostname (from `gethostname`).
    pub hostname: String,
    /// Short hostname (up to the first `.`).
    pub short_hostname: String,
    /// Kernel release string (`uname -r`).
    pub kernel_release: String,
    /// Architecture short string (e.g. `x86-64`, `arm`, `aarch64`).
    pub architecture: String,
    /// Runtime directory (`/run` for system, `$XDG_RUNTIME_DIR` for user).
    pub runtime_dir: String,
    /// Configuration directory root (`/etc` for system, `$XDG_CONFIG_HOME`
    /// for user).
    pub config_dir: String,
    /// Cache directory root (`/var/cache` for system, `$XDG_CACHE_HOME` for
    /// user).
    pub cache_dir: String,
    /// Log directory (`/var/log` for system, `$XDG_STATE_HOME/log` for user).
    pub log_dir: String,
    /// State directory (`/var/lib` for system, `$XDG_STATE_HOME` for user).
    pub state_dir: String,
    /// Temporary directory (`/tmp`).
    pub tmp_dir: String,
    /// Large temporary directory (`/var/tmp`).
    pub var_tmp_dir: String,
    /// os-release fields (lazily parsed from `/etc/os-release` or
    /// `/usr/lib/os-release`).
    pub os_release: HashMap<String, String>,
    /// Whether this is a system or user instance.
    pub is_system: bool,
}

impl SpecifierContext {
    /// Create a context for the system service manager (PID 1).
    pub fn for_system() -> Self {
        let (user_name, user_uid, user_home, user_shell, user_group, user_gid) =
            get_user_info_system();
        let machine_id = read_machine_id();
        let boot_id = read_boot_id();
        let hostname = get_hostname();
        let short_hostname = hostname.split('.').next().unwrap_or(&hostname).to_string();
        let kernel_release = get_kernel_release();
        let architecture = get_architecture();
        let os_release = parse_os_release();

        SpecifierContext {
            user_name,
            user_uid,
            user_home,
            user_shell,
            user_group,
            user_gid,
            machine_id,
            boot_id,
            hostname: hostname.clone(),
            short_hostname,
            kernel_release,
            architecture,
            runtime_dir: "/run".to_string(),
            config_dir: "/etc".to_string(),
            cache_dir: "/var/cache".to_string(),
            log_dir: "/var/log".to_string(),
            state_dir: "/var/lib".to_string(),
            tmp_dir: "/tmp".to_string(),
            var_tmp_dir: "/var/tmp".to_string(),
            os_release,
            is_system: true,
        }
    }

    /// Create a context for a user service manager.
    pub fn for_user() -> Self {
        let (user_name, user_uid, user_home, user_shell, user_group, user_gid) =
            get_current_user_info();
        let machine_id = read_machine_id();
        let boot_id = read_boot_id();
        let hostname = get_hostname();
        let short_hostname = hostname.split('.').next().unwrap_or(&hostname).to_string();
        let kernel_release = get_kernel_release();
        let architecture = get_architecture();
        let os_release = parse_os_release();

        let runtime_dir =
            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", user_uid));
        let config_dir =
            std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{}/.config", user_home));
        let cache_dir =
            std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| format!("{}/.cache", user_home));
        let state_dir = std::env::var("XDG_STATE_HOME")
            .unwrap_or_else(|_| format!("{}/.local/state", user_home));
        let log_dir = format!("{}/log", &state_dir);

        SpecifierContext {
            user_name,
            user_uid,
            user_home,
            user_shell,
            user_group,
            user_gid,
            machine_id,
            boot_id,
            hostname: hostname.clone(),
            short_hostname,
            kernel_release,
            architecture,
            runtime_dir,
            config_dir,
            cache_dir,
            log_dir,
            state_dir,
            tmp_dir: "/tmp".to_string(),
            var_tmp_dir: "/var/tmp".to_string(),
            os_release,
            is_system: false,
        }
    }

    /// Create a minimal context for testing, with all fields set to
    /// controllable values.
    #[cfg(test)]
    pub fn for_test() -> Self {
        SpecifierContext {
            user_name: "testuser".to_string(),
            user_uid: "1000".to_string(),
            user_home: "/home/testuser".to_string(),
            user_shell: "/bin/bash".to_string(),
            user_group: "testuser".to_string(),
            user_gid: "1000".to_string(),
            machine_id: "0123456789abcdef0123456789abcdef".to_string(),
            boot_id: "abcdef0123456789abcdef0123456789".to_string(),
            hostname: "testhost.example.com".to_string(),
            short_hostname: "testhost".to_string(),
            kernel_release: "6.1.0-test".to_string(),
            architecture: "x86-64".to_string(),
            runtime_dir: "/run".to_string(),
            config_dir: "/etc".to_string(),
            cache_dir: "/var/cache".to_string(),
            log_dir: "/var/log".to_string(),
            state_dir: "/var/lib".to_string(),
            tmp_dir: "/tmp".to_string(),
            var_tmp_dir: "/var/tmp".to_string(),
            os_release: {
                let mut m = HashMap::new();
                m.insert("ID".to_string(), "nixos".to_string());
                m.insert("VERSION_ID".to_string(), "24.05".to_string());
                m.insert(
                    "PRETTY_NAME".to_string(),
                    "NixOS 24.05 (Uakari)".to_string(),
                );
                m.insert("BUILD_ID".to_string(), "24.05.20240101".to_string());
                m.insert("IMAGE_VERSION".to_string(), "24.05".to_string());
                m.insert("VERSION".to_string(), "24.05 (Uakari)".to_string());
                m.insert("VARIANT_ID".to_string(), "".to_string());
                m
            },
            is_system: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Core expansion function
// ---------------------------------------------------------------------------

/// Expand all `%X` specifiers in `content` using the given unit name,
/// instance name, and runtime context.
///
/// - `content` — the string to expand (may be multi-line, e.g. a whole
///   unit file section or a single directive value).
/// - `unit_name` — the full unit name including suffix (e.g.
///   `serial-getty@ttyS0.service`).
/// - `instance` — the instance name for template units (e.g. `ttyS0`).
///   Pass `""` for non-template units.
/// - `ctx` — the [`SpecifierContext`] with system/user runtime info.
///
/// Unknown specifiers are kept as-is (the `%X` sequence is preserved).
pub fn resolve_specifiers(
    content: &str,
    unit_name: &str,
    instance: &str,
    ctx: &SpecifierContext,
) -> String {
    let prefix = extract_prefix(unit_name);
    let instance_or_prefix = if instance.is_empty() {
        prefix.as_str()
    } else {
        instance
    };

    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '%' {
            result.push(c);
            continue;
        }

        match chars.peek() {
            Some(&spec) => {
                chars.next();
                match spec {
                    // --- unit identity ---
                    'n' => result.push_str(unit_name),
                    'N' => {
                        // Unit name without the suffix (prefix + instance).
                        // e.g. "foo@bar.service" -> "foo@bar",
                        //      "issue14566-repro.service" -> "issue14566-repro"
                        if let Some(dot) = unit_name.rfind('.') {
                            result.push_str(&unit_name[..dot]);
                        } else {
                            result.push_str(unit_name);
                        }
                    }
                    'p' => result.push_str(&prefix),
                    'P' => {
                        let unescaped = unit_name_unescape(&prefix).unwrap_or(prefix.to_string());
                        result.push_str(&unescaped);
                    }
                    'i' => result.push_str(instance),
                    'I' => {
                        let unescaped =
                            unit_name_unescape(instance).unwrap_or(instance.to_string());
                        result.push_str(&unescaped);
                    }
                    'f' => {
                        // Unescaped instance or prefix with leading `/`.
                        let raw = instance_or_prefix;
                        let unescaped = unit_name_unescape(raw).unwrap_or(raw.to_string());
                        if unescaped.starts_with('/') {
                            result.push_str(&unescaped);
                        } else {
                            result.push('/');
                            result.push_str(&unescaped);
                        }
                    }
                    'j' => {
                        // For instantiated units: part before first `-` in instance.
                        // For non-instantiated units: part after last `-` in prefix.
                        if !instance.is_empty() {
                            let first_dash = instance.find('-').unwrap_or(instance.len());
                            result.push_str(&instance[..first_dash]);
                        } else {
                            let last_dash = prefix.rfind('-').map(|p| p + 1).unwrap_or(0);
                            result.push_str(&prefix[last_dash..]);
                        }
                    }
                    'J' => {
                        // Unescaped %j.
                        let part = if !instance.is_empty() {
                            let first_dash = instance.find('-').unwrap_or(instance.len());
                            &instance[..first_dash]
                        } else {
                            let last_dash = prefix.rfind('-').map(|p| p + 1).unwrap_or(0);
                            &prefix[last_dash..]
                        };
                        let unescaped = unit_name_unescape(part).unwrap_or(part.to_string());
                        result.push_str(&unescaped);
                    }

                    // --- user/group ---
                    'u' => result.push_str(&ctx.user_name),
                    'U' => result.push_str(&ctx.user_uid),
                    'h' => result.push_str(&ctx.user_home),
                    's' => result.push_str(&ctx.user_shell),
                    'g' => result.push_str(&ctx.user_group),
                    'G' => result.push_str(&ctx.user_gid),

                    // --- system identity ---
                    'm' => result.push_str(&ctx.machine_id),
                    'b' => result.push_str(&ctx.boot_id),
                    'H' => result.push_str(&ctx.hostname),
                    'l' => result.push_str(&ctx.short_hostname),
                    'v' => result.push_str(&ctx.kernel_release),
                    'a' => result.push_str(&ctx.architecture),

                    // --- directories ---
                    't' => result.push_str(&ctx.runtime_dir),
                    'T' => result.push_str(&ctx.tmp_dir),
                    'V' => result.push_str(&ctx.var_tmp_dir),
                    'E' => result.push_str(&ctx.config_dir),
                    'C' => result.push_str(&ctx.cache_dir),
                    'S' => result.push_str(&ctx.state_dir),
                    'L' => result.push_str(&ctx.log_dir),

                    // --- credentials ---
                    'd' => {
                        // Credentials directory for the unit at runtime.
                        // Before exec this is not yet known, so we produce
                        // the conventional path.
                        result.push_str(&format!("/run/credentials/{}", unit_name));
                    }

                    // --- os-release ---
                    'o' => {
                        result.push_str(ctx.os_release.get("ID").map(|s| s.as_str()).unwrap_or(""))
                    }
                    'w' => result.push_str(
                        ctx.os_release
                            .get("VERSION")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),
                    'q' => result.push_str(
                        ctx.os_release
                            .get("VERSION_ID")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),
                    'M' => result.push_str(
                        ctx.os_release
                            .get("PRETTY_NAME")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),
                    'A' => result.push_str(
                        ctx.os_release
                            .get("IMAGE_VERSION")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),
                    'B' => result.push_str(
                        ctx.os_release
                            .get("BUILD_ID")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),
                    'W' => result.push_str(
                        ctx.os_release
                            .get("VARIANT_ID")
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    ),

                    // --- literal percent ---
                    '%' => result.push('%'),

                    // --- unknown specifier — preserve as-is ---
                    _ => {
                        result.push('%');
                        result.push(spec);
                    }
                }
            }
            None => {
                // Trailing `%` at end of string — keep as-is
                result.push('%');
            }
        }
    }

    result
}

/// Convenience wrapper that calls [`resolve_specifiers`] with an
/// automatically-detected instance name extracted from `unit_name`.
///
/// If `unit_name` is a template instance like `foo@bar.service`, the
/// instance `bar` is extracted.  Otherwise the instance is `""`.
pub fn resolve_specifiers_auto(content: &str, unit_name: &str, ctx: &SpecifierContext) -> String {
    let instance = extract_instance(unit_name);
    resolve_specifiers(content, unit_name, &instance, ctx)
}

/// Check whether a string contains unresolved systemd specifiers like
/// `%i`, `%I`, `%n`, `%N`, etc.
///
/// `%%` (escaped percent) is **not** considered an unresolved specifier.
pub fn has_unresolved_specifiers(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%'
            && let Some(&next) = chars.peek()
            && next != '%'
            && next.is_alphanumeric()
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Unit name helpers
// ---------------------------------------------------------------------------

/// Extract the prefix from a unit name.
///
/// For `serial-getty@ttyS0.service` this returns `serial-getty`.
/// For `foo.service` this returns `foo`.
fn extract_prefix(unit_name: &str) -> String {
    // Strip the suffix (.service, .socket, etc.)
    let without_suffix = unit_name
        .rfind('.')
        .map(|pos| &unit_name[..pos])
        .unwrap_or(unit_name);

    // If it's a template instance, strip the instance part
    if let Some(at_pos) = without_suffix.find('@') {
        without_suffix[..at_pos].to_string()
    } else {
        without_suffix.to_string()
    }
}

/// Extract the instance name from a unit name.
///
/// For `serial-getty@ttyS0.service` this returns `ttyS0`.
/// For `foo.service` this returns `""`.
fn extract_instance(unit_name: &str) -> String {
    let without_suffix = unit_name
        .rfind('.')
        .map(|pos| &unit_name[..pos])
        .unwrap_or(unit_name);

    if let Some(at_pos) = without_suffix.find('@') {
        without_suffix[at_pos + 1..].to_string()
    } else {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// System information helpers
// ---------------------------------------------------------------------------

/// Read the machine ID from `/etc/machine-id`.
fn read_machine_id() -> String {
    fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().replace('-', ""))
        .unwrap_or_else(|_| "0".repeat(32))
}

/// Read the boot ID from `/proc/sys/kernel/random/boot_id`.
fn read_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().replace('-', ""))
        .unwrap_or_else(|_| "0".repeat(32))
}

/// Get the hostname.
fn get_hostname() -> String {
    nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string())
}

/// Get the kernel release string from `uname`.
fn get_kernel_release() -> String {
    nix::sys::utsname::uname()
        .ok()
        .map(|u| u.release().to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Map the `uname` machine string to the systemd architecture short name.
fn get_architecture() -> String {
    let machine = nix::sys::utsname::uname()
        .ok()
        .map(|u| u.machine().to_string_lossy().into_owned())
        .unwrap_or_default();

    // Map Linux uname machine names to systemd architecture identifiers.
    match machine.as_str() {
        "x86_64" => "x86-64".to_string(),
        "i386" | "i486" | "i586" | "i686" => "x86".to_string(),
        "aarch64" | "aarch64_be" => "arm64".to_string(),
        "armv7l" | "armv6l" | "armv5tel" | "armv5tejl" | "armv4t" | "armv4l" => "arm".to_string(),
        "riscv64" => "riscv64".to_string(),
        "riscv32" => "riscv32".to_string(),
        "ppc64le" | "ppc64" => "ppc64".to_string(),
        "ppc" | "ppcle" => "ppc".to_string(),
        "s390x" => "s390x".to_string(),
        "s390" => "s390".to_string(),
        "mips64" => "mips64".to_string(),
        "mips" => "mips".to_string(),
        "loongarch64" => "loongarch64".to_string(),
        "alpha" => "alpha".to_string(),
        "ia64" => "ia64".to_string(),
        "m68k" => "m68k".to_string(),
        "sparc64" | "sparc" => "sparc".to_string(),
        "sh4" | "sh3" | "sh2" | "sh" => "sh".to_string(),
        other => other.to_string(),
    }
}

/// Get user information for the system service manager (uid 0, root).
fn get_user_info_system() -> (String, String, String, String, String, String) {
    // For the system instance, the "user" is always root.
    (
        "root".to_string(),
        "0".to_string(),
        "/root".to_string(),
        "/bin/sh".to_string(),
        "root".to_string(),
        "0".to_string(),
    )
}

/// Get user information for the current (real) user.
fn get_current_user_info() -> (String, String, String, String, String, String) {
    let uid = nix::unistd::getuid();
    let gid = nix::unistd::getgid();

    let (user_name, user_home, user_shell) = nix::unistd::User::from_uid(uid)
        .ok()
        .flatten()
        .map(|u| {
            (
                u.name,
                u.dir.to_string_lossy().into_owned(),
                u.shell.to_string_lossy().into_owned(),
            )
        })
        .unwrap_or_else(|| {
            (
                std::env::var("USER").unwrap_or_else(|_| uid.to_string()),
                std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
            )
        });

    let group_name = nix::unistd::Group::from_gid(gid)
        .ok()
        .flatten()
        .map(|g| g.name)
        .unwrap_or_else(|| gid.to_string());

    (
        user_name,
        uid.to_string(),
        user_home,
        user_shell,
        group_name,
        gid.to_string(),
    )
}

/// Parse `/etc/os-release` (or `/usr/lib/os-release` as fallback) into a
/// key-value map.
fn parse_os_release() -> HashMap<String, String> {
    static CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let content = fs::read_to_string("/etc/os-release")
                .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
                .unwrap_or_default();
            parse_os_release_content(&content)
        })
        .clone()
}

/// Parse the textual content of an os-release file.
fn parse_os_release_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let mut value = value.trim().to_string();
            // Remove surrounding quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            map.insert(key, value);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SpecifierContext {
        SpecifierContext::for_test()
    }

    // --- basic identity specifiers ---

    #[test]
    fn test_n_full_unit_name() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%n", "foo.service", "", &c),
            "foo.service"
        );
    }

    #[test]
    fn test_n_template_instance() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%n", "serial-getty@ttyS0.service", "ttyS0", &c),
            "serial-getty@ttyS0.service"
        );
    }

    #[test]
    fn test_p_prefix_simple() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%p", "foo.service", "", &c), "foo");
    }

    #[test]
    fn test_p_prefix_template() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%p", "serial-getty@ttyS0.service", "ttyS0", &c),
            "serial-getty"
        );
    }

    #[test]
    fn test_i_instance() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%i", "serial-getty@ttyS0.service", "ttyS0", &c),
            "ttyS0"
        );
    }

    #[test]
    fn test_i_empty_for_non_template() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%i", "foo.service", "", &c), "");
    }

    #[test]
    fn test_percent_percent() {
        let c = ctx();
        assert_eq!(resolve_specifiers("100%%", "foo.service", "", &c), "100%");
    }

    // --- unescaped specifiers ---

    #[test]
    fn test_capital_i_unescape_instance() {
        let c = ctx();
        // Instance "dev-sda" should unescape to "dev/sda"
        // (systemd uses `-` → `/` unescaping in unit names)
        assert_eq!(
            resolve_specifiers("%I", "systemd-fsck@dev-sda.service", "dev-sda", &c),
            "dev/sda"
        );
    }

    #[test]
    fn test_capital_i_plain_instance() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%I", "serial-getty@ttyS0.service", "ttyS0", &c),
            "ttyS0"
        );
    }

    #[test]
    fn test_capital_n_strips_suffix() {
        let c = ctx();
        // %N = unit name without the suffix
        assert_eq!(resolve_specifiers("%N", "foo.service", "", &c), "foo");
    }

    #[test]
    fn test_capital_p_unescape_prefix() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%P", "dev-sda.device", "", &c),
            "dev/sda"
        );
    }

    // --- user/group specifiers ---

    #[test]
    fn test_u_user_name() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%u", "foo.service", "", &c), "testuser");
    }

    #[test]
    fn test_capital_u_uid() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%U", "foo.service", "", &c), "1000");
    }

    #[test]
    fn test_h_home() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%h", "foo.service", "", &c),
            "/home/testuser"
        );
    }

    #[test]
    fn test_s_shell() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%s", "foo.service", "", &c), "/bin/bash");
    }

    #[test]
    fn test_g_group_name() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%g", "foo.service", "", &c), "testuser");
    }

    #[test]
    fn test_capital_g_gid() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%G", "foo.service", "", &c), "1000");
    }

    // --- system identity specifiers ---

    #[test]
    fn test_m_machine_id() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%m", "foo.service", "", &c),
            "0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn test_b_boot_id() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%b", "foo.service", "", &c),
            "abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn test_capital_h_hostname() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%H", "foo.service", "", &c),
            "testhost.example.com"
        );
    }

    #[test]
    fn test_l_short_hostname() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%l", "foo.service", "", &c), "testhost");
    }

    #[test]
    fn test_v_kernel_release() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%v", "foo.service", "", &c),
            "6.1.0-test"
        );
    }

    #[test]
    fn test_a_architecture() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%a", "foo.service", "", &c), "x86-64");
    }

    // --- directory specifiers ---

    #[test]
    fn test_t_runtime_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%t", "foo.service", "", &c), "/run");
    }

    #[test]
    fn test_capital_t_tmp_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%T", "foo.service", "", &c), "/tmp");
    }

    #[test]
    fn test_capital_v_var_tmp_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%V", "foo.service", "", &c), "/var/tmp");
    }

    #[test]
    fn test_capital_e_config_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%E", "foo.service", "", &c), "/etc");
    }

    #[test]
    fn test_capital_c_cache_dir() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%C", "foo.service", "", &c),
            "/var/cache"
        );
    }

    #[test]
    fn test_capital_s_state_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%S", "foo.service", "", &c), "/var/lib");
    }

    #[test]
    fn test_capital_l_log_dir() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%L", "foo.service", "", &c), "/var/log");
    }

    // --- credentials specifier ---

    #[test]
    fn test_d_credentials_dir() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%d", "foo.service", "", &c),
            "/run/credentials/foo.service"
        );
    }

    // --- os-release specifiers ---

    #[test]
    fn test_o_os_id() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%o", "foo.service", "", &c), "nixos");
    }

    #[test]
    fn test_q_version_id() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%q", "foo.service", "", &c), "24.05");
    }

    #[test]
    fn test_capital_m_pretty_name() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%M", "foo.service", "", &c),
            "NixOS 24.05 (Uakari)"
        );
    }

    #[test]
    fn test_capital_a_image_version() {
        let c = ctx();
        assert_eq!(resolve_specifiers("%A", "foo.service", "", &c), "24.05");
    }

    #[test]
    fn test_capital_b_build_id() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%B", "foo.service", "", &c),
            "24.05.20240101"
        );
    }

    #[test]
    fn test_w_version() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%w", "foo.service", "", &c),
            "24.05 (Uakari)"
        );
    }

    #[test]
    fn test_capital_w_variant_id() {
        let c = ctx();
        // VARIANT_ID is set to "" in for_test()
        assert_eq!(resolve_specifiers("%W", "foo.service", "", &c), "");
    }

    // --- %f specifier ---

    #[test]
    fn test_f_unescaped_instance_with_slash() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%f", "systemd-fsck@dev-sda.service", "dev-sda", &c),
            "/dev/sda"
        );
    }

    #[test]
    fn test_f_falls_back_to_prefix() {
        let c = ctx();
        // No instance, so %f uses the unescaped prefix with `/` prepended.
        assert_eq!(resolve_specifiers("%f", "foo.service", "", &c), "/foo");
    }

    // --- %j / %J specifiers ---

    #[test]
    fn test_j_first_component_of_instance() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%j", "foo@dev-sda1.service", "dev-sda1", &c),
            "dev"
        );
    }

    #[test]
    fn test_j_no_dash_returns_full() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("%j", "foo@ttyS0.service", "ttyS0", &c),
            "ttyS0"
        );
    }

    #[test]
    fn test_capital_j_unescaped() {
        let c = ctx();
        // %J should unescape the first component
        assert_eq!(
            resolve_specifiers("%J", "foo@dev-sda1.service", "dev-sda1", &c),
            "dev"
        );
    }

    #[test]
    fn test_j_non_instantiated_last_component() {
        let c = ctx();
        // For non-instantiated units, %j is the part after the last dash
        assert_eq!(
            resolve_specifiers("%j", "TEST-23-UNIT-FILE-specifier-j-wants.service", "", &c),
            "wants"
        );
    }

    #[test]
    fn test_j_non_instantiated_no_dash() {
        let c = ctx();
        // No dash in prefix: %j returns the whole prefix
        assert_eq!(resolve_specifiers("%j", "foo.service", "", &c), "foo");
    }

    // --- compound expressions ---

    #[test]
    fn test_compound_getty() {
        let c = ctx();
        let result = resolve_specifiers(
            "TTYPath=/dev/%I\nDescription=Getty on %I\nUtmpIdentifier=%I",
            "serial-getty@ttyS0.service",
            "ttyS0",
            &c,
        );
        assert_eq!(
            result,
            "TTYPath=/dev/ttyS0\nDescription=Getty on ttyS0\nUtmpIdentifier=ttyS0"
        );
    }

    #[test]
    fn test_compound_all_identity_specifiers() {
        let c = ctx();
        let result = resolve_specifiers("%n %N %i %I %p %P %%", "foo@bar.service", "bar", &c);
        assert_eq!(result, "foo@bar.service foo@bar bar bar foo foo %");
    }

    #[test]
    fn test_compound_system_specifiers() {
        let c = ctx();
        let result = resolve_specifiers(
            "m=%m b=%b H=%H v=%v t=%t u=%u U=%U h=%h s=%s",
            "foo.service",
            "",
            &c,
        );
        assert_eq!(
            result,
            "m=0123456789abcdef0123456789abcdef b=abcdef0123456789abcdef0123456789 \
             H=testhost.example.com v=6.1.0-test t=/run u=testuser U=1000 \
             h=/home/testuser s=/bin/bash"
        );
    }

    // --- unknown specifiers ---

    #[test]
    fn test_unknown_specifier_preserved() {
        let c = ctx();
        // %z is not a known specifier — should be preserved as-is.
        assert_eq!(
            resolve_specifiers("foo %z bar", "foo.service", "", &c),
            "foo %z bar"
        );
    }

    #[test]
    fn test_trailing_percent() {
        let c = ctx();
        assert_eq!(resolve_specifiers("foo%", "foo.service", "", &c), "foo%");
    }

    #[test]
    fn test_no_specifiers() {
        let c = ctx();
        assert_eq!(
            resolve_specifiers("ExecStart=/usr/bin/true", "foo.service", "", &c),
            "ExecStart=/usr/bin/true"
        );
    }

    #[test]
    fn test_empty_string() {
        let c = ctx();
        assert_eq!(resolve_specifiers("", "foo.service", "", &c), "");
    }

    // --- has_unresolved_specifiers ---

    #[test]
    fn test_has_unresolved_yes() {
        assert!(has_unresolved_specifiers("%i"));
        assert!(has_unresolved_specifiers("%I"));
        assert!(has_unresolved_specifiers("%n"));
        assert!(has_unresolved_specifiers("foo%ibar"));
        assert!(has_unresolved_specifiers("systemd-journald@%i"));
    }

    #[test]
    fn test_has_unresolved_no() {
        assert!(!has_unresolved_specifiers("ttyS0"));
        assert!(!has_unresolved_specifiers("%%"));
        assert!(!has_unresolved_specifiers(""));
        assert!(!has_unresolved_specifiers("no-specifiers-here"));
        assert!(!has_unresolved_specifiers("100% done"));
        // `%` followed by non-alphanumeric is not a specifier
        assert!(!has_unresolved_specifiers("foo% bar"));
    }

    // --- extract_prefix ---

    #[test]
    fn test_extract_prefix_simple() {
        assert_eq!(extract_prefix("foo.service"), "foo");
    }

    #[test]
    fn test_extract_prefix_template() {
        assert_eq!(extract_prefix("serial-getty@ttyS0.service"), "serial-getty");
    }

    #[test]
    fn test_extract_prefix_no_suffix() {
        assert_eq!(extract_prefix("foo"), "foo");
    }

    // --- extract_instance ---

    #[test]
    fn test_extract_instance_template() {
        assert_eq!(extract_instance("serial-getty@ttyS0.service"), "ttyS0");
    }

    #[test]
    fn test_extract_instance_no_instance() {
        assert_eq!(extract_instance("foo.service"), "");
    }

    #[test]
    fn test_extract_instance_empty_template() {
        assert_eq!(extract_instance("foo@.service"), "");
    }

    // --- resolve_specifiers_auto ---

    #[test]
    fn test_resolve_specifiers_auto_template() {
        let c = ctx();
        let result = resolve_specifiers_auto("TTYPath=/dev/%I", "serial-getty@ttyS0.service", &c);
        assert_eq!(result, "TTYPath=/dev/ttyS0");
    }

    #[test]
    fn test_resolve_specifiers_auto_non_template() {
        let c = ctx();
        let result = resolve_specifiers_auto("ExecStart=/usr/bin/%p", "foo.service", &c);
        assert_eq!(result, "ExecStart=/usr/bin/foo");
    }

    // --- parse_os_release_content ---

    #[test]
    fn test_parse_os_release_content() {
        let content = r#"
NAME="NixOS"
ID=nixos
VERSION_ID="24.05"
PRETTY_NAME="NixOS 24.05 (Uakari)"
# This is a comment
BUILD_ID=24.05.20240101
"#;
        let map = parse_os_release_content(content);
        assert_eq!(map.get("NAME").unwrap(), "NixOS");
        assert_eq!(map.get("ID").unwrap(), "nixos");
        assert_eq!(map.get("VERSION_ID").unwrap(), "24.05");
        assert_eq!(map.get("PRETTY_NAME").unwrap(), "NixOS 24.05 (Uakari)");
        assert_eq!(map.get("BUILD_ID").unwrap(), "24.05.20240101");
    }

    #[test]
    fn test_parse_os_release_content_single_quotes() {
        let content = "NAME='Fedora Linux'\n";
        let map = parse_os_release_content(content);
        assert_eq!(map.get("NAME").unwrap(), "Fedora Linux");
    }

    #[test]
    fn test_parse_os_release_content_empty() {
        let map = parse_os_release_content("");
        assert!(map.is_empty());
    }

    // --- user-instance directory specifiers ---

    #[test]
    fn test_user_context_directories() {
        let mut c = ctx();
        c.is_system = false;
        c.runtime_dir = "/run/user/1000".to_string();
        c.config_dir = "/home/testuser/.config".to_string();
        c.cache_dir = "/home/testuser/.cache".to_string();
        c.state_dir = "/home/testuser/.local/state".to_string();
        c.log_dir = "/home/testuser/.local/state/log".to_string();

        assert_eq!(
            resolve_specifiers("%t", "foo.service", "", &c),
            "/run/user/1000"
        );
        assert_eq!(
            resolve_specifiers("%E", "foo.service", "", &c),
            "/home/testuser/.config"
        );
        assert_eq!(
            resolve_specifiers("%C", "foo.service", "", &c),
            "/home/testuser/.cache"
        );
        assert_eq!(
            resolve_specifiers("%S", "foo.service", "", &c),
            "/home/testuser/.local/state"
        );
        assert_eq!(
            resolve_specifiers("%L", "foo.service", "", &c),
            "/home/testuser/.local/state/log"
        );
    }
}
