mod arithmetic;
mod params;
mod pattern;

use params::{apply_param_op, expand_param, get_array_elements, is_array_at_expansion, lookup_var};
use pattern::{TrimMode, pattern_replace, shell_pattern_match, trim_pattern};

use crate::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

thread_local! {
    /// File descriptors opened by process substitutions that need to be closed
    /// after the command using them completes.
    static PROCSUB_FDS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Current script name for error messages from expansion code
    static SCRIPT_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    /// Flag set when arithmetic evaluation encounters an error.
    static ARITH_ERROR: RefCell<bool> = const { RefCell::new(false) };
    /// PID of the last process substitution child (for $!)
    static LAST_PROCSUB_PID: RefCell<Option<i32>> = const { RefCell::new(None) };
    /// Whether dotglob shopt is enabled (for glob expansion)
    static DOTGLOB_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Whether globskipdots shopt is enabled (skip . and .. in glob results)
    static GLOBSKIPDOTS_ENABLED: RefCell<bool> = const { RefCell::new(true) };
    /// Whether globstar shopt is enabled (** matches recursively)
    static GLOBSTAR_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// GLOBIGNORE patterns (colon-separated, empty = no ignore)
    static GLOBIGNORE: RefCell<String> = const { RefCell::new(String::new()) };
    /// Callback for running process substitution commands inline (instead of exec'ing
    /// a new shell). Set by the interpreter before word expansion.
    #[allow(clippy::type_complexity)]
    static PROCSUB_RUNNER: RefCell<Option<*mut dyn FnMut(&str) -> i32>> = const { RefCell::new(None) };
}

/// Set the process substitution runner callback (called from the interpreter)
/// Safety: caller must ensure the pointer remains valid until `clear_procsub_runner`
pub fn set_procsub_runner(f: *mut dyn FnMut(&str) -> i32) {
    PROCSUB_RUNNER.with(|r| {
        *r.borrow_mut() = Some(f);
    });
}

/// Clear the process substitution runner
pub fn clear_procsub_runner() {
    PROCSUB_RUNNER.with(|r| {
        *r.borrow_mut() = None;
    });
}

/// Run a process substitution command using the registered runner
#[allow(dead_code)]
fn run_procsub_inline(cmd: &str) -> Option<i32> {
    // Take the runner out to avoid RefCell borrow conflicts during recursive expansion
    let runner = PROCSUB_RUNNER.with(|r| r.borrow_mut().take());
    if let Some(ptr) = runner {
        // Safety: the pointer is valid for the duration of the expansion call
        let f = unsafe { &mut *ptr };
        let result = f(cmd);
        // Put it back
        PROCSUB_RUNNER.with(|r| {
            *r.borrow_mut() = Some(ptr);
        });
        Some(result)
    } else {
        None
    }
}

pub fn set_script_name(name: &str) {
    SCRIPT_NAME.with(|f| *f.borrow_mut() = name.to_string());
}

pub fn set_dotglob(enabled: bool) {
    DOTGLOB_ENABLED.with(|d| *d.borrow_mut() = enabled);
}

pub fn set_globskipdots(enabled: bool) {
    GLOBSKIPDOTS_ENABLED.with(|d| *d.borrow_mut() = enabled);
}

pub fn set_globstar(enabled: bool) {
    GLOBSTAR_ENABLED.with(|d| *d.borrow_mut() = enabled);
}

pub fn set_globignore(value: &str) {
    GLOBIGNORE.with(|g| *g.borrow_mut() = value.to_string());
}

#[allow(dead_code)]
pub fn get_script_name() -> String {
    SCRIPT_NAME.with(|f| f.borrow().clone())
}

pub fn warn_incomplete_comsub_in_pattern_impl(word: &Word, lineno: &str) {
    let parts: Vec<&WordPart> = word.iter().collect();
    for (idx, part) in parts.iter().enumerate() {
        if let WordPart::SingleQuoted(s) = part
            && let Some(pos) = s.find("$(")
        {
            let after_dollar = &s[pos + 2..];
            if !after_dollar.is_empty() && !s[pos..].contains(')') {
                let has_paren_after = parts[idx + 1..]
                    .iter()
                    .any(|p| matches!(p, WordPart::Literal(t) if t.contains(')')));
                if has_paren_after {
                    let name = SCRIPT_NAME.with(|f| f.borrow().clone());
                    if name.is_empty() {
                        eprintln!(
                            "command substitution: line {}: unexpected EOF while looking for matching `)'",
                            lineno
                        );
                    } else {
                        eprintln!(
                            "{}: command substitution: line {}: unexpected EOF while looking for matching `)'",
                            name, lineno
                        );
                    }
                    return;
                }
            }
        }
    }
}

/// Check and clear the arithmetic error flag.
pub fn take_arith_error() -> bool {
    ARITH_ERROR.with(|f| std::mem::replace(&mut *f.borrow_mut(), false))
}

/// Set the arithmetic error flag.
pub fn set_arith_error() {
    ARITH_ERROR.with(|f| *f.borrow_mut() = true);
}

/// Take all pending process substitution fds (draining the list).
/// Get the PID of the last process substitution child (for $!)
pub fn take_last_procsub_pid() -> Option<i32> {
    LAST_PROCSUB_PID.with(|p| p.borrow_mut().take())
}

pub fn take_procsub_fds() -> Vec<i32> {
    PROCSUB_FDS.with(|fds| std::mem::take(&mut *fds.borrow_mut()))
}

