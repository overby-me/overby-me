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
fn cunescape(s: &str) -> Vec<u8> {
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

    /// When true, the command has the ':' prefix: use a clean environment
    /// with only the minimal set of variables (PATH, NOTIFY_SOCKET,
    /// LISTEN_FDS/LISTEN_FDNAMES/LISTEN_PID if applicable) instead of the
    /// service's configured Environment=/EnvironmentFile=/PassEnvironment=.
    #[serde(default)]
    pub clean_environment: bool,

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
    /// Path to the TTY device to use when StandardInput=tty/tty-force/tty-fail.
    /// Defaults to /dev/console if not set.
    pub tty_path: Option<PathBuf>,

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

    /// RestrictNamespaces= — namespace restriction.
    /// "yes" = all denied, "no" = all allowed, or space-separated list.
    /// See systemd.exec(5).
    #[serde(default)]
    pub restrict_namespaces: String,

    /// SystemCallArchitectures= — allowed syscall architectures.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_architectures: Vec<String>,

    /// SystemCallFilter= — seccomp syscall filter.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_filter: Vec<String>,

    /// SystemCallLog= — seccomp syscall audit logging.
    /// Same syntax as SystemCallFilter= but logs rather than blocks.
    /// See systemd.exec(5).
    #[serde(default)]
    pub system_call_log: Vec<String>,

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
/// Falls back to "/bin/sh" if the lookup fails or the shell field is empty.
fn get_login_shell(uid: libc::uid_t) -> String {
    let pwd = unsafe { libc::getpwuid(uid) };
    if !pwd.is_null() {
        let shell = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_shell) };
        let shell = shell.to_string_lossy().into_owned();
        if !shell.is_empty() {
            return shell;
        }
    }
    "/bin/sh".to_owned()
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
        StandardInput::Null | StandardInput::Socket => {}
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

    const SOCKET_PATH: &str = "/run/systemd/journal/stdout";

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
            SOCKET_PATH,
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
        return;
    }

    if config.stderr_is_journal
        && let Some(fd) = open_journal_stream_nonblock(
            SOCKET_PATH,
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
            // Open /dev/null as stdin
            let null_fd =
                unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
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
        StandardInput::Socket => {
            // For Accept=yes socket-activated services, dup the first
            // LISTEN_FD (fd 3) to stdin. The accepted connection fd was
            // placed at fd 3 by the fork_child fd duplication logic.
            let listen_fd = libc::STDERR_FILENO + 1; // fd 3
            unsafe {
                libc::dup2(listen_fd, libc::STDIN_FILENO);
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
    let config: ExecHelperConfig = match serde_json::from_reader(std::io::stdin()) {
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
        StandardInput::Tty | StandardInput::TtyForce | StandardInput::TtyFail => {
            if config.tty_reset || config.tty_vhangup || config.tty_vt_disallocate {
                tty_reset_destructive(&config);
            }
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
    if !config.state_directory.is_empty() {
        let base = Path::new("/var/lib");
        let mode = config.state_directory_mode.unwrap_or(0o755);
        let mut full_paths = Vec::new();
        for dir_name in &config.state_directory {
            let full_path = base.join(dir_name);
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create state directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            // Apply StateDirectoryMode=
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                if let Err(e) = std::fs::set_permissions(&full_path, perms) {
                    log::warn!(
                        "Failed to set mode {:o} on state directory {:?}: {}",
                        mode,
                        full_path,
                        e
                    );
                }
            }
            // Set ownership to the service user/group
            let uid = nix::unistd::Uid::from_raw(config.user);
            let gid = nix::unistd::Gid::from_raw(config.group);
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown state directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            full_paths.push(full_path.to_string_lossy().into_owned());
        }
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("STATE_DIRECTORY", full_paths.join(":")) };
    }

    if !config.logs_directory.is_empty() {
        let base = Path::new("/var/log");
        let mode = config.logs_directory_mode.unwrap_or(0o755);
        let mut full_paths = Vec::new();
        for dir_name in &config.logs_directory {
            let full_path = base.join(dir_name);
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create logs directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            // Apply LogsDirectoryMode=
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                if let Err(e) = std::fs::set_permissions(&full_path, perms) {
                    log::error!(
                        "Failed to set mode {:o} on logs directory {:?}: {}",
                        mode,
                        full_path,
                        e
                    );
                    std::process::exit(1);
                }
            }
            // Set ownership to the service user/group
            let uid = nix::unistd::Uid::from_raw(config.user);
            let gid = nix::unistd::Gid::from_raw(config.group);
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown logs directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            full_paths.push(full_path.to_string_lossy().into_owned());
        }
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("LOGS_DIRECTORY", full_paths.join(":")) };
    }

    if !config.runtime_directory.is_empty() {
        let base = Path::new("/run");
        let mode = config.runtime_directory_mode.unwrap_or(0o755);
        let mut full_paths = Vec::new();
        for dir_name in &config.runtime_directory {
            let full_path = base.join(dir_name);
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create runtime directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            // Apply RuntimeDirectoryMode=
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                if let Err(e) = std::fs::set_permissions(&full_path, perms) {
                    log::warn!(
                        "Failed to set mode {:o} on runtime directory {:?}: {}",
                        mode,
                        full_path,
                        e
                    );
                }
            }
            // Set ownership to the service user/group
            let uid = nix::unistd::Uid::from_raw(config.user);
            let gid = nix::unistd::Gid::from_raw(config.group);
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown runtime directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            full_paths.push(full_path.to_string_lossy().into_owned());
        }
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("RUNTIME_DIRECTORY", full_paths.join(":")) };
    }

    // ── Create CacheDirectory= directories under /var/cache/ ──────────
    if !config.cache_directory.is_empty() {
        let base = Path::new("/var/cache");
        let mode = config.cache_directory_mode.unwrap_or(0o755);
        let mut full_paths = Vec::new();
        for dir_name in &config.cache_directory {
            let full_path = base.join(dir_name);
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!("Failed to create cache directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            // Apply CacheDirectoryMode=
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                if let Err(e) = std::fs::set_permissions(&full_path, perms) {
                    log::warn!(
                        "Failed to set mode {:o} on cache directory {:?}: {}",
                        mode,
                        full_path,
                        e
                    );
                }
            }
            // Set ownership to the service user/group
            let uid = nix::unistd::Uid::from_raw(config.user);
            let gid = nix::unistd::Gid::from_raw(config.group);
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!("Failed to chown cache directory {:?}: {}", full_path, e);
                std::process::exit(1);
            }
            full_paths.push(full_path.to_string_lossy().into_owned());
        }
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("CACHE_DIRECTORY", full_paths.join(":")) };
    }

    // ── Create ConfigurationDirectory= directories under /etc/ ────────
    if !config.configuration_directory.is_empty() {
        let base = Path::new("/etc");
        let mode = config.configuration_directory_mode.unwrap_or(0o755);
        let mut full_paths = Vec::new();
        for dir_name in &config.configuration_directory {
            let full_path = base.join(dir_name);
            if let Err(e) = std::fs::create_dir_all(&full_path) {
                log::error!(
                    "Failed to create configuration directory {:?}: {}",
                    full_path,
                    e
                );
                std::process::exit(1);
            }
            // Apply ConfigurationDirectoryMode=
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(mode);
                if let Err(e) = std::fs::set_permissions(&full_path, perms) {
                    log::warn!(
                        "Failed to set mode {:o} on configuration directory {:?}: {}",
                        mode,
                        full_path,
                        e
                    );
                }
            }
            // Set ownership to the service user/group
            let uid = nix::unistd::Uid::from_raw(config.user);
            let gid = nix::unistd::Gid::from_raw(config.group);
            if let Err(e) = nix::unistd::chown(&full_path, Some(uid), Some(gid)) {
                log::error!(
                    "Failed to chown configuration directory {:?}: {}",
                    full_path,
                    e
                );
                std::process::exit(1);
            }
            full_paths.push(full_path.to_string_lossy().into_owned());
        }
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("CONFIGURATION_DIRECTORY", full_paths.join(":")) };
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
    // hostname changes from the host. In real systemd, "yes" also uses
    // seccomp to block sethostname()/setdomainname() within the namespace;
    // we don't have seccomp yet, so both modes behave like "private" for now.
    if config.protect_hostname && !config.privileged_prefix {
        let ret = unsafe { libc::unshare(libc::CLONE_NEWUTS) };
        if ret != 0 {
            log::warn!(
                "Failed to create UTS namespace for ProtectHostname=: {}",
                std::io::Error::last_os_error()
            );
            // Non-fatal: continue without UTS isolation
        } else if let Some(ref hostname) = config.protect_hostname_name {
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

    // ── PrivateUsers= — user namespace ─────────────────────────────────
    if config.private_users && !config.privileged_prefix {
        // Capture uid/gid BEFORE unshare — after creating the user namespace
        // the process has no mapping yet and getuid()/getgid() return 65534.
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };
        let ret = unsafe { libc::unshare(libc::CLONE_NEWUSER) };
        if ret != 0 {
            log::warn!(
                "Failed to create user namespace for PrivateUsers=: {}",
                std::io::Error::last_os_error()
            );
        } else {
            // Write uid_map and gid_map for minimal identity mapping
            // (map root inside the namespace to the original uid outside).
            let _ = std::fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"));
            // Must deny setgroups before writing gid_map (kernel requirement)
            let _ = std::fs::write("/proc/self/setgroups", "deny\n");
            let _ = std::fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"));
        }
    }

    // ── PrivatePIDs= — PID namespace /proc remount ─────────────────────
    // The process is already PID 1 in a new PID namespace (clone was called
    // with CLONE_NEWPID in start_service). We just need to remount /proc to
    // reflect the new namespace.
    if config.private_pids && !config.privileged_prefix {
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

    // Resolve ambient capabilities BEFORE dropping privileges so we can
    // set PR_SET_KEEPCAPS and retain them across the UID change.
    let ambient_caps = resolve_ambient_caps(&config.ambient_capabilities);

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
        match crate::platform::drop_privileges(
            nix::unistd::Gid::from_raw(config.group),
            &supp_gids,
            nix::unistd::Uid::from_raw(config.user),
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
    }

    // setup environment vars
    // When the ':' prefix is used, start with a clean environment — only
    // the minimal internal variables (PATH, NOTIFY_SOCKET, LISTEN_*) are
    // kept. All other configured Environment=/EnvironmentFile=/PassEnvironment=
    // variables are discarded.
    if config.clean_environment {
        // Clear all inherited environment variables first
        for (key, _) in std::env::vars() {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var(&key) };
        }
        // Only set the essential internal variables from the config
        for (k, v) in &config.env {
            // Keep only PATH, NOTIFY_SOCKET, LISTEN_FDS, LISTEN_FDNAMES,
            // CREDENTIALS_DIRECTORY, STATE_DIRECTORY, RUNTIME_DIRECTORY,
            // LOGS_DIRECTORY, CACHE_DIRECTORY, CONFIGURATION_DIRECTORY,
            // and any *_DIRECTORY vars we set above.
            match k.as_str() {
                "PATH" | "NOTIFY_SOCKET" | "LISTEN_FDS" | "LISTEN_FDNAMES" => {
                    unsafe { std::env::set_var(k, v) };
                }
                _ => {
                    log::trace!("':' prefix: skipping env var {k}");
                }
            }
        }
    } else {
        for (k, v) in &config.env {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var(k, v) };
        }
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
    // Applied via prctl: if NoNewPrivileges is also set, the kernel will
    // prevent gaining SCHED_FIFO/SCHED_RR after exec. For full enforcement
    // seccomp would be needed; we log a note but apply what we can.
    // (The prctl approach relies on NoNewPrivileges being set to be effective.)

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
    effective_args = expand_env_argv(&effective_args);
    effective_cmd = PathBuf::from(expand_env_str(&effective_cmd.to_string_lossy()));

    let (cmd, args) = prepare_exec_args(
        &effective_cmd,
        &effective_args,
        config.login_shell || config.use_first_arg_as_argv0,
    );

    // Use execvp instead of execv so bare command names (e.g. "sh" from
    // ExecStart=sh -c ...) are resolved via PATH, matching systemd behavior.
    match nix::unistd::execvp(&cmd, &args) {
        Ok(_infallible) => unreachable!(),
        Err(e) => {
            log::error!("execvp FAILED for {}: {}", cmd.to_string_lossy(), e,);
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
            // Re-mount API filesystems and writable paths back to read-write.
            // The recursive read-only remount above affects ALL submounts,
            // so we must explicitly restore writability for paths that
            // services need. This matches systemd's behavior where
            // ProtectSystem=strict keeps /dev, /proc, /sys, /run, /tmp
            // writable (they are API/runtime filesystems).
            for rw_path in &[
                "/dev", "/proc", "/sys", "/run", "/tmp", "/var/tmp", "/var/log",
            ] {
                if Path::new(rw_path).exists() {
                    bind_mount_readwrite(rw_path, config);
                }
            }
            // Also restore writability for the NixOS store — it's already
            // read-only by nature but bind-mounting it avoids EROFS errors
            // when services try to follow symlinks through it.
        }
        _ => {} // "no" or unrecognized
    }

    log::trace!("mount_ns: ProtectSystem done, implicit RW paths...");
    // ── Implicit ReadWritePaths from RuntimeDirectory=/StateDirectory=/LogsDirectory=
    // When ProtectSystem=strict is active, the service's runtime, state, and
    // logs directories must be explicitly writable. systemd handles this
    // implicitly; we do the same.
    if config.protect_system == "strict" {
        for dir_name in &config.runtime_directory {
            let full = format!("/run/{}", dir_name);
            if Path::new(&full).exists() {
                bind_mount_readwrite(&full, config);
            }
        }
        for dir_name in &config.state_directory {
            let full = format!("/var/lib/{}", dir_name);
            if Path::new(&full).exists() {
                bind_mount_readwrite(&full, config);
            }
        }
        for dir_name in &config.logs_directory {
            let full = format!("/var/log/{}", dir_name);
            if Path::new(&full).exists() {
                bind_mount_readwrite(&full, config);
            }
        }
        for dir_name in &config.cache_directory {
            let full = format!("/var/cache/{}", dir_name);
            if Path::new(&full).exists() {
                bind_mount_readwrite(&full, config);
            }
        }
        for dir_name in &config.configuration_directory {
            let full = format!("/etc/{}", dir_name);
            if Path::new(&full).exists() {
                bind_mount_readwrite(&full, config);
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
        "mount_ns: ProtectControlGroups={}...",
        config.protect_control_groups
    );
    // ── ProtectControlGroups= ─────────────────────────────────────────
    if config.protect_control_groups {
        remount_read_only("/sys/fs/cgroup", config);
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

    // Step 2: mount TemporaryFileSystem
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
            let opts = if options.is_empty() {
                "mode=0755".to_string()
            } else {
                format!("mode=0755,{}", options)
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

    log::trace!("mount_ns: ALL STEPS COMPLETE");
}

/// Bind-mount a path on top of itself with MS_RDONLY.
fn remount_read_only(path: &str, _config: &ExecHelperConfig) {
    let c_path = match std::ffi::CString::new(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    if !Path::new(path).exists() {
        return;
    }
    // First bind-mount the path on itself
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
fn expand_env_str(s: &str) -> String {
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
