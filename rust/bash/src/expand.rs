use crate::ast::*;
use std::collections::HashMap;

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

/// Represents expanded text with quoting information preserved.
#[derive(Debug, Clone)]
pub enum Segment {
    Quoted(String),
    Unquoted(String),
}

/// Expand a word into a list of strings (after word splitting and globbing).
#[allow(clippy::too_many_arguments)]
pub fn expand_word(
    word: &Word,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<String>>,
    namerefs: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    last_bg_pid: i32,
    opt_flags: &str,
    ifs: &str,
    cmd_sub: CmdSubFn,
) -> Vec<String> {
    let ctx = ExpCtx {
        vars,
        arrays,
        namerefs,
        positional,
        last_status,
        last_bg_pid,
        opt_flags,
    };
    let segments = expand_word_to_segments(word, &ctx, cmd_sub);
    let fields = word_split(&segments, ifs);
    let mut result = Vec::new();
    for field in fields {
        let globbed = glob_expand(&field);
        result.extend(globbed);
    }
    if result.is_empty() && !word.is_empty() {
        let all_quoted = word
            .iter()
            .all(|p| matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_)));
        if all_quoted {
            result.push(String::new());
        }
    }
    result
}

/// Expand a word to a single string (no word splitting or globbing).
#[allow(clippy::too_many_arguments)]
pub fn expand_word_nosplit(
    word: &Word,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<String>>,
    namerefs: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
    last_bg_pid: i32,
    opt_flags: &str,
    cmd_sub: CmdSubFn,
) -> String {
    let ctx = ExpCtx {
        vars,
        arrays,
        namerefs,
        positional,
        last_status,
        last_bg_pid,
        opt_flags,
    };
    let segments = expand_word_to_segments(word, &ctx, cmd_sub);
    segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) => t.as_str(),
        })
        .collect()
}

struct ExpCtx<'a> {
    vars: &'a HashMap<String, String>,
    arrays: &'a HashMap<String, Vec<String>>,
    namerefs: &'a HashMap<String, String>,
    positional: &'a [String],
    last_status: i32,
    last_bg_pid: i32,
    opt_flags: &'a str,
}

impl ExpCtx<'_> {
    fn resolve_nameref(&self, name: &str) -> String {
        let mut resolved = name.to_string();
        let mut seen = std::collections::HashSet::new();
        while let Some(target) = self.namerefs.get(&resolved) {
            if seen.contains(target) {
                break;
            }
            seen.insert(target.clone());
            resolved = target.clone();
        }
        resolved
    }
}

