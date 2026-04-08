mod arithmetic;
mod params;
mod pattern;
pub(crate) mod transform_helpers;

use params::{
    apply_param_op, expand_param, get_array_elements, is_array_at_expansion, lookup_var,
    parse_arith_offset,
};
pub(crate) use pattern::char_in_class as pattern_char_in_class;
use pattern::{TrimMode, pattern_replace, shell_pattern_match, trim_pattern};

use crate::ast::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// Get the first character of IFS, distinguishing unset from empty.
/// - IFS unset → `Some(' ')` (default separator is space)
/// - IFS set to empty string → `None` (no separator)
/// - IFS set to non-empty → `Some(first_char)`
fn ifs_first_char(vars: &HashMap<String, String>) -> Option<char> {
    match vars.get("IFS") {
        None => Some(' '),               // IFS unset → default space
        Some(s) if s.is_empty() => None, // IFS="" → no separator
        Some(s) => s.chars().next(),     // IFS="x..." → first char
    }
}

/// Function type for evaluating command substitutions.
pub type CmdSubFn<'a> = &'a mut dyn FnMut(&str) -> String;

/// Owned snapshot of shell state, used to refresh ExpCtx after funsub execution.
/// Funsubs (`${ cmd; }`) and valuesubs (`${| cmd; }`) run in the current shell
/// and can modify positional params, variables, arrays, etc.  Subsequent word
/// parts in the same word must see those changes.
#[derive(Clone)]
pub struct FunsubState {
    pub vars: HashMap<String, String>,
    pub arrays: HashMap<String, Vec<Option<String>>>,
    pub assoc_arrays: HashMap<String, crate::interpreter::AssocArray>,
    pub namerefs: HashMap<String, String>,
    pub positional: Vec<String>,
    pub last_status: i32,
    pub opt_flags: String,
}

