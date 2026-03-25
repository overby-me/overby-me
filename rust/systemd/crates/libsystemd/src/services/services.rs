use log::trace;

use super::start_service::start_service;
use crate::lock_ext::{MutexExt, RwLockExt};
use crate::runtime_info::{PidEntry, RuntimeInfo};
use crate::units::{
    ActivationSource, Commandline, CommandlinePrefix, KillMode, NotifyKind, ServiceConfig,
    ServiceType, Timeout, UnitId, UnitStatus,
};

/// Check if a unit is masked (symlink to /dev/null) on disk.
fn is_unit_masked(name: &str) -> bool {
    let runtime = std::path::Path::new("/run/systemd/system").join(name);
    if let Ok(target) = std::fs::read_link(&runtime)
        && target == std::path::Path::new("/dev/null")
    {
        return true;
    }
    let persistent = std::path::Path::new("/etc/systemd/system").join(name);
    if let Ok(target) = std::fs::read_link(&persistent)
        && target == std::path::Path::new("/dev/null")
    {
        return true;
    }
    false
}

/// Returns the effective `NotifyAccess` given an optional runtime override and
/// the unit file's configured value.
///
/// This is the core logic extracted for easy testing without needing a full
/// `ServiceConfig`.
pub fn effective_notify_access_from_parts(
    runtime_override: Option<NotifyKind>,
    config_value: NotifyKind,
) -> NotifyKind {
    runtime_override.unwrap_or(config_value)
}

/// Returns the effective `NotifyAccess` for a service, considering any runtime
/// override sent via `NOTIFYACCESS=` in an sd_notify message.
///
/// The runtime override (stored in `Service::notify_access_override`) takes
/// precedence over the unit file setting (`ServiceConfig::notifyaccess`).
pub fn effective_notify_access(srvc: &Service, conf: &ServiceConfig) -> NotifyKind {
    effective_notify_access_from_parts(srvc.notify_access_override, conf.notifyaccess)
}

use std::fmt::Write as _;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::BorrowedFd;
use std::os::unix::io::IntoRawFd;
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixDatagram;
use std::process::{Command, Stdio};

/// This looks like `std::process::Stdio` but it can be some more stuff like journal or kmsg so I explicitly
/// made a new enum here
#[derive(Debug)]
pub enum StdIo {
    File(std::fs::File),
    Piped(RawFd, RawFd),

    /// just like the regular file but will always point to /dev/null
    Null(std::fs::File),
}

impl StdIo {
    #[must_use]
    pub fn write_fd(&self) -> RawFd {
        match self {
            Self::File(f) | Self::Null(f) => f.as_raw_fd(),
            Self::Piped(_r, w) => *w,
        }
    }
    #[must_use]
    pub fn read_fd(&self) -> RawFd {
        match self {
            Self::File(f) | Self::Null(f) => f.as_raw_fd(),
            Self::Piped(r, _w) => *r,
        }
    }
}

#[derive(Debug)]
pub struct Service {
    pub pid: Option<nix::unistd::Pid>,
    /// MAINPID= from sd_notify — the actual daemon PID after a forking service
    /// double-forks. When set, this overrides `pid` for watchdog and signal delivery.
    pub main_pid: Option<nix::unistd::Pid>,
    pub status_msgs: Vec<String>,

    pub process_group: Option<nix::unistd::Pid>,

    pub signaled_ready: bool,
    /// Set to true when the service sends RELOADING=1 via sd_notify.
    /// Cleared when the service sends READY=1 again after reload completes.
    pub reloading: bool,
    /// Set to true when the service sends STOPPING=1 via sd_notify.
    pub stopping: bool,
    /// Timestamp of the last WATCHDOG=1 ping from the service.
    /// Used with WatchdogSec= to detect unresponsive services.
    pub watchdog_last_ping: Option<std::time::Instant>,

