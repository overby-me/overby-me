use crate::ast::{
    AndOr, AssignValue, CaseTerminator, Command, CompoundCommand, CondExpr, ParamOp, Pipeline,
    ProcessSubKind, Program, RedirFd, RedirectKind, Redirection, SimpleCommand, Word, WordPart,
};
use crate::interpreter::{Shell, capitalize_string, is_valid_identifier};
use std::collections::HashMap;

pub type BuiltinFn = fn(&mut Shell, &[String]) -> i32;

/// Check if a parsed program contains any incomplete funsub markers
fn program_has_incomplete_funsub(program: &Program) -> bool {
    fn word_check(word: &Word) -> bool {
        crate::ast::word_has_incomplete_funsub(word)
    }
    fn cmd_check(cmd: &Command) -> bool {
        match cmd {
            Command::Simple(sc) => {
                sc.words.iter().any(word_check)
                    || sc.redirections.iter().any(|r| word_check(&r.target))
                    || sc.assignments.iter().any(|a| match &a.value {
                        AssignValue::Scalar(w) => word_check(w),
                        AssignValue::Array(items) => {
                            items.iter().any(|elem| word_check(&elem.value))
                        }
                        AssignValue::None => false,
                    })
            }
            Command::Compound(cc, redirs) => {
                redirs.iter().any(|r| word_check(&r.target)) || compound_check(cc)
            }
            Command::FunctionDef(_, body, _, _) => compound_check(body),
            Command::Coproc(_, inner) => cmd_check(inner),
        }
    }
    fn compound_check(cc: &CompoundCommand) -> bool {
        match cc {
            CompoundCommand::BraceGroup(cmds) | CompoundCommand::Subshell(cmds) => {
                cmds.iter().any(|cc| andor_check(&cc.list))
            }
            _ => false,
        }
    }
    fn andor_check(ao: &crate::ast::AndOrList) -> bool {
        pipeline_check(&ao.first) || ao.rest.iter().any(|(_, p)| pipeline_check(p))
    }
    fn pipeline_check(p: &Pipeline) -> bool {
        p.commands.iter().any(cmd_check)
    }
    program.iter().any(|cc| andor_check(&cc.list))
}

/// Fix Rust's scientific notation (e.g. "4.2e0") to C/bash format ("4.2e+00")
fn fix_scientific_notation(s: &str, uppercase: bool) -> String {
    let marker = if uppercase { 'E' } else { 'e' };
    if let Some(pos) = s.rfind(marker) {
        let (mantissa, exp_part) = s.split_at(pos);
        let exp_str = &exp_part[1..]; // skip 'e'/'E'
        let exp_val: i32 = exp_str.parse().unwrap_or(0);
        format!("{}{}{:+03}", mantissa, marker, exp_val)
    } else {
        s.to_string()
    }
}

fn list_all_signals() -> Vec<(i32, &'static str)> {
    vec![
        (1, "SIGHUP"),
        (2, "SIGINT"),
        (3, "SIGQUIT"),
        (4, "SIGILL"),
        (5, "SIGTRAP"),
        (6, "SIGABRT"),
        (7, "SIGBUS"),
        (8, "SIGFPE"),
        (9, "SIGKILL"),
        (10, "SIGUSR1"),
        (11, "SIGSEGV"),
        (12, "SIGUSR2"),
        (13, "SIGPIPE"),
        (14, "SIGALRM"),
        (15, "SIGTERM"),
        (16, "SIGSTKFLT"),
        (17, "SIGCHLD"),
        (18, "SIGCONT"),
        (19, "SIGSTOP"),
        (20, "SIGTSTP"),
        (21, "SIGTTIN"),
        (22, "SIGTTOU"),
        (23, "SIGURG"),
        (24, "SIGXCPU"),
        (25, "SIGXFSZ"),
        (26, "SIGVTALRM"),
        (27, "SIGPROF"),
        (28, "SIGWINCH"),
        (29, "SIGIO"),
        (30, "SIGPWR"),
        (31, "SIGSYS"),
        (34, "SIGRTMIN"),
        (35, "SIGRTMIN+1"),
        (36, "SIGRTMIN+2"),
        (37, "SIGRTMIN+3"),
        (38, "SIGRTMIN+4"),
        (39, "SIGRTMIN+5"),
        (40, "SIGRTMIN+6"),
        (41, "SIGRTMIN+7"),
        (42, "SIGRTMIN+8"),
        (43, "SIGRTMIN+9"),
        (44, "SIGRTMIN+10"),
        (45, "SIGRTMIN+11"),
        (46, "SIGRTMIN+12"),
        (47, "SIGRTMIN+13"),
        (48, "SIGRTMIN+14"),
        (49, "SIGRTMIN+15"),
        (50, "SIGRTMAX-14"),
        (51, "SIGRTMAX-13"),
        (52, "SIGRTMAX-12"),
        (53, "SIGRTMAX-11"),
        (54, "SIGRTMAX-10"),
        (55, "SIGRTMAX-9"),
        (56, "SIGRTMAX-8"),
        (57, "SIGRTMAX-7"),
        (58, "SIGRTMAX-6"),
        (59, "SIGRTMAX-5"),
        (60, "SIGRTMAX-4"),
        (61, "SIGRTMAX-3"),
        (62, "SIGRTMAX-2"),
        (63, "SIGRTMAX-1"),
        (64, "SIGRTMAX"),
    ]
}

pub fn builtins() -> HashMap<&'static str, BuiltinFn> {
    let mut map: HashMap<&'static str, BuiltinFn> = HashMap::new();
    map.insert("echo", builtin_echo);
    map.insert("printf", builtin_printf);
    map.insert("cd", builtin_cd);
    map.insert("pwd", builtin_pwd);
    map.insert("export", builtin_export);
    map.insert("unset", builtin_unset);
    map.insert("readonly", builtin_readonly);
    map.insert("local", builtin_local);
    map.insert("declare", builtin_declare);
    map.insert("typeset", builtin_declare);
    map.insert("set", builtin_set);
    map.insert("shift", builtin_shift);
    map.insert("exit", builtin_exit);
    map.insert("return", builtin_return);
    map.insert("true", builtin_true);
    map.insert("false", builtin_false);
    map.insert(":", builtin_true);
    map.insert("test", builtin_test);
    map.insert("[", builtin_test_bracket);
    map.insert("read", builtin_read);
    map.insert("eval", builtin_eval);
    map.insert("exec", builtin_exec);
    map.insert("source", builtin_source);
    map.insert(".", builtin_source);
    map.insert("type", builtin_type);
    map.insert("builtin", builtin_builtin);
    map.insert("command", builtin_command);
    map.insert("which", builtin_which);
    map.insert("hash", builtin_hash);
    map.insert("trap", builtin_trap);
    map.insert("wait", builtin_wait);
    map.insert("kill", builtin_kill);
    map.insert("umask", builtin_umask);
    map.insert("getopts", builtin_getopts);
    map.insert("let", builtin_let);
    map.insert("mapfile", builtin_mapfile);
    map.insert("readarray", builtin_mapfile);
    map.insert("alias", builtin_alias);
    map.insert("unalias", builtin_unalias);
    map.insert("enable", builtin_enable);
    map.insert("shopt", builtin_shopt);
    map.insert("dirs", builtin_dirs);
    map.insert("pushd", builtin_pushd);
    map.insert("popd", builtin_popd);
    map.insert("complete", builtin_complete);
    map.insert("compgen", builtin_compgen);
    map.insert("times", builtin_times);
    map.insert("break", builtin_break);
    map.insert("continue", builtin_continue);
    map.insert("ulimit", builtin_ulimit);
    map.insert("caller", builtin_caller);
    map.insert("jobs", builtin_jobs);
    map.insert("disown", builtin_disown);
    map.insert("fg", builtin_fg);
    map.insert("bg", builtin_bg);
    map.insert("suspend", builtin_suspend);
    map.insert("logout", builtin_logout);
    map
}

fn builtin_break(shell: &mut Shell, args: &[String]) -> i32 {
    if shell.loop_depth == 0 {
        eprintln!(
            "{}: break: only meaningful in a `for', `while', or `until' loop",
            shell.error_prefix()
        );
        return 0;
    }
    let n: i32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
    if n <= 0 {
        eprintln!(
            "{}: break: {}: loop count out of range",
            shell.error_prefix(),
            n
        );
        // bash still breaks after the error
        shell.breaking = 1;
        return 1;
    }
    shell.breaking = n;
    0
}

fn builtin_continue(shell: &mut Shell, args: &[String]) -> i32 {
    if shell.loop_depth == 0 {
        eprintln!(
            "{}: continue: only meaningful in a `for', `while', or `until' loop",
            shell.error_prefix()
        );
        return 0;
    }
    let n: i32 = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);
    if n <= 0 {
        eprintln!(
            "{}: continue: {}: loop count out of range",
            shell.error_prefix(),
            n
        );
        // bash breaks the loop after the error (not continue)
        shell.breaking = 1;
        return 1;
    }
    shell.continuing = n;
    0
}

fn builtin_echo(shell: &mut Shell, args: &[String]) -> i32 {
    let mut newline = true;
    let mut interpret_escapes = false;
    let mut start = 0;

    for (i, arg) in args.iter().enumerate() {
        match arg.as_str() {
            "-n" => {
                newline = false;
                start = i + 1;
            }
            "-e" => {
                interpret_escapes = true;
                start = i + 1;
            }
            "-E" => {
                interpret_escapes = false;
                start = i + 1;
            }
            "-ne" | "-en" => {
                newline = false;
                interpret_escapes = true;
                start = i + 1;
            }
            "-nE" | "-En" => {
                newline = false;
                interpret_escapes = false;
                start = i + 1;
            }
            _ => break,
        }
    }

    let text = args[start..].join(" ");
    let output = if interpret_escapes {
        interpret_echo_escapes(&text)
    } else {
        text
    };

    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // Convert to bytes: chars in U+0080..U+00FF range are written as single
    // bytes (raw byte output like bash), not as multi-byte UTF-8
    let bytes = string_to_raw_bytes(&output);
    let result = if newline {
        out.write_all(&bytes)
            .and_then(|_| out.write_all(b"\n"))
            .and_then(|_| out.flush())
    } else {
        out.write_all(&bytes).and_then(|_| out.flush())
    };
    drop(out);
    match result {
        Ok(()) => 0,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            // In pipeline children, broken pipe is handled by SIGPIPE (SIG_DFL).
            // In lastpipe (current shell), report the error like bash does.
            if !shell.in_pipeline_child {
                eprintln!("{}: echo: write error: Broken pipe", shell.error_prefix());
            }
            1
        }
        Err(e) => {
            let msg = Shell::io_error_message(&e);
            eprintln!("{}: echo: write error: {}", shell.error_prefix(), msg);
            1
        }
    }
}

/// Convert a string to raw bytes. Characters in U+0000..U+007F are written as
/// single ASCII bytes. Characters in U+0080..U+00FF are written as single bytes
/// (Latin-1/raw byte output, matching bash's behavior for $'\xNN'). Characters
/// above U+00FF are written as their UTF-8 encoding.
pub fn string_to_raw_bytes(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        if cp <= 0xFF {
            bytes.push(cp as u8);
        } else {
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            bytes.extend_from_slice(encoded.as_bytes());
        }
    }
    bytes
}

fn interpret_echo_escapes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('r') => result.push('\r'),
                Some('\\') => result.push('\\'),
                Some('a') => result.push('\x07'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0c'),
                Some('v') => result.push('\x0b'),
                Some('e') | Some('E') => result.push('\x1b'),
                Some('c') => break, // Stop output
                Some(first @ '0'..='7') => {
                    // \0NNN or \NNN — octal escape
                    let mut val = first as u8 - b'0';
                    let max_extra = if first == '0' { 3 } else { 2 };
                    for _ in 0..max_extra {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c @ '0'..='7') => {
                                val = val * 8 + (c as u8 - b'0');
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    result.push(val as char);
                }
                Some('x') => {
                    let mut val = 0u8;
                    let mut count = 0;
                    for _ in 0..2 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap() as u8;
                                chars = peek;
                                count += 1;
                            }
                            _ => break,
                        }
                    }
                    if count == 0 {
                        // No hex digits: output literal \x
                        result.push('\\');
                        result.push('x');
                    } else {
                        result.push(val as char);
                    }
                }
                Some('u') => {
                    let mut val = 0u32;
                    for _ in 0..4 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap();
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
                Some('U') => {
                    let mut val = 0u32;
                    for _ in 0..8 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap();
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
                Some(c) => {
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Parse a printf numeric argument, handling 'char prefix, 0x hex, 0 octal
fn parse_printf_int(arg: &str) -> i64 {
    if arg.starts_with("0x") || arg.starts_with("0X") {
        i64::from_str_radix(&arg[2..], 16).unwrap_or(0)
    } else if arg.starts_with("0") && arg.len() > 1 && !arg.contains(['8', '9']) {
        i64::from_str_radix(&arg[1..], 8).unwrap_or(0)
    } else if arg.starts_with('\'') || arg.starts_with('"') {
        arg.chars().nth(1).map(|c| c as i64).unwrap_or(0)
    } else {
        arg.parse().unwrap_or(0)
    }
}

fn builtin_printf(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("printf: usage: printf [-v var] format [arguments]");
        return 1;
    }

    // Handle options
    if args[0].starts_with('-') && args[0] != "-v" && args[0] != "--" {
        eprintln!("printf: usage: printf [-v var] format [arguments]");
        eprintln!(
            "{}: printf: {}: invalid option",
            shell.error_prefix(),
            args[0]
        );
        return 2;
    }

    // Handle -v varname option
    if args.len() >= 3 && args[0] == "-v" {
        let var_name = args[1].clone();
        // Validate variable name
        if !var_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !var_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            eprintln!(
                "{}: printf: `{}': not a valid identifier",
                shell.error_prefix(),
                var_name
            );
            return 2;
        }
        // Build the printf command without -v and capture output
        let inner_args: Vec<String> = args[2..].to_vec();
        let output = shell.capture_output(&format!(
            "printf {}",
            inner_args
                .iter()
                .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        shell.set_var(&var_name, output);
        return 0;
    }
    // Skip -- (end of options marker)
    let args = if !args.is_empty() && args[0] == "--" {
        &args[1..]
    } else {
        args
    };
    // After processing options, format is required
    if args.is_empty() {
        eprintln!("printf: usage: printf [-v var] format [arguments]");
        return 2;
    }
    if args.is_empty() {
        return 0;
    }
    let format = &args[0];
    let fmt_args = &args[1..];
    let mut arg_idx = 0;

    // printf reuses format string until all arguments are consumed
    loop {
        let mut chars = format.chars().peekable();
        let start_arg_idx = arg_idx;
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => println!(),
                    Some('t') => print!("\t"),
                    Some('r') => print!("\r"),
                    Some('\\') => print!("\\"),
                    Some('a') => print!("\x07"),
                    Some('b') => print!("\x08"),
                    Some('f') => print!("\x0c"),
                    Some('v') => print!("\x0b"),
                    Some(c @ '0'..='7') => {
                        let mut val = c as u8 - b'0';
                        for _ in 0..2 {
                            match chars.peek() {
                                Some(d @ '0'..='7') => {
                                    val = val * 8 + (*d as u8 - b'0');
                                    chars.next();
                                }
                                _ => break,
                            }
                        }
                        // Write raw byte (including NUL)
                        use std::io::Write;
                        std::io::stdout().write_all(&[val]).ok();
                    }
                    Some('\'') => print!("'"),
                    Some('"') => print!("\""),
                    Some('x') => {
                        // \xNN hex escape
                        let mut val = 0u8;
                        for _ in 0..2 {
                            match chars.peek() {
                                Some(d) if d.is_ascii_hexdigit() => {
                                    val = val * 16 + d.to_digit(16).unwrap() as u8;
                                    chars.next();
                                }
                                _ => break,
                            }
                        }
                        use std::io::Write;
                        std::io::stdout().write_all(&[val]).ok();
                    }
                    Some(c) => print!("\\{}", c),
                    None => print!("\\"),
                }
            } else if ch == '%' {
                // Parse optional flags, width, precision
                let mut flags = String::new();
                let mut width_str = String::new();
                while let Some(&c) = chars.peek() {
                    if matches!(c, '-' | '+' | ' ' | '0' | '#') {
                        flags.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&'*') {
                    // Width from argument
                    chars.next();
                    let w_arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                    width_str = w_arg.to_string();
                    arg_idx += 1;
                } else {
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() {
                            width_str.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                // Parse precision
                let mut precision: Option<usize> = None;
                if chars.peek() == Some(&'.') {
                    chars.next();
                    if chars.peek() == Some(&'*') {
                        // Precision from argument
                        chars.next();
                        let p_arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        precision = Some(p_arg.parse().unwrap_or(0));
                        arg_idx += 1;
                    } else {
                        let mut prec_str = String::new();
                        while let Some(&c) = chars.peek() {
                            if c.is_ascii_digit() {
                                prec_str.push(c);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        precision = Some(prec_str.parse().unwrap_or(0));
                    }
                }
                // Handle negative width (means left-align)
                let (w, left) = if let Some(stripped) = width_str.strip_prefix('-') {
                    let abs_w: usize = stripped.parse().unwrap_or(0);
                    (abs_w, true)
                } else {
                    (width_str.parse().unwrap_or(0), flags.contains('-'))
                };
                let zero_pad = flags.contains('0');
                match chars.next() {
                    Some('(') => {
                        // %(fmt)T — strftime format
                        let mut fmt = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == ')' {
                                chars.next();
                                break;
                            }
                            fmt.push(c);
                            chars.next();
                        }
                        // Consume the T
                        if chars.peek() == Some(&'T') {
                            chars.next();
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("-1");
                        let timestamp: i64 = if arg == "-1" {
                            // -1 means current time
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64
                        } else if arg == "-2" {
                            // -2 means shell startup time
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs() as i64
                        } else {
                            arg.parse().unwrap_or(0)
                        };

                        // Use libc strftime for formatting
                        #[cfg(unix)]
                        {
                            let tm = unsafe {
                                let t = timestamp as libc::time_t;
                                let mut tm: libc::tm = std::mem::zeroed();
                                libc::localtime_r(&t, &mut tm);
                                tm
                            };
                            let c_fmt = std::ffi::CString::new(fmt.as_str()).unwrap_or_default();
                            let mut buf = [0u8; 512];
                            let len = unsafe {
                                libc::strftime(
                                    buf.as_mut_ptr() as *mut libc::c_char,
                                    buf.len(),
                                    c_fmt.as_ptr(),
                                    &tm,
                                )
                            };
                            if len > 0 {
                                print!("{}", String::from_utf8_lossy(&buf[..len]));
                            }
                        }
                        arg_idx += 1;
                    }
                    Some('s') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        // Apply precision (truncate string)
                        let truncated = if let Some(p) = precision {
                            &arg[..arg.len().min(p)]
                        } else {
                            arg
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", truncated);
                            } else {
                                print!("{:>w$}", truncated);
                            }
                        } else {
                            print!("{}", truncated);
                        }
                        arg_idx += 1;
                    }
                    Some('d') | Some('i') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                            i64::from_str_radix(&arg[2..], 16).unwrap_or(0)
                        } else if arg.starts_with("0") && arg.len() > 1 && !arg.contains(['8', '9'])
                        {
                            i64::from_str_radix(&arg[1..], 8).unwrap_or(0)
                        } else if arg.starts_with('\'') || arg.starts_with('"') {
                            arg.chars().nth(1).map(|c| c as i64).unwrap_or(0)
                        } else {
                            arg.parse().unwrap_or(0)
                        };
                        let show_sign = flags.contains('+');
                        let space_sign = flags.contains(' ');
                        let sign_prefix = if n >= 0 && show_sign {
                            "+"
                        } else if n >= 0 && space_sign {
                            " "
                        } else {
                            ""
                        };
                        let effective_width = if let Some(p) = precision { p.max(w) } else { w };
                        let use_zero_pad = zero_pad || precision.is_some();
                        if effective_width > 0 {
                            if left {
                                let formatted = if n < 0 {
                                    format!("{}", n)
                                } else {
                                    format!("{}{}", sign_prefix, n)
                                };
                                print!("{:<effective_width$}", formatted);
                            } else if use_zero_pad {
                                // For zero-padding, sign/prefix comes first, then zeros, then digits
                                let prefix = if n < 0 { "-" } else { sign_prefix };
                                let abs_n = n.unsigned_abs();
                                let num_width = effective_width.saturating_sub(prefix.len());
                                print!("{}{:0>num_width$}", prefix, abs_n);
                            } else {
                                let formatted = if n < 0 {
                                    format!("{}", n)
                                } else {
                                    format!("{}{}", sign_prefix, n)
                                };
                                print!("{:>effective_width$}", formatted);
                            }
                        } else {
                            let formatted = if n < 0 {
                                format!("{}", n)
                            } else {
                                format!("{}{}", sign_prefix, n)
                            };
                            print!("{}", formatted);
                        }
                        arg_idx += 1;
                    }
                    Some(hex_ch @ ('x' | 'X')) => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = parse_printf_int(arg);
                        let formatted = if hex_ch == 'x' {
                            if flags.contains('#') {
                                format!("{:#x}", n)
                            } else {
                                format!("{:x}", n)
                            }
                        } else if flags.contains('#') {
                            format!("{:#X}", n)
                        } else {
                            format!("{:X}", n)
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", formatted);
                            } else if zero_pad {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                        } else {
                            print!("{}", formatted);
                        }
                        arg_idx += 1;
                    }
                    Some('o') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = parse_printf_int(arg);
                        let formatted = if flags.contains('#') {
                            format!("0{:o}", n) // C-style 0 prefix, not Rust's 0o
                        } else {
                            format!("{:o}", n)
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", formatted);
                            } else if zero_pad {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                        } else {
                            print!("{}", formatted);
                        }
                        arg_idx += 1;
                    }
                    Some('u') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: u64 = parse_printf_int(arg) as u64;
                        let formatted = format!("{}", n);
                        if w > 0 {
                            if left {
                                print!("{:<w$}", formatted);
                            } else if zero_pad {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                        } else {
                            print!("{}", formatted);
                        }
                        arg_idx += 1;
                    }
                    Some(fmt_ch @ ('f' | 'F' | 'e' | 'E' | 'g' | 'G')) => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: f64 = arg.parse().unwrap_or(0.0);
                        let p = precision.unwrap_or(6);
                        let formatted = match fmt_ch {
                            'e' => {
                                let s = format!("{:.p$e}", n);
                                // Rust uses e0, bash/C uses e+00
                                fix_scientific_notation(&s, false)
                            }
                            'E' => {
                                let s = format!("{:.p$E}", n);
                                fix_scientific_notation(&s, true)
                            }
                            'g' | 'G' => {
                                // %g uses shorter of %e and %f, stripping trailing zeros
                                let p = if p == 0 { 1 } else { p };
                                let f_str = format!("{:.prec$}", n, prec = p.saturating_sub(1));
                                let e_str = if fmt_ch == 'G' {
                                    fix_scientific_notation(
                                        &format!("{:.prec$E}", n, prec = p.saturating_sub(1)),
                                        true,
                                    )
                                } else {
                                    fix_scientific_notation(
                                        &format!("{:.prec$e}", n, prec = p.saturating_sub(1)),
                                        false,
                                    )
                                };
                                if e_str.len() < f_str.len() {
                                    e_str
                                } else {
                                    // Strip trailing zeros after decimal point
                                    if f_str.contains('.') {
                                        let trimmed = f_str.trim_end_matches('0');
                                        let trimmed = trimmed.trim_end_matches('.');
                                        trimmed.to_string()
                                    } else {
                                        f_str
                                    }
                                }
                            }
                            _ => format!("{:.p$}", n), // f, F
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                        } else {
                            print!("{}", formatted);
                        }
                        arg_idx += 1;
                    }
                    Some('c') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        if let Some(ch) = arg.chars().next() {
                            print!("{}", ch);
                        }
                        arg_idx += 1;
                    }
                    Some('b') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        let expanded = interpret_echo_escapes(arg);
                        // Use raw byte output for %b (supports NUL bytes and raw bytes)
                        use std::io::Write;
                        std::io::stdout()
                            .write_all(&string_to_raw_bytes(&expanded))
                            .ok();
                        arg_idx += 1;
                    }
                    Some('q') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        let mut quoted = if arg.is_empty() {
                            "''".to_string()
                        } else {
                            shell_escape(arg)
                        };
                        // %q precision truncates the QUOTED form
                        if let Some(p) = precision {
                            let truncated: String = quoted.chars().take(p).collect();
                            quoted = truncated;
                        }
                        if w > 0 {
                            if left {
                                print!("{:<w$}", quoted);
                            } else {
                                print!("{:>w$}", quoted);
                            }
                        } else {
                            print!("{}", quoted);
                        }
                        arg_idx += 1;
                    }
                    Some('Q') => {
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        // %Q: precision truncates the value BEFORE quoting
                        let truncated = if let Some(p) = precision {
                            &arg[..arg.len().min(p)]
                        } else {
                            arg
                        };
                        let quoted = if truncated.is_empty() {
                            "''".to_string()
                        } else {
                            shell_escape(truncated)
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", quoted);
                            } else {
                                print!("{:>w$}", quoted);
                            }
                        } else {
                            print!("{}", quoted);
                        }
                        arg_idx += 1;
                    }
                    Some('n') => {
                        // %n: store number of chars written so far
                        let var_name = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        if !var_name.is_empty()
                            && var_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                            && var_name.chars().next().is_some_and(|c| !c.is_ascii_digit())
                        {
                            // We don't track exact chars written, use 0 as approximation
                            shell.set_var(var_name, "0".to_string());
                        } else if !var_name.is_empty() {
                            eprintln!(
                                "{}: printf: `{}': not a valid identifier",
                                shell.error_prefix(),
                                var_name
                            );
                        }
                        arg_idx += 1;
                    }
                    Some('%') => print!("%"),
                    Some(c) => print!("%{}{}{}", flags, width_str, c),
                    None => print!("%"),
                }
            } else {
                print!("{}", ch);
            }
        }
        // If no format args were consumed in this pass, or all args consumed, stop
        if arg_idx == start_arg_idx || arg_idx >= fmt_args.len() {
            break;
        }
    } // end loop
    // Flush stdout to ensure output goes to the correct fd
    // (redirections may change fd 1 before the buffer is flushed)
    use std::io::Write;
    std::io::stdout().flush().ok();
    0
}

