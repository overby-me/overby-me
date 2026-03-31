use super::*;

pub(super) fn builtin_echo(shell: &mut Shell, args: &[String]) -> i32 {
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
    let (output, stop) = if interpret_escapes {
        interpret_echo_escapes(&text)
    } else {
        (text, false)
    };
    if stop {
        newline = false;
    }

    // Convert to bytes: chars in U+0080..U+00FF range are written as single
    // bytes (raw byte output like bash), not as multi-byte UTF-8
    let mut bytes = string_to_raw_bytes(&output);
    if newline {
        bytes.push(b'\n');
    }
    // Use direct fd write to properly detect errors (Rust's BufWriter
    // may not propagate EBADF from write to read-only fds)
    #[cfg(unix)]
    {
        // Flush Rust stdout first to avoid buffered data being out of order
        std::io::Write::flush(&mut std::io::stdout()).ok();
        match nix::unistd::write(std::io::stdout(), &bytes) {
            Ok(_) => 0,
            Err(nix::Error::EPIPE) => {
                // Broken pipe — in pipeline children, suppress the error
                // and exit silently (bash doesn't report EPIPE in pipelines).
                // In other contexts (e.g. process substitutions), report
                // the error like bash does.
                if shell.in_pipeline_child {
                    std::process::exit(1);
                }
                eprintln!("{}: echo: write error: Broken pipe", shell.error_prefix());
                1
            }
            Err(e) => {
                let msg = Shell::io_error_message(&std::io::Error::from_raw_os_error(e as i32));
                eprintln!("{}: echo: write error: {}", shell.error_prefix(), msg);
                1
            }
        }
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        match out.write_all(&bytes).and_then(|_| out.flush()) {
            Ok(()) => 0,
            Err(e) => {
                let msg = Shell::io_error_message(&e);
                eprintln!("{}: echo: write error: {}", shell.error_prefix(), msg);
                1
            }
        }
    }
}