    /// ERRNO= from sd_notify — the last errno value reported by the service.
    /// Services send this to indicate an error condition (e.g. `ERRNO=2` for ENOENT).
    /// Displayed in `systemctl status` output.
    pub notify_errno: Option<i32>,
    /// BUSERROR= from sd_notify — the last D-Bus error name reported by the service
    /// (e.g. `BUSERROR=org.freedesktop.DBus.Error.TimedOut`). Displayed in status.
    pub notify_bus_error: Option<String>,
    /// EXIT_STATUS= from sd_notify — the last exit status reported by the service.
    /// Can be a numeric exit code or a signal name. Displayed in status.
    pub notify_exit_status: Option<String>,
    /// MONOTONIC_USEC= from sd_notify — the monotonic timestamp (in microseconds)
    /// associated with the notification message. Used for ordering and timing.
    pub notify_monotonic_usec: Option<u64>,
    /// INVOCATION_ID= from sd_notify — the invocation ID for the current service
    /// start. Set by the service manager and verified if the service sends it back.
    pub invocation_id: Option<String>,
    /// WATCHDOG_USEC= from sd_notify — dynamic watchdog timeout override.
    /// When a service sends this, it requests a change to its watchdog interval.
    /// Stored as microseconds; converted to a Duration for watchdog enforcement.
    pub watchdog_usec_override: Option<u64>,
    /// File descriptors received via FDSTORE=1 sd_notify messages with SCM_RIGHTS.
    /// Stored as (fd_name, raw_fd) pairs, respecting FileDescriptorStoreMax=.
    pub stored_fds: Vec<(String, RawFd)>,
    /// NOTIFYACCESS= runtime override from sd_notify.
    /// When a service sends `NOTIFYACCESS=<value>` via sd_notify, this field
    /// is set to override the unit file's `NotifyAccess=` setting at runtime.
    /// Use [`effective_notify_access()`] to get the effective value.
    pub notify_access_override: Option<NotifyKind>,

    pub notifications: Option<UnixDatagram>,
    pub notifications_path: Option<std::path::PathBuf>,

    pub stdout: Option<StdIo>,
    pub stderr: Option<StdIo>,
    pub notifications_buffer: String,
    pub stdout_buffer: Vec<u8>,
    pub stderr_buffer: Vec<u8>,
    /// Set to `true` by the watchdog enforcement thread when this service's
    /// watchdog timeout fires.  The exit handler checks this flag so that
    /// `Restart=on-watchdog` triggers a restart.  Cleared on service
    /// (re)start.
    pub watchdog_timeout_fired: bool,
    /// The exit status of the main service process (exit code or signal number).
    /// Set by the exit handler when the main process exits.
    pub main_exit_status: Option<i32>,
    /// The PID of the main service process at the time it was started.
    /// Unlike `pid` (which is cleared on stop), this persists for `ExecMainPID`.
    pub main_exit_pid: Option<nix::unistd::Pid>,
    /// TRIGGER_PATH — set by the path watcher when a path unit triggers this
    /// service. Contains the filesystem path that matched the path condition.
    pub trigger_path: Option<String>,
    /// TRIGGER_UNIT — set by the path watcher when a path unit triggers this
    /// service. Contains the name of the .path unit that fired the trigger.
    pub trigger_unit: Option<String>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum RunCmdError {
    Timeout(String, String),
    SpawnError(String, String),
    WaitError(String, String),
    BadExitCode(String, crate::signal_handler::ChildTermination),
    ExitBeforeNotify(String, crate::signal_handler::ChildTermination),
    CreatingShmemFailed(String, std::io::ErrorKind),
    Generic(String),
}

impl std::fmt::Display for RunCmdError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let msg = match self {
            Self::BadExitCode(cmd, exit) => format!("{cmd} exited with {exit}"),
            Self::ExitBeforeNotify(cmd, exit) => {
                format!("{cmd} exited before sending READY=1 with {exit}")
            }
            Self::SpawnError(cmd, err) => format!("{cmd} failed to spawn: {err}"),
            Self::WaitError(cmd, err) => {
                format!("{cmd} could not be waited on: {err}")
            }
            Self::CreatingShmemFailed(cmd, err) => {
                format!("{cmd} could not create shared memory for chainloading config: {err:?}")
            }
            Self::Timeout(cmd, err) => format!("{cmd} reached its timeout: {err}"),
            Self::Generic(err) => format!("Generic error: {err}"),
        };
        fmt.write_str(msg.as_str())
    }
}

