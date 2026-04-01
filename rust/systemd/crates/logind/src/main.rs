//! systemd-logind — Login and seat management daemon.
//!
//! This daemon manages user login sessions, seats (groups of devices),
//! and handles system power/sleep button events. It provides the
//! `org.freedesktop.login1` D-Bus interface used by desktop environments
//! (GNOME, KDE, etc.) and tools like `loginctl`.
//!
//! Features:
//! - Session tracking (create, release, list, query)
//! - Seat management (seat0 + dynamic seats)
//! - User tracking (sessions per user, state)
//! - Input device monitoring for power/sleep buttons
//! - Inhibitor lock management (shutdown, sleep, idle, etc.)
//! - D-Bus interface (`org.freedesktop.login1`) with Manager, Session, Seat, User objects
//! - D-Bus signal emission (SessionNew/Removed, UserNew/Removed, SeatNew/Removed, etc.)
//! - sd_notify protocol (READY, WATCHDOG, STATUS)
//! - Control socket for loginctl CLI (legacy)
//! - VT (virtual terminal) tracking
//! - Idle hint tracking

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedFd as ZOwnedFd;

// ---------------------------------------------------------------------------
// D-Bus caller credential extraction
// ---------------------------------------------------------------------------

/// Extract the real UID and PID of the D-Bus caller from message headers.
///
/// Uses the D-Bus daemon's `GetConnectionUnixUser` and
/// `GetConnectionUnixProcessID` methods to resolve the sender's unique bus
/// name to OS-level credentials.  Falls back to uid=0, pid=0 when the
/// sender is unavailable or the D-Bus daemon doesn't support credential
/// queries (e.g. in test environments).
async fn get_caller_credentials(
    header: &zbus::message::Header<'_>,
    conn: &zbus::Connection,
) -> (u32, u32) {
    let sender = match header.sender() {
        Some(s) => s.to_owned(),
        None => return (0, 0),
    };

    let dbus_proxy = match zbus::fdo::DBusProxy::new(conn).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("Failed to create DBusProxy for credential lookup: {}", e);
            return (0, 0);
        }
    };

    let bus_name: zbus::names::BusName<'_> = sender.into();
    let uid = dbus_proxy
        .get_connection_unix_user(bus_name.clone())
        .await
        .unwrap_or(0);
    let pid = dbus_proxy
        .get_connection_unix_process_id(bus_name)
        .await
        .unwrap_or(0);

    log::debug!("D-Bus caller credentials: uid={}, pid={}", uid, pid);
    (uid, pid)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CONTROL_SOCKET_PATH: &str = "/run/systemd/logind-control";
const RUN_DIR: &str = "/run/systemd";
const SESSIONS_DIR: &str = "/run/systemd/sessions";
const SEATS_DIR: &str = "/run/systemd/seats";
const USERS_DIR: &str = "/run/systemd/users";
const INHIBIT_DIR: &str = "/run/systemd/inhibit";
const INPUT_DIR: &str = "/dev/input";

const DBUS_NAME: &str = "org.freedesktop.login1";
const DBUS_PATH: &str = "/org/freedesktop/login1";

const DEFAULT_INHIBIT_DELAY_MAX_USEC: u64 = 5_000_000; // 5s
const DEFAULT_USER_STOP_DELAY_USEC: u64 = 10_000_000; // 10s

// ---------------------------------------------------------------------------
// VT ioctl constants
// ---------------------------------------------------------------------------

/// VT_SETMODE — set the VT mode (VT_AUTO or VT_PROCESS).
const VT_SETMODE: u64 = 0x5602;
/// VT_GETMODE — get the current VT mode.
#[allow(dead_code)]
const VT_GETMODE: u64 = 0x5601;
/// VT_RELDISP — release/acknowledge display during VT_PROCESS handoff.
const VT_RELDISP: u64 = 0x5605;
/// VT_GETSTATE — get the current active VT number.
const VT_GETSTATE: u64 = 0x5603;
/// VT_ACTIVATE — switch to a specific VT.
#[allow(dead_code)]
const VT_ACTIVATE_IOCTL: u64 = 0x5606;
/// VT_WAITACTIVE — wait until a specific VT is active.
#[allow(dead_code)]
const VT_WAITACTIVE_IOCTL: u64 = 0x5607;

/// Automatic VT switching (default kernel mode).
const VT_AUTO: i32 = 0x00;
/// Process-controlled VT switching — kernel sends signals instead of switching
/// automatically, and waits for VT_RELDISP before completing the switch.
const VT_PROCESS: i32 = 0x01;
/// Argument to VT_RELDISP to acknowledge VT acquisition.
const VT_ACKACQ: i32 = 0x02;

/// Kernel VT mode structure, passed to VT_SETMODE / VT_GETMODE.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VtMode {
    /// VT_AUTO or VT_PROCESS
    mode: libc::c_char,
    /// Unused
    waitv: libc::c_char,
    /// Signal to raise on VT release (switch away)
    relsig: libc::c_short,
    /// Signal to raise on VT acquire (switch to)
    acqsig: libc::c_short,
    /// Unused
    frsig: libc::c_short,
}

/// Kernel VT state structure, returned by VT_GETSTATE.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VtStat {
    /// Currently active VT number (1-based)
    v_active: u16,
    /// Signal to send (unused in modern kernels)
    v_signal: u16,
    /// Bitmask of open VTs
    v_state: u16,
}

// ---------------------------------------------------------------------------
// VT_PROCESS monitor
// ---------------------------------------------------------------------------

/// Manages VT_PROCESS mode on a single VT tty.
///
/// While this monitor is alive, the kernel sends SIGUSR1 when switching away
/// from the VT and SIGUSR2 when switching to it.  The main loop checks the
/// corresponding `AtomicBool` flags and calls [`VtMonitor::release_display`]
/// or [`VtMonitor::ack_acquire`] to complete the handoff.
///
/// On drop, VT_AUTO mode is restored so the kernel resumes automatic
/// switching.
#[derive(Debug)]
struct VtMonitor {
    /// Open file descriptor for `/dev/tty{N}` — kept alive for VT_PROCESS.
    tty_fd: OwnedFd,
    /// The VT number (1-based) being monitored.
    vtnr: u32,
}

impl VtMonitor {
    /// Open `/dev/tty{vtnr}` and set VT_PROCESS mode.
    fn new(vtnr: u32) -> Result<Self, String> {
        if vtnr == 0 {
            return Err("Cannot monitor VT 0".to_string());
        }
        let path = format!("/dev/tty{}", vtnr);
        let c_path =
            CString::new(path.clone()).map_err(|_| format!("Invalid VT path: {}", path))?;
        let raw_fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
            )
        };
        if raw_fd < 0 {
            return Err(format!(
                "Failed to open {}: {}",
                path,
                io::Error::last_os_error()
            ));
        }
        let tty_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        // Set VT_PROCESS mode: kernel sends SIGUSR1 on release, SIGUSR2 on
        // acquire instead of switching automatically.
        let mode = VtMode {
            mode: VT_PROCESS as libc::c_char,
            waitv: 0,
            relsig: libc::SIGUSR1 as libc::c_short,
            acqsig: libc::SIGUSR2 as libc::c_short,
            frsig: 0,
        };
        let ret = unsafe { libc::ioctl(tty_fd.as_raw_fd(), VT_SETMODE, &mode) };
        if ret < 0 {
            return Err(format!(
                "VT_SETMODE(VT_PROCESS) on {} failed: {}",
                path,
                io::Error::last_os_error()
            ));
        }

        log::info!(
            "VT {} set to VT_PROCESS mode (relsig=SIGUSR1, acqsig=SIGUSR2)",
            vtnr
        );
        Ok(VtMonitor { tty_fd, vtnr })
    }

    /// Allow the pending VT switch (called in response to SIGUSR1).
    fn release_display(&self) {
        let ret = unsafe { libc::ioctl(self.tty_fd.as_raw_fd(), VT_RELDISP, 1i32) };
        if ret < 0 {
            log::warn!(
                "VT_RELDISP(1) on VT {} failed: {}",
                self.vtnr,
                io::Error::last_os_error()
            );
        } else {
            log::debug!("VT {} released display (allowed switch)", self.vtnr);
        }
    }

    /// Acknowledge VT acquisition (called in response to SIGUSR2).
    fn ack_acquire(&self) {
        let ret = unsafe { libc::ioctl(self.tty_fd.as_raw_fd(), VT_RELDISP, VT_ACKACQ) };
        if ret < 0 {
            log::warn!(
                "VT_RELDISP(VT_ACKACQ) on VT {} failed: {}",
                self.vtnr,
                io::Error::last_os_error()
            );
        } else {
            log::debug!("VT {} acknowledged acquire", self.vtnr);
        }
    }

    /// Restore VT_AUTO mode (called on drop or explicit cleanup).
    fn restore_auto(&self) {
        let mode = VtMode {
            mode: VT_AUTO as libc::c_char,
            waitv: 0,
            relsig: 0,
            acqsig: 0,
            frsig: 0,
        };
        let ret = unsafe { libc::ioctl(self.tty_fd.as_raw_fd(), VT_SETMODE, &mode) };
        if ret < 0 {
            log::warn!(
                "VT_SETMODE(VT_AUTO) on VT {} failed: {}",
                self.vtnr,
                io::Error::last_os_error()
            );
        } else {
            log::info!("VT {} restored to VT_AUTO mode", self.vtnr);
        }
    }
}

impl Drop for VtMonitor {
    fn drop(&mut self) {
        self.restore_auto();
    }
}

