use super::*;

impl Shell {
    fn dup_error_message(src_fd: i32, e: &nix::Error) -> String {
        let msg = match *e {
            nix::Error::EBADF => "Bad file descriptor",
            nix::Error::EINVAL => "invalid value",
            _ => "Bad file descriptor",
        };
        format!("{}: {}", src_fd, msg)
    }

    #[cfg(unix)]
    pub(super) fn setup_redirections(
        &mut self,
        redirections: &[Redirection],
    ) -> Result<Vec<(i32, std::os::unix::io::RawFd)>, String> {
        use std::os::unix::io::{AsRawFd, IntoRawFd};

        let mut saved = Vec::new();
        let is_var_fd = |redir: &Redirection| matches!(&redir.fd, Some(RedirFd::Var(_)));

        for redir in redirections {
            // Expand redirect target without glob expansion
            let target_str = self.expand_word_single(&redir.target);

            // Check for ambiguous redirect (expansion contains IFS chars from variable)
            if !matches!(
                redir.kind,
                RedirectKind::HereDoc(_, _) | RedirectKind::HereString
            ) {
                let ifs = self
                    .vars
                    .get("IFS")
                    .cloned()
                    .unwrap_or_else(|| " \t\n".to_string());
                // If target has IFS characters AND came from a variable expansion,
                // it's an ambiguous redirect
                let has_var = redir
                    .target
                    .iter()
                    .any(|p| matches!(p, WordPart::Variable(_) | WordPart::Param(_)));
                if has_var && target_str.chars().any(|c| ifs.contains(c)) {
                    let raw = crate::ast::word_to_string(&redir.target);
                    self.restore_redirections(saved);
                    return Err(format!("{}: ambiguous redirect", raw));
                }
            }

            // Check for empty target (unset variable)
            if target_str.is_empty()
                && !matches!(
                    redir.kind,
                    RedirectKind::HereDoc(_, _) | RedirectKind::HereString
                )
                && redir
                    .target
                    .iter()
                    .any(|p| matches!(p, WordPart::Variable(_) | WordPart::Param(_)))
            {
                let raw = crate::ast::word_to_string(&redir.target);
                self.restore_redirections(saved);
                return Err(format!("{}: ambiguous redirect", raw));
            }

            match &redir.kind {
                RedirectKind::Output | RedirectKind::Clobber => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if !is_var_fd(redir)
                        && let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
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
                    // Clear CLOEXEC flag so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                }
                RedirectKind::Append => {
                    let fd = self.resolve_redir_fd(&redir.fd, 1);
                    if !is_var_fd(redir)
                        && let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
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
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
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
                        && let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
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
                        // Save fd before closing so it can be restored
                        if !is_var_fd(redir)
                            && let Ok(saved_fd) =
                                nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        self.coproc_checkfd(fd);
                        nix::unistd::close(fd).ok();
                    } else if let Some(src_str) = target_str.strip_suffix('-') {
                        // Move fd: dup src to fd, then close src
                        if let Ok(src_fd) = src_str.parse::<i32>() {
                            if let Ok(saved_fd) =
                                nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                            {
                                saved.push((fd, saved_fd));
                            }
                            nix::unistd::dup2(src_fd, fd)
                                .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if src_fd < 0 {
                            self.restore_redirections(saved);
                            return Err(format!("{}: ambiguous redirect", target_str));
                        }
                        if let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                    } else if redir.fd.is_none() {
                        // >&word where word is not a number and no explicit fd —
                        // redirect both stdout and stderr to the file
                        if let Ok(saved_fd1) = nix::unistd::dup(1) {
                            saved.push((1, saved_fd1));
                        }
                        if let Ok(saved_fd2) = nix::unistd::dup(2) {
                            saved.push((2, saved_fd2));
                        }
                        let file = std::fs::File::create(&target_str).map_err(|e| {
                            format!("{}: {}", target_str, Self::io_error_message(&e))
                        })?;
                        let raw = file.into_raw_fd();
                        nix::unistd::dup2(raw, 1).ok();
                        nix::unistd::dup2(raw, 2).ok();
                        if raw != 1 && raw != 2 {
                            nix::unistd::close(raw).ok();
                        }
                    }
                }
                RedirectKind::DupInput => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if target_str == "-" {
                        // Save fd before closing so it can be restored
                        if !is_var_fd(redir)
                            && let Ok(saved_fd) =
                                nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        self.coproc_checkfd(fd);
                        nix::unistd::close(fd).ok();
                    } else if let Some(src_str) = target_str.strip_suffix('-') {
                        if let Ok(src_fd) = src_str.parse::<i32>() {
                            if src_fd < 0 {
                                self.restore_redirections(saved);
                                return Err(format!("{}: ambiguous redirect", target_str));
                            }
                            if let Ok(saved_fd) =
                                nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                            {
                                saved.push((fd, saved_fd));
                            }
                            nix::unistd::dup2(src_fd, fd)
                                .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if src_fd < 0 {
                            self.restore_redirections(saved);
                            return Err(format!("{}: ambiguous redirect", target_str));
                        }
                        if let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(src_fd, &e))?;
                    }
                }
                RedirectKind::HereDoc(_, _) | RedirectKind::HereString => {
                    let fd = self.resolve_redir_fd(&redir.fd, 0);
                    if let Ok(saved_fd) =
                        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                    {
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
                    if let Ok(saved_fd) =
                        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                    {
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
                    if raw_fd != fd {
                        nix::unistd::dup2(raw_fd, fd).map_err(|e| e.to_string())?;
                        nix::unistd::close(raw_fd).ok();
                    }
                    // Clear CLOEXEC so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
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
    pub(super) fn setup_redirections(
        &self,
        _redirections: &[Redirection],
    ) -> Result<Vec<(i32, i32)>, String> {
        Ok(vec![])
    }

    #[cfg(unix)]
    pub(super) fn restore_redirections(&self, saved: Vec<(i32, std::os::unix::io::RawFd)>) {
        for (fd, saved_fd) in saved.into_iter().rev() {
            nix::unistd::dup2(saved_fd, fd).ok();
            nix::unistd::close(saved_fd).ok();
        }
    }

    #[cfg(not(unix))]
    pub(super) fn restore_redirections(&self, _saved: Vec<(i32, i32)>) {}
}
