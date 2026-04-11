use super::*;
use crate::interpreter::array_effective_len;

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
            if !arg.is_empty() {
                unsafe { std::env::remove_var(arg) };
            }
        } else if let Some(eq_pos) = arg.find('=') {
            let (name, value, is_append) = if eq_pos > 0 && arg.as_bytes()[eq_pos - 1] == b'+' {
                (&arg[..eq_pos - 1], &arg[eq_pos + 1..], true)
            } else {
                (&arg[..eq_pos], &arg[eq_pos + 1..], false)
            };
            if array_mode && value.starts_with('(') && value.ends_with(')') {
                // -a flag with (value): parse as array
                let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                // Case attrs should already be in the sets for export context
                shell.apply_case_attrs_to_array(name, &mut arr);
                let export_val = arr.iter().find_map(|v| v.clone()).unwrap_or_default();
                shell.arrays.insert(name.to_string(), arr);
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
    let mut unset_nameref = false;
    let mut names: Vec<(usize, &str)> = Vec::new(); // (arg_index, name)
    let mut parsing_opts = true;

    for (arg_idx, arg) in args.iter().enumerate() {
        if parsing_opts && arg.starts_with('-') && arg.len() > 1 {
            let opt = arg.as_str();
            match opt {
                "-v" => unset_variables = true,
                "-f" => unset_functions = true,
                "-n" => unset_nameref = true,
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
            names.push((arg_idx, arg.as_str()));
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
    for (arg_idx, name) in names {
        // Validate identifier only with explicit -v flag (not default mode)
        // A name containing '[' is only valid if it also ends with ']' (array ref).
        // Otherwise (e.g. word-split `a[$(echo` without closing `]`), reject it.
        if unset_variables {
            let has_bracket = name.contains('[');
            let is_valid_array_ref = has_bracket && name.ends_with(']');
            if (!has_bracket && !is_valid_identifier(name)) || (has_bracket && !is_valid_array_ref)
            {
                eprintln!(
                    "{}: unset: `{}': not a valid identifier",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
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
            let raw_idx_str = &name[bracket + 1..name.len() - 1];

            // If this argument had its `[` in a quoted context (e.g. unset 'a[$key]'),
            // re-expand the subscript to match bash behavior.
            // Bash treats quoted arguments as "strings" and re-evaluates their subscripts
            // (including $var, ${var}, and $(cmd)), while bare array references (unquoted
            // `[`) are already expanded by word expansion.
            //
            // When `assoc_expand_once` is ON, the subscript is used literally (no expansion),
            // matching bash's behavior where the shopt suppresses subscript re-expansion.
            // When `assoc_expand_once` is OFF, we use `expand_assoc_subscript` for full
            // expansion of $var, ${var}, $(cmd) etc.
            let expanded_idx;
            let idx_str = if shell.unset_quoted_subscript_args.contains(&arg_idx) {
                let aeo = shell
                    .shopt_options
                    .get("assoc_expand_once")
                    .copied()
                    .unwrap_or(false);
                if aeo {
                    // assoc_expand_once ON: use subscript literally (no expansion)
                    raw_idx_str
                } else {
                    // assoc_expand_once OFF: re-expand the subscript
                    expanded_idx = shell.expand_assoc_subscript(raw_idx_str);
                    expanded_idx.as_str()
                }
            } else {
                raw_idx_str
            };

            let resolved = shell.resolve_nameref(base);
            if idx_str == "@" || idx_str == "*" {
                // For indexed arrays: unset arr[@] / arr[*] clears all elements
                // but keeps the array variable (bash keeps it as an empty array).
                // UNLESS BASH_COMPAT <= 51, in which case the entire array is removed.
                // For associative arrays: unset assoc[@] / assoc[*] removes the
                // KEY "@" or "*" (does NOT clear all elements — bash treats these
                // as literal keys in associative array context).
                if shell.assoc_arrays.contains_key(&resolved) {
                    shell
                        .assoc_arrays
                        .get_mut(&resolved)
                        .map(|a| a.remove(idx_str));
                } else if shell.arrays.contains_key(&resolved) {
                    // Check BASH_COMPAT: if set to a version <= 51 (e.g. "51", "5.1"),
                    // unset array[@] removes the entire array variable (bash 5.1 behavior).
                    // Otherwise, just clear elements but keep the empty array.
                    let bash_compat = shell.vars.get("BASH_COMPAT").cloned().unwrap_or_default();
                    let compat_removes_array = if bash_compat.is_empty() {
                        false
                    } else if let Ok(n) = bash_compat.parse::<u32>() {
                        // Two-digit format: "51" means 5.1, "50" means 5.0, etc.
                        n <= 51
                    } else if bash_compat.contains('.') {
                        // Dotted format: "5.1", "5.0", etc.
                        let parts: Vec<&str> = bash_compat.splitn(2, '.').collect();
                        if let (Ok(major), Ok(minor)) = (
                            parts[0].parse::<u32>(),
                            parts.get(1).unwrap_or(&"0").parse::<u32>(),
                        ) {
                            major < 5 || (major == 5 && minor <= 1)
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if compat_removes_array {
                        // BASH_COMPAT <= 51: remove the entire array variable
                        shell.arrays.remove(&resolved);
                        shell.declared_unset.remove(&resolved);
                    } else {
                        // Default (current bash 5.3): clear elements, keep empty array
                        if let Some(arr) = shell.arrays.get_mut(&resolved) {
                            arr.clear();
                        }
                    }
                } else {
                    // Not an array — remove scalar
                    shell.vars.remove(&resolved);
                }
            } else if shell.assoc_arrays.contains_key(&resolved) {
                shell
                    .assoc_arrays
                    .get_mut(&resolved)
                    .map(|a| a.remove(idx_str));
            } else {
                let aeo = shell.is_array_expand_once();
                if aeo {
                    shell.arith_skip_comsub_expand = true;
                }
                let raw_idx = shell.eval_arith_expr(idx_str);
                shell.arith_skip_comsub_expand = false;
                // If subscript evaluation had an arithmetic error, skip the
                // unset but do NOT propagate the error (don't abort the script).
                // Bash continues execution after arithmetic errors in unset
                // subscripts — the variable is left unchanged.
                if crate::expand::take_arith_error() {
                    shell.last_status = 1;
                    continue;
                }
                // Check if base is a scalar (not an array) — unset scalar[n] where n!=0
                // should error with "not an array variable", but only if the variable
                // actually exists as a scalar.  Unsetting a subscript on a completely
                // unset variable is silently ignored (like bash).
                let is_indexed_array = shell.arrays.contains_key(&resolved);
                let is_assoc_array = shell.assoc_arrays.contains_key(&resolved);
                let is_scalar = shell.vars.contains_key(&resolved);
                if !is_indexed_array && !is_assoc_array && is_scalar {
                    // Scalar variable: unset var[0] unsets the scalar,
                    // but unset var[n] (n!=0) errors
                    if raw_idx == 0 {
                        if shell.readonly_vars.contains(&resolved) {
                            eprintln!(
                                "{}: unset: {}: cannot unset: readonly variable",
                                shell.error_prefix(),
                                resolved
                            );
                            status = 1;
                            continue;
                        }
                        shell.vars.remove(&resolved);
                        shell.exports.remove(&resolved);
                        if !resolved.is_empty() {
                            unsafe { std::env::remove_var(&resolved) };
                        }
                    } else {
                        eprintln!(
                            "{}: unset: {}: not an array variable",
                            shell.error_prefix(),
                            base
                        );
                        status = 1;
                    }
                    continue;
                }
                // Completely unset variable with subscript — silently ignore
                if !is_indexed_array && !is_assoc_array {
                    continue;
                }
                if raw_idx < 0
                    && let Some(arr) = shell.arrays.get(&resolved)
                    && raw_idx.abs() > array_effective_len(arr) as i64
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
                        let len = array_effective_len(arr) as i64;
                        (len + raw_idx).max(0) as usize
                    } else {
                        raw_idx as usize
                    };
                    if idx < arr.len() {
                        arr[idx] = None;
                    }
                }
            }
        } else if unset_nameref {
            // unset -n: remove the nameref itself, not the target
            if shell.readonly_vars.contains(name) {
                eprintln!(
                    "{}: unset: {}: cannot unset: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
            shell.namerefs.remove(name);
            shell.vars.remove(name);
            shell.exports.remove(name);
            shell.integer_vars.remove(name);
            shell.uppercase_vars.remove(name);
            shell.lowercase_vars.remove(name);
            shell.capitalize_vars.remove(name);
            if !name.is_empty() {
                unsafe { std::env::remove_var(name) };
            }
        } else if shell.namerefs.contains_key(name) {
            // unset through nameref: unset the target variable, keep the nameref
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
            shell.vars.remove(&resolved);
            shell.exports.remove(&resolved);
            shell.arrays.remove(&resolved);
            shell.assoc_arrays.remove(&resolved);
            shell.integer_vars.remove(&resolved);
            shell.uppercase_vars.remove(&resolved);
            shell.lowercase_vars.remove(&resolved);
            shell.capitalize_vars.remove(&resolved);
            // Don't remove the nameref itself — it stays
            if !resolved.is_empty() {
                unsafe { std::env::remove_var(&resolved) };
            }
        } else {
            // Regular variable unset (no -f or -v flag)
            // Bash behavior: try to unset variable first; if no variable
            // by that name exists, try to unset the function instead.
            if shell.readonly_vars.contains(name) {
                eprintln!(
                    "{}: unset: {}: cannot unset: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
            let had_var = shell.vars.contains_key(name)
                || shell.arrays.contains_key(name)
                || shell.assoc_arrays.contains_key(name)
                || shell.namerefs.contains_key(name);
            shell.vars.remove(name);
            shell.exports.remove(name);
            shell.arrays.remove(name);
            shell.assoc_arrays.remove(name);
            shell.namerefs.remove(name);
            shell.integer_vars.remove(name);
            shell.uppercase_vars.remove(name);
            shell.lowercase_vars.remove(name);
            shell.capitalize_vars.remove(name);
            if !name.is_empty() {
                unsafe { std::env::remove_var(name) };
            }
            // If no variable existed AND -v was not explicitly given,
            // fall through to unset the function (bash default behavior).
            // With explicit -v, only variables are targeted.
            if !had_var && !unset_variables {
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
                shell.func_names.retain(|n| n != name);
                let env_key = format!("BASH_FUNC_{}%%", name);
                unsafe { std::env::remove_var(&env_key) };
            }
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
    if print_all && (args.is_empty() || print_mode || func_mode || array_mode) {
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
        } else if array_mode {
            // readonly -a: list only readonly arrays
            let mut sorted: Vec<_> = shell.arrays.keys().collect();
            sorted.sort();
            for name in sorted {
                if !shell.readonly_vars.contains(name.as_str()) {
                    continue;
                }
                if let Some(arr) = shell.arrays.get(name) {
                    let has_elements = arr.iter().any(|v| v.is_some());
                    if has_elements {
                        let elements: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| v.as_ref().map(|s| (i, s)))
                            .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                            .collect();
                        if shell.opt_posix {
                            println!("readonly -a {}=({})", name, elements.join(" "));
                        } else {
                            println!("declare -ar {}=({})", name, elements.join(" "));
                        }
                    } else if shell.opt_posix {
                        println!("readonly -a {}", name);
                    } else {
                        println!("declare -ar {}", name);
                    }
                }
            }
        } else {
            let mut vnames: Vec<&String> = shell.readonly_vars.iter().collect();
            vnames.sort();
            for name in vnames {
                if shell.arrays.contains_key(name) {
                    // Readonly array: print with -ar flag and array values
                    if let Some(arr) = shell.arrays.get(name) {
                        let has_elements = arr.iter().any(|v| v.is_some());
                        if has_elements {
                            let elements: Vec<String> = arr
                                .iter()
                                .enumerate()
                                .filter_map(|(i, v)| v.as_ref().map(|s| (i, s)))
                                .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                                .collect();
                            println!("declare -ar {}=({})", name, elements.join(" "));
                        } else {
                            println!("declare -ar {}", name);
                        }
                    }
                } else {
                    let val = shell.vars.get(name).cloned().unwrap_or_default();
                    println!("declare -r {}=\"{}\"", name, val);
                }
            }
        }
        return 0;
    }

    let mut status = 0;
    for name in names {
        // readonly a[5] is not supported — individual array elements can't be readonly
        if !func_mode && name.contains('[') && !name.contains('=') {
            eprintln!(
                "{}: readonly: `{}': not a valid identifier",
                shell.error_prefix(),
                name
            );
            status = 1;
            continue;
        }
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
                    let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                    shell.apply_case_attrs_to_array(vname, &mut arr);
                    shell.arrays.insert(vname.to_string(), arr);
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

    // `local` with no args: print all local variables in declare format
    if args.is_empty() {
        if let Some(scope) = shell.local_scopes.last() {
            let mut sorted: Vec<_> = scope.keys().collect();
            sorted.sort();
            for name in sorted {
                // Build flags string
                let mut flags = String::from("-");
                if shell.arrays.contains_key(name.as_str()) {
                    flags.push('a');
                }
                if shell.assoc_arrays.contains_key(name.as_str()) {
                    flags.push('A');
                }
                if shell.integer_vars.contains(name.as_str()) {
                    flags.push('i');
                }
                if shell.readonly_vars.contains(name.as_str()) {
                    flags.push('r');
                }
                if shell.exports.contains_key(name.as_str()) {
                    flags.push('x');
                }
                // If only "-", use "--"
                if flags == "-" {
                    flags = "--".to_string();
                }

                if shell.arrays.contains_key(name.as_str()) {
                    let arr = &shell.arrays[name.as_str()];
                    let has_elements = arr.iter().any(|v| v.is_some());
                    if has_elements {
                        let elems: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                v.as_ref()
                                    .map(|s| format!("[{}]={}", i, quote_for_declare(s)))
                            })
                            .collect();
                        println!("declare {} {}=({})", flags, name, elems.join(" "));
                    } else {
                        println!("declare {} {}", flags, name);
                    }
                } else if shell.assoc_arrays.contains_key(name.as_str()) {
                    let map = &shell.assoc_arrays[name.as_str()];
                    if map.is_empty() {
                        println!("declare {} {}", flags, name);
                    } else {
                        let mut pairs: Vec<_> = map.iter().collect();
                        pairs.sort_by_key(|(k, _)| (*k).clone());
                        let elems: Vec<String> = pairs
                            .iter()
                            .map(|(k, v)| {
                                format!("[{}]={}", quote_assoc_key(k), quote_for_declare(v))
                            })
                            .collect();
                        println!("declare {} {}=({} )", flags, name, elems.join(" "));
                    }
                } else if let Some(val) = shell.vars.get(name.as_str()) {
                    println!("declare {} {}={}", flags, name, quote_for_declare(val));
                } else {
                    println!("declare {} {}", flags, name);
                }
            }
        }
        return 0;
    }

    let mut flag_array = false;
    let mut flag_assoc = false;
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
                    'A' => flag_assoc = true,
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
                // Detect circular nameref: if the target's base name matches
                // the variable name, it's circular (e.g., local -n a=a[0])
                let target_base = if let Some(bracket) = value.find('[') {
                    &value[..bracket]
                } else {
                    value
                };
                if target_base == name {
                    eprintln!(
                        "{}: local: warning: {}: circular name reference",
                        shell.error_prefix(),
                        name
                    );
                } else {
                    shell.namerefs.insert(name.to_string(), value.to_string());
                }
            } else if flag_assoc {
                let trimmed_val = value.trim();
                if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                    let map = crate::builtins::parse_assoc_literal(value);
                    shell.assoc_arrays.insert(name.to_string(), map);
                } else {
                    // Bare value without (): local -A name=value
                    // Bash assigns the value to key "0", not implicit key-value pairing.
                    let mut map = crate::interpreter::AssocArray::default();
                    map.insert("0".to_string(), value.to_string());
                    shell.assoc_arrays.insert(name.to_string(), map);
                }
            } else if flag_array {
                let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                shell.apply_case_attrs_to_array(name, &mut arr);
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
                // No value provided — just mark as nameref (no circular risk)
                shell.namerefs.entry(name_arg.clone()).or_default();
            } else if flag_assoc {
                shell.assoc_arrays.entry(name_arg.clone()).or_default();
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
    // Rebuild dynamic assoc arrays (BASH_CMDS, BASH_ALIASES) from backing stores
    shell.sync_dynamic_assoc_arrays();
    // Use the actual command name (declare/typeset/local) for error messages.
    // Clone to avoid holding a borrow on shell across mutable operations.
    let cmd_name = shell
        .current_builtin
        .as_deref()
        .unwrap_or("declare")
        .to_string();
    let cmd_name = cmd_name.as_str();
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
    // Names already consumed by +n processing (should not be re-processed)
    let mut nameref_consumed: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                        eprintln!(
                            "{}: {}: -{}: invalid option",
                            shell.error_prefix(),
                            cmd_name,
                            ch
                        );
                        eprintln!(
                            "{}: usage: {} [-aAfFgiIlnrtux] [name[=value] ...] or {} -p [-aAfFilnrtux] [name ...]",
                            cmd_name, cmd_name, cmd_name
                        );
                        return 2;
                    }
                }
            }
        } else if arg.starts_with('+') && arg.len() > 1 {
            // +<flag> unsets attribute
            let mut unset_array = false;
            let mut unset_assoc = false;
            let mut unset_integer = false;
            let mut unset_export = false;
            let mut unset_uppercase = false;
            let mut unset_lowercase = false;
            let mut unset_capitalize = false;
            let mut unset_trace = false;
            let mut unset_nameref = false;
            for ch in arg[1..].chars() {
                match ch {
                    'r' => flag_unset_readonly = true,
                    'a' => unset_array = true,
                    'A' => unset_assoc = true,
                    'i' => unset_integer = true,
                    'x' => unset_export = true,
                    'u' => unset_uppercase = true,
                    'l' => unset_lowercase = true,
                    'c' => unset_capitalize = true,
                    't' => unset_trace = true,
                    'n' => unset_nameref = true,
                    _ => {}
                }
            }
            // Check for +A or +a to flag array destruction attempt
            if unset_assoc || unset_array {
                // Process names to emit error
                let remaining_names: Vec<String> = args[i + 1..]
                    .iter()
                    .filter(|a| !a.starts_with('-') && !a.starts_with('+'))
                    .cloned()
                    .collect();
                for rname in &remaining_names {
                    let pure = rname.split('=').next().unwrap_or(rname);
                    if shell.assoc_arrays.contains_key(pure) || shell.arrays.contains_key(pure) {
                        // Readonly check takes precedence over "cannot destroy"
                        if shell.readonly_vars.contains(pure) {
                            eprintln!(
                                "{}: {}: {}: readonly variable",
                                shell.error_prefix(),
                                cmd_name,
                                pure
                            );
                        } else {
                            eprintln!(
                                "{}: {}: {}: cannot destroy array variables in this way",
                                shell.error_prefix(),
                                cmd_name,
                                pure
                            );
                        }
                        return 1;
                    }
                }
            }
            // Apply attribute removal to names that follow
            let remaining_names: Vec<String> = args[i + 1..]
                .iter()
                .filter(|a| !a.starts_with('-') && !a.starts_with('+'))
                .cloned()
                .collect();
            for rname in &remaining_names {
                let pure = rname.split('=').next().unwrap_or(rname);
                if unset_integer {
                    shell.integer_vars.remove(pure);
                }
                if unset_export {
                    shell.exports.remove(pure);
                    if !pure.is_empty() {
                        unsafe { std::env::remove_var(pure) };
                    }
                }
                if unset_uppercase {
                    shell.uppercase_vars.remove(pure);
                }
                if unset_lowercase {
                    shell.lowercase_vars.remove(pure);
                }
                if unset_capitalize {
                    shell.capitalize_vars.remove(pure);
                }
                if unset_trace {
                    shell.traced_funcs.remove(pure);
                }
                if unset_nameref {
                    // typeset +n foo=value: first assign value through the
                    // nameref (to the target variable), then remove the
                    // nameref attribute.  foo retains the target name as its
                    // plain string value.
                    if let Some(eq) = rname.find('=') {
                        let val = &rname[eq + 1..];
                        if let Some(target) = shell.namerefs.get(pure).cloned() {
                            // Assign through nameref to the target variable
                            shell.set_var(&target, val.to_string());
                        }
                        // After removing nameref, foo's value is the old
                        // target name (not the assigned value)
                        if let Some(target) = shell.namerefs.remove(pure) {
                            shell.vars.insert(pure.to_string(), target);
                        }
                        // Mark as consumed so the main declare body doesn't
                        // re-process this name=value and overwrite our result
                        nameref_consumed.insert(rname.clone());
                    } else {
                        // typeset +n foo (no value): just remove nameref,
                        // set foo to the old target name
                        if let Some(target) = shell.namerefs.remove(pure) {
                            shell.vars.insert(pure.to_string(), target);
                        }
                        nameref_consumed.insert(rname.clone());
                    }
                }
            }
        } else if !nameref_consumed.contains(arg) {
            names.push(arg.clone());
        }
        i += 1;
    }

    let _ = flag_global; // stub

    // Check for -f combined with other attributes (invalid)
    if flag_func_body && !names.is_empty() {
        if flag_array {
            eprintln!("{}: {}: -a: invalid option", shell.error_prefix(), cmd_name);
            return 1;
        }
        if flag_integer {
            eprintln!("{}: {}: -i: invalid option", shell.error_prefix(), cmd_name);
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
                "{}: {}: cannot use `-f' to make functions",
                shell.error_prefix(),
                cmd_name
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
                    "{}: {}: {}: readonly function",
                    shell.error_prefix(),
                    cmd_name,
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
            // Extract the base name (before any '[') for validation.
            // Content inside [...] brackets can contain any characters
            // (arithmetic expressions like `a[7 + 8]`, assoc keys, etc.)
            let base_for_check = if let Some(bracket_pos) = pure_name.find('[') {
                // When assoc_expand_once is ON and the variable is an existing
                // associative array, bash finds the FIRST `]` after `[` without
                // tracking `[` depth — this allows keys containing `[`
                // (e.g. "foo[bar") because `m[foo[bar]` finds `]` at the end
                // as the first `]` seen.  But `foo[foo]bar]` finds the first
                // `]` after `foo`, leaving stray `bar]` → rejected.
                //
                // Without AEO (or for non-assoc variables), use depth-based
                // bracket tracking: `[` increments, `]` decrements.
                let base_name = &pure_name[..bracket_pos];
                let is_existing_assoc = shell.assoc_arrays.contains_key(base_name);
                let aeo = shell.is_array_expand_once();

                let close_pos = if aeo && is_existing_assoc {
                    // Find the FIRST `]` after `[`, ignoring nested `[`.
                    // m[foo[bar] → first `]` at index 10 (end) → valid, key="foo[bar"
                    // foo[foo]bar] → first `]` at index 7 → stray "bar]" → rejected
                    pure_name[bracket_pos + 1..]
                        .find(']')
                        .map(|p| bracket_pos + 1 + p)
                } else {
                    let mut depth: i32 = 1;
                    let mut found = None;
                    for (i, ch) in pure_name[bracket_pos + 1..].char_indices() {
                        match ch {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    found = Some(bracket_pos + 1 + i);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    found
                };
                match close_pos {
                    Some(cp) if cp + 1 != pure_name.len() => {
                        // Stray characters after the closing ']' (e.g. A[]])
                        // Show full argument including =value, matching bash
                        eprintln!(
                            "{}: {}: `{}': not a valid identifier",
                            shell.error_prefix(),
                            cmd_name,
                            name
                        );
                        status = 1;
                        continue;
                    }
                    None => {
                        // No matching ']' found (unbalanced brackets)
                        eprintln!(
                            "{}: {}: `{}': not a valid identifier",
                            shell.error_prefix(),
                            cmd_name,
                            name
                        );
                        status = 1;
                        continue;
                    }
                    _ => {} // valid bracket matching
                }
                &pure_name[..bracket_pos]
            } else {
                pure_name
            };
            if !pure_name.is_empty()
                && !base_for_check
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_')
                || base_for_check
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '/')
            {
                eprintln!(
                    "{}: {}: `{}': not a valid identifier",
                    shell.error_prefix(),
                    cmd_name,
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
            let mut status = 0;
            for name in &names {
                if let Some(body) = shell.functions.get(name.as_str()) {
                    print_func(name, body, shell);
                } else {
                    if flag_print {
                        eprintln!(
                            "{}: {}: {}: not found",
                            shell.error_prefix(),
                            cmd_name,
                            name
                        );
                    }
                    status = 1;
                }
            }
            return status;
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
    // When -p is combined with a type flag (-a, -A, -x, -r, -i, -n) and no
    // names, fall through to the type-specific listing sections below instead
    // of printing everything here.
    let has_type_filter = flag_array
        || flag_assoc
        || flag_export
        || flag_readonly
        || flag_integer
        || flag_nameref
        || flag_uppercase
        || flag_lowercase
        || flag_capitalize;
    if flag_print && !(names.is_empty() && has_type_filter) {
        if names.is_empty() {
            // Print all variables (no type filter)
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
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
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
                    if shell.lowercase_vars.contains(name) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name) {
                        flags.push('c');
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
                    if shell.lowercase_vars.contains(name) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name) {
                        flags.push('c');
                    }
                    if flags == "-" {
                        flags.push('-');
                    }
                    if shell.arrays.contains_key(name) {
                        let mut aflags = String::from("-a");
                        if shell.integer_vars.contains(name) {
                            aflags.push('i');
                        }
                        if shell.lowercase_vars.contains(name) {
                            aflags.push('l');
                        }
                        if shell.readonly_vars.contains(name) {
                            aflags.push('r');
                        }
                        if shell.uppercase_vars.contains(name) {
                            aflags.push('u');
                        }
                        if shell.exports.contains_key(name) {
                            aflags.push('x');
                        }
                        if shell.capitalize_vars.contains(name) {
                            aflags.push('c');
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
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
                    }
                    let arr = &shell.arrays[name];
                    let has_elements = arr.iter().any(|v| v.is_some());
                    if has_elements {
                        let elements: Vec<String> = arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                v.as_ref()
                                    .map(|s| format!("[{}]={}", i, quote_for_declare(s)))
                            })
                            .collect();
                        println!("declare {} {}=({})", flags, name, elements.join(" "));
                    } else if shell.declared_unset.contains(name) {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!("declare {} {}=()", flags, name);
                    }
                }
            }
            // Also print associative arrays
            let mut assoc_names: Vec<&String> = shell.assoc_arrays.keys().collect();
            assoc_names.sort();
            for name in assoc_names {
                let assoc = &shell.assoc_arrays[name];
                let mut flags = String::from("-A");
                if shell.integer_vars.contains(name.as_str()) {
                    flags.push('i');
                }
                if shell.lowercase_vars.contains(name.as_str()) {
                    flags.push('l');
                }
                if shell.readonly_vars.contains(name.as_str()) {
                    flags.push('r');
                }
                if shell.uppercase_vars.contains(name.as_str()) {
                    flags.push('u');
                }
                if shell.exports.contains_key(name.as_str()) {
                    flags.push('x');
                }
                if shell.capitalize_vars.contains(name.as_str()) {
                    flags.push('c');
                }
                if assoc.is_empty() && shell.declared_unset.contains(name) {
                    println!("declare {} {}", flags, name);
                } else if assoc.is_empty() {
                    println!("declare {} {}=()", flags, name);
                } else {
                    let elements: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| format!("[{}]={}", quote_assoc_key(k), quote_for_declare(v)))
                        .collect();
                    println!("declare {} {}=({} )", flags, name, elements.join(" "));
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
                    let mut flags = String::from("-a");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
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
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
                    }
                    if assoc.is_empty() && shell.declared_unset.contains(name.as_str()) {
                        println!("declare {} {}", flags, name);
                    } else if assoc.is_empty() {
                        println!("declare {} {}=()", flags, name);
                    } else {
                        let elements: Vec<String> = assoc
                            .iter()
                            .map(|(k, v)| {
                                format!("[{}]={}", quote_assoc_key(k), quote_for_declare(v))
                            })
                            .collect();
                        println!("declare {} {}=({} )", flags, name, elements.join(" "));
                    }
                } else if let Some(value) = shell.vars.get(name) {
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
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
                    if shell.lowercase_vars.contains(name.as_str()) {
                        flags.push('l');
                    }
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.uppercase_vars.contains(name.as_str()) {
                        flags.push('u');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if shell.capitalize_vars.contains(name.as_str()) {
                        flags.push('c');
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
                    eprintln!(
                        "{}: {}: {}: not found",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                    return 1;
                }
            }
        }
        return 0;
    }

    // declare -x with no names: list exports
    if flag_export && names.is_empty() {
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
    // Skip when -a or -A is also set — let the array listing sections handle it
    if flag_readonly && names.is_empty() && !flag_array && !flag_assoc {
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
    if flag_integer && names.is_empty() {
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
    // (also handles `declare -pa` when flag_print + flag_array are both set)
    if flag_array && names.is_empty() {
        let mut sorted: Vec<_> = shell.arrays.keys().collect();
        sorted.sort();
        for name in sorted {
            // When -r flag is also set (declare -ar), only list readonly arrays
            if flag_readonly && !shell.readonly_vars.contains(name.as_str()) {
                continue;
            }
            if let Some(arr) = shell.arrays.get(name) {
                let mut flags = String::from("-a");
                if shell.integer_vars.contains(name.as_str()) {
                    flags.push('i');
                }
                if shell.lowercase_vars.contains(name.as_str()) {
                    flags.push('l');
                }
                if shell.readonly_vars.contains(name.as_str()) {
                    flags.push('r');
                }
                if shell.uppercase_vars.contains(name.as_str()) {
                    flags.push('u');
                }
                if shell.exports.contains_key(name.as_str()) {
                    flags.push('x');
                }
                if shell.capitalize_vars.contains(name.as_str()) {
                    flags.push('c');
                }
                let has_elements = arr.iter().any(|v| v.is_some());
                if has_elements {
                    let elements: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| v.as_ref().map(|s| (i, s)))
                        .map(|(i, v)| format!("[{}]=\"{}\"", i, v))
                        .collect();
                    println!("declare {} {}=({})", flags, name, elements.join(" "));
                } else if shell.declared_unset.contains(name.as_str()) {
                    println!("declare {} {}", flags, name);
                } else {
                    println!("declare {} {}=()", flags, name);
                }
            }
        }
        return 0;
    }

    // declare -n with no names: list all namerefs
    if flag_nameref && names.is_empty() {
        let mut sorted: Vec<_> = shell.namerefs.iter().collect();
        sorted.sort_by_key(|(k, _)| k.to_string());
        for (name, target) in sorted {
            println!("declare -n {}=\"{}\"", name, target);
        }
        return 0;
    }

    // declare -A with no names: list all associative arrays
    // (also handles `declare -pA` when flag_print + flag_assoc are both set)
    if flag_assoc && names.is_empty() {
        let mut sorted: Vec<_> = shell.assoc_arrays.keys().collect();
        sorted.sort();
        for name in sorted {
            if let Some(assoc) = shell.assoc_arrays.get(name) {
                let mut flags = String::from("-A");
                if shell.integer_vars.contains(name.as_str()) {
                    flags.push('i');
                }
                if shell.lowercase_vars.contains(name.as_str()) {
                    flags.push('l');
                }
                if shell.readonly_vars.contains(name.as_str()) {
                    flags.push('r');
                }
                if shell.uppercase_vars.contains(name.as_str()) {
                    flags.push('u');
                }
                if shell.exports.contains_key(name.as_str()) {
                    flags.push('x');
                }
                if shell.capitalize_vars.contains(name.as_str()) {
                    flags.push('c');
                }
                if assoc.is_empty() && shell.declared_unset.contains(name.as_str()) {
                    println!("declare {} {}", flags, name);
                } else if assoc.is_empty() {
                    println!("declare {} {}=()", flags, name);
                } else {
                    let elements: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| format!("[{}]=\"{}\"", quote_assoc_key(k), v))
                        .collect();
                    println!("declare {} {}=({} )", flags, name, elements.join(" "));
                }
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

            // Check for subscripted name: name[key]=value
            if let Some(bracket) = name.find('[') {
                let base = &name[..bracket];
                let idx_str = if name.ends_with(']') {
                    &name[bracket + 1..name.len() - 1]
                } else {
                    &name[bracket + 1..]
                };

                // When -a or -A flag is set, bash strips the subscript from the name
                // e.g., `declare -a e[10]="(test)"` → treat as `declare -a e="(test)"`
                if flag_array || flag_assoc {
                    let stripped_name = base;
                    let resolved_base = shell.resolve_nameref(stripped_name);
                    if shell.readonly_vars.contains(&resolved_base) {
                        eprintln!(
                            "{}: {}: {}: readonly variable",
                            shell.error_prefix(),
                            cmd_name,
                            resolved_base
                        );
                        status = 1;
                        continue;
                    }
                    if make_local {
                        shell.declare_local(stripped_name);
                    }
                    if flag_assoc {
                        let trimmed_val = value.trim();
                        if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                            let map = parse_assoc_literal(value);
                            shell.assoc_arrays.insert(resolved_base.clone(), map);
                        } else {
                            // Bare value without (): assign to key "0"
                            let mut map = crate::interpreter::AssocArray::default();
                            map.insert("0".to_string(), value.to_string());
                            shell.assoc_arrays.insert(resolved_base.clone(), map);
                        }
                    } else if value.starts_with('(') && value.ends_with(')') {
                        let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                        // Apply case transforms using local flags (attrs not yet in sets)
                        if flag_uppercase || flag_lowercase || flag_capitalize {
                            for val in arr.iter_mut().flatten() {
                                *val = if flag_uppercase {
                                    val.to_uppercase()
                                } else if flag_lowercase {
                                    val.to_lowercase()
                                } else {
                                    crate::interpreter::capitalize_string(val)
                                };
                            }
                        }
                        shell.arrays.insert(resolved_base.clone(), arr);
                    } else {
                        // Scalar value: assign to element at the given subscript
                        let aeo = shell.is_array_expand_once();
                        if aeo {
                            shell.arith_skip_comsub_expand = true;
                        }
                        let raw_idx = shell.eval_arith_expr(idx_str);
                        shell.arith_skip_comsub_expand = false;
                        // If subscript evaluation had an arithmetic error, skip the
                        // assignment but do NOT propagate the error (don't abort the
                        // script). Bash continues execution after arithmetic errors
                        // in declare subscripts.
                        if crate::expand::take_arith_error() {
                            status = 1;
                            continue;
                        }
                        let val = if flag_integer {
                            shell.eval_arith_expr(value).to_string()
                        } else {
                            value.to_string()
                        };
                        let val = if flag_uppercase {
                            val.to_uppercase()
                        } else if flag_lowercase {
                            val.to_lowercase()
                        } else if flag_capitalize {
                            crate::interpreter::capitalize_string(&val)
                        } else {
                            val
                        };
                        let arr = shell.arrays.entry(resolved_base.clone()).or_default();
                        let idx = if raw_idx < 0 {
                            let len = array_effective_len(arr) as i64;
                            (len + raw_idx).max(0) as usize
                        } else {
                            raw_idx as usize
                        };
                        while arr.len() <= idx {
                            arr.push(None);
                        }
                        arr[idx] = Some(val);
                    }
                    if flag_readonly {
                        shell.readonly_vars.insert(stripped_name.to_string());
                    }
                    if flag_export {
                        let val = shell.get_var(stripped_name).unwrap_or_default();
                        shell.exports.insert(stripped_name.to_string(), val.clone());
                        unsafe { std::env::set_var(stripped_name, &val) };
                    }
                    if flag_integer {
                        shell.integer_vars.insert(stripped_name.to_string());
                    }
                    continue;
                }

                let resolved_base = shell.resolve_nameref(base);

                // Check readonly on the base name
                if shell.readonly_vars.contains(&resolved_base) {
                    eprintln!(
                        "{}: {}: {}: readonly variable",
                        shell.error_prefix(),
                        cmd_name,
                        resolved_base
                    );
                    status = 1;
                    continue;
                }

                if shell.assoc_arrays.contains_key(&resolved_base) {
                    // Assoc array element assignment
                    let val = if flag_integer || shell.integer_vars.contains(&resolved_base) {
                        shell.eval_arith_expr(value).to_string()
                    } else {
                        value.to_string()
                    };
                    if is_append {
                        let is_int = flag_integer || shell.integer_vars.contains(&resolved_base);
                        shell
                            .assoc_arrays
                            .entry(resolved_base)
                            .or_default()
                            .entry(idx_str.to_string())
                            .and_modify(|v| {
                                if is_int {
                                    let existing: i64 = v.parse().unwrap_or(0);
                                    let addend: i64 = val.parse().unwrap_or(0);
                                    *v = (existing + addend).to_string();
                                } else {
                                    v.push_str(&val);
                                }
                            })
                            .or_insert(val);
                    } else {
                        shell
                            .assoc_arrays
                            .entry(resolved_base)
                            .or_default()
                            .insert(idx_str.to_string(), val);
                    }
                } else {
                    // Indexed array element assignment
                    let aeo = shell.is_array_expand_once();
                    if aeo {
                        shell.arith_skip_comsub_expand = true;
                    }
                    let raw_idx = shell.eval_arith_expr(idx_str);
                    shell.arith_skip_comsub_expand = false;
                    // If subscript evaluation had an arithmetic error, skip the
                    // assignment but do NOT propagate the error (don't abort the
                    // script). Bash continues execution after arithmetic errors
                    // in declare subscripts.
                    if crate::expand::take_arith_error() {
                        status = 1;
                        continue;
                    }
                    let is_int = flag_integer || shell.integer_vars.contains(&resolved_base);
                    let val = if is_int {
                        shell.eval_arith_expr(value).to_string()
                    } else {
                        value.to_string()
                    };
                    let val = if flag_uppercase {
                        val.to_uppercase()
                    } else if flag_lowercase {
                        val.to_lowercase()
                    } else if flag_capitalize {
                        crate::interpreter::capitalize_string(&val)
                    } else {
                        val
                    };
                    let arr = shell.arrays.entry(resolved_base).or_default();
                    let idx = if raw_idx < 0 {
                        let len = array_effective_len(arr) as i64;
                        (len + raw_idx).max(0) as usize
                    } else {
                        raw_idx as usize
                    };
                    while arr.len() <= idx {
                        arr.push(None);
                    }
                    if is_append {
                        if is_int {
                            let existing: i64 = arr[idx]
                                .as_deref()
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0);
                            let addend: i64 = val.parse().unwrap_or(0);
                            arr[idx] = Some((existing + addend).to_string());
                        } else {
                            match &mut arr[idx] {
                                Some(s) => s.push_str(&val),
                                None => arr[idx] = Some(val),
                            }
                        }
                    } else {
                        arr[idx] = Some(val);
                    }
                }
                if flag_readonly {
                    shell.readonly_vars.insert(base.to_string());
                }
                if flag_export {
                    let val = shell.get_var(base).unwrap_or_default();
                    shell.exports.insert(base.to_string(), val.clone());
                    unsafe { std::env::set_var(base, &val) };
                }
                continue;
            }

            // Check if variable is readonly
            if shell.readonly_vars.contains(name) && !make_local {
                // declare -n on a readonly variable: silently skip (bash behavior)
                if flag_nameref {
                    continue;
                }
                eprintln!(
                    "{}: {}: {}: readonly variable",
                    shell.error_prefix(),
                    cmd_name,
                    name
                );
                status = 1;
                continue;
            }

            if make_local {
                shell.declare_local(name);
            }

            if flag_nameref {
                // Detect circular nameref: if the target's base name matches
                // the variable name, it's circular (e.g., declare -n a=a[0])
                let target_base = if let Some(bracket) = value.find('[') {
                    &value[..bracket]
                } else {
                    value
                };
                if target_base == name {
                    eprintln!(
                        "{}: {}: warning: {}: circular name reference",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                } else {
                    shell.vars.remove(name);
                    shell.namerefs.insert(name.to_string(), value.to_string());
                }
            } else if flag_assoc {
                let trimmed_val = value.trim();
                if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                    // Compound assignment: declare -A name=(...)
                    let map = parse_assoc_literal(value);
                    shell.assoc_arrays.insert(name.to_string(), map);
                } else {
                    // Bare value without (): declare -A name=value
                    // Bash assigns the value to key "0", not implicit key-value pairing.
                    let mut map = crate::interpreter::AssocArray::default();
                    map.insert("0".to_string(), value.to_string());
                    shell.assoc_arrays.insert(name.to_string(), map);
                }
                if flag_integer {
                    shell.integer_vars.insert(name.to_string());
                }
            } else if flag_array {
                // Only parse as compound assignment if the value is wrapped
                // in (...).  A bare value like `declare -a foo="[0]=bar"`
                // (where "[0]=bar" came from expansion) should be treated as
                // a scalar string assigned to element [0], not as subscript
                // syntax.  Bash only interprets [idx]=val inside (...).
                let trimmed_val = value.trim();
                if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                    let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                    if flag_integer {
                        // Evaluate each element as arithmetic when -i is set
                        let evaluated: Vec<Option<String>> = arr
                            .into_iter()
                            .map(|v| v.map(|s| shell.eval_arith_expr(&s).to_string()))
                            .collect();
                        shell.arrays.insert(name.to_string(), evaluated);
                        shell.integer_vars.insert(name.to_string());
                    } else {
                        // Apply case transforms directly using local flags, since
                        // uppercase_vars/lowercase_vars aren't populated yet
                        if flag_uppercase || flag_lowercase || flag_capitalize {
                            for val in arr.iter_mut().flatten() {
                                *val = if flag_uppercase {
                                    val.to_uppercase()
                                } else if flag_lowercase {
                                    val.to_lowercase()
                                } else {
                                    crate::interpreter::capitalize_string(val)
                                };
                            }
                        }
                        shell.arrays.insert(name.to_string(), arr);
                    }
                } else {
                    // Non-compound value: assign as scalar to element [0]
                    let val = if flag_integer {
                        shell.integer_vars.insert(name.to_string());
                        shell.eval_arith_expr(value).to_string()
                    } else {
                        let mut v = value.to_string();
                        if flag_uppercase {
                            v = v.to_uppercase();
                        } else if flag_lowercase {
                            v = v.to_lowercase();
                        } else if flag_capitalize {
                            v = crate::interpreter::capitalize_string(&v);
                        }
                        v
                    };
                    if is_append {
                        let arr = shell.arrays.entry(name.to_string()).or_default();
                        if arr.is_empty() {
                            arr.push(Some(val));
                        } else {
                            match &mut arr[0] {
                                Some(s) => s.push_str(&val),
                                None => arr[0] = Some(val),
                            }
                        }
                    } else {
                        let arr = vec![Some(val)];
                        shell.arrays.insert(name.to_string(), arr);
                    }
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
                // Apply to current scalar value
                if let Some(v) = shell.vars.get(name).cloned() {
                    shell.vars.insert(name.to_string(), v.to_uppercase());
                }
                // Apply to array elements
                if let Some(arr) = shell.arrays.get_mut(name) {
                    for val in arr.iter_mut().flatten() {
                        *val = val.to_uppercase();
                    }
                }
            }
            if flag_lowercase {
                shell.lowercase_vars.insert(name.to_string());
                shell.uppercase_vars.remove(name);
                shell.capitalize_vars.remove(name);
                if let Some(v) = shell.vars.get(name).cloned() {
                    shell.vars.insert(name.to_string(), v.to_lowercase());
                }
                // Apply to array elements
                if let Some(arr) = shell.arrays.get_mut(name) {
                    for val in arr.iter_mut().flatten() {
                        *val = val.to_lowercase();
                    }
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
                // Apply to array elements
                if let Some(arr) = shell.arrays.get_mut(name) {
                    for val in arr.iter_mut().flatten() {
                        *val = capitalize_string(val);
                    }
                }
            }
        } else {
            // Strip [subscript] from name in no-value declare.
            // Bash always strips subscripts in this context:
            // e.g., `declare -A chaff[200]` → treat as `declare -A chaff`
            // e.g., `declare -r c[100]` → treat as `declare -r c` (creates array c)
            let (name, had_subscript) = if name_arg.contains('[') {
                let base = name_arg.split('[').next().unwrap_or(name_arg.as_str());
                (base, true)
            } else {
                (name_arg.as_str(), false)
            };
            // Can't remove readonly attribute
            if flag_unset_readonly && shell.readonly_vars.contains(name) {
                eprintln!(
                    "{}: {}: {}: readonly variable",
                    shell.error_prefix(),
                    cmd_name,
                    name
                );
                status = 1;
                continue;
            }

            if make_local {
                shell.declare_local(name);
            }
            if flag_nameref {
                // `typeset -n foo` (no value): use foo's current value as the
                // nameref target, then remove it from regular vars.  This matches
                // bash: `foo=bar; typeset -n foo` → foo is a nameref to "bar".
                let target = shell.vars.remove(name).unwrap_or_default();
                // Detect circular nameref
                let target_base = if let Some(bracket) = target.find('[') {
                    &target[..bracket]
                } else {
                    target.as_str()
                };
                if !target.is_empty() && target_base == name {
                    eprintln!(
                        "{}: {}: warning: {}: circular name reference",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                    // Put the value back since we're not creating the nameref
                    shell.vars.insert(name.to_string(), target);
                } else {
                    shell.namerefs.insert(name.to_string(), target);
                }
            } else if flag_assoc {
                // Error if trying to convert an existing indexed array to assoc
                if shell.arrays.contains_key(name) {
                    // Bash error format for type conversion:
                    // - Inside function with -g flag: two errors:
                    //   "{prefix}: {func}: {name}: cannot convert..."
                    //   "{prefix}: {cmd_name}: {name}: cannot convert..."
                    // - Inside function without -g: one error:
                    //   "{prefix}: {cmd_name}: {name}: cannot convert..."
                    // - At top level: one error:
                    //   "{prefix}: {name}: cannot convert..."
                    if flag_global && let Some(func) = shell.func_names.last() {
                        eprintln!(
                            "{}: {}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            func,
                            name
                        );
                    }
                    if shell.func_names.is_empty() && !flag_global {
                        eprintln!(
                            "{}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            name
                        );
                    } else {
                        eprintln!(
                            "{}: {}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            cmd_name,
                            name
                        );
                    }
                    status = 1;
                } else if !shell.assoc_arrays.contains_key(name) {
                    if make_local {
                        // In a function, `declare -A name` creates a new empty
                        // local associative array — do NOT carry over the global
                        // scalar value as element ["0"].
                        shell.vars.remove(name);
                        let new_map = crate::interpreter::AssocArray::default();
                        shell.assoc_arrays.insert(name.to_string(), new_map);
                        shell.declared_unset.insert(name.to_string());
                    } else {
                        let mut new_map = crate::interpreter::AssocArray::default();
                        // Convert existing scalar value to element [0]
                        if let Some(val) = shell.vars.remove(name) {
                            new_map.insert("0".to_string(), val);
                        } else {
                            shell.declared_unset.insert(name.to_string());
                        }
                        shell.assoc_arrays.insert(name.to_string(), new_map);
                    }
                }
            } else if flag_array {
                // Error if trying to convert an existing assoc array to indexed
                if shell.assoc_arrays.contains_key(name) {
                    // Bash error format for type conversion:
                    // - Inside function with -g flag: two errors:
                    //   "{prefix}: {func}: {name}: cannot convert..."
                    //   "{prefix}: {cmd_name}: {name}: cannot convert..."
                    // - Inside function without -g: one error:
                    //   "{prefix}: {cmd_name}: {name}: cannot convert..."
                    // - At top level: one error:
                    //   "{prefix}: {name}: cannot convert..."
                    if flag_global && let Some(func) = shell.func_names.last() {
                        eprintln!(
                            "{}: {}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            func,
                            name
                        );
                    }
                    if shell.func_names.is_empty() && !flag_global {
                        eprintln!(
                            "{}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            name
                        );
                    } else {
                        eprintln!(
                            "{}: {}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            cmd_name,
                            name
                        );
                    }
                    status = 1;
                } else if !shell.arrays.contains_key(name) {
                    if make_local {
                        // In a function, `declare -a name` creates a new empty
                        // local indexed array — do NOT carry over the global
                        // scalar value as element [0].
                        shell.vars.remove(name);
                        shell.arrays.insert(name.to_string(), vec![]);
                        shell.declared_unset.insert(name.to_string());
                    } else {
                        // Convert existing scalar value to array[0]
                        if let Some(val) = shell.vars.remove(name) {
                            shell.arrays.insert(name.to_string(), vec![Some(val)]);
                        } else {
                            shell.arrays.entry(name.to_string()).or_default();
                            shell.declared_unset.insert(name.to_string());
                        }
                    }
                }
            } else if had_subscript && !shell.arrays.contains_key(name) {
                // declare -r c[100] (with subscript, no -a flag): create empty array
                shell.arrays.entry(name.to_string()).or_default();
                shell.declared_unset.insert(name.to_string());
            } else if !shell.vars.contains_key(name) {
                // declare without = marks the variable as declared-but-unset
                // (don't insert into vars — bash distinguishes this from "")
                shell.declared_unset.insert(name.to_string());
            }

            if flag_integer {
                let was_integer = shell.integer_vars.contains(name);
                shell.integer_vars.insert(name.to_string());
                // When -i is newly applied to an existing array, re-evaluate
                // all elements as arithmetic expressions.  This handles
                // `declare -ai arr=(1+1 2+2 3+3)` where the compound
                // assignment was executed before the integer flag was set.
                if !was_integer && let Some(arr) = shell.arrays.get(name).cloned() {
                    let evaluated: Vec<Option<String>> = arr
                        .into_iter()
                        .map(|v| v.map(|s| shell.eval_arith_expr(&s).to_string()))
                        .collect();
                    shell.arrays.insert(name.to_string(), evaluated);
                }
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
        inner.split('\x1F').filter(|e| !e.is_empty()).collect()
    } else {
        vec![inner]
    };
    let has_sep = inner.contains('\x1F');

    // First pass: check if any entry has [key]=value subscript syntax.
    // If none do, use bash 5.3 implicit key-value pair mode.
    let has_subscripted = entries.iter().any(|entry| {
        let t = entry.trim();
        t.starts_with('[') && t.contains("]=")
    });

    if !has_subscripted {
        // Implicit key-value pair mode: treat consecutive bare words as
        // alternating key-value pairs.  E.g. `(a 1 b 2)` → a=1, b=2.
        let mut bare_words: Vec<String> = Vec::new();
        if has_sep {
            // When entries are \x1F-separated, each entry came from a single
            // compound assignment element that was already expanded (variable
            // expansion, etc.).  Bash does NOT word-split inside compound
            // assignment elements, so each entry is a single word regardless
            // of whitespace.  E.g. `declare -A v=( $foo 3 )` where foo="1 2"
            // produces entries ["1 2", "3"] → key "1 2", value "3".
            for entry in &entries {
                let trimmed = entry.trim();
                if !trimmed.is_empty() {
                    // Strip one level of quotes if present (from quoted elements
                    // like `"key with spaces"` or `'literal'`)
                    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
                        || (trimmed.starts_with('\'')
                            && trimmed.ends_with('\'')
                            && trimmed.len() >= 2)
                    {
                        bare_words.push(trimmed[1..trimmed.len() - 1].to_string());
                    } else {
                        bare_words.push(trimmed.to_string());
                    }
                }
            }
        } else {
            // No \x1F separator — content is a flat string that needs to be
            // tokenized by whitespace (respecting quotes).
            for entry in &entries {
                let mut rest = entry.trim();
                while !rest.is_empty() {
                    // Handle quoted words
                    if rest.starts_with('"') {
                        if let Some(end) = rest[1..].find('"') {
                            bare_words.push(rest[1..1 + end].to_string());
                            rest = rest[2 + end..].trim_start();
                        } else {
                            bare_words.push(rest[1..].to_string());
                            rest = "";
                        }
                    } else if rest.starts_with('\'') {
                        if let Some(end) = rest[1..].find('\'') {
                            bare_words.push(rest[1..1 + end].to_string());
                            rest = rest[2 + end..].trim_start();
                        } else {
                            bare_words.push(rest[1..].to_string());
                            rest = "";
                        }
                    } else {
                        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        bare_words.push(rest[..end].to_string());
                        rest = rest[end..].trim_start();
                    }
                }
            }
        }
        // Pair them as key-value
        let mut i = 0;
        while i + 1 < bare_words.len() {
            map.insert(bare_words[i].clone(), bare_words[i + 1].clone());
            i += 2;
        }
        // Trailing key with no value gets empty string
        if i < bare_words.len() {
            map.insert(bare_words[i].clone(), String::new());
        }
        return map;
    }

    for entry in entries {
        let mut rest = entry.trim();
        while !rest.is_empty() {
            if rest.starts_with('[')
                && let Some(close) = rest.find("]=")
            {
                let key = &rest[1..close];
                let after = &rest[close + 2..];
                // When entries are separated by \x1F, each entry is a single
                // key=value pair, so the entire remainder is the value.
                let (value, remaining) = if has_sep {
                    (after, "")
                } else if let Some(stripped) = after.strip_prefix('"') {
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
                    // Unquoted value: scan forward to find the next
                    // [key]= pattern (which starts a new entry) or the
                    // end of the string.  This ensures values containing
                    // spaces from variable expansion are kept whole.
                    // E.g. `[1 2]=3 4 5` → key="1 2", value="3 4 5"
                    // But `[a]=1 [b]=2` → key="a", value="1", then key="b", value="2"
                    let mut end = after.len();
                    // Look for ` [` followed by `]=` — that indicates
                    // the start of the next subscripted entry.
                    let bytes = after.as_bytes();
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i] == b' ' || bytes[i] == b'\t' {
                            // Found whitespace — check if what follows
                            // is a `[key]=` pattern.
                            let rest_after_ws = after[i..].trim_start();
                            if rest_after_ws.starts_with('[') && rest_after_ws.contains("]=") {
                                // Next entry starts here — trim trailing
                                // whitespace from current value.
                                end = i;
                                break;
                            }
                        }
                        i += 1;
                    }
                    let value_part = after[..end].trim_end();
                    let remaining = after[end..].trim_start();
                    (value_part, remaining)
                };
                map.insert(key.to_string(), value.to_string());
                rest = remaining;
                continue;
            }
            // Skip unknown content (bare element in mixed mode)
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
        // Stop processing remaining arguments after an arithmetic error,
        // matching bash which aborts `let` on the first error.
        if crate::expand::get_arith_error() {
            break;
        }
    }
    shell.arith_is_let = false;

    // Drain non-fatal arithmetic error flag (e.g. "not a valid identifier"
    // from empty subscripts).  These are informational — they don't affect
    // the exit status of `let` and don't abort subshells.
    let had_nonfatal = crate::expand::take_arith_nonfatal_error();

    // Drain (fatal) arithmetic error flag — let handles errors via return status
    let had_error = crate::expand::take_arith_error();

    // In subshells, fatal arithmetic errors during `let` abort the subshell
    // entirely (e.g. `( let "a[\" \"]"=18 ; echo hi )` never reaches
    // `echo hi`).  Non-fatal errors do NOT abort.
    if had_error && !had_nonfatal {
        let bash_subshell = shell
            .vars
            .get("BASH_SUBSHELL")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0);
        if bash_subshell > 0 {
            std::process::exit(1);
        }
    }

    // Non-fatal errors: treat as if the expression evaluated to non-zero
    // (return 0), matching bash where `let 'a[""]=26'` returns 0 after the
    // "not a valid identifier" error.
    if had_nonfatal {
        return 0;
    }

    // let returns 1 if the last expression evaluates to 0 or had error, 0 otherwise
    if had_error || result == 0 { 1 } else { 0 }
}