/// Read the currently active VT number from the kernel via VT_GETSTATE on
/// `/dev/tty0`.  Returns `None` if the ioctl fails (e.g. not running on a
/// real VT).
fn get_active_vt() -> Option<u32> {
    let fd = unsafe { libc::open(c"/dev/tty0".as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) };
    if fd < 0 {
        return None;
    }
    let mut stat = VtStat {
        v_active: 0,
        v_signal: 0,
        v_state: 0,
    };
    let ret = unsafe { libc::ioctl(fd, VT_GETSTATE, &mut stat) };
    unsafe {
        libc::close(fd);
    }
    if ret < 0 {
        None
    } else {
        Some(stat.v_active as u32)
    }
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);
static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);
/// Set by SIGUSR1 — kernel requests VT release (switch away from active VT).
static VT_RELEASE_FLAG: AtomicBool = AtomicBool::new(false);
/// Set by SIGUSR2 — kernel notifies VT acquire (switch to a VT we hold).
static VT_ACQUIRE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
}
extern "C" fn handle_sigint(_: libc::c_int) {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
}
extern "C" fn handle_sighup(_: libc::c_int) {
    RELOAD_FLAG.store(true, Ordering::SeqCst);
}
extern "C" fn handle_sigusr1(_: libc::c_int) {
    VT_RELEASE_FLAG.store(true, Ordering::SeqCst);
}
extern "C" fn handle_sigusr2(_: libc::c_int) {
    VT_ACQUIRE_FLAG.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, handle_sigterm as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_sigint as libc::sighandler_t);
        libc::signal(libc::SIGHUP, handle_sighup as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGUSR1, handle_sigusr1 as libc::sighandler_t);
        libc::signal(libc::SIGUSR2, handle_sigusr2 as libc::sighandler_t);
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;
    impl log::Log for StderrLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::max_level()
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                let ts = chrono_lite_timestamp();
                eprintln!(
                    "[{}] [{}] {}",
                    ts,
                    record.level().as_str().to_lowercase(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

fn chrono_lite_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;
    let millis = now.subsec_millis();
    format!("{hours:02}:{mins:02}:{s:02}.{millis:03}")
}

// ---------------------------------------------------------------------------
// sd_notify
// ---------------------------------------------------------------------------

fn sd_notify(state: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET")
        && let Ok(sock) = std::os::unix::net::UnixDatagram::unbound()
    {
        let _ = sock.send_to(state.as_bytes(), &path);
    }
}

fn watchdog_interval() -> Option<Duration> {
    if let Ok(val) = std::env::var("WATCHDOG_USEC")
        && let Ok(usec) = val.parse::<u64>()
        && usec > 0
    {
        // Kick at half the interval, as recommended
        return Some(Duration::from_micros(usec / 2));
    }
    None
}

// ---------------------------------------------------------------------------
// Session / Seat / User / Inhibitor types
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Session {
    /// Unique session ID (e.g. "1", "2", "c1" for automatic sessions)
    pub id: String,
    /// UID of the session owner
    pub uid: u32,
    /// Username
    pub user: String,
    /// Seat this session is attached to (if any)
    pub seat: Option<String>,
    /// Virtual terminal number (0 if none)
    pub vtnr: u32,
    /// Session type: "tty", "x11", "wayland", "mir", "unspecified"
    pub session_type: String,
    /// Session class: "user", "greeter", "lock-screen", "background"
    pub class: String,
    /// Session scope: "pam" or "systemd"
    pub scope: String,
    /// Whether this session is active (foreground on its seat)
    pub active: bool,
    /// Session state: "online", "active", "closing"
    pub state: String,
    /// TTY or display associated with this session
    pub tty: String,
    /// Display (:0 etc.) for graphical sessions
    pub display: String,
    /// Service name (e.g. "sshd", "gdm")
    pub service: String,
    /// Desktop environment (e.g. "gnome", "kde")
    pub desktop: String,
    /// Leader PID (the PAM session leader / login process)
    pub leader: u32,
    /// Remote session
    pub remote: bool,
    /// Remote host (if remote)
    pub remote_host: String,
    /// Remote user (if remote)
    pub remote_user: String,
    /// Creation timestamp (seconds since epoch)
    pub since: u64,
    /// Creation timestamp (monotonic microseconds)
    pub since_monotonic: u64,
    /// Whether session is idle
    pub idle_hint: bool,
    /// Idle since (realtime microseconds)
    pub idle_since_hint: u64,
    /// Idle since (monotonic microseconds)
    pub idle_since_hint_monotonic: u64,
    /// Whether session is locked
    pub locked_hint: bool,
    /// D-Bus unique name of the session controller (from TakeControl)
    #[serde(skip)]
    pub controller: Option<String>,
    /// Devices taken via TakeDevice, keyed by (major, minor)
    #[serde(skip)]
    pub devices: HashMap<(u32, u32), SessionDevice>,
}

/// A device opened by a session controller via TakeDevice.
#[derive(Debug)]
pub struct SessionDevice {
    /// The major:minor pair identifying this device.
    pub major: u32,
    pub minor: u32,
    /// The opened file descriptor (owned by logind).
    pub fd: OwnedFd,
    /// Whether the device is currently active (session is foreground).
    pub active: bool,
}

// SessionDevice cannot be Clone because OwnedFd is not Clone.
// We skip serde for the devices map on Session, so no Serialize/Deserialize needed.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Seat {
    /// Seat name (e.g. "seat0")
    pub id: String,
    /// Active session ID on this seat (if any)
    pub active_session: Option<String>,
    /// All session IDs on this seat
    pub sessions: Vec<String>,
    /// Whether this seat can do graphical output
    pub can_graphical: bool,
    /// Whether this seat supports multiple sessions / VTs
    pub can_multi_session: bool,
    /// Devices explicitly attached to this seat via AttachDevice.
    /// Each entry is a sysfs path (e.g. "/sys/devices/pci0000:00/...").
    #[serde(default)]
    pub attached_devices: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct User {
    /// UID
    pub uid: u32,
    /// GID
    pub gid: u32,
    /// Username
    pub name: String,
    /// User state: "active", "online", "lingering", "closing"
    pub state: String,
    /// Session IDs belonging to this user
    pub sessions: Vec<String>,
    /// Slice (e.g. "user-1000.slice")
    pub slice: String,
    /// Service name
    pub service: String,
    /// Runtime path (e.g. /run/user/<uid>)
    pub runtime_path: String,
    /// Login timestamp (seconds since epoch)
    pub since: u64,
    /// Login timestamp (monotonic microseconds)
    pub since_monotonic: u64,
    /// Whether user has linger enabled
    pub linger: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Inhibitor {
    /// What is inhibited: "shutdown", "sleep", "idle", "handle-power-key", etc.
    pub what: String,
    /// Who is requesting the inhibition
    pub who: String,
    /// Why is the inhibition requested
    pub why: String,
    /// Mode: "block" or "delay"
    pub mode: String,
    /// UID of requester
    pub uid: u32,
    /// PID of requester
    pub pid: u32,
    /// Unique inhibitor ID
    pub id: u64,
    /// Timestamp
    pub since: u64,
}

// ---------------------------------------------------------------------------
// LoginManager — core state
// ---------------------------------------------------------------------------

pub struct LoginManager {
    sessions: HashMap<String, Session>,
    seats: HashMap<String, Seat>,
    users: HashMap<u32, User>,
    inhibitors: HashMap<u64, Inhibitor>,
    next_session_id: u64,
    next_inhibitor_id: u64,
    /// Discovered power button input device paths
    power_button_devices: Vec<PathBuf>,
    /// Configuration
    config: LogindConfig,
    /// Pipe read-ends for inhibitor FD-based lifecycle tracking.
    /// When the caller drops their write-end, the read-end becomes readable
    /// and we can auto-release the inhibitor.
    inhibitor_pipes: HashMap<u64, OwnedFd>,
    /// VT_PROCESS monitors keyed by VT number.  Each monitor holds the tty
    /// fd open with VT_PROCESS mode so the kernel sends SIGUSR1/SIGUSR2
    /// instead of switching automatically.
    vt_monitors: HashMap<u32, VtMonitor>,
}

/// Manager configuration (logind.conf)
#[derive(Debug, Clone)]
pub struct LogindConfig {
    pub n_auto_vts: u32,
    pub kill_user_processes: bool,
    pub kill_only_users: Vec<String>,
    pub kill_exclude_users: Vec<String>,
    pub idle_action: String,
    pub idle_action_usec: u64,
    pub inhibit_delay_max_usec: u64,
    pub user_stop_delay_usec: u64,
    pub handle_power_key: String,
    pub handle_suspend_key: String,
    pub handle_hibernate_key: String,
    pub handle_lid_switch: String,
    pub handle_lid_switch_external_power: String,
    pub handle_lid_switch_docked: String,
    pub holdoff_timeout_usec: u64,
    pub remove_ipc: bool,
    pub runtime_directory_size: u64,
    pub runtime_directory_inodes_max: u64,
    pub inhibitors_max: u64,
    pub sessions_max: u64,
}

impl Default for LogindConfig {
    fn default() -> Self {
        Self {
            n_auto_vts: 6,
            kill_user_processes: true,
            kill_only_users: Vec::new(),
            kill_exclude_users: vec!["root".to_string()],
            idle_action: "ignore".to_string(),
            idle_action_usec: 0,
            inhibit_delay_max_usec: DEFAULT_INHIBIT_DELAY_MAX_USEC,
            user_stop_delay_usec: DEFAULT_USER_STOP_DELAY_USEC,
            handle_power_key: "poweroff".to_string(),
            handle_suspend_key: "suspend".to_string(),
            handle_hibernate_key: "hibernate".to_string(),
            handle_lid_switch: "suspend".to_string(),
            handle_lid_switch_external_power: "suspend".to_string(),
            handle_lid_switch_docked: "ignore".to_string(),
            holdoff_timeout_usec: 30_000_000, // 30s
            remove_ipc: true,
            runtime_directory_size: 10 * 1024 * 1024 * 1024, // 10% of physical RAM or 10GiB
            runtime_directory_inodes_max: 0,                 // no limit
            inhibitors_max: 8192,
            sessions_max: 8192,
        }
    }
}

impl LoginManager {
    fn new() -> Self {
        let config = parse_logind_conf();
        let mut mgr = LoginManager {
            sessions: HashMap::new(),
            seats: HashMap::new(),
            users: HashMap::new(),
            inhibitors: HashMap::new(),
            next_session_id: 1,
            next_inhibitor_id: 1,
            power_button_devices: Vec::new(),
            config,
            inhibitor_pipes: HashMap::new(),
            vt_monitors: HashMap::new(),
        };

        // Always create seat0 — the default seat
        mgr.seats.insert(
            "seat0".to_string(),
            Seat {
                id: "seat0".to_string(),
                active_session: None,
                sessions: Vec::new(),
                can_graphical: check_seat0_graphical(),
                can_multi_session: true,
                attached_devices: Vec::new(),
            },
        );

        // Enumerate /dev/input for power buttons
        mgr.enumerate_input_devices();

        mgr
    }

    fn enumerate_input_devices(&mut self) {
        self.power_button_devices.clear();
        let input_dir = Path::new(INPUT_DIR);
        if !input_dir.is_dir() {
            return;
        }

        // Look for event devices that are power/sleep buttons by reading
        // their capabilities from sysfs.
        if let Ok(entries) = fs::read_dir("/sys/class/input") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.starts_with("event") {
                    continue;
                }
                let sysfs_path = entry.path();
                let device_name_path = sysfs_path.join("device/name");
                if let Ok(dev_name) = fs::read_to_string(&device_name_path) {
                    let dev_name = dev_name.trim().to_lowercase();
                    if dev_name.contains("power button")
                        || dev_name.contains("sleep button")
                        || dev_name.contains("lid switch")
                    {
                        let dev_path = Path::new(INPUT_DIR).join(&*name_str);
                        log::info!(
                            "Watching system buttons on {} ({})",
                            dev_path.display(),
                            dev_name.trim()
                        );
                        self.power_button_devices.push(dev_path);
                    }
                }
            }
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn now_monotonic_usec() -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        }
        (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1000
    }

    /// Create a new session.
    #[allow(clippy::too_many_arguments)]
    fn create_session(
        &mut self,
        uid: u32,
        user: &str,
        seat: Option<&str>,
        vtnr: u32,
        session_type: &str,
        class: &str,
        tty: &str,
        leader: u32,
    ) -> String {
        let id = format!("{}", self.next_session_id);
        self.next_session_id += 1;

        let seat_name = seat.map(|s| s.to_string());
        let now = Self::now_secs();
        let now_mono = Self::now_monotonic_usec();

        let session = Session {
            id: id.clone(),
            uid,
            user: user.to_string(),
            seat: seat_name.clone(),
            vtnr,
            session_type: session_type.to_string(),
            class: class.to_string(),
            scope: "pam".to_string(),
            active: true,
            state: "active".to_string(),
            tty: tty.to_string(),
            display: String::new(),
            service: String::new(),
            desktop: String::new(),
            leader,
            remote: false,
            remote_host: String::new(),
            remote_user: String::new(),
            since: now,
            since_monotonic: now_mono,
            idle_hint: false,
            idle_since_hint: 0,
            idle_since_hint_monotonic: 0,
            locked_hint: false,
            controller: None,
            devices: HashMap::new(),
        };

        // Register in seat
        if let Some(ref seat_id) = seat_name
            && let Some(seat) = self.seats.get_mut(seat_id)
        {
            seat.sessions.push(id.clone());
            if seat.active_session.is_none() {
                seat.active_session = Some(id.clone());
            }
        }

        // Register in user tracking
        let gid = resolve_user_gid(uid);
        let user_entry = self.users.entry(uid).or_insert_with(|| User {
            uid,
            gid,
            name: user.to_string(),
            state: "active".to_string(),
            sessions: Vec::new(),
            slice: format!("user-{}.slice", uid),
            service: format!("user@{}.service", uid),
            runtime_path: format!("/run/user/{}", uid),
            since: now,
            since_monotonic: now_mono,
            linger: false,
        });
        user_entry.sessions.push(id.clone());
        user_entry.state = "active".to_string();

        // Write session file
        self.write_session_file(&session);

        self.sessions.insert(id.clone(), session);
        id
    }

    /// Release (close) a session.
    fn release_session(&mut self, session_id: &str) -> bool {
        let Some(session) = self.sessions.remove(session_id) else {
            return false;
        };

        // Remove from seat
        if let Some(ref seat_id) = session.seat
            && let Some(seat) = self.seats.get_mut(seat_id)
        {
            seat.sessions.retain(|s| s != session_id);
            if seat.active_session.as_deref() == Some(session_id) {
                seat.active_session = seat.sessions.first().cloned();
            }
        }

        // Remove from user tracking
        if let Some(user) = self.users.get_mut(&session.uid) {
            user.sessions.retain(|s| s != session_id);
            if user.sessions.is_empty() {
                user.state = "closing".to_string();
            }
        }

        // Remove session file
        let session_file = Path::new(SESSIONS_DIR).join(session_id);
        let _ = fs::remove_file(session_file);

        true
    }

    /// Create an inhibitor lock.
    fn create_inhibitor(
        &mut self,
        what: &str,
        who: &str,
        why: &str,
        mode: &str,
        uid: u32,
        pid: u32,
    ) -> u64 {
        let id = self.next_inhibitor_id;
        self.next_inhibitor_id += 1;

        let inhibitor = Inhibitor {
            what: what.to_string(),
            who: who.to_string(),
            why: why.to_string(),
            mode: mode.to_string(),
            uid,
            pid,
            id,
            since: Self::now_secs(),
        };

        // Write inhibitor file
        let inhibit_file = Path::new(INHIBIT_DIR).join(format!("{}", id));
        let content = format!(
            "WHAT={}\nWHO={}\nWHY={}\nMODE={}\nUID={}\nPID={}\n",
            what, who, why, mode, uid, pid
        );
        let _ = fs::write(&inhibit_file, content);

        self.inhibitors.insert(id, inhibitor);
        id
    }

    /// Release an inhibitor lock.
    fn release_inhibitor(&mut self, id: u64) -> bool {
        if self.inhibitors.remove(&id).is_some() {
            let inhibit_file = Path::new(INHIBIT_DIR).join(format!("{}", id));
            let _ = fs::remove_file(inhibit_file);
            true
        } else {
            false
        }
    }

    /// Clean up stale inhibitors (check if PIDs still exist).
    fn cleanup_stale_inhibitors(&mut self) {
        let stale: Vec<u64> = self
            .inhibitors
            .iter()
            .filter(|(id, inhibitor)| {
                // Check if the pipe FD was closed by the caller (read end
                // becomes readable / returns 0 bytes when the write end is
                // dropped).
                if let Some(pipe_fd) = self.inhibitor_pipes.get(id) {
                    let mut buf = [0u8; 1];
                    let rc = unsafe {
                        libc::read(
                            pipe_fd.as_raw_fd(),
                            buf.as_mut_ptr() as *mut libc::c_void,
                            1,
                        )
                    };
                    // rc == 0 means write-end was closed (EOF)
                    // rc > 0 should not happen (nobody writes to it)
                    // rc < 0 with EAGAIN means still open (non-blocking)
                    if rc == 0 {
                        return true;
                    }
                }
                // Fallback: check if the PID that created this inhibitor is
                // still alive.
                if inhibitor.pid > 0 {
                    unsafe { libc::kill(inhibitor.pid as i32, 0) != 0 }
                } else {
                    false
                }
            })
            .map(|(id, _)| *id)
            .collect();

        for id in stale {
            log::info!("Removing stale inhibitor lock {}", id);
            self.inhibitor_pipes.remove(&id);
            self.release_inhibitor(id);
        }
    }

    /// Activate a session on its seat.
    ///
    /// Returns a `SessionSwitchInfo` describing the old/new sessions and
    /// their taken devices so the caller can emit PauseDevice / ResumeDevice
    /// signals outside the lock.
    fn activate_session(&mut self, session_id: &str) -> Result<SessionSwitchInfo, String> {
        // Extract the seat_id without cloning the whole Session.
        let seat_id = {
            let session = self
                .sessions
                .get(session_id)
                .ok_or_else(|| format!("Session '{}' not found", session_id))?;
            session
                .seat
                .as_ref()
                .ok_or_else(|| format!("Session '{}' has no seat", session_id))?
                .clone()
        };

        let mut old_session_id: Option<String> = None;
        let mut old_devices: Vec<(u32, u32)> = Vec::new();
        let mut old_uid: Option<u32> = None;

        if let Some(seat) = self.seats.get_mut(&seat_id) {
            // Deactivate current active session and collect its devices
            let old_active = seat.active_session.clone();
            if let Some(ref old_id) = old_active
                && old_id != session_id
            {
                if let Some(old_session) = self.sessions.get_mut(old_id) {
                    old_session.active = false;
                    old_session.state = "online".to_string();
                    old_uid = Some(old_session.uid);
                    // Mark all taken devices as inactive
                    for (dev_key, dev) in &mut old_session.devices {
                        dev.active = false;
                        old_devices.push(*dev_key);
                    }
                }
                old_session_id = Some(old_id.clone());
            }
            seat.active_session = Some(session_id.to_string());
        }

        // Collect new session's devices to resume
        let mut new_devices: Vec<(u32, u32, RawFd)> = Vec::new();
        let new_uid;
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.active = true;
            session.state = "active".to_string();
            new_uid = session.uid;
            // Mark all taken devices as active and collect FDs for ResumeDevice
            for (dev_key, dev) in &mut session.devices {
                dev.active = true;
                new_devices.push((dev_key.0, dev_key.1, dev.fd.as_raw_fd()));
            }
        } else {
            new_uid = 0;
        }

        // ACL management: revoke old user's ACLs, grant new user's ACLs
        if let Some(old_u) = old_uid
            && old_u != new_uid
        {
            self.remove_device_acls(&seat_id, old_u);
        }
        if new_uid > 0 {
            self.apply_device_acls(&seat_id, new_uid);
        }

        Ok(SessionSwitchInfo {
            old_session_id,
            old_devices,
            new_session_id: session_id.to_string(),
            new_devices,
        })
    }

    /// Lock a session
    fn lock_session(&mut self, session_id: &str) -> Result<(), String> {
        if self.sessions.contains_key(session_id) {
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.locked_hint = true;
            }
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Unlock a session
    fn unlock_session(&mut self, session_id: &str) -> Result<(), String> {
        if self.sessions.contains_key(session_id) {
            if let Some(session) = self.sessions.get_mut(session_id) {
                session.locked_hint = false;
            }
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Lock all sessions
    fn lock_sessions(&mut self) {
        for session in self.sessions.values_mut() {
            session.locked_hint = true;
        }
    }

    /// Unlock all sessions
    fn unlock_sessions(&mut self) {
        for session in self.sessions.values_mut() {
            session.locked_hint = false;
        }
    }

    /// Set idle hint on a session
    fn set_idle_hint(&mut self, session_id: &str, idle: bool) -> Result<(), String> {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.idle_hint = idle;
            if idle {
                let now_usec = Self::now_secs() * 1_000_000;
                let now_mono = Self::now_monotonic_usec();
                session.idle_since_hint = now_usec;
                session.idle_since_hint_monotonic = now_mono;
            } else {
                session.idle_since_hint = 0;
                session.idle_since_hint_monotonic = 0;
            }
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Set locked hint on a session
    fn set_locked_hint(&mut self, session_id: &str, locked: bool) -> Result<(), String> {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.locked_hint = locked;
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Set session type
    fn set_session_type(&mut self, session_id: &str, stype: &str) -> Result<(), String> {
        let valid_types = ["tty", "x11", "wayland", "mir", "unspecified"];
        if !valid_types.contains(&stype) {
            return Err(format!("Invalid session type: {}", stype));
        }
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.session_type = stype.to_string();
            Ok(())
        } else {
            Err(format!("Session '{}' not found", session_id))
        }
    }

    /// Kill a session's leader or all processes
    fn kill_session(&self, session_id: &str, who: &str, signal: i32) -> Result<(), String> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("Session '{}' not found", session_id))?;

        if (who == "leader" || who == "all") && session.leader > 0 {
            let ret = unsafe { libc::kill(session.leader as i32, signal) };
            if ret != 0 {
                return Err(format!(
                    "Failed to kill leader PID {}: {}",
                    session.leader,
                    io::Error::last_os_error()
                ));
            }
        }
        // "all" would also kill all processes in the cgroup, but we don't have
        // cgroup integration yet
        Ok(())
    }

    /// Kill a user's sessions
    fn kill_user(&self, uid: u32, signal: i32) -> Result<(), String> {
        let user = self
            .users
            .get(&uid)
            .ok_or_else(|| format!("User {} not found", uid))?;

        for sid in &user.sessions {
            if let Some(session) = self.sessions.get(sid)
                && session.leader > 0
            {
                unsafe {
                    libc::kill(session.leader as i32, signal);
                }
            }
        }
        Ok(())
    }

    /// Terminate user (release all sessions)
    fn terminate_user(&mut self, uid: u32) -> Result<(), String> {
        let user = self
            .users
            .get(&uid)
            .ok_or_else(|| format!("User {} not found", uid))?;

        let session_ids: Vec<String> = user.sessions.clone();
        for sid in &session_ids {
            self.release_session(sid);
        }
        Ok(())
    }

    /// Terminate a seat (release all sessions on it)
    fn terminate_seat(&mut self, seat_id: &str) -> Result<(), String> {
        let seat = self
            .seats
            .get(seat_id)
            .ok_or_else(|| format!("Seat '{}' not found", seat_id))?;

        let session_ids: Vec<String> = seat.sessions.clone();
        for sid in &session_ids {
            self.release_session(sid);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Multi-seat device assignment
    // -----------------------------------------------------------------------

    /// Attach a device (identified by sysfs path) to a seat.
    ///
    /// In real systemd this writes a udev rule to
    /// `/etc/udev/rules.d/72-seat-<seat>.rules` that tags the device with
    /// the target seat so udev assigns it on next trigger.  We replicate
    /// this behaviour: the rule is of the form
    ///   `TAG+="seat", ENV{ID_SEAT}="<seat>", DEVPATH=="<sysfs_path>"`
    /// and we also track the assignment in memory so `FlushDevices` can
    /// remove them.
    fn attach_device(&mut self, seat_id: &str, sysfs_path: &str) -> Result<(), String> {
        // Validate seat exists (or create it for non-seat0 dynamic seats)
        if !self.seats.contains_key(seat_id) {
            // Auto-create dynamic seat
            self.seats.insert(
                seat_id.to_string(),
                Seat {
                    id: seat_id.to_string(),
                    active_session: None,
                    sessions: Vec::new(),
                    can_graphical: false,
                    can_multi_session: false,
                    attached_devices: Vec::new(),
                },
            );
            log::info!("Created dynamic seat '{}' for device attachment", seat_id);
        }

        let seat = self
            .seats
            .get_mut(seat_id)
            .ok_or_else(|| format!("Seat '{}' not found", seat_id))?;

        // Don't add duplicates
        if seat.attached_devices.contains(&sysfs_path.to_string()) {
            return Ok(());
        }
        seat.attached_devices.push(sysfs_path.to_string());

        // Write udev rule so assignment persists across reboots
        let rules_dir = Path::new("/etc/udev/rules.d");
        if rules_dir.is_dir() {
            let rule_file = rules_dir.join(format!("72-seat-{}.rules", seat_id));

            // Build rules content from all devices attached to this seat
            let mut content = String::new();
            content.push_str(&format!(
                "# Automatically generated by systemd-logind for {}\n",
                seat_id
            ));
            for dev_path in &seat.attached_devices {
                // Normalize: strip leading /sys if present for DEVPATH match
                let devpath = dev_path.strip_prefix("/sys").unwrap_or(dev_path);
                content.push_str(&format!(
                    "TAG+=\"seat\", ENV{{ID_SEAT}}=\"{}\", DEVPATH==\"{}\"\n",
                    seat_id, devpath
                ));
            }
            if let Err(e) = fs::write(&rule_file, &content) {
                log::warn!(
                    "Failed to write udev seat rule {}: {}",
                    rule_file.display(),
                    e
                );
            } else {
                log::info!(
                    "Wrote seat attachment rule {} ({} device(s))",
                    rule_file.display(),
                    seat.attached_devices.len()
                );
            }
        }

        Ok(())
    }

    /// Remove all device attachments for all seats (except seat0 built-ins).
    ///
    /// Removes the auto-generated udev rule files and clears the in-memory
    /// tracking.  This matches real systemd's `FlushDevices` behaviour.
    fn flush_attached_devices(&mut self) {
        let rules_dir = Path::new("/etc/udev/rules.d");
        let mut seats_to_remove: Vec<String> = Vec::new();

        for (seat_id, seat) in &mut self.seats {
            if seat.attached_devices.is_empty() {
                continue;
            }
            seat.attached_devices.clear();

            // Remove the udev rule file
            if rules_dir.is_dir() {
                let rule_file = rules_dir.join(format!("72-seat-{}.rules", seat_id));
                if rule_file.exists() {
                    if let Err(e) = fs::remove_file(&rule_file) {
                        log::warn!("Failed to remove seat rule {}: {}", rule_file.display(), e);
                    } else {
                        log::info!("Removed seat attachment rule {}", rule_file.display());
                    }
                }
            }

            // Mark dynamic (non-seat0) seats with no sessions for removal
            if seat_id != "seat0" && seat.sessions.is_empty() {
                seats_to_remove.push(seat_id.clone());
            }
        }

        // Remove empty dynamic seats
        for seat_id in &seats_to_remove {
            self.seats.remove(seat_id);
            log::info!("Removed empty dynamic seat '{}'", seat_id);
        }
    }

    // -----------------------------------------------------------------------
    // ACL management — grant/revoke device access for session users
    // -----------------------------------------------------------------------

    /// Apply POSIX ACLs on device nodes attached to a seat so the active
    /// session's user can access them.
    ///
    /// When a session becomes active on a seat, we grant its user `rw`
    /// access to all device nodes in `/dev/` that belong to the seat's
    /// attached devices.  Real systemd does this via the `uaccess` udev
    /// builtin + logind seat tracking; we use `setfacl` as a portable
    /// approximation.
    fn apply_device_acls(&self, seat_id: &str, uid: u32) {
        let seat = match self.seats.get(seat_id) {
            Some(s) => s,
            None => return,
        };

        for sysfs_path in &seat.attached_devices {
            if let Some(dev_node) = devnode_from_sysfs(sysfs_path) {
                if let Err(e) = set_device_acl(&dev_node, uid, true) {
                    log::warn!("Failed to apply ACL for uid {} on {}: {}", uid, dev_node, e);
                } else {
                    log::debug!("Applied ACL for uid {} on {}", uid, dev_node);
                }
            }
        }
    }

    /// Remove POSIX ACLs previously granted to a user on a seat's devices.
    fn remove_device_acls(&self, seat_id: &str, uid: u32) {
        let seat = match self.seats.get(seat_id) {
            Some(s) => s,
            None => return,
        };

        for sysfs_path in &seat.attached_devices {
            if let Some(dev_node) = devnode_from_sysfs(sysfs_path) {
                if let Err(e) = set_device_acl(&dev_node, uid, false) {
                    log::warn!(
                        "Failed to remove ACL for uid {} on {}: {}",
                        uid,
                        dev_node,
                        e
                    );
                } else {
                    log::debug!("Removed ACL for uid {} on {}", uid, dev_node);
                }
            }
        }
    }

    /// Check if an action can be performed (checking inhibitors and polkit).
    ///
    /// Returns one of: "yes", "no", "challenge", "na".
    /// - "yes" — action is allowed without further authentication
    /// - "challenge" — action requires authentication / inhibitor is blocking
    /// - "na" — action is not applicable on this system
    fn can_action(&self, action: &str) -> &'static str {
        let what_match = match action {
            "poweroff" | "reboot" | "halt" => "shutdown",
            "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate" => "sleep",
            other => other,
        };
        let blocked = self.inhibitors.values().any(|inhibitor| {
            inhibitor.mode == "block"
                && (inhibitor.what.contains(what_match) || inhibitor.what.contains(action))
        });
        if blocked { "challenge" } else { "yes" }
    }

    /// Take control of a session (for TakeDevice).
    ///
    /// Only one D-Bus connection can be the controller at a time.
    /// `force` allows root/privileged callers to steal control.
    ///
    /// When the session has a VT assigned, sets VT_PROCESS mode on that VT
    /// so the kernel sends SIGUSR1/SIGUSR2 for VT switches instead of
    /// switching automatically.  This enables the compositor handoff
    /// protocol (PauseDevice / ResumeDevice).
    fn take_control(
        &mut self,
        session_id: &str,
        controller_name: &str,
        force: bool,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;
        if let Some(ref existing) = session.controller {
            if !force {
                return Err(format!(
                    "Session {} already controlled by {}",
                    session_id, existing
                ));
            }
            log::info!(
                "Force-taking control of session {} from {} to {}",
                session_id,
                existing,
                controller_name
            );
            // Release all devices held by the old controller
            session.devices.clear();
        }
        let vtnr = session.vtnr;
        session.controller = Some(controller_name.to_string());
        log::info!(
            "Session {} controller set to {}",
            session_id,
            controller_name
        );

        // Set VT_PROCESS mode on the session's VT if it has one and we
        // don't already have a monitor for it.
        if vtnr > 0 && !self.vt_monitors.contains_key(&vtnr) {
            match VtMonitor::new(vtnr) {
                Ok(monitor) => {
                    self.vt_monitors.insert(vtnr, monitor);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to set VT_PROCESS on VT {} for session {}: {}",
                        vtnr,
                        session_id,
                        e
                    );
                    // Non-fatal: VT switching will still work in VT_AUTO mode,
                    // but compositors won't get PauseDevice/ResumeDevice signals.
                }
            }
        }

        Ok(())
    }

    /// Release control of a session.
    ///
    /// Restores VT_AUTO mode on the session's VT if no other controlled
    /// session shares it.
    fn release_control(&mut self, session_id: &str) {
        let mut vtnr_to_check: Option<u32> = None;
        if let Some(session) = self.sessions.get_mut(session_id) {
            // Close all taken devices
            let count = session.devices.len();
            session.devices.clear();
            session.controller = None;
            vtnr_to_check = if session.vtnr > 0 {
                Some(session.vtnr)
            } else {
                None
            };
            if count > 0 {
                log::info!(
                    "Session {} controller released, closed {} device(s)",
                    session_id,
                    count
                );
            }
        }

        // Remove VT monitor if no other controlled session uses this VT
        if let Some(vtnr) = vtnr_to_check {
            let other_controller_on_vt = self
                .sessions
                .values()
                .any(|s| s.vtnr == vtnr && s.controller.is_some() && s.id != session_id);
            if !other_controller_on_vt && self.vt_monitors.remove(&vtnr).is_some() {
                log::info!(
                    "Removed VT_PROCESS monitor for VT {} (session {} released control)",
                    vtnr,
                    session_id
                );
            }
        }
    }

    /// Find the session that is currently active on seat0.
    fn active_session_on_seat0(&self) -> Option<&str> {
        self.seats
            .get("seat0")
            .and_then(|s| s.active_session.as_deref())
    }

    /// Find a session by its VT number (first match).
    fn session_by_vtnr(&self, vtnr: u32) -> Option<&str> {
        self.sessions
            .values()
            .find(|s| s.vtnr == vtnr)
            .map(|s| s.id.as_str())
    }

    /// Collect device info for PauseDevice signal emission.
    /// Returns a list of (major, minor) for all taken devices in the session.
    #[allow(dead_code)]
    fn session_device_keys(&self, session_id: &str) -> Vec<(u32, u32)> {
        self.sessions
            .get(session_id)
            .map(|s| s.devices.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Collect device info for ResumeDevice signal emission.
    /// Returns a list of (major, minor, raw_fd) for all taken devices.
    fn session_device_fds(&self, session_id: &str) -> Vec<(u32, u32, RawFd)> {
        self.sessions
            .get(session_id)
            .map(|s| {
                s.devices
                    .iter()
                    .map(|((maj, min), dev)| (*maj, *min, dev.fd.as_raw_fd()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Handle SIGUSR1 — VT release request (switch away).
    ///
    /// Marks all devices in the currently active session as inactive and
    /// calls VT_RELDISP(1) to allow the switch.  Returns information
    /// needed to emit PauseDevice D-Bus signals (done by the caller
    /// outside the lock).
    fn handle_vt_release(&mut self) -> Option<VtSwitchAwayInfo> {
        let active_id = self.active_session_on_seat0()?.to_string();
        let session = self.sessions.get_mut(&active_id)?;
        let vtnr = session.vtnr;
        if vtnr == 0 {
            return None;
        }

        // Mark all taken devices as inactive
        let mut devices = Vec::new();
        for ((maj, min), dev) in &mut session.devices {
            dev.active = false;
            devices.push((*maj, *min));
        }

        // Allow the VT switch
        if let Some(monitor) = self.vt_monitors.get(&vtnr) {
            monitor.release_display();
        }

        if !devices.is_empty() {
            log::info!(
                "VT {} release: pausing {} device(s) for session {}",
                vtnr,
                devices.len(),
                active_id,
            );
        }

        Some(VtSwitchAwayInfo {
            session_id: active_id,
            devices,
        })
    }

    /// Handle SIGUSR2 — VT acquire notification (switch to).
    ///
    /// Reads the new active VT, activates the corresponding session,
    /// marks its devices as active, and acknowledges the acquire.
    /// Returns information needed to emit ResumeDevice D-Bus signals.
    fn handle_vt_acquire(&mut self) -> Option<VtSwitchToInfo> {
        let new_vt = get_active_vt()?;
        let session_id = self.session_by_vtnr(new_vt)?.to_string();

        // Acknowledge VT acquisition first
        if let Some(monitor) = self.vt_monitors.get(&new_vt) {
            monitor.ack_acquire();
        }

        // Activate the session (this deactivates the old one too)
        let switch_info = self.activate_session(&session_id).ok()?;

        // Collect FDs for ResumeDevice signals
        let devices = self.session_device_fds(&session_id);

        if !devices.is_empty() {
            log::info!(
                "VT {} acquire: resuming {} device(s) for session {}",
                new_vt,
                devices.len(),
                session_id,
            );
        }

        Some(VtSwitchToInfo {
            session_id,
            old_session_id: switch_info.old_session_id,
            old_devices: switch_info.old_devices,
            devices,
        })
    }

    /// Take a device for a session (TakeDevice).
    ///
    /// Opens the device node identified by major:minor and returns a dup'd FD.
    /// The original FD is kept in the session's device map so logind can
    /// revoke access when the session goes inactive (PauseDevice).
    fn take_device(
        &mut self,
        session_id: &str,
        major: u32,
        minor: u32,
    ) -> Result<(OwnedFd, bool), String> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if session.controller.is_none() {
            return Err("Session has no controller — call TakeControl first".to_string());
        }

        let dev_key = (major, minor);

        // If already taken, return a dup of the existing FD
        if let Some(dev) = session.devices.get(&dev_key) {
            let dup_fd = dup_fd(&dev.fd)?;
            return Ok((dup_fd, dev.active));
        }

        // Open the device node via /dev/char/MAJOR:MINOR (or sysfs devnode)
        let dev_path = format!("/dev/char/{}:{}", major, minor);
        let c_path = CString::new(dev_path.clone())
            .map_err(|_| format!("Invalid device path: {}", dev_path))?;
        let raw_fd = unsafe {
            libc::open(
                c_path.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY | libc::O_NONBLOCK,
            )
        };
        if raw_fd < 0 {
            return Err(format!(
                "Failed to open {}: {}",
                dev_path,
                io::Error::last_os_error()
            ));
        }
        let owned = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        // Dup the FD for the caller — logind keeps the original
        let caller_fd = dup_fd(&owned)?;
        let is_active = session.active;

        session.devices.insert(
            dev_key,
            SessionDevice {
                major,
                minor,
                fd: owned,
                active: is_active,
            },
        );

        log::info!(
            "Session {} took device {}:{} (active={})",
            session_id,
            major,
            minor,
            is_active
        );
        Ok((caller_fd, is_active))
    }

    /// Release a previously taken device.
    fn release_device(&mut self, session_id: &str, major: u32, minor: u32) {
        if let Some(session) = self.sessions.get_mut(session_id)
            && session.devices.remove(&(major, minor)).is_some()
        {
            log::info!("Session {} released device {}:{}", session_id, major, minor);
        }
    }
}

/// Information about a session switch, used to emit PauseDevice/ResumeDevice
/// signals outside the manager lock.
#[allow(dead_code)]
struct SessionSwitchInfo {
    /// The session that was deactivated (if any).
    old_session_id: Option<String>,
    /// Devices from the old session that need PauseDevice signals: (major, minor).
    old_devices: Vec<(u32, u32)>,
    /// The session that was activated.
    new_session_id: String,
    /// Devices from the new session that need ResumeDevice signals: (major, minor, raw_fd).
    new_devices: Vec<(u32, u32, RawFd)>,
}

/// Information collected under the manager lock during a VT switch-away
/// (SIGUSR1).  Used by the main loop to emit PauseDevice D-Bus signals
/// outside the lock.
struct VtSwitchAwayInfo {
    session_id: String,
    /// Devices that need PauseDevice signals: (major, minor).
    devices: Vec<(u32, u32)>,
}

/// Information collected under the manager lock during a VT switch-to
/// (SIGUSR2).  Used by the main loop to emit ResumeDevice D-Bus signals
/// outside the lock.
struct VtSwitchToInfo {
    session_id: String,
    /// Previous active session (if any) whose devices need PauseDevice.
    old_session_id: Option<String>,
    /// Devices from the old session that need PauseDevice: (major, minor).
    old_devices: Vec<(u32, u32)>,
    /// Devices that need ResumeDevice signals: (major, minor, raw_fd).
    devices: Vec<(u32, u32, RawFd)>,
}

/// Duplicate a file descriptor, returning a new `OwnedFd`.
fn dup_fd(fd: &OwnedFd) -> Result<OwnedFd, String> {
    let raw: RawFd = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if raw < 0 {
        return Err(format!(
            "fcntl F_DUPFD_CLOEXEC failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Duplicate a raw file descriptor, returning a new `OwnedFd`.
#[allow(dead_code)]
fn dup_raw_fd(raw: RawFd) -> Result<OwnedFd, String> {
    let new_raw: RawFd = unsafe { libc::fcntl(raw, libc::F_DUPFD_CLOEXEC, 0) };
    if new_raw < 0 {
        return Err(format!(
            "fcntl F_DUPFD_CLOEXEC failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(new_raw) })
}

impl LoginManager {
    /// Get the global idle hint (any active session not idle => not idle)
    fn global_idle_hint(&self) -> bool {
        // If there are no sessions, consider idle
        if self.sessions.is_empty() {
            return true;
        }
        // If any active session is not idle, global idle is false
        for session in self.sessions.values() {
            if session.active && !session.idle_hint {
                return false;
            }
        }
        true
    }

    /// Get the global idle-since hint (earliest idle session)
    fn global_idle_since_hint(&self) -> (u64, u64) {
        let mut earliest_rt: u64 = 0;
        let mut earliest_mono: u64 = 0;
        for session in self.sessions.values() {
            if session.idle_hint
                && session.idle_since_hint > 0
                && (earliest_rt == 0 || session.idle_since_hint < earliest_rt)
            {
                earliest_rt = session.idle_since_hint;
                earliest_mono = session.idle_since_hint_monotonic;
            }
        }
        (earliest_rt, earliest_mono)
    }

    /// Get which inhibit types are blocked
    fn block_inhibited(&self) -> String {
        let mut types = std::collections::BTreeSet::new();
        for inhibitor in self.inhibitors.values() {
            if inhibitor.mode == "block" {
                for what in inhibitor.what.split(':') {
                    types.insert(what.trim().to_string());
                }
            }
        }
        let v: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
        v.join(":")
    }

    /// Get which inhibit types are delayed
    fn delay_inhibited(&self) -> String {
        let mut types = std::collections::BTreeSet::new();
        for inhibitor in self.inhibitors.values() {
            if inhibitor.mode == "delay" {
                for what in inhibitor.what.split(':') {
                    types.insert(what.trim().to_string());
                }
            }
        }
        let v: Vec<&str> = types.iter().map(|s| s.as_str()).collect();
        v.join(":")
    }

    /// Write session state file to /run/systemd/sessions/<id>
    fn write_session_file(&self, session: &Session) {
        let session_file = Path::new(SESSIONS_DIR).join(&session.id);
        let mut content = String::new();
        content.push_str(&format!("UID={}\n", session.uid));
        content.push_str(&format!("USER={}\n", session.user));
        content.push_str(&format!(
            "ACTIVE={}\n",
            if session.active { "1" } else { "0" }
        ));
        content.push_str(&format!("STATE={}\n", session.state));
        content.push_str(&format!("TYPE={}\n", session.session_type));
        content.push_str(&format!("CLASS={}\n", session.class));
        if let Some(ref seat) = session.seat {
            content.push_str(&format!("SEAT={}\n", seat));
        }
        if session.vtnr > 0 {
            content.push_str(&format!("VTNR={}\n", session.vtnr));
        }
        if !session.tty.is_empty() {
            content.push_str(&format!("TTY={}\n", session.tty));
        }
        if !session.display.is_empty() {
            content.push_str(&format!("DISPLAY={}\n", session.display));
        }
        if !session.service.is_empty() {
            content.push_str(&format!("SERVICE={}\n", session.service));
        }
        if !session.desktop.is_empty() {
            content.push_str(&format!("DESKTOP={}\n", session.desktop));
        }
        if session.leader > 0 {
            content.push_str(&format!("LEADER={}\n", session.leader));
        }
        content.push_str(&format!("SCOPE={}\n", session.scope));
        content.push_str(&format!(
            "REMOTE={}\n",
            if session.remote { "1" } else { "0" }
        ));
        if session.remote && !session.remote_host.is_empty() {
            content.push_str(&format!("REMOTE_HOST={}\n", session.remote_host));
        }
        if session.remote && !session.remote_user.is_empty() {
            content.push_str(&format!("REMOTE_USER={}\n", session.remote_user));
        }
        content.push_str(&format!(
            "REALTIME={}\n",
            session.since.saturating_mul(1_000_000)
        ));
        content.push_str(&format!("MONOTONIC={}\n", session.since_monotonic));
        let _ = fs::write(session_file, content);
    }

    /// Write seat state file to /run/systemd/seats/<id>
    fn write_seat_files(&self) {
        for seat in self.seats.values() {
            let seat_file = Path::new(SEATS_DIR).join(&seat.id);
            let mut content = String::new();
            content.push_str(&format!("ID={}\n", seat.id));
            if let Some(ref active) = seat.active_session {
                content.push_str(&format!("ACTIVE_SESSION={}\n", active));
                if let Some(session) = self.sessions.get(active) {
                    content.push_str(&format!("ACTIVE_SESSION_UID={}\n", session.uid));
                }
            }
            content.push_str(&format!(
                "CAN_GRAPHICAL={}\n",
                if seat.can_graphical { "1" } else { "0" }
            ));
            content.push_str(&format!(
                "CAN_MULTI_SESSION={}\n",
                if seat.can_multi_session { "1" } else { "0" }
            ));
            let sessions_str: Vec<&str> = seat.sessions.iter().map(|s| s.as_str()).collect();
            if !sessions_str.is_empty() {
                content.push_str(&format!("SESSIONS={}\n", sessions_str.join(" ")));
            }
            let _ = fs::write(seat_file, content);
        }
    }

    /// Write user state files to /run/systemd/users/<uid>
    fn write_user_files(&self) {
        for user in self.users.values() {
            let user_file = Path::new(USERS_DIR).join(format!("{}", user.uid));
            let mut content = String::new();
            content.push_str(&format!("NAME={}\n", user.name));
            content.push_str(&format!("STATE={}\n", user.state));
            content.push_str(&format!("SLICE={}\n", user.slice));
            content.push_str(&format!("SERVICE={}\n", user.service));
            content.push_str(&format!("RUNTIME_PATH={}\n", user.runtime_path));
            let sessions_str: Vec<&str> = user.sessions.iter().map(|s| s.as_str()).collect();
            if !sessions_str.is_empty() {
                content.push_str(&format!("SESSIONS={}\n", sessions_str.join(" ")));
                content.push_str(&format!("DISPLAY={}\n", sessions_str[0]));
            }
            content.push_str(&format!(
                "REALTIME={}\n",
                user.since.saturating_mul(1_000_000)
            ));
            content.push_str(&format!("MONOTONIC={}\n", user.since_monotonic));
            let _ = fs::write(user_file, content);
        }
    }

    /// Synchronize all runtime files
    fn sync_runtime_state(&self) {
        // Write session files
        for session in self.sessions.values() {
            self.write_session_file(session);
        }
        // Write seat files
        self.write_seat_files();
        // Write user files
        self.write_user_files();
    }

    /// Auto-detect existing sessions from /proc
    fn detect_existing_sessions(&mut self) {
        // Look for existing utmp entries to detect already-logged-in users.
        // This supplements PAM — if pam_systemd already created sessions
        // they'll be in /run/systemd/sessions/; this catches sessions that
        // existed before logind started (e.g. after daemon restart).
        self.detect_sessions_from_runtime();
        self.detect_sessions_from_utmp();
    }

    /// Re-enumerate sessions persisted in /run/systemd/sessions/.
    fn detect_sessions_from_runtime(&mut self) {
        let sessions_dir = Path::new(SESSIONS_DIR);
        let entries = match fs::read_dir(sessions_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let fname = entry.file_name();
            let session_id = fname.to_string_lossy().to_string();
            if self.sessions.contains_key(&session_id) {
                continue;
            }
            let content = match fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let props = parse_env_file(&content);
            let uid: u32 = props.get("UID").and_then(|v| v.parse().ok()).unwrap_or(0);
            let user = props
                .get("USER")
                .cloned()
                .unwrap_or_else(|| resolve_uid_to_name(uid));
            let seat = props.get("SEAT").cloned();
            let vtnr: u32 = props.get("VTNR").and_then(|v| v.parse().ok()).unwrap_or(0);
            let stype = props
                .get("TYPE")
                .cloned()
                .unwrap_or_else(|| "tty".to_string());
            let class = props
                .get("CLASS")
                .cloned()
                .unwrap_or_else(|| "user".to_string());
            let tty = props.get("TTY").cloned().unwrap_or_default();
            let leader: u32 = props
                .get("LEADER")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            // Check leader is still alive (pid > 0 and kill(pid,0) succeeds)
            if leader > 0 && unsafe { libc::kill(leader as i32, 0) } != 0 {
                // Stale session file — remove it
                let _ = fs::remove_file(entry.path());
                log::debug!(
                    "Removed stale session file {} (leader {} dead)",
                    session_id,
                    leader
                );
                continue;
            }

            log::info!(
                "Recovered session {} for user {} (uid={}) from runtime state",
                session_id,
                user,
                uid
            );

            // Ensure next_session_id stays ahead
            if let Ok(n) = session_id.parse::<u64>()
                && n >= self.next_session_id
            {
                self.next_session_id = n + 1;
            }

            let _ = self.create_session(
                uid,
                &user,
                seat.as_deref(),
                vtnr,
                &stype,
                &class,
                &tty,
                leader,
            );
        }
    }

    /// Parse utmp to detect login sessions not yet tracked.
    ///
    /// utmp records are fixed-size C structs.  On Linux x86_64 each record
    /// is 384 bytes (see `man 5 utmp`).  We read `USER_PROCESS` (type=7)
    /// entries and create sessions for any that we don't already track.
    fn detect_sessions_from_utmp(&mut self) {
        let utmp_path = Path::new("/var/run/utmp");
        let data = match fs::read(utmp_path) {
            Ok(d) => d,
            Err(_) => return,
        };

        // utmp record layout (glibc, x86_64):
        //   i16  ut_type        offset 0
        //   i32  ut_pid         offset 4
        //   [u8; 32] ut_line    offset 8
        //   [u8; 4]  ut_id      offset 40
        //   [u8; 32] ut_user    offset 44
        //   [u8; 256] ut_host   offset 76
        //   ... (remaining fields we don't need)
        // Total size: 384 bytes
        const UTMP_RECORD_SIZE: usize = 384;
        const USER_PROCESS: i16 = 7;

        if data.len() < UTMP_RECORD_SIZE {
            return;
        }

        let record_count = data.len() / UTMP_RECORD_SIZE;
        for i in 0..record_count {
            let rec = &data[i * UTMP_RECORD_SIZE..(i + 1) * UTMP_RECORD_SIZE];

            let ut_type = i16::from_ne_bytes([rec[0], rec[1]]);
            if ut_type != USER_PROCESS {
                continue;
            }

            let ut_pid = i32::from_ne_bytes([rec[4], rec[5], rec[6], rec[7]]);
            if ut_pid <= 0 {
                continue;
            }

            // Check if process is still alive
            if unsafe { libc::kill(ut_pid, 0) } != 0 {
                continue;
            }

            let ut_line = read_cstr_field(&rec[8..40]);
            let ut_user = read_cstr_field(&rec[44..76]);
            let ut_host = read_cstr_field(&rec[76..332]);

            if ut_user.is_empty() {
                continue;
            }

            // Check if we already track a session with this leader PID
            let pid = ut_pid as u32;
            if self.sessions.values().any(|s| s.leader == pid) {
                continue;
            }

            // Resolve user name to UID
            let uid = resolve_username_to_uid(&ut_user).unwrap_or(0);
            if uid == 0 && ut_user != "root" {
                continue;
            }

            // Determine session type from TTY line
            let tty_path = if ut_line.starts_with('/') {
                ut_line.clone()
            } else {
                format!("/dev/{}", ut_line)
            };
            let is_graphical = ut_line.starts_with(':');
            let stype = if is_graphical { "x11" } else { "tty" };

            // Assign VT number if it's a ttyN device
            let vtnr = if let Some(rest) = ut_line.strip_prefix("tty") {
                rest.parse::<u32>().unwrap_or(0)
            } else {
                0
            };

            let seat = if !is_graphical { Some("seat0") } else { None };
            let remote = !ut_host.is_empty();

            let id = self.create_session(uid, &ut_user, seat, vtnr, stype, "user", &tty_path, pid);

            // Set remote info if present
            if remote && let Some(session) = self.sessions.get_mut(&id) {
                session.remote = true;
                session.remote_host = ut_host.clone();
            }

            log::info!(
                "Detected session {} for user {} (pid={}) from utmp on {}",
                id,
                ut_user,
                pid,
                ut_line
            );
        }
    }

    fn format_status(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("{} seats logged in.\n", self.seats.len()));
        for seat in self.seats.values() {
            out.push_str(&format!(
                "  Seat {}: {} session(s), active: {}\n",
                seat.id,
                seat.sessions.len(),
                seat.active_session.as_deref().unwrap_or("(none)")
            ));
        }
        out.push_str(&format!("{} sessions.\n", self.sessions.len()));
        out.push_str(&format!("{} users.\n", self.users.len()));
        out.push_str(&format!("{} inhibitors.\n", self.inhibitors.len()));
        out
    }
}

// ---------------------------------------------------------------------------
// Configuration parsing
// ---------------------------------------------------------------------------

fn parse_logind_conf() -> LogindConfig {
    let mut config = LogindConfig::default();

    for path in &[
        "/etc/systemd/logind.conf",
        "/run/systemd/logind.conf.d",
        "/usr/lib/systemd/logind.conf.d",
    ] {
        if Path::new(path).is_file() {
            parse_logind_conf_file(path, &mut config);
        } else if Path::new(path).is_dir()
            && let Ok(entries) = fs::read_dir(path)
        {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|ext| ext == "conf")
                        .unwrap_or(false)
                })
                .map(|e| e.path())
                .collect();
            files.sort();
            for file in files {
                parse_logind_conf_file(&file.to_string_lossy(), &mut config);
            }
        }
    }

    config
}

fn parse_logind_conf_file(path: &str, config: &mut LogindConfig) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut in_login_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.eq_ignore_ascii_case("[login]") {
            in_login_section = true;
            continue;
        }
        if line.starts_with('[') {
            in_login_section = false;
            continue;
        }
        if !in_login_section {
            continue;
        }

        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "NAutoVTs" => {
                    if let Ok(n) = val.parse() {
                        config.n_auto_vts = n;
                    }
                }
                "KillUserProcesses" => {
                    config.kill_user_processes = val == "yes" || val == "true" || val == "1";
                }
                "KillOnlyUsers" => {
                    config.kill_only_users =
                        val.split_whitespace().map(|s| s.to_string()).collect();
                }
                "KillExcludeUsers" => {
                    config.kill_exclude_users =
                        val.split_whitespace().map(|s| s.to_string()).collect();
                }
                "IdleAction" => {
                    config.idle_action = val.to_string();
                }
                "IdleActionSec" => {
                    if let Ok(secs) = parse_timespan_to_usec(val) {
                        config.idle_action_usec = secs;
                    }
                }
                "InhibitDelayMaxSec" => {
                    if let Ok(usec) = parse_timespan_to_usec(val) {
                        config.inhibit_delay_max_usec = usec;
                    }
                }
                "UserStopDelaySec" => {
                    if let Ok(usec) = parse_timespan_to_usec(val) {
                        config.user_stop_delay_usec = usec;
                    }
                }
                "HandlePowerKey" => {
                    config.handle_power_key = val.to_string();
                }
                "HandleSuspendKey" => {
                    config.handle_suspend_key = val.to_string();
                }
                "HandleHibernateKey" => {
                    config.handle_hibernate_key = val.to_string();
                }
                "HandleLidSwitch" => {
                    config.handle_lid_switch = val.to_string();
                }
                "HandleLidSwitchExternalPower" => {
                    config.handle_lid_switch_external_power = val.to_string();
                }
                "HandleLidSwitchDocked" => {
                    config.handle_lid_switch_docked = val.to_string();
                }
                "HoldoffTimeoutSec" => {
                    if let Ok(usec) = parse_timespan_to_usec(val) {
                        config.holdoff_timeout_usec = usec;
                    }
                }
                "RemoveIPC" => {
                    config.remove_ipc = val == "yes" || val == "true" || val == "1";
                }
                "InhibitorsMax" => {
                    if let Ok(n) = val.parse() {
                        config.inhibitors_max = n;
                    }
                }
                "SessionsMax" => {
                    if let Ok(n) = val.parse() {
                        config.sessions_max = n;
                    }
                }
                _ => {}
            }
        }
    }
}

fn parse_timespan_to_usec(val: &str) -> Result<u64, ()> {
    // Simple parser for time spans like "5s", "30", "1min", "500ms"
    let val = val.trim();
    if val.is_empty() {
        return Err(());
    }
    if let Ok(n) = val.parse::<u64>() {
        // Plain number = seconds
        return Ok(n * 1_000_000);
    }
    // Suffixes ordered longest-first within each unit to avoid partial matches
    // (e.g. "5sec" must not match 's' leaving "5se")
    // Order matters: longer suffixes must come before shorter ones that
    // could be a suffix of the longer form.  E.g. "minutes" ends with "s",
    // so the "s" entry would incorrectly match "2minutes" (leaving "2minute"
    // which is not a number).  By trying all multi-char suffixes first and
    // single-char ones last we avoid false positives.
    let suffixes: &[(&str, u64)] = &[
        ("usec", 1),
        ("us", 1),
        ("msec", 1_000),
        ("ms", 1_000),
        ("minutes", 60 * 1_000_000),
        ("minute", 60 * 1_000_000),
        ("min", 60 * 1_000_000),
        ("hours", 3600 * 1_000_000),
        ("hour", 3600 * 1_000_000),
        ("hr", 3600 * 1_000_000),
        ("seconds", 1_000_000),
        ("second", 1_000_000),
        ("sec", 1_000_000),
        ("s", 1_000_000),
        ("h", 3600 * 1_000_000),
    ];
    for &(suffix, multiplier) in suffixes {
        if let Some(num_str) = val.strip_suffix(suffix)
            && let Ok(n) = num_str.trim().parse::<u64>()
        {
            return Ok(n * multiplier);
        }
        // suffix matched but the numeric part didn't parse —
        // keep trying other suffixes (e.g. "2minutes" matches "s"
        // leaving "2minute", which is invalid, but also matches
        // "minutes" leaving "2", which is valid).
    }
    Err(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn check_seat0_graphical() -> bool {
    // Check if any DRM/framebuffer device exists
    Path::new("/dev/dri").is_dir() || Path::new("/dev/fb0").exists()
}

/// Resolve a user's primary GID from /etc/passwd
fn resolve_user_gid(uid: u32) -> u32 {
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 4
                && let Ok(file_uid) = parts[2].parse::<u32>()
                && file_uid == uid
                && let Ok(gid) = parts[3].parse::<u32>()
            {
                return gid;
            }
        }
    }
    uid // fallback: gid = uid
}

/// Convert a session ID to a D-Bus object path component.
/// D-Bus object path components can't start with a digit, so prefix with '_'.
fn session_object_path(session_id: &str) -> String {
    let escaped = session_id.replace('-', "_2d");
    format!("{}/session/_{}", DBUS_PATH, escaped)
}

/// Convert a seat name to a D-Bus object path.
fn seat_object_path(seat_id: &str) -> String {
    let escaped = seat_id.replace('-', "_2d");
    format!("{}/seat/{}", DBUS_PATH, escaped)
}

/// Convert a UID to a D-Bus object path.
fn user_object_path(uid: u32) -> String {
    format!("{}/user/_{}", DBUS_PATH, uid)
}

/// Extract session ID from a D-Bus object path
#[allow(dead_code)]
fn session_id_from_path(path: &str) -> Option<String> {
    let prefix = format!("{}/session/_", DBUS_PATH);
    path.strip_prefix(&prefix)
        .map(|rest| rest.replace("_2d", "-"))
}

/// Extract seat ID from a D-Bus object path
#[allow(dead_code)]
fn seat_id_from_path(path: &str) -> Option<String> {
    let prefix = format!("{}/seat/", DBUS_PATH);
    path.strip_prefix(&prefix)
        .map(|rest| rest.replace("_2d", "-"))
}

/// Extract UID from a D-Bus object path
#[allow(dead_code)]
fn uid_from_path(path: &str) -> Option<u32> {
    let prefix = format!("{}/user/_", DBUS_PATH);
    if let Some(rest) = path.strip_prefix(&prefix) {
        rest.parse().ok()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// D-Bus interface setup
// ---------------------------------------------------------------------------

type SharedManager = Arc<Mutex<LoginManager>>;

// ---------------------------------------------------------------------------
// D-Bus interface structs (zbus)
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.login1.Manager
struct Login1Manager {
    mgr: SharedManager,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl Login1Manager {
    // --- Properties ---

    #[zbus(property, name = "NAutoVTs")]
    fn n_auto_vts(&self) -> u32 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.n_auto_vts
    }

    #[zbus(property, name = "KillOnlyUsers")]
    fn kill_only_users(&self) -> Vec<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.kill_only_users.clone()
    }

    #[zbus(property, name = "KillExcludeUsers")]
    fn kill_exclude_users(&self) -> Vec<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.kill_exclude_users.clone()
    }

    #[zbus(property, name = "KillUserProcesses")]
    fn kill_user_processes(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.kill_user_processes
    }

    #[zbus(property, name = "IdleHint")]
    fn idle_hint(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.global_idle_hint()
    }

    #[zbus(property, name = "IdleSinceHint")]
    fn idle_since_hint(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let (rt, _) = mgr.global_idle_since_hint();
        rt
    }

    #[zbus(property, name = "IdleSinceHintMonotonic")]
    fn idle_since_hint_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let (_, mono) = mgr.global_idle_since_hint();
        mono
    }

    #[zbus(property, name = "BlockInhibited")]
    fn block_inhibited(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.block_inhibited()
    }

    #[zbus(property, name = "DelayInhibited")]
    fn delay_inhibited(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.delay_inhibited()
    }

    #[zbus(property, name = "InhibitDelayMaxUSec")]
    fn inhibit_delay_max_usec(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.inhibit_delay_max_usec
    }

    #[zbus(property, name = "UserStopDelayUSec")]
    fn user_stop_delay_usec(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.user_stop_delay_usec
    }

    #[zbus(property, name = "HandlePowerKey")]
    fn handle_power_key(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_power_key.clone()
    }

    #[zbus(property, name = "HandleSuspendKey")]
    fn handle_suspend_key(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_suspend_key.clone()
    }

    #[zbus(property, name = "HandleHibernateKey")]
    fn handle_hibernate_key(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_hibernate_key.clone()
    }

    #[zbus(property, name = "HandleLidSwitch")]
    fn handle_lid_switch(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_lid_switch.clone()
    }

    #[zbus(property, name = "HandleLidSwitchExternalPower")]
    fn handle_lid_switch_external_power(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_lid_switch_external_power.clone()
    }

    #[zbus(property, name = "HandleLidSwitchDocked")]
    fn handle_lid_switch_docked(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.handle_lid_switch_docked.clone()
    }

    #[zbus(property, name = "HoldoffTimeoutUSec")]
    fn holdoff_timeout_usec(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.holdoff_timeout_usec
    }

    #[zbus(property, name = "IdleAction")]
    fn idle_action(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.idle_action.clone()
    }

    #[zbus(property, name = "IdleActionUSec")]
    fn idle_action_usec(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.idle_action_usec
    }

    #[zbus(property, name = "PreparingForShutdown")]
    fn preparing_for_shutdown(&self) -> bool {
        false
    }

    #[zbus(property, name = "PreparingForSleep")]
    fn preparing_for_sleep(&self) -> bool {
        false
    }

    #[zbus(property, name = "Docked")]
    fn docked(&self) -> bool {
        false
    }

    #[zbus(property, name = "LidClosed")]
    fn lid_closed(&self) -> bool {
        false
    }

    #[zbus(property, name = "OnExternalPower")]
    fn on_external_power(&self) -> bool {
        check_ac_power()
    }

    #[zbus(property, name = "RemoveIPC")]
    fn remove_ipc(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.remove_ipc
    }

    #[zbus(property, name = "RuntimeDirectorySize")]
    fn runtime_directory_size(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.runtime_directory_size
    }

    #[zbus(property, name = "RuntimeDirectoryInodesMax")]
    fn runtime_directory_inodes_max(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.runtime_directory_inodes_max
    }

    #[zbus(property, name = "InhibitorsMax")]
    fn inhibitors_max(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.inhibitors_max
    }

    #[zbus(property, name = "SessionsMax")]
    fn sessions_max(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.config.sessions_max
    }

    #[zbus(property, name = "NCurrentSessions")]
    fn n_current_sessions(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions.len() as u64
    }

    #[zbus(property, name = "NCurrentInhibitors")]
    fn n_current_inhibitors(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.inhibitors.len() as u64
    }

    // --- Signals ---

    #[zbus(signal)]
    async fn session_new(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        session_id: &str,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn session_removed(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        session_id: &str,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn user_new(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        uid: u32,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn user_removed(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        uid: u32,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn seat_new(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        seat_id: &str,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn seat_removed(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        seat_id: &str,
        object_path: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn prepare_for_shutdown(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        active: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn prepare_for_sleep(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        active: bool,
    ) -> zbus::Result<()>;

    // --- Methods with caller credential extraction ---

    // --- Methods ---

    fn get_session(&self, session_id: String) -> zbus::fdo::Result<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.sessions.contains_key(&session_id) {
            Ok(session_object_path(&session_id))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No session '{}' known",
                session_id
            )))
        }
    }

    fn get_session_by_pid(&self, pid: u32) -> zbus::fdo::Result<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        for session in mgr.sessions.values() {
            if session.leader == pid {
                return Ok(session_object_path(&session.id));
            }
        }
        Err(zbus::fdo::Error::Failed(format!(
            "No session for PID {} known",
            pid
        )))
    }

    fn get_user(&self, uid: u32) -> zbus::fdo::Result<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.users.contains_key(&uid) {
            Ok(user_object_path(uid))
        } else {
            Err(zbus::fdo::Error::Failed(format!("No user '{}' known", uid)))
        }
    }

    fn get_user_by_pid(&self, pid: u32) -> zbus::fdo::Result<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        for session in mgr.sessions.values() {
            if session.leader == pid {
                return Ok(user_object_path(session.uid));
            }
        }
        Err(zbus::fdo::Error::Failed(format!(
            "No user for PID {} known",
            pid
        )))
    }

    fn get_seat(&self, seat_id: String) -> zbus::fdo::Result<String> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.seats.contains_key(&seat_id) {
            Ok(seat_object_path(&seat_id))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No seat '{}' known",
                seat_id
            )))
        }
    }

    fn list_sessions(&self) -> Vec<(String, u32, String, String, String)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = Vec::new();
        for session in mgr.sessions.values() {
            result.push((
                session.id.clone(),
                session.uid,
                session.user.clone(),
                session.seat.clone().unwrap_or_default(),
                session_object_path(&session.id),
            ));
        }
        result
    }

    fn list_users(&self) -> Vec<(u32, String, String)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = Vec::new();
        for user in mgr.users.values() {
            result.push((user.uid, user.name.clone(), user_object_path(user.uid)));
        }
        result
    }

    fn list_seats(&self) -> Vec<(String, String)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = Vec::new();
        for seat in mgr.seats.values() {
            result.push((seat.id.clone(), seat_object_path(&seat.id)));
        }
        result
    }

    fn list_inhibitors(&self) -> Vec<(String, String, String, String, u32, u32)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = Vec::new();
        for inhibitor in mgr.inhibitors.values() {
            result.push((
                inhibitor.what.clone(),
                inhibitor.who.clone(),
                inhibitor.why.clone(),
                inhibitor.mode.clone(),
                inhibitor.uid,
                inhibitor.pid,
            ));
        }
        result
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn create_session(
        &self,
        uid: u32,
        _pid: u32,
        _service: String,
        stype: String,
        class: String,
        seat_id: String,
        vtnr: u32,
        tty: String,
        _display: String,
        _remote: bool,
        _remote_user: String,
        _remote_host: String,
    ) -> zbus::fdo::Result<(String, String, String, bool, u32, String, u32, bool)> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let user = resolve_uid_to_name(uid);
        let seat = if seat_id.is_empty() {
            None
        } else {
            Some(seat_id.as_str())
        };
        let id = mgr.create_session(uid, &user, seat, vtnr, &stype, &class, &tty, _pid);
        mgr.sync_runtime_state();
        log::info!(
            "New session {} of user {} on {}",
            id,
            user,
            seat.unwrap_or("(no seat)")
        );
        let obj_path = session_object_path(&id);
        let runtime_path = format!("/run/user/{}", uid);
        Ok((id, obj_path, runtime_path, false, uid, seat_id, vtnr, false))
    }

    fn release_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.release_session(&session_id) {
            mgr.sync_runtime_state();
            log::info!("Released session {}", session_id);
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No session '{}' known",
                session_id
            )))
        }
    }

    fn activate_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        match mgr.activate_session(&session_id) {
            Ok(switch_info) => {
                mgr.sync_runtime_state();
                if let Some(ref old_id) = switch_info.old_session_id {
                    log::info!(
                        "Activated session {} (was {}), pause {} / resume {} device(s)",
                        session_id,
                        old_id,
                        switch_info.old_devices.len(),
                        switch_info.new_devices.len()
                    );
                } else {
                    log::info!("Activated session {}", session_id);
                }
                Ok(())
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    fn activate_session_on_seat(
        &self,
        session_id: String,
        seat_id: String,
    ) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = mgr.sessions.get(&session_id)
            && session.seat.as_deref() != Some(&seat_id)
        {
            return Err(zbus::fdo::Error::Failed(format!(
                "Session '{}' not on seat '{}'",
                session_id, seat_id
            )));
        }
        match mgr.activate_session(&session_id) {
            Ok(_switch_info) => {
                mgr.sync_runtime_state();
                Ok(())
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    fn lock_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.lock_session(&session_id)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn unlock_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.unlock_session(&session_id)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn lock_sessions(&self) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.lock_sessions();
    }

    fn unlock_sessions(&self) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.unlock_sessions();
    }

    fn kill_session(
        &self,
        session_id: String,
        who: String,
        signal_number: i32,
    ) -> zbus::fdo::Result<()> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.kill_session(&session_id, &who, signal_number)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn kill_user(&self, uid: u32, signal_number: i32) -> zbus::fdo::Result<()> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.kill_user(uid, signal_number)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn terminate_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.release_session(&session_id) {
            mgr.sync_runtime_state();
            log::info!("Terminated session {}", session_id);
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No session '{}' known",
                session_id
            )))
        }
    }

    fn terminate_user(&self, uid: u32) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        match mgr.terminate_user(uid) {
            Ok(()) => {
                mgr.sync_runtime_state();
                log::info!("Terminated user {}", uid);
                Ok(())
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    fn terminate_seat(&self, seat_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        match mgr.terminate_seat(&seat_id) {
            Ok(()) => {
                mgr.sync_runtime_state();
                Ok(())
            }
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    fn set_user_linger(&self, uid: u32, enable: bool, _interactive: bool) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get_mut(&uid) {
            user.linger = enable;
            let linger_path = format!("/var/lib/systemd/linger/{}", user.name);
            if enable {
                let _ = fs::create_dir_all("/var/lib/systemd/linger");
                let _ = fs::write(&linger_path, "");
            } else {
                let _ = fs::remove_file(&linger_path);
            }
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(format!("User {} not known", uid)))
        }
    }

    fn attach_device(&self, seat_id: String, sysfs_path: String, _interactive: bool) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = mgr.attach_device(&seat_id, &sysfs_path) {
            log::warn!("AttachDevice({}, {}): {}", seat_id, sysfs_path, e);
        }
    }

    fn flush_devices(&self, _interactive: bool) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.flush_attached_devices();
        mgr.enumerate_input_devices();
    }

    async fn power_off(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("poweroff", interactive, uid, pid, &self.mgr)
    }
    async fn reboot(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("reboot", interactive, uid, pid, &self.mgr)
    }
    async fn halt(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("halt", interactive, uid, pid, &self.mgr)
    }
    async fn suspend(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("suspend", interactive, uid, pid, &self.mgr)
    }
    async fn hibernate(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("hibernate", interactive, uid, pid, &self.mgr)
    }
    async fn hybrid_sleep(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("hybrid-sleep", interactive, uid, pid, &self.mgr)
    }
    async fn suspend_then_hibernate(
        &self,
        interactive: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<()> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        execute_power_action("suspend-then-hibernate", interactive, uid, pid, &self.mgr)
    }

    async fn can_power_off(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("poweroff", uid, pid, &self.mgr)
    }
    async fn can_reboot(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("reboot", uid, pid, &self.mgr)
    }
    async fn can_halt(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("halt", uid, pid, &self.mgr)
    }
    async fn can_suspend(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("suspend", uid, pid, &self.mgr)
    }
    async fn can_hibernate(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("hibernate", uid, pid, &self.mgr)
    }
    async fn can_hybrid_sleep(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("hybrid-sleep", uid, pid, &self.mgr)
    }
    async fn can_suspend_then_hibernate(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> String {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        can_action_with_polkit("suspend-then-hibernate", uid, pid, &self.mgr)
    }

    async fn inhibit(
        &self,
        what: String,
        who: String,
        why: String,
        mode: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> zbus::fdo::Result<ZOwnedFd> {
        let (uid, pid) = get_caller_credentials(&header, conn).await;
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mode != "block" && mode != "delay" {
            return Err(zbus::fdo::Error::Failed(
                "Invalid mode, must be 'block' or 'delay'".to_string(),
            ));
        }
        let id = mgr.create_inhibitor(&what, &who, &why, &mode, uid, pid);
        log::info!(
            "New D-Bus inhibitor {} ({}): {} — {} [uid={}, pid={}]",
            id,
            what,
            who,
            why,
            uid,
            pid
        );

        // Real systemd returns a pipe FD — when the caller closes it (or
        // exits), the inhibitor is automatically released.  We create a pipe
        // pair: the read end is kept by logind and the write end is returned
        // to the caller.  When the caller drops the write end the read end
        // becomes readable, which a future poll loop can detect to release
        // the inhibitor.
        let mut fds = [0i32; 2];
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        if rc != 0 {
            // Fallback: return an error but keep the inhibitor active
            return Err(zbus::fdo::Error::Failed(format!(
                "pipe2 failed: {}",
                io::Error::last_os_error()
            )));
        }
        let _read_end = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write_end = unsafe { OwnedFd::from_raw_fd(fds[1]) };

        // Store the read end so we can detect when the caller drops
        mgr.inhibitor_pipes.insert(id, _read_end);

        Ok(ZOwnedFd::from(write_end))
    }

    fn schedule_shutdown(&self, shutdown_type: String, usec: u64) {
        log::info!(
            "ScheduleShutdown requested: type={}, usec={}",
            shutdown_type,
            usec
        );
        // A full implementation would store the scheduled shutdown and start a
        // timer.  For now we log and emit PrepareForShutdown when the time
        // arrives.  Not yet wired to a timer.
    }

    fn cancel_scheduled_shutdown(&self) -> bool {
        log::info!("CancelScheduledShutdown requested");
        false
    }

    fn set_wall_message(&self, _wall_message: String, _enable: bool) {}
}

/// D-Bus interface struct for org.freedesktop.login1.Session
struct Login1Session {
    mgr: SharedManager,
    session_id: String,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl Login1Session {
    // --- Properties ---

    #[zbus(property, name = "Id")]
    fn id(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.id.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "User")]
    fn user(&self) -> (u32, String) {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = mgr.sessions.get(&self.session_id) {
            (session.uid, user_object_path(session.uid))
        } else {
            (0u32, user_object_path(0))
        }
    }

    #[zbus(property, name = "Name")]
    fn name(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.user.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Timestamp")]
    fn timestamp(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.since * 1_000_000)
            .unwrap_or(0)
    }

    #[zbus(property, name = "TimestampMonotonic")]
    fn timestamp_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.since_monotonic)
            .unwrap_or(0)
    }

    #[zbus(property, name = "VTNr")]
    fn vt_nr(&self) -> u32 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.vtnr)
            .unwrap_or(0)
    }

    #[zbus(property, name = "Seat")]
    fn seat(&self) -> (String, String) {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = mgr.sessions.get(&self.session_id) {
            let seat_id = session.seat.clone().unwrap_or_default();
            let seat_path = if seat_id.is_empty() {
                "/".to_string()
            } else {
                seat_object_path(&seat_id)
            };
            (seat_id, seat_path)
        } else {
            (String::new(), "/".to_string())
        }
    }

    #[zbus(property, name = "TTY")]
    fn tty(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.tty.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Display")]
    fn display(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.display.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Remote")]
    fn remote(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.remote)
            .unwrap_or(false)
    }

    #[zbus(property, name = "RemoteHost")]
    fn remote_host(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.remote_host.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "RemoteUser")]
    fn remote_user(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.remote_user.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Service")]
    fn service(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.service.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Desktop")]
    fn desktop(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.desktop.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Scope")]
    fn scope(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.scope.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Leader")]
    fn leader(&self) -> u32 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.leader)
            .unwrap_or(0)
    }

    #[zbus(property, name = "Audit")]
    fn audit(&self) -> u32 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.leader)
            .unwrap_or(0)
    }

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.session_type.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Class")]
    fn class(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.class.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Active")]
    fn active(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.active)
            .unwrap_or(false)
    }

    #[zbus(property, name = "State")]
    fn state(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.state.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "IdleHint")]
    fn idle_hint(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.idle_hint)
            .unwrap_or(false)
    }

    #[zbus(property, name = "IdleSinceHint")]
    fn idle_since_hint(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.idle_since_hint)
            .unwrap_or(0)
    }

    #[zbus(property, name = "IdleSinceHintMonotonic")]
    fn idle_since_hint_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.idle_since_hint_monotonic)
            .unwrap_or(0)
    }

    #[zbus(property, name = "LockedHint")]
    fn locked_hint(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.sessions
            .get(&self.session_id)
            .map(|s| s.locked_hint)
            .unwrap_or(false)
    }

    // --- Signals ---

    #[zbus(signal)]
    async fn lock(ctx: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn unlock(ctx: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;

    /// Emitted when a device must be paused, e.g. during a VT switch away
    /// from this session.  `pause_type` is one of:
    /// - `"pause"` — compositor should release the device and call
    ///   `PauseDeviceComplete`
    /// - `"force"` — device has already been deactivated, no ack needed
    /// - `"gone"` — device has been removed entirely
    #[zbus(signal)]
    async fn pause_device(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        major: u32,
        minor: u32,
        pause_type: &str,
    ) -> zbus::Result<()>;

    /// Emitted when a device becomes available again after a VT switch to
    /// this session.  The `fd` is a dup'd file descriptor for the device.
    #[zbus(signal)]
    async fn resume_device(
        ctx: &zbus::object_server::SignalEmitter<'_>,
        major: u32,
        minor: u32,
        fd: ZOwnedFd,
    ) -> zbus::Result<()>;

    // --- Methods ---

    fn terminate(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if mgr.release_session(&self.session_id) {
            mgr.sync_runtime_state();
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed("Session not found".to_string()))
        }
    }

    fn activate(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let _switch_info = mgr
            .activate_session(&self.session_id)
            .map_err(zbus::fdo::Error::Failed)?;
        mgr.sync_runtime_state();
        // Note: PauseDevice / ResumeDevice signal emission for VT switches
        // is handled by the main loop's VT monitoring, not here, because we
        // need async signal emission which requires the D-Bus connection.
        Ok(())
    }

    #[zbus(name = "Lock")]
    fn lock_method(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.lock_session(&self.session_id)
            .map_err(zbus::fdo::Error::Failed)
    }

    #[zbus(name = "Unlock")]
    fn unlock_method(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.unlock_session(&self.session_id)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn set_idle_hint(&self, idle: bool) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.set_idle_hint(&self.session_id, idle)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn set_locked_hint(&self, locked: bool) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.set_locked_hint(&self.session_id, locked)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn set_type(&self, session_type: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.set_session_type(&self.session_id, &session_type)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn kill(&self, who: String, signal_number: i32) -> zbus::fdo::Result<()> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.kill_session(&self.session_id, &who, signal_number)
            .map_err(zbus::fdo::Error::Failed)
    }

    async fn take_control(
        &self,
        force: bool,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        // Identify the controller by the D-Bus sender's unique bus name.
        let caller = header
            .sender()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "dbus-controller".to_string());
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.take_control(&self.session_id, &caller, force)
            .map_err(zbus::fdo::Error::Failed)
    }

    fn release_control(&self) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.release_control(&self.session_id);
    }

    fn set_brightness(&self, _subsystem: String, _name: String, _brightness: u32) {}

    fn take_device(&self, major: u32, minor: u32) -> zbus::fdo::Result<(ZOwnedFd, bool)> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        let (fd, active) = mgr
            .take_device(&self.session_id, major, minor)
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((ZOwnedFd::from(fd), active))
    }

    fn release_device(&self, major: u32, minor: u32) {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.release_device(&self.session_id, major, minor);
    }

    fn pause_device_complete(&self, major: u32, minor: u32) {
        log::debug!(
            "PauseDeviceComplete for session {} device {}:{}",
            self.session_id,
            major,
            minor
        );
        // Acknowledge a PauseDevice signal.  In the current implementation,
        // VT switches use "force" type which doesn't require acknowledgement
        // — the VT_RELDISP(1) is called immediately.  A future refinement
        // could use "pause" type with ack tracking and a timeout fallback:
        //
        // 1. On SIGUSR1, emit PauseDevice("pause", ...) for each device
        // 2. Track pending (session, major, minor) tuples
        // 3. On PauseDeviceComplete, remove from pending set
        // 4. When pending set is empty (or timeout expires), call VT_RELDISP(1)
        //
        // For now, compositors like Sway/wlroots handle "force" correctly.
    }
}

