use super::*;
use crate::builtins::help_data::HELP_ENTRIES;
use crate::builtins::string_to_raw_bytes;
use crate::interpreter::AssocArray;

pub(super) fn builtin_eval(shell: &mut Shell, args: &[String]) -> i32 {
    // Check for invalid options
    if let Some(first) = args.first()
        && first.starts_with('-')
        && first.len() > 1
        && first != "--"
    {
        eprintln!("{}: eval: {}: invalid option", shell.error_prefix(), first);
        eprintln!("eval: usage: eval [arg ...]");
        return 2;
    }
    let command = args.join(" ");
    // Save procsub fds so inner run_simple_command calls don't close them
    let saved_fds = crate::expand::take_procsub_fds();

    // Parse and check for leftover tokens (eval-specific)
    let mut parser = crate::parser::Parser::new_with_aliases(
        &command,
        shell.aliases.clone(),
        shell.shopt_expand_aliases,
        shell.opt_posix,
    );
    // Set the eval parser's line offset to the current LINENO
    // so error messages reference the correct source file line
    let lineno_offset: usize = shell
        .vars
        .get("LINENO")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    parser.set_line_offset(lineno_offset.saturating_sub(1));
    let result = match parser.parse_program() {
        Ok(program) => {
            if !parser.is_at_eof() {
                let token_desc = parser.current_token_str();
                let name = shell
                    .positional
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                let lineno = shell
                    .vars
                    .get("LINENO")
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                eprintln!(
                    "{}: eval: line {}: syntax error near unexpected token `{}'",
                    name, lineno, token_desc
                );
                eprintln!("{}: eval: line {}: `{}'", name, lineno, command.trim());
                2
            } else {
                // Check for incomplete funsub in the AST
                let has_incomplete = program_has_incomplete_funsub(&program);
                if has_incomplete {
                    let name = shell
                        .positional
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("bash");
                    let lineno = shell
                        .cmd_end_line
                        .or_else(|| {
                            shell
                                .vars
                                .get("LINENO")
                                .and_then(|s| s.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    eprintln!(
                        "{}: eval: line {}: unexpected EOF while looking for matching `}}'",
                        name, lineno
                    );
                    2
                } else {
                    shell.run_program(&program)
                }
            }
        }
        Err(e) => {
            // Check for compound command context in the eval parse error
            if let Some((cmd, cmd_line)) = parser.compound_cmd_context() {
                let name = shell
                    .positional
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                // Use cmd_end_line + 1 for eval error position
                // (bash reports eval errors at the post-parse position)
                let eval_line = shell
                    .cmd_end_line
                    .map(|end| end + 2)
                    .unwrap_or_else(|| parser.current_line());
                eprintln!(
                    "{}: eval: line {}: syntax error: unexpected end of file from `{}' command on line {}",
                    name, eval_line, cmd, cmd_line
                );
                return 2;
            }
            let name = shell
                .positional
                .first()
                .map(|s| s.as_str())
                .unwrap_or("bash");
            let lineno: usize = shell
                .vars
                .get("LINENO")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if name == "bash" || name.is_empty() {
                eprintln!("{}: eval: {}", shell.error_prefix(), e);
            } else {
                eprintln!("{}: eval: line {}: {}", name, lineno, e);
            }
            if e.contains("syntax error") {
                let cmd = &command;
                if name == "bash" || name.is_empty() {
                    eprintln!(
                        "{}: eval: line {}: `{}'",
                        shell.error_prefix(),
                        lineno,
                        cmd.trim()
                    );
                } else {
                    eprintln!("{}: eval: line {}: `{}'", name, lineno, cmd.trim());
                }
            }
            2
        }
    };

    // Restore saved fds (they'll be closed by the caller)
    for fd in saved_fds {
        crate::expand::register_procsub_fd_pub(fd);
    }
    result
}

pub(super) fn builtin_exec(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    // Parse exec flags: -a NAME (set argv[0]), -c (clear env), -l (login shell)
    let mut argv0_override: Option<String> = None;
    let mut clear_env = false;
    let mut login_shell = false;
    let mut cmd_start = 0;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-a" => {
                i += 1;
                if i < args.len() {
                    argv0_override = Some(args[i].clone());
                }
            }
            "-c" => clear_env = true,
            "-l" => {
                login_shell = true;
            }
            s if s.starts_with('-') && s.len() > 1 => {
                eprintln!("{}: exec: {}: invalid option", shell.error_prefix(), s);
                eprintln!(
                    "exec: usage: exec [-cl] [-a name] [command [argument ...]] [redirection ...]"
                );
                return 2;
            }
            _ => {
                cmd_start = i;
                break;
            }
        }
        i += 1;
        cmd_start = i;
    }

    if cmd_start >= args.len() {
        return 0;
    }

    let program = &args[cmd_start];
    let mut cmd_args: Vec<String> = args[cmd_start..].to_vec();
    if let Some(ref a0) = argv0_override {
        cmd_args[0] = a0.clone();
    }
    // -l flag: prefix argv[0] with '-' to indicate login shell
    if login_shell {
        cmd_args[0] = format!("-{}", cmd_args[0]);
    }

    // Resolve the executable path BEFORE clearing the environment,
    // so that PATH is still available for lookup (important in Nix sandboxes).
    #[cfg(unix)]
    let resolved_path = find_executable(program);

    // Set up environment
    if clear_env {
        for (key, _) in std::env::vars() {
            if !key.is_empty() {
                unsafe { std::env::remove_var(&key) };
            }
        }
    } else {
        for (key, value) in &shell.exports {
            unsafe { std::env::set_var(key, value) };
        }
    }

    #[cfg(unix)]
    {
        use std::ffi::CString;

        let path = resolved_path;
        let c_prog = CString::new(path.as_bytes()).unwrap();
        let c_args: Vec<CString> = cmd_args
            .iter()
            .map(|a| CString::new(a.as_bytes()).unwrap())
            .collect();

        nix::unistd::execvp(&c_prog, &c_args).ok();
        let err = std::io::Error::last_os_error();
        let code = if err.kind() == std::io::ErrorKind::NotFound {
            127
        } else {
            126
        };
        eprintln!(
            "{}: exec: {}: {}",
            shell.error_prefix(),
            program,
            io_error_message(&err)
        );
        // exec failure is fatal — exit the shell/subshell
        std::io::Write::flush(&mut std::io::stdout()).ok();
        std::io::Write::flush(&mut std::io::stderr()).ok();
        std::process::exit(code);
    }

    #[cfg(not(unix))]
    {
        eprintln!(
            "{}: exec: not supported on this platform",
            shell.error_prefix()
        );
        1
    }
}

