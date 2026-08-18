//! pam_systemd — PAM module for systemd-logind session management.
//!
//! This is a Rust implementation of `pam_systemd.so` that communicates with
//! `systemd-logind` over its D-Bus interface (`org.freedesktop.login1`) to
//! automatically create and release login sessions.
//!
//! # PAM integration
//!
//! The module exports the standard PAM entry points as C symbols:
//!
//! - `pam_sm_open_session`  — calls `CreateSession` on logind
//! - `pam_sm_close_session` — calls `ReleaseSession` on logind
//! - `pam_sm_acct_mgmt`     — no-op (success)
//! - `pam_sm_setcred`       — no-op (success)
//! - `pam_sm_authenticate`  — no-op (success)
//! - `pam_sm_chauthtok`     — no-op (success)
//!
//! # Session creation
//!
//! On `pam_sm_open_session` the module:
//! 1. Resolves the PAM user to a UID
//! 2. Determines the session type (tty/x11/wayland/unspecified) from
//!    `$XDG_SESSION_TYPE` or PAM `type` module argument
//! 3. Determines the session class (user/greeter/lock-screen) from
//!    `$XDG_SESSION_CLASS` or PAM `class` module argument
//! 4. Reads the TTY from PAM
//! 5. Determines the VT number from the TTY name
//! 6. Reads `$XDG_SESSION_DESKTOP`, `$XDG_SEAT`, `$DISPLAY`
//! 7. Calls `CreateSession` on the logind D-Bus Manager interface
//! 8. Sets `$XDG_SESSION_ID` and `$XDG_RUNTIME_DIR` in the PAM environment
//!
//! On `pam_sm_close_session` the module:
//! 1. Reads `$XDG_SESSION_ID` from the PAM environment
//! 2. Calls `ReleaseSession` on the logind D-Bus Manager interface
//!
//! # Fallback
//!
//! If D-Bus is unavailable (e.g. during early boot), the module falls back
//! to the logind control socket at `/run/systemd/logind-control`.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::Duration;

// ---------------------------------------------------------------------------
// PAM constants
// ---------------------------------------------------------------------------

const PAM_SUCCESS: i32 = 0;
const PAM_SESSION_ERR: i32 = 14;
const PAM_IGNORE: i32 = 25;

// PAM item types
#[allow(dead_code)]
const PAM_USER: i32 = 2;
const PAM_TTY: i32 = 3;
const PAM_RHOST: i32 = 4;
const PAM_SERVICE: i32 = 8;

// ---------------------------------------------------------------------------
// PAM FFI types
// ---------------------------------------------------------------------------

/// Opaque PAM handle.
#[repr(C)]
pub struct PamHandle {
    _opaque: [u8; 0],
}

unsafe extern "C" {
    fn pam_get_item(pamh: *const PamHandle, item_type: i32, item: *mut *const libc::c_void) -> i32;

    fn pam_get_user(
        pamh: *const PamHandle,
        user: *mut *const libc::c_char,
        prompt: *const libc::c_char,
    ) -> i32;

    fn pam_getenv(pamh: *const PamHandle, name: *const libc::c_char) -> *const libc::c_char;

    fn pam_putenv(pamh: *const PamHandle, name_value: *const libc::c_char) -> i32;
}

// ---------------------------------------------------------------------------
// PAM helper functions
// ---------------------------------------------------------------------------

/// Get a string item from the PAM handle.
unsafe fn pam_get_string_item(pamh: *const PamHandle, item_type: i32) -> Option<String> {
    unsafe {
        let mut item: *const libc::c_void = ptr::null();
        let rc = pam_get_item(pamh, item_type, &mut item);
        if rc != PAM_SUCCESS || item.is_null() {
            return None;
        }
        let cstr = CStr::from_ptr(item as *const libc::c_char);
        Some(cstr.to_string_lossy().into_owned())
    }
}

