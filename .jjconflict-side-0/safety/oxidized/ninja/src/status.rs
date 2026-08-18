//! Status line printer.
//!
//! Two display modes:
//!   - **Smart terminal** (TTY on stdout): use `\r{status}\x1b[K` to redraw
//!     in place, and let edge output inherit the terminal so colors flow
//!     through unchanged.
//!   - **Piped**: print one status line per edge with a trailing newline,
//!     and strip ANSI escapes from edge output unless `CLICOLOR_FORCE=1`
//!     overrides that.
//!
//! Driven by `NINJA_STATUS` (currently we honor the entire string with
//! `%f` and `%t` substitutions; an empty value suppresses the prefix
//! entirely).

use std::io::{IsTerminal, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    SmartTerminal,
    Piped,
}

impl Mode {
    pub fn detect() -> Self {
        // Match ninja's line_printer.cc: smart terminal requires
        // isatty(1) AND `term` env var present (non-NULL) AND not equal
        // to "dumb". Crucially, an explicitly-empty TERM ("") is still
        // "smart" — it's only an UNSET TERM that disables smart mode.
        // The upstream output_test.py default_env sets TERM='' so the
        // standard tests do exercise smart-terminal rendering, while
        // tests that pass a custom env without TERM (e.g.
        // test_issue_2586 with env={'NINJA_STATUS':''}) end up in the
        // dumb/piped fallback.
        match std::env::var("TERM") {
            Ok(t) if t != "dumb" && std::io::stdout().is_terminal() => Mode::SmartTerminal,
            _ => Mode::Piped,
        }
    }
}

pub struct Status {
    pub mode: Mode,
    pub quiet: bool,
    pub verbose: bool,
    pub format: String,
    pub total: usize,
    pub finished: usize,
    /// Whether the last write to stdout left the cursor mid-line on a
    /// `\r…\x1b[K` redraw, meaning we owe a `\n` before any non-status
    /// output (or at the very end of the build).
    needs_newline: bool,
    /// True while a `pool = console` edge owns the terminal. While set,
    /// `build_started` is a no-op (no in-place status redraw racing
    /// against the child process's own writes) and `build_finished` for
    /// non-console edges queues into `pending` instead of writing.
    console_locked: bool,
    /// Buffered (description, captured-output) from non-console edges
    /// that completed while the console was locked. Drained on
    /// `unlock_console`.
    pending: Vec<(String, Vec<u8>)>,
}

impl Status {
    pub fn new(mode: Mode, quiet: bool, verbose: bool, total: usize) -> Self {
        let format = std::env::var("NINJA_STATUS").unwrap_or_else(|_| "[%f/%t] ".into());
        Self {
            mode,
            quiet,
            verbose,
            format,
            total,
            finished: 0,
            needs_newline: false,
            console_locked: false,
            pending: Vec::new(),
        }
    }

    /// Returns true while a console-pool edge is running and owns the
    /// terminal. Other (non-console) edge completions are buffered
    /// during this window and flushed when the console unlocks.
    pub fn console_locked(&self) -> bool {
        self.console_locked
    }

    /// Mark the console as locked: a console-pool edge is about to run
    /// with stdin/stdout/stderr inherited, so we must not redraw any
    /// in-place status line on top of its output. Any pending in-place
    /// redraw is broken with a newline first so prior status doesn't
    /// get clobbered by the child.
    pub fn lock_console(&mut self) {
        self.break_line();
        self.console_locked = true;
    }

    /// Release the console lock and drain any non-console completions
    /// that were buffered during the lock. Each is rendered the same
    /// way `build_finished` would have rendered it had it not been
    /// deferred.
    pub fn unlock_console(&mut self) {
        self.console_locked = false;
        let pending = std::mem::take(&mut self.pending);
        for (desc, output) in pending {
            self.render_finished(&desc, &output);
        }
    }

