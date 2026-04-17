//! Make execution engine: variable storage, rule database, and build logic.

use crate::ast::*;
use crate::expand;
use std::cell::RefCell;
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
}

/// A pattern rule entry.
#[derive(Debug, Clone)]
struct PatternRuleEntry {
    target_pattern: String,
    prereq_patterns: Vec<String>,
    recipe: Vec<String>,
    recipe_lines: Vec<usize>,
    source_name: String,
}

pub struct Engine {
    pub vars: RefCell<HashMap<String, Variable>>,
    rules: RefCell<HashMap<String, Vec<RuleEntry>>>,
    pattern_rules: RefCell<Vec<PatternRuleEntry>>,
    default_goal: RefCell<Option<String>>,
    phony_targets: RefCell<HashSet<String>>,
    suffixes: RefCell<Vec<String>>,
    exports: RefCell<HashSet<String>>,
    export_all: RefCell<bool>,
    built_targets: RefCell<HashSet<String>>,
    eval_queue: RefCell<Vec<String>>,
    /// Includes that failed to load (file missing). Retried after the
    /// top-level parse completes so rules defined later in the same
    /// makefile can build the missing include.
    pending_includes: RefCell<Vec<(String, bool)>>,
    /// Names of variables originally inherited from the environment.
    /// We track these separately from `VarOrigin` so we can re-export
    /// them to child processes even after a makefile assignment has
    /// changed their value (and consequently their origin).
    env_inherited: RefCell<HashSet<String>>,
    // Options
    pub jobs: usize,
    pub keep_going: bool,
    pub dry_run: bool,
    pub silent: bool,
    pub touch: bool,
    pub question: bool,
    pub always_make: bool,
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
            export_all: RefCell::new(false),
            built_targets: RefCell::new(HashSet::new()),
            eval_queue: RefCell::new(Vec::new()),
            pending_includes: RefCell::new(Vec::new()),
            env_inherited: RefCell::new(HashSet::new()),
            jobs: 1,
            keep_going: false,
            dry_run: false,
            silent: false,
            touch: false,
            question: false,
            always_make: false,
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
            target_scope_stack: RefCell::new(Vec::new()),
            delete_on_error: RefCell::new(false),
            assume_new: RefCell::new(HashSet::new()),
            rebuilt_targets: RefCell::new(HashSet::new()),
            suppress_default_goal: RefCell::new(false),
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