fn expand_word_to_segments(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> Vec<Segment> {
    let mut segments = Vec::new();
    for part in word {
        expand_part(part, ctx, &mut segments, cmd_sub);
    }
    segments
}

fn expand_part(part: &WordPart, ctx: &ExpCtx, out: &mut Vec<Segment>, cmd_sub: CmdSubFn) {
    match part {
        WordPart::Literal(s) => {
            out.push(Segment::Unquoted(s.clone()));
        }
        WordPart::SingleQuoted(s) => {
            out.push(Segment::Quoted(s.clone()));
        }
        WordPart::DoubleQuoted(parts) => {
            let mut s = String::new();
            for p in parts {
                match p {
                    WordPart::Literal(t) => s.push_str(t),
                    WordPart::Variable(name) => {
                        s.push_str(&lookup_var(name, ctx));
                    }
                    WordPart::Param(expr) => {
                        s.push_str(&expand_param(expr, ctx, cmd_sub));
                    }
                    WordPart::CommandSub(cmd) => {
                        s.push_str(&cmd_sub(cmd));
                    }
                    WordPart::BacktickSub(cmd) => {
                        s.push_str(&cmd_sub(cmd));
                    }
                    WordPart::ArithSub(expr) => {
                        s.push_str(&expand_arith(expr, ctx));
                    }
                    _ => {
                        let mut inner = Vec::new();
                        expand_part(p, ctx, &mut inner, cmd_sub);
                        for seg in inner {
                            match seg {
                                Segment::Quoted(t) | Segment::Unquoted(t) => s.push_str(&t),
                            }
                        }
                    }
                }
            }
            out.push(Segment::Quoted(s));
        }
        WordPart::Tilde(user) => {
            let expanded = if user.is_empty() {
                ctx.vars
                    .get("HOME")
                    .cloned()
                    .unwrap_or_else(|| "~".to_string())
            } else {
                #[cfg(unix)]
                {
                    use std::ffi::CString;
                    if let Ok(cname) = CString::new(user.as_str()) {
                        let pw = unsafe { libc::getpwnam(cname.as_ptr()) };
                        if !pw.is_null() {
                            let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
                            dir.to_string_lossy().to_string()
                        } else {
                            format!("~{}", user)
                        }
                    } else {
                        format!("~{}", user)
                    }
                }
                #[cfg(not(unix))]
                {
                    format!("~{}", user)
                }
            };
            out.push(Segment::Unquoted(expanded));
        }
        WordPart::Variable(name) => {
            let val = lookup_var(name, ctx);
            out.push(Segment::Unquoted(val));
        }
        WordPart::Param(expr) => {
            let val = expand_param(expr, ctx, cmd_sub);
            out.push(Segment::Unquoted(val));
        }
        WordPart::CommandSub(cmd) => {
            let val = cmd_sub(cmd);
            out.push(Segment::Unquoted(val));
        }
        WordPart::BacktickSub(cmd) => {
            let val = cmd_sub(cmd);
            out.push(Segment::Unquoted(val));
        }
        WordPart::ArithSub(expr) => {
            let val = expand_arith(expr, ctx);
            out.push(Segment::Unquoted(val));
        }
        WordPart::ProcessSub(kind, cmd) => {
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let (pipe_r, pipe_w) = nix::unistd::pipe().expect("pipe failed");
                let r_fd = pipe_r.as_raw_fd();
                let w_fd = pipe_w.as_raw_fd();
                // Prevent OwnedFd from closing the fds we need
                std::mem::forget(pipe_r);
                std::mem::forget(pipe_w);

                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Child) => {
                        match kind {
                            ProcessSubKind::Input => {
                                nix::unistd::close(r_fd).ok();
                                nix::unistd::dup2(w_fd, 1).ok();
                                nix::unistd::close(w_fd).ok();
                            }
                            ProcessSubKind::Output => {
                                nix::unistd::close(w_fd).ok();
                                nix::unistd::dup2(r_fd, 0).ok();
                                nix::unistd::close(r_fd).ok();
                            }
                        }
                        use std::ffi::CString;
                        let bash = CString::new("/proc/self/exe").unwrap();
                        let c_flag = CString::new("-c").unwrap();
                        let c_cmd = CString::new(cmd.as_str()).unwrap();
                        nix::unistd::execvp(&bash, &[&bash, &c_flag, &c_cmd]).ok();
                        std::process::exit(127);
                    }
                    Ok(nix::unistd::ForkResult::Parent { .. }) => {
                        let fd = match kind {
                            ProcessSubKind::Input => {
                                nix::unistd::close(w_fd).ok();
                                r_fd
                            }
                            ProcessSubKind::Output => {
                                nix::unistd::close(r_fd).ok();
                                w_fd
                            }
                        };
                        out.push(Segment::Unquoted(format!("/dev/fd/{}", fd)));
                    }
                    Err(e) => {
                        eprintln!("bash: process substitution: {}", e);
                        out.push(Segment::Unquoted(String::new()));
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (kind, cmd);
                out.push(Segment::Unquoted(String::new()));
            }
        }
    }
}

