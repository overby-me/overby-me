//! systemd-ask-password — Query the user for a system password.
//!
//! A drop-in replacement for `systemd-ask-password(1)`. This tool queries
//! the user for a password, either directly on the terminal or by posting
//! a password query file in `/run/systemd/ask-password/` for agents to
//! pick up.
//!
//! The password query protocol works as follows:
//!
//! 1. A question file `ask.XXXX` is created in `/run/systemd/ask-password/`
//!    containing metadata about what password is needed and a Unix socket
//!    path where the answer should be sent.
//!
//! 2. An inotify watch or polling agent (like `systemd-tty-ask-password-agent`)
//!    picks up the question file, prompts the user, and sends the password
//!    back through the specified socket.
//!
//! 3. If `--no-tty` is not given and stdin is a terminal, the tool will
//!    also try to read the password directly from the TTY.
//!
//! ## Kernel keyring caching
//!
//! When `--keyname` is specified, the tool first checks the Linux kernel
//! keyring for a previously cached password under that name. If found, the
//! cached password is returned immediately without prompting. After a
//! password is successfully obtained (from TTY, agent, or stdin), it is
//! stored in the kernel keyring for future lookups. The cached key is
//! placed in the user session keyring (`KEY_SPEC_USER_SESSION_KEYRING`)
//! with a timeout of 2.5 minutes (matching systemd's default).
//!
//! The kernel keyring API uses the `add_key(2)`, `request_key(2)`, and
//! `keyctl(2)` syscalls directly, with no external library dependency.
//!
//! Exit codes:
//!   0 — Password was successfully obtained and printed to stdout.
//!   1 — No password was provided (timeout, cancel, or error).

use clap::Parser;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

/// The directory where password query files are placed.
const ASK_PASSWORD_DIR: &str = "/run/systemd/ask-password";

// ---------------------------------------------------------------------------
// Kernel keyring constants and syscall wrappers
// ---------------------------------------------------------------------------

/// Keyring ID for the user session keyring.
const KEY_SPEC_USER_SESSION_KEYRING: i32 = -5;

/// `keyctl` command: set timeout on a key.
const KEYCTL_SET_TIMEOUT: u32 = 15;

/// `keyctl` command: read the payload of a key.
const KEYCTL_READ: u32 = 11;

/// Default cache timeout in seconds (2.5 minutes, matching systemd).
const KEYRING_CACHE_TIMEOUT_SECS: u32 = 150;

/// Add a key to a keyring via the `add_key(2)` syscall.
///
/// Returns the key serial number on success, or a negative errno on failure.
fn sys_add_key(key_type: &str, description: &str, payload: &[u8], keyring: i32) -> io::Result<i64> {
    let type_cstr = std::ffi::CString::new(key_type)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let desc_cstr = std::ffi::CString::new(description)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let ret = unsafe {
        libc::syscall(
            libc::SYS_add_key,
            type_cstr.as_ptr(),
            desc_cstr.as_ptr(),
            payload.as_ptr() as *const libc::c_void,
            payload.len(),
            keyring,
        )
    };

    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// Search for a key via the `request_key(2)` syscall.
///
/// Returns the key serial number on success, or an error if not found.
fn sys_request_key(key_type: &str, description: &str) -> io::Result<i64> {
    let type_cstr = std::ffi::CString::new(key_type)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let desc_cstr = std::ffi::CString::new(description)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let ret = unsafe {
        libc::syscall(
            libc::SYS_request_key,
            type_cstr.as_ptr(),
            desc_cstr.as_ptr(),
            std::ptr::null::<libc::c_void>(), // callout_info
            0i32,                             // dest_keyring (0 = don't link)
        )
    };

    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

/// Call `keyctl(2)` with KEYCTL_SET_TIMEOUT to set key expiry.
fn sys_keyctl_set_timeout(key_id: i64, timeout_secs: u32) -> io::Result<()> {
    let ret = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_SET_TIMEOUT as libc::c_long,
            key_id as libc::c_long,
            timeout_secs as libc::c_long,
        )
    };

    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Call `keyctl(2)` with KEYCTL_READ to read a key's payload.
fn sys_keyctl_read(key_id: i64) -> io::Result<Vec<u8>> {
    // First call with NULL buffer to get the size
    let size = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_READ as libc::c_long,
            key_id as libc::c_long,
            std::ptr::null::<libc::c_void>(),
            0 as libc::c_long,
        )
    };

    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; size as usize];
    let n = unsafe {
        libc::syscall(
            libc::SYS_keyctl,
            KEYCTL_READ as libc::c_long,
            key_id as libc::c_long,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as libc::c_long,
        )
    };

    if n < 0 {
        return Err(io::Error::last_os_error());
    }

    buf.truncate(n as usize);
    Ok(buf)
}

