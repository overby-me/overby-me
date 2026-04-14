use super::transform_helpers::{expand_backslash_escapes, shell_quote};
use super::*;
use crate::builtins::string_to_raw_bytes;
use crate::interpreter::array_effective_len;

/// Process `&` in a replacement string for pattern substitution.
/// When `patsub_replacement` shopt is enabled, unescaped `&` in the
/// replacement is substituted with the matched text, and `\&` becomes
/// a literal `&`.  When the option is disabled, the replacement is
/// returned as-is.
fn process_replacement_amp(replacement: &str, matched: &str) -> String {
    if !super::get_patsub_replacement() {
        // Still need to strip \x00 markers even when patsub is off.
        // \x00X pairs become literal X (covers quoted & and quoted \).
        if replacement.contains('\x00') {
            let mut r = String::with_capacity(replacement.len());
            let cs: Vec<char> = replacement.chars().collect();
            let mut j = 0;
            while j < cs.len() {
                if cs[j] == '\x00' && j + 1 < cs.len() {
                    r.push(cs[j + 1]);
                    j += 2;
                } else {
                    r.push(cs[j]);
                    j += 1;
                }
            }
            return r;
        }
        return replacement.to_string();
    }
    let chars: Vec<char> = replacement.chars().collect();
    let mut result = String::with_capacity(replacement.len() + matched.len());
    let mut i = 0;
    let mut had_special = false;
    // Quick check: if no `&` or `\&` or `\x00` at all, return as-is
    if !replacement.contains('&') && !replacement.contains('\x00') {
        return replacement.to_string();
    }
    while i < chars.len() {
        if chars[i] == '\x00' && i + 1 < chars.len() {
            // \x00X → literal X (was quoted in the original word).
            // This covers \x00& (quoted &) and \x00\ (quoted \) so that
            // a quoted backslash doesn't accidentally escape a following &.
            result.push(chars[i + 1]);
            i += 2;
            had_special = true;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            // \\ → literal \ (escaped backslash)
            result.push('\\');
            i += 2;
            had_special = true;
        } else if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '&' {
            // \& → literal &
            result.push('&');
            i += 2;
            had_special = true;
        } else if chars[i] == '&' {
            // & → matched text
            result.push_str(matched);
            i += 1;
            had_special = true;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    if had_special {
        result
    } else {
        replacement.to_string()
    }
}

/// Count the number of multibyte characters in a bash-style string.
/// Bash stores raw bytes as chars with their byte value (Latin-1 mapping).
/// In a UTF-8 locale, this interprets the raw byte sequence as UTF-8 and
/// counts characters. In a non-UTF-8 locale, it counts bytes.
fn mbstrlen(s: &str) -> usize {
    // Check if we're in a UTF-8 locale
    let is_utf8 = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .map(|v| {
            let v = v.to_lowercase();
            v.contains("utf-8") || v.contains("utf8")
        })
        .unwrap_or(false);

    if !is_utf8 {
        return s.chars().count();
    }

    // Convert to raw bytes (chars in U+0080..U+00FF become single bytes)
    let raw = string_to_raw_bytes(s);
    // Count UTF-8 characters in the raw byte sequence
    String::from_utf8_lossy(&raw).chars().count()
}

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
    if matches!(&expr.op, ParamOp::ArrayIndices('@', _)) {
        return ctx.arrays.contains_key(&expr.name) || ctx.assoc_arrays.contains_key(&expr.name);
    }
    // ${!prefix@} — variable names matching prefix should split into separate
    // fields (like "$@"), while ${!prefix*} joins with IFS (like "$*").
    if matches!(&expr.op, ParamOp::NamePrefix('@')) {
        return true;
    }
    if let Some(bracket) = expr.name.find('[') {
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        if idx_str == "@" {
            let base = &expr.name[..bracket];
            let resolved = ctx.resolve_nameref(base);
            // Array[@] with any operation should expand per-element,
            // EXCEPT Transform('A') and Transform('a') which produce
            // whole-variable declaration/attribute output (not per-element).
            if !matches!(
                &expr.op,
                ParamOp::Length
                    | ParamOp::Transform('A')
                    | ParamOp::Transform('a')
                    | ParamOp::Transform('K')
            ) {
                return ctx.arrays.contains_key(&resolved)
                    || ctx.assoc_arrays.contains_key(&resolved);
            }
        }
    }
    false
}

/// Get array elements for an array[@] expansion.
pub(super) fn get_array_elements(expr: &ParamExpr, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> Vec<String> {
    // Handle ${@:offset:length} — slice of positional params
    if expr.name == "@" || expr.name == "*" {
        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
            let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);
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
                let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
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
    // ${!prefix@} — return matching variable names as separate elements
    if let ParamOp::NamePrefix('@') = &expr.op {
        let prefix = &expr.name;
        let mut name_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for k in ctx.vars.keys() {
            if k.starts_with(prefix.as_str()) {
                name_set.insert(k.clone());
            }
        }
        for k in ctx.arrays.keys() {
            if k.starts_with(prefix.as_str()) {
                name_set.insert(k.clone());
            }
        }
        for k in ctx.assoc_arrays.keys() {
            if k.starts_with(prefix.as_str()) {
                name_set.insert(k.clone());
            }
        }
        return name_set.into_iter().collect();
    }
    // ${!arr[@]} — return indices/keys as elements
    if let ParamOp::ArrayIndices(_, _) = &expr.op {
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

        // For Substring on indexed arrays, use index-based offset matching:
        // ${arr[@]:offset:length} selects elements starting at array index >= offset,
        // then takes `length` existing (set) elements from that point.
        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
            let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);
            if let Some(arr) = ctx.arrays.get(&resolved) {
                // Collect (index, value) pairs for set elements
                let set_elements: Vec<(usize, &str)> = arr
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.as_ref().map(|s| (i, s.as_str())))
                    .collect();
                let count = set_elements.len();
                // For negative offsets, compute from the array's total length
                // (highest_index + 1), not from the count of set elements.
                // e.g., arr=([1]=a [5]=b [7]=c) has length 8, so -2 → index 6.
                let effective_offset = if offset < 0 {
                    let arr_len = array_effective_len(arr) as i64; // highest_index + 1
                    (arr_len + offset).max(0)
                } else {
                    offset
                };
                let start = set_elements
                    .iter()
                    .position(|(idx, _)| *idx >= effective_offset as usize)
                    .unwrap_or(count);
                let end = if let Some(len_str) = length_str {
                    let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                    if len < 0 {
                        let target = (count as i64 + len).max(0) as usize;
                        target.max(start)
                    } else {
                        (start + len as usize).min(count)
                    }
                } else {
                    count
                };
                return set_elements[start..end]
                    .iter()
                    .map(|(_, v)| v.to_string())
                    .collect();
            }
            if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                let values: Vec<String> = assoc.values().cloned().collect();
                let count = values.len();
                let start = if offset < 0 {
                    (count as i64 + offset).max(0) as usize
                } else {
                    (offset as usize).min(count)
                };
                let end = if let Some(len_str) = length_str {
                    let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                    if len < 0 {
                        let target = (count as i64 + len).max(0) as usize;
                        target.max(start)
                    } else {
                        (start + len as usize).min(count)
                    }
                } else {
                    count
                };
                return values[start..end].to_vec();
            }
            // Scalar treated as single-element array — apply character-level
            // substring (same as ${var:offset:length}).
            if let Some(val) = ctx.vars.get(&resolved) {
                let chars: Vec<char> = val.chars().collect();
                let count = chars.len() as i64;
                let start = if offset < 0 {
                    (count + offset).max(0) as usize
                } else {
                    (offset as usize).min(chars.len())
                };
                let end = if let Some(len_str) = length_str {
                    let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                    if len < 0 {
                        let target = (count + len).max(0) as usize;
                        target.max(start)
                    } else {
                        (start + len as usize).min(chars.len())
                    }
                } else {
                    chars.len()
                };
                let substr: String = chars[start..end].iter().collect();
                if substr.is_empty() {
                    return vec![];
                }
                return vec![substr];
            }
            return vec![];
        }

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
                        // For [*], join with IFS[0] (space if unset, empty if IFS="")
                        // For [@], join with space (field splitting handled by callers)
                        let sep = if idx_str == "*" {
                            let ifs = ctx.vars.get("IFS");
                            match ifs {
                                None => " ".to_string(),
                                Some(s) if s.is_empty() => String::new(),
                                Some(s) => s.chars().next().unwrap_or(' ').to_string(),
                            }
                        } else {
                            " ".to_string()
                        };
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            arr.iter()
                                .filter_map(|v| v.as_ref())
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(&sep)
                        } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                            assoc.values().cloned().collect::<Vec<_>>().join(&sep)
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
                            // Expand $var and ${var} references in the subscript key
                            let expanded_key = if stripped_key.contains('$') {
                                let mut expanded = stripped_key.to_string();
                                while let Some(pos) = expanded.find('$') {
                                    let rest = &expanded[pos + 1..];
                                    if rest.starts_with('{') {
                                        // Find matching '}' — must handle nested brackets
                                        // so that ${a[i]} finds the right closing brace.
                                        let mut brace_depth = 1;
                                        let mut j = 1; // skip opening '{'
                                        while j < rest.len() && brace_depth > 0 {
                                            match rest.as_bytes()[j] {
                                                b'{' => brace_depth += 1,
                                                b'}' => brace_depth -= 1,
                                                _ => {}
                                            }
                                            if brace_depth > 0 {
                                                j += 1;
                                            }
                                        }
                                        if brace_depth == 0 {
                                            let var_name = &rest[1..j];
                                            // Recursively look up — handles array subscripts
                                            // like a[i] inside ${a[i]}
                                            let var_val = lookup_var(var_name, ctx);
                                            expanded = format!(
                                                "{}{}{}",
                                                &expanded[..pos],
                                                var_val,
                                                &rest[j + 1..]
                                            );
                                        } else {
                                            break;
                                        }
                                    } else {
                                        let var_end = rest
                                            .find(|c: char| !c.is_alphanumeric() && c != '_')
                                            .unwrap_or(rest.len());
                                        if var_end == 0 {
                                            // $ followed by non-identifier char (e.g. $(, $!, etc.)
                                            // — leave the $ as-is and skip past it
                                            break;
                                        }
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
                                stripped_key.to_string()
                            };
                            // Strip backslash escapes from the key (e.g. \] → ])
                            // In bash, backslash in an unquoted subscript like ${m[\]]}
                            // escapes the next character, so \] becomes literal ].
                            let expanded_key = if expanded_key.contains('\\') {
                                let mut result = String::new();
                                let mut chars_iter = expanded_key.chars();
                                while let Some(c) = chars_iter.next() {
                                    if c == '\\' {
                                        if let Some(next) = chars_iter.next() {
                                            result.push(next);
                                        } else {
                                            result.push(c);
                                        }
                                    } else {
                                        result.push(c);
                                    }
                                }
                                result
                            } else {
                                expanded_key
                            };
                            // Empty key after expansion is invalid for associative arrays
                            if expanded_key.is_empty() {
                                let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                                    let p = p.borrow();
                                    if p.is_empty() {
                                        "bash".to_string()
                                    } else {
                                        p.clone()
                                    }
                                });
                                eprintln!("{}: [{}]: bad array subscript", prefix, idx_str);
                                crate::expand::set_arith_error();
                                return String::new();
                            }
                            return assoc
                                .get(expanded_key.as_str())
                                .cloned()
                                .unwrap_or_default();
                        }
                        // Check for brace expansion pattern in subscript
                        // (e.g. `${arr[{2..6}]}` → expand braces, look up each index)
                        if idx_str.contains('{')
                            && (idx_str.contains("..") || idx_str.contains(','))
                            && ctx.arrays.contains_key(&resolved)
                        {
                            let expanded = crate::expand::brace_expand(idx_str);
                            if expanded.len() > 1 {
                                let arr = ctx.arrays.get(&resolved).unwrap();
                                let results: Vec<String> = expanded
                                    .iter()
                                    .filter_map(|s| {
                                        let idx: usize = s.trim().parse().ok()?;
                                        arr.get(idx).and_then(|v| v.clone())
                                    })
                                    .collect();
                                return results.join(" ");
                            }
                        }
                        // Numeric index for indexed arrays — use arithmetic evaluation
                        // If subscript contains $(...), expand it first via cmd_sub runner
                        let idx_str = if idx_str.contains("$(") {
                            std::borrow::Cow::Owned(crate::expand::expand_comsubs_in_arith_expr(
                                idx_str,
                                &mut |cmd| crate::expand::run_cmd_sub(cmd).unwrap_or_default(),
                            ))
                        } else {
                            std::borrow::Cow::Borrowed(idx_str)
                        };
                        let raw_idx: i64 = if idx_str.trim().is_empty() {
                            0
                        } else if let Ok(v) = idx_str.trim().parse::<i64>() {
                            v
                        } else {
                            crate::expand::arithmetic::eval_arith_full_with_assoc(
                                &idx_str,
                                ctx.vars,
                                ctx.arrays,
                                ctx.assoc_arrays,
                                ctx.namerefs,
                                ctx.positional,
                                ctx.last_status,
                                ctx.opt_flags,
                            )
                        };
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            let idx = if raw_idx < 0 {
                                let len = array_effective_len(arr) as i64;
                                let computed = len + raw_idx;
                                if computed < 0 {
                                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                                        let p = p.borrow();
                                        if p.is_empty() {
                                            "bash".to_string()
                                        } else {
                                            p.clone()
                                        }
                                    });
                                    eprintln!("{}: {}: bad array subscript", prefix, resolved);
                                    // Signal that a bad subscript was reported
                                    // so expand_part skips the duplicate
                                    // lookup_var call, but do NOT set
                                    // arith_error — bash still runs the command
                                    // with an empty expansion.
                                    crate::expand::set_bad_subscript();
                                    return String::new();
                                }
                                computed as usize
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

            // Resolve namerefs (with warnings for circular references)
            let resolved = ctx.resolve_nameref_warn_expand(name);

            // If the resolved nameref target contains '[', re-dispatch as a
            // subscript lookup.  This handles `declare -n nref='assoc[@]'`
            // where `$nref` should expand to all values of the array.
            if resolved.find('[').is_some() {
                // If the subscript contains $(...) command substitution,
                // expand it via the registered cmd_sub runner before lookup.
                let resolved = if resolved.contains("$(") {
                    if let Some(bracket_pos) = resolved.find('[') {
                        let base = &resolved[..bracket_pos];
                        let subscript_with_brackets = &resolved[bracket_pos..];
                        // Extract subscript content between [ and ]
                        if let (Some(open), Some(close)) = (
                            subscript_with_brackets.find('['),
                            subscript_with_brackets.rfind(']'),
                        ) {
                            let subscript = &subscript_with_brackets[open + 1..close];
                            let expanded_sub = crate::expand::expand_comsubs_in_arith_expr(
                                subscript,
                                &mut |cmd| crate::expand::run_cmd_sub(cmd).unwrap_or_default(),
                            );
                            format!("{}[{}]", base, expanded_sub)
                        } else {
                            resolved
                        }
                    } else {
                        resolved
                    }
                } else {
                    resolved
                };
                // Re-enter lookup_var with the resolved subscripted name
                return lookup_var(&resolved, ctx);
            }

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

