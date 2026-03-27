use super::*;

impl Shell {
    /// Check for pending Unix signals and run their trap handlers
    pub fn check_pending_signals(&mut self) {
        #[cfg(unix)]
        {
            // Check signals 1-64
            for signum in 1..=64 {
                if take_pending_signal(signum) {
                    // Skip SIGCHLD during foreground waits — bash only fires
                    // SIGCHLD trap for background/async jobs, not foreground children
                    if signum == libc::SIGCHLD && self.in_foreground_wait {
                        continue;
                    }
                    // Convert signal number to name
                    let sig_name = match signum {
                        libc::SIGHUP => "HUP",
                        libc::SIGINT => "INT",
                        libc::SIGQUIT => "QUIT",
                        libc::SIGILL => "ILL",
                        libc::SIGTRAP => "TRAP",
                        libc::SIGABRT => "ABRT",
                        libc::SIGBUS => "BUS",
                        libc::SIGFPE => "FPE",
                        libc::SIGUSR1 => "USR1",
                        libc::SIGUSR2 => "USR2",
                        libc::SIGPIPE => "PIPE",
                        libc::SIGALRM => "ALRM",
                        libc::SIGTERM => "TERM",
                        libc::SIGCHLD => "CHLD",
                        libc::SIGCONT => "CONT",
                        libc::SIGWINCH => "WINCH",
                        _ => continue,
                    };
                    if let Some(handler) = self.traps.get(sig_name).cloned()
                        && !handler.is_empty()
                    {
                        // Set BASH_TRAPSIG to the signal number
                        let old_trapsig = self.vars.get("BASH_TRAPSIG").cloned();
                        self.vars
                            .insert("BASH_TRAPSIG".to_string(), signum.to_string());
                        let saved_status = self.last_status;
                        self.in_trap_handler += 1;
                        self.run_string(&handler);
                        self.in_trap_handler -= 1;
                        // If return was called in trap, propagate it only if inside a function
                        if self.returning {
                            if !self.func_names.is_empty() {
                                // Inside a function — propagate return
                                // POSIX interp 1602: "return" without argument in a trap action
                                // causes the function to return with the pre-trap exit status.
                                // "return N" with explicit argument uses N.
                                if !self.return_explicit_arg {
                                    self.last_status = saved_status;
                                }
                                if let Some(v) = old_trapsig {
                                    self.vars.insert("BASH_TRAPSIG".to_string(), v);
                                } else {
                                    self.vars.remove("BASH_TRAPSIG");
                                }
                                return;
                            }
                            // At top level — clear returning (trap handler return is local)
                            self.returning = false;
                        }
                        self.last_status = saved_status;
                        if let Some(v) = old_trapsig {
                            self.vars.insert("BASH_TRAPSIG".to_string(), v);
                        } else {
                            self.vars.remove("BASH_TRAPSIG");
                        }
                        // Re-install the signal handler (some systems reset to SIG_DFL)
                        install_signal_handler(signum);
                    }
                }
            }
        }
    }

    /// Execute the ERR trap if set and the command failed
    pub fn run_err_trap(&mut self) {
        if let Some(handler) = self.traps.get("ERR").cloned()
            && !handler.is_empty()
        {
            let saved = self.last_status;
            let saved_in_trap = self.in_trap_handler;
            self.in_trap_handler += 1;
            self.run_string(&handler);
            self.in_trap_handler = saved_in_trap;
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
        // Remove the trap to prevent recursive invocation (exit inside EXIT trap)
        let handler = self.traps.remove("EXIT").or_else(|| self.traps.remove("0"));
        if let Some(handler) = handler
            && !handler.is_empty()
        {
            self.run_string(&handler);
        }
    }
}