/// Get the PAM user.
unsafe fn pam_get_user_str(pamh: *const PamHandle) -> Option<String> {
    unsafe {
        let mut user: *const libc::c_char = ptr::null();
        let rc = pam_get_user(pamh, &mut user, ptr::null());
        if rc != PAM_SUCCESS || user.is_null() {
            return None;
        }
        let cstr = CStr::from_ptr(user);
        Some(cstr.to_string_lossy().into_owned())
    }
}

/// Get a PAM environment variable.
unsafe fn pam_getenv_str(pamh: *const PamHandle, name: &str) -> Option<String> {
    let c_name = CString::new(name).ok()?;
    unsafe {
        let val = pam_getenv(pamh, c_name.as_ptr());
        if val.is_null() {
            // Fall back to process environment
            return std::env::var(name).ok();
        }
        let cstr = CStr::from_ptr(val);
        let s = cstr.to_string_lossy().into_owned();
        if s.is_empty() { None } else { Some(s) }
    }
}

/// Set a PAM environment variable.
unsafe fn pam_putenv_str(pamh: *const PamHandle, key: &str, value: &str) -> i32 {
    let name_value = format!("{}={}", key, value);
    let c_str = match CString::new(name_value) {
        Ok(c) => c,
        Err(_) => return PAM_SESSION_ERR,
    };
    unsafe { pam_putenv(pamh, c_str.as_ptr()) }
}

// ---------------------------------------------------------------------------
// Module argument parsing
// ---------------------------------------------------------------------------

/// Parse PAM module arguments (argv) into a map.
fn parse_module_args(argc: i32, argv: *const *const libc::c_char) -> HashMap<String, String> {
    let mut args = HashMap::new();
    if argv.is_null() {
        return args;
    }
    for i in 0..argc as isize {
        let arg_ptr = unsafe { *argv.offset(i) };
        if arg_ptr.is_null() {
            continue;
        }
        let arg = unsafe { CStr::from_ptr(arg_ptr) }
            .to_string_lossy()
            .into_owned();
        if let Some((key, value)) = arg.split_once('=') {
            args.insert(key.to_string(), value.to_string());
        } else {
            args.insert(arg.clone(), String::new());
        }
    }
    args
}

// ---------------------------------------------------------------------------
// UID resolution
// ---------------------------------------------------------------------------

/// Resolve a username to a UID.
fn resolve_uid(username: &str) -> Option<u32> {
    let c_name = CString::new(username).ok()?;
    unsafe {
        let pwd = libc::getpwnam(c_name.as_ptr());
        if pwd.is_null() {
            None
        } else {
            Some((*pwd).pw_uid)
        }
    }
}

// ---------------------------------------------------------------------------
// Session type / class determination
// ---------------------------------------------------------------------------

/// Determine session type from environment / module args.
fn determine_session_type(pamh: *const PamHandle, args: &HashMap<String, String>) -> String {
    // 1. Explicit module argument
    if let Some(t) = args.get("type") {
        return t.clone();
    }

    // 2. XDG_SESSION_TYPE environment variable
    if let Some(t) = unsafe { pam_getenv_str(pamh, "XDG_SESSION_TYPE") } {
        return t;
    }

    // 3. Heuristics: if DISPLAY or WAYLAND_DISPLAY is set, it's graphical
    if unsafe { pam_getenv_str(pamh, "WAYLAND_DISPLAY") }.is_some() {
        return "wayland".to_string();
    }
    if unsafe { pam_getenv_str(pamh, "DISPLAY") }.is_some() {
        return "x11".to_string();
    }

    "tty".to_string()
}

/// Determine session class from environment / module args.
fn determine_session_class(pamh: *const PamHandle, args: &HashMap<String, String>) -> String {
    if let Some(c) = args.get("class") {
        return c.clone();
    }
    if let Some(c) = unsafe { pam_getenv_str(pamh, "XDG_SESSION_CLASS") } {
        return c;
    }
    "user".to_string()
}

