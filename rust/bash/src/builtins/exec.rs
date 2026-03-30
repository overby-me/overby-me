use super::*;

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

pub(super) fn builtin_source(shell: &mut Shell, args: &[String]) -> i32 {
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

    shell.source_file_error = false;
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

            shell.positional = saved_positional;
            result
        }
        Err(e) => {
            let msg = io_error_message(&e);
            eprintln!("{}: {}: {}", shell.error_prefix(), filename, msg);
            shell.source_file_error = true;
            1
        }
    }
}

pub(super) fn builtin_help(_shell: &mut Shell, _args: &[String]) -> i32 {
    // Minimal help builtin — just enough to not fail as "command not found"
    println!("GNU bash, version 5.3");
    0
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
            {
                println!("builtin");
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
                    println!("{}{} () \n{}", prefix, name, body_str);
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
            if builtin_map.contains_key(name) {
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
                    println!("{}{} () \n{}", prefix, name, body);
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
            eprintln!("{}: {}: {}", shell.error_prefix(), program, e);
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
    if args.is_empty() {
        // Print hash table (currently empty since we don't cache)
        if shell.hash_table.is_empty() {
            eprintln!("{}: hash: hash table empty", shell.error_prefix());
        } else {
            println!("hits\tcommand");
            for name in &shell.hash_order {
                if let Some((path, hits)) = shell.hash_table.get(name) {
                    println!("   {}\t{}", hits, path);
                }
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
                shell.hash_order.clear();
            }
            "-l" => {
                for name in &shell.hash_order {
                    let Some((path, _)) = shell.hash_table.get(name) else {
                        continue;
                    };
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
                shell.hash_order.retain(|n| n != &args[i]);
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
                    if !shell.hash_table.contains_key(&name) {
                        shell.hash_order.push(name.clone());
                    }
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
                    if !shell.hash_table.contains_key(name) {
                        shell.hash_order.push(name.to_string());
                    }
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
