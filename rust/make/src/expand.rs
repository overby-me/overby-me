//! Variable and function expansion for Make expressions.

use crate::engine::Engine;
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
                c if c.is_alphanumeric() || c == '_' => {
                    // Single character variable
                    let var = c.to_string();
                    i += 1;
                    if let Some(val) = auto_vars.get(var.as_str()) {
                        result.push_str(val);
                    } else {
                        result.push_str(&engine.lookup_var(&var));
                    }
                }
                _ => {
                    result.push('$');
                    // Don't advance - let the outer loop handle this char
                }
            }
        } else if chars[i] == '#' {
            // Comment in expansion context - stop
            break;
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
                engine.lookup_var(varname)
            };
            return substitute_ref(&val, from, to);
        }
    }

    // Check for function call: $(func args)
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
        engine.lookup_var(varname)
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
                let n: usize = expand_with_auto(&args[0], engine, auto_vars)
                    .trim()
                    .parse()
                    .unwrap_or(0);
                let text = expand_with_auto(&args[1], engine, auto_vars);
                let words: Vec<&str> = text.split_whitespace().collect();
                Some(words.get(n.wrapping_sub(1)).unwrap_or(&"").to_string())
            } else {
                Some(String::new())
            }
        }
        "wordlist" => {
            if args.len() >= 3 {
                let s: usize = expand_with_auto(&args[0], engine, auto_vars)
                    .trim()
                    .parse()
                    .unwrap_or(0);
                let e: usize = expand_with_auto(&args[1], engine, auto_vars)
                    .trim()
                    .parse()
                    .unwrap_or(0);
                let text = expand_with_auto(&args[2], engine, auto_vars);
                let words: Vec<&str> = text.split_whitespace().collect();
                let start = s.saturating_sub(1).min(words.len());
                let end = e.min(words.len());
                Some(words[start..end].join(" "))
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
                    let p = std::path::Path::new(name);
                    if p.is_absolute() {
                        name.to_string()
                    } else {
                        cwd.join(name).to_string_lossy().to_string()
                    }
                })
                .collect();
            Some(result.join(" "))
        }
        "shell" => {
            let cmd = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            let shell = engine.lookup_var_or("SHELL", "/bin/sh");
            match std::process::Command::new(&shell)
                .arg("-c")
                .arg(&cmd)
                .output()
            {
                Ok(output) => {
                    let s = String::from_utf8_lossy(&output.stdout)
                        .replace('\n', " ")
                        .trim_end()
                        .to_string();
                    Some(s)
                }
                Err(_) => Some(String::new()),
            }
        }
        "if" => {
            if args.len() >= 2 {
                let cond = expand_with_auto(&args[0], engine, auto_vars);
                if !cond.trim().is_empty() {
                    Some(expand_with_auto(&args[1], engine, auto_vars))
                } else if args.len() >= 3 {
                    Some(expand_with_auto(&args[2], engine, auto_vars))
                } else {
                    Some(String::new())
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
            if args.len() >= 3 {
                let var = args[0].trim();
                let list = expand_with_auto(&args[1], engine, auto_vars);
                let body = &args[2];
                let result: Vec<String> = list
                    .split_whitespace()
                    .map(|word| {
                        let mut inner_auto = auto_vars.clone();
                        let word_owned = word.to_string();
                        // Temporarily set the variable
                        // We use a trick: expand body with var set
                        let body_replaced = body
                            .replace(&format!("$({var})"), &word_owned)
                            .replace(&format!("${{{var}}}"), &word_owned);
                        // Also handle $X for single-char vars
                        let body_replaced = if var.len() == 1 {
                            body_replaced.replace(&format!("${var}"), &word_owned)
                        } else {
                            body_replaced
                        };
                        inner_auto.insert(var, word_owned);
                        expand_with_auto(&body_replaced, engine, &inner_auto)
                    })
                    .collect();
                Some(result.join(" "))
            } else {
                Some(String::new())
            }
        }
        "call" => {
            if !args.is_empty() {
                let func_name = expand_with_auto(&args[0], engine, auto_vars);
                let func_body = engine.lookup_var(func_name.trim());
                let mut body = func_body;
                for (i, arg) in args.iter().skip(1).enumerate() {
                    let val = expand_with_auto(arg, engine, auto_vars);
                    body = body.replace(&format!("$({})", i + 1), &val);
                    body = body.replace(&format!("${{{}}}", i + 1), &val);
                }
                body = body.replace("$(0)", func_name.trim());
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
            eprintln!("*** {msg}.  Stop.");
            std::process::exit(2);
        }
        "warning" => {
            let msg = expand_with_auto(
                args.first().map(|s| s.as_str()).unwrap_or(""),
                engine,
                auto_vars,
            );
            eprintln!("warning: {msg}");
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
    if let Some(percent_pos) = pattern.find('%') {
        let prefix = &pattern[..percent_pos];
        let suffix = &pattern[percent_pos + 1..];
        word.starts_with(prefix)
            && word.ends_with(suffix)
            && word.len() >= prefix.len() + suffix.len()
    } else {
        word == pattern
    }
}

/// Extract the stem from a pattern match.
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