/// Shell-escape a string for use with %q in printf.
/// Convert a Rust io::Error to a bash-style error message
/// Quote a string for declare -p output, using $'...' for control chars
fn quote_for_declare(s: &str) -> String {
    let needs_dollar_quote =
        s.bytes().any(|b| b < 0x20 || b == 0x7f || b > 0x7f) || s.contains('\'');
    if needs_dollar_quote {
        let mut out = String::from("$'");
        for b in s.bytes() {
            match b {
                b'\n' => out.push_str("\\n"),
                b'\t' => out.push_str("\\t"),
                b'\r' => out.push_str("\\r"),
                0x07 => out.push_str("\\a"),
                0x08 => out.push_str("\\b"),
                0x1b => out.push_str("\\E"),
                b'\'' => out.push_str("\\'"),
                b'\\' => out.push_str("\\\\"),
                b if b < 0x20 || b == 0x7f => {
                    // Use octal format like bash
                    out.push_str(&format!("\\{:03o}", b));
                }
                b if b > 0x7f => {
                    // Non-ASCII byte: output as octal
                    out.push_str(&format!("\\{:03o}", b));
                }
                b => out.push(b as char),
            }
        }
        out.push('\'');
        out
    } else {
        format!("\"{}\"", s)
    }
}

fn io_error_message(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory",
        std::io::ErrorKind::PermissionDenied => "Permission denied",
        std::io::ErrorKind::AlreadyExists => "File exists",
        std::io::ErrorKind::BrokenPipe => "Broken pipe",
        std::io::ErrorKind::InvalidInput => "Invalid argument",
        _ => "Input/output error",
    }
}

fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Check if the string needs quoting
    let needs_quoting = s
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '/' && c != '.' && c != '-');
    if !needs_quoting {
        return s.to_string();
    }
    // Check if we can use simple backslash quoting (no control/non-ASCII chars)
    let has_control = s
        .chars()
        .any(|c| c.is_ascii_control() || (c as u32) >= 0x80);
    if !has_control {
        let mut result = String::new();
        for ch in s.chars() {
            if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '/' && ch != '.' && ch != '-' {
                result.push('\\');
            }
            result.push(ch);
        }
        return result;
    }
    // Use $'...' quoting for strings with control characters
    let mut result = String::from("$'");
    for ch in s.chars() {
        match ch {
            '\'' => result.push_str("\\'"),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\t' => result.push_str("\\t"),
            '\r' => result.push_str("\\r"),
            '\x07' => result.push_str("\\a"),
            '\x08' => result.push_str("\\b"),
            '\x0c' => result.push_str("\\f"),
            '\x0b' => result.push_str("\\v"),
            '\x1b' => result.push_str("\\E"),
            c if c.is_ascii_graphic() || c == ' ' => result.push(c),
            c => {
                // Use octal format: for Latin-1 range (U+0080..U+00FF),
                // output as single byte; for others, use UTF-8 bytes
                let cp = c as u32;
                if cp <= 0xFF {
                    result.push_str(&format!("\\{:03o}", cp));
                } else {
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    for b in encoded.as_bytes() {
                        result.push_str(&format!("\\{:03o}", b));
                    }
                }
            }
        }
    }
    result.push('\'');
    result
}

fn builtin_cd(shell: &mut Shell, args: &[String]) -> i32 {
    // Skip -P and -L flags
    let args: Vec<&String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-P" | "-L" | "-e"))
        .collect();
    if args.len() > 1 {
        eprintln!("{}: cd: too many arguments", shell.error_prefix());
        return 1;
    }
    let target = if args.is_empty() {
        match shell
            .vars
            .get("HOME")
            .cloned()
            .or_else(|| std::env::var("HOME").ok())
        {
            Some(h) if !h.is_empty() => h,
            _ => {
                eprintln!("{}: cd: HOME not set", shell.error_prefix());
                return 1;
            }
        }
    } else if args[0].as_str() == "-" {
        shell
            .vars
            .get("OLDPWD")
            .cloned()
            .or_else(|| std::env::var("OLDPWD").ok())
            .unwrap_or_else(|| ".".to_string())
    } else {
        (*args[0]).clone()
    };

    let old = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    match std::env::set_current_dir(&target) {
        Ok(()) => {
            let saved_status = shell.last_status;
            shell.set_var("OLDPWD", old);
            let new = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            shell.set_var("PWD", new.clone());
            unsafe { std::env::set_var("PWD", &new) };
            unsafe {
                std::env::set_var(
                    "OLDPWD",
                    shell.vars.get("OLDPWD").cloned().unwrap_or_default(),
                )
            };
            if !args.is_empty() && args[0] == "-" {
                println!("{}", new);
            }
            // If set_var failed (readonly), return 1
            if shell.last_status != saved_status {
                shell.last_status
            } else {
                0
            }
        }
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::NotFound => "No such file or directory",
                std::io::ErrorKind::PermissionDenied => "Permission denied",
                std::io::ErrorKind::NotADirectory if cfg!(unix) => "Not a directory",
                _ => "No such file or directory",
            };
            eprintln!("{}: cd: {}: {}", shell.error_prefix(), target, msg);
            1
        }
    }
}

fn builtin_pwd(shell: &mut Shell, _args: &[String]) -> i32 {
    match std::env::current_dir() {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("{}: pwd: {}", shell.error_prefix(), e);
            1
        }
    }
}

fn builtin_export(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print all exported variables
        for (key, value) in &shell.exports {
            println!("declare -x {}=\"{}\"", key, value);
        }
        return 0;
    }

    let mut unexport = false;
    let mut export_funcs = false;
    let mut print_mode = false;
    let mut array_mode = false;
    let mut names = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-p" => print_mode = true,
            "-n" => unexport = true,
            "-f" => export_funcs = true,
            "-fn" | "-nf" => {
                unexport = true;
                export_funcs = true;
            }
            a if a.starts_with('-') => {
                if a.contains('a') {
                    array_mode = true;
                }
            }
            _ => names.push(arg.clone()),
        }
    }

    // export -f: export functions to environment
    if export_funcs && !unexport {
        let mut status = 0;
        for name in &names {
            // Reject names that can't be valid function names
            if name.contains('=') || name.contains('/') || name.is_empty() {
                eprintln!("{}: export: {}: cannot export", shell.error_prefix(), name);
                status = 1;
                continue;
            }
            if let Some(body) = shell.functions.get(name.as_str()) {
                let body_str = format_compound_command(body);
                let env_val = format!("() {}", body_str);
                let env_key = format!("BASH_FUNC_{}%%", name);
                unsafe { std::env::set_var(&env_key, &env_val) };
            } else {
                eprintln!("{}: export: {}: not a function", shell.error_prefix(), name);
                status = 1;
            }
        }
        return status;
    }

    if print_mode && names.is_empty() {
        let mut keys: Vec<&String> = shell.exports.keys().collect();
        keys.sort();
        for key in keys {
            let value = &shell.exports[key];
            println!("declare -x {}=\"{}\"", key, value);
        }
        return 0;
    }

    for arg in &names {
        if unexport {
            // Remove export attribute but keep the variable
            shell.exports.remove(arg.as_str());
            unsafe { std::env::remove_var(arg) };
        } else if let Some(eq_pos) = arg.find('=') {
            let (name, value, is_append) = if eq_pos > 0 && arg.as_bytes()[eq_pos - 1] == b'+' {
                (&arg[..eq_pos - 1], &arg[eq_pos + 1..], true)
            } else {
                (&arg[..eq_pos], &arg[eq_pos + 1..], false)
            };
            if array_mode && value.starts_with('(') && value.ends_with(')') {
                // -a flag with (value): parse as array
                let arr = parse_array_literal(value);
                let export_val = arr.first().cloned().unwrap_or_default();
                shell.arrays.insert(name.to_string(), arr);
                shell.exports.insert(name.to_string(), export_val.clone());
                unsafe { std::env::set_var(name, &export_val) };
            } else if shell.arrays.contains_key(name) {
                // Assigning scalar to existing array: set array[0]
                let final_value = value.to_string();
                if let Some(arr) = shell.arrays.get_mut(name) {
                    if arr.is_empty() {
                        arr.push(final_value.clone());
                    } else {
                        arr[0] = final_value.clone();
                    }
                }
                shell.exports.insert(name.to_string(), final_value.clone());
                unsafe { std::env::set_var(name, &final_value) };
            } else {
                let final_value = if is_append {
                    let existing = shell.vars.get(name).cloned().unwrap_or_default();
                    if shell.integer_vars.contains(name) {
                        let e = shell.eval_arith_expr(&existing);
                        let a = shell.eval_arith_expr(value);
                        (e + a).to_string()
                    } else {
                        format!("{}{}", existing, value)
                    }
                } else {
                    value.to_string()
                };
                shell.set_var(name, final_value.clone());
                shell.exports.insert(name.to_string(), final_value.clone());
                unsafe { std::env::set_var(name, &final_value) };
            }
        } else {
            // Export existing variable
            let value = shell
                .vars
                .get(arg.as_str())
                .cloned()
                .or_else(|| std::env::var(arg).ok())
                .unwrap_or_default();
            shell.exports.insert(arg.clone(), value.clone());
            unsafe { std::env::set_var(arg, &value) };
        }
    }
    0
}

