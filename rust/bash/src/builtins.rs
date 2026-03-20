use crate::interpreter::Shell;
use std::collections::HashMap;

pub type BuiltinFn = fn(&mut Shell, &[String]) -> i32;

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
    map
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

    let _ = shell; // Shell not needed for echo but kept for API consistency
    print!("{}", output);
    if newline {
        println!();
    }
    0
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
                Some('0') => {
                    let mut val = 0u8;
                    for _ in 0..3 {
                        // Peek by cloning
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
                    for _ in 0..2 {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some(c) if c.is_ascii_hexdigit() => {
                                val = val * 16 + c.to_digit(16).unwrap() as u8;
                                chars = peek;
                            }
                            _ => break,
                        }
                    }
                    result.push(val as char);
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

fn builtin_printf(_shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("printf: usage: printf format [arguments]");
        return 1;
    }
    let format = &args[0];
    let fmt_args = &args[1..];
    let mut arg_idx = 0;

    let mut chars = format.chars().peekable();
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
                Some('0') => {
                    let mut val = 0u8;
                    for _ in 0..3 {
                        match chars.peek() {
                            Some(c @ '0'..='7') => {
                                val = val * 8 + (*c as u8 - b'0');
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    print!("{}", val as char);
                }
                Some(c) => print!("\\{}", c),
                None => print!("\\"),
            }
        } else if ch == '%' {
            match chars.next() {
                Some('s') => {
                    let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                    print!("{}", arg);
                    arg_idx += 1;
                }
                Some('d') | Some('i') => {
                    let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                    let n: i64 = arg.parse().unwrap_or(0);
                    print!("{}", n);
                    arg_idx += 1;
                }
                Some('x') => {
                    let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                    let n: i64 = arg.parse().unwrap_or(0);
                    print!("{:x}", n);
                    arg_idx += 1;
                }
                Some('o') => {
                    let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("0");
                    let n: i64 = arg.parse().unwrap_or(0);
                    print!("{:o}", n);
                    arg_idx += 1;
                }
                Some('q') => {
                    let arg = fmt_args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                    print!("{}", shell_escape(arg));
                    arg_idx += 1;
                }
                Some('%') => print!("%"),
                Some(c) => print!("%{}", c),
                None => print!("%"),
            }
        } else {
            print!("{}", ch);
        }
    }
    0
}

/// Shell-escape a string for use with %q in printf.
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
    // Use $'...' quoting for strings with special characters
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
                let bytes = c.to_string();
                for b in bytes.as_bytes() {
                    result.push_str(&format!("\\x{:02x}", b));
                }
            }
        }
    }
    result.push('\'');
    result
}

fn builtin_cd(shell: &mut Shell, args: &[String]) -> i32 {
    let target = if args.is_empty() {
        shell
            .vars
            .get("HOME")
            .cloned()
            .or_else(|| std::env::var("HOME").ok())
            .unwrap_or_else(|| "/".to_string())
    } else if args[0] == "-" {
        shell
            .vars
            .get("OLDPWD")
            .cloned()
            .or_else(|| std::env::var("OLDPWD").ok())
            .unwrap_or_else(|| ".".to_string())
    } else {
        args[0].clone()
    };

    let old = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    match std::env::set_current_dir(&target) {
        Ok(()) => {
            shell.vars.insert("OLDPWD".to_string(), old);
            let new = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            shell.vars.insert("PWD".to_string(), new.clone());
            unsafe { std::env::set_var("PWD", &new) };
            unsafe { std::env::set_var("OLDPWD", shell.vars.get("OLDPWD").unwrap()) };
            if !args.is_empty() && args[0] == "-" {
                println!("{}", new);
            }
            0
        }
        Err(e) => {
            eprintln!("bash: cd: {}: {}", target, e);
            1
        }
    }
}