/// Expand backtick command substitutions in a string using cmd_sub.
/// E.g. "`echo foo`" → "foo"
fn expand_backticks_in_str(s: &str, cmd_sub: CmdSubFn) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            i += 1; // skip opening backtick
            let mut cmd = String::new();
            while i < chars.len() && chars[i] != '`' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if matches!(next, '$' | '`' | '\\') {
                        cmd.push(next);
                        i += 2;
                        continue;
                    }
                }
                cmd.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1; // skip closing backtick
            }
            let output = cmd_sub(&cmd);
            // Trim trailing newline like shell does
            result.push_str(output.trim_end_matches('\n'));
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Parse an arithmetic expression for substring offset/length.
/// If the string is a simple integer, parse it directly. If it's a variable name,
/// resolve it. Otherwise, report an arithmetic error.
pub(super) fn parse_arith_offset(
    s: &str,
    param_name: &str,
    ctx: &ExpCtx,
    cmd_sub: CmdSubFn,
) -> i64 {
    if s.is_empty() {
        return 0;
    }
    // Pre-expand ${...} parameter expressions (e.g. ${#arr[@]}) before
    // arithmetic evaluation.  The arithmetic resolver (resolve_arith_vars)
    // uses raw_val.len() for ${#name} which gives byte-length of the
    // joined string, not the element count.  By expanding through the
    // normal word-expansion pipeline first we get the correct value.
    let preexpanded;
    let s = if s.contains("${") || s.contains("$(") {
        let word = crate::lexer::parse_word_string(s);
        preexpanded = expand_word_nosplit_ctx(&word, ctx, cmd_sub);
        &preexpanded
    } else {
        s
    };
    // Expand backtick command substitutions before arithmetic evaluation
    let expanded;
    let s = if s.contains('`') {
        expanded = expand_backticks_in_str(s, cmd_sub);
        &expanded
    } else {
        s
    };
    // Handle $((...)) arithmetic expansion: strip outer $(( and )) and evaluate inner
    let trimmed = s.trim();
    if trimmed.starts_with("$((") && trimmed.ends_with("))") {
        let inner = &trimmed[3..trimmed.len() - 2];
        return crate::expand::arithmetic::eval_arith_full_with_assoc(
            inner,
            ctx.vars,
            ctx.arrays,
            ctx.assoc_arrays,
            ctx.namerefs,
            ctx.positional,
            ctx.last_status,
            ctx.opt_flags,
        );
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
    crate::expand::arithmetic::eval_arith_full_with_assoc(
        s,
        ctx.vars,
        ctx.arrays,
        ctx.assoc_arrays,
        ctx.namerefs,
        ctx.positional,
        ctx.last_status,
        ctx.opt_flags,
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
            let rep = super::expand_replacement_word(replacement, ctx, cmd_sub);
            match op {
                ParamOp::ReplaceAll(..) => pattern_replace(val, &pat, &rep, true),
                ParamOp::ReplacePrefix(..) => {
                    // Iterate longest-first so `#*` matches the entire string
                    for i in (0..=val.len()).rev() {
                        if !val.is_char_boundary(i) {
                            continue;
                        }
                        if shell_pattern_match(&val[..i], &pat) {
                            let effective_rep = process_replacement_amp(&rep, &val[..i]);
                            return format!("{}{}", effective_rep, &val[i..]);
                        }
                    }
                    val.to_string()
                }
                ParamOp::ReplaceSuffix(..) => {
                    // Iterate shortest-position-first so `%*` matches the entire string
                    // (longest suffix match wins)
                    for i in 0..=val.len() {
                        if !val.is_char_boundary(i) {
                            continue;
                        }
                        if shell_pattern_match(&val[i..], &pat) {
                            let effective_rep = process_replacement_amp(&rep, &val[i..]);
                            return format!("{}{}", &val[..i], effective_rep);
                        }
                    }
                    val.to_string()
                }
                _ => pattern_replace(val, &pat, &rep, false),
            }
        }
        ParamOp::Substring(offset_str, length_str) => {
            let offset: i64 = parse_arith_offset(offset_str.trim(), param_name, ctx, cmd_sub);
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = parse_arith_offset(len_str.trim(), param_name, ctx, cmd_sub);
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
        ParamOp::Transform(ch) => match ch {
            'Q' | 'K' | 'k' => shell_quote(val),
            'E' => expand_backslash_escapes(val),
            'U' => val.to_uppercase(),
            'L' => val.to_lowercase(),
            'u' => {
                let mut chars = val.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            }
            // 'a' and 'A' are per-variable, not per-element; return as-is
            _ => val.to_string(),
        },
        // For other operations (NamePrefix, ArrayIndices, Default, Assign, Error, etc.),
        // just return the value unchanged
        _ => val.to_string(),
    }
}

fn is_valid_var_ref(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Special parameters
    if matches!(name, "@" | "*" | "#" | "?" | "-" | "$" | "!" | "0" | "_") {
        return true;
    }
    // Positional params
    if name.parse::<usize>().is_ok() {
        return true;
    }
    // Check for array subscript: name[idx]
    let base = if let Some(bracket) = name.find('[') {
        &name[..bracket]
    } else {
        name
    };
    // Valid variable name: starts with letter/underscore, rest is alnum/underscore
    let mut chars = base.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Decode prompt escape sequences for `${var@P}`.
///
/// Handles `\v`, `\V`, `\$`, `\W`, `\w`, `\h`, `\H`, `\u`, `\s`, `\n`, `\r`,
/// `\a`, `\e`, `\\`, `\[`, `\]`, `\j`, `\!`, `\#`, `\l`, `\d`, `\t`, `\T`,
/// `\@`, `\A`, `\D{fmt}`, `\0NNN` octal, and POSIX `!` history expansion.
///
/// After decoding, `promptvars` expansion (variable/command substitution) is
/// performed by the caller via `cmd_sub`.
fn decode_prompt_string(s: &str, ctx: &ExpCtx) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let mut i = 0;

    // Determine values we might need
    let hostname = {
        let h = ctx
            .vars
            .get("HOSTNAME")
            .cloned()
            .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_default());
        if h.is_empty() {
            // Try to read from system
            #[cfg(unix)]
            {
                let mut buf = [0u8; 256];
                if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) }
                    == 0
                {
                    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..end]).to_string()
                } else {
                    String::new()
                }
            }
            #[cfg(not(unix))]
            String::new()
        } else {
            h
        }
    };

    let version = "5.3"; // dist_version equivalent
    let patch_level = 0; // patch_level equivalent

    let posixly_correct = ctx.vars.contains_key("__POSIX__");

    while i < chars.len() {
        let c = chars[i];

        // POSIX mode: `!` means history number, `!!` means literal `!`
        if posixly_correct && c == '!' {
            if i + 1 < chars.len() && chars[i + 1] == '!' {
                result.push('!');
                i += 2;
            } else {
                // History number — we always return "1"
                result.push('1');
                i += 1;
            }
            continue;
        }

        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            match next {
                '0' | '1' | '2' | '3' | '4' | '5' | '6' | '7' => {
                    // Octal escape: \0NNN (up to 3 octal digits)
                    let mut val = 0u32;
                    let mut count = 0;
                    let mut j = i + 1;
                    while j < chars.len() && count < 3 && chars[j] >= '0' && chars[j] <= '7' {
                        val = val * 8 + (chars[j] as u32 - '0' as u32);
                        j += 1;
                        count += 1;
                    }
                    if val > 0 && val <= 0x7f {
                        result.push(val as u8 as char);
                    } else if val == 0 {
                        // \0 — skip
                    } else {
                        result.push(char::from_u32(val).unwrap_or('?'));
                    }
                    i = j;
                    continue;
                }
                'd' => {
                    // Date: "Weekday Month Day"
                    use std::time::SystemTime;
                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    // Simple date formatting (avoid chrono dependency)
                    // Just output a placeholder — tests don't usually compare exact dates
                    let _ = now;
                    // Use libc localtime + strftime on unix
                    #[cfg(unix)]
                    {
                        let time = now as libc::time_t;
                        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                        unsafe { libc::localtime_r(&time, &mut tm) };
                        let mut buf = [0u8; 64];
                        let fmt = std::ffi::CString::new("%a %b %d")
                            .unwrap_or_else(|_| std::ffi::CString::default());
                        let len = unsafe {
                            libc::strftime(
                                buf.as_mut_ptr() as *mut libc::c_char,
                                buf.len(),
                                fmt.as_ptr(),
                                &tm,
                            )
                        };
                        if len > 0 {
                            result.push_str(&String::from_utf8_lossy(&buf[..len]));
                        }
                    }
                    i += 2;
                    continue;
                }
                't' => {
                    // Time: HH:MM:SS (24-hour)
                    push_strftime(&mut result, "%H:%M:%S");
                    i += 2;
                    continue;
                }
                'T' => {
                    // Time: HH:MM:SS (12-hour)
                    push_strftime(&mut result, "%I:%M:%S");
                    i += 2;
                    continue;
                }
                '@' => {
                    // Time: HH:MM AM/PM
                    push_strftime(&mut result, "%I:%M %p");
                    i += 2;
                    continue;
                }
                'A' => {
                    // Time: HH:MM (24-hour)
                    push_strftime(&mut result, "%H:%M");
                    i += 2;
                    continue;
                }
                'D' => {
                    // \D{format} — strftime format
                    if i + 2 < chars.len() && chars[i + 2] == '{' {
                        let start = i + 3;
                        let mut end = start;
                        while end < chars.len() && chars[end] != '}' {
                            end += 1;
                        }
                        let fmt: String = chars[start..end].iter().collect();
                        let fmt = if fmt.is_empty() {
                            "%X".to_string() // locale-specific time
                        } else {
                            fmt
                        };
                        push_strftime(&mut result, &fmt);
                        i = if end < chars.len() { end + 1 } else { end };
                        continue;
                    }
                    // Not followed by {, treat as literal
                    result.push('\\');
                    result.push('D');
                    i += 2;
                    continue;
                }
                'n' => {
                    // Newline (in non-editing mode, just \n)
                    result.push('\n');
                    i += 2;
                    continue;
                }
                'r' => {
                    result.push('\r');
                    i += 2;
                    continue;
                }
                'a' => {
                    result.push('\x07'); // bell
                    i += 2;
                    continue;
                }
                'e' => {
                    result.push('\x1b'); // escape
                    i += 2;
                    continue;
                }
                '\\' => {
                    result.push('\\');
                    i += 2;
                    continue;
                }
                's' => {
                    // Shell name — bash uses the basename of argv[0] (the shell
                    // binary itself), NOT $0 (which is the script name).  Since
                    // we are a bash-compatible shell, always use "bash".
                    result.push_str("bash");
                    i += 2;
                    continue;
                }
                'v' => {
                    result.push_str(version);
                    i += 2;
                    continue;
                }
                'V' => {
                    result.push_str(&format!("{}.{}", version, patch_level));
                    i += 2;
                    continue;
                }
                'w' => {
                    // Working directory (with ~ substitution for HOME)
                    let pwd = ctx.vars.get("PWD").cloned().unwrap_or_else(|| {
                        std::env::var("PWD").unwrap_or_else(|_| {
                            std::env::current_dir()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| ".".to_string())
                        })
                    });
                    let home = ctx
                        .vars
                        .get("HOME")
                        .cloned()
                        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());
                    if !home.is_empty() && pwd.starts_with(&home) {
                        let rest = &pwd[home.len()..];
                        if rest.is_empty() || rest.starts_with('/') {
                            result.push('~');
                            result.push_str(rest);
                        } else {
                            result.push_str(&pwd);
                        }
                    } else {
                        result.push_str(&pwd);
                    }
                    i += 2;
                    continue;
                }
                'W' => {
                    // Basename of working directory (or ~ if HOME)
                    let pwd = ctx.vars.get("PWD").cloned().unwrap_or_else(|| {
                        std::env::var("PWD").unwrap_or_else(|_| {
                            std::env::current_dir()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_else(|_| ".".to_string())
                        })
                    });
                    let home = ctx
                        .vars
                        .get("HOME")
                        .cloned()
                        .unwrap_or_else(|| std::env::var("HOME").unwrap_or_default());
                    if !home.is_empty() && pwd == home {
                        result.push('~');
                    } else if pwd == "/" {
                        result.push('/');
                    } else {
                        let base = pwd.rsplit('/').next().unwrap_or(&pwd);
                        result.push_str(base);
                    }
                    i += 2;
                    continue;
                }
                'u' => {
                    // Username
                    let user = ctx
                        .vars
                        .get("USER")
                        .cloned()
                        .unwrap_or_else(|| std::env::var("USER").unwrap_or_default());
                    result.push_str(&user);
                    i += 2;
                    continue;
                }
                'h' => {
                    // Hostname (up to first .)
                    let h = if let Some(dot) = hostname.find('.') {
                        &hostname[..dot]
                    } else {
                        &hostname
                    };
                    result.push_str(h);
                    i += 2;
                    continue;
                }
                'H' => {
                    // Full hostname
                    result.push_str(&hostname);
                    i += 2;
                    continue;
                }
                'j' => {
                    // Number of jobs — we don't track jobs well, return "0"
                    result.push('0');
                    i += 2;
                    continue;
                }
                'l' => {
                    // Terminal device basename
                    #[cfg(unix)]
                    {
                        let tty = unsafe { libc::ttyname(0) };
                        if !tty.is_null() {
                            let s = unsafe { std::ffi::CStr::from_ptr(tty) }.to_string_lossy();
                            let base = s.rsplit('/').next().unwrap_or(&s);
                            result.push_str(base);
                        } else {
                            result.push_str("tty");
                        }
                    }
                    #[cfg(not(unix))]
                    result.push_str("tty");
                    i += 2;
                    continue;
                }
                '!' => {
                    // History number — return "1" (no history tracking)
                    result.push('1');
                    i += 2;
                    continue;
                }
                '#' => {
                    // Command number — return "1"
                    result.push('1');
                    i += 2;
                    continue;
                }
                '$' => {
                    // $ or # depending on UID
                    #[cfg(unix)]
                    {
                        let euid = unsafe { libc::geteuid() };
                        result.push(if euid == 0 { '#' } else { '$' });
                    }
                    #[cfg(not(unix))]
                    result.push('$');
                    i += 2;
                    continue;
                }
                '[' | ']' => {
                    // Readline non-printing markers.
                    // When line editing is enabled (set -o emacs / set -o vi),
                    // \[ produces RL_PROMPT_START_IGNORE (0x01) and
                    // \] produces RL_PROMPT_END_IGNORE (0x02).
                    // When line editing is off (default in scripts), these are
                    // silently skipped (no output), matching bash behavior.
                    let line_editing = ctx.vars.contains_key("__LINE_EDITING__");
                    if line_editing {
                        let ch = if next == '[' { '\x01' } else { '\x02' };
                        result.push(ch);
                    }
                    // In both cases, consume the escape sequence
                    i += 2;
                    continue;
                }
                _ => {
                    // Unknown escape — keep as \X
                    result.push('\\');
                    result.push(next);
                    i += 2;
                    continue;
                }
            }
        } else {
            result.push(c);
            i += 1;
        }
    }

    result
}