fn builtin_unset(shell: &mut Shell, args: &[String]) -> i32 {
    let mut unset_functions = false;
    let mut unset_variables = false;
    let mut _unset_nameref = false;
    let mut names = Vec::new();
    let mut parsing_opts = true;

    for arg in args {
        if parsing_opts && arg.starts_with('-') && arg.len() > 1 {
            let opt = arg.as_str();
            match opt {
                "-v" => unset_variables = true,
                "-f" => unset_functions = true,
                "-n" => _unset_nameref = true,
                "--" => parsing_opts = false,
                _ => {
                    eprintln!(
                        "{}: unset: -{}: invalid option",
                        shell.error_prefix(),
                        &opt[1..]
                    );
                    eprintln!("unset: usage: unset [-f] [-v] [-n] [name ...]");
                    return 2;
                }
            }
        } else {
            parsing_opts = false;
            names.push(arg.as_str());
        }
    }

    // Cannot simultaneously unset functions and variables
    if unset_functions && unset_variables {
        eprintln!(
            "{}: unset: cannot simultaneously unset a function and a variable",
            shell.error_prefix()
        );
        return 1;
    }

    let mut status = 0;
    for name in names {
        // Validate identifier only with explicit -v flag (not default mode)
        if unset_variables && !name.contains('[') && !is_valid_identifier(name) {
            eprintln!(
                "{}: unset: `{}': not a valid identifier",
                shell.error_prefix(),
                name
            );
            status = 1;
            continue;
        }
        // Check for un-unsettable special variables
        if !unset_functions
            && matches!(
                name,
                "BASH_LINENO" | "BASH_SOURCE" | "FUNCNAME" | "GROUPS" | "DIRSTACK"
            )
        {
            eprintln!("{}: unset: {}: cannot unset", shell.error_prefix(), name);
            status = 1;
            continue;
        }
        if unset_functions {
            // Check if function is readonly
            if shell.readonly_funcs.contains(name) {
                eprintln!(
                    "{}: unset: {}: cannot unset: readonly function",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
            shell.functions.remove(name);
            // Also remove the exported function env var
            let env_key = format!("BASH_FUNC_{}%%", name);
            unsafe { std::env::remove_var(&env_key) };
        } else if let Some(bracket) = name.find('[') {
            // unset arr[n] — remove specific array element
            let base = &name[..bracket];
            let idx_str = &name[bracket + 1..name.len() - 1];
            let resolved = shell.resolve_nameref(base);
            if idx_str == "@" || idx_str == "*" {
                // unset arr[@] — remove entire array
                shell.arrays.remove(&resolved);
                shell.assoc_arrays.remove(&resolved);
                shell.vars.remove(&resolved);
            } else if shell.assoc_arrays.contains_key(&resolved) {
                shell
                    .assoc_arrays
                    .get_mut(&resolved)
                    .map(|a| a.remove(idx_str));
            } else {
                let raw_idx = shell.eval_arith_expr(idx_str);
                if raw_idx < 0
                    && let Some(arr) = shell.arrays.get(&resolved)
                    && raw_idx.abs() > arr.len() as i64
                {
                    eprintln!(
                        "{}: unset: [{}]: bad array subscript",
                        shell.error_prefix(),
                        raw_idx
                    );
                    status = 1;
                    continue;
                }
                if let Some(arr) = shell.arrays.get_mut(&resolved) {
                    let idx = if raw_idx < 0 {
                        let len = arr.len() as i64;
                        (len + raw_idx).max(0) as usize
                    } else {
                        raw_idx as usize
                    };
                    if idx < arr.len() {
                        arr[idx] = String::new();
                    }
                }
            }
        } else {
            let resolved = shell.resolve_nameref(name);
            if shell.readonly_vars.contains(&resolved) {
                eprintln!(
                    "{}: unset: {}: cannot unset: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
            shell.vars.remove(name);
            shell.exports.remove(name);
            shell.arrays.remove(name);
            shell.assoc_arrays.remove(name);
            shell.namerefs.remove(name);
            shell.integer_vars.remove(name);
            shell.uppercase_vars.remove(name);
            shell.lowercase_vars.remove(name);
            shell.capitalize_vars.remove(name);
            unsafe { std::env::remove_var(name) };
        }
    }
    status
}

fn builtin_readonly(shell: &mut Shell, args: &[String]) -> i32 {
    let mut func_mode = false;
    let mut print_mode = false;
    let mut array_mode = false;
    let mut names = Vec::new();

    for arg in args {
        if let Some(flags) = arg.strip_prefix('-') {
            for ch in flags.chars() {
                match ch {
                    'f' => func_mode = true,
                    'p' => print_mode = true,
                    'a' => array_mode = true,
                    'A' | 'n' => {} // assoc/nameref flags accepted
                    _ => {
                        eprintln!(
                            "{}: readonly: -{}: invalid option",
                            shell.error_prefix(),
                            ch
                        );
                        eprintln!(
                            "readonly: usage: readonly [-aAf] [name[=value] ...] or readonly -p"
                        );
                        return 2;
                    }
                }
            }
        } else {
            names.push(arg.as_str());
        }
    }

    let print_all = names.is_empty();
    if print_all && (args.is_empty() || print_mode) {
        if func_mode {
            // Print readonly functions
            let mut fnames: Vec<&String> = shell.readonly_funcs.iter().collect();
            fnames.sort();
            for name in fnames {
                println!("declare -fr {}", name);
            }
        } else {
            let mut vnames: Vec<&String> = shell.readonly_vars.iter().collect();
            vnames.sort();
            for name in vnames {
                let val = shell.vars.get(name).cloned().unwrap_or_default();
                println!("declare -r {}=\"{}\"", name, val);
            }
        }
        return 0;
    }

    let mut status = 0;
    for name in names {
        if func_mode {
            if shell.readonly_funcs.contains(name) {
                eprintln!(
                    "{}: readonly: {}: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
            } else if shell.functions.contains_key(name) {
                shell.readonly_funcs.insert(name.to_string());
            }
        } else if (shell.readonly_vars.contains(name)
            || name
                .find('=')
                .is_some_and(|eq| shell.readonly_vars.contains(&name[..eq])))
            && !(array_mode
                && name.find('=').is_some_and(|eq| {
                    let v = &name[eq + 1..];
                    v.starts_with('(') && v.ends_with(')')
                }))
        {
            // Already readonly — report error if trying to change value
            // (but skip when -a flag with (...) value — let it fall through for proper error)
            if name.contains('=') {
                let vname = name.split('=').next().unwrap();
                eprintln!("{}: {}: readonly variable", shell.error_prefix(), vname);
                status = 1;
            }
            // readonly without = on already readonly var is a no-op (not an error)
        } else if let Some(eq_pos) = name.find('=') {
            let (vname, value, is_append) = if eq_pos > 0 && name.as_bytes()[eq_pos - 1] == b'+' {
                (&name[..eq_pos - 1], &name[eq_pos + 1..], true)
            } else {
                (&name[..eq_pos], &name[eq_pos + 1..], false)
            };
            if is_append {
                if shell.integer_vars.contains(vname) {
                    let existing_str = shell.vars.get(vname).cloned().unwrap_or_default();
                    let existing = shell.eval_arith_expr(&existing_str);
                    let addend = shell.eval_arith_expr(value);
                    shell.set_var(vname, (existing + addend).to_string());
                } else {
                    let existing = shell.vars.get(vname).cloned().unwrap_or_default();
                    shell.set_var(vname, format!("{}{}", existing, value));
                }
            } else {
                // For quoted args (which reach here after parser-level handles unquoted),
                // assign as scalar. If the variable is already an array, set array[0].
                if shell.readonly_vars.contains(vname) {
                    eprintln!(
                        "{}: readonly: {}: readonly variable",
                        shell.error_prefix(),
                        vname
                    );
                    status = 1;
                } else if array_mode && value.starts_with('(') && value.ends_with(')') {
                    // -a flag with (value): parse as array
                    let arr = parse_array_literal(value);
                    shell.arrays.insert(vname.to_string(), arr);
                } else if shell.arrays.contains_key(vname) {
                    // Existing array: assign to element 0
                    if let Some(arr) = shell.arrays.get_mut(vname) {
                        if arr.is_empty() {
                            arr.push(value.to_string());
                        } else {
                            arr[0] = value.to_string();
                        }
                    }
                } else {
                    shell.set_var(vname, value.to_string());
                }
            }
            shell.readonly_vars.insert(vname.to_string());
        } else {
            shell.readonly_vars.insert(name.to_string());
        }
    }
    status
}

fn builtin_logout(shell: &mut Shell, _args: &[String]) -> i32 {
    if !shell.login_shell {
        eprintln!(
            "{}: logout: not login shell: use `exit'",
            shell.error_prefix()
        );
        return 1;
    }
    std::process::exit(shell.last_status);
}

fn builtin_local(shell: &mut Shell, args: &[String]) -> i32 {
    if shell.local_scopes.is_empty() {
        eprintln!(
            "{}: local: can only be used in a function",
            shell.error_prefix()
        );
        return 1;
    }
    let mut flag_array = false;
    let mut flag_readonly = false;
    let mut flag_nameref = false;
    let mut flag_integer = false;
    let mut names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "-" {
            // local - : save shell options for restoration on function return
            if let Some(last) = shell.saved_opts_stack.last_mut()
                && last.is_none()
            {
                *last = Some((
                    shell.opt_errexit,
                    shell.opt_nounset,
                    shell.opt_xtrace,
                    shell.opt_noclobber,
                    shell.opt_noglob,
                    shell.opt_pipefail,
                ));
            }
        } else if arg == "-p" {
            // local -p: print all local variables in declare format
            if let Some(scope) = shell.local_scopes.last() {
                let mut sorted: Vec<_> = scope.keys().collect();
                sorted.sort();
                for name in sorted {
                    if let Some(val) = shell.vars.get(name.as_str()) {
                        println!("{}={}", name, val);
                    } else {
                        println!("{}", name);
                    }
                }
            }
            return 0;
        } else if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'a' => flag_array = true,
                    'r' => flag_readonly = true,
                    'n' => flag_nameref = true,
                    'i' => flag_integer = true,
                    _ => {}
                }
            }
        } else {
            names.push(arg.clone());
        }
        i += 1;
    }

    for name_arg in &names {
        let var_name;
        if let Some(eq_pos) = name_arg.find('=') {
            let name = &name_arg[..eq_pos];
            let value = &name_arg[eq_pos + 1..];
            var_name = name.to_string();
            shell.declare_local(name);
            if flag_integer {
                shell.integer_vars.insert(name.to_string());
            }
            if flag_nameref {
                shell.namerefs.insert(name.to_string(), value.to_string());
            } else if flag_array {
                let arr = parse_array_literal(value);
                shell.arrays.insert(name.to_string(), arr);
            } else if flag_integer {
                let n = shell.eval_arith_expr(value);
                shell.set_var(name, n.to_string());
            } else {
                shell.set_var(name, value.to_string());
            }
        } else {
            var_name = name_arg.clone();
            shell.declare_local(name_arg);
            if flag_integer {
                shell.integer_vars.insert(name_arg.clone());
            }
            if flag_nameref {
                shell.namerefs.entry(name_arg.clone()).or_default();
            } else if flag_array {
                shell.arrays.entry(name_arg.clone()).or_default();
            } else {
                shell.vars.entry(name_arg.clone()).or_default();
            }
        }
        // Apply readonly attribute
        if flag_readonly {
            shell.readonly_vars.insert(var_name);
        }
    }
    0
}

// ── declare -f formatting helpers ──────────────────────────────────────────

fn format_word(word: &Word) -> String {
    let mut s = String::new();
    for part in word {
        match part {
            WordPart::Literal(t) => s.push_str(t),
            WordPart::SingleQuoted(t) => {
                // Use \char escaping for shell metacharacters (bash style)
                let all_meta = !t.is_empty()
                    && t.chars().all(|c| {
                        matches!(
                            c,
                            '$' | '`'
                                | '\\'
                                | '&'
                                | '|'
                                | ';'
                                | '<'
                                | '>'
                                | '{'
                                | '}'
                                | '%'
                                | '!'
                                | '#'
                                | '*'
                                | '?'
                                | '['
                                | ']'
                                | '~'
                        )
                    });
                if all_meta {
                    for ch in t.chars() {
                        s.push('\\');
                        s.push(ch);
                    }
                } else if t.chars().any(|c| c.is_ascii_control()) {
                    // Use $'...' for control characters
                    s.push_str("$'");
                    for ch in t.chars() {
                        match ch {
                            '\n' => s.push_str("\\n"),
                            '\t' => s.push_str("\\t"),
                            '\r' => s.push_str("\\r"),
                            '\'' => s.push_str("\\'"),
                            '\\' => s.push_str("\\\\"),
                            '\x07' => s.push_str("\\a"),
                            c if c.is_ascii_control() => {
                                s.push_str(&format!("\\{:03o}", c as u8));
                            }
                            c => s.push(c),
                        }
                    }
                    s.push('\'');
                } else {
                    s.push('\'');
                    s.push_str(t);
                    s.push('\'');
                }
            }
            WordPart::DoubleQuoted(parts) => {
                s.push('"');
                for p in parts {
                    match p {
                        WordPart::Literal(t) => s.push_str(t),
                        WordPart::Variable(name) => {
                            s.push('$');
                            s.push_str(name);
                        }
                        WordPart::Param(expr) => {
                            s.push_str(&format_param_expr(&expr.name, &expr.op));
                        }
                        WordPart::CommandSub(cmd) => {
                            s.push_str("$(");
                            s.push_str(cmd);
                            s.push(')');
                        }
                        WordPart::BacktickSub(cmd) => {
                            s.push('`');
                            s.push_str(cmd);
                            s.push('`');
                        }
                        WordPart::ArithSub(expr) => {
                            s.push_str("$((");
                            s.push_str(expr);
                            s.push_str("))");
                        }
                        _ => s.push_str(&format_word_part(p)),
                    }
                }
                s.push('"');
            }
            _ => s.push_str(&format_word_part(part)),
        }
    }
    s
}

fn format_word_part(part: &WordPart) -> String {
    match part {
        WordPart::Literal(t) => t.clone(),
        WordPart::SingleQuoted(t) => {
            if t.chars().any(|c| c.is_ascii_control()) {
                // Use $'...' format for strings with control characters
                let mut s = String::from("$'");
                for ch in t.chars() {
                    match ch {
                        '\n' => s.push_str("\\n"),
                        '\t' => s.push_str("\\t"),
                        '\r' => s.push_str("\\r"),
                        '\'' => s.push_str("\\'"),
                        '\\' => s.push_str("\\\\"),
                        '\x07' => s.push_str("\\a"),
                        '\x08' => s.push_str("\\b"),
                        '\x1b' => s.push_str("\\E"),
                        c if c.is_ascii_control() => {
                            s.push_str(&format!("\\{:03o}", c as u8));
                        }
                        c => s.push(c),
                    }
                }
                s.push('\'');
                s
            } else {
                format!("'{}'", t)
            }
        }
        WordPart::DoubleQuoted(parts) => {
            let mut s = String::from("\"");
            for p in parts {
                s.push_str(&format_word_part(p));
            }
            s.push('"');
            s
        }
        WordPart::Tilde(user) => format!("~{}", user),
        WordPart::Variable(name) => format!("${}", name),
        WordPart::Param(expr) => format_param_expr(&expr.name, &expr.op),
        WordPart::CommandSub(cmd) => {
            let trimmed = cmd.trim();
            // Normalize $(< file) — ensure space after <
            if let Some(rest) = trimmed.strip_prefix('<')
                && !rest.starts_with(' ')
                && !rest.starts_with('<')
            {
                return format!("$(< {})", rest.trim_start());
            }
            format!("$({})", trimmed)
        }
        WordPart::BacktickSub(cmd) => format!("`{}`", cmd),
        WordPart::ArithSub(expr) => format!("$(({}))", expr),
        WordPart::ProcessSub(kind, cmd) => match kind {
            ProcessSubKind::Input => format!("<({})", cmd),
            ProcessSubKind::Output => format!(">({})", cmd),
        },
        WordPart::BadSubstitution(expr) => expr.clone(),
    }
}

fn format_param_expr(name: &str, op: &ParamOp) -> String {
    match op {
        ParamOp::None => format!("${{{}}}", name),
        ParamOp::Length => format!("${{#{}}}", name),
        ParamOp::Indirect => format!("${{!{}}}", name),
        ParamOp::NamePrefix(ch) => format!("${{!{}{}}}", name, ch),
        ParamOp::ArrayIndices(ch) => format!("${{!{}[{}]}}", name, ch),
        ParamOp::Default(colon, w) => {
            let op_str = if *colon { ":-" } else { "-" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Assign(colon, w) => {
            let op_str = if *colon { ":=" } else { "=" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Error(colon, w) => {
            let op_str = if *colon { ":?" } else { "?" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::Alt(colon, w) => {
            let op_str = if *colon { ":+" } else { "+" };
            format!("${{{}{}{}}}", name, op_str, format_word(w))
        }
        ParamOp::TrimSmallLeft(w) => format!("${{{}#{}}}", name, format_word(w)),
        ParamOp::TrimLargeLeft(w) => format!("${{{}##{}}}", name, format_word(w)),
        ParamOp::TrimSmallRight(w) => format!("${{{}%{}}}", name, format_word(w)),
        ParamOp::TrimLargeRight(w) => format!("${{{}%%{}}}", name, format_word(w)),
        ParamOp::Replace(pat, rep) => {
            format!("${{{}/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplaceAll(pat, rep) => {
            format!("${{{}//{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplacePrefix(pat, rep) => {
            format!("${{{}/#/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::ReplaceSuffix(pat, rep) => {
            format!("${{{}/%/{}/{}}}", name, format_word(pat), format_word(rep))
        }
        ParamOp::Substring(offset, len) => {
            if let Some(l) = len {
                format!("${{{}:{}:{}}}", name, offset, l)
            } else {
                format!("${{{}:{}}}", name, offset)
            }
        }
        ParamOp::UpperFirst(w) => format!("${{{}^{}}}", name, format_word(w)),
        ParamOp::UpperAll(w) => format!("${{{}^^{}}}", name, format_word(w)),
        ParamOp::LowerFirst(w) => format!("${{{},{}}}", name, format_word(w)),
        ParamOp::LowerAll(w) => format!("${{{},, {}}}", name, format_word(w)),
        ParamOp::ToggleFirst(w) => format!("${{{}~{}}}", name, format_word(w)),
        ParamOp::ToggleAll(w) => format!("${{{}~~{}}}", name, format_word(w)),
        ParamOp::Transform(ch) => format!("${{{}@{}}}", name, ch),
    }
}

fn format_redirection(redir: &Redirection) -> String {
    let mut s = String::new();
    if let Some(ref fd) = redir.fd {
        match fd {
            RedirFd::Number(n) => {
                // Only print fd number when it differs from the default
                match redir.kind {
                    RedirectKind::DupInput | RedirectKind::DupOutput => {
                        // Always print fd for dup redirects
                        s.push_str(&n.to_string());
                    }
                    RedirectKind::Input
                    | RedirectKind::ReadWrite
                    | RedirectKind::HereDoc(_, _)
                    | RedirectKind::HereString
                    | RedirectKind::ProcessSubIn => {
                        if *n != 0 {
                            s.push_str(&n.to_string());
                        }
                    }
                    _ => {
                        if *n != 1 {
                            s.push_str(&n.to_string());
                        }
                    }
                }
            }
            RedirFd::Var(name) => {
                s.push('{');
                s.push_str(name);
                s.push('}');
            }
        }
    }
    match redir.kind {
        RedirectKind::Input => s.push_str("< "),
        RedirectKind::Output => s.push_str("> "),
        RedirectKind::Append => s.push_str(">> "),
        RedirectKind::Clobber => s.push_str(">| "),
        RedirectKind::DupInput => s.push_str("<&"),
        RedirectKind::DupOutput => s.push_str(">&"),
        RedirectKind::ReadWrite => s.push_str("<> "),
        RedirectKind::HereDoc(strip, ref delim) => {
            if strip {
                s.push_str("<<-");
            } else {
                s.push_str("<<");
            }
            if !delim.is_empty() {
                s.push_str(delim);
                s.push('\n');
                s.push_str(&format_word(&redir.target));
                s.push('\n');
                s.push_str(delim);
                return s;
            }
        }
        RedirectKind::HereString => s.push_str("<<< "),
        RedirectKind::OutputAll => s.push_str("&> "),
        RedirectKind::AppendAll => s.push_str("&>> "),
        RedirectKind::ProcessSubIn => s.push_str("< "),
        RedirectKind::ProcessSubOut => s.push_str("> "),
    }
    s.push_str(&format_word(&redir.target));
    s
}

fn format_simple_command(cmd: &SimpleCommand) -> String {
    let mut parts = Vec::new();
    for a in &cmd.assignments {
        let op = if a.append { "+=" } else { "=" };
        match &a.value {
            AssignValue::None => parts.push(a.name.clone()),
            AssignValue::Scalar(w) => parts.push(format!("{}{}{}", a.name, op, format_word(w))),
            AssignValue::Array(elements) => {
                let elems: Vec<String> = elements
                    .iter()
                    .map(|e| {
                        if let Some(ref idx) = e.index {
                            format!("[{}]={}", format_word(idx), format_word(&e.value))
                        } else {
                            format_word(&e.value)
                        }
                    })
                    .collect();
                parts.push(format!("{}{}({})", a.name, op, elems.join(" ")));
            }
        }
    }
    for w in &cmd.words {
        parts.push(format_word(w));
    }
    for r in &cmd.redirections {
        parts.push(format_redirection(r));
    }
    parts.join(" ")
}

fn format_pipeline_indent(pipeline: &Pipeline, indent: usize) -> String {
    let mut s = String::new();
    if pipeline.negated {
        s.push_str("! ");
    }
    if pipeline.timed {
        if pipeline.time_posix {
            s.push_str("time -p ");
        } else {
            s.push_str("time ");
        }
    }
    let cmds: Vec<String> = pipeline
        .commands
        .iter()
        .map(|c| format_command_indent(c, indent))
        .collect();
    s.push_str(&cmds.join(" | "));
    s
}

fn format_command_indent(cmd: &Command, indent: usize) -> String {
    match cmd {
        Command::Simple(sc) => format_simple_command(sc),
        Command::Compound(cc, redirections) => {
            let mut s = format_compound_command_indent(cc, indent);
            for r in redirections {
                s.push(' ');
                s.push_str(&format_redirection(r));
            }
            s
        }
        Command::FunctionDef(name, body, _, _) => {
            format!("{} () \n{}", name, format_compound_command(body))
        }
        Command::Coproc(name, inner) => {
            let inner_str = format_command_indent(inner, indent);
            match name.as_deref() {
                Some("COPROC") | None => format!("coproc {}", inner_str),
                Some(n) => format!("coproc {} {}", n, inner_str),
            }
        }
    }
}

fn format_program(program: &Program, indent: usize) -> String {
    format_program_impl(program, indent, indent > 1)
}

/// Format a program with control over whether the last command gets a semicolon
fn format_program_impl(program: &Program, indent: usize, semi_last: bool) -> String {
    let prefix = "    ".repeat(indent);
    let mut lines = Vec::new();
    let mut pending_bg: Option<String> = None;
    for (idx, cc) in program.iter().enumerate() {
        let mut line = String::new();
        // If previous command was background, combine on same line
        if let Some(bg_line) = pending_bg.take() {
            line.push_str(&bg_line);
            line.push(' ');
        } else {
            line.push_str(&prefix);
        }
        line.push_str(&format_pipeline_indent(&cc.list.first, indent));
        for (op, pipeline) in &cc.list.rest {
            match op {
                AndOr::And => line.push_str(" && "),
                AndOr::Or => line.push_str(" || "),
            }
            line.push_str(&format_pipeline_indent(pipeline, indent));
        }
        if cc.background {
            line.push_str(" &");
            // Save this line to combine with next command
            pending_bg = Some(line);
            continue;
        }
        // Add semicolons after commands (bash style):
        {
            let is_last = idx == program.len() - 1;
            let add_semi = if is_last { semi_last } else { true };
            if add_semi {
                let trimmed = line.trim_end();
                let is_keyword = trimmed.ends_with('{')
                    || trimmed.ends_with("then")
                    || trimmed.ends_with("do")
                    || trimmed.ends_with("else");
                if !is_keyword && !trimmed.ends_with('&') && !trimmed.is_empty() {
                    line.push(';');
                }
            }
        }
        lines.push(line);
    }
    // If last command was background, push it
    if let Some(bg_line) = pending_bg {
        lines.push(bg_line);
    }
    lines.join("\n")
}

fn format_cond_expr(expr: &CondExpr) -> String {
    match expr {
        CondExpr::Unary(op, word) => format!("{} {}", op, format_word(word)),
        CondExpr::Binary(left, op, right) => {
            format!("{} {} {}", format_word(left), op, format_word(right))
        }
        CondExpr::Not(inner) => format!("! {}", format_cond_expr(inner)),
        CondExpr::And(left, right) => {
            format!("{} && {}", format_cond_expr(left), format_cond_expr(right))
        }
        CondExpr::Or(left, right) => {
            format!("{} || {}", format_cond_expr(left), format_cond_expr(right))
        }
        CondExpr::Word(word) => format_word(word),
    }
}

fn format_compound_command(cmd: &CompoundCommand) -> String {
    format_compound_command_indent(cmd, 0)
}

fn format_compound_command_indent(cmd: &CompoundCommand, indent: usize) -> String {
    let iprefix = "    ".repeat(indent);
    match cmd {
        CompoundCommand::BraceGroup(program) => {
            if program.is_empty() {
                "{ \n}".to_string()
            } else {
                format!(
                    "{{ \n{}\n{}}}",
                    format_program_impl(program, indent + 1, false),
                    iprefix
                )
            }
        }
        CompoundCommand::Subshell(program) => {
            let body = format_program(program, 0);
            let trimmed = body.trim();
            if !trimmed.contains('\n') {
                format!("( {} )", trimmed.trim_end_matches(';'))
            } else {
                // Check if body is a single compound command with a brace group
                // and redirections on the command — format as ( { ... } ) redirects
                let single_compound = if program.len() == 1
                    && program[0].list.rest.is_empty()
                    && !program[0].background
                    && program[0].list.first.commands.len() == 1
                {
                    let cmd = &program[0].list.first.commands[0];
                    if let crate::ast::Command::Compound(
                        CompoundCommand::BraceGroup(inner),
                        redirs,
                    ) = cmd
                    {
                        Some((inner, redirs))
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some((inner, redirs)) = single_compound {
                    let inner_body = format_program_impl(inner, indent + 1, false);
                    let redir_str: String = redirs
                        .iter()
                        .map(|r| format!(" {}", format_redirection(r)))
                        .collect();
                    format!("( {{ \n{}\n{}}} ){redir_str}", inner_body, iprefix,)
                } else {
                    format!("( \n{}\n{})", format_program(program, indent + 1), iprefix)
                }
            }
        }
        CompoundCommand::If(clause) => {
            let mut s = String::from("if ");
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            s.push_str(cond);
            s.push_str("; then\n");
            s.push_str(&format_program(&clause.then_body, indent + 1));
            // Bash expands elif to nested else { if ... fi }
            let mut remaining_elifs = clause.elif_parts.iter().peekable();
            let else_body_ref = clause.else_body.as_ref();
            if remaining_elifs.peek().is_some() {
                // Build nested else/if structure
                let mut else_content = String::new();
                let mut nest_level = 0;
                for (elif_cond, elif_body) in remaining_elifs {
                    let inner_prefix = "    ".repeat(indent + 1 + nest_level);
                    let c = format_program(elif_cond, 0);
                    let c = c.trim().trim_end_matches(';');
                    else_content.push_str(&format!(
                        "\n{}else\n{}if {}; then\n{}",
                        "    ".repeat(indent + nest_level),
                        inner_prefix,
                        c,
                        format_program(elif_body, indent + 2 + nest_level)
                    ));
                    nest_level += 1;
                }
                if let Some(eb) = else_body_ref {
                    else_content.push_str(&format!(
                        "\n{}else\n{}",
                        "    ".repeat(indent + nest_level),
                        format_program(eb, indent + 1 + nest_level)
                    ));
                }
                // Close all nested fi's (all get ; since they're inside the else)
                for i in (0..nest_level).rev() {
                    else_content.push_str(&format!("\n{}fi;", "    ".repeat(indent + 1 + i)));
                }
                s.push_str(&else_content);
            } else if let Some(else_body) = else_body_ref {
                s.push_str(&format!("\n{iprefix}else\n"));
                s.push_str(&format_program(else_body, indent + 1));
            }
            s.push_str(&format!("\n{iprefix}fi"));
            s
        }
        CompoundCommand::For(clause) => {
            let mut s = format!("for {} in", clause.var);
            if let Some(ref words) = clause.words {
                for w in words {
                    s.push(' ');
                    s.push_str(&format_word(w));
                }
            }
            s.push_str(&format!(";\n{iprefix}do\n"));
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::ArithFor(clause) => {
            let init = if clause.init.trim().is_empty() {
                "1".to_string()
            } else {
                clause.init.trim().to_string()
            };
            let cond = if clause.cond.trim().is_empty() {
                "1".to_string()
            } else {
                clause.cond.trim().to_string()
            };
            // Step: keep trailing whitespace from original, empty → "1"
            let step_part = if clause.step.trim().is_empty() {
                "1".to_string()
            } else {
                // Trim start but keep trailing whitespace
                clause.step.trim_start().to_string()
            };
            let mut s = format!("for (({init}; {cond}; {step_part}))\n{iprefix}do\n");
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::While(clause) => {
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            let mut s = format!("while {}; do\n", cond);
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::Until(clause) => {
            let cond = format_program(&clause.condition, 0);
            let cond = cond.trim().trim_end_matches(';');
            let mut s = format!("until {}; do\n", cond);
            s.push_str(&format_program(&clause.body, indent + 1));
            s.push_str(&format!("\n{iprefix}done"));
            s
        }
        CompoundCommand::Case(clause) => {
            let pat_prefix = "    ".repeat(indent + 1);
            let mut s = format!("case {} in \n", format_word(&clause.word));
            for item in &clause.items {
                let patterns: Vec<String> = item.patterns.iter().map(format_word).collect();
                s.push_str(&format!("{pat_prefix}{})\n", patterns.join(" | ")));
                let body = format_program(&item.body, indent + 2);
                let body = body.trim_end_matches(';');
                s.push_str(body);
                s.push('\n');
                let term = match item.terminator {
                    CaseTerminator::Break => ";;",
                    CaseTerminator::FallThrough => ";&",
                    CaseTerminator::TestNext => ";;&",
                };
                s.push_str(&format!("{pat_prefix}{term}\n"));
            }
            s.push_str(&format!("{iprefix}esac"));
            s
        }
        CompoundCommand::Conditional(expr) => {
            format!("[[ {} ]]", format_cond_expr(expr))
        }
        CompoundCommand::Arithmetic(expr) => {
            format!("(( {} ))", expr.trim())
        }
    }
}

fn builtin_declare(shell: &mut Shell, args: &[String]) -> i32 {
    let mut flag_array = false;
    let mut flag_assoc = false; // -A stub
    let mut flag_print = false;
    let mut flag_functions = false;
    let mut flag_func_body = false;
    let mut flag_nameref = false;
    let mut flag_readonly = false;
    let mut flag_unset_readonly = false;
    let mut flag_export = false;
    let mut flag_integer = false;
    let mut flag_uppercase = false;
    let mut flag_lowercase = false;
    let mut flag_capitalize = false;
    let mut flag_global = false; // -g stub
    let mut flag_trace = false;
    let mut names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // End of options — remaining args are names
            i += 1;
            while i < args.len() {
                names.push(args[i].clone());
                i += 1;
            }
            break;
        } else if arg.starts_with('-') && arg.len() > 1 && !arg.contains('=') {
            for ch in arg[1..].chars() {
                match ch {
                    'a' => flag_array = true,
                    'A' => flag_assoc = true,
                    'p' => flag_print = true,
                    'f' => flag_func_body = true,
                    'F' => flag_functions = true,
                    'n' => flag_nameref = true,
                    'r' => flag_readonly = true,
                    'x' => flag_export = true,
                    'i' => flag_integer = true,
                    'u' => flag_uppercase = true,
                    'l' => flag_lowercase = true,
                    'c' => flag_capitalize = true,
                    'g' => flag_global = true,
                    't' => flag_trace = true,
                    'I' => {} // inherit — accepted but not implemented
                    _ => {
                        eprintln!("{}: declare: -{}: invalid option", shell.error_prefix(), ch);
                        eprintln!(
                            "declare: usage: declare [-aAfFgiIlnrtux] [name[=value] ...] or declare -p [-aAfFilnrtux] [name ...]"
                        );
                        return 2;
                    }
                }
            }
        } else if arg.starts_with('+') && arg.len() > 1 {
            // +<flag> unsets attribute
            for ch in arg[1..].chars() {
                if ch == 'r' {
                    flag_unset_readonly = true;
                }
            }
        } else {
            names.push(arg.clone());
        }
        i += 1;
    }

    let _ = flag_global; // stub

    // Check for -f combined with other attributes (invalid)
    if flag_func_body && !names.is_empty() {
        if flag_array {
            eprintln!("{}: declare: -a: invalid option", shell.error_prefix());
            return 1;
        }
        if flag_integer {
            eprintln!("{}: declare: -i: invalid option", shell.error_prefix());
            return 1;
        }
        // Cannot use declare -f to define functions (name=value)
        for name in &names {
            if name.contains('=') {
                eprintln!(
                    "{}: declare: cannot use `-f' to make functions",
                    shell.error_prefix()
                );
                return 1;
            }
        }
    }

    // Check for readonly function operations
    if flag_func_body && !names.is_empty() {
        for name in &names {
            let pure_name = name.split('=').next().unwrap_or(name);
            if shell.readonly_funcs.contains(pure_name) && !flag_readonly {
                eprintln!(
                    "{}: declare: {}: readonly function",
                    shell.error_prefix(),
                    pure_name
                );
                return 1;
            }
        }
    }

    // Validate identifiers (for non-function, non-print modes)
    if !flag_func_body && !flag_functions && !flag_print {
        let mut status = 0;
        for name in &names {
            let pure_name = name.split('=').next().unwrap_or(name);
            let pure_name = pure_name.strip_suffix('+').unwrap_or(pure_name);
            if !pure_name.is_empty()
                && !pure_name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '[' || c == ']')
                || pure_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '/')
            {
                eprintln!(
                    "{}: declare: `{}': not a valid identifier",
                    shell.error_prefix(),
                    pure_name
                );
                status = 1;
            }
        }
        if status != 0 && names.len() == 1 {
            return status;
        }
    }

    // declare -f: print function definitions (with body)
    // But if -r is also set (declare -fr), it's trying to set readonly, not print
    // declare -ft: mark functions as traced
    if flag_func_body && flag_trace && !names.is_empty() {
        for name in &names {
            shell.traced_funcs.insert(name.clone());
        }
        return 0;
    }

    if flag_func_body && !flag_readonly && !flag_unset_readonly && !flag_trace {
        let print_func = |name: &str, body: &CompoundCommand| {
            println!("{} () \n{}", name, format_compound_command(body));
        };
        if names.is_empty() {
            let mut fnames: Vec<&String> = shell.functions.keys().collect();
            fnames.sort();
            for name in fnames {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    print_func(name, body);
                }
            }
        } else {
            let mut found = false;
            for name in &names {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    print_func(name, body);
                    found = true;
                }
            }
            if !found {
                return 1;
            }
        }
        return 0;
    }

    // declare -F: list function names
    if flag_functions {
        if names.is_empty() {
            let mut all_funcs: Vec<String> = shell.func_names.to_vec();
            for name in shell.functions.keys() {
                if !all_funcs.contains(name) {
                    all_funcs.push(name.clone());
                }
            }
            all_funcs.sort();
            for name in &all_funcs {
                let is_ro = shell.readonly_funcs.contains(name.as_str());
                if flag_readonly && !is_ro {
                    continue;
                }
                let flags = if is_ro { "-fr" } else { "-f" };
                println!("declare {} {}", flags, name);
            }
        } else {
            for name in &names {
                if shell.functions.contains_key(name.as_str()) || shell.func_names.contains(name) {
                    let is_ro = shell.readonly_funcs.contains(name.as_str());
                    let flags = if is_ro { "-fr" } else { "-f" };
                    println!("declare {} {}", flags, name);
                } else {
                    return 1;
                }
            }
        }
        return 0;
    }

    // declare -p: print variable info
    if flag_print {
        if names.is_empty() {
            // Print all variables
            let mut var_names: Vec<&String> = shell.vars.keys().collect();
            var_names.sort();
            for name in var_names {
                let value = shell.vars.get(name).cloned().unwrap_or_default();
                if shell.namerefs.contains_key(name) {
                    println!("declare -n {}=\"{}\"", name, shell.namerefs[name]);
                } else if shell.arrays.contains_key(name) {
                    let mut flags = String::from("-a");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    let arr = &shell.arrays[name];
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]={}", i, quote_for_declare(v)))
                        .collect();
                    println!("declare {} {}=({})", flags, name, elements.join(" "));
                } else {
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name) {
                        flags.push('i');
                    }
                    if shell.readonly_vars.contains(name) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name) {
                        flags.push('x');
                    }
                    if flags == "-" {
                        flags.push('-');
                    }
                    println!("declare {} {}={}", flags, name, quote_for_declare(&value));
                }
            }
            // Also print arrays not in vars
            let mut arr_names: Vec<&String> = shell.arrays.keys().collect();
            arr_names.sort();
            for name in arr_names {
                if !shell.vars.contains_key(name) {
                    let mut flags = String::from("-a");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    let arr = &shell.arrays[name];
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]={}", i, quote_for_declare(v)))
                        .collect();
                    println!("declare {} {}=({})", flags, name, elements.join(" "));
                }
            }
            // Also print associative arrays
            let mut assoc_names: Vec<&String> = shell.assoc_arrays.keys().collect();
            assoc_names.sort();
            for name in assoc_names {
                let assoc = &shell.assoc_arrays[name];
                let elements: Vec<String> = assoc
                    .iter()
                    .map(|(k, v)| format!("[{}]={}", k, quote_for_declare(v)))
                    .collect();
                println!("declare -A {}=({} )", name, elements.join(" "));
            }
            // Print namerefs not in vars
            let mut nref_names: Vec<&String> = shell.namerefs.keys().collect();
            nref_names.sort();
            for name in nref_names {
                if !shell.vars.contains_key(name) {
                    println!("declare -n {}=\"{}\"", name, shell.namerefs[name]);
                }
            }
        } else {
            for name in &names {
                if let Some(target) = shell.namerefs.get(name) {
                    println!("declare -n {}=\"{}\"", name, target);
                } else if let Some(arr) = shell.arrays.get(name) {
                    let mut flags = String::from("-a");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]={}", i, quote_for_declare(v)))
                        .collect();
                    println!("declare {} {}=({})", flags, name, elements.join(" "));
                } else if let Some(assoc) = shell.assoc_arrays.get(name) {
                    let mut flags = String::from("-A");
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    let elements: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| format!("[{}]={}", k, quote_for_declare(v)))
                        .collect();
                    println!("declare {} {}=({} )", flags, name, elements.join(" "));
                } else if let Some(value) = shell.vars.get(name) {
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if flags == "-" {
                        flags.push('-');
                    }
                    println!("declare {} {}={}", flags, name, quote_for_declare(value));
                } else {
                    eprintln!("{}: declare: {}: not found", shell.error_prefix(), name);
                    return 1;
                }
            }
        }
        return 0;
    }

    // declare -x with no names: list exports
    if flag_export && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.exports.iter().collect();
        sorted.sort_by_key(|(k, _)| k.to_string());
        for (name, value) in sorted {
            // Use current var value if available
            let val = shell.vars.get(name).unwrap_or(value);
            println!(
                "declare -x {}=\"{}\"",
                name,
                val.replace('\\', "\\\\").replace('"', "\\\"")
            );
        }
        return 0;
    }

    // declare -r with no names: list readonly variables
    if flag_readonly && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.readonly_vars.iter().collect();
        sorted.sort();
        for name in sorted {
            if let Some(val) = shell.vars.get(name) {
                println!(
                    "declare -r {}=\"{}\"",
                    name,
                    val.replace('\\', "\\\\").replace('"', "\\\"")
                );
            } else {
                println!("declare -r {}", name);
            }
        }
        return 0;
    }

    // declare -i with no names: list integer variables
    if flag_integer && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.integer_vars.iter().collect();
        sorted.sort();
        for name in sorted {
            if let Some(val) = shell.vars.get(name) {
                println!("declare -i {}=\"{}\"", name, val);
            }
        }
        return 0;
    }

    // declare -a with no names: list all indexed arrays
    if flag_array && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.arrays.keys().collect();
        sorted.sort();
        for name in sorted {
            if let Some(arr) = shell.arrays.get(name) {
                let elements: Vec<String> = arr
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                    .collect();
                println!("declare -a {}=({})", name, elements.join(" "));
            }
        }
        return 0;
    }

    // declare -n with no names: list all namerefs
    if flag_nameref && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.namerefs.iter().collect();
        sorted.sort_by_key(|(k, _)| k.to_string());
        for (name, target) in sorted {
            println!("declare -n {}=\"{}\"", name, target);
        }
        return 0;
    }

    // declare -A with no names: list all associative arrays
    if flag_assoc && names.is_empty() && !flag_print {
        let mut sorted: Vec<_> = shell.assoc_arrays.keys().collect();
        sorted.sort();
        for name in sorted {
            if let Some(assoc) = shell.assoc_arrays.get(name) {
                let elements: Vec<String> = assoc
                    .iter()
                    .map(|(k, v)| format!("[{}]=\"{}\"", k, v))
                    .collect();
                println!("declare -A {}=({})", name, elements.join(" "));
            }
        }
        return 0;
    }

    // Normal declare: set variables
    // In a function context, declare/typeset creates local variables (unless -g)
    let make_local = !flag_global && !shell.local_scopes.is_empty();

    let mut status = 0;
    for name_arg in &names {
        if let Some(eq_pos) = name_arg.find('=') {
            let (name, value, is_append) = if eq_pos > 0 && name_arg.as_bytes()[eq_pos - 1] == b'+'
            {
                (&name_arg[..eq_pos - 1], &name_arg[eq_pos + 1..], true)
            } else {
                (&name_arg[..eq_pos], &name_arg[eq_pos + 1..], false)
            };

            // Check if variable is readonly
            if shell.readonly_vars.contains(name) && !make_local {
                eprintln!(
                    "{}: declare: {}: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }

            if make_local {
                shell.declare_local(name);
            }

            if flag_nameref {
                shell.namerefs.insert(name.to_string(), value.to_string());
            } else if flag_assoc {
                let map = parse_assoc_literal(value);
                shell.assoc_arrays.insert(name.to_string(), map);
                if flag_integer {
                    shell.integer_vars.insert(name.to_string());
                }
            } else if flag_array {
                let arr = parse_array_literal(value);
                shell.arrays.insert(name.to_string(), arr);
                if flag_integer {
                    shell.integer_vars.insert(name.to_string());
                }
            } else if flag_integer {
                // Mark as integer and evaluate as arithmetic
                shell.integer_vars.insert(name.to_string());
                let n = shell.eval_arith_expr(value);
                if is_append {
                    let existing = shell
                        .vars
                        .get(name)
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0);
                    shell.set_var(name, (existing + n).to_string());
                } else {
                    shell.set_var(name, n.to_string());
                }
            } else if is_append {
                // Check if variable already has integer attribute
                if shell.integer_vars.contains(name) {
                    let existing_str = shell.vars.get(name).cloned().unwrap_or_default();
                    let existing = shell.eval_arith_expr(&existing_str);
                    let addend = shell.eval_arith_expr(value);
                    shell.set_var(name, (existing + addend).to_string());
                } else {
                    let existing = shell.vars.get(name).cloned().unwrap_or_default();
                    shell.set_var(name, format!("{}{}", existing, value));
                }
            } else {
                shell.set_var(name, value.to_string());
            }

            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
            if flag_export {
                let val = shell.get_var(name).unwrap_or_default();
                shell.exports.insert(name.to_string(), val.clone());
                unsafe { std::env::set_var(name, &val) };
            }
            if flag_uppercase {
                shell.uppercase_vars.insert(name.to_string());
                shell.lowercase_vars.remove(name);
                // Apply to current value
                if let Some(v) = shell.vars.get(name).cloned() {
                    shell.vars.insert(name.to_string(), v.to_uppercase());
                }
            }
            if flag_lowercase {
                shell.lowercase_vars.insert(name.to_string());
                shell.uppercase_vars.remove(name);
                shell.capitalize_vars.remove(name);
                if let Some(v) = shell.vars.get(name).cloned() {
                    shell.vars.insert(name.to_string(), v.to_lowercase());
                }
            }
            if flag_capitalize {
                shell.capitalize_vars.insert(name.to_string());
                shell.uppercase_vars.remove(name);
                shell.lowercase_vars.remove(name);
                if let Some(v) = shell.vars.get(name).cloned() {
                    let cap = capitalize_string(&v);
                    shell.vars.insert(name.to_string(), cap);
                }
            }
        } else {
            let name = name_arg.as_str();
            // Can't remove readonly attribute
            if flag_unset_readonly && shell.readonly_vars.contains(name) {
                eprintln!(
                    "{}: declare: {}: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }

            if make_local {
                shell.declare_local(name);
            }
            if flag_nameref {
                shell.namerefs.entry(name.to_string()).or_default();
            } else if flag_assoc {
                shell.assoc_arrays.entry(name.to_string()).or_default();
            } else if flag_array {
                shell.arrays.entry(name.to_string()).or_default();
            } else {
                shell.vars.entry(name.to_string()).or_default();
            }

            if flag_integer {
                shell.integer_vars.insert(name.to_string());
            }
            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
            if flag_export {
                let val = shell.get_var(name).unwrap_or_default();
                shell.exports.insert(name.to_string(), val.clone());
                unsafe { std::env::set_var(name, &val) };
            }
            if flag_uppercase {
                shell.uppercase_vars.insert(name.to_string());
                shell.lowercase_vars.remove(name);
            }
            if flag_lowercase {
                shell.lowercase_vars.insert(name.to_string());
                shell.uppercase_vars.remove(name);
                shell.capitalize_vars.remove(name);
            }
            if flag_capitalize {
                shell.capitalize_vars.insert(name.to_string());
                shell.uppercase_vars.remove(name);
                shell.lowercase_vars.remove(name);
            }
        }
    }
    status
}

/// Parse an associative array literal: `([key1]=val1 [key2]=val2 ...)`
fn parse_assoc_literal(s: &str) -> crate::interpreter::AssocArray {
    let trimmed = s.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    let mut map = crate::interpreter::AssocArray::default();
    // Split on \x1F separator (from inline array parser) or whitespace
    let entries: Vec<&str> = if inner.contains('\x1F') {
        inner.split('\x1F').collect()
    } else {
        vec![inner]
    };
    for entry in entries {
        let mut rest = entry.trim();
        while !rest.is_empty() {
            if rest.starts_with('[')
                && let Some(close) = rest.find("]=")
            {
                let key = &rest[1..close];
                let after = &rest[close + 2..];
                let (value, remaining) = if let Some(stripped) = after.strip_prefix('"') {
                    if let Some(end) = stripped.find('"') {
                        (&stripped[..end], stripped[end + 1..].trim_start())
                    } else {
                        (after, "")
                    }
                } else if let Some(stripped) = after.strip_prefix('\'') {
                    if let Some(end) = stripped.find('\'') {
                        (&stripped[..end], stripped[end + 1..].trim_start())
                    } else {
                        (after, "")
                    }
                } else {
                    let end = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..end], after[end..].trim_start())
                };
                map.insert(key.to_string(), value.to_string());
                rest = remaining;
                continue;
            }
            // Skip unknown content
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            rest = rest[end..].trim_start();
        }
    }
    map
}