// ---------------------------------------------------------------------------
// High-level keyring cache API
// ---------------------------------------------------------------------------

/// Try to retrieve a cached password from the kernel keyring.
///
/// Returns `Some(password)` if the key exists and is readable, `None` otherwise.
fn keyring_cache_get(keyname: &str) -> Option<String> {
    let key_id = sys_request_key("user", keyname).ok()?;
    let payload = sys_keyctl_read(key_id).ok()?;
    String::from_utf8(payload).ok()
}

/// Store a password in the kernel keyring with a timeout.
///
/// The key is placed in the user session keyring and given a 2.5-minute
/// timeout (matching systemd's default).
fn keyring_cache_put(keyname: &str, password: &str) -> io::Result<()> {
    let key_id = sys_add_key(
        "user",
        keyname,
        password.as_bytes(),
        KEY_SPEC_USER_SESSION_KEYRING,
    )?;

    // Set timeout so the cached password expires automatically
    if let Err(e) = sys_keyctl_set_timeout(key_id, KEYRING_CACHE_TIMEOUT_SECS) {
        eprintln!("Warning: failed to set keyring timeout: {e}");
        // Non-fatal — the key was still added
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "systemd-ask-password",
    about = "Query the user for a system password",
    version
)]
struct Cli {
    /// Specify an icon name for the password query (currently informational
    /// only; used by graphical agents).
    #[arg(long, value_name = "ICON")]
    icon: Option<String>,

    /// Specify an identifier for the password query. This is used by
    /// agents to recognize repeated queries for the same password and
    /// cache them.
    #[arg(long, value_name = "ID")]
    id: Option<String>,

    /// Specify a key name for the kernel keyring. If set, the password
    /// is cached in the kernel keyring under this name so subsequent
    /// queries can return it without re-prompting.
    #[arg(long, value_name = "NAME")]
    keyname: Option<String>,

    /// Specify a credential name. Used for automatic credential lookup
    /// via `$CREDENTIALS_DIRECTORY`.
    #[arg(long, value_name = "NAME")]
    credential: Option<String>,

    /// Timeout in seconds. If no password is entered within this time,
    /// the query is cancelled. Defaults to 90 seconds.
    #[arg(long, value_name = "SEC", default_value = "90")]
    timeout: u64,

    /// If set, do not echo the password while typing. This is the default
    /// behavior (kept for compatibility).
    #[arg(long)]
    echo: bool,

    /// Do not print a trailing newline after the password on stdout.
    #[arg(long)]
    no_newline: bool,

    /// Accept empty passwords. By default, empty input is rejected.
    #[arg(long)]
    accept_cached: bool,

    /// Do not query on the TTY directly, only use the agent protocol.
    #[arg(long)]
    no_tty: bool,

    /// Emit question through the agent protocol even if we can ask on
    /// the TTY. This allows both TTY input and agent-based input.
    #[arg(long)]
    multiple: bool,

    /// Do not query via the agent protocol at all.
    #[arg(long)]
    no_agent: bool,

    /// The human-readable prompt message. Defaults to "Password:".
    #[arg(default_value = "Password:")]
    message: String,
}

/// A password question file for the agent protocol.
struct QuestionFile {
    path: PathBuf,
    socket_path: PathBuf,
}