fn builtin_pwd(_shell: &mut Shell, _args: &[String]) -> i32 {
    match std::env::current_dir() {
        Ok(dir) => {
            println!("{}", dir.display());
            0
        }
        Err(e) => {
            eprintln!("bash: pwd: {}", e);
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

    for arg in args {
        if arg == "-n" {
            continue; // Unexport - skip for now
        }
        if arg.starts_with('-') {
            continue;
        }
        if let Some(eq_pos) = arg.find('=') {
            let name = &arg[..eq_pos];
            let value = &arg[eq_pos + 1..];
            shell.vars.insert(name.to_string(), value.to_string());
            shell.exports.insert(name.to_string(), value.to_string());
            unsafe { std::env::set_var(name, value) };
        } else {
            // Export existing variable
            let value = shell
                .vars
                .get(arg)
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
    let mut names = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-v" => {}
            "-f" => unset_functions = true,
            _ => names.push(arg.as_str()),
        }
    }

    for name in names {
        if unset_functions {
            shell.functions.remove(name);
        } else {
            shell.vars.remove(name);
            shell.exports.remove(name);
            shell.arrays.remove(name);
            shell.namerefs.remove(name);
            unsafe { std::env::remove_var(name) };
        }
    }
    0
}

fn builtin_readonly(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        for name in &shell.readonly_vars {
            let val = shell.vars.get(name).cloned().unwrap_or_default();
            println!("declare -r {}=\"{}\"", name, val);
        }
        return 0;
    }

    for arg in args {
        if arg.starts_with('-') {
            continue;
        }
        if let Some(eq_pos) = arg.find('=') {
            let name = &arg[..eq_pos];
            let value = &arg[eq_pos + 1..];
            shell.vars.insert(name.to_string(), value.to_string());
            shell.readonly_vars.insert(name.to_string());
        } else {
            shell.readonly_vars.insert(arg.clone());
        }
    }
    0
}

fn builtin_local(shell: &mut Shell, args: &[String]) -> i32 {
    let mut flag_array = false;
    let mut flag_readonly = false;
    let mut flag_nameref = false;
    let mut names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'a' => flag_array = true,
                    'r' => flag_readonly = true,
                    'n' => flag_nameref = true,
                    '-' => {
                        // local - : save/restore shell options on function return (stub)
                    }
                    _ => {}
                }
            }
        } else {
            names.push(arg.clone());
        }
        i += 1;
    }

    for name_arg in &names {
        if let Some(eq_pos) = name_arg.find('=') {
            let name = &name_arg[..eq_pos];
            let value = &name_arg[eq_pos + 1..];
            if flag_nameref {
                shell.namerefs.insert(name.to_string(), value.to_string());
            } else if flag_array {
                // Parse (val1 val2 ...) syntax
                let arr = parse_array_literal(value);
                shell.arrays.insert(name.to_string(), arr);
            } else {
                shell.set_var(name, value.to_string());
            }
            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
        } else if flag_nameref {
            shell.namerefs.entry(name_arg.clone()).or_default();
        } else if flag_array {
            shell
                .arrays
                .entry(name_arg.clone())
                .or_default();
        } else {
            shell.vars.entry(name_arg.clone()).or_default();
        }
    }
    0
}

