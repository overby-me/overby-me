use crate::ast::*;
use std::collections::HashMap;

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

/// Represents expanded text with quoting information preserved.
#[derive(Debug, Clone)]
pub enum Segment {
    /// Text from quotes — no IFS splitting, no glob expansion
    Quoted(String),
    /// Text from expansions ($var, $(cmd), etc.) — subject to IFS splitting and glob expansion
    Unquoted(String),
    /// Literal text (not from quotes or expansions) — no IFS splitting, but glob expansion applies
    Literal(String),
    /// A field separator — forces a word split here (for "$@" and "${arr[@]}")
    SplitHere,
}

/// Expand a word into a list of strings (after word splitting and globbing).
#[allow(clippy::too_many_arguments)]
pub fn expand_word(
    word: &Word,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<String>>,
    assoc_arrays: &HashMap<String, HashMap<String, String>>,
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
        assoc_arrays,
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
        let braced = brace_expand(&field);
        for b in braced {
            let globbed = glob_expand(&b);
            result.extend(globbed);
        }
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
    assoc_arrays: &HashMap<String, HashMap<String, String>>,
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
        assoc_arrays,
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
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
        })
        .collect()
}

struct ExpCtx<'a> {
    vars: &'a HashMap<String, String>,
    arrays: &'a HashMap<String, Vec<String>>,
    assoc_arrays: &'a HashMap<String, HashMap<String, String>>,
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

    /// Check whether a parameter name is "set" (exists), considering
    /// positional parameters, special variables, shell variables, and env.
    fn is_param_set(&self, name: &str) -> bool {
        // Special variables are always "set"
        match name {
            "#" | "?" | "-" | "$" | "!" | "0" | "@" | "*" => return true,
            _ => {}
        }
        // Positional parameters ($1, $2, ...)
        if let Ok(n) = name.parse::<usize>() {
            return n < self.positional.len();
        }
        // Shell variables, arrays, or environment
        let resolved = self.resolve_nameref(name);
        self.vars.contains_key(&resolved)
            || self.arrays.contains_key(&resolved)
            || self.assoc_arrays.contains_key(&resolved)
            || std::env::var(&resolved).is_ok()
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
            // Literal text: not IFS-split, but glob expansion applies
            out.push(Segment::Literal(s.clone()));
        }
        WordPart::SingleQuoted(s) => {
            out.push(Segment::Quoted(s.clone()));
        }
        WordPart::DoubleQuoted(parts) => {
            let mut s = String::new();
            for p in parts {
                match p {
                    WordPart::Literal(t) => s.push_str(t),
                    WordPart::Variable(name) if name == "@" => {
                        // "$@" — each positional parameter becomes a separate field
                        if ctx.positional.len() > 1 {
                            if !s.is_empty() {
                                out.push(Segment::Quoted(std::mem::take(&mut s)));
                            }
                            for (i, arg) in ctx.positional[1..].iter().enumerate() {
                                if i > 0 {
                                    out.push(Segment::SplitHere);
                                }
                                out.push(Segment::Quoted(arg.clone()));
                            }
                        }
                    }
                    WordPart::Variable(name) => {
                        s.push_str(&lookup_var(name, ctx));
                    }
                    WordPart::Param(expr) if is_array_at_expansion(expr, ctx) => {
                        // "${arr[@]}" — each element becomes a separate field
                        let elements = get_array_elements(expr, ctx);
                        if !s.is_empty() {
                            out.push(Segment::Quoted(std::mem::take(&mut s)));
                        }
                        for (i, elem) in elements.iter().enumerate() {
                            if i > 0 {
                                out.push(Segment::SplitHere);
                            }
                            out.push(Segment::Quoted(elem.clone()));
                        }
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
                                Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => s.push_str(&t),
                                Segment::SplitHere => {
                                    out.push(Segment::Quoted(std::mem::take(&mut s)));
                                    out.push(Segment::SplitHere);
                                }
                            }
                        }
                    }
                }
            }
            if !s.is_empty() {
                out.push(Segment::Quoted(s));
            }
        }
        WordPart::Tilde(user) => {
            let expanded = if user.is_empty() {
                ctx.vars
                    .get("HOME")
                    .cloned()
                    .unwrap_or_else(|| "~".to_string())
            } else if user == "+" {
                ctx.vars
                    .get("PWD")
                    .cloned()
                    .unwrap_or_else(|| "~+".to_string())
            } else if user == "-" {
                ctx.vars
                    .get("OLDPWD")
                    .cloned()
                    .unwrap_or_else(|| "~-".to_string())
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
                        // Export all shell variables so child process inherits them
                        for (k, v) in ctx.vars {
                            unsafe { std::env::set_var(k, v) };
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

/// Check if a ParamExpr is an array[@] expansion.
fn is_array_at_expansion(expr: &ParamExpr, ctx: &ExpCtx) -> bool {
    // Only split into separate words for plain array expansion (no operator)
    if !matches!(&expr.op, ParamOp::None) {
        return false;
    }
    if let Some(bracket) = expr.name.find('[') {
        let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
        if idx_str == "@" {
            let base = &expr.name[..bracket];
            let resolved = ctx.resolve_nameref(base);
            return ctx.arrays.contains_key(&resolved);
        }
    }
    false
}

/// Get array elements for an array[@] expansion.
fn get_array_elements(expr: &ParamExpr, ctx: &ExpCtx) -> Vec<String> {
    if let Some(bracket) = expr.name.find('[') {
        let base = &expr.name[..bracket];
        let resolved = ctx.resolve_nameref(base);
        if let Some(arr) = ctx.arrays.get(&resolved) {
            return arr.clone();
        }
        // Fall back to scalar as single element
        if let Some(val) = ctx.vars.get(&resolved) {
            return vec![val.clone()];
        }
    }
    vec![]
}

fn lookup_var(name: &str, ctx: &ExpCtx) -> String {
    match name {
        "?" => ctx.last_status.to_string(),
        "$" => std::process::id().to_string(),
        "RANDOM" => {
            // Simple pseudo-random using time and pid
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            ((t ^ (std::process::id() as u128 * 2654435761)) % 32768).to_string()
        }
        "SECONDS" => {
            // TODO: track shell start time for accurate SECONDS
            "0".to_string()
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
                            return assoc.get(idx_str).cloned().unwrap_or_default();
                        }
                        // Numeric index for indexed arrays
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
            'Q' => format!("'{}'", val.replace('\'', "'\\''")),
            'U' => val.to_uppercase(),
            'L' => val.to_lowercase(),
            'u' => {
                let mut chars = val.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().to_string() + chars.as_str(),
                }
            }
            _ => val, // @P, @A, @a, @K — return as-is for now
        },
    }
}

fn expand_word_nosplit_ctx(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    let segments = expand_word_to_segments(word, ctx, cmd_sub);
    segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
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
                assoc_arrays: &HashMap::new(),
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

    // Bitwise OR (not ||)
    if let Some(pos) = rfind_op(expr, "|")
        && pos > 0
        && expr.as_bytes()[pos - 1] != b'|'
        && (pos + 1 >= expr.len() || expr.as_bytes()[pos + 1] != b'|')
    {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left | right);
    }

    // Bitwise XOR
    if let Some(pos) = rfind_op(expr, "^") {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left ^ right);
    }

    // Bitwise AND (not &&)
    if let Some(pos) = rfind_op(expr, "&")
        && pos > 0
        && expr.as_bytes()[pos - 1] != b'&'
        && (pos + 1 >= expr.len() || expr.as_bytes()[pos + 1] != b'&')
    {
        let left = eval_arith(&expr[..pos])?;
        let right = eval_arith(&expr[pos + 1..])?;
        return Ok(left & right);
    }

    // Comparison operators (check multi-char ops first to avoid matching << >> as < >)
    for op in &["==", "!=", "<=", ">=", "<<", ">>", "<", ">"] {
        if let Some(pos) = rfind_op(expr, op) {
            let left = eval_arith(&expr[..pos])?;
            let right = eval_arith(&expr[pos + op.len()..])?;
            return match *op {
                "==" => Ok(if left == right { 1 } else { 0 }),
                "!=" => Ok(if left != right { 1 } else { 0 }),
                "<=" => Ok(if left <= right { 1 } else { 0 }),
                ">=" => Ok(if left >= right { 1 } else { 0 }),
                "<" => Ok(if left < right { 1 } else { 0 }),
                ">" => Ok(if left > right { 1 } else { 0 }),
                "<<" => Ok(left << right),
                ">>" => Ok(left >> right),
                _ => unreachable!(),
            };
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

/// Escape glob metacharacters in quoted text so they are treated literally.
/// Uses \x00 as escape prefix (cannot appear in normal shell text).
fn quote_glob_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '*' | '?' | '[' | ']') {
            out.push('\x00');
        }
        out.push(ch);
    }
    out
}

