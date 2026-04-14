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
            // If the name is a nameref, follow it and export the target.
            let export_name = if shell.namerefs.contains_key(arg.as_str()) {
                shell.resolve_nameref(arg)
            } else {
                arg.clone()
            };
            if let Some(value) = shell
                .vars
                .get(export_name.as_str())
                .cloned()
                .or_else(|| std::env::var(&export_name).ok())
            {
                shell.exports.insert(export_name.clone(), value.clone());
                unsafe { std::env::set_var(&export_name, &value) };
            } else {
                // Variable is unset — mark it for export without setting a value.
                // Use declared_unset + exports so the export attribute persists
                // and takes effect when the variable is later assigned.
                shell.declared_unset.insert(export_name.clone());
                shell.exports.insert(export_name.clone(), String::new());
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
                // If the variable doesn't exist at all (not as indexed array,
                // assoc array, or scalar), skip subscript evaluation entirely.
                // Bash silently ignores `unset nonexistent["$subscript"]`.
                let is_indexed_array_early = shell.arrays.contains_key(&resolved);
                let is_scalar_early = shell.vars.contains_key(&resolved);
                if !is_indexed_array_early && !is_scalar_early {
                    continue;
                }
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
            shell.arrays.remove(name);
            shell.assoc_arrays.remove(name);
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
            // Check if the resolved target is a subscripted reference like v[1]
            if let Some(bracket) = resolved.find('[') {
                let base = &resolved[..bracket];
                let idx_str = &resolved[bracket + 1..resolved.len().saturating_sub(1)];
                // Check readonly on the base variable
                if shell.readonly_vars.contains(base) {
                    eprintln!(
                        "{}: unset: {}: cannot unset: readonly variable",
                        shell.error_prefix(),
                        base
                    );
                    status = 1;
                    continue;
                }
                // Unset the specific array element through the nameref
                if shell.assoc_arrays.contains_key(base) {
                    if let Some(assoc) = shell.assoc_arrays.get_mut(base) {
                        assoc.remove(idx_str);
                    }
                } else if shell.arrays.contains_key(base) {
                    let raw_idx = shell.eval_arith_expr(idx_str);
                    if crate::expand::take_arith_error() {
                        shell.last_status = 1;
                        continue;
                    }
                    if let Some(arr) = shell.arrays.get_mut(base) {
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
                } else {
                    // Target is a scalar — remove it entirely
                    shell.vars.remove(base);
                    shell.exports.remove(base);
                    if !base.is_empty() {
                        unsafe { std::env::remove_var(base) };
                    }
                }
            } else {
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
            // Check if this variable is a local in the current function scope.
            // In bash, unsetting a local variable leaves it in a "declared but
            // unset" state — `declare -p v` still shows `declare -- v`.
            let is_local_current = shell
                .local_scopes
                .last()
                .is_some_and(|s| s.contains_key(name));

            // Check if the variable is a local in a PARENT function scope
            // (not the current one).  In bash, `unset` from a child function
            // "peels" one scope layer: it removes the parent's local and
            // restores the saved value from before that local was created,
            // revealing the enclosing scope's variable.
            //
            // With `localvar_unset` shopt enabled, the behavior changes:
            // unset from a child function marks the parent's local as
            // declared-but-unset instead of peeling the scope.
            let is_local_parent = if !is_local_current && shell.local_scopes.len() >= 2 {
                // Find the innermost parent scope that has this variable saved
                let scopes_len = shell.local_scopes.len();
                // Search from the second-to-last (innermost parent) outward
                let mut found_idx = None;
                for i in (0..scopes_len - 1).rev() {
                    if shell.local_scopes[i].contains_key(name) {
                        found_idx = Some(i);
                        break;
                    }
                }
                found_idx
            } else if !is_local_current && shell.local_scopes.len() == 1 {
                // Only one scope and the variable isn't in it as a current-scope local,
                // but check if it's saved in this scope (meaning we're in a nested call
                // within the same function scope)
                if shell.local_scopes[0].contains_key(name) {
                    Some(0)
                } else {
                    None
                }
            } else {
                None
            };

            // Remember if it was exported before we remove it, so we can
            // preserve the export attribute on declared-but-unset locals.
            let was_exported = is_local_current && shell.exports.contains_key(name);

            let localvar_unset = shell
                .shopt_options
                .get("localvar_unset")
                .copied()
                .unwrap_or(false);

            if let Some(parent_idx) = is_local_parent {
                if !localvar_unset {
                    // Default behavior: peel one scope layer.
                    // Remove the saved entry from the parent scope and restore
                    // the saved value to the flat maps, revealing the enclosing
                    // scope's variable.
                    if let Some(saved) = shell.local_scopes[parent_idx].remove(name) {
                        // First, remove current state
                        shell.vars.remove(name);
                        shell.arrays.remove(name);
                        shell.assoc_arrays.remove(name);
                        shell.namerefs.remove(name);
                        shell.integer_vars.remove(name);
                        shell.uppercase_vars.remove(name);
                        shell.lowercase_vars.remove(name);
                        shell.capitalize_vars.remove(name);
                        shell.declared_unset.remove(name);
                        shell.exports.remove(name);

                        // Restore the saved state (from before the local was created)
                        match saved.scalar {
                            Some(val) => {
                                shell.vars.insert(name.to_string(), val);
                            }
                            None => {
                                shell.vars.remove(name);
                            }
                        }
                        match saved.array {
                            Some(arr) => {
                                shell.arrays.insert(name.to_string(), arr);
                            }
                            None => {
                                shell.arrays.remove(name);
                            }
                        }
                        match saved.assoc {
                            Some(assoc) => {
                                shell.assoc_arrays.insert(name.to_string(), assoc);
                            }
                            None => {
                                shell.assoc_arrays.remove(name);
                            }
                        }
                        if saved.was_integer {
                            shell.integer_vars.insert(name.to_string());
                        }
                        if saved.was_readonly {
                            shell.readonly_vars.insert(name.to_string());
                        }
                        if saved.was_declared_unset {
                            shell.declared_unset.insert(name.to_string());
                        }
                        match saved.nameref {
                            Some(target) => {
                                shell.namerefs.insert(name.to_string(), target);
                            }
                            None => {
                                shell.namerefs.remove(name);
                            }
                        }
                        // Restore export state from saved scope
                        match &saved.was_exported {
                            Some(export_val) => {
                                let env_val = shell
                                    .vars
                                    .get(name)
                                    .cloned()
                                    .unwrap_or_else(|| export_val.clone());
                                shell.exports.insert(name.to_string(), env_val.clone());
                                if !name.is_empty() {
                                    unsafe { std::env::set_var(name, &env_val) };
                                }
                            }
                            None => {
                                if shell.exports.contains_key(name) {
                                    shell.exports.remove(name);
                                    if !name.is_empty() {
                                        unsafe { std::env::remove_var(name) };
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // localvar_unset behavior: mark the parent's local as
                    // declared-but-unset without peeling the scope.
                    shell.vars.remove(name);
                    shell.exports.remove(name);
                    shell.arrays.remove(name);
                    shell.assoc_arrays.remove(name);
                    shell.namerefs.remove(name);
                    shell.integer_vars.remove(name);
                    shell.uppercase_vars.remove(name);
                    shell.lowercase_vars.remove(name);
                    shell.capitalize_vars.remove(name);
                    if name == "IGNOREEOF" {
                        shell.shopt_options.insert("ignoreeof".to_string(), false);
                    }
                    if !name.is_empty() {
                        unsafe { std::env::remove_var(name) };
                    }
                    shell.declared_unset.insert(name.to_string());
                }
            } else {
                // Variable is either in the current scope or not in any scope
                shell.vars.remove(name);
                shell.exports.remove(name);
                shell.arrays.remove(name);
                shell.assoc_arrays.remove(name);
                shell.namerefs.remove(name);
                shell.integer_vars.remove(name);
                shell.uppercase_vars.remove(name);
                shell.lowercase_vars.remove(name);
                shell.capitalize_vars.remove(name);
                // Unsetting IGNOREEOF disables the ignoreeof option (matching bash).
                if name == "IGNOREEOF" {
                    shell.shopt_options.insert("ignoreeof".to_string(), false);
                }
                if !name.is_empty() {
                    unsafe { std::env::remove_var(name) };
                }
                // If the variable was a local in the current scope, mark it as
                // declared-but-unset so that `declare -p` still shows it (bash behavior).
                // Also preserve the export attribute if the variable came from
                // temp env — bash shows `declare -x v` after unsetting a local
                // that was created from a temp env prefix assignment.
                if is_local_current && had_var {
                    shell.declared_unset.insert(name.to_string());
                    // If the variable was exported (e.g. from temp env `v=t f`),
                    // re-add the export so `declare -p` shows `-x`.
                    if was_exported {
                        shell.exports.insert(name.to_string(), String::new());
                    }
                }
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
        } else if (shell.readonly_vars.contains(&shell.resolve_nameref(name))
            || name.find('=').is_some_and(|eq| {
                let vname = &name[..eq];
                let resolved = shell.resolve_nameref(vname);
                shell.readonly_vars.contains(&resolved)
            }))
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
                let resolved = shell.resolve_nameref(vname);
                eprintln!("{}: {}: readonly variable", shell.error_prefix(), resolved);
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
                let resolved_vname = shell.resolve_nameref(vname);
                if shell.readonly_vars.contains(&resolved_vname) {
                    eprintln!(
                        "{}: readonly: {}: readonly variable",
                        shell.error_prefix(),
                        resolved_vname
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
            let resolved = shell.resolve_nameref(vname);
            shell.readonly_vars.insert(resolved);
        } else {
            // readonly ref — if ref is a nameref, resolve through it
            // and mark the target readonly (bash behavior)
            let resolved = shell.resolve_nameref(name);
            // If the resolved target contains a subscript (e.g. var[0]),
            // bash rejects it with "not a valid identifier" because
            // readonly operates on whole variables, not array elements.
            if resolved.contains('[') && resolved.ends_with(']') {
                eprintln!(
                    "{}: readonly: `{}': not a valid identifier",
                    shell.error_prefix(),
                    resolved
                );
                status = 1;
            } else {
                shell.readonly_vars.insert(resolved);
            }
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
    let mut flag_inherit = false;
    let mut names = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "-" {
            // local - : save shell options for restoration on function return
            // Capture options before taking mutable ref to avoid borrow conflict.
            let needs_save = shell.saved_opts_stack.last().is_some_and(|o| o.is_none());
            if needs_save {
                let saved = crate::interpreter::SavedOpts::capture(shell);
                if let Some(last) = shell.saved_opts_stack.last_mut() {
                    *last = Some(saved);
                }
            }
        } else if arg == "-p" {
            // local -p [name ...]: print local variables in declare format.
            // With names: print each named local, error "not found" for non-locals.
            // Without names: print all local variables.
            let remaining: Vec<String> = args[i + 1..]
                .iter()
                .filter(|a| !a.starts_with('-'))
                .cloned()
                .collect();
            let print_local_declare = |shell: &crate::interpreter::Shell, name: &str| {
                // Build attribute flags string
                let mut flags = String::new();
                if shell.arrays.contains_key(name) {
                    flags.push('a');
                } else if shell.assoc_arrays.contains_key(name) {
                    flags.push('A');
                }
                if shell.integer_vars.contains(name) {
                    flags.push('i');
                }
                if shell.readonly_vars.contains(name) {
                    flags.push('r');
                }
                if shell.exports.contains_key(name) {
                    flags.push('x');
                }
                if shell.namerefs.contains_key(name) {
                    flags.push('n');
                }
                if shell.lowercase_vars.contains(name) {
                    flags.push('l');
                }
                if shell.uppercase_vars.contains(name) {
                    flags.push('u');
                }
                if shell.capitalize_vars.contains(name) {
                    flags.push('c');
                }
                let flag_str = if flags.is_empty() {
                    "--".to_string()
                } else {
                    format!("-{}", flags)
                };
                if let Some(arr) = shell.arrays.get(name) {
                    let elems: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| {
                            v.as_ref().map(|s| {
                                format!(
                                    "[{}]=\"{}\"",
                                    i,
                                    s.replace('\\', "\\\\")
                                        .replace('"', "\\\"")
                                        .replace('$', "\\$")
                                        .replace('`', "\\`")
                                )
                            })
                        })
                        .collect();
                    if elems.is_empty() && shell.declared_unset.contains(name) {
                        println!("declare {} {}", flag_str, name);
                    } else {
                        println!("declare {} {}=({})", flag_str, name, elems.join(" "));
                    }
                } else if let Some(assoc) = shell.assoc_arrays.get(name) {
                    let elems: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| {
                            format!(
                                "[\"{}\"]=\"{}\"",
                                crate::builtins::quote_assoc_key(k),
                                v.replace('\\', "\\\\")
                                    .replace('"', "\\\"")
                                    .replace('$', "\\$")
                                    .replace('`', "\\`")
                            )
                        })
                        .collect();
                    println!("declare {} {}=({})", flag_str, name, elems.join(" "));
                } else if let Some(target) = shell.namerefs.get(name) {
                    println!("declare {} {}=\"{}\"", flag_str, name, target);
                } else if let Some(val) = shell.vars.get(name) {
                    println!(
                        "declare {} {}=\"{}\"",
                        flag_str,
                        name,
                        val.replace('\\', "\\\\")
                            .replace('"', "\\\"")
                            .replace('$', "\\$")
                            .replace('`', "\\`")
                    );
                } else {
                    // Declared but unset
                    println!("declare {} {}", flag_str, name);
                }
            };
            let mut status = 0;
            if remaining.is_empty() {
                // local -p: print all locals
                if let Some(scope) = shell.local_scopes.last() {
                    let mut sorted: Vec<_> = scope.keys().collect();
                    sorted.sort();
                    for name in sorted {
                        print_local_declare(shell, name);
                    }
                }
                // If `local -` was used in this function, print it
                if shell.saved_opts_stack.last().is_some_and(|o| o.is_some()) {
                    println!("local -");
                }
            } else {
                // local -p name1 name2 ...: print each named local
                for name in &remaining {
                    let is_local = shell
                        .local_scopes
                        .last()
                        .map(|s| s.contains_key(name.as_str()))
                        .unwrap_or(false);
                    if is_local {
                        print_local_declare(shell, name);
                    } else {
                        eprintln!("{}: local: {}: not found", shell.error_prefix(), name);
                        status = 1;
                    }
                }
            }
            return status;
        } else if arg.starts_with('-') && arg.len() > 1 {
            for ch in arg[1..].chars() {
                match ch {
                    'a' => flag_array = true,
                    'A' => flag_assoc = true,
                    'r' => flag_readonly = true,
                    'n' => flag_nameref = true,
                    'i' => flag_integer = true,
                    'I' => flag_inherit = true,
                    _ => {}
                }
            }
        } else {
            names.push(arg.clone());
        }
        i += 1;
    }

    let mut status = 0;
    for name_arg in &names {
        let var_name;
        if let Some(eq_pos) = name_arg.find('=') {
            // Detect += append syntax: name ends with '+' before '='
            let (name, value, is_append) = if eq_pos > 0 && name_arg.as_bytes()[eq_pos - 1] == b'+'
            {
                (&name_arg[..eq_pos - 1], &name_arg[eq_pos + 1..], true)
            } else {
                (&name_arg[..eq_pos], &name_arg[eq_pos + 1..], false)
            };
            var_name = name.to_string();
            // Check readonly BEFORE assignment — `local var=value` should
            // error if:
            //   (a) var is globally readonly (not saved in ANY local scope), OR
            //   (b) var is readonly in the CURRENT function's local scope
            // But if var is readonly from an OUTER function scope (saved in
            // a scope that is not the current one), bash allows shadowing —
            // `declare_local` will save the old value and remove readonly.
            let is_readonly_and_blocking = if shell.readonly_vars.contains(name) {
                // Check if the readonly comes from an outer local scope
                // (i.e., saved in some scope below the top).  If so, allow.
                let in_current_scope = shell
                    .local_scopes
                    .last()
                    .is_some_and(|s| s.contains_key(name));
                let in_any_outer_scope = shell.local_scopes.len() >= 2
                    && shell.local_scopes[..shell.local_scopes.len() - 1]
                        .iter()
                        .any(|s| s.contains_key(name));
                if in_current_scope {
                    // Already declared local in THIS function and readonly → error
                    true
                } else if in_any_outer_scope {
                    // Readonly from an outer function scope → allow shadowing
                    false
                } else {
                    // Globally readonly (not in any local scope) → error
                    true
                }
            } else {
                false
            };
            if is_readonly_and_blocking {
                eprintln!(
                    "{}: local: {}: readonly variable",
                    shell.error_prefix(),
                    name
                );
                status = 1;
                continue;
            }
            // When using += with localvar_inherit (or -I flag), we need to
            // inherit the current value BEFORE declare_local saves and
            // potentially clears it.  For += with compound assignment on
            // arrays, the inherited value is the base that gets appended to.
            // For plain = (no append), declare_local saves the old value
            // and the new assignment overwrites it completely.
            //
            // For += WITHOUT localvar_inherit, declare_local still saves
            // the old value, but the local starts empty and we just assign
            // the compound value (no inheritance, no append to old).
            let inherit = flag_inherit
                || *shell
                    .shopt_options
                    .get("localvar_inherit")
                    .unwrap_or(&false);

            // Snapshot inherited state before declare_local clears it.
            // We need the inherited arrays/assoc/scalar for += append.
            let inherited_array = if is_append && inherit {
                shell.arrays.get(name).cloned()
            } else {
                None
            };
            let inherited_assoc = if is_append && inherit {
                shell.assoc_arrays.get(name).cloned()
            } else {
                None
            };
            let inherited_scalar = if is_append && inherit {
                shell.vars.get(name).cloned()
            } else {
                None
            };
            // Check if variable is currently an array/assoc (before declare_local)
            let was_array = shell.arrays.contains_key(name);
            let was_assoc = shell.assoc_arrays.contains_key(name);

            // When the variable is already a nameref in the CURRENT local
            // scope and we're not setting -n, resolve through the nameref
            // so that -A/-a/compound assignments operate on the TARGET.
            // e.g. `local -n ref=var; local -A ref=([1]=)` should create
            // `var` as an associative array, not `ref`.
            //
            // Important: only treat as existing nameref if the nameref is
            // already in the current local scope (from a previous `local -n`).
            // If the nameref is inherited from global scope, `declare_local`
            // will save and remove it, so we should NOT resolve through it
            // (the intent is to shadow the global nameref with a local scalar).
            let nameref_is_already_local = shell
                .local_scopes
                .last()
                .is_some_and(|s| s.contains_key(name));
            let is_existing_nameref =
                !flag_nameref && shell.namerefs.contains_key(name) && nameref_is_already_local;
            let assign_target = if is_existing_nameref {
                shell.resolve_nameref(name).to_string()
            } else {
                name.to_string()
            };
            let assign_target_name = assign_target.as_str();

            shell.declare_local(name);
            // When assigning through a nameref, also declare_local the
            // target so it gets saved/restored properly on scope exit.
            if is_existing_nameref && assign_target_name != name {
                shell.declare_local(assign_target_name);
            }
            if flag_integer && !is_existing_nameref {
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
                let target = assign_target_name;
                let trimmed_val = value.trim();
                if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                    if is_append {
                        // Append to inherited assoc array
                        let mut map = if inherit {
                            inherited_assoc.unwrap_or_default()
                        } else {
                            crate::interpreter::AssocArray::default()
                        };
                        let new_map = crate::builtins::parse_assoc_literal(value);
                        for (k, v) in new_map.iter() {
                            map.insert(k.clone(), v.clone());
                        }
                        shell.assoc_arrays.insert(target.to_string(), map);
                    } else {
                        let map = crate::builtins::parse_assoc_literal(value);
                        shell.assoc_arrays.insert(target.to_string(), map);
                    }
                } else {
                    // Bare value without (): local -A name=value
                    // Bash assigns the value to key "0", not implicit key-value pairing.
                    let mut map = if is_append && inherit {
                        inherited_assoc.unwrap_or_default()
                    } else {
                        crate::interpreter::AssocArray::default()
                    };
                    map.insert("0".to_string(), value.to_string());
                    shell.assoc_arrays.insert(target.to_string(), map);
                }
                // Remove any scalar on the target
                shell.vars.remove(target);
            } else if flag_array {
                let target = assign_target_name;
                let trimmed_val = value.trim();
                let is_compound = trimmed_val.starts_with('(') && trimmed_val.ends_with(')');
                if is_compound {
                    let new_arr = crate::builtins::parse_indexed_compound_assignment(value);
                    if is_append {
                        let mut arr = if inherit {
                            inherited_array.unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        // Compound append: add new elements after the last existing one
                        let start_idx = crate::interpreter::array_effective_len(&arr);
                        for (i, elem) in new_arr.into_iter().enumerate() {
                            let idx = start_idx + i;
                            while arr.len() <= idx {
                                arr.push(None);
                            }
                            arr[idx] = elem;
                        }
                        shell.apply_case_attrs_to_array(target, &mut arr);
                        shell.arrays.insert(target.to_string(), arr);
                    } else {
                        let mut arr = new_arr;
                        shell.apply_case_attrs_to_array(target, &mut arr);
                        shell.arrays.insert(target.to_string(), arr);
                    }
                } else if is_append {
                    // Bare value append: `local -a a+=Y` appends string Y
                    // to element [0] of the inherited array (not a new element).
                    // If the inherited value is a scalar (not an array),
                    // convert it to an array with element [0] = scalar.
                    let mut arr = if inherit {
                        if let Some(a) = inherited_array.clone() {
                            a
                        } else if let Some(s) = inherited_scalar.clone() {
                            // Scalar → array conversion for +=
                            vec![Some(s)]
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };
                    if arr.is_empty() {
                        arr.push(Some(value.to_string()));
                    } else {
                        let existing = arr[0].clone().unwrap_or_default();
                        arr[0] = Some(format!("{}{}", existing, value));
                    }
                    // Remove scalar since we're now an array
                    shell.vars.remove(target);
                    shell.apply_case_attrs_to_array(target, &mut arr);
                    shell.arrays.insert(target.to_string(), arr);
                } else {
                    // Bare value without (): assign as scalar to element [0]
                    let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                    shell.apply_case_attrs_to_array(target, &mut arr);
                    shell.arrays.insert(target.to_string(), arr);
                }
                // Remove any scalar on the target
                shell.vars.remove(target);
            } else if flag_integer {
                let target_for_int = if is_existing_nameref {
                    shell.integer_vars.insert(assign_target_name.to_string());
                    assign_target_name
                } else {
                    name
                };
                let trimmed_val = value.trim();
                let looks_compound_int = trimmed_val.starts_with('(') && trimmed_val.ends_with(')');
                if looks_compound_int {
                    // Compound assignment with integer attribute — create array
                    // and evaluate each element as arithmetic
                    let arr = crate::builtins::parse_indexed_compound_assignment(value);
                    let evaluated: Vec<Option<String>> = arr
                        .into_iter()
                        .map(|v| {
                            Some(
                                v.map(|s| shell.eval_arith_expr(&s).to_string())
                                    .unwrap_or_else(|| "0".to_string()),
                            )
                        })
                        .collect();
                    shell.arrays.insert(target_for_int.to_string(), evaluated);
                    shell.vars.remove(target_for_int);
                } else if value.is_empty() && shell.arrays.contains_key(target_for_int) {
                    // Target already has an array (from pre-processing compound
                    // assignment) — just ensure integer attribute is applied,
                    // don't overwrite with scalar.
                } else {
                    let n = shell.eval_arith_expr(value);
                    if is_append {
                        let existing = if inherit {
                            inherited_scalar
                                .as_deref()
                                .and_then(|v| v.parse::<i64>().ok())
                                .unwrap_or(0)
                        } else {
                            0
                        };
                        shell.set_var(name, (existing + n).to_string());
                    } else {
                        shell.set_var(name, n.to_string());
                    }
                }
            } else {
                // Scalar or compound assignment without explicit -a/-A flag.
                // If the value looks like a compound assignment (...)  and
                // the variable was an array/assoc, treat as compound.
                let target = assign_target_name;
                let trimmed_val = value.trim();
                let looks_compound = trimmed_val.starts_with('(') && trimmed_val.ends_with(')');
                // Check was_array/was_assoc on the target (not the nameref)
                let target_was_assoc =
                    was_assoc || (is_existing_nameref && shell.assoc_arrays.contains_key(target));
                let target_was_array =
                    was_array || (is_existing_nameref && shell.arrays.contains_key(target));
                if looks_compound && (target_was_assoc || flag_assoc) {
                    // Associative array compound assignment
                    if is_append {
                        let mut map = if inherit {
                            inherited_assoc.unwrap_or_default()
                        } else {
                            crate::interpreter::AssocArray::default()
                        };
                        let new_map = crate::builtins::parse_assoc_literal(value);
                        for (k, v) in new_map.iter() {
                            map.insert(k.clone(), v.clone());
                        }
                        shell.assoc_arrays.insert(target.to_string(), map);
                    } else {
                        let map = crate::builtins::parse_assoc_literal(value);
                        shell.assoc_arrays.insert(target.to_string(), map);
                    }
                } else if looks_compound
                    && (target_was_array || inherit && inherited_array.is_some())
                {
                    // Indexed array compound assignment
                    let new_arr = crate::builtins::parse_indexed_compound_assignment(value);
                    if is_append {
                        let mut arr = if inherit {
                            inherited_array.unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        let start_idx = crate::interpreter::array_effective_len(&arr);
                        for (i, elem) in new_arr.into_iter().enumerate() {
                            let idx = start_idx + i;
                            while arr.len() <= idx {
                                arr.push(None);
                            }
                            arr[idx] = elem;
                        }
                        shell.apply_case_attrs_to_array(target, &mut arr);
                        shell.arrays.insert(target.to_string(), arr);
                    } else {
                        let mut arr = new_arr;
                        shell.apply_case_attrs_to_array(target, &mut arr);
                        shell.arrays.insert(target.to_string(), arr);
                    }
                } else if looks_compound && is_append && inherit && inherited_scalar.is_some() {
                    // Scalar with += and compound value: convert scalar to
                    // array and append (bash behavior for `local s+=(Y)`
                    // where s was a scalar).
                    let mut arr: Vec<Option<String>> = vec![inherited_scalar.clone()];
                    let new_arr = crate::builtins::parse_indexed_compound_assignment(value);
                    let start_idx = crate::interpreter::array_effective_len(&arr);
                    for (i, elem) in new_arr.into_iter().enumerate() {
                        let idx = start_idx + i;
                        while arr.len() <= idx {
                            arr.push(None);
                        }
                        arr[idx] = elem;
                    }
                    shell.apply_case_attrs_to_array(target, &mut arr);
                    shell.arrays.insert(target.to_string(), arr);
                    // Remove scalar since we converted to array
                    shell.vars.remove(target);
                } else if looks_compound && is_existing_nameref {
                    // Compound assignment through a nameref to a target that
                    // doesn't exist yet — create indexed array on the target
                    // (bash behavior: `local -n ref=var; local ref=(X)` creates
                    // `var` as indexed array).
                    let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                    shell.apply_case_attrs_to_array(target, &mut arr);
                    shell.arrays.insert(target.to_string(), arr);
                    shell.vars.remove(target);
                } else if looks_compound {
                    // Compound assignment on a non-array variable without
                    // inheritance — just parse as indexed array
                    let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
                    shell.apply_case_attrs_to_array(name, &mut arr);
                    shell.arrays.insert(name.to_string(), arr);
                } else if is_append {
                    // Scalar append
                    if shell.integer_vars.contains(name) {
                        let existing = if inherit {
                            inherited_scalar
                                .as_deref()
                                .and_then(|v| v.parse::<i64>().ok())
                                .unwrap_or(0)
                        } else {
                            0
                        };
                        let addend = shell.eval_arith_expr(value);
                        shell.set_var(name, (existing + addend).to_string());
                    } else {
                        let existing = if inherit {
                            inherited_scalar.unwrap_or_default()
                        } else {
                            String::new()
                        };
                        shell.set_var(name, format!("{}{}", existing, value));
                    }
                } else {
                    shell.set_var(name, value.to_string());
                }
            }
        } else {
            var_name = name_arg.clone();
            // Check readonly BEFORE declare_local — `local name` (bare, no =)
            // on a globally readonly variable should error, not shadow.
            // Same logic as the name=value path: error if globally readonly
            // (not saved in any local scope) or readonly in current scope.
            // Allow shadowing if readonly comes from an outer function scope.
            if shell.readonly_vars.contains(name_arg.as_str()) {
                let in_current_scope = shell
                    .local_scopes
                    .last()
                    .is_some_and(|s| s.contains_key(name_arg.as_str()));
                let in_any_outer_scope = shell.local_scopes.len() >= 2
                    && shell.local_scopes[..shell.local_scopes.len() - 1]
                        .iter()
                        .any(|s| s.contains_key(name_arg.as_str()));
                if in_current_scope || !in_any_outer_scope {
                    // Globally readonly or readonly in current scope → error
                    eprintln!(
                        "{}: local: {}: readonly variable",
                        shell.error_prefix(),
                        name_arg
                    );
                    status = 1;
                    continue;
                }
            }
            shell.declare_local(name_arg);
            if flag_integer {
                // When the name is a nameref, apply integer attribute to the
                // target, not the nameref itself
                let nameref_already_local = shell
                    .local_scopes
                    .last()
                    .is_some_and(|s| s.contains_key(name_arg));
                if !flag_nameref && shell.namerefs.contains_key(name_arg) && nameref_already_local {
                    let target = shell.resolve_nameref(name_arg);
                    shell.integer_vars.insert(target);
                } else {
                    shell.integer_vars.insert(name_arg.clone());
                }
            }
            if flag_nameref {
                // No value provided — just mark as nameref (no circular risk)
                shell.namerefs.entry(name_arg.clone()).or_default();
            } else if flag_assoc {
                let inherit = flag_inherit
                    || *shell
                        .shopt_options
                        .get("localvar_inherit")
                        .unwrap_or(&false);
                if inherit {
                    // With localvar_inherit, inherit the current value.
                    // If the variable is a scalar, convert to assoc with
                    // key "0" = scalar_value.  If already assoc, keep it.
                    // If indexed array, convert: [idx] → key "idx".
                    if shell.assoc_arrays.contains_key(name_arg.as_str()) {
                        // Already an assoc — keep inherited value
                    } else if let Some(val) = shell.vars.get(name_arg.as_str()).cloned() {
                        let mut map = crate::interpreter::AssocArray::default();
                        map.insert("0".to_string(), val);
                        shell.assoc_arrays.insert(name_arg.clone(), map);
                        shell.vars.remove(name_arg.as_str());
                        shell.declared_unset.remove(name_arg.as_str());
                    } else if let Some(arr) = shell.arrays.get(name_arg.as_str()).cloned() {
                        let mut map = crate::interpreter::AssocArray::default();
                        for (i, v) in arr.iter().enumerate() {
                            if let Some(s) = v {
                                map.insert(i.to_string(), s.clone());
                            }
                        }
                        shell.assoc_arrays.insert(name_arg.clone(), map);
                        shell.arrays.remove(name_arg.as_str());
                        shell.declared_unset.remove(name_arg.as_str());
                    } else {
                        shell.assoc_arrays.entry(name_arg.clone()).or_default();
                    }
                } else {
                    shell.assoc_arrays.entry(name_arg.clone()).or_default();
                }
            } else if flag_array {
                let inherit = flag_inherit
                    || *shell
                        .shopt_options
                        .get("localvar_inherit")
                        .unwrap_or(&false);
                if inherit {
                    // With localvar_inherit, inherit the current value.
                    // If the variable is a scalar, convert to array with
                    // element [0] = scalar_value.  If already an array, keep it.
                    if shell.arrays.contains_key(name_arg.as_str()) {
                        // Already an array — keep inherited value
                        shell.declared_unset.remove(name_arg.as_str());
                    } else if let Some(val) = shell.vars.get(name_arg.as_str()).cloned() {
                        shell.arrays.insert(name_arg.clone(), vec![Some(val)]);
                        shell.vars.remove(name_arg.as_str());
                        shell.declared_unset.remove(name_arg.as_str());
                    } else {
                        shell.arrays.entry(name_arg.clone()).or_default();
                    }
                } else {
                    shell.arrays.entry(name_arg.clone()).or_default();
                }
            } else {
                // `local v` without `=`: creates a declared-but-unset local
                // that shadows any outer/global value.  However, if the
                // variable was set via temp env (`v=t f`), bash inherits the
                // temp env value.  We detect temp env by checking
                // `shell.temp_env_vars` which is set by `run_simple_command`
                // before calling `run_function`.
                //
                // When `-I` flag is set (or `shopt -s localvar_inherit`),
                // the local variable inherits its value from the calling
                // scope instead of being declared-but-unset.
                let inherit = flag_inherit
                    || *shell
                        .shopt_options
                        .get("localvar_inherit")
                        .unwrap_or(&false);
                if inherit {
                    // Keep the inherited value — don't clear the variable.
                    // declare_local already saved the old value; the current
                    // value in shell.vars/arrays/assoc_arrays IS the inherited
                    // value from the calling scope.
                    //
                    // However, if the variable doesn't exist at all (was
                    // completely unset), we still need to create a
                    // declared-but-unset entry so that `declare -p var`
                    // shows "declare -- var" instead of "not found".
                    if !shell.vars.contains_key(name_arg.as_str())
                        && !shell.arrays.contains_key(name_arg.as_str())
                        && !shell.assoc_arrays.contains_key(name_arg.as_str())
                    {
                        shell.declared_unset.insert(name_arg.clone());
                    }
                } else if shell.temp_env_vars.contains(name_arg.as_str()) {
                    // Temp env variable — keep inherited value
                    // (bash shows `declare -x v="t"`)
                } else {
                    // Regular global or unset — shadow with declared-but-unset
                    // (bash shows `declare -- v` with no value)
                    shell.vars.remove(name_arg.as_str());
                    shell.declared_unset.insert(name_arg.clone());
                    // Also remove from process environment so that the
                    // expansion fallback (`std::env::var`) doesn't leak the
                    // exported value.  The scope restore on function exit
                    // will re-export it if needed.
                    if shell.exports.contains_key(name_arg.as_str()) {
                        unsafe { std::env::remove_var(name_arg.as_str()) };
                    }
                }
            }
        }
        // Check readonly BEFORE applying attributes — if the variable is
        // readonly (from outer scope or current scope), `local name=value`
        // should fail with "readonly variable" error.  But `local name`
        // (no `=`) without -r flag is allowed to shadow readonly globals
        // in bash... actually no: bash rejects `local var=value` when var
        // is readonly, but the declare_local above already ran.  We need
        // to check readonly BEFORE the assignment.
        if flag_readonly {
            shell.readonly_vars.insert(var_name);
        }
    }
    status
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
    let mut flag_unset_nameref_global = false; // +n seen anywhere in args
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
                    // When inside a function scope, `typeset +x var`
                    // creates a local that shadows the export attribute.
                    // Don't remove from the process environment here —
                    // the scope restore on function exit will re-sync
                    // the process env based on the saved export state.
                    // Only declare_local the variable so the saved state
                    // captures the current export attribute for later
                    // restoration.
                    if !shell.local_scopes.is_empty() {
                        shell.declare_local(pure);
                    }
                    shell.exports.remove(pure);
                    if shell.local_scopes.is_empty() && !pure.is_empty() {
                        // At global scope, immediately remove from process env
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
                    // Record that +n was seen — actual nameref removal is
                    // deferred until after all flags are parsed so that
                    // attribute flags like -i/-x/-r that appear in later
                    // arguments are available when processing the target.
                    flag_unset_nameref_global = true;
                }
            }
        } else if !nameref_consumed.contains(arg) {
            names.push(arg.clone());
        }
        i += 1;
    }

    // Deferred +n nameref removal: now all flags (-i, -x, -r, etc.) have
    // been parsed, so we can correctly apply them to the nameref target.
    let mut early_status = 0;
    if flag_unset_nameref_global {
        // Re-scan names to find nameref variables that need +n processing
        let all_name_args: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with('-') && !a.starts_with('+') && *a != "--")
            .cloned()
            .collect();
        for rname in &all_name_args {
            let pure = rname.split('=').next().unwrap_or(rname);
            if !shell.namerefs.contains_key(pure) {
                continue; // not a nameref, skip
            }
            // Cannot remove nameref attribute from a readonly variable that
            // has a non-empty target.  Readonly namerefs with empty targets
            // (e.g. `declare -r -n foo5` with no value) CAN have their
            // nameref attribute removed — bash allows this.
            if shell.readonly_vars.contains(pure) {
                let has_target = shell.namerefs.get(pure).is_some_and(|t| !t.is_empty());
                if has_target {
                    eprintln!(
                        "{}: {}: {}: readonly variable",
                        shell.error_prefix(),
                        cmd_name,
                        pure
                    );
                    early_status = 1;
                    shell.last_status = 1;
                    nameref_consumed.insert(rname.clone());
                    names.retain(|n| n != rname);
                    continue;
                }
            }
            if let Some(eq) = rname.find('=') {
                let val = &rname[eq + 1..];
                if let Some(target) = shell.namerefs.get(pure).cloned() {
                    // Apply attribute flags to the TARGET variable
                    if flag_integer {
                        shell.integer_vars.insert(target.clone());
                        // Evaluate value as arithmetic for integer vars
                        let int_val = shell.eval_arith_expr(val);
                        shell.set_var(&target, int_val.to_string());
                    } else {
                        shell.set_var(&target, val.to_string());
                    }
                    if flag_export {
                        let v = shell.get_var(&target).unwrap_or_default();
                        shell.exports.insert(target.clone(), v.clone());
                        unsafe { std::env::set_var(&target, &v) };
                    }
                    if flag_readonly {
                        shell.readonly_vars.insert(target.clone());
                    }
                    if flag_uppercase {
                        shell.uppercase_vars.insert(target.clone());
                    }
                    if flag_lowercase {
                        shell.lowercase_vars.insert(target.clone());
                    }
                    if flag_capitalize {
                        shell.capitalize_vars.insert(target.clone());
                    }
                }
                // After removing nameref, foo's value is the old
                // target name (not the assigned value)
                if let Some(target) = shell.namerefs.remove(pure) {
                    if target.is_empty() {
                        // Nameref had no target — variable becomes declared-but-unset
                        shell.declared_unset.insert(pure.to_string());
                    } else {
                        shell.vars.insert(pure.to_string(), target);
                    }
                }
                nameref_consumed.insert(rname.clone());
                // Remove from names so the main declare body doesn't re-process
                names.retain(|n| n != rname);
            } else {
                // typeset +n foo (no value): just remove nameref,
                // set foo to the old target name
                if let Some(target) = shell.namerefs.remove(pure) {
                    if target.is_empty() {
                        // Nameref had no target — variable becomes declared-but-unset
                        shell.declared_unset.insert(pure.to_string());
                    } else {
                        shell.vars.insert(pure.to_string(), target);
                    }
                }
                nameref_consumed.insert(rname.clone());
                names.retain(|n| n != rname);
            }
        }
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
                    Some(cp) => {
                        // Valid bracket matching — check for empty subscript
                        let subscript = &pure_name[bracket_pos + 1..cp];
                        if subscript.is_empty() {
                            // Empty subscript like a[]=value → "bad array subscript"
                            // But if base name is also empty (e.g. []=value),
                            // that's "not a valid identifier" instead.
                            let base_name = &pure_name[..bracket_pos];
                            if base_name.is_empty() {
                                eprintln!(
                                    "{}: {}: `{}': not a valid identifier",
                                    shell.error_prefix(),
                                    cmd_name,
                                    name
                                );
                            } else {
                                eprintln!(
                                    "{}: {}: bad array subscript",
                                    shell.error_prefix(),
                                    pure_name
                                );
                            }
                            status = 1;
                            continue;
                        }
                    }
                }
                &pure_name[..bracket_pos]
            } else {
                pure_name
            };
            // Reject empty base name: covers both bare empty names and
            // empty names with compound assignments like ''=(a b).
            // Also reject names where base_for_check is empty but pure_name
            // contains '[' (e.g. []=asdf — already caught above, but guard
            // against fallthrough).
            if base_for_check.is_empty() {
                // Empty identifier — if the original arg has '=' with a
                // compound value, bash reports "syntax error near unexpected
                // token `('" for the empty-name compound case.  Otherwise
                // it's "not a valid identifier".
                let has_compound = name.find('=').is_some_and(|eq| {
                    let v = &name[eq + 1..];
                    v.starts_with('(') && v.ends_with(')')
                });
                if has_compound {
                    let prefix = shell.error_prefix();
                    eprintln!("{}: syntax error near unexpected token `('", prefix,);
                    // Show the offending source line (matching bash behavior)
                    if let Some(ref input) = shell.current_execution_input {
                        let lineno: usize = shell
                            .vars
                            .get("LINENO")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        let line = input
                            .lines()
                            .nth(lineno.saturating_sub(1))
                            .unwrap_or(input.lines().next().unwrap_or(input));
                        eprintln!("{}: `{}'", prefix, line.trim_end());
                    }
                } else {
                    eprintln!(
                        "{}: {}: `{}': not a valid identifier",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                }
                status = 1;
            } else if !pure_name.is_empty()
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
                    let target = &shell.namerefs[name];
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    flags.push('n');
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if target.is_empty() {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!("declare {} {}=\"{}\"", flags, name, target);
                    }
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
                    let target = &shell.namerefs[name];
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    flags.push('n');
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if target.is_empty() {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!(
                            "declare {} {}=\"{}\"",
                            flags,
                            name,
                            escape_nameref_target(target)
                        );
                    }
                }
            }
        } else {
            let mut status = 0;
            for name in &names {
                if let Some(target) = shell.namerefs.get(name) {
                    let mut flags = String::from("-");
                    if shell.integer_vars.contains(name.as_str()) {
                        flags.push('i');
                    }
                    flags.push('n');
                    if shell.readonly_vars.contains(name.as_str()) {
                        flags.push('r');
                    }
                    if shell.exports.contains_key(name.as_str()) {
                        flags.push('x');
                    }
                    if target.is_empty() {
                        println!("declare {} {}", flags, name);
                    } else {
                        println!(
                            "declare {} {}=\"{}\"",
                            flags,
                            name,
                            escape_nameref_target(target)
                        );
                    }
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
                    status = 1;
                }
            }
            return status;
        }
        return 0;
    }

    // declare -x with no names: list exports
    if flag_export && names.is_empty() && nameref_consumed.is_empty() {
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
    if flag_readonly
        && names.is_empty()
        && nameref_consumed.is_empty()
        && !flag_array
        && !flag_assoc
    {
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
    // (but not when names were consumed by +n nameref removal)
    if flag_integer && names.is_empty() && nameref_consumed.is_empty() {
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
            println!("declare -n {}=\"{}\"", name, escape_nameref_target(target));
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

    let mut status = early_status;
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
                // Namerefs cannot be array elements — reject immediately.
                // Bash: "reference variable cannot be an array"
                if flag_nameref {
                    let display_name = name;
                    eprintln!(
                        "{}: {}: {}: reference variable cannot be an array",
                        shell.error_prefix(),
                        cmd_name,
                        display_name
                    );
                    status = 1;
                    continue;
                }

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
                    // If the base name is a nameref, remove the nameref attribute
                    // and use the variable itself (matching bash's "removing nameref
                    // attribute" behavior for declare -a/-A on namerefs).
                    if shell.namerefs.contains_key(stripped_name) {
                        eprintln!(
                            "{}: warning: {}: removing nameref attribute",
                            shell.error_prefix(),
                            stripped_name
                        );
                        shell.namerefs.remove(stripped_name);
                    }
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
                        } else if !idx_str.is_empty() {
                            // Subscripted assignment: declare -A arr["key"]=value
                            // Use the subscript as the associative array key,
                            // preserving existing entries.
                            let key = shell.expand_assoc_subscript(idx_str);
                            if !shell.assoc_arrays.contains_key(&resolved_base) {
                                shell.assoc_arrays.insert(
                                    resolved_base.clone(),
                                    crate::interpreter::AssocArray::default(),
                                );
                            }
                            shell.declared_unset.remove(&resolved_base);
                            shell
                                .assoc_arrays
                                .get_mut(&resolved_base)
                                .unwrap()
                                .insert(key, value.to_string());
                        } else {
                            // Bare value without () and no subscript: assign to key "0"
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
                            // Ensure the array exists (create if needed)
                            shell.arrays.entry(resolved_base.clone()).or_default();
                            // Apply flags even though the element assignment is skipped
                            if flag_integer {
                                shell.integer_vars.insert(stripped_name.to_string());
                            }
                            if flag_readonly {
                                shell.readonly_vars.insert(stripped_name.to_string());
                            }
                            if flag_export {
                                let val = shell.get_var(stripped_name).unwrap_or_default();
                                shell.exports.insert(stripped_name.to_string(), val.clone());
                                unsafe { std::env::set_var(stripped_name, &val) };
                            }
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
                        // Ensure the array exists (create if needed)
                        shell.arrays.entry(resolved_base.clone()).or_default();
                        // Apply flags even though the element assignment is skipped
                        if flag_integer {
                            shell.integer_vars.insert(base.to_string());
                        }
                        if flag_readonly {
                            shell.readonly_vars.insert(base.to_string());
                        }
                        if flag_export {
                            let val = shell.get_var(base).unwrap_or_default();
                            shell.exports.insert(base.to_string(), val.clone());
                            unsafe { std::env::set_var(base, &val) };
                        }
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
                if flag_integer {
                    shell.integer_vars.insert(base.to_string());
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

            // When declare -g is used inside a function scope, save the
            // current (local) state so we can restore it after the assignment.
            // The assignment will write to the current maps (which hold the
            // local value), and then we'll propagate the new value to the
            // global scope and restore the local.
            let global_fixup = if flag_global && !shell.local_scopes.is_empty() {
                // Check if any scope has saved this variable (meaning it's been
                // localized by some enclosing function)
                let has_saved = shell.local_scopes.iter().any(|s| s.contains_key(name));
                if has_saved {
                    Some((
                        name.to_string(),
                        shell.vars.get(name).cloned(),
                        shell.arrays.get(name).cloned(),
                        shell.assoc_arrays.get(name).cloned(),
                        shell.integer_vars.contains(name),
                        shell.declared_unset.contains(name),
                    ))
                } else {
                    None
                }
            } else {
                None
            };

            if flag_nameref {
                // When both -i and -n are set with a value, bash silently
                // fails (exit 1) and does NOT create the variable.
                // `-i` and `-n` are incompatible because `-i` requires
                // arithmetic evaluation while `-n` requires a variable name.
                if flag_integer {
                    status = 1;
                    continue;
                }
                // Handle += append for namerefs: if the variable is already
                // a nameref and is_append is true, prepend the existing target
                // to the value. E.g. `declare -n ref=re ref+=f` → target
                // becomes "re"+"f"="ref" (then self-reference check applies).
                let effective_value: String;
                let value = if is_append && shell.namerefs.contains_key(name) {
                    let existing = shell.namerefs.get(name).cloned().unwrap_or_default();
                    effective_value = format!("{}{}", existing, value);
                    effective_value.as_str()
                } else {
                    value
                };
                // Validate the nameref target FIRST — bash checks target validity
                // before checking array conflicts, so `declare -n array='(bad)'`
                // reports "invalid variable name for name reference" even when
                // `array` is already an array.
                let is_array_conflict = name.contains('[')
                    || shell.arrays.contains_key(name)
                    || shell.assoc_arrays.contains_key(name);
                if value.is_empty() {
                    // `declare -n name=` with empty target — bash reports
                    // "not a valid identifier" (not the nameref-specific message)
                    // and does NOT create the variable at all.
                    eprintln!(
                        "{}: {}: `{}': not a valid identifier",
                        shell.error_prefix(),
                        cmd_name,
                        value
                    );
                    status = 1;
                    continue;
                } else if !crate::interpreter::is_valid_nameref_target(value) {
                    eprintln!(
                        "{}: {}: `{}': invalid variable name for name reference",
                        shell.error_prefix(),
                        cmd_name,
                        value
                    );
                    status = 1;
                } else if is_array_conflict {
                    eprintln!(
                        "{}: {}: {}: reference variable cannot be an array",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                    status = 1;
                }
                // Detect self-reference: if the target's base name matches
                // the variable name, it's a self-reference (e.g., declare -n x=x)
                else {
                    let target_base = if let Some(bracket) = value.find('[') {
                        &value[..bracket]
                    } else {
                        value
                    };
                    if target_base == name {
                        // In function scope (local_scopes non-empty), bash treats
                        // self-reference as a warning ("circular name reference")
                        // but still creates the nameref. At global scope, it's an
                        // error ("self references not allowed") and the nameref is
                        // NOT created.
                        if !shell.local_scopes.is_empty() {
                            eprintln!(
                                "{}: {}: warning: {}: circular name reference",
                                shell.error_prefix(),
                                cmd_name,
                                name
                            );
                            // Still create the nameref (bash behavior in function scope)
                            shell.vars.remove(name);
                            if !flag_integer {
                                shell.integer_vars.remove(name);
                            }
                            shell.namerefs.insert(name.to_string(), value.to_string());
                        } else if is_append {
                            // Append case (ref+=f): bash omits command name
                            eprintln!(
                                "{}: {}: nameref variable self references not allowed",
                                shell.error_prefix(),
                                name
                            );
                        } else {
                            eprintln!(
                                "{}: {}: {}: nameref variable self references not allowed",
                                shell.error_prefix(),
                                cmd_name,
                                name
                            );
                        }
                    } else {
                        shell.vars.remove(name);
                        // When converting a non-nameref variable to a nameref,
                        // remove the integer attribute (bash behavior:
                        // `declare -i x; declare -n x=foo` removes -i from x).
                        if !flag_integer {
                            shell.integer_vars.remove(name);
                        }
                        shell.namerefs.insert(name.to_string(), value.to_string());
                    }
                }
            } else if flag_assoc {
                let trimmed_val = value.trim();
                if trimmed_val.starts_with('(') && trimmed_val.ends_with(')') {
                    // Compound assignment: declare -A name=(...)
                    // If the compound value contains $ or ` (e.g. from
                    // declare -A d='($a)'), re-lex and expand so that
                    // variable/command expansions are evaluated.
                    let needs_shell_expand = value.contains('$') || value.contains('`');
                    if needs_shell_expand {
                        let inner = &trimmed_val[1..trimmed_val.len() - 1];
                        let word = crate::lexer::lex_compound_array_content(inner);
                        // Split the lexed word into sub-words on literal
                        // whitespace boundaries, then expand each individually
                        // with expand_word_single (no IFS splitting) so that
                        // e.g. $a where a='x y' stays as one token "x y".
                        let sub_words = split_word_on_whitespace(&word);
                        let expanded: Vec<String> = sub_words
                            .iter()
                            .map(|w| shell.expand_word_single(w))
                            .filter(|s| !s.is_empty())
                            .collect();
                        // Assoc arrays use alternating key-value pairs for
                        // bare elements (no [key]=val syntax).
                        let mut map = crate::interpreter::AssocArray::default();
                        let mut iter = expanded.into_iter();
                        while let Some(key) = iter.next() {
                            let val = iter.next().unwrap_or_default();
                            map.insert(key, val);
                        }
                        shell.assoc_arrays.insert(name.to_string(), map);
                    } else {
                        let map = parse_assoc_literal(value);
                        shell.assoc_arrays.insert(name.to_string(), map);
                    }
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
                    // If the compound value contains $ or ` (e.g. from
                    // declare -a d='($a)'), re-lex and expand so that
                    // variable/command expansions are evaluated with the
                    // current shell state (which includes earlier assignments
                    // in the same declare command).
                    let needs_shell_expand = value.contains('$') || value.contains('`');
                    if needs_shell_expand {
                        let inner = &trimmed_val[1..trimmed_val.len() - 1];
                        let ifs = shell
                            .vars
                            .get("IFS")
                            .cloned()
                            .unwrap_or_else(|| " \t\n".to_string());
                        // Parse compound assignment elements first, respecting
                        // [subscript]=value syntax with bracket nesting.  This
                        // handles `declare -a var="($value)"` where the expanded
                        // text contains `[$(echo total 0)]=1 [2]=2]` — bash
                        // parses [subscript]=value boundaries BEFORE word-splitting.
                        let raw_elems = crate::builtins::parse_compound_assignment_raw(inner);
                        let has_any_subscript = raw_elems.iter().any(|(sub, _)| sub.is_some());
                        if has_any_subscript {
                            // Process compound elements with subscript evaluation
                            let mut arr: Vec<Option<String>> = Vec::new();
                            let mut next_idx: usize = 0;
                            let mut had_error = false;
                            for (sub_opt, val_raw) in &raw_elems {
                                if had_error {
                                    break;
                                }
                                if let Some(subscript) = sub_opt {
                                    // Re-lex and expand the subscript to handle $(...) etc.
                                    let sub_word =
                                        crate::lexer::lex_compound_array_content(subscript);
                                    let expanded_sub = shell.expand_word_single(&sub_word);
                                    let raw_idx = shell.eval_arith_expr(&expanded_sub);
                                    if crate::expand::take_arith_error() {
                                        had_error = true;
                                        arr.clear();
                                        break;
                                    }
                                    // Re-lex and expand the value
                                    let val_word =
                                        crate::lexer::lex_compound_array_content(val_raw);
                                    let expanded_val = shell.expand_word_single(&val_word);
                                    let final_val = if flag_integer {
                                        shell.eval_arith_expr(&expanded_val).to_string()
                                    } else {
                                        expanded_val
                                    };
                                    let idx = if raw_idx < 0 {
                                        let eff_len =
                                            crate::interpreter::array_effective_len(&arr) as i64;
                                        let computed = eff_len + raw_idx;
                                        if computed < 0 {
                                            0usize
                                        } else {
                                            computed as usize
                                        }
                                    } else {
                                        raw_idx as usize
                                    };
                                    while arr.len() <= idx {
                                        arr.push(None);
                                    }
                                    arr[idx] = Some(final_val);
                                    next_idx = idx + 1;
                                } else {
                                    // Bare element — re-lex, expand, and word-split
                                    let val_word =
                                        crate::lexer::lex_compound_array_content(val_raw);
                                    let fields = shell.expand_word_fields(&val_word, &ifs);
                                    for f in fields {
                                        let final_val = if flag_integer {
                                            shell.eval_arith_expr(&f).to_string()
                                        } else {
                                            f
                                        };
                                        while arr.len() <= next_idx {
                                            arr.push(None);
                                        }
                                        arr[next_idx] = Some(final_val);
                                        next_idx += 1;
                                    }
                                }
                            }
                            if !had_error && (flag_uppercase || flag_lowercase || flag_capitalize) {
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
                            if flag_integer {
                                shell.integer_vars.insert(name.to_string());
                            }
                            shell.arrays.insert(name.to_string(), arr);
                        } else {
                            // No subscripts — use the original lex+expand+word-split path
                            let word = crate::lexer::lex_compound_array_content(inner);
                            let expanded = shell.expand_word_fields(&word, &ifs);
                            let mut arr: Vec<Option<String>> =
                                expanded.into_iter().map(Some).collect();
                            if flag_integer {
                                let evaluated: Vec<Option<String>> = arr
                                    .into_iter()
                                    .map(|v| v.map(|s| shell.eval_arith_expr(&s).to_string()))
                                    .collect();
                                shell.arrays.insert(name.to_string(), evaluated);
                                shell.integer_vars.insert(name.to_string());
                            } else {
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
                        }
                    } else {
                        // Use parse_compound_assignment_raw + arithmetic eval
                        // so that subscripts like [foo] are evaluated as
                        // arithmetic (foo → 0) instead of being treated as
                        // literal text via parse::<usize>().
                        let raw_elems = crate::builtins::parse_compound_assignment_raw(value);
                        let has_any_subscript = raw_elems.iter().any(|(sub, _)| sub.is_some());
                        if has_any_subscript {
                            let mut arr: Vec<Option<String>> = Vec::new();
                            let mut next_idx: usize = 0;
                            let mut had_error = false;
                            for (sub_opt, val_raw) in &raw_elems {
                                if had_error {
                                    break;
                                }
                                if let Some(subscript) = sub_opt {
                                    let raw_idx = shell.eval_arith_expr(subscript);
                                    if crate::expand::take_arith_error() {
                                        had_error = true;
                                        arr.clear();
                                        break;
                                    }
                                    let final_val = if flag_integer {
                                        shell.eval_arith_expr(val_raw).to_string()
                                    } else {
                                        val_raw.clone()
                                    };
                                    let idx = if raw_idx < 0 {
                                        let eff_len =
                                            crate::interpreter::array_effective_len(&arr) as i64;
                                        let computed = eff_len + raw_idx;
                                        if computed < 0 {
                                            0usize
                                        } else {
                                            computed as usize
                                        }
                                    } else {
                                        raw_idx as usize
                                    };
                                    while arr.len() <= idx {
                                        arr.push(None);
                                    }
                                    arr[idx] = Some(final_val);
                                    next_idx = idx + 1;
                                } else {
                                    let final_val = if flag_integer {
                                        shell.eval_arith_expr(val_raw).to_string()
                                    } else {
                                        val_raw.clone()
                                    };
                                    while arr.len() <= next_idx {
                                        arr.push(None);
                                    }
                                    arr[next_idx] = Some(final_val);
                                    next_idx += 1;
                                }
                            }
                            if !had_error && (flag_uppercase || flag_lowercase || flag_capitalize) {
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
                            if flag_integer {
                                shell.integer_vars.insert(name.to_string());
                            }
                            shell.arrays.insert(name.to_string(), arr);
                        } else {
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
                        }
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
                // When the name is a nameref with an empty target, validate
                // the RAW value as a nameref target BEFORE arithmetic evaluation.
                // At global scope, bash uses "not a valid identifier" and removes
                // the nameref. In function scope, bash uses "invalid variable name
                // for name reference" and preserves the nameref.
                if let Some(target) = shell.namerefs.get(name)
                    && target.is_empty()
                    && !crate::interpreter::is_valid_nameref_target(value)
                {
                    if shell.local_scopes.is_empty() {
                        eprintln!(
                            "{}: {}: `{}': not a valid identifier",
                            shell.error_prefix(),
                            cmd_name,
                            value
                        );
                        // Remove the nameref entirely (bash behavior at global scope:
                        // variable becomes "not found" after this error)
                        shell.namerefs.remove(name);
                    } else {
                        eprintln!(
                            "{}: {}: `{}': invalid variable name for name reference",
                            shell.error_prefix(),
                            cmd_name,
                            value
                        );
                        // In function scope, bash preserves the nameref
                    }
                    status = 1;
                } else {
                    // Mark as integer and evaluate as arithmetic
                    shell.integer_vars.insert(name.to_string());
                    let n = shell.eval_arith_expr(value);
                    if is_append {
                        let existing = get_existing_through_nameref(shell, name)
                            .and_then(|v| v.parse::<i64>().ok())
                            .unwrap_or(0);
                        shell.set_var(name, (existing + n).to_string());
                    } else {
                        shell.set_var(name, n.to_string());
                    }
                }
            } else if is_append {
                // Check if variable already has integer attribute.
                // When name is a nameref to a subscripted target like "a[0]",
                // resolve_nameref returns "a[0]" — we need to check the BASE
                // name ("a") for the integer attribute, not "a[0]".
                let resolved_for_int = shell.resolve_nameref(name);
                let resolved_base_for_int = if let Some(bracket) = resolved_for_int.find('[') {
                    &resolved_for_int[..bracket]
                } else {
                    resolved_for_int.as_str()
                };
                if shell.integer_vars.contains(name)
                    || shell.integer_vars.contains(&resolved_for_int)
                    || shell.integer_vars.contains(resolved_base_for_int)
                {
                    let existing_str =
                        get_existing_through_nameref(shell, name).unwrap_or_default();
                    let existing = shell.eval_arith_expr(&existing_str);
                    let addend = shell.eval_arith_expr(value);
                    shell.set_var(name, (existing + addend).to_string());
                } else {
                    let existing = get_existing_through_nameref(shell, name).unwrap_or_default();
                    shell.set_var(name, format!("{}{}", existing, value));
                }
            } else {
                // If the variable is already an indexed or associative array
                // and the value looks like a compound assignment (...), treat
                // it as compound (bash behavior: `a=(1 2 3); declare a='(4 5 6)'`
                // re-assigns as compound, not scalar).
                let resolved_for_check = shell.resolve_nameref(name);
                let trimmed_val = value.trim();
                let looks_compound = trimmed_val.starts_with('(') && trimmed_val.ends_with(')');
                // When a compound assignment goes through a nameref to a target
                // that doesn't exist yet (not an array or assoc), create an
                // indexed array on the target (bash behavior: `declare -n ref=var;
                // declare ref=(X)` creates `var` as indexed array).
                let target_is_new_through_nameref = looks_compound
                    && resolved_for_check != name
                    && !shell.arrays.contains_key(&resolved_for_check)
                    && !shell.assoc_arrays.contains_key(&resolved_for_check);
                if looks_compound
                    && (shell.arrays.contains_key(&resolved_for_check)
                        || target_is_new_through_nameref)
                {
                    let needs_shell_expand = value.contains('$') || value.contains('`');
                    if needs_shell_expand {
                        let inner = &trimmed_val[1..trimmed_val.len() - 1];
                        let ifs = shell
                            .vars
                            .get("IFS")
                            .cloned()
                            .unwrap_or_else(|| " \t\n".to_string());
                        let word = crate::lexer::lex_compound_array_content(inner);
                        let expanded = shell.expand_word_fields(&word, &ifs);
                        let mut arr: Vec<Option<String>> = expanded.into_iter().map(Some).collect();
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
                        // Remove any scalar value on the target so it becomes a pure array
                        shell.vars.remove(&resolved_for_check);
                        shell.arrays.insert(resolved_for_check, arr);
                    } else {
                        let mut arr = crate::builtins::parse_indexed_compound_assignment(value);
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
                        // Remove any scalar value on the target so it becomes a pure array
                        shell.vars.remove(&resolved_for_check);
                        shell.arrays.insert(resolved_for_check, arr);
                    }
                } else if looks_compound && shell.assoc_arrays.contains_key(&resolved_for_check) {
                    let needs_shell_expand = value.contains('$') || value.contains('`');
                    if needs_shell_expand {
                        let inner = &trimmed_val[1..trimmed_val.len() - 1];
                        let word = crate::lexer::lex_compound_array_content(inner);
                        // Split into sub-words and expand each individually
                        // (no IFS splitting) for assoc key-value pairing.
                        let sub_words = split_word_on_whitespace(&word);
                        let expanded: Vec<String> = sub_words
                            .iter()
                            .map(|w| shell.expand_word_single(w))
                            .filter(|s| !s.is_empty())
                            .collect();
                        let mut map = crate::interpreter::AssocArray::default();
                        let mut iter = expanded.into_iter();
                        while let Some(key) = iter.next() {
                            let val = iter.next().unwrap_or_default();
                            map.insert(key, val);
                        }
                        shell.assoc_arrays.insert(resolved_for_check, map);
                    } else {
                        let map = parse_assoc_literal(value);
                        shell.assoc_arrays.insert(resolved_for_check, map);
                    }
                } else {
                    // When assigning through a nameref with an empty target
                    // (e.g. `typeset -n foo; typeset foo=12345`), validate
                    // the value as a nameref target. At global scope, bash uses
                    // "not a valid identifier" and removes the nameref. In
                    // function scope, bash uses "invalid variable name for name
                    // reference" and preserves the nameref.
                    if let Some(target) = shell.namerefs.get(name)
                        && target.is_empty()
                        && !crate::interpreter::is_valid_nameref_target(value)
                    {
                        if shell.local_scopes.is_empty() {
                            eprintln!(
                                "{}: {}: `{}': not a valid identifier",
                                shell.error_prefix(),
                                cmd_name,
                                value
                            );
                            // Remove the nameref entirely (bash behavior at global
                            // scope: variable becomes "not found" after this error)
                            shell.namerefs.remove(name);
                        } else {
                            eprintln!(
                                "{}: {}: `{}': invalid variable name for name reference",
                                shell.error_prefix(),
                                cmd_name,
                                value
                            );
                            // In function scope, bash preserves the nameref
                        }
                        status = 1;
                    } else {
                        shell.set_var(name, value.to_string());
                    }
                }
            }

            if flag_readonly {
                shell.readonly_vars.insert(name.to_string());
            }
            if flag_export {
                if shell.namerefs.contains_key(name) {
                    // Exported namerefs: export the nameref variable itself with
                    // the target name as the environment value (bash behavior).
                    let target = shell.namerefs.get(name).cloned().unwrap_or_default();
                    shell.exports.insert(name.to_string(), target.clone());
                    unsafe { std::env::set_var(name, &target) };
                } else {
                    let val = shell.get_var(name).unwrap_or_default();
                    shell.exports.insert(name.to_string(), val.clone());
                    unsafe { std::env::set_var(name, &val) };
                }
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

            // declare -g fixup: propagate the newly-assigned value to the
            // global scope (the saved value in the first local_scopes entry
            // that has this variable), then restore the local value that was
            // in place before the assignment.
            if let Some((
                gname,
                prev_scalar,
                prev_array,
                prev_assoc,
                prev_integer,
                prev_declared_unset,
            )) = global_fixup
            {
                // Read the new value that was just assigned to the current maps
                let new_scalar = shell.vars.get(&gname).cloned();
                let new_array = shell.arrays.get(&gname).cloned();
                let new_assoc = shell.assoc_arrays.get(&gname).cloned();
                let new_integer = shell.integer_vars.contains(&gname);
                let new_declared_unset = shell.declared_unset.contains(&gname);

                // Propagate the new value to the global scope by updating the
                // saved value in the first (bottom-most) scope that has it.
                if let Some(assoc) = new_assoc {
                    shell.set_global_assoc(&gname, assoc);
                } else if let Some(arr) = new_array {
                    shell.set_global_array(&gname, arr);
                } else if let Some(ref val) = new_scalar {
                    shell.set_global_var(&gname, val.clone());
                } else {
                    // Variable was unset or declared-but-unset at current scope;
                    // propagate declared_unset to global if needed.
                    // Clear any saved scalar/array/assoc at global scope.
                    for scope in shell.local_scopes.iter_mut() {
                        if let Some(saved) = scope.get_mut(&gname) {
                            saved.scalar = None;
                            saved.array = None;
                            saved.assoc = None;
                            break;
                        }
                    }
                }
                // Propagate integer attribute to global scope
                if new_integer {
                    shell.set_global_attr_integer(&gname, true);
                }
                // Propagate declared_unset to global scope
                if new_declared_unset {
                    shell.set_global_declared_unset(&gname, true);
                }

                // Restore the local value (what was in the maps before the
                // assignment) so that the enclosing function's local is unchanged.
                match prev_scalar {
                    Some(val) => {
                        shell.vars.insert(gname.clone(), val);
                    }
                    None => {
                        shell.vars.remove(&gname);
                    }
                }
                match prev_array {
                    Some(arr) => {
                        shell.arrays.insert(gname.clone(), arr);
                    }
                    None => {
                        shell.arrays.remove(&gname);
                    }
                }
                match prev_assoc {
                    Some(assoc) => {
                        shell.assoc_arrays.insert(gname.clone(), assoc);
                    }
                    None => {
                        shell.assoc_arrays.remove(&gname);
                    }
                }
                if prev_integer {
                    shell.integer_vars.insert(gname.clone());
                } else {
                    shell.integer_vars.remove(&gname);
                }
                if prev_declared_unset {
                    shell.declared_unset.insert(gname.clone());
                } else {
                    shell.declared_unset.remove(&gname);
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

            // When the variable is a nameref and we're applying attributes
            // (not -n itself), resolve through the nameref chain so that
            // attributes like -i, -r, -x, -a, -A are applied to the target.
            // e.g. `declare -n foo=bar; declare -i foo` → applies -i to bar.
            // But when -n IS set (e.g. `declare -rn foo`), attributes apply
            // to the nameref variable itself, not the target.
            //
            // Special case: when -a or -A is set (array/assoc) and the nameref
            // has NO target (empty), bash silently removes the nameref attribute
            // and creates the array on the variable itself. When the nameref HAS
            // a target, bash resolves through it (applies -a/-A to the target).
            // The "warning: removing nameref attribute" is only emitted when
            // there's a subscript assignment (handled in the subscripted path above).
            let has_attr_flags = flag_integer
                || flag_readonly
                || flag_export
                || flag_array
                || flag_assoc
                || flag_uppercase
                || flag_lowercase
                || flag_capitalize
                || flag_trace
                || flag_unset_readonly;
            let resolved_name: String;
            let attr_name: &str =
                if !flag_nameref && has_attr_flags && shell.namerefs.contains_key(name) {
                    if (flag_array || flag_assoc)
                        && shell.namerefs.get(name).is_none_or(|t| t.is_empty())
                    {
                        // Nameref with no target — silently remove nameref
                        // and use the variable itself for array creation.
                        shell.namerefs.remove(name);
                        name
                    } else {
                        resolved_name = shell.resolve_nameref(name);
                        // If the resolved nameref target contains a subscript
                        // (e.g. XXX[0]), use the base name for attribute
                        // application (e.g. XXX), matching bash behavior.
                        if let Some(bracket) = resolved_name.find('[') {
                            &resolved_name[..bracket]
                        } else {
                            &resolved_name
                        }
                    }
                } else {
                    name
                };

            // Can't remove readonly attribute — check on the resolved target
            // (for namerefs, `declare +r ref` follows the nameref to the target;
            // if the target isn't readonly, no error even if the nameref itself is).
            // When a nameref has no target (resolves to itself), `+r` is a no-op.
            // Exception: when `-n` is also specified (`declare +r -n ref`), we're
            // operating on the nameref itself, so check readonly on the nameref.
            let nameref_resolved_to_self =
                !flag_nameref && shell.namerefs.contains_key(name) && attr_name == name;
            if flag_unset_readonly
                && shell.readonly_vars.contains(attr_name)
                && !nameref_resolved_to_self
            {
                eprintln!(
                    "{}: {}: {}: readonly variable",
                    shell.error_prefix(),
                    cmd_name,
                    attr_name
                );
                status = 1;
                continue;
            }

            if make_local {
                shell.declare_local(name);
                // When attributes are applied through a nameref (attr_name
                // differs from name), also declare_local the target so it
                // gets saved/restored on scope exit.  Without this, the
                // target variable leaks out of the function scope.
                // e.g. `declare -n ref=var; declare -a ref` should make
                // `var` local too, not just `ref`.
                if attr_name != name {
                    shell.declare_local(attr_name);
                }
                // When `declare ref` (no flags) is called on a nameref,
                // attr_name == name because has_attr_flags is false.
                // But the nameref target should still be declared local
                // so that assignments through the nameref (e.g. `ref=X`)
                // don't leak to the global scope.
                if !has_attr_flags && shell.namerefs.contains_key(name) {
                    let resolved = shell.resolve_nameref(name);
                    if resolved != name {
                        shell.declare_local(&resolved);
                    }
                }
                // When a prefix assignment set this variable as a temp env
                // var (e.g. `z=y typeset z`), declare_local just saved the
                // prefix value and cleared the variable.  Restore the prefix
                // value so the local inherits it (matching bash behavior
                // where `z=y typeset z` makes local z have value y).
                if shell.temp_env_vars.contains(name)
                    && !shell.vars.contains_key(name)
                    && !shell.arrays.contains_key(name)
                    && !shell.assoc_arrays.contains_key(name)
                {
                    // The saved value in the scope is what declare_local
                    // captured (the prefix assignment value).  Restore it
                    // to the current maps so the local has the value.
                    if let Some(scope) = shell.local_scopes.last()
                        && let Some(saved) = scope.get(name)
                        && let Some(ref val) = saved.scalar
                    {
                        shell.vars.insert(name.to_string(), val.clone());
                        shell.declared_unset.remove(name);
                    }
                }
            }
            if flag_nameref && !flag_array && !flag_assoc {
                // `typeset -n foo` (no value): if foo is already a nameref,
                // this is a no-op (keep the existing target). Otherwise, use
                // foo's current value as the nameref target, then remove it
                // from regular vars.  This matches bash: `foo=bar; typeset -n foo`
                // → foo is a nameref to "bar".
                // When combined with other flags (e.g. `declare -rn foo`),
                // the nameref is created AND the other attributes are applied
                // to the nameref variable itself, not the target.
                //
                // Reject if the original name had a subscript (e.g.
                // `declare -n array[128]`) — namerefs cannot be array elements.
                // Also reject if the variable is already an array or assoc array.
                // Also reject if the variable is already readonly.
                let is_subscripted_nameref = had_subscript;
                let is_existing_array =
                    shell.arrays.contains_key(name) || shell.assoc_arrays.contains_key(name);
                let is_existing_readonly = !shell.namerefs.contains_key(name)
                    && shell.readonly_vars.contains(name);
                if is_existing_readonly {
                    eprintln!(
                        "{}: {}: {}: readonly variable",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                    eprintln!("{}: readonly variable", name);
                    status = 1;
                    continue;
                } else if is_subscripted_nameref {
                    eprintln!(
                        "{}: {}: {}: reference variable cannot be an array",
                        shell.error_prefix(),
                        cmd_name,
                        name_arg
                    );
                    status = 1;
                } else if is_existing_array && !shell.namerefs.contains_key(name) {
                    eprintln!(
                        "{}: {}: {}: reference variable cannot be an array",
                        shell.error_prefix(),
                        cmd_name,
                        name
                    );
                    status = 1;
                } else if shell.namerefs.contains_key(name) {
                    // Already a nameref — no-op for the nameref part (bash behavior)
                    // Other attribute flags (readonly, etc.) will be applied below.
                } else {
                    let target_opt = shell.vars.remove(name);
                    let target = target_opt.clone().unwrap_or_default();
                    // Validate that the target is a valid variable name.
                    // If the variable had an explicit value (even empty string),
                    // validate it. `r=""; declare -n r` → bash rejects empty as
                    // invalid nameref target. Truly unset → creates unbound nameref.
                    let had_explicit_value = target_opt.is_some();
                    if had_explicit_value
                        && (target.is_empty()
                            || !crate::interpreter::is_valid_nameref_target(&target))
                    {
                        eprintln!(
                            "{}: {}: `{}': invalid variable name for name reference",
                            shell.error_prefix(),
                            cmd_name,
                            target
                        );
                        // Put the value back since we're not creating the nameref
                        shell.vars.insert(name.to_string(), target);
                        status = 1;
                    } else {
                        // Detect circular nameref
                        let target_base = if let Some(bracket) = target.find('[') {
                            &target[..bracket]
                        } else {
                            target.as_str()
                        };
                        if !target.is_empty() && target_base == name {
                            if !shell.local_scopes.is_empty() {
                                eprintln!(
                                    "{}: {}: warning: {}: circular name reference",
                                    shell.error_prefix(),
                                    cmd_name,
                                    name
                                );
                                // Still create the nameref in function scope
                                if !flag_integer {
                                    shell.integer_vars.remove(name);
                                }
                                shell.namerefs.insert(name.to_string(), target);
                            } else {
                                eprintln!(
                                    "{}: {}: warning: {}: circular name reference",
                                    shell.error_prefix(),
                                    cmd_name,
                                    name
                                );
                                // Put the value back since we're not creating the nameref
                                shell.vars.insert(name.to_string(), target);
                            }
                        } else {
                            // When converting a non-nameref variable to a nameref,
                            // remove the integer attribute (bash behavior:
                            // `declare -i x; declare -n x=foo` removes -i from x).
                            if !flag_integer {
                                shell.integer_vars.remove(name);
                            }
                            shell.namerefs.insert(name.to_string(), target);
                        }
                    }
                } // end of !already-nameref block
                // When -n is combined with other attribute flags like -r, -i, -x,
                // fall through to apply those attributes below (to `name` itself,
                // since attr_name == name when flag_nameref is set).
            }
            if !flag_nameref && flag_assoc {
                // Error if trying to convert an existing indexed array to assoc
                if shell.arrays.contains_key(attr_name) {
                    if flag_global && let Some(func) = shell.func_names.last() {
                        eprintln!(
                            "{}: {}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            func,
                            attr_name
                        );
                    }
                    if shell.func_names.is_empty() && !flag_global {
                        eprintln!(
                            "{}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            attr_name
                        );
                    } else {
                        eprintln!(
                            "{}: {}: {}: cannot convert indexed to associative array",
                            shell.error_prefix(),
                            cmd_name,
                            attr_name
                        );
                    }
                    status = 1;
                } else if !shell.assoc_arrays.contains_key(attr_name) {
                    if make_local {
                        let inherit_enabled = *shell
                            .shopt_options
                            .get("localvar_inherit")
                            .unwrap_or(&false);
                        let mut new_map = crate::interpreter::AssocArray::default();
                        if inherit_enabled {
                            // With localvar_inherit, convert inherited scalar
                            // to assoc key "0" = value (bash behavior).
                            if let Some(val) = shell.vars.remove(attr_name) {
                                new_map.insert("0".to_string(), val);
                            } else {
                                shell.declared_unset.insert(attr_name.to_string());
                            }
                        } else {
                            shell.vars.remove(attr_name);
                            shell.declared_unset.insert(attr_name.to_string());
                        }
                        shell.assoc_arrays.insert(attr_name.to_string(), new_map);
                    } else {
                        let mut new_map = crate::interpreter::AssocArray::default();
                        if let Some(val) = shell.vars.remove(attr_name) {
                            new_map.insert("0".to_string(), val);
                        } else {
                            shell.declared_unset.insert(attr_name.to_string());
                        }
                        shell.assoc_arrays.insert(attr_name.to_string(), new_map);
                    }
                }
            } else if !flag_nameref && flag_array {
                // Error if trying to convert an existing assoc array to indexed
                if shell.assoc_arrays.contains_key(attr_name) {
                    if flag_global && let Some(func) = shell.func_names.last() {
                        eprintln!(
                            "{}: {}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            func,
                            attr_name
                        );
                    }
                    if shell.func_names.is_empty() && !flag_global {
                        eprintln!(
                            "{}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            attr_name
                        );
                    } else {
                        eprintln!(
                            "{}: {}: {}: cannot convert associative to indexed array",
                            shell.error_prefix(),
                            cmd_name,
                            attr_name
                        );
                    }
                    status = 1;
                } else if !shell.arrays.contains_key(attr_name) {
                    if make_local {
                        let inherit_enabled = *shell
                            .shopt_options
                            .get("localvar_inherit")
                            .unwrap_or(&false);
                        if inherit_enabled {
                            // With localvar_inherit, convert inherited scalar
                            // to array element [0] = value (bash behavior).
                            if let Some(val) = shell.vars.remove(attr_name) {
                                shell.arrays.insert(attr_name.to_string(), vec![Some(val)]);
                            } else {
                                shell.arrays.insert(attr_name.to_string(), vec![]);
                                shell.declared_unset.insert(attr_name.to_string());
                            }
                        } else {
                            shell.vars.remove(attr_name);
                            shell.arrays.insert(attr_name.to_string(), vec![]);
                            shell.declared_unset.insert(attr_name.to_string());
                        }
                    } else if let Some(val) = shell.vars.remove(attr_name) {
                        shell.arrays.insert(attr_name.to_string(), vec![Some(val)]);
                    } else {
                        shell.arrays.entry(attr_name.to_string()).or_default();
                        shell.declared_unset.insert(attr_name.to_string());
                    }
                }
            } else if !flag_nameref && had_subscript && !shell.arrays.contains_key(attr_name) {
                // declare -r c[100] (with subscript, no -a flag): create array
                // If the variable was previously a scalar, preserve its value as [0]
                // (bash behavior: `array=one; declare -i array[64]` → array[0]="one")
                // The preserved value should NOT be re-evaluated as arithmetic even
                // when -i is set — only future assignments go through arithmetic eval.
                if let Some(val) = shell.vars.remove(attr_name) {
                    shell.arrays.insert(attr_name.to_string(), vec![Some(val)]);
                    // Mark as already-integer so the re-evaluation below is skipped
                    if flag_integer {
                        shell.integer_vars.insert(attr_name.to_string());
                    }
                } else {
                    shell.arrays.entry(attr_name.to_string()).or_default();
                    shell.declared_unset.insert(attr_name.to_string());
                }
            } else if !flag_nameref
                && !shell.arrays.contains_key(attr_name)
                && !shell.assoc_arrays.contains_key(attr_name)
                && !shell.namerefs.contains_key(attr_name)
            {
                // declare without = marks the variable as declared-but-unset
                // (don't insert into vars — bash distinguishes this from "")
                if make_local && shell.vars.contains_key(attr_name) {
                    // In function scope: `declare v` (no =) creates a local
                    // declared-but-unset variable that shadows the global.
                    // Exception: if the variable was set via temp env prefix
                    // (`v=t f`), bash inherits the value.
                    if !shell.temp_env_vars.contains(attr_name) {
                        shell.vars.remove(attr_name);
                        shell.declared_unset.insert(attr_name.to_string());
                        // Also remove from process environment so that the
                        // expansion fallback (`std::env::var`) doesn't leak
                        // the exported value.  The scope restore on function
                        // exit will re-export it if needed.
                        if shell.exports.contains_key(attr_name) {
                            unsafe { std::env::remove_var(attr_name) };
                        }
                    }
                } else if !shell.vars.contains_key(attr_name) {
                    shell.declared_unset.insert(attr_name.to_string());
                }
            }

            if flag_integer {
                let was_integer = shell.integer_vars.contains(attr_name);
                shell.integer_vars.insert(attr_name.to_string());
                if !was_integer && let Some(arr) = shell.arrays.get(attr_name).cloned() {
                    let evaluated: Vec<Option<String>> = arr
                        .into_iter()
                        .map(|v| v.map(|s| shell.eval_arith_expr(&s).to_string()))
                        .collect();
                    shell.arrays.insert(attr_name.to_string(), evaluated);
                }
            }
            if flag_readonly {
                shell.readonly_vars.insert(attr_name.to_string());
            }
            if flag_export {
                // For declared-but-unset variables, mark as exported without
                // storing an empty value (bash shows `declare -ix foo6` not
                // `declare -ix foo6=""`).
                if shell.declared_unset.contains(attr_name) && !shell.vars.contains_key(attr_name) {
                    shell.exports.insert(attr_name.to_string(), String::new());
                    // Don't set env var — variable is unset
                } else {
                    let val = shell.get_var(attr_name).unwrap_or_default();
                    shell.exports.insert(attr_name.to_string(), val.clone());
                    unsafe { std::env::set_var(attr_name, &val) };
                }
            }
            if flag_uppercase {
                shell.uppercase_vars.insert(attr_name.to_string());
                shell.lowercase_vars.remove(attr_name);
                if let Some(v) = shell.vars.get(attr_name).cloned() {
                    shell.vars.insert(attr_name.to_string(), v.to_uppercase());
                }
                if let Some(arr) = shell.arrays.get_mut(attr_name) {
                    for val in arr.iter_mut().flatten() {
                        *val = val.to_uppercase();
                    }
                }
            }
            if flag_lowercase {
                shell.lowercase_vars.insert(attr_name.to_string());
                shell.uppercase_vars.remove(attr_name);
                shell.capitalize_vars.remove(attr_name);
                if let Some(v) = shell.vars.get(attr_name).cloned() {
                    shell.vars.insert(attr_name.to_string(), v.to_lowercase());
                }
                if let Some(arr) = shell.arrays.get_mut(attr_name) {
                    for val in arr.iter_mut().flatten() {
                        *val = val.to_lowercase();
                    }
                }
            }
            if flag_capitalize {
                shell.capitalize_vars.insert(attr_name.to_string());
                shell.uppercase_vars.remove(attr_name);
                shell.lowercase_vars.remove(attr_name);
                if let Some(v) = shell.vars.get(attr_name).cloned() {
                    let cap = capitalize_string(&v);
                    shell.vars.insert(attr_name.to_string(), cap);
                }
                if let Some(arr) = shell.arrays.get_mut(attr_name) {
                    for val in arr.iter_mut().flatten() {
                        *val = capitalize_string(val);
                    }
                }
            }
        }
    }
    status
}

/// Escape `$` and backticks in nameref target values for `declare -p` output.
/// Bash backslash-escapes `$` and `` ` `` in nameref targets so that the output
/// is re-evaluable (e.g. `declare -n foo="x[\$zero]"` instead of `x[$zero]`).
/// Get the existing value of a variable, resolving through namerefs.
/// Handles nameref targets that are array elements (e.g. "bar[0]").
fn get_existing_through_nameref(
    shell: &mut crate::interpreter::Shell,
    name: &str,
) -> Option<String> {
    let resolved = shell.resolve_nameref(name);
    // If the resolved name contains a subscript (e.g. "bar[0]"), look up
    // the array element value.
    if let Some(bracket) = resolved.find('[')
        && resolved.ends_with(']')
    {
        let base = &resolved[..bracket];
        let subscript = &resolved[bracket + 1..resolved.len() - 1];
        if shell.assoc_arrays.contains_key(base) {
            return shell
                .assoc_arrays
                .get(base)
                .and_then(|m| m.get(subscript))
                .cloned();
        } else if shell.arrays.contains_key(base) {
            let idx = shell.eval_arith_expr(subscript);
            let arr = shell.arrays.get(base)?;
            let eff_len = crate::interpreter::array_effective_len(arr) as i64;
            let actual_idx = if idx < 0 {
                let computed = eff_len + idx;
                if computed < 0 {
                    0usize
                } else {
                    computed as usize
                }
            } else {
                idx as usize
            };
            return arr.get(actual_idx).and_then(|v| v.clone());
        }
    }
    // Check if resolved name is an array — get element [0]
    if shell.arrays.contains_key(&resolved) {
        return shell
            .arrays
            .get(&resolved)
            .and_then(|a| a.first())
            .and_then(|v| v.clone());
    }
    shell.vars.get(&resolved).cloned()
}

fn escape_nameref_target(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '$' | '`' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result
}

pub fn parse_assoc_literal(s: &str) -> crate::interpreter::AssocArray {
    parse_assoc_literal_with_buckets(s, 1024)
}

/// Like `parse_assoc_literal` but with a configurable number of hash buckets.
/// Bash uses 128 buckets when converting an existing variable to assoc
/// (`convert_var_to_assoc` → `assoc_create(0)` → `DEFAULT_HASH_BUCKETS`),
/// and 1024 buckets when creating a brand-new assoc variable
/// (`make_new_assoc_variable` → `assoc_create(ASSOC_HASH_BUCKETS)`).
pub fn parse_assoc_literal_with_buckets(
    s: &str,
    nbuckets: usize,
) -> crate::interpreter::AssocArray {
    let trimmed = s.trim();
    let inner = if trimmed.starts_with('(') && trimmed.ends_with(')') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    let mut map = crate::interpreter::AssocArray::new_with_buckets(nbuckets);
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

/// Split a lexed Word into sub-words by splitting on whitespace found in
/// Literal parts.  This is used for assoc compound assignment expansion
/// where each `$var` should expand as a single token (no IFS splitting).
fn split_word_on_whitespace(word: &[crate::ast::WordPart]) -> Vec<Vec<crate::ast::WordPart>> {
    use crate::ast::WordPart;
    let mut result: Vec<Vec<WordPart>> = vec![vec![]];
    for part in word {
        if let WordPart::Literal(s) = part {
            // Split this literal on whitespace; each piece goes into a
            // separate sub-word.
            let mut first = true;
            for segment in s.split_whitespace() {
                if !first {
                    // Start a new sub-word
                    result.push(vec![]);
                }
                first = false;
                result
                    .last_mut()
                    .unwrap()
                    .push(WordPart::Literal(segment.to_string()));
            }
            // If the literal was purely whitespace (or ended with whitespace),
            // we may need to start a new sub-word for the next non-literal part.
            if s.ends_with(|c: char| c.is_whitespace()) && !s.trim().is_empty() {
                result.push(vec![]);
            } else if s.trim().is_empty() && !s.is_empty() {
                // Pure whitespace literal — start new sub-word if current is non-empty
                if !result.last().unwrap().is_empty() {
                    result.push(vec![]);
                }
            }
        } else {
            // Non-literal parts (Variable, CommandSub, etc.) go into
            // the current sub-word as-is.
            result.last_mut().unwrap().push(part.clone());
        }
    }
    // Filter out empty sub-words
    result.into_iter().filter(|w| !w.is_empty()).collect()
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