fn lookup_var(name: &str, ctx: &ExpCtx) -> String {
    match name {
        "?" => ctx.last_status.to_string(),
        "$" => std::process::id().to_string(),
        "#" => {
            let count = if ctx.positional.is_empty() {
                0
            } else {
                ctx.positional.len() - 1
            };
            count.to_string()
        }
        "0" => ctx.positional.first().cloned().unwrap_or_default(),
        "@" | "*" => {
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
            // Check for array subscript: name[idx]
            if let Some(bracket) = name.find('[') {
                let base = &name[..bracket];
                let idx_str = &name[bracket + 1..name.len() - 1];
                let resolved = ctx.resolve_nameref(base);

                return match idx_str {
                    "@" | "*" => {
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            arr.join(" ")
                        } else if let Some(val) = ctx.vars.get(&resolved) {
                            val.clone()
                        } else {
                            String::new()
                        }
                    }
                    _ => {
                        // Numeric index
                        let idx: usize = idx_str.parse().unwrap_or(0);
                        if let Some(arr) = ctx.arrays.get(&resolved) {
                            arr.get(idx).cloned().unwrap_or_default()
                        } else if idx == 0 {
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
                    ctx.arrays.get(&resolved).and_then(|a| a.first().cloned())
                })
                .or_else(|| std::env::var(&resolved).ok())
                .unwrap_or_default()
        }
    }
}

fn expand_param(expr: &ParamExpr, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    // Handle indirect expansion with operators: ${!name+word}, ${!name-word}, etc.
    if expr.name.starts_with('!') && !matches!(expr.op, ParamOp::None | ParamOp::Indirect) {
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

    let val = lookup_var(&expr.name, ctx);

    match &expr.op {
        ParamOp::None => val,
        ParamOp::Length => {
            // ${#arr[@]} — array length
            if let Some(bracket) = expr.name.find('[') {
                let base = &expr.name[..bracket];
                let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
                let resolved = ctx.resolve_nameref(base);
                if (idx_str == "@" || idx_str == "*")
                    && let Some(arr) = ctx.arrays.get(&resolved)
                {
                    return arr.len().to_string();
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
            // ${!arr[@]} or ${!arr[*]} — array indices
            let resolved = ctx.resolve_nameref(&expr.name);
            if let Some(arr) = ctx.arrays.get(&resolved) {
                let indices: Vec<String> = (0..arr.len())
                    .filter(|&i| !arr[i].is_empty() || i == 0)
                    .map(|i| i.to_string())
                    .collect();
                // TODO: when ch == '*', join with first char of IFS instead of space
                let sep = " ";
                indices.join(sep)
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
            let resolved = ctx.resolve_nameref(&expr.name);
            let unset = !ctx.vars.contains_key(&resolved) && std::env::var(&resolved).is_err();
            if unset || empty {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Assign(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let resolved = ctx.resolve_nameref(&expr.name);
            let unset = !ctx.vars.contains_key(&resolved) && std::env::var(&resolved).is_err();
            if unset || empty {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            } else {
                val
            }
        }
        ParamOp::Error(colon, word) => {
            let empty = if *colon { val.is_empty() } else { false };
            let resolved = ctx.resolve_nameref(&expr.name);
            let unset = !ctx.vars.contains_key(&resolved) && std::env::var(&resolved).is_err();
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
            let resolved = ctx.resolve_nameref(&expr.name);
            let unset = !ctx.vars.contains_key(&resolved) && std::env::var(&resolved).is_err();
            if unset || empty {
                String::new()
            } else {
                expand_word_nosplit_ctx(word, ctx, cmd_sub)
            }
        }
        ParamOp::TrimSmallLeft(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::SmallLeft)
        }
        ParamOp::TrimLargeLeft(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::LargeLeft)
        }
        ParamOp::TrimSmallRight(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::SmallRight)
        }
        ParamOp::TrimLargeRight(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(&val, &pat, TrimMode::LargeRight)
        }
        ParamOp::Replace(pattern, replacement)
        | ParamOp::ReplaceAll(pattern, replacement)
        | ParamOp::ReplacePrefix(pattern, replacement)
        | ParamOp::ReplaceSuffix(pattern, replacement) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
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
            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
            let start = if offset < 0 {
                (val.len() as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(val.len())
            };
            if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(val.len() as i64);
                let end = if len < 0 {
                    (val.len() as i64 + len).max(start as i64) as usize
                } else {
                    (start + len as usize).min(val.len())
                };
                val[start..end].to_string()
            } else {
                val[start..].to_string()
            }
        }
        ParamOp::UpperFirst(_) => {
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        }
        ParamOp::UpperAll(_) => val.to_uppercase(),
        ParamOp::LowerFirst(_) => {
            let mut chars = val.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_lowercase().to_string() + chars.as_str(),
            }
        }
        ParamOp::LowerAll(_) => val.to_lowercase(),
    }
}

fn expand_word_nosplit_ctx(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    let segments = expand_word_to_segments(word, ctx, cmd_sub);
    segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) => t.as_str(),
        })
        .collect()
}