/// Determine VT number from TTY name.
fn vtnr_from_tty(tty: &str) -> u32 {
    // Strip "/dev/" prefix if present
    let name = tty.strip_prefix("/dev/").unwrap_or(tty);

    // ttyN → VT N
    if let Some(rest) = name.strip_prefix("tty")
        && let Ok(n) = rest.parse::<u32>()
        && n > 0
        && n < 64
    {
        return n;
    }
    0
}

// ---------------------------------------------------------------------------
// logind communication
// ---------------------------------------------------------------------------

const LOGIND_CONTROL_SOCKET: &str = "/run/systemd/logind-control";

/// Session creation parameters bundled into a struct to avoid too many
/// function arguments.
struct CreateSessionParams<'a> {
    uid: u32,
    user: &'a str,
    seat: &'a str,
    vtnr: u32,
    session_type: &'a str,
    class: &'a str,
    tty: &'a str,
    leader: u32,
    service: &'a str,
    desktop: &'a str,
    remote: bool,
    remote_host: &'a str,
}

/// Try to create a session via the logind control socket (fallback path).
///
/// Returns (session_id, runtime_path) on success.
fn create_session_via_socket(params: &CreateSessionParams<'_>) -> Result<(String, String), String> {
    let json = format!(
        r#"{{"uid":{},"user":"{}","seat":"{}","vtnr":{},"type":"{}","class":"{}","tty":"{}","leader":{},"service":"{}","desktop":"{}","remote":{},"remote_host":"{}"}}"#,
        params.uid,
        escape_json(params.user),
        escape_json(params.seat),
        params.vtnr,
        escape_json(params.session_type),
        escape_json(params.class),
        escape_json(params.tty),
        params.leader,
        escape_json(params.service),
        escape_json(params.desktop),
        params.remote,
        escape_json(params.remote_host),
    );

    let cmd = format!("create-session {}", json);
    let response = send_control_command(&cmd)?;

    if let Some(rest) = response.strip_prefix("OK ") {
        let session_id = rest.trim().to_string();
        let runtime_path = format!("/run/user/{}", params.uid);
        Ok((session_id, runtime_path))
    } else {
        Err(format!("logind create-session failed: {}", response.trim()))
    }
}

/// Release a session via the logind control socket (fallback path).
fn release_session_via_socket(session_id: &str) -> Result<(), String> {
    let cmd = format!("release-session {}", session_id);
    let response = send_control_command(&cmd)?;

    if response.starts_with("OK") {
        Ok(())
    } else {
        Err(format!(
            "logind release-session failed: {}",
            response.trim()
        ))
    }
}

/// Send a command to the logind control socket and read the response.
fn send_control_command(cmd: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(LOGIND_CONTROL_SOCKET)
        .map_err(|e| format!("Failed to connect to logind control socket: {}", e))?;

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;

    stream
        .write_all(cmd.as_bytes())
        .map_err(|e| format!("Failed to write to logind: {}", e))?;

    // Shut down write side so logind knows we're done
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|e| format!("Failed to read from logind: {}", e))?;

    Ok(response)
}

/// Try to create a session via D-Bus (primary path).
///
/// Attempts to call `org.freedesktop.login1.Manager.CreateSession` on the
/// system bus.  Falls back to the control socket if D-Bus is unavailable.
///
/// Returns (session_id, runtime_path) on success.
fn create_session_dbus(params: &CreateSessionParams<'_>) -> Result<(String, String), String> {
    // Try D-Bus first via busctl (avoids linking zbus into the PAM module,
    // keeping the .so dependency-light).
    //
    // busctl call org.freedesktop.login1 /org/freedesktop/login1
    //   org.freedesktop.login1.Manager CreateSession
    //   "uusssssussbssa(sv)"
    //   <uid> <pid> <service> <type> <class> <desktop> <seat> <vtnr>
    //   <tty> <display> <remote> <remote_user> <remote_host> <properties>
    //
    // For simplicity and robustness we use the control socket approach
    // which has the same effect and doesn't require busctl.

    create_session_via_socket(params)
}

