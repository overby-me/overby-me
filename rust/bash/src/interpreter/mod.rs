mod arithmetic;
mod commands;
mod pipeline;
mod redirects;
mod signals;
mod traps;

use crate::ast::*;
use crate::builtins::{self, BuiltinFn};
use crate::expand;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

pub use signals::{install_signal_handler, take_pending_signal};

/// Check if a string is a valid bash identifier (variable name).
pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Saved variable state for local scope restoration
#[derive(Clone)]
pub struct SavedVar {
    pub scalar: Option<String>,
    pub array: Option<Vec<String>>,
    pub assoc: Option<AssocArray>,
    pub was_integer: bool,
    pub was_readonly: bool,
}

/// Bash-compatible hash function (FNV-1 variant) for associative arrays.
/// This ensures iteration order matches bash's hash table ordering.
#[derive(Default, Clone)]
pub struct BashHasher(u32);

impl std::hash::Hasher for BashHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self
                .0
                .wrapping_add(self.0 << 1)
                .wrapping_add(self.0 << 4)
                .wrapping_add(self.0 << 7)
                .wrapping_add(self.0 << 8)
                .wrapping_add(self.0 << 24);
            self.0 ^= b as u32;
        }
    }

    fn finish(&self) -> u64 {
        self.0 as u64
    }
}

impl BashHasher {
    fn new() -> Self {
        BashHasher(2_166_136_261) // FNV_OFFSET
    }
}

#[derive(Clone, Default)]
#[allow(dead_code)]
pub struct BashBuildHasher;

impl std::hash::BuildHasher for BashBuildHasher {
    type Hasher = BashHasher;
    fn build_hasher(&self) -> BashHasher {
        BashHasher::new()
    }
}

/// Bash-compatible hash table for associative arrays.
/// Uses separate chaining with LIFO insertion per bucket, matching bash's exact iteration order.
#[derive(Clone, Debug)]
pub struct AssocArray {
    buckets: Vec<Vec<(String, String)>>,
    len: usize,
}

impl Default for AssocArray {
    fn default() -> Self {
        Self {
            buckets: (0..1024).map(|_| Vec::new()).collect(),
            len: 0,
        }
    }
}

#[allow(dead_code)]
impl AssocArray {
    fn bucket_idx(key: &str) -> usize {
        let mut h = BashHasher::new();
        std::hash::Hasher::write(&mut h, key.as_bytes());
        (std::hash::Hasher::finish(&h) as usize) & 1023
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        let idx = Self::bucket_idx(key);
        self.buckets[idx]
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        let idx = Self::bucket_idx(&key);
        if let Some(entry) = self.buckets[idx].iter_mut().find(|(k, _)| *k == key) {
            let old = std::mem::replace(&mut entry.1, value);
            Some(old)
        } else {
            // LIFO: insert at the front of the bucket
            self.buckets[idx].insert(0, (key, value));
            self.len += 1;
            None
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        let idx = Self::bucket_idx(key);
        if let Some(pos) = self.buckets[idx].iter().position(|(k, _)| k == key) {
            let (_, v) = self.buckets[idx].remove(pos);
            self.len -= 1;
            Some(v)
        } else {
            None
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.buckets.iter().flat_map(|b| b.iter().map(|(k, _)| k))
    }

    pub fn values(&self) -> impl Iterator<Item = &String> {
        self.buckets.iter().flat_map(|b| b.iter().map(|(_, v)| v))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.buckets
            .iter()
            .flat_map(|b| b.iter().map(|(k, v)| (k, v)))
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn entry(&mut self, key: String) -> AssocEntry<'_> {
        let idx = Self::bucket_idx(&key);
        if self.buckets[idx].iter().any(|(k, _)| *k == key) {
            AssocEntry::Occupied(AssocOccupiedEntry {
                buckets: &mut self.buckets[idx],
                key,
            })
        } else {
            AssocEntry::Vacant(AssocVacantEntry {
                buckets: &mut self.buckets[idx],
                len: &mut self.len,
                key,
            })
        }
    }
}

pub enum AssocEntry<'a> {
    Occupied(AssocOccupiedEntry<'a>),
    Vacant(AssocVacantEntry<'a>),
}

pub struct AssocOccupiedEntry<'a> {
    buckets: &'a mut Vec<(String, String)>,
    key: String,
}

pub struct AssocVacantEntry<'a> {
    buckets: &'a mut Vec<(String, String)>,
    len: &'a mut usize,
    key: String,
}

#[allow(dead_code)]
impl<'a> AssocEntry<'a> {
    pub fn or_default(self) -> &'a mut String {
        self.or_insert_with(String::new)
    }