fn expand_arith(expr: &str, ctx: &ExpCtx) -> String {
    let result = eval_arith_full(expr, ctx.vars, ctx.arrays, ctx.positional, ctx.last_status);
    result.to_string()
}

pub fn eval_arith_full(
    expr: &str,
    vars: &HashMap<String, String>,
    _arrays: &HashMap<String, Vec<String>>,
    positional: &[String],
    last_status: i32,
) -> i64 {
    let resolved = resolve_arith_vars(expr, vars, positional, last_status);
    eval_arith(&resolved).unwrap_or(0)
}

fn resolve_arith_vars(
    expr: &str,
    vars: &HashMap<String, String>,
    positional: &[String],
    last_status: i32,
) -> String {
    let mut result = String::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            i += 1;
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            let ctx_dummy = ExpCtx {
                vars,
                arrays: &HashMap::new(),
                namerefs: &HashMap::new(),
                positional,
                last_status,
                last_bg_pid: 0,
                opt_flags: "",
            };
            let val = lookup_var(&name, &ctx_dummy);
            let val = if val.is_empty() { "0".to_string() } else { val };
            result.push_str(&val);
        } else if chars[i].is_alphabetic() || chars[i] == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            // Check for assignment operators: =, +=, -=, *=, /=, %=, ++, --
            let rest: String = chars[i..].iter().collect();
            if rest.starts_with("++") {
                let val: i64 = vars.get(&name).and_then(|v| v.parse().ok()).unwrap_or(0);
                result.push_str(&val.to_string());
                // Note: can't actually modify vars here since we don't have &mut
                // The interpreter's eval_arith_expr handles this
                i += 2;
                continue;
            }
            if rest.starts_with("--") {
                let val: i64 = vars.get(&name).and_then(|v| v.parse().ok()).unwrap_or(0);
                result.push_str(&val.to_string());
                i += 2;
                continue;
            }
            let val = vars
                .get(&name)
                .cloned()
                .or_else(|| std::env::var(&name).ok())
                .unwrap_or_else(|| "0".to_string());
            // If val is not a number, try to resolve it again (for variable indirection in arith)
            let val = if val.parse::<i64>().is_err() && !val.is_empty() {
                val.parse::<i64>().map(|n| n.to_string()).unwrap_or(val)
            } else {
                val
            };
            result.push_str(&val);
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn eval_arith(expr: &str) -> Result<i64, String> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Ok(0);
    }

    // Handle comma operator (evaluate both, return right)
    if let Some(pos) = rfind_op(expr, ",") {
        let _left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(right);
    }

    // Handle ternary operator
    if let Some(q_pos) = find_balanced(expr, '?') {
        let cond = eval_arith(&expr[..q_pos])?;
        let rest = &expr[q_pos + 1..];
        if let Some(c_pos) = find_balanced(rest, ':') {
            let then_val = eval_arith(&rest[..c_pos])?;
            let else_val = eval_arith(&rest[c_pos + 1..])?;
            return Ok(if cond != 0 { then_val } else { else_val });
        }
    }

    // Handle || (logical OR)
    if let Some(pos) = rfind_op(expr, "||") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 2..])?;
        return Ok(if left != 0 || right != 0 { 1 } else { 0 });
    }

    // Handle && (logical AND)
    if let Some(pos) = rfind_op(expr, "&&") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 2..])?;
        return Ok(if left != 0 && right != 0 { 1 } else { 0 });
    }

    // Comparison operators
    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(pos) = rfind_op(expr, op) {
            let left = eval_arith(&expr[..pos])?;
            let right = eval_arith(&expr[pos + op.len()..])?;
            let result = match *op {
                "==" => left == right,
                "!=" => left != right,
                "<=" => left <= right,
                ">=" => left >= right,
                "<" => left < right,
                ">" => left > right,
                _ => false,
            };
            return Ok(if result { 1 } else { 0 });
        }
    }

    // Addition and subtraction
    {
        let mut depth = 0i32;
        let chars: Vec<char> = expr.chars().collect();
        let mut i = chars.len();
        while i > 0 {
            i -= 1;
            match chars[i] {
                ')' => depth += 1,
                '(' => depth -= 1,
                '+' | '-' if depth == 0 && i > 0 => {
                    let prev = chars[i - 1];
                    if !matches!(
                        prev,
                        '+' | '-' | '*' | '/' | '%' | '(' | '<' | '>' | '=' | '!' | '&' | '|'
                    ) {
                        let left = eval_arith(&expr[..i])?;
                        let right = eval_arith(&expr[i + 1..])?;
                        return Ok(if chars[i] == '+' {
                            left + right
                        } else {
                            left - right
                        });
                    }
                }
                _ => {}
            }
        }
    }

    // Multiplication, division, modulo
    {
        let mut depth = 0i32;
        let chars: Vec<char> = expr.chars().collect();
        let mut i = chars.len();
        while i > 0 {
            i -= 1;
            match chars[i] {
                ')' => depth += 1,
                '(' => depth -= 1,
                '*' | '/' | '%' if depth == 0 => {
                    if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
                        continue;
                    }
                    if chars[i] == '*' && i > 0 && chars[i - 1] == '*' {
                        continue;
                    }
                    let left = eval_arith(&expr[..i])?;
                    let right = eval_arith(&expr[i + 1..])?;
                    return match chars[i] {
                        '*' => Ok(left * right),
                        '/' => {
                            if right == 0 {
                                Err("division by zero".to_string())
                            } else {
                                Ok(left / right)
                            }
                        }
                        '%' => {
                            if right == 0 {
                                Err("division by zero".to_string())
                            } else {
                                Ok(left % right)
                            }
                        }
                        _ => unreachable!(),
                    };
                }
                _ => {}
            }
        }
    }

    // Exponentiation
    if let Some(pos) = find_op(expr, "**") {
        let base = eval_arith(&expr[..pos])?;
        let exp = eval_arith(&expr[pos + 2..])?;
        return Ok(base.pow(exp as u32));
    }

    // Unary operators
    if let Some(stripped) = expr.strip_prefix('-') {
        return eval_arith(stripped).map(|n| -n);
    }
    if let Some(stripped) = expr.strip_prefix('+') {
        return eval_arith(stripped);
    }
    if let Some(stripped) = expr.strip_prefix('!') {
        return eval_arith(stripped).map(|n| if n == 0 { 1 } else { 0 });
    }
    if let Some(stripped) = expr.strip_prefix('~') {
        return eval_arith(stripped).map(|n| !n);
    }

    // Parentheses
    if expr.starts_with('(') && expr.ends_with(')') {
        return eval_arith(&expr[1..expr.len() - 1]);
    }

    // Number literal
    let expr = expr.trim();
    if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|e| e.to_string())
    } else if let Some(oct) = expr.strip_prefix('0') {
        if !oct.is_empty() && oct.chars().all(|c| c.is_ascii_digit()) {
            i64::from_str_radix(oct, 8).map_err(|e| e.to_string())
        } else {
            expr.parse::<i64>().map_err(|e| e.to_string())
        }
    } else {
        expr.parse::<i64>().map_err(|e| e.to_string())
    }
}

