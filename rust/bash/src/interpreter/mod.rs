mod arithmetic;
pub(crate) mod commands;
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

/// Compute the effective length of an indexed array for negative subscript resolution.
/// Returns max_assigned_index + 1 (ignoring trailing None slots), matching bash behavior.
pub fn array_effective_len(arr: &[Option<String>]) -> usize {
    arr.iter().rposition(|e| e.is_some()).map_or(0, |i| i + 1)
}

/// Check if a string is a valid bash identifier (variable name).
pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Check if a string is a valid nameref target: either a valid identifier,
/// or a valid identifier followed by `[subscript]` (array element reference).
/// Empty strings are NOT valid nameref targets.
pub fn is_valid_nameref_target(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Check for subscript: name[...]
    if let Some(bracket) = s.find('[') {
        // Must end with ]
        if !s.ends_with(']') {
            return false;
        }
        let base = &s[..bracket];
        is_valid_identifier(base)
    } else {
        is_valid_identifier(s)
    }
}

/// Saved variable state for local scope restoration
#[derive(Clone)]
pub struct SavedVar {
    pub scalar: Option<String>,
    pub array: Option<Vec<Option<String>>>,
    pub assoc: Option<AssocArray>,
    pub was_integer: bool,
    pub was_readonly: bool,
    pub was_declared_unset: bool,
    /// Saved nameref target (Some(target) if was a nameref, None if wasn't)
    pub nameref: Option<String>,
    /// Saved export value (Some(value) if was exported, None if wasn't)
    pub was_exported: Option<String>,
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
    nbuckets: usize,
    len: usize,
}

impl Default for AssocArray {
    fn default() -> Self {
        Self::new_with_buckets(1024)
    }
}

#[allow(dead_code)]
impl AssocArray {
    /// Create an AssocArray with a specific number of buckets (must be power of two).
    /// Bash uses 1024 for regular assoc arrays, 256 for BASH_CMDS (FILENAME_HASH_BUCKETS),
    /// and 64 for BASH_ALIASES (ALIAS_HASH_BUCKETS).
    pub fn new_with_buckets(nbuckets: usize) -> Self {
        Self {
            buckets: (0..nbuckets).map(|_| Vec::new()).collect(),
            nbuckets,
            len: 0,
        }
    }

    pub fn hash_key(key: &str) -> u64 {
        let mut h = BashHasher::new();
        std::hash::Hasher::write(&mut h, key.as_bytes());
        std::hash::Hasher::finish(&h)
    }