impl QuestionFile {
    /// Create a new question file in `/run/systemd/ask-password/`.
    fn create(cli: &Cli, deadline: Instant) -> io::Result<Self> {
        fs::create_dir_all(ASK_PASSWORD_DIR)?;

        let id = uuid::Uuid::new_v4();
        let filename = format!("ask.{}", id.as_simple());
        let path = Path::new(ASK_PASSWORD_DIR).join(&filename);
        let socket_name = format!("sck.{}", id.as_simple());
        let socket_path = Path::new(ASK_PASSWORD_DIR).join(&socket_name);

        let not_after = {
            let remaining = deadline.saturating_duration_since(Instant::now());
            // Convert to CLOCK_MONOTONIC microseconds
            let mut ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
            if ret == 0 {
                let now_usec = ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000;
                now_usec + remaining.as_micros() as u64
            } else {
                0
            }
        };

        let mut content = String::new();
        content.push_str("[Ask]\n");
        content.push_str(&format!("PID={}\n", process::id()));
        content.push_str(&format!("Socket={}\n", socket_path.display()));
        content.push_str(&format!(
            "AcceptCached={}\n",
            if cli.accept_cached { "yes" } else { "no" }
        ));
        content.push_str(&format!("Echo={}\n", if cli.echo { "yes" } else { "no" }));
        content.push_str(&format!("NotAfter={not_after}\n"));
        content.push_str(&format!("Message={}\n", cli.message));

        if let Some(ref icon) = cli.icon {
            content.push_str(&format!("Icon={icon}\n"));
        }

        if let Some(ref id) = cli.id {
            content.push_str(&format!("Id={id}\n"));
        }

        fs::write(&path, &content)?;

        Ok(QuestionFile { path, socket_path })
    }

    /// Listen on the socket for a password response from an agent.
    fn wait_for_response(&self, timeout: Duration) -> io::Result<Option<String>> {
        // Remove stale socket if it exists
        let _ = fs::remove_file(&self.socket_path);

        let sock = UnixDatagram::bind(&self.socket_path)?;
        sock.set_read_timeout(Some(timeout))?;

        // Make socket world-writable so agents running as different users can reply
        let path_cstr =
            std::ffi::CString::new(self.socket_path.to_str().unwrap_or_default()).unwrap();
        unsafe {
            libc::chmod(path_cstr.as_ptr(), 0o666);
        }

        let mut buf = [0u8; 4096];
        match sock.recv(&mut buf) {
            Ok(n) => {
                if n == 0 {
                    return Ok(None);
                }
                // The agent protocol sends the password with a leading '+' for
                // success or '-' for failure/cancellation.
                let response = std::str::from_utf8(&buf[..n])
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if let Some(stripped) = response.strip_prefix('+') {
                    Ok(Some(stripped.to_string()))
                } else {
                    Ok(None)
                }
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(e) => Err(e),
        }
    }
}

impl Drop for QuestionFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(&self.socket_path);
    }
}

/// Check if stdin is a TTY.
fn stdin_is_tty() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

