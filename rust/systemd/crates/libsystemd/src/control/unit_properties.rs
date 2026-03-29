//! Extract unit properties in key=value format for `systemctl show`.
//!
//! Real systemd exposes hundreds of properties per unit via D-Bus.  We expose
//! the most commonly used subset, formatted exactly as `systemctl show` would
//! print them (one `Key=Value` per line, lists space-separated, booleans as
//! `yes`/`no`, timestamps as `n/a` when unavailable).

use crate::lock_ext::RwLockExt;
use crate::units::{
    Commandline, CommandlinePrefix, ExecConfig, FreezerState, KillMode, MountConfig, NotifyKind,
    ServiceConfig, ServiceRestart, ServiceType, SliceConfig, SocketConfig, Specific, SwapConfig,
    Timeout, Unit, UnitConfig, UnitStatus,
};

/// Insertion-order-preserving property map.
///
/// Real systemd emits properties in vtable order (not alphabetical).
/// By preserving the order properties are inserted in `collect_properties`,
/// we can match that behavior when formatting output.
#[derive(Debug, Clone, Default)]
pub struct PropertyMap {
    entries: Vec<(String, String)>,
}

impl PropertyMap {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Insert or update a key-value pair.  If the key already exists
    /// (case-sensitive), the value is updated in place, preserving position.
    pub fn insert(&mut self, key: String, value: String) {
        if let Some(entry) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value;
        } else {
            self.entries.push((key, value));
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// Case-insensitive lookup by key.
    pub fn get_ci(&self, key: &str) -> Option<(&String, &String)> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(k, v)| (k, v))
    }

    /// Exact-match lookup by key, returning the value.
    pub fn get(&self, key: &str) -> Option<&String> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Check if a key exists (exact match).
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }
}