pub enum StartResult {
    Started,
    WaitingForSocket,
    /// ExecCondition= command exited with 1-254; service is skipped (not failed).
    ConditionSkipped,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub enum ServiceErrorReason {
    PrestartFailed(RunCmdError),
    PoststartFailed(RunCmdError),
    StartFailed(RunCmdError),
    PoststopFailed(RunCmdError),
    StopFailed(RunCmdError),

    PrestartAndPoststopFailed(RunCmdError, RunCmdError),
    PoststartAndPoststopFailed(RunCmdError, RunCmdError),
    StartAndPoststopFailed(RunCmdError, RunCmdError),
    StopAndPoststopFailed(RunCmdError, RunCmdError),
    PreparingFailed(String),
    Generic(String),
    AlreadyHasPID(nix::unistd::Pid),
    AlreadyHasPGID(nix::unistd::Pid),
}

impl std::fmt::Display for ServiceErrorReason {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let msg = match self {
            // one failed
            Self::PrestartFailed(e) => format!("Perstart failed: {e}"),
            Self::PoststartFailed(e) => format!("Poststart failed: {e}"),
            Self::StartFailed(e) => format!("Start failed: {e}"),
            Self::StopFailed(e) => format!("Stop failed: {e}"),
            Self::PoststopFailed(e) => format!("Poststop failed: {e}"),

            // Both failed
            Self::PrestartAndPoststopFailed(e, e2) => {
                format!("Perstart failed: {e} and Poststop failed too: {e2}")
            }
            Self::PoststartAndPoststopFailed(e, e2) => {
                format!("Poststart failed: {e} and Poststop failed too: {e2}")
            }
            Self::StartAndPoststopFailed(e, e2) => {
                format!("Start failed: {e} and Poststop failed too: {e2}")
            }
            Self::StopAndPoststopFailed(e, e2) => {
                format!("Stop failed: {e} and Poststop failed too: {e2}")
            }

            // other errors
            Self::Generic(e) => format!("Service error: {e}"),
            Self::AlreadyHasPID(e) => {
                format!("Tried to start already running service (PID: {e})")
            }
            Self::AlreadyHasPGID(e) => {
                format!("Tried to start already running service: (PGID: {e})")
            }
            Self::PreparingFailed(e) => {
                format!("Preparing of service failed because: {e}")
            }
        };
        fmt.write_str(msg.as_str())
    }
}