/// Parse a bash array literal like `(val1 val2 val3)` into a Vec.
pub fn parse_array_literal(s: &str) -> Vec<String> {
    let trimmed = s.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    if inner.trim().is_empty() {
        return Vec::new();
    }

    // Check for \x1F separator (from parser's inline array handling)
    if inner.contains('\x1F') {
        return inner.split('\x1F').map(|s| s.to_string()).collect();
    }

    // Simple word splitting, respecting quotes
    let mut elements = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        if ch == '\\' && !in_single_quote {
            escape_next = true;
            continue;
        }
        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }
        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }
        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                elements.push(current.clone());
                current.clear();
            }
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() {
        elements.push(current);
    }
    elements
}

/// Quote a value for `set` output, matching bash's format.
/// Values that need quoting are wrapped in $'...' with proper escaping.
fn quote_value_for_set(value: &str) -> String {
    // Check if the value needs quoting
    let needs_quoting = value.is_empty()
        || value.starts_with('~')
        || value.starts_with('#')
        || value
            .chars()
            .any(|c| " \t\n\\\"'`$!&|;()<>{}[]?*".contains(c));

    if !needs_quoting {
        return value.to_string();
    }

    // Use single-quote style with \' for embedded single quotes
    // Bash uses a mix: simple values get \-escaping, complex ones get $'...' or '...'
    let mut out = String::new();
    let mut needs_dollar = false;

    for ch in value.chars() {
        match ch {
            '\n' | '\t' | '\r' | '\x07' | '\x08' | '\x0b' | '\x0c' | '\x1b' => {
                needs_dollar = true;
            }
            _ => {}
        }
    }

    if needs_dollar {
        out.push_str("$'");
        for ch in value.chars() {
            match ch {
                '\'' => out.push_str("\\'"),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\x07' => out.push_str("\\a"),
                '\x08' => out.push_str("\\b"),
                '\x0b' => out.push_str("\\v"),
                '\x0c' => out.push_str("\\f"),
                '\x1b' => out.push_str("\\E"),
                c if c.is_control() => {
                    out.push_str(&format!("\\x{:02x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('\'');
    } else if value.contains('\'') {
        // Value contains single quotes — use backslash escaping
        for ch in value.chars() {
            if ch == '\'' {
                out.push('\\');
            }
            out.push(ch);
        }
    } else {
        // Wrap in single quotes
        out.push('\'');
        out.push_str(value);
        out.push('\'');
    }

    out
}

fn builtin_set(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print all variables with proper quoting (like bash)
        let mut vars: Vec<_> = shell.vars.iter().collect();
        vars.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in vars {
            println!("{}={}", key, quote_value_for_set(value));
        }
        return 0;
    }

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Set positional parameters
            let new_positional: Vec<String> = args[i + 1..].to_vec();
            let prog = shell.positional.first().cloned().unwrap_or_default();
            shell.positional = vec![prog];
            shell.positional.extend(new_positional);
            return 0;
        }
        if arg.starts_with('-') || arg.starts_with('+') {
            let enable = arg.starts_with('-');
            let flags = &arg[1..];

            if flags == "o" {
                // set -o option / set +o option
                i += 1;
                if i < args.len() {
                    let option = &args[i];
                    match option.as_str() {
                        "allexport" => shell.opt_allexport = enable,
                        "pipefail" => shell.opt_pipefail = enable,
                        "errexit" => shell.opt_errexit = enable,
                        "nounset" => shell.opt_nounset = enable,
                        "xtrace" => shell.opt_xtrace = enable,
                        "noclobber" => shell.opt_noclobber = enable,
                        "keyword" => shell.opt_keyword = enable,
                        "noglob" => shell.opt_noglob = enable,
                        "noexec" => shell.opt_noexec = enable,
                        "posix" => shell.opt_posix = enable,
                        "hashall" => shell.opt_hashall = enable,
                        "braceexpand"
                        | "emacs"
                        | "errtrace"
                        | "functrace"
                        | "histexpand"
                        | "history"
                        | "ignoreeof"
                        | "interactive-comments"
                        | "monitor" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                            shell.opt_monitor = enable;
                        }
                        "notify" | "onecmd" | "physical" | "privileged" | "verbose" | "vi" => {
                            shell.shopt_options.insert(option.to_string(), enable);
                        }
                        _ => {
                            eprintln!(
                                "{}: set: {}: invalid option name",
                                shell.error_prefix(),
                                option
                            );
                            return 2;
                        }
                    }
                } else {
                    let so = |name: &str, default: bool| -> bool {
                        shell.shopt_options.get(name).copied().unwrap_or(default)
                    };
                    let options: Vec<(&str, bool)> = vec![
                        ("allexport", shell.opt_allexport),
                        ("braceexpand", so("braceexpand", true)),
                        ("emacs", so("emacs", false)),
                        ("errexit", shell.opt_errexit),
                        ("errtrace", so("errtrace", false)),
                        ("functrace", so("functrace", false)),
                        ("hashall", shell.opt_hashall),
                        ("histexpand", so("histexpand", false)),
                        ("history", so("history", false)),
                        ("ignoreeof", so("ignoreeof", false)),
                        ("interactive-comments", so("interactive-comments", true)),
                        ("keyword", shell.opt_keyword),
                        ("monitor", so("monitor", false)),
                        ("noclobber", shell.opt_noclobber),
                        ("noexec", shell.opt_noexec),
                        ("noglob", shell.opt_noglob),
                        ("nolog", so("nolog", false)),
                        ("notify", so("notify", false)),
                        ("nounset", shell.opt_nounset),
                        ("onecmd", so("onecmd", false)),
                        ("physical", so("physical", false)),
                        ("pipefail", shell.opt_pipefail),
                        ("posix", shell.opt_posix),
                        ("privileged", so("privileged", false)),
                        ("verbose", so("verbose", false)),
                        ("vi", so("vi", false)),
                        ("xtrace", shell.opt_xtrace),
                    ];
                    if enable {
                        // set -o: human-readable option listing
                        for (name, val) in &options {
                            println!("{:<15}\t{}", name, if *val { "on" } else { "off" });
                        }
                    } else {
                        // set +o: reusable format
                        for (name, val) in &options {
                            println!("set {}o {}", if *val { "-" } else { "+" }, name);
                        }
                    }
                }
            } else {
                for flag in flags.chars() {
                    match flag {
                        'e' => shell.opt_errexit = enable,
                        'u' => shell.opt_nounset = enable,
                        'x' => shell.opt_xtrace = enable,
                        'f' => shell.opt_noglob = enable,
                        'k' => shell.opt_keyword = enable,
                        'C' => shell.opt_noclobber = enable,
                        'n' => shell.opt_noexec = enable,
                        'h' => shell.opt_hashall = enable,
                        'm' => shell.opt_monitor = enable,
                        'a' | 'b' | 'p' | 't' | 'v' | 'B' | 'E' | 'H' | 'P' | 'T' => {
                            // Known but not fully implemented flags — accept silently
                        }
                        _ => {
                            eprintln!("{}: set: -{}: invalid option", shell.error_prefix(), flag);
                            eprintln!(
                                "set: usage: set [-abefhkmnptuvxBCEHPT] [-o option-name] [--] [-] [arg ...]"
                            );
                            return 2;
                        }
                    }
                }
            }
        } else {
            // Set positional parameters
            let new_positional: Vec<String> = args[i..].to_vec();
            let prog = shell.positional.first().cloned().unwrap_or_default();
            shell.positional = vec![prog];
            shell.positional.extend(new_positional);
            return 0;
        }
        i += 1;
    }
    shell.update_shellopts();
    0
}

fn builtin_shift(shell: &mut Shell, args: &[String]) -> i32 {
    // Skip -- if present
    let args: Vec<&String> = if args.first().map(|s| s.as_str()) == Some("--") {
        args[1..].iter().collect()
    } else {
        args.iter().collect()
    };
    if args.len() > 1 {
        eprintln!("{}: shift: too many arguments", shell.error_prefix());
        return 1;
    }
    let n: i64 = if let Some(s) = args.first() {
        match s.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "{}: shift: {}: numeric argument required",
                    shell.error_prefix(),
                    s
                );
                return 1;
            }
        }
    } else {
        1
    };
    if n < 0 {
        eprintln!(
            "{}: shift: {}: shift count out of range",
            shell.error_prefix(),
            n
        );
        return 1;
    }
    let n = n as usize;
    if shell.positional.len() > 1 {
        let available = shell.positional.len() - 1;
        if n > available {
            // Only print error if shift_verbose is enabled
            if shell
                .shopt_options
                .get("shift_verbose")
                .copied()
                .unwrap_or(false)
            {
                eprintln!(
                    "{}: shift: {}: shift count out of range",
                    shell.error_prefix(),
                    n
                );
            }
            return 1;
        }
        shell.positional.drain(1..=n);
    } else if n > 0 {
        // No positional params to shift
        if shell
            .shopt_options
            .get("shift_verbose")
            .copied()
            .unwrap_or(false)
        {
            eprintln!(
                "{}: shift: {}: shift count out of range",
                shell.error_prefix(),
                n
            );
        }
        return 1;
    }
    0
}

fn builtin_exit(shell: &mut Shell, args: &[String]) -> i32 {
    let code: i32 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(shell.last_status);
    shell.last_status = code;
    shell.run_exit_trap();
    std::io::Write::flush(&mut std::io::stdout()).ok();
    std::io::Write::flush(&mut std::io::stderr()).ok();
    std::process::exit(code);
}

fn builtin_return(shell: &mut Shell, args: &[String]) -> i32 {
    let has_explicit_arg = args.first().and_then(|s| s.parse::<i32>().ok()).is_some();
    let code: i32 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(shell.last_status);
    // return is valid in functions, sourced scripts, and trap handlers
    if shell.local_scopes.is_empty() && !shell.sourcing && shell.in_trap_handler == 0 {
        eprintln!(
            "{}: return: can only `return' from a function or sourced script",
            shell.error_prefix()
        );
        return 1;
    }
    shell.returning = true;
    shell.return_explicit_arg = has_explicit_arg;
    code
}