fn builtin_declare(shell: &mut Shell, args: &[String]) -> i32 {
    let mut flag_array = false;
    let mut flag_assoc = false; // -A stub
    let mut flag_print = false;
    let mut flag_functions = false;
    let mut flag_nameref = false;
    let mut flag_readonly = false;
    let mut flag_export = false;
    let mut flag_integer = false;
    let mut flag_global = false; // -g stub
    let mut names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with('-') && arg.len() > 1 && !arg.contains('=') {
            for ch in arg[1..].chars() {
                match ch {
                    'a' => flag_array = true,
                    'A' => flag_assoc = true,
                    'p' => flag_print = true,
                    'F' => flag_functions = true,
                    'n' => flag_nameref = true,
                    'r' => flag_readonly = true,
                    'x' => flag_export = true,
                    'i' => flag_integer = true,
                    'g' => flag_global = true,
                    _ => {}
                }
            }
        } else if arg.starts_with('+') && arg.len() > 1 {
            // +<flag> unsets attribute — skip flags but don't treat as name
        } else {
            names.push(arg.clone());
        }
        i += 1;
    }

    let _ = flag_assoc; // stub
    let _ = flag_global; // stub

    // declare -F: list function names
    if flag_functions {
        if names.is_empty() {
            for name in &shell.func_names {
                println!("declare -f {}", name);
            }
            // Also list functions from the functions map
            let mut fnames: Vec<&String> = shell.functions.keys().collect();
            fnames.sort();
            for name in fnames {
                if !shell.func_names.contains(name) {
                    println!("declare -f {}", name);
                }
            }
        } else {
            for name in &names {
                if shell.functions.contains_key(name.as_str()) || shell.func_names.contains(name) {
                    println!("declare -f {}", name);
                } else {
                    eprintln!("bash: declare: {}: not found", name);
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
                    let arr = &shell.arrays[name];
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                        .collect();
                    println!("declare -a {}=({})", name, elements.join(" "));
                } else {
                    println!("declare -- {}=\"{}\"", name, value);
                }
            }
            // Also print arrays not in vars
            let mut arr_names: Vec<&String> = shell.arrays.keys().collect();
            arr_names.sort();
            for name in arr_names {
                if !shell.vars.contains_key(name) {
                    let arr = &shell.arrays[name];
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                        .collect();
                    println!("declare -a {}=({})", name, elements.join(" "));
                }
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
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                        .collect();
                    println!("declare -a {}=({})", name, elements.join(" "));
                } else if let Some(value) = shell.vars.get(name) {
                    println!("declare -- {}=\"{}\"", name, value);
                } else {
                    eprintln!("bash: declare: {}: not found", name);
                    return 1;
                }
            }
        }
        return 0;
    }

    // Normal declare: set variables
    for name_arg in &names {
        if let Some(eq_pos) = name_arg.find('=') {
            let name = &name_arg[..eq_pos];
            let value = &name_arg[eq_pos + 1..];

            if flag_nameref {
                shell.namerefs.insert(name.to_string(), value.to_string());
            } else if flag_array {
                let arr = parse_array_literal(value);
                shell.arrays.insert(name.to_string(), arr);
            } else if flag_integer {
                // Evaluate as arithmetic
                let n = shell.eval_arith_expr(value);
                shell.set_var(name, n.to_string());
            } else {
                shell.set_var(name, value.to_string());
            }

            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
            if flag_export {
                let val = shell.get_var(name).cloned().unwrap_or_default();
                shell.exports.insert(name.to_string(), val.clone());
                unsafe { std::env::set_var(name, &val) };
            }
        } else {
            let name = name_arg.as_str();
            if flag_nameref {
                shell.namerefs.entry(name.to_string()).or_default();
            } else if flag_array {
                shell
                    .arrays
                    .entry(name.to_string())
                    .or_default();
            } else {
                shell.vars.entry(name.to_string()).or_default();
            }

            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
            if flag_export {
                let val = shell.get_var(name).cloned().unwrap_or_default();
                shell.exports.insert(name.to_string(), val.clone());
                unsafe { std::env::set_var(name, &val) };
            }
        }
    }
    0
}

