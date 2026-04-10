use super::*;

pub(super) fn builtin_true(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

pub(super) fn builtin_false(_shell: &mut Shell, _args: &[String]) -> i32 {
    1
}

pub(super) fn builtin_getopts(shell: &mut Shell, args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("getopts: usage: getopts optstring name [arg ...]");
        return 2;
    }

    // Check for invalid options to getopts itself
    if args[0].starts_with('-') && args[0].len() > 1 && args[0] != "--" {
        eprintln!(
            "{}: getopts: {}: invalid option",
            shell.error_prefix(),
            args[0]
        );
        eprintln!("getopts: usage: getopts optstring name [arg ...]");
        return 2;
    }
    let raw_optstring = &args[0];
    let varname = &args[1];

    // Validate variable name — if invalid, we still process the option
    // (advancing OPTIND etc.) but use a dummy variable name for assignment.
    // Print error and use dummy so the rest of the function runs normally
    // (bash behavior: getopts with invalid varname still advances OPTIND).
    let invalid_varname = !varname.chars().all(|c| c.is_alphanumeric() || c == '_')
        || varname.chars().next().is_some_and(|c| c.is_ascii_digit())
        || varname.is_empty();
    if invalid_varname {
        eprintln!(
            "{}: getopts: `{}': not a valid identifier",
            shell.error_prefix(),
            varname
        );
    }
    // When the variable name is invalid, redirect assignments to a dummy
    // variable so OPTIND still advances.  We clean it up before returning.
    let varname = if invalid_varname {
        "_GETOPTS_DUMMY_VAR_"
    } else {
        varname.as_str()
    };

    // Check for silent error mode (leading ':') and OPTERR
    let opterr = shell
        .vars
        .get("OPTERR")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(1);
    let silent = raw_optstring.starts_with(':');
    let suppress_errors = silent || opterr == 0;
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
        if invalid_varname {
            shell.vars.remove("_GETOPTS_DUMMY_VAR_");
        }
        return 1;
    }

    let current = opt_args[optind - 1];

    // Check for end-of-options conditions
    if current == "--" {
        // Skip past '--' and signal end of options
        shell.set_var("OPTIND", (optind + 1).to_string());
        shell.vars.remove("_GETOPTS_OPTOFS");
        shell.set_var(varname, "?".to_string());
        if invalid_varname {
            shell.vars.remove("_GETOPTS_DUMMY_VAR_");
        }
        return 1;
    }

    if !current.starts_with('-') || current == "-" {
        shell.set_var(varname, "?".to_string());
        if invalid_varname {
            shell.vars.remove("_GETOPTS_DUMMY_VAR_");
        }
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
        if invalid_varname {
            shell.vars.remove("_GETOPTS_DUMMY_VAR_");
        }
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
                        if !suppress_errors {
                            let name = shell
                                .positional
                                .first()
                                .map(|s| s.as_str())
                                .unwrap_or("bash");
                            eprintln!("{}: option requires an argument -- {}", name, opt_char);
                        }
                        shell.set_var(varname, "?".to_string());
                        shell.vars.remove("OPTARG");
                    }
                    shell.set_var("OPTIND", (optind + 1).to_string());
                    shell.vars.remove("_GETOPTS_OPTOFS");
                    if invalid_varname {
                        shell.vars.remove("_GETOPTS_DUMMY_VAR_");
                        return 1;
                    }
                    return 0;
                }

                shell.set_var(varname, opt_char.to_string());
                if invalid_varname {
                    shell.vars.remove("_GETOPTS_DUMMY_VAR_");
                    return 1;
                }
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
            if invalid_varname {
                shell.vars.remove("_GETOPTS_DUMMY_VAR_");
                return 1;
            }
            0
        }
        None => {
            // Unknown option character
            if silent {
                shell.set_var(varname, "?".to_string());
                shell.set_var("OPTARG", opt_char.to_string());
            } else {
                if !suppress_errors {
                    let name = shell
                        .positional
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("bash");
                    eprintln!("{}: illegal option -- {}", name, opt_char);
                }
                shell.set_var(varname, "?".to_string());
                shell.vars.remove("OPTARG");
            }

            if idx + 1 < opt_chars.len() {
                shell.set_var("_GETOPTS_OPTOFS", (idx + 1).to_string());
            } else {
                shell.set_var("OPTIND", (optind + 1).to_string());
                shell.vars.remove("_GETOPTS_OPTOFS");
            }
            if invalid_varname {
                shell.vars.remove("_GETOPTS_DUMMY_VAR_");
                return 1;
            }
            0
        }
    }
}