pub(super) fn builtin_source(shell: &mut Shell, args: &[String]) -> i32 {
    // Use "." or "source" as the command name (never "command")
    let cmd = match shell.current_builtin.as_deref() {
        Some("source") => "source",
        _ => ".",
    };
    // Handle options
    let mut file_start = 0;
    if let Some((i, arg)) = args.iter().enumerate().next() {
        if arg == "--" {
            file_start = i + 1;
        } else if arg.starts_with('-') && arg.len() > 1 {
            eprintln!("{}: {}: {}: invalid option", shell.error_prefix(), cmd, arg);
            eprintln!("{}: usage: {} [-p path] filename [arguments]", cmd, cmd);
            return 2;
        } else {
            file_start = i;
        }
    }
    if file_start >= args.len() || args.is_empty() {
        eprintln!(
            "{}: {}: filename argument required",
            shell.error_prefix(),
            cmd
        );
        eprintln!("{}: usage: {} [-p path] filename [arguments]", cmd, cmd);
        return 2;
    }
    let args = &args[file_start..];
    if args.is_empty() {
        eprintln!(
            "{}: {}: filename argument required",
            shell.error_prefix(),
            cmd
        );
        eprintln!("{}: usage: {} [-p path] filename [arguments]", cmd, cmd);
        return 2;
    }

    let filename = &args[0];
    let path = if filename.contains('/') {
        filename.to_string()
    } else {
        // Search PATH
        find_in_path(filename)
    };

    shell.source_file_error = false;
    let read_result = (|| -> Result<String, std::io::Error> {
        use std::io::Read;
        let f = std::fs::File::open(&path)?;
        let meta = f.metadata()?;
        let len = meta.len();
        if len > 1 << 30 {
            return Err(std::io::Error::other(format!(
                "file too large ({} bytes)",
                len
            )));
        }
        let mut buf = Vec::with_capacity(len as usize);
        f.take(1 << 30).read_to_end(&mut buf)?;
        String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    })();
    match read_result {
        Ok(content) => {
            // Save and set positional parameters for the sourced script
            let saved_positional = shell.positional.clone();
            if args.len() > 1 {
                let prog = shell.positional.first().cloned().unwrap_or_default();
                shell.positional = vec![prog];
                shell.positional.extend(args[1..].to_vec());
                shell.source_set_params = false;
            }

            // Push source file onto BASH_SOURCE stack
            let bash_source = shell.arrays.entry("BASH_SOURCE".to_string()).or_default();
            bash_source.insert(0, Some(path.clone()));

            let saved_sourcing = shell.sourcing;
            shell.sourcing = true;
            let result = shell.run_string(&content);
            shell.returning = false; // reset return flag after sourced script
            shell.sourcing = saved_sourcing;

            // Pop BASH_SOURCE stack
            if let Some(arr) = shell.arrays.get_mut("BASH_SOURCE")
                && !arr.is_empty()
            {
                arr.remove(0);
            }

            // Run RETURN trap after sourced script completes
            shell.run_return_trap();

            // Only restore positional params if we set them for this source invocation
            // AND the sourced file didn't explicitly change them with `set --`
            if args.len() > 1 && !shell.source_set_params {
                shell.positional = saved_positional;
            }
            result
        }
        Err(e) => {
            if shell.opt_posix && !filename.contains('/') {
                // POSIX mode with bare name (PATH search): include command name
                // and use "file not found".  Paths with '/' use the regular format.
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    "file not found"
                } else {
                    io_error_message(&e)
                };
                eprintln!("{}: {}: {}: {}", shell.error_prefix(), cmd, filename, msg);
            } else {
                // Non-POSIX or path-based: no command name prefix, use OS error message
                let msg = io_error_message(&e);
                eprintln!("{}: {}: {}", shell.error_prefix(), filename, msg);
            }
            shell.source_file_error = true;
            1
        }
    }
}

