use super::*;

/// Check if a ParamExpr is an array[@] expansion.
pub(super) fn is_array_at_expansion(expr: &ParamExpr, ctx: &ExpCtx) -> bool {
    // Check for $@ or $* with Substring (slice) operation
    if (expr.name == "@" || expr.name == "*")
        && matches!(&expr.op, ParamOp::Substring(..))
        && ctx.positional.len() > 1
    {
        return true;
    }
    // ${!arr[@]} — array indices should split into separate fields
    if matches!(&expr.op, ParamOp::ArrayIndices('@')) {
        return ctx.arrays.contains_key(&expr.name) || ctx.assoc_arrays.contains_key(&expr.name);
    }
    if let Some(bracket) = expr.name.find('[') {
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        if idx_str == "@" {
            let base = &expr.name[..bracket];
            let resolved = ctx.resolve_nameref(base);
            // Array[@] with any operation should expand per-element
            if !matches!(&expr.op, ParamOp::Length) {
                return ctx.arrays.contains_key(&resolved)
                    || ctx.assoc_arrays.contains_key(&resolved);
            }
        }
    }
    false
}

/// Get array elements for an array[@] expansion.
pub(super) fn get_array_elements(expr: &ParamExpr, ctx: &ExpCtx) -> Vec<String> {
    // Handle ${@:offset:length} — slice of positional params
    if expr.name == "@" || expr.name == "*" {
        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
            let params = if offset == 0 {
                ctx.positional
            } else {
                &ctx.positional[1..]
            };
            let count = params.len();
            let start = if offset < 0 {
                (count as i64 + offset).max(0) as usize
            } else if offset == 0 {
                0
            } else {
                ((offset - 1) as usize).min(count)
            };
            let end = if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(count as i64);
                if len < 0 {
                    (count as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(count)
                }
            } else {
                count
            };
            return params[start..end].to_vec();
        }
        // Plain $@ — all positional params
        if ctx.positional.len() > 1 {
            return ctx.positional[1..].to_vec();
        }
        return vec![];
    }
    // ${!arr[@]} — return indices/keys as elements
    if let ParamOp::ArrayIndices(_) = &expr.op {
        let resolved = ctx.resolve_nameref(&expr.name);
        if let Some(arr) = ctx.arrays.get(&resolved) {
            return (0..arr.len())
                .filter(|&i| arr[i].is_some())
                .map(|i| i.to_string())
                .collect();
        }
        if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
            return assoc.keys().cloned().collect();
        }
        return vec![];
    }
    if let Some(bracket) = expr.name.find('[') {
        let base = &expr.name[..bracket];
        let resolved = ctx.resolve_nameref(base);
        if let Some(arr) = ctx.arrays.get(&resolved) {
            return arr.iter().filter_map(|v| v.clone()).collect();
        }
        if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
            return assoc.values().cloned().collect();
        }
        // Fall back to scalar as single element
        if let Some(val) = ctx.vars.get(&resolved) {
            return vec![val.clone()];
        }
    }
    vec![]
}

