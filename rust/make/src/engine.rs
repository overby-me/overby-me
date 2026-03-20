//! Make execution engine: variable storage, rule database, and build logic.

use crate::ast::*;
use crate::expand;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Origin of a variable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VarOrigin {
    Default,
    Environment,
    File,
    CommandLine,
    Override,
    #[allow(dead_code)]
    Automatic,
}

impl std::fmt::Display for VarOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

struct Variable {
    value: String,
    flavor: VarFlavor,
    origin: VarOrigin,
}

/// A stored rule.
#[derive(Debug, Clone)]
struct RuleEntry {
    prerequisites: Vec<String>,
    order_only: Vec<String>,
    recipe: Vec<String>,
    #[allow(dead_code)]
    is_double_colon: bool,
}

/// A pattern rule entry.
#[derive(Debug, Clone)]
struct PatternRuleEntry {
    target_pattern: String,
    prereq_patterns: Vec<String>,
    recipe: Vec<String>,
}

pub struct Engine {
    vars: RefCell<HashMap<String, Variable>>,
    rules: RefCell<HashMap<String, Vec<RuleEntry>>>,
    pattern_rules: RefCell<Vec<PatternRuleEntry>>,
    default_goal: RefCell<Option<String>>,
    phony_targets: RefCell<HashSet<String>>,
    suffixes: RefCell<Vec<String>>,
    exports: RefCell<HashSet<String>>,
    export_all: RefCell<bool>,
    built_targets: RefCell<HashSet<String>>,
    eval_queue: RefCell<Vec<String>>,
    // Options
    pub jobs: usize,
    pub keep_going: bool,
    pub dry_run: bool,
    pub silent: bool,
    pub touch: bool,
    pub question: bool,
    pub always_make: bool,
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
            jobs: 1,
            keep_going: false,
            dry_run: false,
            silent: false,
            touch: false,
            question: false,
            always_make: false,
        };

        // Set default variables
        engine.set_var_with_origin("MAKE", "make", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin("SHELL", "/bin/sh", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin("MAKEFLAGS", "", VarFlavor::Simple, VarOrigin::Default);
        engine.set_var_with_origin(".SHELLFLAGS", "-c", VarFlavor::Simple, VarOrigin::Default);
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
        });
    }

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
        self.vars.borrow_mut().insert(
            name.to_string(),
            Variable {
                value: value.to_string(),
                flavor,
                origin,
            },
        );
    }

    /// Lookup a variable, expanding if recursive.
    pub fn lookup_var(&self, name: &str) -> String {
        let vars = self.vars.borrow();
        if let Some(var) = vars.get(name) {
            let value = var.value.clone();
            let flavor = var.flavor;
            drop(vars);
            match flavor {
                VarFlavor::Recursive => expand::expand(&value, self),
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
        self.vars
            .borrow()
            .get(name)
            .map(|v| v.origin)
            .unwrap_or(VarOrigin::Default)
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

    pub fn eval_text(&self, text: &str) {
        self.eval_queue.borrow_mut().push(text.to_string());
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
                        if let Ok(paths) = glob::glob(file) {
                            for entry in paths.flatten() {
                                let path = entry.to_string_lossy().to_string();
                                self.load_file(&path, *silent);
                            }
                        } else {
                            self.load_file(file, *silent);
                        }
                    }
                }
            }
            Directive::Conditional(cond) => {
                self.process_conditional(cond);
            }
            Directive::Export(var) => {
                if let Some(name) = var {
                    let expanded = expand::expand(name, self);
                    self.exports.borrow_mut().insert(expanded);
                } else {
                    *self.export_all.borrow_mut() = true;
                }
            }
            Directive::Unexport(var) => {
                if let Some(name) = var {
                    let expanded = expand::expand(name, self);
                    self.exports.borrow_mut().remove(&expanded);
                } else {
                    *self.export_all.borrow_mut() = false;
                }
            }
            Directive::Override(assign) => {
                self.process_assignment(assign, VarOrigin::Override);
            }
            Directive::Define(name, op, lines) => {
                let value = lines.join("\n");
                let expanded_name = expand::expand(name, self);
                let flavor = match op {
                    AssignOp::Simple => VarFlavor::Simple,
                    _ => VarFlavor::Recursive,
                };
                let final_value = if *op == AssignOp::Simple {
                    expand::expand(&value, self)
                } else {
                    value
                };
                self.set_var(&expanded_name, &final_value, flavor);
            }
            Directive::Undefine(name) => {
                let expanded = expand::expand(name, self);
                self.vars.borrow_mut().remove(&expanded);
            }
            Directive::Vpath(_) => {
                // TODO: VPATH support
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
                let existing = self.lookup_var_raw(&name);
                let new_value = if existing.is_empty() {
                    assign.value.clone()
                } else {
                    format!("{} {}", existing, assign.value)
                };
                let flavor = self.var_flavor(&name);
                let flavor = if flavor == VarFlavor::Undefined {
                    VarFlavor::Recursive
                } else {
                    flavor
                };
                self.set_var_with_origin(&name, &new_value, flavor, origin);
            }
            AssignOp::Shell => {
                let cmd = expand::expand(&assign.value, self);
                let shell = self.lookup_var_or("SHELL", "/bin/sh");
                let output = std::process::Command::new(&shell)
                    .arg("-c")
                    .arg(&cmd)
                    .output()
                    .map(|o| {
                        String::from_utf8_lossy(&o.stdout)
                            .replace('\n', " ")
                            .trim_end()
                            .to_string()
                    })
                    .unwrap_or_default();
                self.set_var_with_origin(&name, &output, VarFlavor::Simple, origin);
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

        // Handle special targets
        for target in &targets {
            match target.as_str() {
                ".PHONY" => {
                    for p in &prereqs {
                        self.phony_targets.borrow_mut().insert(p.clone());
                    }
                    return;
                }
                ".SUFFIXES" => {
                    if prereqs.is_empty() {
                        self.suffixes.borrow_mut().clear();
                    } else {
                        self.suffixes.borrow_mut().extend(prereqs.clone());
                    }
                    return;
                }
                ".DEFAULT"
                | ".PRECIOUS"
                | ".INTERMEDIATE"
                | ".SECONDARY"
                | ".DELETE_ON_ERROR"
                | ".IGNORE"
                | ".SILENT"
                | ".EXPORT_ALL_VARIABLES"
                | ".NOTPARALLEL"
                | ".ONESHELL"
                | ".POSIX" => {
                    if target == ".EXPORT_ALL_VARIABLES" {
                        *self.export_all.borrow_mut() = true;
                    }
                    return;
                }
                _ => {}
            }
        }

        // Old-style suffix rules: .c.o: → %.o: %.c
        // A target like ".XY" where .X and .Y are known suffixes
        if targets.len() == 1 && prereqs.is_empty() && targets[0].starts_with('.') {
            let target = &targets[0];
            if let Some((src_suffix, dst_suffix)) = self.parse_suffix_rule(target) {
                self.pattern_rules.borrow_mut().push(PatternRuleEntry {
                    target_pattern: format!("%{dst_suffix}"),
                    prereq_patterns: vec![format!("%{src_suffix}")],
                    recipe: rule.recipe.clone(),
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
                });
            }
            return;
        }

        // Set default goal
        {
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
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let mut parser = crate::parser::Parser::new(&content);
                match parser.parse() {
                    Ok(directives) => self.load_makefile(&directives),
                    Err(e) => {
                        if !silent {
                            eprintln!("{}: {}", path, e);
                        }
                    }
                }
            }
            Err(e) => {
                if !silent {
                    eprintln!("make: {}: {}", path, e);
                }
            }
        }
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
        let targets = if targets.is_empty() {
            match self.default_goal.borrow().as_ref() {
                Some(goal) => vec![goal.clone()],
                None => {
                    eprintln!("make: *** No targets.  Stop.");
                    return 2;
                }
            }
        } else {
            targets.to_vec()
        };

        for target in &targets {
            let target = expand::expand(target, self);
            match self.build_target(&target) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("make: *** {}.  Stop.", e);
                    if !self.keep_going {
                        return 2;
                    }
                }
            }
        }

        if self.question {
            // In question mode, return 1 if any target needed updating
            return 0;
        }

        0
    }

    fn build_target(&self, target: &str) -> Result<(), String> {
        // Normalize path: strip leading ./ for consistency with rule lookup
        let target = target
            .strip_prefix("./")
            .unwrap_or(target);

        // Already built?
        if self.built_targets.borrow().contains(target) {
            return Ok(());
        }

        let is_phony = self.phony_targets.borrow().contains(target);

        // Find explicit rules
        let rules = self.rules.borrow().get(target).cloned().unwrap_or_default();

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
        let mut stem = String::new();
        // Track pattern-implied prerequisites for $< resolution
        let mut implied_prereqs: Vec<String> = Vec::new();

        for rule in &rules {
            all_prereqs.extend(rule.prerequisites.iter().map(|p| expand::expand(p, self)));
            all_order_only.extend(rule.order_only.iter().map(|p| expand::expand(p, self)));
            if recipe.is_empty() && !rule.recipe.is_empty() {
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
            }
        }

        // Build order-only prerequisites (just ensure they exist)
        for prereq in &all_order_only {
            self.build_target(prereq)?;
        }

        // Build normal prerequisites
        let mut newest_prereq: Option<std::time::SystemTime> = None;
        for prereq in &all_prereqs {
            self.build_target(prereq)?;
            if let Ok(meta) = std::fs::metadata(prereq)
                && let Ok(mtime) = meta.modified()
            {
                newest_prereq = Some(match newest_prereq {
                    Some(t) if mtime > t => mtime,
                    Some(t) => t,
                    None => mtime,
                });
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
            || target_mtime.is_none()
            || match (target_mtime, newest_prereq) {
                (Some(t), Some(p)) => p > t,
                _ => false,
            };

        if !needs_rebuild {
            self.built_targets.borrow_mut().insert(target.to_string());
            return Ok(());
        }

        // No recipe and target doesn't exist?
        if recipe.is_empty() {
            if is_phony || Path::new(target).exists() || !rules.is_empty() {
                self.built_targets.borrow_mut().insert(target.to_string());
                return Ok(());
            }
            return Err(format!("No rule to make target '{target}'"));
        }

        // Execute recipe
        if self.touch {
            if !is_phony {
                if !self.silent {
                    println!("touch {target}");
                }
                std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(target)
                    .ok();
            }
        } else {
            self.execute_recipe(target, &recipe, &all_prereqs, &implied_prereqs, &stem)?;
        }

        self.built_targets.borrow_mut().insert(target.to_string());
        Ok(())
    }

    fn find_pattern_rule(&self, target: &str) -> Option<(PatternRuleEntry, String)> {
        let pattern_rules = self.pattern_rules.borrow();
        for rule in pattern_rules.iter().rev() {
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
            }
        }
        None
    }

    fn execute_recipe(
        &self,
        target: &str,
        recipe: &[String],
        prereqs: &[String],
        implied_prereqs: &[String],
        stem: &str,
    ) -> Result<(), String> {
        // Set up automatic variables
        let mut auto_vars: HashMap<&str, String> = HashMap::new();
        auto_vars.insert("@", target.to_string());
        // $< is the first prerequisite. When a pattern/suffix rule provides
        // the recipe, $< should be the implied source (e.g., the .c file),
        // not an explicit order-only prerequisite like .dirstamp.
        let first_prereq = if !implied_prereqs.is_empty() {
            implied_prereqs.first().cloned().unwrap_or_default()
        } else {
            prereqs.first().cloned().unwrap_or_default()
        };
        auto_vars.insert("<", first_prereq);
        auto_vars.insert("^", dedup_join(prereqs));
        auto_vars.insert("+", prereqs.join(" "));
        auto_vars.insert("*", stem.to_string());

        // $? = prerequisites newer than target
        let target_mtime = std::fs::metadata(target)
            .ok()
            .and_then(|m| m.modified().ok());
        let newer: Vec<&str> = prereqs
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

        // $| = order-only prerequisites
        auto_vars.insert("|", String::new());

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

        for line in recipe {
            let mut silent = self.silent;
            let mut ignore_error = false;
            let mut line = line.as_str();

            // Process line prefixes
            loop {
                line = line.trim_start();
                if let Some(rest) = line.strip_prefix('@') {
                    silent = true;
                    line = rest;
                } else if let Some(rest) = line.strip_prefix('-') {
                    ignore_error = true;
                    line = rest;
                } else if let Some(rest) = line.strip_prefix('+') {
                    // Force execution even in dry-run mode
                    line = rest;
                } else {
                    break;
                }
            }

            let expanded = expand::expand_with_auto(line, self, &auto_vars);

            if expanded.trim().is_empty() {
                continue;
            }

            if !silent {
                println!("{expanded}");
            }

            if self.dry_run {
                continue;
            }

            // Export variables
            let mut cmd = std::process::Command::new(&shell);
            cmd.arg(&shell_flags).arg(&expanded);

            if *self.export_all.borrow() {
                for (name, var) in self.vars.borrow().iter() {
                    cmd.env(name, &var.value);
                }
            } else {
                for name in self.exports.borrow().iter() {
                    let value = self.lookup_var(name);
                    cmd.env(name, &value);
                }
            }

            match cmd.status() {
                Ok(status) => {
                    if !status.success() && !ignore_error {
                        let code = status.code().unwrap_or(2);
                        return Err(format!("[{target}] Error {code}"));
                    }
                }
                Err(e) => {
                    if !ignore_error {
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