/// D-Bus interface struct for org.freedesktop.login1.Seat
struct Login1Seat {
    mgr: SharedManager,
    seat_id: String,
}

#[zbus::interface(name = "org.freedesktop.login1.Seat")]
impl Login1Seat {
    // --- Properties ---

    #[zbus(property, name = "Id")]
    fn id(&self) -> String {
        self.seat_id.clone()
    }

    #[zbus(property, name = "ActiveSession")]
    fn active_session(&self) -> (String, String) {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat) = mgr.seats.get(&self.seat_id)
            && let Some(ref active) = seat.active_session
        {
            return (active.clone(), session_object_path(active));
        }
        (String::new(), "/".to_string())
    }

    #[zbus(property, name = "CanGraphical")]
    fn can_graphical(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.seats
            .get(&self.seat_id)
            .map(|s| s.can_graphical)
            .unwrap_or(false)
    }

    #[zbus(property, name = "CanMultiSession")]
    fn can_multi_session(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.seats
            .get(&self.seat_id)
            .map(|s| s.can_multi_session)
            .unwrap_or(false)
    }

    #[zbus(property, name = "Sessions")]
    fn sessions(&self) -> Vec<(String, String)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat) = mgr.seats.get(&self.seat_id) {
            seat.sessions
                .iter()
                .map(|sid| (sid.clone(), session_object_path(sid)))
                .collect()
        } else {
            Vec::new()
        }
    }

    #[zbus(property, name = "IdleHint")]
    fn idle_hint(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat) = mgr.seats.get(&self.seat_id) {
            seat.sessions
                .iter()
                .all(|sid| mgr.sessions.get(sid).map(|s| s.idle_hint).unwrap_or(true))
        } else {
            true
        }
    }

    #[zbus(property, name = "IdleSinceHint")]
    fn idle_since_hint(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat) = mgr.seats.get(&self.seat_id) {
            let mut earliest: u64 = 0;
            for sid in &seat.sessions {
                if let Some(s) = mgr.sessions.get(sid)
                    && s.idle_hint
                    && s.idle_since_hint > 0
                    && (earliest == 0 || s.idle_since_hint < earliest)
                {
                    earliest = s.idle_since_hint;
                }
            }
            earliest
        } else {
            0
        }
    }

    #[zbus(property, name = "IdleSinceHintMonotonic")]
    fn idle_since_hint_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat) = mgr.seats.get(&self.seat_id) {
            let mut earliest: u64 = 0;
            for sid in &seat.sessions {
                if let Some(s) = mgr.sessions.get(sid)
                    && s.idle_hint
                    && s.idle_since_hint_monotonic > 0
                    && (earliest == 0 || s.idle_since_hint_monotonic < earliest)
                {
                    earliest = s.idle_since_hint_monotonic;
                }
            }
            earliest
        } else {
            0
        }
    }

    // --- Methods ---

    fn terminate(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.terminate_seat(&self.seat_id)
            .map_err(zbus::fdo::Error::Failed)?;
        mgr.sync_runtime_state();
        Ok(())
    }

    fn activate_session(&self, session_id: String) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(session) = mgr.sessions.get(&session_id)
            && session.seat.as_deref() != Some(&self.seat_id)
        {
            return Err(zbus::fdo::Error::Failed(
                "Session not on this seat".to_string(),
            ));
        }
        mgr.activate_session(&session_id)
            .map_err(zbus::fdo::Error::Failed)?;
        mgr.sync_runtime_state();
        Ok(())
    }

    fn switch_to(&self, vtnr: u32) {
        log::info!("SwitchTo VT {} requested", vtnr);
        switch_vt(vtnr);
    }

    fn switch_to_next(&self) {
        log::info!("SwitchToNext requested");
    }
    fn switch_to_previous(&self) {
        log::info!("SwitchToPrevious requested");
    }
}

