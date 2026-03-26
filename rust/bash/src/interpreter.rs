use crate::ast::*;
use crate::builtins::{self, BuiltinFn};
use crate::expand;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};

/// Check if a string is a valid bash identifier (variable name).
fn is_valid_identifier(s: &str) -> bool {
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
    pub hash_table: HashMap<String, (String, u32)>,
    pub positional: Vec<String>,
    pub last_status: i32,
    pub last_bg_pid: i32,
    pub returning: bool,
    pub breaking: i32,
    pub continuing: i32,
    pub in_condition: bool,
    pub in_debug_trap: bool,
    pub errexit_suppressed: bool,
    pub sourcing: bool,
    /// The original script file name for error messages (doesn't change with BASH_ARGV0)
    pub script_name: String,
    pub dir_stack: Vec<String>,
    pub func_names: Vec<String>,
    pub traps: HashMap<String, String>,
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
            hash_table: HashMap::new(),
            positional: vec!["bash".to_string()],
            last_status: 0,
            last_bg_pid: 0,
            returning: false,
            breaking: 0,
            continuing: 0,
            in_condition: false,
            in_debug_trap: false,
            errexit_suppressed: false,
            sourcing: false,
            script_name: String::new(),
            dir_stack: Vec::new(),
            func_names: Vec::new(),
            traps: HashMap::new(),
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
                        && let crate::ast::Command::FunctionDef(fname, fbody) =
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
            let sname = self
                .vars
                .get("_BASH_SOURCE_FILE")
                .or_else(|| self.positional.first())
                .map(|s| s.as_str())
                .unwrap_or("bash");
            let lineno = self.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
            eprintln!(
                "{}: line {}: {}: readonly variable",
                sname, lineno, resolved
            );
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
                },
            );
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
                    let token = parser.current_token_str();
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
                    eprintln!("{}: `{}'", self.syntax_error_prefix(), line);
                    return 2;
                }
                // Parser is stuck — skip one token and retry
                parser.skip_to_next_command();
                if parser.is_at_eof() {
                    break;
                }
                continue;
            }
            last_pos = cur_pos;

            // Update LINENO to current parser line
            // (but not inside trap handlers — they preserve the calling context's LINENO)
            if !self.in_debug_trap {
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
                        std::process::exit(status);
                    }
                    self.errexit_suppressed = false;
                    if self.returning || self.breaking > 0 || self.continuing > 0 {
                        break;
                    }
                }
                Err(e) => {
                    if let Some(msg) = e.strip_prefix("RUNTIME:") {
                        eprintln!("{}: {}", self.error_prefix(), msg);
                    } else if self.dash_c_mode {
                        eprintln!("{}: {}", self.syntax_error_prefix(), e);
                        if e.contains("syntax error") {
                            // Show the line where the error occurred
                            let lineno: usize = self
                                .vars
                                .get("LINENO")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(1);
                            let line = input
                                .lines()
                                .nth(lineno.saturating_sub(1))
                                .unwrap_or(input.lines().next().unwrap_or(input));
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
                    }
                    status = 2;
                    // Skip to the next newline to try to recover
                    parser.skip_to_next_command();
                }
            }
        }
        status
    }

    /// Execute the ERR trap if set and the command failed
    pub fn run_err_trap(&mut self) {
        if let Some(handler) = self.traps.get("ERR").cloned()
            && !handler.is_empty()
        {
            let saved = self.last_status;
            self.run_string(&handler);
            self.last_status = saved;
        }
    }

    /// Execute the DEBUG trap if set. Returns true if the trap returned non-zero
    /// (which causes the command to be skipped in extdebug mode).
    pub fn run_debug_trap(&mut self) -> bool {
        if self.in_debug_trap {
            return false;
        }
        if let Some(handler) = self.traps.get("DEBUG").cloned()
            && !handler.is_empty()
        {
            self.in_debug_trap = true;
            let saved_status = self.last_status;
            let saved_cmd = self.vars.get("BASH_COMMAND").cloned();
            // Don't let the trap handler's parser overwrite LINENO —
            // LINENO should reflect the command being debugged
            let saved_lineno = self.vars.get("LINENO").cloned();
            let trap_status = self.run_string(&handler);
            if let Some(ln) = saved_lineno {
                self.vars.insert("LINENO".to_string(), ln);
            }
            // Restore BASH_COMMAND (trap shouldn't overwrite it)
            if let Some(cmd) = saved_cmd {
                self.vars.insert("BASH_COMMAND".to_string(), cmd);
            }
            self.last_status = saved_status;
            self.in_debug_trap = false;
            trap_status != 0
        } else {
            false
        }
    }

    /// Execute the RETURN trap if set
    pub fn run_return_trap(&mut self) {
        if let Some(handler) = self.traps.get("RETURN").cloned()
            && !handler.is_empty()
        {
            let saved = self.last_status;
            self.run_string(&handler);
            self.last_status = saved;
        }
    }

    /// Execute the EXIT trap if set
    pub fn run_exit_trap(&mut self) {
        if let Some(handler) = self
            .traps
            .get("EXIT")
            .or_else(|| self.traps.get("0"))
            .cloned()
            && !handler.is_empty()
        {
            self.run_string(&handler);
        }
    }

    pub fn run_program(&mut self, program: &Program) -> i32 {
        let mut status = 0;
        for cmd in program {
            if self.returning || self.breaking > 0 || self.continuing > 0 {
                break;
            }
            status = self.run_complete_command(cmd);
            // Run ERR trap on non-zero status (not in conditions or negated)
            if status != 0 && !self.in_condition {
                self.run_err_trap();
            }
            if self.opt_errexit && status != 0 && !self.in_condition && !self.errexit_suppressed {
                std::io::Write::flush(&mut std::io::stdout()).ok();
                std::io::Write::flush(&mut std::io::stderr()).ok();
                std::process::exit(status);
            }
            self.errexit_suppressed = false;
        }
        status
    }

    fn run_complete_command(&mut self, cmd: &CompleteCommand) -> i32 {
        // Reap any finished coproc processes
        #[cfg(unix)]
        self.reap_coprocs();

        // Update LINENO (skip inside trap handlers)
        if !self.in_debug_trap {
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

    fn run_pipeline(&mut self, pipeline: &Pipeline) -> i32 {
        let start_time = if pipeline.timed {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Negated pipelines suppress errexit for inner commands
        let saved_condition = self.in_condition;
        if pipeline.negated {
            self.in_condition = true;
        }
        let status = self.run_pipeline_inner(pipeline);
        self.in_condition = saved_condition;

        if let Some(start) = start_time {
            let elapsed = start.elapsed();
            let secs = elapsed.as_secs_f64();
            if let Some(fmt) = self.vars.get("TIMEFORMAT") {
                let output = fmt
                    .replace("%2R", &format!("{:.2}", secs))
                    .replace("%2U", &format!("{:.2}", 0.0f64))
                    .replace("%2S", &format!("{:.2}", 0.0f64))
                    .replace("%R", &format!("{:.3}", secs))
                    .replace("%U", &format!("{:.3}", 0.0f64))
                    .replace("%S", &format!("{:.3}", 0.0f64));
                eprintln!("{}", output);
            } else if pipeline.time_posix {
                eprintln!("real {:.2}", secs);
                eprintln!("user {:.2}", 0.0f64);
                eprintln!("sys {:.2}", 0.0f64);
            } else {
                eprintln!("real\t{}m{:.3}s", (secs / 60.0) as u64, secs % 60.0);
                eprintln!("user\t{}m{:.3}s", 0, 0.0f64);
                eprintln!("sys\t{}m{:.3}s", 0, 0.0f64);
            }
        }

        status
    }

    fn run_pipeline_inner(&mut self, pipeline: &Pipeline) -> i32 {
        if pipeline.commands.len() == 1 {
            let status = self.run_command(&pipeline.commands[0]);
            return if pipeline.negated {
                if status == 0 { 1 } else { 0 }
            } else {
                status
            };
        }

        #[cfg(unix)]
        {
            use std::os::unix::io::{IntoRawFd, RawFd};

            let mut prev_read_fd: Option<RawFd> = None;
            let mut children: Vec<nix::unistd::Pid> = Vec::new();

            for (i, cmd) in pipeline.commands.iter().enumerate() {
                let is_last = i == pipeline.commands.len() - 1;

                let (read_fd, write_fd): (Option<RawFd>, Option<RawFd>) = if !is_last {
                    let (r, w) = nix::unistd::pipe().expect("pipe failed");
                    (Some(r.into_raw_fd()), Some(w.into_raw_fd()))
                } else {
                    (None, None)
                };

                // lastpipe: run last command in current shell
                if is_last && self.shopt_lastpipe {
                    let stdin_was_pipe = prev_read_fd == Some(0);
                    let saved_stdin = if let Some(fd) = prev_read_fd {
                        // Save current stdin (may fail if closed)
                        let saved = if stdin_was_pipe {
                            None // fd 0 IS the pipe — original stdin was closed
                        } else {
                            nix::unistd::dup(0).ok()
                        };
                        if fd != 0 {
                            nix::unistd::dup2(fd, 0).ok();
                            nix::unistd::close(fd).ok();
                        }
                        saved
                    } else {
                        None
                    };

                    let status = self.run_command(cmd);

                    match saved_stdin {
                        Some(fd) => {
                            nix::unistd::dup2(fd, 0).ok();
                            nix::unistd::close(fd).ok();
                        }
                        None if prev_read_fd.is_some() => {
                            // stdin was closed before, close it again
                            nix::unistd::close(0).ok();
                        }
                        _ => {}
                    }

                    // Wait for all pipeline children
                    let mut statuses = Vec::new();
                    for child in &children {
                        match nix::sys::wait::waitpid(*child, None) {
                            Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => statuses.push(code),
                            _ => statuses.push(0),
                        }
                    }
                    statuses.push(status);
                    self.arrays.insert(
                        "PIPESTATUS".to_string(),
                        statuses.iter().map(|s| s.to_string()).collect(),
                    );

                    let final_status = if self.opt_pipefail {
                        statuses
                            .iter()
                            .rev()
                            .find(|&&s| s != 0)
                            .copied()
                            .unwrap_or(0)
                    } else {
                        status
                    };

                    return if pipeline.negated {
                        if final_status == 0 { 1 } else { 0 }
                    } else {
                        final_status
                    };
                }

                // Flush before fork to prevent buffer duplication
                std::io::Write::flush(&mut std::io::stdout()).ok();
                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Child) => {
                        // Mark as pipeline child (suppresses broken pipe errors)
                        // but NOT in lastpipe pipelines where the reader may close early
                        self.in_pipeline_child = !self.shopt_lastpipe;
                        if let Some(fd) = prev_read_fd
                            && fd != 0
                        {
                            nix::unistd::dup2(fd, 0).ok();
                            nix::unistd::close(fd).ok();
                        }
                        // If fd == 0, it's already stdin (pipe read end assigned to fd 0)
                        if let Some(fd) = write_fd {
                            nix::unistd::dup2(fd, 1).ok();
                            // |& redirects stderr to the pipe too
                            if i < pipeline.pipe_stderr.len() && pipeline.pipe_stderr[i] {
                                nix::unistd::dup2(fd, 2).ok();
                            }
                            if fd != 1 && fd != 2 {
                                nix::unistd::close(fd).ok();
                            }
                        }
                        if let Some(fd) = read_fd
                            && fd != 0
                            && fd != 1
                        {
                            nix::unistd::close(fd).ok();
                        }

                        let status = self.run_command(cmd);
                        std::io::stdout().flush().ok();
                        std::io::stderr().flush().ok();
                        std::process::exit(status);
                    }
                    Ok(nix::unistd::ForkResult::Parent { child }) => {
                        children.push(child);
                        if let Some(fd) = prev_read_fd {
                            nix::unistd::close(fd).ok();
                        }
                        if let Some(fd) = write_fd {
                            nix::unistd::close(fd).ok();
                        }
                        prev_read_fd = read_fd;
                    }
                    Err(e) => {
                        eprintln!("bash: fork: {}", e);
                        return 1;
                    }
                }
            }

            let mut statuses = Vec::new();
            for child in &children {
                match nix::sys::wait::waitpid(*child, None) {
                    Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => statuses.push(code),
                    Ok(nix::sys::wait::WaitStatus::Signaled(_, sig, _)) => {
                        statuses.push(128 + sig as i32);
                    }
                    _ => statuses.push(1),
                }
            }

            // Store PIPESTATUS array
            self.arrays.insert(
                "PIPESTATUS".to_string(),
                statuses.iter().map(|s| s.to_string()).collect(),
            );

            let status = if self.opt_pipefail {
                statuses
                    .iter()
                    .rev()
                    .find(|&&s| s != 0)
                    .copied()
                    .unwrap_or(0)
            } else {
                statuses.last().copied().unwrap_or(0)
            };

            if pipeline.negated {
                if status == 0 { 1 } else { 0 }
            } else {
                status
            }
        }

        #[cfg(not(unix))]
        {
            eprintln!("bash: pipes not supported on this platform");
            1
        }
    }

    pub fn capture_output(&mut self, cmd_str: &str) -> String {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;

            // Flush stdout before forking to prevent buffered data from being
            // inherited by the child and written to the capture pipe.
            std::io::Write::flush(&mut std::io::stdout()).ok();

            let (pipe_r, pipe_w) = match nix::unistd::pipe() {
                Ok(p) => p,
                Err(_) => return String::new(),
            };
            let pipe_r_raw = pipe_r.as_raw_fd();
            let pipe_w_raw = pipe_w.as_raw_fd();

            match unsafe { nix::unistd::fork() } {
                Ok(nix::unistd::ForkResult::Child) => {
                    drop(pipe_r);
                    nix::unistd::dup2(pipe_w_raw, 1).ok();
                    drop(pipe_w);
                    // Command substitution does not inherit errexit
                    // (unless inherit_errexit shopt is set or POSIX mode is on)
                    if !self.shopt_inherit_errexit && !self.opt_posix {
                        self.opt_errexit = false;
                    }
                    let status = self.run_string(cmd_str);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    std::process::exit(status);
                }
                Ok(nix::unistd::ForkResult::Parent { child }) => {
                    drop(pipe_w);
                    let mut output = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        match nix::unistd::read(pipe_r_raw, &mut buf) {
                            Ok(0) => break,
                            Ok(n) => output.extend_from_slice(&buf[..n]),
                            Err(_) => break,
                        }
                    }
                    drop(pipe_r);
                    if let Ok(nix::sys::wait::WaitStatus::Exited(_, code)) =
                        nix::sys::wait::waitpid(child, None)
                    {
                        self.last_status = code;
                    }
                    let mut s = String::from_utf8_lossy(&output).to_string();
                    while s.ends_with('\n') {
                        s.pop();
                    }
                    s
                }
                Err(_) => String::new(),
            }
        }
        #[cfg(not(unix))]
        {
            use std::process::Command;
            match Command::new("/proc/self/exe")
                .arg("-c")
                .arg(cmd_str)
                .output()
            {
                Ok(output) => {
                    let mut s = String::from_utf8_lossy(&output.stdout).to_string();
                    while s.ends_with('\n') {
                        s.pop();
                    }
                    s
                }
                Err(_) => String::new(),
            }
        }
    }

    fn run_command(&mut self, cmd: &Command) -> i32 {
        match cmd {
            Command::Simple(simple) => self.run_simple_command(simple),
            Command::Compound(compound, redirections) => {
                self.run_compound_with_redirects(compound, redirections)
            }
            Command::FunctionDef(name, body) => {
                if self.readonly_funcs.contains(name) {
                    eprintln!("{}: {}: readonly function", self.error_prefix(), name);
                    1
                } else {
                    self.functions.insert(name.clone(), *body.clone());
                    0
                }
            }
            Command::Coproc(name, inner_cmd) => self.run_coproc(name.as_deref(), inner_cmd),
        }
    }

    #[cfg(unix)]
    fn run_coproc(&mut self, name: Option<&str>, cmd: &Command) -> i32 {
        use nix::unistd::{ForkResult, dup2, fork, pipe};
        use std::os::unix::io::IntoRawFd;

        let coproc_name = name.unwrap_or("COPROC");

        // Close ALL previous coproc fds — bash only allows one active coproc
        // Collect all coproc-related array names and their fds
        let coproc_arrays: Vec<(String, Vec<i32>)> = self
            .arrays
            .iter()
            .filter(|(_, v)| v.len() == 2 && v.iter().all(|s| s.parse::<i32>().is_ok()))
            .filter(|(k, _)| {
                // Check if there's a corresponding _PID variable
                self.vars.contains_key(&format!("{}_PID", k))
            })
            .map(|(k, v)| (k.clone(), v.iter().filter_map(|s| s.parse().ok()).collect()))
            .collect();
        for (name, fds) in &coproc_arrays {
            for fd in fds {
                unsafe {
                    libc::close(*fd);
                }
            }
            self.arrays.remove(name);
            self.vars.remove(&format!("{}_PID", name));
        }

        // Create two pipes: one for parent→child stdin, one for child→parent stdout
        let (child_read, parent_write) = match pipe() {
            Ok(p) => (p.0.into_raw_fd(), p.1.into_raw_fd()),
            Err(e) => {
                eprintln!("{}: coproc: {}", self.error_prefix(), e);
                return 1;
            }
        };
        let (parent_read, child_write) = match pipe() {
            Ok(p) => (p.0.into_raw_fd(), p.1.into_raw_fd()),
            Err(e) => {
                eprintln!("{}: coproc: {}", self.error_prefix(), e);
                return 1;
            }
        };

        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                // Child: redirect stdin/stdout to pipes
                let _ = dup2(child_read, 0);
                let _ = dup2(child_write, 1);
                unsafe {
                    libc::close(parent_read);
                    libc::close(parent_write);
                    libc::close(child_read);
                    libc::close(child_write);
                }

                let status = self.run_command(cmd);
                std::process::exit(status);
            }
            Ok(ForkResult::Parent { child }) => {
                // Parent: close child ends, keep parent ends
                unsafe {
                    libc::close(child_read);
                    libc::close(child_write);
                }

                // Move fds to high numbers (63+) like bash does
                let high_read = unsafe { libc::fcntl(parent_read, libc::F_DUPFD, 63) };
                let high_write = unsafe { libc::fcntl(parent_write, libc::F_DUPFD, 60) };
                if high_read >= 0 {
                    unsafe { libc::close(parent_read) };
                }
                if high_write >= 0 {
                    unsafe { libc::close(parent_write) };
                }
                let read_fd = if high_read >= 0 {
                    high_read
                } else {
                    parent_read
                };
                let write_fd = if high_write >= 0 {
                    high_write
                } else {
                    parent_write
                };

                // Set COPROC array: [0]=read_fd, [1]=write_fd
                self.arrays.insert(
                    coproc_name.to_string(),
                    vec![read_fd.to_string(), write_fd.to_string()],
                );

                // Set COPROC_PID
                let pid = child.as_raw();
                self.vars
                    .insert(format!("{}_PID", coproc_name), pid.to_string());

                // Track the background job
                self.last_bg_pid = pid;

                0
            }
            Err(e) => {
                eprintln!("{}: coproc: fork: {}", self.error_prefix(), e);
                1
            }
        }
    }

    #[cfg(not(unix))]
    fn run_coproc(&mut self, _name: Option<&str>, _cmd: &Command) -> i32 {
        eprintln!(
            "{}: coproc: not supported on this platform",
            self.error_prefix()
        );
        1
    }

    /// Apply ${var:=default} assignments from a word's param expansions.
    fn apply_assign_defaults(&mut self, word: &Word) {
        for part in word {
            match part {
                WordPart::Param(expr) => {
                    if let ParamOp::Assign(colon, default_word) = &expr.op {
                        let resolved = self.resolve_nameref(&expr.name);
                        let val = self.vars.get(&resolved).cloned().unwrap_or_default();
                        let empty = if *colon { val.is_empty() } else { false };
                        let unset = !self.vars.contains_key(&resolved);
                        if unset || empty {
                            let raw_val = self.expand_word_single(default_word);
                            // Apply tilde expansion for := defaults
                            let default_val = if let Some(rest) = raw_val.strip_prefix('~') {
                                let home = self.vars.get("HOME").cloned().unwrap_or_default();
                                if rest.is_empty() || rest.starts_with('/') {
                                    format!("{}{}", home, rest)
                                } else {
                                    raw_val
                                }
                            } else {
                                raw_val
                            };
                            self.set_var(&expr.name, default_val);
                        }
                    }
                }
                WordPart::DoubleQuoted(parts) => {
                    // Recurse into double-quoted parts
                    self.apply_assign_defaults(parts);
                }
                _ => {}
            }
        }
    }

    /// Pre-evaluate ArithSub expressions that may have side effects (assignments).
    fn eval_arith_in_word(&mut self, word: &Word) -> Word {
        word.iter()
            .map(|part| match part {
                WordPart::ArithSub(expr) => {
                    let result = self.eval_arith_expr(expr);
                    WordPart::Literal(result.to_string())
                }
                WordPart::DoubleQuoted(parts) => {
                    WordPart::DoubleQuoted(self.eval_arith_in_word(parts))
                }
                other => other.clone(),
            })
            .collect()
    }

    pub fn expand_word_fields(&mut self, word: &Word, ifs: &str) -> Vec<String> {
        self.apply_assign_defaults(word);
        let word = self.eval_arith_in_word(word);
        let mut vars = self.vars.clone();
        self.inject_transform_attrs(&word, &mut vars);
        let arrays = self.arrays.clone();
        let assoc_arrays = self.assoc_arrays.clone();
        let namerefs = self.namerefs.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let last_bg_pid = self.last_bg_pid;
        let opt_flags = self.get_opt_flags();
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word(
            &word,
            &vars,
            &arrays,
            &assoc_arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            &opt_flags,
            ifs,
            &mut cmd_sub,
        )
    }

    /// Get the attribute string for a variable (for ${var@a})
    fn get_var_attrs(&self, name: &str) -> String {
        let resolved = self.resolve_nameref(name);
        let mut attrs = String::new();
        if self.arrays.contains_key(&resolved) {
            attrs.push('a');
        }
        if self.assoc_arrays.contains_key(&resolved) {
            attrs.push('A');
        }
        if self.integer_vars.contains(&resolved) {
            attrs.push('i');
        }
        if self.readonly_vars.contains(&resolved) {
            attrs.push('r');
        }
        if self.exports.contains_key(&resolved) {
            attrs.push('x');
        }
        if self.uppercase_vars.contains(&resolved) {
            attrs.push('u');
        }
        if self.lowercase_vars.contains(&resolved) {
            attrs.push('l');
        }
        if self.namerefs.contains_key(&resolved) {
            attrs.push('n');
        }
        attrs
    }

    /// Inject ${var@a} results into vars before expansion using special key
    fn inject_transform_attrs(&self, word: &Word, vars: &mut HashMap<String, String>) {
        fn scan_parts(
            parts: &[WordPart],
            shell: &crate::interpreter::Shell,
            vars: &mut HashMap<String, String>,
        ) {
            for part in parts {
                if let WordPart::Param(expr) = part
                    && let crate::ast::ParamOp::Transform(ch) = &expr.op
                    && matches!(ch, 'a' | 'A')
                {
                    let attrs = shell.get_var_attrs(&expr.name);
                    vars.insert(format!("__ATTRS__{}", expr.name), attrs);
                }
                if let WordPart::DoubleQuoted(inner) = part {
                    scan_parts(inner, shell, vars);
                }
            }
        }
        scan_parts(word, self, vars);
    }

    pub fn expand_word_single(&mut self, word: &Word) -> String {
        self.apply_assign_defaults(word);
        let word = self.eval_arith_in_word(word);
        let mut vars = self.vars.clone();
        self.inject_transform_attrs(&word, &mut vars);
        let arrays = self.arrays.clone();
        let assoc_arrays = self.assoc_arrays.clone();
        let namerefs = self.namerefs.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let last_bg_pid = self.last_bg_pid;
        let opt_flags = self.get_opt_flags();
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word_nosplit(
            &word,
            &vars,
            &arrays,
            &assoc_arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            &opt_flags,
            &mut cmd_sub,
        )
    }

    /// Expand a word as a pattern (for case, [[ = ]]). Quoted glob chars are escaped.
    pub fn expand_word_pattern(&mut self, word: &Word) -> String {
        self.apply_assign_defaults(word);
        let word = self.eval_arith_in_word(word);
        let vars = self.vars.clone();
        let arrays = self.arrays.clone();
        let assoc_arrays = self.assoc_arrays.clone();
        let namerefs = self.namerefs.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let last_bg_pid = self.last_bg_pid;
        let opt_flags = self.get_opt_flags();
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word_pattern(
            &word,
            &vars,
            &arrays,
            &assoc_arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            &opt_flags,
            &mut cmd_sub,
        )
    }

    /// Expand tilde in assignment context: `~` at start and after `:` are expanded
    pub fn expand_assignment_tilde(&self, value: &str) -> String {
        if !value.contains('~') {
            return value.to_string();
        }
        let home = self.vars.get("HOME").cloned().unwrap_or_default();
        let mut result = String::new();
        let mut chars = value.chars().peekable();
        let mut at_start = true;
        while let Some(c) = chars.next() {
            if c == '~' && at_start {
                // Check what follows: must be / or : or end
                match chars.peek() {
                    None | Some('/') | Some(':') => {
                        result.push_str(&home);
                    }
                    _ => {
                        result.push('~');
                    }
                }
            } else if c == ':' {
                result.push(':');
                // After :, check for tilde
                if chars.peek() == Some(&'~') {
                    chars.next(); // consume ~
                    match chars.peek() {
                        None | Some('/') | Some(':') => {
                            result.push_str(&home);
                        }
                        _ => {
                            result.push('~');
                        }
                    }
                }
            } else {
                result.push(c);
            }
            at_start = false;
        }
        result
    }

    /// Write xtrace output to the appropriate fd (BASH_XTRACEFD or stderr)
    pub fn xtrace_write(&self, msg: &str) {
        let fd = self
            .vars
            .get("BASH_XTRACEFD")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(2);
        #[cfg(unix)]
        {
            use std::io::Write;
            if fd == 2 {
                let _ = writeln!(std::io::stderr(), "{}", msg);
            } else {
                use std::os::unix::io::FromRawFd;
                // Use ManuallyDrop to avoid closing the fd
                let mut f = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) });
                let _ = writeln!(f, "{}", msg);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = fd;
            eprintln!("{}", msg);
        }
    }

    /// Returns the error prefix for runtime error messages (no -c:).
    /// For scripts: "$0: line N:" ; for stdin/interactive: "bash:"
    /// Error prefix for arithmetic errors — uses _BASH_SOURCE_FILE if set.
    /// Returns the context prefix for arithmetic error messages
    fn arith_cmd_prefix(&self) -> &str {
        if self.arith_is_command {
            "((: "
        } else if self.arith_is_let {
            "let: "
        } else {
            ""
        }
    }

    fn arith_error_prefix(&self) -> String {
        let name = self
            .vars
            .get("_BASH_SOURCE_FILE")
            .or_else(|| self.positional.first())
            .map(|s| s.as_str())
            .unwrap_or("bash");
        let lineno = self
            .vars
            .get("LINENO")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        if let Some(ctx) = &self.arith_context {
            format!("{}: line {}: {}", name, lineno, ctx)
        } else {
            format!("{}: line {}", name, lineno)
        }
    }

    pub fn error_prefix(&self) -> String {
        let name = self
            .positional
            .first()
            .map(|s| s.as_str())
            .unwrap_or("bash");
        let lineno = self
            .vars
            .get("LINENO")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        if name == "bash" || name.is_empty() {
            if self.dash_c_mode && lineno > 0 {
                return format!("bash: line {}", lineno);
            }
            "bash".to_string()
        } else {
            format!("{}: line {}", name, lineno)
        }
    }

    /// Returns the error prefix for syntax/parse errors (includes -c: in -c mode).
    pub fn syntax_error_prefix(&self) -> String {
        let name = self
            .positional
            .first()
            .map(|s| s.as_str())
            .unwrap_or("bash");
        let lineno = self
            .vars
            .get("LINENO")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        if self.dash_c_mode {
            format!("{}: -c: line {}", name, lineno)
        } else if name == "bash" || name.is_empty() {
            "bash".to_string()
        } else {
            format!("{}: line {}", name, lineno)
        }
    }

    fn get_opt_flags(&self) -> String {
        let mut flags = String::new();
        if self.opt_errexit {
            flags.push('e');
        }
        if self.opt_nounset {
            flags.push('u');
        }
        if self.opt_xtrace {
            flags.push('x');
        }
        if self.opt_noglob {
            flags.push('f');
        }
        if self.opt_noclobber {
            flags.push('C');
        }
        if self.opt_noexec {
            flags.push('n');
        }
        // Always-on flags
        flags.push('h'); // hashall
        flags.push('B'); // braceexpand
        flags
    }

    /// Update the SHELLOPTS variable to reflect current set -o options
    pub fn update_shellopts(&mut self) {
        let mut opts = Vec::new();
        if self.opt_allexport {
            opts.push("allexport");
        }
        opts.push("braceexpand"); // always on
        if self.opt_errexit {
            opts.push("errexit");
        }
        opts.push("hashall"); // always on
        opts.push("interactive-comments"); // always on
        if self.opt_keyword {
            opts.push("keyword");
        }
        if self.opt_noclobber {
            opts.push("noclobber");
        }
        if self.opt_noexec {
            opts.push("noexec");
        }
        if self.opt_noglob {
            opts.push("noglob");
        }
        if self.opt_nounset {
            opts.push("nounset");
        }
        if self.opt_pipefail {
            opts.push("pipefail");
        }
        if self.opt_posix {
            opts.push("posix");
        }
        if self.opt_xtrace {
            opts.push("xtrace");
        }
        self.vars.insert("SHELLOPTS".to_string(), opts.join(":"));

        // Also update BASHOPTS (shopt options)
        let mut bashopts = Vec::new();
        // Check explicitly tracked options
        if self.shopt_expand_aliases {
            bashopts.push("expand_aliases");
        }
        if self.shopt_extglob {
            bashopts.push("extglob");
        }
        if self.shopt_globstar {
            bashopts.push("globstar");
        }
        if self.shopt_inherit_errexit {
            bashopts.push("inherit_errexit");
        }
        if self.shopt_lastpipe {
            bashopts.push("lastpipe");
        }
        if self.shopt_nocasematch {
            bashopts.push("nocasematch");
        }
        if self.shopt_nullglob {
            bashopts.push("nullglob");
        }
        // Include default-on options
        bashopts.extend_from_slice(&[
            "checkwinsize",
            "cmdhist",
            "extquote",
            "globasciiranges",
            "globskipdots",
            "interactive_comments",
            "patsub_replacement",
            "promptvars",
            "sourcepath",
        ]);
        bashopts.sort();
        self.vars.insert("BASHOPTS".to_string(), bashopts.join(":"));
    }

    fn run_simple_command(&mut self, cmd: &SimpleCommand) -> i32 {
        // Set BASH_COMMAND to the source text before expansion
        // Don't overwrite during DEBUG trap execution
        if !self.in_debug_trap {
            let mut parts = Vec::new();
            for a in &cmd.assignments {
                if a.append {
                    parts.push(format!("{}+=...", a.name));
                } else {
                    match &a.value {
                        AssignValue::Scalar(w) => {
                            parts.push(format!(
                                "{}={}",
                                a.name,
                                crate::ast::word_to_xtrace_string(w)
                            ));
                        }
                        _ => parts.push(format!("{}=...", a.name)),
                    }
                }
            }
            for w in &cmd.words {
                parts.push(crate::ast::word_to_xtrace_string(w));
            }
            if !parts.is_empty() {
                self.vars
                    .insert("BASH_COMMAND".to_string(), parts.join(" "));
            }
        }

        // Run DEBUG trap before command execution (after BASH_COMMAND is set)
        self.run_debug_trap();

        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());

        // Check if first word is an assignment builtin (for tilde expansion)
        let is_assignment_builtin = cmd.words.first().is_some_and(|w| {
            let name = self.expand_word_single(w);
            matches!(name.as_str(), "export" | "declare" | "typeset" | "local")
        });

        // Expand words, applying assignment-context tilde expansion where appropriate
        let mut expanded_words: Vec<String> = Vec::new();
        for (idx, word) in cmd.words.iter().enumerate() {
            // For assignment builtins, arguments with = should not be split
            let is_assign_arg = is_assignment_builtin && idx > 0 && {
                let raw = crate::ast::word_to_string(word);
                raw.contains('=') || raw.starts_with('-') || raw.starts_with('+')
            };
            let fields = if is_assign_arg {
                vec![self.expand_word_single(word)]
            } else {
                self.expand_word_fields(word, &ifs)
            };
            // Check for silent incomplete comsub — suppress without error
            if fields
                .iter()
                .any(|f| f == "SILENT_COMSUB" || f.contains("SILENT_COMSUB"))
            {
                return 1;
            }
            // Check for error incomplete comsub — suppress with error message
            if fields
                .iter()
                .any(|f| f == "INCOMPLETE_COMSUB" || f.contains("INCOMPLETE_COMSUB"))
            {
                let name = self
                    .vars
                    .get("_BASH_SOURCE_FILE")
                    .or_else(|| self.positional.first())
                    .map(|s| s.as_str())
                    .unwrap_or("bash");
                let lineno = self.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
                eprintln!(
                    "{}: command substitution: line {}: unexpected EOF while looking for matching `)'",
                    name, lineno
                );
                return 1;
            }
            // Check if this word has unquoted tilde (for assignment tilde expansion)
            let has_unquoted_tilde = word.iter().any(|p| {
                matches!(p, crate::ast::WordPart::Literal(s) if s.contains('~'))
                    || matches!(p, crate::ast::WordPart::Tilde(_))
            });
            let should_tilde_expand =
                has_unquoted_tilde && (is_assignment_builtin && idx > 0 || !self.opt_posix);
            if should_tilde_expand {
                for mut field in fields {
                    if let Some(eq_pos) = field.find('=') {
                        let val = &field[eq_pos + 1..];
                        let expanded = self.expand_assignment_tilde(val);
                        field = format!("{}={}", &field[..eq_pos], expanded);
                    }
                    expanded_words.push(field);
                }
            } else {
                expanded_words.extend(fields);
            }
        }

        // Check for arithmetic errors during word expansion (e.g., echo $(( 1/0 )))
        if crate::expand::take_arith_error() {
            self.last_status = 1;
            return 1;
        }

        // Handle assignments
        let saved_last_status = self.last_status;
        if !cmd.assignments.is_empty() {
            for assign in &cmd.assignments {
                if expanded_words.is_empty() || self.opt_keyword {
                    // Trace assignment
                    if self.opt_xtrace {
                        match &assign.value {
                            AssignValue::Array(elems) => {
                                let items: Vec<String> = elems
                                    .iter()
                                    .map(|e| crate::ast::word_to_xtrace_string(&e.value))
                                    .collect();
                                if assign.append {
                                    self.xtrace_write(&format!(
                                        "+ {}+=({})",
                                        assign.name,
                                        items.join(" ")
                                    ));
                                } else {
                                    self.xtrace_write(&format!(
                                        "+ {}=({})",
                                        assign.name,
                                        items.join(" ")
                                    ));
                                }
                            }
                            _ => {
                                let val = match &assign.value {
                                    AssignValue::Scalar(w) => self.expand_word_single(w),
                                    AssignValue::None => String::new(),
                                    _ => unreachable!(),
                                };
                                let qval = xtrace_quote(&val);
                                if assign.append {
                                    self.xtrace_write(&format!("+ {}+={}", assign.name, qval));
                                } else {
                                    self.xtrace_write(&format!("+ {}={}", assign.name, qval));
                                }
                            }
                        }
                    }
                    self.execute_assignment(assign);
                }
            }
        }

        // set -k: extract assignment-looking words from expanded_words
        if self.opt_keyword {
            let mut new_words = Vec::new();
            for word in expanded_words.iter() {
                if let Some(eq) = word.find('=')
                    && eq > 0
                    && word[..eq]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && word
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                {
                    let name = &word[..eq];
                    let value = &word[eq + 1..];
                    self.set_var(name, value.to_string());
                    continue;
                }
                new_words.push(word.clone());
            }
            expanded_words = new_words;
        }

        if expanded_words.is_empty() {
            // For assignment-only commands, return the status of the last
            // command substitution (if any), not 0.
            // If no command substitution ran, last_status is unchanged from
            // before the assignment — return 0 for simple assignments.
            return if self.last_status != saved_last_status {
                self.last_status
            } else {
                0
            };
        }

        // Trace
        if self.opt_xtrace {
            // Trace prefix assignments on separate lines
            for assign in &cmd.assignments {
                let val = match &assign.value {
                    AssignValue::Scalar(w) => self.expand_word_single(w),
                    AssignValue::Array(_) => String::new(),
                    AssignValue::None => String::new(),
                };
                let qval = xtrace_quote(&val);
                if assign.append {
                    self.xtrace_write(&format!("+ {}+={}", assign.name, qval));
                } else {
                    self.xtrace_write(&format!("+ {}={}", assign.name, qval));
                }
            }
            let quoted: Vec<String> = expanded_words.iter().map(|w| xtrace_quote(w)).collect();
            self.xtrace_write(&format!("+ {}", quoted.join(" ")));
        }

        // Alias expansion now happens at the lexer level (during parsing),
        // not at runtime. See Lexer::try_alias_expand().

        let command_name = &expanded_words[0];
        let args = &expanded_words[1..];

        // Set up redirections
        let saved_fds = match self.setup_redirections(&cmd.redirections) {
            Ok(fds) => fds,
            Err(e) => {
                eprintln!("{}: {}", self.error_prefix(), e);
                return 1;
            }
        };

        // Check for function
        let status = if let Some(func_body) = self.functions.get(command_name).cloned() {
            // Apply prefix assignments temporarily for function calls
            let prefix_saves: Vec<(String, Option<String>)> = cmd
                .assignments
                .iter()
                .map(|a| {
                    let v = match &a.value {
                        AssignValue::Scalar(w) => self.expand_word_single(w),
                        _ => String::new(),
                    };
                    let old = self.vars.get(&a.name).cloned();
                    if a.append {
                        if self.integer_vars.contains(&a.name) {
                            let existing = self.eval_arith_expr(old.as_deref().unwrap_or("0"));
                            let addend = self.eval_arith_expr(&v);
                            self.vars
                                .insert(a.name.clone(), (existing + addend).to_string());
                        } else {
                            let existing = old.as_deref().unwrap_or("");
                            self.vars
                                .insert(a.name.clone(), format!("{}{}", existing, v));
                        }
                    } else {
                        self.vars.insert(a.name.clone(), v);
                    }
                    (a.name.clone(), old)
                })
                .collect();

            let result = self.run_function(&func_body, command_name, args);

            // Restore prefix assignments
            for (k, old) in prefix_saves {
                match old {
                    Some(v) => {
                        self.vars.insert(k, v);
                    }
                    None => {
                        self.vars.remove(&k);
                    }
                }
            }
            result
        } else if let Some(builtin) = self.builtins.get(command_name.as_str()).copied() {
            let prefix_exports: Vec<(String, String)> = if !expanded_words.is_empty() {
                cmd.assignments
                    .iter()
                    .map(|a| {
                        let v = match &a.value {
                            AssignValue::Scalar(w) => self.expand_word_single(w),
                            _ => String::new(),
                        };
                        let val = if a.append {
                            if self.integer_vars.contains(&a.name) {
                                let existing = self.eval_arith_expr(
                                    &self.vars.get(&a.name).cloned().unwrap_or_default(),
                                );
                                let addend = self.eval_arith_expr(&v);
                                (existing + addend).to_string()
                            } else {
                                let existing = self.vars.get(&a.name).cloned().unwrap_or_default();
                                format!("{}{}", existing, v)
                            }
                        } else {
                            v
                        };
                        (a.name.clone(), val)
                    })
                    .collect()
            } else {
                vec![]
            };

            let saved: Vec<(String, Option<String>)> = prefix_exports
                .iter()
                .map(|(k, v)| {
                    let old = self.vars.get(k).cloned();
                    self.vars.insert(k.clone(), v.clone());
                    (k.clone(), old)
                })
                .collect();

            self.current_builtin = Some(command_name.clone());
            let result = builtin(self, args);
            self.current_builtin = None;

            // In POSIX mode, prefix assignments to special builtins persist
            let is_special = matches!(
                command_name.as_str(),
                "break"
                    | "."
                    | "source"
                    | "continue"
                    | "eval"
                    | "exec"
                    | "exit"
                    | "export"
                    | "readonly"
                    | "return"
                    | "set"
                    | "shift"
                    | "trap"
                    | "unset"
            );
            if !(expanded_words.is_empty() || self.opt_posix && is_special) {
                for (k, old) in saved {
                    match old {
                        Some(v) => {
                            self.vars.insert(k, v);
                        }
                        None => {
                            self.vars.remove(&k);
                        }
                    }
                }
            }

            result
        } else {
            self.run_external(command_name, &expanded_words, &cmd.assignments)
        };

        // For `exec` with no command args, don't restore redirections
        // (they should persist in the current shell)
        let is_exec_no_cmd = command_name == "exec" && args.is_empty();
        if !is_exec_no_cmd {
            self.restore_redirections(saved_fds);
        }

        // Close any file descriptors opened by process substitutions
        // in THIS command (not in nested evals/subshells)
        #[cfg(unix)]
        {
            let fds = crate::expand::take_procsub_fds();
            for fd in fds {
                nix::unistd::close(fd).ok();
            }
        }

        self.last_status = status;
        status
    }

    pub fn execute_assignment(&mut self, assign: &Assignment) {
        // Extract base name (before [subscript])
        let base_name = if let Some(bracket) = assign.name.find('[') {
            &assign.name[..bracket]
        } else {
            &assign.name
        };
        let resolved_base = self.resolve_nameref(base_name);
        // Noassign variables: silently ignore assignments
        if matches!(resolved_base.as_str(), "GROUPS" | "FUNCNAME" | "DIRSTACK") {
            return;
        }
        if self.readonly_vars.contains(&resolved_base) {
            let name = self
                .vars
                .get("_BASH_SOURCE_FILE")
                .or_else(|| self.positional.first())
                .map(|s| s.as_str())
                .unwrap_or("bash");
            let lineno = self.vars.get("LINENO").map(|s| s.as_str()).unwrap_or("0");
            eprintln!(
                "{}: line {}: {}: readonly variable",
                name, lineno, assign.name
            );
            return;
        }
        match &assign.value {
            AssignValue::None => {
                let resolved = self.resolve_nameref(&assign.name);
                self.vars.entry(resolved).or_default();
            }
            AssignValue::Scalar(w) => {
                let raw_value = self.expand_word_single(w);
                // Only apply assignment tilde expansion if word has unquoted content
                let has_unquoted_tilde = w.iter().any(|p| {
                    matches!(p, crate::ast::WordPart::Literal(s) if s.contains('~'))
                        || matches!(p, crate::ast::WordPart::Tilde(_))
                });
                let value = if has_unquoted_tilde {
                    self.expand_assignment_tilde(&raw_value)
                } else {
                    raw_value
                };
                if assign.append {
                    let resolved = self.resolve_nameref(base_name);
                    // Check if it's an array element append (x[n]+=val)
                    if let Some(bracket) = assign.name.find('[') {
                        let idx_str = &assign.name[bracket + 1..assign.name.len() - 1];
                        let raw_idx = self.eval_arith_expr(idx_str);
                        let is_int = self.integer_vars.contains(&resolved);
                        let addend = if is_int {
                            self.eval_arith_expr(&value)
                        } else {
                            0
                        };
                        if let Some(arr) = self.arrays.get_mut(&resolved) {
                            // Handle negative indices
                            let idx = if raw_idx < 0 {
                                let len = arr.len() as i64;
                                (len + raw_idx).max(0) as usize
                            } else {
                                raw_idx as usize
                            };
                            while arr.len() <= idx {
                                arr.push(String::new());
                            }
                            if is_int {
                                let existing: i64 = arr[idx].parse().unwrap_or(0);
                                arr[idx] = (existing + addend).to_string();
                            } else {
                                arr[idx].push_str(&value);
                            }
                        }
                    } else if self.assoc_arrays.contains_key(&resolved) {
                        // Associative array: foo+=val adds [0]=val
                        self.assoc_arrays
                            .entry(resolved)
                            .or_default()
                            .entry("0".to_string())
                            .and_modify(|v| v.push_str(&value))
                            .or_insert(value);
                    } else if self.arrays.contains_key(&resolved) {
                        let is_int = self.integer_vars.contains(&resolved);
                        if is_int {
                            // Integer array: arr+=val adds to element 0
                            let n = self.eval_arith_expr(&value);
                            let arr = self.arrays.entry(resolved).or_default();
                            if arr.is_empty() {
                                arr.push(n.to_string());
                            } else {
                                let existing: i64 = arr[0].parse().unwrap_or(0);
                                arr[0] = (existing + n).to_string();
                            }
                        } else {
                            // String array: arr+=val appends to element 0
                            let arr = self.arrays.entry(resolved).or_default();
                            if arr.is_empty() {
                                arr.push(value);
                            } else {
                                arr[0].push_str(&value);
                            }
                        }
                    } else if self.integer_vars.contains(&resolved) {
                        // Integer append: arithmetic addition
                        let existing_str = self.vars.get(&resolved).cloned().unwrap_or_default();
                        let existing = self.eval_arith_expr(&existing_str);
                        let addend = self.eval_arith_expr(&value);
                        self.set_var(&assign.name, (existing + addend).to_string());
                    } else {
                        let existing = self.vars.get(&resolved).cloned().unwrap_or_default();
                        self.set_var(&assign.name, format!("{}{}", existing, value));
                    }
                } else {
                    // Check for arr[n]=value or map[key]=value
                    if let Some(bracket) = assign.name.find('[') {
                        let base = &assign.name[..bracket];
                        let idx_str = &assign.name[bracket + 1..assign.name.len() - 1];

                        // BASH_ALIASES[key]=value → alias key=value
                        if base == "BASH_ALIASES" {
                            let alias_name = idx_str.to_string();
                            let invalid = alias_name.is_empty()
                                || alias_name.chars().any(|c| {
                                    matches!(
                                        c,
                                        '/' | '$'
                                            | '`'
                                            | '='
                                            | '\\'
                                            | '\''
                                            | '"'
                                            | '&'
                                            | '|'
                                            | ';'
                                            | '('
                                            | ')'
                                            | '<'
                                            | '>'
                                    )
                                });
                            if invalid {
                                eprintln!(
                                    "{}: `{}': invalid alias name",
                                    self.error_prefix(),
                                    alias_name
                                );
                            } else {
                                self.aliases.insert(alias_name, value);
                            }
                        } else {
                            let resolved = self.resolve_nameref(base);
                            // Check if it's an associative array
                            if self.assoc_arrays.contains_key(&resolved) {
                                self.assoc_arrays
                                    .entry(resolved)
                                    .or_default()
                                    .insert(idx_str.to_string(), value);
                            } else {
                                let raw_idx = self.eval_arith_expr(idx_str);
                                let arr = self.arrays.entry(resolved).or_default();
                                let idx = if raw_idx < 0 {
                                    let len = arr.len() as i64;
                                    (len + raw_idx).max(0) as usize
                                } else {
                                    raw_idx as usize
                                };
                                while arr.len() <= idx {
                                    arr.push(String::new());
                                }
                                arr[idx] = value;
                            }
                        }
                    } else {
                        self.set_var(&assign.name, value);
                    }
                }
            }
            AssignValue::Array(elements) => {
                let resolved = self.resolve_nameref(&assign.name);
                // Check if this is an associative array
                if self.assoc_arrays.contains_key(&resolved)
                    || (!assign.append
                        && elements.iter().any(|e| {
                            e.index.as_ref().is_some_and(|idx| {
                                let s = crate::ast::word_to_string(idx);
                                !s.chars().all(|c| c.is_ascii_digit())
                            })
                        }))
                {
                    let map = if assign.append {
                        self.assoc_arrays
                            .get(&resolved)
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        AssocArray::default()
                    };
                    let mut map = map;
                    for elem in elements {
                        let raw = self.expand_word_single(&elem.value);
                        if let Some(idx_word) = &elem.index {
                            let key = self.expand_word_single(idx_word);
                            map.insert(key, raw);
                        } else {
                            // foo+=(val) on assoc array → key "0"
                            map.entry("0".to_string())
                                .and_modify(|v| v.push_str(&raw))
                                .or_insert(raw);
                        }
                    }
                    self.assoc_arrays.insert(resolved, map);
                } else {
                    let mut arr = if assign.append {
                        self.arrays.get(&resolved).cloned().unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    let is_integer = self.integer_vars.contains(&resolved);
                    let mut next_idx = arr.len();
                    for elem in elements {
                        let raw = self.expand_word_single(&elem.value);
                        let value = if is_integer {
                            self.eval_arith_expr(&raw).to_string()
                        } else {
                            raw
                        };
                        if let Some(idx_word) = &elem.index {
                            let idx_str = self.expand_word_single(idx_word);
                            let idx = self.eval_arith_expr(&idx_str).max(0) as usize;
                            while arr.len() <= idx {
                                arr.push(String::new());
                            }
                            if elem.append {
                                if is_integer {
                                    let existing: i64 = arr[idx].parse().unwrap_or(0);
                                    let addend: i64 = value.parse().unwrap_or(0);
                                    arr[idx] = (existing + addend).to_string();
                                } else {
                                    arr[idx].push_str(&value);
                                }
                            } else {
                                arr[idx] = value;
                            }
                            next_idx = idx + 1;
                        } else {
                            while arr.len() <= next_idx {
                                arr.push(String::new());
                            }
                            arr[next_idx] = value;
                            next_idx += 1;
                        }
                    }
                    self.arrays.insert(resolved, arr);
                }
            }
        }
    }

    /// Evaluate an arithmetic expression and return the integer result.
    ///
    /// Find an operator in the expression at top-level (outside parentheses).
    fn find_top_level_arith_op(expr: &str, op: &str) -> Option<usize> {
        let mut paren_depth = 0i32;
        let mut bracket_depth = 0i32;
        let bytes = expr.as_bytes();
        let op_bytes = op.as_bytes();
        for i in 0..bytes.len() {
            match bytes[i] {
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                b'[' => bracket_depth += 1,
                b']' => bracket_depth -= 1,
                _ => {}
            }
            if paren_depth == 0
                && bracket_depth == 0
                && i + op_bytes.len() <= bytes.len()
                && &bytes[i..i + op_bytes.len()] == op_bytes
            {
                return Some(i);
            }
        }
        None
    }

    pub fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let is_top_level = self.arith_top_expr.is_none();
        if is_top_level {
            self.arith_top_expr = Some(expr.to_string());
        }
        let result = self.eval_arith_expr_impl(expr);
        if is_top_level {
            self.arith_top_expr = None;
        }
        result
    }

    fn eval_arith_expr_impl(&mut self, expr: &str) -> i64 {
        self.arith_depth += 1;
        let result = self.eval_arith_expr_inner(expr);
        self.arith_depth -= 1;
        result
    }

    fn eval_arith_expr_inner(&mut self, expr: &str) -> i64 {
        let expr = expr.trim_start();

        // Check recursion depth limit (bash uses 1024, but each level uses
        // significant stack space so we use a lower limit)
        if self.arith_depth > 512 {
            let var_name = expr.trim();
            eprintln!(
                "{}: {}: expression recursion level exceeded (error token is \"{}\")",
                self.arith_error_prefix(),
                var_name,
                var_name
            );
            crate::expand::set_arith_error();
            return 0;
        }

        // Expand command substitutions $(...) and parameter expansions ${...}
        // BEFORE stripping quotes, since commands inside $() need their quotes preserved
        let expanded_cs: String;
        let expr = if expr.contains('$') {
            expanded_cs = self.expand_comsubs_in_arith(expr);
            &expanded_cs
        } else {
            expr
        };

        // Strip double quotes from arith expressions (bash behavior)
        let unquoted: String;
        let expr = if expr.contains('"') {
            unquoted = expr.replace('"', "");
            &unquoted
        } else {
            expr
        };

        // Check for trailing operators (e.g., "4+" → syntax error)
        // Only check top-level expressions (arith_depth == 1 means we're at the outermost call)
        if self.arith_depth == 1 {
            let trimmed = expr.trim();
            if !trimmed.is_empty() {
                let last = trimmed.as_bytes()[trimmed.len() - 1];
                if matches!(last, b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'~')
                    && !trimmed.ends_with("++")
                    && !trimmed.ends_with("--")
                {
                    let op_char = last as char;
                    let top_expr = self
                        .arith_top_expr
                        .clone()
                        .unwrap_or_else(|| trimmed.to_string());
                    eprintln!(
                        "{}: {}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        top_expr,
                        op_char
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
            }
        }

        // Handle comma operator (only at top level, not inside parens)
        {
            let mut depth = 0i32;
            let mut last_comma = None;
            for (i, ch) in expr.char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 0 => last_comma = Some(i),
                    _ => {}
                }
            }
            if let Some(pos) = last_comma {
                self.eval_arith_expr_impl(&expr[..pos]);
                return self.eval_arith_expr_impl(&expr[pos + 1..]);
            }
        }

        // Handle assignment operators: var=, var+=, var-=, var*=, var/=, var%=,
        // var<<=, var>>=, var&=, var|=, var^=
        #[allow(clippy::type_complexity)]
        let assign_ops: &[(&str, fn(i64, i64) -> i64)] = &[
            ("<<=", |a, b| a.wrapping_shl(b as u32)),
            (">>=", |a, b| a.wrapping_shr(b as u32)),
            ("+=", |a, b| a.wrapping_add(b)),
            ("-=", |a, b| a.wrapping_sub(b)),
            ("*=", |a, b| a.wrapping_mul(b)),
            ("/=", |a, b| if b == 0 { 0 } else { a.wrapping_div(b) }),
            ("%=", |a, b| {
                if b == 0 || (a == i64::MIN && b == -1) {
                    0
                } else {
                    a.wrapping_rem(b)
                }
            }),
            ("&=", |a, b| a & b),
            ("|=", |a, b| a | b),
            ("^=", |a, b| a ^ b),
        ];

        for &(op, func) in assign_ops {
            if let Some(pos) = Self::find_top_level_arith_op(expr, op) {
                let name = expr[..pos].trim();
                if !name.is_empty()
                    && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
                    && (name.chars().all(|c| c.is_alphanumeric() || c == '_') || name.contains('['))
                {
                    let rhs = self.eval_arith_expr_impl(&expr[pos + op.len()..]);
                    // Handle array element: name[subscript]
                    if let Some(bracket) = name.find('[') {
                        let base = &name[..bracket];
                        let idx_str = &name[bracket + 1..name.len() - 1];
                        let resolved = self.resolve_nameref(base);
                        let idx = self.eval_arith_expr_impl(idx_str) as usize;
                        let arr = self.arrays.entry(resolved).or_default();
                        while arr.len() <= idx {
                            arr.push(String::new());
                        }
                        let lhs: i64 = arr[idx].parse().unwrap_or(0);
                        let result = func(lhs, rhs);
                        arr[idx] = result.to_string();
                        return result;
                    }
                    let lhs: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let result = func(lhs, rhs);
                    self.set_var(name, result.to_string());
                    return result;
                }
            }
        }

        // Handle simple assignment: var=expr (but not ==)
        if let Some(pos) = Self::find_top_level_arith_op(expr, "=")
            && pos > 0
            && !expr[..pos].ends_with('!')
            && !expr[..pos].ends_with('<')
            && !expr[..pos].ends_with('>')
            && !expr[pos + 1..].starts_with('=')
        {
            let name = expr[..pos].trim();
            if !name.is_empty()
                && name.chars().next().is_some_and(|c| !c.is_ascii_digit())
                && (name.chars().all(|c| c.is_alphanumeric() || c == '_') || name.contains('['))
            {
                let rhs = &expr[pos + 1..];
                if rhs.trim().is_empty() {
                    // Empty RHS: e.g. "j=" → syntax error
                    eprintln!(
                        "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{}\")",
                        self.arith_error_prefix(),
                        self.arith_cmd_prefix(),
                        expr,
                        &expr[pos..]
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                let val = self.eval_arith_expr_impl(rhs);
                if let Some(bracket) = name.find('[') {
                    let base = &name[..bracket];
                    let idx_str = &name[bracket + 1..name.len() - 1];
                    let resolved = self.resolve_nameref(base);
                    let idx = self.eval_arith_expr_impl(idx_str) as usize;
                    let arr = self.arrays.entry(resolved).or_default();
                    while arr.len() <= idx {
                        arr.push(String::new());
                    }
                    arr[idx] = val.to_string();
                } else {
                    self.set_var(name, val.to_string());
                }
                return val;
            }
            // Assignment to non-variable (e.g., 7=4)
            if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                eprintln!(
                    "{}: {}{}: attempted assignment to non-variable (error token is \"{}\")",
                    self.arith_error_prefix(),
                    self.arith_cmd_prefix(),
                    expr,
                    &expr[pos..]
                );
                crate::expand::set_arith_error();
                return 0;
            }
        }

        // Handle post-increment/decrement: var++, var--, arr[idx]++, arr[idx]--
        for (suffix, delta) in &[("++", 1i64), ("--", -1i64)] {
            if let Some(stripped) = expr.trim_end().strip_suffix(suffix) {
                let name = stripped.trim();
                if name.is_empty() {
                    continue;
                }
                // Check for array subscript: name[expr]
                if let Some(bracket) = name.find('[')
                    && name.ends_with(']')
                {
                    let base = &name[..bracket];
                    let idx_str = &name[bracket + 1..name.len() - 1];
                    let resolved = self.resolve_nameref(base);
                    let idx = self.eval_arith_expr(idx_str) as usize;
                    let arr = self.arrays.entry(resolved).or_default();
                    while arr.len() <= idx {
                        arr.push(String::new());
                    }
                    let val: i64 = arr[idx].parse().unwrap_or(0);
                    arr[idx] = (val + delta).to_string();
                    return val;
                }
                if name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        let op_char = if *delta > 0 { "+" } else { "-" };
                        eprintln!(
                            "{}: {}{}: arithmetic syntax error: operand expected (error token is \"{} \")",
                            self.arith_error_prefix(),
                            self.arith_cmd_prefix(),
                            expr,
                            op_char,
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    let val: i64 = self
                        .vars
                        .get(name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    self.set_var(name, (val + delta).to_string());
                    return val;
                }
            }
        }

        // Handle pre-increment/decrement: ++var, --var
        if let Some(stripped) = expr.strip_prefix("++") {
            let name = stripped.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let val: i64 = self
                    .vars
                    .get(name)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let new_val = val + 1;
                self.set_var(name, new_val.to_string());
                return new_val;
            }
        }
        if let Some(stripped) = expr.strip_prefix("--") {
            let name = stripped.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                let val: i64 = self
                    .vars
                    .get(name)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let new_val = val - 1;
                self.set_var(name, new_val.to_string());
                return new_val;
            }
        }

        // Handle ternary operator: expr ? expr : expr
        // Only evaluate the taken branch (short-circuit)
        if let Some(q_pos) = Self::find_top_level_arith_op(expr, "?") {
            let cond = self.eval_arith_expr_impl(&expr[..q_pos]);
            let rest = &expr[q_pos + 1..];
            // Find the matching ':' at top level in the rest
            if let Some(c_pos) = Self::find_top_level_arith_op(rest, ":") {
                return if cond != 0 {
                    self.eval_arith_expr_impl(&rest[..c_pos])
                } else {
                    self.eval_arith_expr_impl(&rest[c_pos + 1..])
                };
            }
        }

        // Handle || at top level (preserves assignments in subexprs)
        {
            let mut depth = 0i32;
            let bytes = expr.as_bytes();
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'|' if depth == 0 && i > 0 && bytes[i - 1] == b'|' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        if left != 0 {
                            return 1;
                        }
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if right != 0 { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle && at top level
        {
            let mut depth = 0i32;
            let bytes = expr.as_bytes();
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'&' if depth == 0 && i > 0 && bytes[i - 1] == b'&' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        if left == 0 {
                            return 0;
                        }
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if right != 0 { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise OR |  (not ||)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'|' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'|')
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'|')
                        && !(i > 0 && bytes[i - 1] == b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left | right;
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise XOR ^
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'^' if depth == 0 && !(i > 0 && bytes[i - 1] == b'=') => {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left ^ right;
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise AND & (not &&)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'&' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'&')
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'&')
                        && !(i > 0 && bytes[i - 1] == b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left & right;
                    }
                    _ => {}
                }
            }
        }

        // Handle parenthesized expressions at top level
        let trimmed_for_paren = expr.trim_end();
        if trimmed_for_paren.starts_with('(') && trimmed_for_paren.ends_with(')') {
            let mut depth = 0i32;
            let mut all_matched = true;
            for (i, ch) in trimmed_for_paren.chars().enumerate() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 && i < trimmed_for_paren.len() - 1 {
                            all_matched = false;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if all_matched {
                return self
                    .eval_arith_expr_impl(&trimmed_for_paren[1..trimmed_for_paren.len() - 1]);
            }
        }

        // Handle comparison operators at top level (right-to-left scan)
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'<' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left <= right { 1 } else { 0 };
                    }
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'>' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left >= right { 1 } else { 0 };
                    }
                    b'=' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'='
                        && !(i >= 2 && bytes[i - 2] == b'!') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left == right { 1 } else { 0 };
                    }
                    b'=' if depth == 0 && i > 0 && bytes[i - 1] == b'!' => {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left != right { 1 } else { 0 };
                    }
                    b'<' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'<')
                        && (i + 1 >= bytes.len()
                            || (bytes[i + 1] != b'=' && bytes[i + 1] != b'<')) =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left < right { 1 } else { 0 };
                    }
                    b'>' if depth == 0
                        && !(i > 0 && bytes[i - 1] == b'>')
                        && (i + 1 >= bytes.len()
                            || (bytes[i + 1] != b'=' && bytes[i + 1] != b'>')) =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return if left > right { 1 } else { 0 };
                    }
                    _ => {}
                }
            }
        }

        // Handle bitwise shift << and >>
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 1 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'<' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'<'
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left << right;
                    }
                    b'>' if depth == 0
                        && i > 0
                        && bytes[i - 1] == b'>'
                        && (i + 1 >= bytes.len() || bytes[i + 1] != b'=') =>
                    {
                        let left = self.eval_arith_expr_impl(&expr[..i - 1]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return left >> right;
                    }
                    _ => {}
                }
            }
        }

        // Handle addition/subtraction at top level
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'+' | b'-' if depth == 0 && i > 0 => {
                        // Look past whitespace to find the real previous character
                        let effective_prev = {
                            let mut j = i - 1;
                            while j > 0 && bytes[j].is_ascii_whitespace() {
                                j -= 1;
                            }
                            bytes[j]
                        };
                        let next = if i + 1 < bytes.len() {
                            bytes[i + 1]
                        } else {
                            b' '
                        };
                        // Check if prev is ++ or -- after a variable (post-increment)
                        // e.g., in "a+++4" or "a ++ + 4"
                        let is_after_postop = matches!(effective_prev, b'+' | b'-') && {
                            // Find where the effective_prev is
                            let mut j = i - 1;
                            while j > 0 && bytes[j].is_ascii_whitespace() {
                                j -= 1;
                            }
                            // j points to the second +/- of ++/--
                            // Check if there's a matching +/- before it
                            if j > 0 && bytes[j - 1] == effective_prev && j >= 2 {
                                // Skip whitespace before the ++ or --
                                let mut k = j - 2;
                                while k > 0 && bytes[k].is_ascii_whitespace() {
                                    k -= 1;
                                }
                                // Check if there's a variable name before (not a digit)
                                bytes[k].is_ascii_alphabetic()
                                    || bytes[k] == b'_'
                                    || bytes[k] == b']'
                            } else {
                                false
                            }
                        };
                        // Skip ++ or -- or after an operator (but not if after post-increment)
                        if (!matches!(
                            effective_prev,
                            b'+' | b'-'
                                | b'*'
                                | b'/'
                                | b'%'
                                | b'('
                                | b'<'
                                | b'>'
                                | b'='
                                | b'!'
                                | b'&'
                                | b'|'
                        ) || is_after_postop)
                            && (next != bytes[i] || {
                                // Allow split when ++ or -- is followed by a variable
                                // e.g., "4+++a" splits as "4" + "++a"
                                // The right side starts at i+1, the ++ is at i+1..i+3,
                                // so the variable starts at i+3 (or after any whitespace)
                                let mut after_op = i + 3;
                                while after_op < bytes.len()
                                    && bytes[after_op].is_ascii_whitespace()
                                {
                                    after_op += 1;
                                }
                                after_op < bytes.len()
                                    && (bytes[after_op].is_ascii_alphabetic()
                                        || bytes[after_op] == b'_'
                                        || bytes[after_op] == b'$'
                                        || bytes[after_op] == b'(')
                            })
                        {
                            let left = self.eval_arith_expr_impl(&expr[..i]);
                            let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                            return if bytes[i] == b'+' {
                                left.wrapping_add(right)
                            } else {
                                left.wrapping_sub(right)
                            };
                        }
                    }
                    _ => {}
                }
            }
        }

        // Handle multiplication/division/modulo at top level
        {
            let bytes = expr.as_bytes();
            let mut depth = 0i32;
            let mut i = bytes.len();
            while i > 0 {
                i -= 1;
                match bytes[i] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'*' | b'/' | b'%' if depth == 0 => {
                        if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                            continue;
                        }
                        if bytes[i] == b'*' && i > 0 && bytes[i - 1] == b'*' {
                            continue;
                        }
                        let left = self.eval_arith_expr_impl(&expr[..i]);
                        let right = self.eval_arith_expr_impl(&expr[i + 1..]);
                        return match bytes[i] {
                            b'*' => left.wrapping_mul(right),
                            b'/' => {
                                if right == 0 {
                                    let top_expr = self.arith_top_expr.as_deref().unwrap_or(expr);
                                    let error_token = expr[i + 1..].trim_start();
                                    eprintln!(
                                        "{}: {}{}: division by 0 (error token is \"{}\")",
                                        self.arith_error_prefix(),
                                        self.arith_cmd_prefix(),
                                        top_expr,
                                        error_token
                                    );
                                    crate::expand::set_arith_error();
                                    0
                                } else {
                                    left.wrapping_div(right)
                                }
                            }
                            b'%' => {
                                if right == 0 || (left == i64::MIN && right == -1) {
                                    if right != 0 {
                                        // MIN % -1 = 0 in bash
                                        0
                                    } else {
                                        let top_expr =
                                            self.arith_top_expr.as_deref().unwrap_or(expr);
                                        let error_token = expr[i + 1..].trim_start();
                                        eprintln!(
                                            "{}: {}{}: division by 0 (error token is \"{}\")",
                                            self.arith_error_prefix(),
                                            self.arith_cmd_prefix(),
                                            top_expr,
                                            error_token
                                        );
                                        crate::expand::set_arith_error();
                                        0
                                    }
                                } else {
                                    left % right
                                }
                            }
                            _ => unreachable!(),
                        };
                    }
                    _ => {}
                }
            }
        }

        // Handle exponentiation
        if let Some(pos) = Self::find_top_level_arith_op(expr, "**") {
            let base = self.eval_arith_expr_impl(&expr[..pos]);
            let exp = self.eval_arith_expr_impl(&expr[pos + 2..]);
            if exp < 0 {
                eprintln!(
                    "{}: {}: exponent less than 0 (error token is \"{}\")",
                    self.arith_error_prefix(),
                    expr.trim(),
                    &expr[pos + 2..].trim()
                );
                crate::expand::set_arith_error();
                return 0;
            }
            return base.wrapping_pow(exp as u32);
        }

        // Unary operators
        if let Some(stripped) = expr.strip_prefix('-') {
            return self.eval_arith_expr_impl(stripped).wrapping_neg();
        }
        if let Some(stripped) = expr.strip_prefix('+') {
            return self.eval_arith_expr_impl(stripped);
        }
        if let Some(stripped) = expr.strip_prefix('!') {
            return if self.eval_arith_expr_impl(stripped) == 0 {
                1
            } else {
                0
            };
        }
        if let Some(stripped) = expr.strip_prefix('~') {
            return !self.eval_arith_expr_impl(stripped);
        }

        // Variable lookup or number literal
        let expr = expr.trim();
        if expr.is_empty() {
            return 0;
        }

        // $var and ${var} reference — strip $ and treat as variable name
        if let Some(stripped) = expr.strip_prefix('$') {
            let name = stripped.trim();
            if name == "?" {
                return self.last_status as i64;
            }
            if name == "$" || name == "{$}" {
                return std::process::id() as i64;
            }
            // Handle ${var} syntax
            let name = if let Some(inner) = name.strip_prefix('{').and_then(|s| s.strip_suffix('}'))
            {
                inner
            } else {
                name
            };
            if !name.is_empty()
                && name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                let val = self.vars.get(name).cloned().unwrap_or_default();
                if val.is_empty() {
                    return 0;
                }
                if let Ok(n) = val.parse::<i64>() {
                    return n;
                }
                return self.eval_arith_expr_impl(&val);
            }
        }

        // Variable reference
        if expr
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && expr.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            let val = self.vars.get(expr).cloned().unwrap_or_default();
            if val.is_empty() {
                return 0;
            }
            // If the variable's value is itself a valid expression, evaluate it
            if let Ok(n) = val.parse::<i64>() {
                return n;
            }
            return self.eval_arith_expr_impl(&val);
        }

        // Number literal
        if let Some(hex) = expr.strip_prefix("0x").or_else(|| expr.strip_prefix("0X")) {
            return i64::from_str_radix(hex.trim(), 16).unwrap_or(0);
        }
        // Base#value notation: e.g., 8#52, 16#2a, 2#1010
        if let Some(hash_pos) = expr.find('#') {
            let base_str = &expr[..hash_pos];
            let value_str = expr[hash_pos + 1..].trim();
            if let Ok(base) = base_str.parse::<u32>() {
                if !(2..=64).contains(&base) {
                    eprintln!(
                        "{}: {}: invalid arithmetic base (error token is \"{}\")",
                        self.arith_error_prefix(),
                        expr,
                        expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                if value_str.is_empty() {
                    eprintln!(
                        "{}: {}: invalid integer constant (error token is \"{}\")",
                        self.arith_error_prefix(),
                        expr,
                        expr
                    );
                    crate::expand::set_arith_error();
                    return 0;
                }
                if base <= 36 {
                    return i64::from_str_radix(value_str, base).unwrap_or_else(|_| {
                        eprintln!(
                            "{}: {}: value too great for base (error token is \"{}\")",
                            self.arith_error_prefix(),
                            expr,
                            expr
                        );
                        crate::expand::set_arith_error();
                        0
                    });
                }
                // Bases 37-64: digits are 0-9, a-z, A-Z, @, _
                let mut result: i64 = 0;
                for ch in value_str.chars() {
                    let digit = match ch {
                        '0'..='9' => ch as u32 - '0' as u32,
                        'a'..='z' => ch as u32 - 'a' as u32 + 10,
                        'A'..='Z' => ch as u32 - 'A' as u32 + 36,
                        '@' => 62,
                        '_' => 63,
                        _ => {
                            eprintln!(
                                "{}: {}: value too great for base (error token is \"{}\")",
                                self.arith_error_prefix(),
                                expr,
                                expr
                            );
                            crate::expand::set_arith_error();
                            return 0;
                        }
                    };
                    if digit >= base {
                        eprintln!(
                            "{}: {}: value too great for base (error token is \"{}\")",
                            self.arith_error_prefix(),
                            expr,
                            expr
                        );
                        crate::expand::set_arith_error();
                        return 0;
                    }
                    result = result * base as i64 + digit as i64;
                }
                return result;
            }
        }
        if expr.starts_with('0')
            && expr.len() > 1
            && expr.chars().skip(1).all(|c| c.is_ascii_digit())
        {
            return i64::from_str_radix(&expr[1..], 8).unwrap_or(0);
        }
        if let Ok(n) = expr.parse::<i64>() {
            return n;
        }

        // Array element: arr[idx]
        if let Some(bracket) = expr.find('[') {
            let close = expr.rfind(']').unwrap_or(expr.len());
            if close <= bracket + 1 {
                return 0;
            }
            let name = &expr[..bracket];
            let idx_str = &expr[bracket + 1..close];
            let resolved = self.resolve_nameref(name);
            let idx = self.eval_arith_expr_impl(idx_str) as usize;
            if let Some(arr) = self.arrays.get(&resolved) {
                return arr.get(idx).and_then(|v| v.parse().ok()).unwrap_or(0);
            }
            return 0;
        }

        // Fall back to reporting error
        eprintln!(
            "{}: {}{}: syntax error: operand expected (error token is \"{}\")",
            self.arith_error_prefix(),
            self.arith_cmd_prefix(),
            expr,
            expr
        );
        crate::expand::set_arith_error();
        0
    }

    /// Expand command substitutions $(...) within an arithmetic expression string.
    fn expand_comsubs_in_arith(&mut self, expr: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = expr.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Handle ${ command; } funsub
            if i + 2 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' && chars[i + 2] == ' '
            {
                // Find matching }
                let mut depth = 1i32;
                let mut j = i + 2;
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
                let cmd: String = chars[i + 2..j].iter().collect();
                let output = self.capture_output(&cmd);
                result.push_str(output.trim());
                i = j + 1;
                continue;
            }
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '(' {
                // Find matching closing paren with case/esac and quote awareness
                let mut depth = 0i32;
                let mut case_depth = 0i32;
                let mut j = i + 1;
                while j < chars.len() {
                    match chars[j] {
                        '\'' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '\'' {
                                j += 1;
                            }
                            if j < chars.len() {
                                j += 1;
                            }
                            continue;
                        }
                        '"' => {
                            j += 1;
                            while j < chars.len() && chars[j] != '"' {
                                if chars[j] == '\\' && j + 1 < chars.len() {
                                    j += 1;
                                }
                                j += 1;
                            }
                            if j < chars.len() {
                                j += 1;
                            }
                            continue;
                        }
                        '(' => depth += 1,
                        ')' => {
                            if case_depth <= 0 {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                    // Track case/esac keywords
                    if chars[j].is_alphabetic() {
                        let mut word = String::new();
                        while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                            word.push(chars[j]);
                            j += 1;
                        }
                        if word == "case" {
                            case_depth += 1;
                        } else if word == "esac" {
                            case_depth -= 1;
                        }
                        continue;
                    }
                    j += 1;
                }
                // Extract the command inside $(...)
                let cmd: String = chars[i + 2..j].iter().collect();
                let output = self.capture_output(&cmd);
                result.push_str(output.trim());
                i = j + 1;
            } else if chars[i] == '$'
                && i + 1 < chars.len()
                && chars[i + 1] == '{'
                && (i + 2 >= chars.len() || chars[i + 2] != ' ')
            {
                // ${...} parameter expansion — find matching } and expand
                let start = i;
                i += 2; // skip ${
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    match chars[i] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing }
                }
                let param_text: String = chars[start..i].iter().collect();
                let expanded =
                    self.expand_word_single(&crate::lexer::parse_word_string(&param_text));
                result.push_str(&expanded);
            } else if chars[i] == '$'
                && i + 1 < chars.len()
                && matches!(chars[i + 1], '#' | '?' | '$' | '!' | '-' | '@' | '*')
            {
                // Special parameter: $#, $?, $$, $!, $-, $@, $*
                let val = match chars[i + 1] {
                    '#' => (self.positional.len().saturating_sub(1)).to_string(),
                    '?' => self.last_status.to_string(),
                    '$' => std::process::id().to_string(),
                    '!' => self.last_bg_pid.to_string(),
                    '-' => self.get_opt_flags().to_string(),
                    '@' | '*' => self.positional[1..].join(" "),
                    _ => String::new(),
                };
                result.push_str(&val);
                i += 2;
            } else if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                // Positional parameter: $0, $1, etc.
                let idx = (chars[i + 1] as u8 - b'0') as usize;
                let val = self.positional.get(idx).cloned().unwrap_or_default();
                result.push_str(&val);
                i += 2;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    fn run_function(&mut self, body: &CompoundCommand, name: &str, args: &[String]) -> i32 {
        // Check FUNCNEST limit
        if let Some(limit_str) = self.vars.get("FUNCNEST")
            && let Ok(limit) = limit_str.parse::<usize>()
            && limit > 0
            && self.func_names.len() >= limit
        {
            eprintln!(
                "{}: {}: maximum function nesting level exceeded ({})",
                self.error_prefix(),
                name,
                limit
            );
            return 1;
        }

        let saved_positional = self.positional.clone();
        let prog = self.positional.first().cloned().unwrap_or_default();
        self.positional = vec![prog];
        self.positional.extend_from_slice(args);
        self.func_names.push(name.to_string());
        self.local_scopes.push(HashMap::new());
        self.saved_opts_stack.push(None); // Will be set by `local -`
        self.arrays.insert(
            "FUNCNAME".to_string(),
            self.func_names.iter().rev().cloned().collect(),
        );
        // Set BASH_SOURCE to current source file
        let source_file = self
            .vars
            .get("_BASH_SOURCE_FILE")
            .or_else(|| self.positional.first())
            .cloned()
            .unwrap_or_default();
        let mut bash_source = vec![source_file; self.func_names.len()];
        if bash_source.is_empty() {
            bash_source.push(String::new());
        }
        self.arrays.insert("BASH_SOURCE".to_string(), bash_source);
        // Set BASH_LINENO
        let lineno = self
            .vars
            .get("LINENO")
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        let bash_lineno = vec![lineno; self.func_names.len().saturating_sub(1)];
        self.arrays.insert("BASH_LINENO".to_string(), bash_lineno);

        // Save procsub fds so inner commands don't close them
        let saved_fds = crate::expand::take_procsub_fds();

        let status = self.run_compound_command(body);

        // Run RETURN trap before restoring scope
        self.run_return_trap();

        // Restore procsub fds
        for fd in saved_fds {
            crate::expand::register_procsub_fd_pub(fd);
        }

        // Restore shell options if `local -` was used
        if let Some(Some((errexit, nounset, xtrace, noclobber, noglob, pipefail))) =
            self.saved_opts_stack.pop()
        {
            self.opt_errexit = errexit;
            self.opt_nounset = nounset;
            self.opt_xtrace = xtrace;
            self.opt_noclobber = noclobber;
            self.opt_noglob = noglob;
            self.opt_pipefail = pipefail;
        }

        // Restore local variables
        if let Some(scope) = self.local_scopes.pop() {
            for (var_name, saved) in scope {
                // Restore scalar
                match saved.scalar {
                    Some(val) => {
                        self.vars.insert(var_name.clone(), val);
                    }
                    None => {
                        self.vars.remove(&var_name);
                    }
                }
                // Restore array
                match saved.array {
                    Some(arr) => {
                        self.arrays.insert(var_name.clone(), arr);
                    }
                    None => {
                        self.arrays.remove(&var_name);
                    }
                }
                // Restore assoc array
                match saved.assoc {
                    Some(assoc) => {
                        self.assoc_arrays.insert(var_name.clone(), assoc);
                    }
                    None => {
                        self.assoc_arrays.remove(&var_name);
                    }
                }
                // Restore integer attribute
                if saved.was_integer {
                    self.integer_vars.insert(var_name);
                } else {
                    self.integer_vars.remove(&var_name);
                }
            }
        }
        self.func_names.pop();
        if self.func_names.is_empty() {
            self.arrays.remove("FUNCNAME");
        } else {
            self.arrays.insert(
                "FUNCNAME".to_string(),
                self.func_names.iter().rev().cloned().collect(),
            );
        }
        // Restore positional params but preserve $0 (BASH_ARGV0 may have changed it)
        let current_zero = self.positional.first().cloned().unwrap_or_default();
        self.positional = saved_positional;
        if !self.positional.is_empty() {
            self.positional[0] = current_zero;
        }
        self.returning = false;
        status
    }

    fn run_external(&mut self, name: &str, args: &[String], assignments: &[Assignment]) -> i32 {
        #[cfg(unix)]
        {
            use std::ffi::CString;

            let path = builtins::find_executable(name);

            match unsafe { nix::unistd::fork() } {
                Ok(nix::unistd::ForkResult::Child) => {
                    // Export shell variables to the environment
                    for (key, export_val) in &self.exports {
                        // Use current shell var value if set (more up-to-date than
                        // the export snapshot), fall back to export value
                        let value = self.vars.get(key).unwrap_or(export_val);
                        unsafe { std::env::set_var(key, value) };
                    }

                    // Apply prefix assignments AFTER exports so they take
                    // precedence over any exported values
                    for assign in assignments {
                        let v = match &assign.value {
                            AssignValue::Scalar(w) => self.expand_word_single(w),
                            _ => String::new(),
                        };
                        let value = if assign.append {
                            let existing_str = self
                                .vars
                                .get(&assign.name)
                                .cloned()
                                .or_else(|| std::env::var(&assign.name).ok())
                                .unwrap_or_default();
                            if self.integer_vars.contains(&assign.name) {
                                let existing = self.eval_arith_expr(&existing_str);
                                let addend = self.eval_arith_expr(&v);
                                (existing + addend).to_string()
                            } else {
                                format!("{}{}", existing_str, v)
                            }
                        } else {
                            v
                        };
                        unsafe { std::env::set_var(&assign.name, &value) };
                    }

                    let c_prog = match CString::new(path.as_bytes()) {
                        Ok(c) => c,
                        Err(_) => {
                            eprintln!("bash: {}: argument list contains NUL byte", name);
                            std::process::exit(1);
                        }
                    };
                    let c_args: Vec<CString> = args
                        .iter()
                        .map(|a| {
                            // Convert to raw bytes: chars U+0080..U+00FF become
                            // single bytes (matching bash's raw byte handling)
                            let raw = crate::builtins::string_to_raw_bytes(a);
                            // Strip NUL bytes (bash truncates at first NUL)
                            let truncated = match raw.iter().position(|&b| b == 0) {
                                Some(pos) => &raw[..pos],
                                None => &raw[..],
                            };
                            CString::new(truncated).unwrap_or_else(|_| CString::new("").unwrap())
                        })
                        .collect();

                    // Reset SIGPIPE to default before exec so external
                    // commands get the standard signal behavior
                    unsafe {
                        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                    }
                    match nix::unistd::execvp(&c_prog, &c_args) {
                        Ok(_) => unreachable!(),
                        Err(e) => {
                            let msg = match e {
                                nix::errno::Errno::ENOENT => {
                                    // For commands without path separator, report "command not found"
                                    // For explicit paths, report the OS error
                                    if name.contains('/') {
                                        "No such file or directory"
                                    } else {
                                        "command not found"
                                    }
                                }
                                nix::errno::Errno::EACCES => "Permission denied",
                                nix::errno::Errno::ENOEXEC => "Exec format error",
                                _ => "command not found",
                            };
                            eprintln!("{}: {}: {}", self.error_prefix(), name, msg);
                            std::process::exit(if e == nix::errno::Errno::ENOENT {
                                127
                            } else {
                                126
                            });
                        }
                    }
                }
                Ok(nix::unistd::ForkResult::Parent { child }) => {
                    match nix::sys::wait::waitpid(child, None) {
                        Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => code,
                        Ok(nix::sys::wait::WaitStatus::Signaled(_, sig, _)) => 128 + sig as i32,
                        _ => 1,
                    }
                }
                Err(e) => {
                    eprintln!("bash: fork: {}", e);
                    1
                }
            }
        }

        #[cfg(not(unix))]
        {
            match std::process::Command::new(name).args(&args[1..]).status() {
                Ok(status) => status.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("bash: {}: {}", name, e);
                    127
                }
            }
        }
    }

    fn run_compound_with_redirects(
        &mut self,
        compound: &CompoundCommand,
        redirections: &[Redirection],
    ) -> i32 {
        let saved_fds = match self.setup_redirections(redirections) {
            Ok(fds) => fds,
            Err(e) => {
                eprintln!("{}: {}", self.error_prefix(), e);
                return 1;
            }
        };

        let status = self.run_compound_command(compound);

        self.restore_redirections(saved_fds);
        status
    }

    fn run_compound_command(&mut self, cmd: &CompoundCommand) -> i32 {
        match cmd {
            CompoundCommand::BraceGroup(program) => self.run_program(program),
            CompoundCommand::Subshell(program) => {
                #[cfg(unix)]
                {
                    // Flush stdout before forking to prevent buffered data from
                    // being duplicated in the child's output.
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    match unsafe { nix::unistd::fork() } {
                        Ok(nix::unistd::ForkResult::Child) => {
                            // Increment BASH_SUBSHELL in subshell
                            let subshell: i32 = self
                                .vars
                                .get("BASH_SUBSHELL")
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            self.vars
                                .insert("BASH_SUBSHELL".to_string(), (subshell + 1).to_string());
                            // Clear inherited traps (subshells don't inherit EXIT trap)
                            self.traps.remove("EXIT");
                            self.traps.remove("0");
                            let status = self.run_program(program);
                            self.last_status = status;
                            // Run EXIT trap in subshell before exiting
                            self.run_exit_trap();
                            std::io::stdout().flush().ok();
                            std::io::stderr().flush().ok();
                            std::process::exit(status);
                        }
                        Ok(nix::unistd::ForkResult::Parent { child }) => {
                            match nix::sys::wait::waitpid(child, None) {
                                Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => code,
                                Ok(nix::sys::wait::WaitStatus::Signaled(_, sig, _)) => {
                                    128 + sig as i32
                                }
                                _ => 1,
                            }
                        }
                        Err(e) => {
                            eprintln!("bash: fork: {}", e);
                            1
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    self.run_program(program)
                }
            }
            CompoundCommand::If(clause) => self.run_if(clause),
            CompoundCommand::For(clause) => self.run_for(clause),
            CompoundCommand::ArithFor(clause) => self.run_arith_for(clause),
            CompoundCommand::While(clause) => self.run_while(clause),
            CompoundCommand::Until(clause) => self.run_until(clause),
            CompoundCommand::Case(clause) => self.run_case(clause),
            CompoundCommand::Conditional(expr) => self.run_conditional(expr),
            CompoundCommand::Arithmetic(expr) => self.run_arithmetic(expr),
        }
    }

    fn run_condition(&mut self, program: &Program) -> i32 {
        let saved = self.in_condition;
        self.in_condition = true;
        let status = self.run_program(program);
        self.in_condition = saved;
        status
    }

    fn run_if(&mut self, clause: &IfClause) -> i32 {
        let cond_status = self.run_condition(&clause.condition);
        if cond_status == 0 {
            return self.run_program(&clause.then_body);
        }

        for (elif_cond, elif_body) in &clause.elif_parts {
            let elif_status = self.run_condition(elif_cond);
            if elif_status == 0 {
                return self.run_program(elif_body);
            }
        }

        if let Some(else_body) = &clause.else_body {
            return self.run_program(else_body);
        }

        0
    }

    fn run_for(&mut self, clause: &ForClause) -> i32 {
        // Validate variable name using raw form if available
        let check_name = clause.var_raw.as_deref().unwrap_or(&clause.var);
        if !is_valid_identifier(check_name) {
            eprintln!(
                "{}: `{}': not a valid identifier",
                self.error_prefix(),
                check_name
            );
            // In POSIX mode, invalid for/select variable is a fatal error
            if self.opt_posix {
                std::process::exit(1);
            }
            return 1;
        }
        self.loop_depth += 1;
        let status = self.run_for_inner(clause);
        self.loop_depth -= 1;
        status
    }

    fn run_for_inner(&mut self, clause: &ForClause) -> i32 {
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());

        let items: Vec<String> = if let Some(words) = &clause.words {
            let mut items = Vec::new();
            for word in words {
                items.extend(self.expand_word_fields(word, &ifs));
            }
            items
        } else if self.positional.len() > 1 {
            self.positional[1..].to_vec()
        } else {
            vec![]
        };

        let mut status = 0;
        for item in items {
            if self.breaking > 0 {
                self.breaking -= 1;
                break;
            }

            // Trace for loop iteration
            if self.opt_xtrace
                && let Some(words) = &clause.words
            {
                let expanded_items: Vec<String> = words
                    .iter()
                    .flat_map(|w| self.expand_word_fields(w, &ifs))
                    .collect();
                self.xtrace_write(&format!(
                    "+ for {} in {}",
                    clause.var,
                    expanded_items.join(" ")
                ));
            }
            self.vars.insert(clause.var.clone(), item);
            // Loop body commands should not trigger errexit individually
            let saved_condition = self.in_condition;
            self.in_condition = true;
            status = self.run_program(&clause.body);
            self.in_condition = saved_condition;

            if self.returning {
                break;
            }
            if self.continuing > 0 {
                self.continuing -= 1;
                // continue to next iteration
            }
        }

        status
    }

    fn run_arith_for(&mut self, clause: &ArithForClause) -> i32 {
        self.loop_depth += 1;
        self.arith_is_command = true;
        let status = self.run_arith_for_inner(clause);
        self.arith_is_command = false;
        self.loop_depth -= 1;
        status
    }

    fn run_arith_for_inner(&mut self, clause: &ArithForClause) -> i32 {
        if !clause.init.is_empty() {
            if self.opt_xtrace {
                self.xtrace_write(&format!("+ (( {} ))", clause.init));
            }
            self.eval_arith_expr(&clause.init);
            if crate::expand::take_arith_error() {
                return 1;
            }
        }

        let mut status = 0;
        loop {
            if self.breaking > 0 {
                self.breaking -= 1;
                break;
            }

            if !clause.cond.is_empty() {
                if self.opt_xtrace {
                    self.xtrace_write(&format!("+ (( {} ))", clause.cond));
                }
                let cond_val = self.eval_arith_expr(&clause.cond);
                if crate::expand::take_arith_error() {
                    break;
                }
                if cond_val == 0 {
                    break;
                }
            }

            {
                let saved_condition = self.in_condition;
                self.in_condition = true;
                status = self.run_program(&clause.body);
                self.in_condition = saved_condition;
                if self.returning {
                    break;
                }
            }

            // Handle continue: decrement counter and skip to step
            if self.continuing > 0 {
                self.continuing -= 1;
            }

            if !clause.step.is_empty() {
                if self.opt_xtrace {
                    self.xtrace_write(&format!("+ (( {} ))", clause.step));
                }
                self.eval_arith_expr(&clause.step);
                // Break if step expression had an error (e.g., 7++)
                if crate::expand::take_arith_error() {
                    break;
                }
            }
        }
        status
    }

    fn run_while(&mut self, clause: &WhileClause) -> i32 {
        self.loop_depth += 1;
        let mut status = 0;
        loop {
            if self.breaking > 0 {
                self.breaking -= 1;
                break;
            }

            let cond_status = self.run_condition(&clause.condition);
            if cond_status != 0 {
                break;
            }

            let saved_condition = self.in_condition;
            self.in_condition = true;
            status = self.run_program(&clause.body);
            self.in_condition = saved_condition;

            if self.returning {
                break;
            }
            if self.continuing > 0 {
                self.continuing -= 1;
            }
        }
        self.loop_depth -= 1;
        status
    }

    fn run_until(&mut self, clause: &WhileClause) -> i32 {
        self.loop_depth += 1;
        let mut status = 0;
        loop {
            if self.breaking > 0 {
                self.breaking -= 1;
                break;
            }

            let cond_status = self.run_condition(&clause.condition);
            if cond_status == 0 {
                break;
            }

            if self.continuing > 0 {
                self.continuing -= 1;
                continue;
            }

            let saved_condition = self.in_condition;
            self.in_condition = true;
            status = self.run_program(&clause.body);
            self.in_condition = saved_condition;

            if self.returning {
                break;
            }
        }
        self.loop_depth -= 1;
        status
    }

    fn run_case(&mut self, clause: &CaseClause) -> i32 {
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());

        let _ = ifs;
        let saved_status = self.last_status;
        let word_expanded = self.expand_word_single(&clause.word);

        if self.opt_xtrace {
            self.xtrace_write(&format!("+ case {} in", word_expanded));
        }

        let mut i = 0;
        while i < clause.items.len() {
            let item = &clause.items[i];
            let saved_pat_status = self.last_status;
            let matched = item.patterns.iter().any(|pattern| {
                let pat_expanded = self.expand_word_pattern(pattern);
                case_pattern_match(&word_expanded, &pat_expanded)
            });

            // If expansion caused a readonly/error, skip case body
            if self.last_status != 0 && self.last_status != saved_pat_status && saved_status == 0 {
                return self.last_status;
            }

            if matched {
                let status = self.run_program(&item.body);
                match item.terminator {
                    CaseTerminator::Break => return status,
                    CaseTerminator::FallThrough => {
                        // Execute next clause(s) unconditionally (;& chains)
                        i += 1;
                        while i < clause.items.len() {
                            let next_status = self.run_program(&clause.items[i].body);
                            match clause.items[i].terminator {
                                CaseTerminator::Break => return next_status,
                                CaseTerminator::FallThrough => {
                                    i += 1;
                                    continue;
                                }
                                CaseTerminator::TestNext => {
                                    i += 1;
                                    break; // resume pattern testing
                                }
                            }
                        }
                        if i >= clause.items.len() {
                            return status;
                        }
                        continue;
                    }
                    CaseTerminator::TestNext => {
                        // Continue testing next patterns
                        i += 1;
                        continue;
                    }
                }
            }
            i += 1;
        }

        0
    }

    /// Execute `[[ conditional expression ]]`
    fn format_cond_for_xtrace(&mut self, expr: &CondExpr) -> String {
        let quote_empty = |s: String| -> String { if s.is_empty() { "''".to_string() } else { s } };
        match expr {
            CondExpr::Word(w) => quote_empty(self.expand_word_single(w)),
            CondExpr::Unary(op, w) => {
                let val = quote_empty(self.expand_word_single(w));
                format!("{} {}", op, val)
            }
            CondExpr::Binary(l, op, r) => {
                let lv = quote_empty(self.expand_word_single(l));
                let rv = quote_empty(self.expand_word_single(r));
                format!("{} {} {}", lv, op, rv)
            }
            CondExpr::Not(e) => {
                let inner = self.format_cond_for_xtrace(e);
                format!("! {}", inner)
            }
            CondExpr::And(a, b) => {
                let av = self.format_cond_for_xtrace(a);
                let bv = self.format_cond_for_xtrace(b);
                format!("{} && {}", av, bv)
            }
            CondExpr::Or(a, b) => {
                let av = self.format_cond_for_xtrace(a);
                let bv = self.format_cond_for_xtrace(b);
                format!("{} || {}", av, bv)
            }
        }
    }

    /// Expand a word for use as a regex pattern in [[ =~ ]].
    /// Unlike expand_word_single, this preserves backslashes from
    /// SingleQuoted parts (which come from \x in the source).
    fn expand_regex_pattern(&mut self, word: &Word) -> String {
        let mut result = String::new();
        for part in word {
            match part {
                WordPart::Literal(s) => result.push_str(s),
                WordPart::SingleQuoted(s) => {
                    if s.len() == 1 {
                        // Single char from backslash escaping — preserve: \. -> \.
                        result.push('\\');
                    }
                    result.push_str(s);
                }
                WordPart::Variable(name) => {
                    let val = self.vars.get(name.as_str()).cloned().unwrap_or_default();
                    result.push_str(&val);
                }
                WordPart::DoubleQuoted(parts) => {
                    for p in parts {
                        match p {
                            WordPart::Literal(s) => result.push_str(s),
                            WordPart::Variable(name) => {
                                let val = self.vars.get(name.as_str()).cloned().unwrap_or_default();
                                result.push_str(&val);
                            }
                            _ => {
                                result.push_str(&self.expand_word_single(&vec![p.clone()]));
                            }
                        }
                    }
                }
                _ => {
                    result.push_str(&self.expand_word_single(&vec![part.clone()]));
                }
            }
        }
        result
    }

    /// Check if any coproc process has exited and clean up its fds/variables
    #[cfg(unix)]
    fn reap_coprocs(&mut self) {
        use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
        use nix::unistd::Pid;

        // Find coproc arrays (they have corresponding _PID variables)
        let coproc_names: Vec<String> = self
            .vars
            .keys()
            .filter(|k| k.ends_with("_PID"))
            .map(|k| k[..k.len() - 4].to_string())
            .filter(|name| self.arrays.contains_key(name))
            .collect();

        for name in coproc_names {
            let pid_key = format!("{}_PID", name);
            if let Some(pid_str) = self.vars.get(&pid_key)
                && let Ok(pid) = pid_str.parse::<i32>()
            {
                // Check if the process has exited via waitpid (non-blocking)
                // or is already gone (kill check)
                let exited = matches!(
                    waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)),
                    Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _))
                );
                if exited {
                    // Process has exited — clean up array and PID.
                    // Don't close fds here — they may have been moved by exec
                    // to other fd numbers (e.g. exec 4<&${COPROC[0]}-).
                    // The fds will be closed when the shell exits or when
                    // explicitly closed by the script.
                    self.arrays.remove(&name);
                    self.vars.remove(&pid_key);
                }
            }
        }
    }

    /// Check if a closed fd matches a coproc fd and update the COPROC array.
    /// Like bash's coproc_checkfd().
    fn coproc_checkfd(&mut self, fd: i32) {
        let fd_str = fd.to_string();
        let coproc_names: Vec<String> = self
            .vars
            .keys()
            .filter(|k| k.ends_with("_PID"))
            .map(|k| k[..k.len() - 4].to_string())
            .filter(|name| self.arrays.contains_key(name))
            .collect();
        for name in coproc_names {
            if let Some(arr) = self.arrays.get_mut(&name) {
                let mut updated = false;
                for elem in arr.iter_mut() {
                    if *elem == fd_str {
                        *elem = "-1".to_string();
                        updated = true;
                    }
                }
                if updated {
                    // Re-export the array variable (COPROC[0], COPROC[1])
                    // This is implicit since we modified the array in place
                }
            }
        }
    }

    fn run_conditional(&mut self, expr: &CondExpr) -> i32 {
        // For And/Or, xtrace is output per sub-expression during eval_cond
        if self.opt_xtrace && !matches!(expr, CondExpr::And(_, _) | CondExpr::Or(_, _)) {
            let trace = self.format_cond_for_xtrace(expr);
            self.xtrace_write(&format!("+ [[ {} ]]", trace));
        }
        match self.eval_cond(expr) {
            Ok(true) => 0,
            Ok(false) => 1,
            Err(_) => 2,
        }
    }

    fn eval_cond(&mut self, expr: &CondExpr) -> Result<bool, ()> {
        match expr {
            CondExpr::Word(w) => {
                let s = self.expand_word_single(w);
                Ok(!s.is_empty())
            }
            CondExpr::Not(e) => self.eval_cond(e).map(|v| !v),
            CondExpr::And(a, b) => {
                if self.opt_xtrace {
                    let trace = self.format_cond_for_xtrace(a);
                    self.xtrace_write(&format!("+ [[ {} ]]", trace));
                }
                let av = self.eval_cond(a)?;
                if !av {
                    return Ok(false);
                }
                if self.opt_xtrace {
                    let trace = self.format_cond_for_xtrace(b);
                    self.xtrace_write(&format!("+ [[ {} ]]", trace));
                }
                self.eval_cond(b)
            }
            CondExpr::Or(a, b) => {
                if self.opt_xtrace {
                    let trace = self.format_cond_for_xtrace(a);
                    self.xtrace_write(&format!("+ [[ {} ]]", trace));
                }
                let av = self.eval_cond(a)?;
                if av {
                    return Ok(true);
                }
                if self.opt_xtrace {
                    let trace = self.format_cond_for_xtrace(b);
                    self.xtrace_write(&format!("+ [[ {} ]]", trace));
                }
                self.eval_cond(b)
            }
            CondExpr::Unary(op, w) => {
                let val = self.expand_word_single(w);
                // -t with non-integer returns error (status 2)
                if op == "-t" && val.parse::<i32>().is_err() {
                    eprintln!("{}: [[: {}: integer expected", self.error_prefix(), val);
                    return Err(());
                }
                Ok(self.eval_cond_unary(op, &val))
            }
            CondExpr::Binary(left, op, right) => {
                let lval = self.expand_word_single(left);
                // For = and != operators, the right side is a pattern
                if op == "=~" {
                    // For =~, check if pattern has explicit quoting (not just backslash escapes).
                    // SingleQuoted with len>1 comes from '...' quoting.
                    // SingleQuoted with len==1 comes from \x backslash escaping.
                    // DoubleQuoted always counts as quoting.
                    let is_quoted = right.iter().any(|p| match p {
                        WordPart::DoubleQuoted(_) => true,
                        WordPart::SingleQuoted(s) => s.len() > 1,
                        _ => false,
                    });
                    // For regex patterns, preserve backslashes from SingleQuoted parts
                    // (which come from backslash-escaping in the source)
                    let rval = self.expand_regex_pattern(right);
                    if is_quoted {
                        // Quoted: literal string match (not regex)
                        let matched = lval.contains(&rval);
                        if matched {
                            self.arrays
                                .insert("BASH_REMATCH".to_string(), vec![rval.clone()]);
                        } else {
                            self.arrays.insert("BASH_REMATCH".to_string(), Vec::new());
                        }
                        return Ok(matched);
                    }
                    return self.eval_cond_binary(&lval, op, &rval);
                }
                let rval = if matches!(op.as_str(), "=" | "==" | "!=") {
                    self.expand_word_pattern(right)
                } else {
                    self.expand_word_single(right)
                };
                self.eval_cond_binary(&lval, op, &rval)
            }
        }
    }

    fn eval_cond_unary(&self, op: &str, val: &str) -> bool {
        match op {
            "-n" => !val.is_empty(),
            "-z" => val.is_empty(),
            "-e" | "-a" => std::path::Path::new(val).exists(),
            "-f" => std::path::Path::new(val).is_file(),
            "-d" => std::path::Path::new(val).is_dir(),
            "-L" | "-h" => std::fs::symlink_metadata(val)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false),
            "-r" => {
                #[cfg(unix)]
                {
                    nix::unistd::access(val, nix::unistd::AccessFlags::R_OK).is_ok()
                }
                #[cfg(not(unix))]
                std::path::Path::new(val).exists()
            }
            "-w" => {
                #[cfg(unix)]
                {
                    nix::unistd::access(val, nix::unistd::AccessFlags::W_OK).is_ok()
                }
                #[cfg(not(unix))]
                std::path::Path::new(val).exists()
            }
            "-x" => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(val)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                false
            }
            "-s" => std::fs::metadata(val).map(|m| m.len() > 0).unwrap_or(false),
            #[cfg(unix)]
            "-b" => {
                use std::os::unix::fs::FileTypeExt;
                std::fs::metadata(val).is_ok_and(|m| m.file_type().is_block_device())
            }
            #[cfg(unix)]
            "-c" => {
                use std::os::unix::fs::FileTypeExt;
                std::fs::metadata(val).is_ok_and(|m| m.file_type().is_char_device())
            }
            #[cfg(unix)]
            "-p" => {
                use std::os::unix::fs::FileTypeExt;
                std::fs::metadata(val).is_ok_and(|m| m.file_type().is_fifo())
            }
            #[cfg(unix)]
            "-S" => {
                use std::os::unix::fs::FileTypeExt;
                std::fs::metadata(val).is_ok_and(|m| m.file_type().is_socket())
            }
            #[cfg(unix)]
            "-u" => {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(val).is_ok_and(|m| m.permissions().mode() & 0o4000 != 0)
            }
            #[cfg(unix)]
            "-g" => {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(val).is_ok_and(|m| m.permissions().mode() & 0o2000 != 0)
            }
            #[cfg(unix)]
            "-k" => {
                use std::os::unix::fs::PermissionsExt;
                std::fs::metadata(val).is_ok_and(|m| m.permissions().mode() & 0o1000 != 0)
            }
            #[cfg(unix)]
            "-O" => {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(val).is_ok_and(|m| m.uid() == unsafe { libc::getuid() })
            }
            #[cfg(unix)]
            "-G" => {
                use std::os::unix::fs::MetadataExt;
                std::fs::metadata(val).is_ok_and(|m| m.gid() == unsafe { libc::getgid() })
            }
            "-t" => {
                if let Ok(fd) = val.parse::<i32>() {
                    #[cfg(unix)]
                    {
                        nix::unistd::isatty(fd).unwrap_or(false)
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = fd;
                        false
                    }
                } else {
                    false // Error already handled by caller
                }
            }
            "-N" => std::path::Path::new(val).exists(),
            "-v" => {
                // Variable is set — handle name[@], name[*], name[n]
                if let Some(bracket) = val.find('[') {
                    let base = &val[..bracket];
                    let idx = &val[bracket + 1..val.len() - 1];
                    if idx == "@" || idx == "*" {
                        self.arrays.contains_key(base) || self.assoc_arrays.contains_key(base)
                    } else if let Ok(n) = idx.parse::<usize>() {
                        self.arrays
                            .get(base)
                            .is_some_and(|a| n < a.len() && !a[n].is_empty())
                    } else {
                        self.assoc_arrays
                            .get(base)
                            .is_some_and(|a| a.get(idx).is_some())
                    }
                } else {
                    self.vars.contains_key(val)
                        || self.arrays.contains_key(val)
                        || self.assoc_arrays.contains_key(val)
                }
            }
            "-R" => {
                // Variable is nameref
                self.namerefs.contains_key(val)
            }
            _ => false,
        }
    }

    fn eval_cond_binary(&mut self, left: &str, op: &str, right: &str) -> Result<bool, ()> {
        match op {
            "=" | "==" => {
                // Pattern matching (right side is a glob pattern)
                Ok(case_pattern_match(left, right))
            }
            "!=" => Ok(!case_pattern_match(left, right)),
            "<" => Ok(left < right),
            ">" => Ok(left > right),
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                fn parse_cond_int(s: &str) -> Option<i64> {
                    if s.is_empty() {
                        return Some(0);
                    }
                    s.parse::<i64>().ok()
                }
                let a = match parse_cond_int(left) {
                    Some(n) => n,
                    None => {
                        // Try arithmetic evaluation with [[ context
                        self.arith_context = Some("[[".to_string());
                        let n = self.eval_arith_expr(left);
                        self.arith_context = None;
                        if crate::expand::take_arith_error() {
                            return Ok(false);
                        }
                        n
                    }
                };
                let b = match parse_cond_int(right) {
                    Some(n) => n,
                    None => {
                        self.arith_context = Some("[[".to_string());
                        let n = self.eval_arith_expr(right);
                        self.arith_context = None;
                        if crate::expand::take_arith_error() {
                            return Ok(false);
                        }
                        n
                    }
                };
                Ok(match op {
                    "-eq" => a == b,
                    "-ne" => a != b,
                    "-lt" => a < b,
                    "-le" => a <= b,
                    "-gt" => a > b,
                    "-ge" => a >= b,
                    _ => unreachable!(),
                })
            }
            "-nt" => {
                let a = std::fs::metadata(left).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(right).and_then(|m| m.modified()).ok();
                Ok(matches!((a, b), (Some(a), Some(b)) if a > b))
            }
            "-ot" => {
                let a = std::fs::metadata(left).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(right).and_then(|m| m.modified()).ok();
                Ok(matches!((a, b), (Some(a), Some(b)) if a < b))
            }
            "-ef" => {
                // Same device and inode
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let a = std::fs::metadata(left).ok();
                    let b = std::fs::metadata(right).ok();
                    Ok(
                        matches!((a, b), (Some(a), Some(b)) if a.dev() == b.dev() && a.ino() == b.ino()),
                    )
                }
                #[cfg(not(unix))]
                Ok(false)
            }
            "=~" => {
                // Regex matching with BASH_REMATCH capture groups
                // Preprocess: convert \X (non-special escapes) to X for regex_lite
                let fixed_pattern = Self::fix_regex_escapes(right);
                let right = &fixed_pattern;
                match regex_lite::Regex::new(right) {
                    Ok(re) => {
                        if let Some(caps) = re.captures(left) {
                            let mut rematch = Vec::new();
                            for i in 0..caps.len() {
                                rematch.push(
                                    caps.get(i)
                                        .map(|m| m.as_str().to_string())
                                        .unwrap_or_default(),
                                );
                            }
                            self.arrays.insert("BASH_REMATCH".to_string(), rematch);
                            Ok(true)
                        } else {
                            self.arrays.insert("BASH_REMATCH".to_string(), Vec::new());
                            Ok(false)
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "{}: conditional binary operator: {}: {}",
                            self.error_prefix(),
                            right,
                            e
                        );
                        self.arrays.insert("BASH_REMATCH".to_string(), Vec::new());
                        Err(())
                    }
                }
            }
            _ => Ok(false),
        }
    }

    /// Convert POSIX bracket expression classes for regex_lite.
    /// In C locale: [[=X=]] → X, [[.X.]] → X (inside bracket expressions)
    fn fix_posix_bracket_classes(pattern: &str) -> String {
        let mut result = String::with_capacity(pattern.len());
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // Look for [= or [. (POSIX class inside bracket expression)
            if i + 3 < chars.len()
                && chars[i] == '['
                && (chars[i + 1] == '=' || chars[i + 1] == '.')
            {
                let delim = chars[i + 1];
                // Find closing =] or .]
                let mut found = None;
                for j in (i + 2)..chars.len().saturating_sub(1) {
                    if chars[j] == delim && chars[j + 1] == ']' {
                        found = Some(j);
                        break;
                    }
                }
                if let Some(close) = found {
                    // Extract the element name between [= and =]
                    let elem: String = chars[i + 2..close].iter().collect();
                    result.push_str(&elem);
                    i = close + 2; // skip past =] or .]
                    continue;
                }
            }
            result.push(chars[i]);
            i += 1;
        }
        result
    }

    /// Fix regex escape sequences for regex_lite compatibility.
    /// POSIX/bash regex treats `\X` where X is non-special as literal X.
    /// regex_lite rejects unknown escapes, so convert them to literal chars.
    fn fix_regex_escapes(pattern: &str) -> String {
        // First pass: convert POSIX collating elements and equivalence classes
        // In C locale: [[=X=]] → X, [[.X.]] → X
        let pattern = Self::fix_posix_bracket_classes(pattern);
        let mut result = String::with_capacity(pattern.len());
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                let next = chars[i + 1];
                let keep = match next {
                    // Regex metacharacters — keep escaped
                    '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
                    | '\\' | '/' => true,
                    // Note: \d, \w, \s etc. are NOT POSIX regex — they're Perl extensions.
                    // In bash's POSIX regex, \d means literal 'd'. Don't keep these.
                    // Whitespace escapes
                    'n' | 'r' | 't' | 'a' | 'f' | 'v' => true,
                    // Backreferences
                    '0'..='9' => true,
                    // \x only valid with following hex digits
                    'x' => i + 2 < chars.len() && chars[i + 2].is_ascii_hexdigit(),
                    // \p only valid with following { for unicode properties
                    'p' | 'P' => i + 2 < chars.len() && chars[i + 2] == '{',
                    _ => false,
                };
                if keep {
                    result.push('\\');
                    result.push(next);
                } else {
                    // Convert to literal char (escape it if it's a regex metachar)
                    result.push(next);
                }
                i += 2;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    /// Execute `(( arithmetic expression ))` — exit status 0 if result != 0.
    fn run_arithmetic(&mut self, expr: &str) -> i32 {
        self.arith_is_command = true;
        let result = self.eval_arith_expr(expr);
        self.arith_is_command = false;
        // Drain any arithmetic error flag — (( )) errors are handled by exit status
        let had_error = crate::expand::take_arith_error();
        if had_error {
            1
        } else if result != 0 {
            0
        } else {
            1
        }
    }

    pub(crate) fn io_error_message(e: &std::io::Error) -> &'static str {
        match e.kind() {
            std::io::ErrorKind::NotFound => "No such file or directory",
            std::io::ErrorKind::PermissionDenied => "Permission denied",
            std::io::ErrorKind::AlreadyExists => "File exists",
            std::io::ErrorKind::BrokenPipe => "Broken pipe",
            _ => {
                // Check raw OS error for more specific messages
                #[cfg(unix)]
                if let Some(errno) = e.raw_os_error() {
                    match errno {
                        libc::EBADF => return "Bad file descriptor",
                        libc::ENOENT => return "No such file or directory",
                        libc::EACCES => return "Permission denied",
                        libc::EISDIR => return "Is a directory",
                        libc::ENOTDIR => return "Not a directory",
                        libc::ENODEV => return "No such device or address",
                        libc::ENXIO => return "No such device or address",
                        _ => {}
                    }
                }
                "No such file or directory"
            }
        }
    }

    fn dup_error_message(src_fd: i32, e: &nix::Error) -> String {
        let msg = match *e {
            nix::Error::EBADF => "Bad file descriptor",
            nix::Error::EINVAL => "invalid value",
            _ => "Bad file descriptor",
        };
        format!("{}: {}", src_fd, msg)
    }

    #[cfg(unix)]
    fn setup_redirections(
        &mut self,
        redirections: &[Redirection],
    ) -> Result<Vec<(i32, std::os::unix::io::RawFd)>, String> {
        use std::os::unix::io::{AsRawFd, IntoRawFd};

        let mut saved = Vec::new();
        let is_var_fd = |redir: &Redirection| matches!(&redir.fd, Some(RedirFd::Var(_)));

        for redir in redirections {
            let target_str = self.expand_word_single(&redir.target);

            match &redir.kind {
                RedirectKind::Output | RedirectKind::Clobber => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if !is_var_fd(redir)
                        && let Ok(saved_fd) = nix::unistd::dup(fd)
                    {
                        saved.push((fd, saved_fd));
                    }

                    // For /dev/fd/N targets (from process substitution), dup directly
                    let raw_fd = if let Some(src_fd) = target_str
                        .strip_prefix("/dev/fd/")
                        .and_then(|s| s.parse::<i32>().ok())
                    {
                        src_fd
                    } else {
                        // Check noclobber: > cannot overwrite existing file (>| can)
                        if self.opt_noclobber
                            && matches!(redir.kind, RedirectKind::Output)
                            && std::path::Path::new(&target_str).exists()
                            && !{
                                use std::os::unix::fs::FileTypeExt;
                                std::fs::symlink_metadata(&target_str)
                                    .map(|m| {
                                        m.file_type().is_char_device() || m.file_type().is_symlink()
                                    })
                                    .unwrap_or(false)
                            }
                        {
                            return Err(format!("{}: cannot overwrite existing file", target_str));
                        }
                        std::fs::File::create(&target_str)
                            .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?
                            .into_raw_fd()
                    };
                    if raw_fd != fd {
                        nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                        nix::unistd::close(raw_fd).ok();
                    }
                }
                RedirectKind::Append => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if !is_var_fd(redir)
                        && let Ok(saved_fd) = nix::unistd::dup(fd)
                    {
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?;
                    let raw_fd = file.into_raw_fd();
                    if raw_fd != fd {
                        nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                        nix::unistd::close(raw_fd).ok();
                    }
                }
                RedirectKind::OutputAll | RedirectKind::AppendAll => {
                    // &> or &>> — redirect both stdout and stderr to file
                    // Save both fd 1 and fd 2
                    if let Ok(saved_fd1) = nix::unistd::dup(1) {
                        saved.push((1, saved_fd1));
                    }
                    if let Ok(saved_fd2) = nix::unistd::dup(2) {
                        saved.push((2, saved_fd2));
                    }

                    let raw_fd = if matches!(redir.kind, RedirectKind::AppendAll) {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&target_str)
                            .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?
                            .into_raw_fd()
                    } else {
                        std::fs::File::create(&target_str)
                            .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?
                            .into_raw_fd()
                    };
                    nix::unistd::dup2(raw_fd, 1).map_err(|e| e.to_string())?;
                    nix::unistd::dup2(raw_fd, 2).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::Input => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if !is_var_fd(redir)
                        && let Ok(saved_fd) = nix::unistd::dup(fd)
                    {
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::File::open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?;
                    let raw_fd = file.into_raw_fd();
                    if raw_fd != fd {
                        nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                        nix::unistd::close(raw_fd).ok();
                    }
                    // Clear close-on-exec flag so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                }
                RedirectKind::DupOutput => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if target_str == "-" {
                        self.coproc_checkfd(fd);
                        nix::unistd::close(fd).ok();
                    } else if let Some(src_str) = target_str.strip_suffix('-') {
                        // Move fd: dup src to fd, then close src
                        if let Ok(src_fd) = src_str.parse::<i32>() {
                            if let Ok(saved_fd) = nix::unistd::dup(fd) {
                                saved.push((fd, saved_fd));
                            }
                            nix::unistd::dup2(src_fd, fd)
                                .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if let Ok(saved_fd) = nix::unistd::dup(fd) {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                    }
                }
                RedirectKind::DupInput => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if target_str == "-" {
                        self.coproc_checkfd(fd);
                        nix::unistd::close(fd).ok();
                    } else if let Some(src_str) = target_str.strip_suffix('-') {
                        // Move fd: dup src to fd, then close src
                        if let Ok(src_fd) = src_str.parse::<i32>() {
                            if let Ok(saved_fd) = nix::unistd::dup(fd) {
                                saved.push((fd, saved_fd));
                            }
                            nix::unistd::dup2(src_fd, fd)
                                .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if let Ok(saved_fd) = nix::unistd::dup(fd) {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                    }
                }
                RedirectKind::HereDoc(_, _) | RedirectKind::HereString => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if let Ok(saved_fd) = nix::unistd::dup(fd) {
                        saved.push((fd, saved_fd));
                    }

                    // Use raw byte conversion for heredoc/herestring content
                    // so that chars like U+00CD (from $'\315') produce single bytes
                    let mut content_bytes = crate::builtins::string_to_raw_bytes(&target_str);
                    content_bytes.push(b'\n');

                    let (pipe_r, pipe_w) = nix::unistd::pipe().map_err(|e| e.to_string())?;
                    nix::unistd::write(&pipe_w, &content_bytes).map_err(|e| e.to_string())?;
                    let pipe_r_raw = pipe_r.as_raw_fd();
                    drop(pipe_w);
                    nix::unistd::dup2(pipe_r_raw, fd).map_err(|e| e.to_string())?;
                    drop(pipe_r);
                }
                RedirectKind::ReadWrite => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if let Ok(saved_fd) = nix::unistd::dup(fd) {
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, Self::io_error_message(&e)))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::ProcessSubIn | RedirectKind::ProcessSubOut => {
                    // Process substitution handled during word expansion
                }
            }
        }

        Ok(saved)
    }

    #[cfg(unix)]
    fn resolve_redir_fd(&mut self, fd: &Option<RedirFd>, default: i32) -> i32 {
        match fd {
            Some(RedirFd::Number(n)) => *n,
            Some(RedirFd::Var(name)) => {
                // Auto-allocate fd: find unused fd >= 10
                for candidate in 10..256i32 {
                    match nix::unistd::dup(candidate) {
                        Ok(dup_fd) => {
                            // fd is in use — close our dup, try next
                            nix::unistd::close(dup_fd).ok();
                        }
                        Err(_) => {
                            // fd is free — use it
                            self.vars.insert(name.clone(), candidate.to_string());
                            return candidate;
                        }
                    }
                }
                default
            }
            None => default,
        }
    }

    #[cfg(not(unix))]
    fn setup_redirections(&self, _redirections: &[Redirection]) -> Result<Vec<(i32, i32)>, String> {
        Ok(vec![])
    }

    #[cfg(unix)]
    fn restore_redirections(&self, saved: Vec<(i32, std::os::unix::io::RawFd)>) {
        for (fd, saved_fd) in saved.into_iter().rev() {
            nix::unistd::dup2(saved_fd, fd).ok();
            nix::unistd::close(saved_fd).ok();
        }
    }

    #[cfg(not(unix))]
    fn restore_redirections(&self, _saved: Vec<(i32, i32)>) {}
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