    pub fn or_insert(self, default: String) -> &'a mut String {
        self.or_insert_with(|| default)
    }

    pub fn or_insert_with<F: FnOnce() -> String>(self, f: F) -> &'a mut String {
        match self {
            AssocEntry::Occupied(e) => {
                let pos = e.buckets.iter().position(|(k, _)| *k == e.key).unwrap();
                &mut e.buckets[pos].1
            }
            AssocEntry::Vacant(e) => {
                e.buckets.insert(0, (e.key, f()));
                *e.len += 1;
                &mut e.buckets[0].1
            }
        }
    }

    pub fn and_modify<F: FnOnce(&mut String)>(self, f: F) -> Self {
        match self {
            AssocEntry::Occupied(e) => {
                let pos = e.buckets.iter().position(|(k, _)| *k == e.key).unwrap();
                f(&mut e.buckets[pos].1);
                AssocEntry::Occupied(AssocOccupiedEntry {
                    buckets: e.buckets,
                    key: e.key,
                })
            }
            AssocEntry::Vacant(e) => AssocEntry::Vacant(e),
        }
    }
}
use std::io::Write;

/// Saved shell options: (errexit, nounset, xtrace, noclobber, noglob, pipefail)
type SavedOpts = (bool, bool, bool, bool, bool, bool);

pub struct Shell {
    pub vars: HashMap<String, String>,
    pub exports: HashMap<String, String>,
    pub readonly_vars: HashSet<String>,
    pub readonly_funcs: HashSet<String>,
    pub integer_vars: HashSet<String>,
    pub uppercase_vars: HashSet<String>,
    pub lowercase_vars: HashSet<String>,
    pub capitalize_vars: HashSet<String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub assoc_arrays: HashMap<String, AssocArray>,
    pub functions: HashMap<String, CompoundCommand>,
    pub func_body_lines: HashMap<String, usize>, // function name → body start line
    pub traced_funcs: HashSet<String>,
    pub hash_table: HashMap<String, (String, u32)>,
    pub positional: Vec<String>,
    pub last_status: i32,
    pub last_bg_pid: i32,
    /// PID of the top-level shell (for $$)
    pub top_level_pid: u32,
    pub returning: bool,
    pub return_explicit_arg: bool, // true if `return N` had explicit argument
    pub breaking: i32,
    pub continuing: i32,
    pub in_condition: bool,
    pub comsub_line_offset: usize, // line offset for command substitution LINENO
    pub in_debug_trap: bool,
    pub in_trap_handler: i32,
    pub errexit_suppressed: bool,
    pub sourcing: bool,
    /// The original script file name for error messages (doesn't change with BASH_ARGV0)
    pub script_name: String,
    pub dir_stack: Vec<String>,
    pub func_names: Vec<String>,
    pub traps: HashMap<String, String>,
    /// True when running inside a command substitution (for error messages)
    pub in_comsub: bool,
    /// Signals that were ignored (SIG_IGN) at shell startup — cannot be trapped
    pub original_ignored_signals: HashSet<String>,
    pub namerefs: HashMap<String, String>,
    /// Stack of local variable scopes. Each scope maps variable names to their
    /// saved values (None if the variable didn't exist before).
    pub local_scopes: Vec<HashMap<String, SavedVar>>,
    pub saved_opts_stack: Vec<Option<SavedOpts>>,

    // Shell options (set)
    pub opt_errexit: bool,
    pub opt_nounset: bool,
    pub opt_xtrace: bool,
    pub opt_pipefail: bool,
    pub opt_keyword: bool,
    pub opt_noclobber: bool,
    pub opt_noglob: bool,
    pub opt_noexec: bool,
    pub opt_posix: bool,
    pub opt_hashall: bool,
    pub opt_monitor: bool,      // set -m / set -o monitor (job control)
    pub script_fd: Option<i32>, // original script fd (for exec 0< detection)
    pub login_shell: bool,
    /// Name of the currently executing builtin (for error messages)
    pub current_builtin: Option<String>,
    pub opt_allexport: bool,

    // Shell options (shopt)
    pub shopt_nullglob: bool,
    pub shopt_extglob: bool,
    pub shopt_globstar: bool,
    pub shopt_inherit_errexit: bool,
    pub shopt_nocasematch: bool,
    pub shopt_lastpipe: bool,
    pub shopt_expand_aliases: bool,
    /// Generic shopt options storage for options not yet individually tracked
    pub shopt_options: HashMap<String, bool>,
    pub in_pipeline_child: bool,
    pub dash_c_mode: bool,
    pub loop_depth: i32,
    /// Original top-level arithmetic expression for error reporting
    arith_top_expr: Option<String>,
    arith_context: Option<String>,
    /// Arithmetic evaluation recursion depth
    arith_depth: u32,
    /// Whether current arithmetic evaluation is from (( )) command (adds ((: prefix to errors)
    arith_is_command: bool,
    /// Whether current arithmetic evaluation is from let builtin (adds let: prefix to errors)
    pub arith_is_let: bool,
    /// Seed for RANDOM variable (bash-compatible LCRNG)
    random_seed: u32,

