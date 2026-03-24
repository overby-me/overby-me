use crate::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

thread_local! {
    /// File descriptors opened by process substitutions that need to be closed
    /// after the command using them completes.
    static PROCSUB_FDS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Flag set when arithmetic evaluation encounters an error.
    static ARITH_ERROR: RefCell<bool> = const { RefCell::new(false) };
}

/// Check and clear the arithmetic error flag.
pub fn take_arith_error() -> bool {
    ARITH_ERROR.with(|f| std::mem::replace(&mut *f.borrow_mut(), false))
}

/// Take all pending process substitution fds (draining the list).
pub fn take_procsub_fds() -> Vec<i32> {
    PROCSUB_FDS.with(|fds| std::mem::take(&mut *fds.borrow_mut()))
}

/// Register a process substitution fd for later cleanup.
pub fn register_procsub_fd_pub(fd: i32) {
    register_procsub_fd(fd);
}

fn register_procsub_fd(fd: i32) {
    PROCSUB_FDS.with(|fds| fds.borrow_mut().push(fd));
}

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
    assoc_arrays: &HashMap<String, crate::interpreter::AssocArray>,
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
    // Pre-expansion brace expansion: expand braces at the word level before
    // variable/command expansion, matching bash's expansion order
    let words_to_expand = word_level_brace_expand(word);

    let mut result = Vec::new();
    for w in &words_to_expand {
        let segments = expand_word_to_segments(w, &ctx, cmd_sub);
        let fields = word_split(&segments, ifs);
        for field in fields {
            let braced = brace_expand(&field);
            for b in braced {
                let globbed = glob_expand(&b);
                result.extend(globbed);
            }
        }
    }
    if result.is_empty() && !word.is_empty() {
        // Check if word contains "$@" or "${arr[@]}" which expand to nothing with 0 elements
        let has_at_expansion = word.iter().any(|p| {
            if let WordPart::DoubleQuoted(parts) = p {
                parts
                    .iter()
                    .any(|inner| matches!(inner, WordPart::Variable(n) if n == "@"))
            } else {
                false
            }
        });
        if !has_at_expansion {
            let all_quoted = word
                .iter()
                .all(|p| matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_)));
            if all_quoted {
                result.push(String::new());
            }
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
    assoc_arrays: &HashMap<String, crate::interpreter::AssocArray>,
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

/// Expand a word for use as a pattern (case, [[=]], etc.).
/// Quoted portions have glob chars escaped; unquoted/literal portions preserve them.
#[allow(clippy::too_many_arguments)]
pub fn expand_word_pattern(
    word: &Word,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<String>>,
    assoc_arrays: &HashMap<String, crate::interpreter::AssocArray>,
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
            Segment::Quoted(t) => quote_glob_chars(t),
            Segment::Unquoted(t) | Segment::Literal(t) => t.clone(),
            Segment::SplitHere => " ".to_string(),
        })
        .collect()
}

