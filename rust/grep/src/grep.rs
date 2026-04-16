use std::fs;
use std::io::{self, BufRead, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process;

use crate::args::{ColorMode, Options};
use crate::matcher::Matcher;

/// Apply color highlighting to a line by wrapping matched portions in ANSI escape codes.
pub(crate) fn colorize_line(line: &str, matcher: &Matcher, color_code: &str) -> String {
    let color_start = format!("\x1b[{}m\x1b[K", color_code);
    let color_end = "\x1b[m\x1b[K";

    let matches: Vec<_> = matcher
        .find_matches(line)
        .into_iter()
        .filter(|(s, e)| s != e) // skip empty matches
        .collect();
    if matches.is_empty() {
        return line.to_string();
    }

    let mut result = String::with_capacity(line.len() + matches.len() * 20);
    let mut last_end = 0;

    for (start, end) in matches {
        if start < last_end {
            continue; // skip overlapping matches
        }
        result.push_str(&line[last_end..start]);
        result.push_str(&color_start);
        result.push_str(&line[start..end]);
        result.push_str(color_end);
        last_end = end;
    }
    result.push_str(&line[last_end..]);
    result
}

/// Returns (match_count, matched, bytes_consumed)
pub(crate) fn grep_reader<R: BufRead>(
    mut reader: R,
    matcher: &Matcher,
    opts: &Options,
    filename: &str,
) -> (usize, bool, usize) {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut match_count: usize = 0;
    let mut byte_offset: usize = 0;

    let show_filename = opts.with_filename && !opts.no_filename;
    let separator = if opts.null_separator { '\0' } else { ':' };
    let fname_sep = if opts.null_separator { '\0' } else { ':' };
    let use_color = opts.color == ColorMode::Always;
    let line_delim: u8 = if opts.null_data { b'\0' } else { b'\n' };
    let mut write_error = false;

    macro_rules! checked_write {
        ($dst:expr, $($arg:tt)*) => {
            if !write_error {
                if write!($dst, $($arg)*).is_err() {
                    write_error = true;
                }
            }
        }
    }
    macro_rules! checked_writeln {
        ($dst:expr, $($arg:tt)*) => {
            if !write_error {
                if writeln!($dst, $($arg)*).is_err() {
                    write_error = true;
                }
            }
        }
    }

    // Binary file detection: peek at the first chunk for NUL bytes
    let mut is_binary = false;
    if !opts.null_data && !opts.text_mode {
        let buf = reader.fill_buf().unwrap_or(&[]);
        if buf.contains(&0) {
            is_binary = true;
        }
    }

    let has_context = opts.context_requested || opts.before_context > 0 || opts.after_context > 0;

    if has_context
        && !opts.count
        && !opts.files_with_matches
        && !opts.files_without_match
        && !opts.only_matching
    {
        // Context mode: collect all lines first (read raw bytes for non-UTF-8)
        let mut lines = Vec::new();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(line_delim, &mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    if buf.last() == Some(&line_delim) {
                        buf.pop();
                    }
                    lines.push(String::from_utf8_lossy(&buf).into_owned());
                }
                Err(_) => break,
            }
        }
        let mut remaining_after: usize = 0;
        let mut last_printed: Option<usize> = None;

        let max_reached = |count: usize| opts.max_count.is_some_and(|max| count >= max);

        for (line_idx, line) in lines.iter().enumerate() {
            let matches = matcher.is_match(line) != opts.invert_match;

            if matches && !max_reached(match_count) {
                match_count += 1;

                // Print before context
                let ctx_start = line_idx.saturating_sub(opts.before_context);
                #[allow(clippy::needless_range_loop)]
                for ctx_idx in ctx_start..line_idx {
                    if last_printed.is_some_and(|lp| ctx_idx <= lp) {
                        continue;
                    }
                    if last_printed.is_some_and(|lp| ctx_idx > lp + 1) {
                        checked_writeln!(out, "--");
                    }
                    print_context_line(
                        &mut out,
                        &lines[ctx_idx],
                        ctx_idx + 1,
                        filename,
                        show_filename,
                        opts,
                    );
                    last_printed = Some(ctx_idx);
                }

                // Print separator if needed
                if last_printed.is_some_and(|lp| line_idx > lp + 1) {
                    checked_writeln!(out, "--");
                }

                // Print matching line
                let has_prefix = show_filename || opts.line_number;
                if show_filename {
                    checked_write!(out, "{filename}{fname_sep}");
                }
                if opts.line_number {
                    checked_write!(out, "{}{separator}", line_idx + 1);
                }
                if opts.initial_tab && has_prefix && !line.is_empty() {
                    checked_write!(out, "\t");
                }
                if use_color {
                    checked_writeln!(out, "{}", colorize_line(line, matcher, &opts.match_color));
                } else {
                    checked_writeln!(out, "{line}");
                }
                last_printed = Some(line_idx);
                remaining_after = opts.after_context;
            } else if remaining_after > 0 {
                if last_printed.is_some_and(|lp| line_idx > lp + 1) {
                    checked_writeln!(out, "--");
                }
                print_context_line(&mut out, line, line_idx + 1, filename, show_filename, opts);
                last_printed = Some(line_idx);
                remaining_after -= 1;
            } else if max_reached(match_count) {
                break;
            }
        }

        return (match_count, match_count > 0, 0);
    }

    // Non-context mode: stream lines (read raw bytes for non-UTF-8 support)
    let mut line_idx = 0;
    let mut line_buf = Vec::new();
    loop {
        line_buf.clear();
        let bytes_read = match reader.read_until(line_delim, &mut line_buf) {
            Ok(n) => n,
            Err(_) => break,
        };
        if bytes_read == 0 {
            break;
        }
        // Strip trailing delimiter
        if line_buf.last() == Some(&line_delim) {
            line_buf.pop();
        }
        let line_len = line_buf.len() + 1;
        let line = String::from_utf8_lossy(&line_buf);

        // Check for binary content in this line and upcoming data
        if !is_binary && !opts.null_data && !opts.text_mode {
            if line_buf.contains(&0) {
                is_binary = true;
            } else if let Ok(upcoming) = reader.fill_buf() {
                if upcoming.contains(&0) {
                    is_binary = true;
                }
            }
        }

        let matches = matcher.is_match(&line) != opts.invert_match;

        if matches {
            if let Some(max) = opts.max_count
                && match_count >= max
            {
                break;
            }
            match_count += 1;

            if opts.quiet {
                return (match_count, true, byte_offset);
            }

            if opts.files_with_matches || opts.files_without_match {
                return (match_count, true, byte_offset);
            }

            if is_binary {
                eprintln!("grep: {filename}: binary file matches");
                return (match_count, true, byte_offset);
            }

            if !opts.count {
                if opts.only_matching && !opts.invert_match {
                    let found: Vec<_> = matcher
                        .find_matches(&line)
                        .into_iter()
                        .filter(|(s, e)| s != e) // skip empty matches
                        .collect();
                    for (start, end) in found {
                        if show_filename {
                            checked_write!(out, "{filename}{fname_sep}");
                        }
                        if opts.line_number {
                            checked_write!(out, "{}{separator}", line_idx + 1);
                        }
                        if opts.byte_offset {
                            checked_write!(out, "{}{separator}", byte_offset + start);
                        }
                        let line_end = if opts.null_data { "\0" } else { "\n" };
                        if use_color {
                            checked_write!(
                                out,
                                "\x1b[{}m\x1b[K{}\x1b[m\x1b[K{line_end}",
                                opts.match_color,
                                &line[start..end]
                            );
                        } else {
                            checked_write!(out, "{}{line_end}", &line[start..end]);
                        }
                    }
                } else {
                    let has_prefix = show_filename || opts.line_number || opts.byte_offset;
                    if show_filename {
                        checked_write!(out, "{filename}{fname_sep}");
                    }
                    if opts.line_number {
                        if opts.initial_tab && show_filename {
                            checked_write!(out, " ");
                        }
                        checked_write!(out, "{}{separator}", line_idx + 1);
                    }
                    if opts.byte_offset {
                        checked_write!(out, "{byte_offset}{separator}");
                    }
                    if opts.initial_tab && has_prefix && !line.is_empty() {
                        checked_write!(out, "\t");
                    }
                    if use_color {
                        checked_writeln!(out, "{}", colorize_line(&line, matcher, &opts.match_color));
                    } else {
                        // Write raw bytes to preserve non-UTF-8 content
                        if !write_error && out.write_all(&line_buf).is_err() { write_error = true; }
                        if !write_error && out.write_all(if opts.null_data { b"\0" } else { b"\n" }).is_err() { write_error = true; }
                    }
                }
            }
        }

        byte_offset += line_len;
        line_idx += 1;
    }

    if write_error {
        drop(out);
        eprintln!("grep: write error: {}", io::Error::last_os_error());
        process::exit(2);
    }

    (match_count, match_count > 0, byte_offset)
}

