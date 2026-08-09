use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

use crate::units::{
    PlatformSpecificServiceFields, RLimitValue, ResourceLimit, StandardInput, UtmpMode,
};

/// Process C-style escape sequences in a string, matching systemd's
/// `cunescape()`.  Handles: `\\`, `\a`, `\b`, `\f`, `\n`, `\r`, `\t`,
/// `\v`, `\xHH` (hex byte), `\NNN` (octal byte).  Unknown escapes are
/// passed through literally.  Returns raw bytes because the unescaped
/// data may contain arbitrary byte values (e.g. `\x00`).
pub(crate) fn cunescape(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'a' => {
                    out.push(0x07);
                    i += 2;
                }
                b'b' => {
                    out.push(0x08);
                    i += 2;
                }
                b'f' => {
                    out.push(0x0c);
                    i += 2;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'v' => {
                    out.push(0x0b);
                    i += 2;
                }
                b'x' if i + 3 < bytes.len() => {
                    // \xHH
                    if let Ok(byte) = u8::from_str_radix(
                        std::str::from_utf8(&bytes[i + 2..i + 4]).unwrap_or(""),
                        16,
                    ) {
                        out.push(byte);
                        i += 4;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                b'0'..=b'7' => {
                    // Octal: up to 3 digits
                    let start = i + 1;
                    let mut end = start + 1;
                    while end < bytes.len()
                        && end < start + 3
                        && bytes[end] >= b'0'
                        && bytes[end] <= b'7'
                    {
                        end += 1;
                    }
                    if let Ok(val) =
                        u8::from_str_radix(std::str::from_utf8(&bytes[start..end]).unwrap_or(""), 8)
                    {
                        out.push(val);
                        i = end;
                    } else {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
                _ => {
                    // Unknown escape — pass through literally
                    out.push(bytes[i]);
                    out.push(bytes[i + 1]);
                    i += 2;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Convert a Linux capability name (e.g. "CAP_SYS_TIME") to its numeric
/// value as defined in `<linux/capability.h>`.  Returns `None` for
/// unrecognised names.
fn cap_name_to_number(name: &str) -> Option<u64> {
    match name.to_uppercase().as_str() {
        "CAP_CHOWN" => Some(0),
        "CAP_DAC_OVERRIDE" => Some(1),
        "CAP_DAC_READ_SEARCH" => Some(2),
        "CAP_FOWNER" => Some(3),
        "CAP_FSETID" => Some(4),
        "CAP_KILL" => Some(5),
        "CAP_SETGID" => Some(6),
        "CAP_SETUID" => Some(7),
        "CAP_SETPCAP" => Some(8),
        "CAP_LINUX_IMMUTABLE" => Some(9),
        "CAP_NET_BIND_SERVICE" => Some(10),
        "CAP_NET_BROADCAST" => Some(11),
        "CAP_NET_ADMIN" => Some(12),
        "CAP_NET_RAW" => Some(13),
        "CAP_IPC_LOCK" => Some(14),
        "CAP_IPC_OWNER" => Some(15),
        "CAP_SYS_MODULE" => Some(16),
        "CAP_SYS_RAWIO" => Some(17),
        "CAP_SYS_CHROOT" => Some(18),
        "CAP_SYS_PTRACE" => Some(19),
        "CAP_SYS_PACCT" => Some(20),
        "CAP_SYS_ADMIN" => Some(21),
        "CAP_SYS_BOOT" => Some(22),
        "CAP_SYS_NICE" => Some(23),
        "CAP_SYS_RESOURCE" => Some(24),
        "CAP_SYS_TIME" => Some(25),
        "CAP_SYS_TTY_CONFIG" => Some(26),
        "CAP_MKNOD" => Some(27),
        "CAP_LEASE" => Some(28),
        "CAP_AUDIT_WRITE" => Some(29),
        "CAP_AUDIT_CONTROL" => Some(30),
        "CAP_SETFCAP" => Some(31),
        "CAP_MAC_OVERRIDE" => Some(32),
        "CAP_MAC_ADMIN" => Some(33),
        "CAP_SYSLOG" => Some(34),
        "CAP_WAKE_ALARM" => Some(35),
        "CAP_BLOCK_SUSPEND" => Some(36),
        "CAP_AUDIT_READ" => Some(37),
        "CAP_PERFMON" => Some(38),
        "CAP_BPF" => Some(39),
        "CAP_CHECKPOINT_RESTORE" => Some(40),
        _ => None,
    }
}

/// Resolve the list of ambient capability names to their numeric values,
/// filtering out `~`-prefixed (deny-list) entries and unknown names.
fn resolve_ambient_caps(names: &[String]) -> Vec<u64> {
    let mut caps = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() || name.starts_with('~') {
            continue;
        }
        if let Some(cap) = cap_name_to_number(name) {
            caps.push(cap);
        } else {
            log::warn!("Unknown ambient capability: {name}");
        }
    }
    caps
}

// ── PAMName= session support ─────────────────────────────────────────────
//
// When a unit sets PAMName=, systemd runs the named PAM stack while spawning
// the service: the account and session phases run as root, before the UID
// drop. This is how pam_systemd's `default-capability-ambient-set=` reaches a
// service — pam_sm_open_session() raises the requested capabilities in this
// process's ambient set, and we read them back afterwards and fold them into
// the ambient set that is re-applied after the UID change (mirroring upstream
// exec-invoke.c setup_pam() + `capability_ambient_set |= ambient_after_pam`).
//
// libpam is loaded at runtime via dlopen so the manager keeps its otherwise
// native-dependency-free build. The absolute library path is baked in at
// compile time from the Nix build (PAM_LIB env), falling back to the standard
// soname when built outside Nix.
const PAM_LIB_PATH: &str = match option_env!("PAM_LIB") {
    Some(p) => p,
    None => "libpam.so.0",
};

// From <security/_pam_types.h>.
const PAM_SUCCESS: libc::c_int = 0;
const PAM_SILENT: libc::c_int = 0x8000;
const PAM_ESTABLISH_CRED: libc::c_int = 0x0002;

// prctl(PR_CAP_AMBIENT, ...) sub-command. Defined locally so the build does
// not depend on the libc crate exposing this particular constant.
const PR_CAP_AMBIENT_IS_SET: libc::c_int = 1;

#[repr(C)]
struct PamMessage {
    msg_style: libc::c_int,
    msg: *const libc::c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut libc::c_char,
    resp_retcode: libc::c_int,
}

#[repr(C)]
struct PamConv {
    conv: Option<
        unsafe extern "C" fn(
            libc::c_int,
            *mut *const PamMessage,
            *mut *mut PamResponse,
            *mut libc::c_void,
        ) -> libc::c_int,
    >,
    appdata_ptr: *mut libc::c_void,
}

/// Minimal PAM conversation callback. Services never authenticate
/// interactively, so we allocate an empty (zeroed) response array for any
/// prompts and ignore informational/error messages. PAM frees the array and
/// each response string with free(3), so it must come from the C allocator.
unsafe extern "C" fn pam_noop_conv(
    num_msg: libc::c_int,
    _msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    _appdata: *mut libc::c_void,
) -> libc::c_int {
    if num_msg <= 0 || resp.is_null() {
        return PAM_SUCCESS;
    }
    let arr =
        unsafe { libc::calloc(num_msg as libc::size_t, std::mem::size_of::<PamResponse>()) }
            as *mut PamResponse;
    if arr.is_null() {
        return 5; // PAM_BUF_ERR
    }
    unsafe { *resp = arr };
    PAM_SUCCESS
}

type PamStartFn = unsafe extern "C" fn(
    *const libc::c_char,
    *const libc::c_char,
    *const PamConv,
    *mut *mut libc::c_void,
) -> libc::c_int;
type PamFlagsFn = unsafe extern "C" fn(*mut libc::c_void, libc::c_int) -> libc::c_int;
type PamEndFn = unsafe extern "C" fn(*mut libc::c_void, libc::c_int) -> libc::c_int;
type PamPutenvFn = unsafe extern "C" fn(*mut libc::c_void, *const libc::c_char) -> libc::c_int;

/// Run the account + session phases of the named PAM stack for `user`, as the
/// current (root) process. On success the process's ambient capability set
/// reflects any `default-capability-ambient-set=` from pam_systemd; the caller
/// reads it back with [`read_ambient_caps`].
///
/// Every failure is returned as `Err` so the caller can warn and continue
/// without the session — running a service without its PAM stack matches the
/// pre-existing behaviour (PAMName= used to be ignored), so this can never
/// regress a service that previously started.
fn run_pam_session(service: &str, user: &str, env: &[(String, String)]) -> Result<(), String> {
    use std::ffi::CString;

    let c_service =
        CString::new(service).map_err(|_| "PAMName contains a NUL byte".to_string())?;
    let c_user = CString::new(user).map_err(|_| "user name contains a NUL byte".to_string())?;
    let c_lib =
        CString::new(PAM_LIB_PATH).map_err(|_| "PAM_LIB path contains a NUL byte".to_string())?;

    // RTLD_GLOBAL so the PAM modules libpam dlopens can resolve its symbols.
    let handle = unsafe { libc::dlopen(c_lib.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        return Err(format!("dlopen({PAM_LIB_PATH}) failed"));
    }

    macro_rules! sym {
        ($name:literal, $ty:ty) => {{
            let s = unsafe { libc::dlsym(handle, concat!($name, "\0").as_ptr() as *const libc::c_char) };
            if s.is_null() {
                return Err(format!("dlsym({}) failed", $name));
            }
            unsafe { std::mem::transmute::<*mut libc::c_void, $ty>(s) }
        }};
    }

    let pam_start = sym!("pam_start", PamStartFn);
    let pam_acct_mgmt = sym!("pam_acct_mgmt", PamFlagsFn);
    let pam_setcred = sym!("pam_setcred", PamFlagsFn);
    let pam_open_session = sym!("pam_open_session", PamFlagsFn);
    let pam_putenv = sym!("pam_putenv", PamPutenvFn);
    let pam_end = sym!("pam_end", PamEndFn);

    let conv = PamConv {
        conv: Some(pam_noop_conv),
        appdata_ptr: std::ptr::null_mut(),
    };
    let mut pamh: *mut libc::c_void = std::ptr::null_mut();

    let r = unsafe { pam_start(c_service.as_ptr(), c_user.as_ptr(), &conv, &mut pamh) };
    if r != PAM_SUCCESS || pamh.is_null() {
        return Err(format!("pam_start failed ({r})"));
    }

    // Account and credential phases are best-effort: warn but keep going so a
    // module quirk can't block the session (and the ambient caps) we need.
    let r = unsafe { pam_acct_mgmt(pamh, PAM_SILENT) };
    if r != PAM_SUCCESS {
        log::warn!("pam_acct_mgmt for service '{service}' returned {r}, continuing");
    }
    // Publish the service's Environment= into the PAM handle BEFORE the session
    // stack runs, matching upstream's order in setup_pam()
    // (src/core/exec-invoke.c: pam_putenv, then pam_setcred(PAM_ESTABLISH_CRED),
    // then pam_open_session).
    //
    // This is what lets a session module read settings from the unit. pam_systemd
    // takes the session CLASS from XDG_SESSION_CLASS here; without it every
    // PAMName= service was announced to logind with pam_systemd's fallback class
    // regardless of what the unit asked for. The process environment is no help:
    // config.env is not applied to it until much later, after the UID drop.
    for (k, v) in env {
        let Ok(nv) = CString::new(format!("{k}={v}")) else {
            log::warn!("PAM environment for '{service}': skipping {k} (NUL byte)");
            continue;
        };
        let r = unsafe { pam_putenv(pamh, nv.as_ptr()) };
        if r != PAM_SUCCESS {
            log::warn!("pam_putenv({k}) for service '{service}' returned {r}, continuing");
        }
    }

    let r = unsafe { pam_setcred(pamh, PAM_ESTABLISH_CRED | PAM_SILENT) };
    if r != PAM_SUCCESS {
        log::warn!("pam_setcred for service '{service}' returned {r}, continuing");
    }

    let r = unsafe { pam_open_session(pamh, PAM_SILENT) };
    if r != PAM_SUCCESS {
        // Tear the handle down (without closing a session that never opened).
        unsafe { pam_end(pamh, r) };
        return Err(format!("pam_open_session failed ({r})"));
    }

    // The session is intentionally left open: pam_close_session must run in the
    // same process, but this process is about to execve() the service. A
    // faithful implementation would fork a holder that closes the session when
    // the service exits; for now the session is reaped by logind's GC. We keep
    // the PAM handle alive (no pam_end) so its session data survives the exec.
    Ok(())
}

/// Read the set of capabilities currently raised in this process's ambient
/// set, via prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_IS_SET, ...). Used to fold
/// PAM-provided ambient caps into the set re-applied after the UID drop.
fn read_ambient_caps() -> Vec<u64> {
    let mut caps = Vec::new();
    for cap in 0u64..=63 {
        let r = unsafe {
            libc::prctl(
                libc::PR_CAP_AMBIENT,
                PR_CAP_AMBIENT_IS_SET,
                cap as libc::c_ulong,
                0,
                0,
            )
        };
        if r == 1 {
            caps.push(cap);
        }
    }
    caps
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ExecHelperConfig {
    pub name: String,

    /// Log level passed from the service manager, matching real systemd's
    /// `--log-level` argument to `sd-executor`.  When set, this is used as
    /// the default log level for the [`crate::kmsg_log::KmsgLogger`] in
    /// this exec-helper child process.  The `SYSTEMD_LOG_LEVEL` environment
    /// variable (if present) takes final precedence, just as in real systemd
    /// where `log_parse_environment()` runs after the CLI arg is applied.
    ///
    /// Expected values: `"error"`, `"warn"`, `"info"`, `"debug"`, `"trace"`,
    /// or syslog numeric strings `"0"`–`"7"`.  `None` means the manager did
    /// not specify a level (defaults to `Warn`).
    #[serde(default)]
    pub log_level: Option<String>,

    pub cmd: PathBuf,
    pub args: Vec<String>,
    /// When true, args[0] is used as argv[0] instead of the filename of cmd.
    /// This corresponds to the '@' prefix in systemd command lines.
    #[serde(default)]
    pub use_first_arg_as_argv0: bool,

    /// When true, the command has the '+' prefix: run with full privileges,
    /// skipping all namespace/sandbox/security restrictions and privilege
    /// drop. The '!' and '!!' prefixes also set this (they affect
    /// NoNewPrivileges handling for SUID/file-capability binaries; we treat
    /// them as equivalent to '+' for now).
    #[serde(default)]
    pub privileged_prefix: bool,

    /// When true, the command has the ':' prefix: environment variable
    /// SUBSTITUTION is not applied to the command line.
    ///
    /// systemd.service(5) says `:` means only that substitution is skipped; it
    /// says nothing about which variables the process receives. This was
    /// previously implemented as "use a clean environment", which also
    /// discarded the service's own Environment=/EnvironmentFile=.
    #[serde(default)]
    pub no_env_expand: bool,

    /// When true, the command has the '|' prefix: run the command via the
    /// user's login shell. The original command and arguments are passed
    /// as `shell -el -c "cmd args..."`. The shell is looked up from the
    /// effective user's passwd entry (after User= is applied).
    #[serde(default)]
    pub login_shell: bool,

    pub env: Vec<(String, String)>,

    pub group: libc::gid_t,
    pub supplementary_groups: Vec<libc::gid_t>,
    pub user: libc::uid_t,

    /// When set, the exec helper exits immediately with this error code.
    /// Used for Type=simple services where user/group resolution fails:
    /// the parent returns success (matching real systemd) while the child
    /// reports the error.
    #[serde(default)]
    pub deferred_exec_error: Option<i32>,

    pub working_directory: Option<PathBuf>,

    /// RootDirectory= — sets the root directory for the executed process.
    /// After setting up the mount namespace, chroot() is called to this path.
    /// See systemd.exec(5).
    #[serde(default)]
    pub root_directory: Option<String>,

    pub state_directory: Vec<String>,
    pub logs_directory: Vec<String>,
    pub logs_directory_mode: Option<u32>,
    pub runtime_directory: Vec<String>,

    /// OOMScoreAdjust= — sets the OOM score adjustment for executed processes.
    /// Written to /proc/self/oom_score_adj before exec. Range: -1000 to 1000.
    pub oom_score_adjust: Option<i32>,

    pub platform_specific: PlatformSpecificServiceFields,

    pub limit_nofile: Option<ResourceLimit>,

    /// How stdin should be set up for the service process.
    #[serde(default)]
    pub stdin_option: StandardInput,
    /// StandardInputText=/StandardInputData= payload: the accumulated bytes to feed
    /// the service on stdin (via an in-memory file) when StandardInput= is left at
    /// its default. None when neither directive is set.
    #[serde(default)]
    pub stdin_data: Option<Vec<u8>>,
    /// Path to the TTY device to use when StandardInput=tty/tty-force/tty-fail.
    /// Defaults to /dev/console if not set.
    pub tty_path: Option<PathBuf>,

    /// TTYColumns= / TTYRows=: terminal window size applied via TIOCSWINSZ when
    /// the service connects to a TTY. None = unset (leave the current size).
    #[serde(default)]
    pub tty_columns: Option<u16>,
    #[serde(default)]
    pub tty_rows: Option<u16>,

    /// TTYReset= — reset the TTY to sane defaults before use.
    /// Matches systemd: resets termios, keyboard mode, switches to text mode.
    #[serde(default)]
    pub tty_reset: bool,
    /// TTYVHangup= — send TIOCVHANGUP to the TTY before use.
    /// Disconnects prior sessions so the new service gets a clean terminal.
    #[serde(default)]
    pub tty_vhangup: bool,
    /// TTYVTDisallocate= — deallocate or clear the VT before use.
    #[serde(default)]
    pub tty_vt_disallocate: bool,

    /// IgnoreSIGPIPE= — if true (the default), SIGPIPE is set to SIG_IGN before
    /// exec'ing the service binary. When false, the default SIGPIPE disposition
    /// (terminate) is left in place. Matches systemd.exec(5).
    #[serde(default = "default_true")]
    pub ignore_sigpipe: bool,

    /// UtmpIdentifier= — the identifier string for utmp/wtmp records (up to 4
    /// characters). When set together with a TTY, a utmp/wtmp login record is
    /// written before exec and a dead record on service exit.
    #[serde(default)]
    pub utmp_identifier: Option<String>,

    /// UtmpMode= — the type of utmp/wtmp record to create (init/login/user).
    #[serde(default)]
    pub utmp_mode: UtmpMode,

    /// ImportCredential= — glob patterns for credentials to import from the
    /// system credential store into the service's credential directory.
    /// The `CREDENTIALS_DIRECTORY` env var is set to the created directory.
    #[serde(default)]
    pub import_credentials: Vec<String>,

    /// LoadCredential=ID:PATH — load a credential from a file at PATH and
    /// place it in the service's credential directory under the name ID.
    #[serde(default)]
    pub load_credentials: Vec<(String, String)>,

    /// LoadCredentialEncrypted=ID:PATH — like LoadCredential= but the file
    /// content is expected to be encrypted. Parsed but decryption is not yet
    /// implemented (credential is loaded as-is).
    #[serde(default)]
    pub load_credentials_encrypted: Vec<(String, String)>,

    /// SetCredential=ID:DATA — set a credential from inline data. The data
    /// is written verbatim to the credential directory under the given ID.
    #[serde(default)]
    pub set_credentials: Vec<(String, String)>,

    /// SetCredentialEncrypted=ID:DATA — like SetCredential= but the inline
    /// data is expected to be encrypted (base64-encoded). Parsed but
    /// decryption is not yet implemented (credential is written as-is).
    #[serde(default)]
    pub set_credentials_encrypted: Vec<(String, String)>,

    /// Whether StandardOutput is set to inherit (or journal/kmsg/tty/unset).
    /// When true AND stdin is a TTY, stdout will be dup'd from the TTY fd.
    #[serde(default = "default_true")]
    pub stdout_is_inherit: bool,
    /// Whether StandardError is set to inherit (or journal/kmsg/tty/unset).
    /// When true AND stdin is a TTY, stderr will be dup'd from the TTY fd.
    #[serde(default = "default_true")]
    pub stderr_is_inherit: bool,
    /// Whether StandardOutput is set to socket (for Accept=yes services).
    /// When true, stdout (fd 1) is dup'd from the first LISTEN_FD (fd 3).
    #[serde(default)]
    pub stdout_is_socket: bool,
    /// Whether StandardError is set to socket (for Accept=yes services).
    /// When true, stderr (fd 2) is dup'd from the first LISTEN_FD (fd 3).
    #[serde(default)]
    pub stderr_is_socket: bool,
    /// Whether StandardOutput should be connected to the journal stream socket.
    #[serde(default)]
    pub stdout_is_journal: bool,
    /// Whether StandardError should be connected to the journal stream socket.
    #[serde(default)]
    pub stderr_is_journal: bool,
    /// Whether StandardOutput is explicitly set to tty.
    /// When true AND stdin is NOT a TTY, the TTY is opened independently for stdout.
    #[serde(default)]
    pub stdout_is_tty: bool,
    /// Whether StandardError is explicitly set to tty.
    /// When true AND stdin is NOT a TTY, the TTY is opened independently for stderr.
    #[serde(default)]
    pub stderr_is_tty: bool,

    /// StandardOutput=file:/path or append:/path or truncate:/path.
    /// When set, stdout is redirected to this file.
    #[serde(default)]
    pub stdout_file_path: Option<String>,
    /// Whether stdout file should be opened in append mode.
    #[serde(default)]
    pub stdout_file_append: bool,

    /// StandardError=file:/path or append:/path or truncate:/path.
    /// When set, stderr is redirected to this file.
    #[serde(default)]
    pub stderr_file_path: Option<String>,
    /// Whether stderr file should be opened in append mode.
    #[serde(default)]
    pub stderr_file_append: bool,

    /// AmbientCapabilities= — Linux capability names (e.g. CAP_SYS_TIME) to
    /// raise as ambient capabilities after dropping privileges.  Ambient
    /// capabilities survive execve() even without file capabilities, so the
    /// unprivileged service process retains them.
    #[serde(default)]
    pub ambient_capabilities: Vec<String>,

    // ── Security & sandboxing directives ───────────────────────────────
    /// DynamicUser= — when true, directories (Runtime/State/Cache/Logs)
    /// are created under a `private/` subdirectory with symlinks from the
    /// standard paths. E.g. `/run/private/<name>` with symlink
    /// `/run/<name>` → `/run/private/<name>`.
    #[serde(default)]
    pub dynamic_user: bool,

    /// NoNewPrivileges= — if true, ensures that the service process and all
    /// its children can never gain new privileges through execve() (e.g.
    /// via setuid/setgid bits or file capabilities). Applied via
    /// prctl(PR_SET_NO_NEW_PRIVS). See systemd.exec(5).
    #[serde(default)]
    pub no_new_privileges: bool,

    /// UMask= — file mode creation mask. Applied via libc::umask() before
    /// exec. Takes an octal value (e.g. 0022, 0077). See systemd.exec(5).
    #[serde(default)]
    pub umask: Option<u32>,

    /// Nice= — scheduling priority. Applied via setpriority(PRIO_PROCESS).
    /// Range: -20 (highest priority) to 19 (lowest). See systemd.exec(5).
    #[serde(default)]
    pub nice: Option<i32>,

    /// CPUSchedulingPolicy= — CPU scheduling policy. Applied via
    /// sched_setscheduler(). Values: "other", "batch", "idle", "fifo", "rr".
    /// See systemd.exec(5).
    #[serde(default)]
    pub cpu_scheduling_policy: Option<String>,

    /// CPUSchedulingPriority= — CPU scheduling priority (1-99 for
    /// real-time policies). Applied via sched_setscheduler().
    /// See systemd.exec(5).
    #[serde(default)]
    pub cpu_scheduling_priority: Option<u32>,

    /// CPUSchedulingResetOnFork= — if true, the scheduling policy is
    /// reset to SCHED_OTHER on fork(). See systemd.exec(5).
    #[serde(default)]
    pub cpu_scheduling_reset_on_fork: Option<bool>,

    /// IOSchedulingClass= — I/O scheduling class. Applied via ioprio_set().
    /// 0=none, 1=realtime, 2=best-effort, 3=idle. See systemd.exec(5).
    #[serde(default)]
    pub io_scheduling_class: u8,

    /// IOSchedulingPriority= — I/O scheduling priority (0-7).
    /// Applied via ioprio_set(). See systemd.exec(5).
    #[serde(default)]
    pub io_scheduling_priority: Option<u8>,

    /// CapabilityBoundingSet= — capability names to keep (allow-list) or
    /// drop (deny-list, prefixed with ~). Applied via prctl(PR_CAPBSET_DROP).
    /// See systemd.exec(5).
    #[serde(default)]
    pub capability_bounding_set: Vec<String>,

    /// ProtectSystem= — mount namespace isolation for OS directories.
    /// "no" = disabled, "yes" = /usr and /boot read-only,
    /// "full" = /usr, /boot, /etc read-only,
    /// "strict" = entire / read-only (except API fs and explicit RW paths).
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_system: String,

    /// MemoryTHP= mode ("inherit"/"disable"/"madvise"/"system"), applied via
    /// prctl(PR_SET_THP_DISABLE). See systemd.exec(5).
    #[serde(default)]
    pub memory_thp: String,

    /// ProtectHome= — mount namespace isolation for home directories.
    /// "no" = disabled, "yes" = /home, /root, /run/user inaccessible,
    /// "read-only" = read-only, "tmpfs" = empty tmpfs. See systemd.exec(5).
    #[serde(default)]
    pub protect_home: String,

    /// PrivateTmp= — if true, /tmp and /var/tmp get private tmpfs mounts.
    /// See systemd.exec(5).
    #[serde(default)]
    pub private_tmp: bool,

    /// PrivateDevices= — if true, /dev is replaced with a minimal private
    /// instance containing only pseudo-devices. See systemd.exec(5).
    #[serde(default)]
    pub private_devices: bool,

    /// PrivateNetwork= — if true, a new network namespace with only lo is
    /// created. See systemd.exec(5).
    #[serde(default)]
    pub private_network: bool,

    /// PrivateUsers= — if true, a new user namespace with minimal mapping
    /// is created. See systemd.exec(5).
    #[serde(default)]
    pub private_users: bool,

    /// PrivateUsersEx= mode ("self"/"identity"/"full"; empty = yes-style).
    #[serde(default)]
    pub private_users_mode: String,

    /// PrivateMounts= — if true, a private mount namespace is created.
    /// See systemd.exec(5).
    #[serde(default)]
    pub private_mounts: bool,

    /// JoinsNamespaceOf= — PID of a running service whose mount namespace
    /// this service should join via setns(2) instead of creating its own.
    /// When set, setup_mount_namespace() is skipped entirely because the
    /// target's namespace already has the required isolation applied.
    #[serde(default)]
    pub join_namespace_pid: Option<u32>,

    /// MountFlags= — mount propagation flags for the mount namespace.
    /// "shared" = MS_SHARED, "slave" = MS_SLAVE (default), "private" = MS_PRIVATE.
    /// See systemd.exec(5).
    #[serde(default)]
    pub mount_flags: Option<String>,

    /// PrivateIPC= — if true, a new IPC namespace is created, isolating
    /// System V IPC objects and POSIX message queues. See systemd.exec(5).
    #[serde(default)]
    pub private_ipc: bool,

    /// NetworkNamespacePath= — path to an existing network namespace to join
    /// (e.g. /run/netns/foo). Mutually exclusive with PrivateNetwork=.
    #[serde(default)]
    pub network_namespace_path: Option<String>,

    /// UserNamespacePath= — path to an existing user namespace to join
    /// (e.g. /proc/PID/ns/user). Joins that namespace via setns instead of
    /// creating a fresh one with PrivateUsers=.
    #[serde(default)]
    pub user_namespace_path: Option<String>,

    /// PAMName= — name of a PAM service stack to run when spawning the
    /// service. The account and session phases are run as root before the
    /// UID drop; pam_systemd's `default-capability-ambient-set=` reaches the
    /// service this way (its ambient caps are folded into the set re-applied
    /// after the UID change). See systemd.exec(5).
    #[serde(default)]
    pub pam_name: Option<String>,

    /// IPCNamespacePath= — path to an existing IPC namespace to join.
    /// Mutually exclusive with PrivateIPC=.
    #[serde(default)]
    pub ipc_namespace_path: Option<String>,

    /// TimerSlackNSec= — timer slack value in nanoseconds for the process.
    /// Applied via prctl(PR_SET_TIMERSLACK). See systemd.exec(5).
    #[serde(default)]
    pub timer_slack_nsec: Option<u64>,

    /// CoredumpFilter= — bitmask written to /proc/self/coredump_filter.
    /// Controls which memory mappings are written to core dumps.
    #[serde(default)]
    pub coredump_filter: Option<String>,

    /// CPUAffinity= — list of CPU indices/ranges to pin the process to.
    /// Applied via sched_setaffinity(). See systemd.exec(5).
    #[serde(default)]
    pub cpu_affinity: Vec<String>,

    /// NUMAPolicy= — NUMA memory policy name (default/preferred/bind/interleave/
    /// local), applied via set_mempolicy() before exec.
    #[serde(default)]
    pub numa_policy: Option<String>,

    /// NUMAMask= — NUMA node mask for NUMAPolicy=, applied with the policy.
    #[serde(default)]
    pub numa_mask: Option<String>,

    /// PrivatePIDs= — if true, a new PID namespace is created and /proc is
    /// remounted so the service process becomes PID 1 in the new namespace.
    /// See systemd.exec(5).
    #[serde(default)]
    pub private_pids: bool,

    /// ProtectKernelTunables= — if true, /proc/sys and similar are read-only.
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_kernel_tunables: bool,

    /// ProtectKernelModules= — if true, module loading is denied and
    /// /usr/lib/modules is inaccessible. See systemd.exec(5).
    #[serde(default)]
    pub protect_kernel_modules: bool,

    /// ProtectKernelLogs= — if true, /dev/kmsg and /proc/kmsg are
    /// inaccessible. See systemd.exec(5).
    #[serde(default)]
    pub protect_kernel_logs: bool,

    /// ProtectControlGroups= — if true, /sys/fs/cgroup is read-only.
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_control_groups: bool,

    /// ProtectControlGroupsEx= — granular cgroup access control.
    /// Values: "no", "yes" (read-only), "private" (new ns, rw), "strict" (new ns, ro).
    #[serde(default)]
    pub protect_control_groups_ex: String,

    /// ProtectClock= — if true, clock writes are denied.
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_clock: bool,

    /// ProtectHostname= — if true, a new UTS namespace is created.
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_hostname: bool,

    /// ProtectHostname= mode — "yes" (read-only, prevents sethostname) or
    /// "private" (new UTS namespace, allows sethostname within it).
    #[serde(default)]
    pub protect_hostname_mode: Option<String>,

    /// ProtectHostname= hostname — optional hostname to set in the new UTS
    /// namespace when ProtectHostname=yes:hostname or private:hostname.
    #[serde(default)]
    pub protect_hostname_name: Option<String>,

    /// Personality= — set the execution domain (personality).
    /// See systemd.exec(5).
    #[serde(default)]
    pub personality: Option<String>,

    /// LockPersonality= — if true, the execution domain is locked.
    /// See systemd.exec(5).
    #[serde(default)]
    pub lock_personality: bool,

    /// KeyringMode= — controls the kernel session keyring setup.
    /// "inherit" = no change, "private" = new anonymous session keyring,
    /// "shared" = new session keyring linked to user keyring.
    /// See systemd.exec(5).
    #[serde(default)]
    pub keyring_mode: Option<String>,

    /// SecureBits= — controls the secure-bits flags of the executed process.
    /// Space-separated list of: keep-caps, keep-caps-locked, no-setuid-fixup,
    /// no-setuid-fixup-locked, noroot, noroot-locked. Applied via
    /// prctl(PR_SET_SECUREBITS). See systemd.exec(5).
    #[serde(default)]
    pub secure_bits: Vec<String>,

    /// MemoryDenyWriteExecute= — if true, W+X memory mappings are denied.
    /// See systemd.exec(5).
    #[serde(default)]
    pub memory_deny_write_execute: bool,

    /// RestrictRealtime= — if true, realtime scheduling is denied.
    /// See systemd.exec(5).
    #[serde(default)]
    pub restrict_realtime: bool,

    /// RestrictSUIDSGID= — if true, setuid/setgid bits on files are denied.
    /// See systemd.exec(5).
    #[serde(default)]
    pub restrict_suid_sgid: bool,

    /// ReadWritePaths= — paths to bind-mount read-write even under
    /// ProtectSystem=strict. See systemd.exec(5).
    #[serde(default)]
    pub read_write_paths: Vec<String>,

    /// The service's cgroup `memory.pressure` path (from `MemoryPressureWatch=`),
    /// bind-mounted read-write after `ProtectControlGroups=` makes
    /// `/sys/fs/cgroup` read-only, so the service can still register PSI
    /// triggers. Matches systemd adding the pressure path to `ReadWritePaths=`.
    #[serde(default)]
    pub memory_pressure_path: Option<String>,

    /// RestrictNamespaces= — namespace restriction.
    /// "yes" = all denied, "no" = all allowed, or space-separated list.
    /// See systemd.exec(5).
    #[serde(default)]
    pub restrict_namespaces: String,

    /// SystemCallArchitectures= — allowed syscall architectures.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_architectures: Vec<String>,

    /// RestrictAddressFamilies=: address families the service may use. An
    /// allow-list (e.g. "AF_UNIX AF_INET") permits only those; a deny-list
    /// ("~AF_PACKET") blocks the listed ones. Enforced via a seccomp filter on
    /// socket(2). See systemd.exec(5).
    #[serde(default)]
    pub restrict_address_families: Vec<String>,

    /// SystemCallFilter= — seccomp syscall filter.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_filter: Vec<String>,

    /// SystemCallLog= — seccomp syscall audit logging.
    /// Same syntax as SystemCallFilter= but logs rather than blocks.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_log: Vec<String>,

    /// SystemCallErrorNumber= sets the errno (a name like "EPERM" or a number)
    /// returned to a filtered syscall instead of the default action of killing
    /// the process. See systemd.exec(5).
    #[serde(default)]
    pub system_call_error_number: Option<String>,

    /// RestrictFileSystems= — filesystem type restriction.
    /// Space-separated list of filesystem type names (e.g. ext4, tmpfs).
    /// A leading `~` inverts the list (deny-list). See systemd.exec(5).
    #[serde(default)]
    pub restrict_file_systems: Vec<String>,

    /// ProtectProc= — hidepid= mount option for /proc.
    /// See systemd.exec(5).
    #[serde(default)]
    pub protect_proc: String,

    /// ProcSubset= — "all" or "pid" for /proc mount.
    /// See systemd.exec(5).
    #[serde(default)]
    pub proc_subset: String,

    // ── Resource limits (LimitXXX=) ──────────────────────────────────
    // All Limit* directives follow the same format: a numeric value, a
    // soft:hard pair, or "infinity". Applied via setrlimit() before exec.
    #[serde(default)]
    pub limit_cpu: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_core: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_fsize: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_data: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_stack: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_rss: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_nproc: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_memlock: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_as: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_locks: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_sigpending: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_msgqueue: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_nice: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_rtprio: Option<ResourceLimit>,
    #[serde(default)]
    pub limit_rttime: Option<ResourceLimit>,

    // ── Directory management ─────────────────────────────────────────
    /// CacheDirectory= — directories to create under /var/cache/.
    #[serde(default)]
    pub cache_directory: Vec<String>,
    /// CacheDirectoryMode= — octal mode for cache directories.
    #[serde(default)]
    pub cache_directory_mode: Option<u32>,
    /// ConfigurationDirectory= — directories to create under /etc/.
    #[serde(default)]
    pub configuration_directory: Vec<String>,
    /// ConfigurationDirectoryMode= — octal mode for configuration directories.
    #[serde(default)]
    pub configuration_directory_mode: Option<u32>,
    /// StateDirectoryMode= — octal mode for state directories.
    #[serde(default)]
    pub state_directory_mode: Option<u32>,
    /// RuntimeDirectoryMode= — octal mode for runtime directories.
    #[serde(default)]
    pub runtime_directory_mode: Option<u32>,

    // ── Path-based mount namespace directives ────────────────────────
    /// ReadOnlyPaths= — paths to make read-only in the mount namespace.
    #[serde(default)]
    pub read_only_paths: Vec<String>,
    /// InaccessiblePaths= — paths to make inaccessible in the mount namespace.
    #[serde(default)]
    pub inaccessible_paths: Vec<String>,
    /// BindPaths= — paths to bind-mount read-write into the mount namespace.
    #[serde(default)]
    pub bind_paths: Vec<String>,
    /// BindReadOnlyPaths= — paths to bind-mount read-only into the mount namespace.
    #[serde(default)]
    pub bind_read_only_paths: Vec<String>,
    /// TemporaryFileSystem= — paths to mount tmpfs on in the mount namespace.
    #[serde(default)]
    pub temporary_file_system: Vec<String>,

    // ── Logging directives ───────────────────────────────────────────
    /// SyslogIdentifier= — the process name ("tag") to prefix log messages with.
    #[serde(default)]
    pub syslog_identifier: Option<String>,
    /// SyslogLevel= — the default syslog priority for stdout messages.
    /// Stored as syslog name (e.g. "notice", "info") or numeric string.
    #[serde(default)]
    pub syslog_level: Option<String>,
    /// SyslogLevelPrefix= — if true (default), strip kernel-style `<N>` priority prefixes.
    #[serde(default)]
    pub syslog_level_prefix: Option<bool>,
    /// `<base>/private` directories to replace with a tmpfs inside the mount
    /// namespace, so a `DynamicUser=` service can reach its own directory
    /// without the 0700 root:root boundary on the host being loosened.
    ///
    /// Computed locally, never on the wire.
    #[serde(skip)]
    pub private_dir_tmpfs: Vec<String>,
    /// Every exec directory as the service sees it, e.g. `/var/lib/zzz` and any
    /// alias like `/var/lib/xxx`.
    ///
    /// One source of truth for `ProtectSystem=strict`'s implicit
    /// `ReadWritePaths=`, which previously re-derived these with
    /// `format!("/var/lib/{dir_name}")` per directory type and so mishandled
    /// aliases (`zzz:xxx` became the nonexistent `/var/lib/zzz:xxx`) and
    /// `DynamicUser=`'s `private/` layout.  Computed locally, never on the wire.
    #[serde(skip)]
    pub exec_dir_paths: Vec<String>,
    /// Exec directories to bind back over a `TemporaryFileSystem=` tmpfs, as
    /// (host source, path the service must see it at).
    ///
    /// Computed inside `run_exec_helper` after the directories are created, so
    /// it is never serialized from PID 1; `skip` keeps it off the wire.
    #[serde(skip)]
    pub exec_dir_binds: Vec<(String, String)>,
    /// LogNamespace= — send stdout/stderr to the journal namespace instance
    /// listening on `/run/systemd/journal.<ns>/stdout` rather than the default
    /// `/run/systemd/journal/stdout`.
    #[serde(default)]
    pub log_namespace: Option<String>,
    /// The service's invocation ID (32-char hex UUID), sent to journald so it
    /// can tag entries with `_SYSTEMD_INVOCATION_ID`.
    #[serde(default)]
    pub invocation_id: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Apply a single resource limit via `setrlimit()`. If the limit is `None`,
/// this is a no-op. On failure, logs an error and exits (matching systemd's
/// behavior — resource limit failures are fatal for service startup).
fn apply_resource_limit(
    name: &str,
    resource: libc::__rlimit_resource_t,
    limit: &Option<ResourceLimit>,
) {
    let limit = match limit {
        Some(l) => l,
        None => return,
    };
    let soft = match limit.soft {
        RLimitValue::Value(v) => v as libc::rlim_t,
        RLimitValue::Infinity => libc::RLIM_INFINITY,
    };
    let hard = match limit.hard {
        RLimitValue::Value(v) => v as libc::rlim_t,
        RLimitValue::Infinity => libc::RLIM_INFINITY,
    };
    let rlim = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    let ret = unsafe { libc::setrlimit(resource, &rlim) };
    if ret != 0 {
        log::warn!(
            "Failed to set {} (soft={}, hard={}): {}",
            name,
            soft,
            hard,
            std::io::Error::last_os_error()
        );
    }
}

fn prepare_exec_args(
    cmd_str: &Path,
    args_str: &[String],
    use_first_arg_as_argv0: bool,
) -> (std::ffi::CString, Vec<std::ffi::CString>) {
    let cmd = std::ffi::CString::new(cmd_str.to_string_lossy().as_bytes()).unwrap();

    let mut args = Vec::new();

    if use_first_arg_as_argv0 {
        // With '@' prefix: args[0] becomes argv[0], remaining args follow
        for word in args_str {
            args.push(std::ffi::CString::new(word.as_str()).unwrap());
        }
    } else {
        // Normal case: filename of cmd becomes argv[0], then all args follow
        let exec_name = std::path::PathBuf::from(cmd_str);
        let exec_name = exec_name.file_name().unwrap();
        let exec_name: Vec<u8> = exec_name.to_str().unwrap().bytes().collect();
        let exec_name = std::ffi::CString::new(exec_name).unwrap();

        args.push(exec_name);

        for word in args_str {
            args.push(std::ffi::CString::new(word.as_str()).unwrap());
        }
    }

    (cmd, args)
}

/// Look up the login shell for the given UID from /etc/passwd.
/// Falls back to "/bin/sh" if the lookup fails, the shell field is empty,
/// or the shell is a nologin/false stub that would refuse `-c` invocation.
fn get_login_shell(uid: libc::uid_t) -> String {
    let pwd = unsafe { libc::getpwuid(uid) };
    if !pwd.is_null() {
        let shell = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_shell) };
        let shell = shell.to_string_lossy().into_owned();
        if !shell.is_empty() && !shell_is_stub(&shell) {
            return shell;
        }
    }
    "/bin/sh".to_owned()
}

/// System users commonly have `/usr/sbin/nologin` or `/bin/false` as their
/// login shell to prevent interactive sessions. When a service uses the `|`
/// prefix to request a login shell invocation, these stubs would reject the
/// `-c` command and fail. Substitute `/bin/sh` instead, matching upstream
/// systemd's behavior.
fn shell_is_stub(shell: &str) -> bool {
    let name = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    matches!(name, "nologin" | "false")
}

/// Open a terminal device, retrying on EIO.
/// This matches systemd's open_terminal() which retries because a TTY in the
/// process of being closed may temporarily return EIO.
fn open_terminal(path: &std::ffi::CStr, flags: libc::c_int) -> libc::c_int {
    for attempt in 0..20u32 {
        let fd = unsafe { libc::open(path.as_ptr(), flags) };
        if fd >= 0 {
            return fd;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::EIO) {
            return -1;
        }
        // EIO — TTY is being closed, retry after 50ms (max ~1s total)
        if attempt >= 19 {
            return -1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    -1
}

/// Apply TTYColumns=/TTYRows= to the terminal at `fd` via TIOCSWINSZ, matching
/// systemd's terminal_set_size_fd(): a dimension left unset (None) keeps the
/// current value read via TIOCGWINSZ, and if both are unset this is a no-op. The
/// window size is a property of the TTY device, so setting it on any fd open to
/// the device (here the service's stdio TTY) takes effect for the service.
fn apply_tty_size(fd: libc::c_int, config: &ExecHelperConfig) {
    set_tty_winsize(fd, config.tty_columns, config.tty_rows);
}

/// Core of terminal_set_size_fd(): set the window size of the TTY at `fd`. An
/// unset (None) dimension keeps the current value read via TIOCGWINSZ; both
/// unset is a no-op.
fn set_tty_winsize(fd: libc::c_int, cols: Option<u16>, rows: Option<u16>) {
    if cols.is_none() && rows.is_none() {
        return;
    }
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } < 0 {
        log::debug!(
            "TIOCGWINSZ failed, not setting terminal size: {}",
            std::io::Error::last_os_error()
        );
        return;
    }
    if let Some(rows) = rows {
        ws.ws_row = rows;
    }
    if let Some(cols) = cols {
        ws.ws_col = cols;
    }
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &ws) } < 0 {
        log::debug!(
            "TIOCSWINSZ failed setting terminal size: {}",
            std::io::Error::last_os_error()
        );
    }
}

/// Perform a "destructive" TTY reset before the service uses it.
/// This matches systemd's exec_context_tty_reset(): it resets terminal settings,
/// hangs up prior sessions, and optionally disallocates the VT.
/// This is called BEFORE opening the TTY for stdin so the service gets a clean terminal.
fn tty_reset_destructive(config: &ExecHelperConfig) {
    let tty_path = match config.tty_path.as_deref() {
        Some(p) => p,
        None => std::path::Path::new("/dev/console"),
    };

    let tty_path_cstr = match std::ffi::CString::new(tty_path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Open the TTY non-blocking and without becoming controlling terminal
    let fd = open_terminal(
        &tty_path_cstr,
        libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK,
    );
    if fd < 0 {
        log::warn!(
            "Failed to open TTY {:?} for reset: {}",
            tty_path,
            std::io::Error::last_os_error()
        );
        return;
    }

    if config.tty_reset {
        // Reset terminal to sane defaults via termios
        // This matches systemd's terminal_reset_ioctl()
        unsafe {
            // Disable exclusive mode
            let _ = libc::ioctl(fd, libc::TIOCNXCL);

            // Switch to text mode (KD_TEXT = 0x00)
            let _ = libc::ioctl(
                fd, 0x4B3A_u64, /* KDSETMODE */
                0_i32,      /* KD_TEXT */
            );

            // Reset termios to sane defaults
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) == 0 {
                termios.c_iflag &= !(libc::IGNBRK
                    | libc::BRKINT
                    | libc::ISTRIP
                    | libc::INLCR
                    | libc::IGNCR
                    | libc::IUCLC);
                termios.c_iflag |= libc::ICRNL | libc::IMAXBEL | libc::IUTF8;
                termios.c_oflag |= libc::ONLCR | libc::OPOST;
                termios.c_cflag |= libc::CREAD;
                termios.c_lflag = libc::ISIG
                    | libc::ICANON
                    | libc::IEXTEN
                    | libc::ECHO
                    | libc::ECHOE
                    | libc::ECHOK
                    | libc::ECHOCTL
                    | libc::ECHOKE;

                termios.c_cc[libc::VINTR] = 3; // ^C
                termios.c_cc[libc::VQUIT] = 28; // ^\
                termios.c_cc[libc::VERASE] = 127;
                termios.c_cc[libc::VKILL] = 21; // ^U
                termios.c_cc[libc::VEOF] = 4; // ^D
                termios.c_cc[libc::VSTART] = 17; // ^Q
                termios.c_cc[libc::VSTOP] = 19; // ^S
                termios.c_cc[libc::VSUSP] = 26; // ^Z
                termios.c_cc[libc::VLNEXT] = 22; // ^V
                termios.c_cc[libc::VWERASE] = 23; // ^W
                termios.c_cc[libc::VREPRINT] = 18; // ^R
                termios.c_cc[libc::VEOL] = 0;
                termios.c_cc[libc::VEOL2] = 0;
                termios.c_cc[libc::VTIME] = 0;
                termios.c_cc[libc::VMIN] = 1;

                let _ = libc::tcsetattr(fd, libc::TCSANOW, &termios);
            }

            // Flush all pending I/O
            let _ = libc::tcflush(fd, libc::TCIOFLUSH);
        }
    }

    if config.tty_vhangup {
        // Send TIOCVHANGUP — this disconnects any previous sessions from the TTY.
        // This is critical: without it, switching to the VT may show a stale/dead session.
        unsafe {
            let ret = libc::ioctl(fd, libc::TIOCVHANGUP);
            if ret < 0 {
                log::warn!(
                    "TIOCVHANGUP failed on {:?}: {}",
                    tty_path,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // Close the fd used for reset — we'll re-open it for actual use.
    // After vhangup the fd is dead anyway.
    unsafe {
        libc::close(fd);
    }

    if config.tty_vt_disallocate {
        // Try to disallocate or at least clear the VT.
        // Extract VT number from path like /dev/tty9
        let tty_str = tty_path.to_string_lossy();
        let tty_name = tty_str.strip_prefix("/dev/").unwrap_or(&tty_str);
        if let Some(vt_num_str) = tty_name.strip_prefix("tty")
            && let Ok(vt_num) = vt_num_str.parse::<libc::c_int>()
            && vt_num > 0
        {
            // Try VT_DISALLOCATE via /dev/tty0
            let tty0 = std::ffi::CString::new("/dev/tty0").unwrap();
            let tty0_fd = open_terminal(
                &tty0,
                libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK,
            );
            if tty0_fd >= 0 {
                let ret = unsafe {
                    libc::ioctl(tty0_fd, 0x5608 /* VT_DISALLOCATE */, vt_num)
                };
                unsafe {
                    libc::close(tty0_fd);
                }
                if ret >= 0 {
                    return; // Successfully disallocated
                }
                // EBUSY means the VT is active — fall through to clear it
            }
        }

        // If we can't disallocate, at least clear the screen
        let clear_fd = open_terminal(
            &tty_path_cstr,
            libc::O_WRONLY | libc::O_NOCTTY | libc::O_CLOEXEC | libc::O_NONBLOCK,
        );
        if clear_fd >= 0 {
            let clear_seq = b"\x1b[r\x1b[H\x1b[3J\x1bc";
            unsafe {
                let _ = libc::write(clear_fd, clear_seq.as_ptr().cast(), clear_seq.len());
                libc::close(clear_fd);
            }
        }
    }
}

/// Set up stdin for the service based on the StandardInput= setting.
/// Called after reading the exec_helper config (which consumed the original stdin).
/// When StandardOutput=tty or StandardError=tty is set but StandardInput is NOT a TTY,
/// we need to independently open the TTY for output. This matches systemd's behavior
/// where `StandardOutput=tty` always connects stdout to the TTY regardless of stdin.
fn setup_tty_output(config: &ExecHelperConfig) {
    if !config.stdout_is_tty && !config.stderr_is_tty {
        return;
    }
    // Only needed when stdin is NOT a TTY (when stdin IS a TTY, setup_stdin
    // already dup2'd the TTY fd onto stdout/stderr via stdout_is_inherit).
    match config.stdin_option {
        StandardInput::Tty | StandardInput::TtyForce | StandardInput::TtyFail => return,
        StandardInput::Null | StandardInput::Socket | StandardInput::File(_) => {}
    }

    let tty_path = config
        .tty_path
        .as_deref()
        .unwrap_or(Path::new("/dev/console"));
    let tty_path_cstr = match std::ffi::CString::new(tty_path.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            log::warn!("Invalid TTYPath for output: {:?}", tty_path);
            return;
        }
    };

    let tty_fd = open_terminal(&tty_path_cstr, libc::O_WRONLY | libc::O_NOCTTY);
    if tty_fd < 0 {
        log::warn!(
            "Failed to open TTY {:?} for output: {}",
            tty_path,
            std::io::Error::last_os_error()
        );
        return;
    }

    apply_tty_size(tty_fd, config);

    if config.stdout_is_tty {
        unsafe {
            libc::dup2(tty_fd, libc::STDOUT_FILENO);
        }
    }
    if config.stderr_is_tty {
        unsafe {
            libc::dup2(tty_fd, libc::STDERR_FILENO);
        }
    }
    if tty_fd != libc::STDOUT_FILENO && tty_fd != libc::STDERR_FILENO {
        unsafe {
            libc::close(tty_fd);
        }
    }
}

/// Connect stdout/stderr directly to journald's stream socket.
fn setup_journal_stream_output(config: &ExecHelperConfig) {
    if !config.stdout_is_journal && !config.stderr_is_journal {
        return;
    }

    // LogNamespace=foo routes the stream to the `systemd-journald@foo` instance,
    // which listens on /run/systemd/journal.foo/stdout (journald derives the
    // same path from its namespace argument).  Without a namespace this is the
    // default instance's socket.
    let socket_path = match config.log_namespace.as_deref() {
        Some(ns) if !ns.is_empty() => format!("/run/systemd/journal.{ns}/stdout"),
        _ => "/run/systemd/journal/stdout".to_owned(),
    };
    let socket_path = socket_path.as_str();

    // Use SyslogIdentifier= if set, otherwise derive from the binary name
    // in the exec command path. This matches C systemd's behavior where
    // SYSLOG_IDENTIFIER is set to the process name, not the unit name.
    let cmd_basename = config
        .cmd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&config.name);
    let identifier = config.syslog_identifier.as_deref().unwrap_or(cmd_basename);

    // Parse SyslogLevel= to a numeric priority (default: 6 = info)
    let priority = parse_syslog_priority(config.syslog_level.as_deref());
    let level_prefix = config.syslog_level_prefix.unwrap_or(true);
    let inv_id = config.invocation_id.as_deref();

    if config.stdout_is_journal
        && let Some(fd) = open_journal_stream_nonblock(
            socket_path,
            identifier,
            &config.name,
            priority,
            level_prefix,
            inv_id,
        )
    {
        unsafe {
            libc::dup2(fd, libc::STDOUT_FILENO);
        }
        if config.stderr_is_journal {
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
            }
        }
        if fd != libc::STDOUT_FILENO && fd != libc::STDERR_FILENO {
            unsafe {
                libc::close(fd);
            }
        }
        set_journal_stream_env(libc::STDOUT_FILENO);
        return;
    }

    if config.stderr_is_journal
        && let Some(fd) = open_journal_stream_nonblock(
            socket_path,
            identifier,
            &config.name,
            priority,
            level_prefix,
            inv_id,
        )
    {
        unsafe {
            libc::dup2(fd, libc::STDERR_FILENO);
            if fd != libc::STDERR_FILENO {
                libc::close(fd);
            }
        }
        set_journal_stream_env(libc::STDERR_FILENO);
    }
}

/// Export `JOURNAL_STREAM=<dev>:<ino>` for the given stdio fd once it is
/// connected to journald's stream socket, mirroring systemd (exec-invoke.c):
/// a service reads it (via sd_journal_stream_fd) to detect that its stdout or
/// stderr already goes to the journal, so it can use the native protocol and
/// skip re-adding timestamps/levels. Runs post-fork in the child, so the
/// non-thread-safe `set_var` is safe here (same as LISTEN_PID/WATCHDOG_PID).
fn set_journal_stream_env(fd: libc::c_int) {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut st) } == 0 {
        unsafe {
            std::env::set_var("JOURNAL_STREAM", format!("{}:{}", st.st_dev, st.st_ino));
        }
    }
}

/// Non-blocking connect to journald's stdout stream socket.
/// Returns None if the socket doesn't exist or can't connect within 100ms.
/// Parse a SyslogLevel= value to a numeric syslog priority (0-7).
/// Defaults to 6 (info) if unset or unrecognized.
fn parse_syslog_priority(level: Option<&str>) -> u8 {
    match level {
        Some(s) => match s.to_lowercase().as_str() {
            "emerg" | "emergency" | "0" => 0,
            "alert" | "1" => 1,
            "crit" | "critical" | "2" => 2,
            "err" | "error" | "3" => 3,
            "warning" | "warn" | "4" => 4,
            "notice" | "5" => 5,
            "info" | "6" => 6,
            "debug" | "7" => 7,
            _ => 6,
        },
        None => 6,
    }
}

fn open_journal_stream_nonblock(
    socket_path: &str,
    identifier: &str,
    unit_name: &str,
    priority: u8,
    level_prefix: bool,
    invocation_id: Option<&str>,
) -> Option<i32> {
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return None;
        }

        // Build sockaddr_un
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
        let path_bytes = socket_path.as_bytes();
        if path_bytes.len() >= addr.sun_path.len() {
            libc::close(fd);
            return None;
        }
        std::ptr::copy_nonoverlapping(
            path_bytes.as_ptr(),
            addr.sun_path.as_mut_ptr() as *mut u8,
            path_bytes.len(),
        );

        // Enable SO_PASSCRED on the sender socket so the kernel attaches
        // per-write credentials (PID/UID/GID) to every message, avoiding a
        // race where the receiver hasn't set SO_PASSCRED yet at write time.
        let enabled: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            &enabled as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

        // Set non-blocking for connect
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);

        let ret = libc::connect(
            fd,
            &addr as *const libc::sockaddr_un as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        );

        if ret < 0 {
            let err = *libc::__errno_location();
            if err != libc::EINPROGRESS {
                libc::close(fd);
                return None;
            }
            // Wait for connect with 100ms timeout
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let poll_ret = libc::poll(&mut pfd, 1, 100);
            if poll_ret <= 0 || (pfd.revents & libc::POLLOUT) == 0 {
                libc::close(fd);
                return None;
            }
            // Check for connect error
            let mut err_val: libc::c_int = 0;
            let mut err_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                &mut err_val as *mut _ as *mut libc::c_void,
                &mut err_len,
            );
            if err_val != 0 {
                libc::close(fd);
                return None;
            }
        }

        // Restore blocking mode for the actual I/O
        libc::fcntl(fd, libc::F_SETFL, flags);

        // Clear CLOEXEC so the fd survives exec
        libc::fcntl(fd, libc::F_SETFD, 0);

        // Send the protocol header:
        // Lines 1-7: standard journal stdout stream protocol
        // Line 8 (extension): invocation ID for _SYSTEMD_INVOCATION_ID tagging
        let lp = if level_prefix { 1 } else { 0 };
        let mut header = format!("{identifier}\n{unit_name}\n{priority}\n{lp}\n0\n0\n0\n");
        if let Some(inv_id) = invocation_id {
            header.push_str(inv_id);
            header.push('\n');
        }
        let written = libc::write(fd, header.as_ptr() as *const libc::c_void, header.len());
        if written < 0 || written as usize != header.len() {
            libc::close(fd);
            return None;
        }

        Some(fd)
    }
}

fn setup_stdin(config: &ExecHelperConfig) {
    match config.stdin_option {
        StandardInput::Null => {
            // StandardInputText=/StandardInputData= (implicit StandardInput=data):
            // feed the accumulated bytes from an in-memory file. No O_CLOEXEC on any
            // fd here — STDIN_FILENO was closed just above, so an fd may land on 0
            // and must survive execve (else the service gets no stdin / EBADF).
            let mut handled = false;
            if let Some(ref data) = config.stdin_data {
                let mfd = unsafe { libc::memfd_create(c"stdin-data".as_ptr(), 0) };
                if mfd < 0 {
                    log::error!(
                        "StandardInputText/Data: memfd_create failed: {}",
                        std::io::Error::last_os_error()
                    );
                } else {
                    let mut off = 0usize;
                    while off < data.len() {
                        let n = unsafe {
                            libc::write(
                                mfd,
                                data[off..].as_ptr() as *const libc::c_void,
                                data.len() - off,
                            )
                        };
                        if n <= 0 {
                            break;
                        }
                        off += n as usize;
                    }
                    unsafe { libc::lseek(mfd, 0, libc::SEEK_SET) };
                    if mfd != libc::STDIN_FILENO {
                        unsafe {
                            libc::dup2(mfd, libc::STDIN_FILENO);
                            libc::close(mfd);
                        }
                    }
                    handled = true;
                }
            }
            if !handled {
                // Open /dev/null as stdin. No O_CLOEXEC: STDIN_FILENO was closed just
                // before setup_stdin, so open() returns fd 0 and the dup2 below is
                // skipped — with O_CLOEXEC set, execve would then close fd 0, leaving
                // the service with NO stdin (a reader gets EBADF) instead of an
                // EOF-yielding /dev/null.
                let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
                if null_fd < 0 {
                    log::error!(
                        "Failed to open /dev/null for stdin: {}",
                        std::io::Error::last_os_error()
                    );
                    std::process::exit(1);
                }
                if null_fd != libc::STDIN_FILENO {
                    unsafe {
                        libc::dup2(null_fd, libc::STDIN_FILENO);
                        libc::close(null_fd);
                    }
                }
            }
        }
        StandardInput::Socket => {
            // For Accept=yes socket-activated services, dup the first
            // LISTEN_FD (fd 3) to stdin. The accepted connection fd was
            // placed at fd 3 by the fork_child fd duplication logic.
            let listen_fd = libc::STDERR_FILENO + 1; // fd 3
            unsafe {
                libc::dup2(listen_fd, libc::STDIN_FILENO);
            }
        }
        StandardInput::File(ref path) => {
            // StandardInput=file:PATH — open the file read-only and make it the
            // process's stdin, mirroring StandardOutput=file:. Opened here in the
            // child (like the Tty arm) so the fd needs no parent hand-off.
            let path_cstr = match std::ffi::CString::new(path.as_bytes()) {
                Ok(c) => c,
                Err(_) => {
                    log::error!("Invalid StandardInput=file: path {:?}", path);
                    std::process::exit(1);
                }
            };
            // No O_CLOEXEC: STDIN_FILENO was just closed (above), so open() may
            // return fd 0 itself, in which case the dup2 is skipped and the fd
            // must survive execve to actually be the service's stdin. When it
            // lands on a higher fd, dup2 moves it to 0 (clearing cloexec anyway).
            let file_fd = unsafe { libc::open(path_cstr.as_ptr(), libc::O_RDONLY) };
            if file_fd < 0 {
                log::error!(
                    "Failed to open StandardInput=file:{} : {}",
                    path,
                    std::io::Error::last_os_error()
                );
                std::process::exit(1);
            }
            if file_fd != libc::STDIN_FILENO {
                unsafe {
                    libc::dup2(file_fd, libc::STDIN_FILENO);
                    libc::close(file_fd);
                }
            }
        }
        StandardInput::Tty | StandardInput::TtyForce | StandardInput::TtyFail => {
            let tty_path = config
                .tty_path
                .as_deref()
                .unwrap_or(Path::new("/dev/console"));
            let tty_path_cstr = match std::ffi::CString::new(tty_path.to_string_lossy().as_bytes())
            {
                Ok(c) => c,
                Err(_) => {
                    log::error!("Invalid TTYPath: {:?}", tty_path);
                    std::process::exit(1);
                }
            };

            // Become session leader so we can acquire a controlling terminal.
            // This is required for ALL tty modes, not just tty-force.
            // Without being a session leader, TIOCSCTTY will fail and the
            // shell won't have a controlling terminal (no job control, no
            // signals, etc).
            //
            // Note: setsid() may fail with EPERM if we are already a session
            // leader (e.g. fork_child already called setsid() for us). That's
            // fine — we just need to BE a session leader, not create a new one.
            unsafe {
                let ret = libc::setsid();
                if ret == -1 {
                    let err = std::io::Error::last_os_error();
                    // EPERM means we're already a session leader — that's OK.
                    if err.raw_os_error() != Some(libc::EPERM) {
                        log::warn!("setsid() failed: {}", err);
                    }
                }
            }

            // Use open_terminal() which retries on EIO, matching systemd behavior
            let tty_fd = open_terminal(&tty_path_cstr, libc::O_RDWR | libc::O_NOCTTY);
            if tty_fd < 0 {
                let err = std::io::Error::last_os_error();
                log::warn!("Failed to open TTY {:?} for stdin: {}", tty_path, err);
                if config.stdin_option == StandardInput::TtyFail {
                    std::process::exit(1);
                }
                // For tty/tty-force, fall back to /dev/null
                log::warn!("Falling back to /dev/null for stdin");
                let null_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
                if null_fd >= 0 && null_fd != libc::STDIN_FILENO {
                    unsafe {
                        libc::dup2(null_fd, libc::STDIN_FILENO);
                        libc::close(null_fd);
                    }
                }
                return;
            }

            apply_tty_size(tty_fd, config);

            // Make this TTY our controlling terminal.
            // For tty-force, pass 1 to steal the TTY even if another session owns it.
            // For tty/tty-fail, pass 0 which will fail if another session owns it.
            // This matches systemd's behavior where all tty modes acquire a
            // controlling terminal — they only differ in how conflicts are handled.
            //
            // Temporarily ignore SIGHUP during TIOCSCTTY, matching systemd's
            // acquire_terminal() — if we already own the tty, TIOCSCTTY can
            // generate a spurious SIGHUP.
            let force_arg: libc::c_int = if config.stdin_option == StandardInput::TtyForce {
                1
            } else {
                0
            };
            unsafe {
                // Ignore SIGHUP during terminal acquisition
                let mut old_sa: libc::sigaction = std::mem::zeroed();
                let mut ignore_sa: libc::sigaction = std::mem::zeroed();
                ignore_sa.sa_sigaction = libc::SIG_IGN;
                libc::sigaction(libc::SIGHUP, &ignore_sa, &mut old_sa);

                let ret = libc::ioctl(tty_fd, libc::TIOCSCTTY, force_arg);

                // Restore old SIGHUP handler
                libc::sigaction(libc::SIGHUP, &old_sa, std::ptr::null_mut());

                if ret < 0 {
                    let err = std::io::Error::last_os_error();
                    log::warn!(
                        "Failed to acquire controlling terminal {:?}: {}",
                        tty_path,
                        err
                    );
                    if config.stdin_option == StandardInput::TtyFail {
                        libc::close(tty_fd);
                        std::process::exit(1);
                    }
                    // For tty/tty-force, continue anyway — the fd is still usable
                    // for I/O even without being the controlling terminal.
                }
            }

            // Dup the TTY fd onto stdin
            if tty_fd != libc::STDIN_FILENO {
                unsafe {
                    libc::dup2(tty_fd, libc::STDIN_FILENO);
                    libc::close(tty_fd);
                }
            }

            // Set stdout/stderr to the TTY when configured as inherit.
            // This is the typical configuration for debug-shell and similar
            // interactive services (StandardOutput=inherit, StandardError=inherit).
            if config.stdout_is_inherit {
                unsafe {
                    libc::dup2(libc::STDIN_FILENO, libc::STDOUT_FILENO);
                }
            }
            if config.stderr_is_inherit {
                unsafe {
                    libc::dup2(libc::STDIN_FILENO, libc::STDERR_FILENO);
                }
            }
        }
    }
}

/// Set up the execution environment for a service and exec into its binary.
///
/// # Ordering invariant — DO NOT reorder stages without careful review
///
/// The stages below must execute in a specific order. Getting this wrong
/// causes silent early-boot crashes (typically SIGABRT during privilege
/// drop) that are extremely difficult to diagnose because mount namespace
/// changes destroy all diagnostic channels (stderr, kmsg, console).
///
/// The critical ordering is:
///
///   1. **Parse config & set up stdio/TTY** — needs stdin pipe from parent
///   2. **Resource limits & scheduling** — must happen as root
///   3. **Create state/logs/runtime directories** — must happen BEFORE the
///      mount namespace, because `ProtectSystem=strict` makes `/` read-only.
///      The mount namespace then bind-mounts these dirs back as writable.
///   4. **Mount namespace** (`setup_mount_namespace`) — applies ProtectSystem,
///      PrivateDevices, ProtectKernelTunables, etc.  After this point,
///      the filesystem is heavily restricted.
///   5. **UTS/network namespaces** — independent of mount namespace
///   6. **Capability bounding set** — must happen before privilege drop
///   7. **Credentials** — needs root to write to /run/credentials
///   8. **OOMScoreAdjust** — negative values need root
///   9. **Privilege drop** (`drop_privileges`) — setresgid + setgroups + setresuid
///  10. **Ambient capabilities** — must happen AFTER privilege drop
///  11. **NoNewPrivileges** — must be last before exec (one-way flag)
///  12. **execv** into the service binary
///
/// If you need to debug early boot crashes, set
/// `Environment=SYSTEMD_LOG_LEVEL=trace` (or `debug`) in the unit's
/// `[Service]` section.  This enables diagnostic writes to `/dev/kmsg`
/// (kernel log ring buffer → serial console) at each stage via the
/// [`crate::kmsg_log::KmsgLogger`], which survives mount namespace
/// changes unlike stderr.
pub fn run_exec_helper() {
    let mut config: ExecHelperConfig = match serde_json::from_reader(std::io::stdin()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[EXEC_HELPER] FATAL: failed to parse config from stdin: {e}");
            std::process::exit(1);
        }
    };

    // Initialise the kmsg logger for this exec-helper child process.
    // The priority order matches real systemd's sd-executor:
    //   1. SYSTEMD_LOG_LEVEL env var   (highest — set in unit's Environment=)
    //   2. log_level from config       (passed by the manager, like --log-level)
    //   3. built-in default: Warn      (lowest)
    let manager_level = config
        .log_level
        .as_deref()
        .and_then(crate::kmsg_log::parse_log_level_filter)
        .unwrap_or(log::LevelFilter::Warn);
    crate::kmsg_log::KmsgLogger::init(&config.name, manager_level);

    log::trace!("config parsed OK");

    // Deferred exec error: when user/group resolution failed for Type=simple,
    // the parent process already returned success. The child exits with the
    // appropriate error code (e.g. 217 for user resolution failure).
    if let Some(exit_code) = config.deferred_exec_error {
        log::error!(
            "Deferred exec error for {}: exiting with code {exit_code}",
            config.name
        );
        std::process::exit(exit_code);
    }

    nix::unistd::close(libc::STDIN_FILENO).expect("I want to be able to close this fd!");

    // Perform "destructive" TTY reset before opening the TTY for stdin.
    // This matches systemd's exec_context_tty_reset() which is called before
    // setup_input(). It resets terminal settings, hangs up prior sessions, and
    // optionally disallocates the VT — ensuring the service gets a clean terminal.
    match config.stdin_option {
        StandardInput::Tty | StandardInput::TtyForce | StandardInput::TtyFail
            if config.tty_reset || config.tty_vhangup || config.tty_vt_disallocate =>
        {
            tty_reset_destructive(&config);
        }
        _ => {}
    }

    // Set up stdin for the actual service process
    setup_stdin(&config);

    // If StandardOutput=tty or StandardError=tty but stdin is NOT a TTY,
    // open the TTY independently for output.
    setup_tty_output(&config);

    // StandardOutput=file:/path, append:/path, truncate:/path
    if let Some(ref path) = config.stdout_file_path {
        let flags = libc::O_WRONLY
            | libc::O_CREAT
            | if config.stdout_file_append {
                libc::O_APPEND
            } else {
                libc::O_TRUNC
            };
        let c_path = std::ffi::CString::new(path.as_str()).unwrap();
        let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o644) };
        if fd >= 0 {
            unsafe {
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::close(fd);
            }
        }
    }
    if let Some(ref path) = config.stderr_file_path {
        let flags = libc::O_WRONLY
            | libc::O_CREAT
            | if config.stderr_file_append {
                libc::O_APPEND
            } else {
                libc::O_TRUNC
            };
        let c_path = std::ffi::CString::new(path.as_str()).unwrap();
        let fd = unsafe { libc::open(c_path.as_ptr(), flags, 0o644) };
        if fd >= 0 {
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
                libc::close(fd);
            }
        }
    }

    // StandardOutput=socket / StandardError=socket: dup the first LISTEN_FD
    // (fd 3) to stdout/stderr. Used by Accept=yes socket-activated services.
    {
        let listen_fd = libc::STDERR_FILENO + 1; // fd 3
        if config.stdout_is_socket {
            unsafe {
                libc::dup2(listen_fd, libc::STDOUT_FILENO);
            }
        }
        if config.stderr_is_socket {
            unsafe {
                libc::dup2(listen_fd, libc::STDERR_FILENO);
            }
        }
    }

    // StandardOutput=journal / StandardError=journal: connect directly to
    // journald's stream socket. This bypasses PID 1's pipe forwarding and
    // ensures journalctl --sync works correctly inside the service.
    setup_journal_stream_output(&config);

    // NOTE: Resource limits (LimitXXX=) are applied later, just before
    // execv(), so that restrictive limits like LimitNOFILE=7 don't prevent
    // the exec helper from opening files during setup (cgroup, mount
    // namespace, etc.).

    if let Err(e) =
        crate::services::fork_os_specific::post_fork_os_specific(&config.platform_specific)
    {
        log::error!("postfork error: {}", e);
        std::process::exit(1);
    }

    // ── Apply UMask= before any file creation ─────────────────────────
    if let Some(mask) = config.umask {
        unsafe { libc::umask(mask as libc::mode_t) };
    }

    // ── Apply Nice= scheduling priority ───────────────────────────────
    if let Some(nice_val) = config.nice {
        // Reset errno before calling — getpriority/setpriority return -1
        // both on error and as a legitimate value, so we must check errno.
        unsafe {
            *libc::__errno_location() = 0;
            let ret = libc::setpriority(libc::PRIO_PROCESS, 0, nice_val);
            if ret == -1 && *libc::__errno_location() != 0 {
                log::warn!(
                    "Failed to set Nice={}: {}",
                    nice_val,
                    std::io::Error::last_os_error()
                );
                // Non-fatal: log and continue, matching systemd's lenient behavior
            }
        }
    }

    // ── Apply CPUSchedulingPolicy= / CPUSchedulingPriority= ───────────
    if let Some(ref policy_str) = config.cpu_scheduling_policy {
        let mut policy = match policy_str.as_str() {
            "other" => libc::SCHED_OTHER,
            "batch" => libc::SCHED_BATCH,
            "idle" => libc::SCHED_IDLE,
            "fifo" => libc::SCHED_FIFO,
            "rr" => libc::SCHED_RR,
            _ => {
                log::warn!("Unknown CPUSchedulingPolicy={}, ignoring", policy_str);
                -1
            }
        };
        if policy >= 0 {
            if config.cpu_scheduling_reset_on_fork.unwrap_or(false) {
                policy |= libc::SCHED_RESET_ON_FORK;
            }
            // For FIFO/RR the priority must be 1-99; for OTHER/BATCH/IDLE it must be 0.
            let prio = if matches!(policy_str.as_str(), "fifo" | "rr") {
                config.cpu_scheduling_priority.unwrap_or(1) as libc::c_int
            } else {
                0
            };
            let param = libc::sched_param {
                sched_priority: prio,
            };
            let ret = unsafe { libc::sched_setscheduler(0, policy, &param) };
            if ret != 0 {
                log::warn!(
                    "Failed to set CPUSchedulingPolicy={} priority={}: {}",
                    policy_str,
                    prio,
                    std::io::Error::last_os_error()
                );
            }
        }
    } else if let Some(prio) = config.cpu_scheduling_priority {
        // Priority without explicit policy — set for current policy
        let param = libc::sched_param {
            sched_priority: prio as libc::c_int,
        };
        let ret = unsafe { libc::sched_setscheduler(0, libc::SCHED_RR, &param) };
        if ret != 0 {
            log::warn!(
                "Failed to set CPUSchedulingPriority={}: {}",
                prio,
                std::io::Error::last_os_error()
            );
        }
    }

    // ── Apply IOSchedulingClass= / IOSchedulingPriority= ─────────────
    {
        let io_class = config.io_scheduling_class;
        let io_prio = config.io_scheduling_priority;
        // Only call ioprio_set if class or priority is explicitly configured.
        // Class 0 means "none" (use kernel default), but if a priority is set
        // we still need to call it.
        if io_class != 0 || io_prio.is_some() {
            let prio_val = io_prio.unwrap_or(4) as u32; // default priority 4
            let class_val = io_class as u32;
            // ioprio = (class << 13) | priority
            let ioprio = (class_val << 13) | (prio_val & 0x1fff);
            // ioprio_set(IOPRIO_WHO_PROCESS=1, 0=self, ioprio)
            let ret = unsafe { libc::syscall(libc::SYS_ioprio_set, 1i32, 0i32, ioprio) };
            if ret < 0 {
                log::warn!(
                    "Failed to set IOSchedulingClass={} IOSchedulingPriority={}: {}",
                    io_class,
                    prio_val,
                    std::io::Error::last_os_error()
                );
                // Non-fatal
            }
        }
    }

    log::trace!("resource limits done, creating directories...");

    // ── Create state/logs/runtime directories BEFORE mount namespace ───
    // These must be created while the filesystem is still writable, because
    // ProtectSystem=strict will make / read-only. The mount namespace setup
    // then bind-mounts these directories read-write. This matches real
    // systemd's ordering: directories are created first, then the mount
    // namespace is applied with those directories whitelisted as writable.
    // Helper: create a managed directory, with DynamicUser=yes support.
    // When dynamic_user is true, create under <base>/private/<name> and
    // symlink <base>/<name> → private/<name> (matching real systemd).
    // Returns the path the service should use (the symlink for dynamic,
    // the direct path otherwise).
    let create_managed_dir_ex = |base: &Path,
                                 dir_name: &str,
                                 mode: u32,
                                 dynamic: bool,
                                 only_create: bool|
     -> String {
        let uid = nix::unistd::Uid::from_raw(config.user);
        let gid = nix::unistd::Gid::from_raw(config.group);
        if dynamic {
            // `private/` is a deliberate security boundary, not an
            // implementation detail: mode 0700 owned root:root so unprivileged
            // host users cannot look into the state of a dynamic user whose UID
            // may later be reused (upstream exec-invoke.c documents the same
            // trick container managers use).  The mode is set explicitly rather
            // than left to the inherited umask.
            //
            // The service itself reaches its directory because the mount
            // namespace replaces `private/` with a tmpfs into which only that
            // service's directory is bound; see `private_dir_tmpfs` below.
            let private_dir = base.join("private");
            let _ = std::fs::create_dir_all(&private_dir);
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &private_dir,
                    std::fs::Permissions::from_mode(0o700),
                );
                let _ = nix::unistd::chown(
                    &private_dir,
                    Some(nix::unistd::Uid::from_raw(0)),
                    Some(nix::unistd::Gid::from_raw(0)),
                );
            }
            let full_path = private_dir.join(dir_name);
            let link_path = base.join(dir_name);
            // Intermediates between the private root and the leaf, at 0755.
            if let Some(parent) = full_path.parent()
                && parent != private_dir
                && !parent.exists()
            {
                let _ = std::fs::create_dir_all(parent);
                let mut p = private_dir.clone();
                for comp in Path::new(dir_name).components().collect::<Vec<_>>().iter().rev().skip(1).rev() {
                    p = p.join(comp);
                    set_dir_mode(&p, 0o755);
                }
            }

            // DynamicUser=0 -> 1 migration.  A previous non-dynamic run left a
            // real directory at <base>/<name>; upstream moves it under
            // private/ and leaves a symlink behind, so the service keeps its
            // data across the switch (TEST-34-DYNAMICUSERMIGRATE).  Only move
            // when there is nothing at the destination: a populated private/
            // copy is the newer state and must win.
            if let Ok(md) = std::fs::symlink_metadata(&link_path)
                && md.is_dir()
                && !full_path.exists()
            {
                match std::fs::rename(&link_path, &full_path) {
                    Ok(()) => log::info!(
                        "DynamicUser=yes: migrated {:?} into {:?}",
                        link_path,
                        full_path
                    ),
                    Err(e) => log::warn!(
                        "DynamicUser=yes: could not migrate {:?} into {:?}: {}",
                        link_path,
                        full_path,
                        e
                    ),
                }
            }

            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create private directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                let _ = std::fs::set_permissions(&full_path, perms);
            }
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            // Link it up from the public place. A nested name (`aaa/bbb`) makes
            // this more than a one-liner:
            //
            //  - the target must be relative to the LINK's directory, not to
            //    `base`. For `/var/lib/aaa/bbb` that is `../private/aaa/bbb`;
            //    a flat `private/aaa/bbb` would resolve to
            //    `/var/lib/aaa/private/aaa/bbb`.
            //  - when a parent was itself configured (`StateDirectory=aaa
            //    aaa/bbb`), `/var/lib/aaa` is already a symlink into private/,
            //    so the link path and the private path are the SAME inode. We
            //    must not create a symlink then, and must not delete what is
            //    there (upstream issue #24783).
            let already_linked = match (
                std::fs::canonicalize(&link_path),
                std::fs::canonicalize(&full_path),
            ) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            };
            // ONLY_CREATE: a configured parent already provides the symlink that
            // covers this path, and link and target are the same inode.
            if !only_create && !already_linked {
                if let Some(parent) = link_path.parent()
                    && !parent.exists()
                {
                    let _ = std::fs::create_dir_all(parent);
                    set_dir_mode(parent, 0o755);
                }
                let target = match link_path.parent() {
                    Some(parent) => path_make_relative(parent, &full_path),
                    None => full_path.clone(),
                };
                let _ = std::fs::remove_file(&link_path); // stale symlink
                if let Err(e) = std::os::unix::fs::symlink(&target, &link_path) {
                    log::warn!(
                        "Failed to create symlink {:?} → {:?}: {}",
                        link_path,
                        target,
                        e
                    );
                }
            }
            log::info!(
                "DynamicUser=yes exec dir: {:?} -> {:?} (uid={} gid={} mode={:o})",
                link_path,
                full_path,
                config.user,
                config.group,
                mode
            );
            full_path.to_string_lossy().into_owned()
        } else {
            let full_path = base.join(dir_name);

            // DynamicUser=1 -> 0 migration, the mirror of the branch above.
            // <base>/<name> is the symlink a dynamic run left behind; drop it
            // and move the real directory back out, so the service keeps its
            // data and <base>/<name> is a directory again.
            //
            // Only fires when the link actually points at private/<name>: an
            // unrelated symlink an admin put there is left alone rather than
            // being replaced by a directory.
            let private_path = base.join("private").join(dir_name);
            if std::fs::read_link(&full_path)
                .is_ok_and(|t| t == Path::new("private").join(dir_name) || t == private_path)
            {
                let _ = std::fs::remove_file(&full_path);
                if private_path.exists()
                    && let Err(e) = std::fs::rename(&private_path, &full_path)
                {
                    log::warn!(
                        "DynamicUser=no: could not migrate {:?} back to {:?}: {}",
                        private_path,
                        full_path,
                        e
                    );
                }
            }

            if let Some(parent) = full_path.parent()
                && parent != base
                && !parent.exists()
            {
                let _ = std::fs::create_dir_all(parent);
                let mut p = base.to_path_buf();
                for comp in Path::new(dir_name).components().collect::<Vec<_>>().iter().rev().skip(1).rev() {
                    p = p.join(comp);
                    set_dir_mode(&p, 0o755);
                }
            }
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                let _ = std::fs::set_permissions(&full_path, perms);
            }
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            full_path.to_string_lossy().into_owned()
        }
    };

    let dynamic = config.dynamic_user;

    // ExecDirectory= base paths. In user-manager mode (SYSTEMD_USER_MANAGER=1,
    // set by run_user_manager) State/Configuration resolve to the XDG base
    // directories under $HOME; otherwise the system paths. Returns the system
    // default byte-for-byte when not a user manager, so the system code path is
    // unchanged.
    let managed_dir_base = |system: &str, user_env: &str, user_rel: &str| -> std::path::PathBuf {
        if std::env::var_os("SYSTEMD_USER_MANAGER").is_none() {
            return std::path::PathBuf::from(system);
        }
        std::env::var_os(user_env)
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from("/root"))
                    .join(user_rel)
            })
    };

    // Parse an ExecDirectory= entry "SRC[:DEST[:FLAGS]]" into (src, dest, ro).
    // DEST is an optional symlink alias (State/Runtime only); FLAGS may contain
    // "ro" for a read-only directory.
    fn parse_exec_dir_entry(entry: &str) -> (String, Option<String>, bool) {
        crate::units::parse_exec_dir_entry(entry)
    }
    // Relative path from `from` to `to` (both absolute). e.g. state→config gives
    // "../../.config", so a symlink at <state>/foo can target "../../.config/foo".
    fn path_make_relative(from: &Path, to: &Path) -> std::path::PathBuf {
        let from_c: Vec<_> = from.components().collect();
        let to_c: Vec<_> = to.components().collect();
        let common = from_c
            .iter()
            .zip(&to_c)
            .take_while(|(a, b)| a == b)
            .count();
        let mut rel = std::path::PathBuf::new();
        for _ in common..from_c.len() {
            rel.push("..");
        }
        for c in &to_c[common..] {
            rel.push(c.as_os_str());
        }
        rel
    }

    /// Set a directory's mode, ignoring failures.
    ///
    /// Intermediate directories of a nested exec directory (`StateDirectory=aaa/bbb`
    /// creates `aaa` on the way to `bbb`) are 0755 in upstream regardless of the
    /// unit's DirectoryMode=, which applies to the leaf only.
    fn set_dir_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }

    // ExecDirectory= entries flagged `:ro` — collected here and enforced as
    // read-only bind mounts once the mount namespace is set up.
    let mut read_only_exec_dirs: Vec<std::path::PathBuf> = Vec::new();
    // (host source, path the service must see it at) for every exec directory
    // created below.  Used to bind them back over a TemporaryFileSystem= tmpfs,
    // which would otherwise hide the service's own StateDirectory= et al.
    let mut exec_dir_binds: Vec<(String, std::path::PathBuf)> = Vec::new();
    // `<base>/private` dirs that need the namespace tmpfs treatment.
    let mut exec_dir_private_bases: Vec<std::path::PathBuf> = Vec::new();
    // Roots of any TemporaryFileSystem= mounts, so directory creation can tell
    // whether a path it is about to write will be hidden by a tmpfs anyway.
    let tmpfs_roots: Vec<PathBuf> = config
        .temporary_file_system
        .iter()
        .map(|e| PathBuf::from(e.split(':').next().unwrap_or(e)))
        .collect();
    // An exec-directory alias under a TemporaryFileSystem= is provided by the
    // bind mount inside the namespace, so it must NOT also be created as a
    // symlink on the host: upstream leaves no trace there, and
    // TEST-34-DYNAMICUSERMIGRATE asserts exactly that (`zzz:yyy` without a
    // tmpfs leaves a symlink, `zzz:xxx` with one leaves nothing).
    let tmpfs_covered = |p: &Path| tmpfs_roots.iter().any(|root| p.starts_with(root));

    if !config.state_directory.is_empty() {
        let base = managed_dir_base("/var/lib", "XDG_STATE_HOME", ".local/state");
        let config_base = managed_dir_base("/etc", "XDG_CONFIG_HOME", ".config");
        let user = std::env::var_os("SYSTEMD_USER_MANAGER").is_some();
        let mode = config.state_directory_mode.unwrap_or(0o755);
        let mut full_paths: Vec<String> = Vec::new();
        for e in crate::units::sorted_exec_dir_entries(&config.state_directory) {
            let (src, dest, ro, only_create) = (e.src, e.dest, e.read_only, e.only_create);
            let src_path = base.join(&src);
            // Migrate-to-symlink: if the state directory is missing but a matching
            // configuration directory exists, create a relative symlink into it so
            // both share storage (mirrors upstream's update-compat symlink). Gated
            // to user mode to leave the system code path unchanged.
            let full = if user && !src_path.exists() && config_base.join(&src).exists() {
                let target = path_make_relative(&base, &config_base).join(&src);
                let _ = std::fs::remove_file(&src_path);
                if let Err(e) = std::os::unix::fs::symlink(&target, &src_path) {
                    log::warn!("Failed to symlink {src_path:?} -> {target:?}: {e}");
                }
                src_path.to_string_lossy().into_owned()
            } else {
                create_managed_dir_ex(&base, &src, mode, dynamic, only_create)
            };
            exec_dir_binds.push((full.clone(), src_path.clone()));
            if dynamic {
                exec_dir_private_bases.push(base.join("private"));
            }
            // Optional destination: a symlink alias pointing at the source dir.
            if let Some(dest) = dest {
                let dest_path = base.join(&dest);
                let _ = std::fs::remove_file(&dest_path);
                if !tmpfs_covered(&dest_path) {
                    // The bind provides it inside the namespace when a tmpfs
                    // covers it; otherwise link it here, relative to the
                    // alias's own directory (see the note in create_exec_dirs).
                    if let Some(parent) = dest_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let target = match dest_path.parent() {
                        Some(parent) => path_make_relative(parent, &base.join(&src)),
                        None => base.join(&src),
                    };
                    if let Err(e) = std::os::unix::fs::symlink(&target, &dest_path) {
                        log::warn!("Failed to symlink {dest_path:?} -> {target:?}: {e}");
                    }
                }
                exec_dir_binds.push((full.clone(), dest_path));
            }
            if ro {
                read_only_exec_dirs.push(base.join(&src));
            }
            full_paths.push(full);
        }
        unsafe { std::env::set_var("STATE_DIRECTORY", full_paths.join(":")) };
    }

    // LogsDirectory=, RuntimeDirectory= and CacheDirectory= take the same
    // `source[:destination[:access-mode]]` syntax as StateDirectory=: the
    // destination is a symlink alias pointing at the source, and `:ro` makes
    // the directory read-only for the service.  Passing the raw entry through
    // would create a directory literally named `zzz:yyy`
    // (TEST-34-DYNAMICUSERMIGRATE).
    let create_exec_dirs = |base: &Path,
                            entries: &[String],
                            mode: u32,
                            dynamic: bool,
                            ro_out: &mut Vec<PathBuf>,
                            bind_out: &mut Vec<(String, PathBuf)>,
                            private_out: &mut Vec<PathBuf>|
     -> Vec<String> {
        let mut full_paths = Vec::with_capacity(entries.len());
        for e in crate::units::sorted_exec_dir_entries(entries) {
            let (src, dest, ro) = (e.src, e.dest, e.read_only);
            let full = create_managed_dir_ex(base, &src, mode, dynamic, e.only_create);
            bind_out.push((full.clone(), base.join(&src)));
            if dynamic {
                private_out.push(base.join("private"));
            }
            full_paths.push(full.clone());
            if let Some(dest) = dest {
                let dest_path = base.join(&dest);
                let _ = std::fs::remove_file(&dest_path);
                if !tmpfs_covered(&dest_path) {
                    if let Some(parent) = dest_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    // Relative to the ALIAS's own directory, not to `base`: a
                    // nested alias `aaa/111` sits in `<base>/aaa`, so a bare
                    // `xxx/yyy` would resolve to `<base>/aaa/xxx/yyy` and
                    // dangle. Flat aliases are unaffected: the relative path
                    // from `<base>` to `<base>/zzz` is still just `zzz`.
                    let target = match dest_path.parent() {
                        Some(parent) => path_make_relative(parent, &base.join(&src)),
                        None => base.join(&src),
                    };
                    if let Err(e) = std::os::unix::fs::symlink(&target, &dest_path) {
                        log::warn!("Failed to symlink {dest_path:?} -> {target:?}: {e}");
                    }
                }
                // The alias is a symlink on the host, which a tmpfs over `base`
                // would hide along with its target; bind the real directory
                // onto the alias path so the service still sees it.
                bind_out.push((full.clone(), dest_path));
            }
            if ro {
                ro_out.push(base.join(&src));
            }
        }
        full_paths
    };

    if !config.logs_directory.is_empty() {
        let mode = config.logs_directory_mode.unwrap_or(0o755);
        let full_paths = create_exec_dirs(
            Path::new("/var/log"),
            &config.logs_directory,
            mode,
            dynamic,
            &mut read_only_exec_dirs,
            &mut exec_dir_binds,
            &mut exec_dir_private_bases,
        );
        unsafe { std::env::set_var("LOGS_DIRECTORY", full_paths.join(":")) };
    }

    if !config.runtime_directory.is_empty() {
        let mode = config.runtime_directory_mode.unwrap_or(0o755);
        let full_paths = create_exec_dirs(
            Path::new("/run"),
            &config.runtime_directory,
            mode,
            dynamic,
            &mut read_only_exec_dirs,
            &mut exec_dir_binds,
            &mut exec_dir_private_bases,
        );
        unsafe { std::env::set_var("RUNTIME_DIRECTORY", full_paths.join(":")) };
    }

    // ── Create CacheDirectory= directories under /var/cache/ ──────────
    if !config.cache_directory.is_empty() {
        let mode = config.cache_directory_mode.unwrap_or(0o755);
        let full_paths = create_exec_dirs(
            Path::new("/var/cache"),
            &config.cache_directory,
            mode,
            dynamic,
            &mut read_only_exec_dirs,
            &mut exec_dir_binds,
            &mut exec_dir_private_bases,
        );
        unsafe { std::env::set_var("CACHE_DIRECTORY", full_paths.join(":")) };
    }

    // ── Create ConfigurationDirectory= directories under /etc/ ────────
    // ConfigurationDirectory never uses private/ even with DynamicUser=yes.
    if !config.configuration_directory.is_empty() {
        let base = managed_dir_base("/etc", "XDG_CONFIG_HOME", ".config");
        let mode = config.configuration_directory_mode.unwrap_or(0o755);
        let full_paths: Vec<String> = config
            .configuration_directory
            .iter()
            .map(|d| {
                // ConfigurationDirectory does not support a symlink destination,
                // but may carry a `:ro` flag; take only the source name.
                let (src, _dest, ro) = parse_exec_dir_entry(d);
                if ro {
                    read_only_exec_dirs.push(base.join(&src));
                }
                let full = create_managed_dir_ex(&base, &src, mode, false, false);
                exec_dir_binds.push((full.clone(), base.join(&src)));
                full
            })
            .collect();
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("CONFIGURATION_DIRECTORY", full_paths.join(":")) };
    }

    // ExecDirectory= entries flagged `:ro` are made read-only for the service by
    // reusing the ReadOnlyPaths= machinery: appending them forces a mount
    // namespace (see needs_mount_ns below) and bind-mounts each read-only.
    for dir in &read_only_exec_dirs {
        config
            .read_only_paths
            .push(dir.to_string_lossy().into_owned());
    }

    // A TemporaryFileSystem= over /var/lib (or /run, /var/cache, /var/log, /etc)
    // hides the exec directories that were just created there, so a service with
    // both StateDirectory=zzz and TemporaryFileSystem=/var/lib cannot see its own
    // state directory.  Upstream always mounts exec directories into the
    // namespace; bind them back over the tmpfs by reusing the BindPaths=
    // machinery, which already opens an O_PATH fd for each source *before* the
    // tmpfs is mounted and binds from /proc/self/fd/N afterwards.
    //
    // Scoped to directories a tmpfs actually covers, so a service without
    // TemporaryFileSystem= keeps exactly its previous mount layout.
    // Carried as an explicit (source, destination) pair rather than a
    // BindPaths= string: an exec directory name may itself contain a colon
    // (`StateDirectory=zzz:x\:yz`), which the `source:dest:options` syntax
    // cannot express.
    // Bind an exec directory into the namespace when the service could not
    // otherwise reach it at the path it expects:
    //
    //  - a TemporaryFileSystem= tmpfs hides it, or
    //  - DynamicUser=yes put the real directory under `<base>/private/`, which
    //    the dynamic user would have to traverse.  Upstream keeps `private/`
    //    closed and mounts the directory at `<base>/<name>` instead, so the
    //    service never walks through it.  `source != dest` is exactly that case.
    config.exec_dir_paths = exec_dir_binds
        .iter()
        .map(|(_, dest)| dest.to_string_lossy().into_owned())
        .collect();

    // DynamicUser=: the namespace gets a tmpfs over `<base>/private`, and each
    // of this service's directories is bound into it under the same name.  That
    // reproduces upstream's arrangement exactly: the 0700 boundary still keeps
    // other host users out, while the service reaches its own directory through
    // the existing `<base>/<name>` symlink.  Binding the symlink path itself
    // does nothing, because the kernel resolves it straight back to the source.
    exec_dir_private_bases.sort();
    exec_dir_private_bases.dedup();
    config.private_dir_tmpfs = exec_dir_private_bases
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    // starts_with, not "parent is the private base": a nested exec directory
    // lives at `<base>/private/quux/pief`, whose parent is `private/quux`, so a
    // direct-child test skipped it entirely and its contents never appeared in
    // the tmpfs, leaving the public symlinks dangling.
    //
    // These are registered before the public/alias binds below so the tmpfs has
    // the real directories in place by the time a symlinked alias path is
    // resolved through it.
    for (source, _) in &exec_dir_binds {
        let src = Path::new(source);
        if exec_dir_private_bases
            .iter()
            .any(|b| src.starts_with(b) && src != b.as_path())
        {
            config
                .exec_dir_binds
                .push((source.clone(), source.clone()));
        }
    }

    if !exec_dir_binds.is_empty() {
        for (source, dest) in &exec_dir_binds {
            if !tmpfs_covered(dest) && Path::new(source) == dest.as_path() {
                continue;
            }
            log::trace!(
                "exec dir {source} bound back over TemporaryFileSystem= at {}",
                dest.display()
            );
            config
                .exec_dir_binds
                .push((source.clone(), dest.to_string_lossy().into_owned()));
        }
    }

    // ── Namespace-based isolation (must happen before privilege drop) ──
    // Determine if we need a mount namespace. Any of the Protect*/Private*
    // directives that manipulate the filesystem require one.
    // When the '+' prefix is used, skip all namespace/sandbox restrictions.
    // The command runs with full root privileges in the host namespaces.
    // The '!' and '!!' prefixes are treated equivalently for now.
    let needs_mount_ns = !config.privileged_prefix
        && (config.private_tmp
            || config.private_devices
            || config.private_mounts
            || config.private_pids
            || config.protect_kernel_tunables
            || config.protect_kernel_modules
            || config.protect_kernel_logs
            || config.protect_control_groups
            || config.protect_clock
            || config.protect_hostname
            || !config.read_write_paths.is_empty()
            || !config.read_only_paths.is_empty()
            || !config.inaccessible_paths.is_empty()
            || !config.bind_paths.is_empty()
            || !config.bind_read_only_paths.is_empty()
            || !config.temporary_file_system.is_empty()
            || matches!(config.protect_system.as_str(), "yes" | "full" | "strict")
            || matches!(config.protect_home.as_str(), "yes" | "read-only" | "tmpfs")
            || matches!(
                config.mount_flags.as_deref(),
                Some("slave") | Some("private")
            )
            || config.root_directory.is_some()
            || matches!(
                config.protect_proc.as_str(),
                "noaccess" | "invisible" | "ptraceable"
            )
            || config.proc_subset == "pid");

    if needs_mount_ns {
        if let Some(ns_pid) = config.join_namespace_pid {
            // JoinsNamespaceOf=: join the running service's mount namespace
            // instead of creating a new one. The target already has all
            // isolation (PrivateTmp, ProtectSystem, etc.) applied.
            log::trace!(
                "joining mount namespace of PID {} (JoinsNamespaceOf)",
                ns_pid
            );
            let ns_path = format!("/proc/{}/ns/mnt", ns_pid);
            match std::fs::File::open(&ns_path) {
                Ok(ns_file) => {
                    use std::os::unix::io::AsRawFd;
                    let ret = unsafe { libc::setns(ns_file.as_raw_fd(), libc::CLONE_NEWNS) };
                    if ret != 0 {
                        log::warn!(
                            "Failed to join mount namespace of PID {}: {}",
                            ns_pid,
                            std::io::Error::last_os_error()
                        );
                        // Fall back to creating a new namespace
                        setup_mount_namespace(&config);
                    } else {
                        log::trace!("successfully joined mount namespace of PID {}", ns_pid);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to open mount namespace of PID {}: {}", ns_pid, e);
                    // Fall back to creating a new namespace
                    setup_mount_namespace(&config);
                }
            }
        } else {
            log::trace!(
                "entering mount namespace (protect_system={}, private_dev={}, private_tmp={}, protect_kernel_tunables={}, protect_kernel_logs={})",
                config.protect_system,
                config.private_devices,
                config.private_tmp,
                config.protect_kernel_tunables,
                config.protect_kernel_logs
            );
            setup_mount_namespace(&config);
        }
        log::trace!("mount namespace setup complete");
    } else {
        log::trace!("no mount namespace needed");
    }

    // ── RootDirectory= — chroot to specified root ─────────────────────
    if let Some(ref root_dir) = config.root_directory
        && !config.privileged_prefix
    {
        let c_root = std::ffi::CString::new(root_dir.as_str()).unwrap_or_default();
        let ret = unsafe { libc::chroot(c_root.as_ptr()) };
        if ret != 0 {
            log::warn!(
                "Failed to chroot to '{}': {}",
                root_dir,
                std::io::Error::last_os_error()
            );
        } else {
            // After chroot, change to / so relative paths work
            let ret = unsafe { libc::chdir(c"/".as_ptr()) };
            if ret != 0 {
                log::warn!(
                    "Failed to chdir to / after chroot: {}",
                    std::io::Error::last_os_error()
                );
            }
            log::trace!("chroot to '{}' successful", root_dir);
        }
    }

    // ── ProtectHostname= — UTS namespace ──────────────────────────────
    // Both "yes" and "private" modes create a new UTS namespace, isolating
    // hostname changes from the host. "yes" additionally installs a minimal
    // seccomp filter that denies sethostname()/setdomainname() (EPERM), so the
    // service cannot change the hostname even within its own namespace even if
    // it holds CAP_SYS_ADMIN, matching systemd. "private" leaves those calls
    // allowed. The filter is installed AFTER our own initial sethostname().
    if config.protect_hostname && !config.privileged_prefix {
        let ret = unsafe { libc::unshare(libc::CLONE_NEWUTS) };
        if ret != 0 {
            log::warn!(
                "Failed to create UTS namespace for ProtectHostname=: {}",
                std::io::Error::last_os_error()
            );
            // Non-fatal: continue without UTS isolation
        } else {
            if let Some(ref hostname) = config.protect_hostname_name {
                // Set the hostname in the new UTS namespace
                let cname = std::ffi::CString::new(hostname.as_str()).unwrap_or_default();
                let ret = unsafe { libc::sethostname(cname.as_ptr(), hostname.len()) };
                if ret != 0 {
                    log::warn!(
                        "Failed to set hostname '{}' in UTS namespace: {}",
                        hostname,
                        std::io::Error::last_os_error()
                    );
                }
            }
            // ProtectHostname=yes (the default mode) locks the hostname down.
            if config.protect_hostname_mode.as_deref() != Some("private") {
                seccomp_block_hostname();
            }
        }
    }

    // ── PrivateNetwork= — network namespace ───────────────────────────
    if config.private_network && !config.privileged_prefix {
        let ret = unsafe { libc::unshare(libc::CLONE_NEWNET) };
        if ret != 0 {
            log::warn!(
                "Failed to create network namespace for PrivateNetwork=: {}",
                std::io::Error::last_os_error()
            );
        } else {
            // Bring up the loopback interface in the new namespace
            bring_up_loopback();
        }
    }

    // ── NetworkNamespacePath= — join existing network namespace ────────
    if let Some(ref ns_path) = config.network_namespace_path
        && !config.privileged_prefix
    {
        match std::fs::File::open(ns_path) {
            Ok(f) => {
                use std::os::unix::io::AsRawFd;
                let ret = unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWNET) };
                if ret != 0 {
                    log::warn!(
                        "Failed to join network namespace {}: {}",
                        ns_path,
                        std::io::Error::last_os_error()
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to open NetworkNamespacePath={}: {}", ns_path, e);
            }
        }
    }

    // ── IPCNamespacePath= — join existing IPC namespace ────────────────
    if let Some(ref ns_path) = config.ipc_namespace_path
        && !config.privileged_prefix
    {
        match std::fs::File::open(ns_path) {
            Ok(f) => {
                use std::os::unix::io::AsRawFd;
                let ret = unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWIPC) };
                if ret != 0 {
                    log::warn!(
                        "Failed to join IPC namespace {}: {}",
                        ns_path,
                        std::io::Error::last_os_error()
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to open IPCNamespacePath={}: {}", ns_path, e);
            }
        }
    }

    // ── PrivateIPC= — IPC namespace ────────────────────────────────────
    if config.private_ipc && !config.privileged_prefix {
        let ret = unsafe { libc::unshare(libc::CLONE_NEWIPC) };
        if ret != 0 {
            log::warn!(
                "Failed to create IPC namespace for PrivateIPC=: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // ── UserNamespacePath= — join an existing user namespace ───────────
    // Enter the caller-specified user namespace (usually /proc/PID/ns/user)
    // via setns(2) instead of creating a fresh one with PrivateUsers=. This
    // runs AFTER the network/IPC namespace setup above so those setns/unshare
    // calls execute while we still hold host CAP_SYS_ADMIN (needed to
    // setns(CLONE_NEWNET) into an existing netns owned by the target user
    // namespace); joining the user namespace last avoids losing that
    // privilege. setns(CLONE_NEWUSER) requires the caller to be
    // single-threaded, which holds for this freshly-exec'd helper.
    if let Some(ref ns_path) = config.user_namespace_path
        && !config.privileged_prefix
    {
        match std::fs::File::open(ns_path) {
            Ok(f) => {
                use std::os::unix::io::AsRawFd;
                let ret = unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWUSER) };
                if ret != 0 {
                    log::warn!(
                        "Failed to join user namespace {}: {}",
                        ns_path,
                        std::io::Error::last_os_error()
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to open UserNamespacePath={}: {}", ns_path, e);
            }
        }
    }

    // ── PrivateUsers= — user namespace ─────────────────────────────────
    if config.private_users && !config.privileged_prefix {
        // Capture uid/gid BEFORE unshare — after creating the user namespace
        // the process has no mapping yet and getuid()/getgid() return 65534.
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        // If the exec directories were id-mapped through a namespace, JOIN that
        // one rather than creating a second: the translation only lines up if
        // the service ends up in the very namespace the mapping was made
        // against. Its maps are already written, so nothing more to do.
        // The service gets its OWN namespace and never joins the mount's.
        //
        // These two namespaces carry OPPOSITE maps and cannot be the same one.
        // The mount idmap needs the on-disk owner on the inside ("<uid> 0 1");
        // the process needs root on the inside ("0 <uid> 1") so its uid 0 is the
        // service's uid on the host. Joining the mount's namespace would hand
        // the process a mapping in which in-namespace 61221 is host root.
        //
        // Separating them ALONE was tried earlier and did not fix the EOVERFLOW,
        // because the map direction was still wrong; the two changes only mean
        // anything together. Upstream keeps them separate for the same reason.
        let joined = false;

        let ret = if joined { 0 } else { unsafe { libc::unshare(libc::CLONE_NEWUSER) } };
        if joined {
            // Maps were written by whoever created the namespace.
        } else if ret != 0 {
            log::warn!(
                "Failed to create user namespace for PrivateUsers=: {}",
                std::io::Error::last_os_error()
            );
        } else {
            // Write uid_map/gid_map per PrivateUsers(Ex)= mode:
            //   yes/self -> "0 <uid> 1" (root maps to the caller's uid),
            //   identity -> "0 0 65536", full -> "0 0 4294967295" (identity map).
            // setgroups is denied for every mode except "full", which allows it.
            let (uid_map, gid_map, deny_setgroups) = match config.private_users_mode.as_str() {
                "identity" => ("0 0 65536\n".to_string(), "0 0 65536\n".to_string(), true),
                "full" => (
                    "0 0 4294967295\n".to_string(),
                    "0 0 4294967295\n".to_string(),
                    false,
                ),
                // PrivateUsers=yes/self. Upstream (PRIVATE_USERS_SELF in
                // exec-invoke.c) identity-maps TWO ids: the caller's own, and
                // the service's target id, so both are representable inside.
                //     <saved_uid> <saved_uid> 1
                //     <uid>       <uid>       1
                // Writing only "0 <uid> 1" left the service's own uid with no
                // mapping, which is why dropping to it used to fail EINVAL, and
                // why /proc/self/uid_map had a single line where
                // TEST-34's check_idmapped reads field 1 of line 2.
                _ => {
                    let mut um = format!("{uid} {uid} 1\n");
                    let mut gm = format!("{gid} {gid} 1\n");
                    if config.user != uid {
                        um.push_str(&format!("{} {} 1\n", config.user, config.user));
                    }
                    if config.group != gid {
                        gm.push_str(&format!("{} {} 1\n", config.group, config.group));
                    }
                    (um, gm, true)
                }
            };
            let _ = std::fs::write("/proc/self/uid_map", &uid_map);
            if deny_setgroups {
                // Must deny setgroups before writing gid_map (kernel requirement
                // when unprivileged; harmless when privileged).
                let _ = std::fs::write("/proc/self/setgroups", "deny\n");
            }
            let _ = std::fs::write("/proc/self/gid_map", &gid_map);
        }
    }

    // ── PrivatePIDs= — PID namespace /proc remount ─────────────────────
    // The process is already PID 1 in a new PID namespace (clone was called
    // with CLONE_NEWPID in start_service). We just need to remount /proc to
    // reflect the new namespace.
    if config.private_pids && !config.privileged_prefix {
        // Unmount old /proc first to avoid stacking mounts
        unsafe { libc::umount2(c"/proc".as_ptr(), libc::MNT_DETACH) };
        let ret = unsafe {
            libc::mount(
                c"proc".as_ptr(),
                c"/proc".as_ptr(),
                c"proc".as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to remount /proc for PrivatePIDs=: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // ── ProtectProc= / ProcSubset= — remount /proc with hidepid=/subset= ──
    // These require a mount namespace to be effective. The mount namespace is
    // typically set up by PrivateMounts=, ProtectSystem=, PrivateTmp=, etc.
    if !config.privileged_prefix {
        let hidepid = match config.protect_proc.as_str() {
            "noaccess" => Some("noaccess"),
            "invisible" => Some("invisible"),
            "ptraceable" => Some("ptraceable"),
            _ => None,
        };
        let subset = match config.proc_subset.as_str() {
            "pid" => Some("pid"),
            _ => None,
        };
        if hidepid.is_some() || subset.is_some() {
            let mut opts = String::new();
            if let Some(h) = hidepid {
                opts.push_str(&format!("hidepid={h}"));
            }
            if let Some(s) = subset {
                if !opts.is_empty() {
                    opts.push(',');
                }
                opts.push_str(&format!("subset={s}"));
            }
            let opts_c = std::ffi::CString::new(opts.as_str()).unwrap();
            let ret = unsafe {
                libc::mount(
                    c"proc".as_ptr(),
                    c"/proc".as_ptr(),
                    c"proc".as_ptr(),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
                    opts_c.as_ptr() as *const libc::c_void,
                )
            };
            if ret != 0 {
                log::warn!(
                    "Failed to remount /proc with {}: {}",
                    opts,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // ── CapabilityBoundingSet= — drop capabilities from bounding set ──
    if !config.capability_bounding_set.is_empty() && !config.privileged_prefix {
        apply_capability_bounding_set(&config);
    }

    log::trace!("namespaces done, setting up credentials...");

    // Import credentials from the system credential store into a per-service
    // credential directory. This must happen BEFORE dropping privileges,
    // because /run/credentials/ is typically only writable by root.
    // Matches systemd's credential directives:
    //   SetCredential=       — write inline data to credential dir
    //   LoadCredential=      — copy file to credential dir
    //   ImportCredential=    — glob-match from system credential stores
    // The order matches systemd: SetCredential first (lowest priority,
    // can be overridden), then LoadCredential/LoadCredentialEncrypted,
    // then ImportCredential (highest priority, won't overwrite).
    log::trace!("credentials setup");
    let has_credentials = !config.import_credentials.is_empty()
        || !config.load_credentials.is_empty()
        || !config.load_credentials_encrypted.is_empty()
        || !config.set_credentials.is_empty()
        || !config.set_credentials_encrypted.is_empty();
    if has_credentials {
        setup_credentials(&config);
    }

    // NOTE: State/logs/runtime directory creation has been moved BEFORE
    // mount namespace setup (see above) so that directories exist when
    // ProtectSystem=strict makes the filesystem read-only. The mount
    // namespace code then bind-mounts them back as read-write.

    // Apply OOMScoreAdjust= setting. Write the value to /proc/self/oom_score_adj
    // before dropping privileges, because negative values (making the process
    // less likely to be OOM-killed) require root or CAP_SYS_RESOURCE.
    if let Some(adj) = config.oom_score_adjust {
        let path = Path::new("/proc/self/oom_score_adj");
        if let Err(e) = std::fs::write(path, format!("{adj}")) {
            log::warn!(
                "Failed to set OOMScoreAdjust to {} ({:?}): {}",
                adj,
                path,
                e
            );
            // Non-fatal: log and continue, matching systemd's lenient behavior
            // when the kernel rejects the value or the file is unavailable.
        }
    }

    // Apply CoredumpFilter= setting by writing to /proc/self/coredump_filter.
    if let Some(ref filter) = config.coredump_filter {
        // Parse the value: systemd supports hex (0x33), named flags, or decimal.
        // We support hex (0x prefix) and decimal numeric values.
        let numeric = if let Some(hex) = filter.strip_prefix("0x") {
            u32::from_str_radix(hex, 16).ok()
        } else if let Some(hex) = filter.strip_prefix("0X") {
            u32::from_str_radix(hex, 16).ok()
        } else {
            filter.parse::<u32>().ok()
        };
        if let Some(val) = numeric {
            let path = std::path::Path::new("/proc/self/coredump_filter");
            if let Err(e) = std::fs::write(path, format!("0x{val:08x}")) {
                log::warn!("Failed to set CoredumpFilter to {}: {}", filter, e);
            }
        }
    }

    // Apply TimerSlackNSec= setting via prctl(PR_SET_TIMERSLACK).
    if let Some(nsec) = config.timer_slack_nsec {
        let ret = unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, nsec as libc::c_ulong) };
        if ret != 0 {
            log::warn!(
                "Failed to set TimerSlackNSec to {}: {}",
                nsec,
                std::io::Error::last_os_error()
            );
        }
    }

    // Apply MemoryTHP= via prctl(PR_SET_THP_DISABLE, ...). See systemd.exec(5).
    {
        const PR_SET_THP_DISABLE: libc::c_int = 41;
        // Bit 1 of the disable flags: keep THPs only where explicitly madvised
        // (kernels >= 6.18). Older kernels reject it with EINVAL.
        const PR_THP_DISABLE_EXCEPT_ADVISED: libc::c_long = 1 << 1;
        let thp_args: Option<(libc::c_long, libc::c_long)> = match config.memory_thp.as_str() {
            "disable" => Some((1, 0)),
            "madvise" => Some((1, PR_THP_DISABLE_EXCEPT_ADVISED)),
            "system" => Some((0, 0)),
            _ => None, // "inherit" (default) or unset: leave THP setting untouched
        };
        if let Some((arg2, arg3)) = thp_args {
            let ret = unsafe { libc::prctl(PR_SET_THP_DISABLE, arg2, arg3, 0, 0) };
            if ret != 0 {
                // Best-effort, mirroring upstream: EINVAL just means the kernel
                // lacks support for the requested flag (e.g. madvise pre-6.18).
                log::warn!(
                    "Failed to apply MemoryTHP={}: {}",
                    config.memory_thp,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // Apply CPUAffinity= setting via sched_setaffinity().
    if !config.cpu_affinity.is_empty() {
        let mut cpus: Vec<usize> = Vec::new();
        for token in &config.cpu_affinity {
            for part in token.split_whitespace() {
                if let Some((start, end)) = part.split_once('-') {
                    if let (Ok(s), Ok(e)) = (start.parse::<usize>(), end.parse::<usize>()) {
                        for c in s..=e {
                            cpus.push(c);
                        }
                    }
                } else if let Ok(c) = part.parse::<usize>() {
                    cpus.push(c);
                }
            }
        }
        if !cpus.is_empty() {
            let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
            unsafe { libc::CPU_ZERO(&mut set) };
            for &cpu in &cpus {
                if cpu < libc::CPU_SETSIZE as usize {
                    unsafe { libc::CPU_SET(cpu, &mut set) };
                }
            }
            let ret =
                unsafe { libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) };
            if ret != 0 {
                log::warn!(
                    "Failed to set CPUAffinity: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // Apply NUMAPolicy=/NUMAMask= via set_mempolicy() before exec. On failure
    // (e.g. an invalid policy such as bind/interleave without a mask) exit 242,
    // matching systemd's EXIT_NUMA_POLICY exec error.
    {
        let numa_policy = crate::numa::NumaPolicy {
            type_: config
                .numa_policy
                .as_deref()
                .and_then(crate::numa::mpol_from_string)
                .unwrap_or(-1),
            nodes: config
                .numa_mask
                .as_deref()
                .and_then(crate::numa::parse_numa_mask)
                .unwrap_or_default(),
        };
        if numa_policy.get_type() >= 0
            && let Err(e) = crate::numa::apply_numa_policy(&numa_policy)
        {
            log::error!("Failed to set NUMA policy (errno {e}), exiting");
            std::process::exit(242);
        }
    }

    // ── KeyringMode= — kernel session keyring setup ──────────────────
    // KEYCTL_JOIN_SESSION_KEYRING (1) with NULL name creates a new
    // anonymous session keyring.  KEYCTL_LINK (8) with
    // KEY_SPEC_USER_KEYRING (-4) into KEY_SPEC_SESSION_KEYRING (-3)
    // links the user keyring so keys are shared between units of the
    // same user.
    if !config.privileged_prefix {
        const KEYCTL_JOIN_SESSION_KEYRING: libc::c_int = 1;
        const KEYCTL_LINK: libc::c_int = 8;
        const KEY_SPEC_USER_KEYRING: libc::c_int = -4;
        const KEY_SPEC_SESSION_KEYRING: libc::c_int = -3;

        match config.keyring_mode.as_deref() {
            Some("private") => {
                let ret = unsafe {
                    libc::syscall(
                        libc::SYS_keyctl,
                        KEYCTL_JOIN_SESSION_KEYRING as libc::c_long,
                        std::ptr::null::<libc::c_char>() as libc::c_long,
                    )
                };
                if ret < 0 {
                    log::warn!(
                        "Failed to create private session keyring: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
            Some("shared") => {
                // Create a new session keyring first
                let ret = unsafe {
                    libc::syscall(
                        libc::SYS_keyctl,
                        KEYCTL_JOIN_SESSION_KEYRING as libc::c_long,
                        std::ptr::null::<libc::c_char>() as libc::c_long,
                    )
                };
                if ret < 0 {
                    log::warn!(
                        "Failed to create shared session keyring: {}",
                        std::io::Error::last_os_error()
                    );
                } else {
                    // Link the user keyring into the new session keyring
                    let ret = unsafe {
                        libc::syscall(
                            libc::SYS_keyctl,
                            KEYCTL_LINK as libc::c_long,
                            KEY_SPEC_USER_KEYRING as libc::c_long,
                            KEY_SPEC_SESSION_KEYRING as libc::c_long,
                        )
                    };
                    if ret < 0 {
                        log::warn!(
                            "Failed to link user keyring into session keyring: {}",
                            std::io::Error::last_os_error()
                        );
                    }
                }
            }
            // "inherit" or None — do nothing
            _ => {}
        }
    }

    // ── SecureBits= — set secure-bits flags ─────────────────────────
    // Must be applied BEFORE privilege drop so keep-caps takes effect
    // before the setuid() call.
    if !config.secure_bits.is_empty() && !config.privileged_prefix {
        let mut bits: libc::c_ulong = 0;
        for flag in &config.secure_bits {
            match flag.as_str() {
                "keep-caps" => bits |= 1 << 4,              // SECBIT_KEEP_CAPS
                "keep-caps-locked" => bits |= 1 << 5,       // SECBIT_KEEP_CAPS_LOCKED
                "no-setuid-fixup" => bits |= 1 << 2,        // SECBIT_NO_SETUID_FIXUP
                "no-setuid-fixup-locked" => bits |= 1 << 3, // SECBIT_NO_SETUID_FIXUP_LOCKED
                "noroot" => bits |= 1 << 0,                 // SECBIT_NOROOT
                "noroot-locked" => bits |= 1 << 1,          // SECBIT_NOROOT_LOCKED
                _ => log::warn!("Unknown SecureBits flag '{}', ignoring", flag),
            }
        }
        if bits != 0 {
            let ret = unsafe { libc::prctl(libc::PR_SET_SECUREBITS, bits) };
            if ret != 0 {
                log::warn!(
                    "Failed to set SecureBits to 0x{:x}: {}",
                    bits,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    log::trace!(
        "pre-privilege-drop (uid={}, gid={}, target_uid={}, target_gid={}, privileged={})",
        nix::unistd::getuid(),
        nix::unistd::getgid(),
        config.user,
        config.group,
        config.privileged_prefix,
    );

    // PAMName=: run the PAM account + session stack as root, before the UID
    // drop, so pam_systemd's `default-capability-ambient-set=` raises ambient
    // caps we can fold into the set re-applied below. Failures are non-fatal
    // (the service still starts without the session, as it did when PAMName=
    // was ignored) so this can never regress an existing PAMName= unit.
    let mut pam_ambient_caps: Vec<u64> = Vec::new();
    if let Some(pam_name) = config.pam_name.as_deref().filter(|s| !s.is_empty()) {
        let username = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(config.user))
            .ok()
            .flatten()
            .map(|u| u.name)
            .unwrap_or_else(|| config.user.to_string());
        match run_pam_session(pam_name, &username, &config.env) {
            Ok(()) => {
                pam_ambient_caps = read_ambient_caps();
                log::trace!(
                    "PAM session '{}' opened for user '{}'; ambient caps from PAM: {:?}",
                    pam_name,
                    username,
                    pam_ambient_caps
                );
            }
            Err(e) => {
                log::warn!("Failed to set up PAM session '{pam_name}', continuing without it: {e}");
            }
        }
    }

    // Resolve ambient capabilities BEFORE dropping privileges so we can
    // set PR_SET_KEEPCAPS and retain them across the UID change. Ambient caps
    // contributed by the PAM session (above) are merged in so the post-drop
    // re-raise keeps them too.
    let mut ambient_caps = resolve_ambient_caps(&config.ambient_capabilities);
    for cap in pam_ambient_caps {
        if !ambient_caps.contains(&cap) {
            ambient_caps.push(cap);
        }
    }

    log::trace!("about to drop privileges...");

    // When the '+' prefix is used, skip privilege drop entirely — the
    // command runs as root (or whatever user PID 1 runs as).
    if nix::unistd::getuid().is_root() && !config.privileged_prefix {
        // If ambient capabilities are requested, tell the kernel to keep
        // permitted capabilities across the setuid() call.  Without this
        // the capability sets are cleared when changing UID from root to
        // an unprivileged user.
        if !ambient_caps.is_empty() {
            let ret = unsafe { libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0) };
            if ret != 0 {
                log::warn!(
                    "PR_SET_KEEPCAPS failed: {}",
                    std::io::Error::last_os_error()
                );
            }
        }

        let supp_gids: Vec<nix::unistd::Gid> = config
            .supplementary_groups
            .iter()
            .map(|gid| nix::unistd::Gid::from_raw(*gid))
            .collect();
        // Every PrivateUsers= mode now maps ids through unchanged: yes/self
        // identity-maps the caller's id and the service's id (see the uid_map
        // above), identity and full map whole ranges. So the service always
        // drops to its real uid/gid.
        //
        // This used to drop to (0, 0) for yes/self, because the map written
        // then was "0 <uid> 1" and the service's own id was not representable.
        // With the map corrected that workaround is wrong: it would leave the
        // service running as namespace root instead of its own identity.
        let (drop_uid, drop_gid, drop_supp) = (config.user, config.group, supp_gids.clone());

        match crate::platform::drop_privileges(
            nix::unistd::Gid::from_raw(drop_gid),
            &drop_supp,
            nix::unistd::Uid::from_raw(drop_uid),
        ) {
            Ok(()) => {
                log::trace!(
                    "privilege drop complete (now uid={}, gid={})",
                    nix::unistd::getuid(),
                    nix::unistd::getgid()
                );
            }
            Err(e) => {
                log::error!("could not drop privileges because: {}", e);
                std::process::exit(1);
            }
        }

        // After dropping privileges, raise the requested ambient
        // capabilities.  We must first re-add each cap to the permitted
        // and effective sets (PR_SET_KEEPCAPS only preserves the permitted
        // set; the effective set is cleared on setuid).  Then we can raise
        // the cap as ambient.
        //
        // We use raw structs matching <linux/capability.h> because the
        // libc crate doesn't expose __user_cap_header_struct /
        // __user_cap_data_struct.
        #[repr(C)]
        struct CapHeader {
            version: u32,
            pid: i32,
        }
        #[repr(C)]
        struct CapData {
            effective: u32,
            permitted: u32,
            inheritable: u32,
        }
        const CAP_V3: u32 = 0x20080522; // _LINUX_CAPABILITY_VERSION_3

        if !ambient_caps.is_empty() {
            // Read current capability sets (version 3 uses two CapData
            // elements covering caps 0-31 and 32-63).
            let mut hdr = CapHeader {
                version: CAP_V3,
                pid: 0,
            };
            let mut data: [CapData; 2] = unsafe { std::mem::zeroed() };

            if unsafe { libc::syscall(libc::SYS_capget, &mut hdr as *mut _, data.as_mut_ptr()) }
                != 0
            {
                log::warn!("capget failed: {}", std::io::Error::last_os_error());
            } else {
                // Set all requested caps in permitted, effective, AND
                // inheritable sets (ambient requires inheritable too).
                for &cap in &ambient_caps {
                    let idx = (cap / 32) as usize;
                    let bit = 1u32 << (cap % 32);
                    if idx < 2 {
                        data[idx].permitted |= bit;
                        data[idx].effective |= bit;
                        data[idx].inheritable |= bit;
                    }
                }

                hdr.version = CAP_V3;
                hdr.pid = 0;
                if unsafe { libc::syscall(libc::SYS_capset, &hdr as *const _, data.as_ptr()) } != 0
                {
                    log::warn!("capset failed: {}", std::io::Error::last_os_error());
                }
            }

            // Now raise each cap as ambient.
            for &cap in &ambient_caps {
                let ret = unsafe {
                    libc::prctl(
                        libc::PR_CAP_AMBIENT,
                        libc::PR_CAP_AMBIENT_RAISE,
                        cap as libc::c_ulong,
                        0,
                        0,
                    )
                };
                if ret != 0 {
                    log::warn!(
                        "PR_CAP_AMBIENT_RAISE failed for cap {}: {}",
                        cap,
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    }

    log::trace!("privilege drop + caps complete, preparing exec args...");

    // ── '|' prefix: login shell wrapping ─────────────────────────────
    // When the '|' prefix is used, the original command is wrapped into:
    //   <login-shell> -el -c "<cmd> <args...>"
    // The shell is looked up from the effective user's passwd entry.
    let (mut effective_cmd, mut effective_args);
    if config.login_shell {
        let shell = get_login_shell(config.user);
        let shell_name = Path::new(&shell)
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("sh"))
            .to_string_lossy()
            .into_owned();

        // Build the original command string for -c
        let mut cmd_str = config.cmd.to_string_lossy().into_owned();
        for arg in &config.args {
            cmd_str.push(' ');
            cmd_str.push_str(arg);
        }

        effective_cmd = PathBuf::from(&shell);
        effective_args = vec![
            format!("-{shell_name}"), // argv[0] = "-bash" (login shell indicator)
            "-c".to_owned(),
            cmd_str,
        ];
        log::trace!(
            "login shell wrapping: {} -el -c {:?}",
            shell,
            &effective_args[2]
        );
    } else {
        effective_cmd = config.cmd.clone();
        effective_args = config.args.clone();
    }

    // NOTE: env var expansion and prepare_exec_args are deferred to right
    // before execvp (below), after all environment variables are set.

    // change working directory if configured
    if let Some(ref dir) = config.working_directory {
        let dir = if dir == Path::new("~") {
            // WorkingDirectory=~ resolves to the home directory of the unit's
            // user.  DynamicUser=yes is incompatible with `~` (a dynamic user
            // context has no home directory), so upstream systemd rejects the
            // combination even when an explicit User= is also given — fail the
            // service rather than silently falling back to a resolved HOME.
            if config.dynamic_user {
                log::error!(
                    "WorkingDirectory=~ is not valid with DynamicUser=yes: no home directory"
                );
                std::process::exit(200); // EXIT_CHDIR
            }
            // Resolve ~ to the home directory from config.env (which is
            // populated from User= in start_service) or fall back to the
            // process environment.
            let home = config
                .env
                .iter()
                .find(|(k, _)| k == "HOME")
                .map(|(_, v)| v.clone())
                .or_else(|| std::env::var("HOME").ok());
            match home {
                Some(h) => PathBuf::from(h),
                None => {
                    log::error!("WorkingDirectory=~ but HOME is not set");
                    std::process::exit(1);
                }
            }
        } else {
            dir.clone()
        };
        if let Err(e) = std::env::set_current_dir(&dir) {
            log::error!("Failed to set working directory to {:?}: {}", dir, e);
            std::process::exit(1);
        }
        // Update the inherited PWD environment variable to match the new
        // working directory.  Many shells (bash, dash) consult $PWD to
        // initialise their internal $PWD instead of calling getcwd(3); a
        // stale PWD inherited from PID 1 would otherwise persist into the
        // child even though the chdir succeeded.  Mirrors upstream
        // systemd's exec_invoke_setup_keyring → pwd handling.
        unsafe {
            std::env::set_var("PWD", &dir);
        }
    } else {
        // No WorkingDirectory= configured: system services default to running
        // in the root directory.  chdir there and export $PWD="/" so shells
        // (which read $PWD instead of calling getcwd) observe "/" rather than a
        // stale or empty PWD inherited from PID 1.  Matches upstream systemd,
        // which runs services from "/" unless WorkingDirectory= says otherwise.
        let _ = std::env::set_current_dir("/");
        unsafe {
            std::env::set_var("PWD", "/");
        }
    }

    // setup environment vars. The ':' prefix does NOT affect these: it only
    // suppresses substitution into the command line further down.
    for (k, v) in &config.env {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(k, v) };
    }

    // Only set LISTEN_PID when LISTEN_FDS is present in the environment.
    // Real systemd only sets LISTEN_PID for socket-activated services.
    // Setting it unconditionally confuses services like systemd-logind that
    // call sd_listen_fds_with_names() and get unexpected results when
    // LISTEN_PID matches but there are no actual FDs to receive.
    // TODO: Audit that the environment access only happens in single-threaded code.
    if std::env::var("LISTEN_FDS").is_ok() {
        unsafe { std::env::set_var("LISTEN_PID", format!("{}", nix::unistd::getpid())) };
    }

    // Stamp WATCHDOG_PID with the exec'd PID whenever WATCHDOG_USEC was
    // exported (WatchdogSec= is set), so sd_watchdog_enabled() — which returns
    // 0 unless $WATCHDOG_PID is unset or equals getpid() — recognises this
    // process as the watchdog owner. Mirrors the LISTEN_PID handling above.
    if std::env::var("WATCHDOG_USEC").is_ok() {
        unsafe { std::env::set_var("WATCHDOG_PID", format!("{}", nix::unistd::getpid())) };
    }

    // Stamp SYSTEMD_EXEC_PID with the exec'd PID for every service (upstream
    // exec-invoke.c sets it unconditionally to exec_pid). sd_notify() uses it
    // to reject notifications forwarded from a process other than the one the
    // manager expects. getpid() here is the process about to execve, which is
    // also the pid the manager tracks — and returns the namespace-local pid
    // (1) automatically inside a PID namespace, matching upstream.
    unsafe { std::env::set_var("SYSTEMD_EXEC_PID", format!("{}", nix::unistd::getpid())) };

    // Reset the signal mask so the child starts with all signals unblocked.
    // The service manager (PID 1) may block signals via sigprocmask (e.g.
    // signal-hook blocks SIGCHLD, SIGTERM, etc. for its iterator thread),
    // and children inherit the blocked mask across fork+exec.  Real systemd
    // calls reset_signal_mask() in exec_child() for the same reason.
    unsafe {
        let mut all_signals: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all_signals);
        libc::sigprocmask(libc::SIG_UNBLOCK, &all_signals, std::ptr::null_mut());
    }

    // Reset all signal handlers to their default dispositions.  After fork
    // the child inherits the parent's handlers, and after the first exec
    // (into this exec-helper) custom handlers are reset to SIG_DFL by the
    // kernel.  However, signals set to SIG_IGN survive exec.  Explicitly
    // restoring SIG_DFL for commonly-inherited ignored signals ensures
    // services see the expected default behaviour.  (SIGPIPE is handled
    // separately below via IgnoreSIGPIPE=.)
    unsafe {
        for sig in [
            libc::SIGHUP,
            libc::SIGUSR1,
            libc::SIGUSR2,
            libc::SIGTERM,
            libc::SIGINT,
        ] {
            libc::signal(sig, libc::SIG_DFL);
        }
    }

    // Apply IgnoreSIGPIPE= setting. When true (the default), set SIGPIPE to
    // SIG_IGN so that writes to broken pipes produce EPIPE errors instead of
    // killing the process. When false, restore the default disposition.
    unsafe {
        if config.ignore_sigpipe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        } else {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }

    // Write utmp/wtmp login record if UtmpIdentifier= is set.
    if config.utmp_identifier.is_some() {
        write_utmp_record(&config);
    }

    // ── Personality= — set the execution domain ──────────────────────
    if let Some(ref personality_str) = config.personality
        && !config.privileged_prefix
    {
        // Map personality string to libc constant.
        // On x86-64, both "x86-64" and "x86" map to PER_LINUX variants;
        // systemd supports many architectures but we handle the common ones.
        let per = match personality_str.as_str() {
            "x86-64" | "x86_64" => Some(0x0000u64), // PER_LINUX
            "x86" => Some(0x0008u64),               // PER_LINUX32
            "s390x" => Some(0x0000u64),             // PER_LINUX
            "s390" => Some(0x0008u64),              // PER_LINUX32
            "ppc64" => Some(0x0000u64),             // PER_LINUX
            "ppc64le" => Some(0x0000u64),           // PER_LINUX
            "ppc" => Some(0x0008u64),               // PER_LINUX32
            "arm64" | "aarch64" => Some(0x0000u64), // PER_LINUX
            "arm" => Some(0x0008u64),               // PER_LINUX32
            _ => {
                log::warn!("Unknown Personality={}, ignoring", personality_str);
                None
            }
        };
        if let Some(domain) = per {
            let ret = unsafe { libc::personality(domain as libc::c_ulong) };
            if ret == -1 {
                log::warn!(
                    "Failed to set Personality={}: {}",
                    personality_str,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // ── LockPersonality= — lock the execution domain ──────────────────
    if config.lock_personality && !config.privileged_prefix {
        // When Personality= is not explicitly set, reset to PER_LINUX (0x0000)
        // to ensure we're in the default execution domain. When Personality=
        // is set, the domain was already configured above — just let
        // NoNewPrivileges prevent personality() changes after exec.
        if config.personality.is_none() {
            let ret = unsafe { libc::personality(0x0000) };
            if ret == -1 {
                log::warn!(
                    "Failed to set personality for LockPersonality=: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // ── RestrictRealtime= — prevent realtime scheduling ───────────────
    // Enforced via a seccomp filter on sched_setscheduler() installed just
    // before execve (install_restrict_realtime), matching systemd. Nothing to
    // do here.

    // ── NoNewPrivileges= — must be applied last before exec ───────────
    // This is a one-way flag: once set, it cannot be unset, and it prevents
    // execve() from granting new privileges (setuid bits, file capabilities).
    // It must be set after all other privilege operations are complete.
    // Skipped when the '+' prefix is used (full privileges mode).
    // Also skipped when '!' or '!!' prefix is used, since those are
    // specifically meant to allow SUID/file-capability privilege elevation.
    if config.no_new_privileges && !config.privileged_prefix {
        let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
        if ret != 0 {
            log::error!(
                "Failed to set NoNewPrivileges: {}",
                std::io::Error::last_os_error()
            );
            std::process::exit(1);
        }
    }

    log::trace!(
        "about to execv {} (uid={}, gid={}, env_count={})",
        effective_cmd.display(),
        nix::unistd::getuid(),
        nix::unistd::getgid(),
        std::env::vars().count()
    );

    // Last chance to notice that a service cannot reach its own exec
    // directories: this runs inside the finished mount namespace and after the
    // privilege drop, so it sees exactly what the service will.  A service
    // silently failing because its StateDirectory= is unreachable is very hard
    // to diagnose from the outside, so this warns rather than staying quiet.
    for dir in &config.exec_dir_paths {
        if let Err(e) = std::fs::metadata(dir) {
            log::warn!(
                "exec dir {dir} is unreachable as uid={} gid={}: {e}",
                nix::unistd::getuid(),
                nix::unistd::getgid(),
            );
            // Walk the ancestors to show which component denied access.
            let mut ancestors: Vec<&Path> = Path::new(dir).ancestors().collect();
            ancestors.reverse();
            for a in ancestors {
                match std::fs::symlink_metadata(a) {
                    Ok(md) => {
                        use std::os::unix::fs::MetadataExt;
                        log::warn!(
                            "  {} mode={:o} uid={} gid={} symlink={}",
                            a.display(),
                            md.mode() & 0o7777,
                            md.uid(),
                            md.gid(),
                            md.file_type().is_symlink()
                        );
                    }
                    Err(e) => log::warn!("  {}: {e}", a.display()),
                }
            }
        }
    }

    // Verify the binary exists and is readable before exec
    log::trace!(
        "cmd exists={}, is_file={}",
        config.cmd.exists(),
        config.cmd.is_file()
    );

    // Check that essential paths are accessible
    log::trace!(
        "/dev/null exists={}, /dev/urandom exists={}, /proc exists={}",
        Path::new("/dev/null").exists(),
        Path::new("/dev/urandom").exists(),
        Path::new("/proc").exists()
    );

    // ── Apply all LimitXXX= resource limits just before exec ──────────
    // Applied last so restrictive limits (e.g. LimitNOFILE=7) don't
    // prevent the exec helper from opening files during setup.
    apply_resource_limit("RLIMIT_NOFILE", libc::RLIMIT_NOFILE, &config.limit_nofile);
    apply_resource_limit("RLIMIT_CPU", libc::RLIMIT_CPU, &config.limit_cpu);
    apply_resource_limit("RLIMIT_CORE", libc::RLIMIT_CORE, &config.limit_core);
    apply_resource_limit("RLIMIT_FSIZE", libc::RLIMIT_FSIZE, &config.limit_fsize);
    apply_resource_limit("RLIMIT_DATA", libc::RLIMIT_DATA, &config.limit_data);
    apply_resource_limit("RLIMIT_STACK", libc::RLIMIT_STACK, &config.limit_stack);
    apply_resource_limit("RLIMIT_RSS", libc::RLIMIT_RSS, &config.limit_rss);
    apply_resource_limit("RLIMIT_NPROC", libc::RLIMIT_NPROC, &config.limit_nproc);
    apply_resource_limit(
        "RLIMIT_MEMLOCK",
        libc::RLIMIT_MEMLOCK,
        &config.limit_memlock,
    );
    apply_resource_limit("RLIMIT_AS", libc::RLIMIT_AS, &config.limit_as);
    apply_resource_limit("RLIMIT_LOCKS", libc::RLIMIT_LOCKS, &config.limit_locks);
    apply_resource_limit(
        "RLIMIT_SIGPENDING",
        libc::RLIMIT_SIGPENDING,
        &config.limit_sigpending,
    );
    apply_resource_limit(
        "RLIMIT_MSGQUEUE",
        libc::RLIMIT_MSGQUEUE,
        &config.limit_msgqueue,
    );
    apply_resource_limit("RLIMIT_NICE", libc::RLIMIT_NICE, &config.limit_nice);
    apply_resource_limit("RLIMIT_RTPRIO", libc::RLIMIT_RTPRIO, &config.limit_rtprio);
    apply_resource_limit("RLIMIT_RTTIME", libc::RLIMIT_RTTIME, &config.limit_rttime);

    // Perform environment variable expansion on command arguments, matching
    // real systemd's replace_env_argv() behavior. $FOO and ${FOO} are expanded
    // using the process environment (which includes CREDENTIALS_DIRECTORY,
    // STATE_DIRECTORY, EnvironmentFile= vars, etc.). $$ becomes literal $.
    // This must happen AFTER all env vars are set (config.env, directory vars,
    // credentials, etc.) so that expansion sees the complete environment.
    if !config.no_env_expand {
        effective_args = expand_env_argv(&effective_args);
        effective_cmd = PathBuf::from(expand_env_str(&effective_cmd.to_string_lossy()));
    }

    let (cmd, args) = prepare_exec_args(
        &effective_cmd,
        &effective_args,
        config.login_shell || config.use_first_arg_as_argv0,
    );

    // SystemCallFilter= seccomp filter: installed as the very last step before
    // execve, so nothing here needs a syscall an allow-list would block (the
    // exec-dir reachability check above runs unconfined, and @default includes
    // execve). Using any filter implies NoNewPrivileges.
    install_system_call_filter(&config);
    // SystemCallLog= stacks a non-blocking audit-logging filter on top.
    install_system_call_log(&config);
    // SystemCallArchitectures= restricts which CPU architectures may be used.
    install_system_call_architectures(&config);
    // RestrictAddressFamilies= filters socket(2) by address family.
    install_restrict_address_families(&config);
    // MemoryDenyWriteExecute= blocks creating writable+executable memory.
    install_memory_deny_write_execute(&config);
    // RestrictRealtime= denies realtime scheduling policies.
    install_restrict_realtime(&config);
    // RestrictSUIDSGID= blocks setting the setuid/setgid mode bits.
    install_restrict_suid_sgid(&config);
    // Protect{KernelModules,Clock,KernelLogs,KernelTunables}= seccomp complements
    // to their mount-namespace protections (block the module/clock/klog/sysctl
    // syscalls).
    install_protect_kernel_modules(&config);
    install_protect_clock(&config);
    install_protect_kernel_logs(&config);
    install_protect_kernel_tunables(&config);

    // Absolute paths go through execv so that unreadable shebangs / empty
    // exec files fail with ENOEXEC instead of being auto-shelled by execvp
    // (POSIX execvp's shell fallback would mask the error and make failing
    // exec look like success). Bare command names still use execvp so they
    // can be resolved via PATH, matching real systemd's behavior.
    let exec_result = if cmd.to_bytes().first() == Some(&b'/') {
        nix::unistd::execv(&cmd, &args)
    } else {
        nix::unistd::execvp(&cmd, &args)
    };
    match exec_result {
        Ok(_infallible) => unreachable!(),
        Err(e) => {
            log::error!("exec FAILED for {}: {}", cmd.to_string_lossy(), e,);
            // Use EXIT_EXEC (203) so that the Type=exec check in
            // wait_for_service can distinguish exec failures from
            // normal program exits (which forward the program's own
            // exit code).
            std::process::exit(203);
        }
    }
}

/// Set up a mount namespace with the requested isolation directives.
/// Called before privilege drop. Requires root or CAP_SYS_ADMIN.
fn setup_mount_namespace(config: &ExecHelperConfig) {
    // Mint the namespace the exec directories will be id-mapped through, before
    // any of them are bound. Only needed when the service both runs under its
    // own user namespace and has directories owned by an id that namespace does
    // not map.
    if config.private_users
        && !config.privileged_prefix
        && config.private_users_mode != "identity"
        && config.private_users_mode != "full"
        && !config.exec_dir_binds.is_empty()
        && IDMAP_USERNS.get().is_none()
        && let Some(fd) = create_mapped_userns(config.user, config.group)
    {
        let _ = IDMAP_USERNS.set(fd);
    }

    log::trace!("mount_ns: unshare(CLONE_NEWNS)...");
    // Create a new mount namespace
    let ret = unsafe { libc::unshare(libc::CLONE_NEWNS) };
    if ret != 0 {
        log::warn!(
            "Failed to create mount namespace: {}",
            std::io::Error::last_os_error()
        );
        return; // Non-fatal: continue without mount isolation
    }

    // MountFlags= — set mount propagation type.
    // "shared" = MS_SHARED, "slave" = MS_SLAVE (default), "private" = MS_PRIVATE.
    let mount_prop_flag = match config.mount_flags.as_deref() {
        Some("shared") => libc::MS_SHARED,
        Some("private") => libc::MS_PRIVATE,
        _ => libc::MS_SLAVE, // default
    };
    log::trace!(
        "mount_ns: making / {:?}...",
        config.mount_flags.as_deref().unwrap_or("slave")
    );
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            mount_prop_flag | libc::MS_REC,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to make / rslave: {}",
            std::io::Error::last_os_error()
        );
        return;
    }

    log::trace!("mount_ns: ProtectSystem={}...", config.protect_system);
    // Create any missing TemporaryFileSystem= mount points BEFORE anything is
    // remounted read-only. Upstream mkdir -p's every mount entry's path, so
    // TemporaryFileSystem=/vol works on a host with no /vol; doing it later
    // fails with EROFS once ProtectSystem= has made / read-only, and the mount
    // then fails with ENOENT. Creating them here is safe because we are already
    // in a private mount namespace, so they never appear on the host.
    for entry in &config.temporary_file_system {
        let path = entry.split_once(':').map(|(p, _)| p).unwrap_or(entry);
        if !path.is_empty()
            && !Path::new(path).exists()
            && let Err(e) = std::fs::create_dir_all(path)
        {
            log::warn!("Failed to create TemporaryFileSystem mount point {path}: {e}");
        }
    }

    // ── ProtectSystem= ────────────────────────────────────────────────
    match config.protect_system.as_str() {
        "yes" => {
            // Make /usr and /boot read-only
            remount_read_only("/usr", config);
            remount_read_only("/boot", config);
            remount_read_only("/efi", config);
        }
        "full" => {
            // Make /usr, /boot, and /etc read-only
            remount_read_only("/usr", config);
            remount_read_only("/boot", config);
            remount_read_only("/efi", config);
            remount_read_only("/etc", config);
        }
        "strict" => {
            // Make the entire root filesystem read-only (recursively).
            remount_read_only("/", config);
            // Restore write access to exactly what upstream's
            // protect_system_strict_table restores and no more: the API
            // filesystems, plus the home-ish paths that ProtectHome= then
            // re-protects on its own further down.
            //
            // /run, /tmp, /var/tmp and /var/log are deliberately NOT here.
            // Restoring them left a strict service able to write all over the
            // runtime and log trees, which is the opposite of what the setting
            // promises; a service that needs one of those gets it back through
            // RuntimeDirectory=/LogsDirectory= or ReadWritePaths=, both of
            // which are applied after this.
            for rw_path in &["/proc", "/sys", "/dev", "/home", "/run/user", "/root"] {
                if Path::new(rw_path).exists() {
                    bind_mount_readwrite(rw_path, config);
                }
            }
        }
        _ => {} // "no" or unrecognized
    }

    log::trace!("mount_ns: ProtectSystem done, implicit RW paths...");
    // ── Implicit ReadWritePaths from RuntimeDirectory=/StateDirectory=/LogsDirectory=
    // When ProtectSystem=strict is active, the service's runtime, state, and
    // logs directories must be explicitly writable. systemd handles this
    // implicitly; we do the same.
    if config.protect_system == "strict" {
        for full in &config.exec_dir_paths {
            if Path::new(full).exists() {
                bind_mount_readwrite(full, config);
            }
        }
    }

    log::trace!("mount_ns: implicit RW paths done, ReadWritePaths...");
    // ── ReadWritePaths= — re-mount paths read-write ───────────────────
    // Applied after ProtectSystem= so they can override read-only mounts.
    for path in &config.read_write_paths {
        if Path::new(path).exists() {
            bind_mount_readwrite(path, config);
        }
    }

    log::trace!(
        "mount_ns: ReadWritePaths done, ProtectHome={}...",
        config.protect_home
    );
    // ── ProtectHome= ──────────────────────────────────────────────────
    match config.protect_home.as_str() {
        "yes" => {
            // Make /home, /root, /run/user inaccessible
            make_inaccessible("/home", config);
            make_inaccessible("/root", config);
            make_inaccessible("/run/user", config);
        }
        "read-only" => {
            remount_read_only("/home", config);
            remount_read_only("/root", config);
            remount_read_only("/run/user", config);
        }
        "tmpfs" => {
            mount_tmpfs_readonly("/home", config);
            mount_tmpfs_readonly("/root", config);
            mount_tmpfs_readonly("/run/user", config);
        }
        _ => {} // "no" or unrecognized
    }

    log::trace!(
        "mount_ns: ProtectHome done, PrivateTmp={}...",
        config.private_tmp
    );
    // ── PrivateTmp= ───────────────────────────────────────────────────
    if config.private_tmp {
        mount_tmpfs("/tmp", config);
        mount_tmpfs("/var/tmp", config);
    }

    log::trace!(
        "mount_ns: PrivateTmp done, PrivateDevices={}...",
        config.private_devices
    );
    // ── PrivateDevices= ───────────────────────────────────────────────
    if config.private_devices {
        // Capture device major/minor numbers BEFORE mounting tmpfs,
        // because mount_tmpfs will hide the original /dev contents.
        let dev_info = capture_dev_info();
        // Mount a minimal tmpfs on /dev.  We must NOT use MS_NODEV here
        // (unlike mount_tmpfs which is fine for /tmp), because device nodes
        // created via mknod below must actually function.  Real systemd
        // uses MS_STRICTATIME only, with mode=0755.
        {
            let ret = unsafe {
                libc::mount(
                    c"tmpfs".as_ptr(),
                    c"/dev".as_ptr(),
                    c"tmpfs".as_ptr(),
                    libc::MS_NOSUID | libc::MS_STRICTATIME,
                    c"mode=0755,size=4m".as_ptr().cast(),
                )
            };
            if ret != 0 {
                log::warn!(
                    "Failed to mount tmpfs on /dev for PrivateDevices=: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
        // Re-create essential pseudo-device nodes using mknod
        create_private_dev_nodes(config, &dev_info);
        log::trace!("mount_ns: PrivateDevices tmpfs + mknod done");
    }

    log::trace!(
        "mount_ns: ProtectKernelTunables={}...",
        config.protect_kernel_tunables
    );
    // ── ProtectKernelTunables= ────────────────────────────────────────
    if config.protect_kernel_tunables {
        log::trace!("mount_ns: remount_read_only /proc/sys...");
        remount_read_only("/proc/sys", config);
        log::trace!("mount_ns: remount_read_only /sys...");
        remount_read_only("/sys", config);
        log::trace!("mount_ns: /sys done, making tunable paths inaccessible...");
        // Additional tunable paths
        make_inaccessible_if_exists("/proc/sysrq-trigger", config);
        make_inaccessible_if_exists("/proc/latency_stats", config);
        make_inaccessible_if_exists("/proc/acpi", config);
        make_inaccessible_if_exists("/proc/timer_stats", config);
        make_inaccessible_if_exists("/proc/fs", config);
        make_inaccessible_if_exists("/proc/irq", config);
        log::trace!("mount_ns: ProtectKernelTunables done");
    }

    log::trace!(
        "mount_ns: ProtectKernelModules={}...",
        config.protect_kernel_modules
    );
    // ── ProtectKernelModules= ─────────────────────────────────────────
    if config.protect_kernel_modules {
        make_inaccessible_if_exists("/usr/lib/modules", config);
        make_inaccessible_if_exists("/lib/modules", config);
    }

    log::trace!(
        "mount_ns: ProtectKernelLogs={}...",
        config.protect_kernel_logs
    );
    // ── ProtectKernelLogs= ────────────────────────────────────────────
    if config.protect_kernel_logs {
        log::trace!(
            "mount_ns: ProtectKernelLogs: /dev/kmsg exists={}, /proc/kmsg exists={}, /dev/null exists={}",
            Path::new("/dev/kmsg").exists(),
            Path::new("/proc/kmsg").exists(),
            Path::new("/dev/null").exists(),
        );
        log::trace!("mount_ns: ProtectKernelLogs: about to make /dev/kmsg inaccessible...");
        make_inaccessible_if_exists("/dev/kmsg", config);
        log::trace!(
            "mount_ns: ProtectKernelLogs: /dev/kmsg done, about to make /proc/kmsg inaccessible..."
        );
        make_inaccessible_if_exists("/proc/kmsg", config);
        log::trace!("mount_ns: ProtectKernelLogs: /proc/kmsg done, ProtectKernelLogs complete");
    }

    log::trace!(
        "mount_ns: ProtectControlGroups={} ProtectControlGroupsEx={}...",
        config.protect_control_groups,
        config.protect_control_groups_ex
    );
    // ── ProtectControlGroups= / ProtectControlGroupsEx= ──────────────
    // ProtectControlGroupsEx supersedes ProtectControlGroups when set.
    match config.protect_control_groups_ex.as_str() {
        "yes" | "private" | "strict" => {
            // Create new cgroup namespace for private/strict
            if config.protect_control_groups_ex != "yes" {
                // ProtectControlGroupsEx=private + DelegateSubgroup=: post_fork
                // already moved us into the subgroup, but the new cgroup namespace
                // must root at the DELEGATED cgroup so the subgroup is visible
                // under /sys/fs/cgroup. Move back to the delegated root before
                // unsharing; we re-enter the subgroup after the fresh mount below.
                if config.protect_control_groups_ex == "private"
                    && config.platform_specific.delegate_subgroup.is_some()
                {
                    let _ = crate::platform::cgroups::move_self_to_cgroup(
                        &config.platform_specific.cgroup_path,
                    );
                }
                let ret = unsafe { libc::unshare(libc::CLONE_NEWCGROUP) };
                if ret != 0 {
                    log::warn!(
                        "mount_ns: unshare(CLONE_NEWCGROUP) failed: {}",
                        std::io::Error::last_os_error()
                    );
                }
            }
            // Remount /sys/fs/cgroup with a fresh cgroup2 mount.
            // For private/strict: in the new namespace, the service sees itself at root.
            // For yes: host namespace, but cgroup hierarchy becomes read-only.
            let cgroup_path = std::ffi::CString::new("/sys/fs/cgroup").unwrap();
            let cgroup2 = std::ffi::CString::new("cgroup2").unwrap();
            let mount_data = std::ffi::CString::new("nsdelegate,memory_recursiveprot").unwrap();
            // Unmount the old cgroup mount
            unsafe { libc::umount2(cgroup_path.as_ptr(), libc::MNT_DETACH) };
            // Mount fresh cgroup2 with appropriate flags
            let mount_flags = libc::MS_NOSUID
                | libc::MS_NODEV
                | libc::MS_NOEXEC
                | if config.protect_control_groups_ex != "private" {
                    libc::MS_RDONLY // yes and strict are read-only
                } else {
                    0 // private is read-write
                };
            let ret = unsafe {
                libc::mount(
                    cgroup2.as_ptr(),
                    cgroup_path.as_ptr(),
                    cgroup2.as_ptr(),
                    mount_flags as libc::c_ulong,
                    mount_data.as_ptr() as *const libc::c_void,
                )
            };
            if ret != 0 {
                log::warn!(
                    "mount_ns: mount cgroup2 failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            // Re-enter the DelegateSubgroup now that the cgroup namespace roots
            // at the delegated cgroup and /sys/fs/cgroup is a fresh RW cgroup2
            // mount: the subgroup is at /sys/fs/cgroup/<sub>.
            if config.protect_control_groups_ex == "private"
                && let Some(ref sub) = config.platform_specific.delegate_subgroup
            {
                let subpath = std::path::Path::new("/sys/fs/cgroup").join(sub);
                let _ = crate::platform::cgroups::move_self_to_cgroup(&subpath);
            }
        }
        _ => {
            // "no" or unset — fall back to boolean ProtectControlGroups
            if config.protect_control_groups {
                remount_read_only("/sys/fs/cgroup", config);
            }
        }
    }

    // Re-make the service's memory.pressure file writable if ProtectControlGroups=
    // just made /sys/fs/cgroup read-only. Must run AFTER the read-only remount
    // above so the read-write bind overrides it, letting the service register PSI
    // triggers (systemd achieves this by appending the path to ReadWritePaths=).
    if let Some(ref mpp) = config.memory_pressure_path {
        let cgroup_read_only = config.protect_control_groups
            || matches!(config.protect_control_groups_ex.as_str(), "yes" | "strict");
        if cgroup_read_only && Path::new(mpp).exists() {
            bind_mount_readwrite(mpp, config);
        }
    }

    log::trace!("mount_ns: ProtectClock={}...", config.protect_clock);
    // ── ProtectClock= ─────────────────────────────────────────────────
    if config.protect_clock {
        // Make clock-related device nodes inaccessible
        make_inaccessible_if_exists("/dev/rtc0", config);
        make_inaccessible_if_exists("/dev/hpet", config);
        // /dev/ptp* devices
        for entry in std::fs::read_dir("/dev").into_iter().flatten().flatten() {
            if let Some(name) = entry.file_name().to_str()
                && name.starts_with("ptp")
            {
                let path = format!("/dev/{}", name);
                make_inaccessible_if_exists(&path, config);
            }
        }
    }
    // ── ReadOnlyPaths= ────────────────────────────────────────────────
    if !config.read_only_paths.is_empty() {
        log::trace!(
            "mount_ns: ReadOnlyPaths ({} entries)...",
            config.read_only_paths.len()
        );
        for path in &config.read_only_paths {
            // Strip leading '-' prefix (makes it non-fatal if path doesn't exist)
            let (optional, path) = if let Some(stripped) = path.strip_prefix('-') {
                (true, stripped)
            } else {
                (false, path.as_str())
            };
            if Path::new(path).exists() {
                remount_read_only(path, config);
            } else if !optional {
                log::warn!("ReadOnlyPaths= path does not exist: {}", path);
            }
        }
    }

    // ── InaccessiblePaths= ────────────────────────────────────────────
    if !config.inaccessible_paths.is_empty() {
        log::trace!(
            "mount_ns: InaccessiblePaths ({} entries)...",
            config.inaccessible_paths.len()
        );
        for path in &config.inaccessible_paths {
            let (optional, path) = if let Some(stripped) = path.strip_prefix('-') {
                (true, stripped)
            } else {
                (false, path.as_str())
            };
            if Path::new(path).exists() {
                make_inaccessible(path, config);
            } else if !optional {
                log::warn!("InaccessiblePaths= path does not exist: {}", path);
            }
        }
    }

    // ── BindPaths= / BindReadOnlyPaths= / TemporaryFileSystem= ────────
    //
    // These three directives interact: TemporaryFileSystem mounts tmpfs over
    // a path (hiding original files), then BindPaths can re-expose specific
    // files on top.  To allow BindPaths sources that live under a
    // TemporaryFileSystem path we open O_PATH file descriptors to sources
    // BEFORE the tmpfs mounts, then use /proc/self/fd/N as the mount source.
    //
    // Order: 1) open source FDs, 2) mount tmpfs, 3) bind-mount via FDs.

    // Collect bind-path entries with pre-opened source FDs.
    struct BindEntry {
        source_fd: Option<std::os::unix::io::RawFd>,
        source_path: String,
        dest: String,
        recursive: bool,
        is_dir: bool,
        read_only: bool,
    }

    let mut bind_entries: Vec<BindEntry> = Vec::new();

    // Helper: parse a bind-path spec and open source FD
    let parse_bind = |entry: &str, read_only: bool| -> Option<BindEntry> {
        let (entry, optional) = if let Some(stripped) = entry.strip_prefix('-') {
            (stripped, true)
        } else {
            (entry, false)
        };
        let parts: Vec<&str> = entry.splitn(3, ':').collect();
        let source = parts[0];
        let dest = if parts.len() > 1 { parts[1] } else { source };
        let recursive = parts.len() > 2 && parts[2].contains("rbind");

        let source_path = Path::new(source);
        if !source_path.exists() {
            if !optional {
                let kind = if read_only {
                    "BindReadOnlyPaths"
                } else {
                    "BindPaths"
                };
                log::warn!("{kind}= source does not exist: {source}");
            }
            return None;
        }
        let is_dir = source_path.is_dir();
        // Open an O_PATH fd so we can reference this source after tmpfs mounts.
        let c_src = std::ffi::CString::new(source).ok()?;
        let fd = unsafe { libc::open(c_src.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        Some(BindEntry {
            source_fd: if fd >= 0 { Some(fd) } else { None },
            source_path: source.to_string(),
            dest: dest.to_string(),
            recursive,
            is_dir,
            read_only,
        })
    };

    // Step 1: open FDs for all bind sources
    for entry in &config.bind_paths {
        if let Some(be) = parse_bind(entry, false) {
            bind_entries.push(be);
        }
    }
    for entry in &config.bind_read_only_paths {
        if let Some(be) = parse_bind(entry, true) {
            bind_entries.push(be);
        }
    }
    // Exec directories that a TemporaryFileSystem= tmpfs would hide.  Same
    // open-then-bind treatment as BindPaths=, but with source and destination
    // already split, since an exec directory name may contain a colon.
    for (source, dest) in &config.exec_dir_binds {
        if !Path::new(source).is_dir() {
            log::warn!("exec dir bind source is not a directory, skipping: {source}");
            continue;
        }
        let Ok(c_src) = std::ffi::CString::new(source.as_str()) else {
            continue;
        };
        let fd = unsafe { libc::open(c_src.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
        bind_entries.push(BindEntry {
            source_fd: if fd >= 0 { Some(fd) } else { None },
            source_path: source.clone(),
            dest: dest.clone(),
            recursive: false,
            is_dir: true,
            read_only: false,
        });
    }

    // Step 2: mount TemporaryFileSystem
    // DynamicUser=: replace the 0700 root:root `<base>/private` boundary with a
    // permissive tmpfs, into which Step 3 binds only this service's own
    // directories.  Must run after Step 1 has taken O_PATH fds on those
    // directories, since the tmpfs hides the originals.
    for path in &config.private_dir_tmpfs {
        let Ok(c_path) = std::ffi::CString::new(path.as_str()) else {
            continue;
        };
        let ret = unsafe {
            libc::mount(
                c"tmpfs".as_ptr(),
                c_path.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV,
                c"mode=0755".as_ptr().cast(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to mount tmpfs over {path}: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    // TemporaryFileSystem= paths whose `ro` option is applied after the binds.
    let mut deferred_ro_tmpfs: Vec<String> = Vec::new();
    if !config.temporary_file_system.is_empty() {
        log::trace!(
            "mount_ns: TemporaryFileSystem ({} entries)...",
            config.temporary_file_system.len()
        );
        for entry in &config.temporary_file_system {
            let (path, options) = if let Some((p, o)) = entry.split_once(':') {
                (p, o)
            } else {
                (entry.as_str(), "")
            };
            if !Path::new(path).exists() {
                let _ = std::fs::create_dir_all(path);
            }
            let c_path = match std::ffi::CString::new(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            // A read-only tmpfs has to be mounted writable first: the bind step
            // below still has to create mount points inside it for the exec
            // directories (TEST-34-DYNAMICUSERMIGRATE uses
            // `TemporaryFileSystem=/var/lib:ro` together with StateDirectory=).
            // The read-only remount is deferred until after Step 3.  It is not
            // recursive, so the exec directories bound inside stay writable,
            // matching upstream.
            let mut ro_tmpfs = false;
            let kept: Vec<&str> = options
                .split(',')
                .filter(|o| {
                    if *o == "ro" {
                        ro_tmpfs = true;
                        false
                    } else {
                        !o.is_empty()
                    }
                })
                .collect();
            if ro_tmpfs {
                deferred_ro_tmpfs.push(path.to_string());
            }
            let opts = if kept.is_empty() {
                "mode=0755".to_string()
            } else {
                format!("mode=0755,{}", kept.join(","))
            };
            let c_opts = match std::ffi::CString::new(opts.as_str()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let ret = unsafe {
                libc::mount(
                    c"tmpfs".as_ptr(),
                    c_path.as_ptr(),
                    c"tmpfs".as_ptr(),
                    libc::MS_NOSUID | libc::MS_NODEV | libc::MS_STRICTATIME,
                    c_opts.as_ptr().cast(),
                )
            };
            if ret != 0 {
                log::warn!(
                    "Failed to mount tmpfs on {}: {}",
                    path,
                    std::io::Error::last_os_error()
                );
            }
        }
    }

    // Step 3: bind-mount using saved FDs (or original paths if no tmpfs involved)
    if !bind_entries.is_empty() {
        log::trace!(
            "mount_ns: BindPaths/BindReadOnlyPaths ({} entries)...",
            bind_entries.len()
        );
    }
    for be in &bind_entries {
        // Determine source path: use /proc/self/fd/N if we have an FD
        let effective_source = if let Some(fd) = be.source_fd {
            format!("/proc/self/fd/{fd}")
        } else {
            be.source_path.clone()
        };

        // Ensure destination mount point exists
        if !Path::new(&be.dest).exists() {
            if be.is_dir {
                let _ = std::fs::create_dir_all(&be.dest);
            } else {
                if let Some(parent) = Path::new(&be.dest).parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::File::create(&be.dest);
            }
        }

        let c_src = match std::ffi::CString::new(effective_source.as_str()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let c_dest = match std::ffi::CString::new(be.dest.as_str()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let flags = if be.recursive {
            libc::MS_BIND | libc::MS_REC
        } else {
            libc::MS_BIND
        };
        let ret = unsafe {
            libc::mount(
                c_src.as_ptr(),
                c_dest.as_ptr(),
                std::ptr::null(),
                flags,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to bind-mount {} -> {}: {}",
                be.source_path,
                be.dest,
                std::io::Error::last_os_error()
            );
        } else if be.read_only {
            // Remount read-only
            let ret = unsafe {
                libc::mount(
                    std::ptr::null(),
                    c_dest.as_ptr(),
                    std::ptr::null(),
                    libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_REC,
                    std::ptr::null(),
                )
            };
            if ret != 0 {
                log::warn!(
                    "Failed to remount {} read-only: {}",
                    be.dest,
                    std::io::Error::last_os_error()
                );
            }
        }
        // Close the O_PATH fd now that the bind mount is done
        if let Some(fd) = be.source_fd {
            unsafe { libc::close(fd) };
        }
    }

    // Now that the exec directories are bound, translate their ownership
    // through the service's own user namespace: they are owned by its OUTSIDE
    // uid, which that namespace does not map, so without this they appear as
    // nobody and the service cannot write its own state directories.
    // (was: Exec directories are owned by the service's outside uid, which its own
    if let Some(userns) = IDMAP_USERNS.get() {
        use std::os::fd::AsFd;

        // Several exec directories share one underlying source: the private
        // state dir is mapped to itself and then to each visible alias.
        //
        // Each destination needs its OWN id-mapped clone. Binding the aliases
        // plainly from an already-mapped source does not work: a plain bind
        // does not carry the idmap, so writes through the alias land with the
        // service's raw uid (measured: the state file came out owned by 61221
        // instead of the mapped id) and the mapping is silently bypassed.
        //
        // But the idmap belongs to the SOURCE mount, so once the source has
        // been mapped in place, cloning it again hits is_idmapped_mnt() and
        // EPERM. The self-mapping (source == dest) is therefore done LAST,
        // leaving the source unmapped while every alias takes its own clone
        // from it.
        let mut ordered: Vec<&(String, String)> = config.exec_dir_binds.iter().collect();
        ordered.sort_by_key(|(source, dest)| u8::from(source == dest));

        let mut mapped_self: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (source, dest) in ordered {
            if source == dest && mapped_self.contains(source.as_str()) {
                continue;
            }
            match idmapped_bind(source, dest, userns.as_fd()) {
                Ok(()) => {
                    if source == dest {
                        mapped_self.insert(source.as_str());
                    }
                }
                Err(e) => log::warn!("Failed to id-map exec directory {dest}: {e}"),
            }
        }
    }

    // Did each exec-directory bind actually take? If a dest resolves to the
    // same (st_dev, st_ino) as its source, the two are one directory and a
    // write through either is visible through the other. Different values mean
    // the bind did not land and the service's writes go somewhere private,
    // succeeding silently while the host sees nothing.
    if IDMAP_USERNS.get().is_some() {
        for (source, dest) in &config.exec_dir_binds {
            let mut a: libc::stat = unsafe { std::mem::zeroed() };
            let mut b: libc::stat = unsafe { std::mem::zeroed() };
            if let (Ok(cs), Ok(cd)) = (
                std::ffi::CString::new(source.as_str()),
                std::ffi::CString::new(dest.as_str()),
            ) && unsafe { libc::stat(cs.as_ptr(), &mut a) } == 0
                && unsafe { libc::stat(cd.as_ptr(), &mut b) } == 0
                && (a.st_dev != b.st_dev || a.st_ino != b.st_ino)
            {
                crate::entrypoints::service_manager::kmsg(&format!(
                    "IDMAP bind-not-shared src={source} ({}:{}) dest={dest} ({}:{})",
                    a.st_dev, a.st_ino, b.st_dev, b.st_ino
                ));
            }
        }
    }

    // Step 4: apply the deferred `ro` on TemporaryFileSystem= entries, now that
    // the binds have created their mount points inside them.  Deliberately not
    // MS_REC: the exec directories bound in above keep their own writability,
    // which is what `TemporaryFileSystem=/var/lib:ro` plus `StateDirectory=` is
    // supposed to give you.
    for path in &deferred_ro_tmpfs {
        let Ok(c_path) = std::ffi::CString::new(path.as_str()) else {
            continue;
        };
        let ret = unsafe {
            libc::mount(
                std::ptr::null(),
                c_path.as_ptr(),
                std::ptr::null(),
                libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to remount TemporaryFileSystem {path} read-only: {}",
                std::io::Error::last_os_error()
            );
        }
    }

    log::trace!("mount_ns: ALL STEPS COMPLETE");

}

/// Bind-mount a path on top of itself with MS_RDONLY.
/// The user namespace the exec directories are id-mapped through, and which
/// PrivateUsers= then joins so the service's view matches the translation.
static IDMAP_USERNS: std::sync::OnceLock<std::os::fd::OwnedFd> = std::sync::OnceLock::new();

// ── id-mapped mounts ──────────────────────────────────────────────────────
//
// A DynamicUser= service under PrivateUsers= runs as in-namespace uid 0, which
// maps to its allocated uid outside. Its state directories are owned by that
// OUTSIDE uid, which has no mapping inside, so they show up as `nobody` and are
// unwritable. Upstream attaches them with an id-mapped mount so the ownership
// is translated; that is what mount_setattr(MOUNT_ATTR_IDMAP) does, given a
// file descriptor for the namespace to translate through.

const MOUNT_ATTR_IDMAP: u64 = 0x0010_0000;
const AT_RECURSIVE: libc::c_int = 0x8000;

#[repr(C)]
#[derive(Default)]
struct MountAttr {
    attr_set: u64,
    attr_clr: u64,
    propagation: u64,
    userns_fd: u64,
}

/// Mint a user namespace whose uid/gid maps match the ones the service will
/// run under, and return an fd for it.
///
/// The namespace has to be created in a child: a process can only write its own
/// maps once, and we need this namespace to outlive the writing while the
/// caller stays where it is. The child unshares and waits; the parent writes
/// the child's maps from outside, which is also the only way to write them
/// without the child holding privileges it does not have.
fn create_mapped_userns(uid: u32, gid: u32) -> Option<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;

    let mut ready = [-1i32; 2];
    let mut done = [-1i32; 2];
    if unsafe { libc::pipe(ready.as_mut_ptr()) } != 0 {
        return None;
    }
    if unsafe { libc::pipe(done.as_mut_ptr()) } != 0 {
        unsafe {
            libc::close(ready[0]);
            libc::close(ready[1]);
        }
        return None;
    }

    let pid = unsafe { libc::fork() };
    if pid < 0 {
        unsafe {
            libc::close(ready[0]);
            libc::close(ready[1]);
            libc::close(done[0]);
            libc::close(done[1]);
        }
        return None;
    }

    if pid == 0 {
        // Child: unshare, tell the parent, then wait for it to write the maps.
        unsafe {
            libc::close(ready[0]);
            libc::close(done[1]);
        }
        let ok = unsafe { libc::unshare(libc::CLONE_NEWUSER) } == 0;
        let byte = [u8::from(ok)];
        unsafe {
            libc::write(ready[1], byte.as_ptr().cast(), 1);
            let mut buf = [0u8; 1];
            libc::read(done[0], buf.as_mut_ptr().cast(), 1);
            libc::_exit(0);
        }
    }

    // Parent.
    unsafe {
        libc::close(ready[1]);
        libc::close(done[0]);
    }

    let mut buf = [0u8; 1];
    let got = unsafe { libc::read(ready[0], buf.as_mut_ptr().cast(), 1) };
    let unshared = got == 1 && buf[0] == 1;

    let ns_fd = if unshared {
        // Deny setgroups before gid_map, as the kernel requires.
        let _ = std::fs::write(format!("/proc/{pid}/setgroups"), "deny\n");
        // Mount-idmap direction: the kernel resolves a file's on-disk id by
        // looking it up on the INSIDE of this namespace, so the on-disk owner
        // goes there and the id the service should see goes outside. Upstream's
        // make_userns() writes the same shape for REMOUNT_IDMAPPING_HOST_OWNER:
        // "<source_owner> <uid_shift> 1".
        //
        // Written "0 <uid> 1" before, which left the on-disk id with no entry on
        // the side being looked up, so every mapped file resolved to nobody
        // (65534) and each write failed with EOVERFLOW.
        let uid_ok = std::fs::write(format!("/proc/{pid}/uid_map"), format!("{uid} 0 1\n")).is_ok();
        let gid_ok = std::fs::write(format!("/proc/{pid}/gid_map"), format!("{gid} 0 1\n")).is_ok();
        if uid_ok && gid_ok {
            let path = format!("/proc/{pid}/ns/user");
            match std::ffi::CString::new(path) {
                Ok(c) => {
                    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
                    if fd >= 0 {
                        Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) })
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        } else {
            None
        }
    } else {
        None
    };

    // Release the child and reap it.
    unsafe {
        let byte = [1u8];
        libc::write(done[1], byte.as_ptr().cast(), 1);
        libc::close(done[1]);
        libc::close(ready[0]);
        let mut status = 0;
        libc::waitpid(pid, &mut status, 0);
    }

    ns_fd
}

/// Bind `source` onto `dest` with its ownership translated through `userns_fd`.
///
/// The idmap can only be set while the mount is DETACHED: the kernel rejects
/// MOUNT_ATTR_IDMAP on a mount that is already attached, which is why this
/// cannot be a plain bind followed by mount_setattr. The sequence is therefore
/// open_tree(OPEN_TREE_CLONE) to get a detached copy, mount_setattr on that,
/// and move_mount to put it in place.
fn idmapped_bind(source: &str, dest: &str, userns_fd: std::os::fd::BorrowedFd<'_>) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    const OPEN_TREE_CLONE: libc::c_uint = 1;
    const AT_EMPTY_PATH: libc::c_int = 0x1000;
    const MOVE_MOUNT_F_EMPTY_PATH: libc::c_uint = 0x0000_0004;

    let c_src = std::ffi::CString::new(source)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let c_dest = std::ffi::CString::new(dest)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;

    // open_tree(OPEN_TREE_CLONE) on a path that is not itself a mount point
    // clones the mount CONTAINING it, which here is the whole root filesystem;
    // idmapping that is refused. Bind the directory onto itself first so the
    // clone has a mount of its own to copy.
    if !is_mount_point(source) {
        let ret = unsafe {
            libc::mount(
                c_src.as_ptr(),
                c_src.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            return Err(std::io::Error::other(format!(
                "self-bind: {}",
                std::io::Error::last_os_error()
            )));
        }
    }

    // A shared mount cannot be idmapped. Whatever propagation this inherited
    // from its parent, make this one private before cloning it.
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c_src.as_ptr(),
            std::ptr::null(),
            libc::MS_PRIVATE,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        return Err(std::io::Error::other(format!(
            "make-private: {}",
            std::io::Error::last_os_error()
        )));
    }

    let tree = unsafe {
        libc::syscall(
            libc::SYS_open_tree,
            libc::AT_FDCWD,
            c_src.as_ptr(),
            OPEN_TREE_CLONE | (libc::O_CLOEXEC as libc::c_uint),
        )
    };
    if tree < 0 {
        return Err(std::io::Error::other(format!(
            "open_tree: {}",
            std::io::Error::last_os_error()
        )));
    }
    let tree = tree as libc::c_int;
    // Owned from here on.
    let close_tree = |fd: libc::c_int| unsafe {
        libc::close(fd);
    };

    let attr = MountAttr {
        attr_set: MOUNT_ATTR_IDMAP,
        userns_fd: userns_fd.as_raw_fd() as u64,
        ..Default::default()
    };
    let ret = unsafe {
        libc::syscall(
            libc::SYS_mount_setattr,
            tree,
            c"".as_ptr(),
            AT_EMPTY_PATH | AT_RECURSIVE,
            &attr as *const MountAttr,
            std::mem::size_of::<MountAttr>(),
        )
    };
    if ret != 0 {
        let e = std::io::Error::other(format!(
            "mount_setattr: {}",
            std::io::Error::last_os_error()
        ));
        close_tree(tree);
        return Err(e);
    }

    let ret = unsafe {
        libc::syscall(
            libc::SYS_move_mount,
            tree,
            c"".as_ptr(),
            libc::AT_FDCWD,
            c_dest.as_ptr(),
            MOVE_MOUNT_F_EMPTY_PATH,
        )
    };
    let result = if ret != 0 {
        Err(std::io::Error::other(format!(
            "move_mount: {}",
            std::io::Error::last_os_error()
        )))
    } else {
        Ok(())
    };
    close_tree(tree);
    result
}

/// Whether `path` is itself a mount point, per /proc/self/mountinfo.
///
/// Field 5 of each line is the mount point. Comparing against it avoids
/// stacking a redundant bind over a mount that is already there.
fn is_mount_point(path: &str) -> bool {
    let Ok(mountinfo) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    mountinfo
        .lines()
        .filter_map(|line| line.split_whitespace().nth(4))
        .any(|mp| mp == path)
}

fn remount_read_only(path: &str, _config: &ExecHelperConfig) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !Path::new(path).exists() {
        return;
    }
    // Bind the path onto itself ONLY when it is not already a mount point.
    //
    // A path that is already a mount can be remounted read-only directly. Doing
    // the bind anyway stacks a SECOND mount over the first, and for `/` that
    // leaves the root mounted twice with contradictory flags: the original
    // read-only, the duplicate read-write, plus a duplicated subtree. Paths
    // then resolve through whichever the kernel picks, so a directory can be
    // writable to `touch` and not to access(W_OK), which is what
    // TEST-34-DYNAMICUSERMIGRATE's `find -type d -writable` calls.
    if !is_mount_point(path) {
        let ret = unsafe {
            libc::mount(
                c_path.as_ptr(),
                c_path.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND | libc::MS_REC,
                std::ptr::null(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to bind-mount {} for read-only: {}",
                path,
                std::io::Error::last_os_error()
            );
            return;
        }
    }
    // Then remount it read-only
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c_path.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REC | libc::MS_RDONLY | libc::MS_REMOUNT,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        // Some mount points (like NixOS /nix/store bind mounts) may fail
        // to remount; this is non-fatal.
        log::warn!(
            "Failed to remount {} read-only: {} (non-fatal)",
            path,
            std::io::Error::last_os_error()
        );
    }
}

/// Bind-mount a path read-write (used to override read-only mounts from ProtectSystem=strict).
/// Two steps are required: first a recursive bind mount to create a new mount point,
/// then a remount WITHOUT MS_RDONLY to clear the read-only flag that was inherited
/// from the parent's recursive read-only remount.
fn bind_mount_readwrite(path: &str, _config: &ExecHelperConfig) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    // Step 1: Bind-mount on itself (creates a new mount point we can remount)
    let ret = unsafe {
        libc::mount(
            c_path.as_ptr(),
            c_path.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REC,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to bind-mount {} for ReadWritePaths=: {}",
            path,
            std::io::Error::last_os_error()
        );
        return;
    }
    // Step 2: Remount without MS_RDONLY to make it writable.
    // MS_BIND | MS_REMOUNT (without MS_RDONLY) clears the read-only flag.
    let ret = unsafe {
        libc::mount(
            std::ptr::null(),
            c_path.as_ptr(),
            std::ptr::null(),
            libc::MS_BIND | libc::MS_REMOUNT,
            std::ptr::null(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to remount {} read-write: {} (non-fatal)",
            path,
            std::io::Error::last_os_error()
        );
    }
}

/// Mount an empty read-only tmpfs over a path (used by ProtectHome=tmpfs).
fn mount_tmpfs_readonly(path: &str, _config: &ExecHelperConfig) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !Path::new(path).exists() {
        let _ = std::fs::create_dir_all(path);
    }
    let ret = unsafe {
        libc::mount(
            c"tmpfs".as_ptr(),
            c_path.as_ptr(),
            c"tmpfs".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_STRICTATIME | libc::MS_RDONLY,
            c"mode=0755,size=0".as_ptr().cast(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to mount read-only tmpfs on {}: {}",
            path,
            std::io::Error::last_os_error()
        );
    }
}

/// Mount an empty tmpfs over a path.
fn mount_tmpfs(path: &str, _config: &ExecHelperConfig) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !Path::new(path).exists() {
        // Create the mount point if it doesn't exist
        let _ = std::fs::create_dir_all(path);
    }
    let ret = unsafe {
        libc::mount(
            c"tmpfs".as_ptr(),
            c_path.as_ptr(),
            c"tmpfs".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV | libc::MS_STRICTATIME,
            c"mode=01777,size=50%".as_ptr().cast(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to mount tmpfs on {}: {}",
            path,
            std::io::Error::last_os_error()
        );
    }
}

/// Make a path inaccessible. For directories, mount an empty tmpfs over them.
/// For files (or other non-directory entries like /proc/sysrq-trigger, /dev/kmsg),
/// bind-mount /dev/null over them. This matches real systemd's behavior which
/// uses different inaccessible sources depending on the file type.
fn make_inaccessible(path: &str, _config: &ExecHelperConfig) {
    log::trace!("make_inaccessible: enter path={path}");

    let p = Path::new(path);
    if !p.exists() {
        log::trace!("make_inaccessible: path={path} does not exist, returning");
        return;
    }
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    log::trace!("make_inaccessible: path={path} is_dir={}", p.is_dir());

    if p.is_dir() {
        // Mount an empty, unreadable tmpfs over the directory
        log::trace!("make_inaccessible: path={path} mounting empty tmpfs...");
        let ret = unsafe {
            libc::mount(
                c"tmpfs".as_ptr(),
                c_path.as_ptr(),
                c"tmpfs".as_ptr(),
                libc::MS_NOSUID | libc::MS_NODEV | libc::MS_RDONLY,
                c"mode=000,size=0".as_ptr().cast(),
            )
        };
        if ret != 0 {
            log::warn!(
                "Failed to make {} inaccessible (tmpfs): {}",
                path,
                std::io::Error::last_os_error()
            );
        }
        log::trace!("make_inaccessible: path={path} tmpfs mount ret={ret}");
    } else {
        // For files, bind-mount a zero-permission inaccessible file over
        // them so that non-root users cannot read the path.  We first try
        // the systemd-standard inaccessible node; if it doesn't exist we
        // create a temporary one.
        log::trace!("make_inaccessible: path={path} creating inaccessible file...");
        let inaccessible = "/run/systemd/inaccessible/reg";
        let _ = std::fs::create_dir_all("/run/systemd/inaccessible");
        if !Path::new(inaccessible).exists()
            && let Ok(f) = std::fs::File::create(inaccessible)
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o000));
        }
        let source = if Path::new(inaccessible).exists() {
            inaccessible
        } else {
            "/dev/null"
        };
        let c_source = match std::ffi::CString::new(source) {
            Ok(c) => c,
            Err(_) => return,
        };
        let ret = unsafe {
            libc::mount(
                c_source.as_ptr(),
                c_path.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            )
        };
        log::trace!("make_inaccessible: path={path} bind-mount {source} ret={ret}");
        if ret != 0 {
            log::warn!(
                "Failed to make {} inaccessible (bind {source}): {}",
                path,
                std::io::Error::last_os_error()
            );
        }
    }
    log::trace!("make_inaccessible: path={path} done");
}

/// Make a path inaccessible only if it exists.
fn make_inaccessible_if_exists(path: &str, config: &ExecHelperConfig) {
    if Path::new(path).exists() {
        make_inaccessible(path, config);
    }
}

/// Device info captured before mounting tmpfs on /dev.
struct DevInfo {
    /// (path, mode, rdev) for each device node that existed.
    nodes: Vec<(&'static str, libc::mode_t, libc::dev_t)>,
}

/// Capture major/minor device numbers from /dev BEFORE mounting tmpfs over it.
fn capture_dev_info() -> DevInfo {
    let devices: &[&str] = &[
        "/dev/null",
        "/dev/zero",
        "/dev/full",
        "/dev/random",
        "/dev/urandom",
        "/dev/tty",
        "/dev/kmsg",
        "/dev/console",
        "/dev/ttyS0",
    ];
    let mut nodes = Vec::new();
    for &dev in devices {
        let c_path = match std::ffi::CString::new(dev) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::stat(c_path.as_ptr(), &mut st) };
        if ret == 0 {
            // Leak the &str since we know these are static strings
            nodes.push((dev, st.st_mode, st.st_rdev));
        }
    }
    DevInfo { nodes }
}

/// Create essential device nodes in a private /dev mount using mknod.
/// Uses device numbers captured before the tmpfs was mounted.
fn create_private_dev_nodes(_config: &ExecHelperConfig, dev_info: &DevInfo) {
    // Create device nodes using mknod with captured major/minor numbers.
    // We cannot bind-mount from /dev/X because the original /dev is now
    // hidden behind the tmpfs.
    for &(dev, mode, rdev) in &dev_info.nodes {
        let c_path = match std::ffi::CString::new(dev) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // mknod with the original device type (S_IFCHR) and device numbers
        let dev_mode = (mode & libc::S_IFMT) | 0o666;
        let ret = unsafe { libc::mknod(c_path.as_ptr(), dev_mode, rdev) };
        if ret != 0 {
            log::warn!(
                "Failed to mknod {} for PrivateDevices=: {} (non-fatal)",
                dev,
                std::io::Error::last_os_error()
            );
        }
    }

    // Create symlinks for standard devices
    let symlinks = [
        ("/dev/stdin", "/proc/self/fd/0"),
        ("/dev/stdout", "/proc/self/fd/1"),
        ("/dev/stderr", "/proc/self/fd/2"),
        ("/dev/fd", "/proc/self/fd"),
    ];
    for (link, target) in &symlinks {
        let _ = std::os::unix::fs::symlink(target, link);
    }

    // Create /dev/shm and /dev/pts directories
    let _ = std::fs::create_dir_all("/dev/shm");
    let _ = std::fs::create_dir_all("/dev/pts");

    // Mount devpts on /dev/pts
    let ret = unsafe {
        libc::mount(
            c"devpts".as_ptr(),
            c"/dev/pts".as_ptr(),
            c"devpts".as_ptr(),
            libc::MS_NOSUID | libc::MS_NOEXEC,
            c"newinstance,ptmxmode=0666,mode=0620".as_ptr().cast(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to mount devpts on /dev/pts: {} (non-fatal)",
            std::io::Error::last_os_error()
        );
    }

    // Create /dev/ptmx -> pts/ptmx
    let _ = std::os::unix::fs::symlink("pts/ptmx", "/dev/ptmx");

    // Mount tmpfs on /dev/shm
    let ret = unsafe {
        libc::mount(
            c"tmpfs".as_ptr(),
            c"/dev/shm".as_ptr(),
            c"tmpfs".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV,
            c"mode=1777".as_ptr().cast(),
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to mount tmpfs on /dev/shm: {} (non-fatal)",
            std::io::Error::last_os_error()
        );
    }
}

/// Install a minimal seccomp filter denying `sethostname()`/`setdomainname()`
/// with EPERM (everything else allowed). Used by ProtectHostname=yes so the
/// service cannot change the hostname even inside its own UTS namespace even
/// while holding CAP_SYS_ADMIN, matching systemd (=private leaves them allowed).
/// Best-effort: failures are logged, not fatal.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn seccomp_block_hostname() {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const SYS_SETHOSTNAME: u32 = 170;
    const SYS_SETDOMAINNAME: u32 = 171;
    const ALLOW: u32 = 0x7fff_0000; // SECCOMP_RET_ALLOW
    let deny: u32 = 0x0005_0000 | (libc::EPERM as u32); // SECCOMP_RET_ERRNO | EPERM
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    // seccomp_data layout: nr @ offset 0, arch @ offset 4.
    let filter = [
        f(0x20, 0, 0, 4),                 // BPF_LD|W|ABS   arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64), // BPF_JEQ arch==x86_64 -> load nr, else allow
        f(0x06, 0, 0, ALLOW),             // other arch: allow (filter is x86_64-only)
        f(0x20, 0, 0, 0),                 // BPF_LD|W|ABS   nr
        f(0x15, 0, 1, SYS_SETHOSTNAME),   // nr==sethostname -> deny, else skip
        f(0x06, 0, 0, deny),
        f(0x15, 0, 1, SYS_SETDOMAINNAME), // nr==setdomainname -> deny, else skip
        f(0x06, 0, 0, deny),
        f(0x06, 0, 0, ALLOW),             // allow everything else
    ];
    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };
    unsafe {
        // NO_NEW_PRIVS is a precondition for loading a seccomp filter unprivileged
        // (and harmless as root); systemd sets it for seccomp-using services too.
        if libc::prctl(
            libc::PR_SET_NO_NEW_PRIVS,
            1 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        ) != 0
        {
            log::warn!(
                "ProtectHostname=yes: PR_SET_NO_NEW_PRIVS failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        // PR_SET_SECCOMP=22, SECCOMP_MODE_FILTER=2.
        if libc::prctl(
            libc::PR_SET_SECCOMP,
            2 as libc::c_ulong,
            &prog as *const libc::sock_fprog as usize as libc::c_ulong,
            0 as libc::c_ulong,
            0 as libc::c_ulong,
        ) != 0
        {
            log::warn!(
                "ProtectHostname=yes: seccomp filter load failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn seccomp_block_hostname() {
    log::warn!("ProtectHostname=yes seccomp hostname lock not implemented for this target");
}

/// Recursively resolve a list of SystemCallFilter tokens (concrete syscall names
/// and `@group` references) into the set of x86_64 syscall numbers they name.
/// Unknown syscall names, syscalls absent on this arch, and unknown groups are
/// skipped. `visited` guards against cyclic `@group` references.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn resolve_syscall_set(tokens: &[&str], out: &mut Vec<i64>, visited: &mut Vec<String>) {
    for tok in tokens {
        let tok = tok.trim();
        if tok.is_empty() {
            continue;
        }
        if let Some(group) = tok.strip_prefix('@') {
            if visited.iter().any(|v| v == group) {
                continue;
            }
            visited.push(group.to_owned());
            match super::seccomp_filter_sets::syscall_group(group) {
                Some(members) => resolve_syscall_set(members, out, visited),
                None => log::debug!("SystemCallFilter: unknown @group '@{group}', ignoring"),
            }
        } else if let Some(nr) = super::seccomp_filter_sets::syscall_nr(tok) {
            if !out.contains(&nr) {
                out.push(nr);
            }
        } else {
            log::debug!("SystemCallFilter: unknown or unavailable syscall '{tok}', ignoring");
        }
    }
}

/// Resolve a `SystemCallErrorNumber=` value (an errno name like "EPERM" or a
/// decimal number) to its numeric value.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn parse_syscall_errno(s: &str) -> Option<u16> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u16>() {
        return Some(n);
    }
    let n = match s {
        "EPERM" => libc::EPERM,
        "EACCES" => libc::EACCES,
        "EINVAL" => libc::EINVAL,
        "ENOSYS" => libc::ENOSYS,
        "ENOMEM" => libc::ENOMEM,
        "EIO" => libc::EIO,
        "ENOENT" => libc::ENOENT,
        "EAGAIN" => libc::EAGAIN,
        "EBADF" => libc::EBADF,
        "EBUSY" => libc::EBUSY,
        "ENOTTY" => libc::ENOTTY,
        "EOPNOTSUPP" => libc::EOPNOTSUPP,
        _ => return None,
    };
    Some(n as u16)
}

/// Build a seccomp cBPF program that allows every syscall except those in
/// `blocked`, which receive `action` (a `SECCOMP_RET_*` value). The filter is
/// x86_64-only; on any other audited arch it allows everything (matching how the
/// hostname filter is scoped).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_seccomp_deny_filter(blocked: &[i64], action: u32) -> Vec<libc::sock_filter> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const ALLOW: u32 = 0x7fff_0000; // SECCOMP_RET_ALLOW
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    // seccomp_data: nr @ offset 0, arch @ offset 4.
    let mut prog = vec![
        f(0x20, 0, 0, 4), // BPF_LD|W|ABS arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64), // JEQ arch==x86_64 -> load nr, else allow
        f(0x06, 0, 0, ALLOW), // other arch: allow (filter is x86_64-only)
        f(0x20, 0, 0, 0), // BPF_LD|W|ABS nr
    ];
    for &nr in blocked {
        // JEQ nr: on match fall through to the RET action; else skip it.
        prog.push(f(0x15, 0, 1, nr as u32));
        prog.push(f(0x06, 0, 0, action));
    }
    prog.push(f(0x06, 0, 0, ALLOW)); // default: allow everything else
    prog
}

/// Install a seccomp deny-list filter for `blocked`, returning false on failure.
/// Sets `PR_SET_NO_NEW_PRIVS` first (a precondition for an unprivileged filter,
/// and implied by using `SystemCallFilter=`).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
/// Build a seccomp cBPF program that allows only the syscalls in `allowed` and
/// applies `action` to every other syscall. Mirrors the deny-list builder but
/// with the per-syscall action being ALLOW and the fall-through default being
/// `action`. x86_64-only; other arches are allowed wholesale.
fn build_seccomp_allow_filter(allowed: &[i64], action: u32) -> Vec<libc::sock_filter> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const ALLOW: u32 = 0x7fff_0000; // SECCOMP_RET_ALLOW
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    let mut prog = vec![
        f(0x20, 0, 0, 4), // BPF_LD|W|ABS arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64), // JEQ arch==x86_64 -> load nr, else allow
        f(0x06, 0, 0, ALLOW), // other arch: allow
        f(0x20, 0, 0, 0), // BPF_LD|W|ABS nr
    ];
    for &nr in allowed {
        // JEQ nr: on match fall through to RET ALLOW; else skip it. Using jf=1
        // (not a jump to a single shared target) keeps every offset within the
        // BPF u8 limit even for large allow sets like @system-service.
        prog.push(f(0x15, 0, 1, nr as u32));
        prog.push(f(0x06, 0, 0, ALLOW));
    }
    prog.push(f(0x06, 0, 0, action)); // default: block everything not listed
    prog
}

/// Load a prebuilt seccomp filter via PR_SET_SECCOMP, setting the mandatory
/// PR_SET_NO_NEW_PRIVS first. Returns false on failure.
fn install_seccomp_program(filter: &[libc::sock_filter]) -> bool {
    let prog = libc::sock_fprog {
        len: filter.len() as u16,
        filter: filter.as_ptr() as *mut libc::sock_filter,
    };
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            log::warn!(
                "SystemCallFilter: PR_SET_NO_NEW_PRIVS failed: {}",
                std::io::Error::last_os_error()
            );
            return false;
        }
        if libc::prctl(
            libc::PR_SET_SECCOMP,
            2, // SECCOMP_MODE_FILTER
            &prog as *const libc::sock_fprog as usize as libc::c_ulong,
            0,
            0,
        ) != 0
        {
            log::warn!(
                "SystemCallFilter: seccomp filter load failed: {}",
                std::io::Error::last_os_error()
            );
            return false;
        }
    }
    true
}

/// Install a seccomp deny-list filter for `blocked`.
fn install_seccomp_deny_filter(blocked: &[i64], action: u32) -> bool {
    if blocked.is_empty() {
        return true;
    }
    install_seccomp_program(&build_seccomp_deny_filter(blocked, action))
}

/// Apply `SystemCallFilter=` for the child.
///
/// Increment 1 supports the deny-list form (`SystemCallFilter=~a b c`), which
/// defaults to allowing every syscall and blocks only the named ones, so it can
/// never accidentally break a service by under-specifying. The allow-list form
/// and `@group` sets require porting systemd's `@default`/`@system-service`/…
/// syscall-set tables and are deferred; until then those cases run without a
/// filter (permissive) rather than risk killing the service. The default action
/// for a blocked syscall is to kill the process (systemd's default), unless
/// `SystemCallErrorNumber=` selects an errno instead.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_system_call_filter(config: &ExecHelperConfig) {
    if config.system_call_filter.is_empty() {
        return;
    }
    let deny_list = config.system_call_filter[0].starts_with('~');

    // Recover the token list. A leading '~' marks the whole list as a deny-list;
    // strip it from the first token (a no-op for an allow-list).
    let tokens: Vec<&str> = config
        .system_call_filter
        .iter()
        .enumerate()
        .map(|(i, tok)| {
            if i == 0 {
                tok.strip_prefix('~').unwrap_or(tok)
            } else {
                tok.as_str()
            }
        })
        .collect();

    // Default action for a filtered syscall is to kill the process (systemd's
    // SECCOMP_ERROR_NUMBER_KILL default); SystemCallErrorNumber= overrides it.
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    let action = match config
        .system_call_error_number
        .as_deref()
        .and_then(parse_syscall_errno)
    {
        Some(errno) => SECCOMP_RET_ERRNO | (errno as u32 & 0xffff),
        None => SECCOMP_RET_KILL_PROCESS,
    };

    let mut set: Vec<i64> = Vec::new();
    let mut visited: Vec<String> = Vec::new();

    if deny_list {
        resolve_syscall_set(&tokens, &mut set, &mut visited);
        if set.is_empty() {
            return;
        }
        if install_seccomp_deny_filter(&set, action) {
            log::debug!(
                "SystemCallFilter: installed deny filter ({} syscalls) for {}",
                set.len(),
                config.name
            );
        }
    } else {
        // Allow-list: permit the listed syscalls plus the mandatory implicit
        // base. systemd always adds @default to an allow-list (load-fragment.c)
        // and `write` for the exec handoff (exec-invoke.c); everything else gets
        // the negative action. Safe because the filter is installed immediately
        // before execve with nothing in between needing a blocked syscall, and
        // @default includes execve/mmap/brk/rt_sigreturn.
        let mut allow_tokens = tokens;
        allow_tokens.push("@default");
        allow_tokens.push("write");
        resolve_syscall_set(&allow_tokens, &mut set, &mut visited);
        if set.is_empty() {
            return;
        }
        if install_seccomp_program(&build_seccomp_allow_filter(&set, action)) {
            log::debug!(
                "SystemCallFilter: installed allow filter ({} syscalls) for {}",
                set.len(),
                config.name
            );
        }
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_system_call_filter(_config: &ExecHelperConfig) {
    log::warn!("SystemCallFilter= not implemented for this target");
}

/// Apply `SystemCallLog=`: a non-blocking seccomp filter that audit-logs
/// (SECCOMP_RET_LOG) selected syscalls while still permitting them. An allow-list
/// (`SystemCallLog=a b c`) logs only the listed syscalls (listed -> LOG, default
/// ALLOW); a deny-list (`~a b c`) logs everything except them (listed -> ALLOW,
/// default LOG). Stacks with any SystemCallFilter= filter. Because LOG permits
/// the syscall, this can never break a service.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_system_call_log(config: &ExecHelperConfig) {
    if config.system_call_log.is_empty() {
        return;
    }
    const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
    let deny_list = config.system_call_log[0].starts_with('~');
    let tokens: Vec<&str> = config
        .system_call_log
        .iter()
        .enumerate()
        .map(|(i, tok)| {
            if i == 0 {
                tok.strip_prefix('~').unwrap_or(tok)
            } else {
                tok.as_str()
            }
        })
        .collect();
    let mut set: Vec<i64> = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    resolve_syscall_set(&tokens, &mut set, &mut visited);
    if set.is_empty() {
        return;
    }
    // Reuse the block-filter builders with LOG as the action: the deny-list
    // builder gives "listed -> action, default ALLOW" (the allow-list log case),
    // and the allow-list builder gives "listed -> ALLOW, default action" (the
    // deny-list log case).
    let prog = if deny_list {
        build_seccomp_allow_filter(&set, SECCOMP_RET_LOG)
    } else {
        build_seccomp_deny_filter(&set, SECCOMP_RET_LOG)
    };
    if install_seccomp_program(&prog) {
        log::debug!(
            "SystemCallLog: installed log filter ({} syscalls) for {}",
            set.len(),
            config.name
        );
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_system_call_log(_config: &ExecHelperConfig) {
    log::warn!("SystemCallLog= not implemented for this target");
}

/// Build a seccomp cBPF program that permits syscalls issued from any audited
/// architecture in `allowed` and applies `action` to all others (by inspecting
/// seccomp_data.arch, not the syscall number).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_seccomp_arch_filter(allowed: &[u32], action: u32) -> Vec<libc::sock_filter> {
    const ALLOW: u32 = 0x7fff_0000; // SECCOMP_RET_ALLOW
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    let mut prog = vec![f(0x20, 0, 0, 4)]; // BPF_LD|W|ABS arch (offset 4)
    for &arch in allowed {
        // JEQ arch==A: on match fall through to RET ALLOW; else skip it.
        prog.push(f(0x15, 0, 1, arch));
        prog.push(f(0x06, 0, 0, ALLOW));
    }
    prog.push(f(0x06, 0, 0, action)); // default: architecture not permitted
    prog
}

/// Apply `SystemCallArchitectures=`: restrict which CPU architectures the
/// service may issue syscalls from (hardening against e.g. the i386 compat ABI
/// on x86_64). Syscalls from any non-listed architecture are killed. If any
/// listed architecture cannot be mapped, the filter is skipped (run permissive)
/// rather than risk killing the service by wrongly restricting it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_system_call_architectures(config: &ExecHelperConfig) {
    if config.system_call_architectures.is_empty() {
        return;
    }
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const AUDIT_ARCH_I386: u32 = 0x4000_0003;
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    let mut archs: Vec<u32> = Vec::new();
    for a in &config.system_call_architectures {
        let v = match a.trim() {
            "native" | "x86-64" | "x86_64" | "amd64" => AUDIT_ARCH_X86_64,
            "x86" | "i386" => AUDIT_ARCH_I386,
            other => {
                log::warn!(
                    "SystemCallArchitectures: unsupported architecture '{other}' for {}; \
                     not restricting",
                    config.name
                );
                return;
            }
        };
        if !archs.contains(&v) {
            archs.push(v);
        }
    }
    if install_seccomp_program(&build_seccomp_arch_filter(&archs, SECCOMP_RET_KILL_PROCESS)) {
        log::debug!(
            "SystemCallArchitectures: restricted to {} architecture(s) for {}",
            archs.len(),
            config.name
        );
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_system_call_architectures(_config: &ExecHelperConfig) {
    log::warn!("SystemCallArchitectures= not implemented for this target");
}

/// Resolve an `AF_*` address-family name to its number.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn address_family_from_name(name: &str) -> Option<u32> {
    let v: libc::c_int = match name {
        "AF_UNSPEC" => libc::AF_UNSPEC,
        "AF_UNIX" | "AF_LOCAL" => libc::AF_UNIX,
        "AF_INET" => libc::AF_INET,
        "AF_INET6" => libc::AF_INET6,
        "AF_NETLINK" | "AF_ROUTE" => libc::AF_NETLINK,
        "AF_PACKET" => libc::AF_PACKET,
        "AF_VSOCK" => libc::AF_VSOCK,
        "AF_ALG" => libc::AF_ALG,
        "AF_BLUETOOTH" => libc::AF_BLUETOOTH,
        "AF_KEY" => libc::AF_KEY,
        "AF_CAN" => libc::AF_CAN,
        "AF_TIPC" => libc::AF_TIPC,
        "AF_RDS" => libc::AF_RDS,
        "AF_LLC" => libc::AF_LLC,
        "AF_PPPOX" => libc::AF_PPPOX,
        "AF_NFC" => libc::AF_NFC,
        "AF_APPLETALK" => libc::AF_APPLETALK,
        "AF_IPX" => libc::AF_IPX,
        "AF_X25" => libc::AF_X25,
        "AF_AX25" => libc::AF_AX25,
        "AF_ATMPVC" => libc::AF_ATMPVC,
        "AF_BRIDGE" => libc::AF_BRIDGE,
        "AF_NETROM" => libc::AF_NETROM,
        "AF_ROSE" => libc::AF_ROSE,
        "AF_SECURITY" => libc::AF_SECURITY,
        "AF_PHONET" => libc::AF_PHONET,
        "AF_IEEE802154" => libc::AF_IEEE802154,
        "AF_CAIF" => libc::AF_CAIF,
        "AF_ISDN" => libc::AF_ISDN,
        "AF_IUCV" => libc::AF_IUCV,
        "AF_RXRPC" => libc::AF_RXRPC,
        _ => return None,
    };
    Some(v as u32)
}

/// Build a seccomp cBPF program that restricts the address family passed to
/// socket(2). For an allow-list, socket() with a family in `families` is allowed
/// and any other family gets EAFNOSUPPORT; for a deny-list, the listed families
/// get EAFNOSUPPORT and others are allowed. Non-socket syscalls always pass.
/// Reads the low 32 bits of args[0] (the family), matching how the kernel
/// truncates socket()'s int domain.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_restrict_address_families_filter(
    families: &[u32],
    deny_list: bool,
    errno: u32,
) -> Vec<libc::sock_filter> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const ALLOW: u32 = 0x7fff_0000;
    let errno_act = 0x0005_0000 | (errno & 0xffff);
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    let n = families.len();
    // Index of the trailing RET ALLOW, used as a jump target.
    let final_idx: usize = if deny_list { 6 + 2 * n } else { 6 + n + 1 };
    let mut prog = vec![
        f(0x20, 0, 0, 4),                 // 0 LD arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64), // 1 JEQ x86_64 -> [3], else [2]
        f(0x06, 0, 0, ALLOW),             // 2 RET ALLOW (non-x86_64)
        f(0x20, 0, 0, 0),                 // 3 LD nr
        // 4 JEQ nr==socket -> [5]; else jump to the trailing RET ALLOW.
        f(0x15, 0, (final_idx - 5) as u8, libc::SYS_socket as u32),
        f(0x20, 0, 0, 16), // 5 LD args[0] low word (the family)
    ];
    if deny_list {
        for &fam in families {
            prog.push(f(0x15, 0, 1, fam)); // JEQ fam -> RET errno; else skip
            prog.push(f(0x06, 0, 0, errno_act));
        }
        prog.push(f(0x06, 0, 0, ALLOW)); // family not listed: allow
    } else {
        for (i, &fam) in families.iter().enumerate() {
            // JEQ fam -> jump to the trailing RET ALLOW; else fall through.
            prog.push(f(0x15, (final_idx - (6 + i) - 1) as u8, 0, fam));
        }
        prog.push(f(0x06, 0, 0, errno_act)); // family not allowed
        prog.push(f(0x06, 0, 0, ALLOW)); // allowed family / non-socket land here
    }
    prog
}

/// Apply `RestrictAddressFamilies=`: filter socket(2) by address family.
/// Blocked families get EAFNOSUPPORT (systemd's default), never a kill, so this
/// cannot crash a service. For an allow-list, any unmappable family name means
/// we cannot safely restrict, so the filter is skipped (run permissive).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_restrict_address_families(config: &ExecHelperConfig) {
    if config.restrict_address_families.is_empty() {
        return;
    }
    let deny_list = config.restrict_address_families[0].starts_with('~');
    let mut families: Vec<u32> = Vec::new();
    for (i, tok) in config.restrict_address_families.iter().enumerate() {
        let name = if i == 0 {
            tok.strip_prefix('~').unwrap_or(tok)
        } else {
            tok.as_str()
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        match address_family_from_name(name) {
            Some(af) => {
                if !families.contains(&af) {
                    families.push(af);
                }
            }
            None if deny_list => {
                log::debug!("RestrictAddressFamilies: unknown family '{name}', ignoring");
            }
            None => {
                log::warn!(
                    "RestrictAddressFamilies: unknown family '{name}' for {}; not restricting",
                    config.name
                );
                return;
            }
        }
    }
    if families.is_empty() {
        return;
    }
    let prog =
        build_restrict_address_families_filter(&families, deny_list, libc::EAFNOSUPPORT as u32);
    if install_seccomp_program(&prog) {
        log::debug!(
            "RestrictAddressFamilies: installed socket() family filter ({} families) for {}",
            families.len(),
            config.name
        );
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_restrict_address_families(_config: &ExecHelperConfig) {
    log::warn!("RestrictAddressFamilies= not implemented for this target");
}

/// Build a seccomp cBPF program that returns `action` when a syscall matches one
/// of `rules` and *every* `(offset, mask)` check in that rule holds, where a
/// check holds iff `(seccomp_data word at offset) & mask == mask`. `offset` is the
/// byte offset of an argument's low 32-bit word in `struct seccomp_data`
/// (16 + 8*arg_index). Any unmatched syscall, or a matched syscall failing a
/// check, is allowed. Jumps to the shared block/allow tails and to each rule's
/// successor are patched after layout, so the rule count is unconstrained (within
/// cBPF's 255-instruction jump reach).
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_seccomp_arg_masked_filter(
    rules: &[(i64, Vec<(u32, u32)>)],
    action: u32,
) -> Vec<libc::sock_filter> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const ALLOW: u32 = 0x7fff_0000;
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    let mut insns = vec![
        f(0x20, 0, 0, 4),                 // LD arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64), // JEQ x86_64 -> rules, else next
        f(0x06, 0, 0, ALLOW),             // RET ALLOW (non-x86_64)
    ];
    let mut to_block: Vec<usize> = Vec::new();
    for (nr, checks) in rules {
        // Indices whose jf must reach this rule's successor (a failed check or a
        // non-matching nr means "not this rule": allow / try the next rule).
        let mut to_next: Vec<usize> = Vec::new();
        insns.push(f(0x20, 0, 0, 0)); // LD nr
        to_next.push(insns.len());
        insns.push(f(0x15, 0, 0, *nr as u32)); // JEQ nr == RN (jf -> next rule)
        let last = checks.len().saturating_sub(1);
        for (j, &(off, mask)) in checks.iter().enumerate() {
            insns.push(f(0x20, 0, 0, off)); // LD arg low word
            insns.push(f(0x54, 0, 0, mask)); // A &= mask (BPF_ALU|BPF_AND|BPF_K)
            let jeq = insns.len();
            insns.push(f(0x15, 0, 0, mask)); // JEQ (arg & mask) == mask
            to_next.push(jeq); // jf -> next rule (check failed)
            if j == last {
                to_block.push(jeq); // jt -> block (every check held)
            }
            // Intermediate checks keep jt = 0 and fall through to the next check.
        }
        let next_rule = insns.len();
        for idx in to_next {
            insns[idx].jf = (next_rule - idx - 1) as u8;
        }
    }
    // The last rule's successor is this ALLOW tail; the block tail follows it.
    let allow_idx = insns.len();
    insns.push(f(0x06, 0, 0, ALLOW)); // RET ALLOW (fallthrough)
    let block_idx = insns.len();
    insns.push(f(0x06, 0, 0, action)); // RET action
    debug_assert_eq!(allow_idx + 1, block_idx);
    for idx in to_block {
        insns[idx].jt = (block_idx - idx - 1) as u8;
    }
    insns
}

/// Build the `RestrictRealtime=` filter: `sched_setscheduler()` is denied unless
/// its policy (args[1]) is one of the permitted non-realtime policies
/// (SCHED_OTHER, SCHED_BATCH, SCHED_IDLE); everything else, including realtime
/// policies and any value with SCHED_RESET_ON_FORK set, gets `action`.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn build_restrict_realtime_filter(action: u32) -> Vec<libc::sock_filter> {
    const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
    const ALLOW: u32 = 0x7fff_0000;
    let f = |code: u16, jt: u8, jf: u8, k: u32| libc::sock_filter { code, jt, jf, k };
    vec![
        f(0x20, 0, 0, 4),                                   // 0 LD arch
        f(0x15, 1, 0, AUDIT_ARCH_X86_64),                   // 1 JEQ x86_64 -> [3], else [2]
        f(0x06, 0, 0, ALLOW),                               // 2 RET ALLOW
        f(0x20, 0, 0, 0),                                   // 3 LD nr
        f(0x15, 0, 5, libc::SYS_sched_setscheduler as u32), // 4 JEQ setscheduler, else ALLOW[10]
        f(0x20, 0, 0, 24),                                  // 5 LD args[1] (policy)
        f(0x15, 3, 0, 0),                                   // 6 JEQ SCHED_OTHER -> ALLOW[10]
        f(0x15, 2, 0, 3),                                   // 7 JEQ SCHED_BATCH -> ALLOW[10]
        f(0x15, 1, 0, 5),                                   // 8 JEQ SCHED_IDLE  -> ALLOW[10]
        f(0x06, 0, 0, action),                              // 9 RET action (realtime policy)
        f(0x06, 0, 0, ALLOW),                               // 10 RET ALLOW
    ]
}

/// Apply `MemoryDenyWriteExecute=`: deny mmap() of writable+executable memory,
/// mprotect()/pkey_mprotect() adding PROT_EXEC, and shmat() with SHM_EXEC. All
/// return EPERM (never a kill), matching systemd. A normally-linked program is
/// unaffected; only runtime code generation (JITs) is blocked.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_memory_deny_write_execute(config: &ExecHelperConfig) {
    if !config.memory_deny_write_execute {
        return;
    }
    let prot_wx = (libc::PROT_EXEC | libc::PROT_WRITE) as u32;
    let prot_x = libc::PROT_EXEC as u32;
    const SHM_EXEC: u32 = 0o100000; // not in every libc
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    // args[2] (prot / shmflg) is at byte offset 16 + 2*8 = 32 in seccomp_data.
    let rules: Vec<(i64, Vec<(u32, u32)>)> = vec![
        (libc::SYS_mmap, vec![(32, prot_wx)]),
        (libc::SYS_mprotect, vec![(32, prot_x)]),
        (libc::SYS_pkey_mprotect, vec![(32, prot_x)]),
        (libc::SYS_shmat, vec![(32, SHM_EXEC)]),
    ];
    if install_seccomp_program(&build_seccomp_arg_masked_filter(&rules, eperm)) {
        log::debug!(
            "MemoryDenyWriteExecute: installed W^X seccomp filter for {}",
            config.name
        );
    }
}

/// Apply `RestrictRealtime=`: deny realtime scheduling policies via seccomp on
/// sched_setscheduler(). Blocked calls get EPERM.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_restrict_realtime(config: &ExecHelperConfig) {
    if !config.restrict_realtime {
        return;
    }
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    if install_seccomp_program(&build_restrict_realtime_filter(eperm)) {
        log::debug!(
            "RestrictRealtime: installed sched_setscheduler filter for {}",
            config.name
        );
    }
}

/// Apply `RestrictSUIDSGID=`: deny setting the setuid/setgid mode bits via
/// chmod/fchmod/fchmodat(2), mkdir/mkdirat, mknod/mknodat, creat, and
/// open/openat (the latter two only when O_CREAT is set, so a plain open() with a
/// stray mode value is not blocked). Blocked calls get EPERM.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_restrict_suid_sgid(config: &ExecHelperConfig) {
    if !config.restrict_suid_sgid {
        return;
    }
    let suid = libc::S_ISUID as u32;
    let sgid = libc::S_ISGID as u32;
    let o_creat = libc::O_CREAT as u32;
    const SYS_FCHMODAT2: i64 = 452;
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    // (nr, byte offset of the mode argument's low word in seccomp_data).
    let simple: [(i64, u32); 9] = [
        (libc::SYS_chmod, 24),    // chmod(path, MODE)
        (libc::SYS_fchmod, 24),   // fchmod(fd, MODE)
        (libc::SYS_fchmodat, 32), // fchmodat(dfd, path, MODE, flags)
        (SYS_FCHMODAT2, 32),      // fchmodat2(dfd, path, MODE, flags)
        (libc::SYS_creat, 24),    // creat(path, MODE)
        (libc::SYS_mkdir, 24),    // mkdir(path, MODE)
        (libc::SYS_mkdirat, 32),  // mkdirat(dfd, path, MODE)
        (libc::SYS_mknod, 24),    // mknod(path, MODE, dev)
        (libc::SYS_mknodat, 32),  // mknodat(dfd, path, MODE, dev)
    ];
    let mut rules: Vec<(i64, Vec<(u32, u32)>)> = Vec::new();
    for &(nr, off) in &simple {
        rules.push((nr, vec![(off, suid)]));
        rules.push((nr, vec![(off, sgid)]));
    }
    for &bit in &[suid, sgid] {
        // open(path, FLAGS, MODE): flags args[1]@24, mode args[2]@32.
        rules.push((libc::SYS_open, vec![(24, o_creat), (32, bit)]));
        // openat(dfd, path, FLAGS, MODE): flags args[2]@32, mode args[3]@40.
        rules.push((libc::SYS_openat, vec![(32, o_creat), (40, bit)]));
    }
    if install_seccomp_program(&build_seccomp_arg_masked_filter(&rules, eperm)) {
        log::debug!(
            "RestrictSUIDSGID: installed setuid/setgid mode filter for {}",
            config.name
        );
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_memory_deny_write_execute(_config: &ExecHelperConfig) {}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_restrict_realtime(_config: &ExecHelperConfig) {}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_restrict_suid_sgid(_config: &ExecHelperConfig) {}

/// Apply `ProtectKernelModules=`: block the module-management syscalls (the
/// @module set: init_module/finit_module/delete_module) with EPERM, matching
/// systemd. This is the seccomp complement to the read-only module directories.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_protect_kernel_modules(config: &ExecHelperConfig) {
    if !config.protect_kernel_modules {
        return;
    }
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    let (mut set, mut visited) = (Vec::new(), Vec::new());
    resolve_syscall_set(&["@module"], &mut set, &mut visited);
    if !set.is_empty() && install_seccomp_deny_filter(&set, eperm) {
        log::debug!(
            "ProtectKernelModules: blocked {} module syscalls for {}",
            set.len(),
            config.name
        );
    }
}

/// Apply `ProtectClock=`: block the clock-setting syscalls (the @clock set:
/// settimeofday/clock_settime/adjtimex and friends) with EPERM, matching systemd.
/// This is the seccomp complement to the /dev/rtc device denial.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_protect_clock(config: &ExecHelperConfig) {
    if !config.protect_clock {
        return;
    }
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    let (mut set, mut visited) = (Vec::new(), Vec::new());
    resolve_syscall_set(&["@clock"], &mut set, &mut visited);
    if !set.is_empty() && install_seccomp_deny_filter(&set, eperm) {
        log::debug!(
            "ProtectClock: blocked {} clock syscalls for {}",
            set.len(),
            config.name
        );
    }
}

/// Apply `ProtectKernelLogs=`: block syslog(2) (the kernel-log syscall) with
/// EPERM, matching systemd's seccomp_protect_syslog(). This is the seccomp
/// complement to the /dev/kmsg and /proc/kmsg denial.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_protect_kernel_logs(config: &ExecHelperConfig) {
    if !config.protect_kernel_logs {
        return;
    }
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    if install_seccomp_deny_filter(&[libc::SYS_syslog], eperm) {
        log::debug!("ProtectKernelLogs: blocked syslog(2) for {}", config.name);
    }
}

/// Apply `ProtectKernelTunables=`: block the (long-removed) _sysctl syscall with
/// EPERM, matching systemd's seccomp_protect_sysctl(). This is vestigial on
/// modern kernels (the syscall is gone); the read-only /proc/sys mount is the
/// substantive protection.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn install_protect_kernel_tunables(config: &ExecHelperConfig) {
    if !config.protect_kernel_tunables {
        return;
    }
    const SYS_SYSCTL: i64 = 156; // _sysctl on x86_64
    let eperm = 0x0005_0000 | (libc::EPERM as u32);
    if install_seccomp_deny_filter(&[SYS_SYSCTL], eperm) {
        log::debug!("ProtectKernelTunables: blocked _sysctl for {}", config.name);
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_protect_kernel_modules(_config: &ExecHelperConfig) {}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_protect_clock(_config: &ExecHelperConfig) {}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_protect_kernel_logs(_config: &ExecHelperConfig) {}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn install_protect_kernel_tunables(_config: &ExecHelperConfig) {}

/// Bring up the loopback interface in a new network namespace.
fn bring_up_loopback() {
    // Use a netlink socket to bring up lo
    let fd = unsafe { libc::socket(libc::AF_NETLINK, libc::SOCK_DGRAM, libc::NETLINK_ROUTE) };
    if fd < 0 {
        return;
    }

    // We'll use the simple approach of running `ip link set lo up` equivalent
    // via a raw netlink RTM_NEWLINK message. For simplicity, just write to
    // /sys/class/net/lo/flags or use ioctl.
    let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if sock >= 0 {
        let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
        let lo_name = b"lo\0";
        unsafe {
            std::ptr::copy_nonoverlapping(
                lo_name.as_ptr(),
                ifr.ifr_name.as_mut_ptr().cast(),
                lo_name.len(),
            );
        }
        // Get current flags
        let ret = unsafe { libc::ioctl(sock, libc::SIOCGIFFLAGS, &mut ifr) };
        if ret == 0 {
            unsafe {
                ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
            }
            let _ = unsafe { libc::ioctl(sock, libc::SIOCSIFFLAGS, &ifr) };
        }
        unsafe { libc::close(sock) };
    }
    unsafe { libc::close(fd) };
}

/// Apply CapabilityBoundingSet= by dropping capabilities not in the allow-list.
fn apply_capability_bounding_set(config: &ExecHelperConfig) {
    let caps = &config.capability_bounding_set;
    if caps.is_empty() {
        return;
    }

    // Determine if this is an allow-list or deny-list.
    // If all entries start with '~', it's a deny-list (drop those caps).
    // If no entries start with '~', it's an allow-list (keep only those caps).
    // Mixed usage: entries with '~' are denied, others are allowed.
    let has_deny = caps.iter().any(|c| c.starts_with('~'));
    let has_allow = caps.iter().any(|c| !c.starts_with('~'));

    if has_deny && !has_allow {
        // Pure deny-list: drop only the specified capabilities
        for cap_name in caps {
            let name = cap_name.strip_prefix('~').unwrap_or(cap_name);
            if let Some(cap_num) = cap_name_to_number(name) {
                let ret = unsafe { libc::prctl(libc::PR_CAPBSET_DROP, cap_num, 0, 0, 0) };
                if ret != 0 {
                    log::warn!(
                        "Failed to drop capability {}: {} (non-fatal)",
                        name,
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    } else {
        // Allow-list (or mixed): keep only the listed capabilities, drop all others.
        // Collect the set of allowed capability numbers.
        let mut allowed: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut denied: std::collections::HashSet<u64> = std::collections::HashSet::new();

        for cap_name in caps {
            if let Some(name) = cap_name.strip_prefix('~') {
                if let Some(num) = cap_name_to_number(name) {
                    denied.insert(num);
                }
            } else if let Some(num) = cap_name_to_number(cap_name) {
                allowed.insert(num);
            }
        }

        // Drop capabilities not in the allow-list (or explicitly denied).
        // Linux has at most ~41 capabilities (as of kernel 6.x).
        for cap_num in 0..64u64 {
            // Check if this capability exists in the bounding set
            let ret =
                unsafe { libc::prctl(libc::PR_CAPBSET_READ, cap_num as libc::c_ulong, 0, 0, 0) };
            if ret < 0 {
                break; // No more capabilities
            }
            if ret == 0 {
                continue; // Already not in bounding set
            }

            let should_drop = if !allowed.is_empty() {
                // Allow-list mode: drop if not in allowed set or explicitly denied
                !allowed.contains(&cap_num) || denied.contains(&cap_num)
            } else {
                // Pure deny mode (shouldn't reach here, but handle gracefully)
                denied.contains(&cap_num)
            };

            if should_drop {
                let ret = unsafe {
                    libc::prctl(libc::PR_CAPBSET_DROP, cap_num as libc::c_ulong, 0, 0, 0)
                };
                if ret != 0 {
                    log::warn!(
                        "Failed to drop capability {}: {} (non-fatal)",
                        cap_num,
                        std::io::Error::last_os_error()
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ImportCredential= helpers
// ---------------------------------------------------------------------------

/// System credential store directories searched in order (matching systemd).
const CREDENTIAL_STORES: &[&str] = &[
    "/run/credentials/@system",
    "/run/credstore",
    "/etc/credstore",
];

/// Path to the host encryption key used for credential encryption/decryption.
const HOST_KEY_PATH: &str = "/var/lib/systemd/credential.secret";

/// Magic bytes identifying an encrypted credential blob: "sHc\0".
const CRED_MAGIC: [u8; 4] = [0x73, 0x48, 0x63, 0x00];

/// Fixed header size: magic(4) + seal_type(4) + timestamp(8) + not_after(8) + name_len(4) = 28.
const CRED_HEADER_FIXED_SIZE: usize = 28;

/// AES-256-GCM nonce size.
const CRED_AES_IV_SIZE: usize = 12;

/// Seal type: null key (SHA-256 of credential name only).
const CRED_SEAL_NULL: u32 = 0;
/// Seal type: host key (SHA-256 of host_key || credential_name).
const CRED_SEAL_HOST: u32 = 1;
/// Seal type: TPM2-sealed secret (SHA-256 of tpm2_secret || credential_name).
const CRED_SEAL_TPM2: u32 = 2;
/// Seal type: host key + TPM2 (SHA-256 of host_key || tpm2_secret || credential_name).
const CRED_SEAL_HOST_TPM2: u32 = 3;

/// Attempt to decrypt an encrypted credential blob.
///
/// The blob may be either raw binary (the wire format produced by
/// `systemd-creds encrypt`) or Base64-encoded. This function tries
/// Base64 decoding first; if that fails it treats the input as raw.
///
/// Returns `Ok(plaintext_bytes)` on success, or `Err(message)` on failure.
/// On any error the caller should fall back to writing the data as-is
/// (matching the previous behaviour) so that services which do their own
/// decryption still work.
fn try_decrypt_credential(data: &[u8], cred_name: &str) -> Result<Vec<u8>, String> {
    // Try Base64 decode first (systemd-creds output is always Base64).
    let blob = {
        let as_str = String::from_utf8_lossy(data);
        let cleaned: String = as_str.chars().filter(|c| !c.is_whitespace()).collect();
        BASE64.decode(&cleaned).unwrap_or_else(|_| data.to_vec())
    };

    if blob.len() < CRED_HEADER_FIXED_SIZE {
        return Err("blob too short for credential header".into());
    }

    // Validate magic.
    if blob[0..4] != CRED_MAGIC {
        return Err("invalid credential magic".into());
    }

    let seal_type = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    let _timestamp = u64::from_le_bytes(blob[8..16].try_into().unwrap());
    let not_after = u64::from_le_bytes(blob[16..24].try_into().unwrap());
    let name_len = u32::from_le_bytes(blob[24..28].try_into().unwrap()) as usize;

    let name_end = CRED_HEADER_FIXED_SIZE + name_len;
    if blob.len() < name_end + CRED_AES_IV_SIZE {
        return Err("blob too short for name + IV".into());
    }

    // Check expiry.
    if not_after != 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        if now > not_after {
            return Err(format!(
                "credential expired (not_after={not_after}, now={now})"
            ));
        }
    }

    // For TPM2 and host+tpm2 seal types, parse and unseal the TPM2 blob
    // that sits between the name and the IV.
    let (tpm2_secret, data_start) =
        if seal_type == CRED_SEAL_TPM2 || seal_type == CRED_SEAL_HOST_TPM2 {
            let tpm2_data = &blob[name_end..];
            let (tpm2_blob, consumed) = crate::tpm2::Tpm2SealedBlob::deserialize(tpm2_data)
                .map_err(|e| format!("failed to parse TPM2 blob: {e}"))?;
            let secret = crate::tpm2::tpm2_unseal_secret(&tpm2_blob)
                .map_err(|e| format!("TPM2 unseal failed: {e}"))?;
            (Some(secret), name_end + consumed)
        } else {
            (None, name_end)
        };

    // Extract IV and ciphertext.
    if blob.len() < data_start + CRED_AES_IV_SIZE {
        return Err("blob too short for IV".into());
    }
    let iv = &blob[data_start..data_start + CRED_AES_IV_SIZE];
    let ciphertext = &blob[data_start + CRED_AES_IV_SIZE..];

    if ciphertext.len() < 16 {
        // AES-GCM tag is 16 bytes minimum
        return Err("blob too short for ciphertext + GCM tag".into());
    }

    // Derive AES-256 key based on seal type.
    let aes_key: [u8; 32] = match seal_type {
        CRED_SEAL_NULL => {
            let mut h = Sha256::new();
            h.update(cred_name.as_bytes());
            h.finalize().into()
        }
        CRED_SEAL_HOST => {
            let host_key = std::fs::read(HOST_KEY_PATH)
                .map_err(|e| format!("cannot read host key {HOST_KEY_PATH}: {e}"))?;
            let mut h = Sha256::new();
            h.update(&host_key);
            h.update(cred_name.as_bytes());
            h.finalize().into()
        }
        CRED_SEAL_TPM2 => {
            let secret = tpm2_secret.as_ref().unwrap();
            crate::tpm2::derive_tpm2_key(secret, cred_name)
        }
        CRED_SEAL_HOST_TPM2 => {
            let host_key = std::fs::read(HOST_KEY_PATH)
                .map_err(|e| format!("cannot read host key {HOST_KEY_PATH}: {e}"))?;
            let secret = tpm2_secret.as_ref().unwrap();
            crate::tpm2::derive_host_tpm2_key(&host_key, secret, cred_name)
        }
        other => {
            return Err(format!("unsupported seal type {other}"));
        }
    };

    let cipher =
        Aes256Gcm::new_from_slice(&aes_key).map_err(|e| format!("AES init failed: {e}"))?;
    let nonce = Nonce::from_slice(iv);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "decryption failed (wrong key or corrupted data)".into())
}

/// Set up the per-service credential directory, processing all credential
/// directives in the correct priority order (matching systemd):
///
/// 1. `SetCredential=` / `SetCredentialEncrypted=` — lowest priority, written
///    first so they can be overridden by later directives.
/// 2. `LoadCredential=` / `LoadCredentialEncrypted=` — medium priority, copies
///    from file paths. Overwrites credentials set by `SetCredential=`.
/// 3. `ImportCredential=` — highest priority, glob-matches from system
///    credential stores. Does NOT overwrite existing credentials (first match
///    wins across stores, but won't override LoadCredential/SetCredential).
///
/// Encrypted variants (`SetCredentialEncrypted=`, `LoadCredentialEncrypted=`)
/// are decrypted at runtime using AES-256-GCM with a key derived from the
/// host secret (`/var/lib/systemd/credential.secret`) or a null key. If
/// decryption fails (e.g. no host key, wrong key, corrupted data), the
/// encrypted content is written as-is so services that handle their own
/// decryption still work.
fn setup_credentials(config: &ExecHelperConfig) {
    let cred_dir = PathBuf::from(format!("/run/credentials/{}", config.name));

    // Create the credential directory.
    if let Err(e) = std::fs::create_dir_all(&cred_dir) {
        log::warn!(
            "Failed to create credentials directory {:?}: {}",
            cred_dir,
            e
        );
        // Non-fatal — the service may still work without credentials.
        return;
    }

    // Restrict permissions to owner-only (0o700), matching systemd.
    let ret = unsafe {
        libc::chmod(
            std::ffi::CString::new(cred_dir.to_string_lossy().as_bytes())
                .unwrap()
                .as_ptr(),
            0o700,
        )
    };
    if ret != 0 {
        log::warn!(
            "Failed to chmod credentials directory {:?}: {}",
            cred_dir,
            std::io::Error::last_os_error()
        );
    }

    // Chown to the service user/group so the service can read them.
    let uid = nix::unistd::Uid::from_raw(config.user);
    let gid = nix::unistd::Gid::from_raw(config.group);
    if let Err(e) = nix::unistd::chown(&cred_dir, Some(uid), Some(gid)) {
        log::warn!(
            "Failed to chown credentials directory {:?}: {}",
            cred_dir,
            e
        );
    }

    let mut _wrote = 0usize;

    // --- Phase 1: SetCredential= (lowest priority) ---
    for (id, data) in &config.set_credentials {
        let dst = cred_dir.join(id);
        let unescaped = cunescape(data);
        match std::fs::write(&dst, &unescaped) {
            Ok(()) => {
                set_credential_perms(&dst, uid, gid);
                _wrote += 1;
            }
            Err(e) => {
                log::warn!("Failed to write SetCredential {:?}: {}", id, e);
            }
        }
    }

    // --- Phase 1b: SetCredentialEncrypted= (same priority as SetCredential) ---
    for (id, data) in &config.set_credentials_encrypted {
        let dst = cred_dir.join(id);
        // Don't overwrite a credential set by SetCredential= with the same ID
        // (first writer wins within the same priority level).
        if dst.exists() {
            continue;
        }
        // Try to decrypt; fall back to writing as-is if decryption fails.
        let write_data = match try_decrypt_credential(data.as_bytes(), id) {
            Ok(plaintext) => plaintext,
            Err(e) => {
                log::warn!(
                    "SetCredentialEncrypted {:?}: decryption failed ({}), writing as-is",
                    id,
                    e
                );
                data.as_bytes().to_vec()
            }
        };
        match std::fs::write(&dst, &write_data) {
            Ok(()) => {
                set_credential_perms(&dst, uid, gid);
                _wrote += 1;
            }
            Err(e) => {
                log::warn!("Failed to write SetCredentialEncrypted {:?}: {}", id, e);
            }
        }
    }

    // --- Phase 2: LoadCredential= (overwrites SetCredential) ---
    for (id, path_str) in &config.load_credentials {
        let src = Path::new(path_str);

        // If the path is not absolute, search credential stores (matching
        // systemd's behaviour for relative LoadCredential= paths).
        let resolved = if src.is_absolute() {
            src.to_path_buf()
        } else {
            let mut found = None;
            for store_dir in CREDENTIAL_STORES {
                let candidate = Path::new(store_dir).join(path_str);
                if candidate.exists() {
                    found = Some(candidate);
                    break;
                }
            }
            match found {
                Some(p) => p,
                None => {
                    log::warn!(
                        "LoadCredential {:?}: path {:?} not found (searched credential stores)",
                        id,
                        path_str
                    );
                    continue;
                }
            }
        };

        if !resolved.exists() {
            log::warn!(
                "LoadCredential {:?}: source {:?} does not exist",
                id,
                resolved
            );
            continue;
        }

        let dst = cred_dir.join(id);

        // If the source is a directory, load all files within it as
        // sub-credentials (matching systemd behaviour).
        if resolved.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&resolved) {
                for entry in entries.flatten() {
                    if entry.path().is_file() {
                        let sub_dst = cred_dir.join(entry.file_name());
                        match std::fs::copy(entry.path(), &sub_dst) {
                            Ok(_) => {
                                set_credential_perms(&sub_dst, uid, gid);
                                _wrote += 1;
                            }
                            Err(e) => {
                                log::warn!(
                                    "Failed to load credential {:?} from dir {:?}: {}",
                                    entry.file_name(),
                                    resolved,
                                    e
                                );
                            }
                        }
                    }
                }
            }
        } else {
            match std::fs::copy(&resolved, &dst) {
                Ok(_) => {
                    set_credential_perms(&dst, uid, gid);
                    _wrote += 1;
                }
                Err(e) => {
                    log::warn!(
                        "Failed to load credential {:?} from {:?}: {}",
                        id,
                        resolved,
                        e
                    );
                }
            }
        }
    }

    // --- Phase 2b: LoadCredentialEncrypted= (same priority as LoadCredential) ---
    for (id, path_str) in &config.load_credentials_encrypted {
        let src = Path::new(path_str);

        let resolved = if src.is_absolute() {
            src.to_path_buf()
        } else {
            let mut found = None;
            for store_dir in CREDENTIAL_STORES {
                let candidate = Path::new(store_dir).join(path_str);
                if candidate.exists() {
                    found = Some(candidate);
                    break;
                }
            }
            match found {
                Some(p) => p,
                None => {
                    log::warn!(
                        "LoadCredentialEncrypted {:?}: path {:?} not found",
                        id,
                        path_str
                    );
                    continue;
                }
            }
        };

        if !resolved.exists() || !resolved.is_file() {
            log::warn!(
                "LoadCredentialEncrypted {:?}: source {:?} not found or not a file",
                id,
                resolved
            );
            continue;
        }

        let dst = cred_dir.join(id);

        // Read the encrypted file and try to decrypt it.
        match std::fs::read(&resolved) {
            Ok(encrypted_data) => {
                let write_data = match try_decrypt_credential(&encrypted_data, id) {
                    Ok(plaintext) => plaintext,
                    Err(e) => {
                        log::warn!(
                            "LoadCredentialEncrypted {:?}: decryption failed ({}), writing as-is",
                            id,
                            e
                        );
                        encrypted_data
                    }
                };
                match std::fs::write(&dst, &write_data) {
                    Ok(()) => {
                        set_credential_perms(&dst, uid, gid);
                        _wrote += 1;
                    }
                    Err(e) => {
                        log::warn!("Failed to write decrypted credential {:?}: {}", id, e);
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "Failed to read encrypted credential {:?} from {:?}: {}",
                    id,
                    resolved,
                    e
                );
            }
        }
    }

    // --- Phase 3: ImportCredential= (highest priority, won't overwrite) ---
    for pattern in &config.import_credentials {
        for store_dir in CREDENTIAL_STORES {
            let store = Path::new(store_dir);
            if !store.is_dir() {
                continue;
            }

            let entries = match std::fs::read_dir(store) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let file_name = entry.file_name();
                let name_str = file_name.to_string_lossy();

                if !glob_match(pattern, &name_str) {
                    continue;
                }

                let src = entry.path();
                if !src.is_file() {
                    continue;
                }

                let dst = cred_dir.join(&file_name);

                // Don't overwrite — first match wins (higher-priority store),
                // and ImportCredential never overwrites LoadCredential/SetCredential.
                if dst.exists() {
                    continue;
                }

                match std::fs::copy(&src, &dst) {
                    Ok(_) => {
                        set_credential_perms(&dst, uid, gid);
                        _wrote += 1;
                    }
                    Err(e) => {
                        log::warn!("Failed to import credential {:?} -> {:?}: {}", src, dst, e);
                    }
                }
            }
        }
    }

    // Always set the env var so the service knows where to look,
    // even if no credentials were found (matches systemd behaviour).
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("CREDENTIALS_DIRECTORY", &cred_dir) };
}

/// Set a credential file to owner-read-only (0o400) and chown to service user/group.
fn set_credential_perms(path: &Path, uid: nix::unistd::Uid, gid: nix::unistd::Gid) {
    let _ = unsafe {
        libc::chmod(
            std::ffi::CString::new(path.to_string_lossy().as_bytes())
                .unwrap()
                .as_ptr(),
            0o400,
        )
    };
    let _ = nix::unistd::chown(path, Some(uid), Some(gid));
}

/// Simple glob matcher supporting `*` (any chars) and `?` (single char).
/// This is intentionally minimal — systemd only uses simple filename globs
/// for ImportCredential=.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn glob_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    let mut pi = 0;
    let mut ti = 0;
    let mut star_pi = usize::MAX;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

// ---------------------------------------------------------------------------
// utmp / wtmp helpers
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn updwtmpx(file: *const libc::c_char, ut: *const libc::utmpx);
}

/// Path to the wtmp file (standard glibc location).
const WTMP_PATH: &[u8] = b"/var/log/wtmp\0";

/// Derive the TTY line name from a TTY path (e.g. "/dev/tty1" → "tty1").
/// Falls back to the full path if no `/dev/` prefix is found.
fn tty_line(config: &ExecHelperConfig) -> String {
    let path = config
        .tty_path
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("/dev/console"));
    let s = path.to_string_lossy();
    s.strip_prefix("/dev/").unwrap_or(&s).to_string()
}

/// Copy a Rust string into a fixed-size `c_char` array, truncating and
/// NUL-terminating as needed.
fn fill_c_char_buf(buf: &mut [libc::c_char], src: &str) {
    let bytes = src.as_bytes();
    let len = bytes.len().min(buf.len() - 1);
    for (i, &b) in bytes[..len].iter().enumerate() {
        buf[i] = b as libc::c_char;
    }
    // Remaining bytes are already zero from `mem::zeroed()`.
}

/// Build a `libc::utmpx` record from the current exec-helper config.
fn build_utmpx(config: &ExecHelperConfig, ut_type: libc::c_short) -> libc::utmpx {
    let mut ut: libc::utmpx = unsafe { std::mem::zeroed() };
    ut.ut_type = ut_type;
    ut.ut_pid = nix::unistd::getpid().as_raw();

    let line = tty_line(config);
    fill_c_char_buf(&mut ut.ut_line, &line);

    if let Some(ref id) = config.utmp_identifier {
        fill_c_char_buf(&mut ut.ut_id, id);
    } else {
        // Derive from TTY line — use last 4 characters (matches systemd).
        let id_str = if line.len() > 4 {
            &line[line.len() - 4..]
        } else {
            &line
        };
        fill_c_char_buf(&mut ut.ut_id, id_str);
    }

    // For LOGIN_PROCESS the user field is conventionally "LOGIN".
    // For INIT_PROCESS it is often empty or the service name.
    // For USER_PROCESS it should be the login name.
    match ut_type {
        libc::LOGIN_PROCESS => fill_c_char_buf(&mut ut.ut_user, "LOGIN"),
        libc::USER_PROCESS => {
            // Resolve uid → username if possible.
            if let Some(pw) = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(config.user))
                .ok()
                .flatten()
            {
                fill_c_char_buf(&mut ut.ut_user, &pw.name);
            }
        }
        _ => { /* INIT_PROCESS / DEAD_PROCESS — user field stays empty */ }
    }

    // Timestamp
    let now = unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        tv
    };
    ut.ut_tv.tv_sec = now.tv_sec as _;
    ut.ut_tv.tv_usec = now.tv_usec as _;

    ut
}

/// Write the initial utmp + wtmp record before exec'ing the service binary.
fn write_utmp_record(config: &ExecHelperConfig) {
    let ut_type: libc::c_short = match config.utmp_mode {
        UtmpMode::Init => libc::INIT_PROCESS as libc::c_short,
        UtmpMode::Login => libc::LOGIN_PROCESS as libc::c_short,
        UtmpMode::User => libc::USER_PROCESS as libc::c_short,
    };

    let ut = build_utmpx(config, ut_type);

    unsafe {
        libc::setutxent();
        let result = libc::pututxline(&ut);
        libc::endutxent();

        if result.is_null() {
            log::warn!(
                "Failed to write utmp record: {}",
                std::io::Error::last_os_error()
            );
        }

        // Append to wtmp as well.
        updwtmpx(WTMP_PATH.as_ptr() as *const libc::c_char, &ut);
    }
}

/// Write a DEAD_PROCESS utmp + wtmp record.  Called from the service manager
/// (parent process) when a service that had `UtmpIdentifier=` exits.
///
/// `identifier` is the `UtmpIdentifier=` value, `tty_path` the configured
/// TTY, and `pid` the PID of the exited service process.
pub fn write_utmp_dead_record(
    identifier: &str,
    tty_path: Option<&std::path::Path>,
    pid: nix::unistd::Pid,
) {
    let mut ut: libc::utmpx = unsafe { std::mem::zeroed() };
    ut.ut_type = libc::DEAD_PROCESS as libc::c_short;
    ut.ut_pid = pid.as_raw();

    let tty = tty_path.unwrap_or_else(|| std::path::Path::new("/dev/console"));
    let line = tty
        .to_string_lossy()
        .strip_prefix("/dev/")
        .unwrap_or(&tty.to_string_lossy())
        .to_string();
    fill_c_char_buf(&mut ut.ut_line, &line);
    fill_c_char_buf(&mut ut.ut_id, identifier);

    let now = unsafe {
        let mut tv: libc::timeval = std::mem::zeroed();
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
        tv
    };
    ut.ut_tv.tv_sec = now.tv_sec as _;
    ut.ut_tv.tv_usec = now.tv_usec as _;

    unsafe {
        libc::setutxent();
        let result = libc::pututxline(&ut);
        libc::endutxent();

        if result.is_null() {
            // Non-fatal — the utmp file may not exist or be writable.
            log::warn!(
                "Failed to write DEAD_PROCESS utmp record for id={}: {}",
                identifier,
                std::io::Error::last_os_error()
            );
        }

        updwtmpx(WTMP_PATH.as_ptr() as *const libc::c_char, &ut);
    }
}

// ---------------------------------------------------------------------------
// Environment variable expansion for ExecStart= command lines.
// Matches systemd's replace_env_argv() / replace_env() from env-util.c.
// ---------------------------------------------------------------------------

/// Expand environment variables in a single string.
/// - `$$` → literal `$`
/// - `${VARNAME}` → value of env var (empty string if unset)
/// - `$VARNAME` → value of env var (only valid identifiers: [A-Za-z_][A-Za-z0-9_]*)
/// - Other `$` usage → left as-is
pub(crate) fn expand_env_str(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'$' {
            result.push(bytes[i] as char);
            i += 1;
            continue;
        }

        // We found a '$'
        if i + 1 >= bytes.len() {
            // Trailing '$' — leave as-is
            result.push('$');
            i += 1;
            continue;
        }

        if bytes[i + 1] == b'$' {
            // $$ → literal $
            result.push('$');
            i += 2;
            continue;
        }

        if bytes[i + 1] == b'{' {
            // ${VARNAME} form
            if let Some(close) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                let var_name = &s[i + 2..i + 2 + close];
                if is_valid_env_name(var_name) {
                    if let Ok(val) = std::env::var(var_name) {
                        result.push_str(&val);
                    }
                    // If unset, expand to empty string (matching systemd)
                    i = i + 2 + close + 1;
                    continue;
                }
            }
            // Invalid ${...} — leave as-is
            result.push('$');
            i += 1;
            continue;
        }

        // $VARNAME form (without braces)
        if is_valid_env_name_start(bytes[i + 1]) {
            let start = i + 1;
            let mut end = start + 1;
            while end < bytes.len() && is_valid_env_name_cont(bytes[end]) {
                end += 1;
            }
            let var_name = &s[start..end];
            if let Ok(val) = std::env::var(var_name) {
                result.push_str(&val);
            }
            i = end;
            continue;
        }

        // $ followed by something that's not a valid identifier start — leave as-is
        result.push('$');
        i += 1;
    }

    result
}

/// Expand environment variables in an argv list.
/// Each argument is expanded in-place. If an entire argument is `$VARNAME`
/// (a single bare variable), systemd would split on whitespace, but we
/// currently do in-place expansion only for simplicity.
fn expand_env_argv(argv: &[String]) -> Vec<String> {
    argv.iter().map(|arg| expand_env_str(arg)).collect()
}

fn is_valid_env_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && is_valid_env_name_start(bytes[0])
        && bytes[1..].iter().all(|&b| is_valid_env_name_cont(b))
}

fn is_valid_env_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_valid_env_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ---------------------------------------------------------------------------
// Tests for credential decryption
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::AeadCore;

    #[test]
    fn test_set_tty_winsize() {
        // A pty master supports the winsize ioctls; posix_openpt avoids the
        // libutil link that openpty()/nix::pty would pull in.
        let fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(fd >= 0, "posix_openpt failed");

        // Seed a known starting size.
        let mut seed: libc::winsize = unsafe { std::mem::zeroed() };
        seed.ws_col = 100;
        seed.ws_row = 40;
        assert_eq!(unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &seed) }, 0);

        let get = |fd| {
            let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
            unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) };
            (ws.ws_col, ws.ws_row)
        };

        // Both unset: no-op.
        set_tty_winsize(fd, None, None);
        assert_eq!(get(fd), (100, 40));

        // Columns only: rows preserved from the current size.
        set_tty_winsize(fd, Some(80), None);
        assert_eq!(get(fd), (80, 40));

        // Both set.
        set_tty_winsize(fd, Some(132), Some(50));
        assert_eq!(get(fd), (132, 50));

        unsafe { libc::close(fd) };
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_seccomp_deny_filter_blocks_syscall() {
        // Fork: a seccomp filter is one-way, so it must be installed in a child
        // to avoid confining the test runner.
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        let action = SECCOMP_RET_ERRNO | (libc::EPERM as u32);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // Child: block mkdir with EPERM, then verify behaviour. Only raw
            // syscalls and _exit are used (async-signal-safe after fork).
            if !install_seccomp_deny_filter(&[libc::SYS_mkdir], action) {
                unsafe { libc::_exit(10) };
            }
            let r = unsafe {
                libc::syscall(libc::SYS_mkdir, c"/tmp/rs-seccomp-test".as_ptr(), 0o755)
            };
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if r != -1 || e != libc::EPERM {
                unsafe { libc::_exit(11) };
            }
            // A syscall not in the deny-list still works.
            if unsafe { libc::syscall(libc::SYS_getpid) } <= 0 {
                unsafe { libc::_exit(12) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "child seccomp checks failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_memory_deny_write_execute_filter() {
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        let action = SECCOMP_RET_ERRNO | (libc::EPERM as u32);
        let rules: Vec<(i64, Vec<(u32, u32)>)> = vec![
            (
                libc::SYS_mmap,
                vec![(32, (libc::PROT_EXEC | libc::PROT_WRITE) as u32)],
            ),
            (libc::SYS_mprotect, vec![(32, libc::PROT_EXEC as u32)]),
        ];
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            if !install_seccomp_program(&build_seccomp_arg_masked_filter(&rules, action)) {
                unsafe { libc::_exit(10) };
            }
            // A writable+executable mapping is blocked with EPERM.
            let p = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if p != libc::MAP_FAILED {
                unsafe { libc::_exit(11) };
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
                unsafe { libc::_exit(12) };
            }
            // A writable, non-executable mapping still works.
            let q = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    4096,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                    -1,
                    0,
                )
            };
            if q == libc::MAP_FAILED {
                unsafe { libc::_exit(13) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "MemoryDenyWriteExecute child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_restrict_realtime_filter() {
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        let action = SECCOMP_RET_ERRNO | (libc::EPERM as u32);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            if !install_seccomp_program(&build_restrict_realtime_filter(action)) {
                unsafe { libc::_exit(10) };
            }
            let param = libc::sched_param { sched_priority: 0 };
            // SCHED_OTHER (policy 0) is permitted: setting it on self succeeds,
            // proving the filter does not over-block sched_setscheduler().
            let r = unsafe {
                libc::syscall(libc::SYS_sched_setscheduler, 0, 0, &param as *const _)
            };
            if r != 0 {
                unsafe { libc::_exit(11) };
            }
            // SCHED_FIFO (policy 1, realtime) is denied with EPERM.
            let r2 = unsafe {
                libc::syscall(libc::SYS_sched_setscheduler, 0, 1, &param as *const _)
            };
            if r2 != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
                unsafe { libc::_exit(12) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "RestrictRealtime child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_restrict_suid_sgid_filter() {
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        let action = SECCOMP_RET_ERRNO | (libc::EPERM as u32);
        // chmod's mode is args[1] (offset 24); one rule per suid/sgid bit.
        let rules: Vec<(i64, Vec<(u32, u32)>)> = vec![
            (libc::SYS_chmod, vec![(24, libc::S_ISUID as u32)]),
            (libc::SYS_chmod, vec![(24, libc::S_ISGID as u32)]),
        ];
        let path = c"/tmp/rs-scf-suid-test";
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // Create an owned file before confining, so the chmod EPERM below can
            // only come from the filter (owner chmod would otherwise succeed).
            let fd = unsafe { libc::open(path.as_ptr(), libc::O_CREAT | libc::O_WRONLY, 0o644) };
            if fd < 0 {
                unsafe { libc::_exit(10) };
            }
            unsafe { libc::close(fd) };
            if !install_seccomp_program(&build_seccomp_arg_masked_filter(&rules, action)) {
                unsafe { libc::_exit(11) };
            }
            // Setting the setuid bit is blocked with EPERM.
            let r = unsafe { libc::syscall(libc::SYS_chmod, path.as_ptr(), 0o4755) };
            if r != -1 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EPERM) {
                unsafe { libc::_exit(12) };
            }
            // A plain mode (no setuid/setgid bit) is still allowed.
            let r2 = unsafe { libc::syscall(libc::SYS_chmod, path.as_ptr(), 0o644) };
            if r2 != 0 {
                unsafe { libc::_exit(13) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        unsafe {
            libc::unlink(path.as_ptr());
        }
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "RestrictSUIDSGID child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_protect_syscall_sets_resolve() {
        // ProtectKernelModules= blocks the module syscalls.
        let (mut m, mut vm) = (Vec::new(), Vec::new());
        resolve_syscall_set(&["@module"], &mut m, &mut vm);
        assert!(m.contains(&libc::SYS_init_module));
        assert!(m.contains(&libc::SYS_finit_module));
        assert!(m.contains(&libc::SYS_delete_module));
        // ProtectClock= blocks the clock-setting syscalls.
        let (mut c, mut vc) = (Vec::new(), Vec::new());
        resolve_syscall_set(&["@clock"], &mut c, &mut vc);
        assert!(c.contains(&libc::SYS_clock_settime));
        assert!(c.contains(&libc::SYS_settimeofday));
        assert!(c.contains(&libc::SYS_adjtimex));
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_resolve_syscall_set() {
        // @mount expands to the mount-family syscalls.
        let (mut out, mut visited) = (Vec::new(), Vec::new());
        resolve_syscall_set(&["@mount"], &mut out, &mut visited);
        assert!(out.contains(&libc::SYS_mount));
        assert!(out.contains(&libc::SYS_umount2));
        assert!(out.contains(&libc::SYS_pivot_root));

        // @system-service recursively pulls in its sub-groups (execve via
        // @default, read via @basic-io) and resolves to a broad set.
        let (mut out2, mut visited2) = (Vec::new(), Vec::new());
        resolve_syscall_set(&["@system-service"], &mut out2, &mut visited2);
        assert!(out2.contains(&libc::SYS_execve));
        assert!(out2.contains(&libc::SYS_read));
        assert!(
            out2.len() > 100,
            "@system-service should resolve to many syscalls, got {}",
            out2.len()
        );

        // A concrete name resolves; unknown group and unknown syscall are skipped.
        let (mut out3, mut visited3) = (Vec::new(), Vec::new());
        resolve_syscall_set(
            &["mkdir", "@no-such-group", "not_a_syscall"],
            &mut out3,
            &mut visited3,
        );
        assert_eq!(out3, vec![libc::SYS_mkdir]);
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_seccomp_allow_filter() {
        const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
        let action = SECCOMP_RET_ERRNO | (libc::EPERM as u32);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // Allow only getpid + exit(_group); everything else gets EPERM.
            let allowed = [libc::SYS_getpid, libc::SYS_exit_group, libc::SYS_exit];
            if !install_seccomp_program(&build_seccomp_allow_filter(&allowed, action)) {
                unsafe { libc::_exit(10) };
            }
            // An allowed syscall still works.
            if unsafe { libc::syscall(libc::SYS_getpid) } <= 0 {
                unsafe { libc::_exit(11) };
            }
            // A syscall outside the allow-list is blocked (EPERM).
            let r = unsafe {
                libc::syscall(libc::SYS_mkdir, c"/tmp/rs-scf-allow".as_ptr(), 0o755)
            };
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if r != -1 || e != libc::EPERM {
                unsafe { libc::_exit(12) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "allow-list child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_seccomp_log_filter_is_nonblocking() {
        const SECCOMP_RET_LOG: u32 = 0x7ffc_0000;
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // SystemCallLog=getpid (allow-list): getpid -> LOG, everything else
            // ALLOW. LOG permits the syscall, so nothing is ever blocked.
            let prog = build_seccomp_deny_filter(&[libc::SYS_getpid], SECCOMP_RET_LOG);
            if !install_seccomp_program(&prog) {
                unsafe { libc::_exit(10) };
            }
            // The logged syscall still returns normally.
            if unsafe { libc::syscall(libc::SYS_getpid) } <= 0 {
                unsafe { libc::_exit(11) };
            }
            // A non-logged syscall (default ALLOW) also works.
            if unsafe { libc::syscall(libc::SYS_getppid) } <= 0 {
                unsafe { libc::_exit(12) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "log-filter child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_seccomp_arch_filter_allows_native() {
        const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;
        const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // Restrict to x86_64 only; native syscalls must still work (only a
            // non-native arch, e.g. the i386 compat ABI, would be killed).
            let prog = build_seccomp_arch_filter(&[AUDIT_ARCH_X86_64], SECCOMP_RET_KILL_PROCESS);
            if !install_seccomp_program(&prog) {
                unsafe { libc::_exit(10) };
            }
            if unsafe { libc::syscall(libc::SYS_getpid) } <= 0 {
                unsafe { libc::_exit(11) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "arch-filter child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_restrict_address_families_filter() {
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // Allow-list AF_UNIX only.
            let prog = build_restrict_address_families_filter(
                &[libc::AF_UNIX as u32],
                false,
                libc::EAFNOSUPPORT as u32,
            );
            if !install_seccomp_program(&prog) {
                unsafe { libc::_exit(10) };
            }
            // An AF_UNIX socket is permitted.
            let s = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
            if s < 0 {
                unsafe { libc::_exit(11) };
            }
            unsafe { libc::close(s) };
            // An AF_INET socket is blocked with EAFNOSUPPORT.
            let s2 = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if s2 >= 0 || e != libc::EAFNOSUPPORT {
                unsafe { libc::_exit(12) };
            }
            unsafe { libc::_exit(0) };
        }
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(libc::WIFEXITED(status), "child did not exit normally");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "restrict-address-families child failed (code {})",
            libc::WEXITSTATUS(status)
        );
    }
    use aes_gcm::aead::OsRng;

    /// Build an encrypted credential blob in our wire format using null-key sealing.
    fn make_encrypted_blob(
        plaintext: &[u8],
        cred_name: &str,
        seal_type: u32,
        timestamp: u64,
        not_after: u64,
    ) -> Vec<u8> {
        // Derive key
        let aes_key: [u8; 32] = if seal_type == CRED_SEAL_NULL {
            let mut h = Sha256::new();
            h.update(cred_name.as_bytes());
            h.finalize().into()
        } else {
            panic!("test helper only supports null seal");
        };

        let cipher = Aes256Gcm::new_from_slice(&aes_key).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, plaintext).unwrap();

        let name_bytes = cred_name.as_bytes();
        let name_len = name_bytes.len() as u32;

        let mut blob = Vec::new();
        blob.extend_from_slice(&CRED_MAGIC);
        blob.extend_from_slice(&seal_type.to_le_bytes());
        blob.extend_from_slice(&timestamp.to_le_bytes());
        blob.extend_from_slice(&not_after.to_le_bytes());
        blob.extend_from_slice(&name_len.to_le_bytes());
        blob.extend_from_slice(name_bytes);
        blob.extend_from_slice(nonce.as_slice());
        blob.extend_from_slice(&ciphertext);
        blob
    }

    fn now_usec() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }

    #[test]
    fn test_try_decrypt_null_key_roundtrip() {
        let plaintext = b"super-secret-password";
        let cred_name = "db-pass";
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_ok(), "decryption failed: {:?}", result.err());
        assert_eq!(result.unwrap(), plaintext);
    }

    #[test]
    fn test_try_decrypt_base64_encoded_roundtrip() {
        let plaintext = b"hello-credential";
        let cred_name = "test-cred";
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        // Base64-encode the blob (as systemd-creds would output).
        let b64 = BASE64.encode(&blob);

        let result = try_decrypt_credential(b64.as_bytes(), cred_name);
        assert!(
            result.is_ok(),
            "base64 decryption failed: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap(), plaintext);
    }

    #[test]
    fn test_try_decrypt_bad_magic() {
        let mut blob = vec![0u8; 100];
        blob[0] = 0xFF; // corrupt magic
        let result = try_decrypt_credential(&blob, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic"));
    }

    #[test]
    fn test_try_decrypt_truncated_header() {
        let blob = vec![0x73, 0x48, 0x63, 0x00]; // just the magic, no more
        let result = try_decrypt_credential(&blob, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn test_try_decrypt_expired_credential() {
        let plaintext = b"expired-data";
        let cred_name = "expiring";
        // not_after = 1 µs after epoch → already expired
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, 0, 1);

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_try_decrypt_not_expired_credential() {
        let plaintext = b"still-valid";
        let cred_name = "future";
        let not_after = now_usec() + 3_600_000_000; // 1 hour from now
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), not_after);

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), plaintext);
    }

    #[test]
    fn test_try_decrypt_empty_plaintext() {
        let plaintext = b"";
        let cred_name = "empty";
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"");
    }

    #[test]
    fn test_try_decrypt_large_payload() {
        let plaintext: Vec<u8> = (0..8192).map(|i| (i % 256) as u8).collect();
        let cred_name = "big";
        let blob = make_encrypted_blob(&plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), plaintext);
    }

    #[test]
    fn test_try_decrypt_corrupted_ciphertext() {
        let plaintext = b"important";
        let cred_name = "test";
        let mut blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        // Corrupt the last byte (part of the GCM authentication tag).
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("decryption failed"));
    }

    #[test]
    fn test_try_decrypt_wrong_credential_name() {
        // Decrypting with the wrong name should fail because the
        // AES key is derived from the credential name.
        let plaintext = b"secret";
        let cred_name = "correct-name";
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        let result = try_decrypt_credential(&blob, "wrong-name");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("decryption failed"));
    }

    #[test]
    fn test_try_decrypt_unsupported_seal_type() {
        let plaintext = b"data";
        let cred_name = "test";
        // Use seal type 99 (unsupported).
        let mut blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);
        // Overwrite seal_type field at offset 4..8.
        blob[4..8].copy_from_slice(&99u32.to_le_bytes());

        let result = try_decrypt_credential(&blob, cred_name);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsupported seal type"));
    }

    #[test]
    fn test_try_decrypt_not_a_credential_blob() {
        // Plain text that isn't a credential blob at all should fail gracefully.
        let result = try_decrypt_credential(b"just plain text data", "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_try_decrypt_base64_with_whitespace() {
        let plaintext = b"whitespace-test";
        let cred_name = "ws";
        let blob = make_encrypted_blob(plaintext, cred_name, CRED_SEAL_NULL, now_usec(), 0);

        // Base64-encode and insert whitespace/newlines (as might appear in unit files).
        let b64 = BASE64.encode(&blob);
        let with_ws = format!("  {}  \n  ", b64);

        let result = try_decrypt_credential(with_ws.as_bytes(), cred_name);
        assert!(
            result.is_ok(),
            "whitespace base64 decryption failed: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap(), plaintext);
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("*.txt", "file.txt"));
        assert!(!glob_match("*.txt", "file.rs"));
        assert!(glob_match("pre*suf", "pre-middle-suf"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("h?llo", "hello"));
        assert!(glob_match("h?llo", "hallo"));
        assert!(!glob_match("h?llo", "hllo"));
    }

    #[test]
    fn test_glob_match_combined() {
        assert!(glob_match("*.service", "sshd.service"));
        assert!(glob_match("my-cred-?", "my-cred-a"));
        assert!(!glob_match("my-cred-?", "my-cred-ab"));
    }

    #[test]
    fn test_expand_env_str_dollar_dollar() {
        assert_eq!(expand_env_str("$$"), "$");
        assert_eq!(expand_env_str("/proc/$$/comm"), "/proc/$/comm");
        assert_eq!(expand_env_str("a$$b"), "a$b");
    }

    #[test]
    fn test_expand_env_str_braced_var() {
        // SAFETY: tests run single-threaded via --test-threads=1
        unsafe { std::env::set_var("TEST_EXPAND_FOO", "/tmp/creds") };
        assert_eq!(
            expand_env_str("${TEST_EXPAND_FOO}/passwd"),
            "/tmp/creds/passwd"
        );
        assert_eq!(expand_env_str("${TEST_EXPAND_FOO}"), "/tmp/creds");
        unsafe { std::env::remove_var("TEST_EXPAND_FOO") };
    }

    #[test]
    fn test_expand_env_str_unbraced_var() {
        unsafe { std::env::set_var("TEST_EXPAND_BAR", "hello") };
        assert_eq!(expand_env_str("$TEST_EXPAND_BAR"), "hello");
        assert_eq!(expand_env_str("$TEST_EXPAND_BAR/world"), "hello/world");
        unsafe { std::env::remove_var("TEST_EXPAND_BAR") };
    }

    #[test]
    fn test_expand_env_str_unset_var() {
        // Unset variables expand to empty string
        unsafe { std::env::remove_var("TEST_EXPAND_NONEXISTENT") };
        assert_eq!(expand_env_str("${TEST_EXPAND_NONEXISTENT}"), "");
        assert_eq!(expand_env_str("$TEST_EXPAND_NONEXISTENT"), "");
    }

    #[test]
    fn test_expand_env_str_no_expansion() {
        // $2, ${FOO[1]}, trailing $ — not expanded
        assert_eq!(expand_env_str("$2"), "$2");
        assert_eq!(expand_env_str("${FOO[1]}"), "${FOO[1]}");
        assert_eq!(expand_env_str("end$"), "end$");
        assert_eq!(expand_env_str("no vars here"), "no vars here");
    }
}