struct ExpCtx<'a> {
    vars: &'a HashMap<String, String>,
    arrays: &'a HashMap<String, Vec<String>>,
    assoc_arrays: &'a HashMap<String, crate::interpreter::AssocArray>,
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
                        let mut elements = get_array_elements(expr, ctx);
                        // For Substring on arrays (but not @/*): slice the array
                        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
                            // @/* slicing is already handled in get_array_elements
                            if expr.name != "@" && expr.name != "*" {
                                let offset: i64 = offset_str.trim().parse().unwrap_or(0);
                                let count = elements.len();
                                let start = if offset < 0 {
                                    (count as i64 + offset).max(0) as usize
                                } else {
                                    (offset as usize).min(count)
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
                                elements = elements[start..end].to_vec();
                            }
                        }
                        if !s.is_empty() {
                            out.push(Segment::Quoted(std::mem::take(&mut s)));
                        }
                        for (i, elem) in elements.iter().enumerate() {
                            if i > 0 {
                                out.push(Segment::SplitHere);
                            }
                            // Apply param operation (^, ^^, ,, etc.) to each element
                            // (but not Substring, which was already handled as array slice)
                            let modified = if matches!(&expr.op, ParamOp::Substring(..)) {
                                elem.clone()
                            } else {
                                apply_param_op(elem, &expr.op, ctx, cmd_sub)
                            };
                            out.push(Segment::Quoted(modified));
                        }
                    }
                    WordPart::Param(expr) => {
                        s.push_str(&expand_param(expr, ctx, cmd_sub));
                    }
                    WordPart::CommandSub(cmd) => {
                        let trimmed = cmd.trim();
                        if let Some(file) = trimmed.strip_prefix("< ").or_else(|| trimmed.strip_prefix("<\t")) {
                            let file = file.trim();
                            if let Ok(content) = std::fs::read_to_string(file) {
                                s.push_str(content.trim_end_matches('\n'));
                            }
                        } else {
                            s.push_str(&cmd_sub(cmd));
                        }
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
                                Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => {
                                    s.push_str(&t)
                                }
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
            // Check if this is an array[@] with an operator that should apply per-element
            if let Some(bracket) = expr.name.find('[') {
                let base = &expr.name[..bracket];
                let idx = &expr.name[bracket + 1..expr.name.len().saturating_sub(1)];
                let resolved = ctx.resolve_nameref(base);
                if (idx == "@" || idx == "*")
                    && !matches!(expr.op, ParamOp::None | ParamOp::Length)
                    && let Some(arr) = ctx.arrays.get(&resolved)
                {
                    // Apply operation to each element separately
                    let mut first = true;
                    for elem in arr {
                        if !first {
                            out.push(if idx == "@" {
                                Segment::SplitHere
                            } else {
                                let ifs_char = ctx
                                    .vars
                                    .get("IFS")
                                    .and_then(|s| s.chars().next())
                                    .unwrap_or(' ');
                                Segment::Unquoted(ifs_char.to_string())
                            });
                        }
                        first = false;
                        // Create a temporary param expr for this single element
                        let single_expr = ParamExpr {
                            name: elem.clone(),
                            op: expr.op.clone(),
                        };
                        // Apply the operation using the element value directly
                        let result = apply_param_op(elem, &single_expr.op, ctx, cmd_sub);
                        out.push(Segment::Unquoted(result));
                    }
                    return;
                }
            }
            // Save original variable value before expansion (needed for quoting check)
            let orig_val = lookup_var(&expr.name, ctx);
            let orig_set = ctx.is_param_set(&expr.name);
            let mut val = expand_param(expr, ctx, cmd_sub);
            // Apply tilde expansion for default/assign values
            if matches!(
                &expr.op,
                ParamOp::Default(..) | ParamOp::Assign(..) | ParamOp::Alt(..)
            ) && val.starts_with('~')
                && (val.len() == 1 || val.as_bytes().get(1) == Some(&b'/'))
            {
                let home = ctx.vars.get("HOME").cloned().unwrap_or_default();
                val = format!("{}{}", home, &val[1..]);
            }
            // Check if the parameter expansion used a default/alt word with
            // quoted content — if so, the result should be treated as quoted
            // to prevent field splitting (e.g., ${B:-"$A"} preserves quotes)
            let has_quoted_word = match &expr.op {
                ParamOp::Default(_, word) | ParamOp::Alt(_, word) => {
                    let is_active = match &expr.op {
                        ParamOp::Default(colon, _) => {
                            let empty = if *colon { orig_val.is_empty() } else { false };
                            !orig_set || empty
                        }
                        ParamOp::Alt(colon, _) => {
                            let empty = if *colon { orig_val.is_empty() } else { false };
                            !(!orig_set || empty)
                        }
                        _ => false,
                    };
                    is_active
                        && word
                            .iter()
                            .any(|p| matches!(p, WordPart::DoubleQuoted(_) | WordPart::SingleQuoted(_)))
                }
                _ => false,
            };
            if has_quoted_word {
                out.push(Segment::Quoted(val));
            } else {
                out.push(Segment::Unquoted(val));
            }
        }
        WordPart::CommandSub(cmd) => {
            // Optimize $(< file) — read file content directly
            let trimmed = cmd.trim();
            let val = if let Some(file) = trimmed.strip_prefix("< ").or_else(|| trimmed.strip_prefix("<\t")) {
                let file = file.trim();
                // Expand the filename
                let expanded = expand_word_nosplit_ctx(
                    &vec![WordPart::Literal(file.to_string())],
                    ctx,
                    &mut |c| cmd_sub(c),
                );
                match std::fs::read_to_string(expanded.trim()) {
                    Ok(content) => {
                        // Strip trailing newlines (like command substitution)
                        content.trim_end_matches('\n').to_string()
                    }
                    Err(_) => String::new(),
                }
            } else {
                cmd_sub(cmd)
            };
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
                                register_procsub_fd(r_fd);
                                r_fd
                            }
                            ProcessSubKind::Output => {
                                nix::unistd::close(r_fd).ok();
                                register_procsub_fd(w_fd);
                                w_fd
                            }
                        };
                        out.push(Segment::Unquoted(format!("/proc/self/fd/{}", fd)));
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
            // Only for None (plain expansion) and Substring (array slice)
            if matches!(&expr.op, ParamOp::None | ParamOp::Substring(..)) {
                return ctx.arrays.contains_key(&resolved)
                    || ctx.assoc_arrays.contains_key(&resolved);
            }
        }
    }
    false
}