/// Convert a string to raw bytes. Characters in U+0000..U+007F are written as
/// single ASCII bytes. Characters in U+0080..U+00FF are written as single bytes
/// (Latin-1/raw byte output, matching bash's behavior for $'\xNN'). Characters
/// above U+00FF are written as their UTF-8 encoding.
pub(super) fn builtin_printf(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("printf: usage: printf [-v var] format [arguments]");
        return 1;
    }

    // Handle options
    if args[0].starts_with('-') && args[0] != "-v" && args[0] != "--" {
        eprintln!(
            "{}: printf: {}: invalid option",
            shell.error_prefix(),
            args[0]
        );
        eprintln!("printf: usage: printf [-v var] format [arguments]");
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
        // Capture printf output to variable instead of stdout
        // Use pipe to capture raw output (preserving trailing newlines)
        let inner_args: Vec<String> = args[2..].to_vec();
        let (read_fd, write_fd) = {
            use std::os::fd::IntoRawFd;
            let (r, w) = nix::unistd::pipe().unwrap();
            (r.into_raw_fd(), w.into_raw_fd())
        };
        let saved_stdout = nix::fcntl::fcntl(1, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10)).unwrap();
        nix::unistd::dup2(write_fd, 1).ok();
        // Run printf with remaining args
        let result = builtin_printf(shell, &inner_args);
        use std::io::Write;
        std::io::stdout().flush().ok();
        // Restore stdout and close pipe write end
        nix::unistd::dup2(saved_stdout, 1).ok();
        nix::unistd::close(saved_stdout).ok();
        nix::unistd::close(write_fd).ok();
        // Read captured output
        let mut output = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match nix::unistd::read(read_fd, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => output.extend_from_slice(&buf[..n]),
            }
        }
        nix::unistd::close(read_fd).ok();
        let output_str = String::from_utf8_lossy(&output).to_string();
        shell.set_var(&var_name, output_str);
        return result;
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
    let mut had_error = false;
    let mut bytes_written: usize = 0;

    // printf reuses format string until all arguments are consumed
    loop {
        let mut chars = format.chars().peekable();
        let start_arg_idx = arg_idx;
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => {
                        println!();
                        bytes_written += 1;
                    }
                    Some('t') => {
                        print!("\t");
                        bytes_written += 1;
                    }
                    Some('r') => {
                        print!("\r");
                        bytes_written += 1;
                    }
                    Some('\\') => {
                        print!("\\");
                        bytes_written += 1;
                    }
                    Some('a') => {
                        print!("\x07");
                        bytes_written += 1;
                    }
                    Some('b') => {
                        print!("\x08");
                        bytes_written += 1;
                    }
                    Some('f') => {
                        print!("\x0c");
                        bytes_written += 1;
                    }
                    Some('v') => {
                        print!("\x0b");
                        bytes_written += 1;
                    }
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
                        bytes_written += 1;
                    }
                    Some('\'') => {
                        print!("'");
                        bytes_written += 1;
                    }
                    Some('"') => {
                        print!("\"");
                        bytes_written += 1;
                    }
                    Some('?') => {
                        print!("?");
                        bytes_written += 1;
                    }
                    Some('x') => {
                        // \xNN hex escape
                        let mut val = 0u8;
                        let mut count = 0;
                        for _ in 0..2 {
                            match chars.peek() {
                                Some(d) if d.is_ascii_hexdigit() => {
                                    val = val * 16 + d.to_digit(16).unwrap() as u8;
                                    chars.next();
                                    count += 1;
                                }
                                _ => break,
                            }
                        }
                        if count == 0 {
                            eprintln!(
                                "{}: printf: missing hex digit for \\x",
                                shell.error_prefix()
                            );
                            had_error = true;
                        } else {
                            use std::io::Write;
                            std::io::stdout().write_all(&[val]).ok();
                            bytes_written += 1;
                        }
                    }
                    Some(c) => {
                        print!("\\{}", c);
                        bytes_written += 2;
                    }
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
                let mut inline_precision_overflow = false;
                if chars.peek() == Some(&'.') {
                    chars.next();
                    if chars.peek() == Some(&'*') {
                        // Precision from argument
                        chars.next();
                        let p_arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let p_parsed = p_arg.parse::<i64>();
                        let p_overflow = p_parsed
                            .as_ref()
                            .map(|v| *v > i32::MAX as i64)
                            .unwrap_or_else(|_| {
                                // parse failed — check if it's a large number
                                p_arg.chars().all(|c| c.is_ascii_digit()) && !p_arg.is_empty()
                            });
                        if p_overflow {
                            eprintln!(
                                "{}: printf: {}: Numerical result out of range",
                                shell.error_prefix(),
                                p_arg
                            );
                            had_error = true;
                            precision = None; // continue with no truncation
                        } else {
                            precision = Some(p_parsed.unwrap_or(0).max(0) as usize);
                        }
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
                        let pp = prec_str.parse::<i64>();
                        let pp_overflow =
                            pp.as_ref()
                                .map(|v| *v > i32::MAX as i64)
                                .unwrap_or_else(|_| {
                                    !prec_str.is_empty()
                                        && prec_str.chars().all(|c| c.is_ascii_digit())
                                });
                        inline_precision_overflow = pp_overflow;
                        if pp_overflow {
                            had_error = true;
                            // Don't break here; let the format specifier decide
                            // whether to output or not. %Q/%q still output,
                            // %s and others do not.
                            precision = None;
                        } else {
                            precision = Some(pp.unwrap_or(0).max(0) as usize);
                        }
                    }
                }
                // Handle negative width (means left-align)
                // Detect overflow: if width string is non-empty but parse fails, it's overflow
                let width_overflow = !width_str.is_empty() && {
                    let abs = width_str.strip_prefix('-').unwrap_or(&width_str);
                    !abs.is_empty()
                        && abs
                            .parse::<i64>()
                            .map(|v| v > i32::MAX as i64)
                            .unwrap_or(true)
                };
                if width_overflow {
                    eprintln!(
                        "{}: printf: Value too large for defined data type",
                        shell.error_prefix()
                    );
                    had_error = true;
                }
                let (w, left) = if width_overflow {
                    (0, flags.contains('-'))
                } else if let Some(stripped) = width_str.strip_prefix('-') {
                    let abs_w: usize = stripped.parse().unwrap_or(0);
                    (abs_w, true)
                } else {
                    (width_str.parse().unwrap_or(0), flags.contains('-'))
                };
                let zero_pad = flags.contains('0');
                // Skip length modifiers (l, ll, h, hh, L, z, j, t)
                // — bash ignores these since all integers are native-width
                if let Some(&c) = chars.peek()
                    && matches!(c, 'l' | 'h' | 'L' | 'z' | 'j' | 't')
                {
                    chars.next();
                    if let Some(&c2) = chars.peek()
                        && ((c == 'l' && c2 == 'l') || (c == 'h' && c2 == 'h'))
                    {
                        chars.next();
                    }
                }
                match chars.next() {
                    Some('(') => {
                        // %(fmt)T — strftime format; track nested parens
                        let mut fmt = String::new();
                        let mut depth = 1i32;
                        while let Some(&c) = chars.peek() {
                            chars.next();
                            if c == '(' {
                                depth += 1;
                                fmt.push(c);
                            } else if c == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                fmt.push(c);
                            } else {
                                fmt.push(c);
                            }
                        }
                        // Check terminator — must be 'T'
                        let term = chars.peek().copied();
                        if term == Some('T') {
                            chars.next();
                        } else {
                            // Non-T terminator: warn and print literal
                            let term_ch = term.unwrap_or('\0');
                            if term_ch != '\0' {
                                use std::io::Write;
                                std::io::stdout().flush().ok();
                                eprintln!(
                                    "{}: printf: warning: `{}': invalid time format specification",
                                    shell.error_prefix(),
                                    term_ch
                                );
                                chars.next();
                            }
                            print!(
                                "%({}){}",
                                fmt,
                                if term_ch != '\0' {
                                    term_ch.to_string()
                                } else {
                                    String::new()
                                }
                            );
                            bytes_written += 3 + fmt.len() + if term_ch != '\0' { 1 } else { 0 };
                            arg_idx += 1;
                            continue; // skip strftime
                        }
                        // Default empty format to %X (locale time)
                        if fmt.is_empty() {
                            fmt = "%X".to_string();
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("-1");
                        let timestamp: i64 = if arg == "-1" || arg == "-2" {
                            // -1 = current time, -2 = shell startup time
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
                            let mut buf = [0u8; 4096];
                            let len = unsafe {
                                libc::strftime(
                                    buf.as_mut_ptr() as *mut libc::c_char,
                                    buf.len(),
                                    c_fmt.as_ptr(),
                                    &tm,
                                )
                            };
                            let result = String::from_utf8_lossy(&buf[..len]).to_string();
                            // Apply width
                            if w > 0 {
                                if left {
                                    print!("{:<w$}", result);
                                } else {
                                    print!("{:>w$}", result);
                                }
                                bytes_written += w.max(result.len());
                            } else {
                                print!("{}", result);
                                bytes_written += result.len();
                            }
                        }
                        arg_idx += 1;
                    }
                    Some('s') | Some('S') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        // Apply precision (truncate string, byte-safe)
                        let truncated = if let Some(p) = precision {
                            // Find the byte offset for the p-th char boundary
                            let end = arg
                                .char_indices()
                                .nth(p)
                                .map(|(i, _)| i)
                                .unwrap_or(arg.len());
                            &arg[..end]
                        } else {
                            arg
                        };
                        let w = w.min(4096);
                        if w > 0 {
                            let printed_len = w.max(truncated.len());
                            if left {
                                print!("{:<w$}", truncated);
                            } else {
                                print!("{:>w$}", truncated);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", truncated);
                            bytes_written += truncated.len();
                        }
                        arg_idx += 1;
                    }
                    Some('d') | Some('i') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg_provided = arg_idx < fmt_args.len();
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = parse_printf_int(arg);
                        // Check for non-numeric or overflow
                        let abs_arg = arg.strip_prefix('-').unwrap_or(arg);
                        let is_overflow = !arg.is_empty()
                            && arg.parse::<i64>().is_err()
                            && abs_arg.parse::<u64>().is_ok();
                        if is_overflow {
                            eprintln!(
                                "{}: printf: {}: Numerical result out of range",
                                shell.error_prefix(),
                                arg
                            );
                            had_error = true;
                        } else if arg_provided
                            && !arg.starts_with('\'')
                            && !arg.starts_with('"')
                            && (arg.is_empty()
                                || (arg.parse::<i64>().is_err()
                                    && abs_arg.parse::<u64>().is_err()
                                    && !arg.starts_with("0x")
                                    && !arg.starts_with("0X")
                                    && !(arg.starts_with('0') && arg.len() > 1)))
                        {
                            eprintln!("{}: printf: {}: invalid number", shell.error_prefix(), arg);
                            had_error = true;
                        }
                        let show_sign = flags.contains('+');
                        let space_sign = flags.contains(' ');
                        let sign_prefix = if n >= 0 && show_sign {
                            "+"
                        } else if n >= 0 && space_sign {
                            " "
                        } else {
                            ""
                        };
                        let effective_width = w;
                        // For integers, precision specifies minimum digits (zero-padded)
                        // When precision is specified, the 0 flag is ignored for width padding
                        let use_zero_pad = zero_pad && precision.is_none();
                        // Format number with precision (minimum digits)
                        let abs_n = n.unsigned_abs();
                        let prefix = if n < 0 { "-" } else { sign_prefix };
                        let digits = if let Some(p) = precision {
                            format!("{:0>width$}", abs_n, width = p)
                        } else {
                            abs_n.to_string()
                        };
                        let formatted = format!("{}{}", prefix, digits);
                        if effective_width > 0 {
                            if left {
                                let ew = effective_width.min(4096);
                                print!("{:<ew$}", formatted);
                                bytes_written += ew.max(formatted.len());
                            } else if use_zero_pad {
                                // Zero-pad: sign first, then zeros
                                let total_len = formatted.len();
                                if total_len < effective_width {
                                    let pad = effective_width - total_len;
                                    print!("{}{}{}", prefix, "0".repeat(pad), digits);
                                    bytes_written += effective_width;
                                } else {
                                    print!("{}", formatted);
                                    bytes_written += formatted.len();
                                }
                            } else {
                                let ew = effective_width.min(4096);
                                print!("{:>ew$}", formatted);
                                bytes_written += ew.max(formatted.len());
                            }
                        } else {
                            print!("{}", formatted);
                            bytes_written += formatted.len();
                        }
                        arg_idx += 1;
                    }
                    Some(hex_ch @ ('x' | 'X')) => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg_provided = arg_idx < fmt_args.len();
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = parse_printf_int(arg);
                        if arg_provided && !arg.starts_with('\'') && !arg.starts_with('"') {
                            let abs_arg = arg.strip_prefix('-').unwrap_or(arg);
                            if arg.is_empty()
                                || (arg.parse::<i64>().is_err()
                                    && abs_arg.parse::<u64>().is_err()
                                    && !arg.starts_with("0x")
                                    && !arg.starts_with("0X")
                                    && !(arg.starts_with('0') && arg.len() > 1))
                            {
                                eprintln!(
                                    "{}: printf: {}: invalid number",
                                    shell.error_prefix(),
                                    arg
                                );
                                had_error = true;
                            }
                        }
                        // Apply precision (minimum digits) for hex
                        let raw_hex = if hex_ch == 'x' {
                            format!("{:x}", n)
                        } else {
                            format!("{:X}", n)
                        };
                        let digits = if let Some(p) = precision {
                            format!("{:0>width$}", raw_hex, width = p)
                        } else {
                            raw_hex
                        };
                        let formatted = if flags.contains('#') && n != 0 {
                            if hex_ch == 'x' {
                                format!("0x{}", digits)
                            } else {
                                format!("0X{}", digits)
                            }
                        } else {
                            digits
                        };
                        // Precision overrides 0 flag for integers
                        let use_zero = zero_pad && precision.is_none();
                        if w > 0 {
                            let printed_len = w.max(formatted.len());
                            if left {
                                print!("{:<w$}", formatted);
                            } else if use_zero {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", formatted);
                            bytes_written += formatted.len();
                        }
                        arg_idx += 1;
                    }
                    Some('o') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg_provided = arg_idx < fmt_args.len();
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: i64 = parse_printf_int(arg);
                        if arg_provided && !arg.starts_with('\'') && !arg.starts_with('"') {
                            let abs_arg = arg.strip_prefix('-').unwrap_or(arg);
                            if arg.is_empty()
                                || (arg.parse::<i64>().is_err()
                                    && abs_arg.parse::<u64>().is_err()
                                    && !arg.starts_with("0x")
                                    && !arg.starts_with("0X")
                                    && !(arg.starts_with('0') && arg.len() > 1))
                            {
                                eprintln!(
                                    "{}: printf: {}: invalid number",
                                    shell.error_prefix(),
                                    arg
                                );
                                had_error = true;
                            }
                        }
                        let formatted = if flags.contains('#') {
                            format!("0{:o}", n) // C-style 0 prefix, not Rust's 0o
                        } else {
                            format!("{:o}", n)
                        };
                        if w > 0 {
                            let printed_len = w.max(formatted.len());
                            if left {
                                print!("{:<w$}", formatted);
                            } else if zero_pad {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", formatted);
                            bytes_written += formatted.len();
                        }
                        arg_idx += 1;
                    }
                    Some('u') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg_provided = arg_idx < fmt_args.len();
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: u64 = parse_printf_int(arg) as u64;
                        if arg_provided && !arg.starts_with('\'') && !arg.starts_with('"') {
                            let abs_arg = arg.strip_prefix('-').unwrap_or(arg);
                            if arg.is_empty()
                                || (arg.parse::<i64>().is_err()
                                    && abs_arg.parse::<u64>().is_err()
                                    && !arg.starts_with("0x")
                                    && !arg.starts_with("0X")
                                    && !(arg.starts_with('0') && arg.len() > 1))
                            {
                                eprintln!(
                                    "{}: printf: {}: invalid number",
                                    shell.error_prefix(),
                                    arg
                                );
                                had_error = true;
                            }
                        }
                        let formatted = format!("{}", n);
                        if w > 0 {
                            let printed_len = w.max(formatted.len());
                            if left {
                                print!("{:<w$}", formatted);
                            } else if zero_pad {
                                print!("{:0>w$}", formatted);
                            } else {
                                print!("{:>w$}", formatted);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", formatted);
                            bytes_written += formatted.len();
                        }
                        arg_idx += 1;
                    }
                    Some(fmt_ch @ ('f' | 'F' | 'e' | 'E' | 'g' | 'G')) => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                        let n: f64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                            i64::from_str_radix(&arg[2..], 16).unwrap_or(0) as f64
                        } else if arg.starts_with("0")
                            && arg.len() > 1
                            && arg.chars().skip(1).all(|c| c.is_ascii_digit())
                            && !arg.contains('.')
                        {
                            i64::from_str_radix(&arg[1..], 8).unwrap_or(0) as f64
                        } else if arg.starts_with('\'') || arg.starts_with('"') {
                            arg.chars().nth(1).map(|c| c as i64 as f64).unwrap_or(0.0)
                        } else {
                            match arg.parse() {
                                Ok(v) => v,
                                Err(_) if !arg.is_empty() => {
                                    eprintln!(
                                        "{}: printf: {}: invalid number",
                                        shell.error_prefix(),
                                        arg
                                    );
                                    had_error = true;
                                    0.0
                                }
                                _ => 0.0,
                            }
                        };
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
                                // C %g: use %e if exponent < -4 or >= precision, else %f
                                // Strip trailing zeros from result
                                let p = if p == 0 { 1 } else { p };
                                let upper = fmt_ch == 'G';
                                // Determine exponent
                                let exponent = if n == 0.0 {
                                    0i32
                                } else {
                                    n.abs().log10().floor() as i32
                                };
                                let use_scientific = exponent < -4 || exponent >= p as i32;
                                let raw = if use_scientific {
                                    let e_prec = p.saturating_sub(1);
                                    let s = if upper {
                                        format!("{:.prec$E}", n, prec = e_prec)
                                    } else {
                                        format!("{:.prec$e}", n, prec = e_prec)
                                    };
                                    fix_scientific_notation(&s, upper)
                                } else {
                                    // For %f style, precision = significant digits - digits before decimal - 1
                                    let f_prec = (p as i32 - exponent - 1).max(0) as usize;
                                    format!("{:.prec$}", n, prec = f_prec)
                                };
                                // Strip trailing zeros (unless # flag)
                                if !flags.contains('#') && raw.contains('.') {
                                    let trimmed = raw.trim_end_matches('0');
                                    trimmed.trim_end_matches('.').to_string()
                                } else {
                                    raw
                                }
                            }
                            _ => format!("{:.p$}", n), // f, F
                        };
                        // Apply sign prefix
                        let sign_prefix = if n >= 0.0 && flags.contains('+') {
                            "+"
                        } else if n >= 0.0 && flags.contains(' ') {
                            " "
                        } else {
                            ""
                        };
                        let display = if !sign_prefix.is_empty() && !formatted.starts_with('-') {
                            format!("{}{}", sign_prefix, formatted)
                        } else {
                            formatted
                        };
                        if w > 0 {
                            if left {
                                print!("{:<w$}", display);
                                bytes_written += w.max(display.len());
                            } else if zero_pad && !left {
                                // Zero-pad: put sign first, then zeros, then number
                                let total_len = display.len();
                                if total_len < w {
                                    let pad_count = w - total_len;
                                    if display.starts_with('-') || display.starts_with('+') {
                                        let (sign, rest) = display.split_at(1);
                                        print!("{}{}{}", sign, "0".repeat(pad_count), rest);
                                    } else {
                                        print!("{}{}", "0".repeat(pad_count), display);
                                    }
                                    bytes_written += w;
                                } else {
                                    print!("{}", display);
                                    bytes_written += display.len();
                                }
                            } else {
                                print!("{:>w$}", display);
                                bytes_written += w.max(display.len());
                            }
                        } else {
                            print!("{}", display);
                            bytes_written += display.len();
                        }
                        arg_idx += 1;
                    }
                    Some('c') | Some('C') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        if let Some(ch) = arg.chars().next() {
                            let ch_str = ch.to_string();
                            if w > 0 {
                                let printed_len = w.max(ch_str.len());
                                if left {
                                    print!("{:<w$}", ch_str);
                                } else {
                                    print!("{:>w$}", ch_str);
                                }
                                bytes_written += printed_len;
                            } else {
                                print!("{}", ch_str);
                                bytes_written += ch_str.len();
                            }
                        } else {
                            // Empty/missing argument: output a NUL byte (bash behavior)
                            use std::io::Write;
                            std::io::stdout().write_all(&[0u8]).ok();
                            bytes_written += 1;
                        }
                        arg_idx += 1;
                    }
                    Some('b') => {
                        if inline_precision_overflow {
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            break;
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        let has_stop = arg.contains("\\c");
                        // Check for \x with no hex digits in %b argument
                        // Must skip \\x (escaped backslash + literal x)
                        {
                            let bytes = arg.as_bytes();
                            let mut i = 0;
                            while i < bytes.len().saturating_sub(1) {
                                if bytes[i] == b'\\' {
                                    if bytes[i + 1] == b'\\' {
                                        i += 2; // skip escaped backslash
                                        continue;
                                    }
                                    if bytes[i + 1] == b'x' {
                                        let next = bytes.get(i + 2).copied().unwrap_or(0);
                                        if !next.is_ascii_hexdigit() {
                                            eprintln!(
                                                "{}: printf: missing hex digit for \\x",
                                                shell.error_prefix()
                                            );
                                            had_error = true;
                                            break;
                                        }
                                        i += 2;
                                        continue;
                                    }
                                }
                                i += 1;
                            }
                        }
                        let (expanded, _) = interpret_echo_escapes(arg);
                        // Apply precision (truncate) then width (pad)
                        let truncated = if let Some(p) = precision {
                            let end = expanded
                                .char_indices()
                                .nth(p)
                                .map(|(i, _)| i)
                                .unwrap_or(expanded.len());
                            &expanded[..end]
                        } else {
                            &expanded
                        };
                        let w = w.min(4096);
                        if w > 0 {
                            let printed_len = w.max(truncated.len());
                            if left {
                                print!("{:<w$}", truncated);
                            } else {
                                print!("{:>w$}", truncated);
                            }
                            bytes_written += printed_len;
                        } else {
                            // Use raw byte output for %b (supports NUL bytes and raw bytes)
                            let raw = string_to_raw_bytes(truncated);
                            bytes_written += raw.len();
                            use std::io::Write;
                            std::io::stdout().write_all(&raw).ok();
                        }
                        arg_idx += 1;
                        // \c in %b stops all further printf output
                        if has_stop {
                            use std::io::Write;
                            std::io::stdout().flush().ok();
                            return if had_error { 1 } else { 0 };
                        }
                    }
                    Some('q') => {
                        if inline_precision_overflow {
                            // %q with inline overflow: print error but still output
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            // precision is already None, so no truncation — fall through
                        }
                        let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                        let use_locale_quote = flags.contains('#');
                        let mut quoted = if arg.is_empty() {
                            "''".to_string()
                        } else if use_locale_quote {
                            // %#q uses single-quote style (locale-aware)
                            format!("'{}'", arg.replace('\'', "'\\''"))
                        } else {
                            shell_escape(arg)
                        };
                        // %q precision truncates the QUOTED form
                        if let Some(p) = precision {
                            let truncated: String = quoted.chars().take(p).collect();
                            quoted = truncated;
                        }
                        if w > 0 {
                            let printed_len = w.max(quoted.len());
                            if left {
                                print!("{:<w$}", quoted);
                            } else {
                                print!("{:>w$}", quoted);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", quoted);
                            bytes_written += quoted.len();
                        }
                        arg_idx += 1;
                    }
                    Some('Q') => {
                        if inline_precision_overflow {
                            // %Q with inline overflow: use "Numerical result out of range"
                            // and still output (bash behavior)
                            eprintln!(
                                "{}: printf: Value too large for defined data type",
                                shell.error_prefix()
                            );
                            // precision is already None, so no truncation — fall through
                        }
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
                            let printed_len = w.max(quoted.len());
                            if left {
                                print!("{:<w$}", quoted);
                            } else {
                                print!("{:>w$}", quoted);
                            }
                            bytes_written += printed_len;
                        } else {
                            print!("{}", quoted);
                            bytes_written += quoted.len();
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
                            use std::io::Write;
                            std::io::stdout().flush().ok();
                            shell.set_var(var_name, bytes_written.to_string());
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
                    Some(c) => {
                        // Invalid format character — do NOT flush stdout here;
                        // bash lets the error (stderr, unbuffered) appear before
                        // any pending stdout content.
                        eprintln!(
                            "{}: printf: `{}': invalid format character",
                            shell.error_prefix(),
                            c
                        );
                        had_error = true;
                        break;
                    }
                    None => {
                        // Missing format character at end of string
                        let fmt_spec = format!("%{}{}", flags, width_str);
                        eprintln!(
                            "{}: printf: `{}': missing format character",
                            shell.error_prefix(),
                            fmt_spec
                        );
                        had_error = true;
                        break;
                    }
                }
            } else {
                print!("{}", ch);
                bytes_written += ch.len_utf8();
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
    if had_error { 1 } else { 0 }
}

/// Shell-escape a string for use with %q in printf.
/// Convert a Rust io::Error to a bash-style error message
pub(super) fn builtin_read(shell: &mut Shell, args: &[String]) -> i32 {
    let mut prompt = String::new();
    let mut raw = false;
    let mut _use_readline = false;
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
                    's' => {}
                    'e' => _use_readline = true,
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
                        // Check if fd number follows in same arg (e.g., -ru3 → fd=3)
                        let remaining: String = fchars[j + 1..].iter().collect();
                        if !remaining.is_empty() {
                            match remaining.parse::<i32>() {
                                Ok(f) => fd = Some(f),
                                Err(_) => {
                                    eprintln!(
                                        "{}: read: {}: invalid file descriptor specification",
                                        shell.error_prefix(),
                                        remaining
                                    );
                                    return 1;
                                }
                            }
                        } else {
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
                        }
                        break;
                    }
                    't' => {
                        i += 1;
                        if i < args.len() {
                            match args[i].parse::<f64>() {
                                Ok(t) if t < 0.0 => {
                                    eprintln!(
                                        "{}: read: {}: invalid timeout specification",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 2;
                                }
                                Ok(t) => timeout_secs = Some(t),
                                Err(_) => {
                                    eprintln!(
                                        "{}: read: {}: invalid timeout specification",
                                        shell.error_prefix(),
                                        args[i]
                                    );
                                    return 2;
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
            "-s" => {}
            "-e" => _use_readline = true,
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
                    match args[i].parse::<f64>() {
                        Ok(t) if t < 0.0 => {
                            eprintln!(
                                "{}: read: {}: invalid timeout specification",
                                shell.error_prefix(),
                                args[i]
                            );
                            return 2;
                        }
                        Ok(t) => timeout_secs = Some(t),
                        Err(_) => {
                            eprintln!(
                                "{}: read: {}: invalid timeout specification",
                                shell.error_prefix(),
                                args[i]
                            );
                            return 2;
                        }
                    }
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

    // Check if read fd is valid before attempting to read
    #[cfg(unix)]
    {
        if nix::fcntl::fcntl(read_fd, nix::fcntl::FcntlArg::F_GETFD).is_err() {
            // fd is closed/invalid — return 1 (failure)
            return 1;
        }
    }

    // Handle timeout: check if data is available within the timeout period
    #[cfg(unix)]
    if let Some(secs) = timeout_secs {
        use nix::poll::{PollFd, PollFlags, PollTimeout};
        use std::os::unix::io::BorrowedFd;
        let poll_fd = PollFd::new(
            unsafe { BorrowedFd::borrow_raw(read_fd) },
            PollFlags::POLLIN,
        );
        // -t 0 (exactly zero) is a polling check (returns 0 if data ready, 1 otherwise).
        // Very small positive values (e.g. 0.00001) are real timeouts that should
        // return 142 on expiry, so clamp them to at least 1ms.
        let is_poll = secs == 0.0;
        let timeout = if is_poll {
            PollTimeout::ZERO
        } else {
            let ms = (secs * 1000.0).ceil().max(1.0) as i32;
            PollTimeout::from(ms.min(i32::from(u16::MAX)) as u16)
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
                    return 0; // data available
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
        return if eof_reached { 1 } else { 0 };
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
        shell
            .arrays
            .insert(arr_name, fields.into_iter().map(Some).collect());
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
            //
            // POSIX rule: a sequence of zero or more IFS-whitespace chars,
            // followed by an optional IFS-non-whitespace char, followed by
            // zero or more IFS-whitespace chars, forms a *single* delimiter.
            //
            // Strategy: when we see ANY IFS character we scan ahead to
            // consume the entire delimiter sequence before pushing a field.
            if ifs_ws.contains(&ch) {
                // Started with IFS whitespace — skip all consecutive ws
                let mut scan = ci + 1;
                while scan < chars.len() && ifs_ws.contains(&chars[scan]) {
                    scan += 1;
                }
                // Check if a non-whitespace IFS char follows (merges into
                // a single delimiter with the surrounding whitespace).
                if scan < chars.len()
                    && ifs.contains(chars[scan])
                    && !ifs_ws.contains(&chars[scan])
                    && fields.len() < max_fields - 1
                {
                    // Absorb the non-ws IFS char
                    scan += 1;
                    // …and any trailing IFS whitespace after it
                    while scan < chars.len() && ifs_ws.contains(&chars[scan]) {
                        scan += 1;
                    }
                    // This whole span is ONE delimiter — push one field
                    fields.push(std::mem::take(&mut current));
                    last_escaped_pos = None;
                    ci = scan;
                    continue;
                }
                // Plain whitespace (no following non-ws IFS char):
                // only acts as delimiter if the current field is non-empty
                if !current.is_empty() {
                    fields.push(std::mem::take(&mut current));
                    last_escaped_pos = None;
                }
                ci = scan;
                continue;
            } else {
                // Started with IFS non-whitespace — always produces a
                // field boundary.  Also absorb trailing IFS whitespace.
                fields.push(std::mem::take(&mut current));
                last_escaped_pos = None;
                ci += 1;
                while ci < chars.len() && ifs_ws.contains(&chars[ci]) {
                    ci += 1;
                }
                continue;
            }
        } else {
            current.push(ch);
            ci += 1;
        }
    }
    // Strip trailing IFS characters from the last field.
    //
    // For single variable: strip ALL trailing IFS chars (whitespace and
    // non-whitespace delimiters) unconditionally.  Bash does not protect
    // escaped chars in the single-var case.
    //
    // For multiple variables: strip trailing IFS whitespace.  Then, if the
    // entire remaining content (after whitespace strip) is exactly ONE
    // non-whitespace IFS delimiter character and nothing else, strip it.
    // This matches bash: `a:b:c:` → z="c" (trailing `:` stripped because
    // the remainder `c:` after stripping `c` is just `:`), but `a:b:c::`
    // → z="c::" (remainder is more than a single delimiter).
    // For `:::` with 3 vars, remainder is `:` (single char) → stripped.
    // For `::::` with 3 vars, remainder is `::` → NOT stripped.
    let trim_limit = if var_names.len() == 1 {
        0
    } else {
        last_escaped_pos.map(|p| p + 1).unwrap_or(0)
    };
    let mut end = current.len();
    // Strip trailing IFS whitespace
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
    if var_names.len() == 1 {
        // For single variable: strip ALL trailing non-whitespace IFS
        // delimiters (and their preceding whitespace) in a loop.
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
    } else {
        // For multiple variables: strip a trailing non-whitespace IFS
        // delimiter ONLY when the remainder (after IFS-whitespace stripping)
        // contains NO internal IFS characters that act as delimiters.
        //
        // Specifically, do NOT strip if the remainder before the trailing
        // delimiter contains:
        //   - any non-ws IFS delimiters (e.g. `b:c:` keeps trailing `:`)
        //   - IFS whitespace BETWEEN non-IFS content (e.g. `a b:` with
        //     IFS=": " keeps trailing `:` because the space between `a`
        //     and `b` is an IFS delimiter)
        //
        // Examples with IFS=':' and 3 vars:
        //   remainder ":"    → strip → ""
        //   remainder "::"   → keep  → "::"
        //   remainder "c:"   → strip → "c"
        //   remainder "c::"  → keep  → "c::"
        //   remainder "b:c:" → keep  → "b:c:"
        // Examples with IFS=": " and 2 vars:
        //   remainder "a b:" → keep  → "a b:" (space is IFS between content)
        //   remainder "a:"   → strip → "a"
        if end > trim_limit
            && let Some(c) = current[..end].chars().last()
            && ifs.contains(c)
            && !ifs_ws.contains(&c)
        {
            let tentative = end - c.len_utf8();
            // Check if the remainder (before the trailing delimiter)
            // contains any non-ws IFS delimiters.
            let has_internal_non_ws_ifs = current[trim_limit..tentative]
                .chars()
                .any(|ch| ifs.contains(ch) && !ifs_ws.contains(&ch));
            // Check if the remainder has IFS whitespace BETWEEN
            // non-IFS content (i.e., IFS ws that acts as a word
            // separator, not just leading/trailing padding).
            let has_ifs_ws_between_content = {
                let inner = &current[trim_limit..tentative];
                // Trim leading and trailing IFS whitespace, then
                // check if any IFS whitespace remains inside.
                let trimmed = inner
                    .trim_start_matches(|ch: char| ifs_ws.contains(&ch))
                    .trim_end_matches(|ch: char| ifs_ws.contains(&ch));
                trimmed.chars().any(|ch| ifs_ws.contains(&ch))
            };
            if !has_internal_non_ws_ifs && !has_ifs_ws_between_content {
                end = tentative;
                // Also strip IFS whitespace before the removed delimiter
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

pub(super) fn builtin_mapfile(shell: &mut Shell, args: &[String]) -> i32 {
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
                        // Use the first character's raw byte value (not UTF-8 byte)
                        // so that $'\xff' gives delimiter 0xff, not 0xc3
                        args[i].chars().next().map(|c| c as u8).unwrap_or(0)
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

    // Split by delimiter byte
    // Use raw byte splitting to handle non-UTF-8 delimiters (like $'\xff')
    let mut start = 0;
    for pos in 0..input_data.len() {
        if input_data[pos] == delim {
            // Include delimiter in the line (will be stripped by -t if needed)
            let segment = &input_data[start..pos];
            // Convert bytes to string, treating each byte as a Latin-1 character
            // to preserve non-UTF-8 bytes (like bash does)
            let mut line: String = segment.iter().map(|&b| b as char).collect();
            line.push(delim as char);
            lines.push(line);
            start = pos + 1;
        }
    }
    // Remaining data (no trailing delimiter)
    if start < input_data.len() {
        let segment = &input_data[start..];
        let line: String = segment.iter().map(|&b| b as char).collect();
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
        arr.push(None);
    }
    for (idx, line) in lines.iter().enumerate() {
        let pos = origin + idx;
        if pos < arr.len() {
            arr[pos] = Some(line.clone());
        } else {
            arr.push(Some(line.clone()));
        }
    }

    0
}