/// Parse a bash array literal like `(val1 val2 val3)` into a Vec.
fn parse_array_literal(s: &str) -> Vec<String> {
    let trimmed = s.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    if inner.trim().is_empty() {
        return Vec::new();
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

fn builtin_set(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print all variables
        let mut vars: Vec<_> = shell.vars.iter().collect();
        vars.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in vars {
            println!("{}={}", key, value);
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
                        "pipefail" => shell.opt_pipefail = enable,
                        "errexit" => shell.opt_errexit = enable,
                        "nounset" => shell.opt_nounset = enable,
                        "xtrace" => shell.opt_xtrace = enable,
                        "noclobber" => shell.opt_noclobber = enable,
                        "noglob" => shell.opt_noglob = enable,
                        _ => {}
                    }
                } else if !enable {
                    // set +o - print settings in reusable format
                    println!("set {}o errexit", if shell.opt_errexit { "-" } else { "+" });
                    println!("set {}o nounset", if shell.opt_nounset { "-" } else { "+" });
                    println!("set {}o xtrace", if shell.opt_xtrace { "-" } else { "+" });
                    println!(
                        "set {}o pipefail",
                        if shell.opt_pipefail { "-" } else { "+" }
                    );
                }
            } else {
                for flag in flags.chars() {
                    match flag {
                        'e' => shell.opt_errexit = enable,
                        'u' => shell.opt_nounset = enable,
                        'x' => shell.opt_xtrace = enable,
                        'f' => shell.opt_noglob = enable,
                        'C' => shell.opt_noclobber = enable,
                        'n' => shell.opt_noexec = enable,
                        _ => {}
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
    0
}

fn builtin_shift(shell: &mut Shell, args: &[String]) -> i32 {
    let n: usize = args.first().and_then(|s| s.parse().ok()).unwrap_or(1);

    if shell.positional.len() > 1 {
        let available = shell.positional.len() - 1;
        if n > available {
            eprintln!("bash: shift: shift count out of range");
            return 1;
        }
        shell.positional.drain(1..=n);
    }
    0
}

fn builtin_exit(shell: &mut Shell, args: &[String]) -> i32 {
    let code: i32 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(shell.last_status);
    std::process::exit(code);
}

fn builtin_return(shell: &mut Shell, args: &[String]) -> i32 {
    let code: i32 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(shell.last_status);
    shell.returning = true;
    code
}

fn builtin_true(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

fn builtin_false(_shell: &mut Shell, _args: &[String]) -> i32 {
    1
}

fn builtin_test(_shell: &mut Shell, args: &[String]) -> i32 {
    eval_test_expr(args)
}

fn builtin_test_bracket(_shell: &mut Shell, args: &[String]) -> i32 {
    // Remove trailing ]
    let args = if args.last().map(|s| s.as_str()) == Some("]") {
        &args[..args.len() - 1]
    } else {
        eprintln!("bash: [: missing `]'");
        return 2;
    };
    eval_test_expr(args)
}

fn eval_test_expr(args: &[String]) -> i32 {
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
                return if eval_test_expr(&args[1..]) == 0 {
                    1
                } else {
                    0
                };
            }
            "-n" => return if !args[1].is_empty() { 0 } else { 1 },
            "-z" => return if args[1].is_empty() { 0 } else { 1 },
            "-e" | "-a" => {
                return if std::path::Path::new(&args[1]).exists() {
                    0
                } else {
                    1
                };
            }
            "-f" => {
                return if std::path::Path::new(&args[1]).is_file() {
                    0
                } else {
                    1
                };
            }
            "-d" => {
                return if std::path::Path::new(&args[1]).is_dir() {
                    0
                } else {
                    1
                };
            }
            "-L" | "-h" => {
                return if std::fs::symlink_metadata(&args[1])
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                };
            }
            "-r" => {
                return if std::path::Path::new(&args[1]).exists() {
                    0
                } else {
                    1
                };
            } // Simplified
            "-w" => {
                return if std::path::Path::new(&args[1]).exists() {
                    0
                } else {
                    1
                };
            } // Simplified
            "-x" => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    return if std::fs::metadata(&args[1])
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
                return if std::fs::metadata(&args[1])
                    .map(|m| m.len() > 0)
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                };
            }
            _ => {}
        }
    }

    if args.len() == 3 {
        match args[1].as_str() {
            "=" | "==" => return if args[0] == args[2] { 0 } else { 1 },
            "!=" => return if args[0] != args[2] { 0 } else { 1 },
            "-eq" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a == b { 0 } else { 1 };
            }
            "-ne" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a != b { 0 } else { 1 };
            }
            "-lt" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a < b { 0 } else { 1 };
            }
            "-le" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a <= b { 0 } else { 1 };
            }
            "-gt" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a > b { 0 } else { 1 };
            }
            "-ge" => {
                let a: i64 = args[0].parse().unwrap_or(0);
                let b: i64 = args[2].parse().unwrap_or(0);
                return if a >= b { 0 } else { 1 };
            }
            "-nt" => {
                // Newer than
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
                    _ => 1,
                };
            }
            "-ot" => {
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
                    _ => 1,
                };
            }
            _ => {}
        }
    }

    // Handle -a (and) and -o (or)
    for (i, arg) in args.iter().enumerate() {
        if arg == "-a" && i > 0 && i < args.len() - 1 {
            let left = eval_test_expr(&args[..i]);
            let right = eval_test_expr(&args[i + 1..]);
            return if left == 0 && right == 0 { 0 } else { 1 };
        }
    }
    for (i, arg) in args.iter().enumerate() {
        if arg == "-o" && i > 0 && i < args.len() - 1 {
            let left = eval_test_expr(&args[..i]);
            let right = eval_test_expr(&args[i + 1..]);
            return if left == 0 || right == 0 { 0 } else { 1 };
        }
    }

    // Handle ! prefix with 3+ args
    if args[0] == "!" {
        return if eval_test_expr(&args[1..]) == 0 {
            1
        } else {
            0
        };
    }

    1 // Default: false
}

