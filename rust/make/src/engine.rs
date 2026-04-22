//! Make execution engine: variable storage, rule database, and build logic.

use crate::ast::*;
use crate::expand;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Origin of a variable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VarOrigin {
    Undefined,
    Default,
    Environment,
    File,
    CommandLine,
    Override,
    Automatic,
}

impl std::fmt::Display for VarOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VarOrigin::Undefined => write!(f, "undefined"),
            VarOrigin::Default => write!(f, "default"),
            VarOrigin::Environment => write!(f, "environment"),
            VarOrigin::File => write!(f, "file"),
            VarOrigin::CommandLine => write!(f, "command line"),
            VarOrigin::Override => write!(f, "override"),
            VarOrigin::Automatic => write!(f, "automatic"),
        }
    }
}

/// Variable flavor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VarFlavor {
    Recursive,
    Simple,
    Undefined,
}

impl std::fmt::Display for VarFlavor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VarFlavor::Recursive => write!(f, "recursive"),
            VarFlavor::Simple => write!(f, "simple"),
            VarFlavor::Undefined => write!(f, "undefined"),
        }
    }
}

#[derive(Clone)]
pub struct Variable {
    pub value: String,
    pub flavor: VarFlavor,
    pub origin: VarOrigin,
}

/// A stored rule.
#[derive(Debug, Clone)]
struct RuleEntry {
    prerequisites: Vec<String>,
    order_only: Vec<String>,
    recipe: Vec<String>,
    recipe_lines: Vec<usize>,
    source_name: String,
    #[allow(dead_code)]
    is_double_colon: bool,
    /// Other targets in the same `&:` grouped rule. When this rule
    /// runs its recipe, all listed targets are treated as built.
    group: Vec<String>,
    /// Stem from a static pattern rule (the part matching `%` in the
    /// target-pattern). Used to set `$*` during recipe execution.
    stem: Option<String>,
    /// True if `.SECONDEXPANSION:` was active when this rule was defined.
    second_expand: bool,
    /// Raw (first-pass-expanded) prereq text for second expansion.
    raw_prereq_text: Option<String>,
    /// Raw (first-pass-expanded) order-only text for second expansion.
    raw_order_only_text: Option<String>,
}

/// A pattern rule entry.
#[derive(Debug, Clone)]
struct PatternRuleEntry {
    target_pattern: String,
    prereq_patterns: Vec<String>,
    order_only_patterns: Vec<String>,
    recipe: Vec<String>,
    recipe_lines: Vec<usize>,
    source_name: String,
    /// Double-colon pattern rules are "terminal": they only match when
    /// prerequisites actually exist (or "should exist" because they are
    /// mentioned as an explicit prerequisite of a non-pattern rule).
    is_terminal: bool,
    /// Other target patterns from the same multi-target pattern rule.
    /// After the recipe fires, targets derived from these patterns
    /// (with the same stem) are considered built too.
    sibling_patterns: Vec<String>,
    /// True if `.SECONDEXPANSION:` was active when this rule was defined.
    second_expand: bool,
    /// Raw prereq pattern text (joined, first-pass-expanded) for SE.
    raw_prereq_text: Option<String>,
    /// Raw order-only pattern text for SE.
    raw_order_only_text: Option<String>,
    /// True if registered from `&:` grouped-target pattern rule. For
    /// such rules, all sibling targets are produced together and we
    /// don't warn when the recipe doesn't update individual files.
    is_grouped: bool,
}

