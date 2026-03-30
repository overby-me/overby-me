use super::*;

pub(super) fn builtin_export(shell: &mut Shell, args: &[String]) -> i32 {
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
                let redirs = shell
                    .func_redirections
                    .get(name.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let body_str = format_func_body_with_redirs(body, 0, redirs);
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
                shell
                    .arrays
                    .insert(name.to_string(), arr.into_iter().map(Some).collect());
                shell.exports.insert(name.to_string(), export_val.clone());
                unsafe { std::env::set_var(name, &export_val) };
            } else if shell.arrays.contains_key(name) {
                // Assigning scalar to existing array: set array[0]
                let final_value = value.to_string();
                if let Some(arr) = shell.arrays.get_mut(name) {
                    if arr.is_empty() {
                        arr.push(Some(final_value.clone()));
                    } else {
                        arr[0] = Some(final_value.clone());
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
            // Export existing variable (or mark unset variable for export)
            if let Some(value) = shell
                .vars
                .get(arg.as_str())
                .cloned()
                .or_else(|| std::env::var(arg).ok())
            {
                shell.exports.insert(arg.clone(), value.clone());
                unsafe { std::env::set_var(arg, &value) };
            } else {
                // Variable is unset — mark it for export without setting a value.
                // Use declared_unset + exports so the export attribute persists
                // and takes effect when the variable is later assigned.
                shell.declared_unset.insert(arg.clone());
                shell.exports.insert(arg.clone(), String::new());
            }
        }
    }
    0
}

pub(super) fn builtin_unset(shell: &mut Shell, args: &[String]) -> i32 {
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
                        arr[idx] = None;
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

pub(super) fn builtin_readonly(shell: &mut Shell, args: &[String]) -> i32 {
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
    if print_all && (args.is_empty() || print_mode || func_mode) {
        if func_mode {
            // Print readonly functions with their bodies
            let mut fnames: Vec<&String> = shell.readonly_funcs.iter().collect();
            fnames.sort();
            for name in fnames {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    let redirs = shell
                        .func_redirections
                        .get(name.as_str())
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    let body_str = format_func_body_with_redirs(body, 0, redirs);
                    println!("{} () \n{}", name, body_str);
                }
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
                    shell
                        .arrays
                        .insert(vname.to_string(), arr.into_iter().map(Some).collect());
                } else if shell.arrays.contains_key(vname) {
                    // Existing array: assign to element 0
                    if let Some(arr) = shell.arrays.get_mut(vname) {
                        if arr.is_empty() {
                            arr.push(Some(value.to_string()));
                        } else {
                            arr[0] = Some(value.to_string());
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

pub(super) fn builtin_local(shell: &mut Shell, args: &[String]) -> i32 {
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
                shell
                    .arrays
                    .insert(name.to_string(), arr.into_iter().map(Some).collect());
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

pub(super) fn builtin_declare(shell: &mut Shell, args: &[String]) -> i32 {
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
        // But allow names with = as function names for lookup (function a=2)
        if !flag_print
            && names
                .iter()
                .any(|n| n.contains('=') && !shell.functions.contains_key(n.as_str()))
        {
            eprintln!(
                "{}: declare: cannot use `-f' to make functions",
                shell.error_prefix()
            );
            return 1;
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
        // declare -xf: only exported functions (not implemented → print nothing)
        if flag_export && names.is_empty() {
            return 0;
        }
        let print_func = |name: &str, body: &CompoundCommand, shell: &Shell| {
            // Bash prints 'function' keyword when the name is not a valid identifier
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
        };
        if names.is_empty() {
            let mut fnames: Vec<&String> = shell.functions.keys().collect();
            fnames.sort();
            for name in fnames {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    print_func(name, body, shell);
                }
            }
        } else {
            let mut found = false;
            for name in &names {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    print_func(name, body, shell);
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
            // With -x flag, only list exported functions
            if flag_export {
                // No function export mechanism implemented yet — print nothing
                return 0;
            }
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
                    // declare -F name: just print the name (no declare prefix)
                    println!("{}", name);
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
                    let has_elements = arr.iter().any(|v| v.is_some());
                    if shell.declared_unset.contains(name) && !has_elements {
                        println!("declare {} {}", flags, name);
                    } else {
                        let elements: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                v.as_ref()
                                    .map(|s| format!("[{}]={}", i, quote_for_declare(s)))
                            })
                            .collect();
                        println!("declare {} {}=({})", flags, name, elements.join(" "));
                    }
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
                    if shell.declared_unset.contains(name) {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!("declare {} {}={}", flags, name, quote_for_declare(&value));
                    }
                }
            }
            // Also print declared-but-unset variables not in vars
            {
                let mut unset_names: Vec<&String> = shell
                    .declared_unset
                    .iter()
                    .filter(|n| !shell.vars.contains_key(n.as_str()))
                    .collect();
                unset_names.sort();
                for name in unset_names {
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
                    if shell.arrays.contains_key(name) {
                        let mut aflags = String::from("-a");
                        if shell.readonly_vars.contains(name) {
                            aflags.push('r');
                        }
                        println!("declare {} {}", aflags, name);
                    } else {
                        println!("declare {} {}", flags, name);
                    }
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
                        .filter_map(|(i, v)| {
                            v.as_ref()
                                .map(|s| format!("[{}]={}", i, quote_for_declare(s)))
                        })
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
                    let has_elements = arr.iter().any(|v| v.is_some());
                    if shell.declared_unset.contains(name) && !has_elements {
                        println!("declare {} {}", flags, name);
                    } else {
                        let elements: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                v.as_ref()
                                    .map(|s| format!("[{}]={}", i, quote_for_declare(s)))
                            })
                            .collect();
                        println!("declare {} {}=({})", flags, name, elements.join(" "));
                    }
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
                    if shell.declared_unset.contains(name) {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!("declare {} {}={}", flags, name, quote_for_declare(value));
                    }
                } else if shell.declared_unset.contains(name) {
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
                    if shell.arrays.contains_key(name) {
                        println!("declare -a{} {}", &flags[1..], name);
                    } else {
                        println!("declare {} {}", flags, name);
                    }
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
                    .filter_map(|(i, v)| v.as_ref().map(|s| (i, s)))
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
                // declare -n on a readonly variable: silently skip (bash behavior)
                if flag_nameref {
                    continue;
                }
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
                shell
                    .arrays
                    .insert(name.to_string(), arr.into_iter().map(Some).collect());
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
                if !shell.assoc_arrays.contains_key(name) {
                    shell.assoc_arrays.entry(name.to_string()).or_default();
                    shell.declared_unset.insert(name.to_string());
                }
            } else if flag_array {
                if !shell.arrays.contains_key(name) {
                    shell.arrays.entry(name.to_string()).or_default();
                    shell.declared_unset.insert(name.to_string());
                }
            } else if !shell.vars.contains_key(name) {
                // declare without = marks the variable as declared-but-unset
                // (don't insert into vars — bash distinguishes this from "")
                shell.declared_unset.insert(name.to_string());
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

pub fn parse_assoc_literal(s: &str) -> crate::interpreter::AssocArray {
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

pub(super) fn builtin_let(shell: &mut Shell, args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("{}: let: expression expected", shell.error_prefix());
        return 1;
    }

    // Skip leading -- (option terminator)
    let args = if args.first().map(|s| s.as_str()) == Some("--") {
        &args[1..]
    } else {
        args
    };

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

    // Drain arithmetic error flag — let handles errors via return status
    let had_error = crate::expand::take_arith_error();

    // let returns 1 if the last expression evaluates to 0 or had error, 0 otherwise
    if had_error || result == 0 { 1 } else { 0 }
}
