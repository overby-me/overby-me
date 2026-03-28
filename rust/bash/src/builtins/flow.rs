use super::*;

pub(super) fn builtin_break(shell: &mut Shell, args: &[String]) -> i32 {
    if shell.loop_depth == 0 {
        // In POSIX mode, break outside a loop is silently ignored
        if !shell.opt_posix {
            eprintln!(
                "{}: break: only meaningful in a `for', `while', or `until' loop",
                shell.error_prefix()
            );
        }
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

pub(super) fn builtin_continue(shell: &mut Shell, args: &[String]) -> i32 {
    if shell.loop_depth == 0 {
        if !shell.opt_posix {
            eprintln!(
                "{}: continue: only meaningful in a `for', `while', or `until' loop",
                shell.error_prefix()
            );
        }
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

pub(super) fn builtin_exit(shell: &mut Shell, args: &[String]) -> i32 {
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

pub(super) fn builtin_return(shell: &mut Shell, args: &[String]) -> i32 {
    let has_explicit_arg = args.first().and_then(|s| s.parse::<i32>().ok()).is_some();
    let code: i32 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(shell.last_status);
    // return is valid in functions, sourced scripts, and trap handlers
    if shell.local_scopes.is_empty() && !shell.sourcing && shell.in_trap_handler == 0 {
        // In a subshell (forked child), return acts like exit
        #[cfg(unix)]
        {
            let current_pid = unsafe { libc::getpid() } as u32;
            if current_pid != shell.top_level_pid {
                // We're in a subshell — exit with the code
                std::process::exit(code);
            }
        }
        // Not in a valid context for return
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

pub(super) fn builtin_shift(shell: &mut Shell, args: &[String]) -> i32 {
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

pub(super) fn builtin_logout(shell: &mut Shell, _args: &[String]) -> i32 {
    if !shell.login_shell {
        eprintln!(
            "{}: logout: not login shell: use `exit'",
            shell.error_prefix()
        );
        return 1;
    }
    std::process::exit(shell.last_status);
}