impl Service {
    pub fn start(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
        _source: ActivationSource,
    ) -> Result<StartResult, ServiceErrorReason> {
        if let Some(pid) = self.pid {
            return Err(ServiceErrorReason::AlreadyHasPID(pid));
        }
        if let Some(pgid) = self.process_group {
            return Err(ServiceErrorReason::AlreadyHasPID(pgid));
        }
        if conf.accept {
            return Err(ServiceErrorReason::Generic(
                "Inetd style activation is not supported".into(),
            ));
        }
        // Always start the service when start() is called.  If the service
        // has associated sockets, the socket units will already have been
        // activated (opened) before we get here thanks to Requires=/After=
        // ordering.  Real systemd starts socket-having services immediately
        // when they are pulled in by Wants=/Requires= from a target; the
        // WaitingForSocket deferral only applies to services that are NOT
        // in the active dependency graph and are started on-demand when
        // traffic arrives on their socket — but in that case start() is
        // called with ActivationSource::SocketActivation anyway.
        trace!("Start service {name}");

        // Clear watchdog state from any previous run so that the watchdog
        // enforcement thread doesn't immediately kill the freshly started
        // service based on stale state.
        self.watchdog_timeout_fired = false;
        self.watchdog_last_ping = None;
        self.watchdog_usec_override = None;
        self.main_exit_status = None;

        super::prepare_service::prepare_service(
            self,
            conf,
            name,
            &run_info.config.notification_sockets_dir,
        )
        .map_err(ServiceErrorReason::PreparingFailed)?;

        // ExecCondition= — if any command exits 1-254, skip the service.
        match self.run_exec_condition(conf, id.clone(), name, run_info) {
            Ok(true) => {} // conditions met, proceed
            Ok(false) => return Ok(StartResult::ConditionSkipped),
            Err(e) => {
                return Err(match self.run_poststop(conf, id.clone(), name, run_info) {
                    Ok(()) => ServiceErrorReason::PrestartFailed(e),
                    Err(poststop_err) => {
                        ServiceErrorReason::PrestartAndPoststopFailed(e, poststop_err)
                    }
                });
            }
        }

        self.run_prestart(conf, id.clone(), name, run_info)
            .map_err(
                |prestart_err| match self.run_poststop(conf, id.clone(), name, run_info) {
                    Ok(()) => ServiceErrorReason::PrestartFailed(prestart_err),
                    Err(poststop_err) => {
                        ServiceErrorReason::PrestartAndPoststopFailed(prestart_err, poststop_err)
                    }
                },
            )?;

        // Re-open stdout/stderr after ExecStartPre, which may have deleted
        // the output file (e.g. ExecStartPre=rm -f ...). In real systemd,
        // each process invocation gets fresh file descriptors.
        self.stdout = None;
        self.stderr = None;
        super::prepare_service::reopen_stdio(self, conf)
            .map_err(ServiceErrorReason::PreparingFailed)?;

        if conf.exec.is_some() {
            // Service has an ExecStart command — fork and wait for it.
            let has_minus_prefix = conf
                .exec
                .as_ref()
                .map(|e| e.prefixes.contains(&CommandlinePrefix::Minus))
                .unwrap_or(false);
            {
                let mut pid_table_locked = run_info.pid_table.lock_poisoned();
                // This mainly just forks the process. The waiting (if necessary) is done below
                // Doing it under the lock of the pid_table prevents races between processes exiting very
                // fast and inserting the new pid into the pid table
                match start_service(
                    &run_info.config.self_path,
                    self,
                    conf,
                    name,
                    &run_info.fd_store.read_poisoned(),
                ) {
                    Ok(()) => {
                        if let Some(new_pid) = self.pid {
                            pid_table_locked
                                .insert(new_pid, PidEntry::Service(id.clone(), conf.srcv_type));
                        }
                    }
                    Err(e) if has_minus_prefix => {
                        // ExecStart=- prefix: ignore spawn errors (e.g. binary not found).
                        // Record the error exit status for ExecMainStatus property.
                        trace!("Ignore spawn error for ExecStart with '-' prefix for {name}: {e}");
                        self.main_exit_status = Some(203); // EXIT_EXEC (exec format error)
                    }
                    Err(e) => return Err(ServiceErrorReason::StartFailed(e)),
                }
            }

            // Only wait for the service if it was actually spawned (has a PID).
            if self.pid.is_some() {
                super::fork_parent::wait_for_service(self, conf, name, run_info).map_err(
                    |start_err| match self.run_poststop(conf, id.clone(), name, run_info) {
                        Ok(()) => ServiceErrorReason::StartFailed(start_err),
                        Err(poststop_err) => {
                            ServiceErrorReason::StartAndPoststopFailed(start_err, poststop_err)
                        }
                    },
                )?;
            }
        } else {
            // Exec-less oneshot service (e.g. systemd-reboot.service).
            // No main process to fork — the service succeeds immediately.
            trace!("Service {name} has no ExecStart, treating as immediately successful oneshot");
        }

        // For non-notify services, initialize the watchdog reference
        // timestamp now (the process has been forked and is running).
        // For Type=notify services this will be overwritten when READY=1
        // arrives via the notification handler.
        if conf.watchdog_sec.is_some() && self.watchdog_last_ping.is_none() {
            self.watchdog_last_ping = Some(std::time::Instant::now());
        }

        self.run_poststart(conf, id.clone(), name, run_info)
            .map_err(
                |poststart_err| match self.run_poststop(conf, id.clone(), name, run_info) {
                    Ok(()) => ServiceErrorReason::PrestartFailed(poststart_err),
                    Err(poststop_err) => {
                        ServiceErrorReason::PoststartAndPoststopFailed(poststart_err, poststop_err)
                    }
                },
            )?;
        Ok(StartResult::Started)
    }