fn builtin_read(shell: &mut Shell, args: &[String]) -> i32 {
    let mut prompt = String::new();
    let mut raw = false;
    let mut var_names = Vec::new();
    let mut array_name: Option<String> = None;
    let mut delim: Option<char> = None;
    let mut nchars: Option<usize> = None;
    let mut fd: Option<i32> = None;
    let mut i = 0;

    while i < args.len() {
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
                    delim = args[i].chars().next();
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
                    nchars = args[i].parse().ok();
                }
            }
            "-t" => {
                i += 1; // Skip timeout argument
            }
            "-u" => {
                i += 1;
                if i < args.len() {
                    fd = args[i].parse().ok();
                }
            }
            arg if !arg.starts_with('-') => {
                var_names.push(arg.to_string());
            }
            _ => {}
        }
        i += 1;
    }

    if var_names.is_empty() && array_name.is_none() {
        var_names.push("REPLY".to_string());
    }

    if !prompt.is_empty() {
        eprint!("{}", prompt);
    }

    let mut line = String::new();

    // Read input based on options
    if let Some(n) = nchars {
        // Read exactly n characters
        use std::io::Read as _;
        let mut buf = vec![0u8; n];
        let reader: Box<dyn std::io::Read> = if let Some(fd_num) = fd {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                Box::new(unsafe { std::fs::File::from_raw_fd(fd_num) })
            }
            #[cfg(not(unix))]
            {
                let _ = fd_num;
                Box::new(std::io::stdin())
            }
        } else {
            Box::new(std::io::stdin())
        };
        let mut reader = reader;
        match reader.read(&mut buf) {
            Ok(0) => return 1,
            Ok(bytes_read) => {
                line = String::from_utf8_lossy(&buf[..bytes_read]).to_string();
            }
            Err(_) => return 1,
        }
        // Prevent the File from being dropped and closing the fd if it came from -u
        if fd.is_some() {
            #[cfg(unix)]
            {
                // Leak the reader to prevent closing the fd
                let _ = Box::into_raw(Box::new(reader));
            }
        }
    } else if let Some(delim_char) = delim {
        // Read until delimiter
        use std::io::Read as _;
        let reader: Box<dyn std::io::Read> = if let Some(fd_num) = fd {
            #[cfg(unix)]
            {
                use std::os::unix::io::FromRawFd;
                Box::new(unsafe { std::fs::File::from_raw_fd(fd_num) })
            }
            #[cfg(not(unix))]
            {
                let _ = fd_num;
                Box::new(std::io::stdin())
            }
        } else {
            Box::new(std::io::stdin())
        };
        let mut reader = reader;
        let mut buf = [0u8; 1];
        loop {
            match reader.read(&mut buf) {
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
        if fd.is_some() {
            #[cfg(unix)]
            {
                let _ = Box::into_raw(Box::new(reader));
            }
        }
    } else if let Some(fd_num) = fd {
        // Read a line from a specific file descriptor
        #[cfg(unix)]
        {
            use std::io::BufRead;
            use std::os::unix::io::FromRawFd;
            let file = unsafe { std::fs::File::from_raw_fd(fd_num) };
            let mut reader = std::io::BufReader::new(file);
            match reader.read_line(&mut line) {
                Ok(0) => return 1,
                Err(_) => return 1,
                _ => {}
            }
            // Prevent closing the fd
            let inner = reader.into_inner();
            std::mem::forget(inner);
        }
        #[cfg(not(unix))]
        {
            let _ = fd_num;
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => return 1,
                Err(_) => return 1,
                _ => {}
            }
        }
    } else {
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => return 1, // EOF
            Err(_) => return 1,
            _ => {}
        }
    }

    // Remove trailing newline
    if line.ends_with('\n') {
        line.pop();
    }
    if line.ends_with('\r') {
        line.pop();
    }

    if !raw {
        // Handle backslash continuation
        line = line.replace("\\\n", "");
    }

    let ifs = shell
        .vars
        .get("IFS")
        .cloned()
        .unwrap_or_else(|| " \t\n".to_string());

    // Handle -a: read into array
    if let Some(arr_name) = array_name {
        let fields: Vec<String> = line
            .split(|c: char| ifs.contains(c))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        shell.arrays.insert(arr_name, fields);
        return 0;
    }

    if var_names.len() == 1 {
        shell.set_var(&var_names[0], line);
    } else {
        let fields: Vec<&str> = line
            .splitn(var_names.len(), |c: char| ifs.contains(c))
            .collect();
        for (j, name) in var_names.iter().enumerate() {
            let value = fields.get(j).unwrap_or(&"").trim().to_string();
            shell.set_var(name, value);
        }
    }

    0
}