pub(super) fn builtin_umask(shell: &mut Shell, args: &[String]) -> i32 {
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

        let Some(mask_str) = mask_arg else {
            return 0;
        };
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

        // Apply symbolic mask — full POSIX parsing.
        //
        // Grammar: symbolic_mode = clause { ',' clause }
        //          clause        = [who...] op_perm { op_perm }
        //          who           = 'u' | 'g' | 'o' | 'a'
        //          op_perm       = op [perm...]
        //          op            = '+' | '-' | '='
        //          perm          = 'r' | 'w' | 'x' | 'X' | 's' | 't' | 'u' | 'g' | 'o'
        //
        // When perm contains 'u', 'g', or 'o', it means "copy permissions
        // from that class".  'X' means set execute only if any execute bit
        // is already set in the current (intermediate) mask value.
        //
        // The umask stores the COMPLEMENT of allowed permissions, so:
        //   '+' means CLEAR those mask bits (allow the permission)
        //   '-' means SET those mask bits (deny the permission)
        //   '=' means SET all who bits, then CLEAR the specified ones
        let current = nix::sys::stat::umask(Mode::empty());
        nix::sys::stat::umask(current);
        let mut bits = current.bits();

        // Helper: extract the rwx triple for a given class from the mask.
        // Returns the ALLOWED permissions (complement of mask bits).
        let class_perms = |mask: u32, class: char| -> u32 {
            let allowed = !mask & 0o777;
            match class {
                'u' => (allowed >> 6) & 7,
                'g' => (allowed >> 3) & 7,
                'o' => allowed & 7,
                _ => 0,
            }
        };

        // Expand a 3-bit rwx value into the full 9-bit mask according to
        // who_mask (which specifies u/g/o positions).
        let expand_perm = |rwx: u32, who_mask: u32| -> u32 {
            let mut result = 0u32;
            if who_mask & 0o700 != 0 {
                result |= rwx << 6;
            }
            if who_mask & 0o070 != 0 {
                result |= rwx << 3;
            }
            if who_mask & 0o007 != 0 {
                result |= rwx;
            }
            result
        };

        for part in mask_str.split(',') {
            let chars: Vec<char> = part.chars().collect();
            let mut i = 0;

            // Parse who characters
            let mut who_mask = 0u32;
            while i < chars.len() && "ugoa".contains(chars[i]) {
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

            // Parse one or more op+perm sequences for this who
            while i < chars.len() && valid_op.contains(&chars[i]) {
                let op = chars[i];
                i += 1;

                // Collect permission bits for this operator
                let mut perm_rwx = 0u32; // 3-bit rwx value
                let mut has_x_cond = false; // 'X' seen
                while i < chars.len() && !valid_op.contains(&chars[i]) && chars[i] != ',' {
                    match chars[i] {
                        'r' => perm_rwx |= 4,
                        'w' => perm_rwx |= 2,
                        'x' => perm_rwx |= 1,
                        'X' => has_x_cond = true,
                        's' | 't' => { /* setuid/setgid/sticky — ignored for umask */ }
                        // Reference permissions: copy from another class
                        'u' | 'g' | 'o' => {
                            perm_rwx |= class_perms(bits, chars[i]);
                        }
                        _ => {}
                    }
                    i += 1;
                }

                // Handle conditional execute: set x only if any x bit is
                // currently allowed (i.e., cleared in the mask)
                if has_x_cond {
                    let allowed = !bits & 0o777;
                    if allowed & 0o111 != 0 {
                        perm_rwx |= 1;
                    }
                }

                let effective = expand_perm(perm_rwx, who_mask);
                match op {
                    '+' => bits &= !effective, // allow → clear mask bits
                    '-' => bits |= effective,  // deny → set mask bits
                    '=' => {
                        bits |= who_mask; // deny all for who
                        bits &= !effective; // then allow specified
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

pub(super) fn builtin_caller(shell: &mut Shell, args: &[String]) -> i32 {
    // caller [expr]
    // Without EXPR: returns "$line $filename"
    // With EXPR: returns "$line $subroutine $filename"
    //
    // Frame indexing (bash convention):
    //   FUNCNAME[0] = current function
    //   FUNCNAME[1] = caller of current function
    //   BASH_LINENO[N] = line number where FUNCNAME[N] was invoked
    //   BASH_SOURCE[N+1] = source file of the context that called FUNCNAME[N]
    //
    // caller    → BASH_LINENO[0], BASH_SOURCE[1]  (no subroutine name)
    // caller N  → BASH_LINENO[N], FUNCNAME[N+1], BASH_SOURCE[N+1]
    //             (requires FUNCNAME[N+1] to exist, else returns 1)
    //
    // Returns 1 if not in a function or EXPR is out of range.

    // Must be inside a function
    if shell.func_names.is_empty() {
        return 1;
    }

    let funcname_arr = shell.arrays.get("FUNCNAME").cloned().unwrap_or_default();
    let lineno_arr = shell.arrays.get("BASH_LINENO").cloned().unwrap_or_default();
    let source_arr = shell.arrays.get("BASH_SOURCE").cloned().unwrap_or_default();

    let get_elem = |arr: &[Option<String>], idx: usize| -> Option<String> {
        arr.get(idx).and_then(|v| v.as_ref()).cloned()
    };

    // Resolve filename: if BASH_SOURCE[idx] is missing/empty, use "NULL"
    // (bash uses "NULL" when there's no source file, e.g. in -c mode)
    let resolve_filename = |idx: usize| -> String {
        match get_elem(&source_arr, idx) {
            Some(f) if !f.is_empty() => f,
            _ => "NULL".to_string(),
        }
    };

    if args.is_empty() {
        // No EXPR: print "$line $filename"
        // line = BASH_LINENO[0] (where current function was called)
        // file = BASH_SOURCE[1] (source of the caller's context)
        let line = get_elem(&lineno_arr, 0).unwrap_or_else(|| "0".to_string());
        let filename = resolve_filename(1);
        println!("{} {}", line, filename);
        0
    } else {
        // With EXPR: go back EXPR frames
        let expr_str = &args[0];
        let frame: usize = match expr_str.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!(
                    "{}: caller: {}: invalid value",
                    shell.error_prefix(),
                    expr_str
                );
                return 1;
            }
        };

        // Need FUNCNAME[frame+1] to exist (the caller at this frame)
        if frame + 1 >= funcname_arr.len() {
            return 1;
        }

        let line = get_elem(&lineno_arr, frame).unwrap_or_else(|| "0".to_string());
        let subroutine = get_elem(&funcname_arr, frame + 1).unwrap_or_default();
        let filename = resolve_filename(frame + 1);
        println!("{} {} {}", line, subroutine, filename);
        0
    }
}

pub(super) fn builtin_alias(shell: &mut Shell, args: &[String]) -> i32 {
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

pub(super) fn builtin_unalias(shell: &mut Shell, args: &[String]) -> i32 {
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

pub(super) fn builtin_jobs(shell: &mut Shell, args: &[String]) -> i32 {
    use crate::interpreter::JobStatus;

    // Reap finished jobs before listing
    shell.reap_jobs();

    let mut show_pids = false;
    let mut show_running = false;
    let mut show_stopped = false;
    for arg in args {
        match arg.as_str() {
            "-l" | "-p" => show_pids = true,
            "-n" => {} // only show jobs that changed status — we don't track this yet
            "-r" => show_running = true,
            "-s" => show_stopped = true,
            _ => {}
        }
    }

    let total = shell.jobs.len();
    for (i, job) in shell.jobs.iter().enumerate() {
        // Filter by status if -r or -s specified
        if show_running && job.status != JobStatus::Running {
            continue;
        }
        if show_stopped && job.status != JobStatus::Stopped {
            continue;
        }

        // Determine current/previous job marker
        let marker = if i == total - 1 {
            "+"
        } else if i == total.saturating_sub(2) {
            "-"
        } else {
            " "
        };

        let status_str = match &job.status {
            JobStatus::Running => "Running",
            JobStatus::Done(0) => "Done",
            JobStatus::Done(_code) => {
                // For non-zero exit, bash shows "Done(N)" but only sometimes;
                // for simplicity, show "Done" for exit 0, "Exit N" for others
                // Actually bash shows "Done" for 0, "Exit N" for non-zero
                // but in basic job listing just "Done" is shown
                "Done"
            }
            JobStatus::Stopped => "Stopped",
        };

        // Bash format: [N]±  Status                     command
        // The status field is left-aligned in a ~27-char field
        if show_pids {
            println!(
                "[{}]{}  {} {} {}",
                job.number, marker, job.pid, status_str, job.command
            );
        } else {
            // Match bash's formatting: status is padded to ~27 chars
            println!(
                "[{}]{}  {:<27}{}",
                job.number, marker, status_str, job.command
            );
        }
    }

    // Remove jobs that have been reported as Done
    shell
        .jobs
        .retain(|j| matches!(j.status, JobStatus::Running | JobStatus::Stopped));

    0
}

pub(super) fn builtin_disown(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

pub(super) fn builtin_fg(shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("{}: fg: no job control", shell.error_prefix());
    1
}

pub(super) fn builtin_bg(shell: &mut Shell, _args: &[String]) -> i32 {
    eprintln!("{}: bg: no job control", shell.error_prefix());
    1
}
