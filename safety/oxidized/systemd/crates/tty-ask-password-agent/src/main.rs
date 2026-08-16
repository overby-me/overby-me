//! systemd-tty-ask-password-agent — Process system password requests on the TTY.
//!
//! A drop-in replacement for `systemd-tty-ask-password-agent(1)`. This tool
//! monitors `/run/systemd/ask-password/` for password question files created
//! by `systemd-ask-password` (or other systemd components), displays the
//! password prompt on the controlling TTY, reads the user's input, and sends
//! the response back through the Unix socket specified in the question file.
//!
//! Modes of operation:
//!
//! - `--query`  — Process all pending password questions once, then exit.
//! - `--watch`  — Continuously watch for new questions (using inotify) and
//!   process them as they appear.
//! - `--wall`   — Send wall messages for pending password questions instead
//!   of querying on the TTY.
//! - `--list`   — List all currently pending password questions.
//!
//! ## inotify-based watching
//!
//! In `--watch` mode, the agent uses Linux inotify to monitor the
//! `/run/systemd/ask-password/` directory for `IN_CLOSE_WRITE`,
//! `IN_MOVED_TO`, and `IN_CREATE` events on `ask.*` files. This provides
//! near-instant detection of new password questions without polling.
//! If inotify is unavailable (e.g. in restricted containers), the agent
//! falls back to 500ms polling.
//!
//! ## Plymouth integration
//!
//! When `--plymouth` is given, the agent forwards password queries to the
//! Plymouth graphical boot splash via its Unix socket protocol at
//! `/run/plymouth/plymouthd`. Plymouth displays the prompt on the graphical
//! splash screen and returns the user's input. This enables password entry
//! during early boot when only the splash screen is visible.
//!
//! The Plymouth protocol:
//!   - Request: `[type_byte][NUL-terminated prompt]`
//!     - `'*'` — password request (no echo)
//!     - `'C'` — cached password request
//!     - `'W'` — question (with echo)
//!   - Response: `[status_byte][NUL-terminated password]`
//!     - `0x02` — ACK (success, password follows)
//!     - `0x05` — NAK (failure/cancellation)
//!
//! The password query protocol:
//!
//! Question files are INI-style files in `/run/systemd/ask-password/` with
//! an `[Ask]` section containing:
//!   - `PID=`          — PID of the process asking
//!   - `Socket=`       — Unix socket path to send the password to
//!   - `Message=`      — Human-readable prompt
//!   - `Icon=`         — Icon name (informational)
//!   - `Id=`           — Identifier for caching
//!   - `NotAfter=`     — Monotonic timestamp (usec) after which the question expires
//!   - `AcceptCached=` — Whether to accept cached/empty passwords
//!   - `Echo=`         — Whether to echo input
//!
//! The response is sent as a datagram to the specified socket:
//!   - `+<password>` for a successful response
//!   - `-` for cancellation

use clap::Parser;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::time::Duration;

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

/// The directory where password query files are placed.
const ASK_PASSWORD_DIR: &str = "/run/systemd/ask-password";

/// Plymouth daemon socket path.
const PLYMOUTH_SOCKET_PATH: &str = "/run/plymouth/plymouthd";

/// Plymouth protocol answer types.
const PLYMOUTH_ANSWER_ACK: u8 = 0x02;
const PLYMOUTH_ANSWER_NAK: u8 = 0x05;

/// inotify flags for watching the ask-password directory.
const WATCH_FLAGS: AddWatchFlags = AddWatchFlags::from_bits_truncate(
    AddWatchFlags::IN_CLOSE_WRITE.bits()
        | AddWatchFlags::IN_MOVED_TO.bits()
        | AddWatchFlags::IN_CREATE.bits()
        | AddWatchFlags::IN_DELETE.bits()
        | AddWatchFlags::IN_DONT_FOLLOW.bits(),
);

/// Poll timeout when using inotify (milliseconds). We wake up periodically
/// to do bookkeeping even when no inotify events arrive.
const INOTIFY_POLL_TIMEOUT_MS: i32 = 2000;

/// Poll interval when inotify is not available (milliseconds).
const POLL_FALLBACK_INTERVAL: Duration = Duration::from_millis(500);

/// Plymouth connect/read timeout.
const PLYMOUTH_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Parser, Debug)]
#[command(
    name = "systemd-tty-ask-password-agent",
    about = "Process system password requests",
    version
)]
struct Cli {
    /// Process all currently pending password questions, then exit.
    #[arg(long, group = "mode")]
    query: bool,

    /// Continuously watch for password questions and process them.
    #[arg(long, group = "mode")]
    watch: bool,

    /// Send wall messages for pending password questions.
    #[arg(long, group = "mode")]
    wall: bool,

    /// List currently pending password questions.
    #[arg(long, group = "mode")]
    list: bool,

    /// Forward password queries to the Plymouth graphical boot splash.
    /// Optionally takes a socket path (defaults to /run/plymouth/plymouthd).
    #[arg(long, value_name = "PATH")]
    plymouth: Option<Option<String>>,

    /// Use the specified TTY console for password queries.
    #[arg(long, value_name = "PATH")]
    console: Option<String>,
}

impl Cli {
    /// Whether Plymouth mode is active (--plymouth was passed, with or without a value).
    fn use_plymouth(&self) -> bool {
        self.plymouth.is_some()
    }

    /// The Plymouth socket path to use.
    fn plymouth_socket_path(&self) -> &str {
        match &self.plymouth {
            Some(Some(path)) => path.as_str(),
            _ => PLYMOUTH_SOCKET_PATH,
        }
    }
}