    pub fn kill_all_remaining_processes(&mut self, conf: &ServiceConfig, name: &str) {
        trace!(
            "Kill all process for {name} (kill_mode: {:?}, send_sighup: {})",
            conf.kill_mode, conf.send_sighup
        );

        // SendSIGHUP= — send SIGHUP to remaining processes before SIGKILL.
        // This notifies shell-like processes that their connection has been
        // severed, matching systemd.kill(5) behaviour.
        if conf.send_sighup
            && let Some(proc_group) = self.process_group
        {
            match nix::sys::signal::kill(proc_group, nix::sys::signal::Signal::SIGHUP) {
                Ok(()) => {
                    trace!("SendSIGHUP: success sending SIGHUP to process group for service {name}")
                }
                Err(e) => {
                    trace!(
                        "SendSIGHUP: error sending SIGHUP to process group for service {name}: {e}"
                    )
                }
            }
        }

        match conf.kill_mode {
            KillMode::ControlGroup => {
                if let Some(proc_group) = self.process_group {
                    match nix::sys::signal::kill(proc_group, nix::sys::signal::Signal::SIGKILL) {
                        Ok(()) => trace!("Success killing process group for service {name}"),
                        Err(e) => trace!("Error killing process group for service {name}: {e}"),
                    }
                } else {
                    trace!(
                        "Tried to kill service that didn't have a process-group. This might have resulted in orphan processes."
                    );
                }
                match super::kill_os_specific::kill(conf, nix::sys::signal::Signal::SIGKILL) {
                    Ok(()) => trace!("Success killing process os specifically for service {name}"),
                    Err(e) => {
                        trace!("Error killing process os specifically for service {name}: {e}")
                    }
                }
            }
            KillMode::Process => {
                if let Some(pid) = self.pid {
                    match nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL) {
                        Ok(()) => trace!("Success killing main process for service {name}"),
                        Err(e) => trace!("Error killing main process for service {name}: {e}"),
                    }
                } else {
                    trace!("KillMode=process but service {name} has no main pid to kill");
                }
            }
            KillMode::Mixed => {
                // Send SIGTERM to the main process, SIGKILL to the rest of the group
                if let Some(pid) = self.pid {
                    match nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
                        Ok(()) => {
                            trace!("Success sending SIGTERM to main process for service {name}")
                        }
                        Err(e) => {
                            trace!("Error sending SIGTERM to main process for service {name}: {e}")
                        }
                    }
                }
                if let Some(proc_group) = self.process_group {
                    match nix::sys::signal::kill(proc_group, nix::sys::signal::Signal::SIGKILL) {
                        Ok(()) => {
                            trace!("Success killing remaining process group for service {name}")
                        }
                        Err(e) => {
                            trace!("Error killing remaining process group for service {name}: {e}")
                        }
                    }
                }
                match super::kill_os_specific::kill(conf, nix::sys::signal::Signal::SIGKILL) {
                    Ok(()) => trace!("Success killing process os specifically for service {name}"),
                    Err(e) => {
                        trace!("Error killing process os specifically for service {name}: {e}")
                    }
                }
            }
            KillMode::None => {
                trace!("KillMode=none for service {name}, not killing any remaining processes");
            }
        }
    }

    fn stop(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), RunCmdError> {
        self.run_stop_cmd(conf, id, name, run_info)
    }
    pub fn kill(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), ServiceErrorReason> {
        // When a unit is masked (symlink to /dev/null), skip ExecStop and
        // ExecStopPost commands — just kill the processes directly.
        // See systemd issue #38802.
        if is_unit_masked(name) {
            trace!("Unit {name} is masked, skipping ExecStop/ExecStopPost");
            self.kill_all_remaining_processes(conf, name);
            self.pid = None;
            self.process_group = None;
            return Ok(());
        }

        self.stop(conf, id.clone(), name, run_info)
            .map_err(|stop_err| {
                trace!(
                    "Stop process failed with: {stop_err:?} for service: {name}. Running poststop commands"
                );
                match self.run_poststop(conf, id.clone(), name, run_info) {
                    Ok(()) => ServiceErrorReason::StopFailed(stop_err),
                    Err(poststop_err) => {
                        ServiceErrorReason::StopAndPoststopFailed(stop_err, poststop_err)
                    }
                }
            })
            .and_then(|()| {
                trace!(
                    "Stop processes for service: {name} ran successfully. Running poststop commands"
                );
                self.run_poststop(conf, id.clone(), name, run_info)
                    .map_err(ServiceErrorReason::PoststopFailed)
            })
    }

    #[must_use]
    pub const fn get_start_timeout(&self, conf: &ServiceConfig) -> Option<std::time::Duration> {
        if let Some(timeout) = &conf.starttimeout {
            match timeout {
                Timeout::Duration(dur) => Some(*dur),
                Timeout::Infinity => None,
            }
        } else if let Some(timeout) = &conf.generaltimeout {
            match timeout {
                Timeout::Duration(dur) => Some(*dur),
                Timeout::Infinity => None,
            }
        } else {
            // Match real systemd's DefaultTimeoutStartSec (90 seconds).
            // The previous 1s default caused notify/dbus services to time out
            // before they could send their readiness notification.
            Some(std::time::Duration::from_secs(90))
        }
    }

    const fn get_stop_timeout(&self, conf: &ServiceConfig) -> Option<std::time::Duration> {
        if let Some(timeout) = &conf.stoptimeout {
            match timeout {
                Timeout::Duration(dur) => Some(*dur),
                Timeout::Infinity => None,
            }
        } else if let Some(timeout) = &conf.generaltimeout {
            match timeout {
                Timeout::Duration(dur) => Some(*dur),
                Timeout::Infinity => None,
            }
        } else {
            // TODO is 1 sec ok?
            Some(std::time::Duration::from_millis(1000))
        }
    }

    fn run_cmd(
        &mut self,
        cmdline: &Commandline,
        id: UnitId,
        name: &str,
        timeout: Option<std::time::Duration>,
        run_info: &RuntimeInfo,
        working_directory: Option<&std::path::PathBuf>,
    ) -> Result<(), RunCmdError> {
        let mut cmd = Command::new(&cmdline.cmd);
        if cmdline.prefixes.contains(&CommandlinePrefix::AtSign) {
            // With '@' prefix: first arg becomes argv[0], remaining args are normal arguments
            use std::os::unix::process::CommandExt;
            if let Some(argv0) = cmdline.args.first() {
                cmd.arg0(argv0);
            }
            for part in cmdline.args.iter().skip(1) {
                cmd.arg(part);
            }
        } else {
            for part in &cmdline.args {
                cmd.arg(part);
            }
        }
        if let Some(dir) = working_directory {
            cmd.current_dir(dir);
        }
        use std::os::unix::io::FromRawFd;
        let stdout = if let Some(stdio) = &self.stdout {
            unsafe {
                let duped = nix::unistd::dup(BorrowedFd::borrow_raw(stdio.write_fd())).unwrap();
                Stdio::from(std::fs::File::from_raw_fd(duped.into_raw_fd()))
            }
        } else {
            Stdio::piped()
        };
        let stderr = if let Some(stdio) = &self.stderr {
            unsafe {
                let duped = nix::unistd::dup(BorrowedFd::borrow_raw(stdio.write_fd())).unwrap();
                Stdio::from(std::fs::File::from_raw_fd(duped.into_raw_fd()))
            }
        } else {
            Stdio::piped()
        };

        cmd.stdout(stdout);
        cmd.stderr(stderr);
        cmd.stdin(Stdio::null());

        // Pass TRIGGER_PATH / TRIGGER_UNIT env vars for path-activated services
        if let Some(ref tp) = self.trigger_path {
            cmd.env("TRIGGER_PATH", tp);
        }
        if let Some(ref tu) = self.trigger_unit {
            cmd.env("TRIGGER_UNIT", tu);
        }

        trace!("Run {cmdline:?} for service: {name}");
        let spawn_result = {
            let mut pid_table_locked = run_info.pid_table.lock_poisoned();
            let res = cmd.spawn();
            if let Ok(child) = &res {
                pid_table_locked.insert(
                    nix::unistd::Pid::from_raw(child.id() as i32),
                    PidEntry::Helper(id.clone(), name.to_string()),
                );
            }
            res
        };
        match spawn_result {
            Ok(mut child) => {
                trace!("Wait for {cmdline:?} for service: {name}");
                let wait_result: Result<(), RunCmdError> = match wait_for_helper_child(
                    &child, run_info, timeout,
                ) {
                    WaitResult::InTime(Err(e)) => {
                        return Err(RunCmdError::WaitError(cmdline.to_string(), format!("{e}")));
                    }
                    WaitResult::InTime(Ok(exitstatus)) => {
                        if exitstatus.success() {
                            trace!("success running {cmdline:?} for service: {name}");
                            Ok(())
                        } else if cmdline.prefixes.contains(&CommandlinePrefix::Minus) {
                            trace!(
                                "Ignore error exit code: {exitstatus:?} while running {cmdline:?} for service: {name}"
                            );
                            Ok(())
                        } else {
                            trace!(
                                "Error exit code: {exitstatus:?} while running {cmdline:?} for service: {name}"
                            );
                            Err(RunCmdError::BadExitCode(cmdline.to_string(), exitstatus))
                        }
                    }
                    WaitResult::TimedOut => {
                        trace!("Timeout running {cmdline:?} for service: {name}");
                        let _ = child.kill();
                        Err(RunCmdError::Timeout(
                            cmdline.to_string(),
                            format!("Timeout ({timeout:?}) reached"),
                        ))
                    }
                };
                {
                    let unit = run_info.unit_table.get(&id).unwrap();
                    let status = &*unit.common.status.read_poisoned();
                    use std::io::Read;
                    if let Some(stream) = &mut child.stderr {
                        let mut buf = Vec::new();
                        let _bytes = stream.read_to_end(&mut buf).unwrap();
                        self.stderr_buffer.extend(buf);
                        self.log_stderr_lines(name, status).unwrap();
                    }
                    if let Some(stream) = &mut child.stdout {
                        let mut buf = Vec::new();
                        let _bytes = stream.read_to_end(&mut buf).unwrap();
                        self.stdout_buffer.extend(buf);
                        self.log_stdout_lines(name, status).unwrap();
                    }
                }

                run_info
                    .pid_table
                    .lock()
                    .unwrap()
                    .remove(&nix::unistd::Pid::from_raw(child.id() as i32));
                wait_result
            }
            Err(e) => {
                if cmdline.prefixes.contains(&CommandlinePrefix::Minus) {
                    trace!(
                        "Ignore spawn error for {cmdline:?} for service: {name} (has '-' prefix): {e}"
                    );
                    Ok(())
                } else {
                    Err(RunCmdError::SpawnError(cmdline.to_string(), format!("{e}")))
                }
            }
        }
    }

    fn run_all_cmds(
        &mut self,
        cmds: &Vec<Commandline>,
        id: UnitId,
        name: &str,
        timeout: Option<std::time::Duration>,
        run_info: &RuntimeInfo,
        working_directory: Option<&std::path::PathBuf>,
    ) -> Result<(), RunCmdError> {
        for cmd in cmds {
            self.run_cmd(cmd, id.clone(), name, timeout, run_info, working_directory)?;
        }
        Ok(())
    }

    fn run_stop_cmd(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), RunCmdError> {
        if conf.stop.is_empty() {
            return Ok(());
        }
        let timeout = self.get_stop_timeout(conf);
        let cmds = conf.stop.clone();
        self.run_all_cmds(
            &cmds,
            id,
            name,
            timeout,
            run_info,
            conf.exec_config.working_directory.as_ref(),
        )
    }
    /// Run ExecCondition= commands. Returns Ok(true) to proceed, Ok(false) to
    /// skip the service (condition not met), or Err on hard failure (exit 255
    /// or signal).
    fn run_exec_condition(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<bool, RunCmdError> {
        if conf.exec_condition.is_empty() {
            return Ok(true);
        }
        let timeout = self.get_start_timeout(conf);
        for cmd in &conf.exec_condition.clone() {
            match self.run_cmd(
                cmd,
                id.clone(),
                name,
                timeout,
                run_info,
                conf.exec_config.working_directory.as_ref(),
            ) {
                Ok(()) => {} // exit 0 — condition met, continue
                Err(RunCmdError::BadExitCode(
                    _,
                    crate::signal_handler::ChildTermination::Exit(code),
                )) if code != 255 => {
                    // Exit 1-254: condition not met, skip service (not a failure)
                    trace!("ExecCondition {cmd:?} exited with {code}, skipping service {name}");
                    return Ok(false);
                }
                Err(e) => {
                    // Exit 255 or signal: hard failure
                    return Err(e);
                }
            }
        }
        Ok(true)
    }

    fn run_prestart(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), RunCmdError> {
        if conf.startpre.is_empty() {
            return Ok(());
        }
        let timeout = self.get_start_timeout(conf);
        let cmds = conf.startpre.clone();
        self.run_all_cmds(
            &cmds,
            id,
            name,
            timeout,
            run_info,
            conf.exec_config.working_directory.as_ref(),
        )
    }
    fn run_poststart(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), RunCmdError> {
        if conf.startpost.is_empty() {
            return Ok(());
        }
        let timeout = self.get_start_timeout(conf);
        let cmds = conf.startpost.clone();
        self.run_all_cmds(
            &cmds,
            id,
            name,
            timeout,
            run_info,
            conf.exec_config.working_directory.as_ref(),
        )
    }
    fn run_poststop(
        &mut self,
        conf: &ServiceConfig,
        id: UnitId,
        name: &str,
        run_info: &RuntimeInfo,
    ) -> Result<(), RunCmdError> {
        trace!("Run poststop for {name}");
        let timeout = self.get_stop_timeout(conf);
        let cmds = conf.stoppost.clone();
        let res = self.run_all_cmds(
            &cmds,
            id,
            name,
            timeout,
            run_info,
            conf.exec_config.working_directory.as_ref(),
        );

        if conf.srcv_type != ServiceType::OneShot {
            // already happened when the oneshot process exited in the exit handler
            self.kill_all_remaining_processes(conf, name);
        }
        self.pid = None;
        self.process_group = None;
        res
    }

    pub fn log_stdout_lines(&mut self, name: &str, status: &UnitStatus) -> std::io::Result<()> {
        let mut prefix = String::new();
        prefix.push('[');
        prefix.push_str(name);
        prefix.push(']');
        let _ = write!(prefix, "[{status}]");
        prefix.push(' ');
        let mut outbuf: Vec<u8> = Vec::new();
        while self.stdout_buffer.contains(&b'\n') {
            let split_pos = self.stdout_buffer.iter().position(|r| *r == b'\n').unwrap();
            let (line, lines) = self.stdout_buffer.split_at(split_pos + 1);

            // drop \n at the end of the line
            let line = &line[0..line.len() - 1].to_vec();
            self.stdout_buffer = lines.to_vec();
            if line.is_empty() {
                continue;
            }
            outbuf.clear();
            outbuf.extend(prefix.as_bytes());
            outbuf.extend(line);
            outbuf.push(b'\n');
            std::io::stdout().write_all(&outbuf)?;
        }
        Ok(())
    }
    pub fn log_stderr_lines(&mut self, name: &str, status: &UnitStatus) -> std::io::Result<()> {
        let mut prefix = String::new();
        prefix.push('[');
        prefix.push_str(name);
        prefix.push(']');
        let _ = write!(prefix, "[{status}]");
        prefix.push_str("[STDERR]");
        prefix.push(' ');

        let mut outbuf: Vec<u8> = Vec::new();
        while self.stderr_buffer.contains(&b'\n') {
            let split_pos = self.stderr_buffer.iter().position(|r| *r == b'\n').unwrap();
            let (line, lines) = self.stderr_buffer.split_at(split_pos + 1);

            // drop \n at the end of the line
            let line = &line[0..line.len() - 1].to_vec();
            self.stderr_buffer = lines.to_vec();
            if line.is_empty() {
                continue;
            }
            outbuf.clear();
            outbuf.extend(prefix.as_bytes());
            outbuf.extend(line);
            outbuf.push(b'\n');
            std::io::stderr().write_all(&outbuf).unwrap();
        }
        Ok(())
    }
}