    /// Format the status prefix using `NINJA_STATUS`. Recognizes `%f`
    /// (finished count), `%t` (total), and `%%` (literal `%`). Anything
    /// else passes through unchanged.
    fn render_prefix(&self) -> String {
        let mut out = String::new();
        let bytes = self.format.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'f' => out.push_str(&self.finished.to_string()),
                    b't' => out.push_str(&self.total.to_string()),
                    b'%' => out.push('%'),
                    other => {
                        out.push('%');
                        out.push(other as char);
                    }
                }
                i += 2;
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    }

    /// Print the status line for an edge that is about to start.
    /// Smart-terminal mode draws an in-place `[F/T] desc` (where F is the
    /// number of edges already finished). Piped mode does nothing on
    /// start — it only emits the post-completion line so logs stay
    /// uncluttered, matching ninja's `LinePrinter::Print` behavior.
    pub fn build_started(&mut self, description: &str) {
        if self.quiet {
            return;
        }
        // While a console-pool edge owns the terminal, suppress all
        // status redraws — the child is writing directly to the same
        // stdout we'd be drawing on.
        if self.console_locked {
            return;
        }
        // Smart terminal draws an in-place "in-progress" line on start.
        // Piped (and verbose-on-smart) modes wait until completion to
        // emit a durable line.
        if matches!(self.mode, Mode::SmartTerminal) && !self.verbose {
            self.draw_status(description);
        }
    }

    /// Called after a *non-console* edge finishes. Bumps `finished`,
    /// then either writes (status + captured output) immediately or, if
    /// the console is currently locked by a console-pool edge, queues
    /// the (description, output) pair to be flushed when the console
    /// unlocks. This preserves the upstream semantics where a console
    /// edge's terminal output is never interleaved with a competing
    /// edge's captured stdout.
    pub fn build_finished(&mut self, description: &str, output: &[u8]) {
        self.finished += 1;
        if self.console_locked {
            self.pending
                .push((description.to_string(), output.to_vec()));
            return;
        }
        self.render_finished(description, output);
    }

    /// Called after the console-pool edge itself finishes. Bumps
    /// `finished` and releases the console lock (which drains any
    /// queued non-console completions). The corresponding status line
    /// for this edge is emitted up-front by `build_started_console`
    /// so the child's subsequent terminal output appears beneath it.
    pub fn build_finished_console(&mut self, _description: &str) {
        self.finished += 1;
        self.unlock_console();
    }

    /// Print the status line for a console-pool edge that is about to
    /// run. Always prints (even in piped mode), because the console
    /// edge owns the terminal and its output will land directly on
    /// stdout — we want the user to see what is running before its
    /// output appears.
    pub fn build_started_console(&mut self, description: &str) {
        if self.quiet {
            return;
        }
        // Force one durable line; smart-terminal's in-place redraw
        // would just be clobbered by the child's own writes.
        let prefix = self.render_prefix();
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{prefix}{description}");
        let _ = stdout.flush();
        self.needs_newline = false;
    }

    /// Inner helper shared by `build_finished` and `unlock_console`.
    fn render_finished(&mut self, description: &str, output: &[u8]) {
        if self.quiet {
            self.write_output(output);
            return;
        }
        match self.mode {
            Mode::SmartTerminal => {
                // Redraw IN PLACE (overwriting the started line) so the
                // before/after pair collapses to a single visible row.
                self.draw_status(description);
                if !output.is_empty() {
                    self.break_line();
                    self.write_output(output);
                }
            }
            Mode::Piped => {
                self.draw_status(description);
                self.write_output(output);
            }
        }
    }

    /// Final flush: if we're sitting on an in-place `\r…\x1b[K` line,
    /// terminate it with a newline so the shell prompt lands on its
    /// own line.
    pub fn finish(&mut self) {
        if self.needs_newline {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout);
            let _ = stdout.flush();
            self.needs_newline = false;
        }
    }

    /// Write the status line for the current state. Smart terminal uses
    /// in-place redraw; piped emits a fresh line each time.
    fn draw_status(&mut self, description: &str) {
        let prefix = self.render_prefix();
        let mut stdout = std::io::stdout().lock();
        // Verbose mode forces line-by-line printing even on a smart
        // terminal: we want one durable status row per edge (matching
        // ninja's `--verbose` log format used by `test_issue_1214`).
        let smart = matches!(self.mode, Mode::SmartTerminal) && !self.verbose;
        if smart {
            let _ = write!(stdout, "\r{prefix}{description}\x1b[K");
            let _ = stdout.flush();
            self.needs_newline = true;
        } else {
            let _ = writeln!(stdout, "{prefix}{description}");
            let _ = stdout.flush();
            self.needs_newline = false;
        }
    }

    /// Move the cursor off any in-place status line so the next bytes
    /// land on a fresh row.
    fn break_line(&mut self) {
        if self.needs_newline {
            let mut stdout = std::io::stdout().lock();
            let _ = writeln!(stdout);
            let _ = stdout.flush();
            self.needs_newline = false;
        }
    }

    /// Emit `output` to stdout, stripping ANSI when piped (unless
    /// `CLICOLOR_FORCE=1`). Always ends on a newline.
    fn write_output(&mut self, output: &[u8]) {
        if output.is_empty() {
            return;
        }
        let mut stdout = std::io::stdout().lock();
        let buf: &[u8] = match self.mode {
            Mode::SmartTerminal => output,
            Mode::Piped => {
                let force = std::env::var("CLICOLOR_FORCE")
                    .map(|v| v == "1")
                    .unwrap_or(false);
                if force {
                    output
                } else {
                    let owned = strip_ansi(output);
                    let _ = stdout.write_all(&owned);
                    if !owned.ends_with(b"\n") {
                        let _ = writeln!(stdout);
                    }
                    let _ = stdout.flush();
                    return;
                }
            }
        };
        let _ = stdout.write_all(buf);
        if !buf.ends_with(b"\n") {
            let _ = writeln!(stdout);
        }
        let _ = stdout.flush();
    }
}