/// Take procsub fds whose `/dev/fd/N` path does NOT appear in any of the given words.
/// Fds that DO appear are kept for later cleanup.
#[allow(dead_code)]
pub fn take_procsub_fds_not_in(words: &[String]) -> Vec<i32> {
    PROCSUB_FDS.with(|fds| {
        let mut all = fds.borrow_mut();
        let mut unused = Vec::new();
        let mut kept = Vec::new();
        for fd in all.drain(..) {
            let path = format!("/dev/fd/{}", fd);
            if words.iter().any(|w| w.contains(&path)) {
                kept.push(fd);
            } else {
                unused.push(fd);
            }
        }
        *all = kept;
        unused
    })
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
    top_level_pid: u32,
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
        top_level_pid,
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
    top_level_pid: u32,
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
        top_level_pid,
        opt_flags,
    };
    let segments = expand_word_to_segments(word, &ctx, cmd_sub);
    // Check for incomplete comsub marker before stripping \x00
    for seg in &segments {
        match seg {
            Segment::Unquoted(t) | Segment::Quoted(t) if t.starts_with("\x00INCOMPLETE_COMSUB") => {
                return t.clone();
            }
            _ => {}
        }
    }
    let result: String = segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
        })
        .collect();
    result.replace('\x00', "")
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
    top_level_pid: u32,
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
        top_level_pid,
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
    top_level_pid: u32,
    opt_flags: &'a str,
}