pub(super) fn lookup_var(name: &str, ctx: &ExpCtx) -> String {
    match name {
        "?" => ctx.last_status.to_string(),
        "$" => ctx.top_level_pid.to_string(),
        "RANDOM" => crate::expand::next_random().to_string(),
        "BASHPID" => std::process::id().to_string(),
        "SRANDOM" => {
            // Secure random 32-bit number
            let mut buf = [0u8; 4];
            #[cfg(unix)]
            {
                use std::io::Read;
                if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
                    let _ = f.read_exact(&mut buf);
                }
            }
            u32::from_ne_bytes(buf).to_string()
        }
        "BASH_SUBSHELL" => ctx
            .vars
            .get("BASH_SUBSHELL")
            .cloned()
            .unwrap_or_else(|| "0".to_string()),
        "SECONDS" => {
            use std::sync::OnceLock;
            static START: OnceLock<std::time::Instant> = OnceLock::new();
            let start = START.get_or_init(std::time::Instant::now);
            start.elapsed().as_secs().to_string()
        }
        "EPOCHSECONDS" => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string(),
        "EPOCHREALTIME" => {
            let d = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            format!("{}.{:06}", d.as_secs(), d.subsec_micros())
        }
        "BASH_COMMAND" => ctx.vars.get("BASH_COMMAND").cloned().unwrap_or_default(),
        "#" => {
            let count = if ctx.positional.is_empty() {
                0
            } else {
                ctx.positional.len() - 1
            };
            count.to_string()
        }
        "0" => ctx.positional.first().cloned().unwrap_or_default(),
        "*" => {
            if ctx.positional.len() > 1 {
                // $* joins with first char of IFS (space if IFS unset, empty if IFS="")
                let ifs = ctx.vars.get("IFS");
                let sep = match ifs {
                    None => " ".to_string(),
                    Some(s) if s.is_empty() => String::new(),
                    Some(s) => s.chars().next().unwrap_or(' ').to_string(),
                };
                ctx.positional[1..].join(&sep)
            } else {
                String::new()
            }
        }
        "@" => {
            // $@ always joins with space in lookup context
            // (actual field splitting into separate args is handled by callers)
            if ctx.positional.len() > 1 {
                ctx.positional[1..].join(" ")
            } else {
                String::new()
            }
        }
        "-" => ctx.opt_flags.to_string(),
        "!" => {
            if ctx.last_bg_pid > 0 {
                ctx.last_bg_pid.to_string()
            } else {
                String::new()
            }
        }
        _ => {
            // Numeric positional parameters: $1, ${10}, etc.
            if let Ok(n) = name.parse::<usize>() {
                if n < ctx.positional.len() {
                    return ctx.positional[n].clone();
                }
                return String::new();
            }

            // Check for array subscript: name[idx]
            if let Some(bracket) = name.find('[') {
                let base = &name[..bracket];
                let idx_str = &name[bracket + 1..name.len() - 1];
                let resolved = ctx.resolve_nameref(base);

                return match idx_str {
                    "@" | "*" => {
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            arr.iter()
                                .filter_map(|v| v.as_ref())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" ")
                        } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                            assoc.values().cloned().collect::<Vec<_>>().join(" ")
                        } else if let Some(val) = ctx.vars.get(&resolved) {
                            val.clone()
                        } else {
                            String::new()
                        }
                    }
                    _ => {
                        // Check associative array first (string key)
                        if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                            // Strip surrounding quotes from subscript key
                            // e.g., ${arr["0"]} → key "0", ${arr['key']} → key "key"
                            let stripped_key = if idx_str.len() >= 2
                                && ((idx_str.starts_with('"') && idx_str.ends_with('"'))
                                    || (idx_str.starts_with('\'') && idx_str.ends_with('\'')))
                            {
                                &idx_str[1..idx_str.len() - 1]
                            } else {
                                idx_str
                            };
                            return assoc.get(stripped_key).cloned().unwrap_or_default();
                        }
                        // Numeric index for indexed arrays (supports negative)
                        // Expand $var and ${var} references in subscript
                        let expanded_idx = if idx_str.contains('$') {
                            let mut expanded = idx_str.to_string();
                            while let Some(pos) = expanded.find('$') {
                                let rest = &expanded[pos + 1..];
                                if rest.starts_with('{') {
                                    if let Some(close) = rest.find('}') {
                                        let var_name = &rest[1..close];
                                        let var_val =
                                            ctx.vars.get(var_name).cloned().unwrap_or_default();
                                        expanded = format!(
                                            "{}{}{}",
                                            &expanded[..pos],
                                            var_val,
                                            &rest[close + 1..]
                                        );
                                    } else {
                                        break;
                                    }
                                } else {
                                    let var_end = rest
                                        .find(|c: char| !c.is_alphanumeric() && c != '_')
                                        .unwrap_or(rest.len());
                                    let var_name = &rest[..var_end];
                                    let var_val =
                                        ctx.vars.get(var_name).cloned().unwrap_or_default();
                                    expanded = format!(
                                        "{}{}{}",
                                        &expanded[..pos],
                                        var_val,
                                        &rest[var_end..]
                                    );
                                }
                            }
                            expanded
                        } else {
                            idx_str.to_string()
                        };
                        let raw_idx: i64 = expanded_idx.parse().unwrap_or(0);
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            let idx = if raw_idx < 0 {
                                let len = arr.len() as i64;
                                (len + raw_idx).max(0) as usize
                            } else {
                                raw_idx as usize
                            };
                            arr.get(idx).and_then(|v| v.clone()).unwrap_or_default()
                        } else if raw_idx == 0 {
                            ctx.vars.get(&resolved).cloned().unwrap_or_default()
                        } else {
                            String::new()
                        }
                    }
                };
            }

            // Check positional parameters
            if let Ok(n) = name.parse::<usize>() {
                if n < ctx.positional.len() {
                    return ctx.positional[n].clone();
                }
                return String::new();
            }

            // Resolve namerefs
            let resolved = ctx.resolve_nameref(name);

            // Check variables, then environment
            ctx.vars
                .get(&resolved)
                .cloned()
                .or_else(|| {
                    // If it's also an array, return element 0
                    ctx.arrays
                        .get(&resolved)
                        .and_then(|a| a.first().and_then(|v| v.clone()))
                })
                .or_else(|| {
                    // For associative arrays, $var returns element with key "0"
                    ctx.assoc_arrays
                        .get(&resolved)
                        .and_then(|a| a.get("0").cloned())
                })
                .or_else(|| std::env::var(&resolved).ok())
                .unwrap_or_default()
        }
    }
}