    pub aliases: HashMap<String, String>,
    builtins: HashMap<&'static str, BuiltinFn>,
}

impl Shell {
    pub fn new() -> Self {
        let mut vars = HashMap::new();
        let mut exports = HashMap::new();

        // Import environment variables
        for (key, value) in std::env::vars() {
            vars.insert(key.clone(), value.clone());
            exports.insert(key, value);
        }

        // Set default variables
        if let Ok(pwd) = std::env::current_dir() {
            vars.insert("PWD".to_string(), pwd.to_string_lossy().to_string());
        }
        vars.entry("IFS".to_string())
            .or_insert_with(|| " \t\n".to_string());
        vars.insert("BASH".to_string(), "/proc/self/exe".to_string());
        // Report as bash 5.3 for compatibility with setup.sh version check
        vars.insert("BASH_VERSION".to_string(), "5.3.0(1)-rust".to_string());
        // BASH_VERSINFO is stored as both a var (for $BASH_VERSINFO) and array
        vars.insert("SHELL".to_string(), "/bin/bash".to_string());
        vars.insert("OPTIND".to_string(), "1".to_string());
        vars.insert("OPTERR".to_string(), "1".to_string());
        vars.insert("LINENO".to_string(), "1".to_string());
        vars.insert("RANDOM".to_string(), "0".to_string());
        vars.insert("SECONDS".to_string(), "0".to_string());
        vars.insert("BASHPID".to_string(), std::process::id().to_string());
        vars.insert("BASH_SUBSHELL".to_string(), "0".to_string());
        {
            let mut buf = [0u8; 256];
            if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0 {
                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                vars.insert(
                    "HOSTNAME".to_string(),
                    String::from_utf8_lossy(&buf[..len]).to_string(),
                );
            }
        }
        #[cfg(unix)]
        {
            vars.insert("UID".to_string(), unsafe { libc::getuid() }.to_string());
            vars.insert("EUID".to_string(), unsafe { libc::geteuid() }.to_string());
        }
        // SHLVL: increment from environment
        let shlvl: i32 = std::env::var("SHLVL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
            + 1;
        vars.insert("SHLVL".to_string(), shlvl.to_string());
        exports.insert("SHLVL".to_string(), shlvl.to_string());
        unsafe { std::env::set_var("SHLVL", shlvl.to_string()) };
        if let Ok(hostname) = std::env::var("HOSTNAME") {
            vars.entry("HOSTNAME".to_string()).or_insert(hostname);
        }
        vars.insert("OSTYPE".to_string(), "linux-gnu".to_string());
        vars.insert(
            "MACHTYPE".to_string(),
            format!("{}-pc-linux-gnu", std::env::consts::ARCH),
        );
        vars.insert("PPID".to_string(), {
            #[cfg(unix)]
            {
                nix::unistd::getppid().to_string()
            }
            #[cfg(not(unix))]
            {
                "0".to_string()
            }
        });

        let mut shell = Self {
            vars,
            exports,
            readonly_vars: HashSet::new(),
            readonly_funcs: HashSet::new(),
            integer_vars: HashSet::new(),
            uppercase_vars: HashSet::new(),
            lowercase_vars: HashSet::new(),
            capitalize_vars: HashSet::new(),
            arrays: HashMap::new(),
            assoc_arrays: HashMap::new(),
            functions: HashMap::new(),
            func_body_lines: HashMap::new(),
            traced_funcs: HashSet::new(),
            hash_table: HashMap::new(),
            positional: vec!["bash".to_string()],
            last_status: 0,
            last_bg_pid: 0,
            top_level_pid: std::process::id(),
            returning: false,
            return_explicit_arg: false,
            breaking: 0,
            continuing: 0,
            in_condition: false,
            comsub_line_offset: 0,
            in_debug_trap: false,
            in_trap_handler: 0,
            errexit_suppressed: false,
            sourcing: false,
            script_name: String::new(),
            dir_stack: Vec::new(),
            func_names: Vec::new(),
            traps: HashMap::new(),
            in_comsub: false,
            original_ignored_signals: HashSet::new(),
            namerefs: HashMap::new(),
            local_scopes: Vec::new(),
            saved_opts_stack: Vec::new(),
            opt_errexit: false,
            opt_nounset: false,
            opt_xtrace: false,
            opt_pipefail: false,
            opt_keyword: false,
            opt_noclobber: false,
            opt_noglob: false,
            opt_noexec: false,
            opt_posix: false,
            opt_hashall: true, // enabled by default
            opt_monitor: false,
            script_fd: None,
            login_shell: false,
            current_builtin: None,
            opt_allexport: false,
            shopt_nullglob: false,
            shopt_extglob: false,
            shopt_globstar: false,
            shopt_inherit_errexit: false,
            shopt_nocasematch: false,
            shopt_lastpipe: false,
            shopt_expand_aliases: false,
            shopt_options: HashMap::new(),
            in_pipeline_child: false,
            dash_c_mode: false,
            loop_depth: 0,
            arith_top_expr: None,
            arith_context: None,
            arith_depth: 0,
            arith_is_command: false,
            arith_is_let: false,
            random_seed: std::process::id(),
            aliases: HashMap::new(),
            builtins: builtins::builtins(),
        };

        // Set up BASH_VERSINFO array (must be after struct init)
        shell.arrays.insert(
            "BASH_VERSINFO".to_string(),
            vec![
                "5".to_string(),
                "3".to_string(),
                "0".to_string(),
                "1".to_string(),
                "release".to_string(),
                std::env::consts::ARCH.to_string(),
            ],
        );

        // Set up GROUPS array (readonly)
        #[cfg(unix)]
        {
            let gid = unsafe { libc::getgid() };
            shell
                .arrays
                .insert("GROUPS".to_string(), vec![gid.to_string()]);
            // GROUPS is noassign (silently ignored, not readonly)

            // Detect privileged mode: if effective UID != real UID or effective GID != real GID
            let privileged =
                unsafe { libc::geteuid() != libc::getuid() || libc::getegid() != libc::getgid() };
            if privileged {
                shell.shopt_options.insert("privileged".to_string(), true);
            }
        }

        // Import exported functions from environment (BASH_FUNC_name%% variables)
        let func_vars: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| k.starts_with("BASH_FUNC_") && k.ends_with("%%"))
            .collect();
        for (key, value) in func_vars {
            let name = &key["BASH_FUNC_".len()..key.len() - "%%".len()];
            // Validate function name — reject names with whitespace, special chars, etc.
            let valid_name = !name.is_empty()
                && !name.contains(|c: char| c.is_whitespace() || c == '#' || c == '\0')
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':');
            if !valid_name {
                continue;
            }
            // The value starts with "() { " and contains the function body
            if let Some(body) = value.strip_prefix("() ") {
                let func_source = format!("{} () {}", name, body);
                let mut parser = crate::parser::Parser::new(&func_source);
                if let Ok(program) = parser.parse_program() {
                    // Only accept if the program is exactly one function definition
                    // (security: prevent Shellshock-style command injection)
                    if program.len() == 1
                        && program[0].list.first.commands.len() == 1
                        && program[0].list.rest.is_empty()
                        && let crate::ast::Command::FunctionDef(fname, fbody, _, _) =
                            &program[0].list.first.commands[0]
                    {
                        shell.functions.insert(fname.clone(), *fbody.clone());
                    }
                }
            }
            // Remove the BASH_FUNC variable from our vars
            shell.vars.remove(&key);
            shell.exports.remove(&key);
        }