/// Read a password directly from the terminal.
///
/// If `echo` is false, terminal echo is disabled while reading.
fn read_password_from_tty(prompt: &str, echo: bool) -> io::Result<String> {
    // Open /dev/tty directly so we can read even if stdin is redirected
    let tty_path = "/dev/tty";
    let tty_fd = {
        let path_cstr = std::ffi::CString::new(tty_path).unwrap();
        let fd = unsafe { libc::open(path_cstr.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        fd
    };

    // Write prompt
    let prompt_bytes = prompt.as_bytes();
    unsafe {
        libc::write(tty_fd, prompt_bytes.as_ptr().cast(), prompt_bytes.len());
        libc::write(tty_fd, b" ".as_ptr().cast(), 1);
    }

    // Save terminal settings and disable echo if needed
    let mut old_termios: libc::termios = unsafe { std::mem::zeroed() };
    let termios_saved = unsafe { libc::tcgetattr(tty_fd, &mut old_termios) } == 0;

    if !echo && termios_saved {
        let mut new_termios = old_termios;
        new_termios.c_lflag &= !(libc::ECHO);
        unsafe {
            libc::tcsetattr(tty_fd, libc::TCSANOW, &new_termios);
        }
    }

    // Read password
    let mut password = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = unsafe { libc::read(tty_fd, byte.as_mut_ptr().cast(), 1) };
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
            // Restore terminal settings
            if !echo && termios_saved {
                unsafe {
                    libc::tcsetattr(tty_fd, libc::TCSANOW, &old_termios);
                }
            }
            // Print newline
            unsafe {
                libc::write(tty_fd, b"\n".as_ptr().cast(), 1);
            }
            unsafe {
                libc::close(tty_fd);
            }
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        password.push(byte[0]);
    }

    // Restore terminal settings
    if !echo && termios_saved {
        unsafe {
            libc::tcsetattr(tty_fd, libc::TCSANOW, &old_termios);
        }
        // Print newline since echo was off
        unsafe {
            libc::write(tty_fd, b"\n".as_ptr().cast(), 1);
        }
    }

    unsafe {
        libc::close(tty_fd);
    }

    String::from_utf8(password).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Try to read a password from $CREDENTIALS_DIRECTORY/<name>.
fn try_credential(name: &str) -> Option<String> {
    let dir = std::env::var("CREDENTIALS_DIRECTORY").ok()?;
    let path = Path::new(&dir).join(name);
    fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim_end().to_string())
}

/// Output a password and exit successfully. If `keyname` is set, cache
/// the password in the kernel keyring before printing.
fn output_password(password: &str, no_newline: bool, keyname: Option<&str>) -> ! {
    // Cache in kernel keyring if requested
    if let Some(kn) = keyname
        && let Err(e) = keyring_cache_put(kn, password)
    {
        eprintln!("Warning: failed to cache password in kernel keyring: {e}");
    }

    print!("{password}");
    if !no_newline {
        println!();
    }
    io::stdout().flush().ok();
    process::exit(0);
}

// ---------------------------------------------------------------------------
// Varlink server (io.systemd.AskPassword)
// ---------------------------------------------------------------------------

/// The io.systemd.AskPassword interface description, returned by
/// `org.varlink.service.GetInterfaceDescription`.
const ASK_PASSWORD_INTERFACE: &str = "\
interface io.systemd.AskPassword

method Ask(
  message: ?string,
  keyname: ?string,
  icon: ?string,
  id: ?string,
  timeoutUSec: ?int,
  untilUSec: ?int,
  acceptCached: ?bool,
  pushCache: ?bool,
  echo: ?string
) -> (
  passwords: []string
)

error NoPasswordAvailable()
error TimeoutReached()
";

/// Serve the `io.systemd.AskPassword` Varlink interface. Entered when invoked
/// via socket activation or `varlinkctl call <exe>` — both set `LISTEN_FDS` and
/// pass the socket as fd 3.
fn serve_varlink() -> i32 {
    use std::os::fd::{FromRawFd, RawFd};
    const FD: RawFd = 3;

    // Is fd 3 a listening socket (socket activation) or an already-connected
    // stream (`varlinkctl` exec mode)?
    let listening = unsafe {
        let mut val: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        libc::getsockopt(
            FD,
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            &mut val as *mut _ as *mut libc::c_void,
            &mut len,
        ) == 0
            && val != 0
    };

    if listening {
        let listener = unsafe { UnixListener::from_raw_fd(FD) };
        for conn in listener.incoming() {
            match conn {
                Ok(stream) => handle_varlink_conn(stream),
                Err(_) => break,
            }
        }
    } else {
        let stream = unsafe { UnixStream::from_raw_fd(FD) };
        handle_varlink_conn(stream);
    }
    0
}

/// Read NUL-delimited JSON requests from one connection and reply to each.
fn handle_varlink_conn(mut stream: UnixStream) {
    let mut buf = Vec::new();
    let mut read_buf = [0u8; 4096];
    loop {
        let n = match stream.read(&mut read_buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&read_buf[..n]);
        while let Some(pos) = buf.iter().position(|&b| b == 0) {
            let msg: Vec<u8> = buf[..pos].to_vec();
            buf.drain(..=pos);
            let response = dispatch_varlink(&msg);
            let mut out = serde_json::to_vec(&response).unwrap_or_default();
            out.push(0);
            if stream.write_all(&out).is_err() {
                return;
            }
        }
    }
}

/// Parse and dispatch a single Varlink message.
fn dispatch_varlink(msg_bytes: &[u8]) -> serde_json::Value {
    let msg: serde_json::Value = match serde_json::from_slice(msg_bytes) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({
                "error": "org.varlink.service.InvalidParameter",
                "parameters": {"parameter": format!("JSON parse error: {e}")}
            });
        }
    };
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = msg
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    match method {
        "org.varlink.service.GetInfo" => serde_json::json!({
            "parameters": {
                "vendor": "The systemd (rust) Project",
                "product": "systemd-ask-password",
                "version": env!("CARGO_PKG_VERSION"),
                "url": "https://systemd.io/",
                "interfaces": ["org.varlink.service", "io.systemd.AskPassword"]
            }
        }),
        "org.varlink.service.GetInterfaceDescription" => serde_json::json!({
            "parameters": {"description": ASK_PASSWORD_INTERFACE}
        }),
        "io.systemd.AskPassword.Ask" => vl_method_ask(&params),
        _ => serde_json::json!({
            "error": "org.varlink.service.MethodNotFound",
            "parameters": {"method": method}
        }),
    }
}

