//! Logging utilities matching direnv's log format with ANSI color support.

use crate::config::DEFAULT_LOG_FORMAT;

const ERROR_COLOR: &str = "\x1b[31m";
const CLEAR_COLOR: &str = "\x1b[0m";

/// Log a status message to stderr in direnv format.
pub fn log_status(format: &str, color: bool, msg: &str) {
    if format.is_empty() {
        return;
    }
    let line = if color {
        format!("{CLEAR_COLOR}{}", format.replace("%s", msg))
    } else {
        format.replace("%s", msg)
    };
    eprintln!("{line}");
}

/// Log an error message to stderr in direnv format.
pub fn log_error(format: &str, color: bool, msg: &str) {
    let fmt = if format.is_empty() {
        DEFAULT_LOG_FORMAT
    } else {
        format
    };
    let formatted = fmt.replace("%s", &format!("error {msg}"));
    let line = if color {
        format!("{ERROR_COLOR}{formatted}{CLEAR_COLOR}")
    } else {
        formatted
    };
    eprintln!("{line}");
}

/// Log a status message using the default format (no color).
pub fn log_status_default(msg: &str) {
    log_status(DEFAULT_LOG_FORMAT, false, msg);
}

/// Log an error using the default format (no color).
pub fn log_error_default(msg: &str) {
    log_error(DEFAULT_LOG_FORMAT, false, msg);
}