/// Get array elements for an array[@] expansion.
fn get_array_elements(expr: &ParamExpr, ctx: &ExpCtx) -> Vec<String> {
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
                .filter(|&i| !arr[i].is_empty() || i == 0)
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
            return arr.clone();
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
        "BASH_SUBSHELL" => ctx.vars.get("BASH_SUBSHELL").cloned().unwrap_or_else(|| "0".to_string()),
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

/// Apply a parameter operation to a pre-resolved value (for array per-element operations)
fn apply_param_op(val: &str, op: &ParamOp, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
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
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::SmallLeft)
        }
        ParamOp::TrimLargeLeft(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::LargeLeft)
        }
        ParamOp::TrimSmallRight(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::SmallRight)
        }
        ParamOp::TrimLargeRight(pattern) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
            trim_pattern(val, &pat, TrimMode::LargeRight)
        }
        ParamOp::Replace(pattern, replacement)
        | ParamOp::ReplaceAll(pattern, replacement)
        | ParamOp::ReplacePrefix(pattern, replacement)
        | ParamOp::ReplaceSuffix(pattern, replacement) => {
            let pat = expand_word_nosplit_ctx(pattern, ctx, cmd_sub);
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
            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(char_count as i64);
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
            .map(|elem| apply_param_op(elem, &expr.op, ctx, cmd_sub))
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
                arr.clone()
            } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                assoc.values().cloned().collect()
            } else if let Some(val) = ctx.vars.get(&resolved) {
                vec![val.clone()]
            } else {
                vec![]
            };
            let modified: Vec<String> = elements
                .iter()
                .map(|elem| apply_param_op(elem, &expr.op, ctx, cmd_sub))
                .collect();
            return modified.join(" ");
        }
    }

    let val = lookup_var(&expr.name, ctx);

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
                        return arr.len().to_string();
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
                    .filter(|&i| !arr[i].is_empty() || i == 0)
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
            let char_count = val.chars().count();
            let start = if offset < 0 {
                (char_count as i64 + offset).max(0) as usize
            } else {
                (offset as usize).min(char_count)
            };
            if let Some(len_str) = length_str {
                let len: i64 = len_str.trim().parse().unwrap_or(char_count as i64);
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
    match eval_arith(&resolved) {
        Ok(val) => val,
        Err(e) => {
            let name = vars
                .get("_BASH_SOURCE_FILE")
                .or_else(|| positional.first())
                .map(|s| s.as_str())
                .unwrap_or("bash");
            let lineno = vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
            eprintln!(
                "{}: line {}: ((: {} : {} (error token is \"{}\")",
                name,
                lineno,
                expr.trim(),
                e,
                find_error_token(expr, &e)
            );
            ARITH_ERROR.with(|f| *f.borrow_mut() = true);
            0
        }
    }
}

fn find_error_token(expr: &str, _error: &str) -> String {
    // Simple heuristic: return the part after the operator that caused the error
    let trimmed = expr.trim();
    if let Some(pos) = trimmed.find("/ 0") {
        format!("0 {}", &trimmed[pos + 3..].trim_start())
    } else {
        trimmed.to_string()
    }
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
                    // Skip if this is part of ++ or -- (check next char)
                    let next = if i + 1 < chars.len() { chars[i + 1] } else { ' ' };
                    if !matches!(
                        prev,
                        '+' | '-' | '*' | '/' | '%' | '(' | '<' | '>' | '=' | '!' | '&' | '|'
                    ) && !(next == chars[i]) {
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
                                Err("division by 0".to_string())
                            } else {
                                Ok(left / right)
                            }
                        }
                        '%' => {
                            if right == 0 {
                                Err("division by 0".to_string())
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
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | ',' | '\\') {
            out.push('\x00');
        }
        out.push(ch);
    }
    out
}

/// Remove the \x00 escape prefixes added by quote_glob_chars.
#[allow(dead_code)]
pub fn unquote_glob_chars(s: &str) -> String {
    s.replace('\x00', "")
}