thread_local! {
    /// After a funsub/valuesub executes, the interpreter stores fresh shell
    /// state here so the expand module can pick it up and refresh its ExpCtx.
    static FUNSUB_UPDATED_STATE: RefCell<Option<FunsubState>> = const { RefCell::new(None) };

    /// File descriptors opened by process substitutions that need to be closed
    /// after the command using them completes.
    static PROCSUB_FDS: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Current script name for error messages from expansion code
    static SCRIPT_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    /// Flag set when arithmetic evaluation encounters an error.
    static ARITH_ERROR: RefCell<bool> = const { RefCell::new(false) };
    /// Flag set when a nounset (set -u) error occurs — should exit the shell/subshell.
    static NOUNSET_ERROR: RefCell<bool> = const { RefCell::new(false) };
    /// Flag set when a bad array subscript error was already printed during
    /// lookup_var.  This prevents duplicate errors from expand_param calling
    /// lookup_var again, but unlike ARITH_ERROR it does NOT abort the command
    /// (bash prints the error and still runs the command with empty expansion).
    static BAD_SUBSCRIPT: RefCell<bool> = const { RefCell::new(false) };
    /// RANDOM PRNG state (bash-compatible linear congruential generator)
    static RANDOM_STATE: RefCell<u32> = const { RefCell::new(0) };
    static RANDOM_SEEDED: RefCell<bool> = const { RefCell::new(false) };
    static RANDOM_LAST: RefCell<u32> = const { RefCell::new(0) };
    /// PID of the last process substitution child (for $!)
    static LAST_PROCSUB_PID: RefCell<Option<i32>> = const { RefCell::new(None) };
    /// Whether dotglob shopt is enabled (for glob expansion)
    static DOTGLOB_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Whether globskipdots shopt is enabled (skip . and .. in glob results)
    static GLOBSKIPDOTS_ENABLED: RefCell<bool> = const { RefCell::new(true) };
    /// Whether globstar shopt is enabled (** matches recursively)
    static GLOBSTAR_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Whether nullglob shopt is enabled (unmatched globs expand to nothing)
    static NULLGLOB_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Whether nocasematch shopt is enabled (case-insensitive pattern matching)
    static NOCASEMATCH_ENABLED: RefCell<bool> = const { RefCell::new(false) };
    /// Whether POSIX mode is enabled (disables glob expansion in $(< file*) etc.)
    static POSIX_MODE: RefCell<bool> = const { RefCell::new(false) };
    /// Whether patsub_replacement shopt is enabled (`&` in replacement = matched text)
    static PATSUB_REPLACEMENT: RefCell<bool> = const { RefCell::new(true) };
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

pub fn clear_procsub_runner() {
    PROCSUB_RUNNER.with(|r| {
        *r.borrow_mut() = None;
    });
}

/// Called by the interpreter after a funsub/valuesub executes to store
/// the updated shell state so subsequent word-part expansions see changes.
pub fn set_funsub_state(state: FunsubState) {
    FUNSUB_UPDATED_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
}

/// Take (and clear) any pending funsub state update.
pub fn take_funsub_state() -> Option<FunsubState> {
    FUNSUB_UPDATED_STATE.with(|s| s.borrow_mut().take())
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

pub fn set_nullglob(enabled: bool) {
    NULLGLOB_ENABLED.with(|d| *d.borrow_mut() = enabled);
}

pub fn set_nocasematch(enabled: bool) {
    NOCASEMATCH_ENABLED.with(|d| *d.borrow_mut() = enabled);
}

pub fn get_nocasematch() -> bool {
    NOCASEMATCH_ENABLED.with(|d| *d.borrow())
}

pub fn set_posix_mode(enabled: bool) {
    POSIX_MODE.with(|d| *d.borrow_mut() = enabled);
}

pub fn get_posix_mode() -> bool {
    POSIX_MODE.with(|d| *d.borrow())
}

pub fn set_patsub_replacement(enabled: bool) {
    PATSUB_REPLACEMENT.with(|d| *d.borrow_mut() = enabled);
}

pub fn get_patsub_replacement() -> bool {
    PATSUB_REPLACEMENT.with(|d| *d.borrow())
}

/// Seed the RANDOM PRNG (called when RANDOM=N is assigned)
pub fn seed_random(seed: u32) {
    RANDOM_STATE.with(|s| *s.borrow_mut() = seed);
    RANDOM_SEEDED.with(|s| *s.borrow_mut() = true);
    // Also reset last_random_value for the duplicate-avoidance loop
    RANDOM_LAST.with(|l| *l.borrow_mut() = seed);
}

/// Get next RANDOM value (bash-compatible Park-Miller PRNG with XOR mixing).
pub fn next_random() -> u16 {
    RANDOM_STATE.with(|s| {
        let mut state = s.borrow_mut();
        if !RANDOM_SEEDED.with(|f| *f.borrow()) {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u32;
            *state = t ^ std::process::id().wrapping_mul(2654435761);
            RANDOM_SEEDED.with(|f| *f.borrow_mut() = true);
            RANDOM_LAST.with(|l| *l.borrow_mut() = *state);
        }
        // brand() loop with duplicate avoidance
        let last = RANDOM_LAST.with(|l| *l.borrow());
        loop {
            if *state == 0 {
                *state = 123459876;
            }
            // Park-Miller PRNG
            let h = *state / 127773;
            let l = *state % 127773;
            let result = (16807i64 * l as i64) - (2836i64 * h as i64);
            *state = if result < 0 {
                (result + 0x7fffffff) as u32
            } else {
                result as u32
            };
            // XOR mixing: t = (r >> 16) ^ (r & 0xffff)
            let r = *state;
            let t = (r >> 16) ^ (r & 0xffff);
            if t != last {
                RANDOM_LAST.with(|l| *l.borrow_mut() = t);
                return (t & 32767) as u16;
            }
        }
    })
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

/// Peek at the arithmetic error flag without clearing it.
pub fn get_arith_error() -> bool {
    ARITH_ERROR.with(|f| *f.borrow())
}

pub fn set_bad_subscript() {
    BAD_SUBSCRIPT.with(|f| *f.borrow_mut() = true);
}

pub fn take_bad_subscript() -> bool {
    BAD_SUBSCRIPT.with(|f| std::mem::replace(&mut *f.borrow_mut(), false))
}

/// Check and clear the nounset error flag.
pub fn take_nounset_error() -> bool {
    NOUNSET_ERROR.with(|f| std::mem::replace(&mut *f.borrow_mut(), false))
}

/// Set the nounset error flag (signals that the shell/subshell should exit).
pub fn set_nounset_error() {
    NOUNSET_ERROR.with(|f| *f.borrow_mut() = true);
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
    /// Like SplitHere but from $* with null IFS.  In word-splitting contexts
    /// this forces a field break (like SplitHere).  In non-splitting contexts
    /// (assignments, double-quotes) this joins with IFS[0] — i.e. empty string
    /// when IFS is null — instead of space.
    SplitHereStar,
}

/// Expand a word into a list of strings (after word splitting and globbing).
#[allow(clippy::too_many_arguments)]
pub fn expand_word(
    word: &Word,
    vars: &HashMap<String, String>,
    arrays: &HashMap<String, Vec<Option<String>>>,
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
        // Check if word contains "$@" / "${@}" or "${arr[@]}" which expand to nothing with 0 elements
        let has_at_expansion = word.iter().any(|p| {
            if let WordPart::DoubleQuoted(parts) = p {
                parts.iter().any(|inner| {
                    matches!(inner, WordPart::Variable(n) if n == "@")
                        || matches!(inner, WordPart::Param(expr) if expr.name == "@" && matches!(expr.op, ParamOp::None))
                })
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
    arrays: &HashMap<String, Vec<Option<String>>>,
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
    // SplitHereStar uses IFS[0] as separator (empty when IFS is null).
    // SplitHere (from $@) always uses space.
    let star_sep = match vars.get("IFS") {
        None => " ",
        Some(s) if s.is_empty() => "",
        Some(s) => &s[..s.chars().next().unwrap().len_utf8()],
    };
    let result: String = segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
            Segment::SplitHereStar => star_sep,
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
    arrays: &HashMap<String, Vec<Option<String>>>,
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
            Segment::SplitHere | Segment::SplitHereStar => " ".to_string(),
        })
        .collect()
}

struct ExpCtx<'a> {
    vars: &'a HashMap<String, String>,
    arrays: &'a HashMap<String, Vec<Option<String>>>,
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
            "#" | "?" | "-" | "$" | "0" => return true,
            "!" => return self.last_bg_pid != 0,
            "@" | "*" => return self.positional.len() > 1,
            _ => {}
        }
        // Positional parameters ($1, $2, ...)
        if let Ok(n) = name.parse::<usize>() {
            return n < self.positional.len();
        }
        // Shell variables, arrays, or environment
        let resolved = self.resolve_nameref(name);
        // For bare names (no subscript), check if the name has a value.
        // If name is a subscripted reference like arr[@], arrays are checked elsewhere.
        // For bare array names, bash treats ${arr-default} as ${arr[0]-default},
        // so check if element [0] is set (not just if the array exists).
        if let Some(bracket) = name.find('[') {
            // Has subscript — check the specific element
            let base = &name[..bracket];
            let idx_str = &name[bracket + 1..name.len() - 1];
            let base_resolved = self.resolve_nameref(base);
            if idx_str == "@" || idx_str == "*" {
                // ${arr[@]-default}: set if array has any elements
                if let Some(arr) = self.arrays.get(&base_resolved) {
                    return arr.iter().any(|v| v.is_some());
                }
                if let Some(assoc) = self.assoc_arrays.get(&base_resolved) {
                    return !assoc.is_empty();
                }
                return false;
            }
            if let Some(arr) = self.arrays.get(&base_resolved)
                && let Ok(n) = idx_str.parse::<usize>()
            {
                return arr.get(n).is_some_and(|v| v.is_some());
            }
            if let Some(assoc) = self.assoc_arrays.get(&base_resolved) {
                return assoc.contains_key(idx_str);
            }
            return self.vars.contains_key(&base_resolved);
        }
        if self.vars.contains_key(&resolved) {
            return true;
        }
        // Bare array name: ${A-default} checks element [0] / key "0"
        if let Some(arr) = self.arrays.get(&resolved) {
            return arr.first().is_some_and(|v| v.is_some());
        }
        if let Some(assoc) = self.assoc_arrays.get(&resolved) {
            return assoc.contains_key("0");
        }
        std::env::var(&resolved).is_ok()
    }
}

fn expand_word_to_segments(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> Vec<Segment> {
    let mut segments = Vec::new();
    // After a funsub/valuesub executes, the shell state may have changed.
    // We need to refresh the ExpCtx so subsequent word parts see the updates.
    let mut funsub_owned: Option<FunsubState> = None;
    // Clear any stale funsub state before we start
    let _ = take_funsub_state();
    for part in word {
        let fresh_ctx;
        let effective_ctx = if let Some(ref state) = funsub_owned {
            fresh_ctx = ExpCtx {
                vars: &state.vars,
                arrays: &state.arrays,
                assoc_arrays: &state.assoc_arrays,
                namerefs: &state.namerefs,
                positional: &state.positional,
                last_status: state.last_status,
                last_bg_pid: ctx.last_bg_pid,
                top_level_pid: ctx.top_level_pid,
                opt_flags: &state.opt_flags,
            };
            &fresh_ctx
        } else {
            ctx
        };
        expand_part(part, effective_ctx, &mut segments, cmd_sub);
        // Check if a funsub/valuesub updated the shell state
        if let Some(new_state) = take_funsub_state() {
            funsub_owned = Some(new_state);
        }
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
            let mut dq_funsub_owned: Option<FunsubState> = None;
            for p in parts {
                // After a funsub/valuesub inside double quotes, refresh the
                // expansion context so subsequent parts see the updated state
                // (e.g. "$*${ set -- a b c;}$*" — second $* must see new params).
                let dq_fresh_ctx;
                #[allow(unused_assignments)]
                let mut ctx = ctx;
                if let Some(ref state) = dq_funsub_owned {
                    dq_fresh_ctx = ExpCtx {
                        vars: &state.vars,
                        arrays: &state.arrays,
                        assoc_arrays: &state.assoc_arrays,
                        namerefs: &state.namerefs,
                        positional: &state.positional,
                        last_status: state.last_status,
                        last_bg_pid: ctx.last_bg_pid,
                        top_level_pid: ctx.top_level_pid,
                        opt_flags: &state.opt_flags,
                    };
                    ctx = &dq_fresh_ctx;
                }
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
                    // "${@}" with braces — same behavior as "$@"
                    WordPart::Param(expr)
                        if expr.name == "@" && matches!(expr.op, ParamOp::None) =>
                    {
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
                    // "${*}" with braces — same behavior as "$*"
                    WordPart::Param(expr)
                        if expr.name == "*" && matches!(expr.op, ParamOp::None) =>
                    {
                        if ctx.positional.len() > 1 {
                            let sep = ifs_first_char(ctx.vars);
                            for (i, arg) in ctx.positional[1..].iter().enumerate() {
                                if i > 0
                                    && let Some(c) = sep
                                {
                                    s.push(c);
                                }
                                s.push_str(arg);
                            }
                        }
                    }
                    WordPart::Variable(name) => {
                        // Check if this variable is a nameref that resolves to
                        // arr[@] or arr[*] — if so, produce SplitHere markers
                        // (like "${arr[@]}") instead of a single joined string.
                        let resolved_nr = ctx.resolve_nameref(name);
                        let nameref_is_at_star = if resolved_nr != *name {
                            if let Some(bracket) = resolved_nr.find('[') {
                                let idx =
                                    &resolved_nr[bracket + 1..resolved_nr.len().saturating_sub(1)];
                                idx == "@" || idx == "*"
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if nameref_is_at_star {
                            let bracket = resolved_nr.find('[').unwrap();
                            let base = &resolved_nr[..bracket];
                            let idx = &resolved_nr[bracket + 1..resolved_nr.len() - 1];
                            let resolved_base = ctx.resolve_nameref(base);
                            let elements: Vec<String> =
                                if let Some(arr) = ctx.arrays.get(&resolved_base) {
                                    arr.iter().filter_map(|v| v.clone()).collect()
                                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved_base) {
                                    assoc.values().cloned().collect()
                                } else if let Some(val) = ctx.vars.get(&resolved_base) {
                                    vec![val.clone()]
                                } else {
                                    vec![]
                                };
                            if idx == "@" {
                                // Like "${arr[@]}" — each element is a separate field
                                if !elements.is_empty() {
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
                            } else {
                                // Like "${arr[*]}" — join with IFS first char
                                let sep = ifs_first_char(ctx.vars);
                                for (i, elem) in elements.iter().enumerate() {
                                    if i > 0
                                        && let Some(c) = sep
                                    {
                                        s.push(c);
                                    }
                                    s.push_str(elem);
                                }
                            }
                        } else {
                            let val = lookup_var(name, ctx);
                            let is_pos_unbound = if let Ok(n) = name.parse::<usize>() {
                                n > 0 && n >= ctx.positional.len()
                            } else {
                                false
                            };
                            if val.is_empty()
                                && ctx.opt_flags.contains('u')
                                && !matches!(name.as_str(), "?" | "$" | "#" | "@" | "*" | "-" | "0")
                                && !(name == "!" && ctx.last_bg_pid != 0)
                                && (is_pos_unbound
                                    || (name.parse::<usize>().is_err()
                                        && !ctx.vars.contains_key(name.as_str())
                                        && !ctx.arrays.contains_key(name.as_str())
                                        && std::env::var(name.as_str()).is_err()))
                            {
                                let sname = ctx
                                    .vars
                                    .get("_BASH_SOURCE_FILE")
                                    .or_else(|| ctx.positional.first())
                                    .map(|s| s.as_str())
                                    .unwrap_or("bash");
                                let lineno =
                                    ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                                // Include $ prefix only for positional params (numeric names)
                                if name.chars().all(|c| c.is_ascii_digit()) {
                                    eprintln!(
                                        "{}: line {}: ${}: unbound variable",
                                        sname, lineno, name
                                    );
                                } else {
                                    eprintln!(
                                        "{}: line {}: {}: unbound variable",
                                        sname, lineno, name
                                    );
                                }
                                set_arith_error();
                                set_nounset_error();
                            }
                            s.push_str(&val);
                        }
                    }
                    WordPart::Param(expr)
                        if (expr.name == "@" || expr.name == "*")
                            && !matches!(
                                expr.op,
                                ParamOp::None
                                    | ParamOp::Length
                                    | ParamOp::Substring(..)
                                    | ParamOp::Transform('A')
                                    | ParamOp::Transform('a')
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
                            ifs_first_char(ctx.vars)
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
                        // "${arr[@]@k}" — lowercase k produces separate words
                        // for each key-value pair (key and value as separate argv entries)
                        if matches!(&expr.op, ParamOp::Transform('k')) {
                            if let Some(bracket) = expr.name.find('[') {
                                let base = &expr.name[..bracket];
                                let resolved = ctx.resolve_nameref(base);
                                if !s.is_empty() {
                                    out.push(Segment::Quoted(std::mem::take(&mut s)));
                                }
                                let mut first = true;
                                if let Some(arr) = ctx.arrays.get(&resolved) {
                                    for (i, v) in arr.iter().enumerate() {
                                        if let Some(val) = v {
                                            if !first {
                                                out.push(Segment::SplitHere);
                                            }
                                            first = false;
                                            out.push(Segment::Quoted(i.to_string()));
                                            out.push(Segment::SplitHere);
                                            out.push(Segment::Quoted(val.clone()));
                                        }
                                    }
                                } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                                    for (k, v) in assoc.iter() {
                                        if !first {
                                            out.push(Segment::SplitHere);
                                        }
                                        first = false;
                                        out.push(Segment::Quoted(k.clone()));
                                        out.push(Segment::SplitHere);
                                        out.push(Segment::Quoted(v.clone()));
                                    }
                                }
                            }
                        } else {
                            // "${arr[@]}" — each element becomes a separate field
                            // For Substring, get_array_elements handles index-based slicing
                            // for all array types (indexed, assoc, and positional @/*)
                            let elements = get_array_elements(expr, ctx, cmd_sub);
                            // Determine if this is $* (join with IFS) or $@ (split)
                            let is_star = if let Some(bracket) = expr.name.find('[') {
                                &expr.name[bracket + 1..expr.name.len() - 1] == "*"
                            } else {
                                expr.name == "*"
                            };
                            if is_star {
                                // "${*:...}" or "${arr[*]:...}" — join with IFS[0]
                                let ifs_sep = ifs_first_char(ctx.vars);
                                for (i, elem) in elements.iter().enumerate() {
                                    if i > 0
                                        && let Some(c) = ifs_sep
                                    {
                                        s.push(c);
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
                    }
                    WordPart::Param(expr) => {
                        // Handle ${!var} indirect expansion where var=@ or var=*
                        // In double-quoted context, "${!foo}" where foo=@ should
                        // produce SplitHere markers like "$@" does.
                        if matches!(expr.op, ParamOp::Indirect) {
                            let target = lookup_var(&expr.name, ctx);
                            if target == "@" && ctx.positional.len() > 1 {
                                if !s.is_empty() {
                                    out.push(Segment::Quoted(std::mem::take(&mut s)));
                                }
                                for (i, arg) in ctx.positional[1..].iter().enumerate() {
                                    if i > 0 {
                                        out.push(Segment::SplitHere);
                                    }
                                    out.push(Segment::Quoted(arg.clone()));
                                }
                            } else if target == "*" && ctx.positional.len() > 1 {
                                let sep = ifs_first_char(ctx.vars);
                                for (i, arg) in ctx.positional[1..].iter().enumerate() {
                                    if i > 0
                                        && let Some(c) = sep
                                    {
                                        s.push(c);
                                    }
                                    s.push_str(arg);
                                }
                            } else if target.ends_with("[@]") {
                                let base = &target[..target.len() - 3];
                                let resolved = ctx.resolve_nameref(base);
                                if !s.is_empty() {
                                    out.push(Segment::Quoted(std::mem::take(&mut s)));
                                }
                                let elements: Vec<String> =
                                    if let Some(arr) = ctx.arrays.get(&resolved) {
                                        arr.iter().filter_map(|v| v.clone()).collect()
                                    } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                                        assoc.values().cloned().collect()
                                    } else if let Some(val) = ctx.vars.get(&resolved) {
                                        vec![val.clone()]
                                    } else {
                                        vec![]
                                    };
                                for (i, elem) in elements.iter().enumerate() {
                                    if i > 0 {
                                        out.push(Segment::SplitHere);
                                    }
                                    out.push(Segment::Quoted(elem.clone()));
                                }
                            } else if target.ends_with("[*]") {
                                let base = &target[..target.len() - 3];
                                let resolved = ctx.resolve_nameref(base);
                                let sep = ifs_first_char(ctx.vars);
                                let elements: Vec<String> =
                                    if let Some(arr) = ctx.arrays.get(&resolved) {
                                        arr.iter().filter_map(|v| v.clone()).collect()
                                    } else if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                                        assoc.values().cloned().collect()
                                    } else if let Some(val) = ctx.vars.get(&resolved) {
                                        vec![val.clone()]
                                    } else {
                                        vec![]
                                    };
                                for (i, elem) in elements.iter().enumerate() {
                                    if i > 0
                                        && let Some(c) = sep
                                    {
                                        s.push(c);
                                    }
                                    s.push_str(elem);
                                }
                            } else {
                                let val = expand_param(expr, ctx, cmd_sub);
                                s.push_str(&val);
                            }
                        } else {
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
                                if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op
                                {
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
                                            Segment::SplitHere | Segment::SplitHereStar => {
                                                out.push(Segment::Quoted(std::mem::take(&mut s)));
                                                out.push(seg.clone());
                                            }
                                        }
                                    }
                                }
                            } else {
                                s.push_str(&expand_param(expr, ctx, cmd_sub));
                            }
                        }
                    }
                    WordPart::CommandSub(cmd) | WordPart::FunSub(cmd) | WordPart::ValueSub(cmd) => {
                        // Determine nofork prefix for funsub/valuesub dispatch
                        let nofork_prefix = match p {
                            WordPart::FunSub(_) => "\x01FUNSUB:",
                            WordPart::ValueSub(_) => "\x01VALUESUB:",
                            _ => "",
                        };
                        if cmd.starts_with('\x00') {
                            // Incomplete comsub — mark for suppression
                            if !s.is_empty() {
                                out.push(Segment::Quoted(std::mem::take(&mut s)));
                            }
                            out.push(Segment::Unquoted(cmd.clone()));
                            continue;
                        }
                        let trimmed = cmd.trim();
                        if nofork_prefix.is_empty() {
                            if let Some(file) = trimmed
                                .strip_prefix("< ")
                                .or_else(|| trimmed.strip_prefix("<\t"))
                            {
                                let file = file.trim();
                                // Parse the filename into word parts so $var,
                                // ${var}, $(cmd), tilde etc. are expanded.
                                let file_parts = crate::lexer::lex_compound_array_content(file);
                                let expanded =
                                    expand_word_nosplit_ctx(&file_parts, ctx, &mut |c| cmd_sub(c));
                                let expanded = expanded.trim().to_string();
                                // Glob expansion (unless posix mode)
                                let resolved = if !get_posix_mode()
                                    && (expanded.contains('*')
                                        || expanded.contains('?')
                                        || expanded.contains('['))
                                {
                                    match glob::glob(&expanded) {
                                        Ok(mut paths) => {
                                            if let Some(Ok(p)) = paths.next() {
                                                if paths.next().is_none() {
                                                    p.to_string_lossy().to_string()
                                                } else {
                                                    expanded.clone()
                                                }
                                            } else {
                                                expanded.clone()
                                            }
                                        }
                                        Err(_) => expanded.clone(),
                                    }
                                } else {
                                    expanded.clone()
                                };
                                match std::fs::read_to_string(&resolved) {
                                    Ok(content) => {
                                        s.push_str(content.trim_end_matches('\n'));
                                    }
                                    Err(e) => {
                                        let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                                            let p = p.borrow();
                                            if p.is_empty() {
                                                "bash".to_string()
                                            } else {
                                                p.clone()
                                            }
                                        });
                                        let msg = if let Some(code) = e.raw_os_error() {
                                            let c_msg = unsafe { libc::strerror(code) };
                                            if c_msg.is_null() {
                                                e.to_string()
                                            } else {
                                                unsafe { std::ffi::CStr::from_ptr(c_msg) }
                                                    .to_string_lossy()
                                                    .to_string()
                                            }
                                        } else {
                                            e.to_string()
                                        };
                                        eprintln!("{}: {}: {}", prefix, resolved, msg);
                                        set_arith_error();
                                    }
                                }
                            } else {
                                s.push_str(&cmd_sub(cmd));
                            }
                        } else {
                            s.push_str(&cmd_sub(&format!("{}{}", nofork_prefix, cmd)));
                            // Funsub/valuesub may have modified shell state; pick up
                            // the refresh so remaining parts in this DoubleQuoted word
                            // see the updated positional params, variables, etc.
                            if let Some(new_state) = take_funsub_state() {
                                dq_funsub_owned = Some(new_state);
                            }
                        }
                    }
                    WordPart::BacktickSub(cmd) => {
                        s.push_str(&cmd_sub(cmd));
                    }
                    WordPart::ArithSub(expr) => {
                        s.push_str(&expand_arith(expr, ctx, cmd_sub));
                    }
                    _ => {
                        let mut inner = Vec::new();
                        expand_part(p, ctx, &mut inner, cmd_sub);
                        for seg in inner {
                            match seg {
                                Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => {
                                    s.push_str(&t)
                                }
                                Segment::SplitHere | Segment::SplitHereStar => {
                                    out.push(Segment::Quoted(std::mem::take(&mut s)));
                                    out.push(seg.clone());
                                }
                            }
                        }
                    }
                }
            }
            // Push Quoted for DoubleQuoted — even empty strings create a field.
            // EXCEPT when the word contains "$@" or "${@}" that expanded to nothing
            // (no positional params) AND the accumulated string is empty.
            // This covers "$@", "${@}", "$xxx${@}" (where $xxx is also empty), etc.
            let has_at = parts.iter().any(|p| {
                matches!(p, WordPart::Variable(n) if n == "@")
                    || matches!(p, WordPart::Param(expr) if expr.name == "@" && matches!(expr.op, ParamOp::None))
            });
            let at_was_empty = has_at && s.is_empty() && ctx.positional.len() <= 1;
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
            } else if let Some(dir) = expand_tilde_dirstack(user, ctx) {
                // ~N, ~+N, ~-N — directory stack indices
                dir
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
            let is_pos_unbound = if let Ok(n) = name.parse::<usize>() {
                n > 0 && n >= ctx.positional.len()
            } else {
                false
            };
            if val.is_empty()
                && ctx.opt_flags.contains('u')
                && !matches!(name.as_str(), "?" | "$" | "#" | "@" | "*" | "-" | "0")
                // $! is unbound when no background job has been started
                && !(name == "!" && ctx.last_bg_pid != 0)
                && (is_pos_unbound
                    || (name.parse::<usize>().is_err()
                        && !ctx.vars.contains_key(name.as_str())
                        && !ctx.arrays.contains_key(name.as_str())
                        && std::env::var(name.as_str()).is_err()))
            {
                let sname = ctx
                    .vars
                    .get("_BASH_SOURCE_FILE")
                    .or_else(|| ctx.positional.first())
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                let lineno = ctx.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                // Include $ prefix only for positional params (numeric names)
                if name.chars().all(|c| c.is_ascii_digit()) {
                    eprintln!("{}: line {}: ${}: unbound variable", sname, lineno, name);
                } else {
                    eprintln!("{}: line {}: {}: unbound variable", sname, lineno, name);
                }
                set_arith_error();
                set_nounset_error();
            }
            // Unquoted $@ produces separate words even with null IFS
            // (the splitting is inherent to $@, not dependent on IFS)
            // In bash 5.3, unquoted $* also produces separate words when IFS
            // is null (empty string) — the positional parameters are not
            // concatenated into a single word.  We use SplitHere so that
            // word_split honours the boundaries.  expand_word_nosplit_ctx
            // maps SplitHere to IFS[0] (empty when IFS is null) so that
            // assignment contexts like ${c=$*} still get "12" not "1 2".
            if name == "@" && ctx.positional.len() > 1 {
                for (i, arg) in ctx.positional[1..].iter().enumerate() {
                    if i > 0 {
                        out.push(Segment::SplitHere);
                    }
                    out.push(Segment::Unquoted(arg.clone()));
                }
            } else if name == "*"
                && ctx.positional.len() > 1
                && ctx.vars.get("IFS").map(|s| s.is_empty()).unwrap_or(false)
            {
                // IFS is set but empty: $* produces separate words (bash 5.3).
                // Use SplitHereStar so non-splitting contexts (assignments)
                // join with IFS[0] (empty) instead of space.
                for (i, arg) in ctx.positional[1..].iter().enumerate() {
                    if i > 0 {
                        out.push(Segment::SplitHereStar);
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
                    ParamOp::None
                        | ParamOp::Length
                        | ParamOp::Substring(..)
                        | ParamOp::Transform('A')
                        | ParamOp::Transform('a')
                )
                && ctx.positional.len() > 1
            {
                let mut first = true;
                for elem in &ctx.positional[1..] {
                    let result = apply_param_op(elem, &expr.op, ctx, cmd_sub, &expr.name);
                    // In unquoted context, skip empty results from per-element
                    // operations like ${@%%pattern} — bash removes them.
                    if result.is_empty() {
                        continue;
                    }
                    if !first {
                        out.push(if expr.name == "@" {
                            Segment::SplitHere
                        } else {
                            match ifs_first_char(ctx.vars) {
                                Some(c) => Segment::Unquoted(c.to_string()),
                                None => Segment::Unquoted(String::new()),
                            }
                        });
                    }
                    first = false;
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
                        ParamOp::None
                            | ParamOp::Length
                            | ParamOp::Substring(..)
                            | ParamOp::Transform('A')
                            | ParamOp::Transform('a')
                            | ParamOp::Transform('K')
                            | ParamOp::Transform('k')
                    )
                    && let Some(arr) = ctx.arrays.get(&resolved)
                {
                    // Apply operation to each element separately
                    let mut first = true;
                    for elem in arr.iter().filter_map(|v| v.as_ref()) {
                        // Create a temporary param expr for this single element
                        let single_expr = ParamExpr {
                            name: elem.clone(),
                            op: expr.op.clone(),
                        };
                        // Apply the operation using the element value directly
                        let result =
                            apply_param_op(elem, &single_expr.op, ctx, cmd_sub, &single_expr.name);
                        // In unquoted context, skip empty results so they don't
                        // produce empty fields (e.g. ${arr[@]%%pattern} where
                        // an element is fully removed).
                        if result.is_empty() {
                            continue;
                        }
                        if !first {
                            out.push(if idx == "@" {
                                Segment::SplitHere
                            } else {
                                match ifs_first_char(ctx.vars) {
                                    Some(c) => Segment::Unquoted(c.to_string()),
                                    None => Segment::Unquoted(String::new()),
                                }
                            });
                        }
                        first = false;
                        out.push(Segment::Unquoted(result));
                    }
                    return;
                }
                // Handle ${arr[@]:offset:length} — array slice
                if (idx == "@" || idx == "*")
                    && let ParamOp::Substring(offset_str, length_str) = &expr.op
                {
                    let offset: i64 =
                        parse_arith_offset(offset_str.trim(), &expr.name, ctx, cmd_sub);

                    // For indexed arrays, use index-based offset matching:
                    // ${arr[@]:N:L} selects set elements with array index >= N,
                    // then takes L of them. Negative offsets are relative to
                    // highest_index + 1 (the array Vec length).
                    if let Some(raw_arr) = ctx.arrays.get(&resolved) {
                        let set_elements: Vec<(usize, &str)> = raw_arr
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| v.as_ref().map(|s| (i, s.as_str())))
                            .collect();
                        let count = set_elements.len();
                        let effective_offset = if offset < 0 {
                            let arr_len = raw_arr.len() as i64;
                            (arr_len + offset).max(0)
                        } else {
                            offset
                        };
                        let start = set_elements
                            .iter()
                            .position(|(i, _)| *i >= effective_offset as usize)
                            .unwrap_or(count);
                        let end = if let Some(len_str) = length_str {
                            let len: i64 =
                                parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
                            if len < 0 {
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
                        let mut first = true;
                        for elem in &sliced {
                            if !first {
                                out.push(if idx == "@" {
                                    Segment::SplitHere
                                } else {
                                    match ifs_first_char(ctx.vars) {
                                        Some(c) => Segment::Unquoted(c.to_string()),
                                        None => Segment::Unquoted(String::new()),
                                    }
                                });
                            }
                            first = false;
                            out.push(Segment::Unquoted(elem.to_string()));
                        }
                        return;
                    }

                    // For assoc arrays, use list-position-based offset
                    if let Some(assoc) = ctx.assoc_arrays.get(&resolved) {
                        let arr: Vec<String> = assoc.values().cloned().collect();
                        let count = arr.len();
                        let start = if offset < 0 {
                            (count as i64 + offset).max(0) as usize
                        } else {
                            (offset as usize).min(count)
                        };
                        let end = if let Some(len_str) = length_str {
                            let len: i64 =
                                parse_arith_offset(len_str.trim(), &expr.name, ctx, cmd_sub);
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
                                    match ifs_first_char(ctx.vars) {
                                        Some(c) => Segment::Unquoted(c.to_string()),
                                        None => Segment::Unquoted(String::new()),
                                    }
                                });
                            }
                            first = false;
                            out.push(Segment::Unquoted(elem.clone()));
                        }
                        return;
                    }
                }
            }
            // For ${#arr[-N]} (Length op with negative out-of-bounds subscript),
            // check BEFORE calling lookup_var so we use the bash-specific
            // `[-N]: bad array subscript` error format (not `arr: bad ...`).
            if matches!(&expr.op, crate::ast::ParamOp::Length)
                && let Some(bracket) = expr.name.find('[')
            {
                let base = &expr.name[..bracket];
                let idx_str = &expr.name[bracket + 1..expr.name.len() - 1];
                let resolved = ctx.resolve_nameref(base);
                if idx_str != "@" && idx_str != "*" && !ctx.assoc_arrays.contains_key(&resolved) {
                    let raw_idx: i64 = if idx_str.trim().is_empty() {
                        0
                    } else if let Ok(v) = idx_str.trim().parse::<i64>() {
                        v
                    } else {
                        crate::expand::arithmetic::eval_arith_full_with_assoc(
                            idx_str,
                            ctx.vars,
                            ctx.arrays,
                            ctx.assoc_arrays,
                            ctx.namerefs,
                            ctx.positional,
                            ctx.last_status,
                            ctx.opt_flags,
                        )
                    };
                    if raw_idx < 0 {
                        let arr_len = ctx
                            .arrays
                            .get(&resolved)
                            .map(|a| a.len() as i64)
                            .unwrap_or(0);
                        if arr_len + raw_idx < 0 {
                            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                                let p = p.borrow();
                                if p.is_empty() {
                                    "bash".to_string()
                                } else {
                                    p.clone()
                                }
                            });
                            eprintln!("{}: [{}]: bad array subscript", prefix, raw_idx);
                            set_arith_error();
                            return;
                        }
                    }
                }
            }
            // For Default/Alt words in unquoted context, check if we should
            // expand per-part for mixed quoting (e.g., ${IFS+foo 'bar' baz})
            // Clear any pre-existing arith error so we only detect errors
            // from this specific lookup_var call.
            let had_prior_error = take_arith_error();
            let orig_val = lookup_var(&expr.name, ctx);
            // If lookup_var triggered a fatal arith error, bail out.
            if get_arith_error() {
                return;
            }
            // If lookup_var printed a bad-subscript warning (non-fatal),
            // consume the flag and skip calling expand_param again
            // (which would call lookup_var a second time, duplicating
            // the error message).  The expansion continues with empty val.
            let had_bad_sub = take_bad_subscript();
            // Restore prior error flag if there was one
            if had_prior_error {
                set_arith_error();
            }
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
            // - contains "$@" (quoted) which always needs separate fields
            // - contains $@ (unquoted) but only when IFS is null — with
            //   non-null IFS, unquoted $@ is space-joined then IFS-split
            let needs_per_part = if let ParamOp::Default(_, word) | ParamOp::Alt(_, word) = &expr.op
            {
                let has_literal = word.iter().any(|p| matches!(p, WordPart::Literal(_)));
                let has_quoted = word
                    .iter()
                    .any(|p| matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_)));
                let has_quoted_at = word.iter().any(|p| {
                        matches!(p, WordPart::DoubleQuoted(parts) if parts.iter().any(|ip| matches!(ip, WordPart::Variable(n) if n == "@")))
                    });
                let has_unquoted_at_or_star = word
                    .iter()
                    .any(|p| matches!(p, WordPart::Variable(n) if n == "@" || n == "*"));
                let ifs_is_null = ctx.vars.get("IFS").map(|s| s.is_empty()).unwrap_or(false);
                // Per-part expansion is needed when:
                // - mixed literal + quoted content (e.g. ${IFS+foo 'bar' baz})
                // - "$@" (quoted) in the word — always produces separate fields
                // - $@ or $* (unquoted) in the word AND IFS is null — with null
                //   IFS the positional params keep their word boundaries (bash 5.3)
                // With non-null IFS, unquoted $@ in ${var-$@} is space-joined
                // then IFS-split, so per-part expansion would be wrong.
                (has_literal && has_quoted)
                    || has_quoted_at
                    || (has_unquoted_at_or_star && ifs_is_null)
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
            } else if had_bad_sub {
                // Bad subscript already reported — use the empty orig_val
                // without calling expand_param (which would re-trigger the
                // lookup and print the error a second time).
                out.push(Segment::Unquoted(orig_val));
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
        WordPart::CommandSub(cmd) | WordPart::FunSub(cmd) | WordPart::ValueSub(cmd) => {
            // Check for incomplete comsub/funsub marker
            if cmd.starts_with('\x00') {
                out.push(Segment::Unquoted(cmd.clone()));
                return;
            }
            // Determine nofork prefix for funsub/valuesub dispatch
            let nofork_prefix = match part {
                WordPart::FunSub(_) => "\x01FUNSUB:",
                WordPart::ValueSub(_) => "\x01VALUESUB:",
                _ => "",
            };
            // Optimize $(< file) — read file content directly (only for regular comsub)
            let trimmed = cmd.trim();
            let val = if nofork_prefix.is_empty()
                && trimmed
                    .strip_prefix("< ")
                    .or_else(|| trimmed.strip_prefix("<\t"))
                    .is_some()
            {
                let file = trimmed
                    .strip_prefix("< ")
                    .or_else(|| trimmed.strip_prefix("<\t"))
                    .unwrap()
                    .trim();
                // Parse the filename into word parts so that $var, ${var},
                // $(cmd), tilde, etc. are expanded properly.
                let file_parts = crate::lexer::lex_compound_array_content(file);
                let expanded = expand_word_nosplit_ctx(&file_parts, ctx, &mut |c| cmd_sub(c));
                let expanded = expanded.trim().to_string();
                // Handle glob expansion in the filename (like bash does for
                // $(< $TMPDIR/bashtmp.x*)) — unless in posix mode where
                // glob expansion in redirections is disabled.
                let resolved = if !get_posix_mode()
                    && (expanded.contains('*') || expanded.contains('?') || expanded.contains('['))
                {
                    match glob::glob(&expanded) {
                        Ok(mut paths) => {
                            if let Some(Ok(p)) = paths.next() {
                                // Only use glob if exactly one match
                                if paths.next().is_none() {
                                    p.to_string_lossy().to_string()
                                } else {
                                    expanded.clone()
                                }
                            } else {
                                expanded.clone()
                            }
                        }
                        Err(_) => expanded.clone(),
                    }
                } else {
                    expanded.clone()
                };
                match std::fs::read_to_string(&resolved) {
                    Ok(content) => {
                        // Strip trailing newlines (like command substitution)
                        content.trim_end_matches('\n').to_string()
                    }
                    Err(e) => {
                        // Report error for non-existent files (bash does this).
                        // Use strerror-style message matching bash (no "os error N").
                        let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                            let p = p.borrow();
                            if p.is_empty() {
                                "bash".to_string()
                            } else {
                                p.clone()
                            }
                        });
                        let msg = if let Some(code) = e.raw_os_error() {
                            let c_msg = unsafe { libc::strerror(code) };
                            if c_msg.is_null() {
                                e.to_string()
                            } else {
                                unsafe { std::ffi::CStr::from_ptr(c_msg) }
                                    .to_string_lossy()
                                    .to_string()
                            }
                        } else {
                            e.to_string()
                        };
                        eprintln!("{}: {}: {}", prefix, resolved, msg);
                        // Signal failure so capture_output sees exit 1.
                        set_arith_error();
                        String::new()
                    }
                }
            } else if nofork_prefix.is_empty() {
                cmd_sub(cmd)
            } else {
                cmd_sub(&format!("{}{}", nofork_prefix, cmd))
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
            let val = expand_arith(expr, ctx, cmd_sub);
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
                        // Reset SIGPIPE to default so writes to closed pipes
                        // kill the child silently (matching bash behavior)
                        unsafe {
                            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
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
            set_arith_error(); // Signal expansion error to abort command
            // Push empty to avoid breaking segment collection
            out.push(Segment::Unquoted(String::new()));
        }
        WordPart::SyntaxError(msg) => {
            let prefix = EXPAND_ERROR_PREFIX.with(|p| {
                let p = p.borrow();
                if p.is_empty() {
                    "bash".to_string()
                } else {
                    p.clone()
                }
            });
            eprintln!("{}: {}", prefix, msg);
            set_arith_error(); // Signal expansion error to abort command
            out.push(Segment::Unquoted(String::new()));
        }
    }
}