fn builtin_eval(shell: &mut Shell, args: &[String]) -> i32 {
    let command = args.join(" ");
    shell.run_string(&command)
}

fn builtin_exec(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    let program = &args[0];
    let cmd_args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Set up environment
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

        // exec replaces the current process
        nix::unistd::execvp(&c_prog, &c_args).ok();
        eprintln!(
            "bash: exec: {}: {}",
            program,
            std::io::Error::last_os_error()
        );
        126
    }

    #[cfg(not(unix))]
    {
        eprintln!("bash: exec: not supported on this platform");
        1
    }
}

fn builtin_source(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("bash: source: filename argument required");
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

            let result = shell.run_string(&content);

            shell.positional = saved_positional;
            result
        }
        Err(e) => {
            eprintln!("bash: {}: {}", filename, e);
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
            "-p" => flag_p = true,
            "-a" | "-f" | "-P" => {}
            _ => names.push(arg.as_str()),
        }
    }

    for name in names {
        if flag_t {
            // Print type word only
            if shell.functions.contains_key(name) {
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
            if shell.functions.contains_key(name) {
                println!("{} is a function", name);
            } else if builtin_map.contains_key(name) {
                println!("{} is a shell builtin", name);
            } else if let Some(path) = find_in_path_opt(name) {
                println!("{} is {}", name, path);
            } else {
                eprintln!("bash: type: {}: not found", name);
                status = 1;
            }
        }
    }
    status
}

fn builtin_command(_shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    let mut show_type = false;
    let mut cmd_args = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-v" | "-V" => show_type = true,
            _ => cmd_args.push(arg.clone()),
        }
    }

    if show_type {
        let builtin_map = builtins();
        for name in &cmd_args {
            if builtin_map.contains_key(name.as_str()) {
                println!("{}", name);
            } else if let Some(path) = find_in_path_opt(name) {
                println!("{}", path);
            } else {
                return 1;
            }
        }
        return 0;
    }

    // Execute command bypassing functions
    if cmd_args.is_empty() {
        return 0;
    }

    let program = &cmd_args[0];
    let exec_args: Vec<String> = cmd_args[1..].to_vec();
    match std::process::Command::new(program)
        .args(&exec_args)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("bash: {}: {}", program, e);
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

fn builtin_hash(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // No-op for now
}