enum WaitResult {
    TimedOut,
    InTime(std::io::Result<crate::signal_handler::ChildTermination>),
}

/// Wait for the termination of a subprocess, with an optional timeout.
/// An error does not mean that the waiting actually failed.
/// This might also happen because it was collected by the `signal_handler`.
/// This could be fixed by using the `waitid()` with WNOWAIT in the signal handler but
/// that has not been ported to rust
fn wait_for_helper_child(
    child: &std::process::Child,
    run_info: &RuntimeInfo,
    time_out: Option<std::time::Duration>,
) -> WaitResult {
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    let mut counter = 1u64;
    let start_time = std::time::Instant::now();
    loop {
        if let Some(time_out) = time_out
            && start_time.elapsed() >= time_out
        {
            return WaitResult::TimedOut;
        }
        {
            let mut pid_table_locked = run_info.pid_table.lock_poisoned();
            match pid_table_locked.get(&pid) {
                Some(entry) => {
                    match entry {
                        PidEntry::ServiceExited(_) => {
                            // Should never happen
                            unreachable!(
                                "Was waiting on helper process but pid got saved as PidEntry::OneshotExited"
                            );
                        }
                        PidEntry::Service(_, _) => {
                            // Should never happen
                            unreachable!(
                                "Was waiting on helper process but pid got saved as PidEntry::Service"
                            );
                        }
                        PidEntry::Helper(_, _) => {
                            // Need to wait longer
                        }
                        PidEntry::HelperExited(_) => {
                            let entry_owned = pid_table_locked.remove(&pid).unwrap();
                            if let PidEntry::HelperExited(termination_owned) = entry_owned {
                                return WaitResult::InTime(Ok(termination_owned));
                            }
                        }
                    }
                }
                None => {
                    // Should not happen. Either there is an Helper entry or a Exited entry
                    unreachable!("No entry for child found")
                }
            }
        }
        // exponential backoff to get low latencies for fast processes
        // but not hog the cpu for too long
        // start at 0.05 ms
        // capped to 10 ms to not introduce too big latencies
        // TODO review those numbers
        let sleep_dur = std::time::Duration::from_micros(counter * 50);
        let sleep_cap = std::time::Duration::from_millis(10);
        let sleep_dur = sleep_dur.min(sleep_cap);
        if sleep_dur < sleep_cap {
            counter *= 2;
        }
        std::thread::sleep(sleep_dur);
    }
}