pub(crate) fn print_context_line<W: Write>(
    out: &mut W,
    line: &str,
    line_num: usize,
    filename: &str,
    show_filename: bool,
    opts: &Options,
) {
    let has_prefix = show_filename || opts.line_number;
    if show_filename {
        let _ = write!(out, "{filename}-");
    }
    if opts.line_number {
        let _ = write!(out, "{line_num}-");
    }
    if opts.initial_tab && has_prefix && !line.is_empty() {
        let _ = write!(out, "\t");
    }
    let _ = writeln!(out, "{line}");
}

/// Check if a file path refers to the same file as stdout (inode comparison).
#[cfg(unix)]
pub(crate) fn is_input_same_as_stdout(path: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let stdout = io::stdout();
    let stdout_fd = stdout.lock().as_raw_fd();
    let stdout_meta = unsafe {
        let mut stat: libc::stat = std::mem::zeroed();
        if libc::fstat(stdout_fd, &mut stat) != 0 {
            return false;
        }
        (stat.st_dev, stat.st_ino)
    };
    if let Ok(file_meta) = fs::metadata(path) {
        return (file_meta.dev(), file_meta.ino()) == stdout_meta;
    }
    false
}

#[cfg(not(unix))]
pub(crate) fn is_input_same_as_stdout(_path: &Path) -> bool {
    false
}