        shell.update_shellopts();
        shell
    }

    /// Resolve a variable name through namerefs.
    pub fn resolve_nameref(&self, name: &str) -> String {
        let mut resolved = name.to_string();
        let mut seen = HashSet::new();
        while let Some(target) = self.namerefs.get(&resolved) {
            if seen.contains(target) {
                break;
            }
            seen.insert(target.clone());
            resolved = target.clone();
        }
        resolved
    }

    /// Get a variable value, resolving namerefs.
    pub fn get_var(&mut self, name: &str) -> Option<String> {
        let resolved = self.resolve_nameref(name);
        if resolved == "RANDOM" {
            // Bash-compatible LCRNG: seed = seed * 1103515245 + 12345
            self.random_seed = self
                .random_seed
                .wrapping_mul(1103515245)
                .wrapping_add(12345);
            let val = ((self.random_seed >> 16) & 0x7fff).to_string();
            self.vars.insert("RANDOM".to_string(), val.clone());
            return Some(val);
        }
        self.vars.get(&resolved).cloned()
    }

    /// Set a variable value, resolving namerefs.
    pub fn set_var(&mut self, name: &str, value: String) {
        let resolved = self.resolve_nameref(name);
        if self.readonly_vars.contains(&resolved) {
            if let Some(fname) = self.func_names.last() {
                eprintln!(
                    "{}: {}: {}: readonly variable",
                    self.error_prefix(),
                    fname,
                    resolved
                );
            } else {
                eprintln!("{}: {}: readonly variable", self.error_prefix(), resolved);
            }
            self.last_status = 1;
            return;
        }
        // Setting RANDOM sets the seed
        if resolved == "RANDOM"
            && let Ok(seed) = value.parse::<u32>()
        {
            self.random_seed = seed;
        }
        // Resetting OPTIND also resets the getopts internal offset
        if resolved == "OPTIND" {
            self.vars.remove("_GETOPTS_OPTOFS");
        }
        // Integer variables: evaluate value as arithmetic expression
        let value = if self.integer_vars.contains(&resolved) {
            self.eval_arith_expr(&value).to_string()
        } else if self.uppercase_vars.contains(&resolved) {
            value.to_uppercase()
        } else if self.lowercase_vars.contains(&resolved) {
            value.to_lowercase()
        } else if self.capitalize_vars.contains(&resolved) {
            capitalize_string(&value)
        } else {
            value
        };
        // If variable is exported, update the export
        if self.exports.contains_key(&resolved) {
            self.exports.insert(resolved.clone(), value.clone());
            unsafe { std::env::set_var(&resolved, &value) };
        }
        // Locale variables: warn if locale cannot be set
        if matches!(
            resolved.as_str(),
            "LC_ALL" | "LC_CTYPE" | "LC_COLLATE" | "LC_MESSAGES" | "LC_NUMERIC" | "LANG"
        ) && !value.is_empty()
        {
            #[cfg(unix)]
            {
                use std::ffi::CString;
                if let Ok(cval) = CString::new(value.clone()) {
                    let result = unsafe { libc::setlocale(libc::LC_ALL, cval.as_ptr()) };
                    if result.is_null() {
                        let name = self
                            .positional
                            .first()
                            .or_else(|| self.vars.get("_BASH_SOURCE_FILE"))
                            .map(|s| s.as_str())
                            .unwrap_or("bash");
                        let lineno = self.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                        eprintln!(
                            "{}: line {}: warning: setlocale: {}: cannot change locale ({}): No such file or directory",
                            name, lineno, resolved, value
                        );
                    }
                }
            }
        }
        // BASH_ARGV0 updates $0
        if resolved == "BASH_ARGV0" && !self.positional.is_empty() {
            self.positional[0] = value.clone();
        }
        // BASH_XTRACEFD: validate file descriptor
        if resolved == "BASH_XTRACEFD"
            && let Ok(fd) = value.parse::<i32>()
        {
            #[cfg(unix)]
            {
                // Check if fd is valid
                let valid = fd >= 0 && nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_GETFD).is_ok();
                if !valid && fd != 2 {
                    eprintln!(
                        "{}: BASH_XTRACEFD: {}: invalid value for trace file descriptor",
                        self.error_prefix(),
                        fd
                    );
                }
            }
        }
        // POSIXLY_CORRECT enables POSIX mode
        if resolved == "POSIXLY_CORRECT" {
            self.opt_posix = true;
        }
        self.vars.insert(resolved, value);
    }

    /// Declare a local variable — saves the old value for restoration on function exit.
    pub fn declare_local(&mut self, name: &str) {
        if let Some(scope) = self.local_scopes.last_mut()
            && !scope.contains_key(name)
        {
            scope.insert(
                name.to_string(),
                SavedVar {
                    scalar: self.vars.get(name).cloned(),
                    array: self.arrays.get(name).cloned(),
                    assoc: self.assoc_arrays.get(name).cloned(),
                    was_integer: self.integer_vars.contains(name),
                    was_readonly: self.readonly_vars.contains(name),
                },
            );
            // Remove readonly for the local scope (will be re-applied if -r is used)
            self.readonly_vars.remove(name);
        }
    }

    /// Get an array, resolving namerefs.
    #[allow(dead_code)]
    pub fn get_array(&self, name: &str) -> Option<&Vec<String>> {
        let resolved = self.resolve_nameref(name);
        self.arrays.get(&resolved)
    }

    /// Set an array, resolving namerefs.
    #[allow(dead_code)]
    pub fn set_array(&mut self, name: &str, values: Vec<String>) {
        let resolved = self.resolve_nameref(name);
        self.arrays.insert(resolved, values);
    }

    pub fn run_string(&mut self, input: &str) -> i32 {
        let mut parser = Parser::new_with_aliases(
            input,
            self.aliases.clone(),
            self.shopt_expand_aliases,
            self.opt_posix,
        );
        // Apply command substitution line offset so LINENO reflects the script line
        if self.comsub_line_offset > 0 {
            parser.set_line_offset(self.comsub_line_offset);
            self.comsub_line_offset = 0; // consume the offset
        }

        // Incremental parse-execute loop (for both scripts and -c mode)
        // Parse one command at a time, execute it, then parse the next
        // This allows scripts to continue after parse errors (like bash)
        let mut status = 0;
        let mut last_pos = usize::MAX;
        loop {
            parser.skip_newlines_and_semis();
            if parser.is_at_eof() {
                break;
            }
            // Safety: detect if we're stuck (parser didn't advance)
            let cur_pos = parser.current_pos();
            if cur_pos == last_pos {
                if self.dash_c_mode {
                    // In -c mode, a stuck parser is a syntax error
                    // Check for compound command context for better error messages
                    if parser.current_token_str() == "EOF"
                        && let Some((cmd, cmd_line)) = parser.compound_cmd_context()
                    {
                        eprintln!(
                            "{}: syntax error: unexpected end of file from `{}' command on line {}",
                            self.syntax_error_prefix(),
                            cmd,
                            cmd_line
                        );
                        return 2;
                    }
                    let token = parser.current_token_str();
                    let comsub_suffix = if self.in_comsub {
                        " while looking for matching `)'"
                    } else {
                        ""
                    };
                    eprintln!(
                        "{}: syntax error near unexpected token `{}'{}",
                        self.syntax_error_prefix(),
                        token,
                        comsub_suffix
                    );
                    let lineno: usize = self
                        .vars
                        .get("LINENO")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    let line = input
                        .lines()
                        .nth(lineno.saturating_sub(1))
                        .unwrap_or(input.lines().next().unwrap_or(input));
                    eprintln!("{}: `{}'", self.syntax_error_prefix(), line);
                    return 2;
                }
                // Parser is stuck — emit syntax error and skip
                let token = parser.current_token_str();
                if token != "EOF" {
                    eprintln!(
                        "{}: syntax error near unexpected token `{}'",
                        self.syntax_error_prefix(),
                        token
                    );
                    let lineno: usize = self
                        .vars
                        .get("LINENO")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    let line = input
                        .lines()
                        .nth(lineno.saturating_sub(1))
                        .unwrap_or(input.lines().next().unwrap_or(input));
                    eprintln!("{}: `{}'", self.syntax_error_prefix(), line.trim_end());
                    status = 2;
                }
                parser.skip_to_next_command();
                if parser.is_at_eof() {
                    break;
                }
                continue;
            }
            last_pos = cur_pos;

            // Update LINENO to current parser line
            // (but not inside trap handlers — they preserve the calling context's LINENO)
            if !self.in_debug_trap && self.in_trap_handler == 0 {
                self.vars
                    .insert("LINENO".to_string(), parser.current_line().to_string());
            }

            match parser.parse_complete_command_pub() {
                Ok(cmd) => {
                    // Emit heredoc EOF warnings
                    for (eof_line, start_line, delim) in parser.take_heredoc_eof_warnings() {
                        let name = self
                            .positional
                            .first()
                            .map(|s| s.as_str())
                            .unwrap_or("bash");
                        eprintln!(
                            "{}: line {}: warning: here-document at line {} delimited by end-of-file (wanted `{}')",
                            name, eof_line, start_line, delim
                        );
                    }
                    if let Some(line_num) = parser.heredoc_overflow_line() {
                        let name = self
                            .positional
                            .first()
                            .map(|s| s.as_str())
                            .unwrap_or("bash");
                        eprintln!(
                            "{}: line {}: maximum here-document count exceeded",
                            name, line_num
                        );
                        return 2;
                    }
                    if self.opt_noexec {
                        continue;
                    }
                    status = self.run_complete_command(&cmd);
                    // Sync aliases back to parser (alias/unalias may have changed them)
                    parser.update_aliases(
                        self.aliases.clone(),
                        self.shopt_expand_aliases,
                        self.opt_posix,
                    );
                    // Run ERR trap on non-zero status
                    if status != 0 && !self.in_condition {
                        self.run_err_trap();
                    }
                    if self.opt_errexit
                        && status != 0
                        && !self.in_condition
                        && !self.errexit_suppressed
                    {
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                        std::io::Write::flush(&mut std::io::stderr()).ok();
                        self.last_status = status;
                        self.run_exit_trap();
                        std::process::exit(self.last_status);
                    }
                    self.errexit_suppressed = false;
                    // Check if exec redirected fd 0 (stdin) — if so, read new
                    // content from fd 0 and continue execution from there
                    #[cfg(unix)]
                    if self.script_fd.is_some() && !self.in_pipeline_child {
                        // Detect if exec changed fd 0:
                        // - Different inode (different file)
                        // - Same inode but fd 0 is a new open file descriptor
                        //   (detected via /proc/self/fd/0 readlink or by checking
                        //   if the underlying file description changed)
                        let fd0_stat = nix::sys::stat::fstat(0).ok();
                        let script_stat =
                            self.script_fd.and_then(|fd| nix::sys::stat::fstat(fd).ok());
                        let changed = match (fd0_stat, script_stat) {
                            (Some(f0), Some(sf)) => {
                                f0.st_dev != sf.st_dev || f0.st_ino != sf.st_ino
                            }
                            _ => false,
                        };
                        if changed {
                            let mut new_content = String::new();
                            std::io::Read::read_to_string(&mut std::io::stdin(), &mut new_content)
                                .ok();
                            if !new_content.is_empty() {
                                let saved_script_fd = self.script_fd.take();
                                status = self.run_string(&new_content);
                                self.script_fd = saved_script_fd;
                            }
                            return status;
                        }
                    }
                    // Check for pending signals after each command
                    self.check_pending_signals();
                    if self.returning || self.breaking > 0 || self.continuing > 0 {
                        break;
                    }
                }
                Err(e) => {
                    if let Some(msg) = e.strip_prefix("\x00COND_ERROR") {
                        // Conditional expression error — print with prefix
                        eprintln!("{}: {}", self.syntax_error_prefix(), msg);
                        // For EOF errors, also print compound command context
                        if parser.current_token_str() == "EOF" {
                            if let Some((cmd, cmd_line)) = parser.compound_cmd_context() {
                                // Use line 2 for the compound command EOF error (like bash)
                                let name = self
                                    .positional
                                    .first()
                                    .map(|s| s.as_str())
                                    .unwrap_or("bash");
                                let prefix = if self.dash_c_mode {
                                    format!("{}: -c: line 2", name)
                                } else {
                                    self.syntax_error_prefix()
                                };
                                eprintln!(
                                    "{}: syntax error: unexpected end of file from `{}' command on line {}",
                                    prefix, cmd, cmd_line
                                );
                            }
                            return 2;
                        }
                        // Also print generic syntax error like bash does
                        let token = parser.current_token_str();
                        eprintln!(
                            "{}: syntax error near `{}'",
                            self.syntax_error_prefix(),
                            token
                        );
                        let lineno: usize = self
                            .vars
                            .get("LINENO")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        let line = input
                            .lines()
                            .nth(lineno.saturating_sub(1))
                            .unwrap_or(input.lines().next().unwrap_or(input));
                        eprintln!("{}: `{}'", self.syntax_error_prefix(), line.trim());
                    } else if let Some(msg) = e.strip_prefix("RUNTIME:") {
                        eprintln!("{}: {}", self.error_prefix(), msg);
                    } else if self.dash_c_mode {
                        let comsub_suffix = if self.in_comsub && e.contains("syntax error") {
                            " while looking for matching `)'"
                        } else {
                            ""
                        };
                        eprintln!("{}: {}{}", self.syntax_error_prefix(), e, comsub_suffix);
                        if e.contains("syntax error") {
                            // Show the line where the error occurred
                            let lineno: usize = self
                                .vars
                                .get("LINENO")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(1);
                            // Use outer -c string when in comsub for source line display
                            let display_source = if self.in_comsub {
                                self.vars.get("_BASH_C_STRING").cloned()
                            } else {
                                None
                            };
                            let line = if let Some(ref src) = display_source {
                                src.lines()
                                    .nth(lineno.saturating_sub(1))
                                    .unwrap_or(src.lines().next().unwrap_or(src))
                            } else {
                                input
                                    .lines()
                                    .nth(lineno.saturating_sub(1))
                                    .unwrap_or(input.lines().next().unwrap_or(input))
                            };
                            // "syntax error: X" → second line gets "syntax error: `...'"
                            // "syntax error near X" → second line gets just "`...'"
                            if e.starts_with("syntax error:") {
                                let display_line = if let Some(pos) = line.find("((") {
                                    &line[pos..]
                                } else {
                                    line
                                };
                                eprintln!(
                                    "{}: syntax error: `{}'",
                                    self.syntax_error_prefix(),
                                    display_line
                                );
                            } else {
                                eprintln!("{}: `{}'", self.syntax_error_prefix(), line);
                            }
                        }
                        return 2;
                    } else {
                        eprintln!("{}: {}", self.error_prefix(), e);
                        if e.contains("syntax error") {
                            let lineno: usize = self
                                .vars
                                .get("LINENO")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(1);
                            let line = input
                                .lines()
                                .nth(lineno.saturating_sub(1))
                                .unwrap_or(input.lines().next().unwrap_or(input));
                            eprintln!("{}: `{}'", self.error_prefix(), line.trim_end());
                        }
                    }
                    status = 2;
                    // Skip to the next newline to try to recover
                    parser.skip_to_next_command();
                }
            }
        }
        status
    }

    pub fn run_program(&mut self, program: &Program) -> i32 {
        let mut status = 0;
        for cmd in program {
            if self.returning || self.breaking > 0 || self.continuing > 0 {
                break;
            }
            status = self.run_complete_command(cmd);
            // Check for pending signals after each command
            if self.in_trap_handler == 0 {
                self.check_pending_signals();
                if self.returning || self.breaking > 0 {
                    break;
                }
            }
            // Run ERR trap on non-zero status (not in conditions or negated)
            if status != 0 && !self.in_condition {
                self.run_err_trap();
            }
            if self.opt_errexit && status != 0 && !self.in_condition && !self.errexit_suppressed {
                std::io::Write::flush(&mut std::io::stdout()).ok();
                std::io::Write::flush(&mut std::io::stderr()).ok();
                self.last_status = status;
                self.run_exit_trap();
                std::process::exit(self.last_status);
            }
            self.errexit_suppressed = false;
        }
        status
    }

    fn run_complete_command(&mut self, cmd: &CompleteCommand) -> i32 {
        // Reap any finished coproc processes
        #[cfg(unix)]
        self.reap_coprocs();

        // Update LINENO (skip inside trap handlers to preserve calling context)
        if !self.in_debug_trap && self.in_trap_handler == 0 {
            self.vars.insert("LINENO".to_string(), cmd.line.to_string());
        }

        if cmd.background {
            #[cfg(unix)]
            {
                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Parent { child }) => {
                        self.last_bg_pid = child.as_raw();
                        self.vars.insert("!".to_string(), child.to_string());
                        return 0;
                    }
                    Ok(nix::unistd::ForkResult::Child) => {
                        let status = self.run_and_or_list(&cmd.list);
                        std::process::exit(status);
                    }
                    Err(e) => {
                        eprintln!("bash: fork: {}", e);
                        return 1;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                eprintln!("bash: background execution not supported");
                return 1;
            }
        }

        let status = self.run_and_or_list(&cmd.list);
        self.last_status = status;
        status
    }

    fn run_and_or_list(&mut self, list: &AndOrList) -> i32 {
        let has_rest = !list.rest.is_empty();
        let saved = self.in_condition;
        // All commands in &&/|| list are in condition context except the last
        // that actually runs
        if has_rest {
            self.in_condition = true;
        }
        let mut status = self.run_pipeline(&list.first);
        let mut last_ran_in_condition = has_rest; // first pipeline is in condition if has_rest

        for (i, (op, pipeline)) in list.rest.iter().enumerate() {
            // Check for break/continue/return from previous command
            if self.breaking > 0 || self.continuing > 0 || self.returning {
                break;
            }
            let is_last = i == list.rest.len() - 1;
            if is_last {
                self.in_condition = saved;
            }
            match op {
                AndOr::And => {
                    if status == 0 {
                        status = self.run_pipeline(pipeline);
                        last_ran_in_condition = !is_last;
                    }
                    // If skipped (status != 0), the status is from a condition command
                }
                AndOr::Or => {
                    if status != 0 {
                        status = self.run_pipeline(pipeline);
                        last_ran_in_condition = !is_last;
                    }
                    // If skipped (status == 0), not a failure
                }
            }
        }

        self.in_condition = saved;

        // Only suppress errexit if the failing status came from a condition-position command
        if has_rest && status != 0 && last_ran_in_condition {
            self.errexit_suppressed = true;
        }
        // Negated pipelines that return non-zero (from flipping 0→1) suppress errexit
        if list.first.negated && !has_rest && status != 0 {
            self.errexit_suppressed = true;
        }

        status
    }
}