    fn setup_default_rules(&self) {
        // C compilation
        self.add_pattern_rule(
            "%.o",
            &["%.c"],
            &["$(CC) $(CPPFLAGS) $(CFLAGS) -c -o $@ $<"],
        );
        // C++ compilation
        self.add_pattern_rule(
            "%.o",
            &["%.cpp"],
            &["$(CXX) $(CPPFLAGS) $(CXXFLAGS) -c -o $@ $<"],
        );
        self.add_pattern_rule(
            "%.o",
            &["%.cc"],
            &["$(CXX) $(CPPFLAGS) $(CXXFLAGS) -c -o $@ $<"],
        );
        // Assembly
        self.add_pattern_rule("%.o", &["%.s"], &["$(AS) $(ASFLAGS) -o $@ $<"]);
        // Linking (implicit rule for executables)
        self.add_pattern_rule("%", &["%.o"], &["$(CC) $(LDFLAGS) -o $@ $^ $(LDLIBS)"]);

        // Default CC/CXX
        self.set_var_default("CC", "cc");
        self.set_var_default("CXX", "c++");
        self.set_var_default("AS", "as");
        self.set_var_default("AR", "ar");
        self.set_var_default("RM", "rm -f");
        self.set_var_default("CFLAGS", "");
        self.set_var_default("CXXFLAGS", "");
        self.set_var_default("CPPFLAGS", "");
        self.set_var_default("LDFLAGS", "");
        self.set_var_default("LDLIBS", "");
        self.set_var_default("ARFLAGS", "rv");
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
            recipe: recipe.iter().map(|s| s.to_string()).collect(),
            recipe_lines: vec![0; recipe.len()],
            source_name: "<built-in>".to_string(),
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
                VarFlavor::Recursive => expand::expand_with_auto(&value, self, auto_vars),
                VarFlavor::Simple => value,
                VarFlavor::Undefined => String::new(),
            }
        } else {
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

    /// After the top-level parse, try to build any deferred include files
    /// whose build rules are now known. If a rule exists (explicit or
    /// pattern), run it and reload the file; otherwise emit the standard
    /// missing-include diagnostic unless the include was silent.
    pub fn finalize_includes(&self) {
        let pending: Vec<(String, bool)> = self.pending_includes.borrow_mut().drain(..).collect();
        for (file, silent) in pending {
            // Search via `load_file` which consults `-I` directories too.
            if self.resolve_include_path(&file).is_some() {
                self.load_file(&file, silent);
                continue;
            }
            let has_rule =
                self.rules.borrow().contains_key(&file) || self.find_pattern_rule(&file).is_some();
            if has_rule {
                let _ = self.build_target(&file);
            }
            if self.resolve_include_path(&file).is_some() {
                self.load_file(&file, silent);
            } else if !silent {
                eprintln!("make: {}: No such file or directory", file);
            }
        }
    }

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
            Directive::Assignment(assign) => {
                self.process_assignment(assign, VarOrigin::File);
            }
            Directive::Rule(rule) => {
                self.process_rule(rule);
            }
            Directive::Include(files, silent) => {
                for file_pattern in files {
                    let expanded = expand::expand(file_pattern, self);
                    for file in expanded.split_whitespace() {
                        let mut matched = false;
                        if let Ok(paths) = glob::glob(file) {
                            for entry in paths.flatten() {
                                let path = entry.to_string_lossy().to_string();
                                self.load_file(&path, *silent);
                                matched = true;
                            }
                        }
                        if !matched {
                            // Defer: a rule to build this file may appear
                            // later in the same makefile. `finalize_includes`
                            // retries after the full parse.
                            self.pending_includes
                                .borrow_mut()
                                .push((file.to_string(), *silent));
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
                    for name in names {
                        let expanded = expand::expand(name, self);
                        for word in expanded.split_whitespace() {
                            exports.remove(word);
                        }
                    }
                } else {
                    *self.export_all.borrow_mut() = false;
                }
            }
            Directive::Override(assign) => {
                self.process_assignment(assign, VarOrigin::Override);
            }
            Directive::Define(name, op, lines) => {
                let expanded_name = expand::expand(name, self);
                let body = lines.join("\n");
                self.apply_define(&expanded_name, *op, &body, VarOrigin::File);
            }
            Directive::OverrideDefine(name, op, lines) => {
                let expanded_name = expand::expand(name, self);
                let body = lines.join("\n");
                self.apply_define(&expanded_name, *op, &body, VarOrigin::Override);
            }
            Directive::Undefine(name) => {
                // `undefine` from a makefile doesn't clobber command-line
                // or override variables; those still win.
                let expanded = expand::expand(name, self);
                let mut vars = self.vars.borrow_mut();
                let remove = matches!(
                    vars.get(&expanded).map(|v| v.origin),
                    Some(VarOrigin::File) | Some(VarOrigin::Environment) | Some(VarOrigin::Default)
                );
                if remove {
                    vars.remove(&expanded);
                }
            }
            Directive::OverrideUndefine(name) => {
                // `override undefine VAR` force-removes the variable
                // regardless of origin.
                let expanded = expand::expand(name, self);
                self.vars.borrow_mut().remove(&expanded);
            }
            Directive::TargetVarAssign(targets_str, assign) => {
                let targets_expanded = expand::expand(targets_str, self);
                let mut tv = self.target_vars.borrow_mut();
                for target in targets_expanded.split_whitespace() {
                    tv.entry(target.to_string()).or_default().push((
                        assign.name.clone(),
                        assign.op,
                        assign.value.clone(),
                    ));
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
        }
    }

    fn process_assignment(&self, assign: &Assignment, origin: VarOrigin) {
        let name = expand::expand(&assign.name, self);

        match assign.op {
            AssignOp::Simple => {
                let value = expand::expand(&assign.value, self);
                self.set_var_with_origin(&name, &value, VarFlavor::Simple, origin);
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
                let rhs = if existing_flavor == VarFlavor::Simple {
                    expand::expand(&assign.value, self)
                } else {
                    assign.value.clone()
                };
                let new_value = if existing.is_empty() {
                    rhs
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
        let prereqs: Vec<String> = expand::expand(&rule.prerequisites.join(" "), self)
            .split_whitespace()
            .map(|s| self.resolve_library_prereq(s))
            .collect();

        let order_only: Vec<String> = expand::expand(&rule.order_only.join(" "), self)
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

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
                    // recipe commands stop on first failure.
                    self.set_var_with_origin(
                        ".SHELLFLAGS",
                        "-ec",
                        VarFlavor::Simple,
                        VarOrigin::Default,
                    );
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
        // A target like ".XY" where .X and .Y are known suffixes
        if targets.len() == 1 && prereqs.is_empty() && targets[0].starts_with('.') {
            let target = &targets[0];
            if let Some((src_suffix, dst_suffix)) = self.parse_suffix_rule(target) {
                self.pattern_rules.borrow_mut().push(PatternRuleEntry {
                    target_pattern: format!("%{dst_suffix}"),
                    prereq_patterns: vec![format!("%{src_suffix}")],
                    recipe: rule.recipe.clone(),
                    recipe_lines: rule.recipe_lines.clone(),
                    source_name: rule.source_name.clone(),
                });
                return;
            }
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
                    recipe: rule.recipe.clone(),
                    recipe_lines: rule.recipe_lines.clone(),
                    source_name: rule.source_name.clone(),
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

        // Store explicit rules
        let entry = RuleEntry {
            prerequisites: prereqs,
            order_only,
            recipe: rule.recipe.clone(),
            recipe_lines: rule.recipe_lines.clone(),
            source_name: rule.source_name.clone(),
            is_double_colon: rule.is_double_colon,
        };

        for target in &targets {
            self.rules
                .borrow_mut()
                .entry(target.clone())
                .or_default()
                .push(entry.clone());
        }
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
                let mut parser = crate::parser::Parser::new_with_source(&content, resolved.clone());
                match parser.parse() {
                    Ok(directives) => self.load_makefile(&directives),
                    Err(e) => {
                        if !silent {
                            eprintln!("{}: {}", resolved, e);
                        }
                    }
                }
            }
            Err(e) => {
                if !silent {
                    eprintln!("make: {}: {}", resolved, e);
                }
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
        if should_print {
            println!("{make_tag}: Entering directory '{}'", cwd_for_msg.display());
        }
        self.finalize_includes();

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
            match self.build_target(&target_expanded) {
                Ok(()) => {
                    // If build_target returned Ok without running any recipe,
                    // emit GNU make's diagnostic. Not under -s/-q.
                    if !*self.recipe_executed.borrow()
                        && !before_built
                        && !self.question
                        && !self.silent
                        && !had_error
                    {
                        if *self.target_had_recipe.borrow() {
                            println!("{make_tag}: '{target_expanded}' is up to date.");
                        } else {
                            println!("{make_tag}: Nothing to be done for '{target_expanded}'.");
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

        let is_phony = self.phony_targets.borrow().contains(target);

        // Find explicit rules
        let rules = self.rules.borrow().get(target).cloned().unwrap_or_default();

        // Double-colon rules are independent: each rule is evaluated
        // separately, with its own prereqs and its own recipe. Fall
        // through to the normal path only if all rules are single-colon.
        let any_double = rules.iter().any(|r| r.is_double_colon);
        if any_double {
            self.built_targets.borrow_mut().insert(target.to_string());
            for (idx, rule) in rules.iter().enumerate() {
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
                    self.build_target_for(prereq, Some(target))?;
                }
                for prereq in &order_only {
                    self.build_target_for(prereq, Some(target))?;
                }
                if rule.recipe.is_empty() {
                    continue;
                }
                // Mimic the single-rule rebuild decision per entry.
                let target_mtime = if is_phony {
                    None
                } else {
                    std::fs::metadata(target)
                        .ok()
                        .and_then(|m| m.modified().ok())
                };
                let mut needs = self.always_make || is_phony || target_mtime.is_none();
                if !needs {
                    for p in &prereqs {
                        let pm = std::fs::metadata(p).ok().and_then(|m| m.modified().ok());
                        if pm.is_none() {
                            needs = true;
                            break;
                        }
                        if let (Some(t), Some(pt)) = (target_mtime, pm)
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
                self.rebuilt_targets.borrow_mut().insert(target.to_string());
                if self.touch {
                    if !is_phony {
                        if !self.silent {
                            println!("touch {target}");
                        }
                        *self.recipe_executed.borrow_mut() = true;
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
                } else if let Err(e) = self.execute_recipe(
                    target,
                    &rule.recipe,
                    &rule.recipe_lines,
                    &rule.source_name,
                    &prereqs,
                    &order_only,
                    &[],
                    "",
                ) {
                    return Err(e);
                } else {
                    *self.recipe_executed.borrow_mut() = true;
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

        if let Some((pat_rule, pat_stem)) = &pattern_match {
            stem = pat_stem.clone();
            for pp in &pat_rule.prereq_patterns {
                let prereq = pp.replace('%', &stem);
                implied_prereqs.push(prereq.clone());
                all_prereqs.push(prereq);
            }
            if recipe.is_empty() {
                recipe = pat_rule.recipe.clone();
                recipe_lines = pat_rule.recipe_lines.clone();
                recipe_source = pat_rule.source_name.clone();
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
        let assume_new = self.assume_new.borrow();
        let has_assume_new_prereq = all_prereqs.iter().any(|p| assume_new.contains(p));
        drop(assume_new);
        for prereq in &all_prereqs {
            if let Err(e) = self.build_target_for(prereq, Some(target)) {
                pop_scope(self);
                return Err(e);
            }
            if self.rebuilt_targets.borrow().contains(prereq) {
                has_rebuilt_prereq = true;
            }
            if let Ok(meta) = std::fs::metadata(prereq)
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
            }
        }

        // Build .EXTRA_PREREQS after normal prereqs but before
        // order-only, and keep them out of `$^`/`$<`.
        if !in_extras {
            for prereq in &extra_prereqs {
                self.build_target_for(prereq, Some(target))?;
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
            std::fs::metadata(target)
                .ok()
                .and_then(|m| m.modified().ok())
        };

        let needs_rebuild = self.always_make
            || is_phony
            || has_phony_prereq
            || has_assume_new_prereq
            || has_rebuilt_prereq
            || target_mtime.is_none()
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
            if is_phony || Path::new(target).exists() || !rules.is_empty() {
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
            pop_scope(self);
            return Err(msg);
        }

        // Mark this target as rebuilt so dependents see it as updated
        // (needed in dry-run where file mtime doesn't actually change).
        self.rebuilt_targets.borrow_mut().insert(target.to_string());

        // Execute recipe
        if self.touch {
            if !is_phony {
                if !self.silent {
                    println!("touch {target}");
                }
                *self.recipe_executed.borrow_mut() = true;
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
            // -q: don't execute the recipe; report that update was needed.
            *self.question_needs_update.borrow_mut() = true;
        } else if let Err(e) = self.execute_recipe(
            target,
            &recipe,
            &recipe_lines,
            &recipe_source,
            &all_prereqs,
            &all_order_only,
            &implied_prereqs,
            &stem,
        ) {
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
                pop_scope(self);
                // Return an empty marker error; outer build() will treat
                // the empty string as "already reported".
                return Err(String::new());
            }
            pop_scope(self);
            return Err(e);
        }

        self.built_targets.borrow_mut().insert(target.to_string());
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

    fn find_pattern_rule(&self, target: &str) -> Option<(PatternRuleEntry, String)> {
        let suffixes_cleared = *self.suffixes_cleared.borrow();
        let pattern_rules = self.pattern_rules.borrow();
        // Fallback candidate: the last user-defined pattern rule whose
        // target matches, used when no rule has an existing/buildable
        // prereq. GNU make still applies *some* matching rule even when
        // the prereq is missing; we pick the most-recently-declared.
        let mut fallback: Option<(PatternRuleEntry, String)> = None;
        for rule in pattern_rules.iter().rev() {
            // `.SUFFIXES:` (empty) disables built-in suffix-based pattern
            // rules. We tag built-ins with source_name "<built-in>".
            if suffixes_cleared && rule.source_name == "<built-in>" {
                continue;
            }
            if let Some(stem) = expand::pattern_stem(target, &rule.target_pattern) {
                // Check that at least one prerequisite exists or can be built
                let prereqs_ok = rule.prereq_patterns.is_empty()
                    || rule.prereq_patterns.iter().any(|pp| {
                        let prereq = pp.replace('%', &stem);
                        Path::new(&prereq).exists() || self.rules.borrow().contains_key(&prereq)
                    });
                if prereqs_ok {
                    return Some((rule.clone(), stem));
                }
                if rule.source_name != "<built-in>" && fallback.is_none() {
                    fallback = Some((rule.clone(), stem));
                }
            }
        }
        fallback
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
    ) -> Result<(), String> {
        // Apply target-specific variable assignments (including those
        // inherited from ancestor targets currently being built). Save
        // prior values so we can restore them afterwards.
        let tv_entries: Vec<(String, AssignOp, String)> = self.collect_target_vars(target);
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
    ) -> Result<(), String> {
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

        // $? = prerequisites newer than target
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
        auto_vars.insert("?", newer.join(" "));

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
        let mut expanded_lines: Vec<(String, usize, bool, bool)> = Vec::new();
        for (idx, line) in recipe.iter().enumerate() {
            let line_no = recipe_lines.get(idx).copied().unwrap_or(0);
            if line_no > 0 {
                *self.current_source.borrow_mut() = Some((recipe_source.to_string(), line_no));
            }
            let mut outer_silent = false;
            let mut outer_ignore = false;
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
                    raw = rest;
                } else {
                    break;
                }
            }
            let expanded_raw = expand::expand_with_auto(raw, self, &auto_vars);
            for sub in expanded_raw.split('\n') {
                if !sub.is_empty() {
                    expanded_lines.push((sub.to_string(), line_no, outer_silent, outer_ignore));
                }
            }
        }
        *self.current_source.borrow_mut() = None;

        let target_is_silent =
            *self.silent_all.borrow() || self.silent_targets.borrow().contains(target);

        for (expanded_raw, line_no, outer_silent, outer_ignore) in &expanded_lines {
            let mut silent = self.silent || target_is_silent || *outer_silent;
            let mut ignore_error = *outer_ignore;

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

            // In dry-run mode (-n), GNU make ignores the `@` silence prefix
            // so every recipe line is echoed, even silenced ones.
            if !silent || self.dry_run {
                println!("{expanded}");
            }

            if self.dry_run {
                continue;
            }

            // Export variables
            let mut cmd = std::process::Command::new(&shell);
            // Split .SHELLFLAGS on whitespace like GNU make does,
            // so that e.g. "-e -c" becomes two separate arguments.
            for flag in shell_flags.split_whitespace() {
                cmd.arg(flag);
            }
            cmd.arg(&expanded);

            if *self.export_all.borrow() {
                for (name, var) in self.vars.borrow().iter() {
                    cmd.env(name, &var.value);
                }
            } else {
                // Re-export any variable the make process inherited
                // from its environment — even if a makefile assignment
                // later changed the value (and thus the origin). The
                // child would otherwise inherit our stale env entry.
                for name in self.env_inherited.borrow().iter() {
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
                            return Err(format!("[{loc}{target}] Error {code}"));
                        }
                    }
                }
                Err(e) => {
                    if !ignore_error && !self.ignore_errors {
                        return Err(format!("{target}: {e}"));
                    }
                }
            }
        }

        Ok(())
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
