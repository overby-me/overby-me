use super::*;

pub(super) fn builtin_test(shell: &mut Shell, args: &[String]) -> i32 {
    eval_test_expr(args, shell, "test", false)
}

pub(super) fn builtin_test_bracket(shell: &mut Shell, args: &[String]) -> i32 {
    // Remove trailing ]
    let args = if args.last().map(|s| s.as_str()) == Some("]") {
        &args[..args.len() - 1]
    } else {
        eprintln!("{}: [: missing `]'", shell.error_prefix());
        return 2;
    };
    eval_test_expr(args, shell, "[", false)
}

pub(super) fn test_paren_error(shell: &Shell, cmd_name: &str) {
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

pub(super) fn eval_test_expr(
    args: &[String],
    shell: &Shell,
    cmd_name: &str,
    sub_expr: bool,
) -> i32 {
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

pub(super) fn is_test_binop(s: &str) -> bool {
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

pub(super) fn is_test_unop(s: &str) -> bool {
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
