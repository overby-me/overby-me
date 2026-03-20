use crate::ast::*;
use crate::builtins::{self, BuiltinFn};
use crate::expand;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};

pub struct Shell {
    pub vars: HashMap<String, String>,
    pub exports: HashMap<String, String>,
    pub readonly_vars: HashSet<String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub functions: HashMap<String, CompoundCommand>,
    pub positional: Vec<String>,
    pub last_status: i32,
    pub last_bg_pid: i32,
    pub returning: bool,
    pub breaking: i32,
    pub continuing: i32,
    pub in_condition: bool,
    pub dir_stack: Vec<String>,
    pub func_names: Vec<String>,
    pub traps: HashMap<String, String>,
    pub namerefs: HashMap<String, String>,

    // Shell options (set)
    pub opt_errexit: bool,
    pub opt_nounset: bool,
    pub opt_xtrace: bool,
    pub opt_pipefail: bool,
    pub opt_noclobber: bool,
    pub opt_noglob: bool,
    pub opt_noexec: bool,

    // Shell options (shopt)
    pub shopt_nullglob: bool,
    pub shopt_extglob: bool,
    pub shopt_inherit_errexit: bool,
    pub shopt_nocasematch: bool,

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
        vars.insert("BASH_VERSINFO".to_string(), "5".to_string());
        vars.insert("SHELL".to_string(), "/bin/bash".to_string());
        vars.insert("OPTIND".to_string(), "1".to_string());
        vars.insert("OPTERR".to_string(), "1".to_string());
        vars.insert("LINENO".to_string(), "1".to_string());
        vars.insert("RANDOM".to_string(), "0".to_string());
        vars.insert("SECONDS".to_string(), "0".to_string());
        vars.insert("BASHPID".to_string(), std::process::id().to_string());
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

        Self {
            vars,
            exports,
            readonly_vars: HashSet::new(),
            arrays: HashMap::new(),
            functions: HashMap::new(),
            positional: vec!["bash".to_string()],
            last_status: 0,
            last_bg_pid: 0,
            returning: false,
            breaking: 0,
            continuing: 0,
            in_condition: false,
            dir_stack: Vec::new(),
            func_names: Vec::new(),
            traps: HashMap::new(),
            namerefs: HashMap::new(),
            opt_errexit: false,
            opt_nounset: false,
            opt_xtrace: false,
            opt_pipefail: false,
            opt_noclobber: false,
            opt_noglob: false,
            opt_noexec: false,
            shopt_nullglob: false,
            shopt_extglob: false,
            shopt_inherit_errexit: false,
            shopt_nocasematch: false,
            builtins: builtins::builtins(),
        }
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
    pub fn get_var(&self, name: &str) -> Option<&String> {
        let resolved = self.resolve_nameref(name);
        self.vars.get(&resolved)
    }

    /// Set a variable value, resolving namerefs.
    pub fn set_var(&mut self, name: &str, value: String) {
        let resolved = self.resolve_nameref(name);
        if self.readonly_vars.contains(&resolved) {
            eprintln!("bash: {}: readonly variable", resolved);
            return;
        }
        // If variable is exported, update the export
        if self.exports.contains_key(&resolved) {
            self.exports.insert(resolved.clone(), value.clone());
            unsafe { std::env::set_var(&resolved, &value) };
        }
        self.vars.insert(resolved, value);
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
        let mut parser = Parser::new(input);
        match parser.parse_program() {
            Ok(program) => self.run_program(&program),
            Err(e) => {
                eprintln!("bash: syntax error: {}", e);
                2
            }
        }
    }

    pub fn run_program(&mut self, program: &Program) -> i32 {
        let mut status = 0;
        for cmd in program {
            if self.returning || self.breaking > 0 {
                break;
            }
            status = self.run_complete_command(cmd);
            if self.opt_errexit && status != 0 && !self.in_condition {
                std::io::Write::flush(&mut std::io::stdout()).ok();
                std::io::Write::flush(&mut std::io::stderr()).ok();
                std::process::exit(status);
            }
        }
        status
    }

    fn run_complete_command(&mut self, cmd: &CompleteCommand) -> i32 {
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
        if has_rest {
            self.in_condition = true;
        }
        let mut status = self.run_pipeline(&list.first);

        for (i, (op, pipeline)) in list.rest.iter().enumerate() {
            let is_last = i == list.rest.len() - 1;
            if is_last {
                self.in_condition = saved;
            }
            match op {
                AndOr::And => {
                    if status == 0 {
                        status = self.run_pipeline(pipeline);
                    }
                }
                AndOr::Or => {
                    if status != 0 {
                        status = self.run_pipeline(pipeline);
                    }
                }
            }
        }

        self.in_condition = saved;
        status
    }