/// Parse an arithmetic expression for substring offset/length.
/// If the string is a simple integer, parse it directly. If it's a variable name,
/// resolve it. Otherwise, report an arithmetic error.
fn parse_arith_offset(s: &str, param_name: &str, ctx: &ExpCtx) -> i64 {
    if s.is_empty() {
        return 0;
    }
    // Try direct integer parse first
    if let Ok(v) = s.parse::<i64>() {
        return v;
    }
    // Try as variable name (evaluates to 0 if unset, like in arithmetic)
    let first = s.as_bytes()[0];
    if (first == b'_' || first.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        let val = ctx
            .vars
            .get(s)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        return val;
    }
    // Check if expression starts with a valid arithmetic token (digit, variable, unary op, paren)
    let first_char = s.trim().as_bytes().first().copied().unwrap_or(0);
    if !first_char.is_ascii_alphanumeric()
        && !matches!(first_char, b'_' | b'-' | b'+' | b'!' | b'~' | b'(' | b'$')
    {
        // Not a valid arithmetic expression — report error
        let prefix = EXPAND_ERROR_PREFIX.with(|p| {
            let p = p.borrow();
            if p.is_empty() {
                "bash".to_string()
            } else {
                p.clone()
            }
        });
        eprintln!(
            "{}: {}: {}: arithmetic syntax error: operand expected (error token is \"{}\")",
            prefix, param_name, s, s
        );
        crate::expand::set_arith_error();
        return 0;
    }
    // Use full arithmetic evaluation for complex expressions (ternary, operators, etc.)
    crate::expand::arithmetic::eval_arith_full(
        s,
        ctx.vars,
        &std::collections::HashMap::new(),
        ctx.positional,
        ctx.last_status,
    )
}