/// Remove ANSI CSI escape sequences (`\x1b[...m` etc.) from `s`. Used in
/// piped mode so colored child-process output doesn't leak control codes
/// to log files.
pub fn strip_ansi(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == 0x1b && i + 1 < s.len() && s[i + 1] == b'[' {
            i += 2;
            while i < s.len() && !(0x40..=0x7e).contains(&s[i]) {
                i += 1;
            }
            if i < s.len() {
                i += 1;
            }
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_console_buffers_pending_completions_until_unlock() {
        // Use Piped so we don't try to interact with a real TTY; that
        // also means draw_status writes a full line, which is fine for
        // the bookkeeping we're checking here.
        let mut s = Status::new(
            Mode::Piped,
            /*quiet=*/ true,
            /*verbose=*/ false,
            3,
        );
        assert!(!s.console_locked());

        s.lock_console();
        assert!(s.console_locked());

        // While locked, a non-console completion bumps `finished` but
        // queues into `pending` rather than writing.
        s.build_finished("non-console A", b"out-A");
        s.build_finished("non-console B", b"");
        assert_eq!(s.finished, 2);
        assert_eq!(s.pending.len(), 2);
        assert_eq!(s.pending[0].0, "non-console A");
        assert_eq!(s.pending[0].1, b"out-A");

        // build_started while locked must be a no-op.
        s.build_started("would-be redraw");

        // Unlocking via the console-edge completion path drains the
        // queue and bumps `finished` for the console edge itself.
        s.build_finished_console("the console edge");
        assert!(!s.console_locked());
        assert!(s.pending.is_empty());
        assert_eq!(s.finished, 3);
    }

    #[test]
    fn unlock_console_directly_drains_pending() {
        let mut s = Status::new(Mode::Piped, true, false, 2);
        s.lock_console();
        s.build_finished("a", b"");
        assert_eq!(s.pending.len(), 1);
        s.unlock_console();
        assert!(s.pending.is_empty());
        assert!(!s.console_locked());
    }
}