fn builtin_trap(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        // Print current traps
        for (signal, handler) in &shell.traps {
            println!("trap -- '{}' {}", handler, signal);
        }
        return 0;
    }

    if args.len() == 1 {
        // trap '' or trap - : list traps or reset
        if args[0] == "-l" {
            // List signal names
            println!("EXIT HUP INT QUIT TERM");
            return 0;
        }
        if args[0] == "-p" {
            for (signal, handler) in &shell.traps {
                println!("trap -- '{}' {}", handler, signal);
            }
            return 0;
        }
    }

    // trap [-p] 'handler' signal [signal...]
    let mut handler_idx = 0;
    let mut sig_start = 1;

    // Skip -p flag if present
    if args.first().map(|s| s.as_str()) == Some("-p") {
        if args.len() < 2 {
            for (signal, handler) in &shell.traps {
                println!("trap -- '{}' {}", handler, signal);
            }
            return 0;
        }
        handler_idx = 1;
        sig_start = 2;
    }

    if args.len() < sig_start + 1 {
        // Just a handler with no signals - might be a single signal to reset
        // If the first arg looks like a signal name, reset it
        if handler_idx == 0 && args.len() == 1 {
            return 0;
        }
    }

    let handler = &args[handler_idx];

    for sig in &args[sig_start..] {
        let signal = sig.to_uppercase();
        let signal = signal.strip_prefix("SIG").unwrap_or(&signal).to_string();

        if handler == "-" || handler.is_empty() {
            // Reset trap
            shell.traps.remove(&signal);
        } else {
            shell.traps.insert(signal, handler.clone());
        }
    }
    0
}

fn builtin_wait(_shell: &mut Shell, _args: &[String]) -> i32 {
    // TODO: Wait for background jobs
    0
}

fn builtin_kill(_shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;

        let mut signal = Signal::SIGTERM;
        let mut pids = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                let sig_name = &arg[1..];
                if let Ok(n) = sig_name.parse::<i32>() {
                    signal = Signal::try_from(n).unwrap_or(Signal::SIGTERM);
                } else {
                    let upper = sig_name.to_uppercase();
                    let upper = upper.strip_prefix("SIG").unwrap_or(&upper);
                    signal = match upper {
                        "HUP" => Signal::SIGHUP,
                        "INT" => Signal::SIGINT,
                        "QUIT" => Signal::SIGQUIT,
                        "KILL" => Signal::SIGKILL,
                        "TERM" => Signal::SIGTERM,
                        "STOP" => Signal::SIGSTOP,
                        "CONT" => Signal::SIGCONT,
                        "USR1" => Signal::SIGUSR1,
                        "USR2" => Signal::SIGUSR2,
                        _ => Signal::SIGTERM,
                    };
                }
            } else if let Ok(pid) = arg.parse::<i32>() {
                pids.push(pid);
            }
            i += 1;
        }

        let mut status = 0;
        for pid in pids {
            if signal::kill(Pid::from_raw(pid), signal).is_err() {
                eprintln!("bash: kill: ({}) - No such process", pid);
                status = 1;
            }
        }
        status
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        eprintln!("bash: kill: not supported on this platform");
        1
    }
}

fn builtin_umask(_shell: &mut Shell, args: &[String]) -> i32 {
    #[cfg(unix)]
    {
        use nix::sys::stat::Mode;

        if args.is_empty() {
            let current = nix::sys::stat::umask(Mode::empty());
            nix::sys::stat::umask(current);
            println!("{:04o}", current.bits());
            return 0;
        }

        if let Ok(mask) = u32::from_str_radix(args[0].trim_start_matches('0'), 8) {
            nix::sys::stat::umask(Mode::from_bits_truncate(mask));
            0
        } else {
            eprintln!("bash: umask: {}: invalid octal number", args[0]);
            1
        }
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        0
    }
}

