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
        // Smart terminal draws an in-place "in-progress" line on start.
        // Piped (and verbose-on-smart) modes wait until completion to
        // emit a durable line.
        if matches!(self.mode, Mode::SmartTerminal) && !self.verbose {
            self.draw_status(description);
        }
    }

    /// Called after an edge finishes. Bumps `finished`, then:
    ///   - In smart-terminal mode, redraws the status with the new count
    ///     (still showing the just-finished edge's description), then
    ///     dumps any captured output below it.
    ///   - In piped mode, writes one fresh status line followed by the
    ///     captured output (ANSI stripped unless `CLICOLOR_FORCE=1`).
    pub fn build_finished(&mut self, description: &str, output: &[u8]) {
        self.finished += 1;
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