/// Unescape `\\:` and `\\#` in target/prereq names per GNU make rules.
fn contains_unescaped_colon(t: &str) -> bool {
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // Count preceding backslashes
            let mut k = 0;
            while i > k && bytes[i - 1 - k] == b'\\' {
                k += 1;
            }
            if k % 2 == 0 {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn unescape_name(t: &str) -> String {
    let bytes = t.as_bytes();
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b'\\' {
                j += 1;
            }
            let n = j - i;
            if j < bytes.len() && (bytes[j] == b':' || bytes[j] == b'#') {
                for _ in 0..(n / 2) {
                    out.push('\\');
                }
                if n % 2 == 1 {
                    out.push(bytes[j] as char);
                    i = j + 1;
                } else {
                    i = j;
                }
                continue;
            } else {
                for _ in 0..n {
                    out.push('\\');
                }
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Replace only the FIRST `%` in each whitespace-token of `text`,
/// recursing into `$(...)` / `${...}` groups so each whitespace word
/// inside a function call also gets its first `%` replaced. This
/// matches GNU make's static-pattern stem substitution semantics:
/// the literal `%` placeholder is per-word, and `$(wordlist ... %.1 %.2)`
/// has both `%`s replaced because each is its own word inside the call.
fn replace_first_percent_per_token(text: &str, stem: &str, dir_transfer: bool) -> String {
    fn process(s: &str, stem: &str, dir_transfer: bool) -> String {
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        let mut tok = String::new();
        let flush = |tok: &mut String, out: &mut String, stem: &str, dir_transfer: bool| {
            if !tok.is_empty() {
                // Apply GNU make directory-transfer semantics for implicit
                // pattern rules: when stem contains '/', substitute only
                // the base part for '%' and prepend the directory. Tokens
                // without '%' are left as-is. Static pattern rules do NOT
                // use directory transfer — the full stem is substituted.
                if dir_transfer {
                    if let Some(slash_pos) = stem.rfind('/') {
                        if expand::find_unescaped_percent(tok).is_some() {
                            let dir = &stem[..=slash_pos];
                            let base = &stem[slash_pos + 1..];
                            let substituted = expand::replace_first_unescaped_percent(tok, base);
                            out.push_str(dir);
                            out.push_str(&substituted);
                        } else {
                            out.push_str(&expand::unescape_percent(tok));
                        }
                    } else {
                        out.push_str(&expand::replace_first_unescaped_percent(tok, stem));
                    }
                } else {
                    out.push_str(&expand::replace_first_unescaped_percent(tok, stem));
                }
                tok.clear();
            }
        };
        while i < bytes.len() {
            let c = bytes[i];
            // Detect `$(` / `${` and recurse on the inner contents.
            if c == b'$' && i + 1 < bytes.len() {
                let n = bytes[i + 1];
                if n == b'(' || n == b'{' {
                    let open = n;
                    let close = if open == b'(' { b')' } else { b'}' };
                    // Find matching close.
                    let mut depth = 1i32;
                    let mut j = i + 2;
                    while j < bytes.len() && depth > 0 {
                        let cc = bytes[j];
                        if cc == b'$' && j + 1 < bytes.len() {
                            let nn = bytes[j + 1];
                            if nn == open {
                                depth += 1;
                                j += 2;
                                continue;
                            }
                        }
                        if cc == open {
                            depth += 1;
                        } else if cc == close {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        j += 1;
                    }
                    flush(&mut tok, &mut out, stem, dir_transfer);
                    out.push(b'$' as char);
                    out.push(open as char);
                    let inner = &s[i + 2..j];
                    out.push_str(&process(inner, stem, dir_transfer));
                    out.push(close as char);
                    i = j + 1;
                    continue;
                }
                // `$$` or `$X` etc. Consume both chars literally as part of token.
                tok.push(c as char);
                tok.push(n as char);
                i += 2;
                continue;
            }
            if c.is_ascii_whitespace() || c == b',' {
                flush(&mut tok, &mut out, stem, dir_transfer);
                out.push(c as char);
                i += 1;
                continue;
            }
            tok.push(c as char);
            i += 1;
        }
        flush(&mut tok, &mut out, stem, dir_transfer);
        out
    }
    process(text, stem, dir_transfer)
}

/// Find the first `|` that's outside `$(...)`/`${...}` and not the
/// `$|` auto-var reference. Used to split SE-expanded prereq text
/// into normal vs order-only halves.
fn find_orderonly_pipe(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() {
            let nxt = bytes[i + 1];
            if nxt == b'(' {
                paren_depth += 1;
                i += 2;
                continue;
            }
            if nxt == b'{' {
                brace_depth += 1;
                i += 2;
                continue;
            }
            i += 2;
            continue;
        }
        if c == b'(' && paren_depth > 0 {
            paren_depth += 1;
        } else if c == b')' && paren_depth > 0 {
            paren_depth -= 1;
        } else if c == b'{' && brace_depth > 0 {
            brace_depth += 1;
        } else if c == b'}' && brace_depth > 0 {
            brace_depth -= 1;
        } else if c == b'|' && paren_depth == 0 && brace_depth == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn validate_balanced_refs(text: &str, source: &str, line_no: usize) {
    let bytes = text.as_bytes();
    // stack entries: (open_kind: '(' or '{', name: String)
    let mut stack: Vec<(u8, String)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'$' && i + 1 < bytes.len() {
            let n = bytes[i + 1];
            if n == b'$' {
                i += 2;
                continue;
            }
            if n == b'(' || n == b'{' {
                // read first whitespace-terminated word after the open
                let start = i + 2;
                let mut j = start;
                while j < bytes.len() {
                    let cj = bytes[j];
                    if cj == b' '
                        || cj == b'\t'
                        || cj == b'\n'
                        || cj == b'('
                        || cj == b')'
                        || cj == b'{'
                        || cj == b'}'
                        || cj == b':'
                        || cj == b','
                        || cj == b'$'
                    {
                        break;
                    }
                    j += 1;
                }
                let name = std::str::from_utf8(&bytes[start..j])
                    .unwrap_or("")
                    .to_string();
                stack.push((n, name));
                i += 2;
                continue;
            }
            i += 2;
            continue;
        }
        if c == b'(' {
            if let Some(top) = stack.last()
                && top.0 == b'('
            {
                // nested paren inside a function arg — push sentinel
                stack.push((b'(', String::new()));
            }
        } else if c == b')' {
            if let Some(top) = stack.last()
                && top.0 == b'('
            {
                stack.pop();
            }
        } else if c == b'{' {
            if let Some(top) = stack.last()
                && top.0 == b'{'
            {
                stack.push((b'{', String::new()));
            }
        } else if c == b'}'
            && let Some(top) = stack.last()
            && top.0 == b'{'
        {
            stack.pop();
        }
        i += 1;
    }
    if let Some((kind, name)) = stack.last() {
        let close = if *kind == b'(' { ')' } else { '}' };
        let is_func = expand::is_builtin_function(name);
        let msg = if is_func {
            format!("*** unterminated call to function '{name}': missing '{close}'.  Stop.")
        } else {
            "*** unterminated variable reference.  Stop.".to_string()
        };
        if source.is_empty() {
            eprintln!("make: {msg}");
        } else {
            eprintln!("{source}:{line_no}: {msg}");
        }
        std::process::exit(2);
    }
}

pub struct Engine {
    pub vars: RefCell<HashMap<String, Variable>>,
    rules: RefCell<HashMap<String, Vec<RuleEntry>>>,
    pattern_rules: RefCell<Vec<PatternRuleEntry>>,
    default_goal: RefCell<Option<String>>,
    phony_targets: RefCell<HashSet<String>>,
    suffixes: RefCell<Vec<String>>,
    pub exports: RefCell<HashSet<String>>,
    /// Names explicitly `unexport`ed. Only consulted when
    /// `export_all` is true (global `export` / `.EXPORT_ALL_VARIABLES`)
    /// so a later `unexport BOTZ` can suppress the blanket export
    /// without affecting vars exported via an explicit `export`.
    unexports: RefCell<HashSet<String>>,
    export_all: RefCell<bool>,
    built_targets: RefCell<HashSet<String>>,
    /// Targets that were marked as built because a sibling in their
    /// `&:` grouped rule had its recipe run. Used to distinguish
    /// "built by group" from "built by own recipe" in the top-level
    /// build loop so the correct diagnostic message is emitted.
    group_built_targets: RefCell<HashSet<String>>,
    /// Keys identifying grouped-target rules whose recipe has already
    /// been executed. For double-colon grouped rules, this prevents
    /// re-running the same recipe for each member of the group.
    group_recipe_done: RefCell<HashSet<String>>,
    eval_queue: RefCell<Vec<String>>,
    /// Includes that failed to load (file missing). Retried after the
    /// top-level parse completes so rules defined later in the same
    /// makefile can build the missing include.
    pub pending_includes: RefCell<Vec<(String, bool, String, usize)>>,
    /// All files included via `include`/`-include` — tracked for
    /// include-file remaking. Even files that loaded successfully need
    /// their build rules checked (GNU make restarts after remaking).
    included_files: RefCell<Vec<String>>,
    /// Names of variables originally inherited from the environment.
    /// We track these separately from `VarOrigin` so we can re-export
    /// them to child processes even after a makefile assignment has
    /// changed their value (and consequently their origin).
    pub env_inherited: RefCell<HashSet<String>>,
    /// Names of variables assigned with `:::=`. Such vars are simply
    /// expanded but a later `+=` appends text verbatim (recursive
    /// semantics), matching GNU make.
    immediate_recursive: RefCell<HashSet<String>>,
    /// Recursion depth into `$(shell …)`. Used to break infinite
    /// loops when re-exporting a recursive env-inherited variable
    /// whose body itself calls `$(shell …)` — at depth > 0 we skip
    /// re-expanding recursive bodies and fall back to the raw value.
    pub shell_depth: RefCell<usize>,
    /// Targets of the most recently processed explicit rule. Used to
    /// attach `Directive::RecipeLine` entries that appear in a taken
    /// conditional branch after a rule (e.g. `all:\nifeq…\n\t@echo\n…`).
    last_rule_targets: RefCell<Option<Vec<String>>>,
    // Options
    pub jobs: usize,
    pub keep_going: bool,
    pub dry_run: bool,
    pub silent: bool,
    pub trace: bool,
    pub touch: bool,
    pub question: bool,
    pub always_make: Cell<bool>,
    /// `--shuffle` mode. None = no shuffle. Some(mode) where mode is:
    /// "reverse" | "none" | "identity" | random seed string.
    pub shuffle_mode: RefCell<Option<String>>,
    /// `-e`: env vars override makefile assignments
    pub env_overrides: bool,
    /// `-i`: ignore errors in recipes
    pub ignore_errors: bool,
    /// Explicit `-w` / `--print-directory` or `--no-print-directory`
    /// choice. `None` = pick default (print only if sub-make).
    /// Last-one-wins on the command line.
    pub print_directory_opt: Option<bool>,
    /// Set to true when any recipe is actually executed; used to emit the
    /// "… is up to date." diagnostic for goals that required no work.
    recipe_executed: RefCell<bool>,
    /// Set to true when the last build_target found a real recipe (even
    /// if it didn't need to run). Distinguishes "up to date" from
    /// "Nothing to be done".
    target_had_recipe: RefCell<bool>,
    /// Search path for `include` directives (populated from `-I`).
    pub include_dirs: RefCell<Vec<String>>,
    /// Targets declared silent via `.SILENT: targets`.
    silent_targets: RefCell<HashSet<String>>,
    /// Set by `.SILENT:` (no prereqs) — silence all recipes.
    silent_all: RefCell<bool>,
    /// Source location (file, line) of the current makefile directive being
    /// loaded. Used by `$(error)` / `$(warning)` for diagnostics.
    pub current_source: RefCell<Option<(String, usize)>>,
    /// Recipe/source info for the `.DEFAULT` special target — applied to
    /// any target that has no explicit or pattern rule.
    #[allow(clippy::type_complexity)]
    default_rule: RefCell<Option<(Vec<String>, Vec<usize>, String)>>,
    /// True once `.SUFFIXES:` (with no prerequisites) has been seen —
    /// built-in suffix/pattern rules are disabled in that case.
    suffixes_cleared: RefCell<bool>,
    /// Question mode (`-q`) — set to true when a target would need an
    /// update so the process exits with status 1.
    question_needs_update: RefCell<bool>,
    /// Target-specific variable assignments: target -> (var, op, value).
    /// Applied to each recipe's expansion scope when building that target.
    #[allow(clippy::type_complexity)]
    target_vars: RefCell<HashMap<String, Vec<(String, AssignOp, String, bool, bool)>>>,
    /// Per-target variable names that were declared with `export`
    /// (e.g. `two: export SHELL := …`). Added to `exports` only
    /// while the owning target's recipe runs.
    target_exports: RefCell<HashMap<String, Vec<String>>>,
    /// Stack of target-specific variable bindings active during the
    /// currently-recursing build. Prereq builds see the union of their
    /// ancestors' bindings so target-specific vars propagate downward.
    #[allow(clippy::type_complexity)]
    target_scope_stack: RefCell<Vec<(String, AssignOp, String, bool, bool)>>,
    /// When `.DELETE_ON_ERROR:` is set, make removes the target file if
    /// its recipe fails.
    delete_on_error: RefCell<bool>,
    /// Files marked "new" by `-W FILE` (assume-new / what-if). Targets
    /// depending on them are rebuilt as if the file was just modified.
    pub assume_new: RefCell<HashSet<String>>,
    /// Targets whose recipe was queued for rebuild this run (even in
    /// dry-run mode). Used so dependents of a rebuilt target also
    /// rebuild without relying on mtime updates that didn't happen.
    rebuilt_targets: RefCell<HashSet<String>>,
    /// Lock to prevent `process_rule` from setting the default goal.
    /// Used when auto-loading MAKEFILES env-var files so rules defined
    /// there don't steal the default goal from the primary makefile.
    pub suppress_default_goal: RefCell<bool>,
    /// Source location (file, line) where each variable was defined.
    /// Used for error reporting (e.g. unterminated variable reference).
    var_source_locs: RefCell<HashMap<String, (String, usize)>>,
    /// Source location tracking for the current recursive expansion chain.
    /// Updated in `lookup_var_with_auto` when expanding a recursive
    /// variable that has a known file definition location. Used by the
    /// expand module to report unterminated variable reference errors at
    /// the correct definition line. Separate from `current_source` which
    /// is used by `$(error)` / `$(warning)` for invocation-site reporting.
    pub expand_chain_source: RefCell<Option<(String, usize)>>,
    /// True while a recipe is being expanded. `$(eval)` inside a recipe
    /// must not define new prerequisites (GNU make Savannah bug #12124).
    pub in_recipe: RefCell<bool>,
    /// Targets that failed to build during -k (keep-going) mode.
    /// Prevents re-attempting targets that already failed when they
    /// appear as prerequisites of different dependents.
    failed_targets: RefCell<HashSet<String>>,
    /// When `--warn-undefined-variables` is set, emit a warning each
    /// time an undefined variable is expanded.
    pub warn_undefined_variables: Cell<bool>,
    /// When Some, keep-going error messages are collected here instead
    /// of being printed immediately. Used by finalize_includes to
    /// control output ordering (NSFD before build errors).
    pub buffered_kgo_errors: RefCell<Option<Vec<String>>>,
    /// True when `.POSIX` special target has been seen.
    posix_mode: RefCell<bool>,
    /// Set to true by finalize_includes when an included makefile was
    /// remade and the process should re-exec itself.
    pub needs_reexec: Cell<bool>,
    /// Recursion depth counter for build_target_for. Prevents stack
    /// overflow when implicit rule search creates an infinite chain
    /// (e.g. %: %.c matching hello -> hello.c -> hello.c.c -> ...).
    build_depth: Cell<usize>,
    pub printed_entering: Cell<bool>,
    /// Targets for which implicit rule search should be skipped.
    /// Used when building prereqs of terminal pattern rules.
    skip_implicit: RefCell<HashSet<String>>,
    /// Global variables declared with `private` — their values are
    /// temporarily removed from the variable map during recipe execution
    /// so they are not visible in recipe contexts.
    private_vars: RefCell<HashSet<String>>,
    /// Global exports that came from `private export` declarations —
    /// temporarily removed from the export set during recipe execution.
    private_exports: RefCell<HashSet<String>>,
    /// `.ONESHELL` special target: when set, all recipe lines are
    /// combined and passed to a single shell invocation.
    oneshell: Cell<bool>,
    /// `.NOTPARALLEL` disables `--shuffle` reordering.
    notparallel: Cell<bool>,
    /// Targets listed as prerequisites of `.SECONDARY`. These intermediate
    /// files are not automatically deleted after building.
    secondary_targets: RefCell<HashSet<String>>,
    /// Set by `.SECONDARY:` (no prereqs) — all targets are treated as secondary.
    secondary_all: Cell<bool>,
    /// Targets listed as prerequisites of `.INTERMEDIATE`. These files
    /// are treated as intermediate even if they are mentioned elsewhere.
    intermediate_targets: RefCell<HashSet<String>>,
    /// Explicit file names listed as prerequisites of `.NOTINTERMEDIATE`.
    notintermediate_files: RefCell<HashSet<String>>,
    /// `vpath PATTERN DIRS` directives.
    /// Each entry: (pattern with `%`, list of directories).
    vpath_patterns: RefCell<Vec<(String, Vec<String>)>>,
    /// Patterns (containing `%`) listed as prerequisites of `.NOTINTERMEDIATE`.
    notintermediate_patterns: RefCell<Vec<String>>,
    /// Set by `.NOTINTERMEDIATE:` (no prereqs) — all targets are treated
    /// as not-intermediate.
    notintermediate_all: Cell<bool>,
    /// Files to be deleted as intermediates after the top-level goal completes.
    /// (file, is_pattern_derived). Pattern-derived intermediates are
    /// deleted in reverse build order; explicit `.INTERMEDIATE`-listed
    /// files (or those reached as explicit prereqs) are deleted in build
    /// order. This matches GNU make's observable output.
    pending_intermediate_deletions: RefCell<Vec<(String, bool)>>,
    /// Pattern-specific variable assignments: (pattern, var_name, op, value, is_override, is_private).
    /// Applied to targets matching the pattern via `expand::pattern_stem`.
    #[allow(clippy::type_complexity)]
    pattern_vars: RefCell<Vec<(String, String, AssignOp, String, bool, bool)>>,
    /// Pattern-specific exports: (pattern, var_name).
    /// `~`-prefixed names indicate unexport entries.
    pattern_exports: RefCell<Vec<(String, String)>>,
    /// Files listed as prerequisites of `.PRECIOUS`. These files are not
    /// deleted on error even when `.DELETE_ON_ERROR` is active.
    precious_targets: RefCell<HashSet<String>>,
    /// Patterns listed as prerequisites of `.PRECIOUS` (contain `%`).
    precious_patterns: RefCell<Vec<String>>,
    /// Targets currently being built — used for circular dependency detection.
    building_chain: RefCell<HashSet<String>>,
    /// Files that existed (locally or via VPATH) before make tried to build them.
    /// Used to suppress intermediate deletion for pre-existing files.
    vpath_preexisting: RefCell<HashSet<String>>,
    /// Files whose recipe ran but didn't create the file locally.
    /// VPATH resolution is revoked for these files (GNU make "un-vpath").
    vpath_revoked: RefCell<HashSet<String>>,
    /// `.SECONDEXPANSION:` has been seen — affects rules defined after.
    pub second_expansion_enabled: Cell<bool>,
    /// Command-line-derived short flags for MAKEFLAGS (e.g. "erR").
    /// Makefile assignments cannot remove these flags.
    pub cmdline_mflags: RefCell<String>,
    /// Command-line-derived long flags for MAKEFLAGS (e.g. ["--trace"]).
    pub cmdline_mflags_long: RefCell<Vec<String>>,
}

/// GNU make's directory-transfer stem substitution for pattern rules.
/// When the stem contains a `/`, the directory part is extracted and
/// prepended to the result. E.g. stem="lib/bye", pattern="3%4" → "lib/3bye4".
fn pattern_subst_with_dir(pattern: &str, stem: &str) -> String {
    if !pattern.contains('%') {
        return pattern.to_string();
    }
    if let Some(slash_pos) = stem.rfind('/') {
        let dir = &stem[..=slash_pos]; // "lib/"
        let base = &stem[slash_pos + 1..]; // "bye"
        let substituted = pattern.replacen('%', base, 1);
        format!("{}{}", dir, substituted)
    } else {
        pattern.replacen('%', stem, 1)
    }
}

impl Engine {
    pub fn new() -> Self {
        let engine = Self {
            vars: RefCell::new(HashMap::new()),
            rules: RefCell::new(HashMap::new()),
            pattern_rules: RefCell::new(Vec::new()),
            default_goal: RefCell::new(None),
            phony_targets: RefCell::new(HashSet::new()),
            suffixes: RefCell::new(vec![
                ".o".into(),
                ".c".into(),
                ".cc".into(),
                ".cpp".into(),
                ".s".into(),
                ".S".into(),
                ".f".into(),
                ".F".into(),
            ]),
            exports: RefCell::new(HashSet::new()),
            unexports: RefCell::new(HashSet::new()),
            export_all: RefCell::new(false),
            built_targets: RefCell::new(HashSet::new()),
            group_built_targets: RefCell::new(HashSet::new()),
            group_recipe_done: RefCell::new(HashSet::new()),
            eval_queue: RefCell::new(Vec::new()),
            pending_includes: RefCell::new(Vec::new()),
            included_files: RefCell::new(Vec::new()),
            env_inherited: RefCell::new(HashSet::new()),
            immediate_recursive: RefCell::new(HashSet::new()),
            shell_depth: RefCell::new(0),
            last_rule_targets: RefCell::new(None),
            jobs: 1,
            keep_going: false,
            dry_run: false,
            silent: false,
            trace: false,
            touch: false,
            question: false,
            always_make: Cell::new(false),
            shuffle_mode: RefCell::new(None),
            env_overrides: false,
            ignore_errors: false,
            print_directory_opt: None,
            recipe_executed: RefCell::new(false),
            target_had_recipe: RefCell::new(false),
            include_dirs: RefCell::new(Vec::new()),
            silent_targets: RefCell::new(HashSet::new()),
            silent_all: RefCell::new(false),
            current_source: RefCell::new(None),
            default_rule: RefCell::new(None),
            suffixes_cleared: RefCell::new(false),
            question_needs_update: RefCell::new(false),
            target_vars: RefCell::new(HashMap::new()),
            target_exports: RefCell::new(HashMap::new()),
            target_scope_stack: RefCell::new(Vec::new()),

            delete_on_error: RefCell::new(false),
            assume_new: RefCell::new(HashSet::new()),
            rebuilt_targets: RefCell::new(HashSet::new()),
            suppress_default_goal: RefCell::new(false),
            var_source_locs: RefCell::new(HashMap::new()),
            expand_chain_source: RefCell::new(None),
            in_recipe: RefCell::new(false),
            failed_targets: RefCell::new(HashSet::new()),
            warn_undefined_variables: Cell::new(false),
            buffered_kgo_errors: RefCell::new(None),
            posix_mode: RefCell::new(false),
            needs_reexec: Cell::new(false),
            build_depth: Cell::new(0),
            printed_entering: Cell::new(false),
            skip_implicit: RefCell::new(HashSet::new()),
            private_vars: RefCell::new(HashSet::new()),
            private_exports: RefCell::new(HashSet::new()),
            oneshell: Cell::new(false),
            notparallel: Cell::new(false),
            secondary_targets: RefCell::new(HashSet::new()),
            secondary_all: Cell::new(false),
            intermediate_targets: RefCell::new(HashSet::new()),
            notintermediate_files: RefCell::new(HashSet::new()),
            vpath_patterns: RefCell::new(Vec::new()),
            notintermediate_patterns: RefCell::new(Vec::new()),
            notintermediate_all: Cell::new(false),
            pending_intermediate_deletions: RefCell::new(Vec::new()),
            vpath_preexisting: RefCell::new(HashSet::new()),
            vpath_revoked: RefCell::new(HashSet::new()),
            pattern_vars: RefCell::new(Vec::new()),
            pattern_exports: RefCell::new(Vec::new()),
            precious_targets: RefCell::new(HashSet::new()),
            precious_patterns: RefCell::new(Vec::new()),
            building_chain: RefCell::new(HashSet::new()),
            second_expansion_enabled: Cell::new(false),
            cmdline_mflags: RefCell::new(String::new()),
            cmdline_mflags_long: RefCell::new(Vec::new()),
        };

        // Set default variables
        engine.set_var_with_origin("MAKE", "make", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin("SHELL", "/bin/sh", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin("MAKEFLAGS", "", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin(".SHELLFLAGS", "-c", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin(
            ".LIBPATTERNS",
            "lib%.so lib%.a",
            VarFlavor::Simple,
            VarOrigin::Default,
        );
        // MAKELEVEL: inherit from env if present (sub-make), else 0.
        let makelevel = std::env::var("MAKELEVEL").unwrap_or_else(|_| "0".to_string());
        engine.set_var_with_origin(
            "MAKELEVEL",
            &makelevel,
            VarFlavor::Simple,
            VarOrigin::Default,
        );
        engine.set_var_with_origin(
            "CURDIR",
            &std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy(),
            VarFlavor::Simple,
            VarOrigin::Default,
        );
        engine.set_var_with_origin(
            "MAKE_VERSION",
            "0.1.0-rust",
            VarFlavor::Simple,
            VarOrigin::Default,
        );

        // Import environment variables
        for (key, value) in std::env::vars() {
            engine.env_inherited.borrow_mut().insert(key.clone());
            if !engine.vars.borrow().contains_key(&key) {
                engine.set_var_with_origin(&key, &value, VarFlavor::Simple, VarOrigin::Environment);
            }
        }

        // Set up default suffix rules
        engine.setup_default_rules();

        engine
    }

    /// Disable the built-in pattern rules (what `-r` does) — removes
    /// the rules added by `setup_default_rules`.
    pub fn disable_builtin_rules(&self) {
        self.pattern_rules
            .borrow_mut()
            .retain(|r| r.source_name != "<built-in>");
        *self.suffixes_cleared.borrow_mut() = true;
    }

    /// Drop the built-in variables (`CC`, `CXX`, ...). Used by `-R`,
    /// which disables both rules and variables.
    pub fn disable_builtin_vars(&self) {
        let names = [
            "CC", "CXX", "AS", "AR", "RM", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "LDLIBS",
            "ARFLAGS",
        ];
        let mut vars = self.vars.borrow_mut();
        for n in names {
            if let Some(var) = vars.get(n)
                && var.origin == VarOrigin::Default
            {
                vars.remove(n);
            }
        }
    }

    fn setup_default_rules(&self) {
        // C compilation. Recipes use the GNU-make canonical templates so
        // overrides like `CC="@echo cc"` or `OUTPUT_OPTION=` work as
        // documented.
        self.add_pattern_rule("%.o", &["%.c"], &["$(COMPILE.c) $(OUTPUT_OPTION) $<"]);
        self.add_pattern_rule("%.o", &["%.cpp"], &["$(COMPILE.cpp) $(OUTPUT_OPTION) $<"]);
        self.add_pattern_rule("%.o", &["%.cc"], &["$(COMPILE.cc) $(OUTPUT_OPTION) $<"]);
        self.add_pattern_rule("%.o", &["%.s"], &["$(COMPILE.s) -o $@ $<"]);
        // Fortran compilation
        self.add_pattern_rule("%.o", &["%.f"], &["$(COMPILE.f) $(OUTPUT_OPTION) $<"]);
        self.add_pattern_rule("%.o", &["%.F"], &["$(COMPILE.F) $(OUTPUT_OPTION) $<"]);
        // Linking
        self.add_pattern_rule(
            "%",
            &["%.o"],
            &["$(LINK.o) $^ $(LOADLIBES) $(LDLIBS) -o $@"],
        );
        self.add_pattern_rule(
            "%",
            &["%.f"],
            &["$(LINK.f) $^ $(LOADLIBES) $(LDLIBS) -o $@"],
        );
        self.add_pattern_rule(
            "%",
            &["%.F"],
            &["$(LINK.F) $^ $(LOADLIBES) $(LDLIBS) -o $@"],
        );

        self.set_var_default("CC", "cc");
        self.set_var_default("CXX", "g++");
        self.set_var_default("AS", "as");
        self.set_var_default("AR", "ar");
        self.set_var_default("RM", "rm -f");
        self.set_var_default("CFLAGS", "");
        self.set_var_default("CXXFLAGS", "");
        self.set_var_default("CPPFLAGS", "");
        self.set_var_default("LDFLAGS", "");
        self.set_var_default("LDLIBS", "");
        self.set_var_default("LOADLIBES", "");
        self.set_var_default("ASFLAGS", "");
        self.set_var_default("ARFLAGS", "rv");
        self.set_var_default("TARGET_ARCH", "");
        self.set_var_default("OUTPUT_OPTION", "-o $@");
        self.set_var_default("COMPILE.c", "$(CC) $(CFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c");
        self.set_var_default(
            "COMPILE.cpp",
            "$(CXX) $(CXXFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c",
        );
        self.set_var_default(
            "COMPILE.cc",
            "$(CXX) $(CXXFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c",
        );
        self.set_var_default("COMPILE.s", "$(AS) $(ASFLAGS)");
        self.set_var_default("LINK.o", "$(CC) $(LDFLAGS) $(TARGET_ARCH)");
        self.set_var_default("FC", "f77");
        self.set_var_default("FFLAGS", "");
        self.set_var_default("COMPILE.f", "$(FC) $(FFLAGS) $(TARGET_ARCH) -c");
        self.set_var_default("LINK.f", "$(FC) $(FFLAGS) $(LDFLAGS) $(TARGET_ARCH)");
        self.set_var_default("COMPILE.F", "$(FC) $(FFLAGS) $(CPPFLAGS) $(TARGET_ARCH) -c");
        self.set_var_default(
            "LINK.F",
            "$(FC) $(FFLAGS) $(CPPFLAGS) $(LDFLAGS) $(TARGET_ARCH)",
        );
    }

    /// Check if a target name is an old-style suffix rule (e.g., ".c.o").
    /// Returns (source_suffix, target_suffix) if it is.
    fn parse_suffix_rule(&self, target: &str) -> Option<(String, String)> {
        let suffixes = self.suffixes.borrow();
        // Try all possible splits: .src.dst
        for dst in suffixes.iter() {
            if target.ends_with(dst.as_str()) && target.len() > dst.len() {
                let src = &target[..target.len() - dst.len()];
                if suffixes.iter().any(|s| s == src) {
                    return Some((src.to_string(), dst.to_string()));
                }
            }
        }
        None
    }

    fn set_var_default(&self, name: &str, value: &str) {
        let vars = self.vars.borrow();
        if !vars.contains_key(name) {
            drop(vars);
            self.set_var_with_origin(name, value, VarFlavor::Recursive, VarOrigin::Default);
        }
    }

    fn add_pattern_rule(&self, target: &str, prereqs: &[&str], recipe: &[&str]) {
        self.pattern_rules.borrow_mut().push(PatternRuleEntry {
            target_pattern: target.to_string(),
            prereq_patterns: prereqs.iter().map(|s| s.to_string()).collect(),
            order_only_patterns: Vec::new(),
            recipe: recipe.iter().map(|s| s.to_string()).collect(),
            recipe_lines: vec![0; recipe.len()],
            source_name: "<built-in>".to_string(),
            is_terminal: false,
            sibling_patterns: Vec::new(),
            second_expand: false,
            raw_prereq_text: None,
            raw_order_only_text: None,
            is_grouped: false,
        });
    }

    #[allow(dead_code)]
    pub fn set_var(&self, name: &str, value: &str, flavor: VarFlavor) {
        self.set_var_with_origin(name, value, flavor, VarOrigin::File);
    }

    pub fn set_var_with_origin(
        &self,
        name: &str,
        value: &str,
        flavor: VarFlavor,
        origin: VarOrigin,
    ) {
        // Keep the internal default_goal in sync with user assignments to
        // the special `.DEFAULT_GOAL` variable. Empty / whitespace resets.
        if name == ".DEFAULT_GOAL" {
            let trimmed = value.trim();
            *self.default_goal.borrow_mut() = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        // MAKEFLAGS merge: when a makefile assigns MAKEFLAGS, command-line
        // flags can never be removed. Merge the makefile-provided short
        // flags with the saved command-line flags and re-add long flags.
        if name == "MAKEFLAGS" && origin == VarOrigin::File && !self.env_overrides {
            let cmdline_short = self.cmdline_mflags.borrow();
            let cmdline_long = self.cmdline_mflags_long.borrow();

            // Split out the MAKEOVERRIDES suffix. Two formats:
            //   "$(if $(MAKEOVERRIDES), -- $(MAKEOVERRIDES))" (no initial overrides)
            //   " -- $(MAKEOVERRIDES)" (when overrides existed at startup)
            // Flags may appear both before AND after this suffix
            // (e.g. after `+=` the raw value is
            //   "<suffix> -r").
            let (before_suffix, after_suffix) =
                if let Some(pos) = value.find("$(if $(MAKEOVERRIDES)") {
                    // Find the matching closing paren for the $(if ...) call.
                    let mut depth = 0;
                    let mut end = pos;
                    for (i, ch) in value[pos..].char_indices() {
                        match ch {
                            '(' if i > 0 && value.as_bytes().get(pos + i - 1) == Some(&b'$') => {
                                depth += 1;
                            }
                            '(' => {
                                depth += 1;
                            }
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = pos + i + 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    (value[..pos].trim_end(), value[end..].trim_start())
                } else if let Some(pos) = value.find(" -- $(MAKEOVERRIDES)") {
                    let end = pos + " -- $(MAKEOVERRIDES)".len();
                    (value[..pos].trim_end(), value[end..].trim_start())
                } else {
                    (value, "")
                };
            // Combine both parts for flag parsing.
            let flags_part = if after_suffix.is_empty() {
                before_suffix.to_string()
            } else if before_suffix.is_empty() {
                after_suffix.to_string()
            } else {
                format!("{before_suffix} {after_suffix}")
            };

            // Known single-char flags that GNU make supports.
            let known_short: &str = "BdeiknqrRsStw";

            // Parse all tokens from the file-assigned value. Extract:
            // - bare cluster (no '-' prefix) → short flags
            // - `-X` where X is a single known short flag → short flag
            // - `-XY...` cluster of short flags → each char is a short flag
            // - `--long-option` → file-added long flag
            let mut file_short = String::new();
            let mut file_long: Vec<String> = Vec::new();
            let mut past_separator = false;
            for tok in flags_part.as_str().split_whitespace() {
                if tok == "--" {
                    past_separator = true;
                    continue;
                }
                // Skip variable assignments (after -- or containing =)
                if past_separator || tok.contains('=') {
                    continue;
                }
                if tok.starts_with("--") {
                    // Long flag from file (e.g. --trace, --warn-undefined-variables)
                    file_long.push(tok.to_string());
                } else if tok.starts_with("-I") || tok.starts_with("-l") || tok.starts_with("-O") {
                    // -Idir, -l2.5, -Otarget — flags with attached values, preserve as long flags
                    file_long.push(tok.to_string());
                } else if let Some(stripped) = tok.strip_prefix('-') {
                    // Short flag cluster with dash: -r, -rR, -Idir, etc.
                    // Extract known short-flag chars; stop at first
                    // flag that takes an argument (I, j, l, o, W, f, C).
                    let chars: Vec<char> = stripped.chars().collect();
                    for &ch in &chars {
                        if known_short.contains(ch) {
                            file_short.push(ch);
                        } else {
                            // Unknown or arg-taking flag — stop parsing cluster
                            break;
                        }
                    }
                } else {
                    // Bare cluster (no dash) — each char is a short flag
                    for ch in tok.chars() {
                        if known_short.contains(ch) {
                            file_short.push(ch);
                        }
                    }
                }
            }

            // Merge short flags: union of cmdline and file-assigned flags.
            // Suppress mutually-exclusive pairs: cmdline "off" flag wins over file "on" flag.
            let mut merged_chars: Vec<char> = cmdline_short.chars().collect();
            for ch in file_short.chars() {
                if merged_chars.contains(&ch) {
                    continue;
                }
                // Cmdline --no-print-directory suppresses file w
                if ch == 'w' && cmdline_long.contains(&"--no-print-directory".to_string()) {
                    continue;
                }
                // Cmdline --no-silent suppresses file s
                if ch == 's' && cmdline_long.contains(&"--no-silent".to_string()) {
                    continue;
                }
                // Cmdline S suppresses file k; cmdline k suppresses file S
                if ch == 'k' && cmdline_short.contains('S') {
                    continue;
                }
                if ch == 'S' && cmdline_short.contains('k') {
                    continue;
                }
                merged_chars.push(ch);
            }
            merged_chars.sort_by(|a, b| {
                let la = a.to_ascii_lowercase();
                let lb = b.to_ascii_lowercase();
                la.cmp(&lb).then(b.cmp(a))
            });

            let mut merged = String::from_iter(&merged_chars);

            // Suppress cmdline long flags that conflict with merged short flags.
            // If file added 'w' and cmdline had --no-print-directory, 'w' was already
            // suppressed above. But if file added 'w' and cmdline did NOT have
            // --no-print-directory, suppress --no-print-directory that might be in cmdline.
            // Conversely, if merged has 's', remove --no-silent from long flags.
            // Actually: cmdline long flags always win, so we only filter file-added longs.

            // Collect all long flags: cmdline first (always preserved), then file-added.
            // Deduplicate by value (e.g. -Ilocaltmp appearing twice).
            let mut all_long: Vec<String> = Vec::new();
            for fl in cmdline_long.iter().chain(file_long.iter()) {
                if all_long.contains(fl) {
                    continue;
                }
                // File --no-print-directory suppressed if cmdline has w
                if fl == "--no-print-directory" && cmdline_short.contains('w') {
                    continue;
                }
                // File --no-silent suppressed if cmdline has s
                if fl == "--no-silent" && cmdline_short.contains('s') {
                    continue;
                }
                all_long.push(fl.clone());
            }
            // Sort long flags in GNU make's canonical order, preserving
            // insertion order within each category (stable sort).
            let long_category = |f: &str| -> u8 {
                if f.starts_with("-I") {
                    0
                } else if f.starts_with("-l") {
                    1
                } else if f.starts_with("-O") {
                    2
                } else if f.starts_with("--debug") {
                    3
                } else if f == "--trace" {
                    4
                } else if f == "--no-print-directory" {
                    5
                } else if f == "--warn-undefined-variables" {
                    6
                } else if f == "--no-silent" {
                    7
                } else if f.starts_with("--eval") {
                    8
                } else {
                    9
                }
            };
            all_long.sort_by_key(|f| long_category(f));

            // Append long flags.
            if merged.is_empty() && !all_long.is_empty() {
                merged.push(' ');
            }
            for (idx, long) in all_long.iter().enumerate() {
                if idx > 0 || !merged_chars.is_empty() {
                    merged.push(' ');
                }
                merged.push_str(long);
            }

            // Re-append the dynamic MAKEOVERRIDES suffix.
            // Preserve the original format: if the value had " -- $(MAKEOVERRIDES)"
            // (unconditional separator), keep that; otherwise use $(if ...).
            if value.contains(" -- $(MAKEOVERRIDES)") && !value.contains("$(if $(MAKEOVERRIDES)") {
                merged.push_str(" -- $(MAKEOVERRIDES)");
            } else {
                merged.push_str("$(if $(MAKEOVERRIDES), -- $(MAKEOVERRIDES))");
            }

            // Apply side effects for Cell-based fields (safe through &self).
            for ch in &merged_chars {
                if ch == &'B' {
                    self.always_make.set(true);
                }
            }
            // When -w is set via MAKEFLAGS in a makefile, print
            // "Entering directory" immediately if not yet printed.
            if merged_chars.contains(&'w') && !self.printed_entering.get() {
                let makelevel: i32 = self.lookup_var("MAKELEVEL").parse().unwrap_or(0);
                let make_tag = if makelevel > 0 {
                    format!("make[{makelevel}]")
                } else {
                    "make".to_string()
                };
                if let Ok(cwd) = std::env::current_dir() {
                    println!("{make_tag}: Entering directory '{}'", cwd.display());
                }
                self.printed_entering.set(true);
            }
            for fl in &all_long {
                if fl == "--warn-undefined-variables" {
                    self.warn_undefined_variables.set(true);
                }
            }

            // Update .INCLUDE_DIRS when -I flags change via MAKEFLAGS.
            for fl in &all_long {
                if let Some(dir) = fl.strip_prefix("-I") {
                    if dir == "-" {
                        self.include_dirs.borrow_mut().clear();
                    } else if !self.include_dirs.borrow().contains(&dir.to_string()) {
                        self.include_dirs.borrow_mut().push(dir.to_string());
                    }
                }
            }

            // Bypass origin check — always allow this merged write.
            // Preserve the existing origin (e.g. CommandLine) so that
            // subsequent file-level `+=` is blocked when `-e` is active.
            let existing_origin = self
                .vars
                .borrow()
                .get("MAKEFLAGS")
                .map(|v| v.origin)
                .unwrap_or(VarOrigin::Default);
            self.vars.borrow_mut().insert(
                "MAKEFLAGS".to_string(),
                Variable {
                    value: merged,
                    flavor: VarFlavor::Recursive,
                    origin: existing_origin,
                },
            );
            return;
        }

        // Origin precedence: Override > CommandLine > Environment (with -e) > File >
        // Environment (default) > Default. Skip the assignment when an existing
        // variable has higher precedence.
        if let Some(existing) = self.vars.borrow().get(name)
            && !Self::can_override(origin, existing.origin, self.env_overrides)
        {
            return;
        }
        self.vars.borrow_mut().insert(
            name.to_string(),
            Variable {
                value: value.to_string(),
                flavor,
                origin,
            },
        );
    }

    fn origin_rank(origin: VarOrigin, env_overrides: bool) -> u8 {
        match origin {
            VarOrigin::Override => 5,
            VarOrigin::CommandLine => 4,
            VarOrigin::Environment if env_overrides => 3,
            VarOrigin::File => 2,
            VarOrigin::Environment => 1,
            VarOrigin::Default => 0,
            VarOrigin::Undefined => 0,
            VarOrigin::Automatic => 4, // automatic = command-line priority
        }
    }

    fn can_override(new: VarOrigin, existing: VarOrigin, env_overrides: bool) -> bool {
        Self::origin_rank(new, env_overrides) >= Self::origin_rank(existing, env_overrides)
    }

    /// Lookup a variable, expanding if recursive.
    pub fn lookup_var(&self, name: &str) -> String {
        self.lookup_var_with_auto(name, &HashMap::new())
    }

    pub fn lookup_var_with_auto(&self, name: &str, auto_vars: &HashMap<&str, String>) -> String {
        // Dynamic built-ins — reflect current engine state rather than a
        // stored copy.
        if name == ".DEFAULT_GOAL" {
            return self.default_goal.borrow().clone().unwrap_or_default();
        }
        if name == ".VARIABLES" {
            let vars = self.vars.borrow();
            let mut names: Vec<&str> = vars.keys().map(|s| s.as_str()).collect();
            names.sort();
            return names.join(" ");
        }
        if name == ".INCLUDE_DIRS" {
            return self.include_dirs.borrow().join(" ");
        }
        let vars = self.vars.borrow();
        if let Some(var) = vars.get(name) {
            let value = var.value.clone();
            let flavor = var.flavor;
            drop(vars);
            match flavor {
                VarFlavor::Recursive => {
                    // Save/restore expand_chain_source so that inner
                    // recursive lookups don't clobber the outer
                    // definition location used for error reporting.
                    let old_chain = self.expand_chain_source.borrow().clone();
                    if let Some(loc) = self.var_source_locs.borrow().get(name).cloned() {
                        *self.expand_chain_source.borrow_mut() = Some(loc);
                    }
                    let result = expand::expand_with_auto(&value, self, auto_vars);
                    *self.expand_chain_source.borrow_mut() = old_chain;
                    result
                }
                VarFlavor::Simple => value,
                VarFlavor::Undefined => String::new(),
            }
        } else {
            self.warn_undefined(name);
            String::new()
        }
    }

    pub fn lookup_var_or(&self, name: &str, default: &str) -> String {
        let val = self.lookup_var(name);
        if val.is_empty() {
            default.to_string()
        } else {
            val
        }
    }

    pub fn lookup_var_raw(&self, name: &str) -> String {
        self.vars
            .borrow()
            .get(name)
            .map(|v| v.value.clone())
            .unwrap_or_default()
    }

    /// Emit a warning for an undefined variable reference when
    /// `--warn-undefined-variables` is active. Skips automatic variables
    /// and GNU make's built-in "no-warn" names.
    fn warn_undefined(&self, name: &str) {
        if !self.warn_undefined_variables.get() {
            return;
        }
        // Automatic variables -- never warn.
        if matches!(
            name,
            "@" | "<"
                | "^"
                | "*"
                | "?"
                | "+"
                | "|"
                | "%"
                | "@D"
                | "@F"
                | "<D"
                | "<F"
                | "^D"
                | "^F"
                | "*D"
                | "*F"
                | "?D"
                | "?F"
                | "+D"
                | "+F"
                | "|D"
                | "|F"
                | "%D"
                | "%F"
        ) {
            return;
        }
        // GNU make built-in "no-warn" list.
        const NO_WARN: &[&str] = &[
            "GNUMAKEFLAGS",
            "MAKEFLAGS",
            "MAKE_COMMAND",
            "MAKECMDGOALS",
            "MAKE_RESTARTS",
            "MAKE_TERMERR",
            "MAKE_TERMOUT",
            "MAKELEVEL",
            "MFLAGS",
            "SUFFIXES",
            ".DEFAULT",
            ".DEFAULT_GOAL",
            ".EXTRA_PREREQS",
            ".FEATURES",
            ".INCLUDE_DIRS",
            ".LOADED",
            ".RECIPEPREFIX",
            ".SHELLFLAGS",
            ".VARIABLES",
            "-*-command-variables-*-",
            "-*-eval-flags-*-",
            "VPATH",
            "GPATH",
        ];
        if NO_WARN.contains(&name) {
            return;
        }
        if let Some((file, line)) = self.current_source.borrow().clone() {
            eprintln!("{file}:{line}: warning: undefined variable '{name}'");
        }
    }

    /// Resolve a filename through VPATH. Returns the VPATH-resolved path
    /// if the file is found in a VPATH directory, or None if not found.
    /// If the file already exists in the current directory, returns None
    /// (the caller should check the current directory first).
    /// Files in `vpath_revoked` are skipped (GNU make "un-vpath" behavior).
    fn resolve_vpath(&self, name: &str) -> Option<String> {
        // GNU make "un-vpath": if a recipe ran for this file but didn't
        // create it locally, don't resolve via VPATH anymore.
        if self.vpath_revoked.borrow().contains(name) {
            return None;
        }
        // GNU make's vpath/VPATH search only fires when the named file
        // is NOT in the current directory. If `name` exists locally,
        // do not redirect to a vpath copy.
        if Path::new(name).exists() {
            return None;
        }
        // Try `vpath PATTERN DIRS` directives first — patterns whose
        // `%` expansion matches `name` consult their attached dirs.
        // ALL matching patterns are tried (in declaration order),
        // not just the first match (GNU vpath search semantics).
        let vp = self.vpath_patterns.borrow();
        for (pat, dirs) in vp.iter() {
            if self.vpath_pattern_matches(name, pat) {
                for dir in dirs {
                    let candidate = Path::new(dir).join(name);
                    if candidate.exists() {
                        return Some(candidate.to_string_lossy().to_string());
                    }
                }
            }
        }
        drop(vp);
        let vpath = self.lookup_var("VPATH");
        if vpath.is_empty() {
            return None;
        }
        for dir in vpath.split(&[':', ' '][..]) {
            let dir = dir.trim();
            if dir.is_empty() {
                continue;
            }
            let candidate = Path::new(dir).join(name);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    }

    /// Vpath search that returns the resolved path along with the
    /// vpath-pattern index and directory index within that pattern.
    /// This is used by `resolve_library_prereq` to pick the
    /// "earliest" match across all `.LIBPATTERNS` candidates,
    /// matching GNU make's linker-compatible library search order.
    /// Also searches the general `VPATH` variable (those entries
    /// come after all `vpath` directive entries).
    fn resolve_vpath_with_index(&self, name: &str) -> Option<(String, usize, usize)> {
        if Path::new(name).exists() {
            return None;
        }
        let vp = self.vpath_patterns.borrow();
        for (vi, (pat, dirs)) in vp.iter().enumerate() {
            if self.vpath_pattern_matches(name, pat) {
                for (pi, dir) in dirs.iter().enumerate() {
                    let candidate = Path::new(dir).join(name);
                    if candidate.exists() {
                        return Some((candidate.to_string_lossy().to_string(), vi, pi));
                    }
                }
            }
        }
        let vpath_base = vp.len();
        drop(vp);
        let vpath = self.lookup_var("VPATH");
        if !vpath.is_empty() {
            for (pi, dir) in vpath.split(&[':', ' '][..]).enumerate() {
                let dir = dir.trim();
                if dir.is_empty() {
                    continue;
                }
                let candidate = Path::new(dir).join(name);
                if candidate.exists() {
                    return Some((candidate.to_string_lossy().to_string(), vpath_base, pi));
                }
            }
        }
        None
    }

    /// Check if a vpath pattern matches a name. Handles both
    /// `%`-patterns (via `pattern_stem`) and literal patterns
    /// (exact string comparison, for `vpath hello.c src`).
    fn vpath_pattern_matches(&self, name: &str, pat: &str) -> bool {
        if expand::find_unescaped_percent(pat).is_some() {
            expand::pattern_stem(name, pat).is_some()
        } else {
            // Literal pattern: match exactly.
            pat == name
        }
    }

    /// Resolve a target name through vpath directives, looking for
    /// another target that has registered rules (not file existence).
    /// Used for "same file" merging (sv 62650) where `vpath hello.c
    /// src` plus rules for both `hello.c` and `src/hello.c` should
    /// redirect to `src/hello.c`.
    fn resolve_vpath_to_rule_target(&self, name: &str) -> Option<String> {
        // Don't redirect if the file exists locally (GNU behavior).
        if Path::new(name).exists() {
            return None;
        }
        let rules = self.rules.borrow();
        let vp = self.vpath_patterns.borrow();
        for (pat, dirs) in vp.iter() {
            if self.vpath_pattern_matches(name, pat) {
                for dir in dirs {
                    let candidate = Path::new(dir).join(name).to_string_lossy().to_string();
                    if rules.contains_key(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
        drop(vp);
        drop(rules);
        let vpath = self.lookup_var("VPATH");
        if !vpath.is_empty() {
            let rules = self.rules.borrow();
            for dir in vpath.split(&[':', ' '][..]) {
                let dir = dir.trim();
                if dir.is_empty() {
                    continue;
                }
                let candidate = Path::new(dir).join(name).to_string_lossy().to_string();
                if rules.contains_key(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Look up a target's vpath-resolved name when there is no
    /// explicit rule for the original name. Searches both
    /// `vpath PATTERN DIRS` directives (matching `name` against
    /// the pattern) and the general `VPATH` variable, returning
    /// the first dir+name combination that has a registered rule.
    /// Used so `fail.te` (with `vpath %.te vpath-d/`) finds the
    /// rule defined as `vpath-d/fail.te`, matching GNU make.
    fn resolve_vpath_rule(&self, name: &str) -> Option<String> {
        let rules = self.rules.borrow();
        let vp = self.vpath_patterns.borrow();
        // Try ALL matching patterns in declaration order, not just
        // the first matching pattern.
        for (pat, dirs) in vp.iter() {
            if self.vpath_pattern_matches(name, pat) {
                for dir in dirs {
                    let candidate = Path::new(dir).join(name).to_string_lossy().to_string();
                    if rules.contains_key(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
        drop(vp);
        let vpath = self.lookup_var("VPATH");
        if !vpath.is_empty() {
            for dir in vpath.split(&[':', ' '][..]) {
                let dir = dir.trim();
                if dir.is_empty() {
                    continue;
                }
                let candidate = Path::new(dir).join(name).to_string_lossy().to_string();
                if rules.contains_key(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Check if a file exists, either in the current directory or via VPATH.
    fn file_exists_or_vpath(&self, name: &str) -> bool {
        Path::new(name).exists() || self.resolve_vpath(name).is_some()
    }

    /// Get file metadata (for mtime checks), resolving through VPATH if needed.
    fn metadata_or_vpath(&self, name: &str) -> Option<std::fs::Metadata> {
        if let Ok(meta) = std::fs::metadata(name) {
            return Some(meta);
        }
        if let Some(resolved) = self.resolve_vpath(name) {
            return std::fs::metadata(&resolved).ok();
        }
        None
    }
    pub fn var_origin(&self, name: &str) -> VarOrigin {
        // Automatic variables (`$@`, `$<`, `$^`, `$*`, `$?`, `$+`, `$|`, `$%`,
        // and the `D`/`F` variants) always report "automatic".
        if matches!(name, "@" | "<" | "^" | "*" | "?" | "+" | "|" | "%")
            || matches!(
                name,
                "@D" | "@F"
                    | "<D"
                    | "<F"
                    | "^D"
                    | "^F"
                    | "*D"
                    | "*F"
                    | "?D"
                    | "?F"
                    | "+D"
                    | "+F"
                    | "|D"
                    | "|F"
                    | "%D"
                    | "%F"
            )
        {
            return VarOrigin::Automatic;
        }
        self.vars
            .borrow()
            .get(name)
            .map(|v| v.origin)
            .unwrap_or(VarOrigin::Undefined)
    }

    pub fn var_flavor(&self, name: &str) -> VarFlavor {
        self.vars
            .borrow()
            .get(name)
            .map(|v| v.flavor)
            .unwrap_or(VarFlavor::Undefined)
    }

    pub fn is_var_defined(&self, name: &str) -> bool {
        self.vars.borrow().contains_key(name)
    }

    /// Load and process a Makefile.
    pub fn load_makefile(&self, directives: &[Directive]) {
        for directive in directives {
            self.process_directive(directive);
        }
        // Process any eval'd content
        self.process_eval_queue();
    }

    /// After the top-level parse, try to build any deferred include files.
    /// Returns 0 on success, 2 if mandatory includes failed.
    pub fn finalize_includes(&self) -> i32 {
        let pending: Vec<(String, bool, String, usize)> =
            self.pending_includes.borrow_mut().drain(..).collect();

        // Track whether any included makefile was remade.  If so, the
        // process must re-exec so the rebuilt makefile is re-read from
        // scratch (GNU make "restart" semantics).
        let mut any_include_remade = false;

        // Temporarily disable -B (always-make) during include remaking
        // so we only rebuild includes that are genuinely out of date.
        // Without this, -B would cause an infinite re-exec loop.
        let saved_always_make = self.always_make.get();
        self.always_make.set(false);

        // On restart (MAKE_RESTARTS > 0), temporarily clear assume_new
        // (-W files) during include remaking. GNU make only applies -W
        // for include remaking on the first run; on restart, -W should
        // only affect target building, not trigger another restart.
        let saved_assume_new = {
            let restarts: u32 = std::env::var("MAKE_RESTARTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if restarts > 0 {
                let saved = self.assume_new.borrow().clone();
                self.assume_new.borrow_mut().clear();
                Some(saved)
            } else {
                None
            }
        };

        // Phase 0: Try to remake include files that were successfully loaded.
        // GNU make always tries to rebuild included files; if the recipe
        // runs but doesn't change the file, the build proceeds normally.
        let loaded_includes: Vec<String> = self.included_files.borrow_mut().drain(..).collect();
        for file in &loaded_includes {
            let has_explicit = self.rules.borrow().contains_key(file.as_str());
            let has_pattern = self.find_pattern_rule(file).is_some();
            if !has_explicit && !has_pattern {
                continue;
            }
            let is_phony = self.phony_targets.borrow().contains(file.as_str());
            let is_double_colon_no_prereqs = {
                let rules = self.rules.borrow();
                if let Some(entries) = rules.get(file.as_str()) {
                    entries.iter().any(|e| e.is_double_colon)
                        && entries.iter().all(|e| e.prerequisites.is_empty())
                } else {
                    false
                }
            };
            if is_phony || is_double_colon_no_prereqs {
                continue;
            }
            // Save mtime before attempting rebuild
            let mtime_before = std::fs::metadata(file).ok().and_then(|m| m.modified().ok());
            let _ = self.build_target(file);
            // If the file changed, re-read it
            let mtime_after = std::fs::metadata(file).ok().and_then(|m| m.modified().ok());
            if mtime_before != mtime_after {
                // File was modified — do NOT re-read it here.
                // The re-exec'd process will re-read from scratch.
                // Re-reading now would prematurely evaluate $(info ...)
                // and similar before the restart.
                any_include_remade = true;
            }
            // Mark as built so main build doesn't re-evaluate
            self.built_targets.borrow_mut().insert(file.clone());
        }
        // Clear rebuilt_targets from the remaking phase so the main build
        // doesn't cascade unnecessary rebuilds.
        self.rebuilt_targets.borrow_mut().clear();

        // Phase 1: Try to load files that might now exist (e.g. created by
        // $(shell) during parsing). Collect those that still need building.
        struct PendingEntry {
            file: String,
            silent: bool,
            source: String,
            line_no: usize,
            file_exists_unreadable: bool,
        }
        let mut to_build: Vec<PendingEntry> = Vec::new();
        for (file, silent, source, line_no) in &pending {
            if self.load_file_with_loc(file, true, None) {
                continue;
            }
            // Check if file exists but is unreadable
            let file_exists_unreadable =
                std::path::Path::new(file).exists() && std::fs::read_to_string(file).is_err();
            to_build.push(PendingEntry {
                file: file.clone(),
                silent: *silent,
                source: source.clone(),
                line_no: *line_no,
                file_exists_unreadable,
            });
        }

        if to_build.is_empty() {
            // Restore -B before returning.
            self.always_make.set(saved_always_make);
            if let Some(ref saved) = saved_assume_new {
                *self.assume_new.borrow_mut() = saved.clone();
            }
            if any_include_remade {
                self.needs_reexec.set(true);
            }
            return 0;
        }

        // Phase 2: Attempt to build each pending include file.
        struct BuildResult {
            file: String,
            silent: bool,
            source: String,
            line_no: usize,
            build_err: Option<String>,
            had_rule: bool,
            skip_reason: Option<&'static str>, // "phony" or "double_colon"
            #[allow(dead_code)]
            printed_nsfd: bool,
            kgo_errors: Vec<String>,
        }
        let mut results: Vec<BuildResult> = Vec::new();
        for entry in &to_build {
            let has_explicit = self.rules.borrow().contains_key(entry.file.as_str());
            let has_pattern = self.find_pattern_rule(&entry.file).is_some();
            let has_rule = has_explicit || has_pattern;

            let is_phony = self.phony_targets.borrow().contains(entry.file.as_str());
            let is_double_colon_no_prereqs = {
                let rules = self.rules.borrow();
                if let Some(entries) = rules.get(entry.file.as_str()) {
                    entries.iter().any(|e| e.is_double_colon)
                        && entries.iter().all(|e| e.prerequisites.is_empty())
                } else {
                    false
                }
            };

            let skip_reason = if is_phony {
                Some("phony")
            } else if is_double_colon_no_prereqs {
                Some("double_colon")
            } else {
                None
            };

            let should_build = has_rule && skip_reason.is_none();

            let printed_nsfd = false;

            // Activate keep-going error buffering so we can print NSFD
            // before any build errors in Phase 3.
            *self.buffered_kgo_errors.borrow_mut() = Some(Vec::new());

            let build_err = if should_build {
                // For unreadable files, force rebuild by temporarily setting
                // always_make. This ensures the recipe runs even though the
                // file "exists" (but can't be read).
                let saved_always = self.always_make.get();
                if entry.file_exists_unreadable {
                    self.always_make.set(true);
                }
                let result = match self.build_target(&entry.file) {
                    Ok(()) => None,
                    Err(e) => {
                        // Mark as built to prevent pattern rule siblings
                        // from re-running the same recipe.
                        self.built_targets.borrow_mut().insert(entry.file.clone());
                        Some(e)
                    }
                };
                if entry.file_exists_unreadable {
                    self.always_make.set(saved_always);
                }
                result
            } else {
                None
            };

            let kgo_errors = self
                .buffered_kgo_errors
                .borrow_mut()
                .take()
                .unwrap_or_default();

            results.push(BuildResult {
                file: entry.file.clone(),
                silent: entry.silent,
                source: entry.source.clone(),
                line_no: entry.line_no,
                build_err,
                had_rule: should_build,
                skip_reason,
                printed_nsfd,
                kgo_errors,
            });
        }

        // Restore -B before Phase 3 error reporting (but after all
        // include remaking builds are done).
        self.always_make.set(saved_always_make);

        // Phase 3: Try to load rebuilt files and report errors.
        let mut has_fatal = false;
        for res in &results {
            // If the file was rebuilt (had a rule, build succeeded) and now
            // exists, mark for restart without loading. The re-exec'd
            // process will load it from scratch. Loading now would
            // prematurely evaluate $(info ...) etc.
            if res.had_rule && res.build_err.is_none() && std::path::Path::new(&res.file).exists() {
                any_include_remade = true;
                continue;
            }
            if self.load_file_with_loc(&res.file, true, None) {
                continue;
            }

            if res.silent {
                // -include: silently skip all failures, even build errors.
                // The error will surface if/when the target is needed
                // during the main build phase.
                continue;
            }

            has_fatal = true;

            let file_exists = std::path::Path::new(&res.file).exists();
            let err_msg = if file_exists {
                match std::fs::read_to_string(&res.file) {
                    Err(e) => clean_io_error(&format!("{}", e)),
                    Ok(_) => "No such file or directory".to_string(),
                }
            } else {
                "No such file or directory".to_string()
            };

            if let Some(ref build_err) = res.build_err {
                // Print NSFD first, then any buffered keep-going errors.
                if !res.printed_nsfd {
                    eprintln!("{}:{}: {}: {}", res.source, res.line_no, res.file, err_msg);
                }
                for kgo_err in &res.kgo_errors {
                    eprintln!("{kgo_err}");
                }
                if !build_err.is_empty() {
                    if build_err.starts_with('[') {
                        eprintln!("make: *** {build_err}");
                    } else {
                        eprintln!("make: *** {build_err}.  Stop.");
                    }
                }
                if self.keep_going {
                    eprintln!(
                        "{}:{}: Failed to remake makefile '{}'.",
                        res.source, res.line_no, res.file
                    );
                }
            } else if res.had_rule {
                // Rule existed, build OK, but file still missing/unreadable
                eprintln!(
                    "{}:{}: Failed to remake makefile '{}'.",
                    res.source, res.line_no, res.file
                );
            } else if res.skip_reason.is_some() {
                // Phony or double-colon-no-prereqs: just print NSFD
                eprintln!("{}:{}: {}: {}", res.source, res.line_no, res.file, err_msg);
            } else {
                // No rule at all
                eprintln!("{}:{}: {}: {}", res.source, res.line_no, res.file, err_msg);
                eprintln!("make: *** No rule to make target '{}'.  Stop.", res.file);
            }
        }

        // Clear build state from include remaking so the main build
        // starts fresh. Without this, targets rebuilt during include
        // remaking would cascade unnecessary rebuilds in the main build.
        self.built_targets.borrow_mut().clear();
        self.rebuilt_targets.borrow_mut().clear();

        // Mark successfully remade include files as "built" so the main
        // build doesn't re-evaluate them (matching GNU make's restart
        // behavior where include files are up-to-date in the second pass).
        for res in &results {
            if res.build_err.is_none() && res.had_rule {
                self.built_targets.borrow_mut().insert(res.file.clone());
            }
        }

        // Restore assume_new (-W files) for the main build phase.
        if let Some(saved) = saved_assume_new {
            *self.assume_new.borrow_mut() = saved;
        }

        if has_fatal {
            2
        } else {
            if any_include_remade {
                self.needs_reexec.set(true);
            }
            0
        }
    }

    #[allow(dead_code)]
    fn resolve_include_path(&self, path: &str) -> Option<String> {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
        for dir in self.include_dirs.borrow().iter() {
            let candidate = std::path::Path::new(dir).join(path);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    }

    fn apply_define(&self, name: &str, op: AssignOp, body: &str, origin: VarOrigin) {
        let flavor = match op {
            AssignOp::Simple => VarFlavor::Simple,
            _ => VarFlavor::Recursive,
        };
        match op {
            AssignOp::Conditional => {
                if matches!(self.var_origin(name), VarOrigin::Undefined) {
                    self.set_var_with_origin(name, body, flavor, origin);
                }
            }
            AssignOp::Append => {
                let mut vars = self.vars.borrow_mut();
                match vars.get_mut(name) {
                    Some(existing) => {
                        existing.value.push(' ');
                        existing.value.push_str(body);
                    }
                    None => {
                        vars.insert(
                            name.to_string(),
                            Variable {
                                value: body.to_string(),
                                flavor,
                                origin,
                            },
                        );
                    }
                }
            }
            AssignOp::Simple => {
                let expanded = expand::expand(body, self);
                self.set_var_with_origin(name, &expanded, flavor, origin);
            }
            AssignOp::ImmediateRecursive => {
                let expanded = expand::expand(body, self);
                // Escape `$` → `$$` so the stored recursive value
                // round-trips through one more expansion without
                // losing literal dollar signs (same as `:::=` in
                // `process_assignment`).
                let escaped = expanded.replace('$', "$$");
                self.set_var_with_origin(name, &escaped, VarFlavor::Recursive, origin);
            }
            AssignOp::Shell => {
                let cmd = expand::expand(body, self);
                let shell = self.lookup_var_or("SHELL", "/bin/sh");
                let shell_flags = self.lookup_var_or(".SHELLFLAGS", "-c");
                let mut shell_cmd = std::process::Command::new(&shell);
                for flag in shell_flags.split_whitespace() {
                    shell_cmd.arg(flag);
                }
                let output = shell_cmd
                    .arg(&cmd)
                    .output()
                    .map(|o| {
                        let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                        if s.ends_with('\n') {
                            s.pop();
                        }
                        s.replace('\n', " ")
                    })
                    .unwrap_or_default();
                self.set_var_with_origin(name, &output, VarFlavor::Recursive, origin);
            }
            AssignOp::Recursive => {
                self.set_var_with_origin(name, body, flavor, origin);
            }
        }
    }

    pub fn eval_text(&self, text: &str) {
        // `$(eval ...)` must take effect immediately so a subsequent
        // expansion in the same recipe / variable sees the updated state.
        let mut parser = crate::parser::Parser::new(text);
        if let Ok(directives) = parser.parse() {
            self.load_makefile(&directives);
        }
    }

    fn process_eval_queue(&self) {
        loop {
            let queue: Vec<String> = self.eval_queue.borrow_mut().drain(..).collect();
            if queue.is_empty() {
                break;
            }
            for text in queue {
                let mut parser = crate::parser::Parser::new(&text);
                if let Ok(directives) = parser.parse() {
                    for d in &directives {
                        self.process_directive(d);
                    }
                }
            }
        }
    }

    fn process_directive(&self, directive: &Directive) {
        match directive {
            Directive::Assignment(assign, source, line_no) => {
                *self.current_source.borrow_mut() = Some((source.clone(), *line_no));
                self.process_assignment(assign, VarOrigin::File);
                // Only record source location if the assignment actually took
                // effect (i.e., wasn't overridden by a higher-priority origin
                // like CommandLine). This ensures that unterminated-reference
                // errors correctly trace through the actual expansion chain.
                let expanded_name = expand::expand(&assign.name, self);
                let origin = self.vars.borrow().get(&expanded_name).map(|v| v.origin);
                if origin == Some(VarOrigin::File) {
                    self.var_source_locs
                        .borrow_mut()
                        .insert(expanded_name, (source.clone(), *line_no));
                }
                *self.current_source.borrow_mut() = None;
            }
            Directive::Rule(rule) => {
                self.process_rule(rule);
            }
            Directive::Include(files, silent, source, line_no) => {
                for file_pattern in files {
                    let expanded = expand::expand(file_pattern, self);
                    for file in expanded.split_whitespace() {
                        let mut matched = false;
                        let mut loaded = false;
                        if let Ok(paths) = glob::glob(file) {
                            for entry in paths.flatten() {
                                let path = entry.to_string_lossy().to_string();
                                // Try to load; suppress errors during initial parse
                                // since finalize_includes will retry with proper
                                // error reporting after attempting to rebuild.
                                if self.load_file_with_loc(&path, true, None) {
                                    loaded = true;
                                    self.included_files.borrow_mut().push(path.clone());
                                }
                                matched = true;
                            }
                        }
                        if !matched || !loaded {
                            // Defer: a rule to build this file may appear
                            // later in the same makefile. `finalize_includes`
                            // retries after the full parse.
                            self.pending_includes.borrow_mut().push((
                                file.to_string(),
                                *silent,
                                source.clone(),
                                *line_no,
                            ));
                        }
                    }
                }
            }
            Directive::Conditional(cond) => {
                self.process_conditional(cond);
            }
            Directive::Export(vars) => {
                if let Some(names) = vars {
                    let mut exports = self.exports.borrow_mut();
                    for name in names {
                        let expanded = expand::expand(name, self);
                        for word in expanded.split_whitespace() {
                            exports.insert(word.to_string());
                        }
                    }
                } else {
                    *self.export_all.borrow_mut() = true;
                }
            }
            Directive::ExportAssign(assign) => {
                // `export VAR = value` is an assignment plus an export.
                self.process_assignment(assign, VarOrigin::File);
                let expanded = expand::expand(&assign.name, self);
                self.exports.borrow_mut().insert(expanded);
            }
            Directive::Unexport(vars) => {
                if let Some(names) = vars {
                    let mut exports = self.exports.borrow_mut();
                    let mut unexports = self.unexports.borrow_mut();
                    for name in names {
                        let expanded = expand::expand(name, self);
                        for word in expanded.split_whitespace() {
                            exports.remove(word);
                            unexports.insert(word.to_string());
                        }
                    }
                } else {
                    *self.export_all.borrow_mut() = false;
                }
            }
            Directive::UnexportAssign(assign) => {
                // `unexport VAR = value` assigns and marks unexported.
                self.process_assignment(assign, VarOrigin::File);
                let expanded = expand::expand(&assign.name, self);
                self.exports.borrow_mut().remove(&expanded);
                self.unexports.borrow_mut().insert(expanded);
            }
            Directive::Override(assign) => {
                self.process_assignment(assign, VarOrigin::Override);
            }
            Directive::Define(name, op, lines, source, line_no) => {
                let expanded_name = expand::expand(name, self);
                if expanded_name.trim().is_empty() {
                    eprintln!("{source}:{line_no}: *** empty variable name.  Stop.");
                    std::process::exit(2);
                }
                let body = lines.join("\n");
                self.apply_define(&expanded_name, *op, &body, VarOrigin::File);
            }
            Directive::OverrideDefine(name, op, lines, source, line_no) => {
                let expanded_name = expand::expand(name, self);
                if expanded_name.trim().is_empty() {
                    eprintln!("{source}:{line_no}: *** empty variable name.  Stop.");
                    std::process::exit(2);
                }
                let body = lines.join("\n");
                self.apply_define(&expanded_name, *op, &body, VarOrigin::Override);
            }
            Directive::Undefine(name, source, line_no) => {
                // `undefine` from a makefile doesn't clobber command-line
                // or override variables; those still win.
                let expanded = expand::expand(name, self);
                if expanded.trim().is_empty() {
                    eprintln!("{source}:{line_no}: *** empty variable name.  Stop.");
                    std::process::exit(2);
                }
                let mut vars = self.vars.borrow_mut();
                let remove = matches!(
                    vars.get(&expanded).map(|v| v.origin),
                    Some(VarOrigin::File) | Some(VarOrigin::Environment) | Some(VarOrigin::Default)
                );
                if remove {
                    vars.remove(&expanded);
                }
            }
            Directive::OverrideUndefine(name, source, line_no) => {
                // `override undefine VAR` force-removes the variable
                // regardless of origin.
                let expanded = expand::expand(name, self);
                if expanded.trim().is_empty() {
                    eprintln!("{source}:{line_no}: *** empty variable name.  Stop.");
                    std::process::exit(2);
                }
                self.vars.borrow_mut().remove(&expanded);
            }
            Directive::TargetVarAssign(targets_str, assign) => {
                let targets_expanded = expand::expand(targets_str, self);
                // Simple (`:=` / `::=`) and shell (`!=`) target-specific
                // assignments are expanded at declaration time so they
                // capture the *current* binding (e.g. a foreach var).
                let value = match assign.op {
                    AssignOp::Simple => expand::expand(&assign.value, self),
                    _ => assign.value.clone(),
                };
                // Parser prefixes the name with `^` for override and
                // `!`/`~` for export/unexport. Strip them off and
                // record in the appropriate data structures.
                let is_override = assign.name.starts_with('^');
                let raw_name = if is_override {
                    &assign.name[1..]
                } else {
                    &assign.name
                };
                let (name, do_export, do_unexport) = if let Some(rest) = raw_name.strip_prefix('!')
                {
                    (rest.to_string(), true, false)
                } else if let Some(rest) = raw_name.strip_prefix('~') {
                    (rest.to_string(), false, true)
                } else {
                    (raw_name.to_string(), false, false)
                };
                let (name, is_private) = if let Some(rest) = name.strip_prefix('@') {
                    (rest.to_string(), true)
                } else {
                    (name, false)
                };
                let mut tv = self.target_vars.borrow_mut();
                let mut pv = self.pattern_vars.borrow_mut();
                for target in targets_expanded.split_whitespace() {
                    if target.contains('%') {
                        // Pattern-specific variable
                        pv.push((
                            target.to_string(),
                            name.clone(),
                            assign.op,
                            value.clone(),
                            is_override,
                            is_private,
                        ));
                    } else {
                        tv.entry(target.to_string()).or_default().push((
                            name.clone(),
                            assign.op,
                            value.clone(),
                            is_override,
                            is_private,
                        ));
                    }
                }
                drop(tv);
                drop(pv);
                if do_export {
                    let mut te = self.target_exports.borrow_mut();
                    let mut pe = self.pattern_exports.borrow_mut();
                    for target in targets_expanded.split_whitespace() {
                        if target.contains('%') {
                            pe.push((target.to_string(), name.clone()));
                        } else {
                            te.entry(target.to_string()).or_default().push(name.clone());
                        }
                    }
                }
                if do_unexport {
                    // Target-specific `unexport` — record the names so
                    // execute_recipe can add them to `unexports` for the
                    // duration of the target's recipe.
                    let mut te = self.target_exports.borrow_mut();
                    let mut pe = self.pattern_exports.borrow_mut();
                    for target in targets_expanded.split_whitespace() {
                        // Use a `~` prefix to distinguish unexport entries
                        // from export entries in the same map.
                        if target.contains('%') {
                            pe.push((target.to_string(), format!("~{}", name)));
                        } else {
                            te.entry(target.to_string())
                                .or_default()
                                .push(format!("~{}", name));
                        }
                    }
                }
            }
            Directive::PrivateAssign(assign, source, line_no) => {
                *self.current_source.borrow_mut() = Some((source.clone(), *line_no));
                self.process_assignment(assign, VarOrigin::File);
                let expanded_name = expand::expand(&assign.name, self);
                self.private_vars.borrow_mut().insert(expanded_name);
                *self.current_source.borrow_mut() = None;
            }
            Directive::PrivateExportAssign(assign) => {
                self.process_assignment(assign, VarOrigin::File);
                let expanded_name = expand::expand(&assign.name, self);
                self.exports.borrow_mut().insert(expanded_name.clone());
                self.private_vars.borrow_mut().insert(expanded_name.clone());
                self.private_exports.borrow_mut().insert(expanded_name);
            }
            Directive::Vpath(spec) => {
                match spec {
                    None => {
                        // `vpath` with no args clears all patterns.
                        self.vpath_patterns.borrow_mut().clear();
                    }
                    Some((pat_raw, dirs_raw)) => {
                        let pat = expand::expand(pat_raw, self);
                        let dirs_str = expand::expand(dirs_raw, self);
                        let dirs: Vec<String> = dirs_str
                            .split(&[':', ' ', '\t'][..])
                            .filter(|s| !s.is_empty())
                            .map(|s| s.trim_end_matches('/').to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        let mut vp = self.vpath_patterns.borrow_mut();
                        if dirs.is_empty() {
                            // `vpath PATTERN` clears entries for that pattern.
                            vp.retain(|(p, _)| p != &pat);
                        } else {
                            // GNU make appends a new entry for each `vpath`
                            // directive, even when the pattern is identical
                            // to an earlier one. This matters for search
                            // order: `vpath % a b` then `vpath % c` means
                            // search a, b, c in that order.
                            vp.push((pat, dirs));
                        }
                    }
                }
            }
            Directive::Expand(expr, source, line_no) => {
                // Evaluate a bare expression for side effects. The
                // expansion's textual result is discarded (GNU make behavior
                // for a standalone `$(info ...)`, `$(error ...)`, etc.).
                // Record the source location so `$(error)` / `$(warning)`
                // inside the expression can format `file:line:` diagnostics.
                *self.current_source.borrow_mut() = Some((source.clone(), *line_no));
                let _ = expand::expand(expr, self);
                *self.current_source.borrow_mut() = None;
            }
            Directive::RecipeLine(text, line_no) => {
                // Append to the most recent explicit rule's latest entry
                // (one per target, since `process_rule` pushes one entry
                // per target in the rule). Silent no-op if no rule is
                // in scope — GNU make would have warned already at
                // parse time about a stray recipe line.
                let targets = self.last_rule_targets.borrow().clone();
                if let Some(targets) = targets {
                    let mut rules = self.rules.borrow_mut();
                    for t in &targets {
                        if let Some(entries) = rules.get_mut(t)
                            && let Some(last) = entries.last_mut()
                        {
                            last.recipe.push(text.clone());
                            last.recipe_lines.push(*line_no);
                        }
                    }
                }
            }
        }
    }

    fn process_assignment(&self, assign: &Assignment, origin: VarOrigin) {
        let name = expand::expand(&assign.name, self);

        match assign.op {
            AssignOp::Simple => {
                let value = expand::expand(&assign.value, self);
                self.set_var_with_origin(&name, &value, VarFlavor::Simple, origin);
            }
            AssignOp::ImmediateRecursive => {
                // `:::=` expands RHS immediately. The result is stored
                // as recursive flavor so a later `+=` appends raw text
                // that WILL be re-expanded on lookup. To avoid double
                // expansion of the original RHS, `$` is escaped to `$$`
                // so the one additional expansion round is a no-op for
                // the already-expanded portion.
                let expanded = expand::expand(&assign.value, self);
                let escaped = expanded.replace('$', "$$");
                self.set_var_with_origin(&name, &escaped, VarFlavor::Recursive, origin);
            }
            AssignOp::Recursive => {
                self.set_var_with_origin(&name, &assign.value, VarFlavor::Recursive, origin);
            }
            AssignOp::Conditional => {
                if !self.is_var_defined(&name) {
                    self.set_var_with_origin(&name, &assign.value, VarFlavor::Recursive, origin);
                }
            }
            AssignOp::Append => {
                let existing_flavor = self.var_flavor(&name);
                let existing = self.lookup_var_raw(&name);
                // GNU make: if var is simple-flavored, RHS is expanded at
                // append time. Recursive / new vars keep RHS unexpanded.
                // Vars assigned with `:::=` keep append RHS raw too
                // (recursive semantics) even though the stored flavor is
                // Simple.
                let keep_raw = existing_flavor != VarFlavor::Simple
                    || self.immediate_recursive.borrow().contains(&name);
                let rhs = if keep_raw {
                    assign.value.clone()
                } else {
                    expand::expand(&assign.value, self)
                };
                let new_value = if existing.is_empty() {
                    rhs
                } else if rhs.is_empty() {
                    existing
                } else {
                    format!("{existing} {rhs}")
                };
                let flavor = if existing_flavor == VarFlavor::Undefined {
                    VarFlavor::Recursive
                } else {
                    existing_flavor
                };
                self.set_var_with_origin(&name, &new_value, flavor, origin);
            }
            AssignOp::Shell => {
                let cmd = expand::expand(&assign.value, self);
                let shell = self.lookup_var_or("SHELL", "/bin/sh");
                let shell_flags = self.lookup_var_or(".SHELLFLAGS", "-c");
                let mut shell_cmd = std::process::Command::new(&shell);
                for flag in shell_flags.split_whitespace() {
                    shell_cmd.arg(flag);
                }
                // Always export MAKEFLAGS, MAKELEVEL, and MAKE for sub-make
                // compatibility (GNU make exports these to all children).
                shell_cmd.env("MAKEFLAGS", self.lookup_var("MAKEFLAGS"));
                let makelevel: i32 = self.lookup_var_or("MAKELEVEL", "0").parse().unwrap_or(0);
                shell_cmd.env("MAKELEVEL", (makelevel + 1).to_string());
                shell_cmd.env("MAKE", self.lookup_var("MAKE"));
                let output = shell_cmd
                    .arg(&cmd)
                    .output()
                    .map(|o| {
                        // GNU make: strip exactly the final trailing `\n`
                        // (if any), then convert remaining `\n` into spaces.
                        let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                        if s.ends_with('\n') {
                            s.pop();
                        }
                        s.replace('\n', " ")
                    })
                    .unwrap_or_default();
                // The `!=` flavor is recursively-expanded in GNU make so
                // that `$(var)` references inside its value expand lazily.
                self.set_var_with_origin(&name, &output, VarFlavor::Recursive, origin);
            }
        }
    }

    fn process_rule(&self, rule: &Rule) {
        // Expand targets and prerequisites. Backslash-escaped spaces in
        // target tokens (already collapsed to literal spaces by the
        // parser's escape-aware splitter) are preserved here by using
        // a sentinel byte during the post-expansion split.
        const SENT: char = '\x01';
        fn split_unescaped_ws(s: &str) -> Vec<String> {
            // First, replace any pre-existing literal spaces in the source
            // string with the sentinel so they survive split_whitespace.
            // (Strings reaching here from parser tokens have already had
            // `\\ ` collapsed to a literal space.)
            // Then split on whitespace and convert sentinels back.
            // Inputs from variable expansion never contain SENT (control
            // char), so this is safe.
            let with_sent = s.replace(' ', &SENT.to_string());
            // The original \t / \n separators in expansion output should
            // still split. Restore literal sentinel for tabs not present
            // here (only ' ' was preserved as sentinel above).
            with_sent
                .split(|c: char| c.is_whitespace() && c != SENT)
                .filter(|t| !t.is_empty())
                .map(|t| t.replace(SENT, " "))
                .collect()
        }
        let targets: Vec<String> = rule
            .targets
            .iter()
            .flat_map(|t| {
                // If the parser-token contains a literal space (from a
                // `\\<space>` escape) treat the whole token as a single
                // target after expansion. Otherwise, expand and split on
                // whitespace as usual. Spaces *inside* a `$(...)` /
                // `${...}` reference (e.g. `$(filter %.o,$(files))`) are
                // not escape-spaces and must be ignored here.
                fn has_unwrapped_space(s: &str) -> bool {
                    let bytes = s.as_bytes();
                    let mut paren = 0i32;
                    let mut brace = 0i32;
                    let mut i = 0;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == b'$' && i + 1 < bytes.len() {
                            match bytes[i + 1] {
                                b'(' => {
                                    paren += 1;
                                    i += 2;
                                    continue;
                                }
                                b'{' => {
                                    brace += 1;
                                    i += 2;
                                    continue;
                                }
                                b'$' => {
                                    i += 2;
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        if (c == b'(' || c == b'{') && (paren > 0 || brace > 0) {
                            if c == b'(' {
                                paren += 1;
                            } else {
                                brace += 1;
                            }
                        } else if c == b')' && paren > 0 {
                            paren -= 1;
                        } else if c == b'}' && brace > 0 {
                            brace -= 1;
                        } else if paren == 0 && brace == 0 && c == b' ' {
                            return true;
                        }
                        i += 1;
                    }
                    false
                }
                if has_unwrapped_space(t) {
                    vec![expand::expand(t, self)]
                } else {
                    expand::expand(t, self)
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                }
            })
            .flat_map(|t| {
                // Expand filesystem globs in target names. Skip pattern
                // rule targets (containing `%`) and unmatched globs are
                // kept literal.
                if !t.contains('%')
                    && t.contains(['*', '?', '['])
                    && let Ok(paths) = glob::glob(&t)
                {
                    let matched: Vec<String> = paths
                        .flatten()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    if !matched.is_empty() {
                        return matched;
                    }
                }
                vec![t]
            })
            .collect();
        let _ = split_unescaped_ws; // currently unused; reserved

        // After expansion, a target token may contain an unescaped `:`
        // (e.g. `path = pre:` then `$(path)foo : ;`). In GNU make this
        // re-parses as a static pattern rule with the middle field
        // being the part after the embedded colon — and since that
        // middle field has no `%`, it's a fatal error.
        // Check BEFORE we unescape so `foo\:bar` (literal `:`) doesn't
        // trigger this.
        for t in &targets {
            if rule.pattern.is_none() && contains_unescaped_colon(t) && !t.contains('%') {
                eprintln!(
                    "{}:{}: *** target pattern contains no '%'.  Stop.",
                    rule.source_name, rule.line_no
                );
                std::process::exit(2);
            }
        }
        let targets: Vec<String> = targets.into_iter().map(|t| unescape_name(&t)).collect();

        // Join prereqs before expansion so `$<space>` and similar
        // single-char references span what the parser split apart.
        // A `|` in the expansion output (e.g. from
        // `$(var)` where var contains a pipe) splits normal from
        // order-only prereqs — GNU make re-parses the expanded text.
        let se_active = self.second_expansion_enabled.get();
        let prereq_text_full = expand::expand(&rule.prerequisites.join(" "), self);
        let extra_order_only_text = expand::expand(&rule.order_only.join(" "), self);
        // For second-expansion rules, save the FULL first-pass-expanded text
        // (including any $| sequences) — | split is deferred to second pass.
        let raw_prereq_for_se = if se_active {
            Some(prereq_text_full.clone())
        } else {
            None
        };
        let raw_orderonly_for_se = if se_active {
            Some(extra_order_only_text.clone())
        } else {
            None
        };
        let is_pure_pattern = rule.pattern.is_some() && targets.iter().any(|t| t.contains('%'));
        if se_active && !is_pure_pattern {
            if let Some(t) = &raw_prereq_for_se {
                validate_balanced_refs(t, &rule.source_name, rule.line_no);
            }
            if let Some(t) = &raw_orderonly_for_se {
                validate_balanced_refs(t, &rule.source_name, rule.line_no);
            }
        }
        let (prereq_text, post_pipe_order_only) =
            if let Some(idx) = find_orderonly_pipe(&prereq_text_full) {
                (
                    prereq_text_full[..idx].to_string(),
                    prereq_text_full[idx + 1..].to_string(),
                )
            } else {
                (prereq_text_full, String::new())
            };
        let prereqs: Vec<String> = prereq_text
            .split_whitespace()
            .map(unescape_name)
            .flat_map(|s| {
                let s = s.as_str();
                // Expand filesystem globs in prerequisites. Unmatched
                // patterns retain literal form so an explicit rule can
                // still build them.
                if s.contains(['*', '?', '['])
                    && let Ok(paths) = glob::glob(s)
                {
                    let matched: Vec<String> = paths
                        .flatten()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    if !matched.is_empty() {
                        return matched;
                    }
                }
                vec![s.to_string()]
            })
            .collect();
        let order_only: Vec<String> = format!("{post_pipe_order_only} {extra_order_only_text}")
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();
        // GNU make forbids defining new prerequisites inside a recipe
        // (Savannah bug #12124). When $(eval) is called during recipe
        // expansion and the eval'd text contains a rule with prereqs,
        // error out.
        if *self.in_recipe.borrow() && (!prereqs.is_empty() || !order_only.is_empty()) {
            let (src, line) = self
                .current_source
                .borrow()
                .clone()
                .unwrap_or_else(|| (rule.source_name.clone(), rule.line_no));
            // GNU make uses `make: ***` (without source prefix) when
            // the source is stdin (`-`); otherwise it prefixes with
            // `<file>:<line>:`.
            if src == "-" {
                eprintln!("make: *** prerequisites cannot be defined in recipes.  Stop.");
            } else {
                eprintln!("{src}:{line}: *** prerequisites cannot be defined in recipes.  Stop.");
            }
            std::process::exit(2);
        }

        // Handle special targets. A rule line may combine a special target
        // with normal ones (e.g. `.DEFAULT all:`). We handle each special
        // in turn and continue processing non-special targets normally.
        let mut normal_targets: Vec<String> = Vec::new();
        for target in &targets {
            match target.as_str() {
                ".PHONY" => {
                    for p in &prereqs {
                        self.phony_targets.borrow_mut().insert(p.clone());
                    }
                }
                ".SUFFIXES" => {
                    if prereqs.is_empty() {
                        self.suffixes.borrow_mut().clear();
                        *self.suffixes_cleared.borrow_mut() = true;
                    } else {
                        self.suffixes.borrow_mut().extend(prereqs.clone());
                    }
                }
                ".SILENT" => {
                    if prereqs.is_empty() {
                        *self.silent_all.borrow_mut() = true;
                    } else {
                        let mut s = self.silent_targets.borrow_mut();
                        for p in &prereqs {
                            s.insert(p.clone());
                        }
                    }
                }
                ".DEFAULT" => {
                    if !rule.recipe.is_empty() {
                        *self.default_rule.borrow_mut() = Some((
                            rule.recipe.clone(),
                            rule.recipe_lines.clone(),
                            rule.source_name.clone(),
                        ));
                    }
                }
                ".POSIX" => {
                    // POSIX mode changes .SHELLFLAGS to include -e so
                    *self.posix_mode.borrow_mut() = true;
                    // each recipe line's shell stops at the first
                    // failing command. Lines prefixed with `-` have
                    // the `-e` stripped at exec time so the remaining
                    // commands after `;` still run (matches GNU make).
                    self.set_var_with_origin(
                        ".SHELLFLAGS",
                        "-ec",
                        VarFlavor::Simple,
                        VarOrigin::Default,
                    );
                    // POSIX-standard defaults for built-in variables
                    // (see IEEE Std 1003.1-2008 `make` utility).
                    for (name, val) in [
                        ("ARFLAGS", "-rv"),
                        ("CC", "c99"),
                        ("CFLAGS", "-O1"),
                        ("FC", "fort77"),
                        ("FFLAGS", "-O1"),
                        ("LEX", "lex"),
                        ("SCCSGETFLAGS", "-s"),
                        ("YACC", "yacc"),
                    ] {
                        self.set_var_with_origin(
                            name,
                            val,
                            VarFlavor::Recursive,
                            VarOrigin::Default,
                        );
                    }
                }
                ".SECONDEXPANSION" => {
                    self.second_expansion_enabled.set(true);
                }
                ".DELETE_ON_ERROR" => {
                    *self.delete_on_error.borrow_mut() = true;
                }
                ".SECONDARY" => {
                    if prereqs.is_empty() {
                        if self.notintermediate_all.get() {
                            eprintln!(
                                "make: *** .NOTINTERMEDIATE and .SECONDARY are mutually exclusive.  Stop."
                            );
                            std::process::exit(2);
                        }
                        self.secondary_all.set(true);
                    } else {
                        for p in &prereqs {
                            if self.notintermediate_files.borrow().contains(p) {
                                eprintln!(
                                    "make: *** {} cannot be both .NOTINTERMEDIATE and .SECONDARY.  Stop.",
                                    p
                                );
                                std::process::exit(2);
                            }
                        }
                        self.secondary_targets.borrow_mut().extend(prereqs.clone());
                    }
                }
                ".INTERMEDIATE" => {
                    for p in &prereqs {
                        if self.notintermediate_files.borrow().contains(p) {
                            eprintln!(
                                "make: *** {} cannot be both .NOTINTERMEDIATE and .INTERMEDIATE.  Stop.",
                                p
                            );
                            std::process::exit(2);
                        }
                    }
                    self.intermediate_targets
                        .borrow_mut()
                        .extend(prereqs.clone());
                }
                ".NOTINTERMEDIATE" => {
                    if prereqs.is_empty() {
                        // Global form: .NOTINTERMEDIATE:
                        if self.secondary_all.get() {
                            eprintln!(
                                "make: *** .NOTINTERMEDIATE and .SECONDARY are mutually exclusive.  Stop."
                            );
                            std::process::exit(2);
                        }
                        self.notintermediate_all.set(true);
                    } else {
                        for p in &prereqs {
                            if p.contains('%') {
                                // Pattern form: .NOTINTERMEDIATE: %.x
                                self.notintermediate_patterns.borrow_mut().push(p.clone());
                            } else {
                                // Explicit file form: .NOTINTERMEDIATE: hello.x
                                if self.intermediate_targets.borrow().contains(p) {
                                    eprintln!(
                                        "make: *** {} cannot be both .NOTINTERMEDIATE and .INTERMEDIATE.  Stop.",
                                        p
                                    );
                                    std::process::exit(2);
                                }
                                if self.secondary_targets.borrow().contains(p) {
                                    eprintln!(
                                        "make: *** {} cannot be both .NOTINTERMEDIATE and .SECONDARY.  Stop.",
                                        p
                                    );
                                    std::process::exit(2);
                                }
                                self.notintermediate_files.borrow_mut().insert(p.clone());
                            }
                        }
                    }
                }
                ".WAIT" => {
                    // GNU make 4.4: .WAIT is a synchronization marker in
                    // prerequisite lists. Declaring it as a target with
                    // prereqs or a recipe is invalid (warn, then ignore).
                    // An empty `.WAIT:` declaration is harmless and is kept
                    // as a normal target so users can write it for
                    // backwards compatibility.
                    if !prereqs.is_empty() {
                        eprintln!(
                            "{}:{}: .WAIT should not have prerequisites",
                            rule.source_name, rule.line_no
                        );
                    }
                    if !rule.recipe.is_empty() {
                        eprintln!(
                            "{}:{}: .WAIT should not have commands",
                            rule.source_name, rule.line_no
                        );
                    }
                    if prereqs.is_empty() && rule.recipe.is_empty() {
                        // Treat as a no-op rule registration.
                        normal_targets.push(target.clone());
                    }
                }
                ".PRECIOUS"
                | ".IGNORE"
                | ".EXPORT_ALL_VARIABLES"
                | ".NOTPARALLEL"
                | ".ONESHELL" => {
                    if target == ".EXPORT_ALL_VARIABLES" {
                        *self.export_all.borrow_mut() = true;
                    }
                    if target == ".NOTPARALLEL" {
                        self.notparallel.set(true);
                    }
                    if target == ".ONESHELL" {
                        self.oneshell.set(true);
                    }
                    if target == ".PRECIOUS" {
                        for p in &prereqs {
                            if p.contains('%') {
                                self.precious_patterns.borrow_mut().push(p.clone());
                            } else {
                                self.precious_targets.borrow_mut().insert(p.clone());
                            }
                        }
                    }
                }
                _ => normal_targets.push(target.clone()),
            }
        }
        // If the rule was purely special targets, stop here. Otherwise
        // continue processing as if only the non-special targets were
        // listed.
        if normal_targets.is_empty() {
            return;
        }
        let targets = normal_targets;

        // Old-style suffix rules: .c.o: → %.o: %.c
        // A target like ".XY" where .X and .Y are known suffixes.
        // In non-POSIX mode, a suffix rule with prerequisites still
        // generates a pattern rule (prereqs are ignored) but we also
        // warn and fall through to register the explicit rule.
        if targets.len() == 1 && targets[0].starts_with('.') {
            let target = &targets[0];
            if let Some((src_suffix, dst_suffix)) = self.parse_suffix_rule(target) {
                if !prereqs.is_empty() && *self.posix_mode.borrow() {
                    // POSIX mode: suffix rule with prereqs is just a
                    // normal rule — no pattern rule, no warning.
                    // Fall through to explicit-rule handling below.
                } else {
                    if !prereqs.is_empty() {
                        // Non-POSIX mode: warn and still create the
                        // pattern rule, then fall through to also
                        // register the explicit rule.
                        eprintln!(
                            "{}:{}: warning: ignoring prerequisites on suffix rule definition",
                            rule.source_name, rule.line_no
                        );
                    }
                    self.pattern_rules.borrow_mut().push(PatternRuleEntry {
                        target_pattern: format!("%{dst_suffix}"),
                        prereq_patterns: vec![format!("%{src_suffix}")],
                        order_only_patterns: Vec::new(),
                        recipe: rule.recipe.clone(),
                        recipe_lines: rule.recipe_lines.clone(),
                        source_name: rule.source_name.clone(),
                        is_terminal: false,
                        sibling_patterns: Vec::new(),
                        second_expand: false,
                        raw_prereq_text: None,
                        raw_order_only_text: None,
                        is_grouped: false,
                    });
                    if prereqs.is_empty() {
                        return;
                    }
                    // Non-POSIX with prereqs: fall through to also
                    // register `.src.dst: prereqs` as explicit rule.
                }
            }
        }

        // Static pattern rule: `targets: target-pattern: prereq-patterns`.
        // The targets are concrete; each one's stem is derived by
        // matching against the target-pattern, and prereqs are formed
        // by replacing `%` in the prereq patterns with the stem.
        // Register as explicit rules so normal lookup finds them.
        if let Some(pattern) = &rule.pattern
            && !targets.iter().any(|t| expand::has_unescaped_percent(t))
        {
            let target_pattern = expand::expand(&pattern.target_pattern, self);
            let expanded_prereq_patterns: Vec<String> = pattern
                .prereq_patterns
                .iter()
                .flat_map(|p| {
                    expand::expand(p, self)
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                })
                .collect();
            for raw_target in &targets {
                let target = expand::unescape_percent(raw_target);
                let stem = match expand::pattern_stem(&target, &target_pattern) {
                    Some(s) => s,
                    None => {
                        eprintln!("make: target '{target}' doesn't match the target pattern");
                        continue;
                    }
                };
                let resolved_prereqs: Vec<String> = if se_active {
                    // SE: defer all prereq computation to second pass.
                    Vec::new()
                } else {
                    expanded_prereq_patterns
                        .iter()
                        .map(|p| expand::replace_first_unescaped_percent(p, &stem))
                        .filter(|s| !s.is_empty())
                        .collect()
                };
                // For SE: build per-target raw text by substituting %->stem into the
                // raw (joined, first-pass-expanded) prereq patterns first.
                // Escape `$` in stem so substitution is literal (won't
                // re-trigger variable expansion at second-expansion time).
                let stem_lit = stem.replace('$', "$$");
                let raw_pr_per_target = if se_active {
                    // Re-derive raw text from the original pattern (with $$ already expanded once).
                    // pattern.prereq_patterns each first-pass-expanded then joined.
                    let joined: String = pattern
                        .prereq_patterns
                        .iter()
                        .map(|p| expand::expand(p, self))
                        .collect::<Vec<_>>()
                        .join(" ");
                    Some(replace_first_percent_per_token(&joined, &stem_lit, false))
                } else {
                    None
                };
                let mut rules = self.rules.borrow_mut();
                let raw_oo_per_target = raw_orderonly_for_se
                    .as_ref()
                    .map(|t| replace_first_percent_per_token(t, &stem_lit, false));
                if se_active {
                    if let Some(t) = &raw_pr_per_target {
                        validate_balanced_refs(t, &rule.source_name, rule.line_no);
                    }
                    if let Some(t) = &raw_oo_per_target {
                        validate_balanced_refs(t, &rule.source_name, rule.line_no);
                    }
                }
                rules
                    .entry(target.to_string())
                    .or_default()
                    .push(RuleEntry {
                        prerequisites: resolved_prereqs,
                        order_only: order_only.clone(),
                        recipe: rule.recipe.clone(),
                        recipe_lines: rule.recipe_lines.clone(),
                        source_name: rule.source_name.clone(),
                        is_double_colon: rule.is_double_colon,
                        group: Vec::new(),
                        stem: Some(stem.clone()),
                        second_expand: se_active,
                        raw_prereq_text: raw_pr_per_target,
                        raw_order_only_text: raw_oo_per_target,
                    });
            }
            if !*self.suppress_default_goal.borrow() {
                let mut default = self.default_goal.borrow_mut();
                if default.is_none()
                    && let Some(t) = targets.iter().find(|t| !t.starts_with('.'))
                {
                    // Escape `$` to `$$` because default_goal is re-expanded
                    // at consumption time. Targets here have already had
                    // their first-pass `$$`->`$` collapse applied by
                    // expand::expand, so a literal `$` in the target name
                    // (from `foo$$bar`) would otherwise be incorrectly expanded.
                    let unesc = expand::unescape_percent(t);
                    *default = Some(unesc.replace('$', "$$").replace(' ', "\x01"));
                }
            }
            *self.last_rule_targets.borrow_mut() = Some(
                targets
                    .iter()
                    .map(|t| expand::unescape_percent(t))
                    .collect(),
            );
            return;
        }

        // Pattern rule
        if let Some(pattern) = &rule.pattern {
            // For SE pattern rules: store the joined (first-pass-expanded) raw text
            // so % can be replaced with stem then second-expanded at build time.
            let raw_pat_prereq_text: Option<String> = if se_active {
                Some(
                    pattern
                        .prereq_patterns
                        .iter()
                        .map(|p| expand::expand(p, self))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            } else {
                None
            };
            let raw_pat_orderonly_text: Option<String> = if se_active {
                Some(order_only.join(" "))
            } else {
                None
            };
            if se_active {
                if let Some(t) = &raw_pat_prereq_text {
                    validate_balanced_refs(t, "", 0);
                }
                if let Some(t) = &raw_pat_orderonly_text {
                    validate_balanced_refs(t, "", 0);
                }
            }
            for target_pat in &targets {
                self.pattern_rules.borrow_mut().push(PatternRuleEntry {
                    target_pattern: target_pat.clone(),
                    prereq_patterns: pattern
                        .prereq_patterns
                        .iter()
                        .flat_map(|p| {
                            expand::expand(p, self)
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                        })
                        .collect(),
                    order_only_patterns: order_only.clone(),
                    recipe: rule.recipe.clone(),
                    recipe_lines: rule.recipe_lines.clone(),
                    source_name: rule.source_name.clone(),
                    is_terminal: rule.is_double_colon,
                    sibling_patterns: targets
                        .iter()
                        .filter(|t| *t != target_pat)
                        .cloned()
                        .collect(),
                    second_expand: se_active,
                    raw_prereq_text: raw_pat_prereq_text.clone(),
                    raw_order_only_text: raw_pat_orderonly_text.clone(),
                    is_grouped: rule.is_grouped,
                });
            }
            return;
        }

        // Set default goal (unless suppressed — e.g. when loading files
        // from the MAKEFILES env var, which must not steal the default
        // goal from the primary makefile).
        if !*self.suppress_default_goal.borrow() {
            let mut default = self.default_goal.borrow_mut();
            if default.is_none() {
                for t in &targets {
                    if !t.starts_with('.') || t.contains('/') {
                        // Preserve `\\<space>`-escaped target names: store
                        // literal spaces as a sentinel control char that
                        // survives the consumer's split_whitespace, then
                        // gets restored when looking up the rule.
                        // Also escape `$` to `$$` because the consumer
                        // re-expands `default_goal` (to handle the
                        // `.DEFAULT_GOAL = $N` user-assignment case).
                        let escaped = t.replace('$', "$$").replace(' ', "\x01");
                        *default = Some(escaped);
                        break;
                    }
                }
            }
        }

        // Grouped targets (`&:`) must provide a recipe.
        if rule.is_grouped && rule.recipe.is_empty() {
            eprintln!(
                "{}:{}: *** grouped targets must provide a recipe.  Stop.",
                rule.source_name, rule.line_no
            );
            std::process::exit(2);
        }

        // Store explicit rules. For `&:` grouped rules, tag each
        // stored rule with the sibling targets so running the recipe
        // once marks them all as built.
        let group: Vec<String> = if rule.is_grouped {
            targets.clone()
        } else {
            Vec::new()
        };
        let entry = RuleEntry {
            prerequisites: prereqs,
            order_only,
            recipe: rule.recipe.clone(),
            recipe_lines: rule.recipe_lines.clone(),
            source_name: rule.source_name.clone(),
            is_double_colon: rule.is_double_colon,
            group,
            stem: None,
            second_expand: se_active,
            raw_prereq_text: raw_prereq_for_se,
            raw_order_only_text: raw_orderonly_for_se,
        };

        for target in &targets {
            self.rules
                .borrow_mut()
                .entry(target.clone())
                .or_default()
                .push(entry.clone());
        }
        *self.last_rule_targets.borrow_mut() = Some(targets.clone());
    }

    fn process_conditional(&self, cond: &Conditional) {
        let result = match &cond.kind {
            CondKind::Ifdef(var) => {
                let name = expand::expand(var, self);
                self.is_var_defined(&name)
            }
            CondKind::Ifndef(var) => {
                let name = expand::expand(var, self);
                !self.is_var_defined(&name)
            }
            CondKind::Ifeq(a, b) => {
                let a_val = expand::expand(a, self);
                let b_val = expand::expand(b, self);
                a_val == b_val
            }
            CondKind::Ifneq(a, b) => {
                let a_val = expand::expand(a, self);
                let b_val = expand::expand(b, self);
                a_val != b_val
            }
        };

        if result {
            self.load_makefile(&cond.then_body);
        } else if let Some(else_body) = &cond.else_body {
            self.load_makefile(else_body);
        }
    }

    pub fn load_file(&self, path: &str, silent: bool) {
        self.load_file_with_loc(path, silent, None);
    }

    /// Load a file with optional source location for error messages.
    pub fn load_file_with_loc(
        &self,
        path: &str,
        silent: bool,
        loc: Option<(String, usize)>,
    ) -> bool {
        // Resolve path: if it doesn't exist at the given (possibly relative)
        // location, search `include_dirs` in order (populated by `-I`).
        let resolved = if std::path::Path::new(path).exists() {
            path.to_string()
        } else {
            let mut found: Option<String> = None;
            for dir in self.include_dirs.borrow().iter() {
                let candidate = std::path::Path::new(dir).join(path);
                if candidate.exists() {
                    found = Some(candidate.to_string_lossy().to_string());
                    break;
                }
            }
            found.unwrap_or_else(|| path.to_string())
        };
        match std::fs::read_to_string(&resolved) {
            Ok(content) => {
                self.append_makefile_list(&resolved);
                let content = content.strip_prefix('\u{FEFF}').unwrap_or(&content);
                let mut parser = crate::parser::Parser::new_with_source(content, path.to_string());
                match parser.parse() {
                    Ok(directives) => self.load_makefile(&directives),
                    Err(e) => {
                        if !silent {
                            eprintln!("{}: {}", resolved, e);
                        }
                    }
                }
                true
            }
            Err(e) => {
                if !silent {
                    let msg = clean_io_error(&format!("{}", e));
                    if let Some((ref src, ln)) = loc {
                        eprintln!("{}:{}: {}: {}", src, ln, path, msg);
                    } else {
                        eprintln!("make: {}: {}", path, msg);
                    }
                }
                false
            }
        }
    }

    fn append_makefile_list(&self, path: &str) {
        let current = self.lookup_var_raw("MAKEFILE_LIST");
        let new_value = if current.is_empty() {
            path.to_string()
        } else {
            format!("{current} {path}")
        };
        self.set_var_with_origin(
            "MAKEFILE_LIST",
            &new_value,
            VarFlavor::Simple,
            VarOrigin::Default,
        );
    }

    /// Load a Makefile from a string (used for stdin input via `-f -`).
    pub fn load_string(&self, content: &str) {
        let mut parser = crate::parser::Parser::new(content);
        match parser.parse() {
            Ok(directives) => self.load_makefile(&directives),
            Err(e) => {
                eprintln!("make: stdin: {e}");
            }
        }
    }

    /// Build the specified targets.
    pub fn build(&self, targets: &[String]) -> i32 {
        let level: i32 = self.lookup_var_or("MAKELEVEL", "0").parse().unwrap_or(0);
        // GNU make auto-prints directory info for sub-makes (MAKELEVEL>0)
        // by default; `-w`/`--print-directory` forces on,
        // `--no-print-directory` forces off (last one wins on the CLI).
        let should_print = match self.print_directory_opt {
            Some(choice) => choice,
            None => level > 0,
        };
        let make_tag = if level == 0 {
            "make".to_string()
        } else {
            format!("make[{level}]")
        };
        let cwd_for_msg = std::env::current_dir().unwrap_or_default();
        // If this is a re-exec (MAKE_RESTARTS > 0), skip the Entering
        // message — the previous invocation already printed it.
        let restarts: i32 = std::env::var("MAKE_RESTARTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if self.printed_entering.get() {
            // Already printed in main before load_file
        } else if should_print && restarts == 0 {
            println!("{make_tag}: Entering directory '{}'", cwd_for_msg.display());
        }
        let inc_rc = self.finalize_includes();
        if inc_rc != 0 {
            if should_print {
                println!("{make_tag}: Leaving directory '{}'", cwd_for_msg.display());
            }
            return inc_rc;
        }

        // If an included makefile was remade, signal the caller to
        // re-exec the process (GNU make "restart" semantics).
        // Don't print Leaving — the re-exec'd process will print
        // Entering/Leaving from scratch.
        if self.needs_reexec.get() {
            return -1; // sentinel: caller should re-exec
        }

        let targets = if targets.is_empty() {
            match self.default_goal.borrow().as_ref() {
                Some(goal) => {
                    let expanded = expand::expand(goal, self);
                    let parts: Vec<String> = expanded
                        .split_whitespace()
                        .map(|s| s.replace('\x01', " "))
                        .collect();
                    if parts.is_empty() {
                        eprintln!("make: *** No targets.  Stop.");
                        return 2;
                    }
                    if parts.len() > 1 {
                        eprintln!("make: *** .DEFAULT_GOAL contains more than one target.  Stop.");
                        return 2;
                    }
                    parts
                }
                None => {
                    eprintln!("make: *** No targets.  Stop.");
                    return 2;
                }
            }
        } else {
            targets.to_vec()
        };

        // Apply --shuffle to top-level goals (e.g. `make --shuffle=reverse a b c`).
        let mut targets = targets;
        self.shuffle_prereqs(&mut targets);

        let mut had_error = false;
        for target in &targets {
            // Targets are already fully expanded — default_goal has
            // been expanded by the caller above, and command-line
            // targets are taken literally.
            let target_expanded = target.clone();
            *self.recipe_executed.borrow_mut() = false;
            *self.target_had_recipe.borrow_mut() = true;
            let before_built = self.built_targets.borrow().contains(&target_expanded);
            let is_group_built = self.group_built_targets.borrow().contains(&target_expanded);
            match self.build_target(&target_expanded) {
                Ok(()) => {
                    // Delete intermediate files collected during the build.
                    // This happens after the goal completes so all dependents
                    // have finished using the intermediates.
                    {
                        let mut pending = self.pending_intermediate_deletions.borrow_mut();
                        // GNU make groups intermediate deletions: pattern-
                        // derived prereqs (chained via implicit rules) are
                        // listed in reverse build order, while explicit
                        // prereqs marked `.INTERMEDIATE` retain forward order.
                        let drained: Vec<(String, bool)> = pending.drain(..).collect();
                        let mut to_delete: Vec<String> = Vec::new();
                        // Reversed pattern-derived first (LIFO chain unwind),
                        // then explicit-prereq intermediates in build order.
                        for (f, is_pat) in drained.iter().rev() {
                            if *is_pat && Path::new(f.as_str()).exists() {
                                to_delete.push(f.clone());
                            }
                        }
                        for (f, is_pat) in drained.iter() {
                            if !*is_pat && Path::new(f.as_str()).exists() {
                                to_delete.push(f.clone());
                            }
                        }
                        if !to_delete.is_empty() {
                            println!("rm {}", to_delete.join(" "));
                            for f in &to_delete {
                                let _ = std::fs::remove_file(f);
                            }
                        }
                    }
                    // If build_target returned Ok without running any recipe,
                    // emit GNU make's diagnostic. Not under -s/-q.
                    if !*self.recipe_executed.borrow()
                        && (!before_built || is_group_built)
                        && !self.question
                        && !self.silent
                        && !had_error
                    {
                        if is_group_built || !*self.target_had_recipe.borrow() {
                            println!("{make_tag}: Nothing to be done for '{target_expanded}'.");
                        } else {
                            println!("{make_tag}: '{target_expanded}' is up to date.");
                        }
                    }
                }
                Err(e) => {
                    had_error = true;
                    // Recipe errors (formatted as `[file:line: target] Error N`)
                    // don't get the trailing "Stop." diagnostic; dependency
                    // errors do. Empty string = already reported.
                    if e.is_empty() {
                        // nothing — caller already emitted the diagnostic
                    } else if e.starts_with('[') {
                        eprintln!("make: *** {e}");
                    } else {
                        eprintln!("make: *** {e}.  Stop.");
                    }
                    if !self.keep_going {
                        if should_print {
                            println!("{make_tag}: Leaving directory '{}'", cwd_for_msg.display());
                        }
                        return 2;
                    }
                    // -k mode: report which target couldn't be remade
                    // and keep going with later goals.
                    eprintln!(
                        "{make_tag}: Target '{target_expanded}' not remade because of errors."
                    );
                }
            }
        }

        if should_print {
            println!("{make_tag}: Leaving directory '{}'", cwd_for_msg.display());
        }

        if self.question {
            // `-q`: exit 1 if any target required an update.
            return if *self.question_needs_update.borrow() {
                1
            } else {
                0
            };
        }

        if had_error { 2 } else { 0 }
    }

    fn build_target(&self, target: &str) -> Result<(), String> {
        self.build_target_for(target, None)
    }

    /// Apply --shuffle ordering to a list of prerequisites in place.
    /// `reverse` reverses the order; `none`/`identity`/empty are no-ops.
    /// A numeric seed (or `random`) shuffles deterministically.
    fn shuffle_prereqs(&self, items: &mut [String]) {
        if self.notparallel.get() {
            return;
        }
        let mode = self.shuffle_mode.borrow();
        let Some(m) = mode.as_deref() else { return };
        match m {
            "reverse" => items.reverse(),
            "none" | "identity" | "" => {}
            seed_str => {
                // Simple deterministic Fisher-Yates with a seeded LCG.
                // For "random" use system time as seed.
                let seed: u64 = if seed_str == "random" {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(1)
                } else {
                    seed_str.parse::<u64>().unwrap_or(1)
                };
                let mut state = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
                let mut next = || {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state
                };
                for i in (1..items.len()).rev() {
                    let j = (next() as usize) % (i + 1);
                    items.swap(i, j);
                }
            }
        }
    }

    /// Gather target-specific variables for a target *and* those inherited
    /// from any ancestors currently being built. GNU make propagates
    /// target-specific vars from a parent to all of its prereqs.
    fn collect_target_vars(&self, _target: &str) -> Vec<(String, AssignOp, String, bool, bool)> {
        // The target_scope_stack already contains this target's own
        // vars (pushed by build_target_for) plus all ancestor vars.
        // Just return its contents — the order is parents-first,
        // which execute_recipe applies sequentially.
        self.target_scope_stack.borrow().clone()
    }

    /// Find all pattern-specific variable entries matching a target.
    /// Entries are sorted by stem length descending (longest stem = least
    /// specific first) so that more-specific patterns naturally override
    /// less-specific ones when applied sequentially.
    fn collect_pattern_vars(&self, target: &str) -> Vec<(String, AssignOp, String, bool, bool)> {
        let pv = self.pattern_vars.borrow();
        // Collect all matching entries with their stem lengths and definition index
        let mut matches: Vec<(usize, usize, &str, AssignOp, &str, bool, bool)> = Vec::new();
        for (idx, (pattern, name, op, value, is_override, is_private)) in pv.iter().enumerate() {
            if let Some(stem) = expand::pattern_stem(target, pattern) {
                matches.push((stem.len(), idx, name, *op, value, *is_override, *is_private));
            }
        }
        // Sort by stem length descending (longest/least-specific first),
        // then by definition order for same-length stems.
        // This means more-specific (shorter stem) entries come last and
        // naturally override less-specific ones.
        matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        matches
            .into_iter()
            .map(|(_, _, name, op, value, is_override, is_private)| {
                (
                    name.to_string(),
                    op,
                    value.to_string(),
                    is_override,
                    is_private,
                )
            })
            .collect()
    }

    /// Find pattern-specific export entries matching a target.
    fn collect_pattern_exports(&self, target: &str) -> Vec<String> {
        let pe = self.pattern_exports.borrow();
        pe.iter()
            .filter(|(pattern, _)| expand::pattern_stem(target, pattern).is_some())
            .map(|(_, name)| name.clone())
            .collect()
    }

    fn build_target_for(&self, target: &str, needed_by: Option<&str>) -> Result<(), String> {
        // Normalize path: strip leading ./ for consistency with rule lookup
        let target = target.strip_prefix("./").unwrap_or(target);

        // Already built?
        if self.built_targets.borrow().contains(target) {
            return Ok(());
        }

        // Already failed in -k mode? Don't re-attempt.
        if self.keep_going && self.failed_targets.borrow().contains(target) {
            return Err(String::new());
        }

        // Guard against infinite implicit-rule recursion.
        // e.g. %: %.c matches hello -> hello.c -> hello.c.c -> ...
        const MAX_BUILD_DEPTH: usize = 50;
        let depth = self.build_depth.get();
        if depth >= MAX_BUILD_DEPTH {
            return Err(format!(
                "Implicit rule recursion depth exceeded for target '{}'",
                target
            ));
        }
        self.build_depth.set(depth + 1);
        // Scope guard: decrement build_depth on all return paths.
        struct DepthGuard<'a>(&'a Cell<usize>);
        impl<'a> Drop for DepthGuard<'a> {
            fn drop(&mut self) {
                self.0.set(self.0.get().saturating_sub(1));
            }
        }
        let _depth_guard = DepthGuard(&self.build_depth);

        // Circular dependency detection: if this target is already in
        // the build chain, drop it with a warning and return Ok.
        if self.building_chain.borrow().contains(target) {
            if let Some(parent) = needed_by {
                eprintln!(
                    "make: Circular {} <- {} dependency dropped.",
                    parent, target
                );
            }
            return Ok(());
        }
        self.building_chain.borrow_mut().insert(target.to_string());
        // Scope guard to remove from building_chain on all return paths.
        struct ChainGuard<'a>(&'a RefCell<HashSet<String>>, String);
        impl<'a> Drop for ChainGuard<'a> {
            fn drop(&mut self) {
                self.0.borrow_mut().remove(&self.1);
            }
        }
        let _chain_guard = ChainGuard(&self.building_chain, target.to_string());

        // Track whether the target already exists (locally or via VPATH)
        // before we attempt to build it. Pre-existing files should not
        // be deleted as intermediates (GNU make behavior).
        if !self.vpath_preexisting.borrow().contains(target)
            && (Path::new(target).exists() || self.resolve_vpath(target).is_some())
        {
            self.vpath_preexisting
                .borrow_mut()
                .insert(target.to_string());
        }

        let is_phony = self.phony_targets.borrow().contains(target);

        // Find explicit rules. If none exist for `target` directly,
        // try to resolve via `vpath` directives / `VPATH` so that
        // `fail.te` (with `vpath %.te vpath-d/`) finds rules
        // defined for `vpath-d/fail.te`. The original `target` name
        // remains as `$@`/build key — only the rule lookup is
        // redirected.
        let mut rules = self.rules.borrow().get(target).cloned().unwrap_or_default();
        // When the target itself has no rule but a VPATH-resolved name
        // does, redirect the build to that resolved name. GNU make
        // treats `vpa/foo.x` as the actual target (it's what will be
        // updated on disk) when `foo.x` was requested under `VPATH=vpa`.
        // GPATH redirect: if the target has no local file but VPATH
        // resolves it to a file in a GPATH directory, redirect the
        // build to the VPATH location. The file "stays" in the VPATH
        // dir and is considered up-to-date there.
        let gpath_redirected: Option<String> =
            if !is_phony && !Path::new(target).exists() && rules.is_empty() {
                let gpath_raw = self.lookup_var("GPATH");
                if !gpath_raw.is_empty() {
                    let gpath_dirs: Vec<String> = gpath_raw
                        .split(&[':', ' ', '\t'][..])
                        .filter(|s| !s.is_empty())
                        .map(|s| s.trim_end_matches('/').to_string())
                        .collect();
                    if let Some(resolved) = self.resolve_vpath(target) {
                        let resolved_dir = std::path::Path::new(&resolved)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let resolved_dir_trimmed = resolved_dir.trim_end_matches('/');
                        if gpath_dirs.iter().any(|g| g == resolved_dir_trimmed) {
                            Some(resolved)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
        if gpath_redirected.is_some() {
            // The file exists at the GPATH location — treat it as
            // up-to-date. Return immediately (nothing to build).
            self.built_targets.borrow_mut().insert(target.to_string());
            return Ok(());
        }

        let vpath_redirected: Option<String> = if rules.is_empty() && !is_phony {
            self.resolve_vpath_rule(target)
        } else if !is_phony {
            // "Same file" merging (sv 62650): when the target has rules
            // but vpath also resolves to another target with rules, GNU
            // make warns and redirects to the vpath-resolved target.
            // We check vpath directories for a rule-registered name
            // (not file existence) to handle targets that are only rules.
            self.resolve_vpath_to_rule_target(target).map(|resolved| {
                let source = self
                    .rules
                    .borrow()
                    .get(target)
                    .and_then(|entries| entries.first())
                    .map(|e| {
                        format!(
                            "{}:{}",
                            e.source_name,
                            e.recipe_lines.first().copied().unwrap_or(0)
                        )
                    })
                    .unwrap_or_default();
                // Only warn if the local target actually has a recipe.
                let has_recipe = self
                    .rules
                    .borrow()
                    .get(target)
                    .map(|entries| entries.iter().any(|e| !e.recipe.is_empty()))
                    .unwrap_or(false);
                if has_recipe {
                    eprintln!(
                        "{}: Recipe was specified for file '{}' at {},",
                        source, target, source
                    );
                    eprintln!(
                        "{}: but '{}' is now considered the same file as '{}'.",
                        source, target, resolved
                    );
                    eprintln!(
                        "{}: Recipe for '{}' will be ignored in favor of the one for '{}'.",
                        source, target, resolved
                    );
                }
                resolved
            })
        } else {
            None
        };
        // Save merged-away SE rules so we can fire their side effects
        // after the resolved target's SE rules (GNU ordering).
        let merged_away_se_rules: Vec<RuleEntry> = if vpath_redirected.is_some() {
            rules
                .iter()
                .filter(|r| {
                    r.second_expand
                        && r.raw_prereq_text
                            .as_deref()
                            .is_some_and(|s| s.contains('$'))
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let target = match &vpath_redirected {
            Some(vt) => {
                rules = self
                    .rules
                    .borrow()
                    .get(vt.as_str())
                    .cloned()
                    .unwrap_or_default();
                // Append merged-away SE rules (with recipe cleared) so
                // their side effects fire after the resolved target's
                // SE rules, matching GNU make's ordering.
                for mut merged in merged_away_se_rules {
                    merged.recipe.clear();
                    merged.recipe_lines.clear();
                    rules.push(merged);
                }
                vt.as_str()
            }
            None => target,
        };

        // Push this target's own variable bindings onto the scope stack
        // so prereq builds inherit them (GNU make semantics). Popped
        // before any return path via `pop_scope`.
        // Pattern-specific vars come first (least-specific to most-specific),
        // then target-specific vars override them.
        let own_vars: Vec<(String, AssignOp, String, bool, bool)> = {
            let mut vars = self.collect_pattern_vars(target);
            vars.extend(
                self.target_vars
                    .borrow()
                    .get(target)
                    .cloned()
                    .unwrap_or_default(),
            );
            vars
        };
        // Only push non-private vars to the scope stack (inherited by prereqs)
        let inheritable: Vec<_> = own_vars
            .iter()
            .filter(|(_, _, _, _, is_private)| !is_private)
            .cloned()
            .collect();
        let scope_push_count = inheritable.len();
        self.target_scope_stack.borrow_mut().extend(inheritable);

        // Apply this target's export/unexport entries to the global
        // export/unexport sets so prereq builds inherit them. We save
        // what was changed so pop_scope can undo it.
        // Skip exports/unexports for private variables — private vars
        // don't propagate to prereqs, so their export status shouldn't
        // either.
        let private_var_names: std::collections::HashSet<String> = own_vars
            .iter()
            .filter(|(_, _, _, _, is_private)| *is_private)
            .map(|(name, _, _, _, _)| name.clone())
            .collect();
        let mut own_export_entries: Vec<String> = self.collect_pattern_exports(target);
        own_export_entries.extend(
            self.target_exports
                .borrow()
                .get(target)
                .cloned()
                .unwrap_or_default(),
        );
        own_export_entries.retain(|entry| {
            let varname = entry.strip_prefix('~').unwrap_or(entry);
            !private_var_names.contains(varname)
        });
        let mut scope_added_exports: Vec<String> = Vec::new();
        let mut scope_added_unexports: Vec<String> = Vec::new();
        let mut scope_removed_from_unexports: Vec<String> = Vec::new();
        let mut scope_removed_from_exports: Vec<String> = Vec::new();
        {
            let mut exports = self.exports.borrow_mut();
            let mut unexports = self.unexports.borrow_mut();
            for entry in &own_export_entries {
                if let Some(uname) = entry.strip_prefix('~') {
                    // unexport: remove from exports, add to unexports
                    if exports.remove(uname) {
                        scope_removed_from_exports.push(uname.to_string());
                    }
                    if unexports.insert(uname.to_string()) {
                        scope_added_unexports.push(uname.to_string());
                    }
                } else {
                    // export: remove from unexports, add to exports
                    if unexports.remove(entry) {
                        scope_removed_from_unexports.push(entry.clone());
                    }
                    if exports.insert(entry.clone()) {
                        scope_added_exports.push(entry.clone());
                    }
                }
            }
        }

        let pop_scope = |s: &Self| {
            let mut stack = s.target_scope_stack.borrow_mut();
            let new_len = stack.len().saturating_sub(scope_push_count);
            stack.truncate(new_len);
            drop(stack);
            // Undo export/unexport changes
            {
                let mut exports = s.exports.borrow_mut();
                let mut unexports = s.unexports.borrow_mut();
                for n in &scope_added_exports {
                    exports.remove(n);
                }
                for n in &scope_added_unexports {
                    unexports.remove(n);
                }
                for n in &scope_removed_from_unexports {
                    unexports.insert(n.clone());
                }
                for n in &scope_removed_from_exports {
                    exports.insert(n.clone());
                }
            }
        };

        // Double-colon rules are independent: each rule is evaluated
        // separately, with its own prereqs and its own recipe. Fall
        // through to the normal path only if all rules are single-colon.
        let any_double = rules.iter().any(|r| r.is_double_colon);
        if any_double {
            self.built_targets.borrow_mut().insert(target.to_string());
            // Capture target mtime once BEFORE any rules run. Each
            // double-colon rule is evaluated independently against the
            // original state; a recipe in an earlier rule that creates or
            // touches the target must not make later rules think the
            // target is already up-to-date.
            let initial_target_mtime = if is_phony {
                None
            } else {
                std::fs::metadata(target)
                    .ok()
                    .and_then(|m| m.modified().ok())
            };
            for (idx, rule) in rules.iter().enumerate() {
                // For grouped double-colon rules, skip if group recipe already ran.
                if !rule.group.is_empty() {
                    let key = rule.group.join("\\0");
                    if self.group_recipe_done.borrow().contains(&key) {
                        continue;
                    }
                }
                // For .SECONDEXPANSION rules, expand the raw text now.
                let (prereqs, order_only): (Vec<String>, Vec<String>) = if rule.second_expand
                    && rule.raw_prereq_text.is_some()
                {
                    let mut auto_vars: HashMap<&str, String> = HashMap::new();
                    auto_vars.insert("@", target.to_string());
                    auto_vars.insert("*", rule.stem.clone().unwrap_or_default());
                    auto_vars.insert("<", String::new());
                    auto_vars.insert("^", String::new());
                    auto_vars.insert("+", String::new());
                    auto_vars.insert("|", String::new());
                    auto_vars.insert("?", String::new());
                    auto_vars.insert("%", String::new());
                    add_df_variants(&mut auto_vars);
                    let raw = rule.raw_prereq_text.as_deref().unwrap_or("");
                    let prev_in_recipe = *self.in_recipe.borrow();
                    *self.in_recipe.borrow_mut() = true;
                    let exp = self.with_target_vars_applied(target, || {
                        expand::expand_with_auto(raw, self, &auto_vars)
                    });
                    *self.in_recipe.borrow_mut() = prev_in_recipe;
                    let (n, o) = if let Some(idx) = exp.find('|') {
                        (exp[..idx].to_string(), exp[idx + 1..].to_string())
                    } else {
                        (exp, String::new())
                    };
                    let mut pr: Vec<String> = n
                        .split_whitespace()
                        .map(|s| self.resolve_library_prereq(&unescape_name(s)))
                        .collect();
                    let mut oo: Vec<String> = o.split_whitespace().map(|s| s.to_string()).collect();
                    if let Some(orig_oo) = &rule.raw_order_only_text {
                        let oo_exp = self.with_target_vars_applied(target, || {
                            expand::expand_with_auto(orig_oo, self, &auto_vars)
                        });
                        oo.extend(oo_exp.split_whitespace().map(|s| s.to_string()));
                    }
                    // Apply auto-vars-driven update to first prereq for $<
                    let _ = (&mut pr, &mut oo);
                    (pr, oo)
                } else {
                    let pr: Vec<String> = rule
                        .prerequisites
                        .iter()
                        .map(|p| self.resolve_library_prereq(p))
                        .collect();
                    (pr, rule.order_only.clone())
                };
                for prereq in &prereqs {
                    if prereq == target || self.building_chain.borrow().contains(prereq.as_str()) {
                        eprintln!(
                            "make: Circular {} <- {} dependency dropped.",
                            target, prereq
                        );
                        continue;
                    }
                    if let Err(e) = self.build_target_for(prereq, Some(target)) {
                        pop_scope(self);
                        return Err(e);
                    }
                }
                for prereq in &order_only {
                    if let Err(e) = self.build_target_for(prereq, Some(target)) {
                        pop_scope(self);
                        return Err(e);
                    }
                }
                if rule.recipe.is_empty() {
                    continue;
                }
                // Use the initial target mtime captured before the loop.
                let mut needs =
                    self.always_make.get() || is_phony || initial_target_mtime.is_none();
                if !needs {
                    for p in &prereqs {
                        let pm = self.metadata_or_vpath(p).and_then(|m| m.modified().ok());
                        if pm.is_none() {
                            needs = true;
                            break;
                        }
                        if let (Some(t), Some(pt)) = (initial_target_mtime, pm)
                            && pt > t
                        {
                            needs = true;
                            break;
                        }
                    }
                }
                if !needs {
                    continue;
                }
                let _ = idx;
                if self.touch {
                    if !is_phony {
                        if !self.silent {
                            println!("touch {target}");
                        }
                        *self.recipe_executed.borrow_mut() = true;
                        self.rebuilt_targets.borrow_mut().insert(target.to_string());
                        if !self.dry_run {
                            std::fs::OpenOptions::new()
                                .create(true)
                                .truncate(false)
                                .write(true)
                                .open(target)
                                .ok();
                        }
                    }
                } else if self.question {
                    *self.question_needs_update.borrow_mut() = true;
                } else {
                    // --trace: print trace line before executing recipe
                    if self.trace {
                        let trace_line_no = rule.recipe_lines.first().copied().unwrap_or(0);
                        let reason = if initial_target_mtime.is_none() && !is_phony {
                            "target does not exist".to_string()
                        } else if self.always_make.get() {
                            "always-make flag is set".to_string()
                        } else if is_phony {
                            "target is .PHONY".to_string()
                        } else {
                            let newer: Vec<&str> = prereqs
                                .iter()
                                .filter(|p| {
                                    if let (Some(tm), Ok(pm)) = (
                                        initial_target_mtime,
                                        std::fs::metadata(p.as_str()).and_then(|m| m.modified()),
                                    ) {
                                        pm > tm
                                    } else {
                                        false
                                    }
                                })
                                .map(|s| s.as_str())
                                .collect();
                            if newer.is_empty() {
                                "target does not exist".to_string()
                            } else {
                                newer.join(" ")
                            }
                        };
                        eprintln!(
                            "{}:{}: update target '{}' due to: {}",
                            rule.source_name, trace_line_no, target, reason
                        );
                    }
                    match self.execute_recipe(
                        target,
                        &rule.recipe,
                        &rule.recipe_lines,
                        &rule.source_name,
                        &prereqs,
                        &order_only,
                        &[],
                        rule.stem.as_deref().unwrap_or(""),
                    ) {
                        Err(e) => {
                            pop_scope(self);
                            return Err(e);
                        }
                        Ok(ran) => {
                            if ran {
                                self.rebuilt_targets.borrow_mut().insert(target.to_string());
                            }
                        }
                    }
                }
                // Mark grouped rule as done so sibling targets skip it.
                if !rule.group.is_empty() {
                    let key = rule.group.join("\\0");
                    self.group_recipe_done.borrow_mut().insert(key);
                    let mut built = self.built_targets.borrow_mut();
                    for sibling in &rule.group {
                        built.insert(sibling.clone());
                    }
                }
            }
            *self.target_had_recipe.borrow_mut() = rules.iter().any(|r| !r.recipe.is_empty());
            pop_scope(self);
            return Ok(());
        }

        // Find matching pattern rule if no explicit recipe
        let has_recipe = rules.iter().any(|r| !r.recipe.is_empty());
        let pattern_match =
            if !has_recipe && !is_phony && !self.skip_implicit.borrow().contains(target) {
                self.find_pattern_rule(target)
            } else {
                None
            };

        // For non-double-colon grouped targets, if the group's recipe
        // was already executed by a sibling, fire any second-expansion
        // side effects for this target's auto-vars then early-exit.
        // GNU make second-expands prereqs per-target even when the
        // recipe runs only once for the group.
        for rule in &rules {
            if !rule.group.is_empty() {
                let key = rule.group.join("\0");
                if self.group_recipe_done.borrow().contains(&key) {
                    let needs_se = rule.second_expand
                        && (rule
                            .raw_prereq_text
                            .as_deref()
                            .is_some_and(|s| s.contains('$'))
                            || rule
                                .raw_order_only_text
                                .as_deref()
                                .is_some_and(|s| s.contains('$')));
                    if needs_se {
                        let mut auto_vars: HashMap<&str, String> = HashMap::new();
                        auto_vars.insert("@", target.to_string());
                        auto_vars.insert("*", rule.stem.clone().unwrap_or_default());
                        auto_vars.insert("<", String::new());
                        auto_vars.insert("^", String::new());
                        auto_vars.insert("+", String::new());
                        auto_vars.insert("|", String::new());
                        auto_vars.insert("?", String::new());
                        auto_vars.insert("%", String::new());
                        add_df_variants(&mut auto_vars);
                        let raw = rule.raw_prereq_text.as_deref().unwrap_or("");
                        let _ = self.with_target_vars_applied(target, || {
                            expand::expand_with_auto(raw, self, &auto_vars)
                        });
                    }
                    *self.target_had_recipe.borrow_mut() = false;
                    self.built_targets.borrow_mut().insert(target.to_string());
                    pop_scope(self);
                    return Ok(());
                }
            }
        }

        // Collect all prerequisites
        // Resolve -l library prereqs now (build time) rather than parse
        // time, so that rules registered later in the makefile (e.g.
        // `libcat.a:`) are visible to the library search.
        let mut all_prereqs: Vec<String> = Vec::new();
        let mut all_order_only: Vec<String> = Vec::new();
        // SE-derived prereqs are collected separately and merged after non-SE
        // prereqs and order-only have been built.
        let mut se_prereqs: Vec<String> = Vec::new();
        let mut se_order_only: Vec<String> = Vec::new();
        let mut recipe: Vec<String> = Vec::new();
        let mut recipe_lines: Vec<usize> = Vec::new();
        let mut recipe_source: String = String::new();
        let mut stem = String::new();
        // Track pattern-implied prerequisites for $< resolution
        let mut implied_prereqs: Vec<String> = Vec::new();
        // Track each rule's contributed prereq slice so we can later
        // reorder so the recipe-bearing rule's prereqs come first
        // (GNU make semantics; affects $^/$+/$< ordering for the recipe).
        let mut rule_prereq_slices: Vec<(usize, usize, usize, usize, bool)> = Vec::new(); // (norm_start, norm_end, oo_start, oo_end, has_recipe)

        for rule in &rules {
            let _slice_start = all_prereqs.len();
            let _oo_slice_start = all_order_only.len();
            // Only treat as SE if raw_prereq_text contains '$' or
            // raw_order_only_text does — otherwise normal handling
            // suffices and matches GNU's per-rule build ordering.
            let needs_se = rule.second_expand
                && (rule
                    .raw_prereq_text
                    .as_deref()
                    .is_some_and(|s| s.contains('$'))
                    || rule
                        .raw_order_only_text
                        .as_deref()
                        .is_some_and(|s| s.contains('$')));
            // Resolve -l prereqs for non-SE rules at build time.
            let prereqs_resolved: Vec<String> = if !needs_se {
                rule.prerequisites
                    .iter()
                    .map(|p| self.resolve_library_prereq(p))
                    .collect()
            } else {
                rule.prerequisites.clone()
            };
            if needs_se {
                // Build auto_vars from prereqs collected so far (from non-SE rules
                // and prior SE rules — GNU make behavior).
                let target_str = target.to_string();
                let stem_str = rule.stem.clone().unwrap_or_default();
                let plus_str = all_prereqs.join(" ");
                // $^ deduplicated, $+ keeps duplicates.
                let mut seen_set: HashSet<String> = HashSet::new();
                let caret: Vec<String> = all_prereqs
                    .iter()
                    .filter(|p| seen_set.insert((*p).clone()))
                    .cloned()
                    .collect();
                let caret_str = caret.join(" ");
                // $| dedups (matches GNU make execute_recipe behavior).
                let mut pipe_seen: HashSet<String> = HashSet::new();
                let pipe_str: String = all_order_only
                    .iter()
                    .filter(|p| pipe_seen.insert((*p).clone()))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let lt_str = all_prereqs.first().cloned().unwrap_or_default();
                let mut auto_vars: HashMap<&str, String> = HashMap::new();
                auto_vars.insert("@", target_str.clone());
                auto_vars.insert("*", stem_str.clone());
                auto_vars.insert("<", lt_str);
                auto_vars.insert("^", caret_str);
                auto_vars.insert("+", plus_str);
                auto_vars.insert("|", pipe_str);
                auto_vars.insert("?", String::new());
                auto_vars.insert("%", String::new());
                add_df_variants(&mut auto_vars);
                let raw = rule.raw_prereq_text.as_deref().unwrap_or("");
                // Set in_recipe so $(eval) defining new prereqs errors out.
                let prev_in_recipe = *self.in_recipe.borrow();
                *self.in_recipe.borrow_mut() = true;
                let expanded = self.with_target_vars_applied(target, || {
                    expand::expand_with_auto(raw, self, &auto_vars)
                });
                *self.in_recipe.borrow_mut() = prev_in_recipe;
                // Re-parse for embedded `|` (order-only).
                let (norm_part, oo_part) = if let Some(idx) = find_orderonly_pipe(&expanded) {
                    (expanded[..idx].to_string(), expanded[idx + 1..].to_string())
                } else {
                    (expanded, String::new())
                };
                for tok in norm_part.split_whitespace() {
                    let t = unescape_name(tok);
                    let expanded_list: Vec<String> = if t.contains(['*', '?', '[']) {
                        if let Ok(paths) = glob::glob(&t) {
                            let m: Vec<String> = paths
                                .flatten()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect();
                            if m.is_empty() { vec![t.clone()] } else { m }
                        } else {
                            vec![t.clone()]
                        }
                    } else {
                        vec![t.clone()]
                    };
                    for ep in expanded_list {
                        let resolved = self.resolve_library_prereq(&ep);
                        se_prereqs.push(resolved.clone());
                        all_prereqs.push(resolved);
                    }
                }
                // Order-only from raw_order_only_text + post-pipe.
                let mut oo_combined = oo_part;
                if let Some(orig_oo) = &rule.raw_order_only_text
                    && !orig_oo.trim().is_empty()
                {
                    let oo_exp = expand::expand_with_auto(orig_oo, self, &auto_vars);
                    if !oo_combined.is_empty() {
                        oo_combined.push(' ');
                    }
                    oo_combined.push_str(&oo_exp);
                }
                for tok in oo_combined.split_whitespace() {
                    se_order_only.push(tok.to_string());
                    all_order_only.push(tok.to_string());
                }
            } else {
                // For SE rules that contain no `$`, raw_*_text holds
                // the (already %-substituted) prereqs but rule.prerequisites
                // was deferred to Vec::new(). Use raw text in that case.
                if rule.second_expand && rule.raw_prereq_text.is_some() {
                    if let Some(rp) = &rule.raw_prereq_text {
                        for tok in rp.split_whitespace() {
                            all_prereqs.push(self.resolve_library_prereq(&unescape_name(tok)));
                        }
                    }
                    if let Some(ro) = &rule.raw_order_only_text {
                        for tok in ro.split_whitespace() {
                            all_order_only.push(tok.to_string());
                        }
                    }
                } else {
                    all_prereqs.extend(prereqs_resolved.iter().cloned());
                    all_order_only.extend(rule.order_only.iter().cloned());
                }
            }
            if recipe.is_empty() && !rule.recipe.is_empty() {
                recipe_lines = rule.recipe_lines.clone();
                recipe_source = rule.source_name.clone();
                recipe = rule.recipe.clone();
            }
            // Static pattern rule stem: last one wins for $*.
            if let Some(s) = &rule.stem {
                stem = s.clone();
            }
            rule_prereq_slices.push((
                _slice_start,
                all_prereqs.len(),
                _oo_slice_start,
                all_order_only.len(),
                !rule.recipe.is_empty(),
            ));
        }
        // Promote order-only entries that also appear as normal prereqs
        // — GNU make semantics: a prereq declared in both positions
        // counts as normal and is removed from `$|`.
        let normal_set: HashSet<String> = all_prereqs.iter().cloned().collect();
        all_order_only.retain(|o| !normal_set.contains(o));

        // Track prereqs that are derived from pattern substitution (`%`);
        // these may be intermediate files that do not trigger a rebuild
        // if the final target already exists and they are not
        // explicitly mentioned elsewhere.
        let mut pattern_derived_prereqs: HashSet<String> = HashSet::new();
        let _pat_slice_start = all_prereqs.len();
        let _pat_oo_slice_start = all_order_only.len();
        if let Some((pat_rule, pat_stem)) = &pattern_match {
            stem = pat_stem.clone();
            // SE for pattern rule: substitute % then expand_with_auto.
            // Only treat as SE if raw text actually contains `$` —
            // otherwise the non-SE path suffices and pushes to all_prereqs.
            let pat_needs_se = pat_rule.second_expand
                && (pat_rule
                    .raw_prereq_text
                    .as_deref()
                    .is_some_and(|s| s.contains('$'))
                    || pat_rule
                        .raw_order_only_text
                        .as_deref()
                        .is_some_and(|s| s.contains('$')));
            if pat_needs_se {
                let target_str = target.to_string();
                let stem_str = stem.clone();
                let plus_str = all_prereqs.join(" ");
                let mut seen_set: HashSet<String> = HashSet::new();
                let caret: Vec<String> = all_prereqs
                    .iter()
                    .filter(|p| seen_set.insert((*p).clone()))
                    .cloned()
                    .collect();
                let caret_str = caret.join(" ");
                // $| dedups (matches GNU make execute_recipe behavior).
                let mut pipe_seen: HashSet<String> = HashSet::new();
                let pipe_str: String = all_order_only
                    .iter()
                    .filter(|p| pipe_seen.insert((*p).clone()))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" ");
                let lt_str = all_prereqs.first().cloned().unwrap_or_default();
                let mut auto_vars: HashMap<&str, String> = HashMap::new();
                auto_vars.insert("@", target_str);
                auto_vars.insert("*", stem_str.clone());
                auto_vars.insert("<", lt_str);
                auto_vars.insert("^", caret_str);
                auto_vars.insert("+", plus_str);
                auto_vars.insert("|", pipe_str);
                auto_vars.insert("?", String::new());
                auto_vars.insert("%", String::new());
                add_df_variants(&mut auto_vars);
                let raw = pat_rule.raw_prereq_text.clone().unwrap_or_default();
                // Track which raw tokens contained `%` (pattern-derived
                // intermediates) before stem substitution. Tokens without
                // `%` come from variables/SE expansions; they are NOT
                // intermediate (they must be mentioned to be built).
                // Use nesting-aware split so `$$(...)` with internal
                // whitespace stays as a single token.
                let raw_tokens: Vec<(String, bool)> =
                    crate::parser::split_whitespace_respecting_refs(&raw)
                        .into_iter()
                        .map(|t| {
                            let p = t.contains('%');
                            (t, p)
                        })
                        .collect();
                let prev_in_recipe = *self.in_recipe.borrow();
                *self.in_recipe.borrow_mut() = true;
                // For directory-transfer: split stem into dir + base,
                // substitute only base for %, expand, then prepend dir
                // to each resulting token. This matches GNU make's SE
                // pattern-rule semantics where directory transfer happens
                // AFTER second expansion, not before.
                let (stem_dir, stem_base) = if let Some(slash) = stem.rfind('/') {
                    (Some(&stem[..=slash]), &stem[slash + 1..])
                } else {
                    (None, stem.as_str())
                };
                let mut expanded_per_token: Vec<(String, bool)> = Vec::new();
                for (rt, is_pat) in &raw_tokens {
                    // Substitute % with base stem only (no dir prepend yet).
                    let with_stem =
                        replace_first_percent_per_token(rt, &stem_base.replace('$', "$$"), false);
                    let exp = self.with_target_vars_applied(target, || {
                        expand::expand_with_auto(&with_stem, self, &auto_vars)
                    });
                    // Now prepend directory to each expanded token if the
                    // original raw token contained % (pattern-derived) and
                    // the stem has a directory component.
                    let exp = if let (true, Some(dir)) = (*is_pat, stem_dir) {
                        exp.split_whitespace()
                            .map(|tok| format!("{dir}{tok}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        exp
                    };
                    expanded_per_token.push((exp, *is_pat));
                }
                *self.in_recipe.borrow_mut() = prev_in_recipe;
                // Re-join then re-parse for embedded `|` (order-only).
                let joined: String = expanded_per_token
                    .iter()
                    .map(|(e, _)| e.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let oo_part = if let Some(idx) = find_orderonly_pipe(&joined) {
                    joined[idx + 1..].to_string()
                } else {
                    String::new()
                };
                for (exp, is_pat) in &expanded_per_token {
                    let exp_norm = if let Some(idx) = find_orderonly_pipe(exp) {
                        &exp[..idx]
                    } else {
                        exp.as_str()
                    };
                    for tok in exp_norm.split_whitespace() {
                        let ep0 = unescape_name(tok);
                        let expanded_list: Vec<String> = if ep0.contains(['*', '?', '[']) {
                            if let Ok(paths) = glob::glob(&ep0) {
                                let m: Vec<String> = paths
                                    .flatten()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .collect();
                                if m.is_empty() { vec![ep0.clone()] } else { m }
                            } else {
                                vec![ep0.clone()]
                            }
                        } else {
                            vec![ep0.clone()]
                        };
                        for ep in expanded_list {
                            if *is_pat {
                                pattern_derived_prereqs.insert(ep.clone());
                                implied_prereqs.push(ep.clone());
                            }
                            se_prereqs.push(ep.clone());
                            all_prereqs.push(ep);
                        }
                    }
                }
                let mut oo_combined = oo_part;
                if let Some(orig_oo) = &pat_rule.raw_order_only_text
                    && !orig_oo.trim().is_empty()
                {
                    let with_stem_oo = replace_first_percent_per_token(
                        orig_oo,
                        &stem_base.replace('$', "$$"),
                        false,
                    );
                    let oo_exp = expand::expand_with_auto(&with_stem_oo, self, &auto_vars);
                    if !oo_combined.is_empty() {
                        oo_combined.push(' ');
                    }
                    oo_combined.push_str(&oo_exp);
                }
                for tok in oo_combined.split_whitespace() {
                    se_order_only.push(tok.to_string());
                    all_order_only.push(tok.to_string());
                }
                if recipe.is_empty() {
                    recipe = pat_rule.recipe.clone();
                    recipe_lines = pat_rule.recipe_lines.clone();
                    recipe_source = pat_rule.source_name.clone();
                }
            } else {
                for pp in &pat_rule.prereq_patterns {
                    let prereq = pattern_subst_with_dir(pp, &stem);
                    // Expand filesystem globs in pattern rule prereqs.
                    let expanded_list: Vec<String> = if prereq.contains(['*', '?', '[']) {
                        if let Ok(paths) = glob::glob(&prereq) {
                            let m: Vec<String> = paths
                                .flatten()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect();
                            if m.is_empty() {
                                vec![prereq.clone()]
                            } else {
                                m
                            }
                        } else {
                            vec![prereq.clone()]
                        }
                    } else {
                        vec![prereq.clone()]
                    };
                    for ep in expanded_list {
                        if pp.contains('%') {
                            pattern_derived_prereqs.insert(ep.clone());
                        }
                        implied_prereqs.push(ep.clone());
                        all_prereqs.push(ep);
                    }
                }
                for op in &pat_rule.order_only_patterns {
                    let oo = expand::expand(&pattern_subst_with_dir(op, &stem), self);
                    for tok in oo.split_whitespace() {
                        all_order_only.push(tok.to_string());
                    }
                }
                if recipe.is_empty() {
                    recipe = pat_rule.recipe.clone();
                    recipe_lines = pat_rule.recipe_lines.clone();
                    recipe_source = pat_rule.source_name.clone();
                }
            } // end else (non-SE)
        }
        // Multi-target pattern rules: merge explicit prereqs from
        // sibling targets. When `%.t1 %.t2:` matches `x.t1` and
        // `x.t2` has explicit prereqs (e.g. `x.t2: dep`), those
        // prereqs must be built before the grouped recipe runs.
        if let Some((pat_rule, pat_stem)) = &pattern_match
            && !pat_rule.sibling_patterns.is_empty()
        {
            for sibling_pat in &pat_rule.sibling_patterns {
                let sibling = sibling_pat.replacen('%', pat_stem, 1);
                if sibling == target {
                    continue;
                }
                // Look up explicit rules for the sibling target.
                let sibling_rules = self
                    .rules
                    .borrow()
                    .get(&sibling)
                    .cloned()
                    .unwrap_or_default();
                for sr in &sibling_rules {
                    for p in &sr.prerequisites {
                        let resolved = self.resolve_library_prereq(p);
                        if !all_prereqs.contains(&resolved) {
                            all_prereqs.push(resolved);
                        }
                    }
                    for o in &sr.order_only {
                        if !all_order_only.contains(o) {
                            all_order_only.push(o.clone());
                        }
                    }
                }
            }
        }

        // For terminal pattern rules, prevent further implicit rule
        // chaining for the derived prereqs.
        if let Some((pat_rule, _)) = &pattern_match
            && pat_rule.is_terminal
        {
            let mut si = self.skip_implicit.borrow_mut();
            for prereq in &implied_prereqs {
                si.insert(prereq.clone());
            }
        }
        let clean_skip = |s: &Self| {
            if let Some((pat_rule, _)) = &pattern_match
                && pat_rule.is_terminal
            {
                let mut si = s.skip_implicit.borrow_mut();
                for prereq in &implied_prereqs {
                    si.remove(prereq);
                }
            }
        };
        // Track the pattern-match's prereq contribution for the
        // recipe-rule-first reorder.
        if pattern_match.is_some() {
            rule_prereq_slices.push((
                _pat_slice_start,
                all_prereqs.len(),
                _pat_oo_slice_start,
                all_order_only.len(),
                true,
            ));
        }
        // Compute per-rule build groups (each rule's normal prereqs followed
        // by its order-only prereqs). This MUST happen before the head/tail
        // reorder below mutates `all_prereqs` and destroys slice indices.
        // GNU make 4.4.1 builds prereqs grouped per-rule with the
        // recipe-bearing rule's prereqs first.
        let mut rule_build_groups: Vec<(Vec<String>, Vec<String>, bool)> = Vec::new();
        for (ns, ne, oos, ooe, has_recipe) in &rule_prereq_slices {
            let mut norm: Vec<String> = Vec::new();
            if *ne <= all_prereqs.len() && *ns <= *ne {
                norm.extend(all_prereqs[*ns..*ne].iter().cloned());
            }
            let mut oo: Vec<String> = Vec::new();
            if *ooe <= all_order_only.len() && *oos <= *ooe {
                oo.extend(all_order_only[*oos..*ooe].iter().cloned());
            }
            rule_build_groups.push((norm, oo, *has_recipe));
        }
        // Reorder so groups whose rule carries the recipe come first
        // (preserving relative order within each partition). This affects
        // both build iteration and the $^ display order.
        if rule_build_groups.iter().any(|(_, _, h)| *h)
            && rule_build_groups.iter().filter(|(_, _, h)| *h).count() < rule_build_groups.len()
        {
            let (head, tail): (Vec<_>, Vec<_>) =
                rule_build_groups.iter().cloned().partition(|(_, _, h)| *h);
            rule_build_groups = head.into_iter().chain(tail).collect();
        }
        // Reorder again with the pattern-match slice included.
        if rule_prereq_slices.iter().any(|(_, _, _, _, has)| *has)
            && rule_prereq_slices
                .iter()
                .filter(|(_, _, _, _, has)| *has)
                .count()
                < rule_prereq_slices.len()
        {
            let mut head: Vec<String> = Vec::new();
            let mut tail: Vec<String> = Vec::new();
            let mut covered: Vec<bool> = vec![false; all_prereqs.len()];
            for (start, end, _oos, _ooe, has_recipe) in &rule_prereq_slices {
                if *end > all_prereqs.len() {
                    continue;
                }
                if *start >= all_prereqs.len() {
                    continue;
                }
                for idx in *start..*end {
                    if covered[idx] {
                        continue;
                    }
                    covered[idx] = true;
                    if *has_recipe {
                        head.push(all_prereqs[idx].clone());
                    } else {
                        tail.push(all_prereqs[idx].clone());
                    }
                }
            }
            for (idx, c) in covered.iter().enumerate() {
                if !c {
                    tail.push(all_prereqs[idx].clone());
                }
            }
            head.extend(tail);
            all_prereqs = head;
        }
        // Re-apply the normal-vs-order-only promotion after pattern
        // match injected new entries.
        let normal_set2: HashSet<String> = all_prereqs.iter().cloned().collect();
        all_order_only.retain(|o| !normal_set2.contains(o));

        // Vpath "same file" resolution for prereqs: if a prereq name
        // resolves via vpath to another target that has explicit rules,
        // update the prereq name to the resolved path. This makes `$^`
        // show vpath-resolved paths (e.g. `src/hello.c` instead of
        // `hello.c` when `vpath hello.c src` is active).
        for prereq in all_prereqs.iter_mut() {
            if let Some(resolved) = self.resolve_vpath_to_rule_target(prereq) {
                *prereq = resolved;
            }
        }

        // GPATH support: when a prereq doesn't exist locally but is
        // found via VPATH in a directory listed in GPATH, replace the
        // prereq name with its VPATH-resolved path. This makes the
        // file "stay" in the VPATH directory (treated as up-to-date
        // there) instead of triggering a rebuild in the current dir.
        let gpath_raw = self.lookup_var("GPATH");
        if !gpath_raw.is_empty() {
            let gpath_dirs: Vec<String> = gpath_raw
                .split(&[':', ' ', '\t'][..])
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_end_matches('/').to_string())
                .collect();
            if !gpath_dirs.is_empty() {
                for prereq in all_prereqs.iter_mut() {
                    if Path::new(prereq.as_str()).exists() {
                        continue;
                    }
                    if let Some(resolved) = self.resolve_vpath(prereq) {
                        // Check if the resolved path's directory is in GPATH.
                        let resolved_dir = Path::new(&resolved)
                            .parent()
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let resolved_dir_trimmed = resolved_dir.trim_end_matches('/');
                        if gpath_dirs.iter().any(|g| g == resolved_dir_trimmed) {
                            *prereq = resolved;
                        }
                    }
                }
            }
        }

        // Fall back to `.DEFAULT`'s recipe if we still have nothing (and no
        // explicit rules exist for this target).
        if recipe.is_empty()
            && rules.is_empty()
            && pattern_match.is_none()
            && let Some((r, l, s)) = self.default_rule.borrow().as_ref()
        {
            recipe = r.clone();
            recipe_lines = l.clone();
            recipe_source = s.clone();
        }

        // If the explicit rule provides the recipe but the target has a
        // recognized suffix, compute the stem as the target minus the suffix.
        // This matches GNU make behavior for suffix rules on explicit targets.
        if stem.is_empty() && pattern_match.is_none() {
            let suffixes = self.suffixes.borrow();
            for suf in suffixes.iter() {
                if target.ends_with(suf.as_str()) {
                    stem = target[..target.len() - suf.len()].to_string();
                    break;
                }
            }
        }

        // .EXTRA_PREREQS: extra prereqs added to every target, built but
        // not visible in `$<`/`$^`/etc. Skip if the target being built
        // is itself one of the extra prereqs (avoid cycle).
        //
        // A target-specific `.EXTRA_PREREQS` overrides the global one
        // for this target. Globs (`*`, `?`) in entries are expanded
        // via filesystem matching — unmatched entries are retained
        // literally so an explicit rule can still build them.
        let extra_raw = {
            let tv = self.target_vars.borrow();
            tv.get(target)
                .and_then(|entries| {
                    entries
                        .iter()
                        .rev()
                        .find(|(n, _, _, _, _)| n == ".EXTRA_PREREQS")
                })
                .map(|(_, _, v, _, _)| expand::expand(v, self))
                .unwrap_or_else(|| self.lookup_var(".EXTRA_PREREQS"))
        };
        let extra_prereqs: Vec<String> = extra_raw
            .split_whitespace()
            .flat_map(|s| {
                if s.contains(['*', '?', '['])
                    && let Ok(paths) = glob::glob(s)
                {
                    let matches: Vec<String> = paths
                        .flatten()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect();
                    if !matches.is_empty() {
                        return matches;
                    }
                }
                vec![s.to_string()]
            })
            .collect();
        let in_extras = extra_prereqs.iter().any(|p| p == target);

        // Build normal prerequisites first. If a prereq has no file on
        // disk after being built (a phony / FORCE-style rule) we treat
        // it as "newer than any target" so its dependents always
        // rebuild, matching GNU make semantics.
        let mut newest_prereq: Option<std::time::SystemTime> = None;
        let mut has_phony_prereq = false;
        let mut has_rebuilt_prereq = false;
        let has_assume_new_prereq = {
            let assume_new = self.assume_new.borrow();
            if assume_new.is_empty() {
                false
            } else {
                all_prereqs.iter().any(|p| {
                    let np = p.strip_prefix("./").unwrap_or(p);
                    // Check the prereq name directly
                    if assume_new.iter().any(|a| {
                        let na = a.strip_prefix("./").unwrap_or(a);
                        np == na
                    }) {
                        return true;
                    }
                    // Also check the VPATH-resolved path
                    if let Some(resolved) = self.resolve_vpath(p) {
                        let nr = resolved.strip_prefix("./").unwrap_or(&resolved);
                        assume_new.iter().any(|a| {
                            let na = a.strip_prefix("./").unwrap_or(a);
                            nr == na
                        })
                    } else {
                        false
                    }
                })
            }
        };
        // Pre-compute whether we should skip building missing intermediate
        // prereqs. GNU make treats missing intermediates as "infinitely
        // old" — they don't trigger rebuilds unless the target actually
        // needs updating. Check ALL existing prereqs (recursively)
        // against the target's mtime, not just the intermediate's
        // immediate sources.
        // Capture target mtime before recipe execution. Used later to
        // determine if the recipe actually created/updated the target
        // (for the peer-target warning).
        let pre_recipe_target_mtime: Option<std::time::SystemTime> = if !is_phony {
            std::fs::metadata(target)
                .ok()
                .and_then(|m| m.modified().ok())
        } else {
            None
        };

        let skip_missing_intermediates = if !is_phony && Path::new(target).exists() {
            let target_m = std::fs::metadata(target)
                .ok()
                .and_then(|m| m.modified().ok());
            let mut any_source_newer = false;
            if let Some(tm) = target_m {
                // Check all existing prereqs — if any existing file is
                // newer than the target, we must rebuild (and thus must
                // build any missing intermediates too).
                for prereq in &all_prereqs {
                    let prereq_path = self
                        .resolve_vpath(prereq)
                        .unwrap_or_else(|| prereq.to_string());
                    if let Ok(pm) =
                        std::fs::metadata(prereq_path.as_str()).and_then(|m| m.modified())
                        && pm > tm
                    {
                        any_source_newer = true;
                        break;
                    }
                }
                // If any non-intermediate prereq doesn't exist as a
                // file, it will be built, which means the target will
                // need rebuilding — so don't skip intermediates.
                // This handles cases like `%.tsk: %.z test.z` where
                // `test.z` is explicitly mentioned (non-intermediate),
                // doesn't exist, and will be built — forcing `hello.z`
                // (intermediate) to also be built.
                if !any_source_newer {
                    for prereq in &all_prereqs {
                        let prereq_resolved = self
                            .resolve_vpath(prereq)
                            .unwrap_or_else(|| prereq.to_string());
                        if !Path::new(prereq.as_str()).exists()
                            && !Path::new(prereq_resolved.as_str()).exists()
                            && !self.check_intermediate(
                                prereq,
                                pattern_derived_prereqs.contains(prereq),
                            )
                            && !self.phony_targets.borrow().contains(prereq)
                        {
                            any_source_newer = true;
                            break;
                        }
                    }
                }
                // Also check if any prereq would be rebuilt by its own
                // chain — recursively check intermediate prereq sources.
                if !any_source_newer {
                    fn check_chain_sources(
                        engine: &Engine,
                        file: &str,
                        target_m: std::time::SystemTime,
                        depth: usize,
                    ) -> bool {
                        if depth > 6 {
                            return false;
                        }
                        let resolved_file = engine
                            .resolve_vpath(file)
                            .unwrap_or_else(|| file.to_string());
                        if let Some((rule, stem)) = engine
                            .find_pattern_rule(file)
                            .or_else(|| engine.find_pattern_rule(&resolved_file))
                        {
                            for pp in &rule.prereq_patterns {
                                let src = pattern_subst_with_dir(pp, &stem);
                                let src = engine.resolve_vpath(&src).unwrap_or(src);
                                if let Ok(sm) = std::fs::metadata(&src).and_then(|m| m.modified()) {
                                    if sm > target_m {
                                        return true;
                                    }
                                    // Also check this existing file's own
                                    // chain — its sources might be newer
                                    if check_chain_sources(engine, &src, target_m, depth + 1) {
                                        return true;
                                    }
                                } else if check_chain_sources(engine, &src, target_m, depth + 1) {
                                    return true;
                                }
                            }
                        }
                        false
                    }
                    for prereq in &all_prereqs {
                        let pr = self
                            .resolve_vpath(prereq)
                            .unwrap_or_else(|| prereq.to_string());
                        if !Path::new(prereq.as_str()).exists()
                            && !Path::new(pr.as_str()).exists()
                            && check_chain_sources(self, prereq, tm, 0)
                        {
                            any_source_newer = true;
                            break;
                        }
                    }
                }
            }
            !any_source_newer
        } else {
            false
        };

        // Build prerequisites per-rule (GNU make 4.4.1 semantics):
        // iterate rule groups in order, with the recipe-bearing rule's
        // group placed first. Within each group, build normal prereqs
        // followed by order-only prereqs. A global `seen` set prevents
        // building the same prereq twice across groups.
        //
        // Automatic variables ($^, $<, $?, $+, $|) still preserve their
        // declared order via `all_prereqs`/`all_order_only` (which were
        // already reordered above for the recipe-rule-first display).
        let shuffle_active = self.shuffle_mode.borrow().is_some();
        let mut first_err: Option<String> = None;
        let mut built_seen: HashSet<String> = HashSet::new();

        // Build the per-rule iteration sequence. Each entry is
        // (prereq, is_order_only).
        let mut per_rule_seq: Vec<(String, bool)> = Vec::new();
        if shuffle_active {
            // Flatten all rule groups into a single sequence before
            // shuffling, so prereqs across all rules are shuffled
            // together (matching GNU make).
            let mut all_norm: Vec<String> = Vec::new();
            let mut all_oo: Vec<String> = Vec::new();
            let normal_set: HashSet<&String> = all_prereqs.iter().collect();
            for (norm, oo, _has) in &rule_build_groups {
                all_norm.extend(norm.iter().cloned());
                all_oo.extend(oo.iter().filter(|o| !normal_set.contains(o)).cloned());
            }
            all_norm.extend(all_oo);
            self.shuffle_prereqs(&mut all_norm);
            for p_ in all_norm {
                per_rule_seq.push((p_, false));
            }
        } else {
            for (norm, oo, _has) in &rule_build_groups {
                let norm_v: Vec<String> = norm.clone();
                let normal_set: HashSet<&String> = all_prereqs.iter().collect();
                let oo_v: Vec<String> = oo
                    .iter()
                    .filter(|o| !normal_set.contains(o))
                    .cloned()
                    .collect();
                for p_ in norm_v {
                    per_rule_seq.push((p_, false));
                }
                for p_ in oo_v {
                    per_rule_seq.push((p_, true));
                }
            }
        }

        // GNU make builds non-intermediate prereqs before intermediate
        // ones. This matters when e.g. `%.tsk: %.z test.z` has both an
        // intermediate `hello.z` and an explicitly-mentioned `test.z`:
        // `test.z` must be built first so we know whether the target
        // needs rebuilding (which forces the intermediate to be built).
        // Stable-partition: non-intermediates keep their relative order,
        // intermediates keep theirs, but all non-intermediates come first.
        if pattern_match.is_some() {
            let mut non_intermediate: Vec<(String, bool)> = Vec::new();
            let mut intermediate: Vec<(String, bool)> = Vec::new();
            for entry in per_rule_seq.drain(..) {
                if !entry.1
                    && !Path::new(entry.0.as_str()).exists()
                    && self.check_intermediate(&entry.0, pattern_derived_prereqs.contains(&entry.0))
                    && !self.phony_targets.borrow().contains(&entry.0)
                {
                    intermediate.push(entry);
                } else {
                    non_intermediate.push(entry);
                }
            }
            per_rule_seq = non_intermediate;
            per_rule_seq.extend(intermediate);
        }

        // Per-prereq build with full state tracking. Order-only prereqs
        // skip the mtime/has_phony tracking (they don't influence the
        // target's rebuild decision).
        for (prereq, is_oo) in &per_rule_seq {
            if !built_seen.insert(prereq.clone()) {
                continue;
            }
            // Circular dependency: drop with a warning.
            if self.building_chain.borrow().contains(prereq.as_str()) && prereq != target {
                eprintln!(
                    "make: Circular {} <- {} dependency dropped.",
                    target, prereq
                );
                continue;
            }
            if !*is_oo
                && skip_missing_intermediates
                && !Path::new(prereq.as_str()).exists()
                && self.check_intermediate(prereq, pattern_derived_prereqs.contains(prereq))
                && !self.phony_targets.borrow().contains(prereq)
            {
                continue;
            }
            if let Err(e) = self.build_target_for(prereq, Some(target)) {
                if self.keep_going {
                    if !e.is_empty() {
                        let msg = if e.starts_with('[') {
                            format!("make: *** {e}")
                        } else {
                            format!("make: *** {e}.")
                        };
                        if let Some(ref mut buf) = *self.buffered_kgo_errors.borrow_mut() {
                            buf.push(msg);
                        } else {
                            eprintln!("{msg}");
                        }
                    }
                    if first_err.is_none() {
                        first_err = Some(String::new());
                    }
                    continue;
                }
                clean_skip(self);
                pop_scope(self);
                return Err(e);
            }
            if *is_oo {
                continue;
            }
            if self.rebuilt_targets.borrow().contains(prereq)
                && !self.check_intermediate(prereq, pattern_derived_prereqs.contains(prereq))
            {
                has_rebuilt_prereq = true;
            }
            if let Some(meta) = self.metadata_or_vpath(prereq)
                && let Ok(mtime) = meta.modified()
            {
                newest_prereq = Some(match newest_prereq {
                    Some(t) if mtime > t => mtime,
                    Some(t) => t,
                    None => mtime,
                });
            } else if self.phony_targets.borrow().contains(prereq)
                || self.rules.borrow().contains_key(prereq)
            {
                has_phony_prereq = true;
            } else {
                let is_intermediate =
                    self.check_intermediate(prereq, pattern_derived_prereqs.contains(prereq));
                if !is_intermediate {
                    has_phony_prereq = true;
                }
            }
        }

        if first_err.is_some() {
            self.failed_targets.borrow_mut().insert(target.to_string());
            clean_skip(self);
            pop_scope(self);
            return Err(String::new());
        }

        // Build .EXTRA_PREREQS after all rule prereqs, kept out of $^/$<.
        if !in_extras {
            for prereq in &extra_prereqs {
                if let Err(e) = self.build_target_for(prereq, Some(target)) {
                    if self.keep_going {
                        if !e.is_empty() {
                            let msg = if e.starts_with('[') {
                                format!("make: *** {e}")
                            } else {
                                format!("make: *** {e}.")
                            };
                            if let Some(ref mut buf) = *self.buffered_kgo_errors.borrow_mut() {
                                buf.push(msg);
                            } else {
                                eprintln!("{msg}");
                            }
                        }
                        first_err = Some(String::new());
                        continue;
                    }
                    clean_skip(self);
                    pop_scope(self);
                    return Err(e);
                }
            }
            if first_err.is_some() {
                self.failed_targets.borrow_mut().insert(target.to_string());
                clean_skip(self);
                pop_scope(self);
                return Err(String::new());
            }
        }

        // SE entries already accounted for in all_prereqs/all_order_only.
        let normal_set3: HashSet<String> = all_prereqs.iter().cloned().collect();
        all_order_only.retain(|o| !normal_set3.contains(o));
        clean_skip(self);

        // Determine if we need to rebuild
        let target_mtime = if is_phony {
            None
        } else {
            self.metadata_or_vpath(target)
                .and_then(|m| m.modified().ok())
        };

        // For grouped targets, check whether any sibling is missing or
        // older than the newest prerequisite.  If so, the group needs
        // rebuilding even when *this* target is up to date.
        let mut group_needs_rebuild = false;
        if !is_phony {
            for rule in &rules {
                if !rule.group.is_empty() {
                    for sibling in &rule.group {
                        if sibling == target {
                            continue;
                        }
                        let sibling_mtime = std::fs::metadata(sibling)
                            .ok()
                            .and_then(|m| m.modified().ok());
                        if sibling_mtime.is_none() {
                            group_needs_rebuild = true;
                            break;
                        }
                        if let (Some(s), Some(p)) = (sibling_mtime, newest_prereq)
                            && p > s
                        {
                            group_needs_rebuild = true;
                            break;
                        }
                    }
                    if group_needs_rebuild {
                        break;
                    }
                }
            }
        }

        let needs_rebuild = self.always_make.get()
            || is_phony
            || has_phony_prereq
            || has_assume_new_prereq
            || has_rebuilt_prereq
            || target_mtime.is_none()
            || group_needs_rebuild
            || match (target_mtime, newest_prereq) {
                (Some(t), Some(p)) => p > t,
                _ => false,
            };

        if !needs_rebuild {
            *self.target_had_recipe.borrow_mut() = !recipe.is_empty();
            self.built_targets.borrow_mut().insert(target.to_string());
            pop_scope(self);
            return Ok(());
        }

        // No recipe and target doesn't exist?
        if recipe.is_empty() {
            *self.target_had_recipe.borrow_mut() = false;
            if is_phony || self.file_exists_or_vpath(target) || !rules.is_empty() {
                self.built_targets.borrow_mut().insert(target.to_string());
                pop_scope(self);
                return Ok(());
            }
            let msg = match needed_by {
                Some(parent) => {
                    format!("No rule to make target '{target}', needed by '{parent}'")
                }
                None => format!("No rule to make target '{target}'"),
            };
            self.failed_targets.borrow_mut().insert(target.to_string());
            pop_scope(self);
            return Err(msg);
        }

        // Grouped (`&:`) rules: before running the single recipe,
        // fire SECONDEXPANSION side effects for each sibling target
        // and build the prereqs they expand to. GNU make evaluates
        // each sibling's per-target SE (and the prereqs contributed
        // by each rule attached to that sibling) before the grouped
        // recipe runs. Rules attached to a sibling are processed with
        // the currently-executing grouped rule first, then the other
        // rules in declaration order.
        for rule in &rules {
            if rule.group.is_empty() {
                continue;
            }
            for sibling in &rule.group {
                if sibling == target {
                    continue;
                }
                let raw_sibling_rules = self
                    .rules
                    .borrow()
                    .get(sibling.as_str())
                    .cloned()
                    .unwrap_or_default();
                // Order: non-SE rules first (their prereqs are
                // already resolved and contribute to auto-vars seen by
                // the SE rules), then SE rules with the currently-
                // executing grouped rule first, then the rest in
                // declaration order.
                let mut sibling_rules: Vec<RuleEntry> = Vec::with_capacity(raw_sibling_rules.len());
                let needs_se_for = |sr: &RuleEntry| -> bool {
                    sr.second_expand
                        && (sr
                            .raw_prereq_text
                            .as_deref()
                            .is_some_and(|s| s.contains('$'))
                            || sr
                                .raw_order_only_text
                                .as_deref()
                                .is_some_and(|s| s.contains('$')))
                };
                for sr in &raw_sibling_rules {
                    if !needs_se_for(sr) {
                        sibling_rules.push(sr.clone());
                    }
                }
                for sr in &raw_sibling_rules {
                    if needs_se_for(sr) && sr.group == rule.group {
                        sibling_rules.push(sr.clone());
                    }
                }
                for sr in &raw_sibling_rules {
                    if needs_se_for(sr) && sr.group != rule.group {
                        sibling_rules.push(sr.clone());
                    }
                }
                // Accumulate prereqs from non-SE / already-resolved
                // rules so SE auto-vars ($<, $^, etc.) reflect prior
                // contributions, matching GNU make semantics.
                let mut sib_prereqs: Vec<String> = Vec::new();
                let mut sib_order_only: Vec<String> = Vec::new();
                for sib_rule in &sibling_rules {
                    let needs_se = sib_rule.second_expand
                        && (sib_rule
                            .raw_prereq_text
                            .as_deref()
                            .is_some_and(|s| s.contains('$'))
                            || sib_rule
                                .raw_order_only_text
                                .as_deref()
                                .is_some_and(|s| s.contains('$')));
                    if needs_se {
                        let stem_str = sib_rule.stem.clone().unwrap_or_default();
                        let plus_str = sib_prereqs.join(" ");
                        let mut seen: HashSet<String> = HashSet::new();
                        let caret: Vec<String> = sib_prereqs
                            .iter()
                            .filter(|p| seen.insert((*p).clone()))
                            .cloned()
                            .collect();
                        let caret_str = caret.join(" ");
                        let mut pseen: HashSet<String> = HashSet::new();
                        let pipe_str: String = sib_order_only
                            .iter()
                            .filter(|p| pseen.insert((*p).clone()))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" ");
                        let lt_str = sib_prereqs.first().cloned().unwrap_or_default();
                        let mut auto_vars: HashMap<&str, String> = HashMap::new();
                        auto_vars.insert("@", sibling.clone());
                        auto_vars.insert("*", stem_str);
                        auto_vars.insert("<", lt_str);
                        auto_vars.insert("^", caret_str);
                        auto_vars.insert("+", plus_str);
                        auto_vars.insert("|", pipe_str);
                        auto_vars.insert("?", String::new());
                        auto_vars.insert("%", String::new());
                        add_df_variants(&mut auto_vars);
                        let raw = sib_rule.raw_prereq_text.as_deref().unwrap_or("");
                        let exp = self.with_target_vars_applied(sibling, || {
                            expand::expand_with_auto(raw, self, &auto_vars)
                        });
                        for tok in exp.split_whitespace() {
                            sib_prereqs.push(tok.to_string());
                        }
                        if let Some(orig_oo) = &sib_rule.raw_order_only_text {
                            let oo_exp = expand::expand_with_auto(orig_oo, self, &auto_vars);
                            for tok in oo_exp.split_whitespace() {
                                sib_order_only.push(tok.to_string());
                            }
                        }
                    } else {
                        sib_prereqs.extend(
                            sib_rule
                                .prerequisites
                                .iter()
                                .map(|p| self.resolve_library_prereq(p)),
                        );
                        sib_order_only.extend(sib_rule.order_only.iter().cloned());
                    }
                }
                // Build sibling prereqs derived from SE expansion.
                for prereq in &sib_prereqs {
                    if prereq == target || prereq == sibling {
                        continue;
                    }
                    if self.building_chain.borrow().contains(prereq.as_str()) {
                        continue;
                    }
                    if let Err(e) = self.build_target_for(prereq, Some(sibling))
                        && !self.keep_going
                    {
                        pop_scope(self);
                        return Err(e);
                    }
                }
                for prereq in &sib_order_only {
                    if prereq == target || prereq == sibling {
                        continue;
                    }
                    if self.building_chain.borrow().contains(prereq.as_str()) {
                        continue;
                    }
                    if let Err(e) = self.build_target_for(prereq, Some(sibling))
                        && !self.keep_going
                    {
                        pop_scope(self);
                        return Err(e);
                    }
                }
            }
        }

        // Execute recipe
        if self.touch {
            if !is_phony {
                if !self.silent {
                    println!("touch {target}");
                }
                *self.recipe_executed.borrow_mut() = true;
                self.rebuilt_targets.borrow_mut().insert(target.to_string());
                // Dry-run + touch: print only, don't actually touch.
                if !self.dry_run {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(false)
                        .write(true)
                        .open(target)
                        .ok();
                }
            }
        } else if self.question {
            // -q: don't execute the recipe; report that update was
            // needed — but ignore recipes that would produce nothing
            // to run (empty/whitespace-only lines, or lines that
            // expand to empty). A target whose entire recipe is a
            // no-op doesn't count as "needing update".
            let has_real_recipe = recipe.iter().any(|line| {
                let mut s = line.as_str().trim_start();
                while let Some(r) = s.strip_prefix(['@', '-', '+']) {
                    s = r.trim_start();
                }
                if s.trim().is_empty() {
                    return false;
                }
                let expanded = expand::expand(s, self);
                // Collapse backslash-newline (shell continuations) before
                // checking emptiness — they are not meaningful content.
                let collapsed = expanded.replace(
                    "\\\
", " ",
                );
                !collapsed.trim().is_empty()
            });
            if has_real_recipe {
                *self.question_needs_update.borrow_mut() = true;
            }
        } else {
            // --trace: print trace line before executing recipe
            if self.trace {
                let trace_line_no = recipe_lines.first().copied().unwrap_or(0);
                let reason = if target_mtime.is_none() && !is_phony {
                    "target does not exist".to_string()
                } else if self.always_make.get() {
                    "always-make flag is set".to_string()
                } else if is_phony {
                    "target is .PHONY".to_string()
                } else {
                    // Find which prereqs are newer
                    let newer: Vec<&str> = all_prereqs
                        .iter()
                        .filter(|p| {
                            if let (Some(tm), Ok(pm)) = (
                                target_mtime,
                                std::fs::metadata(p.as_str()).and_then(|m| m.modified()),
                            ) {
                                pm > tm
                            } else {
                                false
                            }
                        })
                        .map(|s| s.as_str())
                        .collect();
                    if newer.is_empty() {
                        "target does not exist".to_string()
                    } else {
                        newer.join(" ")
                    }
                };
                eprintln!(
                    "{}:{}: update target '{}' due to: {}",
                    recipe_source, trace_line_no, target, reason
                );
            }
            // Resolve prerequisites through VPATH for automatic variables.
            // GNU make replaces prereq names with VPATH-resolved paths
            // in $<, $^, $?, $+, $|.
            let vpath_prereqs: Vec<String> = all_prereqs
                .iter()
                .map(|p| self.resolve_vpath(p).unwrap_or_else(|| p.clone()))
                .collect();
            let vpath_order_only: Vec<String> = all_order_only
                .iter()
                .map(|p| self.resolve_vpath(p).unwrap_or_else(|| p.clone()))
                .collect();
            let vpath_implied: Vec<String> = implied_prereqs
                .iter()
                .map(|p| self.resolve_vpath(p).unwrap_or_else(|| p.clone()))
                .collect();
            match self.execute_recipe(
                target,
                &recipe,
                &recipe_lines,
                &recipe_source,
                &vpath_prereqs,
                &vpath_order_only,
                &vpath_implied,
                &stem,
            ) {
                Err(e) => {
                    // `.DELETE_ON_ERROR:` removes the target file so a partial
                    // build isn't mistaken for a successful one next time.
                    // GNU emits the error diagnostic first, then the deletion notice.
                    let is_precious = self.precious_targets.borrow().contains(target)
                        || self
                            .precious_patterns
                            .borrow()
                            .iter()
                            .any(|pat| expand::pattern_stem(target, pat).is_some());
                    if *self.delete_on_error.borrow()
                        && !is_phony
                        && !is_precious
                        && Path::new(target).exists()
                    {
                        // Emit diagnostics in the correct order and consume the
                        // error so the outer caller doesn't print it again.
                        if e.starts_with('[') {
                            eprintln!("make: *** {e}");
                        } else {
                            eprintln!("make: *** {e}.  Stop.");
                        }
                        eprintln!("make: *** Deleting file '{target}'");
                        let _ = std::fs::remove_file(target);
                        // Mark pattern rule siblings as built so they don't
                        // re-run the same recipe.
                        if let Some((ref pat_rule, _)) = pattern_match {
                            for sibling_pat in &pat_rule.sibling_patterns {
                                let sibling = sibling_pat.replacen('%', &stem, 1);
                                self.built_targets.borrow_mut().insert(sibling);
                            }
                        }
                        self.failed_targets.borrow_mut().insert(target.to_string());
                        pop_scope(self);
                        return Err(String::new());
                    }
                    // Mark pattern rule siblings as built so they don't
                    // re-run the same recipe.
                    if let Some((ref pat_rule, _)) = pattern_match {
                        for sibling_pat in &pat_rule.sibling_patterns {
                            let sibling = sibling_pat.replacen('%', &stem, 1);
                            self.built_targets.borrow_mut().insert(sibling);
                        }
                    }
                    pop_scope(self);
                    return Err(e);
                }
                Ok(ran) => {
                    if ran {
                        // Mark this target as rebuilt so dependents see it
                        // as updated (needed in dry-run where file mtime
                        // doesn't actually change).
                        self.rebuilt_targets.borrow_mut().insert(target.to_string());
                    }
                }
            }
        }

        self.built_targets.borrow_mut().insert(target.to_string());
        // Mark sibling targets in the same `&:` group as built too —
        // the single recipe we just ran is considered to have updated
        // the whole group.  Also mark the group recipe as done and
        // add siblings to group_built_targets so the top-level build
        // loop can emit the correct diagnostic.
        for rule in &rules {
            if !rule.group.is_empty() {
                let key = rule.group.join("\0");
                self.group_recipe_done.borrow_mut().insert(key.clone());
                {
                    let mut built = self.built_targets.borrow_mut();
                    for sibling in &rule.group {
                        if sibling != target {
                            built.insert(sibling.clone());
                            self.group_built_targets
                                .borrow_mut()
                                .insert(sibling.clone());
                            self.rebuilt_targets.borrow_mut().insert(sibling.clone());
                        }
                    }
                }
            }
        }
        // Multi-target pattern rules: mark sibling targets (other
        // target patterns with the same stem) as built. If the recipe
        // ran but didn't update a sibling's file, warn (GNU make).
        if let Some((pat_rule, _)) = &pattern_match
            && !pat_rule.sibling_patterns.is_empty()
        {
            for sibling_pat in &pat_rule.sibling_patterns {
                let sibling = sibling_pat.replacen('%', &stem, 1);
                // GNU make only warns when the recipe actually created
                // or updated the target file. If the target already
                // existed before the recipe ran and wasn't modified,
                // no warning is emitted (the recipe didn't "update" it).
                let nontrivial_recipe = pat_rule.recipe.iter().any(|l| !l.trim().is_empty());
                // Check if the target's mtime changed (recipe actually
                // updated the file). Compare current mtime against the
                // pre-recipe mtime we captured earlier.
                let target_was_updated = if let (Some(before), Ok(after_meta)) =
                    (pre_recipe_target_mtime, std::fs::metadata(target))
                {
                    // Target existed before — only "updated" if mtime changed.
                    after_meta
                        .modified()
                        .map(|after| after > before)
                        .unwrap_or(false)
                } else {
                    // Target didn't exist before — "updated" if it exists now.
                    Path::new(target).exists()
                };
                if nontrivial_recipe
                    && !pat_rule.is_grouped
                    && !self.dry_run
                    && !self.touch
                    && !self.question
                    && self.rebuilt_targets.borrow().contains(target)
                    && target_was_updated
                    && !self.phony_targets.borrow().contains(&sibling)
                    && !Path::new(&sibling).exists()
                {
                    let line_no = pat_rule.recipe_lines.first().copied().unwrap_or(0);
                    eprintln!(
                        "{}:{}: warning: pattern recipe did not update peer target '{}'.",
                        pat_rule.source_name, line_no, sibling
                    );
                }
                self.built_targets.borrow_mut().insert(sibling.clone());
                self.rebuilt_targets.borrow_mut().insert(sibling.clone());
            }
        }

        // GNU make "un-vpath": after running a target's recipe, if the
        // file was not created locally, revoke VPATH resolution so that
        // subsequent rules using this file as a prereq see the local
        // (missing) name in $< / $^ instead of the VPATH-resolved path.
        if self.rebuilt_targets.borrow().contains(target)
            && !is_phony
            && !Path::new(target).exists()
        {
            self.vpath_revoked.borrow_mut().insert(target.to_string());
        }

        // Collect intermediate files for deferred deletion. They are
        // deleted after the top-level goal completes (in `build`).
        if !self.dry_run && !self.touch && !self.question {
            let rebuilt = self.rebuilt_targets.borrow();
            let preexisting = self.vpath_preexisting.borrow();
            let mut pending = self.pending_intermediate_deletions.borrow_mut();
            for prereq in &all_prereqs {
                let is_pat = pattern_derived_prereqs.contains(prereq);
                if rebuilt.contains(prereq.as_str())
                    && self.check_intermediate(prereq, is_pat)
                    && !self.secondary_targets.borrow().contains(prereq.as_str())
                    && !self.secondary_all.get()
                    && !preexisting.contains(prereq.as_str())
                    && !pending.iter().any(|(p, _)| p == prereq)
                {
                    pending.push((prereq.clone(), is_pat));
                }
            }
        }

        pop_scope(self);
        Ok(())
    }

    /// Resolve `-l<name>` prereqs using .LIBPATTERNS with GNU make's
    /// linker-compatible search order: try all LIBPATTERNS candidates
    /// locally first (return immediately if found), then try all
    /// candidates through vpath and pick the one with the earliest
    /// vpath position (lowest vpath_index, then lowest path_index).
    ///
    /// Also emits a "same file" warning when `-l<name>` matches a
    /// pattern rule (e.g. `-l%: lib%.a`) AND the resolved library
    /// candidate has its own explicit rule — the pattern rule's
    /// recipe is ignored in favor of the explicit rule (SV 54549).
    fn resolve_library_prereq(&self, name: &str) -> String {
        let Some(lib) = name.strip_prefix("-l") else {
            return name.to_string();
        };
        let patterns = self.lookup_var(".LIBPATTERNS");
        let mut first: Option<String> = None;
        let mut candidates: Vec<String> = Vec::new();
        for pat in patterns.split_whitespace() {
            if !pat.contains('%') {
                eprintln!("make: .LIBPATTERNS element '{pat}' is not a pattern");
                continue;
            }
            let candidate = pat.replacen('%', lib, 1);
            if first.is_none() {
                first = Some(candidate.clone());
            }
            // Check locally or as an explicit rule target — return immediately.
            if std::path::Path::new(&candidate).exists()
                || self.rules.borrow().contains_key(&candidate)
            {
                // Emit LIBPATTERNS conflict warning if the original -l name
                // matches a pattern rule whose recipe would be ignored.
                self.warn_lib_pattern_conflict(name, &candidate);
                return candidate;
            }
            candidates.push(candidate);
        }

        // Try all candidates through vpath and pick the earliest match.
        let mut best: Option<(String, usize, usize)> = None;
        for candidate in &candidates {
            if let Some((resolved, vi, pi)) = self.resolve_vpath_with_index(candidate) {
                let dominated = match &best {
                    None => true,
                    Some((_, bv, bp)) => vi < *bv || (vi == *bv && pi < *bp),
                };
                if dominated {
                    best = Some((resolved, vi, pi));
                }
            }
        }
        if let Some((resolved, _, _)) = best {
            return resolved;
        }

        first.unwrap_or_else(|| name.to_string())
    }

    /// Emit a "same file" warning when a `-l<name>` prereq matches a
    /// pattern rule (like `-l%: lib%.a`) and the resolved LIBPATTERNS
    /// candidate (like `libcat.a`) has its own explicit rule. GNU make
    /// ignores the pattern rule's recipe in favor of the explicit one.
    fn warn_lib_pattern_conflict(&self, lib_name: &str, resolved: &str) {
        // Check if the resolved candidate has an explicit rule with a recipe.
        let has_explicit_recipe = self
            .rules
            .borrow()
            .get(resolved)
            .map(|entries| entries.iter().any(|e| !e.recipe.is_empty()))
            .unwrap_or(false);
        if !has_explicit_recipe {
            return;
        }
        // Check if the -l name matches any pattern rule that also has a recipe.
        // Prefer user-defined rules over built-in ones (built-in rules have
        // source_name "<built-in>").
        let pat_rules = self.pattern_rules.borrow();
        let mut best_match: Option<(&str, usize)> = None;
        for pr in pat_rules.iter() {
            if pr.recipe.is_empty() {
                continue;
            }
            if expand::pattern_stem(lib_name, &pr.target_pattern).is_some() {
                let line_no = pr.recipe_lines.first().copied().unwrap_or(0);
                let is_builtin = pr.source_name == "<built-in>";
                // Prefer user-defined over built-in; last user-defined wins.
                if best_match.is_none() || !is_builtin {
                    best_match = Some((&pr.source_name, line_no));
                }
            }
        }
        if let Some((source, line_no)) = best_match {
            eprintln!(
                "{}:{}: Recipe was specified for file '{}' at {}:{},",
                source, line_no, lib_name, source, line_no
            );
            eprintln!(
                "{}:{}: but '{}' is now considered the same file as '{}'.",
                source, line_no, lib_name, resolved
            );
            eprintln!(
                "{}:{}: Recipe for '{}' will be ignored in favor of the one for '{}'.",
                source, line_no, lib_name, resolved
            );
        }
    }

    /// Check if a file is "mentioned" in the makefile — either as an
    /// explicit target or as a prerequisite of a non-pattern (explicit)
    /// rule. Such files "should exist" for terminal pattern rule matching.
    fn is_mentioned_file(&self, file: &str) -> bool {
        let rules = self.rules.borrow();
        // Check if it is an explicit target.
        if rules.contains_key(file) {
            return true;
        }
        // Check if it appears as a prerequisite of any explicit rule.
        for entries in rules.values() {
            for entry in entries {
                if entry.prerequisites.iter().any(|p| p == file) {
                    return true;
                }
            }
        }
        false
    }

    /// Check whether a file matches any `.NOTINTERMEDIATE` pattern (e.g. `%.x`).
    fn matches_notintermediate_pattern(&self, file: &str) -> bool {
        for pat in self.notintermediate_patterns.borrow().iter() {
            if expand::pattern_stem(file, pat).is_some() {
                return true;
            }
        }
        false
    }

    /// Determine whether a prereq file should be treated as intermediate
    /// (its absence does NOT trigger a rebuild of the parent).
    ///
    /// Priority (highest to lowest):
    /// 1. Per-file `.INTERMEDIATE: file`  → intermediate
    /// 2. Per-file `.SECONDARY: file`     → intermediate (secondary)
    /// 3. Per-file `.NOTINTERMEDIATE: file` → NOT intermediate
    /// 4. Pattern `.NOTINTERMEDIATE: %.x`  → NOT intermediate
    /// 5. Global `.NOTINTERMEDIATE:`       → NOT intermediate
    /// 6. Global `.SECONDARY:`             → intermediate (secondary)
    /// 7. Default: pattern-derived && !mentioned → intermediate
    fn check_intermediate(&self, file: &str, is_pattern_derived: bool) -> bool {
        // 1. Explicit .INTERMEDIATE on this file always wins
        if self.intermediate_targets.borrow().contains(file) {
            return true;
        }
        // 2. Explicit .SECONDARY on this file wins over patterns/global
        if self.secondary_targets.borrow().contains(file) {
            return true;
        }
        // 3. Explicit .NOTINTERMEDIATE on this file
        if self.notintermediate_files.borrow().contains(file) {
            return false;
        }
        // 4. Pattern .NOTINTERMEDIATE
        if self.matches_notintermediate_pattern(file) {
            return false;
        }
        // 5. Global .NOTINTERMEDIATE
        if self.notintermediate_all.get() {
            return false;
        }
        // 6. Global .SECONDARY
        if self.secondary_all.get() {
            return true;
        }
        // 7. Default intermediacy: pattern-derived and not explicitly mentioned
        is_pattern_derived && !self.is_mentioned_file(file)
    }

    fn find_pattern_rule(&self, target: &str) -> Option<(PatternRuleEntry, String)> {
        let mut chain = Vec::new();
        self.find_pattern_rule_inner(target, 0, &mut chain)
    }

    /// Depth-limited implicit rule search. Recursively checks whether a
    /// pattern rule's prerequisites can be satisfied (exist on disk, have
    /// an explicit rule, or can be built by further implicit rule
    /// chaining up to MAX_IMPLICIT_CHAIN_DEPTH).
    ///
    /// Improvements over naive first-match:
    /// - Shortest-stem selection: among all matching rules, prefer the
    ///   one with the shortest stem (most specific match).
    /// - Match-anything restriction: bare `%` rules cannot build
    ///   intermediates that aren't explicitly mentioned in the makefile.
    /// - Cycle detection: tracks the chain of targets being resolved
    ///   and treats circular prerequisites as satisfied (dropped at
    ///   build time with a warning).
    fn find_pattern_rule_inner(
        &self,
        target: &str,
        depth: usize,
        chain: &mut Vec<String>,
    ) -> Option<(PatternRuleEntry, String)> {
        const MAX_IMPLICIT_CHAIN_DEPTH: usize = 6;
        if depth >= MAX_IMPLICIT_CHAIN_DEPTH {
            return None;
        }

        // Cycle detection: if this target is already in the resolution
        // chain, we have a circular dependency. Return None so the
        // caller skips this prereq (it will be dropped at build time).
        if chain.contains(&target.to_string()) {
            return None;
        }
        chain.push(target.to_string());

        let suffixes_cleared = *self.suffixes_cleared.borrow();
        let pattern_rules = self.pattern_rules.borrow();
        let result =
            self.find_pattern_rule_search(target, depth, chain, &pattern_rules, suffixes_cleared);

        chain.pop();
        result
    }

    /// Inner search logic for find_pattern_rule_inner, separated to
    /// allow the chain push/pop to wrap this cleanly.
    fn find_pattern_rule_search(
        &self,
        target: &str,
        depth: usize,
        chain: &mut Vec<String>,
        pattern_rules: &[PatternRuleEntry],
        suffixes_cleared: bool,
    ) -> Option<(PatternRuleEntry, String)> {
        // Collect user-defined pattern rules with empty recipes — these
        // cancel corresponding built-in rules with the same target AND
        // prereq patterns (GNU make semantics).
        let mut cancelled_rules: Vec<(&str, &Vec<String>)> = Vec::new();
        for rule in pattern_rules.iter() {
            if rule.source_name != "<built-in>"
                && rule.recipe.is_empty()
                && expand::pattern_stem(target, &rule.target_pattern).is_some()
            {
                cancelled_rules.push((&rule.target_pattern, &rule.prereq_patterns));
            }
        }

        // Collect all matching (rule, stem) pairs and pick the shortest
        // stem (most specific match). GNU make prefers shorter stems.
        for allow_chaining in [false, true] {
            let mut best: Option<(PatternRuleEntry, String)> = None;

            // First: user-defined rules
            for rule in pattern_rules.iter() {
                if rule.source_name == "<built-in>" {
                    continue;
                }
                if let Some(candidate) =
                    self.try_pattern_rule(target, rule, depth, chain, allow_chaining)
                    && best
                        .as_ref()
                        .is_none_or(|(_, s)| candidate.1.len() < s.len())
                {
                    best = Some(candidate);
                }
            }

            // Second: built-in rules (only if suffixes not cleared)
            if !suffixes_cleared {
                for rule in pattern_rules.iter() {
                    if rule.source_name != "<built-in>" {
                        continue;
                    }
                    // Skip built-in rules cancelled by user-defined
                    // empty-recipe rules with matching target AND prereq patterns.
                    if cancelled_rules
                        .iter()
                        .any(|(tp, pp)| *tp == rule.target_pattern && **pp == rule.prereq_patterns)
                    {
                        continue;
                    }
                    if let Some(candidate) =
                        self.try_pattern_rule(target, rule, depth, chain, allow_chaining)
                        && best
                            .as_ref()
                            .is_none_or(|(_, s)| candidate.1.len() < s.len())
                    {
                        best = Some(candidate);
                    }
                }
            }

            if best.is_some() {
                return best;
            }
        }
        None
    }

    /// Try a single pattern rule against a target. Returns Some((rule, stem))
    /// if the rule matches and all its prerequisites are satisfiable.
    fn try_pattern_rule(
        &self,
        target: &str,
        rule: &PatternRuleEntry,
        depth: usize,
        chain: &mut Vec<String>,
        allow_chaining: bool,
    ) -> Option<(PatternRuleEntry, String)> {
        let stem = expand::pattern_stem(target, &rule.target_pattern)?;

        if rule.recipe.is_empty() {
            return None;
        }

        // Non-terminal match-anything rules (target pattern is just `%`)
        // cannot build intermediates unless the target is an explicit
        // target in the makefile (has its own rule entry). Being merely
        // a prerequisite of some other rule doesn't count.
        // Terminal match-anything rules don't have this restriction.
        if depth > 0
            && rule.target_pattern == "%"
            && !rule.is_terminal
            && !self.rules.borrow().contains_key(target)
        {
            return None;
        }

        if rule.is_terminal {
            // For SE terminal rules, defer prereq verification to build
            // time so side-effect functions like $(info) don't fire twice.
            if rule.second_expand && rule.raw_prereq_text.is_some() {
                return Some((rule.clone(), stem));
            }
            // Terminal rules: prereqs must exist on disk, be explicit
            // targets, or (in pass 2) be mentioned in the makefile.
            // No implicit chaining allowed for terminal rules.
            let prereqs_ok = rule.prereq_patterns.is_empty()
                || rule.prereq_patterns.iter().all(|pp| {
                    let prereq = pattern_subst_with_dir(pp, &stem);
                    // Circular prereq: treat as satisfied (dropped at build time)
                    if chain.contains(&prereq) {
                        return true;
                    }
                    self.file_exists_or_vpath(&prereq)
                        || self.rules.borrow().contains_key(prereq.as_str())
                        || self.resolve_vpath_rule(&prereq).is_some()
                        || (allow_chaining && self.is_mentioned_file(&prereq))
                });
            return if prereqs_ok {
                Some((rule.clone(), stem))
            } else {
                None
            };
        }

        // For .SECONDEXPANSION pattern rules, the prereq text may
        // contain side-effect-having functions like $(info). We can't
        // expand here without those firing twice (once for matching
        // verification, once for actual build). Accept the rule as
        // matching and let build_target_for handle verification.
        let se_expanded_prereqs: Option<Vec<String>> =
            if rule.second_expand && rule.raw_prereq_text.is_some() {
                Some(Vec::new())
            } else {
                None
            };

        // Non-terminal rule: two-pass prereq check.
        // Pass 1 (allow_chaining=false): prereqs must exist on disk,
        //   be phony, or be explicit targets. No is_mentioned_file.
        // Pass 2 (allow_chaining=true): additionally, prereqs can be
        //   satisfied by is_mentioned_file or by chaining through
        //   another implicit rule.
        let prereq_iter: Vec<String> = if let Some(se) = &se_expanded_prereqs {
            se.clone()
        } else {
            rule.prereq_patterns
                .iter()
                .map(|pp| pattern_subst_with_dir(pp, &stem))
                .collect()
        };
        let prereqs_ok = prereq_iter.is_empty()
            || prereq_iter.iter().all(|prereq| {
                let prereq = prereq.clone();
                // Circular prereq in chain: treat as satisfied (will be
                // dropped with a warning at build time).
                if chain.contains(&prereq) {
                    return true;
                }
                if prereq.contains(['*', '?', '['])
                    && let Ok(mut paths) = glob::glob(&prereq)
                    && paths.next().is_some()
                {
                    return true;
                }
                self.file_exists_or_vpath(&prereq)
                    || self.phony_targets.borrow().contains(&prereq)
                    || self.rules.borrow().contains_key(prereq.as_str())
                    || self.resolve_vpath_rule(&prereq).is_some()
                    || (allow_chaining
                        && (self.is_mentioned_file(&prereq)
                            || self
                                .find_pattern_rule_inner(&prereq, depth + 1, chain)
                                .is_some()))
            });

        if prereqs_ok {
            Some((rule.clone(), stem))
        } else {
            None
        }
    }

    /// Run `f` with this target's collected target/pattern-specific
    /// variables temporarily applied to `self.vars`, mirroring what
    /// `execute_recipe` does for recipe execution. Restores prior
    /// state on return. Used by `.SECONDEXPANSION` so prereq text
    /// like `$$(x_a)` resolves to target-specific values.
    fn with_target_vars_applied<R>(&self, target: &str, f: impl FnOnce() -> R) -> R {
        let mut tv_entries: Vec<(String, AssignOp, String, bool, bool)> =
            self.collect_target_vars(target);
        for entry in &self.collect_pattern_vars(target) {
            if entry.4 {
                tv_entries.push(entry.clone());
            }
        }
        if let Some(entries) = self.target_vars.borrow().get(target) {
            for entry in entries {
                if entry.4 {
                    tv_entries.push(entry.clone());
                }
            }
        }
        let mut saved: Vec<(String, Option<Variable>)> = Vec::new();
        let override_names: std::collections::HashSet<String> = tv_entries
            .iter()
            .filter(|(_, _, _, is_override, _)| *is_override)
            .map(|(name, _, _, _, _)| expand::expand(name, self))
            .collect();
        for (name, op, value, is_override, _is_private) in &tv_entries {
            let name = expand::expand(name, self);
            saved.push((name.clone(), self.vars.borrow().get(&name).cloned()));
            let original_origin = saved.last().and_then(|(_, v)| v.as_ref().map(|v| v.origin));
            if !is_override
                && (matches!(original_origin, Some(VarOrigin::CommandLine))
                    || override_names.contains(&name))
            {
                continue;
            }
            if matches!(op, AssignOp::Conditional) && self.is_var_defined(&name) {
                continue;
            }
            let flavor = match op {
                AssignOp::Simple | AssignOp::Shell => VarFlavor::Simple,
                _ => VarFlavor::Recursive,
            };
            self.vars.borrow_mut().insert(
                name.clone(),
                Variable {
                    value: value.clone(),
                    flavor,
                    origin: VarOrigin::Automatic,
                },
            );
        }
        let result = f();
        // Restore in reverse order.
        for (name, prev) in saved.into_iter().rev() {
            let mut vars = self.vars.borrow_mut();
            match prev {
                Some(v) => {
                    vars.insert(name, v);
                }
                None => {
                    vars.remove(&name);
                }
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_recipe(
        &self,
        target: &str,
        recipe: &[String],
        recipe_lines: &[usize],
        recipe_source: &str,
        prereqs: &[String],
        order_only: &[String],
        implied_prereqs: &[String],
        stem: &str,
    ) -> Result<bool, String> {
        // Apply target-specific variable assignments (including those
        // inherited from ancestor targets currently being built). Save
        // prior values so we can restore them afterwards.
        let mut tv_entries: Vec<(String, AssignOp, String, bool, bool)> =
            self.collect_target_vars(target);
        // Also include this target's own private vars (which were NOT
        // pushed onto the scope stack): first from pattern-specific,
        // then from target-specific.
        for entry in &self.collect_pattern_vars(target) {
            if entry.4 {
                // is_private
                tv_entries.push(entry.clone());
            }
        }
        if let Some(own) = self.target_vars.borrow().get(target) {
            for entry in own {
                if entry.4 {
                    // is_private
                    tv_entries.push(entry.clone());
                }
            }
        }
        // Target-specific `export VAR := …` — add VAR to the export
        // set for the duration of this recipe. Include pattern-specific
        // exports. Inherited exports from ancestor targets are already
        // applied to the global export/unexport sets by the scope push
        // in build_target_for, so we only need own exports here.
        let mut export_names: Vec<String> = self
            .target_exports
            .borrow()
            .get(target)
            .cloned()
            .unwrap_or_default();
        export_names.extend(self.collect_pattern_exports(target));
        let mut added_exports: Vec<String> = Vec::new();
        let mut added_unexports: Vec<String> = Vec::new();
        let mut removed_from_unexports: Vec<String> = Vec::new();
        {
            let mut exports = self.exports.borrow_mut();
            let mut unexports = self.unexports.borrow_mut();
            for n in &export_names {
                if let Some(uname) = n.strip_prefix('~') {
                    // Target-specific unexport
                    exports.remove(uname);
                    if unexports.insert(uname.to_string()) {
                        added_unexports.push(uname.to_string());
                    }
                } else {
                    // Target-specific export: also remove from unexports
                    // in case a global `unexport` was in effect.
                    if unexports.remove(n) {
                        removed_from_unexports.push(n.clone());
                    }
                    if exports.insert(n.clone()) {
                        added_exports.push(n.clone());
                    }
                }
            }
        }
        let override_vars: std::collections::HashSet<String> = tv_entries
            .iter()
            .filter(|(_, _, _, is_override, _)| *is_override)
            .map(|(name, _, _, _, _)| expand::expand(name, self))
            .collect();

        let mut saved: Vec<(String, Option<Variable>)> = Vec::new();
        for (name, op, value, is_override, _is_private) in &tv_entries {
            let name = expand::expand(name, self);
            saved.push((name.clone(), self.vars.borrow().get(&name).cloned()));

            // Check the ORIGINAL origin (from the saved state) to avoid
            // false negatives when a prior entry in this loop changed the origin.
            let original_origin = saved.last().and_then(|(_, v)| v.as_ref().map(|v| v.origin));

            // Skip non-override entries if:
            // 1. The current var was originally a command-line variable, OR
            // 2. Another target-specific entry for this var has `override`
            if !is_override
                && (matches!(original_origin, Some(VarOrigin::CommandLine))
                    || override_vars.contains(&name))
            {
                continue;
            }

            if matches!(op, AssignOp::Conditional) && self.is_var_defined(&name) {
                continue;
            }

            if matches!(op, AssignOp::Append) {
                // `+=` — append to existing value, respecting flavor.
                let existing_flavor = self.var_flavor(&name);
                let existing = self.lookup_var_raw(&name);
                let keep_raw = existing_flavor != VarFlavor::Simple
                    || self.immediate_recursive.borrow().contains(&name);
                let rhs = if keep_raw {
                    value.clone()
                } else {
                    expand::expand(value, self)
                };
                let new_value = if existing.is_empty() {
                    rhs
                } else if rhs.is_empty() {
                    existing
                } else {
                    format!("{existing} {rhs}")
                };
                let flavor = if existing_flavor == VarFlavor::Undefined {
                    VarFlavor::Recursive
                } else {
                    existing_flavor
                };
                self.vars.borrow_mut().insert(
                    name.clone(),
                    Variable {
                        value: new_value,
                        flavor,
                        origin: VarOrigin::Automatic,
                    },
                );
            } else {
                let flavor = match op {
                    AssignOp::Simple | AssignOp::Shell => VarFlavor::Simple,
                    _ => VarFlavor::Recursive,
                };
                // Note: Simple (:=) target-specific values are
                // already expanded at declaration time in
                // process_directive, so use as-is. Recursive (=)
                // values are stored raw and will be expanded on
                // lookup by lookup_var_with_auto.
                let final_value = value.clone();
                self.vars.borrow_mut().insert(
                    name.clone(),
                    Variable {
                        value: final_value,
                        flavor,
                        origin: VarOrigin::Automatic,
                    },
                );
            }
        }
        // Temporarily remove global `private` variables that were NOT
        // overridden by a target-specific assignment. This ensures
        // private globals are invisible inside recipe execution while
        // target-specific values remain.
        let overridden_names: std::collections::HashSet<String> =
            saved.iter().map(|(n, _)| n.clone()).collect();
        let mut private_saved: Vec<(String, Option<Variable>)> = Vec::new();
        for pname in self.private_vars.borrow().iter() {
            if !overridden_names.contains(pname) {
                let old = self.vars.borrow().get(pname).cloned();
                if old.is_some() {
                    private_saved.push((pname.clone(), old));
                    self.vars.borrow_mut().remove(pname);
                }
            }
        }
        // Also temporarily remove private exports, but NOT if this
        // target has its own (target-specific) export for the same var.
        let target_export_set: std::collections::HashSet<&str> = export_names
            .iter()
            .filter_map(|n| {
                if n.starts_with('~') {
                    None
                } else {
                    Some(n.as_str())
                }
            })
            .collect();
        let mut private_export_saved: Vec<String> = Vec::new();
        for pname in self.private_exports.borrow().iter() {
            if !target_export_set.contains(pname.as_str()) && self.exports.borrow().contains(pname)
            {
                private_export_saved.push(pname.clone());
                self.exports.borrow_mut().remove(pname);
            }
        }

        let result = self.execute_recipe_inner(
            target,
            recipe,
            recipe_lines,
            recipe_source,
            prereqs,
            order_only,
            implied_prereqs,
            stem,
        );
        // Restore private global variables.
        for (name, prev) in private_saved {
            if let Some(v) = prev {
                self.vars.borrow_mut().insert(name, v);
            }
        }
        // Restore private exports.
        for name in private_export_saved {
            self.exports.borrow_mut().insert(name);
        }

        // Restore prior variable state.
        for (name, prev) in saved.into_iter().rev() {
            let mut vars = self.vars.borrow_mut();
            match prev {
                Some(v) => {
                    vars.insert(name, v);
                }
                None => {
                    vars.remove(&name);
                }
            }
        }
        // Remove the target-specific exports we added above.
        {
            let mut exports = self.exports.borrow_mut();
            for n in &added_exports {
                exports.remove(n);
            }
        }
        // Remove the target-specific unexports we added above.
        {
            let mut unexports = self.unexports.borrow_mut();
            for n in &added_unexports {
                unexports.remove(n);
            }
        }
        // Restore unexports that were removed by target-specific exports.
        {
            let mut unexports = self.unexports.borrow_mut();
            for n in &removed_from_unexports {
                unexports.insert(n.clone());
            }
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_recipe_inner(
        &self,
        target: &str,
        recipe: &[String],
        recipe_lines: &[usize],
        recipe_source: &str,
        prereqs: &[String],
        order_only: &[String],
        implied_prereqs: &[String],
        stem: &str,
    ) -> Result<bool, String> {
        *self.in_recipe.borrow_mut() = true;
        let mut any_commands = false;
        // `.WAIT` is a synchronization barrier, never a real file — exclude
        // it from all automatic variables.
        let filter_wait = |v: &[String]| -> Vec<String> {
            v.iter()
                .filter(|s| s.as_str() != ".WAIT")
                .cloned()
                .collect()
        };
        let prereqs_v = filter_wait(prereqs);
        let order_only_v = filter_wait(order_only);
        let implied_v = filter_wait(implied_prereqs);

        // Set up automatic variables
        let mut auto_vars: HashMap<&str, String> = HashMap::new();
        auto_vars.insert("@", target.to_string());
        // $< is the first prerequisite. When a pattern/suffix rule provides
        // the recipe, $< should be the implied source (e.g., the .c file),
        // not an explicit order-only prerequisite like .dirstamp.
        let first_prereq = if !implied_v.is_empty() {
            implied_v.first().cloned().unwrap_or_default()
        } else {
            prereqs_v.first().cloned().unwrap_or_default()
        };
        auto_vars.insert("<", first_prereq);
        auto_vars.insert("^", dedup_join(&prereqs_v));
        auto_vars.insert("+", prereqs_v.join(" "));
        auto_vars.insert("*", stem.to_string());

        // $? = prerequisites newer than target.
        // With -B (always-make), all prerequisites are considered newer.
        let newer_str = if self.always_make.get() {
            prereqs_v.join(" ")
        } else {
            let target_mtime = std::fs::metadata(target)
                .ok()
                .and_then(|m| m.modified().ok());
            let newer: Vec<&str> = prereqs_v
                .iter()
                .filter(|p| {
                    if let Some(t_mtime) = target_mtime {
                        match std::fs::metadata(p).ok().and_then(|m| m.modified().ok()) {
                            Some(p_mtime) => p_mtime > t_mtime,
                            None => true, // nonexistent prereq is always "newer"
                        }
                    } else {
                        true
                    }
                })
                .map(|s| s.as_str())
                .collect();
            newer.join(" ")
        };
        auto_vars.insert("?", newer_str);

        // $| = order-only prerequisites.
        auto_vars.insert("|", dedup_join(&order_only_v));

        // Directory variants
        auto_vars.insert(
            "@D",
            Path::new(target)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string()),
        );
        auto_vars.insert(
            "@F",
            Path::new(target)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default(),
        );

        // $(*D), $(*F)
        {
            let stem_val = auto_vars.get("*").cloned().unwrap_or_default();
            auto_vars.insert("*D", {
                let p = Path::new(&stem_val)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if p.is_empty() { ".".to_string() } else { p }
            });
            auto_vars.insert(
                "*F",
                Path::new(&stem_val)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
            );
        }

        // $(<D), $(<F)
        {
            let first_val = auto_vars.get("<").cloned().unwrap_or_default();
            auto_vars.insert("<D", {
                let p = Path::new(&first_val)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if p.is_empty() { ".".to_string() } else { p }
            });
            auto_vars.insert(
                "<F",
                Path::new(&first_val)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
            );
        }

        // D/F variants for list variables: $^, $+, $?, $|
        {
            fn dir_of(s: &str) -> String {
                let p = Path::new(s)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                if p.is_empty() { ".".to_string() } else { p }
            }
            fn file_of(s: &str) -> String {
                Path::new(s)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default()
            }
            for (var, d_var, f_var) in &[
                ("^", "^D", "^F"),
                ("+", "+D", "+F"),
                ("?", "?D", "?F"),
                ("|", "|D", "|F"),
            ] {
                let val = auto_vars.get(*var).cloned().unwrap_or_default();
                let dirs: Vec<String> = val.split_whitespace().map(dir_of).collect();
                let files: Vec<String> = val.split_whitespace().map(file_of).collect();
                auto_vars.insert(d_var, dirs.join(" "));
                auto_vars.insert(f_var, files.join(" "));
            }
        }

        // Set up environment for subprocesses
        let shell = self.lookup_var_or("SHELL", "/bin/sh");
        let shell_flags = self.lookup_var_or(".SHELLFLAGS", "-c");

        // Strip prefix chars (@, -, +) from the raw recipe line BEFORE
        // expansion — these apply to every sub-line produced by the
        // expansion. Prefix chars appearing within an expansion (e.g.
        // `$(V)echo hello` where `$(V)` is `@`) apply to that
        // sub-line only. This matches GNU make's behavior.
        let mut expanded_lines: Vec<(String, usize, bool, bool, bool)> = Vec::new();
        for (idx, line) in recipe.iter().enumerate() {
            let line_no = recipe_lines.get(idx).copied().unwrap_or(0);
            if line_no > 0 {
                *self.current_source.borrow_mut() = Some((recipe_source.to_string(), line_no));
            }
            let mut outer_silent = false;
            let mut outer_ignore = false;
            let mut outer_force = false;
            let mut raw = line.as_str();
            // In ONESHELL mode, only strip prefix chars from the first
            // recipe line. Subsequent lines pass @/-/+ through as content
            // (e.g. Perl's @array sigil).
            if !self.oneshell.get() || idx == 0 {
                loop {
                    raw = raw.trim_start();
                    if let Some(rest) = raw.strip_prefix('@') {
                        outer_silent = true;
                        raw = rest;
                    } else if let Some(rest) = raw.strip_prefix('-') {
                        outer_ignore = true;
                        raw = rest;
                    } else if let Some(rest) = raw.strip_prefix('+') {
                        outer_force = true;
                        raw = rest;
                    } else {
                        break;
                    }
                }
            }
            // Clear expand_chain_source before each recipe line so
            // direct function errors report the recipe line, not a
            // stale variable definition location.
            *self.expand_chain_source.borrow_mut() = None;
            // Detect $(MAKE) or ${MAKE} in the raw (unexpanded) line -- these
            // lines are "recursive" and must be executed even in -n mode.
            let has_make_ref = line.contains("$(MAKE)") || line.contains("${MAKE}");
            let force_execute = outer_force || has_make_ref;
            let expanded_raw = expand::expand_with_auto(raw, self, &auto_vars);
            // Split on newline, but keep `\<newline>` joined — the
            // backslash is an intentional shell line continuation that
            // was preserved from the makefile (so the shell itself sees
            // the joined command).
            let mut pieces: Vec<String> = Vec::new();
            let mut buf = String::new();
            let mut chars = expanded_raw.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' && chars.peek() == Some(&'\n') {
                    chars.next();
                    buf.push('\\');
                    buf.push('\n');
                } else if c == '\n' {
                    pieces.push(std::mem::take(&mut buf));
                } else {
                    buf.push(c);
                }
            }
            if !buf.is_empty() {
                pieces.push(buf);
            }
            for sub in pieces {
                // A piece is "effectively empty" if, after stripping
                // backslash-newline sequences (shell continuations) and
                // whitespace, nothing remains.
                let stripped = sub.replace("\\\n", " ");
                if !stripped.trim().is_empty() {
                    expanded_lines.push((sub, line_no, outer_silent, outer_ignore, force_execute));
                }
            }
        }
        *self.current_source.borrow_mut() = None;

        // `.ONESHELL`: combine all expanded recipe lines into a single
        // shell invocation. Only the first line's prefix flags (@/-/+)
        // control the whole combined script.
        // For Bourne-compatible shells (.SHELLFLAGS contains `-c`),
        // prefix chars on non-first lines are stripped from content.
        // For other shells (e.g. Perl), non-first lines pass through as-is.
        if self.oneshell.get() && expanded_lines.len() > 1 {
            let (_, first_line_no, first_silent, first_ignore, first_force) = &expanded_lines[0];
            let os_silent = *first_silent;
            let os_ignore = *first_ignore;
            let os_force = *first_force;
            let os_line_no = *first_line_no;

            // Bourne-compatible shells use `-c` in .SHELLFLAGS.
            let is_bourne = shell_flags.split_whitespace().any(|f| f == "-c");

            let mut combined_parts: Vec<String> = Vec::new();
            for (i, (raw, _ln, _s, _i, _f)) in expanded_lines.iter().enumerate() {
                let mut s = raw.as_str();
                // For Bourne shells, strip prefix chars from non-first
                // lines too. For non-Bourne shells (Perl, etc.), keep
                // non-first lines as-is since @/-/+ may be part of the
                // language syntax.
                if i > 0 && is_bourne {
                    loop {
                        s = s.trim_start();
                        if let Some(rest) = s.strip_prefix('@') {
                            s = rest;
                        } else if let Some(rest) = s.strip_prefix('-') {
                            s = rest;
                        } else if let Some(rest) = s.strip_prefix('+') {
                            s = rest;
                        } else {
                            break;
                        }
                    }
                }
                if !s.trim().is_empty() {
                    combined_parts.push(s.to_string());
                }
            }
            let combined = combined_parts.join("\n");
            expanded_lines = vec![(combined, os_line_no, os_silent, os_ignore, os_force)];
        }

        let target_is_silent =
            *self.silent_all.borrow() || self.silent_targets.borrow().contains(target);

        for (expanded_raw, line_no, outer_silent, outer_ignore, outer_force_exec) in &expanded_lines
        {
            let mut silent = self.silent || target_is_silent || *outer_silent;
            let mut ignore_error = *outer_ignore;
            let mut force_exec = *outer_force_exec;

            let mut expanded_str = expanded_raw.as_str();

            // Inner prefix chars from the expansion itself (still honor
            // them for the sub-line they belong to).
            loop {
                expanded_str = expanded_str.trim_start();
                if let Some(rest) = expanded_str.strip_prefix('@') {
                    silent = true;
                    expanded_str = rest;
                } else if let Some(rest) = expanded_str.strip_prefix('-') {
                    ignore_error = true;
                    expanded_str = rest;
                } else if let Some(rest) = expanded_str.strip_prefix('+') {
                    force_exec = true;
                    expanded_str = rest;
                } else {
                    break;
                }
            }

            let expanded = expanded_str.to_string();

            if expanded.trim().is_empty() {
                continue;
            }

            *self.recipe_executed.borrow_mut() = true;
            any_commands = true;

            // In dry-run mode (-n), GNU make ignores the `@` silence prefix
            // so every recipe line is echoed, even silenced ones.
            if !silent || self.dry_run || self.trace {
                println!("{expanded}");
            }

            if self.dry_run && !force_exec {
                continue;
            }

            // GNU make direct-execution optimization: when a recipe line
            // contains no shell metacharacters and the first word is not
            // a shell builtin, execute the command directly (bypassing
            // $SHELL). This lets libc's execvp() handle ENOEXEC (scripts
            // without a shebang) by automatically retrying with /bin/sh.
            let use_shell = {
                const SHELL_META: &[char] = &[
                    '#', ';', '"', '\'', '*', '?', '[', ']', '&', '|', '<', '>', '(', ')', '{',
                    '}', '$', '`', '^', '~', '!', '\\', '\n', '=',
                ];
                if expanded.contains(SHELL_META) {
                    true
                } else {
                    const SHELL_BUILTINS: &[&str] = &[
                        "cd", "eval", "exec", "exit", "login", "logout", "set", "umask", "wait",
                        "while", "for", "case", "if", ":", ".", "break", "continue", "export",
                        "read", "readonly", "shift", "times", "trap", "switch", "test",
                    ];
                    let first_word = expanded.split_whitespace().next().unwrap_or("");
                    SHELL_BUILTINS.contains(&first_word)
                }
            };

            // Pre-compute environment variables so they can be applied
            // to both a direct-execution attempt and a shell fallback.
            let mut env_set: Vec<(String, String)> = Vec::new();
            let mut env_remove: Vec<String> = Vec::new();
            if *self.export_all.borrow() {
                let unexports = self.unexports.borrow();
                for (name, var) in self.vars.borrow().iter() {
                    if unexports.contains(name) {
                        continue;
                    }
                    env_set.push((name.clone(), var.value.clone()));
                }
            } else {
                let shell_exported = self.exports.borrow().contains("SHELL");
                let unexports = self.unexports.borrow();
                for name in self.env_inherited.borrow().iter() {
                    if name == "SHELL" && !shell_exported {
                        continue;
                    }
                    if unexports.contains(name) {
                        env_remove.push(name.clone());
                        continue;
                    }
                    let value = self.lookup_var(name);
                    env_set.push((name.clone(), value));
                }
                for name in self.exports.borrow().iter() {
                    let value = self.lookup_var(name);
                    env_set.push((name.clone(), value));
                }
            }
            let current_level: i32 = self.lookup_var_or("MAKELEVEL", "0").parse().unwrap_or(0);
            env_set.push(("MAKELEVEL".into(), (current_level + 1).to_string()));
            env_remove.push("MAKE_RESTARTS".into());
            env_set.push(("MAKEFLAGS".into(), self.lookup_var("MAKEFLAGS")));
            env_set.push(("MAKE".into(), self.lookup_var("MAKE")));

            // Helper: apply pre-computed env to a Command.
            let apply_env = |cmd: &mut std::process::Command| {
                for (k, v) in &env_set {
                    cmd.env(k, v);
                }
                for k in &env_remove {
                    cmd.env_remove(k);
                }
            };

            // Helper: build the shell-based Command.
            let build_shell_cmd = |sh: &str, flags: &str, expanded: &str, ignore_error: bool| {
                let mut shell_parts = sh.split_whitespace();
                let shell_prog = shell_parts.next().unwrap_or("/bin/sh");
                let mut c = std::process::Command::new(shell_prog);
                for extra in shell_parts {
                    c.arg(extra);
                }
                let posix_relax =
                    ignore_error && matches!(self.var_origin(".SHELLFLAGS"), VarOrigin::Default);
                for flag in shell_split(flags) {
                    if posix_relax && flag.starts_with('-') && flag.contains('e') {
                        let stripped: String = flag.chars().filter(|ch| *ch != 'e').collect();
                        if stripped != "-" {
                            c.arg(stripped);
                        }
                    } else {
                        c.arg(flag);
                    }
                }
                c.arg(expanded);
                c
            };

            let status_result = if use_shell {
                let mut cmd = build_shell_cmd(&shell, &shell_flags, &expanded, ignore_error);
                apply_env(&mut cmd);
                cmd.status()
            } else {
                // No shell metacharacters — try direct execution.
                let parts: Vec<&str> = expanded.split_whitespace().collect();
                let mut cmd = std::process::Command::new(parts[0]);
                if parts.len() > 1 {
                    cmd.args(&parts[1..]);
                }
                apply_env(&mut cmd);
                match cmd.status() {
                    Err(ref e) if e.raw_os_error() == Some(8) => {
                        // ENOEXEC — script has no shebang. Fall back to
                        // /bin/sh -c, matching GNU make / POSIX behavior.
                        let mut sh = std::process::Command::new("/bin/sh");
                        sh.arg("-c");
                        sh.arg(&expanded);
                        apply_env(&mut sh);
                        sh.status()
                    }
                    Err(ref e) if e.raw_os_error() == Some(2) => {
                        // ENOENT — command not found.
                        let shell_prog = shell.split_whitespace().next().unwrap_or("/bin/sh");
                        let is_default_shell = shell_prog == "/bin/sh"
                            || shell_prog == "sh"
                            || shell_prog.ends_with("/sh");
                        if !is_default_shell {
                            // Custom shell: fall back to it.
                            let mut cmd =
                                build_shell_cmd(&shell, &shell_flags, &expanded, ignore_error);
                            apply_env(&mut cmd);
                            cmd.status()
                        } else {
                            // Default shell: report like GNU make.
                            let cmd_name = expanded.split_whitespace().next().unwrap_or(&expanded);
                            eprintln!("make: {cmd_name}: No such file or directory");
                            // Synthesize exit code 127 via a tiny shell process.
                            std::process::Command::new("/bin/sh")
                                .arg("-c")
                                .arg("exit 127")
                                .status()
                        }
                    }
                    Err(ref e) if e.raw_os_error() == Some(13) => {
                        // EACCES — permission denied (non-executable file, directory, etc.)
                        let cmd_name = expanded.split_whitespace().next().unwrap_or(&expanded);
                        eprintln!("make: {cmd_name}: Permission denied");
                        // Synthesize exit code 127, matching GNU make.
                        std::process::Command::new("/bin/sh")
                            .arg("-c")
                            .arg("exit 127")
                            .status()
                    }
                    other => other,
                }
            };

            // For diagnostics, include the source file:line of the failing
            // recipe line.
            let loc = if *line_no > 0 {
                format!("{recipe_source}:{line_no}: ")
            } else {
                String::new()
            };
            match status_result {
                Ok(status) => {
                    if !status.success() {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(sig) = status.signal() {
                            let sig_name = match sig {
                                1 => "Hangup",
                                2 => "Interrupt",
                                6 => "Aborted",
                                9 => "Killed",
                                11 => "Segmentation fault",
                                13 => "Broken pipe",
                                15 => "Terminated",
                                _ => "Unknown signal",
                            };
                            if ignore_error || self.ignore_errors {
                                eprintln!("make: [{loc}{target}] {sig_name} (ignored)");
                            } else {
                                *self.in_recipe.borrow_mut() = false;
                                return Err(format!("[{loc}{target}] {sig_name}"));
                            }
                        } else {
                            let code = status.code().unwrap_or(2);
                            if ignore_error || self.ignore_errors {
                                eprintln!("make: [{loc}{target}] Error {code} (ignored)");
                            } else {
                                *self.in_recipe.borrow_mut() = false;
                                return Err(format!("[{loc}{target}] Error {code}"));
                            }
                        }
                    }
                }
                Err(e) => {
                    if !ignore_error && !self.ignore_errors {
                        *self.in_recipe.borrow_mut() = false;
                        return Err(format!("{target}: {e}"));
                    }
                }
            }
        }

        *self.in_recipe.borrow_mut() = false;
        Ok(any_commands)
    }
}

/// Split a string like `/bin/sh` does for unquoted-yet-tokenized
/// contexts: whitespace separates tokens, but single- and
/// double-quoted substrings stay together with their quotes stripped.
/// Backslashes outside quotes escape the next char; inside quotes
/// they're preserved. Good enough for `.SHELLFLAGS`.
fn shell_split(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if in_token {
                    tokens.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            '\'' => {
                in_token = true;
                for nc in chars.by_ref() {
                    if nc == '\'' {
                        break;
                    }
                    cur.push(nc);
                }
            }
            '"' => {
                in_token = true;
                for nc in chars.by_ref() {
                    if nc == '"' {
                        break;
                    }
                    cur.push(nc);
                }
            }
            '\\' => {
                in_token = true;
                if let Some(nc) = chars.next() {
                    cur.push(nc);
                }
            }
            _ => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        tokens.push(cur);
    }
    tokens
}

/// Populate D (directory) and F (file) variants for the standard
/// automatic variables already present in `auto_vars`. Used by the
/// second-expansion code paths so prereq text like `$$(@D)` resolves
/// during SE expansion.
fn add_df_variants(auto_vars: &mut HashMap<&'static str, String>) {
    use std::path::Path;
    fn dir_of(s: &str) -> String {
        let p = Path::new(s)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if p.is_empty() { ".".to_string() } else { p }
    }
    fn file_of(s: &str) -> String {
        Path::new(s)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| s.to_string())
    }
    for (var, d_var, f_var) in [
        ("@", "@D", "@F"),
        ("*", "*D", "*F"),
        ("<", "<D", "<F"),
        ("^", "^D", "^F"),
        ("+", "+D", "+F"),
        ("?", "?D", "?F"),
        ("|", "|D", "|F"),
    ] {
        let val = auto_vars.get(var).cloned().unwrap_or_default();
        let dirs: Vec<String> = val.split_whitespace().map(dir_of).collect();
        let files: Vec<String> = val.split_whitespace().map(file_of).collect();
        auto_vars.insert(d_var, dirs.join(" "));
        auto_vars.insert(f_var, files.join(" "));
    }
}

fn dedup_join(items: &[String]) -> String {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for item in items {
        if seen.insert(item.as_str()) {
            result.push(item.as_str());
        }
    }
    result.join(" ")
}

/// Clean up Rust IO error messages to match GNU make's format.
/// e.g. "Permission denied (os error 13)" -> "Permission denied"
fn clean_io_error(msg: &str) -> String {
    if let Some(idx) = msg.find(" (os error ") {
        msg[..idx].to_string()
    } else {
        msg.to_string()
    }
}
