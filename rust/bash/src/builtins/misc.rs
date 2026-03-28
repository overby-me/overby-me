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

    // Validate variable name
    if !varname.chars().all(|c| c.is_alphanumeric() || c == '_')
        || varname
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        || varname.is_empty()
    {
        eprintln!(
            "{}: getopts: `{}': not a valid identifier",
            shell.error_prefix(),
            varname
        );
        return 1;
    }

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
                        if !suppress_errors {
                            let name = shell.positional.first().map(|s| s.as_str()).unwrap_or("bash");
                            eprintln!(
                                "{}: option requires an argument -- {}",
                                name,
                                opt_char
                            );
                        }
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
                if !suppress_errors {
                    let name = shell.positional.first().map(|s| s.as_str()).unwrap_or("bash");
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

pub(super) fn builtin_caller(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // stub
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

pub(super) fn builtin_jobs(_shell: &mut Shell, _args: &[String]) -> i32 {
    // Minimal stub — job control is not fully implemented
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