/// Remove both \x00 quote markers and backslash quoting (for word expansion, NOT case patterns)
fn remove_quotes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\x00' {
            // Quote marker — skip it, keep the next char
            if let Some(next) = chars.next() {
                result.push(next);
            }
        } else if ch == '\\' {
            // Backslash quote removal: keep the next char, discard backslash
            if let Some(next) = chars.next() {
                result.push(next);
            }
        } else {
            result.push(ch);
        }
    }
    result
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

    for segment in segments {
        match segment {
            Segment::SplitHere => {
                // Force a field break here (for "$@" and "${arr[@]}")
                fields.push(std::mem::take(&mut current));
            }
            Segment::Quoted(s) => {
                current.push_str(&quote_glob_chars(s));
            }
            Segment::Literal(s) => {
                // Literal text: not IFS-split, glob chars preserved
                current.push_str(s);
            }
            Segment::Unquoted(s) => {
                let ifs_ws: Vec<char> = ifs.chars().filter(|c| c.is_whitespace()).collect();
                let ifs_non_ws: Vec<char> = ifs.chars().filter(|c| !c.is_whitespace()).collect();
                // State machine for IFS splitting:
                // 0 = start/after ws delim, 1 = in field, 2 = after non-ws delim
                let mut state = 0u8;
                for ch in s.chars() {
                    if ifs_non_ws.contains(&ch) {
                        match state {
                            1 => {
                                // End current field
                                fields.push(std::mem::take(&mut current));
                            }
                            2 => {
                                // Consecutive non-ws delim: push empty field between them
                                fields.push(String::new());
                            }
                            _ => {
                                // State 0: ws already consumed a delimiter
                                // If no fields yet, this is a leading non-ws → push empty
                                // If fields exist, ws+nonws is a single delimiter → absorb
                                if fields.is_empty() {
                                    fields.push(String::new());
                                }
                            }
                        }
                        state = 2;
                    } else if ifs_ws.contains(&ch) {
                        if state == 1 {
                            // End current field on whitespace
                            fields.push(std::mem::take(&mut current));
                            state = 0;
                        }
                        // In states 0 or 2, whitespace is consumed
                    } else {
                        current.push(ch);
                        state = 1;
                    }
                }
            }
        }
    }

    // Push the last field. For "$@" with empty trailing elements, we must
    // keep empty fields — but only if there was actual quoted content after
    // the last SplitHere (even if that content was empty).
    let had_quoted_after_split = segments
        .iter()
        .rev()
        .take_while(|s| !matches!(s, Segment::SplitHere))
        .any(|s| matches!(s, Segment::Quoted(_)));
    if !current.is_empty() || (has_split && had_quoted_after_split) {
        fields.push(current);
    }

    fields
}

/// Check if a character matches a glob-style pattern (used for case modification).
/// Supports `?` (any char), `[abc]` (character class), and literal characters.
fn char_matches_pattern(c: char, pattern: &str) -> bool {
    if pattern == "?" {
        return true;
    }
    if pattern.starts_with('[') && pattern.ends_with(']') {
        let inner = &pattern[1..pattern.len() - 1];
        let (negate, inner) = if inner.starts_with('!') || inner.starts_with('^') {
            (true, &inner[1..])
        } else {
            (false, inner)
        };
        let mut found = false;
        let mut chars = inner.chars().peekable();
        while let Some(ch) = chars.next() {
            if chars.peek() == Some(&'-') {
                chars.next(); // consume '-'
                if let Some(end) = chars.next()
                    && c >= ch
                    && c <= end
                {
                    found = true;
                }
            } else if ch == c {
                found = true;
            }
        }
        return if negate { !found } else { found };
    }
    // Literal pattern: match case-insensitively (pattern char matches this char)
    pattern
        .chars()
        .any(|p| p == c || p.to_lowercase().eq(c.to_lowercase()))
}