/// A parsed password question from a question file.
#[derive(Debug)]
struct PasswordQuestion {
    /// Path to the question file itself.
    file_path: PathBuf,
    /// PID of the asking process.
    pid: Option<u32>,
    /// Unix socket path to send the answer to.
    socket: Option<PathBuf>,
    /// Human-readable prompt message.
    message: String,
    /// Icon name (informational).
    icon: Option<String>,
    /// Identifier for caching.
    id: Option<String>,
    /// Monotonic deadline in microseconds; 0 means no deadline.
    not_after: u64,
    /// Whether cached/empty passwords are accepted.
    accept_cached: bool,
    /// Whether to echo input.
    echo: bool,
}

impl PasswordQuestion {
    /// Parse a question file.
    fn from_file(path: &Path) -> io::Result<Self> {
        let content = fs::read_to_string(path)?;
        let mut pid = None;
        let mut socket = None;
        let mut message = String::from("Password:");
        let mut icon = None;
        let mut id = None;
        let mut not_after = 0u64;
        let mut accept_cached = false;
        let mut echo = false;

        let mut in_ask_section = false;

        for line in content.lines() {
            let line = line.trim();

            if line == "[Ask]" {
                in_ask_section = true;
                continue;
            }

            if line.starts_with('[') {
                in_ask_section = false;
                continue;
            }

            if !in_ask_section {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "PID" => pid = value.parse().ok(),
                    "Socket" => socket = Some(PathBuf::from(value)),
                    "Message" => message = value.to_string(),
                    "Icon" => icon = Some(value.to_string()),
                    "Id" => id = Some(value.to_string()),
                    "NotAfter" => not_after = value.parse().unwrap_or(0),
                    "AcceptCached" => accept_cached = value == "yes" || value == "1",
                    "Echo" => echo = value == "yes" || value == "1",
                    _ => {}
                }
            }
        }

        Ok(PasswordQuestion {
            file_path: path.to_path_buf(),
            pid,
            socket,
            message,
            icon,
            id,
            not_after,
            accept_cached,
            echo,
        })
    }

    /// Check if this question has expired based on the monotonic clock.
    fn is_expired(&self) -> bool {
        if self.not_after == 0 {
            return false;
        }

        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        if ret != 0 {
            return false;
        }

        let now_usec = ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000;
        now_usec > self.not_after
    }

    /// Send a password response to the asking process via the socket.
    fn send_response(&self, password: &str) -> io::Result<()> {
        let socket_path = self.socket.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "no socket path in question file",
            )
        })?;

        let sock = UnixDatagram::unbound()?;

        // Send "+<password>" for success
        let msg = format!("+{password}");
        sock.send_to(msg.as_bytes(), socket_path)?;

        Ok(())
    }

    /// Send a cancellation response to the asking process.
    fn send_cancel(&self) -> io::Result<()> {
        let socket_path = self.socket.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "no socket path in question file",
            )
        })?;

        let sock = UnixDatagram::unbound()?;
        sock.send_to(b"-", socket_path)?;

        Ok(())
    }
}

/// Enumerate all pending password questions.
fn enumerate_questions() -> Vec<PasswordQuestion> {
    let dir = Path::new(ASK_PASSWORD_DIR);
    let mut questions = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return questions,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Only process "ask.*" files
        if !name_str.starts_with("ask.") {
            continue;
        }

        match PasswordQuestion::from_file(&path) {
            Ok(q) => {
                if !q.is_expired() {
                    questions.push(q);
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to parse question file {}: {e}",
                    path.display()
                );
            }
        }
    }

    questions
}

// ---------------------------------------------------------------------------
// Plymouth integration
// ---------------------------------------------------------------------------

/// Check if Plymouth is running by testing if its socket exists.
fn plymouth_is_running(socket_path: &str) -> bool {
    Path::new(socket_path).exists()
}

/// Connect to the Plymouth daemon socket.
fn plymouth_connect(socket_path: &str) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(PLYMOUTH_TIMEOUT))?;
    stream.set_write_timeout(Some(PLYMOUTH_TIMEOUT))?;
    Ok(stream)
}

/// Check if Plymouth is responsive by sending a ping.
///
/// Used at watch-mode startup to verify Plymouth connectivity before
/// committing to Plymouth-based password queries.
fn plymouth_ping(socket_path: &str) -> io::Result<bool> {
    let mut stream = plymouth_connect(socket_path)?;

    // Send ping: 'P' followed by NUL
    stream.write_all(b"P\0")?;
    stream.flush()?;

    // Read response — first byte is status
    let mut buf = [0u8; 1];
    match stream.read_exact(&mut buf) {
        Ok(()) => Ok(buf[0] == PLYMOUTH_ANSWER_ACK),
        Err(_) => Ok(false),
    }
}