/// The `io.systemd.AskPassword.Ask` method: run the agent flow and return the
/// collected passwords. Mirrors upstream `vl_method_ask`.
fn vl_method_ask(params: &serde_json::Value) -> serde_json::Value {
    let str_field = |k: &str| params.get(k).and_then(|v| v.as_str()).map(String::from);
    let message = str_field("message").unwrap_or_else(|| "Password:".to_string());
    let keyname = str_field("keyname");
    let accept_cached = params
        .get("acceptCached")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // timeoutUSec is microseconds; 0 means cache-only (do not ask agents).
    let (timeout_secs, no_agent) = match params.get("timeoutUSec").and_then(|v| v.as_u64()) {
        Some(0) => (0u64, true),
        Some(us) => (us / 1_000_000, false),
        None => (90, false),
    };

    let cli = Cli {
        icon: str_field("icon"),
        id: str_field("id"),
        keyname: keyname.clone(),
        credential: None,
        timeout: timeout_secs.max(1),
        echo: false,
        no_newline: false,
        accept_cached,
        no_tty: true,
        multiple: false,
        no_agent,
        message,
    };

    // Check the kernel keyring cache first, like the CLI path.
    if let Some(ref kn) = keyname
        && let Some(cached) = keyring_cache_get(kn)
    {
        return serde_json::json!({ "parameters": {"passwords": [cached]} });
    }

    if no_agent {
        return serde_json::json!({ "error": "io.systemd.AskPassword.NoPasswordAvailable" });
    }

    let deadline = Instant::now() + Duration::from_secs(cli.timeout);
    let question = match QuestionFile::create(&cli, deadline) {
        Ok(q) => q,
        Err(_) => {
            return serde_json::json!({ "error": "io.systemd.AskPassword.NoPasswordAvailable" });
        }
    };
    match question.wait_for_response(Duration::from_secs(cli.timeout)) {
        Ok(Some(pw)) => serde_json::json!({ "parameters": {"passwords": [pw]} }),
        Ok(None) => serde_json::json!({ "error": "io.systemd.AskPassword.TimeoutReached" }),
        Err(_) => serde_json::json!({ "error": "io.systemd.AskPassword.NoPasswordAvailable" }),
    }
}

