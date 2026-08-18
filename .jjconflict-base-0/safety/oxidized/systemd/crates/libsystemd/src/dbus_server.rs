//! Minimal D-Bus server implementing `org.freedesktop.systemd1`.
//!
//! PID 1 connects to the system bus once `dbus.service` is running and
//! registers the well-known name `org.freedesktop.systemd1`, then exports
//! `/org/freedesktop/systemd1` implementing the `Manager` interface.
//!
//! The module is a no-op stub when the `dbus_support` cargo feature is off.

#[cfg(not(feature = "dbus_support"))]
pub fn start_dbus_server_thread(_run_info: crate::runtime_info::ArcMutRuntimeInfo) {
    // no-op
}

/// Job lifecycle hooks called by the `JobRegistry` (units/jobs.rs). With
/// D-Bus support compiled in and the system bus connected they emit
/// JobNew/JobRemoved and register/unregister the per-job object; otherwise
/// they are no-ops.
#[cfg(not(feature = "dbus_support"))]
pub fn notify_job_created(_job: &crate::units::jobs::Job) {
    // no-op
}

#[cfg(not(feature = "dbus_support"))]
pub fn notify_job_removed(_job: &crate::units::jobs::Job) {
    // no-op
}

#[cfg(feature = "dbus_support")]
pub use inner::{notify_job_created, notify_job_removed, start_dbus_server_thread};

#[cfg(feature = "dbus_support")]
mod inner {
    use crate::lock_ext::RwLockExt;
    use crate::runtime_info::ArcMutRuntimeInfo;
    use crate::units::UnitStatus;
    use log::{info, trace, warn};
    use zbus::blocking::Connection;
    use zbus::interface;

    /// The `/org/freedesktop/systemd1` object implementing the Manager interface.
    struct Manager {
        run_info: ArcMutRuntimeInfo,
    }

    /// Per-unit object exported at
    /// `/org/freedesktop/systemd1/unit/<escaped_name>`.  Implements
    /// `org.freedesktop.systemd1.Unit` with a subset of properties that
    /// tests actually query via `busctl get-property`.
    struct UnitObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Unit")]
    impl UnitObj {
        #[zbus(property)]
        fn id(&self) -> String {
            self.unit_name.clone()
        }