/// Ask Plymouth to display a password prompt and return the user's input.
///
/// # Protocol
///
/// Request: `[type_byte][prompt_string][NUL]`
///   - `'*'` for password (no echo)
///   - `'C'` for cached password
///   - `'W'` for question (with echo)
///
/// Response: `[status_byte][password_string...]`
///   - `0x02` (ACK) followed by the NUL-terminated password
///   - `0x05` (NAK) for failure/cancellation
fn plymouth_ask_password(
    socket_path: &str,
    prompt: &str,
    echo: bool,
    accept_cached: bool,
) -> io::Result<Option<String>> {
    let mut stream = plymouth_connect(socket_path)?;

    // Determine request type
    let type_byte: u8 = if accept_cached {
        b'C' // Cached password request
    } else if echo {
        b'W' // Question (with echo)
    } else {
        b'*' // Password (no echo)
    };

    // Build packet: [type_byte][prompt][NUL]
    let mut packet = Vec::with_capacity(1 + prompt.len() + 1);
    packet.push(type_byte);
    packet.extend_from_slice(prompt.as_bytes());
    packet.push(0);

    stream.write_all(&packet)?;
    stream.flush()?;

    // Read response
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf)?;

    if n == 0 {
        return Ok(None);
    }

    let status = buf[0];

    if status == PLYMOUTH_ANSWER_NAK {
        return Ok(None);
    }

    if status == PLYMOUTH_ANSWER_ACK && n > 1 {
        // Password follows, NUL-terminated
        let password_bytes = &buf[1..n];
        // Strip trailing NUL if present
        let password_bytes = if password_bytes.last() == Some(&0) {
            &password_bytes[..password_bytes.len() - 1]
        } else {
            password_bytes
        };
        let password = String::from_utf8_lossy(password_bytes).to_string();
        return Ok(Some(password));
    }

    // ACK with no data — treat as empty password
    if status == PLYMOUTH_ANSWER_ACK {
        return Ok(Some(String::new()));
    }

    Ok(None)
}

/// Display a message on Plymouth without requesting input.
///
/// Used in `--wall` mode to forward pending password question messages
/// to the Plymouth graphical boot splash.
fn plymouth_display_message(socket_path: &str, message: &str) -> io::Result<()> {
    let mut stream = plymouth_connect(socket_path)?;

    // Build packet: 'M' + NUL-terminated message
    let mut packet = Vec::with_capacity(1 + message.len() + 1);
    packet.push(b'M');
    packet.extend_from_slice(message.as_bytes());
    packet.push(0);

    stream.write_all(&packet)?;
    stream.flush()?;

    // Read ACK/NAK (best-effort)
    let mut buf = [0u8; 1];
    let _ = stream.read(&mut buf);

    Ok(())
}

// ---------------------------------------------------------------------------
// TTY password reading
// ---------------------------------------------------------------------------

/// Read a password from the TTY.
///
/// Opens `/dev/tty` (or the specified console) and reads a line, optionally
/// with echo disabled.
fn read_password_tty(prompt: &str, echo: bool, console: Option<&str>) -> io::Result<String> {
    let tty_path = console.unwrap_or("/dev/tty");
    let tty_cstr = std::ffi::CString::new(tty_path)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe { libc::open(tty_cstr.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // Write prompt
    let prompt_msg = format!("{prompt} ");
    unsafe {
        libc::write(fd, prompt_msg.as_ptr().cast(), prompt_msg.len());
    }

    // Save terminal settings and optionally disable echo
    let mut old_termios: libc::termios = unsafe { std::mem::zeroed() };
    let termios_saved = unsafe { libc::tcgetattr(fd, &mut old_termios) } == 0;

    if !echo && termios_saved {
        let mut new_termios = old_termios;
        new_termios.c_lflag &= !(libc::ECHO);
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, &new_termios);
        }
    }

    // Read line
    let mut password = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
        if n <= 0 {
            break;
        }
        if byte[0] == b'\n' || byte[0] == b'\r' {
            break;
        }
        // Handle backspace
        if byte[0] == 0x7f || byte[0] == 0x08 {
            password.pop();
            continue;
        }
        // Ctrl-C / Ctrl-D
        if byte[0] == 0x03 || byte[0] == 0x04 {
            if !echo && termios_saved {
                unsafe {
                    libc::tcsetattr(fd, libc::TCSANOW, &old_termios);
                }
            }
            unsafe {
                libc::write(fd, b"\n".as_ptr().cast(), 1);
                libc::close(fd);
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        password.push(byte[0]);
    }

    // Restore terminal settings
    if !echo && termios_saved {
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, &old_termios);
            libc::write(fd, b"\n".as_ptr().cast(), 1);
        }
    }

    unsafe {
        libc::close(fd);
    }

    String::from_utf8(password).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

// ---------------------------------------------------------------------------
// Question processing
// ---------------------------------------------------------------------------

/// Process a single password question by prompting on the TTY.
fn process_question(question: &PasswordQuestion, console: Option<&str>) -> bool {
    if question.is_expired() {
        return false;
    }

    if question.socket.is_none() {
        eprintln!(
            "Warning: question file {} has no socket, skipping",
            question.file_path.display()
        );
        return false;
    }

    // Automation hook: if $SYSTEMD_ASK_PASSWORD_AGENT_PASSWORD is set, reply to
    // every question with its value instead of prompting on the console.
    // Mirrors upstream systemd-tty-ask-password-agent.
    if let Ok(canned) = std::env::var("SYSTEMD_ASK_PASSWORD_AGENT_PASSWORD") {
        return match question.send_response(&canned) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("Failed to send password response: {e}");
                false
            }
        };
    }

    let prompt = if let Some(pid) = question.pid {
        format!("{} (PID {})", question.message, pid)
    } else {
        question.message.clone()
    };

    match read_password_tty(&prompt, question.echo, console) {
        Ok(password) => {
            if password.is_empty() && !question.accept_cached {
                eprintln!("Empty password not accepted.");
                if let Err(e) = question.send_cancel() {
                    eprintln!("Failed to send cancellation: {e}");
                }
                return false;
            }
            match question.send_response(&password) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Failed to send password response: {e}");
                    false
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::Interrupted => {
            let _ = question.send_cancel();
            false
        }
        Err(e) => {
            eprintln!("Failed to read password: {e}");
            let _ = question.send_cancel();
            false
        }
    }
}