/// Remove the \x00 escape prefixes added by quote_glob_chars.
pub fn unquote_glob_chars(s: &str) -> String {
    s.replace('\x00', "")
}

/// Returns true if the string contains unescaped glob metacharacters.
fn has_glob_chars(s: &str) -> bool {
    let mut prev_null = false;
    for ch in s.chars() {
        if ch == '\x00' {
            prev_null = true;
            continue;
        }
        if matches!(ch, '*' | '?' | '[') && !prev_null {
            return true;
        }
        prev_null = false;
    }
    false
}

fn word_split(segments: &[Segment], ifs: &str) -> Vec<String> {
    if segments.is_empty() {
        return vec![];
    }

    // Check if there are any SplitHere markers or unquoted segments
    let has_split = segments.iter().any(|s| matches!(s, Segment::SplitHere));
    let all_nosplit = segments
        .iter()
        .all(|s| matches!(s, Segment::Quoted(_) | Segment::Literal(_)));
    if all_nosplit && !has_split {
        let s: String = segments
            .iter()
            .map(|seg| match seg {
                Segment::Quoted(t) => quote_glob_chars(t),
                Segment::Literal(t) => t.clone(),
                Segment::Unquoted(t) => t.clone(),
                Segment::SplitHere => String::new(),
            })
            .collect();
        return vec![s];
    }

    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_field = false;

    for segment in segments {
        match segment {
            Segment::SplitHere => {
                // Force a field break here (for "$@" and "${arr[@]}")
                fields.push(std::mem::take(&mut current));
                in_field = false;
            }
            Segment::Quoted(s) => {
                current.push_str(&quote_glob_chars(s));
                in_field = true;
            }
            Segment::Literal(s) => {
                // Literal text: not IFS-split, glob chars preserved
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

/// Brace expansion: {a,b,c} → ["a", "b", "c"], pre{a,b}post → ["preapost", "prebpost"]
/// Also handles sequences: {1..5} → ["1", "2", "3", "4", "5"]
fn brace_expand(s: &str) -> Vec<String> {
    // Find the first unquoted { with matching }
    let chars: Vec<char> = s.chars().collect();
    let mut depth = 0;
    let mut brace_start = None;
    let mut has_comma = false;
    let mut has_dotdot = false;

    for (i, &ch) in chars.iter().enumerate() {
        match ch {
            '{' if depth == 0 => {
                brace_start = Some(i);
                depth = 1;
            }
            '{' => depth += 1,
            '}' if depth == 1 => {
                if let Some(start) = brace_start {
                    let inner = &s[start + 1..i];
                    if inner.contains(',') {
                        has_comma = true;
                    }
                    if inner.contains("..") {
                        has_dotdot = true;
                    }
                    if has_comma || has_dotdot {
                        let prefix = &s[..start];
                        let suffix = &s[i + 1..];

                        if has_comma {
                            // Split on commas (respecting nested braces)
                            let alternatives = split_brace_alternatives(inner);
                            let mut result = Vec::new();
                            for alt in &alternatives {
                                let expanded =
                                    brace_expand(&format!("{}{}{}", prefix, alt, suffix));
                                result.extend(expanded);
                            }
                            return result;
                        } else if has_dotdot {
                            // Sequence: {start..end} or {start..end..step}
                            let parts: Vec<&str> = inner.split("..").collect();
                            if parts.len() >= 2 {
                                let mut result = Vec::new();
                                if let (Ok(start_n), Ok(end_n)) =
                                    (parts[0].parse::<i64>(), parts[1].parse::<i64>())
                                {
                                    let step: i64 = if parts.len() >= 3 {
                                        parts[2].parse().unwrap_or(1)
                                    } else {
                                        1
                                    };
                                    let step = if step == 0 { 1 } else { step.abs() };
                                    if start_n <= end_n {
                                        let mut n = start_n;
                                        while n <= end_n {
                                            result.extend(brace_expand(&format!(
                                                "{}{}{}",
                                                prefix, n, suffix
                                            )));
                                            n += step;
                                        }
                                    } else {
                                        let mut n = start_n;
                                        while n >= end_n {
                                            result.extend(brace_expand(&format!(
                                                "{}{}{}",
                                                prefix, n, suffix
                                            )));
                                            n -= step;
                                        }
                                    }
                                } else if parts[0].len() == 1 && parts[1].len() == 1 {
                                    // Character range: {a..z}
                                    let start_c = parts[0].chars().next().unwrap();
                                    let end_c = parts[1].chars().next().unwrap();
                                    if start_c <= end_c {
                                        for c in start_c..=end_c {
                                            result.extend(brace_expand(&format!(
                                                "{}{}{}",
                                                prefix, c, suffix
                                            )));
                                        }
                                    } else {
                                        let mut c = start_c;
                                        while c >= end_c {
                                            result.extend(brace_expand(&format!(
                                                "{}{}{}",
                                                prefix, c, suffix
                                            )));
                                            if c == '\0' {
                                                break;
                                            }
                                            c = (c as u8 - 1) as char;
                                        }
                                    }
                                }
                                if !result.is_empty() {
                                    return result;
                                }
                            }
                        }
                    }
                }
                depth = 0;
                brace_start = None;
                has_comma = false;
                has_dotdot = false;
            }
            '}' => depth -= 1,
            ',' if depth == 1 => has_comma = true,
            _ => {}
        }
    }

    vec![s.to_string()]
}

fn split_brace_alternatives(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                result.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    result.push(current);
    result
}

fn glob_expand(field: &str) -> Vec<String> {
    // Only glob if there are unescaped (unquoted) glob metacharacters
    if has_glob_chars(field) {
        // Strip the \x00 escape markers before globbing — the quoted chars
        // have already been accounted for by not reaching here if all were quoted.
        // We need to build a glob pattern where quoted metacharacters are escaped
        // with [] bracket quoting so the glob crate treats them literally.
        let mut pattern = String::with_capacity(field.len());
        let mut chars = field.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x00' {
                // Next char is a quoted glob char — escape it for glob
                if let Some(next) = chars.next() {
                    pattern.push('[');
                    pattern.push(next);
                    pattern.push(']');
                }
            } else {
                pattern.push(ch);
            }
        }
        match glob::glob(&pattern) {
            Ok(paths) => {
                let mut results: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                if results.is_empty() {
                    vec![unquote_glob_chars(field)]
                } else {
                    results.sort();
                    results
                }
            }
            Err(_) => vec![unquote_glob_chars(field)],
        }
    } else {
        vec![unquote_glob_chars(field)]
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