thread_local! {
    /// Error prefix set by the interpreter for expansion error messages
    pub static EXPAND_ERROR_PREFIX: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
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
        // Special variables are always "set" (except $@ and $* with no args)
        match name {
            "#" | "?" | "-" | "$" | "!" | "0" => return true,
            "@" | "*" => return self.positional.len() > 1,
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
    // If any segment came from an incomplete comsub/funsub, suppress output
    let has_any_incomplete = segments.iter().any(|s| match s {
        Segment::Unquoted(t) | Segment::Quoted(t) => {
            t.starts_with("\x00INCOMPLETE_COMSUB")
                || t == "\x00SILENT_COMSUB"
                || t.contains("\x00INCOMPLETE_FUNSUB")
        }
        _ => false,
    });
    if has_any_incomplete {
        // Check for incomplete funsub
        let has_funsub = segments.iter().any(|s| match s {
            Segment::Unquoted(t) | Segment::Quoted(t) => t.contains("\x00INCOMPLETE_FUNSUB"),
            _ => false,
        });
        if has_funsub {
            return vec![Segment::Unquoted("\x00INCOMPLETE_FUNSUB".to_string())];
        }
        // Check if it's a noisy (error) or silent suppression
        let is_error = segments.iter().any(|s| match s {
            Segment::Unquoted(t) | Segment::Quoted(t) => t.starts_with("\x00INCOMPLETE_COMSUB"),
            _ => false,
        });
        if is_error {
            // Preserve the line info from the marker
            let marker = segments
                .iter()
                .find_map(|s| match s {
                    Segment::Unquoted(t) | Segment::Quoted(t)
                        if t.starts_with("\x00INCOMPLETE_COMSUB") =>
                    {
                        Some(t.clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "\x00INCOMPLETE_COMSUB".to_string());
            return vec![Segment::Unquoted(marker)];
        }
        // Silent — suppress with marker so interpreter can detect
        return vec![Segment::Unquoted("\x00SILENT_COMSUB".to_string())];
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
                        let val = lookup_var(name, ctx);
                        if val.is_empty()
                            && ctx.opt_flags.contains('u')
                            && !matches!(name.as_str(), "?" | "$" | "#" | "@" | "*" | "-" | "0")
                            && !(name == "!" && ctx.last_bg_pid != 0)
                            && name.parse::<usize>().is_err()
                            && !ctx.vars.contains_key(name.as_str())
                            && !ctx.arrays.contains_key(name.as_str())
                            && std::env::var(name.as_str()).is_err()
                        {
                            let sname = ctx
                                .vars
                                .get("_BASH_SOURCE_FILE")
                                .or_else(|| ctx.positional.first())
                                .map(|s| s.as_str())
                                .unwrap_or("bash");
                            let lineno = ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                            eprintln!("{}: line {}: {}: unbound variable", sname, lineno, name);
                            set_arith_error();
                        }
                        s.push_str(&val);
                    }
                    WordPart::Param(expr)
                        if (expr.name == "@" || expr.name == "*")
                            && !matches!(
                                expr.op,
                                ParamOp::None | ParamOp::Length | ParamOp::Substring(..)
                            )
                            && ctx.positional.len() > 1 =>
                    {
                        // "${@%pattern}" etc. — apply op to each element separately
                        if !s.is_empty() {
                            out.push(Segment::Quoted(std::mem::take(&mut s)));
                        }
                        let sep = if expr.name == "@" {
                            None // SplitHere
                        } else {
                            Some(
                                ctx.vars
                                    .get("IFS")
                                    .and_then(|s| s.chars().next())
                                    .unwrap_or(' '),
                            )
                        };
                        for (i, elem) in ctx.positional[1..].iter().enumerate() {
                            if i > 0 {
                                match sep {
                                    None => out.push(Segment::SplitHere),
                                    Some(c) => s.push(c),
                                }
                            }
                            let result = apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name);
                            if sep.is_none() {
                                out.push(Segment::Quoted(result));
                            } else {
                                s.push_str(&result);
                            }
                        }
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
                        // Determine if this is $* (join with IFS) or $@ (split)
                        let is_star = if let Some(bracket) = expr.name.find('[') {
                            &expr.name[bracket + 1..expr.name.len() - 1] == "*"
                        } else {
                            expr.name == "*"
                        };
                        if is_star {
                            // "${*:...}" or "${arr[*]:...}" — join with IFS[0]
                            let ifs_char = ctx
                                .vars
                                .get("IFS")
                                .and_then(|s| s.chars().next())
                                .unwrap_or(' ');
                            for (i, elem) in elements.iter().enumerate() {
                                if i > 0 {
                                    s.push(ifs_char);
                                }
                                let modified = if matches!(&expr.op, ParamOp::Substring(..)) {
                                    elem.clone()
                                } else {
                                    apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name)
                                };
                                s.push_str(&modified);
                            }
                        } else {
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
                                    apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name)
                                };
                                out.push(Segment::Quoted(modified));
                            }
                        }
                    }
                    WordPart::Param(expr) => {
                        // Check if this is ${var+"$@"} or ${var-"$@"} — needs per-field expansion
                        let has_at_in_word = match &expr.op {
                            ParamOp::Default(_, word) | ParamOp::Alt(_, word) => {
                                word.iter().any(|p| {
                                    matches!(p, WordPart::Variable(n) if n == "@")
                                        || matches!(p, WordPart::DoubleQuoted(parts) if parts.iter().any(|ip| matches!(ip, WordPart::Variable(n) if n == "@")))
                                })
                            }
                            _ => false,
                        };
                        let is_alt_active = match &expr.op {
                            ParamOp::Default(colon, _) => {
                                let val = lookup_var(&expr.name, ctx);
                                let set = ctx.is_param_set(&expr.name);
                                let empty = if *colon { val.is_empty() } else { false };
                                !set || empty
                            }
                            ParamOp::Alt(colon, _) => {
                                let val = lookup_var(&expr.name, ctx);
                                let set = ctx.is_param_set(&expr.name);
                                let empty = if *colon { val.is_empty() } else { false };
                                set && !empty
                            }
                            _ => false,
                        };
                        if has_at_in_word && is_alt_active {
                            // Expand the word per-part to get SplitHere from $@
                            if !s.is_empty() {
                                out.push(Segment::Quoted(std::mem::take(&mut s)));
                            }
                            if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op {
                                let mut inner = Vec::new();
                                for part in word {
                                    // For bare $@ inside dquoted Default/Alt,
                                    // produce SplitHere markers like "$@" does
                                    if matches!(part, WordPart::Variable(n) if n == "@") {
                                        for (i, arg) in ctx.positional[1..].iter().enumerate() {
                                            if i > 0 {
                                                inner.push(Segment::SplitHere);
                                            }
                                            inner.push(Segment::Quoted(arg.clone()));
                                        }
                                    } else {
                                        expand_part(part, ctx, &mut inner, cmd_sub);
                                    }
                                }
                                for seg in inner {
                                    match seg {
                                        Segment::Quoted(t)
                                        | Segment::Unquoted(t)
                                        | Segment::Literal(t) => s.push_str(&t),
                                        Segment::SplitHere => {
                                            out.push(Segment::Quoted(std::mem::take(&mut s)));
                                            out.push(Segment::SplitHere);
                                        }
                                    }
                                }
                            }
                        } else {
                            s.push_str(&expand_param(expr, ctx, cmd_sub));
                        }
                    }
                    WordPart::CommandSub(cmd) => {
                        if cmd.starts_with('\x00') {
                            // Incomplete comsub — mark for suppression
                            if !s.is_empty() {
                                out.push(Segment::Quoted(std::mem::take(&mut s)));
                            }
                            out.push(Segment::Unquoted(cmd.clone()));
                            continue;
                        }
                        let trimmed = cmd.trim();
                        if let Some(file) = trimmed
                            .strip_prefix("< ")
                            .or_else(|| trimmed.strip_prefix("<\t"))
                        {
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
            // Push Quoted for DoubleQuoted — even empty strings create a field.
            // EXCEPT when the only content was "$@" with no positional params,
            // which should produce zero fields, not one empty field.
            let only_at = parts
                .iter()
                .all(|p| matches!(p, WordPart::Variable(n) if n == "@"))
                && !parts.is_empty();
            let at_was_empty = only_at && s.is_empty() && ctx.positional.len() <= 1;
            if !at_was_empty {
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
            if val.is_empty()
                && ctx.opt_flags.contains('u')
                && !matches!(name.as_str(), "?" | "$" | "#" | "@" | "*" | "-" | "0")
                // $! is unbound when no background job has been started
                && !(name == "!" && ctx.last_bg_pid != 0)
                && name.parse::<usize>().is_err()
                && !ctx.vars.contains_key(name.as_str())
                && !ctx.arrays.contains_key(name.as_str())
                && std::env::var(name.as_str()).is_err()
            {
                let sname = ctx
                    .vars
                    .get("_BASH_SOURCE_FILE")
                    .or_else(|| ctx.positional.first())
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                let lineno = ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                eprintln!("{}: line {}: {}: unbound variable", sname, lineno, name);
                set_arith_error();
            }
            // Unquoted $@ should still produce separate words (like "$@")
            // even with null IFS — the splitting is inherent to $@
            if name == "@" && ctx.positional.len() > 1 {
                for (i, arg) in ctx.positional[1..].iter().enumerate() {
                    if i > 0 {
                        out.push(Segment::SplitHere);
                    }
                    out.push(Segment::Unquoted(arg.clone()));
                }
            } else {
                out.push(Segment::Unquoted(val));
            }
        }
        WordPart::Param(expr) => {
            // Handle unquoted ${@%pattern}, ${@#pattern}, ${@/pat/rep} etc.
            // Each positional param should be expanded separately
            if (expr.name == "@" || expr.name == "*")
                && !matches!(
                    expr.op,
                    ParamOp::None | ParamOp::Length | ParamOp::Substring(..)
                )
                && ctx.positional.len() > 1
            {
                let mut first = true;
                for elem in &ctx.positional[1..] {
                    if !first {
                        out.push(if expr.name == "@" {
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
                    let result = apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name);
                    out.push(Segment::Unquoted(result));
                }
                return;
            }
            // Check if this is an array[@] with an operator that should apply per-element
            if let Some(bracket) = expr.name.find('[') {
                let base = &expr.name[..bracket];
                let idx = &expr.name[bracket + 1..expr.name.len().saturating_sub(1)];
                let resolved = ctx.resolve_nameref(base);
                if (idx == "@" || idx == "*")
                    && !matches!(
                        expr.op,
                        ParamOp::None | ParamOp::Length | ParamOp::Substring(..)
                    )
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
                        let result =
                            apply_param_op(elem, &single_expr.op, ctx, cmd_sub, &single_expr.name);
                        out.push(Segment::Unquoted(result));
                    }
                    return;
                }
                // Handle ${arr[@]:offset:length} — array slice
                if (idx == "@" || idx == "*") && matches!(expr.op, ParamOp::Substring(..)) {
                    let elements: Option<Vec<String>> = if let Some(arr) = ctx.arrays.get(&resolved)
                    {
                        Some(arr.clone())
                    } else {
                        ctx.assoc_arrays
                            .get(&resolved)
                            .map(|a| a.values().cloned().collect())
                    };
                    if let Some(arr) = elements {
                        if let ParamOp::Substring(offset_str, length_str) = &expr.op {
                            let offset: i64 = offset_str.trim().parse().unwrap_or(0);
                            let count = arr.len();
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
                            let mut first = true;
                            for elem in &arr[start..end] {
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
                                out.push(Segment::Unquoted(elem.clone()));
                            }
                        }
                        return;
                    }
                }
            }
            // For Default/Alt words in unquoted context, check if we should
            // expand per-part for mixed quoting (e.g., ${IFS+foo 'bar' baz})
            let orig_val = lookup_var(&expr.name, ctx);
            let orig_set = ctx.is_param_set(&expr.name);
            let is_default_alt_active = match &expr.op {
                ParamOp::Default(colon, _) => {
                    let empty = if *colon { orig_val.is_empty() } else { false };
                    !orig_set || empty
                }
                ParamOp::Alt(colon, _) => {
                    let empty = if *colon { orig_val.is_empty() } else { false };
                    orig_set && !empty
                }
                _ => false,
            };
            // Check if word needs per-part expansion:
            // - mixed quoting (both literal and quoted parts)
            // - contains $@ which needs to produce separate fields
            let needs_per_part = if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op
            {
                let has_literal = word.iter().any(|p| matches!(p, WordPart::Literal(_)));
                let has_quoted = word
                    .iter()
                    .any(|p| matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_)));
                let has_at = word.iter().any(|p| {
                        matches!(p, WordPart::Variable(n) if n == "@")
                            || matches!(p, WordPart::DoubleQuoted(parts) if parts.iter().any(|ip| matches!(ip, WordPart::Variable(n) if n == "@")))
                    });
                (has_literal && has_quoted) || has_at
            } else {
                false
            };
            if is_default_alt_active && needs_per_part {
                // Per-part expansion: Literals are Unquoted (split), rest keeps quoting
                if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op {
                    for part in word {
                        match part {
                            WordPart::Literal(s) => out.push(Segment::Unquoted(s.clone())),
                            _ => expand_part(part, ctx, out, cmd_sub),
                        }
                    }
                }
            } else {
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
                // Check if the word has quoted content for field-splitting protection
                let has_quoted_word = if is_default_alt_active {
                    if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op {
                        word.iter().any(|p| {
                            matches!(p, WordPart::DoubleQuoted(_) | WordPart::SingleQuoted(_))
                        })
                    } else {
                        false
                    }
                } else {
                    false
                };
                if has_quoted_word {
                    out.push(Segment::Quoted(val));
                } else {
                    out.push(Segment::Unquoted(val));
                }
            }
        }
        WordPart::CommandSub(cmd) => {
            // Check for incomplete comsub marker
            if cmd.starts_with('\x00') {
                out.push(Segment::Unquoted(cmd.clone()));
                return;
            }
            // Optimize $(< file) — read file content directly
            let trimmed = cmd.trim();
            let val = if let Some(file) = trimmed
                .strip_prefix("< ")
                .or_else(|| trimmed.strip_prefix("<\t"))
            {
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
            // Protect backslashes from quote removal with \x00 markers
            let val = val.replace('\\', "\x00\\");
            out.push(Segment::Unquoted(val));
        }
        WordPart::BacktickSub(cmd) => {
            let val = cmd_sub(cmd);
            // Protect backslashes from quote removal with \x00 markers
            let val = val.replace('\\', "\x00\\");
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
                let r_raw = pipe_r.as_raw_fd();
                let w_raw = pipe_w.as_raw_fd();
                // Move procsub pipe fds to high numbers (>=10) to avoid conflicts
                // with user-requested fds like `exec 3< <(cmd)`
                let r_fd =
                    nix::fcntl::fcntl(r_raw, nix::fcntl::FcntlArg::F_DUPFD(10)).unwrap_or(r_raw);
                let w_fd =
                    nix::fcntl::fcntl(w_raw, nix::fcntl::FcntlArg::F_DUPFD(10)).unwrap_or(w_raw);
                if r_fd != r_raw {
                    nix::unistd::close(r_raw).ok();
                }
                if w_fd != w_raw {
                    nix::unistd::close(w_raw).ok();
                }
                // Prevent OwnedFd from closing (they're already moved/closed above)
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
                        // Run command inline if procsub runner is available
                        // (preserves LINENO and script name for error messages)
                        if let Some(status) = run_procsub_inline(cmd) {
                            std::io::Write::flush(&mut std::io::stdout()).ok();
                            std::io::Write::flush(&mut std::io::stderr()).ok();
                            std::process::exit(status);
                        }
                        // Fallback: exec a new shell
                        unsafe {
                            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                        }
                        for (k, v) in ctx.vars {
                            unsafe { std::env::set_var(k, v) };
                        }
                        use std::ffi::CString;
                        let bash = CString::new("/proc/self/exe").unwrap();
                        let c_flag = CString::new("-c").unwrap();
                        let c_cmd = CString::new(cmd.as_str()).unwrap();
                        let script_name =
                            ctx.positional.first().map(|s| s.as_str()).unwrap_or("bash");
                        let c_name = CString::new(script_name)
                            .unwrap_or_else(|_| CString::new("bash").unwrap());
                        nix::unistd::execvp(&bash, &[&bash, &c_flag, &c_cmd, &c_name]).ok();
                        std::process::exit(127);
                    }
                    Ok(nix::unistd::ForkResult::Parent { child, .. }) => {
                        // Store PID for $! (bash sets $! to last procsub PID)
                        LAST_PROCSUB_PID.with(|p| {
                            *p.borrow_mut() = Some(child.as_raw());
                        });
                        let fd = match kind {
                            ProcessSubKind::Input => {
                                nix::unistd::close(w_fd).ok();
                                // Clear CLOEXEC so child processes can access /proc/self/fd/N
                                nix::fcntl::fcntl(
                                    r_fd,
                                    nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                                )
                                .ok();
                                register_procsub_fd(r_fd);
                                r_fd
                            }
                            ProcessSubKind::Output => {
                                nix::unistd::close(r_fd).ok();
                                nix::fcntl::fcntl(
                                    w_fd,
                                    nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                                )
                                .ok();
                                register_procsub_fd(w_fd);
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
        WordPart::BadSubstitution(expr) => {
            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                let p = p.borrow();
                if p.is_empty() {
                    "bash".to_string()
                } else {
                    p.clone()
                }
            });
            eprintln!("{}: {}: bad substitution", prefix, expr);
            // Push empty to avoid breaking segment collection
            out.push(Segment::Unquoted(String::new()));
        }
    }
}