fn builtin_true(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

fn builtin_false(_shell: &mut Shell, _args: &[String]) -> i32 {
    1
}

fn builtin_test(shell: &mut Shell, args: &[String]) -> i32 {
    eval_test_expr(args, shell, "test", false)
}

fn builtin_test_bracket(shell: &mut Shell, args: &[String]) -> i32 {
    // Remove trailing ]
    let args = if args.last().map(|s| s.as_str()) == Some("]") {
        &args[..args.len() - 1]
    } else {
        eprintln!("{}: [: missing `]'", shell.error_prefix());
        return 2;
    };
    eval_test_expr(args, shell, "[", false)
}

/// Helper: format test error for [ command, appending ", found ]" when appropriate
fn test_paren_error(shell: &Shell, cmd_name: &str) {
    if cmd_name == "[" {
        eprintln!(
            "{}: {}: `)' expected, found ]",
            shell.error_prefix(),
            cmd_name
        );
    } else {
        eprintln!("{}: {}: `)' expected", shell.error_prefix(), cmd_name);
    }
}

fn eval_test_expr(args: &[String], shell: &Shell, cmd_name: &str, sub_expr: bool) -> i32 {
    if args.is_empty() {
        return 1; // Empty test is false
    }

    if args.len() == 1 {
        // Single arg: true if non-empty
        return if args[0].is_empty() { 1 } else { 0 };
    }

    if args.len() == 2 {
        match args[0].as_str() {
            "!" => {
                return if eval_test_expr(&args[1..], shell, cmd_name, true) == 0 {
                    1
                } else {
                    0
                };
            }
            "-n" => return if !args[1].is_empty() { 0 } else { 1 },
            "-z" => return if args[1].is_empty() { 0 } else { 1 },
            "-v" => {
                let name = &args[1];
                let is_set =
                    if let Some(bracket) = name.find('[') {
                        let base = &name[..bracket];
                        let idx = &name[bracket + 1..name.len() - 1];
                        if idx == "@" || idx == "*" {
                            shell.arrays.contains_key(base) || shell.assoc_arrays.contains_key(base)
                        } else {
                            shell.arrays.get(base).is_some_and(|a| {
                                idx.parse::<usize>().ok().is_some_and(|n| n < a.len())
                            }) || shell
                                .assoc_arrays
                                .get(base)
                                .is_some_and(|a| a.get(idx).is_some())
                        }
                    } else {
                        shell.vars.contains_key(name.as_str())
                            || shell.arrays.contains_key(name.as_str())
                            || shell.assoc_arrays.contains_key(name.as_str())
                    };
                return if is_set { 0 } else { 1 };
            }
            "-e" | "-a" => {
                return if !args[1].is_empty() && std::path::Path::new(&args[1]).exists() {
                    0
                } else {
                    1
                };
            }
            "-f" => {
                return if !args[1].is_empty() && std::path::Path::new(&args[1]).is_file() {
                    0
                } else {
                    1
                };
            }
            "-d" => {
                return if !args[1].is_empty() && std::path::Path::new(&args[1]).is_dir() {
                    0
                } else {
                    1
                };
            }
            "-L" | "-h" => {
                return if !args[1].is_empty()
                    && std::fs::symlink_metadata(&args[1])
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    0
                } else {
                    1
                };
            }
            "-N" => {
                // File exists and has been modified since last read
                // In a simplified implementation, check if mtime > atime
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    return if !args[1].is_empty()
                        && std::fs::metadata(&args[1]).is_ok_and(|m| m.mtime() > m.atime())
                    {
                        0
                    } else {
                        1
                    };
                }
                #[cfg(not(unix))]
                {
                    return 1;
                }
            }
            #[cfg(unix)]
            "-r" => {
                return if !args[1].is_empty()
                    && nix::unistd::access(args[1].as_str(), nix::unistd::AccessFlags::R_OK).is_ok()
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-w" => {
                return if !args[1].is_empty()
                    && nix::unistd::access(args[1].as_str(), nix::unistd::AccessFlags::W_OK).is_ok()
                {
                    0
                } else {
                    1
                };
            }
            "-x" => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    return if !args[1].is_empty()
                        && std::fs::metadata(&args[1])
                            .map(|m| m.permissions().mode() & 0o111 != 0)
                            .unwrap_or(false)
                    {
                        0
                    } else {
                        1
                    };
                }
                #[cfg(not(unix))]
                return 1;
            }
            "-s" => {
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .map(|m| m.len() > 0)
                        .unwrap_or(false)
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-c" => {
                use std::os::unix::fs::FileTypeExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1]).is_ok_and(|m| m.file_type().is_char_device())
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-b" => {
                use std::os::unix::fs::FileTypeExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1]).is_ok_and(|m| m.file_type().is_block_device())
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-p" => {
                use std::os::unix::fs::FileTypeExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1]).is_ok_and(|m| m.file_type().is_fifo())
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-S" => {
                use std::os::unix::fs::FileTypeExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1]).is_ok_and(|m| m.file_type().is_socket())
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-u" => {
                use std::os::unix::fs::PermissionsExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .is_ok_and(|m| m.permissions().mode() & 0o4000 != 0)
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-g" => {
                use std::os::unix::fs::PermissionsExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .is_ok_and(|m| m.permissions().mode() & 0o2000 != 0)
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-k" => {
                use std::os::unix::fs::PermissionsExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .is_ok_and(|m| m.permissions().mode() & 0o1000 != 0)
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-O" => {
                use std::os::unix::fs::MetadataExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .is_ok_and(|m| m.uid() == unsafe { libc::getuid() })
                {
                    0
                } else {
                    1
                };
            }
            #[cfg(unix)]
            "-G" => {
                use std::os::unix::fs::MetadataExt;
                return if !args[1].is_empty()
                    && std::fs::metadata(&args[1])
                        .is_ok_and(|m| m.gid() == unsafe { libc::getgid() })
                {
                    0
                } else {
                    1
                };
            }
            "-t" => {
                let fd: i32 = match args[1].parse() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!(
                            "{}: {}: {}: integer expected",
                            shell.error_prefix(),
                            cmd_name,
                            args[1]
                        );
                        return 2;
                    }
                };
                #[cfg(unix)]
                {
                    return if nix::unistd::isatty(fd).unwrap_or(false) {
                        0
                    } else {
                        1
                    };
                }
                #[cfg(not(unix))]
                {
                    let _ = fd;
                    return 1;
                }
            }
            "-o" => {
                // Shell option test
                let opt = &args[1];
                let is_set = match opt.as_str() {
                    "errexit" => shell.opt_errexit,
                    "nounset" => shell.opt_nounset,
                    "xtrace" => shell.opt_xtrace,
                    "noclobber" => shell.opt_noclobber,
                    "noglob" => shell.opt_noglob,
                    "noexec" => shell.opt_noexec,
                    "posix" => shell.opt_posix,
                    "pipefail" => shell.opt_pipefail,
                    _ => false,
                };
                return if is_set { 0 } else { 1 };
            }
            "-R" => {
                // Nameref test
                return if shell.namerefs.contains_key(args[1].as_str()) {
                    0
                } else {
                    1
                };
            }
            op if op.starts_with('-') => {
                // Unknown unary operator
                eprintln!(
                    "{}: {}: {}: unary operator expected",
                    shell.error_prefix(),
                    cmd_name,
                    op
                );
                return 2;
            }
            _ => {
                if args[0] == "(" && sub_expr {
                    // ( ) as sub-expression — paren error
                    test_paren_error(shell, cmd_name);
                } else {
                    // Two args, first is not a known unary operator
                    eprintln!(
                        "{}: {}: {}: unary operator expected",
                        shell.error_prefix(),
                        cmd_name,
                        args[0]
                    );
                }
                return 2;
            }
        }
    }

    if args.len() == 3 {
        // Handle ( expr ) grouping
        if args[0] == "(" && args[2] == ")" {
            return eval_test_expr(&args[1..2], shell, cmd_name, true);
        }
        match args[1].as_str() {
            "=" | "==" => return if args[0] == args[2] { 0 } else { 1 },
            "!=" => return if args[0] != args[2] { 0 } else { 1 },
            "<" => return if args[0] < args[2] { 0 } else { 1 },
            ">" => return if args[0] > args[2] { 0 } else { 1 },
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                let prefix = shell.error_prefix();
                let a = match args[0].parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("{}: {}: {}: integer expected", prefix, cmd_name, args[0]);
                        return 2;
                    }
                };
                let b = match args[2].parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("{}: {}: {}: integer expected", prefix, cmd_name, args[2]);
                        return 2;
                    }
                };
                return match args[1].as_str() {
                    "-eq" => {
                        if a == b {
                            0
                        } else {
                            1
                        }
                    }
                    "-ne" => {
                        if a != b {
                            0
                        } else {
                            1
                        }
                    }
                    "-lt" => {
                        if a < b {
                            0
                        } else {
                            1
                        }
                    }
                    "-le" => {
                        if a <= b {
                            0
                        } else {
                            1
                        }
                    }
                    "-gt" => {
                        if a > b {
                            0
                        } else {
                            1
                        }
                    }
                    "-ge" => {
                        if a >= b {
                            0
                        } else {
                            1
                        }
                    }
                    _ => unreachable!(),
                };
            }
            "-nt" => {
                // Newer than — existing is newer than non-existent
                let a_exists = std::path::Path::new(&args[0]).exists();
                let b_exists = std::path::Path::new(&args[2]).exists();
                let a = std::fs::metadata(&args[0]).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(&args[2]).and_then(|m| m.modified()).ok();
                return match (a, b) {
                    (Some(a), Some(b)) => {
                        if a > b {
                            0
                        } else {
                            1
                        }
                    }
                    (Some(_), None) if a_exists && !b_exists => 0,
                    _ => 1,
                };
            }
            "-ot" => {
                let a_exists = std::path::Path::new(&args[0]).exists();
                let b_exists = std::path::Path::new(&args[2]).exists();
                let a = std::fs::metadata(&args[0]).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(&args[2]).and_then(|m| m.modified()).ok();
                return match (a, b) {
                    (Some(a), Some(b)) => {
                        if a < b {
                            0
                        } else {
                            1
                        }
                    }
                    // Non-existent file is older than existing
                    (None, Some(_)) if !a_exists && b_exists => 0,
                    _ => 1,
                };
            }
            #[cfg(unix)]
            "-ef" => {
                use std::os::unix::fs::MetadataExt;
                let a = std::fs::metadata(&args[0]).ok();
                let b = std::fs::metadata(&args[2]).ok();
                return match (a, b) {
                    (Some(a), Some(b)) => {
                        if a.dev() == b.dev() && a.ino() == b.ino() {
                            0
                        } else {
                            1
                        }
                    }
                    _ => 1,
                };
            }
            // -a and -o as binary (AND/OR) — fall through to general handler
            "-a" | "-o" => {}
            _ => {
                // 3 args: middle arg is not a valid binary operator
                if args[0] == "!" {
                    // ! expr — fall through to general handler
                } else if args[0] == "(" {
                    // ( X — missing )
                    test_paren_error(shell, cmd_name);
                    return 2;
                } else {
                    eprintln!(
                        "{}: {}: {}: binary operator expected",
                        shell.error_prefix(),
                        cmd_name,
                        args[1]
                    );
                    return 2;
                }
            }
        }
    }

    // Handle parenthesized grouping: ( expr )
    if args.first().map(|s| s.as_str()) == Some("(") {
        // Find matching close paren, handling nesting
        let mut depth = 0;
        let mut close = None;
        for (i, arg) in args.iter().enumerate() {
            if arg == "(" {
                depth += 1;
            } else if arg == ")" {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
        }
        if let Some(close_idx) = close {
            if close_idx == args.len() - 1 {
                // All args are inside parens
                return eval_test_expr(&args[1..close_idx], shell, cmd_name, true);
            }
            // Parens with stuff after — continue processing
        } else {
            // Missing closing )
            test_paren_error(shell, cmd_name);
            return 2;
        }
    }

    // For many-arg expressions, detect structural errors before splitting on -a/-o
    if !sub_expr && args.len() >= 4 {
        // Flatten args outside parens and check for structural issues
        let mut depth = 0;
        // Check for trailing -a/-o with no right operand (at top paren level)
        for (i, arg) in args.iter().enumerate() {
            if arg == "(" {
                depth += 1;
            } else if arg == ")" {
                depth -= 1;
            } else if depth == 0 {
                // trailing -a/-o
                if (arg == "-a" || arg == "-o") && i == args.len() - 1 {
                    eprintln!("{}: {}: argument expected", shell.error_prefix(), cmd_name);
                    return 2;
                }
                // Check for value followed by binary op at end (e.g. `4 -ne` with nothing after)
                if i + 2 == args.len()
                    && !arg.starts_with('-')
                    && arg != "("
                    && arg != ")"
                    && is_test_binop(&args[i + 1])
                {
                    eprintln!(
                        "{}: {}: syntax error: `{}' unexpected",
                        shell.error_prefix(),
                        cmd_name,
                        args[i + 1]
                    );
                    return 2;
                }
                // Check for unary op at end with no argument after a complete expression
                if i + 1 == args.len()
                    && is_test_unop(arg)
                    && i >= 2
                    && !is_test_binop(&args[i - 1])
                    && args[i - 1] != "-a"
                    && args[i - 1] != "-o"
                {
                    eprintln!(
                        "{}: {}: syntax error: `{}' unexpected",
                        shell.error_prefix(),
                        cmd_name,
                        arg
                    );
                    return 2;
                }
                // Adjacent non-operators after -a/-o (e.g. `-a 3 4`)
                if i >= 1
                    && (args[i - 1] == "-a" || args[i - 1] == "-o")
                    && !arg.starts_with('-')
                    && arg != "("
                    && arg != ")"
                    && i + 1 < args.len()
                    && !is_test_binop(&args[i + 1])
                    && args[i + 1] != "-a"
                    && args[i + 1] != "-o"
                    && args[i + 1] != ")"
                    && !args[i + 1].starts_with('-')
                {
                    eprintln!("{}: {}: too many arguments", shell.error_prefix(), cmd_name);
                    return 2;
                }
            }
        }
    }

    // Handle -a (and) and -o (or), skipping inside parentheses
    {
        let mut depth = 0;
        for (i, arg) in args.iter().enumerate() {
            if arg == "(" {
                depth += 1;
            } else if arg == ")" {
                depth -= 1;
            } else if arg == "-a" && depth == 0 && i > 0 && i < args.len() - 1 {
                let left = eval_test_expr(&args[..i], shell, cmd_name, true);
                if left == 2 {
                    return 2;
                }
                let right = eval_test_expr(&args[i + 1..], shell, cmd_name, true);
                if right == 2 {
                    return 2;
                }
                return if left == 0 && right == 0 { 0 } else { 1 };
            }
        }
    }
    {
        let mut depth = 0;
        for (i, arg) in args.iter().enumerate() {
            if arg == "(" {
                depth += 1;
            } else if arg == ")" {
                depth -= 1;
            } else if arg == "-o" && depth == 0 && i > 0 && i < args.len() - 1 {
                let left = eval_test_expr(&args[..i], shell, cmd_name, true);
                if left == 2 {
                    return 2;
                }
                let right = eval_test_expr(&args[i + 1..], shell, cmd_name, true);
                if right == 2 {
                    return 2;
                }
                return if left == 0 || right == 0 { 0 } else { 1 };
            }
        }
    }

    // Handle ! prefix with 3+ args
    if args[0] == "!" {
        return if eval_test_expr(&args[1..], shell, cmd_name, true) == 0 {
            1
        } else {
            0
        };
    }

    // For 4+ args that didn't match any pattern: report appropriate error
    if args.len() >= 4 {
        // Check for trailing -a/-o with no right operand
        if args.last().is_some_and(|last| last == "-a" || last == "-o") {
            eprintln!("{}: {}: argument expected", shell.error_prefix(), cmd_name);
            return 2;
        }
        // Check for repeated operators or misplaced tokens
        for i in 0..args.len() {
            let a = &args[i];
            if is_test_binop(a) && i + 1 < args.len() && is_test_binop(&args[i + 1]) {
                eprintln!(
                    "{}: {}: syntax error: `{}' unexpected",
                    shell.error_prefix(),
                    cmd_name,
                    args[i + 1]
                );
                return 2;
            }
            if is_test_unop(a) && i + 1 < args.len() && is_test_binop(&args[i + 1]) {
                eprintln!(
                    "{}: {}: syntax error: `{}' unexpected",
                    shell.error_prefix(),
                    cmd_name,
                    args[i + 1]
                );
                return 2;
            }
        }
        eprintln!("{}: {}: too many arguments", shell.error_prefix(), cmd_name);
        return 2;
    }

    1 // Default: false
}

fn is_test_binop(s: &str) -> bool {
    matches!(
        s,
        "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
            | "-nt"
            | "-ot"
            | "-ef"
            | "="
            | "=="
            | "!="
            | "<"
            | ">"
    )
}

fn is_test_unop(s: &str) -> bool {
    matches!(
        s,
        "-a" | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-k"
            | "-n"
            | "-p"
            | "-r"
            | "-s"
            | "-t"
            | "-u"
            | "-w"
            | "-x"
            | "-z"
            | "-G"
            | "-L"
            | "-N"
            | "-O"
            | "-R"
            | "-S"
            | "-v"
            | "-o"
    )
}