pub(super) fn builtin_help(shell: &mut Shell, args: &[String]) -> i32 {
    let mut flag_d = false; // short description
    let mut flag_s = false; // short usage synopsis
    let mut flag_m = false; // man page format
    let mut patterns: Vec<String> = Vec::new();

    let mut i = 0;
    let mut past_opts = false;
    while i < args.len() {
        let arg = &args[i];
        if !past_opts && arg == "--" {
            past_opts = true;
            i += 1;
            continue;
        }
        if !past_opts && arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'd' => flag_d = true,
                    's' => flag_s = true,
                    'm' => flag_m = true,
                    _ => {
                        eprintln!("{}: help: -{}: invalid option", shell.error_prefix(), ch);
                        eprintln!("help: usage: help [-dms] [pattern ...]");
                        return 2;
                    }
                }
            }
        } else {
            patterns.push(arg.clone());
        }
        i += 1;
    }

    // If no patterns given, print the full builtin listing
    if patterns.is_empty() {
        help_print_listing(shell);
        return 0;
    }

    // Match patterns against help entries
    let mut status = 0;
    for pattern in &patterns {
        let is_glob = pattern.contains('*') || pattern.contains('?') || pattern.contains('[');

        let matches: Vec<&crate::builtins::help_data::HelpEntry> = HELP_ENTRIES
            .iter()
            .filter(|e| help_name_matches(e.name, pattern))
            .collect();

        if matches.is_empty() {
            eprintln!(
                "{}: help: no help topics match `{}'.  Try `help help' or `man -k {}' or `info {}'.",
                shell.error_prefix(),
                pattern,
                pattern,
                pattern
            );
            status = 1;
            continue;
        }

        // Bash prints a header when the pattern is a glob
        if is_glob {
            println!("Shell commands matching keyword `{}'", pattern);
            println!();
        }

        for entry in &matches {
            if flag_m {
                help_print_manpage(entry);
            } else if flag_d {
                println!("{} - {}", entry.name, entry.short_desc);
            } else if flag_s {
                println!("{}: {}", entry.name, entry.synopsis);
            } else {
                help_print_full(entry);
            }
        }
    }

    status
}

/// Check if a help entry name matches a pattern (glob-style with * and ?).
fn help_name_matches(name: &str, pattern: &str) -> bool {
    // Bash matches against the builtin name. Patterns can use * and ?.
    // Also match prefix: e.g. "rea" matches "read", "readarray", "readonly"
    if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
        glob_match(pattern, name)
    } else {
        // Exact match or prefix match
        name == pattern || name.starts_with(pattern)
    }
}

/// Simple glob matching for help patterns.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_impl(&pat, &txt, 0, 0)
}