/// Brace expansion: {a,b,c} → ["a", "b", "c"], pre{a,b}post → ["preapost", "prebpost"]
/// Also handles sequences: {1..5} → ["1", "2", "3", "4", "5"]
/// Pre-expansion brace expansion at the Word level.
/// Converts the word to raw text, checks for unquoted brace patterns,
/// brace-expands, and re-lexes each result into Words.
fn word_level_brace_expand(word: &Word) -> Vec<Word> {
    // Convert word to raw text, tracking whether brace expansion
    // should happen at the word level (before variable expansion).
    // This is only needed when a brace pattern is split across word parts,
    // e.g., $var{x,y} where $var is a Variable and {x,y} is Literal.
    let mut raw = String::new();
    let mut has_brace_in_literal = false;

    // Check specifically for Variable followed by Literal starting with {
    for i in 0..word.len().saturating_sub(1) {
        if matches!(&word[i], WordPart::Variable(_))
            && matches!(&word[i + 1], WordPart::Literal(s) if s.starts_with('{'))
        {
            has_brace_in_literal = true;
            break;
        }
    }

    for part in word {
        match part {
            WordPart::Literal(s) => {
                raw.push_str(s);
            }
            WordPart::SingleQuoted(s) => {
                // Single-quoted text: braces are not special
                raw.push('\'');
                raw.push_str(s);
                raw.push('\'');
            }
            WordPart::DoubleQuoted(parts) => {
                raw.push('"');
                for p in parts {
                    match p {
                        WordPart::Literal(s) => raw.push_str(s),
                        WordPart::Variable(name) => {
                            raw.push('$');
                            raw.push_str(name);
                        }
                        WordPart::Param(expr) => {
                            // Reconstruct ${...} — simplified
                            raw.push_str("${");
                            raw.push_str(&expr.name);
                            raw.push('}');
                        }
                        WordPart::CommandSub(cmd) => {
                            raw.push_str("$(");
                            raw.push_str(cmd);
                            raw.push(')');
                        }
                        _ => {}
                    }
                }
                raw.push('"');
            }
            WordPart::Variable(name) => {
                raw.push('$');
                raw.push_str(name);
            }
            WordPart::Param(expr) => {
                raw.push_str("${");
                raw.push_str(&expr.name);
                raw.push('}');
            }
            WordPart::Tilde(user) => {
                raw.push('~');
                raw.push_str(user);
            }
            WordPart::CommandSub(cmd) => {
                raw.push_str("$(");
                raw.push_str(cmd);
                raw.push(')');
            }
            WordPart::BacktickSub(cmd) => {
                raw.push('`');
                raw.push_str(cmd);
                raw.push('`');
            }
            WordPart::ArithSub(expr) => {
                raw.push_str("$((");
                raw.push_str(expr);
                raw.push_str("))");
            }
            _ => {}
        }
    }

    // Check if brace expansion would produce multiple results
    if !has_brace_in_literal {
        return vec![word.clone()];
    }

    let expanded = brace_expand(&raw);
    if expanded.len() <= 1 {
        return vec![word.clone()];
    }

    // Re-lex each expanded string into a Word
    expanded
        .into_iter()
        .map(|s| {
            let chars: Vec<char> = s.chars().collect();
            let mut i = 0;
            let mut parts = Vec::new();
            let mut literal = String::new();

            while i < chars.len() {
                match chars[i] {
                    '$' => {
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        i += 1;
                        parts.push(crate::lexer::parse_dollar(&chars, &mut i, false));
                    }
                    '\'' => {
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        i += 1;
                        let mut sq = String::new();
                        while i < chars.len() && chars[i] != '\'' {
                            sq.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1;
                        }
                        parts.push(WordPart::SingleQuoted(sq));
                    }
                    '"' => {
                        if !literal.is_empty() {
                            parts.push(WordPart::Literal(std::mem::take(&mut literal)));
                        }
                        i += 1;
                        let mut dq_parts = Vec::new();
                        let mut dq_lit = String::new();
                        while i < chars.len() && chars[i] != '"' {
                            if chars[i] == '$' {
                                if !dq_lit.is_empty() {
                                    dq_parts
                                        .push(WordPart::Literal(std::mem::take(&mut dq_lit)));
                                }
                                i += 1;
                                dq_parts.push(crate::lexer::parse_dollar(&chars, &mut i, true));
                            } else {
                                dq_lit.push(chars[i]);
                                i += 1;
                            }
                        }
                        if i < chars.len() {
                            i += 1;
                        }
                        if !dq_lit.is_empty() {
                            dq_parts.push(WordPart::Literal(dq_lit));
                        }
                        parts.push(WordPart::DoubleQuoted(dq_parts));
                    }
                    '~' if i == 0 => {
                        let mut user = String::new();
                        i += 1;
                        while i < chars.len()
                            && chars[i] != '/'
                            && !chars[i].is_whitespace()
                        {
                            user.push(chars[i]);
                            i += 1;
                        }
                        parts.push(WordPart::Tilde(user));
                    }
                    c => {
                        literal.push(c);
                        i += 1;
                    }
                }
            }
            if !literal.is_empty() {
                parts.push(WordPart::Literal(literal));
            }
            parts
        })
        .collect()
}