/// Expand `~N`, `~+N`, `~-N` tilde prefixes using the DIRSTACK array.
///
/// - `~N` / `~+N` → `DIRSTACK[N]` (0 = current dir, counting from top)
/// - `~-N` → `DIRSTACK[len - 1 - N]` (counting from bottom)
///
/// Returns `None` if the pattern doesn't match or the index is out of range.
fn expand_tilde_dirstack(user: &str, ctx: &ExpCtx) -> Option<String> {
    let dirstack = ctx.arrays.get("DIRSTACK")?;
    let stack_len = dirstack.iter().filter(|v| v.is_some()).count();
    if stack_len == 0 {
        return None;
    }

    // Parse the index from the user string
    let (negative, num_str) = if let Some(rest) = user.strip_prefix('-') {
        // ~-N — count from the bottom of the stack
        (true, rest)
    } else if let Some(rest) = user.strip_prefix('+') {
        // ~+N — same as ~N, count from the top
        (false, rest)
    } else {
        // ~N — count from the top
        (false, user)
    };

    let idx: usize = num_str.parse().ok()?;

    let actual_idx = if negative {
        // ~-0 = last entry, ~-1 = second-to-last, etc.
        if idx >= stack_len {
            return None;
        }
        stack_len - 1 - idx
    } else {
        // ~0 = first entry (current dir), ~1 = second, etc.
        idx
    };

    dirstack.get(actual_idx).and_then(|v| v.as_ref()).cloned()
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
    // SplitHereStar uses IFS[0] as separator (empty when IFS is null).
    // SplitHere (from $@) always uses space.
    let star_sep = match ctx.vars.get("IFS") {
        None => " ",
        Some(s) if s.is_empty() => "",
        Some(s) => &s[..s.chars().next().unwrap().len_utf8()],
    };
    let result: String = segments
        .iter()
        .map(|s| match s {
            Segment::Quoted(t) | Segment::Unquoted(t) | Segment::Literal(t) => t.as_str(),
            Segment::SplitHere => " ",
            Segment::SplitHereStar => star_sep,
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
            Segment::SplitHere | Segment::SplitHereStar => result.push(' '),
        }
    }
    result
}