        #[zbus(property)]
        fn description(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.unit.description.clone())
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn active_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if let Some(u) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) {
                let status = u.common.status.read_poisoned();
                map_active_sub(&status).0.to_string()
            } else {
                "inactive".to_string()
            }
        }

        #[zbus(property)]
        fn sub_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if let Some(u) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) {
                let status = u.common.status.read_poisoned();
                // Socket units report `listening` when bound and idle, and
                // `running` while an accepted connection is being serviced.
                if matches!(&*status, UnitStatus::Started(_))
                    && let crate::units::Specific::Socket(specific) = &u.specific
                {
                    let st = specific.state.read_poisoned();
                    let sub = if st.sock.deferred {
                        "deferred"
                    } else if specific.conf.accept {
                        if st.sock.active_accept_connections > 0 {
                            "running"
                        } else {
                            "listening"
                        }
                    } else if st.sock.activated {
                        "running"
                    } else {
                        "listening"
                    };
                    return sub.to_string();
                }
                map_active_sub(&status).1.to_string()
            } else {
                "dead".to_string()
            }
        }

        #[zbus(property)]
        fn load_state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            if ri.unit_table.values().any(|u| u.id.name == self.unit_name) {
                "loaded".to_string()
            } else {
                "not-found".to_string()
            }
        }

        /// The job installed for this unit as (id, path), or (0, "/") when
        /// none is.
        #[zbus(property)]
        fn job(&self) -> (u32, zbus::zvariant::OwnedObjectPath) {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            registry
                .iter()
                .find(|job| job.unit.name == self.unit_name)
                .map_or_else(
                    || (0, zbus::zvariant::OwnedObjectPath::try_from("/").unwrap()),
                    |job| (job.id, job_object_path(job.id)),
                )
        }

        #[zbus(property)]
        fn can_start(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_stop(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_reload(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| match &u.specific {
                    crate::units::Specific::Service(srvc) => {
                        !srvc.conf.reload.is_empty()
                            || matches!(
                                srvc.conf.srcv_type,
                                crate::units::ServiceType::NotifyReload
                            )
                    }
                    _ => false,
                })
                .unwrap_or(false)
        }

        #[zbus(property)]
        fn can_isolate(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.unit.allow_isolate)
                .unwrap_or(false)
        }

        /// `Documentation=` URIs.
        #[zbus(property)]
        fn documentation(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.unit.documentation.clone())
                .unwrap_or_default()
        }

        /// Whether the unit has the default implicit dependencies
        /// (Before=shutdown.target / After=basic.target etc.).
        #[zbus(property)]
        fn default_dependencies(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.unit.default_dependencies)
                .unwrap_or(true)
        }

        #[zbus(property)]
        fn can_freeze(&self) -> bool {
            true
        }

        /// Invocation ID of the current or most-recent activation, as a
        /// lowercase hex string without dashes.  Empty when the unit has
        /// never been activated.
        #[zbus(property)]
        fn invocation_id(&self) -> Vec<u8> {
            let ri = self.run_info.read_poisoned();
            let id = ri
                .unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| u.common.invocation_id.lock().unwrap().clone())
                .unwrap_or_default();
            // Upstream returns an ay (byte array) of the raw 16-byte id.
            // We store as hex — decode back to bytes.
            let mut out = Vec::with_capacity(16);
            let mut iter = id.chars().filter(|c| c.is_ascii_hexdigit());
            while let (Some(a), Some(b)) = (iter.next(), iter.next()) {
                if let Ok(byte) = u8::from_str_radix(&format!("{a}{b}"), 16) {
                    out.push(byte);
                }
            }
            out
        }

        /// Activation details of the current activation, as `a(ss)`.  When a
        /// service was activated by a `.path` or `.timer` unit, this reports
        /// `[("trigger_unit", <unit>), ("trigger_path", <path>)]` (trigger_path
        /// only for path triggers).  Empty once the unit is no longer active
        /// from that trigger — real systemd drops ActivationDetails on
        /// deactivation, so we gate on the unit being currently started.
        #[zbus(property)]
        fn activation_details(&self) -> Vec<(String, String)> {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return Vec::new();
            };
            if !matches!(&*unit.common.status.read_poisoned(), UnitStatus::Started(_)) {
                return Vec::new();
            }
            let crate::units::Specific::Service(srvc) = &unit.specific else {
                return Vec::new();
            };
            let state = srvc.state.read_poisoned();
            let mut out = Vec::new();
            if let Some(tu) = &state.srvc.trigger_unit {
                out.push(("trigger_unit".to_string(), tu.clone()));
            }
            if let Some(tp) = &state.srvc.trigger_path {
                out.push(("trigger_path".to_string(), tp.clone()));
            }
            out
        }

        /// Freeze the unit via the cgroup v2 freezer (like `systemctl freeze`).
        fn freeze(&self) -> zbus::fdo::Result<()> {
            invoke_command(
                &self.run_info,
                crate::control::Command::Freeze(self.unit_name.clone()),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Thaw a previously frozen unit (like `systemctl thaw`).
        fn thaw(&self) -> zbus::fdo::Result<()> {
            invoke_command(
                &self.run_info,
                crate::control::Command::Thaw(self.unit_name.clone()),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// When the unit last transitioned out of inactive (activation
        /// began), in microseconds since the Unix epoch.  `0` means never.
        #[zbus(property)]
        fn inactive_exit_timestamp(&self) -> u64 {
            ts_field(&self.run_info, &self.unit_name, |t| t.inactive_exit)
        }

        /// When the unit last became active, in microseconds since the
        /// Unix epoch.  `0` means never.
        #[zbus(property)]
        fn active_enter_timestamp(&self) -> u64 {
            ts_field(&self.run_info, &self.unit_name, |t| t.active_enter)
        }

        /// When the unit last left active state (stop began).
        #[zbus(property)]
        fn active_exit_timestamp(&self) -> u64 {
            ts_field(&self.run_info, &self.unit_name, |t| t.active_exit)
        }

        /// When the unit last re-entered inactive state (stop completed).
        #[zbus(property)]
        fn inactive_enter_timestamp(&self) -> u64 {
            ts_field(&self.run_info, &self.unit_name, |t| t.inactive_enter)
        }

        /// All names this unit answers to — primary id plus any aliases.
        #[zbus(property)]
        fn names(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return vec![self.unit_name.clone()];
            };
            let mut out = vec![unit.id.name.clone()];
            out.extend(unit.common.unit.aliases.iter().cloned());
            out
        }

        /// Absolute path of the unit's source fragment file, if any.
        #[zbus(property)]
        fn fragment_path(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| u.common.unit.fragment_path.as_ref())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn wants(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.wants)
        }

        #[zbus(property)]
        fn requires(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.requires)
        }

        #[zbus(property)]
        fn wanted_by(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.wanted_by)
        }

        #[zbus(property)]
        fn required_by(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.required_by)
        }

        #[zbus(property)]
        fn after(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.after)
        }

        #[zbus(property)]
        fn before(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.before)
        }

        #[zbus(property)]
        fn conflicts(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.conflicts)
        }

        #[zbus(property)]
        fn part_of(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.part_of)
        }

        #[zbus(property)]
        fn binds_to(&self) -> Vec<String> {
            dep_names(&self.run_info, &self.unit_name, |d| &d.binds_to)
        }

        /// Whether the unit file is masked (`/dev/null` symlink) or enabled.
        /// Returns one of `"masked"`, `"enabled"`, `"disabled"`, or
        /// `"static"` — we don't track enable-state reliably for all
        /// runtime-installed units, so non-masked units report `"enabled"`
        /// by default.
        #[zbus(property)]
        fn unit_file_state(&self) -> String {
            let name = &self.unit_name;
            let runtime = std::path::Path::new("/run/systemd/system").join(name);
            let persistent = std::path::Path::new("/etc/systemd/system").join(name);
            let masked = std::fs::read_link(&runtime)
                .or_else(|_| std::fs::read_link(&persistent))
                .map(|t| t == std::path::Path::new("/dev/null"))
                .unwrap_or(false);
            if masked {
                "masked".to_string()
            } else {
                "enabled".to_string()
            }
        }

        /// Drop-in config files merged into this unit.
        #[zbus(property)]
        fn drop_in_paths(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    u.common
                        .unit
                        .loaded_dropin_files
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect()
                })
                .unwrap_or_default()
        }

        /// Whether runtime bind mounts (`systemctl bind`) can be applied
        /// to this unit.  Only services that already have a private mount
        /// namespace (PrivateMounts=yes, PrivateTmp=yes, or similar) can
        /// accept live mounts.
        #[zbus(property)]
        fn can_live_mount(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return false;
            };
            let crate::units::Specific::Service(srvc) = &unit.specific else {
                return false;
            };
            srvc.conf.exec_config.private_tmp
                || srvc.conf.exec_config.private_devices
                || srvc.conf.exec_config.private_mounts
                || srvc.conf.exec_config.private_users
                || !srvc.conf.exec_config.bind_paths.is_empty()
                || !srvc.conf.exec_config.bind_read_only_paths.is_empty()
                || !srvc.conf.exec_config.read_only_paths.is_empty()
                || !srvc.conf.exec_config.inaccessible_paths.is_empty()
        }
    }

    /// Per-unit object additionally exposing `org.freedesktop.systemd1.Service`
    /// for .service units.  Registered on the same object path as the Unit
    /// interface; zbus routes introspection/property queries to whichever
    /// interface matches.
    struct ServiceObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    /// Per-path object exposing `org.freedesktop.systemd1.Path` for
    /// `.path` units.
    struct PathObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Path")]
    impl PathObj {
        /// Unit= value — the unit this path activates when it matches.
        #[zbus(property)]
        fn unit(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Path(p) => Some(p.conf.unit.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| self.unit_name.replace(".path", ".service"))
        }

        #[zbus(property)]
        fn make_directory(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Path(p) => Some(p.conf.make_directory),
                    _ => None,
                })
                .unwrap_or(false)
        }

        #[zbus(property)]
        fn directory_mode(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Path(p) => Some(p.conf.directory_mode),
                    _ => None,
                })
                .unwrap_or(0o755)
        }
    }

    /// Per-slice object exposing `org.freedesktop.systemd1.Slice` for
    /// `.slice` units — exposing the common cgroup resource controls so
    /// tests can `busctl get-property` them (e.g.
    /// `org.freedesktop.systemd1.Slice MemoryMax`).
    struct SliceObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Slice")]
    impl SliceObj {
        #[zbus(property)]
        fn memory_max(&self) -> u64 {
            memory_limit_bytes(&self.run_info, &self.unit_name, |c| &c.memory_max)
        }

        #[zbus(property)]
        fn memory_min(&self) -> u64 {
            memory_limit_bytes(&self.run_info, &self.unit_name, |c| &c.memory_min)
        }

        #[zbus(property)]
        fn memory_low(&self) -> u64 {
            memory_limit_bytes(&self.run_info, &self.unit_name, |c| &c.memory_low)
        }

        #[zbus(property)]
        fn memory_high(&self) -> u64 {
            memory_limit_bytes(&self.run_info, &self.unit_name, |c| &c.memory_high)
        }

        #[zbus(property)]
        fn memory_swap_max(&self) -> u64 {
            memory_limit_bytes(&self.run_info, &self.unit_name, |c| &c.memory_swap_max)
        }
    }

    /// Resolve a `MemoryLimit` into a `u64` (the D-Bus wire type for
    /// MemoryMax=, MemoryMin=, etc.).  Absent or infinity limits map to
    /// `u64::MAX` — matching upstream's "-1 / infinity" encoding.
    fn memory_limit_bytes<F>(run_info: &ArcMutRuntimeInfo, unit_name: &str, select: F) -> u64
    where
        F: Fn(&crate::units::SliceConfig) -> &Option<crate::units::MemoryLimit>,
    {
        let ri = run_info.read_poisoned();
        let Some(unit) = ri.unit_table.values().find(|u| u.id.name == unit_name) else {
            return u64::MAX;
        };
        let crate::units::Specific::Slice(slice) = &unit.specific else {
            return u64::MAX;
        };
        match select(&slice.conf) {
            None => u64::MAX,
            Some(crate::units::MemoryLimit::Infinity) => u64::MAX,
            Some(crate::units::MemoryLimit::Bytes(n)) => *n,
            Some(crate::units::MemoryLimit::Percent(_)) => u64::MAX,
        }
    }

    /// Per-timer object exposing `org.freedesktop.systemd1.Timer` for
    /// `.timer` units.
    struct TimerObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Timer")]
    impl TimerObj {
        /// Unit that this timer activates when it fires.
        #[zbus(property)]
        fn unit(&self) -> String {
            let ri = self.run_info.read_poisoned();
            let Some(unit) = ri.unit_table.values().find(|u| u.id.name == self.unit_name) else {
                return String::new();
            };
            // Convention: foo.timer triggers foo.service.
            unit.id.name.replace(".timer", ".service")
        }

        /// OnCalendar= specifications as strings.
        #[zbus(property)]
        fn timers_calendar(&self) -> Vec<String> {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Timer(t) => Some(t.conf.on_calendar.clone()),
                    _ => None,
                })
                .unwrap_or_default()
        }

        #[zbus(property)]
        fn persistent(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Timer(t) => Some(t.conf.persistent),
                    _ => None,
                })
                .unwrap_or(false)
        }
    }

    /// Per-socket object exposing `org.freedesktop.systemd1.Socket` for
    /// `.socket` units.
    struct SocketObj {
        run_info: ArcMutRuntimeInfo,
        unit_name: String,
    }

    #[interface(name = "org.freedesktop.systemd1.Socket")]
    impl SocketObj {
        #[zbus(property)]
        fn accept(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Socket(sock) => Some(sock.conf.accept),
                    _ => None,
                })
                .unwrap_or(false)
        }

        #[zbus(property)]
        fn max_connections(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Socket(sock) => Some(sock.conf.max_connections as u32),
                    _ => None,
                })
                .unwrap_or(0)
        }

        #[zbus(property)]
        fn socket_mode(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Socket(sock) => sock.conf.socket_mode,
                    _ => None,
                })
                .unwrap_or(0o666)
        }

        #[zbus(property)]
        fn pass_credentials(&self) -> bool {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Socket(sock) => Some(sock.conf.pass_credentials),
                    _ => None,
                })
                .unwrap_or(false)
        }
    }

    #[interface(name = "org.freedesktop.systemd1.Service")]
    impl ServiceObj {
        #[zbus(property)]
        fn main_pid(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let pid = u.common.main_pid.load(std::sync::atomic::Ordering::Acquire);
                    if pid > 0 { pid as u32 } else { 0 }
                })
                .unwrap_or(0)
        }

        #[zbus(property)]
        fn exec_main_pid(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let pid = u
                        .common
                        .main_exit_pid
                        .load(std::sync::atomic::Ordering::Acquire);
                    if pid > 0 { pid as u32 } else { 0 }
                })
                .unwrap_or(0)
        }

        #[zbus(property)]
        fn exec_main_status(&self) -> i32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    u.common
                        .main_exit_status
                        .load(std::sync::atomic::Ordering::Acquire)
                })
                .unwrap_or(-1)
        }

        #[zbus(property)]
        fn type_(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .and_then(|u| match &u.specific {
                    crate::units::Specific::Service(srvc) => {
                        Some(service_type_string(&srvc.conf.srcv_type))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "simple".to_string())
        }

        #[zbus(property)]
        fn result(&self) -> String {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    let status = u.common.status.read_poisoned();
                    match &*status {
                        UnitStatus::Stopped(_, errs) if !errs.is_empty() => "failure",
                        _ => "success",
                    }
                    .to_string()
                })
                .unwrap_or_else(|| "success".to_string())
        }

        /// Number of times this service has been restarted (Restart= fires).
        #[zbus(property)]
        fn n_restarts(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .find(|u| u.id.name == self.unit_name)
                .map(|u| {
                    u.common
                        .n_restarts
                        .load(std::sync::atomic::Ordering::Acquire) as u32
                })
                .unwrap_or(0)
        }

        /// `ExecStart=` as a list of `(path, argv, ignore_errors)` tuples
        /// — signature `a(sasb)` to match upstream's wire type.
        #[zbus(property)]
        fn exec_start(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.exec)
        }

        #[zbus(property)]
        fn exec_start_pre(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.startpre)
        }

        #[zbus(property)]
        fn exec_start_post(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.startpost)
        }

        #[zbus(property)]
        fn exec_stop(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.stop)
        }

        #[zbus(property)]
        fn exec_stop_post(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.stoppost)
        }

        #[zbus(property)]
        fn exec_reload(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.reload)
        }

        #[zbus(property)]
        fn exec_condition(&self) -> Vec<(String, Vec<String>, bool)> {
            exec_commandlines(&self.run_info, &self.unit_name, |c| &c.exec_condition)
        }
    }

    fn exec_commandlines<F>(
        run_info: &ArcMutRuntimeInfo,
        unit_name: &str,
        select: F,
    ) -> Vec<(String, Vec<String>, bool)>
    where
        F: Fn(&crate::units::ServiceConfig) -> &[crate::units::Commandline],
    {
        let ri = run_info.read_poisoned();
        ri.unit_table
            .values()
            .find(|u| u.id.name == unit_name)
            .and_then(|u| match &u.specific {
                crate::units::Specific::Service(srvc) => Some(
                    select(&srvc.conf)
                        .iter()
                        .map(|c| {
                            let mut argv = Vec::with_capacity(c.args.len() + 1);
                            argv.push(c.cmd.clone());
                            argv.extend(c.args.iter().cloned());
                            let ignore_errors = c
                                .prefixes
                                .contains(&crate::units::unit_parsing::CommandlinePrefix::Minus);
                            (c.cmd.clone(), argv, ignore_errors)
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Look up a unit by name and read an Option<u64> timestamp field from
    /// its UnitTimestamps, returning 0 when the unit doesn't exist or the
    /// field is unset.
    fn ts_field<F>(run_info: &ArcMutRuntimeInfo, unit_name: &str, select: F) -> u64
    where
        F: Fn(&crate::units::UnitTimestamps) -> Option<u64>,
    {
        let ri = run_info.read_poisoned();
        ri.unit_table
            .values()
            .find(|u| u.id.name == unit_name)
            .and_then(|u| {
                let ts = u.common.timestamps.read_poisoned();
                select(&ts)
            })
            .unwrap_or(0)
    }

    /// Look up a unit by name and return the names of its dependency entries
    /// selected by `select` — e.g. `|d| &d.wants`.
    fn dep_names<F>(run_info: &ArcMutRuntimeInfo, unit_name: &str, select: F) -> Vec<String>
    where
        F: Fn(&crate::units::Dependencies) -> &[crate::units::UnitId],
    {
        let ri = run_info.read_poisoned();
        ri.unit_table
            .values()
            .find(|u| u.id.name == unit_name)
            .map(|u| {
                select(&u.common.dependencies)
                    .iter()
                    .map(|id| id.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn service_type_string(t: &crate::units::ServiceType) -> String {
        match t {
            crate::units::ServiceType::Simple => "simple",
            crate::units::ServiceType::Forking => "forking",
            crate::units::ServiceType::OneShot => "oneshot",
            crate::units::ServiceType::Dbus => "dbus",
            crate::units::ServiceType::Notify => "notify",
            crate::units::ServiceType::NotifyReload => "notify-reload",
            crate::units::ServiceType::Idle => "idle",
            crate::units::ServiceType::Exec => "exec",
        }
        .to_string()
    }

    /// Per-unit tuple returned by `ListUnits`: (name, description, load,
    /// active, sub, follower, obj_path, job_id, job_type, job_obj_path).
    type ListUnitsEntry = (
        String,
        String,
        String,
        String,
        String,
        String,
        zbus::zvariant::OwnedObjectPath,
        u32,
        String,
        zbus::zvariant::OwnedObjectPath,
    );

    /// Per-job tuple returned by `ListJobs`: (id, unit, type, state,
    /// job_obj_path, unit_obj_path).
    type ListJobsEntry = (
        u32,
        String,
        String,
        String,
        zbus::zvariant::OwnedObjectPath,
        zbus::zvariant::OwnedObjectPath,
    );

    /// Per-job object exported at `/org/freedesktop/systemd1/job/<id>` while
    /// the job is installed. Properties read the live registry and fall back
    /// to the values captured at registration once the job is gone (a client
    /// may still hold the path briefly after JobRemoved).
    struct JobObj {
        run_info: ArcMutRuntimeInfo,
        id: u32,
        unit_name: String,
        kind: &'static str,
    }

    #[interface(name = "org.freedesktop.systemd1.Job")]
    impl JobObj {
        #[zbus(property)]
        fn id(&self) -> u32 {
            self.id
        }

        #[zbus(property)]
        fn unit(&self) -> (String, zbus::zvariant::OwnedObjectPath) {
            (self.unit_name.clone(), unit_object_path(&self.unit_name))
        }

        #[zbus(property)]
        fn job_type(&self) -> String {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            registry.get(self.id).map_or_else(
                || self.kind.to_string(),
                |job| job.kind.as_str().to_string(),
            )
        }

        #[zbus(property)]
        fn state(&self) -> String {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            registry.get(self.id).map_or_else(
                || "running".to_string(),
                |job| job.state.as_str().to_string(),
            )
        }
    }

    /// Resolve the real UID of a D-Bus method's caller via the bus daemon's
    /// `GetConnectionUnixUser`.  A message with no sender comes from a direct
    /// peer connection (no bus) and is treated as trusted (uid 0).  A message
    /// that *does* have a sender but whose UID cannot be resolved fails closed
    /// (returns a non-root sentinel) so a permission check never grants root by
    /// accident.
    async fn caller_uid(header: &zbus::message::Header<'_>, conn: &zbus::Connection) -> u32 {
        let Some(sender) = header.sender() else {
            return 0;
        };
        let Ok(proxy) = zbus::fdo::DBusProxy::new(conn).await else {
            return u32::MAX;
        };
        let bus_name: zbus::names::BusName<'_> = sender.to_owned().into();
        proxy
            .get_connection_unix_user(bus_name)
            .await
            .unwrap_or(u32::MAX)
    }

    #[interface(name = "org.freedesktop.systemd1.Manager")]
    impl Manager {
        /// Returns the version string of the service manager.
        #[zbus(property)]
        fn version(&self) -> String {
            env!("CARGO_PKG_VERSION").to_string()
        }

        /// Returns the architecture string in systemd's canonical spelling
        /// (e.g. "x86-64", not Rust's "x86_64"), matching
        /// `architecture_to_string(uname_architecture())`.
        #[zbus(property)]
        fn architecture(&self) -> String {
            vpick_core::native_arch()
                .map(vpick_core::arch_to_string)
                .unwrap_or(std::env::consts::ARCH)
                .to_string()
        }

        /// Number of loaded units.
        #[zbus(property)]
        fn n_names(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table.len() as u32
        }

        /// Number of installed jobs.
        #[zbus(property)]
        fn n_jobs(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            registry.len() as u32
        }

        /// Emitted when a job is installed.
        #[zbus(signal)]
        async fn job_new(
            emitter: &zbus::object_server::SignalEmitter<'_>,
            id: u32,
            job: zbus::zvariant::OwnedObjectPath,
            unit: String,
        ) -> zbus::Result<()>;

        /// Emitted when a job completes, with its result string
        /// (done/failed/canceled/timeout/dependency/skipped).
        #[zbus(signal)]
        async fn job_removed(
            emitter: &zbus::object_server::SignalEmitter<'_>,
            id: u32,
            job: zbus::zvariant::OwnedObjectPath,
            unit: String,
            result: String,
        ) -> zbus::Result<()>;

        /// Returns the installed jobs as (id, unit, type, state, job_path,
        /// unit_path) tuples, ordered by job ID.
        fn list_jobs(&self) -> Vec<ListJobsEntry> {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            let mut jobs: Vec<_> = registry.iter().collect();
            jobs.sort_by_key(|job| job.id);
            jobs.into_iter()
                .map(|job| {
                    (
                        job.id,
                        job.unit.name.clone(),
                        job.kind.as_str().to_string(),
                        job.state.as_str().to_string(),
                        job_object_path(job.id),
                        unit_object_path(&job.unit.name),
                    )
                })
                .collect()
        }

        /// Returns the object path of the job with the given ID.
        fn get_job(&self, id: u32) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            if registry.get(id).is_some() {
                Ok(job_object_path(id))
            } else {
                Err(zbus::fdo::Error::Failed(format!(
                    "Job {id} does not exist."
                )))
            }
        }

        /// Number of failed units.
        #[zbus(property)]
        fn n_failed_units(&self) -> u32 {
            let ri = self.run_info.read_poisoned();
            ri.unit_table
                .values()
                .filter(|u| {
                    matches!(
                        &*u.common.status.read_poisoned(),
                        UnitStatus::Stopped(_, errs) if !errs.is_empty()
                    )
                })
                .count() as u32
        }

        /// Whether the service manager enables per-service watchdogs by
        /// default.  Always true — individual units can still set
        /// `WatchdogSec=0` to disable.
        #[zbus(property)]
        fn service_watchdogs(&self) -> bool {
            true
        }

        /// systemd compile-time features list.  We report a minimal set.
        #[zbus(property)]
        fn features(&self) -> String {
            "+ACL +BLKID +CRYPTSETUP +GCRYPT +KMOD +LIBCRYPTSETUP +PAM +SECCOMP +SELINUP"
                .to_string()
        }

        /// Virtualization detection.  Always "none" for now — we don't
        /// re-implement systemd-detect-virt.
        #[zbus(property)]
        fn virtualization(&self) -> String {
            "none".to_string()
        }

        /// Show status at boot (whether emergency/status messages appear).
        #[zbus(property)]
        fn show_status(&self) -> String {
            "no".to_string()
        }

        /// Returns all currently loaded units as a list of tuples:
        /// (name, description, load_state, active_state, sub_state,
        ///  follower, object_path, job_id, job_type, job_object_path).
        fn list_units(&self) -> Vec<ListUnitsEntry> {
            let ri = self.run_info.read_poisoned();
            let registry = ri.jobs.lock().unwrap();
            let mut out = Vec::new();
            let root = zbus::zvariant::OwnedObjectPath::try_from("/").unwrap();
            for unit in ri.unit_table.values() {
                let status = unit.common.status.read_poisoned();
                let (active, sub) = map_active_sub(&status);
                let desc = unit.common.unit.description.clone();
                let obj = unit_object_path(&unit.id.name);
                let (job_id, job_type, job_path) = registry.job_for_unit(&unit.id).map_or_else(
                    || (0, String::new(), root.clone()),
                    |job| {
                        (
                            job.id,
                            job.kind.as_str().to_string(),
                            job_object_path(job.id),
                        )
                    },
                );
                out.push((
                    unit.id.name.clone(),
                    desc,
                    "loaded".to_string(),
                    active.to_string(),
                    sub.to_string(),
                    String::new(),
                    obj,
                    job_id,
                    job_type,
                    job_path,
                ));
            }
            out
        }

        /// Returns the object path of the unit with the given name.
        fn get_unit(&self, name: String) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let ri = self.run_info.read_poisoned();
            if ri.unit_table.values().any(|u| u.id.name == name) {
                Ok(unit_object_path(&name))
            } else {
                Err(zbus::fdo::Error::Failed(format!("Unit {name} not found")))
            }
        }

        /// Loads the unit with the given name (from disk on demand, resolving
        /// symlink aliases) and returns its object path.  Unlike GetUnit this
        /// does not require the unit to be pre-loaded.  dbus-broker calls
        /// LoadUnit to map an activatable bus name to its unit object path
        /// before starting it, so on-demand activation of daemons such as
        /// systemd-timedated depends on this method existing.
        fn load_unit(&self, name: String) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            match crate::control::find_or_load_unit(&name, &self.run_info) {
                Ok(id) => Ok(unit_object_path(&id.name)),
                Err(e) => Err(zbus::fdo::Error::Failed(format!(
                    "Failed to load unit {name}: {e}"
                ))),
            }
        }

        /// Start the given unit.  Installs a real job (the synchronous
        /// control handler merges into and completes it) and returns its
        /// object path, like upstream StartUnit.
        fn start_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let mut args = Vec::new();
            if !mode.is_empty() && mode != "replace" {
                args.push(format!("--job-mode={mode}"));
            }
            args.push(name.clone());
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::Start,
                crate::control::Command::Start(args),
            )
        }

        /// Stop the given unit.
        fn stop_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let mut args = Vec::new();
            if !mode.is_empty() && mode != "replace" {
                args.push(format!("--job-mode={mode}"));
            }
            args.push(name.clone());
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::Stop,
                crate::control::Command::Stop(args),
            )
        }

        /// Restart the given unit.
        fn restart_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::Restart,
                crate::control::Command::Restart(name.clone()),
            )
        }

        /// Reload the given unit (ExecReload= or Type=notify-reload SIGHUP).
        fn reload_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::Reload,
                crate::control::Command::Reload(name.clone()),
            )
        }

        /// Restart the unit only if it is currently running.  No-op when
        /// the unit is inactive.  Matches `systemctl try-restart`.
        fn try_restart_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::TryRestart,
                crate::control::Command::TryRestart(name.clone()),
            )
        }

        /// Reload the unit if it supports reloading (Type=notify-reload or
        /// has ExecReload=), otherwise restart it.
        fn reload_or_restart_unit(
            &self,
            name: String,
            mode: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            // Restart covers both halves in our implementation; the job is a
            // restart job, matching what the reduced merge table can express.
            run_job_command(
                &self.run_info,
                &name,
                &mode,
                crate::units::jobs::JobKind::Restart,
                crate::control::Command::Restart(name.clone()),
            )
        }

        /// Reload unit files from disk (equivalent to `systemctl daemon-reload`).
        fn reload(&self) -> zbus::fdo::Result<()> {
            invoke_command(&self.run_info, crate::control::Command::LoadAllNew)
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Send a Unix signal to one or more of a unit's processes.  `whom`
        /// selects the recipient: `"main"`, `"control"`, or `"all"` (default).
        fn kill_unit(&self, name: String, whom: String, signal: i32) -> zbus::fdo::Result<()> {
            let whom_norm = if whom.is_empty() {
                "all".to_string()
            } else {
                whom
            };
            invoke_command(
                &self.run_info,
                crate::control::Command::Kill(name, signal, whom_norm, None, false, None),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Reset the "failed" state of one unit (clears stored errors so it
        /// can be restarted without `--force`).
        fn reset_failed_unit(&self, name: String) -> zbus::fdo::Result<()> {
            invoke_command(
                &self.run_info,
                crate::control::Command::ResetFailed(Some(name)),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Reset all failed units (clears stored errors everywhere).
        fn reset_failed(&self) -> zbus::fdo::Result<()> {
            invoke_command(&self.run_info, crate::control::Command::ResetFailed(None))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Freeze the given unit's cgroup (pause all processes in it).
        fn freeze_unit(&self, name: String) -> zbus::fdo::Result<()> {
            invoke_command(&self.run_info, crate::control::Command::Freeze(name))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Clean the given unit — remove its RuntimeDirectory/StateDirectory/
        /// CacheDirectory/LogsDirectory/ConfigurationDirectory as requested
        /// by the `what` list (values: "configuration", "runtime", "state",
        /// "cache", "logs", "all").  Empty list defaults to runtime+cache.
        fn clean_unit(&self, name: String, what: Vec<String>) -> zbus::fdo::Result<()> {
            let what_str = if what.is_empty() {
                None
            } else {
                Some(what.join(","))
            };
            invoke_command(
                &self.run_info,
                crate::control::Command::Clean(name, what_str),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Thaw a previously frozen unit's cgroup (resume processes).
        fn thaw_unit(&self, name: String) -> zbus::fdo::Result<()> {
            invoke_command(&self.run_info, crate::control::Command::Thaw(name))
                .map_err(zbus::fdo::Error::Failed)?;
            Ok(())
        }

        /// Attach the given PIDs to a unit's cgroup subtree.  `path` selects a
        /// sub-cgroup beneath the unit's own cgroup ("" = the unit itself).
        ///
        /// Permission model mirrors upstream `bus_unit_method_attach_processes`:
        /// only a privileged (uid 0) caller may attach processes to a unit that
        /// is not delegated to the caller.  We do not track per-unit delegation
        /// owners (all system units are root-owned), so any unprivileged caller
        /// is denied — the exact case exercised by TEST-07-PID1.attach_processes,
        /// where a non-root user calls this on a `Delegate=yes` system unit that
        /// has no ref_uid for them.
        #[zbus(name = "AttachProcessesToUnit")]
        async fn attach_processes_to_unit(
            &self,
            name: String,
            path: String,
            pids: Vec<u32>,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &zbus::Connection,
        ) -> zbus::fdo::Result<()> {
            let uid = caller_uid(&header, conn).await;

            // Resolve the unit's own cgroup path (also validates it exists).
            let cgroup = {
                let ri = self.run_info.read_poisoned();
                let Some(unit) = ri.unit_table.values().find(|u| u.id.name == name) else {
                    return Err(zbus::fdo::Error::Failed(format!("Unit {name} not found")));
                };
                match &unit.specific {
                    crate::units::Specific::Service(svc) => {
                        Some(svc.conf.platform_specific.cgroup_path.clone())
                    }
                    _ => None,
                }
            };

            // Unprivileged callers may only attach to a unit delegated to them;
            // we have no such delegation, so deny every non-root caller.
            if uid != 0 {
                return Err(zbus::fdo::Error::AccessDenied(format!(
                    "Access denied: unit {name} is not delegated to uid {uid}"
                )));
            }

            // Privileged path: migrate the PIDs into the unit's cgroup subtree.
            let Some(mut cg) = cgroup else {
                return Err(zbus::fdo::Error::Failed(format!(
                    "Unit {name} has no cgroup to attach processes to"
                )));
            };
            if !path.is_empty() {
                cg.push(path.trim_start_matches('/'));
            }
            let procs = cg.join("cgroup.procs");
            for pid in pids {
                if let Err(e) = std::fs::write(&procs, format!("{pid}\n")) {
                    return Err(zbus::fdo::Error::Failed(format!(
                        "Failed to attach pid {pid} to {}: {e}",
                        procs.display()
                    )));
                }
            }
            Ok(())
        }

        /// Subscribe to manager signals.  We don't emit signals yet — the
        /// method is a no-op accept for compatibility with callers that
        /// blindly call Subscribe() before making any query (systemctl
        /// does this during `monitor`/`list-units`).
        fn subscribe(&self) -> zbus::fdo::Result<()> {
            Ok(())
        }

        /// Unsubscribe from manager signals — matching no-op.
        fn unsubscribe(&self) -> zbus::fdo::Result<()> {
            Ok(())
        }

        /// Find the unit that owns the given PID.  Returns the unit's
        /// object path, or an error if the PID isn't tracked.
        ///
        /// The D-Bus method name must be `GetUnitByPID` (acronym uppercase) to
        /// match upstream; without this override zbus would derive
        /// `GetUnitByPid` from the snake_case fn name and clients calling
        /// `GetUnitByPID` get "Unknown method".
        #[zbus(name = "GetUnitByPID")]
        fn get_unit_by_pid(&self, pid: u32) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            let ri = self.run_info.read_poisoned();
            let pid_i32 = pid as i32;
            // Fast path: check per-unit atomic MainPID.
            let owner = ri
                .unit_table
                .values()
                .find(|u| u.common.main_pid.load(std::sync::atomic::Ordering::Acquire) == pid_i32)
                .map(|u| u.id.name.clone());
            match owner {
                Some(name) => Ok(unit_object_path(&name)),
                None => Err(zbus::fdo::Error::Failed(format!(
                    "PID {pid} not tracked by any unit"
                ))),
            }
        }

        /// Bind-mount `source` onto `destination` inside the mount namespace
        /// of the running service `name`.  If `mkdir` is true, the destination
        /// is created (as a file or directory mirroring the source type)
        /// inside the namespace before the mount.  Only works for services
        /// that already have a private mount namespace (PrivateTmp=,
        /// BindPaths=, PrivateMounts=, etc.) — i.e. `CanLiveMount=true`.
        fn bind_mount_unit(
            &self,
            name: String,
            source: String,
            destination: String,
            read_only: bool,
            mkdir_flag: bool,
        ) -> zbus::fdo::Result<()> {
            crate::control::bind_mount_into_unit(
                &self.run_info,
                &name,
                &source,
                &destination,
                read_only,
                mkdir_flag,
            )
            .map_err(zbus::fdo::Error::Failed)
        }

        /// Create a transient unit from the given name, mode, and property
        /// dictionary.  Properties are translated into `-p NAME=VALUE`
        /// strings and fed through the existing `StartTransient` control
        /// command.  FDs passed via `ExtraFileDescriptors` are dup'd out of
        /// the D-Bus message, FD_CLOEXEC cleared, and stored as the
        /// service's stored_fds so they flow through LISTEN_FDS.
        fn start_transient_unit(
            &self,
            name: String,
            _mode: String,
            properties: Vec<(String, zbus::zvariant::OwnedValue)>,
            _aux: Vec<(String, Vec<(String, zbus::zvariant::OwnedValue)>)>,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
            // Reject exec commands carrying an empty argv, as upstream does
            // (systemd issue #20933). An a(sasb) entry with no argv flattens to
            // an empty string further down and would then be accepted silently,
            // so the malformed call has to be refused here where an error can
            // still be returned. Every entry is checked, not just the last:
            // the test's "bad-middle" case puts the empty argv second of three.
            for (key, value) in &properties {
                let base = key.strip_suffix("Ex").unwrap_or(key.as_str());
                if !matches!(
                    base,
                    "ExecStart"
                        | "ExecStartPre"
                        | "ExecStartPost"
                        | "ExecCondition"
                        | "ExecReload"
                        | "ExecStop"
                        | "ExecStopPost"
                ) {
                    continue;
                }
                if let zbus::zvariant::Value::Array(items) = &**value {
                    for item in items.iter() {
                        if let zbus::zvariant::Value::Structure(st) = item {
                            let empty = match st.fields().get(1) {
                                Some(zbus::zvariant::Value::Array(args)) => args.is_empty(),
                                _ => true,
                            };
                            if empty {
                                return Err(zbus::fdo::Error::InvalidArgs(format!(
                                    "{key}: executable path is empty"
                                )));
                            }
                        }
                    }
                }
            }

            let mut params = crate::control::TransientUnitParams {
                no_env_expand: false,
                unit_name: name.clone(),
                command: None,
                description: None,
                user: None,
                group: None,
                working_directory: None,
                service_type: None,
                remain_after_exit: false,
                properties: Vec::new(),
                environment: Vec::new(),
                scope: false,
                wait: false,
                pipe: false,
                slice: None,
                on_calendar: None,
                on_active: Vec::new(),
                on_boot: None,
                on_startup: None,
                on_unit_active: None,
                on_unit_inactive: None,
                on_clock_change: false,
                on_timezone_change: false,
                timer_properties: Vec::new(),
                path_properties: Vec::new(),
                socket_properties: Vec::new(),
                nice: None,
                extra_file_descriptors: Vec::new(),
            };
            for (key, val) in properties {
                if key == "ExtraFileDescriptors" {
                    params
                        .extra_file_descriptors
                        .extend(extract_fd_entries(&val));
                    continue;
                }
                if let Some(entry) = format_property(&key, &val) {
                    match key.as_str() {
                        "Description" => params.description = Some(entry),
                        "User" => params.user = Some(entry),
                        "Group" => params.group = Some(entry),
                        "WorkingDirectory" => params.working_directory = Some(entry),
                        "Type" => params.service_type = Some(entry),
                        "RemainAfterExit" => params.remain_after_exit = entry == "true",
                        "Slice" => params.slice = Some(entry),
                        _ => params.properties.push(format!("{key}={entry}")),
                    }
                }
            }
            invoke_command(
                &self.run_info,
                crate::control::Command::StartTransient(params),
            )
            .map_err(zbus::fdo::Error::Failed)?;
            // Upstream returns the start job's path, not the unit path. The
            // transient start completed synchronously above, so install and
            // finish the job inline now that the unit exists in the table,
            // unless another producer already has one installed for it.
            let (unit_id, jobs) = {
                let ri = self.run_info.read_poisoned();
                (
                    ri.unit_table
                        .values()
                        .find(|u| u.id.name == name)
                        .map(|u| u.id.clone()),
                    ri.jobs.clone(),
                )
            };
            if let Some(unit_id) = unit_id {
                let mut registry = jobs.lock().unwrap();
                if let Some(existing) = registry.job_for_unit(&unit_id) {
                    return Ok(job_object_path(existing.id));
                }
                if let Ok(job_id) = registry.create(
                    unit_id,
                    crate::units::jobs::JobKind::Start,
                    crate::units::ActivationSource::Regular,
                    crate::units::jobs::JobMode::Replace,
                ) {
                    registry.set_running(job_id);
                    registry.finish(job_id, crate::units::jobs::JobResult::Done);
                    return Ok(job_object_path(job_id));
                }
            }
            Ok(unit_object_path(&name))
        }
    }

    /// Extract `ExtraFileDescriptors` entries (type `a(hs)`) from a D-Bus
    /// variant into a list of (raw_fd, name) pairs. The caller takes
    /// ownership of the fds — we use `into_raw_fd()` on the wrapped
    /// OwnedFd so the zbus message's drop does not close them.
    fn extract_fd_entries(val: &zbus::zvariant::OwnedValue) -> Vec<(std::os::fd::RawFd, String)> {
        use std::os::fd::AsRawFd;
        use zbus::zvariant::Value;
        let inner: &Value = val;
        let mut out = Vec::new();
        let Value::Array(items) = inner else {
            return out;
        };
        for item in items.iter() {
            let Value::Structure(s) = item else {
                continue;
            };
            let fields = s.fields();
            let Some(Value::Fd(fd)) = fields.first() else {
                continue;
            };
            let Some(Value::Str(name)) = fields.get(1) else {
                continue;
            };
            // The zbus Fd borrows from the message; duplicate it so the
            // fd remains valid after the message is dropped.
            let borrowed_raw = fd.as_raw_fd();
            let dup = unsafe { libc::dup(borrowed_raw) };
            if dup < 0 {
                warn!(
                    "dbus-server: dup of ExtraFileDescriptors fd failed: {}",
                    std::io::Error::last_os_error()
                );
                continue;
            }
            // Clear FD_CLOEXEC so the service process inherits it.
            unsafe {
                let flags = libc::fcntl(dup, libc::F_GETFD);
                if flags >= 0 {
                    libc::fcntl(dup, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                }
            }
            out.push((dup, name.to_string()));
        }
        out
    }

    /// Convert a D-Bus variant value into the text form that the transient
    /// unit property parser expects.  Returns `None` when the variant type
    /// is complex enough that text fallback would be meaningless; the
    /// caller can then decide to skip the property.
    fn format_property(key: &str, val: &zbus::zvariant::OwnedValue) -> Option<String> {
        use zbus::zvariant::Value;
        let inner: &Value = val;
        match inner {
            Value::Str(s) => Some(s.to_string()),
            Value::Bool(b) => Some(if *b { "true" } else { "false" }.to_string()),
            Value::U8(u) => Some(u.to_string()),
            Value::I16(i) => Some(i.to_string()),
            Value::U16(u) => Some(u.to_string()),
            Value::I32(i) => Some(i.to_string()),
            Value::U32(u) => Some(u.to_string()),
            Value::I64(i) => Some(i.to_string()),
            Value::U64(u) => Some(u.to_string()),
            Value::F64(f) => Some(f.to_string()),
            Value::Array(items) => {
                // Handle arrays of strings ("as") — used by Documentation=,
                // Environment=, PassEnvironment=, etc.  Join with spaces so
                // the transient unit parser can split back into entries.
                let strs: Vec<String> = items
                    .iter()
                    .filter_map(|a| match a {
                        Value::Str(s) => Some(s.to_string()),
                        _ => None,
                    })
                    .collect();
                if !strs.is_empty() {
                    return Some(strs.join(" "));
                }

                // Complex values — e.g. ExecStart as a(sasb).  Flatten
                // into a shell-quoted argv string so the text parser can
                // split it back out.  Any argument containing whitespace
                // or quotes is double-quoted and inner " / \ escaped —
                // matching `systemd.service(5)`'s quoting rules.
                if key == "ExecStart" {
                    let shell_quote = |s: &str| -> String {
                        let needs_quoting = s.is_empty()
                            || s.chars()
                                .any(|c| c.is_whitespace() || c == '"' || c == '\\' || c == '\'');
                        if !needs_quoting {
                            return s.to_owned();
                        }
                        let mut out = String::with_capacity(s.len() + 2);
                        out.push('"');
                        for c in s.chars() {
                            if c == '"' || c == '\\' {
                                out.push('\\');
                            }
                            out.push(c);
                        }
                        out.push('"');
                        out
                    };
                    let mut out = String::new();
                    for item in items.iter() {
                        if let Value::Structure(s) = item {
                            let fields = s.fields();
                            if let Some(Value::Array(args)) = fields.get(1) {
                                let words: Vec<String> = args
                                    .iter()
                                    .filter_map(|a| match a {
                                        Value::Str(s) => Some(shell_quote(s)),
                                        _ => None,
                                    })
                                    .collect();
                                if !out.is_empty() {
                                    out.push(' ');
                                }
                                out.push_str(&words.join(" "));
                            }
                        }
                    }
                    if out.is_empty() { None } else { Some(out) }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Run a Command through the normal control pathway.
    fn invoke_command(
        run_info: &ArcMutRuntimeInfo,
        cmd: crate::control::Command,
    ) -> Result<(), String> {
        crate::control::execute_command(cmd, run_info.clone()).map(|_| ())
    }

    /// Install a job for a D-Bus verb, run the corresponding synchronous
    /// control command (whose handler merges into and completes the job by
    /// ID), and return the job's object path like upstream does. The
    /// defensive finish afterwards is an idempotent no-op unless an early
    /// error left the job installed.
    fn run_job_command(
        run_info: &ArcMutRuntimeInfo,
        name: &str,
        mode: &str,
        kind: crate::units::jobs::JobKind,
        cmd: crate::control::Command,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        let id =
            crate::control::find_or_load_unit(name, run_info).map_err(zbus::fdo::Error::Failed)?;
        let jmode = if mode == "fail" {
            crate::units::jobs::JobMode::Fail
        } else {
            crate::units::jobs::JobMode::Replace
        };
        let jobs = run_info.read_poisoned().jobs.clone();
        let job_id = jobs
            .lock()
            .unwrap()
            .create(id, kind, crate::units::ActivationSource::Regular, jmode)
            .map_err(zbus::fdo::Error::Failed)?;
        let result = invoke_command(run_info, cmd);
        jobs.lock().unwrap().finish(
            job_id,
            if result.is_ok() {
                crate::units::jobs::JobResult::Done
            } else {
                crate::units::jobs::JobResult::Failed
            },
        );
        result.map_err(zbus::fdo::Error::Failed)?;
        Ok(job_object_path(job_id))
    }

    /// Map a `UnitStatus` to the D-Bus `ActiveState` and `SubState` strings
    /// that C systemd exports.
    fn map_active_sub(status: &UnitStatus) -> (&'static str, &'static str) {
        match status {
            UnitStatus::NeverStarted => ("inactive", "dead"),
            UnitStatus::Starting => ("activating", "start"),
            UnitStatus::Started(_) => ("active", "running"),
            UnitStatus::Stopping => ("deactivating", "stop"),
            UnitStatus::Restarting => ("activating", "auto-restart"),
            UnitStatus::Stopped(_, errs) if !errs.is_empty() => ("failed", "failed"),
            UnitStatus::Stopped(_, _) => ("inactive", "dead"),
        }
    }

    /// Encode a unit name into a D-Bus object path the way C systemd does.
    /// Example: `dbus.service` → `/org/freedesktop/systemd1/unit/dbus_2eservice`.
    fn unit_object_path(name: &str) -> zbus::zvariant::OwnedObjectPath {
        let mut encoded = String::with_capacity(name.len() * 2);
        for (i, b) in name.bytes().enumerate() {
            let keep = (b.is_ascii_alphanumeric() && (i > 0 || !b.is_ascii_digit()))
                || (b == b'_' && i > 0);
            if keep {
                encoded.push(b as char);
            } else {
                encoded.push_str(&format!("_{b:02x}"));
            }
        }
        let path = format!("/org/freedesktop/systemd1/unit/{encoded}");
        zbus::zvariant::OwnedObjectPath::try_from(path.as_str())
            .unwrap_or_else(|_| zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
    }

    /// Job object path: `/org/freedesktop/systemd1/job/<id>`.
    fn job_object_path(id: u32) -> zbus::zvariant::OwnedObjectPath {
        zbus::zvariant::OwnedObjectPath::try_from(format!("/org/freedesktop/systemd1/job/{id}"))
            .unwrap_or_else(|_| zbus::zvariant::OwnedObjectPath::try_from("/").unwrap())
    }

    /// Queue to the job-signal thread. Set once the system bus is up; events
    /// sent before then are dropped, matching upstream where boot-time jobs
    /// predate the bus.
    static JOB_DBUS_TX: std::sync::OnceLock<std::sync::mpsc::Sender<JobDbusEvent>> =
        std::sync::OnceLock::new();

    enum JobDbusEvent {
        Created {
            id: u32,
            unit: String,
            kind: &'static str,
        },
        Removed {
            id: u32,
            unit: String,
            result: &'static str,
        },
    }

    pub fn notify_job_created(job: &crate::units::jobs::Job) {
        if let Some(tx) = JOB_DBUS_TX.get() {
            let _ = tx.send(JobDbusEvent::Created {
                id: job.id,
                unit: job.unit.name.clone(),
                kind: job.kind.as_str(),
            });
        }
    }

    pub fn notify_job_removed(job: &crate::units::jobs::Job) {
        if let Some(tx) = JOB_DBUS_TX.get() {
            let _ = tx.send(JobDbusEvent::Removed {
                id: job.id,
                unit: job.unit.name.clone(),
                result: job.result.map_or("done", |r| r.as_str()),
            });
        }
    }

    /// Consume job lifecycle events on a dedicated thread: register and
    /// unregister the per-job objects and emit JobNew/JobRemoved. Runs off
    /// the zbus executor so the blocking emission cannot deadlock a method
    /// handler, at the cost of the signal trailing the method reply, which
    /// is the ordering sd-bus clients expect anyway.
    fn spawn_job_signal_thread(conn: Connection, run_info: ArcMutRuntimeInfo) {
        let (tx, rx) = std::sync::mpsc::channel::<JobDbusEvent>();
        if JOB_DBUS_TX.set(tx).is_err() {
            // A previous bus connection already owns the channel.
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("dbus-job-signals".into())
            .spawn(move || {
                let emitter = match zbus::object_server::SignalEmitter::new(
                    conn.inner(),
                    "/org/freedesktop/systemd1",
                ) {
                    Ok(emitter) => emitter,
                    Err(e) => {
                        warn!("dbus-server: no signal emitter for jobs: {e}");
                        return;
                    }
                };
                for event in rx {
                    match event {
                        JobDbusEvent::Created { id, unit, kind } => {
                            let path = job_object_path(id);
                            let obj = JobObj {
                                run_info: run_info.clone(),
                                id,
                                unit_name: unit.clone(),
                                kind,
                            };
                            let _ = conn.object_server().at(&path, obj);
                            let _ = zbus::block_on(Manager::job_new(&emitter, id, path, unit));
                        }
                        JobDbusEvent::Removed { id, unit, result } => {
                            let path = job_object_path(id);
                            let _ = conn.object_server().remove::<JobObj, _>(&path);
                            let _ = zbus::block_on(Manager::job_removed(
                                &emitter,
                                id,
                                path,
                                unit,
                                result.to_string(),
                            ));
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            warn!("dbus-server: failed to spawn job signal thread: {e}");
        }
    }

    /// Spawn a background thread that (eventually) connects to the system
    /// bus and exports the Manager interface.  Retries connecting every few
    /// seconds until `dbus.service` is available.
    pub fn start_dbus_server_thread(run_info: ArcMutRuntimeInfo) {
        std::thread::Builder::new()
            .name("dbus-server".into())
            .spawn(move || serve(run_info))
            .expect("dbus-server thread");
    }

    fn serve(run_info: ArcMutRuntimeInfo) {
        let manager = Manager {
            run_info: run_info.clone(),
        };
        let conn = loop {
            match Connection::system() {
                Ok(c) => break c,
                Err(e) => {
                    trace!("dbus-server: waiting for system bus: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        };
        if let Err(e) = conn
            .object_server()
            .at("/org/freedesktop/systemd1", manager)
        {
            warn!("dbus-server: failed to export Manager object: {e}");
            return;
        }
        match conn.request_name("org.freedesktop.systemd1") {
            Ok(()) => {
                info!("dbus-server: registered org.freedesktop.systemd1 on system bus");
            }
            Err(e) => {
                warn!("dbus-server: request_name org.freedesktop.systemd1 failed: {e}");
                return;
            }
        }

        spawn_job_signal_thread(conn.clone(), run_info.clone());

        // Register per-unit objects for all currently-loaded units.  New
        // units added later (e.g. transient units from StartTransientUnit)
        // are registered by register_unit_object() called from the
        // appropriate paths.
        {
            let ri = run_info.read_poisoned();
            let names: Vec<String> = ri.unit_table.values().map(|u| u.id.name.clone()).collect();
            drop(ri);
            let mut registered = 0usize;
            for name in names {
                if register_unit_object(&conn, &run_info, &name).is_ok() {
                    registered += 1;
                }
            }
            info!("dbus-server: registered {registered} per-unit objects");
        }

        // Periodically reconcile per-unit D-Bus objects with the unit
        // table: register objects for newly-added units (transient
        // services from StartTransientUnit, implicit slices,
        // daemon-reload additions) and unregister objects for units
        // removed on daemon-reload.  A 2 s cadence is fast enough that
        // `busctl get-property` sees new objects quickly and slow
        // enough to not contend on the read lock.
        let mut known: std::collections::HashSet<String> = {
            let ri = run_info.read_poisoned();
            ri.unit_table.values().map(|u| u.id.name.clone()).collect()
        };
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let current_names: std::collections::HashSet<String> = {
                let ri = run_info.read_poisoned();
                ri.unit_table.values().map(|u| u.id.name.clone()).collect()
            };
            // Additions
            for name in &current_names {
                if !known.contains(name) && register_unit_object(&conn, &run_info, name).is_ok() {
                    trace!("dbus-server: registered new unit object {name}");
                    known.insert(name.clone());
                }
            }
            // Removals — clean up stale object paths when a unit was
            // removed (daemon-reload with disk deletion, for example).
            let stale: Vec<String> = known
                .iter()
                .filter(|n| !current_names.contains(*n))
                .cloned()
                .collect();
            for name in stale {
                let path = unit_object_path(&name);
                let _ = conn.object_server().remove::<UnitObj, _>(&path);
                if name.ends_with(".service") {
                    let _ = conn.object_server().remove::<ServiceObj, _>(&path);
                } else if name.ends_with(".socket") {
                    let _ = conn.object_server().remove::<SocketObj, _>(&path);
                } else if name.ends_with(".timer") {
                    let _ = conn.object_server().remove::<TimerObj, _>(&path);
                } else if name.ends_with(".slice") {
                    let _ = conn.object_server().remove::<SliceObj, _>(&path);
                } else if name.ends_with(".path") {
                    let _ = conn.object_server().remove::<PathObj, _>(&path);
                }
                known.remove(&name);
                trace!("dbus-server: unregistered removed unit object {name}");
            }
        }
    }

    /// Register a Unit object at the encoded object path for `unit_name`.
    /// For `.service` units, also attach the Service interface.
    fn register_unit_object(
        conn: &Connection,
        run_info: &ArcMutRuntimeInfo,
        unit_name: &str,
    ) -> zbus::Result<()> {
        let path = unit_object_path(unit_name);
        let obj = UnitObj {
            run_info: run_info.clone(),
            unit_name: unit_name.to_owned(),
        };
        conn.object_server().at(&path, obj)?;
        if unit_name.ends_with(".service") {
            let svc = ServiceObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, svc)?;
        }
        if unit_name.ends_with(".socket") {
            let sock = SocketObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, sock)?;
        }
        if unit_name.ends_with(".timer") {
            let tmr = TimerObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, tmr)?;
        }
        if unit_name.ends_with(".slice") {
            let sl = SliceObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, sl)?;
        }
        if unit_name.ends_with(".path") {
            let pp = PathObj {
                run_info: run_info.clone(),
                unit_name: unit_name.to_owned(),
            };
            conn.object_server().at(&path, pp)?;
        }
        Ok(())
    }
}