fn brace_expand(s: &str) -> Vec<String> {
    // Bash algorithm: find the first '{', then its matching '}' (tracking depth),
    // check if the content has commas or '..' at depth 0.
    let chars: Vec<char> = s.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if (chars[i] == '\\' || chars[i] == '\x00') && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if chars[i] == '{' {
            let start = i;
            // Find matching '}' tracking depth
            let mut depth = 1;
            let mut has_comma = false;
            let mut has_dotdot = false;
            let mut j = i + 1;
            let mut prev_was_dot = false;
            while j < chars.len() && depth > 0 {
                if (chars[j] == '\\' || chars[j] == '\x00') && j + 1 < chars.len() {
                    prev_was_dot = false;
                    j += 2;
                    continue;
                }
                match chars[j] {
                    '{' => {
                        depth += 1;
                        prev_was_dot = false;
                    }
                    '}' => {
                        depth -= 1;
                        prev_was_dot = false;
                    }
                    ',' if depth == 1 => {
                        has_comma = true;
                        prev_was_dot = false;
                    }
                    '.' if depth == 1 && prev_was_dot => {
                        has_dotdot = true;
                    }
                    '.' if depth == 1 => {
                        prev_was_dot = true;
                        j += 1;
                        continue;
                    }
                    _ => {
                        prev_was_dot = false;
                    }
                }
                j += 1;
            }
            if depth == 0 && (has_comma || has_dotdot) {
                let end = j - 1; // position of matching '}'
                let inner = &s[start + 1..end];
                let prefix = &s[..start];
                let suffix = &s[end + 1..];

                if has_comma {
                    // Split on commas (respecting nested braces)
                    let alternatives = split_brace_alternatives(inner);
                    // Expand braces within each alternative and in the suffix separately
                    let suffix_expanded = brace_expand(suffix);
                    let mut result = Vec::new();
                    for alt in &alternatives {
                        let alt_expanded = brace_expand(alt);
                        for ae in &alt_expanded {
                            for se in &suffix_expanded {
                                result.push(format!("{}{}{}", prefix, ae, se));
                            }
                        }
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
                            // If step is present but not a valid integer, don't expand
                            let step: i64 = if parts.len() >= 3 {
                                match parts[2].parse::<i64>() {
                                    Ok(v) => v,
                                    Err(_) => {
                                        // Invalid step — return as literal
                                        return vec![s.to_string()];
                                    }
                                }
                            } else {
                                1
                            };
                            let step = if step == 0 { 1 } else { step.abs() };
                            // Zero-padding: pad to the widest operand width
                            let width = std::cmp::max(parts[0].len(), parts[1].len());
                            let needs_pad = parts[0].starts_with('0') && parts[0].len() > 1
                                || parts[1].starts_with('0') && parts[1].len() > 1
                                || parts[0].starts_with("-0") && parts[0].len() > 2
                                || parts[1].starts_with("-0") && parts[1].len() > 2;
                            if start_n <= end_n {
                                let mut n = start_n;
                                while n <= end_n {
                                    let num_str = if needs_pad {
                                        if n < 0 {
                                            format!("-{:0>w$}", -n, w = width - 1)
                                        } else {
                                            format!("{:0>w$}", n, w = width)
                                        }
                                    } else {
                                        n.to_string()
                                    };
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, num_str, suffix
                                    )));
                                    n += step;
                                }
                            } else {
                                let mut n = start_n;
                                while n >= end_n {
                                    let num_str = if needs_pad {
                                        if n < 0 {
                                            format!("-{:0>w$}", -n, w = width - 1)
                                        } else {
                                            format!("{:0>w$}", n, w = width)
                                        }
                                    } else {
                                        n.to_string()
                                    };
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, num_str, suffix
                                    )));
                                    n -= step;
                                }
                            }
                        } else if parts[0].len() == 1
                            && parts[1].len() == 1
                            && parts[0].chars().next().unwrap().is_ascii_alphabetic()
                                == parts[1].chars().next().unwrap().is_ascii_alphabetic()
                        {
                            // Character range: {a..z} or {a..z..2}
                            let start_c = parts[0].chars().next().unwrap() as i32;
                            let end_c = parts[1].chars().next().unwrap() as i32;
                            let step: i32 = if parts.len() >= 3 {
                                match parts[2].parse::<i32>() {
                                    Ok(v) => v,
                                    Err(_) => {
                                        return vec![s.to_string()];
                                    }
                                }
                            } else {
                                1
                            };
                            let step = if step == 0 { 1 } else { step.abs() };
                            if start_c <= end_c {
                                let mut c = start_c;
                                while c <= end_c {
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, c as u8 as char, suffix
                                    )));
                                    c += step;
                                }
                            } else {
                                let mut c = start_c;
                                while c >= end_c {
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, c as u8 as char, suffix
                                    )));
                                    c -= step;
                                }
                            }
                        }
                        if !result.is_empty() {
                            return result;
                        }
                    }
                    // Sequence was invalid — don't skip, let inner braces expand
                }
            }
        }
        i += 1;
    }

    vec![s.to_string()]
}