/// Returns (match_count, matched, had_error)
pub(crate) fn grep_file(path: &Path, matcher: &Matcher, opts: &Options) -> (usize, bool, bool) {
    let filename = if path.as_os_str() == "-" {
        opts.label.clone()
    } else {
        path.to_string_lossy().to_string()
    };

    // Check if input file is also the output (would cause infinite loop)
    let check_same = !opts.quiet
        && !opts.files_with_matches
        && !opts.files_without_match
        && opts.max_count.is_none();
    if check_same && path.as_os_str() != "-" && is_input_same_as_stdout(path) {
        eprintln!("grep: {}: input file is also the output", path.display());
        return (0, false, true);
    }
    #[cfg(unix)]
    if check_same && path.as_os_str() == "-" {
        // Check if stdin is the same as stdout
        use std::os::unix::io::AsRawFd;
        let stdin_fd = io::stdin().as_raw_fd();
        let stdout_fd = io::stdout().lock().as_raw_fd();
        unsafe {
            let mut stdin_stat: libc::stat = std::mem::zeroed();
            let mut stdout_stat: libc::stat = std::mem::zeroed();
            if libc::fstat(stdin_fd, &mut stdin_stat) == 0
                && libc::fstat(stdout_fd, &mut stdout_stat) == 0
                && stdin_stat.st_dev == stdout_stat.st_dev
                && stdin_stat.st_ino == stdout_stat.st_ino
                && stdin_stat.st_ino != 0
            {
                eprintln!("grep: (standard input): input file is also the output");
                return (0, false, true);
            }
        }
    }

    if path.as_os_str() == "-" {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let (count, matched, bytes_consumed) = grep_reader(reader, matcher, opts, &filename);
        // If -m was used, seek stdin to allow subsequent processes to read the rest
        #[cfg(unix)]
        if opts.max_count.is_some() {
            use std::os::unix::io::AsRawFd;
            let fd = io::stdin().as_raw_fd();
            unsafe {
                let result = libc::lseek(fd, bytes_consumed as libc::off_t, libc::SEEK_SET);
                let _ = result; // ignore seek failures (pipes, etc.)
            }
        }
        return (count, matched, false);
    }

    // Skip device files when -D skip is used
    if opts.skip_devices && path.as_os_str() != "-" {
        if let Ok(metadata) = fs::metadata(path) {
            let ft = metadata.file_type();
            if !ft.is_file() && !ft.is_dir() && !ft.is_symlink() {
                return (0, false, false);
            }
        }
    }

    // Skip directories when --directories=skip is used (non-recursive mode)
    if opts.skip_directories && path.is_dir() {
        return (0, false, false);
    }

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            if !opts.no_messages {
                eprintln!("grep: {}: {e}", path.display());
            }
            return (0, false, true);
        }
    };

    let reader = io::BufReader::new(file);
    let (count, matched, _) = grep_reader(reader, matcher, opts, &filename);
    (count, matched, false)
}
