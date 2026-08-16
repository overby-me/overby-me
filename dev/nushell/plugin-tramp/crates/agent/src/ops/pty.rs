//! PTY (pseudo-terminal) operations for the tramp-agent RPC server.
//!
//! Implements the following RPC methods:
//!
//! | Method              | Description                                        |
//! |---------------------|----------------------------------------------------|
//! | `process.start_pty` | Start a process with a PTY, return a handle         |
//! | `process.resize`    | Send a window size change to a PTY process          |
//!
//! PTY processes reuse the existing `process.read`, `process.write`, and
//! `process.kill` methods from the [`super::process`] module — the PTY
//! master fd is wrapped in tokio's `AsyncFd` and presented as regular
//! stdout/stdin to the managed process infrastructure.
//!
//! ## Platform support
//!
//! PTY support is only available on Unix platforms (Linux, macOS, FreeBSD).
//! On non-Unix platforms, `process.start_pty` returns an error.

use rmpv::Value;

use crate::rpc::Response;

// ---------------------------------------------------------------------------
// Unix PTY implementation
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::io;
    use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rmpv::Value;
    use tokio::sync::Mutex;

    use crate::rpc::{Response, error_code};

    /// Global counter for PTY process handles.
    /// Uses a separate range (starting at 1_000_000) from regular process
    /// handles to avoid collisions.
    static NEXT_PTY_HANDLE: AtomicU64 = AtomicU64::new(1_000_000);

    /// Maximum bytes to read from the PTY master in a single read call.
    const READ_BUF_SIZE: usize = 64 * 1024;

    // -----------------------------------------------------------------------
    // libc helpers
    // -----------------------------------------------------------------------

    /// Open a PTY master/slave pair using `openpty(3)`.
    ///
    /// Returns `(master_fd, slave_fd)` on success.
    fn openpty() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut master_raw: RawFd = -1;
        let mut slave_raw: RawFd = -1;

        let ret = unsafe {
            libc::openpty(
                &mut master_raw,
                &mut slave_raw,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };

        if ret != 0 {
            return Err(io::Error::last_os_error());
        }

        // Safety: openpty returned valid fds on success.
        let master = unsafe { OwnedFd::from_raw_fd(master_raw) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave_raw) };

        Ok((master, slave))
    }

    /// Set the window size on a PTY master fd.
    fn set_winsize(fd: RawFd, rows: u16, cols: u16) -> io::Result<()> {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let ret = unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Set a file descriptor to non-blocking mode.
    fn set_nonblocking(fd: RawFd) -> io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        let ret = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Async wrapper for the PTY master fd
    // -----------------------------------------------------------------------

    /// An async wrapper around a PTY master file descriptor.
    ///
    /// Uses tokio's `AsyncFd` for readiness-based async I/O on the raw fd.
    struct AsyncPtyMaster {
        inner: tokio::io::unix::AsyncFd<OwnedFd>,
    }

    impl AsyncPtyMaster {
        /// Wrap a PTY master fd for async I/O.
        ///
        /// The fd is set to non-blocking mode before wrapping.
        fn new(fd: OwnedFd) -> io::Result<Self> {
            set_nonblocking(fd.as_raw_fd())?;
            let inner = tokio::io::unix::AsyncFd::new(fd)?;
            Ok(Self { inner })
        }

        /// Get the raw fd (for ioctl calls like TIOCSWINSZ).
        fn as_raw_fd(&self) -> RawFd {
            self.inner.get_ref().as_raw_fd()
        }

        /// Read available data from the PTY master.
        ///
        /// Returns `Ok(bytes)` with the data read, or `Ok(vec![])` if no
        /// data is available within the timeout.
        async fn read_available(&self, timeout: std::time::Duration) -> io::Result<Vec<u8>> {
            let mut buf = vec![0u8; READ_BUF_SIZE];

            match tokio::time::timeout(timeout, self.inner.readable()).await {
                Ok(Ok(mut guard)) => {
                    match guard.try_io(|inner| {
                        let fd = inner.get_ref().as_raw_fd();
                        let n = unsafe {
                            libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                        };
                        if n < 0 {
                            Err(io::Error::last_os_error())
                        } else {
                            Ok(n as usize)
                        }
                    }) {
                        Ok(Ok(n)) => {
                            buf.truncate(n);
                            Ok(buf)
                        }
                        Ok(Err(e)) => {
                            // EIO is returned when the slave side is closed
                            // (child exited).
                            if e.raw_os_error() == Some(libc::EIO) {
                                Ok(vec![])
                            } else {
                                Err(e)
                            }
                        }
                        Err(_would_block) => Ok(vec![]),
                    }
                }
                Ok(Err(e)) => Err(e),
                Err(_timeout) => Ok(vec![]),
            }
        }

        /// Write data to the PTY master (i.e. send it to the child's stdin).
        async fn write_all(&self, data: &[u8]) -> io::Result<()> {
            let mut offset = 0;
            while offset < data.len() {
                let mut guard = self.inner.writable().await?;
                match guard.try_io(|inner| {
                    let fd = inner.get_ref().as_raw_fd();
                    let remaining = &data[offset..];
                    let n = unsafe {
                        libc::write(
                            fd,
                            remaining.as_ptr() as *const libc::c_void,
                            remaining.len(),
                        )
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    Ok(Ok(n)) => {
                        offset += n;
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_would_block) => continue,
                }
            }
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Managed PTY process
    // -----------------------------------------------------------------------

    /// A managed PTY process with its master fd and child PID.
    struct ManagedPtyProcess {
        /// The PTY master fd for I/O and ioctls.
        master: AsyncPtyMaster,
        /// The child process PID.
        pid: u32,
        /// Accumulated output that hasn't been read yet.
        output_buf: Vec<u8>,
        /// Whether the child has exited.
        exited: bool,
        /// Exit code if the child has exited.
        exit_code: Option<i32>,
    }

    /// Shared PTY process table.
    pub struct PtyTable {
        processes: Mutex<HashMap<u64, ManagedPtyProcess>>,
    }

    impl PtyTable {
        pub fn new() -> Self {
            Self {
                processes: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for PtyTable {
        fn default() -> Self {
            Self::new()
        }
    }

    // -----------------------------------------------------------------------
    // Helper: extract parameters
    // -----------------------------------------------------------------------

    fn get_str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Response> {
        params
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_str())
            })
            .ok_or_else(|| {
                Response::err(
                    0,
                    error_code::INVALID_PARAMS,
                    format!("missing or invalid parameter: {key}"),
                )
            })
    }

    fn get_str_array_param<'a>(params: &'a Value, key: &str) -> Option<Vec<&'a str>> {
        params.as_map().and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        })
    }

    fn get_u64_param(params: &Value, key: &str) -> Result<u64, Response> {
        params
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some(key))
                    .and_then(|(_, v)| v.as_u64())
            })
            .ok_or_else(|| {
                Response::err(
                    0,
                    error_code::INVALID_PARAMS,
                    format!("missing or invalid parameter: {key}"),
                )
            })
    }

    fn get_u16_param(params: &Value, key: &str) -> Result<u16, Response> {
        let val = get_u64_param(params, key)?;
        u16::try_from(val).map_err(|_| {
            Response::err(
                0,
                error_code::INVALID_PARAMS,
                format!("parameter '{key}' value {val} out of range for u16"),
            )
        })
    }

    fn get_optional_str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
        params.as_map().and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_str())
        })
    }

    fn get_env_param(params: &Value, key: &str) -> Option<Vec<(String, String)>> {
        params.as_map().and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_map())
                .map(|pairs| {
                    pairs
                        .iter()
                        .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.as_str()?.to_owned())))
                        .collect()
                })
        })
    }

    fn get_bin_param<'a>(params: &'a Value, key: &str) -> Option<&'a [u8]> {
        params.as_map().and_then(|m| {
            m.iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .and_then(|(_, v)| v.as_slice())
        })
    }

    // -----------------------------------------------------------------------
    // Helper: check child status via waitpid (non-blocking)
    // -----------------------------------------------------------------------

    /// Non-blocking check if a child process has exited.
    ///
    /// Returns `Some(exit_code)` if the child has exited, `None` otherwise.
    fn try_wait_pid(pid: u32) -> Option<i32> {
        let mut status: libc::c_int = 0;
        let ret = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if ret > 0 {
            if libc::WIFEXITED(status) {
                Some(libc::WEXITSTATUS(status))
            } else if libc::WIFSIGNALED(status) {
                // Killed by signal — return 128 + signal number (convention).
                Some(128 + libc::WTERMSIG(status))
            } else {
                Some(-1)
            }
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // RPC method handlers
    // -----------------------------------------------------------------------

    /// Synchronous helper that performs the PTY allocation, fork, and exec.
    ///
    /// All raw pointer work (`*const libc::c_char`) is confined to this
    /// synchronous function so that the `async fn start_pty` never holds
    /// non-`Send` types across an `.await` point.
    ///
    /// On success returns `Ok((master_fd, child_pid))`.
    fn fork_pty(
        program: &str,
        args: &[&str],
        cwd: Option<&str>,
        env_vars: Option<&Vec<(String, String)>>,
        rows: u16,
        cols: u16,
    ) -> Result<(OwnedFd, u32), String> {
        // 1. Create the PTY pair.
        let (master_fd, slave_fd) = openpty().map_err(|e| format!("failed to open PTY: {e}"))?;

        // 2. Set initial window size.
        let master_raw = master_fd.as_raw_fd();
        set_winsize(master_raw, rows, cols)
            .map_err(|e| format!("failed to set PTY window size: {e}"))?;

        // 3. Prepare C strings for execvp.
        let slave_raw = slave_fd.as_raw_fd();

        let c_program =
            std::ffi::CString::new(program).map_err(|e| format!("invalid program name: {e}"))?;

        let c_args: Vec<std::ffi::CString> = {
            let mut v = vec![c_program.clone()];
            for arg in args {
                v.push(std::ffi::CString::new(*arg).map_err(|e| format!("invalid argument: {e}"))?);
            }
            v
        };

        let c_arg_ptrs: Vec<*const libc::c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        let c_env: Option<Vec<std::ffi::CString>> = env_vars.map(|vars| {
            vars.iter()
                .filter_map(|(k, v)| std::ffi::CString::new(format!("{k}={v}")).ok())
                .collect()
        });

        let c_cwd: Option<std::ffi::CString> = cwd.and_then(|d| std::ffi::CString::new(d).ok());

        // 4. Fork.
        let pid = unsafe { libc::fork() };

        if pid < 0 {
            return Err(format!("fork failed: {}", io::Error::last_os_error()));
        }

        if pid == 0 {
            // ---- Child process ----
            unsafe {
                libc::setsid();
                libc::ioctl(slave_raw, libc::TIOCSCTTY, 0);

                libc::dup2(slave_raw, libc::STDIN_FILENO);
                libc::dup2(slave_raw, libc::STDOUT_FILENO);
                libc::dup2(slave_raw, libc::STDERR_FILENO);

                if slave_raw > libc::STDERR_FILENO {
                    libc::close(slave_raw);
                }
                libc::close(master_raw);

                if let Some(ref dir) = c_cwd {
                    libc::chdir(dir.as_ptr());
                }

                if let Some(ref vars) = c_env {
                    for var in vars {
                        libc::putenv(var.as_ptr() as *mut libc::c_char);
                    }
                }

                libc::execvp(c_program.as_ptr(), c_arg_ptrs.as_ptr());
                libc::_exit(127);
            }
        }

        // ---- Parent process ----
        // Close the slave fd — we only use the master.
        drop(slave_fd);

        Ok((master_fd, pid as u32))
    }

    /// `process.start_pty` — start a process with a pseudo-terminal.
    ///
    /// This allocates a PTY pair, forks a child process with the slave
    /// PTY as its controlling terminal, and returns a handle that can be
    /// used with `process.read`, `process.write`, `process.resize`, and
    /// `process.kill`.
    ///
    /// Params:
    /// - `program`: the program to execute (string, required)
    /// - `args`: arguments (array of strings, optional, default `[]`)
    /// - `cwd`: working directory (string, optional)
    /// - `env`: environment variables to set (map of string→string, optional)
    /// - `rows`: initial terminal rows (u16, optional, default 24)
    /// - `cols`: initial terminal columns (u16, optional, default 80)
    ///
    /// Result: `{ handle: <u64>, pid: <u64> }`
    pub async fn start_pty(id: u64, params: &Value, table: &PtyTable) -> Response {
        let program = match get_str_param(params, "program") {
            Ok(p) => p,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let args = get_str_array_param(params, "args").unwrap_or_default();
        let cwd = get_optional_str_param(params, "cwd");
        let env_vars = get_env_param(params, "env");
        let rows = params
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("rows"))
                    .and_then(|(_, v)| v.as_u64())
                    .and_then(|v| u16::try_from(v).ok())
            })
            .unwrap_or(24);
        let cols = params
            .as_map()
            .and_then(|m| {
                m.iter()
                    .find(|(k, _)| k.as_str() == Some("cols"))
                    .and_then(|(_, v)| v.as_u64())
                    .and_then(|v| u16::try_from(v).ok())
            })
            .unwrap_or(80);

        // All fork/exec work is done synchronously in `fork_pty` so that
        // no non-Send raw pointers live across await points.
        let (master_fd, child_pid) =
            match fork_pty(program, &args, cwd, env_vars.as_ref(), rows, cols) {
                Ok(pair) => pair,
                Err(msg) => {
                    return Response::err(id, error_code::IO_ERROR, msg);
                }
            };

        // Wrap the master fd for async I/O.
        let master = match AsyncPtyMaster::new(master_fd) {
            Ok(m) => m,
            Err(e) => {
                // Kill the child since we can't communicate with it.
                unsafe { libc::kill(child_pid as libc::pid_t, libc::SIGKILL) };
                return Response::err(
                    id,
                    error_code::IO_ERROR,
                    format!("failed to set up async PTY master: {e}"),
                );
            }
        };

        let handle = NEXT_PTY_HANDLE.fetch_add(1, Ordering::Relaxed);
        let managed = ManagedPtyProcess {
            master,
            pid: child_pid,
            output_buf: Vec::new(),
            exited: false,
            exit_code: None,
        };

        table.processes.lock().await.insert(handle, managed);

        Response::ok(
            id,
            Value::Map(vec![
                (
                    Value::String("handle".into()),
                    Value::Integer(handle.into()),
                ),
                (
                    Value::String("pid".into()),
                    Value::Integer((child_pid as u64).into()),
                ),
            ]),
        )
    }

    /// `process.resize` — send a window size change to a PTY process.
    ///
    /// Params:
    /// - `handle`: PTY process handle (u64, required)
    /// - `rows`: new terminal rows (u16, required)
    /// - `cols`: new terminal columns (u16, required)
    ///
    /// Result: `{}` (empty map on success).
    pub async fn resize(id: u64, params: &Value, table: &PtyTable) -> Response {
        let handle = match get_u64_param(params, "handle") {
            Ok(h) => h,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let rows = match get_u16_param(params, "rows") {
            Ok(r) => r,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let cols = match get_u16_param(params, "cols") {
            Ok(c) => c,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let processes = table.processes.lock().await;

        let Some(managed) = processes.get(&handle) else {
            return Response::err(
                id,
                error_code::NOT_FOUND,
                format!("no PTY process with handle {handle}"),
            );
        };

        if let Err(e) = set_winsize(managed.master.as_raw_fd(), rows, cols) {
            return Response::err(
                id,
                error_code::IO_ERROR,
                format!("failed to resize PTY: {e}"),
            );
        }

        // Send SIGWINCH to the child process group so it picks up the
        // new window size.
        unsafe {
            libc::kill(-(managed.pid as libc::pid_t), libc::SIGWINCH);
        }

        Response::ok(id, Value::Map(vec![]))
    }

    /// `process.read` for PTY processes — read buffered output from the PTY.
    ///
    /// This is invoked by the main dispatch when the handle falls in the
    /// PTY range.
    ///
    /// Result: `{ stdout: <binary>, stderr: <binary (always empty)>,
    ///   running: <bool>, exit_code: <integer|null> }`
    pub async fn read(id: u64, params: &Value, table: &PtyTable) -> Response {
        let handle = match get_u64_param(params, "handle") {
            Ok(h) => h,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let mut processes = table.processes.lock().await;

        let Some(managed) = processes.get_mut(&handle) else {
            return Response::err(
                id,
                error_code::NOT_FOUND,
                format!("no PTY process with handle {handle}"),
            );
        };

        // Read available data from the PTY master.
        match managed
            .master
            .read_available(std::time::Duration::from_millis(10))
            .await
        {
            Ok(data) if !data.is_empty() => {
                managed.output_buf.extend_from_slice(&data);
            }
            Ok(_) => {} // No data available.
            Err(e) => {
                // EIO means slave closed — child probably exited.
                if e.raw_os_error() != Some(libc::EIO) {
                    eprintln!("tramp-agent: PTY read error for handle {handle}: {e}");
                }
            }
        }

        // Drain accumulated buffer.
        let output_data = std::mem::take(&mut managed.output_buf);

        // Check if the child has exited.
        if !managed.exited
            && let Some(code) = try_wait_pid(managed.pid)
        {
            managed.exited = true;
            managed.exit_code = Some(code);
        }

        let running = !managed.exited;
        let exit_code_val = match managed.exit_code {
            Some(code) => Value::Integer(code.into()),
            None => Value::Nil,
        };

        // Clean up if the child has exited.
        if !running {
            processes.remove(&handle);
        }

        Response::ok(
            id,
            Value::Map(vec![
                (Value::String("stdout".into()), Value::Binary(output_data)),
                (Value::String("stderr".into()), Value::Binary(vec![])),
                (Value::String("running".into()), Value::Boolean(running)),
                (Value::String("exit_code".into()), exit_code_val),
            ]),
        )
    }

    /// `process.write` for PTY processes — write data to the PTY master.
    ///
    /// Result: `{}` (empty map on success).
    pub async fn write(id: u64, params: &Value, table: &PtyTable) -> Response {
        let handle = match get_u64_param(params, "handle") {
            Ok(h) => h,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let data = match get_bin_param(params, "data") {
            Some(d) => d,
            None => {
                return Response::err(
                    id,
                    error_code::INVALID_PARAMS,
                    "missing or invalid binary parameter: data",
                );
            }
        };

        let processes = table.processes.lock().await;

        let Some(managed) = processes.get(&handle) else {
            return Response::err(
                id,
                error_code::NOT_FOUND,
                format!("no PTY process with handle {handle}"),
            );
        };

        if let Err(e) = managed.master.write_all(data).await {
            return Response::err(
                id,
                error_code::IO_ERROR,
                format!("failed to write to PTY: {e}"),
            );
        }

        Response::ok(id, Value::Map(vec![]))
    }

    /// `process.kill` for PTY processes.
    ///
    /// Result: `{}` (empty map on success).
    pub async fn kill(id: u64, params: &Value, table: &PtyTable) -> Response {
        let handle = match get_u64_param(params, "handle") {
            Ok(h) => h,
            Err(mut e) => {
                e.id = id;
                return e;
            }
        };

        let mut processes = table.processes.lock().await;

        let Some(managed) = processes.remove(&handle) else {
            return Response::err(
                id,
                error_code::NOT_FOUND,
                format!("no PTY process with handle {handle}"),
            );
        };

        let ret = unsafe { libc::kill(managed.pid as libc::pid_t, libc::SIGKILL) };
        if ret != 0 {
            let err = io::Error::last_os_error();
            // ESRCH means the process already exited — not an error.
            if err.raw_os_error() != Some(libc::ESRCH) {
                return Response::err(
                    id,
                    error_code::IO_ERROR,
                    format!("failed to kill PTY process: {err}"),
                );
            }
        }

        // Reap the child to avoid zombies.
        unsafe {
            let mut status: libc::c_int = 0;
            libc::waitpid(managed.pid as libc::pid_t, &mut status, 0);
        }

        Response::ok(id, Value::Map(vec![]))
    }

    /// Check if a handle belongs to the PTY process table (handles ≥ 1_000_000).
    pub fn is_pty_handle(handle: u64) -> bool {
        handle >= 1_000_000
    }
}

// ---------------------------------------------------------------------------
// Public API (re-exported from the unix module on Unix, stubs elsewhere)
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub use unix::PtyTable;

#[cfg(unix)]
pub use unix::is_pty_handle;

/// `process.start_pty` — start a process with a pseudo-terminal.
///
/// On non-Unix platforms, this always returns an error.
pub async fn start_pty(
    id: u64,
    params: &Value,
    #[cfg(unix)] table: &unix::PtyTable,
    #[cfg(not(unix))] _table: &(),
) -> Response {
    #[cfg(unix)]
    {
        unix::start_pty(id, params, table).await
    }

    #[cfg(not(unix))]
    {
        let _ = params;
        Response::err(
            id,
            error_code::INTERNAL_ERROR,
            "PTY support is not available on this platform",
        )
    }
}

/// `process.resize` — send a window size change to a PTY process.
///
/// On non-Unix platforms, this always returns an error.
pub async fn resize(
    id: u64,
    params: &Value,
    #[cfg(unix)] table: &unix::PtyTable,
    #[cfg(not(unix))] _table: &(),
) -> Response {
    #[cfg(unix)]
    {
        unix::resize(id, params, table).await
    }

    #[cfg(not(unix))]
    {
        let _ = params;
        Response::err(
            id,
            error_code::INTERNAL_ERROR,
            "PTY support is not available on this platform",
        )
    }
}

/// `process.read` for PTY handles — dispatched from the main read handler
/// when the handle is in the PTY range.
pub async fn read(
    id: u64,
    params: &Value,
    #[cfg(unix)] table: &unix::PtyTable,
    #[cfg(not(unix))] _table: &(),
) -> Response {
    #[cfg(unix)]
    {
        unix::read(id, params, table).await
    }

    #[cfg(not(unix))]
    {
        let _ = params;
        Response::err(
            id,
            error_code::INTERNAL_ERROR,
            "PTY support is not available on this platform",
        )
    }
}

/// `process.write` for PTY handles.
pub async fn write(
    id: u64,
    params: &Value,
    #[cfg(unix)] table: &unix::PtyTable,
    #[cfg(not(unix))] _table: &(),
) -> Response {
    #[cfg(unix)]
    {
        unix::write(id, params, table).await
    }

    #[cfg(not(unix))]
    {
        let _ = params;
        Response::err(
            id,
            error_code::INTERNAL_ERROR,
            "PTY support is not available on this platform",
        )
    }
}

/// `process.kill` for PTY handles.
pub async fn kill(
    id: u64,
    params: &Value,
    #[cfg(unix)] table: &unix::PtyTable,
    #[cfg(not(unix))] _table: &(),
) -> Response {
    #[cfg(unix)]
    {
        unix::kill(id, params, table).await
    }

    #[cfg(not(unix))]
    {
        let _ = params;
        Response::err(
            id,
            error_code::INTERNAL_ERROR,
            "PTY support is not available on this platform",
        )
    }
}

// ---------------------------------------------------------------------------
// Stub PTY table for non-Unix platforms
// ---------------------------------------------------------------------------

#[cfg(not(unix))]
pub struct PtyTable;

#[cfg(not(unix))]
impl PtyTable {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(unix))]
impl Default for PtyTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(unix))]
pub fn is_pty_handle(_handle: u64) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rmpv::Value;

    fn make_params(pairs: Vec<(&str, Value)>) -> Value {
        Value::Map(
            pairs
                .into_iter()
                .map(|(k, v)| (Value::String(k.into()), v))
                .collect(),
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_pty_and_read_output() {
        let table = unix::PtyTable::new();

        // Start `echo hello` in a PTY.
        let params = make_params(vec![
            ("program", Value::String("echo".into())),
            (
                "args",
                Value::Array(vec![Value::String("hello_pty".into())]),
            ),
        ]);

        let resp = start_pty(1, &params, &table).await;
        assert!(
            resp.error.is_none(),
            "start_pty should succeed: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        assert!(is_pty_handle(handle));

        // Give the process a moment to produce output and exit.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Read the output.
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let read_resp = read(2, &read_params, &table).await;
        assert!(
            read_resp.error.is_none(),
            "read should succeed: {:?}",
            read_resp.error
        );

        let read_result = read_resp.result.unwrap();
        let read_map = read_result.as_map().unwrap();

        let stdout = read_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();

        let output = String::from_utf8_lossy(stdout);
        assert!(
            output.contains("hello_pty"),
            "expected 'hello_pty' in output, got: {output:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_pty_with_custom_size() {
        let table = unix::PtyTable::new();

        let params = make_params(vec![
            ("program", Value::String("true".into())),
            ("rows", Value::Integer(50.into())),
            ("cols", Value::Integer(120.into())),
        ]);

        let resp = start_pty(1, &params, &table).await;
        assert!(
            resp.error.is_none(),
            "start_pty with custom size should succeed: {:?}",
            resp.error
        );

        // Wait for process to exit and clean up.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        // Read to clean up the process from the table.
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let _ = read(2, &read_params, &table).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resize_pty() {
        let table = unix::PtyTable::new();

        // Start a long-running process.
        let params = make_params(vec![
            ("program", Value::String("sleep".into())),
            ("args", Value::Array(vec![Value::String("10".into())])),
        ]);

        let resp = start_pty(1, &params, &table).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        // Resize the PTY.
        let resize_params = make_params(vec![
            ("handle", Value::Integer(handle.into())),
            ("rows", Value::Integer(40.into())),
            ("cols", Value::Integer(100.into())),
        ]);

        let resize_resp = resize(2, &resize_params, &table).await;
        assert!(
            resize_resp.error.is_none(),
            "resize should succeed: {:?}",
            resize_resp.error
        );

        // Kill the process.
        let kill_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let kill_resp = kill(3, &kill_params, &table).await;
        assert!(
            kill_resp.error.is_none(),
            "kill should succeed: {:?}",
            kill_resp.error
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_to_pty() {
        let table = unix::PtyTable::new();

        // Start cat (reads from stdin, writes to stdout).
        let params = make_params(vec![("program", Value::String("cat".into()))]);

        let resp = start_pty(1, &params, &table).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        // Write data to the PTY.
        let write_params = make_params(vec![
            ("handle", Value::Integer(handle.into())),
            ("data", Value::Binary(b"test_input\n".to_vec())),
        ]);

        let write_resp = write(2, &write_params, &table).await;
        assert!(
            write_resp.error.is_none(),
            "write should succeed: {:?}",
            write_resp.error
        );

        // Give cat time to echo back.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Read back the echoed output.
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let read_resp = read(3, &read_params, &table).await;
        assert!(read_resp.error.is_none());

        let read_result = read_resp.result.unwrap();
        let read_map = read_result.as_map().unwrap();
        let stdout = read_map
            .iter()
            .find(|(k, _)| k.as_str() == Some("stdout"))
            .unwrap()
            .1
            .as_slice()
            .unwrap();

        let output = String::from_utf8_lossy(stdout);
        assert!(
            output.contains("test_input"),
            "expected 'test_input' in PTY output, got: {output:?}"
        );

        // Kill the process.
        let kill_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let _ = kill(4, &kill_params, &table).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_pty_process() {
        let table = unix::PtyTable::new();

        let params = make_params(vec![
            ("program", Value::String("sleep".into())),
            ("args", Value::Array(vec![Value::String("60".into())])),
        ]);

        let resp = start_pty(1, &params, &table).await;
        assert!(resp.error.is_none());

        let result = resp.result.unwrap();
        let map = result.as_map().unwrap();
        let handle = map
            .iter()
            .find(|(k, _)| k.as_str() == Some("handle"))
            .unwrap()
            .1
            .as_u64()
            .unwrap();

        let kill_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let kill_resp = kill(2, &kill_params, &table).await;
        assert!(
            kill_resp.error.is_none(),
            "kill should succeed: {:?}",
            kill_resp.error
        );

        // Trying to read after kill should fail (handle removed).
        let read_params = make_params(vec![("handle", Value::Integer(handle.into()))]);
        let read_resp = read(3, &read_params, &table).await;
        assert!(read_resp.error.is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resize_nonexistent_handle() {
        let table = unix::PtyTable::new();

        let params = make_params(vec![
            ("handle", Value::Integer(9999999u64.into())),
            ("rows", Value::Integer(24.into())),
            ("cols", Value::Integer(80.into())),
        ]);

        let resp = resize(1, &params, &table).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, crate::rpc::error_code::NOT_FOUND);
    }

    #[cfg(unix)]
    #[test]
    fn pty_handle_detection() {
        assert!(!is_pty_handle(0));
        assert!(!is_pty_handle(999_999));
        assert!(is_pty_handle(1_000_000));
        assert!(is_pty_handle(1_000_001));
    }
}