/// Apply a parameter operation to a pre-resolved value (for array per-element operations)
pub(super) fn apply_param_op(
    val: &str,
    op: &ParamOp,
    ctx: &ExpCtx,
    cmd_sub: CmdSubFn,
    param_name: &str,
) -> String {
    match op {
        ParamOp::None | ParamOp::Length | ParamOp::Indirect => val.to_string(),
        ParamOp::UpperFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        c.to_uppercase().to_string() + chars.as_str()
                    } else {
                        val.to_string()
                    }
                }
            }
        }
        ParamOp::UpperAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            if pat_str.is_empty() {
                val.to_uppercase()
            } else {
                val.chars()
                    .map(|c| {
                        if char_matches_pattern(c, &pat_str) {
                            c.to_uppercase().collect::<String>()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
        }
        ParamOp::LowerFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        c.to_lowercase().to_string() + chars.as_str()
                    } else {
                        val.to_string()
                    }
                }
            }
        }
        ParamOp::LowerAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            if pat_str.is_empty() {
                val.to_lowercase()
            } else {
                val.chars()
                    .map(|c| {
                        if char_matches_pattern(c, &pat_str) {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
        }
        ParamOp::ToggleFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        let toggled = if c.is_uppercase() {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_uppercase().collect::<String>()
                        };
                        toggled + chars.as_str()
                    } else {
                        val.to_string()
                    }
                }
            }
        }
        ParamOp::ToggleAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            val.chars()
                .map(|c| {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        if c.is_uppercase() {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_uppercase().collect::<String>()
                        }
                    } else {
                        c.to_string()
                    }
                })
                .collect()
        }
        ParamOp::TrimSmallLeft(pattern) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::SmallLeft)
        }
        ParamOp::TrimLargeLeft(pattern) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::LargeLeft)
        }
        ParamOp::TrimSmallRight(pattern) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::SmallRight)
        }
        ParamOp::TrimLargeRight(pattern) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::LargeRight)
        }
        ParamOp::Replace(pattern, replacement)
        | ParamOp::ReplaceAll(pattern, replacement)
        | ParamOp::ReplacePrefix(pattern, replacement)
        | ParamOp::ReplaceSuffix(pattern, replacement) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            let rep = expand_word_nosplit_ctx(replacement, ctx, cmd_sub);
            match op {
                ParamOp::ReplaceAll(..) => pattern_replace(val, &pat, &rep, true),
                ParamOp::ReplacePrefix(..) => {
                    for i in 0..=val.len() {
                        if shell_pattern_match(&val[..i], &pat) {
                            return format!("{}{}", rep, &val[i..]);
                        }
                    }
                    val.to_string()
                }
                ParamOp::ReplaceSuffix(..) => {
                    for i in (0..=val.len()).rev() {
                        if shell_pattern_match(&val[i..], &pat) {
                            return format!("{}{}", &val[..i], rep);
                        }
                    }
                    val.to_string()
                }
                _ => pattern_replace(val, &pat, &rep, false),
            }
        }
        ParamOp::Substring(offset_str, length_str) => {
            let offset: i64 = parse_arith_offset(offset_str.trim(), param_name, ctx);
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = parse_arith_offset(len_str.trim(), param_name, ctx);
                let end = if len < 0 {
                    (char_count as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(char_count)
                };
                val.chars().skip(start).take(end - start).collect()
            } else {
                val.chars().skip(start).collect()
            }
        }
        // For other operations, just return the value unchanged
        _ => val.to_string(),
    }
}