/// Helper: push strftime-formatted current time into a string.
#[cfg(unix)]
fn push_strftime(out: &mut String, fmt: &str) {
    use std::time::SystemTime;
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let time = now as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&time, &mut tm) };
    let mut buf = [0u8; 128];
    if let Ok(fmt_c) = std::ffi::CString::new(fmt) {
        let len = unsafe {
            libc::strftime(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                fmt_c.as_ptr(),
                &tm,
            )
        };
        if len > 0 {
            out.push_str(&String::from_utf8_lossy(&buf[..len]));
        }
    }
}

#[cfg(not(unix))]
fn push_strftime(out: &mut String, _fmt: &str) {
    out.push_str("??");
}

pub(super) fn expand_param(expr: &ParamExpr, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    // Pre-expand $(...) command substitutions inside associative array subscripts.
    // Pre-expand $(...) command substitutions inside associative array subscripts.
    // lookup_var doesn't have access to cmd_sub, so we expand comsubs in the
    // subscript portion of the name here before any lookups.  E.g.
    // ${A[$(echo Darwin)]} → expand $(echo Darwin) to "Darwin" first.
    // BUT: do NOT expand $(...) that is inside single quotes — e.g.
    // ${A['$(echo Darwin)']} should look up the literal key "$(echo Darwin)".
    let comsub_expanded: Option<ParamExpr>;
    let expr = if expr.name.contains("$(") || expr.name.contains('`') {
        if let Some(bracket) = expr.name.find('[') {
            let base = &expr.name[..bracket];
            let subscript = &expr.name[bracket + 1..];
            // Check if the $( or backtick is outside single quotes in the subscript.
            // If ALL $( occurrences are inside '...', skip expansion.
            let has_unquoted_comsub = {
                let mut in_sq = false;
                let mut in_dq = false;
                let chars: Vec<char> = subscript.chars().collect();
                let mut found = false;
                let mut j = 0;
                while j < chars.len() {
                    if !in_sq && !in_dq && chars[j] == '\'' {
                        in_sq = true;
                        j += 1;
                    } else if in_sq && chars[j] == '\'' {
                        in_sq = false;
                        j += 1;
                    } else if !in_sq && !in_dq && chars[j] == '"' {
                        in_dq = true;
                        j += 1;
                    } else if in_dq && chars[j] == '"' {
                        in_dq = false;
                        j += 1;
                    } else if !in_sq
                        && ((chars[j] == '$' && j + 1 < chars.len() && chars[j + 1] == '(')
                            || chars[j] == '`')
                    {
                        found = true;
                        break;
                    } else {
                        j += 1;
                    }
                }
                found
            };
            if has_unquoted_comsub && !super::get_array_expand_once() {
                let expanded_sub = super::expand_comsubs_in_arith_expr(subscript, cmd_sub);
                let expanded_name = format!("{}[{}", base, expanded_sub);
                comsub_expanded = Some(ParamExpr {
                    name: expanded_name,
                    op: expr.op.clone(),
                });
                comsub_expanded.as_ref().unwrap()
            } else {
                expr
            }
        } else {
            expr
        }
    } else {
        expr
    };

    // Handle indirect expansion with operators: ${!name+word}, ${!name-word}, etc.
    if expr.name.starts_with('!')
        && expr.name.len() > 1
        && !matches!(expr.op, ParamOp::None | ParamOp::Indirect)
    {
        let real_name = &expr.name[1..];

        // Check if the indirect variable is completely unset.
        // In bash, ${!name-word}, ${!name+word}, ${!name:-word}, etc. all
        // produce "invalid indirect expansion" when `name` itself is unset
        // (not found in vars, arrays, assoc_arrays, positional params,
        // special variables, or process env).
        //
        // When real_name contains a subscript (e.g. "varname[@]"),
        // check the base name before the bracket.
        let base_for_check = if let Some(bracket) = real_name.find('[') {
            &real_name[..bracket]
        } else {
            real_name
        };
        let var_exists = ctx.vars.contains_key(base_for_check)
            || ctx.arrays.contains_key(base_for_check)
            || ctx.assoc_arrays.contains_key(base_for_check)
            || ctx.namerefs.contains_key(base_for_check)
            || std::env::var(base_for_check).is_ok()
            // Positional parameters: any non-negative integer is a valid
            // positional param name (even if beyond current $# — it's just
            // unset, not nonexistent).
            || base_for_check.parse::<usize>().is_ok()
            // Special variables: @, *, #, ?, -, !, $, 0, _
            || matches!(base_for_check, "@" | "*" | "#" | "?" | "-" | "!" | "$" | "0" | "_");
        if !var_exists {
            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                let p = p.borrow();
                if p.is_empty() {
                    "bash".to_string()
                } else {
                    p.clone()
                }
            });
            eprintln!("{}: {}: invalid indirect expansion", prefix, real_name);
            set_arith_error();
            return String::new();
        }

        // First resolve the indirect: get the value of real_name, use as variable name
        let target = lookup_var(real_name, ctx);
        // Check if the resolved target is a valid variable name.
        // If not (e.g., "aaa bbb" from array expansion), emit an error.
        if !target.is_empty() && !is_valid_var_ref(&target) {
            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                let p = p.borrow();
                if p.is_empty() {
                    "bash".to_string()
                } else {
                    p.clone()
                }
            });
            eprintln!("{}: {}: invalid variable name", prefix, target);
            set_arith_error();
            return String::new();
        }

        // Check nounset BEFORE recursing into the resolved target.
        // bash reports the original indirect name (e.g. "!bar") in the
        // error message, not the resolved target name.  We must check
        // here because the recursive expand_param call would use the
        // target name in its error message instead.
        if ctx.opt_flags.contains('u')
            && !matches!(
                expr.op,
                ParamOp::Default(..) | ParamOp::Assign(..) | ParamOp::Alt(..) | ParamOp::Error(..)
            )
        {
            // Check if the resolved target is "unbound" for nounset:
            // scalar access to an array requires element[0] to be set.
            let target_val = if !target.is_empty() {
                lookup_var(&target, ctx)
            } else {
                String::new()
            };
            let target_unbound = if target.is_empty() {
                true
            } else if target_val.is_empty() {
                !ctx.vars.contains_key(&target)
                    && !ctx.arrays.get(&target).is_some_and(|arr| {
                        if matches!(&expr.op, ParamOp::Transform('a') | ParamOp::Transform('A')) {
                            arr.iter().any(|v| v.is_some())
                        } else {
                            arr.first().is_some_and(|v| v.is_some())
                        }
                    })
                    && !ctx.assoc_arrays.get(&target).is_some_and(|assoc| {
                        if matches!(&expr.op, ParamOp::Transform('a') | ParamOp::Transform('A')) {
                            !assoc.is_empty()
                        } else {
                            assoc.contains_key("0")
                        }
                    })
                    && std::env::var(&target).is_err()
            } else {
                false
            };
            if target_unbound {
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
                set_arith_error();
                set_nounset_error();
                return String::new();
            }
        }

        // Now apply the operator to the target variable
        let indirect_expr = ParamExpr {
            name: target,
            op: expr.op.clone(),
        };
        return expand_param(&indirect_expr, ctx, cmd_sub);
    }

    // Helper: get the join separator based on whether this is a * or @ expansion.
    // For *, join with IFS[0] (space if IFS unset, empty if IFS="").
    // For @, always join with space (field splitting handled by callers).
    let ifs_join_sep = |is_star: bool| -> String {
        if is_star {
            match ctx.vars.get("IFS") {
                None => " ".to_string(),
                Some(s) if s.is_empty() => String::new(),
                Some(s) => s.chars().next().unwrap_or(' ').to_string(),
            }
        } else {
            " ".to_string()
        }
    };

    // For $@ and $* with operations, apply per-element
    if (expr.name == "@" || expr.name == "*")
        && !matches!(expr.op, ParamOp::None | ParamOp::Length)
        && ctx.positional.len() > 1
    {
        let sep = ifs_join_sep(expr.name == "*");

        // ${@@K} / ${@@k} — for positional params, single-quote each element (same as @Q)
        if matches!(&expr.op, ParamOp::Transform('K') | ParamOp::Transform('k')) {
            let quoted: Vec<String> = ctx.positional[1..].iter().map(|s| shell_quote(s)).collect();
            return quoted.join(" ");
        }

        // ${@@A} / ${*@A} — assignment form for positional params: set -- 'val1' 'val2' ...
        if matches!(&expr.op, ParamOp::Transform('A')) {
            let quoted: Vec<String> = ctx.positional[1..]
                .iter()
                .map(|s| format!("'{}'", s.replace('\'', "'\\''")))
                .collect();
            return format!("set -- {}", quoted.join(" "));
        }

        // For Substring: slice the positional params array
        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
            let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);
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
                let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                if len < 0 {
                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                        let p = p.borrow();
                        if p.is_empty() {
                            "bash".to_string()
                        } else {
                            p.clone()
                        }
                    });
                    eprintln!("{}: {}: substring expression < 0", prefix, len_str.trim());
                    set_arith_error();
                    return String::new();
                } else {
                    (start + len as usize).min(count)
                }
            } else {
                count
            };
            let sliced: Vec<&str> = params[start..end].iter().map(|s| s.as_str()).collect();
            return sliced.join(&sep);
        }
        // Handle Default/Alt/Error/Assign for $@/$* with ops.
        // Bash checks the space-joined value for :- / :+ variants.
        if matches!(
            &expr.op,
            ParamOp::Default(..) | ParamOp::Alt(..) | ParamOp::Error(..) | ParamOp::Assign(..)
        ) {
            let val = lookup_var(&expr.name, ctx);
            let set = ctx.is_param_set(&expr.name);
            let use_default = match &expr.op {
                ParamOp::Default(colon, _)
                | ParamOp::Assign(colon, _)
                | ParamOp::Error(colon, _) => {
                    let empty = if *colon { val.is_empty() } else { false };
                    !set || empty
                }
                _ => false,
            };
            let use_alt = match &expr.op {
                ParamOp::Alt(colon, _) => {
                    let empty = if *colon { val.is_empty() } else { false };
                    set && !empty
                }
                _ => false,
            };
            if use_default {
                if let ParamOp::Error(_, word) = &expr.op {
                    let msg = expand_word_nosplit_ctx(word, ctx, cmd_sub);
                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                        let p = p.borrow();
                        if p.is_empty() {
                            "bash".to_string()
                        } else {
                            p.clone()
                        }
                    });
                    let error_msg = if msg.is_empty() {
                        "parameter null or not set"
                    } else {
                        &msg
                    };
                    eprintln!("{}: {}: {}", prefix, expr.name, error_msg);
                    std::process::exit(1);
                }
                return if let ParamOp::Default(_, word) | ParamOp::Assign(_, word) = &expr.op {
                    expand_word_nosplit_ctx(word, ctx, cmd_sub)
                } else {
                    String::new()
                };
            } else if use_alt {
                return if let ParamOp::Alt(_, word) = &expr.op {
                    expand_word_nosplit_ctx(word, ctx, cmd_sub)
                } else {
                    String::new()
                };
            } else if matches!(&expr.op, ParamOp::Alt(..)) {
                return String::new();
            }
            // Default/Assign/Error not active — fall through to normal element expansion
        }

        let elements: Vec<String> = ctx.positional[1..]
            .iter()
            .map(|elem| apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name))
            .collect();
        return elements.join(&sep);
    }

    // For array[@] or array[*] with operations, apply per-element
    if let Some(bracket) = expr.name.find('[') {
        let base = &expr.name[..bracket];
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        if (idx_str == "@" || idx_str == "*") && !matches!(expr.op, ParamOp::None | ParamOp::Length)
        {
            let resolved = ctx.resolve_nameref(base);
            let sep = ifs_join_sep(idx_str == "*");

            // For Substring on arrays, slice the array by index/element position
            // rather than applying character-level substring to each element.
            // ${arr[@]:offset:length} selects elements starting at array index >= offset.
            if let ParamOp::Substring(offset_str, length_str) = &expr.op {
                let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);
                if let Some(arr) = ctx.arrays.get(&resolved) {
                    // Collect (index, value) pairs for set elements
                    let set_elements: Vec<(usize, &str)> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| v.as_ref().map(|s| (i, s.as_str())))
                        .collect();
                    let count = set_elements.len();
                    // For negative offsets, compute from the array's total length
                    // (highest_index + 1), not from the count of set elements.
                    let effective_offset = if offset < 0 {
                        let arr_len = array_effective_len(arr) as i64;
                        (arr_len + offset).max(0)
                    } else {
                        offset
                    };
                    let start = set_elements
                        .iter()
                        .position(|(idx, _)| *idx >= effective_offset as usize)
                        .unwrap_or(count);
                    let end = if let Some(len_str) = length_str {
                        let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                        if len < 0 {
                            // Negative length: count from end of set-elements list
                            let target = (count as i64 + len).max(0) as usize;
                            target.max(start)
                        } else {
                            (start + len as usize).min(count)
                        }
                    } else {
                        count
                    };
                    let sliced: Vec<&str> =
                        set_elements[start..end].iter().map(|(_, v)| *v).collect();
                    return sliced.join(&sep);
                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                    // For assoc arrays, slice the values list
                    let values: Vec<&str> = assoc.values().map(|s| s.as_str()).collect();
                    let count = values.len();
                    let start = if offset < 0 {
                        (count as i64 + offset).max(0) as usize
                    } else {
                        (offset as usize).min(count)
                    };
                    let end = if let Some(len_str) = length_str {
                        let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                        if len < 0 {
                            let target = (count as i64 + len).max(0) as usize;
                            target.max(start)
                        } else {
                            (start + len as usize).min(count)
                        }
                    } else {
                        count
                    };
                    return values[start..end].join(&sep);
                } else if let Some(val) = ctx.vars.get(&resolved) {
                    // Scalar treated as single-element array — apply character-level
                    // substring (same as ${var:offset:length}).
                    let chars: Vec<char> = val.chars().collect();
                    let count = chars.len() as i64;
                    let start = if offset < 0 {
                        (count + offset).max(0) as usize
                    } else {
                        (offset as usize).min(chars.len())
                    };
                    let end = if let Some(len_str) = length_str {
                        let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                        if len < 0 {
                            let target = (count + len).max(0) as usize;
                            target.max(start)
                        } else {
                            (start + len as usize).min(chars.len())
                        }
                    } else {
                        chars.len()
                    };
                    return chars[start..end].iter().collect();
                } else {
                    return String::new();
                }
            }

            // ${arr[@]@K} / ${arr[@]@k} — key-value pairs for arrays
            if matches!(&expr.op, ParamOp::Transform('K') | ParamOp::Transform('k')) {
                let uppercase = matches!(&expr.op, ParamOp::Transform('K'));
                if let Some(arr) = ctx.arrays.get(&resolved) {
                    let pairs: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| {
                            v.as_ref().map(|val| {
                                if uppercase {
                                    let qv = val
                                        .replace('\\', "\\\\")
                                        .replace('"', "\\\"")
                                        .replace('$', "\\$")
                                        .replace('`', "\\`");
                                    format!("{} \"{}\"", i, qv)
                                } else {
                                    format!("{} {}", i, val)
                                }
                            })
                        })
                        .collect();
                    return pairs.join(" ");
                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                    let pairs: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| {
                            if uppercase {
                                // Bash quotes keys only when they contain
                                // special characters; values are always quoted.
                                let needs_key_quote = k.is_empty()
                                    || k.chars().any(|c| {
                                        !c.is_alphanumeric()
                                            && c != '_'
                                            && c != '-'
                                            && c != '.'
                                            && c != '/'
                                            && c != '='
                                    });
                                let qv = v
                                    .replace('\\', "\\\\")
                                    .replace('"', "\\\"")
                                    .replace('$', "\\$")
                                    .replace('`', "\\`");
                                if needs_key_quote {
                                    let qk = k
                                        .replace('\\', "\\\\")
                                        .replace('"', "\\\"")
                                        .replace('$', "\\$")
                                        .replace('`', "\\`");
                                    format!("\"{}\" \"{}\"", qk, qv)
                                } else {
                                    format!("{} \"{}\"", k, qv)
                                }
                            } else {
                                format!("{} {}", k, v)
                            }
                        })
                        .collect();
                    return pairs.join(" ");
                }
                // Scalar — fall through to normal handler (single-quote the value)
            }

            // For non-Substring ops on arrays, apply per-element
            // ${arr[@]@A} — full declaration form for arrays
            #[inline]
            fn quote_assoc_key_for_at_a(key: &str) -> String {
                // Quote associative array keys for @A transform output,
                // matching bash's declare -p format: keys with shell-special
                // characters are wrapped in "..." with $, `, \, " escaped.
                let needs_quoting = key.is_empty()
                    || key.chars().any(|c| {
                        matches!(
                            c,
                            ' ' | '$'
                                | '!'
                                | '`'
                                | '"'
                                | '\\'
                                | '\''
                                | '('
                                | ')'
                                | '{'
                                | '}'
                                | '<'
                                | '>'
                                | '|'
                                | '&'
                                | ';'
                                | '*'
                                | '?'
                                | '['
                                | ']'
                                | '~'
                                | '#'
                                | '@'
                        )
                    });
                if needs_quoting {
                    let mut out = String::from("\"");
                    for ch in key.chars() {
                        match ch {
                            '$' | '`' | '\\' | '"' => {
                                out.push('\\');
                                out.push(ch);
                            }
                            _ => out.push(ch),
                        }
                    }
                    out.push('"');
                    out
                } else {
                    key.to_string()
                }
            }
            if let ParamOp::Transform('A') = &expr.op {
                let attrs_key = format!("__ATTRS__{}", resolved);
                let attrs = ctx.vars.get(&attrs_key).cloned().unwrap_or_default();
                let is_unset = ctx.vars.contains_key(&format!("__UNSET__{}", resolved));
                if let Some(arr) = ctx.arrays.get(&resolved) {
                    let elems: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| {
                            v.as_ref()
                                .map(|val| format!("[{}]=\"{}\"", i, val.replace('"', "\\\"")))
                        })
                        .collect();
                    let flags = if attrs.is_empty() {
                        "a".to_string()
                    } else {
                        attrs
                    };
                    // If declared-but-unset, omit =() to match bash.
                    // An explicitly assigned empty array (e.g. B=()) should
                    // still show =(), only truly unset arrays omit it.
                    if is_unset {
                        return format!("declare -{} {}", flags, resolved);
                    }
                    return format!("declare -{} {}=({})", flags, resolved, elems.join(" "));
                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                    let elems: Vec<String> = assoc
                        .iter()
                        .map(|(k, v)| {
                            let quoted_key = quote_assoc_key_for_at_a(k);
                            format!("[{}]=\"{}\"", quoted_key, v.replace('"', "\\\""))
                        })
                        .collect();
                    let flags = if attrs.is_empty() {
                        "A".to_string()
                    } else {
                        attrs
                    };
                    // Bash adds a trailing space before ) for assoc arrays
                    return format!("declare -{} {}=({} )", flags, resolved, elems.join(" "));
                } else {
                    // Scalar variable (possibly declared-but-unset without -a flag)
                    // — fall through to the scalar @A handler at the bottom of expand_param
                    // by NOT returning here. We need to skip the per-element code below.
                    let val = lookup_var(&expr.name, ctx);
                    if attrs.is_empty() {
                        if is_unset {
                            return String::new();
                        } else {
                            return format!("{}='{}'", resolved, val.replace('\'', "'\\''"));
                        }
                    } else {
                        let flags = format!("-{}", attrs);
                        if is_unset {
                            return format!("declare {} {}", flags, resolved);
                        } else {
                            return format!(
                                "declare {} {}='{}'",
                                flags,
                                resolved,
                                val.replace('\'', "'\\''")
                            );
                        }
                    }
                }
            }

            // ${arr[@]@a} — attribute string, repeated per element
            if let ParamOp::Transform('a') = &expr.op {
                let attrs_key = format!("__ATTRS__{}", resolved);
                let attrs = ctx.vars.get(&attrs_key).cloned().unwrap_or_default();
                let count = if let Some(arr) = ctx.arrays.get(&resolved) {
                    arr.iter().filter(|v| v.is_some()).count()
                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                    assoc.len()
                } else if ctx.vars.contains_key(&resolved) {
                    1
                } else {
                    0
                };
                if count > 0 {
                    let repeated: Vec<String> = (0..count).map(|_| attrs.clone()).collect();
                    return repeated.join(&sep);
                }
                return attrs;
            }

            // Handle Default/Alt/Error/Assign for array [@]/[*] expansions.
            // Bash checks the space-joined value for :- / :+ variants, and
            // whether the array is set for - / + variants.
            if matches!(
                &expr.op,
                ParamOp::Default(..) | ParamOp::Alt(..) | ParamOp::Error(..) | ParamOp::Assign(..)
            ) {
                let val = lookup_var(&expr.name, ctx);
                let set = ctx.is_param_set(&expr.name);
                let use_default = match &expr.op {
                    ParamOp::Default(colon, _)
                    | ParamOp::Assign(colon, _)
                    | ParamOp::Error(colon, _) => {
                        let empty = if *colon { val.is_empty() } else { false };
                        !set || empty
                    }
                    _ => false,
                };
                let use_alt = match &expr.op {
                    ParamOp::Alt(colon, _) => {
                        let empty = if *colon { val.is_empty() } else { false };
                        set && !empty
                    }
                    _ => false,
                };
                if use_default {
                    if let ParamOp::Error(_, word) = &expr.op {
                        let msg = expand_word_nosplit_ctx(word, ctx, cmd_sub);
                        let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                            let p = p.borrow();
                            if p.is_empty() {
                                "bash".to_string()
                            } else {
                                p.clone()
                            }
                        });
                        let error_msg = if msg.is_empty() {
                            "parameter null or not set"
                        } else {
                            &msg
                        };
                        eprintln!("{}: {}: {}", prefix, expr.name, error_msg);
                        std::process::exit(1);
                    }
                    return if let ParamOp::Default(_, word) | ParamOp::Assign(_, word) = &expr.op {
                        expand_word_nosplit_ctx(word, ctx, cmd_sub)
                    } else {
                        String::new()
                    };
                } else if use_alt {
                    return if let ParamOp::Alt(_, word) = &expr.op {
                        expand_word_nosplit_ctx(word, ctx, cmd_sub)
                    } else {
                        String::new()
                    };
                } else if matches!(&expr.op, ParamOp::Alt(..)) {
                    // Alt not active — produce empty
                    return String::new();
                }
                // Default/Assign/Error not active — fall through to normal element expansion
            }

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
            return modified.join(&sep);
        }
    }

    // For ${#arr[expr]} (Length op) on indexed arrays with no set elements,
    // return "0" immediately WITHOUT calling lookup_var — lookup_var would
    // evaluate the subscript through arithmetic, producing spurious errors
    // (e.g. when array_expand_once prevents $(...) expansion in subscripts).
    // Bash skips subscript evaluation entirely for empty arrays in this case.
    if matches!(&expr.op, ParamOp::Length)
        && let Some(bracket) = expr.name.find('[')
    {
        let base = &expr.name[..bracket];
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        let resolved = ctx.resolve_nameref(base);
        if idx_str != "@" && idx_str != "*" && !ctx.assoc_arrays.contains_key(&resolved) {
            // If the variable exists as an indexed array but has no
            // set elements, ${#arr[expr]} returns "0" without
            // evaluating the subscript (bash optimisation).
            //
            // Skip this shortcut when:
            // - The variable is a scalar (e.g. x=hello) — scalars are
            //   treated as single-element arrays, so ${#x[0]} must
            //   return the string length, not 0.
            // - The variable doesn't exist at all — we must NOT
            //   return "0" here because `set -u` (nounset) should
            //   produce an "unbound variable" error for completely
            //   unset variables, even with ${#var[N]} syntax.
            if let Some(arr) = ctx.arrays.get(&resolved) {
                let has_set_elements = arr.iter().any(|v| v.is_some());
                if !has_set_elements {
                    return "0".to_string();
                }
            }
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

    // For ${#arr[N]} (Length with subscript), bash exempts from nounset
    // when the array base name exists and has at least one set element —
    // the length of a missing element is simply 0, not an error.
    // Only completely unset arrays/variables trigger nounset for ${#var[N]}.
    let length_subscript_exempt = if matches!(&expr.op, ParamOp::Length)
        && let Some(bracket) = expr.name.find('[')
    {
        let base = &expr.name[..bracket];
        let resolved_base = ctx.resolve_nameref(base);
        ctx.arrays
            .get(&resolved_base)
            .is_some_and(|a| a.iter().any(|v| v.is_some()))
            || ctx
                .assoc_arrays
                .get(&resolved_base)
                .is_some_and(|a| !a.is_empty())
    } else {
        false
    };

    if !(!val.is_empty()
        || !ctx.opt_flags.contains('u')
        || length_subscript_exempt
        // Skip if nounset error was already reported (e.g. from subscript
        // evaluation when name is a nameref to `a[k]` and `k` is unbound)
        || is_nounset_error()
        || matches!(
            expr.op,
            ParamOp::Default(..) | ParamOp::Assign(..) | ParamOp::Alt(..) | ParamOp::Error(..)
        )
        || matches!(
            expr.name.as_str(),
            "?" | "$" | "#" | "@" | "*" | "-" | "0"
        )
        // $! is only exempt from nounset if a background job has been started
        || expr.name == "!" && ctx.last_bg_pid != 0)
        && (is_positional_unbound
            || (expr.name.parse::<usize>().is_err()
                && !ctx.vars.contains_key(&expr.name)
                // For arrays accessed as scalar (no [@] subscript), the
                // "bound" check depends on the operation:
                //
                // - For @a/@A transforms: the variable is bound if the array
                //   has ANY set elements (even without element[0]).  This is
                //   because @a/@A report attributes/declaration of the whole
                //   variable, not element[0].
                //
                // - For all other ops (${foo}, ${#foo}, ${foo@Q}, etc.): the
                //   variable is bound only if element[0] is set.  bash treats
                //   empty arrays and sparse arrays without element[0] as
                //   "unbound" for these scalar references.
                && !ctx.arrays.get(&expr.name).is_some_and(|arr| {
                    if matches!(&expr.op, ParamOp::Transform('a') | ParamOp::Transform('A')) {
                        arr.iter().any(|v| v.is_some())
                    } else {
                        arr.first().is_some_and(|v| v.is_some())
                    }
                })
                && !ctx.assoc_arrays.get(&expr.name).is_some_and(|assoc| {
                    if matches!(&expr.op, ParamOp::Transform('a') | ParamOp::Transform('A')) {
                        !assoc.is_empty()
                    } else {
                        assoc.contains_key("0")
                    }
                })
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
                    // Scalar with [@] or [*]: ${#scalar[@]} returns 1 if set, 0 if unset.
                    // Scalars are treated as single-element arrays.
                    if ctx.vars.contains_key(&resolved) {
                        return "1".to_string();
                    }
                    return "0".to_string();
                }
                // ${#arr[N]} — for indexed arrays, check empty-array shortcut
                // and negative-N bounds.
                if !ctx.assoc_arrays.contains_key(&resolved) && idx_str != "@" && idx_str != "*" {
                    // If the variable exists as an indexed array but has
                    // no set elements, ${#arr[expr]} returns "0" without
                    // evaluating the subscript (bash optimisation).
                    // Skip for scalars (${#x[0]} → string length) and
                    // completely unset variables (nounset should fire).
                    if let Some(arr) = ctx.arrays.get(&resolved) {
                        let has_set_elements = arr.iter().any(|v| v.is_some());
                        if !has_set_elements {
                            return "0".to_string();
                        }
                    }
                    // Don't re-evaluate the subscript here — lookup_var above
                    // already evaluated it via eval_arith_full_with_assoc.
                    // Re-evaluating would produce a duplicate error message
                    // when the subscript is an invalid expression (e.g. with
                    // array_expand_once preventing $(...) expansion).
                    // For negative-index bounds checking, parse only simple
                    // integer literals; complex expressions were already
                    // validated during the lookup_var call.
                    if let Ok(v) = idx_str.trim().parse::<i64>()
                        && v < 0
                    {
                        let arr_len = ctx
                            .arrays
                            .get(&resolved)
                            .map(|a| array_effective_len(a) as i64)
                            .unwrap_or(0);
                        if arr_len + v < 0 {
                            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                                let p = p.borrow();
                                if p.is_empty() {
                                    "bash".to_string()
                                } else {
                                    p.clone()
                                }
                            });
                            eprintln!("{}: [{}]: bad array subscript", prefix, v);
                            crate::expand::set_arith_error();
                            return String::new();
                        }
                    }
                }
            }
            mbstrlen(&val).to_string()
        }
        ParamOp::Indirect => {
            // ${!var} — indirect expansion
            // For namerefs, ${!var} returns the nameref target name (what var
            // references), NOT the value of the target variable.  This is
            // bash's behavior: `declare -n ref=foo; ${!ref}` → "foo".
            // For non-namerefs, ${!var} does classic indirect expansion:
            // get the value of var, use that as a variable name, look up its value.
            let base_name = if let Some(bracket) = expr.name.find('[') {
                &expr.name[..bracket]
            } else {
                expr.name.as_str()
            };
            if ctx.namerefs.contains_key(base_name) {
                if expr.name.contains('[') {
                    // ${!nameref[N]} — invalid indirect expansion through nameref
                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                        let p = p.borrow();
                        if p.is_empty() {
                            "bash".to_string()
                        } else {
                            p.clone()
                        }
                    });
                    eprintln!("{}: {}: invalid indirect expansion", prefix, expr.name);
                    set_arith_error();
                    String::new()
                } else {
                    // var is a nameref — return the target name (resolving the
                    // full nameref chain).  If the nameref target is empty
                    // (unbound nameref), return empty.
                    ctx.resolve_nameref(&expr.name)
                }
            } else {
                let target = lookup_var(&expr.name, ctx);
                if target.is_empty() {
                    // Check if this is an unset variable (not just empty) —
                    // bash reports "invalid indirect expansion" for truly unset vars
                    let is_truly_unset = !ctx.vars.contains_key(&expr.name)
                        && !ctx.arrays.contains_key(&expr.name)
                        && !ctx.assoc_arrays.contains_key(&expr.name)
                        && !ctx.namerefs.contains_key(&expr.name)
                        && std::env::var(&expr.name).is_err()
                        && expr.name.parse::<usize>().is_err()
                        && !matches!(
                            expr.name.as_str(),
                            "@" | "*" | "#" | "?" | "-" | "!" | "$" | "0" | "_"
                        );
                    if is_truly_unset {
                        let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                            let p = p.borrow();
                            if p.is_empty() {
                                "bash".to_string()
                            } else {
                                p.clone()
                            }
                        });
                        eprintln!("{}: {}: invalid indirect expansion", prefix, expr.name);
                        set_arith_error();
                    }
                    String::new()
                } else if !is_valid_var_ref(&target) {
                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                        let p = p.borrow();
                        if p.is_empty() {
                            "bash".to_string()
                        } else {
                            p.clone()
                        }
                    });
                    eprintln!("{}: {}: invalid variable name", prefix, target);
                    set_arith_error();
                    String::new()
                } else {
                    lookup_var(&target, ctx)
                }
            }
        }
        ParamOp::NamePrefix(ch) => {
            // ${!prefix@} or ${!prefix*} — variable names matching prefix
            // Must include scalars, indexed arrays, and associative arrays
            let prefix = &expr.name;
            let mut name_set: std::collections::BTreeSet<&String> =
                std::collections::BTreeSet::new();
            for k in ctx.vars.keys() {
                if k.starts_with(prefix.as_str()) {
                    name_set.insert(k);
                }
            }
            for k in ctx.arrays.keys() {
                if k.starts_with(prefix.as_str()) {
                    name_set.insert(k);
                }
            }
            for k in ctx.assoc_arrays.keys() {
                if k.starts_with(prefix.as_str()) {
                    name_set.insert(k);
                }
            }
            let names: Vec<&str> = name_set.iter().map(|s| s.as_str()).collect();
            // ${!prefix*} joins with first char of IFS (like "$*");
            // ${!prefix@} always joins with space (like "$@" in double quotes).
            let sep = if *ch == '*' {
                match super::ifs_first_char(ctx.vars) {
                    Some(c) => c.to_string(),
                    None => String::new(), // IFS="" → no separator
                }
            } else {
                " ".to_string()
            };
            names.join(&sep)
        }
        ParamOp::ArrayIndices(_ch, transform) => {
            // ${!arr[@]} or ${!arr[*]} — array indices/keys
            let resolved = ctx.resolve_nameref(&expr.name);
            let elements: Vec<String> = if let Some(arr) = ctx.arrays.get(&resolved) {
                (0..arr.len())
                    .filter(|&i| arr[i].is_some())
                    .map(|i| i.to_string())
                    .collect()
            } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                assoc.keys().cloned().collect()
            } else {
                // Scalar variable — index 0
                if ctx.vars.contains_key(&resolved) {
                    vec!["0".to_string()]
                } else {
                    vec![]
                }
            };
            // Apply optional transform to each element
            let elements: Vec<String> = if let Some(t) = transform {
                elements
                    .into_iter()
                    .map(|e| match t {
                        'Q' => shell_quote(&e),
                        'E' => expand_backslash_escapes(&e),
                        'U' => e.to_uppercase(),
                        'L' => e.to_lowercase(),
                        'u' => {
                            let mut chars = e.chars();
                            match chars.next() {
                                None => String::new(),
                                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                            }
                        }
                        _ => e,
                    })
                    .collect()
            } else {
                elements
            };
            elements.join(" ")
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
                // Cannot assign to positional or special parameters
                let is_positional_or_special = expr.name.parse::<usize>().is_ok()
                    || matches!(
                        expr.name.as_str(),
                        "@" | "*" | "#" | "?" | "-" | "$" | "!" | "_"
                    );
                if is_positional_or_special {
                    let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                        let p = p.borrow();
                        if p.is_empty() {
                            "bash".to_string()
                        } else {
                            p.clone()
                        }
                    });
                    eprintln!("{}: ${}: cannot assign in this way", prefix, expr.name);
                    set_arith_error();
                    return String::new();
                }
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
                let prefix = super::EXPAND_ERROR_PREFIX.with(|p| {
                    let p = p.borrow();
                    if p.is_empty() {
                        "bash".to_string()
                    } else {
                        p.clone()
                    }
                });
                let error_msg = if msg.is_empty() {
                    "parameter null or not set"
                } else {
                    &msg
                };
                eprintln!("{}: {}: {}", prefix, expr.name, error_msg);
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
            let rep = super::expand_replacement_word(replacement, ctx, cmd_sub);
            match &expr.op {
                ParamOp::ReplaceAll(..) => pattern_replace(&val, &pat, &rep, true),
                ParamOp::ReplacePrefix(..) => {
                    // Replace only if pattern matches at start (longest match first)
                    for i in (0..=val.len()).rev() {
                        if !val.is_char_boundary(i) {
                            continue;
                        }
                        if shell_pattern_match(&val[..i], &pat) {
                            let effective_rep = process_replacement_amp(&rep, &val[..i]);
                            return format!("{}{}", effective_rep, &val[i..]);
                        }
                    }
                    val
                }
                ParamOp::ReplaceSuffix(..) => {
                    // Replace only if pattern matches at end (longest match first)
                    for i in 0..=val.len() {
                        if !val.is_char_boundary(i) {
                            continue;
                        }
                        if shell_pattern_match(&val[i..], &pat) {
                            let effective_rep = process_replacement_amp(&rep, &val[i..]);
                            return format!("{}{}", &val[..i], effective_rep);
                        }
                    }
                    val
                }
                _ => pattern_replace(&val, &pat, &rep, false),
            }
        }
        ParamOp::Substring(offset_str, length_str) => {
            let offset: i64 = parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
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
        ParamOp::Transform(ch) => {
            // For transforms, unset variables produce empty string (not quoted empty).
            // Bash distinguishes unset from empty: ${unset@Q} → "", ${empty@Q} → "''".
            // Check if the variable is truly unset (not in vars, arrays, assoc_arrays,
            // not a special variable, and not in the environment).
            let base_name = if let Some(bracket) = expr.name.find('[') {
                &expr.name[..bracket]
            } else {
                expr.name.as_str()
            };
            let is_special = matches!(
                base_name,
                "?" | "$"
                    | "#"
                    | "@"
                    | "*"
                    | "-"
                    | "0"
                    | "!"
                    | "RANDOM"
                    | "BASHPID"
                    | "SRANDOM"
                    | "SECONDS"
                    | "EPOCHSECONDS"
                    | "EPOCHREALTIME"
                    | "BASH_COMMAND"
                    | "LINENO"
                    | "BASH_SUBSHELL"
            );
            let is_positional = base_name.parse::<usize>().is_ok();
            // A variable is "truly unset" if it was never declared at all.
            // declared-but-unset variables (e.g. `declare -lr VAR1`) have
            // an __UNSET__ entry injected by inject_transform_attrs, so
            // they are NOT truly unset — @A and @a should still report
            // their declaration/attributes.
            // Note: __ATTRS__ is injected for ANY @a/@A transform (even
            // for unset vars), so we cannot use it to determine existence.
            // Instead, check vars, arrays, assoc_arrays, declared_unset
            // markers (__UNSET__), and environment.
            let has_declared_unset = ctx.vars.contains_key(&format!("__UNSET__{}", base_name));
            let is_truly_unset = !is_special
                && !is_positional
                && !has_declared_unset
                && !ctx.vars.contains_key(base_name)
                && !ctx.arrays.contains_key(base_name)
                && !ctx.assoc_arrays.contains_key(base_name)
                && std::env::var(base_name).is_err();
            if is_truly_unset {
                return String::new();
            }
            match ch {
                'E' => expand_backslash_escapes(&val),
                'Q' => shell_quote(&val),
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
                    // Strip [@] or [*] subscript from name for attribute lookup
                    let base_name = if let Some(bracket) = expr.name.find('[') {
                        &expr.name[..bracket]
                    } else {
                        expr.name.as_str()
                    };
                    ctx.vars
                        .get(&format!("__ATTRS__{}", base_name))
                        .cloned()
                        .unwrap_or_default()
                }
                'A' => {
                    // Assignment form: declare -FLAGS name='value'
                    // Strip [@] or [*] subscript from name for attribute/unset lookup
                    let base_name = if let Some(bracket) = expr.name.find('[') {
                        &expr.name[..bracket]
                    } else {
                        expr.name.as_str()
                    };
                    let attrs = ctx
                        .vars
                        .get(&format!("__ATTRS__{}", base_name))
                        .cloned()
                        .unwrap_or_default();
                    // If the variable is declared but unset (no assignment),
                    // omit the ='...' suffix to match bash behavior.
                    let is_unset = ctx.vars.get(&format!("__UNSET__{}", base_name)).is_some();
                    // For arrays accessed as scalar (${foo@A} not ${foo[@]@A}),
                    // if element[0] is not set (empty array, or sparse array
                    // without index 0), bash shows `declare -FLAGS name` with
                    // no value — same as declared-but-unset.
                    let is_array_no_elem0 = ctx
                        .arrays
                        .get(base_name)
                        .is_some_and(|arr| !arr.first().is_some_and(|v| v.is_some()))
                        || ctx
                            .assoc_arrays
                            .get(base_name)
                            .is_some_and(|assoc| !assoc.contains_key("0"));
                    let effectively_unset = is_unset || is_array_no_elem0;
                    if attrs.is_empty() {
                        // Plain variable with no special attributes: use name='value'
                        // form without "declare -- " prefix, matching bash behavior.
                        // For unset variables (never declared or declared-unset with
                        // no attributes), return empty.
                        if effectively_unset {
                            String::new()
                        } else {
                            format!("{}='{}'", base_name, val.replace('\'', "'\\''"))
                        }
                    } else {
                        let flags = format!("-{}", attrs);
                        if effectively_unset {
                            format!("declare {} {}", flags, base_name)
                        } else {
                            format!(
                                "declare {} {}='{}'",
                                flags,
                                base_name,
                                val.replace('\'', "'\\''")
                            )
                        }
                    }
                }
                'K' | 'k' => shell_quote(&val),
                'P' => {
                    // Prompt string expansion: decode prompt escapes, then
                    // expand variables/command substitutions in the result
                    // (matching bash's promptvars behavior).
                    let decoded = decode_prompt_string(&val, ctx);
                    // Perform variable expansion on the decoded string via
                    // cmd_sub.  Escape `"`, backtick, and `\` for double-quote
                    // context so that $VAR references are expanded but special
                    // chars don't break the quoting.
                    let mut escaped = String::with_capacity(decoded.len() + 16);
                    for dc in decoded.chars() {
                        if dc == '"' || dc == '`' || dc == '\\' {
                            escaped.push('\\');
                        }
                        escaped.push(dc);
                    }
                    cmd_sub(&format!("printf '%s' \"{}\"", escaped))
                }
                _ => val,
            }
        }
    }
}