    fn bucket_idx(&self, key: &str) -> usize {
        (Self::hash_key(key) as usize) & (self.nbuckets - 1)
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        let idx = self.bucket_idx(key);
        self.buckets[idx]
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        let idx = self.bucket_idx(&key);
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
        let idx = self.bucket_idx(key);
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

    pub fn nbuckets(&self) -> usize {
        self.nbuckets
    }

    pub fn entry(&mut self, key: String) -> AssocEntry<'_> {
        let idx = self.bucket_idx(&key);
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

/// Saved shell options for `local -`: saves ALL `set -o` and `shopt -o` options
/// so they can be restored on function return.
#[derive(Debug, Clone)]
pub struct SavedOpts {
    pub opt_errexit: bool,
    pub opt_nounset: bool,
    pub opt_xtrace: bool,
    pub opt_noclobber: bool,
    pub opt_noglob: bool,
    pub opt_pipefail: bool,
    pub opt_keyword: bool,
    pub opt_hashall: bool,
    pub opt_allexport: bool,
    pub opt_monitor: bool,
    pub opt_physical: bool,
    pub opt_posix: bool,
    pub opt_noexec: bool,
    /// All shopt options (including `-o` aliases like ignoreeof, braceexpand, etc.)
    pub shopt_options: HashMap<String, bool>,
    // Dedicated shopt fields
    pub shopt_nullglob: bool,
    pub shopt_extglob: bool,
    pub shopt_globstar: bool,
    pub shopt_inherit_errexit: bool,
    pub shopt_nocasematch: bool,
    pub shopt_lastpipe: bool,
    pub shopt_expand_aliases: bool,
}

impl SavedOpts {
    /// Capture all current shell options.
    pub fn capture(shell: &Shell) -> Self {
        SavedOpts {
            opt_errexit: shell.opt_errexit,
            opt_nounset: shell.opt_nounset,
            opt_xtrace: shell.opt_xtrace,
            opt_noclobber: shell.opt_noclobber,
            opt_noglob: shell.opt_noglob,
            opt_pipefail: shell.opt_pipefail,
            opt_keyword: shell.opt_keyword,
            opt_hashall: shell.opt_hashall,
            opt_allexport: shell.opt_allexport,
            opt_monitor: shell.opt_monitor,
            opt_physical: shell.opt_physical,
            opt_posix: shell.opt_posix,
            opt_noexec: shell.opt_noexec,
            shopt_options: shell.shopt_options.clone(),
            shopt_nullglob: shell.shopt_nullglob,
            shopt_extglob: shell.shopt_extglob,
            shopt_globstar: shell.shopt_globstar,
            shopt_inherit_errexit: shell.shopt_inherit_errexit,
            shopt_nocasematch: shell.shopt_nocasematch,
            shopt_lastpipe: shell.shopt_lastpipe,
            shopt_expand_aliases: shell.shopt_expand_aliases,
        }
    }

    /// Restore all saved shell options.
    pub fn restore(self, shell: &mut Shell) {
        shell.opt_errexit = self.opt_errexit;
        shell.opt_nounset = self.opt_nounset;
        shell.opt_xtrace = self.opt_xtrace;
        shell.opt_noclobber = self.opt_noclobber;
        shell.opt_noglob = self.opt_noglob;
        shell.opt_pipefail = self.opt_pipefail;
        shell.opt_keyword = self.opt_keyword;
        shell.opt_hashall = self.opt_hashall;
        shell.opt_allexport = self.opt_allexport;
        shell.opt_monitor = self.opt_monitor;
        shell.opt_physical = self.opt_physical;
        shell.opt_posix = self.opt_posix;
        shell.opt_noexec = self.opt_noexec;
        shell.shopt_options = self.shopt_options;
        shell.shopt_nullglob = self.shopt_nullglob;
        shell.shopt_extglob = self.shopt_extglob;
        shell.shopt_globstar = self.shopt_globstar;
        shell.shopt_inherit_errexit = self.shopt_inherit_errexit;
        shell.shopt_nocasematch = self.shopt_nocasematch;
        shell.shopt_lastpipe = self.shopt_lastpipe;
        shell.shopt_expand_aliases = self.shopt_expand_aliases;

        // Update SHELLOPTS and BASHOPTS to reflect restored options.
        shell.update_shellopts();

        // Sync IGNOREEOF variable with ignoreeof shopt state.
        // When `set -o ignoreeof` is active, bash sets IGNOREEOF=10;
        // when disabled, bash removes IGNOREEOF.  Restoring via `local -`
        // must trigger the same side effects.
        let ignoreeof_on = shell
            .shopt_options
            .get("ignoreeof")
            .copied()
            .unwrap_or(false);
        if ignoreeof_on {
            shell.vars.insert("IGNOREEOF".to_string(), "10".to_string());
            if shell.exports.contains_key("IGNOREEOF") {
                shell
                    .exports
                    .insert("IGNOREEOF".to_string(), "10".to_string());
                unsafe { std::env::set_var("IGNOREEOF", "10") };
            }
        } else {
            shell.vars.remove("IGNOREEOF");
            shell.exports.remove("IGNOREEOF");
            unsafe { std::env::remove_var("IGNOREEOF") };
        }
    }
}

/// A tracked background job.
#[derive(Debug, Clone)]
pub struct Job {
    /// Job number (1-based)
    pub number: usize,
    /// Process ID of the job leader
    pub pid: i32,
    /// The command text (for display by `jobs`)
    pub command: String,
    /// Current status
    pub status: JobStatus,
}

/// Status of a background job.
#[derive(Debug, Clone, PartialEq)]
pub enum JobStatus {
    Running,
    Done(i32),
    Stopped,
}

/// Result of resolving a nameref chain for assignment, distinguishing
/// between different kinds of circularity.
#[allow(dead_code)]
pub enum NamerefResolveResult {
    /// Normal resolution — the nameref chain resolved to a final target.
    Resolved,
    /// Exact circular reference (e.g. a→b→a) — caller should assign to
    /// the enclosing scope.
    CircularExact,
    /// Circular through subscript (e.g. a→b→a[1]) — caller should remove
    /// the nameref attribute from the original variable and assign to the
    /// contained target string.
    CircularSubscript(String),
}

pub struct Shell {
    pub vars: HashMap<String, String>,
    pub exports: HashMap<String, String>,
    pub declared_unset: HashSet<String>,
    pub readonly_vars: HashSet<String>,
    pub readonly_funcs: HashSet<String>,
    pub integer_vars: HashSet<String>,
    pub uppercase_vars: HashSet<String>,
    pub lowercase_vars: HashSet<String>,
    pub capitalize_vars: HashSet<String>,
    pub arrays: HashMap<String, Vec<Option<String>>>,
    /// The source text currently being executed by `run_string`, used to display
    /// the offending source line after syntax error messages (matching bash).
    pub current_execution_input: Option<String>,
    pub assoc_arrays: HashMap<String, AssocArray>,
    pub functions: HashMap<String, CompoundCommand>,
    pub func_redirections: HashMap<String, Vec<Redirection>>, // function name → redirects
    pub func_has_keyword: HashSet<String>, // functions defined with 'function' keyword
    pub func_body_lines: HashMap<String, usize>, // function name → body start line
    pub traced_funcs: HashSet<String>,
    pub hash_table: HashMap<String, (String, u32)>,
    pub hash_order: Vec<String>,
    pub disabled_builtins: HashSet<String>,
    /// Tracked background jobs (for `jobs` builtin)
    pub jobs: Vec<Job>,
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
    /// True when executing inside a funsub (`${ ... }`) or valuesub (`${| ... }`).
    pub in_funsub: bool,
    /// Bash's `execute_for_command` resets `line_number = for_command->line`
    /// before each iteration; combined with YACC look-ahead the net effect is
    /// +1 on LINENO for body commands.  `run_complete_command` adds this value
    /// when setting LINENO, and `run_for_inner` sets/clears it.
    pub for_line_adjust: usize,
    pub in_debug_trap: bool,
    pub in_trap_handler: i32,
    pub errexit_suppressed: bool,
    pub sourcing: bool,
    pub source_set_params: bool,
    pub source_file_error: bool,
    pub cmd_end_line: Option<usize>,
    pub in_preparsed_program: bool,
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
    pub opt_physical: bool,     // set -P / set -o physical (resolve symlinks in cd/pwd)
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
    /// Fds allocated by {var} redirections that should be closed when
    /// `varredir_close` is enabled and the command completes (non-exec).
    pub varredir_close_fds: Vec<i32>,
    /// Set to true when {var} fd allocation failed (e.g. ulimit too low).
    /// The redirect handler checks this to emit a secondary target-file error.
    pub redir_alloc_failed: bool,
    pub in_pipeline_child: bool,
    pub in_foreground_wait: bool, // suppress SIGCHLD trap during foreground waits
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
    /// Whether we are currently inside a recursive subscript evaluation
    /// (e.g. `arith_subscript_key` → `eval_arith_expr_impl` for indexed arrays).
    /// When true, errors show just the subscript content without `let:`/`((:` prefix.
    pub(super) arith_in_subscript: bool,
    /// When true, `eval_arith_expr_inner` skips `strip_arith_quotes` — the
    /// caller already dequoted the expression (e.g. assignment subscripts
    /// extracted from the AST go through `dequote_subscript` which converts
    /// `\"` → `"`, so the `"` are literal arithmetic-invalid characters that
    /// must NOT be silently stripped).
    pub arith_skip_quote_strip: bool,
    /// When true, `eval_arith_expr_inner` skips `expand_comsubs_in_arith` —
    /// used when `array_expand_once` (or `assoc_expand_once`) is set and
    /// we're evaluating an array subscript that should NOT have `$(...)` expanded.
    pub arith_skip_comsub_expand: bool,
    /// Seed for RANDOM variable (bash-compatible LCRNG)
    random_seed: u32,
    /// When set, an expansion error (e.g. arithmetic syntax error in `$((...))`)
    /// occurred on this line.  Subsequent commands on the same line should be
    /// skipped (matching bash's `DISCARD` longjmp behavior).
    pub expansion_error_line: Option<usize>,
    /// For `unset` builtin: indices (into the args slice, 0-based) of arguments
    /// whose `[` came from a quoted context in the AST.  These "string"
    /// arguments need their subscript re-expanded (variable expansion only)
    /// inside `builtin_unset`, matching bash's behavior where quoted tokens
    /// are not recognized as valid array references before word expansion.
    pub unset_quoted_subscript_args: HashSet<usize>,

    pub aliases: HashMap<String, String>,
    builtins: HashMap<&'static str, BuiltinFn>,
    /// Dirty flags for dynamic assoc arrays — set when backing store changes
    pub bash_cmds_dirty: bool,
    pub bash_aliases_dirty: bool,
    /// Variables set as prefix assignments for the current function call
    /// (e.g., `VAR=val func`). These are tracked so that `local VAR` (no `=`)
    /// inside the function inherits the temp env value instead of becoming
    /// declared-but-unset.
    pub temp_env_vars: HashSet<String>,
}

impl Shell {
    /// Check status of background jobs and update their status.
    /// Removes jobs that have been reported as Done.
    pub fn reap_jobs(&mut self) {
        #[cfg(unix)]
        {
            use nix::sys::wait::{WaitStatus, waitpid};
            use nix::unistd::Pid;
            for job in self.jobs.iter_mut() {
                if job.status == JobStatus::Running {
                    match waitpid(
                        Pid::from_raw(job.pid),
                        Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                    ) {
                        Ok(WaitStatus::Exited(_, code)) => {
                            job.status = JobStatus::Done(code);
                        }
                        Ok(WaitStatus::Signaled(_, sig, _)) => {
                            job.status = JobStatus::Done(128 + sig as i32);
                        }
                        Ok(WaitStatus::StillAlive) | Err(_) => {}
                        _ => {}
                    }
                }
            }
        }
    }

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
            declared_unset: HashSet::new(),
            readonly_vars: HashSet::new(),
            readonly_funcs: HashSet::new(),
            integer_vars: HashSet::new(),
            uppercase_vars: HashSet::new(),
            lowercase_vars: HashSet::new(),
            capitalize_vars: HashSet::new(),
            arrays: HashMap::new(),
            assoc_arrays: {
                let mut m = HashMap::new();
                m.insert("BASH_ALIASES".to_string(), AssocArray::default());
                m.insert("BASH_CMDS".to_string(), AssocArray::default());
                m
            },
            functions: HashMap::new(),
            func_redirections: HashMap::new(),
            func_has_keyword: HashSet::new(),
            func_body_lines: HashMap::new(),
            traced_funcs: HashSet::new(),
            hash_table: HashMap::new(),
            hash_order: Vec::new(),
            disabled_builtins: HashSet::new(),
            jobs: Vec::new(),
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
            in_funsub: false,
            for_line_adjust: 0,
            in_debug_trap: false,
            in_trap_handler: 0,
            errexit_suppressed: false,
            sourcing: false,
            source_set_params: false,
            source_file_error: false,
            cmd_end_line: None,
            in_preparsed_program: false,
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
            opt_physical: false,
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
            varredir_close_fds: Vec::new(),
            redir_alloc_failed: false,
            in_pipeline_child: false,
            in_foreground_wait: false,
            dash_c_mode: false,
            loop_depth: 0,
            arith_top_expr: None,
            arith_context: None,
            arith_depth: 0,
            arith_is_command: false,
            arith_is_let: false,
            arith_in_subscript: false,
            arith_skip_quote_strip: false,
            arith_skip_comsub_expand: false,
            random_seed: std::process::id(),
            expansion_error_line: None,
            unset_quoted_subscript_args: HashSet::new(),
            aliases: HashMap::new(),
            builtins: builtins::builtins(),
            bash_cmds_dirty: true,
            bash_aliases_dirty: true,
            temp_env_vars: HashSet::new(),
            current_execution_input: None,
        };

        // Set up BASH_VERSINFO array (must be after struct init)
        shell.arrays.insert(
            "BASH_VERSINFO".to_string(),
            vec![
                Some("5".to_string()),
                Some("3".to_string()),
                Some("0".to_string()),
                Some("1".to_string()),
                Some("release".to_string()),
                Some(std::env::consts::ARCH.to_string()),
            ],
        );

        // Set up GROUPS array (readonly)
        #[cfg(unix)]
        {
            let gid = unsafe { libc::getgid() };
            shell
                .arrays
                .insert("GROUPS".to_string(), vec![Some(gid.to_string())]);
            // GROUPS is noassign (silently ignored, not readonly)

            // Detect privileged mode: if effective UID != real UID or effective GID != real GID
            let privileged =
                unsafe { libc::geteuid() != libc::getuid() || libc::getegid() != libc::getgid() };
            if privileged {
                shell.shopt_options.insert("privileged".to_string(), true);
            }
        }

        // Initialize builtin arrays that bash always has available
        // BASH_ARGC, BASH_ARGV, BASH_LINENO, DIRSTACK are empty arrays (=())
        // FUNCNAME is declared-but-unset (bash prints "declare -a FUNCNAME")
        shell.arrays.entry("BASH_ARGC".to_string()).or_default();
        shell.arrays.entry("BASH_ARGV".to_string()).or_default();
        shell.arrays.entry("BASH_LINENO".to_string()).or_default();
        shell.arrays.entry("DIRSTACK".to_string()).or_default();
        shell.arrays.entry("FUNCNAME".to_string()).or_default();
        shell.declared_unset.insert("FUNCNAME".to_string());
        // PIPESTATUS is set dynamically after each pipeline
        shell
            .arrays
            .entry("PIPESTATUS".to_string())
            .or_insert_with(|| vec![Some("0".to_string())]);

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
                        && let crate::ast::Command::FunctionDef {
                            name: fname,
                            body: fbody,
                            has_function_keyword,
                            redirections,
                            ..
                        } = &program[0].list.first.commands[0]
                    {
                        shell.functions.insert(fname.clone(), *fbody.clone());
                        if *has_function_keyword {
                            shell.func_has_keyword.insert(fname.clone());
                        }
                        if !redirections.is_empty() {
                            shell
                                .func_redirections
                                .insert(fname.clone(), redirections.clone());
                        }
                    }
                }
            }
            // Remove the BASH_FUNC variable from our vars
            shell.vars.remove(&key);
            shell.exports.remove(&key);
        }

        shell.update_shellopts();
        shell.readonly_vars.insert("SHELLOPTS".to_string());
        shell.readonly_vars.insert("BASHOPTS".to_string());
        shell
    }

    /// Check if array_expand_once or assoc_expand_once is enabled.
    /// When true, $(...) in array subscripts should NOT be expanded.
    pub fn is_array_expand_once(&self) -> bool {
        self.shopt_options
            .get("array_expand_once")
            .copied()
            .unwrap_or(false)
            || self
                .shopt_options
                .get("assoc_expand_once")
                .copied()
                .unwrap_or(false)
    }

    /// Rebuild BASH_CMDS associative array from the hash table.
    /// In bash, BASH_CMDS is a dynamic variable — `get_hashcmd` calls `build_hashcmd`
    /// which destroys the old assoc and rebuilds it from `hashed_filenames` each time.
    /// The assoc is created with `hashed_filenames->nbuckets` (FILENAME_HASH_BUCKETS = 256),
    /// so iteration order follows 256-bucket hashing, not the default 1024.
    pub fn rebuild_bash_cmds(&mut self) {
        const FILENAME_HASH_BUCKETS: usize = 256;
        let mut assoc = AssocArray::new_with_buckets(FILENAME_HASH_BUCKETS);
        // Iterate hash_table entries in 256-bucket order (matching bash's build_hashcmd)
        // Collect all entries with their bucket index, then insert in bucket order
        let mut entries: Vec<(usize, String, String)> = self
            .hash_table
            .iter()
            .map(|(name, (path, _))| {
                let bucket = AssocArray::hash_key(name) as usize & (FILENAME_HASH_BUCKETS - 1);
                (bucket, name.clone(), path.clone())
            })
            .collect();
        // Sort by bucket index to match bash's iteration of hashed_filenames buckets 0..N
        // Within the same bucket, bash iterates in LIFO (reverse insertion) order.
        // Since we can't perfectly replicate LIFO within buckets across HashMap,
        // we sort by bucket only (entries in same bucket are rare with 256 buckets).
        entries.sort_by_key(|(bucket, _, _)| *bucket);
        for (_, name, path) in entries {
            assoc.insert(name, path);
        }
        self.assoc_arrays.insert("BASH_CMDS".to_string(), assoc);
        self.bash_cmds_dirty = false;
    }

    /// Rebuild BASH_ALIASES associative array from the aliases HashMap.
    /// In bash, BASH_ALIASES is a dynamic variable — `get_aliasvar` calls `build_aliasvar`
    /// which rebuilds the assoc from the alias hash table (ALIAS_HASH_BUCKETS = 64).
    pub fn rebuild_bash_aliases(&mut self) {
        const ALIAS_HASH_BUCKETS: usize = 64;
        let mut assoc = AssocArray::new_with_buckets(ALIAS_HASH_BUCKETS);
        let mut entries: Vec<(usize, String, String)> = self
            .aliases
            .iter()
            .map(|(name, value)| {
                let bucket = AssocArray::hash_key(name) as usize & (ALIAS_HASH_BUCKETS - 1);
                (bucket, name.clone(), value.clone())
            })
            .collect();
        entries.sort_by_key(|(bucket, _, _)| *bucket);
        for (_, name, value) in entries {
            assoc.insert(name, value);
        }
        self.assoc_arrays.insert("BASH_ALIASES".to_string(), assoc);
        self.bash_aliases_dirty = false;
    }

    /// Rebuild dynamic associative arrays (BASH_CMDS, BASH_ALIASES) from their
    /// backing stores, but only if the backing store has changed since last rebuild.
    pub fn sync_dynamic_assoc_arrays(&mut self) {
        if self.bash_cmds_dirty {
            self.rebuild_bash_cmds();
        }
        if self.bash_aliases_dirty {
            self.rebuild_bash_aliases();
        }
    }

    /// Check if a nameref resolution is circular (the name resolves back to itself).
    /// Returns true if the nameref chain for `name` leads to a cycle.
    /// Also detects circularity through subscripted references: a→b→a[1] is
    /// circular because the base name of the target "a[1]" matches "a".
    pub fn is_circular_nameref(&self, name: &str) -> bool {
        const MAX_NAMEREF_DEPTH: usize = 8;
        let mut resolved = name.to_string();
        let mut seen = HashSet::new();
        let mut seen_bases = HashSet::new();
        seen.insert(name.to_string());
        seen_bases.insert(name.to_string());
        let mut depth = 0;
        while let Some(target) = self.namerefs.get(&resolved) {
            if target.is_empty() || depth >= MAX_NAMEREF_DEPTH {
                return false;
            }
            // Check both the full target and its base name (before '[')
            let target_base = if let Some(bracket) = target.find('[') {
                &target[..bracket]
            } else {
                target.as_str()
            };
            if seen.contains(target) || seen_bases.contains(target_base) {
                return true;
            }
            seen.insert(target.clone());
            seen_bases.insert(target_base.to_string());
            resolved = target.clone();
            depth += 1;
        }
        false
    }

    /// Resolve a nameref chain for assignment, detecting two kinds of circularity:
    /// 1. Exact circular (a→b→a): returns `CircularExact` — caller should assign
    ///    to enclosing scope.
    /// 2. Subscript circular (a→b→a[1]): returns `CircularSubscript(target)` —
    ///    caller should remove the nameref attribute and assign to `target`.
    /// 3. Normal resolution: returns `Resolved(target)`.
    pub fn resolve_nameref_for_assign(&self, name: &str) -> NamerefResolveResult {
        const MAX_NAMEREF_DEPTH: usize = 8;
        let mut resolved = name.to_string();
        let mut seen = HashSet::new();
        let mut seen_bases = HashSet::new();
        seen.insert(name.to_string());
        seen_bases.insert(name.to_string());
        let mut depth = 0;
        while let Some(target) = self.namerefs.get(&resolved) {
            if target.is_empty() || depth >= MAX_NAMEREF_DEPTH {
                break;
            }
            let target_base = if let Some(bracket) = target.find('[') {
                &target[..bracket]
            } else {
                target.as_str()
            };
            // Exact circular: target is exactly in seen (e.g. a→b→a)
            // Don't emit warnings here — let the caller fall through to
            // resolve_nameref_warn which handles warnings correctly.
            if seen.contains(target) {
                return NamerefResolveResult::CircularExact;
            }
            // Subscript circular: target's base name matches a variable
            // in the chain (e.g. a→b→a[1]) but full target is different.
            // Don't print "circular name reference" — the caller will
            // print "removing nameref attribute" instead.
            if seen_bases.contains(target_base) && !seen.contains(target) {
                return NamerefResolveResult::CircularSubscript(target.clone());
            }
            seen.insert(target.clone());
            seen_bases.insert(target_base.to_string());
            resolved = target.clone();
            depth += 1;
        }
        NamerefResolveResult::Resolved
    }

    /// Resolve a variable name through namerefs.
    /// Bash limits nameref resolution depth to 8 to prevent infinite loops.
    pub fn resolve_nameref(&self, name: &str) -> String {
        const MAX_NAMEREF_DEPTH: usize = 8;
        let mut resolved = name.to_string();
        let mut seen = HashSet::new();
        let mut depth = 0;
        while let Some(target) = self.namerefs.get(&resolved) {
            // Don't follow empty nameref targets — an empty target means
            // the nameref hasn't been bound yet (e.g. `declare -n ref`).
            if target.is_empty() || seen.contains(target) || depth >= MAX_NAMEREF_DEPTH {
                break;
            }
            seen.insert(target.clone());
            resolved = target.clone();
            depth += 1;
        }
        resolved
    }

    /// Resolve a variable name through namerefs, emitting warnings for
    /// circular references and maximum depth exceeded (matching bash behavior).
    /// Bash warns "circular name reference" when it first detects the cycle,
    /// then "maximum nameref depth (8) exceeded" when depth limit is hit.
    /// Returns the resolved name (which may be the original if circular).
    pub fn resolve_nameref_warn(&self, name: &str) -> String {
        const MAX_NAMEREF_DEPTH: usize = 8;
        let mut resolved = name.to_string();
        let mut seen = HashSet::new();
        let mut depth = 0;
        let mut warned_circular = false;
        while let Some(target) = self.namerefs.get(&resolved) {
            if target.is_empty() {
                break;
            }
            if depth >= MAX_NAMEREF_DEPTH {
                eprintln!(
                    "{}: warning: {}: maximum nameref depth ({}) exceeded",
                    self.error_prefix(),
                    name,
                    MAX_NAMEREF_DEPTH
                );
                break;
            }
            if seen.contains(target) && !warned_circular {
                eprintln!(
                    "{}: warning: {}: circular name reference",
                    self.error_prefix(),
                    name
                );
                warned_circular = true;
                // For self-references (target == resolved, e.g. v→v),
                // continue iterating up to MAX_NAMEREF_DEPTH like bash does,
                // which will then emit "maximum depth exceeded".
                // For multi-node cycles (target != resolved, e.g. v→w→x→v),
                // break immediately — bash only emits "circular name reference"
                // without continuing to max depth.
                if target != &resolved {
                    break;
                }
            }
            seen.insert(target.clone());
            resolved = target.clone();
            depth += 1;
        }
        resolved
    }

    /// Get a variable value, resolving namerefs.
    pub fn get_var(&mut self, name: &str) -> Option<String> {
        let resolved = self.resolve_nameref_warn(name);
        // When a nameref is circular (resolves back to itself) and we're
        // inside a function scope, read the value from the enclosing scope's
        // saved variable rather than the local nameref variable.
        // E.g. `function f { typeset -n ref=$1; echo $ref; }; ref=hello; f ref`
        // — the local nameref `ref` is circular, so $ref should read the
        // enclosing scope's `ref` value ("hello").
        if resolved == name
            && self.namerefs.contains_key(name)
            && self.is_circular_nameref(name)
            && !self.local_scopes.is_empty()
        {
            for scope in self.local_scopes.iter().rev() {
                if let Some(saved) = scope.get(name) {
                    return saved.scalar.clone();
                }
            }
        }
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
        // Check for subscript-circular namerefs BEFORE the empty-target check.
        // E.g. a→b→a[1]: bash removes the nameref attribute from `a` with a
        // warning and assigns to the resolved target `a[1]`.
        if self.namerefs.contains_key(name) {
            match self.resolve_nameref_for_assign(name) {
                NamerefResolveResult::CircularSubscript(target) => {
                    eprintln!(
                        "{}: warning: {}: removing nameref attribute",
                        self.error_prefix(),
                        name
                    );
                    self.namerefs.remove(name);
                    // Now assign to the resolved target (e.g. a[1]).
                    // Since the nameref was removed, this can proceed normally
                    // through the subscript handling in set_var's later code.
                    // We call set_var recursively — the nameref is gone so
                    // the chain won't be followed again.
                    if target.contains('[') && target.ends_with(']') {
                        // Target is subscripted (e.g. "a[1]") — assign to
                        // the array element directly via set_var on the base.
                        let bracket = target.find('[').unwrap();
                        let base = &target[..bracket];
                        let subscript = &target[bracket + 1..target.len() - 1];
                        if subscript != "@" && subscript != "*" {
                            self.declared_unset.remove(base);
                            let aeo = self.is_array_expand_once();
                            let expanded_sub;
                            let eval_str =
                                if !aeo && (subscript.contains('$') || subscript.contains('`')) {
                                    expanded_sub = self.expand_comsubs_in_arith(subscript);
                                    expanded_sub.as_str()
                                } else {
                                    subscript
                                };
                            let idx = self.eval_arith_expr(eval_str);
                            if crate::expand::take_arith_error() {
                                return;
                            }
                            let arr = self.arrays.entry(base.to_string()).or_default();
                            let actual_idx = if idx < 0 { 0usize } else { idx as usize };
                            while arr.len() <= actual_idx {
                                arr.push(None);
                            }
                            arr[actual_idx] = Some(value);
                            self.vars.remove(base);
                        }
                    } else {
                        // Non-subscripted target — just assign as scalar
                        self.vars.insert(target, value);
                    }
                    return;
                }
                _ => {
                    // Fall through to normal processing (exact circular and
                    // normal resolution handled below).
                }
            }
        }

        // If name is a nameref with an empty target, rebind the nameref
        // to point to `value` instead of assigning through it.
        // This matches bash: `declare -n ref; ref=x` sets ref's target to "x".
        if let Some(target) = self.namerefs.get(name)
            && target.is_empty()
        {
            // Validate that the new target is a valid variable name
            // (optionally with [subscript]). Bash rejects invalid names
            // like `/`, `%`, `42`, empty string, etc. with
            // "not a valid identifier".
            if !is_valid_nameref_target(&value) {
                // Include context prefix: arithmetic ((:, let:) or builtin name
                let prefix = if self.arith_is_command {
                    "((: ".to_string()
                } else if self.arith_is_let {
                    "let: ".to_string()
                } else if let Some(ref builtin) = self.current_builtin {
                    format!("{}: ", builtin)
                } else {
                    String::new()
                };
                eprintln!(
                    "{}: {}`{}': not a valid identifier",
                    self.error_prefix(),
                    prefix,
                    value
                );
                self.last_status = 1;
                return;
            }
            self.namerefs.insert(name.to_string(), value);
            return;
        }
        let resolved = self.resolve_nameref_warn(name);
        // Reject assignment through nameref to var[@] or var[*]
        if self.namerefs.contains_key(name) && resolved.contains('[') && resolved.ends_with(']') {
            let bracket = resolved.find('[').unwrap();
            let subscript = &resolved[bracket + 1..resolved.len() - 1];
            if subscript == "@" || subscript == "*" {
                eprintln!(
                    "{}: {}[{}]: bad array subscript",
                    self.error_prefix(),
                    &resolved[..bracket],
                    subscript
                );
                self.last_status = 1;
                return;
            }
        }
        // When a nameref is circular (resolves back to itself) and we're
        // inside a function scope, bash assigns to the variable at the
        // enclosing scope rather than the local nameref variable.
        // E.g. `function f { typeset -n v=$1; v=inside; }; v=global; f v`
        // — the local nameref `v` points to `v` (circular), so bash
        // assigns `inside` to the global `v`.
        if resolved == name
            && self.namerefs.contains_key(name)
            && self.is_circular_nameref(name)
            && !self.local_scopes.is_empty()
        {
            // Compute the final value BEFORE iterating over scopes to
            // avoid borrow-checker conflicts with `self`.
            let final_value = if self.integer_vars.contains(name) {
                self.eval_arith_expr(&value).to_string()
            } else if self.uppercase_vars.contains(name) {
                value.to_uppercase()
            } else if self.lowercase_vars.contains(name) {
                value.to_lowercase()
            } else {
                value.clone()
            };
            let has_export = self.exports.contains_key(name);
            // Find the saved scope entry for this variable and update it
            // so the value propagates to the enclosing scope on function exit.
            // Walk scopes from innermost to outermost looking for a saved entry.
            let mut found_scope = false;
            for scope in self.local_scopes.iter_mut().rev() {
                if let Some(saved) = scope.get_mut(name) {
                    saved.scalar = Some(final_value.clone());
                    // Also update exports if the variable was exported
                    if saved.was_exported.is_some() {
                        saved.was_exported = Some(final_value.clone());
                    }
                    found_scope = true;
                    break;
                }
            }
            if found_scope {
                // Also update self.vars so that reads within the function
                // (via ctx.vars.get in expansion code) see the updated
                // value immediately.
                self.vars.insert(name.to_string(), final_value.clone());
                if has_export {
                    self.exports.insert(name.to_string(), final_value.clone());
                    unsafe { std::env::set_var(name, &final_value) };
                }
                return;
            }
            // No saved scope entry — fall through to normal assignment
            // (this shouldn't normally happen for circular namerefs in
            // function scope, but handle it gracefully).
        }
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
            crate::expand::seed_random(seed);
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
        // IGNOREEOF: setting this variable enables the ignoreeof option
        // (matching bash behavior where `IGNOREEOF=N` enables ignoreeof).
        if resolved == "IGNOREEOF" {
            self.shopt_options.insert("ignoreeof".to_string(), true);
        }
        // Auto-export when set -a (allexport) is active
        if self.opt_allexport && !self.exports.contains_key(&resolved) {
            self.exports.insert(resolved.clone(), value.clone());
            if !resolved.is_empty() {
                unsafe { std::env::set_var(&resolved, &value) };
            }
        }
        // If the resolved name contains a subscript (e.g. nameref target is
        // "var[0]"), parse it and assign to the array element — but ONLY when
        // the base variable already exists as an array or assoc array.
        // When the base doesn't exist as an array, fall through to create a
        // scalar named "var[123]" (matching nix bash behavior).
        if let Some(bracket) = resolved.find('[')
            && resolved.ends_with(']')
        {
            let base = &resolved[..bracket];
            let subscript = &resolved[bracket + 1..resolved.len() - 1];
            if self.assoc_arrays.contains_key(base) {
                let key = subscript.to_string();
                self.declared_unset.remove(base);
                self.assoc_arrays
                    .entry(base.to_string())
                    .or_default()
                    .insert(key, value);
                return;
            } else if self.arrays.contains_key(base) {
                // Base exists as an indexed array — assign to element.
                self.declared_unset.remove(base);
                let aeo = self.is_array_expand_once();
                let expanded_sub;
                let eval_str = if !aeo && (subscript.contains('$') || subscript.contains('`')) {
                    expanded_sub = self.expand_comsubs_in_arith(subscript);
                    expanded_sub.as_str()
                } else {
                    subscript
                };
                let idx = self.eval_arith_expr(eval_str);
                if crate::expand::take_arith_error() {
                    return;
                }
                let arr = self.arrays.entry(base.to_string()).or_default();
                let eff_len = array_effective_len(arr) as i64;
                let actual_idx = if idx < 0 {
                    let computed = eff_len + idx;
                    if computed < 0 {
                        0usize
                    } else {
                        computed as usize
                    }
                } else {
                    idx as usize
                };
                while arr.len() <= actual_idx {
                    arr.push(None);
                }
                arr[actual_idx] = Some(value);
                return;
            }
            // Base is not an array — create an indexed array and assign
            // to the specified element (matching bash behavior where
            // nameref targets like "var[123]" auto-create arrays).
            // Skip for @ and * subscripts — these are special array
            // references, not element indices.
            // Also skip when the base name is itself a nameref — this
            // indicates a circular nameref (e.g. a→b→a[1]) and creating
            // an array would conflict with the nameref attribute.
            if subscript != "@" && subscript != "*" && !self.namerefs.contains_key(base) {
                self.declared_unset.remove(base);
                let aeo = self.is_array_expand_once();
                let expanded_sub;
                let eval_str = if !aeo && (subscript.contains('$') || subscript.contains('`')) {
                    expanded_sub = self.expand_comsubs_in_arith(subscript);
                    expanded_sub.as_str()
                } else {
                    subscript
                };
                let idx = self.eval_arith_expr(eval_str);
                if crate::expand::take_arith_error() {
                    return;
                }
                let arr = self.arrays.entry(base.to_string()).or_default();
                let actual_idx = if idx < 0 { 0usize } else { idx as usize };
                while arr.len() <= actual_idx {
                    arr.push(None);
                }
                arr[actual_idx] = Some(value);
                // Remove any scalar with the same base name
                self.vars.remove(base);
                return;
            }
        }
        self.declared_unset.remove(&resolved);
        // If the variable is an existing indexed array, assign to element [0]
        // instead of creating a separate scalar entry (bash behavior:
        // `declare -a x; x=val` sets x[0]=val, not a scalar x)
        if self.arrays.contains_key(&resolved) {
            let arr = self.arrays.get_mut(&resolved).unwrap();
            if arr.is_empty() {
                arr.push(Some(value));
            } else {
                arr[0] = Some(value);
            }
        } else {
            self.vars.insert(resolved, value);
        }
    }

    /// Apply case-modification attributes (uppercase/lowercase/capitalize) to a value.
    /// Used for both scalar and array element assignments to ensure `-u`, `-l`, `-c`
    /// attributes are respected.
    pub fn apply_case_attrs(&self, name: &str, value: String) -> String {
        let resolved = self.resolve_nameref(name);
        if self.uppercase_vars.contains(&resolved) {
            value.to_uppercase()
        } else if self.lowercase_vars.contains(&resolved) {
            value.to_lowercase()
        } else if self.capitalize_vars.contains(&resolved) {
            capitalize_string(&value)
        } else {
            value
        }
    }

    /// Apply case-modification attributes to all elements of an indexed array in-place.
    /// Called after building the array but before inserting into `self.arrays`.
    pub fn apply_case_attrs_to_array(&self, name: &str, arr: &mut [Option<String>]) {
        let resolved = self.resolve_nameref(name);
        let is_upper = self.uppercase_vars.contains(&resolved);
        let is_lower = self.lowercase_vars.contains(&resolved);
        let is_cap = self.capitalize_vars.contains(&resolved);
        if !is_upper && !is_lower && !is_cap {
            return;
        }
        for val in arr.iter_mut().flatten() {
            *val = if is_upper {
                val.to_uppercase()
            } else if is_lower {
                val.to_lowercase()
            } else {
                capitalize_string(val)
            };
        }
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
                    was_declared_unset: self.declared_unset.contains(name),
                    nameref: self.namerefs.get(name).cloned(),
                    was_exported: self.exports.get(name).cloned(),
                },
            );
            // Remove readonly for the local scope (will be re-applied if -r is used)
            self.readonly_vars.remove(name);
            // Remove nameref for the local scope (will be re-applied if -n is used)
            self.namerefs.remove(name);
            // When OPTIND is made local, also save/restore the internal
            // getopts character-offset variable so that recursive getopts
            // calls (e.g. inside functions that `typeset OPTIND=1`) don't
            // clobber the caller's offset state.
            if name == "OPTIND" {
                let ofs_name = "_GETOPTS_OPTOFS".to_string();
                if !scope.contains_key(&ofs_name) {
                    scope.insert(
                        ofs_name.clone(),
                        SavedVar {
                            scalar: self.vars.get(&ofs_name).cloned(),
                            array: None,
                            assoc: None,
                            was_integer: false,
                            was_readonly: false,
                            was_declared_unset: false,
                            nameref: None,
                            was_exported: None,
                        },
                    );
                }
            }
        }
    }

    /// Set a variable at global scope, bypassing any local scopes.
    /// Used by `declare -g` to write to the global scope even when
    /// inside nested function calls with local variables.
    ///
    /// Walks `local_scopes` from bottom (oldest) to top looking for the
    /// first scope that has saved this variable.  If found, updates the
    /// saved value there (so it will be restored as the new global value
    /// when all functions return).  If no scope has saved the variable,
    /// the current maps ARE global, so we write directly.
    pub fn set_global_var(&mut self, name: &str, value: String) {
        // Find the first (bottom-most) scope that saved this variable
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.scalar = Some(value.clone());
                // Also clear array/assoc in the saved state since we're
                // setting a scalar at global scope
                saved.array = None;
                saved.assoc = None;
                // Sync exports if the variable is exported
                if self.exports.contains_key(name) {
                    self.exports.insert(name.to_string(), value.clone());
                    unsafe { std::env::set_var(name, &value) };
                }
                return;
            }
        }
        // No scope has saved this variable — current state IS global
        self.vars.insert(name.to_string(), value.clone());
        self.arrays.remove(name);
        self.assoc_arrays.remove(name);
        if self.exports.contains_key(name) {
            self.exports.insert(name.to_string(), value.clone());
            unsafe { std::env::set_var(name, &value) };
        }
    }

    /// Set an array at global scope, bypassing any local scopes.
    /// Used by `declare -g` / `declare -ga`.
    pub fn set_global_array(&mut self, name: &str, arr: Vec<Option<String>>) {
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.array = Some(arr);
                saved.scalar = None;
                saved.assoc = None;
                return;
            }
        }
        // No scope has saved this variable — current state IS global
        self.arrays.insert(name.to_string(), arr);
        self.vars.remove(name);
        self.assoc_arrays.remove(name);
    }

    /// Set an associative array at global scope, bypassing any local scopes.
    /// Used by `declare -g` / `declare -gA`.
    pub fn set_global_assoc(&mut self, name: &str, assoc: AssocArray) {
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.assoc = Some(assoc);
                saved.scalar = None;
                saved.array = None;
                return;
            }
        }
        // No scope has saved this variable — current state IS global
        self.assoc_arrays.insert(name.to_string(), assoc);
        self.vars.remove(name);
        self.arrays.remove(name);
    }

    /// Set an attribute on a variable at global scope.
    /// Used by `declare -g` with attribute flags like `-i`, `-r`, `-x`.
    pub fn set_global_attr_integer(&mut self, name: &str, set: bool) {
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.was_integer = set;
                return;
            }
        }
        if set {
            self.integer_vars.insert(name.to_string());
        } else {
            self.integer_vars.remove(name);
        }
    }

    /// Set the readonly attribute on a variable at global scope.
    #[allow(dead_code)]
    pub fn set_global_attr_readonly(&mut self, name: &str, set: bool) {
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.was_readonly = set;
                return;
            }
        }
        if set {
            self.readonly_vars.insert(name.to_string());
        } else {
            self.readonly_vars.remove(name);
        }
    }

    /// Set the declared_unset flag on a variable at global scope.
    pub fn set_global_declared_unset(&mut self, name: &str, set: bool) {
        for scope in self.local_scopes.iter_mut() {
            if let Some(saved) = scope.get_mut(name) {
                saved.was_declared_unset = set;
                return;
            }
        }
        if set {
            self.declared_unset.insert(name.to_string());
        } else {
            self.declared_unset.remove(name);
        }
    }

    /// Get an array, resolving namerefs.
    #[allow(dead_code)]
    pub fn get_array(&self, name: &str) -> Option<&Vec<Option<String>>> {
        let resolved = self.resolve_nameref(name);
        self.arrays.get(&resolved)
    }

    /// Set an array, resolving namerefs.
    #[allow(dead_code)]
    pub fn set_array(&mut self, name: &str, values: Vec<Option<String>>) {
        let resolved = self.resolve_nameref(name);
        self.insert_array(resolved, values);
    }

    /// Insert an array and clear declared_unset status.
    /// Use this instead of `self.arrays.insert(...)` directly.
    pub fn insert_array(&mut self, name: String, values: Vec<Option<String>>) {
        self.declared_unset.remove(&name);
        self.arrays.insert(name, values);
    }

    pub fn run_string(&mut self, input: &str) -> i32 {
        let saved_execution_input = self.current_execution_input.take();
        self.current_execution_input = Some(input.to_string());
        let mut parser = Parser::new_with_aliases(
            input,
            self.aliases.clone(),
            self.shopt_expand_aliases,
            self.opt_posix,
        );
        // Apply command substitution line offset so LINENO reflects the script line.
        // comsub_line_offset stores the actual 1-based LINENO of the `$(` line.
        // Use set_line_number() (absolute set) instead of set_line_offset() (relative
        // add) so that a leading '\n' consumed during parser construction doesn't
        // cause an off-by-one.
        if self.comsub_line_offset > 0 {
            parser.set_line_number(self.comsub_line_offset);
            self.comsub_line_offset = 0; // consume the offset
        }

        // Incremental parse-execute loop (for both scripts and -c mode)
        // Parse one command at a time, execute it, then parse the next
        // This allows scripts to continue after parse errors (like bash)
        let mut status = 0;
        let mut last_pos = usize::MAX;
        let saved_preparsed = self.in_preparsed_program;
        self.in_preparsed_program = false; // run_string uses parser position for LINENO
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
                        if let Some(count_str) = delim.strip_prefix("\x00COMSUB_UNTERMINATED:") {
                            // Special sentinel: unterminated here-document(s) inside comsub
                            let count: usize = count_str.parse().unwrap_or(1);
                            eprintln!(
                                "{}: line {}: warning: command substitution: {} unterminated here-document{}",
                                name,
                                eof_line,
                                count,
                                if count != 1 { "s" } else { "" }
                            );
                        } else {
                            eprintln!(
                                "{}: line {}: warning: here-document at line {} delimited by end-of-file (wanted `{}')",
                                name, eof_line, start_line, delim
                            );
                        }
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
                    // Skip commands on the same line as a prior expansion error
                    // (matching bash's DISCARD behavior: `echo $(( bad )) ; echo skip`
                    // skips everything after `;` on the same line).
                    if let Some(err_line) = self.expansion_error_line {
                        if cmd.line == err_line {
                            // Still on the error line — skip this command
                            continue;
                        }
                        // Moved to a new line — clear the flag
                        self.expansion_error_line = None;
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
                            use std::io::Read;
                            let mut buf = Vec::new();
                            std::io::stdin().take(1 << 30).read_to_end(&mut buf).ok();
                            let new_content = String::from_utf8(buf).unwrap_or_else(|e| {
                                String::from_utf8_lossy(e.as_bytes()).into_owned()
                            });
                            if !new_content.is_empty() {
                                let saved_script_fd = self.script_fd.take();
                                status = self.run_string(&new_content);
                                self.script_fd = saved_script_fd;
                            }
                            self.in_preparsed_program = saved_preparsed;
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
                    // Emit heredoc EOF warnings before the error message
                    // (bash prints heredoc warnings before syntax errors)
                    for (eof_line, start_line, delim) in parser.take_heredoc_eof_warnings() {
                        let name = self
                            .positional
                            .first()
                            .map(|s| s.as_str())
                            .unwrap_or("bash");
                        if let Some(count_str) = delim.strip_prefix("\x00COMSUB_UNTERMINATED:") {
                            let count: usize = count_str.parse().unwrap_or(1);
                            eprintln!(
                                "{}: line {}: warning: command substitution: {} unterminated here-document{}",
                                name,
                                eof_line,
                                count,
                                if count != 1 { "s" } else { "" }
                            );
                        } else {
                            eprintln!(
                                "{}: line {}: warning: here-document at line {} delimited by end-of-file (wanted `{}')",
                                name, eof_line, start_line, delim
                            );
                        }
                    }
                    // Extract accurate line number from COMSUB_LINE:N: prefix
                    // (set by take_word_checked before advance() moves the
                    // lexer past the error token's line).  Only COMSUB errors
                    // update LINENO — for other parse errors the pre-parse
                    // LINENO (set at the top of the loop) is already correct.
                    let (e, comsub_error_line) = if let Some(rest) = e.strip_prefix("COMSUB_LINE:")
                    {
                        if let Some(colon) = rest.find(':') {
                            let line: usize =
                                rest[..colon].parse().unwrap_or(parser.current_line());
                            // Re-add the COMSUB: prefix that downstream
                            // code expects for comsub error handling.
                            (format!("COMSUB:{}", &rest[colon + 1..]), Some(line))
                        } else {
                            (e, None)
                        }
                    } else {
                        (e, None)
                    };
                    if let Some(line) = comsub_error_line
                        && !self.in_debug_trap
                        && self.in_trap_handler == 0
                    {
                        self.vars.insert("LINENO".to_string(), line.to_string());
                    }
                    // Check for recoverable syntax errors (e.g. bad array compound assignment)
                    // These are marked by the parser with a \x01RECOVERABLE\x01 prefix
                    let (recoverable, e) = if let Some(msg) = e.strip_prefix("\x01RECOVERABLE\x01")
                    {
                        (true, msg.to_string())
                    } else {
                        (false, e)
                    };
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
                        // Strip COMSUB: prefix from errors originating in command
                        // substitutions parsed at parse time (like C bash's
                        // parse_comsub/xparse_dolparen).
                        let (display_err, is_comsub_error) =
                            if let Some(inner) = e.strip_prefix("COMSUB:") {
                                (inner.to_string(), true)
                            } else {
                                (e.clone(), false)
                            };
                        // Add "while looking for matching ')'" suffix for comsub
                        // errors, but NOT when the unexpected token is already `)`
                        // — that means the paren WAS found but the content before
                        // it was bad (e.g. `$( if x; then echo foo )`).
                        let comsub_suffix = if (self.in_comsub || is_comsub_error)
                            && display_err.contains("syntax error")
                            && !display_err.contains("token `)'")
                        {
                            " while looking for matching `)'"
                        } else {
                            ""
                        };
                        eprintln!(
                            "{}: {}{}",
                            self.syntax_error_prefix(),
                            display_err,
                            comsub_suffix
                        );
                        if display_err.contains("syntax error") {
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
                            if display_err.starts_with("syntax error:") {
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
                        // Strip COMSUB: prefix and add comsub suffix in
                        // non -c mode too (e.g. script files with comsub
                        // parse errors).
                        let (display_err2, is_comsub2) =
                            if let Some(inner) = e.strip_prefix("COMSUB:") {
                                (inner.to_string(), true)
                            } else {
                                (e.clone(), false)
                            };
                        // "unexpected end of file" errors get special treatment:
                        // include line number in prefix and skip source line display
                        // (bash prints "name: line N: syntax error: unexpected end of file ...")
                        if display_err2.contains("unexpected end of file") {
                            let name = self
                                .positional
                                .first()
                                .map(|s| s.as_str())
                                .unwrap_or("bash");
                            let eof_line = parser.current_line();
                            eprintln!("{}: line {}: {}", name, eof_line, display_err2);
                        } else {
                            let comsub_suffix2 = if (self.in_comsub || is_comsub2)
                                && display_err2.contains("syntax error")
                                && !display_err2.contains("token `)'")
                            {
                                " while looking for matching `)'"
                            } else {
                                ""
                            };
                            eprintln!(
                                "{}: {}{}",
                                self.error_prefix(),
                                display_err2,
                                comsub_suffix2
                            );
                            if display_err2.contains("syntax error") {
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
                    }
                    status = if recoverable { 1 } else { 2 };
                    if recoverable {
                        self.last_status = 1;
                    }
                    if e.contains("syntax error") && !recoverable {
                        // Non-recoverable syntax errors in non-interactive shells cause exit
                        return 2;
                    }
                    // Skip to the next newline to try to recover
                    parser.skip_to_next_command();
                }
            }
        }
        self.in_preparsed_program = saved_preparsed;
        self.current_execution_input = saved_execution_input;
        status
    }

    pub fn run_program(&mut self, program: &Program) -> i32 {
        let saved_preparsed = self.in_preparsed_program;
        self.in_preparsed_program = true;
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
        self.in_preparsed_program = saved_preparsed;
        status
    }

    fn run_complete_command(&mut self, cmd: &CompleteCommand) -> i32 {
        // Reap any finished coproc processes
        #[cfg(unix)]
        self.reap_coprocs();

        // Update LINENO (skip inside trap handlers to preserve calling context)
        if !self.in_debug_trap && self.in_trap_handler == 0 {
            let effective_line = cmd.line + self.for_line_adjust;
            self.vars
                .insert("LINENO".to_string(), effective_line.to_string());
        }
        // Store end_line for compound redirect error reporting
        self.cmd_end_line = Some(cmd.end_line);

        // Reap finished background jobs (non-blocking waitpid check)
        self.reap_jobs();

        if cmd.background {
            // Build command text for job tracking before forking
            let cmd_text = crate::builtins::format_complete_command(cmd);
            #[cfg(unix)]
            {
                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Parent { child }) => {
                        self.last_bg_pid = child.as_raw();
                        self.vars.insert("!".to_string(), child.to_string());
                        // Reap any finished jobs before adding new ones
                        self.reap_jobs();
                        let job_num = self.jobs.last().map_or(1, |j| j.number + 1);
                        self.jobs.push(Job {
                            number: job_num,
                            pid: child.as_raw(),
                            command: cmd_text,
                            status: JobStatus::Running,
                        });
                        return 0;
                    }
                    Ok(nix::unistd::ForkResult::Child) => {
                        // Close CLOEXEC fds to prevent pipe leaks from command
                        // substitution contexts (saved redirect fds hold comsub
                        // pipe write ends open)
                        for fd in 3..1024 {
                            if let Ok(flags) = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_GETFD)
                                && flags & libc::FD_CLOEXEC != 0
                            {
                                nix::unistd::close(fd).ok();
                            }
                        }
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
                c if c.is_control() => out.push_str(&format!("\\{:03o}", c as u32)),
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