fn split_brace_alternatives(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if (ch == '\\' || ch == '\x00') && i + 1 < chars.len() {
            current.push(ch);
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }
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
        i += 1;
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
                    vec![remove_quotes(field)]
                } else {
                    results.sort();
                    results
                }
            }
            Err(_) => vec![remove_quotes(field)],
        }
    } else {
        vec![remove_quotes(field)]
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

fn extglob_star_match_ex(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    if pattern_match_impl(text, ti, pattern, rest_pi) {
        return true;
    }
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0)
                && extglob_star_match_ex(text, end, alts, pattern, rest_pi)
            {
                return true;
            }
        }
    }
    false
}

fn extglob_plus_match_ex(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                if pattern_match_impl(text, end, pattern, rest_pi) {
                    return true;
                }
                if extglob_star_match_ex(text, end, alts, pattern, rest_pi) {
                    return true;
                }
            }
        }
    }
    false
}

fn find_extglob_close_ex(pattern: &[char], start: usize) -> Option<usize> {
    let mut depth = 1;
    let mut i = start;
    while i < pattern.len() {
        if pattern[i] == '(' && i > 0 && matches!(pattern[i - 1], '@' | '?' | '*' | '+' | '!') {
            depth += 1;
        } else if pattern[i] == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_extglob_alts_ex(pattern: &[char]) -> Vec<Vec<char>> {
    let mut alts = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0;
    for &ch in pattern {
        if ch == '(' {
            depth += 1;
            current.push(ch);
        } else if ch == ')' {
            depth -= 1;
            current.push(ch);
        } else if ch == '|' && depth == 0 {
            alts.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    alts.push(current);
    alts
}

fn pattern_match_impl(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    while pi < pattern.len() {
        // Extglob
        if pi + 1 < pattern.len()
            && pattern[pi + 1] == '('
            && matches!(pattern[pi], '@' | '?' | '*' | '+' | '!')
        {
            let op = pattern[pi];
            if let Some(close) = find_extglob_close_ex(pattern, pi + 2) {
                let inner: Vec<char> = pattern[pi + 2..close].to_vec();
                let rest_pi = close + 1;
                let alts = split_extglob_alts_ex(&inner);
                match op {
                    '@' => {
                        for alt in &alts {
                            let mut combined = alt.clone();
                            combined.extend_from_slice(&pattern[rest_pi..]);
                            if pattern_match_impl(text, ti, &combined, 0) {
                                return true;
                            }
                        }
                        return false;
                    }
                    '?' => {
                        if pattern_match_impl(text, ti, pattern, rest_pi) {
                            return true;
                        }
                        for alt in &alts {
                            let mut combined = alt.clone();
                            combined.extend_from_slice(&pattern[rest_pi..]);
                            if pattern_match_impl(text, ti, &combined, 0) {
                                return true;
                            }
                        }
                        return false;
                    }
                    '*' => return extglob_star_match_ex(text, ti, &alts, pattern, rest_pi),
                    '+' => return extglob_plus_match_ex(text, ti, &alts, pattern, rest_pi),
                    '!' => {
                        for end in ti..=text.len() {
                            let mut any_match = false;
                            for alt in &alts {
                                if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                                    any_match = true;
                                    break;
                                }
                            }
                            if !any_match && pattern_match_impl(text, end, pattern, rest_pi) {
                                return true;
                            }
                        }
                        return false;
                    }
                    _ => unreachable!(),
                }
            }
        }

        match pattern[pi] {
            // \x00 prefix means the next char is quoted (literal, not a glob char)
            '\x00' => {
                pi += 1;
                if pi >= pattern.len() {
                    return false;
                }
                if ti >= text.len() || text[ti] != pattern[pi] {
                    return false;
                }
                ti += 1;
                pi += 1;
            }
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
                let bracket_start = pi;
                pi += 1;
                let negate = pi < pattern.len() && (pattern[pi] == '!' || pattern[pi] == '^');
                if negate {
                    pi += 1;
                }
                let mut matched = false;
                let ch = text[ti];
                // In POSIX, ] at the start of a bracket expression is a literal
                let bracket_first = pi;
                while pi < pattern.len() && (pattern[pi] != ']' || pi == bracket_first) {
                    // Handle backslash or \x00 escape inside bracket
                    if (pattern[pi] == '\\' || pattern[pi] == '\x00') && pi + 1 < pattern.len() {
                        pi += 1;
                        if pattern[pi] == ch {
                            matched = true;
                        }
                        pi += 1;
                        continue;
                    }
                    // POSIX character class: [:class:]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == ':'
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == ':')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        let class_name: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        let in_class = match class_name.as_str() {
                            "alpha" => ch.is_alphabetic(),
                            "digit" => ch.is_ascii_digit(),
                            "alnum" => ch.is_alphanumeric(),
                            "upper" => ch.is_uppercase(),
                            "lower" => ch.is_lowercase(),
                            "space" => ch.is_whitespace(),
                            "blank" => ch == ' ' || ch == '\t',
                            "print" => !ch.is_control() || ch == ' ',
                            "graph" => !ch.is_control() && ch != ' ',
                            "cntrl" => ch.is_control(),
                            "punct" => ch.is_ascii_punctuation(),
                            "xdigit" => ch.is_ascii_hexdigit(),
                            "ascii" => ch.is_ascii(),
                            _ => false,
                        };
                        if in_class {
                            matched = true;
                        }
                        pi = pi + 2 + end + 2;
                        continue;
                    }
                    // POSIX equivalence class: [=x=]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == '='
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == '=')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        // In C locale, equivalence class matches the character itself
                        let equiv: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        if equiv.len() == 1 && ch == equiv.chars().next().unwrap() {
                            matched = true;
                        }
                        pi = pi + 2 + end + 2;
                        continue;
                    }
                    // POSIX collating symbol: [.x.] or [.name.]
                    if pi + 1 < pattern.len()
                        && pattern[pi] == '['
                        && pattern[pi + 1] == '.'
                        && let Some(end) =
                            pattern[pi + 2..]
                                .iter()
                                .position(|&c| c == '.')
                                .filter(|&pos| {
                                    pi + 2 + pos + 1 < pattern.len()
                                        && pattern[pi + 2 + pos + 1] == ']'
                                })
                    {
                        // Extract the collating element name
                        let elem: String = pattern[pi + 2..pi + 2 + end].iter().collect();
                        // For single-char elements, match directly
                        // For multi-char or named elements, use lookup
                        let collating_char = match elem.as_str() {
                            "hyphen" | "-" => Some('-'),
                            "space" | " " => Some(' '),
                            "tab" => Some('\t'),
                            "newline" => Some('\n'),
                            "grave-accent" | "`" => Some('`'),
                            s if s.len() == 1 => s.chars().next(),
                            _ => None, // multi-char collating elements not fully supported
                        };
                        // Check if this is part of a range: [.a.]-[.z.]
                        let collating_end_pi = pi + 2 + end + 2;
                        if collating_end_pi + 1 < pattern.len()
                            && pattern[collating_end_pi] == '-'
                            && pattern[collating_end_pi + 1] != ']'
                        {
                            // Check if range end is another collating symbol or a literal
                            if collating_end_pi + 2 < pattern.len()
                                && pattern[collating_end_pi + 1] == '['
                                && pattern[collating_end_pi + 2] == '.'
                            {
                                // Range: [.x.]-[.y.]
                                if let Some(end2) = pattern[collating_end_pi + 3..]
                                    .iter()
                                    .position(|&c| c == '.')
                                    .filter(|&pos| {
                                        collating_end_pi + 3 + pos + 1 < pattern.len()
                                            && pattern[collating_end_pi + 3 + pos + 1] == ']'
                                    })
                                {
                                    let elem2: String = pattern
                                        [collating_end_pi + 3..collating_end_pi + 3 + end2]
                                        .iter()
                                        .collect();
                                    let range_start = match elem.as_str() {
                                        s if s.len() == 1 => s.chars().next(),
                                        _ => collating_char,
                                    };
                                    let range_end = match elem2.as_str() {
                                        s if s.len() == 1 => s.chars().next(),
                                        _ => None,
                                    };
                                    if let (Some(rs), Some(re)) = (range_start, range_end)
                                        && ch >= rs
                                        && ch <= re
                                    {
                                        matched = true;
                                    }
                                    pi = collating_end_pi + 3 + end2 + 2;
                                    continue;
                                }
                            } else {
                                // Range: [.x.]-y (collating start, literal end)
                                let range_end = pattern[collating_end_pi + 1];
                                if let Some(rs) = collating_char
                                    && ch >= rs
                                    && ch <= range_end
                                {
                                    matched = true;
                                }
                                pi = collating_end_pi + 2;
                                continue;
                            }
                        }
                        if let Some(cc) = collating_char
                            && ch == cc
                        {
                            matched = true;
                        }
                        pi = collating_end_pi;
                        continue;
                    }
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' && pattern[pi + 2] != ']' {
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
                    pi += 1; // skip closing ]
                } else {
                    // Unclosed bracket — treat [ as literal, ignore any partial matches
                    if ti >= text.len() || text[ti] != '[' {
                        return false;
                    }
                    ti += 1;
                    pi = bracket_start + 1;
                    continue;
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
