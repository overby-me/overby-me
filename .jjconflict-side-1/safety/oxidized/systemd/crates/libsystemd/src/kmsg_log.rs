//! Kernel message log backend for early-boot / pre-exec contexts.
//!
//! This module provides [`KmsgLogger`], a [`log::Log`] implementation that
//! writes structured messages to `/dev/kmsg` (the kernel ring buffer) with
//! a stderr fallback.  It is designed for use in the exec helper child
//! process where:
//!
//! - The normal `fern` / stdout logger from the service manager is not
//!   available (separate process).
//! - stdout/stderr may be redirected or closed.
//! - `/dev/kmsg` survives mount namespace changes and is visible on the
//!   serial console via `dmesg`, making it the only reliable diagnostic
//!   channel during early boot.
//!
//! # Kernel log format
//!
//! Messages are written as:
//!
//! ```text
//! <priority>rust-systemd[unit]: message\n
//! ```
//!
//! where `priority` follows the syslog convention (`<4>` = warning,
//! `<6>` = info, `<7>` = debug).  The kernel strips the priority prefix
//! and stores it as metadata, so `dmesg --level` filtering works.
//!
//! # Thread safety
//!
//! Each [`log`] call opens `/dev/kmsg`, writes, and closes the fd.  This
//! is intentional — the file descriptor can become invalid after
//! `ProtectKernelLogs=` hides `/dev/kmsg` behind a bind-mount.  Opening
//! fresh each time means we gracefully degrade (writes silently fail)
//! instead of writing to a stale fd.
//!
//! # Usage
//!
//! ```rust,no_run
//! use libsystemd::kmsg_log::KmsgLogger;
//!
//! KmsgLogger::init("systemd-timesyncd", log::LevelFilter::Trace);
//! log::info!("starting up");
//! log::trace!("mount namespace: PrivateDevices=true");
//! ```
//!
//! # Log level configuration
//!
//! [`KmsgLogger::init`] reads the `SYSTEMD_LOG_LEVEL` environment variable
//! (matching real systemd's convention) and uses it when present.  The
//! caller-supplied `default_level` is used as a fallback.

use std::sync::OnceLock;

/// Global storage for the unit name prefix.
///
/// Set once by [`KmsgLogger::init`] and read on every log call.  Using
/// `OnceLock` avoids a heap allocation per message and is lock-free after
/// initialisation.
static UNIT_NAME: OnceLock<String> = OnceLock::new();

/// A [`log::Log`] backend that writes to `/dev/kmsg` with stderr fallback.
///
/// See the [module-level documentation](self) for details.
pub struct KmsgLogger;

impl KmsgLogger {
    /// Initialise the global logger.
    ///
    /// * `unit_name` — included in every message as the syslog identifier
    ///   (e.g. `"systemd-timesyncd"`).
    /// * `default_level` — used when `SYSTEMD_LOG_LEVEL` is not set.
    ///
    /// If `SYSTEMD_LOG_LEVEL` is set to one of `error`, `warn`, `info`,
    /// `debug`, or `trace` (case-insensitive), that level is used instead.
    ///
    /// This function may only be called once per process.  Subsequent calls
    /// are silently ignored (the `log` crate enforces this).
    pub fn init(unit_name: &str, default_level: log::LevelFilter) {
        let _ = UNIT_NAME.set(unit_name.to_owned());

        let level = resolve_log_level(default_level);

        // `set_logger` returns `Err` if a logger is already set — harmless.
        let _ = log::set_logger(&INSTANCE);
        log::set_max_level(level);
    }
}

/// Single static instance used by `log::set_logger`.
static INSTANCE: KmsgLogger = KmsgLogger;