/// Read the FreezerState from a unit's common state.
/// Uses `try_read` to avoid blocking when the state write-lock is held
/// (e.g. during `reactivate`), preventing an ABBA deadlock with the
/// unit-status lock in `collect_properties`.
fn get_freezer_state(unit: &Unit) -> FreezerState {
    match &unit.specific {
        Specific::Service(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Socket(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Target(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Slice(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Timer(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Path(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Mount(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Swap(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
        Specific::Device(s) => s
            .state
            .try_read()
            .map(|g| g.common.freezer_state)
            .unwrap_or_default(),
    }
}

/// Public accessor for get_freezer_state.
pub fn get_freezer_state_pub(unit: &Unit) -> FreezerState {
    get_freezer_state(unit)
}

/// Set the FreezerState on a unit's common state.
pub fn set_freezer_state(unit: &Unit, state: FreezerState) {
    match &unit.specific {
        Specific::Service(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Socket(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Target(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Slice(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Timer(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Path(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Mount(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Swap(s) => s.state.write_poisoned().common.freezer_state = state,
        Specific::Device(s) => s.state.write_poisoned().common.freezer_state = state,
    }
}

/// Collect all properties of a unit into an ordered map.
///
/// The returned `BTreeMap` keeps keys in alphabetical order, matching the
/// default output of `systemctl show` (which lists properties sorted).
pub fn collect_properties(unit: &Unit) -> PropertyMap {
    let mut props = PropertyMap::new();

    // ── Identity ──────────────────────────────────────────────────────
    insert(&mut props, "Id", &unit.id.name);
    {
        let mut names = vec![unit.id.name.clone()];
        for alias in &unit.common.unit.aliases {
            if !names.contains(alias) {
                names.push(alias.clone());
            }
        }
        insert(&mut props, "Names", &names.join(" "));
    }

    // ── [Unit] section ────────────────────────────────────────────────
    insert_unit_config(&mut props, &unit.common.unit);

    // ── Dependencies ──────────────────────────────────────────────────
    insert_dep_list(&mut props, "Wants", &unit.common.dependencies.wants);
    insert_dep_list(&mut props, "WantedBy", &unit.common.dependencies.wanted_by);
    insert_dep_list(&mut props, "Requires", &unit.common.dependencies.requires);
    insert_dep_list(
        &mut props,
        "RequiredBy",
        &unit.common.dependencies.required_by,
    );
    insert_dep_list(&mut props, "Conflicts", &unit.common.dependencies.conflicts);
    insert_dep_list(
        &mut props,
        "ConflictedBy",
        &unit.common.dependencies.conflicted_by,
    );
    insert_dep_list(&mut props, "Before", &unit.common.dependencies.before);
    insert_dep_list(&mut props, "After", &unit.common.dependencies.after);
    insert_dep_list(&mut props, "PartOf", &unit.common.dependencies.part_of);
    insert_dep_list(&mut props, "PartOfBy", &unit.common.dependencies.part_of_by);
    insert_dep_list(&mut props, "BindsTo", &unit.common.dependencies.binds_to);
    insert_dep_list(&mut props, "BoundBy", &unit.common.dependencies.bound_by);
    insert_dep_list(&mut props, "Upholds", &unit.common.dependencies.upholds);
    insert_dep_list(&mut props, "UpheldBy", &unit.common.dependencies.upheld_by);
    insert_dep_list(
        &mut props,
        "PropagatesStopTo",
        &unit.common.dependencies.propagates_stop_to,
    );

    // ── Status ────────────────────────────────────────────────────────
    // Drop the unit-status READ lock before reading type-specific state
    // to avoid an ABBA deadlock with `reactivate` which acquires the
    // service-state WRITE lock first, then the unit-status WRITE lock
    // (via `state_transition_restarting`).
    {
        let status = unit.common.status.read_poisoned();
        insert_status(&mut props, &status);
    }

    // ── Lifecycle timestamps ─────────────────────────────────────────
    insert_timestamps(&mut props, unit);

    // ── FreezerState (cgroup freezer) ────────────────────────────────
    let freezer_state = get_freezer_state(unit);
    insert(&mut props, "FreezerState", freezer_state.as_str());

    // ── Can* capability booleans ─────────────────────────────────────
    let can_start = if unit.common.unit.refuse_manual_start {
        "no"
    } else {
        "yes"
    };
    let can_stop = if unit.common.unit.refuse_manual_stop {
        "no"
    } else {
        "yes"
    };
    insert(&mut props, "CanStart", can_start);
    insert(&mut props, "CanStop", can_stop);
    let can_reload = match &unit.specific {
        Specific::Service(svc) => {
            if svc.conf.reload.is_empty() {
                "no"
            } else {
                "yes"
            }
        }
        _ => "no",
    };
    insert(&mut props, "CanReload", can_reload);
    insert(
        &mut props,
        "CanIsolate",
        if unit.common.unit.allow_isolate {
            "yes"
        } else {
            "no"
        },
    );
    insert(&mut props, "CanFreeze", "yes");
    insert(&mut props, "CanClean", "no"); // TODO: implement systemctl clean

    // ── Type-specific properties ──────────────────────────────────────
    match &unit.specific {
        Specific::Service(svc) => {
            insert(&mut props, "Type", &format_service_type(svc.conf.srcv_type));
            // ControlGroup — cgroup path relative to cgroup root (like real systemd)
            #[cfg(target_os = "linux")]
            {
                let full_path = svc.conf.platform_specific.cgroup_path.display().to_string();
                let relative = full_path
                    .strip_prefix("/sys/fs/cgroup")
                    .unwrap_or(&full_path);
                let relative = if relative.is_empty() { "/" } else { relative };
                insert(&mut props, "ControlGroup", relative);
            }
            insert_service_config(&mut props, &svc.conf);
            insert_exec_config(&mut props, &svc.conf.exec_config);

            // Try to read service state without blocking.  During oneshot
            // activation the state write-lock is held for the entire
            // wait_for_service duration; blocking here would make
            // `systemctl show` hang.  Fall back to defaults if contended.
            let state_guard = svc.state.try_read();
            let state_ref = state_guard.as_ref().ok();

            if let Some(state) = state_ref {
                insert(
                    &mut props,
                    "NotifyAccess",
                    &format_notify_access(crate::services::effective_notify_access(
                        &state.srvc,
                        &svc.conf,
                    )),
                );
                // ControlPID (ExecStop/ExecStopPost process — not tracked separately)
                insert(&mut props, "ControlPID", "0");
                if let Some(pid) = state.srvc.pid {
                    insert(&mut props, "MainPID", &pid.to_string());
                } else {
                    insert(&mut props, "MainPID", "0");
                }
                if let Some(exit_pid) = state.srvc.main_exit_pid {
                    insert(&mut props, "ExecMainPID", &exit_pid.to_string());
                } else if let Some(pid) = state.srvc.pid {
                    insert(&mut props, "ExecMainPID", &pid.to_string());
                } else {
                    insert(&mut props, "ExecMainPID", "0");
                }
                insert(
                    &mut props,
                    "ExecMainStatus",
                    &state
                        .srvc
                        .main_exit_status
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "0".to_string()),
                );
                // Result — "success" if stopped cleanly, "timeout" / "watchdog" /
                // "exit-code" / "signal" on failure.
                // Inserted here (before NRestarts) to match systemd's vtable order.
                {
                    let status = unit.common.status.read_poisoned();
                    let result = if state.srvc.runtime_max_timeout_fired {
                        "timeout"
                    } else if state.srvc.watchdog_timeout_fired {
                        "watchdog"
                    } else {
                        match &*status {
                            UnitStatus::Stopped(_, errors) if !errors.is_empty() => "exit-code",
                            _ => "success",
                        }
                    };
                    insert(&mut props, "Result", result);
                }
                insert(
                    &mut props,
                    "NRestarts",
                    &unit
                        .common
                        .n_restarts
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .to_string(),
                );
                // ExecMain timestamps
                let fmt_ts = |v: Option<u64>| match v {
                    Some(usec) => format_usec_timestamp(usec),
                    None => "n/a".to_string(),
                };
                let fmt_usec = |v: Option<u64>| match v {
                    Some(usec) => usec.to_string(),
                    None => "0".to_string(),
                };
                insert(
                    &mut props,
                    "ExecMainStartTimestamp",
                    &fmt_ts(state.srvc.exec_main_start_timestamp),
                );
                insert(
                    &mut props,
                    "ExecMainStartTimestampMonotonic",
                    &fmt_usec(state.srvc.exec_main_start_timestamp),
                );
                insert(
                    &mut props,
                    "ExecMainHandoffTimestamp",
                    &fmt_ts(state.srvc.exec_main_handoff_timestamp),
                );
                insert(
                    &mut props,
                    "ExecMainHandoffTimestampMonotonic",
                    &fmt_usec(state.srvc.exec_main_handoff_timestamp),
                );
                insert(
                    &mut props,
                    "ExecMainExitTimestamp",
                    &fmt_ts(state.srvc.exec_main_exit_timestamp),
                );
                insert(
                    &mut props,
                    "ExecMainExitTimestampMonotonic",
                    &fmt_usec(state.srvc.exec_main_exit_timestamp),
                );
                // StatusText from sd_notify STATUS=
                insert(
                    &mut props,
                    "StatusText",
                    state
                        .srvc
                        .status_msgs
                        .last()
                        .map(|s| s.as_str())
                        .unwrap_or(""),
                );
                // StatusErrno from sd_notify ERRNO=
                insert(
                    &mut props,
                    "StatusErrno",
                    &state.srvc.notify_errno.unwrap_or(0).to_string(),
                );
            } else {
                // State lock is contended (service is activating).
                // Provide sensible defaults so `systemctl show` doesn't hang.
                insert(&mut props, "NotifyAccess", "none");
                insert(&mut props, "MainPID", "0");
                insert(&mut props, "ExecMainPID", "0");
                insert(&mut props, "ExecMainStatus", "0");
                {
                    let status = unit.common.status.read_poisoned();
                    let result = match &*status {
                        UnitStatus::Stopped(_, errors) if !errors.is_empty() => "exit-code",
                        _ => "success",
                    };
                    insert(&mut props, "Result", result);
                }
                insert(
                    &mut props,
                    "NRestarts",
                    &unit
                        .common
                        .n_restarts
                        .load(std::sync::atomic::Ordering::Relaxed)
                        .to_string(),
                );
            }

            insert(
                &mut props,
                "ExitType",
                match svc.conf.exit_type {
                    crate::units::ExitType::Main => "main",
                    crate::units::ExitType::Cgroup => "cgroup",
                },
            );

            // ── sd_notify reported fields ─────────────────────────────
            if let Some(state) = state_ref {
                if let Some(errno) = state.srvc.notify_errno {
                    insert(&mut props, "StatusErrno", &errno.to_string());
                } else {
                    insert(&mut props, "StatusErrno", "0");
                }
                if let Some(ref bus_error) = state.srvc.notify_bus_error {
                    insert(&mut props, "StatusBusError", bus_error);
                }
                if let Some(ref exit_status) = state.srvc.notify_exit_status {
                    insert(&mut props, "StatusExitStatus", exit_status);
                }
                if let Some(ref invocation_id) = state.srvc.invocation_id {
                    insert(&mut props, "InvocationID", invocation_id);
                }
                if let Some(usec) = state.srvc.watchdog_usec_override {
                    insert(&mut props, "WatchdogUSec", &usec.to_string());
                }
                insert(
                    &mut props,
                    "NFileDescriptorStore",
                    &state.srvc.stored_fds.len().to_string(),
                );
            } else {
                insert(&mut props, "StatusErrno", "0");
                insert(&mut props, "NFileDescriptorStore", "0");
            }
            insert(
                &mut props,
                "FileDescriptorStoreMax",
                &svc.conf.file_descriptor_store_max.to_string(),
            );

            // Kill-related properties
            insert(
                &mut props,
                "KillSignal",
                &svc.conf.kill_signal.unwrap_or(15).to_string(),
            );
            insert(
                &mut props,
                "RestartKillSignal",
                &svc.conf
                    .restart_kill_signal
                    .map_or("0".to_string(), |s| s.to_string()),
            );
            insert(
                &mut props,
                "FinalKillSignal",
                &svc.conf.final_kill_signal.unwrap_or(9).to_string(),
            );
            insert(
                &mut props,
                "SendSIGHUP",
                if svc.conf.send_sighup { "yes" } else { "no" },
            );
            // CleanResult / ReloadResult — always "success" (cleaning/reload
            // failures not tracked yet)
            insert(&mut props, "CleanResult", "success");
            insert(&mut props, "ReloadResult", "success");
        }
        Specific::Socket(sock) => {
            insert_socket_config(&mut props, &sock.conf);
            insert_exec_config(&mut props, &sock.conf.exec_config);
        }
        Specific::Target(_) => {
            // Targets have no type-specific properties beyond [Unit].
        }
        Specific::Slice(slice) => {
            insert_slice_config(&mut props, &slice.conf);
        }
        Specific::Mount(mnt) => {
            insert_mount_config(&mut props, &mnt.conf);
        }
        Specific::Swap(swp) => {
            insert_swap_config(&mut props, &swp.conf);
        }
        Specific::Timer(tmr) => {
            // Timer-specific properties
            insert(&mut props, "Unit", &tmr.conf.unit);
            insert(
                &mut props,
                "Persistent",
                if tmr.conf.persistent { "yes" } else { "no" },
            );
            insert(
                &mut props,
                "RemainAfterElapse",
                if tmr.conf.remain_after_elapse {
                    "yes"
                } else {
                    "no"
                },
            );
            insert(
                &mut props,
                "WakeSystem",
                if tmr.conf.wake_system { "yes" } else { "no" },
            );
            // NextElapseUSecRealtime — compute next calendar trigger time
            if !tmr.conf.on_calendar.is_empty() {
                if let Some(next) = compute_next_calendar_elapse(&tmr.conf.on_calendar) {
                    insert(&mut props, "NextElapseUSecRealtime", &next);
                } else {
                    insert(&mut props, "NextElapseUSecRealtime", "n/a");
                }
            } else {
                insert(&mut props, "NextElapseUSecRealtime", "n/a");
            }
            // LastTriggerUSec — when the timer last fired
            {
                let state = tmr.state.read_poisoned();
                if let Some(usec) = state.last_trigger_usec {
                    insert(&mut props, "LastTriggerUSec", &format_usec_timestamp(usec));
                    insert(&mut props, "LastTriggerUSecMonotonic", &usec.to_string());
                } else {
                    insert(&mut props, "LastTriggerUSec", "n/a");
                    insert(&mut props, "LastTriggerUSecMonotonic", "0");
                }
            }
            // TimersCalendar — list of OnCalendar= specs with next elapse times
            if !tmr.conf.on_calendar.is_empty() {
                let mut parts = Vec::new();
                for spec in &tmr.conf.on_calendar {
                    parts.push(format!("OnCalendar={}", spec));
                }
                insert(&mut props, "TimersCalendar", &parts.join(" ; "));
            }
        }
        Specific::Path(path) => {
            // Path-specific properties
            insert(&mut props, "Unit", &path.conf.unit);
            insert(
                &mut props,
                "MakeDirectory",
                if path.conf.make_directory {
                    "yes"
                } else {
                    "no"
                },
            );
            insert(
                &mut props,
                "DirectoryMode",
                &format!("{:04o}", path.conf.directory_mode),
            );
            // Result — "success" normally, "trigger-limit-hit" on rate limit
            let state = path.state.read_poisoned();
            insert(
                &mut props,
                "Result",
                match state.result {
                    crate::units::PathResult::Success => "success",
                    crate::units::PathResult::TriggerLimitHit => "trigger-limit-hit",
                },
            );
        }
        Specific::Device(dev) => {
            // Device-specific properties
            if let Some(ref sysfs) = dev.conf.sysfs_path {
                insert(&mut props, "SysFSPath", sysfs);
            }
        }
    }

    // ── LoadState / UnitFileState (synthetic) ─────────────────────────
    // Determine LoadState from the unit's fragment path:
    // - "masked" if the unit file is a symlink to /dev/null
    // - "bad-setting" if the unit has invalid configuration
    // - "loaded" otherwise
    let load_state = {
        let is_masked = match &unit.common.unit.fragment_path {
            Some(p) => std::fs::read_link(p)
                .map(|t| t == std::path::Path::new("/dev/null"))
                .unwrap_or(false),
            None => {
                let name = &unit.id.name;
                let runtime = std::path::Path::new("/run/systemd/system").join(name);
                let persistent = std::path::Path::new("/etc/systemd/system").join(name);
                std::fs::read_link(&runtime)
                    .or_else(|_| std::fs::read_link(&persistent))
                    .map(|t| t == std::path::Path::new("/dev/null"))
                    .unwrap_or(false)
            }
        };
        if is_masked {
            "masked"
        } else if has_bad_setting(unit) {
            "bad-setting"
        } else {
            "loaded"
        }
    };
    insert(&mut props, "LoadState", load_state);
    // UnitFileState reflects whether the unit file is masked, enabled, etc.
    let unit_file_state = if load_state == "masked" {
        "masked"
    } else {
        "enabled"
    };
    insert(&mut props, "UnitFileState", unit_file_state);

    // NeedDaemonReload — stub value, overridden by control.rs with real check
    // when unit_dirs are available.
    insert(&mut props, "NeedDaemonReload", "no");

    // Transient — whether the unit was created via start-transient (systemd-run)
    let is_transient = unit
        .common
        .unit
        .fragment_path
        .as_ref()
        .map(|p| {
            p.starts_with("/run/systemd/transient") || p.starts_with("/run/systemd/transient/")
        })
        .unwrap_or(false);
    insert_bool(&mut props, "Transient", is_transient);

    props
}

/// Check whether a unit has semantically invalid configuration.
///
/// Returns `true` for cases like Type=exec/simple/forking/notify/idle without
/// ExecStart=, matching systemd's `LoadState=bad-setting`.
fn has_bad_setting(unit: &Unit) -> bool {
    if let Specific::Service(srvc) = &unit.specific {
        let has_exec_start = !srvc.conf.exec.is_empty();
        if !has_exec_start {
            // Type=oneshot is valid without ExecStart (e.g. systemd-reboot.service).
            // All other service types require ExecStart.
            match srvc.conf.srcv_type {
                ServiceType::Simple
                | ServiceType::Exec
                | ServiceType::Forking
                | ServiceType::Notify
                | ServiceType::NotifyReload
                | ServiceType::Dbus
                | ServiceType::Idle => return true,
                ServiceType::OneShot => {}
            }
        }
    }
    false
}

/// Check whether a unit's on-disk configuration has changed since it was loaded.
///
/// Returns `true` (need reload) when:
/// - The main unit file (`fragment_path`) has been modified since `loaded_at`
/// - Drop-in `.conf` files have been added or removed in any `{unit_name}.d/`
///   directory under the unit search paths
pub fn need_daemon_reload(unit: &Unit, unit_dirs: &[std::path::PathBuf]) -> bool {
    let cfg = &unit.common.unit;
    let loaded_at = cfg.loaded_at;

    // Check if the main fragment file has been modified.
    if let Some(ref fpath) = cfg.fragment_path {
        match std::fs::metadata(fpath) {
            Ok(meta) => {
                if let Ok(mtime) = meta.modified()
                    && mtime > loaded_at
                {
                    return true;
                }
            }
            Err(_) => {
                // File no longer exists — need reload to detect removal
                return true;
            }
        }
    }

    // Collect current set of drop-in .conf file paths across all unit dirs.
    let mut current_dropins: Vec<std::path::PathBuf> = Vec::new();
    let dropin_dirname = format!("{}.d", unit.id.name);
    for dir in unit_dirs {
        let dropin_dir = dir.join(&dropin_dirname);
        if let Ok(entries) = std::fs::read_dir(&dropin_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().ends_with(".conf") {
                    current_dropins.push(entry.path());
                }
            }
        }
    }

    // Compare current drop-in set with what was loaded.
    let mut loaded_set: Vec<&std::path::Path> = cfg
        .loaded_dropin_files
        .iter()
        .map(|p| p.as_path())
        .collect();
    loaded_set.sort();
    current_dropins.sort();
    let current_set: Vec<&std::path::Path> = current_dropins.iter().map(|p| p.as_path()).collect();

    if loaded_set != current_set {
        return true;
    }

    // Check if any existing drop-in file has been modified since load.
    for path in &current_dropins {
        if let Ok(meta) = std::fs::metadata(path)
            && let Ok(mtime) = meta.modified()
            && mtime > loaded_at
        {
            return true;
        }
    }

    false
}

/// Compute the next calendar elapse time from a list of OnCalendar= specs.
/// Returns a human-readable timestamp like "Mon 2026-03-24 12:15:00 UTC".
fn compute_next_calendar_elapse(specs: &[String]) -> Option<String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    let mut earliest: Option<u64> = None;

    for spec in specs {
        if let Some(next) = next_calendar_event(spec, now) {
            earliest = Some(match earliest {
                Some(e) => e.min(next),
                None => next,
            });
        }
    }

    let next = earliest?;
    // Format as "Day YYYY-MM-DD HH:MM:SS UTC" using libc gmtime_r
    let ts = next as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&ts, &mut tm);
    }
    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let wday = weekdays.get(tm.tm_wday as usize).unwrap_or(&"???");
    Some(format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        wday,
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    ))
}

/// Parse a calendar event spec and find the next trigger time after `now` (unix seconds).
/// Supports a subset of systemd calendar event syntax:
///   - `*:0/15:0` — every 15 minutes
///   - `*-*-* HH:MM:SS` — daily at specific time
///   - `daily`, `weekly`, `monthly`, etc.
fn next_calendar_event(spec: &str, now: u64) -> Option<u64> {
    // Parse the spec: format is [DOW] [YYYY-MM-DD] [HH:MM:SS]
    // The most common pattern we need: `*:M/S:S` or `*:0/N:0`
    let spec = spec.trim();

    // Get current time components via gmtime_r
    let ts = now as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&ts, &mut tm);
    }

    // Simple pattern: try to parse the time part as H:M:S
    // The spec format varies: could be just time ("*:0/15:0") or date+time
    // For the test, we need to handle "*:0/15:0" which means every 15 min at second 0

    // Split by spaces — last part should be time
    let parts: Vec<&str> = spec.split_whitespace().collect();
    let time_part = parts.last()?;

    // Split time by ':'
    let time_fields: Vec<&str> = time_part.split(':').collect();
    if time_fields.len() != 3 {
        return None;
    }

    let (hour_spec, min_spec, sec_spec) = (time_fields[0], time_fields[1], time_fields[2]);

    // Parse second — typically "0" or "*"
    let sec: u32 = if sec_spec == "*" {
        0
    } else {
        sec_spec.parse().ok()?
    };

    // Parse minute — may be "0/15" meaning every 15 starting at 0
    let (min_start, min_step) = if let Some((start, step)) = min_spec.split_once('/') {
        let s = if start.is_empty() || start == "*" {
            0
        } else {
            start.parse::<u32>().ok()?
        };
        (s, step.parse::<u32>().ok()?)
    } else if min_spec == "*" {
        (0, 1) // every minute
    } else {
        let m = min_spec.parse::<u32>().ok()?;
        (m, 0) // exact minute, no repetition within the hour
    };

    // Parse hour — "*" means every hour
    let hour_any = hour_spec == "*";
    let hour_exact: Option<u32> = if hour_any {
        None
    } else {
        Some(hour_spec.parse().ok()?)
    };

    // Find the next occurrence
    let current_hour = tm.tm_hour as u32;
    let current_min = tm.tm_min as u32;
    let current_sec = tm.tm_sec as u32;

    // Try to find the next trigger within the next 24 hours
    for hour_offset in 0..25u32 {
        let h = if hour_any {
            (current_hour + hour_offset) % 24
        } else {
            let eh = hour_exact.unwrap();
            if hour_offset <= 1 {
                eh
            } else {
                break;
            }
        };

        if !hour_any && h != hour_exact.unwrap() {
            continue;
        }

        // Find the first minute that matches
        if min_step > 0 {
            let mut m = min_start;
            while m < 60 {
                let candidate_total = h * 3600 + m * 60 + sec;
                let current_total = if hour_offset > 0 && hour_any {
                    0 // past the current hour, any time is fine
                } else {
                    current_hour * 3600 + current_min * 60 + current_sec
                };
                let day_offset = if hour_any {
                    hour_offset / 24
                } else if hour_offset > 0 {
                    1
                } else {
                    0
                };
                let actual_current = if hour_any && hour_offset > 0 {
                    // We're in a future hour
                    let actual_h = (current_hour + hour_offset) % 24;
                    if actual_h < current_hour || (actual_h == current_hour && hour_offset > 0) {
                        // Wrapped to next day — any time works
                        0
                    } else {
                        current_hour * 3600 + current_min * 60 + current_sec
                    }
                } else {
                    current_total
                };

                if candidate_total > actual_current || day_offset > 0 {
                    // Build the unix timestamp
                    let mut next_tm = tm;
                    next_tm.tm_hour = h as libc::c_int;
                    next_tm.tm_min = m as libc::c_int;
                    next_tm.tm_sec = sec as libc::c_int;
                    // Add day offset
                    next_tm.tm_mday += day_offset as libc::c_int;
                    let result = unsafe { libc::timegm(&mut next_tm) };
                    if result >= 0 {
                        return Some(result as u64);
                    }
                }
                m += min_step;
            }
        } else {
            // Exact minute
            let candidate_total = h * 3600 + min_start * 60 + sec;
            let current_total = current_hour * 3600 + current_min * 60 + current_sec;
            let day_offset = if hour_any {
                if (current_hour + hour_offset) >= 24 {
                    1
                } else {
                    0
                }
            } else if hour_offset > 0 {
                1
            } else {
                0
            };
            if candidate_total > current_total || day_offset > 0 {
                let mut next_tm = tm;
                next_tm.tm_hour = h as libc::c_int;
                next_tm.tm_min = min_start as libc::c_int;
                next_tm.tm_sec = sec as libc::c_int;
                next_tm.tm_mday += day_offset as libc::c_int;
                let result = unsafe { libc::timegm(&mut next_tm) };
                if result >= 0 {
                    return Some(result as u64);
                }
            }
        }
    }

    None
}

/// Format properties as `Key=Value\n` lines, optionally filtered to a set of
/// property names.
pub fn format_properties(props: &PropertyMap, filter: Option<&[String]>) -> String {
    let mut out = String::new();
    // Always iterate in insertion order (which matches systemd's vtable
    // order).  When a filter is provided, skip properties not in the set.
    for (key, value) in props.iter() {
        let include = match filter {
            Some(keys) => keys.iter().any(|k| k.eq_ignore_ascii_case(key)),
            None => true,
        };
        if include {
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push('\n');
        }
    }
    out
}

/// Format properties as a JSON object (for the JSON-RPC transport).
pub fn properties_to_json(props: &PropertyMap) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in props.iter() {
        map.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    serde_json::Value::Object(map)
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn insert(props: &mut PropertyMap, key: &str, value: &str) {
    props.insert(key.to_string(), value.to_string());
}

fn format_rlimit_value(val: &crate::units::unit_parsing::RLimitValue) -> String {
    match val {
        crate::units::unit_parsing::RLimitValue::Value(n) => n.to_string(),
        crate::units::unit_parsing::RLimitValue::Infinity => "infinity".to_string(),
    }
}

fn insert_bool(props: &mut PropertyMap, key: &str, value: bool) {
    props.insert(
        key.to_string(),
        if value { "yes" } else { "no" }.to_string(),
    );
}

fn insert_dep_list(props: &mut PropertyMap, key: &str, ids: &[crate::units::UnitId]) {
    let value: String = ids
        .iter()
        .map(|id| id.name.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    props.insert(key.to_string(), value);
}

fn insert_string_list(props: &mut PropertyMap, key: &str, items: &[String]) {
    props.insert(key.to_string(), items.join(" "));
}

fn insert_timeout(props: &mut PropertyMap, key: &str, timeout: &Option<Timeout>) {
    let value = match timeout {
        Some(Timeout::Duration(d)) => format!("{}us", d.as_micros()),
        Some(Timeout::Infinity) => "infinity".to_string(),
        None => "infinity".to_string(),
    };
    props.insert(key.to_string(), value);
}

fn insert_commandlines(props: &mut PropertyMap, key: &str, cmds: &[Commandline]) {
    if cmds.is_empty() {
        props.insert(key.to_string(), String::new());
        return;
    }
    // systemctl show formats multi-command ExecStart as
    //   { path=/bin/foo ; argv[]=/bin/foo arg1 ; ignore_errors=no ; ... }
    let parts: Vec<String> = cmds
        .iter()
        .map(|cmd| {
            let ignore_errors = if cmd.prefixes.contains(&CommandlinePrefix::Minus) {
                "yes"
            } else {
                "no"
            };
            let mut s = String::new();
            s.push_str("{ path=");
            s.push_str(&cmd.cmd);
            s.push_str(" ; argv[]=");
            s.push_str(&cmd.cmd);
            for arg in &cmd.args {
                s.push(' ');
                s.push_str(arg);
            }
            s.push_str(" ; ignore_errors=");
            s.push_str(ignore_errors);
            s.push_str(" ; }");
            s
        })
        .collect();
    props.insert(key.to_string(), parts.join(" ; "));
}

/// Format exec commandlines for ExecXYZEx properties with flags instead of ignore_errors.
fn insert_commandlines_ex(props: &mut PropertyMap, key: &str, cmds: &[Commandline]) {
    if cmds.is_empty() {
        props.insert(key.to_string(), String::new());
        return;
    }
    let parts: Vec<String> = cmds
        .iter()
        .map(|cmd| {
            let mut flags = Vec::new();
            for prefix in &cmd.prefixes {
                match prefix {
                    CommandlinePrefix::Minus => flags.push("ignore-failure"),
                    CommandlinePrefix::Colon => flags.push("no-env-expand"),
                    CommandlinePrefix::Plus => flags.push("ambient"),
                    CommandlinePrefix::Exclamation => flags.push("no-setuid"),
                    CommandlinePrefix::DoubleExclamation => flags.push("sandbox"),
                    CommandlinePrefix::AtSign => flags.push("no-argv0"),
                }
            }
            let flags_str = if flags.is_empty() {
                String::new()
            } else {
                flags.join(" ")
            };
            let mut s = String::new();
            s.push_str("{ path=");
            s.push_str(&cmd.cmd);
            s.push_str(" ; argv[]=");
            s.push_str(&cmd.cmd);
            for arg in &cmd.args {
                s.push(' ');
                s.push_str(arg);
            }
            s.push_str(" ; flags=");
            s.push_str(&flags_str);
            s.push_str(" ; }");
            s
        })
        .collect();
    props.insert(key.to_string(), parts.join(" ; "));
}

fn insert_optional(props: &mut PropertyMap, key: &str, value: &Option<String>) {
    match value {
        Some(v) => props.insert(key.to_string(), v.clone()),
        None => props.insert(key.to_string(), String::new()),
    };
}

// ── Section inserters ────────────────────────────────────────────────────

fn insert_unit_config(props: &mut PropertyMap, conf: &UnitConfig) {
    insert(props, "Description", &conf.description);
    insert_string_list(props, "Documentation", &conf.documentation);

    match &conf.fragment_path {
        Some(p) => insert(props, "FragmentPath", &p.display().to_string()),
        None => insert(props, "FragmentPath", ""),
    }

    // SourcePath (same as FragmentPath for our purposes)
    match &conf.fragment_path {
        Some(p) => insert(props, "SourcePath", &p.display().to_string()),
        None => insert(props, "SourcePath", ""),
    }

    // Drop-in files
    if conf.loaded_dropin_files.is_empty() {
        insert(props, "DropInPaths", "");
    } else {
        let paths: Vec<String> = conf
            .loaded_dropin_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        insert(props, "DropInPaths", &paths.join(" "));
    }

    insert_bool(props, "DefaultDependencies", conf.default_dependencies);
    insert_bool(props, "RefuseManualStart", conf.refuse_manual_start);
    insert_bool(props, "RefuseManualStop", conf.refuse_manual_stop);
    insert_bool(props, "IgnoreOnIsolate", conf.ignore_on_isolate);
    insert_bool(props, "AllowIsolate", conf.allow_isolate);

    insert(
        props,
        "SuccessAction",
        format_unit_action(&conf.success_action),
    );
    insert(
        props,
        "FailureAction",
        format_unit_action(&conf.failure_action),
    );

    insert_timeout(props, "JobTimeoutUSec", &conf.job_timeout_sec);
    insert(
        props,
        "JobTimeoutAction",
        format_unit_action(&conf.job_timeout_action),
    );

    if !conf.on_success.is_empty() {
        insert_string_list(props, "OnSuccess", &conf.on_success);
    } else {
        insert(props, "OnSuccess", "");
    }
    if !conf.on_failure.is_empty() {
        insert_string_list(props, "OnFailure", &conf.on_failure);
    } else {
        insert(props, "OnFailure", "");
    }

    insert_timeout(
        props,
        "StartLimitIntervalUSec",
        &conf.start_limit_interval_sec,
    );
    match conf.start_limit_burst {
        Some(n) => insert(props, "StartLimitBurst", &n.to_string()),
        None => insert(props, "StartLimitBurst", "5"),
    }
    insert(
        props,
        "StartLimitAction",
        format_unit_action(&conf.start_limit_action),
    );

    if !conf.conditions.is_empty() {
        let cond_str: Vec<String> = conf.conditions.iter().map(|c| format!("{c:?}")).collect();
        insert(props, "Conditions", &cond_str.join(" "));
    }

    if !conf.assertions.is_empty() {
        let assert_str: Vec<String> = conf.assertions.iter().map(|a| format!("{a:?}")).collect();
        insert(props, "Asserts", &assert_str.join(" "));
    }
}

fn insert_status(props: &mut PropertyMap, status: &UnitStatus) {
    let (active_state, sub_state) = match status {
        UnitStatus::NeverStarted => ("inactive", "dead"),
        UnitStatus::Starting => ("activating", "start"),
        UnitStatus::Stopping => ("deactivating", "stop"),
        UnitStatus::Restarting => ("activating", "auto-restart"),
        UnitStatus::Started(s) => {
            let sub = match s {
                crate::units::StatusStarted::Running => "running",
                crate::units::StatusStarted::WaitingForSocket => "waiting",
            };
            ("active", sub)
        }
        UnitStatus::Stopped(_, errors) => {
            if errors.is_empty() {
                ("inactive", "dead")
            } else {
                ("failed", "failed")
            }
        }
    };
    insert(props, "ActiveState", active_state);
    insert(props, "SubState", sub_state);
}

fn insert_timestamps(props: &mut PropertyMap, unit: &Unit) {
    let ts = unit.common.timestamps.read_poisoned();
    let fmt = |v: Option<u64>| match v {
        Some(usec) => format_usec_timestamp(usec),
        None => "n/a".to_string(),
    };
    let fmt_usec = |v: Option<u64>| match v {
        Some(usec) => usec.to_string(),
        None => "0".to_string(),
    };
    insert(props, "InactiveExitTimestamp", &fmt(ts.inactive_exit));
    insert(
        props,
        "InactiveExitTimestampMonotonic",
        &fmt_usec(ts.inactive_exit),
    );
    insert(props, "ActiveEnterTimestamp", &fmt(ts.active_enter));
    insert(
        props,
        "ActiveEnterTimestampMonotonic",
        &fmt_usec(ts.active_enter),
    );
    insert(props, "ActiveExitTimestamp", &fmt(ts.active_exit));
    insert(
        props,
        "ActiveExitTimestampMonotonic",
        &fmt_usec(ts.active_exit),
    );
    insert(props, "InactiveEnterTimestamp", &fmt(ts.inactive_enter));
    insert(
        props,
        "InactiveEnterTimestampMonotonic",
        &fmt_usec(ts.inactive_enter),
    );
    insert(props, "StateChangeTimestamp", &fmt(ts.state_change));
    insert(
        props,
        "StateChangeTimestampMonotonic",
        &fmt_usec(ts.state_change),
    );
}

fn insert_service_config(props: &mut PropertyMap, conf: &ServiceConfig) {
    // Restart policy
    insert(props, "Restart", &format_restart(&conf.restart));
    match &conf.restart_sec {
        Some(Timeout::Duration(d)) => insert(props, "RestartUSec", &format!("{}us", d.as_micros())),
        Some(Timeout::Infinity) => insert(props, "RestartUSec", "infinity"),
        None => insert(props, "RestartUSec", "100000us"),
    }

    // Exec lines
    insert_commandlines(props, "ExecStart", &conf.exec);
    insert_commandlines_ex(props, "ExecStartEx", &conf.exec);
    insert_commandlines(props, "ExecCondition", &conf.exec_condition);
    insert_commandlines_ex(props, "ExecConditionEx", &conf.exec_condition);
    insert_commandlines(props, "ExecStartPre", &conf.startpre);
    insert_commandlines_ex(props, "ExecStartPreEx", &conf.startpre);
    insert_commandlines(props, "ExecStartPost", &conf.startpost);
    insert_commandlines_ex(props, "ExecStartPostEx", &conf.startpost);
    insert_commandlines(props, "ExecReload", &conf.reload);
    insert_commandlines_ex(props, "ExecReloadEx", &conf.reload);
    insert_commandlines(props, "ExecStop", &conf.stop);
    insert_commandlines_ex(props, "ExecStopEx", &conf.stop);
    insert_commandlines(props, "ExecStopPost", &conf.stoppost);
    insert_commandlines_ex(props, "ExecStopPostEx", &conf.stoppost);

    // Kill mode / signal
    insert(props, "KillMode", &format_kill_mode(conf.kill_mode));

    // Notify access
    insert(
        props,
        "NotifyAccess",
        &format_notify_access(conf.notifyaccess),
    );

    // Timeouts
    insert_timeout(props, "TimeoutStartUSec", &conf.starttimeout);
    insert_timeout(props, "TimeoutStopUSec", &conf.stoptimeout);

    // PID file
    match &conf.pid_file {
        Some(p) => insert(props, "PIDFile", &p.display().to_string()),
        None => insert(props, "PIDFile", ""),
    }

    // Sockets
    if !conf.sockets.is_empty() {
        let sock_names: Vec<String> = conf.sockets.iter().map(|s| s.name.clone()).collect();
        insert(props, "Sockets", &sock_names.join(" "));
    }

    // Slice
    insert_optional(props, "Slice", &conf.slice);

    // Remain after exit
    insert_bool(props, "RemainAfterExit", conf.remain_after_exit);

    // D-Bus name
    insert_optional(props, "BusName", &conf.dbus_name);

    // Delegate
    insert_bool(
        props,
        "Delegate",
        !matches!(conf.delegate, crate::units::Delegate::No),
    );

    // Watchdog
    insert_timeout(props, "WatchdogUSec", &conf.watchdog_sec);

    // RuntimeMaxUSec
    insert_timeout(props, "RuntimeMaxUSec", &conf.runtime_max_sec);

    // TasksMax
    match &conf.tasks_max {
        Some(crate::units::TasksMax::Value(n)) => insert(props, "TasksMax", &n.to_string()),
        Some(crate::units::TasksMax::Infinity) => insert(props, "TasksMax", "infinity"),
        Some(crate::units::TasksMax::Percent(p)) => insert(props, "TasksMax", &format!("{p}%")),
        None => insert(props, "TasksMax", "infinity"),
    }

    // Resource limits (LimitNOFILE, etc.)
    if let Some(ref rl) = conf.limit_nofile {
        insert(props, "LimitNOFILE", &format_rlimit_value(&rl.hard));
        insert(props, "LimitNOFILESoft", &format_rlimit_value(&rl.soft));
    }
}

fn insert_exec_config(props: &mut PropertyMap, conf: &ExecConfig) {
    insert_optional(props, "User", &conf.user);
    insert_optional(props, "Group", &conf.group);

    if !conf.supplementary_groups.is_empty() {
        insert(
            props,
            "SupplementaryGroups",
            &conf.supplementary_groups.join(" "),
        );
    }

    match &conf.working_directory {
        Some(p) => insert(props, "WorkingDirectory", &p.display().to_string()),
        None => insert(props, "WorkingDirectory", ""),
    }

    // Environment variables
    if let Some(env) = &conf.environment
        && !env.vars.is_empty()
    {
        let env_strs: Vec<String> = env.vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
        insert(props, "Environment", &env_strs.join(" "));
    }

    // Environment files
    if !conf.environment_files.is_empty() {
        insert(
            props,
            "EnvironmentFile",
            &conf
                .environment_files
                .iter()
                .map(|(path, optional)| {
                    if *optional {
                        format!("-{}", path.display())
                    } else {
                        path.display().to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    // Directories
    if !conf.state_directory.is_empty() {
        insert(props, "StateDirectory", &conf.state_directory.join(" "));
    }
    if !conf.logs_directory.is_empty() {
        insert(props, "LogsDirectory", &conf.logs_directory.join(" "));
    }
    if !conf.runtime_directory.is_empty() {
        insert(props, "RuntimeDirectory", &conf.runtime_directory.join(" "));
    }

    // Security settings
    insert_bool(props, "NoNewPrivileges", conf.no_new_privileges);
    insert_bool(props, "ProtectKernelModules", conf.protect_kernel_modules);
    insert_bool(props, "ProtectKernelTunables", conf.protect_kernel_tunables);
    insert_bool(props, "ProtectKernelLogs", conf.protect_kernel_logs);
    insert_bool(props, "ProtectControlGroups", conf.protect_control_groups);
    insert_bool(props, "ProtectClock", conf.protect_clock);
    insert_bool(props, "ProtectHostname", conf.protect_hostname);
    insert_bool(props, "PrivateTmp", conf.private_tmp);
    insert_bool(props, "PrivateDevices", conf.private_devices);
    insert_bool(props, "PrivateNetwork", conf.private_network);
    insert_bool(props, "PrivateUsers", conf.private_users);
    insert_bool(props, "PrivateMounts", conf.private_mounts);
    insert_bool(
        props,
        "MemoryDenyWriteExecute",
        conf.memory_deny_write_execute,
    );
    insert_bool(props, "LockPersonality", conf.lock_personality);
    insert_bool(props, "RestrictSUIDSGID", conf.restrict_suid_sgid);
    insert_bool(props, "RestrictRealtime", conf.restrict_realtime);
    insert_bool(props, "RemoveIPC", conf.remove_ipc);
    insert_bool(props, "DynamicUser", conf.dynamic_user);

    match &conf.protect_system {
        crate::units::ProtectSystem::No => insert(props, "ProtectSystem", "no"),
        crate::units::ProtectSystem::Yes => insert(props, "ProtectSystem", "yes"),
        crate::units::ProtectSystem::Full => insert(props, "ProtectSystem", "full"),
        crate::units::ProtectSystem::Strict => insert(props, "ProtectSystem", "strict"),
    }

    match &conf.protect_home {
        crate::units::ProtectHome::No => insert(props, "ProtectHome", "no"),
        crate::units::ProtectHome::Yes => insert(props, "ProtectHome", "yes"),
        crate::units::ProtectHome::ReadOnly => insert(props, "ProtectHome", "read-only"),
        crate::units::ProtectHome::Tmpfs => insert(props, "ProtectHome", "tmpfs"),
    }

    // OOMScoreAdjust
    if let Some(adj) = conf.oom_score_adjust {
        insert(props, "OOMScoreAdjust", &adj.to_string());
    }

    // Nice
    if let Some(nice) = conf.nice {
        insert(props, "Nice", &nice.to_string());
    }

    // UMask
    if let Some(umask) = conf.umask {
        insert(props, "UMask", &format!("{:04o}", umask));
    }

    // PAM name
    insert_optional(props, "PAMName", &conf.pam_name);

    // Credential directives
    if !conf.import_credentials.is_empty() {
        insert(
            props,
            "ImportCredential",
            &conf.import_credentials.join(" "),
        );
    }
    if !conf.load_credentials.is_empty() {
        let lc: Vec<String> = conf
            .load_credentials
            .iter()
            .map(|(id, path)| format!("{id}:{path}"))
            .collect();
        insert(props, "LoadCredential", &lc.join(" "));
    }
    if !conf.set_credentials.is_empty() {
        let sc: Vec<String> = conf
            .set_credentials
            .iter()
            .map(|(id, _data)| id.clone())
            .collect();
        insert(props, "SetCredential", &sc.join(" "));
    }

    // Capability sets
    if !conf.capability_bounding_set.is_empty() {
        insert(
            props,
            "CapabilityBoundingSet",
            &conf.capability_bounding_set.join(" "),
        );
    }
    if !conf.ambient_capabilities.is_empty() {
        insert(
            props,
            "AmbientCapabilities",
            &conf.ambient_capabilities.join(" "),
        );
    }

    // System call filter
    if !conf.system_call_filter.is_empty() {
        insert(
            props,
            "SystemCallFilter",
            &conf.system_call_filter.join(" "),
        );
    }
    // System call log
    if !conf.system_call_log.is_empty() {
        insert(props, "SystemCallLog", &conf.system_call_log.join(" "));
    }
    if !conf.system_call_architectures.is_empty() {
        insert(
            props,
            "SystemCallArchitectures",
            &conf.system_call_architectures.join(" "),
        );
    }

    // Read-write paths
    if !conf.read_write_paths.is_empty() {
        insert(props, "ReadWritePaths", &conf.read_write_paths.join(" "));
    }

    // Restrict address families
    if !conf.restrict_address_families.is_empty() {
        insert(
            props,
            "RestrictAddressFamilies",
            &conf.restrict_address_families.join(" "),
        );
    }

    // Restrict file systems
    if !conf.restrict_file_systems.is_empty() {
        insert(
            props,
            "RestrictFileSystems",
            &conf.restrict_file_systems.join(" "),
        );
    }

    // IP address allow/deny (from ServiceConfig but stored in ExecConfig vicinity)
    // These are on ServiceConfig, not ExecConfig, so handled in insert_service_config.
}

fn insert_socket_config(props: &mut PropertyMap, conf: &SocketConfig) {
    // List listen addresses
    let mut listen_items = Vec::new();
    for sock in &conf.sockets {
        let desc = match &sock.specialized {
            crate::sockets::SpecializedSocketConfig::UnixSocket(unix) => {
                let addr = match unix {
                    crate::sockets::UnixSocketConfig::Stream(s) => s,
                    crate::sockets::UnixSocketConfig::Sequential(s) => s,
                    crate::sockets::UnixSocketConfig::Datagram(s) => s,
                };
                format!("unix:{addr}")
            }
            crate::sockets::SpecializedSocketConfig::Fifo(fifo) => {
                format!("fifo:{}", fifo.path.display())
            }
            crate::sockets::SpecializedSocketConfig::TcpSocket(tcp) => {
                format!("tcp:{}", tcp.addr)
            }
            crate::sockets::SpecializedSocketConfig::UdpSocket(udp) => {
                format!("udp:{}", udp.addr)
            }
            crate::sockets::SpecializedSocketConfig::NetlinkSocket(nl) => {
                format!("netlink:{}", nl.family)
            }
            crate::sockets::SpecializedSocketConfig::SpecialFile(sp) => {
                format!("special:{}", sp.path.display())
            }
        };
        listen_items.push(desc);
    }
    if !listen_items.is_empty() {
        insert(props, "Listen", &listen_items.join(" "));
    }

    // Services triggered by this socket
    let srvc_names: Vec<String> = conf.services.iter().map(|s| s.name.clone()).collect();
    if !srvc_names.is_empty() {
        insert(props, "TriggerService", &srvc_names.join(" "));
    }

    insert_bool(props, "Accept", conf.accept);
    insert(props, "MaxConnections", &conf.max_connections.to_string());
    insert(
        props,
        "MaxConnectionsPerSource",
        &conf.max_connections_per_source.to_string(),
    );
    match conf.socket_mode {
        Some(mode) => insert(props, "SocketMode", &format!("{mode:04o}")),
        None => insert(props, "SocketMode", "0666"),
    }
    match conf.directory_mode {
        Some(mode) => insert(props, "DirectoryMode", &format!("{mode:04o}")),
        None => insert(props, "DirectoryMode", "0755"),
    }
    insert_bool(props, "PassCredentials", conf.pass_credentials);
    insert_bool(props, "PassSecurity", conf.pass_security);
    insert_bool(props, "PassPacketInfo", conf.pass_packet_info);
    insert_bool(props, "RemoveOnStop", conf.remove_on_stop);
    insert_bool(props, "Writable", conf.writable);
    if !conf.filedesc_name.is_empty() {
        insert(props, "FileDescriptorName", &conf.filedesc_name);
    }
    if let Some(ref tc) = conf.tcp_congestion {
        insert(props, "TCPCongestion", tc);
    }
    if !conf.exec_start_pre.is_empty() {
        let cmds: Vec<String> = conf.exec_start_pre.iter().map(format_commandline).collect();
        insert(props, "ExecStartPre", &cmds.join(" ; "));
    }
    if !conf.exec_start_post.is_empty() {
        let cmds: Vec<String> = conf
            .exec_start_post
            .iter()
            .map(format_commandline)
            .collect();
        insert(props, "ExecStartPost", &cmds.join(" ; "));
    }
    if !conf.exec_stop_pre.is_empty() {
        let cmds: Vec<String> = conf.exec_stop_pre.iter().map(format_commandline).collect();
        insert(props, "ExecStopPre", &cmds.join(" ; "));
    }
    if !conf.exec_stop_post.is_empty() {
        let cmds: Vec<String> = conf.exec_stop_post.iter().map(format_commandline).collect();
        insert(props, "ExecStopPost", &cmds.join(" ; "));
    }
    match &conf.timeout_sec {
        Some(crate::units::Timeout::Duration(d)) => {
            insert(props, "TimeoutUSec", &format!("{}us", d.as_micros()));
        }
        Some(crate::units::Timeout::Infinity) => {
            insert(props, "TimeoutUSec", "infinity");
        }
        None => {}
    }
    insert_bool(
        props,
        "PassFileDescriptorsToExec",
        conf.pass_file_descriptors_to_exec,
    );
}

fn format_commandline(cmd: &crate::units::Commandline) -> String {
    let mut s = String::new();
    for p in &cmd.prefixes {
        match p {
            crate::units::CommandlinePrefix::Minus => s.push('-'),
            crate::units::CommandlinePrefix::AtSign => s.push('@'),
            crate::units::CommandlinePrefix::Plus => s.push('+'),
            crate::units::CommandlinePrefix::Colon => s.push(':'),
            crate::units::CommandlinePrefix::Exclamation => s.push('!'),
            crate::units::CommandlinePrefix::DoubleExclamation => s.push_str("!!"),
        }
    }
    s.push_str(&cmd.cmd);
    for arg in &cmd.args {
        s.push(' ');
        s.push_str(arg);
    }
    s
}

fn insert_slice_config(props: &mut PropertyMap, conf: &SliceConfig) {
    // Memory limits
    match &conf.memory_min {
        Some(crate::units::MemoryLimit::Bytes(n)) => insert(props, "MemoryMin", &n.to_string()),
        Some(crate::units::MemoryLimit::Percent(p)) => insert(props, "MemoryMin", &format!("{p}%")),
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemoryMin", "infinity"),
        None => insert(props, "MemoryMin", ""),
    }
    match &conf.memory_low {
        Some(crate::units::MemoryLimit::Bytes(n)) => insert(props, "MemoryLow", &n.to_string()),
        Some(crate::units::MemoryLimit::Percent(p)) => insert(props, "MemoryLow", &format!("{p}%")),
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemoryLow", "infinity"),
        None => insert(props, "MemoryLow", ""),
    }
    match &conf.memory_high {
        Some(crate::units::MemoryLimit::Bytes(n)) => insert(props, "MemoryHigh", &n.to_string()),
        Some(crate::units::MemoryLimit::Percent(p)) => {
            insert(props, "MemoryHigh", &format!("{p}%"))
        }
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemoryHigh", "infinity"),
        None => insert(props, "MemoryHigh", "infinity"),
    }
    match &conf.memory_max {
        Some(crate::units::MemoryLimit::Bytes(n)) => insert(props, "MemoryMax", &n.to_string()),
        Some(crate::units::MemoryLimit::Percent(p)) => insert(props, "MemoryMax", &format!("{p}%")),
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemoryMax", "infinity"),
        None => insert(props, "MemoryMax", "infinity"),
    }
    match &conf.memory_swap_max {
        Some(crate::units::MemoryLimit::Bytes(n)) => insert(props, "MemorySwapMax", &n.to_string()),
        Some(crate::units::MemoryLimit::Percent(p)) => {
            insert(props, "MemorySwapMax", &format!("{p}%"))
        }
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemorySwapMax", "infinity"),
        None => insert(props, "MemorySwapMax", "infinity"),
    }
    match &conf.memory_zswap_max {
        Some(crate::units::MemoryLimit::Bytes(n)) => {
            insert(props, "MemoryZSwapMax", &n.to_string())
        }
        Some(crate::units::MemoryLimit::Percent(p)) => {
            insert(props, "MemoryZSwapMax", &format!("{p}%"))
        }
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "MemoryZSwapMax", "infinity"),
        None => insert(props, "MemoryZSwapMax", "infinity"),
    }
    match &conf.default_memory_min {
        Some(crate::units::MemoryLimit::Bytes(n)) => {
            insert(props, "DefaultMemoryMin", &n.to_string())
        }
        Some(crate::units::MemoryLimit::Percent(p)) => {
            insert(props, "DefaultMemoryMin", &format!("{p}%"))
        }
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "DefaultMemoryMin", "infinity"),
        None => insert(props, "DefaultMemoryMin", ""),
    }
    match &conf.default_memory_low {
        Some(crate::units::MemoryLimit::Bytes(n)) => {
            insert(props, "DefaultMemoryLow", &n.to_string())
        }
        Some(crate::units::MemoryLimit::Percent(p)) => {
            insert(props, "DefaultMemoryLow", &format!("{p}%"))
        }
        Some(crate::units::MemoryLimit::Infinity) => insert(props, "DefaultMemoryLow", "infinity"),
        None => insert(props, "DefaultMemoryLow", ""),
    }

    // CPU
    match conf.cpu_weight {
        Some(n) => insert(props, "CPUWeight", &n.to_string()),
        None => insert(props, "CPUWeight", ""),
    }
    match conf.startup_cpu_weight {
        Some(n) => insert(props, "StartupCPUWeight", &n.to_string()),
        None => insert(props, "StartupCPUWeight", ""),
    }
    match conf.cpu_quota {
        Some(p) => insert(props, "CPUQuota", &format!("{p}%")),
        None => insert(props, "CPUQuota", ""),
    }
    insert_timeout(props, "CPUQuotaPeriodUSec", &conf.cpu_quota_period_sec);
    insert_optional(props, "AllowedCPUs", &conf.allowed_cpus);
    insert_optional(props, "StartupAllowedCPUs", &conf.startup_allowed_cpus);
    insert_optional(props, "AllowedMemoryNodes", &conf.allowed_memory_nodes);
    insert_optional(
        props,
        "StartupAllowedMemoryNodes",
        &conf.startup_allowed_memory_nodes,
    );

    // IO
    match conf.io_weight {
        Some(n) => insert(props, "IOWeight", &n.to_string()),
        None => insert(props, "IOWeight", ""),
    }
    match conf.startup_io_weight {
        Some(n) => insert(props, "StartupIOWeight", &n.to_string()),
        None => insert(props, "StartupIOWeight", ""),
    }
    // Per-device IO limits are formatted as "DEVICE VALUE" entries
    if !conf.io_device_weight.is_empty() {
        let vals: Vec<String> = conf
            .io_device_weight
            .iter()
            .map(|d| format!("{} {}", d.device, d.value))
            .collect();
        insert(props, "IODeviceWeight", &vals.join(" "));
    }
    if !conf.io_read_bandwidth_max.is_empty() {
        let vals: Vec<String> = conf
            .io_read_bandwidth_max
            .iter()
            .map(|d| format!("{} {}", d.device, d.value))
            .collect();
        insert(props, "IOReadBandwidthMax", &vals.join(" "));
    }
    if !conf.io_write_bandwidth_max.is_empty() {
        let vals: Vec<String> = conf
            .io_write_bandwidth_max
            .iter()
            .map(|d| format!("{} {}", d.device, d.value))
            .collect();
        insert(props, "IOWriteBandwidthMax", &vals.join(" "));
    }
    if !conf.io_read_iops_max.is_empty() {
        let vals: Vec<String> = conf
            .io_read_iops_max
            .iter()
            .map(|d| format!("{} {}", d.device, d.value))
            .collect();
        insert(props, "IOReadIOPSMax", &vals.join(" "));
    }
    if !conf.io_write_iops_max.is_empty() {
        let vals: Vec<String> = conf
            .io_write_iops_max
            .iter()
            .map(|d| format!("{} {}", d.device, d.value))
            .collect();
        insert(props, "IOWriteIOPSMax", &vals.join(" "));
    }
    if !conf.io_device_latency_target_sec.is_empty() {
        insert(
            props,
            "IODeviceLatencyTargetSec",
            &conf.io_device_latency_target_sec.join(" "),
        );
    }

    // Tasks
    match &conf.tasks_max {
        Some(crate::units::TasksMax::Value(n)) => insert(props, "TasksMax", &n.to_string()),
        Some(crate::units::TasksMax::Infinity) => insert(props, "TasksMax", "infinity"),
        Some(crate::units::TasksMax::Percent(p)) => insert(props, "TasksMax", &format!("{p}%")),
        None => insert(props, "TasksMax", "infinity"),
    }

    // Accounting toggles
    match conf.cpu_accounting {
        Some(v) => insert_bool(props, "CPUAccounting", v),
        None => insert(props, "CPUAccounting", "no"),
    }
    match conf.memory_accounting {
        Some(v) => insert_bool(props, "MemoryAccounting", v),
        None => insert(props, "MemoryAccounting", "yes"),
    }
    match conf.io_accounting {
        Some(v) => insert_bool(props, "IOAccounting", v),
        None => insert(props, "IOAccounting", "no"),
    }
    match conf.tasks_accounting {
        Some(v) => insert_bool(props, "TasksAccounting", v),
        None => insert(props, "TasksAccounting", "yes"),
    }

    // Delegate
    match &conf.delegate {
        crate::units::Delegate::Yes => insert(props, "Delegate", "yes"),
        crate::units::Delegate::No => insert(props, "Delegate", "no"),
        crate::units::Delegate::Controllers(c) => insert(props, "Delegate", &c.join(" ")),
    }
    insert_optional(props, "DelegateSubgroup", &conf.delegate_subgroup);

    // Device access control
    if !conf.device_allow.is_empty() {
        insert(props, "DeviceAllow", &conf.device_allow.join(" "));
    }
    insert(
        props,
        "DevicePolicy",
        match conf.device_policy {
            crate::units::DevicePolicy::Auto => "auto",
            crate::units::DevicePolicy::Closed => "closed",
            crate::units::DevicePolicy::Strict => "strict",
        },
    );

    // IP address filtering
    if !conf.ip_address_allow.is_empty() {
        insert(props, "IPAddressAllow", &conf.ip_address_allow.join(" "));
    }
    if !conf.ip_address_deny.is_empty() {
        insert(props, "IPAddressDeny", &conf.ip_address_deny.join(" "));
    }

    // BPF / network filtering
    insert_string_list(props, "IPIngressFilterPath", &conf.ip_ingress_filter_path);
    insert_string_list(props, "IPEgressFilterPath", &conf.ip_egress_filter_path);
    insert_string_list(props, "BPFProgram", &conf.bpf_program);
    insert_string_list(props, "SocketBindAllow", &conf.socket_bind_allow);
    insert_string_list(props, "SocketBindDeny", &conf.socket_bind_deny);
    insert_string_list(
        props,
        "RestrictNetworkInterfaces",
        &conf.restrict_network_interfaces,
    );
    insert_string_list(props, "NFTSet", &conf.nft_set);
    insert_string_list(props, "DisableControllers", &conf.disable_controllers);

    // Managed OOM
    insert_optional(props, "ManagedOOMSwap", &conf.managed_oom_swap);
    insert_optional(
        props,
        "ManagedOOMMemoryPressure",
        &conf.managed_oom_memory_pressure,
    );
    insert_optional(
        props,
        "ManagedOOMMemoryPressureLimit",
        &conf.managed_oom_memory_pressure_limit,
    );
    insert_optional(props, "ManagedOOMPreference", &conf.managed_oom_preference);

    // Memory pressure
    insert(
        props,
        "MemoryPressureWatch",
        match conf.memory_pressure_watch {
            crate::units::MemoryPressureWatch::Auto => "auto",
            crate::units::MemoryPressureWatch::On => "on",
            crate::units::MemoryPressureWatch::Off => "off",
            crate::units::MemoryPressureWatch::Skip => "skip",
        },
    );
    insert_timeout(
        props,
        "MemoryPressureThresholdUSec",
        &conf.memory_pressure_threshold_sec,
    );
}

fn insert_swap_config(props: &mut PropertyMap, conf: &SwapConfig) {
    insert(props, "What", &conf.what);
    match conf.priority {
        Some(p) => insert(props, "Priority", &p.to_string()),
        None => insert(props, "Priority", "-1"),
    }
    insert_optional(props, "Options", &conf.options);
    match conf.timeout_sec {
        Some(secs) => insert(props, "TimeoutUSec", &format!("{}us", secs * 1_000_000)),
        None => insert(props, "TimeoutUSec", "infinity"),
    }
}

fn insert_mount_config(props: &mut PropertyMap, conf: &MountConfig) {
    insert(props, "What", &conf.what);
    insert(props, "Where", &conf.where_);
    insert_optional(props, "Type", &conf.fs_type);
    insert_optional(props, "Options", &conf.options);
    insert_bool(props, "SloppyOptions", conf.sloppy_options);
    insert_bool(props, "LazyUnmount", conf.lazy_unmount);
    insert_bool(props, "ReadWriteOnly", conf.read_write_only);
    insert_bool(props, "ForceUnmount", conf.force_unmount);
    insert(
        props,
        "DirectoryMode",
        &format!("{:04o}", conf.directory_mode),
    );
    match conf.timeout_sec {
        Some(secs) => insert(props, "TimeoutUSec", &format!("{}us", secs * 1_000_000)),
        None => insert(props, "TimeoutUSec", "infinity"),
    }
}

// ── Enum formatters ──────────────────────────────────────────────────────

fn format_service_type(t: ServiceType) -> String {
    match t {
        ServiceType::Simple => "simple",
        ServiceType::Exec => "exec",
        ServiceType::Forking => "forking",
        ServiceType::OneShot => "oneshot",
        ServiceType::Dbus => "dbus",
        ServiceType::Notify => "notify",
        ServiceType::NotifyReload => "notify-reload",
        ServiceType::Idle => "idle",
    }
    .to_owned()
}

fn format_restart(r: &ServiceRestart) -> String {
    match r {
        ServiceRestart::Always => "always",
        ServiceRestart::No => "no",
        ServiceRestart::OnFailure => "on-failure",
        ServiceRestart::OnSuccess => "on-success",
        ServiceRestart::OnWatchdog => "on-watchdog",
        ServiceRestart::OnAbnormal => "on-abnormal",
        ServiceRestart::OnAbort => "on-abort",
    }
    .to_owned()
}

fn format_kill_mode(m: KillMode) -> String {
    match m {
        KillMode::ControlGroup => "control-group",
        KillMode::Process => "process",
        KillMode::Mixed => "mixed",
        KillMode::None => "none",
    }
    .to_owned()
}

fn format_unit_action(a: &crate::units::UnitAction) -> &'static str {
    use crate::units::UnitAction;
    match a {
        UnitAction::None => "none",
        UnitAction::Exit => "exit",
        UnitAction::ExitForce => "exit-force",
        UnitAction::Reboot => "reboot",
        UnitAction::RebootForce => "reboot-force",
        UnitAction::RebootImmediate => "reboot-immediate",
        UnitAction::Poweroff => "poweroff",
        UnitAction::PoweroffForce => "poweroff-force",
        UnitAction::PoweroffImmediate => "poweroff-immediate",
        UnitAction::Halt => "halt",
        UnitAction::HaltForce => "halt-force",
        UnitAction::HaltImmediate => "halt-immediate",
        UnitAction::Kexec => "kexec",
        UnitAction::KexecForce => "kexec-force",
        UnitAction::KexecImmediate => "kexec-immediate",
    }
}

fn format_notify_access(n: NotifyKind) -> String {
    match n {
        NotifyKind::None => "none",
        NotifyKind::Main => "main",
        NotifyKind::All => "all",
        NotifyKind::Exec => "exec",
    }
    .to_owned()
}

/// Format a microsecond timestamp as "Day YYYY-MM-DD HH:MM:SS UTC".
fn format_usec_timestamp(usec: u64) -> String {
    let secs = (usec / 1_000_000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&secs, &mut tm);
    }
    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let wday = weekdays.get(tm.tm_wday as usize).unwrap_or(&"???");
    format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        wday,
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_properties_all() {
        let mut props = PropertyMap::new();
        props.insert("ActiveState".to_owned(), "active".to_owned());
        props.insert("Description".to_owned(), "Test Unit".to_owned());
        props.insert("Id".to_owned(), "test.service".to_owned());

        let output = format_properties(&props, None);
        assert!(output.contains("ActiveState=active\n"));
        assert!(output.contains("Description=Test Unit\n"));
        assert!(output.contains("Id=test.service\n"));
    }

    #[test]
    fn test_format_properties_filtered() {
        let mut props = PropertyMap::new();
        props.insert("ActiveState".to_owned(), "active".to_owned());
        props.insert("Description".to_owned(), "Test Unit".to_owned());
        props.insert("Id".to_owned(), "test.service".to_owned());

        let filter = vec!["Id".to_owned(), "ActiveState".to_owned()];
        let output = format_properties(&props, Some(&filter));
        assert!(output.contains("ActiveState=active\n"));
        assert!(output.contains("Id=test.service\n"));
        assert!(!output.contains("Description="));
    }

    #[test]
    fn test_format_properties_filter_case_insensitive() {
        let mut props = PropertyMap::new();
        props.insert("ActiveState".to_owned(), "active".to_owned());

        let filter = vec!["activestate".to_owned()];
        let output = format_properties(&props, Some(&filter));
        assert!(output.contains("ActiveState=active\n"));
    }

    #[test]
    fn test_format_properties_empty_filter() {
        let mut props = PropertyMap::new();
        props.insert("Id".to_owned(), "test.service".to_owned());

        let filter: Vec<String> = vec![];
        let output = format_properties(&props, Some(&filter));
        assert!(output.is_empty());
    }

    #[test]
    fn test_format_service_type() {
        assert_eq!(format_service_type(ServiceType::Simple), "simple");
        assert_eq!(format_service_type(ServiceType::Forking), "forking");
        assert_eq!(format_service_type(ServiceType::OneShot), "oneshot");
        assert_eq!(format_service_type(ServiceType::Notify), "notify");
        assert_eq!(format_service_type(ServiceType::Dbus), "dbus");
        assert_eq!(format_service_type(ServiceType::Idle), "idle");
        assert_eq!(format_service_type(ServiceType::Exec), "exec");
        assert_eq!(
            format_service_type(ServiceType::NotifyReload),
            "notify-reload"
        );
    }

    #[test]
    fn test_format_restart() {
        assert_eq!(format_restart(&ServiceRestart::Always), "always");
        assert_eq!(format_restart(&ServiceRestart::No), "no");
        assert_eq!(format_restart(&ServiceRestart::OnFailure), "on-failure");
    }

    #[test]
    fn test_format_kill_mode() {
        assert_eq!(format_kill_mode(KillMode::ControlGroup), "control-group");
        assert_eq!(format_kill_mode(KillMode::Process), "process");
        assert_eq!(format_kill_mode(KillMode::Mixed), "mixed");
        assert_eq!(format_kill_mode(KillMode::None), "none");
    }

    #[test]
    fn test_format_notify_access() {
        assert_eq!(format_notify_access(NotifyKind::None), "none");
        assert_eq!(format_notify_access(NotifyKind::Main), "main");
        assert_eq!(format_notify_access(NotifyKind::All), "all");
        assert_eq!(format_notify_access(NotifyKind::Exec), "exec");
    }

    #[test]
    fn test_insert_bool() {
        let mut props = PropertyMap::new();
        insert_bool(&mut props, "TestYes", true);
        insert_bool(&mut props, "TestNo", false);
        assert_eq!(props.get("TestYes").unwrap(), "yes");
        assert_eq!(props.get("TestNo").unwrap(), "no");
    }

    #[test]
    fn test_insert_timeout_duration() {
        let mut props = PropertyMap::new();
        insert_timeout(
            &mut props,
            "TestTimeout",
            &Some(Timeout::Duration(std::time::Duration::from_secs(30))),
        );
        assert_eq!(props.get("TestTimeout").unwrap(), "30000000us");
    }

    #[test]
    fn test_insert_timeout_infinity() {
        let mut props = PropertyMap::new();
        insert_timeout(&mut props, "TestTimeout", &Some(Timeout::Infinity));
        assert_eq!(props.get("TestTimeout").unwrap(), "infinity");
    }

    #[test]
    fn test_insert_timeout_none() {
        let mut props = PropertyMap::new();
        insert_timeout(&mut props, "TestTimeout", &None);
        assert_eq!(props.get("TestTimeout").unwrap(), "infinity");
    }

    #[test]
    fn test_insert_dep_list() {
        use crate::units::{UnitId, UnitIdKind};
        let mut props = PropertyMap::new();
        let deps = vec![
            UnitId {
                kind: UnitIdKind::Target,
                name: "multi-user.target".to_owned(),
            },
            UnitId {
                kind: UnitIdKind::Service,
                name: "sshd.service".to_owned(),
            },
        ];
        insert_dep_list(&mut props, "After", &deps);
        assert_eq!(
            props.get("After").unwrap(),
            "multi-user.target sshd.service"
        );
    }

    #[test]
    fn test_insert_dep_list_empty() {
        let mut props = PropertyMap::new();
        insert_dep_list(&mut props, "Before", &[]);
        assert_eq!(props.get("Before").unwrap(), "");
    }

    #[test]
    fn test_properties_to_json() {
        let mut props = PropertyMap::new();
        props.insert("Id".to_owned(), "test.service".to_owned());
        props.insert("ActiveState".to_owned(), "active".to_owned());

        let json = properties_to_json(&props);
        assert_eq!(json["Id"], "test.service");
        assert_eq!(json["ActiveState"], "active");
    }

    #[test]
    fn test_insert_commandlines_empty() {
        let mut props = PropertyMap::new();
        insert_commandlines(&mut props, "ExecStart", &[]);
        assert_eq!(props.get("ExecStart").unwrap(), "");
    }

    #[test]
    fn test_insert_commandlines_single() {
        let mut props = PropertyMap::new();
        let cmds = vec![Commandline {
            cmd: "/usr/bin/foo".to_owned(),
            args: vec!["--bar".to_owned(), "baz".to_owned()],
            prefixes: vec![],
        }];
        insert_commandlines(&mut props, "ExecStart", &cmds);
        let val = props.get("ExecStart").unwrap();
        assert!(val.contains("path=/usr/bin/foo"));
        assert!(val.contains("--bar baz"));
    }

    #[test]
    fn test_insert_optional_some() {
        let mut props = PropertyMap::new();
        insert_optional(&mut props, "PIDFile", &Some("/run/foo.pid".to_owned()));
        assert_eq!(props.get("PIDFile").unwrap(), "/run/foo.pid");
    }

    #[test]
    fn test_insert_optional_none() {
        let mut props = PropertyMap::new();
        insert_optional(&mut props, "PIDFile", &None);
        assert_eq!(props.get("PIDFile").unwrap(), "");
    }

    #[test]
    fn test_format_properties_ordering() {
        // PropertyMap preserves insertion order (matching systemd vtable order)
        let mut props = PropertyMap::new();
        props.insert("Zebra".to_owned(), "z".to_owned());
        props.insert("Alpha".to_owned(), "a".to_owned());
        props.insert("Middle".to_owned(), "m".to_owned());

        let output = format_properties(&props, None);
        let lines: Vec<&str> = output.trim().split('\n').collect();
        assert_eq!(lines[0], "Zebra=z");
        assert_eq!(lines[1], "Alpha=a");
        assert_eq!(lines[2], "Middle=m");
    }

    // ── Slice config property tests ──────────────────────────────────

    fn default_slice_config() -> SliceConfig {
        SliceConfig {
            memory_min: None,
            memory_low: None,
            memory_high: None,
            memory_max: None,
            memory_swap_max: None,
            cpu_weight: None,
            startup_cpu_weight: None,
            cpu_quota: None,
            io_weight: None,
            startup_io_weight: None,
            io_device_weight: Vec::new(),
            io_read_bandwidth_max: Vec::new(),
            io_write_bandwidth_max: Vec::new(),
            io_read_iops_max: Vec::new(),
            io_write_iops_max: Vec::new(),
            tasks_max: None,
            delegate: crate::units::Delegate::No,
            cpu_accounting: None,
            memory_accounting: None,
            io_accounting: None,
            tasks_accounting: None,
            device_allow: Vec::new(),
            device_policy: crate::units::DevicePolicy::Auto,
            ip_address_allow: Vec::new(),
            ip_address_deny: Vec::new(),
            managed_oom_swap: None,
            managed_oom_memory_pressure: None,
            managed_oom_memory_pressure_limit: None,
            managed_oom_preference: None,
            memory_pressure_watch: crate::units::MemoryPressureWatch::Auto,
            cpu_quota_period_sec: None,
            allowed_cpus: None,
            startup_allowed_cpus: None,
            allowed_memory_nodes: None,
            startup_allowed_memory_nodes: None,
            default_memory_min: None,
            default_memory_low: None,
            memory_zswap_max: None,
            io_device_latency_target_sec: Vec::new(),
            disable_controllers: Vec::new(),
            memory_pressure_threshold_sec: None,
            ip_ingress_filter_path: Vec::new(),
            ip_egress_filter_path: Vec::new(),
            bpf_program: Vec::new(),
            socket_bind_allow: Vec::new(),
            socket_bind_deny: Vec::new(),
            restrict_network_interfaces: Vec::new(),
            nft_set: Vec::new(),
            delegate_subgroup: None,
        }
    }

    #[test]
    fn test_slice_config_defaults() {
        let conf = default_slice_config();
        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        // Memory defaults
        assert_eq!(props.get("MemoryMin").unwrap(), "");
        assert_eq!(props.get("MemoryLow").unwrap(), "");
        assert_eq!(props.get("MemoryHigh").unwrap(), "infinity");
        assert_eq!(props.get("MemoryMax").unwrap(), "infinity");
        assert_eq!(props.get("MemorySwapMax").unwrap(), "infinity");
        assert_eq!(props.get("MemoryZSwapMax").unwrap(), "infinity");
        assert_eq!(props.get("DefaultMemoryMin").unwrap(), "");
        assert_eq!(props.get("DefaultMemoryLow").unwrap(), "");

        // CPU defaults
        assert_eq!(props.get("CPUWeight").unwrap(), "");
        assert_eq!(props.get("StartupCPUWeight").unwrap(), "");
        assert_eq!(props.get("CPUQuota").unwrap(), "");
        assert_eq!(props.get("CPUQuotaPeriodUSec").unwrap(), "infinity");

        // IO defaults
        assert_eq!(props.get("IOWeight").unwrap(), "");
        assert_eq!(props.get("StartupIOWeight").unwrap(), "");

        // Tasks defaults
        assert_eq!(props.get("TasksMax").unwrap(), "infinity");

        // Accounting defaults
        assert_eq!(props.get("CPUAccounting").unwrap(), "no");
        assert_eq!(props.get("MemoryAccounting").unwrap(), "yes");
        assert_eq!(props.get("IOAccounting").unwrap(), "no");
        assert_eq!(props.get("TasksAccounting").unwrap(), "yes");

        // Delegate defaults
        assert_eq!(props.get("Delegate").unwrap(), "no");

        // Device defaults
        assert_eq!(props.get("DevicePolicy").unwrap(), "auto");

        // Memory pressure defaults
        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "auto");
        assert_eq!(
            props.get("MemoryPressureThresholdUSec").unwrap(),
            "infinity"
        );
    }

    #[test]
    fn test_slice_config_memory_bytes() {
        let mut conf = default_slice_config();
        conf.memory_min = Some(crate::units::MemoryLimit::Bytes(1048576));
        conf.memory_low = Some(crate::units::MemoryLimit::Bytes(2097152));
        conf.memory_high = Some(crate::units::MemoryLimit::Bytes(4294967296));
        conf.memory_max = Some(crate::units::MemoryLimit::Bytes(8589934592));
        conf.memory_swap_max = Some(crate::units::MemoryLimit::Bytes(1073741824));
        conf.memory_zswap_max = Some(crate::units::MemoryLimit::Bytes(536870912));
        conf.default_memory_min = Some(crate::units::MemoryLimit::Bytes(524288));
        conf.default_memory_low = Some(crate::units::MemoryLimit::Bytes(1048576));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryMin").unwrap(), "1048576");
        assert_eq!(props.get("MemoryLow").unwrap(), "2097152");
        assert_eq!(props.get("MemoryHigh").unwrap(), "4294967296");
        assert_eq!(props.get("MemoryMax").unwrap(), "8589934592");
        assert_eq!(props.get("MemorySwapMax").unwrap(), "1073741824");
        assert_eq!(props.get("MemoryZSwapMax").unwrap(), "536870912");
        assert_eq!(props.get("DefaultMemoryMin").unwrap(), "524288");
        assert_eq!(props.get("DefaultMemoryLow").unwrap(), "1048576");
    }

    #[test]
    fn test_slice_config_memory_percent() {
        let mut conf = default_slice_config();
        conf.memory_max = Some(crate::units::MemoryLimit::Percent(80));
        conf.memory_high = Some(crate::units::MemoryLimit::Percent(70));
        conf.default_memory_low = Some(crate::units::MemoryLimit::Percent(25));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryMax").unwrap(), "80%");
        assert_eq!(props.get("MemoryHigh").unwrap(), "70%");
        assert_eq!(props.get("DefaultMemoryLow").unwrap(), "25%");
    }

    #[test]
    fn test_slice_config_memory_infinity() {
        let mut conf = default_slice_config();
        conf.memory_min = Some(crate::units::MemoryLimit::Infinity);
        conf.default_memory_min = Some(crate::units::MemoryLimit::Infinity);

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryMin").unwrap(), "infinity");
        assert_eq!(props.get("DefaultMemoryMin").unwrap(), "infinity");
    }

    #[test]
    fn test_slice_config_cpu_weight() {
        let mut conf = default_slice_config();
        conf.cpu_weight = Some(500);
        conf.startup_cpu_weight = Some(100);

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("CPUWeight").unwrap(), "500");
        assert_eq!(props.get("StartupCPUWeight").unwrap(), "100");
    }

    #[test]
    fn test_slice_config_cpu_quota() {
        let mut conf = default_slice_config();
        conf.cpu_quota = Some(200);
        conf.cpu_quota_period_sec = Some(Timeout::Duration(std::time::Duration::from_millis(100)));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("CPUQuota").unwrap(), "200%");
        assert_eq!(props.get("CPUQuotaPeriodUSec").unwrap(), "100000us");
    }

    #[test]
    fn test_slice_config_io_weight() {
        let mut conf = default_slice_config();
        conf.io_weight = Some(500);
        conf.startup_io_weight = Some(100);

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("IOWeight").unwrap(), "500");
        assert_eq!(props.get("StartupIOWeight").unwrap(), "100");
    }

    #[test]
    fn test_slice_config_io_device_limits() {
        let mut conf = default_slice_config();
        conf.io_device_weight = vec![crate::units::IoDeviceLimit {
            device: "/dev/sda".to_owned(),
            value: 200,
        }];
        conf.io_read_bandwidth_max = vec![crate::units::IoDeviceLimit {
            device: "/dev/sda".to_owned(),
            value: 1048576,
        }];
        conf.io_write_bandwidth_max = vec![crate::units::IoDeviceLimit {
            device: "/dev/sdb".to_owned(),
            value: 524288,
        }];
        conf.io_read_iops_max = vec![crate::units::IoDeviceLimit {
            device: "/dev/sda".to_owned(),
            value: 1000,
        }];
        conf.io_write_iops_max = vec![crate::units::IoDeviceLimit {
            device: "/dev/sda".to_owned(),
            value: 500,
        }];
        conf.io_device_latency_target_sec =
            vec!["/dev/sda 25ms".to_owned(), "/dev/sdb 50ms".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("IODeviceWeight").unwrap(), "/dev/sda 200");
        assert_eq!(props.get("IOReadBandwidthMax").unwrap(), "/dev/sda 1048576");
        assert_eq!(props.get("IOWriteBandwidthMax").unwrap(), "/dev/sdb 524288");
        assert_eq!(props.get("IOReadIOPSMax").unwrap(), "/dev/sda 1000");
        assert_eq!(props.get("IOWriteIOPSMax").unwrap(), "/dev/sda 500");
        assert_eq!(
            props.get("IODeviceLatencyTargetSec").unwrap(),
            "/dev/sda 25ms /dev/sdb 50ms"
        );
    }

    #[test]
    fn test_slice_config_tasks_max_value() {
        let mut conf = default_slice_config();
        conf.tasks_max = Some(crate::units::TasksMax::Value(4096));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("TasksMax").unwrap(), "4096");
    }

    #[test]
    fn test_slice_config_tasks_max_percent() {
        let mut conf = default_slice_config();
        conf.tasks_max = Some(crate::units::TasksMax::Percent(50));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("TasksMax").unwrap(), "50%");
    }

    #[test]
    fn test_slice_config_accounting_toggles() {
        let mut conf = default_slice_config();
        conf.cpu_accounting = Some(true);
        conf.memory_accounting = Some(false);
        conf.io_accounting = Some(true);
        conf.tasks_accounting = Some(false);

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("CPUAccounting").unwrap(), "yes");
        assert_eq!(props.get("MemoryAccounting").unwrap(), "no");
        assert_eq!(props.get("IOAccounting").unwrap(), "yes");
        assert_eq!(props.get("TasksAccounting").unwrap(), "no");
    }

    #[test]
    fn test_slice_config_delegate_yes() {
        let mut conf = default_slice_config();
        conf.delegate = crate::units::Delegate::Yes;

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("Delegate").unwrap(), "yes");
    }

    #[test]
    fn test_slice_config_delegate_controllers() {
        let mut conf = default_slice_config();
        conf.delegate =
            crate::units::Delegate::Controllers(vec!["cpu".to_owned(), "memory".to_owned()]);
        conf.delegate_subgroup = Some("supervisor".to_owned());

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("Delegate").unwrap(), "cpu memory");
        assert_eq!(props.get("DelegateSubgroup").unwrap(), "supervisor");
    }

    #[test]
    fn test_slice_config_device_policy() {
        let mut conf = default_slice_config();
        conf.device_policy = crate::units::DevicePolicy::Strict;
        conf.device_allow = vec!["char-tty rw".to_owned(), "/dev/null rw".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("DevicePolicy").unwrap(), "strict");
        assert_eq!(
            props.get("DeviceAllow").unwrap(),
            "char-tty rw /dev/null rw"
        );
    }

    #[test]
    fn test_slice_config_ip_filters() {
        let mut conf = default_slice_config();
        conf.ip_address_allow = vec!["10.0.0.0/8".to_owned(), "192.168.0.0/16".to_owned()];
        conf.ip_address_deny = vec!["any".to_owned()];
        conf.ip_ingress_filter_path = vec!["/sys/fs/bpf/ingress".to_owned()];
        conf.ip_egress_filter_path = vec!["/sys/fs/bpf/egress".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(
            props.get("IPAddressAllow").unwrap(),
            "10.0.0.0/8 192.168.0.0/16"
        );
        assert_eq!(props.get("IPAddressDeny").unwrap(), "any");
        assert_eq!(
            props.get("IPIngressFilterPath").unwrap(),
            "/sys/fs/bpf/ingress"
        );
        assert_eq!(
            props.get("IPEgressFilterPath").unwrap(),
            "/sys/fs/bpf/egress"
        );
    }

    #[test]
    fn test_slice_config_bpf_and_network() {
        let mut conf = default_slice_config();
        conf.bpf_program = vec!["egress:/sys/fs/bpf/prog".to_owned()];
        conf.socket_bind_allow = vec!["tcp:80".to_owned(), "tcp:443".to_owned()];
        conf.socket_bind_deny = vec!["any".to_owned()];
        conf.restrict_network_interfaces = vec!["eth0".to_owned(), "lo".to_owned()];
        conf.nft_set = vec!["inet:filter:allowed_ips".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("BPFProgram").unwrap(), "egress:/sys/fs/bpf/prog");
        assert_eq!(props.get("SocketBindAllow").unwrap(), "tcp:80 tcp:443");
        assert_eq!(props.get("SocketBindDeny").unwrap(), "any");
        assert_eq!(props.get("RestrictNetworkInterfaces").unwrap(), "eth0 lo");
        assert_eq!(props.get("NFTSet").unwrap(), "inet:filter:allowed_ips");
    }

    #[test]
    fn test_slice_config_cpuset() {
        let mut conf = default_slice_config();
        conf.allowed_cpus = Some("0-3".to_owned());
        conf.startup_allowed_cpus = Some("0-1".to_owned());
        conf.allowed_memory_nodes = Some("0".to_owned());
        conf.startup_allowed_memory_nodes = Some("0-1".to_owned());

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("AllowedCPUs").unwrap(), "0-3");
        assert_eq!(props.get("StartupAllowedCPUs").unwrap(), "0-1");
        assert_eq!(props.get("AllowedMemoryNodes").unwrap(), "0");
        assert_eq!(props.get("StartupAllowedMemoryNodes").unwrap(), "0-1");
    }

    #[test]
    fn test_slice_config_disable_controllers() {
        let mut conf = default_slice_config();
        conf.disable_controllers = vec!["cpu".to_owned(), "io".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("DisableControllers").unwrap(), "cpu io");
    }

    #[test]
    fn test_slice_config_managed_oom() {
        let mut conf = default_slice_config();
        conf.managed_oom_swap = Some("kill".to_owned());
        conf.managed_oom_memory_pressure = Some("kill".to_owned());
        conf.managed_oom_memory_pressure_limit = Some("50%".to_owned());
        conf.managed_oom_preference = Some("avoid".to_owned());

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("ManagedOOMSwap").unwrap(), "kill");
        assert_eq!(props.get("ManagedOOMMemoryPressure").unwrap(), "kill");
        assert_eq!(props.get("ManagedOOMMemoryPressureLimit").unwrap(), "50%");
        assert_eq!(props.get("ManagedOOMPreference").unwrap(), "avoid");
    }

    #[test]
    fn test_slice_config_memory_pressure_watch() {
        let mut conf = default_slice_config();
        conf.memory_pressure_watch = crate::units::MemoryPressureWatch::On;
        conf.memory_pressure_threshold_sec =
            Some(Timeout::Duration(std::time::Duration::from_secs(2)));

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "on");
        assert_eq!(
            props.get("MemoryPressureThresholdUSec").unwrap(),
            "2000000us"
        );
    }

    #[test]
    fn test_slice_config_memory_pressure_watch_off() {
        let mut conf = default_slice_config();
        conf.memory_pressure_watch = crate::units::MemoryPressureWatch::Off;

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "off");
    }

    #[test]
    fn test_slice_config_memory_pressure_watch_skip() {
        let mut conf = default_slice_config();
        conf.memory_pressure_watch = crate::units::MemoryPressureWatch::Skip;

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "skip");
    }

    #[test]
    fn test_slice_config_device_policy_closed() {
        let mut conf = default_slice_config();
        conf.device_policy = crate::units::DevicePolicy::Closed;

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("DevicePolicy").unwrap(), "closed");
    }

    #[test]
    fn test_slice_config_combined_all_directives() {
        let mut conf = default_slice_config();
        conf.memory_max = Some(crate::units::MemoryLimit::Bytes(4294967296));
        conf.cpu_weight = Some(500);
        conf.cpu_quota = Some(150);
        conf.io_weight = Some(200);
        conf.tasks_max = Some(crate::units::TasksMax::Value(1024));
        conf.delegate = crate::units::Delegate::Yes;
        conf.cpu_accounting = Some(true);
        conf.memory_accounting = Some(true);
        conf.io_accounting = Some(true);
        conf.tasks_accounting = Some(true);
        conf.device_policy = crate::units::DevicePolicy::Strict;
        conf.allowed_cpus = Some("0-7".to_owned());
        conf.delegate_subgroup = Some("app".to_owned());
        conf.managed_oom_swap = Some("kill".to_owned());
        conf.memory_pressure_watch = crate::units::MemoryPressureWatch::On;
        conf.disable_controllers = vec!["hugetlb".to_owned()];
        conf.socket_bind_allow = vec!["tcp:8080".to_owned()];

        let mut props = PropertyMap::new();
        insert_slice_config(&mut props, &conf);

        assert_eq!(props.get("MemoryMax").unwrap(), "4294967296");
        assert_eq!(props.get("CPUWeight").unwrap(), "500");
        assert_eq!(props.get("CPUQuota").unwrap(), "150%");
        assert_eq!(props.get("IOWeight").unwrap(), "200");
        assert_eq!(props.get("TasksMax").unwrap(), "1024");
        assert_eq!(props.get("Delegate").unwrap(), "yes");
        assert_eq!(props.get("CPUAccounting").unwrap(), "yes");
        assert_eq!(props.get("MemoryAccounting").unwrap(), "yes");
        assert_eq!(props.get("IOAccounting").unwrap(), "yes");
        assert_eq!(props.get("TasksAccounting").unwrap(), "yes");
        assert_eq!(props.get("DevicePolicy").unwrap(), "strict");
        assert_eq!(props.get("AllowedCPUs").unwrap(), "0-7");
        assert_eq!(props.get("DelegateSubgroup").unwrap(), "app");
        assert_eq!(props.get("ManagedOOMSwap").unwrap(), "kill");
        assert_eq!(props.get("MemoryPressureWatch").unwrap(), "on");
        assert_eq!(props.get("DisableControllers").unwrap(), "hugetlb");
        assert_eq!(props.get("SocketBindAllow").unwrap(), "tcp:8080");
    }
}