fn main() {
    // Socket-activated / `varlinkctl`-invoked Varlink server mode: LISTEN_FDS is
    // set and the (listening or connected) socket is passed as fd 3.
    if std::env::var_os("LISTEN_FDS").is_some() {
        std::process::exit(serve_varlink());
    }

    let cli = Cli::parse();

    let timeout = Duration::from_secs(cli.timeout);
    let deadline = Instant::now() + timeout;

    // Try credential lookup first
    if let Some(ref cred_name) = cli.credential
        && let Some(password) = try_credential(cred_name)
    {
        output_password(&password, cli.no_newline, cli.keyname.as_deref());
    }

    // Try kernel keyring cache lookup
    if let Some(ref keyname) = cli.keyname
        && let Some(cached) = keyring_cache_get(keyname)
    {
        output_password(&cached, cli.no_newline, None); // already cached, no need to re-store
    }

    let can_tty = !cli.no_tty && stdin_is_tty();
    let use_agent = !cli.no_agent && (!can_tty || cli.multiple);

    // If using the agent protocol, create the question file
    let question = if use_agent {
        match QuestionFile::create(&cli, deadline) {
            Ok(q) => Some(q),
            Err(e) => {
                eprintln!("Warning: failed to create password question file: {e}");
                None
            }
        }
    } else {
        None
    };

    // If we can query directly on the TTY, do so
    if can_tty {
        match read_password_from_tty(&cli.message, cli.echo) {
            Ok(password) => {
                if password.is_empty() && !cli.accept_cached {
                    eprintln!("Empty password not accepted.");
                    process::exit(1);
                }
                output_password(&password, cli.no_newline, cli.keyname.as_deref());
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                process::exit(1);
            }
            Err(e) => {
                eprintln!("Failed to read password from TTY: {e}");
                // Fall through to agent protocol if available
                if question.is_none() {
                    process::exit(1);
                }
            }
        }
    }

    // Wait for agent response
    if let Some(ref q) = question {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match q.wait_for_response(remaining) {
            Ok(Some(password)) => {
                if password.is_empty() && !cli.accept_cached {
                    eprintln!("Empty password not accepted.");
                    process::exit(1);
                }
                output_password(&password, cli.no_newline, cli.keyname.as_deref());
            }
            Ok(None) => {
                eprintln!("No password provided (timeout or cancellation).");
                process::exit(1);
            }
            Err(e) => {
                eprintln!("Error waiting for password: {e}");
                process::exit(1);
            }
        }
    }

    // If we get here, neither TTY nor agent worked
    if !can_tty && !use_agent {
        // Nothing to do — read from stdin as a fallback
        let mut password = String::new();
        match io::stdin().read_to_string(&mut password) {
            Ok(_) => {
                let password = password.trim_end_matches('\n').trim_end_matches('\r');
                if password.is_empty() && !cli.accept_cached {
                    eprintln!("Empty password not accepted.");
                    process::exit(1);
                }
                output_password(password, cli.no_newline, cli.keyname.as_deref());
            }
            Err(e) => {
                eprintln!("Error reading from stdin: {e}");
                process::exit(1);
            }
        }
    }

    eprintln!("No password obtained.");
    process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdin_is_tty_does_not_panic() {
        // In a test environment, stdin is typically not a TTY
        let _ = stdin_is_tty();
    }

    #[test]
    fn test_try_credential_missing() {
        // With no CREDENTIALS_DIRECTORY set, should return None
        unsafe { std::env::remove_var("CREDENTIALS_DIRECTORY") };
        assert!(try_credential("test").is_none());
    }

    #[test]
    fn test_try_credential_with_dir() {
        let dir = std::env::temp_dir().join("ask-password-test-creds");
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join("mypassword"), "s3cret\n").unwrap();

        unsafe { std::env::set_var("CREDENTIALS_DIRECTORY", dir.to_str().unwrap()) };
        let result = try_credential("mypassword");
        assert_eq!(result, Some("s3cret".to_string()));

        unsafe { std::env::remove_var("CREDENTIALS_DIRECTORY") };
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_question_file_protocol() {
        // Only run if we can create the directory (typically need root or /tmp)
        let test_dir = std::env::temp_dir().join("ask-password-test");
        let _ = fs::create_dir_all(&test_dir);

        // We can't easily test the full protocol without root, but we can
        // verify the data structures work.
        let id = uuid::Uuid::new_v4();
        assert!(!id.to_string().is_empty());

        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn test_ask_password_dir_constant() {
        assert_eq!(ASK_PASSWORD_DIR, "/run/systemd/ask-password");
    }

    // -----------------------------------------------------------------------
    // Kernel keyring caching tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_keyring_constants() {
        assert_eq!(KEY_SPEC_USER_SESSION_KEYRING, -5);
        assert_eq!(KEYCTL_SET_TIMEOUT, 15);
        assert_eq!(KEYCTL_READ, 11);
        assert_eq!(KEYRING_CACHE_TIMEOUT_SECS, 150);
    }

    #[test]
    fn test_keyring_cache_get_nonexistent() {
        // A random key name that certainly doesn't exist
        let keyname = format!("rust-systemd-test-nonexistent-{}", uuid::Uuid::new_v4());
        let result = keyring_cache_get(&keyname);
        assert!(
            result.is_none(),
            "non-existent key should return None, got: {:?}",
            result
        );
    }

    /// Helper: returns true if the kernel keyring supports full round-trip
    /// (add + request + read). Some sandboxed environments (e.g. Nix build)
    /// allow `add_key` but restrict `keyctl(KEYCTL_READ)` or `request_key`,
    /// so we probe once and skip round-trip assertions when unsupported.
    fn keyring_round_trip_works() -> bool {
        let probe_name = format!("rust-systemd-probe-{}", uuid::Uuid::new_v4());
        let probe_payload = b"probe";
        match sys_add_key(
            "user",
            &probe_name,
            probe_payload,
            KEY_SPEC_USER_SESSION_KEYRING,
        ) {
            Ok(_key_id) => {
                // Check if we can read back via request_key + keyctl_read
                match sys_request_key("user", &probe_name) {
                    Ok(found_id) => match sys_keyctl_read(found_id) {
                        Ok(data) => data == probe_payload,
                        Err(_) => false,
                    },
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }

    #[test]
    fn test_keyring_cache_put_and_get() {
        // This test requires the kernel keyring to be available and fully
        // functional. Some sandboxed environments allow add_key but restrict
        // keyctl(KEYCTL_READ), so we skip gracefully.
        if !keyring_round_trip_works() {
            eprintln!(
                "Skipping keyring put+get test (round-trip not supported in this environment)"
            );
            return;
        }

        let keyname = format!("rust-systemd-test-{}", uuid::Uuid::new_v4());
        let password = "test-password-12345";

        keyring_cache_put(&keyname, password).expect("put should succeed");
        let cached = keyring_cache_get(&keyname);
        assert_eq!(
            cached,
            Some(password.to_string()),
            "cached password should match what was stored"
        );
    }

    #[test]
    fn test_keyring_cache_put_overwrites() {
        if !keyring_round_trip_works() {
            eprintln!(
                "Skipping keyring overwrite test (round-trip not supported in this environment)"
            );
            return;
        }

        let keyname = format!("rust-systemd-test-overwrite-{}", uuid::Uuid::new_v4());
        let password1 = "first-password";
        let password2 = "second-password";

        keyring_cache_put(&keyname, password1).expect("first put should succeed");
        keyring_cache_put(&keyname, password2).expect("overwrite should succeed");
        let cached = keyring_cache_get(&keyname);
        assert_eq!(
            cached,
            Some(password2.to_string()),
            "cached password should be the latest one"
        );
    }

    #[test]
    fn test_keyring_cache_put_empty_password() {
        if !keyring_round_trip_works() {
            eprintln!("Skipping keyring empty test (round-trip not supported in this environment)");
            return;
        }

        let keyname = format!("rust-systemd-test-empty-{}", uuid::Uuid::new_v4());

        keyring_cache_put(&keyname, "").expect("put empty should succeed");
        let cached = keyring_cache_get(&keyname);
        assert_eq!(
            cached,
            Some(String::new()),
            "empty password should be cached correctly"
        );
    }

    #[test]
    fn test_keyring_cache_put_unicode_password() {
        if !keyring_round_trip_works() {
            eprintln!(
                "Skipping keyring unicode test (round-trip not supported in this environment)"
            );
            return;
        }

        let keyname = format!("rust-systemd-test-unicode-{}", uuid::Uuid::new_v4());
        let password = "пароль🔑密码";

        keyring_cache_put(&keyname, password).expect("put unicode should succeed");
        let cached = keyring_cache_get(&keyname);
        assert_eq!(
            cached,
            Some(password.to_string()),
            "unicode password should round-trip correctly"
        );
    }

    #[test]
    fn test_sys_request_key_invalid_type() {
        // Using an invalid key type should fail gracefully
        let result = sys_request_key("invalid-type-that-does-not-exist", "test");
        assert!(result.is_err(), "invalid key type should fail");
    }

    #[test]
    fn test_sys_add_key_nul_in_name() {
        // Key name with NUL byte should fail at CString creation
        let result = sys_add_key(
            "user",
            "test\0name",
            b"payload",
            KEY_SPEC_USER_SESSION_KEYRING,
        );
        assert!(
            result.is_err(),
            "key name with NUL should fail CString creation"
        );
    }

    #[test]
    fn test_sys_request_key_nul_in_name() {
        let result = sys_request_key("user", "test\0name");
        assert!(
            result.is_err(),
            "key name with NUL should fail CString creation"
        );
    }

    #[test]
    fn test_sys_keyctl_read_invalid_key() {
        // Reading from an invalid key ID should fail
        let result = sys_keyctl_read(-999999);
        assert!(result.is_err(), "reading invalid key ID should fail");
    }

    #[test]
    fn test_sys_keyctl_set_timeout_invalid_key() {
        let result = sys_keyctl_set_timeout(-999999, 60);
        assert!(
            result.is_err(),
            "setting timeout on invalid key should fail"
        );
    }

    #[test]
    fn test_keyring_cache_long_password() {
        if !keyring_round_trip_works() {
            eprintln!(
                "Skipping keyring long password test (round-trip not supported in this environment)"
            );
            return;
        }

        let keyname = format!("rust-systemd-test-long-{}", uuid::Uuid::new_v4());
        // 4KB password
        let password: String = (0..4096).map(|i| (b'a' + (i % 26) as u8) as char).collect();

        keyring_cache_put(&keyname, &password).expect("put long should succeed");
        let cached = keyring_cache_get(&keyname);
        assert_eq!(
            cached,
            Some(password),
            "long password should round-trip correctly"
        );
    }
}