fn case_pattern_match(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    shell_pattern_match(text, pattern)
}

fn shell_pattern_match(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    pattern_match_impl(&t, 0, &p, 0)
}

/// Match *(alt1|alt2|...) — zero or more matches of any alternative
fn extglob_star_match(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    // Try matching rest directly (zero matches)
    if pattern_match_impl(text, ti, pattern, rest_pi) {
        return true;
    }
    // Try each alternative consuming some text, then recurse
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                // Matched alt from ti..end, now try * again from end
                if extglob_star_match(text, end, alts, pattern, rest_pi) {
                    return true;
                }
            }
        }
    }
    false
}

/// Match +(alt1|alt2|...) — one or more matches of any alternative
fn extglob_plus_match(
    text: &[char],
    ti: usize,
    alts: &[Vec<char>],
    pattern: &[char],
    rest_pi: usize,
) -> bool {
    // Must match at least one alternative
    for alt in alts {
        for end in ti + 1..=text.len() {
            if pattern_match_impl(&text[ti..end], 0, alt, 0) {
                // After one match, try rest (like * from here)
                if pattern_match_impl(text, end, pattern, rest_pi) {
                    return true;
                }
                // Or try more matches
                if extglob_star_match(text, end, alts, pattern, rest_pi) {
                    return true;
                }
            }
        }
    }
    false
}