fn find_balanced(expr: &str, target: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in expr.chars().enumerate() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c == target && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

fn find_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' {
            depth -= 1;
        } else if depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            return Some(i);
        }
    }
    None
}

fn rfind_op(expr: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    let mut result = None;
    for i in 0..bytes.len() {
        if bytes[i] == b'(' {
            depth += 1;
        } else if bytes[i] == b')' {
            depth -= 1;
        } else if depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            result = Some(i);
        }
    }
    result
}

fn word_split(segments: &[Segment], ifs: &str) -> Vec<String> {
    if segments.is_empty() {
        return vec![];
    }

    let all_quoted = segments.iter().all(|s| matches!(s, Segment::Quoted(_)));
    if all_quoted {
        let s: String = segments
            .iter()
            .map(|seg| match seg {
                Segment::Quoted(t) | Segment::Unquoted(t) => t.as_str(),
            })
            .collect();
        return vec![s];
    }

    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_field = false;

    for segment in segments {
        match segment {
            Segment::Quoted(s) => {
                current.push_str(s);
                in_field = true;
            }
            Segment::Unquoted(s) => {
                for ch in s.chars() {
                    if ifs.contains(ch) {
                        if in_field {
                            fields.push(std::mem::take(&mut current));
                            in_field = false;
                        }
                    } else {
                        current.push(ch);
                        in_field = true;
                    }
                }
            }
        }
    }

    if in_field || !current.is_empty() {
        fields.push(current);
    }

    fields
}