fn glob_match_impl(pat: &[char], txt: &[char], mut pi: usize, mut ti: usize) -> bool {
    while pi < pat.len() {
        match pat[pi] {
            '*' => {
                // Skip consecutive *
                while pi < pat.len() && pat[pi] == '*' {
                    pi += 1;
                }
                if pi == pat.len() {
                    return true;
                }
                // Try matching * with 0, 1, 2, ... chars
                for start in ti..=txt.len() {
                    if glob_match_impl(pat, txt, pi, start) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= txt.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            '[' => {
                if ti >= txt.len() {
                    return false;
                }
                // Find closing ]
                let mut end = pi + 1;
                if end < pat.len() && pat[end] == '!' {
                    end += 1;
                }
                if end < pat.len() && pat[end] == ']' {
                    end += 1;
                }
                while end < pat.len() && pat[end] != ']' {
                    end += 1;
                }
                if end >= pat.len() {
                    // No closing ], treat [ as literal
                    if txt[ti] != '[' {
                        return false;
                    }
                    pi += 1;
                    ti += 1;
                    continue;
                }
                let negate = pi + 1 < pat.len() && pat[pi + 1] == '!';
                let start = if negate { pi + 2 } else { pi + 1 };
                let mut matched = false;
                let mut j = start;
                while j < end {
                    if j + 2 < end && pat[j + 1] == '-' {
                        if txt[ti] >= pat[j] && txt[ti] <= pat[j + 2] {
                            matched = true;
                        }
                        j += 3;
                    } else {
                        if txt[ti] == pat[j] {
                            matched = true;
                        }
                        j += 1;
                    }
                }
                if negate {
                    matched = !matched;
                }
                if !matched {
                    return false;
                }
                pi = end + 1;
                ti += 1;
            }
            c => {
                if ti >= txt.len() || txt[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == txt.len()
}

/// Print the two-column listing of all builtins (same as `help` with no args).
fn help_print_listing(shell: &Shell) {
    // Print version header
    println!("GNU bash, version 5.3");

    // Print intro text
    println!("These shell commands are defined internally.  Type `help' to see this list.");
    println!("Type `help name' to find out more about the function `name'.");
    println!("Use `info bash' to find out more about the shell in general.");
    println!("Use `man -k' or `info' to find out more about commands not in this list.");
    println!();
    println!("A star (*) next to a name means that the command is disabled.");
    println!();

    // Two-column layout matching bash's dispcolumn():
    // screenwidth = 80, width = screenwidth / 2 = 40
    // Left column: prefix(1) + synopsis(width-2=38) + '>' if truncated, padded to `width`
    // Right column: prefix(1) + synopsis(width-3=37) + '>' if truncated
    let width = 40;
    let total = HELP_ENTRIES.len();
    let rows = total.div_ceil(2);

    for row in 0..rows {
        let left_idx = row;
        let right_idx = row + rows;

        // Format left column
        let left = format_help_column(
            &HELP_ENTRIES[left_idx],
            &shell.disabled_builtins,
            width,
            false,
            shell,
        );

        if right_idx < total {
            // Format right column
            let right = format_help_column(
                &HELP_ENTRIES[right_idx],
                &shell.disabled_builtins,
                width,
                true,
                shell,
            );
            // Left column padded to `width` columns, then right column
            print!("{:<w$}", left, w = width);
            println!("{}", right);
        } else {
            println!("{}", left);
        }
    }
}

/// Format a help column entry matching bash's truncation logic.
///
/// In C locale, bash uses dispcolumn() with strncpy-based truncation:
///   Left:  strncpy(buf+1, doc, width-2); buf[width-2]='>'; buf[width-1]='\0';
///   Right: strncpy(buf+1, doc, width-3); buf[width-3]='>'; buf[width-2]='\0';
///   The '>' is visible when synopsis fills to within 1 char of the end.
///   Left threshold: len >= width-2-1 = 37.  Keep = width-2-1 = 37 chars + '>'.
///   Right threshold: len >= width-3-1 = 36.  Keep = width-3-1 = 36 chars + '>'.
///
/// In UTF-8 locale, bash uses wdispcolumn() with different right-column logic:
///   Right column uses wcstr[dispchars-1]='>' instead of wcstr[dispchars]='>'
///   which effectively shows one fewer character before '>'.
///   Left threshold: len >= 37.  Keep = min(len,38)-1 = 37 chars + '>'.
///   Right threshold: len >= 37.  Keep = min(len,38)-2 = 36 chars + '>'.
///   BUT slen is clamped to min(len, width-2=38) first, so for len >= 38,
///   right keep = 38-2 = 36; for len == 37, right keep = 37-2 = 35.
fn format_help_column(
    entry: &crate::builtins::help_data::HelpEntry,
    disabled_builtins: &std::collections::HashSet<String>,
    width: usize,
    is_right: bool,
    shell: &Shell,
) -> String {
    let disabled = disabled_builtins.contains(entry.name);
    let prefix = if disabled { '*' } else { ' ' };
    let synopsis = entry.synopsis;
    let synopsis_chars: Vec<char> = synopsis.chars().collect();
    let len = synopsis_chars.len();

    // Detect UTF-8 locale from shell variables (LC_ALL is special in bash —
    // setting it as a shell variable affects locale even without export)
    let is_utf8 = is_utf8_locale(shell);

    if is_utf8 {
        // wdispcolumn logic: clamp slen, then use dispchars/dispchars-1
        let max_slen = width - 2; // 38
        let effective_len = len.min(max_slen);
        // dispcols = effective_len + 1 (for prefix, ASCII assumption)
        let truncated = effective_len + 1 >= max_slen; // effective_len >= 37
        if truncated {
            let keep = if is_right {
                effective_len.saturating_sub(2)
            } else {
                effective_len.saturating_sub(1)
            };
            let truncated_text: String = synopsis_chars[..keep].iter().collect();
            format!("{}{}>", prefix, truncated_text)
        } else {
            format!("{}{}", prefix, synopsis)
        }
    } else {
        // dispcolumn (C locale) logic: strncpy-based truncation
        let max_copy = if is_right { width - 3 } else { width - 2 };
        let threshold = max_copy - 1;
        if len >= threshold {
            let keep = max_copy - 1;
            let truncated_text: String = synopsis_chars[..keep].iter().collect();
            format!("{}{}>", prefix, truncated_text)
        } else {
            format!("{}{}", prefix, synopsis)
        }
    }
}

/// Check if the current locale is UTF-8 (for help column truncation logic).
/// Checks shell variables first (bash treats LC_* as special), then falls back
/// to environment variables.
fn is_utf8_locale(shell: &Shell) -> bool {
    // Check LC_ALL, then LC_CTYPE, then LANG — first in shell vars, then env
    for var in &["LC_ALL", "LC_CTYPE", "LANG"] {
        // Check shell variable first (bash applies LC_* even without export)
        if let Some(val) = shell.vars.get(*var)
            && !val.is_empty()
        {
            let lower = val.to_lowercase();
            return lower.contains("utf-8") || lower.contains("utf8");
        }
        // Fall back to environment variable
        if let Ok(val) = std::env::var(var)
            && !val.is_empty()
        {
            let lower = val.to_lowercase();
            return lower.contains("utf-8") || lower.contains("utf8");
        }
    }
    false
}

/// Print full help for a single entry.
fn help_print_full(entry: &crate::builtins::help_data::HelpEntry) {
    println!("{}: {}", entry.name, entry.synopsis);
    for line in entry.long_help.lines() {
        println!("    {}", line);
    }
}

/// Print help in man-page format.
fn help_print_manpage(entry: &crate::builtins::help_data::HelpEntry) {
    println!("NAME");
    println!("    {} - {}", entry.name, entry.short_desc);
    println!();
    println!("SYNOPSIS");
    println!("    {}", entry.synopsis);
    println!();
    println!("DESCRIPTION");
    for line in entry.long_help.lines() {
        println!("    {}", line);
    }
    println!();
    println!("SEE ALSO");
    println!("    bash(1)");
    println!();
    println!("IMPLEMENTATION");
    println!("    GNU bash, version 5.3");
    println!("    Copyright (C) 2025 Free Software Foundation, Inc.");
    println!("    License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>");
    println!();
}

pub(super) fn builtin_type(shell: &mut Shell, args: &[String]) -> i32 {
    let builtin_map = builtins();
    let mut status = 0;
    let mut flag_t = false;
    let mut flag_p = false;
    let mut flag_a = false;
    let mut names = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-t" => flag_t = true,
            "-p" | "-P" => flag_p = true,
            "-a" => flag_a = true,
            "-f" => {}
            a if a.starts_with('-') && a.len() > 1 => {
                eprintln!("{}: type: {}: invalid option", shell.error_prefix(), a);
                eprintln!("type: usage: type [-afptP] name [name ...]");
                return 2;
            }
            _ => names.push(arg.as_str()),
        }
    }

    for name in names {
        let is_keyword = matches!(
            name,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "case"
                | "esac"
                | "for"
                | "select"
                | "while"
                | "until"
                | "do"
                | "done"
                | "in"
                | "function"
                | "time"
                | "{"
                | "}"
                | "!"
                | "[["
                | "]]"
                | "coproc"
        );
        if flag_t {
            // Print type word only
            if shell.aliases.contains_key(name) && shell.shopt_expand_aliases {
                println!("alias");
            } else if is_keyword {
                println!("keyword");
            } else if let Some((_, hits)) = shell.hash_table.get_mut(name) {
                *hits += 1;
                println!("file");
            } else if shell.opt_posix
                && matches!(
                    name,
                    "break"
                        | "."
                        | "source"
                        | "continue"
                        | "eval"
                        | "exec"
                        | "exit"
                        | "export"
                        | "readonly"
                        | "return"
                        | "set"
                        | "shift"
                        | "trap"
                        | "unset"
                        | ":"
                )
                && builtin_map.contains_key(name)
                && !shell.disabled_builtins.contains(name)
            {
                println!("builtin");
            } else if shell.functions.contains_key(name) {
                println!("function");
            } else if builtin_map.contains_key(name) && !shell.disabled_builtins.contains(name) {
                println!("builtin");
            } else if find_in_path_opt(name).is_some() {
                println!("file");
            }
            // If not found, print nothing and set status
            else {
                status = 1;
            }
        } else if flag_p {
            // Print path only for external commands — check hash table first
            if let Some((path, hits)) = shell.hash_table.get_mut(name) {
                *hits += 1;
                let path = path.clone();
                println!("{}", path);
            } else if let Some(path) = find_in_path_opt(name) {
                println!("{}", path);
            } else {
                status = 1;
            }
        } else {
            let mut found = false;
            // Helper to print function info
            let print_func_info = |name: &str, shell: &Shell| {
                if let Some(body) = shell.functions.get(name) {
                    println!("{} is a function", name);
                    let needs_keyword = shell.func_has_keyword.contains(name)
                        && !name.chars().all(|c| c.is_alphanumeric() || c == '_');
                    let prefix = if needs_keyword { "function " } else { "" };
                    let redirs = shell
                        .func_redirections
                        .get(name)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    let body_str = format_func_body_with_redirs(body, 0, redirs);
                    // Use raw byte output so PUA-encoded bytes (e.g. $'\001')
                    // are written as single raw bytes matching bash behavior.
                    let output = format!("{}{} () \n{}\n", prefix, name, body_str);
                    let bytes = string_to_raw_bytes(&output);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    #[cfg(unix)]
                    {
                        nix::unistd::write(std::io::stdout(), &bytes).ok();
                    }
                    #[cfg(not(unix))]
                    {
                        use std::io::Write;
                        std::io::stdout().write_all(&bytes).ok();
                    }
                    true
                } else {
                    false
                }
            };
            // Alias
            if shell.shopt_expand_aliases
                && let Some(alias_val) = shell.aliases.get(name)
            {
                println!("{} is aliased to `{}'", name, alias_val);
                found = true;
                if !flag_a {
                    continue;
                }
            }
            // Keyword
            if is_keyword {
                println!("{} is a shell keyword", name);
                found = true;
                if !flag_a {
                    continue;
                }
            }
            // In POSIX mode, special builtins before functions
            if shell.opt_posix
                && matches!(
                    name,
                    "break"
                        | "."
                        | "source"
                        | "continue"
                        | "eval"
                        | "exec"
                        | "exit"
                        | "export"
                        | "readonly"
                        | "return"
                        | "set"
                        | "shift"
                        | "trap"
                        | "unset"
                        | ":"
                )
                && builtin_map.contains_key(name)
                && !shell.disabled_builtins.contains(name)
            {
                println!("{} is a special shell builtin", name);
                found = true;
                if !flag_a {
                    continue;
                }
            }
            // Function
            if print_func_info(name, shell) {
                found = true;
                if !flag_a {
                    continue;
                }
            }
            // Builtin (skip if already shown as special builtin)
            if builtin_map.contains_key(name) && !shell.disabled_builtins.contains(name) {
                let already_shown_as_special = shell.opt_posix
                    && matches!(
                        name,
                        "break"
                            | "."
                            | "source"
                            | "continue"
                            | "eval"
                            | "exec"
                            | "exit"
                            | "export"
                            | "readonly"
                            | "return"
                            | "set"
                            | "shift"
                            | "trap"
                            | "unset"
                            | ":"
                    );
                if !already_shown_as_special {
                    println!("{} is a shell builtin", name);
                    found = true;
                    if !flag_a {
                        continue;
                    }
                }
            }
            // Check hash table first
            if let Some((hpath, hits)) = shell.hash_table.get_mut(name) {
                *hits += 1;
                let hpath = hpath.clone();
                println!("{} is hashed ({})", name, hpath);
                found = true;
                if !flag_a {
                    continue;
                }
            }
            // File
            if let Some(path) = find_in_path_opt(name) {
                if !found {
                    println!("{} is {}", name, path);
                }
                found = true;
                if !flag_a {
                    continue;
                }
            }
            if !found {
                eprintln!("{}: type: {}: not found", shell.error_prefix(), name);
                status = 1;
            }
        }
    }
    status
}

pub(super) fn builtin_builtin(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }
    // Check for invalid options
    if args[0].starts_with('-') && args[0] != "--" {
        eprintln!(
            "{}: builtin: {}: invalid option",
            shell.error_prefix(),
            args[0]
        );
        eprintln!("builtin: usage: builtin [shell-builtin [arg ...]]");
        return 2;
    }
    let builtin_map = builtins();
    let name = if args[0] == "--" { &args[1] } else { &args[0] };
    if let Some(func) = builtin_map.get(name.as_str()) {
        func(shell, &args[1..])
    } else {
        eprintln!(
            "{}: builtin: {}: not a shell builtin",
            shell.error_prefix(),
            name
        );
        1
    }
}

pub(super) fn builtin_command(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    let mut flag_v = false;
    let mut flag_big_v = false;
    let mut cmd_args = Vec::new();

    let mut parsing_opts = true;
    for arg in args {
        if parsing_opts {
            match arg.as_str() {
                "-v" => {
                    flag_v = true;
                    continue;
                }
                "-V" => {
                    flag_big_v = true;
                    continue;
                }
                "-p" => continue,
                "--" => {
                    parsing_opts = false;
                    continue;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("{}: command: {}: invalid option", shell.error_prefix(), s);
                    eprintln!("command: usage: command [-pVv] command [arg ...]");
                    return 2;
                }
                _ => parsing_opts = false,
            }
        }
        cmd_args.push(arg.clone());
    }

    if flag_v || flag_big_v {
        let builtin_map = builtins();
        for name in &cmd_args {
            if flag_big_v {
                // Verbose output (like type)
                let is_keyword = matches!(
                    name.as_str(),
                    "if" | "then"
                        | "else"
                        | "elif"
                        | "fi"
                        | "case"
                        | "esac"
                        | "for"
                        | "select"
                        | "while"
                        | "until"
                        | "do"
                        | "done"
                        | "in"
                        | "function"
                        | "time"
                        | "{"
                        | "}"
                        | "!"
                        | "[["
                        | "]]"
                        | "coproc"
                );
                if is_keyword {
                    println!("{} is a shell keyword", name);
                } else if shell.shopt_expand_aliases
                    && let Some(value) = shell.aliases.get(name.as_str())
                {
                    println!("{} is aliased to `{}'", name, value);
                } else if shell.opt_posix
                    && matches!(
                        name.as_str(),
                        "break"
                            | "."
                            | "source"
                            | "continue"
                            | "eval"
                            | "exec"
                            | "exit"
                            | "export"
                            | "readonly"
                            | "return"
                            | "set"
                            | "shift"
                            | "trap"
                            | "unset"
                            | ":"
                    )
                    && builtin_map.contains_key(name.as_str())
                {
                    println!("{} is a special shell builtin", name);
                } else if let Some(func_body) = shell.functions.get(name.as_str()) {
                    println!("{} is a function", name);
                    let prefix = if shell.func_has_keyword.contains(name.as_str()) {
                        "function "
                    } else {
                        ""
                    };
                    let redirs = shell
                        .func_redirections
                        .get(name.as_str())
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    let body = format_func_body_with_redirs(func_body, 0, redirs);
                    // Use raw byte output so PUA-encoded bytes (e.g. $'\001')
                    // are written as single raw bytes matching bash behavior.
                    let output = format!("{}{} () \n{}\n", prefix, name, body);
                    let bytes = string_to_raw_bytes(&output);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    #[cfg(unix)]
                    {
                        nix::unistd::write(std::io::stdout(), &bytes).ok();
                    }
                    #[cfg(not(unix))]
                    {
                        use std::io::Write;
                        std::io::stdout().write_all(&bytes).ok();
                    }
                } else if builtin_map.contains_key(name.as_str()) {
                    println!("{} is a shell builtin", name);
                } else if let Some((hpath, _)) = shell.hash_table.get(name.as_str()) {
                    println!("{} is hashed ({})", name, hpath);
                } else if let Some(path) = find_in_path_opt(name) {
                    println!("{} is {}", name, path);
                } else {
                    eprintln!("{}: command: {}: not found", shell.error_prefix(), name);
                    return 1;
                }
                continue;
            }
            // -v: just print name/path
            let is_keyword = matches!(
                name.as_str(),
                "if" | "then"
                    | "else"
                    | "elif"
                    | "fi"
                    | "case"
                    | "esac"
                    | "for"
                    | "select"
                    | "while"
                    | "until"
                    | "do"
                    | "done"
                    | "in"
                    | "function"
                    | "time"
                    | "{"
                    | "}"
                    | "!"
                    | "[["
                    | "]]"
                    | "coproc"
            );
            let is_posix_special = shell.opt_posix
                && matches!(
                    name.as_str(),
                    "break"
                        | "."
                        | "source"
                        | "continue"
                        | "eval"
                        | "exec"
                        | "exit"
                        | "export"
                        | "readonly"
                        | "return"
                        | "set"
                        | "shift"
                        | "trap"
                        | "unset"
                        | ":"
                )
                && builtin_map.contains_key(name.as_str());
            if is_keyword || is_posix_special || shell.functions.contains_key(name.as_str()) {
                println!("{}", name);
            } else if shell.shopt_expand_aliases && shell.aliases.contains_key(name.as_str()) {
                let val = &shell.aliases[name.as_str()];
                println!("alias {}='{}'", name, val);
            } else if builtin_map.contains_key(name.as_str()) {
                println!("{}", name);
            } else if let Some((hpath, _)) = shell.hash_table.get(name.as_str()) {
                println!("{}", hpath);
            } else if let Some(path) = find_in_path_opt(name) {
                println!("{}", path);
            } else {
                return 1;
            }
        }
        return 0;
    }

    // Execute command bypassing functions (but not builtins)
    if cmd_args.is_empty() {
        return 0;
    }

    let program = &cmd_args[0];
    let exec_args = &cmd_args[1..];

    // Check for builtins first (command bypasses functions, not builtins)
    let builtin_map = builtins();
    if let Some(builtin_fn) = builtin_map.get(program.as_str()) {
        let args_owned: Vec<String> = exec_args.iter().map(|s| s.to_string()).collect();
        return builtin_fn(shell, &args_owned);
    }

    // External command
    let exec_args_owned: Vec<String> = exec_args.iter().map(|s| s.to_string()).collect();
    match std::process::Command::new(program)
        .args(&exec_args_owned)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => {
                    if program.contains('/') {
                        "No such file or directory"
                    } else {
                        "command not found"
                    }
                }
                std::io::ErrorKind::PermissionDenied => "Permission denied",
                _ => "command not found",
            };
            eprintln!("{}: {}: {}", shell.error_prefix(), program, msg);
            127
        }
    }
}