fn builtin_getopts(shell: &mut Shell, args: &[String]) -> i32 {
    if args.len() < 2 {
        eprintln!("bash: getopts: usage: getopts optstring name [arg]");
        return 2;
    }

    let optstring = &args[0];
    let varname = &args[1];
    let opt_args = if args.len() > 2 {
        args[2..].to_vec()
    } else if shell.positional.len() > 1 {
        shell.positional[1..].to_vec()
    } else {
        vec![]
    };

    let optind: usize = shell
        .vars
        .get("OPTIND")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    if optind > opt_args.len() || optind == 0 {
        shell.vars.insert(varname.clone(), "?".to_string());
        return 1;
    }

    let current = &opt_args[optind - 1];
    if !current.starts_with('-') || current == "-" {
        shell.vars.insert(varname.clone(), "?".to_string());
        return 1;
    }

    let opt_char = current.chars().nth(1).unwrap_or('?');
    let opt_str = opt_char.to_string();

    if let Some(pos) = optstring.find(opt_char) {
        let needs_arg = optstring.chars().nth(pos + 1) == Some(':');
        if needs_arg {
            if current.len() > 2 {
                let optarg = &current[2..];
                shell.vars.insert("OPTARG".to_string(), optarg.to_string());
            } else if optind < opt_args.len() {
                shell
                    .vars
                    .insert("OPTARG".to_string(), opt_args[optind].clone());
                shell
                    .vars
                    .insert("OPTIND".to_string(), (optind + 2).to_string());
                shell.vars.insert(varname.clone(), opt_str);
                return 0;
            } else {
                eprintln!(
                    "bash: getopts: option requires an argument -- '{}'",
                    opt_char
                );
                shell.vars.insert(varname.clone(), "?".to_string());
                shell
                    .vars
                    .insert("OPTIND".to_string(), (optind + 1).to_string());
                return 0;
            }
        }
        shell.vars.insert(varname.clone(), opt_str);
    } else {
        eprintln!("bash: getopts: illegal option -- '{}'", opt_char);
        shell.vars.insert(varname.clone(), "?".to_string());
    }

    shell
        .vars
        .insert("OPTIND".to_string(), (optind + 1).to_string());
    0
}

fn builtin_let(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("bash: let: expression expected");
        return 1;
    }

    let mut result = 0i64;
    for expr in args {
        result = shell.eval_arith_expr(expr);
    }

    // let returns 1 if the last expression evaluates to 0, 0 otherwise
    if result == 0 { 1 } else { 0 }
}

fn builtin_mapfile(shell: &mut Shell, args: &[String]) -> i32 {
    let varname = args
        .last()
        .cloned()
        .unwrap_or_else(|| "MAPFILE".to_string());
    let mut lines = Vec::new();

    let stdin = std::io::stdin();
    use std::io::BufRead;
    for line in stdin.lock().lines() {
        match line {
            Ok(l) => lines.push(l),
            Err(_) => break,
        }
    }

    // Store as array
    shell.arrays.insert(varname.clone(), lines.clone());

    // Also store as indexed values for compatibility
    for (i, line) in lines.iter().enumerate() {
        shell
            .vars
            .insert(format!("{}[{}]", varname, i), line.clone());
    }
    shell
        .vars
        .insert(format!("{}[@]", varname), lines.join(" "));
    0
}

fn builtin_alias(_shell: &mut Shell, _args: &[String]) -> i32 {
    // TODO: Implement aliases
    0
}

fn builtin_unalias(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

fn builtin_enable(_shell: &mut Shell, _args: &[String]) -> i32 {
    0
}

fn builtin_shopt(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }

    let mut set = false;
    let mut unset = false;
    let mut opts = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-s" => set = true,
            "-u" => unset = true,
            "-q" => {}
            _ => opts.push(arg.as_str()),
        }
    }

    for opt in opts {
        match opt {
            "nullglob" => {
                if set {
                    shell.shopt_nullglob = true;
                } else if unset {
                    shell.shopt_nullglob = false;
                }
            }
            "extglob" => {
                if set {
                    shell.shopt_extglob = true;
                } else if unset {
                    shell.shopt_extglob = false;
                }
            }
            "inherit_errexit" => {
                if set {
                    shell.shopt_inherit_errexit = true;
                } else if unset {
                    shell.shopt_inherit_errexit = false;
                }
            }
            "nocasematch" => {
                if set {
                    shell.shopt_nocasematch = true;
                } else if unset {
                    shell.shopt_nocasematch = false;
                }
            }
            _ => {}
        }
    }
    0
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

fn builtin_compgen(_shell: &mut Shell, _args: &[String]) -> i32 {
    0 // No-op
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