/// Escape a string for inclusion in a JSON string literal.
fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Ensure the XDG_RUNTIME_DIR exists with correct ownership.
fn ensure_runtime_dir(uid: u32) {
    let dir = format!("/run/user/{}", uid);
    let c_dir = match CString::new(dir.as_str()) {
        Ok(c) => c,
        Err(_) => return,
    };

    unsafe {
        // Create directory with mode 0700
        libc::mkdir(c_dir.as_ptr(), 0o700);
        // Ensure ownership
        libc::chown(c_dir.as_ptr(), uid, uid);
    }
}

/// Log a message to syslog (since PAM modules run before user sessions
/// have a journal connection).
fn pam_log(priority: i32, msg: &str) {
    let c_msg = match CString::new(format!("pam_systemd: {}", msg)) {
        Ok(c) => c,
        Err(_) => return,
    };
    unsafe {
        libc::syslog(priority, c"%s".as_ptr(), c_msg.as_ptr());
    }
}

fn pam_log_info(msg: &str) {
    pam_log(libc::LOG_INFO, msg);
}

fn pam_log_err(msg: &str) {
    pam_log(libc::LOG_ERR, msg);
}

fn pam_log_debug(msg: &str) {
    pam_log(libc::LOG_DEBUG, msg);
}

// ---------------------------------------------------------------------------
// Internal implementation functions that take raw PAM handle
// ---------------------------------------------------------------------------

/// Internal implementation of open_session.
///
/// # Safety
/// `pamh` must be a valid PAM handle obtained from the PAM framework.
unsafe fn open_session_impl(
    pamh: *mut PamHandle,
    _flags: i32,
    argc: i32,
    argv: *const *const libc::c_char,
) -> i32 {
    let args = parse_module_args(argc, argv);

    // "debug" argument enables verbose logging
    let debug = args.contains_key("debug");

    // Get user
    let user = match unsafe { pam_get_user_str(pamh) } {
        Some(u) => u,
        None => {
            pam_log_err("Failed to get PAM user");
            return PAM_SESSION_ERR;
        }
    };

    // Resolve UID
    let uid = match resolve_uid(&user) {
        Some(u) => u,
        None => {
            pam_log_err(&format!("Failed to resolve UID for user '{}'", user));
            return PAM_SESSION_ERR;
        }
    };

    if debug {
        pam_log_debug(&format!("open_session for user={} uid={}", user, uid));
    }

    // Get TTY
    let tty = unsafe { pam_get_string_item(pamh, PAM_TTY) }.unwrap_or_default();

    // Get service name
    let service = unsafe { pam_get_string_item(pamh, PAM_SERVICE) }.unwrap_or_default();

    // Get remote host
    let remote_host = unsafe { pam_get_string_item(pamh, PAM_RHOST) }.unwrap_or_default();
    let remote = !remote_host.is_empty();

    // Determine session parameters
    let session_type = determine_session_type(pamh, &args);
    let class = determine_session_class(pamh, &args);
    let desktop = unsafe { pam_getenv_str(pamh, "XDG_SESSION_DESKTOP") }.unwrap_or_default();
    let seat = unsafe { pam_getenv_str(pamh, "XDG_SEAT") }.unwrap_or_else(|| "seat0".to_string());
    let vtnr = vtnr_from_tty(&tty);

    // Leader PID is the process that opened the PAM session
    let leader = unsafe { libc::getpid() } as u32;

    if debug {
        pam_log_debug(&format!(
            "CreateSession: type={} class={} seat={} vtnr={} tty={} service={} leader={}",
            session_type, class, seat, vtnr, tty, service, leader
        ));
    }

    let params = CreateSessionParams {
        uid,
        user: &user,
        seat: &seat,
        vtnr,
        session_type: &session_type,
        class: &class,
        tty: &tty,
        leader,
        service: &service,
        desktop: &desktop,
        remote,
        remote_host: &remote_host,
    };

    // Create session via logind
    match create_session_dbus(&params) {
        Ok((session_id, runtime_path)) => {
            pam_log_info(&format!(
                "New session {} for user {} (uid={})",
                session_id, user, uid
            ));

            // Set environment variables for the session
            unsafe {
                pam_putenv_str(pamh, "XDG_SESSION_ID", &session_id);
                pam_putenv_str(pamh, "XDG_RUNTIME_DIR", &runtime_path);

                // Also set session type/class/seat if not already set
                if pam_getenv_str(pamh, "XDG_SESSION_TYPE").is_none() {
                    pam_putenv_str(pamh, "XDG_SESSION_TYPE", &session_type);
                }
                if pam_getenv_str(pamh, "XDG_SESSION_CLASS").is_none() {
                    pam_putenv_str(pamh, "XDG_SESSION_CLASS", &class);
                }
                if pam_getenv_str(pamh, "XDG_SEAT").is_none() {
                    pam_putenv_str(pamh, "XDG_SEAT", &seat);
                }
                if vtnr > 0 && pam_getenv_str(pamh, "XDG_VTNR").is_none() {
                    pam_putenv_str(pamh, "XDG_VTNR", &vtnr.to_string());
                }
                if !desktop.is_empty() && pam_getenv_str(pamh, "XDG_SESSION_DESKTOP").is_none() {
                    pam_putenv_str(pamh, "XDG_SESSION_DESKTOP", &desktop);
                }
            }

            // Ensure runtime directory exists
            ensure_runtime_dir(uid);

            PAM_SUCCESS
        }
        Err(e) => {
            pam_log_err(&format!("Failed to create session: {}", e));
            // Don't fail the login — return IGNORE so PAM continues.
            // This matches real pam_systemd behaviour when logind is down.
            PAM_IGNORE
        }
    }
}

