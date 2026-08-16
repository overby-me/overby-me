//! test-sleep — sleep for a given timespan (default: forever).
//!
//! A Rust equivalent of the C `test-sleep` manual test binary from
//! upstream systemd. Integration tests (TEST-87 coredump, TEST-82
//! softreboot) use it as a real ELF binary that can be forked, left in an
//! interruptible sleep, and then signalled to produce a coredump. Because
//! the coredump matching keys on the executable path, it must be a genuine
//! binary (not a shell script that execs `sleep`).
//!
//! Usage: test-sleep [TIMESPAN]
//!   TIMESPAN  a systemd-style timespan (e.g. "10", "5s", "2min", "infinity").
//!             Absent or "infinity" means sleep until signalled.

use std::time::Duration;

fn main() {
    // Mirror upstream: argv[1] is a timespan; absent or "infinity" => forever.
    match std::env::args().nth(1).as_deref() {
        None | Some("infinity") => loop {
            // Sleep in an interruptible state (nanosleep) until a signal
            // arrives; re-arm periodically so a spurious wakeup can't exit.
            std::thread::sleep(Duration::from_secs(3600));
        },
        Some(s) => std::thread::sleep(parse_timespan(s)),
    }
}

/// Parse a minimal subset of systemd timespans: a bare number of seconds or
/// a value with a trailing unit (s, min, h, d). Unparseable input falls back
/// to a long sleep, matching the "sleep until killed" intent.
fn parse_timespan(s: &str) -> Duration {
    let s = s.trim();
    let (num, mult): (&str, u64) = if let Some(v) = s.strip_suffix("min") {
        (v, 60)
    } else if let Some(v) = s.strip_suffix('h') {
        (v, 3600)
    } else if let Some(v) = s.strip_suffix('d') {
        (v, 86400)
    } else if let Some(v) = s.strip_suffix('s') {
        (v, 1)
    } else {
        (s, 1)
    };
    match num.trim().parse::<u64>() {
        Ok(n) => Duration::from_secs(n.saturating_mul(mult)),
        Err(_) => Duration::from_secs(3600 * 24 * 365),
    }
}