fn builtin_read(shell: &mut Shell, args: &[String]) -> i32 {
    let mut prompt = String::new();
    let mut raw = false;
    let mut var_names = Vec::new();
    let mut array_name: Option<String> = None;
    let mut delim: Option<char> = None;
    let mut nchars: Option<usize> = None;
    let mut fd: Option<i32> = None;
    let mut timeout_secs: Option<f64> = None;
    let mut i = 0;

    while i < args.len() {
        // Handle combined flags like -rd, -rn, etc.
        if args[i].starts_with('-') && args[i].len() > 1 && !args[i].starts_with("--") {
            let flags = &args[i][1..];
            let mut j = 0;
            let fchars: Vec<char> = flags.chars().collect();
            while j < fchars.len() {
                match fchars[j] {
                    'r' => raw = true,
                    's' | 'e' => {}
                    'p' => {
                        // -p takes next arg (or rest of combined flag)
                        i += 1;
                        if i < args.len() {
                            prompt = args[i].clone();
                        }
                        break;
                    }
                    'd' => {
                        // Check if delimiter char follows in same arg (e.g., -d\n)
                        if j + 1 < fchars.len() {
                            delim = Some(fchars[j + 1]);
                        } else {
                            // -d takes next arg
                            i += 1;
                            if i < args.len() {
                                delim = Some(args[i].chars().next().unwrap_or('\0'));
                            }
                        }
                        break;
                    }
                    'a' => {
                        i += 1;
                        if i < args.len() {
                            array_name = Some(args[i].clone());
                        }
                        break;
                    }
                    'n' | 'N' => {
                        i += 1;
                        if i < args.len() {
                            match args[i].parse::<i64>() {
                                Ok(n) if n < 0 => {
                                    eprintln!(
                                        "{}: read: {}: invalid number",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 2;
                                }
                                Ok(n) => nchars = Some(n as usize),
                                Err(_) => {
                                    eprintln!(
                                        "{}: read: {}: invalid number",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 2;
                                }
                            }
                        }
                        break;
                    }
                    'u' => {
                        i += 1;
                        if i < args.len() {
                            match args[i].parse::<i32>() {
                                Ok(f) => fd = Some(f),
                                Err(_) => {
                                    eprintln!(
                                        "{}: read: {}: invalid file descriptor specification",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 1;
                                }
                            }
                        }
                        break;
                    }
                    't' => {
                        i += 1;
                        if i < args.len() {
                            match args[i].parse::<f64>() {
                                Ok(t) => timeout_secs = Some(t),
                                Err(_) => {
                                    eprintln!(
                                        "{}: read: {}: invalid timeout specification",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 1;
                                }
                            }
                        }
                        break;
                    }
                    'E' | 'i' => {} // accepted but not implemented
                    _ => {
                        eprintln!(
                            "{}: read: -{}: invalid option",
                            shell.error_prefix(),
                            fchars[j]
                        );
                        eprintln!(
                            "read: usage: read [-Eers] [-a array] [-d delim] [-i text] [-n nchars] [-N nchars] [-p prompt] [-t timeout] [-u fd] [name ...]"
                        );
                        return 2;
                    }
                }
                j += 1;
            }
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "-r" => raw = true,
            "-s" | "-e" => {}
            "-p" => {
                i += 1;
                if i < args.len() {
                    prompt = args[i].clone();
                }
            }
            "-d" => {
                i += 1;
                if i < args.len() {
                    delim = Some(args[i].chars().next().unwrap_or('\0'));
                }
            }
            "-a" => {
                i += 1;
                if i < args.len() {
                    array_name = Some(args[i].clone());
                }
            }
            "-n" | "-N" => {
                i += 1;
                if i < args.len() {
                    match args[i].parse::<isize>() {
                        Ok(n) if n >= 0 => nchars = Some(n as usize),
                        Ok(_) => {
                            eprintln!(
                                "{}: read: {}: invalid number",
                                shell.error_prefix(),
                                args[i]
                            );
                            return 2;
                        }
                        Err(_) => nchars = Some(0),
                    }
                }
            }
            "-t" => {
                i += 1;
                if i < args.len() {
                    timeout_secs = args[i].parse().ok();
                }
            }
            "-u" => {
                i += 1;
                if i < args.len() {
                    fd = args[i].parse().ok();
                }
            }
            arg if !arg.starts_with('-') => {
                // Validate identifier
                if !arg.chars().all(|c| c.is_alphanumeric() || c == '_')
                    || arg.chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    eprintln!(
                        "{}: read: `{}': not a valid identifier",
                        shell.error_prefix(),
                        arg
                    );
                    return 1;
                }
                var_names.push(arg.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    // Validate array name
    if let Some(ref name) = array_name
        && !is_valid_identifier(name)
    {
        eprintln!(
            "{}: read: `{}': not a valid identifier",
            shell.error_prefix(),
            name
        );
        return 1;
    }

    // Validate variable names
    for name in &var_names {
        if !is_valid_identifier(name) {
            eprintln!(
                "{}: read: `{}': not a valid identifier",
                shell.error_prefix(),
                name
            );
            return 1;
        }
    }

    // Readonly checks happen during assignment, not here
    // (bash reads input first, then errors on readonly during assignment)

    let is_reply = var_names.is_empty() && array_name.is_none();
    if is_reply {
        var_names.push("REPLY".to_string());
    }

    if !prompt.is_empty() {
        eprint!("{}", prompt);
    }

    // Validate fd if specified
    #[cfg(unix)]
    if let Some(f) = fd
        && nix::fcntl::fcntl(f, nix::fcntl::FcntlArg::F_GETFD).is_err()
    {
        eprintln!(
            "{}: read: {}: invalid file descriptor: Bad file descriptor",
            shell.error_prefix(),
            f
        );
        return 1;
    }

    let mut line = String::new();
    let mut eof_reached = false;

    // Determine which fd to read from
    let read_fd = fd.unwrap_or(0); // 0 = stdin

    // Handle timeout: check if data is available within the timeout period
    #[cfg(unix)]
    if let Some(secs) = timeout_secs {
        use nix::poll::{PollFd, PollFlags, PollTimeout};
        use std::os::unix::io::BorrowedFd;
        let poll_fd = PollFd::new(
            unsafe { BorrowedFd::borrow_raw(read_fd) },
            PollFlags::POLLIN,
        );
        let timeout_ms = (secs * 1000.0) as i32;
        let is_poll = timeout_ms <= 0; // -t 0 is just a polling check
        let timeout = if is_poll {
            PollTimeout::ZERO
        } else {
            PollTimeout::from(timeout_ms as u16)
        };
        match nix::poll::poll(&mut [poll_fd], timeout) {
            Ok(0) => {
                if is_poll {
                    return 1; // polling: no data available
                }
                return 142; // timeout — exit code > 128
            }
            Err(_) => return if is_poll { 1 } else { 142 },
            _ => {
                if is_poll {
                    // Check for POLLNVAL (closed/invalid fd) or POLLERR
                    let revents = poll_fd.revents().unwrap_or(PollFlags::empty());
                    if revents.intersects(PollFlags::POLLNVAL | PollFlags::POLLERR) {
                        return 1; // invalid fd
                    }
                    return 0; // polling: data available (or EOF)
                }
            }
        }
    }

    // Read input based on options
    if let Some(n) = nchars {
        if n == 0 {
            // read -n 0: just test if fd is valid
            // (returns 0 if fd is valid, 1 otherwise)
            #[cfg(unix)]
            {
                use nix::fcntl::{FcntlArg, fcntl};
                match fcntl(read_fd, FcntlArg::F_GETFD) {
                    Ok(_) => {} // fd is valid, continue to assign empty
                    Err(_) => return 1,
                }
            }
        } else {
            // Read exactly n characters
            #[cfg(unix)]
            {
                let mut buf = vec![0u8; n];
                match nix::unistd::read(read_fd, &mut buf) {
                    Ok(0) => eof_reached = true,
                    Ok(bytes_read) => {
                        // Convert raw bytes to chars (Latin-1 mapping for 0x80-0xFF)
                        line = buf[..bytes_read].iter().map(|&b| b as char).collect();
                    }
                    Err(_) => return 1,
                }
            }
            #[cfg(not(unix))]
            {
                use std::io::Read as _;
                let mut buf = vec![0u8; n];
                match std::io::stdin().read(&mut buf) {
                    Ok(0) => return 1,
                    Ok(bytes_read) => {
                        line = buf[..bytes_read].iter().map(|&b| b as char).collect();
                    }
                    Err(_) => return 1,
                }
            }
        } // close the else block for n > 0
    } else if let Some(delim_char) = delim {
        // Read until delimiter character (byte by byte)
        #[cfg(unix)]
        {
            let mut buf = [0u8; 1];
            let mut hit_eof = false;
            loop {
                match nix::unistd::read(read_fd, &mut buf) {
                    Ok(0) => {
                        hit_eof = true;
                        break;
                    }
                    Ok(_) => {
                        let ch = buf[0] as char;
                        if ch == delim_char {
                            break;
                        }
                        line.push(ch);
                    }
                    Err(_) => {
                        hit_eof = true;
                        break;
                    }
                }
            }
            if hit_eof {
                eof_reached = true;
                // Don't return early — still need to assign variables
                // (and check readonly) even on EOF with empty data
            }
        }
        #[cfg(not(unix))]
        {
            use std::io::Read as _;
            let mut buf = [0u8; 1];
            loop {
                match std::io::stdin().read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {
                        let ch = buf[0] as char;
                        if ch == delim_char {
                            break;
                        }
                        line.push(ch);
                    }
                    Err(_) => break,
                }
            }
        }
    } else if fd.is_some() {
        // Read a line from a specific file descriptor (byte-by-byte to avoid buffering issues)
        #[cfg(unix)]
        {
            let mut buf = [0u8; 1];
            loop {
                match nix::unistd::read(read_fd, &mut buf) {
                    Ok(0) => {
                        eof_reached = true;
                        break;
                    }
                    Ok(_) => {
                        let ch = buf[0] as char;
                        if ch == '\n' {
                            break;
                        }
                        line.push(ch);
                    }
                    Err(_) => {
                        eof_reached = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            use std::io::Read as _;
            let mut buf = [0u8; 1];
            loop {
                match std::io::stdin().read(&mut buf) {
                    Ok(0) => {
                        eof_reached = true;
                        break;
                    }
                    Ok(_) => {
                        let ch = buf[0] as char;
                        if ch == '\n' {
                            break;
                        }
                        line.push(ch);
                    }
                    Err(_) => {
                        eof_reached = true;
                        break;
                    }
                }
            }
        }
    } else {
        // Read byte-by-byte from fd to avoid buffering issues
        // (Rust's stdin() has a shared buffer that breaks when fd 0 is redirected)
        #[cfg(unix)]
        {
            let mut buf = [0u8; 1];
            loop {
                match nix::unistd::read(read_fd, &mut buf) {
                    Ok(0) => {
                        eof_reached = true;
                        break;
                    }
                    Ok(_) => {
                        let ch = buf[0] as char;
                        if ch == '\n' {
                            // In non-raw mode, backslash-newline is line continuation
                            if !raw && line.ends_with('\\') {
                                line.pop(); // remove the backslash
                                continue; // read next line
                            }
                            break;
                        }
                        line.push(ch);
                    }
                    Err(_) => {
                        eof_reached = true;
                        break;
                    }
                }
            }
        }
        #[cfg(not(unix))]
        {
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => return 1,
                Err(_) => return 1,
                _ => {}
            }
        }
    }

    // Remove trailing newline (but not when -d is used with non-newline delimiter)
    if delim.is_none() || delim == Some('\n') {
        if line.ends_with('\n') {
            line.pop();
        }
        if line.ends_with('\r') {
            line.pop();
        }
    }

    if !raw {
        // Handle backslash line continuation only here
        // Backslash before IFS chars is handled during field splitting below
        line = line.replace("\\\n", "");
        // Remove trailing backslash (continuation at EOF)
        if line.ends_with('\\') {
            line.pop();
        }
    }

    let ifs = shell
        .vars
        .get("IFS")
        .cloned()
        .unwrap_or_else(|| " \t\n".to_string());

    // When no variable names given (reading into REPLY), store raw line without IFS processing
    if is_reply {
        shell.set_var("REPLY", line);
        return 0;
    }

    // Handle -a: read into array
    if let Some(arr_name) = array_name {
        let ifs_ws_chars: Vec<char> = ifs.chars().filter(|c| c.is_whitespace()).collect();
        let ifs_non_ws: String = ifs.chars().filter(|c| !c.is_whitespace()).collect();

        // Strip leading IFS whitespace
        let trimmed = line.trim_start_matches(|c: char| ifs_ws_chars.contains(&c));

        let mut fields: Vec<String> = if !ifs_non_ws.is_empty() {
            trimmed
                .split(|c: char| ifs.contains(c))
                .map(|s| s.to_string())
                .collect()
        } else {
            trimmed
                .split(|c: char| ifs.contains(c))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        };
        // Remove trailing empty field produced by trailing IFS delimiter
        if fields.last().is_some_and(|s| s.is_empty()) && !ifs_non_ws.is_empty() {
            fields.pop();
        }
        // Also strip trailing IFS whitespace from last field
        if let Some(last) = fields.last_mut() {
            let new_last = last.trim_end_matches(|c: char| ifs_ws_chars.contains(&c));
            *last = new_last.to_string();
        }
        shell.arrays.insert(arr_name, fields);
        return 0;
    }

    // Split the line into fields respecting backslash escapes (if not raw)
    let ifs_ws: Vec<char> = ifs.chars().filter(|c| c.is_whitespace()).collect();

    // Parse the line into fields
    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut last_escaped_pos: Option<usize> = None; // track last escaped char position in current
    let chars: Vec<char> = line.chars().collect();
    let mut ci = 0;
    let max_fields = var_names.len();

    // Skip leading IFS whitespace
    while ci < chars.len() && ifs_ws.contains(&chars[ci]) {
        ci += 1;
    }

    while ci < chars.len() {
        let ch = chars[ci];
        if !raw && ch == '\\' && ci + 1 < chars.len() {
            // Backslash escapes the next character
            ci += 1;
            current.push(chars[ci]);
            last_escaped_pos = Some(current.len() - 1);
            ci += 1;
        } else if fields.len() < max_fields - 1 && ifs.contains(ch) {
            // IFS character — end current field
            if ifs_ws.contains(&ch) {
                // IFS whitespace: skip consecutive whitespace
                if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                    last_escaped_pos = None;
                }
                while ci + 1 < chars.len() && ifs_ws.contains(&chars[ci + 1]) {
                    ci += 1;
                }
            } else {
                // IFS non-whitespace: always produces a field boundary
                fields.push(std::mem::take(&mut current));
                last_escaped_pos = None;
            }
            ci += 1;
        } else {
            current.push(ch);
            ci += 1;
        }
    }
    // Strip trailing IFS characters from last field
    // For single variable: strip all trailing IFS chars (even escaped)
    // For multiple variables: strip trailing IFS whitespace, then trailing IFS non-ws
    let trim_limit = if var_names.len() == 1 {
        0
    } else {
        last_escaped_pos.map(|p| p + 1).unwrap_or(0)
    };
    let mut end = current.len();
    // First strip trailing IFS whitespace
    while end > trim_limit {
        if let Some(c) = current[..end].chars().last() {
            if ifs_ws.contains(&c) {
                end -= c.len_utf8();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    // Then strip trailing non-whitespace IFS delimiters from the last field
    // (when there are multiple variables and this is the remainder)
    if var_names.len() > 1 {
        while end > trim_limit {
            if let Some(c) = current[..end].chars().last() {
                if ifs.contains(c) && !ifs_ws.contains(&c) {
                    end -= c.len_utf8();
                    // Also strip IFS whitespace before the non-ws delimiter
                    while end > trim_limit {
                        if let Some(c2) = current[..end].chars().last() {
                            if ifs_ws.contains(&c2) {
                                end -= c2.len_utf8();
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }
    fields.push(current[..end].to_string());

    // Assign to variables
    let mut read_status = if eof_reached { 1 } else { 0 };
    for (j, name) in var_names.iter().enumerate() {
        let value = fields.get(j).cloned().unwrap_or_default();
        if shell.readonly_vars.contains(name.as_str())
            || shell.readonly_vars.contains(&shell.resolve_nameref(name))
        {
            let resolved = shell.resolve_nameref(name);
            eprintln!("{}: {}: readonly variable", shell.error_prefix(), resolved);
            if !eof_reached {
                read_status = 2;
            }
            break; // bash stops assigning after readonly error
        }
        shell.set_var(name, value);
    }

    read_status
}

fn builtin_eval(shell: &mut Shell, args: &[String]) -> i32 {
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
                        .vars
                        .get("LINENO")
                        .and_then(|s| s.parse::<usize>().ok())
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
                let eval_line = parser.current_line();
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

fn builtin_exec(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    // Parse exec flags: -a NAME (set argv[0]), -c (clear env), -l (login shell)
    let mut argv0_override: Option<String> = None;
    let mut clear_env = false;
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
                // Login shell — prefix argv[0] with -
                // Will be applied below
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

    // Set up environment
    if clear_env {
        for (key, _) in std::env::vars() {
            unsafe { std::env::remove_var(&key) };
        }
    }
    for (key, value) in &shell.exports {
        unsafe { std::env::set_var(key, value) };
    }

    #[cfg(unix)]
    {
        use std::ffi::CString;

        let path = find_executable(program);
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

fn builtin_source(shell: &mut Shell, args: &[String]) -> i32 {
    let cmd = shell.current_builtin.as_deref().unwrap_or(".");
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

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Save and set positional parameters for the sourced script
            let saved_positional = shell.positional.clone();
            if args.len() > 1 {
                let prog = shell.positional.first().cloned().unwrap_or_default();
                shell.positional = vec![prog];
                shell.positional.extend(args[1..].to_vec());
            }

            // Push source file onto BASH_SOURCE stack
            let bash_source = shell.arrays.entry("BASH_SOURCE".to_string()).or_default();
            bash_source.insert(0, path.clone());

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

            shell.positional = saved_positional;
            result
        }
        Err(e) => {
            let msg = io_error_message(&e);
            eprintln!("{}: {}: {}", shell.error_prefix(), filename, msg);
            1
        }
    }
}

fn builtin_type(shell: &mut Shell, args: &[String]) -> i32 {
    let builtin_map = builtins();
    let mut status = 0;
    let mut flag_t = false;
    let mut flag_p = false;
    let mut names = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-t" => flag_t = true,
            "-p" | "-P" => flag_p = true,
            "-a" | "-f" => {}
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
            } else if shell.functions.contains_key(name) {
                println!("function");
            } else if builtin_map.contains_key(name) {
                println!("builtin");
            } else if find_in_path_opt(name).is_some() {
                println!("file");
            }
            // If not found, print nothing and set status
            else {
                status = 1;
            }
        } else if flag_p {
            // Print path only for external commands
            if let Some(path) = find_in_path_opt(name) {
                println!("{}", path);
            } else {
                status = 1;
            }
        } else {
            // Default behavior
            if shell.shopt_expand_aliases
                && let Some(alias_val) = shell.aliases.get(name)
            {
                println!("{} is aliased to `{}'", name, alias_val);
            } else if is_keyword {
                println!("{} is a shell keyword", name);
            } else if let Some(body) = shell.functions.get(name) {
                println!("{} is a function", name);
                println!("{} () \n{}", name, format_compound_command(body));
            } else if builtin_map.contains_key(name) {
                println!("{} is a shell builtin", name);
            } else if let Some(path) = find_in_path_opt(name) {
                println!("{} is {}", name, path);
            } else {
                eprintln!("{}: type: {}: not found", shell.error_prefix(), name);
                status = 1;
            }
        }
    }
    status
}

fn builtin_builtin(shell: &mut Shell, args: &[String]) -> i32 {
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

fn builtin_command(shell: &mut Shell, args: &[String]) -> i32 {
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
                } else if let Some(value) = shell.aliases.get(name.as_str()) {
                    println!("{} is aliased to `{}'", name, value);
                } else if let Some(func_body) = shell.functions.get(name.as_str()) {
                    println!("{} is a function", name);
                    let body = format_compound_command_indent(func_body, 0);
                    println!("{} () \n{}", name, body);
                } else if builtin_map.contains_key(name.as_str()) {
                    println!("{} is a shell builtin", name);
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
            if is_keyword || shell.functions.contains_key(name.as_str()) {
                println!("{}", name);
            } else if shell.aliases.contains_key(name.as_str()) {
                let val = &shell.aliases[name.as_str()];
                println!("alias {}='{}'", name, val);
            } else if builtin_map.contains_key(name.as_str()) {
                println!("{}", name);
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
            eprintln!("{}: {}: {}", shell.error_prefix(), program, e);
            127
        }
    }
}

fn builtin_which(_shell: &mut Shell, args: &[String]) -> i32 {
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

fn builtin_hash(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print hash table (currently empty since we don't cache)
        if shell.hash_table.is_empty() {
            eprintln!("{}: hash: hash table empty", shell.error_prefix());
        } else {
            println!("hits\tcommand");
            for (name, (path, hits)) in &shell.hash_table {
                println!("   {}\t{}", hits, path);
                let _ = name;
            }
        }
        return 0;
    }

    let mut i = 0;
    let mut status = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-r" => {
                shell.hash_table.clear();
            }
            "-l" => {
                for (name, (path, _)) in &shell.hash_table {
                    println!("builtin hash -p {} {}", path, name);
                }
            }
            "-t" => {
                i += 1;
                while i < args.len() && !args[i].starts_with('-') {
                    if let Some((path, _)) = shell.hash_table.get(&args[i]) {
                        println!("{}", path);
                    } else {
                        eprintln!("{}: hash: {}: not found", shell.error_prefix(), args[i]);
                        status = 1;
                    }
                    i += 1;
                }
                continue;
            }
            "-d" => {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "{}: hash: -d: option requires an argument",
                        shell.error_prefix()
                    );
                    return 1;
                }
                shell.hash_table.remove(&args[i]);
            }
            "-p" => {
                if !shell.opt_hashall {
                    eprintln!("{}: hash: hashing disabled", shell.error_prefix());
                    return 1;
                }
                i += 1;
                if i + 1 < args.len() {
                    let path = args[i].clone();
                    i += 1;
                    let name = args[i].clone();
                    shell.hash_table.insert(name, (path, 0));
                }
            }
            opt if opt.starts_with('-') => {
                eprintln!("{}: hash: {}: invalid option", shell.error_prefix(), opt);
                eprintln!("hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]");
                return 1;
            }
            name => {
                // Look up command and add to hash table
                if let Some(path) = find_command_path(name) {
                    shell.hash_table.insert(name.to_string(), (path, 0));
                } else {
                    eprintln!("{}: hash: {}: not found", shell.error_prefix(), name);
                    status = 1;
                }
            }
        }
        i += 1;
    }
    status
}

fn builtin_trap(shell: &mut Shell, args: &[String]) -> i32 {
    fn signal_number(s: &str) -> i32 {
        let upper = s.to_uppercase();
        let name = upper.strip_prefix("SIG").unwrap_or(&upper);
        match name {
            "EXIT" | "0" => 0,
            "HUP" | "1" => 1,
            "INT" | "2" => 2,
            "QUIT" | "3" => 3,
            "ILL" | "4" => 4,
            "TRAP" | "5" => 5,
            "ABRT" | "6" => 6,
            "BUS" | "7" => 7,
            "FPE" | "8" => 8,
            "KILL" | "9" => 9,
            "USR1" | "10" => 10,
            "SEGV" | "11" => 11,
            "USR2" | "12" => 12,
            "PIPE" | "13" => 13,
            "ALRM" | "14" => 14,
            "TERM" | "15" => 15,
            "CHLD" | "17" => 17,
            "CONT" | "18" => 18,
            "STOP" | "19" => 19,
            "TSTP" | "20" => 20,
            "DEBUG" => 100,
            "ERR" => 101,
            "RETURN" => 102,
            _ => 999,
        }
    }

    // Normalize signal name for display
    fn normalize_signal_name(s: &str) -> String {
        match s {
            "0" | "EXIT" | "exit" => "EXIT".to_string(),
            "ERR" | "err" => "ERR".to_string(),
            "DEBUG" | "debug" => "DEBUG".to_string(),
            "RETURN" | "return" => "RETURN".to_string(),
            _ => {
                let upper = s.to_uppercase();
                let name = upper.strip_prefix("SIG").unwrap_or(&upper);
                format!("SIG{}", name)
            }
        }
    }

    if args.is_empty() {
        // Print current traps in signal number order
        let mut sorted: Vec<_> = shell.traps.iter().collect();
        sorted.sort_by_key(|(sig, _)| signal_number(sig));
        for (signal, handler) in sorted {
            println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
        }
        return 0;
    }

    // Check for invalid options
    for arg in args {
        if arg.starts_with('-')
            && arg.len() > 1
            && !matches!(arg.as_str(), "-p" | "-P" | "-l" | "-L" | "--")
        {
            eprintln!("{}: trap: {}: invalid option", shell.error_prefix(), arg);
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
        if arg == "--" {
            break;
        }
    }

    if args.len() == 1 {
        // trap '' or trap - : list traps or reset
        if args[0] == "-l" || args[0] == "-L" {
            // List signal names (same format as kill -l)
            let signals = list_all_signals();
            for (i, (num, name)) in signals.iter().enumerate() {
                print!("{:2}) {}", num, name);
                if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                    println!();
                } else {
                    print!("\t");
                }
            }
            return 0;
        }
        if args[0] == "-p" {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
    }

    // Handle -p with signal arguments: trap -p SIG1 SIG2 ...
    if args.first().map(|s| s.as_str()) == Some("-p") && args.len() >= 2 {
        // Check for conflicting -P
        if args.iter().any(|a| a == "-P") {
            eprintln!(
                "{}: trap: cannot specify both -p and -P",
                shell.error_prefix()
            );
            return 2;
        }
        let mut status = 0;
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Validate signal name
            if signal_number(&norm) == 999 {
                eprintln!(
                    "{}: trap: {}: invalid signal specification",
                    shell.error_prefix(),
                    sig_arg
                );
                status = 1;
                continue;
            }
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return status;
    }

    // trap [-p|-P] 'handler' signal [signal...]
    let handler_idx = 0;
    let sig_start = 1;

    // Check for conflicting -p and -P
    if args.contains(&"-p".to_string()) && args.contains(&"-P".to_string()) {
        eprintln!(
            "{}: trap: cannot specify both -p and -P",
            shell.error_prefix()
        );
        return 2;
    }

    // Handle -P flag — print just the handler command for specified signals
    if args.first().map(|s| s.as_str()) == Some("-P") {
        if args.len() < 2 {
            eprintln!(
                "{}: trap: -P requires at least one signal name",
                shell.error_prefix()
            );
            return 1;
        }
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("{}", handler);
            }
        }
        return 0;
    }

    // Handle -p flag — print traps for specified signals
    if args.first().map(|s| s.as_str()) == Some("-p") {
        if args.len() < 2 {
            let mut sorted: Vec<_> = shell.traps.iter().collect();
            sorted.sort_by_key(|(sig, _)| signal_number(sig));
            for (signal, handler) in sorted {
                println!("trap -- '{}' {}", handler, normalize_signal_name(signal));
            }
            return 0;
        }
        // trap -p SIG1 SIG2 ... — print traps for specific signals
        for sig_arg in &args[1..] {
            let norm = normalize_signal_name(sig_arg);
            // Traps are stored without SIG prefix, so strip it for lookup
            let lookup = norm.strip_prefix("SIG").unwrap_or(&norm);
            let key = if lookup == "EXIT" {
                shell.traps.get("EXIT").or_else(|| shell.traps.get("0"))
            } else {
                shell.traps.get(lookup).or_else(|| shell.traps.get(&norm))
            };
            if let Some(handler) = key {
                println!("trap -- '{}' {}", handler, norm);
            }
        }
        return 0;
    }

    if args.len() < sig_start + 1 {
        // Handler with no signals specified
        if handler_idx == 0 && args.len() == 1 {
            // Single arg that looks like a signal name → reset it
            let norm = normalize_signal_name(&args[0]);
            if signal_number(&norm) != 999 {
                // It's a signal name — reset it
                let signal = norm.strip_prefix("SIG").unwrap_or(&norm).to_string();
                if !shell.original_ignored_signals.contains(&signal) {
                    shell.traps.remove(&signal);
                }
                return 0;
            }
            // Not a signal name — error (e.g., trap "" with no signal)
            eprintln!("trap: usage: trap [-Plp] [[action] signal_spec ...]");
            return 2;
        }
    }

    let handler = &args[handler_idx];

    let mut status = 0;
    for sig in &args[sig_start..] {
        let signal = sig.to_uppercase();
        let signal = signal.strip_prefix("SIG").unwrap_or(&signal).to_string();

        // Validate signal name/number
        let valid = matches!(
            signal.as_str(),
            "EXIT"
                | "0"
                | "HUP"
                | "INT"
                | "QUIT"
                | "ILL"
                | "TRAP"
                | "ABRT"
                | "BUS"
                | "FPE"
                | "KILL"
                | "USR1"
                | "SEGV"
                | "USR2"
                | "PIPE"
                | "ALRM"
                | "TERM"
                | "STKFLT"
                | "CHLD"
                | "CONT"
                | "STOP"
                | "TSTP"
                | "TTIN"
                | "TTOU"
                | "URG"
                | "XCPU"
                | "XFSZ"
                | "VTALRM"
                | "PROF"
                | "WINCH"
                | "IO"
                | "PWR"
                | "SYS"
                | "DEBUG"
                | "ERR"
                | "RETURN"
        ) || signal.parse::<u32>().is_ok_and(|n| n <= 64);

        if !valid {
            eprintln!(
                "{}: trap: {}: invalid signal specification",
                shell.error_prefix(),
                sig
            );
            status = 1;
            continue;
        }

        // Signals that were ignored at startup cannot be trapped or reset
        if shell.original_ignored_signals.contains(&signal) {
            // Silently ignore the request (bash behavior)
            continue;
        }
        if handler == "-" {
            // Reset trap to default
            shell.traps.remove(&signal);
            // Reset Unix signal disposition to default
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_DFL);
                }
            }
        } else if handler.is_empty() {
            // Empty handler = ignore signal
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                unsafe {
                    libc::signal(signum, libc::SIG_IGN);
                }
            }
        } else {
            // Set trap handler and install signal catcher
            shell.traps.insert(signal.clone(), handler.clone());
            #[cfg(unix)]
            if let Some(signum) = signal_name_to_number(&signal) {
                crate::interpreter::install_signal_handler(signum);
            }
        }
    }
    status
}

#[cfg(unix)]
fn signal_name_to_number(name: &str) -> Option<i32> {
    match name {
        "HUP" => Some(libc::SIGHUP),
        "INT" => Some(libc::SIGINT),
        "QUIT" => Some(libc::SIGQUIT),
        "ILL" => Some(libc::SIGILL),
        "TRAP" => Some(libc::SIGTRAP),
        "ABRT" => Some(libc::SIGABRT),
        "BUS" => Some(libc::SIGBUS),
        "FPE" => Some(libc::SIGFPE),
        "KILL" => Some(libc::SIGKILL),
        "USR1" => Some(libc::SIGUSR1),
        "SEGV" => Some(libc::SIGSEGV),
        "USR2" => Some(libc::SIGUSR2),
        "PIPE" => Some(libc::SIGPIPE),
        "ALRM" => Some(libc::SIGALRM),
        "TERM" => Some(libc::SIGTERM),
        "CHLD" => Some(libc::SIGCHLD),
        "CONT" => Some(libc::SIGCONT),
        "STOP" => Some(libc::SIGSTOP),
        "TSTP" => Some(libc::SIGTSTP),
        _ => None,
    }
}

fn builtin_wait(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::wait::{WaitStatus, waitpid};
        use nix::unistd::Pid;

        // Handle -n flag (wait for any single job)
        if args.first().map(|s| s.as_str()) == Some("-n") {
            match waitpid(Pid::from_raw(-1), None) {
                Ok(WaitStatus::Exited(_, code)) => {
                    shell.last_status = code;
                    return code;
                }
                Ok(WaitStatus::Signaled(_, sig, _)) => {
                    let code = 128 + sig as i32;
                    shell.last_status = code;
                    return code;
                }
                _ => return shell.last_status,
            }
        }

        if args.is_empty() {
            // Wait for all background children
            // Use blocking wait with SIGCHLD trap support
            let has_chld_trap = shell.traps.contains_key("CHLD");
            loop {
                match waitpid(Pid::from_raw(-1), None) {
                    Ok(WaitStatus::Exited(_, code)) => {
                        shell.last_status = code;
                        if has_chld_trap {
                            // Clear any pending SIGCHLD from the signal handler
                            // to avoid double-firing
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(WaitStatus::Signaled(_, sig, _)) => {
                        shell.last_status = 128 + sig as i32;
                        if has_chld_trap {
                            crate::interpreter::take_pending_signal(libc::SIGCHLD);
                            if let Some(handler) = shell.traps.get("CHLD").cloned()
                                && !handler.is_empty()
                            {
                                shell.in_trap_handler += 1;
                                shell.run_string(&handler);
                                shell.in_trap_handler -= 1;
                            }
                        }
                    }
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
        } else {
            // Wait for specific PIDs
            for arg in args {
                if let Ok(pid) = arg.parse::<i32>() {
                    match waitpid(Pid::from_raw(pid), None) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            shell.last_status = code;
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            shell.last_status = 128 + sig as i32;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    shell.last_status
}

fn builtin_kill(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        if args.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }

        // Handle kill -l [signum]
        if args.first().map(|s| s.as_str()) == Some("-l")
            || args.first().map(|s| s.as_str()) == Some("-L")
        {
            if args.len() > 1 {
                let sig_names: &[(&str, i32)] = &[
                    ("HUP", 1),
                    ("INT", 2),
                    ("QUIT", 3),
                    ("ILL", 4),
                    ("TRAP", 5),
                    ("ABRT", 6),
                    ("BUS", 7),
                    ("FPE", 8),
                    ("KILL", 9),
                    ("USR1", 10),
                    ("SEGV", 11),
                    ("USR2", 12),
                    ("PIPE", 13),
                    ("ALRM", 14),
                    ("TERM", 15),
                    ("STKFLT", 16),
                    ("CHLD", 17),
                    ("CONT", 18),
                    ("STOP", 19),
                    ("TSTP", 20),
                    ("TTIN", 21),
                    ("TTOU", 22),
                    ("URG", 23),
                    ("XCPU", 24),
                    ("XFSZ", 25),
                    ("VTALRM", 26),
                    ("PROF", 27),
                    ("WINCH", 28),
                    ("IO", 29),
                    ("PWR", 30),
                    ("SYS", 31),
                ];
                for arg in &args[1..] {
                    if let Ok(num) = arg.parse::<i32>() {
                        // kill -l <signum> — print signal name
                        let num = if num > 128 { num - 128 } else { num };
                        if let Some((name, _)) = sig_names.iter().find(|(_, n)| *n == num) {
                            println!("{}", name);
                        }
                    } else {
                        // kill -l <name> — print signal number
                        let upper = arg.to_uppercase();
                        let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
                        if let Some((_, num)) = sig_names.iter().find(|(n, _)| *n == upper) {
                            println!("{}", num);
                        } else {
                            eprintln!(
                                "{}: kill: {}: invalid signal specification",
                                shell.error_prefix(),
                                arg
                            );
                            return 1;
                        }
                    }
                }
            } else {
                // kill -l — list all signals
                // Use same signal list as trap -l
                let signals = list_all_signals();
                for (i, (num, name)) in signals.iter().enumerate() {
                    print!("{:2}) {}", num, name);
                    if (i + 1) % 5 == 0 || i == signals.len() - 1 {
                        println!();
                    } else {
                        print!("\t");
                    }
                }
            }
            return 0;
        }

        let mut signal = Signal::SIGTERM;
        let mut pids = Vec::new();

        let parse_signal = |name: &str| -> Option<Signal> {
            let upper = name.to_uppercase();
            let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
            match upper {
                "HUP" => Some(Signal::SIGHUP),
                "INT" => Some(Signal::SIGINT),
                "QUIT" => Some(Signal::SIGQUIT),
                "ILL" => Some(Signal::SIGILL),
                "TRAP" => Some(Signal::SIGTRAP),
                "ABRT" => Some(Signal::SIGABRT),
                "BUS" => Some(Signal::SIGBUS),
                "FPE" => Some(Signal::SIGFPE),
                "KILL" => Some(Signal::SIGKILL),
                "USR1" => Some(Signal::SIGUSR1),
                "SEGV" => Some(Signal::SIGSEGV),
                "USR2" => Some(Signal::SIGUSR2),
                "PIPE" => Some(Signal::SIGPIPE),
                "ALRM" => Some(Signal::SIGALRM),
                "TERM" => Some(Signal::SIGTERM),
                "CHLD" => Some(Signal::SIGCHLD),
                "CONT" => Some(Signal::SIGCONT),
                "STOP" => Some(Signal::SIGSTOP),
                "TSTP" => Some(Signal::SIGTSTP),
                "TTIN" => Some(Signal::SIGTTIN),
                "TTOU" => Some(Signal::SIGTTOU),
                _ => None,
            }
        };

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-s" || arg == "-n" {
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "{}: kill: {}: option requires an argument",
                        shell.error_prefix(),
                        arg
                    );
                    return 2;
                }
                let sig_arg = &args[i];
                if let Ok(n) = sig_arg.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_arg) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_arg
                    );
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let sig_name = &arg[1..];
                if let Ok(n) = sig_name.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else if let Some(sig) = parse_signal(sig_name) {
                    signal = sig;
                } else {
                    eprintln!(
                        "{}: kill: {}: invalid signal specification",
                        shell.error_prefix(),
                        sig_name
                    );
                    return 1;
                }
            } else if arg.is_empty() {
                eprintln!(
                    "{}: kill: `': not a pid or valid job spec",
                    shell.error_prefix()
                );
                return 1;
            } else if let Ok(pid) = arg.parse::<i32>() {
                pids.push(pid);
            } else {
                eprintln!(
                    "{}: kill: `{}': not a pid or valid job spec",
                    shell.error_prefix(),
                    arg
                );
                return 1;
            }
            i += 1;
        }

        if pids.is_empty() {
            eprintln!(
                "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
            );
            return 2;
        }
        let mut status = 0;
        for pid in pids {
            if signal::kill(Pid::from_raw(pid), signal).is_err() {
                eprintln!(
                    "{}: kill: ({}) - No such process",
                    shell.error_prefix(),
                    pid
                );
                status = 1;
            }
        }
        status
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!(
            "{}: kill: not supported on this platform",
            shell.error_prefix()
        );
        1
    }
}

fn builtin_umask(shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::stat::Mode;

        let mut symbolic = false;
        let mut print_mode = false;
        let mut mask_arg = None;

        for arg in args {
            match arg.as_str() {
                "-S" => symbolic = true,
                "-p" => print_mode = true,
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("{}: umask: {}: invalid option", shell.error_prefix(), s);
                    eprintln!("umask: usage: umask [-p] [-S] [mode]");
                    return 1;
                }
                _ => mask_arg = Some(arg.as_str()),
            }
        }

        if mask_arg.is_none() {
            let current = nix::sys::stat::umask(Mode::empty());
            nix::sys::stat::umask(current);
            if symbolic {
                let bits = current.bits();
                let u = 7 - ((bits >> 6) & 7);
                let g = 7 - ((bits >> 3) & 7);
                let o = 7 - (bits & 7);
                let mode_str = |m: u32| -> String {
                    let mut s = String::new();
                    if m & 4 != 0 {
                        s.push('r');
                    }
                    if m & 2 != 0 {
                        s.push('w');
                    }
                    if m & 1 != 0 {
                        s.push('x');
                    }
                    s
                };
                if print_mode {
                    println!(
                        "umask -S u={},g={},o={}",
                        mode_str(u),
                        mode_str(g),
                        mode_str(o)
                    );
                } else {
                    println!("u={},g={},o={}", mode_str(u), mode_str(g), mode_str(o));
                }
            } else if print_mode {
                println!("umask {:04o}", current.bits());
            } else {
                println!("{:04o}", current.bits());
            }
            return 0;
        }

        let mask_str = mask_arg.unwrap();
        // Try octal first
        if mask_str.chars().all(|c| c.is_ascii_digit()) {
            if mask_str.chars().any(|c| c == '8' || c == '9') {
                eprintln!(
                    "{}: umask: {}: octal number out of range",
                    shell.error_prefix(),
                    mask_str
                );
                return 1;
            }
            if let Ok(mask) = u32::from_str_radix(mask_str, 8) {
                nix::sys::stat::umask(Mode::from_bits_truncate(mask));
                return 0;
            }
        }

        // Try symbolic mode: [ugoa][+-=][rwx]
        // Simplified: just check for basic valid characters
        let valid_who = ['u', 'g', 'o', 'a'];
        let valid_op = ['+', '-', '='];
        let valid_perm = ['r', 'w', 'x', 'X', 's', 't'];
        let first = mask_str.chars().next().unwrap_or(' ');
        if !valid_who.contains(&first) && !valid_op.contains(&first) {
            eprintln!(
                "{}: umask: `{}': invalid symbolic mode character",
                shell.error_prefix(),
                first
            );
            return 1;
        }
        // Check for valid operator
        let has_op = mask_str.chars().any(|c| valid_op.contains(&c));
        if !has_op {
            eprintln!(
                "{}: umask: `{}': invalid symbolic mode operator",
                shell.error_prefix(),
                mask_str
                    .chars()
                    .find(|c| !valid_who.contains(c))
                    .unwrap_or(' ')
            );
            return 1;
        }
        // Check permission chars
        for ch in mask_str.chars() {
            if !valid_who.contains(&ch)
                && !valid_op.contains(&ch)
                && !valid_perm.contains(&ch)
                && ch != ','
            {
                eprintln!(
                    "{}: umask: `{}': invalid symbolic mode character",
                    shell.error_prefix(),
                    ch
                );
                return 1;
            }
        }

        // Apply symbolic mask (simplified)
        let current = nix::sys::stat::umask(Mode::empty());
        nix::sys::stat::umask(current);
        let mut bits = current.bits();
        for part in mask_str.split(',') {
            let chars: Vec<char> = part.chars().collect();
            let mut i = 0;
            let mut who_mask = 0u32;
            while i < chars.len() && valid_who.contains(&chars[i]) {
                match chars[i] {
                    'u' => who_mask |= 0o700,
                    'g' => who_mask |= 0o070,
                    'o' => who_mask |= 0o007,
                    'a' => who_mask |= 0o777,
                    _ => {}
                }
                i += 1;
            }
            if who_mask == 0 {
                who_mask = 0o777;
            }
            if i < chars.len() && valid_op.contains(&chars[i]) {
                let op = chars[i];
                i += 1;
                let mut perm = 0u32;
                while i < chars.len() && valid_perm.contains(&chars[i]) {
                    match chars[i] {
                        'r' => perm |= 0o444,
                        'w' => perm |= 0o222,
                        'x' => perm |= 0o111,
                        _ => {}
                    }
                    i += 1;
                }
                let effective = perm & who_mask;
                match op {
                    '+' => bits &= !effective,
                    '-' => bits |= effective,
                    '=' => {
                        bits |= who_mask;
                        bits &= !effective;
                    }
                    _ => {}
                }
            }
        }
        nix::sys::stat::umask(Mode::from_bits_truncate(bits));
        0
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        0
    }
}

fn builtin_getopts(shell: &mut Shell, args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("getopts: usage: getopts optstring name [arg ...]");
        return 2;
    }

    let raw_optstring = &args[0];
    let varname = &args[1];

    // Check for silent error mode (leading ':')
    let silent = raw_optstring.starts_with(':');
    let optstring = if silent {
        &raw_optstring[1..]
    } else {
        raw_optstring.as_str()
    };

    // Determine the arguments to process: explicit args or positional params
    let opt_args: Vec<&str> = if args.len() > 2 {
        args[2..].iter().map(|s| s.as_str()).collect()
    } else if shell.positional.len() > 1 {
        shell.positional[1..].iter().map(|s| s.as_str()).collect()
    } else {
        vec![]
    };

    // OPTIND is 1-based index into opt_args
    let optind: usize = shell
        .vars
        .get("OPTIND")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    // Internal character offset within the current argument (0-based offset
    // into the option characters, i.e. after the leading '-'). We store this
    // in the shell variable `_GETOPTS_OPTOFS`.
    let char_ofs: usize = shell
        .vars
        .get("_GETOPTS_OPTOFS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if optind == 0 || optind > opt_args.len() {
        shell.set_var(varname, "?".to_string());
        return 1;
    }

    let current = opt_args[optind - 1];

    // Check for end-of-options conditions
    if current == "--" {
        // Skip past '--' and signal end of options
        shell.set_var("OPTIND", (optind + 1).to_string());
        shell.vars.remove("_GETOPTS_OPTOFS");
        shell.set_var(varname, "?".to_string());
        return 1;
    }

    if !current.starts_with('-') || current == "-" {
        shell.set_var(varname, "?".to_string());
        return 1;
    }

    // The option characters are everything after the leading '-'
    let opt_chars: Vec<char> = current[1..].chars().collect();

    // Determine which character we're processing
    let idx = if char_ofs > 0 { char_ofs } else { 0 };

    if idx >= opt_chars.len() {
        // Shouldn't happen, but be safe — move to next arg
        shell.set_var("OPTIND", (optind + 1).to_string());
        shell.vars.remove("_GETOPTS_OPTOFS");
        shell.set_var(varname, "?".to_string());
        return 1;
    }

    let opt_char = opt_chars[idx];

    // Look up the option character in optstring
    let opt_pos = optstring.find(opt_char);

    match opt_pos {
        Some(pos) => {
            let needs_arg = optstring.chars().nth(pos + 1) == Some(':');

            if needs_arg {
                // Option requires an argument
                if idx + 1 < opt_chars.len() {
                    // Rest of current argument is the option-argument
                    // e.g. -oVALUE — chars after 'o' are the value
                    let byte_start = current[1..]
                        .char_indices()
                        .nth(idx + 1)
                        .map(|(i, _)| i)
                        .unwrap_or(current.len() - 1);
                    let optarg = &current[1 + byte_start..];
                    shell.set_var("OPTARG", optarg.to_string());
                    shell.set_var("OPTIND", (optind + 1).to_string());
                    shell.vars.remove("_GETOPTS_OPTOFS");
                } else if optind < opt_args.len() {
                    // Next argument is the option-argument
                    let optarg = opt_args[optind]; // optind is 1-based, so opt_args[optind] is next
                    shell.set_var("OPTARG", optarg.to_string());
                    shell.set_var("OPTIND", (optind + 2).to_string());
                    shell.vars.remove("_GETOPTS_OPTOFS");
                } else {
                    // Missing required argument
                    if silent {
                        shell.set_var(varname, ":".to_string());
                        shell.set_var("OPTARG", opt_char.to_string());
                    } else {
                        eprintln!(
                            "{}: option requires an argument -- {}",
                            shell.error_prefix(),
                            opt_char
                        );
                        shell.set_var(varname, "?".to_string());
                        shell.vars.remove("OPTARG");
                    }
                    shell.set_var("OPTIND", (optind + 1).to_string());
                    shell.vars.remove("_GETOPTS_OPTOFS");
                    return 0;
                }

                shell.set_var(varname, opt_char.to_string());
                return 0;
            }

            // Option does NOT require an argument
            shell.vars.remove("OPTARG");
            shell.set_var(varname, opt_char.to_string());

            if idx + 1 < opt_chars.len() {
                // More option characters in this argument — save offset for next call
                shell.set_var("_GETOPTS_OPTOFS", (idx + 1).to_string());
                // OPTIND stays the same
            } else {
                // Done with this argument — advance OPTIND
                shell.set_var("OPTIND", (optind + 1).to_string());
                shell.vars.remove("_GETOPTS_OPTOFS");
            }
            0
        }
        None => {
            // Unknown option character
            if silent {
                shell.set_var(varname, "?".to_string());
                shell.set_var("OPTARG", opt_char.to_string());
            } else {
                eprintln!("{}: illegal option -- {}", shell.error_prefix(), opt_char);
                shell.set_var(varname, "?".to_string());
                shell.vars.remove("OPTARG");
            }

            if idx + 1 < opt_chars.len() {
                shell.set_var("_GETOPTS_OPTOFS", (idx + 1).to_string());
            } else {
                shell.set_var("OPTIND", (optind + 1).to_string());
                shell.vars.remove("_GETOPTS_OPTOFS");
            }
            0
        }
    }
}

fn builtin_let(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("{}: let: expression expected", shell.error_prefix());
        return 1;
    }

    let mut result = 0i64;
    shell.arith_is_let = true;
    for expr in args {
        result = shell.eval_arith_expr(expr);
    }
    shell.arith_is_let = false;

    // let returns 1 if the last expression evaluates to 0, 0 otherwise
    if result == 0 { 1 } else { 0 }
}