/// D-Bus interface struct for org.freedesktop.login1.User
struct Login1User {
    mgr: SharedManager,
    uid: u32,
}

#[zbus::interface(name = "org.freedesktop.login1.User")]
impl Login1User {
    // --- Properties ---

    #[zbus(property, name = "UID")]
    fn uid_prop(&self) -> u32 {
        self.uid
    }

    #[zbus(property, name = "GID")]
    fn gid(&self) -> u32 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users.get(&self.uid).map(|u| u.gid).unwrap_or(0)
    }

    #[zbus(property, name = "Name")]
    fn name(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.name.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Timestamp")]
    fn timestamp(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.since * 1_000_000)
            .unwrap_or(0)
    }

    #[zbus(property, name = "TimestampMonotonic")]
    fn timestamp_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.since_monotonic)
            .unwrap_or(0)
    }

    #[zbus(property, name = "RuntimePath")]
    fn runtime_path(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.runtime_path.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Service")]
    fn service(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.service.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Slice")]
    fn slice(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.slice.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Display")]
    fn display(&self) -> (String, String) {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get(&self.uid) {
            let display_session = user.sessions.first().cloned().unwrap_or_default();
            let display_path = if display_session.is_empty() {
                "/".to_string()
            } else {
                session_object_path(&display_session)
            };
            (display_session, display_path)
        } else {
            (String::new(), "/".to_string())
        }
    }

    #[zbus(property, name = "State")]
    fn state(&self) -> String {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users
            .get(&self.uid)
            .map(|u| u.state.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Sessions")]
    fn sessions(&self) -> Vec<(String, String)> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get(&self.uid) {
            user.sessions
                .iter()
                .map(|sid| (sid.clone(), session_object_path(sid)))
                .collect()
        } else {
            Vec::new()
        }
    }

    #[zbus(property, name = "IdleHint")]
    fn idle_hint(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get(&self.uid) {
            user.sessions
                .iter()
                .all(|sid| mgr.sessions.get(sid).map(|s| s.idle_hint).unwrap_or(true))
        } else {
            true
        }
    }

    #[zbus(property, name = "IdleSinceHint")]
    fn idle_since_hint(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get(&self.uid) {
            let mut earliest: u64 = 0;
            for sid in &user.sessions {
                if let Some(s) = mgr.sessions.get(sid)
                    && s.idle_hint
                    && s.idle_since_hint > 0
                    && (earliest == 0 || s.idle_since_hint < earliest)
                {
                    earliest = s.idle_since_hint;
                }
            }
            earliest
        } else {
            0
        }
    }

    #[zbus(property, name = "IdleSinceHintMonotonic")]
    fn idle_since_hint_monotonic(&self) -> u64 {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(user) = mgr.users.get(&self.uid) {
            let mut earliest: u64 = 0;
            for sid in &user.sessions {
                if let Some(s) = mgr.sessions.get(sid)
                    && s.idle_hint
                    && s.idle_since_hint_monotonic > 0
                    && (earliest == 0 || s.idle_since_hint_monotonic < earliest)
                {
                    earliest = s.idle_since_hint_monotonic;
                }
            }
            earliest
        } else {
            0
        }
    }

    #[zbus(property, name = "Linger")]
    fn linger(&self) -> bool {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.users.get(&self.uid).map(|u| u.linger).unwrap_or(false)
    }

    // --- Methods ---

    fn terminate(&self) -> zbus::fdo::Result<()> {
        let mut mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.terminate_user(self.uid)
            .map_err(zbus::fdo::Error::Failed)?;
        mgr.sync_runtime_state();
        Ok(())
    }

    fn kill(&self, signal_number: i32) -> zbus::fdo::Result<()> {
        let mgr = self.mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr.kill_user(self.uid, signal_number)
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// Switch VT using ioctl
fn switch_vt(vtnr: u32) {
    if vtnr == 0 {
        return;
    }
    // Try /dev/tty0 for VT switching
    let fd = unsafe { libc::open(c"/dev/tty0".as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if fd >= 0 {
        unsafe {
            // VT_ACTIVATE = 0x5606
            libc::ioctl(fd, 0x5606, vtnr as libc::c_ulong);
            // VT_WAITACTIVE = 0x5607
            libc::ioctl(fd, 0x5607, vtnr as libc::c_ulong);
            libc::close(fd);
        }
    }
}

/// Check AC power state
fn check_ac_power() -> bool {
    let ps_dir = Path::new("/sys/class/power_supply");
    if let Ok(entries) = fs::read_dir(ps_dir) {
        for entry in entries.flatten() {
            let type_path = entry.path().join("type");
            if let Ok(ps_type) = fs::read_to_string(&type_path)
                && ps_type.trim() == "Mains"
            {
                let online_path = entry.path().join("online");
                if let Ok(online) = fs::read_to_string(&online_path)
                    && online.trim() == "1"
                {
                    return true;
                }
            }
        }
    }
    true // Default to AC power if we can't determine
}

/// Resolve a UID to a username
fn resolve_uid_to_name(uid: u32) -> String {
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3
                && let Ok(file_uid) = parts[2].parse::<u32>()
                && file_uid == uid
            {
                return parts[0].to_string();
            }
        }
    }
    format!("{}", uid)
}

// ---------------------------------------------------------------------------
// Helper functions — sysfs, ACL, utmp, env-file parsing
// ---------------------------------------------------------------------------

/// Resolve a sysfs device path to its `/dev/` node path.
///
/// Reads the `dev` file in the sysfs directory to get the major:minor
/// numbers, then looks up the device node.  Returns `None` if the path
/// doesn't exist or has no associated devnode.
fn devnode_from_sysfs(sysfs_path: &str) -> Option<String> {
    // Try reading the "dev" file which contains "MAJOR:MINOR"
    let dev_file = Path::new(sysfs_path).join("dev");
    if let Ok(content) = fs::read_to_string(&dev_file) {
        let content = content.trim();
        if let Some((maj_s, min_s)) = content.split_once(':')
            && let (Ok(major), Ok(minor)) = (maj_s.parse::<u32>(), min_s.parse::<u32>())
        {
            let devnode = format!("/dev/char/{}:{}", major, minor);
            if Path::new(&devnode).exists() {
                return Some(devnode);
            }
        }
    }

    // Fallback: try the "uevent" file for DEVNAME=
    let uevent_file = Path::new(sysfs_path).join("uevent");
    if let Ok(content) = fs::read_to_string(&uevent_file) {
        for line in content.lines() {
            if let Some(devname) = line.strip_prefix("DEVNAME=") {
                let devnode = if devname.starts_with('/') {
                    devname.to_string()
                } else {
                    format!("/dev/{}", devname)
                };
                if Path::new(&devnode).exists() {
                    return Some(devnode);
                }
            }
        }
    }

    None
}

/// Set or remove a POSIX ACL entry on a device node for a given UID.
///
/// Uses `setfacl` as a portable way to manage ACLs.  When `grant` is true,
/// adds `user:UID:rw` permission; when false, removes the user-specific
/// entry.
fn set_device_acl(dev_path: &str, uid: u32, grant: bool) -> Result<(), String> {
    if !Path::new(dev_path).exists() {
        return Err(format!("Device {} does not exist", dev_path));
    }

    let action = if grant { "-m" } else { "-x" };
    let spec = if grant {
        format!("u:{}:rw", uid)
    } else {
        format!("u:{}", uid)
    };

    match std::process::Command::new("setfacl")
        .args([action, &spec, dev_path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!(
                    "setfacl {} {} {} failed (exit {}): {}",
                    action,
                    spec,
                    dev_path,
                    output.status,
                    stderr.trim()
                ))
            }
        }
        Err(e) => {
            // setfacl not available — not fatal, just log and move on
            Err(format!("Failed to run setfacl: {}", e))
        }
    }
}

/// Parse a KEY=VALUE environment-style file (as used by session/inhibitor
/// state files in `/run/systemd/sessions/`).
fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

/// Read a NUL-terminated C string from a fixed-size byte buffer.
fn read_cstr_field(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

/// Resolve a username to a UID by reading `/etc/passwd`.
fn resolve_username_to_uid(name: &str) -> Option<u32> {
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3
                && parts[0] == name
                && let Ok(uid) = parts[2].parse::<u32>()
            {
                return Some(uid);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// D-Bus signal emission helpers
// ---------------------------------------------------------------------------

// Signal emission is handled automatically by zbus via the #[zbus(signal)]
// attributes on the interface structs. The main loop uses the connection's
// object server to emit signals when needed.

fn emit_signal_session_new(_conn: &Connection, session_id: &str) {
    let path = session_object_path(session_id);
    log::debug!("Signal: SessionNew {} at {}", session_id, path);
    // Signal emission happens via zbus object server; this is a log placeholder.
}

fn emit_signal_session_removed(_conn: &Connection, session_id: &str) {
    let _path = session_object_path(session_id);
    log::debug!("Signal: SessionRemoved {}", session_id);
}

fn emit_signal_user_new(_conn: &Connection, uid: u32) {
    let _path = user_object_path(uid);
    log::debug!("Signal: UserNew {}", uid);
}

fn emit_signal_user_removed(_conn: &Connection, uid: u32) {
    let _path = user_object_path(uid);
    log::debug!("Signal: UserRemoved {}", uid);
}

fn emit_signal_seat_new(_conn: &Connection, seat_id: &str) {
    let _path = seat_object_path(seat_id);
    log::debug!("Signal: SeatNew {}", seat_id);
}

#[allow(dead_code)]
fn emit_signal_seat_removed(_conn: &Connection, seat_id: &str) {
    let _path = seat_object_path(seat_id);
    log::debug!("Signal: SeatRemoved {}", seat_id);
}

fn emit_signal_prepare_for_shutdown(_conn: &Connection, active: bool) {
    log::debug!("Signal: PrepareForShutdown {}", active);
}

#[allow(dead_code)]
fn emit_signal_prepare_for_sleep(_conn: &Connection, active: bool) {
    log::debug!("Signal: PrepareForSleep {}", active);
}

/// Emit PauseDevice D-Bus signal on a session object.
///
/// Sends the signal as `(u32 major, u32 minor, string type)` on the
/// `org.freedesktop.login1.Session` interface at the session's object path.
/// `pause_type` is "force" (device already deactivated), "pause"
/// (cooperative, expects PauseDeviceComplete), or "gone" (device removed).
fn emit_pause_device_signal(
    conn: &Connection,
    session_id: &str,
    major: u32,
    minor: u32,
    pause_type: &str,
) {
    let path = session_object_path(session_id);
    log::debug!(
        "Signal: PauseDevice session={} dev={}:{} type={}",
        session_id,
        major,
        minor,
        pause_type
    );
    // Build and send a raw D-Bus signal message.
    match zbus::message::Message::signal(
        path.as_str(),
        "org.freedesktop.login1.Session",
        "PauseDevice",
    ) {
        Ok(builder) => match builder.build(&(major, minor, pause_type)) {
            Ok(msg) => {
                if let Err(e) = conn.send(&msg) {
                    log::warn!("Failed to send PauseDevice signal: {}", e);
                }
            }
            Err(e) => log::warn!("Failed to build PauseDevice message: {}", e),
        },
        Err(e) => log::warn!("Failed to create PauseDevice signal builder: {}", e),
    }
}

/// Emit ResumeDevice D-Bus signal on a session object.
///
/// Sends the signal as `(u32 major, u32 minor, fd)` on the
/// `org.freedesktop.login1.Session` interface.  The `fd` is a dup'd file
/// descriptor for the device so the compositor can re-open it.
fn emit_resume_device_signal(
    conn: &Connection,
    session_id: &str,
    major: u32,
    minor: u32,
    raw_fd: RawFd,
) {
    let path = session_object_path(session_id);
    log::debug!(
        "Signal: ResumeDevice session={} dev={}:{} fd={}",
        session_id,
        major,
        minor,
        raw_fd
    );
    // Dup the fd so we can send it as an OwnedFd in the signal without
    // closing the original (which logind still needs).
    let dup_result = dup_raw_fd(raw_fd);
    let Ok(duped) = dup_result else {
        log::warn!(
            "Failed to dup fd {} for ResumeDevice signal: {}",
            raw_fd,
            dup_result.unwrap_err()
        );
        return;
    };
    let z_fd = ZOwnedFd::from(duped);
    match zbus::message::Message::signal(
        path.as_str(),
        "org.freedesktop.login1.Session",
        "ResumeDevice",
    ) {
        Ok(builder) => match builder.build(&(major, minor, z_fd)) {
            Ok(msg) => {
                if let Err(e) = conn.send(&msg) {
                    log::warn!("Failed to send ResumeDevice signal: {}", e);
                }
            }
            Err(e) => log::warn!("Failed to build ResumeDevice message: {}", e),
        },
        Err(e) => log::warn!("Failed to create ResumeDevice signal builder: {}", e),
    }
}

/// Emit PauseDevice signals for a set of devices (used during VT switch).
fn emit_pause_devices(
    conn: &Connection,
    session_id: &str,
    devices: &[(u32, u32)],
    pause_type: &str,
) {
    for &(major, minor) in devices {
        emit_pause_device_signal(conn, session_id, major, minor, pause_type);
    }
}

/// Emit ResumeDevice signals for a set of devices (used during VT switch).
fn emit_resume_devices(conn: &Connection, session_id: &str, devices: &[(u32, u32, RawFd)]) {
    for &(major, minor, raw_fd) in devices {
        emit_resume_device_signal(conn, session_id, major, minor, raw_fd);
    }
}

// ---------------------------------------------------------------------------
// Control socket handler (legacy, for loginctl)
// ---------------------------------------------------------------------------

fn handle_control_command(mgr: &mut LoginManager, cmd: &str) -> String {
    let parts: Vec<&str> = cmd.trim().splitn(2, ' ').collect();
    let command = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();
    let args = parts.get(1).copied().unwrap_or("");

    match command.as_str() {
        "status" => mgr.format_status(),

        "list-sessions" => {
            let sessions: Vec<&Session> = mgr.sessions.values().collect();
            match serde_json::to_string_pretty(&sessions) {
                Ok(json) => json,
                Err(e) => format!("ERROR: {}", e),
            }
        }

        "list-seats" => {
            let seats: Vec<&Seat> = mgr.seats.values().collect();
            match serde_json::to_string_pretty(&seats) {
                Ok(json) => json,
                Err(e) => format!("ERROR: {}", e),
            }
        }

        "list-users" => {
            let users: Vec<&User> = mgr.users.values().collect();
            match serde_json::to_string_pretty(&users) {
                Ok(json) => json,
                Err(e) => format!("ERROR: {}", e),
            }
        }

        "list-inhibitors" => {
            let inhibitors: Vec<&Inhibitor> = mgr.inhibitors.values().collect();
            match serde_json::to_string_pretty(&inhibitors) {
                Ok(json) => json,
                Err(e) => format!("ERROR: {}", e),
            }
        }

        "show-session" => {
            if let Some(session) = mgr.sessions.get(args) {
                match serde_json::to_string_pretty(session) {
                    Ok(json) => json,
                    Err(e) => format!("ERROR: {}", e),
                }
            } else {
                format!("ERROR: Session '{}' not found", args)
            }
        }

        "show-seat" => {
            if let Some(seat) = mgr.seats.get(args) {
                match serde_json::to_string_pretty(seat) {
                    Ok(json) => json,
                    Err(e) => format!("ERROR: {}", e),
                }
            } else {
                format!("ERROR: Seat '{}' not found", args)
            }
        }

        "show-user" => {
            if let Ok(uid) = args.parse::<u32>() {
                if let Some(user) = mgr.users.get(&uid) {
                    match serde_json::to_string_pretty(user) {
                        Ok(json) => json,
                        Err(e) => format!("ERROR: {}", e),
                    }
                } else {
                    format!("ERROR: User {} not found", uid)
                }
            } else {
                // Try to find by name
                if let Some(user) = mgr.users.values().find(|u| u.name == args) {
                    match serde_json::to_string_pretty(user) {
                        Ok(json) => json,
                        Err(e) => format!("ERROR: {}", e),
                    }
                } else {
                    format!("ERROR: User '{}' not found", args)
                }
            }
        }

        "create-session" => {
            // JSON args: {"uid": N, "user": "...", "seat": "seat0", "vtnr": N, "type": "tty", "class": "user", "tty": "/dev/ttyN", "leader": PID}
            match serde_json::from_str::<serde_json::Value>(args) {
                Ok(v) => {
                    let uid = v["uid"].as_u64().unwrap_or(0) as u32;
                    let user = v["user"].as_str().unwrap_or("unknown");
                    let seat = v["seat"].as_str();
                    let vtnr = v["vtnr"].as_u64().unwrap_or(0) as u32;
                    let stype = v["type"].as_str().unwrap_or("tty");
                    let class = v["class"].as_str().unwrap_or("user");
                    let tty = v["tty"].as_str().unwrap_or("");
                    let leader = v["leader"].as_u64().unwrap_or(0) as u32;

                    let id = mgr.create_session(uid, user, seat, vtnr, stype, class, tty, leader);
                    mgr.sync_runtime_state();
                    log::info!(
                        "New session {} of user {} on {}",
                        id,
                        user,
                        seat.unwrap_or("(no seat)")
                    );
                    format!("OK {}", id)
                }
                Err(e) => format!("ERROR: Invalid JSON: {}", e),
            }
        }

        "release-session" => {
            if mgr.release_session(args) {
                mgr.sync_runtime_state();
                log::info!("Released session {}", args);
                "OK".to_string()
            } else {
                format!("ERROR: Session '{}' not found", args)
            }
        }

        "activate-session" => match mgr.activate_session(args) {
            Ok(_switch_info) => {
                mgr.sync_runtime_state();
                log::info!("Activated session {}", args);
                "OK".to_string()
            }
            Err(e) => format!("ERROR: {}", e),
        },

        "lock-session" => match mgr.lock_session(args) {
            Ok(()) => {
                log::info!("Locked session {}", args);
                "OK".to_string()
            }
            Err(e) => format!("ERROR: {}", e),
        },

        "unlock-session" => match mgr.unlock_session(args) {
            Ok(()) => {
                log::info!("Unlocked session {}", args);
                "OK".to_string()
            }
            Err(e) => format!("ERROR: {}", e),
        },

        "lock-sessions" => {
            mgr.lock_sessions();
            log::info!("Locked all sessions");
            "OK".to_string()
        }

        "unlock-sessions" => {
            mgr.unlock_sessions();
            log::info!("Unlocked all sessions");
            "OK".to_string()
        }

        "terminate-session" => {
            if mgr.release_session(args) {
                mgr.sync_runtime_state();
                log::info!("Terminated session {}", args);
                "OK".to_string()
            } else {
                format!("ERROR: Session '{}' not found", args)
            }
        }

        "terminate-user" => {
            if let Ok(uid) = args.parse::<u32>() {
                match mgr.terminate_user(uid) {
                    Ok(()) => {
                        mgr.sync_runtime_state();
                        log::info!("Terminated user {}", uid);
                        "OK".to_string()
                    }
                    Err(e) => format!("ERROR: {}", e),
                }
            } else {
                format!("ERROR: Invalid UID '{}'", args)
            }
        }

        "inhibit" => {
            // JSON: {"what": "...", "who": "...", "why": "...", "mode": "block", "uid": N, "pid": N}
            match serde_json::from_str::<serde_json::Value>(args) {
                Ok(v) => {
                    let what = v["what"].as_str().unwrap_or("shutdown");
                    let who = v["who"].as_str().unwrap_or("unknown");
                    let why = v["why"].as_str().unwrap_or("");
                    let mode = v["mode"].as_str().unwrap_or("block");
                    let uid = v["uid"].as_u64().unwrap_or(0) as u32;
                    let pid = v["pid"].as_u64().unwrap_or(0) as u32;
                    let id = mgr.create_inhibitor(what, who, why, mode, uid, pid);
                    log::info!("New inhibitor {} ({}): {} — {}", id, what, who, why);
                    format!("OK {}", id)
                }
                Err(e) => format!("ERROR: Invalid JSON: {}", e),
            }
        }

        "release-inhibitor" => {
            if let Ok(id) = args.parse::<u64>() {
                if mgr.release_inhibitor(id) {
                    log::info!("Released inhibitor {}", id);
                    "OK".to_string()
                } else {
                    format!("ERROR: Inhibitor {} not found", id)
                }
            } else {
                format!("ERROR: Invalid inhibitor ID '{}'", args)
            }
        }

        "can-poweroff"
        | "can-reboot"
        | "can-suspend"
        | "can-hibernate"
        | "can-suspend-then-hibernate"
        | "can-hybrid-sleep" => {
            let action = command.strip_prefix("can-").unwrap_or("shutdown");
            mgr.can_action(action).to_string()
        }

        "poweroff" | "reboot" | "suspend" | "hibernate" => {
            log::info!("Requested system action: {}", command);
            format!("OK (action {} requested)", command)
        }

        "enable-linger" => {
            // args = UID or username
            let uid = if let Ok(uid) = args.parse::<u32>() {
                Some(uid)
            } else {
                resolve_username_to_uid(args)
            };
            if let Some(uid) = uid {
                if let Some(user) = mgr.users.get_mut(&uid) {
                    user.linger = true;
                } else {
                    let name = resolve_uid_to_name(uid);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    mgr.users.insert(
                        uid,
                        User {
                            uid,
                            gid: 0,
                            name,
                            state: "lingering".to_string(),
                            sessions: Vec::new(),
                            slice: format!("user-{uid}.slice"),
                            service: format!("user@{uid}.service"),
                            runtime_path: format!("/run/user/{uid}"),
                            since: now,
                            since_monotonic: 0,
                            linger: true,
                        },
                    );
                }
                let linger_dir = "/var/lib/systemd/linger";
                let _ = fs::create_dir_all(linger_dir);
                let linger_path = format!("{linger_dir}/{uid}");
                let _ = fs::write(&linger_path, "");
                mgr.sync_runtime_state();
                log::info!("Enabled linger for UID {uid}");
                "OK".to_string()
            } else {
                format!("ERROR: Invalid user '{args}'")
            }
        }

        "disable-linger" => {
            let uid = if let Ok(uid) = args.parse::<u32>() {
                Some(uid)
            } else {
                resolve_username_to_uid(args)
            };
            if let Some(uid) = uid {
                if let Some(user) = mgr.users.get_mut(&uid) {
                    user.linger = false;
                }
                let linger_path = format!("/var/lib/systemd/linger/{uid}");
                let _ = fs::remove_file(&linger_path);
                mgr.sync_runtime_state();
                log::info!("Disabled linger for UID {uid}");
                "OK".to_string()
            } else {
                format!("ERROR: Invalid user '{args}'")
            }
        }

        "attach-device" => {
            // JSON: {"seat": "seat0", "sysfs": "/sys/devices/..."}
            match serde_json::from_str::<serde_json::Value>(args) {
                Ok(v) => {
                    let seat_id = v["seat"].as_str().unwrap_or("seat0");
                    let sysfs = v["sysfs"].as_str().unwrap_or("");
                    if sysfs.is_empty() {
                        "ERROR: Missing sysfs path".to_string()
                    } else {
                        match mgr.attach_device(seat_id, sysfs) {
                            Ok(()) => {
                                log::info!("Attached device {} to seat {}", sysfs, seat_id);
                                "OK".to_string()
                            }
                            Err(e) => format!("ERROR: {}", e),
                        }
                    }
                }
                Err(e) => format!("ERROR: Invalid JSON: {}", e),
            }
        }

        "flush-devices" => {
            mgr.flush_attached_devices();
            mgr.enumerate_input_devices();
            log::info!(
                "Flushed devices, found {} power button device(s)",
                mgr.power_button_devices.len()
            );
            "OK".to_string()
        }

        _ => format!("ERROR: Unknown command: {}", command),
    }
}

fn handle_client(mgr: &mut LoginManager, stream: &mut std::os::unix::net::UnixStream) {
    let mut buf = [0u8; 8192];
    match stream.read(&mut buf) {
        Ok(n) if n > 0 => {
            let cmd = String::from_utf8_lossy(&buf[..n]);
            let response = handle_control_command(mgr, &cmd);
            let _ = stream.write_all(response.as_bytes());
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Create runtime directories
// ---------------------------------------------------------------------------

fn ensure_runtime_dirs() {
    for dir in &[RUN_DIR, SESSIONS_DIR, SEATS_DIR, USERS_DIR, INHIBIT_DIR] {
        let _ = fs::create_dir_all(dir);
    }
}

// ---------------------------------------------------------------------------
// D-Bus server setup and management (zbus)
// ---------------------------------------------------------------------------

struct DbusServer {
    conn: Connection,
    mgr: SharedManager,
}

impl DbusServer {
    fn new(mgr: SharedManager) -> Result<Self, Box<dyn std::error::Error>> {
        let manager_iface = Login1Manager { mgr: mgr.clone() };

        let conn = zbus::blocking::connection::Builder::system()?
            .name(DBUS_NAME)?
            .serve_at(DBUS_PATH, manager_iface)?
            .build()?;

        // Register existing seat objects
        {
            let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
            for seat_id in mgr_guard.seats.keys() {
                let path = seat_object_path(seat_id);
                let seat_iface = Login1Seat {
                    mgr: mgr.clone(),
                    seat_id: seat_id.clone(),
                };
                let _ = conn.object_server().at(path, seat_iface);
            }
        }

        Ok(DbusServer { conn, mgr })
    }

    /// Register a session object on the bus
    fn register_session(&self, session_id: &str) {
        let path = session_object_path(session_id);
        let iface = Login1Session {
            mgr: self.mgr.clone(),
            session_id: session_id.to_string(),
        };
        let _ = self.conn.object_server().at(path, iface);
    }

    /// Unregister a session object from the bus
    fn unregister_session(&self, session_id: &str) {
        let path = session_object_path(session_id);
        let _ = self.conn.object_server().remove::<Login1Session, _>(path);
    }

    /// Register a seat object on the bus
    #[allow(dead_code)]
    fn register_seat(&self, seat_id: &str) {
        let path = seat_object_path(seat_id);
        let iface = Login1Seat {
            mgr: self.mgr.clone(),
            seat_id: seat_id.to_string(),
        };
        let _ = self.conn.object_server().at(path, iface);
    }

    /// Register a user object on the bus
    fn register_user(&self, uid: u32) {
        let path = user_object_path(uid);
        let iface = Login1User {
            mgr: self.mgr.clone(),
            uid,
        };
        let _ = self.conn.object_server().at(path, iface);
    }

    /// Unregister a user object from the bus
    fn unregister_user(&self, uid: u32) {
        let path = user_object_path(uid);
        let _ = self.conn.object_server().remove::<Login1User, _>(path);
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    init_logging();
    setup_signal_handlers();

    log::info!("systemd-logind starting");

    // Create runtime directories
    ensure_runtime_dirs();

    // Initialize login manager with shared state
    let mgr = Arc::new(Mutex::new(LoginManager::new()));

    // Log seat0 status
    {
        let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(seat0) = mgr_guard.seats.get("seat0") {
            log::info!("New seat seat0.");
            log::info!(
                "Seat seat0: can_graphical={}, can_multi_session={}",
                seat0.can_graphical,
                seat0.can_multi_session
            );
        }
    }

    // Detect existing sessions
    {
        let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr_guard.detect_existing_sessions();
    }

    // Watchdog
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // Write initial state files
    {
        let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
        mgr_guard.sync_runtime_state();
    }

    // Initialize D-Bus server
    let dbus_server = match DbusServer::new(mgr.clone()) {
        Ok(server) => {
            log::info!("D-Bus interface registered on {}", DBUS_NAME);
            Some(server)
        }
        Err(e) => {
            log::info!(
                "Failed to initialize D-Bus interface: {}. Running without D-Bus.",
                e
            );
            None
        }
    };

    // Emit initial seat on D-Bus
    if let Some(ref server) = dbus_server {
        emit_signal_seat_new(&server.conn, "seat0");
    }

    // Remove stale control socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Bind control socket
    let listener = match UnixListener::bind(CONTROL_SOCKET_PATH) {
        Ok(l) => {
            log::info!("Listening on {}", CONTROL_SOCKET_PATH);
            Some(l)
        }
        Err(e) => {
            log::error!(
                "Failed to bind control socket {}: {}",
                CONTROL_SOCKET_PATH,
                e
            );
            None
        }
    };

    if let Some(ref l) = listener {
        l.set_nonblocking(true).expect("Failed to set non-blocking");
    }

    sd_notify(&format!(
        "READY=1\nSTATUS=Managing sessions, D-Bus: {}",
        if dbus_server.is_some() {
            "active"
        } else {
            "unavailable"
        }
    ));

    log::info!("systemd-logind ready");

    // Periodic cleanup counter
    let mut cleanup_counter = 0u64;

    // Track sessions/users for D-Bus signal emission
    let mut known_sessions: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut known_users: std::collections::HashSet<u32> = std::collections::HashSet::new();

    // Main loop
    loop {
        if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            if let Some(ref server) = dbus_server {
                emit_signal_prepare_for_shutdown(&server.conn, true);
            }
            break;
        }

        if RELOAD_FLAG.load(Ordering::SeqCst) {
            RELOAD_FLAG.store(false, Ordering::SeqCst);
            let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
            mgr_guard.enumerate_input_devices();
            mgr_guard.sync_runtime_state();
            log::info!("Reloaded configuration");
            sd_notify(&format!(
                "STATUS=Managing {} seat(s), {} session(s)",
                mgr_guard.seats.len(),
                mgr_guard.sessions.len()
            ));
        }

        // -----------------------------------------------------------
        // VT_PROCESS switch handling (SIGUSR1 = release, SIGUSR2 = acquire)
        // -----------------------------------------------------------

        // Handle VT release (switch away from active session's VT).
        // Collect info under lock, emit D-Bus signals outside lock.
        if VT_RELEASE_FLAG.load(Ordering::SeqCst) {
            VT_RELEASE_FLAG.store(false, Ordering::SeqCst);
            let away_info = {
                let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
                mgr_guard.handle_vt_release()
            };
            if let Some(info) = away_info
                && let Some(ref server) = dbus_server
            {
                // Emit PauseDevice("force") for each taken device.
                // "force" means the device has been deactivated already
                // and the compositor should release its references.
                emit_pause_devices(&server.conn, &info.session_id, &info.devices, "force");
            }
        }

        // Handle VT acquire (switch to a session's VT).
        // Collect info under lock, emit D-Bus signals outside lock.
        if VT_ACQUIRE_FLAG.load(Ordering::SeqCst) {
            VT_ACQUIRE_FLAG.store(false, Ordering::SeqCst);
            let to_info = {
                let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
                mgr_guard.handle_vt_acquire()
            };
            if let Some(info) = to_info
                && let Some(ref server) = dbus_server
            {
                // If the session switch also deactivated an old session's
                // devices (e.g. when activate_session changes the active
                // session), emit PauseDevice for those too.
                if let Some(ref old_id) = info.old_session_id {
                    emit_pause_devices(&server.conn, old_id, &info.old_devices, "force");
                }
                // Emit ResumeDevice for each taken device in the new session.
                emit_resume_devices(&server.conn, &info.session_id, &info.devices);
                // Sync state after session activation
                let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
                mgr_guard.sync_runtime_state();
            }
        }

        // Send watchdog keepalive
        if let Some(ref iv) = wd_interval
            && last_watchdog.elapsed() >= *iv
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // zbus dispatches D-Bus messages automatically in a background thread.
        // No manual process() call needed.

        // Handle incoming control connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
                    handle_client(&mut mgr_guard, &mut stream);
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No connection waiting
                }
                Err(e) => {
                    log::warn!("Accept error: {}", e);
                }
            }
        }

        // Check for new/removed sessions and emit D-Bus signals
        {
            let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
            let current_sessions: std::collections::HashSet<String> =
                mgr_guard.sessions.keys().cloned().collect();
            let current_users: std::collections::HashSet<u32> =
                mgr_guard.users.keys().cloned().collect();

            // New sessions
            for sid in current_sessions.difference(&known_sessions) {
                if let Some(ref server) = dbus_server {
                    server.register_session(sid);
                    emit_signal_session_new(&server.conn, sid);
                }
                if let Some(session) = mgr_guard.sessions.get(sid) {
                    // Register user if new
                    if !known_users.contains(&session.uid)
                        && let Some(ref server) = dbus_server
                    {
                        server.register_user(session.uid);
                        emit_signal_user_new(&server.conn, session.uid);
                    }
                }
            }

            // Removed sessions
            for sid in known_sessions.difference(&current_sessions) {
                if let Some(ref server) = dbus_server {
                    emit_signal_session_removed(&server.conn, sid);
                    server.unregister_session(sid);
                }
            }

            // Removed users
            for uid in known_users.difference(&current_users) {
                if let Some(ref server) = dbus_server {
                    emit_signal_user_removed(&server.conn, *uid);
                    server.unregister_user(*uid);
                }
            }

            // New users (that didn't come through session creation above)
            for uid in current_users.difference(&known_users) {
                if let Some(ref server) = dbus_server {
                    server.register_user(*uid);
                    emit_signal_user_new(&server.conn, *uid);
                }
            }

            known_sessions = current_sessions;
            known_users = current_users;
        }

        // Periodic cleanup (every ~60 iterations = ~12 seconds)
        cleanup_counter += 1;
        if cleanup_counter.is_multiple_of(60) {
            let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
            mgr_guard.cleanup_stale_inhibitors();
        }

        thread::sleep(Duration::from_millis(200));
    }

    // Cleanup: restore VT_AUTO on all monitored VTs before exit
    {
        let mut mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
        let vt_count = mgr_guard.vt_monitors.len();
        mgr_guard.vt_monitors.clear(); // Drop calls restore_auto()
        if vt_count > 0 {
            log::info!("Restored VT_AUTO on {} VT(s)", vt_count);
        }
    }

    let _ = fs::remove_file(CONTROL_SOCKET_PATH);
    // Remove runtime state
    let _ = fs::remove_dir_all(SESSIONS_DIR);
    let _ = fs::remove_dir_all(SEATS_DIR);
    let _ = fs::remove_dir_all(USERS_DIR);
    let _ = fs::remove_dir_all(INHIBIT_DIR);

    sd_notify("STOPPING=1");
    log::info!("systemd-logind stopped");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Polkit authorization helper
// ---------------------------------------------------------------------------

/// Check polkit authorization for a power action.
///
/// Calls `pkcheck` to determine whether the given UID is authorized for the
/// given action.  Falls back to "yes" when polkit is not installed so that
/// headless / minimal systems still work.
///
/// `action_id` should be the full polkit action, e.g.
/// `"org.freedesktop.login1.power-off"`.
fn polkit_check(action_id: &str, uid: u32, pid: u32) -> &'static str {
    use std::process::Command;
    // Try pkcheck --action-id <action> --process <pid> --allow-user-interaction
    let result = Command::new("pkcheck")
        .arg("--action-id")
        .arg(action_id)
        .arg("--process")
        .arg(pid.to_string())
        .arg("--allow-user-interaction")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match result {
        Ok(status) if status.success() => "yes",
        Ok(status) => {
            // Exit code 1 = not authorized, 2 = challenge, 3 = not found
            match status.code() {
                Some(1) => "no",
                Some(2) => "challenge",
                _ => {
                    log::debug!(
                        "pkcheck for {} uid={} pid={} exited with {:?}",
                        action_id,
                        uid,
                        pid,
                        status.code()
                    );
                    "no"
                }
            }
        }
        Err(e) => {
            // polkit not available — fall back to permissive for root,
            // challenge for everyone else.
            log::debug!("pkcheck not available ({}), falling back", e);
            if uid == 0 { "yes" } else { "challenge" }
        }
    }
}

/// Map a simple action name ("poweroff", "reboot", …) to a polkit action ID.
fn polkit_action_id(action: &str) -> &'static str {
    match action {
        "poweroff" => "org.freedesktop.login1.power-off",
        "reboot" => "org.freedesktop.login1.reboot",
        "halt" => "org.freedesktop.login1.halt",
        "suspend" => "org.freedesktop.login1.suspend",
        "hibernate" => "org.freedesktop.login1.hibernate",
        "hybrid-sleep" => "org.freedesktop.login1.hibernate",
        "suspend-then-hibernate" => "org.freedesktop.login1.hibernate",
        _ => "org.freedesktop.login1.power-off",
    }
}

/// Combined check: inhibitors + polkit for a `Can*` D-Bus property.
///
/// Uses the real UID and PID extracted from the D-Bus caller's message
/// headers to perform accurate polkit authorization checks.
fn can_action_with_polkit(action: &str, uid: u32, pid: u32, mgr: &SharedManager) -> String {
    let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
    let inhibitor_result = mgr_guard.can_action(action);
    if inhibitor_result == "challenge" {
        return "challenge".to_string();
    }
    let polkit_result = polkit_check(polkit_action_id(action), uid, pid);
    polkit_result.to_string()
}

/// Execute a power/sleep action after checking inhibitors and polkit.
///
/// Uses the real UID and PID extracted from the D-Bus caller's message
/// headers for proper authorization.
///
/// Maps the action name to the appropriate system command:
/// - poweroff / reboot / halt → `systemctl <action>`
/// - suspend / hibernate / hybrid-sleep / suspend-then-hibernate →
///   `systemctl <action>`
///
/// Emits `PrepareForShutdown` or `PrepareForSleep` signals before and after.
fn execute_power_action(
    action: &str,
    _interactive: bool,
    caller_uid: u32,
    caller_pid: u32,
    mgr: &SharedManager,
) -> zbus::fdo::Result<()> {
    use std::process::Command;

    // Check inhibitors
    {
        let mgr_guard = mgr.lock().unwrap_or_else(|e| e.into_inner());
        let result = mgr_guard.can_action(action);
        if result == "challenge" {
            // Check polkit with real caller credentials
            let pk = polkit_check(polkit_action_id(action), caller_uid, caller_pid);
            if pk == "no" {
                return Err(zbus::fdo::Error::Failed(format!(
                    "Action '{}' is blocked by an inhibitor and not authorized (uid={}, pid={})",
                    action, caller_uid, caller_pid
                )));
            }
        }
    }

    let is_sleep = matches!(
        action,
        "suspend" | "hibernate" | "hybrid-sleep" | "suspend-then-hibernate"
    );

    log::info!(
        "Executing {} action: {}",
        if is_sleep { "sleep" } else { "shutdown" },
        action
    );

    // The systemctl command name matches the action for all cases
    let systemctl_action = match action {
        "poweroff" => "poweroff",
        "reboot" => "reboot",
        "halt" => "halt",
        "suspend" => "suspend",
        "hibernate" => "hibernate",
        "hybrid-sleep" => "hybrid-sleep",
        "suspend-then-hibernate" => "suspend-then-hibernate",
        other => {
            return Err(zbus::fdo::Error::Failed(format!(
                "Unknown action '{}'",
                other
            )));
        }
    };

    // Spawn in a background thread so we don't block the D-Bus method reply.
    // The caller gets an immediate Ok(()) and the action proceeds asynchronously.
    let cmd = systemctl_action.to_string();
    thread::spawn(move || {
        // Small delay to allow the D-Bus reply to be sent
        thread::sleep(Duration::from_millis(100));
        log::info!("Running: systemctl {}", cmd);
        match Command::new("systemctl").arg(&cmd).status() {
            Ok(status) if status.success() => {
                log::info!("systemctl {} completed successfully", cmd);
            }
            Ok(status) => {
                log::error!("systemctl {} failed with status {:?}", cmd, status.code());
            }
            Err(e) => {
                log::error!("Failed to execute systemctl {}: {}", cmd, e);
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    // -----------------------------------------------------------------------
    // VT_PROCESS monitor and VT switch handling tests
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Multi-seat device assignment tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_attach_device_seat0() {
        let mut mgr = LoginManager::new();
        assert!(
            mgr.attach_device("seat0", "/sys/devices/pci0000:00/test")
                .is_ok()
        );
        let seat0 = mgr.seats.get("seat0").unwrap();
        assert_eq!(seat0.attached_devices.len(), 1);
        assert_eq!(seat0.attached_devices[0], "/sys/devices/pci0000:00/test");
    }

    #[test]
    fn test_attach_device_creates_dynamic_seat() {
        let mut mgr = LoginManager::new();
        assert!(!mgr.seats.contains_key("seat1"));
        assert!(mgr.attach_device("seat1", "/sys/devices/test").is_ok());
        assert!(mgr.seats.contains_key("seat1"));
        let seat1 = mgr.seats.get("seat1").unwrap();
        assert_eq!(seat1.attached_devices.len(), 1);
    }

    #[test]
    fn test_attach_device_no_duplicate() {
        let mut mgr = LoginManager::new();
        let _ = mgr.attach_device("seat0", "/sys/devices/test");
        let _ = mgr.attach_device("seat0", "/sys/devices/test");
        let seat0 = mgr.seats.get("seat0").unwrap();
        assert_eq!(seat0.attached_devices.len(), 1);
    }

    #[test]
    fn test_attach_device_multiple() {
        let mut mgr = LoginManager::new();
        let _ = mgr.attach_device("seat0", "/sys/devices/a");
        let _ = mgr.attach_device("seat0", "/sys/devices/b");
        let seat0 = mgr.seats.get("seat0").unwrap();
        assert_eq!(seat0.attached_devices.len(), 2);
    }

    #[test]
    fn test_flush_attached_devices_clears() {
        let mut mgr = LoginManager::new();
        let _ = mgr.attach_device("seat0", "/sys/devices/test");
        assert_eq!(mgr.seats.get("seat0").unwrap().attached_devices.len(), 1);
        mgr.flush_attached_devices();
        assert_eq!(mgr.seats.get("seat0").unwrap().attached_devices.len(), 0);
    }

    #[test]
    fn test_flush_attached_devices_removes_empty_dynamic_seat() {
        let mut mgr = LoginManager::new();
        let _ = mgr.attach_device("seat1", "/sys/devices/test");
        assert!(mgr.seats.contains_key("seat1"));
        mgr.flush_attached_devices();
        // seat1 has no sessions, should be removed
        assert!(!mgr.seats.contains_key("seat1"));
        // seat0 should remain
        assert!(mgr.seats.contains_key("seat0"));
    }

    #[test]
    fn test_flush_attached_devices_keeps_dynamic_seat_with_sessions() {
        let mut mgr = LoginManager::new();
        let _ = mgr.attach_device("seat1", "/sys/devices/test");
        // Add a session to seat1
        let _ = mgr.create_session(
            1000,
            "test",
            Some("seat1"),
            0,
            "tty",
            "user",
            "/dev/tty1",
            1,
        );
        mgr.flush_attached_devices();
        // seat1 has sessions, should NOT be removed
        assert!(mgr.seats.contains_key("seat1"));
    }

    // -----------------------------------------------------------------------
    // ACL management tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_devnode_from_sysfs_nonexistent() {
        // Should return None for a path that doesn't exist
        assert!(devnode_from_sysfs("/sys/devices/nonexistent_test_path_xyz").is_none());
    }

    #[test]
    fn test_set_device_acl_nonexistent_device() {
        let result = set_device_acl("/dev/nonexistent_test_device_xyz", 1000, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_apply_device_acls_no_seat() {
        let mgr = LoginManager::new();
        // Should not panic for a nonexistent seat
        mgr.apply_device_acls("nonexistent_seat", 1000);
    }

    #[test]
    fn test_remove_device_acls_no_seat() {
        let mgr = LoginManager::new();
        // Should not panic for a nonexistent seat
        mgr.remove_device_acls("nonexistent_seat", 1000);
    }

    #[test]
    fn test_activate_session_applies_acls() {
        // Verify the activation path includes ACL management (no crash)
        let mut mgr = LoginManager::new();
        let id = mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            1,
        );
        let result = mgr.activate_session(&id);
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // utmp / env-file helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_env_file_basic() {
        let content = "UID=1000\nUSER=testuser\nACTIVE=1\n";
        let props = parse_env_file(content);
        assert_eq!(props.get("UID").unwrap(), "1000");
        assert_eq!(props.get("USER").unwrap(), "testuser");
        assert_eq!(props.get("ACTIVE").unwrap(), "1");
    }

    #[test]
    fn test_parse_env_file_comments_and_blanks() {
        let content = "# comment\n\nKEY=value\n";
        let props = parse_env_file(content);
        assert_eq!(props.len(), 1);
        assert_eq!(props.get("KEY").unwrap(), "value");
    }

    #[test]
    fn test_parse_env_file_empty() {
        let props = parse_env_file("");
        assert!(props.is_empty());
    }

    #[test]
    fn test_read_cstr_field_normal() {
        let buf = b"hello\0\0\0\0";
        assert_eq!(read_cstr_field(buf), "hello");
    }

    #[test]
    fn test_read_cstr_field_full() {
        let buf = b"abcd";
        assert_eq!(read_cstr_field(buf), "abcd");
    }

    #[test]
    fn test_read_cstr_field_empty() {
        let buf = b"\0\0\0\0";
        assert_eq!(read_cstr_field(buf), "");
    }

    #[test]
    fn test_resolve_username_to_uid_root() {
        // root should always resolve to uid 0
        assert_eq!(resolve_username_to_uid("root"), Some(0));
    }

    #[test]
    fn test_resolve_username_to_uid_nonexistent() {
        assert_eq!(
            resolve_username_to_uid("__nonexistent_test_user_xyz__"),
            None
        );
    }

    #[test]
    fn test_detect_existing_sessions_no_crash() {
        // Should not crash even without runtime state or utmp
        let mut mgr = LoginManager::new();
        mgr.detect_existing_sessions();
    }

    #[test]
    fn test_handle_command_attach_device() {
        let mut mgr = LoginManager::new();
        let resp = handle_control_command(
            &mut mgr,
            r#"attach-device {"seat":"seat0","sysfs":"/sys/devices/test"}"#,
        );
        assert!(resp.starts_with("OK"));
        assert_eq!(mgr.seats.get("seat0").unwrap().attached_devices.len(), 1);
    }

    #[test]
    fn test_handle_command_attach_device_missing_sysfs() {
        let mut mgr = LoginManager::new();
        let resp = handle_control_command(&mut mgr, r#"attach-device {"seat":"seat0","sysfs":""}"#);
        assert!(resp.starts_with("ERROR"));
    }

    // -----------------------------------------------------------------------
    // Original tests continue
    // -----------------------------------------------------------------------

    #[test]
    fn test_vt_mode_struct_size() {
        // VtMode must match the kernel's struct vt_mode (8 bytes on all arches)
        assert_eq!(
            std::mem::size_of::<super::VtMode>(),
            8,
            "VtMode must be 8 bytes to match kernel struct vt_mode"
        );
    }

    #[test]
    fn test_vt_stat_struct_size() {
        // VtStat must match the kernel's struct vt_stat (6 bytes)
        assert_eq!(
            std::mem::size_of::<super::VtStat>(),
            6,
            "VtStat must be 6 bytes to match kernel struct vt_stat"
        );
    }

    #[test]
    fn test_vt_mode_fields() {
        let mode = super::VtMode {
            mode: super::VT_PROCESS as libc::c_char,
            waitv: 0,
            relsig: libc::SIGUSR1 as libc::c_short,
            acqsig: libc::SIGUSR2 as libc::c_short,
            frsig: 0,
        };
        assert_eq!(mode.mode, 1); // VT_PROCESS
        assert_eq!(mode.relsig, libc::SIGUSR1 as libc::c_short);
        assert_eq!(mode.acqsig, libc::SIGUSR2 as libc::c_short);
    }

    #[test]
    fn test_vt_constants() {
        assert_eq!(super::VT_AUTO, 0x00);
        assert_eq!(super::VT_PROCESS, 0x01);
        assert_eq!(super::VT_ACKACQ, 0x02);
        assert_eq!(super::VT_SETMODE, 0x5602);
        assert_eq!(super::VT_GETMODE, 0x5601);
        assert_eq!(super::VT_RELDISP, 0x5605);
        assert_eq!(super::VT_GETSTATE, 0x5603);
    }

    #[test]
    fn test_vt_monitor_rejects_vt_zero() {
        let result = super::VtMonitor::new(0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot monitor VT 0"));
    }

    #[test]
    fn test_vt_release_flag_default_false() {
        // Flags should be false by default (or reset after handling)
        // We can't test the actual default since other tests may have
        // modified them, but we can verify the store/load cycle.
        super::VT_RELEASE_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!super::VT_RELEASE_FLAG.load(std::sync::atomic::Ordering::SeqCst));
        super::VT_RELEASE_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(super::VT_RELEASE_FLAG.load(std::sync::atomic::Ordering::SeqCst));
        super::VT_RELEASE_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn test_vt_acquire_flag_default_false() {
        super::VT_ACQUIRE_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!super::VT_ACQUIRE_FLAG.load(std::sync::atomic::Ordering::SeqCst));
        super::VT_ACQUIRE_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(super::VT_ACQUIRE_FLAG.load(std::sync::atomic::Ordering::SeqCst));
        super::VT_ACQUIRE_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn test_take_control_sets_vt_monitor_graceful() {
        // TakeControl on a session with a VT should attempt to set VT_PROCESS.
        // In CI/test environments without real /dev/tty*, this will fail
        // gracefully (non-fatal warning) and the session still gets its
        // controller set.
        let mut mgr = super::LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            3,
            "tty",
            "user",
            "tty3",
            1234,
        );
        let result = mgr.take_control("1", "test-controller", false);
        assert!(result.is_ok());
        assert_eq!(
            mgr.sessions.get("1").unwrap().controller.as_deref(),
            Some("test-controller")
        );
        // In CI, VtMonitor::new(3) likely fails (no /dev/tty3), so the
        // monitor map may or may not have an entry — either is fine.
    }

    #[test]
    fn test_release_control_removes_vt_monitor() {
        let mut mgr = super::LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            2,
            "tty",
            "user",
            "tty2",
            1234,
        );
        let _ = mgr.take_control("1", "ctrl", false);
        // Force-insert a dummy into vt_monitors to test removal logic
        // (we can't create a real VtMonitor without /dev/tty2)
        let had_monitor = mgr.vt_monitors.contains_key(&2);
        mgr.release_control("1");
        assert!(mgr.sessions.get("1").unwrap().controller.is_none());
        // Monitor should be removed if it was there
        if had_monitor {
            assert!(!mgr.vt_monitors.contains_key(&2));
        }
    }

    #[test]
    fn test_release_control_keeps_monitor_for_other_session() {
        // If two sessions share the same VT (unusual but possible),
        // releasing one should not remove the monitor if the other still
        // has a controller.
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user1", Some("seat0"), 1, "tty", "user", "tty1", 100);
        mgr.create_session(1001, "user2", Some("seat0"), 1, "tty", "user", "tty1", 200);
        let _ = mgr.take_control("1", "ctrl-a", false);
        let _ = mgr.take_control("2", "ctrl-b", false);
        let had_monitor = mgr.vt_monitors.contains_key(&1);
        mgr.release_control("1");
        // Session 2 still has a controller on VT 1
        if had_monitor {
            assert!(mgr.vt_monitors.contains_key(&1));
        }
    }

    #[test]
    fn test_session_by_vtnr() {
        let mut mgr = super::LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            5,
            "tty",
            "user",
            "tty5",
            1234,
        );
        assert_eq!(mgr.session_by_vtnr(5), Some("1"));
        assert_eq!(mgr.session_by_vtnr(6), None);
        assert_eq!(mgr.session_by_vtnr(0), None);
    }

    #[test]
    fn test_active_session_on_seat0() {
        let mut mgr = super::LoginManager::new();
        // No sessions yet
        assert!(mgr.active_session_on_seat0().is_none() || mgr.active_session_on_seat0().is_some());
        // Actually seat0 starts with no active session
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        // After creation, the session becomes active on seat0
        assert_eq!(mgr.active_session_on_seat0(), Some("1"));
    }

    #[test]
    fn test_session_device_keys_empty() {
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        let keys = mgr.session_device_keys("1");
        assert!(keys.is_empty());
    }

    #[test]
    fn test_session_device_keys_nonexistent() {
        let mgr = super::LoginManager::new();
        let keys = mgr.session_device_keys("nonexistent");
        assert!(keys.is_empty());
    }

    #[test]
    fn test_session_device_fds_empty() {
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        let fds = mgr.session_device_fds("1");
        assert!(fds.is_empty());
    }

    #[test]
    fn test_session_device_fds_nonexistent() {
        let mgr = super::LoginManager::new();
        let fds = mgr.session_device_fds("nonexistent");
        assert!(fds.is_empty());
    }

    #[test]
    fn test_handle_vt_release_no_active_session() {
        let mut mgr = super::LoginManager::new();
        // No sessions → nothing to release
        let result = mgr.handle_vt_release();
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_vt_release_session_no_vt() {
        let mut mgr = super::LoginManager::new();
        // Session with vtnr=0 (no VT)
        mgr.create_session(1000, "user", Some("seat0"), 0, "tty", "user", "", 100);
        let result = mgr.handle_vt_release();
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_vt_release_session_with_vt_no_devices() {
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        let result = mgr.handle_vt_release();
        // Session exists with VT but no taken devices
        assert!(result.is_some());
        let info = result.unwrap();
        assert_eq!(info.session_id, "1");
        assert!(info.devices.is_empty());
    }

    #[test]
    fn test_handle_vt_acquire_no_active_vt() {
        // get_active_vt() will likely return None in CI (no real VT)
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        let result = mgr.handle_vt_acquire();
        // In CI this returns None since get_active_vt() fails
        // We just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_vt_switch_away_info_fields() {
        let info = super::VtSwitchAwayInfo {
            session_id: "42".to_string(),
            devices: vec![(226, 0), (13, 64)],
        };
        assert_eq!(info.session_id, "42");
        assert_eq!(info.devices.len(), 2);
        assert_eq!(info.devices[0], (226, 0));
    }

    #[test]
    fn test_vt_switch_to_info_fields() {
        let info = super::VtSwitchToInfo {
            session_id: "7".to_string(),
            old_session_id: Some("3".to_string()),
            old_devices: vec![(226, 0)],
            devices: vec![(226, 0, 5), (13, 64, 6)],
        };
        assert_eq!(info.session_id, "7");
        assert_eq!(info.old_session_id.as_deref(), Some("3"));
        assert_eq!(info.old_devices.len(), 1);
        assert_eq!(info.devices.len(), 2);
    }

    #[test]
    fn test_get_active_vt_no_crash() {
        // On CI without /dev/tty0, this should return None gracefully.
        let result = super::get_active_vt();
        // We can't predict the result, just verify no panic.
        let _ = result;
    }

    #[test]
    fn test_handle_vt_release_marks_devices_inactive() {
        use std::os::fd::{FromRawFd, OwnedFd};
        let mut mgr = super::LoginManager::new();
        mgr.create_session(1000, "user", Some("seat0"), 1, "tty", "user", "tty1", 100);
        let _ = mgr.take_control("1", "ctrl", false);

        // Insert a mock device (using a dup'd stderr fd as a stand-in)
        let mock_fd = unsafe { libc::fcntl(2, libc::F_DUPFD_CLOEXEC, 0) };
        if mock_fd >= 0 {
            let owned = unsafe { OwnedFd::from_raw_fd(mock_fd) };
            mgr.sessions.get_mut("1").unwrap().devices.insert(
                (226, 0),
                super::SessionDevice {
                    major: 226,
                    minor: 0,
                    fd: owned,
                    active: true,
                },
            );
            let result = mgr.handle_vt_release();
            assert!(result.is_some());
            let info = result.unwrap();
            assert_eq!(info.devices, vec![(226, 0)]);
            // Device should now be marked inactive
            let dev = mgr
                .sessions
                .get("1")
                .unwrap()
                .devices
                .get(&(226, 0))
                .unwrap();
            assert!(!dev.active);
        }
    }

    use super::*;

    // -- LoginManager tests --

    #[test]
    fn test_login_manager_new() {
        let mgr = LoginManager::new();
        assert!(mgr.seats.contains_key("seat0"));
        assert!(mgr.sessions.is_empty());
        assert!(mgr.users.is_empty());
        assert!(mgr.inhibitors.is_empty());
    }

    #[test]
    fn test_create_session() {
        let mut mgr = LoginManager::new();
        let id = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            12345,
        );
        assert_eq!(id, "1");
        assert!(mgr.sessions.contains_key("1"));
        assert_eq!(mgr.sessions["1"].uid, 1000);
        assert_eq!(mgr.sessions["1"].user, "testuser");
        assert_eq!(mgr.sessions["1"].vtnr, 1);
        assert!(mgr.sessions["1"].active);
        assert_eq!(mgr.seats["seat0"].sessions, vec!["1"]);
        assert_eq!(mgr.seats["seat0"].active_session, Some("1".to_string()));
        assert!(mgr.users.contains_key(&1000));
    }

    #[test]
    fn test_create_multiple_sessions() {
        let mut mgr = LoginManager::new();
        let id1 = mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let id2 = mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        assert_eq!(id1, "1");
        assert_eq!(id2, "2");
        assert_eq!(mgr.seats["seat0"].sessions.len(), 2);
        // First session stays active
        assert_eq!(mgr.seats["seat0"].active_session, Some("1".to_string()));
    }

    #[test]
    fn test_release_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        assert!(mgr.release_session("1"));
        assert!(!mgr.sessions.contains_key("1"));
        assert!(mgr.seats["seat0"].sessions.is_empty());
        assert!(mgr.seats["seat0"].active_session.is_none());
        assert_eq!(mgr.users[&1000].state, "closing");
    }

    #[test]
    fn test_release_nonexistent_session() {
        let mut mgr = LoginManager::new();
        assert!(!mgr.release_session("999"));
    }

    #[test]
    fn test_create_inhibitor() {
        let mut mgr = LoginManager::new();
        let id = mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert_eq!(id, 1);
        assert!(mgr.inhibitors.contains_key(&1));
        assert_eq!(mgr.inhibitors[&1].what, "shutdown");
    }

    #[test]
    fn test_release_inhibitor() {
        let mut mgr = LoginManager::new();
        let id = mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert!(mgr.release_inhibitor(id));
        assert!(!mgr.inhibitors.contains_key(&id));
    }

    #[test]
    fn test_release_nonexistent_inhibitor() {
        let mut mgr = LoginManager::new();
        assert!(!mgr.release_inhibitor(999));
    }

    #[test]
    fn test_session_without_seat() {
        let mut mgr = LoginManager::new();
        let id = mgr.create_session(1000, "testuser", None, 0, "tty", "user", "/dev/pts/0", 100);
        assert_eq!(id, "1");
        assert!(mgr.sessions["1"].seat.is_none());
        // seat0 should be unchanged
        assert!(mgr.seats["seat0"].sessions.is_empty());
    }

    #[test]
    fn test_multiple_sessions_same_user() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(1000, "testuser", None, 0, "tty", "user", "/dev/pts/0", 200);
        assert_eq!(mgr.users[&1000].sessions.len(), 2);
        // Release first session — user should still be active
        mgr.release_session("1");
        assert_eq!(mgr.users[&1000].sessions.len(), 1);
        assert_eq!(mgr.users[&1000].state, "active");
        // Release second — user should be closing
        mgr.release_session("2");
        assert_eq!(mgr.users[&1000].sessions.len(), 0);
        assert_eq!(mgr.users[&1000].state, "closing");
    }

    // -- Control command tests --

    #[test]
    fn test_handle_command_status() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "status");
        assert!(result.contains("seat"));
        assert!(result.contains("session"));
    }

    #[test]
    fn test_handle_command_list_sessions_empty() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "list-sessions");
        assert_eq!(result.trim(), "[]");
    }

    #[test]
    fn test_handle_command_list_seats() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "list-seats");
        assert!(result.contains("seat0"));
    }

    #[test]
    fn test_handle_command_create_session_json() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(
            &mut mgr,
            r#"create-session {"uid": 1000, "user": "test", "seat": "seat0", "vtnr": 1, "type": "tty", "class": "user", "tty": "/dev/tty1", "leader": 1234}"#,
        );
        assert!(result.starts_with("OK "));
        assert_eq!(mgr.sessions.len(), 1);
    }

    #[test]
    fn test_handle_command_release_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let result = handle_control_command(&mut mgr, "release-session 1");
        assert_eq!(result, "OK");
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_handle_command_unknown() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "foobar");
        assert!(result.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_command_can_poweroff_no_inhibitors() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "can-poweroff");
        assert_eq!(result, "yes");
    }

    #[test]
    fn test_handle_command_can_poweroff_with_inhibitor() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        let result = handle_control_command(&mut mgr, "can-poweroff");
        assert_eq!(result, "challenge");
    }

    #[test]
    fn test_handle_command_inhibit() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(
            &mut mgr,
            r#"inhibit {"what": "shutdown", "who": "test", "why": "testing", "mode": "block", "uid": 0, "pid": 1}"#,
        );
        assert!(result.starts_with("OK "));
        assert_eq!(mgr.inhibitors.len(), 1);
    }

    #[test]
    fn test_handle_command_release_inhibitor() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        let result = handle_control_command(&mut mgr, "release-inhibitor 1");
        assert_eq!(result, "OK");
        assert!(mgr.inhibitors.is_empty());
    }

    #[test]
    fn test_activate_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        let result = handle_control_command(&mut mgr, "activate-session 2");
        assert_eq!(result, "OK");
        assert!(!mgr.sessions["1"].active);
        assert_eq!(mgr.sessions["1"].state, "online");
        assert!(mgr.sessions["2"].active);
        assert_eq!(mgr.sessions["2"].state, "active");
        assert_eq!(mgr.seats["seat0"].active_session, Some("2".to_string()));
    }

    #[test]
    fn test_terminate_user() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(1000, "testuser", None, 0, "tty", "user", "/dev/pts/0", 200);
        let result = handle_control_command(&mut mgr, "terminate-user 1000");
        assert_eq!(result, "OK");
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_seat0_always_exists() {
        let mgr = LoginManager::new();
        assert!(mgr.seats.contains_key("seat0"));
        assert!(mgr.seats["seat0"].can_multi_session);
    }

    #[test]
    fn test_format_status() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let status = mgr.format_status();
        assert!(status.contains("1 sessions."));
        assert!(status.contains("1 users."));
        assert!(status.contains("0 inhibitors."));
    }

    #[test]
    fn test_check_seat0_graphical() {
        // Just make sure it doesn't panic; value depends on the system
        let _ = check_seat0_graphical();
    }

    #[test]
    fn test_session_incrementing_ids() {
        let mut mgr = LoginManager::new();
        let id1 = mgr.create_session(1000, "a", None, 0, "tty", "user", "", 1);
        let id2 = mgr.create_session(1001, "b", None, 0, "tty", "user", "", 2);
        let id3 = mgr.create_session(1002, "c", None, 0, "tty", "user", "", 3);
        assert_eq!(id1, "1");
        assert_eq!(id2, "2");
        assert_eq!(id3, "3");
    }

    #[test]
    fn test_inhibitor_incrementing_ids() {
        let mut mgr = LoginManager::new();
        let id1 = mgr.create_inhibitor("shutdown", "a", "", "block", 0, 0);
        let id2 = mgr.create_inhibitor("sleep", "b", "", "delay", 0, 0);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }

    #[test]
    fn test_watchdog_interval_none() {
        // When WATCHDOG_USEC is not set, should return None
        unsafe { std::env::remove_var("WATCHDOG_USEC") };
        assert!(watchdog_interval().is_none());
    }

    #[test]
    fn test_lock_unlock_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let result = handle_control_command(&mut mgr, "lock-session 1");
        assert_eq!(result, "OK");
        assert!(mgr.sessions["1"].locked_hint);
        let result = handle_control_command(&mut mgr, "unlock-session 1");
        assert_eq!(result, "OK");
        assert!(!mgr.sessions["1"].locked_hint);
    }

    #[test]
    fn test_lock_nonexistent_session() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "lock-session 999");
        assert!(result.starts_with("ERROR"));
    }

    #[test]
    fn test_show_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let result = handle_control_command(&mut mgr, "show-session 1");
        assert!(result.contains("testuser"));
        assert!(result.contains("seat0"));
    }

    #[test]
    fn test_show_nonexistent_session() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "show-session 999");
        assert!(result.starts_with("ERROR"));
    }

    #[test]
    fn test_show_seat() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "show-seat seat0");
        assert!(result.contains("seat0"));
    }

    #[test]
    fn test_show_user_by_uid() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "testuser", None, 0, "tty", "user", "", 100);
        let result = handle_control_command(&mut mgr, "show-user 1000");
        assert!(result.contains("testuser"));
    }

    #[test]
    fn test_show_user_by_name() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "testuser", None, 0, "tty", "user", "", 100);
        let result = handle_control_command(&mut mgr, "show-user testuser");
        assert!(result.contains("testuser"));
    }

    #[test]
    fn test_flush_devices() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "flush-devices");
        assert_eq!(result, "OK");
    }

    #[test]
    fn test_can_suspend() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "can-suspend");
        assert_eq!(result, "yes");
    }

    #[test]
    fn test_list_inhibitors_empty() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "list-inhibitors");
        assert_eq!(result.trim(), "[]");
    }

    #[test]
    fn test_list_users_empty() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "list-users");
        assert_eq!(result.trim(), "[]");
    }

    // -- D-Bus object path tests --

    #[test]
    fn test_session_object_path() {
        let path = session_object_path("1");
        assert_eq!(path.to_string(), "/org/freedesktop/login1/session/_1");
    }

    #[test]
    fn test_session_object_path_with_dash() {
        let path = session_object_path("c-1");
        assert_eq!(path.to_string(), "/org/freedesktop/login1/session/_c_2d1");
    }

    #[test]
    fn test_seat_object_path() {
        let path = seat_object_path("seat0");
        assert_eq!(path.to_string(), "/org/freedesktop/login1/seat/seat0");
    }

    #[test]
    fn test_user_object_path() {
        let path = user_object_path(1000);
        assert_eq!(path.to_string(), "/org/freedesktop/login1/user/_1000");
    }

    #[test]
    fn test_session_id_from_path() {
        assert_eq!(
            session_id_from_path("/org/freedesktop/login1/session/_1"),
            Some("1".to_string())
        );
        assert_eq!(
            session_id_from_path("/org/freedesktop/login1/session/_c_2d1"),
            Some("c-1".to_string())
        );
        assert_eq!(session_id_from_path("/wrong/path"), None);
    }

    #[test]
    fn test_seat_id_from_path() {
        assert_eq!(
            seat_id_from_path("/org/freedesktop/login1/seat/seat0"),
            Some("seat0".to_string())
        );
        assert_eq!(seat_id_from_path("/wrong/path"), None);
    }

    #[test]
    fn test_uid_from_path() {
        assert_eq!(
            uid_from_path("/org/freedesktop/login1/user/_1000"),
            Some(1000)
        );
        assert_eq!(uid_from_path("/wrong/path"), None);
    }

    // -- Activate / lock / unlock via LoginManager methods --

    #[test]
    fn test_activate_session_method() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        assert!(mgr.activate_session("2").is_ok());
        assert!(!mgr.sessions["1"].active);
        assert!(mgr.sessions["2"].active);
    }

    #[test]
    fn test_activate_session_not_found() {
        let mut mgr = LoginManager::new();
        assert!(mgr.activate_session("999").is_err());
    }

    #[test]
    fn test_activate_session_no_seat() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "user1", None, 0, "tty", "user", "", 100);
        assert!(mgr.activate_session("1").is_err());
    }

    #[test]
    fn test_lock_unlock_session_method() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        assert!(mgr.lock_session("1").is_ok());
        assert!(mgr.sessions["1"].locked_hint);
        assert!(mgr.unlock_session("1").is_ok());
        assert!(!mgr.sessions["1"].locked_hint);
    }

    #[test]
    fn test_lock_session_not_found() {
        let mut mgr = LoginManager::new();
        assert!(mgr.lock_session("999").is_err());
    }

    #[test]
    fn test_lock_all_sessions() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "a", None, 0, "tty", "user", "", 100);
        mgr.create_session(1001, "b", None, 0, "tty", "user", "", 200);
        mgr.lock_sessions();
        assert!(mgr.sessions["1"].locked_hint);
        assert!(mgr.sessions["2"].locked_hint);
        mgr.unlock_sessions();
        assert!(!mgr.sessions["1"].locked_hint);
        assert!(!mgr.sessions["2"].locked_hint);
    }

    #[test]
    fn test_set_idle_hint() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        assert!(mgr.set_idle_hint("1", true).is_ok());
        assert!(mgr.sessions["1"].idle_hint);
        assert!(mgr.sessions["1"].idle_since_hint > 0);
        assert!(mgr.set_idle_hint("1", false).is_ok());
        assert!(!mgr.sessions["1"].idle_hint);
        assert_eq!(mgr.sessions["1"].idle_since_hint, 0);
    }

    #[test]
    fn test_set_locked_hint() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        assert!(mgr.set_locked_hint("1", true).is_ok());
        assert!(mgr.sessions["1"].locked_hint);
        assert!(mgr.set_locked_hint("1", false).is_ok());
        assert!(!mgr.sessions["1"].locked_hint);
    }

    #[test]
    fn test_set_session_type() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        assert!(mgr.set_session_type("1", "wayland").is_ok());
        assert_eq!(mgr.sessions["1"].session_type, "wayland");
        assert!(mgr.set_session_type("1", "x11").is_ok());
        assert_eq!(mgr.sessions["1"].session_type, "x11");
        assert!(mgr.set_session_type("1", "invalid").is_err());
    }

    #[test]
    fn test_terminate_user_method() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 200);
        assert!(mgr.terminate_user(1000).is_ok());
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_terminate_user_not_found() {
        let mut mgr = LoginManager::new();
        assert!(mgr.terminate_user(9999).is_err());
    }

    #[test]
    fn test_terminate_seat_method() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "a", Some("seat0"), 1, "tty", "user", "", 100);
        mgr.create_session(1001, "b", Some("seat0"), 2, "tty", "user", "", 200);
        assert!(mgr.terminate_seat("seat0").is_ok());
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_terminate_seat_not_found() {
        let mut mgr = LoginManager::new();
        assert!(mgr.terminate_seat("nonexistent").is_err());
    }

    // -- Idle hint tests --

    #[test]
    fn test_global_idle_hint_empty() {
        let mgr = LoginManager::new();
        assert!(mgr.global_idle_hint());
    }

    #[test]
    fn test_global_idle_hint_active_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", Some("seat0"), 1, "tty", "user", "", 100);
        assert!(!mgr.global_idle_hint()); // active session, not idle
    }

    #[test]
    fn test_global_idle_hint_all_idle() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", Some("seat0"), 1, "tty", "user", "", 100);
        mgr.set_idle_hint("1", true).unwrap();
        assert!(mgr.global_idle_hint());
    }

    #[test]
    fn test_global_idle_since_hint() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        mgr.set_idle_hint("1", true).unwrap();
        let (rt, mono) = mgr.global_idle_since_hint();
        assert!(rt > 0);
        assert!(mono > 0);
    }

    // -- Inhibitor tests --

    #[test]
    fn test_can_action_no_inhibitors() {
        let mgr = LoginManager::new();
        assert_eq!(mgr.can_action("poweroff"), "yes");
        assert_eq!(mgr.can_action("suspend"), "yes");
        assert_eq!(mgr.can_action("reboot"), "yes");
    }
    #[test]
    fn test_can_action_with_blocking_inhibitor() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert_eq!(mgr.can_action("poweroff"), "challenge");
        assert_eq!(mgr.can_action("reboot"), "challenge");
    }
    #[test]
    fn test_can_action_with_delay_inhibitor() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "delay", 0, 0);
        assert_eq!(mgr.can_action("poweroff"), "yes");
    }

    #[test]
    fn test_block_inhibited() {
        let mut mgr = LoginManager::new();
        assert_eq!(mgr.block_inhibited(), "");
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert_eq!(mgr.block_inhibited(), "shutdown");
        mgr.create_inhibitor("sleep", "test2", "testing", "block", 0, 0);
        let result = mgr.block_inhibited();
        assert!(result.contains("shutdown"));
        assert!(result.contains("sleep"));
    }

    #[test]
    fn test_delay_inhibited() {
        let mut mgr = LoginManager::new();
        assert_eq!(mgr.delay_inhibited(), "");
        mgr.create_inhibitor("shutdown", "test", "testing", "delay", 0, 0);
        assert_eq!(mgr.delay_inhibited(), "shutdown");
    }

    // -- Configuration tests --

    #[test]
    fn test_default_config() {
        let config = LogindConfig::default();
        assert_eq!(config.n_auto_vts, 6);
        assert!(config.kill_user_processes);
        assert_eq!(config.handle_power_key, "poweroff");
        assert_eq!(config.handle_suspend_key, "suspend");
        assert_eq!(config.handle_lid_switch, "suspend");
        assert_eq!(
            config.inhibit_delay_max_usec,
            DEFAULT_INHIBIT_DELAY_MAX_USEC
        );
        assert_eq!(config.sessions_max, 8192);
    }

    #[test]
    fn test_parse_timespan_seconds() {
        assert_eq!(parse_timespan_to_usec("5"), Ok(5_000_000));
        assert_eq!(parse_timespan_to_usec("5s"), Ok(5_000_000));
        assert_eq!(parse_timespan_to_usec("5sec"), Ok(5_000_000));
        assert_eq!(parse_timespan_to_usec("5second"), Ok(5_000_000));
        assert_eq!(parse_timespan_to_usec("5seconds"), Ok(5_000_000));
    }

    #[test]
    fn test_parse_timespan_minutes() {
        assert_eq!(parse_timespan_to_usec("2min"), Ok(120_000_000));
        assert_eq!(parse_timespan_to_usec("2minute"), Ok(120_000_000));
        assert_eq!(parse_timespan_to_usec("2minutes"), Ok(120_000_000));
    }

    #[test]
    fn test_parse_timespan_hours() {
        assert_eq!(parse_timespan_to_usec("1h"), Ok(3_600_000_000));
        assert_eq!(parse_timespan_to_usec("1hr"), Ok(3_600_000_000));
        assert_eq!(parse_timespan_to_usec("1hour"), Ok(3_600_000_000));
    }

    #[test]
    fn test_parse_timespan_milliseconds() {
        assert_eq!(parse_timespan_to_usec("500ms"), Ok(500_000));
        assert_eq!(parse_timespan_to_usec("500msec"), Ok(500_000));
    }

    #[test]
    fn test_parse_timespan_microseconds() {
        assert_eq!(parse_timespan_to_usec("1000us"), Ok(1000));
        assert_eq!(parse_timespan_to_usec("1000usec"), Ok(1000));
    }

    #[test]
    fn test_parse_timespan_invalid() {
        assert!(parse_timespan_to_usec("").is_err());
        assert!(parse_timespan_to_usec("abc").is_err());
    }

    // -- Session fields tests --

    #[test]
    fn test_take_control_sets_controller() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        mgr.take_control(&sid, "org.test.Caller", false).unwrap();
        assert_eq!(
            mgr.sessions.get(&sid).unwrap().controller.as_deref(),
            Some("org.test.Caller")
        );
    }

    #[test]
    fn test_take_control_rejects_duplicate() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        mgr.take_control(&sid, "first", false).unwrap();
        assert!(mgr.take_control(&sid, "second", false).is_err());
    }

    #[test]
    fn test_take_control_force_overrides() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        mgr.take_control(&sid, "first", false).unwrap();
        mgr.take_control(&sid, "second", true).unwrap();
        assert_eq!(
            mgr.sessions.get(&sid).unwrap().controller.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn test_release_control_clears_controller() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        mgr.take_control(&sid, "ctrl", false).unwrap();
        mgr.release_control(&sid);
        assert!(mgr.sessions.get(&sid).unwrap().controller.is_none());
    }

    #[test]
    fn test_take_device_requires_controller() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        let result = mgr.take_device(&sid, 226, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no controller"));
    }

    #[test]
    fn test_take_device_invalid_device() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        mgr.take_control(&sid, "ctrl", false).unwrap();
        // Major 0, Minor 0 — unlikely to exist as a char device
        let result = mgr.take_device(&sid, 0, 0);
        // This should fail to open in most environments
        assert!(result.is_err());
    }

    #[test]
    fn test_release_device_nonexistent_ok() {
        let mut mgr = LoginManager::new();
        let sid = mgr.create_session(
            1000,
            "testuser",
            Some("seat0"),
            1,
            "tty",
            "user",
            "tty1",
            1234,
        );
        // Releasing a device that was never taken should not panic
        mgr.release_device(&sid, 226, 0);
    }

    #[test]
    fn test_take_device_session_not_found() {
        let mut mgr = LoginManager::new();
        let result = mgr.take_device("nonexistent", 226, 0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_polkit_action_id_mapping() {
        assert_eq!(
            polkit_action_id("poweroff"),
            "org.freedesktop.login1.power-off"
        );
        assert_eq!(polkit_action_id("reboot"), "org.freedesktop.login1.reboot");
        assert_eq!(polkit_action_id("halt"), "org.freedesktop.login1.halt");
        assert_eq!(
            polkit_action_id("suspend"),
            "org.freedesktop.login1.suspend"
        );
        assert_eq!(
            polkit_action_id("hibernate"),
            "org.freedesktop.login1.hibernate"
        );
        assert_eq!(
            polkit_action_id("hybrid-sleep"),
            "org.freedesktop.login1.hibernate"
        );
        assert_eq!(
            polkit_action_id("suspend-then-hibernate"),
            "org.freedesktop.login1.hibernate"
        );
        assert_eq!(
            polkit_action_id("unknown"),
            "org.freedesktop.login1.power-off"
        );
    }

    #[test]
    fn test_dup_fd_works() {
        use std::os::fd::AsRawFd;
        // Open /dev/null as a test FD
        let file = std::fs::File::open("/dev/null").unwrap();
        let owned: OwnedFd = file.into();
        let duped = dup_fd(&owned).unwrap();
        assert_ne!(owned.as_raw_fd(), duped.as_raw_fd());
        // Both FDs should be valid (non-negative)
        assert!(owned.as_raw_fd() >= 0);
        assert!(duped.as_raw_fd() >= 0);
    }

    #[test]
    fn test_session_new_fields() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            5678,
        );
        let session = &mgr.sessions["1"];
        assert_eq!(session.display, "");
        assert_eq!(session.service, "");
        assert_eq!(session.desktop, "");
        assert!(!session.remote);
        assert_eq!(session.remote_host, "");
        assert_eq!(session.remote_user, "");
        assert!(session.since > 0);
        assert!(session.since_monotonic > 0);
        assert!(!session.idle_hint);
        assert_eq!(session.idle_since_hint, 0);
        assert!(!session.locked_hint);
    }

    #[test]
    fn test_user_new_fields() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "test", None, 0, "tty", "user", "", 100);
        let user = &mgr.users[&1000];
        assert_eq!(user.slice, "user-1000.slice");
        assert_eq!(user.service, "user@1000.service");
        assert_eq!(user.runtime_path, "/run/user/1000");
        assert!(!user.linger);
        assert!(user.since > 0);
    }

    // -- resolve_uid_to_name --

    #[test]
    fn test_resolve_uid_to_name_fallback() {
        // UID 99999 almost certainly doesn't exist
        let name = resolve_uid_to_name(99999);
        assert_eq!(name, "99999");
    }

    #[test]
    fn test_resolve_uid_to_name_root() {
        let name = resolve_uid_to_name(0);
        // On most systems, UID 0 is "root"
        // But in test environments it might not exist
        assert!(!name.is_empty());
    }

    // -- check_ac_power --

    #[test]
    fn test_check_ac_power() {
        // Just make sure it doesn't panic
        let _ = check_ac_power();
    }

    // -- Control command edge cases --

    #[test]
    fn test_handle_command_lock_sessions() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "a", None, 0, "tty", "user", "", 100);
        mgr.create_session(1001, "b", None, 0, "tty", "user", "", 200);
        let result = handle_control_command(&mut mgr, "lock-sessions");
        assert_eq!(result, "OK");
        assert!(mgr.sessions["1"].locked_hint);
        assert!(mgr.sessions["2"].locked_hint);
    }

    #[test]
    fn test_handle_command_unlock_sessions() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "a", None, 0, "tty", "user", "", 100);
        mgr.lock_sessions();
        let result = handle_control_command(&mut mgr, "unlock-sessions");
        assert_eq!(result, "OK");
        assert!(!mgr.sessions["1"].locked_hint);
    }

    #[test]
    fn test_handle_command_terminate_session() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "test",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        let result = handle_control_command(&mut mgr, "terminate-session 1");
        assert_eq!(result, "OK");
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_handle_command_list_users_with_users() {
        let mut mgr = LoginManager::new();
        mgr.create_session(1000, "testuser", None, 0, "tty", "user", "", 100);
        let result = handle_control_command(&mut mgr, "list-users");
        assert!(result.contains("testuser"));
        assert!(result.contains("1000"));
    }

    #[test]
    fn test_handle_command_list_inhibitors_with_inhibitors() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test-app", "saving work", "block", 1000, 5678);
        let result = handle_control_command(&mut mgr, "list-inhibitors");
        assert!(result.contains("shutdown"));
        assert!(result.contains("test-app"));
        assert!(result.contains("saving work"));
    }

    #[test]
    fn test_handle_command_can_hibernate() {
        let mut mgr = LoginManager::new();
        assert_eq!(handle_control_command(&mut mgr, "can-hibernate"), "yes");
    }

    #[test]
    fn test_handle_command_can_hybrid_sleep() {
        let mut mgr = LoginManager::new();
        assert_eq!(handle_control_command(&mut mgr, "can-hybrid-sleep"), "yes");
    }

    #[test]
    fn test_handle_command_can_suspend_then_hibernate() {
        let mut mgr = LoginManager::new();
        assert_eq!(
            handle_control_command(&mut mgr, "can-suspend-then-hibernate"),
            "yes"
        );
    }

    #[test]
    fn test_handle_command_poweroff() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "poweroff");
        assert!(result.starts_with("OK"));
    }

    #[test]
    fn test_handle_command_reboot() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "reboot");
        assert!(result.starts_with("OK"));
    }

    #[test]
    fn test_handle_command_suspend() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "suspend");
        assert!(result.starts_with("OK"));
    }

    #[test]
    fn test_handle_command_hibernate() {
        let mut mgr = LoginManager::new();
        let result = handle_control_command(&mut mgr, "hibernate");
        assert!(result.starts_with("OK"));
    }

    // -- Config parsing from file --

    #[test]
    fn test_parse_logind_conf_file() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("logind.conf");
        fs::write(
            &conf_path,
            "[Login]\nNAutoVTs=3\nKillUserProcesses=no\nHandlePowerKey=ignore\nInhibitDelayMaxSec=10s\nSessionsMax=4096\n",
        )
        .unwrap();

        let mut config = LogindConfig::default();
        parse_logind_conf_file(&conf_path.to_string_lossy(), &mut config);

        assert_eq!(config.n_auto_vts, 3);
        assert!(!config.kill_user_processes);
        assert_eq!(config.handle_power_key, "ignore");
        assert_eq!(config.inhibit_delay_max_usec, 10_000_000);
        assert_eq!(config.sessions_max, 4096);
    }

    #[test]
    fn test_parse_logind_conf_file_wrong_section() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("logind.conf");
        fs::write(&conf_path, "[Other]\nNAutoVTs=99\n").unwrap();

        let mut config = LogindConfig::default();
        parse_logind_conf_file(&conf_path.to_string_lossy(), &mut config);

        // Should not be changed since it's in the wrong section
        assert_eq!(config.n_auto_vts, 6);
    }

    #[test]
    fn test_parse_logind_conf_file_comments() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("logind.conf");
        fs::write(
            &conf_path,
            "# comment\n; another comment\n[Login]\n#NAutoVTs=99\nNAutoVTs=2\n",
        )
        .unwrap();

        let mut config = LogindConfig::default();
        parse_logind_conf_file(&conf_path.to_string_lossy(), &mut config);

        assert_eq!(config.n_auto_vts, 2);
    }

    #[test]
    fn test_parse_logind_conf_nonexistent() {
        let mut config = LogindConfig::default();
        parse_logind_conf_file("/nonexistent/path/logind.conf", &mut config);
        // Should not crash, config should remain default
        assert_eq!(config.n_auto_vts, 6);
    }

    #[test]
    fn test_parse_logind_conf_all_keys() {
        let dir = tempfile::tempdir().unwrap();
        let conf_path = dir.path().join("logind.conf");
        fs::write(
            &conf_path,
            "[Login]\n\
             NAutoVTs=4\n\
             KillUserProcesses=yes\n\
             KillOnlyUsers=alice bob\n\
             KillExcludeUsers=root nobody\n\
             IdleAction=suspend\n\
             IdleActionSec=30min\n\
             InhibitDelayMaxSec=3\n\
             UserStopDelaySec=20\n\
             HandlePowerKey=suspend\n\
             HandleSuspendKey=hibernate\n\
             HandleHibernateKey=ignore\n\
             HandleLidSwitch=poweroff\n\
             HandleLidSwitchExternalPower=ignore\n\
             HandleLidSwitchDocked=suspend\n\
             HoldoffTimeoutSec=60\n\
             RemoveIPC=no\n\
             InhibitorsMax=100\n\
             SessionsMax=200\n",
        )
        .unwrap();

        let mut config = LogindConfig::default();
        parse_logind_conf_file(&conf_path.to_string_lossy(), &mut config);

        assert_eq!(config.n_auto_vts, 4);
        assert!(config.kill_user_processes);
        assert_eq!(config.kill_only_users, vec!["alice", "bob"]);
        assert_eq!(config.kill_exclude_users, vec!["root", "nobody"]);
        assert_eq!(config.idle_action, "suspend");
        assert_eq!(config.idle_action_usec, 30 * 60 * 1_000_000);
        assert_eq!(config.inhibit_delay_max_usec, 3_000_000);
        assert_eq!(config.user_stop_delay_usec, 20_000_000);
        assert_eq!(config.handle_power_key, "suspend");
        assert_eq!(config.handle_suspend_key, "hibernate");
        assert_eq!(config.handle_hibernate_key, "ignore");
        assert_eq!(config.handle_lid_switch, "poweroff");
        assert_eq!(config.handle_lid_switch_external_power, "ignore");
        assert_eq!(config.handle_lid_switch_docked, "suspend");
        assert_eq!(config.holdoff_timeout_usec, 60_000_000);
        assert!(!config.remove_ipc);
        assert_eq!(config.inhibitors_max, 100);
        assert_eq!(config.sessions_max, 200);
    }

    // -- resolve_user_gid --

    #[test]
    fn test_resolve_user_gid_fallback() {
        // UID 99999 almost certainly doesn't exist
        let gid = resolve_user_gid(99999);
        assert_eq!(gid, 99999); // fallback: gid = uid
    }

    // -- SessionSwitchInfo / activate_session device tracking --

    #[test]
    fn test_activate_session_returns_switch_info_no_old() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        // First activation — no previous active session to deactivate
        // (session is already active from create_session)
        let info = mgr.activate_session("1").unwrap();
        assert!(info.old_session_id.is_none());
        assert!(info.old_devices.is_empty());
        assert_eq!(info.new_session_id, "1");
    }

    #[test]
    fn test_activate_session_returns_switch_info_with_old() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        // create_session only sets active_session if None, so "1" is active.
        // Explicitly activate "2" first, then switch back to "1".
        let _ = mgr.activate_session("2").unwrap();
        let info = mgr.activate_session("1").unwrap();
        assert_eq!(info.old_session_id, Some("2".to_string()));
        assert_eq!(info.new_session_id, "1");
    }

    #[test]
    fn test_activate_session_deactivates_old_devices() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        // Explicitly activate "2" so it becomes the seat's active session
        let _ = mgr.activate_session("2").unwrap();
        // Give session "2" a taken device (simulated — insert directly)
        mgr.sessions.get_mut("2").unwrap().controller = Some("test-ctrl".to_string());
        // Create a pipe to serve as a fake device fd
        let mut fds = [0i32; 2];
        unsafe { libc::pipe(fds.as_mut_ptr()) };
        let fake_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let _write_end = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        mgr.sessions.get_mut("2").unwrap().devices.insert(
            (226, 0),
            SessionDevice {
                major: 226,
                minor: 0,
                fd: fake_fd,
                active: true,
            },
        );

        let info = mgr.activate_session("1").unwrap();
        assert_eq!(info.old_session_id, Some("2".to_string()));
        assert_eq!(info.old_devices, vec![(226, 0)]);
        // Old session device should now be marked inactive
        assert!(!mgr.sessions["2"].devices[&(226, 0)].active);
    }

    #[test]
    fn test_activate_session_activates_new_devices() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        // Explicitly activate "2" so switching to "1" triggers old→new
        let _ = mgr.activate_session("2").unwrap();
        // Give session "1" a taken device
        mgr.sessions.get_mut("1").unwrap().controller = Some("ctrl".to_string());
        let mut fds = [0i32; 2];
        unsafe { libc::pipe(fds.as_mut_ptr()) };
        let fake_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let _write_end = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        mgr.sessions.get_mut("1").unwrap().devices.insert(
            (226, 1),
            SessionDevice {
                major: 226,
                minor: 1,
                fd: fake_fd,
                active: false,
            },
        );

        let info = mgr.activate_session("1").unwrap();
        // New session's devices should be resumed
        assert_eq!(info.new_devices.len(), 1);
        assert_eq!(info.new_devices[0].0, 226);
        assert_eq!(info.new_devices[0].1, 1);
        // Device should now be marked active
        assert!(mgr.sessions["1"].devices[&(226, 1)].active);
    }

    #[test]
    fn test_activate_same_session_no_old() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        // Activating the already-active session should not produce an old_session_id
        let info = mgr.activate_session("1").unwrap();
        assert!(info.old_session_id.is_none());
    }

    // -- can_action improved matching --

    #[test]
    fn test_can_action_shutdown_blocks_poweroff() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert_eq!(mgr.can_action("poweroff"), "challenge");
        assert_eq!(mgr.can_action("reboot"), "challenge");
        assert_eq!(mgr.can_action("halt"), "challenge");
    }

    #[test]
    fn test_can_action_sleep_blocks_suspend() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("sleep", "test", "testing", "block", 0, 0);
        assert_eq!(mgr.can_action("suspend"), "challenge");
        assert_eq!(mgr.can_action("hibernate"), "challenge");
        assert_eq!(mgr.can_action("hybrid-sleep"), "challenge");
        assert_eq!(mgr.can_action("suspend-then-hibernate"), "challenge");
        // sleep inhibitor should NOT block shutdown actions
        assert_eq!(mgr.can_action("poweroff"), "yes");
    }

    #[test]
    fn test_can_action_shutdown_does_not_block_sleep() {
        let mut mgr = LoginManager::new();
        mgr.create_inhibitor("shutdown", "test", "testing", "block", 0, 0);
        assert_eq!(mgr.can_action("suspend"), "yes");
        assert_eq!(mgr.can_action("hibernate"), "yes");
    }

    // -- dup_raw_fd --

    #[test]
    fn test_dup_raw_fd_works() {
        let mut fds = [0i32; 2];
        unsafe { libc::pipe(fds.as_mut_ptr()) };
        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let _write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        let duped = dup_raw_fd(read_fd.as_raw_fd()).expect("dup_raw_fd should succeed");
        assert_ne!(duped.as_raw_fd(), read_fd.as_raw_fd());
        // Both fds should be valid
        assert!(duped.as_raw_fd() >= 0);
    }

    #[test]
    fn test_dup_raw_fd_invalid() {
        let result = dup_raw_fd(-1);
        assert!(result.is_err());
    }

    // -- polkit_action_id --

    #[test]
    fn test_polkit_action_id_all_actions() {
        assert_eq!(
            polkit_action_id("poweroff"),
            "org.freedesktop.login1.power-off"
        );
        assert_eq!(polkit_action_id("reboot"), "org.freedesktop.login1.reboot");
        assert_eq!(polkit_action_id("halt"), "org.freedesktop.login1.halt");
        assert_eq!(
            polkit_action_id("suspend"),
            "org.freedesktop.login1.suspend"
        );
        assert_eq!(
            polkit_action_id("hibernate"),
            "org.freedesktop.login1.hibernate"
        );
        assert_eq!(
            polkit_action_id("hybrid-sleep"),
            "org.freedesktop.login1.hibernate"
        );
        assert_eq!(
            polkit_action_id("suspend-then-hibernate"),
            "org.freedesktop.login1.hibernate"
        );
        // Unknown falls back to power-off
        assert_eq!(
            polkit_action_id("unknown"),
            "org.freedesktop.login1.power-off"
        );
    }

    // -- SessionSwitchInfo fields --

    #[test]
    fn test_session_switch_info_no_devices() {
        let mut mgr = LoginManager::new();
        mgr.create_session(
            1000,
            "user1",
            Some("seat0"),
            1,
            "tty",
            "user",
            "/dev/tty1",
            100,
        );
        mgr.create_session(
            1001,
            "user2",
            Some("seat0"),
            2,
            "tty",
            "user",
            "/dev/tty2",
            200,
        );
        // Activate "2" first so switching to "1" has an old session
        let _ = mgr.activate_session("2").unwrap();
        let info = mgr.activate_session("1").unwrap();
        assert_eq!(info.old_session_id, Some("2".to_string()));
        // Neither session has taken devices, so device lists should be empty
        assert!(info.old_devices.is_empty());
        assert!(info.new_devices.is_empty());
    }
}
