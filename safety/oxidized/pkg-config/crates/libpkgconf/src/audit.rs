//! Audit logging for dependency resolution.
//!
//! This module provides audit logging support, allowing pkg-config operations
//! to be recorded to a log file for debugging and analysis. The log file
//! captures dependency resolution decisions, package lookups, and version
//! checks.
//!
//! Logging is activated via:
//! - The `--log-file` CLI flag
//! - The `PKG_CONFIG_LOG` environment variable
//!
//! # Log Format
//!
//! The log file uses a simple text format with one entry per line:
//!
//! ```text
//! [2025-01-01T00:00:00] RESOLVE: zlib >= 1.2.11
//! [2025-01-01T00:00:00] FOUND: zlib 1.2.13 at /usr/lib/pkgconfig/zlib.pc
//! [2025-01-01T00:00:00] DEPENDENCY: zlib requires libfoo >= 1.0
//! ```

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// An audit logger that records dependency resolution events to a file.
///
/// The logger is designed to be created once per client session and passed
/// around during resolution. It appends entries to the configured log file.
#[derive(Debug)]
pub struct AuditLog {
    /// Path to the log file.
    path: PathBuf,

    /// The open file handle, wrapped in a mutex for interior mutability.
    file: Mutex<File>,
}

impl AuditLog {
    /// Open an audit log at the given file path.
    ///
    /// The file is opened in append mode. If the file does not exist, it is
    /// created. If it cannot be opened, an error is returned.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, std::io::Error> {
        let path = path.into();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        Ok(Self {
            path,
            file: Mutex::new(file),
        })
    }

    /// Get the path of the log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write a raw log entry with a timestamp prefix.
    pub fn log(&self, message: &str) {
        if let Ok(mut file) = self.file.lock() {
            let timestamp = Self::timestamp();
            let _ = writeln!(file, "[{timestamp}] {message}");
        }
    }

    /// Log a package resolution attempt.
    pub fn log_resolve(&self, package: &str, constraint: &str) {
        if constraint.is_empty() {
            self.log(&format!("RESOLVE: {package}"));
        } else {
            self.log(&format!("RESOLVE: {package} {constraint}"));
        }
    }

    /// Log a successful package lookup.
    pub fn log_found(&self, package: &str, version: &str, path: Option<&Path>) {
        if let Some(p) = path {
            self.log(&format!("FOUND: {package} {version} at {}", p.display()));
        } else {
            self.log(&format!("FOUND: {package} {version} (virtual)"));
        }
    }

    /// Log a package-not-found event.
    pub fn log_not_found(&self, package: &str) {
        self.log(&format!("NOT_FOUND: {package}"));
    }

    /// Log a dependency relationship.
    pub fn log_dependency(&self, parent: &str, dep_spec: &str) {
        self.log(&format!("DEPENDENCY: {parent} requires {dep_spec}"));
    }

    /// Log a version mismatch.
    pub fn log_version_mismatch(
        &self,
        package: &str,
        found: &str,
        required: &str,
        comparator: &str,
    ) {
        self.log(&format!(
            "VERSION_MISMATCH: {package} {found} does not satisfy {comparator} {required}"
        ));
    }

    /// Log a conflict detection.
    pub fn log_conflict(&self, package: &str, conflicts_with: &str) {
        self.log(&format!(
            "CONFLICT: {package} conflicts with {conflicts_with}"
        ));
    }

    /// Log a provider resolution (when a package is satisfied by another's Provides).
    pub fn log_provider(&self, requested: &str, provider: &str) {
        self.log(&format!(
            "PROVIDER: {requested} satisfied by provider {provider}"
        ));
    }

    /// Log the start of a solve operation.
    pub fn log_solve_start(&self, queries: &[String]) {
        self.log(&format!("SOLVE_START: {}", queries.join(", ")));
    }

    /// Log the completion of a solve operation.
    pub fn log_solve_end(&self, success: bool) {
        if success {
            self.log("SOLVE_END: success");
        } else {
            self.log("SOLVE_END: failure");
        }
    }

    /// Log collected flags output.
    pub fn log_flags(&self, flag_type: &str, flags: &str) {
        self.log(&format!("FLAGS({flag_type}): {flags}"));
    }

    /// Generate a simple timestamp string.
    ///
    /// Uses a basic format without depending on external crates.
    fn timestamp() -> String {
        // Use a monotonic-style identifier since we don't want to pull in
        // a datetime crate. In practice, the ordering of log entries is
        // what matters most for debugging.
        //
        // We use the system time as seconds since epoch for a reasonable
        // human-readable timestamp.
        use std::time::SystemTime;

        match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => {
                let secs = d.as_secs();
                // Simple UTC breakdown (not accounting for leap seconds,
                // which is fine for logging purposes).
                let days = secs / 86400;
                let time_secs = secs % 86400;
                let hours = time_secs / 3600;
                let minutes = (time_secs % 3600) / 60;
                let seconds = time_secs % 60;

                // Compute year/month/day from days since epoch (1970-01-01)
                let (year, month, day) = days_to_ymd(days);

                format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
            }
            Err(_) => "unknown".to_string(),
        }
    }
}

/// Convert days since Unix epoch to (year, month, day).
///
/// This is a simplified civil calendar calculation suitable for logging.
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