fn builtin_mapfile(shell: &mut Shell, args: &[String]) -> i32 {
    let mut strip_trailing = false;
    let mut count: Option<usize> = None;
    let mut origin: usize = 0;
    let mut has_origin = false;
    let mut skip: usize = 0;
    let mut delim: u8 = b'\n';
    let mut callback: Option<String> = None;
    let mut quantum: usize = 5000;
    let mut varname = "MAPFILE".to_string();
    let mut fd: Option<i32> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" => strip_trailing = true,
            "-n" => {
                i += 1;
                if i < args.len() {
                    count = args[i].parse().ok();
                }
            }
            "-O" => {
                i += 1;
                if i < args.len() {
                    origin = args[i].parse().unwrap_or(0);
                    has_origin = true;
                }
            }
            "-s" => {
                i += 1;
                if i < args.len() {
                    skip = args[i].parse().unwrap_or(0);
                }
            }
            "-d" => {
                i += 1;
                if i < args.len() {
                    delim = if args[i].is_empty() {
                        0 // NUL delimiter
                    } else {
                        args[i].as_bytes()[0]
                    };
                }
            }
            "-C" => {
                i += 1;
                if i < args.len() {
                    callback = Some(args[i].clone());
                }
            }
            "-c" => {
                i += 1;
                if i < args.len() {
                    quantum = args[i].parse().unwrap_or(5000);
                }
            }
            "-u" => {
                i += 1;
                if i < args.len() {
                    match args[i].parse::<i32>() {
                        Ok(f) => fd = Some(f),
                        Err(_) => {
                            eprintln!(
                                "{}: mapfile: {}: invalid file descriptor specification",
                                shell.error_prefix(),
                                args[i]
                            );
                            return 1;
                        }
                    }
                }
            }
            a if a.starts_with('-') => {
                eprintln!("{}: mapfile: {}: invalid option", shell.error_prefix(), a);
                return 2;
            }
            _ => {
                if args[i].is_empty() {
                    eprintln!(
                        "{}: mapfile: empty array variable name",
                        shell.error_prefix()
                    );
                    return 1;
                }
                if !args[i].chars().all(|c| c.is_alphanumeric() || c == '_')
                    || args[i].chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    eprintln!(
                        "{}: mapfile: `{}': not a valid identifier",
                        shell.error_prefix(),
                        args[i]
                    );
                    return 1;
                }
                varname = args[i].clone();
            }
        }
        i += 1;
    }

    // Validate fd if specified
    #[cfg(unix)]
    if let Some(f) = fd
        && nix::fcntl::fcntl(f, nix::fcntl::FcntlArg::F_GETFD).is_err()
    {
        eprintln!(
            "{}: mapfile: {}: invalid file descriptor: Bad file descriptor",
            shell.error_prefix(),
            f
        );
        return 1;
    }

    // Read lines from stdin or specified fd
    let mut lines = Vec::new();
    use std::io::Read;

    let mut input_data = Vec::new();
    if let Some(fd_num) = fd {
        #[cfg(unix)]
        {
            use std::os::unix::io::FromRawFd;
            let mut file = unsafe { std::fs::File::from_raw_fd(fd_num) };
            let _ = file.read_to_end(&mut input_data);
            // Don't close — leak the fd so it remains valid
            std::mem::forget(file);
        }
    } else {
        let stdin = std::io::stdin();
        let _ = stdin.lock().read_to_end(&mut input_data);
    }

    // Split by delimiter
    let mut start = 0;
    for pos in 0..input_data.len() {
        if input_data[pos] == delim {
            let line = String::from_utf8_lossy(&input_data[start..=pos]).to_string();
            lines.push(line);
            start = pos + 1;
        }
    }
    // Remaining data (no trailing delimiter)
    if start < input_data.len() {
        let line = String::from_utf8_lossy(&input_data[start..]).to_string();
        lines.push(line);
    }

    // Apply skip
    if skip > 0 {
        lines = lines.into_iter().skip(skip).collect();
    }

    // Apply count limit
    if let Some(n) = count
        && n > 0
    {
        lines.truncate(n);
    }

    // Strip trailing delimiter if -t
    if strip_trailing {
        let delim_char = delim as char;
        for line in &mut lines {
            if line.ends_with(delim_char) {
                line.pop();
            }
        }
    }

    // Execute callback if specified
    if let Some(ref cb) = callback {
        for (idx, line) in lines.iter().enumerate() {
            if (idx + 1) % quantum == 0 {
                let cmd = format!("{} {} {}", cb, origin + idx, shell_quote(line));
                shell.run_string(&cmd);
            }
        }
    }

    // Store as array starting at origin
    // Without -O, clear the array first. With -O, preserve existing elements.
    if !has_origin {
        shell.arrays.insert(varname.clone(), Vec::new());
    }
    let arr = shell.arrays.entry(varname.clone()).or_default();
    // Extend array to fit origin + lines
    while arr.len() < origin {
        arr.push(String::new());
    }
    for (idx, line) in lines.iter().enumerate() {
        let pos = origin + idx;
        if pos < arr.len() {
            arr[pos] = line.clone();
        } else {
            arr.push(line.clone());
        }
    }

    0
}

fn shell_quote(s: &str) -> String {
    if s.contains('\'') {
        format!("$'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        format!("'{}'", s)
    }
}

fn builtin_alias(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print all aliases
        let mut names: Vec<&String> = shell.aliases.keys().collect();
        names.sort();
        for name in names {
            let value = &shell.aliases[name];
            println!("alias {}='{}'", name, value.replace('\'', "'\\''"));
        }
        return 0;
    }

    let mut print_only = false;
    let mut status = 0;
    let mut names = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-p" => print_only = true,
            a if a.starts_with('-') => {
                eprintln!("{}: alias: {}: invalid option", shell.error_prefix(), a);
                eprintln!("alias: usage: alias [-p] [name[=value] ... ]");
                return 2;
            }
            _ => names.push(arg.clone()),
        }
    }

    if print_only && names.is_empty() {
        let mut all: Vec<&String> = shell.aliases.keys().collect();
        all.sort();
        for name in all {
            let value = &shell.aliases[name];
            println!("alias {}='{}'", name, value.replace('\'', "'\\''"));
        }
        return 0;
    }

    for name in &names {
        if let Some(eq_pos) = name.find('=') {
            let alias_name = &name[..eq_pos];
            let alias_value = &name[eq_pos + 1..];
            // Validate alias name - reject shell metacharacters
            let invalid = alias_name.chars().any(|c| {
                matches!(
                    c,
                    '/' | '$'
                        | '`'
                        | ' '
                        | '\t'
                        | '\n'
                        | ';'
                        | '&'
                        | '|'
                        | '('
                        | ')'
                        | '<'
                        | '>'
                        | '"'
                        | '\\'
                )
            });
            if invalid || alias_name.is_empty() {
                eprintln!(
                    "{}: alias: `{}': invalid alias name",
                    shell.error_prefix(),
                    alias_name
                );
                status = 1;
                continue;
            }
            shell
                .aliases
                .insert(alias_name.to_string(), alias_value.to_string());
        } else if let Some(value) = shell.aliases.get(name.as_str()) {
            println!("alias {}='{}'", name, value.replace('\'', "'\\''"));
        } else {
            eprintln!("{}: alias: {}: not found", shell.error_prefix(), name);
            status = 1;
        }
    }
    status
}