/// Like expand_word_nosplit_ctx but preserves quoting context for `&` in
/// replacement strings of `${var/pat/rep}`.  When `patsub_replacement` is
/// enabled, unquoted `&` means "matched text" while quoted `&` (inside
/// `"..."` or `'...'`) is literal.  We mark quoted `&` with a `\x00`
/// prefix so that `apply_replacement_amp` can distinguish them.
///
/// Also performs tilde expansion on the replacement: a leading unquoted
/// `~` (or `~/`) is expanded to `$HOME`, matching bash behavior.
fn expand_replacement_word(word: &Word, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    // Check whether the replacement word starts with an unquoted tilde.
    // Tilde expansion only happens when `~` is unquoted and at the very
    // start of the replacement word (optionally followed by `/`).
    let tilde_prefix = match word.first() {
        Some(WordPart::Literal(s)) if s.starts_with('~') => {
            // Check for ~/ or lone ~
            let rest = &s[1..];
            if rest.is_empty() || rest.starts_with('/') {
                ctx.vars.get("HOME").cloned()
            } else {
                None
            }
        }
        Some(WordPart::Tilde(user)) if user.is_empty() => ctx.vars.get("HOME").cloned(),
        _ => None,
    };

    let patsub = get_patsub_replacement();
    if !patsub {
        // When patsub_replacement is off, `&` has no special meaning —
        // just do a normal expansion (no markers needed).
        let val = expand_word_nosplit_ctx(word, ctx, cmd_sub);
        return apply_tilde_in_replacement(&val, tilde_prefix.as_deref());
    }

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
    let star_sep = match ctx.vars.get("IFS") {
        None => " ",
        Some(s) if s.is_empty() => "",
        Some(s) => &s[..s.chars().next().unwrap().len_utf8()],
    };
    let mut result = String::new();
    for s in &segments {
        match s {
            Segment::Quoted(t) => {
                // Quoted context: mark `&` and `\` with `\x00` prefix so
                // they stay literal.  Without marking `\`, a quoted `\`
                // followed by an unquoted `&` would be misparsed as `\&`
                // (escaped-amp) in apply_replacement_amp.
                for ch in t.chars() {
                    if ch == '&' || ch == '\\' {
                        result.push('\x00');
                        result.push(ch);
                    } else if ch == '\x00' {
                        // Strip existing \x00 markers
                    } else {
                        result.push(ch);
                    }
                }
            }
            Segment::Unquoted(t) | Segment::Literal(t) => {
                // Unquoted context: `&` stays bare (will be matched-text),
                // `\x00` markers are stripped.
                result.push_str(&t.replace('\x00', ""));
            }
            Segment::SplitHere => result.push(' '),
            Segment::SplitHereStar => result.push_str(star_sep),
        }
    }
    apply_tilde_in_replacement(&result, tilde_prefix.as_deref())
}