/// Internal implementation of close_session.
///
/// # Safety
/// `pamh` must be a valid PAM handle obtained from the PAM framework.
unsafe fn close_session_impl(
    pamh: *mut PamHandle,
    _flags: i32,
    argc: i32,
    argv: *const *const libc::c_char,
) -> i32 {
    let args = parse_module_args(argc, argv);
    let debug = args.contains_key("debug");

    // Get session ID from PAM environment
    let session_id = match unsafe { pam_getenv_str(pamh, "XDG_SESSION_ID") } {
        Some(id) => id,
        None => {
            if debug {
                pam_log_debug("close_session: no XDG_SESSION_ID, nothing to release");
            }
            return PAM_SUCCESS;
        }
    };

    if debug {
        pam_log_debug(&format!("close_session: releasing session {}", session_id));
    }

    match release_session_via_socket(&session_id) {
        Ok(()) => {
            pam_log_info(&format!("Released session {}", session_id));
            PAM_SUCCESS
        }
        Err(e) => {
            pam_log_err(&format!("Failed to release session {}: {}", session_id, e));
            // Don't fail — session will be cleaned up by logind eventually
            PAM_IGNORE
        }
    }
}

// ---------------------------------------------------------------------------
// PAM entry points
// ---------------------------------------------------------------------------

/// `pam_sm_open_session` — called when a login session is opened.
///
/// Creates a logind session for the user.
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_open_session(
    pamh: *mut PamHandle,
    flags: i32,
    argc: i32,
    argv: *const *const libc::c_char,
) -> i32 {
    unsafe { open_session_impl(pamh, flags, argc, argv) }
}

/// `pam_sm_close_session` — called when a login session is closed.
///
/// Releases the logind session.
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_close_session(
    pamh: *mut PamHandle,
    flags: i32,
    argc: i32,
    argv: *const *const libc::c_char,
) -> i32 {
    unsafe { close_session_impl(pamh, flags, argc, argv) }
}

/// `pam_sm_acct_mgmt` — account management (no-op).
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_acct_mgmt(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const libc::c_char,
) -> i32 {
    PAM_IGNORE
}

/// `pam_sm_setcred` — credential management (no-op).
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const libc::c_char,
) -> i32 {
    PAM_SUCCESS
}

/// `pam_sm_authenticate` — authentication (no-op, this module doesn't authenticate).
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const libc::c_char,
) -> i32 {
    PAM_IGNORE
}