    fn run_pipeline(&mut self, pipeline: &Pipeline) -> i32 {
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
            use std::os::unix::io::{AsRawFd, RawFd};

            let mut prev_read_fd: Option<RawFd> = None;
            let mut children: Vec<nix::unistd::Pid> = Vec::new();

            for (i, cmd) in pipeline.commands.iter().enumerate() {
                let is_last = i == pipeline.commands.len() - 1;

                let (read_fd, write_fd): (Option<RawFd>, Option<RawFd>) = if !is_last {
                    let (r, w) = nix::unistd::pipe().expect("pipe failed");
                    (Some(r.as_raw_fd()), Some(w.as_raw_fd()))
                } else {
                    (None, None)
                };

                match unsafe { nix::unistd::fork() } {
                    Ok(nix::unistd::ForkResult::Child) => {
                        if let Some(fd) = prev_read_fd {
                            nix::unistd::dup2(fd, 0).ok();
                            nix::unistd::close(fd).ok();
                        }
                        if let Some(fd) = write_fd {
                            nix::unistd::dup2(fd, 1).ok();
                            nix::unistd::close(fd).ok();
                        }
                        if let Some(fd) = read_fd {
                            nix::unistd::close(fd).ok();
                        }

                        let status = self.run_command(cmd);
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
                self.functions.insert(name.clone(), *body.clone());
                0
            }
        }
    }

    pub fn expand_word_fields(&mut self, word: &Word, ifs: &str) -> Vec<String> {
        let vars = self.vars.clone();
        let arrays = self.arrays.clone();
        let namerefs = self.namerefs.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let last_bg_pid = self.last_bg_pid;
        let opt_flags = self.get_opt_flags();
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word(
            word,
            &vars,
            &arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            &opt_flags,
            ifs,
            &mut cmd_sub,
        )
    }

    pub fn expand_word_single(&mut self, word: &Word) -> String {
        let vars = self.vars.clone();
        let arrays = self.arrays.clone();
        let namerefs = self.namerefs.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let last_bg_pid = self.last_bg_pid;
        let opt_flags = self.get_opt_flags();
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word_nosplit(
            word,
            &vars,
            &arrays,
            &namerefs,
            &positional,
            last_status,
            last_bg_pid,
            &opt_flags,
            &mut cmd_sub,
        )
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
        flags
    }

    fn run_simple_command(&mut self, cmd: &SimpleCommand) -> i32 {
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());

        // Expand words
        let mut expanded_words: Vec<String> = Vec::new();
        for word in &cmd.words {
            let fields = self.expand_word_fields(word, &ifs);
            expanded_words.extend(fields);
        }

        // Handle assignments
        if !cmd.assignments.is_empty() {
            for assign in &cmd.assignments {
                if expanded_words.is_empty() {
                    self.execute_assignment(assign);
                }
            }
        }

        if expanded_words.is_empty() {
            return 0;
        }

        // Trace
        if self.opt_xtrace {
            eprintln!("+ {}", expanded_words.join(" "));
        }

        let command_name = &expanded_words[0];
        let args = &expanded_words[1..];

        // Set up redirections
        let saved_fds = match self.setup_redirections(&cmd.redirections) {
            Ok(fds) => fds,
            Err(e) => {
                eprintln!("bash: {}", e);
                return 1;
            }
        };

        // Check for function
        let status = if let Some(func_body) = self.functions.get(command_name).cloned() {
            self.run_function(&func_body, command_name, args)
        } else if let Some(builtin) = self.builtins.get(command_name.as_str()).copied() {
            let prefix_exports: Vec<(String, String)> = if !expanded_words.is_empty() {
                cmd.assignments
                    .iter()
                    .map(|a| {
                        let v = match &a.value {
                            AssignValue::Scalar(w) => self.expand_word_single(w),
                            _ => String::new(),
                        };
                        (a.name.clone(), v)
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

            let result = builtin(self, args);

            if !expanded_words.is_empty() {
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

        self.last_status = status;
        status
    }

    pub fn execute_assignment(&mut self, assign: &Assignment) {
        match &assign.value {
            AssignValue::None => {
                let resolved = self.resolve_nameref(&assign.name);
                self.vars.entry(resolved).or_default();
            }
            AssignValue::Scalar(w) => {
                let value = self.expand_word_single(w);
                if assign.append {
                    let resolved = self.resolve_nameref(&assign.name);
                    // Check if it's an array append
                    if self.arrays.contains_key(&resolved) {
                        self.arrays.entry(resolved).or_default().push(value);
                    } else {
                        let existing = self.vars.get(&resolved).cloned().unwrap_or_default();
                        self.set_var(&assign.name, format!("{}{}", existing, value));
                    }
                } else {
                    // Check for arr[n]=value
                    if let Some(bracket) = assign.name.find('[') {
                        let base = &assign.name[..bracket];
                        let idx_str = &assign.name[bracket + 1..assign.name.len() - 1];
                        let resolved = self.resolve_nameref(base);
                        let idx: usize = self.eval_arith_expr(idx_str).max(0) as usize;
                        let arr = self.arrays.entry(resolved).or_default();
                        while arr.len() <= idx {
                            arr.push(String::new());
                        }
                        arr[idx] = value;
                    } else {
                        self.set_var(&assign.name, value);
                    }
                }
            }
            AssignValue::Array(elements) => {
                let resolved = self.resolve_nameref(&assign.name);
                let mut arr = if assign.append {
                    self.arrays.get(&resolved).cloned().unwrap_or_default()
                } else {
                    Vec::new()
                };
                let mut next_idx = arr.len();
                for elem in elements {
                    let value = self.expand_word_single(&elem.value);
                    if let Some(idx_word) = &elem.index {
                        let idx_str = self.expand_word_single(idx_word);
                        let idx = self.eval_arith_expr(&idx_str).max(0) as usize;
                        while arr.len() <= idx {
                            arr.push(String::new());
                        }
                        arr[idx] = value;
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

    /// Evaluate an arithmetic expression and return the integer result.
    pub fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let vars = self.vars.clone();
        let arrays = self.arrays.clone();
        let positional = self.positional.clone();
        expand::eval_arith_full(expr, &vars, &arrays, &positional, self.last_status)
    }

    fn run_function(&mut self, body: &CompoundCommand, name: &str, args: &[String]) -> i32 {
        let saved_positional = self.positional.clone();
        let prog = self.positional.first().cloned().unwrap_or_default();
        self.positional = vec![prog];
        self.positional.extend_from_slice(args);
        self.func_names.push(name.to_string());

        let status = self.run_compound_command(body);

        self.func_names.pop();
        self.positional = saved_positional;
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
                    for assign in assignments {
                        let value = match &assign.value {
                            AssignValue::Scalar(w) => self.expand_word_single(w),
                            _ => String::new(),
                        };
                        unsafe { std::env::set_var(&assign.name, &value) };
                    }

                    for (key, value) in &self.exports {
                        unsafe { std::env::set_var(key, value) };
                    }

                    let c_prog = CString::new(path.as_bytes()).expect("CString::new failed");
                    let c_args: Vec<CString> = args
                        .iter()
                        .map(|a| CString::new(a.as_bytes()).expect("CString::new failed"))
                        .collect();

                    nix::unistd::execvp(&c_prog, &c_args).ok();
                    eprintln!("bash: {}: command not found", name);
                    std::process::exit(127);
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
                eprintln!("bash: {}", e);
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
                    match unsafe { nix::unistd::fork() } {
                        Ok(nix::unistd::ForkResult::Child) => {
                            let status = self.run_program(program);
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
            if self.continuing > 0 {
                self.continuing -= 1;
                continue;
            }

            self.vars.insert(clause.var.clone(), item);
            status = self.run_program(&clause.body);

            if self.returning {
                break;
            }
        }

        status
    }

    fn run_arith_for(&mut self, clause: &ArithForClause) -> i32 {
        if !clause.init.is_empty() {
            self.eval_arith_with_side_effects(&clause.init);
        }

        let mut status = 0;
        loop {
            if self.breaking > 0 {
                self.breaking -= 1;
                break;
            }

            if !clause.cond.is_empty() {
                let cond_val = self.eval_arith_with_side_effects(&clause.cond);
                if cond_val == 0 {
                    break;
                }
            }

            if self.continuing > 0 {
                self.continuing -= 1;
            } else {
                status = self.run_program(&clause.body);
                if self.returning {
                    break;
                }
            }

            if !clause.step.is_empty() {
                self.eval_arith_with_side_effects(&clause.step);
            }
        }
        status
    }

    fn run_while(&mut self, clause: &WhileClause) -> i32 {
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

            if self.continuing > 0 {
                self.continuing -= 1;
                continue;
            }

            status = self.run_program(&clause.body);

            if self.returning {
                break;
            }
        }
        status
    }

    fn run_until(&mut self, clause: &WhileClause) -> i32 {
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

            status = self.run_program(&clause.body);

            if self.returning {
                break;
            }
        }
        status
    }

    fn run_case(&mut self, clause: &CaseClause) -> i32 {
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());

        let word_expanded = self.expand_word_fields(&clause.word, &ifs).join(" ");

        let mut i = 0;
        while i < clause.items.len() {
            let item = &clause.items[i];
            let matched = item.patterns.iter().any(|pattern| {
                let pat_expanded = self.expand_word_single(pattern);
                case_pattern_match(&word_expanded, &pat_expanded)
            });

            if matched {
                let status = self.run_program(&item.body);
                match item.terminator {
                    CaseTerminator::Break => return status,
                    CaseTerminator::FallThrough => {
                        // Execute next body unconditionally
                        i += 1;
                        if i < clause.items.len() {
                            return self.run_program(&clause.items[i].body);
                        }
                        return status;
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
    fn run_conditional(&mut self, expr: &CondExpr) -> i32 {
        if self.eval_cond(expr) { 0 } else { 1 }
    }

    fn eval_cond(&mut self, expr: &CondExpr) -> bool {
        match expr {
            CondExpr::Word(w) => {
                let s = self.expand_word_single(w);
                !s.is_empty()
            }
            CondExpr::Not(e) => !self.eval_cond(e),
            CondExpr::And(a, b) => self.eval_cond(a) && self.eval_cond(b),
            CondExpr::Or(a, b) => self.eval_cond(a) || self.eval_cond(b),
            CondExpr::Unary(op, w) => {
                let val = self.expand_word_single(w);
                self.eval_cond_unary(op, &val)
            }
            CondExpr::Binary(left, op, right) => {
                let lval = self.expand_word_single(left);
                let rval = self.expand_word_single(right);
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
            "-r" | "-w" => std::path::Path::new(val).exists(),
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
            "-b" | "-c" | "-g" | "-k" | "-p" | "-t" | "-u" | "-G" | "-N" | "-O" | "-S" => {
                // Simplified: just check existence for most
                std::path::Path::new(val).exists()
            }
            "-v" => {
                // Variable is set
                self.vars.contains_key(val) || self.arrays.contains_key(val)
            }
            "-R" => {
                // Variable is nameref
                self.namerefs.contains_key(val)
            }
            _ => false,
        }
    }

    fn eval_cond_binary(&self, left: &str, op: &str, right: &str) -> bool {
        match op {
            "=" | "==" => {
                // Pattern matching (right side is a glob pattern)
                case_pattern_match(left, right)
            }
            "!=" => !case_pattern_match(left, right),
            "<" => left < right,
            ">" => left > right,
            "-eq" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a == b
            }
            "-ne" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a != b
            }
            "-lt" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a < b
            }
            "-le" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a <= b
            }
            "-gt" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a > b
            }
            "-ge" => {
                let a: i64 = left.parse().unwrap_or(0);
                let b: i64 = right.parse().unwrap_or(0);
                a >= b
            }
            "-nt" => {
                let a = std::fs::metadata(left).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(right).and_then(|m| m.modified()).ok();
                matches!((a, b), (Some(a), Some(b)) if a > b)
            }
            "-ot" => {
                let a = std::fs::metadata(left).and_then(|m| m.modified()).ok();
                let b = std::fs::metadata(right).and_then(|m| m.modified()).ok();
                matches!((a, b), (Some(a), Some(b)) if a < b)
            }
            "-ef" => {
                // Same device and inode
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    let a = std::fs::metadata(left).ok();
                    let b = std::fs::metadata(right).ok();
                    matches!((a, b), (Some(a), Some(b)) if a.dev() == b.dev() && a.ino() == b.ino())
                }
                #[cfg(not(unix))]
                false
            }
            "=~" => {
                // Regex matching
                match regex_lite::Regex::new(right) {
                    Ok(re) => re.is_match(left),
                    Err(_) => false,
                }
            }
            _ => false,
        }
    }

    /// Execute `(( arithmetic expression ))` — exit status 0 if result != 0.
    fn run_arithmetic(&mut self, expr: &str) -> i32 {
        let result = self.eval_arith_with_side_effects(expr);
        if result != 0 { 0 } else { 1 }
    }

    /// Evaluate arithmetic with side effects (assignments, ++, --).
    fn eval_arith_with_side_effects(&mut self, expr: &str) -> i64 {
        let expr = expr.trim();

        // Handle comma operator: eval left for side effects, return right
        if let Some(pos) = expr.rfind(',') {
            self.eval_arith_with_side_effects(&expr[..pos]);
            return self.eval_arith_with_side_effects(&expr[pos + 1..]);
        }

        // Handle assignment operators: =, +=, -=, *=, /=, %=
        for op in &[
            "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "=",
        ] {
            if let Some(pos) = find_arith_assign(expr, op) {
                let name = expr[..pos].trim().to_string();
                let rhs = &expr[pos + op.len()..];
                let rhs_val = self.eval_arith_with_side_effects(rhs);
                let result = if *op == "=" {
                    rhs_val
                } else {
                    let lhs_val: i64 = self
                        .vars
                        .get(&name)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    match *op {
                        "+=" => lhs_val + rhs_val,
                        "-=" => lhs_val - rhs_val,
                        "*=" => lhs_val * rhs_val,
                        "/=" => {
                            if rhs_val != 0 {
                                lhs_val / rhs_val
                            } else {
                                0
                            }
                        }
                        "%=" => {
                            if rhs_val != 0 {
                                lhs_val % rhs_val
                            } else {
                                0
                            }
                        }
                        _ => rhs_val,
                    }
                };
                self.set_var(&name, result.to_string());
                return result;
            }
        }

        // Handle ++ and -- (post and pre)
        let trimmed = expr.trim();
        if let Some(name) = trimmed.strip_suffix("++") {
            let name = name.trim();
            let val: i64 = self
                .vars
                .get(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            self.set_var(name, (val + 1).to_string());
            return val; // post-increment returns old value
        }
        if let Some(name) = trimmed.strip_suffix("--") {
            let name = name.trim();
            let val: i64 = self
                .vars
                .get(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            self.set_var(name, (val - 1).to_string());
            return val;
        }
        if let Some(name) = trimmed.strip_prefix("++") {
            let name = name.trim();
            let val: i64 = self
                .vars
                .get(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            self.set_var(name, (val + 1).to_string());
            return val + 1;
        }
        if let Some(name) = trimmed.strip_prefix("--") {
            let name = name.trim();
            let val: i64 = self
                .vars
                .get(name)
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            self.set_var(name, (val - 1).to_string());
            return val - 1;
        }

        // Fall through to regular evaluation
        self.eval_arith_expr(expr)
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
                    if !is_var_fd(redir) {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::File::create(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::Append => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if !is_var_fd(redir) {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::Input => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if !is_var_fd(redir) {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                    }

                    let file = std::fs::File::open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::DupOutput => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if target_str == "-" {
                        nix::unistd::close(fd).ok();
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                        nix::unistd::dup2(src_fd, fd).map_err(|e| e.to_string())?;
                    }
                }
                RedirectKind::DupInput => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if target_str == "-" {
                        nix::unistd::close(fd).ok();
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                        nix::unistd::dup2(src_fd, fd).map_err(|e| e.to_string())?;
                    }
                }
                RedirectKind::HereDoc(_) | RedirectKind::HereString => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

                    let content = format!("{}\n", target_str);

                    let (pipe_r, pipe_w) = nix::unistd::pipe().map_err(|e| e.to_string())?;
                    nix::unistd::write(&pipe_w, content.as_bytes()).map_err(|e| e.to_string())?;
                    let pipe_r_raw = pipe_r.as_raw_fd();
                    drop(pipe_w);
                    nix::unistd::dup2(pipe_r_raw, fd).map_err(|e| e.to_string())?;
                    drop(pipe_r);
                }
                RedirectKind::ReadWrite => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
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
/// Returns position only if left side is a valid variable name.
fn find_arith_assign(expr: &str, op: &str) -> Option<usize> {
    let bytes = expr.as_bytes();
    let op_bytes = op.as_bytes();
    // For "=" we need to exclude "==", "!=", "<=", ">="
    for i in 0..bytes.len() {
        if i + op_bytes.len() <= bytes.len() && &bytes[i..i + op_bytes.len()] == op_bytes {
            // Make sure this isn't part of ==, !=, <=, >=
            if op == "=" {
                if i > 0 && matches!(bytes[i - 1], b'!' | b'<' | b'>' | b'=') {
                    continue;
                }
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    continue;
                }
            }
            // Check left side is a valid variable name
            let left = expr[..i].trim();
            if !left.is_empty()
                && left.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !left.starts_with(|c: char| c.is_ascii_digit())
            {
                return Some(i);
            }
        }
    }
    None
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

fn pattern_match_impl(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    let mut ti = ti;
    let mut pi = pi;

    while pi < pattern.len() {
        match pattern[pi] {
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
                pi += 1;
                let negate = pi < pattern.len() && (pattern[pi] == '!' || pattern[pi] == '^');
                if negate {
                    pi += 1;
                }
                let mut matched = false;
                let ch = text[ti];
                while pi < pattern.len() && pattern[pi] != ']' {
                    if pi + 2 < pattern.len() && pattern[pi + 1] == '-' {
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
                    pi += 1;
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