/// Apply tilde expansion to a replacement string value.
/// Only expands if `tilde_home` is `Some` (meaning the word started with
/// an unquoted `~`).  Replaces the leading `~` with the home directory.
fn apply_tilde_in_replacement(val: &str, tilde_home: Option<&str>) -> String {
    if let Some(home) = tilde_home
        && let Some(rest) = val.strip_prefix('~')
    {
        return format!("{}{}", home, rest);
    }
    val.to_string()
}

fn expand_arith(expr: &str, ctx: &ExpCtx, cmd_sub: CmdSubFn) -> String {
    // Pre-expand command substitutions ($(...) and ${ ...; }) in the
    // arithmetic expression before evaluating it.  eval_arith_full only
    // handles variable references, not command/funsub substitutions.
    let expanded;
    let expr = if expr.contains("$(")
        || expr.contains("${ ")
        || expr.contains("${|")
        || expr.contains('`')
    {
        expanded = expand_comsubs_in_arith_expr(expr, cmd_sub);
        &expanded
    } else {
        expr
    };
    let result = crate::expand::arithmetic::eval_arith_full_with_assoc(
        expr,
        ctx.vars,
        ctx.arrays,
        ctx.assoc_arrays,
        ctx.namerefs,
        ctx.positional,
        ctx.last_status,
        ctx.opt_flags,
    );
    result.to_string()
}