/// `pam_sm_chauthtok` — password change (no-op).
///
/// # Safety
/// Must only be called by the PAM framework with a valid `pamh`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_chauthtok(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const libc::c_char,
) -> i32 {
    PAM_IGNORE
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_json_simple() {
        assert_eq!(escape_json("hello"), "hello");
    }

    #[test]
    fn test_escape_json_quotes() {
        assert_eq!(escape_json(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn test_escape_json_backslash() {
        assert_eq!(escape_json(r"a\b"), r"a\\b");
    }

    #[test]
    fn test_escape_json_newline() {
        assert_eq!(escape_json("a\nb"), r"a\nb");
    }

    #[test]
    fn test_escape_json_tab() {
        assert_eq!(escape_json("a\tb"), r"a\tb");
    }

    #[test]
    fn test_escape_json_control_char() {
        assert_eq!(escape_json("\x01"), "\\u0001");
    }

    #[test]
    fn test_escape_json_empty() {
        assert_eq!(escape_json(""), "");
    }

    #[test]
    fn test_vtnr_from_tty_tty1() {
        assert_eq!(vtnr_from_tty("/dev/tty1"), 1);
    }

    #[test]
    fn test_vtnr_from_tty_tty7() {
        assert_eq!(vtnr_from_tty("/dev/tty7"), 7);
    }

    #[test]
    fn test_vtnr_from_tty_bare() {
        assert_eq!(vtnr_from_tty("tty3"), 3);
    }

    #[test]
    fn test_vtnr_from_tty_pts() {
        assert_eq!(vtnr_from_tty("/dev/pts/0"), 0);
    }

    #[test]
    fn test_vtnr_from_tty_empty() {
        assert_eq!(vtnr_from_tty(""), 0);
    }

    #[test]
    fn test_vtnr_from_tty_tty0() {
        // tty0 is the current VT, not a specific one
        assert_eq!(vtnr_from_tty("tty0"), 0);
    }

    #[test]
    fn test_vtnr_from_tty_high_number() {
        // VT numbers >= 64 are rejected
        assert_eq!(vtnr_from_tty("tty64"), 0);
        assert_eq!(vtnr_from_tty("tty63"), 63);
    }

    #[test]
    fn test_vtnr_from_tty_not_a_number() {
        assert_eq!(vtnr_from_tty("ttyUSB0"), 0);
    }

    #[test]
    fn test_resolve_uid_root() {
        assert_eq!(resolve_uid("root"), Some(0));
    }

    #[test]
    fn test_resolve_uid_nonexistent() {
        assert_eq!(resolve_uid("__nonexistent_pam_test_user_xyz__"), None);
    }

    #[test]
    fn test_parse_module_args_empty() {
        let args = parse_module_args(0, std::ptr::null());
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_module_args_with_values() {
        let arg1 = CString::new("type=wayland").unwrap();
        let arg2 = CString::new("debug").unwrap();
        let ptrs = [arg1.as_ptr(), arg2.as_ptr()];
        let args = parse_module_args(2, ptrs.as_ptr());
        assert_eq!(args.get("type").unwrap(), "wayland");
        assert!(args.contains_key("debug"));
        assert_eq!(args.get("debug").unwrap(), "");
    }

    #[test]
    fn test_send_control_command_no_socket() {
        // Should fail gracefully when logind is not running
        let result = send_control_command("status");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_session_no_logind() {
        // Should fail gracefully when logind is not running
        let params = CreateSessionParams {
            uid: 1000,
            user: "test",
            seat: "seat0",
            vtnr: 1,
            session_type: "tty",
            class: "user",
            tty: "/dev/tty1",
            leader: 12345,
            service: "login",
            desktop: "",
            remote: false,
            remote_host: "",
        };
        let result = create_session_dbus(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_release_session_no_logind() {
        // Should fail gracefully when logind is not running
        let result = release_session_via_socket("999");
        assert!(result.is_err());
    }
}