impl log::Log for KmsgLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        // Filtering is handled by `log::set_max_level`; we always accept
        // messages that pass the global filter.
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let unit = UNIT_NAME
            .get()
            .map(String::as_str)
            .unwrap_or("rust-systemd");
        let priority = level_to_kmsg_priority(record.level());

        // Format: <priority>rust-systemd[unit]: message\n
        //
        // We include the module path (if available) as a structured prefix
        // so that grep on `dmesg` output can isolate subsystems.
        let msg = match record.module_path() {
            Some(module) => format!(
                "<{priority}>rust-systemd[{unit}] {module}: {}\n",
                record.args()
            ),
            None => format!("<{priority}>rust-systemd[{unit}]: {}\n", record.args()),
        };

        // ── Write to /dev/kmsg ────────────────────────────────────────
        // SAFETY: open/write/close are async-signal-safe and don't allocate.
        // We intentionally open+close each time (see module docs).
        let wrote_kmsg = unsafe {
            let fd = libc::open(c"/dev/kmsg".as_ptr(), libc::O_WRONLY | libc::O_NOCTTY);
            if fd >= 0 {
                libc::write(fd, msg.as_ptr().cast(), msg.len());
                libc::close(fd);
                true
            } else {
                false
            }
        };

        // ── Fallback: stderr ──────────────────────────────────────────
        // When /dev/kmsg is unavailable (hidden by ProtectKernelLogs or
        // running in a container), write a human-readable version to stderr.
        // We always write to stderr at Warn and above regardless of kmsg
        // success, so operators see errors even without serial console.
        if !wrote_kmsg || record.level() <= log::Level::Warn {
            let stderr_msg = match record.module_path() {
                Some(module) => format!(
                    "[{level} {unit} {module}] {}\n",
                    record.args(),
                    level = record.level(),
                ),
                None => format!(
                    "[{level} {unit}] {}\n",
                    record.args(),
                    level = record.level(),
                ),
            };
            unsafe {
                libc::write(
                    libc::STDERR_FILENO,
                    stderr_msg.as_ptr().cast(),
                    stderr_msg.len(),
                );
            }
        }
    }

    fn flush(&self) {
        // /dev/kmsg and stderr are unbuffered; nothing to flush.
    }
}

/// Map [`log::Level`] to a syslog/kmsg numeric priority.
///
/// These match the `<N>` prefix that the kernel expects in `/dev/kmsg`
/// writes, and align with systemd's own priority mapping.
const fn level_to_kmsg_priority(level: log::Level) -> u8 {
    match level {
        log::Level::Error => 3, // LOG_ERR
        log::Level::Warn => 4,  // LOG_WARNING
        log::Level::Info => 6,  // LOG_INFO
        log::Level::Debug => 7, // LOG_DEBUG
        log::Level::Trace => 7, // LOG_DEBUG (kmsg has no "trace" level)
    }
}

/// Parse a log level string into a [`log::LevelFilter`].
///
/// Recognised values (case-insensitive): `error`/`err`, `warn`/`warning`,
/// `info`/`notice`, `debug`, `trace`.  Numeric syslog levels `0`–`7` are
/// also accepted.  Returns `None` for unrecognised values.
///
/// This is the public parsing primitive used by both [`KmsgLogger::init`]
/// (via [`resolve_log_level`]) and [`crate::entrypoints::exec_helper`] (to
/// parse the `log_level` field from [`crate::entrypoints::ExecHelperConfig`]).
pub fn parse_log_level_filter(s: &str) -> Option<log::LevelFilter> {
    match s.to_ascii_lowercase().as_str() {
        "0" | "emerg" | "emergency" => Some(log::LevelFilter::Error),
        "1" | "alert" => Some(log::LevelFilter::Error),
        "2" | "crit" | "critical" => Some(log::LevelFilter::Error),
        "3" | "err" | "error" => Some(log::LevelFilter::Error),
        "4" | "warning" | "warn" => Some(log::LevelFilter::Warn),
        "5" | "notice" => Some(log::LevelFilter::Info),
        "6" | "info" => Some(log::LevelFilter::Info),
        "7" | "debug" => Some(log::LevelFilter::Debug),
        "trace" => Some(log::LevelFilter::Trace),
        _ => None,
    }
}