fn expand_word_nosplit_ctx(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    let segments = expand_word_to_segments(word, ctx, cmd_sub);
    // Check for incomplete comsub marker before stripping \x00
    for seg in &segments {
        match seg {
            Segment::Unquoted(t) | Segment::Quoted(t) if t.starts_with("\x00INCOMPLETE_COMSUB") => {
                return t.clone();
            }
            _ => {}
        }
    }
    let result: String = segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
        })
        .collect();
    // Strip any \x00 escape markers from backtick results
    result.replace('\x00', "")
}

/// Like expand_word_nosplit_ctx but adds \x00 quoting prefix before glob metacharacters
/// in quoted content. This lets the pattern matcher distinguish literal characters
/// from glob metacharacters in ${var//pat/rep} patterns.
fn expand_pattern_word(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    let segments = expand_word_to_segments(word, ctx, cmd_sub);
    let mut result = String::new();
    for s in &segments {
        match s {
            Segment::Quoted(t) => {
                // Use quote_glob_chars to protect glob metacharacters
                result.push_str(&quote_glob_chars(t));
            }
            Segment::Unquoted(t) | Segment::Literal(t) => {
                // Strip \x00 markers from unquoted segments so that backslashes
                // from command substitution results act as pattern escape characters
                // (e.g., backtick result \\ means "match one literal \")
                result.push_str(&t.replace('\x00', ""));
            }
            Segment::SplitHere => result.push(' '),
        }
    }
    result
}