pub(super) fn expand_param(expr: &ParamExpr, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    // Handle indirect expansion with operators: ${!name+word}, ${!name-word}, etc.
    if expr.name.starts_with('!')
        && expr.name.len() > 1
        && !matches!(expr.op, ParamOp::None | ParamOp::Indirect)
    {
        let real_name = &expr.name[1..];
        // First resolve the indirect: get the value of real_name, use as variable name
        let target = lookup_var(real_name, ctx);
        // Now apply the operator to the target variable
        let indirect_expr = ParamExpr {
            name: target,
            op: expr.op.clone(),
        };
        return expand_param(&indirect_expr, ctx, cmd_sub);
    }

    // For $@ and $* with operations, apply per-element
    if (expr.name == "@" || expr.name == "*")
        && !matches!(expr.op, ParamOp::None | ParamOp::Length)
        && ctx.positional.len() > 1
    {
        // For Substring: slice the positional params array
        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
            // ${@:0} includes $0, ${@:1} starts at $1
            let params = if offset == 0 {
                ctx.positional
            } else {
                &ctx.positional[1..]
            };
            let count = params.len();
            let start = if offset < 0 {
                (count as i64 + offset).max(0) as usize
            } else if offset == 0 {
                0
            } else {
                ((offset - 1) as usize).min(count)
            };
            let end = if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(count as i64);
                if len < 0 {
                    (count as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(count)
                }
            } else {
                count
            };
            let sliced: Vec<&str> = params[start..end].iter().map(|s| s.as_str()).collect();
            return sliced.join(" ");
        }
        let elements: Vec<String> = ctx.positional[1..]
            .iter()
            .map(|elem| apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name))
            .collect();
        return elements.join(" ");
    }

    // For array[@] or array[*] with operations, apply per-element
    if let Some(bracket) = expr.name.find('[') {
        let base = &expr.name[..bracket];
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        if (idx_str == "@" || idx_str == "*") && !matches!(expr.op, ParamOp::None | ParamOp::Length)
        {
            let resolved = ctx.resolve_nameref(base);
            let elements: Vec<String> = if let Some(arr) = ctx.arrays.get(&resolved) {
                arr.iter().filter_map(|v| v.clone()).collect()
            } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                assoc.values().cloned().collect()
            } else if let Some(val) = ctx.vars.get(&resolved) {
                vec![val.clone()]
            } else {
                vec![]
            };
            let modified: Vec<String> = elements
                .iter()
                .map(|elem| apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name))
                .collect();
            return modified.join(" ");
        }
    }

    let val = lookup_var(&expr.name, ctx);

    // set -u (nounset): error on unset variables, unless operation provides a default
    // Operations that provide defaults (:-,  :=, :+, :?) are OK; trim/replace/etc are not
    // set -u (nounset): check positional params ($1..$9, ${10}...) as unbound
    // when they exceed the number of set positional params
    let is_positional_unbound = if let Ok(n) = expr.name.parse::<usize>() {
        n > 0 && n >= ctx.positional.len()
    } else {
        false
    };

    if val.is_empty()
        && ctx.opt_flags.contains('u')
        && !matches!(
            expr.op,
            ParamOp::Default(..) | ParamOp::Assign(..) | ParamOp::Alt(..) | ParamOp::Error(..)
        )
        && !matches!(
            expr.name.as_str(),
            "?" | "$" | "#" | "@" | "*" | "-" | "0"
        )
        // $! is only exempt from nounset if a background job has been started
        && !(expr.name == "!" && ctx.last_bg_pid != 0)
        && (is_positional_unbound
            || (expr.name.parse::<usize>().is_err()
                && !ctx.vars.contains_key(&expr.name)
                && !ctx.arrays.contains_key(&expr.name)
                && !ctx.assoc_arrays.contains_key(&expr.name)
                && std::env::var(&expr.name).is_err()))
    {
        let sname = ctx
            .vars
            .get("_BASH_SOURCE_FILE")
            .or_else(|| ctx.positional.first())
            .map(|s| s.as_str())
            .unwrap_or("bash");
        let lineno = ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
        eprintln!(
            "{}: line {}: {}: unbound variable",
            sname, lineno, expr.name
        );
        set_arith_error(); // Reuse arith error flag to signal abort
        set_nounset_error();
        return String::new();
    }

    match &expr.op {
        ParamOp::None => val,
        ParamOp::Length => {
            // ${#@} or ${#*} — positional parameter count
            if expr.name == "@" || expr.name == "*" {
                return (ctx.positional.len().saturating_sub(1)).to_string();
            }
            // ${#arr[@]} — array length
            if let Some(bracket) = expr.name.find('[') {
                let base = &expr.name[..bracket];
                let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
                let resolved = ctx.resolve_nameref(base);
                if idx_str == "@" || idx_str == "*" {
                    if let Some(arr) = ctx.arrays.get(&resolved) {
                        return arr.iter().filter(|v| v.is_some()).count().to_string();
                    }
                    if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                        return assoc.len().to_string();
                    }
                }
            }
            val.len().to_string()
        }
        ParamOp::Indirect => {
            // ${!var} — indirect expansion
            let target = lookup_var(&expr.name, ctx);
            if target.is_empty() {
                String::new()
            } else {
                lookup_var(&target, ctx)
            }
        }
        ParamOp::NamePrefix(_ch) => {
            // ${!prefix@} or ${!prefix*} — variable names matching prefix
            let prefix = &expr.name;
            let mut names: Vec<&String> = ctx
                .vars
                .keys()
                .filter(|k| k.starts_with(prefix.as_str()))
                .collect();
            names.sort();
            // TODO: when ch == '*', join with first char of IFS instead of space
            let sep = " ";
            names
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(sep)
        }
        ParamOp::ArrayIndices(_ch) => {
            // ${!arr[@]} or ${!arr[*]} — array indices/keys
            let resolved = ctx.resolve_nameref(&expr.name);
            if let Some(arr) = ctx.arrays.get(&resolved) {
                let indices: Vec<String> = (0..arr.len())
                    .filter(|&i| arr[i].is_some())
                    .map(|i| i.to_string())
                    .collect();
                indices.join(" ")
            } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                let keys: Vec<String> = assoc.keys().cloned().collect();
                keys.join(" ")
            } else {
                // Scalar variable — index 0
                if ctx.vars.contains_key(&resolved) {
                    "0".to_string()
                } else {
                    String::new()
                }
            }
        }
        ParamOp::Default(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = !ctx.is_param_set(&expr.name);
            if unset || empty {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Assign(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = !ctx.is_param_set(&expr.name);
            if unset || empty {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Error(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = !ctx.is_param_set(&expr.name);
            if unset || empty {
                let msg = expand_word_nosplit_ctx(word, ctx, cmd_sub);
                eprintln!(
                    "bash: {}: {}",
                    expr.name,
                    if msg.is_empty() {
                        "parameter null or not set"
                    } else {
                        &msg
                    }
                );
                std::process::exit(1);
            }
            val
        }
        ParamOp::Alt(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let unset = !ctx.is_param_set(&expr.name);
            if unset || empty {
                String::new()
            } else {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            }
        }
        ParamOp::TrimSmallLeft(pattern) | ParamOp::TrimLargeLeft(pattern) => {
            crate::lexer::warn_incomplete_comsub_in_pattern(
                pattern,
                ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0"),
            );
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            let mode = if matches!(&expr.op, ParamOp::TrimSmallLeft(_)) {
                TrimMode::SmallLeft
            } else {
                TrimMode::LargeLeft
            };
            trim_pattern(&val, &pat, mode)
        }
        ParamOp::TrimSmallRight(pattern) => {
            crate::lexer::warn_incomplete_comsub_in_pattern(
                pattern,
                ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0"),
            );
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::SmallRight)
        }
        ParamOp::TrimLargeRight(pattern) => {
            crate::lexer::warn_incomplete_comsub_in_pattern(
                pattern,
                ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0"),
            );
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::LargeRight)
        }
        ParamOp::Replace(pattern, replacement)
        | ParamOp::ReplaceAll(pattern, replacement)
        | ParamOp::ReplacePrefix(pattern, replacement)
        | ParamOp::ReplaceSuffix(pattern, replacement) => {
            let pat = expand_pattern_word(pattern, ctx, cmd_sub);
            let rep = expand_word_nosplit_ctx(replacement, ctx, cmd_sub);
            match &expr.op {
                ParamOp::ReplaceAll(..) => pattern_replace(&val, &pat, &rep, true),
                ParamOp::ReplacePrefix(..) => {
                    // Replace only if pattern matches at start
                    for i in 0..=val.len() {
                        if shell_pattern_match(&val[..i], &pat) {
                            return format!("{}{}", rep, &val[i..]);
                        }
                    }
                    val
                }
                ParamOp::ReplaceSuffix(..) => {
                    // Replace only if pattern matches at end
                    for i in (0..=val.len()).rev() {
                        if shell_pattern_match(&val[i..], &pat) {
                            return format!("{}{}", &val[..i], rep);
                        }
                    }
                    val
                }
                _ => pattern_replace(&val, &pat, &rep, false),
            }
        }
        ParamOp::Substring(offset_str, length_str) => {
            let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx);
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx);
                let end = if len < 0 {
                    (char_count as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(char_count)
                };
                val.chars().skip(start).take(end - start).collect()
            } else {
                val.chars().skip(start).collect()
            }
        }
        ParamOp::UpperFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        c.to_uppercase().to_string() + chars.as_str()
                    } else {
                        val.clone()
                    }
                }
            }
        }
        ParamOp::UpperAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            if pat_str.is_empty() {
                val.to_uppercase()
            } else {
                val.chars()
                    .map(|c| {
                        if char_matches_pattern(c, &pat_str) {
                            c.to_uppercase().collect::<String>()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
        }
        ParamOp::LowerFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        c.to_lowercase().to_string() + chars.as_str()
                    } else {
                        val.clone()
                    }
                }
            }
        }
        ParamOp::LowerAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            if pat_str.is_empty() {
                val.to_lowercase()
            } else {
                val.chars()
                    .map(|c| {
                        if char_matches_pattern(c, &pat_str) {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
        }
        ParamOp::ToggleFirst(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    if pat_str.is_empty() || char_matches_pattern(c, &pat_str) {
                        let toggled = if c.is_uppercase() {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_uppercase().collect::<String>()
                        };
                        toggled + chars.as_str()
                    } else {
                        val.clone()
                    }
                }
            }
        }
        ParamOp::ToggleAll(pat) => {
            let pat_str = expand_word_nosplit_ctx(pat, ctx, cmd_sub);
            if pat_str.is_empty() {
                val.chars()
                    .map(|c| {
                        if c.is_uppercase() {
                            c.to_lowercase().collect::<String>()
                        } else {
                            c.to_uppercase().collect::<String>()
                        }
                    })
                    .collect()
            } else {
                val.chars()
                    .map(|c| {
                        if char_matches_pattern(c, &pat_str) {
                            if c.is_uppercase() {
                                c.to_lowercase().collect::<String>()
                            } else {
                                c.to_uppercase().collect::<String>()
                            }
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
        }
        #[allow(clippy::match_single_binding)]
        ParamOp::Transform(ch) => match ch {
            'E' => {
                // Expand backslash escapes like $'...'
                val.replace("\\n", "\n")
                    .replace("\\t", "\t")
                    .replace("\\r", "\r")
                    .replace("\\\\", "\\")
                    .replace("\\a", "\x07")
                    .replace("\\b", "\x08")
            }
            'Q' => {
                let has_control = val.bytes().any(|b| b < 0x20 || b == 0x7f);
                if val.is_empty() {
                    "''".to_string()
                } else if has_control {
                    // Use $'...' quoting for strings with control chars
                    let mut s = String::from("$'");
                    for ch in val.chars() {
                        match ch {
                            '\x07' => s.push_str("\\a"),
                            '\x08' => s.push_str("\\b"),
                            '\x1b' => s.push_str("\\E"),
                            '\x0c' => s.push_str("\\f"),
                            '\n' => s.push_str("\\n"),
                            '\r' => s.push_str("\\r"),
                            '\t' => s.push_str("\\t"),
                            '\x0b' => s.push_str("\\v"),
                            '\'' => s.push_str("\\'"),
                            '\\' => s.push_str("\\\\"),
                            c if (c as u32) < 0x20 || c == '\x7f' => {
                                s.push_str(&format!("\\x{:02x}", c as u32));
                            }
                            c => s.push(c),
                        }
                    }
                    s.push('\'');
                    s
                } else {
                    format!("'{}'", val.replace('\'', "'\\''"))
                }
            }
            'U' => val.to_uppercase(),
            'L' => val.to_lowercase(),
            'u' => {
                let mut chars = val.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            }
            'a' => {
                // Variable attributes — look up pre-computed attrs from interpreter
                ctx.vars
                    .get(&format!("__ATTRS__{}", expr.name))
                    .cloned()
                    .unwrap_or_default()
            }
            'A' => {
                // Assignment form: declare -FLAGS name='value'
                let attrs = ctx
                    .vars
                    .get(&format!("__ATTRS__{}", expr.name))
                    .cloned()
                    .unwrap_or_default();
                let flags = if attrs.is_empty() {
                    "--".to_string()
                } else {
                    format!("-{}", attrs)
                };
                format!(
                    "declare {} {}='{}'",
                    flags,
                    expr.name,
                    val.replace('\'', "'\\''")
                )
            }
            _ => val, // @P, @K — return as-is for now
        },
    }
}