/// Resolve the effective log level from `SYSTEMD_LOG_LEVEL` env var,
/// falling back to `default`.
fn resolve_log_level(default: log::LevelFilter) -> log::LevelFilter {
    let Ok(val) = std::env::var("SYSTEMD_LOG_LEVEL") else {
        return default;
    };

    parse_log_level_filter(&val).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level_to_kmsg_priority() {
        assert_eq!(level_to_kmsg_priority(log::Level::Error), 3);
        assert_eq!(level_to_kmsg_priority(log::Level::Warn), 4);
        assert_eq!(level_to_kmsg_priority(log::Level::Info), 6);
        assert_eq!(level_to_kmsg_priority(log::Level::Debug), 7);
        assert_eq!(level_to_kmsg_priority(log::Level::Trace), 7);
    }

    /// Helper to run a closure with `SYSTEMD_LOG_LEVEL` set to a specific
    /// value (or unset), restoring the previous value afterwards.
    ///
    /// This avoids test-order dependence when multiple tests mutate the
    /// same env var in the same process.
    fn with_log_level<F: FnOnce()>(val: Option<&str>, f: F) {
        let prev = std::env::var("SYSTEMD_LOG_LEVEL").ok();
        match val {
            Some(v) => unsafe { std::env::set_var("SYSTEMD_LOG_LEVEL", v) },
            None => unsafe { std::env::remove_var("SYSTEMD_LOG_LEVEL") },
        }
        f();
        match prev {
            Some(v) => unsafe { std::env::set_var("SYSTEMD_LOG_LEVEL", v) },
            None => unsafe { std::env::remove_var("SYSTEMD_LOG_LEVEL") },
        }
    }

    #[test]
    fn test_parse_log_level_filter() {
        assert_eq!(
            parse_log_level_filter("error"),
            Some(log::LevelFilter::Error)
        );
        assert_eq!(parse_log_level_filter("warn"), Some(log::LevelFilter::Warn));
        assert_eq!(parse_log_level_filter("info"), Some(log::LevelFilter::Info));
        assert_eq!(
            parse_log_level_filter("debug"),
            Some(log::LevelFilter::Debug)
        );
        assert_eq!(
            parse_log_level_filter("trace"),
            Some(log::LevelFilter::Trace)
        );
        assert_eq!(parse_log_level_filter("3"), Some(log::LevelFilter::Error));
        assert_eq!(parse_log_level_filter("7"), Some(log::LevelFilter::Debug));
        assert_eq!(
            parse_log_level_filter("WARNING"),
            Some(log::LevelFilter::Warn)
        );
        assert_eq!(parse_log_level_filter("garbage"), None);
        assert_eq!(parse_log_level_filter(""), None);
    }

    /// All env-var-dependent `resolve_log_level` assertions live in a single
    /// test to avoid parallel-thread races on `SYSTEMD_LOG_LEVEL`.
    #[test]
    fn test_resolve_log_level() {
        // ── Without env var: returns the caller-supplied default ───────
        with_log_level(None, || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Warn),
                log::LevelFilter::Warn
            );
            assert_eq!(
                resolve_log_level(log::LevelFilter::Trace),
                log::LevelFilter::Trace
            );
        });

        // ── With env var: overrides the default ───────────────────────
        with_log_level(Some("debug"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Warn),
                log::LevelFilter::Debug
            );
        });

        with_log_level(Some("7"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Error),
                log::LevelFilter::Debug
            );
        });

        with_log_level(Some("warning"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Trace),
                log::LevelFilter::Warn
            );
        });

        with_log_level(Some("trace"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Warn),
                log::LevelFilter::Trace
            );
        });

        with_log_level(Some("NOTICE"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Error),
                log::LevelFilter::Info
            );
        });

        with_log_level(Some("garbage"), || {
            assert_eq!(
                resolve_log_level(log::LevelFilter::Info),
                log::LevelFilter::Info
            );
        });
    }
}