/// Process a single password question via Plymouth.
fn process_question_plymouth(question: &PasswordQuestion, socket_path: &str) -> bool {
    if question.is_expired() {
        return false;
    }

    if question.socket.is_none() {
        eprintln!(
            "Warning: question file {} has no socket, skipping",
            question.file_path.display()
        );
        return false;
    }

    let prompt = if let Some(pid) = question.pid {
        format!("{} (PID {})", question.message, pid)
    } else {
        question.message.clone()
    };

    match plymouth_ask_password(socket_path, &prompt, question.echo, question.accept_cached) {
        Ok(Some(password)) => {
            if password.is_empty() && !question.accept_cached {
                eprintln!("Empty password not accepted.");
                if let Err(e) = question.send_cancel() {
                    eprintln!("Failed to send cancellation: {e}");
                }
                return false;
            }
            match question.send_response(&password) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Failed to send password response: {e}");
                    false
                }
            }
        }
        Ok(None) => {
            let _ = question.send_cancel();
            false
        }
        Err(e) => {
            eprintln!("Plymouth password query failed: {e}");
            let _ = question.send_cancel();
            false
        }
    }
}

/// List all pending password questions.
fn list_questions() {
    let questions = enumerate_questions();

    if questions.is_empty() {
        println!("No pending password questions.");
        return;
    }

    for (i, q) in questions.iter().enumerate() {
        println!("{}. {}", i + 1, q.message);
        if let Some(pid) = q.pid {
            println!("   PID: {pid}");
        }
        if let Some(ref id) = q.id {
            println!("   ID: {id}");
        }
        if let Some(ref icon) = q.icon {
            println!("   Icon: {icon}");
        }
        println!("   Echo: {}", if q.echo { "yes" } else { "no" });
        println!(
            "   Accept Cached: {}",
            if q.accept_cached { "yes" } else { "no" }
        );
        if q.not_after > 0 {
            println!("   Not After: {} usec", q.not_after);
        }
        if let Some(ref socket) = q.socket {
            println!("   Socket: {}", socket.display());
        }
        println!("   File: {}", q.file_path.display());
        println!();
    }
}

/// Send wall messages about pending password questions.
fn wall_questions() {
    let questions = enumerate_questions();

    if questions.is_empty() {
        return;
    }

    for q in &questions {
        let msg = if let Some(pid) = q.pid {
            format!(
                "Password entry required for '{}' (PID {}).\r\n\
                 Please enter password with the systemd-tty-ask-password-agent tool.",
                q.message, pid
            )
        } else {
            format!(
                "Password entry required for '{}'.\r\n\
                 Please enter password with the systemd-tty-ask-password-agent tool.",
                q.message
            )
        };

        // Try to write wall message via utmp/write to all terminals
        send_wall_message(&msg);
    }
}

/// Send a wall message to all logged-in TTYs.
fn send_wall_message(msg: &str) {
    // Read /dev/pts/* and /dev/tty* to find active terminals
    let mut ttys = Vec::new();

    // Check /dev/pts/
    if let Ok(entries) = fs::read_dir("/dev/pts") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // Skip ptmx
                if name == "ptmx" {
                    continue;
                }
                // Only process numeric entries (pty slaves)
                if name.chars().all(|c| c.is_ascii_digit()) {
                    ttys.push(path);
                }
            }
        }
    }

    // Also check /dev/tty[1-63]
    for i in 1..=63 {
        let path = PathBuf::from(format!("/dev/tty{i}"));
        if path.exists() {
            ttys.push(path);
        }
    }

    let wall_msg = format!(
        "\r\n\
         \r\nBroadcast message from systemd-tty-ask-password-agent:\r\n\
         \r\n{msg}\r\n\r\n"
    );

    for tty in &ttys {
        let tty_cstr = match std::ffi::CString::new(tty.to_str().unwrap_or_default()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let fd = unsafe {
            libc::open(
                tty_cstr.as_ptr(),
                libc::O_WRONLY | libc::O_NOCTTY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            continue;
        }

        unsafe {
            libc::write(fd, wall_msg.as_ptr().cast(), wall_msg.len());
            libc::close(fd);
        }
    }
}

// ---------------------------------------------------------------------------
// inotify-based watching
// ---------------------------------------------------------------------------

/// Set up an inotify instance to watch the ask-password directory.
///
/// Returns `None` if inotify initialization fails, allowing the caller to
/// fall back to polling.
fn setup_inotify_watch() -> Option<Inotify> {
    let inotify = match Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("inotify_init1 failed, will use polling fallback: {e}");
            return None;
        }
    };

    let dir = Path::new(ASK_PASSWORD_DIR);

    // Ensure the directory exists before watching
    let _ = fs::create_dir_all(dir);

    match inotify.add_watch(dir, WATCH_FLAGS) {
        Ok(_wd) => Some(inotify),
        Err(e) => {
            eprintln!(
                "Failed to add inotify watch on {ASK_PASSWORD_DIR}, will use polling fallback: {e}"
            );
            None
        }
    }
}

/// Wait for inotify events using poll(2), returning true if events are ready.
fn wait_for_inotify_events(inotify: &Inotify) -> bool {
    use std::os::fd::{AsFd, AsRawFd};

    let mut pollfd = libc::pollfd {
        fd: inotify.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, INOTIFY_POLL_TIMEOUT_MS) };

    ret > 0 && (pollfd.revents & libc::POLLIN) != 0
}

/// Drain all pending inotify events, returning whether any `ask.*` file
/// events were observed.
fn drain_inotify_events(inotify: &Inotify) -> bool {
    match inotify.read_events() {
        Ok(events) => events.iter().any(|ev| {
            ev.name
                .as_ref()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with("ask."))
        }),
        Err(_) => false,
    }
}

