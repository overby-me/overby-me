use crate::ast::*;
use crate::builtins::{self, BuiltinFn};
use crate::expand;
use crate::parser::Parser;
use std::collections::{HashMap, HashSet};

pub struct Shell {
    pub vars: HashMap<String, String>,
    pub exports: HashMap<String, String>,
    pub readonly_vars: HashSet<String>,
    pub functions: HashMap<String, CompoundCommand>,
    pub positional: Vec<String>,
    pub last_status: i32,
    pub returning: bool,
    pub breaking: i32,
    pub continuing: i32,
    pub in_condition: bool,
    pub dir_stack: Vec<String>,

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
        vars.insert("BASH_VERSION".to_string(), "0.1.0(1)-rust".to_string());
        vars.insert("BASH_VERSINFO".to_string(), "0".to_string());
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
            functions: HashMap::new(),
            positional: vec!["bash".to_string()],
            last_status: 0,
            returning: false,
            breaking: 0,
            continuing: 0,
            in_condition: false,
            dir_stack: Vec::new(),
            opt_errexit: false,
            opt_nounset: false,
            opt_xtrace: false,
            opt_pipefail: false,
            opt_noclobber: false,
            opt_noglob: false,
            opt_noexec: false,
            shopt_nullglob: false,
            shopt_extglob: false,
            builtins: builtins::builtins(),
        }
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
                // Flush stdout before errexit
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
        // When there are && or || operators, the non-final commands
        // are in a "condition" context and exempt from errexit.
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
                        // Set up stdin from previous pipe
                        if let Some(fd) = prev_read_fd {
                            nix::unistd::dup2(fd, 0).ok();
                            nix::unistd::close(fd).ok();
                        }
                        // Set up stdout to next pipe
                        if let Some(fd) = write_fd {
                            nix::unistd::dup2(fd, 1).ok();
                            nix::unistd::close(fd).ok();
                        }
                        // Close unused read end
                        if let Some(fd) = read_fd {
                            nix::unistd::close(fd).ok();
                        }

                        let status = self.run_command(cmd);
                        std::process::exit(status);
                    }
                    Ok(nix::unistd::ForkResult::Parent { child }) => {
                        children.push(child);

                        // Close previous read end
                        if let Some(fd) = prev_read_fd {
                            nix::unistd::close(fd).ok();
                        }
                        // Close write end (parent doesn't need it)
                        if let Some(fd) = write_fd {
                            nix::unistd::close(fd).ok();
                        }
                        // Save read end for next iteration
                        prev_read_fd = read_fd;
                    }
                    Err(e) => {
                        eprintln!("bash: fork: {}", e);
                        return 1;
                    }
                }
            }

            // Wait for all children
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

            let status = if self.opt_pipefail {
                // Return the last non-zero status, or 0 if all succeeded
                statuses
                    .iter()
                    .rev()
                    .find(|&&s| s != 0)
                    .copied()
                    .unwrap_or(0)
            } else {
                // Return status of last command
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

    /// Execute a command string and capture its stdout (for command substitution).
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
                    // Redirect stdout to pipe
                    nix::unistd::dup2(pipe_w_raw, 1).ok();
                    drop(pipe_w);
                    let status = self.run_string(cmd_str);
                    // Flush stdout before exit
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    std::process::exit(status);
                }
                Ok(nix::unistd::ForkResult::Parent { child }) => {
                    drop(pipe_w);
                    // Read all output from pipe
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
                    nix::sys::wait::waitpid(child, None).ok();
                    let mut s = String::from_utf8_lossy(&output).to_string();
                    // Remove trailing newlines (bash behavior)
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

    fn expand_word_fields(&mut self, word: &Word, ifs: &str) -> Vec<String> {
        let vars = self.vars.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word(word, &vars, &positional, last_status, ifs, &mut cmd_sub)
    }

    fn expand_word_single(&mut self, word: &Word) -> String {
        let vars = self.vars.clone();
        let positional = self.positional.clone();
        let last_status = self.last_status;
        let mut cmd_sub = |cmd: &str| -> String { self.capture_output(cmd) };
        expand::expand_word_nosplit(word, &vars, &positional, last_status, &mut cmd_sub)
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
                let value = assign
                    .value
                    .as_ref()
                    .map(|w| self.expand_word_single(w))
                    .unwrap_or_default();

                if expanded_words.is_empty() {
                    // No command - set shell variable
                    if assign.append {
                        let existing = self.vars.get(&assign.name).cloned().unwrap_or_default();
                        self.vars
                            .insert(assign.name.clone(), format!("{}{}", existing, value));
                    } else {
                        self.vars.insert(assign.name.clone(), value.clone());
                    }
                    // If variable is already exported, update the export
                    if self.exports.contains_key(&assign.name) {
                        self.exports.insert(assign.name.clone(), value.clone());
                        unsafe { std::env::set_var(&assign.name, &value) };
                    }
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
            self.run_function(&func_body, args)
        } else if let Some(builtin) = self.builtins.get(command_name.as_str()).copied() {
            // Temporarily set exports for command prefix assignments
            let prefix_exports: Vec<(String, String)> = if !expanded_words.is_empty() {
                cmd.assignments
                    .iter()
                    .map(|a| {
                        let v = a
                            .value
                            .as_ref()
                            .map(|w| self.expand_word_single(w))
                            .unwrap_or_default();
                        (a.name.clone(), v)
                    })
                    .collect()
            } else {
                vec![]
            };

            // Save and set prefix assignments
            let saved: Vec<(String, Option<String>)> = prefix_exports
                .iter()
                .map(|(k, v)| {
                    let old = self.vars.get(k).cloned();
                    self.vars.insert(k.clone(), v.clone());
                    (k.clone(), old)
                })
                .collect();

            let result = builtin(self, args);

            // Restore prefix assignments (only if there was a command)
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

        // Restore redirections
        self.restore_redirections(saved_fds);

        self.last_status = status;
        status
    }

    fn run_function(&mut self, body: &CompoundCommand, args: &[String]) -> i32 {
        let saved_positional = self.positional.clone();
        let prog = self.positional.first().cloned().unwrap_or_default();
        self.positional = vec![prog];
        self.positional.extend_from_slice(args);

        let status = self.run_compound_command(body);

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
                    // Set prefix assignments as environment variables
                    for assign in assignments {
                        let value = assign
                            .value
                            .as_ref()
                            .map(|w| self.expand_word_single(w))
                            .unwrap_or_default();
                        unsafe { std::env::set_var(&assign.name, &value) };
                    }

                    // Set exported variables
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
            CompoundCommand::While(clause) => self.run_while(clause),
            CompoundCommand::Until(clause) => self.run_until(clause),
            CompoundCommand::Case(clause) => self.run_case(clause),
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
        } else {
            // Default: iterate over positional parameters
            if self.positional.len() > 1 {
                self.positional[1..].to_vec()
            } else {
                vec![]
            }
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

        for item in &clause.items {
            for pattern in &item.patterns {
                let pat_expanded = self.expand_word_single(pattern);
                if case_pattern_match(&word_expanded, &pat_expanded) {
                    return self.run_program(&item.body);
                }
            }
        }

        0
    }

    #[cfg(unix)]
    fn setup_redirections(
        &mut self,
        redirections: &[Redirection],
    ) -> Result<Vec<(i32, std::os::unix::io::RawFd)>, String> {
        use std::os::unix::io::{AsRawFd, IntoRawFd};

        let mut saved = Vec::new();

        for redir in redirections {
            let target_str = self.expand_word_single(&redir.target);

            match &redir.kind {
                RedirectKind::Output | RedirectKind::Clobber => {
                    let fd = redir.fd.unwrap_or(1);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

                    let file = std::fs::File::create(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::Append => {
                    let fd = redir.fd.unwrap_or(1);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

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
                    let fd = redir.fd.unwrap_or(0);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

                    let file = std::fs::File::open(&target_str)
                        .map_err(|e| format!("{}: {}", target_str, e))?;
                    let raw_fd = file.into_raw_fd();
                    nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                    nix::unistd::close(raw_fd).ok();
                }
                RedirectKind::DupOutput => {
                    let fd = redir.fd.unwrap_or(1);
                    if target_str == "-" {
                        nix::unistd::close(fd).ok();
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                        nix::unistd::dup2(src_fd, fd).map_err(|e| e.to_string())?;
                    }
                }
                RedirectKind::DupInput => {
                    let fd = redir.fd.unwrap_or(0);
                    if target_str == "-" {
                        nix::unistd::close(fd).ok();
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                        saved.push((fd, saved_fd));
                        nix::unistd::dup2(src_fd, fd).map_err(|e| e.to_string())?;
                    }
                }
                RedirectKind::HereDoc(_) | RedirectKind::HereString => {
                    let fd = redir.fd.unwrap_or(0);
                    let saved_fd = nix::unistd::dup(fd).map_err(|e| e.to_string())?;
                    saved.push((fd, saved_fd));

                    let content = format!("{}\n", target_str);

                    // Create a pipe and write the content
                    let (pipe_r, pipe_w) = nix::unistd::pipe().map_err(|e| e.to_string())?;
                    nix::unistd::write(&pipe_w, content.as_bytes()).map_err(|e| e.to_string())?;
                    let pipe_r_raw = pipe_r.as_raw_fd();
                    drop(pipe_w);
                    nix::unistd::dup2(pipe_r_raw, fd).map_err(|e| e.to_string())?;
                    drop(pipe_r);
                }
                RedirectKind::ReadWrite => {
                    let fd = redir.fd.unwrap_or(0);
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
            }
        }

        Ok(saved)
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