/// Find an assignment operator in an arithmetic expression.
/// Quote a word for xtrace output, matching bash's format
pub fn capitalize_string(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let mut result: String = first.to_uppercase().collect();
            result.extend(chars.map(|c| c.to_ascii_lowercase()));
            result
        }
    }
}

fn xtrace_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Check if quoting is needed
    let needs_quoting = s.chars().any(|c| {
        matches!(
            c,
            '|' | '&'
                | ';'
                | '('
                | ')'
                | '<'
                | '>'
                | ' '
                | '\t'
                | '\n'
                | '\''
                | '"'
                | '`'
                | '$'
                | '\\'
                | '!'
                | '{'
                | '}'
                | '*'
                | '?'
                | '['
                | ']'
                | '#'
                | '~'
        )
    });
    if !needs_quoting {
        return s.to_string();
    }
    // Check for control characters
    let has_control = s.chars().any(|c| c.is_control());
    if has_control {
        let mut out = String::from("$'");
        for ch in s.chars() {
            match ch {
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\x07' => out.push_str("\\a"),
                '\x08' => out.push_str("\\b"),
                '\'' => out.push_str("\\'"),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('\'');
        out
    } else if !s.contains('\'') {
        format!("'{}'", s)
    } else {
        // Use backslash escaping for words with single quotes
        let mut out = String::new();
        for ch in s.chars() {
            if matches!(
                ch,
                '|' | '&'
                    | ';'
                    | '('
                    | ')'
                    | '<'
                    | '>'
                    | ' '
                    | '\t'
                    | '"'
                    | '`'
                    | '$'
                    | '\\'
                    | '!'
                    | '{'
                    | '}'
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '#'
                    | '~'
                    | '\''
            ) {
                out.push('\\');
            }
            out.push(ch);
        }
        out
    }
}