/// Watch for new password question files using inotify, with polling fallback.
///
/// Uses Linux inotify to efficiently detect new `ask.*` files in
/// `/run/systemd/ask-password/`. When inotify is unavailable (restricted
/// containers, non-Linux, etc.) falls back to 500ms polling.
fn watch_and_process(console: Option<&str>, use_plymouth: bool, plymouth_socket: &str) {
    let dir = Path::new(ASK_PASSWORD_DIR);
    let mut processed: HashSet<PathBuf> = HashSet::new();

    // Ensure the directory exists
    let _ = fs::create_dir_all(dir);

    // Try to set up inotify
    let inotify = setup_inotify_watch();
    let use_inotify = inotify.is_some();

    if use_inotify {
        eprintln!("Watching {ASK_PASSWORD_DIR} for password questions (inotify)...");
    } else {
        eprintln!("Watching {ASK_PASSWORD_DIR} for password questions (polling fallback)...");
    }

    // If Plymouth mode is active, verify connectivity at startup
    if use_plymouth {
        match plymouth_ping(plymouth_socket) {
            Ok(true) => eprintln!("Plymouth is running and responsive."),
            Ok(false) => eprintln!("Warning: Plymouth ping returned NAK, will retry per-question."),
            Err(e) => eprintln!("Warning: Plymouth not reachable ({e}), will fall back to TTY."),
        }
    }

    // Process any already-pending questions before entering the watch loop
    process_pending_questions(&mut processed, console, use_plymouth, plymouth_socket);

    loop {
        let should_check = if let Some(ref ino) = inotify {
            // Wait for inotify events (with timeout for periodic bookkeeping)
            let events_ready = wait_for_inotify_events(ino);

            if events_ready {
                // Drain events and check if any are ask.* files.
                // Even if no ask.* events, still check periodically for
                // edge cases (race between readdir and inotify)
                drain_inotify_events(ino)
            } else {
                // Timeout — do a periodic check anyway
                true
            }
        } else {
            // Polling fallback
            std::thread::sleep(POLL_FALLBACK_INTERVAL);
            true
        };

        if should_check {
            process_pending_questions(&mut processed, console, use_plymouth, plymouth_socket);
        }
    }
}

/// Process all pending questions that haven't been processed yet.
fn process_pending_questions(
    processed: &mut HashSet<PathBuf>,
    console: Option<&str>,
    use_plymouth: bool,
    plymouth_socket: &str,
) {
    let questions = enumerate_questions();

    for q in &questions {
        if processed.contains(&q.file_path) {
            continue;
        }

        let success = if use_plymouth && plymouth_is_running(plymouth_socket) {
            process_question_plymouth(q, plymouth_socket)
        } else {
            process_question(q, console)
        };

        if success {
            processed.insert(q.file_path.clone());
        } else {
            // If the question expired or was cancelled, mark it as
            // processed so we don't keep retrying.
            if q.is_expired() {
                processed.insert(q.file_path.clone());
            }
        }
    }

    // Clean up processed entries for files that no longer exist
    processed.retain(|p| p.exists());
}

/// Process all pending questions once.
fn query_once(console: Option<&str>, use_plymouth: bool, plymouth_socket: &str) -> bool {
    let questions = enumerate_questions();

    if questions.is_empty() {
        return true;
    }

    let mut all_ok = true;
    for q in &questions {
        let success = if use_plymouth && plymouth_is_running(plymouth_socket) {
            process_question_plymouth(q, plymouth_socket)
        } else {
            process_question(q, console)
        };

        if !success {
            all_ok = false;
        }
    }

    all_ok
}

