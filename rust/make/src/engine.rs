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
    target_vars: RefCell<HashMap<String, Vec<(String, AssignOp, String)>>>,
    /// Per-target variable names that were declared with `export`
    /// (e.g. `two: export SHELL := …`). Added to `exports` only
    /// while the owning target's recipe runs.
    target_exports: RefCell<HashMap<String, Vec<String>>>,
    /// Stack of target-specific variable bindings active during the
    /// currently-recursing build. Prereq builds see the union of their
    /// ancestors' bindings so target-specific vars propagate downward.
    target_scope_stack: RefCell<Vec<(String, AssignOp, String)>>,
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
    /// True when `.POSIX` special target has been seen.
    posix_mode: RefCell<bool>,
    /// Set to true by finalize_includes when an included makefile was
    /// remade and the process should re-exec itself.
    pub needs_reexec: Cell<bool>,
    /// Recursion depth counter for build_target_for. Prevents stack
    /// overflow when implicit rule search creates an infinite chain
    /// (e.g. %: %.c matching hello -> hello.c -> hello.c.c -> ...).
    build_depth: Cell<usize>,
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
            posix_mode: RefCell::new(false),
            needs_reexec: Cell::new(false),
            build_depth: Cell::new(0),
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
        // Linking
        self.add_pattern_rule(
            "%",
            &["%.o"],
            &["$(LINK.o) $^ $(LOADLIBES) $(LDLIBS) -o $@"],
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
    fn resolve_vpath(&self, name: &str) -> Option<String> {
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

            // Print "file not found" diagnostic before attempting build.
            // GNU make prints NSFD first, then any build errors. However,
            // only print it when:
            //   1. It's a non-silent include
            //   2. The file doesn't exist (not just unreadable)
            //   3. There IS a rule to try building it (otherwise phase 3
            //      handles the error for the no-rule case)
            //   4. The build will actually be attempted (not skipped)
            // Print NSFD before build only in keep-going mode, where the
            // ordering matters (build errors appear during build, and
            // NSFD must precede them). In non-keep-going mode, Phase 3
            // handles the error after we know the outcome.
            let printed_nsfd = if self.keep_going
                && !entry.silent
                && should_build
                && !std::path::Path::new(&entry.file).exists()
                && !entry.file_exists_unreadable
            {
                eprintln!(
                    "{}:{}: {}: No such file or directory",
                    entry.source, entry.line_no, entry.file
                );
                true
            } else {
                false
            };

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

            results.push(BuildResult {
                file: entry.file.clone(),
                silent: entry.silent,
                source: entry.source.clone(),
                line_no: entry.line_no,
                build_err,
                had_rule: should_build,
                skip_reason,
                printed_nsfd,
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
                // Build attempted but failed. Print NSFD only if not
                // already printed during Phase 2.
                if !res.printed_nsfd {
                    eprintln!("{}:{}: {}: {}", res.source, res.line_no, res.file, err_msg);
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
                // Parser prefixes the name with `!` when the
                // declaration had the `export` modifier; strip it
                // back off and record the var as a target-specific
                // export so the recipe's child env sees it when this
                // target builds.
                let (name, do_export) = match assign.name.strip_prefix('!') {
                    Some(rest) => (rest.to_string(), true),
                    None => (assign.name.clone(), false),
                };
                let mut tv = self.target_vars.borrow_mut();
                for target in targets_expanded.split_whitespace() {
                    tv.entry(target.to_string()).or_default().push((
                        name.clone(),
                        assign.op,
                        value.clone(),
                    ));
                }
                if do_export {
                    let mut te = self.target_exports.borrow_mut();
                    for target in targets_expanded.split_whitespace() {
                        te.entry(target.to_string()).or_default().push(name.clone());
                    }
                }
            }
            Directive::Vpath(_) => {
                // TODO: VPATH support
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
        // Expand targets and prerequisites
        let targets: Vec<String> = rule
            .targets
            .iter()
            .flat_map(|t| {
                expand::expand(t, self)
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .collect();

        // Join prereqs before expansion so `$<space>` and similar
        // single-char references span what the parser split apart.
        // A `|` in the expansion output (e.g. from
        // `$(var)` where var contains a pipe) splits normal from
        // order-only prereqs — GNU make re-parses the expanded text.
        let prereq_text = expand::expand(&rule.prerequisites.join(" "), self);
        let extra_order_only_text = expand::expand(&rule.order_only.join(" "), self);
        let (prereq_text, post_pipe_order_only) = if let Some(idx) = prereq_text.find('|') {
            (
                prereq_text[..idx].to_string(),
                prereq_text[idx + 1..].to_string(),
            )
        } else {
            (prereq_text, String::new())
        };
        let prereqs: Vec<String> = prereq_text
            .split_whitespace()
            .map(|s| self.resolve_library_prereq(s))
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
            eprintln!("{src}:{line}: *** prerequisites cannot be defined in recipes.  Stop.");
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
                ".DELETE_ON_ERROR" => {
                    *self.delete_on_error.borrow_mut() = true;
                }
                ".PRECIOUS"
                | ".INTERMEDIATE"
                | ".SECONDARY"
                | ".IGNORE"
                | ".EXPORT_ALL_VARIABLES"
                | ".NOTPARALLEL"
                | ".ONESHELL" => {
                    if target == ".EXPORT_ALL_VARIABLES" {
                        *self.export_all.borrow_mut() = true;
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
            && !targets.iter().any(|t| t.contains('%'))
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
            for target in &targets {
                let stem = match expand::pattern_stem(target, &target_pattern) {
                    Some(s) => s,
                    None => {
                        eprintln!("make: target '{target}' doesn't match the target pattern");
                        continue;
                    }
                };
                let resolved_prereqs: Vec<String> = expanded_prereq_patterns
                    .iter()
                    .map(|p| p.replace('%', &stem))
                    .collect();
                let mut rules = self.rules.borrow_mut();
                rules.entry(target.clone()).or_default().push(RuleEntry {
                    prerequisites: resolved_prereqs,
                    order_only: order_only.clone(),
                    recipe: rule.recipe.clone(),
                    recipe_lines: rule.recipe_lines.clone(),
                    source_name: rule.source_name.clone(),
                    is_double_colon: rule.is_double_colon,
                    group: Vec::new(),
                });
            }
            if !*self.suppress_default_goal.borrow() {
                let mut default = self.default_goal.borrow_mut();
                if default.is_none()
                    && let Some(t) = targets.iter().find(|t| !t.starts_with('.'))
                {
                    *default = Some(t.clone());
                }
            }
            *self.last_rule_targets.borrow_mut() = Some(targets.clone());
            return;
        }

        // Pattern rule
        if let Some(pattern) = &rule.pattern {
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
                    if !t.starts_with('.') {
                        *default = Some(t.clone());
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
        if should_print && restarts == 0 {
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
                    let parts: Vec<String> =
                        expanded.split_whitespace().map(|s| s.to_string()).collect();
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

        let mut had_error = false;
        for target in &targets {
            let target_expanded = expand::expand(target, self);
            *self.recipe_executed.borrow_mut() = false;
            *self.target_had_recipe.borrow_mut() = true;
            let before_built = self.built_targets.borrow().contains(&target_expanded);
            let is_group_built = self.group_built_targets.borrow().contains(&target_expanded);
            match self.build_target(&target_expanded) {
                Ok(()) => {
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

    /// Gather target-specific variables for a target *and* those inherited
    /// from any ancestors currently being built. GNU make propagates
    /// target-specific vars from a parent to all of its prereqs.
    fn collect_target_vars(&self, target: &str) -> Vec<(String, AssignOp, String)> {
        let mut result: Vec<(String, AssignOp, String)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        // Inherited vars from parent scopes (innermost last, already merged).
        let parent_scope = self.target_scope_stack.borrow();
        for (n, op, v) in parent_scope.iter() {
            if seen.insert(n.clone()) {
                result.push((n.clone(), *op, v.clone()));
            }
        }
        // Target's own vars (take precedence — replace any inherited).
        let tv = self.target_vars.borrow();
        if let Some(entries) = tv.get(target) {
            for (n, op, v) in entries {
                // Remove any inherited entry with same name.
                result.retain(|(existing, _, _)| existing != n);
                result.push((n.clone(), *op, v.clone()));
            }
        }
        result
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

        let is_phony = self.phony_targets.borrow().contains(target);

        // Find explicit rules
        let rules = self.rules.borrow().get(target).cloned().unwrap_or_default();

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
                let prereqs: Vec<String> = rule
                    .prerequisites
                    .iter()
                    .flat_map(|p| {
                        expand::expand(p, self)
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let order_only: Vec<String> = rule
                    .order_only
                    .iter()
                    .flat_map(|p| {
                        expand::expand(p, self)
                            .split_whitespace()
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                for prereq in &prereqs {
                    if prereq == target {
                        eprintln!(
                            "make: Circular {} <- {} dependency dropped.",
                            target, prereq
                        );
                        continue;
                    }
                    self.build_target_for(prereq, Some(target))?;
                }
                for prereq in &order_only {
                    self.build_target_for(prereq, Some(target))?;
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
                        "",
                    ) {
                        Err(e) => return Err(e),
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
            return Ok(());
        }

        // Find matching pattern rule if no explicit recipe
        let has_recipe = rules.iter().any(|r| !r.recipe.is_empty());
        let pattern_match = if !has_recipe {
            self.find_pattern_rule(target)
        } else {
            None
        };

        // For non-double-colon grouped targets, if the group's recipe
        // was already executed by a sibling, mark this target as built
        // and skip re-execution.
        for rule in &rules {
            if !rule.group.is_empty() {
                let key = rule.group.join("\0");
                if self.group_recipe_done.borrow().contains(&key) {
                    *self.target_had_recipe.borrow_mut() = false;
                    self.built_targets.borrow_mut().insert(target.to_string());
                    return Ok(());
                }
            }
        }

        // Collect all prerequisites
        let mut all_prereqs: Vec<String> = Vec::new();
        let mut all_order_only: Vec<String> = Vec::new();
        let mut recipe: Vec<String> = Vec::new();
        let mut recipe_lines: Vec<usize> = Vec::new();
        let mut recipe_source: String = String::new();
        let mut stem = String::new();
        // Track pattern-implied prerequisites for $< resolution
        let mut implied_prereqs: Vec<String> = Vec::new();

        for rule in &rules {
            all_prereqs.extend(rule.prerequisites.iter().map(|p| expand::expand(p, self)));
            all_order_only.extend(rule.order_only.iter().map(|p| expand::expand(p, self)));
            if recipe.is_empty() && !rule.recipe.is_empty() {
                recipe_lines = rule.recipe_lines.clone();
                recipe_source = rule.source_name.clone();
                recipe = rule.recipe.clone();
            }
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
        if let Some((pat_rule, pat_stem)) = &pattern_match {
            stem = pat_stem.clone();
            for pp in &pat_rule.prereq_patterns {
                let prereq = pp.replace('%', &stem);
                if pp.contains('%') {
                    pattern_derived_prereqs.insert(prereq.clone());
                }
                implied_prereqs.push(prereq.clone());
                all_prereqs.push(prereq);
            }
            for op in &pat_rule.order_only_patterns {
                let oo = expand::expand(&op.replace('%', &stem), self);
                for tok in oo.split_whitespace() {
                    all_order_only.push(tok.to_string());
                }
            }
            if recipe.is_empty() {
                recipe = pat_rule.recipe.clone();
                recipe_lines = pat_rule.recipe_lines.clone();
                recipe_source = pat_rule.source_name.clone();
            }
        }
        // Re-apply the normal-vs-order-only promotion after pattern
        // match injected new entries.
        let normal_set2: HashSet<String> = all_prereqs.iter().cloned().collect();
        all_order_only.retain(|o| !normal_set2.contains(o));

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

        // Push this target's own variable bindings onto the scope stack
        // so prereq builds inherit them (GNU make semantics). Popped
        // before any return path via `pop_scope`.
        let own_vars: Vec<(String, AssignOp, String)> = self
            .target_vars
            .borrow()
            .get(target)
            .cloned()
            .unwrap_or_default();
        let scope_push_count = own_vars.len();
        self.target_scope_stack.borrow_mut().extend(own_vars);
        let pop_scope = |s: &Self| {
            let mut stack = s.target_scope_stack.borrow_mut();
            let new_len = stack.len().saturating_sub(scope_push_count);
            stack.truncate(new_len);
        };

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
                .and_then(|entries| entries.iter().rev().find(|(n, _, _)| n == ".EXTRA_PREREQS"))
                .map(|(_, _, v)| expand::expand(v, self))
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
        let mut first_err: Option<String> = None;
        for prereq in &all_prereqs {
            if let Err(e) = self.build_target_for(prereq, Some(target)) {
                if self.keep_going {
                    // Emit each diagnostic now so -k runs can see all
                    // of them, then remember the first error to bubble
                    // up after the loop.
                    if !e.is_empty() {
                        if e.starts_with('[') {
                            eprintln!("make: *** {e}");
                        } else {
                            eprintln!("make: *** {e}.");
                        }
                    }
                    if first_err.is_none() {
                        first_err = Some(String::new());
                    }
                    continue;
                }
                pop_scope(self);
                return Err(e);
            }
            if self.rebuilt_targets.borrow().contains(prereq) {
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
                // Prereq has a rule but no resulting file — phony / FORCE.
                has_phony_prereq = true;
            } else {
                // Prereq was built (e.g. by a pattern rule) but no file
                // exists.  If the prereq is NOT “intermediate” (i.e. it
                // was not derived purely from a %-pattern, or it is
                // explicitly mentioned elsewhere in the makefile), then
                // the missing file triggers a rebuild of the parent.
                let is_intermediate =
                    pattern_derived_prereqs.contains(prereq) && !self.is_mentioned_file(prereq);
                if !is_intermediate {
                    has_phony_prereq = true;
                }
            }
        }

        // If -k mode collected any prereq failures above, surface an
        // empty error now so the outer driver knows this target didn't
        // remake. Skip the remaining prereq work for this target.
        if first_err.is_some() {
            self.failed_targets.borrow_mut().insert(target.to_string());
            pop_scope(self);
            return Err(String::new());
        }

        // Build .EXTRA_PREREQS after normal prereqs but before
        // order-only, and keep them out of `$^`/`$<`.
        if !in_extras {
            for prereq in &extra_prereqs {
                if let Err(e) = self.build_target_for(prereq, Some(target)) {
                    if self.keep_going {
                        if !e.is_empty() {
                            if e.starts_with('[') {
                                eprintln!("make: *** {e}");
                            } else {
                                eprintln!("make: *** {e}.");
                            }
                        }
                        first_err = Some(String::new());
                        continue;
                    }
                    pop_scope(self);
                    return Err(e);
                }
            }
            if first_err.is_some() {
                self.failed_targets.borrow_mut().insert(target.to_string());
                pop_scope(self);
                return Err(String::new());
            }
        }

        // Build order-only prerequisites after normal prereqs.
        // Order-only targets just need to exist — they don't affect
        // the target's rebuild decision based on mtime.
        for prereq in &all_order_only {
            if let Err(e) = self.build_target_for(prereq, Some(target)) {
                pop_scope(self);
                return Err(e);
            }
        }

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
                !expand::expand(s, self).trim().is_empty()
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
                    if *self.delete_on_error.borrow() && !is_phony && Path::new(target).exists() {
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
                                let sibling = sibling_pat.replace('%', &stem);
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
                            let sibling = sibling_pat.replace('%', &stem);
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
                self.group_recipe_done.borrow_mut().insert(key);
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
        // Multi-target pattern rules: mark sibling targets (other
        // target patterns with the same stem) as built.
        if let Some((pat_rule, _)) = &pattern_match
            && !pat_rule.sibling_patterns.is_empty()
        {
            for sibling_pat in &pat_rule.sibling_patterns {
                let sibling = sibling_pat.replace('%', &stem);
                self.built_targets.borrow_mut().insert(sibling.clone());
                self.rebuilt_targets.borrow_mut().insert(sibling.clone());
            }
        }
        pop_scope(self);
        Ok(())
    }

    /// Resolve `-l<name>` prereqs using .LIBPATTERNS. Each `%` pattern is
    /// tried; the first that matches an existing file is used. If none
    /// match, fall back to the first pattern expansion (GNU make uses
    /// this for the `$<` / recipe view).
    fn resolve_library_prereq(&self, name: &str) -> String {
        let Some(lib) = name.strip_prefix("-l") else {
            return name.to_string();
        };
        let patterns = self.lookup_var(".LIBPATTERNS");
        let mut first: Option<String> = None;
        for pat in patterns.split_whitespace() {
            if !pat.contains('%') {
                eprintln!("make: .LIBPATTERNS element '{pat}' is not a pattern");
                continue;
            }
            let candidate = pat.replace('%', lib);
            if first.is_none() {
                first = Some(candidate.clone());
            }
            if std::path::Path::new(&candidate).exists()
                || self.rules.borrow().contains_key(&candidate)
            {
                return candidate;
            }
        }
        first.unwrap_or_else(|| name.to_string())
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

    fn find_pattern_rule(&self, target: &str) -> Option<(PatternRuleEntry, String)> {
        self.find_pattern_rule_inner(target, 0)
    }

    /// Depth-limited implicit rule search. Recursively checks whether a
    /// pattern rule's prerequisites can be satisfied (exist on disk, have
    /// an explicit rule, or can be built by further implicit rule
    /// chaining up to MAX_IMPLICIT_CHAIN_DEPTH).
    fn find_pattern_rule_inner(
        &self,
        target: &str,
        depth: usize,
    ) -> Option<(PatternRuleEntry, String)> {
        const MAX_IMPLICIT_CHAIN_DEPTH: usize = 3;
        if depth >= MAX_IMPLICIT_CHAIN_DEPTH {
            return None;
        }
        let suffixes_cleared = *self.suffixes_cleared.borrow();
        let pattern_rules = self.pattern_rules.borrow();
        for rule in pattern_rules.iter().rev() {
            if suffixes_cleared && rule.source_name == "<built-in>" {
                continue;
            }
            if let Some(stem) = expand::pattern_stem(target, &rule.target_pattern) {
                if rule.recipe.is_empty() {
                    continue;
                }
                if rule.is_terminal {
                    let prereqs_ok = rule.prereq_patterns.is_empty()
                        || rule.prereq_patterns.iter().all(|pp| {
                            let prereq = pp.replace('%', &stem);
                            Path::new(&prereq).exists()
                        });
                    if prereqs_ok {
                        return Some((rule.clone(), stem));
                    }
                    continue;
                }
                // Non-terminal rule: check that ALL prereqs exist or
                // can be built (via explicit rule, "ought to exist"
                // mention, or implicit rule chaining with bounded
                // depth).
                let prereqs_ok = rule.prereq_patterns.is_empty()
                    || rule.prereq_patterns.iter().all(|pp| {
                        let prereq = pp.replace('%', &stem);
                        Path::new(&prereq).exists()
                            || self.is_mentioned_file(&prereq)
                            || self.phony_targets.borrow().contains(&prereq)
                            || self.rules.borrow().contains_key(prereq.as_str())
                            || self.find_pattern_rule_inner(&prereq, depth + 1).is_some()
                    });
                if prereqs_ok {
                    return Some((rule.clone(), stem));
                }
            }
        }
        None
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
        let tv_entries: Vec<(String, AssignOp, String)> = self.collect_target_vars(target);
        // Target-specific `export VAR := …` — add VAR to the export
        // set for the duration of this recipe.
        let export_names: Vec<String> = self
            .target_exports
            .borrow()
            .get(target)
            .cloned()
            .unwrap_or_default();
        let mut added_exports: Vec<String> = Vec::new();
        {
            let mut exports = self.exports.borrow_mut();
            for n in &export_names {
                if exports.insert(n.clone()) {
                    added_exports.push(n.clone());
                }
            }
        }
        let mut saved: Vec<(String, Option<Variable>)> = Vec::new();
        for (name, op, value) in &tv_entries {
            saved.push((name.clone(), self.vars.borrow().get(name).cloned()));
            let flavor = match op {
                AssignOp::Simple | AssignOp::Shell => VarFlavor::Simple,
                _ => VarFlavor::Recursive,
            };
            let final_value = if matches!(op, AssignOp::Simple) {
                expand::expand(value, self)
            } else {
                value.clone()
            };
            // Store directly, bypassing origin precedence — target-specific
            // assignments win within the target's scope.
            self.vars.borrow_mut().insert(
                name.clone(),
                Variable {
                    value: final_value,
                    flavor,
                    origin: VarOrigin::Automatic,
                },
            );
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
                        std::fs::metadata(p)
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .is_some_and(|p_mtime| p_mtime > t_mtime)
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
                if !sub.is_empty() {
                    expanded_lines.push((sub, line_no, outer_silent, outer_ignore, force_execute));
                }
            }
        }
        *self.current_source.borrow_mut() = None;

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

            // Export variables
            // SHELL may itself contain arguments (e.g. `SHELL := echo hi`);
            // GNU make splits on whitespace, using the first word as
            // the program and the rest as leading arguments — so a
            // target-specific `SHELL := echo hi` with `.SHELLFLAGS :=
            // ho ho` runs `echo hi ho ho <recipe>`.
            let mut shell_parts = shell.split_whitespace();
            let shell_prog = shell_parts.next().unwrap_or("/bin/sh");
            let mut cmd = std::process::Command::new(shell_prog);
            for extra in shell_parts {
                cmd.arg(extra);
            }
            // Split .SHELLFLAGS on whitespace, honoring single- and
            // double-quoted sections as single tokens (and stripping
            // the quotes) — matches GNU make's shell-style split for
            // flags like `'ho;ho'`. When this line has the
            // ignore-errors prefix (`-`) AND the `-ec` came from
            // implicit `.POSIX:` handling (origin Default), drop any
            // `-e` so a `;`-separated command chain keeps running
            // past a failing first command. If the user explicitly
            // assigned `.SHELLFLAGS` to include `-e`, we respect it
            // verbatim.
            let posix_relax =
                ignore_error && matches!(self.var_origin(".SHELLFLAGS"), VarOrigin::Default);
            for flag in shell_split(&shell_flags) {
                if posix_relax && flag.starts_with('-') && flag.contains('e') {
                    let stripped: String = flag.chars().filter(|c| *c != 'e').collect();
                    if stripped != "-" {
                        cmd.arg(stripped);
                    }
                } else {
                    cmd.arg(flag);
                }
            }
            cmd.arg(&expanded);

            if *self.export_all.borrow() {
                let unexports = self.unexports.borrow();
                for (name, var) in self.vars.borrow().iter() {
                    if unexports.contains(name) {
                        continue;
                    }
                    cmd.env(name, &var.value);
                }
            } else {
                // Re-export any variable the make process inherited
                // from its environment — even if a makefile assignment
                // later changed the value (and thus the origin). The
                // child would otherwise inherit our stale env entry.
                // SHELL is special: `SHELL := …` changes the shell
                // make uses internally but must not be exported to
                // recipes (keeps the user's login shell visible there)
                // — unless SHELL has been explicitly `export`ed.
                let shell_exported = self.exports.borrow().contains("SHELL");
                let unexports = self.unexports.borrow();
                for name in self.env_inherited.borrow().iter() {
                    if name == "SHELL" && !shell_exported {
                        continue;
                    }
                    if unexports.contains(name) {
                        cmd.env_remove(name);
                        continue;
                    }
                    let value = self.lookup_var(name);
                    cmd.env(name, &value);
                }
                for name in self.exports.borrow().iter() {
                    let value = self.lookup_var(name);
                    cmd.env(name, &value);
                }
            }
            // Increment MAKELEVEL in the child environment so sub-makes
            // see a bumped value, matching GNU make.
            let current_level: i32 = self.lookup_var_or("MAKELEVEL", "0").parse().unwrap_or(0);
            cmd.env("MAKELEVEL", (current_level + 1).to_string());
            // Always propagate MAKEFLAGS and MAKE to children so recipes
            // can `$(MAKE)` recursively and see flag state.
            cmd.env("MAKEFLAGS", self.lookup_var("MAKEFLAGS"));
            cmd.env("MAKE", self.lookup_var("MAKE"));

            // For diagnostics, include the source file:line of the failing
            // recipe line.
            let loc = if *line_no > 0 {
                format!("{recipe_source}:{line_no}: ")
            } else {
                String::new()
            };
            match cmd.status() {
                Ok(status) => {
                    if !status.success() {
                        let code = status.code().unwrap_or(2);
                        if ignore_error || self.ignore_errors {
                            // Emit GNU make's "(ignored)" diagnostic when a
                            // recipe with `-` prefix (or `-i`) silently
                            // swallows the failure.
                            eprintln!("make: [{loc}{target}] Error {code} (ignored)");
                        } else {
                            {
                                *self.in_recipe.borrow_mut() = false;
                                return Err(format!("[{loc}{target}] Error {code}"));
                            }
                        }
                    }
                }
                Err(e) => {
                    if !ignore_error && !self.ignore_errors {
                        {
                            *self.in_recipe.borrow_mut() = false;
                            return Err(format!("{target}: {e}"));
                        }
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