fn builtin_unalias(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("unalias: usage: unalias [-a] name [name ...]");
        return 2;
    }

    let mut status = 0;
    for arg in args {
        match arg.as_str() {
            "-a" => {
                shell.aliases.clear();
            }
            a if a.starts_with('-') => {
                eprintln!("{}: unalias: {}: invalid option", shell.error_prefix(), a);
                eprintln!("unalias: usage: unalias [-a] name [name ...]");
                return 2;
            }
            _ => {
                if shell.aliases.remove(arg.as_str()).is_none() {
                    eprintln!("{}: unalias: {}: not found", shell.error_prefix(), arg);
                    status = 1;
                }
            }
        }
    }
    status
}

fn builtin_enable(shell: &mut Shell, args: &[String]) -> i32 {
    let builtins = builtins();
    let mut status = 0;
    for arg in args {
        if arg.starts_with('-') {
            continue; // skip flags
        }
        if !builtins.contains_key(arg.as_str()) {
            eprintln!(
                "{}: enable: {}: not a shell builtin",
                shell.error_prefix(),
                arg
            );
            status = 1;
        }
    }
    status
}

fn builtin_shopt(shell: &mut Shell, args: &[String]) -> i32 {
    let mut set = false;
    let mut unset = false;
    let mut query = false;
    let mut print_mode = false;
    let mut set_o = false;
    let mut opts = Vec::new();

    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            let mut valid = true;
            for ch in arg[1..].chars() {
                match ch {
                    's' => set = true,
                    'u' => unset = true,
                    'q' => query = true,
                    'p' => print_mode = true,
                    'o' => set_o = true,
                    _ => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                eprintln!("{}: shopt: {}: invalid option", shell.error_prefix(), arg);
                eprintln!("shopt: usage: shopt [-pqsu] [-o] [optname ...]");
                return 2;
            }
        } else {
            opts.push(arg.as_str());
        }
    }

    // Cannot set and unset simultaneously
    if set && unset {
        eprintln!(
            "{}: shopt: cannot set and unset shell options simultaneously",
            shell.error_prefix()
        );
        return 1;
    }

    // Handle -o (set -o options) separately — delegates to set -o options
    if set_o {
        let set_options: Vec<(&str, bool)> = vec![
            ("allexport", shell.opt_allexport),
            (
                "braceexpand",
                shell
                    .shopt_options
                    .get("braceexpand")
                    .copied()
                    .unwrap_or(true),
            ),
            (
                "emacs",
                shell.shopt_options.get("emacs").copied().unwrap_or(false),
            ),
            ("errexit", shell.opt_errexit),
            (
                "errtrace",
                shell
                    .shopt_options
                    .get("errtrace")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "functrace",
                shell
                    .shopt_options
                    .get("functrace")
                    .copied()
                    .unwrap_or(false),
            ),
            ("hashall", shell.opt_hashall),
            (
                "histexpand",
                shell
                    .shopt_options
                    .get("histexpand")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "history",
                shell.shopt_options.get("history").copied().unwrap_or(false),
            ),
            (
                "ignoreeof",
                shell
                    .shopt_options
                    .get("ignoreeof")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "interactive-comments",
                shell
                    .shopt_options
                    .get("interactive-comments")
                    .copied()
                    .unwrap_or(true),
            ),
            ("keyword", shell.opt_keyword),
            (
                "monitor",
                shell.shopt_options.get("monitor").copied().unwrap_or(false),
            ),
            ("noclobber", shell.opt_noclobber),
            ("noexec", shell.opt_noexec),
            ("noglob", shell.opt_noglob),
            ("nolog", false),
            (
                "notify",
                shell.shopt_options.get("notify").copied().unwrap_or(false),
            ),
            ("nounset", shell.opt_nounset),
            (
                "onecmd",
                shell.shopt_options.get("onecmd").copied().unwrap_or(false),
            ),
            (
                "physical",
                shell
                    .shopt_options
                    .get("physical")
                    .copied()
                    .unwrap_or(false),
            ),
            ("pipefail", shell.opt_pipefail),
            ("posix", shell.opt_posix),
            (
                "privileged",
                shell
                    .shopt_options
                    .get("privileged")
                    .copied()
                    .unwrap_or(false),
            ),
            (
                "verbose",
                shell.shopt_options.get("verbose").copied().unwrap_or(false),
            ),
            (
                "vi",
                shell.shopt_options.get("vi").copied().unwrap_or(false),
            ),
            ("xtrace", shell.opt_xtrace),
        ];

        if opts.is_empty() {
            // List all set -o options
            if !query {
                for (name, val) in &set_options {
                    if print_mode && set {
                        // shopt -s -p -o: only print ON options
                        if *val {
                            println!("set -o {}", name);
                        }
                    } else if print_mode && unset {
                        // shopt -u -p -o: only print OFF options
                        if !*val {
                            println!("set +o {}", name);
                        }
                    } else if print_mode {
                        // shopt -p -o: print all in set format
                        println!("set {}o {}", if *val { "-" } else { "+" }, name);
                    } else if set {
                        // shopt -s -o: list only ON options (human-readable)
                        if *val {
                            println!("{:<15}\ton", name);
                        }
                    } else if unset {
                        // shopt -u -o: list only OFF options (human-readable)
                        if !*val {
                            println!("{:<15}\toff", name);
                        }
                    } else {
                        println!("{:<15}\t{}", name, if *val { "on" } else { "off" });
                    }
                }
            }
            return 0;
        }

        // Handle specific set -o options
        let mut status = 0;
        for opt in &opts {
            if let Some((_, val)) = set_options.iter().find(|(n, _)| n == opt) {
                if set {
                    // set the option via builtin_set logic
                    match *opt {
                        "allexport" => shell.opt_allexport = true,
                        "errexit" => shell.opt_errexit = true,
                        "nounset" => shell.opt_nounset = true,
                        "xtrace" => shell.opt_xtrace = true,
                        "noclobber" => shell.opt_noclobber = true,
                        "noglob" => shell.opt_noglob = true,
                        "posix" => shell.opt_posix = true,
                        "pipefail" => shell.opt_pipefail = true,
                        _ => {
                            shell.shopt_options.insert(opt.to_string(), true);
                        }
                    }
                } else if unset {
                    match *opt {
                        "allexport" => shell.opt_allexport = false,
                        "errexit" => shell.opt_errexit = false,
                        "nounset" => shell.opt_nounset = false,
                        "xtrace" => shell.opt_xtrace = false,
                        "noclobber" => shell.opt_noclobber = false,
                        "noglob" => shell.opt_noglob = false,
                        "posix" => shell.opt_posix = false,
                        "pipefail" => shell.opt_pipefail = false,
                        _ => {
                            shell.shopt_options.insert(opt.to_string(), false);
                        }
                    }
                } else if !query {
                    if print_mode {
                        println!("set {}o {}", if *val { "-" } else { "+" }, opt);
                    } else {
                        println!("{:<15}\t{}", opt, if *val { "on" } else { "off" });
                    }
                } else if !*val {
                    status = 1;
                }
            } else {
                eprintln!(
                    "{}: shopt: {}: invalid option name",
                    shell.error_prefix(),
                    opt
                );
                status = 1;
            }
        }
        return status;
    }

    // All known shopt option names (accept silently even if not fully implemented)
    let all_known_opts = [
        "array_expand_once",
        "assoc_expand_once",
        "autocd",
        "bash_source_fullpath",
        "cdable_vars",
        "cdspell",
        "checkhash",
        "checkjobs",
        "checkwinsize",
        "cmdhist",
        "compat31",
        "compat32",
        "compat40",
        "compat41",
        "compat42",
        "compat43",
        "compat44",
        "complete_fullquote",
        "direxpand",
        "dirspell",
        "dotglob",
        "execfail",
        "expand_aliases",
        "extdebug",
        "extglob",
        "extquote",
        "failglob",
        "force_fignore",
        "globasciiranges",
        "globskipdots",
        "globstar",
        "gnu_errfmt",
        "histappend",
        "histreedit",
        "histverify",
        "hostcomplete",
        "huponexit",
        "inherit_errexit",
        "interactive_comments",
        "lastpipe",
        "lithist",
        "localvar_inherit",
        "localvar_unset",
        "login_shell",
        "mailwarn",
        "no_empty_cmd_completion",
        "nocaseglob",
        "nocasematch",
        "noexpand_translation",
        "nullglob",
        "patsub_replacement",
        "progcomp",
        "progcomp_alias",
        "promptvars",
        "restricted_shell",
        "shift_verbose",
        "sourcepath",
        "varredir_close",
        "xpg_echo",
    ];

    // Options we actually track for listing
    let _shopt_options: Vec<(&str, bool)> = vec![
        ("expand_aliases", shell.shopt_expand_aliases),
        ("extglob", shell.shopt_extglob),
        ("globstar", false),
        ("inherit_errexit", shell.shopt_inherit_errexit),
        ("lastpipe", shell.shopt_lastpipe),
        ("nocasematch", shell.shopt_nocasematch),
        ("nullglob", shell.shopt_nullglob),
        ("xpg_echo", false),
    ];

    // Build the full options table with current values
    // Helper: get shopt option value, checking per-field bools first, then generic map
    let get_opt = |name: &str| -> bool {
        match name {
            "expand_aliases" => shell.shopt_expand_aliases,
            "extglob" => shell.shopt_extglob,
            "globstar" => shell.shopt_globstar,
            "inherit_errexit" => shell.shopt_inherit_errexit,
            "lastpipe" => shell.shopt_lastpipe,
            "nocasematch" => shell.shopt_nocasematch,
            "nullglob" => shell.shopt_nullglob,
            _ => *shell.shopt_options.get(name).unwrap_or(&false),
        }
    };
    // Default values for options (used when not explicitly set)
    let defaults: &[(&str, bool)] = &[
        ("checkwinsize", false),
        ("cmdhist", true),
        ("complete_fullquote", true),
        ("extquote", true),
        ("force_fignore", true),
        ("globasciiranges", true),
        ("globskipdots", true),
        ("hostcomplete", true),
        ("interactive_comments", true),
        ("patsub_replacement", true),
        ("progcomp", true),
        ("promptvars", true),
        ("sourcepath", true),
    ];
    let option_names: &[&str] = &[
        "array_expand_once",
        "assoc_expand_once",
        "autocd",
        "bash_source_fullpath",
        "cdable_vars",
        "cdspell",
        "checkhash",
        "checkjobs",
        "checkwinsize",
        "cmdhist",
        "compat31",
        "compat32",
        "compat40",
        "compat41",
        "compat42",
        "compat43",
        "compat44",
        "complete_fullquote",
        "direxpand",
        "dirspell",
        "dotglob",
        "execfail",
        "expand_aliases",
        "extdebug",
        "extglob",
        "extquote",
        "failglob",
        "force_fignore",
        "globasciiranges",
        "globskipdots",
        "globstar",
        "gnu_errfmt",
        "histappend",
        "histreedit",
        "histverify",
        "hostcomplete",
        "huponexit",
        "inherit_errexit",
        "interactive_comments",
        "lastpipe",
        "lithist",
        "localvar_inherit",
        "localvar_unset",
        "login_shell",
        "mailwarn",
        "no_empty_cmd_completion",
        "nocaseglob",
        "nocasematch",
        "noexpand_translation",
        "nullglob",
        "patsub_replacement",
        "progcomp",
        "progcomp_alias",
        "promptvars",
        "restricted_shell",
        "shift_verbose",
        "sourcepath",
        "varredir_close",
        "xpg_echo",
    ];
    let all_options: Vec<(&str, bool)> = option_names
        .iter()
        .map(|&name| {
            let val = if shell.shopt_options.contains_key(name) {
                get_opt(name)
            } else {
                // Check defaults
                defaults
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| *v)
                    .unwrap_or_else(|| get_opt(name))
            };
            (name, val)
        })
        .collect();

    if opts.is_empty() && !set && !unset {
        // List all shopt options
        if !query {
            for (name, val) in &all_options {
                if print_mode {
                    println!("shopt {} {}", if *val { "-s" } else { "-u" }, name);
                } else {
                    println!("{:<20}\t{}", name, if *val { "on" } else { "off" });
                }
            }
        }
        return 0;
    }

    if opts.is_empty() && (set || unset) {
        // List options that are set (-s) or unset (-u)
        if !query {
            for (name, val) in &all_options {
                if (set && *val) || (unset && !*val) {
                    if print_mode {
                        println!("shopt {} {}", if *val { "-s" } else { "-u" }, name);
                    } else {
                        println!("{:<20}\t{}", name, if *val { "on" } else { "off" });
                    }
                }
            }
        }
        return 0;
    }

    let mut status = 0;
    for opt in &opts {
        match *opt {
            "nullglob" => {
                if set {
                    shell.shopt_nullglob = true;
                } else if unset {
                    shell.shopt_nullglob = false;
                } else if !query {
                    println!(
                        "{:<24}{}",
                        "nullglob",
                        if shell.shopt_nullglob { "on" } else { "off" }
                    );
                } else if !shell.shopt_nullglob {
                    status = 1;
                }
            }
            "globstar" => {
                if set {
                    shell.shopt_globstar = true;
                } else if unset {
                    shell.shopt_globstar = false;
                } else if !query {
                    println!(
                        "{:<24}{}",
                        "globstar",
                        if shell.shopt_globstar { "on" } else { "off" }
                    );
                } else if !shell.shopt_globstar {
                    status = 1;
                }
            }
            "extglob" => {
                if set {
                    shell.shopt_extglob = true;
                } else if unset {
                    shell.shopt_extglob = false;
                } else if !query {
                    println!(
                        "{:<24}{}",
                        "extglob",
                        if shell.shopt_extglob { "on" } else { "off" }
                    );
                } else if !shell.shopt_extglob {
                    status = 1;
                }
            }
            "inherit_errexit" => {
                if set {
                    shell.shopt_inherit_errexit = true;
                } else if unset {
                    shell.shopt_inherit_errexit = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} inherit_errexit",
                            if shell.shopt_inherit_errexit {
                                "-s"
                            } else {
                                "-u"
                            }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "inherit_errexit",
                            if shell.shopt_inherit_errexit {
                                "on"
                            } else {
                                "off"
                            }
                        );
                    }
                } else if !shell.shopt_inherit_errexit {
                    status = 1;
                }
            }
            "nocasematch" => {
                if set {
                    shell.shopt_nocasematch = true;
                } else if unset {
                    shell.shopt_nocasematch = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} nocasematch",
                            if shell.shopt_nocasematch { "-s" } else { "-u" }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "nocasematch",
                            if shell.shopt_nocasematch { "on" } else { "off" }
                        );
                    }
                } else if !shell.shopt_nocasematch {
                    status = 1;
                }
            }
            "lastpipe" => {
                if set {
                    shell.shopt_lastpipe = true;
                } else if unset {
                    shell.shopt_lastpipe = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} lastpipe",
                            if shell.shopt_lastpipe { "-s" } else { "-u" }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "lastpipe",
                            if shell.shopt_lastpipe { "on" } else { "off" }
                        );
                    }
                } else if !shell.shopt_lastpipe {
                    status = 1;
                }
            }
            "expand_aliases" => {
                if set {
                    shell.shopt_expand_aliases = true;
                } else if unset {
                    shell.shopt_expand_aliases = false;
                } else if !query {
                    if print_mode {
                        println!(
                            "shopt {} expand_aliases",
                            if shell.shopt_expand_aliases {
                                "-s"
                            } else {
                                "-u"
                            }
                        );
                    } else {
                        println!(
                            "{:<20}\t{}",
                            "expand_aliases",
                            if shell.shopt_expand_aliases {
                                "on"
                            } else {
                                "off"
                            }
                        );
                    }
                } else if !shell.shopt_expand_aliases {
                    status = 1;
                }
            }
            _ if all_known_opts.contains(opt) => {
                if set {
                    shell.shopt_options.insert(opt.to_string(), true);
                } else if unset {
                    shell.shopt_options.insert(opt.to_string(), false);
                } else if let Some((_, val)) = all_options.iter().find(|(n, _)| n == opt) {
                    if !query {
                        if print_mode {
                            println!("shopt {} {}", if *val { "-s" } else { "-u" }, opt);
                        } else {
                            println!("{:<20}\t{}", opt, if *val { "on" } else { "off" });
                        }
                    } else if !*val {
                        status = 1;
                    }
                }
            }
            _ => {
                if !query {
                    eprintln!(
                        "{}: shopt: {}: invalid shell option name",
                        shell.error_prefix(),
                        opt
                    );
                }
                status = 1;
            }
        }
    }
    status
}

fn builtin_dirs(_shell: &mut Shell, _args: &[String]) -> i32 {
    match std::env::current_dir() {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("bash: dirs: {}", e);
            1
        }
    }
}

fn builtin_pushd(shell: &mut Shell, args: &[String]) -> i32 {
    let dir = args.first().cloned().unwrap_or_else(|| {
        shell
            .vars
            .get("HOME")
            .cloned()
            .unwrap_or_else(|| "/".to_string())
    });

    let current = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    shell.dir_stack.push(current);

    builtin_cd(shell, &[dir])
}

fn builtin_popd(shell: &mut Shell, _args: &[String]) -> i32 {
    if let Some(dir) = shell.dir_stack.pop() {
        builtin_cd(shell, &[dir])
    } else {
        eprintln!("bash: popd: directory stack empty");
        1
    }
}

fn builtin_complete(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // No-op
}

fn builtin_compgen(shell: &mut Shell, args: &[String]) -> i32 {
    // Parse -A action and optional prefix
    let mut action = None;
    let mut prefix = String::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-A" => {
                i += 1;
                if i < args.len() {
                    action = Some(args[i].clone());
                }
            }
            a if !a.starts_with('-') => prefix = a.to_string(),
            _ => {}
        }
        i += 1;
    }

    match action.as_deref() {
        Some("function") => {
            let mut names: Vec<&String> = shell.functions.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("variable") => {
            let mut names: Vec<&String> = shell.vars.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("alias") => {
            let mut names: Vec<&String> = shell.aliases.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("builtin") => {
            let builtins = builtins();
            let mut names: Vec<&&str> = builtins.keys().collect();
            names.sort();
            for name in names {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        Some("shopt") => {
            // List all shopt option names
            let all_shopts = [
                "array_expand_once",
                "assoc_expand_once",
                "autocd",
                "bash_source_fullpath",
                "cdable_vars",
                "cdspell",
                "checkhash",
                "checkjobs",
                "checkwinsize",
                "cmdhist",
                "compat31",
                "compat32",
                "compat40",
                "compat41",
                "compat42",
                "compat43",
                "compat44",
                "complete_fullquote",
                "direxpand",
                "dirspell",
                "dotglob",
                "execfail",
                "expand_aliases",
                "extdebug",
                "extglob",
                "extquote",
                "failglob",
                "force_fignore",
                "globasciiranges",
                "globskipdots",
                "globstar",
                "gnu_errfmt",
                "histappend",
                "histreedit",
                "histverify",
                "hostcomplete",
                "huponexit",
                "inherit_errexit",
                "interactive_comments",
                "lastpipe",
                "lithist",
                "localvar_inherit",
                "localvar_unset",
                "login_shell",
                "mailwarn",
                "no_empty_cmd_completion",
                "nocaseglob",
                "nocasematch",
                "noexpand_translation",
                "nullglob",
                "patsub_replacement",
                "progcomp",
                "progcomp_alias",
                "promptvars",
                "restricted_shell",
                "shift_verbose",
                "sourcepath",
                "varredir_close",
                "xpg_echo",
            ];
            for name in &all_shopts {
                if prefix.is_empty() || name.starts_with(&prefix) {
                    println!("{}", name);
                }
            }
        }
        _ => {}
    }
    0
}

fn builtin_times(_shell: &mut Shell, _args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // Get resource usage for this process
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            usage.ru_utime.tv_sec / 60,
            usage.ru_utime.tv_sec % 60,
            usage.ru_utime.tv_usec / 1000,
            usage.ru_stime.tv_sec / 60,
            usage.ru_stime.tv_sec % 60,
            usage.ru_stime.tv_usec / 1000,
        );
        // Get resource usage for children
        let mut child_usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut child_usage) };
        println!(
            "{}m{}.{:03}s {}m{}.{:03}s",
            child_usage.ru_utime.tv_sec / 60,
            child_usage.ru_utime.tv_sec % 60,
            child_usage.ru_utime.tv_usec / 1000,
            child_usage.ru_stime.tv_sec / 60,
            child_usage.ru_stime.tv_sec % 60,
            child_usage.ru_stime.tv_usec / 1000,
        );
        0
    }
    #[cfg(not(unix))]
    {
        println!("0m0.000s 0m0.000s");
        println!("0m0.000s 0m0.000s");
        0
    }
}

pub fn find_command_path(name: &str) -> Option<String> {
    find_in_path_opt(name)
}

pub fn find_executable(name: &str) -> String {
    if name.contains('/') {
        return name.to_string();
    }
    find_in_path(name)
}

fn find_in_path(name: &str) -> String {
    find_in_path_opt(name).unwrap_or_else(|| name.to_string())
}

fn find_in_path_opt(name: &str) -> Option<String> {
    if name.contains('/') {
        if std::path::Path::new(name).exists() {
            return Some(name.to_string());
        }
        return None;
    }

    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let full = format!("{}/{}", dir, name);
            if std::path::Path::new(&full).exists() {
                return Some(full);
            }
        }
    }
    None
}

fn builtin_ulimit(_shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        // Handle basic -n (open files) case
        let mut resource = libc::RLIMIT_FSIZE;
        let mut set_value: Option<u64> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-n" => resource = libc::RLIMIT_NOFILE,
                "-c" => resource = libc::RLIMIT_CORE,
                "-d" => resource = libc::RLIMIT_DATA,
                "-f" => resource = libc::RLIMIT_FSIZE,
                "-l" => resource = libc::RLIMIT_MEMLOCK,
                "-m" => resource = libc::RLIMIT_RSS,
                "-s" => resource = libc::RLIMIT_STACK,
                "-t" => resource = libc::RLIMIT_CPU,
                "-v" => resource = libc::RLIMIT_AS,
                "-S" | "-H" => {} // soft/hard limit flags
                "unlimited" => set_value = Some(libc::RLIM_INFINITY),
                val => {
                    if let Ok(n) = val.parse::<u64>() {
                        set_value = Some(n);
                    }
                }
            }
            i += 1;
        }

        if let Some(val) = set_value {
            let rlim = libc::rlimit {
                rlim_cur: val,
                rlim_max: val,
            };
            unsafe { libc::setrlimit(resource, &rlim) };
        } else {
            let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
            unsafe { libc::getrlimit(resource, &mut rlim) };
            if rlim.rlim_cur == libc::RLIM_INFINITY {
                println!("unlimited");
            } else {
                println!("{}", rlim.rlim_cur);
            }
        }
    }
    0
}

fn builtin_caller(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // stub
}

fn builtin_jobs(_shell: &mut Shell, _args: &[String]) -> i32 {
    // Minimal stub — job control is not fully implemented
    0
}

fn builtin_disown(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

fn builtin_fg(shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("{}: fg: no job control", shell.error_prefix());
    1
}

fn builtin_bg(shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("{}: bg: no job control", shell.error_prefix());
    1
}

fn builtin_suspend(_shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("bash: suspend: cannot suspend");
    1
}