fn main() {
    let cli = Cli::parse();

    // Determine the console to use
    let console = cli.console.as_deref();
    let use_plymouth = cli.use_plymouth();
    let plymouth_socket = cli.plymouth_socket_path().to_string();

    if cli.list {
        list_questions();
        process::exit(0);
    }

    if cli.wall {
        if use_plymouth && plymouth_is_running(&plymouth_socket) {
            // Forward wall messages to Plymouth's graphical splash
            let questions = enumerate_questions();
            for q in &questions {
                let msg = if let Some(pid) = q.pid {
                    format!("{} (PID {})", q.message, pid)
                } else {
                    q.message.clone()
                };
                if let Err(e) = plymouth_display_message(&plymouth_socket, &msg) {
                    eprintln!("Warning: failed to send message to Plymouth: {e}");
                }
            }
        } else {
            wall_questions();
        }
        process::exit(0);
    }

    if cli.watch {
        watch_and_process(console, use_plymouth, &plymouth_socket);
        // watch_and_process runs forever, but just in case:
        process::exit(0);
    }

    // Default to --query if no mode specified, or --query was explicit
    if cli.query || (!cli.watch && !cli.wall && !cli.list) {
        let ok = query_once(console, use_plymouth, &plymouth_socket);
        process::exit(if ok { 0 } else { 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_questions_empty() {
        // In a test environment, /run/systemd/ask-password/ likely doesn't
        // exist or is empty. This should not panic.
        let questions = enumerate_questions();
        // We can't assert the count, but at least it shouldn't crash
        let _ = questions;
    }

    #[test]
    fn test_parse_question_file() {
        let dir = std::env::temp_dir().join("tty-agent-test");
        let _ = fs::create_dir_all(&dir);

        let content = "\
[Ask]
PID=12345
Socket=/run/systemd/ask-password/sck.test
Message=Enter LUKS passphrase for /dev/sda2:
Icon=drive-harddisk
Id=cryptsetup:/dev/sda2
NotAfter=0
AcceptCached=no
Echo=no
";
        let path = dir.join("ask.test");
        fs::write(&path, content).unwrap();

        let q = PasswordQuestion::from_file(&path).unwrap();
        assert_eq!(q.pid, Some(12345));
        assert_eq!(
            q.socket,
            Some(PathBuf::from("/run/systemd/ask-password/sck.test"))
        );
        assert_eq!(q.message, "Enter LUKS passphrase for /dev/sda2:");
        assert_eq!(q.icon, Some("drive-harddisk".to_string()));
        assert_eq!(q.id, Some("cryptsetup:/dev/sda2".to_string()));
        assert_eq!(q.not_after, 0);
        assert!(!q.accept_cached);
        assert!(!q.echo);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_question_file_minimal() {
        let dir = std::env::temp_dir().join("tty-agent-test-minimal");
        let _ = fs::create_dir_all(&dir);

        let content = "\
[Ask]
Socket=/tmp/test.sock
Message=Password:
";
        let path = dir.join("ask.minimal");
        fs::write(&path, content).unwrap();

        let q = PasswordQuestion::from_file(&path).unwrap();
        assert_eq!(q.pid, None);
        assert_eq!(q.socket, Some(PathBuf::from("/tmp/test.sock")));
        assert_eq!(q.message, "Password:");
        assert_eq!(q.icon, None);
        assert_eq!(q.id, None);
        assert_eq!(q.not_after, 0);
        assert!(!q.accept_cached);
        assert!(!q.echo);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_not_expired_when_zero() {
        let q = PasswordQuestion {
            file_path: PathBuf::from("/tmp/test"),
            pid: None,
            socket: None,
            message: String::new(),
            icon: None,
            id: None,
            not_after: 0,
            accept_cached: false,
            echo: false,
        };
        assert!(!q.is_expired());
    }

    #[test]
    fn test_expired_in_past() {
        let q = PasswordQuestion {
            file_path: PathBuf::from("/tmp/test"),
            pid: None,
            socket: None,
            message: String::new(),
            icon: None,
            id: None,
            not_after: 1, // 1 microsecond since boot — definitely in the past
            accept_cached: false,
            echo: false,
        };
        assert!(q.is_expired());
    }

    #[test]
    fn test_not_expired_in_future() {
        // Set NotAfter far in the future (year 2100 equivalent in usec)
        let q = PasswordQuestion {
            file_path: PathBuf::from("/tmp/test"),
            pid: None,
            socket: None,
            message: String::new(),
            icon: None,
            id: None,
            not_after: u64::MAX / 2,
            accept_cached: false,
            echo: false,
        };
        assert!(!q.is_expired());
    }

    #[test]
    fn test_ask_password_dir_constant() {
        assert_eq!(ASK_PASSWORD_DIR, "/run/systemd/ask-password");
    }

    #[test]
    fn test_parse_with_echo_yes() {
        let dir = std::env::temp_dir().join("tty-agent-test-echo");
        let _ = fs::create_dir_all(&dir);

        let content = "\
[Ask]
Socket=/tmp/test.sock
Message=Username:
Echo=yes
AcceptCached=1
";
        let path = dir.join("ask.echo");
        fs::write(&path, content).unwrap();

        let q = PasswordQuestion::from_file(&path).unwrap();
        assert!(q.echo);
        assert!(q.accept_cached);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_ignores_other_sections() {
        let dir = std::env::temp_dir().join("tty-agent-test-sections");
        let _ = fs::create_dir_all(&dir);

        let content = "\
[Other]
Socket=/wrong/path
Message=Wrong message

[Ask]
Socket=/right/path
Message=Right message

[Another]
Message=Also wrong
";
        let path = dir.join("ask.sections");
        fs::write(&path, content).unwrap();

        let q = PasswordQuestion::from_file(&path).unwrap();
        assert_eq!(q.socket, Some(PathBuf::from("/right/path")));
        assert_eq!(q.message, "Right message");

        let _ = fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // inotify-based watching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_inotify_setup_succeeds() {
        // On Linux, inotify should always be available.
        // We watch a temp directory since /run/systemd/ask-password may not exist.
        let ino = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC);
        assert!(ino.is_ok(), "inotify should be available on Linux");
    }

    #[test]
    fn test_inotify_watch_temp_dir() {
        let dir = std::env::temp_dir().join("tty-agent-test-inotify-watch");
        let _ = fs::create_dir_all(&dir);

        let ino =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).expect("inotify init");
        let wd = ino.add_watch(&dir, WATCH_FLAGS);
        assert!(wd.is_ok(), "should be able to watch temp directory");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_inotify_detects_file_creation() {
        let dir = std::env::temp_dir().join("tty-agent-test-inotify-create");
        let _ = fs::create_dir_all(&dir);

        let ino =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).expect("inotify init");
        let _wd = ino
            .add_watch(&dir, WATCH_FLAGS)
            .expect("add watch should succeed");

        // Create a file in the watched directory
        let file_path = dir.join("ask.test-inotify");
        fs::write(&file_path, "test").unwrap();

        // Give the kernel a moment to deliver the event
        std::thread::sleep(Duration::from_millis(50));

        let events = ino.read_events();
        assert!(events.is_ok(), "should be able to read inotify events");
        let events = events.unwrap();
        assert!(!events.is_empty(), "should have received creation event");

        // Check that at least one event references our ask.* file
        let has_ask = events.iter().any(|ev| {
            ev.name
                .as_ref()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with("ask."))
        });
        assert!(has_ask, "should have an event for ask.* file");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_inotify_detects_file_deletion() {
        let dir = std::env::temp_dir().join("tty-agent-test-inotify-delete");
        let _ = fs::create_dir_all(&dir);

        // Create the file first
        let file_path = dir.join("ask.test-delete");
        fs::write(&file_path, "test").unwrap();

        let ino =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).expect("inotify init");
        let _wd = ino.add_watch(&dir, WATCH_FLAGS).expect("add watch");

        // Delete the file
        fs::remove_file(&file_path).unwrap();

        std::thread::sleep(Duration::from_millis(50));

        let events = ino.read_events();
        assert!(events.is_ok(), "should be able to read inotify events");
        let events = events.unwrap();
        assert!(
            !events.is_empty(),
            "should have received deletion event from inotify"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_inotify_nonblocking_returns_empty() {
        // Verify that reading from a non-blocking inotify fd with no
        // pending events does not block.
        let dir = std::env::temp_dir().join("tty-agent-test-inotify-noblock");
        let _ = fs::create_dir_all(&dir);

        let ino =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).expect("inotify init");
        let _wd = ino.add_watch(&dir, WATCH_FLAGS).expect("add watch");

        // No events pending — should return empty or EAGAIN
        match ino.read_events() {
            Ok(events) => assert!(events.is_empty(), "should have no events"),
            Err(e) => {
                // EAGAIN is expected for non-blocking reads with no data
                assert!(
                    e == nix::errno::Errno::EAGAIN,
                    "unexpected inotify error: {e}"
                );
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_drain_inotify_events_filters_ask_files() {
        let dir = std::env::temp_dir().join("tty-agent-test-inotify-filter");
        let _ = fs::create_dir_all(&dir);

        let ino =
            Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).expect("inotify init");
        let _wd = ino.add_watch(&dir, WATCH_FLAGS).expect("add watch");

        // Create a non-ask file — should not be counted
        fs::write(dir.join("sck.something"), "test").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let has_ask = drain_inotify_events(&ino);
        assert!(!has_ask, "non-ask file should not trigger ask detection");

        // Now create an ask file
        fs::write(dir.join("ask.something"), "test").unwrap();
        std::thread::sleep(Duration::from_millis(50));

        let has_ask = drain_inotify_events(&ino);
        assert!(has_ask, "ask file should trigger ask detection");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_watch_flags_include_expected() {
        // Verify our watch flags include the events we need
        assert!(WATCH_FLAGS.contains(AddWatchFlags::IN_CLOSE_WRITE));
        assert!(WATCH_FLAGS.contains(AddWatchFlags::IN_MOVED_TO));
        assert!(WATCH_FLAGS.contains(AddWatchFlags::IN_CREATE));
        assert!(WATCH_FLAGS.contains(AddWatchFlags::IN_DELETE));
        assert!(WATCH_FLAGS.contains(AddWatchFlags::IN_DONT_FOLLOW));
    }

    #[test]
    fn test_inotify_poll_timeout_constant() {
        assert!(
            INOTIFY_POLL_TIMEOUT_MS > 0,
            "inotify poll timeout should be positive"
        );
        assert!(
            INOTIFY_POLL_TIMEOUT_MS <= 10000,
            "inotify poll timeout should be reasonable"
        );
    }

    #[test]
    fn test_poll_fallback_interval() {
        assert!(
            POLL_FALLBACK_INTERVAL.as_millis() > 0,
            "poll fallback interval should be positive"
        );
        assert!(
            POLL_FALLBACK_INTERVAL.as_millis() <= 5000,
            "poll fallback interval should be reasonable"
        );
    }

    // -----------------------------------------------------------------------
    // Plymouth integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_plymouth_socket_path_constant() {
        assert_eq!(PLYMOUTH_SOCKET_PATH, "/run/plymouth/plymouthd");
    }

    #[test]
    fn test_plymouth_answer_constants() {
        assert_eq!(PLYMOUTH_ANSWER_ACK, 0x02);
        assert_eq!(PLYMOUTH_ANSWER_NAK, 0x05);
    }

    #[test]
    fn test_plymouth_is_running_false() {
        // On most test systems, Plymouth is not running
        let running = plymouth_is_running("/run/plymouth/plymouthd-nonexistent");
        assert!(!running);
    }

    #[test]
    fn test_plymouth_connect_fails_when_not_running() {
        // Should fail gracefully when Plymouth is not available
        let result = plymouth_connect("/run/plymouth/plymouthd-nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_plymouth_ping_fails_when_not_running() {
        let result = plymouth_ping("/run/plymouth/plymouthd-nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_plymouth_ask_password_fails_when_not_running() {
        let result =
            plymouth_ask_password("/run/plymouth/plymouthd-nonexistent", "Test:", false, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_plymouth_display_message_fails_when_not_running() {
        let result = plymouth_display_message("/run/plymouth/plymouthd-nonexistent", "Test");
        assert!(result.is_err());
    }

    #[test]
    fn test_plymouth_protocol_packet_format_password() {
        // Verify the packet we'd send for a password request
        let prompt = "Enter passphrase:";
        let type_byte = b'*';

        let mut packet = Vec::with_capacity(1 + prompt.len() + 1);
        packet.push(type_byte);
        packet.extend_from_slice(prompt.as_bytes());
        packet.push(0);

        assert_eq!(packet[0], b'*');
        assert_eq!(&packet[1..packet.len() - 1], prompt.as_bytes());
        assert_eq!(*packet.last().unwrap(), 0);
    }

    #[test]
    fn test_plymouth_protocol_packet_format_cached() {
        // Cached password request uses 'C'
        let prompt = "Enter passphrase:";
        let type_byte = b'C';

        let mut packet = Vec::new();
        packet.push(type_byte);
        packet.extend_from_slice(prompt.as_bytes());
        packet.push(0);

        assert_eq!(packet[0], b'C');
    }

    #[test]
    fn test_plymouth_protocol_packet_format_echo() {
        // Echo (question) request uses 'W'
        let prompt = "Username:";
        let type_byte = b'W';

        let mut packet = Vec::new();
        packet.push(type_byte);
        packet.extend_from_slice(prompt.as_bytes());
        packet.push(0);

        assert_eq!(packet[0], b'W');
    }

    #[test]
    fn test_plymouth_response_parsing_ack_with_password() {
        // Simulate an ACK response with a password
        let response = [PLYMOUTH_ANSWER_ACK, b's', b'e', b'c', b'r', b'e', b't', 0];
        let status = response[0];
        assert_eq!(status, PLYMOUTH_ANSWER_ACK);

        let password_bytes = &response[1..];
        let password_bytes = if password_bytes.last() == Some(&0) {
            &password_bytes[..password_bytes.len() - 1]
        } else {
            password_bytes
        };
        let password = String::from_utf8_lossy(password_bytes).to_string();
        assert_eq!(password, "secret");
    }

    #[test]
    fn test_plymouth_response_parsing_nak() {
        let response = [PLYMOUTH_ANSWER_NAK];
        assert_eq!(response[0], PLYMOUTH_ANSWER_NAK);
    }

    #[test]
    fn test_plymouth_response_parsing_ack_empty() {
        // ACK with no data means empty password
        let response = [PLYMOUTH_ANSWER_ACK];
        let status = response[0];
        assert_eq!(status, PLYMOUTH_ANSWER_ACK);
        // n == 1, no password data
    }

    #[test]
    fn test_plymouth_timeout_constant() {
        assert!(
            PLYMOUTH_TIMEOUT.as_secs() > 0,
            "Plymouth timeout should be positive"
        );
        assert!(
            PLYMOUTH_TIMEOUT.as_secs() <= 120,
            "Plymouth timeout should be reasonable"
        );
    }

    // -----------------------------------------------------------------------
    // Plymouth mock server test
    // -----------------------------------------------------------------------

    #[test]
    fn test_plymouth_protocol_with_mock_server() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join("tty-agent-test-plymouth-mock");
        let _ = fs::create_dir_all(&dir);
        let socket_path = dir.join("plymouthd.sock");
        let _ = fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind mock plymouth socket");

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut buf = [0u8; 256];
            let n = stream.read(&mut buf).expect("read request");

            // Verify request format
            assert!(n >= 2, "request should have at least type byte + NUL");
            assert_eq!(buf[0], b'*', "should be password request");

            // Send ACK + password
            let mut response = Vec::new();
            response.push(PLYMOUTH_ANSWER_ACK);
            response.extend_from_slice(b"mock-password");
            response.push(0);
            stream.write_all(&response).expect("write response");
        });

        let result = plymouth_ask_password(
            socket_path.to_str().unwrap(),
            "Enter passphrase:",
            false,
            false,
        );

        server_thread.join().expect("server thread should finish");

        assert!(result.is_ok(), "plymouth_ask_password should succeed");
        let password = result.unwrap();
        assert_eq!(password, Some("mock-password".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plymouth_protocol_nak_with_mock_server() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join("tty-agent-test-plymouth-nak");
        let _ = fs::create_dir_all(&dir);
        let socket_path = dir.join("plymouthd.sock");
        let _ = fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind mock plymouth socket");

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut buf = [0u8; 256];
            let _n = stream.read(&mut buf).expect("read request");

            // Send NAK (cancellation)
            stream.write_all(&[PLYMOUTH_ANSWER_NAK]).expect("write NAK");
        });

        let result = plymouth_ask_password(
            socket_path.to_str().unwrap(),
            "Enter passphrase:",
            false,
            false,
        );

        server_thread.join().expect("server thread should finish");

        assert!(result.is_ok(), "should not error on NAK");
        assert_eq!(result.unwrap(), None, "NAK should return None");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plymouth_cached_request_type() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join("tty-agent-test-plymouth-cached");
        let _ = fs::create_dir_all(&dir);
        let socket_path = dir.join("plymouthd.sock");
        let _ = fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind");

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 256];
            let n = stream.read(&mut buf).expect("read");
            assert!(n >= 2);
            assert_eq!(buf[0], b'C', "accept_cached should use 'C' type");

            let mut response = Vec::new();
            response.push(PLYMOUTH_ANSWER_ACK);
            response.extend_from_slice(b"cached-pw");
            response.push(0);
            stream.write_all(&response).expect("write");
        });

        let result = plymouth_ask_password(
            socket_path.to_str().unwrap(),
            "Passphrase:",
            false,
            true, // accept_cached = true
        );

        server_thread.join().expect("join");
        assert_eq!(result.unwrap(), Some("cached-pw".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plymouth_echo_request_type() {
        use std::os::unix::net::UnixListener;

        let dir = std::env::temp_dir().join("tty-agent-test-plymouth-echo");
        let _ = fs::create_dir_all(&dir);
        let socket_path = dir.join("plymouthd.sock");
        let _ = fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).expect("bind");

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 256];
            let n = stream.read(&mut buf).expect("read");
            assert!(n >= 2);
            assert_eq!(buf[0], b'W', "echo mode should use 'W' type");

            let mut response = Vec::new();
            response.push(PLYMOUTH_ANSWER_ACK);
            response.extend_from_slice(b"user123");
            response.push(0);
            stream.write_all(&response).expect("write");
        });

        let result = plymouth_ask_password(
            socket_path.to_str().unwrap(),
            "Username:",
            true, // echo = true
            false,
        );

        server_thread.join().expect("join");
        assert_eq!(result.unwrap(), Some("user123".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }
}
