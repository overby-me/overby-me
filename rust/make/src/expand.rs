//! Variable and function expansion for Make expressions.

use crate::engine::{Engine, VarFlavor, VarOrigin, Variable};
use std::collections::HashMap;

/// Expand all variable references and function calls in a string.
pub fn expand(s: &str, engine: &Engine) -> String {
    expand_with_auto(s, engine, &HashMap::new())
}

/// Expand with automatic variables available.
pub fn expand_with_auto(s: &str, engine: &Engine, auto_vars: &HashMap<&str, String>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' {
            i += 1;
            if i >= chars.len() {
                result.push('$');
                break;
            }
            match chars[i] {
                '$' => {
                    result.push('$');
                    i += 1;
                }
                '(' => {
                    i += 1;
                    let expr = read_balanced(&chars, &mut i, '(', ')');
                    result.push_str(&expand_expr(&expr, engine, auto_vars));
                }
                '{' => {
                    i += 1;
                    let expr = read_balanced(&chars, &mut i, '{', '}');
                    result.push_str(&expand_expr(&expr, engine, auto_vars));
                }
                '@' | '<' | '^' | '+' | '?' | '*' | '|' => {
                    let var = chars[i].to_string();
                    i += 1;
                    // Check for $(@D), $(@F) etc.
                    if i < chars.len() && chars[i] == '(' {
                        // This was actually $(@ ...) but we already consumed $@
                        // Just look up the single-char var
                    }
                    if let Some(val) = auto_vars.get(var.as_str()) {
                        result.push_str(val);
                    } else {
                        result.push_str(&engine.lookup_var(&var));
                    }
                }
                c => {
                    // Any other single character is a single-char variable
                    // reference. `$a`, `$ `, `$,`, `$:` — each looks up a
                    // one-character variable (usually undefined → empty,
                    // which is exactly what idioms like `$\`-continuation
                    // rely on).
                    let var = c.to_string();
                    i += 1;
                    if let Some(val) = auto_vars.get(var.as_str()) {
                        result.push_str(val);
                    } else {
                        result.push_str(&engine.lookup_var(&var));
                    }
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

fn read_balanced(chars: &[char], i: &mut usize, open: char, close: char) -> String {
    let mut depth = 1;
    let mut result = String::new();
    while *i < chars.len() && depth > 0 {
        if chars[*i] == open {
            depth += 1;
            result.push(chars[*i]);
        } else if chars[*i] == close {
            depth -= 1;
            if depth > 0 {
                result.push(chars[*i]);
            }
        } else {
            result.push(chars[*i]);
        }
        *i += 1;
    }
    result
}

/// Expand a $(expr) or ${expr} — either a variable reference or function call.
fn expand_expr(expr: &str, engine: &Engine, auto_vars: &HashMap<&str, String>) -> String {
    // GNU make splits function args on raw commas *before* expansion, so a
    // comma inside a variable value doesn't split the arg list. For
    // substitution refs and plain variable lookups we still need the
    // variable name expanded (to support `$($(x)...)`).

    // First, check if the RAW expr looks like a function call: `<name>
    // <args>` where <name> is a known built-in (no space-containing names).
    if let Some(space_pos) = expr.find([' ', '\t']) {
        let func_name_raw = &expr[..space_pos];
        let func_name = func_name_raw.trim();
        if is_builtin_function(func_name) {
            let args_str = expr[space_pos + 1..].trim_start();
            if let Some(result) = call_function(func_name, args_str, engine, auto_vars) {
                return result;
            }
        }
    }

    let expr_expanded = expand_with_auto(expr, engine, auto_vars);

    // Check for substitution reference: $(VAR:a=b)
    if let Some(colon_pos) = find_subst_colon(&expr_expanded) {
        let varname = &expr_expanded[..colon_pos];
        let subst = &expr_expanded[colon_pos + 1..];
        if let Some(eq_pos) = subst.find('=') {
            let from = &subst[..eq_pos];
            let to = &subst[eq_pos + 1..];
            let val = if let Some(v) = auto_vars.get(varname) {
                v.clone()
            } else {
                engine.lookup_var_with_auto(varname, auto_vars)
            };
            return substitute_ref(&val, from, to);
        }
    }

    // Check for function call: $(func args). If the function name itself
    // contained a variable reference (now expanded), we still need this
    // path so that `$($(fn-name) args)` dispatches.
    if let Some(space_pos) = expr_expanded.find([' ', '\t']) {
        let func_name = &expr_expanded[..space_pos];
        let args_str = expr_expanded[space_pos + 1..].trim_start();

        if let Some(result) = call_function(func_name, args_str, engine, auto_vars) {
            return result;
        }
    }

    // Plain variable lookup
    let varname = expr_expanded.trim();
    if let Some(val) = auto_vars.get(varname) {
        val.clone()
    } else {
        engine.lookup_var_with_auto(varname, auto_vars)
    }
}

fn find_subst_colon(s: &str) -> Option<usize> {
    let mut depth = 0u32;
    for (i, c) in s.chars().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
            }
            ':' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn substitute_ref(value: &str, from: &str, to: &str) -> String {
    // `$(VAR:from=to)` is equivalent to `$(patsubst from,to,$(VAR))` when
    // `from` contains `%`; otherwise it's a plain suffix substitution.
    if from.contains('%') || to.contains('%') {
        return patsubst(value, from, to);
    }
    value
        .split_whitespace()
        .map(|word| {
            if let Some(stripped) = word.strip_suffix(from) {
                format!("{stripped}{to}")
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Try to call a built-in Make function.
fn call_function(
    name: &str,
    args_str: &str,
    engine: &Engine,
    auto_vars: &HashMap<&str, String>,
) -> Option<String> {
    // Split args on commas (respecting nested parens)
    let args = split_args(args_str);

    match name {
        "subst" => {
            if args.len() >= 3 {
                let from = expand_with_auto(&args[0], engine, auto_vars);
                let to = expand_with_auto(&args[1], engine, auto_vars);
                let text = expand_with_auto(&args[2], engine, auto_vars);
                Some(text.replace(&from, &to))
            } else {
                Some(String::new())
            }
        }
        "patsubst" => {
            if args.len() >= 3 {
                let pattern = expand_with_auto(&args[0], engine, auto_vars);
                let replacement = expand_with_auto(&args[1], engine, auto_vars);
                let text = expand_with_auto(&args[2], engine, auto_vars);
                Some(patsubst(&text, &pattern, &replacement))
            } else {
                Some(String::new())
            }
        }
        "strip" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
        }
        "findstring" => {
            if args.len() >= 2 {
                let find = expand_with_auto(&args[0], engine, auto_vars);
                let text = expand_with_auto(&args[1], engine, auto_vars);
                Some(if text.contains(&find) {
                    find
                } else {
                    String::new()
                })
            } else {
                Some(String::new())
            }
        }
        "filter" => {
            if args.len() >= 2 {
                let patterns: Vec<String> = expand_with_auto(&args[0], engine, auto_vars)
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let result: Vec<&str> = text
                    .split_whitespace()
                    .filter(|word| patterns.iter().any(|p| pattern_match(word, p)))
                    .collect();
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "filter-out" => {
            if args.len() >= 2 {
                let patterns: Vec<String> = expand_with_auto(&args[0], engine, auto_vars)
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let result: Vec<&str> = text
                    .split_whitespace()
                    .filter(|word| !patterns.iter().any(|p| pattern_match(word, p)))
                    .collect();
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "sort" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let mut words: Vec<&str> = text.split_whitespace().collect();
            words.sort();
            words.dedup();
            Some(words.join(" "))
        }
        "word" => {
            if args.len() >= 2 {
                let raw = expand_with_auto(&args[0], engine, auto_vars);
                let n = parse_numeric_arg(engine, "word", "first", &raw, true);
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let words: Vec<&str> = text.split_whitespace().collect();
                Some(words.get(n.wrapping_sub(1)).unwrap_or(&"").to_string())
            } else {
                Some(String::new())
            }
        }
        "wordlist" => {
            if args.len() >= 3 {
                let raw_s = expand_with_auto(&args[0], engine, auto_vars);
                let s = parse_numeric_arg(engine, "wordlist", "first", &raw_s, false);
                let raw_e = expand_with_auto(&args[1], engine, auto_vars);
                let e = parse_numeric_arg(engine, "wordlist", "second", &raw_e, false);
                let text = expand_with_auto(&args[2], engine, auto_vars);
                let words: Vec<&str> = text.split_whitespace().collect();
                if s == 0 || e == 0 || s > e {
                    Some(String::new())
                } else {
                    let start = s.saturating_sub(1).min(words.len());
                    let end = e.min(words.len());
                    Some(words[start..end].join(" "))
                }
            } else {
                Some(String::new())
            }
        }
        "words" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(text.split_whitespace().count().to_string())
        }
        "firstword" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(text.split_whitespace().next().unwrap_or("").to_string())
        }
        "lastword" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(
                text.split_whitespace()
                    .next_back()
                    .unwrap_or("")
                    .to_string(),
            )
        }
        "dir" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let result: Vec<String> = text
                .split_whitespace()
                .map(|name| {
                    if let Some(pos) = name.rfind('/') {
                        name[..=pos].to_string()
                    } else {
                        "./".to_string()
                    }
                })
                .collect();
            Some(result.join(" "))
        }
        "notdir" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let result: Vec<&str> = text
                .split_whitespace()
                .map(|name| {
                    if let Some(pos) = name.rfind('/') {
                        &name[pos + 1..]
                    } else {
                        name
                    }
                })
                .collect();
            Some(result.join(" "))
        }
        "suffix" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let result: Vec<&str> = text
                .split_whitespace()
                .filter_map(|name| {
                    let base = name.rfind('/').map(|p| &name[p + 1..]).unwrap_or(name);
                    base.rfind('.')
                        .map(|p| &name[name.len() - (base.len() - p)..])
                })
                .collect();
            Some(result.join(" "))
        }
        "basename" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let result: Vec<String> = text
                .split_whitespace()
                .map(|name| {
                    let base = name.rfind('/').map(|p| &name[p + 1..]).unwrap_or(name);
                    if let Some(dot) = base.rfind('.') {
                        name[..name.len() - (base.len() - dot)].to_string()
                    } else {
                        name.to_string()
                    }
                })
                .collect();
            Some(result.join(" "))
        }
        "addsuffix" => {
            if args.len() >= 2 {
                let suffix = expand_with_auto(&args[0], engine, auto_vars);
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let result: Vec<String> = text
                    .split_whitespace()
                    .map(|w| format!("{w}{suffix}"))
                    .collect();
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "addprefix" => {
            if args.len() >= 2 {
                let prefix = expand_with_auto(&args[0], engine, auto_vars);
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let result: Vec<String> = text
                    .split_whitespace()
                    .map(|w| format!("{prefix}{w}"))
                    .collect();
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "join" => {
            if args.len() >= 2 {
                let list1_expanded = expand_with_auto(&args[0], engine, auto_vars);
                let list1: Vec<&str> = list1_expanded.split_whitespace().collect();
                let list2_expanded = expand_with_auto(&args[1], engine, auto_vars);
                let list2: Vec<&str> = list2_expanded.split_whitespace().collect();
                let max = list1.len().max(list2.len());
                let mut result = Vec::new();
                for i in 0..max {
                    let a = list1.get(i).unwrap_or(&"");
                    let b = list2.get(i).unwrap_or(&"");
                    result.push(format!("{a}{b}"));
                }
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "wildcard" => {
            let pattern = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let mut result = Vec::new();
            for pat in pattern.split_whitespace() {
                if let Ok(paths) = glob::glob(pat) {
                    for entry in paths.flatten() {
                        result.push(entry.to_string_lossy().to_string());
                    }
                }
            }
            Some(result.join(" "))
        }
        "realpath" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let result: Vec<String> = text
                .split_whitespace()
                .filter_map(|name| {
                    std::fs::canonicalize(name)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                })
                .collect();
            Some(result.join(" "))
        }
        "abspath" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let cwd = std::env::current_dir().unwrap_or_default();
            let result: Vec<String> = text
                .split_whitespace()
                .map(|name| {
                    let joined = if std::path::Path::new(name).is_absolute() {
                        name.to_string()
                    } else {
                        cwd.join(name).to_string_lossy().to_string()
                    };
                    normalize_path(&joined)
                })
                .collect();
            Some(result.join(" "))
        }
        "file" => {
            // `$(file OP filename,text)` / `$(file OP filename)`:
            //   >  filename — write text (overwrite)
            //   >> filename — append text
            //   <  filename — read file contents
            if args.is_empty() {
                return Some(String::new());
            }
            let spec = expand_with_auto(&args[0], engine, auto_vars);
            let spec = spec.trim_start();
            let (op, filename) = if let Some(rest) = spec.strip_prefix(">>") {
                ("append", rest.trim().to_string())
            } else if let Some(rest) = spec.strip_prefix('>') {
                ("write", rest.trim().to_string())
            } else if let Some(rest) = spec.strip_prefix('<') {
                ("read", rest.trim().to_string())
            } else {
                return Some(String::new());
            };
            match op {
                "write" | "append" => {
                    use std::io::Write;
                    let mut text = if args.len() >= 2 {
                        expand_with_auto(&args[1], engine, auto_vars)
                    } else {
                        String::new()
                    };
                    if !text.is_empty() && !text.ends_with('\n') {
                        text.push('\n');
                    }
                    let open_result = if op == "append" {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&filename)
                    } else {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&filename)
                    };
                    if let Ok(mut f) = open_result {
                        let _ = f.write_all(text.as_bytes());
                    }
                    Some(String::new())
                }
                "read" => {
                    // GNU make: strip a single trailing newline from the
                    // file's contents.
                    let mut s = std::fs::read_to_string(&filename).unwrap_or_default();
                    if s.ends_with('\n') {
                        s.pop();
                    }
                    Some(s)
                }
                _ => Some(String::new()),
            }
        }
        "shell" => {
            let cmd = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let shell = engine.lookup_var_or("SHELL", "/bin/sh");
            let shell_flags = engine.lookup_var_or(".SHELLFLAGS", "-c");
            let mut shell_cmd = std::process::Command::new(&shell);
            for flag in shell_flags.split_whitespace() {
                shell_cmd.arg(flag);
            }
            // Re-export env-inherited vars with their current values
            // so `$(shell echo $$FOO)` sees makefile-side updates.
            // Snapshot the sets first, and use raw values for simple
            // vars to avoid re-entering expansion (a recursive var
            // whose body contains `$(shell …)` would otherwise cause
            // infinite recursion through this same code path).
            let env_names: Vec<String> = engine
                .env_inherited
                .borrow()
                .iter()
                .filter(|n| n.as_str() != "SHELL")
                .cloned()
                .collect();
            for name in &env_names {
                if engine.var_flavor(name) == VarFlavor::Simple {
                    shell_cmd.env(name, engine.lookup_var_raw(name));
                }
            }
            let export_names: Vec<String> = engine.exports.borrow().iter().cloned().collect();
            for name in &export_names {
                if engine.var_flavor(name) == VarFlavor::Simple {
                    shell_cmd.env(name, engine.lookup_var_raw(name));
                }
            }
            match shell_cmd.arg(&cmd).output() {
                Ok(output) => {
                    let status = output
                        .status
                        .code()
                        .or_else(|| {
                            use std::os::unix::process::ExitStatusExt;
                            output.status.signal().map(|s| 128 + s)
                        })
                        .unwrap_or(0);
                    engine.set_var_with_origin(
                        ".SHELLSTATUS",
                        &status.to_string(),
                        VarFlavor::Simple,
                        VarOrigin::Default,
                    );
                    let s = String::from_utf8_lossy(&output.stdout)
                        .replace('\n', " ")
                        .trim_end()
                        .to_string();
                    Some(s)
                }
                Err(_) => {
                    engine.set_var_with_origin(
                        ".SHELLSTATUS",
                        "127",
                        VarFlavor::Simple,
                        VarOrigin::Default,
                    );
                    Some(String::new())
                }
            }
        }
        "if" => {
            if args.len() >= 2 {
                let cond = expand_with_auto(&args[0], engine, auto_vars);
                if !cond.trim().is_empty() {
                    Some(expand_with_auto(&args[1], engine, auto_vars))
                } else if args.len() >= 3 {
                    // Extra commas in the else branch stay literal in GNU
                    // make — `$(if ,t,f,g)` → else = "f,g".
                    let else_text = args[2..].join(",");
                    Some(expand_with_auto(&else_text, engine, auto_vars))
                } else {
                    Some(String::new())
                }
            } else {
                Some(String::new())
            }
        }
        "let" => {
            if args.len() < 3 {
                let prefix = match engine.current_source.borrow().as_ref() {
                    Some((file, line)) => format!("{file}:{line}: "),
                    None => String::new(),
                };
                eprintln!(
                    "{prefix}*** insufficient number of arguments ({}) to function 'let'.  Stop.",
                    args.len()
                );
                std::process::exit(2);
            }
            if args.len() >= 3 {
                let names_raw = expand_with_auto(&args[0], engine, auto_vars);
                let names: Vec<&str> = names_raw.split_whitespace().collect();
                let values_raw = expand_with_auto(&args[1], engine, auto_vars);
                // Match values to names. If more values than names, the
                // last name captures the remainder (including the whitespace
                // that separated it from the next word).
                let body = args[2..].join(",");
                let values = values_raw.as_str();
                // Save prior bindings so we can restore after the body.
                let mut saved: Vec<(String, Option<Variable>)> = Vec::new();
                let mut cursor = 0usize;
                for (i, name) in names.iter().enumerate() {
                    if name.is_empty() {
                        continue;
                    }
                    saved.push((name.to_string(), engine.vars.borrow().get(*name).cloned()));
                    let is_last = i + 1 == names.len();
                    // Skip leading whitespace for next word.
                    while cursor < values.len()
                        && values[cursor..]
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_whitespace())
                    {
                        cursor += values[cursor..].chars().next().unwrap().len_utf8();
                    }
                    let slice = &values[cursor..];
                    let assigned = if is_last {
                        // Last var gets the remainder (no trim on right in
                        // GNU make — whitespace is preserved).
                        cursor = values.len();
                        slice.to_string()
                    } else {
                        // Consume next whitespace-separated word.
                        let word_end = slice
                            .char_indices()
                            .find(|(_, c)| c.is_whitespace())
                            .map(|(i, _)| i)
                            .unwrap_or(slice.len());
                        let word = &slice[..word_end];
                        cursor += word_end;
                        word.to_string()
                    };
                    engine.set_var_with_origin(
                        name,
                        &assigned,
                        VarFlavor::Simple,
                        VarOrigin::Automatic,
                    );
                }
                let result = expand_with_auto(&body, engine, auto_vars);
                // Restore prior bindings.
                for (name, prev) in saved.into_iter().rev() {
                    let mut vars = engine.vars.borrow_mut();
                    match prev {
                        Some(v) => {
                            vars.insert(name, v);
                        }
                        None => {
                            vars.remove(&name);
                        }
                    }
                }
                Some(result)
            } else {
                Some(String::new())
            }
        }
        "intcmp" => {
            if args.len() >= 2 {
                let lhs = expand_with_auto(&args[0], engine, auto_vars);
                let rhs = expand_with_auto(&args[1], engine, auto_vars);
                let lhs_n: Option<i64> = lhs.trim().parse().ok();
                let rhs_n: Option<i64> = rhs.trim().parse().ok();
                let check_nonnumeric = |val: &str, which: &str| {
                    if lhs_n.is_none() && which == "first" || rhs_n.is_none() && which == "second" {
                        let prefix = match engine.current_source.borrow().as_ref() {
                            Some((file, line)) => format!("{file}:{line}: "),
                            None => String::new(),
                        };
                        if val.trim().is_empty() {
                            eprintln!(
                                "{prefix}*** non-numeric {which} argument to 'intcmp' function: empty value.  Stop."
                            );
                        } else {
                            eprintln!(
                                "{prefix}*** non-numeric {which} argument to 'intcmp' function: '{}'.  Stop.",
                                val.trim()
                            );
                        }
                        std::process::exit(2);
                    }
                };
                check_nonnumeric(&lhs, "first");
                check_nonnumeric(&rhs, "second");
                match (lhs_n, rhs_n) {
                    (Some(l), Some(r)) => {
                        let ord = l.cmp(&r);
                        let pick = |idx: usize| {
                            args.get(idx)
                                .map(|a| expand_with_auto(a, engine, auto_vars))
                                .unwrap_or_default()
                        };
                        // `$(intcmp L,R)` — empty unless equal (returns L).
                        // `$(intcmp L,R,lt)` — lt or empty.
                        // `$(intcmp L,R,lt,eq)` — lt / eq / empty.
                        // `$(intcmp L,R,lt,eq,gt)` — three-way.
                        let result = match (ord, args.len()) {
                            (std::cmp::Ordering::Equal, 2) => lhs.trim().to_string(),
                            (std::cmp::Ordering::Equal, _) => pick(3),
                            (std::cmp::Ordering::Less, n) if n >= 3 => pick(2),
                            (std::cmp::Ordering::Greater, n) if n >= 5 => pick(4),
                            _ => String::new(),
                        };
                        Some(result)
                    }
                    _ => Some(String::new()),
                }
            } else {
                Some(String::new())
            }
        }
        "or" => {
            for arg in &args {
                let val = expand_with_auto(arg, engine, auto_vars);
                if !val.trim().is_empty() {
                    return Some(val);
                }
            }
            Some(String::new())
        }
        "and" => {
            let mut last = String::new();
            for arg in &args {
                last = expand_with_auto(arg, engine, auto_vars);
                if last.trim().is_empty() {
                    return Some(String::new());
                }
            }
            Some(last)
        }
        "foreach" => {
            if args.len() < 3 {
                let prefix = match engine.current_source.borrow().as_ref() {
                    Some((file, line)) => format!("{file}:{line}: "),
                    None => String::new(),
                };
                eprintln!(
                    "{prefix}*** insufficient number of arguments ({}) to function 'foreach'.  Stop.",
                    args.len()
                );
                std::process::exit(2);
            }
            if args.len() >= 3 {
                let var = args[0].trim();
                let list = expand_with_auto(&args[1], engine, auto_vars);
                let body = &args[2];
                // Save any existing binding so we can restore it after
                // the foreach completes. This makes the binding visible
                // to anything that looks up the variable via
                // `engine.lookup_var` (e.g. `$(eval …)`).
                let saved = engine.vars.borrow().get(var).cloned();
                let result: Vec<String> = list
                    .split_whitespace()
                    .map(|word| {
                        let word_owned = word.to_string();
                        engine.vars.borrow_mut().insert(
                            var.to_string(),
                            Variable {
                                value: word_owned.clone(),
                                flavor: VarFlavor::Simple,
                                origin: VarOrigin::Automatic,
                            },
                        );
                        let mut inner_auto = auto_vars.clone();
                        inner_auto.insert(var, word_owned);
                        expand_with_auto(body, engine, &inner_auto)
                    })
                    .collect();
                match saved {
                    Some(v) => {
                        engine.vars.borrow_mut().insert(var.to_string(), v);
                    }
                    None => {
                        engine.vars.borrow_mut().remove(var);
                    }
                }
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "call" => {
            if !args.is_empty() {
                let func_name = expand_with_auto(&args[0], engine, auto_vars);
                let func_name = func_name.trim();
                // If the named function is actually a built-in and the
                // user hasn't defined a variable by that name, dispatch
                // directly to the built-in. GNU make allows `call` over
                // built-ins for things like `$(call notdir,…)`.
                if is_builtin_function(func_name) && engine.lookup_var_raw(func_name).is_empty() {
                    let rest_args: Vec<String> = args
                        .iter()
                        .skip(1)
                        .map(|a| expand_with_auto(a, engine, auto_vars))
                        .collect();
                    let joined = rest_args.join(",");
                    if let Some(result) = call_function(func_name, &joined, engine, auto_vars) {
                        return Some(result);
                    }
                }
                // Use the raw (unexpanded) body so `$1`/`$2`/… remain
                // intact for substitution below.
                let mut body = engine.lookup_var_raw(func_name);
                // Replace numbered parameter references. `$1`–`$9` (bare)
                // and `$(1)`/`${1}` forms.
                body = body.replace("$(0)", func_name);
                body = body.replace("${0}", func_name);
                for (i, arg) in args.iter().skip(1).enumerate() {
                    let val = expand_with_auto(arg, engine, auto_vars);
                    let n = i + 1;
                    body = body.replace(&format!("$({n})"), &val);
                    body = body.replace(&format!("${{{n}}}"), &val);
                    // Bare `$<digit>` reference (only single digit).
                    if n <= 9 {
                        body = body.replace(&format!("${n}"), &val);
                    }
                }
                Some(expand_with_auto(&body, engine, auto_vars))
            } else {
                Some(String::new())
            }
        }
        "value" => {
            let varname = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(engine.lookup_var_raw(varname.trim()))
        }
        "origin" => {
            let varname = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(engine.var_origin(varname.trim()).to_string())
        }
        "flavor" => {
            let varname = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            Some(engine.var_flavor(varname.trim()).to_string())
        }
        "error" => {
            let msg = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let prefix = match engine.current_source.borrow().as_ref() {
                Some((file, line)) => format!("{file}:{line}: "),
                None => String::new(),
            };
            eprintln!("{prefix}*** {msg}.  Stop.");
            std::process::exit(2);
        }
        "warning" => {
            let msg = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let prefix = match engine.current_source.borrow().as_ref() {
                Some((file, line)) => format!("{file}:{line}: "),
                None => String::new(),
            };
            eprintln!("{prefix}{msg}");
            Some(String::new())
        }
        "info" => {
            let msg = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            println!("{msg}");
            Some(String::new())
        }
        "eval" => {
            let text = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            // eval re-parses and executes the text as makefile content
            // We return empty but the engine processes it
            engine.eval_text(&text);
            Some(String::new())
        }
        _ => None, // Not a known function — fall through to variable lookup
    }
}

fn is_builtin_function(name: &str) -> bool {
    matches!(
        name,
        "subst"
            | "patsubst"
            | "strip"
            | "findstring"
            | "filter"
            | "filter-out"
            | "sort"
            | "word"
            | "wordlist"
            | "words"
            | "firstword"
            | "lastword"
            | "dir"
            | "notdir"
            | "suffix"
            | "basename"
            | "addsuffix"
            | "addprefix"
            | "join"
            | "wildcard"
            | "realpath"
            | "abspath"
            | "if"
            | "or"
            | "and"
            | "intcmp"
            | "foreach"
            | "call"
            | "value"
            | "eval"
            | "origin"
            | "flavor"
            | "shell"
            | "file"
            | "error"
            | "warning"
            | "info"
            | "let"
            | "guile"
    )
}

fn split_args(s: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0u32;
    for ch in s.chars() {
        if ch == ',' && depth == 0 {
            args.push(std::mem::take(&mut current));
        } else {
            if ch == '(' || ch == '{' {
                depth += 1;
            } else if (ch == ')' || ch == '}') && depth > 0 {
                depth -= 1;
            }
            current.push(ch);
        }
    }
    args.push(current);
    args
}

pub fn patsubst(text: &str, pattern: &str, replacement: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            if let Some(percent_pos) = pattern.find('%') {
                let prefix = &pattern[..percent_pos];
                let suffix = &pattern[percent_pos + 1..];
                if word.starts_with(prefix)
                    && word.ends_with(suffix)
                    && word.len() >= prefix.len() + suffix.len()
                {
                    let stem_end = word.len() - suffix.len();
                    let stem = &word[prefix.len()..stem_end];
                    replacement.replace('%', stem)
                } else {
                    word.to_string()
                }
            } else if word == pattern {
                replacement.to_string()
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn pattern_match(word: &str, pattern: &str) -> bool {
    // Find the first unescaped `%`. `\%` = literal `%`, `\\%` = literal
    // backslash followed by wildcard `%`.
    let bytes = pattern.as_bytes();
    let mut i = 0;
    let mut unesc = String::new();
    let mut pct: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'%' {
            unesc.push('%');
            i += 2;
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            unesc.push('\\');
            i += 2;
        } else if bytes[i] == b'%' && pct.is_none() {
            pct = Some(unesc.len());
            i += 1;
        } else {
            unesc.push(bytes[i] as char);
            i += 1;
        }
    }
    if let Some(p) = pct {
        let prefix = &unesc[..p];
        let suffix = &unesc[p..];
        word.starts_with(prefix)
            && word.ends_with(suffix)
            && word.len() >= prefix.len() + suffix.len()
    } else {
        word == unesc
    }
}

/// Extract the stem from a pattern match.
/// Parse a numeric argument for `word`/`wordlist`. On invalid input,
/// emit GNU make's diagnostic (with the current source location) and
/// exit. `strict_nonzero` means zero is also rejected ("must be
/// greater than 0"), used for `word`'s first argument.
fn parse_numeric_arg(
    engine: &Engine,
    func: &str,
    which: &str,
    raw: &str,
    strict_nonzero: bool,
) -> usize {
    let prefix = match engine.current_source.borrow().as_ref() {
        Some((file, line)) => format!("{file}:{line}: "),
        None => String::new(),
    };
    if raw.trim().is_empty() {
        eprintln!("{prefix}*** invalid {which} argument to '{func}' function: empty value.  Stop.");
        std::process::exit(2);
    }
    match raw.trim().parse::<usize>() {
        Ok(n) => {
            if strict_nonzero && n == 0 {
                eprintln!(
                    "{prefix}*** first argument to '{func}' function must be greater than 0.  Stop."
                );
                std::process::exit(2);
            }
            n
        }
        Err(_) => {
            // Disambiguate "out of range" from "not a number" the way
            // GNU make does.
            if raw.trim().chars().all(|c| c.is_ascii_digit()) {
                eprintln!(
                    "{prefix}*** invalid {which} argument to '{func}' function: '{}' out of range.  Stop.",
                    raw.trim()
                );
            } else {
                eprintln!(
                    "{prefix}*** invalid {which} argument to '{func}' function: '{}'.  Stop.",
                    raw
                );
            }
            std::process::exit(2);
        }
    }
}

/// Normalize a path by collapsing redundant separators and resolving
/// `.` and `..` segments, matching GNU make's `$(abspath)` behavior.
/// Assumes `path` is already absolute; otherwise preserves the input.
fn normalize_path(path: &str) -> String {
    if !path.starts_with('/') {
        return path.to_string();
    }
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stack.join("/"))
    }
}

pub fn pattern_stem(word: &str, pattern: &str) -> Option<String> {
    if let Some(percent_pos) = pattern.find('%') {
        let prefix = &pattern[..percent_pos];
        let suffix = &pattern[percent_pos + 1..];
        if word.starts_with(prefix)
            && word.ends_with(suffix)
            && word.len() >= prefix.len() + suffix.len()
        {
            let stem_end = word.len() - suffix.len();
            Some(word[prefix.len()..stem_end].to_string())
        } else {
            None
        }
    } else {
        None
    }
}