/// Expand command substitutions and funsubs within an arithmetic expression string.
/// This is used by the expand layer (which doesn't have Shell access) to pre-process
/// $(...), ${ ...; }, and ${| ...; } before passing to eval_arith_full.
fn expand_comsubs_in_arith_expr(expr: &str, cmd_sub: CmdSubFn) -> String {
    let chars: Vec<char> = expr.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '$' {
            // Escaped $ — keep as-is
            result.push('\\');
            result.push('$');
            i += 2;
            continue;
        }
        if chars[i] == '$' && i + 1 < chars.len() {
            // ${ cmd; } funsub or ${| cmd; } valuesub
            if chars[i + 1] == '{'
                && i + 2 < chars.len()
                && matches!(chars[i + 2], ' ' | '\t' | '\n' | '|')
            {
                let is_valuesub = chars[i + 2] == '|';
                let start = if is_valuesub { i + 3 } else { i + 2 };
                let mut depth = 1i32;
                let mut j = start;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        '\'' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '\'' {
                                j += 1;
                            }
                        }
                        '"' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '"' {
                                if chars[j] == '\\' && j + 1 < chars.len() {
                                    j += 1;
                                }
                                j += 1;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let cmd: String = chars[start..j].iter().collect();
                let prefix = if is_valuesub {
                    "\x01VALUESUB:"
                } else {
                    "\x01FUNSUB:"
                };
                let output = cmd_sub(&format!("{}{}", prefix, cmd));
                result.push_str(output.trim());
                i = j + 1; // skip past '}'
                continue;
            }
            // $(...) regular comsub
            if chars[i + 1] == '(' && !(i + 2 < chars.len() && chars[i + 2] == '(') {
                let mut depth = 1i32;
                let mut j = i + 2;
                while j < chars.len() && depth > 0 {
                    match chars[j] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        '\'' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '\'' {
                                j += 1;
                            }
                        }
                        '"' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '"' {
                                if chars[j] == '\\' && j + 1 < chars.len() {
                                    j += 1;
                                }
                                j += 1;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                let cmd: String = chars[i + 2..j - 1].iter().collect();
                let output = cmd_sub(&cmd);
                result.push_str(output.trim());
                i = j;
                continue;
            }
            // `backtick`
        }
        if chars[i] == '`' {
            i += 1;
            let mut cmd = String::new();
            while i < chars.len() && chars[i] != '`' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    cmd.push(chars[i + 1]);
                    i += 2;
                } else {
                    cmd.push(chars[i]);
                    i += 1;
                }
            }
            if i < chars.len() {
                i += 1;
            } // skip closing `
            let output = cmd_sub(&cmd);
            result.push_str(output.trim());
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

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
    let has_split = segments
        .iter()
        .any(|s| matches!(s, Segment::SplitHere | Segment::SplitHereStar));
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
                Segment::SplitHere | Segment::SplitHereStar => String::new(),
            })
            .collect();
        return vec![s];
    }

    let mut fields: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut has_quoted_since_split = false;

    for segment in segments {
        match segment {
            Segment::SplitHere | Segment::SplitHereStar => {
                // Force a field break here (for "$@", "${arr[@]}", $* with null IFS)
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
                // Build a set of "decoded" IFS chars: for each PUA-encoded
                // char in IFS, also accept the non-PUA form (and vice versa).
                // This allows control chars from pipe output (U+0001) to match
                // PUA-encoded IFS ($'\001' → U+E001) without re-encoding all
                // command substitution output (which would break character
                // class tests in posixexp).
                let ifs_match = |ch: char, ifs_set: &[char]| -> bool {
                    if ifs_set.contains(&ch) {
                        return true;
                    }
                    let cp = ch as u32;
                    // If ch is a raw control char (U+0001-U+001F, U+007F, U+0080-U+00FF),
                    // check if the PUA-encoded form is in IFS
                    if (1..=0xFF).contains(&cp) && !ch.is_whitespace() {
                        let pua = crate::builtins::raw_byte_char(cp as u8);
                        if ifs_set.contains(&pua) {
                            return true;
                        }
                    }
                    // If ch is a PUA-encoded char, check if the decoded form is in IFS
                    if crate::builtins::is_pua_raw_byte(cp) {
                        let decoded =
                            char::from_u32(cp - crate::builtins::RAW_BYTE_BASE).unwrap_or(ch);
                        if ifs_set.contains(&decoded) {
                            return true;
                        }
                    }
                    false
                };
                // If we have accumulated content from Quoted/Literal segments,
                // start in "in field" state so IFS whitespace causes a split
                let mut state: u8 = if !current.is_empty() || has_quoted_since_split {
                    1
                } else {
                    0
                };
                for ch in s.chars() {
                    if ifs_match(ch, &ifs_non_ws) {
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
                    } else if ifs_match(ch, &ifs_ws) {
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
                        WordPart::FunSub(cmd) => {
                            raw.push_str("${");
                            raw.push_str(cmd);
                            raw.push('}');
                        }
                        WordPart::ValueSub(cmd) => {
                            raw.push_str("${|");
                            raw.push_str(cmd);
                            raw.push('}');
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
            WordPart::FunSub(cmd) => {
                raw.push_str("${");
                raw.push_str(cmd);
                raw.push('}');
            }
            WordPart::ValueSub(cmd) => {
                raw.push_str("${|");
                raw.push_str(cmd);
                raw.push('}');
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

pub(crate) fn brace_expand(s: &str) -> Vec<String> {
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
                let file_type = entry.file_type().ok();
                let is_symlink = file_type.as_ref().map(|t| t.is_symlink()).unwrap_or(false);
                // Follow symlinks to determine if the target is a directory
                let is_dir = if is_symlink {
                    std::fs::metadata(entry.path())
                        .map(|m| m.is_dir())
                        .unwrap_or(false)
                } else {
                    file_type.map(|t| t.is_dir()).unwrap_or(false)
                };
                entries.push((path.clone(), is_dir));
                // Recurse into real directories but NOT symlinked directories
                if is_dir && !is_symlink {
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
    let original_pattern = pattern.to_string();
    let pattern = {
        let mut normalized = pattern.to_string();
        while normalized.contains("**/**") {
            normalized = normalized.replace("**/**", "**");
        }
        normalized
    };
    let was_collapsed = original_pattern != pattern;

    // Determine base directory from the prefix before the first **
    let first_star_pos = {
        let chars: Vec<char> = pattern.chars().collect();
        let mut pos = 0;
        let mut i = 0;
        let mut found = false;
        while i < chars.len().saturating_sub(1) {
            if chars[i] == '\x00' {
                i += 2;
                continue;
            }
            if chars[i] == '*' && chars[i + 1] == '*' {
                pos = i;
                found = true;
                break;
            }
            i += 1;
        }
        if found { pos } else { 0 }
    };

    let prefix_pat: String = pattern.chars().take(first_star_pos).collect();
    let base = prefix_pat.trim_end_matches('/');
    let base_dir = if base.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        std::path::PathBuf::from(base)
    };

    if !base_dir.exists() {
        return vec![pattern.to_string()];
    }

    let dir_only = pattern.ends_with('/');
    let all_entries = walk_dir(&base_dir, base, dotglob);

    // Split pattern into segments by '/', filtering empty segments
    let pat_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

    // Count the number of ways a path matches the pattern (bash produces
    // duplicates for multi-** patterns)
    fn count_matches(path_segs: &[&str], pat_segs: &[&str], pi: usize, si: usize) -> usize {
        // Both exhausted = one match
        if si >= pat_segs.len() {
            return if pi >= path_segs.len() { 1 } else { 0 };
        }
        // Pattern segments remain but path exhausted
        if pi >= path_segs.len() {
            return if pat_segs[si..].iter().all(|s| *s == "**") {
                1
            } else {
                0
            };
        }
        if pat_segs[si] == "**" {
            // ** matches zero or more path segments
            let mut total = 0;
            for skip in 0..=(path_segs.len() - pi) {
                total += count_matches(path_segs, pat_segs, pi + skip, si + 1);
            }
            total
        } else {
            // Match single segment with glob pattern
            let glob_opts = glob::MatchOptions {
                require_literal_separator: true,
                ..Default::default()
            };
            if let Ok(p) = glob::Pattern::new(pat_segs[si])
                && p.matches_with(path_segs[pi], glob_opts)
            {
                return count_matches(path_segs, pat_segs, pi + 1, si + 1);
            }
            0
        }
    }

    let mut results = Vec::new();

    // For patterns like "prefix/**", also include the base directory itself
    let is_suffix_only_stars = {
        let after_base = if base.is_empty() {
            &pattern
        } else {
            pattern
                .strip_prefix(base)
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(&pattern)
        };
        after_base.chars().all(|c| c == '*' || c == '/')
    };
    if !base.is_empty() && is_suffix_only_stars {
        // a/** → include "a/" (trailing slash)
        // a/**/** → include "a" (no slash, because multi-** collapsed means
        // ** matching zero segments gives just the directory name)
        if was_collapsed {
            results.push(base.to_string());
        } else {
            results.push(format!("{}/", base));
        }
    }

    for (path, is_dir) in &all_entries {
        if dir_only && !is_dir {
            continue;
        }
        let path_to_check = if dir_only {
            format!("{}/", path)
        } else {
            path.clone()
        };
        let path_segs: Vec<&str> = path_to_check.split('/').filter(|s| !s.is_empty()).collect();
        if count_matches(&path_segs, &pat_segments, 0, 0) > 0 {
            // Count how many ways the pattern matches (bash produces duplicates
            // for multi-** patterns where each ** can consume different segments)
            let match_count = count_matches(&path_segs, &pat_segments, 0, 0);
            for _ in 0..match_count {
                if dir_only {
                    results.push(format!("{}/", path));
                } else {
                    results.push(path.clone());
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
        // Check if the original field starts with "./" (possibly with \x00 quote
        // markers).  Bash preserves the "./" prefix in glob results when the user
        // wrote it, but the `glob` crate normalises it away.
        let field_starts_dot_slash = field.starts_with("./") || field.starts_with("\x00./") || {
            let mut fi = field.chars().peekable();
            let mut prefix = String::new();
            while let Some(&c) = fi.peek() {
                if c == '\x00' {
                    fi.next();
                    fi.next();
                    continue;
                }
                prefix.push(c);
                fi.next();
                if prefix == "./" {
                    break;
                }
                if prefix.len() >= 2 {
                    break;
                }
            }
            prefix == "./"
        };
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
                    let nullglob = NULLGLOB_ENABLED.with(|d| *d.borrow());
                    if results.is_empty() {
                        if nullglob {
                            vec![]
                        } else {
                            vec![remove_quotes(field)]
                        }
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
                        let nullglob = NULLGLOB_ENABLED.with(|d| *d.borrow());
                        if results.is_empty() {
                            if nullglob {
                                vec![]
                            } else {
                                vec![remove_quotes(field)]
                            }
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
                        // The glob crate normalises "./" away from results.
                        // When the original pattern started with "./", re-add
                        // the prefix so output matches bash behaviour.
                        // When the pattern did NOT start with "./" but the
                        // glob crate returned "./" (e.g. pattern "*/foo"),
                        // strip it like before.
                        .map(|s| {
                            if field_starts_dot_slash && !s.starts_with("./") {
                                format!("./{}", s)
                            } else if !field_starts_dot_slash && s.starts_with("./") && s.len() > 2
                            {
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
                    let nullglob = NULLGLOB_ENABLED.with(|d| *d.borrow());
                    if results.is_empty() {
                        if nullglob {
                            vec![]
                        } else {
                            vec![remove_quotes(field)]
                        }
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
