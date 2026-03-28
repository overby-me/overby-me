use super::*;

impl Shell {
    pub(super) fn run_command(&mut self, cmd: &Command) -> i32 {
        match cmd {
            Command::Simple(simple) => self.run_simple_command(simple),
            Command::Compound(compound, redirections) => {
                self.run_compound_with_redirects(compound, redirections)
            }
            Command::FunctionDef {
                name,
                body,
                body_line,
                end_line,
                has_function_keyword,
                redirections,
            } => {
                // Set LINENO to end of function definition
                // Use end_line + 2 for POSIX errors (bash reports line after the complete command)
                self.vars.insert("LINENO".to_string(), end_line.to_string());
                // In POSIX mode, cannot define functions with names of special builtins
                if self.opt_posix
                    && matches!(
                        name.as_str(),
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
                    )
                {
                    // For POSIX special builtin errors, bash reports LINENO at the
                    // end of the complete command (past the function body and newline)
                    let posix_line = end_line + 2;
                    self.vars
                        .insert("LINENO".to_string(), posix_line.to_string());
                    eprintln!("{}: `{}': is a special builtin", self.error_prefix(), name);
                    // Fatal error in POSIX mode — exit the shell/subshell
                    std::process::exit(2);
                }
                // Validate function name: reject names with variable expansions ($)
                // or other invalid chars
                let name_invalid = if *has_function_keyword {
                    // With 'function' keyword: reject names containing $ or backtick
                    // (these indicate variable expansion in the name)
                    name.contains('$') || name.contains('`')
                } else {
                    // Without 'function' keyword (name()): reject names with spaces
                    // or process substitution chars
                    name.contains(' ') || name.starts_with('<') || name.starts_with('>')
                };
                if name_invalid {
                    // Show the name with quotes if it contains spaces (matching bash)
                    let display_name = if name.contains(' ') {
                        format!("'{}'", name)
                    } else {
                        name.to_string()
                    };
                    eprintln!(
                        "{}: `{}': not a valid identifier",
                        self.error_prefix(),
                        display_name
                    );
                    return 1;
                }
                if self.readonly_funcs.contains(name) {
                    eprintln!("{}: {}: readonly function", self.error_prefix(), name);
                    1
                } else {
                    self.functions.insert(name.clone(), *body.clone());
                    self.func_body_lines.insert(name.clone(), *body_line);
                    if *has_function_keyword {
                        self.func_has_keyword.insert(name.clone());
                    }
                    if !redirections.is_empty() {
                        self.func_redirections
                            .insert(name.clone(), redirections.clone());
                    }
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
        let top_pid = self.top_level_pid;
        let opt_flags = self.get_opt_flags();
        // Set error prefix for expansion error messages
        crate::expand::EXPAND_ERROR_PREFIX.with(|p| {
            *p.borrow_mut() = self.error_prefix();
        });
        // Register inline runner for process substitutions using raw pointer
        // Safety: the pointer is valid for the duration of this function call
        let self_ptr = self as *mut Shell;
        let mut procsub_runner = move |cmd: &str| -> i32 {
            let shell = unsafe { &mut *self_ptr };
            // Mark as pipeline child so EPIPE errors are suppressed in process sub children
            let saved = shell.in_pipeline_child;
            shell.in_pipeline_child = true;
            let status = shell.run_string(cmd);
            shell.in_pipeline_child = saved;
            status
        };
        crate::expand::set_procsub_runner(&mut procsub_runner as *mut dyn FnMut(&str) -> i32);
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        let result = expand::expand_word(
            &word,
            &vars,
            &arrays,
            &assoc_arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            top_pid,
            &opt_flags,
            ifs,
            &mut cmd_sub,
        );
        crate::expand::clear_procsub_runner();
        result
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
        let top_pid = self.top_level_pid;
        let opt_flags = self.get_opt_flags();
        crate::expand::EXPAND_ERROR_PREFIX.with(|p| {
            *p.borrow_mut() = self.error_prefix();
        });
        let self_ptr2 = self as *mut Shell;
        let mut procsub_runner = move |cmd: &str| -> i32 {
            let shell = unsafe { &mut *self_ptr2 };
            let saved = shell.in_pipeline_child;
            shell.in_pipeline_child = true;
            let status = shell.run_string(cmd);
            shell.in_pipeline_child = saved;
            status
        };
        crate::expand::set_procsub_runner(&mut procsub_runner as *mut dyn FnMut(&str) -> i32);
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        let result = expand::expand_word_nosplit(
            &word,
            &vars,
            &arrays,
            &assoc_arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            top_pid,
            &opt_flags,
            &mut cmd_sub,
        );
        crate::expand::clear_procsub_runner();
        result
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
        let top_pid = self.top_level_pid;
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
            top_pid,
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
    /// Get the expanded PS4 prompt for xtrace
    pub fn get_ps4(&self) -> String {
        let ps4 = self
            .vars
            .get("PS4")
            .cloned()
            .unwrap_or_else(|| "+ ".to_string());
        // Expand PS4 (simple variable expansion, mainly $LINENO)
        let mut result = String::new();
        let chars: Vec<char> = ps4.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() {
                if chars[i + 1] == '{' {
                    // ${VAR}
                    i += 2;
                    let mut name = String::new();
                    while i < chars.len() && chars[i] != '}' {
                        name.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                    result.push_str(
                        self.vars
                            .get(name.as_str())
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    );
                } else if chars[i + 1].is_alphabetic() || chars[i + 1] == '_' {
                    // $VAR
                    i += 1;
                    let mut name = String::new();
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        name.push(chars[i]);
                        i += 1;
                    }
                    result.push_str(
                        self.vars
                            .get(name.as_str())
                            .map(|s| s.as_str())
                            .unwrap_or(""),
                    );
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            } else if chars[i] == '\\' && i + 1 < chars.len() {
                result.push(chars[i + 1]);
                i += 2;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        }
        result
    }

    pub fn xtrace_write(&self, msg: &str) {
        // Replace leading "+" prefix with expanded PS4
        // Bash replicates the first char of PS4 based on nesting depth
        let output = if msg.starts_with('+') {
            let ps4 = self.get_ps4();
            // Count leading '+' characters from the message
            let msg_depth = msg.chars().take_while(|&c| c == '+').count();
            // Add trap handler nesting depth
            let depth = msg_depth + self.in_trap_handler as usize;
            let rest = msg[msg_depth..].trim_start();
            if ps4.is_empty() {
                format!("{} {}", "+".repeat(depth), rest)
            } else {
                let first_char = ps4.chars().next().unwrap();
                let remainder = &ps4[first_char.len_utf8()..];
                format!(
                    "{}{}{}",
                    first_char.to_string().repeat(depth),
                    remainder,
                    rest
                )
            }
        } else {
            msg.to_string()
        };
        let msg = &output;
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
    pub(super) fn arith_cmd_prefix(&self) -> &str {
        if self.arith_is_command {
            "((: "
        } else if self.arith_is_let {
            "let: "
        } else {
            ""
        }
    }

    pub(super) fn arith_error_prefix(&self) -> String {
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

    pub(super) fn get_opt_flags(&self) -> String {
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
        // Don't overwrite during DEBUG or ERR trap execution
        if !self.in_debug_trap && self.in_trap_handler == 0 {
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
            // Check for incomplete funsub (${ ... } without terminator)
            if fields.iter().any(|f| f.contains("INCOMPLETE_FUNSUB")) {
                eprintln!(
                    "{}: unexpected EOF while looking for matching `}}'",
                    self.error_prefix()
                );
                return 1;
            }
            // Check for error incomplete comsub — suppress with error message
            if fields
                .iter()
                .any(|f| f == "INCOMPLETE_COMSUB" || f.contains("INCOMPLETE_COMSUB"))
            {
                if self.dash_c_mode {
                    // In -c mode, report as syntax error near ')' with source line
                    let prefix = self.syntax_error_prefix();
                    eprintln!("{}: syntax error near unexpected token `)'", prefix);
                    // Show the source line from the original -c input
                    if let Some(src) = self.vars.get("_BASH_C_STRING") {
                        let lineno: usize = self
                            .vars
                            .get("LINENO")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(1);
                        let line = src.lines().nth(lineno.saturating_sub(1)).unwrap_or(src);
                        eprintln!("{}: `{}'", prefix, line);
                    }
                } else {
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
                }
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
            // No command words — apply redirections for null commands (e.g., `> file`)
            if !cmd.redirections.is_empty() {
                match self.setup_redirections(&cmd.redirections) {
                    Ok(saved_fds) => {
                        self.restore_redirections(saved_fds);
                    }
                    Err(e) => {
                        eprintln!("{}: {}", self.error_prefix(), e);
                        return 1;
                    }
                }
            }
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

        // Check for function (but in POSIX mode, special builtins take precedence)
        let is_posix_special_builtin = self.opt_posix
            && matches!(
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
        let status = if !is_posix_special_builtin
            && let Some(func_body) = self.functions.get(command_name).cloned()
        {
            // Apply prefix assignments temporarily for function calls
            // Save: (name, set_value, old_var, old_export)
            let prefix_saves: Vec<(String, String, Option<String>, Option<String>)> = cmd
                .assignments
                .iter()
                .map(|a| {
                    let v = match &a.value {
                        AssignValue::Scalar(w) => self.expand_word_single(w),
                        _ => String::new(),
                    };
                    let old_var = self.vars.get(&a.name).cloned();
                    let old_export = self.exports.get(&a.name).cloned();
                    let final_val = if a.append {
                        if self.integer_vars.contains(&a.name) {
                            let existing = self.eval_arith_expr(old_var.as_deref().unwrap_or("0"));
                            let addend = self.eval_arith_expr(&v);
                            (existing + addend).to_string()
                        } else {
                            let existing = old_var.as_deref().unwrap_or("");
                            format!("{}{}", existing, v)
                        }
                    } else {
                        v
                    };
                    self.vars.insert(a.name.clone(), final_val.clone());
                    // Export prefix assignments so printenv/child processes see them
                    self.exports.insert(a.name.clone(), final_val.clone());
                    unsafe { std::env::set_var(&a.name, &final_val) };
                    (a.name.clone(), final_val, old_var, old_export)
                })
                .collect();

            let result = self.run_function(&func_body, command_name, args);

            // Restore prefix assignments (vars + exports)
            for (k, set_val, old_var, old_export) in &prefix_saves {
                // In POSIX mode, if the variable was modified inside the function
                // (e.g., by special builtin prefix assignments that persist),
                // don't restore it
                if self.opt_posix {
                    let current = self.vars.get(k).cloned();
                    if current.as_deref() != Some(set_val.as_str()) {
                        continue; // variable was changed inside function, keep the change
                    }
                }
                match old_var {
                    Some(v) => {
                        self.vars.insert(k.clone(), v.clone());
                    }
                    None => {
                        self.vars.remove(k);
                    }
                }
                match old_export {
                    Some(v) => {
                        self.exports.insert(k.clone(), v.clone());
                        unsafe { std::env::set_var(k, v) };
                    }
                    None => {
                        self.exports.remove(k);
                        unsafe { std::env::remove_var(k) };
                    }
                }
            }
            result
        } else if let Some(builtin) = self.builtins.get(command_name.as_str()).copied() {
            // For assignment builtins (readonly, export, declare, local), handle
            // compound array assignments: name=(val) — perform the assignment and
            // pass just the name to the builtin (bash parser-level assignment behavior)
            let is_assign_builtin = matches!(
                command_name.as_str(),
                "readonly" | "export" | "declare" | "typeset" | "local"
            );
            if is_assign_builtin {
                // Check if -a flag is present (affects error prefix for readonly errors)
                let has_array_flag = args
                    .iter()
                    .any(|a| a.starts_with('-') && a.len() > 1 && a.contains('a'));
                let has_assoc_flag = args
                    .iter()
                    .any(|a| a.starts_with('-') && a.len() > 1 && a.contains('A'));
                let mut new_args = Vec::new();
                let mut modified = false;
                for (arg_idx, arg) in args.iter().enumerate() {
                    // Check for name=(value) pattern from inline array parsing
                    // Only if the original word had unquoted array syntax (not from 'name=(val)')
                    let word_idx = arg_idx + 1; // skip command name
                    let is_quoted_arg = word_idx < cmd.words.len()
                        && cmd.words[word_idx].iter().any(|p| {
                            matches!(p, WordPart::SingleQuoted(_) | WordPart::DoubleQuoted(_))
                        });
                    if !is_quoted_arg && let Some(eq_pos) = arg.find('=') {
                        let name = &arg[..eq_pos];
                        let value = &arg[eq_pos + 1..];
                        if value.starts_with('(')
                            && value.ends_with(')')
                            && !name.is_empty()
                            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                            && !name.starts_with(|c: char| c.is_ascii_digit())
                        {
                            // Perform the array assignment
                            if self.readonly_vars.contains(name) {
                                if has_array_flag {
                                    if let Some(fname) = self.func_names.last() {
                                        eprintln!(
                                            "{}: {}: {}: readonly variable",
                                            self.error_prefix(),
                                            fname,
                                            name
                                        );
                                    } else {
                                        eprintln!(
                                            "{}: {}: readonly variable",
                                            self.error_prefix(),
                                            name
                                        );
                                    }
                                } else {
                                    eprintln!(
                                        "{}: {}: readonly variable",
                                        self.error_prefix(),
                                        name
                                    );
                                }
                                self.last_status = 1;
                            } else if has_assoc_flag {
                                let map = crate::builtins::parse_assoc_literal(value);
                                self.assoc_arrays.insert(name.to_string(), map);
                            } else {
                                let arr = crate::builtins::parse_array_literal(value);
                                self.arrays.insert(name.to_string(), arr);
                            }
                            new_args.push(name.to_string());
                            modified = true;
                            continue;
                        }
                    }
                    new_args.push(arg.clone());
                }
                if modified {
                    // Use the modified args with array assignments handled
                    let all_words: Vec<String> = std::iter::once(command_name.clone())
                        .chain(new_args)
                        .collect();
                    // Re-bind args to the new allocation
                    let args = &all_words[1..];
                    // Fall through to the builtin call below with the modified args
                    // (but we need to handle it inline since args lifetime changes)
                    let prefix_exports: Vec<(String, String)> = vec![];
                    let saved: Vec<(String, Option<String>)> = vec![];
                    self.current_builtin = Some(command_name.clone());
                    let result = builtin(self, args);
                    self.current_builtin = None;
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
                    drop(prefix_exports);
                    self.restore_redirections(saved_fds);
                    return result;
                }
            }
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

            // Check readonly for prefix assignments
            for (k, _) in &prefix_exports {
                if self.readonly_vars.contains(k) {
                    eprintln!("{}: {}: readonly variable", self.error_prefix(), k);
                    self.restore_redirections(saved_fds);
                    return 1;
                }
            }
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
            // In POSIX mode, errors from special builtins are fatal (exit the shell)
            if self.opt_posix && is_special && result != 0 {
                // Determine if this error should be fatal
                let is_fatal = match command_name.as_str() {
                    // return: fatal only if not inside a function (invalid context)
                    "return" => !self.returning,
                    // break/continue: never fatal (silently ignored outside loops)
                    "break" | "continue" => false,
                    // shift: not fatal (shift count out of range)
                    "shift" => false,
                    // trap: not fatal (invalid signal)
                    "trap" => false,
                    // exit: never fatal (it's already exiting)
                    "exit" => false,
                    // . and source: fatal only on file error (not found etc.)
                    "." | "source" => self.source_file_error,
                    // All other special builtins: fatal
                    _ => true,
                };
                if is_fatal {
                    self.restore_redirections(saved_fds);
                    std::process::exit(result);
                }
            }
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
        // (they should persist in the current shell).
        // Also handle `command exec` and `builtin exec` which delegate to exec.
        let is_exec_no_cmd = (command_name == "exec" && args.is_empty())
            || ((command_name == "command" || command_name == "builtin")
                && args.first().map(|s| s.as_str()) == Some("exec")
                && args.len() <= 1);
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

        // Save LINENO so ERR trap after function sees the call-site line
        let saved_lineno = self.vars.get("LINENO").cloned();

        // ERR trap is not inherited by functions unless errtrace is set
        let saved_err_trap = if !self.shopt_options.get("errtrace").copied().unwrap_or(false) {
            self.traps.remove("ERR")
        } else {
            None
        };
        // DEBUG trap: not inherited unless functrace is set OR function is traced
        let is_traced = self.traced_funcs.contains(name);
        let inherit_debug = self
            .shopt_options
            .get("functrace")
            .copied()
            .unwrap_or(false)
            || is_traced;
        let saved_debug_trap = if !inherit_debug {
            self.traps.remove("DEBUG")
        } else {
            None
        };
        // Without functrace, the function doesn't inherit the parent's DEBUG trap,
        // but any trap set inside the function IS visible after the function returns.

        // Fire DEBUG trap at function entry (bash fires it once at the call site
        // and once at the start of the function body for traced functions)
        if inherit_debug {
            // Set LINENO to the function body start line (where { is)
            if let Some(&body_line) = self.func_body_lines.get(name) {
                self.vars
                    .insert("LINENO".to_string(), body_line.to_string());
            }
            self.run_debug_trap();
        }

        // Apply function-level redirections (from f() { ... } >>file)
        let func_redirs = self.func_redirections.get(name).cloned();
        let func_saved_fds = if let Some(ref redirs) = func_redirs {
            self.setup_redirections(redirs).ok()
        } else {
            None
        };

        let mut status = self.run_compound_command(body);
        // If returning was set (by builtin_return or a trap handler), use last_status
        if self.returning {
            status = self.last_status;
        }

        // Restore function-level redirections
        if let Some(fds) = func_saved_fds {
            self.restore_redirections(fds);
        }

        // Restore ERR and DEBUG traps and LINENO
        if let Some(err_trap) = saved_err_trap {
            self.traps.insert("ERR".to_string(), err_trap);
        }
        // Only restore DEBUG trap if the function didn't set a new one
        if let Some(debug_trap) = saved_debug_trap
            && !self.traps.contains_key("DEBUG")
        {
            self.traps.insert("DEBUG".to_string(), debug_trap);
        }
        if let Some(ln) = saved_lineno {
            self.vars.insert("LINENO".to_string(), ln);
        }

        // Run RETURN trap before restoring scope
        // RETURN trap is only inherited by functions when functrace is set
        // or the function is traced
        if inherit_debug {
            self.run_return_trap();
        }

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
                    self.integer_vars.insert(var_name.clone());
                } else {
                    self.integer_vars.remove(&var_name);
                }
                // Restore readonly attribute
                if saved.was_readonly {
                    self.readonly_vars.insert(var_name);
                } else {
                    self.readonly_vars.remove(&var_name);
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
        self.return_explicit_arg = false;
        status
    }

    fn run_external(&mut self, name: &str, args: &[String], assignments: &[Assignment]) -> i32 {
        #[cfg(unix)]
        {
            use std::ffi::CString;

            // Flush stdout before forking to prevent buffered builtin output
            // from appearing after the external command's output
            std::io::Write::flush(&mut std::io::stdout()).ok();

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

                    // Check if this is a script without shebang — run with ourselves
                    // instead of letting execvp fall back to /bin/sh
                    if name.contains('/')
                        && let Ok(mut f) = std::fs::File::open(name)
                    {
                        use std::io::Read;
                        let mut header = [0u8; 4];
                        if f.read(&mut header).unwrap_or(0) >= 2
                                && header[0] != 0x7f // not ELF
                                && !(header[0] == b'#' && header[1] == b'!')
                        // no shebang
                        {
                            // Text file without shebang — run with ourselves
                            // Try multiple paths: current_exe, /proc/self/exe, argv[0]
                            let exe_path = std::env::current_exe()
                                .map(|p| p.to_string_lossy().to_string())
                                .or_else(|_| {
                                    std::fs::read_link("/proc/self/exe")
                                        .map(|p| p.to_string_lossy().to_string())
                                })
                                .unwrap_or_else(|_| {
                                    std::env::args()
                                        .next()
                                        .unwrap_or_else(|| "bash".to_string())
                                });
                            let self_exe = std::ffi::CString::new(exe_path.as_str()).unwrap();
                            let mut new_args = vec![self_exe.clone()];
                            new_args.extend(c_args.iter().cloned());
                            nix::unistd::execvp(&self_exe, &new_args).ok();
                        }
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
                                nix::errno::Errno::ENOEXEC => {
                                    // Fall back to running script with ourselves
                                    // (like bash does for scripts without shebang)
                                    use std::ffi::CString;
                                    // Try current_exe first, fall back to /proc/self/exe
                                    let exe_path = std::env::current_exe()
                                        .map(|p| p.to_string_lossy().to_string())
                                        .unwrap_or_else(|_| "/proc/self/exe".to_string());
                                    let self_exe = CString::new(exe_path.as_str()).unwrap();
                                    let mut new_args = vec![self_exe.clone()];
                                    new_args.extend(c_args.iter().cloned());
                                    nix::unistd::execvp(&self_exe, &new_args).ok();
                                    "Exec format error"
                                }
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
                    // Wait for child, but check for pending signals on EINTR
                    self.in_foreground_wait = true;
                    let result = loop {
                        match nix::sys::wait::waitpid(
                            child,
                            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
                        ) {
                            Ok(nix::sys::wait::WaitStatus::Exited(_, code)) => break code,
                            Ok(nix::sys::wait::WaitStatus::Signaled(_, sig, _)) => {
                                break 128 + sig as i32;
                            }
                            Ok(nix::sys::wait::WaitStatus::StillAlive) => {
                                // Child still running — check signals
                                self.check_pending_signals();
                                if self.returning || self.breaking > 0 {
                                    // Signal handler requested return — leave child running
                                    break 128;
                                }
                                // Brief sleep to avoid busy-waiting
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            Err(_) => break 1,
                            _ => break 1,
                        }
                    };
                    self.in_foreground_wait = false;
                    result
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
        // For redirect errors in pre-parsed programs (subshells/functions),
        // use end_line because cmd.line is the start, not the post-parse position.
        // This is only needed when NOT running from run_string (which sets LINENO
        // from the parser's current position before each command).
        let saved_lineno = if !redirections.is_empty()
            && let Some(end_line) = self.cmd_end_line
            && self.in_preparsed_program
        {
            let old = self.vars.get("LINENO").cloned();
            self.vars
                .insert("LINENO".to_string(), end_line.to_string());
            old
        } else {
            None
        };
        let saved_fds = match self.setup_redirections(redirections) {
            Ok(fds) => fds,
            Err(e) => {
                eprintln!("{}: {}", self.error_prefix(), e);
                // Restore LINENO
                if let Some(ln) = saved_lineno {
                    self.vars.insert("LINENO".to_string(), ln);
                }
                return 1;
            }
        };
        // Restore LINENO after redirect setup (it was only temporarily changed for error reporting)
        if let Some(ln) = saved_lineno {
            self.vars.insert("LINENO".to_string(), ln);
        }

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
                            // Clear inherited traps (subshells don't inherit EXIT/ERR traps)
                            self.traps.remove("EXIT");
                            self.traps.remove("0");
                            if !self.shopt_options.get("errtrace").copied().unwrap_or(false) {
                                self.traps.remove("ERR");
                            }
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
            // In bash, select/for identifier errors inside functions use the
            // function body start line for LINENO
            if let Some(fname) = self.func_names.last()
                && let Some(&body_line) = self.func_body_lines.get(fname.as_str())
            {
                self.vars
                    .insert("LINENO".to_string(), body_line.to_string());
            }
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
            // Check for readonly variable
            if self.readonly_vars.contains(&clause.var) {
                eprintln!("{}: {}: readonly variable", self.error_prefix(), clause.var);
                return 1;
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

            // Check for pending signals (allows trap handlers to break loops)
            self.check_pending_signals();
            if self.returning || self.breaking > 0 {
                if self.breaking > 0 {
                    self.breaking -= 1;
                }
                break;
            }

            let cond_status = self.run_condition(&clause.condition);
            if self.returning || self.breaking > 0 {
                if self.breaking > 0 {
                    self.breaking -= 1;
                }
                break;
            }
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

            // Check signals at the end of each loop iteration too
            if self.in_trap_handler == 0 {
                self.check_pending_signals();
                if self.returning || self.breaking > 0 {
                    if self.breaking > 0 {
                        self.breaking -= 1;
                    }
                    break;
                }
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
    /// Format conditional expression using raw source text (for BASH_COMMAND)
    fn format_cond_raw_helper(expr: &CondExpr) -> String {
        match expr {
            CondExpr::Word(w) => crate::ast::word_to_string(w),
            CondExpr::Unary(op, w) => {
                format!("{} {}", op, crate::ast::word_to_string(w))
            }
            CondExpr::Binary(l, op, r) => {
                format!(
                    "{} {} {}",
                    crate::ast::word_to_string(l),
                    op,
                    crate::ast::word_to_string(r)
                )
            }
            CondExpr::Not(e) => format!("! {}", Self::format_cond_raw_helper(e)),
            CondExpr::And(a, b) => format!(
                "{} && {}",
                Self::format_cond_raw_helper(a),
                Self::format_cond_raw_helper(b)
            ),
            CondExpr::Or(a, b) => format!(
                "{} || {}",
                Self::format_cond_raw_helper(a),
                Self::format_cond_raw_helper(b)
            ),
        }
    }

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

    /// Expand a regex pattern with mixed quoted/unquoted parts.
    /// Unquoted parts are treated as regex. Quoted parts have regex metacharacters escaped.
    fn expand_regex_pattern_mixed(&mut self, word: &Word) -> String {
        let mut result = String::new();
        for part in word {
            match part {
                WordPart::Literal(s) => result.push_str(s),
                WordPart::SingleQuoted(s) => {
                    if s.len() == 1 {
                        // Backslash escape from \x — preserve as regex escape
                        result.push('\\');
                        result.push_str(s);
                    } else {
                        // Quoted string — escape regex metacharacters except [ and ]
                        // because escaping ] changes bracket expression semantics
                        // in regex_lite (where \] inside [...] is literal ], extending the bracket)
                        for ch in s.chars() {
                            if "\\(){}.*+?|^$".contains(ch) {
                                result.push('\\');
                            }
                            result.push(ch);
                        }
                    }
                }
                WordPart::Variable(name) => {
                    let val = self.vars.get(name.as_str()).cloned().unwrap_or_default();
                    result.push_str(&val);
                }
                WordPart::DoubleQuoted(parts) => {
                    // Quoted — escape regex metacharacters (except [ and ])
                    let mut expanded = String::new();
                    for p in parts {
                        match p {
                            WordPart::Literal(s) => expanded.push_str(s),
                            WordPart::Variable(name) => {
                                let val = self.vars.get(name.as_str()).cloned().unwrap_or_default();
                                expanded.push_str(&val);
                            }
                            _ => {
                                expanded.push_str(&self.expand_word_single(&vec![p.clone()]));
                            }
                        }
                    }
                    for ch in expanded.chars() {
                        if "\\(){}.*+?|^$".contains(ch) {
                            result.push('\\');
                        }
                        result.push(ch);
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
    pub(super) fn reap_coprocs(&mut self) {
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
    pub(super) fn coproc_checkfd(&mut self, fd: i32) {
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
        // Set BASH_COMMAND for the conditional (using raw source text, not expanded)
        if !self.in_debug_trap && self.in_trap_handler == 0 {
            let raw = Self::format_cond_raw_helper(expr);
            self.vars
                .insert("BASH_COMMAND".to_string(), format!("[[ {} ]]", raw));
        }
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
                    // Check if the ENTIRE pattern is quoted (all parts are quoted/backslash-escaped).
                    // If entirely quoted, do literal string match.
                    // If mixed (some quoted, some not), build regex with quoted parts escaped.
                    let has_unquoted = right
                        .iter()
                        .any(|p| matches!(p, WordPart::Literal(_) | WordPart::Variable(_)));
                    let has_quoted = right.iter().any(|p| match p {
                        WordPart::DoubleQuoted(_) => true,
                        WordPart::SingleQuoted(s) => s.len() > 1,
                        _ => false,
                    });
                    let is_fully_quoted = has_quoted && !has_unquoted;
                    if is_fully_quoted {
                        // Fully quoted: literal string match (not regex)
                        let rval = self.expand_regex_pattern(right);
                        let matched = lval.contains(&rval);
                        if matched {
                            self.arrays
                                .insert("BASH_REMATCH".to_string(), vec![rval.clone()]);
                        } else {
                            self.arrays.insert("BASH_REMATCH".to_string(), Vec::new());
                        }
                        return Ok(matched);
                    }
                    if has_quoted && has_unquoted {
                        // Mixed: build regex with quoted parts having metacharacters escaped
                        let rval = self.expand_regex_pattern_mixed(right);
                        return self.eval_cond_binary(&lval, op, &rval);
                    }
                    // Fully unquoted: treat as regex
                    let rval = self.expand_regex_pattern(right);
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
