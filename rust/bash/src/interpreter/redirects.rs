use super::*;

impl Shell {
    fn dup_error_message(raw_target: &str, e: &nix::Error) -> String {
        let msg = match *e {
            nix::Error::EBADF => "Bad file descriptor",
            nix::Error::EINVAL => "invalid value",
            _ => "Bad file descriptor",
        };
        format!("{}: {}", raw_target, msg)
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
            // Get raw target text (before expansion) for error messages
            let raw_target = crate::ast::word_to_string(&redir.target);
            // Expand redirect target without glob expansion
            let target_str = self.expand_word_single(&redir.target);

            // Check for expansion errors (bad substitution, etc.) during heredoc/here-string expansion
            if matches!(
                redir.kind,
                RedirectKind::HereDoc(_, _) | RedirectKind::HereString
            ) && crate::expand::take_arith_error()
            {
                self.last_status = 1;
                self.restore_redirections(saved);
                return Err(String::new());
            }

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
                    let (fd, var_fail) = match self.resolve_redir_fd(&redir.fd, 1) {
                        Ok(fd) => (fd, false),
                        Err(fd) => (fd, true),
                    };
                    let alloc_failed = self.redir_alloc_failed;
                    self.redir_alloc_failed = false;
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
                        if let Err(e) = nix::unistd::dup2(raw_fd, fd) {
                            nix::unistd::close(raw_fd).ok();
                            if alloc_failed {
                                let io_msg = "Invalid argument";
                                self.restore_redirections(saved);
                                return Err(format!("{}: {}", target_str, io_msg));
                            }
                            return Err(e.to_string());
                        }
                        nix::unistd::close(raw_fd).ok();
                    }
                    // Clear CLOEXEC flag so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                    if var_fail {
                        self.last_status = 1;
                        self.restore_redirections(saved);
                        return Err(String::new());
                    }
                }
                RedirectKind::Append => {
                    let (fd, var_fail) = match self.resolve_redir_fd(&redir.fd, 1) {
                        Ok(fd) => (fd, false),
                        Err(fd) => (fd, true),
                    };
                    let alloc_failed = self.redir_alloc_failed;
                    self.redir_alloc_failed = false;
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
                        if let Err(e) = nix::unistd::dup2(raw_fd, fd) {
                            nix::unistd::close(raw_fd).ok();
                            if alloc_failed {
                                let io_msg = "Invalid argument";
                                self.restore_redirections(saved);
                                return Err(format!("{}: {}", target_str, io_msg));
                            }
                            return Err(e.to_string());
                        }
                        nix::unistd::close(raw_fd).ok();
                    }
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                    if var_fail {
                        self.last_status = 1;
                        self.restore_redirections(saved);
                        return Err(String::new());
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
                    let (fd, var_fail) = match self.resolve_redir_fd(&redir.fd, 0) {
                        Ok(fd) => (fd, false),
                        Err(fd) => (fd, true),
                    };
                    let alloc_failed = self.redir_alloc_failed;
                    self.redir_alloc_failed = false;
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
                        if let Err(e) = nix::unistd::dup2(raw_fd, fd) {
                            nix::unistd::close(raw_fd).ok();
                            if alloc_failed {
                                // The root cause is EINVAL from ulimit-limited
                                // F_DUPFD, so always report "Invalid argument".
                                let io_msg = "Invalid argument";
                                self.restore_redirections(saved);
                                return Err(format!("{}: {}", target_str, io_msg));
                            }
                            return Err(e.to_string());
                        }
                        nix::unistd::close(raw_fd).ok();
                    }
                    // Clear close-on-exec flag so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                    if var_fail {
                        self.last_status = 1;
                        self.restore_redirections(saved);
                        return Err(String::new());
                    }
                }
                RedirectKind::DupOutput => {
                    // For close operations ({v}>&-), read the variable's current
                    // value instead of allocating a new fd.
                    let fd = if target_str == "-" && is_var_fd(redir) {
                        match self.resolve_var_fd_for_close(&redir.fd) {
                            Ok(fd) => fd,
                            Err(name) => {
                                self.restore_redirections(saved);
                                return Err(format!("{}: ambiguous redirect", name));
                            }
                        }
                    } else {
                        self.resolve_redir_fd(&redir.fd, 1).unwrap_or_else(|fd| fd)
                    };
                    // Empty target from expansion is ambiguous redirect
                    if target_str.is_empty() {
                        self.restore_redirections(saved);
                        return Err(format!("{}: ambiguous redirect", raw_target));
                    }
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
                                .map_err(|e| Self::dup_error_message(&raw_target, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if src_fd < 0 {
                            self.restore_redirections(saved);
                            return Err(format!("{}: ambiguous redirect", target_str));
                        }
                        // Validate the source fd exists BEFORE saving, so that
                        // the save (F_DUPFD_CLOEXEC) doesn't allocate the same
                        // fd number as src_fd, which would mask the error.
                        if nix::fcntl::fcntl(src_fd, nix::fcntl::FcntlArg::F_GETFD).is_err() {
                            // Print the error BEFORE restoring redirections so
                            // that it goes through any already-setup redirect
                            // chain (e.g. 2>&1 piped to grep).
                            let msg = Self::dup_error_message(&raw_target, &nix::Error::EBADF);
                            eprintln!("{}: {}", self.error_prefix(), msg);
                            self.restore_redirections(saved);
                            return Err(String::new());
                        }
                        if let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(&raw_target, &e))?;
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
                    // For close operations ({v}<&-), read the variable's current
                    // value instead of allocating a new fd.
                    let fd = if target_str == "-" && is_var_fd(redir) {
                        match self.resolve_var_fd_for_close(&redir.fd) {
                            Ok(fd) => fd,
                            Err(name) => {
                                self.restore_redirections(saved);
                                return Err(format!("{}: ambiguous redirect", name));
                            }
                        }
                    } else {
                        self.resolve_redir_fd(&redir.fd, 0).unwrap_or_else(|fd| fd)
                    };
                    if target_str.is_empty() {
                        self.restore_redirections(saved);
                        return Err(format!("{}: ambiguous redirect", raw_target));
                    }
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
                                .map_err(|e| Self::dup_error_message(&raw_target, &e))?;
                            self.coproc_checkfd(src_fd);
                            nix::unistd::close(src_fd).ok();
                        }
                    } else if let Ok(src_fd) = target_str.parse::<i32>() {
                        if src_fd < 0 {
                            self.restore_redirections(saved);
                            return Err(format!("{}: ambiguous redirect", target_str));
                        }
                        // Validate the source fd exists BEFORE saving (same
                        // reasoning as the DupOutput case above).
                        if nix::fcntl::fcntl(src_fd, nix::fcntl::FcntlArg::F_GETFD).is_err() {
                            let msg = Self::dup_error_message(&raw_target, &nix::Error::EBADF);
                            eprintln!("{}: {}", self.error_prefix(), msg);
                            self.restore_redirections(saved);
                            return Err(String::new());
                        }
                        if let Ok(saved_fd) =
                            nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                        {
                            saved.push((fd, saved_fd));
                        }
                        nix::unistd::dup2(src_fd, fd)
                            .map_err(|e| Self::dup_error_message(&raw_target, &e))?;
                    }
                }
                RedirectKind::HereDoc(_, _) | RedirectKind::HereString => {
                    let (fd, var_readonly) = match self.resolve_redir_fd(&redir.fd, 0) {
                        Ok(fd) => (fd, false),
                        Err(fd) => (fd, true),
                    };
                    if let Ok(saved_fd) =
                        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(10))
                    {
                        saved.push((fd, saved_fd));
                    }

                    // Check for incomplete comsub in heredoc body
                    let target_str = if let Some(marker_pos) =
                        target_str.find("\x00INCOMPLETE_COMSUB:")
                    {
                        let after = &target_str[marker_pos + "\x00INCOMPLETE_COMSUB:".len()..];
                        let file_line: usize = after
                            .chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                            .parse()
                            .unwrap_or(0);
                        let name = self
                            .vars
                            .get("_BASH_SOURCE_FILE")
                            .or_else(|| self.positional.first())
                            .map(|s| s.as_str())
                            .unwrap_or("bash");
                        eprintln!(
                            "{}: command substitution: line {}: unexpected EOF while looking for matching `)'",
                            name, file_line
                        );
                        // Strip the marker, keep any content before it
                        target_str[..marker_pos].to_string()
                    } else {
                        target_str
                    };

                    // Use raw byte conversion for heredoc/herestring content
                    // so that chars like U+00CD (from $'\315') produce single bytes
                    let mut content_bytes = crate::builtins::string_to_raw_bytes(&target_str);
                    // For here-strings, always append a trailing newline.
                    // For here-documents, append a trailing newline only when
                    // the heredoc had at least one content line.  We detect
                    // this by checking if the redirect target word has any
                    // parts: an empty word (`[]`) means no body at all
                    // (delimiter on the first line or immediate EOF → 0-byte
                    // content), while a non-empty word (even `[Literal("")]`)
                    // means there was at least one content line and a trailing
                    // newline is needed.
                    let heredoc_had_body = !redir.target.is_empty();
                    if matches!(redir.kind, RedirectKind::HereString) || heredoc_had_body {
                        content_bytes.push(b'\n');
                    }

                    // Use an anonymous file (memfd) instead of a pipe to avoid
                    // blocking when heredoc content exceeds the pipe buffer
                    // size (~64KB).  The pipe approach would deadlock because
                    // write() blocks when the buffer is full and nobody is
                    // reading yet.  memfd_create gives us a seekable anonymous
                    // file with no filesystem footprint.
                    let memfd = nix::sys::memfd::memfd_create(
                        c"bash-heredoc",
                        nix::sys::memfd::MemFdCreateFlag::MFD_CLOEXEC,
                    )
                    .map_err(|e| e.to_string())?;
                    let mem_raw = memfd.as_raw_fd();
                    nix::unistd::write(&memfd, &content_bytes).map_err(|e| e.to_string())?;
                    nix::sys::stat::fstat(mem_raw).ok(); // no-op to keep borrow checker happy
                    // Seek back to the beginning so reads start from offset 0
                    nix::unistd::lseek(mem_raw, 0, nix::unistd::Whence::SeekSet)
                        .map_err(|e| e.to_string())?;
                    if mem_raw != fd {
                        nix::unistd::dup2(mem_raw, fd).map_err(|e| e.to_string())?;
                        drop(memfd);
                    } else {
                        // memfd is already the target fd — don't close it.
                        std::mem::forget(memfd);
                    }
                    if var_readonly {
                        self.last_status = 1;
                        self.restore_redirections(saved);
                        return Err(String::new());
                    }
                }
                RedirectKind::ReadWrite => {
                    let (fd, var_fail) = match self.resolve_redir_fd(&redir.fd, 0) {
                        Ok(fd) => (fd, false),
                        Err(fd) => (fd, true),
                    };
                    let alloc_failed = self.redir_alloc_failed;
                    self.redir_alloc_failed = false;
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
                        if let Err(e) = nix::unistd::dup2(raw_fd, fd) {
                            nix::unistd::close(raw_fd).ok();
                            if alloc_failed {
                                let io_msg = "Invalid argument";
                                self.restore_redirections(saved);
                                return Err(format!("{}: {}", target_str, io_msg));
                            }
                            return Err(e.to_string());
                        }
                        nix::unistd::close(raw_fd).ok();
                    }
                    // Clear CLOEXEC so child processes inherit this fd
                    nix::fcntl::fcntl(
                        fd,
                        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::empty()),
                    )
                    .ok();
                    if var_fail {
                        self.last_status = 1;
                        self.restore_redirections(saved);
                        return Err(String::new());
                    }
                }
                RedirectKind::ProcessSubIn | RedirectKind::ProcessSubOut => {
                    // Process substitution handled during word expansion
                }
            }
        }

        Ok(saved)
    }

    /// Resolve the fd for a close operation (`{v}>&-` or `{v}<&-`).
    /// Reads the current value of the variable instead of allocating a new fd.
    /// Returns `Err(varname)` when the variable is unset (for "ambiguous redirect").
    #[cfg(unix)]
    fn resolve_var_fd_for_close(&self, fd: &Option<RedirFd>) -> Result<i32, String> {
        match fd {
            Some(RedirFd::Number(n)) => Ok(*n),
            Some(RedirFd::Var(name)) => {
                // Resolve namerefs first, then read the variable's current value
                let val = if let Some(bracket) = name.find('[') {
                    let base = &name[..bracket];
                    let resolved_base = self.resolve_nameref(base);
                    let subscript = &name[bracket + 1..name.len() - 1];
                    if let Ok(idx) = subscript.parse::<usize>() {
                        self.arrays
                            .get(&resolved_base)
                            .and_then(|arr| arr.get(idx))
                            .and_then(|v| v.as_ref())
                            .cloned()
                    } else {
                        self.assoc_arrays
                            .get(&resolved_base)
                            .and_then(|m| m.get(subscript))
                            .cloned()
                    }
                } else {
                    let resolved = self.resolve_nameref(name);
                    self.vars.get(&resolved).cloned()
                };
                match val.and_then(|s| s.parse::<i32>().ok()) {
                    Some(fd) => Ok(fd),
                    None => Err(name.clone()),
                }
            }
            None => Ok(1), // shouldn't happen for {var} close, but fallback
        }
    }

    /// Resolve the fd for a redirection. Returns `Ok(fd)` on success.
    /// Returns `Err(fd)` when the variable is readonly — the fd is still
    /// allocated (so the file open / dup2 can proceed and create files)
    /// but the variable won't be assigned, and the caller must treat
    /// the redirection as failed after the I/O operation completes.
    #[cfg(unix)]
    fn resolve_redir_fd(&mut self, fd: &Option<RedirFd>, default: i32) -> Result<i32, i32> {
        match fd {
            Some(RedirFd::Number(n)) => Ok(*n),
            Some(RedirFd::Var(name)) => {
                // Resolve namerefs: if name is a nameref, follow it to the
                // real variable name so we assign/check-readonly on the target.
                let resolved_name = if name.contains('[') {
                    // Array subscripts: resolve the base name through namerefs
                    let bracket = name.find('[').unwrap();
                    let base = &name[..bracket];
                    let resolved_base = self.resolve_nameref(base);
                    if resolved_base != base {
                        format!("{}{}", resolved_base, &name[bracket..])
                    } else {
                        name.clone()
                    }
                } else {
                    self.resolve_nameref(name)
                };

                // For array subscripts like fd[0], check readonly on the base name
                let base_name = if let Some(bracket) = resolved_name.find('[') {
                    &resolved_name[..bracket]
                } else {
                    resolved_name.as_str()
                };
                let is_readonly = self.readonly_vars.contains(base_name);
                // Auto-allocate fd >= 10 using F_DUPFD on a known-open fd.
                // This correctly respects ulimit -n (unlike scanning with dup).
                match nix::fcntl::fcntl(0, nix::fcntl::FcntlArg::F_DUPFD(10)) {
                    Ok(candidate) => {
                        // We got a valid fd >= 10.  Close it immediately — we
                        // only needed to discover the number.  The actual
                        // open/dup2 will happen in the caller.
                        nix::unistd::close(candidate).ok();

                        if is_readonly {
                            eprintln!("{}: {}: readonly variable", self.error_prefix(), name);
                            eprintln!(
                                "{}: {}: cannot assign fd to variable",
                                self.error_prefix(),
                                name
                            );
                            return Err(candidate);
                        }
                        // Handle array subscript: {fd[0]} → set arrays["fd"][0]
                        if let Some(bracket) = resolved_name.find('[') {
                            let base = &resolved_name[..bracket];
                            let subscript = &resolved_name[bracket + 1..resolved_name.len() - 1];
                            if let Ok(idx) = subscript.parse::<usize>() {
                                let arr = self.arrays.entry(base.to_string()).or_default();
                                while arr.len() <= idx {
                                    arr.push(None);
                                }
                                arr[idx] = Some(candidate.to_string());
                            } else {
                                // Non-numeric subscript — treat as assoc array
                                let assoc = self.assoc_arrays.entry(base.to_string()).or_default();
                                assoc.insert(subscript.to_string(), candidate.to_string());
                            }
                        } else {
                            self.vars
                                .insert(resolved_name.clone(), candidate.to_string());
                        }
                        // Track this fd for varredir_close: if the shopt is
                        // enabled, non-exec redirections will close it when
                        // the command finishes.
                        if self
                            .shopt_options
                            .get("varredir_close")
                            .copied()
                            .unwrap_or(false)
                        {
                            self.varredir_close_fds.push(candidate);
                        }
                        Ok(candidate)
                    }
                    Err(e) => {
                        // F_DUPFD failed — likely ulimit too low for fd >= 10
                        if is_readonly {
                            eprintln!("{}: {}: readonly variable", self.error_prefix(), name);
                            eprintln!(
                                "{}: {}: cannot assign fd to variable",
                                self.error_prefix(),
                                name
                            );
                            Err(default)
                        } else {
                            let msg = match e {
                                nix::Error::EINVAL => "cannot duplicate fd: Invalid argument",
                                nix::Error::EMFILE => "cannot duplicate fd: Too many open files",
                                _ => "cannot duplicate fd: Bad file descriptor",
                            };
                            // Bash prints the "redirection error" without a line
                            // number, using just the script name prefix.
                            let script_name = self
                                .vars
                                .get("_BASH_SOURCE_FILE")
                                .or_else(|| self.positional.first())
                                .cloned()
                                .unwrap_or_else(|| "bash".to_string());
                            eprintln!("{}: redirection error: {}", script_name, msg);
                            // Return -1 so subsequent dup2 fails and produces
                            // the target-file error that bash emits.
                            self.redir_alloc_failed = true;
                            Err(-1)
                        }
                    }
                }
            }
            None => Ok(default),
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
    pub(super) fn restore_redirections(&mut self, saved: Vec<(i32, std::os::unix::io::RawFd)>) {
        // When varredir_close is enabled, close any fds that were allocated
        // by {var} redirections during this command's setup.
        let varredir_fds = std::mem::take(&mut self.varredir_close_fds);
        for fd in varredir_fds {
            nix::unistd::close(fd).ok();
        }
        for (fd, saved_fd) in saved.into_iter().rev() {
            nix::unistd::dup2(saved_fd, fd).ok();
            nix::unistd::close(saved_fd).ok();
        }
    }

    #[cfg(not(unix))]
    pub(super) fn restore_redirections(&self, _saved: Vec<(i32, i32)>) {}
}