fn glob_expand(field: &str) -> Vec<String> {
    if field.contains('*') || field.contains('?') || field.contains('[') {
        match glob::glob(field) {
            Ok(paths) => {
                let mut results: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                if results.is_empty() {
                    vec![field.to_string()]
                } else {
                    results.sort();
                    results
                }
            }
            Err(_) => vec![field.to_string()],
        }
    } else {
        vec![field.to_string()]
    }
}

enum TrimMode {
    SmallLeft,
    LargeLeft,
    SmallRight,
    LargeRight,
}

fn trim_pattern(value: &str, pattern: &str, mode: TrimMode) -> String {
    match mode {
        TrimMode::SmallLeft => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeLeft => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[..i], pattern) {
                    return value[i..].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::SmallRight => {
            for i in (0..=value.len()).rev() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
        TrimMode::LargeRight => {
            for i in 0..=value.len() {
                if shell_pattern_match(&value[i..], pattern) {
                    return value[..i].to_string();
                }
            }
            value.to_string()
        }
    }
}

fn pattern_replace(value: &str, pattern: &str, replacement: &str, all: bool) -> String {
    if pattern.is_empty() {
        return value.to_string();
    }

    let mut result = String::new();
    let mut i = 0;
    let chars: Vec<char> = value.chars().collect();

    while i < chars.len() {
        let mut found = false;
        for j in (i + 1..=chars.len()).rev() {
            let substr: String = chars[i..j].iter().collect();
            if shell_pattern_match(&substr, pattern) {
                result.push_str(replacement);
                i = j;
                found = true;
                if !all {
                    let rest: String = chars[i..].iter().collect();
                    result.push_str(&rest);
                    return result;
                }
                break;
            }
        }
        if !found {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn shell_pattern_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    pattern_match_impl(&t, 0, &p, 0)
}

fn pattern_match_impl(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    while pi < pattern.len() {
        match pattern[pi] {
            '*' => {
                pi += 1;
                while pi < pattern.len() && pattern[pi] == '*' {
                    pi += 1;
                }
                if pi == pattern.len() {
                    return true;
                }
                for i in ti..=text.len() {
                    if pattern_match_impl(text, i, pattern, pi) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= text.len() {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            '[' => {
                if ti >= text.len() {
                    return false;
                }
                pi += 1;
                let negate = pi < pattern.len() && (pattern[pi] == '!' || pattern[pi] == '^');
                if negate {
                    pi += 1;
                }
                let mut matched = false;
                let ch = text[ti];
                while pi < pattern.len() && pattern[pi] != ']' {
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' {
                        if ch >= pattern[pi] && ch <= pattern[pi + 2] {
                            matched = true;
                        }
                        pi += 3;
                    } else {
                        if ch == pattern[pi] {
                            matched = true;
                        }
                        pi += 1;
                    }
                }
                if pi < pattern.len() {
                    pi += 1;
                }
                if matched == negate {
                    return false;
                }
                ti += 1;
            }
            '\\' => {
                pi += 1;
                if pi >= pattern.len() || ti >= text.len() {
                    return false;
                }
                if text[ti] != pattern[pi] {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
            ch => {
                if ti >= text.len() || text[ti] != ch {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
        }
    }

    ti == text.len()
}