/// Find the matching closing ')' for an extglob pattern, handling nesting.
fn find_extglob_close(pattern: &[char], start: usize) -> Option<usize> {
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

/// Split extglob alternatives at top-level '|' characters.
fn split_extglob_alts(pattern: &[char]) -> Vec<Vec<char>> {
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
        // Extglob: @(...), ?(...), *(...), +(...), !(...)
        if pi + 1 < pattern.len()
            && pattern[pi + 1] == '('
            && matches!(pattern[pi], '@' | '?' | '*' | '+' | '!')
        {
            let op = pattern[pi];
            if let Some(close) = find_extglob_close(pattern, pi + 2) {
                let inner: Vec<char> = pattern[pi + 2..close].to_vec();
                let rest_pi = close + 1;
                let alts = split_extglob_alts(&inner);

                match op {
                    '@' => {
                        // Exactly one match of the alternatives
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
                        // Zero or one match
                        // Try with zero matches (skip the extglob)
                        if pattern_match_impl(text, ti, pattern, rest_pi) {
                            return true;
                        }
                        // Try with one match
                        for alt in &alts {
                            let mut combined = alt.clone();
                            combined.extend_from_slice(&pattern[rest_pi..]);
                            if pattern_match_impl(text, ti, &combined, 0) {
                                return true;
                            }
                        }
                        return false;
                    }
                    '*' => {
                        return extglob_star_match(text, ti, &alts, pattern, rest_pi);
                    }
                    '+' => {
                        return extglob_plus_match(text, ti, &alts, pattern, rest_pi);
                    }
                    '!' => {
                        // Anything that doesn't match any of the alternatives
                        // Try all possible lengths of text
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
                        pi += 1; // skip backslash
                        if pattern[pi] == ch {
                            matched = true;
                        }
                        pi += 1;
                        continue;
                    }
                    // POSIX character class: [:class:]
                    if pi + 1 < pattern.len() && pattern[pi] == '[' && pattern[pi + 1] == ':' {
                        // Find closing :]
                        if let Some(end) =
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
                            pi = pi + 2 + end + 2; // skip past :]
                            continue;
                        }
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
                    // Unclosed bracket expression — treat [ as literal
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
                if pi >= pattern.len() || ti >= text.len() || text[ti] != pattern[pi] {
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
