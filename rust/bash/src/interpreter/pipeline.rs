use super::*;

/// Create a pipe, ensuring neither end reuses fd 0, 1, or 2.
/// When stdin/stdout/stderr are closed, pipe() can return low fds
/// that conflict with standard streams, causing pipeline deadlocks.
#[cfg(unix)]
fn safe_pipe() -> nix::Result<(std::os::unix::io::RawFd, std::os::unix::io::RawFd)> {
    use std::os::unix::io::IntoRawFd;
    let (r, w) = nix::unistd::pipe()?;
    let mut r_fd = r.into_raw_fd();
    let mut w_fd = w.into_raw_fd();
    // Move any fd that landed on 0, 1, or 2 to a higher number
    if r_fd < 3 {
        let new = nix::fcntl::fcntl(r_fd, nix::fcntl::FcntlArg::F_DUPFD(10)).unwrap_or(r_fd);
        if new != r_fd {
            nix::unistd::close(r_fd).ok();
            r_fd = new;
        }
    }
    if w_fd < 3 {
        let new = nix::fcntl::fcntl(w_fd, nix::fcntl::FcntlArg::F_DUPFD(10)).unwrap_or(w_fd);
        if new != w_fd {
            nix::unistd::close(w_fd).ok();
            w_fd = new;
        }
    }
    Ok((r_fd, w_fd))
}

impl Shell {
    pub(super) fn run_pipeline(&mut self, pipeline: &Pipeline) -> i32 {
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
            use std::os::unix::io::RawFd;

            let mut prev_read_fd: Option<RawFd> = None;
            let mut children: Vec<nix::unistd::Pid> = Vec::new();

            for (i, cmd) in pipeline.commands.iter().enumerate() {
                let is_last = i == pipeline.commands.len() - 1;

                let (read_fd, write_fd): (Option<RawFd>, Option<RawFd>) = if !is_last {
                    let (r, w) = safe_pipe().expect("pipe failed");
                    (Some(r), Some(w))
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
                        // Clear EXIT trap in subshell (pipeline children are subshells)
                        self.traps.remove("EXIT");
                        self.traps.remove("0");
                        // Restore SIGPIPE to default in pipeline children so that
                        // broken pipe silently kills the process (matching bash behavior)
                        unsafe {
                            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
                        }
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
                        // Run EXIT trap before exiting pipeline child
                        if let Some(handler) = self
                            .traps
                            .get("EXIT")
                            .or_else(|| self.traps.get("0"))
                            .cloned()
                            && !handler.is_empty()
                        {
                            self.run_string(&handler);
                        }
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
            // Flush stdout before forking to prevent buffered data from being
            // inherited by the child and written to the capture pipe.
            std::io::Write::flush(&mut std::io::stdout()).ok();

            let (pipe_r_raw, pipe_w_raw) = match safe_pipe() {
                Ok(p) => p,
                Err(_) => return String::new(),
            };

            match unsafe { nix::unistd::fork() } {
                Ok(nix::unistd::ForkResult::Child) => {
                    nix::unistd::close(pipe_r_raw).ok();
                    nix::unistd::dup2(pipe_w_raw, 1).ok();
                    nix::unistd::close(pipe_w_raw).ok();
                    // Command substitution does not inherit errexit or ERR trap
                    // (unless inherit_errexit/errtrace shopt is set or POSIX mode is on)
                    if !self.shopt_inherit_errexit && !self.opt_posix {
                        self.opt_errexit = false;
                    }
                    if !self.shopt_options.get("errtrace").copied().unwrap_or(false) {
                        self.traps.remove("ERR");
                    }
                    // Clear EXIT trap in command substitution subshell
                    self.traps.remove("EXIT");
                    self.traps.remove("0");
                    // Set comsub_line_offset so LINENO inside comsub reflects the script line
                    let lineno: usize = self
                        .vars
                        .get("LINENO")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(1);
                    self.comsub_line_offset = lineno.saturating_sub(1);
                    self.in_comsub = true;
                    let status = self.run_string(cmd_str);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                    std::process::exit(status);
                }
                Ok(nix::unistd::ForkResult::Parent { child }) => {
                    nix::unistd::close(pipe_w_raw).ok();
                    let mut output = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        match nix::unistd::read(pipe_r_raw, &mut buf) {
                            Ok(0) => break,
                            Ok(n) => output.extend_from_slice(&buf[..n]),
                            Err(_) => break,
                        }
                    }
                    nix::unistd::close(pipe_r_raw).ok();
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
}