fn expand_arith(expr: &str, ctx: &ExpCtx) -> String {
    let result = eval_arith_full(expr, ctx.vars, ctx.arrays, ctx.positional, ctx.last_status);
    result.to_string()
}

pub use arithmetic::eval_arith_full;

/// Escape glob metacharacters in quoted text so they are treated literally.
/// Uses \x00 as escape prefix (cannot appear in normal shell text).
fn quote_glob_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if matches!(ch, '*' | '?' | '[' | ']' | '{' | '}' | ',' | '\\') {
            out.push('\x00');
        }
        // Also quote extglob operators (@+!*?) before ( to prevent glob expansion
        if matches!(ch, '@' | '+' | '!') && i + 1 < chars.len() && chars[i + 1] == '(' {
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
        } else {
            // All other characters (including \) are kept as-is.
            // Backslashes from the original word are already handled via \x00
            // markers (SingleQuoted → Segment::Quoted → quote_glob_chars).
            // Bare \ at this point is from variable expansion and is literal.
            result.push(ch);
        }
    }
    result
}

/// Returns true if the string contains unescaped glob metacharacters.
fn has_glob_chars(s: &str) -> bool {
    let mut prev_null = false;
    let mut prev_backslash = false;
    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\x00' {
            prev_null = true;
            prev_backslash = false;
            continue;
        }
        if matches!(ch, '*' | '?' | '[') && !prev_null && !prev_backslash {
            return true;
        }
        // Check for extglob patterns: +(, @(, ?(, !(, *(
        if matches!(ch, '+' | '@' | '?' | '!' | '*')
            && !prev_null
            && !prev_backslash
            && i + 1 < chars.len()
            && chars[i + 1] == '('
        {
            return true;
        }
        prev_backslash = ch == '\\' && !prev_null;
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
    let mut has_quoted_since_split = false;

    for segment in segments {
        match segment {
            Segment::SplitHere => {
                // Force a field break here (for "$@" and "${arr[@]}")
                fields.push(std::mem::take(&mut current));
                has_quoted_since_split = false;
            }
            Segment::Quoted(s) => {
                current.push_str(&quote_glob_chars(s));
                has_quoted_since_split = true;
            }
            Segment::Literal(s) => {
                // Literal text: not IFS-split, glob chars preserved
                current.push_str(s);
            }
            Segment::Unquoted(s) => {
                let ifs_ws: Vec<char> = ifs.chars().filter(|c| c.is_whitespace()).collect();
                let ifs_non_ws: Vec<char> = ifs.chars().filter(|c| !c.is_whitespace()).collect();
                // If we have accumulated content from Quoted/Literal segments,
                // start in "in field" state so IFS whitespace causes a split
                let mut state: u8 = if !current.is_empty() || has_quoted_since_split {
                    1
                } else {
                    0
                };
                for ch in s.chars() {
                    if ifs_non_ws.contains(&ch) {
                        match state {
                            1 => {
                                // End current field
                                fields.push(std::mem::take(&mut current));
                                has_quoted_since_split = false;
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
                            has_quoted_since_split = false;
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

    // Push the last field if:
    // 1. Current is non-empty, OR
    // 2. There was quoted content since the last split (empty quoted string = empty field)
    if !current.is_empty() || has_quoted_since_split {
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
                                    dq_parts.push(WordPart::Literal(std::mem::take(&mut dq_lit)));
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
                        while i < chars.len() && chars[i] != '/' && !chars[i].is_whitespace() {
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
                                    match n.checked_add(step) {
                                        Some(next) => n = next,
                                        None => break,
                                    }
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
                                    match n.checked_sub(step) {
                                        Some(next) => n = next,
                                        None => break,
                                    }
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
                                    // Backslash (0x5C) in char ranges produces empty
                                    // (bash 5.3 outputs NUL which echo drops)
                                    let ch_str = if c == 0x5C {
                                        String::new()
                                    } else {
                                        (c as u8 as char).to_string()
                                    };
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, ch_str, suffix
                                    )));
                                    c += step;
                                }
                            } else {
                                let mut c = start_c;
                                while c >= end_c {
                                    let ch_str = if c == 0x5C {
                                        String::new()
                                    } else {
                                        (c as u8 as char).to_string()
                                    };
                                    result.extend(brace_expand(&format!(
                                        "{}{}{}",
                                        prefix, ch_str, suffix
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

/// Expand a pattern containing ** (globstar) by recursively walking directories
fn globstar_expand(pattern: &str) -> Vec<String> {
    let dotglob = DOTGLOB_ENABLED.with(|d| *d.borrow());

    /// Recursively walk a directory, returning all entries (files and dirs)
    fn walk_dir(dir: &std::path::Path, prefix: &str, dotglob: bool) -> Vec<(String, bool)> {
        let mut entries = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut dir_entries: Vec<_> = rd.flatten().collect();
            dir_entries.sort_by_key(|e| e.file_name());
            for entry in dir_entries {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                if name.starts_with('.') && !dotglob {
                    continue;
                }
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", prefix, name)
                };
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                entries.push((path.clone(), is_dir));
                if is_dir {
                    entries.extend(walk_dir(&entry.path(), &path, dotglob));
                }
            }
        }
        entries
    }

    // Find unquoted ** in the pattern (skip \*\* which is quoted)
    let has_real_globstar = {
        let chars: Vec<char> = pattern.chars().collect();
        let mut found = false;
        let mut i = 0;
        while i < chars.len().saturating_sub(1) {
            if chars[i] == '\x00' {
                i += 2; // skip quoted char
                continue;
            }
            if chars[i] == '*' && chars[i + 1] == '*' {
                found = true;
                break;
            }
            i += 1;
        }
        found
    };

    if !has_real_globstar {
        return vec![pattern.to_string()];
    }

    // Normalize: collapse consecutive ** segments
    // **/** → **, **/**/** → **, **/a/**/** → **/a/**
    let pattern = {
        let mut normalized = pattern.to_string();
        while normalized.contains("**/**") {
            normalized = normalized.replace("**/**", "**");
        }
        normalized
    };

    // Split pattern on first unquoted **
    let (prefix_pat, suffix_pat) = {
        let chars: Vec<char> = pattern.chars().collect();
        let mut split_pos = 0;
        let mut i = 0;
        while i < chars.len().saturating_sub(1) {
            if chars[i] == '\x00' {
                i += 2;
                continue;
            }
            if chars[i] == '*' && chars[i + 1] == '*' {
                split_pos = i;
                break;
            }
            i += 1;
        }
        let prefix: String = chars[..split_pos].iter().collect();
        let suffix: String = chars[split_pos + 2..].iter().collect();
        (prefix, suffix)
    };

    // Determine base directory (from prefix before **)
    let base = prefix_pat.trim_end_matches('/');
    let suffix = suffix_pat.trim_start_matches('/');
    let base_dir = if base.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        std::path::PathBuf::from(base)
    };

    if !base_dir.exists() {
        return vec![pattern.to_string()];
    }

    let mut results = Vec::new();
    let all_entries = walk_dir(&base_dir, base, dotglob);

    if suffix.is_empty() && suffix_pat.is_empty() {
        // Pattern is just "**" or "prefix/**"
        // Include all entries (files and dirs, no trailing slashes)
        if !base.is_empty() {
            // For dir/**, include "dir/" first
            results.push(format!("{}/", base));
        }
        for (path, _is_dir) in &all_entries {
            results.push(path.clone());
        }
    } else if suffix.is_empty() && suffix_pat == "/" {
        // Pattern is "**/" — match directories only
        if !base.is_empty() {
            results.push(format!("{}/", base));
        }
        for (path, is_dir) in &all_entries {
            if *is_dir {
                results.push(format!("{}/", path));
            }
        }
    } else {
        // Pattern is "**/suffix" or "prefix/**/suffix"
        // ** matches zero or more directory levels
        let dir_only = suffix_pat.ends_with('/');
        let suffix_trimmed = suffix.trim_end_matches('/');

        // Collect all directory paths for matching
        // Each entry is (relative_path, is_dir)
        // We try matching the suffix against each possible tail of the path
        for (path, is_dir) in &all_entries {
            // Try matching the suffix against various tail segments of the path
            let path_to_check = if base.is_empty() {
                path.clone()
            } else {
                // Strip the base prefix to get the relative portion after **
                path.strip_prefix(&format!("{}/", base))
                    .unwrap_or(path)
                    .to_string()
            };

            // Try matching suffix against each possible tail
            // e.g., for path "bar/foo/e" and suffix "foo/e", try:
            //   "bar/foo/e", "foo/e", "e"
            let mut tail = path_to_check.as_str();
            loop {
                let match_target = if dir_only && *is_dir {
                    format!("{}/", tail)
                } else {
                    tail.to_string()
                };
                // Use glob pattern matching for the suffix
                let suffix_to_match = if dir_only {
                    format!("{}/", suffix_trimmed)
                } else {
                    suffix.to_string()
                };
                let glob_opts = glob::MatchOptions {
                    require_literal_separator: true,
                    ..Default::default()
                };
                if glob::Pattern::new(&suffix_to_match)
                    .map(|p| p.matches_with(&match_target, glob_opts))
                    .unwrap_or(false)
                {
                    if dir_only {
                        results.push(format!("{}/", path));
                    } else {
                        results.push(path.clone());
                    }
                    break;
                }
                // Try shorter tail (remove first segment)
                if let Some(pos) = tail.find('/') {
                    tail = &tail[pos + 1..];
                } else {
                    break;
                }
            }
        }

        // Also check base-level entries (** matches zero segments)
        if let Ok(rd) = std::fs::read_dir(&base_dir) {
            let mut dir_entries: Vec<_> = rd.flatten().collect();
            dir_entries.sort_by_key(|e| e.file_name());
            for entry in dir_entries {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && !dotglob {
                    continue;
                }
                let is_dir_entry = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let path = if base.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", base, name)
                };
                let suffix_to_match = if dir_only {
                    format!("{}/", suffix_trimmed)
                } else {
                    suffix.to_string()
                };
                let name_to_match = if dir_only && is_dir_entry {
                    format!("{}/", name)
                } else {
                    name.clone()
                };
                let glob_opts2 = glob::MatchOptions {
                    require_literal_separator: true,
                    ..Default::default()
                };
                if glob::Pattern::new(&suffix_to_match)
                    .map(|p| p.matches_with(&name_to_match, glob_opts2))
                    .unwrap_or(false)
                    && (!dir_only || is_dir_entry)
                {
                    let final_path = if dir_only {
                        format!("{}/", path)
                    } else {
                        path.clone()
                    };
                    if !results.contains(&final_path) {
                        results.push(final_path);
                    }
                }
            }
        }
    }

    apply_globignore(&mut results);
    if results.is_empty() {
        vec![remove_quotes(&pattern)]
    } else {
        results.sort();
        results
    }
}

/// Split GLOBIGNORE on colons, respecting bracket expressions [...]
fn split_globignore(globignore: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = globignore.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ':' => {
                if !current.is_empty() {
                    patterns.push(std::mem::take(&mut current));
                }
            }
            '[' => {
                // Inside bracket expression — skip until matching ]
                current.push('[');
                i += 1;
                // Handle [! or [^ negation
                if i < chars.len() && (chars[i] == '!' || chars[i] == '^') {
                    current.push(chars[i]);
                    i += 1;
                }
                // Handle ] as first char in bracket (literal])
                if i < chars.len() && chars[i] == ']' {
                    current.push(']');
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    current.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    current.push(']');
                }
            }
            c => current.push(c),
        }
        i += 1;
    }
    if !current.is_empty() {
        patterns.push(current);
    }
    patterns
}

/// Filter glob results by GLOBIGNORE patterns
fn apply_globignore(results: &mut Vec<String>) {
    let globignore = GLOBIGNORE.with(|g| g.borrow().clone());
    if globignore.is_empty() {
        return;
    }
    let patterns = split_globignore(&globignore);
    if patterns.is_empty() {
        return;
    }
    results.retain(|name| {
        // Don't filter . and ..
        if name == "." || name == ".." {
            return true;
        }
        for pat in &patterns {
            if crate::interpreter::commands::case_pattern_match(name, pat) {
                return false;
            }
        }
        true
    });
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
        // Check for globstar (**) — must be enabled and pattern contains **
        let globstar = GLOBSTAR_ENABLED.with(|g| *g.borrow());
        if globstar && pattern.contains("**") {
            return globstar_expand(&pattern);
        }

        // Check if pattern has extglob chars that the glob crate can't handle
        // Check if pattern needs our custom matcher (extglob or POSIX char classes)
        let needs_custom_match = {
            let pb = pattern.as_bytes();
            let mut found = false;
            // Check for balanced extglob patterns (parens AND brackets must be balanced)
            for i in 0..pb.len().saturating_sub(1) {
                if pb[i + 1] == b'(' && matches!(pb[i], b'+' | b'@' | b'?' | b'!' | b'*') {
                    let mut depth = 1;
                    let mut j = i + 2;
                    let mut bracket_depth = 0i32;
                    while j < pb.len() && depth > 0 {
                        match pb[j] {
                            b'(' => depth += 1,
                            b')' if bracket_depth == 0 => depth -= 1,
                            b'[' => bracket_depth += 1,
                            b']' if bracket_depth > 0 => bracket_depth -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                    // Only valid if both parens and brackets are balanced
                    if depth == 0 && bracket_depth == 0 {
                        found = true;
                        break;
                    }
                }
            }
            // Check for POSIX character classes [[:class:]] or [^...] negation
            if !found && pattern.contains("[[:") {
                found = true;
            }
            if !found && pattern.contains("[^") {
                found = true;
            }
            found
        };
        let has_extglob = needs_custom_match;
        if has_extglob {
            // Use our case_pattern_match for extglob support
            // For simple (non-path) patterns, match against current directory entries
            if !pattern.contains('/') {
                let dir = std::env::current_dir().unwrap_or_default();
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    let dotglob = DOTGLOB_ENABLED.with(|d| *d.borrow());
                    let skipdots = GLOBSKIPDOTS_ENABLED.with(|d| *d.borrow());
                    // Add . and .. when globskipdots is off and pattern explicitly matches
                    // dot-only patterns (not negation or mixed patterns)
                    let extra: Vec<String> = if !skipdots && !pattern.starts_with("!(") {
                        let starts_dot_star = pattern.starts_with(".*")
                            || pattern.starts_with("@(.*")
                            || pattern.contains("|.*");
                        let starts_dot_q = pattern.starts_with(".?")
                            || pattern.starts_with("@(.?")
                            || pattern.starts_with("*(.?")
                            || pattern.starts_with("+(.?")
                            || pattern.contains("|.?");
                        if starts_dot_star {
                            // .* matches both . and ..
                            vec![".".to_string(), "..".to_string()]
                        } else if starts_dot_q {
                            // .? matches .. but not . (. is only 1 char)
                            vec!["..".to_string()]
                        } else {
                            vec![]
                        }
                    } else {
                        vec![]
                    };
                    let mut results: Vec<String> = extra
                        .into_iter()
                        .chain(
                            entries
                                .filter_map(|e| e.ok())
                                .map(|e| e.file_name().to_string_lossy().to_string()),
                        )
                        .filter(|name| {
                            // Negation patterns !(...)  never match dotfiles (without dotglob)
                            if pattern.starts_with("!(") {
                                if name == "." || name == ".." {
                                    return false;
                                }
                                if name.starts_with('.') && !dotglob {
                                    return false;
                                }
                            }
                            // Skip dotfiles unless dotglob is set or pattern explicitly matches dots
                            if name.starts_with('.') && !dotglob {
                                let allows_dot = pattern.starts_with('.')
                                    || (pattern.starts_with("*(") || pattern.starts_with("?("))
                                        && pattern.contains(").");
                                if !allows_dot {
                                    // For extglob with dot alternatives, extract each dot alt
                                    // and match against those specifically
                                    // Use proper top-level splitting that respects nested parens
                                    let inner_start = pattern.find('(').map(|p| p + 1);
                                    let inner_end = pattern.rfind(')');
                                    if let (Some(start), Some(end)) = (inner_start, inner_end) {
                                        let inner = &pattern[start..end];
                                        // Split on | at top level only (not inside nested parens)
                                        let mut alts = Vec::new();
                                        let mut depth = 0i32;
                                        let mut current = String::new();
                                        for ch in inner.chars() {
                                            match ch {
                                                '(' => {
                                                    depth += 1;
                                                    current.push(ch);
                                                }
                                                ')' => {
                                                    depth -= 1;
                                                    current.push(ch);
                                                }
                                                '|' if depth == 0 => {
                                                    alts.push(std::mem::take(&mut current));
                                                }
                                                _ => current.push(ch),
                                            }
                                        }
                                        alts.push(current);
                                        for alt in &alts {
                                            if alt.starts_with('.')
                                                && crate::interpreter::commands::case_pattern_match(
                                                    name, alt,
                                                )
                                            {
                                                return true;
                                            }
                                        }
                                    }
                                    return false;
                                }
                            }
                            crate::interpreter::commands::case_pattern_match(name, &pattern)
                        })
                        .collect();
                    apply_globignore(&mut results);
                    if results.is_empty() {
                        vec![remove_quotes(field)]
                    } else {
                        results.sort();
                        results
                    }
                } else {
                    vec![remove_quotes(field)]
                }
            } else {
                // For path patterns, fall back to glob crate (extglob in paths not supported yet)
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
            }
        } else {
            let dotglob = DOTGLOB_ENABLED.with(|d| *d.borrow());
            // If pattern starts with '.', always allow dotfile matching
            let pattern_starts_dot = pattern.starts_with('.');
            let glob_opts = glob::MatchOptions {
                require_literal_leading_dot: !dotglob && !pattern_starts_dot,
                ..Default::default()
            };
            match glob::glob_with(&pattern, glob_opts) {
                Ok(paths) => {
                    let mut results: Vec<String> = paths
                        .filter_map(|p| p.ok())
                        .map(|p| p.to_string_lossy().to_string())
                        // Strip ./ prefix from glob results
                        .map(|s| {
                            if s.starts_with("./") && s.len() > 2 {
                                s[2..].to_string()
                            } else {
                                s
                            }
                        })
                        // Exclude . and .. when globskipdots is on (default)
                        .filter(|s| {
                            let skipdots = GLOBSKIPDOTS_ENABLED.with(|d| *d.borrow());
                            if skipdots {
                                s != "." && s != ".." && s != "./" && s != "../"
                            } else {
                                true
                            }
                        })
                        .collect();
                    apply_globignore(&mut results);
                    if results.is_empty() {
                        vec![remove_quotes(field)]
                    } else {
                        results.sort();
                        results
                    }
                }
                Err(_) => vec![remove_quotes(field)],
            }
        }
    } else {
        vec![remove_quotes(field)]
    }
}