pub(super) fn builtin_which(_shell: &mut Shell, args: &[String]) -> i32 {
    let mut status = 0;
    for arg in args {
        if let Some(path) = find_in_path_opt(arg) {
            println!("{}", path);
        } else {
            eprintln!("which: no {} in (PATH)", arg);
            status = 1;
        }
    }
    status
}

pub(super) fn builtin_hash(shell: &mut Shell, args: &[String]) -> i32 {
    // First pass: parse all flags and collect positional args.
    // Flags can be combined (e.g. `-lt` means `-l` + `-t`).
    let mut flag_r = false;
    let mut flag_l = false;
    let mut flag_t = false;
    let mut flag_d = false;
    let mut flag_p = false;
    let mut p_path: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Everything after `--` is positional
            i += 1;
            while i < args.len() {
                positional.push(args[i].clone());
                i += 1;
            }
            break;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            // Parse each character in the flag group
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'r' => flag_r = true,
                    'l' => flag_l = true,
                    't' => flag_t = true,
                    'd' => flag_d = true,
                    'p' => {
                        flag_p = true;
                    }
                    ch => {
                        eprintln!("{}: hash: -{}: invalid option", shell.error_prefix(), ch);
                        eprintln!("hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]");
                        return 2;
                    }
                }
                j += 1;
            }
        } else {
            // Not a flag — this and everything after are positional args
            while i < args.len() {
                positional.push(args[i].clone());
                i += 1;
            }
            break;
        }
        i += 1;
    }

    // If -p is set, consume the first two positional args as path and name
    if flag_p {
        // -p requires a pathname and a name
        if positional.len() < 2 {
            eprintln!(
                "{}: hash: -p: option requires an argument",
                shell.error_prefix()
            );
            return 1;
        }
        p_path = Some(positional.remove(0));
        // The rest of positional starts with the name(s)
    }

    // No args and no action flags: print the hash table
    if !flag_r && !flag_l && !flag_t && !flag_d && !flag_p && positional.is_empty() {
        if shell.hash_table.is_empty() {
            eprintln!("hash: hash table empty");
        } else {
            println!("hits\tcommand");
            // Iterate in 256-bucket order (FILENAME_HASH_BUCKETS) to match bash
            let mut entries: Vec<(usize, &str, &str, u32)> = shell
                .hash_table
                .iter()
                .map(|(name, (path, hits))| {
                    let bucket = AssocArray::hash_key(name) as usize & 255;
                    (bucket, name.as_str(), path.as_str(), *hits)
                })
                .collect();
            entries.sort_by_key(|(bucket, _, _, _)| *bucket);
            for (_, _, path, hits) in &entries {
                println!("   {}\t{}", hits, path);
            }
        }
        return 0;
    }

    let mut status = 0;

    // -r: clear hash table
    if flag_r {
        shell.hash_table.clear();
        shell.hash_order.clear();
        shell.bash_cmds_dirty = true;
    }

    // -p path name: set hash entry
    if flag_p && let Some(ref path) = p_path {
        if !shell.opt_hashall {
            eprintln!("{}: hash: hashing disabled", shell.error_prefix());
            return 1;
        }
        // Check if path is a directory
        // Consume the name argument regardless of whether the path is valid
        let name = if !positional.is_empty() {
            Some(positional.remove(0))
        } else {
            None
        };
        let p = std::path::Path::new(path.as_str());
        if p.is_dir() {
            eprintln!("{}: hash: {}: Is a directory", shell.error_prefix(), path);
            status = 1;
        } else if let Some(name) = name {
            if !shell.hash_table.contains_key(&name) {
                shell.hash_order.push(name.clone());
            }
            shell.hash_table.insert(name.clone(), (path.clone(), 0));
            shell.bash_cmds_dirty = true;
        }
    }

    // -d: delete entries
    if flag_d {
        if positional.is_empty() {
            eprintln!(
                "{}: hash: -d: option requires an argument",
                shell.error_prefix()
            );
            return 1;
        }
        for name in &positional {
            if shell.hash_table.remove(name).is_some() {
                shell.hash_order.retain(|n| n != name);
                shell.bash_cmds_dirty = true;
            } else {
                eprintln!("{}: hash: {}: not found", shell.error_prefix(), name);
                status = 1;
            }
        }
        return status;
    }

    // -t: print hash paths for names
    if flag_t {
        for name in &positional {
            if let Some((path, _)) = shell.hash_table.get(name) {
                if flag_l {
                    // Long format: builtin hash -p <path> <name>
                    println!("builtin hash -p {} {}", path, name);
                } else {
                    println!("{}", path);
                }
            } else {
                eprintln!("{}: hash: {}: not found", shell.error_prefix(), name);
                status = 1;
            }
        }
        return status;
    }

    // -l alone (without -t): print all entries in long format
    if flag_l && !flag_t && positional.is_empty() {
        // Iterate in 256-bucket order (FILENAME_HASH_BUCKETS) to match bash
        let mut entries: Vec<(usize, String, String)> = shell
            .hash_table
            .iter()
            .map(|(name, (path, _))| {
                let bucket = AssocArray::hash_key(name) as usize & 255;
                (bucket, name.clone(), path.clone())
            })
            .collect();
        entries.sort_by_key(|(bucket, _, _)| *bucket);
        for (_, name, path) in &entries {
            println!("builtin hash -p {} {}", path, name);
        }
        return status;
    }

    // Bare names: look up command and add to hash table
    for name in &positional {
        if let Some(path) = find_command_path(name) {
            if !shell.hash_table.contains_key(name) {
                shell.hash_order.push(name.to_string());
            }
            shell.hash_table.insert(name.to_string(), (path.clone(), 0));
            shell.bash_cmds_dirty = true;
        } else {
            eprintln!("{}: hash: {}: not found", shell.error_prefix(), name);
            status = 1;
        }
    }

    status
}
